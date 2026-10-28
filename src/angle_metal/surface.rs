//! Surface management for ANGLE on Metal, which renders to `IOSurface`s through pbuffers.

use crate::base::egl::device::EGL_FUNCTIONS;
use crate::base::egl::error::ToWindowingApiError;
use crate::base::egl::ffi::{
    EGL_IOSURFACE_ANGLE, EGL_IOSURFACE_PLANE_ANGLE, EGL_TEXTURE_INTERNAL_FORMAT_ANGLE,
    EGL_TEXTURE_TYPE_ANGLE,
};
use crate::egl;
use crate::egl::types::{EGLConfig, EGLDisplay, EGLSurface, EGLint};
use crate::macos::surface::{IoSurfaceBinding, IoSurfaceSurface};
use crate::renderbuffers::Renderbuffers;
use crate::{gl, ContextID, Error, Gl};

use glow::{Framebuffer, HasContext, Texture};
use objc2_io_surface::IOSurfaceRef;
use std::fmt::{self, Debug, Formatter};
use std::marker::PhantomData;
use std::thread;

/// The ANGLE on Metal objects of an [`IoSurfaceSurface`]: a pbuffer of the `IOSurface`, bound
/// to the texture that the surface's framebuffer renders to.
pub(crate) struct AngleMetalBufferBinding {
    pub(crate) context_id: ContextID,
    pub(crate) egl_pbuffer: EGLSurface,
    pub(crate) framebuffer_object: Option<Framebuffer>,
    pub(crate) texture_object: Option<Texture>,
    pub(crate) renderbuffers: Renderbuffers,
    pub(crate) destroyed: bool,
}

/// An implementation of [`crate::SurfaceTexture`] for ANGLE on Metal.
pub struct AngleMetalSurfaceTexture {
    pub(crate) surface: IoSurfaceSurface,
    /// The pbuffer of the surface's `IOSurface` that the texture is bound to. A pbuffer bound to
    /// one texture cannot be bound to another, so the surface texture has a pbuffer of its own.
    pub(crate) egl_pbuffer: EGLSurface,
    pub(crate) texture_object: Option<Texture>,
    pub(crate) phantom: PhantomData<*const ()>,
}

impl Drop for AngleMetalBufferBinding {
    fn drop(&mut self) {
        if !self.destroyed && !thread::panicking() {
            panic!("Should have destroyed the surface first with `destroy_surface()`!")
        }
    }
}

impl Debug for AngleMetalSurfaceTexture {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "SurfaceTexture({:?})", self.surface)
    }
}

impl IoSurfaceSurface {
    pub(crate) fn angle_metal(&self) -> Result<&AngleMetalBufferBinding, Error> {
        #[allow(unreachable_patterns)]
        match self.binding {
            IoSurfaceBinding::AngleMetal(ref binding) => Ok(binding),
            _ => Err(Error::IncompatibleSurface),
        }
    }

    pub(crate) fn angle_metal_mut(&mut self) -> Result<&mut AngleMetalBufferBinding, Error> {
        #[allow(unreachable_patterns)]
        match self.binding {
            IoSurfaceBinding::AngleMetal(ref mut binding) => Ok(binding),
            _ => Err(Error::IncompatibleSurface),
        }
    }
}

/// A pbuffer of the BGRA `io_surface`, with `EGL_ANGLE_iosurface_client_buffer`.
pub(crate) unsafe fn create_io_surface_pbuffer(
    egl_display: EGLDisplay,
    egl_config: EGLConfig,
    io_surface: &IOSurfaceRef,
) -> Result<EGLSurface, Error> {
    // The IOSurfaces surfman creates are BGRA, those of other APIs can also be RGBA.
    let internal_format = if io_surface.pixel_format() == u32::from_be_bytes(*b"RGBA") {
        gl::RGBA
    } else {
        gl::BGRA
    };
    let attributes = [
        egl::WIDTH as EGLint,
        io_surface.width() as EGLint,
        egl::HEIGHT as EGLint,
        io_surface.height() as EGLint,
        EGL_IOSURFACE_PLANE_ANGLE as EGLint,
        0,
        egl::TEXTURE_TARGET as EGLint,
        egl::TEXTURE_2D as EGLint,
        EGL_TEXTURE_INTERNAL_FORMAT_ANGLE as EGLint,
        internal_format as EGLint,
        egl::TEXTURE_FORMAT as EGLint,
        egl::TEXTURE_RGBA as EGLint,
        EGL_TEXTURE_TYPE_ANGLE as EGLint,
        gl::UNSIGNED_BYTE as EGLint,
        egl::NONE as EGLint,
    ];
    EGL_FUNCTIONS.with(|egl| {
        let egl_pbuffer = egl.CreatePbufferFromClientBuffer(
            egl_display,
            EGL_IOSURFACE_ANGLE,
            io_surface as *const IOSurfaceRef as _,
            egl_config,
            attributes.as_ptr(),
        );
        if egl_pbuffer == egl::NO_SURFACE {
            return Err(Error::SurfaceCreationFailed(
                egl.GetError().to_windowing_api_error(),
            ));
        }
        Ok(egl_pbuffer)
    })
}

/// A new texture of the current context that `egl_pbuffer` is bound to.
pub(crate) unsafe fn bind_pbuffer_to_gl_texture(
    gl: &Gl,
    egl_display: EGLDisplay,
    egl_pbuffer: EGLSurface,
) -> Result<Texture, Error> {
    let texture = gl.create_texture().unwrap();
    let texture_binding = gl.get_parameter_texture(gl::TEXTURE_BINDING_2D);
    gl.bind_texture(gl::TEXTURE_2D, Some(texture));
    let bound = EGL_FUNCTIONS.with(|egl| {
        if egl.BindTexImage(egl_display, egl_pbuffer, egl::BACK_BUFFER as EGLint) == egl::FALSE {
            return Err(Error::SurfaceTextureCreationFailed(
                egl.GetError().to_windowing_api_error(),
            ));
        }
        Ok(())
    });
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as _);
    gl.bind_texture(gl::TEXTURE_2D, texture_binding);
    match bound {
        Ok(()) => Ok(texture),
        Err(err) => {
            gl.delete_texture(texture);
            Err(err)
        }
    }
}

pub(crate) unsafe fn release_pbuffer_from_gl_texture(
    gl: &Gl,
    egl_display: EGLDisplay,
    egl_pbuffer: EGLSurface,
    texture: Texture,
) {
    EGL_FUNCTIONS
        .with(|egl| egl.ReleaseTexImage(egl_display, egl_pbuffer, egl::BACK_BUFFER as EGLint));
    gl.delete_texture(texture);
}

/// Represents the CPU view of the pixel data of this surface.
pub struct SurfaceDataGuard<'a> {
    phantom: PhantomData<&'a ()>,
}
