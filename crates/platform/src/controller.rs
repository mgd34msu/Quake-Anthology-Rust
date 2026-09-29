//! SDL game controllers: assignment, events, rumble, LEDs, and sensors.
//!
//! Port of donor `src/platform/controller.ts`. Explicit-before-automatic
//! assignment intent comes from the game layer; this module owns exactly one
//! event provider with independently routed native handles. No gameplay or
//! key bindings live here.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use crate::error::{Error, Result};
use crate::ffi_util::{c_string, c_string_lossy, sdl_error, LoadedLibrary};
use crate::native_libraries::{NativeLibrary, NativeLibraryOptions};

const SUBSYSTEM: u32 = 0x2000;
/// SDL controller axis count.
pub const AXIS_COUNT: usize = 6;
/// SDL controller button count.
pub const BUTTON_COUNT: usize = 21;

/// Controller sensor kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControllerSensor {
    /// Accelerometer.
    Accelerometer,
    /// Gyroscope.
    Gyro,
    /// Left accelerometer.
    AccelerometerLeft,
    /// Left gyroscope.
    GyroLeft,
    /// Right accelerometer.
    AccelerometerRight,
    /// Right gyroscope.
    GyroRight,
}

impl ControllerSensor {
    fn sdl_id(self) -> i32 {
        match self {
            Self::Accelerometer => 1,
            Self::Gyro => 2,
            Self::AccelerometerLeft => 3,
            Self::GyroLeft => 4,
            Self::AccelerometerRight => 5,
            Self::GyroRight => 6,
        }
    }

    fn from_sdl_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Accelerometer),
            2 => Some(Self::Gyro),
            3 => Some(Self::AccelerometerLeft),
            4 => Some(Self::GyroLeft),
            5 => Some(Self::AccelerometerRight),
            6 => Some(Self::GyroRight),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Accelerometer => "accelerometer",
            Self::Gyro => "gyro",
            Self::AccelerometerLeft => "accelerometer-left",
            Self::GyroLeft => "gyro-left",
            Self::AccelerometerRight => "accelerometer-right",
            Self::GyroRight => "gyro-right",
        }
    }
}

const SENSOR_ORDER: [ControllerSensor; 6] = [
    ControllerSensor::Accelerometer,
    ControllerSensor::Gyro,
    ControllerSensor::AccelerometerLeft,
    ControllerSensor::GyroLeft,
    ControllerSensor::AccelerometerRight,
    ControllerSensor::GyroRight,
];

/// Sensor capability and state.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorInfo {
    /// Sensor kind.
    pub kind: ControllerSensor,
    /// Whether the sensor is enabled.
    pub enabled: bool,
    /// Sample rate in Hz.
    pub rate_hz: f32,
}

/// Static device capabilities.
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerCapabilities {
    /// Per-axis presence (6 entries).
    pub axes: Vec<bool>,
    /// Per-button presence (21 entries).
    pub buttons: Vec<bool>,
    /// Rumble supported.
    pub rumble: bool,
    /// Trigger rumble supported.
    pub trigger_rumble: bool,
    /// LED supported.
    pub led: bool,
    /// Touchpad count.
    pub touchpads: i32,
    /// Present sensors.
    pub sensors: Vec<SensorInfo>,
}

/// A connected controller device.
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerDevice {
    /// SDL instance id.
    pub instance: i32,
    /// Device name.
    pub name: String,
    /// Lowercase GUID, when the mapping header carries one.
    pub guid: Option<String>,
    /// Serial string, when reported.
    pub serial: Option<String>,
    /// Ordinal among identical GUIDs. Stable while the provider is open, even
    /// when an earlier identical pad disconnects.
    pub ordinal: u32,
    /// SDL virtual device.
    pub virtual_device: bool,
    /// Capabilities snapshot.
    pub capabilities: ControllerCapabilities,
}

/// Live axis/button snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerState {
    /// Axis values (6 entries).
    pub axes: Vec<i16>,
    /// Button states (21 entries).
    pub buttons: Vec<bool>,
}

/// Slot assignment intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControllerSelection {
    /// Claim the first unclaimed device.
    Automatic,
    /// Leave the slot empty.
    None,
    /// Claim the nth device with this GUID.
    Device {
        /// 32-hex-digit GUID.
        guid: String,
        /// Ordinal among identical GUIDs.
        ordinal: u32,
    },
    /// Claim the device with this GUID and serial.
    Serial {
        /// 32-hex-digit GUID.
        guid: String,
        /// Serial string.
        serial: String,
    },
}

/// Outcome of a rumble/LED/sensor operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControllerOperationResult {
    /// Accepted by the backend.
    Accepted,
    /// Device lacks the capability.
    Unsupported {
        /// Cause.
        reason: String,
    },
    /// Device is gone.
    Disconnected {
        /// Cause.
        reason: String,
    },
    /// Backend call failed.
    Failed {
        /// Cause.
        reason: String,
    },
}

/// Touchpad contact phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchpadPhase {
    /// Contact started.
    Down,
    /// Contact moved.
    Motion,
    /// Contact ended.
    Up,
}

/// Controller event.
#[derive(Clone, Debug, PartialEq)]
pub enum ControllerEvent {
    /// Device connected.
    Connected {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device description.
        device: ControllerDevice,
    },
    /// Device remapped.
    Remapped {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device description.
        device: ControllerDevice,
    },
    /// Slot assignment changed.
    Assignment {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Slot index.
        slot: usize,
        /// Previously routed instance.
        previous: Option<i32>,
        /// Newly routed instance.
        instance: Option<i32>,
    },
    /// Device disconnected.
    Disconnected {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Slot the device held, if any.
        slot: Option<usize>,
    },
    /// Axis motion.
    Axis {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Slot the device holds, if any.
        slot: Option<usize>,
        /// Axis index.
        axis: u8,
        /// Axis value.
        value: i16,
    },
    /// Button transition.
    Button {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Slot the device holds, if any.
        slot: Option<usize>,
        /// Button index.
        button: u8,
        /// True for press, false for release.
        down: bool,
    },
    /// Touchpad contact.
    Touchpad {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Slot the device holds, if any.
        slot: Option<usize>,
        /// Contact phase.
        phase: TouchpadPhase,
        /// Touchpad index.
        touchpad: i32,
        /// Finger index.
        finger: i32,
        /// X position.
        x: f32,
        /// Y position.
        y: f32,
        /// Pressure.
        pressure: f32,
    },
    /// Sensor sample.
    Sensor {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Slot the device holds, if any.
        slot: Option<usize>,
        /// Sensor kind.
        sensor: ControllerSensor,
        /// X component.
        x: f32,
        /// Y component.
        y: f32,
        /// Z component.
        z: f32,
        /// Sensor timestamp in microseconds.
        timestamp_us: u64,
    },
    /// Unrecognized record in the controller range.
    Unrecognized {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Slot the device holds, if any.
        slot: Option<usize>,
        /// SDL event type.
        event_type: u32,
        /// Raw record bytes.
        bytes: [u8; 56],
    },
}

/// Sensor read outcome.
#[derive(Clone, Debug, PartialEq)]
pub enum SensorRead {
    /// A live sample.
    Sample {
        /// X component.
        x: f32,
        /// Y component.
        y: f32,
        /// Z component.
        z: f32,
    },
    /// Operation status (device lacks the sensor, disconnected, or failed).
    Status(ControllerOperationResult),
}

/// Whether a virtual mapping was added or updated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingOutcome {
    /// New mapping added.
    Added,
    /// Existing mapping updated.
    Updated,
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes(bytes[offset..offset + 4].try_into().expect("record field"))
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_ne_bytes(bytes[offset..offset + 4].try_into().expect("record field"))
}

fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_ne_bytes(bytes[offset..offset + 2].try_into().expect("record field"))
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_ne_bytes(bytes[offset..offset + 4].try_into().expect("record field"))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_ne_bytes(bytes[offset..offset + 8].try_into().expect("record field"))
}

/// Decoded 56-byte controller record, before slot routing.
#[derive(Clone, Debug, PartialEq)]
pub enum ControllerRecord {
    /// Device added; carries a device index, not an instance.
    Added {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device index.
        device_index: i32,
    },
    /// Axis motion.
    Axis {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Axis index.
        axis: u8,
        /// Axis value.
        value: i16,
    },
    /// Button transition.
    Button {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Button index.
        button: u8,
        /// True for press, false for release.
        down: bool,
    },
    /// Device removed.
    Removed {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
    },
    /// Device remapped.
    Remapped {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
    },
    /// Touchpad contact.
    Touchpad {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Contact phase.
        phase: TouchpadPhase,
        /// Touchpad index.
        touchpad: i32,
        /// Finger index.
        finger: i32,
        /// X position.
        x: f32,
        /// Y position.
        y: f32,
        /// Pressure.
        pressure: f32,
    },
    /// Sensor sample.
    Sensor {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Sensor kind.
        sensor: ControllerSensor,
        /// X component.
        x: f32,
        /// Y component.
        y: f32,
        /// Z component.
        z: f32,
        /// Sensor timestamp in microseconds.
        timestamp_us: u64,
    },
    /// Anything else in the range, including unknown sensor ids.
    Unrecognized {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// SDL event type.
        event_type: u32,
    },
}

