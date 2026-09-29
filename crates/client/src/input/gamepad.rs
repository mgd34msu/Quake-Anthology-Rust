//! Gamepad sticks, triggers, and gyro calibration.
//!
//! Donor provenance: `src/input/gamepad.ts` (Q1 Ironwail-style
//! radial curve, Q2 axial curve, gyro calibration policy).

use std::collections::HashMap;

use qa_core::math::{vec2, vec3, Vec2, Vec3};
use thiserror::Error;

use super::{AxisDirection, ControllerAxis, PhysicalInput};

/// Gamepad tuning or sample error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GamepadError {
    /// Deadzone or response curve is out of range.
    #[error("Invalid gamepad deadzone or response curve")]
    BadCurve,
    /// A sensitivity is not finite.
    #[error("Invalid gamepad sensitivity")]
    BadSensitivity,
    /// Trigger threshold is outside `[0, 1]`.
    #[error("Invalid trigger threshold")]
    BadTrigger,
    /// Gyro sample or timestamp is invalid.
    #[error("Invalid gyro sample or timestamp")]
    BadGyro,
}

/// Stick response curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StickCurve {
    /// Q1 Ironwail-style radial curve.
    Radial {
        /// Deadzone radius.
        deadzone: f64,
        /// Outer threshold.
        outer_threshold: f64,
        /// Response exponent.
        exponent: f64,
    },
    /// Q2 axial curve.
    Axial {
        /// Per-axis deadzone.
        deadzone: f64,
        /// Response exponent.
        exponent: f64,
    },
}

/// Gyro tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroTuning {
    /// Gyro aiming enabled.
    pub enabled: bool,
    /// Yaw sensitivity.
    pub yaw_sensitivity: f64,
    /// Pitch sensitivity.
    pub pitch_sensitivity: f64,
    /// Gyro axis used for yaw.
    pub yaw_axis_y: bool,
}

/// Gamepad tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadTuning {
    /// Move-stick curve.
    pub stick_move: StickCurve,
    /// Look-stick curve.
    pub stick_look: StickCurve,
    /// Swap move and look sticks.
    pub swap_sticks: bool,
    /// Look yaw rate in degrees per second.
    pub yaw_degrees_per_second: f64,
    /// Look pitch rate in degrees per second.
    pub pitch_degrees_per_second: f64,
    /// Invert look pitch.
    pub invert_pitch: bool,
    /// Move forward sensitivity.
    pub forward_sensitivity: f64,
    /// Move side sensitivity.
    pub side_sensitivity: f64,
    /// Trigger digital threshold.
    pub trigger_threshold: f64,
    /// Gyro tuning.
    pub gyro: GyroTuning,
}

/// Default tuning: radial 0.175/0.02/exponent 2, 240/130 dps.
#[must_use]
pub const fn default_gamepad_tuning() -> GamepadTuning {
    GamepadTuning {
        stick_move: StickCurve::Radial {
            deadzone: 0.175,
            outer_threshold: 0.02,
            exponent: 2.0,
        },
        stick_look: StickCurve::Radial {
            deadzone: 0.175,
            outer_threshold: 0.02,
            exponent: 2.0,
        },
        swap_sticks: false,
        yaw_degrees_per_second: 240.0,
        pitch_degrees_per_second: 130.0,
        invert_pitch: false,
        forward_sensitivity: 1.0,
        side_sensitivity: 1.0,
        trigger_threshold: 0.2,
        gyro: GyroTuning {
            enabled: false,
            yaw_sensitivity: 1.0,
            pitch_sensitivity: 1.0,
            yaw_axis_y: true,
        },
    }
}

