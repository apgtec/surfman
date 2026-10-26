//! Functionality common to backends using EGL displays.

use crate::egl;
use crate::egl::types::{EGLDisplay, EGLint};
use crate::egl::Egl;

#[cfg(not(target_os = "windows"))]
use libc::{dlopen, dlsym, RTLD_LAZY};
use std::ffi::{CStr, CString};
use std::mem;
use std::os::raw::c_void;
use std::ptr;
use std::sync::LazyLock;
#[cfg(target_os = "windows")]
use winapi::shared::minwindef::HMODULE;
#[cfg(target_os = "windows")]
use winapi::um::libloaderapi;

thread_local! {
    pub static EGL_FUNCTIONS: Egl = Egl::load_with(get_proc_address);
}

#[cfg(target_os = "windows")]
static EGL_LIBRARY: LazyLock<EGLLibraryWrapper> = LazyLock::new(|| unsafe {
    let module = libloaderapi::LoadLibraryA(c"libEGL.dll".as_ptr());
    EGLLibraryWrapper(module)
});

#[cfg(target_env = "ohos")]
static EGL_POTENTIAL_SO_NAMES: [&CStr; 1] = [c"libEGL.so"];

#[cfg(not(any(target_os = "windows", target_os = "macos", target_env = "ohos")))]
static EGL_POTENTIAL_SO_NAMES: [&CStr; 2] = [c"libEGL.so.1", c"libEGL.so"];

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
static EGL_LIBRARY: LazyLock<EGLLibraryWrapper> = LazyLock::new(|| {
    for soname in EGL_POTENTIAL_SO_NAMES {
        unsafe {
            let handle = dlopen(soname.as_ptr(), RTLD_LAZY);
            if !handle.is_null() {
                return EGLLibraryWrapper(handle);
            }
        }
    }
    panic!("Unable to load the libEGL shared object");
});

// ANGLE's, which is optional: only the ANGLE backend uses it, and it fails to connect without it.
#[cfg(target_os = "macos")]
static EGL_LIBRARY: LazyLock<EGLLibraryWrapper> = LazyLock::new(|| {
    for name in [c"@rpath/libEGL.dylib", c"libEGL.dylib"] {
        unsafe {
            let handle = dlopen(name.as_ptr(), RTLD_LAZY);
            if !handle.is_null() {
                return EGLLibraryWrapper(handle);
            }
        }
    }
    EGLLibraryWrapper(std::ptr::null_mut())
});

/// Whether ANGLE's `libEGL.dylib` could be loaded.
#[cfg(target_os = "macos")]
pub(crate) fn egl_library_loaded() -> bool {
    !EGL_LIBRARY.0.is_null()
}

#[cfg(target_os = "windows")]
struct EGLLibraryWrapper(HMODULE);
#[cfg(not(target_os = "windows"))]
struct EGLLibraryWrapper(*mut c_void);

unsafe impl Send for EGLLibraryWrapper {}
unsafe impl Sync for EGLLibraryWrapper {}

#[cfg(target_env = "ohos")]
static GLES_POTENTIAL_SO_NAMES: [&CStr; 1] = [c"libGLESv3.so"];

#[cfg(not(any(target_os = "windows", target_os = "macos", target_env = "ohos")))]
static GLES_POTENTIAL_SO_NAMES: [&CStr; 2] = [c"libGLESv2.so.2", c"libGLESv2.so"];

/// The GLES client library, or null if none could be loaded.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
static GLES_LIBRARY: LazyLock<GLESLibraryWrapper> = LazyLock::new(|| {
    for soname in GLES_POTENTIAL_SO_NAMES {
        unsafe {
            let handle = dlopen(soname.as_ptr(), RTLD_LAZY);
            if !handle.is_null() {
                return GLESLibraryWrapper(handle);
            }
        }
    }
    GLESLibraryWrapper(ptr::null_mut())
});

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
struct GLESLibraryWrapper(*mut c_void);

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
unsafe impl Send for GLESLibraryWrapper {}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
unsafe impl Sync for GLESLibraryWrapper {}

#[cfg(target_os = "windows")]
fn get_proc_address(symbol_name: &str) -> *const c_void {
    unsafe {
        let symbol_name: CString = CString::new(symbol_name).unwrap();
        let symbol_ptr = symbol_name.as_ptr();
        libloaderapi::GetProcAddress(EGL_LIBRARY.0, symbol_ptr).cast()
    }
}

#[cfg(not(target_os = "windows"))]
fn get_proc_address(symbol_name: &str) -> *const c_void {
    unsafe {
        if EGL_LIBRARY.0.is_null() {
            return std::ptr::null();
        }
        let symbol_name: CString = CString::new(symbol_name).unwrap();
        let symbol_ptr = symbol_name.as_ptr();
        dlsym(EGL_LIBRARY.0, symbol_ptr).cast_const()
    }
}

/// Looks up a symbol exported by the GLES client library; null if it is not
/// found.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(crate) fn lookup_gles_symbol(symbol_name: &CStr) -> *const c_void {
    if GLES_LIBRARY.0.is_null() {
        return ptr::null();
    }
    unsafe { dlsym(GLES_LIBRARY.0, symbol_name.as_ptr()).cast_const() }
}

/// ANGLE reports EGL 1.5 and forces `EGL_KHR_get_all_proc_addresses` on, so
/// no fallback is needed.
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn lookup_gles_symbol(_: &CStr) -> *const c_void {
    ptr::null()
}

pub(crate) unsafe fn lookup_egl_extension(name: &CStr) -> *mut c_void {
    EGL_FUNCTIONS.with(|egl| mem::transmute(egl.GetProcAddress(name.as_ptr())))
}

/// Returns whether `egl_display` supports the EGL extension `name`.
///
/// `eglGetProcAddress` can return functions of extensions that the display does not support, so
/// those functions being present is not enough.
pub(crate) fn display_supports_extension(egl_display: EGLDisplay, name: &str) -> bool {
    EGL_FUNCTIONS.with(|egl| {
        let extensions = unsafe { egl.QueryString(egl_display, egl::EXTENSIONS as EGLint) };
        if extensions.is_null() {
            return false;
        }
        let extensions = unsafe { CStr::from_ptr(extensions) };
        extensions
            .to_bytes()
            .split(|&byte| byte == b' ')
            .any(|extension| extension == name.as_bytes())
    })
}
