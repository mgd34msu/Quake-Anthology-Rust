//! Q3 fonts: DAT records, legacy atlases, string drawing.
//!
//! Donor provenance: `src/text/q3-font.ts` (from `quak3e` font
//! rendering: `fontInfo_t` DAT records, charset/proportional/banner
//! atlases, `CG_DrawString`, UI string styles).

use qa_core::binary::{BinaryError, BinaryReader};
use qa_core::math::Vec4;

use super::draw2d::{Draw2D, PictureAsset, Rect, TextureRect, WHITE};
use crate::ClientError;

/// `fontInfo_t` record size.
pub const FONT_RECORD_SIZE: usize = 20548;
/// Glyph count per font.
pub const FONT_GLYPH_COUNT: usize = 256;

/// Glyph metrics (`GlyphMetrics`).
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphMetrics {
    /// Height.
    pub height: i32,
    /// Top.
    pub top: i32,
    /// Bottom.
    pub bottom: i32,
    /// Pitch.
    pub pitch: i32,
    /// Horizontal advance.
    pub x_skip: i32,
    /// Image width.
    pub image_width: i32,
    /// Image height.
    pub image_height: i32,
    /// Left.
    pub s: f32,
    /// Top.
    pub t: f32,
    /// Right.
    pub s2: f32,
    /// Bottom.
    pub t2: f32,
    /// Shader name.
    pub shader_name: String,
}

/// Font data (`FontData`).
#[derive(Debug, Clone, PartialEq)]
pub struct FontData {
    /// Name.
    pub name: String,
    /// Glyph scale.
    pub glyph_scale: f32,
    /// Glyphs.
    pub glyphs: Vec<GlyphMetrics>,
}

/// A registered glyph (`RegisteredGlyph`).
#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredGlyph {
    /// Metrics.
    pub metrics: GlyphMetrics,
    /// Picture.
    pub picture: Option<PictureAsset>,
}

/// A registered font (`RegisteredFont`).
#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredFont {
    /// Name.
    pub name: String,
    /// Glyph scale.
    pub glyph_scale: f32,
    /// Glyphs.
    pub glyphs: Vec<RegisteredGlyph>,
}

/// Legacy fonts (`LegacyFonts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyFonts {
    /// Charset.
    pub charset: PictureAsset,
    /// Proportional.
    pub proportional: PictureAsset,
    /// Glow.
    pub glow: PictureAsset,
    /// Banner.
    pub banner: PictureAsset,
}

/// Font set profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontProfile {
    /// UI.
    Ui,
    /// Cgame.
    Cgame,
}

/// A font set (`FontSet`).
#[derive(Debug, Clone, PartialEq)]
pub struct FontSet {
    /// Small font.
    pub small: RegisteredFont,
    /// Normal font.
    pub normal: RegisteredFont,
    /// Big font.
    pub big: RegisteredFont,
    /// Profile.
    pub profile: FontProfile,
    /// Small threshold.
    pub small_threshold: f32,
    /// Big threshold.
    pub big_threshold: f32,
}

/// UI style flags.
pub const UI_LEFT: u32 = 0;
/// Center.
pub const UI_CENTER: u32 = 1;
/// Right.
pub const UI_RIGHT: u32 = 2;
/// Small font.
pub const UI_SMALLFONT: u32 = 0x10;
/// Giant font.
pub const UI_GIANTFONT: u32 = 0x40;
/// Drop shadow.
pub const UI_DROPSHADOW: u32 = 0x800;
/// Blink.
pub const UI_BLINK: u32 = 0x1000;
/// Inverse.
pub const UI_INVERSE: u32 = 0x2000;
/// Pulse.
pub const UI_PULSE: u32 = 0x4000;

const COLORS: [Vec4; 8] = [
    Vec4 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 1.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 1.0,
        z: 1.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 0.0,
        z: 1.0,
        w: 1.0,
    },
    WHITE,
];

fn byte_text(text: &str) -> Result<&str, ClientError> {
    let end = text.find('\0').unwrap_or(text.len());
    let value = &text[..end];
    if value.chars().any(|ch| ch as u32 > 255) {
        return Err(ClientError::BadText(
            "Quake text requires an 8-bit source string".to_string(),
        ));
    }
    Ok(value)
}

fn escape_at(text: &[u8], index: usize) -> bool {
    text.get(index) == Some(&b'^') && index + 1 < text.len() && text[index + 1] != b'^'
}

