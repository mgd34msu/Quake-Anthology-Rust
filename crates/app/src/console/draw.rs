//! Console overlay display rendering.
//!
//! Donor provenance: `src/console/draw.ts` (`drawConsole`) with the
//! per-glyph fit from `src/console/metrics.ts` (`consoleGlyphMetrics`).
//! Same clipped picture sink, same fixed-cell layout with centered
//! variable-width ink, same scrollback runs, same input-field scroll and
//! caret blink, same selected-entry help lines. All text and draw types
//! come from `qa-client` read-only; nothing here duplicates them.
//!
//! The donor takes a `Draw2D` value; this port takes its parts (the
//! command sink plus the coordinate space) so the clipped wrapper can
//! borrow the caller's sink without reallocating it.

use qa_client::text::atlas::{resolve_text_glyph, TextFontSelection};
use qa_client::text::draw2d::{clip_picture, CoordinateSpace, Draw2D, PictureAsset, Rect, TextDrawSink, TextureRect};
use qa_client::text::layout::{ColorCodes, SeatTextPresentation, TextAlign, TextLayoutOptionsWithoutFont};
use qa_core::identity::SeatId;
use qa_core::math::{vec2, vec4, Vec4};

use super::buffer::ConsoleRow;
use super::discovery::{ConsoleDiscoveryEntry, DiscoveryKind};
use super::field::ConsoleField;
use super::llm_batch::json_quote;
use super::ConsoleError;

/// Console palette (donor `colors`).
fn palette() -> [Vec4; 8] {
    [
        vec4(0.0, 0.0, 0.0, 1.0),
        vec4(1.0, 0.0, 0.0, 1.0),
        vec4(0.0, 1.0, 0.0, 1.0),
        vec4(1.0, 1.0, 0.0, 1.0),
        vec4(0.0, 0.0, 1.0, 1.0),
        vec4(0.0, 1.0, 1.0, 1.0),
        vec4(1.0, 0.0, 1.0, 1.0),
        vec4(1.0, 1.0, 1.0, 1.0),
    ]
}

/// Selected-entry help tint.
fn help_color() -> Vec4 {
    vec4(0.65, 0.85, 1.0, 1.0)
}

/// A sink that clips every picture to the console background bounds.
struct ClipSink<'a> {
    inner: &'a mut dyn TextDrawSink,
    bounds: Rect,
}

impl TextDrawSink for ClipSink<'_> {
    fn seat(&self) -> &SeatId {
        self.inner.seat()
    }

    fn target(&self) -> Rect {
        self.inner.target()
    }

    fn set_color(&mut self, color: Option<Vec4>) {
        self.inner.set_color(color);
    }

    fn stretch_pixels(&mut self, rect: Rect, uv: TextureRect, picture: PictureAsset) {
        if let Some((rect, uv)) = clip_picture(&rect, &uv, &self.bounds) {
            self.inner.stretch_pixels(rect, uv, picture);
        }
    }
}

/// Per-glyph console fit (donor `consoleGlyphMetrics`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsoleGlyphFit {
    /// Line height override for this glyph.
    pub line_height: f32,
    /// Ink top offset for this glyph.
    pub top: f32,
}

/// Fit one glyph into its fixed cell.
///
/// Text-ness follows the donor (ASCII printables plus Unicode letters
/// and numbers; combining marks use the same advance path). Alternate
/// banks share geometry, so resolution always uses the base bank.
pub fn console_glyph_metrics(
    font: &TextFontSelection,
    character: char,
    line_height: f32,
    cell_width: f32,
) -> Result<ConsoleGlyphFit, ConsoleError> {
    let resolved = resolve_text_glyph(font, character as u32, false)
        .map_err(|error| ConsoleError::BadCommand(error.to_string()))?;
    let code = character as u32;
    let is_text = (32..=126).contains(&code) || character.is_alphabetic() || character.is_numeric();
    let cap = if is_text { resolved.atlas.cap_ink.as_ref() } else { None };
    let normalization = cap.map_or(1.0, |cap| if cap.height > 0.0 { 6.0 / cap.height } else { 1.0 });
    let natural = resolved.glyph.width as f32 * line_height / resolved.atlas.line_height.max(1) as f32 * normalization;
    let fit = (cell_width / natural.max(1.0)).min(1.0);
    Ok(ConsoleGlyphFit {
        line_height: line_height * normalization * fit,
        top: cap.map_or(0.0, |cap| cap.top) * line_height / 8.0 * normalization * fit,
    })
}

