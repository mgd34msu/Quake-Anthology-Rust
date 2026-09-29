//! BNVIB haptics scheduling per seat.
//!
//! Donor provenance: `src/input/haptics.ts` (BNVIB format and motor
//! downmix, scheduling and device ownership per seat).

use std::collections::HashMap;

use qa_core::binary::BinaryReader;
use qa_core::identity::SeatId;
use qa_platform::controller::ControllerOperationResult;
use thiserror::Error;

/// Haptics error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HapticError {
    /// BNVIB metadata size is unsupported.
    #[error("Unsupported BNVIB metadata size {0}")]
    BadMetadataSize(u32),
    /// BNVIB format version is unsupported.
    #[error("Unsupported BNVIB format")]
    BadFormat,
    /// BNVIB sample rate must be positive.
    #[error("BNVIB sample rate must be positive")]
    BadSampleRate,
    /// BNVIB data size is not a multiple of four.
    #[error("BNVIB data size is not a multiple of four")]
    BadDataSize,
    /// BNVIB loop exceeds sample data.
    #[error("BNVIB loop exceeds sample data")]
    BadLoop,
    /// BNVIB data ends early.
    #[error("BNVIB data ends early: {0}")]
    Truncated(String),
    /// Vibration strength must be between 0 and 1.
    #[error("Vibration strength must be between 0 and 1")]
    BadStrength,
}

/// One BNVIB rumble sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BnvibSample {
    /// Low-motor amplitude.
    pub amp_low: u8,
    /// Low-motor frequency byte.
    pub freq_low: u8,
    /// High-motor amplitude.
    pub amp_high: u8,
    /// High-motor frequency byte.
    pub freq_high: u8,
}

/// BNVIB loop region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BnvibLoop {
    /// First loop sample.
    pub start_sample: u32,
    /// One past the last loop sample.
    pub end_sample: u32,
    /// Silent samples between loops.
    pub interval_samples: u32,
}

/// Decoded BNVIB pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BnvibPattern {
    /// Sample rate in Hz.
    pub sample_rate_hz: u16,
    /// Samples.
    pub samples: Vec<BnvibSample>,
    /// Loop region, if any.
    pub loop_region: Option<BnvibLoop>,
}

/// Decode a BNVIB pattern.
pub fn parse_bnvib(bytes: &[u8]) -> Result<BnvibPattern, HapticError> {
    let mut reader = BinaryReader::new(bytes, "BNVIB");
    let truncated = |error: qa_core::binary::BinaryError| HapticError::Truncated(error.to_string());
    let size = reader.u32().map_err(truncated)?;
    if size != 4 && size != 12 && size != 16 {
        return Err(HapticError::BadMetadataSize(size));
    }
    if reader.u16().map_err(truncated)? != 3 {
        return Err(HapticError::BadFormat);
    }
    let sample_rate_hz = reader.u16().map_err(truncated)?;
    if sample_rate_hz == 0 {
        return Err(HapticError::BadSampleRate);
    }
    let loop_region = if size == 4 {
        None
    } else {
        Some(BnvibLoop {
            start_sample: reader.u32().map_err(truncated)?,
            end_sample: reader.u32().map_err(truncated)?,
            interval_samples: if size == 16 {
                reader.u32().map_err(truncated)?
            } else {
                0
            },
        })
    };
    let data_size = reader.u32().map_err(truncated)?;
    if data_size % 4 != 0 {
        return Err(HapticError::BadDataSize);
    }
    let mut samples = Vec::new();
    for _ in 0..data_size / 4 {
        samples.push(BnvibSample {
            amp_low: reader.u8().map_err(truncated)?,
            freq_low: reader.u8().map_err(truncated)?,
            amp_high: reader.u8().map_err(truncated)?,
            freq_high: reader.u8().map_err(truncated)?,
        });
    }
    if let Some(region) = &loop_region {
        if region.start_sample >= region.end_sample || u64::from(region.end_sample) > samples.len() as u64 {
            return Err(HapticError::BadLoop);
        }
    }
    Ok(BnvibPattern {
        sample_rate_hz,
        samples,
        loop_region,
    })
}

