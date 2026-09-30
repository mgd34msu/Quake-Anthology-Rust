//! Q3 source-derived semantics (donor `tools/reference/q3/semantics.ts`).
//!
//! Bounded transcriptions of pinned original operations: velocity clipping,
//! the pmove clock and subdivision, timer expiry, the weapon state machine,
//! client connection, and nested VM calls. Float behavior follows the donor
//! exactly: binary32 stores are `as f32` round-trips, bit operations apply
//! JavaScript `ToInt32` coercion, and `Math.max`/`Math.trunc` map to their
//! `f64` counterparts with NaN propagated like JavaScript.

use crate::error::ToolsError;
use crate::json::Json;

/// A three-component vector.
pub type Vector3 = [f64; 3];

/// Binary32 store: `Math.fround` round-to-nearest ties-to-even.
fn fround(value: f64) -> f64 {
    f64::from(value as f32)
}

/// JavaScript `ToInt32` coercion for bit operations.
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    ((value.trunc() % 4_294_967_296.0) as i64) as u32 as i32
}

/// JavaScript `Math.max`: NaN propagates (unlike `f64::max`).
fn js_max(first: f64, second: f64) -> f64 {
    if first.is_nan() || second.is_nan() {
        f64::NAN
    } else {
        first.max(second)
    }
}

fn vector_json(vector: &Vector3) -> Json {
    Json::array(vector.iter().map(|component| Json::float(*component)).collect())
}

/// Velocity-clip input.
#[derive(Debug, Clone, Copy)]
pub struct ClipInput {
    /// Entering velocity.
    pub velocity: Vector3,
    /// Clip plane normal.
    pub normal: Vector3,
    /// Overbounce factor.
    pub overbounce: f64,
}

impl ClipInput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("velocity".to_owned(), vector_json(&self.velocity)),
            ("normal".to_owned(), vector_json(&self.normal)),
            ("overbounce".to_owned(), Json::float(self.overbounce)),
        ])
    }
}

/// Velocity-clip output.
#[derive(Debug, Clone, Copy)]
pub struct ClipOutput {
    /// Backoff along the normal.
    pub backoff: f64,
    /// Clipped velocity.
    pub velocity: Vector3,
}

impl ClipOutput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("backoff".to_owned(), Json::float(self.backoff)),
            ("velocity".to_owned(), vector_json(&self.velocity)),
        ])
    }
}

/// Clip a velocity against a plane (`PM_ClipVelocity` staging).
#[must_use]
pub fn clip_velocity(input: &ClipInput) -> ClipOutput {
    let x = fround(input.velocity[0]);
    let y = fround(input.velocity[1]);
    let z = fround(input.velocity[2]);
    let nx = fround(input.normal[0]);
    let ny = fround(input.normal[1]);
    let nz = fround(input.normal[2]);
    let overbounce = fround(input.overbounce);
    let dot = fround(fround(fround(x * nx) + fround(y * ny)) + fround(z * nz));
    let backoff = fround(if dot < 0.0 { dot * overbounce } else { dot / overbounce });
    ClipOutput {
        backoff,
        velocity: [
            fround(x - fround(nx * backoff)),
            fround(y - fround(ny * backoff)),
            fround(z - fround(nz * backoff)),
        ],
    }
}

/// Single-clock output.
#[derive(Debug, Clone, Copy)]
pub struct ClockOutput {
    /// Consumed command time.
    pub command_time: f64,
    /// Clamped milliseconds.
    pub msec: f64,
    /// Frame time in seconds.
    pub frametime: f64,
}

impl ClockOutput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("commandTime".to_owned(), Json::float(self.command_time)),
            ("msec".to_owned(), Json::float(self.msec)),
            ("frametime".to_owned(), Json::float(self.frametime)),
        ])
    }
}