/// Console draw inputs (donor `ConsoleDrawOptions`).
pub struct ConsoleDrawOptions<'a, 'b> {
    /// Command sink receiving clipped pictures.
    pub commands: &'a mut dyn TextDrawSink,
    /// Coordinate space.
    pub space: CoordinateSpace,
    /// Seat text presentation (must belong to the sink's seat).
    pub text: &'a SeatTextPresentation<'b>,
    /// Visible scrollback rows (oldest first).
    pub rows: &'a [ConsoleRow],
    /// Input field, when the console reads input.
    pub field: Option<&'a ConsoleField>,
    /// Console height in pixels.
    pub height: f32,
    /// Integer pixel scale.
    pub scale: f32,
    /// Fixed cell width override (defaults to the line height).
    pub cell_width: Option<f32>,
    /// Current time in milliseconds (caret blink).
    pub now_ms: i64,
    /// Background picture, when the console shows one.
    pub background: Option<PictureAsset>,
    /// Completion entry previewed for help lines.
    pub selected_entry: Option<&'a ConsoleDiscoveryEntry>,
}

/// Measure fixed-cell text.
fn measure(value: &str, cell_width: f32) -> f32 {
    value.chars().count() as f32 * cell_width
}

/// Fit text into a pixel width with an optional ellipsis (donor `fit`).
fn fit(value: &str, maximum: f32, ellipsis: bool, cell_width: f32) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| match character {
            '\r' | '\n' | '\t' => ' ',
            _ => character,
        })
        .collect();
    if measure(&cleaned, cell_width) <= maximum {
        return cleaned;
    }
    let suffix = if ellipsis && measure("...", cell_width) <= maximum {
        "..."
    } else {
        ""
    };
    let characters: Vec<char> = cleaned.chars().collect();
    let mut low = 0;
    let mut high = characters.len();
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let candidate: String = characters[..middle].iter().collect::<String>() + suffix;
        if measure(&candidate, cell_width) <= maximum {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    characters[..low].iter().collect::<String>() + suffix
}

/// Draw fixed-cell text with centered variable-width ink (donor `drawCells`).
#[allow(clippy::too_many_arguments)]
fn draw_cells(
    draw: &mut Draw2D,
    text: &SeatTextPresentation,
    font: &TextFontSelection,
    value: &str,
    mut x: f32,
    y: f32,
    scale: f32,
    color: Vec4,
    cell_width: f32,
    line_height: f32,
    alternate: bool,
) -> Result<(), ConsoleError> {
    for character in value.chars() {
        let glyph = console_glyph_metrics(font, character, line_height, cell_width)?;
        let mut encoded = [0u8; 4];
        let single = character.encode_utf8(&mut encoded);
        let options = TextLayoutOptionsWithoutFont {
            text: single,
            scale,
            color,
            color_codes: ColorCodes::Literal,
            force_color: false,
            alternate,
            max_width: None,
            align: TextAlign::Left,
            line_height: Some(glyph.line_height),
            max_glyphs: None,
            tab_columns: 4,
        };
        let width = text
            .layout(options.clone())
            .map_err(|error| ConsoleError::BadCommand(error.to_string()))?
            .width;
        text.draw(draw, options, vec2(x + (cell_width - width) / 2.0, y - glyph.top), 0.0)
            .map_err(|error| ConsoleError::BadCommand(error.to_string()))?;
        x += cell_width;
    }
    Ok(())
}

