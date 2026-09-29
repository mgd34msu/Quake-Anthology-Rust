//! Quake II contract/wire adapters.
//!
//! Donor provenance: `fromQ2Entity`, `toQ2Entity`,
//! `toQ2RereleaseEntity`, `fromQ2Player`, `toQ2Player`,
//! `toQ2RereleasePlayer`, `fromQ2Command`, `toQ2Command`, and
//! `toQ2RereleaseCommand` in `src/network/q2/adapters.ts`, mapping the
//! `src/contracts/protocol.ts` shapes onto the `EntityStateT` /
//! `PlayerStateT` / `UsercmdT` wire state from `src/network/q2/state.ts`.
//!
//! No Q2 contract shapes exist in `qa-core` or `qa-net` (the unified
//! [`UserCommand`](crate::common::commands::UserCommand) is the separate
//! all-`f64` layer, and `qa-core` math vectors are single-precision), so
//! the contracts are mirrored here at donor `f64` precision. Wire state
//! reuses [`EntityState`], [`PlayerState`], and [`Usercmd`](crate::q2::Usercmd),
//! and angle packing reuses [`angle_to_short`] / [`short_to_angle`].

use thiserror::Error;

use crate::q2::{
    angle_to_short, short_to_angle, EntityState, PlayerState, Usercmd, MAX_STATS_STORAGE,
};

/// Adapter failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2AdapterError {
    /// Contract stats exceed the 64 wire slots.
    #[error("Q2 wire has at most 64 player stats")]
    TooManyStats,
}

/// Contract three-vector (`Vec3` in `src/contracts/math.ts`, donor precision).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Q2Vec3 {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
}

/// Contract four-vector (`Vec4` in `src/contracts/math.ts`, donor precision).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Q2Vec4 {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
    /// W component.
    pub w: f64,
}

/// Classic entity contract (`Q2EntityState`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2EntityState {
    /// Entity number.
    pub number: u16,
    /// Origin.
    pub origin: Q2Vec3,
    /// Angles in degrees.
    pub angles: Q2Vec3,
    /// Old origin.
    pub old_origin: Q2Vec3,
    /// Model indexes.
    pub model_indexes: [u16; 4],
    /// Frame.
    pub frame: i32,
    /// Skin.
    pub skin: i32,
    /// Effects.
    pub effects: u32,
    /// Render effects.
    pub render_effects: u32,
    /// Solid.
    pub solid: u32,
    /// Sound.
    pub sound: u16,
    /// Event.
    pub event: u8,
}

/// Rerelease entity contract (`Q2RereleaseEntityState`).
///
/// The donor extends the classic shape while overriding `effects` with a
/// 64-bit value; `base.effects` is unused here and the 64-bit
/// [`effects`](Self::effects) carries both halves.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2RereleaseEntityState {
    /// Classic base fields.
    pub base: Q2EntityState,
    /// 64-bit effects (`morefx` in the high half).
    pub effects: u64,
    /// Entity alpha.
    pub alpha: f64,
    /// Entity scale.
    pub scale: f64,
    /// Instance bits.
    pub instance_bits: u8,
    /// Looping-sound volume.
    pub loop_volume: f64,
    /// Looping-sound attenuation.
    pub loop_attenuation: f64,
    /// Owner entity.
    pub owner: u16,
    /// Previous frame.
    pub old_frame: u16,
}

/// Entity contract union (`Q2EntityState | Q2RereleaseEntityState`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2Entity {
    /// Classic contract.
    Classic(Q2EntityState),
    /// Rerelease contract.
    Rerelease(Q2RereleaseEntityState),
}

/// Classic movement contract (`Q2MovementState`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2MovementState {
    /// Movement type.
    pub move_type: u8,
    /// Fixed-point origin (eighths).
    pub origin_eighths: [i32; 3],
    /// Fixed-point velocity (eighths).
    pub velocity_eighths: [i32; 3],
    /// Movement flags.
    pub flags: i32,
    /// Movement timer (eight-millisecond units).
    pub time: i32,
    /// Gravity.
    pub gravity: i16,
    /// Delta angles as wire shorts.
    pub delta_angle_shorts: [i16; 3],
}

