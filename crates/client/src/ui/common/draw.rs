//! UI command rendering through seat-clipped 2D draws.
//!
//! Donor provenance: `src/ui/common/draw.ts` (`renderUiCommands`). Command
//! coordinates are absolute drawable pixels; glyphs and images clip to the
//! seat viewport. Emission drains per command so text, fill, and image draws
//! keep donor order with a final color reset.

use qa_core::identity::SeatId;
use qa_core::math::Vec4;

use crate::error::ClientError;
use crate::text::draw2d::{
    clip_picture, CoordinateSpace, Draw2D, DrawCommand, ImagePicture, MaterialPicture, PictureAsset, Rect,
    TextDrawSink, TextureRect, WHITE,
};
use crate::ui::common::layout::intersect;
use crate::ui::types::{ResourceId, UiDrawCommand, UiDrawContext};

/// One retained emit: color selection or an image stretch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiEmitCommand {
    /// Set the 2D drawing color.
    SetColor(Vec4),
    /// Stretch one image picture.
    StretchPic {
        /// Destination rectangle.
        rect: Rect,
        /// Source coordinates.
        uv: TextureRect,
        /// Picture image.
        image: ImagePicture,
    },
}

/// One material-backed 2D quad.
#[derive(Debug, Clone, PartialEq)]
pub struct UiMaterialDraw {
    /// Owning seat.
    pub seat: SeatId,
    /// Destination rectangle.
    pub rect: Rect,
    /// Source coordinates.
    pub uv: TextureRect,
    /// Draw color.
    pub color: Vec4,
    /// Material picture.
    pub picture: MaterialPicture,
}

/// Renderer services backing [`render_ui_commands`].
pub trait UiRenderServices {
    /// Draw one text command through the 2D context.
    fn draw_text(
        &mut self,
        context: &UiDrawContext,
        command: &UiDrawCommand,
        draw: &mut Draw2D,
    ) -> Result<(), ClientError>;
    /// White pixel picture for fills.
    fn white(&self) -> PictureAsset;
    /// Look up one image picture by resource.
    fn picture(&self, resource: &ResourceId) -> Result<PictureAsset, ClientError>;
    /// Emit one retained command.
    fn emit(&mut self, command: UiEmitCommand);
    /// Draw one material-backed quad.
    fn material(&mut self, draw: UiMaterialDraw);
}

/// Seat-clipped 2D sink recording donor-ordered draws.
struct UiDrawSink {
    seat: SeatId,
    target: Rect,
    clip: Rect,
    color: Vec4,
    commands: Vec<DrawCommand>,
}

impl UiDrawSink {
    /// Sink clipped to one seat viewport.
    fn new(seat: SeatId, target: Rect) -> Self {
        Self {
            seat,
            target,
            clip: target,
            color: WHITE,
            commands: Vec::new(),
        }
    }
}

impl TextDrawSink for UiDrawSink {
    fn seat(&self) -> &SeatId {
        &self.seat
    }

    fn target(&self) -> Rect {
        self.target
    }

    fn set_color(&mut self, color: Option<Vec4>) {
        self.color = color.unwrap_or(WHITE);
    }

    fn stretch_pixels(&mut self, rect: Rect, uv: TextureRect, picture: PictureAsset) {
        let Some((rect, uv)) = clip_picture(&rect, &uv, &self.clip) else {
            return;
        };
        self.commands.push(DrawCommand::SetColor(self.color));
        match picture {
            PictureAsset::Image(_) => self.commands.push(DrawCommand::StretchPic { rect, uv, picture }),
            PictureAsset::Material(material) => self.commands.push(DrawCommand::Material {
                seat: self.seat.clone(),
                rect,
                uv,
                color: self.color,
                picture: material,
            }),
        }
    }
}

