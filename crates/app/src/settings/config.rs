//! Seat configuration documents and the config file store.
//!
//! Donor provenance: `src/settings/config.ts` (`parseSeatSettings`,
//! `settingsPath`, `writeAtomic`, `ConfigStore`, `parseGyroProfile` with
//! `validateGamepadTuning` from `src/input/gamepad.ts`). Same document
//! versions, validation messages, path-escape rules, and atomic-write
//! protocol. Async file calls become synchronous `std::fs` calls;
//! `execute` dispatches through [`crate::console`] instead of appending to
//! the donor `CommandBuffer`.

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use qa_client::input::{AxisDirection, ControllerAxis, InputAction, InputBinding, InputBindingTarget, PhysicalInput};
use qa_core::cmd::Dialect;
use qa_core::cvar::CvarRegistry;

use super::json::{parse_json, stringify, stringify_pretty, Json};
use super::SettingsError;
use crate::console::commands::{ConsoleCommandServices, ConsoleCommands};

/// Settings document version.
pub const SETTINGS_VERSION: u32 = 1;
/// Maximum gyro settings file size in bytes.
pub const MAX_GYRO_BYTES: usize = 8192;

/// Stick response curve (donor `StickCurve`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StickCurve {
    /// Radial deadzone with an outer threshold.
    Radial {
        /// Deadzone radius (`0..<1`).
        deadzone: f64,
        /// Response exponent (positive).
        exponent: f64,
        /// Outer threshold (`deadzone + outer < 1`).
        outer_threshold: f64,
    },
    /// Per-axis deadzone.
    Axial {
        /// Deadzone (`0..<1`).
        deadzone: f64,
        /// Response exponent (positive).
        exponent: f64,
    },
}

/// Gyro yaw axis (donor `yawAxis`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GyroYawAxis {
    /// Yaw around Y.
    Y,
    /// Yaw around Z.
    Z,
}

/// Gyro tuning (donor `GamepadTuning["gyro"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroTuning {
    /// Whether gyro aiming is enabled.
    pub enabled: bool,
    /// Yaw axis.
    pub yaw_axis: GyroYawAxis,
    /// Yaw sensitivity.
    pub yaw_sensitivity: f64,
    /// Pitch sensitivity.
    pub pitch_sensitivity: f64,
}

/// Gamepad tuning (donor `GamepadTuning`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadTuning {
    /// Movement stick curve.
    pub move_curve: StickCurve,
    /// Look stick curve.
    pub look_curve: StickCurve,
    /// Swap the sticks.
    pub swap_sticks: bool,
    /// Turn rate in degrees per second.
    pub yaw_degrees_per_second: f64,
    /// Look rate in degrees per second.
    pub pitch_degrees_per_second: f64,
    /// Invert the look pitch.
    pub invert_pitch: bool,
    /// Forward sensitivity.
    pub forward_sensitivity: f64,
    /// Strafe sensitivity.
    pub side_sensitivity: f64,
    /// Trigger press threshold (`0..=1`).
    pub trigger_threshold: f64,
    /// Gyro tuning.
    pub gyro: GyroTuning,
}

/// Validate gamepad ranges (donor `validateGamepadTuning`).
pub fn validate_gamepad_tuning(tuning: &GamepadTuning) -> Result<(), SettingsError> {
    for curve in [tuning.move_curve, tuning.look_curve] {
        let (deadzone, exponent, outer) = match curve {
            StickCurve::Radial {
                deadzone,
                exponent,
                outer_threshold,
            } => (deadzone, exponent, Some(outer_threshold)),
            StickCurve::Axial { deadzone, exponent } => (deadzone, exponent, None),
        };
        if !deadzone.is_finite() || deadzone < 0.0 || deadzone >= 1.0 || !exponent.is_finite() || exponent <= 0.0 {
            return Err(SettingsError::BadValue(
                "Invalid gamepad deadzone or response curve".to_string(),
            ));
        }
        if let Some(outer) = outer {
            if !outer.is_finite() || outer < 0.0 || deadzone + outer >= 1.0 {
                return Err(SettingsError::BadValue(
                    "Invalid gamepad deadzone or response curve".to_string(),
                ));
            }
        }
    }
    for value in [
        tuning.yaw_degrees_per_second,
        tuning.pitch_degrees_per_second,
        tuning.forward_sensitivity,
        tuning.side_sensitivity,
        tuning.gyro.yaw_sensitivity,
        tuning.gyro.pitch_sensitivity,
    ] {
        if !value.is_finite() {
            return Err(SettingsError::BadValue("Invalid gamepad sensitivity".to_string()));
        }
    }
    if !tuning.trigger_threshold.is_finite() || tuning.trigger_threshold < 0.0 || tuning.trigger_threshold > 1.0 {
        return Err(SettingsError::BadValue("Invalid trigger threshold".to_string()));
    }
    Ok(())
}

/// Mouse tuning (donor `MouseTuning`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseTuning {
    /// Overall sensitivity.
    pub sensitivity: f64,
    /// Acceleration.
    pub acceleration: f64,
    /// Filter (smooth) mouse input.
    pub filter: bool,
    /// Yaw scale.
    pub yaw: f64,
    /// Pitch scale.
    pub pitch: f64,
    /// Strafe scale.
    pub side: f64,
    /// Forward scale.
    pub forward: f64,
    /// Spring the look back to center.
    pub look_spring: bool,
    /// Strafe while mouse-looking.
    pub look_strafe: bool,
    /// Free look.
    pub free_look: bool,
    /// Invert the mouse pitch.
    pub invert_pitch: bool,
}

