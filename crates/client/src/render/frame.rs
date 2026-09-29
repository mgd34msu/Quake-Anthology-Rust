//! Ordered scene-frame assembly.
//!
//! Donor provenance: `src/render/commands/frame.ts` (`SceneFrameBuilder`,
//! `clipPicture`). One builder accumulates the commands for a single frame:
//! a draw-buffer selection, drained image operations, views, and 2D pictures,
//! closed by an optional buffer swap. Local seat pictures are clipped before
//! they enter the global queue.
//!
//! Session checks compare [`SessionId`] values for equality. [`SeatId`]
//! exposes no session accessor, so callers pass the seat's session alongside
//! seat-targeted submissions.

use qa_core::identity::SessionId;
use qa_core::math::Vec4;

use super::error::RenderError;
use super::types::{
    DrawBuffer, ImageResourceOperation, Rect, RenderCommand, RenderFrame, RenderView, RendererImage, TextureRect,
    ViewTarget,
};
use crate::render::scene::resources::SceneImageRegistry;
use crate::render::scene::world::PreparedWorldView;

/// Clip a picture rectangle (plus its UVs) to `clip`, normalizing inverted
/// rectangles. Returns `None` for empty or fully clipped pictures.
#[must_use]
pub fn clip_picture(rect: &Rect, uv: &TextureRect, clip: &Rect) -> Option<(Rect, TextureRect)> {
    if rect.width == 0.0 || rect.height == 0.0 {
        return None;
    }
    let left = rect.x.min(rect.x + rect.width).max(clip.x);
    let right = rect.x.max(rect.x + rect.width).min(clip.x + clip.width);
    let top = rect.y.min(rect.y + rect.height).max(clip.y);
    let bottom = rect.y.max(rect.y + rect.height).min(clip.y + clip.height);
    if left >= right || top >= bottom {
        return None;
    }
    Some((
        Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        },
        TextureRect {
            s1: uv.s1 + (uv.s2 - uv.s1) * (left - rect.x) / rect.width,
            s2: uv.s1 + (uv.s2 - uv.s1) * (right - rect.x) / rect.width,
            t1: uv.t1 + (uv.t2 - uv.t1) * (top - rect.y) / rect.height,
            t2: uv.t1 + (uv.t2 - uv.t1) * (bottom - rect.y) / rect.height,
        },
    ))
}

/// Accumulates the ordered commands for one frame.
pub struct SceneFrameBuilder {
    /// Image registry feeding the frame: ownership checks and drained uploads.
    pub images: SceneImageRegistry,
    commands: Vec<RenderCommand>,
    sequence: u64,
    active: bool,
}

impl SceneFrameBuilder {
    /// Builder over an image registry.
    #[must_use]
    pub fn new(images: SceneImageRegistry) -> Self {
        Self {
            images,
            commands: Vec::new(),
            sequence: 0,
            active: false,
        }
    }

    /// Open a frame, selecting the draw buffer and flushing queued uploads.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::OutOfOrder`] when a frame is already open.
    pub fn begin(&mut self, buffer: DrawBuffer, clear: bool) -> Result<(), RenderError> {
        if self.active {
            return Err(RenderError::OutOfOrder(
                "A scene frame is already being prepared".to_string(),
            ));
        }
        self.active = true;
        self.commands = vec![RenderCommand::DrawBuffer { buffer, clear }];
        let operations = self.images.drain_operations();
        self.resources(&operations)
    }

    /// Append image resource operations to the open frame.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::OutOfOrder`] when no frame is open.
    pub fn resources(&mut self, operations: &[ImageResourceOperation]) -> Result<(), RenderError> {
        self.require_active()?;
        self.commands
            .extend(operations.iter().cloned().map(RenderCommand::ImageResource));
        Ok(())
    }

    /// Append a view, flushing registry uploads queued since the last view.
    /// Seat-targeted views must belong to the registry's session; previews
    /// are session-independent.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::OutOfOrder`] when no frame is open, or
    /// [`RenderError::ForeignOwner`] when a seat view belongs to another
    /// session.
    pub fn view(&mut self, session: &SessionId, view: RenderView) -> Result<(), RenderError> {
        self.require_active()?;
        if matches!(view.target, ViewTarget::Seat(_)) && session != &self.images.owner().session {
            return Err(RenderError::ForeignOwner(
                "Render seat belongs to another session".to_string(),
            ));
        }
        let operations = self.images.drain_operations();
        self.resources(&operations)?;
        self.commands.push(RenderCommand::View(view));
        Ok(())
    }

