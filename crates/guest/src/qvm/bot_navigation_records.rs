//! Q3 botlib VM record layouts: goals, movement, and AAS entity state.
//!
//! Provenance: `src/compat/qvm/bot-navigation-records.ts` (Q3 botlib VM
//! records, derived from id Software and quake-3-ts). The donor's live
//! getter/setter references become owned reads plus explicit writes; the
//! byte layouts are unchanged.

use qa_core::math::Vec3;

use crate::error::GuestError;

/// Byte length of `bot_goal_t`.
pub const QVM_BOT_GOAL_BYTES: usize = 56;
/// Byte length of `bot_initmove_t`.
pub const QVM_BOT_INIT_MOVE_BYTES: usize = 68;
/// Byte length of `bot_moveresult_t`.
pub const QVM_BOT_MOVE_RESULT_BYTES: usize = 52;
/// Byte length of `bot_entitystate_t`.
pub const QVM_BOT_ENTITY_STATE_BYTES: usize = 112;
/// Byte length of `aas_entityinfo_t`.
pub const QVM_AAS_ENTITY_INFO_BYTES: usize = 140;

/// Bot goal record.
#[derive(Debug, Clone, PartialEq)]
pub struct BotGoal {
    /// Goal origin.
    pub origin: Vec3,
    /// Goal area number.
    pub area: i32,
    /// Goal bounds minimum.
    pub mins: Vec3,
    /// Goal bounds maximum.
    pub maxs: Vec3,
    /// Goal entity number.
    pub entity: i32,
    /// Goal number.
    pub number: i32,
    /// Goal flags.
    pub flags: i32,
    /// Item info.
    pub item_info: i32,
}

/// Which goal fields a query writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalWriteFields {
    /// All fields.
    Full,
    /// Level-item query fields (stops before `item_info`).
    LevelItem,
    /// Map-location query fields (stops before `number`).
    Location,
}

/// Movement initialization record.
#[derive(Debug, Clone, PartialEq)]
pub struct BotInitMove {
    /// Start origin.
    pub origin: Vec3,
    /// Start velocity.
    pub velocity: Vec3,
    /// View offset.
    pub view_offset: Vec3,
    /// Entity number.
    pub entity_num: i32,
    /// Client number.
    pub client: i32,
    /// Think time.
    pub think_time: f32,
    /// Presence type.
    pub presence_type: i32,
    /// View angles.
    pub view_angles: Vec3,
    /// OR-ed movement flags.
    pub or_move_flags: i32,
}

/// Movement result record.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BotMoveResult {
    /// Movement failed.
    pub failure: bool,
    /// Movement type.
    pub move_type: i32,
    /// Movement blocked.
    pub blocked: bool,
    /// Blocking entity.
    pub block_entity: i32,
    /// Travel type.
    pub travel_type: i32,
    /// Result flags.
    pub flags: i32,
    /// Weapon.
    pub weapon: i32,
    /// Move direction.
    pub move_direction: Vec3,
    /// Ideal view angles.
    pub ideal_view_angles: Vec3,
}

/// Bot-visible entity state update.
#[derive(Debug, Clone, PartialEq)]
pub struct BotEntityUpdate {
    /// Entity type.
    pub entity_type: i32,
    /// Entity flags.
    pub flags: i32,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Previous origin.
    pub old_origin: Vec3,
    /// Bounds minimum.
    pub mins: Vec3,
    /// Bounds maximum.
    pub maxs: Vec3,
    /// Ground entity.
    pub ground_entity: i32,
    /// Solid encoding.
    pub solid: i32,
    /// Model index.
    pub model_index: i32,
    /// Second model index.
    pub model_index2: i32,
    /// Frame.
    pub frame: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parameter: i32,
    /// Powerups.
    pub powerups: i32,
    /// Weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_animation: i32,
    /// Torso animation.
    pub torso_animation: i32,
}

/// AAS entity info record: an update plus query metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct AasEntityInfo {
    /// Whether the info is valid.
    pub valid: bool,
    /// Entity number.
    pub number: i32,
    /// Entity state fields.
    pub update: BotEntityUpdate,
    /// Last visible origin.
    pub last_visible_origin: Vec3,
    /// Last update time.
    pub last_update_time: f32,
    /// Update interval.
    pub update_interval: f32,
}