/// Advance one pmove clock (`PmoveSingle` clock writes).
#[must_use]
pub fn single_clock(command_time: f64, server_time: f64) -> ClockOutput {
    let mut msec = server_time - command_time;
    if msec < 1.0 {
        msec = 1.0;
    } else if msec > 200.0 {
        msec = 200.0;
    }
    ClockOutput { command_time: server_time, msec, frametime: fround(msec * 0.001) }
}

/// Move subdivision mode.
#[derive(Debug, Clone, Copy)]
pub enum Subdivision {
    /// Variable steps capped at 66 ms.
    Variable,
    /// Fixed steps of `msec` milliseconds.
    Fixed {
        /// Step size; must be a positive integer.
        msec: f64,
    },
}

impl Subdivision {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::Variable => Json::object(vec![("kind".to_owned(), Json::string("variable"))]),
            Self::Fixed { msec } => Json::object(vec![
                ("kind".to_owned(), Json::string("fixed")),
                ("msec".to_owned(), Json::float(*msec)),
            ]),
        }
    }
}

/// Move-subdivision input.
#[derive(Debug, Clone, Copy)]
pub struct MoveInput {
    /// Command time.
    pub command_time: f64,
    /// Server time.
    pub server_time: f64,
    /// Frame count.
    pub framecount: f64,
    /// Subdivision mode.
    pub subdivision: Subdivision,
    /// Whether jump is held.
    pub jump_held: bool,
    /// Upward command.
    pub upmove: f64,
}

impl MoveInput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("commandTime".to_owned(), Json::float(self.command_time)),
            ("serverTime".to_owned(), Json::float(self.server_time)),
            ("framecount".to_owned(), Json::float(self.framecount)),
            ("subdivision".to_owned(), self.subdivision.to_json()),
            ("jumpHeld".to_owned(), Json::boolean(self.jump_held)),
            ("upmove".to_owned(), Json::float(self.upmove)),
        ])
    }
}

/// One subdivision step.
#[derive(Debug, Clone, Copy)]
pub struct MoveStep {
    /// Command time after the step.
    pub command_time: f64,
    /// Step milliseconds.
    pub msec: f64,
    /// Upward command during the step.
    pub upmove: f64,
}

impl MoveStep {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("commandTime".to_owned(), Json::float(self.command_time)),
            ("msec".to_owned(), Json::float(self.msec)),
            ("upmove".to_owned(), Json::float(self.upmove)),
        ])
    }
}

/// Move-subdivision output.
#[derive(Debug, Clone)]
pub struct MoveOutput {
    /// Final command time.
    pub command_time: f64,
    /// Final frame count.
    pub framecount: f64,
    /// Final upward command.
    pub upmove: f64,
    /// Subdivision steps.
    pub steps: Vec<MoveStep>,
}

impl MoveOutput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("commandTime".to_owned(), Json::float(self.command_time)),
            ("framecount".to_owned(), Json::float(self.framecount)),
            ("upmove".to_owned(), Json::float(self.upmove)),
            ("steps".to_owned(), Json::array(self.steps.iter().map(MoveStep::to_json).collect())),
        ])
    }
}

/// Subdivide one move into clock steps (`Pmove` time loop).
pub fn subdivide_move(input: &MoveInput) -> Result<MoveOutput, ToolsError> {
    if let Subdivision::Fixed { msec } = input.subdivision {
        if !msec.is_finite() || msec.fract() != 0.0 || msec < 1.0 {
            return Err(ToolsError::invalid("Fixed subdivision must be a positive integer"));
        }
    }
    let mut command_time = input.command_time;
    let mut framecount = input.framecount;
    let mut upmove = input.upmove;
    let mut steps = Vec::new();
    if input.server_time < command_time {
        return Ok(MoveOutput { command_time, framecount, upmove, steps });
    }
    if input.server_time > command_time + 1000.0 {
        command_time = input.server_time - 1000.0;
    }
    framecount = f64::from(to_int32(framecount + 1.0) & 63);
    while command_time != input.server_time {
        let mut msec = input.server_time - command_time;
        let maximum = match input.subdivision {
            Subdivision::Fixed { msec } => msec,
            Subdivision::Variable => 66.0,
        };
        if msec > maximum {
            msec = maximum;
        }
        let single = single_clock(command_time, command_time + msec);
        command_time = single.command_time;
        steps.push(MoveStep { command_time, msec: single.msec, upmove });
        if input.jump_held {
            upmove = 20.0;
        }
    }
    Ok(MoveOutput { command_time, framecount, upmove, steps })
}

