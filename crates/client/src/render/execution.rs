//! Ordered command interpretation shared by the frontend and worker.
//!
//! Donor provenance: `src/render/execution.ts` (`RenderExecutor`). Execution
//! commands mirror render commands with owned payloads: lazy batch iterables
//! become [`Vec`]s and swap captures become [`Vec<u32>`]. Dynamic textures
//! resolve per draw (uploads first, then binding), object-opacity batches run
//! inside the backend's opacity scope, and stretch-pictures project through
//! normalized device coordinates over a full-framebuffer view.

use qa_core::math::{vec2, Vec4};

use super::dynamic_texture::resolve_draw_textures;
use super::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch, DrawBuffer,
    ImageResourceOperation, OrderedBackend, PreparedDraw, Rect, RenderCommand, RenderOperation, RenderState,
    RenderVertex, RenderView, RenderViewState, RendererImage, TextureBinding, TextureRect,
};

/// One executable operation: draws plus pass-through immediates.
#[derive(Clone, Debug, PartialEq)]
pub enum ExecutionOperation {
    /// Draw batches in order.
    Draw(Vec<DrawBatch>),
    /// Draw with whole-object opacity against the current backdrop.
    ObjectOpacity {
        /// Opacity in 0..=1.
        opacity: f32,
        /// Batches.
        batches: Vec<DrawBatch>,
    },
    /// Any other operation, drawn immediately by the backend. Wrapped draw
    /// and object-opacity operations take the draw path, as if unwrapped.
    Immediate(RenderOperation),
}

impl From<RenderOperation> for ExecutionOperation {
    fn from(operation: RenderOperation) -> Self {
        match operation {
            RenderOperation::Draw(batches) => Self::Draw(batches),
            RenderOperation::ObjectOpacity { opacity, batches } => Self::ObjectOpacity { opacity, batches },
            other => Self::Immediate(other),
        }
    }
}

/// One executable view: state plus before/after operations.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionView {
    /// Viewport, clear, and clip state.
    pub state: RenderViewState,
    /// Operations before the view begins.
    pub before_view: Vec<ExecutionOperation>,
    /// Operations inside the view.
    pub operations: Vec<ExecutionOperation>,
}

impl From<RenderView> for ExecutionView {
    fn from(view: RenderView) -> Self {
        Self {
            state: view.state,
            before_view: view.before_view.into_iter().map(Into::into).collect(),
            operations: view.operations.into_iter().map(Into::into).collect(),
        }
    }
}

/// One executable command.
#[derive(Clone, Debug, PartialEq)]
pub enum ExecutionCommand {
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
    /// Render a view.
    View(ExecutionView),
    /// Swap front/back buffers, capturing the listed seats.
    SwapBuffers {
        /// Captured seat indices.
        captures: Vec<u32>,
    },
}

impl From<RenderCommand> for ExecutionCommand {
    fn from(command: RenderCommand) -> Self {
        match command {
            RenderCommand::DrawBuffer { buffer, clear } => Self::DrawBuffer { buffer, clear },
            RenderCommand::ImageResource(operation) => Self::ImageResource(operation),
            RenderCommand::SetColor(color) => Self::SetColor(color),
            RenderCommand::StretchPic { rect, uv, image } => Self::StretchPic { rect, uv, image },
            RenderCommand::View(view) => Self::View(view.into()),
            RenderCommand::SwapBuffers => Self::SwapBuffers { captures: Vec::new() },
        }
    }
}

/// Host services driven by execution: journaling and presentation.
pub trait ExecutionServices {
    /// Observe a successfully applied image operation.
    fn image_applied(&mut self, operation: &ImageResourceOperation);
    /// Present the frame, capturing the listed seats.
    fn swap(&mut self, captures: &[u32]);
}

/// Interprets execution commands against an ordered backend.
pub struct RenderExecutor<B: OrderedBackend> {
    backend: B,
    services: Box<dyn ExecutionServices>,
    color: Vec4,
}

impl<B: OrderedBackend> RenderExecutor<B> {
    /// Executor over a backend plus host services. The 2D color starts white.
    pub fn new(backend: B, services: impl ExecutionServices + 'static) -> Self {
        Self {
            backend,
            services: Box::new(services),
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
        }
    }