/// Rerelease movement contract (`Q2RereleaseMovementState`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2RereleaseMovementState {
    /// Movement type.
    pub move_type: u8,
    /// Origin.
    pub origin: Q2Vec3,
    /// Velocity.
    pub velocity: Q2Vec3,
    /// Movement flags.
    pub flags: i32,
    /// Movement timer (milliseconds).
    pub time: i32,
    /// Gravity.
    pub gravity: i16,
    /// Delta angles in degrees.
    pub delta_angles: Q2Vec3,
    /// View height.
    pub view_height: i32,
}

/// Shared player view contract (the donor's private `Q2PlayerView`,
/// public here so states are constructible).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2PlayerView {
    /// View angles in degrees.
    pub view_angles: Q2Vec3,
    /// View offset.
    pub view_offset: Q2Vec3,
    /// Kick angles.
    pub kick_angles: Q2Vec3,
    /// Gun angles.
    pub gun_angles: Q2Vec3,
    /// Gun offset.
    pub gun_offset: Q2Vec3,
    /// Gun model index.
    pub gun_index: i32,
    /// Gun frame.
    pub gun_frame: i32,
    /// Field of view.
    pub fov: u8,
    /// Refresh flags.
    pub render_flags: u8,
    /// Stats (32 classic, 64 rerelease).
    pub stats: Vec<i16>,
}

/// Classic player contract (`Q2PlayerState`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2PlayerState {
    /// Shared view.
    pub view: Q2PlayerView,
    /// Movement state.
    pub movement: Q2MovementState,
    /// Screen blend.
    pub blend: Q2Vec4,
}

/// Rerelease player contract (`Q2RereleasePlayerState`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2RereleasePlayerState {
    /// Shared view.
    pub view: Q2PlayerView,
    /// Movement state.
    pub movement: Q2RereleaseMovementState,
    /// Gun skin.
    pub gun_skin: i32,
    /// Gun frame rate.
    pub gun_rate: u8,
    /// Screen blend.
    pub screen_blend: Q2Vec4,
    /// Damage blend.
    pub damage_blend: Q2Vec4,
    /// Team id.
    pub team_id: u8,
}

/// Player contract union (`Q2PlayerState | Q2RereleasePlayerState`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2Player {
    /// Classic contract.
    Classic(Q2PlayerState),
    /// Rerelease contract.
    Rerelease(Q2RereleasePlayerState),
}

/// Classic command contract (`Q2UserCommand`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2UserCommand {
    /// Milliseconds.
    pub milliseconds: u8,
    /// View angles as wire shorts.
    pub angle_shorts: [i16; 3],
    /// Forward move.
    pub forward_move: i16,
    /// Side move.
    pub side_move: i16,
    /// Up move.
    pub up_move: i16,
    /// Buttons.
    pub buttons: u8,
    /// Impulse.
    pub impulse: u8,
    /// Light level.
    pub light_level: u8,
}

/// Rerelease command contract (`Q2RereleaseUserCommand`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2RereleaseUserCommand {
    /// Milliseconds.
    pub milliseconds: u8,
    /// View angles in degrees.
    pub angles: Q2Vec3,
    /// Forward move.
    pub forward_move: i16,
    /// Side move.
    pub side_move: i16,
    /// Buttons.
    pub buttons: u8,
    /// Server frame the command was generated for.
    pub server_frame: i32,
}

/// Command contract union (`Q2UserCommand | Q2RereleaseUserCommand`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2Command {
    /// Classic contract.
    Classic(Q2UserCommand),
    /// Rerelease contract.
    Rerelease(Q2RereleaseUserCommand),
}

fn vec_to_array(value: &Q2Vec3) -> [f64; 3] {
    [value.x, value.y, value.z]
}

fn array_to_vec(value: &[f64; 3]) -> Q2Vec3 {
    Q2Vec3 {
        x: value[0],
        y: value[1],
        z: value[2],
    }
}

fn vec_to_f32(value: &Q2Vec3) -> [f32; 3] {
    [value.x as f32, value.y as f32, value.z as f32]
}

fn f32_to_vec(value: &[f32; 3]) -> Q2Vec3 {
    Q2Vec3 {
        x: f64::from(value[0]),
        y: f64::from(value[1]),
        z: f64::from(value[2]),
    }
}

fn color_to_array(value: &Q2Vec4) -> [f64; 4] {
    [value.x, value.y, value.z, value.w]
}

fn array_to_color(value: &[f64; 4]) -> Q2Vec4 {
    Q2Vec4 {
        x: value[0],
        y: value[1],
        z: value[2],
        w: value[3],
    }
}

