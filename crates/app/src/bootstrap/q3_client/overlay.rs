//! Quake III HUD overlay submission order.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-client/overlay.ts`
//! (`drawQ3Overlay`, `clippedCamera`). Original pictures, glyphs, and
//! model icons retain their interleaved source order between white
//! set-color brackets. The port owns submission order, stretch-pic
//! clipping, seat/render-flag validation, and the cropping camera
//! math; text-batch and scene emission dispatch through
//! [`Q3OverlayRenderer`] because the material/world stack lives outside
//! this wave's scope. Camera clipping applies to the renderer's own
//! camera: the presented scene camera is a projection-less content
//! mirror, so the seam clips when it builds the real view.

use qa_client::render::frame::{clip_picture, SceneFrameBuilder};
use qa_client::render::material2d::MaterialTextDraw;
use qa_client::render::types::{Rect as RenderRect, RenderCommand, RendererImage, TextureRect};
use qa_client::view::SceneCamera;
use qa_content::q3::presentation::refdef::RDF_NOWORLDMODEL;
use qa_content::q3::presentation::scene::Q3PresentedScene;
use qa_core::identity::SeatId;
use qa_core::math::Vec4;
use thiserror::Error;

/// Overlay 2D command (donor `set-color` / `stretch-pic` commands).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3OverlayCommand {
    /// Set the 2D drawing color.
    SetColor(Vec4),
    /// Stretch a picture.
    StretchPic {
        /// Destination rectangle.
        rect: RenderRect,
        /// Source coordinates.
        uv: TextureRect,
        /// Picture image.
        image: RendererImage,
    },
}

/// One overlay submission (donor `Q3OverlaySubmission`).
#[derive(Debug, Clone)]
pub enum Q3OverlaySubmission<'a> {
    /// 2D command.
    Command(Q3OverlayCommand),
    /// HUD text with its owning seat.
    Text {
        /// Owning seat.
        seat: SeatId,
        /// Text draw.
        draw: MaterialTextDraw<'a>,
    },
    /// HUD scene without a world model.
    Scene(Box<Q3PresentedScene>),
}

/// Overlay view target shared by text and scene emission.
#[derive(Debug, Clone, Copy)]
pub struct Q3OverlayTarget<'a> {
    /// Overlay camera (text material context).
    pub camera: &'a SceneCamera,
    /// Overlay viewport (scene clip).
    pub viewport: &'a RenderRect,
    /// Presenting seat.
    pub seat: &'a SeatId,
    /// Time in milliseconds.
    pub time_ms: i32,
}

/// Overlay failure.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum Q3OverlayError {
    /// Source HUD text belongs to another seat.
    #[error("Source HUD text belongs to another seat")]
    WrongSeat,
    /// Source HUD cannot replace a world view or another seat.
    #[error("Source HUD cannot replace a world view or another seat")]
    WorldViewRejected,
    /// Frame submission failed.
    #[error(transparent)]
    Render(#[from] qa_client::render::error::RenderError),
}

/// Text and scene emission for overlay submissions.
pub trait Q3OverlayRenderer {
    /// Emit prepared HUD text.
    fn draw_text(
        &mut self,
        draw: &MaterialTextDraw<'_>,
        target: &Q3OverlayTarget<'_>,
        frames: &mut SceneFrameBuilder,
    ) -> Result<(), Q3OverlayError>;
    /// Emit a clipped HUD scene without a world model.
    fn draw_scene(
        &mut self,
        scene: &Q3PresentedScene,
        target: &Q3OverlayTarget<'_>,
        frames: &mut SceneFrameBuilder,
    ) -> Result<(), Q3OverlayError>;
}

fn white() -> Vec4 {
    Vec4 {
        x: 1.0,
        y: 1.0,
        z: 1.0,
        w: 1.0,
    }
}

