//! Per-seat event dispatch and platform routing.
//!
//! Donor provenance: the `SeatInput` event half of `src/input/seat.ts`
//! (held inputs, buttons, mouse, focus, sampling) and
//! `src/input/router.ts` (`InputRouter`). The binding-table half of
//! seat state lives in [`super::SeatKeys`]/[`super::BindingTable`];
//! [`Seat`] composes those with buttons, gamepad, and command output.

use std::collections::{HashMap, HashSet};

use qa_core::cmd::{command_separator_offset, source_command_text, Dialect};
use qa_core::identity::SeatId;
use qa_core::math::{vec2, Vec2, Vec3};
use qa_platform::controller::{ControllerEvent, SdlControllers};
use qa_platform::controller::{ControllerOperationResult, ControllerSelection, ControllerSensor, ControllerState};
use qa_platform::sdl::{SdlEvent, SdlInputLease, SdlWindow};
use thiserror::Error;

use super::bindings::ButtonSeat;
use super::commands::CommandRegistry;
use super::gamepad::{controller_axis_name, normalized_controller_axis, GamepadError, GamepadInput};
use super::sdl_keys::{sdl_event_time, sdl_game_key};
use super::{
    quake_mouse_button, BindingTable, ButtonSample, ButtonTiming, InputBinding, InputBindingTarget, InputButton,
    KeyCode, PhysicalInput, SeatFocus, SourceAction,
};

/// Seat error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SeatError {
    /// Input event delivered to the wrong seat.
    #[error("Input event delivered to the wrong seat")]
    WrongSeat,
    /// Invalid input event timestamp.
    #[error("Invalid input event timestamp")]
    BadTimestamp,
    /// Input sample requires a positive frame duration.
    #[error("Input sample requires a positive frame duration")]
    BadSample,
    /// Input profile requires released keys.
    #[error("Input profile requires released keys")]
    ProfileHeld,
    /// Gamepad error.
    #[error(transparent)]
    Gamepad(#[from] GamepadError),
}

/// Router error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RouterError {
    /// Duplicate input seat.
    #[error("Duplicate input seat")]
    DuplicateSeat,
    /// Unknown input seat.
    #[error("Unknown input seat")]
    UnknownSeat,
    /// Keyboard seat is not published.
    #[error("Keyboard seat is not published")]
    KeyboardNotPublished,
    /// Keyboard seat is not retained.
    #[error("Keyboard seat is not retained")]
    KeyboardNotRetained,
    /// Retained input seats must be distinct current owners.
    #[error("Retained input seats must be distinct current owners")]
    BadRetainedSeats,
    /// Source joystick seat is not active.
    #[error("Source joystick seat is not active")]
    BadSourceSeat,
    /// Keyboard route refers to an unregistered seat.
    #[error("Keyboard route refers to an unregistered seat")]
    BadKeyboardSeat,
    /// Gyro route refers to an unregistered seat.
    #[error("Gyro route refers to an unregistered seat")]
    BadGyroSeat,
    /// Input router is closed.
    #[error("Input router is closed")]
    Closed,
    /// Seat error.
    #[error(transparent)]
    Seat(#[from] SeatError),
    /// Platform backend error.
    #[error("Platform input error: {0}")]
    Platform(String),
}

/// Seat input event.
#[derive(Debug, Clone, PartialEq)]
pub enum SeatInputEvent {
    /// Window focus changed.
    Focus {
        /// Seat id.
        seat: SeatId,
        /// Event time in ms.
        time_ms: f64,
        /// Window focused.
        focused: bool,
    },
    /// Key transition.
    Key {
        /// Seat id.
        seat: SeatId,
        /// Event time in ms.
        time_ms: f64,
        /// Quake key code.
        code: i32,
        /// Press (false for release).
        down: bool,
    },
    /// Text input (menus only).
    Text {
        /// Seat id.
        seat: SeatId,
        /// Event time in ms.
        time_ms: f64,
        /// UTF-8 text.
        text: String,
    },
    /// Mouse motion.
    MouseMotion {
        /// Seat id.
        seat: SeatId,
        /// Event time in ms.
        time_ms: f64,
        /// Pointer position.
        position: (f64, f64),
        /// Motion delta.
        delta: (f64, f64),
    },
    /// Mouse button transition.
    MouseButton {
        /// Seat id.
        seat: SeatId,
        /// Event time in ms.
        time_ms: f64,
        /// Physical button id.
        button: u8,
        /// Press (false for release).
        down: bool,
    },
    /// Mouse wheel motion.
    MouseWheel {
        /// Seat id.
        seat: SeatId,
        /// Event time in ms.
        time_ms: f64,
        /// Wheel delta.
        delta: (f64, f64),
    },
    /// Controller button transition.
    ControllerButton {
        /// Seat id.
        seat: SeatId,
        /// Event time in ms.
        time_ms: f64,
        /// Device instance.
        device: i32,
        /// Button index.
        button: u8,
        /// Press (false for release).
        down: bool,
    },
    /// Controller axis motion.
    ControllerAxis {
        /// Seat id.
        seat: SeatId,
        /// Event time in ms.
        time_ms: f64,
        /// Device instance.
        device: i32,
        /// Axis name.
        axis: super::ControllerAxis,
        /// Normalized value.
        value: f64,
    },
}

impl SeatInputEvent {
    fn seat(&self) -> &SeatId {
        match self {
            SeatInputEvent::Focus { seat, .. }
            | SeatInputEvent::Key { seat, .. }
            | SeatInputEvent::Text { seat, .. }
            | SeatInputEvent::MouseMotion { seat, .. }
            | SeatInputEvent::MouseButton { seat, .. }
            | SeatInputEvent::MouseWheel { seat, .. }
            | SeatInputEvent::ControllerButton { seat, .. }
            | SeatInputEvent::ControllerAxis { seat, .. } => seat,
        }
    }

    fn time_ms(&self) -> f64 {
        match self {
            SeatInputEvent::Focus { time_ms, .. }
            | SeatInputEvent::Key { time_ms, .. }
            | SeatInputEvent::Text { time_ms, .. }
            | SeatInputEvent::MouseMotion { time_ms, .. }
            | SeatInputEvent::MouseButton { time_ms, .. }
            | SeatInputEvent::MouseWheel { time_ms, .. }
            | SeatInputEvent::ControllerButton { time_ms, .. }
            | SeatInputEvent::ControllerAxis { time_ms, .. } => *time_ms,
        }
    }
}

/// One sampled seat frame.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatFrame {
    /// Seat id.
    pub seat: SeatId,
    /// Sample time in ms.
    pub now_ms: f64,
    /// Frame length in ms.
    pub frame_ms: f64,
    /// Focus at sample time.
    pub focus: SeatFocus,
    /// Sampled buttons in creation order.
    pub buttons: Vec<ButtonSample>,
    /// Accumulated mouse delta.
    pub mouse: Vec2,
    /// Gamepad move stick.
    pub gamepad_move: Vec2,
    /// Gamepad look degrees.
    pub gamepad_look_degrees: Vec2,
    /// Pending impulse.
    pub impulse: i32,
    /// Held input count.
    pub any_key_down: usize,
}

/// UI event callback: returns true when the UI consumes the event.
pub type UiCallback = Box<dyn FnMut(&SeatInputEvent, &SeatFocus) -> bool>;

struct UiEntry {
    id: u64,
    callback: UiCallback,
    focus: SeatFocus,
    active: bool,
}

#[derive(Debug, Clone)]
struct HeldBinding {
    input: PhysicalInput,
    target: Option<InputBindingTarget>,
}

fn command_key(input: &PhysicalInput) -> i32 {
    match input {
        PhysicalInput::Key(code) => *code,
        PhysicalInput::MouseButton(button) => KeyCode::Mouse1 as i32 + quake_mouse_button(*button) - 1,
        PhysicalInput::ControllerButton { device, button } => 65536 + device * 64 + button,
        PhysicalInput::ControllerAxis {
            device,
            axis,
            direction,
        } => {
            let index = match axis {
                super::ControllerAxis::LeftX => 0,
                super::ControllerAxis::LeftY => 1,
                super::ControllerAxis::RightX => 2,
                super::ControllerAxis::RightY => 3,
                super::ControllerAxis::LeftTrigger => 4,
                super::ControllerAxis::RightTrigger => 5,
            };
            65536 + device * 64 + 32 + index * 2 + i32::from(*direction == super::AxisDirection::Positive)
        }
    }
}

