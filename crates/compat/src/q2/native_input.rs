//! Port of `src/compat/q2/native-input.ts`.
//! Bridges public ClientThink/Pmove boundaries over synthetic client tables.

use qa_core::math::{Bounds, Vec3};
use qa_world::client::{ClientCommand, ClientFamily};

use super::native_primary_weapons::{HostResult, NativeActorId, NativeHostError};

/// Teleport-time pmove flag selecting absolute yaw.
pub const PMF_TIME_TELEPORT: u32 = 32;
/// On-ground pmove flag.
pub const PMF_ON_GROUND: u32 = 4;
/// Ducked pmove flag.
pub const PMF_DUCKED: u32 = 1;

/// Classic 16-byte user command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicUserCommand {
    /// Milliseconds.
    pub milliseconds: u8,
    /// Buttons bitmask.
    pub buttons: u8,
    /// Angle shorts.
    pub angle_shorts: [i16; 3],
    /// Forward move.
    pub forward_move: i16,
    /// Side move.
    pub side_move: i16,
    /// Up move.
    pub up_move: i16,
    /// Weapon impulse.
    pub impulse: u8,
    /// Light level.
    pub light_level: u8,
}

/// Read a classic user command from 16 bytes.
#[must_use]
pub fn read_classic_user_command(bytes: &[u8; 16]) -> ClassicUserCommand {
    let short = |offset: usize| i16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    ClassicUserCommand {
        milliseconds: bytes[0],
        buttons: bytes[1],
        angle_shorts: [short(2), short(4), short(6)],
        forward_move: short(8),
        side_move: short(10),
        up_move: short(12),
        impulse: bytes[14],
        light_level: bytes[15],
    }
}