/// Validate tuning, mirroring the donor's `RangeError`s.
pub fn validate_gamepad_tuning(tuning: &GamepadTuning) -> Result<(), GamepadError> {
    for curve in [&tuning.stick_move, &tuning.stick_look] {
        let (deadzone, exponent, outer) = match curve {
            StickCurve::Radial {
                deadzone,
                outer_threshold,
                exponent,
            } => (*deadzone, *exponent, Some(*outer_threshold)),
            StickCurve::Axial { deadzone, exponent } => (*deadzone, *exponent, None),
        };
        if !deadzone.is_finite() || !(0.0..1.0).contains(&deadzone) || !exponent.is_finite() || exponent <= 0.0 {
            return Err(GamepadError::BadCurve);
        }
        if let Some(outer) = outer {
            if !outer.is_finite() || outer < 0.0 || deadzone + outer >= 1.0 {
                return Err(GamepadError::BadCurve);
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
            return Err(GamepadError::BadSensitivity);
        }
    }
    if !tuning.trigger_threshold.is_finite() || !(0.0..=1.0).contains(&tuning.trigger_threshold) {
        return Err(GamepadError::BadTrigger);
    }
    Ok(())
}

/// Apply a stick response curve.
#[must_use]
pub fn apply_stick_curve(axis: Vec2, curve: &StickCurve) -> Vec2 {
    match *curve {
        StickCurve::Axial { deadzone, exponent } => {
            let apply = |value: f32| {
                let shaped = ((f64::from(value).abs() - deadzone) / (1.0 - deadzone)).clamp(0.0, 1.0).powf(exponent);
                f64::from(value.signum()) * shaped
            };
            vec2(apply(axis.x) as f32, apply(axis.y) as f32)
        }
        StickCurve::Radial {
            deadzone,
            outer_threshold,
            exponent,
        } => {
            let magnitude = f64::from(axis.x).hypot(f64::from(axis.y));
            if magnitude <= deadzone {
                return vec2(0.0, 0.0);
            }
            let scale = ((magnitude - deadzone) / (1.0 - deadzone - outer_threshold))
                .min(1.0)
                .powf(exponent)
                / magnitude;
            vec2((f64::from(axis.x) * scale) as f32, (f64::from(axis.y) * scale) as f32)
        }
    }
}

const AXIS_NAMES: [ControllerAxis; 6] = [
    ControllerAxis::LeftX,
    ControllerAxis::LeftY,
    ControllerAxis::RightX,
    ControllerAxis::RightY,
    ControllerAxis::LeftTrigger,
    ControllerAxis::RightTrigger,
];

/// Map a controller axis index to its name.
#[must_use]
pub const fn controller_axis_name(axis: u8) -> Option<ControllerAxis> {
    if (axis as usize) < AXIS_NAMES.len() {
        Some(AXIS_NAMES[axis as usize])
    } else {
        None
    }
}

/// Normalize a raw axis value; triggers clamp at zero.
#[must_use]
pub fn normalized_controller_axis(axis: ControllerAxis, raw: i16) -> f64 {
    let low = match axis {
        ControllerAxis::LeftTrigger | ControllerAxis::RightTrigger => 0.0,
        _ => -1.0,
    };
    (f64::from(raw) / 32767.0).clamp(low, 1.0)
}

/// Sampled gamepad frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadSample {
    /// Curved move stick.
    pub stick_move: Vec2,
    /// Look contribution in degrees.
    pub look_degrees: Vec2,
}

/// Gyro calibration state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GyroCalibrationState {
    /// No bias captured.
    Idle,
    /// Capturing a still controller.
    Calibrating {
        /// Completion fraction.
        progress: f64,
        /// Samples collected.
        samples: u32,
    },
    /// Bias captured.
    Ready {
        /// Gyro bias in radians per second.
        bias: Vec3,
    },
}

#[derive(Debug, Clone, Copy)]
struct GyroCapture {
    start: f64,
    last: f64,
    samples: u32,
    mean: [f64; 3],
    deviation: [f64; 3],
}

const GYRO_CALIBRATION_DURATION_MS: f64 = 2000.0;
const GYRO_CALIBRATION_SAMPLES: u32 = 64;

/// Per-seat gamepad state: sticks, triggers, and gyro.
#[derive(Debug, Clone)]
pub struct GamepadInput {
    axes: HashMap<ControllerAxis, f64>,
    preview_axes: HashMap<ControllerAxis, f64>,
    gyro_sample: Option<[f64; 3]>,
    gyro_bias: Option<[f64; 3]>,
    calibrating: bool,
    capture: Option<GyroCapture>,
    /// Active tuning.
    pub tuning: GamepadTuning,
}

impl GamepadInput {
    /// Gamepad with default tuning.
    #[must_use]
    pub fn new() -> Self {
        let tuning = default_gamepad_tuning();
        debug_assert!(validate_gamepad_tuning(&tuning).is_ok());
        Self {
            axes: HashMap::new(),
            preview_axes: HashMap::new(),
            gyro_sample: None,
            gyro_bias: None,
            calibrating: false,
            capture: None,
            tuning,
        }
    }

