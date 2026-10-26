//! Functionality common to backends using EGL surfaces.

use super::context::CurrentContextGuard;
use super::device::EGL_FUNCTIONS;
use crate::base::egl::error::ToWindowingApiError;
use crate::base::egl::ffi::{
    EGLClientBuffer, EGLImageKHR, EGL_EXTENSION_FUNCTIONS, EGL_GL_TEXTURE_2D_KHR,
    EGL_IMAGE_PRESERVED_KHR, EGL_NO_IMAGE_KHR,
};
use crate::egl;
use crate::egl::types::{EGLAttrib, EGLConfig, EGLContext, EGLDisplay, EGLSurface, EGLint};
use crate::gl;
use crate::gl_utils;
use crate::renderbuffers::Renderbuffers;
use crate::Gl;
use crate::{ContextAttributes, ContextID, Error, SurfaceID, SurfaceInfo};

use euclid::default::Size2D;
use glow::{Framebuffer, HasContext, PixelUnpackData, Texture};
use std::fmt::{self, Debug, Formatter};
use std::marker::PhantomData;
use std::mem;
use std::os::raw::c_void;
use std::ptr;

#[cfg(angle_metal)]
use crate::base::egl::ffi::{
    EGL_IOSURFACE_ANGLE, EGL_IOSURFACE_PLANE_ANGLE, EGL_TEXTURE_INTERNAL_FORMAT_ANGLE,
    EGL_TEXTURE_TYPE_ANGLE,
};
#[cfg(angle_metal)]
use objc2_core_foundation::CFRetained;
#[cfg(angle_metal)]
use objc2_io_surface::IOSurfaceRef;

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct ExternalEGLSurfaces {
    pub(crate) draw: EGLSurface,
    pub(crate) read: EGLSurface,
}

pub struct EGLBackedSurface {
    pub(crate) context_id: ContextID,
    pub(crate) size: Size2D<i32>,
    pub(crate) objects: EGLSurfaceObjects,
    pub(crate) destroyed: bool,
}

impl Debug for EGLBackedSurface {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "Surface({:x})", self.id().0)
    }
}

unsafe impl Send for EGLBackedSurface {}

#[allow(dead_code)]
pub(crate) enum EGLSurfaceObjects {
    TextureImage {
        egl_image: EGLImageKHR,
        framebuffer_object: Option<Framebuffer>,
        texture_object: Option<Texture>,
        renderbuffers: Renderbuffers,
    },
    Window {
        native_window: *const c_void,
        egl_surface: EGLSurface,
    },
    /// An IOSurface, which ANGLE binds to a texture through a pbuffer.
    #[cfg(angle_metal)]
    IOSurface {
        io_surface: CFRetained<IOSurfaceRef>,
        egl_display: EGLDisplay,
        egl_config: EGLConfig,
        egl_pbuffer: EGLSurface,
        framebuffer_object: Option<Framebuffer>,
        texture_object: Option<Texture>,
        renderbuffers: Renderbuffers,
    },
}

pub(crate) struct EGLSurfaceTexture {
    pub(crate) surface: EGLBackedSurface,
    pub(crate) texture_object: Option<Texture>,
    /// The pbuffer of the surface's IOSurface that the texture is bound to.
    #[cfg(angle_metal)]
    pub(crate) egl_pbuffer: EGLSurface,
    pub(crate) phantom: PhantomData<*const ()>,
}

impl Debug for EGLSurfaceTexture {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        write!(f, "SurfaceTexture({:?})", self.surface)
    }
}