/// Decode one 56-byte controller record. Offsets follow SDL2 `SDL_events.h`.
pub fn decode_controller_record(bytes: &[u8; 56]) -> ControllerRecord {
    let event_type = read_u32(bytes, 0);
    let timestamp = read_u32(bytes, 4);
    let instance = read_i32(bytes, 8);
    match event_type {
        0x650 => ControllerRecord::Axis {
            timestamp,
            instance,
            axis: bytes[12],
            value: read_i16(bytes, 16),
        },
        0x651 | 0x652 => ControllerRecord::Button {
            timestamp,
            instance,
            button: bytes[12],
            down: event_type == 0x651,
        },
        0x653 => ControllerRecord::Added {
            timestamp,
            device_index: instance,
        },
        0x654 => ControllerRecord::Removed { timestamp, instance },
        0x655 => ControllerRecord::Remapped { timestamp, instance },
        0x656..=0x658 => ControllerRecord::Touchpad {
            timestamp,
            instance,
            phase: if event_type == 0x656 {
                TouchpadPhase::Down
            } else if event_type == 0x657 {
                TouchpadPhase::Motion
            } else {
                TouchpadPhase::Up
            },
            touchpad: read_i32(bytes, 12),
            finger: read_i32(bytes, 16),
            x: read_f32(bytes, 20),
            y: read_f32(bytes, 24),
            pressure: read_f32(bytes, 28),
        },
        0x659 => match ControllerSensor::from_sdl_id(read_i32(bytes, 12)) {
            Some(sensor) => ControllerRecord::Sensor {
                timestamp,
                instance,
                sensor,
                x: read_f32(bytes, 16),
                y: read_f32(bytes, 20),
                z: read_f32(bytes, 24),
                timestamp_us: read_u64(bytes, 32),
            },
            None => ControllerRecord::Unrecognized {
                timestamp,
                instance,
                event_type,
            },
        },
        _ => ControllerRecord::Unrecognized {
            timestamp,
            instance,
            event_type,
        },
    }
}

/// True when `value` is a 32-hex-digit GUID.
pub fn is_guid(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Validate one slot selection.
pub fn validate_selection(selection: &ControllerSelection) -> Result<()> {
    match selection {
        ControllerSelection::Automatic | ControllerSelection::None => Ok(()),
        ControllerSelection::Device { guid, ordinal } => {
            if !is_guid(guid) {
                return Err(Error::InvalidInput(
                    "controller assignment GUID requires 32 hexadecimal digits".to_string(),
                ));
            }
            if *ordinal > i32::MAX as u32 {
                return Err(Error::OutOfRange("controller ordinal exceeds int32".to_string()));
            }
            Ok(())
        }
        ControllerSelection::Serial { guid, serial } => {
            if !is_guid(guid) {
                return Err(Error::InvalidInput(
                    "controller assignment GUID requires 32 hexadecimal digits".to_string(),
                ));
            }
            if serial.is_empty() || serial.contains('\0') {
                return Err(Error::InvalidInput(
                    "controller serial must be nonempty and contain no NUL".to_string(),
                ));
            }
            Ok(())
        }
    }
}

/// Minimal device identity for assignment resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignDevice {
    /// SDL instance id.
    pub instance: i32,
    /// Lowercase GUID, when known.
    pub guid: Option<String>,
    /// Ordinal among identical GUIDs.
    pub ordinal: u32,
    /// Serial string, when reported.
    pub serial: Option<String>,
}

/// Resolve slot selections to device instances. Explicit selections claim
/// first (requiring exactly one match); automatic selections then claim the
/// first unclaimed device in order.
pub fn resolve_assignments(selections: &[ControllerSelection], devices: &[AssignDevice]) -> Vec<Option<i32>> {
    let mut routes: Vec<Option<i32>> = selections.iter().map(|_| None).collect();
    let mut claimed = std::collections::HashSet::new();
    for (slot, selection) in selections.iter().enumerate() {
        let (guid, ordinal, serial) = match selection {
            ControllerSelection::Device { guid, ordinal } => (guid.to_lowercase(), Some(*ordinal), None),
            ControllerSelection::Serial { guid, serial } => (guid.to_lowercase(), None, Some(serial.clone())),
            _ => continue,
        };
        let matches: Vec<&AssignDevice> = devices
            .iter()
            .filter(|device| {
                !claimed.contains(&device.instance)
                    && device.guid.as_deref() == Some(guid.as_str())
                    && ordinal.is_none_or(|o| device.ordinal == o)
                    && serial.as_deref().is_none_or(|s| device.serial.as_deref() == Some(s))
            })
            .collect();
        if matches.len() == 1 {
            let device = matches[0];
            routes[slot] = Some(device.instance);
            claimed.insert(device.instance);
        }
    }
    for (slot, selection) in selections.iter().enumerate() {
        if *selection != ControllerSelection::Automatic {
            continue;
        }
        if let Some(device) = devices.iter().find(|device| !claimed.contains(&device.instance)) {
            routes[slot] = Some(device.instance);
            claimed.insert(device.instance);
        }
    }
    routes
}

/// Validate an SDL controller mapping line: GUID, name, and bindings.
pub fn validate_mapping(mapping: &str) -> Result<()> {
    let mut parts = mapping.split(',');
    let guid = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if !is_guid(guid) || name.is_empty() {
        return Err(Error::InvalidInput(
            "SDL controller mapping requires a GUID, name and bindings".to_string(),
        ));
    }
    Ok(())
}

/// Scale a 0..=1 rumble amplitude to SDL magnitude.
pub fn rumble_magnitude(value: f64) -> Result<u16> {
    if !value.is_finite() || value < 0.0 || value > 1.0 {
        return Err(Error::OutOfRange(
            "rumble amplitude must be finite and in 0..1".to_string(),
        ));
    }
    Ok((value * 65535.0).round() as u16)
}

struct ControllerSdl {
    _lib: LoadedLibrary,
    sdl_set_main_ready: unsafe extern "C" fn(),
    sdl_set_hint: unsafe extern "C" fn(*const u8, *const u8) -> i32,
    sdl_init_sub_system: unsafe extern "C" fn(u32) -> i32,
    sdl_quit_sub_system: unsafe extern "C" fn(u32),
    sdl_get_error: unsafe extern "C" fn() -> *const u8,
    sdl_get_version: unsafe extern "C" fn(*mut u8),
    sdl_get_revision: unsafe extern "C" fn() -> *const u8,
    sdl_get_ticks: unsafe extern "C" fn() -> u32,
    sdl_num_joysticks: unsafe extern "C" fn() -> i32,
    sdl_joystick_get_device_instance_id: unsafe extern "C" fn(i32) -> i32,
    sdl_joystick_instance_id: unsafe extern "C" fn(*mut c_void) -> i32,
    sdl_joystick_is_virtual: unsafe extern "C" fn(i32) -> i32,
    sdl_is_game_controller: unsafe extern "C" fn(i32) -> i32,
    sdl_game_controller_open: unsafe extern "C" fn(i32) -> *mut c_void,
    sdl_game_controller_close: unsafe extern "C" fn(*mut c_void),
    sdl_game_controller_get_joystick: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    sdl_game_controller_get_attached: unsafe extern "C" fn(*mut c_void) -> i32,
    sdl_game_controller_name: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    sdl_game_controller_get_serial: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    sdl_game_controller_mapping: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    sdl_game_controller_add_mapping: unsafe extern "C" fn(*const u8) -> i32,
    sdl_free: unsafe extern "C" fn(*mut c_void),
    sdl_game_controller_update: unsafe extern "C" fn(),
    sdl_game_controller_event_state: unsafe extern "C" fn(i32) -> i32,
    sdl_game_controller_has_axis: unsafe extern "C" fn(*mut c_void, i32) -> i32,
    sdl_game_controller_has_button: unsafe extern "C" fn(*mut c_void, i32) -> i32,
    sdl_game_controller_get_axis: unsafe extern "C" fn(*mut c_void, i32) -> i16,
    sdl_game_controller_get_button: unsafe extern "C" fn(*mut c_void, i32) -> u8,
    sdl_game_controller_has_rumble: unsafe extern "C" fn(*mut c_void) -> i32,
    sdl_game_controller_has_rumble_triggers: unsafe extern "C" fn(*mut c_void) -> i32,
    sdl_game_controller_has_led: unsafe extern "C" fn(*mut c_void) -> i32,
    sdl_game_controller_rumble: unsafe extern "C" fn(*mut c_void, u16, u16, u32) -> i32,
    sdl_game_controller_rumble_triggers: unsafe extern "C" fn(*mut c_void, u16, u16, u32) -> i32,
    sdl_game_controller_set_led: unsafe extern "C" fn(*mut c_void, u8, u8, u8) -> i32,
    sdl_game_controller_get_num_touchpads: unsafe extern "C" fn(*mut c_void) -> i32,
    sdl_game_controller_has_sensor: unsafe extern "C" fn(*mut c_void, i32) -> i32,
    sdl_game_controller_set_sensor_enabled: unsafe extern "C" fn(*mut c_void, i32, i32) -> i32,
    sdl_game_controller_is_sensor_enabled: unsafe extern "C" fn(*mut c_void, i32) -> i32,
    sdl_game_controller_get_sensor_data_rate: unsafe extern "C" fn(*mut c_void, i32) -> f32,
    sdl_game_controller_get_sensor_data: unsafe extern "C" fn(*mut c_void, i32, *mut f32, i32) -> i32,
    sdl_peep_events: unsafe extern "C" fn(*mut u8, i32, i32, u32, u32) -> i32,
}

