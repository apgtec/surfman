//! A wrapper around ANGLE `EGLDisplay`s.

use super::connection::{AngleMetalConnection, NativeConnectionWrapper};
use super::surface::SurfaceDataGuard;
use crate::angle_metal::context::{AngleMetalContext, NativeContext};
use crate::angle_metal::surface::{
    bind_pbuffer_to_gl_texture, create_io_surface_pbuffer, release_pbuffer_from_gl_texture,
    AngleMetalBufferBinding, AngleMetalSurfaceTexture,
};
use crate::base::egl::context::{self, CurrentContextGuard, DefaultFramebuffer, EGLBackedContext};
use crate::base::egl::device::EGL_FUNCTIONS;
use crate::base::egl::ffi::{
    EGL_EXTENSION_FUNCTIONS, EGL_SYNC_METAL_SHARED_EVENT_ANGLE,
    EGL_SYNC_METAL_SHARED_EVENT_OBJECT_ANGLE, EGL_SYNC_METAL_SHARED_EVENT_SIGNALED_ANGLE,
    EGL_SYNC_METAL_SHARED_EVENT_SIGNAL_VALUE_HI_ANGLE,
    EGL_SYNC_METAL_SHARED_EVENT_SIGNAL_VALUE_LO_ANGLE,
};
use crate::base::egl::surface::EGLBackedSurface;
use crate::base::io_surface::adapter::AppleAdapter;
use crate::base::io_surface::device::Device as SystemDevice;
use crate::context::ContextID;
use crate::egl::types::{EGLAttrib, EGLint};
use crate::macos::surface::{IoSurfaceBinding, IoSurfaceSurface};
use crate::renderbuffers::Renderbuffers;
use crate::{
    egl, gl_utils, Adapter, BufferHandle, Context, EglContextDescriptor, OffscreenSurface, Surface,
    SurfaceTexture, WindowSurface,
};
use crate::{gl, ContextDescriptor};
use crate::{ContextAttributes, Gl, SurfaceInfo};
use crate::{Error, GLApi, SurfaceAccess};
use euclid::default::Size2D;
use glow::{HasContext, Texture};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CFRetained;
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::MTLSharedEvent;
use raw_window_handle::WindowHandle;
use std::marker::PhantomData;
use std::os::raw::c_void;
use std::sync::Arc;

const SURFACE_GL_TEXTURE_TARGET: u32 = gl::TEXTURE_2D;

const EGL_OPENGL_ES3_BIT: EGLint = 0x40;

/// A thread-local handle to a device.
///
/// Devices contain most of the relevant surface management methods.
pub struct AngleMetalDevice {
    pub(crate) native_connection: Arc<NativeConnectionWrapper>,
    pub(crate) adapter: AppleAdapter,
}

/// Wraps an adapter.
///
/// On ANGLE, devices and adapters are essentially identical types.
#[derive(Clone)]
pub struct NativeDevice {
    /// The hardware adapter corresponding to this device.
    pub adapter: AppleAdapter,
}

impl AngleMetalDevice {
    #[inline]
    pub(crate) fn new(
        connection: &AngleMetalConnection,
        adapter: &Adapter,
    ) -> Result<AngleMetalDevice, Error> {
        Ok(AngleMetalDevice {
            native_connection: connection.native_connection.clone(),
            adapter: adapter.apple()?.clone(),
        })
    }

    /// Returns the native device corresponding to this device.
    ///
    /// This method is essentially an alias for the `adapter()` method, since there is no explicit
    /// concept of a device on this backend.
    #[inline]
    pub fn native_device(&self) -> NativeDevice {
        NativeDevice {
            adapter: self.adapter.clone(),
        }
    }

    /// Returns the adapter that this device was created with.
    #[inline]
    pub fn adapter(&self) -> Adapter {
        self.adapter.clone().into()
    }

    /// Returns the OpenGL API flavor that this device supports (OpenGL or OpenGL ES).
    #[inline]
    pub fn gl_api(&self) -> GLApi {
        GLApi::GLES
    }

    /// Creates a context descriptor with the given attributes.
    ///
    /// Context descriptors are local to this device.
    #[inline]
    pub fn create_context_descriptor(
        &self,
        attributes: &ContextAttributes,
    ) -> Result<ContextDescriptor, Error> {
        unsafe {
            EglContextDescriptor::new(
                self.native_connection.egl_display,
                attributes,
                &[
                    egl::SURFACE_TYPE as EGLint,
                    egl::PBUFFER_BIT as EGLint,
                    egl::RENDERABLE_TYPE as EGLint,
                    EGL_OPENGL_ES3_BIT,
                    egl::COLOR_BUFFER_TYPE as EGLint,
                    egl::RGB_BUFFER as EGLint,
                ],
            )
            .map(Into::into)
        }
    }