fn escape_color(code: u8, alpha: f32) -> Vec4 {
    let color = COLORS[(code.wrapping_sub(48) & 7) as usize];
    Vec4 { w: alpha, ..color }
}

fn black(alpha: f32) -> Vec4 {
    Vec4 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: alpha,
    }
}

/// Read font data (`readFontData`).
pub fn read_font_data(bytes: &[u8], source: &str) -> Result<FontData, ClientError> {
    if bytes.len() != FONT_RECORD_SIZE {
        return Err(ClientError::BadFont(format!(
            "{source}:0: fontInfo_t must contain exactly 20548 bytes"
        )));
    }
    let map_err = |error: BinaryError| ClientError::BadFont(format!("{error}"));
    let mut reader = BinaryReader::new(bytes, source);
    let mut glyphs = Vec::with_capacity(FONT_GLYPH_COUNT);
    for _ in 0..FONT_GLYPH_COUNT {
        let height = reader.i32().map_err(map_err)?;
        let top = reader.i32().map_err(map_err)?;
        let bottom = reader.i32().map_err(map_err)?;
        let pitch = reader.i32().map_err(map_err)?;
        let x_skip = reader.i32().map_err(map_err)?;
        let image_width = reader.i32().map_err(map_err)?;
        let image_height = reader.i32().map_err(map_err)?;
        let s = reader.f32().map_err(map_err)?;
        let t = reader.f32().map_err(map_err)?;
        let s2 = reader.f32().map_err(map_err)?;
        let t2 = reader.f32().map_err(map_err)?;
        reader.i32().map_err(map_err)?;
        let name_bytes = reader.bytes(32).map_err(map_err)?;
        let end = name_bytes.iter().position(|byte| *byte == 0).unwrap_or(32);
        glyphs.push(GlyphMetrics {
            height,
            top,
            bottom,
            pitch,
            x_skip,
            image_width,
            image_height,
            s,
            t,
            s2,
            t2,
            shader_name: String::from_utf8_lossy(&name_bytes[..end]).into_owned(),
        });
    }
    let glyph_scale = reader.f32().map_err(map_err)?;
    let name_bytes = reader.bytes(64).map_err(map_err)?;
    let end = name_bytes.iter().position(|byte| *byte == 0).unwrap_or(64);
    Ok(FontData {
        name: String::from_utf8_lossy(&name_bytes[..end]).into_owned(),
        glyph_scale,
        glyphs,
    })
}

/// Parse and validate font data (`parseFontData`).
pub fn parse_font_data(bytes: &[u8], source: &str) -> Result<FontData, ClientError> {
    let data = read_font_data(bytes, source)?;
    for (index, glyph) in data.glyphs.iter().enumerate() {
        for (component, value) in [glyph.s, glyph.t, glyph.s2, glyph.t2].iter().enumerate() {
            if !value.is_finite() {
                return Err(ClientError::BadFont(format!(
                    "{source}:{}: non-finite float",
                    index * 80 + 28 + component * 4
                )));
            }
        }
        if glyph.height < 0 || glyph.pitch < 0 || glyph.image_width < 0 || glyph.image_height < 0 {
            return Err(ClientError::BadFont(format!(
                "{source}:{}: Negative glyph bitmap dimension",
                index * 80
            )));
        }
    }
    if !data.glyph_scale.is_finite() {
        return Err(ClientError::BadFont(format!("{source}:20480: non-finite float")));
    }
    if data.glyph_scale <= 0.0 {
        return Err(ClientError::BadFont(format!(
            "{source}:20480: Font scale must be positive"
        )));
    }
    Ok(data)
}

/// Draw a charset character (`drawChar`).
pub fn draw_char(draw: &mut Draw2D, charset: PictureAsset, x: f32, y: f32, width: f32, height: f32, code: u8) {
    if code == 32 {
        return;
    }
    let s = f32::from(code & 15) / 16.0;
    let t = f32::from(code >> 4) / 16.0;
    draw.stretch_pic(
        Rect { x, y, width, height },
        TextureRect {
            s,
            t,
            s2: s + 1.0 / 16.0,
            t2: t + 1.0 / 16.0,
        },
        charset,
    );
}

/// Fixed text options (`FixedTextOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct FixedTextOptions<'a> {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Text.
    pub text: &'a str,
    /// Color.
    pub color: Vec4,
    /// Character width.
    pub char_width: f32,
    /// Character height.
    pub char_height: f32,
    /// Maximum characters.
    pub max_chars: i32,
    /// Force color.
    pub force_color: bool,
    /// Shadow.
    pub shadow: bool,
}

