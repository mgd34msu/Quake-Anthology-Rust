//! One device binding table and usercmd builder, independent of map/game rules.
use qa_core::{
    primitives::{
        CommandProducer, CommandUnits, ModuleId, RuleSetId, UserCmd, Vec3, WeaponId, buttons,
    },
    sys_events::{DeviceId, EventKind, EventTime, KeySource, MAX_INPUT_DEVICES, SeatId, SysEvent},
};

pub mod bindings;
mod command;
mod dispatch;
pub mod keys;
mod view;
pub use bindings::{BindError, Binding};
pub use command::UserCmdBuilder;
pub use dispatch::{DispatchCursor, DispatchResume, DispatchStep, InputAtom, InputCapture};

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
    /// True only when the sole command sink admits the complete line.
    fn command(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<KeySource>,
        producer: Option<CommandProducer>,
        text: &str,
    ) -> bool;
    fn command_bytes(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<KeySource>,
        producer: Option<CommandProducer>,
        text: &[u8],
    ) -> bool {
        std::str::from_utf8(text).is_ok_and(|text| self.command(seat, time, source, producer, text))
    }
    /// Cancel pending commands inherited from this physical source.
    fn cancel_commands(&mut self, source: KeySource, producer: Option<CommandProducer>);
}
#[derive(Clone, Copy, Default)]
struct Button {
    held: [Option<ActionSource>; 2],
    down_at: EventTime,
    elapsed: u64,
    pressed: bool,
    released: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct ActionSource {
    key: KeySource,
    producer: Option<CommandProducer>,
}
impl Button {
    fn active(&self) -> bool {
        self.held.iter().any(Option::is_some)
    }
    fn set(&mut self, source: ActionSource, down: bool, time: EventTime) {
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
    fn sample(&mut self, time: EventTime, period: EventTime, key_state: KeyState) -> (f32, bool) {
        if self.active() {
            self.elapsed = self.elapsed.saturating_add(time.since(self.down_at));
            self.down_at = time;
        }
        let fraction = if matches!(key_state, KeyState::TransitionBits) {
            // NQ/QW CL_KeyState uses transition bits, not elapsed milliseconds.
            match (self.pressed, self.released, self.active()) {
                (true, true, true) => 0.75,
                (true, true, false) => 0.25,
                (true, false, true) => 0.5,
                (false, false, true) => 1.0,
                _ => 0.0,
            }
        } else if period.0 == 0 {
            0.0
        } else {
            (self.elapsed as f32 / period.0 as f32).clamp(0.0, 1.0)
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
    weapon: Option<WeaponId>,
    zoom_sensitivity: Option<f32>,
    catchers: u32,
    catcher_owners: [Option<ModuleId>; 32],
}
impl Seat {
    fn clear_mouse(&mut self) {
        self.mouse = [0; 2];
        self.mouse_previous = [0.0; 2];
    }
}

/// Cached command tuning; map geometry and module formats never select it.
#[derive(Clone, Copy, Debug)]
pub struct InputPolicy {
    pub rules: RuleSetId,
    pub units: CommandUnits,
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
enum KeyState {
    TransitionBits,
    Elapsed,
}
#[derive(Clone, Copy)]
enum Lookspring {
    Enabled,
    NotFreelook,
    NotFreelookAndEnabled,
}
#[derive(Clone, Copy)]
enum Center {
    Drift,
    DeltaPitch,
}
#[derive(Clone, Copy)]
enum PitchClamp {
    Fixed { pitch: [f32; 2], roll: f32 },
    RelativeToPrevious(f32),
    DeltaAngle(f32),
}
#[derive(Clone, Copy)]
pub(crate) struct CommandRules {
    settings: SettingsRules,
    key_scale: Option<[f32; 2]>,
    key_limit: Option<[f32; 2]>,
    accumulation: Accumulation,
    vertical_actions: bool,
    pitch_drift: bool,
    key_state: KeyState,
    yaw_anglemod: bool,
    keyboard_look: bool,
    lookspring: Lookspring,
    center: Center,
    pitch_clamp: PitchClamp,
    noclip_disables_drift: bool,
    drift_threshold: Option<f32>,
    drift_uses_ideal_pitch: bool,
    integer_frame_time: bool,
    caught_buttons: u32,
    finish_duration: Option<(u16, u16)>,
}
/// Native setting semantics; values and defaults come from the console catalog.
#[derive(Clone, Copy)]
pub struct SettingsRules {
    pub back_follows_forward: bool,
    pub explicit_run: bool,
    pub integer_run: bool,
    pub integer_mouse_flags: bool,
}
const Q1_INPUT: CommandRules = CommandRules {
    settings: SettingsRules {
        back_follows_forward: false,
        explicit_run: true,
        integer_run: false,
        integer_mouse_flags: false,
    },
    key_scale: None,
    key_limit: None,
    accumulation: Accumulation::Float,
    vertical_actions: false,
    pitch_drift: true,
    key_state: KeyState::TransitionBits,
    yaw_anglemod: true,
    keyboard_look: true,
    lookspring: Lookspring::Enabled,
    center: Center::Drift,
    pitch_clamp: PitchClamp::Fixed {
        pitch: [-70.0, 80.0],
        roll: 50.0,
    },
    noclip_disables_drift: true,
    drift_threshold: None,
    drift_uses_ideal_pitch: true,
    integer_frame_time: false,
    caught_buttons: 0,
    finish_duration: None,
};
const Q2_INPUT: CommandRules = CommandRules {
    settings: SettingsRules {
        back_follows_forward: true,
        explicit_run: false,
        integer_run: true,
        integer_mouse_flags: false,
    },
    vertical_actions: true,
    pitch_drift: false,
    key_state: KeyState::Elapsed,
    yaw_anglemod: false,
    lookspring: Lookspring::NotFreelookAndEnabled,
    center: Center::DeltaPitch,
    pitch_clamp: PitchClamp::DeltaAngle(89.0),
    ..Q1_INPUT
};
const INPUT_RULES: [CommandRules; 5] = [
    Q1_INPUT,
    CommandRules {
        accumulation: Accumulation::Short,
        finish_duration: Some((250, 100)),
        noclip_disables_drift: false,
        drift_threshold: Some(200.0),
        drift_uses_ideal_pitch: false,
        ..Q1_INPUT
    },
    CommandRules {
        accumulation: Accumulation::Short,
        finish_duration: Some((250, 100)),
        ..Q2_INPUT
    },
    Q2_INPUT,
    CommandRules {
        settings: SettingsRules {
            integer_mouse_flags: true,
            ..Q2_INPUT.settings
        },
        key_scale: Some([64.0, 127.0]),
        key_limit: Some([-128.0, 127.0]),
        accumulation: Accumulation::Int,
        keyboard_look: false,
        lookspring: Lookspring::NotFreelook,
        pitch_clamp: PitchClamp::RelativeToPrevious(90.0),
        integer_frame_time: true,
        caught_buttons: buttons::TALK,
        ..Q2_INPUT
    },
];
pub fn settings_rules(rules: RuleSetId) -> SettingsRules {
    INPUT_RULES[rules as usize].settings
}
impl InputPolicy {
    pub fn pitch_drift(self) -> bool {
        self.command_rules().pitch_drift
    }
    pub(crate) fn command_rules(self) -> CommandRules {
        INPUT_RULES[self.rules as usize]
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
enum BindingHold {
    #[default]
    None,
    Pending(Option<CommandProducer>),
    Acquired(Option<CommandProducer>),
}

#[derive(Clone, Copy)]
struct Device {
    id: Option<DeviceId>,
    seat: Option<SeatId>,
    held: [bool; CONTROLS],
    held_count: u16,
    acquired: [BindingHold; CONTROLS],
    axes: [i16; 6],
}
impl Default for Device {
    fn default() -> Self {
        Self {
            id: None,
            seat: None,
            held: [false; CONTROLS],
            held_count: 0,
            acquired: [BindingHold::None; CONTROLS],
            axes: [0; 6],
        }
    }
}

impl Device {
    fn assigned(id: DeviceId, seat: SeatId) -> Self {
        Self {
            id: Some(id),
            seat: Some(seat),
            ..Self::default()
        }
    }
    fn set_held(&mut self, control: u16, down: bool) -> Option<bool> {
        let held = self.held.get_mut(usize::from(control))?;
        let previous = std::mem::replace(held, down);
        if previous != down {
            if down {
                self.held_count += 1;
            } else {
                self.held_count -= 1;
            }
        }
        Some(previous)
    }
}

fn seat_devices(devices: &[Device], index: usize) -> impl Iterator<Item = &Device> {
    devices
        .iter()
        .filter(move |device| device.seat.is_some_and(|seat| seat.index() == index))
}

pub use qa_core::primitives::CommandIntent;

pub struct Input {
    bindings: Box<[Option<Binding>]>,
    devices: Box<[Device]>,
    seats: [Seat; SeatId::COUNT],
    previous: Option<EventTime>,
    frame_span: EventTime,
    text_seat: SeatId,
    rejected_devices: u64,
}
impl Default for Input {
    fn default() -> Self {
        Self::load()
    }
}
impl Input {
    /// The client module supplies selection and zoom independently of physics.
    pub fn set_usercmd_value(&mut self, seat: SeatId, weapon: Option<WeaponId>, scale: f32) {
        let state = &mut self.seats[seat.index()];
        state.weapon = weapon;
        state.zoom_sensitivity = Some(scale);
    }

    pub fn reset_usercmd_value(&mut self, seat: SeatId) {
        let state = &mut self.seats[seat.index()];
        state.weapon = None;
        state.zoom_sensitivity = None;
    }

    pub fn load() -> Self {
        let mut input = Self {
            bindings: (0..CONTROLS).map(|_| None).collect(),
            devices: vec![Device::default(); MAX_INPUT_DEVICES].into_boxed_slice(),
            seats: [Seat::default(); SeatId::COUNT],
            previous: None,
            frame_span: EventTime::default(),
            text_seat: SeatId::FIRST,
            rejected_devices: 0,
        };
        input.assign(DeviceId::Keyboard(0), SeatId::FIRST);
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
        if let Some(index) = self.device_index(id) {
            if self.devices[index].held_count != 0 {
                return false;
            }
            self.remove(id, self.previous.unwrap_or_default());
            self.devices[index] = Device::assigned(id, seat);
            return true;
        }
        let Some(device) = self.devices.iter_mut().find(|device| device.id.is_none()) else {
            return false;
        };
        *device = Device::assigned(id, seat);
        true
    }
    /// Text/IME belongs to the editor's seat, not the last physical keyboard.
    pub fn set_text_seat(&mut self, seat: SeatId) {
        self.text_seat = seat;
    }
    pub fn text_seat(&self) -> SeatId {
        self.text_seat
    }
    /// Catchers suppress this seat's game input while the same held-key and
    /// release accounting continues. No second usercmd builder is introduced.
    pub fn set_catcher(&mut self, seat: SeatId, owner: u32, active: bool, time: EventTime) {
        let state = &self.seats[seat.index()];
        let catchers = if active {
            state.catchers | owner
        } else {
            state.catchers & !owner
        };
        self.set_catchers(seat, catchers, time);
    }
    pub fn set_catchers(&mut self, seat: SeatId, catchers: u32, time: EventTime) {
        let state = &mut self.seats[seat.index()];
        if (state.catchers != 0) != (catchers != 0) {
            state.clear_mouse();
            for button in &mut state.actions {
                button.sample(time, EventTime::default(), KeyState::Elapsed);
            }
        }
        state.catchers = catchers;
        for (bit, owner) in state.catcher_owners.iter_mut().enumerate() {
            if catchers & (1 << bit) == 0 {
                *owner = None;
            }
        }
    }
    pub fn key_catchers(&self, seat: SeatId) -> u32 {
        self.seats[seat.index()].catchers
    }
    pub fn replace_owned_key_catchers(
        &mut self,
        seat: SeatId,
        catchers: u32,
        module: ModuleId,
        time: EventTime,
    ) {
        self.set_catchers(seat, catchers, time);
        for (bit, owner) in self.seats[seat.index()]
            .catcher_owners
            .iter_mut()
            .enumerate()
        {
            *owner = (catchers & (1 << bit) != 0).then_some(module);
        }
    }
    pub fn catcher_module(&self, seat: SeatId, bit: u32) -> Option<ModuleId> {
        if !bit.is_power_of_two() || self.key_catchers(seat) & bit == 0 {
            return None;
        }
        self.seats[seat.index()].catcher_owners[bit.trailing_zeros() as usize]
    }
    pub fn retire_catcher_module(&mut self, module: ModuleId, time: EventTime) {
        for seat in SeatId::ALL {
            let state = &self.seats[seat.index()];
            let mut catchers = state.catchers;
            for (bit, owner) in state.catcher_owners.iter().enumerate() {
                if *owner == Some(module) {
                    catchers &= !(1 << bit);
                }
            }
            self.set_catchers(seat, catchers, time);
        }
    }
    pub fn is_down(&self, seat: SeatId, control: u16) -> bool {
        let control = keys::normalize(control);
        seat_devices(&self.devices, seat.index()).any(|device| {
            device
                .held
                .get(usize::from(control))
                .copied()
                .unwrap_or(false)
                || (224..=227).contains(&control) && device.held[usize::from(control + 4)]
        })
    }
    pub fn rejected_devices(&self) -> u64 {
        self.rejected_devices
    }
    /// The first clock event seeds input timing after window/startup work.
    pub fn seed(&mut self, time: EventTime) {
        self.previous.get_or_insert(time);
    }
    /// Load/spawn fixangle seeds the same angles later emitted by usercmds.
    /// The caller converts and validates external angles at the map/module ABI.
    pub fn set_view_angles(&mut self, seat: SeatId, angles: Vec3) {
        self.seats[seat.index()].angles = angles;
        self.seats[seat.index()].clear_mouse();
    }
    /// CLIENT resolves this request against its current native view policy.
    pub fn center_view(&mut self, seat: SeatId) {
        self.seats[seat.index()].center = true;
    }
    /// NQ/QW render-view drift runs after command construction and prediction.
    /// Return the raw pitch transition, preserving accepted offsets without
    /// changing native assignment rounding when the caller has no offset.
    pub fn drift_view(
        &mut self,
        seat: SeatId,
        time: EventTime,
        policy: InputPolicy,
        player: &qa_core::primitives::PlayerState,
        forward: f32,
    ) -> Option<(f32, f32)> {
        if !player.prediction.angles
            || !player.movement.mode.allows_view_angles()
            || !policy.pitch_drift()
            || self.seats[seat.index()].catchers != 0
        {
            return None;
        }
        let state = &mut self.seats[seat.index()];
        let rules = policy.command_rules();
        let previous = state.angles.0[0];
        state.angles.0[0] = state.drift.advance(
            time,
            self.frame_span.seconds(),
            state.angles.0[0],
            view::DriftInput {
                grounded: player.movement.grounded,
                disabled: rules.noclip_disables_drift
                    && matches!(
                        player.movement.mode,
                        qa_core::primitives::MovementMode::Noclip
                    ),
                forward,
                threshold: rules.drift_threshold.unwrap_or(policy.speed[0]),
                ideal_pitch: if rules.drift_uses_ideal_pitch {
                    player.ideal_pitch
                } else {
                    0.0
                },
                speed: policy.center_speed,
                delay: policy.center_delay,
            },
        );
        Some((previous, state.angles.0[0]))
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
        if !text.is_empty() {
            return self.bind_bytes(control, text.as_bytes(), time, target);
        }
        if self.bind(control, None, time, target) {
            Ok(())
        } else {
            Err(BindError::Control)
        }
    }
    /// Native empty bindings retain their allocated empty value, distinct from
    /// an unbound control. Human bind_text keeps its empty-to-unbound behavior.
    pub fn bind_bytes(
        &mut self,
        control: u16,
        text: &[u8],
        time: EventTime,
        target: &mut impl Target,
    ) -> Result<(), BindError> {
        self.bind_bytes_owned(control, text, None, time, target)
    }
    pub fn bind_bytes_owned(
        &mut self,
        control: u16,
        text: &[u8],
        producer: Option<CommandProducer>,
        time: EventTime,
        target: &mut impl Target,
    ) -> Result<(), BindError> {
        if self
            .binding(control)
            .is_some_and(|binding| binding.bytes() == text && binding.producer == producer)
        {
            return Ok(());
        }
        let mut binding = Binding::parse_bytes(text)?;
        binding.producer = producer;
        if self.bind(control, Some(binding), time, target) {
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
            for physical in keys::physical_controls(control) {
                if self.devices[index].held[usize::from(physical)]
                    && let Some(id) = self.devices[index].id
                {
                    self.key(id, physical, false, false, time, target);
                    self.devices[index].set_held(physical, true);
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
    /// Retired definitions release through the same executor as replacement.
    /// Their physical holds cannot reacquire a binding before the next key-up.
    pub fn retain_producers(
        &mut self,
        mut retained: impl FnMut(CommandProducer) -> bool,
        time: EventTime,
        target: &mut impl Target,
    ) {
        for control in 0..CONTROLS {
            let remove = self.bindings[control]
                .as_ref()
                .and_then(Binding::producer)
                .is_some_and(|producer| !retained(producer));
            if !remove {
                continue;
            }
            self.bind(control as u16, None, time, target);
        }
        // Queued releases can be pruned after their physical key is already up.
        // Action ownership, rather than physical state, identifies these holds.
        self.release_sources(time, |source| {
            source.producer.is_some_and(|producer| !retained(producer))
        });
    }
    pub fn retain_bindings(
        &mut self,
        retained: impl FnMut(CommandProducer) -> bool,
        time: EventTime,
        target: &mut impl Target,
    ) {
        self.retain_producers(retained, time, target);
    }
    pub fn retire_bindings(
        &mut self,
        producer: CommandProducer,
        time: EventTime,
        target: &mut impl Target,
    ) {
        self.retain_producers(|owner| owner != producer, time, target);
    }
    pub fn button(
        &mut self,
        seat: SeatId,
        action: Action,
        down: bool,
        key: Option<KeySource>,
        producer: Option<CommandProducer>,
        time: EventTime,
    ) {
        let button = &mut self.seats[seat.index()].actions[action as usize];
        if !down && key.is_none() && producer.is_none() {
            button.released |= button.active();
            button.held = [None; 2];
            // Keep an impulse pressed and accumulated elapsed time this frame.
        } else if !down && key.is_none() {
            for source in button.held.into_iter().flatten() {
                if source.producer == producer {
                    button.set(source, false, time);
                }
            }
        } else {
            button.set(
                ActionSource {
                    key: key.unwrap_or(KeySource {
                        device: DeviceId::Keyboard(0),
                        control: u16::MAX,
                    }),
                    producer,
                },
                down,
                time,
            );
        }
    }
    fn release_sources(&mut self, time: EventTime, mut matches: impl FnMut(ActionSource) -> bool) {
        for seat in &mut self.seats {
            for button in &mut seat.actions {
                let sources = button.held;
                for source in sources
                    .into_iter()
                    .flatten()
                    .filter(|source| matches(*source))
                {
                    button.set(source, false, time);
                }
            }
        }
    }
    /// Bounded command rejection cancels this source, never its other holders.
    pub fn cancel_source(
        &mut self,
        source: KeySource,
        producer: Option<CommandProducer>,
        time: EventTime,
    ) {
        self.release_sources(time, |held| held.key == source && held.producer == producer);
        if let Some(index) = self.device_index(source.device)
            && let Some(acquired) = self.devices[index]
                .acquired
                .get_mut(usize::from(source.control))
        {
            if matches!(*acquired, BindingHold::Pending(owner) | BindingHold::Acquired(owner) if owner == producer)
            {
                *acquired = BindingHold::None;
            }
        }
    }
    fn remove(&mut self, id: DeviceId, time: EventTime) {
        self.release_sources(time, |source| source.key.device == id);
        if let Some(index) = self.device_index(id) {
            self.devices[index] = Device::default();
        }
    }
    fn device_index(&self, id: DeviceId) -> Option<usize> {
        self.devices.iter().position(|device| device.id == Some(id))
    }
    fn clear_mouse(&mut self) {
        for seat in &mut self.seats {
            seat.clear_mouse();
        }
    }
    fn ensure_device(&mut self, id: DeviceId) -> Option<usize> {
        if let Some(index) = self.device_index(id) {
            return Some(index);
        }
        if !matches!(id, DeviceId::Keyboard(_) | DeviceId::Mouse(_)) {
            return None;
        }
        let Some(index) = self.devices.iter().position(|device| device.id.is_none()) else {
            self.rejected_devices = self.rejected_devices.saturating_add(1);
            return None;
        };
        self.devices[index] = Device::assigned(id, SeatId::FIRST);
        Some(index)
    }
    fn device_seat(&self, id: DeviceId) -> Option<SeatId> {
        self.device_index(id)
            .and_then(|index| self.devices[index].seat)
    }
    pub fn build_frame_with_policy(
        &mut self,
        time: EventTime,
        policies: &[InputPolicy; SeatId::COUNT],
    ) -> [UserCmd; SeatId::COUNT] {
        let period = EventTime(time.since(self.previous.unwrap_or(time)));
        self.previous = Some(time);
        self.frame_span = period;
        std::array::from_fn(|index| {
            let policy = policies[index];
            let rules = policy.command_rules();
            let frame_time = if rules.integer_frame_time {
                EventTime::from_milliseconds(period.milliseconds())
            } else {
                period
            };
            let frame_seconds = frame_time.seconds();
            let frame_milliseconds = if rules.integer_frame_time {
                frame_time.milliseconds() as f32
            } else {
                (frame_seconds * 1000.0) as f32
            };
            let seat = &mut self.seats[index];
            let look_released = seat.actions[Action::MouseLook as usize].released;
            let spring = look_released
                && match rules.lookspring {
                    Lookspring::Enabled => policy.lookspring,
                    Lookspring::NotFreelook => !policy.freelook,
                    Lookspring::NotFreelookAndEnabled => !policy.freelook && policy.lookspring,
                };
            if std::mem::take(&mut seat.center) || spring {
                match rules.center {
                    Center::Drift => seat.drift.start(time, policy.center_speed),
                    Center::DeltaPitch => seat.angles.0[0] = -policy.delta_pitch,
                }
            }
            // Modifiers use native held state; action buttons also retain taps.
            let held = seat.actions.each_ref().map(Button::active);
            let sampled = seat
                .actions
                .each_mut()
                .map(|button| button.sample(time, period, rules.key_state));
            if seat.catchers != 0 {
                seat.clear_mouse();
                return UserCmdBuilder::build(
                    period,
                    time,
                    CommandIntent {
                        view_angles: seat.angles,
                        weapon: seat.weapon,
                        buttons: rules.caught_buttons,
                        ..CommandIntent::default()
                    },
                    policy,
                );
            }
            let fraction = |action: Action| sampled[action as usize].0;
            let strafe = held[Action::Strafe as usize];
            let look = held[Action::MouseLook as usize];
            let klook = held[Action::KeyboardLook as usize] && rules.keyboard_look;
            let speed_key = held[Action::Walk as usize];
            let previous_pitch = seat.angles.0[0];
            let raw = seat.mouse.map(|value| value as f32);
            let mut mouse = if policy.filter {
                std::array::from_fn(|axis| (raw[axis] + seat.mouse_previous[axis]) * 0.5)
            } else {
                raw
            };
            seat.mouse_previous = raw;
            let rate = qa_core::math::length(Vec3([mouse[0], mouse[1], 0.0]))
                / frame_milliseconds.max(1.0);
            let mut gain = policy.sensitivity + rate * policy.acceleration;
            if let Some(scale) = seat.zoom_sensitivity {
                gain *= scale;
            }
            mouse.iter_mut().for_each(|value| *value *= gain);
            let horizontal_strafe = strafe || policy.look_strafe && look;
            let mouse_pitch = !strafe && (policy.freelook || look);
            if rules.pitch_drift
                && (klook
                    || look
                    || fraction(Action::LookUp) != 0.0
                    || fraction(Action::LookDown) != 0.0
                    || mouse_pitch && mouse[1] != 0.0)
            {
                seat.drift.stop(time);
            }
            let seconds = frame_seconds as f32;
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
                if rules.yaw_anglemod {
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
            if let PitchClamp::Fixed { pitch, roll } = rules.pitch_clamp {
                seat.angles.0[0] = seat.angles.0[0].clamp(pitch[0], pitch[1]);
                seat.angles.0[2] = seat.angles.0[2].clamp(-roll, roll);
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
                weapon: seat.weapon,
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
            match rules.pitch_clamp {
                PitchClamp::Fixed { pitch, .. } => {
                    seat.angles.0[0] = seat.angles.0[0].clamp(pitch[0], pitch[1]);
                }
                PitchClamp::RelativeToPrevious(limit) => {
                    seat.angles.0[0] =
                        seat.angles.0[0].clamp(previous_pitch - limit, previous_pitch + limit);
                }
                PitchClamp::DeltaAngle(limit) => {
                    let delta = qa_core::math::wrap_above_180(policy.delta_pitch);
                    seat.angles.0[0] = seat.angles.0[0].clamp(-limit - delta, limit - delta);
                }
            }
            for device in seat_devices(&self.devices, index) {
                intent.axes[usize::from(intent.axis_count)] = [
                    -f32::from(device.axes[1]) / 32768.0,
                    f32::from(device.axes[0]) / 32768.0,
                ];
                intent.axis_count += 1;
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
            if seat_devices(&self.devices, index).any(|device| device.held_count != 0) {
                mask |= buttons::ANY;
            }
            intent.view_angles = seat.angles;
            intent.buttons = mask;
            UserCmdBuilder::build(period, time, intent, policy)
        })
    }
}
