//! Functionality common to EGL-based backends.

#![allow(dead_code)]

pub(crate) mod context;
pub(crate) mod device;
#[cfg(free_unix)]
pub(crate) mod dma_buf;
pub(crate) mod error;
pub(crate) mod ffi;
#[cfg(free_unix)]
pub(crate) mod native_fence;
pub(crate) mod surface;
