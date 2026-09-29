//! Native menu skins and nine-slice image panels.
//!
//! Donor provenance: `src/ui/common/skin.ts` in full (skin colors, font
//! scaling, cap-ink capture, nine-slice subdivision). Layout math uses `f32`;
//! font metrics come from [`crate::text::atlas`].

use qa_core::math::{Vec2, Vec4};

use crate::error::ClientError;
use crate::text::atlas::{CapInk, TextFontSelection};
use crate::text::draw2d::Rect;
use crate::ui::types::{ResourceId, UiDrawCommand};

/// Nine-slice border insets in atlas-region pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiBorder {
    /// Left inset.
    pub l: f32,
    /// Top inset.
    pub t: f32,
    /// Right inset.
    pub r: f32,
    /// Bottom inset.
    pub b: f32,
}

/// One nine-sliceable menu image.
#[derive(Debug, Clone, PartialEq)]
pub struct UiImageSlice {
    /// Image resource.
    pub resource: ResourceId,
    /// Atlas region width in pixels.
    pub width: f32,
    /// Atlas region height in pixels.
    pub height: f32,
    /// Atlas region corners.
    pub uv: [Vec2; 2],
    /// Border insets in atlas-region pixels.
    pub border: UiBorder,
    /// Border scale override; destinations scale insets down independently.
    pub border_scale: Option<f32>,
}

/// Menu palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiSkinColors {
    /// Body text.
    pub text: Vec4,
    /// Disabled text and controls.
    pub disabled: Vec4,
    /// Titles, focused text, and highlights.
    pub accent: Vec4,
    /// Menu panel fill.
    pub panel: Vec4,
    /// Unfocused control fill.
    pub control: Vec4,
    /// Focused control and selection fill.
    pub focused: Vec4,
}

/// Native menu skin: fonts, palette, and panel art.
#[derive(Debug, Clone, PartialEq)]
pub struct UiSkin {
    /// Body font resource.
    pub font: ResourceId,
    /// Title font resource override.
    pub title_font: Option<ResourceId>,
    /// Title scale override.
    pub title_scale: Option<f32>,
    /// Body text scale; every atlas normalizes to an eight-unit text line.
    pub font_scale: f32,
    /// Cap-ink box for caption and HUD text.
    pub cap_ink: Option<CapInk>,
    /// Default list row height in UI units.
    pub line_height: f32,
    /// Menu panel slice.
    pub panel: Option<UiImageSlice>,
    /// Unfocused control slice.
    pub button: Option<UiImageSlice>,
    /// Focused control slice.
    pub focus: Option<UiImageSlice>,
    /// Full-screen menu background resource.
    pub background: Option<ResourceId>,
    /// Palette.
    pub colors: UiSkinColors,
}

/// Default skin for one body font.
#[must_use]
pub fn default_ui_skin(font: &ResourceId) -> UiSkin {
    UiSkin {
        font: font.clone(),
        title_font: None,
        title_scale: None,
        font_scale: 1.5,
        cap_ink: None,
        line_height: 14.0,
        panel: None,
        button: None,
        focus: None,
        background: None,
        colors: UiSkinColors {
            text: Vec4 { x: 0.92, y: 0.88, z: 0.78, w: 1.0 },
            disabled: Vec4 { x: 0.4, y: 0.4, z: 0.4, w: 1.0 },
            accent: Vec4 { x: 1.0, y: 0.65, z: 0.22, w: 1.0 },
            panel: Vec4 { x: 0.055, y: 0.06, z: 0.065, w: 0.94 },
            control: Vec4 { x: 0.12, y: 0.13, z: 0.14, w: 0.9 },
            focused: Vec4 { x: 0.3, y: 0.19, z: 0.09, w: 0.98 },
        },
    }
}

/// Rescale a skin for one menu font selection and line height.
pub fn ui_skin_font(skin: &UiSkin, font: &TextFontSelection, line_height: f32) -> Result<UiSkin, ClientError> {
    let height = match font {
        TextFontSelection::Atlas { font, .. } => font.line_height,
        TextFontSelection::Classic { classic, .. } => classic.line_height,
    };
    if height == 0 || line_height <= 0.0 {
        return Err(ClientError::BadUi("Menu font line height must be positive".to_string()));
    }
    let mut scaled = skin.clone();
    scaled.font_scale = line_height / 8.0;
    scaled.line_height = line_height;
    Ok(scaled)
}

/// Capture a skin's cap-ink box from one HUD font selection.
#[must_use]
pub fn hud_skin_font(skin: &UiSkin, font: &TextFontSelection) -> UiSkin {
    let atlas = match font {
        TextFontSelection::Atlas { font, .. } => font,
        TextFontSelection::Classic { classic, .. } => classic,
    };
    let mut scaled = skin.clone();
    scaled.cap_ink = Some(atlas.cap_ink.unwrap_or(CapInk { top: 0.0, height: 8.0 }));
    scaled
}