/// Controller selection (donor `ControllerSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerSelection {
    /// Pick automatically.
    Automatic,
    /// No controller.
    None,
    /// A specific device by GUID and ordinal.
    Device {
        /// 32 hexadecimal digits.
        guid: String,
        /// Device ordinal.
        ordinal: u32,
    },
    /// A specific device by GUID and serial.
    Serial {
        /// 32 hexadecimal digits.
        guid: String,
        /// Device serial.
        serial: String,
    },
}

/// Seat settings document (donor `SeatSettings`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatSettings {
    /// Document version (always 1).
    pub version: u32,
    /// Input bindings.
    pub bindings: Vec<InputBinding>,
    /// Gamepad tuning.
    pub gamepad: GamepadTuning,
    /// Mouse tuning.
    pub mouse: MouseTuning,
    /// Always run.
    pub always_run: Option<bool>,
    /// Console history lines.
    pub history: Vec<String>,
    /// Rumble enabled.
    pub rumble: bool,
    /// Rumble strength (`0..=1`).
    pub rumble_strength: f64,
    /// Controller selection.
    pub controller: ControllerSelection,
}

/// Gyro profile identity (donor `GyroProfileIdentity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GyroProfileIdentity {
    /// Per-seat profile.
    Seat,
    /// Per-device profile.
    Device {
        /// Lowercase 32 hexadecimal digits.
        guid: String,
        /// Device serial.
        serial: String,
    },
}

/// Gyro profile document (donor `GyroProfile`).
#[derive(Debug, Clone, PartialEq)]
pub struct GyroProfile {
    /// Document version (always 1).
    pub version: u32,
    /// Profile identity.
    pub identity: GyroProfileIdentity,
    /// Gyro tuning.
    pub tuning: GyroTuning,
}

/// Input routing document: which seat owns the keyboard, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRouting {
    /// Keyboard seat index, or [`None`] for unrouted.
    pub keyboard_seat: Option<u32>,
}

fn as_object(value: &Json) -> Result<&Vec<(String, Json)>, SettingsError> {
    match value {
        Json::Object(members) => Ok(members),
        _ => Err(SettingsError::BadValue("Expected a settings object".to_string())),
    }
}

fn member<'a>(members: &'a [(String, Json)], key: &str) -> Result<&'a Json, SettingsError> {
    members
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .ok_or_else(|| SettingsError::BadValue("Expected a settings object".to_string()))
}

fn number(value: &Json) -> Result<f64, SettingsError> {
    match value {
        Json::Number(number) if number.is_finite() => Ok(*number),
        _ => Err(SettingsError::BadValue("Expected a finite settings number".to_string())),
    }
}

