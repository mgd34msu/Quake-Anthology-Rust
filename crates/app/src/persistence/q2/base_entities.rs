//! Quake II base-entities checkpoint ported from `src/persistence/q2-base-entities.ts`.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, num, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;
use super::movers::{read_q2_linear_motion_checkpoint, write_q2_linear_motion_checkpoint, Q2LinearMotionEntry};

/// Saved platform state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlatformState {
    /// Top.
    pub top: Vec3,
    /// Bottom.
    pub bottom: Vec3,
    /// Phase.
    pub phase: String,
}

/// Saved secret-door state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SecretState {
    /// First position.
    pub first: Vec3,
    /// Second position.
    pub second: Vec3,
    /// Home position.
    pub home: Vec3,
    /// Shootable flag.
    pub shootable: bool,
    /// Blocked time.
    pub blocked_time: f64,
    /// Message time.
    pub message_time: f64,
}

/// Saved turret-breach state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BreachState {
    /// Goal.
    pub goal: Vec3,
    /// Muzzle.
    pub muzzle: Vec3,
    /// Pitch maximum.
    pub pitch_max: f64,
    /// Pitch minimum.
    pub pitch_min: f64,
    /// Yaw minimum.
    pub yaw_min: f64,
    /// Yaw maximum.
    pub yaw_max: f64,
}

/// Saved turret driver.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TurretDriver {
    /// Actor.
    pub actor: SavedActorId,
    /// Breach.
    pub breach: Option<SavedActorId>,
    /// Radius.
    pub radius: f64,
    /// Yaw offset.
    pub yaw_offset: f64,
    /// Height.
    pub height: f64,
    /// Monster die callback.
    pub monster_die: String,
}

/// Q2 base-entities checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BaseEntitiesCheckpoint {
    /// Platforms.
    pub platforms: Vec<(SavedActorId, Q2PlatformState)>,
    /// Secret doors.
    pub secrets: Vec<(SavedActorId, Q2SecretState)>,
    /// Linear motion.
    pub linear: Vec<Q2LinearMotionEntry>,
    /// Animations.
    pub animations: Vec<(SavedActorId, u64, u64)>,
    /// Clocks.
    pub clocks: Vec<(SavedActorId, f64)>,
    /// Turret breaches.
    pub breaches: Vec<(SavedActorId, Q2BreachState)>,
    /// Turret drivers.
    pub drivers: Vec<Q2TurretDriver>,
    /// Wind expiries.
    pub wind_times: Vec<(SavedActorId, f64)>,
}

fn nonnegative(reader: SaveReader) -> Result<u64, PersistenceError> {
    let value = reader.integer(0)?;
    u64::try_from(value).map_err(|_| PersistenceError::from(reader.fail("expected an integer in range")))
}

/// Read a Q2 base-entities checkpoint.
pub fn read_q2_base_entities_checkpoint(reader: SaveReader) -> Result<Q2BaseEntitiesCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    let movers = reader.field("movers");
    let scenery = reader.field("scenery");
    let turrets = reader.field("turrets");
    Ok(Q2BaseEntitiesCheckpoint {
        platforms: movers.field("platforms").list(
            |value| -> Result<(SavedActorId, Q2PlatformState), PersistenceError> {
                let state = value.field("state");
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    Q2PlatformState {
                        top: read_vector(state.field("top"))?,
                        bottom: read_vector(state.field("bottom"))?,
                        phase: state.field("phase").choice_str(&["top", "bottom", "up", "down"])?,
                    },
                ))
            },
        )?,
        secrets: movers
            .field("secrets")
            .list(|value| -> Result<(SavedActorId, Q2SecretState), PersistenceError> {
                let state = value.field("state");
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    Q2SecretState {
                        first: read_vector(state.field("first"))?,
                        second: read_vector(state.field("second"))?,
                        home: read_vector(state.field("home"))?,
                        shootable: state.field("shootable").boolean()?,
                        blocked_time: state.field("blockedTime").number()?,
                        message_time: state.field("messageTime").number()?,
                    },
                ))
            })?,
        linear: read_q2_linear_motion_checkpoint(movers.field("linear"))?,
        animations: scenery.field("animations").list(
            |value| -> Result<(SavedActorId, u64, u64), PersistenceError> {
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    nonnegative(value.field("first"))?,
                    {
                        let end = value.field("end").integer(1)?;
                        u64::try_from(end).map_err(|_| {
                            PersistenceError::from(value.field("end").fail("expected an integer in range"))
                        })?
                    },
                ))
            },
        )?,
        clocks: scenery
            .field("clocks")
            .list(|value| -> Result<(SavedActorId, f64), PersistenceError> {
                Ok((read_saved_actor(value.field("actor"))?, value.field("value").number()?))
            })?,
        breaches: turrets.field("breaches").list(
            |value| -> Result<(SavedActorId, Q2BreachState), PersistenceError> {
                let state = value.field("state");
                Ok((
                    read_saved_actor(value.field("actor"))?,
                    Q2BreachState {
                        goal: read_vector(state.field("goal"))?,
                        muzzle: read_vector(state.field("muzzle"))?,
                        pitch_max: state.field("pitchMax").number()?,
                        pitch_min: state.field("pitchMin").number()?,
                        yaw_min: state.field("yawMin").number()?,
                        yaw_max: state.field("yawMax").number()?,
                    },
                ))
            },
        )?,
        drivers: turrets
            .field("drivers")
            .list(|value| -> Result<Q2TurretDriver, PersistenceError> {
                Ok(Q2TurretDriver {
                    actor: read_saved_actor(value.field("actor"))?,
                    breach: value
                        .field("breach")
                        .nullable(|item| read_saved_actor(item).map_err(PersistenceError::from))?,
                    radius: value.field("radius").number()?,
                    yaw_offset: value.field("yawOffset").number()?,
                    height: value.field("height").number()?,
                    monster_die: value.field("monsterDie").string()?,
                })
            })?,
        wind_times: reader
            .field("windTimes")
            .list(|value| -> Result<(SavedActorId, f64), PersistenceError> {
                Ok((read_saved_actor(value.field("actor"))?, value.field("until").number()?))
            })?,
    })
}