/// Live per-seat input: bindings, buttons, mouse, gamepad, and focus.
pub struct Seat {
    seat: SeatId,
    dialect: Dialect,
    /// Seat gamepad.
    pub gamepad: GamepadInput,
    focus: SeatFocus,
    window_focused: bool,
    bindings: BindingTable,
    held: HashMap<String, HeldBinding>,
    buttons: HashMap<SourceAction, InputButton>,
    button_order: Vec<SourceAction>,
    mouse: (f64, f64),
    pending_impulse: u8,
    ui: Vec<UiEntry>,
    current_ui: usize,
    next_ui: u64,
}

impl Seat {
    /// Seat with default gamepad tuning.
    #[must_use]
    pub fn new(seat: SeatId, dialect: Dialect, ui_event: UiCallback) -> Self {
        Self {
            seat,
            dialect,
            gamepad: GamepadInput::new(),
            focus: SeatFocus::Game,
            window_focused: true,
            bindings: BindingTable::new(),
            held: HashMap::new(),
            buttons: HashMap::new(),
            button_order: Vec::new(),
            mouse: (0.0, 0.0),
            pending_impulse: 0,
            ui: vec![UiEntry {
                id: 0,
                callback: ui_event,
                focus: SeatFocus::Game,
                active: true,
            }],
            current_ui: 0,
            next_ui: 1,
        }
    }

    /// Seat id.
    #[must_use]
    pub const fn seat(&self) -> &SeatId {
        &self.seat
    }

    /// Command dialect.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Seat focus.
    #[must_use]
    pub const fn focus(&self) -> &SeatFocus {
        &self.focus
    }

    /// Window focus.
    #[must_use]
    pub const fn focused(&self) -> bool {
        self.window_focused
    }

    /// All bindings.
    #[must_use]
    pub fn bindings(&self) -> Vec<&InputBinding> {
        self.bindings.bindings()
    }

    /// Whether an input is held.
    #[must_use]
    pub fn is_down(&self, input: &PhysicalInput) -> bool {
        self.held.contains_key(&super::physical_input_key(input))
    }

    /// Bind an input.
    pub fn bind(&mut self, binding: InputBinding) {
        self.bindings.bind(binding);
    }

    /// Remove a binding.
    pub fn unbind(&mut self, input: &PhysicalInput) {
        self.bindings.unbind(input);
    }

    /// Remove all bindings.
    pub fn unbind_all(&mut self) {
        self.bindings.unbind_all();
    }

    /// Target bound to an input, if any.
    #[must_use]
    pub fn binding(&self, input: &PhysicalInput) -> Option<&InputBindingTarget> {
        self.bindings.binding(input)
    }

    /// Re-target controller bindings at a new device.
    pub fn remap_controller_bindings(&mut self, device: i32) {
        let owned: Vec<InputBinding> = self
            .bindings
            .bindings()
            .iter()
            .filter(|binding| {
                matches!(
                    binding.input,
                    PhysicalInput::ControllerButton { .. } | PhysicalInput::ControllerAxis { .. }
                )
            })
            .map(|binding| (*binding).clone())
            .collect();
        for binding in owned {
            self.bindings.unbind(&binding.input);
            let input = match binding.input {
                PhysicalInput::ControllerButton { button, .. } => PhysicalInput::ControllerButton { device, button },
                PhysicalInput::ControllerAxis { axis, direction, .. } => PhysicalInput::ControllerAxis {
                    device,
                    axis,
                    direction,
                },
                other => other,
            };
            self.bindings.bind(InputBinding {
                input,
                target: binding.target.clone(),
            });
        }
    }

    /// Set focus, releasing everything first.
    pub fn set_focus(&mut self, focus: SeatFocus, now_ms: f64, commands: &mut dyn CommandRegistry) {
        self.release(now_ms, commands);
        self.focus = focus;
    }

    /// Push a UI event handler; returns a token for [`Seat::unbind_ui_event`].
    pub fn bind_ui_event(&mut self, callback: UiCallback, now_ms: f64, commands: &mut dyn CommandRegistry) -> u64 {
        self.ui[self.current_ui].focus = self.focus.clone();
        self.release(now_ms, commands);
        let id = self.next_ui;
        self.next_ui += 1;
        self.ui.push(UiEntry {
            id,
            callback,
            focus: self.focus.clone(),
            active: true,
        });
        self.current_ui = self.ui.len() - 1;
        id
    }

    /// Remove a UI handler, restoring the previous active focus.
    pub fn unbind_ui_event(&mut self, id: u64, now_ms: f64, commands: &mut dyn CommandRegistry) {
        let Some(index) = self.ui.iter().position(|entry| entry.id == id) else {
            return;
        };
        if !self.ui[index].active {
            return;
        }
        self.ui[index].active = false;
        if index != self.current_ui {
            return;
        }
        self.release(now_ms, commands);
        let mut restored = index;
        while restored > 0 && !self.ui[restored].active {
            restored -= 1;
        }
        self.current_ui = restored;
        self.focus = self.ui[restored].focus.clone();
    }

    /// Whether anything is held or any button is active.
    #[must_use]
    pub fn has_held_input(&self) -> bool {
        !self.held.is_empty() || self.buttons.values().any(InputButton::active)
    }

    /// Switch dialects (requires released keys).
    pub fn set_profile(&mut self, dialect: Dialect) -> Result<(), SeatError> {
        if self.dialect == dialect {
            return Ok(());
        }
        if self.has_held_input() {
            return Err(SeatError::ProfileHeld);
        }
        self.dialect = dialect;
        Ok(())
    }

    /// Set the pending impulse byte.
    pub fn set_impulse(&mut self, value: u8) {
        self.pending_impulse = value;
    }

    /// Button state for an action, creating it on demand.
    pub fn button_mut(&mut self, action: SourceAction) -> &mut InputButton {
        if !self.buttons.contains_key(&action) {
            self.button_order.push(action);
        }
        self.buttons.entry(action).or_default()
    }

    /// Press or release a named button from a key.
    pub fn command_button(&mut self, action: SourceAction, key: &str, down: bool, time_ms: i64) {
        let button = self.button_mut(action);
        if down {
            button.down(key, time_ms);
        } else {
            button.up(key, time_ms);
        }
    }

    fn run_binding(
        &mut self,
        input: &PhysicalInput,
        target: &Option<InputBindingTarget>,
        down: bool,
        now_ms: f64,
        commands: &mut dyn CommandRegistry,
    ) {
        let Some(target) = target.clone() else {
            return;
        };
        if let InputBindingTarget::Action(action) = target {
            let key = super::physical_input_key(input);
            self.command_button(SourceAction::Action(action), &key, down, now_ms as i64);
            return;
        }
        let InputBindingTarget::Command(text) = target else {
            return;
        };
        let key = command_key(input);
        let Ok(mut remaining) = source_command_text(&text) else {
            return;
        };
        let mut had_button = false;
        while !remaining.is_empty() {
            let offset = command_separator_offset(&remaining, self.dialect).min(remaining.len());
            let segment = remaining[..offset].trim().to_string();
            remaining = if remaining.len() > offset {
                remaining[offset + 1..].to_string()
            } else {
                String::new()
            };
            if segment.is_empty() {
                continue;
            }
            if let Some(rest) = segment.strip_prefix('+') {
                commands.append(
                    &format!(
                        "{} {key} {}\n",
                        if down { format!("+{rest}") } else { format!("-{rest}") },
                        now_ms.trunc() as i64
                    ),
                    &self.seat,
                );
                had_button = true;
            } else if down || had_button {
                commands.append(&format!("{segment}\n"), &self.seat);
            }
        }
    }

    fn digital(
        &mut self,
        input: PhysicalInput,
        down: bool,
        time_ms: f64,
        consumed: bool,
        commands: &mut dyn CommandRegistry,
    ) {
        let key = super::physical_input_key(&input);
        if down {
            if self.held.contains_key(&key) {
                return;
            }
            let target = if !consumed && self.focus == SeatFocus::Game && self.window_focused {
                self.bindings.binding(&input).cloned()
            } else {
                None
            };
            let held = HeldBinding { input, target };
            self.run_binding(&held.input, &held.target, true, time_ms, commands);
            self.held.insert(key, held);
        } else if let Some(previous) = self.held.remove(&key) {
            self.run_binding(&previous.input, &previous.target, false, time_ms, commands);
        }
    }

