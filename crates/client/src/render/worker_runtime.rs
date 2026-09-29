//! Worker-side command interpreter over an ordered backend.
//!
//! Donor provenance: `src/render/execution.ts` (`RenderExecutor`) and the
//! `execute` dispatch in `src/render/worker-runtime.ts`. Image operations
//! apply to the backend and report through the `image_applied` callback (the
//! frontend journals them); buffer swaps report through `swap`. The 2D color,
//! stretch-picture quad, and view before/after ordering match the donor.

use std::collections::HashMap;
use std::sync::Arc;

use qa_core::math::{vec2, vec4, Vec4};

use super::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch, DrawBuffer,
    ImageResourceOperation, OrderedBackend, PreparedDraw, Rect, RenderCommand, RenderOperation, RenderState,
    RenderVertex, RendererImage, TextureBinding, TextureRect,
};

/// One decoded view operation: batch draws stay structured so the runtime
/// can resolve dynamic textures, everything else runs as immediate work.
#[derive(Clone, Debug, PartialEq)]
pub enum DecodedOperation {
    /// Draw batches in order.
    Draw(Vec<DrawBatch>),
    /// Draw with whole-object opacity against the current backdrop.
    ObjectOpacity {
        /// Opacity in 0..=1.
        opacity: f32,
        /// Batches.
        batches: Vec<DrawBatch>,
    },
    /// Any non-batch operation, run through `draw_immediate`.
    Immediate(RenderOperation),
}

impl From<&RenderOperation> for DecodedOperation {
    fn from(operation: &RenderOperation) -> Self {
        match operation {
            RenderOperation::Draw(batches) => Self::Draw(batches.clone()),
            RenderOperation::ObjectOpacity { opacity, batches } => Self::ObjectOpacity {
                opacity: *opacity,
                batches: batches.clone(),
            },
            other => Self::Immediate(other.clone()),
        }
    }
}

/// One decoded worker command. The wire decoder owns the wire format and
/// produces these owned values; swap carries capture ids like the donor
/// `swap-buffers` execution command.
#[derive(Clone, Debug, PartialEq)]
pub enum DecodedCommand {
    /// Select the draw buffer.
    DrawBuffer {
        /// Buffer.
        buffer: DrawBuffer,
        /// Clear after selecting.
        clear: bool,
    },
    /// Image resource operation.
    ImageResource(ImageResourceOperation),
    /// Set the 2D drawing color.
    SetColor(Vec4),
    /// Stretch a picture.
    StretchPic {
        /// Destination rectangle.
        rect: Rect,
        /// Source coordinates.
        uv: TextureRect,
        /// Picture image.
        image: RendererImage,
    },
    /// Render a view: state plus before/inside operations.
    View {
        /// Shared view state.
        state: super::types::RenderViewState,
        /// Operations before the view begins.
        before_view: Vec<DecodedOperation>,
        /// Operations inside the view.
        operations: Vec<DecodedOperation>,
    },
    /// Swap front/back buffers.
    SwapBuffers {
        /// Capture ids armed for this swap.
        captures: Vec<u32>,
    },
}

impl From<&RenderCommand> for DecodedCommand {
    fn from(command: &RenderCommand) -> Self {
        match command {
            RenderCommand::DrawBuffer { buffer, clear } => Self::DrawBuffer {
                buffer: *buffer,
                clear: *clear,
            },
            RenderCommand::ImageResource(operation) => Self::ImageResource(operation.clone()),
            RenderCommand::SetColor(color) => Self::SetColor(*color),
            RenderCommand::StretchPic { rect, uv, image } => Self::StretchPic {
                rect: *rect,
                uv: *uv,
                image: image.clone(),
            },
            RenderCommand::View(view) => Self::View {
                state: view.state,
                before_view: view.before_view.iter().map(DecodedOperation::from).collect(),
                operations: view.operations.iter().map(DecodedOperation::from).collect(),
            },
            RenderCommand::SwapBuffers => Self::SwapBuffers { captures: Vec::new() },
        }
    }
}

/// Journal-notification callback for applied image operations.
pub type ImageAppliedCallback = Box<dyn FnMut(&ImageResourceOperation)>;

/// Swap-notification callback: capture ids presented by `swap-buffers`.
pub type SwapCallback = Box<dyn FnMut(&[u32])>;

/// Worker-side interpreter owning its backend, 2D color, and callbacks.
pub struct WorkerRuntime<B: OrderedBackend> {
    backend: B,
    color: Vec4,
    on_image_applied: ImageAppliedCallback,
    on_swap: SwapCallback,
}