    /// Append a prepared world view: its image operations, then the view.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::OutOfOrder`] when no frame is open, or
    /// [`RenderError::ForeignOwner`] when a seat view belongs to another
    /// session.
    pub fn world(&mut self, session: &SessionId, prepared: &PreparedWorldView) -> Result<(), RenderError> {
        self.resources(&prepared.image_operations)?;
        self.view(session, prepared.view.clone())
    }

    /// Append a seat-local 2D picture, clipped to the viewport and offset
    /// into framebuffer coordinates. Fully clipped pictures add nothing.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::ForeignOwner`] when the seat belongs to another
    /// session or the image to another owner, or [`RenderError::OutOfOrder`]
    /// when no frame is open.
    #[allow(clippy::too_many_arguments)]
    pub fn picture(
        &mut self,
        session: &SessionId,
        viewport: &Rect,
        rect: &Rect,
        image: &RendererImage,
        uv: &TextureRect,
        color: Vec4,
    ) -> Result<(), RenderError> {
        if session != &self.images.owner().session {
            return Err(RenderError::ForeignOwner(
                "HUD seat belongs to another session".to_string(),
            ));
        }
        self.images.require(image)?;
        let clip = Rect {
            x: 0.0,
            y: 0.0,
            width: viewport.width,
            height: viewport.height,
        };
        let Some((clipped_rect, clipped_uv)) = clip_picture(rect, uv, &clip) else {
            return Ok(());
        };
        self.command(RenderCommand::SetColor(color))?;
        self.command(RenderCommand::StretchPic {
            rect: Rect {
                x: viewport.x + clipped_rect.x,
                y: viewport.y + clipped_rect.y,
                width: clipped_rect.width,
                height: clipped_rect.height,
            },
            uv: clipped_uv,
            image: image.clone(),
        })
    }