    /// Feed one event; returns true when the seat keeps it.
    pub fn input(&mut self, event: &SeatInputEvent, commands: &mut dyn CommandRegistry) -> Result<bool, SeatError> {
        if event.seat() != &self.seat {
            return Err(SeatError::WrongSeat);
        }
        let time = event.time_ms();
        if !time.is_finite() || time < 0.0 {
            return Err(SeatError::BadTimestamp);
        }
        if let SeatInputEvent::Focus { focused, .. } = event {
            self.window_focused = *focused;
            if !focused {
                self.release(time, commands);
            }
            let focus = self.focus.clone();
            let callback = &mut self.ui[self.current_ui].callback;
            return Ok(callback(event, &focus));
        }
        if !self.window_focused {
            return Ok(false);
        }
        let focus = self.focus.clone();
        let callback = &mut self.ui[self.current_ui].callback;
        let consumed = callback(event, &focus);
        match event {
            SeatInputEvent::Key { code, down, .. } => {
                self.digital(PhysicalInput::Key(*code), *down, time, consumed, commands);
            }
            SeatInputEvent::MouseButton { button, down, .. } => {
                self.digital(
                    PhysicalInput::MouseButton(i32::from(*button)),
                    *down,
                    time,
                    consumed,
                    commands,
                );
            }
            SeatInputEvent::ControllerButton {
                device, button, down, ..
            } => {
                self.digital(
                    PhysicalInput::ControllerButton {
                        device: *device,
                        button: i32::from(*button),
                    },
                    *down,
                    time,
                    consumed,
                    commands,
                );
            }
            SeatInputEvent::ControllerAxis {
                device, axis, value, ..
            } => {
                self.gamepad.preview_axis(*axis, *value);
                if !consumed && self.focus == SeatFocus::Game {
                    self.gamepad.axis(*axis, *value);
                }
                for direction in [super::AxisDirection::Negative, super::AxisDirection::Positive] {
                    let deflected = if direction == super::AxisDirection::Positive {
                        *value
                    } else {
                        -*value
                    };
                    self.digital(
                        PhysicalInput::ControllerAxis {
                            device: *device,
                            axis: *axis,
                            direction,
                        },
                        deflected > self.gamepad.tuning.trigger_threshold,
                        time,
                        consumed,
                        commands,
                    );
                }
            }
            SeatInputEvent::MouseMotion { delta, .. } => {
                if !consumed && self.focus == SeatFocus::Game {
                    self.mouse.0 += delta.0;
                    self.mouse.1 += delta.1;
                }
            }
            SeatInputEvent::MouseWheel { delta, .. } => {
                let code = if delta.1 > 0.0 {
                    KeyCode::MouseWheelUp as i32
                } else {
                    KeyCode::MouseWheelDown as i32
                };
                for _ in 0..delta.1.abs().floor().max(0.0) as i64 {
                    self.digital(PhysicalInput::Key(code), true, time, consumed, commands);
                    self.digital(PhysicalInput::Key(code), false, time, consumed, commands);
                }
            }
            SeatInputEvent::Text { .. } | SeatInputEvent::Focus { .. } => {}
        }
        Ok(consumed || self.focus == SeatFocus::Game)
    }

    /// Feed a gyro sample.
    pub fn gyro(&mut self, sample: Vec3, time_ms: f64) -> Result<(), SeatError> {
        if self.window_focused {
            Ok(self.gamepad.gyro(sample, time_ms, self.focus == SeatFocus::Game)?)
        } else {
            Ok(())
        }
    }

    /// Release one controller device.
    pub fn release_device(&mut self, device: i32, time_ms: f64, commands: &mut dyn CommandRegistry) {
        let owned: Vec<(String, HeldBinding)> = self
            .held
            .iter()
            .filter(|(_, held)| match &held.input {
                PhysicalInput::ControllerButton { device: owned, .. }
                | PhysicalInput::ControllerAxis { device: owned, .. } => *owned == device,
                _ => false,
            })
            .map(|(key, held)| (key.clone(), held.clone()))
            .collect();
        for (key, held) in owned {
            self.run_binding(&held.input, &held.target, false, time_ms, commands);
            self.held.remove(&key);
        }
        self.gamepad.clear();
        self.gamepad.reset_gyro_calibration();
    }

    /// Release everything.
    pub fn release(&mut self, time_ms: f64, commands: &mut dyn CommandRegistry) {
        let owned: Vec<HeldBinding> = self.held.values().cloned().collect();
        for held in owned {
            self.run_binding(&held.input, &held.target, false, time_ms, commands);
        }
        self.held.clear();
        for button in self.buttons.values_mut() {
            button.release(time_ms as i64);
        }
        self.gamepad.clear();
        self.mouse = (0.0, 0.0);
        self.pending_impulse = 0;
    }

    /// Sample one frame, draining mouse and impulse.
    pub fn sample(&mut self, now_ms: f64, frame_ms: f64) -> Result<SeatFrame, SeatError> {
        if !now_ms.is_finite() || !frame_ms.is_finite() || frame_ms <= 0.0 {
            return Err(SeatError::BadSample);
        }
        let timing = if self.dialect.is_q1() {
            ButtonTiming::Q1
        } else if self.dialect == Dialect::Q3 {
            ButtonTiming::Q3
        } else {
            ButtonTiming::Q2
        };
        let mut buttons = Vec::new();
        for action in self.button_order.clone() {
            let button = self.buttons.get_mut(&action).expect("ordered button exists");
            let pressed = button.pressed();
            let active = button.active();
            let fraction = button.sample(timing, now_ms as i64, frame_ms);
            buttons.push(ButtonSample {
                action,
                fraction,
                active,
                pressed,
            });
        }
        let gamepad = self.gamepad.sample(frame_ms);
        let frame = SeatFrame {
            seat: self.seat.clone(),
            now_ms,
            frame_ms,
            focus: self.focus.clone(),
            buttons,
            mouse: vec2(self.mouse.0 as f32, self.mouse.1 as f32),
            gamepad_move: gamepad.stick_move,
            gamepad_look_degrees: gamepad.look_degrees,
            impulse: i32::from(self.pending_impulse),
            any_key_down: self.held.len(),
        };
        self.mouse = (0.0, 0.0);
        self.pending_impulse = 0;
        Ok(frame)
    }
}

impl ButtonSeat for Seat {
    fn command_button(&mut self, action: SourceAction, key: &str, down: bool, time_ms: i64) {
        Seat::command_button(self, action, key, down, time_ms);
    }

    fn release_button(&mut self, action: SourceAction, time_ms: i64) {
        self.button_mut(action).release(time_ms);
    }

    fn set_impulse_byte(&mut self, value: u8) {
        self.pending_impulse = value;
    }
}

/// Window event surface borrowed by the router.
pub trait RouterWindow {
    /// Logical window size.
    fn logical_size(&mut self) -> Result<(i32, i32), RouterError>;
    /// Drawable size in pixels.
    fn drawable_size(&mut self) -> Result<(i32, i32), RouterError>;
    /// Pump platform events.
    fn poll_events(&mut self) -> Result<Vec<SdlEvent>, RouterError>;
    /// Capture relative mouse motion.
    fn set_relative_mouse(&mut self, enabled: bool) -> Result<(), RouterError>;
}

/// Controller backend surface borrowed by the router.
pub trait RouterControllers {
    /// Route assignment slots.
    fn set_assignments(&mut self, selections: &[ControllerSelection]) -> Result<(), RouterError>;
    /// Enable a controller sensor.
    fn set_sensor_enabled(
        &mut self,
        instance: i32,
        sensor: ControllerSensor,
        enabled: bool,
    ) -> Result<ControllerOperationResult, RouterError>;
    /// Pump controller events.
    fn poll_events(&mut self) -> Result<Vec<ControllerEvent>, RouterError>;
    /// Snapshot live axes and buttons.
    fn snapshot(&mut self, instance: i32) -> Result<Option<ControllerState>, RouterError>;
    /// Current slot assignments.
    fn assignments(&mut self) -> Result<Vec<Option<i32>>, RouterError>;
}

fn platform(error: qa_platform::error::Error) -> RouterError {
    RouterError::Platform(error.to_string())
}

/// [`RouterWindow`] over a live SDL window and its input lease.
pub struct SdlRouterWindow {
    window: SdlWindow,
    lease: Option<SdlInputLease>,
}

