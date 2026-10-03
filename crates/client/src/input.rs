//! Input handling: buttons, mouse, seats, bindings, gamepad, MIDI,
//! haptics, source devices, routing, and per-family commands.
//!
//! Donor provenance: `src/input/buttons.ts` (`InputButton`),
//! `src/input/mouse.ts` (`MouseInput`), `src/input/pitch-drift.ts`
//! (`PitchDrift`), `src/input/impulse.ts` (`parseImpulse`),
//! `src/input/user-command.ts` (`InputCommandBuilder`),
//! `src/input/binding-store.ts` (`BindingStore`, `physicalInputKey`),
//! `src/input/key-codes.ts` (`KeyCode`, `KEY_CHAR_FLAG`),
//! `src/input/mouse-buttons.ts` (`quakeMouseButton`),
//! `src/input/seat.ts` (`SeatInput`, `SourceAction`),
//! `src/input/keys.ts` (`stringToKeynum`, `keynumToString`),
//! `src/input/bindings.ts` (named inputs, defaults, `bind` family),
//! `src/input/sdl-keys.ts` (`sdlGameKey`, `sdlEventTime`),
//! `src/input/mouse-settings.ts` (`MouseSettings`),
//! `src/input/device-settings.ts` (MIDI/source cvars),
//! `src/input/weapon-bindings.ts` (catalogs, `use` resolution),
//! `src/input/client-commands.ts` (`ClientCommandBindings`),
//! `src/input/router.ts` (`InputRouter`), `src/input/haptics.ts`
//! (BNVIB), `src/input/gamepad.ts` (`GamepadInput`),
//! `src/input/midi.ts` (`SourceMidiInput`),
//! `src/input/source-input.ts` (`SourceInputState`),
//! `src/input/source-joystick.ts` (`SourceJoystickState`),
//! `src/input/source-midi.ts` (`SourceMidiDecoder`) and
//! `src/contracts/{protocol.ts (UserCommand),ui.ts,common.ts}`.
//!
//! Script execution for key bindings stays with application wiring
//! through [`commands::CommandRegistry`]; this module owns device
//! state, bindings, and command generation. SDL and controller event
//! shapes come from `qa-platform`; headless tests inject synthetic
//! events and fake backends.

pub mod bindings;
pub mod commands;
pub mod device;
pub mod gamepad;
pub mod haptics;
pub mod joystick;
pub mod keycodes;
pub mod midi;
pub mod mouse_buttons;
pub mod mouse_settings;
pub mod router;
pub mod sdl_keys;
pub mod source;
pub mod source_midi;
pub mod weapons;

use std::collections::{BTreeMap, BTreeSet};

use qa_core::math::{Vec2, Vec3};
use qa_world::client::ClientFamily;

use crate::ClientError;

/// Key codes (`ui/keycodes.h`); printable ASCII passes through directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum KeyCode {
    /// Tab.
    Tab = 9,
    /// Enter.
    Enter = 13,
    /// Escape.
    Escape = 27,
    /// Space.
    Space = 32,
    /// Backspace.
    Backspace = 127,
    /// Command.
    Command = 128,
    /// Caps lock.
    CapsLock = 129,
    /// Power.
    Power = 130,
    /// Pause.
    Pause = 131,
    /// Cursor up.
    Up = 132,
    /// Cursor down.
    Down = 133,
    /// Cursor left.
    Left = 134,
    /// Cursor right.
    Right = 135,
    /// Alt.
    Alt = 136,
    /// Control.
    Control = 137,
    /// Shift.
    Shift = 138,
    /// Insert.
    Insert = 139,
    /// Delete.
    Delete = 140,
    /// Page down.
    PageDown = 141,
    /// Page up.
    PageUp = 142,
    /// Home.
    Home = 143,
    /// End.
    End = 144,
    /// F1.
    F1 = 145,
    /// F2.
    F2 = 146,
    /// F3.
    F3 = 147,
    /// F4.
    F4 = 148,
    /// F5.
    F5 = 149,
    /// F6.
    F6 = 150,
    /// F7.
    F7 = 151,
    /// F8.
    F8 = 152,
    /// F9.
    F9 = 153,
    /// F10.
    F10 = 154,
    /// F11.
    F11 = 155,
    /// F12.
    F12 = 156,
    /// F13.
    F13 = 157,
    /// F14.
    F14 = 158,
    /// F15.
    F15 = 159,
    /// Keypad home.
    KeypadHome = 160,
    /// Keypad up.
    KeypadUp = 161,
    /// Keypad page up.
    KeypadPageUp = 162,
    /// Keypad left.
    KeypadLeft = 163,
    /// Keypad 5.
    Keypad5 = 164,
    /// Keypad right.
    KeypadRight = 165,
    /// Keypad end.
    KeypadEnd = 166,
    /// Keypad down.
    KeypadDown = 167,
    /// Keypad page down.
    KeypadPageDown = 168,
    /// Keypad enter.
    KeypadEnter = 169,
    /// Keypad insert.
    KeypadInsert = 170,
    /// Keypad delete.
    KeypadDelete = 171,
    /// Keypad slash.
    KeypadSlash = 172,
    /// Keypad minus.
    KeypadMinus = 173,
    /// Keypad plus.
    KeypadPlus = 174,
    /// Keypad num lock.
    KeypadNumLock = 175,
    /// Keypad star.
    KeypadStar = 176,
    /// Keypad equals.
    KeypadEquals = 177,
    /// Mouse button 1.
    Mouse1 = 178,
    /// Mouse button 2.
    Mouse2 = 179,
    /// Mouse button 3.
    Mouse3 = 180,
    /// Mouse button 4.
    Mouse4 = 181,
    /// Mouse button 5.
    Mouse5 = 182,
    /// Wheel down.
    MouseWheelDown = 183,
    /// Wheel up.
    MouseWheelUp = 184,
    /// Joystick button 1.
    Joy1 = 185,
    /// Joystick button 2.
    Joy2 = 186,
    /// Joystick button 3.
    Joy3 = 187,
    /// Joystick button 4.
    Joy4 = 188,
    /// Joystick button 5.
    Joy5 = 189,
    /// Joystick button 6.
    Joy6 = 190,
    /// Joystick button 7.
    Joy7 = 191,
    /// Joystick button 8.
    Joy8 = 192,
    /// Joystick button 9.
    Joy9 = 193,
    /// Joystick button 10.
    Joy10 = 194,
    /// Joystick button 11.
    Joy11 = 195,
    /// Joystick button 12.
    Joy12 = 196,
    /// Joystick button 13.
    Joy13 = 197,
    /// Joystick button 14.
    Joy14 = 198,
    /// Joystick button 15.
    Joy15 = 199,
    /// Joystick button 16.
    Joy16 = 200,
    /// Joystick button 17.
    Joy17 = 201,
    /// Joystick button 18.
    Joy18 = 202,
    /// Joystick button 19.
    Joy19 = 203,
    /// Joystick button 20.
    Joy20 = 204,
    /// Joystick button 21.
    Joy21 = 205,
    /// Joystick button 22.
    Joy22 = 206,
    /// Joystick button 23.
    Joy23 = 207,
    /// Joystick button 24.
    Joy24 = 208,
    /// Joystick button 25.
    Joy25 = 209,
    /// Joystick button 26.
    Joy26 = 210,
    /// Joystick button 27.
    Joy27 = 211,
    /// Joystick button 28.
    Joy28 = 212,
    /// Joystick button 29.
    Joy29 = 213,
    /// Joystick button 30.
    Joy30 = 214,
    /// Joystick button 31.
    Joy31 = 215,
    /// Joystick button 32.
    Joy32 = 216,
    /// Auxiliary key 1.
    Aux1 = 217,
    /// Auxiliary key 2.
    Aux2 = 218,
    /// Auxiliary key 3.
    Aux3 = 219,
    /// Auxiliary key 4.
    Aux4 = 220,
    /// Auxiliary key 5.
    Aux5 = 221,
    /// Auxiliary key 6.
    Aux6 = 222,
    /// Auxiliary key 7.
    Aux7 = 223,
    /// Auxiliary key 8.
    Aux8 = 224,
    /// Auxiliary key 9.
    Aux9 = 225,
    /// Auxiliary key 10.
    Aux10 = 226,
    /// Auxiliary key 11.
    Aux11 = 227,
    /// Auxiliary key 12.
    Aux12 = 228,
    /// Auxiliary key 13.
    Aux13 = 229,
    /// Auxiliary key 14.
    Aux14 = 230,
    /// Auxiliary key 15.
    Aux15 = 231,
    /// Auxiliary key 16.
    Aux16 = 232,
}

