//! Common UI: skins, layout, native controller, legacy runtime.

pub mod accessibility;
pub mod art_manifest;
pub mod assets;
pub mod captions;
pub mod controller;
pub mod draw;
pub mod layout;
pub mod legacy;
pub mod menu_theme;
pub mod skin;

pub use assets::{load_native_ui_art, NativeUiArt};
pub use controller::{headless_options, NativeUiController, NativeUiOptions, UiMenuFactory, UiSound};
pub use draw::{render_ui_commands, UiEmitCommand, UiMaterialDraw, UiRenderServices};
pub use layout::{contains, fit_ui, intersect, menu_row, transform_ui, ui_point, MenuRowOptions, UiTransform};
pub use skin::{
    default_ui_skin, hud_skin_font, nine_slice, ui_skin_font, UiBorder, UiImageSlice, UiSkin, UiSkinColors,
};

#[cfg(test)]
mod tests {
    use qa_core::math::Vec2;

    use super::*;
    use crate::text::draw2d::Rect;
    use crate::ui::types::ResourceId;

    #[test]
    fn reexported_layout_and_skin_helpers_agree() {
        let row = menu_row(2, &MenuRowOptions::default());
        assert_eq!(
            row,
            Rect {
                x: 64.0,
                y: 148.0,
                width: 512.0,
                height: 28.0
            }
        );
        let font = ResourceId::new("resource:test:font").expect("font");
        let skin = default_ui_skin(&font);
        assert_eq!(skin.font.as_str(), "resource:test:font");
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 960.0,
        };
        let fit = fit_ui(&area, 1.0).expect("fit");
        assert!((fit.scale - 2.0).abs() < f32::EPSILON);
        assert!(contains(&area, Vec2 { x: 64.0, y: 64.0 }));
    }
}