impl SdlRouterWindow {
    /// Attach to an open window, acquiring the input lease.
    pub fn attach(mut window: SdlWindow) -> Result<Self, RouterError> {
        let lease = window.begin_input().map_err(platform)?;
        Ok(Self {
            window,
            lease: Some(lease),
        })
    }
}

impl RouterWindow for SdlRouterWindow {
    fn logical_size(&mut self) -> Result<(i32, i32), RouterError> {
        self.window.logical_size().map_err(platform)
    }

    fn drawable_size(&mut self) -> Result<(i32, i32), RouterError> {
        self.window.drawable_size_signed().map_err(platform)
    }

    fn poll_events(&mut self) -> Result<Vec<SdlEvent>, RouterError> {
        self.window.poll_events().map_err(platform)
    }

    fn set_relative_mouse(&mut self, enabled: bool) -> Result<(), RouterError> {
        if let Some(lease) = &self.lease {
            lease.set_relative_mouse(enabled).map_err(platform)?;
        }
        Ok(())
    }
}

impl Drop for SdlRouterWindow {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            let _ = lease.close();
        }
    }
}

/// [`RouterControllers`] over live SDL controllers.
pub struct SdlRouterControllers {
    controllers: SdlControllers,
}

impl SdlRouterControllers {
    /// Wrap live controllers.
    #[must_use]
    pub const fn new(controllers: SdlControllers) -> Self {
        Self { controllers }
    }
}

impl RouterControllers for SdlRouterControllers {
    fn set_assignments(&mut self, selections: &[ControllerSelection]) -> Result<(), RouterError> {
        self.controllers.set_assignments(selections).map_err(platform)
    }

    fn set_sensor_enabled(
        &mut self,
        instance: i32,
        sensor: ControllerSensor,
        enabled: bool,
    ) -> Result<ControllerOperationResult, RouterError> {
        self.controllers
            .set_sensor_enabled(instance, sensor, enabled)
            .map_err(platform)
    }

    fn poll_events(&mut self) -> Result<Vec<ControllerEvent>, RouterError> {
        self.controllers.poll_events().map_err(platform)
    }

    fn snapshot(&mut self, instance: i32) -> Result<Option<ControllerState>, RouterError> {
        self.controllers.snapshot(instance).map_err(platform)
    }

    fn assignments(&mut self) -> Result<Vec<Option<i32>>, RouterError> {
        self.controllers.assignments().map_err(platform)
    }
}

/// Event outside every seat route.
#[derive(Debug, Clone, PartialEq)]
pub enum UnhandledEvent {
    /// Platform event.
    Platform(SdlEvent),
    /// Controller event.
    Controller(ControllerEvent),
}

/// Controller-operation observer.
pub type ControllerOperationFn = Box<dyn FnMut(&SeatId, &ControllerOperationResult)>;

/// One seat route: seat plus controller selection.
pub struct SeatRoute {
    /// Seat.
    pub seat: Seat,
    /// Controller selection.
    pub controller: ControllerSelection,
}

/// Platform event router: keyboard, pointer, and controller dispatch.
pub struct InputRouter {
    routes: Vec<SeatRoute>,
    controller_seats: Vec<Option<usize>>,
    keyboard: Option<usize>,
    source_joystick: Option<i32>,
    source_seat: Option<SeatId>,
    keyboard_keys: HashMap<i32, i32>,
    device_seats: HashMap<i32, usize>,
    calibration_sensors: HashSet<i32>,
    window: Option<Box<dyn RouterWindow>>,
    controllers: Option<Box<dyn RouterControllers>>,
    commands: Box<dyn CommandRegistry>,
    now: Box<dyn Fn() -> f64>,
    ticks: Box<dyn Fn() -> u32>,
    subframe: bool,
    unhandled: Box<dyn FnMut(UnhandledEvent)>,
    controller_operation: Option<ControllerOperationFn>,
    platform_active: bool,
    closed: bool,
}

/// SDL focus-gained window event.
pub const WINDOW_FOCUS_GAINED: u8 = 12;
/// SDL focus-lost window event.
pub const WINDOW_FOCUS_LOST: u8 = 13;

