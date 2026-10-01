//! Frontend preference overrides ported from
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/frontend-preferences.ts`.
//!
//! Only user-selected overrides cross into a game; saved values stay with
//! their seat. [`FrontendPreferences`] holds the override set plus baselines
//! loaded from the config store, and builds the settings rows that edit the
//! overrides.
//!
//! Live game state stays behind caller traits: [`FrontendAudio`] covers the
//! unported `ApplicationAudio` (`audio.ts`) volumes, and
//! [`FrontendLocalInput`] covers one local seat's haptics, mouse tuning, and
//! run toggle (the donor `LocalInput` builder/haptics types are unported).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use qa_client::audio::output::{AudioOutputFormat as StoredAudioOutputFormat, DEFAULT_AUDIO_OUTPUT_FORMAT};
use qa_client::input::mouse_settings::{read_mouse_tuning, register_mouse_settings, write_mouse_tuning};
use qa_client::input::{default_mouse_tuning, MouseTuning as ClientMouseTuning};
use qa_client::ui::settings::{
    bind_audio_settings, bind_controller_vibration, bind_mouse_motion_settings, bind_primary_input_settings,
    AudioOutputSettings, AudioSettings, ControllerVibrationSettings, MouseMotionSettings, PrimaryInputSettings,
    SettingBinding, SettingsValueService,
};
use qa_core::cmd::Dialect;
use qa_core::cvar::{CvarError, CvarRegistry};
use thiserror::Error;

use super::audio::playlist_settings::MusicPreferences;
use super::audio_settings::{load_audio_settings, save_audio_settings, AudioSaveRequest, AudioSettingsError};
use crate::settings::config::{ConfigStore, MouseTuning as StoredMouseTuning, SeatSettings};
use crate::settings::SettingsError;

/// Default effects volume when neither overrides nor baseline set one.
pub const DEFAULT_EFFECTS_VOLUME: f64 = 0.7;
/// Default music volume when neither overrides nor baseline set one.
pub const DEFAULT_MUSIC_VOLUME: f64 = 0.25;
/// Seat document the frontend baselines load from.
pub const FRONTEND_BASELINE_SEAT: &str = "input/seat-1.json";

/// Input-side preference values (donor `FrontendInputValues`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrontendInputValues {
    /// Controller vibration enabled.
    pub controller_vibration: bool,
    /// Vibration strength in `[0, 1]`.
    pub controller_vibration_strength: f64,
    /// Mouse sensitivity.
    pub sensitivity: f64,
    /// Pitch scale.
    pub pitch: f64,
    /// Yaw scale.
    pub yaw: f64,
    /// Invert the mouse pitch axis.
    pub invert_mouse: bool,
    /// Mouse acceleration.
    pub acceleration: f64,
    /// Mouse smoothing filter.
    pub filter: bool,
    /// Look spring (Q1 only).
    pub look_spring: bool,
    /// Look strafe (Q1 only).
    pub look_strafe: bool,
    /// Free look.
    pub free_look: bool,
    /// Always run.
    pub always_run: bool,
}

/// Full preference values (donor `FrontendPreferenceValues`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrontendPreferenceValues {
    /// Effects volume in `[0, 1]`.
    pub effects_volume: f64,
    /// Music volume in `[0, 1]`.
    pub music_volume: f64,
    /// Input-side values.
    pub input: FrontendInputValues,
}

/// Input-side overrides (donor `Partial<FrontendInputValues>`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrontendInputOverrides {
    /// Controller vibration enabled.
    pub controller_vibration: Option<bool>,
    /// Vibration strength in `[0, 1]`.
    pub controller_vibration_strength: Option<f64>,
    /// Mouse sensitivity.
    pub sensitivity: Option<f64>,
    /// Pitch scale.
    pub pitch: Option<f64>,
    /// Yaw scale.
    pub yaw: Option<f64>,
    /// Invert the mouse pitch axis.
    pub invert_mouse: Option<bool>,
    /// Mouse acceleration.
    pub acceleration: Option<f64>,
    /// Mouse smoothing filter.
    pub filter: Option<bool>,
    /// Look spring (Q1 only).
    pub look_spring: Option<bool>,
    /// Look strafe (Q1 only).
    pub look_strafe: Option<bool>,
    /// Free look.
    pub free_look: Option<bool>,
    /// Always run.
    pub always_run: Option<bool>,
}

/// Full preference overrides (donor `FrontendPreferenceOverrides`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrontendPreferenceOverrides {
    /// Effects volume in `[0, 1]`.
    pub effects_volume: Option<f64>,
    /// Music volume in `[0, 1]`.
    pub music_volume: Option<f64>,
    /// Input-side overrides.
    pub input: FrontendInputOverrides,
}

impl FrontendPreferenceValues {
    /// Split off the input-side values.
    #[must_use]
    pub fn input_values(&self) -> FrontendInputValues {
        self.input
    }
}

impl FrontendPreferenceOverrides {
    /// Split off the input-side overrides.
    #[must_use]
    pub fn input_overrides(&self) -> FrontendInputOverrides {
        self.input
    }
}

/// Live audio volumes behind the unported `ApplicationAudio`.
pub trait FrontendAudio {
    /// Effects volume in `[0, 1]`.
    fn effects_volume(&self) -> f64;
    /// Set the effects volume.
    fn set_effects_volume(&mut self, volume: f64);
    /// Music volume in `[0, 1]`.
    fn music_volume(&self) -> f64;
    /// Set the music volume.
    fn set_music_volume(&mut self, volume: f64);
}

/// One local seat's haptics, mouse tuning, and run toggle behind the
/// unported donor `LocalInput` builder/haptics types.
pub trait FrontendLocalInput {
    /// Read the seat's input-side preference values.
    fn read_frontend_input(&self) -> FrontendInputValues;
    /// Apply input-side overrides to the seat.
    fn apply_frontend_input(&mut self, values: &FrontendInputOverrides);
}

/// Apply overrides to live audio plus every local seat (donor
/// `applyFrontendPreferences`).
pub fn apply_frontend_preferences<A: FrontendAudio, L: FrontendLocalInput>(
    values: &FrontendPreferenceOverrides,
    locals: &mut [L],
    audio: &mut A,
) {
    if let Some(volume) = values.effects_volume {
        audio.set_effects_volume(volume);
    }
    if let Some(volume) = values.music_volume {
        audio.set_music_volume(volume);
    }
    let input = values.input_overrides();
    for local in locals {
        apply_frontend_input(&input, local);
    }
}

/// Read one seat's input-side values (donor `readFrontendInput`).
#[must_use]
pub fn read_frontend_input<L: FrontendLocalInput>(local: &L) -> FrontendInputValues {
    local.read_frontend_input()
}

/// Apply input-side overrides to one seat (donor `applyFrontendInput`).
pub fn apply_frontend_input<L: FrontendLocalInput>(values: &FrontendInputOverrides, local: &mut L) {
    local.apply_frontend_input(values);
}

/// Merge overrides onto stored mouse tuning (donor `frontendMouseTuning`).
fn frontend_mouse_tuning(values: &FrontendInputOverrides, mouse: StoredMouseTuning) -> StoredMouseTuning {
    StoredMouseTuning {
        sensitivity: values.sensitivity.unwrap_or(mouse.sensitivity),
        pitch: values.pitch.unwrap_or(mouse.pitch),
        yaw: values.yaw.unwrap_or(mouse.yaw),
        acceleration: values.acceleration.unwrap_or(mouse.acceleration),
        filter: values.filter.unwrap_or(mouse.filter),
        look_spring: values.look_spring.unwrap_or(mouse.look_spring),
        look_strafe: values.look_strafe.unwrap_or(mouse.look_strafe),
        free_look: values.free_look.unwrap_or(mouse.free_look),
        invert_pitch: values.invert_mouse.unwrap_or(mouse.invert_pitch),
        ..mouse
    }
}

/// Merge overrides onto saved seat settings (donor `frontendSeatSettings`).
#[must_use]
pub fn frontend_seat_settings(values: &FrontendPreferenceOverrides, saved: &SeatSettings) -> SeatSettings {
    let mut next = saved.clone();
    next.mouse = frontend_mouse_tuning(&values.input_overrides(), saved.mouse);
    if let Some(always_run) = values.input.always_run {
        next.always_run = Some(always_run);
    }
    if let Some(vibration) = values.input.controller_vibration {
        next.rumble = vibration;
    }
    if let Some(strength) = values.input.controller_vibration_strength {
        next.rumble_strength = strength;
    }
    next
}

/// Read the first seat's full preference values, or [`None`] with no local
/// seats (donor `readFrontendPreferences`).
#[must_use]
pub fn read_frontend_preferences<A: FrontendAudio, L: FrontendLocalInput>(
    locals: &[L],
    audio: &A,
) -> Option<FrontendPreferenceValues> {
    let local = locals.first()?;
    Some(FrontendPreferenceValues {
        effects_volume: audio.effects_volume(),
        music_volume: audio.music_volume(),
        input: local.read_frontend_input(),
    })
}

/// Diff two value snapshots onto the selected overrides (donor
/// `changedFrontendPreferences`). Pitch and yaw compare with total ordering,
/// matching donor `Object.is` semantics for signed zero and NaN.
#[must_use]
pub fn changed_frontend_preferences(
    before: &FrontendPreferenceValues,
    after: &FrontendPreferenceValues,
    selected: &FrontendPreferenceOverrides,
) -> FrontendPreferenceOverrides {
    let mut out = *selected;
    if after.effects_volume != before.effects_volume {
        out.effects_volume = Some(after.effects_volume);
    }
    if after.music_volume != before.music_volume {
        out.music_volume = Some(after.music_volume);
    }
    let (b, a) = (&before.input, &after.input);
    let mut input = out.input;
    if a.controller_vibration_strength != b.controller_vibration_strength {
        input.controller_vibration_strength = Some(a.controller_vibration_strength);
    }
    if a.controller_vibration != b.controller_vibration {
        input.controller_vibration = Some(a.controller_vibration);
    }
    if a.sensitivity != b.sensitivity {
        input.sensitivity = Some(a.sensitivity);
    }
    if a.pitch.total_cmp(&b.pitch) != Ordering::Equal {
        input.pitch = Some(a.pitch);
    }
    if a.yaw.total_cmp(&b.yaw) != Ordering::Equal {
        input.yaw = Some(a.yaw);
    }
    if a.acceleration != b.acceleration {
        input.acceleration = Some(a.acceleration);
    }
    if a.filter != b.filter {
        input.filter = Some(a.filter);
    }
    if a.look_spring != b.look_spring {
        input.look_spring = Some(a.look_spring);
    }
    if a.look_strafe != b.look_strafe {
        input.look_strafe = Some(a.look_strafe);
    }
    if a.free_look != b.free_look {
        input.free_look = Some(a.free_look);
    }
    if a.invert_mouse != b.invert_mouse {
        input.invert_mouse = Some(a.invert_mouse);
    }
    if a.always_run != b.always_run {
        input.always_run = Some(a.always_run);
    }
    out.input = input;
    out
}

/// Baseline/saved-state failures.
#[derive(Debug, Error)]
pub enum FrontendPreferencesError {
    /// Audio settings store failure.
    #[error(transparent)]
    AudioSettings(#[from] AudioSettingsError),
    /// Config store failure.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// Mouse cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// Stored volume baselines (donor `audioBaseline`; only the volumes are
/// ever read back).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AudioBaseline {
    /// Saved effects volume.
    pub effects_volume: Option<f64>,
    /// Saved music volume.
    pub music_volume: Option<f64>,
}

/// Override plus baseline state shared with the settings rows.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FrontendState {
    /// User-selected overrides.
    values: FrontendPreferenceOverrides,
    /// Saved volume baselines.
    audio_baseline: AudioBaseline,
    /// Mouse tuning baseline.
    mouse_baseline: ClientMouseTuning,
    /// Saved always-run baseline.
    always_run_baseline: Option<bool>,
    /// Saved vibration baseline.
    vibration_baseline: ControllerVibrationSettings,
}

impl Default for FrontendState {
    fn default() -> Self {
        Self {
            values: FrontendPreferenceOverrides::default(),
            audio_baseline: AudioBaseline::default(),
            mouse_baseline: default_mouse_tuning(),
            always_run_baseline: None,
            vibration_baseline: ControllerVibrationSettings {
                controller_vibration: true,
                controller_vibration_strength: 1.0,
            },
        }
    }
}

/// Default always-run for a dialect (donor
/// `defaultViewInputTuning(dialect).alwaysRun`: Q1 families default off).
fn dialect_always_run(dialect: Dialect) -> bool {
    !matches!(dialect, Dialect::Q1Netquake | Dialect::Q1Quakeworld)
}

/// Value service bridging one settings row group to shared frontend state.
#[derive(Clone)]
struct FrontendService {
    /// Shared state.
    state: Rc<RefCell<FrontendState>>,
    /// Current command dialect.
    dialect: Rc<dyn Fn() -> Dialect>,
}

impl FrontendService {
    /// Current audio values: overrides, then baselines, then defaults.
    fn audio_values(state: &FrontendState) -> AudioSettings {
        AudioSettings {
            effects_volume: state
                .values
                .effects_volume
                .or(state.audio_baseline.effects_volume)
                .unwrap_or(DEFAULT_EFFECTS_VOLUME) as f32,
            music_volume: state
                .values
                .music_volume
                .or(state.audio_baseline.music_volume)
                .unwrap_or(DEFAULT_MUSIC_VOLUME) as f32,
        }
    }

    /// Current primary-input values.
    fn primary_values(&self, state: &FrontendState) -> PrimaryInputSettings {
        PrimaryInputSettings {
            sensitivity: state
                .values
                .input
                .sensitivity
                .unwrap_or(state.mouse_baseline.sensitivity) as f32,
            pitch: state.values.input.pitch.unwrap_or(state.mouse_baseline.pitch) as f32,
            yaw: state.values.input.yaw.unwrap_or(state.mouse_baseline.yaw) as f32,
            invert_mouse: state
                .values
                .input
                .invert_mouse
                .unwrap_or(state.mouse_baseline.invert_pitch),
            always_run: state
                .values
                .input
                .always_run
                .or(state.always_run_baseline)
                .unwrap_or_else(|| dialect_always_run((self.dialect)())),
        }
    }

    /// Current mouse-motion values.
    fn motion_values(state: &FrontendState) -> MouseMotionSettings {
        MouseMotionSettings {
            acceleration: state
                .values
                .input
                .acceleration
                .unwrap_or(state.mouse_baseline.acceleration) as f32,
            look_spring: state
                .values
                .input
                .look_spring
                .unwrap_or(state.mouse_baseline.look_spring),
            look_strafe: state
                .values
                .input
                .look_strafe
                .unwrap_or(state.mouse_baseline.look_strafe),
            filter: state.values.input.filter.unwrap_or(state.mouse_baseline.filter),
            free_look: state.values.input.free_look.unwrap_or(state.mouse_baseline.free_look),
        }
    }

    /// Current vibration values.
    fn vibration_values(state: &FrontendState) -> ControllerVibrationSettings {
        ControllerVibrationSettings {
            controller_vibration: state
                .values
                .input
                .controller_vibration
                .unwrap_or(state.vibration_baseline.controller_vibration),
            controller_vibration_strength: state
                .values
                .input
                .controller_vibration_strength
                .map(|strength| strength as f32)
                .unwrap_or(state.vibration_baseline.controller_vibration_strength),
        }
    }
}

impl SettingsValueService<AudioSettings> for FrontendService {
    fn read(&self) -> AudioSettings {
        Self::audio_values(&self.state.borrow())
    }

    fn write(&self, update: &dyn Fn(&mut AudioSettings)) {
        let mut current = self.read();
        update(&mut current);
        let mut state = self.state.borrow_mut();
        state.values.effects_volume = Some(f64::from(current.effects_volume));
        state.values.music_volume = Some(f64::from(current.music_volume));
    }
}

impl SettingsValueService<PrimaryInputSettings> for FrontendService {
    fn read(&self) -> PrimaryInputSettings {
        let state = self.state.borrow();
        self.primary_values(&state)
    }

    fn write(&self, update: &dyn Fn(&mut PrimaryInputSettings)) {
        let mut current = self.read();
        update(&mut current);
        let mut state = self.state.borrow_mut();
        state.values.input.sensitivity = Some(f64::from(current.sensitivity));
        state.values.input.pitch = Some(f64::from(current.pitch));
        state.values.input.yaw = Some(f64::from(current.yaw));
        state.values.input.invert_mouse = Some(current.invert_mouse);
        state.values.input.always_run = Some(current.always_run);
    }
}

impl SettingsValueService<MouseMotionSettings> for FrontendService {
    fn read(&self) -> MouseMotionSettings {
        Self::motion_values(&self.state.borrow())
    }

    fn write(&self, update: &dyn Fn(&mut MouseMotionSettings)) {
        let mut current = self.read();
        update(&mut current);
        let mut state = self.state.borrow_mut();
        state.values.input.acceleration = Some(f64::from(current.acceleration));
        state.values.input.look_spring = Some(current.look_spring);
        state.values.input.look_strafe = Some(current.look_strafe);
        state.values.input.filter = Some(current.filter);
        state.values.input.free_look = Some(current.free_look);
    }
}

impl SettingsValueService<ControllerVibrationSettings> for FrontendService {
    fn read(&self) -> ControllerVibrationSettings {
        Self::vibration_values(&self.state.borrow())
    }

    fn write(&self, update: &dyn Fn(&mut ControllerVibrationSettings)) {
        let mut current = self.read();
        update(&mut current);
        let mut state = self.state.borrow_mut();
        state.values.input.controller_vibration = Some(current.controller_vibration);
        state.values.input.controller_vibration_strength = Some(f64::from(current.controller_vibration_strength));
    }
}

/// UI output format to stored output format (identical fields, distinct types).
fn stored_format(format: &qa_client::ui::settings::AudioOutputFormat) -> StoredAudioOutputFormat {
    StoredAudioOutputFormat {
        sample_rate: format.sample_rate,
        channels: format.channels,
        sample_bits: format.sample_bits,
    }
}

/// Stored mouse tuning to client mouse tuning (identical fields).
fn client_tuning(mouse: StoredMouseTuning) -> ClientMouseTuning {
    ClientMouseTuning {
        sensitivity: mouse.sensitivity,
        acceleration: mouse.acceleration,
        filter: mouse.filter,
        yaw: mouse.yaw,
        pitch: mouse.pitch,
        side: mouse.side,
        forward: mouse.forward,
        free_look: mouse.free_look,
        look_spring: mouse.look_spring,
        look_strafe: mouse.look_strafe,
        invert_pitch: mouse.invert_pitch,
    }
}

/// User-selected frontend overrides plus saved baselines (donor
/// `FrontendPreferences`).
#[derive(Clone)]
pub struct FrontendPreferences {
    /// Shared override/baseline state.
    state: Rc<RefCell<FrontendState>>,
    /// Current command dialect.
    dialect: Rc<dyn Fn() -> Dialect>,
    /// Audio output surface, when the backend exposes one.
    audio_output: Option<Rc<dyn AudioOutputSettings>>,
    /// Music preference reader, when wired.
    music_settings: Option<Rc<dyn Fn() -> MusicPreferences>>,
}

impl FrontendPreferences {
    /// Create empty overrides reading the current dialect.
    pub fn new(dialect: impl Fn() -> Dialect + 'static) -> Self {
        Self {
            state: Rc::new(RefCell::new(FrontendState::default())),
            dialect: Rc::new(dialect),
            audio_output: None,
            music_settings: None,
        }
    }

    /// Current user-selected overrides.
    #[must_use]
    pub fn values(&self) -> FrontendPreferenceOverrides {
        self.state.borrow().values
    }

    /// Replace the user-selected overrides.
    pub fn set_values(&self, values: FrontendPreferenceOverrides) {
        self.state.borrow_mut().values = values;
    }

    /// Saved volume baselines.
    #[must_use]
    pub fn audio_baseline(&self) -> AudioBaseline {
        self.state.borrow().audio_baseline
    }

    /// Current audio values: overrides, then baselines, then defaults.
    #[must_use]
    pub fn audio_values(&self) -> AudioSettings {
        FrontendService::audio_values(&self.state.borrow())
    }

    /// Load volume, mouse, run, and vibration baselines (donor
    /// `loadBaseline`). Saved mouse tuning round-trips through a scratch
    /// cvar registry exactly like the donor's scratch `MouseSettings`.
    pub fn load_baseline(&self, store: &ConfigStore) -> Result<(), FrontendPreferencesError> {
        let audio = load_audio_settings(store)?;
        let saved = store.load_seat(FRONTEND_BASELINE_SEAT)?;
        let mut registry = CvarRegistry::new((self.dialect)());
        register_mouse_settings(&mut registry)?;
        if let Some(saved) = &saved {
            write_mouse_tuning(&mut registry, &client_tuning(saved.mouse))?;
        }
        let mut state = self.state.borrow_mut();
        state.audio_baseline = AudioBaseline {
            effects_volume: audio.as_ref().map(|audio| audio.effects_volume),
            music_volume: audio.as_ref().map(|audio| audio.music_volume),
        };
        state.mouse_baseline = read_mouse_tuning(&registry);
        state.always_run_baseline = saved.as_ref().and_then(|saved| saved.always_run);
        state.vibration_baseline = ControllerVibrationSettings {
            controller_vibration: saved.as_ref().is_none_or(|saved| saved.rumble),
            controller_vibration_strength: saved.as_ref().map_or(1.0, |saved| saved.rumble_strength) as f32,
        };
        Ok(())
    }

    /// Persist the audio baseline when an output, music reader, or volume
    /// override is wired (donor `saveAudioBaseline`).
    pub fn save_audio_baseline(&self, store: &ConfigStore) -> Result<(), FrontendPreferencesError> {
        let state = self.state.borrow();
        if self.audio_output.is_none()
            && self.music_settings.is_none()
            && state.values.effects_volume.is_none()
            && state.values.music_volume.is_none()
        {
            return Ok(());
        }
        let fallback = FrontendService::audio_values(&state);
        let saved = load_audio_settings(store)?;
        let effects_volume = state
            .values
            .effects_volume
            .or(saved.as_ref().map(|saved| saved.effects_volume))
            .unwrap_or(f64::from(fallback.effects_volume));
        let music_volume = state
            .values
            .music_volume
            .or(saved.as_ref().map(|saved| saved.music_volume))
            .unwrap_or(f64::from(fallback.music_volume));
        let device_name = self
            .audio_output
            .as_ref()
            .map(|output| output.selected())
            .unwrap_or_else(|| saved.as_ref().and_then(|saved| saved.device_name.clone()));
        let output_format = self
            .audio_output
            .as_ref()
            .and_then(|output| output.format())
            .map(|format| stored_format(&format.read()))
            .or_else(|| saved.as_ref().map(|saved| saved.output_format))
            .unwrap_or(DEFAULT_AUDIO_OUTPUT_FORMAT);
        let music = self
            .music_settings
            .as_ref()
            .map(|read| read())
            .unwrap_or(MusicPreferences {
                music_shuffle: saved.as_ref().and_then(|saved| saved.music_shuffle).unwrap_or(false),
                menu_track: saved
                    .as_ref()
                    .and_then(|saved| saved.menu_track.clone())
                    .unwrap_or_else(|| "auto".to_string()),
            });
        let changed = saved.as_ref().is_none_or(|saved| {
            saved.music_shuffle != Some(music.music_shuffle)
                || saved.menu_track.as_deref() != Some(music.menu_track.as_str())
                || saved.device_name != device_name
                || saved.effects_volume != effects_volume
                || saved.music_volume != music_volume
                || saved.output_format.sample_rate != output_format.sample_rate
                || saved.output_format.sample_bits != output_format.sample_bits
                || saved.output_format.channels != output_format.channels
        });
        if changed {
            save_audio_settings(
                store,
                &AudioSaveRequest {
                    music_preferences: Some(music),
                    output_format: Some(output_format),
                    selected_output: device_name,
                    effects_volume,
                    music_volume,
                },
            )?;
        }
        drop(state);
        self.state.borrow_mut().audio_baseline = AudioBaseline {
            effects_volume: Some(effects_volume),
            music_volume: Some(music_volume),
        };
        Ok(())
    }

    /// Build the settings rows editing these overrides (donor `bindings`),
    /// remembering the output and music surfaces for later saves.
    pub fn bindings(
        &mut self,
        output: Option<Rc<dyn AudioOutputSettings>>,
        music_settings: Option<Rc<dyn Fn() -> MusicPreferences>>,
    ) -> Vec<SettingBinding> {
        self.audio_output = output.clone();
        self.music_settings = music_settings;
        let service = Rc::new(FrontendService {
            state: Rc::clone(&self.state),
            dialect: Rc::clone(&self.dialect),
        });
        let dialect = Rc::clone(&self.dialect);
        let mut rows = bind_controller_vibration(service.clone());
        rows.extend(bind_audio_settings(service.clone(), output));
        rows.extend(bind_primary_input_settings(service.clone()));
        rows.extend(bind_mouse_motion_settings(
            service,
            Rc::new(move || matches!(dialect(), Dialect::Q1Netquake | Dialect::Q1Quakeworld)),
        ));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use qa_client::ui::settings::SettingBindingKind;

    fn store(name: &str) -> ConfigStore {
        let root: PathBuf = std::env::temp_dir().join(format!("qa-frontend-preferences-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        ConfigStore::new(root)
    }

    fn input_values() -> FrontendInputValues {
        FrontendInputValues {
            controller_vibration: true,
            controller_vibration_strength: 1.0,
            sensitivity: 3.0,
            pitch: 0.022,
            yaw: 0.022,
            invert_mouse: false,
            acceleration: 0.0,
            filter: false,
            look_spring: false,
            look_strafe: false,
            free_look: true,
            always_run: true,
        }
    }

    fn full_values() -> FrontendPreferenceValues {
        FrontendPreferenceValues {
            effects_volume: 0.7,
            music_volume: 0.25,
            input: input_values(),
        }
    }

    struct StubAudio {
        effects: f64,
        music: f64,
    }

    impl FrontendAudio for StubAudio {
        fn effects_volume(&self) -> f64 {
            self.effects
        }
        fn set_effects_volume(&mut self, volume: f64) {
            self.effects = volume;
        }
        fn music_volume(&self) -> f64 {
            self.music
        }
        fn set_music_volume(&mut self, volume: f64) {
            self.music = volume;
        }
    }

    struct StubLocal {
        values: FrontendInputValues,
    }

    impl FrontendLocalInput for StubLocal {
        fn read_frontend_input(&self) -> FrontendInputValues {
            self.values
        }
        fn apply_frontend_input(&mut self, values: &FrontendInputOverrides) {
            if let Some(vibration) = values.controller_vibration {
                self.values.controller_vibration = vibration;
            }
            if let Some(strength) = values.controller_vibration_strength {
                self.values.controller_vibration_strength = strength;
            }
            if let Some(sensitivity) = values.sensitivity {
                self.values.sensitivity = sensitivity;
            }
            if let Some(always_run) = values.always_run {
                self.values.always_run = always_run;
            }
        }
    }

    #[test]
    fn applies_volumes_and_seat_overrides() {
        let mut audio = StubAudio {
            effects: 0.5,
            music: 0.5,
        };
        let mut locals = vec![StubLocal { values: input_values() }];
        let values = FrontendPreferenceOverrides {
            effects_volume: Some(0.9),
            music_volume: None,
            input: FrontendInputOverrides {
                sensitivity: Some(5.0),
                always_run: Some(false),
                ..FrontendInputOverrides::default()
            },
        };
        apply_frontend_preferences(&values, &mut locals, &mut audio);
        assert_eq!((audio.effects, audio.music), (0.9, 0.5));
        assert_eq!(locals[0].values.sensitivity, 5.0);
        assert!(!locals[0].values.always_run);
        let read = read_frontend_preferences(&locals, &audio).expect("first seat");
        assert_eq!(read.effects_volume, 0.9);
        assert_eq!(read.input.sensitivity, 5.0);
        let empty: Vec<StubLocal> = Vec::new();
        assert_eq!(read_frontend_preferences(&empty, &audio), None);
    }

    #[test]
    fn merges_overrides_onto_saved_seat() {
        let store = store("seat");
        let saved = SeatSettings {
            version: 1,
            bindings: Vec::new(),
            gamepad: crate::settings::config::GamepadTuning {
                move_curve: crate::settings::config::StickCurve::Axial {
                    deadzone: 0.2,
                    exponent: 1.0,
                },
                look_curve: crate::settings::config::StickCurve::Axial {
                    deadzone: 0.2,
                    exponent: 1.0,
                },
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
            },
            mouse: StoredMouseTuning {
                sensitivity: 3.0,
                acceleration: 0.0,
                filter: false,
                yaw: 0.022,
                pitch: 0.022,
                side: 0.8,
                forward: 1.0,
                look_spring: false,
                look_strafe: false,
                free_look: true,
                invert_pitch: false,
            },
            always_run: None,
            history: Vec::new(),
            rumble: true,
            rumble_strength: 1.0,
            controller: crate::settings::config::ControllerSelection::Automatic,
        };
        let _ = store;
        let values = FrontendPreferenceOverrides {
            effects_volume: None,
            music_volume: None,
            input: FrontendInputOverrides {
                sensitivity: Some(6.0),
                invert_mouse: Some(true),
                always_run: Some(true),
                controller_vibration_strength: Some(0.5),
                ..FrontendInputOverrides::default()
            },
        };
        let merged = frontend_seat_settings(&values, &saved);
        assert_eq!(merged.mouse.sensitivity, 6.0);
        assert!(merged.mouse.invert_pitch);
        assert_eq!(merged.always_run, Some(true));
        assert!(merged.rumble);
        assert_eq!(merged.rumble_strength, 0.5);
    }

    #[test]
    fn diffs_values_with_object_is_pitch_semantics() {
        let before = full_values();
        let mut after = before;
        after.input.sensitivity = 4.0;
        after.input.pitch = -0.0;
        after.input.yaw = f64::NAN;
        let mut nan_before = before;
        nan_before.input.yaw = f64::NAN;
        let diff = changed_frontend_preferences(&nan_before, &after, &FrontendPreferenceOverrides::default());
        assert_eq!(diff.input.sensitivity, Some(4.0));
        assert_eq!(diff.input.pitch, Some(-0.0));
        assert_eq!(diff.input.yaw, None);
        assert_eq!(diff.effects_volume, None);
    }

    #[test]
    fn baselines_default_without_saved_documents() {
        let frontend = FrontendPreferences::new(|| Dialect::Q2Classic);
        frontend.load_baseline(&store("empty")).expect("load");
        assert_eq!(frontend.audio_baseline(), AudioBaseline::default());
        assert_eq!(
            frontend.audio_values(),
            AudioSettings {
                effects_volume: DEFAULT_EFFECTS_VOLUME as f32,
                music_volume: DEFAULT_MUSIC_VOLUME as f32,
            }
        );
    }

    #[test]
    fn unwired_audio_baseline_save_is_a_noop() {
        let frontend = FrontendPreferences::new(|| Dialect::Q2Classic);
        let root: PathBuf = std::env::temp_dir().join(format!("qa-frontend-preferences-{}-noop", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        frontend
            .save_audio_baseline(&ConfigStore::new(root.clone()))
            .expect("save");
        assert!(!root.join("audio.json").exists());
    }

    #[test]
    fn volume_override_saves_and_reloads_baseline() {
        let frontend = FrontendPreferences::new(|| Dialect::Q2Classic);
        frontend.set_values(FrontendPreferenceOverrides {
            effects_volume: Some(0.8),
            music_volume: Some(0.4),
            input: FrontendInputOverrides::default(),
        });
        let store = store("save");
        frontend.save_audio_baseline(&store).expect("save");
        assert_eq!(
            frontend.audio_baseline(),
            AudioBaseline {
                effects_volume: Some(0.8),
                music_volume: Some(0.4),
            }
        );
        let reloaded = FrontendPreferences::new(|| Dialect::Q2Classic);
        reloaded.load_baseline(&store).expect("load");
        assert_eq!(reloaded.audio_baseline(), frontend.audio_baseline());
    }

    #[test]
    fn bindings_edit_the_shared_overrides() {
        let mut frontend = FrontendPreferences::new(|| Dialect::Q1Netquake);
        let rows = frontend.bindings(None, None);
        assert!(rows.len() >= 12);
        let sensitivity = rows
            .iter()
            .find(|row| row.label == "Mouse sensitivity")
            .expect("sensitivity row");
        let SettingBindingKind::Slider { read, write, .. } = &sensitivity.kind else {
            panic!("sensitivity is a slider");
        };
        assert_eq!(read(), default_mouse_tuning().sensitivity as f32);
        write(7.5);
        assert_eq!(frontend.values().input.sensitivity, Some(7.5));
        let freelook = rows.iter().find(|row| row.label == "Free look").expect("freelook row");
        let SettingBindingKind::Toggle {
            write: set_freelook, ..
        } = &freelook.kind
        else {
            panic!("free look is a toggle");
        };
        set_freelook(false);
        let lookspring = rows
            .iter()
            .find(|row| row.label == "Look spring")
            .expect("lookspring row");
        assert!((lookspring.enabled)());
    }

    #[test]
    fn lookspring_row_disables_outside_q1() {
        let mut frontend = FrontendPreferences::new(|| Dialect::Q3);
        let rows = frontend.bindings(None, None);
        let lookspring = rows
            .iter()
            .find(|row| row.label == "Look spring")
            .expect("lookspring row");
        assert!(!(lookspring.enabled)());
    }
}
