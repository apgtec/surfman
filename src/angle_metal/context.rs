//! OpenGL ES rendering contexts on ANGLE.

use crate::base::egl::context::EGLBackedContext;
pub use crate::base::egl::context::NativeContext;
use crate::Gl;

/// An implementation of [`crate::Context`] for ANGLE on Metal.
pub struct AngleMetalContext(pub(crate) EGLBackedContext, pub(crate) Gl);
