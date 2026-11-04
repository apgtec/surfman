// surfman/src/platform/windows/wgl/surface.rs
//
//! An implementation of the GPU device for Windows using WGL/Direct3D interoperability.

use crate::renderbuffers::Renderbuffers;
use crate::windows::surface::{Direct3dBinding, Direct3dSurface, Win32Binding, Win32Surface};
use crate::{ContextID, Error, OffscreenSurface, Surface, SurfaceID, SurfaceInfo};

use euclid::default::Size2D;

use std::fmt::{self, Debug, Formatter};
use std::marker::PhantomData;
use std::thread;
use winapi::shared::minwindef::FALSE;
use winapi::shared::ntdef::HANDLE;
use winapi::um::d3d11::ID3D11Texture2D;
use winapi::um::wingdi;
use winapi::um::winuser;
use wio::com::ComPtr;

/// A surface bound to a WGL context.
pub(crate) enum WglSurface {
    Window(Win32Surface),
    Buffer(Direct3dSurface),
    GlTexture(GlTextureSurface),
}

/// An implementation of [`crate::OffscreenSurface`] for WGL without `WGL_NV_DX_interop`: a plain
/// OpenGL texture, which the contexts reading it share.
pub struct GlTextureSurface {
    pub(crate) size: Size2D<i32>,
    pub(crate) context_id: ContextID,
    pub(crate) gl_texture: Option<glow::Texture>,
    pub(crate) gl_framebuffer: Option<glow::Framebuffer>,
    pub(crate) renderbuffers: Renderbuffers,
    // Signalled when the context that last rendered to or read the texture is done with it,
    // for the next one to wait on.
    pub(crate) gl_sync: Option<glow::Fence>,
    pub(crate) destroyed: bool,
}

/// The WGL objects of a [`Win32Surface`].
pub(crate) struct WglWindowBinding {
    pub(crate) destroyed: bool,
}

/// The WGL objects of a [`Direct3dSurface`].
pub(crate) struct WglBufferBinding {
    pub(crate) d3d11_texture: ComPtr<ID3D11Texture2D>,
    pub(crate) gl_dx_interop_object: HANDLE,
    pub(crate) gl_texture: Option<glow::Texture>,
    pub(crate) gl_framebuffer: Option<glow::Framebuffer>,
    pub(crate) renderbuffers: Renderbuffers,
    pub(crate) destroyed: bool,
}

/// An implementation of [`crate::SurfaceTexture`] for WGL platforms.
pub struct WglSurfaceTexture {
    pub(crate) surface: OffscreenSurface,
    #[allow(dead_code)]
    pub(crate) local_d3d11_texture: Option<ComPtr<ID3D11Texture2D>>,
    pub(crate) local_gl_dx_interop_object: HANDLE,
    pub(crate) gl_texture: Option<glow::Texture>,
    pub(crate) phantom: PhantomData<*const ()>,
}

impl Drop for WglWindowBinding {
    fn drop(&mut self) {
        if !self.destroyed && !thread::panicking() {
            panic!("Should have destroyed the surface first with `destroy_window_surface()`!")
        }
    }
}

impl Drop for WglBufferBinding {
    fn drop(&mut self) {
        if !self.destroyed && !thread::panicking() {
            panic!("Should have destroyed the surface first with `destroy_offscreen_surface()`!")
        }
    }
}

impl Drop for GlTextureSurface {
    fn drop(&mut self) {
        if !self.destroyed && !thread::panicking() {
            panic!("Should have destroyed the surface first with `destroy_offscreen_surface()`!")
        }
    }
}

unsafe impl Send for GlTextureSurface {}

impl Debug for GlTextureSurface {
    fn fmt(&self, formatter: &mut Formatter) -> Result<(), fmt::Error> {
        write!(formatter, "GlTextureSurface({:x})", self.id().0)
    }
}

