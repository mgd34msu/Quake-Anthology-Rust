//! Text layout, measurement, and seat scaling.
//!
//! Donor provenance: `src/text/layout.ts` (one layout calculation
//! supplies both measurement and draw positions).

use qa_core::identity::SeatId;
use qa_core::math::{Vec2, Vec4};

use super::atlas::{glyph_uv, resolve_text_glyph, AtlasGlyph, TextAtlas, TextFontSelection};
use super::draw2d::{Draw2D, Rect, TextureRect};
use crate::ClientError;

const PALETTE: [Vec4; 8] = [
    Vec4 { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },
    Vec4 { x: 1.0, y: 0.0, z: 0.0, w: 1.0 },
    Vec4 { x: 0.0, y: 1.0, z: 0.0, w: 1.0 },
    Vec4 { x: 1.0, y: 1.0, z: 0.0, w: 1.0 },
    Vec4 { x: 0.0, y: 0.0, z: 1.0, w: 1.0 },
    Vec4 { x: 0.0, y: 1.0, z: 1.0, w: 1.0 },
    Vec4 { x: 1.0, y: 0.0, z: 1.0, w: 1.0 },
    Vec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 },
];

/// Color-code mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCodes {
    /// Literal carets.
    Literal,
    /// Q3 `^n` escapes.
    Q3,
}

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    /// Left.
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
}

/// Layout options (`TextLayoutOptions`).
#[derive(Debug, Clone)]
pub struct TextLayoutOptions<'a> {
    /// Text.
    pub text: &'a str,
    /// Font.
    pub font: &'a TextFontSelection,
    /// Scale.
    pub scale: f32,
    /// Base color.
    pub color: Vec4,
    /// Color codes.
    pub color_codes: ColorCodes,
    /// Ignore `^n` changes.
    pub force_color: bool,
    /// Alternate (gold) tint.
    pub alternate: bool,
    /// Maximum width.
    pub max_width: Option<f32>,
    /// Alignment.
    pub align: TextAlign,
    /// Line height override.
    pub line_height: Option<f32>,
    /// Maximum glyphs.
    pub max_glyphs: Option<usize>,
    /// Tab columns.
    pub tab_columns: usize,
}

/// A positioned glyph (`PositionedGlyph`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph<'a> {
    /// Source byte offset.
    pub source_offset: usize,
    /// Atlas.
    pub atlas: &'a TextAtlas,
    /// Glyph.
    pub glyph: AtlasGlyph,
    /// Destination rect.
    pub rect: Rect,
    /// UVs.
    pub uv: TextureRect,
    /// Color.
    pub color: Vec4,
    /// Visible (spaces measure but do not draw).
    pub visible: bool,
}

/// A laid-out line (`TextLine`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine<'a> {
    /// Width.
    pub width: f32,
    /// Y offset.
    pub y: f32,
    /// Glyphs.
    pub glyphs: Vec<PositionedGlyph<'a>>,
}

/// A laid-out block (`TextLayout`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout<'a> {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
    /// Line height.
    pub line_height: f32,
    /// Lines.
    pub lines: Vec<TextLine<'a>>,
    /// Glyph count.
    pub glyph_count: usize,
}

struct Cell<'a> {
    atlas: &'a TextAtlas,
    glyph: AtlasGlyph,
    color: Vec4,
    source_offset: usize,
    character: char,
    advance: f32,
    scale: f32,
    visible: bool,
}