impl EGLBackedSurface {
    pub(crate) fn new_generic(
        gl: &Gl,
        egl_display: EGLDisplay,
        egl_context: EGLContext,
        context_id: ContextID,
        context_attributes: &ContextAttributes,
        size: &Size2D<i32>,
    ) -> EGLBackedSurface {
        let egl_image_attribs = [
            EGL_IMAGE_PRESERVED_KHR as EGLint,
            egl::FALSE as EGLint,
            egl::NONE as EGLint,
            0,
        ];

        unsafe {
            // Create our texture.
            let texture_object = gl.create_texture().ok();
            // Save the current texture binding
            let old_texture_object = gl.get_parameter_texture(gl::TEXTURE_BINDING_2D);
            gl.bind_texture(gl::TEXTURE_2D, texture_object);
            // Unbind PIXEL_UNPACK_BUFFER, because if it is bound,
            // it can cause errors in glTexImage2D.
            // TODO: should this be inside a check for GL 2.0?
            let unpack_buffer = gl.get_parameter_buffer(gl::PIXEL_UNPACK_BUFFER_BINDING);
            if unpack_buffer.is_some() {
                gl.bind_buffer(gl::PIXEL_UNPACK_BUFFER, None);
            }
            gl.tex_image_2d(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                size.width,
                size.height,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                PixelUnpackData::Slice(None),
            );
            // Restore the old bindings
            gl.bind_texture(gl::TEXTURE_2D, old_texture_object);
            if unpack_buffer.is_some() {
                gl.bind_buffer(gl::PIXEL_UNPACK_BUFFER, unpack_buffer);
            }

            // Create our image.
            let egl_client_buffer =
                texture_object.map_or(0, |tex| tex.0.get()) as usize as EGLClientBuffer;
            let egl_image = (EGL_EXTENSION_FUNCTIONS.CreateImageKHR)(
                egl_display,
                egl_context,
                EGL_GL_TEXTURE_2D_KHR,
                egl_client_buffer,
                egl_image_attribs.as_ptr(),
            );

            // Create the framebuffer, and bind the texture to it.
            let framebuffer_object =
                gl_utils::create_and_bind_framebuffer(gl, gl::TEXTURE_2D, texture_object);

            // Bind renderbuffers as appropriate.
            let renderbuffers = Renderbuffers::new(gl, size, context_attributes);
            renderbuffers.bind_to_current_framebuffer(gl);

            debug_assert_eq!(
                gl.check_framebuffer_status(gl::FRAMEBUFFER),
                gl::FRAMEBUFFER_COMPLETE
            );

            EGLBackedSurface {
                context_id,
                size: *size,
                objects: EGLSurfaceObjects::TextureImage {
                    egl_image,
                    framebuffer_object: Some(framebuffer_object),
                    texture_object,
                    renderbuffers,
                },
                destroyed: false,
            }
        }
    }

    pub(crate) fn new_window(
        egl_display: EGLDisplay,
        egl_config: EGLConfig,
        native_window: *mut c_void,
        context_id: ContextID,
        size: &Size2D<i32>,
    ) -> EGLBackedSurface {
        EGL_FUNCTIONS.with(|egl| unsafe {
            let window_surface_attribs = [egl::NONE as EGLAttrib];
            let egl_surface = egl.CreatePlatformWindowSurface(
                egl_display,
                egl_config,
                native_window,
                window_surface_attribs.as_ptr(),
            );
            assert_ne!(egl_surface, egl::NO_SURFACE);

            EGLBackedSurface {
                context_id,
                size: *size,
                objects: EGLSurfaceObjects::Window {
                    native_window,
                    egl_surface,
                },
                destroyed: false,
            }
        })
    }

