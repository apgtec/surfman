//! Surfaces shared by the macOS backends.

#[cfg(angle_metal)]
use crate::angle_metal::surface::AngleMetalBufferBinding;
use crate::cgl::surface::CglSurface;
use crate::{ContextID, SurfaceID, SurfaceInfo};

use euclid::default::Size2D;
use objc2_core_foundation::CFRetained;
use objc2_io_surface::IOSurfaceRef;
use std::fmt::{self, Debug, Formatter};

/// An implementation of [`crate::OffscreenSurface`] backed by an `IOSurface`.
///
/// The `IOSurface` can be used by other devices, graphics APIs and processes.
pub struct IoSurfaceSurface {
    pub(crate) io_surface: CFRetained<IOSurfaceRef>,
    pub(crate) binding: IoSurfaceBinding,
}

/// The objects that bind an [`IoSurfaceSurface`] to the context of the backend that created it.
pub(crate) enum IoSurfaceBinding {
    Cgl(CglSurface),
    #[cfg(angle_metal)]
    AngleMetal(AngleMetalBufferBinding),
}

unsafe impl Send for IoSurfaceSurface {}

impl IoSurfaceSurface {
    /// Returns the size of the surface.
    #[inline]
    pub fn size(&self) -> Size2D<i32> {
        Size2D::new(
            self.io_surface.width() as i32,
            self.io_surface.height() as i32,
        )
    }

    /// Returns the `IOSurface` that holds the surface's pixels.
    #[inline]
    pub fn io_surface(&self) -> CFRetained<IOSurfaceRef> {
        self.io_surface.clone()
    }

    pub(crate) fn context_id(&self) -> ContextID {
        match self.binding {
            IoSurfaceBinding::Cgl(ref surface) => surface.context_id,
            #[cfg(angle_metal)]
            IoSurfaceBinding::AngleMetal(ref binding) => binding.context_id,
        }
    }

    pub(crate) fn id(&self) -> SurfaceID {
        SurfaceID(&*self.io_surface as *const IOSurfaceRef as usize)
    }

    pub(crate) fn info(&self) -> SurfaceInfo {
        SurfaceInfo {
            size: self.size(),
            id: self.id(),
            context_id: self.context_id(),
            framebuffer_object: match self.binding {
                IoSurfaceBinding::Cgl(ref surface) => surface.framebuffer_object,
                #[cfg(angle_metal)]
                IoSurfaceBinding::AngleMetal(ref binding) => binding.framebuffer_object,
            },
        }
    }
}

impl Debug for IoSurfaceSurface {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "Surface({:x})", self.id().0)
    }
}