/// Timer-drop input.
#[derive(Debug, Clone, Copy)]
pub struct TimerInput {
    /// Elapsed milliseconds.
    pub msec: f64,
    /// Powerup time remaining.
    pub pm_time: f64,
    /// Player flags.
    pub flags: f64,
    /// Legs animation timer.
    pub legs_timer: f64,
    /// Torso animation timer.
    pub torso_timer: f64,
}

/// Timer-drop output.
#[derive(Debug, Clone, Copy)]
pub struct TimerOutput {
    /// Powerup time remaining.
    pub pm_time: f64,
    /// Player flags.
    pub flags: f64,
    /// Legs animation timer.
    pub legs_timer: f64,
    /// Torso animation timer.
    pub torso_timer: f64,
}

impl TimerOutput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("pmTime".to_owned(), Json::float(self.pm_time)),
            ("flags".to_owned(), Json::float(self.flags)),
            ("legsTimer".to_owned(), Json::float(self.legs_timer)),
            ("torsoTimer".to_owned(), Json::float(self.torso_timer)),
        ])
    }
}

/// Expire powerup and animation timers (`PM_DropTimers`).
#[must_use]
pub fn drop_timers(input: &TimerInput) -> TimerOutput {
    let mut pm_time = input.pm_time;
    let mut flags = input.flags;
    let mut legs_timer = input.legs_timer;
    let mut torso_timer = input.torso_timer;
    if pm_time != 0.0 {
        if input.msec >= pm_time {
            flags = f64::from(to_int32(flags) & !(256 | 32 | 64));
            pm_time = 0.0;
        } else {
            pm_time -= input.msec;
        }
    }
    if legs_timer > 0.0 {
        legs_timer = js_max(0.0, legs_timer - input.msec);
    }
    if torso_timer > 0.0 {
        torso_timer = js_max(0.0, torso_timer - input.msec);
    }
    TimerOutput { pm_time, flags, legs_timer, torso_timer }
}

/// Modeled weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weapon {
    /// Machinegun.
    Machinegun,
    /// Rocket launcher.
    Rocket,
    /// Lightning gun.
    Lightning,
}

impl Weapon {
    /// The donor weapon name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Machinegun => "machinegun",
            Self::Rocket => "rocket",
            Self::Lightning => "lightning",
        }
    }
}

/// Weapon state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponState {
    /// Ready.
    Ready,
    /// Raising.
    Raising,
    /// Dropping.
    Dropping,
    /// Firing.
    Firing,
}

impl WeaponState {
    /// The donor state name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Raising => "raising",
            Self::Dropping => "dropping",
            Self::Firing => "firing",
        }
    }
}

/// Ammunition per weapon.
#[derive(Debug, Clone, Copy)]
pub struct Ammo {
    /// Machinegun rounds.
    pub machinegun: f64,
    /// Rockets.
    pub rocket: f64,
    /// Lightning cells (`-1` is infinite).
    pub lightning: f64,
}

impl Ammo {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("machinegun".to_owned(), Json::float(self.machinegun)),
            ("rocket".to_owned(), Json::float(self.rocket)),
            ("lightning".to_owned(), Json::float(self.lightning)),
        ])
    }

    fn get(&self, weapon: Weapon) -> f64 {
        match weapon {
            Weapon::Machinegun => self.machinegun,
            Weapon::Rocket => self.rocket,
            Weapon::Lightning => self.lightning,
        }
    }

    fn decrement(&mut self, weapon: Weapon) {
        match weapon {
            Weapon::Machinegun => self.machinegun -= 1.0,
            Weapon::Rocket => self.rocket -= 1.0,
            Weapon::Lightning => self.lightning -= 1.0,
        }
    }
}

