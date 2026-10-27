//! FFI-related functionality common to the various EGL backends.

#![allow(dead_code)]

use crate::egl::types::{
    EGLAttrib, EGLBoolean, EGLContext, EGLDeviceEXT, EGLDisplay, EGLSurface, EGLSync,
};
use crate::egl::types::{EGLenum, EGLint};

use std::os::raw::c_void;
use std::sync::LazyLock;

pub enum EGLClientBufferOpaque {}
pub type EGLClientBuffer = *mut EGLClientBufferOpaque;

pub enum EGLImageKHROpaque {}
pub type EGLImageKHR = *mut EGLImageKHROpaque;

pub const EGL_GL_TEXTURE_2D_KHR: EGLenum = 0x30b1;
pub const EGL_IMAGE_PRESERVED_KHR: EGLenum = 0x30d2;
pub const EGL_CONTEXT_MINOR_VERSION_KHR: EGLenum = 0x30fb;
pub const EGL_CONTEXT_OPENGL_PROFILE_MASK: EGLenum = 0x30fd;
pub const EGL_PLATFORM_DEVICE_EXT: EGLenum = 0x313f;
pub const EGL_NATIVE_BUFFER_ANDROID: EGLenum = 0x3140;
pub const EGL_PLATFORM_X11_KHR: EGLenum = 0x31d5;
pub const EGL_PLATFORM_WAYLAND_KHR: EGLenum = 0x31d8;
pub const EGL_PLATFORM_SURFACELESS_MESA: EGLenum = 0x31dd;
pub const EGL_D3D_TEXTURE_2D_SHARE_HANDLE_ANGLE: EGLenum = 0x3200;
pub const EGL_BAD_DEVICE_EXT: EGLenum = 0x322b;
pub const EGL_DEVICE_EXT: EGLenum = 0x322c;
pub const EGL_D3D11_DEVICE_ANGLE: EGLenum = 0x33a1;
pub const EGL_DXGI_KEYED_MUTEX_ANGLE: EGLenum = 0x33a2;
pub const EGL_D3D_TEXTURE_ANGLE: EGLenum = 0x33a3;
pub const EGL_LINUX_DMA_BUF_EXT: EGLenum = 0x3270;
pub const EGL_LINUX_DRM_FOURCC_EXT: EGLenum = 0x3271;
pub const EGL_DMA_BUF_PLANE_FD_EXT: [EGLenum; 4] = [0x3272, 0x3275, 0x3278, 0x3440];
pub const EGL_DMA_BUF_PLANE_OFFSET_EXT: [EGLenum; 4] = [0x3273, 0x3276, 0x3279, 0x3441];
pub const EGL_DMA_BUF_PLANE_PITCH_EXT: [EGLenum; 4] = [0x3274, 0x3277, 0x327a, 0x3442];
pub const EGL_DMA_BUF_PLANE_MODIFIER_LO_EXT: [EGLenum; 4] = [0x3443, 0x3445, 0x3447, 0x3449];
pub const EGL_DMA_BUF_PLANE_MODIFIER_HI_EXT: [EGLenum; 4] = [0x3444, 0x3446, 0x3448, 0x344a];
pub const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
pub const EGL_SYNC_NATIVE_FENCE_ANDROID: EGLenum = 0x3144;
pub const EGL_SYNC_NATIVE_FENCE_FD_ANDROID: EGLenum = 0x3145;
pub const EGL_NO_NATIVE_FENCE_FD_ANDROID: EGLint = -1;

pub const EGL_NO_DEVICE_EXT: EGLDeviceEXT = 0 as EGLDeviceEXT;
pub const EGL_NO_IMAGE_KHR: EGLImageKHR = 0 as EGLImageKHR;

pub const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT: EGLint = 1;
pub const EGL_CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT: EGLint = 2;