/// BNVIB frequency byte to Hz.
#[must_use]
pub fn bnvib_frequency(byte: u8) -> f64 {
    10.0 * 2f64.powf(f64::from(byte) / 32.0)
}

/// Tactile pattern path for a sound path.
#[must_use]
pub fn tactile_path_for_sound(sound: &str) -> Option<String> {
    let path = sound.strip_prefix('#').unwrap_or(sound);
    if !path.ends_with(".wav") || path.len() <= 4 {
        return None;
    }
    Some(format!("tactile/{}.bnvib", &path[..path.len() - 4]))
}

/// Motor output sink.
pub trait RumbleSink {
    /// Set motor levels for a duration.
    fn set_motors(&mut self, low: f64, high: f64, duration_ms: u32) -> ControllerOperationResult;
}

/// Scheduler step result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HapticsResult {
    /// A backend operation ran.
    Operation(ControllerOperationResult),
    /// Motor output did not change.
    Unchanged,
}

/// BNVIB playback scheduler over one sink.
pub struct BnvibScheduler {
    sink: Box<dyn RumbleSink>,
    playing: Option<(BnvibPattern, f64)>,
    gain: f64,
    last_index: i64,
    last_output: f64,
}

impl BnvibScheduler {
    /// Scheduler over a sink.
    #[must_use]
    pub fn new(sink: Box<dyn RumbleSink>) -> Self {
        Self {
            sink,
            playing: None,
            gain: 1.0,
            last_index: -1,
            last_output: f64::NEG_INFINITY,
        }
    }

    /// Current strength.
    #[must_use]
    pub const fn strength(&self) -> f64 {
        self.gain
    }

    /// Whether a pattern is playing.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.playing.is_some()
    }

    /// Set the strength gain.
    pub fn set_strength(&mut self, value: f64, now_ms: f64) -> Result<HapticsResult, HapticError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(HapticError::BadStrength);
        }
        if value == self.gain {
            return Ok(HapticsResult::Unchanged);
        }
        self.gain = value;
        self.last_index = -1;
        Ok(self.update(now_ms))
    }

    /// Start a pattern.
    pub fn play(&mut self, pattern: BnvibPattern, now_ms: f64) -> HapticsResult {
        self.playing = Some((pattern, now_ms));
        self.last_index = -1;
        self.update(now_ms)
    }

    /// Stop playback and silence the motors.
    pub fn stop(&mut self) -> ControllerOperationResult {
        self.playing = None;
        self.last_index = -1;
        self.sink.set_motors(0.0, 0.0, 0)
    }

    /// Advance playback to now.
    pub fn update(&mut self, now_ms: f64) -> HapticsResult {
        let Some((pattern, started)) = self.playing.clone() else {
            return HapticsResult::Unchanged;
        };
        let period = 1000.0 / f64::from(pattern.sample_rate_hz);
        let mut index = ((now_ms - started) / period).floor() as i64;
        index = index.max(0);
        if index >= pattern.samples.len() as i64 {
            let Some(region) = &pattern.loop_region else {
                return HapticsResult::Operation(self.stop());
            };
            let loop_length = i64::from(region.end_sample - region.start_sample);
            let position = (index - pattern.samples.len() as i64) % (loop_length + i64::from(region.interval_samples));
            index = if position >= loop_length {
                -2
            } else {
                i64::from(region.start_sample) + position
            };
        }
        let hold = (50.0f64).max(period.ceil() + 20.0);
        if index == self.last_index && now_ms - self.last_output < hold / 2.0 {
            return HapticsResult::Unchanged;
        }
        self.last_index = index;
        self.last_output = now_ms;
        if index == -2 {
            return HapticsResult::Operation(self.sink.set_motors(0.0, 0.0, hold as u32));
        }
        let Some(sample) = pattern.samples.get(usize::try_from(index).unwrap_or(usize::MAX)) else {
            return HapticsResult::Operation(self.stop());
        };
        HapticsResult::Operation(self.sink.set_motors(
            f64::from(sample.amp_low) / 255.0 * self.gain,
            f64::from(sample.amp_high) / 255.0 * self.gain,
            hold as u32,
        ))
    }
}

