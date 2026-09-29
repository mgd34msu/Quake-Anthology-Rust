//! Ordered CPU command execution.
//!
//! Donor provenance: `src/render/cpu/commands.ts` in full — ordered commands
//! following id Software `tr_cmds.c` and `tr_backend.c`. Copyright (C)
//! 1999-2005 Id Software, Inc.

use qa_core::math::{vec4, Vec4};

use super::super::types::{OrderedBackend, RenderCommand, RenderFrame, RenderOperation};
use super::rasterizer::SoftwareRenderer;

/// Presentation surface for finished CPU frames.
pub trait CpuPresenter {
    /// Backend name; CPU rendering requires `"cpu"`.
    fn backend(&self) -> &str;
    /// Drawable size in pixels.
    fn drawable_size(&self) -> (u32, u32);
    /// Present finished RGBA pixels.
    fn present(&mut self, pixels: &[u8]);
}

/// The window owns presentation; this target owns command ordering and CPU rendering.
pub struct CpuRenderTarget {
    backend: SoftwareRenderer,
    presenter: Option<Box<dyn CpuPresenter>>,
    color: Vec4,
}

impl CpuRenderTarget {
    /// Create a headless target.
    #[must_use]
    pub fn new(backend: SoftwareRenderer) -> Self {
        Self {
            backend,
            presenter: None,
            color: vec4(1.0, 1.0, 1.0, 1.0),
        }
    }

    /// Create a presenting target.
    pub fn with_presenter(backend: SoftwareRenderer, presenter: Box<dyn CpuPresenter>) -> Self {
        if presenter.backend() != "cpu" {
            panic!("CPU rendering requires an SDL CPU window");
        }
        Self {
            backend,
            presenter: Some(presenter),
            color: vec4(1.0, 1.0, 1.0, 1.0),
        }
    }

    /// Borrow the backend.
    #[must_use]
    pub const fn backend(&self) -> &SoftwareRenderer {
        &self.backend
    }

    /// Mutably borrow the backend.
    pub fn backend_mut(&mut self) -> &mut SoftwareRenderer {
        &mut self.backend
    }

    fn operations(&mut self, operations: &[RenderOperation]) {
        for operation in operations {
            match operation {
                RenderOperation::Draw(batches) => {
                    for batch in batches {
                        self.backend.draw(batch);
                    }
                }
                RenderOperation::ObjectOpacity { opacity, batches } => {
                    self.backend.with_object_opacity(*opacity, |backend| {
                        for batch in batches {
                            backend.draw(batch);
                        }
                    });
                }
                operation => self.backend.draw_immediate(operation),
            }
        }
    }

    /// Execute one frame's commands in order.
    pub fn execute(&mut self, frame: &RenderFrame) {
        let owner = self.backend.owner().clone();
        owner
            .require(&frame.owner, "CPU frame belongs to another renderer owner")
            .unwrap_or_else(|error| panic!("{error}"));
        for command in &frame.commands {
            match command {
                RenderCommand::ImageResource(operation) => self.backend.apply_image_resource(operation),
                RenderCommand::DrawBuffer { buffer, clear } => self.backend.select_draw_buffer(*buffer, *clear),
                RenderCommand::SetColor(color) => self.color = *color,
                RenderCommand::StretchPic { rect, uv, image } => {
                    let color = self.color;
                    self.backend.draw_stretch_pic(image, rect, uv, &color);
                }
                RenderCommand::View(view) => {
                    self.operations(&view.before_view);
                    self.backend.begin_view(&view.state);
                    self.operations(&view.operations);
                }
                RenderCommand::SwapBuffers => self.present(),
            }
        }
    }

    /// Finish and present the frame.
    pub fn present(&mut self) {
        self.backend.finish();
        let Some(presenter) = self.presenter.as_mut() else {
            return;
        };
        let (width, height) = presenter.drawable_size();
        if width != self.backend.width() || height != self.backend.height() {
            panic!("CPU framebuffer dimensions must match the SDL drawable");
        }
        presenter.present(self.backend.pixels());
    }

    /// Release backend resources.
    pub fn close(&mut self) {
        self.backend.close();
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec4;

    use super::super::super::types::{
        fresh_owner_identity, ImageLevel, ImageResourceOperation, ImageSource, RenderImage, RendererImage,
        ResourceOwner, TextureFilter, TextureSampling,
    };
    use super::*;

    struct TestPresenter {
        size: (u32, u32),
        presented: Vec<u8>,
    }

    impl CpuPresenter for TestPresenter {
        fn backend(&self) -> &str {
            "cpu"
        }

        fn drawable_size(&self) -> (u32, u32) {
            self.size
        }

        fn present(&mut self, pixels: &[u8]) {
            self.presented = pixels.to_vec();
        }
    }

    #[test]
    fn frame_uploads_stretches_and_presents() {
        let authority = IdentityOwner::create("cpu-commands").unwrap();
        let owner = ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0);
        let mut target = CpuRenderTarget::new(SoftwareRenderer::new(2, 2, owner.clone()));
        let handle = RendererImage {
            owner: owner.clone(),
            ordinal: 1,
            source: ImageSource::Generated {
                name: "pic".to_string(),
            },
            width: 2,
            height: 2,
        };
        target.execute(&RenderFrame {
            owner: owner.clone(),
            sequence: 0,
            commands: vec![
                RenderCommand::ImageResource(ImageResourceOperation::CreateImage {
                    image: handle.clone(),
                    content: RenderImage::Rgba8 {
                        levels: vec![ImageLevel {
                            width: 2,
                            height: 2,
                            pixels: [10u8, 20, 30, 255].repeat(4),
                        }],
                        border_color: vec4(0.0, 0.0, 0.0, 0.0),
                    },
                    sampling: TextureSampling {
                        repeat: true,
                        filter: TextureFilter::Nearest,
                    },
                }),
                RenderCommand::SetColor(vec4(1.0, 1.0, 1.0, 1.0)),
                RenderCommand::StretchPic {
                    rect: super::super::super::types::Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 2.0,
                        height: 2.0,
                    },
                    uv: super::super::super::types::TextureRect {
                        s1: 0.0,
                        t1: 0.0,
                        s2: 1.0,
                        t2: 1.0,
                    },
                    image: handle,
                },
                RenderCommand::SwapBuffers,
            ],
        });
        assert_eq!(target.backend().pixels(), [10u8, 20, 30, 255].repeat(4).as_slice());
        target.close();
    }

    #[test]
    fn presenter_receives_finished_pixels() {
        let authority = IdentityOwner::create("cpu-present").unwrap();
        let owner = ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0);
        let presenter = Box::new(TestPresenter {
            size: (1, 1),
            presented: Vec::new(),
        });
        let mut target = CpuRenderTarget::with_presenter(SoftwareRenderer::new(1, 1, owner.clone()), presenter);
        target.execute(&RenderFrame {
            owner,
            sequence: 1,
            commands: vec![RenderCommand::SwapBuffers],
        });
        assert_eq!(target.backend().pixels(), &[0u8, 0, 0, 0][..]);
        target.close();
    }

    #[test]
    #[should_panic(expected = "another renderer owner")]
    fn foreign_frame_is_rejected() {
        let authority = IdentityOwner::create("cpu-foreign").unwrap();
        let owner = ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0);
        let mut target = CpuRenderTarget::new(SoftwareRenderer::new(1, 1, owner));
        let foreign_owner = ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0);
        target.execute(&RenderFrame {
            owner: foreign_owner,
            sequence: 0,
            commands: Vec::new(),
        });
    }
}
