//! One device binding table and usercmd builder, independent of map/game rules.
use qa_core::{
    primitives::{MovementRules, UserCmd, Vec3, buttons},
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent},
};

pub mod bindings;
pub mod keys;
pub use bindings::{BindError, Binding};
use qa_core::text::FixedText;
use std::fmt::Write;

const CONTROLS: usize = 1024;
const ACTIONS: usize = 30;
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
            }
        }
    }
    fn sample(&mut self, time: EventTime, period: u64) -> (f32, bool) {
        if self.active() {
            self.elapsed = self.elapsed.saturating_add(time.since(self.down_at));
            self.down_at = time;
        }
        let fraction = if period == 0 {
            0.0
        } else {
            (self.elapsed as f32 / period as f32).clamp(0.0, 1.0)
        };
        let pressed = self.pressed || self.active();
        self.elapsed = 0;
        self.pressed = false;
        (fraction, pressed)
    }
}
#[derive(Clone, Copy, Default)]
struct Seat {
    actions: [Button; ACTIONS],
    angles: Vec3,
    mouse: [i64; 2],
    mouse_previous: [f32; 2],
}

/// Cached command tuning; map geometry and module formats never select it.
#[derive(Clone, Copy, Debug)]
pub struct InputPolicy {
    pub rules: Option<MovementRules>,
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
}
impl InputPolicy {
    pub fn native(rules: MovementRules) -> Self {
        let q1 = matches!(rules, MovementRules::Quake | MovementRules::QuakeWorld);
        Self {
            rules: Some(rules),
            speed: [200.0, if q1 { 350.0 } else { 200.0 }, 200.0],
            back_speed: 200.0,
            angle_speed: [
                140.0,
                if rules == MovementRules::Quake3 {
                    140.0
                } else {
                    150.0
                },
            ],
            angle_multiplier: 1.5,
            move_multiplier: 2.0,
            always_run: matches!(
                rules,
                MovementRules::Quake2Rerelease | MovementRules::Quake3
            ),
            sensitivity: 3.0,
            acceleration: 0.0,
            mouse_scale: [0.022; 2],
            mouse_side: 0.8,
            mouse_forward: 1.0,
            filter: false,
            freelook: true,
            look_strafe: false,
        }
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
/// One stateless intent conversion for local, remote-module and bot callers.
pub struct UserCmdBuilder;
impl UserCmdBuilder {
    pub fn build(duration: std::time::Duration, time: EventTime, intent: CommandIntent) -> UserCmd {
        UserCmd {
            duration_ms: duration.as_millis().min(u128::from(u16::MAX)) as u16,
            duration_ns: duration.as_nanos().min(u128::from(u64::MAX)) as u64,
            server_time_ms: time.milliseconds() as i32,
            view_angles: intent.view_angles,
            movement: intent.movement,
            buttons: intent.buttons,
            impulse: intent.impulse,
            weapon: intent.weapon,
            light_level: intent.light_level,
        }
    }
}

pub struct Input {
    bindings: Box<[Option<Binding>]>,
    devices: [Device; 16],
    seats: [Seat; SeatId::COUNT],
    previous: Option<EventTime>,
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
        std::array::from_fn(|index| {
            let policy = policies[index];
            let seat = &mut self.seats[index];
            let sampled = seat
                .actions
                .each_mut()
                .map(|button| button.sample(time, period));
            let strafe = sampled[Action::Strafe as usize].1;
            let look = sampled[Action::MouseLook as usize].1;
            let speed_key = sampled[Action::Walk as usize].1;
            let running = speed_key != policy.always_run;
            let q3 = policy.rules == Some(MovementRules::Quake3);
            let q1 = matches!(
                policy.rules,
                Some(MovementRules::Quake | MovementRules::QuakeWorld)
            );
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
            let seconds = period as f32 * 1e-9;
            let turn = sampled[Action::TurnLeft as usize].0 - sampled[Action::TurnRight as usize].0;
            let angle_scale = if speed_key {
                policy.angle_multiplier
            } else {
                1.0
            };
            if !strafe {
                seat.angles.0[1] += turn * policy.angle_speed[0] * seconds * angle_scale;
            }
            seat.angles.0[0] += (sampled[Action::LookDown as usize].0
                - sampled[Action::LookUp as usize].0)
                * policy.angle_speed[1]
                * seconds
                * angle_scale;
            seat.mouse = [0; 2];
            let mut movement = [
                sampled[0].0 - sampled[1].0,
                sampled[3].0 - sampled[2].0,
                sampled[4].0 - sampled[5].0,
            ];
            if strafe {
                movement[1] -= turn;
            }
            if sampled[Action::KeyboardLook as usize].1 && !q3 {
                seat.angles.0[0] -= movement[0] * policy.angle_speed[1] * seconds * angle_scale;
                movement[0] = 0.0;
            }
            // CL_AdjustAngles precedes the platform mouse contribution.
            if q1 {
                seat.angles.0[1] = qa_core::math::anglemod(seat.angles.0[1]);
                seat.angles.0[0] = seat.angles.0[0].clamp(-70.0, 80.0);
                seat.angles.0[2] = seat.angles.0[2].clamp(-50.0, 50.0);
            }
            if !horizontal_strafe {
                seat.angles.0[1] -= mouse[0] * policy.mouse_scale[0];
            }
            if mouse_pitch {
                seat.angles.0[0] += mouse[1] * policy.mouse_scale[1];
            }
            if policy.rules.is_some() {
                seat.angles.0[0] = if q1 {
                    seat.angles.0[0].clamp(-70.0, 80.0)
                } else {
                    seat.angles.0[0].clamp(-89.0, 89.0)
                };
            }
            if policy.rules.is_some() && !q1 {
                movement[2] = sampled[Action::Up as usize]
                    .0
                    .max(sampled[Action::Jump as usize].0)
                    - sampled[Action::Down as usize]
                        .0
                        .max(sampled[Action::Crouch as usize].0);
            }
            for device in &self.devices {
                if device.seat.is_some_and(|seat| seat.index() == index) {
                    movement[0] -= f32::from(device.axes[1]) / 32768.0;
                    movement[1] += f32::from(device.axes[0]) / 32768.0;
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
            ] {
                if sampled[action as usize].1 {
                    mask |= bit;
                }
            }
            if q3 {
                if running {
                    mask &= !buttons::WALK;
                } else {
                    mask |= buttons::WALK;
                }
            }
            if self.devices.iter().any(|device| {
                device.seat.is_some_and(|seat| seat.index() == index)
                    && device.held.iter().any(|held| *held)
            }) {
                mask |= buttons::ANY;
            }
            let intent = CommandIntent {
                movement: std::array::from_fn(|axis| {
                    let native_speed = if axis == 0 && movement[axis] < 0.0 {
                        policy.back_speed
                    } else {
                        policy.speed[axis]
                    };
                    let key_speed = if q3 {
                        if running { 127.0 } else { 64.0 }
                    } else {
                        native_speed * if running { policy.move_multiplier } else { 1.0 }
                    };
                    let extra = if axis == 0 && !mouse_pitch {
                        -mouse[1] * policy.mouse_forward
                    } else if axis == 1 && horizontal_strafe {
                        mouse[0] * policy.mouse_side
                    } else {
                        0.0
                    };
                    let limit = if q3 { 127.0 } else { f32::from(i16::MAX) };
                    (movement[axis].clamp(-1.0, 1.0) * key_speed + extra).clamp(-limit, limit)
                        as i16
                }),
                view_angles: seat.angles,
                buttons: mask,
                ..CommandIntent::default()
            };
            UserCmdBuilder::build(std::time::Duration::from_nanos(period), time, intent)
        })
    }
}