/// Clock for haptics.
pub type HapticsClock = Box<dyn Fn() -> f64>;
/// Controller lookup for a seat.
pub type HapticsController = Box<dyn Fn(&SeatId) -> Option<i32>>;
/// Rumble backend.
pub type HapticsRumble = Box<dyn Fn(i32, f64, f64, u32) -> ControllerOperationResult>;
/// Pattern loader: content and path to bytes.
pub type HapticsLoader = Box<dyn Fn(&str, &str) -> Option<Vec<u8>>>;

/// Per-seat haptics: device tracking, pattern cache, scheduler.
pub struct SeatHaptics {
    scheduler: BnvibScheduler,
    cache: HashMap<String, Option<BnvibPattern>>,
    seat: SeatId,
    controller: HapticsController,
    load: HapticsLoader,
    now: HapticsClock,
    generation: u64,
    device: Option<i32>,
    sink_device: std::rc::Rc<std::cell::RefCell<Option<i32>>>,
    active: bool,
    preference: bool,
    closed: bool,
}

struct SinkRouter {
    device: std::rc::Rc<std::cell::RefCell<Option<i32>>>,
    rumble: HapticsRumble,
}

impl RumbleSink for SinkRouter {
    fn set_motors(&mut self, low: f64, high: f64, duration_ms: u32) -> ControllerOperationResult {
        self.device.borrow().map_or(
            ControllerOperationResult::Disconnected {
                reason: "Seat has no assigned controller".to_string(),
            },
            |device| (self.rumble)(device, low, high, duration_ms),
        )
    }
}

impl SeatHaptics {
    /// Haptics for a seat. The rumble backend routes through the
    /// seat's current controller.
    #[must_use]
    pub fn new(
        seat: SeatId,
        controller: HapticsController,
        load: HapticsLoader,
        now: HapticsClock,
        rumble: HapticsRumble,
    ) -> Self {
        let sink_device = std::rc::Rc::new(std::cell::RefCell::new(None));
        Self {
            scheduler: BnvibScheduler::new(Box::new(SinkRouter {
                device: sink_device.clone(),
                rumble,
            })),
            cache: HashMap::new(),
            seat,
            controller,
            load,
            now,
            generation: 0,
            device: None,
            sink_device,
            active: true,
            preference: true,
            closed: false,
        }
    }