macro_rules! define_controller_loader {
    ($struct:ident, $($field:ident : $cname:literal;)*) => {
        impl $struct {
            /// # Safety
            ///
            /// Resolved symbols are only invoked with the SDL ABI below.
            unsafe fn load(options: &NativeLibraryOptions) -> Result<Arc<Self>> {
                // SAFETY: loading maps the image without invoking its code.
                let lib = unsafe { LoadedLibrary::open(NativeLibrary::Sdl2, options)? };
                let this = Arc::new(Self {
                    $( $field: unsafe { lib.symbol(concat!($cname, "\0").as_bytes())? }, )*
                    _lib: lib,
                });
                Ok(this)
            }

            /// # Safety
            ///
            /// SDL calls below uphold their own argument contracts.
            unsafe fn error(&self) -> String {
                // SAFETY: SDL guarantees a valid thread-local error string.
                unsafe { sdl_error(self.sdl_get_error) }
            }

            /// # Safety
            ///
            /// `value` must be SDL's return for `operation`.
            unsafe fn checked(&self, value: i32, operation: &str) -> Result<i32> {
                if value < 0 {
                    // SAFETY: error read immediately after the failing call.
                    return Err(Error::native(operation, unsafe { self.error() }));
                }
                Ok(value)
            }
        }
    };
}

define_controller_loader! { ControllerSdl,
    sdl_set_main_ready: "SDL_SetMainReady";
    sdl_set_hint: "SDL_SetHint";
    sdl_init_sub_system: "SDL_InitSubSystem";
    sdl_quit_sub_system: "SDL_QuitSubSystem";
    sdl_get_error: "SDL_GetError";
    sdl_get_version: "SDL_GetVersion";
    sdl_get_revision: "SDL_GetRevision";
    sdl_get_ticks: "SDL_GetTicks";
    sdl_num_joysticks: "SDL_NumJoysticks";
    sdl_joystick_get_device_instance_id: "SDL_JoystickGetDeviceInstanceID";
    sdl_joystick_instance_id: "SDL_JoystickInstanceID";
    sdl_joystick_is_virtual: "SDL_JoystickIsVirtual";
    sdl_is_game_controller: "SDL_IsGameController";
    sdl_game_controller_open: "SDL_GameControllerOpen";
    sdl_game_controller_close: "SDL_GameControllerClose";
    sdl_game_controller_get_joystick: "SDL_GameControllerGetJoystick";
    sdl_game_controller_get_attached: "SDL_GameControllerGetAttached";
    sdl_game_controller_name: "SDL_GameControllerName";
    sdl_game_controller_get_serial: "SDL_GameControllerGetSerial";
    sdl_game_controller_mapping: "SDL_GameControllerMapping";
    sdl_game_controller_add_mapping: "SDL_GameControllerAddMapping";
    sdl_free: "SDL_free";
    sdl_game_controller_update: "SDL_GameControllerUpdate";
    sdl_game_controller_event_state: "SDL_GameControllerEventState";
    sdl_game_controller_has_axis: "SDL_GameControllerHasAxis";
    sdl_game_controller_has_button: "SDL_GameControllerHasButton";
    sdl_game_controller_get_axis: "SDL_GameControllerGetAxis";
    sdl_game_controller_get_button: "SDL_GameControllerGetButton";
    sdl_game_controller_has_rumble: "SDL_GameControllerHasRumble";
    sdl_game_controller_has_rumble_triggers: "SDL_GameControllerHasRumbleTriggers";
    sdl_game_controller_has_led: "SDL_GameControllerHasLED";
    sdl_game_controller_rumble: "SDL_GameControllerRumble";
    sdl_game_controller_rumble_triggers: "SDL_GameControllerRumbleTriggers";
    sdl_game_controller_set_led: "SDL_GameControllerSetLED";
    sdl_game_controller_get_num_touchpads: "SDL_GameControllerGetNumTouchpads";
    sdl_game_controller_has_sensor: "SDL_GameControllerHasSensor";
    sdl_game_controller_set_sensor_enabled: "SDL_GameControllerSetSensorEnabled";
    sdl_game_controller_is_sensor_enabled: "SDL_GameControllerIsSensorEnabled";
    sdl_game_controller_get_sensor_data_rate: "SDL_GameControllerGetSensorDataRate";
    sdl_game_controller_get_sensor_data: "SDL_GameControllerGetSensorData";
    sdl_peep_events: "SDL_PeepEvents";
}

fn controller_owner() -> &'static Mutex<Option<u64>> {
    static OWNER: OnceLock<Mutex<Option<u64>>> = OnceLock::new();
    OWNER.get_or_init(|| Mutex::new(None))
}

fn next_owner_id() -> u64 {
    static IDS: AtomicU64 = AtomicU64::new(1);
    IDS.fetch_add(1, Ordering::SeqCst)
}

struct OwnedController {
    pointer: *mut c_void,
    instance: i32,
    ordinal: u32,
    virtual_device: bool,
    axes: Vec<i16>,
    buttons: Vec<bool>,
    guid: Option<String>,
    name: String,
    serial: Option<String>,
}

impl OwnedController {
    fn assign_descriptor(&self) -> AssignDevice {
        AssignDevice {
            instance: self.instance,
            guid: self.guid.clone(),
            ordinal: self.ordinal,
            serial: self.serial.clone(),
        }
    }
}

/// One event owner, with independently routed native handles.
pub struct SdlControllers {
    sdl: Arc<ControllerSdl>,
    id: u64,
    opened: Vec<OwnedController>,
    pending: Vec<ControllerEvent>,
    selections: Vec<ControllerSelection>,
    routes: Vec<Option<i32>>,
    previous_event_state: i32,
    owner: thread::ThreadId,
    _not_send: PhantomData<*const ()>,
}

impl SdlControllers {
    /// SDL runtime version and revision with live process discovery.
    pub fn runtime() -> Result<(String, String)> {
        Self::runtime_with(&NativeLibraryOptions::default())
    }

    /// SDL runtime version and revision with explicit library discovery.
    pub fn runtime_with(lib_options: &NativeLibraryOptions) -> Result<(String, String)> {
        // SAFETY: loading maps the image; out-pointers describe live bytes.
        unsafe {
            let sdl = ControllerSdl::load(lib_options)?;
            let mut bytes = [0u8; 3];
            (sdl.sdl_get_version)(bytes.as_mut_ptr());
            Ok((
                format!("{}.{}.{}", bytes[0], bytes[1], bytes[2]),
                c_string_lossy((sdl.sdl_get_revision)()),
            ))
        }
    }

    /// Open the single controller event owner.
    pub fn open() -> Result<Self> {
        Self::open_with(&NativeLibraryOptions::default())
    }

    /// Open with explicit library discovery (tests inject overrides).
    pub fn open_with(lib_options: &NativeLibraryOptions) -> Result<Self> {
        if controller_owner().lock().expect("owner").is_some() {
            return Err(Error::InvalidInput(
                "SDL controller events already have an owner".to_string(),
            ));
        }
        // SAFETY: loading maps the image; SDL calls below use validated arguments.
        let sdl = unsafe { ControllerSdl::load(lib_options)? };
        unsafe {
            Self::initialize(&sdl)?;
            let previous = (sdl.sdl_game_controller_event_state)(-1);
            let id = next_owner_id();
            *controller_owner().lock().expect("owner") = Some(id);
            let mut owner = Self {
                sdl,
                id,
                opened: Vec::new(),
                pending: Vec::new(),
                selections: Vec::new(),
                routes: Vec::new(),
                previous_event_state: previous,
                owner: thread::current().id(),
                _not_send: PhantomData,
            };
            owner.sdl_event_state(1);
            let ticks = (owner.sdl.sdl_get_ticks)();
            if let Err(error) = owner.discover(ticks, None) {
                owner.close();
                return Err(error);
            }
            Ok(owner)
        }
    }

    /// # Safety
    ///
    /// The SDL handle must be fully loaded.
    unsafe fn initialize(sdl: &ControllerSdl) -> Result<()> {
        // SAFETY: hint buffers are live for the call.
        unsafe {
            (sdl.sdl_set_main_ready)();
            let key = c_string("SDL_NO_SIGNAL_HANDLERS")?;
            let value = c_string("1")?;
            if (sdl.sdl_set_hint)(key.as_ptr(), value.as_ptr()) != 1 {
                return Err(Error::InvalidInput(
                    "SDL must leave signal handling to the Unix signal owner".to_string(),
                ));
            }
            sdl.checked((sdl.sdl_init_sub_system)(SUBSYSTEM), "SDL_InitSubSystem controller")?;
        }
        Ok(())
    }