/// Fill the classic wire fields shared by both entity contracts.
fn fill_entity(wire: &mut EntityState, state: &Q2EntityState) {
    wire.number = state.number;
    wire.origin = vec_to_array(&state.origin);
    wire.angles = vec_to_array(&state.angles);
    wire.old_origin = vec_to_array(&state.old_origin);
    [
        wire.modelindex,
        wire.modelindex2,
        wire.modelindex3,
        wire.modelindex4,
    ] = state.model_indexes;
    wire.frame = state.frame;
    wire.skinnum = state.skin;
    wire.renderfx = state.render_effects as i32;
    wire.solid = state.solid;
    wire.sound = state.sound;
    wire.event = state.event;
}

/// Convert an entity contract to wire state (`fromQ2Entity`).
#[must_use]
pub fn from_q2_entity(state: &Q2Entity) -> EntityState {
    let mut wire = EntityState::default();
    match state {
        Q2Entity::Classic(state) => {
            fill_entity(&mut wire, state);
            wire.effects = state.effects as i32;
        }
        Q2Entity::Rerelease(state) => {
            fill_entity(&mut wire, &state.base);
            wire.effects = state.effects as u32 as i32;
            wire.morefx = (state.effects >> 32) as u32 as i32;
            wire.alpha = state.alpha;
            wire.scale = state.scale;
            wire.instance_bits = state.instance_bits;
            wire.loop_volume = state.loop_volume;
            wire.loop_attenuation = state.loop_attenuation;
            wire.owner = state.owner;
            wire.old_frame = state.old_frame;
        }
    }
    wire
}

/// Convert wire state to the classic entity contract (`toQ2Entity`).
#[must_use]
pub fn to_q2_entity(wire: &EntityState) -> Q2EntityState {
    Q2EntityState {
        number: wire.number,
        origin: array_to_vec(&wire.origin),
        angles: array_to_vec(&wire.angles),
        old_origin: array_to_vec(&wire.old_origin),
        model_indexes: [wire.modelindex, wire.modelindex2, wire.modelindex3, wire.modelindex4],
        frame: wire.frame,
        skin: wire.skinnum,
        effects: wire.effects as u32,
        render_effects: wire.renderfx as u32,
        solid: wire.solid,
        sound: wire.sound,
        event: wire.event,
    }
}

/// Convert wire state to the rerelease entity contract (`toQ2RereleaseEntity`).
#[must_use]
pub fn to_q2_rerelease_entity(wire: &EntityState) -> Q2RereleaseEntityState {
    Q2RereleaseEntityState {
        base: to_q2_entity(wire),
        effects: u64::from(wire.effects as u32) | (u64::from(wire.morefx as u32) << 32),
        alpha: wire.alpha,
        scale: wire.scale,
        instance_bits: wire.instance_bits,
        loop_volume: wire.loop_volume,
        loop_attenuation: wire.loop_attenuation,
        owner: wire.owner,
        old_frame: wire.old_frame,
    }
}

/// Read the shared player view (`playerView`).
fn player_view(wire: &PlayerState) -> Q2PlayerView {
    Q2PlayerView {
        view_angles: array_to_vec(&wire.viewangles),
        view_offset: array_to_vec(&wire.viewoffset),
        kick_angles: array_to_vec(&wire.kick_angles),
        gun_angles: array_to_vec(&wire.gunangles),
        gun_offset: array_to_vec(&wire.gunoffset),
        gun_index: wire.gunindex,
        gun_frame: wire.gunframe,
        fov: wire.fov,
        render_flags: wire.rdflags,
        stats: wire.stats.to_vec(),
    }
}