    /// A generic surface on a new IOSurface, for ANGLE, which cannot make EGL images of textures
    /// on Metal.
    #[cfg(angle_metal)]
    pub(crate) fn new_io_surface(
        gl: &Gl,
        egl_display: EGLDisplay,
        egl_config: EGLConfig,
        context_id: ContextID,
        context_attributes: &ContextAttributes,
        size: &Size2D<i32>,
        access: crate::SurfaceAccess,
    ) -> Result<EGLBackedSurface, Error> {
        let system_device = crate::base::io_surface::device::Device::new(
            crate::base::io_surface::adapter::AppleAdapter {
                is_low_power: false,
            },
        )?;
        let io_surface = system_device.create_io_surface(size, access);

        unsafe {
            let egl_pbuffer = create_io_surface_pbuffer(egl_display, egl_config, &io_surface)?;
            let texture_object = match bind_pbuffer_to_gl_texture(gl, egl_display, egl_pbuffer) {
                Ok(texture_object) => texture_object,
                Err(err) => {
                    EGL_FUNCTIONS.with(|egl| egl.DestroySurface(egl_display, egl_pbuffer));
                    return Err(err);
                }
            };

            let framebuffer_object =
                gl_utils::create_and_bind_framebuffer(gl, gl::TEXTURE_2D, Some(texture_object));
            let renderbuffers = Renderbuffers::new(gl, size, context_attributes);
            renderbuffers.bind_to_current_framebuffer(gl);

            debug_assert_eq!(
                gl.check_framebuffer_status(gl::FRAMEBUFFER),
                gl::FRAMEBUFFER_COMPLETE
            );

            Ok(EGLBackedSurface {
                context_id,
                size: *size,
                objects: EGLSurfaceObjects::IOSurface {
                    io_surface,
                    egl_display,
                    egl_config,
                    egl_pbuffer,
                    framebuffer_object: Some(framebuffer_object),
                    texture_object: Some(texture_object),
                    renderbuffers,
                },
                destroyed: false,
            })
        }
    }

    pub(crate) fn to_surface_texture(
        self,
        gl: &Gl,
    ) -> Result<EGLSurfaceTexture, (Error, EGLBackedSurface)> {
        unsafe {
            let egl_image = match self.objects {
                EGLSurfaceObjects::TextureImage { egl_image, .. } => egl_image,
                EGLSurfaceObjects::Window { .. } => return Err((Error::WidgetAttached, self)),
                #[cfg(angle_metal)]
                EGLSurfaceObjects::IOSurface {
                    ref io_surface,
                    egl_display,
                    egl_config,
                    ..
                } => {
                    // A pbuffer bound to one texture cannot be bound to another, so the surface
                    // texture has a pbuffer of its own.
                    let egl_pbuffer =
                        match create_io_surface_pbuffer(egl_display, egl_config, io_surface) {
                            Ok(egl_pbuffer) => egl_pbuffer,
                            Err(err) => return Err((err, self)),
                        };
                    return match bind_pbuffer_to_gl_texture(gl, egl_display, egl_pbuffer) {
                        Ok(texture_object) => Ok(EGLSurfaceTexture {
                            surface: self,
                            texture_object: Some(texture_object),
                            egl_pbuffer,
                            phantom: PhantomData,
                        }),
                        Err(err) => {
                            EGL_FUNCTIONS.with(|egl| egl.DestroySurface(egl_display, egl_pbuffer));
                            Err((err, self))
                        }
                    };
                }
            };
            let texture_object = bind_egl_image_to_gl_texture(gl, egl_image);
            Ok(EGLSurfaceTexture {
                surface: self,
                texture_object: Some(texture_object),
                #[cfg(angle_metal)]
                egl_pbuffer: egl::NO_SURFACE,
                phantom: PhantomData,
            })
        }
    }

