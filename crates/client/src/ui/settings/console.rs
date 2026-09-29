//! Console settings bindings.
//!
//! Ported from donor `src/ui/settings/console.ts`: the `con_scale`
//! accessibility choice (Auto plus 1x through 4x).

use std::rc::Rc;

use super::{bind_cvar_setting, CvarSettingKind, CvarSettingSpec, SettingBinding, SettingCategory, SettingCvars};
use crate::ui::types::UiChoice;
use crate::ClientError;

/// Console size choices.
fn console_choices() -> Vec<UiChoice> {
    vec![
        UiChoice {
            id: "0".to_string(),
            label: "Auto".to_string(),
        },
        UiChoice {
            id: "1".to_string(),
            label: "1x".to_string(),
        },
        UiChoice {
            id: "2".to_string(),
            label: "2x".to_string(),
        },
        UiChoice {
            id: "3".to_string(),
            label: "3x".to_string(),
        },
        UiChoice {
            id: "4".to_string(),
            label: "4x".to_string(),
        },
    ]
}

/// Bind console settings; requires `con_scale` to already be registered.
pub fn bind_console_settings(cvars: &Rc<dyn SettingCvars>) -> Result<Vec<SettingBinding>, ClientError> {
    let spec = CvarSettingSpec {
        name: "con_scale".to_string(),
        label: "Console text size".to_string(),
        category: SettingCategory::Accessibility,
        restart: None,
        kind: CvarSettingKind::Choice {
            choices: console_choices(),
        },
    };
    Ok(vec![bind_cvar_setting(cvars, spec, None)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::super::{CvarView, SettingBindingKind};
    use crate::ui::types::CommandDialect;

    struct FakeCvars {
        values: RefCell<HashMap<String, String>>,
    }

    impl FakeCvars {
        fn new() -> Self {
            Self {
                values: RefCell::new(HashMap::new()),
            }
        }

        fn with_scale(value: &str) -> Self {
            let fake = Self::new();
            fake.values
                .borrow_mut()
                .insert("con_scale".to_string(), value.to_string());
            fake
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
            self.stored(name).map(|value| CvarView {
                value: value.clone(),
                latched_value: None,
                reset_value: value,
                flags: 0,
            })
        }

        fn set(&self, name: &str, value: &str) {
            self.values.borrow_mut().insert(name.to_string(), value.to_string());
        }

        fn variable_value(&self, name: &str) -> f32 {
            self.stored(name)
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|parsed| parsed.is_finite())
                .unwrap_or(0.0)
        }
    }

    #[test]
    fn console_choices_match_donor_table() {
        let choices = console_choices();
        let ids: Vec<&str> = choices.iter().map(|choice| choice.id.as_str()).collect();
        let labels: Vec<&str> = choices.iter().map(|choice| choice.label.as_str()).collect();
        assert_eq!(ids, vec!["0", "1", "2", "3", "4"]);
        assert_eq!(labels, vec!["Auto", "1x", "2x", "3x", "4x"]);
    }

    #[test]
    fn bind_returns_single_accessibility_choice() {
        let cvars: Rc<dyn SettingCvars> = Rc::new(FakeCvars::with_scale("0"));
        let bindings = bind_console_settings(&cvars).expect("con_scale has an owner");
        assert_eq!(bindings.len(), 1);
        let binding = &bindings[0];
        assert_eq!(binding.label, "Console text size");
        assert_eq!(binding.category, SettingCategory::Accessibility);
        assert_eq!(binding.id.as_str(), "ui:settings:con_scale");
        assert!((binding.enabled)());
        match &binding.kind {
            SettingBindingKind::Choice { read, write, choices } => {
                let listed = choices();
                assert_eq!(listed.len(), 5);
                assert_eq!(listed[0].id, "0");
                assert_eq!(listed[0].label, "Auto");
                assert_eq!(read(), "0");
                write("3");
                assert_eq!(read(), "3");
            }
            _ => panic!("expected a choice binding"),
        }
    }

    #[test]
    fn bind_fails_without_owner() {
        let cvars: Rc<dyn SettingCvars> = Rc::new(FakeCvars::new());
        assert!(bind_console_settings(&cvars).is_err());
    }
}
