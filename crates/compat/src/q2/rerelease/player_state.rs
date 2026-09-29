//! Q2 rerelease player-state and user-command codecs.
//!
//! Donor: `src/compat/q2/rerelease/player-state.ts` — bridges
//! `player_state_t` / `usercmd_t` bytes and host-side state records.

use qa_core::math::{Vec3, Vec4};
use qa_guest::core::contracts::{GuestAccess, GuestAddress};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use qa_world::client::{ClientCommand, ClientFamily};
use thiserror::Error;

use super::layouts::{player_state_layout, usercmd_layout};

/// Player-state codec failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlayerStateError {
    /// Too many stats for the 64-entry source array.
    #[error("Rerelease player state has more than 64 source stats")]
    TooManyStats,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// `pmove_state_t` host record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RereleaseMovementState {
    /// Movement type.
    pub move_type: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Movement flags.
    pub flags: u16,
    /// Time in milliseconds.
    pub time_milliseconds: u16,
    /// Gravity.
    pub gravity: i16,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// View height.
    pub view_height: i8,
}

/// `player_state_t` host record.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleasePlayerState {
    /// Movement state.
    pub movement: RereleaseMovementState,
    /// View angles.
    pub view_angles: Vec3,
    /// View offset.
    pub view_offset: Vec3,
    /// Kick angles.
    pub kick_angles: Vec3,
    /// Gun angles.
    pub gun_angles: Vec3,
    /// Gun offset.
    pub gun_offset: Vec3,
    /// Gun model index.
    pub gun_index: i32,
    /// Gun skin.
    pub gun_skin: i32,
    /// Gun frame.
    pub gun_frame: i32,
    /// Gun rate.
    pub gun_rate: i32,
    /// Screen blend.
    pub screen_blend: Vec4,
    /// Damage blend.
    pub damage_blend: Vec4,
    /// Field of view.
    pub fov: f32,
    /// Render flags.
    pub render_flags: u8,
    /// Stats array (up to 64).
    pub stats: Vec<i16>,
    /// Team id.
    pub team_id: u8,
}

/// `usercmd_t` host record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RereleaseUserCommand {
    /// Milliseconds.
    pub milliseconds: u8,
    /// Buttons.
    pub buttons: u8,
    /// Angles.
    pub angles: Vec3,
    /// Forward move.
    pub forward_move: f32,
    /// Side move.
    pub side_move: f32,
    /// Server frame.
    pub server_frame: u32,
}

fn write_vec3(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64, value: Vec3) -> Result<(), GuestError> {
    let at = memory.offset(base, offset)?;
    memory.write_f32(at, value.x)?;
    memory.write_f32(memory.offset(at, 4)?, value.y)?;
    memory.write_f32(memory.offset(at, 8)?, value.z)?;
    Ok(())
}

