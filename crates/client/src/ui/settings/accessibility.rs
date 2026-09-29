//! Per-seat accessibility preferences.
//!
//! Ported from donor `src/ui/settings/accessibility.ts`: archived
//! `ui_seat{1..4}_<key>` cvars for scales, toggles, typeface, and colors.

use std::rc::Rc;

use super::language::register_language_settings;
use super::{CvarValidator, RegisterCvars, SettingCvars};
use crate::ui::types::{InterfaceColorMode, Typeface, UiPreferenceValues, DEFAULT_UI_PREFERENCES};

/// Seats with preference cvars.
const SEAT_COUNT: u32 = 4;
/// Toggle validation message.
const TOGGLE_ERROR: &str = "Expected 0 or 1";

/// Cvar name for a seat index (zero-based) and donor camelCase key.
fn preference_name(seat: u32, key: &str) -> String {
    format!("ui_seat{}_{}", seat + 1, key)
}

/// Donor typeface name.
fn typeface_name(value: Typeface) -> &'static str {
    match value {
        Typeface::Standard => "standard",
        Typeface::Bold => "bold",
    }
}

/// Donor color-mode name.
fn color_mode_name(value: InterfaceColorMode) -> &'static str {
    match value {
        InterfaceColorMode::Standard => "standard",
        InterfaceColorMode::BlueYellow => "blue-yellow",
        InterfaceColorMode::Monochrome => "monochrome",
    }
}

/// Parse a typeface, falling back to standard on unknown values.
fn parse_typeface(value: &str) -> Typeface {
    if value == "bold" {
        Typeface::Bold
    } else {
        Typeface::Standard
    }
}

/// Parse a color mode, falling back to standard on unknown values.
fn parse_color_mode(value: &str) -> InterfaceColorMode {
    match value {
        "blue-yellow" => InterfaceColorMode::BlueYellow,
        "monochrome" => InterfaceColorMode::Monochrome,
        _ => InterfaceColorMode::Standard,
    }
}

/// Numeric range validator matching donor finite/range checks.
fn range_validator(low: f32, high: f32) -> CvarValidator {
    Rc::new(move |value: &str| match value.parse::<f32>() {
        Ok(parsed) if parsed.is_finite() && parsed >= low && parsed <= high => None,
        _ => Some(format!("Expected {low} through {high}")),
    })
}

/// Toggle validator matching donor `0`/`1` checks.
fn toggle_validator() -> CvarValidator {
    Rc::new(|value: &str| {
        if value == "0" || value == "1" {
            None
        } else {
            Some(TOGGLE_ERROR.to_string())
        }
    })
}

/// Choice validator with a donor-style `Expected a, b` message.
fn choice_validator(choices: &[&str]) -> CvarValidator {
    let owned: Vec<String> = choices.iter().map(|choice| (*choice).to_string()).collect();
    Rc::new(move |value: &str| {
        if owned.iter().any(|choice| choice == value) {
            None
        } else {
            Some(format!("Expected {}", owned.join(", ")))
        }
    })
}

/// Register archived preference cvars for all seats.
pub fn register_accessibility_settings(cvars: &mut dyn RegisterCvars) {
    register_language_settings(cvars);
    let defaults = DEFAULT_UI_PREFERENCES;
    let numerics = [
        ("hudScale", defaults.hud_scale, 0.5_f32, 1.5_f32),
        ("textScale", defaults.text_scale, 0.75_f32, 2.0_f32),
        ("menuScale", defaults.menu_scale, 0.75_f32, 1.0_f32),
        ("crosshairSize", defaults.crosshair_size, 2.0_f32, 32.0_f32),
    ];
    let toggles = [
        ("highContrast", defaults.high_contrast),
        ("reducedFlashes", defaults.reduced_flashes),
        ("captions", defaults.captions),
        ("crosshair", defaults.crosshair),
    ];
    for seat in 0..SEAT_COUNT {
        for (key, default, low, high) in numerics {
            let name = preference_name(seat, key);
            cvars.register(&name, &default.to_string(), true);
            cvars.bind_validator(&name, range_validator(low, high));
        }
        for (key, default) in toggles {
            let name = preference_name(seat, key);
            cvars.register(&name, if default { "1" } else { "0" }, true);
            cvars.bind_validator(&name, toggle_validator());
        }
        let typeface = preference_name(seat, "typeface");
        cvars.register(&typeface, typeface_name(defaults.typeface), true);
        cvars.bind_validator(&typeface, choice_validator(&["standard", "bold"]));
        let color_mode = preference_name(seat, "colorMode");
        cvars.register(&color_mode, color_mode_name(defaults.color_mode), true);
        cvars.bind_validator(
            &color_mode,
            choice_validator(&["standard", "blue-yellow", "monochrome"]),
        );
    }
}