fn natural(value: &Json) -> Result<u32, SettingsError> {
    let number = number(value)?;
    if number < 0.0 || number.trunc() != number || number > f64::from(u32::MAX) {
        return Err(SettingsError::BadValue(
            "Expected a nonnegative settings integer".to_string(),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(number as u32)
}

fn boolean(value: &Json) -> Result<bool, SettingsError> {
    match value {
        Json::Bool(value) => Ok(*value),
        _ => Err(SettingsError::BadValue("Expected a settings boolean".to_string())),
    }
}

fn text(value: &Json) -> Result<String, SettingsError> {
    match value {
        Json::String(text) => Ok(text.clone()),
        _ => Err(SettingsError::BadValue("Expected settings text".to_string())),
    }
}

fn texts(value: &Json) -> Result<Vec<String>, SettingsError> {
    match value {
        Json::Array(items) => items.iter().map(text).collect(),
        _ => Err(SettingsError::BadValue("Expected a settings list".to_string())),
    }
}

fn curve(value: &Json) -> Result<StickCurve, SettingsError> {
    let members = as_object(value)?;
    let deadzone = number(member(members, "deadzone")?)?;
    let exponent = number(member(members, "exponent")?)?;
    match member(members, "kind")? {
        Json::String(kind) if kind == "radial" => Ok(StickCurve::Radial {
            deadzone,
            exponent,
            outer_threshold: number(member(members, "outerThreshold")?)?,
        }),
        Json::String(kind) if kind == "axial" => Ok(StickCurve::Axial { deadzone, exponent }),
        _ => Err(SettingsError::BadValue("Unknown stick curve".to_string())),
    }
}

fn yaw_axis(value: &Json) -> Result<GyroYawAxis, SettingsError> {
    match value {
        Json::String(axis) if axis == "y" => Ok(GyroYawAxis::Y),
        Json::String(axis) if axis == "z" => Ok(GyroYawAxis::Z),
        _ => Err(SettingsError::BadValue("Unknown gyro yaw axis".to_string())),
    }
}

fn gyro_tuning(value: &Json) -> Result<GyroTuning, SettingsError> {
    let members = as_object(value)?;
    Ok(GyroTuning {
        enabled: boolean(member(members, "enabled")?)?,
        yaw_axis: yaw_axis(member(members, "yawAxis")?)?,
        yaw_sensitivity: number(member(members, "yawSensitivity")?)?,
        pitch_sensitivity: number(member(members, "pitchSensitivity")?)?,
    })
}

fn gamepad(value: &Json) -> Result<GamepadTuning, SettingsError> {
    let members = as_object(value)?;
    let tuning = GamepadTuning {
        move_curve: curve(member(members, "move")?)?,
        look_curve: curve(member(members, "look")?)?,
        swap_sticks: boolean(member(members, "swapSticks")?)?,
        yaw_degrees_per_second: number(member(members, "yawDegreesPerSecond")?)?,
        pitch_degrees_per_second: number(member(members, "pitchDegreesPerSecond")?)?,
        invert_pitch: boolean(member(members, "invertPitch")?)?,
        forward_sensitivity: number(member(members, "forwardSensitivity")?)?,
        side_sensitivity: number(member(members, "sideSensitivity")?)?,
        trigger_threshold: number(member(members, "triggerThreshold")?)?,
        gyro: gyro_tuning(member(members, "gyro")?)?,
    };
    validate_gamepad_tuning(&tuning)?;
    Ok(tuning)
}

fn mouse(value: &Json) -> Result<MouseTuning, SettingsError> {
    let members = as_object(value)?;
    let optional = |key: &str| -> Result<bool, SettingsError> {
        match members.iter().find(|(name, _)| name == key) {
            None => Ok(false),
            Some((_, value)) => boolean(value),
        }
    };
    Ok(MouseTuning {
        sensitivity: number(member(members, "sensitivity")?)?,
        acceleration: number(member(members, "acceleration")?)?,
        filter: boolean(member(members, "filter")?)?,
        yaw: number(member(members, "yaw")?)?,
        pitch: number(member(members, "pitch")?)?,
        side: number(member(members, "side")?)?,
        forward: number(member(members, "forward")?)?,
        look_spring: optional("lookSpring")?,
        look_strafe: optional("lookStrafe")?,
        free_look: boolean(member(members, "freeLook")?)?,
        invert_pitch: boolean(member(members, "invertPitch")?)?,
    })
}

fn axis(value: &Json) -> Result<ControllerAxis, SettingsError> {
    match value {
        Json::String(axis) if axis == "left-x" => Ok(ControllerAxis::LeftX),
        Json::String(axis) if axis == "left-y" => Ok(ControllerAxis::LeftY),
        Json::String(axis) if axis == "right-x" => Ok(ControllerAxis::RightX),
        Json::String(axis) if axis == "right-y" => Ok(ControllerAxis::RightY),
        Json::String(axis) if axis == "left-trigger" => Ok(ControllerAxis::LeftTrigger),
        Json::String(axis) if axis == "right-trigger" => Ok(ControllerAxis::RightTrigger),
        _ => Err(SettingsError::BadValue("Unknown controller axis".to_string())),
    }
}

fn physical(value: &Json) -> Result<PhysicalInput, SettingsError> {
    let members = as_object(value)?;
    let kind = member(members, "kind")?;
    match kind {
        Json::String(kind) if kind == "key" => Ok(PhysicalInput::Key(natural(member(members, "code")?)? as i32)),
        Json::String(kind) if kind == "mouse-button" => {
            Ok(PhysicalInput::MouseButton(natural(member(members, "button")?)? as i32))
        }
        Json::String(kind) if kind == "controller-button" => Ok(PhysicalInput::ControllerButton {
            device: natural(member(members, "device")?)? as i32,
            button: natural(member(members, "button")?)? as i32,
        }),
        Json::String(kind) if kind == "controller-axis" => {
            let direction = match member(members, "direction")? {
                Json::String(direction) if direction == "positive" => AxisDirection::Positive,
                Json::String(direction) if direction == "negative" => AxisDirection::Negative,
                _ => return Err(SettingsError::BadValue("Unknown axis direction".to_string())),
            };
            Ok(PhysicalInput::ControllerAxis {
                device: natural(member(members, "device")?)? as i32,
                axis: axis(member(members, "axis")?)?,
                direction,
            })
        }
        _ => Err(SettingsError::BadValue("Unknown binding input".to_string())),
    }
}

fn action(value: &Json) -> Result<InputAction, SettingsError> {
    match value {
        Json::String(action) if action == "attack" => Ok(InputAction::Attack),
        Json::String(action) if action == "jump" => Ok(InputAction::Jump),
        Json::String(action) if action == "forward" => Ok(InputAction::Forward),
        Json::String(action) if action == "back" => Ok(InputAction::Back),
        Json::String(action) if action == "move-left" => Ok(InputAction::MoveLeft),
        Json::String(action) if action == "move-right" => Ok(InputAction::MoveRight),
        Json::String(action) if action == "move-up" => Ok(InputAction::MoveUp),
        Json::String(action) if action == "move-down" => Ok(InputAction::MoveDown),
        Json::String(action) if action == "use" => Ok(InputAction::Use),
        Json::String(action) if action == "crouch" => Ok(InputAction::Crouch),
        Json::String(action) if action == "walk" => Ok(InputAction::Walk),
        Json::String(action) if action == "scores" => Ok(InputAction::Scores),
        Json::String(action) if action == "next-weapon" => Ok(InputAction::NextWeapon),
        Json::String(action) if action == "previous-weapon" => Ok(InputAction::PreviousWeapon),
        Json::String(action) if action == "menu" => Ok(InputAction::Menu),
        _ => Err(SettingsError::BadValue("Unknown input action".to_string())),
    }
}

fn target(value: &Json) -> Result<InputBindingTarget, SettingsError> {
    let members = as_object(value)?;
    match member(members, "kind")? {
        Json::String(kind) if kind == "action" => Ok(InputBindingTarget::Action(action(member(members, "action")?)?)),
        Json::String(kind) if kind == "command" => Ok(InputBindingTarget::Command(text(member(members, "text")?)?)),
        _ => Err(SettingsError::BadValue("Unknown binding target".to_string())),
    }
}

fn is_guid_hex(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn controller(value: &Json) -> Result<ControllerSelection, SettingsError> {
    let members = as_object(value)?;
    match member(members, "kind")? {
        Json::String(kind) if kind == "automatic" => Ok(ControllerSelection::Automatic),
        Json::String(kind) if kind == "none" => Ok(ControllerSelection::None),
        Json::String(kind) if kind == "device" => {
            let guid = text(member(members, "guid")?)?;
            if !is_guid_hex(&guid) {
                return Err(SettingsError::BadValue(
                    "Controller GUID requires 32 hexadecimal digits".to_string(),
                ));
            }
            Ok(ControllerSelection::Device {
                guid,
                ordinal: natural(member(members, "ordinal")?)?,
            })
        }
        Json::String(kind) if kind == "serial" => {
            let guid = text(member(members, "guid")?)?;
            if !is_guid_hex(&guid) {
                return Err(SettingsError::BadValue(
                    "Controller GUID requires 32 hexadecimal digits".to_string(),
                ));
            }
            Ok(ControllerSelection::Serial {
                guid,
                serial: text(member(members, "serial")?)?,
            })
        }
        _ => Err(SettingsError::BadValue("Unknown controller selection".to_string())),
    }
}

/// Parse a seat settings document (donor `parseSeatSettings`).
pub fn parse_seat_settings(value: &Json) -> Result<SeatSettings, SettingsError> {
    let members = as_object(value)?;
    let version = member(members, "version")?;
    let bindings = member(members, "bindings")?;
    if version != &Json::Number(1.0) || !matches!(bindings, Json::Array(_)) {
        return Err(SettingsError::BadValue(
            "Unsupported seat settings document".to_string(),
        ));
    }
    let bindings = match bindings {
        Json::Array(items) => items
            .iter()
            .map(|item| {
                let binding = as_object(item)?;
                Ok(InputBinding {
                    input: physical(member(binding, "input")?)?,
                    target: target(member(binding, "target")?)?,
                })
            })
            .collect::<Result<Vec<_>, SettingsError>>()?,
        _ => {
            return Err(SettingsError::BadValue(
                "Unsupported seat settings document".to_string(),
            ))
        }
    };
    let always_run = match members.iter().find(|(name, _)| name == "alwaysRun") {
        None => None,
        Some((_, value)) => Some(boolean(value)?),
    };
    let rumble_strength = match members.iter().find(|(name, _)| name == "rumbleStrength") {
        None => 1.0,
        Some((_, value)) => {
            let strength = number(value)?;
            if !(0.0..=1.0).contains(&strength) {
                return Err(SettingsError::BadValue(
                    "Vibration strength must be between zero and one".to_string(),
                ));
            }
            strength
        }
    };
    Ok(SeatSettings {
        version: SETTINGS_VERSION,
        bindings,
        gamepad: gamepad(member(members, "gamepad")?)?,
        mouse: mouse(member(members, "mouse")?)?,
        always_run,
        history: texts(member(members, "history")?)?,
        rumble: boolean(member(members, "rumble")?)?,
        rumble_strength,
        controller: controller(member(members, "controller")?)?,
    })
}

/// Parse a seat settings document from text.
pub fn parse_seat_settings_text(text: &str) -> Result<SeatSettings, SettingsError> {
    parse_seat_settings(&parse_json(text)?)
}

/// Parse a gyro profile document (donor `parseGyroProfile`).
pub fn parse_gyro_profile(value: &Json) -> Result<GyroProfile, SettingsError> {
    let members = as_object(value)?;
    if member(members, "version")? != &Json::Number(1.0) {
        return Err(SettingsError::BadValue("Unsupported gyro settings version".to_string()));
    }
    let identity = as_object(member(members, "identity")?)?;
    let identity = match member(identity, "kind")? {
        Json::String(kind) if kind == "seat" => GyroProfileIdentity::Seat,
        Json::String(kind) if kind == "device" => {
            let guid = text(member(identity, "guid")?)?;
            let serial = text(member(identity, "serial")?)?;
            let lowercase = guid.len() == 32
                && guid
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
            if !lowercase || serial.is_empty() || serial.len() > 256 || serial.contains('\0') {
                return Err(SettingsError::BadValue("Invalid gyro device identity".to_string()));
            }
            GyroProfileIdentity::Device { guid, serial }
        }
        _ => return Err(SettingsError::BadValue("Unknown gyro profile identity".to_string())),
    };
    Ok(GyroProfile {
        version: SETTINGS_VERSION,
        identity,
        tuning: gyro_tuning(member(members, "tuning")?)?,
    })
}

/// Explicit per-item directory overrides (user input; no donor equivalent).
/// Each [`None`] field selects the automatic default for that item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostDirectories {
    /// Game data root override.
    pub corpus_root: Option<String>,
    /// Writable user content root override.
    pub user_content_root: Option<String>,
}

fn optional_text(value: &Json) -> Result<Option<String>, SettingsError> {
    match value {
        Json::Null => Ok(None),
        Json::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.contains('\0') {
                return Err(SettingsError::BadValue(
                    "Host directory paths must not be blank".to_string(),
                ));
            }
            Ok(Some(trimmed.to_string()))
        }
        _ => Err(SettingsError::BadValue("Expected settings text".to_string())),
    }
}

/// Parse a host directories document body.
pub fn parse_host_directories(value: &Json) -> Result<HostDirectories, SettingsError> {
    let members = as_object(value)?;
    if member(members, "version")? != &Json::Number(1.0) {
        return Err(SettingsError::BadValue(
            "Unsupported host directories document".to_string(),
        ));
    }
    let field = |key: &str| {
        members
            .iter()
            .find(|(name, _)| name == key)
            .map_or(Ok(None), |(_, value)| optional_text(value))
    };
    Ok(HostDirectories {
        corpus_root: field("corpusRoot")?,
        user_content_root: field("userContentRoot")?,
    })
}

/// Parse an input routing document body.
pub fn parse_input_routing(value: &Json) -> Result<InputRouting, SettingsError> {
    let members = as_object(value)?;
    if member(members, "version")? != &Json::Number(1.0) {
        return Err(SettingsError::BadValue(
            "Unsupported input routing document".to_string(),
        ));
    }
    let seat = member(members, "keyboardSeat")?;
    Ok(InputRouting {
        keyboard_seat: if seat == &Json::Null {
            None
        } else {
            Some(natural(seat)?)
        },
    })
}

fn action_name(action: InputAction) -> &'static str {
    match action {
        InputAction::Attack => "attack",
        InputAction::Jump => "jump",
        InputAction::Forward => "forward",
        InputAction::Back => "back",
        InputAction::MoveLeft => "move-left",
        InputAction::MoveRight => "move-right",
        InputAction::MoveUp => "move-up",
        InputAction::MoveDown => "move-down",
        InputAction::Use => "use",
        InputAction::Crouch => "crouch",
        InputAction::Walk => "walk",
        InputAction::Scores => "scores",
        InputAction::NextWeapon => "next-weapon",
        InputAction::PreviousWeapon => "previous-weapon",
        InputAction::Menu => "menu",
    }
}

fn axis_name(axis: ControllerAxis) -> &'static str {
    match axis {
        ControllerAxis::LeftX => "left-x",
        ControllerAxis::LeftY => "left-y",
        ControllerAxis::RightX => "right-x",
        ControllerAxis::RightY => "right-y",
        ControllerAxis::LeftTrigger => "left-trigger",
        ControllerAxis::RightTrigger => "right-trigger",
    }
}

