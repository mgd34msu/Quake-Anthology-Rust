//! Shared field-of-view preference with per-owner cvar binding.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/view-settings.ts`
//! (`ApplicationViewSettings`). A user preference supplies the ordinary view;
//! game cameras retain their own FOV. The donor holds a live registry
//! reference; here the registry travels as an explicit argument (bound or
//! not), which keeps the single-owner rule without shared borrowing. The
//! settings-row binding snapshots its label at build time (the donor label
//! is a live getter) and shares `Rc<RefCell>` handles following the
//! `qa_client` settings pattern. FOV validation reuses
//! [`super::shared_setting_cvars::validate_field_of_view`].

use std::cell::RefCell;
use std::rc::Rc;

use qa_client::ui::settings::SettingBinding;
use qa_client::ui::settings::SettingBindingKind;
use qa_client::ui::settings::SettingCategory;
use qa_client::ui::types::UiControlId;
use qa_core::cvar::flags;
use qa_core::cvar::CvarError;
use qa_core::cvar::CvarRegistry;
use thiserror::Error;

use super::shared_setting_cvars::validate_field_of_view;
use crate::settings::config::ConfigStore;
use crate::settings::json::parse_json;
use crate::settings::json::stringify;
use crate::settings::json::Json;

/// View settings failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ViewSettingsError {
    /// A cvar owner is already bound.
    #[error("View settings already have a cvar owner")]
    AlreadyBound,
    /// Field of view outside 60..=160.
    #[error("Field of view must be between 60 and 160 degrees")]
    BadFieldOfView,
    /// Stored preferences are invalid.
    #[error("Invalid view preferences")]
    BadPreferences,
    /// Settings-row id was rejected.
    #[error("View settings id is invalid: {0}")]
    BadId(String),
    /// Settings store failure.
    #[error("{0}")]
    Store(String),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// Default field of view in degrees.
pub const DEFAULT_FIELD_OF_VIEW: f64 = 90.0;
/// Minimum field of view in degrees.
pub const MIN_FIELD_OF_VIEW: f64 = 60.0;
/// Maximum field of view in degrees.
pub const MAX_FIELD_OF_VIEW: f64 = 160.0;
/// Settings-row id.
pub const VIEW_FIELD_OF_VIEW_ID: &str = "ui:view:field-of-view";
/// Stored preferences file.
pub const VIEW_PREFERENCES_FILE: &str = "view.json";

/// `fov` documentation summary.
pub const FOV_DOC_SUMMARY: &str =
    "Shared field-of-view preference in degrees. Game zoom and special cameras retain control.";
/// `fov` usage line.
pub const FOV_DOC_USAGE: &str = "fov [degrees]";
/// `fov` examples.
pub const FOV_DOC_EXAMPLES: [&str; 2] = ["fov 120", "set fov 90"];
/// `fov` allowed values.
pub const FOV_ALLOWED_VALUES: [&str; 1] = ["60 through 160"];

/// Shared field-of-view preference.
pub struct ApplicationViewSettings {
    selected: Option<f64>,
    bound: bool,
    explicit: bool,
    changed: Box<dyn FnMut(f64)>,
}

impl ApplicationViewSettings {
    /// Create preferences reporting changes through `changed`.
    pub fn new(changed: impl FnMut(f64) + 'static) -> Self {
        Self {
            selected: None,
            bound: false,
            explicit: false,
            changed: Box::new(changed),
        }
    }

    /// Current field of view (live cvar when bound, else the selection or 90).
    #[must_use]
    pub fn field_of_view(&self, cvars: Option<&CvarRegistry>) -> f64 {
        match (self.bound, cvars) {
            (true, Some(cvars)) => cvars.variable_string("fov").parse().unwrap_or(f64::NAN),
            _ => self.selected.unwrap_or(DEFAULT_FIELD_OF_VIEW),
        }
    }

    /// Explicit override, when the user picked a value.
    #[must_use]
    pub fn override_value(&self, cvars: Option<&CvarRegistry>) -> Option<f64> {
        self.explicit.then(|| self.field_of_view(cvars))
    }

