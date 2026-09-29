//! High-contrast and color-mode palette remapping.
//!
//! Donor provenance: `src/ui/common/accessibility.ts` (`accessibleColors`).

use qa_core::math::vec4;

use crate::ui::common::skin::UiSkinColors;
use crate::ui::types::{InterfaceColorMode, UiAppearance};

/// Remap one skin palette for the current appearance.
#[must_use]
pub fn accessible_colors(colors: &UiSkinColors, appearance: &UiAppearance) -> UiSkinColors {
    let white = vec4(1.0, 1.0, 1.0, 1.0);
    let black = vec4(0.0, 0.0, 0.0, 1.0);
    let mut base = colors.clone();
    if appearance.high_contrast {
        base.text = white;
        base.disabled = vec4(0.65, 0.65, 0.65, 1.0);
        base.accent = vec4(1.0, 1.0, 0.0, 1.0);
        base.panel = black;
        base.control = black;
        base.focused = vec4(0.2, 0.2, 0.2, 1.0);
    }
    match appearance.color_mode {
        InterfaceColorMode::Monochrome => {
            base.text = white;
            base.accent = white;
            base.focused = vec4(0.25, 0.25, 0.25, 1.0);
        }
        InterfaceColorMode::BlueYellow => {
            base.accent = vec4(1.0, 0.9, 0.2, 1.0);
            base.focused = vec4(0.06, 0.2, 0.42, 1.0);
        }
        InterfaceColorMode::Standard => {}
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::common::skin::default_ui_skin;
    use crate::ui::types::ResourceId;

    fn colors() -> UiSkinColors {
        default_ui_skin(&ResourceId::new("resource:test:font").unwrap()).colors
    }

    #[test]
    fn standard_appearance_passes_through() {
        let base = colors();
        assert_eq!(accessible_colors(&base, &UiAppearance::default()), base);
    }

    #[test]
    fn high_contrast_matches_donor() {
        let appearance = UiAppearance { high_contrast: true, ..UiAppearance::default() };
        let remapped = accessible_colors(&colors(), &appearance);
        assert_eq!(remapped.text, vec4(1.0, 1.0, 1.0, 1.0));
        assert_eq!(remapped.disabled, vec4(0.65, 0.65, 0.65, 1.0));
        assert_eq!(remapped.accent, vec4(1.0, 1.0, 0.0, 1.0));
        assert_eq!(remapped.panel, vec4(0.0, 0.0, 0.0, 1.0));
        assert_eq!(remapped.control, vec4(0.0, 0.0, 0.0, 1.0));
        assert_eq!(remapped.focused, vec4(0.2, 0.2, 0.2, 1.0));
    }

    #[test]
    fn color_modes_remap_accent_and_focus() {
        let mono = UiAppearance { color_mode: InterfaceColorMode::Monochrome, ..UiAppearance::default() };
        let remapped = accessible_colors(&colors(), &mono);
        assert_eq!(remapped.text, vec4(1.0, 1.0, 1.0, 1.0));
        assert_eq!(remapped.accent, vec4(1.0, 1.0, 1.0, 1.0));
        assert_eq!(remapped.focused, vec4(0.25, 0.25, 0.25, 1.0));
        let blue_yellow = UiAppearance { color_mode: InterfaceColorMode::BlueYellow, ..UiAppearance::default() };
        let remapped = accessible_colors(&colors(), &blue_yellow);
        assert_eq!(remapped.accent, vec4(1.0, 0.9, 0.2, 1.0));
        assert_eq!(remapped.focused, vec4(0.06, 0.2, 0.42, 1.0));
    }

    #[test]
    fn high_contrast_combines_with_monochrome() {
        let appearance = UiAppearance {
            high_contrast: true,
            color_mode: InterfaceColorMode::Monochrome,
            ..UiAppearance::default()
        };
        let remapped = accessible_colors(&colors(), &appearance);
        assert_eq!(remapped.accent, vec4(1.0, 1.0, 1.0, 1.0));
        assert_eq!(remapped.focused, vec4(0.25, 0.25, 0.25, 1.0));
        assert_eq!(remapped.panel, vec4(0.0, 0.0, 0.0, 1.0));
    }
}
