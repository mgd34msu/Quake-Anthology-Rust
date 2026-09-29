//! UI and world-space text (`UiTextRenderer`, `WorldTextStore`).
//!
//! Donor provenance: `src/text/ui.ts` and `src/text/world.ts`.
//! Resource/content ids are headless `u32` handles.

use std::collections::BTreeMap;

use qa_core::identity::SeatId;
use qa_core::math::{Vec3, Vec4};

use super::atlas::TextFontSelection;
use super::draw2d::Draw2D;
use super::layout::{draw_text_layout, layout_text, ColorCodes, TextAlign, TextLayout, TextLayoutOptions};
use crate::ClientError;

/// A UI text command (data subset of the `UiDrawCommand` text kind).
#[derive(Debug, Clone, PartialEq)]
pub struct UiTextCommand {
    /// Font resource.
    pub font: u32,
    /// Text.
    pub text: String,
    /// Scale.
    pub scale: f32,
    /// Color.
    pub color: Vec4,
    /// Origin.
    pub origin: super::draw2d::RectOrigin,
    /// Alignment.
    pub align: TextAlign,
    /// Drop shadow.
    pub shadow: bool,
}

/// A UI text renderer (`UiTextRenderer`).
#[derive(Debug, Default)]
pub struct UiTextRenderer {
    seat: SeatId,
    fonts: BTreeMap<u32, TextFontSelection>,
}

impl UiTextRenderer {
    /// New renderer.
    #[must_use]
    pub const fn new(seat: SeatId) -> Self {
        Self {
            seat,
            fonts: BTreeMap::new(),
        }
    }

    /// Bind a font.
    pub fn bind(&mut self, resource: u32, font: TextFontSelection) {
        self.fonts.insert(resource, font);
    }

    /// Release a font.
    pub fn release(&mut self, resource: u32) {
        self.fonts.remove(&resource);
    }

    /// Clear fonts.
    pub fn clear(&mut self) {
        self.fonts.clear();
    }

    /// Draw a text command.
    pub fn draw(
        &self,
        seat: &SeatId,
        command: &UiTextCommand,
        draw: &mut Draw2D,
    ) -> Result<TextLayout<'_>, ClientError> {
        if self.seat != *seat || self.seat != *draw.commands.seat() {
            return Err(ClientError::BadText(
                "UI text belongs to a different seat".to_string(),
            ));
        }
        let font = self.fonts.get(&command.font).ok_or_else(|| {
            ClientError::BadText("UI font has not been registered for this seat".to_string())
        })?;
        let layout = layout_text(&TextLayoutOptions {
            text: &command.text,
            font,
            scale: command.scale,
            color: command.color,
            color_codes: ColorCodes::Literal,
            force_color: false,
            alternate: false,
            max_width: None,
            align: TextAlign::Left,
            line_height: None,
            max_glyphs: None,
            tab_columns: 4,
        })?;
        let offset = match command.align {
            TextAlign::Center => layout.width / 2.0,
            TextAlign::Right => layout.width,
            TextAlign::Left => 0.0,
        };
        draw_text_layout(
            draw,
            &layout,
            qa_core::math::vec2(command.origin.x - offset, command.origin.y),
            if command.shadow { 1.0 } else { 0.0 },
        );
        Ok(layout)
    }
}

/// World text orientation (`WorldTextInput["orientation"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorldTextOrientation {
    /// Billboard.
    Billboard,
    /// Fixed angles.
    Fixed {
        /// Angles.
        angles: Vec3,
    },
}

/// World text input (`WorldTextInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldTextInput {
    /// Text.
    pub text: String,
    /// Origin.
    pub origin: Vec3,
    /// Color.
    pub color: Vec4,
    /// Cell size.
    pub cell_size: f32,
    /// Distance cull factor.
    pub distance_cull_factor: Option<f32>,
    /// Orientation.
    pub orientation: WorldTextOrientation,
    /// Depth test.
    pub depth_test: bool,
    /// Font.
    pub font: WorldTextFont,
}

/// World text font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldTextFont {
    /// Classic.
    Classic,
    /// Selected.
    Selected,
}

/// World text with content id (`WorldText`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldText {
    /// Input.
    pub input: WorldTextInput,
    /// Content id.
    pub content: u32,
}

#[derive(Debug, Clone)]
struct TimedText {
    text: WorldText,
    lifetime: WorldTextLifetime,
}

#[derive(Debug, Clone, Copy)]
enum WorldTextLifetime {
    Seconds { expires: f64 },
    Frame { first_frame: Option<u64> },
}

/// One server world owns these entries (`WorldTextStore`).
#[derive(Debug, Default)]
pub struct WorldTextStore {
    entries: Vec<TimedText>,
}

