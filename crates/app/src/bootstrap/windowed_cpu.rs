//! Windowed CPU composition backend: [`SoftwareRenderer`] behind [`NativeRenderBackend`].
//!
//! Donor provenance: `src/render/cpu/commands.ts` (`CpuRenderTarget.execute`
//! ordering). Serial commands dispatch exactly like the reference CPU
//! target: draw buffer selection, per-view before-view operations, view
//! state, then view operations. The renderer swap finishes the software
//! frame and pushes `software_pixels` through the window's
//! `present_pixels` onto an SDL CPU window, so captures and presentation
//! share the reference CPU framebuffer.

use qa_client::render::cpu::rasterizer::SoftwareRenderer;
use qa_client::render::types::{
    ImageResourceOperation, OrderedBackend, RenderOperation, RenderView as ClientRenderView, ResourceOwner,
};
use qa_core::math::{vec4, Vec4};

use super::renderer::{CaptureId, ImageLevel, NativeRenderBackend, RenderCommand, RenderDriverInfo, RenderGlConfig};

/// [`NativeRenderBackend`] over the software rasterizer.
pub(crate) struct NativeCpuBackend {
    renderer: SoftwareRenderer,
    width: i32,
    height: i32,
    worker_interval: i32,
    color: Vec4,
}

impl NativeCpuBackend {
    /// Open a software backend over a fresh CPU framebuffer.
    pub(crate) fn open(width: i32, height: i32, gamma: f64, owner: ResourceOwner) -> Result<Self, String> {
        if width <= 0 || height <= 0 {
            return Err("windowed CPU backend requires positive dimensions".to_string());
        }
        let mut renderer = SoftwareRenderer::new(width as u32, height as u32, owner);
        renderer.set_output_gamma(gamma as f32);
        Ok(Self {
            renderer,
            width,
            height,
            worker_interval: 1,
            color: vec4(1.0, 1.0, 1.0, 1.0),
        })
    }

    /// Execute one operation list (donor `CpuRenderTarget` operation path).
    fn execute_operations(&mut self, operations: &[RenderOperation]) {
        for operation in operations {
            match operation {
                RenderOperation::Draw(batches) => {
                    for batch in batches {
                        self.renderer.draw(batch);
                    }
                }
                RenderOperation::ObjectOpacity { opacity, batches } => {
                    self.renderer.with_object_opacity(*opacity, |renderer| {
                        for batch in batches {
                            renderer.draw(batch);
                        }
                    });
                }
                operation => self.renderer.draw_immediate(operation),
            }
        }
    }

    /// Execute one ordered view: before-view operations, the view state,
    /// then the view operations (donor `CpuRenderTarget` view path).
    fn execute_view(&mut self, view: &ClientRenderView) {
        self.execute_operations(&view.before_view);
        self.renderer.begin_view(&view.state);
        self.execute_operations(&view.operations);
    }
}

impl NativeRenderBackend for NativeCpuBackend {
    type Error = String;

    fn width(&self) -> i32 {
        self.width
    }

    fn height(&self) -> i32 {
        self.height
    }

    fn is_worker(&self) -> bool {
        false
    }

    fn is_software(&self) -> bool {
        true
    }

    fn set_output_gamma(&mut self, gamma: f64) -> Result<(), String> {
        self.renderer.set_output_gamma(gamma as f32);
        Ok(())
    }

    fn apply_image_resource(&mut self, operation: &ImageResourceOperation) {
        self.renderer.apply_image_resource(operation);
    }

    fn execute_serial_command(&mut self, command: &RenderCommand) {
        match command {
            RenderCommand::SetColor { color } => {
                self.color = *color;
            }
            RenderCommand::DrawBuffer { buffer, clear } => {
                self.renderer.select_draw_buffer(*buffer, *clear);
            }
            RenderCommand::View(view) => {
                self.execute_view(view);
            }
            RenderCommand::Draw | RenderCommand::SwapBuffers => {}
        }
    }

    fn execute_worker(
        &mut self,
        _commands: &[RenderCommand],
        _captures: &[CaptureId],
        _operations: Vec<ImageResourceOperation>,
    ) {
    }

    fn synchronize(&mut self) {}

    fn set_worker_swap_interval(&mut self, interval: i32) {
        self.worker_interval = interval;
    }

    fn worker_swap_interval(&self) -> i32 {
        self.worker_interval
    }

    fn resize_backend(&mut self, width: i32, height: i32) {
        // Software resizes reopen through the factory plus the image journal
        // (`NativeRenderer::resize`); this only records the requested size.
        self.width = width;
        self.height = height;
    }

    fn finish_software(&mut self) {
        self.renderer.finish();
    }

    fn software_pixels(&self) -> Vec<u8> {
        self.renderer.pixels().to_vec()
    }

    fn read_pixels_backend(&mut self) -> ImageLevel {
        self.renderer.finish();
        ImageLevel {
            width: self.width.max(0) as u32,
            height: self.height.max(0) as u32,
            pixels: self.renderer.pixels().to_vec(),
        }
    }

    fn present_backend(&mut self) {
        // Software frames present through the window (`present_pixels`), not
        // through a GL swap.
    }

    fn driver(&self) -> Option<RenderDriverInfo> {
        None
    }

    fn gl_config(&self) -> Option<RenderGlConfig> {
        None
    }

    fn close_backend(&mut self) -> Result<(), String> {
        self.renderer.close();
        Ok(())
    }
}