/// Crop a camera to a clip rectangle, preserving the source icon's pixel
/// size and position. Returns `None` when the crop is empty.
#[must_use]
pub fn clipped_camera(camera: &SceneCamera, clip: &RenderRect) -> Option<SceneCamera> {
    let area = camera.viewport;
    let (ax, ay, aw, ah) = (area.x as f32, area.y as f32, area.width as f32, area.height as f32);
    let x = ax.max(clip.x);
    let y = ay.max(clip.y);
    let width = (ax + aw).min(clip.x + clip.width) - x;
    let height = (ay + ah).min(clip.y + clip.height) - y;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    if x == ax && y == ay && width == aw && height == ah {
        return Some(*camera);
    }
    let sx = aw / width;
    let sy = ah / height;
    let tx = (2.0 * (ax - x) + aw - width) / width;
    let ty = -((2.0 * (ay - y) + ah - height) / height);
    let p = camera.projection;
    Some(SceneCamera {
        viewport: qa_client::view::Rect {
            x: x.round() as i32,
            y: y.round() as i32,
            width: width.round() as i32,
            height: height.round() as i32,
        },
        projection: [
            sx * p[0] + tx * p[3],
            sy * p[1] + ty * p[3],
            p[2],
            p[3],
            sx * p[4] + tx * p[7],
            sy * p[5] + ty * p[7],
            p[6],
            p[7],
            sx * p[8] + tx * p[11],
            sy * p[9] + ty * p[11],
            p[10],
            p[11],
            sx * p[12] + tx * p[15],
            sy * p[13] + ty * p[15],
            p[14],
            p[15],
        ],
        ..*camera
    })
}