    fn sdl_event_state(&self, state: i32) {
        // SAFETY: the event-state flag is validated by construction.
        unsafe {
            (self.sdl.sdl_game_controller_event_state)(state);
        }
    }

    fn check_owner(&self) -> Result<()> {
        if thread::current().id() != self.owner {
            return Err(Error::InvalidInput(
                "SDL controllers must be owned by the owning thread".to_string(),
            ));
        }
        Ok(())
    }

    /// Whether the provider is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        *controller_owner().lock().expect("owner") != Some(self.id)
    }

    fn require_open(&self) -> Result<()> {
        self.check_owner()?;
        if self.is_closed() {
            return Err(Error::Closed("SDL controller provider".to_string()));
        }
        Ok(())
    }

    /// Connected device descriptions.
    pub fn devices(&self) -> Result<Vec<ControllerDevice>> {
        self.require_open()?;
        // SAFETY: every owned pointer is a live controller.
        unsafe {
            let mut devices = Vec::with_capacity(self.opened.len());
            for device in &self.opened {
                devices.push(self.describe(device)?);
            }
            Ok(devices)
        }
    }

    /// Current slot routes (instance per slot).
    pub fn assignments(&self) -> Result<Vec<Option<i32>>> {
        self.require_open()?;
        Ok(self.routes.clone())
    }

    /// SDL runtime version.
    pub fn version(&self) -> Result<String> {
        self.require_open()?;
        // SAFETY: out-pointers describe live bytes.
        unsafe {
            let mut bytes = [0u8; 3];
            (self.sdl.sdl_get_version)(bytes.as_mut_ptr());
            Ok(format!("{}.{}.{}", bytes[0], bytes[1], bytes[2]))
        }
    }

    /// Live axis/button snapshot, or `None` when the device is gone.
    pub fn snapshot(&self, instance: i32) -> Result<Option<ControllerState>> {
        self.require_open()?;
        let Some(device) = self.opened.iter().find(|device| device.instance == instance) else {
            return Ok(None);
        };
        // SAFETY: the controller handle is live.
        let attached = unsafe { (self.sdl.sdl_game_controller_get_attached)(device.pointer) != 0 };
        if !attached {
            return Ok(None);
        }
        Ok(Some(ControllerState {
            axes: device.axes.clone(),
            buttons: device.buttons.clone(),
        }))
    }

    /// Replace slot selections, emitting assignment events.
    pub fn set_assignments(&mut self, selections: &[ControllerSelection]) -> Result<()> {
        self.require_open()?;
        for selection in selections {
            validate_selection(selection)?;
        }
        self.collect()?;
        self.selections = selections
            .iter()
            .map(|selection| match selection {
                ControllerSelection::Device { guid, ordinal } => ControllerSelection::Device {
                    guid: guid.to_lowercase(),
                    ordinal: *ordinal,
                },
                ControllerSelection::Serial { guid, serial } => ControllerSelection::Serial {
                    guid: guid.to_lowercase(),
                    serial: serial.clone(),
                },
                other => other.clone(),
            })
            .collect();
        // SAFETY: no arguments.
        let ticks = unsafe { (self.sdl.sdl_get_ticks)() };
        self.resolve(ticks);
        Ok(())
    }

    fn slot(&self, instance: i32) -> Option<usize> {
        self.routes.iter().position(|route| *route == Some(instance))
    }

    fn resolve(&mut self, timestamp: u32) {
        let descriptors: Vec<AssignDevice> = self.opened.iter().map(OwnedController::assign_descriptor).collect();
        let routes = resolve_assignments(&self.selections, &descriptors);
        for slot in 0..routes.len().max(self.routes.len()) {
            let previous = self.routes.get(slot).copied().flatten();
            let instance = routes.get(slot).copied().flatten();
            if previous == instance {
                continue;
            }
            if let Some(previous) = previous {
                self.stop(previous);
            }
            self.pending.push(ControllerEvent::Assignment {
                timestamp,
                slot,
                previous,
                instance,
            });
        }
        self.routes = routes;
    }

    /// # Safety
    ///
    /// The controller handle must be live.
    unsafe fn identity(sdl: &ControllerSdl, pointer: *mut c_void) -> Result<(Option<String>, String, Option<String>)> {
        // SAFETY: the controller handle is live; the mapping is freed below.
        unsafe {
            let mapping = (sdl.sdl_game_controller_mapping)(pointer);
            let mut guid = None;
            if !mapping.is_null() {
                let text = c_string_lossy(mapping.cast::<u8>());
                (sdl.sdl_free)(mapping);
                if let Some(head) = text.split(',').next() {
                    if is_guid(head) {
                        guid = Some(head.to_lowercase());
                    }
                }
            }
            let name_ptr = (sdl.sdl_game_controller_name)(pointer);
            let name = if name_ptr.is_null() {
                "Unnamed SDL controller".to_string()
            } else {
                c_string_lossy(name_ptr.cast::<u8>())
            };
            let serial_ptr = (sdl.sdl_game_controller_get_serial)(pointer);
            let serial = if serial_ptr.is_null() {
                None
            } else {
                Some(c_string_lossy(serial_ptr.cast::<u8>()))
            };
            Ok((guid, name, serial))
        }
    }

    /// # Safety
    ///
    /// The controller handle must be live.
    unsafe fn capabilities(sdl: &ControllerSdl, pointer: *mut c_void) -> Result<ControllerCapabilities> {
        // SAFETY: the controller handle is live.
        unsafe {
            let mut axes = Vec::with_capacity(AXIS_COUNT);
            for axis in 0..AXIS_COUNT {
                axes.push((sdl.sdl_game_controller_has_axis)(pointer, axis as i32) != 0);
            }
            let mut buttons = Vec::with_capacity(BUTTON_COUNT);
            for button in 0..BUTTON_COUNT {
                buttons.push((sdl.sdl_game_controller_has_button)(pointer, button as i32) != 0);
            }
            let mut sensors = Vec::new();
            for kind in SENSOR_ORDER {
                if (sdl.sdl_game_controller_has_sensor)(pointer, kind.sdl_id()) == 0 {
                    continue;
                }
                sensors.push(SensorInfo {
                    kind,
                    enabled: (sdl.sdl_game_controller_is_sensor_enabled)(pointer, kind.sdl_id()) != 0,
                    rate_hz: (sdl.sdl_game_controller_get_sensor_data_rate)(pointer, kind.sdl_id()),
                });
            }
            Ok(ControllerCapabilities {
                axes,
                buttons,
                rumble: (sdl.sdl_game_controller_has_rumble)(pointer) != 0,
                trigger_rumble: (sdl.sdl_game_controller_has_rumble_triggers)(pointer) != 0,
                led: (sdl.sdl_game_controller_has_led)(pointer) != 0,
                touchpads: sdl.checked(
                    (sdl.sdl_game_controller_get_num_touchpads)(pointer),
                    "SDL_GameControllerGetNumTouchpads",
                )?,
                sensors,
            })
        }
    }

    /// # Safety
    ///
    /// The device handle must be live.
    unsafe fn describe(&self, device: &OwnedController) -> Result<ControllerDevice> {
        // SAFETY: caller guarantees a live handle.
        unsafe {
            Ok(ControllerDevice {
                instance: device.instance,
                name: device.name.clone(),
                guid: device.guid.clone(),
                serial: device.serial.clone(),
                ordinal: device.ordinal,
                virtual_device: device.virtual_device,
                capabilities: Self::capabilities(&self.sdl, device.pointer)?,
            })
        }
    }

    fn discover(&mut self, timestamp: u32, device_index: Option<i32>) -> Result<()> {
        // SAFETY: device indexes stay within the SDL count below.
        unsafe {
            let count = self.sdl.checked((self.sdl.sdl_num_joysticks)(), "SDL_NumJoysticks")?;
            let end = match device_index {
                None => count,
                Some(index) => count.min(index + 1),
            };
            let start = device_index.unwrap_or(0);
            let mut index = start;
            while index >= 0 && index < end {
                let instance = (self.sdl.sdl_joystick_get_device_instance_id)(index);
                if instance >= 0
                    && !self.opened.iter().any(|device| device.instance == instance)
                    && (self.sdl.sdl_is_game_controller)(index) != 0
                {
                    self.open_device(timestamp, index, instance)?;
                }
                index += 1;
            }
            self.resolve(timestamp);
            Ok(())
        }
    }

    /// # Safety
    ///
    /// `index` must be a live game-controller device index.
    unsafe fn open_device(&mut self, timestamp: u32, index: i32, instance: i32) -> Result<()> {
        // SAFETY: caller guarantees a live device index.
        unsafe {
            let pointer = (self.sdl.sdl_game_controller_open)(index);
            if pointer.is_null() {
                return Err(Error::native("SDL_GameControllerOpen", self.sdl.error()));
            }
            let result = self.adopt_device(timestamp, index, instance, pointer);
            if result.is_err() {
                (self.sdl.sdl_game_controller_close)(pointer);
            }
            result
        }
    }

    /// # Safety
    ///
    /// `pointer` must be a newly opened controller; ownership moves into
    /// `self.opened` on success, or stays with the caller on error.
    unsafe fn adopt_device(&mut self, timestamp: u32, index: i32, instance: i32, pointer: *mut c_void) -> Result<()> {
        // SAFETY: caller guarantees a newly opened controller.
        unsafe {
            let joystick = (self.sdl.sdl_game_controller_get_joystick)(pointer);
            if joystick.is_null() {
                return Err(Error::native("SDL_GameControllerGetJoystick", self.sdl.error()));
            }
            let opened_instance = self
                .sdl
                .checked((self.sdl.sdl_joystick_instance_id)(joystick), "SDL_JoystickInstanceID")?;
            if opened_instance != instance {
                return Err(Error::InvalidInput(
                    "SDL controller instance changed while opening".to_string(),
                ));
            }
            let (guid, name, serial) = Self::identity(&self.sdl, pointer)?;
            let mut ordinals: std::collections::HashSet<u32> = self
                .opened
                .iter()
                .filter(|device| device.guid == guid)
                .map(|device| device.ordinal)
                .collect();
            let mut ordinal = 0;
            while !ordinals.insert(ordinal) {
                ordinal += 1;
            }
            let mut axes = Vec::with_capacity(AXIS_COUNT);
            for axis in 0..AXIS_COUNT {
                axes.push((self.sdl.sdl_game_controller_get_axis)(pointer, axis as i32));
            }
            let mut buttons = Vec::with_capacity(BUTTON_COUNT);
            for button in 0..BUTTON_COUNT {
                buttons.push((self.sdl.sdl_game_controller_get_button)(pointer, button as i32) != 0);
            }
            let device = OwnedController {
                pointer,
                instance,
                ordinal,
                virtual_device: (self.sdl.sdl_joystick_is_virtual)(index) != 0,
                axes,
                buttons,
                guid,
                name,
                serial,
            };
            let description = self.describe(&device)?;
            self.opened.push(device);
            self.pending.push(ControllerEvent::Connected {
                timestamp,
                device: description,
            });
            Ok(())
        }
    }

    fn remove(&mut self, instance: i32, timestamp: u32) {
        let Some(position) = self.opened.iter().position(|device| device.instance == instance) else {
            return;
        };
        let device = self.opened.remove(position);
        let slot = self.slot(instance);
        self.stop(instance);
        // SAFETY: the controller handle is live until this call.
        unsafe {
            (self.sdl.sdl_game_controller_close)(device.pointer);
        }
        self.pending.push(ControllerEvent::Disconnected {
            timestamp,
            instance,
            slot,
        });
        self.resolve(timestamp);
    }

    fn collect(&mut self) -> Result<()> {
        let mut bytes = [0u8; 56];
        // SAFETY: the event buffer is live for each call.
        unsafe {
            (self.sdl.sdl_game_controller_update)();
            loop {
                let count = self.sdl.checked(
                    (self.sdl.sdl_peep_events)(bytes.as_mut_ptr(), 1, 2, 0x650, 0x66f),
                    "SDL_PeepEvents controller",
                )?;
                if count == 0 {
                    break;
                }
                self.handle_record(&bytes)?;
            }
            // A device can disappear while SDL events are disabled by another subsystem.
            let gone: Vec<i32> = self
                .opened
                .iter()
                .filter(|device| (self.sdl.sdl_game_controller_get_attached)(device.pointer) == 0)
                .map(|device| device.instance)
                .collect();
            if !gone.is_empty() {
                let ticks = (self.sdl.sdl_get_ticks)();
                for instance in gone {
                    self.remove(instance, ticks);
                }
            }
            Ok(())
        }
    }

    fn handle_record(&mut self, bytes: &[u8; 56]) -> Result<()> {
        match decode_controller_record(bytes) {
            // ADDED carries a device index. Opening the whole final device list here
            // would open later arrivals before queued removals release their ordinals.
            ControllerRecord::Added {
                timestamp,
                device_index,
            } => self.discover(timestamp, Some(device_index)),
            ControllerRecord::Axis {
                timestamp,
                instance,
                axis,
                value,
            } => {
                if (axis as usize) < AXIS_COUNT {
                    if let Some(device) = self.opened.iter_mut().find(|d| d.instance == instance) {
                        device.axes[axis as usize] = value;
                    } else {
                        return Ok(());
                    }
                    let slot = self.slot(instance);
                    self.pending.push(ControllerEvent::Axis {
                        timestamp,
                        instance,
                        slot,
                        axis,
                        value,
                    });
                }
                Ok(())
            }
            ControllerRecord::Button {
                timestamp,
                instance,
                button,
                down,
            } => {
                if (button as usize) < BUTTON_COUNT {
                    if let Some(device) = self.opened.iter_mut().find(|d| d.instance == instance) {
                        device.buttons[button as usize] = down;
                    } else {
                        return Ok(());
                    }
                    let slot = self.slot(instance);
                    self.pending.push(ControllerEvent::Button {
                        timestamp,
                        instance,
                        slot,
                        button,
                        down,
                    });
                }
                Ok(())
            }
            ControllerRecord::Removed { timestamp, instance } => {
                if self.opened.iter().any(|d| d.instance == instance) {
                    self.remove(instance, timestamp);
                }
                Ok(())
            }
            ControllerRecord::Remapped { timestamp, instance } => {
                let sdl = Arc::clone(&self.sdl);
                {
                    let Some(device) = self.opened.iter_mut().find(|d| d.instance == instance) else {
                        return Ok(());
                    };
                    // SAFETY: the device handle is live; state is refreshed below.
                    unsafe {
                        let (guid, name, serial) = Self::identity(&sdl, device.pointer)?;
                        device.guid = guid;
                        device.name = name;
                        device.serial = serial;
                        for axis in 0..AXIS_COUNT {
                            device.axes[axis] = (sdl.sdl_game_controller_get_axis)(device.pointer, axis as i32);
                        }
                        for button in 0..BUTTON_COUNT {
                            device.buttons[button] =
                                (sdl.sdl_game_controller_get_button)(device.pointer, button as i32) != 0;
                        }
                    }
                }
                let description = {
                    let device = self
                        .opened
                        .iter()
                        .find(|d| d.instance == instance)
                        .expect("just updated");
                    // SAFETY: the device handle is live.
                    unsafe { self.describe(device)? }
                };
                self.pending.push(ControllerEvent::Remapped {
                    timestamp,
                    device: description,
                });
                self.resolve(timestamp);
                Ok(())
            }
            ControllerRecord::Touchpad {
                timestamp,
                instance,
                phase,
                touchpad,
                finger,
                x,
                y,
                pressure,
            } => {
                if !self.opened.iter().any(|d| d.instance == instance) {
                    return Ok(());
                }
                let slot = self.slot(instance);
                self.pending.push(ControllerEvent::Touchpad {
                    timestamp,
                    instance,
                    slot,
                    phase,
                    touchpad,
                    finger,
                    x,
                    y,
                    pressure,
                });
                Ok(())
            }
            ControllerRecord::Sensor {
                timestamp,
                instance,
                sensor,
                x,
                y,
                z,
                timestamp_us,
            } => {
                if !self.opened.iter().any(|d| d.instance == instance) {
                    return Ok(());
                }
                let slot = self.slot(instance);
                self.pending.push(ControllerEvent::Sensor {
                    timestamp,
                    instance,
                    slot,
                    sensor,
                    x,
                    y,
                    z,
                    timestamp_us,
                });
                Ok(())
            }
            ControllerRecord::Unrecognized {
                timestamp,
                instance,
                event_type,
            } => {
                if !self.opened.iter().any(|d| d.instance == instance) {
                    return Ok(());
                }
                let slot = self.slot(instance);
                self.pending.push(ControllerEvent::Unrecognized {
                    timestamp,
                    instance,
                    slot,
                    event_type,
                    bytes: *bytes,
                });
                Ok(())
            }
        }
    }

    /// Drain pending events.
    pub fn poll_events(&mut self) -> Result<Vec<ControllerEvent>> {
        self.require_open()?;
        self.collect()?;
        Ok(std::mem::take(&mut self.pending))
    }

    /// Add a controller mapping line.
    pub fn add_mapping(&mut self, mapping: &str) -> Result<MappingOutcome> {
        self.require_open()?;
        validate_mapping(mapping)?;
        let line = c_string(mapping)?;
        // SAFETY: the mapping buffer is live for the call.
        let result = unsafe {
            self.sdl.checked(
                (self.sdl.sdl_game_controller_add_mapping)(line.as_ptr()),
                "SDL_GameControllerAddMapping",
            )?
        };
        // SAFETY: no arguments.
        let ticks = unsafe { (self.sdl.sdl_get_ticks)() };
        self.discover(ticks, None)?;
        Ok(if result == 0 {
            MappingOutcome::Updated
        } else {
            MappingOutcome::Added
        })
    }

    /// Capability-gated operation on one device.
    fn operate(
        &self,
        instance: i32,
        capability: ControllerCapability,
        mut operation: impl FnMut(*mut c_void) -> i32,
    ) -> Result<ControllerOperationResult> {
        self.require_open()?;
        let Some(device) = self.opened.iter().find(|device| device.instance == instance) else {
            return Ok(ControllerOperationResult::Disconnected {
                reason: format!("SDL controller {instance} is not connected"),
            });
        };
        // SAFETY: the controller handle is live.
        unsafe {
            if (self.sdl.sdl_game_controller_get_attached)(device.pointer) == 0 {
                return Ok(ControllerOperationResult::Disconnected {
                    reason: format!("SDL controller {instance} is not connected"),
                });
            }
            let support = Self::capabilities(&self.sdl, device.pointer)?;
            let supported = match capability {
                ControllerCapability::Rumble => support.rumble,
                ControllerCapability::TriggerRumble => support.trigger_rumble,
                ControllerCapability::Led => support.led,
                ControllerCapability::Sensor(kind) => support.sensors.iter().any(|sensor| sensor.kind == kind),
            };
            if !supported {
                return Ok(ControllerOperationResult::Unsupported {
                    reason: format!("{} does not support {}", device.name, capability.name()),
                });
            }
            if operation(device.pointer) >= 0 {
                return Ok(ControllerOperationResult::Accepted);
            }
            let detail = self.sdl.error();
            Ok(ControllerOperationResult::Failed {
                reason: if detail.is_empty() {
                    format!(
                        "SDL controller {} failed without an SDL error message",
                        capability.name()
                    )
                } else {
                    detail
                },
            })
        }
    }

    /// Rumble with low/high amplitudes in 0..=1 for a duration in ms.
    pub fn rumble(&self, instance: i32, low: f64, high: f64, duration_ms: u32) -> Result<ControllerOperationResult> {
        let low_magnitude = rumble_magnitude(low)?;
        let high_magnitude = rumble_magnitude(high)?;
        // SAFETY: magnitudes and duration are validated.
        unsafe {
            self.operate(instance, ControllerCapability::Rumble, |pointer| {
                (self.sdl.sdl_game_controller_rumble)(pointer, low_magnitude, high_magnitude, duration_ms)
            })
        }
    }

    /// Rumble triggers with left/right amplitudes in 0..=1 for a duration in ms.
    pub fn rumble_triggers(
        &self,
        instance: i32,
        left: f64,
        right: f64,
        duration_ms: u32,
    ) -> Result<ControllerOperationResult> {
        let left_magnitude = rumble_magnitude(left)?;
        let right_magnitude = rumble_magnitude(right)?;
        // SAFETY: magnitudes and duration are validated.
        unsafe {
            self.operate(instance, ControllerCapability::TriggerRumble, |pointer| {
                (self.sdl.sdl_game_controller_rumble_triggers)(pointer, left_magnitude, right_magnitude, duration_ms)
            })
        }
    }

    /// Set the controller LED color.
    pub fn set_led(&self, instance: i32, red: u8, green: u8, blue: u8) -> Result<ControllerOperationResult> {
        // SAFETY: color components are validated by their type.
        unsafe {
            self.operate(instance, ControllerCapability::Led, |pointer| {
                (self.sdl.sdl_game_controller_set_led)(pointer, red, green, blue)
            })
        }
    }

    /// Enable or disable a sensor.
    pub fn set_sensor_enabled(
        &self,
        instance: i32,
        sensor: ControllerSensor,
        enabled: bool,
    ) -> Result<ControllerOperationResult> {
        // SAFETY: the sensor id is validated by its type.
        unsafe {
            self.operate(instance, ControllerCapability::Sensor(sensor), |pointer| {
                (self.sdl.sdl_game_controller_set_sensor_enabled)(pointer, sensor.sdl_id(), i32::from(enabled))
            })
        }
    }

    /// Read one sensor sample.
    pub fn read_sensor(&self, instance: i32, sensor: ControllerSensor) -> Result<SensorRead> {
        let mut values = [0f32; 3];
        // SAFETY: the sensor id and sample buffer are validated; the buffer
        // is live for the call.
        let result = unsafe {
            self.operate(instance, ControllerCapability::Sensor(sensor), |pointer| {
                (self.sdl.sdl_game_controller_get_sensor_data)(pointer, sensor.sdl_id(), values.as_mut_ptr(), 3)
            })?
        };
        if result != ControllerOperationResult::Accepted {
            return Ok(SensorRead::Status(result));
        }
        Ok(SensorRead::Sample {
            x: values[0],
            y: values[1],
            z: values[2],
        })
    }

    fn stop(&self, instance: i32) {
        let Some(device) = self.opened.iter().find(|device| device.instance == instance) else {
            return;
        };
        // SAFETY: the controller handle is live.
        unsafe {
            if (self.sdl.sdl_game_controller_get_attached)(device.pointer) == 0 {
                return;
            }
            if (self.sdl.sdl_game_controller_has_rumble)(device.pointer) != 0 {
                (self.sdl.sdl_game_controller_rumble)(device.pointer, 0, 0, 0);
            }
            if (self.sdl.sdl_game_controller_has_rumble_triggers)(device.pointer) != 0 {
                (self.sdl.sdl_game_controller_rumble_triggers)(device.pointer, 0, 0, 0);
            }
        }
    }

    /// Close the provider and quit the subsystem. Idempotent.
    pub fn close(&mut self) {
        if thread::current().id() != self.owner || self.is_closed() {
            return;
        }
        let instances: Vec<i32> = self.opened.iter().map(|device| device.instance).collect();
        for instance in instances {
            self.stop(instance);
        }
        for device in self.opened.drain(..) {
            // SAFETY: the controller handle is live until this call.
            unsafe {
                (self.sdl.sdl_game_controller_close)(device.pointer);
            }
        }
        self.pending.clear();
        self.routes.clear();
        self.selections.clear();
        self.sdl_event_state(self.previous_event_state);
        // SAFETY: quits the subsystem this provider initialized.
        unsafe {
            (self.sdl.sdl_quit_sub_system)(SUBSYSTEM);
        }
        *controller_owner().lock().expect("owner") = None;
    }
}