impl InputRouter {
    /// Router over published seats.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        routes: Vec<SeatRoute>,
        keyboard: Option<SeatId>,
        controllers: Option<Box<dyn RouterControllers>>,
        commands: Box<dyn CommandRegistry>,
        now: Box<dyn Fn() -> f64>,
        ticks: Box<dyn Fn() -> u32>,
        subframe: bool,
        unhandled: Box<dyn FnMut(UnhandledEvent)>,
        defer_platform: bool,
        controller_operation: Option<ControllerOperationFn>,
    ) -> Result<Self, RouterError> {
        for (index, route) in routes.iter().enumerate() {
            if routes[..index]
                .iter()
                .any(|previous| previous.seat.seat() == route.seat.seat())
            {
                return Err(RouterError::DuplicateSeat);
            }
        }
        let platform_active = !defer_platform;
        let mut router = Self {
            controller_seats: (0..routes.len()).map(Some).collect(),
            routes,
            keyboard: None,
            source_joystick: None,
            source_seat: None,
            keyboard_keys: HashMap::new(),
            device_seats: HashMap::new(),
            calibration_sensors: HashSet::new(),
            window: None,
            controllers,
            commands,
            now,
            ticks,
            subframe,
            unhandled,
            controller_operation,
            platform_active,
            closed: false,
        };
        router.set_keyboard_seat(keyboard)?;
        if platform_active {
            let selections: Vec<ControllerSelection> =
                router.routes.iter().map(|route| route.controller.clone()).collect();
            if let Some(controllers) = router.controllers.as_mut() {
                controllers.set_assignments(&selections)?;
            }
        }
        Ok(router)
    }

    /// Routed seats.
    pub fn seats(&self) -> Vec<&Seat> {
        self.routes.iter().map(|route| &route.seat).collect()
    }

    /// Seat by id.
    pub fn seat(&self, id: &SeatId) -> Option<&Seat> {
        self.routes
            .iter()
            .find(|route| route.seat.seat() == id)
            .map(|route| &route.seat)
    }

    fn seat_index(&self, id: &SeatId) -> Option<usize> {
        self.routes.iter().position(|route| route.seat.seat() == id)
    }

    /// Route the legacy source joystick at a seat.
    pub fn set_source_joystick(&mut self, instance: Option<i32>, seat: Option<SeatId>) -> Result<(), RouterError> {
        if let Some(seat) = &seat {
            if self.seat_index(seat).is_none() {
                return Err(RouterError::BadSourceSeat);
            }
        }
        if instance == self.source_joystick && seat == self.source_seat {
            return Ok(());
        }
        let now = (self.now)();
        for selected in [self.source_joystick, instance].into_iter().flatten() {
            if let Some(index) = self.device_seats.get(&selected).copied() {
                let (routes, commands) = (&mut self.routes, &mut *self.commands);
                routes[index].seat.release_device(selected, now, commands);
            }
        }
        self.source_joystick = instance;
        self.source_seat = if instance.is_none() { None } else { seat };
        Ok(())
    }

    /// Publish a new seat set.
    pub fn publish_seats(&mut self, routes: Vec<SeatRoute>, keyboard: Option<SeatId>) -> Result<(), RouterError> {
        for (index, route) in routes.iter().enumerate() {
            if routes[..index]
                .iter()
                .any(|previous| previous.seat.seat() == route.seat.seat())
            {
                return Err(RouterError::DuplicateSeat);
            }
        }
        if let Some(keyboard) = &keyboard {
            if !routes.iter().any(|route| route.seat.seat() == keyboard) {
                return Err(RouterError::KeyboardNotPublished);
            }
        }
        let now = (self.now)();
        {
            let (routes, commands) = (&mut self.routes, &mut *self.commands);
            for route in routes {
                route.seat.release(now, commands);
            }
        }
        self.routes = routes;
        self.source_joystick = None;
        self.source_seat = None;
        self.set_keyboard_seat(keyboard)?;
        self.restart()?;
        self.update_capture()
    }

    /// Retain a subset of seats without rebinding their devices.
    pub fn retain_seats(&mut self, seats: &[SeatId], keyboard: Option<SeatId>) -> Result<(), RouterError> {
        for (index, seat) in seats.iter().enumerate() {
            if seats[..index].contains(seat) || self.seat_index(seat).is_none() {
                return Err(RouterError::BadRetainedSeats);
            }
        }
        if let Some(keyboard) = &keyboard {
            if !seats.contains(keyboard) {
                return Err(RouterError::KeyboardNotRetained);
            }
        }
        let now = (self.now)();
        for index in 0..self.routes.len() {
            if seats.contains(self.routes[index].seat.seat()) {
                continue;
            }
            let (routes, commands) = (&mut self.routes, &mut *self.commands);
            routes[index].seat.release(now, commands);
            routes[index].seat.gamepad.cancel_gyro_calibration();
        }
        self.finish_gyro_calibration()?;
        let mut kept = Vec::new();
        let mut remap: HashMap<usize, usize> = HashMap::new();
        for (index, route) in std::mem::take(&mut self.routes).into_iter().enumerate() {
            if seats.contains(route.seat.seat()) {
                remap.insert(index, kept.len());
                kept.push(route);
            }
        }
        self.routes = kept;
        self.controller_seats = self
            .controller_seats
            .iter()
            .map(|slot| slot.and_then(|index| remap.get(&index).copied()))
            .collect();
        let dropped: Vec<i32> = self
            .device_seats
            .iter()
            .filter(|(_, index)| !remap.contains_key(*index))
            .map(|(instance, _)| *instance)
            .collect();
        for instance in dropped {
            self.device_seats.remove(&instance);
            if self.source_joystick == Some(instance) {
                self.source_joystick = None;
                self.source_seat = None;
            }
        }
        for index in self.device_seats.values_mut() {
            *index = remap[&*index];
        }
        if let Some(seat) = &self.source_seat {
            if !seats.contains(seat) {
                self.source_joystick = None;
                self.source_seat = None;
            }
        }
        if let Some(keyboard) = self.keyboard {
            if !remap.contains_key(&keyboard) {
                self.keyboard = None;
                self.keyboard_keys.clear();
            } else {
                self.keyboard = remap.get(&keyboard).copied();
            }
        }
        let current = self
            .keyboard
            .and_then(|index| self.routes.get(index))
            .map(|route| route.seat.seat().clone());
        if current.as_ref() != keyboard.as_ref() {
            self.set_keyboard_seat(keyboard)?;
        }
        self.update_capture()
    }

    /// Keyboard seat id.
    #[must_use]
    pub fn keyboard_seat(&self) -> Option<SeatId> {
        self.keyboard
            .and_then(|index| self.routes.get(index))
            .map(|route| route.seat.seat().clone())
    }

    /// Controller selection for a seat.
    pub fn controller_selection(&self, id: &SeatId) -> Result<ControllerSelection, RouterError> {
        self.routes
            .iter()
            .find(|route| route.seat.seat() == id)
            .map(|route| route.controller.clone())
            .ok_or(RouterError::UnknownSeat)
    }

    /// Replace a seat's controller selection.
    pub fn set_controller_selection(&mut self, id: &SeatId, selection: ControllerSelection) -> Result<(), RouterError> {
        let Some(index) = self.seat_index(id) else {
            return Err(RouterError::UnknownSeat);
        };
        self.routes[index].controller = selection;
        self.restart()
    }

    /// Controller instance routed to a seat.
    #[must_use]
    pub fn controller_for(&self, id: &SeatId) -> Option<i32> {
        if self.source_joystick.is_some() && self.source_seat.as_ref() == Some(id) {
            return self.source_joystick;
        }
        for (instance, index) in &self.device_seats {
            if Some(*instance) != self.source_joystick
                && self.routes.get(*index).is_some_and(|route| route.seat.seat() == id)
            {
                return Some(*instance);
            }
        }
        None
    }

    /// Enable or disable a seat's gyro sensor.
    pub fn set_gyro_enabled(&mut self, id: &SeatId, enabled: bool) -> Result<ControllerOperationResult, RouterError> {
        let Some(index) = self.seat_index(id) else {
            return Err(RouterError::BadGyroSeat);
        };
        if !self.platform_active || self.controllers.is_none() {
            return Ok(ControllerOperationResult::Disconnected {
                reason: "Seat has no assigned controller".to_string(),
            });
        }
        let Some(instance) = self.controller_for(id) else {
            return Ok(ControllerOperationResult::Disconnected {
                reason: "Seat has no assigned controller".to_string(),
            });
        };
        let result = self
            .controllers
            .as_mut()
            .ok_or(RouterError::Closed)?
            .set_sensor_enabled(instance, ControllerSensor::Gyro, enabled)?;
        if result == ControllerOperationResult::Accepted {
            self.routes[index].seat.gamepad.cancel_gyro_calibration();
            self.calibration_sensors.remove(&instance);
            self.routes[index].seat.gamepad.tuning.gyro.enabled = enabled;
        }
        Ok(result)
    }

    /// Calibration state for a seat.
    pub fn gyro_calibration(&self, id: &SeatId) -> Result<super::gamepad::GyroCalibrationState, RouterError> {
        self.seat(id)
            .map(|seat| seat.gamepad.gyro_calibration())
            .ok_or(RouterError::BadGyroSeat)
    }

    /// Begin gyro calibration for a seat.
    pub fn begin_gyro_calibration(&mut self, id: &SeatId) -> Result<ControllerOperationResult, RouterError> {
        let Some(index) = self.seat_index(id) else {
            return Err(RouterError::BadGyroSeat);
        };
        if !self.platform_active || self.controllers.is_none() {
            return Ok(ControllerOperationResult::Disconnected {
                reason: "Seat has no assigned controller".to_string(),
            });
        }
        let Some(instance) = self.controller_for(id) else {
            return Ok(ControllerOperationResult::Disconnected {
                reason: "Seat has no assigned controller".to_string(),
            });
        };
        if !self.routes[index].seat.focused() {
            return Ok(ControllerOperationResult::Failed {
                reason: "Focus the game window before calibrating".to_string(),
            });
        }
        let result = self
            .controllers
            .as_mut()
            .ok_or(RouterError::Closed)?
            .set_sensor_enabled(instance, ControllerSensor::Gyro, true)?;
        if result == ControllerOperationResult::Accepted {
            if !self.routes[index].seat.gamepad.tuning.gyro.enabled {
                self.calibration_sensors.insert(instance);
            }
            self.routes[index].seat.gamepad.begin_gyro_calibration();
        }
        Ok(result)
    }

    /// Cancel gyro calibration for a seat.
    pub fn cancel_gyro_calibration(&mut self, id: &SeatId) -> Result<(), RouterError> {
        let Some(index) = self.seat_index(id) else {
            return Err(RouterError::BadGyroSeat);
        };
        self.routes[index].seat.gamepad.cancel_gyro_calibration();
        self.finish_gyro_calibration()
    }

    /// Reset gyro calibration for a seat.
    pub fn reset_gyro_calibration(&mut self, id: &SeatId) -> Result<(), RouterError> {
        let Some(index) = self.seat_index(id) else {
            return Err(RouterError::BadGyroSeat);
        };
        self.routes[index].seat.gamepad.reset_gyro_calibration();
        self.finish_gyro_calibration()
    }

    fn finish_gyro_calibration(&mut self) -> Result<(), RouterError> {
        if !self.platform_active {
            return Ok(());
        }
        let sensors: Vec<i32> = self.calibration_sensors.iter().copied().collect();
        for instance in sensors {
            let index = self.device_seats.get(&instance).copied();
            if index.is_some_and(|index| {
                matches!(
                    self.routes[index].seat.gamepad.gyro_calibration(),
                    super::gamepad::GyroCalibrationState::Calibrating { .. }
                )
            }) {
                continue;
            }
            self.calibration_sensors.remove(&instance);
            let Some(index) = index else {
                continue;
            };
            if self.routes[index].seat.gamepad.tuning.gyro.enabled {
                continue;
            }
            if let Some(controllers) = self.controllers.as_mut() {
                let result = controllers.set_sensor_enabled(instance, ControllerSensor::Gyro, false)?;
                if let Some(operation) = self.controller_operation.as_mut() {
                    let seat = self.routes[index].seat.seat().clone();
                    operation(&seat, &result);
                }
            }
        }
        Ok(())
    }

    /// Route the keyboard at a seat.
    pub fn set_keyboard_seat(&mut self, id: Option<SeatId>) -> Result<(), RouterError> {
        if self.platform_active {
            if let Some(index) = self.keyboard {
                let now = (self.now)();
                let (routes, commands) = (&mut self.routes, &mut *self.commands);
                routes[index].seat.release(now, commands);
            }
        }
        self.keyboard_keys.clear();
        match id {
            None => self.keyboard = None,
            Some(id) => {
                let Some(index) = self.seat_index(&id) else {
                    return Err(RouterError::BadKeyboardSeat);
                };
                self.keyboard = Some(index);
            }
        }
        Ok(())
    }

    /// Attach the platform window.
    pub fn attach_window(&mut self, window: Box<dyn RouterWindow>) -> Result<(), RouterError> {
        self.detach_window()?;
        if !self.platform_active {
            self.platform_active = true;
            self.restart()?;
        }
        self.window = Some(window);
        self.update_capture()
    }

    /// Move window ownership to another router.
    pub fn transfer_window_to(&mut self, next: &mut Self) {
        next.platform_active = self.platform_active;
        self.platform_active = false;
        for instance in std::mem::take(&mut self.calibration_sensors) {
            next.calibration_sensors.insert(instance);
        }
        next.window = self.window.take();
    }

    /// Detach the platform window.
    pub fn detach_window(&mut self) -> Result<(), RouterError> {
        if self.platform_active {
            let now = (self.now)();
            let (routes, commands) = (&mut self.routes, &mut *self.commands);
            for route in routes {
                route.seat.release(now, commands);
            }
        }
        self.finish_gyro_calibration()?;
        self.keyboard_keys.clear();
        self.window = None;
        Ok(())
    }

    /// Sync relative-mouse capture to keyboard focus.
    pub fn update_capture(&mut self) -> Result<(), RouterError> {
        self.finish_gyro_calibration()?;
        let playing = self.keyboard.is_some_and(|index| {
            self.routes[index].seat.focused() && *self.routes[index].seat.focus() == SeatFocus::Game
        });
        if let Some(window) = self.window.as_mut() {
            window.set_relative_mouse(playing)?;
        }
        Ok(())
    }

    fn pointer_position(&mut self, x: i32, y: i32) -> Result<(f64, f64), RouterError> {
        let Some(window) = self.window.as_mut() else {
            return Ok((f64::from(x), f64::from(y)));
        };
        let logical = window.logical_size()?;
        let drawable = window.drawable_size()?;
        Ok((
            f64::from(x) * f64::from(drawable.0) / f64::from(logical.0),
            f64::from(y) * f64::from(drawable.1) / f64::from(logical.1),
        ))
    }

    fn time(&self, timestamp: u32) -> i64 {
        sdl_event_time(timestamp, (self.ticks)(), (self.now)() as i64, self.subframe)
    }

    fn check_open(&self) -> Result<(), RouterError> {
        if self.closed {
            return Err(RouterError::Closed);
        }
        Ok(())
    }

    /// Feed one platform event.
    pub fn handle_platform(&mut self, event: SdlEvent) -> Result<(), RouterError> {
        self.check_open()?;
        let time_ms = self.time(event.timestamp());
        if let SdlEvent::Window { event: kind, .. } = &event {
            if *kind == WINDOW_FOCUS_GAINED || *kind == WINDOW_FOCUS_LOST {
                let focused = *kind == WINDOW_FOCUS_GAINED;
                for index in 0..self.routes.len() {
                    let seat = self.routes[index].seat.seat().clone();
                    let (routes, commands) = (&mut self.routes, &mut *self.commands);
                    routes[index].seat.input(
                        &SeatInputEvent::Focus {
                            seat,
                            time_ms: time_ms as f64,
                            focused,
                        },
                        commands,
                    )?;
                }
                if !focused {
                    self.keyboard_keys.clear();
                }
                return self.update_capture();
            }
        }
        let Some(keyboard) = self.keyboard else {
            (self.unhandled)(UnhandledEvent::Platform(event));
            return Ok(());
        };
        let seat_id = self.routes[keyboard].seat.seat().clone();
        let translated = match event {
            SdlEvent::Key {
                down,
                scancode,
                keycode,
                modifiers,
                ..
            } => {
                let code = self
                    .keyboard_keys
                    .get(&scancode)
                    .copied()
                    .unwrap_or_else(|| sdl_game_key(keycode, modifiers));
                if code == 0 {
                    return Ok(());
                }
                if down {
                    self.keyboard_keys.insert(scancode, code);
                } else {
                    self.keyboard_keys.remove(&scancode);
                }
                SeatInputEvent::Key {
                    seat: seat_id,
                    time_ms: time_ms as f64,
                    code,
                    down,
                }
            }
            SdlEvent::Text { text, .. } => SeatInputEvent::Text {
                seat: seat_id,
                time_ms: time_ms as f64,
                text,
            },
            SdlEvent::MouseMotion { x, y, dx, dy, .. } => {
                let position = self.pointer_position(x, y)?;
                SeatInputEvent::MouseMotion {
                    seat: seat_id,
                    time_ms: time_ms as f64,
                    position,
                    delta: (f64::from(dx), f64::from(dy)),
                }
            }
            SdlEvent::MouseButton { button, down, x, y, .. } => {
                if *self.routes[keyboard].seat.focus() != SeatFocus::Game {
                    let position = self.pointer_position(x, y)?;
                    let (routes, commands) = (&mut self.routes, &mut *self.commands);
                    routes[keyboard].seat.input(
                        &SeatInputEvent::MouseMotion {
                            seat: seat_id.clone(),
                            time_ms: time_ms as f64,
                            position,
                            delta: (0.0, 0.0),
                        },
                        commands,
                    )?;
                }
                SeatInputEvent::MouseButton {
                    seat: seat_id,
                    time_ms: time_ms as f64,
                    button,
                    down,
                }
            }
            SdlEvent::MouseWheel {
                precise_x,
                precise_y,
                flipped,
                ..
            } => {
                let sign = if flipped { -1.0 } else { 1.0 };
                SeatInputEvent::MouseWheel {
                    seat: seat_id,
                    time_ms: time_ms as f64,
                    delta: (f64::from(precise_x) * sign, f64::from(precise_y) * sign),
                }
            }
            other => {
                (self.unhandled)(UnhandledEvent::Platform(other));
                return Ok(());
            }
        };
        {
            let (routes, commands) = (&mut self.routes, &mut *self.commands);
            routes[keyboard].seat.input(&translated, commands)?;
        }
        self.update_capture()
    }

    /// Feed one controller event.
    pub fn handle_controller(&mut self, event: ControllerEvent) -> Result<(), RouterError> {
        self.check_open()?;
        let time_ms = self.time(event.timestamp());
        if let ControllerEvent::Assignment {
            slot,
            previous,
            instance,
            ..
        } = &event
        {
            if let Some(previous) = previous {
                if let Some(index) = self.device_seats.get(previous).copied() {
                    let (routes, commands) = (&mut self.routes, &mut *self.commands);
                    routes[index].seat.release_device(*previous, time_ms as f64, commands);
                }
                self.finish_gyro_calibration()?;
                self.device_seats.remove(previous);
            }
            let seat = self.controller_seats.get(*slot).copied().flatten();
            if let (Some(instance), Some(index)) = (instance, seat) {
                self.routes[index].seat.gamepad.reset_gyro_calibration();
                self.routes[index].seat.remap_controller_bindings(*instance);
                self.device_seats.insert(*instance, index);
                if self.routes[index].seat.gamepad.tuning.gyro.enabled {
                    let seat_id = self.routes[index].seat.seat().clone();
                    let result = self.set_gyro_enabled(&seat_id, true)?;
                    if let Some(operation) = self.controller_operation.as_mut() {
                        operation(&seat_id, &result);
                    }
                }
            }
            (self.unhandled)(UnhandledEvent::Controller(event.clone()));
            return Ok(());
        }
        let instance = match &event {
            ControllerEvent::Disconnected { instance, .. }
            | ControllerEvent::Axis { instance, .. }
            | ControllerEvent::Button { instance, .. }
            | ControllerEvent::Touchpad { instance, .. }
            | ControllerEvent::Sensor { instance, .. }
            | ControllerEvent::Unrecognized { instance, .. } => Some(*instance),
            ControllerEvent::Connected { .. }
            | ControllerEvent::Remapped { .. }
            | ControllerEvent::Assignment { .. } => None,
        };
        let Some(instance) = instance else {
            (self.unhandled)(UnhandledEvent::Controller(event));
            return Ok(());
        };
        let Some(index) = self.device_seats.get(&instance).copied() else {
            (self.unhandled)(UnhandledEvent::Controller(event));
            return Ok(());
        };
        if Some(instance) == self.source_joystick && !matches!(event, ControllerEvent::Disconnected { .. }) {
            return Ok(());
        }
        let seat_id = self.routes[index].seat.seat().clone();
        match event {
            ControllerEvent::Disconnected { .. } => {
                let (routes, commands) = (&mut self.routes, &mut *self.commands);
                routes[index].seat.release_device(instance, time_ms as f64, commands);
                self.calibration_sensors.remove(&instance);
                self.device_seats.remove(&instance);
            }
            ControllerEvent::Button { button, down, .. } => {
                let (routes, commands) = (&mut self.routes, &mut *self.commands);
                routes[index].seat.input(
                    &SeatInputEvent::ControllerButton {
                        seat: seat_id,
                        time_ms: time_ms as f64,
                        device: instance,
                        button,
                        down,
                    },
                    commands,
                )?;
            }
            ControllerEvent::Axis { axis, value, .. } => {
                if let Some(name) = controller_axis_name(axis) {
                    let (routes, commands) = (&mut self.routes, &mut *self.commands);
                    routes[index].seat.input(
                        &SeatInputEvent::ControllerAxis {
                            seat: seat_id,
                            time_ms: time_ms as f64,
                            device: instance,
                            axis: name,
                            value: normalized_controller_axis(name, value),
                        },
                        commands,
                    )?;
                }
            }
            ControllerEvent::Sensor {
                sensor,
                x,
                y,
                z,
                timestamp,
                timestamp_us,
                slot,
                ..
            } => {
                if sensor == ControllerSensor::Gyro {
                    let sample_time = if timestamp_us == 0 {
                        f64::from(timestamp)
                    } else {
                        timestamp_us as f64 / 1000.0
                    };
                    self.routes[index]
                        .seat
                        .gyro(qa_core::math::vec3(x, y, z), sample_time)?;
                    self.finish_gyro_calibration()?;
                } else {
                    (self.unhandled)(UnhandledEvent::Controller(ControllerEvent::Sensor {
                        timestamp,
                        instance,
                        slot,
                        sensor,
                        x,
                        y,
                        z,
                        timestamp_us,
                    }));
                }
            }
            other => {
                (self.unhandled)(UnhandledEvent::Controller(other));
            }
        }
        Ok(())
    }

    /// Pump window and controller events, then refresh snapshots.
    pub fn pump(&mut self) -> Result<(), RouterError> {
        let window_events = self
            .window
            .as_mut()
            .map(|window| window.poll_events())
            .transpose()?
            .unwrap_or_default();
        for event in window_events {
            self.handle_platform(event)?;
        }
        let controller_events = self
            .controllers
            .as_mut()
            .map(|controllers| controllers.poll_events())
            .transpose()?
            .unwrap_or_default();
        for event in controller_events {
            self.handle_controller(event)?;
        }
        let devices: Vec<(i32, usize)> = self
            .device_seats
            .iter()
            .map(|(instance, index)| (*instance, *index))
            .collect();
        for (instance, index) in devices {
            if Some(instance) == self.source_joystick {
                continue;
            }
            if !self.routes[index].seat.focused() || *self.routes[index].seat.focus() != SeatFocus::Game {
                continue;
            }
            let snapshot = self
                .controllers
                .as_mut()
                .map(|controllers| controllers.snapshot(instance))
                .transpose()?
                .flatten();
            if let Some(snapshot) = snapshot {
                for (number, value) in snapshot.axes.iter().enumerate() {
                    if let Some(axis) = controller_axis_name(number as u8) {
                        self.routes[index]
                            .seat
                            .gamepad
                            .axis(axis, normalized_controller_axis(axis, *value));
                    }
                }
            }
        }
        self.update_capture()
    }

    /// Re-release everything and re-resolve assignments.
    pub fn restart(&mut self) -> Result<(), RouterError> {
        if self.platform_active {
            let now = (self.now)();
            let (routes, commands) = (&mut self.routes, &mut *self.commands);
            for route in routes {
                route.seat.release(now, commands);
                route.seat.gamepad.reset_gyro_calibration();
            }
        }
        self.finish_gyro_calibration()?;
        self.device_seats.clear();
        self.keyboard_keys.clear();
        self.controller_seats = (0..self.routes.len()).map(Some).collect();
        if self.platform_active {
            let selections: Vec<ControllerSelection> =
                self.routes.iter().map(|route| route.controller.clone()).collect();
            if let Some(controllers) = self.controllers.as_mut() {
                controllers.set_assignments(&selections)?;
            }
        }
        let assignments = self
            .controllers
            .as_mut()
            .map(|controllers| controllers.assignments())
            .transpose()?
            .unwrap_or_default();
        for (slot, instance) in assignments.iter().enumerate() {
            if let (Some(instance), Some(route)) = (instance, self.routes.get_mut(slot)) {
                if self.platform_active {
                    route.seat.remap_controller_bindings(*instance);
                }
                self.device_seats.insert(*instance, slot);
            }
        }
        self.update_capture()
    }

    /// Close the router.
    pub fn close(&mut self) -> Result<(), RouterError> {
        if self.closed {
            return Ok(());
        }
        self.detach_window()?;
        if self.platform_active {
            for route in &mut self.routes {
                route.seat.gamepad.reset_gyro_calibration();
            }
        }
        self.device_seats.clear();
        self.closed = true;
        Ok(())
    }
}