/// Write a Q2 base-entities checkpoint.
#[must_use]
pub fn write_q2_base_entities_checkpoint(checkpoint: &Q2BaseEntitiesCheckpoint) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    obj(vec![
        ("version", int(1)),
        (
            "movers",
            obj(vec![
                (
                    "platforms",
                    arr(checkpoint
                        .platforms
                        .iter()
                        .map(|(actor, state)| {
                            obj(vec![
                                ("actor", write_saved_actor(*actor)),
                                (
                                    "state",
                                    obj(vec![
                                        ("top", write_vector(state.top)),
                                        ("bottom", write_vector(state.bottom)),
                                        ("phase", str(&state.phase)),
                                    ]),
                                ),
                            ])
                        })
                        .collect()),
                ),
                (
                    "secrets",
                    arr(checkpoint
                        .secrets
                        .iter()
                        .map(|(actor, state)| {
                            obj(vec![
                                ("actor", write_saved_actor(*actor)),
                                (
                                    "state",
                                    obj(vec![
                                        ("first", write_vector(state.first)),
                                        ("second", write_vector(state.second)),
                                        ("home", write_vector(state.home)),
                                        ("shootable", boolean(state.shootable)),
                                        ("blockedTime", num(state.blocked_time)),
                                        ("messageTime", num(state.message_time)),
                                    ]),
                                ),
                            ])
                        })
                        .collect()),
                ),
                ("linear", write_q2_linear_motion_checkpoint(&checkpoint.linear)),
            ]),
        ),
        (
            "scenery",
            obj(vec![
                (
                    "animations",
                    arr(checkpoint
                        .animations
                        .iter()
                        .map(|(actor, first, end)| {
                            obj(vec![
                                ("actor", write_saved_actor(*actor)),
                                ("first", int(*first as i64)),
                                ("end", int(*end as i64)),
                            ])
                        })
                        .collect()),
                ),
                (
                    "clocks",
                    arr(checkpoint
                        .clocks
                        .iter()
                        .map(|(actor, value)| obj(vec![("actor", write_saved_actor(*actor)), ("value", num(*value))]))
                        .collect()),
                ),
            ]),
        ),
        (
            "turrets",
            obj(vec![
                (
                    "breaches",
                    arr(checkpoint
                        .breaches
                        .iter()
                        .map(|(actor, state)| {
                            obj(vec![
                                ("actor", write_saved_actor(*actor)),
                                (
                                    "state",
                                    obj(vec![
                                        ("goal", write_vector(state.goal)),
                                        ("muzzle", write_vector(state.muzzle)),
                                        ("pitchMax", num(state.pitch_max)),
                                        ("pitchMin", num(state.pitch_min)),
                                        ("yawMin", num(state.yaw_min)),
                                        ("yawMax", num(state.yaw_max)),
                                    ]),
                                ),
                            ])
                        })
                        .collect()),
                ),
                (
                    "drivers",
                    arr(checkpoint
                        .drivers
                        .iter()
                        .map(|driver| {
                            obj(vec![
                                ("actor", write_saved_actor(driver.actor)),
                                ("breach", driver.breach.map_or(SaveJson::Null, write_saved_actor)),
                                ("radius", num(driver.radius)),
                                ("yawOffset", num(driver.yaw_offset)),
                                ("height", num(driver.height)),
                                ("monsterDie", str(&driver.monster_die)),
                            ])
                        })
                        .collect()),
                ),
            ]),
        ),
        (
            "windTimes",
            arr(checkpoint
                .wind_times
                .iter()
                .map(|(actor, until)| obj(vec![("actor", write_saved_actor(*actor)), ("until", num(*until))]))
                .collect()),
        ),
    ])
}

/// Encode a Q2 base-entities checkpoint.
#[must_use]
pub fn encode_q2_base_entities_checkpoint(checkpoint: &Q2BaseEntitiesCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_q2_base_entities_checkpoint(checkpoint))
}

/// Decode a Q2 base-entities checkpoint.
pub fn decode_q2_base_entities_checkpoint(bytes: &[u8]) -> Result<Q2BaseEntitiesCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_q2_base_entities_checkpoint(SaveReader::at(&payload, "q2-base-entities"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_entities_round_trip() {
        let zero = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        let checkpoint = Q2BaseEntitiesCheckpoint {
            platforms: vec![(
                SavedActorId { slot: 1, generation: 0 },
                Q2PlatformState {
                    top: zero,
                    bottom: zero,
                    phase: "top".to_string(),
                },
            )],
            secrets: Vec::new(),
            linear: Vec::new(),
            animations: vec![(SavedActorId { slot: 2, generation: 0 }, 0, 4)],
            clocks: vec![(SavedActorId { slot: 3, generation: 0 }, 1.5)],
            breaches: Vec::new(),
            drivers: vec![Q2TurretDriver {
                actor: SavedActorId { slot: 4, generation: 0 },
                breach: None,
                radius: 128.0,
                yaw_offset: 0.0,
                height: 32.0,
                monster_die: "turret_die".to_string(),
            }],
            wind_times: Vec::new(),
        };
        let bytes = encode_q2_base_entities_checkpoint(&checkpoint);
        assert_eq!(decode_q2_base_entities_checkpoint(&bytes).unwrap(), checkpoint);
    }
}