impl Drop for SdlControllers {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Clone, Copy)]
enum ControllerCapability {
    Rumble,
    TriggerRumble,
    Led,
    Sensor(ControllerSensor),
}

impl ControllerCapability {
    fn name(self) -> &'static str {
        match self {
            Self::Rumble => "rumble",
            Self::TriggerRumble => "triggerRumble",
            Self::Led => "led",
            Self::Sensor(sensor) => sensor.name(),
        }
    }
}

struct VirtualSdl {
    _lib: LoadedLibrary,
    sdl_get_error: unsafe extern "C" fn() -> *const u8,
    sdl_joystick_attach_virtual_ex: unsafe extern "C" fn(*const u8) -> i32,
    sdl_joystick_detach_virtual: unsafe extern "C" fn(i32) -> i32,
    sdl_joystick_open: unsafe extern "C" fn(i32) -> *mut c_void,
    sdl_joystick_close: unsafe extern "C" fn(*mut c_void),
    sdl_joystick_set_virtual_axis: unsafe extern "C" fn(*mut c_void, i32, i16) -> i32,
    sdl_joystick_set_virtual_button: unsafe extern "C" fn(*mut c_void, i32, u8) -> i32,
    sdl_joystick_update: unsafe extern "C" fn(),
}

define_controller_loader! { VirtualSdl,
    sdl_get_error: "SDL_GetError";
    sdl_joystick_attach_virtual_ex: "SDL_JoystickAttachVirtualEx";
    sdl_joystick_detach_virtual: "SDL_JoystickDetachVirtual";
    sdl_joystick_open: "SDL_JoystickOpen";
    sdl_joystick_close: "SDL_JoystickClose";
    sdl_joystick_set_virtual_axis: "SDL_JoystickSetVirtualAxis";
    sdl_joystick_set_virtual_button: "SDL_JoystickSetVirtualButton";
    sdl_joystick_update: "SDL_JoystickUpdate";
}