/// Draw a cgame string (`drawCgString`).
pub fn draw_cg_string(draw: &mut Draw2D, charset: PictureAsset, options: &FixedTextOptions) -> Result<(), ClientError> {
    let text = byte_text(options.text)?;
    let bytes = text.as_bytes();
    let maximum = if options.max_chars <= 0 {
        32767
    } else {
        options.max_chars as usize
    };
    for shadow in if options.shadow { vec![true, false] } else { vec![false] } {
        let mut x = options.x.trunc();
        let mut count = 0usize;
        draw.set_color(Some(if shadow { black(options.color.w) } else { options.color }));
        let mut index = 0usize;
        while index < bytes.len() && count < maximum {
            if escape_at(bytes, index) {
                if !shadow && !options.force_color {
                    draw.set_color(Some(escape_color(bytes[index + 1], options.color.w)));
                }
                index += 2;
                continue;
            }
            draw_char(
                draw,
                charset,
                x + if shadow { 2.0 } else { 0.0 },
                options.y.trunc() + if shadow { 2.0 } else { 0.0 },
                options.char_width.trunc(),
                options.char_height.trunc(),
                bytes[index],
            );
            x += options.char_width.trunc();
            count += 1;
            index += 1;
        }
    }
    draw.set_color(None);
    Ok(())
}

/// UI text options (`UiTextOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct UiTextOptions<'a> {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Text.
    pub text: &'a str,
    /// Color.
    pub color: Vec4,
    /// Style.
    pub style: u32,
    /// Time.
    pub time: i32,
}

fn aligned_x(x: f32, width: f32, style: u32) -> f32 {
    (x.trunc() as i32
        - if style & 7 == UI_CENTER {
            (width / 2.0).trunc() as i32
        } else if style & 7 == UI_RIGHT {
            width as i32
        } else {
            0
        }) as f32
}

fn pulse(time: i32) -> f32 {
    0.5 + 0.5 * ((time / 75) as f32).sin()
}

/// Draw a UI string (`drawUiString`).
pub fn draw_ui_string(draw: &mut Draw2D, charset: PictureAsset, options: &UiTextOptions) -> Result<(), ClientError> {
    if options.style & UI_BLINK != 0 && (options.time / 200) & 1 != 0 {
        return Ok(());
    }
    let text = byte_text(options.text)?;
    let bytes = text.as_bytes();
    let width = if options.style & UI_SMALLFONT != 0 {
        8.0
    } else if options.style & UI_GIANTFONT != 0 {
        32.0
    } else {
        16.0
    };
    let height = if options.style & UI_GIANTFONT != 0 && options.style & UI_SMALLFONT == 0 {
        48.0
    } else {
        16.0
    };
    let mut color = options.color;
    if options.style & UI_PULSE != 0 {
        let t = pulse(options.time);
        let channel = |value: f32| (value + t * (0.8 * value - value)).clamp(0.0, 1.0);
        color = Vec4 {
            x: channel(color.x),
            y: channel(color.y),
            z: channel(color.z),
            w: channel(color.w),
        };
    }
    let start = aligned_x(options.x, bytes.len() as f32 * width, options.style);
    for (offset, pass_color) in if options.style & UI_DROPSHADOW != 0 {
        vec![(2.0, black(color.w)), (0.0, color)]
    } else {
        vec![(0.0, color)]
    } {
        if options.y.trunc() + offset < -height {
            continue;
        }
        draw.set_color(Some(pass_color));
        let rect = draw.adjust(&Rect {
            x: start + offset,
            y: options.y.trunc() + offset,
            width,
            height,
        });
        let mut x = rect.x;
        for (index, byte) in bytes.iter().enumerate() {
            if escape_at(bytes, index) {
                continue;
            }
            if index > 0 && escape_at(bytes, index - 1) {
                draw.set_color(Some(escape_color(*byte, pass_color.w)));
                continue;
            }
            let ch = *byte as i8 as i32;
            let s = f32::from((ch & 15) as u8) / 16.0;
            let t = f32::from(((ch >> 4) & 15) as u8) / 16.0;
            if ch != 32 {
                draw.stretch_pixels(
                    Rect { x, ..rect },
                    TextureRect {
                        s,
                        t,
                        s2: s + 1.0 / 16.0,
                        t2: t + 1.0 / 16.0,
                    },
                    charset,
                );
            }
            x += rect.width;
        }
        draw.set_color(None);
    }
    Ok(())
}

