//! Per-seat language preferences.
//!
//! Ported from donor `src/ui/settings/language.ts`: archived
//! `ui_seat{1..4}_language` cvars holding lowercase language names.

use std::rc::Rc;

use super::{CvarValidator, RegisterCvars, SettingCvars};
use crate::ClientError;

/// Seats with language cvars.
const SEAT_COUNT: u32 = 4;
/// Fallback language when a seat cvar is missing.
const DEFAULT_LANGUAGE: &str = "english";
/// Validation message.
const LANGUAGE_ERROR: &str = "Expected a lowercase language name";

/// Cvar name for a seat index (zero-based).
fn language_name(seat: u32) -> String {
    format!("ui_seat{}_language", seat + 1)
}

/// Whether a value matches donor `/^[a-z]+$/`.
fn is_valid_language(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_lowercase())
}

/// Register archived language cvars for all seats.
pub fn register_language_settings(cvars: &mut dyn RegisterCvars) {
    for seat in 0..SEAT_COUNT {
        let name = language_name(seat);
        cvars.register(&name, DEFAULT_LANGUAGE, true);
        let validator: CvarValidator = Rc::new(|value| {
            if is_valid_language(value) {
                None
            } else {
                Some(LANGUAGE_ERROR.to_string())
            }
        });
        cvars.bind_validator(&name, validator);
    }
}

/// Read a seat language, falling back to English when the cvar is missing.
pub fn read_seat_language(cvars: &dyn SettingCvars, seat: u32) -> String {
    cvars
        .find(&language_name(seat))
        .map(|entry| entry.value)
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string())
}

/// Write a seat language, rejecting names that are not lowercase ASCII.
pub fn write_seat_language(cvars: &dyn SettingCvars, seat: u32, language: &str) -> Result<(), ClientError> {
    if !is_valid_language(language) {
        return Err(ClientError::BadUi(LANGUAGE_ERROR.to_string()));
    }
    cvars.set(&language_name(seat), language);
    Ok(())
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
        archive: HashMap<String, bool>,
        validators: HashMap<String, CvarValidator>,
    }

    impl FakeCvars {
        fn new() -> Self {
            Self {
                values: RefCell::new(HashMap::new()),
                archive: HashMap::new(),
                validators: HashMap::new(),
            }
        }

        fn snapshot(&self, name: &str) -> Option<CvarView> {
            self.values.borrow().get(name).map(|value| CvarView {
                value: value.clone(),
                latched_value: None,
                reset_value: value.clone(),
                flags: 0,
            })
        }

        fn stored(&self, name: &str) -> Option<String> {
            self.values.borrow().get(name).cloned()
        }
    }

    impl SettingCvars for FakeCvars {
        fn dialect(&self) -> CommandDialect {
            CommandDialect::Q3
        }

        fn find(&self, name: &str) -> Option<CvarView> {
            self.snapshot(name)
        }

        fn set(&self, name: &str, value: &str) {
            if let Some(validator) = self.validators.get(name) {
                if validator(value).is_some() {
                    return;
                }
            }
            self.values.borrow_mut().insert(name.to_string(), value.to_string());
        }

        fn variable_value(&self, name: &str) -> f32 {
            self.stored(name)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|parsed| parsed.is_finite())
                .unwrap_or(0.0)
        }
    }

    impl RegisterCvars for FakeCvars {
        fn register(&mut self, name: &str, value: &str, archive: bool) {
            self.values.borrow_mut().insert(name.to_string(), value.to_string());
            self.archive.insert(name.to_string(), archive);
        }

        fn bind_validator(&mut self, name: &str, validator: CvarValidator) {
            self.validators.insert(name.to_string(), validator);
        }
    }

    #[test]
    fn language_names_cover_four_seats() {
        let names: Vec<String> = (0..4).map(language_name).collect();
        assert_eq!(
            names,
            vec![
                "ui_seat1_language".to_string(),
                "ui_seat2_language".to_string(),
                "ui_seat3_language".to_string(),
                "ui_seat4_language".to_string(),
            ]
        );
    }

    #[test]
    fn language_validation_matches_lowercase_names() {
        for valid in ["english", "french", "german", "a"] {
            assert!(is_valid_language(valid), "expected valid: {valid}");
        }
        for invalid in ["", "English", "ENGLISH", "en-US", "en_us", "e2", "../bad", "a b", "0"] {
            assert!(!is_valid_language(invalid), "expected invalid: {invalid}");
        }
    }

    #[test]
    fn register_creates_archived_cvars_with_validators() {
        let mut cvars = FakeCvars::new();
        register_language_settings(&mut cvars);
        for seat in 0..4 {
            let name = language_name(seat);
            assert_eq!(cvars.stored(&name).as_deref(), Some("english"));
            assert_eq!(cvars.archive.get(&name), Some(&true));
            let validator = cvars.validators.get(&name).expect("validator bound");
            assert_eq!(validator("french"), None);
            assert_eq!(validator("../bad"), Some(LANGUAGE_ERROR.to_string()));
        }
    }

    #[test]
    fn language_round_trip_keeps_seats_independent() {
        let mut cvars = FakeCvars::new();
        register_language_settings(&mut cvars);
        write_seat_language(&cvars, 0, "french").expect("valid write");
        assert_eq!(read_seat_language(&cvars, 0), "french");
        assert_eq!(read_seat_language(&cvars, 1), "english");
        cvars.set("ui_seat1_language", "german");
        assert_eq!(read_seat_language(&cvars, 0), "german");
        cvars.set("ui_seat1_language", "../bad");
        assert_eq!(read_seat_language(&cvars, 0), "german");
    }

    #[test]
    fn write_rejects_non_lowercase_and_missing_falls_back() {
        let mut cvars = FakeCvars::new();
        register_language_settings(&mut cvars);
        for invalid in ["", "English", "en-US", "../bad", "e2"] {
            let result = write_seat_language(&cvars, 0, invalid);
            assert!(result.is_err(), "expected error for {invalid}");
        }
        assert_eq!(read_seat_language(&cvars, 0), "english");
        let empty = FakeCvars::new();
        assert_eq!(read_seat_language(&empty, 2), "english");
    }
}