    /// Gamepad with explicit tuning.
    pub fn with_tuning(tuning: GamepadTuning) -> Result<Self, GamepadError> {
        validate_gamepad_tuning(&tuning)?;
        Ok(Self {
            axes: HashMap::new(),
            preview_axes: HashMap::new(),
            gyro_sample: None,
            gyro_bias: None,
            calibrating: false,
            capture: None,
            tuning,
        })
    }

    fn axis_value(&self, axis: ControllerAxis) -> f64 {
        self.axes.get(&axis).copied().unwrap_or(0.0)
    }

    fn preview_value(&self, axis: ControllerAxis) -> f64 {
        self.preview_axes.get(&axis).copied().unwrap_or(0.0)
    }

    /// Record an axis value for menus as well as gameplay.
    pub fn axis(&mut self, axis: ControllerAxis, value: f64) {
        self.preview_axis(axis, value);
        self.axes.insert(axis, value.clamp(-1.0, 1.0));
    }

    /// Record an axis value for menus only.
    pub fn preview_axis(&mut self, axis: ControllerAxis, value: f64) {
        self.preview_axes.insert(axis, value.clamp(-1.0, 1.0));
    }

    /// Raw and curved sticks for menus.
    pub fn preview(&self) -> GamepadPreview {
        let left = vec2(self.preview_value(ControllerAxis::LeftX) as f32, self.preview_value(ControllerAxis::LeftY) as f32);
        let right = vec2(
            self.preview_value(ControllerAxis::RightX) as f32,
            self.preview_value(ControllerAxis::RightY) as f32,
        );
        let (stick_move, look) = if self.tuning.swap_sticks { (right, left) } else { (left, right) };
        GamepadPreview {
            move_raw: stick_move,
            move_curved: apply_stick_curve(stick_move, &self.tuning.stick_move),
            look_raw: look,
            look_curved: apply_stick_curve(look, &self.tuning.stick_look),
        }
    }

    /// Current calibration state.
    pub fn gyro_calibration(&self) -> GyroCalibrationState {
        if self.calibrating {
            let (progress, samples) = self.capture.map_or((0.0, 0), |capture| {
                let progress = ((capture.last - capture.start) / GYRO_CALIBRATION_DURATION_MS)
                    .min(f64::from(capture.samples) / f64::from(GYRO_CALIBRATION_SAMPLES))
                    .min(1.0);
                (progress, capture.samples)
            });
            GyroCalibrationState::Calibrating { progress, samples }
        } else if let Some(bias) = self.gyro_bias {
            GyroCalibrationState::Ready {
                bias: vec3(bias[0] as f32, bias[1] as f32, bias[2] as f32),
            }
        } else {
            GyroCalibrationState::Idle
        }
    }

    /// Start capturing a still-controller bias.
    pub fn begin_gyro_calibration(&mut self) {
        self.calibrating = true;
        self.capture = None;
        self.gyro_sample = None;
    }

    /// Cancel calibration without keeping a bias.
    pub fn cancel_gyro_calibration(&mut self) {
        self.calibrating = false;
        self.capture = None;
        self.gyro_sample = None;
    }

    /// Cancel calibration and drop the bias.
    pub fn reset_gyro_calibration(&mut self) {
        self.cancel_gyro_calibration();
        self.gyro_bias = None;
    }