/// A proportional atlas metric (x, y, width).
pub type AtlasMetric = (i32, i32, i32);

/// Invalid metric.
pub const INVALID_METRIC: AtlasMetric = (0, 0, -1);

const PROP_ASCII: [AtlasMetric; 65] = [
    (0, 0, 8),
    (11, 122, 7),
    (154, 181, 14),
    (55, 122, 17),
    (79, 122, 18),
    (101, 122, 23),
    (153, 122, 18),
    (9, 93, 7),
    (207, 122, 8),
    (230, 122, 9),
    (177, 122, 18),
    (30, 152, 18),
    (85, 181, 7),
    (34, 93, 11),
    (110, 181, 6),
    (130, 152, 14),
    (22, 64, 17),
    (41, 64, 12),
    (58, 64, 17),
    (78, 64, 18),
    (98, 64, 19),
    (120, 64, 18),
    (141, 64, 18),
    (204, 64, 16),
    (162, 64, 17),
    (182, 64, 18),
    (59, 181, 7),
    (35, 181, 7),
    (203, 152, 14),
    (56, 93, 14),
    (228, 152, 14),
    (177, 181, 18),
    (28, 122, 22),
    (5, 4, 18),
    (27, 4, 18),
    (48, 4, 18),
    (69, 4, 17),
    (90, 4, 13),
    (106, 4, 13),
    (121, 4, 18),
    (143, 4, 17),
    (164, 4, 8),
    (175, 4, 16),
    (195, 4, 18),
    (216, 4, 12),
    (230, 4, 23),
    (6, 34, 18),
    (27, 34, 18),
    (48, 34, 18),
    (68, 34, 18),
    (90, 34, 17),
    (110, 34, 18),
    (130, 34, 14),
    (146, 34, 18),
    (166, 34, 19),
    (185, 34, 29),
    (215, 34, 18),
    (234, 34, 18),
    (5, 64, 14),
    (60, 152, 7),
    (106, 151, 13),
    (83, 152, 7),
    (128, 122, 17),
    (4, 152, 21),
    (134, 181, 5),
];

const PROP_END: [AtlasMetric; 4] = [(153, 152, 13), (11, 181, 5), (180, 152, 13), (79, 93, 17)];

const BANNER: [AtlasMetric; 26] = [
    (11, 12, 33),
    (49, 12, 31),
    (85, 12, 31),
    (120, 12, 30),
    (156, 12, 21),
    (183, 12, 21),
    (207, 12, 32),
    (13, 55, 30),
    (49, 55, 13),
    (66, 55, 29),
    (101, 55, 31),
    (135, 55, 21),
    (158, 55, 40),
    (204, 55, 32),
    (12, 97, 31),
    (48, 97, 31),
    (82, 97, 30),
    (118, 97, 30),
    (153, 97, 30),
    (185, 97, 25),
    (213, 97, 30),
    (11, 139, 32),
    (42, 139, 51),
    (93, 139, 32),
    (126, 139, 31),
    (158, 139, 25),
];

/// Proportional metric (`propMetric`).
pub fn prop_metric(code: u8) -> Result<AtlasMetric, ClientError> {
    let mut ch = u32::from(code) & 127;
    if ch < 32 || ch == 127 {
        return Ok(INVALID_METRIC);
    }
    if (97..=122).contains(&ch) {
        ch -= 32;
    }
    if ch >= 123 {
        PROP_END
            .get((ch - 123) as usize)
            .copied()
            .ok_or_else(|| ClientError::BadText("Invalid proportional glyph index".to_string()))
    } else {
        PROP_ASCII
            .get((ch - 32) as usize)
            .copied()
            .ok_or_else(|| ClientError::BadText("Invalid proportional glyph index".to_string()))
    }
}

fn banner_metric(code: u8) -> Result<AtlasMetric, ClientError> {
    BANNER
        .get(code.wrapping_sub(65) as usize)
        .copied()
        .filter(|_| (65..=90).contains(&code))
        .ok_or_else(|| ClientError::BadText("Invalid banner glyph index".to_string()))
}