/// One weapon step.
#[derive(Debug, Clone, Copy)]
pub struct WeaponStep {
    /// Step milliseconds.
    pub msec: f64,
    /// Selected weapon.
    pub weapon: Weapon,
    /// Whether attack is held.
    pub attack: bool,
    /// Whether haste applies.
    pub haste: bool,
}

impl WeaponStep {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("msec".to_owned(), Json::float(self.msec)),
            ("weapon".to_owned(), Json::string(self.weapon.as_str())),
            ("attack".to_owned(), Json::boolean(self.attack)),
            ("haste".to_owned(), Json::boolean(self.haste)),
        ])
    }
}

/// Weapon-sequence input.
#[derive(Debug, Clone)]
pub struct WeaponInput {
    /// Current weapon.
    pub weapon: Weapon,
    /// Current state.
    pub weapon_state: WeaponState,
    /// State time remaining.
    pub weapon_time: f64,
    /// Torso animation.
    pub torso_anim: f64,
    /// Ammunition.
    pub ammo: Ammo,
    /// Event sequence.
    pub event_sequence: f64,
    /// Steps.
    pub steps: Vec<WeaponStep>,
}

impl WeaponInput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("weapon".to_owned(), Json::string(self.weapon.as_str())),
            ("weaponState".to_owned(), Json::string(self.weapon_state.as_str())),
            ("weaponTime".to_owned(), Json::float(self.weapon_time)),
            ("torsoAnim".to_owned(), Json::float(self.torso_anim)),
            ("ammo".to_owned(), self.ammo.to_json()),
            ("eventSequence".to_owned(), Json::float(self.event_sequence)),
            ("steps".to_owned(), Json::array(self.steps.iter().map(WeaponStep::to_json).collect())),
        ])
    }
}

/// One observed weapon state.
#[derive(Debug, Clone, Copy)]
pub struct WeaponObservation {
    /// Current weapon.
    pub weapon: Weapon,
    /// Current state.
    pub weapon_state: WeaponState,
    /// State time remaining.
    pub weapon_time: f64,
    /// Torso animation.
    pub torso_anim: f64,
    /// Ammunition.
    pub ammo: Ammo,
    /// Event sequence.
    pub event_sequence: f64,
}

impl WeaponObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("weapon".to_owned(), Json::string(self.weapon.as_str())),
            ("weaponState".to_owned(), Json::string(self.weapon_state.as_str())),
            ("weaponTime".to_owned(), Json::float(self.weapon_time)),
            ("torsoAnim".to_owned(), Json::float(self.torso_anim)),
            ("ammo".to_owned(), self.ammo.to_json()),
            ("eventSequence".to_owned(), Json::float(self.event_sequence)),
        ])
    }
}

/// One predictable event with its ring slot and state.
#[derive(Debug, Clone, Copy)]
pub struct WeaponEvent {
    /// Event number.
    pub event: f64,
    /// Event parameter (always zero).
    pub parm: f64,
    /// Ring slot.
    pub index: f64,
    /// State at emission.
    pub state: WeaponObservation,
}

impl WeaponEvent {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("event".to_owned(), Json::float(self.event)),
            ("parm".to_owned(), Json::float(self.parm)),
            ("index".to_owned(), Json::float(self.index)),
            ("state".to_owned(), self.state.to_json()),
        ])
    }
}

/// Weapon-sequence output.
#[derive(Debug, Clone)]
pub struct WeaponSequence {
    /// Observed states.
    pub states: Vec<WeaponObservation>,
    /// Emitted events.
    pub events: Vec<WeaponEvent>,
    /// Two-slot event ring.
    pub ring: [f64; 2],
    /// Causal trace.
    pub trace: Vec<String>,
}