/// Read a seat's preferences, falling back to defaults when cvars are missing.
pub fn read_ui_preferences(cvars: &dyn SettingCvars, seat: u32) -> UiPreferenceValues {
    let defaults = DEFAULT_UI_PREFERENCES;
    let number = |key: &str, fallback: f32| -> f32 {
        cvars
            .find(&preference_name(seat, key))
            .and_then(|entry| entry.value.parse::<f32>().ok())
            .filter(|parsed| parsed.is_finite())
            .unwrap_or(fallback)
    };
    let flag = |key: &str, fallback: bool| -> bool {
        cvars
            .find(&preference_name(seat, key))
            .map(|entry| entry.value == "1")
            .unwrap_or(fallback)
    };
    let typeface = cvars
        .find(&preference_name(seat, "typeface"))
        .map(|entry| parse_typeface(&entry.value))
        .unwrap_or(defaults.typeface);
    let color_mode = cvars
        .find(&preference_name(seat, "colorMode"))
        .map(|entry| parse_color_mode(&entry.value))
        .unwrap_or(defaults.color_mode);
    UiPreferenceValues {
        hud_scale: number("hudScale", defaults.hud_scale),
        text_scale: number("textScale", defaults.text_scale),
        menu_scale: number("menuScale", defaults.menu_scale),
        crosshair_size: number("crosshairSize", defaults.crosshair_size),
        high_contrast: flag("highContrast", defaults.high_contrast),
        reduced_flashes: flag("reducedFlashes", defaults.reduced_flashes),
        captions: flag("captions", defaults.captions),
        crosshair: flag("crosshair", defaults.crosshair),
        typeface,
        color_mode,
    }
}

