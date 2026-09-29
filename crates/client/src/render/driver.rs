//! Headless synchronous driver over an ordered backend.
//!
//! Donor provenance: `src/render/execution.ts` (`RenderExecutor`) and
//! `src/render/image-journal.ts` (`RenderImageJournal`). The driver runs the
//! same ordered command stream as the worker runtime without any transport:
//! image operations apply and journal locally, views run before/inside
//! operations around `begin_view`, and swaps report through the hook. The
//! journal replays creations and per-level updates with the texture-mode
//! change between older and newer images, in donor insertion order.

use std::collections::HashMap;
use std::sync::Arc;

use qa_core::math::{vec2, vec4, Vec4};

use super::error::RenderError;
use super::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch,
    ImageResourceOperation, OrderedBackend, PreparedDraw, Rect, RenderCommand, RenderFrame, RenderOperation,
    RenderState, RenderVertex, RendererImage, TextureBinding, TextureFilter, TextureRect,
};

/// One resident image: its creation plus per-level replacements, each tagged
/// with a record sequence so replay follows donor insertion order.
struct ResidentEntry {
    order: u64,
    before_texture_mode: bool,
    creation: ImageResourceOperation,
    updates: HashMap<u32, (u64, ImageResourceOperation)>,
}

/// Successful image mutations retained for backend replacement.
struct ImageJournal {
    next_order: u64,
    resident: HashMap<u32, ResidentEntry>,
    texture_mode: Option<TextureFilter>,
}

impl ImageJournal {
    fn new() -> Self {
        Self {
            next_order: 0,
            resident: HashMap::new(),
            texture_mode: None,
        }
    }

    fn replay(&self, apply: &mut dyn FnMut(&ImageResourceOperation)) {
        let mut ordered: Vec<&ResidentEntry> = self.resident.values().collect();
        ordered.sort_by_key(|entry| entry.order);
        for entry in ordered.iter().filter(|entry| entry.before_texture_mode) {
            Self::upload(entry, apply);
        }
        if let Some(filter) = self.texture_mode {
            apply(&ImageResourceOperation::TextureMode { filter });
        }
        for entry in ordered.iter().filter(|entry| !entry.before_texture_mode) {
            Self::upload(entry, apply);
        }
    }

    fn upload(entry: &ResidentEntry, apply: &mut dyn FnMut(&ImageResourceOperation)) {
        apply(&entry.creation);
        let mut updates: Vec<&(u64, ImageResourceOperation)> = entry.updates.values().collect();
        updates.sort_by_key(|(order, _)| *order);
        for (_, update) in updates {
            apply(update);
        }
    }

    fn record(&mut self, operation: &ImageResourceOperation) -> Result<(), RenderError> {
        match operation {
            ImageResourceOperation::CreateImage { image, .. } => {
                let order = self.next_order;
                self.next_order += 1;
                self.resident.insert(
                    image.ordinal,
                    ResidentEntry {
                        order,
                        before_texture_mode: false,
                        creation: operation.clone(),
                        updates: HashMap::new(),
                    },
                );
            }
            ImageResourceOperation::UpdateImage { image, level, .. } => {
                let Some(entry) = self.resident.get_mut(&image.ordinal) else {
                    return Err(RenderError::UnknownImage(image.ordinal));
                };
                let order = self.next_order;
                self.next_order += 1;
                entry.updates.insert(*level, (order, operation.clone()));
            }
            ImageResourceOperation::ReleaseImage { image } => {
                self.resident.remove(&image.ordinal);
            }
            ImageResourceOperation::TextureMode { filter } => {
                self.texture_mode = Some(*filter);
                for entry in self.resident.values_mut() {
                    entry.before_texture_mode = true;
                }
            }
        }
        Ok(())
    }
}

/// Swap-notification callback: capture ids presented by `swap-buffers`.
pub type SwapCallback = Box<dyn FnMut(&[u32])>;

/// Headless driver: ordered frames straight into the backend, no threads.
pub struct SyncDriver<B: OrderedBackend> {
    backend: B,
    journal: ImageJournal,
    color: Vec4,
    on_swap: SwapCallback,
}