trait EventTimestamp {
    fn timestamp(&self) -> u32;
}

impl EventTimestamp for SdlEvent {
    fn timestamp(&self) -> u32 {
        match self {
            SdlEvent::Quit { timestamp }
            | SdlEvent::Key { timestamp, .. }
            | SdlEvent::Text { timestamp, .. }
            | SdlEvent::MouseMotion { timestamp, .. }
            | SdlEvent::MouseButton { timestamp, .. }
            | SdlEvent::MouseWheel { timestamp, .. }
            | SdlEvent::Window { timestamp, .. }
            | SdlEvent::JoystickAxis { timestamp, .. }
            | SdlEvent::JoystickHat { timestamp, .. }
            | SdlEvent::JoystickButton { timestamp, .. }
            | SdlEvent::JoystickRemoved { timestamp, .. }
            | SdlEvent::Unsupported { timestamp, .. } => *timestamp,
        }
    }
}

impl EventTimestamp for ControllerEvent {
    fn timestamp(&self) -> u32 {
        match self {
            ControllerEvent::Connected { timestamp, .. }
            | ControllerEvent::Remapped { timestamp, .. }
            | ControllerEvent::Assignment { timestamp, .. }
            | ControllerEvent::Disconnected { timestamp, .. }
            | ControllerEvent::Axis { timestamp, .. }
            | ControllerEvent::Button { timestamp, .. }
            | ControllerEvent::Touchpad { timestamp, .. }
            | ControllerEvent::Sensor { timestamp, .. }
            | ControllerEvent::Unrecognized { timestamp, .. } => *timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::commands::CommandHandler;
    use crate::input::InputAction;
    use qa_core::identity::IdentityOwner;

    struct FakeRegistry {
        appended: Vec<(String, SeatId)>,
    }

    impl CommandRegistry for FakeRegistry {
        fn register_engine(&mut self, _name: &str, _handler: CommandHandler) -> bool {
            true
        }

        fn register(&mut self, _name: &str, _handler: CommandHandler) -> bool {
            true
        }

        fn unregister(&mut self, _name: &str) {}

        fn exists(&self, _name: &str) -> bool {
            false
        }

        fn append(&mut self, text: &str, seat: &SeatId) {
            self.appended.push((text.to_string(), seat.clone()));
        }
    }

    struct FakeWindow {
        events: Vec<SdlEvent>,
        relative: bool,
    }

    impl RouterWindow for FakeWindow {
        fn logical_size(&mut self) -> Result<(i32, i32), RouterError> {
            Ok((640, 480))
        }

        fn drawable_size(&mut self) -> Result<(i32, i32), RouterError> {
            Ok((1280, 960))
        }

        fn poll_events(&mut self) -> Result<Vec<SdlEvent>, RouterError> {
            Ok(std::mem::take(&mut self.events))
        }

        fn set_relative_mouse(&mut self, enabled: bool) -> Result<(), RouterError> {
            self.relative = enabled;
            Ok(())
        }
    }

    struct FakeControllers {
        events: Vec<ControllerEvent>,
        slots: Vec<Option<i32>>,
        sensors: Vec<(i32, bool)>,
    }

    impl RouterControllers for FakeControllers {
        fn set_assignments(&mut self, selections: &[ControllerSelection]) -> Result<(), RouterError> {
            self.slots.resize(selections.len(), None);
            Ok(())
        }

        fn set_sensor_enabled(
            &mut self,
            instance: i32,
            _sensor: ControllerSensor,
            enabled: bool,
        ) -> Result<ControllerOperationResult, RouterError> {
            self.sensors.push((instance, enabled));
            Ok(ControllerOperationResult::Accepted)
        }

        fn poll_events(&mut self) -> Result<Vec<ControllerEvent>, RouterError> {
            Ok(std::mem::take(&mut self.events))
        }

        fn snapshot(&mut self, _instance: i32) -> Result<Option<ControllerState>, RouterError> {
            Ok(None)
        }

        fn assignments(&mut self) -> Result<Vec<Option<i32>>, RouterError> {
            Ok(self.slots.clone())
        }
    }

    fn seat(seat: SeatId) -> Seat {
        Seat::new(seat, Dialect::Q3, Box::new(|_, _| false))
    }

    #[test]
    fn seat_runs_bindings_and_samples() {
        let owner = IdentityOwner::create("test").unwrap();
        let id = owner.seat(0);
        let mut seat = seat(id.clone());
        let mut commands = FakeRegistry { appended: Vec::new() };
        seat.bind(InputBinding {
            input: PhysicalInput::Key(32),
            target: InputBindingTarget::Command("+jump".to_string()),
        });
        seat.bind(InputBinding {
            input: PhysicalInput::Key(97),
            target: InputBindingTarget::Action(InputAction::Attack),
        });
        assert!(seat
            .input(
                &SeatInputEvent::Key {
                    seat: id.clone(),
                    time_ms: 10.0,
                    code: 32,
                    down: true,
                },
                &mut commands
            )
            .unwrap());
        assert!(seat.is_down(&PhysicalInput::Key(32)));
        assert_eq!(commands.appended, vec![("+jump 32 10\n".to_string(), id.clone())]);
        seat.input(
            &SeatInputEvent::Key {
                seat: id.clone(),
                time_ms: 12.0,
                code: 97,
                down: true,
            },
            &mut commands,
        )
        .unwrap();
        seat.input(
            &SeatInputEvent::MouseMotion {
                seat: id.clone(),
                time_ms: 12.0,
                position: (0.0, 0.0),
                delta: (4.0, -2.0),
            },
            &mut commands,
        )
        .unwrap();
        let frame = seat.sample(20.0, 10.0).unwrap();
        assert_eq!(frame.mouse, vec2(4.0, -2.0));
        assert_eq!(frame.any_key_down, 2);
        assert!(frame
            .buttons
            .iter()
            .any(|button| button.action == SourceAction::Action(InputAction::Attack) && button.active));
        assert!(seat.sample(20.0, 0.0).is_err());
        seat.release(30.0, &mut commands);
        assert!(!seat.has_held_input());
    }

    #[test]
    fn wheel_and_focus_flow() {
        let owner = IdentityOwner::create("test").unwrap();
        let id = owner.seat(0);
        let mut seat = seat(id.clone());
        let mut commands = FakeRegistry { appended: Vec::new() };
        seat.bind(InputBinding {
            input: PhysicalInput::Key(KeyCode::MouseWheelUp as i32),
            target: InputBindingTarget::Command("weapprev".to_string()),
        });
        seat.input(
            &SeatInputEvent::MouseWheel {
                seat: id.clone(),
                time_ms: 5.0,
                delta: (0.0, 2.0),
            },
            &mut commands,
        )
        .unwrap();
        assert_eq!(commands.appended.len(), 2);
        seat.input(
            &SeatInputEvent::Focus {
                seat: id.clone(),
                time_ms: 6.0,
                focused: false,
            },
            &mut commands,
        )
        .unwrap();
        assert!(!seat.focused());
        assert!(!seat
            .input(
                &SeatInputEvent::Key {
                    seat: id.clone(),
                    time_ms: 7.0,
                    code: 32,
                    down: true,
                },
                &mut commands
            )
            .unwrap());
    }

    #[test]
    fn router_routes_keys_buttons_and_focus() {
        let owner = IdentityOwner::create("test").unwrap();
        let first = owner.seat(0);
        let second = owner.seat(1);
        let dropped = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = dropped.clone();
        let mut router = InputRouter::new(
            vec![
                SeatRoute {
                    seat: seat(first.clone()),
                    controller: ControllerSelection::Automatic,
                },
                SeatRoute {
                    seat: seat(second.clone()),
                    controller: ControllerSelection::None,
                },
            ],
            Some(first.clone()),
            Some(Box::new(FakeControllers {
                events: Vec::new(),
                slots: vec![None, None],
                sensors: Vec::new(),
            })),
            Box::new(FakeRegistry { appended: Vec::new() }),
            Box::new(|| 1000.0),
            Box::new(|| 990),
            true,
            Box::new(move |event| sink.borrow_mut().push(event)),
            false,
            None,
        )
        .unwrap();
        router
            .attach_window(Box::new(FakeWindow {
                events: Vec::new(),
                relative: false,
            }))
            .unwrap();
        router
            .handle_platform(SdlEvent::Key {
                timestamp: 990,
                down: true,
                repeat: false,
                scancode: 4,
                keycode: 97,
                modifiers: 0,
            })
            .unwrap();
        assert!(router.seat(&first).unwrap().is_down(&PhysicalInput::Key(97)));
        assert!(!router.seat(&second).unwrap().is_down(&PhysicalInput::Key(97)));
        router
            .handle_platform(SdlEvent::Window {
                timestamp: 991,
                event: WINDOW_FOCUS_LOST,
                data1: 0,
                data2: 0,
            })
            .unwrap();
        assert!(!router.seat(&first).unwrap().has_held_input());
        router
            .handle_platform(SdlEvent::Window {
                timestamp: 992,
                event: WINDOW_FOCUS_GAINED,
                data1: 0,
                data2: 0,
            })
            .unwrap();
        router
            .handle_platform(SdlEvent::MouseButton {
                timestamp: 993,
                down: true,
                button: 1,
                clicks: 1,
                x: 10,
                y: 10,
            })
            .unwrap();
        assert!(router.seat(&first).unwrap().is_down(&PhysicalInput::MouseButton(1)));
        router.handle_platform(SdlEvent::Quit { timestamp: 994 }).unwrap();
        assert_eq!(dropped.borrow().len(), 1);
        router
            .retain_seats(std::slice::from_ref(&first), Some(first.clone()))
            .unwrap();
        assert_eq!(router.seats().len(), 1);
        assert_eq!(router.keyboard_seat(), Some(first));
        router.close().unwrap();
        assert!(router.handle_platform(SdlEvent::Quit { timestamp: 995 }).is_err());
    }

    #[test]
    fn router_assigns_controllers_and_calibrates_gyro() {
        let owner = IdentityOwner::create("test").unwrap();
        let id = owner.seat(0);
        let mut router = InputRouter::new(
            vec![SeatRoute {
                seat: seat(id.clone()),
                controller: ControllerSelection::Automatic,
            }],
            Some(id.clone()),
            Some(Box::new(FakeControllers {
                events: Vec::new(),
                slots: vec![Some(5)],
                sensors: Vec::new(),
            })),
            Box::new(FakeRegistry { appended: Vec::new() }),
            Box::new(|| 1000.0),
            Box::new(|| 990),
            false,
            Box::new(|_| {}),
            false,
            None,
        )
        .unwrap();
        router.restart().unwrap();
        assert_eq!(router.controller_for(&id), Some(5));
        assert_eq!(
            router.begin_gyro_calibration(&id).unwrap(),
            ControllerOperationResult::Accepted
        );
        assert!(matches!(
            router.gyro_calibration(&id).unwrap(),
            crate::input::gamepad::GyroCalibrationState::Calibrating { .. }
        ));
        router
            .handle_controller(ControllerEvent::Button {
                timestamp: 990,
                instance: 5,
                slot: Some(0),
                button: 0,
                down: true,
            })
            .unwrap();
        assert!(router
            .seat(&id)
            .unwrap()
            .is_down(&PhysicalInput::ControllerButton { device: 5, button: 0 }));
        router
            .handle_controller(ControllerEvent::Disconnected {
                timestamp: 991,
                instance: 5,
                slot: Some(0),
            })
            .unwrap();
        assert_eq!(router.controller_for(&id), None);
        router.cancel_gyro_calibration(&id).unwrap();
    }
}