/// Flag ORed onto character codes delivered as keys.
pub const KEY_CHAR_FLAG: i32 = 1024;

/// Map a physical (SDL) mouse button to a Quake button (middle/right swap).
#[must_use]
pub const fn quake_mouse_button(physical_button: i32) -> i32 {
    if physical_button == 2 {
        3
    } else if physical_button == 3 {
        2
    } else {
        physical_button
    }
}

/// Map a Quake mouse button back to its physical ID (same swap).
#[must_use]
pub const fn physical_mouse_button(quake_button: i32) -> i32 {
    quake_mouse_button(quake_button)
}

/// Named input action (`InputAction` contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputAction {
    /// Fire.
    Attack,
    /// Jump.
    Jump,
    /// Move forward.
    Forward,
    /// Move back.
    Back,
    /// Strafe left.
    MoveLeft,
    /// Strafe right.
    MoveRight,
    /// Move up / jump (Q1).
    MoveUp,
    /// Move down / crouch (Q1).
    MoveDown,
    /// Use.
    Use,
    /// Crouch.
    Crouch,
    /// Walk modifier.
    Walk,
    /// Scoreboard.
    Scores,
    /// Next weapon.
    NextWeapon,
    /// Previous weapon.
    PreviousWeapon,
    /// Menu.
    Menu,
}

/// Source action: named actions plus turn/look/strafe and Q3 buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceAction {
    /// Named action.
    Action(InputAction),
    /// Turn left.
    TurnLeft,
    /// Turn right.
    TurnRight,
    /// Look up.
    LookUp,
    /// Look down.
    LookDown,
    /// Strafe modifier.
    Strafe,
    /// Mouse look.
    MouseLook,
    /// Keyboard look.
    KeyLook,
    /// Holster (Q2 rerelease).
    Holster,
    /// Q3 `buttonN` (0..15).
    Button(u8),
}

/// Physical input device event (`PhysicalInput` contract).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PhysicalInput {
    /// Keyboard code.
    Key(i32),
    /// Physical mouse button ID.
    MouseButton(i32),
    /// Controller button.
    ControllerButton {
        /// Device index.
        device: i32,
        /// Button index.
        button: i32,
    },
    /// Controller axis direction.
    ControllerAxis {
        /// Device index.
        device: i32,
        /// Axis name.
        axis: ControllerAxis,
        /// Held direction.
        direction: AxisDirection,
    },
}

/// Controller axis names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControllerAxis {
    /// Left stick X.
    LeftX,
    /// Left stick Y.
    LeftY,
    /// Right stick X.
    RightX,
    /// Right stick Y.
    RightY,
    /// Left trigger.
    LeftTrigger,
    /// Right trigger.
    RightTrigger,
}

/// Controller axis direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisDirection {
    /// Positive deflection.
    Positive,
    /// Negative deflection.
    Negative,
}

/// Binding-table key (`physicalInputKey`).
#[must_use]
pub fn physical_input_key(input: &PhysicalInput) -> String {
    match input {
        PhysicalInput::Key(code) => format!("key:{code}"),
        PhysicalInput::MouseButton(button) => format!("mouse:{button}"),
        PhysicalInput::ControllerButton { device, button } => {
            format!("pad:{device}:button:{button}")
        }
        PhysicalInput::ControllerAxis {
            device,
            axis,
            direction,
        } => {
            let axis = match axis {
                ControllerAxis::LeftX => "left-x",
                ControllerAxis::LeftY => "left-y",
                ControllerAxis::RightX => "right-x",
                ControllerAxis::RightY => "right-y",
                ControllerAxis::LeftTrigger => "left-trigger",
                ControllerAxis::RightTrigger => "right-trigger",
            };
            let direction = match direction {
                AxisDirection::Positive => "positive",
                AxisDirection::Negative => "negative",
            };
            format!("pad:{device}:axis:{axis}:{direction}")
        }
    }
}

/// Binding target: named action or console command text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputBindingTarget {
    /// Named action.
    Action(InputAction),
    /// Console command text.
    Command(String),
}

/// Input binding: physical input to target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBinding {
    /// Physical input.
    pub input: PhysicalInput,
    /// Binding target.
    pub target: InputBindingTarget,
}

/// Device-free binding table (`BindingStore`).
#[derive(Debug, Clone, Default)]
pub struct BindingTable {
    bindings: BTreeMap<String, InputBinding>,
}

impl BindingTable {
    /// Empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bindings: BTreeMap::new(),
        }
    }

    /// All bindings.
    #[must_use]
    pub fn bindings(&self) -> Vec<&InputBinding> {
        self.bindings.values().collect()
    }

    /// Target bound to an input, if any.
    #[must_use]
    pub fn binding(&self, input: &PhysicalInput) -> Option<&InputBindingTarget> {
        self.bindings
            .get(&physical_input_key(input))
            .map(|binding| &binding.target)
    }

    /// Bind an input, replacing any previous binding.
    pub fn bind(&mut self, binding: InputBinding) {
        self.bindings.insert(physical_input_key(&binding.input), binding);
    }

    /// Remove a binding.
    pub fn unbind(&mut self, input: &PhysicalInput) {
        self.bindings.remove(&physical_input_key(input));
    }

    /// Remove all bindings.
    pub fn unbind_all(&mut self) {
        self.bindings.clear();
    }
}

/// Seat focus (`SeatInputFocus` contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatFocus {
    /// In-game input.
    Game,
    /// Menu input.
    Menu {
        /// Menu ID.
        menu: String,
        /// Focused control, if any.
        control: Option<String>,
    },
    /// Console input.
    Console,
    /// Chat input.
    Chat {
        /// Team chat.
        team: bool,
        /// Current text.
        text: String,
    },
}

