//! One device binding table and usercmd builder, independent of map/game rules.
use qa_core::{
    primitives::{UserCmd, Vec3, buttons},
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent},
};

const CONTROLS: usize = 576;
const ACTIONS: usize = 11;
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
}
pub enum Binding {
    Action(Action),
    Command {
        press: String,
        release: Option<String>,
    },
}
pub trait Target {
    fn key(&mut self, _seat: SeatId, _control: u16, _down: bool, _repeat: bool) -> bool {
        false
    }
    fn character(&mut self, seat: SeatId, value: char);
    fn command(&mut self, seat: SeatId, text: &str);
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

/// Movement policy supplies units; the event service selects no movement game.
/// Bots submit the same intent to the same builder.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandIntent {
    pub movement: [i16; 3],
    pub view_angles: Vec3,
    pub buttons: u32,
    pub impulse: u8,
}
pub struct UserCmdBuilder {
    previous: [EventTime; SeatId::COUNT],
}
impl Default for UserCmdBuilder {
    fn default() -> Self {
        Self {
            previous: [EventTime::default(); SeatId::COUNT],
        }
    }
}
impl UserCmdBuilder {
    pub fn build(&mut self, seat: SeatId, time: EventTime, intent: CommandIntent) -> UserCmd {
        let duration = time.since(self.previous[seat.index()]);
        self.previous[seat.index()] = time;
        UserCmd {
            duration_ms: (duration / 1_000_000).min(u64::from(u16::MAX)) as u16,
            server_time_ms: time.milliseconds() as i32,
            view_angles: intent.view_angles,
            movement: intent.movement,
            buttons: intent.buttons,
            impulse: intent.impulse,
            ..UserCmd::default()
        }
    }
}

pub struct Input {
    bindings: Box<[Option<Binding>]>,
    devices: [Device; 16],
    seats: [Seat; SeatId::COUNT],
    previous: EventTime,
    builder: UserCmdBuilder,
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
            previous: EventTime::default(),
            builder: UserCmdBuilder::default(),
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
            input.bind(control, Some(Binding::Action(action)));
        }
        input
    }
    /// Assignment is a load/menu operation; release held controls first.
    pub fn assign(&mut self, id: DeviceId, seat: SeatId) -> bool {
        if let Some(index) = self.devices.iter().position(|device| device.id == Some(id)) {
            if self.devices[index].held.iter().any(|held| *held) {
                return false;
            }
            self.remove(id, self.previous);
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
    pub fn bind(&mut self, control: u16, binding: Option<Binding>) -> bool {
        // Release a live control before replacing its binding. Otherwise its
        // old held action could never receive the corresponding key-up.
        if self.devices.iter().any(|device| {
            device
                .held
                .get(usize::from(control))
                .copied()
                .unwrap_or(false)
        }) {
            return false;
        }
        let Some(slot) = self.bindings.get_mut(usize::from(control)) else {
            return false;
        };
        *slot = binding;
        true
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
            } if button < 32 => self.key(
                device,
                512 + u16::from(button),
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
        let Some(device) = self.devices.iter_mut().find(|entry| entry.id == Some(id)) else {
            return;
        };
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
        match &self.bindings[usize::from(control)] {
            Some(Binding::Action(action)) => {
                if !down || !consumed {
                    self.seats[seat.index()].actions[*action as usize].set(
                        KeySource {
                            device: id,
                            control,
                        },
                        down,
                        time,
                    );
                }
            }
            Some(Binding::Command { press, release }) => {
                if down {
                    if !consumed {
                        target.command(seat, press);
                    }
                } else if let Some(text) = release {
                    target.command(seat, text);
                }
            }
            None => {}
        }
    }
    pub fn build_frame(
        &mut self,
        time: EventTime,
        speed: [i16; 3],
        mouse_scale: [f32; 2],
        bots: [Option<CommandIntent>; SeatId::COUNT],
    ) -> [UserCmd; SeatId::COUNT] {
        let period = time.since(self.previous);
        self.previous = time;
        std::array::from_fn(|index| {
            let seat = &mut self.seats[index];
            let sampled = seat
                .actions
                .each_mut()
                .map(|button| button.sample(time, period));
            seat.angles.0[1] -= seat.mouse[0] as f32 * mouse_scale[0];
            seat.angles.0[0] += seat.mouse[1] as f32 * mouse_scale[1];
            seat.mouse = [0; 2];
            let mut movement = [
                sampled[0].0 - sampled[1].0,
                sampled[3].0 - sampled[2].0,
                sampled[4].0 - sampled[5].0,
            ];
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
            let intent = bots[index].unwrap_or(CommandIntent {
                movement: std::array::from_fn(|axis| {
                    (movement[axis].clamp(-1.0, 1.0) * f32::from(speed[axis])) as i16
                }),
                view_angles: seat.angles,
                buttons: mask,
                impulse: 0,
            });
            self.builder.build(SeatId::ALL[index], time, intent)
        })
    }
}