/// Lay out text (`layoutText`).
pub fn layout_text(options: &TextLayoutOptions) -> Result<TextLayout<'_>, ClientError> {
    if !options.scale.is_finite() || options.scale <= 0.0 {
        return Err(ClientError::BadText("Text scale must be positive".to_string()));
    }
    let line_height = options.line_height.unwrap_or(8.0 * options.scale);
    let maximum = options.max_width.unwrap_or(f32::INFINITY);
    if !line_height.is_finite() || line_height <= 0.0 || maximum <= 0.0 || maximum.is_nan() {
        return Err(ClientError::BadText("Invalid text layout dimensions".to_string()));
    }
    if options.tab_columns == 0 {
        return Err(ClientError::BadText("Invalid tab width".to_string()));
    }
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    let mut width = 0.0f32;
    let mut color = options.color;
    let mut count = 0usize;
    let mut skip_through = 0usize;

    let mut flush = |current: &mut Vec<Cell>, width: &mut f32, rows: &mut Vec<Vec<Cell>>| {
        rows.push(core::mem::take(current));
        *width = 0.0;
    };

    let chars: Vec<(usize, char)> = options.text.char_indices().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let (source_offset, character) = chars[index];
        if source_offset < skip_through {
            index += 1;
            continue;
        }
        if character == '\0' {
            break;
        }
        let next = chars.get(index + 1).map(|(_, ch)| *ch);
        if options.color_codes == ColorCodes::Q3
            && character == '^'
            && next.is_some_and(|next| next != '^')
        {
            // The following unit is consumed even for non-digits.
            if let Some(next) = next {
                let palette = PALETTE[(next as u32).wrapping_sub(48) as usize & 7];
                if !options.force_color {
                    color = Vec4 {
                        w: options.color.w,
                        ..palette
                    };
                }
                skip_through = source_offset + '^'.len_utf8() + next.len_utf8();
            }
            index += 1;
            continue;
        }
        if character == '\r' {
            index += 1;
            continue;
        }
        if character == '\n' {
            flush(&mut current, &mut width, &mut rows);
            index += 1;
            continue;
        }
        if options.max_glyphs.is_some_and(|max| count >= max) {
            break;
        }
        let codepoint = if character == '\t' { 32 } else { character as u32 };
        let resolved = resolve_text_glyph(options.font, codepoint, options.alternate)?;
        let scale = line_height / resolved.atlas.line_height.max(1) as f32;
        let advance = if character == '\t' {
            let tab = line_height * options.tab_columns as f32;
            tab - width % tab
        } else {
            resolved.glyph.advance as f32 * scale
        };
        let tint = if resolved.glyph.color {
            Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: color.w,
            }
        } else if options.alternate {
            Vec4 {
                x: 0.85,
                y: 0.65,
                z: 0.12,
                w: color.w,
            }
        } else {
            color
        };
        let cell = Cell {
            atlas: resolved.atlas,
            glyph: resolved.glyph,
            color: tint,
            source_offset,
            character,
            advance,
            scale,
            visible: resolved.visible && character != '\t',
        };
        if width + cell.advance > maximum && !current.is_empty() {
            let mut break_at: Option<usize> = None;
            for (position, existing) in current.iter().enumerate().rev() {
                if existing.character == ' ' || existing.character == '\t' {
                    break_at = Some(position);
                    break;
                }
            }
            match break_at {
                Some(position) => {
                    let mut remainder = current.split_off(position + 1);
                    current.pop();
                    flush(&mut current, &mut width, &mut rows);
                    width = remainder.iter().map(|cell| cell.advance).sum();
                    if width + cell.advance > maximum && !remainder.is_empty() {
                        flush(&mut remainder, &mut width, &mut rows);
                    }
                    current = remainder;
                }
                None => flush(&mut current, &mut width, &mut rows),
            }
        }
        current.push(cell);
        width += advance;
        count += 1;
        index += 1;
    }
    if !current.is_empty() || rows.is_empty() || options.text.ends_with('\n') {
        flush(&mut current, &mut width, &mut rows);
    }
    let mut lines = Vec::with_capacity(rows.len());
    let mut widest = 0.0f32;
    for (line_number, row) in rows.into_iter().enumerate() {
        let row_width: f32 = row.iter().map(|cell| cell.advance).sum();
        let y = line_number as f32 * line_height;
        let available = if maximum.is_finite() { maximum } else { row_width };
        let mut x = match options.align {
            TextAlign::Center => (available - row_width) / 2.0,
            TextAlign::Right => available - row_width,
            TextAlign::Left => 0.0,
        };
        let mut glyphs = Vec::with_capacity(row.len());
        for cell in row {
            let resolved = super::atlas::ResolvedTextGlyph {
                atlas: cell.atlas,
                glyph: cell.glyph,
                codepoint: cell.character as u32,
                visible: cell.character != ' ' && cell.character != '\t',
            };
            glyphs.push(PositionedGlyph {
                source_offset: cell.source_offset,
                atlas: cell.atlas,
                glyph: cell.glyph,
                rect: Rect {
                    x,
                    y,
                    width: cell.glyph.width as f32 * cell.scale,
                    height: cell.glyph.height as f32 * cell.scale,
                },
                uv: glyph_uv(&resolved),
                color: cell.color,
                visible: cell.visible,
            });
            x += cell.advance;
        }
        widest = widest.max(row_width);
        lines.push(TextLine {
            width: row_width,
            y,
            glyphs,
        });
    }
    Ok(TextLayout {
        width: widest,
        height: lines.len() as f32 * line_height,
        line_height,
        lines,
        glyph_count: count,
    })
}