/// Rumble effect captured by a virtual controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VirtualControllerRumble {
    /// Main rumble motors.
    Rumble {
        /// Low-frequency magnitude.
        low: u16,
        /// High-frequency magnitude.
        high: u16,
    },
    /// Trigger rumble motors.
    TriggerRumble {
        /// Left magnitude.
        low: u16,
        /// Right magnitude.
        high: u16,
    },
}

type RumbleSink = Mutex<Vec<VirtualControllerRumble>>;

unsafe extern "C" fn virtual_rumble(userdata: *mut c_void, low: u16, high: u16) -> i32 {
    if !userdata.is_null() {
        // SAFETY: userdata is the leaked rumble sink for this joystick.
        let sink = unsafe { &*(userdata as *const RumbleSink) };
        if let Ok(mut effects) = sink.lock() {
            effects.push(VirtualControllerRumble::Rumble { low, high });
        }
    }
    0
}

unsafe extern "C" fn virtual_trigger_rumble(userdata: *mut c_void, low: u16, high: u16) -> i32 {
    if !userdata.is_null() {
        // SAFETY: userdata is the leaked rumble sink for this joystick.
        let sink = unsafe { &*(userdata as *const RumbleSink) };
        if let Ok(mut effects) = sink.lock() {
            effects.push(VirtualControllerRumble::TriggerRumble { low, high });
        }
    }
    0
}

