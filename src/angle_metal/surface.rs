//! Wrapper for EGL surfaces on ANGLE.

use crate::base::egl::device::EGL_FUNCTIONS;
use crate::base::egl::surface::{EGLBackedSurface, EGLSurfaceTexture};
use crate::egl;
use crate::egl::types::{EGLDisplay, EGLSurface, EGLint};

use glow::Texture;
use objc2_core_foundation::CFRetained;
use objc2_io_surface::IOSurfaceRef;
use std::marker::PhantomData;

/// Represents a hardware buffer of pixels that can be rendered to via the CPU or GPU and either
/// displayed in a native widget or bound to a texture for reading.
///
/// Surfaces come in two varieties: generic and widget surfaces. Generic surfaces can be bound to a
/// texture but cannot be displayed in a widget (without using other APIs such as Core Animation,
/// DirectComposition, or XPRESENT). Widget surfaces are the opposite: they can be displayed in a
/// widget but not bound to a texture.
///
/// Surfaces are specific to a given context and cannot be rendered to from any context other than
/// the one they were created with. However, they can be *read* from any context on any thread (as
/// long as that context shares the same adapter and connection), by wrapping them in a
/// `SurfaceTexture`.
///
/// Depending on the platform, each surface may be internally double-buffered.
///
/// Surfaces must be destroyed with the `destroy_surface()` method, or a panic will occur.
#[derive(Debug)]
pub struct Surface(pub(crate) EGLBackedSurface);

/// Represents an OpenGL texture that wraps a surface.
///
/// Reading from the associated OpenGL texture reads from the surface. It is undefined behavior to
/// write to such a texture (e.g. by binding it to a framebuffer and rendering to that
/// framebuffer).
///
/// Surface textures are local to a context, but that context does not have to be the same context
/// as that associated with the underlying surface. The texture must be destroyed with the
/// `destroy_surface_texture()` method, or a panic will occur.
#[derive(Debug)]
pub struct SurfaceTexture(pub(crate) EGLSurfaceTexture);

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

/// A placeholder wrapper for a native widget.
#[derive(Clone)]
pub struct NativeWidget;

unsafe impl Send for Surface {}

/// Represents the CPU view of the pixel data of this surface.
pub struct SurfaceDataGuard<'a> {
    phantom: PhantomData<&'a ()>,
}
