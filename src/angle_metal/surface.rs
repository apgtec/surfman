//! Wrapper for EGL surfaces on ANGLE.

use crate::base::egl::surface::{EGLBackedSurface, EGLSurfaceTexture};

use std::marker::PhantomData;

/// An implementation of [`crate::Surface`] for ANGLE on Metal.
#[derive(Debug)]
pub struct AngleMetalSurface(pub(crate) EGLBackedSurface);

/// An implementation of [`crate::SurfaceTexture`] for ANGLE on Metal.
#[derive(Debug)]
pub struct AngleMetalSurfaceTexture(pub(crate) EGLSurfaceTexture);

unsafe impl Send for AngleMetalSurface {}

/// Represents the CPU view of the pixel data of this surface.
pub struct SurfaceDataGuard<'a> {
    phantom: PhantomData<&'a ()>,
}
