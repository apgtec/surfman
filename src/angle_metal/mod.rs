//! ANGLE on Metal, on macOS: OpenGL ES that ANGLE runs with Metal. Like the surfaceless Mesa
//! backend, it only supports off-screen surfaces.

pub mod connection;
pub mod context;
pub mod device;
pub mod surface;

crate::implement_interfaces!();

#[cfg(test)]
#[path = "../tests.rs"]
mod tests;
