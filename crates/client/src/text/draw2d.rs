//! 2D drawing coordinates and retained pictures (`cg_drawtools.c`).
//!
//! Donor provenance: `src/text/draw2d.ts` (coordinate transforms from
//! `cg_drawtools.c` and `ui_atoms.c`) and the `clipPicture` helper from
//! `src/render/commands/frame.ts`.
//!
//! Headless: draws record [`DrawCommand`] values through a
//! [`TextDrawSink`] instead of touching a renderer.

use qa_core::identity::SeatId;
use qa_core::math::Vec4;

/// A rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Origin x.
    pub x: f32,
    /// Origin y.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// A 2D origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectOrigin {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
}

/// A texture rectangle (`TextureRect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureRect {
    /// Left.
    pub s: f32,
    /// Top.
    pub t: f32,
    /// Right.
    pub s2: f32,
    /// Bottom.
    pub t2: f32,
}

/// Full texture UVs.
pub const FULL_UV: TextureRect = TextureRect {
    s: 0.0,
    t: 0.0,
    s2: 1.0,
    t2: 1.0,
};

/// White color.
pub const WHITE: Vec4 = Vec4 {
    x: 1.0,
    y: 1.0,
    z: 1.0,
    w: 1.0,
};

/// An image picture (`ImagePicture`, headless handle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImagePicture {
    /// Image handle.
    pub image: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// A material picture (`MaterialPicture`, headless handle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialPicture {
    /// Material order.
    pub order: u32,
}

/// A picture asset (`PictureAsset`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureAsset {
    /// Image.
    Image(ImagePicture),
    /// Material.
    Material(MaterialPicture),
}

/// A recorded draw command.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    /// Set the draw color (white when `None` was passed).
    SetColor(Vec4),
    /// Stretch a picture.
    StretchPic {
        /// Destination.
        rect: Rect,
        /// UVs.
        uv: TextureRect,
        /// Picture.
        picture: PictureAsset,
    },
    /// Stretch a material picture (retained for the material evaluator).
    Material {
        /// Seat.
        seat: SeatId,
        /// Destination.
        rect: Rect,
        /// UVs.
        uv: TextureRect,
        /// Color.
        color: Vec4,
        /// Picture.
        picture: MaterialPicture,
    },
}

/// Clip a picture to a target (`clipPicture`).
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
            s: uv.s + (uv.s2 - uv.s) * (left - rect.x) / rect.width,
            s2: uv.s + (uv.s2 - uv.s) * (right - rect.x) / rect.width,
            t: uv.t + (uv.t2 - uv.t) * (top - rect.y) / rect.height,
            t2: uv.t + (uv.t2 - uv.t) * (bottom - rect.y) / rect.height,
        },
    ))
}

/// A 2D draw sink (`TextDrawSink`).
pub trait TextDrawSink {
    /// Owning seat.
    fn seat(&self) -> &SeatId;
    /// Target rectangle.
    fn target(&self) -> Rect;
    /// Set the draw color (`None` resets to white).
    fn set_color(&mut self, color: Option<Vec4>);
    /// Stretch pixels.
    fn stretch_pixels(&mut self, rect: Rect, uv: TextureRect, picture: PictureAsset);
}

/// A recording command sink (`TextCommandSink`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextCommandSink {
    /// Owning seat.
    pub seat: SeatId,
    /// Target rectangle.
    pub target: Rect,
    /// Recorded commands.
    pub commands: Vec<DrawCommand>,
    color: Vec4,
}

impl TextCommandSink {
    /// New sink.
    #[must_use]
    pub const fn new(seat: SeatId, target: Rect) -> Self {
        Self {
            seat,
            target,
            commands: Vec::new(),
            color: WHITE,
        }
    }
}

impl TextDrawSink for TextCommandSink {
    fn seat(&self) -> &SeatId {
        &self.seat
    }

    fn target(&self) -> Rect {
        self.target
    }

    fn set_color(&mut self, color: Option<Vec4>) {
        self.color = color.unwrap_or(WHITE);
        self.commands.push(DrawCommand::SetColor(self.color));
    }

    fn stretch_pixels(&mut self, rect: Rect, uv: TextureRect, picture: PictureAsset) {
        let destination = Rect {
            x: rect.x + self.target.x,
            y: rect.y + self.target.y,
            width: rect.width,
            height: rect.height,
        };
        match picture {
            PictureAsset::Material(material) => self.commands.push(DrawCommand::Material {
                seat: self.seat.clone(),
                rect: destination,
                uv,
                color: self.color,
                picture: material,
            }),
            PictureAsset::Image(_) => {
                if let Some((rect, uv)) = clip_picture(&destination, &uv, &self.target) {
                    self.commands.push(DrawCommand::StretchPic { rect, uv, picture });
                }
            }
        }
    }
}

/// Coordinate space (`CoordinateSpace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateSpace {
    /// Pixels.
    Pixels,
    /// Stretch 640x480.
    Stretch640,
    /// Base UI 640.
    BaseUi640,
    /// Team UI 640.
    TeamUi640,
}

/// 2D drawing (`Draw2D`).
pub struct Draw2D<'a> {
    /// Command sink.
    pub commands: &'a mut dyn TextDrawSink,
    /// Coordinate space.
    pub space: CoordinateSpace,
}

impl<'a> Draw2D<'a> {
    /// New 2D context.
    #[must_use]
    pub fn new(commands: &'a mut dyn TextDrawSink, space: CoordinateSpace) -> Self {
        Self { commands, space }
    }

    /// Target width.
    #[must_use]
    pub fn width(&self) -> f32 {
        self.commands.target().width
    }