fn physical_json(input: &PhysicalInput) -> Json {
    match input {
        PhysicalInput::Key(code) => Json::Object(vec![
            ("kind".to_string(), Json::String("key".to_string())),
            ("code".to_string(), Json::Number(f64::from(*code))),
        ]),
        PhysicalInput::MouseButton(button) => Json::Object(vec![
            ("kind".to_string(), Json::String("mouse-button".to_string())),
            ("button".to_string(), Json::Number(f64::from(*button))),
        ]),
        PhysicalInput::ControllerButton { device, button } => Json::Object(vec![
            ("kind".to_string(), Json::String("controller-button".to_string())),
            ("device".to_string(), Json::Number(f64::from(*device))),
            ("button".to_string(), Json::Number(f64::from(*button))),
        ]),
        PhysicalInput::ControllerAxis {
            device,
            axis,
            direction,
        } => Json::Object(vec![
            ("kind".to_string(), Json::String("controller-axis".to_string())),
            ("device".to_string(), Json::Number(f64::from(*device))),
            ("axis".to_string(), Json::String(axis_name(*axis).to_string())),
            (
                "direction".to_string(),
                Json::String(
                    match direction {
                        AxisDirection::Positive => "positive",
                        AxisDirection::Negative => "negative",
                    }
                    .to_string(),
                ),
            ),
        ]),
    }
}