/// Seat key state: held inputs, bindings, focus, and impulse.
#[derive(Debug, Clone)]
pub struct SeatKeys {
    held: BTreeSet<String>,
    /// Binding table.
    pub bindings: BindingTable,
    /// Current focus.
    pub focus: SeatFocus,
    pending_impulse: u8,
}

impl SeatKeys {
    /// Fresh seat in game focus.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: BTreeSet::new(),
            bindings: BindingTable::new(),
            focus: SeatFocus::Game,
            pending_impulse: 0,
        }
    }

    /// Whether an input is held.
    #[must_use]
    pub fn is_down(&self, input: &PhysicalInput) -> bool {
        self.held.contains(&physical_input_key(input))
    }

    /// Press an input (repeats are ignored).
    pub fn press(&mut self, input: &PhysicalInput) {
        self.held.insert(physical_input_key(input));
    }

    /// Release an input.
    pub fn release(&mut self, input: &PhysicalInput) {
        self.held.remove(&physical_input_key(input));
    }

    /// Release everything (focus loss).
    pub fn release_all(&mut self) {
        self.held.clear();
    }

    /// Whether anything is held.
    #[must_use]
    pub fn has_held(&self) -> bool {
        !self.held.is_empty()
    }

    /// Set the pending impulse byte.
    pub fn set_impulse(&mut self, value: u8) {
        self.pending_impulse = value;
    }

    /// Pending impulse byte.
    #[must_use]
    pub const fn impulse(&self) -> u8 {
        self.pending_impulse
    }
}

impl Default for SeatKeys {
    fn default() -> Self {
        Self::new()
    }
}

/// Button sampling timing: Q1 fractions, Q2/Q3 millisecond shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonTiming {
    /// Quake I impulse fractions.
    Q1,
    /// Quake II millisecond share.
    Q2,
    /// Quake III `float32` millisecond share.
    Q3,
}

/// `kbutton_t` state with multi-source tracking.
#[derive(Debug, Clone, Default)]
pub struct InputButton {
    sources: BTreeSet<String>,
    down_time_ms: i64,
    milliseconds: i64,
    impulse_down: bool,
    impulse_up: bool,
}

/// Default hold credit when the release timestamp is missing.
pub const MISSING_TIME_MS: i64 = 10;

impl InputButton {
    /// Released button.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sources: BTreeSet::new(),
            down_time_ms: 0,
            milliseconds: 0,
            impulse_down: false,
            impulse_up: false,
        }
    }

    /// Whether any source holds the button.
    #[must_use]
    pub fn active(&self) -> bool {
        !self.sources.is_empty()
    }

    /// Whether the button went down since the last sample.
    #[must_use]
    pub const fn pressed(&self) -> bool {
        self.impulse_down
    }

    /// Press from a source (repeat presses are ignored).
    pub fn down(&mut self, source: &str, time_ms: i64) {
        if self.sources.contains(source) {
            return;
        }
        let was_active = self.active();
        self.sources.insert(source.to_string());
        if was_active {
            return;
        }
        self.down_time_ms = time_ms;
        self.impulse_down = true;
    }

    /// Release a source, crediting held time when the last source lifts.
    pub fn up(&mut self, source: &str, time_ms: i64) {
        if !self.sources.remove(source) || !self.sources.is_empty() {
            return;
        }
        self.milliseconds += if time_ms == 0 {
            MISSING_TIME_MS
        } else {
            (time_ms - self.down_time_ms).max(0)
        };
        self.impulse_up = true;
    }

    /// Release all sources.
    pub fn release(&mut self, time_ms: i64) {
        let sources: Vec<String> = self.sources.iter().cloned().collect();
        for source in &sources {
            self.up(source, time_ms);
        }
    }

    /// Sample the held fraction and clear impulses.
    pub fn sample(&mut self, timing: ButtonTiming, now_ms: i64, frame_ms: f64) -> f32 {
        let result = match timing {
            ButtonTiming::Q1 => {
                if self.impulse_down && self.impulse_up {
                    if self.active() {
                        0.75
                    } else {
                        0.25
                    }
                } else if self.impulse_down {
                    if self.active() {
                        0.5
                    } else {
                        0.0
                    }
                } else if self.active() {
                    1.0
                } else {
                    0.0
                }
            }
            ButtonTiming::Q2 | ButtonTiming::Q3 => {
                if self.active() {
                    self.milliseconds += if self.down_time_ms == 0 {
                        now_ms
                    } else {
                        (now_ms - self.down_time_ms).max(0)
                    };
                    self.down_time_ms = now_ms;
                }
                let value = if timing == ButtonTiming::Q3 {
                    f64::from(self.milliseconds as f32 / frame_ms as f32)
                } else {
                    self.milliseconds as f64 / frame_ms
                };
                value.clamp(0.0, 1.0) as f32
            }
        };
        self.milliseconds = 0;
        self.impulse_down = false;
        self.impulse_up = false;
        result
    }

    /// Clear all state.
    pub fn clear(&mut self) {
        self.sources.clear();
        self.milliseconds = 0;
        self.down_time_ms = 0;
        self.impulse_down = false;
        self.impulse_up = false;
    }
}

/// Mouse tuning (`MouseTuning`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseTuning {
    /// Base sensitivity.
    pub sensitivity: f64,
    /// Acceleration gain.
    pub acceleration: f64,
    /// Average with the previous sample.
    pub filter: bool,
    /// Yaw degrees per unit.
    pub yaw: f64,
    /// Pitch degrees per unit.
    pub pitch: f64,
    /// Strafe units per unit.
    pub side: f64,
    /// Forward units per unit.
    pub forward: f64,
    /// Free look without `mlook`.
    pub free_look: bool,
    /// Snap pitch back when `mlook` releases.
    pub look_spring: bool,
    /// Strafe while mouse-looking.
    pub look_strafe: bool,
    /// Invert pitch.
    pub invert_pitch: bool,
}

/// Default tuning: sensitivity 3, yaw/pitch 0.022, side 0.8, free look.
#[must_use]
pub const fn default_mouse_tuning() -> MouseTuning {
    MouseTuning {
        sensitivity: 3.0,
        acceleration: 0.0,
        filter: false,
        yaw: 0.022,
        pitch: 0.022,
        side: 0.8,
        forward: 1.0,
        free_look: true,
        look_spring: false,
        look_strafe: false,
        invert_pitch: false,
    }
}

/// Sampled mouse movement.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MouseMove {
    /// Yaw degrees.
    pub yaw: f64,
    /// Pitch degrees.
    pub pitch: f64,
    /// Strafe units.
    pub side: f64,
    /// Forward units.
    pub forward: f64,
}

/// Q1/Q2/Q3 mouse filtering and scaling (`IN_MouseMove` / `CL_MouseMove`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseInput {
    previous: Vec2,
    /// Active tuning.
    pub tuning: MouseTuning,
}