#[allow(non_snake_case)]
pub(crate) struct EGLExtensionFunctions {
    // Ubiquitous extensions assumed to be present
    pub(crate) CreateImageKHR: extern "C" fn(
        dpy: EGLDisplay,
        ctx: EGLContext,
        target: EGLenum,
        buffer: EGLClientBuffer,
        attrib_list: *const EGLint,
    ) -> EGLImageKHR,
    pub(crate) DestroyImageKHR: extern "C" fn(dpy: EGLDisplay, image: EGLImageKHR) -> EGLBoolean,
    pub(crate) ImageTargetTexture2DOES: extern "C" fn(target: EGLenum, image: EGLImageKHR),

    // Optional extensions
    pub(crate) ExportDMABUFImageMESA: Option<
        extern "C" fn(
            dpy: EGLDisplay,
            image: EGLImageKHR,
            fds: *mut i32,
            strides: *mut EGLint,
            offsets: *mut EGLint,
        ) -> EGLBoolean,
    >,
    pub(crate) ExportDMABUFImageQueryMESA: Option<
        extern "C" fn(
            dpy: EGLDisplay,
            image: EGLImageKHR,
            fourcc: *mut i32,
            num_planes: *mut i32,
            modifiers: *mut u64,
        ) -> EGLBoolean,
    >,
    pub(crate) DupNativeFenceFDANDROID:
        Option<extern "C" fn(dpy: EGLDisplay, sync: EGLSync) -> EGLint>,
    pub(crate) CreateDeviceANGLE: Option<
        extern "C" fn(
            device_type: EGLint,
            native_device: *mut c_void,
            attrib_list: *const EGLAttrib,
        ) -> EGLDeviceEXT,
    >,
    pub(crate) GetNativeClientBufferANDROID:
        Option<extern "C" fn(buffer: *const c_void) -> EGLClientBuffer>,
    pub(crate) QueryDeviceAttribEXT: Option<
        extern "C" fn(device: EGLDeviceEXT, attribute: EGLint, value: *mut EGLAttrib) -> EGLBoolean,
    >,
    pub(crate) QueryDisplayAttribEXT: Option<
        extern "C" fn(dpy: EGLDisplay, attribute: EGLint, value: *mut EGLAttrib) -> EGLBoolean,
    >,
    pub(crate) QuerySurfacePointerANGLE: Option<
        extern "C" fn(
            dpy: EGLDisplay,
            surface: EGLSurface,
            attribute: EGLint,
            value: *mut *mut c_void,
        ) -> EGLBoolean,
    >,
}

pub(crate) static EGL_EXTENSION_FUNCTIONS: LazyLock<EGLExtensionFunctions> = LazyLock::new(|| {
    use crate::base::egl::device::lookup_egl_extension as get;
    use std::mem::transmute as cast;
    unsafe {
        EGLExtensionFunctions {
            CreateImageKHR: cast(get(c"eglCreateImageKHR")),
            DestroyImageKHR: cast(get(c"eglDestroyImageKHR")),
            ImageTargetTexture2DOES: cast(get(c"glEGLImageTargetTexture2DOES")),

            ExportDMABUFImageMESA: cast(get(c"eglExportDMABUFImageMESA")),
            ExportDMABUFImageQueryMESA: cast(get(c"eglExportDMABUFImageQueryMESA")),
            DupNativeFenceFDANDROID: cast(get(c"eglDupNativeFenceFDANDROID")),
            CreateDeviceANGLE: cast(get(c"eglCreateDeviceANGLE")),
            GetNativeClientBufferANDROID: cast(get(c"eglGetNativeClientBufferANDROID")),
            QueryDeviceAttribEXT: cast(get(c"eglQueryDeviceAttribEXT")),
            QueryDisplayAttribEXT: cast(get(c"eglQueryDisplayAttribEXT")),
            QuerySurfacePointerANGLE: cast(get(c"eglQuerySurfacePointerANGLE")),
        }
    }
});
