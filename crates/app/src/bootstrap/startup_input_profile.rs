//! First-seat input profile shared by the frontend and gameplay.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/startup-input-profile.ts`
//! (`StartupInputProfile`). The front end edits the same first-seat profile
//! that gameplay loads. Sync port: the donor's async store calls become the
//! sync [`ConfigStore`] methods. The first-seat input (unported `SeatInput`;
//! gamepad tuning lives in [`GamepadTuning`](crate::settings::config::GamepadTuning))
//! arrives through [`StartupSeatInput`]; default bindings reuse `qa_client`,
//! seat documents reuse `crate::settings`, and the frontend-preference fold
//! ([`frontend_preferences`](super::frontend_preferences)) arrives as a
//! caller-supplied closure over [`FrontendPreferenceOverrides`].

use std::collections::BTreeMap;

use qa_client::input::bindings::default_bindings;
use qa_client::input::default_mouse_tuning;
use qa_client::input::weapons::WeaponBindingItem;
use qa_client::input::InputBinding;
use qa_client::input::PhysicalInput;
use qa_core::cmd::Dialect;
use thiserror::Error;

use crate::settings::config::ConfigStore;
use crate::settings::config::ControllerSelection;
use crate::settings::config::GamepadTuning;
use crate::settings::config::MouseTuning;
use crate::settings::config::SeatSettings;
use crate::settings::json::Json;
use crate::settings::SettingsError;

/// Stored first-seat profile path.
pub const SEAT_PROFILE_PATH: &str = "input/seat-1.json";

/// Startup input profile failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StartupInputProfileError {
    /// Settings store failure.
    #[error("{0}")]
    Store(String),
}

impl From<SettingsError> for StartupInputProfileError {
    fn from(error: SettingsError) -> Self {
        Self::Store(error.to_string())
    }
}

/// Frontend preference overrides folded into the seat document.
pub type FrontendPreferenceOverrides = BTreeMap<String, Json>;

/// First-seat input edited by the frontend (absorbed `SeatInput` surface).
pub trait StartupSeatInput {
    /// Current bindings in order.
    fn bindings(&self) -> Vec<InputBinding>;
    /// Bind one input, replacing any previous binding.
    fn bind(&mut self, binding: InputBinding);
    /// Remove all bindings.
    fn unbind_all(&mut self);
    /// Current gamepad tuning.
    fn gamepad_tuning(&self) -> GamepadTuning;
    /// Replace gamepad tuning.
    fn set_gamepad_tuning(&mut self, tuning: GamepadTuning);
}

fn settings_mouse_tuning() -> MouseTuning {
    let tuning = default_mouse_tuning();
    MouseTuning {
        sensitivity: tuning.sensitivity,
        acceleration: tuning.acceleration,
        filter: tuning.filter,
        yaw: tuning.yaw,
        pitch: tuning.pitch,
        side: tuning.side,
        forward: tuning.forward,
        look_spring: tuning.look_spring,
        look_strafe: tuning.look_strafe,
        free_look: tuning.free_look,
        invert_pitch: tuning.invert_pitch,
    }
}

fn normalized_bindings(bindings: &[InputBinding]) -> Vec<InputBinding> {
    bindings
        .iter()
        .map(|binding| {
            let input = match &binding.input {
                PhysicalInput::ControllerButton { button, .. } => PhysicalInput::ControllerButton {
                    device: 0,
                    button: *button,
                },
                PhysicalInput::ControllerAxis { axis, direction, .. } => PhysicalInput::ControllerAxis {
                    device: 0,
                    axis: *axis,
                    direction: *direction,
                },
                other => other.clone(),
            };
            InputBinding {
                input,
                target: binding.target.clone(),
            }
        })
        .collect()
}

/// First-seat input profile with change baselines.
#[derive(Debug)]
pub struct StartupInputProfile<Input> {
    settings: ConfigStore,
    input: Input,
    baseline_bindings: Vec<InputBinding>,
    baseline_gamepad: GamepadTuning,
}