/// Render UI commands through seat services with a final color reset.
pub fn render_ui_commands(
    context: &UiDrawContext,
    commands: &[UiDrawCommand],
    services: &mut dyn UiRenderServices,
) -> Result<(), ClientError> {
    let mut sink = UiDrawSink::new(context.binding.seat.clone(), context.binding.viewport);
    for command in commands {
        match command {
            UiDrawCommand::Clip { rect } => {
                sink.clip = rect
                    .map(|rect| intersect(&context.binding.viewport, &rect))
                    .unwrap_or(context.binding.viewport);
            }
            UiDrawCommand::Text { .. } => {
                let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Pixels);
                services.draw_text(context, command, &mut draw)?;
                drain(&mut sink, services);
            }
            UiDrawCommand::Fill { rect, color } => {
                let white = services.white();
                sink.set_color(Some(*color));
                sink.stretch_pixels(
                    *rect,
                    TextureRect {
                        s: 0.0,
                        t: 0.0,
                        s2: 0.0,
                        t2: 0.0,
                    },
                    white,
                );
                drain(&mut sink, services);
            }
            UiDrawCommand::Image {
                rect,
                resource,
                tex_coords,
                color,
            } => {
                let picture = services.picture(resource)?;
                sink.set_color(Some(*color));
                sink.stretch_pixels(
                    *rect,
                    TextureRect {
                        s: tex_coords[0].x,
                        t: tex_coords[0].y,
                        s2: tex_coords[1].x,
                        t2: tex_coords[1].y,
                    },
                    picture,
                );
                drain(&mut sink, services);
            }
        }
    }
    services.emit(UiEmitCommand::SetColor(WHITE));
    Ok(())
}