impl WeaponSequence {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("states".to_owned(), Json::array(self.states.iter().map(WeaponObservation::to_json).collect())),
            ("events".to_owned(), Json::array(self.events.iter().map(WeaponEvent::to_json).collect())),
            ("ring".to_owned(), Json::array(self.ring.iter().map(|slot| Json::float(*slot)).collect())),
            ("trace".to_owned(), Json::array(self.trace.iter().map(Json::string).collect())),
        ])
    }
}

/// Run the weapon state machine over direct `PM_Weapon` durations.
#[must_use]
pub fn weapon_sequence(input: &WeaponInput) -> WeaponSequence {
    let mut weapon = input.weapon;
    let mut weapon_state = input.weapon_state;
    let mut weapon_time = input.weapon_time;
    let mut torso_anim = input.torso_anim;
    let mut ammo = input.ammo;
    let mut event_sequence = input.event_sequence;
    let mut states = Vec::new();
    let mut events = Vec::new();
    let mut ring = [0.0, 0.0];
    let mut trace: Vec<String> = Vec::new();
    let observe = |weapon: Weapon,
                   weapon_state: WeaponState,
                   weapon_time: f64,
                   torso_anim: f64,
                   ammo: Ammo,
                   event_sequence: f64| {
        WeaponObservation { weapon, weapon_state, weapon_time, torso_anim, ammo, event_sequence }
    };
    for (index, step) in input.steps.iter().enumerate() {
        trace.push(format!("step:{index}"));
        if weapon_time > 0.0 {
            weapon_time -= step.msec;
        }
        if weapon_time <= 0.0 || weapon_state != WeaponState::Firing {
            if weapon != step.weapon && weapon_state != WeaponState::Dropping {
                let slot = to_int32(event_sequence) & 1;
                ring[slot as usize] = 22.0;
                events.push(WeaponEvent {
                    event: 22.0,
                    parm: 0.0,
                    index: f64::from(slot),
                    state: observe(weapon, weapon_state, weapon_time, torso_anim, ammo, event_sequence),
                });
                trace.push(format!("event:22:sequence:{event_sequence}"));
                event_sequence += 1.0;
                weapon_state = WeaponState::Dropping;
                weapon_time += 200.0;
                torso_anim = f64::from((to_int32(torso_anim) & 128) ^ 128 | 9);
                trace.push(format!("animation:{torso_anim}"));
            }
        }
        if weapon_time > 0.0 {
            states.push(observe(weapon, weapon_state, weapon_time, torso_anim, ammo, event_sequence));
            continue;
        }
        if weapon_state == WeaponState::Dropping {
            weapon = step.weapon;
            weapon_state = WeaponState::Raising;
            weapon_time += 250.0;
            torso_anim = f64::from((to_int32(torso_anim) & 128) ^ 128 | 10);
            trace.push(format!("animation:{torso_anim}"));
        } else if weapon_state == WeaponState::Raising {
            weapon_state = WeaponState::Ready;
            torso_anim = f64::from((to_int32(torso_anim) & 128) ^ 128 | 11);
            trace.push(format!("animation:{torso_anim}"));
        } else if !step.attack {
            weapon_time = 0.0;
            weapon_state = WeaponState::Ready;
        } else {
            torso_anim = f64::from((to_int32(torso_anim) & 128) ^ 128 | 7);
            trace.push(format!("animation:{torso_anim}"));
            weapon_state = WeaponState::Firing;
            if ammo.get(weapon) == 0.0 {
                let slot = to_int32(event_sequence) & 1;
                ring[slot as usize] = 21.0;
                events.push(WeaponEvent {
                    event: 21.0,
                    parm: 0.0,
                    index: f64::from(slot),
                    state: observe(weapon, weapon_state, weapon_time, torso_anim, ammo, event_sequence),
                });
                trace.push(format!("event:21:sequence:{event_sequence}"));
                event_sequence += 1.0;
                weapon_time += 500.0;
            } else {
                if ammo.get(weapon) != -1.0 {
                    ammo.decrement(weapon);
                    trace.push(format!("ammo:{}:{}", weapon.as_str(), ammo.get(weapon)));
                }
                let slot = to_int32(event_sequence) & 1;
                ring[slot as usize] = 23.0;
                events.push(WeaponEvent {
                    event: 23.0,
                    parm: 0.0,
                    index: f64::from(slot),
                    state: observe(weapon, weapon_state, weapon_time, torso_anim, ammo, event_sequence),
                });
                trace.push(format!("event:23:sequence:{event_sequence}"));
                event_sequence += 1.0;
                let mut add_time: f64 = match weapon {
                    Weapon::Rocket => 800.0,
                    Weapon::Machinegun => 100.0,
                    Weapon::Lightning => 50.0,
                };
                if step.haste {
                    add_time = (add_time / 1.3).trunc();
                }
                weapon_time += add_time;
            }
        }
        states.push(observe(weapon, weapon_state, weapon_time, torso_anim, ammo, event_sequence));
    }
    WeaponSequence { states, events, ring, trace }
}