impl<B: OrderedBackend> SyncDriver<B> {
    /// Build a driver over `backend`, reporting swaps through `on_swap`.
    pub fn new(backend: B, on_swap: impl FnMut(&[u32]) + 'static) -> Self {
        Self {
            backend,
            journal: ImageJournal::new(),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            on_swap: Box::new(on_swap),
        }
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

    /// Replace the backend, replaying the journal onto it.
    pub fn replace_backend(&mut self, backend: B) {
        self.backend = backend;
        let backend = &mut self.backend;
        self.journal
            .replay(&mut |operation| backend.apply_image_resource(operation));
    }

    /// Execute one frame's commands in order.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::ForeignOwner`] when the frame belongs to
    /// another renderer lifetime, or [`RenderError::UnknownImage`] when an
    /// update names an ordinal with no resident image.
    pub fn execute_frame(&mut self, frame: &RenderFrame) -> Result<(), RenderError> {
        self.backend.owner().require(&frame.owner, "frame")?;
        for command in &frame.commands {
            match command {
                RenderCommand::DrawBuffer { buffer, clear } => {
                    self.backend.select_draw_buffer(*buffer, *clear);
                }
                RenderCommand::ImageResource(operation) => {
                    self.backend.apply_image_resource(operation);
                    self.journal.record(operation)?;
                }
                RenderCommand::SetColor(color) => self.color = *color,
                RenderCommand::StretchPic { rect, uv, image } => self.stretch_pic(rect, uv, image),
                RenderCommand::View(view) => {
                    self.operations(&view.before_view);
                    self.backend.begin_view(&view.state);
                    self.operations(&view.operations);
                }
                RenderCommand::SwapBuffers => (self.on_swap)(&[]),
            }
        }
        Ok(())
    }

    fn operations(&mut self, operations: &[RenderOperation]) {
        for operation in operations {
            match operation {
                RenderOperation::Draw(batches) => {
                    for batch in batches {
                        self.draw_batch(batch);
                    }
                }
                RenderOperation::ObjectOpacity { opacity, batches } => {
                    let backend = &mut self.backend;
                    backend.with_object_opacity(*opacity, |inner| {
                        for batch in batches {
                            let resolved = Self::resolve_batch(batch, &mut |upload| {
                                inner.apply_image_resource(&upload);
                            });
                            Self::draw_resolved(inner, &resolved);
                        }
                    });
                }
                other => self.backend.draw_immediate(other),
            }
        }
    }

    fn draw_batch(&mut self, batch: &DrawBatch) {
        let resolved = {
            let backend = &mut self.backend;
            Self::resolve_batch(batch, &mut |upload| {
                backend.apply_image_resource(&upload);
            })
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
mod tests {
    use qa_core::math::vec4;

    use super::super::types::{
        CullFace, DrawBuffer, ImageLevel, ImageResourceOperation, LevelContent, RenderCommand, RenderFrame,
        RenderImage, RenderOperation, RenderView, SourceTime, TextureSampling, ViewTarget,
    };
    use super::super::worker_runtime::support::{test_batch, test_image, test_view_state, FakeBackend};
    use super::*;

    fn frame_for(driver: &SyncDriver<FakeBackend>, commands: Vec<RenderCommand>) -> RenderFrame {
        RenderFrame {
            owner: driver.backend().owner.clone(),
            sequence: 1,
            commands,
        }
    }

    fn create_command(driver: &SyncDriver<FakeBackend>, ordinal: u32) -> RenderCommand {
        let image = test_image(&driver.backend().owner, ordinal);
        RenderCommand::ImageResource(ImageResourceOperation::CreateImage {
            image,
            content: RenderImage::Rgba8 {
                levels: vec![ImageLevel {
                    width: 4,
                    height: 4,
                    pixels: vec![1; 64],
                }],
                border_color: vec4(0.0, 0.0, 0.0, 1.0),
            },
            sampling: TextureSampling {
                repeat: false,
                filter: TextureFilter::Linear,
            },
        })
    }

    #[test]
    fn frame_runs_every_command_kind_in_order() {
        let swaps = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let swaps_inner = std::rc::Rc::clone(&swaps);
        let mut driver = SyncDriver::new(FakeBackend::new(), move |captures: &[u32]| {
            swaps_inner.borrow_mut().push(captures.to_vec());
        });
        let owner = driver.backend().owner.clone();
        let frame = frame_for(
            &driver,
            vec![
                RenderCommand::DrawBuffer {
                    buffer: DrawBuffer::Back,
                    clear: false,
                },
                RenderCommand::ImageResource(ImageResourceOperation::TextureMode {
                    filter: TextureFilter::Linear,
                }),
                RenderCommand::SetColor(vec4(1.0, 0.0, 0.0, 1.0)),
                RenderCommand::StretchPic {
                    rect: super::super::types::Rect {
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
                    image: test_image(&owner, 9),
                },
                RenderCommand::View(RenderView {
                    state: test_view_state(),
                    target: ViewTarget::Preview("main".to_string()),
                    time: SourceTime::Milliseconds(16.0),
                    before_view: vec![RenderOperation::Cull(CullFace::Back)],
                    operations: vec![
                        RenderOperation::Draw(vec![test_batch(&test_image(&owner, 1))]),
                        RenderOperation::ObjectOpacity {
                            opacity: 0.5,
                            batches: vec![test_batch(&test_image(&owner, 2))],
                        },
                        RenderOperation::DepthRange([0.0, 1.0]),
                    ],
                }),
                RenderCommand::SwapBuffers,
            ],
        );
        driver.execute_frame(&frame).unwrap();

        let log = &driver.backend().log;
        assert_eq!(log[0], "buffer:Back:false");
        assert_eq!(log[1], "image:mode");
        assert_eq!(log[2], "view:64x32");
        assert!(log[3].starts_with("prepare:6:"), "{log:?}");
        assert_eq!(&log[4..8], ["begin", "texture:0:image:9", "draw", "cleanup"]);
        assert_eq!(log[8], "immediate:cull");
        assert_eq!(log[9], "view:64x32");
        assert_eq!(&log[11..15], ["begin", "texture:0:image:1", "draw", "cleanup"]);
        assert_eq!(log[15], "opacity-begin:0.5");
        assert_eq!(log[21], "opacity-end");
        assert_eq!(log[22], "immediate:depth-range");
        assert_eq!(*swaps.borrow(), [Vec::<u32>::new()]);
    }

    #[test]
    fn foreign_owner_frame_is_rejected_untouched() {
        let mut driver = SyncDriver::new(FakeBackend::new(), |_: &[u32]| {});
        let mut foreign = driver.backend().owner.clone();
        foreign.generation += 1;
        let frame = RenderFrame {
            owner: foreign,
            sequence: 2,
            commands: vec![RenderCommand::SwapBuffers],
        };
        assert_eq!(
            driver.execute_frame(&frame),
            Err(RenderError::ForeignOwner("frame".to_string()))
        );
        assert!(driver.backend().log.is_empty());
    }

    #[test]
    fn update_without_resident_image_fails() {
        let mut driver = SyncDriver::new(FakeBackend::new(), |_: &[u32]| {});
        let owner = driver.backend().owner.clone();
        let frame = frame_for(
            &driver,
            vec![RenderCommand::ImageResource(ImageResourceOperation::UpdateImage {
                image: test_image(&owner, 9),
                level: 0,
                content: LevelContent::Rgba(ImageLevel {
                    width: 1,
                    height: 1,
                    pixels: vec![0; 4],
                }),
            })],
        );
        assert_eq!(driver.execute_frame(&frame), Err(RenderError::UnknownImage(9)));
    }

    #[test]
    fn replace_backend_replays_journal_in_donor_order() {
        let mut driver = SyncDriver::new(FakeBackend::new(), |_: &[u32]| {});
        let owner = driver.backend().owner.clone();
        let update = RenderCommand::ImageResource(ImageResourceOperation::UpdateImage {
            image: test_image(&owner, 0),
            level: 0,
            content: LevelContent::Rgba(ImageLevel {
                width: 2,
                height: 2,
                pixels: vec![2; 16],
            }),
        });
        let frame = frame_for(
            &driver,
            vec![
                create_command(&driver, 0),
                RenderCommand::ImageResource(ImageResourceOperation::TextureMode {
                    filter: TextureFilter::Nearest,
                }),
                create_command(&driver, 1),
                update,
                RenderCommand::ImageResource(ImageResourceOperation::ReleaseImage {
                    image: test_image(&owner, 1),
                }),
            ],
        );
        driver.execute_frame(&frame).unwrap();
        driver.replace_backend(FakeBackend::new());
        assert_eq!(
            driver.backend().log,
            ["image:create:0", "image:update:0:0", "image:mode",]
        );
    }
}