/// Convert a player contract to wire state (`fromQ2Player`).
pub fn from_q2_player(state: &Q2Player) -> Result<PlayerState, Q2AdapterError> {
    let mut wire = PlayerState::default();
    let view = match state {
        Q2Player::Classic(state) => &state.view,
        Q2Player::Rerelease(state) => &state.view,
    };
    wire.viewangles = vec_to_array(&view.view_angles);
    wire.viewoffset = vec_to_array(&view.view_offset);
    wire.kick_angles = vec_to_array(&view.kick_angles);
    wire.gunangles = vec_to_array(&view.gun_angles);
    wire.gunoffset = vec_to_array(&view.gun_offset);
    wire.gunindex = view.gun_index;
    wire.gunframe = view.gun_frame;
    wire.fov = view.fov;
    wire.rdflags = view.render_flags;
    if view.stats.len() > MAX_STATS_STORAGE {
        return Err(Q2AdapterError::TooManyStats);
    }
    wire.stats[..view.stats.len()].copy_from_slice(&view.stats);
    match state {
        Q2Player::Classic(state) => {
            wire.pmove.pm_type = state.movement.move_type;
            wire.pmove.pm_flags = state.movement.flags;
            wire.pmove.gravity = state.movement.gravity;
            wire.pmove.origin = state.movement.origin_eighths;
            wire.pmove.velocity = state.movement.velocity_eighths;
            wire.pmove.delta_angles = state.movement.delta_angle_shorts;
            wire.pmove.pm_time = state.movement.time;
            wire.blend = color_to_array(&state.blend);
        }
        Q2Player::Rerelease(state) => {
            wire.pmove.pm_type = state.movement.move_type;
            wire.pmove.pm_flags = state.movement.flags;
            wire.pmove.gravity = state.movement.gravity;
            wire.pmove.delta_angles_f = vec_to_f32(&state.movement.delta_angles);
            wire.pmove.delta_angle_float = true;
            wire.pmove.origin_f = vec_to_f32(&state.movement.origin);
            wire.pmove.velocity_f = vec_to_f32(&state.movement.velocity);
            wire.pmove.pm_time = state.movement.time;
            wire.pmove.viewheight = state.movement.view_height;
            wire.pmove.delta_angles = [
                angle_to_short(state.movement.delta_angles.x) as i16,
                angle_to_short(state.movement.delta_angles.y) as i16,
                angle_to_short(state.movement.delta_angles.z) as i16,
            ];
            wire.blend = color_to_array(&state.screen_blend);
            wire.damage_blend = color_to_array(&state.damage_blend);
            wire.gunskin = state.gun_skin;
            wire.gunrate = state.gun_rate;
            wire.team_id = state.team_id;
        }
    }
    Ok(wire)
}

/// Convert wire state to the classic player contract (`toQ2Player`).
#[must_use]
pub fn to_q2_player(wire: &PlayerState) -> Q2PlayerState {
    let mut view = player_view(wire);
    view.stats.truncate(32);
    Q2PlayerState {
        view,
        movement: Q2MovementState {
            move_type: wire.pmove.pm_type,
            origin_eighths: wire.pmove.origin,
            velocity_eighths: wire.pmove.velocity,
            flags: wire.pmove.pm_flags,
            time: wire.pmove.pm_time,
            gravity: wire.pmove.gravity,
            delta_angle_shorts: wire.pmove.delta_angles,
        },
        blend: array_to_color(&wire.blend),
    }
}

/// Convert wire state to the rerelease player contract (`toQ2RereleasePlayer`).
#[must_use]
pub fn to_q2_rerelease_player(wire: &PlayerState) -> Q2RereleasePlayerState {
    Q2RereleasePlayerState {
        view: player_view(wire),
        movement: Q2RereleaseMovementState {
            move_type: wire.pmove.pm_type,
            origin: f32_to_vec(&wire.pmove.origin_f),
            velocity: f32_to_vec(&wire.pmove.velocity_f),
            flags: wire.pmove.pm_flags,
            time: wire.pmove.pm_time,
            gravity: wire.pmove.gravity,
            delta_angles: if wire.pmove.delta_angle_float {
                f32_to_vec(&wire.pmove.delta_angles_f)
            } else {
                Q2Vec3 {
                    x: short_to_angle(wire.pmove.delta_angles[0]),
                    y: short_to_angle(wire.pmove.delta_angles[1]),
                    z: short_to_angle(wire.pmove.delta_angles[2]),
                }
            },
            view_height: wire.pmove.viewheight,
        },
        gun_skin: wire.gunskin,
        gun_rate: wire.gunrate,
        screen_blend: array_to_color(&wire.blend),
        damage_blend: array_to_color(&wire.damage_blend),
        team_id: wire.team_id,
    }
}