impl WorldTextStore {
    /// New store.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Submit text.
    pub fn submit(
        &mut self,
        text: WorldText,
        now_seconds: f64,
        lifetime_seconds: f64,
    ) -> Result<(), ClientError> {
        let mut values = vec![
            now_seconds,
            lifetime_seconds,
            text.input.cell_size as f64,
            text.input.origin.x as f64,
            text.input.origin.y as f64,
            text.input.origin.z as f64,
            text.input.color.x as f64,
            text.input.color.y as f64,
            text.input.color.z as f64,
            text.input.color.w as f64,
        ];
        if let Some(factor) = text.input.distance_cull_factor {
            values.push(factor as f64);
        }
        if let WorldTextOrientation::Fixed { angles } = text.input.orientation {
            values.extend([angles.x as f64, angles.y as f64, angles.z as f64]);
        }
        if !values.iter().all(|value| value.is_finite())
            || lifetime_seconds < 0.0
            || text.input.cell_size <= 0.0
        {
            return Err(ClientError::BadText(
                "Invalid world text geometry or lifetime".to_string(),
            ));
        }
        self.prune(now_seconds);
        self.entries.push(TimedText {
            text,
            lifetime: if lifetime_seconds == 0.0 {
                WorldTextLifetime::Frame { first_frame: None }
            } else {
                WorldTextLifetime::Seconds {
                    expires: now_seconds + lifetime_seconds,
                }
            },
        });
        Ok(())
    }

    /// Snapshot visible entries (`snapshot`).
    pub fn snapshot(&mut self, now_seconds: f64, frame: u64) -> Vec<WorldText> {
        self.prune(now_seconds);
        let mut visible = Vec::new();
        for entry in &mut self.entries {
            match entry.lifetime {
                WorldTextLifetime::Seconds { .. } => visible.push(entry.text.clone()),
                WorldTextLifetime::Frame { first_frame } => match first_frame {
                    None => {
                        entry.lifetime = WorldTextLifetime::Frame {
                            first_frame: Some(frame),
                        };
                        visible.push(entry.text.clone());
                    }
                    Some(first) if first == frame => visible.push(entry.text.clone()),
                    _ => {}
                },
            }
        }
        // Drop frame entries from older frames.
        self.entries.retain(|entry| match entry.lifetime {
            WorldTextLifetime::Seconds { .. } => true,
            WorldTextLifetime::Frame { first_frame } => {
                first_frame.is_none_or(|first| first == frame)
            }
        });
        visible
    }

    fn prune(&mut self, now_seconds: f64) {
        self.entries.retain(|entry| match entry.lifetime {
            WorldTextLifetime::Frame { .. } => true,
            WorldTextLifetime::Seconds { expires } => expires > now_seconds,
        });
    }

    /// Clear entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(content: u32) -> WorldText {
        WorldText {
            input: WorldTextInput {
                text: "hi".to_string(),
                origin: qa_core::math::vec3(0.0, 0.0, 0.0),
                color: qa_core::math::vec4(1.0, 1.0, 1.0, 1.0),
                cell_size: 8.0,
                distance_cull_factor: None,
                orientation: WorldTextOrientation::Billboard,
                depth_test: true,
                font: WorldTextFont::Classic,
            },
            content,
        }
    }

    #[test]
    fn frame_text_lives_one_frame() {
        let mut store = WorldTextStore::new();
        store.submit(text(1), 0.0, 0.0).unwrap();
        assert_eq!(store.snapshot(0.0, 7).len(), 1);
        assert!(store.snapshot(0.0, 8).is_empty());
    }

    #[test]
    fn timed_text_expires() {
        let mut store = WorldTextStore::new();
        store.submit(text(1), 0.0, 5.0).unwrap();
        assert_eq!(store.snapshot(4.0, 1).len(), 1);
        assert!(store.snapshot(6.0, 2).is_empty());
    }

    #[test]
    fn bad_geometry_is_an_error() {
        let mut store = WorldTextStore::new();
        let mut bad = text(1);
        bad.input.cell_size = 0.0;
        assert!(store.submit(bad, 0.0, 1.0).is_err());
    }

    #[test]
    fn ui_draw_needs_bound_font() {
        use crate::text::draw2d::{CoordinateSpace, Rect, TextCommandSink};
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("test").unwrap();
        let renderer = UiTextRenderer::new(owner.seat(0));
        let mut sink = TextCommandSink::new(
            owner.seat(0),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
        );
        let mut draw = Draw2D::new(&mut sink, CoordinateSpace::Pixels);
        let seat = owner.seat(0);
        assert!(
            renderer
                .draw(
                    &seat,
                    &UiTextCommand {
                        font: 1,
                        text: "hi".to_string(),
                        scale: 1.0,
                        color: qa_core::math::vec4(1.0, 1.0, 1.0, 1.0),
                        origin: crate::text::draw2d::RectOrigin { x: 0.0, y: 0.0 },
                        align: TextAlign::Left,
                        shadow: false,
                    },
                    &mut draw,
                )
                .is_err()
        );
    }
}