    pub(crate) fn destroy(
        &mut self,
        gl: &Gl,
        egl_display: EGLDisplay,
        context_id: ContextID,
    ) -> Result<Option<*const c_void>, Error> {
        if context_id != self.context_id {
            return Err(Error::IncompatibleSurface);
        }

        unsafe {
            match self.objects {
                EGLSurfaceObjects::TextureImage {
                    ref mut egl_image,
                    ref mut framebuffer_object,
                    ref mut texture_object,
                    ref mut renderbuffers,
                } => {
                    gl.bind_framebuffer(gl::FRAMEBUFFER, None);
                    if let Some(framebuffer) = framebuffer_object.take() {
                        gl.delete_framebuffer(framebuffer);
                    }
                    renderbuffers.destroy(gl);

                    let result = (EGL_EXTENSION_FUNCTIONS.DestroyImageKHR)(egl_display, *egl_image);
                    assert_ne!(result, egl::FALSE);
                    *egl_image = EGL_NO_IMAGE_KHR;

                    if let Some(texture) = texture_object.take() {
                        gl.delete_texture(texture);
                    }

                    self.destroyed = true;
                    Ok(None)
                }
                EGLSurfaceObjects::Window {
                    ref mut egl_surface,
                    ref mut native_window,
                } => {
                    EGL_FUNCTIONS.with(|egl| {
                        egl.DestroySurface(egl_display, *egl_surface);
                        *egl_surface = egl::NO_SURFACE;
                    });

                    self.destroyed = true;
                    Ok(Some(mem::replace(native_window, ptr::null())))
                }
                #[cfg(angle_metal)]
                EGLSurfaceObjects::IOSurface {
                    ref mut egl_pbuffer,
                    ref mut framebuffer_object,
                    ref mut texture_object,
                    ref mut renderbuffers,
                    ..
                } => {
                    gl.bind_framebuffer(gl::FRAMEBUFFER, None);
                    if let Some(framebuffer) = framebuffer_object.take() {
                        gl.delete_framebuffer(framebuffer);
                    }
                    renderbuffers.destroy(gl);

                    if let Some(texture) = texture_object.take() {
                        release_pbuffer_from_gl_texture(gl, egl_display, *egl_pbuffer, texture);
                    }
                    EGL_FUNCTIONS.with(|egl| egl.DestroySurface(egl_display, *egl_pbuffer));
                    *egl_pbuffer = egl::NO_SURFACE;

                    self.destroyed = true;
                    Ok(None)
                }
            }
        }
    }

    // TODO(pcwalton): Damage regions.
    pub(crate) fn present(
        &self,
        egl_display: EGLDisplay,
        egl_context: EGLContext,
    ) -> Result<(), Error> {
        unsafe {
            match self.objects {
                EGLSurfaceObjects::Window { egl_surface, .. } => {
                    // The surface must be bound to the current context in EGL 1.4. Temporarily
                    // make this surface current to enforce this.
                    let _guard = CurrentContextGuard::new();

                    EGL_FUNCTIONS.with(|egl| {
                        let result =
                            egl.MakeCurrent(egl_display, egl_surface, egl_surface, egl_context);
                        if result == egl::FALSE {
                            let err = egl.GetError().to_windowing_api_error();
                            return Err(Error::MakeCurrentFailed(err));
                        }

                        let ok = egl.SwapBuffers(egl_display, egl_surface);
                        if ok != egl::FALSE {
                            Ok(())
                        } else {
                            Err(Error::PresentFailed(
                                egl.GetError().to_windowing_api_error(),
                            ))
                        }
                    })
                }
                EGLSurfaceObjects::TextureImage { .. } => Err(Error::NoWidgetAttached),
                #[cfg(angle_metal)]
                EGLSurfaceObjects::IOSurface { .. } => Err(Error::NoWidgetAttached),
            }
        }
    }

    pub(crate) fn info(&self) -> SurfaceInfo {
        SurfaceInfo {
            size: self.size,
            id: self.id(),
            context_id: self.context_id,
            framebuffer_object: match self.objects {
                EGLSurfaceObjects::TextureImage {
                    framebuffer_object, ..
                } => framebuffer_object,
                EGLSurfaceObjects::Window { .. } => None,
                #[cfg(angle_metal)]
                EGLSurfaceObjects::IOSurface {
                    framebuffer_object, ..
                } => framebuffer_object,
            },
        }
    }

    pub(crate) fn id(&self) -> SurfaceID {
        match self.objects {
            EGLSurfaceObjects::TextureImage { egl_image, .. } => SurfaceID(egl_image as usize),
            EGLSurfaceObjects::Window { egl_surface, .. } => SurfaceID(egl_surface as usize),
            #[cfg(angle_metal)]
            EGLSurfaceObjects::IOSurface { ref io_surface, .. } => {
                SurfaceID(&**io_surface as *const IOSurfaceRef as usize)
            }
        }
    }

    pub(crate) fn native_window(&self) -> Result<*const c_void, Error> {
        match self.objects {
            EGLSurfaceObjects::TextureImage { .. } => Err(Error::NoWidgetAttached),
            EGLSurfaceObjects::Window { native_window, .. } => Ok(native_window),
            #[cfg(angle_metal)]
            EGLSurfaceObjects::IOSurface { .. } => Err(Error::NoWidgetAttached),
        }
    }