fn target_json(target: &InputBindingTarget) -> Json {
    match target {
        InputBindingTarget::Action(action) => Json::Object(vec![
            ("kind".to_string(), Json::String("action".to_string())),
            ("action".to_string(), Json::String(action_name(*action).to_string())),
        ]),
        InputBindingTarget::Command(text) => Json::Object(vec![
            ("kind".to_string(), Json::String("command".to_string())),
            ("text".to_string(), Json::String(text.clone())),
        ]),
    }
}

fn curve_json(curve: &StickCurve) -> Json {
    match curve {
        StickCurve::Radial {
            deadzone,
            exponent,
            outer_threshold,
        } => Json::Object(vec![
            ("kind".to_string(), Json::String("radial".to_string())),
            ("deadzone".to_string(), Json::Number(*deadzone)),
            ("exponent".to_string(), Json::Number(*exponent)),
            ("outerThreshold".to_string(), Json::Number(*outer_threshold)),
        ]),
        StickCurve::Axial { deadzone, exponent } => Json::Object(vec![
            ("kind".to_string(), Json::String("axial".to_string())),
            ("deadzone".to_string(), Json::Number(*deadzone)),
            ("exponent".to_string(), Json::Number(*exponent)),
        ]),
    }
}

fn gyro_json(tuning: &GyroTuning) -> Json {
    Json::Object(vec![
        ("enabled".to_string(), Json::Bool(tuning.enabled)),
        (
            "yawAxis".to_string(),
            Json::String(
                match tuning.yaw_axis {
                    GyroYawAxis::Y => "y",
                    GyroYawAxis::Z => "z",
                }
                .to_string(),
            ),
        ),
        ("yawSensitivity".to_string(), Json::Number(tuning.yaw_sensitivity)),
        ("pitchSensitivity".to_string(), Json::Number(tuning.pitch_sensitivity)),
    ])
}