impl<B: OrderedBackend> WorkerRuntime<B> {
    /// Build a runtime over `backend` with journal/swap callbacks.
    pub fn new(
        backend: B,
        on_image_applied: impl FnMut(&ImageResourceOperation) + 'static,
        on_swap: impl FnMut(&[u32]) + 'static,
    ) -> Self {
        Self {
            backend,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            on_image_applied: Box::new(on_image_applied),
            on_swap: Box::new(on_swap),
        }
    }

    /// Replace the backend, keeping color and callbacks.
    pub fn replace_backend(&mut self, backend: B) {
        self.backend = backend;
    }

    /// Borrow the backend.
    #[must_use]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Mutably borrow the backend.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Current 2D drawing color.
    #[must_use]
    pub const fn color(&self) -> Vec4 {
        self.color
    }

    /// Apply one image operation, then report it for journaling.
    pub fn image(&mut self, operation: &ImageResourceOperation) {
        self.backend.apply_image_resource(operation);
        (self.on_image_applied)(operation);
    }

    /// Execute one decoded command in donor order.
    pub fn execute_decoded(&mut self, command: &DecodedCommand) {
        match command {
            DecodedCommand::DrawBuffer { buffer, clear } => {
                self.backend.select_draw_buffer(*buffer, *clear);
            }
            DecodedCommand::ImageResource(operation) => self.image(operation),
            DecodedCommand::SetColor(color) => self.color = *color,
            DecodedCommand::StretchPic { rect, uv, image } => self.stretch_pic(rect, uv, image),
            DecodedCommand::View {
                state,
                before_view,
                operations,
            } => {
                self.decoded_operations(before_view);
                self.backend.begin_view(state);
                self.decoded_operations(operations);
            }
            DecodedCommand::SwapBuffers { captures } => (self.on_swap)(captures),
        }
    }

    /// Execute one ordered command directly, without a wire round-trip.
    pub fn execute(&mut self, command: &RenderCommand) {
        let decoded = DecodedCommand::from(command);
        self.execute_decoded(&decoded);
    }

    fn decoded_operations(&mut self, operations: &[DecodedOperation]) {
        for operation in operations {
            match operation {
                DecodedOperation::Draw(batches) => {
                    for batch in batches {
                        self.draw_batch(batch);
                    }
                }
                DecodedOperation::ObjectOpacity { opacity, batches } => {
                    let backend = &mut self.backend;
                    let applied: &mut dyn FnMut(&ImageResourceOperation) = &mut *self.on_image_applied;
                    backend.with_object_opacity(*opacity, |inner| {
                        for batch in batches {
                            let mut apply = |operation: ImageResourceOperation| {
                                inner.apply_image_resource(&operation);
                                applied(&operation);
                            };
                            let resolved = Self::resolve_batch(batch, &mut apply);
                            Self::draw_resolved(inner, &resolved);
                        }
                    });
                }
                DecodedOperation::Immediate(operation) => {
                    self.backend.draw_immediate(operation);
                }
            }
        }
    }

    fn draw_batch(&mut self, batch: &DrawBatch) {
        let resolved = {
            let backend = &mut self.backend;
            let applied = &mut *self.on_image_applied;
            let mut apply = |operation: ImageResourceOperation| {
                backend.apply_image_resource(&operation);
                applied(&operation);
            };
            Self::resolve_batch(batch, &mut apply)
        };
        Self::draw_resolved(&mut self.backend, &resolved);
    }

    fn draw_resolved(backend: &mut B, batch: &DrawBatch) {
        let mut prepared = backend.prepare_geometry(batch);
        prepared.begin();
        prepared.apply_texture(0, &batch.texture);
        if let BatchVertices::Pair { second_texture, .. } = &batch.vertices {
            prepared.apply_texture(1, &second_texture.binding);
        }
        prepared.draw();
        prepared.cleanup();
    }

    fn resolve_batch(batch: &DrawBatch, apply: &mut dyn FnMut(ImageResourceOperation)) -> DrawBatch {
        let mut cache: HashMap<*const (), RendererImage> = HashMap::new();
        let mut resolved = batch.clone();
        resolved.texture = Self::resolve_binding(&batch.texture, &mut cache, &mut *apply);
        if let BatchVertices::Pair { second_texture, .. } = &batch.vertices {
            let binding = Self::resolve_binding(&second_texture.binding, &mut cache, &mut *apply);
            if let BatchVertices::Pair { second_texture, .. } = &mut resolved.vertices {
                second_texture.binding = binding;
            }
        }
        resolved
    }