fn require(view: &[u8], length: usize, record: &'static str) -> Result<(), GuestError> {
    if view.len() < length {
        return Err(GuestError::abi(format!(
            "{record} record requires {length} bytes, received {}",
            view.len()
        )));
    }
    Ok(())
}

fn read_i32(view: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(view[offset..offset + 4].try_into().expect("checked record"))
}

fn read_f32(view: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(view[offset..offset + 4].try_into().expect("checked record"))
}

fn read_vec3(view: &[u8], offset: usize) -> Vec3 {
    Vec3 {
        x: read_f32(view, offset),
        y: read_f32(view, offset + 4),
        z: read_f32(view, offset + 8),
    }
}

fn write_i32(view: &mut [u8], offset: usize, value: i32) {
    view[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_f32(view: &mut [u8], offset: usize, value: f32) {
    view[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_vec3(view: &mut [u8], offset: usize, value: &Vec3) {
    write_f32(view, offset, value.x);
    write_f32(view, offset + 4, value.y);
    write_f32(view, offset + 8, value.z);
}

/// Read an owned goal from a source record.
pub fn read_bot_goal(view: &[u8]) -> Result<BotGoal, GuestError> {
    require(view, QVM_BOT_GOAL_BYTES, "QVM bot_goal_t")?;
    Ok(BotGoal {
        origin: read_vec3(view, 0),
        area: read_i32(view, 12),
        mins: read_vec3(view, 16),
        maxs: read_vec3(view, 28),
        entity: read_i32(view, 40),
        number: read_i32(view, 44),
        flags: read_i32(view, 48),
        item_info: read_i32(view, 52),
    })
}

/// Write a goal; queries write only their source fields.
pub fn write_bot_goal(view: &mut [u8], goal: &BotGoal, fields: GoalWriteFields) -> Result<(), GuestError> {
    let needed = match fields {
        GoalWriteFields::Full => QVM_BOT_GOAL_BYTES,
        GoalWriteFields::LevelItem => 52,
        GoalWriteFields::Location => 44,
    };
    require(view, needed, "QVM bot_goal_t")?;
    write_i32(view, 12, goal.area);
    write_vec3(view, 0, &goal.origin);
    write_i32(view, 40, goal.entity);
    write_vec3(view, 16, &goal.mins);
    write_vec3(view, 28, &goal.maxs);
    if fields == GoalWriteFields::Location {
        return Ok(());
    }
    write_i32(view, 44, goal.number);
    write_i32(view, 48, goal.flags);
    if fields == GoalWriteFields::Full {
        write_i32(view, 52, goal.item_info);
    }
    Ok(())
}

/// Read an owned movement-initialization record.
pub fn read_bot_init_move(view: &[u8]) -> Result<BotInitMove, GuestError> {
    require(view, QVM_BOT_INIT_MOVE_BYTES, "QVM bot movement record")?;
    Ok(BotInitMove {
        origin: read_vec3(view, 0),
        velocity: read_vec3(view, 12),
        view_offset: read_vec3(view, 24),
        entity_num: read_i32(view, 36),
        client: read_i32(view, 40),
        think_time: read_f32(view, 44),
        presence_type: read_i32(view, 48),
        view_angles: read_vec3(view, 52),
        or_move_flags: read_i32(view, 64),
    })
}

/// Read an owned movement-result record.
pub fn read_bot_move_result(view: &[u8]) -> Result<BotMoveResult, GuestError> {
    require(view, QVM_BOT_MOVE_RESULT_BYTES, "QVM bot movement record")?;
    Ok(BotMoveResult {
        failure: read_i32(view, 0) != 0,
        move_type: read_i32(view, 4),
        blocked: read_i32(view, 8) != 0,
        block_entity: read_i32(view, 12),
        travel_type: read_i32(view, 16),
        flags: read_i32(view, 20),
        weapon: read_i32(view, 24),
        move_direction: read_vec3(view, 28),
        ideal_view_angles: read_vec3(view, 40),
    })
}

/// Write a movement-result record.
pub fn write_bot_move_result(view: &mut [u8], result: &BotMoveResult) -> Result<(), GuestError> {
    require(view, QVM_BOT_MOVE_RESULT_BYTES, "QVM bot movement record")?;
    write_i32(view, 0, i32::from(result.failure));
    write_i32(view, 4, result.move_type);
    write_i32(view, 8, i32::from(result.blocked));
    write_i32(view, 12, result.block_entity);
    write_i32(view, 16, result.travel_type);
    write_i32(view, 20, result.flags);
    write_i32(view, 24, result.weapon);
    write_vec3(view, 28, &result.move_direction);
    write_vec3(view, 40, &result.ideal_view_angles);
    Ok(())
}

/// Read an owned entity-state update.
pub fn read_bot_entity_state(view: &[u8]) -> Result<BotEntityUpdate, GuestError> {
    require(view, QVM_BOT_ENTITY_STATE_BYTES, "QVM bot_entitystate_t")?;
    Ok(BotEntityUpdate {
        entity_type: read_i32(view, 0),
        flags: read_i32(view, 4),
        origin: read_vec3(view, 8),
        angles: read_vec3(view, 20),
        old_origin: read_vec3(view, 32),
        mins: read_vec3(view, 44),
        maxs: read_vec3(view, 56),
        ground_entity: read_i32(view, 68),
        solid: read_i32(view, 72),
        model_index: read_i32(view, 76),
        model_index2: read_i32(view, 80),
        frame: read_i32(view, 84),
        event: read_i32(view, 88),
        event_parameter: read_i32(view, 92),
        powerups: read_i32(view, 96),
        weapon: read_i32(view, 100),
        legs_animation: read_i32(view, 104),
        torso_animation: read_i32(view, 108),
    })
}

/// Write an AAS entity-info record.
pub fn write_aas_entity_info(view: &mut [u8], info: &AasEntityInfo) -> Result<(), GuestError> {
    require(view, QVM_AAS_ENTITY_INFO_BYTES, "QVM aas_entityinfo_t")?;
    let update = &info.update;
    write_i32(view, 0, i32::from(info.valid));
    write_i32(view, 4, update.entity_type);
    write_i32(view, 8, update.flags);
    write_f32(view, 12, info.last_update_time);
    write_f32(view, 16, info.update_interval);
    write_i32(view, 20, info.number);
    write_vec3(view, 24, &update.origin);
    write_vec3(view, 36, &update.angles);
    write_vec3(view, 48, &update.old_origin);
    write_vec3(view, 60, &info.last_visible_origin);
    write_vec3(view, 72, &update.mins);
    write_vec3(view, 84, &update.maxs);
    write_i32(view, 96, update.ground_entity);
    write_i32(view, 100, update.solid);
    write_i32(view, 104, update.model_index);
    write_i32(view, 108, update.model_index2);
    write_i32(view, 112, update.frame);
    write_i32(view, 116, update.event);
    write_i32(view, 120, update.event_parameter);
    write_i32(view, 124, update.powerups);
    write_i32(view, 128, update.weapon);
    write_i32(view, 132, update.legs_animation);
    write_i32(view, 136, update.torso_animation);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal() -> BotGoal {
        BotGoal {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            area: 11,
            mins: Vec3 {
                x: -1.0,
                y: -2.0,
                z: -3.0,
            },
            maxs: Vec3 { x: 4.0, y: 5.0, z: 6.0 },
            entity: 7,
            number: 8,
            flags: 9,
            item_info: 10,
        }
    }

    #[test]
    fn goal_round_trip() {
        let mut bytes = vec![0u8; QVM_BOT_GOAL_BYTES];
        write_bot_goal(&mut bytes, &goal(), GoalWriteFields::Full).unwrap();
        assert_eq!(read_bot_goal(&bytes).unwrap(), goal());
    }

    #[test]
    fn goal_short_read_fails() {
        assert!(read_bot_goal(&[0u8; 55]).is_err());
        let mut bytes = vec![0u8; 55];
        assert!(write_bot_goal(&mut bytes, &goal(), GoalWriteFields::Full).is_err());
    }

    #[test]
    fn goal_partial_writes_leave_tail() {
        let goal = goal();
        let mut bytes = vec![0xAAu8; QVM_BOT_GOAL_BYTES];
        write_bot_goal(&mut bytes, &goal, GoalWriteFields::Location).unwrap();
        assert_eq!(&bytes[44..], &[0xAAu8; 12]);
        assert_eq!(read_bot_goal(&bytes).unwrap().area, 11);
        let mut bytes = vec![0xAAu8; QVM_BOT_GOAL_BYTES];
        write_bot_goal(&mut bytes, &goal, GoalWriteFields::LevelItem).unwrap();
        assert_eq!(&bytes[52..], &[0xAAu8; 4]);
        assert_eq!(read_bot_goal(&bytes).unwrap().flags, 9);
    }

    #[test]
    fn init_move_layout() {
        let mut bytes = vec![0u8; QVM_BOT_INIT_MOVE_BYTES];
        bytes[36..40].copy_from_slice(&17i32.to_le_bytes());
        bytes[44..48].copy_from_slice(&0.5f32.to_le_bytes());
        bytes[64..68].copy_from_slice(&3i32.to_le_bytes());
        let parsed = read_bot_init_move(&bytes).unwrap();
        assert_eq!(parsed.entity_num, 17);
        assert_eq!(parsed.think_time, 0.5);
        assert_eq!(parsed.or_move_flags, 3);
        assert!(read_bot_init_move(&bytes[..67]).is_err());
    }

    #[test]
    fn move_result_round_trip() {
        let result = BotMoveResult {
            failure: true,
            move_type: 2,
            blocked: true,
            block_entity: 5,
            travel_type: 6,
            flags: 7,
            weapon: 8,
            move_direction: Vec3 {
                x: 1.0,
                y: 0.0,
                z: -1.0,
            },
            ideal_view_angles: Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
        };
        let mut bytes = vec![0u8; QVM_BOT_MOVE_RESULT_BYTES];
        write_bot_move_result(&mut bytes, &result).unwrap();
        assert_eq!(read_bot_move_result(&bytes).unwrap(), result);
        assert!(read_bot_move_result(&bytes[..51]).is_err());
    }

    #[test]
    fn entity_state_layout() {
        let mut bytes = vec![0u8; QVM_BOT_ENTITY_STATE_BYTES];
        bytes[0..4].copy_from_slice(&4i32.to_le_bytes());
        bytes[68..72].copy_from_slice(&9i32.to_le_bytes());
        bytes[108..112].copy_from_slice(&12i32.to_le_bytes());
        let parsed = read_bot_entity_state(&bytes).unwrap();
        assert_eq!(parsed.entity_type, 4);
        assert_eq!(parsed.ground_entity, 9);
        assert_eq!(parsed.torso_animation, 12);
        assert!(read_bot_entity_state(&bytes[..111]).is_err());
    }

    fn update() -> BotEntityUpdate {
        BotEntityUpdate {
            entity_type: 1,
            flags: 2,
            origin: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            angles: Vec3 { x: 2.0, y: 2.0, z: 2.0 },
            old_origin: Vec3 { x: 3.0, y: 3.0, z: 3.0 },
            mins: Vec3 {
                x: -1.0,
                y: -1.0,
                z: -1.0,
            },
            maxs: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            ground_entity: 5,
            solid: 6,
            model_index: 7,
            model_index2: 8,
            frame: 9,
            event: 10,
            event_parameter: 11,
            powerups: 12,
            weapon: 13,
            legs_animation: 14,
            torso_animation: 15,
        }
    }

    #[test]
    fn aas_entity_info_layout() {
        let info = AasEntityInfo {
            valid: true,
            number: 42,
            update: update(),
            last_visible_origin: Vec3 { x: 9.0, y: 8.0, z: 7.0 },
            last_update_time: 1.25,
            update_interval: 0.1,
        };
        let mut bytes = vec![0u8; QVM_AAS_ENTITY_INFO_BYTES];
        write_aas_entity_info(&mut bytes, &info).unwrap();
        assert_eq!(read_i32(&bytes, 0), 1);
        assert_eq!(read_i32(&bytes, 20), 42);
        assert_eq!(read_f32(&bytes, 12), 1.25);
        assert_eq!(read_f32(&bytes, 16), 0.1);
        assert_eq!(read_vec3(&bytes, 60), info.last_visible_origin);
        assert_eq!(read_vec3(&bytes, 24), info.update.origin);
        assert_eq!(read_i32(&bytes, 136), 15);
        let mut short = vec![0u8; 139];
        assert!(write_aas_entity_info(&mut short, &info).is_err());
    }
}
