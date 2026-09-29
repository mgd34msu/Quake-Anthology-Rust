//! Quake II hand-grenade checkpoint ported from `src/persistence/q2-hand-grenades.ts`.

use qa_core::identity::SavedActorId;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, num, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;

/// Hand-grenade loadout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandGrenadeLoadout {
    /// Enabled flag.
    pub enabled: bool,
    /// Initial ammo.
    pub initial_ammo: u64,
    /// Capacity.
    pub capacity: u64,
    /// Infinite ammo.
    pub infinite_ammo: bool,
}

#[allow(clippy::cast_sign_loss)]
fn read_loadout(reader: SaveReader) -> Result<HandGrenadeLoadout, PersistenceError> {
    let initial_ammo = reader.field("initialAmmo").integer(0)?;
    let capacity = reader.field("capacity").integer(0)?;
    if initial_ammo > capacity {
        return Err(PersistenceError::from(
            reader.fail("initial hand grenade allowance exceeds capacity"),
        ));
    }
    Ok(HandGrenadeLoadout {
        enabled: reader.field("enabled").boolean()?,
        initial_ammo: initial_ammo as u64,
        capacity: capacity as u64,
        infinite_ammo: reader.field("infiniteAmmo").boolean()?,
    })
}

fn write_loadout(loadout: &HandGrenadeLoadout) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    obj(vec![
        ("enabled", boolean(loadout.enabled)),
        ("initialAmmo", int(loadout.initial_ammo as i64)),
        ("capacity", int(loadout.capacity as i64)),
        ("infiniteAmmo", boolean(loadout.infinite_ammo)),
    ])
}

/// Hand-grenade action.
#[derive(Debug, Clone, PartialEq)]
pub enum HandAction {
    /// Idle.
    Idle,
    /// Disarmed.
    Disarmed,
    /// Preparing.
    Preparing {
        /// Frame.
        frame: u64,
        /// Next time.
        next_at: f64,
        /// Release queued.
        release_queued: bool,
    },
    /// Cooking.
    Cooking {
        /// Expiry.
        expires_at: f64,
    },
    /// Releasing.
    Releasing {
        /// Expiry.
        expires_at: f64,
        /// Throw time.
        throw_at: f64,
    },
    /// Recovering.
    Recovering {
        /// Ready time.
        ready_at: f64,
        /// Require release.
        require_release: bool,
    },
}

#[allow(clippy::cast_sign_loss)]
fn read_action(reader: SaveReader) -> Result<HandAction, PersistenceError> {
    match reader
        .field("kind")
        .choice_str(&["idle", "disarmed", "preparing", "cooking", "releasing", "recovering"])?
        .as_str()
    {
        "idle" => Ok(HandAction::Idle),
        "disarmed" => Ok(HandAction::Disarmed),
        "preparing" => {
            let frame = reader.field("frame").integer(1)?;
            if frame > 11 {
                return Err(PersistenceError::from(
                    reader
                        .field("frame")
                        .fail("hand grenade preparation exceeds hold frame"),
                ));
            }
            Ok(HandAction::Preparing {
                frame: frame as u64,
                next_at: reader.field("nextAt").finite()?,
                release_queued: reader.field("releaseQueued").boolean()?,
            })
        }
        "cooking" => Ok(HandAction::Cooking {
            expires_at: reader.field("expiresAt").finite()?,
        }),
        "releasing" => Ok(HandAction::Releasing {
            expires_at: reader.field("expiresAt").finite()?,
            throw_at: reader.field("throwAt").finite()?,
        }),
        _ => Ok(HandAction::Recovering {
            ready_at: reader.field("readyAt").finite()?,
            require_release: reader.field("requireRelease").boolean()?,
        }),
    }
}

