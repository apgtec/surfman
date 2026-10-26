//! Wrapper for EGL surfaces on ANGLE.

use crate::base::egl::device::EGL_FUNCTIONS;
use crate::base::egl::surface::{EGLBackedSurface, EGLSurfaceTexture};
use crate::egl;
use crate::egl::types::{EGLDisplay, EGLSurface, EGLint};

use glow::Texture;
use objc2_core_foundation::CFRetained;
use objc2_io_surface::IOSurfaceRef;
use std::marker::PhantomData;

/// An implementation of [`crate::Surface`] for ANGLE on Metal.
#[derive(Debug)]
pub struct AngleMetalSurface(pub(crate) EGLBackedSurface);

/// An implementation of [`crate::SurfaceTexture`] for ANGLE on Metal.
#[derive(Debug)]
pub struct AngleMetalSurfaceTexture(pub(crate) EGLSurfaceTexture);

unsafe impl Send for AngleMetalSurface {}

/// A texture of a context holding an IOSurface the caller owns, which may have been made by
/// another API, such as Metal. The texture can be rendered to as well as read from.
///
/// Destroy it with `destroy_io_surface_texture()`. A caller that deletes the texture itself, with
/// the context current, drops this afterwards, which unbinds the IOSurface.
#[derive(Debug)]
pub struct IOSurfaceTexture {
    /// Kept alive while the pbuffer refers to it.
    #[allow(dead_code)]
    pub(crate) io_surface: CFRetained<IOSurfaceRef>,
    pub(crate) egl_display: EGLDisplay,
    pub(crate) egl_pbuffer: EGLSurface,
    pub(crate) texture_object: Texture,
}

impl IOSurfaceTexture {
    /// Returns the texture holding the IOSurface.
    #[inline]
    pub fn texture_object(&self) -> Texture {
        self.texture_object
    }
}

impl Drop for IOSurfaceTexture {
    fn drop(&mut self) {
        EGL_FUNCTIONS.with(|egl| unsafe {
            egl.ReleaseTexImage(
                self.egl_display,
                self.egl_pbuffer,
                egl::BACK_BUFFER as EGLint,
            );
            egl.DestroySurface(self.egl_display, self.egl_pbuffer);
        });
    }
}

/// Represents the CPU view of the pixel data of this surface.
pub struct SurfaceDataGuard<'a> {
    phantom: PhantomData<&'a ()>,
}