/// Draw the console overlay (donor `drawConsole`).
pub fn draw_console(options: ConsoleDrawOptions) -> Result<(), ConsoleError> {
    if options.commands.seat() != &options.text.seat {
        return Err(ConsoleError::BadCommand(
            "Console text belongs to another viewport seat".to_string(),
        ));
    }
    let font = options.text.font.clone();
    let target = options.commands.target();
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: target.width,
        height: options.height.min(target.height),
    };
    let mut sink = ClipSink {
        inner: options.commands,
        bounds,
    };
    let mut draw = Draw2D::new(&mut sink, options.space);
    if let Some(background) = options.background {
        draw.draw_pic(
            Rect {
                x: 0.0,
                y: 0.0,
                width: target.width,
                height: options.height,
            },
            background,
        );
    }
    let scale = options.scale;
    let line_height = 8.0 * scale;
    let field_lines = usize::from(options.field.is_some());
    let available = (target.width - 16.0 * scale).max(1.0);
    let cell_width = options.cell_width.unwrap_or(line_height);
    let mut help = Vec::new();
    if options.field.is_some() {
        if let Some(entry) = options.selected_entry {
            match &entry.usage {
                Some(usage) => help.push(format!("Usage: {usage}")),
                None => help.push(entry.name.clone()),
            }
            if let Some(summary) = &entry.summary {
                help.push(summary.clone());
            }
            match entry.kind {
                DiscoveryKind::Cvar => {
                    help.push(format!("Current: {}", json_quote(entry.value.as_deref().unwrap_or(""))))
                }
                DiscoveryKind::Alias => {
                    help.push(format!("Expands to: {}", entry.alias_value.as_deref().unwrap_or("")));
                }
                DiscoveryKind::Command => {}
            }
        }
    }
    let bottom = (options.height.min(target.height) - 2.0 * scale).max(0.0);
    let total_rows = (bottom / line_height).floor().max(0.0) as usize;
    if total_rows < field_lines {
        return Ok(());
    }
    let help_lines = help[..help.len().min(3).min(total_rows - field_lines)].to_vec();
    let row_count = options.rows.len().min(total_rows - field_lines - help_lines.len());
    let mut y = bottom - line_height * (row_count + field_lines + help_lines.len()) as f32;
    let colors = palette();
    let white = vec4(1.0, 1.0, 1.0, 1.0);
    for row in &options.rows[options.rows.len() - row_count..] {
        let mut x = 8.0 * scale;
        let mut index = 0;
        while index < row.cells.len() {
            let first = row.cells[index];
            let mut run = String::new();
            run.push(first.character);
            index += 1;
            while index < row.cells.len()
                && row.cells[index].color == first.color
                && row.cells[index].alternate == first.alternate
            {
                run.push(row.cells[index].character);
                index += 1;
            }
            let color = colors.get(usize::from(first.color)).copied().unwrap_or(white);
            draw_cells(
                &mut draw,
                options.text,
                &font,
                &run,
                x,
                y,
                scale,
                color,
                cell_width,
                line_height,
                first.alternate,
            )?;
            x += measure(&run, cell_width);
        }
        y += line_height;
    }
    if let Some(field) = options.field {
        let characters: Vec<char> = field.text().chars().collect();
        let caret = if field.overstrike { "_" } else { "|" };
        let cursor_width = measure(caret, cell_width);
        let mut scroll = field.scroll.min(field.cursor);
        while scroll < field.cursor.min(characters.len()) {
            let prefix: String = characters[scroll..field.cursor.min(characters.len())].iter().collect();
            if measure(&format!("]{prefix}"), cell_width) + cursor_width <= available {
                break;
            }
            scroll += 1;
        }
        let tail: String = characters[scroll.min(characters.len())..].iter().collect();
        let visible = fit(
            &tail,
            (available - measure("]", cell_width) - cursor_width).max(0.0),
            false,
            cell_width,
        );
        draw_cells(
            &mut draw,
            options.text,
            &font,
            &format!("]{visible}"),
            8.0 * scale,
            y,
            scale,
            white,
            cell_width,
            line_height,
            false,
        )?;
        if ((options.now_ms / 256) & 1) == 0 {
            let prefix: String = characters[scroll.min(characters.len())..field.cursor.min(characters.len())]
                .iter()
                .collect();
            let prefix = format!("]{prefix}");
            draw_cells(
                &mut draw,
                options.text,
                &font,
                caret,
                8.0 * scale + measure(&prefix, cell_width),
                y,
                scale,
                white,
                cell_width,
                line_height,
                false,
            )?;
        }
        y += line_height;
        for line in &help_lines {
            draw_cells(
                &mut draw,
                options.text,
                &font,
                &fit(line, available, true, cell_width),
                8.0 * scale,
                y,
                scale,
                help_color(),
                cell_width,
                line_height,
                false,
            )?;
            y += line_height;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::text::atlas::{classic_charset, AtlasGlyph, TextAtlas};
    use qa_client::text::draw2d::{DrawCommand, TextCommandSink};
    use qa_client::text::layout::SeatTextPresentation;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    use super::super::buffer::ConsoleBuffer;
    use super::super::discovery::DiscoveryKind;

    fn presentation<'a>(owner: &IdentityOwner, font: &'a TextFontSelection) -> SeatTextPresentation<'a> {
        SeatTextPresentation::new(owner.seat(0), font)
    }

    fn sink(owner: &IdentityOwner, width: f32, height: f32) -> TextCommandSink {
        TextCommandSink::new(
            owner.seat(0),
            Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
        )
    }

    fn stretch_rects(sink: &TextCommandSink) -> Vec<Rect> {
        sink.commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::StretchPic { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect()
    }

    fn read_lines(sink: &TextCommandSink, width: f32, cell: f32) -> std::collections::BTreeMap<i64, String> {
        let mut lines: std::collections::BTreeMap<i64, String> = std::collections::BTreeMap::new();
        for command in &sink.commands {
            if let DrawCommand::StretchPic { rect, uv, .. } = command {
                let code = (uv.t * 16.0).round() as i64 * 16 + (uv.s * 16.0).round() as i64;
                let character = char::from_u32(code as u32).unwrap_or('?');
                let key = rect.y.round() as i64;
                let entry = lines.entry(key).or_default();
                let column = ((rect.x - 8.0) / cell).round() as usize;
                while entry.chars().count() < column {
                    entry.push(' ');
                }
                entry.push(character);
            }
        }
        let _ = width;
        lines
    }

    #[test]
    fn help_stays_below_input_inside_viewport() {
        for width in [320.0f32, 640.0] {
            let owner = IdentityOwner::create(&format!("console-draw-{width}")).unwrap();
            let font = TextFontSelection::Classic {
                classic: classic_charset(1, 128, 128, "test font", false).unwrap(),
                unicode: None,
            };
            let text = presentation(&owner, &font);
            let height = width * 0.75 / 2.0;
            let mut commands = sink(&owner, width, height * 2.0);
            let rows = {
                let mut field = ConsoleField::new();
                field.set_text(&"draft_".repeat(30));
                let before = (field.text(), field.cursor, field.scroll);
                let mut buffer = ConsoleBuffer::with_capacity(Dialect::Q3, width as usize / 8 - 2, 32_768).unwrap();
                for index in 0..60 {
                    buffer.print(&format!("scrollback {index}\n"), index).unwrap();
                }
                let rows = buffer.visible(60);
                draw_console(ConsoleDrawOptions {
                    commands: &mut commands,
                    space: CoordinateSpace::Pixels,
                    text: &text,
                    rows: &rows,
                    field: Some(&field),
                    height,
                    scale: 1.0,
                    cell_width: None,
                    now_ms: 0,
                    background: None,
                    selected_entry: Some(&ConsoleDiscoveryEntry {
                        name: "setting".to_string(),
                        kind: DiscoveryKind::Cvar,
                        summary: Some("Actual registered summary".to_string()),
                        usage: Some(format!("setting <{}>", "long_argument_".repeat(12))),
                        examples: Vec::new(),
                        allowed_values: None,
                        alias_value: None,
                        value: Some("42".to_string()),
                        reset_value: Some("0".to_string()),
                        latched_value: None,
                    }),
                })
                .unwrap();
                assert_eq!((field.text(), field.cursor, field.scroll), before);
                rows
            };
            let _ = rows;
            for rect in stretch_rects(&commands) {
                assert!(rect.x >= 8.0, "{rect:?}");
                assert!(rect.x + rect.width <= width - 8.0 + 1e-3, "{rect:?}");
                assert!(rect.y >= 0.0, "{rect:?}");
                assert!(rect.y + rect.height <= height + 1e-3, "{rect:?}");
            }
            let lines = read_lines(&commands, width, 8.0);
            let key = |offset: f32| (height - offset).round() as i64;
            assert!(
                lines.get(&key(34.0)).is_some_and(|line| line.starts_with(']')),
                "{lines:?}"
            );
            assert!(
                lines.get(&key(34.0)).is_some_and(|line| line.ends_with('|')),
                "{lines:?}"
            );
            assert!(
                lines
                    .get(&key(26.0))
                    .is_some_and(|line| line.starts_with("Usage: setting <")),
                "{lines:?}"
            );
            assert!(
                lines.get(&key(26.0)).is_some_and(|line| line.ends_with("...")),
                "{lines:?}"
            );
            assert_eq!(
                lines.get(&key(18.0)).map(String::as_str),
                Some("Actual registered summary")
            );
            assert_eq!(lines.get(&key(10.0)).map(String::as_str), Some("Current: \"42\""));
        }
    }

    #[test]
    fn variable_width_glyphs_and_caret_share_fixed_cells() {
        let owner = IdentityOwner::create("console-variable-font").unwrap();
        let classic = classic_charset(1, 128, 128, "test font", false).unwrap();
        let mut glyphs = classic.glyphs.clone();
        let narrow = glyphs.get(&105).cloned().expect("missing test glyph");
        glyphs.insert(
            105,
            AtlasGlyph {
                width: 2,
                advance: 2,
                ..narrow
            },
        );
        let font = TextFontSelection::Atlas {
            font: TextAtlas {
                glyphs,
                ..classic.clone()
            },
            classic,
            fallbacks: Vec::new(),
        };
        let text = presentation(&owner, &font);
        let mut commands = sink(&owner, 640.0, 480.0);
        let mut field = ConsoleField::new();
        field.set_text("iW");
        draw_console(ConsoleDrawOptions {
            commands: &mut commands,
            space: CoordinateSpace::Pixels,
            text: &text,
            rows: &[],
            field: Some(&field),
            height: 240.0,
            scale: 2.0,
            cell_width: Some(16.0),
            now_ms: 0,
            background: None,
            selected_entry: None,
        })
        .unwrap();
        assert_eq!(
            stretch_rects(&commands)
                .iter()
                .map(|rect| (rect.x, rect.width))
                .collect::<Vec<_>>(),
            vec![(16.0, 16.0), (38.0, 4.0), (48.0, 16.0), (64.0, 16.0)]
        );
    }

    #[test]
    fn clips_glyph_ink_to_background_including_overshoot() {
        let owner = IdentityOwner::create("console-clipping").unwrap();
        let classic = classic_charset(1, 128, 128, "test font", false).unwrap();
        let mut glyphs = classic.glyphs.clone();
        let tall = glyphs.get(&65).cloned().expect("missing test glyph");
        glyphs.insert(65, AtlasGlyph { height: 16, ..tall });
        let font = TextFontSelection::Atlas {
            font: TextAtlas {
                glyphs,
                ..classic.clone()
            },
            classic,
            fallbacks: Vec::new(),
        };
        let text = presentation(&owner, &font);
        let mut commands = sink(&owner, 640.0, 480.0);
        let mut field = ConsoleField::new();
        field.set_text("A");
        draw_console(ConsoleDrawOptions {
            commands: &mut commands,
            space: CoordinateSpace::Pixels,
            text: &text,
            rows: &[],
            field: Some(&field),
            height: 240.0,
            scale: 2.0,
            cell_width: Some(16.0),
            now_ms: 0,
            background: None,
            selected_entry: None,
        })
        .unwrap();
        let rects = stretch_rects(&commands);
        assert_eq!(rects.len(), 3);
        assert!(rects.iter().all(|rect| rect.y >= 0.0 && rect.y + rect.height <= 240.0));
        assert_eq!(rects[1].height, 20.0);
    }

    #[test]
    fn rejects_text_from_another_seat() {
        let owner = IdentityOwner::create("console-seat").unwrap();
        let other = IdentityOwner::create("console-other").unwrap();
        let font = TextFontSelection::Classic {
            classic: classic_charset(1, 128, 128, "test font", false).unwrap(),
            unicode: None,
        };
        let text = presentation(&other, &font);
        let mut commands = sink(&owner, 640.0, 480.0);
        let error = draw_console(ConsoleDrawOptions {
            commands: &mut commands,
            space: CoordinateSpace::Pixels,
            text: &text,
            rows: &[],
            field: None,
            height: 240.0,
            scale: 1.0,
            cell_width: None,
            now_ms: 0,
            background: None,
            selected_entry: None,
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "Console text belongs to another viewport seat");
    }
}
