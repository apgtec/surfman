//! Exporting EGL-backed buffer surfaces as DMA-BUFs and importing DMA-BUFs as surfaces.

use super::device::{display_supports_extension, EGL_FUNCTIONS};
use super::error::ToWindowingApiError;
use super::ffi::{
    DRM_FORMAT_MOD_INVALID, EGL_DMA_BUF_PLANE_FD_EXT, EGL_DMA_BUF_PLANE_MODIFIER_HI_EXT,
    EGL_DMA_BUF_PLANE_MODIFIER_LO_EXT, EGL_DMA_BUF_PLANE_OFFSET_EXT, EGL_DMA_BUF_PLANE_PITCH_EXT,
    EGL_EXTENSION_FUNCTIONS, EGL_LINUX_DMA_BUF_EXT, EGL_LINUX_DRM_FOURCC_EXT, EGL_NO_IMAGE_KHR,
};
use super::surface::{bind_egl_image_to_gl_texture, EGLBufferSurface};
use crate::buffer_handle::{DmaBuf, DmaBufPlane};
use crate::egl;
use crate::egl::types::{EGLDisplay, EGLint};
use crate::renderbuffers::Renderbuffers;
use crate::{gl, gl_utils, ContextAttributes, ContextID, Error, Gl, WindowingApiError};

use euclid::default::Size2D;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr;

fn egl_error() -> WindowingApiError {
    EGL_FUNCTIONS.with(|egl| unsafe { egl.GetError() }.to_windowing_api_error())
}

impl EGLBufferSurface {
    /// Exports the image behind this buffer surface as a DMA-BUF, with
    /// `EGL_MESA_image_dma_buf_export`.
    pub(crate) fn dma_buf(&self, egl_display: EGLDisplay) -> Result<DmaBuf, Error> {
        let egl_image = self.egl_image;
        let (Some(query), Some(export)) = (
            EGL_EXTENSION_FUNCTIONS.ExportDMABUFImageQueryMESA,
            EGL_EXTENSION_FUNCTIONS.ExportDMABUFImageMESA,
        ) else {
            return Err(Error::RequiredExtensionUnavailable);
        };
        if !display_supports_extension(egl_display, "EGL_MESA_image_dma_buf_export") {
            return Err(Error::RequiredExtensionUnavailable);
        }

        let (mut fourcc, mut plane_count) = (0, 0);
        let mut modifiers = [DRM_FORMAT_MOD_INVALID; 4];
        if query(
            egl_display,
            egl_image,
            &mut fourcc,
            &mut plane_count,
            ptr::null_mut(),
        ) == egl::FALSE
            || !(1..=4).contains(&plane_count)
            || query(
                egl_display,
                egl_image,
                &mut fourcc,
                &mut plane_count,
                modifiers.as_mut_ptr(),
            ) == egl::FALSE
        {
            return Err(Error::SurfaceExportFailed(egl_error()));
        }

        let (mut fds, mut strides, mut offsets) = ([-1; 4], [0; 4], [0; 4]);
        if export(
            egl_display,
            egl_image,
            fds.as_mut_ptr(),
            strides.as_mut_ptr(),
            offsets.as_mut_ptr(),
        ) == egl::FALSE
        {
            return Err(Error::SurfaceExportFailed(egl_error()));
        }

        // Planes in the same file may come back with the same descriptor, or with -1 for the
        // first plane's. Give each plane a descriptor of its own.
        let mut planes: Vec<DmaBufPlane> = Vec::with_capacity(plane_count as usize);
        for plane in 0..plane_count as usize {
            let shared = planes
                .iter()
                .find(|previous| fds[plane] < 0 || previous.fd.as_raw_fd() == fds[plane]);
            let fd = match shared {
                Some(previous) => previous
                    .fd
                    .try_clone()
                    .map_err(|_| Error::SurfaceExportFailed(WindowingApiError::Failed))?,
                None if fds[plane] < 0 => {
                    return Err(Error::SurfaceExportFailed(WindowingApiError::Failed))
                }
                None => unsafe { OwnedFd::from_raw_fd(fds[plane]) },
            };
            planes.push(DmaBufPlane {
                fd,
                offset: offsets[plane] as u32,
                stride: strides[plane] as u32,
            });
        }

        Ok(DmaBuf {
            width: self.size.width,
            height: self.size.height,
            fourcc: fourcc as u32,
            modifier: modifiers[0],
            planes,
        })
    }

    /// Creates a buffer surface that renders into a DMA-BUF, with
    /// `EGL_EXT_image_dma_buf_import`. The context must be current.
    pub(crate) fn from_dma_buf(
        gl: &Gl,
        egl_display: EGLDisplay,
        context_id: ContextID,
        context_attributes: &ContextAttributes,
        dma_buf: &DmaBuf,
    ) -> Result<EGLBufferSurface, Error> {
        if !(1..=4).contains(&dma_buf.planes.len()) {
            return Err(Error::SurfaceImportFailed(WindowingApiError::BadParameter));
        }

        let mut attributes = vec![
            egl::WIDTH as EGLint,
            dma_buf.width,
            egl::HEIGHT as EGLint,
            dma_buf.height,
            EGL_LINUX_DRM_FOURCC_EXT as EGLint,
            dma_buf.fourcc as EGLint,
        ];
        for (index, plane) in dma_buf.planes.iter().enumerate() {
            attributes.extend_from_slice(&[
                EGL_DMA_BUF_PLANE_FD_EXT[index] as EGLint,
                plane.fd.as_raw_fd(),
                EGL_DMA_BUF_PLANE_OFFSET_EXT[index] as EGLint,
                plane.offset as EGLint,
                EGL_DMA_BUF_PLANE_PITCH_EXT[index] as EGLint,
                plane.stride as EGLint,
            ]);
            if dma_buf.modifier != DRM_FORMAT_MOD_INVALID {
                attributes.extend_from_slice(&[
                    EGL_DMA_BUF_PLANE_MODIFIER_LO_EXT[index] as EGLint,
                    dma_buf.modifier as u32 as EGLint,
                    EGL_DMA_BUF_PLANE_MODIFIER_HI_EXT[index] as EGLint,
                    (dma_buf.modifier >> 32) as u32 as EGLint,
                ]);
            }
        }
        attributes.push(egl::NONE as EGLint);

        let egl_image = (EGL_EXTENSION_FUNCTIONS.CreateImageKHR)(
            egl_display,
            egl::NO_CONTEXT,
            EGL_LINUX_DMA_BUF_EXT,
            ptr::null_mut(),
            attributes.as_ptr(),
        );
        if egl_image == EGL_NO_IMAGE_KHR {
            return Err(Error::SurfaceImportFailed(egl_error()));
        }

        let size = Size2D::new(dma_buf.width, dma_buf.height);
        unsafe {
            let texture_object = bind_egl_image_to_gl_texture(gl, egl_image);
            let framebuffer_object =
                gl_utils::create_and_bind_framebuffer(gl, gl::TEXTURE_2D, Some(texture_object));
            let renderbuffers = Renderbuffers::new(gl, &size, context_attributes);
            renderbuffers.bind_to_current_framebuffer(gl);

            Ok(EGLBufferSurface {
                context_id,
                size,
                egl_image,
                framebuffer_object: Some(framebuffer_object),
                texture_object: Some(texture_object),
                renderbuffers,
                destroyed: false,
            })
        }
    }
}
