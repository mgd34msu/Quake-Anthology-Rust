//! Aspect-correct 640x480 UI space fitted to each seat's safe area.
//!
//! Donor provenance: `src/ui/common/layout.ts` in full (`fitUi`, `uiPoint`,
//! `contains`, `intersect`, `transformUi`, `menuRow`; Q3 `ui_atoms.c`). All
//! coordinates are `f32`.

use qa_core::math::Vec2;

use crate::error::ClientError;
use crate::text::draw2d::Rect;
use crate::ui::types::UiDrawCommand;

/// Offset and scale mapping 640x480 UI units onto drawable pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiTransform {
    /// Pixel x of UI origin.
    pub x: f32,
    /// Pixel y of UI origin.
    pub y: f32,
    /// Pixels per UI unit.
    pub scale: f32,
}

/// Fit UI space into a safe area at one scale, letterboxing the remainder.
pub fn fit_ui(area: &Rect, scale: f32) -> Result<UiTransform, ClientError> {
    if !scale.is_finite() || scale <= 0.0 || area.width <= 0.0 || area.height <= 0.0 {
        return Err(ClientError::BadUi(
            "UI requires a positive scale and safe area".to_string(),
        ));
    }
    let factor = (area.width / 640.0).min(area.height / 480.0) * scale;
    Ok(UiTransform {
        x: area.x + (area.width - 640.0 * factor) / 2.0,
        y: area.y + (area.height - 480.0 * factor) / 2.0,
        scale: factor,
    })
}

/// Map one drawable-pixel point into UI units.
#[must_use]
pub fn ui_point(point: Vec2, transform: &UiTransform) -> Vec2 {
    Vec2 {
        x: (point.x - transform.x) / transform.scale,
        y: (point.y - transform.y) / transform.scale,
    }
}

/// Whether a point falls inside a rect (right and bottom edges exclusive).
#[must_use]
pub fn contains(rect: &Rect, point: Vec2) -> bool {
    point.x >= rect.x && point.y >= rect.y && point.x < rect.x + rect.width && point.y < rect.y + rect.height
}

/// Intersect two rects, clamping empty results to zero area.
#[must_use]
pub fn intersect(a: &Rect, b: &Rect) -> Rect {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    Rect {
        x,
        y,
        width: ((a.x + a.width).min(b.x + b.width) - x).max(0.0),
        height: ((a.y + a.height).min(b.y + b.height) - y).max(0.0),
    }
}

/// Map one draw command from UI units into drawable pixels.
#[must_use]
pub fn transform_ui(command: &UiDrawCommand, transform: &UiTransform) -> UiDrawCommand {
    let rect = |value: &Rect| Rect {
        x: transform.x + value.x * transform.scale,
        y: transform.y + value.y * transform.scale,
        width: value.width * transform.scale,
        height: value.height * transform.scale,
    };
    match command {
        UiDrawCommand::Fill { rect: value, color } => UiDrawCommand::Fill {
            rect: rect(value),
            color: *color,
        },
        UiDrawCommand::Image {
            rect: value,
            resource,
            tex_coords,
            color,
        } => UiDrawCommand::Image {
            rect: rect(value),
            resource: resource.clone(),
            tex_coords: *tex_coords,
            color: *color,
        },
        UiDrawCommand::Clip { rect: value } => UiDrawCommand::Clip {
            rect: value.as_ref().map(rect),
        },
        UiDrawCommand::Text {
            origin,
            text,
            font,
            scale,
            color,
            align,
            shadow,
        } => UiDrawCommand::Text {
            origin: Vec2 {
                x: transform.x + origin.x * transform.scale,
                y: transform.y + origin.y * transform.scale,
            },
            text: text.clone(),
            font: font.clone(),
            scale: scale * transform.scale,
            color: *color,
            align: *align,
            shadow: *shadow,
        },
    }
}

/// `menu_row` overrides; every field falls back to its donor default.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MenuRowOptions {
    /// Left edge; default 64.
    pub x: Option<f32>,
    /// First row top; default 92.
    pub y: Option<f32>,
    /// Row width; default 512.
    pub width: Option<f32>,
    /// Row height; default 28.
    pub height: Option<f32>,
}