/// Proportional string width (`proportionalStringWidth`).
pub fn proportional_string_width(input: &str) -> Result<i32, ClientError> {
    let text = byte_text(input)?;
    let mut width = 0i32;
    for byte in text.bytes() {
        if prop_metric(byte)?.2 != -1 {
            width += prop_metric(byte)?.2 + 3;
        }
    }
    Ok(width - 3)
}

/// Banner string width (`bannerStringWidth`).
pub fn banner_string_width(input: &str) -> Result<i32, ClientError> {
    let text = byte_text(input)?;
    let mut width = 0i32;
    for byte in text.bytes() {
        if byte == 32 {
            width += 12;
        } else if (65..=90).contains(&byte) {
            width += banner_metric(byte)?.2 + 4;
        }
    }
    Ok(width - 4)
}

#[allow(clippy::too_many_arguments)]
fn atlas_pass(
    draw: &mut Draw2D,
    picture: PictureAsset,
    text: &str,
    x: f32,
    y: f32,
    color: Vec4,
    size: f32,
    banner: bool,
    profile: FontProfile,
) -> Result<(), ClientError> {
    draw.set_color(Some(color));
    let position = draw.adjust(&Rect {
        x,
        y,
        width: 0.0,
        height: 0.0,
    });
    let vertical_scale = if profile == FontProfile::Cgame {
        draw.scale_x()
    } else {
        draw.scale_y()
    };
    let top = if profile == FontProfile::Cgame {
        y * draw.scale_x()
    } else {
        position.y
    };
    let mut ax = position.x;
    let mut aw = 0.0f32;
    let gap = (if banner { 4.0 } else { 3.0 } * draw.scale_x()) * size;
    let height = (if banner { 36.0 } else { 27.0 } * vertical_scale) * size;
    for byte in text.bytes() {
        let code = byte & 127;
        if banner && code != 32 && !(65..=90).contains(&code) {
            continue;
        }
        let metric = if banner && code != 32 {
            banner_metric(code)?
        } else {
            prop_metric(code)?
        };
        if code == 32 {
            aw = (if banner { 12.0 } else { 8.0 } * draw.scale_x()) * size;
        } else if metric.2 != -1 {
            aw = (metric.2 as f32 * draw.scale_x()) * size;
            draw.stretch_pixels(
                Rect {
                    x: ax,
                    y: top,
                    width: aw,
                    height,
                },
                TextureRect {
                    s: metric.0 as f32 / 256.0,
                    t: metric.1 as f32 / 256.0,
                    s2: (metric.0 + metric.2) as f32 / 256.0,
                    t2: (metric.1 + if banner { 36 } else { 27 }) as f32 / 256.0,
                },
                picture,
            );
        } else if profile == FontProfile::Cgame {
            aw = 0.0;
        }
        ax += aw + gap;
    }
    draw.set_color(None);
    Ok(())
}

fn proportional_text(
    draw: &mut Draw2D,
    fonts: &LegacyFonts,
    options: &UiTextOptions,
    profile: FontProfile,
) -> Result<(), ClientError> {
    let text = byte_text(options.text)?;
    let size = if options.style & UI_SMALLFONT != 0 { 0.75 } else { 1.0 };
    let x = aligned_x(
        options.x,
        (proportional_string_width(text)? as f32 * size).trunc(),
        options.style,
    );
    let y = options.y.trunc();
    if options.style & UI_DROPSHADOW != 0 {
        atlas_pass(
            draw,
            fonts.proportional,
            text,
            x + 2.0,
            y + 2.0,
            black(options.color.w),
            size,
            false,
            profile,
        )?;
    }
    if options.style & UI_INVERSE != 0 {
        let inverse = if profile == FontProfile::Cgame { 0.8 } else { 0.7 };
        atlas_pass(
            draw,
            fonts.proportional,
            text,
            x,
            y,
            Vec4 {
                x: options.color.x * inverse,
                y: options.color.y * inverse,
                z: options.color.z * inverse,
                w: options.color.w,
            },
            size,
            false,
            profile,
        )?;
        return Ok(());
    }
    atlas_pass(
        draw,
        fonts.proportional,
        text,
        x,
        y,
        options.color,
        size,
        false,
        profile,
    )?;
    if options.style & UI_PULSE != 0 {
        atlas_pass(
            draw,
            fonts.glow,
            text,
            x,
            y,
            Vec4 {
                w: pulse(options.time),
                ..options.color
            },
            size,
            false,
            profile,
        )?;
    }
    Ok(())
}