/// Draw overlay submissions in source order.
#[allow(clippy::too_many_arguments)]
pub fn draw_q3_overlay<R: Q3OverlayRenderer>(
    renderer: &mut R,
    submissions: &[Q3OverlaySubmission<'_>],
    frames: &mut SceneFrameBuilder,
    camera: &SceneCamera,
    viewport: &RenderRect,
    seat: &SeatId,
    time_ms: i32,
) -> Result<(), Q3OverlayError> {
    let target = Q3OverlayTarget {
        camera,
        viewport,
        seat,
        time_ms,
    };
    frames.command(RenderCommand::SetColor(white()))?;
    for submission in submissions {
        match submission {
            Q3OverlaySubmission::Command(Q3OverlayCommand::SetColor(color)) => {
                frames.command(RenderCommand::SetColor(*color))?;
            }
            Q3OverlaySubmission::Command(Q3OverlayCommand::StretchPic { rect, uv, image }) => {
                if let Some((rect, uv)) = clip_picture(rect, uv, viewport) {
                    frames.command(RenderCommand::StretchPic {
                        rect,
                        uv,
                        image: image.clone(),
                    })?;
                }
            }
            Q3OverlaySubmission::Text { seat: draw_seat, draw } => {
                if draw_seat != seat {
                    return Err(Q3OverlayError::WrongSeat);
                }
                renderer.draw_text(draw, &target, frames)?;
            }
            Q3OverlaySubmission::Scene(scene) => {
                if scene.source.render_flags & RDF_NOWORLDMODEL == 0 || scene.seat != *seat {
                    return Err(Q3OverlayError::WorldViewRejected);
                }
                renderer.draw_scene(scene, &target, frames)?;
            }
        }
    }
    frames.command(RenderCommand::SetColor(white()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::materials::compile::{
        compile_shader_script, CompileOptions, RegisteredImage, ShaderRegistrationHost, SourceImageRequest,
    };
    use qa_client::render::scene::resources::SceneImageRegistry;
    use qa_client::render::types::{DrawBuffer, ImageSource, ResourceOwner};
    use qa_client::view::{CameraClip, Rect as ViewRect};
    use qa_content::q3::presentation::refdef::Refdef;
    use qa_content::q3::presentation::scene::{
        snapshot_q3_scene_admission, Q3SceneAdmission, Q3SceneContent, SceneAdmissionOrigin,
    };
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, vec4};

    struct ShaderHost;

    impl ShaderRegistrationHost for ShaderHost {
        fn white_image(&self) -> RegisteredImage {
            RegisteredImage { image: 1, tmu: 0 }
        }
        fn default_image(&self) -> RegisteredImage {
            RegisteredImage { image: 2, tmu: 0 }
        }
        fn lightmap_image(&self) -> RegisteredImage {
            RegisteredImage { image: 3, tmu: 1 }
        }
        fn find_image(&mut self, _request: &SourceImageRequest) -> Option<RegisteredImage> {
            Some(RegisteredImage { image: 4, tmu: 0 })
        }
        fn play_shader_cinematic(
            &mut self,
            _name: &str,
        ) -> Option<qa_client::materials::compile::RegisteredShaderVideo> {
            None
        }
        fn apply_sun(&mut self, _sun: qa_client::materials::material::RegisteredSun) {}
        fn initialize_sky_tex_coords(&mut self, _height: f32) {}
        fn print_warning(&mut self, _message: &str) {}
    }

    struct StubRenderer {
        texts: usize,
        scenes: usize,
    }

    impl Q3OverlayRenderer for StubRenderer {
        fn draw_text(
            &mut self,
            _draw: &MaterialTextDraw<'_>,
            target: &Q3OverlayTarget<'_>,
            frames: &mut SceneFrameBuilder,
        ) -> Result<(), Q3OverlayError> {
            self.texts += 1;
            assert_eq!(target.time_ms, 99);
            frames.command(RenderCommand::SetColor(vec4(0.0, 1.0, 0.0, 1.0)))?;
            Ok(())
        }

        fn draw_scene(
            &mut self,
            scene: &Q3PresentedScene,
            _target: &Q3OverlayTarget<'_>,
            _frames: &mut SceneFrameBuilder,
        ) -> Result<(), Q3OverlayError> {
            self.scenes += 1;
            assert_ne!(scene.source.render_flags & RDF_NOWORLDMODEL, 0);
            Ok(())
        }
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("q3-overlay-test").expect("owner")
    }

    fn frame_builder(registry: &IdentityOwner) -> SceneFrameBuilder {
        let owner = ResourceOwner {
            identity: 1,
            session: registry.session().clone(),
            generation: 0,
        };
        let mut frames = SceneFrameBuilder::new(SceneImageRegistry::new(owner));
        frames.begin(DrawBuffer::Back, false).expect("begin");
        frames
    }

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            viewport: ViewRect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn image(registry: &IdentityOwner) -> RendererImage {
        RendererImage {
            owner: ResourceOwner {
                identity: 1,
                session: registry.session().clone(),
                generation: 0,
            },
            ordinal: 3,
            source: ImageSource::Generated {
                name: "pic".to_string(),
            },
            width: 64,
            height: 64,
        }
    }

    fn admission() -> Q3SceneAdmission {
        snapshot_q3_scene_admission(SceneAdmissionOrigin::Native, Vec::new(), Vec::new())
    }

    fn presented(_registry: &IdentityOwner, render_flags: i32, seat: SeatId) -> Q3PresentedScene {
        Q3PresentedScene {
            content: Q3SceneContent {
                admission: admission(),
                models: Vec::new(),
                effects: Vec::new(),
                special_entities: Vec::new(),
                portals: Vec::new(),
                lights: Vec::new(),
            },
            seat,
            viewport: qa_content::q3::presentation::scene::Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            camera: qa_content::q3::presentation::scene::SceneCamera {
                viewport: qa_content::q3::presentation::scene::Rect {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                },
                origin: vec3(0.0, 0.0, 0.0),
                axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                projection: qa_content::q3::presentation::scene::perspective_projection(90.0, 90.0, 1.0, 1024.0),
            },
            source: Refdef {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
                fov_x: 90.0,
                fov_y: 90.0,
                view_origin: vec3(0.0, 0.0, 0.0),
                view_axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                render_flags,
                ..Default::default()
            },
        }
    }

    #[test]
    fn covering_clip_returns_camera() {
        let camera = camera();
        let clip = RenderRect {
            x: -10.0,
            y: -10.0,
            width: 700.0,
            height: 520.0,
        };
        assert_eq!(clipped_camera(&camera, &clip), Some(camera));
    }

    #[test]
    fn empty_crop_returns_none() {
        let camera = camera();
        let clip = RenderRect {
            x: 700.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        assert_eq!(clipped_camera(&camera, &clip), None);
    }

    #[test]
    fn partial_crop_scales_projection() {
        let camera = camera();
        let clip = RenderRect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 480.0,
        };
        let cropped = clipped_camera(&camera, &clip).expect("cropped");
        assert_eq!(
            cropped.viewport,
            ViewRect {
                x: 0,
                y: 0,
                width: 320,
                height: 480
            }
        );
        assert_eq!(cropped.projection[0], 2.0);
        assert_eq!(cropped.projection[5], 1.0);
        assert_eq!(cropped.projection[12], 1.0);
    }

    #[test]
    fn commands_keep_source_order_between_white_brackets() {
        let registry = owner();
        let mut frames = frame_builder(&registry);
        let seat = registry.seat(0);
        let viewport = RenderRect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        let uv = TextureRect {
            s1: 0.0,
            t1: 0.0,
            s2: 1.0,
            t2: 1.0,
        };
        let mut renderer = StubRenderer { texts: 0, scenes: 0 };
        draw_q3_overlay(
            &mut renderer,
            &[
                Q3OverlaySubmission::Command(Q3OverlayCommand::SetColor(vec4(1.0, 0.0, 0.0, 1.0))),
                Q3OverlaySubmission::Command(Q3OverlayCommand::StretchPic {
                    rect: RenderRect {
                        x: 600.0,
                        y: 0.0,
                        width: 100.0,
                        height: 100.0,
                    },
                    uv,
                    image: image(&registry),
                }),
                Q3OverlaySubmission::Command(Q3OverlayCommand::StretchPic {
                    rect: RenderRect {
                        x: 900.0,
                        y: 0.0,
                        width: 10.0,
                        height: 10.0,
                    },
                    uv,
                    image: image(&registry),
                }),
            ],
            &mut frames,
            &camera(),
            &viewport,
            &seat,
            99,
        )
        .expect("overlay");
        let frame = frames.finish(false).expect("finish");
        // Draw buffer, white, red, clipped picture, white; the offscreen
        // picture drops out.
        assert_eq!(frame.commands.len(), 5);
        assert!(matches!(frame.commands[1], RenderCommand::SetColor(_)));
        match &frame.commands[3] {
            RenderCommand::StretchPic { rect, .. } => assert_eq!(rect.width, 40.0),
            command => panic!("expected a clipped picture, got {command:?}"),
        }
        assert!(matches!(frame.commands[4], RenderCommand::SetColor(_)));
    }

    #[test]
    fn text_checks_seat_and_delegates() {
        let registry = owner();
        let compiled = compile_shader_script(
            "text\n{\n {\n map white\n }\n}\n",
            &mut ShaderHost,
            "<test>",
            &CompileOptions::default(),
        )
        .expect("compile")
        .remove(0);
        let draw = MaterialTextDraw {
            rect: RenderRect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
            },
            uv: qa_client::text::draw2d::TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            color: vec4(1.0, 1.0, 1.0, 1.0),
            compiled: &compiled,
        };
        let seat = registry.seat(0);
        let viewport = RenderRect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        let mut renderer = StubRenderer { texts: 0, scenes: 0 };
        let mut frames = frame_builder(&registry);
        draw_q3_overlay(
            &mut renderer,
            &[Q3OverlaySubmission::Text {
                seat: seat.clone(),
                draw,
            }],
            &mut frames,
            &camera(),
            &viewport,
            &seat,
            99,
        )
        .expect("overlay");
        assert_eq!(renderer.texts, 1);
        let other = registry.seat(1);
        let draw = MaterialTextDraw {
            rect: RenderRect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
            },
            uv: qa_client::text::draw2d::TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            color: vec4(1.0, 1.0, 1.0, 1.0),
            compiled: &compiled,
        };
        let mut frames = frame_builder(&registry);
        assert_eq!(
            draw_q3_overlay(
                &mut renderer,
                &[Q3OverlaySubmission::Text { seat: other, draw }],
                &mut frames,
                &camera(),
                &viewport,
                &seat,
                99,
            )
            .unwrap_err(),
            Q3OverlayError::WrongSeat
        );
    }

    #[test]
    fn scene_rejects_world_views_and_foreign_seats() {
        let registry = owner();
        let seat = registry.seat(0);
        let viewport = RenderRect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        let mut renderer = StubRenderer { texts: 0, scenes: 0 };
        let mut frames = frame_builder(&registry);
        assert_eq!(
            draw_q3_overlay(
                &mut renderer,
                &[Q3OverlaySubmission::Scene(Box::new(presented(
                    &registry,
                    0,
                    seat.clone()
                )))],
                &mut frames,
                &camera(),
                &viewport,
                &seat,
                99,
            )
            .unwrap_err(),
            Q3OverlayError::WorldViewRejected
        );
        let mut frames = frame_builder(&registry);
        assert_eq!(
            draw_q3_overlay(
                &mut renderer,
                &[Q3OverlaySubmission::Scene(Box::new(presented(
                    &registry,
                    RDF_NOWORLDMODEL,
                    registry.seat(1),
                )))],
                &mut frames,
                &camera(),
                &viewport,
                &seat,
                99,
            )
            .unwrap_err(),
            Q3OverlayError::WorldViewRejected
        );
        let mut frames = frame_builder(&registry);
        draw_q3_overlay(
            &mut renderer,
            &[Q3OverlaySubmission::Scene(Box::new(presented(
                &registry,
                RDF_NOWORLDMODEL,
                seat.clone(),
            )))],
            &mut frames,
            &camera(),
            &viewport,
            &seat,
            99,
        )
        .expect("overlay");
        assert_eq!(renderer.scenes, 1);
    }
}