fn gamepad_json(tuning: &GamepadTuning) -> Json {
    Json::Object(vec![
        ("move".to_string(), curve_json(&tuning.move_curve)),
        ("look".to_string(), curve_json(&tuning.look_curve)),
        ("swapSticks".to_string(), Json::Bool(tuning.swap_sticks)),
        (
            "yawDegreesPerSecond".to_string(),
            Json::Number(tuning.yaw_degrees_per_second),
        ),
        (
            "pitchDegreesPerSecond".to_string(),
            Json::Number(tuning.pitch_degrees_per_second),
        ),
        ("invertPitch".to_string(), Json::Bool(tuning.invert_pitch)),
        (
            "forwardSensitivity".to_string(),
            Json::Number(tuning.forward_sensitivity),
        ),
        ("sideSensitivity".to_string(), Json::Number(tuning.side_sensitivity)),
        ("triggerThreshold".to_string(), Json::Number(tuning.trigger_threshold)),
        ("gyro".to_string(), gyro_json(&tuning.gyro)),
    ])
}

fn mouse_json(tuning: &MouseTuning) -> Json {
    Json::Object(vec![
        ("sensitivity".to_string(), Json::Number(tuning.sensitivity)),
        ("acceleration".to_string(), Json::Number(tuning.acceleration)),
        ("filter".to_string(), Json::Bool(tuning.filter)),
        ("yaw".to_string(), Json::Number(tuning.yaw)),
        ("pitch".to_string(), Json::Number(tuning.pitch)),
        ("side".to_string(), Json::Number(tuning.side)),
        ("forward".to_string(), Json::Number(tuning.forward)),
        ("lookSpring".to_string(), Json::Bool(tuning.look_spring)),
        ("lookStrafe".to_string(), Json::Bool(tuning.look_strafe)),
        ("freeLook".to_string(), Json::Bool(tuning.free_look)),
        ("invertPitch".to_string(), Json::Bool(tuning.invert_pitch)),
    ])
}

fn controller_json(selection: &ControllerSelection) -> Json {
    match selection {
        ControllerSelection::Automatic => {
            Json::Object(vec![("kind".to_string(), Json::String("automatic".to_string()))])
        }
        ControllerSelection::None => Json::Object(vec![("kind".to_string(), Json::String("none".to_string()))]),
        ControllerSelection::Device { guid, ordinal } => Json::Object(vec![
            ("kind".to_string(), Json::String("device".to_string())),
            ("guid".to_string(), Json::String(guid.clone())),
            ("ordinal".to_string(), Json::Number(f64::from(*ordinal))),
        ]),
        ControllerSelection::Serial { guid, serial } => Json::Object(vec![
            ("kind".to_string(), Json::String("serial".to_string())),
            ("guid".to_string(), Json::String(guid.clone())),
            ("serial".to_string(), Json::String(serial.clone())),
        ]),
    }
}

