//! GPU-side synchronization with other devices and APIs through sync file descriptors, with
//! `EGL_ANDROID_native_fence_sync`.

use super::device::{display_supports_extension, EGL_FUNCTIONS};
use super::error::ToWindowingApiError;
use super::ffi::{
    EGL_EXTENSION_FUNCTIONS, EGL_NO_NATIVE_FENCE_FD_ANDROID, EGL_SYNC_NATIVE_FENCE_ANDROID,
    EGL_SYNC_NATIVE_FENCE_FD_ANDROID,
};
use crate::egl;
use crate::egl::types::{EGLAttrib, EGLDisplay};
use crate::{Error, Gl, WindowingApiError};

use glow::HasContext;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};

/// Returns a sync file descriptor that signals once the GPU has finished what the current
/// context has been given so far.
pub(crate) fn export(gl: &Gl, egl_display: EGLDisplay) -> Result<OwnedFd, Error> {
    let dup_native_fence = EGL_EXTENSION_FUNCTIONS
        .DupNativeFenceFDANDROID
        .filter(|_| supports_native_fences(egl_display))
        .ok_or(Error::RequiredExtensionUnavailable)?;
    let attributes = [egl::NONE as EGLAttrib];
    EGL_FUNCTIONS.with(|egl| unsafe {
        let sync = egl.CreateSync(
            egl_display,
            EGL_SYNC_NATIVE_FENCE_ANDROID,
            attributes.as_ptr(),
        );
        if sync == egl::NO_SYNC {
            return Err(Error::Failed);
        }
        // The fence only gets a file descriptor once it has been flushed to the GPU.
        gl.flush();
        let fd = dup_native_fence(egl_display, sync);
        egl.DestroySync(egl_display, sync);
        if fd == EGL_NO_NATIVE_FENCE_FD_ANDROID {
            return Err(Error::SurfaceExportFailed(
                egl.GetError().to_windowing_api_error(),
            ));
        }
        Ok(OwnedFd::from_raw_fd(fd))
    })
}

/// Has the GPU wait for a sync file descriptor to signal before it runs what the current context
/// is given from now on.
pub(crate) fn wait(egl_display: EGLDisplay, fence: OwnedFd) -> Result<(), Error> {
    if EGL_EXTENSION_FUNCTIONS.DupNativeFenceFDANDROID.is_none()
        || !supports_native_fences(egl_display)
    {
        return Err(Error::RequiredExtensionUnavailable);
    }
    let fd = fence.into_raw_fd();
    let attributes = [
        EGL_SYNC_NATIVE_FENCE_FD_ANDROID as EGLAttrib,
        fd as EGLAttrib,
        egl::NONE as EGLAttrib,
    ];
    EGL_FUNCTIONS.with(|egl| unsafe {
        let sync = egl.CreateSync(
            egl_display,
            EGL_SYNC_NATIVE_FENCE_ANDROID,
            attributes.as_ptr(),
        );
        if sync == egl::NO_SYNC {
            // EGL only takes over the file descriptor when it makes the sync.
            drop(OwnedFd::from_raw_fd(fd));
            return Err(Error::SurfaceImportFailed(
                egl.GetError().to_windowing_api_error(),
            ));
        }
        let waited = egl.WaitSync(egl_display, sync, 0);
        egl.DestroySync(egl_display, sync);
        if waited == egl::TRUE as _ {
            Ok(())
        } else {
            Err(Error::SurfaceImportFailed(WindowingApiError::Failed))
        }
    })
}

fn supports_native_fences(egl_display: EGLDisplay) -> bool {
    display_supports_extension(egl_display, "EGL_ANDROID_native_fence_sync")
}