/// Convert a command contract to wire state (`fromQ2Command`).
#[must_use]
pub fn from_q2_command(command: &Q2Command) -> Usercmd {
    let mut wire = Usercmd::default();
    match command {
        Q2Command::Classic(command) => {
            wire.msec = command.milliseconds;
            wire.angles = command.angle_shorts;
            wire.forwardmove = command.forward_move;
            wire.sidemove = command.side_move;
            wire.upmove = command.up_move;
            wire.buttons = command.buttons;
            wire.impulse = command.impulse;
            wire.lightlevel = command.light_level;
        }
        Q2Command::Rerelease(command) => {
            wire.msec = command.milliseconds;
            wire.angles = [
                angle_to_short(command.angles.x) as i16,
                angle_to_short(command.angles.y) as i16,
                angle_to_short(command.angles.z) as i16,
            ];
            wire.forwardmove = command.forward_move;
            wire.sidemove = command.side_move;
            wire.buttons = command.buttons;
            wire.server_frame = command.server_frame;
        }
    }
    wire
}

/// Convert wire state to the classic command contract (`toQ2Command`).
#[must_use]
pub fn to_q2_command(wire: &Usercmd) -> Q2UserCommand {
    Q2UserCommand {
        milliseconds: wire.msec,
        angle_shorts: wire.angles,
        forward_move: wire.forwardmove,
        side_move: wire.sidemove,
        up_move: wire.upmove,
        buttons: wire.buttons,
        impulse: wire.impulse,
        light_level: wire.lightlevel,
    }
}

