//! One device binding table and usercmd builder, independent of map/game rules.
use qa_core::{
    primitives::{RuleSetId, UserCmd, Vec3, buttons},
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent},
};

pub mod bindings;
mod command;
pub mod keys;
mod view;
pub use bindings::{BindError, Binding};
pub use command::UserCmdBuilder;
use qa_core::text::FixedText;
use std::fmt::Write;

const CONTROLS: usize = 1024;
const ACTIONS: usize = 31;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Up,
    Down,
    Attack,
    Jump,
    Use,
    Crouch,
    Walk,
    TurnLeft,
    TurnRight,
    LookUp,
    LookDown,
    Strafe,
    MouseLook,
    KeyboardLook,
    Talk,
    Gesture,
    Affirmative,
    Negative,
    GetFlag,
    GuardBase,
    Patrol,
    FollowMe,
    Any,
    Extra12,
    Extra13,
    Extra14,
    Holster,
}
pub trait Target {
    fn key(&mut self, _seat: SeatId, _control: u16, _down: bool, _repeat: bool) -> bool {
        false
    }
    fn character(&mut self, seat: SeatId, value: char);
    fn command(&mut self, seat: SeatId, time: EventTime, text: &str);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeySource {
    device: DeviceId,
    control: u16,
}
#[derive(Clone, Copy, Default)]
struct Button {
    held: [Option<KeySource>; 2],
    down_at: EventTime,
    elapsed: u64,
    pressed: bool,
    released: bool,
}
impl Button {
    fn active(&self) -> bool {
        self.held.iter().any(Option::is_some)
    }
    fn set(&mut self, source: KeySource, down: bool, time: EventTime) {
        if down {
            // qsrc Q3 IN_KeyDown: repeated sources do not acquire another slot
            // or change downtime. Two keys may hold the same action.
            if self.held.contains(&Some(source)) {
                return;
            }
            let active = self.active();
            let Some(slot) = self.held.iter_mut().find(|slot| slot.is_none()) else {
                return;
            };
            *slot = Some(source);
            if !active {
                self.down_at = time;
                self.pressed = true;
            }
        } else {
            let Some(slot) = self.held.iter_mut().find(|slot| **slot == Some(source)) else {
                return;
            };
            *slot = None;
            if !self.active() {
                self.elapsed = self.elapsed.saturating_add(time.since(self.down_at));
                self.released = true;
            }
        }
    }
    fn sample(&mut self, time: EventTime, period: u64, q1: bool) -> (f32, bool) {
        if self.active() {
            self.elapsed = self.elapsed.saturating_add(time.since(self.down_at));
            self.down_at = time;
        }
        let fraction = if q1 {
            // NQ/QW CL_KeyState uses transition bits, not elapsed milliseconds.
            match (self.pressed, self.released, self.active()) {
                (true, true, true) => 0.75,
                (true, true, false) => 0.25,
                (true, false, true) => 0.5,
                (false, false, true) => 1.0,
                _ => 0.0,
            }
        } else if period == 0 {
            0.0
        } else {
            (self.elapsed as f32 / period as f32).clamp(0.0, 1.0)
        };
        let pressed = self.pressed || self.active();
        self.elapsed = 0;
        self.pressed = false;
        self.released = false;
        (fraction, pressed)
    }
}
#[derive(Clone, Copy, Default)]
struct Seat {
    actions: [Button; ACTIONS],
    angles: Vec3,
    mouse: [i64; 2],
    mouse_previous: [f32; 2],
    center: bool,
    drift: view::PitchDrift,
}

/// Cached command tuning; map geometry and module formats never select it.
#[derive(Clone, Copy, Debug)]
pub struct InputPolicy {
    pub rules: Option<RuleSetId>,
    pub speed: [f32; 3],
    pub back_speed: f32,
    pub angle_speed: [f32; 2],
    pub angle_multiplier: f32,
    pub move_multiplier: f32,
    pub always_run: bool,
    pub sensitivity: f32,
    pub acceleration: f32,
    pub mouse_scale: [f32; 2],
    pub mouse_side: f32,
    pub mouse_forward: f32,
    pub filter: bool,
    pub freelook: bool,
    pub look_strafe: bool,
    /// Q2 CL_ClampPitch subtracts the authoritative player's delta angle.
    pub delta_pitch: f32,
    pub lookspring: bool,
    pub center_speed: f32,
    pub center_delay: f32,
}

#[derive(Clone, Copy)]
pub(crate) enum Accumulation {
    Float,
    Short,
    Int,
}
impl Accumulation {
    pub(crate) fn narrow(self, value: f32) -> f32 {
        match self {
            Self::Float => value,
            Self::Short => (value as i32 as i16) as f32,
            Self::Int => (value as i32) as f32,
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct CommandRules {
    speed: [f32; 3],
    pitch_speed: f32,
    always_run: bool,
    mouse_side: f32,
    mouse_forward: f32,
    key_scale: Option<[f32; 2]>,
    key_limit: Option<[f32; 2]>,
    accumulation: Accumulation,
    vertical_actions: bool,
    pitch_drift: bool,
}
const Q1_INPUT: CommandRules = CommandRules {
    speed: [200.0, 350.0, 200.0],
    pitch_speed: 150.0,
    always_run: false,
    mouse_side: 0.8,
    mouse_forward: 1.0,
    key_scale: None,
    key_limit: None,
    accumulation: Accumulation::Float,
    vertical_actions: false,
    pitch_drift: true,
};
const Q2_INPUT: CommandRules = CommandRules {
    speed: [200.0; 3],
    vertical_actions: true,
    pitch_drift: false,
    ..Q1_INPUT
};
const INPUT_RULES: [CommandRules; 5] = [
    Q1_INPUT,
    CommandRules {
        accumulation: Accumulation::Short,
        ..Q1_INPUT
    },
    CommandRules {
        accumulation: Accumulation::Short,
        ..Q2_INPUT
    },
    CommandRules {
        always_run: true,
        ..Q2_INPUT
    },
    CommandRules {
        pitch_speed: 140.0,
        always_run: true,
        mouse_side: 0.25,
        mouse_forward: 0.25,
        key_scale: Some([64.0, 127.0]),
        key_limit: Some([-128.0, 127.0]),
        accumulation: Accumulation::Int,
        ..Q2_INPUT
    },
];
impl InputPolicy {
    pub fn native(rules: RuleSetId) -> Self {
        let data = INPUT_RULES[rules as usize];
        Self {
            rules: Some(rules),
            speed: data.speed,
            back_speed: 200.0,
            angle_speed: [140.0, data.pitch_speed],
            angle_multiplier: 1.5,
            move_multiplier: 2.0,
            always_run: data.always_run,
            sensitivity: 3.0,
            acceleration: 0.0,
            mouse_scale: [0.022; 2],
            mouse_side: data.mouse_side,
            mouse_forward: data.mouse_forward,
            filter: false,
            freelook: true,
            look_strafe: false,
            delta_pitch: 0.0,
            lookspring: false,
            center_speed: 500.0,
            center_delay: 0.15,
        }
    }
    pub(crate) fn command_rules(self) -> CommandRules {
        self.rules
            .map_or(Q1_INPUT, |rules| INPUT_RULES[rules as usize])
    }
}
#[derive(Clone, Copy)]
struct Device {
    id: Option<DeviceId>,
    seat: Option<SeatId>,
    held: [bool; CONTROLS],
    acquired: [bool; CONTROLS],
    axes: [i16; 6],
}
impl Default for Device {
    fn default() -> Self {
        Self {
            id: None,
            seat: None,
            held: [false; CONTROLS],
            acquired: [false; CONTROLS],
            axes: [0; 6],
        }
    }
}

pub use qa_core::primitives::CommandIntent;

pub struct Input {
    bindings: Box<[Option<Binding>]>,
    devices: [Device; 16],
    seats: [Seat; SeatId::COUNT],
    previous: Option<EventTime>,
    frame_ns: u64,
    freelook: [bool; SeatId::COUNT],
}
impl Default for Input {
    fn default() -> Self {
        Self::load()
    }
}
impl Input {
    pub fn load() -> Self {
        let mut input = Self {
            bindings: (0..CONTROLS).map(|_| None).collect(),
            devices: [Device::default(); 16],
            seats: [Seat::default(); SeatId::COUNT],
            previous: None,
            frame_ns: 0,
            freelook: [true; SeatId::COUNT],
        };
        input.assign(DeviceId::Keyboard, SeatId::FIRST);
        input.assign(DeviceId::Mouse(0), SeatId::FIRST);
        for (control, action) in [
            (26, Action::Forward),
            (22, Action::Back),
            (4, Action::Left),
            (7, Action::Right),
            (44, Action::Jump),
            (224, Action::Crouch),
            (513, Action::Attack),
            (544, Action::Jump),
        ] {
            input.bindings[control] = Some(Binding::for_action(action));
        }
        input
    }
    /// Assignment is a load/menu operation; release held controls first.
    pub fn assign(&mut self, id: DeviceId, seat: SeatId) -> bool {
        if let Some(index) = self.devices.iter().position(|device| device.id == Some(id)) {
            if self.devices[index].held.iter().any(|held| *held) {
                return false;
            }
            self.remove(id, self.previous.unwrap_or_default());
            self.devices[index] = Device {
                id: Some(id),
                seat: Some(seat),
                ..Device::default()
            };
            return true;
        }
        let Some(device) = self.devices.iter_mut().find(|device| device.id.is_none()) else {
            return false;
        };
        *device = Device {
            id: Some(id),
            seat: Some(seat),
            ..Device::default()
        };
        true
    }
    /// The first clock event seeds input timing after window/startup work.
    pub fn seed(&mut self, time: EventTime) {
        self.previous.get_or_insert(time);
    }
    /// Load/spawn fixangle seeds the same angles later emitted by usercmds.
    /// The caller converts and validates external angles at the map/module ABI.
    pub fn set_view_angles(&mut self, seat: SeatId, angles: Vec3) {
        self.seats[seat.index()].angles = angles;
        self.seats[seat.index()].mouse = [0; 2];
        self.seats[seat.index()].mouse_previous = [0.0; 2];
    }
    /// CLIENT resolves this request against its current native view policy.
    pub fn center_view(&mut self, seat: SeatId) {
        self.seats[seat.index()].center = true;
    }
    /// NQ/QW render-view drift runs after command construction and prediction.
    /// The caller supplies ground and ideal-pitch state independently of map.
    pub fn drift_view(
        &mut self,
        seat: SeatId,
        time: EventTime,
        policy: InputPolicy,
        player: &qa_core::primitives::PlayerState,
        forward: f32,
    ) -> Vec3 {
        let state = &mut self.seats[seat.index()];
        if policy.rules.is_some() && policy.command_rules().pitch_drift {
            state.angles.0[0] = state.drift.advance(
                time,
                EventTime(self.frame_ns).seconds(),
                state.angles.0[0],
                view::DriftInput {
                    grounded: player.movement.grounded,
                    disabled: policy.rules != Some(RuleSetId::QuakeWorld)
                        && matches!(
                            player.movement.mode,
                            qa_core::primitives::MovementMode::Noclip
                        ),
                    forward,
                    threshold: if policy.rules == Some(RuleSetId::QuakeWorld) {
                        200.0
                    } else {
                        policy.speed[0]
                    },
                    ideal_pitch: if policy.rules == Some(RuleSetId::QuakeWorld) {
                        0.0
                    } else {
                        player.ideal_pitch
                    },
                    speed: policy.center_speed,
                    delay: policy.center_delay,
                },
            );
        }
        state.angles
    }
    pub fn binding(&self, control: u16) -> Option<&Binding> {
        self.bindings
            .get(keys::normalize(control) as usize)
            .and_then(Option::as_ref)
    }
    pub fn bindings(&self) -> impl Iterator<Item = (u16, &Binding)> {
        self.bindings
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.as_ref().map(|b| (i as u16, b)))
    }
    pub fn bind_text(
        &mut self,
        control: u16,
        text: &str,
        time: EventTime,
        target: &mut impl Target,
    ) -> Result<(), BindError> {
        if self
            .binding(control)
            .is_some_and(|binding| binding.text() == text)
        {
            return Ok(());
        }
        let binding = if text.is_empty() {
            None
        } else {
            Some(Binding::parse(text)?)
        };
        if self.bind(control, binding, time, target) {
            Ok(())
        } else {
            Err(BindError::Control)
        }
    }
    pub fn bind(
        &mut self,
        control: u16,
        binding: Option<Binding>,
        time: EventTime,
        target: &mut impl Target,
    ) -> bool {
        let control = keys::normalize(control);
        if control as usize >= CONTROLS {
            return false;
        }
        // Release the acquired old binding before replacement, while retaining
        // physical hold state. Repeats cannot acquire the new binding until up.
        for index in 0..self.devices.len() {
            // Only modifiers have two physical controls for one config name.
            for physical in [
                Some(control),
                (224..=227).contains(&control).then_some(control + 4),
            ]
            .into_iter()
            .flatten()
            {
                if self.devices[index].held[usize::from(physical)]
                    && let Some(id) = self.devices[index].id
                {
                    self.key(id, physical, false, false, time, target);
                    self.devices[index].held[usize::from(physical)] = true;
                }
            }
        }
        self.bindings[control as usize] = binding;
        true
    }
    pub fn unbind_all(&mut self, time: EventTime, target: &mut impl Target) {
        for control in 0..CONTROLS {
            if self.bindings[control].is_some() {
                self.bind(control as u16, None, time, target);
            }
        }
    }
    pub fn button(
        &mut self,
        seat: SeatId,
        action: Action,
        down: bool,
        key: Option<u16>,
        time: EventTime,
    ) {
        let button = &mut self.seats[seat.index()].actions[action as usize];
        if !down && key.is_none() {
            button.released |= button.active();
            button.held = [None; 2];
            // Keep an impulse pressed and accumulated elapsed time this frame.
        } else {
            button.set(
                KeySource {
                    device: DeviceId::Keyboard,
                    control: key.unwrap_or(u16::MAX),
                },
                down,
                time,
            );
        }
    }
    pub fn set_freelook(&mut self, seat: SeatId, enabled: bool) {
        self.freelook[seat.index()] = enabled;
    }
    fn remove(&mut self, id: DeviceId, time: EventTime) {
        for seat in &mut self.seats {
            for button in &mut seat.actions {
                let sources = button.held;
                for source in sources
                    .into_iter()
                    .flatten()
                    .filter(|source| source.device == id)
                {
                    button.set(source, false, time);
                }
            }
        }
        for device in &mut self.devices {
            if device.id == Some(id) {
                *device = Device::default();
            }
        }
    }
    pub fn dispatch(&mut self, event: SysEvent<'_>, target: &mut impl Target) {
        match event.kind {
            EventKind::Key {
                device,
                code,
                down,
                repeat,
                ..
            } => self.key(device, code, down, repeat, event.time, target),
            EventKind::MouseButton {
                device,
                button,
                down,
            } if button <= 32 => self.key(
                device,
                if button == 32 {
                    580
                } else {
                    512 + u16::from(button)
                },
                down,
                false,
                event.time,
                target,
            ),
            EventKind::ControllerButton {
                device,
                button,
                down,
            } if button < 32 => self.key(
                device,
                544 + u16::from(button),
                down,
                false,
                event.time,
                target,
            ),
            EventKind::MouseWheel { device, x, y } => {
                for (delta, positive, negative) in [(y, 576, 577), (x, 579, 578)] {
                    let control = if delta > 0 { positive } else { negative };
                    // Native wheel input is a momentary key, once per event
                    // and nonzero axis, including high-resolution devices.
                    if delta != 0 {
                        self.key(device, control, true, false, event.time, target);
                        self.key(device, control, false, false, event.time, target);
                    }
                }
            }
            EventKind::Mouse { device, dx, dy } => {
                if let Some(seat) = self.device_seat(device) {
                    let state = &mut self.seats[seat.index()];
                    state.mouse[0] = state.mouse[0].saturating_add(i64::from(dx));
                    state.mouse[1] = state.mouse[1].saturating_add(i64::from(dy));
                }
            }
            EventKind::ControllerAxis {
                device,
                axis,
                value,
            } => {
                if let Some(device) = self
                    .devices
                    .iter_mut()
                    .find(|entry| entry.id == Some(device))
                    && let Some(slot) = device.axes.get_mut(usize::from(axis))
                {
                    *slot = value;
                }
            }
            EventKind::Char { device, value } => {
                if let Some(seat) = self.device_seat(device) {
                    target.character(seat, value);
                }
            }
            EventKind::Time => self.seed(event.time),
            EventKind::Focus(false) => {
                for index in 0..self.devices.len() {
                    if let Some(id) = self.devices[index].id {
                        self.release_device(id, event.time, target);
                    }
                }
                for seat in &mut self.seats {
                    seat.actions.fill(Button::default());
                    seat.mouse = [0; 2];
                    seat.mouse_previous = [0.0; 2];
                }
                for device in &mut self.devices {
                    device.held.fill(false);
                    device.axes.fill(0);
                }
            }
            EventKind::DeviceRemoved(id) => {
                self.release_device(id, event.time, target);
                self.remove(id, event.time);
            }
            _ => {}
        }
    }
    fn device_seat(&self, id: DeviceId) -> Option<SeatId> {
        self.devices
            .iter()
            .find(|entry| entry.id == Some(id))
            .and_then(|device| device.seat)
    }
    pub fn release_device(&mut self, id: DeviceId, time: EventTime, target: &mut impl Target) {
        let Some(index) = self.devices.iter().position(|device| device.id == Some(id)) else {
            return;
        };
        for control in 0..CONTROLS {
            if self.devices[index].held[control] {
                self.key(id, control as u16, false, false, time, target);
            }
        }
    }
    fn key(
        &mut self,
        id: DeviceId,
        control: u16,
        down: bool,
        repeat: bool,
        time: EventTime,
        target: &mut impl Target,
    ) {
        let Some(device_index) = self.devices.iter().position(|entry| entry.id == Some(id)) else {
            return;
        };
        let device = &mut self.devices[device_index];
        let Some(seat) = device.seat else {
            return;
        };
        let Some(held) = device.held.get_mut(usize::from(control)) else {
            return;
        };
        let was_held = *held;
        *held = down;
        // Editing receives repeats. Held actions acquire/release only once.
        let consumed = target.key(seat, control, down, repeat);
        if was_held == down {
            return;
        }
        if down {
            device.acquired[usize::from(control)] = !consumed;
        } else if !std::mem::take(&mut device.acquired[usize::from(control)]) {
            return;
        }
        if let Some(binding) = &self.bindings[usize::from(keys::normalize(control))] {
            if down && consumed {
                return;
            }
            let mut button_seen = false;
            for part in &binding.parts[..binding.count] {
                button_seen |= part.button;
                let text = &binding.text.as_str()[part.start as usize..part.end as usize];
                if let Some(action) = part.action {
                    self.seats[seat.index()].actions[action as usize].set(
                        KeySource {
                            device: id,
                            control,
                        },
                        down,
                        time,
                    );
                } else if part.button {
                    let mut command = FixedText::<1088>::default();
                    if down {
                        let _ = command.write_str(text);
                    } else {
                        let _ = write!(command, "-{}", &text[1..]);
                    }
                    let _ = write!(
                        command,
                        " {} {}",
                        device_index * CONTROLS + control as usize,
                        time.milliseconds()
                    );
                    target.command(seat, time, command.as_str());
                } else if down || button_seen {
                    target.command(seat, time, text);
                }
            }
        }
    }
    pub fn build_frame(
        &mut self,
        time: EventTime,
        speed: [i16; 3],
        mouse_scale: [f32; 2],
    ) -> [UserCmd; SeatId::COUNT] {
        let policies = std::array::from_fn(|seat| InputPolicy {
            rules: None,
            speed: speed.map(f32::from),
            back_speed: f32::from(speed[0]),
            angle_speed: [140.0; 2],
            angle_multiplier: 1.0,
            move_multiplier: 1.0,
            always_run: false,
            sensitivity: 1.0,
            acceleration: 0.0,
            mouse_scale,
            mouse_side: 0.0,
            mouse_forward: 0.0,
            filter: false,
            freelook: self.freelook[seat],
            look_strafe: false,
            delta_pitch: 0.0,
            lookspring: false,
            center_speed: 500.0,
            center_delay: 0.15,
        });
        self.build_frame_with_policy(time, &policies)
    }
    pub fn build_frame_with_policy(
        &mut self,
        time: EventTime,
        policies: &[InputPolicy; SeatId::COUNT],
    ) -> [UserCmd; SeatId::COUNT] {
        let period = time.since(self.previous.unwrap_or(time));
        self.previous = Some(time);
        self.frame_ns = period;
        std::array::from_fn(|index| {
            let policy = policies[index];
            let q3 = policy.rules == Some(RuleSetId::Quake3);
            let q1 = matches!(policy.rules, Some(RuleSetId::Quake | RuleSetId::QuakeWorld));
            let seat = &mut self.seats[index];
            let look_released = seat.actions[Action::MouseLook as usize].released;
            let spring = look_released
                && if q1 {
                    policy.lookspring
                } else if q3 {
                    !policy.freelook
                } else {
                    !policy.freelook && policy.lookspring
                };
            if std::mem::take(&mut seat.center) || spring {
                if q1 {
                    seat.drift.start(time, policy.center_speed);
                } else {
                    seat.angles.0[0] = -policy.delta_pitch;
                }
            }
            // Modifiers use native held state; action buttons also retain taps.
            let held = seat.actions.each_ref().map(Button::active);
            let sampled = seat
                .actions
                .each_mut()
                .map(|button| button.sample(time, period, q1));
            let fraction = |action: Action| sampled[action as usize].0;
            let strafe = held[Action::Strafe as usize];
            let look = held[Action::MouseLook as usize];
            let klook = held[Action::KeyboardLook as usize] && !q3;
            let speed_key = held[Action::Walk as usize];
            let previous_pitch = seat.angles.0[0];
            let raw = seat.mouse.map(|value| value as f32);
            let mut mouse = if policy.filter {
                std::array::from_fn(|axis| (raw[axis] + seat.mouse_previous[axis]) * 0.5)
            } else {
                raw
            };
            seat.mouse_previous = raw;
            let rate = (mouse[0] * mouse[0] + mouse[1] * mouse[1]).sqrt()
                / (period as f32 * 1e-6).max(1.0);
            let gain = policy.sensitivity + rate * policy.acceleration;
            mouse.iter_mut().for_each(|value| *value *= gain);
            let horizontal_strafe = strafe || policy.look_strafe && look;
            let mouse_pitch = !strafe && (policy.freelook || look);
            if q1
                && (klook
                    || look
                    || fraction(Action::LookUp) != 0.0
                    || fraction(Action::LookDown) != 0.0
                    || mouse_pitch && mouse[1] != 0.0)
            {
                seat.drift.stop(time);
            }
            let seconds = period as f32 * 1e-9;
            let angle_scale = if speed_key {
                policy.angle_multiplier
            } else {
                1.0
            };
            let angle_speed = seconds * angle_scale;
            if !strafe {
                seat.angles.0[1] -=
                    angle_speed * policy.angle_speed[0] * fraction(Action::TurnRight);
                seat.angles.0[1] +=
                    angle_speed * policy.angle_speed[0] * fraction(Action::TurnLeft);
                if q1 {
                    seat.angles.0[1] = qa_core::math::anglemod(seat.angles.0[1]);
                }
            }
            if klook {
                seat.angles.0[0] -= angle_speed * policy.angle_speed[1] * fraction(Action::Forward);
                seat.angles.0[0] += angle_speed * policy.angle_speed[1] * fraction(Action::Back);
            }
            seat.angles.0[0] -= angle_speed * policy.angle_speed[1] * fraction(Action::LookUp);
            seat.angles.0[0] += angle_speed * policy.angle_speed[1] * fraction(Action::LookDown);
            seat.mouse = [0; 2];
            // CL_AdjustAngles precedes the platform mouse contribution.
            if q1 {
                seat.angles.0[0] = seat.angles.0[0].clamp(-70.0, 80.0);
                seat.angles.0[2] = seat.angles.0[2].clamp(-50.0, 50.0);
            }

            let mut intent = CommandIntent {
                movement: [
                    if klook {
                        [0.0; 2]
                    } else {
                        [fraction(Action::Forward), fraction(Action::Back)]
                    },
                    [fraction(Action::Right), fraction(Action::Left)],
                    [fraction(Action::Up), fraction(Action::Down)],
                ],
                vertical_actions: [fraction(Action::Jump), fraction(Action::Crouch)],
                strafe: if strafe {
                    [fraction(Action::TurnRight), fraction(Action::TurnLeft)]
                } else {
                    [0.0; 2]
                },
                speed_modifier: speed_key,
                ..CommandIntent::default()
            };
            if !horizontal_strafe {
                seat.angles.0[1] -= mouse[0] * policy.mouse_scale[0];
            } else {
                intent.mouse_movement[0] = mouse[0];
            }
            if mouse_pitch {
                seat.angles.0[0] += mouse[1] * policy.mouse_scale[1];
            } else {
                intent.mouse_movement[1] = mouse[1];
            }
            if q1 {
                seat.angles.0[0] = seat.angles.0[0].clamp(-70.0, 80.0);
            } else if q3 {
                seat.angles.0[0] =
                    seat.angles.0[0].clamp(previous_pitch - 90.0, previous_pitch + 90.0);
            } else if policy.rules.is_some() {
                let delta = if policy.delta_pitch > 180.0 {
                    policy.delta_pitch - 360.0
                } else {
                    policy.delta_pitch
                };
                seat.angles.0[0] = seat.angles.0[0].clamp(-89.0 - delta, 89.0 - delta);
            }
            for device in &self.devices {
                if device.seat.is_some_and(|seat| seat.index() == index) {
                    intent.axes[usize::from(intent.axis_count)] = [
                        -f32::from(device.axes[1]) / 32768.0,
                        f32::from(device.axes[0]) / 32768.0,
                    ];
                    intent.axis_count += 1;
                }
            }
            let mut mask = 0;
            for (action, bit) in [
                (Action::Attack, buttons::ATTACK),
                (Action::Jump, buttons::JUMP),
                (Action::Use, buttons::USE),
                (Action::Crouch, buttons::CROUCH),
                (Action::Walk, buttons::WALK),
                (Action::Talk, buttons::TALK),
                (Action::Gesture, buttons::GESTURE),
                (Action::Affirmative, buttons::AFFIRMATIVE),
                (Action::Negative, buttons::NEGATIVE),
                (Action::GetFlag, buttons::GETFLAG),
                (Action::GuardBase, buttons::GUARDBASE),
                (Action::Patrol, buttons::PATROL),
                (Action::FollowMe, buttons::FOLLOWME),
                (Action::Any, buttons::ANY),
                (Action::Extra12, buttons::EXTRA12),
                (Action::Extra13, buttons::EXTRA13),
                (Action::Extra14, buttons::EXTRA14),
                (Action::Holster, buttons::HOLSTER),
            ] {
                if sampled[action as usize].1 {
                    mask |= bit;
                }
            }
            if self.devices.iter().any(|device| {
                device.seat.is_some_and(|seat| seat.index() == index)
                    && device.held.iter().any(|held| *held)
            }) {
                mask |= buttons::ANY;
            }
            intent.view_angles = seat.angles;
            intent.buttons = mask;
            UserCmdBuilder::build(
                std::time::Duration::from_nanos(period),
                time,
                intent,
                policy,
            )
        })
    }
}