impl<Input: StartupSeatInput> StartupInputProfile<Input> {
    /// Retain live input, baselining its current state.
    #[must_use]
    pub fn retained(settings: ConfigStore, input: Input) -> Self {
        let baseline_bindings = input.bindings();
        let baseline_gamepad = input.gamepad_tuning();
        Self {
            settings,
            input,
            baseline_bindings,
            baseline_gamepad,
        }
    }

    /// Open the profile, loading the saved seat or dialect defaults.
    pub fn open(
        settings: ConfigStore,
        mut input: Input,
        dialect: Dialect,
        items: &[WeaponBindingItem],
    ) -> Result<Self, StartupInputProfileError> {
        let saved = settings.load_seat(SEAT_PROFILE_PATH)?;
        input.unbind_all();
        let bindings = saved
            .as_ref()
            .map(|saved| saved.bindings.clone())
            .unwrap_or_else(|| default_bindings(0, dialect, items));
        for binding in bindings {
            input.bind(binding);
        }
        if let Some(saved) = &saved {
            input.set_gamepad_tuning(saved.gamepad);
        }
        Ok(Self::retained(settings, input))
    }

    /// Borrow the live input.
    #[must_use]
    pub fn input(&self) -> &Input {
        &self.input
    }

    /// Mutably borrow the live input.
    pub fn input_mut(&mut self) -> &mut Input {
        &mut self.input
    }