/// Draw a layout (`drawTextLayout`).
pub fn draw_text_layout(
    draw: &mut Draw2D,
    layout: &TextLayout,
    origin: Vec2,
    shadow_offset: f32,
) {
    let mut pass = |shadow: bool| {
        for line in &layout.lines {
            for glyph in &line.glyphs {
                if !glyph.visible {
                    continue;
                }
                draw.set_color(Some(if shadow {
                    Vec4 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: glyph.color.w,
                    }
                } else {
                    glyph.color
                }));
                draw.stretch_pic(
                    Rect {
                        x: origin.x + glyph.rect.x + if shadow { shadow_offset } else { 0.0 },
                        y: origin.y + glyph.rect.y + if shadow { shadow_offset } else { 0.0 },
                        width: glyph.rect.width,
                        height: glyph.rect.height,
                    },
                    glyph.uv,
                    super::atlas::atlas_picture(glyph.atlas),
                );
            }
        }
    };
    if shadow_offset > 0.0 {
        pass(true);
    }
    pass(false);
    draw.set_color(None);
}

/// Seat text scales (`SeatTextScale`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeatTextScale {
    /// Console scale.
    pub console: f32,
    /// Console width.
    pub console_width: f32,
    /// Console height.
    pub console_height: f32,
    /// Status-bar scale.
    pub status_bar: f32,
    /// Crosshair scale.
    pub crosshair: f32,
}

/// Seat scaling formulas (`seatTextScale`).
#[must_use]
pub fn seat_text_scale(
    viewport: Rect,
    console_scale: f32,
    status_bar_scale: f32,
    crosshair_scale: f32,
) -> SeatTextScale {
    let automatic = (viewport.height / 300.0).floor().max(1.0);
    let requested = if console_scale > 0.0 {
        console_scale
    } else {
        automatic
    };
    let console_width = ((viewport.width / requested).max(320.0).min(viewport.width.max(320.0))).floor().max(8.0);
    let console_width = ((console_width as i32) & !7) as f32;
    let fit = ((viewport.width / 320.0).floor() as i32)
        .min((viewport.height / 144.0).floor() as i32)
        .max(1) as f32;
    SeatTextScale {
        console: viewport.width / console_width,
        console_width,
        console_height: if viewport.width <= 0.0 {
            viewport.height
        } else {
            (console_width * viewport.height / viewport.width).round()
        },
        status_bar: if status_bar_scale > 0.0 {
            status_bar_scale.max(1.0).min(fit)
        } else {
            fit
        },
        crosshair: crosshair_scale.max(1.0).min(10.0),
    }
}

/// Seat text presentation (`SeatTextPresentation`).
pub struct SeatTextPresentation<'a> {
    /// Seat.
    pub seat: SeatId,
    /// Font.
    pub font: &'a TextFontSelection,
}

impl<'a> SeatTextPresentation<'a> {
    /// New presentation.
    #[must_use]
    pub const fn new(seat: SeatId, font: &'a TextFontSelection) -> Self {
        Self { seat, font }
    }