    /// Feed a gyro sample in radians per second.
    pub fn gyro(&mut self, value: Vec3, time_ms: f64, aiming: bool) -> Result<(), GamepadError> {
        let sample = [f64::from(value.x), f64::from(value.y), f64::from(value.z)];
        if !sample.iter().all(|value| value.is_finite()) || !time_ms.is_finite() || time_ms < 0.0 {
            return Err(GamepadError::BadGyro);
        }
        self.gyro_sample = aiming.then_some(sample);
        if !self.calibrating {
            return Ok(());
        }
        self.gyro_sample = None;
        if sample[0].hypot(sample[1]).hypot(sample[2]) > 0.15 {
            self.capture = None;
            return Ok(());
        }
        let mut previous = self.capture;
        if let Some(capture) = previous {
            if time_ms == capture.last {
                return Ok(());
            }
            if time_ms < capture.last || time_ms - capture.last > 250.0 {
                previous = None;
            }
        }
        let Some(previous) = previous else {
            self.capture = Some(GyroCapture {
                start: time_ms,
                last: time_ms,
                samples: 1,
                mean: sample,
                deviation: [0.0, 0.0, 0.0],
            });
            return Ok(());
        };
        let samples = previous.samples + 1;
        let mean = [
            previous.mean[0] + (sample[0] - previous.mean[0]) / f64::from(samples),
            previous.mean[1] + (sample[1] - previous.mean[1]) / f64::from(samples),
            previous.mean[2] + (sample[2] - previous.mean[2]) / f64::from(samples),
        ];
        let deviation = [
            previous.deviation[0] + (sample[0] - previous.mean[0]) * (sample[0] - mean[0]),
            previous.deviation[1] + (sample[1] - previous.mean[1]) * (sample[1] - mean[1]),
            previous.deviation[2] + (sample[2] - previous.mean[2]) * (sample[2] - mean[2]),
        ];
        if deviation[0].max(deviation[1]).max(deviation[2]) / f64::from(samples - 1) > 0.01f64.powi(2) {
            self.capture = None;
            return Ok(());
        }
        self.capture = Some(GyroCapture {
            start: previous.start,
            last: time_ms,
            samples,
            mean,
            deviation,
        });
        if time_ms - previous.start >= GYRO_CALIBRATION_DURATION_MS && samples >= GYRO_CALIBRATION_SAMPLES {
            self.gyro_bias = Some(mean);
            self.cancel_gyro_calibration();
        }
        Ok(())
    }

    /// Sample curved sticks and look degrees for a frame.
    pub fn sample(&self, frame_ms: f64) -> GamepadSample {
        let left = vec2(self.axis_value(ControllerAxis::LeftX) as f32, self.axis_value(ControllerAxis::LeftY) as f32);
        let right = vec2(
            self.axis_value(ControllerAxis::RightX) as f32,
            self.axis_value(ControllerAxis::RightY) as f32,
        );
        let (raw_move, raw_look) = if self.tuning.swap_sticks { (right, left) } else { (left, right) };
        let stick_move = apply_stick_curve(raw_move, &self.tuning.stick_move);
        let look = apply_stick_curve(raw_look, &self.tuning.stick_look);
        let seconds = frame_ms / 1000.0;
        let gyro = &self.tuning.gyro;
        let scale = if gyro.enabled && !self.calibrating && self.gyro_sample.is_some() {
            180.0 / std::f64::consts::PI * seconds
        } else {
            0.0
        };
        let yaw_index = usize::from(!gyro.yaw_axis_y) + 1;
        let yaw = self.gyro_sample.map_or(0.0, |sample| sample[yaw_index] - self.gyro_bias.map_or(0.0, |bias| bias[yaw_index]));
        let pitch = self.gyro_sample.map_or(0.0, |sample| sample[0] - self.gyro_bias.map_or(0.0, |bias| bias[0]));
        let look_x = f64::from(look.x) * self.tuning.yaw_degrees_per_second * seconds - yaw * gyro.yaw_sensitivity * scale;
        let look_y = (f64::from(look.y) * self.tuning.pitch_degrees_per_second * seconds
            - pitch * gyro.pitch_sensitivity * scale)
            * if self.tuning.invert_pitch { -1.0 } else { 1.0 };
        GamepadSample {
            stick_move: vec2(
                (f64::from(stick_move.x) * self.tuning.side_sensitivity) as f32,
                (-f64::from(stick_move.y) * self.tuning.forward_sensitivity) as f32,
            ),
            look_degrees: vec2(look_x as f32, look_y as f32),
        }
    }

    /// Clear sticks, previews, and calibration.
    pub fn clear(&mut self) {
        self.preview_axes.clear();
        self.axes.clear();
        self.cancel_gyro_calibration();
    }
}

impl Default for GamepadInput {
    fn default() -> Self {
        Self::new()
    }
}

/// Raw and curved sticks for menus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadPreview {
    /// Raw move stick.
    pub move_raw: Vec2,
    /// Curved move stick.
    pub move_curved: Vec2,
    /// Raw look stick.
    pub look_raw: Vec2,
    /// Curved look stick.
    pub look_curved: Vec2,
}

