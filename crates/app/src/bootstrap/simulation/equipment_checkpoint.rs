//! Hand-grenade runtime checkpoint reader.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/equipment-checkpoint.ts`
//! (`readHandGrenadeRuntimeCheckpoint`).
//!
//! The reader builds content records where the conversion is mechanical (the
//! six grenade action variants) and keeps the foundation entities in the
//! persistence shape for [`Q2FoundationCheckpointBridge`](super::equipment_runtime::Q2FoundationCheckpointBridge).

use std::collections::HashMap;

use qa_content::q2::equipment::hand_grenades::{HandGrenadeActorCheckpoint, HandGrenadeCheckpoint, HandGrenadeLoadout};
use qa_content::q2::foundation::host::Q2Edition;
use qa_content::q2::foundation::weapons::hand_action::HandAction;
use qa_world::save::records::read_saved_actor;
use qa_world::save::shared::{read_random, SaveRandomState};
use qa_world::save::value::SaveReader;
use qa_world::WorldError;

use super::equipment_runtime::{
    HandControlEntry, HandGrenadeRuntimeCheckpoint, HandGrenadeSourceCheckpoint, CHECKPOINT_VERSION,
};
use super::random::{GlibcCheckpoint, Mt19937Checkpoint, RandomCheckpoint};
use crate::persistence::q2::foundation::read_q2_foundation_checkpoint;
use crate::persistence::q2::hand_grenades::{read_hand_grenades_checkpoint, HandAction as SavedHandAction};

/// Read a hand-grenade runtime checkpoint.
pub fn read_hand_grenade_runtime_checkpoint(reader: SaveReader) -> Result<HandGrenadeRuntimeCheckpoint, WorldError> {
    let source = reader.field("source");
    let random = read_source_random_checkpoint(source.field("random"))?;
    reader.field("version").literal_i64(1)?;
    Ok(HandGrenadeRuntimeCheckpoint {
        version: CHECKPOINT_VERSION,
        controller: read_grenade_controller(reader.field("controller"))?,
        controls: reader.field("controls").list(|entry| {
            Ok(HandControlEntry {
                actor: read_saved_actor(entry.field("actor"))?,
                held: entry.field("held").boolean()?,
                pressed: entry.field("pressed").boolean()?,
                released: entry.field("released").boolean()?,
            })
        })?,
        source: HandGrenadeSourceCheckpoint {
            entities: read_q2_foundation_checkpoint(source.field("entities"))
                .map_err(|error| WorldError::BadSave(error.to_string()))?,
            random,
        },
    })
}

/// Read a session RNG stream narrowed to the equipment profiles.
pub fn read_source_random_checkpoint(reader: SaveReader) -> Result<RandomCheckpoint, WorldError> {
    match read_random(reader.clone())? {
        SaveRandomState::GlibcRandom {
            words,
            front,
            rear,
            draws,
        } => {
            let mut converted = Vec::with_capacity(words.len());
            for word in words {
                converted
                    .push(i32::try_from(word).map_err(|_| reader.field("words").fail("expected int32 glibc words"))?);
            }
            let front = usize::try_from(front).map_err(|_| reader.field("front").fail("expected a glibc cursor"))?;
            let rear = usize::try_from(rear).map_err(|_| reader.field("rear").fail("expected a glibc cursor"))?;
            let checkpoint = GlibcCheckpoint {
                words: converted
                    .try_into()
                    .map_err(|_: Vec<i32>| reader.field("words").fail("expected the glibc word table"))?,
                front,
                rear,
                draws,
            };
            if front >= checkpoint.words.len() || rear >= checkpoint.words.len() {
                return Err(reader.field("front").fail("glibc cursor is out of range"));
            }
            Ok(RandomCheckpoint::Glibc(checkpoint))
        }
        SaveRandomState::RereleaseMt19937 { words, index, draws } => {
            let checkpoint = Mt19937Checkpoint {
                words: words
                    .try_into()
                    .map_err(|_: Vec<u32>| reader.field("words").fail("expected the MT19937 word table"))?,
                index: index as usize,
                draws,
            };
            Ok(RandomCheckpoint::Mt19937(Box::new(checkpoint)))
        }
        _ => Err(reader
            .field("kind")
            .fail("expected glibc-random or q2-rerelease-mt19937")),
    }
}