/// Prior VM context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorVm {
    /// User-interface VM.
    Ui,
}

impl PriorVm {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ui => "ui",
        }
    }
}

/// Client-connect input.
#[derive(Debug, Clone)]
pub struct ConnectInput {
    /// Whether the client is banned.
    pub banned: bool,
    /// Client address.
    pub ip: String,
    /// Configured password.
    pub configured_password: String,
    /// Provided password.
    pub provided_password: String,
    /// Whether the client already has the bot flag.
    pub existing_bot_flag: bool,
    /// Prior VM context.
    pub prior_vm: Option<PriorVm>,
}

impl ConnectInput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("banned".to_owned(), Json::boolean(self.banned)),
            ("ip".to_owned(), Json::string(&self.ip)),
            ("configuredPassword".to_owned(), Json::string(&self.configured_password)),
            ("providedPassword".to_owned(), Json::string(&self.provided_password)),
            ("existingBotFlag".to_owned(), Json::boolean(self.existing_bot_flag)),
            ("priorVm".to_owned(), self.prior_vm.map_or(Json::Null, |vm| Json::string(vm.as_str()))),
        ])
    }
}

/// Server slot state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    /// Free slot.
    Free,
    /// Connected slot.
    Connected,
}

impl ServerState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Connected => "connected",
        }
    }
}

/// Current VM context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentVm {
    /// User-interface VM.
    Ui,
    /// Game VM.
    Game,
}

impl CurrentVm {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::Game => "game",
        }
    }
}

/// Client-connect output.
#[derive(Debug, Clone)]
pub struct ConnectOutput {
    /// Immediate return value.
    pub return_value: f64,
    /// Denial text.
    pub denial: Option<String>,
    /// Server slot state.
    pub server_state: ServerState,
    /// Current VM context.
    pub current_vm: CurrentVm,
    /// Call trace.
    pub trace: Vec<String>,
}

impl ConnectOutput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("returnValue".to_owned(), Json::float(self.return_value)),
            ("denial".to_owned(), self.denial.as_ref().map_or(Json::Null, Json::string)),
            ("serverState".to_owned(), Json::string(self.server_state.as_str())),
            ("currentVm".to_owned(), Json::string(self.current_vm.as_str())),
            ("trace".to_owned(), Json::array(self.trace.iter().map(Json::string).collect())),
        ])
    }
}