impl MouseInput {
    /// Fresh mouse with default tuning.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            previous: Vec2 { x: 0.0, y: 0.0 },
            tuning: default_mouse_tuning(),
        }
    }

    /// Sample raw movement for a frame.
    pub fn sample(
        &mut self,
        raw: Vec2,
        frame_ms: f64,
        strafe: bool,
        mouse_look: bool,
        zoom_sensitivity: f64,
        binary32: bool,
    ) -> Result<MouseMove, ClientError> {
        if frame_ms <= 0.0 || !frame_ms.is_finite() {
            return Err(ClientError::BadMouseFrame);
        }
        let tuning = self.tuning;
        let (x, y) = if binary32 {
            let mut x = raw.x;
            let mut y = raw.y;
            if tuning.filter {
                x = (raw.x + self.previous.x) * 0.5;
                y = (raw.y + self.previous.y) * 0.5;
            }
            self.previous = raw;
            let rate = (x * x + y * y).sqrt() / frame_ms as f32;
            let gain = (tuning.sensitivity as f32 + rate * tuning.acceleration as f32) * zoom_sensitivity as f32;
            (f64::from(x * gain), f64::from(y * gain))
        } else {
            let mut x = f64::from(raw.x);
            let mut y = f64::from(raw.y);
            if tuning.filter {
                x = (x + f64::from(self.previous.x)) * 0.5;
                y = (y + f64::from(self.previous.y)) * 0.5;
            }
            self.previous = raw;
            let rate = (x * x + y * y).sqrt() / frame_ms;
            let gain = (tuning.sensitivity + rate * tuning.acceleration) * zoom_sensitivity;
            (x * gain, y * gain)
        };
        let horizontal_strafe = strafe || tuning.look_strafe && mouse_look;
        Ok(MouseMove {
            yaw: if horizontal_strafe { 0.0 } else { -tuning.yaw * x },
            side: if horizontal_strafe { tuning.side * x } else { 0.0 },
            pitch: if !strafe && (mouse_look || tuning.free_look) {
                tuning.pitch * y * if tuning.invert_pitch { -1.0 } else { 1.0 }
            } else {
                0.0
            },
            forward: if strafe || !(mouse_look || tuning.free_look) {
                -tuning.forward * y
            } else {
                0.0
            },
        })
    }

    /// Clear the filter history.
    pub fn clear(&mut self) {
        self.previous = Vec2 { x: 0.0, y: 0.0 };
    }
}

impl Default for MouseInput {
    fn default() -> Self {
        Self::new()
    }
}

/// Q1 pitch-drift state: grounded, ideal pitch, and disable flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchDriftState {
    /// On ground.
    pub grounded: bool,
    /// Ideal pitch in degrees.
    pub ideal_pitch: f64,
    /// Drift disabled.
    pub disabled: bool,
}

/// Q1 `V_DriftPitch` state machine.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PitchDrift {
    stopped: bool,
    velocity: f64,
    moving_seconds: f64,
}

impl PitchDrift {
    /// Stopped drift.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stopped: true,
            velocity: 0.0,
            moving_seconds: 0.0,
        }
    }

    /// Clear drift state.
    pub fn clear(&mut self) {
        self.stopped = true;
        self.velocity = 0.0;
        self.moving_seconds = 0.0;
    }

    /// Drift the pitch toward ideal (`V_DriftPitch`).
    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &mut self,
        pitch: f64,
        elapsed_ms: f64,
        state: &PitchDriftState,
        manual: bool,
        start: bool,
        forward: f64,
        forward_threshold: f64,
        speed: f64,
        delay: f64,
    ) -> f64 {
        if manual {
            self.clear();
        } else if start && (self.stopped || self.velocity == 0.0) {
            self.stopped = false;
            self.velocity = speed;
            self.moving_seconds = 0.0;
        }
        if state.disabled || !state.grounded {
            self.moving_seconds = 0.0;
            self.velocity = 0.0;
            return pitch;
        }
        let seconds = elapsed_ms / 1000.0;
        if self.stopped {
            self.moving_seconds = if manual || forward.abs() < forward_threshold {
                0.0
            } else {
                self.moving_seconds + seconds
            };
            if self.moving_seconds > delay {
                self.stopped = false;
                self.velocity = speed;
                self.moving_seconds = 0.0;
            }
            return pitch;
        }
        let delta = state.ideal_pitch - pitch;
        if delta == 0.0 {
            self.velocity = 0.0;
            return pitch;
        }
        let step = delta.abs().min(seconds * self.velocity);
        self.velocity += seconds * speed;
        if step == delta.abs() {
            self.velocity = 0.0;
        }
        pitch + delta.signum() * step
    }
}

/// Quake `Q_atoi` with hex and character-literal forms (`IN_Impulse`).
fn quake_integer(text: &str) -> i32 {
    let bytes = text.as_bytes();
    let mut offset = usize::from(bytes.first() == Some(&b'-'));
    let sign = if offset == 1 { -1 } else { 1 };
    let mut value: i32 = 0;
    if bytes.get(offset) == Some(&b'0') && matches!(bytes.get(offset + 1), Some(b'x' | b'X')) {
        offset += 2;
        while offset < bytes.len() {
            let code = bytes[offset];
            let digit = if code.is_ascii_digit() {
                i32::from(code) - 48
            } else if (b'a'..=b'f').contains(&code) {
                i32::from(code) - i32::from(b'a') + 10
            } else if (b'A'..=b'F').contains(&code) {
                i32::from(code) - i32::from(b'A') + 10
            } else {
                break;
            };
            value = value.wrapping_shl(4).wrapping_add(digit);
            offset += 1;
        }
    } else if bytes.get(offset) == Some(&b'\'') {
        value = bytes.get(offset + 1).map_or(0, |code| i32::from(*code));
    } else {
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            value = value.wrapping_mul(10).wrapping_add(i32::from(bytes[offset]) - 48);
            offset += 1;
        }
    }
    value.wrapping_mul(sign)
}

/// JavaScript `parseInt(text, 10)` truncated to a byte (Q2 classic).
fn js_parse_int_byte(text: &str) -> i32 {
    let trimmed = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}' || c == '\u{00A0}');
    let (sign, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (-1i64, &trimmed[1..]),
        Some(b'+') => (1i64, &trimmed[1..]),
        _ => (1i64, trimmed),
    };
    let mut value: i64 = 0;
    let mut any = false;
    for byte in digits.bytes() {
        if !byte.is_ascii_digit() {
            break;
        }
        any = true;
        value = value.wrapping_mul(10).wrapping_add(i64::from(byte) - 48);
    }
    if !any {
        return 0;
    }
    (value.wrapping_mul(sign) as i32) & 255
}