/// Write a classic user command into 16 bytes.
pub fn write_classic_user_command(bytes: &mut [u8; 16], command: &ClassicUserCommand) {
    bytes[0] = command.milliseconds;
    bytes[1] = command.buttons;
    for (axis, value) in command.angle_shorts.iter().enumerate() {
        bytes[2 + axis * 2..4 + axis * 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[8..10].copy_from_slice(&command.forward_move.to_le_bytes());
    bytes[10..12].copy_from_slice(&command.side_move.to_le_bytes());
    bytes[12..14].copy_from_slice(&command.up_move.to_le_bytes());
    bytes[14] = command.impulse;
    bytes[15] = command.light_level;
}

/// Rerelease 28-byte user command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RereleaseUserCommand {
    /// Milliseconds.
    pub milliseconds: u8,
    /// Buttons bitmask.
    pub buttons: u8,
    /// View angles.
    pub angles: Vec3,
    /// Forward move.
    pub forward_move: f32,
    /// Side move.
    pub side_move: f32,
    /// Server frame.
    pub server_frame: u32,
}

/// Rerelease user command length in bytes.
pub const RERELEASE_COMMAND_BYTES: usize = 28;

/// Read a rerelease user command from 28 bytes.
#[must_use]
pub fn read_rerelease_user_command(bytes: &[u8]) -> RereleaseUserCommand {
    let float = |offset: usize| {
        f32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
    };
    RereleaseUserCommand {
        milliseconds: bytes[0],
        buttons: bytes[1],
        angles: Vec3 {
            x: float(4),
            y: float(8),
            z: float(12),
        },
        forward_move: float(16),
        side_move: float(20),
        server_frame: u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
    }
}

/// Write a rerelease user command into 28 bytes.
pub fn write_rerelease_user_command(bytes: &mut [u8], command: &RereleaseUserCommand) {
    bytes[0] = command.milliseconds;
    bytes[1] = command.buttons;
    for (axis, value) in [command.angles.x, command.angles.y, command.angles.z]
        .into_iter()
        .enumerate()
    {
        bytes[4 + axis * 4..8 + axis * 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[16..20].copy_from_slice(&command.forward_move.to_le_bytes());
    bytes[20..24].copy_from_slice(&command.side_move.to_le_bytes());
    bytes[24..28].copy_from_slice(&command.server_frame.to_le_bytes());
}

/// Native input command in either dialect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeCommand {
    /// Classic command.
    Classic(ClassicUserCommand),
    /// Rerelease command.
    Rerelease(RereleaseUserCommand),
}

impl NativeCommand {
    /// Command milliseconds.
    #[must_use]
    pub const fn milliseconds(self) -> u8 {
        match self {
            Self::Classic(command) => command.milliseconds,
            Self::Rerelease(command) => command.milliseconds,
        }
    }
}

/// Map a native command into a world client command.
#[must_use]
pub fn client_command_from(command: &NativeCommand) -> ClientCommand {
    match command {
        NativeCommand::Classic(command) => ClientCommand {
            family: ClientFamily::Q2Classic,
            buttons: i32::from(command.buttons),
            impulse: i32::from(command.impulse),
            forward_move: f64::from(command.forward_move),
            side_move: f64::from(command.side_move),
            right_move: 0.0,
            up_move: f64::from(command.up_move),
        },
        NativeCommand::Rerelease(command) => ClientCommand {
            family: ClientFamily::Q2Rerelease,
            buttons: i32::from(command.buttons),
            impulse: 0,
            forward_move: f64::from(command.forward_move),
            side_move: f64::from(command.side_move),
            right_move: 0.0,
            up_move: 0.0,
        },
    }
}

fn clamp_pitch(pitch: f64) -> f64 {
    if pitch > 89.0 && pitch < 180.0 {
        89.0
    } else if pitch < 271.0 && pitch >= 180.0 {
        271.0
    } else {
        pitch
    }
}

/// Absolute classic aim from angle shorts, client deltas and pmove flags.
#[must_use]
pub fn classic_aim(shorts: [i16; 3], delta: [i16; 3], flags: u8) -> Vec3 {
    if u32::from(flags) & PMF_TIME_TELEPORT != 0 {
        let yaw = f64::from(shorts[1].wrapping_add(delta[1])) * 360.0 / 65536.0;
        return Vec3 {
            x: 0.0,
            y: yaw as f32,
            z: 0.0,
        };
    }
    let mut angles = [0.0f32; 3];
    for axis in 0..3 {
        let word = shorts[axis].wrapping_add(delta[axis]);
        angles[axis] = (f64::from(word) * 360.0 / 65536.0) as f32;
    }
    angles[0] = clamp_pitch(f64::from(angles[0])) as f32;
    Vec3 {
        x: angles[0],
        y: angles[1],
        z: angles[2],
    }
}

/// Absolute rerelease aim from float angles, client deltas and pmove flags.
#[must_use]
pub fn rerelease_aim(angles: Vec3, delta: Vec3, flags: u16) -> Vec3 {
    if u32::from(flags) & PMF_TIME_TELEPORT != 0 {
        return Vec3 {
            x: 0.0,
            y: angles.y + delta.y,
            z: 0.0,
        };
    }
    Vec3 {
        x: clamp_pitch(f64::from(angles.x + delta.x)) as f32,
        y: angles.y + delta.y,
        z: angles.z + delta.z,
    }
}

/// Client identity bound to one input slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputIdentity {
    /// Canonical actor.
    pub actor: NativeActorId,
    /// Client slot.
    pub slot: u32,
}

/// Application scope of one input dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationScope {
    /// Client command application.
    ClientCommand,
    /// Movement slice application.
    MovementSlice,
}

/// Application request delivered to the caller hooks.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationRequest {
    /// Client identity.
    pub identity: InputIdentity,
    /// Application scope.
    pub scope: ApplicationScope,
    /// Input command.
    pub command: NativeCommand,
    /// Absolute aim.
    pub aim: Vec3,
    /// Accepted canonical command.
    pub accepted: Option<ClientCommand>,
    /// Frame tick.
    pub frame_tick: u32,
}

/// Client movement outputs projected into a movement slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientMovement {
    /// Projected pmove type, when overridden.
    pub mode: Option<i32>,
    /// Requested body bounds override.
    pub body_bounds: Option<Bounds>,
}