    /// Creates a new OpenGL context.
    ///
    /// The context initially has no surface attached. Until a surface is bound to it, rendering
    /// commands will fail or have no effect.
    #[inline]
    pub fn create_context(
        &self,
        descriptor: &ContextDescriptor,
        share_with: Option<&Context>,
    ) -> Result<Context, Error> {
        let share_with = share_with.map(Context::angle_metal).transpose()?;
        unsafe {
            let mut context = EGLBackedContext::new(
                self.native_connection.egl_display,
                descriptor.egl()?,
                share_with.map(|ctx| &ctx.0),
                self.gl_api(),
                DefaultFramebuffer::DummyPbuffer,
            )?;
            if let Err(error) = context.make_current(self.native_connection.egl_display) {
                context.destroy(self.native_connection.egl_display);
                return Err(error);
            }
            Ok(
                AngleMetalContext(context, Gl::from_loader_function(context::get_proc_address))
                    .into(),
            )
        }
    }

    /// Wraps an `EGLContext` in a native context and returns it. The context must be current.
    ///
    /// The context is not retained, as there is no way to do this in the EGL API. Therefore,
    /// it is the caller's responsibility to ensure that the returned `Context` object remains
    /// alive as long as the `EGLContext` is.
    #[inline]
    pub unsafe fn create_context_from_native_context(
        &self,
        native_context: NativeContext,
    ) -> Result<Context, Error> {
        Ok(AngleMetalContext(
            EGLBackedContext::from_native_context(native_context),
            Gl::from_loader_function(context::get_proc_address),
        )
        .into())
    }

    /// Destroys a context.
    ///
    /// The context must have been created on this device.
    pub fn destroy_context(&self, context: &mut Context) -> Result<(), Error> {
        if let Ok(Some(surface)) = self.unbind_surface_from_context(context) {
            let mut surface: OffscreenSurface = match surface {
                Surface::Offscreen(surface) => surface,
                Surface::Window(_) => unreachable!("ANGLE on Metal has no window surfaces"),
            };
            self.destroy_offscreen_surface(context, &mut surface)?;
        }

        let context: &mut AngleMetalContext = context.try_into()?;
        unsafe {
            context.0.destroy(self.native_connection.egl_display);
            Ok(())
        }
    }

    /// Given a context, returns its underlying EGL context and attached surfaces.
    #[inline]
    pub fn native_context(&self, context: &Context) -> NativeContext {
        let context = context
            .angle_metal()
            .expect("Passed incorrect context type");
        context.0.native_context()
    }

    /// Returns the descriptor that this context was created with.
    #[inline]
    pub fn context_descriptor(&self, context: &Context) -> ContextDescriptor {
        let context = context
            .angle_metal()
            .expect("Passed incorrect context type");
        unsafe {
            EglContextDescriptor::from_egl_context(
                &context.1,
                self.native_connection.egl_display,
                context.0.egl_context,
            )
            .into()
        }
    }

    /// Makes the context the current OpenGL context for this thread.
    ///
    /// After calling this function, it is valid to use OpenGL rendering commands.
    #[inline]
    pub fn make_context_current(&self, context: &Context) -> Result<(), Error> {
        let context = context.angle_metal()?;
        unsafe { context.0.make_current(self.native_connection.egl_display) }
    }

    /// Removes the current OpenGL context from this thread.
    ///
    /// After calling this function, OpenGL rendering commands will fail until a new context is
    /// made current.
    #[inline]
    pub fn make_no_context_current(&self) -> Result<(), Error> {
        unsafe { context::make_no_context_current(self.native_connection.egl_display) }
    }

    #[inline]
    pub(crate) fn temporarily_make_context_current(
        &self,
        context: &Context,
    ) -> Result<CurrentContextGuard, Error> {
        let guard = CurrentContextGuard::new();
        self.make_context_current(context)?;
        Ok(guard)
    }

    /// Returns the attributes that the context descriptor was created with.
    #[inline]
    pub fn context_descriptor_attributes(
        &self,
        context_descriptor: &ContextDescriptor,
    ) -> ContextAttributes {
        let context_descriptor = context_descriptor
            .egl()
            .expect("Passed incompatible context descriptor");
        unsafe { context_descriptor.attributes(self.native_connection.egl_display) }
    }

