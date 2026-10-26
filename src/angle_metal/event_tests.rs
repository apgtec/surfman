//! Tests of the Metal shared events of ANGLE on Metal.

use super::Connection;
use crate::{
    gl, BufferHandle, ContextAttributeFlags, ContextAttributes, GLVersion, Gl, OffscreenSurface,
    SurfaceAccess, SurfaceType,
};

use euclid::default::Size2D;
use glow::HasContext;
use objc2_metal::MTLSharedEvent as _;
use std::ffi::c_void;

extern "C" {
    fn IOSurfaceLock(buffer: *const c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(buffer: *const c_void, options: u32, seed: *mut u32) -> i32;
}

const READ_ONLY: u32 = 1;

// Tests that a Metal shared event signalled after drawing into a buffer surface, swapped out the way
// a swap chain does, reaches its value only once the drawing is in the surface's IOSurface.
#[test]
fn test_metal_shared_event_waits_for_drawing() {
    let connection = Connection::new().unwrap();
    let adapter = connection.create_adapter(Default::default()).unwrap();
    let device = connection.create_device(&adapter).unwrap();
    let descriptor = device
        .create_context_descriptor(&ContextAttributes {
            version: GLVersion::new(3, 0),
            flags: ContextAttributeFlags::empty(),
        })
        .unwrap();
    let mut context = device.create_context(&descriptor, None).unwrap();
    let size = Size2D::new(4096, 4096);
    let new_surface = |context: &_| {
        device
            .create_surface(
                context,
                SurfaceAccess::GPUOnly,
                SurfaceType::Generic { size },
            )
            .unwrap()
    };

    unsafe {
        // A large red source, which each frame blits many times to keep the GPU busy.
        let source = new_surface(&context);
        device
            .bind_surface_to_context(&mut context, source)
            .unwrap();
        device.make_context_current(&context).unwrap();
        let gl = Gl::from_loader_function(|symbol| device.get_proc_address(&context, symbol));
        let surface_framebuffer = |context: &_| {
            device
                .context_surface_info(context)
                .unwrap()
                .unwrap()
                .framebuffer_object
        };
        gl.bind_framebuffer(gl::FRAMEBUFFER, surface_framebuffer(&context));
        gl.viewport(0, 0, size.width, size.height);
        gl.clear_color(1.0, 0.0, 0.0, 1.0);
        gl.clear(gl::COLOR_BUFFER_BIT);
        let source: OffscreenSurface = device
            .unbind_surface_from_context(&mut context)
            .unwrap()
            .unwrap()
            .try_into()
            .unwrap();
        let source_texture = device.create_surface_texture(&mut context, source).unwrap();
        let read_framebuffer = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(gl::READ_FRAMEBUFFER, Some(read_framebuffer));
        gl.framebuffer_texture_2d(
            gl::READ_FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            device.surface_gl_texture_target(),
            device.surface_texture_object(&source_texture),
            0,
        );

        let mut spare = vec![new_surface(&context)];
        let first = new_surface(&context);
        device.bind_surface_to_context(&mut context, first).unwrap();
        let mut event = None;
        let mut early = Vec::new();
        let mut failed = Vec::new();
        for frame in 1..=60u64 {
            let draw_framebuffer = surface_framebuffer(&context);
            gl.bind_framebuffer(gl::DRAW_FRAMEBUFFER, draw_framebuffer);
            gl.bind_framebuffer(gl::READ_FRAMEBUFFER, Some(read_framebuffer));
            for _ in 0..40 {
                gl.blit_framebuffer(
                    0,
                    0,
                    size.width,
                    size.height,
                    0,
                    0,
                    size.width,
                    size.height,
                    gl::COLOR_BUFFER_BIT,
                    gl::NEAREST,
                );
            }
            gl.bind_framebuffer(gl::FRAMEBUFFER, draw_framebuffer);
            let green = frame as u8 * 4;
            gl.clear_color(0.0, green as f32 / 255.0, 1.0, 1.0);
            gl.clear(gl::COLOR_BUFFER_BIT);

            // Swapped out as a swap chain does: the drawn surface out, the next one in.
            let drawn = device
                .unbind_surface_from_context(&mut context)
                .unwrap()
                .unwrap();
            device
                .bind_surface_to_context(&mut context, spare.pop().unwrap())
                .unwrap();

            let angle_metal_device = device.angle_metal().unwrap();
            let signalled = match angle_metal_device.signal_metal_shared_event(
                &context,
                event.as_deref(),
                frame,
            ) {
                Ok(signalled) => signalled,
                Err(error) => {
                    failed.push((frame, error));
                    spare.push(drawn);
                    continue;
                }
            };
            while signalled.signaledValue() < frame {}
            event = Some(signalled);

            let drawn: OffscreenSurface = drawn.try_into().unwrap();
            let BufferHandle::IoSurface(io_surface) = device.buffer_handle(&drawn).unwrap();
            let io_surface_pointer = &*io_surface as *const _ as *const c_void;
            IOSurfaceLock(io_surface_pointer, READ_ONLY, std::ptr::null_mut());
            let base = io_surface.base_address().as_ptr() as *const u8;
            let pixel = std::slice::from_raw_parts(
                base.add(2048 * io_surface.bytes_per_row() + 2048 * 4),
                4,
            )
            .to_vec();
            IOSurfaceUnlock(io_surface_pointer, READ_ONLY, std::ptr::null_mut());
            // BGRA.
            if pixel != [255, green, 0, 255] {
                early.push((frame, pixel));
            }
            spare.push(drawn.into());
        }
        assert!(
            failed.is_empty(),
            "{} of 60 signals failed: {failed:?}",
            failed.len()
        );
        assert!(
            early.is_empty(),
            "{} of 60 frames read before their drawing landed: {early:?}",
            early.len()
        );

        let bound = device.unbind_surface_from_context(&mut context).unwrap();
        for mut surface in spare.into_iter().chain(bound) {
            device.destroy_surface(&mut context, &mut surface).unwrap();
        }
        let mut source = device
            .destroy_surface_texture(&mut context, source_texture)
            .unwrap()
            .into();
        device.destroy_surface(&mut context, &mut source).unwrap();
        device.destroy_context(&mut context).unwrap();
    }
}