/// Rect for one row of a vertical menu list.
#[must_use]
pub fn menu_row(index: i32, options: &MenuRowOptions) -> Rect {
    let height = options.height.unwrap_or(28.0);
    Rect {
        x: options.x.unwrap_or(64.0),
        y: options.y.unwrap_or(92.0) + (index as f32) * height,
        width: options.width.unwrap_or(512.0),
        height,
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec2, vec4};

    use super::*;

    #[test]
    fn fit_ui_matches_donor_letterbox() {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        assert_eq!(
            fit_ui(&area, 1.0).unwrap(),
            UiTransform {
                x: 0.0,
                y: 0.0,
                scale: 1.0
            }
        );
        let wide = Rect {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 720.0,
        };
        assert_eq!(
            fit_ui(&wide, 1.0).unwrap(),
            UiTransform {
                x: 160.0,
                y: 0.0,
                scale: 1.5
            }
        );
        let scaled = fit_ui(&area, 2.0).unwrap();
        assert_eq!(
            scaled,
            UiTransform {
                x: -320.0,
                y: -240.0,
                scale: 2.0
            }
        );
        assert!(fit_ui(&area, 0.0).is_err());
        assert!(fit_ui(&area, f32::NAN).is_err());
        assert!(fit_ui(
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 480.0
            },
            1.0
        )
        .is_err());
    }

    #[test]
    fn ui_point_round_trips() {
        let transform = UiTransform {
            x: 160.0,
            y: 0.0,
            scale: 1.5,
        };
        assert_eq!(ui_point(vec2(160.0, 0.0), &transform), vec2(0.0, 0.0));
        assert_eq!(ui_point(vec2(1120.0, 720.0), &transform), vec2(640.0, 480.0));
    }

    #[test]
    fn contains_uses_exclusive_far_edges() {
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };
        assert!(contains(&rect, vec2(10.0, 20.0)));
        assert!(!contains(&rect, vec2(40.0, 20.0)));
        assert!(!contains(&rect, vec2(10.0, 60.0)));
        assert!(!contains(&rect, vec2(9.9, 20.0)));
    }

    #[test]
    fn intersect_clamps_empty() {
        let a = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let b = Rect {
            x: 5.0,
            y: 5.0,
            width: 10.0,
            height: 10.0,
        };
        assert_eq!(
            intersect(&a, &b),
            Rect {
                x: 5.0,
                y: 5.0,
                width: 5.0,
                height: 5.0
            }
        );
        let far = Rect {
            x: 50.0,
            y: 50.0,
            width: 5.0,
            height: 5.0,
        };
        assert_eq!(
            intersect(&a, &far),
            Rect {
                x: 50.0,
                y: 50.0,
                width: 0.0,
                height: 0.0
            }
        );
    }

    #[test]
    fn transform_ui_maps_every_kind() {
        let transform = UiTransform {
            x: 160.0,
            y: 0.0,
            scale: 1.5,
        };
        let white = vec4(1.0, 1.0, 1.0, 1.0);
        let fill = UiDrawCommand::Fill {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
            color: white,
        };
        match transform_ui(&fill, &transform) {
            UiDrawCommand::Fill { rect, .. } => {
                assert_eq!(
                    rect,
                    Rect {
                        x: 160.0,
                        y: 0.0,
                        width: 960.0,
                        height: 720.0
                    }
                );
            }
            other => panic!("expected fill, got {other:?}"),
        }
        let text = UiDrawCommand::Text {
            origin: vec2(320.0, 48.0),
            text: "hi".to_string(),
            font: crate::ui::types::ResourceId::new("resource:test:font").unwrap(),
            scale: 2.0,
            color: white,
            align: crate::ui::types::TextAlign::Center,
            shadow: true,
        };
        match transform_ui(&text, &transform) {
            UiDrawCommand::Text { origin, scale, .. } => {
                assert_eq!(origin, vec2(640.0, 72.0));
                assert_eq!(scale, 3.0);
            }
            other => panic!("expected text, got {other:?}"),
        }
        let clip = UiDrawCommand::Clip { rect: None };
        assert_eq!(transform_ui(&clip, &transform), clip);
    }

    #[test]
    fn menu_row_defaults_and_overrides() {
        let defaults = MenuRowOptions::default();
        assert_eq!(
            menu_row(0, &defaults),
            Rect {
                x: 64.0,
                y: 92.0,
                width: 512.0,
                height: 28.0
            }
        );
        assert_eq!(
            menu_row(2, &defaults),
            Rect {
                x: 64.0,
                y: 148.0,
                width: 512.0,
                height: 28.0
            }
        );
        let custom = MenuRowOptions {
            x: Some(10.0),
            y: Some(20.0),
            width: Some(30.0),
            height: Some(40.0),
        };
        assert_eq!(
            menu_row(1, &custom),
            Rect {
                x: 10.0,
                y: 60.0,
                width: 30.0,
                height: 40.0
            }
        );
    }
}