    /// Save changed bindings, tuning, preferences, and history.
    pub fn save(
        &mut self,
        values: &FrontendPreferenceOverrides,
        history: Option<&[String]>,
        apply_preferences: &mut dyn FnMut(&FrontendPreferenceOverrides, SeatSettings) -> SeatSettings,
    ) -> Result<(), StartupInputProfileError> {
        let bindings = normalized_bindings(&self.input.bindings());
        let current = self.input.bindings();
        let bindings_changed = current != self.baseline_bindings;
        let gamepad_current = self.input.gamepad_tuning();
        let gamepad_changed = gamepad_current != self.baseline_gamepad;
        if !bindings_changed && !gamepad_changed && values.is_empty() && history.is_none() {
            return Ok(());
        }
        let saved = self.settings.load_seat(SEAT_PROFILE_PATH)?;
        let baseline = saved.unwrap_or_else(|| SeatSettings {
            version: 1,
            bindings: bindings.clone(),
            gamepad: gamepad_current,
            mouse: settings_mouse_tuning(),
            always_run: None,
            history: Vec::new(),
            rumble: true,
            rumble_strength: 1.0,
            controller: ControllerSelection::Automatic,
        });
        let mut merged = baseline.clone();
        if bindings_changed {
            merged.bindings = bindings;
        }
        if gamepad_changed {
            merged.gamepad = gamepad_current;
        }
        let preferences = apply_preferences(values, merged);
        let mut selected = preferences;
        if let Some(history) = history {
            selected.history = history.to_vec();
        }
        if bindings_changed || gamepad_changed || selected != baseline {
            self.settings.save_seat(SEAT_PROFILE_PATH, &selected)?;
        }
        self.baseline_bindings = current;
        self.baseline_gamepad = gamepad_current;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use qa_client::input::InputAction;

    use super::*;

    #[derive(Debug)]
    struct FakeInput {
        bindings: Vec<InputBinding>,
        gamepad: GamepadTuning,
    }

    fn gamepad() -> GamepadTuning {
        use crate::settings::config::StickCurve;
        let curve = StickCurve::Radial {
            deadzone: 0.1,
            exponent: 1.0,
            outer_threshold: 0.05,
        };
        GamepadTuning {
            move_curve: curve,
            look_curve: curve,
            swap_sticks: false,
            yaw_degrees_per_second: 180.0,
            pitch_degrees_per_second: 180.0,
            invert_pitch: false,
            forward_sensitivity: 1.0,
            side_sensitivity: 1.0,
            trigger_threshold: 0.5,
            gyro: crate::settings::config::GyroTuning {
                enabled: false,
                yaw_axis: crate::settings::config::GyroYawAxis::Y,
                yaw_sensitivity: 1.0,
                pitch_sensitivity: 1.0,
            },
        }
    }

    impl StartupSeatInput for FakeInput {
        fn bindings(&self) -> Vec<InputBinding> {
            self.bindings.clone()
        }

        fn bind(&mut self, binding: InputBinding) {
            self.bindings.push(binding);
        }

        fn unbind_all(&mut self) {
            self.bindings.clear();
        }

        fn gamepad_tuning(&self) -> GamepadTuning {
            self.gamepad
        }

        fn set_gamepad_tuning(&mut self, tuning: GamepadTuning) {
            self.gamepad = tuning;
        }
    }

    fn store(name: &str) -> ConfigStore {
        let root = std::env::temp_dir().join(format!("qa-startup-input-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        ConfigStore::new(root)
    }

    fn binding() -> InputBinding {
        InputBinding {
            input: PhysicalInput::Key(32),
            target: qa_client::input::InputBindingTarget::Action(InputAction::Jump),
        }
    }

    #[test]
    fn open_loads_defaults_when_absent() {
        let input = FakeInput {
            bindings: vec![binding()],
            gamepad: gamepad(),
        };
        let mut profile = StartupInputProfile::open(store("open"), input, Dialect::Q3, &[]).unwrap();
        assert!(!profile.input().bindings().is_empty());
        profile
            .save(&FrontendPreferenceOverrides::new(), None, &mut |_, baseline| baseline)
            .unwrap();
    }

    #[test]
    fn retained_baselines_live_state() {
        let input = FakeInput {
            bindings: vec![binding()],
            gamepad: gamepad(),
        };
        let store = store("retained");
        let mut profile = StartupInputProfile::retained(store, input);
        profile
            .save(&FrontendPreferenceOverrides::new(), None, &mut |_, baseline| baseline)
            .unwrap();
        profile.input_mut().bind(binding());
        profile
            .save(&FrontendPreferenceOverrides::new(), None, &mut |_, baseline| baseline)
            .unwrap();
        let saved = profile.settings.load_seat(SEAT_PROFILE_PATH).unwrap().unwrap();
        assert_eq!(saved.bindings.len(), 2);
        assert_eq!(saved.version, 1);
        assert!(saved.rumble);
    }

    #[test]
    fn save_normalizes_controller_devices_and_history() {
        let controller = InputBinding {
            input: PhysicalInput::ControllerButton { device: 2, button: 0 },
            target: qa_client::input::InputBindingTarget::Action(InputAction::Jump),
        };
        let input = FakeInput {
            bindings: Vec::new(),
            gamepad: gamepad(),
        };
        let mut profile = StartupInputProfile::retained(store("normalize"), input);
        profile.input_mut().bind(controller);
        profile
            .save(
                &FrontendPreferenceOverrides::new(),
                Some(&["say hi".to_string()]),
                &mut |_, baseline| baseline,
            )
            .unwrap();
        let saved = profile.settings.load_seat(SEAT_PROFILE_PATH).unwrap().unwrap();
        assert!(matches!(
            saved.bindings[0].input,
            PhysicalInput::ControllerButton { device: 0, .. }
        ));
        assert_eq!(saved.history, vec!["say hi".to_string()]);
    }

    #[test]
    fn save_applies_preferences_and_gamepad() {
        let input = FakeInput {
            bindings: Vec::new(),
            gamepad: gamepad(),
        };
        let mut profile = StartupInputProfile::retained(store("prefs"), input);
        let mut tuned = gamepad();
        tuned.invert_pitch = true;
        profile.input_mut().set_gamepad_tuning(tuned);
        let mut values = FrontendPreferenceOverrides::new();
        values.insert("rumble".to_string(), Json::Bool(false));
        profile
            .save(&values, None, &mut |values, mut baseline| {
                if values.contains_key("rumble") {
                    baseline.rumble = false;
                }
                baseline
            })
            .unwrap();
        let saved = profile.settings.load_seat(SEAT_PROFILE_PATH).unwrap().unwrap();
        assert!(saved.gamepad.invert_pitch);
        assert!(!saved.rumble);
    }
}
