//! Native handles to the memory of buffer surfaces, for sharing it with other devices, graphics
//! APIs and processes.

#[cfg(free_unix)]
use std::os::fd::OwnedFd;

#[cfg(any(android_platform, ohos_platform))]
pub use crate::hardware_buffer::surface::NativeHardwareBuffer;
#[cfg(macos_platform)]
use objc2_core_foundation::CFRetained;
#[cfg(macos_platform)]
use objc2_io_surface::IOSurfaceRef;
#[cfg(windows_platform)]
use winapi::um::winnt::HANDLE;

/// A native handle to the memory of a buffer surface.
///
/// A handle comes from [`crate::Device::buffer_handle`] and can be imported into another device
/// with [`crate::Device::import_buffer_handle`], or into another graphics API that understands
/// the native handle. The handle refers to the same memory as the surface: whatever is rendered
/// to one is visible through the other, so access from different devices or APIs must be ordered
/// by the caller.
#[derive(Debug)]
pub enum BufferHandle {
    /// An `IOSurface`, on macOS.
    #[cfg(macos_platform)]
    IoSurface(CFRetained<IOSurfaceRef>),
    /// A Linux DMA-BUF.
    #[cfg(free_unix)]
    DmaBuf(DmaBuf),
    /// The DXGI share handle of a Direct3D 11 texture, as returned by
    /// `IDXGIResource::GetSharedHandle`. It is not an NT handle and must not be closed.
    #[cfg(windows_platform)]
    D3DTexture(HANDLE),
    /// An Android `AHardwareBuffer` or an OpenHarmony `OH_NativeBuffer`.
    #[cfg(any(android_platform, ohos_platform))]
    HardwareBuffer(NativeHardwareBuffer),
}

/// A Linux DMA-BUF image: the file descriptors and layout of each of its planes.
#[cfg(free_unix)]
#[derive(Debug)]
pub struct DmaBuf {
    /// The width of the image, in pixels.
    pub width: i32,
    /// The height of the image, in pixels.
    pub height: i32,
    /// The DRM fourcc code of the pixel format.
    pub fourcc: u32,
    /// The DRM format modifier, which describes the tiling of the planes.
    pub modifier: u64,
    /// The planes of the image.
    pub planes: Vec<DmaBufPlane>,
}

/// One plane of a [`DmaBuf`].
#[cfg(free_unix)]
#[derive(Debug)]
pub struct DmaBufPlane {
    /// The DMA-BUF file descriptor that holds the plane.
    pub fd: OwnedFd,
    /// The offset of the plane in the file, in bytes.
    pub offset: u32,
    /// The stride of the plane's rows, in bytes.
    pub stride: u32,
}