fn read_grenade_controller(reader: SaveReader) -> Result<HandGrenadeCheckpoint, WorldError> {
    let saved = read_hand_grenades_checkpoint(reader).map_err(|error| WorldError::BadSave(error.to_string()))?;
    let edition = match saved.edition.as_str() {
        "classic" => Q2Edition::Classic,
        "rerelease" => Q2Edition::Rerelease,
        other => {
            return Err(WorldError::BadSave(format!("unknown hand grenade edition {other}")));
        }
    };
    let mut actors = HashMap::new();
    for (actor, state) in saved.actors {
        actors.insert(
            format!("{}:{}", actor.slot, actor.generation),
            HandGrenadeActorCheckpoint {
                actor,
                config: HandGrenadeLoadout {
                    enabled: state.config.enabled,
                    initial_ammo: state.config.initial_ammo as f64,
                    capacity: state.config.capacity as f64,
                    infinite_ammo: state.config.infinite_ammo,
                },
                action: convert_action(state.action)?,
            },
        );
    }
    Ok(HandGrenadeCheckpoint {
        version: 1,
        edition,
        actors,
    })
}

fn convert_action(action: SavedHandAction) -> Result<HandAction, WorldError> {
    match action {
        SavedHandAction::Idle => Ok(HandAction::Idle),
        SavedHandAction::Disarmed => Ok(HandAction::Disarmed),
        SavedHandAction::Preparing {
            frame,
            next_at,
            release_queued,
        } => Ok(HandAction::Preparing {
            frame: i32::try_from(frame)
                .map_err(|_| WorldError::BadSave(format!("hand grenade frame {frame} exceeds hold range")))?,
            next_at,
            release_queued,
        }),
        SavedHandAction::Cooking { expires_at } => Ok(HandAction::Cooking { expires_at }),
        SavedHandAction::Releasing { expires_at, throw_at } => Ok(HandAction::Releasing { expires_at, throw_at }),
        SavedHandAction::Recovering {
            ready_at,
            require_release,
        } => Ok(HandAction::Recovering {
            ready_at,
            require_release,
        }),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::SavedActorId;
    use qa_world::save::records::write_saved_actor;
    use qa_world::save::value::{arr, boolean, int, num, obj, str as save_str, SaveJson};

    use super::*;

    fn glibc_random() -> SaveJson {
        obj(vec![
            ("kind", save_str("glibc-random")),
            ("draws", int(310)),
            ("words", arr((0..31).map(int).collect())),
            ("front", int(3)),
            ("rear", int(0)),
        ])
    }

    fn entities() -> SaveJson {
        obj(vec![
            ("version", int(1)),
            ("nextSourceSlot", int(9)),
            ("sequence", int(0)),
            ("freedSlots", arr(Vec::new())),
            (
                "counters",
                obj(vec![
                    ("totalSecrets", num(1.0)),
                    ("foundSecrets", num(0.0)),
                    ("totalGoals", num(0.0)),
                    ("foundGoals", num(0.0)),
                    ("totalMonsters", num(0.0)),
                    ("killedMonsters", num(0.0)),
                    ("serverFlags", num(0.0)),
                ]),
            ),
            ("entities", arr(Vec::new())),
        ])
    }

    fn controller() -> SaveJson {
        obj(vec![
            ("version", int(1)),
            ("edition", save_str("classic")),
            (
                "actors",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(SavedActorId { slot: 2, generation: 0 })),
                    (
                        "state",
                        obj(vec![
                            (
                                "config",
                                obj(vec![
                                    ("enabled", boolean(true)),
                                    ("initialAmmo", int(5)),
                                    ("capacity", int(50)),
                                    ("infiniteAmmo", boolean(false)),
                                ]),
                            ),
                            (
                                "action",
                                obj(vec![
                                    ("kind", save_str("preparing")),
                                    ("frame", int(3)),
                                    ("nextAt", num(101.5)),
                                    ("releaseQueued", boolean(false)),
                                ]),
                            ),
                        ]),
                    ),
                ])]),
            ),
        ])
    }

    fn checkpoint() -> SaveJson {
        obj(vec![
            ("version", int(1)),
            ("controller", controller()),
            (
                "controls",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(SavedActorId { slot: 2, generation: 0 })),
                    ("held", boolean(true)),
                    ("pressed", boolean(true)),
                    ("released", boolean(false)),
                ])]),
            ),
            (
                "source",
                obj(vec![("entities", entities()), ("random", glibc_random())]),
            ),
        ])
    }

    #[test]
    fn reads_full_checkpoint() {
        let json = checkpoint();
        let checkpoint = read_hand_grenade_runtime_checkpoint(SaveReader::new(&json)).expect("read");
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.controller.edition, Q2Edition::Classic);
        assert_eq!(checkpoint.controller.actors.len(), 1);
        let entry = checkpoint.controller.actors.values().next().expect("actor");
        assert_eq!(entry.actor, SavedActorId { slot: 2, generation: 0 });
        assert!(entry.config.enabled);
        assert_eq!(entry.config.initial_ammo, 5.0);
        assert_eq!(entry.config.capacity, 50.0);
        assert_eq!(
            entry.action,
            HandAction::Preparing {
                frame: 3,
                next_at: 101.5,
                release_queued: false,
            }
        );
        assert_eq!(
            checkpoint.controls,
            vec![HandControlEntry {
                actor: SavedActorId { slot: 2, generation: 0 },
                held: true,
                pressed: true,
                released: false,
            }]
        );
        assert_eq!(checkpoint.source.entities.next_source_slot, 9);
        assert!(matches!(
            checkpoint.source.random,
            RandomCheckpoint::Glibc(GlibcCheckpoint { draws: 310, .. })
        ));
    }

    #[test]
    fn reads_mt19937_random() {
        let json = obj(vec![
            ("kind", save_str("q2-rerelease-mt19937")),
            ("draws", int(12)),
            ("words", arr(vec![int(42); 624])),
            ("index", int(7)),
            ("distribution", save_str("msvc-2022-17.6")),
        ]);
        let random = read_source_random_checkpoint(SaveReader::new(&json)).expect("mt");
        assert!(matches!(
            random,
            RandomCheckpoint::Mt19937(boxed) if boxed.draws == 12 && boxed.index == 7
        ));
    }

    #[test]
    fn rejects_foreign_random() {
        let json = obj(vec![("kind", save_str("q3-lcg")), ("seed", int(1)), ("draws", int(0))]);
        assert!(read_source_random_checkpoint(SaveReader::new(&json)).is_err());
    }

    #[test]
    fn rejects_bad_version() {
        let mut json = checkpoint();
        let SaveJson::Object(entries) = &mut json else {
            panic!("checkpoint object");
        };
        for (key, value) in entries.iter_mut() {
            if key == "version" {
                *value = int(2);
            }
        }
        assert!(read_hand_grenade_runtime_checkpoint(SaveReader::new(&json)).is_err());
    }

    #[test]
    fn rejects_bad_edition() {
        let json = obj(vec![
            ("version", int(1)),
            (
                "controller",
                obj(vec![
                    ("version", int(1)),
                    ("edition", save_str("rogue")),
                    ("actors", arr(Vec::new())),
                ]),
            ),
            ("controls", arr(Vec::new())),
            (
                "source",
                obj(vec![("entities", entities()), ("random", glibc_random())]),
            ),
        ]);
        assert!(read_hand_grenade_runtime_checkpoint(SaveReader::new(&json)).is_err());
    }
}