    /// Whether the seat wants haptics.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.preference
    }

    /// Current strength.
    #[must_use]
    pub fn strength(&self) -> f64 {
        self.scheduler.strength()
    }

    /// Set the strength gain.
    pub fn set_strength(&mut self, value: f64) -> Result<HapticsResult, HapticError> {
        let now = (self.now)();
        self.scheduler.set_strength(value, now)
    }

    /// Enable or disable haptics (disabling cancels playback).
    pub fn set_enabled(&mut self, enabled: bool) -> ControllerOperationResult {
        self.preference = enabled;
        if enabled {
            return ControllerOperationResult::Accepted;
        }
        self.cancel()
    }

    /// Seat activity gate (hiding cancels playback).
    pub fn set_active(&mut self, active: bool) {
        if self.active == active {
            return;
        }
        self.active = active;
        if !active {
            self.cancel();
        }
    }

    fn synchronize_device(&mut self) {
        let device = (self.controller)(&self.seat);
        if device == self.device {
            return;
        }
        self.cancel();
        self.device = device;
        *self.sink_device.borrow_mut() = device;
    }

    /// Cancel playback.
    pub fn cancel(&mut self) -> ControllerOperationResult {
        self.generation += 1;
        self.scheduler.stop()
    }

    /// Play the tactile pattern paired with a sound.
    ///
    /// The loader runs synchronously; a corrupt pattern errors like a
    /// rejected donor load.
    pub fn sound(&mut self, content: &str, sound: &str) -> Result<Option<HapticsResult>, HapticError> {
        if self.closed {
            return Ok(None);
        }
        self.synchronize_device();
        if !self.preference || !self.active || self.device.is_none() {
            return Ok(None);
        }
        let path = tactile_path_for_sound(sound.strip_prefix("sound/").unwrap_or(sound));
        let Some(path) = path else {
            return Ok(None);
        };
        let key = format!("{content}/{path}");
        let pattern = match self.cache.get(&key) {
            Some(cached) => cached.clone(),
            None => {
                let bytes = (self.load)(content, &path);
                let pattern = bytes.map(|bytes| parse_bnvib(&bytes)).transpose()?;
                self.cache.insert(key, pattern.clone());
                pattern
            }
        };
        let Some(pattern) = pattern else {
            return Ok(None);
        };
        let now = (self.now)();
        Ok(Some(self.scheduler.play(pattern, now)))
    }

    /// Advance playback to now.
    pub fn update(&mut self) -> HapticsResult {
        if self.closed {
            return HapticsResult::Unchanged;
        }
        self.synchronize_device();
        let now = (self.now)();
        self.scheduler.update(now)
    }

    /// Drop cached patterns after an asset change.
    pub fn invalidate_assets(&mut self) {
        self.cancel();
        self.cache.clear();
    }

    /// Close haptics for the seat.
    pub fn close(&mut self) -> ControllerOperationResult {
        self.closed = true;
        self.cache.clear();
        self.cancel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern_bytes() -> Vec<u8> {
        let mut bytes = vec![4, 0, 0, 0, 3, 0, 60, 0, 8, 0, 0, 0];
        bytes.extend_from_slice(&[255, 0, 128, 0, 0, 0, 0, 0]);
        bytes
    }

    struct FakeSink {
        calls: Vec<(f64, f64, u32)>,
    }

    impl RumbleSink for FakeSink {
        fn set_motors(&mut self, low: f64, high: f64, duration_ms: u32) -> ControllerOperationResult {
            self.calls.push((low, high, duration_ms));
            ControllerOperationResult::Accepted
        }
    }

    #[test]
    fn parses_and_schedules_patterns() {
        let pattern = parse_bnvib(&pattern_bytes()).unwrap();
        assert_eq!(pattern.sample_rate_hz, 60);
        assert_eq!(pattern.samples.len(), 2);
        assert_eq!(pattern.loop_region, None);
        assert!((bnvib_frequency(32) - 20.0).abs() < 1e-9);
        assert_eq!(
            tactile_path_for_sound("#sound/shot.wav"),
            Some("tactile/sound/shot.bnvib".to_string())
        );
        assert_eq!(
            tactile_path_for_sound("shot.wav"),
            Some("tactile/shot.bnvib".to_string())
        );
        assert_eq!(tactile_path_for_sound("shot.ogg"), None);
        assert_eq!(parse_bnvib(&[5, 0, 0, 0]), Err(HapticError::BadMetadataSize(5)));
        let mut scheduler = BnvibScheduler::new(Box::new(FakeSink { calls: Vec::new() }));
        assert!(matches!(scheduler.play(pattern, 0.0), HapticsResult::Operation(_)));
        assert!(scheduler.active());
        assert!(matches!(scheduler.update(1.0), HapticsResult::Unchanged));
        assert!(scheduler.set_strength(2.0, 0.0).is_err());
    }

    #[test]
    fn seat_routes_by_device_and_preference() {
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("test").unwrap();
        let seat = owner.seat(0);
        let bytes = pattern_bytes();
        let mut haptics = SeatHaptics::new(
            seat,
            Box::new(|_| Some(3)),
            Box::new(move |_, _| Some(bytes.clone())),
            Box::new(|| 0.0),
            Box::new(|device, _, _, _| {
                assert_eq!(device, 3);
                ControllerOperationResult::Accepted
            }),
        );
        assert!(haptics.sound("base", "sound/shot.wav").unwrap().is_some());
        haptics.set_enabled(false);
        assert!(!haptics.enabled());
        assert!(haptics.sound("base", "sound/shot.wav").unwrap().is_none());
        haptics.set_enabled(true);
        haptics.invalidate_assets();
        assert_eq!(haptics.update(), HapticsResult::Unchanged);
        haptics.close();
        assert!(haptics.sound("base", "sound/shot.wav").unwrap().is_none());
    }
}