/// Draw a proportional string (`drawProportionalString`).
pub fn draw_proportional_string(
    draw: &mut Draw2D,
    fonts: &LegacyFonts,
    options: &UiTextOptions,
) -> Result<(), ClientError> {
    proportional_text(draw, fonts, options, FontProfile::Ui)
}

/// Draw a cgame proportional string (`drawCgProportionalString`).
pub fn draw_cg_proportional_string(
    draw: &mut Draw2D,
    fonts: &LegacyFonts,
    options: &UiTextOptions,
) -> Result<(), ClientError> {
    proportional_text(draw, fonts, options, FontProfile::Cgame)
}

fn banner_text(
    draw: &mut Draw2D,
    fonts: &LegacyFonts,
    options: &UiTextOptions,
    profile: FontProfile,
) -> Result<(), ClientError> {
    let text = byte_text(options.text)?;
    let x = aligned_x(options.x, banner_string_width(text)? as f32, options.style);
    let y = options.y.trunc();
    if options.style & UI_DROPSHADOW != 0 {
        atlas_pass(
            draw,
            fonts.banner,
            text,
            x + 2.0,
            y + 2.0,
            black(options.color.w),
            1.0,
            true,
            profile,
        )?;
    }
    atlas_pass(draw, fonts.banner, text, x, y, options.color, 1.0, true, profile)
}

/// Draw a banner string (`drawBannerString`).
pub fn draw_banner_string(draw: &mut Draw2D, fonts: &LegacyFonts, options: &UiTextOptions) -> Result<(), ClientError> {
    banner_text(draw, fonts, options, FontProfile::Ui)
}

/// Draw a cgame banner string (`drawCgBannerString`).
pub fn draw_cg_banner_string(
    draw: &mut Draw2D,
    fonts: &LegacyFonts,
    options: &UiTextOptions,
) -> Result<(), ClientError> {
    banner_text(draw, fonts, options, FontProfile::Cgame)
}

fn select_font(fonts: &FontSet, scale: f32) -> Result<&RegisteredFont, ClientError> {
    if !scale.is_finite() {
        return Err(ClientError::BadText("Non-finite text scale".to_string()));
    }
    Ok(if scale <= fonts.small_threshold {
        &fonts.small
    } else if fonts.profile == FontProfile::Ui {
        if scale >= fonts.big_threshold {
            &fonts.big
        } else {
            &fonts.normal
        }
    } else if scale > fonts.big_threshold {
        &fonts.big
    } else {
        &fonts.normal
    })
}

fn glyph_at(font: &RegisteredFont, code: u8) -> Result<&RegisteredGlyph, ClientError> {
    font.glyphs
        .get(usize::from(code))
        .ok_or_else(|| ClientError::BadText(format!("Missing font glyph {}", code)))
}

fn text_metric(fonts: &FontSet, input: &str, scale: f32, limit: i32, height: bool) -> Result<i32, ClientError> {
    let text = byte_text(input)?;
    let bytes = text.as_bytes();
    let font = select_font(fonts, scale)?;
    let use_scale = scale * font.glyph_scale;
    let maximum = if limit > 0 {
        (bytes.len() as i32).min(limit) as usize
    } else {
        bytes.len()
    };
    let mut value = 0i32;
    let mut count = 0usize;
    let mut index = 0usize;
    while index < bytes.len() && count < maximum {
        if escape_at(bytes, index) {
            index += 2;
            continue;
        }
        let glyph = glyph_at(font, bytes[index])?;
        if height {
            value = value.max(glyph.metrics.height);
        } else {
            value += glyph.metrics.x_skip;
        }
        count += 1;
        index += 1;
    }
    Ok((value as f32 * use_scale).trunc() as i32)
}

/// Text width (`textWidth`).
pub fn text_width(fonts: &FontSet, text: &str, scale: f32, limit: i32) -> Result<i32, ClientError> {
    text_metric(fonts, text, scale, limit, false)
}

/// Text height (`textHeight`).
pub fn text_height(fonts: &FontSet, text: &str, scale: f32, limit: i32) -> Result<i32, ClientError> {
    text_metric(fonts, text, scale, limit, true)
}

/// Text paint options (`TextPaintOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextPaintOptions<'a> {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Scale.
    pub scale: f32,
    /// Color.
    pub color: Vec4,
    /// Text.
    pub text: &'a str,
    /// Adjust.
    pub adjust: f32,
    /// Limit.
    pub limit: i32,
    /// Style.
    pub style: i32,
}