    /// Target height.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.commands.target().height
    }

    /// Horizontal scale.
    #[must_use]
    pub fn scale_x(&self) -> f32 {
        match self.space {
            CoordinateSpace::Pixels => 1.0,
            CoordinateSpace::Stretch640 => self.width() / 640.0,
            CoordinateSpace::BaseUi640 => self.height() * (1.0 / 480.0),
            CoordinateSpace::TeamUi640 => self.width() * (1.0 / 640.0),
        }
    }

    /// Vertical scale.
    #[must_use]
    pub fn scale_y(&self) -> f32 {
        match self.space {
            CoordinateSpace::Pixels => 1.0,
            CoordinateSpace::Stretch640 => self.height() / 480.0,
            CoordinateSpace::BaseUi640 => self.scale_x(),
            CoordinateSpace::TeamUi640 => self.height() * (1.0 / 480.0),
        }
    }

    /// Horizontal bias (base UI letterboxing).
    #[must_use]
    pub fn bias_x(&self) -> f32 {
        if self.space == CoordinateSpace::BaseUi640 && self.width() as i64 * 480 > self.height() as i64 * 640 {
            0.5 * (self.width() - self.height() * (640.0 / 480.0))
        } else {
            0.0
        }
    }

    /// Set the draw color.
    pub fn set_color(&mut self, color: Option<Vec4>) {
        self.commands.set_color(color);
    }

    /// Adjust a rectangle into pixels (`adjust`).
    #[must_use]
    pub fn adjust(&self, rect: &Rect) -> Rect {
        let x = rect.x * self.scale_x();
        Rect {
            x: if self.space == CoordinateSpace::TeamUi640 {
                x
            } else {
                x + self.bias_x()
            },
            y: rect.y * self.scale_y(),
            width: rect.width * self.scale_x(),
            height: rect.height * self.scale_y(),
        }
    }

    /// Stretch a picture.
    pub fn stretch_pic(&mut self, rect: Rect, uv: TextureRect, picture: PictureAsset) {
        let adjusted = self.adjust(&rect);
        self.commands.stretch_pixels(adjusted, uv, picture);
    }

    /// Stretch pixels.
    pub fn stretch_pixels(&mut self, rect: Rect, uv: TextureRect, picture: PictureAsset) {
        self.commands.stretch_pixels(rect, uv, picture);
    }

    /// Draw a picture.
    pub fn draw_pic(&mut self, rect: Rect, picture: PictureAsset) {
        self.stretch_pic(rect, FULL_UV, picture);
    }

    /// Draw a handle picture (negative sizes flip UVs).
    pub fn draw_handle_pic(&mut self, rect: Rect, picture: PictureAsset) {
        self.stretch_pic(
            Rect {
                width: rect.width.abs(),
                height: rect.height.abs(),
                ..rect
            },
            TextureRect {
                s: if rect.width < 0.0 { 1.0 } else { 0.0 },
                s2: if rect.width < 0.0 { 0.0 } else { 1.0 },
                t: if rect.height < 0.0 { 1.0 } else { 0.0 },
                t2: if rect.height < 0.0 { 0.0 } else { 1.0 },
            },
            picture,
        );
    }

    /// Fill a rectangle.
    pub fn fill_rect(&mut self, rect: Rect, color: Vec4, picture: PictureAsset) {
        self.set_color(Some(color));
        self.stretch_pic(
            rect,
            TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 0.0,
                t2: 0.0,
            },
            picture,
        );
        self.set_color(None);
    }
}

/// A retained font file (`RetainedFontFile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedFontFile {
    /// Bytes.
    pub bytes: Vec<u8>,
    /// Length.
    pub length: usize,
}

/// Font file reads (`FontFileReader`, sync).
pub trait FontFileReader {
    /// Read a file length (-1 when missing).
    fn read_file_length(&mut self, path: &str) -> i64;
    /// Read a retained file.
    fn read_file_retained(&mut self, path: &str) -> Option<RetainedFontFile>;
    /// Free a retained file.
    fn free_file(&mut self, file: &RetainedFontFile);
}

/// Font asset services (`FontAssetServices`, sync).
pub trait FontAssetServices {
    /// Register a picture.
    fn register_picture(&mut self, path: &str, mip: bool) -> PictureAsset;
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    fn sink() -> TextCommandSink {
        let owner = IdentityOwner::create("test").unwrap();
        TextCommandSink::new(
            owner.seat(0),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
        )
    }

    #[test]
    fn clips_to_target() {
        let rect = Rect {
            x: -10.0,
            y: 0.0,
            width: 100.0,
            height: 10.0,
        };
        let clip = Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        let (rect, uv) = clip_picture(&rect, &FULL_UV, &clip).unwrap();
        assert_eq!(rect.x, 0.0);
        assert_eq!(rect.width, 90.0);
        assert!((uv.s - 0.1).abs() < 1e-6);
    }

    #[test]
    fn empty_rect_clips_to_nothing() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 10.0,
        };
        assert!(clip_picture(
            &rect,
            &FULL_UV,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0
            },
        )
        .is_none());
    }

    #[test]
    fn stretch_scales() {
        let mut sink = sink();
        let draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
        let rect = draw.adjust(&Rect {
            x: 320.0,
            y: 240.0,
            width: 64.0,
            height: 48.0,
        });
        assert_eq!(rect.x, 320.0);
        assert_eq!(rect.width, 64.0);
    }

    #[test]
    fn material_pictures_retain() {
        let mut sink = sink();
        let picture = PictureAsset::Material(MaterialPicture { order: 3 });
        sink.stretch_pixels(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 8.0,
            },
            FULL_UV,
            picture,
        );
        assert!(matches!(sink.commands[0], DrawCommand::Material { .. }));
    }
}