    /// Fetches the address of an OpenGL function associated with this context.
    ///
    /// OpenGL functions are local to a context. You should not use OpenGL functions on one context
    /// with any other context.
    ///
    /// This method is typically used with a function like `gl::load_with()` from the `gl` crate to
    /// load OpenGL function pointers.
    #[inline]
    pub fn get_proc_address(&self, _: &Context, symbol_name: &str) -> *const c_void {
        context::get_proc_address(symbol_name)
    }

    /// See [`crate::Device::bind_surface_to_context`].
    pub fn bind_surface_to_context(
        &self,
        context: &mut Context,
        surface: Surface,
    ) -> Result<Option<Surface>, (Error, Surface)> {
        let context: &mut AngleMetalContext = match context.try_into() {
            Ok(context) => context,
            Err(error) => return Err((error, surface)),
        };
        let surface = match surface {
            Surface::Offscreen(OffscreenSurface::IoSurface(surface))
                if surface.angle_metal().is_ok() =>
            {
                surface
            }
            surface => return Err((Error::IncompatibleSurface, surface)),
        };
        unsafe {
            context
                .0
                .bind_surface(
                    &context.1,
                    self.native_connection.egl_display,
                    EGLBackedSurface::IoSurface(surface),
                )
                .map(|surface| surface.map(egl_surface_into_surface))
                .map_err(|(error, surface)| (error, egl_surface_into_surface(surface)))
        }
    }

    /// Removes and returns any attached surface from this context.
    ///
    /// Any pending OpenGL commands targeting this surface will be automatically flushed, so the
    /// surface is safe to read from immediately when this function returns.
    pub fn unbind_surface_from_context(
        &self,
        context: &mut Context,
    ) -> Result<Option<Surface>, Error> {
        let context: &mut AngleMetalContext = context.try_into()?;
        unsafe {
            context
                .0
                .unbind_surface(&context.1, self.native_connection.egl_display)
                .map(|maybe_surface| maybe_surface.map(egl_surface_into_surface))
        }
    }

    /// Displays the contents of the currently bound surface to the screen, if
    /// it is a widget surface.
    ///
    /// Widget surfaces are internally double-buffered, so changes to them don't
    /// show up in their associated widgets until this method is called.
    pub fn present_bound_surface(&self, context: &mut Context) -> Result<(), Error> {
        let context: &mut AngleMetalContext = context.try_into()?;
        context
            .0
            .present_bound_surface(self.native_connection.egl_display)
    }

    /// Resizes a widget surface.
    pub fn resize_bound_surface(
        &self,
        context: &mut Context,
        size: Size2D<i32>,
    ) -> Result<(), Error> {
        let context: &mut AngleMetalContext = context.try_into()?;
        context.0.resize_bound_surface(size)
    }

    /// Returns a unique ID representing a context.
    ///
    /// This ID is unique to all currently-allocated contexts. If you destroy a context and create
    /// a new one, the new context might have the same ID as the destroyed one.
    #[inline]
    pub fn context_id(&self, context: &Context) -> ContextID {
        let context = context
            .angle_metal()
            .expect("Passed incorrect context type");
        context.0.id
    }

    /// Returns various information about the surface attached to a context.
    ///
    /// This includes, most notably, the OpenGL framebuffer object needed to render to the surface.
    #[inline]
    pub fn context_surface_info(&self, context: &Context) -> Result<Option<SurfaceInfo>, Error> {
        let context = context.angle_metal()?;
        context.0.surface_info()
    }

    /// See [`crate::Device::create_window_surface`].
    ///
    /// ANGLE on Metal has no window surfaces, so this always returns an
    /// `UnsupportedOnThisPlatform` error.
    pub fn create_window_surface(
        &self,
        _: &Context,
        _: SurfaceAccess,
        _: WindowHandle<'_>,
        _: Size2D<i32>,
    ) -> Result<WindowSurface, Error> {
        Err(Error::UnsupportedOnThisPlatform)
    }

    /// See [`crate::Device::create_offscreen_surface`].
    pub fn create_offscreen_surface(
        &self,
        context: &Context,
        access: SurfaceAccess,
        size: Size2D<i32>,
    ) -> Result<OffscreenSurface, Error> {
        self.create_generic_surface(context, access, &size)
    }

