//! A connection to the window server.
//!
//! Window server connections are implicit in the Win32 API.

use super::adapter::WglAdapter;
use super::device::{NativeDevice, WglDevice};
use crate::{Adapter, AdapterPreferences, Device, Error, GLApi};

use std::sync::{Arc, Mutex};

/// Represents a connection to the display server.
///
/// Window server connections are implicit in the Win32 API. Without `WGL_NV_DX_interop`, a
/// connection and its clones also make a share group: generic surfaces are then plain OpenGL
/// textures, which other contexts read directly, so every context made on a device of this
/// connection shares objects with the first context such a device made or adopted, unless it is
/// given another context to share with.
#[derive(Clone)]
pub struct WglConnection {
    // The first context's `HGLRC`, or 0 before there is one.
    pub(crate) gl_share_root: Arc<Mutex<usize>>,
}

/// An empty placeholder for native connections.
///
/// Window server connections are implicit in the Win32 API, so this is a zero-sized type.
#[derive(Clone)]
pub struct NativeConnection;

impl WglConnection {
    /// Connects to the default display.
    #[inline]
    pub fn new() -> Result<WglConnection, Error> {
        Ok(WglConnection {
            gl_share_root: Arc::default(),
        })
    }

    /// An alias for `Connection::new()`, present for consistency with other backends.
    #[inline]
    pub unsafe fn from_native_connection(_: NativeConnection) -> Result<WglConnection, Error> {
        WglConnection::new()
    }

    /// Returns the underlying native connection.
    #[inline]
    pub fn native_connection(&self) -> NativeConnection {
        NativeConnection
    }

    /// Returns the OpenGL API flavor that this connection supports (OpenGL or OpenGL ES).
    #[inline]
    pub fn gl_api(&self) -> GLApi {
        GLApi::GL
    }

    /// Returns an adapter on this system according to the provided preferences.
    #[inline]
    pub fn create_adapter(&self, preferences: AdapterPreferences) -> Result<Adapter, Error> {
        Ok(WglAdapter::new(preferences).into())
    }

    /// Opens a device.
    #[inline]
    pub fn create_device(&self, adapter: &Adapter) -> Result<Device, Error> {
        WglDevice::new(self, adapter.wgl()?).map(Device::from)
    }

    /// Opens a device whose buffer surfaces are OpenGL textures that the other contexts of the
    /// connection's share group read directly, as when `WGL_NV_DX_interop` is missing.
    ///
    /// This is for sharing surfaces with an OpenGL context that the embedder owns, through
    /// `create_context_from_native_context()`. These surfaces have no buffer handle.
    #[inline]
    pub fn create_device_without_dx_interop(&self, adapter: &Adapter) -> Result<WglDevice, Error> {
        Ok(WglDevice::new_without_dx_interop(self, adapter.wgl()?))
    }

    /// Creates a `Device` from a Direct3D 11 device and associated GL/DX interop handle.
    ///
    /// The handle can be created by calling `wglDXOpenDeviceNV` from the `WGL_NV_DX_interop`
    /// extension.
    ///
    /// This method increases the reference count on the Direct3D 11 device and takes ownership of
    /// the GL/DX interop handle.
    #[inline]
    pub unsafe fn create_device_from_native_device(
        &self,
        native_device: NativeDevice,
    ) -> Result<Device, Error> {
        WglDevice::from_native_device(self, native_device).map(Device::from)
    }

    /// Opens the display connection corresponding to the given `DisplayHandle`.
    pub fn from_display_handle(
        _: raw_window_handle::DisplayHandle,
    ) -> Result<WglConnection, Error> {
        WglConnection::new()
    }
}

impl Default for NativeConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeConnection {
    /// Creates a native connection.
    ///
    /// This is a no-op method present for consistency with other backends.
    #[inline]
    pub fn new() -> NativeConnection {
        NativeConnection
    }
}