    fn resolve_binding(
        binding: &TextureBinding,
        cache: &mut HashMap<*const (), RendererImage>,
        apply: &mut dyn FnMut(ImageResourceOperation),
    ) -> TextureBinding {
        match binding {
            TextureBinding::DynamicImage(source) => {
                let key = Arc::as_ptr(source) as *const ();
                if let Some(image) = cache.get(&key) {
                    return TextureBinding::BindImage(image.clone());
                }
                let image = source.resolve(apply);
                cache.insert(key, image.clone());
                TextureBinding::BindImage(image)
            }
            TextureBinding::BindImage(image) => TextureBinding::BindImage(image.clone()),
            TextureBinding::RetainCurrentTexture => TextureBinding::RetainCurrentTexture,
        }
    }

    fn stretch_pic(&mut self, rect: &Rect, uv: &TextureRect, image: &RendererImage) {
        let (width, height) = (self.backend.width(), self.backend.height());
        self.backend.begin_view(&super::types::RenderViewState {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height: height as f32,
            },
            clear: None,
            clip_plane: None,
        });
        let (w, h) = (width as f32, height as f32);
        let left = rect.x / w * 2.0 - 1.0;
        let right = (rect.x + rect.width) / w * 2.0 - 1.0;
        let top = 1.0 - rect.y / h * 2.0;
        let bottom = 1.0 - (rect.y + rect.height) / h * 2.0;
        let color = self.color;
        let vertex = |x: f32, y: f32, s: f32, t: f32| RenderVertex {
            position: vec4(x, y, 0.0, 1.0),
            tex_coord: vec2(s, t),
            color,
        };
        let batch = DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0, 1, 2, 0, 2, 3],
            texture: TextureBinding::BindImage(image.clone()),
            state: RenderState {
                blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                depth_test: DepthTest::Always,
                depth_write: false,
                alpha_test: AlphaTest::None,
                cull: CullFace::None,
                depth_range: [0.0, 1.0],
                polygon_offset: None,
            },
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![
                vertex(left, top, uv.s1, uv.t1),
                vertex(right, top, uv.s2, uv.t1),
                vertex(right, bottom, uv.s2, uv.t2),
                vertex(left, bottom, uv.s1, uv.t2),
            ]),
        };
        self.draw_batch(&batch);
    }
}