fn read_vec3(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> Result<Vec3, GuestError> {
    let at = memory.offset(base, offset)?;
    Ok(Vec3 {
        x: memory.read_f32(at)?,
        y: memory.read_f32(memory.offset(at, 4)?)?,
        z: memory.read_f32(memory.offset(at, 8)?)?,
    })
}

fn write_color(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64, value: Vec4) -> Result<(), GuestError> {
    write_vec3(
        memory,
        base,
        offset,
        Vec3 {
            x: value.x,
            y: value.y,
            z: value.z,
        },
    )?;
    memory.write_f32(memory.offset(base, offset + 12)?, value.w)?;
    Ok(())
}

fn read_color(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> Result<Vec4, GuestError> {
    let rgb = read_vec3(memory, base, offset)?;
    Ok(Vec4 {
        x: rgb.x,
        y: rgb.y,
        z: rgb.z,
        w: memory.read_f32(memory.offset(base, offset + 12)?)?,
    })
}

/// Write a movement state at a guest address (52 bytes).
pub fn write_movement_state(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    value: &RereleaseMovementState,
) -> Result<(), GuestError> {
    memory.write_i32(address, value.move_type)?;
    write_vec3(memory, address, 4, value.origin)?;
    write_vec3(memory, address, 16, value.velocity)?;
    memory.write_u16(memory.offset(address, 28)?, value.flags)?;
    memory.write_u16(memory.offset(address, 30)?, value.time_milliseconds)?;
    memory.write_i16(memory.offset(address, 32)?, value.gravity)?;
    write_vec3(memory, address, 36, value.delta_angles)?;
    memory.write_i8(memory.offset(address, 48)?, value.view_height)?;
    Ok(())
}

/// Read a movement state from a guest address.
pub fn read_movement_state(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
) -> Result<RereleaseMovementState, GuestError> {
    Ok(RereleaseMovementState {
        move_type: memory.read_i32(address)?,
        origin: read_vec3(memory, address, 4)?,
        velocity: read_vec3(memory, address, 16)?,
        flags: memory.read_u16(memory.offset(address, 28)?)?,
        time_milliseconds: memory.read_u16(memory.offset(address, 30)?)?,
        gravity: memory.read_i16(memory.offset(address, 32)?)?,
        delta_angles: read_vec3(memory, address, 36)?,
        view_height: memory.read_i8(memory.offset(address, 48)?)?,
    })
}

/// Write a player state at a guest address.
pub fn write_player_state(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    value: &RereleasePlayerState,
) -> Result<(), PlayerStateError> {
    if value.stats.len() > 64 {
        return Err(PlayerStateError::TooManyStats);
    }
    let layout = player_state_layout();
    memory.check(address, layout.byte_length, GuestAccess::Write)?;
    write_movement_state(memory, address, &value.movement)?;
    write_vec3(memory, address, 52, value.view_angles)?;
    write_vec3(memory, address, 64, value.view_offset)?;
    write_vec3(memory, address, 76, value.kick_angles)?;
    write_vec3(memory, address, 88, value.gun_angles)?;
    write_vec3(memory, address, 100, value.gun_offset)?;
    memory.write_i32(memory.offset(address, 112)?, value.gun_index)?;
    memory.write_i32(memory.offset(address, 116)?, value.gun_skin)?;
    memory.write_i32(memory.offset(address, 120)?, value.gun_frame)?;
    memory.write_i32(memory.offset(address, 124)?, value.gun_rate)?;
    write_color(memory, address, 128, value.screen_blend)?;
    write_color(memory, address, 144, value.damage_blend)?;
    memory.write_f32(memory.offset(address, 160)?, value.fov)?;
    memory.write_u8(memory.offset(address, 164)?, value.render_flags)?;
    for index in 0..64 {
        memory.write_i16(
            memory.offset(address, 166 + index * 2)?,
            value.stats.get(index as usize).copied().unwrap_or(0),
        )?;
    }
    memory.write_u8(memory.offset(address, 294)?, value.team_id)?;
    Ok(())
}

/// Read a player state from a guest address.
pub fn read_player_state(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
) -> Result<RereleasePlayerState, PlayerStateError> {
    let layout = player_state_layout();
    memory.check(address, layout.byte_length, GuestAccess::Read)?;
    let mut stats = Vec::with_capacity(64);
    for index in 0..64 {
        stats.push(memory.read_i16(memory.offset(address, 166 + index * 2)?)?);
    }
    Ok(RereleasePlayerState {
        movement: read_movement_state(memory, address)?,
        view_angles: read_vec3(memory, address, 52)?,
        view_offset: read_vec3(memory, address, 64)?,
        kick_angles: read_vec3(memory, address, 76)?,
        gun_angles: read_vec3(memory, address, 88)?,
        gun_offset: read_vec3(memory, address, 100)?,
        gun_index: memory.read_i32(memory.offset(address, 112)?)?,
        gun_skin: memory.read_i32(memory.offset(address, 116)?)?,
        gun_frame: memory.read_i32(memory.offset(address, 120)?)?,
        gun_rate: memory.read_i32(memory.offset(address, 124)?)?,
        screen_blend: read_color(memory, address, 128)?,
        damage_blend: read_color(memory, address, 144)?,
        fov: memory.read_f32(memory.offset(address, 160)?)?,
        render_flags: memory.read_u8(memory.offset(address, 164)?)?,
        stats,
        team_id: memory.read_u8(memory.offset(address, 294)?)?,
    })
}

/// Write a user command at a guest address (28 bytes).
pub fn write_user_command(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    value: &RereleaseUserCommand,
) -> Result<(), GuestError> {
    let layout = usercmd_layout();
    memory.check(address, layout.byte_length, GuestAccess::Write)?;
    memory.write_u8(address, value.milliseconds)?;
    memory.write_u8(memory.offset(address, 1)?, value.buttons)?;
    write_vec3(memory, address, 4, value.angles)?;
    memory.write_f32(memory.offset(address, 16)?, value.forward_move)?;
    memory.write_f32(memory.offset(address, 20)?, value.side_move)?;
    memory.write_u32(memory.offset(address, 24)?, value.server_frame)?;
    Ok(())
}

/// Read a user command from a guest address.
pub fn read_user_command(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
) -> Result<RereleaseUserCommand, GuestError> {
    let layout = usercmd_layout();
    memory.check(address, layout.byte_length, GuestAccess::Read)?;
    Ok(RereleaseUserCommand {
        milliseconds: memory.read_u8(address)?,
        buttons: memory.read_u8(memory.offset(address, 1)?)?,
        angles: read_vec3(memory, address, 4)?,
        forward_move: memory.read_f32(memory.offset(address, 16)?)?,
        side_move: memory.read_f32(memory.offset(address, 20)?)?,
        server_frame: memory.read_u32(memory.offset(address, 24)?)?,
    })
}

/// Convert a rerelease user command into shared client input.
#[must_use]
pub fn to_client_command(value: &RereleaseUserCommand) -> ClientCommand {
    ClientCommand {
        family: ClientFamily::Q2Rerelease,
        buttons: i32::from(value.buttons),
        impulse: 0,
        forward_move: f64::from(value.forward_move),
        side_move: f64::from(value.side_move),
        right_move: 0.0,
        up_move: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "player-state-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn sample_movement() -> RereleaseMovementState {
        RereleaseMovementState {
            move_type: 1,
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            velocity: Vec3 { x: 4.0, y: 5.0, z: 6.0 },
            flags: 7,
            time_milliseconds: 25,
            gravity: 800,
            delta_angles: Vec3 { x: 0.5, y: 1.5, z: 2.5 },
            view_height: 22,
        }
    }

    #[test]
    fn player_state_round_trips() {
        let mut memory = test_memory();
        let address = memory.allocate(&GuestAllocationOptions::bytes(296)).expect("alloc");
        let value = RereleasePlayerState {
            movement: sample_movement(),
            view_angles: Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
            view_offset: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 22.0,
            },
            kick_angles: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
            gun_angles: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            gun_offset: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            gun_index: 5,
            gun_skin: 6,
            gun_frame: 7,
            gun_rate: 8,
            screen_blend: Vec4 {
                x: 0.1,
                y: 0.2,
                z: 0.3,
                w: 0.4,
            },
            damage_blend: Vec4 {
                x: 0.5,
                y: 0.6,
                z: 0.7,
                w: 0.8,
            },
            fov: 90.0,
            render_flags: 3,
            stats: vec![11, 22, 33],
            team_id: 2,
        };
        write_player_state(&mut memory, address, &value).expect("write");
        let back = read_player_state(&mut memory, address).expect("read");
        assert_eq!(back.movement, value.movement);
        assert_eq!(back.view_angles, value.view_angles);
        assert_eq!(back.stats[0..3], [11, 22, 33]);
        assert_eq!(back.stats.len(), 64);
        assert_eq!(back.team_id, 2);
        let bad = RereleasePlayerState {
            stats: vec![0; 65],
            ..value
        };
        assert_eq!(
            write_player_state(&mut memory, address, &bad).unwrap_err(),
            PlayerStateError::TooManyStats
        );
    }

    #[test]
    fn user_command_round_trips_and_converts() {
        let mut memory = test_memory();
        let address = memory.allocate(&GuestAllocationOptions::bytes(28)).expect("alloc");
        let value = RereleaseUserCommand {
            milliseconds: 25,
            buttons: 9,
            angles: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            forward_move: 100.0,
            side_move: -50.0,
            server_frame: 1234,
        };
        write_user_command(&mut memory, address, &value).expect("write");
        let back = read_user_command(&mut memory, address).expect("read");
        assert_eq!(back, value);
        let shared = to_client_command(&value);
        assert_eq!(shared.family, ClientFamily::Q2Rerelease);
        assert_eq!(shared.buttons, 9);
        assert_eq!(shared.forward_move, 100.0);
        assert_eq!(shared.side_move, -50.0);
        assert_eq!(shared.effective_impulse(), 0);
    }
}