    fn create_generic_surface(
        &self,
        context: &Context,
        access: SurfaceAccess,
        size: &Size2D<i32>,
    ) -> Result<OffscreenSurface, Error> {
        let system_device = SystemDevice::new(self.adapter.clone())?;
        let io_surface = system_device.create_io_surface(size, access);
        self.create_surface_from_io_surface(context, io_surface)
            .map(Into::into)
    }

    // A buffer surface on an IOSurface, rendered to through a pbuffer, since ANGLE cannot make
    // EGL images of textures on Metal.
    fn create_surface_from_io_surface(
        &self,
        context: &Context,
        io_surface: CFRetained<IOSurfaceRef>,
    ) -> Result<IoSurfaceSurface, Error> {
        let _guard = self.temporarily_make_context_current(context)?;
        let context_descriptor = self.context_descriptor(context);
        let context_attributes = self.context_descriptor_attributes(&context_descriptor);
        let egl_display = self.native_connection.egl_display;
        let egl_config = unsafe { context_descriptor.egl()?.to_egl_config(egl_display) };
        let context = context.angle_metal()?;
        let gl = &context.1;
        let size = Size2D::new(io_surface.width() as i32, io_surface.height() as i32);

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
            let renderbuffers = Renderbuffers::new(gl, &size, &context_attributes);
            renderbuffers.bind_to_current_framebuffer(gl);

            debug_assert_eq!(
                gl.check_framebuffer_status(gl::FRAMEBUFFER),
                gl::FRAMEBUFFER_COMPLETE
            );

            Ok(IoSurfaceSurface {
                io_surface,
                binding: IoSurfaceBinding::AngleMetal(AngleMetalBufferBinding {
                    context_id: context.0.id,
                    egl_pbuffer,
                    framebuffer_object: Some(framebuffer_object),
                    texture_object: Some(texture_object),
                    renderbuffers,
                    destroyed: false,
                }),
            })
        }
    }

    /// Creates a surface texture from an existing buffer surface for use with the given context.
    ///
    /// The surface texture is local to the supplied context and takes ownership of the surface.
    /// Destroying the surface texture allows you to retrieve the surface again.
    ///
    /// *The supplied context does not have to be the same context that the surface is associated
    /// with.* This allows you to render to a surface in one context and sample from that surface
    /// in another context.
    pub fn create_surface_texture(
        &self,
        context: &Context,
        surface: OffscreenSurface,
    ) -> Result<SurfaceTexture, (Error, OffscreenSurface)> {
        let surface: IoSurfaceSurface = surface.try_into()?;
        let _guard = match self.temporarily_make_context_current(context) {
            Ok(guard) => guard,
            Err(err) => return Err((err, surface.into())),
        };

        let egl_display = self.native_connection.egl_display;
        let context_descriptor = self.context_descriptor(context);
        let egl_config = match context_descriptor.egl() {
            Ok(descriptor) => unsafe { descriptor.to_egl_config(egl_display) },
            Err(error) => return Err((error, surface.into())),
        };
        let context = match context.angle_metal() {
            Ok(context) => context,
            Err(error) => return Err((error, surface.into())),
        };

        unsafe {
            let egl_pbuffer =
                match create_io_surface_pbuffer(egl_display, egl_config, &surface.io_surface) {
                    Ok(egl_pbuffer) => egl_pbuffer,
                    Err(err) => return Err((err, surface.into())),
                };
            match bind_pbuffer_to_gl_texture(&context.1, egl_display, egl_pbuffer) {
                Ok(texture_object) => Ok(AngleMetalSurfaceTexture {
                    surface,
                    egl_pbuffer,
                    texture_object: Some(texture_object),
                    phantom: PhantomData,
                }
                .into()),
                Err(err) => {
                    EGL_FUNCTIONS.with(|egl| egl.DestroySurface(egl_display, egl_pbuffer));
                    Err((err, surface.into()))
                }
            }
        }
    }

    /// See [`crate::Device::destroy_window_surface`].
    ///
    /// ANGLE on Metal has no window surfaces, so this always returns an `IncompatibleSurface`
    /// error.
    pub fn destroy_window_surface(&self, _: &Context, _: &mut WindowSurface) -> Result<(), Error> {
        Err(Error::IncompatibleSurface)
    }

    /// See [`crate::Device::destroy_offscreen_surface`].
    pub fn destroy_offscreen_surface(
        &self,
        context: &Context,
        surface: &mut OffscreenSurface,
    ) -> Result<(), Error> {
        let egl_display = self.native_connection.egl_display;
        let context = context.angle_metal()?;
        let surface: &mut IoSurfaceSurface = surface.try_into()?;
        let binding = surface.angle_metal_mut()?;
        if binding.context_id != context.0.id {
            return Err(Error::IncompatibleSurface);
        }

        let gl = &context.1;
        unsafe {
            gl.bind_framebuffer(gl::FRAMEBUFFER, None);
            if let Some(framebuffer) = binding.framebuffer_object.take() {
                gl.delete_framebuffer(framebuffer);
            }
            binding.renderbuffers.destroy(gl);

            if let Some(texture) = binding.texture_object.take() {
                release_pbuffer_from_gl_texture(gl, egl_display, binding.egl_pbuffer, texture);
            }
            EGL_FUNCTIONS.with(|egl| egl.DestroySurface(egl_display, binding.egl_pbuffer));
            binding.egl_pbuffer = egl::NO_SURFACE;
        }

        binding.destroyed = true;
        Ok(())
    }

    /// Destroys a surface texture and returns the underlying surface.
    ///
    /// The supplied context must be the same context the surface texture was created with, or an
    /// `IncompatibleSurfaceTexture` error is returned.
    ///
    /// All surface textures must be explicitly destroyed with this function, or a panic will
    /// occur.
    pub fn destroy_surface_texture(
        &self,
        context: &Context,
        surface_texture: SurfaceTexture,
    ) -> Result<OffscreenSurface, (Error, SurfaceTexture)> {
        let mut surface_texture: AngleMetalSurfaceTexture = surface_texture.try_into()?;
        let _guard = match self.temporarily_make_context_current(context) {
            Ok(guard) => guard,
            Err(err) => return Err((err, surface_texture.into())),
        };
        let context = match context.angle_metal() {
            Ok(context) => context,
            Err(error) => return Err((error, surface_texture.into())),
        };

        let egl_display = self.native_connection.egl_display;
        unsafe {
            if let Some(texture) = surface_texture.texture_object.take() {
                release_pbuffer_from_gl_texture(
                    &context.1,
                    egl_display,
                    surface_texture.egl_pbuffer,
                    texture,
                );
            }
            EGL_FUNCTIONS.with(|egl| egl.DestroySurface(egl_display, surface_texture.egl_pbuffer));
        }
        Ok(surface_texture.surface.into())
    }

    /// Displays the contents of a window surface on screen.
    ///
    /// ANGLE on Metal has no window surfaces, so this always returns an `IncompatibleSurface`
    /// error.
    pub fn present_surface(&self, _: &Context, _: &mut WindowSurface) -> Result<(), Error> {
        Err(Error::IncompatibleSurface)
    }

    /// Resizes a window surface.
    ///
    /// ANGLE on Metal has no window surfaces, so this always returns an `IncompatibleSurface`
    /// error.
    pub fn resize_surface(
        &self,
        _: &Context,
        _: &mut WindowSurface,
        _: Size2D<i32>,
    ) -> Result<(), Error> {
        Err(Error::IncompatibleSurface)
    }

    /// Returns a pointer to the underlying surface data for reading or writing by the CPU.
    #[inline]
    pub fn lock_surface_data<'s>(&self, _: &'s mut Surface) -> Result<SurfaceDataGuard<'s>, Error> {
        Err(Error::Unimplemented)
    }

    /// Returns the OpenGL texture target needed to read from this surface texture.
    ///
    /// This will be `GL_TEXTURE_2D` or `GL_TEXTURE_RECTANGLE`, depending on platform.
    #[inline]
    pub fn surface_gl_texture_target(&self) -> u32 {
        SURFACE_GL_TEXTURE_TARGET
    }

    /// Returns the OpenGL texture object containing the contents of this surface.
    ///
    /// It is only legal to read from, not write to, this texture object.
    #[inline]
    pub fn surface_texture_object(&self, surface_texture: &SurfaceTexture) -> Option<Texture> {
        surface_texture.angle_metal().ok()?.texture_object
    }

    /// Returns the `IOSurface` behind a buffer surface, which other devices, graphics APIs or
    /// processes can use.
    pub fn buffer_handle(&self, surface: &OffscreenSurface) -> Result<BufferHandle, Error> {
        Ok(BufferHandle::IoSurface(surface.io_surface()?.io_surface()))
    }

    /// Creates a buffer surface of the given context that renders into an `IOSurface`.
    pub fn import_buffer_handle(
        &self,
        context: &Context,
        handle: &BufferHandle,
    ) -> Result<OffscreenSurface, Error> {
        let BufferHandle::IoSurface(io_surface) = handle;
        self.create_surface_from_io_surface(context, io_surface.clone())
            .map(Into::into)
    }

    /// Has the GPU signal `event`, or a new event without one, with `value` once it has finished
    /// what `context` has been given so far, and submits that. Returns the event, which Metal
    /// command buffers can then wait on instead of the CPU waiting for the context.
    pub fn signal_metal_shared_event(
        &self,
        context: &Context,
        event: Option<&ProtocolObject<dyn MTLSharedEvent>>,
        value: u64,
    ) -> Result<Retained<ProtocolObject<dyn MTLSharedEvent>>, Error> {
        let copy_event = EGL_EXTENSION_FUNCTIONS
            .CopyMetalSharedEventANGLE
            .ok_or(Error::RequiredExtensionUnavailable)?;
        let _guard = self.temporarily_make_context_current(context)?;
        let context = context.angle_metal()?;
        let egl_display = self.native_connection.egl_display;
        let mut attributes = vec![
            EGL_SYNC_METAL_SHARED_EVENT_SIGNAL_VALUE_LO_ANGLE as EGLAttrib,
            (value & 0xffff_ffff) as EGLAttrib,
            EGL_SYNC_METAL_SHARED_EVENT_SIGNAL_VALUE_HI_ANGLE as EGLAttrib,
            (value >> 32) as EGLAttrib,
        ];
        // ANGLE rejects a null event, and makes one when the attribute is left out.
        if let Some(event) = event {
            attributes.push(EGL_SYNC_METAL_SHARED_EVENT_OBJECT_ANGLE as EGLAttrib);
            attributes.push(event as *const _ as *const c_void as EGLAttrib);
        }
        attributes.push(egl::NONE as EGLAttrib);
        unsafe {
            let event = EGL_FUNCTIONS.with(|egl| {
                let sync = egl.CreateSync(
                    egl_display,
                    EGL_SYNC_METAL_SHARED_EVENT_ANGLE,
                    attributes.as_ptr(),
                );
                if sync == egl::NO_SYNC {
                    return Err(Error::Failed);
                }
                // The signal is queued with the context's work, which outlives the sync.
                let event = copy_event(egl_display, sync);
                egl.DestroySync(egl_display, sync);
                Ok(event)
            })?;
            context.1.flush();
            Retained::from_raw(event.cast()).ok_or(Error::Failed)
        }
    }

    /// Has the GPU wait for `event` to reach `value` before it runs what `context` is given from
    /// now on, such as until another API's command buffers are done with what it draws into.
    pub fn wait_for_metal_shared_event(
        &self,
        context: &Context,
        event: &ProtocolObject<dyn MTLSharedEvent>,
        value: u64,
    ) -> Result<(), Error> {
        let _guard = self.temporarily_make_context_current(context)?;
        let egl_display = self.native_connection.egl_display;
        let attributes = [
            EGL_SYNC_METAL_SHARED_EVENT_OBJECT_ANGLE as EGLAttrib,
            event as *const _ as *const c_void as EGLAttrib,
            EGL_SYNC_METAL_SHARED_EVENT_SIGNAL_VALUE_LO_ANGLE as EGLAttrib,
            (value & 0xffff_ffff) as EGLAttrib,
            EGL_SYNC_METAL_SHARED_EVENT_SIGNAL_VALUE_HI_ANGLE as EGLAttrib,
            (value >> 32) as EGLAttrib,
            egl::SYNC_CONDITION as EGLAttrib,
            EGL_SYNC_METAL_SHARED_EVENT_SIGNALED_ANGLE as EGLAttrib,
            egl::NONE as EGLAttrib,
        ];
        EGL_FUNCTIONS.with(|egl| unsafe {
            let sync = egl.CreateSync(
                egl_display,
                EGL_SYNC_METAL_SHARED_EVENT_ANGLE,
                attributes.as_ptr(),
            );
            if sync == egl::NO_SYNC {
                return Err(Error::Failed);
            }
            let waited = egl.WaitSync(egl_display, sync, 0);
            egl.DestroySync(egl_display, sync);
            if waited == egl::TRUE as _ {
                Ok(())
            } else {
                Err(Error::Failed)
            }
        })
    }
}

fn egl_surface_into_surface(surface: EGLBackedSurface) -> Surface {
    match surface {
        EGLBackedSurface::IoSurface(surface) => Surface::Offscreen(surface.into()),
        EGLBackedSurface::Window(_) | EGLBackedSurface::Buffer(_) => {
            unreachable!("ANGLE on Metal contexts only have IOSurface surfaces")
        }
    }
}