/// Convert wire state to the rerelease command contract (`toQ2RereleaseCommand`).
#[must_use]
pub fn to_q2_rerelease_command(wire: &Usercmd, server_frame: i32) -> Q2RereleaseUserCommand {
    Q2RereleaseUserCommand {
        milliseconds: wire.msec,
        angles: Q2Vec3 {
            x: short_to_angle(wire.angles[0]),
            y: short_to_angle(wire.angles[1]),
            z: short_to_angle(wire.angles[2]),
        },
        forward_move: wire.forwardmove,
        side_move: wire.sidemove,
        buttons: wire.buttons,
        server_frame,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity_state() -> Q2EntityState {
        Q2EntityState {
            number: 7,
            origin: Q2Vec3 { x: 1.5, y: -2.5, z: 100.0 },
            angles: Q2Vec3 { x: 0.0, y: 90.0, z: 0.0 },
            old_origin: Q2Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            model_indexes: [1, 2, 3, 4],
            frame: 9,
            skin: 2,
            effects: 0x8000_0001,
            render_effects: 0xFFFF_FFFE,
            solid: 6210,
            sound: 44,
            event: 3,
        }
    }

    #[test]
    fn classic_entity_round_trip() {
        let state = entity_state();
        let wire = from_q2_entity(&Q2Entity::Classic(state.clone()));
        assert_eq!(wire.number, 7);
        assert_eq!(wire.origin, [1.5, -2.5, 100.0]);
        assert_eq!(wire.modelindex4, 4);
        assert_eq!(wire.effects as u32, 0x8000_0001);
        assert_eq!(wire.renderfx as u32, 0xFFFF_FFFE);
        assert_eq!(to_q2_entity(&wire), state);
    }

    #[test]
    fn rerelease_entity_splits_effects() {
        let state = Q2RereleaseEntityState {
            base: entity_state(),
            effects: 0x0000_0002_0000_0003,
            alpha: 0.5,
            scale: 2.0,
            instance_bits: 9,
            loop_volume: 0.75,
            loop_attenuation: -1.0,
            owner: 12,
            old_frame: 34,
        };
        let wire = from_q2_entity(&Q2Entity::Rerelease(state));
        assert_eq!(wire.effects, 3);
        assert_eq!(wire.morefx, 2);
        assert_eq!(wire.alpha, 0.5);
        assert_eq!(wire.owner, 12);
        let back = to_q2_rerelease_entity(&wire);
        assert_eq!(back.effects, 0x0000_0002_0000_0003);
        assert_eq!(back.loop_attenuation, -1.0);
        assert_eq!(back.base.number, 7);
    }

    fn player_view_fixture() -> Q2PlayerView {
        Q2PlayerView {
            view_angles: Q2Vec3 { x: 10.0, y: 20.0, z: 30.0 },
            view_offset: Q2Vec3 { x: 0.0, y: 0.0, z: 22.0 },
            kick_angles: Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            gun_angles: Q2Vec3 { x: 4.0, y: 5.0, z: 6.0 },
            gun_offset: Q2Vec3 { x: 7.0, y: 8.0, z: 9.0 },
            gun_index: 11,
            gun_frame: 13,
            fov: 90,
            render_flags: 2,
            stats: vec![1, 2, 3],
        }
    }

    #[test]
    fn classic_player_round_trip() {
        let state = Q2PlayerState {
            view: player_view_fixture(),
            movement: Q2MovementState {
                move_type: 4,
                origin_eighths: [80, -160, 240],
                velocity_eighths: [1, 2, 3],
                flags: 7,
                time: 33,
                gravity: 800,
                delta_angle_shorts: [100, 200, 300],
            },
            blend: Q2Vec4 { x: 0.1, y: 0.2, z: 0.3, w: 0.4 },
        };
        let wire = from_q2_player(&Q2Player::Classic(state.clone())).unwrap();
        assert_eq!(wire.pmove.origin, [80, -160, 240]);
        assert_eq!(wire.pmove.gravity, 800);
        assert_eq!(wire.stats[2], 3);
        assert_eq!(wire.stats[3], 0);
        let back = to_q2_player(&wire);
        assert_eq!(back.movement, state.movement);
        assert_eq!(back.blend, state.blend);
        let mut stats = vec![0; 32];
        stats[..3].copy_from_slice(&[1, 2, 3]);
        assert_eq!(back.view.stats, stats);
    }

    #[test]
    fn rerelease_player_uses_float_delta_angles() {
        let state = Q2RereleasePlayerState {
            view: player_view_fixture(),
            movement: Q2RereleaseMovementState {
                move_type: 4,
                origin: Q2Vec3 { x: 10.0, y: -20.0, z: 30.0 },
                velocity: Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                flags: 7,
                time: 120,
                gravity: 800,
                delta_angles: Q2Vec3 { x: 90.0, y: 180.0, z: 0.0 },
                view_height: 22,
            },
            gun_skin: 5,
            gun_rate: 40,
            screen_blend: Q2Vec4 { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },
            damage_blend: Q2Vec4 { x: 1.0, y: 0.0, z: 0.0, w: 0.5 },
            team_id: 2,
        };
        let wire = from_q2_player(&Q2Player::Rerelease(state)).unwrap();
        assert!(wire.pmove.delta_angle_float);
        assert_eq!(wire.pmove.delta_angles, [16384, -32768, 0]);
        assert_eq!(wire.pmove.origin_f, [10.0, -20.0, 30.0]);
        let back = to_q2_rerelease_player(&wire);
        assert_eq!(back.movement.delta_angles.x, 90.0);
        assert_eq!(back.team_id, 2);
    }

    #[test]
    fn rerelease_player_falls_back_to_short_angles() {
        let mut wire = PlayerState::default();
        wire.pmove.delta_angles = [16384, 0, -16384];
        let back = to_q2_rerelease_player(&wire);
        assert_eq!(back.movement.delta_angles.x, 90.0);
        assert_eq!(back.movement.delta_angles.z, -90.0);
    }

    #[test]
    fn too_many_stats_fails() {
        let mut view = player_view_fixture();
        view.stats = vec![0; 65];
        let state = Q2PlayerState {
            view,
            movement: Q2MovementState::default(),
            blend: Q2Vec4::default(),
        };
        assert_eq!(
            from_q2_player(&Q2Player::Classic(state)),
            Err(Q2AdapterError::TooManyStats)
        );
    }

    #[test]
    fn classic_command_round_trip() {
        let command = Q2UserCommand {
            milliseconds: 50,
            angle_shorts: [1000, -2000, 3000],
            forward_move: 100,
            side_move: -50,
            up_move: 25,
            buttons: 3,
            impulse: 7,
            light_level: 128,
        };
        let wire = from_q2_command(&Q2Command::Classic(command.clone()));
        assert_eq!(wire.server_frame, 0);
        assert_eq!(to_q2_command(&wire), command);
    }

    #[test]
    fn rerelease_command_packs_angles() {
        let command = Q2RereleaseUserCommand {
            milliseconds: 50,
            angles: Q2Vec3 { x: 90.0, y: 180.0, z: 0.0 },
            forward_move: 100,
            side_move: -50,
            buttons: 3,
            server_frame: 1234,
        };
        let wire = from_q2_command(&Q2Command::Rerelease(command));
        assert_eq!(wire.angles, [16384, -32768, 0]);
        assert_eq!(wire.server_frame, 1234);
        let back = to_q2_rerelease_command(&wire, 1234);
        assert_eq!(back.angles.x, 90.0);
        assert_eq!(back.angles.y, -180.0);
        assert_eq!(back.server_frame, 1234);
    }
}