/// Digital inputs for one axis deflection.
#[must_use]
pub fn axis_digitals(device: i32, axis: ControllerAxis) -> [PhysicalInput; 2] {
    [
        PhysicalInput::ControllerAxis {
            device,
            axis,
            direction: AxisDirection::Negative,
        },
        PhysicalInput::ControllerAxis {
            device,
            axis,
            direction: AxisDirection::Positive,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_shape_sticks() {
        let radial = StickCurve::Radial {
            deadzone: 0.175,
            outer_threshold: 0.02,
            exponent: 2.0,
        };
        assert_eq!(apply_stick_curve(vec2(0.1, 0.0), &radial), vec2(0.0, 0.0));
        let full = apply_stick_curve(vec2(1.0, 0.0), &radial);
        assert!((f64::from(full.x) - 1.0).abs() < 1e-6);
        let axial = StickCurve::Axial {
            deadzone: 0.2,
            exponent: 1.0,
        };
        let shaped = apply_stick_curve(vec2(0.6, -0.6), &axial);
        assert!((f64::from(shaped.x) - 0.5).abs() < 1e-6);
        assert!((f64::from(shaped.y) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn tuning_validation_rejects_bad_ranges() {
        let mut tuning = default_gamepad_tuning();
        assert!(validate_gamepad_tuning(&tuning).is_ok());
        tuning.trigger_threshold = 2.0;
        assert_eq!(validate_gamepad_tuning(&tuning), Err(GamepadError::BadTrigger));
        tuning.trigger_threshold = 0.2;
        tuning.stick_move = StickCurve::Axial {
            deadzone: 1.0,
            exponent: 1.0,
        };
        assert_eq!(validate_gamepad_tuning(&tuning), Err(GamepadError::BadCurve));
    }

    #[test]
    fn axes_map_and_normalize() {
        assert_eq!(controller_axis_name(0), Some(ControllerAxis::LeftX));
        assert_eq!(controller_axis_name(5), Some(ControllerAxis::RightTrigger));
        assert_eq!(controller_axis_name(6), None);
        assert_eq!(normalized_controller_axis(ControllerAxis::LeftX, -32767), -1.0);
        assert_eq!(normalized_controller_axis(ControllerAxis::LeftTrigger, -32767), 0.0);
        assert!((normalized_controller_axis(ControllerAxis::RightY, 16384) - 0.500015).abs() < 1e-6);
    }

    #[test]
    fn sampling_curves_and_scales() {
        let mut pad = GamepadInput::new();
        pad.axis(ControllerAxis::LeftX, 1.0);
        pad.axis(ControllerAxis::RightX, 0.5);
        let sample = pad.sample(16.0);
        assert!((f64::from(sample.stick_move.x) - 1.0).abs() < 1e-4);
        assert_eq!(sample.stick_move.y, 0.0);
        let expected = f64::from(apply_stick_curve(vec2(0.5, 0.0), &pad.tuning.stick_look).x) * 240.0 * 0.016;
        assert!((f64::from(sample.look_degrees.x) - expected).abs() < 1e-4);
        let preview = pad.preview();
        assert_eq!(preview.move_raw, vec2(1.0, 0.0));
        pad.clear();
        assert_eq!(pad.sample(16.0).stick_move, vec2(0.0, 0.0));
    }

    #[test]
    fn gyro_calibration_captures_still_bias() {
        let mut pad = GamepadInput::new();
        pad.tuning.gyro.enabled = true;
        pad.begin_gyro_calibration();
        for index in 0..70 {
            pad.gyro(vec3(0.01, -0.02, 0.005), f64::from(index) * 100.0, true).unwrap();
        }
        match pad.gyro_calibration() {
            GyroCalibrationState::Ready { bias } => {
                assert!((f64::from(bias.x) - 0.01).abs() < 1e-6);
                assert!((f64::from(bias.y) + 0.02).abs() < 1e-6);
            }
            state => panic!("expected ready bias, got {state:?}"),
        }
        pad.reset_gyro_calibration();
        assert_eq!(pad.gyro_calibration(), GyroCalibrationState::Idle);
        assert!(pad.gyro(vec3(f32::NAN, 0.0, 0.0), 0.0, true).is_err());
    }
}