/// Render a seat settings document.
#[must_use]
pub fn seat_settings_json(settings: &SeatSettings) -> Json {
    let mut members = vec![
        ("version".to_string(), Json::Number(1.0)),
        (
            "bindings".to_string(),
            Json::Array(
                settings
                    .bindings
                    .iter()
                    .map(|binding| {
                        Json::Object(vec![
                            ("input".to_string(), physical_json(&binding.input)),
                            ("target".to_string(), target_json(&binding.target)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("gamepad".to_string(), gamepad_json(&settings.gamepad)),
        ("mouse".to_string(), mouse_json(&settings.mouse)),
        (
            "history".to_string(),
            Json::Array(settings.history.iter().map(|line| Json::String(line.clone())).collect()),
        ),
        ("rumble".to_string(), Json::Bool(settings.rumble)),
        ("rumbleStrength".to_string(), Json::Number(settings.rumble_strength)),
        ("controller".to_string(), controller_json(&settings.controller)),
    ];
    if let Some(always_run) = settings.always_run {
        members.push(("alwaysRun".to_string(), Json::Bool(always_run)));
    }
    Json::Object(members)
}

/// Render a gyro profile document.
#[must_use]
pub fn gyro_profile_json(profile: &GyroProfile) -> Json {
    let identity = match &profile.identity {
        GyroProfileIdentity::Seat => Json::Object(vec![("kind".to_string(), Json::String("seat".to_string()))]),
        GyroProfileIdentity::Device { guid, serial } => Json::Object(vec![
            ("kind".to_string(), Json::String("device".to_string())),
            ("guid".to_string(), Json::String(guid.clone())),
            ("serial".to_string(), Json::String(serial.clone())),
        ]),
    };
    Json::Object(vec![
        ("version".to_string(), Json::Number(1.0)),
        ("identity".to_string(), identity),
        ("tuning".to_string(), gyro_json(&profile.tuning)),
    ])
}

/// Render host directories as a settings document.
#[must_use]
pub fn host_directories_json(directories: &HostDirectories) -> Json {
    let field = |value: &Option<String>| value.clone().map_or(Json::Null, Json::String);
    Json::Object(vec![
        ("version".to_string(), Json::Number(1.0)),
        ("corpusRoot".to_string(), field(&directories.corpus_root)),
        ("userContentRoot".to_string(), field(&directories.user_content_root)),
    ])
}

/// Resolve `name` under `root`, rejecting escapes of the writable directory.
pub fn settings_path(root: &Path, name: &str) -> Result<PathBuf, SettingsError> {
    if name.is_empty() || name.contains('\0') {
        return Err(SettingsError::BadPath(
            "Settings path escapes the writable directory".to_string(),
        ));
    }
    let mut cleaned = PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => cleaned.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(SettingsError::BadPath(
                    "Settings path escapes the writable directory".to_string(),
                ));
            }
        }
    }
    if cleaned.as_os_str().is_empty() {
        return Err(SettingsError::BadPath(
            "Settings path escapes the writable directory".to_string(),
        ));
    }
    Ok(root.join(cleaned))
}

static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `contents` atomically: temp file plus rename (donor `writeAtomic`).
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let counter = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = format!(
        "{}.{}.{counter}.tmp",
        path.as_os_str().to_string_lossy(),
        std::process::id()
    );
    let temporary = PathBuf::from(temporary);
    let result = (|| -> Result<(), SettingsError> {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        use std::io::Write;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

/// File store rooted at the writable directory (donor `ConfigStore`).
#[derive(Debug, Clone)]
pub struct ConfigStore {
    /// Writable root.
    pub root: PathBuf,
}

impl ConfigStore {
    /// Open a store at `root`.
    #[must_use]
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Load a gyro profile, or [`None`] when the file is absent.
    pub fn load_gyro(&self, name: &str) -> Result<Option<GyroProfile>, SettingsError> {
        let Some(text) = self.load_text(name)? else {
            return Ok(None);
        };
        if text.len() > MAX_GYRO_BYTES {
            return Err(SettingsError::BadValue("Gyro settings file is too large".to_string()));
        }
        parse_gyro_profile(&parse_json(&text)?).map(Some)
    }

    /// Save a gyro profile.
    pub fn save_gyro(&self, name: &str, profile: &GyroProfile) -> Result<(), SettingsError> {
        let validated = parse_gyro_profile(&gyro_profile_json(profile))?;
        self.dump(name, &format!("{}\n", stringify(&gyro_profile_json(&validated))))
    }

    /// Save input routing.
    pub fn save_input_routing(&self, name: &str, keyboard_seat: Option<u32>) -> Result<(), SettingsError> {
        let seat = keyboard_seat.map_or(Json::Null, |seat| Json::Number(f64::from(seat)));
        self.dump(
            name,
            &format!(
                "{}\n",
                stringify(&Json::Object(vec![
                    ("version".to_string(), Json::Number(1.0)),
                    ("keyboardSeat".to_string(), seat),
                ]))
            ),
        )
    }

    /// Load input routing, or [`None`] when the file is absent.
    pub fn load_input_routing(&self, name: &str) -> Result<Option<InputRouting>, SettingsError> {
        let Some(text) = self.load_text(name)? else {
            return Ok(None);
        };
        parse_input_routing(&parse_json(&text)?).map(Some)
    }

    /// Load host directories, or [`None`] when the file is absent.
    pub fn load_directories(&self) -> Result<Option<HostDirectories>, SettingsError> {
        let Some(text) = self.load_text("directories.json")? else {
            return Ok(None);
        };
        parse_host_directories(&parse_json(&text)?).map(Some)
    }

    /// Save host directories.
    pub fn save_directories(&self, directories: &HostDirectories) -> Result<(), SettingsError> {
        let validated = parse_host_directories(&host_directories_json(directories))?;
        self.dump(
            "directories.json",
            &format!("{}\n", stringify(&host_directories_json(&validated))),
        )
    }

    /// Save seat settings (pretty-printed, donor `saveSeat`).
    pub fn save_seat(&self, name: &str, settings: &SeatSettings) -> Result<(), SettingsError> {
        let validated = parse_seat_settings(&seat_settings_json(settings))?;
        let path = settings_path(&self.root, name)?;
        write_atomic(
            &path,
            format!("{}\n", stringify_pretty(&seat_settings_json(&validated))).as_bytes(),
        )
    }

    /// Load seat settings, or [`None`] when the file is absent.
    pub fn load_seat(&self, name: &str) -> Result<Option<SeatSettings>, SettingsError> {
        let Some(text) = self.load_text(name)? else {
            return Ok(None);
        };
        parse_seat_settings(&parse_json(&text)?).map(Some)
    }

    /// Save cvar archives plus binding lines (donor `saveCvars`).
    pub fn save_cvars(&self, name: &str, cvars: &CvarRegistry, bindings: &[String]) -> Result<(), SettingsError> {
        let mut lines = bindings.to_vec();
        lines.extend(cvars.archive_commands(&|_| true));
        let path = settings_path(&self.root, name)?;
        write_atomic(
            &path,
            format!("// Generated by quake-typescript\n{}\n", lines.join("\n")).as_bytes(),
        )
    }

    /// Execute a config file as console text (donor `execute`).
    pub fn execute(
        &self,
        name: &str,
        commands: &mut ConsoleCommands,
        dialect: Dialect,
        cvars: &mut CvarRegistry,
        services: &mut dyn ConsoleCommandServices,
    ) -> Result<(), SettingsError> {
        let path = settings_path(&self.root, name)?;
        let text = std::fs::read_to_string(&path)?;
        commands
            .execute(
                &format!("{text}\n"),
                dialect,
                qa_core::cmd::TextMode::Source,
                cvars,
                services,
            )
            .map_err(SettingsError::from)
    }

    /// Load raw text, or [`None`] when the file is absent.
    pub fn load_text(&self, name: &str) -> Result<Option<String>, SettingsError> {
        let path = settings_path(&self.root, name)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(SettingsError::from(error)),
        }
    }

    /// Atomically write raw text (donor `dump`).
    pub fn dump(&self, name: &str, contents: &str) -> Result<(), SettingsError> {
        let path = settings_path(&self.root, name)?;
        write_atomic(&path, contents.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> GamepadTuning {
        GamepadTuning {
            move_curve: StickCurve::Radial {
                deadzone: 0.1,
                exponent: 1.0,
                outer_threshold: 0.1,
            },
            look_curve: StickCurve::Axial {
                deadzone: 0.2,
                exponent: 2.0,
            },
            swap_sticks: false,
            yaw_degrees_per_second: 180.0,
            pitch_degrees_per_second: 90.0,
            invert_pitch: false,
            forward_sensitivity: 1.0,
            side_sensitivity: 1.0,
            trigger_threshold: 0.5,
            gyro: GyroTuning {
                enabled: true,
                yaw_axis: GyroYawAxis::Y,
                yaw_sensitivity: 1.0,
                pitch_sensitivity: 1.0,
            },
        }
    }

    fn mouse_tuning() -> MouseTuning {
        MouseTuning {
            sensitivity: 3.0,
            acceleration: 0.0,
            filter: false,
            yaw: 0.022,
            pitch: 0.022,
            side: 1.0,
            forward: 1.0,
            look_spring: false,
            look_strafe: false,
            free_look: true,
            invert_pitch: false,
        }
    }

    fn seat() -> SeatSettings {
        SeatSettings {
            version: SETTINGS_VERSION,
            bindings: vec![InputBinding {
                input: PhysicalInput::Key(32),
                target: InputBindingTarget::Action(InputAction::Jump),
            }],
            gamepad: tuning(),
            mouse: mouse_tuning(),
            always_run: Some(true),
            history: vec!["status".to_string()],
            rumble: true,
            rumble_strength: 0.5,
            controller: ControllerSelection::Automatic,
        }
    }

    #[test]
    fn parses_and_round_trips_seat_settings() {
        let settings = seat();
        let parsed = parse_seat_settings(&seat_settings_json(&settings)).unwrap();
        assert_eq!(parsed, settings);
        assert_eq!(
            parse_seat_settings_text("{\"version\":2,\"bindings\":[]}")
                .unwrap_err()
                .to_string(),
            "Unsupported seat settings document"
        );
        let mut bad = seat_settings_json(&settings);
        if let Json::Object(members) = &mut bad {
            members.retain(|(name, _)| name != "rumbleStrength");
        }
        assert_eq!(parse_seat_settings(&bad).unwrap().rumble_strength, 1.0);
    }

    #[test]
    fn rejects_bad_tuning_and_identity() {
        let mut bad = tuning();
        bad.trigger_threshold = 2.0;
        assert!(validate_gamepad_tuning(&bad).is_err());
        bad = tuning();
        bad.move_curve = StickCurve::Radial {
            deadzone: 0.9,
            exponent: 1.0,
            outer_threshold: 0.2,
        };
        assert!(validate_gamepad_tuning(&bad).is_err());
        let profile = GyroProfile {
            version: SETTINGS_VERSION,
            identity: GyroProfileIdentity::Device {
                guid: "ABCDEF0123456789abcdef0123456789".to_string(),
                serial: "pad".to_string(),
            },
            tuning: tuning().gyro,
        };
        assert!(parse_gyro_profile(&gyro_profile_json(&profile)).is_err());
    }

    #[test]
    fn guards_paths_and_writes_atomically() {
        let root = std::env::temp_dir().join(format!("qa-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = ConfigStore::new(root.clone());
        assert!(settings_path(&root, "../escape").is_err());
        assert!(settings_path(&root, "").is_err());
        store.save_seat("seat.json", &seat()).unwrap();
        assert_eq!(store.load_seat("seat.json").unwrap().unwrap(), seat());
        assert!(store.load_seat("missing.json").unwrap().is_none());
        store.save_input_routing("routing.json", Some(2)).unwrap();
        assert_eq!(
            store.load_input_routing("routing.json").unwrap().unwrap(),
            InputRouting { keyboard_seat: Some(2) }
        );
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        cvars
            .register("volume", "0.7", qa_core::cvar::q2_flags::ARCHIVE)
            .unwrap();
        store
            .save_cvars("qa.cfg", &cvars, &["bind x +attack".to_string()])
            .unwrap();
        let text = store.load_text("qa.cfg").unwrap().unwrap();
        assert!(text.starts_with("// Generated by quake-typescript\n"));
        assert!(text.contains("bind x +attack"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn host_directories_round_trip_through_store() {
        let root = std::env::temp_dir().join(format!("qa-settings-dirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = ConfigStore::new(root.clone());
        assert!(store.load_directories().unwrap().is_none());
        let directories = HostDirectories {
            corpus_root: Some("/home/operator/retro".to_string()),
            user_content_root: None,
        };
        store.save_directories(&directories).unwrap();
        assert_eq!(store.load_directories().unwrap().unwrap(), directories);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn host_directories_reject_bad_documents() {
        let bad_version = parse_json(r#"{"version":2,"corpusRoot":null,"userContentRoot":null}"#).unwrap();
        assert!(parse_host_directories(&bad_version).is_err());
        let blank = parse_json(r#"{"version":1,"corpusRoot":"  ","userContentRoot":null}"#).unwrap();
        assert!(parse_host_directories(&blank).is_err());
        let wrong_type = parse_json(r#"{"version":1,"corpusRoot":7,"userContentRoot":null}"#).unwrap();
        assert!(parse_host_directories(&wrong_type).is_err());
        let sparse = parse_json(r#"{"version":1}"#).unwrap();
        assert_eq!(parse_host_directories(&sparse).unwrap(), HostDirectories::default());
    }
}