/// Text cursor (`TextCursor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextCursor {
    /// Position.
    pub position: usize,
    /// Character.
    pub character: u8,
    /// Time.
    pub time: i32,
}

fn paint_glyph(draw: &mut Draw2D, glyph: &RegisteredGlyph, x: f32, baseline: f32, scale: f32) {
    if let Some(picture) = glyph.picture {
        draw.stretch_pic(
            Rect {
                x,
                y: baseline - scale * glyph.metrics.top as f32,
                width: glyph.metrics.image_width as f32 * scale,
                height: glyph.metrics.image_height as f32 * scale,
            },
            TextureRect {
                s: glyph.metrics.s,
                t: glyph.metrics.t,
                s2: glyph.metrics.s2,
                t2: glyph.metrics.t2,
            },
            picture,
        );
    }
}

fn paint_text(
    draw: &mut Draw2D,
    fonts: &FontSet,
    options: &TextPaintOptions,
    cursor: Option<TextCursor>,
) -> Result<(), ClientError> {
    let text = byte_text(options.text)?;
    let bytes = text.as_bytes();
    let font = select_font(fonts, options.scale)?;
    let scale = options.scale * font.glyph_scale;
    let maximum = if options.limit > 0 {
        (bytes.len() as i32).min(options.limit) as usize
    } else {
        bytes.len()
    };
    let mut x = options.x;
    let mut color = options.color;
    let mut count = 0usize;
    let cursor_glyph = match cursor {
        Some(cursor) => Some(glyph_at(font, cursor.character)?.clone()),
        None => None,
    };
    let cursor_visible = cursor.is_some_and(|cursor| (cursor.time / 200) & 1 == 0);
    draw.set_color(Some(color));
    let mut index = 0usize;
    while index < bytes.len() && count < maximum {
        if escape_at(bytes, index) {
            color = escape_color(bytes[index + 1], options.color.w);
            draw.set_color(Some(color));
            index += 2;
            continue;
        }
        let glyph = glyph_at(font, bytes[index])?.clone();
        let baseline = options.y;
        if options.style == 3 || options.style == 6 {
            let offset = if options.style == 3 { 1.0 } else { 2.0 };
            if let Some(picture) = glyph.picture {
                draw.set_color(Some(black(color.w)));
                draw.stretch_pic(
                    Rect {
                        x: x + offset,
                        y: baseline - scale * glyph.metrics.top as f32 + offset,
                        width: glyph.metrics.image_width as f32 * scale,
                        height: glyph.metrics.image_height as f32 * scale,
                    },
                    TextureRect {
                        s: glyph.metrics.s,
                        t: glyph.metrics.t,
                        s2: glyph.metrics.s2,
                        t2: glyph.metrics.t2,
                    },
                    picture,
                );
                draw.set_color(Some(color));
            }
        }
        paint_glyph(draw, &glyph, x, baseline, scale);
        if cursor_visible {
            if let (Some(cursor), Some(cursor_glyph)) = (cursor, &cursor_glyph) {
                if cursor.position == count {
                    paint_glyph(draw, cursor_glyph, x, baseline, scale);
                }
            }
        }
        x += glyph.metrics.x_skip as f32 * scale + if cursor.is_none() { options.adjust } else { 0.0 };
        count += 1;
        index += 1;
    }
    if cursor_visible {
        if let (Some(cursor), Some(cursor_glyph)) = (cursor, &cursor_glyph) {
            if cursor.position == maximum {
                paint_glyph(draw, cursor_glyph, x, options.y, scale);
            }
        }
    }
    draw.set_color(None);
    Ok(())
}

/// Paint text (`textPaint`).
pub fn text_paint(draw: &mut Draw2D, fonts: &FontSet, options: &TextPaintOptions) -> Result<(), ClientError> {
    paint_text(draw, fonts, options, None)
}

/// Paint text with a cursor (`textPaintWithCursor`).
pub fn text_paint_with_cursor(
    draw: &mut Draw2D,
    fonts: &FontSet,
    options: &TextPaintOptions,
    cursor: TextCursor,
) -> Result<(), ClientError> {
    paint_text(
        draw,
        fonts,
        &TextPaintOptions {
            adjust: 0.0,
            ..options.clone()
        },
        Some(cursor),
    )
}