/// Virtual controller attach options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VirtualControllerOptions {
    /// Device name.
    pub name: Option<String>,
    /// Install rumble capture callbacks.
    pub rumble: bool,
}

/// A real SDL virtual joystick, for platform diagnostics without fabricated input records.
pub struct VirtualSdlController {
    sdl: Arc<ControllerSdl>,
    virtual_sdl: Arc<VirtualSdl>,
    pointer: *mut c_void,
    instance: i32,
    sink: *mut RumbleSink,
    name_bytes: Vec<u8>,
    owner: thread::ThreadId,
    _not_send: PhantomData<*const ()>,
}

impl VirtualSdlController {
    /// Attach a virtual joystick.
    pub fn attach(options: &VirtualControllerOptions) -> Result<Self> {
        Self::attach_with(options, &NativeLibraryOptions::default())
    }

    /// Attach with explicit library discovery (tests inject overrides).
    pub fn attach_with(options: &VirtualControllerOptions, lib_options: &NativeLibraryOptions) -> Result<Self> {
        // SAFETY: loading maps images without invoking their code.
        let sdl = unsafe { ControllerSdl::load(lib_options)? };
        // SAFETY: subsystem initialization uses validated arguments.
        unsafe {
            SdlControllers::initialize(&sdl)?;
        }
        let result = Self::attach_inner(&sdl, options, lib_options);
        if result.is_err() {
            // SAFETY: quits the subsystem initialized above.
            unsafe {
                (sdl.sdl_quit_sub_system)(SUBSYSTEM);
            }
        }
        result
    }

    fn attach_inner(
        sdl: &Arc<ControllerSdl>,
        options: &VirtualControllerOptions,
        lib_options: &NativeLibraryOptions,
    ) -> Result<Self> {
        #[cfg(not(target_pointer_width = "64"))]
        {
            let _ = (sdl, options, lib_options);
            return Err(Error::Unsupported(
                "SDL virtual descriptor requires a 64-bit host".to_string(),
            ));
        }
        #[cfg(target_pointer_width = "64")]
        {
            // SAFETY: loading maps the image; the descriptor layout below
            // matches SDL2's virtual joystick descriptor.
            let virtual_sdl = unsafe { VirtualSdl::load(lib_options)? };
            let name = c_string(options.name.as_deref().unwrap_or("Quake SDL virtual controller"))?;
            let sink: *mut RumbleSink = Box::into_raw(Box::new(Mutex::new(Vec::new())));
            let mut descriptor = [0u8; 88];
            descriptor[0..2].copy_from_slice(&1u16.to_ne_bytes());
            descriptor[2..4].copy_from_slice(&1u16.to_ne_bytes());
            descriptor[4..6].copy_from_slice(&(AXIS_COUNT as u16).to_ne_bytes());
            descriptor[6..8].copy_from_slice(&(BUTTON_COUNT as u16).to_ne_bytes());
            descriptor[16..20].copy_from_slice(&((1u32 << BUTTON_COUNT) - 1).to_ne_bytes());
            descriptor[20..24].copy_from_slice(&((1u32 << AXIS_COUNT) - 1).to_ne_bytes());
            descriptor[24..32].copy_from_slice(&(name.as_ptr() as u64).to_ne_bytes());
            descriptor[32..40].copy_from_slice(&(sink as u64).to_ne_bytes());
            if options.rumble {
                descriptor[56..64].copy_from_slice(&(virtual_rumble as *const () as usize as u64).to_ne_bytes());
                descriptor[64..72]
                    .copy_from_slice(&(virtual_trigger_rumble as *const () as usize as u64).to_ne_bytes());
            }
            // SAFETY: the descriptor, name, and sink are live for the attach call.
            unsafe {
                let index = match virtual_sdl.checked(
                    (virtual_sdl.sdl_joystick_attach_virtual_ex)(descriptor.as_ptr()),
                    "SDL_JoystickAttachVirtualEx",
                ) {
                    Ok(index) => index,
                    Err(error) => {
                        drop(Box::from_raw(sink));
                        return Err(error);
                    }
                };
                let pointer = (virtual_sdl.sdl_joystick_open)(index);
                if pointer.is_null() {
                    (virtual_sdl.sdl_joystick_detach_virtual)(index);
                    drop(Box::from_raw(sink));
                    return Err(Error::native("SDL_JoystickOpen virtual", sdl.error()));
                }
                let instance = match sdl.checked(
                    (sdl.sdl_joystick_instance_id)(pointer),
                    "SDL_JoystickInstanceID virtual",
                ) {
                    Ok(instance) => instance,
                    Err(error) => {
                        (virtual_sdl.sdl_joystick_detach_virtual)(index);
                        (virtual_sdl.sdl_joystick_close)(pointer);
                        drop(Box::from_raw(sink));
                        return Err(error);
                    }
                };
                // Virtual trigger joysticks use -32768 for the unpressed end of their mapping.
                for axis in [4, 5] {
                    if let Err(error) = virtual_sdl.checked(
                        (virtual_sdl.sdl_joystick_set_virtual_axis)(pointer, axis, -32768),
                        "SDL_JoystickSetVirtualAxis trigger",
                    ) {
                        (virtual_sdl.sdl_joystick_detach_virtual)(index);
                        (virtual_sdl.sdl_joystick_close)(pointer);
                        drop(Box::from_raw(sink));
                        return Err(error);
                    }
                }
                (virtual_sdl.sdl_joystick_update)();
                Ok(Self {
                    sdl: Arc::clone(sdl),
                    virtual_sdl,
                    pointer,
                    instance,
                    sink,
                    name_bytes: name,
                    owner: thread::current().id(),
                    _not_send: PhantomData,
                })
            }
        }
    }

    /// Device instance id.
    #[must_use]
    pub fn instance(&self) -> i32 {
        self.instance
    }

    /// Whether the controller is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.pointer.is_null()
    }

    fn opened(&self) -> Result<*mut c_void> {
        if thread::current().id() != self.owner {
            return Err(Error::InvalidInput(
                "SDL controllers must be owned by the owning thread".to_string(),
            ));
        }
        if self.pointer.is_null() {
            return Err(Error::Closed("SDL virtual controller".to_string()));
        }
        Ok(self.pointer)
    }

    /// Set a virtual axis value.
    pub fn set_axis(&self, axis: usize, value: i16) -> Result<()> {
        let pointer = self.opened()?;
        if axis >= AXIS_COUNT {
            return Err(Error::OutOfRange("controller axis exceeds int32".to_string()));
        }
        // SAFETY: the joystick handle is live; arguments are validated.
        unsafe {
            self.virtual_sdl.checked(
                (self.virtual_sdl.sdl_joystick_set_virtual_axis)(pointer, axis as i32, value),
                "SDL_JoystickSetVirtualAxis",
            )?;
            (self.virtual_sdl.sdl_joystick_update)();
        }
        Ok(())
    }

    /// Set a virtual button state.
    pub fn set_button(&self, button: usize, down: bool) -> Result<()> {
        let pointer = self.opened()?;
        if button >= BUTTON_COUNT {
            return Err(Error::OutOfRange("controller button exceeds int32".to_string()));
        }
        // SAFETY: the joystick handle is live; arguments are validated.
        unsafe {
            self.virtual_sdl.checked(
                (self.virtual_sdl.sdl_joystick_set_virtual_button)(pointer, button as i32, u8::from(down)),
                "SDL_JoystickSetVirtualButton",
            )?;
            (self.virtual_sdl.sdl_joystick_update)();
        }
        Ok(())
    }

    /// Drain captured rumble effects.
    pub fn drain_rumble(&self) -> Result<Vec<VirtualControllerRumble>> {
        self.opened()?;
        // SAFETY: the sink is live until close reclaims it.
        let sink = unsafe { &*self.sink };
        Ok(sink
            .lock()
            .map_err(|_| Error::native("SDL virtual rumble", "effect sink is poisoned".to_string()))?
            .drain(..)
            .collect())
    }

    /// Detach the virtual joystick. Idempotent.
    pub fn close(&mut self) {
        if thread::current().id() != self.owner || self.pointer.is_null() {
            return;
        }
        let pointer = self.pointer;
        self.pointer = std::ptr::null_mut();
        // SAFETY: handles are live until their release calls below.
        unsafe {
            if let Ok(count) = self
                .sdl
                .checked((self.sdl.sdl_num_joysticks)(), "SDL_NumJoysticks virtual detach")
            {
                for index in 0..count {
                    if (self.sdl.sdl_joystick_get_device_instance_id)(index) == self.instance {
                        (self.virtual_sdl.sdl_joystick_detach_virtual)(index);
                        break;
                    }
                }
            }
            (self.virtual_sdl.sdl_joystick_close)(pointer);
            drop(Box::from_raw(self.sink));
            (self.sdl.sdl_quit_sub_system)(SUBSYSTEM);
        }
        let _ = &self.name_bytes;
    }
}