    /// The IOSurface holding the surface's pixels.
    #[cfg(angle_metal)]
    pub(crate) fn io_surface(&self) -> Option<CFRetained<IOSurfaceRef>> {
        match self.objects {
            EGLSurfaceObjects::IOSurface { ref io_surface, .. } => Some(io_surface.clone()),
            _ => None,
        }
    }

    pub(crate) fn unbind(&self, gl: &Gl, egl_display: EGLDisplay, egl_context: EGLContext) {
        // If we're current, we stay current, but with no surface attached.
        unsafe {
            EGL_FUNCTIONS.with(|egl| {
                if egl.GetCurrentContext() != egl_context {
                    return;
                }

                egl.MakeCurrent(egl_display, egl::NO_SURFACE, egl::NO_SURFACE, egl_context);

                match self.objects {
                    EGLSurfaceObjects::TextureImage {
                        framebuffer_object: Some(framebuffer_object),
                        ..
                    } => {
                        gl_utils::unbind_framebuffer_if_necessary(gl, framebuffer_object);
                    }
                    #[cfg(angle_metal)]
                    EGLSurfaceObjects::IOSurface {
                        framebuffer_object: Some(framebuffer_object),
                        ..
                    } => {
                        gl_utils::unbind_framebuffer_if_necessary(gl, framebuffer_object);
                    }
                    _ => {}
                }
            })
        }
    }

    pub(crate) fn egl_surfaces(&self) -> ExternalEGLSurfaces {
        match self.objects {
            EGLSurfaceObjects::Window { egl_surface, .. } => ExternalEGLSurfaces {
                draw: egl_surface,
                read: egl_surface,
            },
            EGLSurfaceObjects::TextureImage { .. } => ExternalEGLSurfaces::default(),
            #[cfg(angle_metal)]
            EGLSurfaceObjects::IOSurface { .. } => ExternalEGLSurfaces::default(),
        }
    }

    pub(crate) fn resize(&mut self, size: Size2D<i32>) {
        self.size = size;
    }
}

impl EGLSurfaceTexture {
    pub(crate) fn destroy(mut self, gl: &Gl) -> EGLBackedSurface {
        #[cfg(angle_metal)]
        if let EGLSurfaceObjects::IOSurface { egl_display, .. } = self.surface.objects {
            unsafe {
                if let Some(texture) = self.texture_object.take() {
                    release_pbuffer_from_gl_texture(gl, egl_display, self.egl_pbuffer, texture);
                }
                EGL_FUNCTIONS.with(|egl| egl.DestroySurface(egl_display, self.egl_pbuffer));
            }
        }
        if let Some(texture) = self.texture_object.take() {
            unsafe {
                gl.delete_texture(texture);
            }
        }
        self.surface
    }
}

impl Default for ExternalEGLSurfaces {
    #[inline]
    fn default() -> ExternalEGLSurfaces {
        ExternalEGLSurfaces {
            draw: egl::NO_SURFACE,
            read: egl::NO_SURFACE,
        }
    }
}

#[allow(dead_code)]
pub(crate) unsafe fn create_pbuffer_surface(
    egl_display: EGLDisplay,
    egl_config: EGLConfig,
    size: &Size2D<i32>,
) -> EGLSurface {
    let attributes = [
        egl::WIDTH as EGLint,
        size.width as EGLint,
        egl::HEIGHT as EGLint,
        size.height as EGLint,
        egl::TEXTURE_FORMAT as EGLint,
        egl::TEXTURE_RGBA as EGLint,
        egl::TEXTURE_TARGET as EGLint,
        egl::TEXTURE_2D as EGLint,
        egl::NONE as EGLint,
        0,
        0,
        0,
    ];

    EGL_FUNCTIONS.with(|egl| {
        let egl_surface = egl.CreatePbufferSurface(egl_display, egl_config, attributes.as_ptr());
        assert_ne!(egl_surface, egl::NO_SURFACE);
        egl_surface
    })
}