/// Paint text with a limit (`textPaintLimit`).
pub fn text_paint_limit(
    draw: &mut Draw2D,
    fonts: &FontSet,
    options: &TextPaintOptions,
    max_x: f32,
) -> Result<f32, ClientError> {
    let text = byte_text(options.text)?;
    let bytes = text.as_bytes();
    let cgame = FontSet {
        profile: FontProfile::Cgame,
        small: fonts.small.clone(),
        normal: fonts.normal.clone(),
        big: fonts.big.clone(),
        small_threshold: fonts.small_threshold,
        big_threshold: fonts.big_threshold,
    };
    let font = select_font(&cgame, options.scale)?;
    let scale = options.scale * font.glyph_scale;
    let maximum = if options.limit > 0 {
        (bytes.len() as i32).min(options.limit) as usize
    } else {
        bytes.len()
    };
    let mut x = options.x;
    let mut result = max_x;
    let mut count = 0usize;
    draw.set_color(Some(options.color));
    let mut index = 0usize;
    while index < bytes.len() && count < maximum {
        if escape_at(bytes, index) {
            draw.set_color(Some(escape_color(bytes[index + 1], options.color.w)));
            index += 2;
            continue;
        }
        let rest = &text[index..index + 1];
        if text_width(fonts, rest, options.scale, 1)? as f32 + x > max_x {
            result = 0.0;
            break;
        }
        let glyph = glyph_at(font, bytes[index])?.clone();
        paint_glyph(draw, &glyph, x, options.y, scale);
        x += glyph.metrics.x_skip as f32 * scale + options.adjust;
        result = x;
        count += 1;
        index += 1;
    }
    draw.set_color(None);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::draw2d::{CoordinateSpace, Rect, TextCommandSink};
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

    fn picture() -> PictureAsset {
        PictureAsset::Image(crate::text::draw2d::ImagePicture {
            image: 1,
            width: 256,
            height: 256,
        })
    }

    #[test]
    fn record_size_is_checked() {
        assert!(read_font_data(&[0u8; 100], "<test>").is_err());
        let data = read_font_data(&vec![0u8; FONT_RECORD_SIZE], "<test>").unwrap();
        assert_eq!(data.glyphs.len(), 256);
        assert!(parse_font_data(&vec![0u8; FONT_RECORD_SIZE], "<test>").is_err());
    }

    #[test]
    fn widths_use_metrics() {
        assert_eq!(proportional_string_width("A").unwrap(), 18);
        assert_eq!(banner_string_width("A").unwrap(), 33);
        assert!(byte_text("héllo ☃").is_err());
    }

    #[test]
    fn cg_string_draws_and_resets_color() {
        let mut sink = sink();
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Pixels);
        draw_cg_string(
            &mut draw,
            picture(),
            &FixedTextOptions {
                x: 0.0,
                y: 0.0,
                text: "A^1B",
                color: WHITE,
                char_width: 8.0,
                char_height: 8.0,
                max_chars: 0,
                force_color: false,
                shadow: true,
            },
        )
        .unwrap();
        assert!(!sink.commands.is_empty());
    }

    #[test]
    fn ui_blink_skips() {
        let mut sink = sink();
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Stretch640);
        draw_ui_string(
            &mut draw,
            picture(),
            &UiTextOptions {
                x: 320.0,
                y: 240.0,
                text: "hi",
                color: WHITE,
                style: UI_BLINK,
                time: 200,
            },
        )
        .unwrap();
        assert!(sink.commands.is_empty());
    }

    #[test]
    fn font_set_metrics() {
        let glyph = RegisteredGlyph {
            metrics: GlyphMetrics {
                height: 16,
                top: 12,
                bottom: 4,
                pitch: 8,
                x_skip: 10,
                image_width: 8,
                image_height: 16,
                s: 0.0,
                t: 0.0,
                s2: 1.0,
                t2: 1.0,
                shader_name: String::new(),
            },
            picture: None,
        };
        let font = RegisteredFont {
            name: "test".to_string(),
            glyph_scale: 1.0,
            glyphs: vec![glyph; 256],
        };
        let fonts = FontSet {
            small: font.clone(),
            normal: font.clone(),
            big: font,
            profile: FontProfile::Ui,
            small_threshold: 0.5,
            big_threshold: 2.0,
        };
        assert_eq!(text_width(&fonts, "AB", 1.0, 0).unwrap(), 20);
        assert_eq!(text_height(&fonts, "AB", 1.0, 0).unwrap(), 16);
    }
}