    /// Append one command. Buffer swaps belong to [`finish`](Self::finish).
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::OutOfOrder`] when no frame is open or when
    /// submitting a buffer swap directly.
    pub fn command(&mut self, command: RenderCommand) -> Result<(), RenderError> {
        self.require_active()?;
        if matches!(command, RenderCommand::SwapBuffers) {
            return Err(RenderError::OutOfOrder(
                "Buffer swaps are submitted by finishing the frame".to_string(),
            ));
        }
        self.commands.push(command);
        Ok(())
    }

    /// Close the frame, appending a buffer swap unless `present` is false.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::OutOfOrder`] when no frame is open.
    pub fn finish(&mut self, present: bool) -> Result<RenderFrame, RenderError> {
        self.require_active()?;
        if present {
            self.commands.push(RenderCommand::SwapBuffers);
        }
        let frame = RenderFrame {
            owner: self.images.owner().clone(),
            sequence: self.sequence,
            commands: std::mem::take(&mut self.commands),
        };
        self.sequence += 1;
        self.active = false;
        Ok(frame)
    }

    /// Abandon the open frame without producing output.
    pub fn discard(&mut self) {
        self.commands.clear();
        self.active = false;
    }

    fn require_active(&self) -> Result<(), RenderError> {
        if self.active {
            Ok(())
        } else {
            Err(RenderError::OutOfOrder(
                "Begin a scene frame before submitting commands".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec4;

    use super::super::types::{
        ImageLevel, RenderImage, RenderViewState, ResourceOwner, SourceTime, TextureFilter, TextureSampling,
    };
    use super::*;

    fn authority(name: &str) -> IdentityOwner {
        IdentityOwner::create(name).unwrap()
    }

    fn builder(session: &SessionId) -> SceneFrameBuilder {
        SceneFrameBuilder::new(SceneImageRegistry::new(ResourceOwner::new(1, session.clone(), 0)))
    }

    fn viewport() -> Rect {
        Rect {
            x: 100.0,
            y: 50.0,
            width: 640.0,
            height: 480.0,
        }
    }

    fn render_view(target: ViewTarget) -> RenderView {
        RenderView {
            state: RenderViewState {
                viewport: viewport(),
                clear: None,
                clip_plane: None,
            },
            target,
            time: SourceTime::Seconds(0.0),
            before_view: Vec::new(),
            operations: Vec::new(),
        }
    }

    fn full_uv() -> TextureRect {
        TextureRect {
            s1: 0.0,
            t1: 0.0,
            s2: 1.0,
            t2: 1.0,
        }
    }

    fn registered_image(builder: &mut SceneFrameBuilder) -> RendererImage {
        builder
            .images
            .register(
                "pic",
                RenderImage::Rgba8 {
                    levels: vec![ImageLevel {
                        width: 64,
                        height: 64,
                        pixels: vec![1; 64 * 64 * 4],
                    }],
                    border_color: vec4(0.0, 0.0, 0.0, 1.0),
                },
                TextureSampling {
                    repeat: false,
                    filter: TextureFilter::Linear,
                },
            )
            .unwrap()
    }

    #[test]
    fn begin_finish_orders_commands_and_bumps_sequence() {
        let owner = authority("frame-order");
        let mut builder = builder(owner.session());
        builder.begin(DrawBuffer::Back, false).unwrap();
        builder
            .command(RenderCommand::SetColor(vec4(1.0, 1.0, 1.0, 1.0)))
            .unwrap();
        let frame = builder.finish(true).unwrap();
        assert_eq!(frame.sequence, 0);
        assert_eq!(frame.owner, *builder.images.owner());
        assert_eq!(frame.commands.len(), 3);
        assert!(matches!(
            frame.commands[0],
            RenderCommand::DrawBuffer {
                buffer: DrawBuffer::Back,
                clear: false
            }
        ));
        assert!(matches!(frame.commands[1], RenderCommand::SetColor(_)));
        assert!(matches!(frame.commands[2], RenderCommand::SwapBuffers));
        builder.begin(DrawBuffer::Front, true).unwrap();
        let second = builder.finish(false).unwrap();
        assert_eq!(second.sequence, 1);
        assert_eq!(second.commands.len(), 1);
    }

    #[test]
    fn out_of_order_calls_fail() {
        let owner = authority("frame-ooo");
        let mut builder = builder(owner.session());
        assert!(matches!(
            builder.command(RenderCommand::SwapBuffers),
            Err(RenderError::OutOfOrder(_))
        ));
        assert!(matches!(builder.finish(true), Err(RenderError::OutOfOrder(_))));
        assert!(matches!(builder.resources(&[]), Err(RenderError::OutOfOrder(_))));
        builder.begin(DrawBuffer::Back, false).unwrap();
        assert!(matches!(
            builder.begin(DrawBuffer::Back, false),
            Err(RenderError::OutOfOrder(_))
        ));
        assert!(matches!(
            builder.command(RenderCommand::SwapBuffers),
            Err(RenderError::OutOfOrder(_))
        ));
    }

    #[test]
    fn discard_abandons_and_releases() {
        let owner = authority("frame-discard");
        let mut builder = builder(owner.session());
        builder.begin(DrawBuffer::Back, false).unwrap();
        builder
            .command(RenderCommand::SetColor(vec4(0.0, 0.0, 0.0, 1.0)))
            .unwrap();
        builder.discard();
        assert!(matches!(builder.finish(true), Err(RenderError::OutOfOrder(_))));
        builder.begin(DrawBuffer::Back, false).unwrap();
        let frame = builder.finish(false).unwrap();
        assert_eq!(frame.commands.len(), 1);
    }

    #[test]
    fn views_flush_uploads_and_check_seat_sessions() {
        let owner = authority("frame-view");
        let other = authority("frame-view-other");
        let mut builder = builder(owner.session());
        builder.begin(DrawBuffer::Back, false).unwrap();
        let seat = owner.seat(0);
        builder
            .view(owner.session(), render_view(ViewTarget::Seat(seat)))
            .unwrap();
        let foreign_seat = other.seat(0);
        assert!(matches!(
            builder.view(other.session(), render_view(ViewTarget::Seat(foreign_seat))),
            Err(RenderError::ForeignOwner(_))
        ));
        builder
            .view(other.session(), render_view(ViewTarget::Preview("map".to_string())))
            .unwrap();
        let frame = builder.finish(false).unwrap();
        assert_eq!(frame.commands.len(), 3, "draw-buffer plus two views");
    }

    #[test]
    fn world_appends_operations_before_its_view() {
        let owner = authority("frame-world");
        let mut builder = builder(owner.session());
        builder.begin(DrawBuffer::Back, false).unwrap();
        let prepared = PreparedWorldView {
            image_operations: vec![ImageResourceOperation::TextureMode {
                filter: TextureFilter::Linear,
            }],
            view: render_view(ViewTarget::Preview("world".to_string())),
        };
        builder.world(owner.session(), &prepared).unwrap();
        let frame = builder.finish(false).unwrap();
        assert!(matches!(
            frame.commands[1],
            RenderCommand::ImageResource(ImageResourceOperation::TextureMode { .. })
        ));
        assert!(matches!(frame.commands[2], RenderCommand::View(_)));
    }

    #[test]
    fn picture_clips_offsets_and_rejects_foreign_seats() {
        let owner = authority("frame-picture");
        let other = authority("frame-picture-other");
        let mut builder = builder(owner.session());
        builder.begin(DrawBuffer::Back, false).unwrap();
        let image = registered_image(&mut builder);
        let rect = Rect {
            x: -10.0,
            y: 20.0,
            width: 100.0,
            height: 40.0,
        };
        builder
            .picture(
                owner.session(),
                &viewport(),
                &rect,
                &image,
                &full_uv(),
                vec4(1.0, 1.0, 1.0, 1.0),
            )
            .unwrap();
        assert!(matches!(
            builder.picture(
                other.session(),
                &viewport(),
                &rect,
                &image,
                &full_uv(),
                vec4(1.0, 1.0, 1.0, 1.0),
            ),
            Err(RenderError::ForeignOwner(_))
        ));
        let foreign_image = RendererImage {
            owner: ResourceOwner::new(9, other.session().clone(), 0),
            ..image.clone()
        };
        assert!(matches!(
            builder.picture(
                owner.session(),
                &viewport(),
                &rect,
                &foreign_image,
                &full_uv(),
                vec4(1.0, 1.0, 1.0, 1.0),
            ),
            Err(RenderError::ForeignOwner(_))
        ));
        let frame = builder.finish(false).unwrap();
        assert_eq!(frame.commands.len(), 3);
        assert!(matches!(frame.commands[1], RenderCommand::SetColor(_)));
        let RenderCommand::StretchPic { rect, uv, image: bound } = &frame.commands[2] else {
            panic!("expected stretch-pic");
        };
        assert_eq!(bound, &image);
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (100.0, 70.0, 90.0, 40.0));
        assert!((uv.s1 - 0.1).abs() < 1e-6);
        assert_eq!((uv.s2, uv.t1, uv.t2), (1.0, 0.0, 1.0));
    }

    #[test]
    fn fully_clipped_picture_adds_nothing() {
        let owner = authority("frame-clip-none");
        let mut builder = builder(owner.session());
        builder.begin(DrawBuffer::Back, false).unwrap();
        let image = registered_image(&mut builder);
        builder
            .picture(
                owner.session(),
                &viewport(),
                &Rect {
                    x: 700.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                &image,
                &full_uv(),
                vec4(1.0, 1.0, 1.0, 1.0),
            )
            .unwrap();
        let frame = builder.finish(false).unwrap();
        assert_eq!(frame.commands.len(), 1);
    }

    #[test]
    fn clip_picture_handles_edges() {
        let clip = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        assert_eq!(
            clip_picture(
                &Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 20.0,
                    height: 20.0,
                },
                &full_uv(),
                &clip
            ),
            Some((
                Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 20.0,
                    height: 20.0,
                },
                full_uv()
            ))
        );
        assert_eq!(
            clip_picture(
                &Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 0.0,
                    height: 5.0,
                },
                &full_uv(),
                &clip
            ),
            None
        );
        assert_eq!(
            clip_picture(
                &Rect {
                    x: 200.0,
                    y: 200.0,
                    width: 10.0,
                    height: 10.0,
                },
                &full_uv(),
                &clip
            ),
            None
        );
        let (rect, _) = clip_picture(
            &Rect {
                x: 30.0,
                y: 30.0,
                width: -20.0,
                height: -20.0,
            },
            &full_uv(),
            &clip,
        )
        .unwrap();
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (10.0, 10.0, 20.0, 20.0));
    }
}