/// Forward one command's recorded draws to services.
fn drain(sink: &mut UiDrawSink, services: &mut dyn UiRenderServices) {
    let seat = sink.seat.clone();
    let color = sink.color;
    for recorded in sink.commands.drain(..) {
        match recorded {
            DrawCommand::SetColor(next) => services.emit(UiEmitCommand::SetColor(next)),
            DrawCommand::StretchPic { rect, uv, picture } => match picture {
                PictureAsset::Image(image) => services.emit(UiEmitCommand::StretchPic { rect, uv, image }),
                PictureAsset::Material(material) => {
                    services.material(UiMaterialDraw {
                        seat: seat.clone(),
                        rect,
                        uv,
                        color,
                        picture: material,
                    });
                }
            },
            DrawCommand::Material {
                seat,
                rect,
                uv,
                color,
                picture,
            } => {
                services.material(UiMaterialDraw {
                    seat,
                    rect,
                    uv,
                    color,
                    picture,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec2, vec4};

    use super::*;
    use crate::text::draw2d::FULL_UV;
    use crate::ui::types::{
        ContentId, DopplerSelection, EnvironmentSelection, PresentationSelection, ProviderRef, SeatPresentationBinding,
        TextAlign,
    };

    struct FakeServices {
        white: PictureAsset,
        pictures: HashMap<String, PictureAsset>,
        texts: Vec<String>,
        emits: Vec<UiEmitCommand>,
        materials: Vec<UiMaterialDraw>,
        draw_glyph: bool,
    }

    impl FakeServices {
        fn new() -> Self {
            Self {
                white: PictureAsset::Image(ImagePicture {
                    image: 1,
                    width: 1,
                    height: 1,
                }),
                pictures: HashMap::new(),
                texts: Vec::new(),
                emits: Vec::new(),
                materials: Vec::new(),
                draw_glyph: false,
            }
        }
    }

    impl UiRenderServices for FakeServices {
        fn draw_text(
            &mut self,
            _context: &UiDrawContext,
            command: &UiDrawCommand,
            draw: &mut Draw2D,
        ) -> Result<(), ClientError> {
            if let UiDrawCommand::Text {
                text,
                origin,
                scale,
                color,
                ..
            } = command
            {
                self.texts.push(text.clone());
                if self.draw_glyph {
                    draw.set_color(Some(*color));
                    draw.stretch_pixels(
                        Rect {
                            x: origin.x,
                            y: origin.y,
                            width: 8.0 * scale,
                            height: 8.0 * scale,
                        },
                        FULL_UV,
                        self.white,
                    );
                }
            }
            Ok(())
        }

        fn white(&self) -> PictureAsset {
            self.white
        }

        fn picture(&self, resource: &ResourceId) -> Result<PictureAsset, ClientError> {
            self.pictures
                .get(resource.as_str())
                .copied()
                .ok_or_else(|| ClientError::BadUi(format!("missing picture: {resource}")))
        }

        fn emit(&mut self, command: UiEmitCommand) {
            self.emits.push(command);
        }

        fn material(&mut self, draw: UiMaterialDraw) {
            self.materials.push(draw);
        }
    }

    fn context(viewport: Rect) -> UiDrawContext {
        let authority = IdentityOwner::create("ui-draw-test").unwrap();
        UiDrawContext {
            binding: SeatPresentationBinding {
                seat: authority.seat(0),
                client: authority.client(0, 0),
                viewport,
                safe_area: viewport,
                hud_scale: 1.0,
                presentation: PresentationSelection {
                    doppler: DopplerSelection::Disabled,
                    environment: EnvironmentSelection::Disabled,
                    assets: ContentId::new("assets"),
                    hud: ProviderRef {
                        provider: "hud".to_string(),
                        content: ContentId::new("hud"),
                    },
                    effects: ProviderRef {
                        provider: "fx".to_string(),
                        content: ContentId::new("fx"),
                    },
                    audio: ProviderRef {
                        provider: "audio".to_string(),
                        content: ContentId::new("audio"),
                    },
                },
            },
            time_ms: 0,
        }
    }

    fn viewport() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        }
    }

    #[test]
    fn fill_dispatches_and_resets_color() {
        let context = context(viewport());
        let red = vec4(1.0, 0.0, 0.0, 1.0);
        let commands = [UiDrawCommand::Fill {
            rect: Rect {
                x: 10.0,
                y: 20.0,
                width: 30.0,
                height: 40.0,
            },
            color: red,
        }];
        let mut services = FakeServices::new();
        render_ui_commands(&context, &commands, &mut services).unwrap();
        assert_eq!(
            services.emits,
            [
                UiEmitCommand::SetColor(red),
                UiEmitCommand::StretchPic {
                    rect: Rect {
                        x: 10.0,
                        y: 20.0,
                        width: 30.0,
                        height: 40.0
                    },
                    uv: TextureRect {
                        s: 0.0,
                        t: 0.0,
                        s2: 0.0,
                        t2: 0.0
                    },
                    image: ImagePicture {
                        image: 1,
                        width: 1,
                        height: 1
                    },
                },
                UiEmitCommand::SetColor(WHITE),
            ]
        );
    }

    #[test]
    fn clip_intersects_viewport() {
        let context = context(viewport());
        let white = vec4(1.0, 1.0, 1.0, 1.0);
        let commands = [
            UiDrawCommand::Clip {
                rect: Some(Rect {
                    x: 600.0,
                    y: 0.0,
                    width: 100.0,
                    height: 480.0,
                }),
            },
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 590.0,
                    y: 0.0,
                    width: 60.0,
                    height: 10.0,
                },
                color: white,
            },
            UiDrawCommand::Clip { rect: None },
            UiDrawCommand::Fill {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                color: white,
            },
        ];
        let mut services = FakeServices::new();
        render_ui_commands(&context, &commands, &mut services).unwrap();
        match services.emits[1] {
            UiEmitCommand::StretchPic { rect, .. } => {
                assert_eq!(
                    rect,
                    Rect {
                        x: 600.0,
                        y: 0.0,
                        width: 40.0,
                        height: 10.0
                    }
                );
            }
            ref other => panic!("expected stretch, got {other:?}"),
        }
        match services.emits[3] {
            UiEmitCommand::StretchPic { rect, .. } => {
                assert_eq!(
                    rect,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 10.0,
                        height: 10.0
                    }
                );
            }
            ref other => panic!("expected stretch, got {other:?}"),
        }
    }

    #[test]
    fn clipped_away_fill_emits_nothing_until_reset() {
        let context = context(viewport());
        let commands = [UiDrawCommand::Fill {
            rect: Rect {
                x: 700.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            color: vec4(1.0, 1.0, 1.0, 1.0),
        }];
        let mut services = FakeServices::new();
        render_ui_commands(&context, &commands, &mut services).unwrap();
        assert_eq!(services.emits, [UiEmitCommand::SetColor(WHITE)]);
    }

    #[test]
    fn image_uses_texcoords_and_picture_lookup() {
        let context = context(viewport());
        let resource = ResourceId::new("resource:test:panel").unwrap();
        let mut services = FakeServices::new();
        services.pictures.insert(
            resource.as_str().to_string(),
            PictureAsset::Image(ImagePicture {
                image: 9,
                width: 64,
                height: 64,
            }),
        );
        let commands = [UiDrawCommand::Image {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
            },
            resource: resource.clone(),
            tex_coords: [vec2(0.0, 0.25), vec2(0.5, 0.75)],
            color: WHITE,
        }];
        render_ui_commands(&context, &commands, &mut services).unwrap();
        match services.emits[1] {
            UiEmitCommand::StretchPic { uv, image, .. } => {
                assert_eq!(
                    uv,
                    TextureRect {
                        s: 0.0,
                        t: 0.25,
                        s2: 0.5,
                        t2: 0.75
                    }
                );
                assert_eq!(image.image, 9);
            }
            ref other => panic!("expected stretch, got {other:?}"),
        }
        let missing = [UiDrawCommand::Image {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 8.0,
            },
            resource: ResourceId::new("resource:test:missing").unwrap(),
            tex_coords: [vec2(0.0, 0.0), vec2(1.0, 1.0)],
            color: WHITE,
        }];
        assert!(render_ui_commands(&context, &missing, &mut services).is_err());
    }

    #[test]
    fn text_forwards_and_glyphs_clip() {
        let context = context(viewport());
        let commands = [
            UiDrawCommand::Clip {
                rect: Some(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 4.0,
                    height: 480.0,
                }),
            },
            UiDrawCommand::Text {
                origin: vec2(0.0, 0.0),
                text: "hi".to_string(),
                font: ResourceId::new("resource:test:font").unwrap(),
                scale: 2.0,
                color: WHITE,
                align: TextAlign::Left,
                shadow: false,
            },
        ];
        let mut services = FakeServices::new();
        services.draw_glyph = true;
        render_ui_commands(&context, &commands, &mut services).unwrap();
        assert_eq!(services.texts, ["hi"]);
        match services.emits[1] {
            UiEmitCommand::StretchPic { rect, uv, .. } => {
                assert_eq!(
                    rect,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 4.0,
                        height: 16.0
                    }
                );
                assert_eq!(uv.s2, 0.25);
            }
            ref other => panic!("expected stretch, got {other:?}"),
        }
    }

    #[test]
    fn material_pictures_route_to_material() {
        let context = context(viewport());
        let resource = ResourceId::new("resource:test:shader").unwrap();
        let mut services = FakeServices::new();
        services.pictures.insert(
            resource.as_str().to_string(),
            PictureAsset::Material(MaterialPicture { order: 3 }),
        );
        let commands = [UiDrawCommand::Image {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 16.0,
            },
            resource,
            tex_coords: [vec2(0.0, 0.0), vec2(1.0, 1.0)],
            color: WHITE,
        }];
        render_ui_commands(&context, &commands, &mut services).unwrap();
        assert_eq!(services.materials.len(), 1);
        assert_eq!(
            services.materials[0].rect,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 16.0
            }
        );
        assert_eq!(services.emits.len(), 2);
    }
}