/// Evaluate the immediate client-connect path (`SV_DirectConnect` prologue).
#[must_use]
pub fn client_connect(input: &ConnectInput) -> ConnectOutput {
    let mut trace = vec![
        "VM_Call:game:enter".to_owned(),
        "vmMain:GAME_CLIENT_CONNECT".to_owned(),
        "trap_GetUserinfo".to_owned(),
        "G_FilterPacket".to_owned(),
    ];
    let mut denial: Option<String> = None;
    if input.banned {
        denial = Some("You are banned from this server.".to_owned());
    } else if !input.existing_bot_flag && input.ip != "localhost" {
        trace.push("password:check".to_owned());
        if !input.configured_password.is_empty()
            && input.configured_password.to_lowercase() != "none"
            && input.configured_password != input.provided_password
        {
            denial = Some("Invalid password".to_owned());
        }
    }
    if denial.is_none() {
        trace.extend(
            [
                "client:zero",
                "connected:CON_CONNECTING",
                "G_InitSessionData",
                "G_ReadSessionData",
                "G_LogPrintf",
                "ClientUserinfoChanged",
                "trap_SendServerCommand:connected",
                "CalculateRanks",
            ]
            .into_iter()
            .map(str::to_owned),
        );
    }
    let return_value = if denial.is_none() { 0.0 } else { 32.0 };
    trace.push(format!("ClientConnect:return:{return_value}"));
    trace.push(format!("vmMain:return:{return_value}"));
    trace.push(format!("VM_Call:return:{return_value}"));
    let current_vm = input.prior_vm.map_or(CurrentVm::Game, |vm| match vm {
        PriorVm::Ui => CurrentVm::Ui,
    });
    if let Some(reason) = &denial {
        trace.push("VM_ExplicitArgPtr:game:32".to_owned());
        trace.push(format!("NET_OutOfBandPrint:print\n{reason}\n"));
        trace.push("server:return".to_owned());
    } else {
        trace.extend(
            ["SV_UserinfoChanged", "NET_OutOfBandPrint:connectResponse", "server:CS_CONNECTED"]
                .into_iter()
                .map(str::to_owned),
        );
    }
    let server_state = if denial.is_none() { ServerState::Connected } else { ServerState::Free };
    ConnectOutput { return_value, denial, server_state, current_vm, trace }
}

/// Nested-VM output.
#[derive(Debug, Clone)]
pub struct NestedOutput {
    /// Outermost result.
    pub result: f64,
    /// Current VM after the outermost return.
    pub current_vm: Option<String>,
    /// Call trace.
    pub trace: Vec<String>,
}

impl NestedOutput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("result".to_owned(), Json::float(self.result)),
            ("currentVm".to_owned(), self.current_vm.as_ref().map_or(Json::Null, Json::string)),
            ("trace".to_owned(), Json::array(self.trace.iter().map(Json::string).collect())),
        ])
    }
}

/// Evaluate nested VM save/restore with synthetic entry callbacks.
#[must_use]
pub fn nested_vm(prior_vm: Option<&str>) -> NestedOutput {
    fn call(
        vm: &str,
        current_vm: &mut Option<String>,
        trace: &mut Vec<String>,
        entry: &mut dyn FnMut(&mut Option<String>, &mut Vec<String>) -> f64,
    ) -> f64 {
        let old_vm = current_vm.clone();
        *current_vm = Some(vm.to_owned());
        trace.push(format!("enter:{vm}:current:{}", current_vm.as_deref().unwrap_or("null")));
        let result = entry(current_vm, trace);
        if old_vm.is_some() {
            *current_vm = old_vm;
        }
        trace.push(format!("return:{vm}:{result}:current:{}", current_vm.as_deref().unwrap_or("null")));
        result
    }
    let mut current_vm = prior_vm.map(str::to_owned);
    let mut trace = Vec::new();
    let result = call("game", &mut current_vm, &mut trace, &mut |current_vm, trace| {
        trace.push(format!("host:before:current:{}", current_vm.as_deref().unwrap_or("null")));
        let nested = call("cgame", current_vm, trace, &mut |_, _| 7.0);
        trace.push(format!("host:after:{nested}:current:{}", current_vm.as_deref().unwrap_or("null")));
        nested + 1.0
    });
    NestedOutput { result, current_vm, trace }
}