    /// Swap the backend, keeping services and the 2D color.
    pub fn replace_backend(&mut self, backend: B) {
        self.backend = backend;
    }

    /// Apply one image operation, then notify services.
    pub fn image(&mut self, operation: &ImageResourceOperation) {
        self.backend.apply_image_resource(operation);
        self.services.image_applied(operation);
    }

    /// Interpret one command.
    pub fn execute(&mut self, command: &ExecutionCommand) {
        match command {
            ExecutionCommand::DrawBuffer { buffer, clear } => {
                self.backend.select_draw_buffer(*buffer, *clear);
            }
            ExecutionCommand::ImageResource(operation) => self.image(operation),
            ExecutionCommand::SetColor(color) => self.color = *color,
            ExecutionCommand::StretchPic { rect, uv, image } => self.picture(rect, uv, image),
            ExecutionCommand::View(view) => {
                self.operations(&view.before_view);
                self.backend.begin_view(&view.state);
                self.operations(&view.operations);
            }
            ExecutionCommand::SwapBuffers { captures } => self.services.swap(captures),
        }
    }

    fn draw(&mut self, batch: &DrawBatch) {
        let resolved = resolve_draw_textures(batch, &mut |operation| self.image(&operation));
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

    fn draw_opacity(backend: &mut B, services: &mut dyn ExecutionServices, opacity: f32, batches: &[DrawBatch]) {
        backend.with_object_opacity(opacity, |backend| {
            for batch in batches {
                let resolved = resolve_draw_textures(batch, &mut |operation| {
                    backend.apply_image_resource(&operation);
                    services.image_applied(&operation);
                });
                Self::draw_resolved(backend, &resolved);
            }
        });
    }

    fn operations(&mut self, operations: &[ExecutionOperation]) {
        for operation in operations {
            match operation {
                ExecutionOperation::Draw(batches) => {
                    for batch in batches {
                        self.draw(batch);
                    }
                }
                ExecutionOperation::ObjectOpacity { opacity, batches } => {
                    Self::draw_opacity(&mut self.backend, &mut *self.services, *opacity, batches);
                }
                ExecutionOperation::Immediate(operation) => match operation {
                    RenderOperation::Draw(batches) => {
                        for batch in batches {
                            self.draw(batch);
                        }
                    }
                    RenderOperation::ObjectOpacity { opacity, batches } => {
                        Self::draw_opacity(&mut self.backend, &mut *self.services, *opacity, batches);
                    }
                    other => self.backend.draw_immediate(other),
                },
            }
        }
    }

    fn picture(&mut self, rect: &Rect, uv: &TextureRect, image: &RendererImage) {
        let width = self.backend.width() as f32;
        let height = self.backend.height() as f32;
        self.backend.begin_view(&RenderViewState {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            clear: None,
            clip_plane: None,
        });
        let left = rect.x / width * 2.0 - 1.0;
        let right = (rect.x + rect.width) / width * 2.0 - 1.0;
        let top = 1.0 - rect.y / height * 2.0;
        let bottom = 1.0 - (rect.y + rect.height) / height * 2.0;
        let color = self.color;
        self.draw(&DrawBatch {
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
                RenderVertex {
                    position: Vec4 {
                        x: left,
                        y: top,
                        z: 0.0,
                        w: 1.0,
                    },
                    tex_coord: vec2(uv.s1, uv.t1),
                    color,
                },
                RenderVertex {
                    position: Vec4 {
                        x: right,
                        y: top,
                        z: 0.0,
                        w: 1.0,
                    },
                    tex_coord: vec2(uv.s2, uv.t1),
                    color,
                },
                RenderVertex {
                    position: Vec4 {
                        x: right,
                        y: bottom,
                        z: 0.0,
                        w: 1.0,
                    },
                    tex_coord: vec2(uv.s2, uv.t2),
                    color,
                },
                RenderVertex {
                    position: Vec4 {
                        x: left,
                        y: bottom,
                        z: 0.0,
                        w: 1.0,
                    },
                    tex_coord: vec2(uv.s1, uv.t2),
                    color,
                },
            ]),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec4, Vec2};

    use super::super::types::{DynamicImageSource, ImageSource, ResourceOwner, TextureFilter};
    use super::*;

    struct FakePrepared<'a> {
        batches: &'a mut Vec<DrawBatch>,
        bindings: &'a mut Vec<(u32, TextureBinding)>,
        batch: DrawBatch,
    }