    /// Lay out text.
    pub fn layout(
        &self,
        options: TextLayoutOptionsWithoutFont<'a>,
    ) -> Result<TextLayout<'a>, ClientError> {
        layout_text(&options.with_font(self.font))
    }

    /// Draw text.
    pub fn draw(
        &self,
        draw: &mut Draw2D,
        options: TextLayoutOptionsWithoutFont<'a>,
        origin: Vec2,
        shadow_offset: f32,
    ) -> Result<TextLayout<'a>, ClientError> {
        if self.seat != *draw.commands.seat() {
            return Err(ClientError::BadText(
                "Text presentation belongs to a different seat".to_string(),
            ));
        }
        let layout = self.layout(options)?;
        draw_text_layout(draw, &layout, origin, shadow_offset);
        Ok(layout)
    }
}

/// Layout options without a font (seat supplies it).
#[derive(Debug, Clone)]
pub struct TextLayoutOptionsWithoutFont<'a> {
    /// Text.
    pub text: &'a str,
    /// Scale.
    pub scale: f32,
    /// Base color.
    pub color: Vec4,
    /// Color codes.
    pub color_codes: ColorCodes,
    /// Force color.
    pub force_color: bool,
    /// Alternate tint.
    pub alternate: bool,
    /// Maximum width.
    pub max_width: Option<f32>,
    /// Alignment.
    pub align: TextAlign,
    /// Line height.
    pub line_height: Option<f32>,
    /// Maximum glyphs.
    pub max_glyphs: Option<usize>,
    /// Tab columns.
    pub tab_columns: usize,
}

impl<'a> TextLayoutOptionsWithoutFont<'a> {
    fn with_font(&self, font: &'a TextFontSelection) -> TextLayoutOptions<'a> {
        TextLayoutOptions {
            text: self.text,
            font,
            scale: self.scale,
            color: self.color,
            color_codes: self.color_codes,
            force_color: self.force_color,
            alternate: self.alternate,
            max_width: self.max_width,
            align: self.align,
            line_height: self.line_height,
            max_glyphs: self.max_glyphs,
            tab_columns: self.tab_columns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::atlas::{classic_charset, TextFontSelection};

    fn selection() -> TextFontSelection {
        TextFontSelection::Classic {
            classic: classic_charset(1, 128, 128, "conchars", false).unwrap(),
            unicode: None,
        }
    }

    fn options<'a>(font: &'a TextFontSelection, text: &'a str) -> TextLayoutOptions<'a> {
        TextLayoutOptions {
            text,
            font,
            scale: 1.0,
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            color_codes: ColorCodes::Literal,
            force_color: false,
            alternate: false,
            max_width: None,
            align: TextAlign::Left,
            line_height: None,
            max_glyphs: None,
            tab_columns: 4,
        }
    }

    #[test]
    fn measures_single_line() {
        let font = selection();
        let layout = layout_text(&options(&font, "AB")).unwrap();
        assert_eq!(layout.lines.len(), 1);
        assert_eq!(layout.glyph_count, 2);
        assert!((layout.width - 16.0).abs() < 1e-6);
    }

    #[test]
    fn q3_escapes_change_color() {
        let font = selection();
        let mut opts = options(&font, "A^1B");
        opts.color_codes = ColorCodes::Q3;
        let layout = layout_text(&opts).unwrap();
        assert_eq!(layout.glyph_count, 2);
        assert_eq!(layout.lines[0].glyphs[1].color.x, 1.0);
        assert_eq!(layout.lines[0].glyphs[1].color.y, 0.0);
    }

    #[test]
    fn wraps_at_word_boundary() {
        let font = selection();
        let mut opts = options(&font, "A B C");
        opts.max_width = Some(20.0);
        let layout = layout_text(&opts).unwrap();
        assert!(layout.lines.len() >= 2);
    }

    #[test]
    fn bad_scale_is_an_error() {
        let font = selection();
        let mut opts = options(&font, "A");
        opts.scale = 0.0;
        assert!(layout_text(&opts).is_err());
    }

    #[test]
    fn seat_scale_defaults() {
        let scale = seat_text_scale(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
            0.0,
            1.0,
            1.0,
        );
        assert!(scale.console > 0.0);
        assert_eq!(scale.crosshair, 1.0);
    }
}