    /// Whether a cvar owner is bound.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.bound
    }

    /// Bind a cvar owner, adopting or publishing the selection.
    pub fn bind_cvars(&mut self, cvars: &mut CvarRegistry) -> Result<(), ViewSettingsError> {
        if self.bound {
            return Err(ViewSettingsError::AlreadyBound);
        }
        let existing = cvars.get("fov");
        let stored = existing
            .as_ref()
            .is_some_and(|entry| entry.value != "90" && validate_field_of_view(&entry.value).is_none());
        if !cvars.dialect().is_q1()
            || existing.is_none()
            || existing.as_ref().is_some_and(|entry| entry.reset_value != "90")
        {
            cvars.register("fov", "90", flags::ARCHIVE)?;
        }
        if validate_field_of_view(&cvars.variable_string("fov")).is_some() {
            cvars.set("fov", "90", true)?;
        }
        if !stored {
            if let Some(selected) = self.selected {
                cvars.set("fov", &selected.to_string(), false)?;
            }
        }
        self.bound = true;
        self.selected = None;
        let current: f64 = cvars.variable_string("fov").parse().unwrap_or(f64::NAN);
        self.explicit |= stored && current != DEFAULT_FIELD_OF_VIEW;
        Ok(())
    }

    /// Release the cvar owner, retaining the live value as the selection.
    pub fn unbind_cvars(&mut self, cvars: &CvarRegistry) {
        if !self.bound {
            return;
        }
        self.selected = Some(self.field_of_view(Some(cvars)));
        self.bound = false;
    }

    /// Fold an external `fov` change (donor `bindValue` hook).
    pub fn note_cvar_changed(&mut self, text: &str) -> Result<(), ViewSettingsError> {
        if validate_field_of_view(text).is_some() {
            return Err(ViewSettingsError::BadFieldOfView);
        }
        self.explicit = true;
        (self.changed)(text.parse().unwrap_or(f64::NAN));
        Ok(())
    }

    /// Set the field of view, publishing to the bound owner when present.
    pub fn set_field_of_view(&mut self, cvars: Option<&mut CvarRegistry>, value: f64) -> Result<(), ViewSettingsError> {
        if !value.is_finite() || value < MIN_FIELD_OF_VIEW || value > MAX_FIELD_OF_VIEW {
            return Err(ViewSettingsError::BadFieldOfView);
        }
        self.explicit = true;
        match (self.bound, cvars) {
            (true, Some(cvars)) => {
                cvars.set("fov", &value.to_string(), false)?;
                (self.changed)(value);
                Ok(())
            }
            _ => {
                self.selected = Some(value);
                (self.changed)(value);
                Ok(())
            }
        }
    }

    /// Settings-row binding over shared handles.
    pub fn binding(
        settings: &Rc<RefCell<Self>>,
        cvars: &Rc<RefCell<CvarRegistry>>,
    ) -> Result<SettingBinding, ViewSettingsError> {
        let id =
            UiControlId::new(VIEW_FIELD_OF_VIEW_ID).map_err(|error| ViewSettingsError::BadId(error.to_string()))?;
        let label = format!(
            "Field of view ({})",
            settings.borrow().field_of_view(Some(&cvars.borrow()))
        );
        let read_settings = Rc::clone(settings);
        let read_cvars = Rc::clone(cvars);
        let write_settings = Rc::clone(settings);
        let write_cvars = Rc::clone(cvars);
        Ok(SettingBinding {
            id,
            label,
            category: SettingCategory::Display,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Slider {
                read: Rc::new(move || {
                    #[allow(clippy::cast_possible_truncation)]
                    let value = read_settings.borrow().field_of_view(Some(&read_cvars.borrow())) as f32;
                    value
                }),
                write: Rc::new(move |value| {
                    write_settings
                        .borrow_mut()
                        .set_field_of_view(Some(&mut write_cvars.borrow_mut()), f64::from(value))
                        .expect("slider clamps to 60..=160");
                }),
                minimum: MIN_FIELD_OF_VIEW as f32,
                maximum: MAX_FIELD_OF_VIEW as f32,
                step: 5.0,
                format_value: None,
            },
        })
    }

    /// Load stored preferences, keeping current values when absent.
    pub fn load(&mut self, store: &ConfigStore, cvars: Option<&mut CvarRegistry>) -> Result<(), ViewSettingsError> {
        let Some(text) = store
            .load_text(VIEW_PREFERENCES_FILE)
            .map_err(|error| ViewSettingsError::Store(error.to_string()))?
        else {
            return Ok(());
        };
        let value = parse_json(&text).map_err(|_| ViewSettingsError::BadPreferences)?;
        let (version, field_of_view) = match &value {
            Json::Object(_) => (value.get("version"), value.get("fieldOfView")),
            _ => (None, None),
        };
        let (Some(Json::Number(version)), Some(Json::Number(field_of_view))) = (version, field_of_view) else {
            return Err(ViewSettingsError::BadPreferences);
        };
        if *version != 1.0 {
            return Err(ViewSettingsError::BadPreferences);
        }
        self.set_field_of_view(cvars, *field_of_view)
    }

    /// Save explicit preferences.
    pub fn save(&self, store: &ConfigStore, cvars: Option<&CvarRegistry>) -> Result<(), ViewSettingsError> {
        if !self.explicit {
            return Ok(());
        }
        let mut text = stringify(&Json::Object(vec![
            ("version".to_string(), Json::Number(1.0)),
            ("fieldOfView".to_string(), Json::Number(self.field_of_view(cvars))),
        ]));
        text.push('\n');
        store
            .dump(VIEW_PREFERENCES_FILE, &text)
            .map_err(|error| ViewSettingsError::Store(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use qa_core::cmd::Dialect;

    use super::*;

    fn make_settings(calls: &Rc<RefCell<Vec<f64>>>) -> ApplicationViewSettings {
        let calls = Rc::clone(calls);
        ApplicationViewSettings::new(move |value| calls.borrow_mut().push(value))
    }

    #[test]
    fn unbound_selection_reports_and_fires() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut settings = make_settings(&calls);
        assert_eq!(settings.field_of_view(None), 90.0);
        assert_eq!(settings.override_value(None), None);
        settings.set_field_of_view(None, 120.0).unwrap();
        assert_eq!(settings.field_of_view(None), 120.0);
        assert_eq!(settings.override_value(None), Some(120.0));
        assert_eq!(*calls.borrow(), vec![120.0]);
        assert_eq!(
            settings.set_field_of_view(None, 200.0),
            Err(ViewSettingsError::BadFieldOfView)
        );
    }

    #[test]
    fn bind_adopts_stored_and_publishes_selection() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut settings = make_settings(&calls);
        settings.set_field_of_view(None, 110.0).unwrap();
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        settings.bind_cvars(&mut cvars).unwrap();
        assert!(settings.is_bound());
        assert_eq!(cvars.variable_string("fov"), "110");
        assert_eq!(settings.field_of_view(Some(&cvars)), 110.0);
        assert_eq!(settings.bind_cvars(&mut cvars), Err(ViewSettingsError::AlreadyBound));

        let mut stored = CvarRegistry::new(Dialect::Q3);
        stored.register("fov", "100", flags::ARCHIVE).unwrap();
        let mut settings = make_settings(&calls);
        settings.bind_cvars(&mut stored).unwrap();
        assert_eq!(settings.field_of_view(Some(&stored)), 100.0);
        assert_eq!(settings.override_value(Some(&stored)), Some(100.0));
        settings.unbind_cvars(&stored);
        assert!(!settings.is_bound());
        assert_eq!(settings.field_of_view(None), 100.0);
    }

    #[test]
    fn bind_repairs_invalid_stored_value() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars.register("fov", "abc", flags::ARCHIVE).unwrap();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut settings = make_settings(&calls);
        settings.bind_cvars(&mut cvars).unwrap();
        assert_eq!(cvars.variable_string("fov"), "90");
        settings.note_cvar_changed("120").unwrap();
        assert_eq!(*calls.borrow(), vec![120.0]);
        assert_eq!(
            settings.note_cvar_changed("wide"),
            Err(ViewSettingsError::BadFieldOfView)
        );
    }

    #[test]
    fn binding_row_round_trips_slider() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let settings = Rc::new(RefCell::new(make_settings(&calls)));
        let cvars = Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3)));
        settings.borrow_mut().bind_cvars(&mut cvars.borrow_mut()).unwrap();
        let binding = ApplicationViewSettings::binding(&settings, &cvars).unwrap();
        assert_eq!(binding.label, "Field of view (90)");
        assert!(matches!(binding.category, SettingCategory::Display));
        let SettingBindingKind::Slider {
            read,
            write,
            minimum,
            maximum,
            step,
            ..
        } = &binding.kind
        else {
            panic!("expected a slider");
        };
        assert_eq!((*minimum, *maximum, *step), (60.0, 160.0, 5.0));
        assert_eq!(read(), 90.0);
        write(100.0);
        assert_eq!(read(), 100.0);
        assert_eq!(cvars.borrow().variable_string("fov"), "100");
    }

    #[test]
    fn load_and_save_round_trip() {
        let root = std::env::temp_dir().join(format!("view-settings-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let store = ConfigStore::new(root.clone());
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut settings = make_settings(&calls);
        settings.load(&store, None).unwrap();
        store
            .dump(VIEW_PREFERENCES_FILE, "{\"version\":1,\"fieldOfView\":120}")
            .unwrap();
        settings.load(&store, None).unwrap();
        assert_eq!(settings.field_of_view(None), 120.0);
        settings.save(&store, None).unwrap();
        let text = store.load_text(VIEW_PREFERENCES_FILE).unwrap().unwrap();
        assert!(text.ends_with('\n'));
        let value = parse_json(&text).unwrap();
        assert_eq!(value.get("fieldOfView"), Some(&Json::Number(120.0)));
        store.dump(VIEW_PREFERENCES_FILE, "{\"version\":2}").unwrap();
        assert_eq!(settings.load(&store, None), Err(ViewSettingsError::BadPreferences));
        std::fs::remove_dir_all(&root).ok();
    }
}