impl Drop for VirtualSdlController {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn missing_lib() -> NativeLibraryOptions {
        let mut environment = HashMap::new();
        environment.insert(
            "QUAKE_SDL2_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL2.so".to_string(),
        );
        NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        }
    }

    fn record() -> [u8; 56] {
        [0u8; 56]
    }

    fn put_u32(bytes: &mut [u8; 56], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
    }

    fn put_i32(bytes: &mut [u8; 56], offset: usize, value: i32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
    }

    #[test]
    fn decode_axis_button_touchpad_sensor() {
        let mut axis = record();
        put_u32(&mut axis, 0, 0x650);
        put_u32(&mut axis, 4, 11);
        put_i32(&mut axis, 8, 3);
        axis[12] = 4;
        axis[16..18].copy_from_slice(&(-32768i16).to_ne_bytes());
        assert!(matches!(
            decode_controller_record(&axis),
            ControllerRecord::Axis {
                timestamp: 11,
                instance: 3,
                axis: 4,
                value: -32768
            }
        ));
        let mut button = record();
        put_u32(&mut button, 0, 0x651);
        put_i32(&mut button, 8, 3);
        button[12] = 7;
        assert!(matches!(
            decode_controller_record(&button),
            ControllerRecord::Button {
                button: 7,
                down: true,
                ..
            }
        ));
        let mut added = record();
        put_u32(&mut added, 0, 0x653);
        put_i32(&mut added, 8, 2);
        assert!(matches!(
            decode_controller_record(&added),
            ControllerRecord::Added { device_index: 2, .. }
        ));
        let mut touch = record();
        put_u32(&mut touch, 0, 0x657);
        put_i32(&mut touch, 8, 3);
        put_i32(&mut touch, 12, 1);
        put_i32(&mut touch, 16, 2);
        touch[20..24].copy_from_slice(&0.5f32.to_ne_bytes());
        touch[24..28].copy_from_slice(&0.25f32.to_ne_bytes());
        touch[28..32].copy_from_slice(&1.0f32.to_ne_bytes());
        assert!(matches!(
            decode_controller_record(&touch),
            ControllerRecord::Touchpad {
                phase: TouchpadPhase::Motion,
                touchpad: 1,
                finger: 2,
                ..
            }
        ));
        let mut sensor = record();
        put_u32(&mut sensor, 0, 0x659);
        put_i32(&mut sensor, 8, 3);
        put_i32(&mut sensor, 12, 2);
        sensor[16..20].copy_from_slice(&1.0f32.to_ne_bytes());
        sensor[20..24].copy_from_slice(&2.0f32.to_ne_bytes());
        sensor[24..28].copy_from_slice(&3.0f32.to_ne_bytes());
        sensor[32..40].copy_from_slice(&99u64.to_ne_bytes());
        assert!(matches!(
            decode_controller_record(&sensor),
            ControllerRecord::Sensor {
                sensor: ControllerSensor::Gyro,
                timestamp_us: 99,
                ..
            }
        ));
        let mut unknown_sensor = record();
        put_u32(&mut unknown_sensor, 0, 0x659);
        put_i32(&mut unknown_sensor, 12, 42);
        assert!(matches!(
            decode_controller_record(&unknown_sensor),
            ControllerRecord::Unrecognized { .. }
        ));
        let mut unknown = record();
        put_u32(&mut unknown, 0, 0x66e);
        assert!(matches!(
            decode_controller_record(&unknown),
            ControllerRecord::Unrecognized { event_type: 0x66e, .. }
        ));
    }

    #[test]
    fn selections_validate() {
        let guid = "00112233445566778899aabbccddeeff";
        assert!(validate_selection(&ControllerSelection::Automatic).is_ok());
        assert!(validate_selection(&ControllerSelection::None).is_ok());
        assert!(validate_selection(&ControllerSelection::Device {
            guid: guid.to_string(),
            ordinal: 0
        })
        .is_ok());
        assert!(validate_selection(&ControllerSelection::Device {
            guid: "short".to_string(),
            ordinal: 0
        })
        .is_err());
        assert!(validate_selection(&ControllerSelection::Serial {
            guid: guid.to_string(),
            serial: String::new()
        })
        .is_err());
        assert!(validate_selection(&ControllerSelection::Serial {
            guid: guid.to_string(),
            serial: "sn-1".to_string()
        })
        .is_ok());
        assert!(validate_mapping("00112233445566778899aabbccddeeff,Pad,a:b0").is_ok());
        assert!(validate_mapping("short,Pad,a:b0").is_err());
        assert!(validate_mapping("00112233445566778899aabbccddeeff,,a:b0").is_err());
    }

    #[test]
    fn assignments_prefer_explicit_then_automatic() {
        let guid_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let guid_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();
        let devices = vec![
            AssignDevice {
                instance: 10,
                guid: Some(guid_a.clone()),
                ordinal: 0,
                serial: Some("s0".to_string()),
            },
            AssignDevice {
                instance: 11,
                guid: Some(guid_a.clone()),
                ordinal: 1,
                serial: Some("s1".to_string()),
            },
            AssignDevice {
                instance: 12,
                guid: Some(guid_b.clone()),
                ordinal: 0,
                serial: None,
            },
        ];
        // Explicit device claim.
        let routes = resolve_assignments(
            &[ControllerSelection::Device {
                guid: guid_a.clone(),
                ordinal: 1,
            }],
            &devices,
        );
        assert_eq!(routes, vec![Some(11)]);
        // Ambiguous explicit claim (two devices share the GUID/serial shape
        // when matching serial-less) stays empty; serial disambiguates.
        let routes = resolve_assignments(
            &[ControllerSelection::Serial {
                guid: guid_a.clone(),
                serial: "s0".to_string(),
            }],
            &devices,
        );
        assert_eq!(routes, vec![Some(10)]);
        // Automatic claims the first unclaimed device in order.
        let routes = resolve_assignments(
            &[ControllerSelection::Automatic, ControllerSelection::Automatic],
            &devices,
        );
        assert_eq!(routes, vec![Some(10), Some(11)]);
        // Explicit claims win over automatic regardless of slot order.
        let routes = resolve_assignments(
            &[
                ControllerSelection::Automatic,
                ControllerSelection::Device {
                    guid: guid_b.clone(),
                    ordinal: 0,
                },
            ],
            &devices,
        );
        assert_eq!(routes, vec![Some(10), Some(12)]);
        // None leaves the slot empty.
        let routes = resolve_assignments(&[ControllerSelection::None], &devices);
        assert_eq!(routes, vec![None]);
    }

    #[test]
    fn rumble_magnitudes_scale() {
        assert_eq!(rumble_magnitude(0.0).unwrap(), 0);
        assert_eq!(rumble_magnitude(1.0).unwrap(), 65535);
        assert_eq!(rumble_magnitude(0.5).unwrap(), 32768);
        assert!(rumble_magnitude(-0.1).is_err());
        assert!(rumble_magnitude(f64::NAN).is_err());
        assert!(rumble_magnitude(f64::INFINITY).is_err());
    }

    #[test]
    fn missing_sdl_names_library() {
        let options = missing_lib();
        let Err(error) = SdlControllers::open_with(&options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("sdl2"), "{error}");
        let Err(error) = SdlControllers::runtime_with(&options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        let Err(error) = VirtualSdlController::attach_with(&VirtualControllerOptions::default(), &options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
    }

    #[test]
    fn live_provider_reports_honestly() {
        match SdlControllers::open() {
            Ok(mut provider) => {
                assert!(!provider.is_closed());
                let _ = provider.devices().unwrap();
                provider.set_assignments(&[ControllerSelection::Automatic]).unwrap();
                let _ = provider.poll_events().unwrap();
                provider.close();
                assert!(provider.is_closed());
                // Second owner while open is refused; after close it may proceed.
                let _ = SdlControllers::open().map(|mut second| second.close());
            }
            Err(error) => assert!(!error.to_string().is_empty(), "{error}"),
        }
        match VirtualSdlController::attach(&VirtualControllerOptions {
            name: Some("qa-platform test pad".to_string()),
            rumble: true,
        }) {
            Ok(mut pad) => {
                pad.set_axis(0, 1000).unwrap();
                pad.set_button(3, true).unwrap();
                assert!(pad.set_axis(6, 0).is_err());
                assert!(pad.set_button(21, false).is_err());
                let _ = pad.drain_rumble().unwrap();
                pad.close();
                assert!(pad.is_closed());
            }
            Err(error) => assert!(!error.to_string().is_empty(), "{error}"),
        }
    }
}