/// JavaScript `Number(text)` (Q2 rerelease / Q3 impulse path).
fn js_number(text: &str) -> f64 {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}' || c == '\u{00A0}');
    if trimmed.is_empty() {
        return 0.0;
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .or_else(|| trimmed.strip_prefix("+0x"))
        .or_else(|| trimmed.strip_prefix("+0X"))
    {
        return i64::from_str_radix(hex, 16).map_or(f64::NAN, |value| value as f64);
    }
    if let Some(hex) = trimmed.strip_prefix("-0x").or_else(|| trimmed.strip_prefix("-0X")) {
        return i64::from_str_radix(hex, 16).map_or(f64::NAN, |value| -(value as f64));
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

/// Parse an impulse for a family (`IN_Impulse`).
///
/// Q1 masks Quake-`atoi` to a byte; Q2 classic masks decimal `parseInt`;
/// Q2 rerelease and Q3 return the raw number (validated downstream).
#[must_use]
pub fn parse_impulse(text: &str, family: ClientFamily) -> f64 {
    match family {
        ClientFamily::Q1Netquake | ClientFamily::Q1Quakeworld => f64::from(quake_integer(text) & 255),
        ClientFamily::Q2Classic => f64::from(js_parse_int_byte(text)),
        ClientFamily::Q2Rerelease | ClientFamily::Q3 => js_number(text),
    }
}

/// Validate a parsed impulse as a seat impulse byte (`setImpulse`).
pub fn seat_impulse(value: f64) -> Result<u8, ClientError> {
    if !value.is_finite() || value.fract() != 0.0 || !(0.0..=255.0).contains(&value) {
        return Err(ClientError::BadImpulse);
    }
    Ok(value as u8)
}

/// View/input tuning (`ViewInputTuning`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewInputTuning {
    /// Forward speed.
    pub forward_speed: f64,
    /// Back speed.
    pub back_speed: f64,
    /// Side speed (350 for Q1, 200 otherwise).
    pub side_speed: f64,
    /// Up speed.
    pub up_speed: f64,
    /// Keyboard yaw speed.
    pub yaw_speed: f64,
    /// Keyboard pitch speed.
    pub pitch_speed: f64,
    /// Angle speed multiplier while walking.
    pub angle_speed_multiplier: f64,
    /// Move multiplier while running.
    pub move_speed_multiplier: f64,
    /// Always run (Q1 defaults off, others on).
    pub always_run: bool,
}

/// Default tuning for a family.
#[must_use]
pub const fn default_tuning(family: ClientFamily) -> ViewInputTuning {
    ViewInputTuning {
        forward_speed: 200.0,
        back_speed: 200.0,
        side_speed: match family {
            ClientFamily::Q1Netquake | ClientFamily::Q1Quakeworld => 350.0,
            ClientFamily::Q2Classic | ClientFamily::Q2Rerelease | ClientFamily::Q3 => 200.0,
        },
        up_speed: 200.0,
        yaw_speed: 140.0,
        pitch_speed: 150.0,
        angle_speed_multiplier: 1.5,
        move_speed_multiplier: 2.0,
        always_run: match family {
            ClientFamily::Q1Netquake | ClientFamily::Q1Quakeworld => false,
            ClientFamily::Q2Classic | ClientFamily::Q2Rerelease | ClientFamily::Q3 => true,
        },
    }
}

/// One sampled button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonSample {
    /// Action.
    pub action: SourceAction,
    /// Held fraction for the frame.
    pub fraction: f32,
    /// Held at sample time.
    pub active: bool,
    /// Went down this frame.
    pub pressed: bool,
}

/// One frame of seat input for command generation.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatSample {
    /// Sampled buttons.
    pub buttons: Vec<ButtonSample>,
    /// Raw mouse delta.
    pub mouse: Vec2,
    /// Gamepad move stick.
    pub gamepad_move: Vec2,
    /// Gamepad look contribution in degrees.
    pub gamepad_look_degrees: Vec2,
    /// Pending impulse.
    pub impulse: i32,
    /// Nonzero when any key is down.
    pub any_key_down: i32,
    /// Whether focus is in-game.
    pub focus_is_game: bool,
    /// Frame length in milliseconds.
    pub frame_ms: f64,
}

/// Per-family frame context (`UserCommandFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameContext {
    /// NetQuake acknowledgement plus optional pitch drift.
    Q1Netquake {
        /// Acknowledged server time in seconds.
        ack_time_s: f64,
        /// Pitch-drift state, when the server sent client data.
        pitch_drift: Option<PitchDriftState>,
    },
    /// QuakeWorld plus optional pitch drift.
    Q1Quakeworld {
        /// Pitch-drift state, when the server sent client data.
        pitch_drift: Option<PitchDriftState>,
    },
    /// Q2 classic delta angles, light level, and attack gate.
    Q2Classic {
        /// Delta angles.
        delta_angles: Vec3,
        /// Light level.
        light_level: i32,
        /// Whether attack is allowed this frame.
        attack_allowed: bool,
    },
    /// Q2 rerelease delta angles, server frame, and attack gate.
    Q2Rerelease {
        /// Delta angles.
        delta_angles: Vec3,
        /// Current server frame.
        server_frame: i32,
        /// Whether attack is allowed this frame.
        attack_allowed: bool,
    },
    /// Q3 server time, weapon, and zoom sensitivity.
    Q3 {
        /// Server time in milliseconds.
        server_time_ms: i32,
        /// Selected weapon.
        weapon: i32,
        /// Zoom sensitivity scale.
        sensitivity: f64,
    },
}

/// Built per-family user command (`UserCommand` contract).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BuiltCommand {
    /// NetQuake move.
    Q1Netquake {
        /// Acknowledged server time in seconds.
        ack_time_s: f64,
        /// View angles.
        view_angles: Vec3,
        /// Forward units.
        forward: i32,
        /// Side units.
        side: i32,
        /// Up units.
        up: i32,
        /// Button bits.
        buttons: i32,
        /// Impulse byte.
        impulse: i32,
    },
    /// QuakeWorld move.
    Q1Quakeworld {
        /// Frame milliseconds (capped at 100 past 250).
        msec: i32,
        /// View angles.
        angles: Vec3,
        /// Forward units.
        forward: i32,
        /// Side units.
        side: i32,
        /// Up units.
        up: i32,
        /// Button bits.
        buttons: i32,
        /// Impulse byte.
        impulse: i32,
    },
    /// Q2 classic move.
    Q2Classic {
        /// Frame milliseconds.
        msec: i32,
        /// Angle shorts.
        angle_shorts: [i32; 3],
        /// Forward units.
        forward: i32,
        /// Side units.
        side: i32,
        /// Up units.
        up: i32,
        /// Button bits.
        buttons: i32,
        /// Impulse byte.
        impulse: i32,
        /// Light level.
        light_level: i32,
    },
    /// Q2 rerelease move.
    Q2Rerelease {
        /// Frame milliseconds.
        msec: i32,
        /// View angles.
        angles: Vec3,
        /// Forward units.
        forward: f64,
        /// Side units.
        side: f64,
        /// Button bits.
        buttons: i32,
        /// Server frame.
        server_frame: i32,
    },
    /// Q3 move.
    Q3 {
        /// Server time in milliseconds.
        server_time_ms: i32,
        /// Angle words.
        angle_words: [i32; 3],
        /// Forward units.
        forward: i32,
        /// Right units.
        right: i32,
        /// Up units.
        up: i32,
        /// Button bits.
        buttons: i32,
        /// Selected weapon.
        weapon: i32,
    },
}

fn angle_word(angle: f64) -> i32 {
    (angle * 65536.0 / 360.0) as i32 & 65535
}

fn q1_yaw(angle: f64) -> f64 {
    f64::from(angle_word(angle)) * (360.0 / 65536.0)
}