/// Movement projection over one pmove state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmoveView {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Grounded.
    pub grounded: bool,
    /// View offset.
    pub view_offset: Vec3,
    /// Crouched.
    pub crouched: bool,
}

/// Input services owned by the caller.
pub struct InputServices {
    /// Whether client applications are active.
    pub applications_active: bool,
    /// Identity behind a slot.
    pub identity: Box<dyn FnMut(u32) -> Option<InputIdentity>>,
    /// Whether an identity is live.
    pub live: Box<dyn FnMut(InputIdentity) -> bool>,
    /// Accepted canonical command.
    pub accepted: Box<dyn FnMut(NativeActorId) -> Option<ClientCommand>>,
    /// Current frame tick.
    pub frame_tick: Box<dyn FnMut() -> u32>,
    /// Retired identity notice.
    pub retired: Box<dyn FnMut(InputIdentity)>,
    /// Rewrite a command before the original runs.
    pub original_command: Option<Box<dyn FnMut(InputIdentity, NativeCommand) -> NativeCommand>>,
    /// Movement outputs for a movement slice.
    pub client_outputs: Option<Box<dyn FnMut(NativeActorId) -> Option<ClientMovement>>>,
    /// Current body bounds.
    pub body_bounds: Option<Box<dyn FnMut(NativeActorId) -> Option<Bounds>>>,
    /// Begin a client application; returns an opaque token.
    pub begin_application: Box<dyn FnMut(ApplicationRequest) -> u64>,
    /// Finish a client application.
    pub finish_application: Box<dyn FnMut(u64, bool)>,
    /// Movement wrapper around a movement slice.
    pub movement: Option<Box<dyn FnMut(InputIdentity, PmoveView)>>,
}

/// Input dispatch outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// No identity claimed the slot; the original ran untouched.
    PassedThrough,
    /// The dispatch ran through applications.
    Ran,
    /// The actor retired mid-dispatch; the original was skipped cleanly.
    Swallowed,
}

/// Synthetic client table: per-slot client state plus the pending command.
pub struct SyntheticInputTable {
    /// Per-slot client state bytes.
    pub states: Vec<Vec<u8>>,
    /// Per-slot pending command bytes.
    pub commands: Vec<Vec<u8>>,
    /// Classic (16-byte commands) or rerelease (28-byte commands) encoding.
    pub classic: bool,
}

impl SyntheticInputTable {
    /// Build a table with zeroed slots.
    #[must_use]
    pub fn table(slots: usize, state_bytes: usize, classic: bool) -> Self {
        let command_bytes = if classic { 16 } else { RERELEASE_COMMAND_BYTES };
        Self {
            states: vec![vec![0u8; state_bytes]; slots],
            commands: vec![vec![0u8; command_bytes]; slots],
            classic,
        }
    }
}

/// Public ClientThink/Pmove boundaries, including calls made inside the
/// original game module.
pub struct NativeInputBinding {
    services: InputServices,
    scopes: Vec<InputIdentity>,
    current: Option<u64>,
}

impl NativeInputBinding {
    /// Build the binding over caller-owned services.
    #[must_use]
    pub fn new(services: InputServices) -> Self {
        Self {
            services,
            scopes: Vec::new(),
            current: None,
        }
    }

    fn assert_live(&mut self, identity: InputIdentity) -> HostResult<()> {
        if (self.services.live)(identity) {
            Ok(())
        } else {
            Err(NativeHostError::Retired)
        }
    }

    fn parse_command(&self, table: &SyntheticInputTable, slot: usize) -> HostResult<NativeCommand> {
        let bytes = &table.commands[slot];
        if table.classic {
            let mut fixed = [0u8; 16];
            fixed.copy_from_slice(&bytes[..16]);
            Ok(NativeCommand::Classic(read_classic_user_command(&fixed)))
        } else {
            Ok(NativeCommand::Rerelease(read_rerelease_user_command(bytes)))
        }
    }

