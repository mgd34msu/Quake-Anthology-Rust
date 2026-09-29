//! Main-menu theme: title font, skin, backdrop, and panel commands.
//!
//! Donor provenance: `src/ui/common/menu-theme.ts` in full.

use qa_core::math::{vec4, Vec4};

use crate::text::draw2d::Rect;
use crate::ui::common::layout::{fit_ui, transform_ui};
use crate::ui::common::skin::{UiSkin, UiSkinColors};
use crate::ui::types::{ResourceId, UiDrawCommand, UiDrawContext};

/// Build one engine resource id from a namespaced literal.
fn engine_resource(id: &'static str) -> ResourceId {
    match ResourceId::new(id) {
        Ok(resource) => resource,
        Err(_) => unreachable!("engine resource id is namespaced: {id}"),
    }
}

/// Title font resource for main menus.
#[must_use]
pub fn menu_title_font() -> ResourceId {
    engine_resource("resource:engine-menu:title")
}

/// Main-menu skin for one body font.
#[must_use]
pub fn menu_skin(font: &ResourceId) -> UiSkin {
    UiSkin {
        font: font.clone(),
        title_font: Some(menu_title_font()),
        title_scale: Some(4.0),
        font_scale: 2.6,
        cap_ink: None,
        line_height: 24.0,
        panel: None,
        button: None,
        focus: None,
        background: None,
        colors: UiSkinColors {
            text: vec4(0.9, 0.92, 0.94, 1.0),
            disabled: vec4(0.48, 0.5, 0.53, 1.0),
            accent: vec4(1.0, 0.73, 0.35, 1.0),
            panel: vec4(0.0, 0.0, 0.0, 0.0),
            control: vec4(0.06, 0.08, 0.1, 0.83),
            focused: vec4(0.23, 0.16, 0.09, 0.97),
        },
    }
}

/// Full-viewport main-menu backdrop, center-cropping the art to the viewport.
#[must_use]
pub fn menu_backdrop(context: &UiDrawContext) -> UiDrawCommand {
    let viewport = &context.binding.viewport;
    let ratio = viewport.width / viewport.height;
    let art_ratio = 1672.0 / 941.0;
    let crop_x = if ratio < art_ratio { (1.0 - ratio / art_ratio) / 2.0 } else { 0.0 };
    let crop_y = if ratio > art_ratio { (1.0 - art_ratio / ratio) / 2.0 } else { 0.0 };
    UiDrawCommand::Image {
        rect: *viewport,
        resource: engine_resource("resource:engine-menu:main-background"),
        tex_coords: [
            qa_core::math::Vec2 { x: crop_x, y: crop_y },
            qa_core::math::Vec2 { x: 1.0 - crop_x, y: 1.0 - crop_y },
        ],
        color: Vec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 },
    }
}

/// Menu side panel mapped from UI units into the seat safe area.
///
/// A degenerate safe area yields the untransformed fill.
#[must_use]
pub fn menu_panel(context: &UiDrawContext, narrow: bool) -> UiDrawCommand {
    let command = UiDrawCommand::Fill {
        rect: Rect {
            x: 40.0,
            y: 28.0,
            width: if narrow { 264.0 } else { 560.0 },
            height: 420.0,
        },
        color: vec4(0.025, 0.035, 0.045, if narrow { 0.86 } else { 0.96 }),
    };
    match fit_ui(&context.binding.safe_area, 1.0) {
        Ok(transform) => transform_ui(&command, &transform),
        Err(_) => command,
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::*;
    use crate::ui::types::{
        ContentId, DopplerSelection, EnvironmentSelection, PresentationSelection, ProviderRef, SeatPresentationBinding,
    };

    fn context(viewport: Rect, safe_area: Rect) -> UiDrawContext {
        let authority = IdentityOwner::create("menu-theme-test").unwrap();
        UiDrawContext {
            binding: SeatPresentationBinding {
                seat: authority.seat(0),
                client: authority.client(0, 0),
                viewport,
                safe_area,
                hud_scale: 1.0,
                presentation: PresentationSelection {
                    doppler: DopplerSelection::Disabled,
                    environment: EnvironmentSelection::Disabled,
                    assets: ContentId::new("assets"),
                    hud: ProviderRef { provider: "hud".to_string(), content: ContentId::new("hud") },
                    effects: ProviderRef { provider: "fx".to_string(), content: ContentId::new("fx") },
                    audio: ProviderRef { provider: "audio".to_string(), content: ContentId::new("audio") },
                },
            },
            time_ms: 0,
        }
    }

    #[test]
    fn title_font_and_skin_match_donor() {
        assert_eq!(menu_title_font().as_str(), "resource:engine-menu:title");
        let skin = menu_skin(&ResourceId::new("resource:test:font").unwrap());
        assert_eq!(skin.title_font.as_ref().unwrap().as_str(), "resource:engine-menu:title");
        assert_eq!(skin.title_scale, Some(4.0));
        assert_eq!(skin.font_scale, 2.6);
        assert_eq!(skin.line_height, 24.0);
        assert_eq!(skin.colors.accent, vec4(1.0, 0.73, 0.35, 1.0));
    }

    #[test]
    fn backdrop_crops_to_viewport_ratio() {
        let viewport = Rect { x: 0.0, y: 0.0, width: 640.0, height: 480.0 };
        let command = menu_backdrop(&context(viewport, viewport));
        match command {
            UiDrawCommand::Image { rect, resource, tex_coords, .. } => {
                assert_eq!(rect, viewport);
                assert_eq!(resource.as_str(), "resource:engine-menu:main-background");
                let ratio = 640.0 / 480.0;
                let art = 1672.0 / 941.0;
                let crop = (1.0 - ratio / art) / 2.0;
                assert_eq!(tex_coords[0].x, crop);
                assert_eq!(tex_coords[0].y, 0.0);
                assert_eq!(tex_coords[1].x, 1.0 - crop);
                assert_eq!(tex_coords[1].y, 1.0);
            }
            other => panic!("expected image, got {other:?}"),
        }
    }

    #[test]
    fn panel_maps_into_safe_area() {
        let area = Rect { x: 0.0, y: 0.0, width: 640.0, height: 480.0 };
        let command = menu_panel(&context(area, area), false);
        match command {
            UiDrawCommand::Fill { rect, color } => {
                assert_eq!(rect, Rect { x: 40.0, y: 28.0, width: 560.0, height: 420.0 });
                assert_eq!(color, vec4(0.025, 0.035, 0.045, 0.96));
            }
            other => panic!("expected fill, got {other:?}"),
        }
        let narrow = menu_panel(&context(area, area), true);
        match narrow {
            UiDrawCommand::Fill { rect, color } => {
                assert_eq!(rect.width, 264.0);
                assert_eq!(color.w, 0.86);
            }
            other => panic!("expected fill, got {other:?}"),
        }
    }
}