/// `CL_AdjustAngles` / `CL_BaseMove` / `CL_FinishMove` command builder.
#[derive(Debug, Clone, PartialEq)]
pub struct InputCommandBuilder {
    family: ClientFamily,
    /// Owned mouse filter state.
    pub mouse: MouseInput,
    tuning: ViewInputTuning,
    angles: [f64; 3],
    pitch_drift: PitchDrift,
    previous_mlook: bool,
    drift_speed: f64,
    drift_delay: f64,
}

impl InputCommandBuilder {
    /// Builder with default tuning for a family.
    #[must_use]
    pub fn new(family: ClientFamily) -> Self {
        Self {
            family,
            mouse: MouseInput::new(),
            tuning: default_tuning(family),
            angles: [0.0, 0.0, 0.0],
            pitch_drift: PitchDrift::new(),
            previous_mlook: false,
            drift_speed: 500.0,
            drift_delay: 0.15,
        }
    }

    /// Command family.
    #[must_use]
    pub const fn family(&self) -> ClientFamily {
        self.family
    }

    /// Active tuning.
    #[must_use]
    pub const fn tuning(&self) -> ViewInputTuning {
        self.tuning
    }

    /// Replace tuning.
    pub fn set_tuning(&mut self, tuning: ViewInputTuning) {
        self.tuning = tuning;
    }

    /// Pitch-drift speed/delay settings.
    pub fn set_drift_settings(&mut self, speed: f64, delay: f64) {
        self.drift_speed = speed;
        self.drift_delay = delay;
    }

    /// Current view angles.
    #[must_use]
    pub fn view_angles(&self) -> Vec3 {
        Vec3 {
            x: self.angles[0] as f32,
            y: self.angles[1] as f32,
            z: self.angles[2] as f32,
        }
    }

    /// Set view angles (must be finite).
    pub fn set_view_angles(&mut self, angles: Vec3) -> Result<(), ClientError> {
        if ![angles.x, angles.y, angles.z].iter().all(|value| value.is_finite()) {
            return Err(ClientError::BadViewAngles);
        }
        self.angles = [f64::from(angles.x), f64::from(angles.y), f64::from(angles.z)];
        Ok(())
    }

    /// Center the view with a pitch delta.
    pub fn center_view(&mut self, delta_pitch: f64) {
        self.angles[0] = -delta_pitch;
    }

    /// Clear angles, mouse, drift, and look state.
    pub fn clear(&mut self) {
        self.angles = [0.0, 0.0, 0.0];
        self.mouse.clear();
        self.pitch_drift.clear();
        self.previous_mlook = false;
    }