    fn write_command(&self, bytes: &mut [u8], command: &NativeCommand) {
        match command {
            NativeCommand::Classic(command) => {
                let mut fixed = [0u8; 16];
                write_classic_user_command(&mut fixed, command);
                bytes[..16].copy_from_slice(&fixed);
            }
            NativeCommand::Rerelease(command) => write_rerelease_user_command(bytes, command),
        }
    }

    fn aim(&mut self, command: &NativeCommand, state: &[u8]) -> Vec3 {
        match command {
            NativeCommand::Classic(command) => {
                let short = |offset: usize| i16::from_le_bytes([state[offset], state[offset + 1]]);
                classic_aim(
                    command.angle_shorts,
                    [short(20), short(22), short(24)],
                    state[16],
                )
            }
            NativeCommand::Rerelease(command) => {
                let float = |offset: usize| {
                    f32::from_le_bytes([state[offset], state[offset + 1], state[offset + 2], state[offset + 3]])
                };
                rerelease_aim(
                    command.angles,
                    Vec3 {
                        x: float(36),
                        y: float(40),
                        z: float(44),
                    },
                    u16::from_le_bytes([state[28], state[29]]),
                )
            }
        }
    }

    /// Dispatch one ClientThink for a slot.
    pub fn dispatch_think(
        &mut self,
        table: &mut SyntheticInputTable,
        slot: usize,
        run_original: impl FnOnce() -> HostResult<()>,
    ) -> HostResult<DispatchOutcome> {
        let identity = (self.services.identity)(slot as u32);
        let identity = match identity {
            Some(identity) => identity,
            None => {
                run_original()?;
                return Ok(DispatchOutcome::PassedThrough);
            }
        };
        self.scopes.push(identity);
        let outcome = (|| {
            if !self.services.applications_active {
                self.assert_live(identity)?;
                run_original()?;
                self.assert_live(identity)?;
                return Ok(DispatchOutcome::Ran);
            }
            let command = self.parse_command(table, slot)?;
            let aim = self.aim(&command, &table.states[slot]);
            let accepted = (self.services.accepted)(identity.actor);
            let frame_tick = (self.services.frame_tick)();
            let command = self.apply(
                table,
                slot,
                identity,
                ApplicationScope::ClientCommand,
                command,
                aim,
                accepted,
                frame_tick,
                run_original,
            )?;
            Ok(command)
        })();
        self.scopes.pop();
        match outcome {
            Err(NativeHostError::Retired) => {
                (self.services.retired)(identity);
                Ok(DispatchOutcome::Swallowed)
            }
            outcome => outcome,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        table: &mut SyntheticInputTable,
        slot: usize,
        identity: InputIdentity,
        scope: ApplicationScope,
        command: NativeCommand,
        aim: Vec3,
        accepted: Option<ClientCommand>,
        frame_tick: u32,
        run_original: impl FnOnce() -> HostResult<()>,
    ) -> HostResult<DispatchOutcome> {
        let previous = self.current;
        let token = (self.services.begin_application)(ApplicationRequest {
            identity,
            scope,
            command,
            aim,
            accepted,
            frame_tick,
        });
        self.current = Some(token);
        let mut failed = true;
        let mut projected: Option<(i32, i32)> = None;
        let outcome = (|| {
            self.assert_live(identity)?;
            if scope == ApplicationScope::ClientCommand {
                let effective = match &self.services.original_command {
                    Some(rewrite) => rewrite(identity, command),
                    None => command,
                };
                self.write_command(&mut table.commands[slot], &effective);
            }
            if scope == ApplicationScope::MovementSlice {
                let mode = self
                    .services
                    .client_outputs
                    .as_mut()
                    .and_then(|outputs| outputs(identity.actor))
                    .and_then(|outputs| outputs.mode);
                if let Some(mode) = mode {
                    let before = i32::from_le_bytes([
                        table.states[slot][0],
                        table.states[slot][1],
                        table.states[slot][2],
                        table.states[slot][3],
                    ]);
                    projected = Some((before, mode));
                    table.states[slot][0..4].copy_from_slice(&mode.to_le_bytes());
                }
            }
            run_original()?;
            failed = false;
            (self.services.finish_application)(token, false);
            self.assert_live(identity)?;
            Ok(DispatchOutcome::Ran)
        })();
        if failed {
            (self.services.finish_application)(token, true);
        }
        if let Some((before, mode)) = projected {
            let current = i32::from_le_bytes([
                table.states[slot][0],
                table.states[slot][1],
                table.states[slot][2],
                table.states[slot][3],
            ]);
            if (self.services.live)(identity) && current == mode {
                table.states[slot][0..4].copy_from_slice(&before.to_le_bytes());
            }
        }
        self.current = previous;
        outcome
    }

    /// Dispatch one Pmove for the innermost think scope.
    pub fn dispatch_movement(
        &mut self,
        table: &mut SyntheticInputTable,
        slot: usize,
        is_player: bool,
        run_original: impl FnOnce(Option<Bounds>) -> HostResult<()>,
    ) -> HostResult<DispatchOutcome> {
        let identity = match self.scopes.last().copied() {
            Some(identity) => identity,
            None => {
                run_original(None)?;
                return Ok(DispatchOutcome::PassedThrough);
            }
        };
        self.assert_live(identity)?;
        if !table.classic && !is_player {
            run_original(None)?;
            return Ok(DispatchOutcome::PassedThrough);
        }
        if !self.services.applications_active {
            return self.run_body(identity, run_original);
        }
        let view = self.pmove_view(table, slot);
        if let Some(movement) = self.services.movement.as_mut() {
            movement(identity, view);
        }
        let command = self.parse_command(table, slot)?;
        let aim = self.aim(&command, &table.states[slot]);
        let accepted = (self.services.accepted)(identity.actor);
        let frame_tick = (self.services.frame_tick)();
        let body = self
            .services
            .body_bounds
            .as_mut()
            .and_then(|bounds| bounds(identity.actor));
        if self.services.client_outputs.is_some() && body.is_none() {
            return Err(NativeHostError::Fault(
                "native body shape lost its current actor bounds".to_string(),
            ));
        }
        self.apply(
            table,
            slot,
            identity,
            ApplicationScope::MovementSlice,
            command,
            aim,
            accepted,
            frame_tick,
            || run_original(body),
        )
    }

    fn pmove_view(&self, table: &SyntheticInputTable, slot: usize) -> PmoveView {
        let state = &table.states[slot];
        let short = |offset: usize| i16::from_le_bytes([state[offset], state[offset + 1]]) as f32 * 0.125;
        let float = |offset: usize| {
            f32::from_le_bytes([state[offset], state[offset + 1], state[offset + 2], state[offset + 3]])
        };
        let (origin, velocity, grounded, crouched) = if table.classic {
            (
                Vec3 { x: short(4), y: short(6), z: short(8) },
                Vec3 { x: short(10), y: short(12), z: short(14) },
                state[16] & 4 != 0,
                state[16] & 1 != 0,
            )
        } else {
            (
                Vec3 { x: float(4), y: float(8), z: float(12) },
                Vec3 { x: float(16), y: float(20), z: float(24) },
                u16::from_le_bytes([state[28], state[29]]) & 4 != 0,
                u16::from_le_bytes([state[28], state[29]]) & 1 != 0,
            )
        };
        PmoveView {
            origin,
            velocity,
            grounded,
            view_offset: Vec3 {
                x: float(40),
                y: float(44),
                z: float(48),
            },
            crouched,
        }
    }

    fn run_body(
        &mut self,
        identity: InputIdentity,
        run_original: impl FnOnce(Option<Bounds>) -> HostResult<()>,
    ) -> HostResult<DispatchOutcome> {
        self.assert_live(identity)?;
        let body = self
            .services
            .body_bounds
            .as_mut()
            .and_then(|bounds| bounds(identity.actor));
        run_original(body)?;
        self.assert_live(identity)?;
        Ok(DispatchOutcome::Ran)
    }

    /// Retire every scope held by an actor, releasing its identities.
    pub fn release(&mut self, actor: NativeActorId) {
        for scope in self.scopes.clone() {
            if scope.actor == actor {
                (self.services.retired)(scope);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn services(live: bool) -> InputServices {
        InputServices {
            applications_active: true,
            identity: Box::new(|slot| {
                (slot == 0).then_some(InputIdentity {
                    actor: NativeActorId { slot: 3, generation: 0 },
                    slot,
                })
            }),
            live: Box::new(move |_| live),
            accepted: Box::new(|_| None),
            frame_tick: Box::new(|| 77),
            retired: Box::new(|_| {}),
            original_command: None,
            client_outputs: None,
            body_bounds: None,
            begin_application: Box::new(|request| {
                assert_eq!(request.frame_tick, 77);
                9
            }),
            finish_application: Box::new(|_, _| {}),
            movement: None,
        }
    }

    #[test]
    fn roundtrips_command_bytes() {
        let mut bytes = [0u8; 16];
        let classic = ClassicUserCommand {
            milliseconds: 50,
            buttons: 3,
            angle_shorts: [100, -200, 300],
            forward_move: 400,
            side_move: -10,
            up_move: 20,
            impulse: 7,
            light_level: 9,
        };
        write_classic_user_command(&mut bytes, &classic);
        assert_eq!(read_classic_user_command(&bytes), classic);
        let mut wide = vec![0u8; RERELEASE_COMMAND_BYTES];
        let rerelease = RereleaseUserCommand {
            milliseconds: 16,
            buttons: 1,
            angles: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            forward_move: 4.0,
            side_move: 5.0,
            server_frame: 6,
        };
        write_rerelease_user_command(&mut wide, &rerelease);
        assert_eq!(read_rerelease_user_command(&wide), rerelease);
        let mapped = client_command_from(&NativeCommand::Classic(classic));
        assert_eq!(mapped.family, ClientFamily::Q2Classic);
        assert!(mapped.attack_pressed());
    }

    #[test]
    fn computes_absolute_aim() {
        let aim = classic_aim([0, 8192, 0], [0, 0, 0], 0);
        assert!((aim.y - 45.0).abs() < 0.01);
        let teleport = classic_aim([1000, 8192, 500], [0, 0, 0], 32);
        assert_eq!(teleport.x, 0.0);
        assert!((teleport.y - 45.0).abs() < 0.01);
        let pitch = classic_aim([20000, 0, 0], [0, 0, 0], 0);
        assert_eq!(pitch.x, 89.0);
        let modern = rerelease_aim(
            Vec3 { x: 10.0, y: 20.0, z: 0.0 },
            Vec3 { x: 1.0, y: 2.0, z: 0.0 },
            0,
        );
        assert_eq!(modern, Vec3 { x: 11.0, y: 22.0, z: 0.0 });
    }

    #[test]
    fn dispatches_think_and_movement() {
        let mut binding = NativeInputBinding::new(services(true));
        let mut table = SyntheticInputTable::table(2, 64, true);
        let command = ClassicUserCommand {
            milliseconds: 50,
            buttons: 2,
            angle_shorts: [0, 8192, 0],
            forward_move: 0,
            side_move: 0,
            up_move: 0,
            impulse: 0,
            light_level: 0,
        };
        let mut bytes = [0u8; 16];
        write_classic_user_command(&mut bytes, &command);
        table.commands[0].copy_from_slice(&bytes);
        let outcome = binding
            .dispatch_think(&mut table, 0, || Ok(()))
            .expect("think");
        assert_eq!(outcome, DispatchOutcome::Ran);
        let passed = binding
            .dispatch_think(&mut table, 1, || Ok(()))
            .expect("think");
        assert_eq!(passed, DispatchOutcome::PassedThrough);
    }

    #[test]
    fn swallows_retired_actors() {
        let mut binding = NativeInputBinding::new(services(false));
        let mut table = SyntheticInputTable::table(1, 64, true);
        let outcome = binding
            .dispatch_think(&mut table, 0, || Ok(()))
            .expect("think");
        assert_eq!(outcome, DispatchOutcome::Swallowed);
        binding.release(NativeActorId { slot: 3, generation: 0 });
    }
}