/// Subdivide one slice into image commands over a destination rect.
///
/// Insets are pixels in the atlas region, independently scaled down for small
/// destinations. Degenerate cells are skipped.
pub fn nine_slice(slice: &UiImageSlice, rect: &Rect, color: Vec4) -> Result<Vec<UiDrawCommand>, ClientError> {
    let (left, top, right, bottom) = (slice.border.l, slice.border.t, slice.border.r, slice.border.b);
    if slice.width <= 0.0
        || slice.height <= 0.0
        || left < 0.0
        || top < 0.0
        || right < 0.0
        || bottom < 0.0
        || left + right > slice.width
        || top + bottom > slice.height
    {
        return Err(ClientError::BadUi("Invalid UI nine-slice image".to_string()));
    }
    let border_scale = slice.border_scale.unwrap_or(1.0);
    let sx = border_scale.min(rect.width / (left + right).max(1.0));
    let sy = border_scale.min(rect.height / (top + bottom).max(1.0));
    let xs = [rect.x, rect.x + left * sx, rect.x + rect.width - right * sx, rect.x + rect.width];
    let ys = [rect.y, rect.y + top * sy, rect.y + rect.height - bottom * sy, rect.y + rect.height];
    let (uv0, uv1) = (slice.uv[0], slice.uv[1]);
    let (du, dv) = (uv1.x - uv0.x, uv1.y - uv0.y);
    let us = [uv0.x, uv0.x + du * left / slice.width, uv1.x - du * right / slice.width, uv1.x];
    let vs = [uv0.y, uv0.y + dv * top / slice.height, uv1.y - dv * bottom / slice.height, uv1.y];
    let mut commands = Vec::with_capacity(9);
    for y in 0..3 {
        for x in 0..3 {
            let (x0, x1, y0, y1) = (xs[x], xs[x + 1], ys[y], ys[y + 1]);
            let (u0, u1, v0, v1) = (us[x], us[x + 1], vs[y], vs[y + 1]);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            commands.push(UiDrawCommand::Image {
                rect: Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 },
                resource: slice.resource.clone(),
                tex_coords: [Vec2 { x: u0, y: v0 }, Vec2 { x: u1, y: v1 }],
                color,
            });
        }
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> ResourceId {
        ResourceId::new("resource:test:font").unwrap()
    }

    fn slice() -> UiImageSlice {
        UiImageSlice {
            resource: ResourceId::new("resource:test:panel").unwrap(),
            width: 100.0,
            height: 60.0,
            uv: [Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 1.0, y: 1.0 }],
            border: UiBorder { l: 10.0, t: 10.0, r: 10.0, b: 10.0 },
            border_scale: None,
        }
    }

    #[test]
    fn default_skin_matches_donor() {
        let skin = default_ui_skin(&font());
        assert_eq!(skin.font_scale, 1.5);
        assert_eq!(skin.line_height, 14.0);
        assert_eq!(skin.colors.accent, Vec4 { x: 1.0, y: 0.65, z: 0.22, w: 1.0 });
        assert!(skin.panel.is_none() && skin.background.is_none());
    }

    #[test]
    fn skin_font_scales_line_height() {
        let atlas = crate::text::atlas::classic_charset(7, 128, 128, "test", false).unwrap();
        let font_selection = TextFontSelection::Classic { classic: atlas, unicode: None };
        let skin = ui_skin_font(&default_ui_skin(&font()), &font_selection, 24.0).unwrap();
        assert_eq!(skin.font_scale, 3.0);
        assert_eq!(skin.line_height, 24.0);
        assert!(ui_skin_font(&default_ui_skin(&font()), &font_selection, 0.0).is_err());
    }

    #[test]
    fn hud_font_captures_cap_ink() {
        let atlas = crate::text::atlas::classic_charset(7, 128, 128, "test", false).unwrap();
        let font_selection = TextFontSelection::Classic { classic: atlas, unicode: None };
        let skin = hud_skin_font(&default_ui_skin(&font()), &font_selection);
        assert_eq!(skin.cap_ink, Some(CapInk { top: 0.0, height: 8.0 }));
    }

    #[test]
    fn nine_slice_emits_exact_cells() {
        let rect = Rect { x: 0.0, y: 0.0, width: 100.0, height: 60.0 };
        let white = Vec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 };
        let commands = nine_slice(&slice(), &rect, white).unwrap();
        assert_eq!(commands.len(), 9);
        match &commands[0] {
            UiDrawCommand::Image { rect, tex_coords, .. } => {
                assert_eq!(*rect, Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 });
                assert_eq!(*tex_coords, [Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 0.1, y: 10.0 / 60.0 }]);
            }
            other => panic!("expected image, got {other:?}"),
        }
        match &commands[4] {
            UiDrawCommand::Image { rect, tex_coords, .. } => {
                assert_eq!(*rect, Rect { x: 10.0, y: 10.0, width: 80.0, height: 40.0 });
                assert_eq!(*tex_coords, [Vec2 { x: 0.1, y: 10.0 / 60.0 }, Vec2 { x: 0.9, y: 50.0 / 60.0 }]);
            }
            other => panic!("expected image, got {other:?}"),
        }
    }

    #[test]
    fn nine_slice_skips_degenerate_cells() {
        let rect = Rect { x: 0.0, y: 0.0, width: 20.0, height: 20.0 };
        let white = Vec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 };
        let commands = nine_slice(&slice(), &rect, white).unwrap();
        assert_eq!(commands.len(), 4);
    }

    #[test]
    fn nine_slice_rejects_bad_geometry() {
        let rect = Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
        let white = Vec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 };
        let mut bad = slice();
        bad.border.l = 60.0;
        bad.border.r = 60.0;
        assert!(nine_slice(&bad, &rect, white).is_err());
        let mut negative = slice();
        negative.border.t = -1.0;
        assert!(nine_slice(&negative, &rect, white).is_err());
    }
}