    /// Build one user command from a seat sample.
    pub fn build(&mut self, sample: &SeatSample, frame: &FrameContext) -> Result<BuiltCommand, ClientError> {
        let matches = matches!(
            (self.family, frame),
            (ClientFamily::Q1Netquake, FrameContext::Q1Netquake { .. })
                | (ClientFamily::Q1Quakeworld, FrameContext::Q1Quakeworld { .. })
                | (ClientFamily::Q2Classic, FrameContext::Q2Classic { .. })
                | (ClientFamily::Q2Rerelease, FrameContext::Q2Rerelease { .. })
                | (ClientFamily::Q3, FrameContext::Q3 { .. })
        );
        if !matches {
            return Err(ClientError::DialectMismatch);
        }
        let q3 = matches!(frame, FrameContext::Q3 { .. });
        let q1 = matches!(
            frame,
            FrameContext::Q1Netquake { .. } | FrameContext::Q1Quakeworld { .. }
        );
        let round: fn(f64) -> f64 = if q3 {
            |value| f64::from(value as f32)
        } else {
            |value| value
        };
        let sample_of = |action: SourceAction| sample.buttons.iter().find(|button| button.action == action);
        let fraction = |action: SourceAction| sample_of(action).map_or(0.0, |button| f64::from(button.fraction));
        let active = |action: SourceAction| sample_of(action).is_some_and(|button| button.active);
        let pressed = |action: SourceAction| sample_of(action).is_some_and(|button| button.active || button.pressed);
        let named = |action: InputAction| SourceAction::Action(action);
        let frame_ms = sample.frame_ms;
        let speed = active(named(InputAction::Walk));
        let strafe = active(SourceAction::Strafe);
        let klook = active(SourceAction::KeyLook);
        let angle_speed = round(frame_ms / 1000.0 * if speed { self.tuning.angle_speed_multiplier } else { 1.0 });
        let previous_pitch = self.angles[0];
        let mut pitch = self.angles[0];
        let mut yaw = self.angles[1];
        let mut roll = self.angles[2];
        if !strafe {
            yaw = round(yaw - round(round(angle_speed * self.tuning.yaw_speed) * fraction(SourceAction::TurnRight)));
            yaw = round(yaw + round(round(angle_speed * self.tuning.yaw_speed) * fraction(SourceAction::TurnLeft)));
            if q1 {
                yaw = q1_yaw(yaw);
            }
        }
        if klook && !q3 {
            pitch -= angle_speed * self.tuning.pitch_speed * fraction(named(InputAction::Forward));
            pitch += angle_speed * self.tuning.pitch_speed * fraction(named(InputAction::Back));
        }
        pitch = round(pitch - round(round(angle_speed * self.tuning.pitch_speed) * fraction(SourceAction::LookUp)));
        pitch = round(pitch + round(round(angle_speed * self.tuning.pitch_speed) * fraction(SourceAction::LookDown)));
        if q1 {
            pitch = pitch.clamp(-70.0, 80.0);
            roll = roll.clamp(-50.0, 50.0);
        }
        let running = speed != self.tuning.always_run;
        let move_speed = if q3 {
            if running {
                127.0
            } else {
                64.0
            }
        } else {
            1.0
        };
        let mut forward = 0.0;
        let mut side = 0.0;
        let mut up = 0.0;
        let add = |value: f64, amount: f64| -> f64 {
            if q3 {
                (round(round(value) + round(amount))).trunc()
            } else {
                value + amount
            }
        };
        if strafe {
            let speed = if q3 { move_speed } else { self.tuning.side_speed };
            side = add(side, round(speed * fraction(SourceAction::TurnRight)));
            side = add(side, -round(speed * fraction(SourceAction::TurnLeft)));
        }
        let side_speed = if q3 { move_speed } else { self.tuning.side_speed };
        side = add(side, round(side_speed * fraction(named(InputAction::MoveRight))));
        side = add(side, -round(side_speed * fraction(named(InputAction::MoveLeft))));
        let jump = if q1 {
            fraction(named(InputAction::MoveUp))
        } else {
            fraction(named(InputAction::Jump)).max(fraction(named(InputAction::MoveUp)))
        };
        let crouch = if q1 {
            fraction(named(InputAction::MoveDown))
        } else {
            fraction(named(InputAction::Crouch)).max(fraction(named(InputAction::MoveDown)))
        };
        let up_speed = if q3 { move_speed } else { self.tuning.up_speed };
        up = add(up, round(up_speed * jump));
        up = add(up, -round(up_speed * crouch));
        if !klook || q3 {
            let forward_speed = if q3 { move_speed } else { self.tuning.forward_speed };
            let back_speed = if q3 { move_speed } else { self.tuning.back_speed };
            forward = add(forward, round(forward_speed * fraction(named(InputAction::Forward))));
            forward = add(forward, -round(back_speed * fraction(named(InputAction::Back))));
        }
        if !q3 && running {
            forward *= self.tuning.move_speed_multiplier;
            side *= self.tuning.move_speed_multiplier;
            up *= self.tuning.move_speed_multiplier;
        }
        let sensitivity = match frame {
            FrameContext::Q3 { sensitivity, .. } => *sensitivity,
            _ => 1.0,
        };
        let mouse = self.mouse.sample(
            sample.mouse,
            frame_ms,
            strafe,
            active(SourceAction::MouseLook),
            sensitivity,
            q3,
        )?;
        forward = add(forward, mouse.forward);
        side = add(side, mouse.side);
        yaw = round(yaw + mouse.yaw - f64::from(sample.gamepad_look_degrees.x));
        pitch = round(pitch + mouse.pitch + f64::from(sample.gamepad_look_degrees.y));
        let pad_forward = if q3 {
            move_speed
        } else {
            self.tuning.forward_speed
                * if running {
                    self.tuning.move_speed_multiplier
                } else {
                    1.0
                }
        };
        let pad_side = if q3 {
            move_speed
        } else {
            self.tuning.side_speed
                * if running {
                    self.tuning.move_speed_multiplier
                } else {
                    1.0
                }
        };
        forward = add(forward, f64::from(sample.gamepad_move.y) * pad_forward);
        side = add(side, f64::from(sample.gamepad_move.x) * pad_side);
        let drift = match frame {
            FrameContext::Q1Netquake { pitch_drift, .. } | FrameContext::Q1Quakeworld { pitch_drift } => *pitch_drift,
            _ => None,
        };
        if q1 {
            if let Some(state) = drift {
                let mouse_look = active(SourceAction::MouseLook);
                let manual = mouse_look
                    || self.mouse.tuning.free_look
                    || klook && (active(named(InputAction::Forward)) || active(named(InputAction::Back)))
                    || fraction(SourceAction::LookUp) != 0.0
                    || fraction(SourceAction::LookDown) != 0.0
                    || sample.gamepad_look_degrees.y != 0.0;
                let threshold = if matches!(frame, FrameContext::Q1Quakeworld { .. }) {
                    200.0
                } else {
                    self.tuning.forward_speed
                };
                pitch = self.pitch_drift.sample(
                    pitch,
                    frame_ms,
                    &state,
                    manual,
                    !mouse_look
                        && (self.previous_mlook || pressed(SourceAction::MouseLook))
                        && self.mouse.tuning.look_spring,
                    forward,
                    threshold,
                    self.drift_speed,
                    self.drift_delay,
                );
            }
        }
        self.previous_mlook = active(SourceAction::MouseLook);
        let mut buttons = 0;
        let any = sample.any_key_down != 0;
        if q3 {
            for index in 0..15u8 {
                if pressed(SourceAction::Button(index)) {
                    buttons |= 1 << index;
                }
            }
            if pressed(named(InputAction::Attack)) {
                buttons |= 1;
            }
            if pressed(named(InputAction::Use)) {
                buttons |= 4;
            }
            if !running {
                buttons |= 16;
            }
            if !sample.focus_is_game {
                buttons |= 2;
            } else if any {
                buttons |= 2048;
            }
            pitch = pitch.clamp(previous_pitch - 90.0, previous_pitch + 90.0);
        } else {
            let attack_allowed = match frame {
                FrameContext::Q2Classic { attack_allowed, .. } | FrameContext::Q2Rerelease { attack_allowed, .. } => {
                    *attack_allowed
                }
                _ => true,
            };
            if pressed(named(InputAction::Attack)) && attack_allowed {
                buttons |= 1;
            }
            if q1 && pressed(named(InputAction::Jump)) || !q1 && pressed(named(InputAction::Use)) {
                buttons |= 2;
            }
            if !q1 && any && sample.focus_is_game {
                buttons |= 128;
            }
            if matches!(frame, FrameContext::Q2Rerelease { .. }) {
                if pressed(SourceAction::Holster) {
                    buttons |= 4;
                }
                if pressed(named(InputAction::Jump)) || pressed(named(InputAction::MoveUp)) {
                    buttons |= 8;
                }
                if pressed(named(InputAction::Crouch)) || pressed(named(InputAction::MoveDown)) {
                    buttons |= 16;
                }
            }
        }
        if q1 {
            pitch = pitch.clamp(-70.0, 80.0);
        }
        if let FrameContext::Q2Classic { delta_angles, .. } | FrameContext::Q2Rerelease { delta_angles, .. } = frame {
            let mut delta = f64::from(delta_angles.x);
            if delta > 180.0 {
                delta -= 360.0;
            }
            if pitch + delta < -360.0 {
                pitch += 360.0;
            }
            if pitch + delta > 360.0 {
                pitch -= 360.0;
            }
            pitch = pitch.clamp(-89.0 - delta, 89.0 - delta);
            forward = forward.clamp(-400.0, 400.0);
            side = side.clamp(-400.0, 400.0);
        }
        self.angles = [pitch, yaw, roll];
        let msec = (if frame_ms > 250.0 { 100.0 } else { frame_ms }) as i32;
        let angles = self.view_angles();
        match frame {
            FrameContext::Q1Netquake { ack_time_s, .. } => Ok(BuiltCommand::Q1Netquake {
                ack_time_s: *ack_time_s,
                view_angles: angles,
                forward: forward as i32,
                side: side as i32,
                up: up as i32,
                buttons,
                impulse: sample.impulse,
            }),
            FrameContext::Q1Quakeworld { .. } => Ok(BuiltCommand::Q1Quakeworld {
                msec,
                angles,
                forward: forward as i32,
                side: side as i32,
                up: up as i32,
                buttons,
                impulse: sample.impulse,
            }),
            FrameContext::Q2Classic { light_level, .. } => Ok(BuiltCommand::Q2Classic {
                msec,
                angle_shorts: [angle_word(pitch), angle_word(yaw), angle_word(roll)],
                forward: forward as i32,
                side: side as i32,
                up: up as i32,
                buttons,
                impulse: sample.impulse,
                light_level: *light_level,
            }),
            FrameContext::Q2Rerelease { server_frame, .. } => Ok(BuiltCommand::Q2Rerelease {
                msec,
                angles,
                forward,
                side,
                buttons,
                server_frame: *server_frame,
            }),
            FrameContext::Q3 {
                server_time_ms, weapon, ..
            } => Ok(BuiltCommand::Q3 {
                server_time_ms: *server_time_ms,
                angle_words: [angle_word(pitch), angle_word(yaw), angle_word(roll)],
                forward: forward.clamp(-127.0, 127.0) as i32,
                right: side.clamp(-127.0, 127.0) as i32,
                up: up.clamp(-127.0, 127.0) as i32,
                buttons,
                weapon: *weapon,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec2;

    fn held(action: SourceAction, fraction: f32) -> ButtonSample {
        ButtonSample {
            action,
            fraction,
            active: true,
            pressed: true,
        }
    }

    fn sample(buttons: Vec<ButtonSample>) -> SeatSample {
        SeatSample {
            buttons,
            mouse: vec2(0.0, 0.0),
            gamepad_move: vec2(0.0, 0.0),
            gamepad_look_degrees: vec2(0.0, 0.0),
            impulse: 0,
            any_key_down: 1,
            focus_is_game: true,
            frame_ms: 16.0,
        }
    }

    #[test]
    fn buttons_sample_per_family_timing() {
        let mut button = InputButton::new();
        button.down("key:32", 100);
        button.up("key:32", 116);
        assert!((button.sample(ButtonTiming::Q1, 116, 16.0) - 0.25).abs() < f32::EPSILON);
        button.down("key:32", 200);
        assert!((button.sample(ButtonTiming::Q2, 208, 16.0) - 0.5).abs() < 1e-6);
        button.release(208);
        button.down("key:32", 300);
        assert!((button.sample(ButtonTiming::Q3, 308, 16.0) - 0.5).abs() < 1e-6);
        button.release(400);
        assert_eq!(button.sample(ButtonTiming::Q1, 400, 16.0), 0.0);
        button.down("a", 500);
        button.down("b", 500);
        button.up("a", 510);
        assert!(button.active());
        button.release(520);
        assert!(!button.active());
    }

    #[test]
    fn mouse_filters_scales_and_strafes() {
        let mut mouse = MouseInput::new();
        let plain = mouse.sample(vec2(10.0, 0.0), 16.0, false, false, 1.0, false).unwrap();
        assert!((plain.yaw + 0.022 * 30.0).abs() < 1e-9);
        mouse.tuning.filter = true;
        mouse.clear();
        let _ = mouse.sample(vec2(10.0, 0.0), 16.0, false, false, 1.0, false).unwrap();
        let filtered = mouse.sample(vec2(10.0, 0.0), 16.0, false, false, 1.0, false).unwrap();
        assert!((filtered.yaw + 0.022 * 30.0).abs() < 1e-9);
        let strafe = mouse.sample(vec2(10.0, 5.0), 16.0, true, false, 1.0, false).unwrap();
        assert_eq!(strafe.yaw, 0.0);
        assert!(strafe.side > 0.0);
        assert!(strafe.forward < 0.0);
        assert!(mouse.sample(vec2(0.0, 0.0), 0.0, false, false, 1.0, false).is_err());
    }

    #[test]
    fn impulses_parse_per_family() {
        assert_eq!(parse_impulse("12", ClientFamily::Q1Netquake), 12.0);
        assert_eq!(parse_impulse("0x10", ClientFamily::Q1Quakeworld), 16.0);
        assert_eq!(parse_impulse("'A", ClientFamily::Q1Quakeworld), 65.0);
        assert_eq!(parse_impulse("300", ClientFamily::Q1Netquake), 44.0);
        assert_eq!(parse_impulse("12x", ClientFamily::Q2Classic), 12.0);
        assert_eq!(parse_impulse("abc", ClientFamily::Q2Classic), 0.0);
        assert_eq!(parse_impulse("2.5", ClientFamily::Q3), 2.5);
        assert_eq!(parse_impulse("", ClientFamily::Q2Rerelease), 0.0);
        assert_eq!(seat_impulse(12.0).unwrap(), 12);
        assert!(seat_impulse(256.0).is_err());
        assert!(seat_impulse(f64::NAN).is_err());
    }

    #[test]
    fn q1_command_encodes_moves_and_buttons() {
        let mut builder = InputCommandBuilder::new(ClientFamily::Q1Quakeworld);
        let attack = held(SourceAction::Action(InputAction::Attack), 1.0);
        let forward = held(SourceAction::Action(InputAction::Forward), 1.0);
        let command = builder
            .build(
                &sample(vec![attack, forward]),
                &FrameContext::Q1Quakeworld { pitch_drift: None },
            )
            .unwrap();
        match command {
            BuiltCommand::Q1Quakeworld {
                forward, buttons, msec, ..
            } => {
                assert_eq!((forward, buttons, msec), (200, 1, 16));
            }
            _ => panic!("wrong family"),
        }
        assert!(builder
            .build(
                &sample(vec![]),
                &FrameContext::Q3 {
                    server_time_ms: 0,
                    weapon: 0,
                    sensitivity: 1.0,
                }
            )
            .is_err());
    }

    #[test]
    fn q2_classic_quantizes_angles_and_gates_attack() {
        let mut builder = InputCommandBuilder::new(ClientFamily::Q2Classic);
        let right = held(SourceAction::TurnRight, 1.0);
        let attack = held(SourceAction::Action(InputAction::Attack), 1.0);
        let frame = FrameContext::Q2Classic {
            delta_angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            light_level: 60,
            attack_allowed: false,
        };
        let command = builder.build(&sample(vec![right, attack]), &frame).unwrap();
        match command {
            BuiltCommand::Q2Classic {
                angle_shorts,
                buttons,
                light_level,
                ..
            } => {
                assert_eq!(angle_shorts[1], angle_word(builder.angles[1]));
                assert_eq!(buttons, 128);
                assert_eq!(light_level, 60);
            }
            _ => panic!("wrong family"),
        }
    }

    #[test]
    fn q3_command_packs_buttons_and_clamps() {
        let mut builder = InputCommandBuilder::new(ClientFamily::Q3);
        let buttons = vec![
            held(SourceAction::Action(InputAction::Attack), 1.0),
            held(SourceAction::Button(5), 1.0),
            held(SourceAction::Action(InputAction::Forward), 1.0),
        ];
        let command = builder
            .build(
                &sample(buttons),
                &FrameContext::Q3 {
                    server_time_ms: 900,
                    weapon: 4,
                    sensitivity: 1.0,
                },
            )
            .unwrap();
        match command {
            BuiltCommand::Q3 {
                buttons,
                forward,
                weapon,
                angle_words,
                ..
            } => {
                assert_eq!(buttons & 1, 1);
                assert_eq!(buttons & (1 << 5), 1 << 5);
                assert_eq!(forward, 127);
                assert_eq!(weapon, 4);
                assert_eq!(angle_words.len(), 3);
            }
            _ => panic!("wrong family"),
        }
    }

    #[test]
    fn seat_tracks_keys_bindings_and_impulse() {
        let mut seat = SeatKeys::new();
        let key = PhysicalInput::Key(KeyCode::Space as i32);
        seat.bindings.bind(InputBinding {
            input: key.clone(),
            target: InputBindingTarget::Action(InputAction::Jump),
        });
        assert!(matches!(
            seat.bindings.binding(&key),
            Some(InputBindingTarget::Action(InputAction::Jump))
        ));
        seat.press(&key);
        assert!(seat.is_down(&key));
        assert!(seat.has_held());
        seat.release(&key);
        assert!(!seat.has_held());
        seat.set_impulse(7);
        assert_eq!(seat.impulse(), 7);
        assert_eq!(physical_input_key(&PhysicalInput::MouseButton(1)), "mouse:1");
        assert_eq!(quake_mouse_button(2), 3);
        assert_eq!(physical_mouse_button(3), 2);
    }
}
