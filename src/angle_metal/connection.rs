//! Represents a connection to a display server.

use super::device::{AngleMetalDevice, NativeDevice};
use crate::base::egl::device::{egl_library_loaded, EGL_FUNCTIONS};
use crate::base::egl::ffi::{
    EGL_PLATFORM_ANGLE_ANGLE, EGL_PLATFORM_ANGLE_TYPE_ANGLE, EGL_PLATFORM_ANGLE_TYPE_METAL_ANGLE,
};
use crate::base::io_surface::adapter::AppleAdapter;
use crate::egl::types::{EGLAttrib, EGLDisplay};
use crate::info::GLApi;
use crate::Error;
use crate::{egl, Adapter, AdapterPreferences, PowerPreference, RenderingPreference};

use std::os::raw::c_void;
use std::sync::Arc;

/// A connection to an ANGLE display.
#[derive(Clone)]
pub struct AngleMetalConnection {
    pub(crate) native_connection: Arc<NativeConnectionWrapper>,
}

/// Native connections.
#[derive(Clone)]
pub struct NativeConnection(Arc<NativeConnectionWrapper>);

/// Native connections.
pub struct NativeConnectionWrapper {
    pub(crate) egl_display: EGLDisplay,
}

unsafe impl Send for NativeConnectionWrapper {}
unsafe impl Sync for NativeConnectionWrapper {}

impl NativeConnection {
    /// Wraps an EGL display the embedder already opened and initialised, so that devices created
    /// from this connection share it.
    ///
    /// # Safety
    /// `egl_display` must be a valid, initialised `EGLDisplay` that outlives the connection.
    #[inline]
    pub unsafe fn from_raw_display(egl_display: EGLDisplay) -> NativeConnection {
        NativeConnection(Arc::new(NativeConnectionWrapper { egl_display }))
    }
}

impl AngleMetalConnection {
    /// Opens an ANGLE display that renders with Metal. Fails if ANGLE's `libEGL.dylib` cannot be
    /// loaded.
    #[inline]
    pub fn new() -> Result<AngleMetalConnection, Error> {
        if !egl_library_loaded() {
            return Err(Error::ConnectionFailed);
        }
        unsafe {
            EGL_FUNCTIONS.with(|egl| {
                let egl_display_attributes = [
                    EGL_PLATFORM_ANGLE_TYPE_ANGLE as EGLAttrib,
                    EGL_PLATFORM_ANGLE_TYPE_METAL_ANGLE as EGLAttrib,
                    egl::NONE as EGLAttrib,
                ];
                let egl_display = egl.GetPlatformDisplay(
                    EGL_PLATFORM_ANGLE_ANGLE,
                    egl::DEFAULT_DISPLAY as *mut c_void,
                    egl_display_attributes.as_ptr(),
                );
                if egl_display == egl::NO_DISPLAY {
                    return Err(Error::ConnectionFailed);
                }

                let (mut egl_major_version, mut egl_minor_version) = (0, 0);
                let ok =
                    egl.Initialize(egl_display, &mut egl_major_version, &mut egl_minor_version);
                if ok == egl::FALSE {
                    return Err(Error::ConnectionFailed);
                }

                let native_connection =
                    NativeConnection(Arc::new(NativeConnectionWrapper { egl_display }));

                AngleMetalConnection::from_native_connection(native_connection)
            })
        }
    }

    /// An alias for `Connection::new()`, present for consistency with other backends.
    #[inline]
    pub unsafe fn from_native_connection(
        native_connection: NativeConnection,
    ) -> Result<AngleMetalConnection, Error> {
        Ok(AngleMetalConnection {
            native_connection: native_connection.0,
        })
    }

    /// Returns the underlying native connection.
    #[inline]
    pub fn native_connection(&self) -> NativeConnection {
        NativeConnection(self.native_connection.clone())
    }

    /// Returns the OpenGL API flavor that this connection supports (OpenGL or OpenGL ES).
    #[inline]
    pub fn gl_api(&self) -> GLApi {
        GLApi::GLES
    }

    /// Returns an adapter on this system according to the provided preferences. ANGLE renders
    /// with the system's default Metal device whichever is asked for.
    #[inline]
    pub fn create_adapter(&self, preferences: AdapterPreferences) -> Result<Adapter, Error> {
        let is_low_power = matches!(preferences.power, PowerPreference::LowPower)
            || matches!(preferences.rendering, RenderingPreference::Software);
        Ok(AppleAdapter { is_low_power }.into())
    }

    /// Opens the hardware device corresponding to the given adapter.
    ///
    /// Device handles are local to a single thread.
    #[inline]
    pub fn create_device(&self, adapter: &Adapter) -> Result<AngleMetalDevice, Error> {
        AngleMetalDevice::new(self, adapter)
    }

    /// An alias for `connection.create_device()` with the default adapter.
    #[inline]
    pub unsafe fn create_device_from_native_device(
        &self,
        _: NativeDevice,
    ) -> Result<AngleMetalDevice, Error> {
        AngleMetalDevice::new(self, &self.create_adapter(Default::default())?)
    }

    /// Opens the display connection corresponding to the given `DisplayHandle`.
    pub fn from_display_handle(
        _: raw_window_handle::DisplayHandle,
    ) -> Result<AngleMetalConnection, Error> {
        Err(Error::IncompatibleRawDisplayHandle)
    }
}
