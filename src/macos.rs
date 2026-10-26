//! The default backend for macOS with the `sm-angle-metal` feature, which dynamically switches
//! between Apple's OpenGL and ANGLE on Metal. Apple's OpenGL is the default; ANGLE is the
//! alternate.

/// Apple's OpenGL or ANGLE display connections.
pub mod connection {
    use crate::angle_metal::device::AngleMetalDevice;
    use crate::cgl::device::CglDevice;
    use crate::multi::connection::Connection as MultiConnection;

    /// Either an Apple OpenGL or an ANGLE display connection.
    pub type Connection = MultiConnection<CglDevice, AngleMetalDevice>;
}

/// Thread-local handles to devices.
pub mod device {
    use crate::angle_metal::device::AngleMetalDevice;
    use crate::cgl::device::CglDevice;
    use crate::multi::device::Device as MultiDevice;

    /// A thread-local handle to a device.
    ///
    /// Devices contain most of the relevant surface management methods.
    pub type Device = MultiDevice<CglDevice, AngleMetalDevice>;
}