/// Write a seat's preferences.
pub fn write_ui_preferences(cvars: &dyn SettingCvars, seat: u32, values: &UiPreferenceValues) {
    cvars.set(&preference_name(seat, "hudScale"), &values.hud_scale.to_string());
    cvars.set(&preference_name(seat, "textScale"), &values.text_scale.to_string());
    cvars.set(&preference_name(seat, "menuScale"), &values.menu_scale.to_string());
    cvars.set(
        &preference_name(seat, "crosshairSize"),
        &values.crosshair_size.to_string(),
    );
    cvars.set(
        &preference_name(seat, "highContrast"),
        if values.high_contrast { "1" } else { "0" },
    );
    cvars.set(
        &preference_name(seat, "reducedFlashes"),
        if values.reduced_flashes { "1" } else { "0" },
    );
    cvars.set(
        &preference_name(seat, "captions"),
        if values.captions { "1" } else { "0" },
    );
    cvars.set(
        &preference_name(seat, "crosshair"),
        if values.crosshair { "1" } else { "0" },
    );
    cvars.set(&preference_name(seat, "typeface"), typeface_name(values.typeface));
    cvars.set(&preference_name(seat, "colorMode"), color_mode_name(values.color_mode));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::super::CvarView;
    use crate::ui::types::CommandDialect;

    struct FakeCvars {
        values: RefCell<HashMap<String, String>>,
        validators: HashMap<String, CvarValidator>,
    }

    impl FakeCvars {
        fn new() -> Self {
            Self {
                values: RefCell::new(HashMap::new()),
                validators: HashMap::new(),
            }
        }

        fn stored(&self, name: &str) -> Option<String> {
            self.values.borrow().get(name).cloned()
        }

        fn force(&self, name: &str, value: &str) {
            self.values.borrow_mut().insert(name.to_string(), value.to_string());
        }
    }

    impl SettingCvars for FakeCvars {
        fn dialect(&self) -> CommandDialect {
            CommandDialect::Q3
        }

        fn find(&self, name: &str) -> Option<CvarView> {
            self.stored(name).map(|value| CvarView {
                value: value.clone(),
                latched_value: None,
                reset_value: value,
                flags: 0,
            })
        }

        fn set(&self, name: &str, value: &str) {
            if let Some(validator) = self.validators.get(name) {
                if validator(value).is_some() {
                    return;
                }
            }
            self.force(name, value);
        }

        fn variable_value(&self, name: &str) -> f32 {
            self.stored(name)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|parsed| parsed.is_finite())
                .unwrap_or(0.0)
        }
    }

    impl RegisterCvars for FakeCvars {
        fn register(&mut self, name: &str, value: &str, _archive: bool) {
            self.force(name, value);
        }

        fn bind_validator(&mut self, name: &str, validator: CvarValidator) {
            self.validators.insert(name.to_string(), validator);
        }
    }

    fn keys() -> Vec<&'static str> {
        vec![
            "hudScale",
            "textScale",
            "menuScale",
            "crosshairSize",
            "highContrast",
            "reducedFlashes",
            "captions",
            "crosshair",
            "typeface",
            "colorMode",
        ]
    }

    #[test]
    fn preference_names_cover_four_seats() {
        for seat in 0..4 {
            for key in keys() {
                assert_eq!(preference_name(seat, key), format!("ui_seat{}_{}", seat + 1, key));
            }
        }
        assert_eq!(preference_name(0, "hudScale"), "ui_seat1_hudScale");
        assert_eq!(preference_name(3, "colorMode"), "ui_seat4_colorMode");
    }

    #[test]
    fn validators_match_donor_ranges_and_choices() {
        let hud = range_validator(0.5, 1.5);
        assert_eq!(hud("1"), None);
        assert_eq!(hud("0.5"), None);
        assert_eq!(hud("1.5"), None);
        assert_eq!(hud("0.49"), Some("Expected 0.5 through 1.5".to_string()));
        assert_eq!(hud("1.51"), Some("Expected 0.5 through 1.5".to_string()));
        assert_eq!(hud("bogus"), Some("Expected 0.5 through 1.5".to_string()));
        assert_eq!(hud(""), Some("Expected 0.5 through 1.5".to_string()));
        let crosshair = range_validator(2.0, 32.0);
        assert_eq!(crosshair("8"), None);
        assert_eq!(crosshair("99"), Some("Expected 2 through 32".to_string()));
        let toggle = toggle_validator();
        assert_eq!(toggle("0"), None);
        assert_eq!(toggle("1"), None);
        assert_eq!(toggle("2"), Some(TOGGLE_ERROR.to_string()));
        let typeface = choice_validator(&["standard", "bold"]);
        assert_eq!(typeface("bold"), None);
        assert_eq!(typeface("italic"), Some("Expected standard, bold".to_string()));
        let colors = choice_validator(&["standard", "blue-yellow", "monochrome"]);
        assert_eq!(colors("monochrome"), None);
        assert_eq!(
            colors("red"),
            Some("Expected standard, blue-yellow, monochrome".to_string())
        );
    }

    #[test]
    fn register_seeds_defaults_and_language() {
        let mut cvars = FakeCvars::new();
        register_accessibility_settings(&mut cvars);
        assert_eq!(cvars.stored("ui_seat1_hudScale").as_deref(), Some("1"));
        assert_eq!(cvars.stored("ui_seat1_crosshairSize").as_deref(), Some("8"));
        assert_eq!(cvars.stored("ui_seat1_captions").as_deref(), Some("1"));
        assert_eq!(cvars.stored("ui_seat1_highContrast").as_deref(), Some("0"));
        assert_eq!(cvars.stored("ui_seat4_colorMode").as_deref(), Some("standard"));
        assert_eq!(cvars.stored("ui_seat1_language").as_deref(), Some("english"));
        cvars.set("ui_seat1_textScale", "99");
        assert_eq!(cvars.stored("ui_seat1_textScale").as_deref(), Some("1"));
    }

    #[test]
    fn preferences_round_trip_per_seat() {
        let mut cvars = FakeCvars::new();
        register_accessibility_settings(&mut cvars);
        let custom = UiPreferenceValues {
            hud_scale: 1.5,
            text_scale: 2.0,
            menu_scale: 0.8,
            crosshair_size: 12.0,
            high_contrast: true,
            reduced_flashes: true,
            captions: false,
            crosshair: false,
            typeface: Typeface::Bold,
            color_mode: InterfaceColorMode::Monochrome,
        };
        write_ui_preferences(&cvars, 0, &custom);
        assert_eq!(read_ui_preferences(&cvars, 0), custom);
        assert_eq!(read_ui_preferences(&cvars, 1), DEFAULT_UI_PREFERENCES);
        write_ui_preferences(&cvars, 0, &DEFAULT_UI_PREFERENCES);
        assert_eq!(read_ui_preferences(&cvars, 0), DEFAULT_UI_PREFERENCES);
    }

    #[test]
    fn reads_fall_back_on_missing_or_garbled_values() {
        let empty = FakeCvars::new();
        assert_eq!(read_ui_preferences(&empty, 0), DEFAULT_UI_PREFERENCES);
        let mut cvars = FakeCvars::new();
        register_accessibility_settings(&mut cvars);
        cvars.force("ui_seat1_hudScale", "bogus");
        cvars.force("ui_seat1_highContrast", "2");
        cvars.force("ui_seat1_typeface", "italic");
        cvars.force("ui_seat1_colorMode", "red");
        let read = read_ui_preferences(&cvars, 0);
        assert_eq!(read.hud_scale, DEFAULT_UI_PREFERENCES.hud_scale);
        assert!(!read.high_contrast);
        assert_eq!(read.typeface, Typeface::Standard);
        assert_eq!(read.color_mode, InterfaceColorMode::Standard);
        cvars.force("ui_seat1_textScale", "99");
        assert_eq!(read_ui_preferences(&cvars, 0).text_scale, 99.0);
    }
}