#[cfg(test)]
pub(crate) mod support {
    use std::sync::atomic::{AtomicU32, Ordering};

    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec2, vec4};

    use super::super::types::{
        BatchLighting, BatchPrimitive, BatchVertices, CullFace, DrawBatch, DrawBuffer, DynamicImageSource,
        ImageResourceOperation, ImageSource, OrderedBackend, PreparedDraw, Rect, RenderOperation, RenderState,
        RenderVertex, RenderViewState, RendererImage, ResourceOwner, TextureBinding,
    };

    /// Recording backend: every call appends one line to `log`.
    pub struct FakeBackend {
        /// Owning renderer lifetime.
        pub owner: ResourceOwner,
        /// Framebuffer width.
        pub width: u32,
        /// Framebuffer height.
        pub height: u32,
        /// Call log.
        pub log: Vec<String>,
    }

    impl FakeBackend {
        /// Fresh 64x32 backend with its own owner.
        pub fn new() -> Self {
            let authority = IdentityOwner::create("worker-runtime-test").unwrap();
            Self {
                owner: ResourceOwner::new(1, authority.session().clone(), 0),
                width: 64,
                height: 32,
                log: Vec::new(),
            }
        }
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Recording prepared draw borrowing the backend log.
    pub struct FakePrepared<'a> {
        log: &'a mut Vec<String>,
    }

    impl PreparedDraw for FakePrepared<'_> {
        fn begin(&mut self) {
            self.log.push("begin".to_string());
        }

        fn apply_texture(&mut self, unit: u32, binding: &TextureBinding) {
            let name = match binding {
                TextureBinding::DynamicImage(_) => "dynamic".to_string(),
                TextureBinding::BindImage(image) => format!("image:{}", image.ordinal),
                TextureBinding::RetainCurrentTexture => "retain".to_string(),
            };
            self.log.push(format!("texture:{unit}:{name}"));
        }

        fn draw(&mut self) {
            self.log.push("draw".to_string());
        }

        fn cleanup(&mut self) {
            self.log.push("cleanup".to_string());
        }
    }

    fn operation_name(operation: &ImageResourceOperation) -> String {
        match operation {
            ImageResourceOperation::CreateImage { image, .. } => {
                format!("create:{}", image.ordinal)
            }
            ImageResourceOperation::UpdateImage { image, level, .. } => {
                format!("update:{}:{level}", image.ordinal)
            }
            ImageResourceOperation::ReleaseImage { image } => {
                format!("release:{}", image.ordinal)
            }
            ImageResourceOperation::TextureMode { .. } => "mode".to_string(),
        }
    }

    fn immediate_name(operation: &RenderOperation) -> &'static str {
        match operation {
            RenderOperation::Draw(_) => "draw",
            RenderOperation::ObjectOpacity { .. } => "object-opacity",
            RenderOperation::Q2Fog(_) => "q2-fog",
            RenderOperation::DepthAtlas { .. } => "depth-atlas",
            RenderOperation::DepthRange(_) => "depth-range",
            RenderOperation::Cull(_) => "cull",
            RenderOperation::PolygonOffset(_) => "polygon-offset",
            RenderOperation::DisablePortalClip => "disable-portal-clip",
            RenderOperation::SkySide { .. } => "sky-side",
            RenderOperation::ShadowVolume { .. } => "shadow-volume",
            RenderOperation::ShadowFinish { .. } => "shadow-finish",
        }
    }

    fn batch_summary(batch: &DrawBatch) -> String {
        let first = match &batch.vertices {
            BatchVertices::Single(vertices) => vertices.first().map(|vertex| vertex.color),
            BatchVertices::Pair { vertices, .. } => vertices.first().map(|vertex| vertex.base.color),
        };
        format!("prepare:{}:{first:?}", batch.indices.len())
    }

    impl OrderedBackend for FakeBackend {
        type Prepared<'a> = FakePrepared<'a>;

        fn owner(&self) -> &ResourceOwner {
            &self.owner
        }

        fn width(&self) -> u32 {
            self.width
        }

        fn height(&self) -> u32 {
            self.height
        }

        fn stencil_bits(&self) -> u32 {
            8
        }

        fn apply_image_resource(&mut self, operation: &ImageResourceOperation) {
            self.log.push(format!("image:{}", operation_name(operation)));
        }

        fn select_draw_buffer(&mut self, buffer: DrawBuffer, clear: bool) {
            self.log.push(format!("buffer:{buffer:?}:{clear}"));
        }

        fn set_overdraw_measurement(&mut self, enabled: bool) {
            self.log.push(format!("overdraw:{enabled}"));
        }

        fn read_stencil_overdraw(&self, destination: &mut [u8]) {
            destination.fill(0);
        }

        fn read_depth_pixel(&self, _window_x: i32, _window_y: i32) -> f32 {
            1.0
        }

        fn begin_view(&mut self, view: &RenderViewState) {
            self.log
                .push(format!("view:{}x{}", view.viewport.width, view.viewport.height));
        }

        fn with_object_opacity(&mut self, opacity: f32, draw: impl FnOnce(&mut Self)) {
            self.log.push(format!("opacity-begin:{opacity}"));
            draw(self);
            self.log.push("opacity-end".to_string());
        }

        fn draw_immediate(&mut self, operation: &RenderOperation) {
            self.log.push(format!("immediate:{}", immediate_name(operation)));
        }

        fn prepare_geometry(&mut self, batch: &DrawBatch) -> Self::Prepared<'_> {
            self.log.push(batch_summary(batch));
            FakePrepared { log: &mut self.log }
        }

        fn clear_color_buffer(&mut self) {
            self.log.push("clear".to_string());
        }

        fn draw_show_image(&mut self, image: &RendererImage, _rect: &Rect, proportional: bool) {
            self.log.push(format!("show:{}:{proportional}", image.ordinal));
        }

        fn finish(&mut self) {
            self.log.push("finish".to_string());
        }

        fn close(&mut self) {
            self.log.push("close".to_string());
        }
    }

    /// Generated image handle under `owner`.
    pub fn test_image(owner: &ResourceOwner, ordinal: u32) -> RendererImage {
        RendererImage {
            owner: owner.clone(),
            ordinal,
            source: ImageSource::Generated {
                name: format!("test{ordinal}"),
            },
            width: 4,
            height: 4,
        }
    }

    /// Single-vertex batch binding `image`.
    pub fn test_batch(image: &RendererImage) -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0],
            texture: TextureBinding::BindImage(image.clone()),
            state: RenderState::opaque(CullFace::None),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![RenderVertex {
                position: vec4(0.0, 0.0, 0.0, 1.0),
                tex_coord: vec2(0.0, 0.0),
                color: vec4(1.0, 1.0, 1.0, 1.0),
            }]),
        }
    }

    /// View state covering the whole fake framebuffer.
    pub fn test_view_state() -> RenderViewState {
        RenderViewState {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 32.0,
            },
            clear: None,
            clip_plane: None,
        }
    }

    /// Dynamic source replaying scripted uploads, counting resolutions.
    pub struct ScriptSource {
        /// Image to bind.
        pub image: RendererImage,
        /// Uploads to run through `apply` on each resolution.
        pub uploads: Vec<ImageResourceOperation>,
        /// Resolution count.
        pub calls: AtomicU32,
    }

    impl DynamicImageSource for ScriptSource {
        fn resolve(&self, apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage {
            self.calls.fetch_add(1, Ordering::SeqCst);
            for upload in &self.uploads {
                apply(upload.clone());
            }
            self.image.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use qa_core::math::vec4;

    use super::super::types::{
        CullFace, DrawBuffer, ImageResourceOperation, Rect, RenderOperation, TextureBinding, TextureFilter, TextureRect,
    };
    use super::support::*;
    use super::*;

    fn runtime() -> (
        WorkerRuntime<FakeBackend>,
        std::rc::Rc<std::cell::RefCell<Vec<ImageResourceOperation>>>,
        std::rc::Rc<std::cell::RefCell<Vec<Vec<u32>>>>,
    ) {
        let applied = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let swaps = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let applied_inner = std::rc::Rc::clone(&applied);
        let swaps_inner = std::rc::Rc::clone(&swaps);
        let runtime = WorkerRuntime::new(
            FakeBackend::new(),
            move |operation: &ImageResourceOperation| applied_inner.borrow_mut().push(operation.clone()),
            move |captures: &[u32]| swaps_inner.borrow_mut().push(captures.to_vec()),
        );
        (runtime, applied, swaps)
    }

    fn texture_mode() -> ImageResourceOperation {
        ImageResourceOperation::TextureMode {
            filter: TextureFilter::Linear,
        }
    }

    #[test]
    fn image_operations_apply_then_report() {
        let (mut runtime, applied, _) = runtime();
        runtime.execute_decoded(&DecodedCommand::ImageResource(texture_mode()));
        assert_eq!(runtime.backend().log, ["image:mode"]);
        assert_eq!(*applied.borrow(), [texture_mode()]);
    }

    #[test]
    fn draw_buffer_selects_backend_buffer() {
        let (mut runtime, _, _) = runtime();
        runtime.execute_decoded(&DecodedCommand::DrawBuffer {
            buffer: DrawBuffer::Back,
            clear: true,
        });
        assert_eq!(runtime.backend().log, ["buffer:Back:true"]);
    }

    #[test]
    fn set_color_feeds_stretch_pic_vertices() {
        let (mut runtime, _, _) = runtime();
        let color = vec4(0.25, 0.5, 1.0, 0.75);
        runtime.execute_decoded(&DecodedCommand::SetColor(color));
        assert_eq!(runtime.color(), color);
        let owner = runtime.backend().owner.clone();
        runtime.execute_decoded(&DecodedCommand::StretchPic {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 32.0,
            },
            uv: TextureRect {
                s1: 0.0,
                t1: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            image: test_image(&owner, 3),
        });
        let log = &runtime.backend().log;
        assert_eq!(log[0], "view:64x32");
        assert!(log[1].contains("prepare:6:"), "quad indices: {log:?}");
        assert!(log[1].contains("0.25"), "2D color reaches vertices: {log:?}");
        assert_eq!(&log[2..], ["begin", "texture:0:image:3", "draw", "cleanup"]);
    }

    #[test]
    fn view_runs_before_ops_then_view_then_ops() {
        let (mut runtime, _, _) = runtime();
        let owner = runtime.backend().owner.clone();
        let batch = test_batch(&test_image(&owner, 1));
        runtime.execute_decoded(&DecodedCommand::View {
            state: test_view_state(),
            before_view: vec![DecodedOperation::Immediate(RenderOperation::Cull(CullFace::Back))],
            operations: vec![
                DecodedOperation::Draw(vec![batch]),
                DecodedOperation::ObjectOpacity {
                    opacity: 0.5,
                    batches: vec![test_batch(&test_image(&owner, 2))],
                },
                DecodedOperation::Immediate(RenderOperation::DepthRange([0.0, 1.0])),
            ],
        });
        let log = &runtime.backend().log;
        assert_eq!(log[0], "immediate:cull");
        assert_eq!(log[1], "view:64x32");
        assert!(log[2].starts_with("prepare:"), "{log:?}");
        assert_eq!(&log[3..7], ["begin", "texture:0:image:1", "draw", "cleanup"]);
        assert_eq!(log[7], "opacity-begin:0.5");
        assert_eq!(&log[9..13], ["begin", "texture:0:image:2", "draw", "cleanup"]);
        assert_eq!(log[13], "opacity-end");
        assert_eq!(log[14], "immediate:depth-range");
    }

    #[test]
    fn swap_buffers_reports_captures() {
        let (mut runtime, _, swaps) = runtime();
        runtime.execute_decoded(&DecodedCommand::SwapBuffers { captures: vec![7, 9] });
        assert_eq!(*swaps.borrow(), [vec![7, 9]]);
        assert!(runtime.backend().log.is_empty());
    }

    #[test]
    fn dynamic_sources_resolve_once_per_draw_with_uploads() {
        let (mut runtime, applied, _) = runtime();
        let owner = runtime.backend().owner.clone();
        let source = Arc::new(ScriptSource {
            image: test_image(&owner, 5),
            uploads: vec![texture_mode()],
            calls: AtomicU32::new(0),
        });
        let mut batch = test_batch(&test_image(&owner, 0));
        batch.texture = TextureBinding::DynamicImage(source.clone());
        batch.vertices = BatchVertices::Pair {
            vertices: vec![super::super::types::MultitextureVertex {
                base: super::super::types::RenderVertex {
                    position: vec4(0.0, 0.0, 0.0, 1.0),
                    tex_coord: qa_core::math::vec2(0.0, 0.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                tex_coord2: qa_core::math::vec2(1.0, 1.0),
            }],
            second_texture: super::super::types::TextureBundle {
                binding: TextureBinding::DynamicImage(source.clone()),
                environment: super::super::types::PairEnvironment::Modulate,
            },
        };
        runtime.execute_decoded(&DecodedCommand::View {
            state: test_view_state(),
            before_view: Vec::new(),
            operations: vec![DecodedOperation::Draw(vec![batch])],
        });
        assert_eq!(source.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(*applied.borrow(), [texture_mode()]);
        let log = &runtime.backend().log;
        assert_eq!(log[0], "view:64x32");
        assert_eq!(log[1], "image:mode");
        assert!(log[2].starts_with("prepare:"), "{log:?}");
        assert_eq!(log[4], "texture:0:image:5");
        assert_eq!(log[5], "texture:1:image:5");
    }

    #[test]
    fn render_command_conversion_keeps_view_structure() {
        let (mut runtime, _, _) = runtime();
        let owner = runtime.backend().owner.clone();
        let frame_command = RenderCommand::View(super::super::types::RenderView {
            state: test_view_state(),
            target: super::super::types::ViewTarget::Preview("p".to_string()),
            time: super::super::types::SourceTime::Milliseconds(16.0),
            before_view: vec![RenderOperation::DisablePortalClip],
            operations: vec![RenderOperation::Draw(vec![test_batch(&test_image(&owner, 1))])],
        });
        let decoded = DecodedCommand::from(&frame_command);
        let DecodedCommand::View {
            before_view,
            operations,
            ..
        } = &decoded
        else {
            panic!("expected a decoded view");
        };
        assert_eq!(before_view.len(), 1);
        assert!(matches!(before_view[0], DecodedOperation::Immediate(_)));
        assert!(matches!(operations[0], DecodedOperation::Draw(_)));
        runtime.execute(&frame_command);
        assert_eq!(runtime.backend().log[0], "immediate:disable-portal-clip");
        assert_eq!(
            DecodedCommand::from(&RenderCommand::SwapBuffers),
            DecodedCommand::SwapBuffers { captures: Vec::new() }
        );
    }

    #[test]
    fn backend_replacement_keeps_color() {
        let (mut runtime, _, _) = runtime();
        runtime.execute_decoded(&DecodedCommand::SetColor(vec4(0.0, 1.0, 0.0, 1.0)));
        runtime.replace_backend(FakeBackend::new());
        assert_eq!(runtime.color(), vec4(0.0, 1.0, 0.0, 1.0));
        assert!(runtime.backend().log.is_empty());
        runtime.backend_mut().log.push("probe".to_string());
        assert_eq!(runtime.backend().log, ["probe"]);
    }
}