fn write_action(action: &HandAction) -> SaveJson {
    match action {
        HandAction::Idle => obj(vec![("kind", str("idle"))]),
        HandAction::Disarmed => obj(vec![("kind", str("disarmed"))]),
        HandAction::Preparing {
            frame,
            next_at,
            release_queued,
        } => obj(vec![
            ("kind", str("preparing")),
            #[allow(clippy::cast_possible_wrap)]
            ("frame", int(*frame as i64)),
            ("nextAt", num(*next_at)),
            ("releaseQueued", boolean(*release_queued)),
        ]),
        HandAction::Cooking { expires_at } => obj(vec![("kind", str("cooking")), ("expiresAt", num(*expires_at))]),
        HandAction::Releasing { expires_at, throw_at } => obj(vec![
            ("kind", str("releasing")),
            ("expiresAt", num(*expires_at)),
            ("throwAt", num(*throw_at)),
        ]),
        HandAction::Recovering {
            ready_at,
            require_release,
        } => obj(vec![
            ("kind", str("recovering")),
            ("readyAt", num(*ready_at)),
            ("requireRelease", boolean(*require_release)),
        ]),
    }
}

/// Hand-grenade actor state.
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeActorState {
    /// Loadout.
    pub config: HandGrenadeLoadout,
    /// Action.
    pub action: HandAction,
}

/// Hand-grenades checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadesCheckpoint {
    /// Edition.
    pub edition: String,
    /// Actors.
    pub actors: Vec<(SavedActorId, HandGrenadeActorState)>,
}

/// Read a hand-grenades checkpoint.
pub fn read_hand_grenades_checkpoint(reader: SaveReader) -> Result<HandGrenadesCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    Ok(HandGrenadesCheckpoint {
        edition: reader.field("edition").choice_str(&["classic", "rerelease"])?,
        actors: reader.field("actors").list(
            |entry| -> Result<(SavedActorId, HandGrenadeActorState), PersistenceError> {
                let state = entry.field("state");
                Ok((
                    read_saved_actor(entry.field("actor"))?,
                    HandGrenadeActorState {
                        config: read_loadout(state.field("config"))?,
                        action: read_action(state.field("action"))?,
                    },
                ))
            },
        )?,
    })
}

/// Write a hand-grenades checkpoint.
#[must_use]
pub fn write_hand_grenades_checkpoint(checkpoint: &HandGrenadesCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(1)),
        ("edition", str(&checkpoint.edition)),
        (
            "actors",
            arr(checkpoint
                .actors
                .iter()
                .map(|(actor, state)| {
                    obj(vec![
                        ("actor", write_saved_actor(*actor)),
                        (
                            "state",
                            obj(vec![
                                ("config", write_loadout(&state.config)),
                                ("action", write_action(&state.action)),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Encode a hand-grenades checkpoint.
#[must_use]
pub fn encode_hand_grenades_checkpoint(checkpoint: &HandGrenadesCheckpoint) -> Vec<u8> {
    encode_checkpoint_value(&write_hand_grenades_checkpoint(checkpoint))
}

/// Decode a hand-grenades checkpoint.
pub fn decode_hand_grenades_checkpoint(bytes: &[u8]) -> Result<HandGrenadesCheckpoint, PersistenceError> {
    let payload = decode_checkpoint_value(bytes)?;
    read_hand_grenades_checkpoint(SaveReader::at(&payload, "q2-hand-grenades"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grenades_round_trip() {
        let checkpoint = HandGrenadesCheckpoint {
            edition: "classic".to_string(),
            actors: vec![(
                SavedActorId { slot: 1, generation: 0 },
                HandGrenadeActorState {
                    config: HandGrenadeLoadout {
                        enabled: true,
                        initial_ammo: 1,
                        capacity: 5,
                        infinite_ammo: false,
                    },
                    action: HandAction::Preparing {
                        frame: 3,
                        next_at: 1.0,
                        release_queued: false,
                    },
                },
            )],
        };
        let bytes = encode_hand_grenades_checkpoint(&checkpoint);
        assert_eq!(decode_hand_grenades_checkpoint(&bytes).unwrap(), checkpoint);
    }

    #[test]
    fn loadout_limits_hold() {
        let bad = HandGrenadesCheckpoint {
            edition: "classic".to_string(),
            actors: vec![(
                SavedActorId { slot: 1, generation: 0 },
                HandGrenadeActorState {
                    config: HandGrenadeLoadout {
                        enabled: true,
                        initial_ammo: 9,
                        capacity: 5,
                        infinite_ammo: false,
                    },
                    action: HandAction::Idle,
                },
            )],
        };
        let json = write_hand_grenades_checkpoint(&bad);
        assert!(read_hand_grenades_checkpoint(SaveReader::new(&json)).is_err());
    }
}