    impl PreparedDraw for FakePrepared<'_> {
        fn begin(&mut self) {}

        fn apply_texture(&mut self, unit: u32, binding: &TextureBinding) {
            self.bindings.push((unit, binding.clone()));
        }

        fn draw(&mut self) {
            self.batches.push(self.batch.clone());
        }

        fn cleanup(&mut self) {}
    }

    #[derive(Default)]
    struct FakeBackend {
        width: u32,
        height: u32,
        owner: Option<ResourceOwner>,
        applied_images: Vec<ImageResourceOperation>,
        draw_buffers: Vec<(DrawBuffer, bool)>,
        views: Vec<RenderViewState>,
        immediates: Vec<RenderOperation>,
        batches: Vec<DrawBatch>,
        bindings: Vec<(u32, TextureBinding)>,
        opacities: Vec<f32>,
    }

    impl FakeBackend {
        fn sized(width: u32, height: u32) -> Self {
            let authority = IdentityOwner::create("execution-test").unwrap();
            Self {
                width,
                height,
                owner: Some(ResourceOwner::new(1, authority.session().clone(), 0)),
                ..Self::default()
            }
        }
    }

    impl OrderedBackend for FakeBackend {
        type Prepared<'a> = FakePrepared<'a>;

        fn owner(&self) -> &ResourceOwner {
            self.owner.as_ref().expect("fake backend owner")
        }

        fn width(&self) -> u32 {
            self.width
        }

        fn height(&self) -> u32 {
            self.height
        }

        fn stencil_bits(&self) -> u32 {
            0
        }

        fn apply_image_resource(&mut self, operation: &ImageResourceOperation) {
            self.applied_images.push(operation.clone());
        }

        fn select_draw_buffer(&mut self, buffer: DrawBuffer, clear: bool) {
            self.draw_buffers.push((buffer, clear));
        }

        fn set_overdraw_measurement(&mut self, _enabled: bool) {}

        fn read_stencil_overdraw(&self, destination: &mut [u8]) {
            destination.fill(0);
        }

        fn read_depth_pixel(&self, _window_x: i32, _window_y: i32) -> f32 {
            0.0
        }

        fn begin_view(&mut self, view: &RenderViewState) {
            self.views.push(*view);
        }

        fn with_object_opacity(&mut self, opacity: f32, draw: impl FnOnce(&mut Self)) {
            self.opacities.push(opacity);
            draw(self);
        }

        fn draw_immediate(&mut self, operation: &RenderOperation) {
            self.immediates.push(operation.clone());
        }

        fn prepare_geometry(&mut self, batch: &DrawBatch) -> Self::Prepared<'_> {
            FakePrepared {
                batches: &mut self.batches,
                bindings: &mut self.bindings,
                batch: batch.clone(),
            }
        }

        fn clear_color_buffer(&mut self) {}

        fn draw_show_image(&mut self, _image: &RendererImage, _rect: &Rect, _proportional: bool) {}

        fn finish(&mut self) {}

        fn close(&mut self) {}
    }

    #[derive(Default)]
    struct FakeLog {
        applied: Vec<ImageResourceOperation>,
        swaps: Vec<Vec<u32>>,
    }

    #[derive(Clone, Default)]
    struct FakeServices {
        log: Rc<RefCell<FakeLog>>,
    }

    impl ExecutionServices for FakeServices {
        fn image_applied(&mut self, operation: &ImageResourceOperation) {
            self.log.borrow_mut().applied.push(operation.clone());
        }

        fn swap(&mut self, captures: &[u32]) {
            self.log.borrow_mut().swaps.push(captures.to_vec());
        }
    }

    fn executor(width: u32, height: u32) -> (RenderExecutor<FakeBackend>, FakeServices) {
        let services = FakeServices::default();
        (
            RenderExecutor::new(FakeBackend::sized(width, height), services.clone()),
            services,
        )
    }

    fn image(ordinal: u32) -> RendererImage {
        let authority = IdentityOwner::create("execution-image").unwrap();
        RendererImage {
            owner: ResourceOwner::new(1, authority.session().clone(), 0),
            ordinal,
            source: ImageSource::Generated {
                name: "pic".to_string(),
            },
            width: 64,
            height: 64,
        }
    }

    fn batch(texture: TextureBinding) -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0],
            texture,
            state: RenderState::opaque(CullFace::Back),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![RenderVertex {
                position: vec4(0.0, 0.0, 0.0, 1.0),
                tex_coord: Vec2 { x: 0.0, y: 0.0 },
                color: vec4(1.0, 1.0, 1.0, 1.0),
            }]),
        }
    }

    struct UploadSource {
        image: RendererImage,
    }

    impl DynamicImageSource for UploadSource {
        fn resolve(&self, apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage {
            apply(ImageResourceOperation::TextureMode {
                filter: TextureFilter::Linear,
            });
            self.image.clone()
        }
    }

    #[test]
    fn image_applies_then_notifies() {
        let (mut executor, services) = executor(64, 64);
        let operation = ImageResourceOperation::TextureMode {
            filter: TextureFilter::Nearest,
        };
        executor.execute(&ExecutionCommand::ImageResource(operation.clone()));
        assert_eq!(executor.backend.applied_images, [operation.clone()]);
        assert_eq!(services.log.borrow().applied, [operation]);
    }

    #[test]
    fn draw_buffer_and_swap_dispatch() {
        let (mut executor, services) = executor(64, 64);
        executor.execute(&ExecutionCommand::DrawBuffer {
            buffer: DrawBuffer::Back,
            clear: true,
        });
        executor.execute(&ExecutionCommand::SwapBuffers { captures: vec![0, 2] });
        assert_eq!(executor.backend.draw_buffers, [(DrawBuffer::Back, true)]);
        assert_eq!(services.log.borrow().swaps, [vec![0, 2]]);
    }

    #[test]
    fn view_runs_before_view_then_state_then_operations() {
        let (mut executor, _services) = executor(64, 64);
        let expected = image(1);
        let batch = batch(TextureBinding::BindImage(expected.clone()));
        executor.execute(&ExecutionCommand::View(ExecutionView {
            state: RenderViewState {
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                clear: None,
                clip_plane: None,
            },
            before_view: vec![ExecutionOperation::Immediate(RenderOperation::Cull(CullFace::Front))],
            operations: vec![ExecutionOperation::Draw(vec![batch.clone()])],
        }));
        assert_eq!(executor.backend.immediates, [RenderOperation::Cull(CullFace::Front)]);
        assert_eq!(executor.backend.views.len(), 1);
        assert_eq!(executor.backend.batches, [batch]);
        assert_eq!(executor.backend.bindings, [(0, TextureBinding::BindImage(expected))]);
    }

    #[test]
    fn object_opacity_scopes_its_batches() {
        let (mut executor, _services) = executor(64, 64);
        executor.execute(&ExecutionCommand::View(ExecutionView {
            state: RenderViewState {
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                clear: None,
                clip_plane: None,
            },
            before_view: Vec::new(),
            operations: vec![ExecutionOperation::ObjectOpacity {
                opacity: 0.5,
                batches: vec![batch(TextureBinding::RetainCurrentTexture)],
            }],
        }));
        assert_eq!(executor.backend.opacities, [0.5]);
        assert_eq!(executor.backend.batches.len(), 1);
    }

    #[test]
    fn dynamic_textures_upload_before_binding() {
        let (mut executor, services) = executor(64, 64);
        let expected = image(3);
        let source = Arc::new(UploadSource {
            image: expected.clone(),
        });
        executor.execute(&ExecutionCommand::View(ExecutionView {
            state: RenderViewState {
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                clear: None,
                clip_plane: None,
            },
            before_view: Vec::new(),
            operations: vec![ExecutionOperation::Draw(vec![batch(TextureBinding::DynamicImage(
                source,
            ))])],
        }));
        assert_eq!(executor.backend.applied_images.len(), 1);
        assert_eq!(services.log.borrow().applied.len(), 1);
        assert_eq!(executor.backend.bindings, [(0, TextureBinding::BindImage(expected))]);
    }

    #[test]
    fn stretch_pic_projects_ndc_corners_over_full_viewport() {
        let (mut executor, _services) = executor(640, 480);
        let expected = image(9);
        executor.execute(&ExecutionCommand::SetColor(vec4(1.0, 0.5, 0.25, 1.0)));
        executor.execute(&ExecutionCommand::StretchPic {
            rect: Rect {
                x: 160.0,
                y: 120.0,
                width: 320.0,
                height: 240.0,
            },
            uv: TextureRect {
                s1: 0.0,
                t1: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            image: expected.clone(),
        });
        assert_eq!(
            executor.backend.views,
            [RenderViewState {
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 640.0,
                    height: 480.0,
                },
                clear: None,
                clip_plane: None,
            }]
        );
        assert_eq!(executor.backend.batches.len(), 1);
        let drawn = &executor.backend.batches[0];
        assert_eq!(drawn.indices, [0, 1, 2, 0, 2, 3]);
        assert_eq!(drawn.texture, TextureBinding::BindImage(expected));
        assert_eq!(drawn.lighting, BatchLighting::Vertex);
        assert_eq!(
            drawn.state,
            RenderState {
                blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                depth_test: DepthTest::Always,
                depth_write: false,
                alpha_test: AlphaTest::None,
                cull: CullFace::None,
                depth_range: [0.0, 1.0],
                polygon_offset: None,
            }
        );
        let BatchVertices::Single(vertices) = &drawn.vertices else {
            panic!("expected single-textured picture");
        };
        let corners: Vec<(f32, f32, f32, f32)> = vertices
            .iter()
            .map(|vertex| {
                (
                    vertex.position.x,
                    vertex.position.y,
                    vertex.tex_coord.x,
                    vertex.tex_coord.y,
                )
            })
            .collect();
        assert_eq!(
            corners,
            [
                (-0.5, 0.5, 0.0, 0.0),
                (0.5, 0.5, 1.0, 0.0),
                (0.5, -0.5, 1.0, 1.0),
                (-0.5, -0.5, 0.0, 1.0),
            ]
        );
        assert!(vertices.iter().all(|vertex| vertex.position.z == 0.0
            && vertex.position.w == 1.0
            && vertex.color == vec4(1.0, 0.5, 0.25, 1.0)));
    }

    #[test]
    fn replace_backend_keeps_color_and_services() {
        let (mut executor, services) = executor(64, 64);
        executor.execute(&ExecutionCommand::SetColor(vec4(0.0, 1.0, 0.0, 1.0)));
        executor.execute(&ExecutionCommand::SwapBuffers { captures: vec![1] });
        executor.replace_backend(FakeBackend::sized(320, 200));
        executor.execute(&ExecutionCommand::StretchPic {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 320.0,
                height: 200.0,
            },
            uv: TextureRect {
                s1: 0.0,
                t1: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            image: image(2),
        });
        assert_eq!(executor.backend.width(), 320);
        let BatchVertices::Single(vertices) = &executor.backend.batches[0].vertices else {
            panic!("expected single-textured picture");
        };
        assert_eq!(vertices[0].position.x, -1.0);
        assert_eq!(vertices[0].position.y, 1.0);
        assert_eq!(vertices[0].color, vec4(0.0, 1.0, 0.0, 1.0));
        assert_eq!(services.log.borrow().swaps, [vec![1]]);
    }

    #[test]
    fn render_commands_convert_to_execution() {
        assert_eq!(
            ExecutionCommand::from(RenderCommand::SwapBuffers),
            ExecutionCommand::SwapBuffers { captures: Vec::new() }
        );
        let view = RenderView {
            state: RenderViewState {
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                },
                clear: None,
                clip_plane: None,
            },
            target: super::super::types::ViewTarget::Preview("p".to_string()),
            time: super::super::types::SourceTime::Seconds(0.0),
            before_view: vec![RenderOperation::Cull(CullFace::Back)],
            operations: vec![RenderOperation::Draw(Vec::new())],
        };
        let converted = ExecutionView::from(view.clone());
        assert_eq!(converted.state, view.state);
        assert_eq!(
            converted.before_view,
            [ExecutionOperation::Immediate(RenderOperation::Cull(CullFace::Back))]
        );
        assert_eq!(converted.operations, [ExecutionOperation::Draw(Vec::new())]);
    }
}
