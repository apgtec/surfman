//! ANGLE on Metal, on macOS: OpenGL ES that ANGLE runs with Metal. Like the surfaceless Mesa
//! backend, it only supports off-screen surfaces.

pub mod connection;
pub mod context;
pub mod device;
pub mod surface;

#[cfg(test)]
use self::connection::AngleMetalConnection as Connection;
#[cfg(test)]
use self::device::AngleMetalDevice as ConcreteDevice;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
#[path = "../tests.rs"]
mod tests;
