//! Active powerup timers HUD.
//!
//! Donor provenance: `src/ui/hud/powerups.ts` in full (row/column packing,
//! scale clamping, right-aligned label plus countdown rows). Layout math uses
//! `f32`; text measurement falls back to eight units per character when no
//! measurer is supplied.

use std::rc::Rc;

use qa_core::math::Vec2;

use crate::text::draw2d::Rect;
use crate::ui::common::skin::UiSkin;
use crate::ui::types::{ItemId, TextAlign, UiDrawCommand};

/// Text width measurer: width of `text` rendered at `scale`.
pub type MeasureText = Rc<dyn Fn(&str, f32) -> f32>;

/// One active powerup timer.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerupTimerView {
    /// Powerup item.
    pub item: ItemId,
    /// Unlocalized label.
    pub label: String,
    /// Seconds remaining.
    pub remaining_seconds: f32,
}

/// Draw right-aligned powerup timer rows packed into columns.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn draw_powerup_timers(
    timers: &[PowerupTimerView],
    area: &Rect,
    bottom: f32,
    skin: &UiSkin,
    requested_scale: f32,
    measure_text: &Option<MeasureText>,
    localize: &dyn Fn(&str) -> String,
) -> Vec<UiDrawCommand> {
    let active: Vec<&PowerupTimerView> = timers.iter().filter(|timer| timer.remaining_seconds > 0.0).collect();
    if active.is_empty() {
        return Vec::new();
    }
    let cap_height = skin.cap_ink.map_or(8.0, |ink| ink.height);
    let available_height = 0.0_f32.max(bottom - area.y - 8.0);
    let rows = active
        .len()
        .min(1_usize.max((available_height / 12.0).floor() as usize));
    let columns = active.len().div_ceil(rows);
    let scale = (8.0 / cap_height)
        .max(requested_scale)
        .min(1.0_f32.max(available_height / rows as f32 - 4.0) / cap_height);
    let row_height = cap_height * scale + 4.0;
    let width = 200.0_f32.min(area.width / 2_usize.max(columns) as f32 - 8.0);
    let mut commands = Vec::with_capacity(active.len() * 2);
    for (index, timer) in active.iter().enumerate() {
        let column = index / rows;
        let row = index % rows;
        let x = area.x + area.width - (column + 1) as f32 * (width + 8.0) + 4.0;
        let y = bottom - (rows - row) as f32 * row_height;
        let value = format!("{}s", timer.remaining_seconds.ceil() as i32);
        let mut label: Vec<char> = localize(&timer.label).chars().collect();
        loop {
            let candidate = format!("{} {value}", label.iter().collect::<String>());
            let measured = match measure_text {
                Some(measure) => measure(&candidate, scale),
                None => candidate.encode_utf16().count() as f32 * 8.0 * scale,
            };
            if label.is_empty() || measured <= width - 8.0 {
                break;
            }
            label.pop();
        }
        let text = format!("{} {value}", label.iter().collect::<String>());
        let text = text.trim_start().to_string();
        commands.push(UiDrawCommand::Fill {
            rect: Rect {
                x,
                y,
                width,
                height: row_height,
            },
            color: skin.colors.panel,
        });
        commands.push(UiDrawCommand::Text {
            origin: Vec2 {
                x: x + width - 4.0,
                y: y + 2.0 - skin.cap_ink.map_or(0.0, |ink| ink.top) * scale,
            },
            text,
            font: skin.font.clone(),
            scale,
            color: skin.colors.text,
            align: TextAlign::Right,
            shadow: true,
        });
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::atlas::CapInk;
    use crate::ui::common::skin::default_ui_skin;
    use crate::ui::types::ResourceId;

    fn font() -> ResourceId {
        ResourceId::new("resource:test:font").unwrap()
    }

    fn timer(label: &str, remaining_seconds: f32) -> PowerupTimerView {
        PowerupTimerView {
            item: ItemId::new("item:test:powerup"),
            label: label.to_string(),
            remaining_seconds,
        }
    }

    fn identity(text: &str) -> String {
        text.to_string()
    }

    fn text_of(command: &UiDrawCommand) -> (Vec2, String, f32) {
        match command {
            UiDrawCommand::Text {
                origin,
                text,
                scale,
                color,
                align,
                shadow,
                ..
            } => {
                assert_eq!(*align, TextAlign::Right);
                assert!(*shadow);
                let skin = default_ui_skin(&font());
                assert_eq!(*color, skin.colors.text);
                (*origin, text.clone(), *scale)
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    fn fill_of(command: &UiDrawCommand) -> Rect {
        match command {
            UiDrawCommand::Fill { rect, .. } => *rect,
            other => panic!("expected fill, got {other:?}"),
        }
    }

    #[test]
    fn empty_and_expired_timers_draw_nothing() {
        let skin = default_ui_skin(&font());
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
        };
        let none: Option<MeasureText> = None;
        assert!(draw_powerup_timers(&[], &area, 200.0, &skin, 1.0, &none, &identity).is_empty());
        let expired = vec![timer("Quad", 0.0), timer("Haste", -2.0)];
        assert!(draw_powerup_timers(&expired, &area, 200.0, &skin, 1.0, &none, &identity).is_empty());
    }

    #[test]
    fn single_timer_draws_exact_fill_and_countdown() {
        let skin = default_ui_skin(&font());
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
        };
        let none: Option<MeasureText> = None;
        let timers = vec![timer("Quad", 4.2)];
        let commands = draw_powerup_timers(&timers, &area, 200.0, &skin, 1.0, &none, &identity);
        assert_eq!(commands.len(), 2);
        assert_eq!(
            fill_of(&commands[0]),
            Rect {
                x: 164.0,
                y: 188.0,
                width: 152.0,
                height: 12.0
            }
        );
        let (origin, text, scale) = text_of(&commands[1]);
        assert_eq!(text, "Quad 5s");
        assert_eq!(scale, 1.0);
        assert_eq!(origin, Vec2 { x: 312.0, y: 190.0 });
    }

    #[test]
    fn timers_pack_into_columns_when_height_is_short() {
        let skin = default_ui_skin(&font());
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
        };
        let none: Option<MeasureText> = None;
        let timers = vec![
            timer("T0", 10.0),
            timer("T1", 20.0),
            timer("T2", 30.0),
            timer("T3", 40.0),
            timer("T4", 50.0),
        ];
        let commands = draw_powerup_timers(&timers, &area, 40.0, &skin, 1.0, &none, &identity);
        assert_eq!(commands.len(), 10);
        let width = 320.0_f32 / 3.0 - 8.0;
        let expected = [
            ("T0 10s", 0_usize, 0_usize),
            ("T1 20s", 0, 1),
            ("T2 30s", 1, 0),
            ("T3 40s", 1, 1),
            ("T4 50s", 2, 0),
        ];
        for (index, &(text, column, row)) in expected.iter().enumerate() {
            let x = 320.0 - (column + 1) as f32 * (width + 8.0) + 4.0;
            let y = 40.0 - (2 - row) as f32 * 12.0;
            assert_eq!(
                fill_of(&commands[index * 2]),
                Rect {
                    x,
                    y,
                    width,
                    height: 12.0
                }
            );
            let (origin, actual, scale) = text_of(&commands[index * 2 + 1]);
            assert_eq!(actual, text.to_string());
            assert_eq!(scale, 1.0);
            assert_eq!(
                origin,
                Vec2 {
                    x: x + width - 4.0,
                    y: y + 2.0
                }
            );
        }
        assert_eq!(fill_of(&commands[0]).y, 16.0);
        assert_eq!(fill_of(&commands[2]).y, 28.0);
    }

    #[test]
    fn long_labels_trim_until_countdown_fits() {
        let skin = default_ui_skin(&font());
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 200.0,
        };
        let none: Option<MeasureText> = None;
        let timers = vec![timer("Invisibility", 8.6)];
        let commands = draw_powerup_timers(&timers, &area, 200.0, &skin, 1.0, &none, &identity);
        assert_eq!(commands.len(), 2);
        assert_eq!(
            fill_of(&commands[0]),
            Rect {
                x: 64.0,
                y: 188.0,
                width: 52.0,
                height: 12.0
            }
        );
        let (_, text, _) = text_of(&commands[1]);
        assert_eq!(text, "In 9s");
    }

    #[test]
    fn localize_applies_and_expired_timers_are_skipped() {
        let skin = default_ui_skin(&font());
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
        };
        let none: Option<MeasureText> = None;
        let timers = vec![timer("spent", 0.0), timer("quad", 4.2), timer("rotten", -1.0)];
        let commands = draw_powerup_timers(&timers, &area, 200.0, &skin, 1.0, &none, &|text| text.to_uppercase());
        assert_eq!(commands.len(), 2);
        let (_, text, _) = text_of(&commands[1]);
        assert_eq!(text, "QUAD 5s");
    }

    #[test]
    fn small_cap_height_raises_minimum_scale() {
        let mut skin = default_ui_skin(&font());
        skin.cap_ink = Some(CapInk { top: 1.0, height: 4.0 });
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
        };
        let none: Option<MeasureText> = None;
        let timers = vec![timer("Quad", 4.2)];
        let commands = draw_powerup_timers(&timers, &area, 200.0, &skin, 1.0, &none, &identity);
        assert_eq!(commands.len(), 2);
        assert_eq!(
            fill_of(&commands[0]),
            Rect {
                x: 164.0,
                y: 188.0,
                width: 152.0,
                height: 12.0
            }
        );
        let (origin, text, scale) = text_of(&commands[1]);
        assert_eq!(text, "Quad 5s");
        assert_eq!(scale, 2.0);
        assert_eq!(origin, Vec2 { x: 312.0, y: 188.0 });
    }

    #[test]
    fn custom_measurer_can_trim_the_whole_label() {
        let skin = default_ui_skin(&font());
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
        };
        let wide: Option<MeasureText> = Some(Rc::new(|_text: &str, _scale: f32| 1000.0));
        let timers = vec![timer("Quad", 4.2)];
        let commands = draw_powerup_timers(&timers, &area, 200.0, &skin, 1.0, &wide, &identity);
        assert_eq!(commands.len(), 2);
        let (_, text, _) = text_of(&commands[1]);
        assert_eq!(text, "5s");
    }
}