impl GlTextureSurface {
    /// Returns the size of the surface.
    #[inline]
    pub fn size(&self) -> Size2D<i32> {
        self.size
    }

    /// Returns the OpenGL texture that holds the surface's contents, which contexts in the
    /// surface's share group can read.
    #[inline]
    pub fn gl_texture(&self) -> Option<glow::Texture> {
        self.gl_texture
    }

    pub(crate) fn id(&self) -> SurfaceID {
        SurfaceID(
            self.gl_texture
                .map_or(0, |texture| texture.0.get() as usize),
        )
    }

    pub(crate) fn info(&self) -> SurfaceInfo {
        SurfaceInfo {
            size: self.size,
            id: self.id(),
            context_id: self.context_id,
            framebuffer_object: self.gl_framebuffer,
        }
    }
}

impl Debug for WglSurfaceTexture {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "SurfaceTexture({:?})", self.surface)
    }
}

impl Win32Surface {
    pub(crate) fn wgl(&self) -> Result<&WglWindowBinding, Error> {
        #[allow(unreachable_patterns)]
        match self.binding {
            Win32Binding::Wgl(ref binding) => Ok(binding),
            _ => Err(Error::IncompatibleSurface),
        }
    }

    pub(crate) fn wgl_mut(&mut self) -> Result<&mut WglWindowBinding, Error> {
        #[allow(unreachable_patterns)]
        match self.binding {
            Win32Binding::Wgl(ref mut binding) => Ok(binding),
            _ => Err(Error::IncompatibleSurface),
        }
    }

    pub(crate) fn present_wgl(&self) -> Result<(), Error> {
        self.wgl()?;
        let dc = unsafe { winuser::GetDC(self.window) };
        let ok = unsafe { wingdi::SwapBuffers(dc) };
        assert_ne!(ok, FALSE);
        unsafe { winuser::ReleaseDC(self.window, dc) };
        Ok(())
    }
}

impl Direct3dSurface {
    pub(crate) fn wgl(&self) -> Result<&WglBufferBinding, Error> {
        #[allow(unreachable_patterns)]
        match self.binding {
            Direct3dBinding::Wgl(ref binding) => Ok(binding),
            _ => Err(Error::IncompatibleSurface),
        }
    }

    pub(crate) fn wgl_mut(&mut self) -> Result<&mut WglBufferBinding, Error> {
        #[allow(unreachable_patterns)]
        match self.binding {
            Direct3dBinding::Wgl(ref mut binding) => Ok(binding),
            _ => Err(Error::IncompatibleSurface),
        }
    }
}

impl WglSurface {
    pub(crate) fn context_id(&self) -> ContextID {
        match self {
            WglSurface::Window(surface) => surface.context_id,
            WglSurface::Buffer(surface) => surface.context_id,
            WglSurface::GlTexture(surface) => surface.context_id,
        }
    }

    pub(crate) fn is_wgl(&self) -> bool {
        match self {
            WglSurface::Window(surface) => surface.wgl().is_ok(),
            WglSurface::Buffer(surface) => surface.wgl().is_ok(),
            WglSurface::GlTexture(_) => true,
        }
    }

    pub(crate) fn info(&self) -> SurfaceInfo {
        match self {
            WglSurface::Window(surface) => surface.info(),
            WglSurface::Buffer(surface) => surface.info(),
            WglSurface::GlTexture(surface) => surface.info(),
        }
    }
}

impl From<WglSurface> for Surface {
    fn from(surface: WglSurface) -> Surface {
        match surface {
            WglSurface::Window(surface) => Surface::Window(surface.into()),
            WglSurface::Buffer(surface) => Surface::Offscreen(surface.into()),
            WglSurface::GlTexture(surface) => Surface::Offscreen(surface.into()),
        }
    }
}

/// Represents the CPU view of the pixel data of this surface.
pub struct SurfaceDataGuard<'a> {
    phantom: PhantomData<&'a ()>,
}