/// A pbuffer of the BGRA `io_surface`, with `EGL_ANGLE_iosurface_client_buffer`.
#[cfg(angle_metal)]
unsafe fn create_io_surface_pbuffer(
    egl_display: EGLDisplay,
    egl_config: EGLConfig,
    io_surface: &IOSurfaceRef,
) -> Result<EGLSurface, Error> {
    let attributes = [
        egl::WIDTH as EGLint,
        io_surface.width() as EGLint,
        egl::HEIGHT as EGLint,
        io_surface.height() as EGLint,
        EGL_IOSURFACE_PLANE_ANGLE as EGLint,
        0,
        egl::TEXTURE_TARGET as EGLint,
        egl::TEXTURE_2D as EGLint,
        EGL_TEXTURE_INTERNAL_FORMAT_ANGLE as EGLint,
        gl::BGRA as EGLint,
        egl::TEXTURE_FORMAT as EGLint,
        egl::TEXTURE_RGBA as EGLint,
        EGL_TEXTURE_TYPE_ANGLE as EGLint,
        gl::UNSIGNED_BYTE as EGLint,
        egl::NONE as EGLint,
    ];
    EGL_FUNCTIONS.with(|egl| {
        let egl_pbuffer = egl.CreatePbufferFromClientBuffer(
            egl_display,
            EGL_IOSURFACE_ANGLE,
            io_surface as *const IOSurfaceRef as _,
            egl_config,
            attributes.as_ptr(),
        );
        if egl_pbuffer == egl::NO_SURFACE {
            return Err(Error::SurfaceCreationFailed(
                egl.GetError().to_windowing_api_error(),
            ));
        }
        Ok(egl_pbuffer)
    })
}

/// A new texture of the current context that `egl_pbuffer` is bound to.
#[cfg(angle_metal)]
unsafe fn bind_pbuffer_to_gl_texture(
    gl: &Gl,
    egl_display: EGLDisplay,
    egl_pbuffer: EGLSurface,
) -> Result<Texture, Error> {
    let texture = gl.create_texture().unwrap();
    let texture_binding = gl.get_parameter_texture(gl::TEXTURE_BINDING_2D);
    gl.bind_texture(gl::TEXTURE_2D, Some(texture));
    let bound = EGL_FUNCTIONS.with(|egl| {
        if egl.BindTexImage(egl_display, egl_pbuffer, egl::BACK_BUFFER as EGLint) == egl::FALSE {
            return Err(Error::SurfaceTextureCreationFailed(
                egl.GetError().to_windowing_api_error(),
            ));
        }
        Ok(())
    });
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as _);
    gl.bind_texture(gl::TEXTURE_2D, texture_binding);
    match bound {
        Ok(()) => Ok(texture),
        Err(err) => {
            gl.delete_texture(texture);
            Err(err)
        }
    }
}

#[cfg(angle_metal)]
unsafe fn release_pbuffer_from_gl_texture(
    gl: &Gl,
    egl_display: EGLDisplay,
    egl_pbuffer: EGLSurface,
    texture: Texture,
) {
    EGL_FUNCTIONS
        .with(|egl| egl.ReleaseTexImage(egl_display, egl_pbuffer, egl::BACK_BUFFER as EGLint));
    gl.delete_texture(texture);
}

#[allow(dead_code)]
pub(crate) unsafe fn bind_egl_image_to_gl_texture(gl: &Gl, egl_image: EGLImageKHR) -> Texture {
    let texture = gl.create_texture().unwrap();

    let texture_binding = gl.get_parameter_texture(gl::TEXTURE_BINDING_2D);

    // FIXME(pcwalton): Should this be `GL_TEXTURE_EXTERNAL_OES`?
    gl.bind_texture(gl::TEXTURE_2D, Some(texture));
    (EGL_EXTENSION_FUNCTIONS.ImageTargetTexture2DOES)(gl::TEXTURE_2D, egl_image);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as _);
    gl.tex_parameter_i32(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as _);
    gl.bind_texture(gl::TEXTURE_2D, texture_binding);

    debug_assert_eq!(gl.get_error(), gl::NO_ERROR);
    texture
}
