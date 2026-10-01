//! Grapple runtime checkpoint reader.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/grapple-checkpoint.ts`
//! (`readGrappleRuntimeCheckpoint`).
//!
//! The QVM component arrives through a caller-supplied reader (value seam for
//! donor `readQvmGrappleSourceCheckpoint`); everything else builds content
//! records inline or keeps the persistence shape for the foundation bridges.

use qa_content::q2::equipment::grapple_services::{CtfGrappleCheckpoint, CtfGrapplePhase, LmctfGrappleCheckpoint};
use qa_world::save::records::read_saved_actor;
use qa_world::save::shared::read_vector;
use qa_world::save::value::SaveReader;
use qa_world::WorldError;

use super::equipment_checkpoint::read_source_random_checkpoint;
use super::grapple_runtime::{
    CtfGrappleStateEntry, GrappleControlEntry, GrappleRuntimeCheckpoint, GrappleSourceCheckpoint,
    GrappleWeaponAnimationEntry, LmctfGrappleStateEntry, QvmGrappleSourceCheckpoint, CHECKPOINT_VERSION,
};
use super::weapon_slot_checkpoint::read_grapple_weapon_state;
use crate::persistence::q1::foundation::read_q1_foundation_checkpoint;
use crate::persistence::q2::foundation::read_q2_foundation_checkpoint;

/// Read a grapple runtime checkpoint.
///
/// `read_qvm` reads the QVM source component (donor
/// `readQvmGrappleSourceCheckpoint` in
/// `src/app/bootstrap/simulation/qvm-grapple-source.ts`; canonical home:
/// `simulation::qvm_grapple_source`); unify post-merge.
pub fn read_grapple_runtime_checkpoint(
    reader: SaveReader,
    read_qvm: &dyn Fn(SaveReader) -> Result<QvmGrappleSourceCheckpoint, WorldError>,
) -> Result<GrappleRuntimeCheckpoint, WorldError> {
    reader.field("version").literal_i64(2)?;
    let random_field = reader.field("random");
    let random = match read_source_random_checkpoint(random_field.clone()) {
        Ok(random) => random,
        Err(error) => {
            let kind = random_field.field("kind").string().unwrap_or_default();
            if matches!(kind.as_str(), "q3-lcg" | "msvcrt-rand" | "guest") {
                return Err(random_field.fail("Unsupported grapple random stream"));
            }
            return Err(error);
        }
    };
    let source = reader.field("source");
    let kind = source
        .field("kind")
        .choice_str(&["q1-threewave", "q2-ctf", "q2-lmctf", "q3-qvm"])?;
    let source = match kind.as_str() {
        "q1-threewave" => GrappleSourceCheckpoint::Q1Threewave {
            entities: read_q1_foundation_checkpoint(source.field("entities"))
                .map_err(|error| WorldError::BadSave(error.to_string()))?,
        },
        "q2-ctf" => {
            let states = source.field("states").list(|entry| {
                let state = entry.field("state");
                let phase = match state
                    .field("grappleState")
                    .choice_str(&["fly", "pull", "hang"])?
                    .as_str()
                {
                    "fly" => CtfGrapplePhase::Fly,
                    "pull" => CtfGrapplePhase::Pull,
                    _ => CtfGrapplePhase::Hang,
                };
                let no_knockback_field = state.field("grappleNoKnockback");
                let no_knockback = if no_knockback_field.is_missing() {
                    None
                } else {
                    no_knockback_field.nullable(|value| value.boolean())?
                };
                Ok(CtfGrappleStateEntry {
                    actor: read_saved_actor(entry.field("actor"))?,
                    state: CtfGrappleCheckpoint {
                        grapple: state.field("grapple").nullable(read_saved_actor)?,
                        grapple_state: phase,
                        grapple_release_time: state.field("grappleReleaseTime").finite()?,
                        grapple_no_knockback: no_knockback,
                    },
                })
            })?;
            GrappleSourceCheckpoint::Q2Ctf {
                entities: read_q2_foundation_checkpoint(source.field("entities"))
                    .map_err(|error| WorldError::BadSave(error.to_string()))?,
                states,
            }
        }
        "q2-lmctf" => {
            let states = source.field("states").list(|entry| {
                let state = entry.field("state");
                let hook_state = match state.field("hookState").integer(0)? {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    _ => return Err(state.field("hookState").fail("expected 0, 1 or 2")),
                };
                Ok(LmctfGrappleStateEntry {
                    actor: read_saved_actor(entry.field("actor"))?,
                    state: LmctfGrappleCheckpoint {
                        hook: state.field("hook").nullable(read_saved_actor)?,
                        hook_state,
                        hook_length: state.field("hookLength").finite()?,
                        hook_held: state.field("hookHeld").boolean()?,
                    },
                })
            })?;
            GrappleSourceCheckpoint::Q2Lmctf {
                entities: read_q2_foundation_checkpoint(source.field("entities"))
                    .map_err(|error| WorldError::BadSave(error.to_string()))?,
                states,
            }
        }
        _ => GrappleSourceCheckpoint::Q3Qvm {
            component: read_qvm(source.field("component"))?,
            holstered: source.field("holstered").list(|entry| read_saved_actor(entry))?,
        },
    };
    Ok(GrappleRuntimeCheckpoint {
        version: CHECKPOINT_VERSION,
        weapon_animations: reader.field("weaponAnimations").list(|entry| {
            Ok(GrappleWeaponAnimationEntry {
                actor: read_saved_actor(entry.field("actor"))?,
                state: read_grapple_weapon_state(entry.field("state"))?,
                next_frame_at: entry.field("nextFrameAt").finite()?,
                kick_origin: read_vector(entry.field("kickOrigin"))?,
                kick_pitch: entry.field("kickPitch").finite()?,
            })
        })?,
        controls: reader.field("controls").list(|entry| {
            let teleport_bit = entry.field("teleportBit").nullable(|value| match value.integer(0)? {
                0 => Ok(0),
                4 => Ok(4),
                _ => Err(value.fail("expected 0 or 4")),
            })?;
            Ok(GrappleControlEntry {
                actor: read_saved_actor(entry.field("actor"))?,
                teleport_bit,
                jump: entry.field("jump").boolean()?,
                held: entry.field("held").boolean()?,
                pressed: entry.field("pressed").boolean()?,
                released: entry.field("released").boolean()?,
                previous_velocity: read_vector(entry.field("previousVelocity"))?,
                prediction_suppressed: entry.field("predictionSuppressed").boolean()?,
            })
        })?,
        random,
        source,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::SavedActorId;
    use qa_core::math::vec3;
    use qa_world::save::records::write_saved_actor;
    use qa_world::save::shared::write_vector;
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

    fn q2_entities() -> SaveJson {
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

    fn q1_entities() -> SaveJson {
        obj(vec![
            ("provider", save_str("q1:test")),
            ("format", save_str("q1-foundation")),
            ("version", int(5)),
            (
                "precaches",
                obj(vec![
                    ("phase", save_str("frozen")),
                    ("models", arr(Vec::new())),
                    ("sounds", arr(Vec::new())),
                ]),
            ),
            ("edition", save_str("classic")),
            ("time", num(100.0)),
            ("frameSeconds", num(0.1)),
            ("forceRetouch", num(0.0)),
            ("sequence", int(0)),
            ("nextDynamicSlot", int(1)),
            ("totalSecrets", num(0.0)),
            ("foundSecrets", num(0.0)),
            ("totalMonsters", num(0.0)),
            ("killedMonsters", num(0.0)),
            ("worldType", num(0.0)),
            ("mapName", save_str("test")),
            (
                "basis",
                obj(vec![
                    ("forward", write_vector(vec3(1.0, 0.0, 0.0))),
                    ("right", write_vector(vec3(0.0, 1.0, 0.0))),
                    ("up", write_vector(vec3(0.0, 0.0, 1.0))),
                ]),
            ),
            ("world", SaveJson::Null),
            ("sightEntity", SaveJson::Null),
            ("sightTime", num(0.0)),
            ("intermission", SaveJson::Null),
            ("entities", arr(Vec::new())),
            ("players", arr(Vec::new())),
            ("extensions", arr(Vec::new())),
        ])
    }

    fn weapon_state() -> SaveJson {
        obj(vec![
            ("handoff", save_str("active")),
            (
                "animation",
                obj(vec![
                    ("phase", save_str("firing")),
                    ("frame", int(2)),
                    ("latchedAttack", boolean(true)),
                    ("sourceFiring", boolean(false)),
                    ("thinkTime", num(0.0)),
                    ("fireFinished", num(0.0)),
                    ("fireBuffered", boolean(false)),
                    ("lastFiringTime", num(0.0)),
                ]),
            ),
        ])
    }

    fn animations() -> SaveJson {
        arr(vec![obj(vec![
            ("actor", write_saved_actor(SavedActorId { slot: 2, generation: 0 })),
            ("state", weapon_state()),
            ("nextFrameAt", num(100.1)),
            ("kickOrigin", write_vector(vec3(1.0, 2.0, 3.0))),
            ("kickPitch", num(4.0)),
        ])])
    }

    fn controls() -> SaveJson {
        arr(vec![obj(vec![
            ("actor", write_saved_actor(SavedActorId { slot: 2, generation: 0 })),
            ("teleportBit", int(4)),
            ("jump", boolean(true)),
            ("held", boolean(true)),
            ("pressed", boolean(false)),
            ("released", boolean(false)),
            ("previousVelocity", write_vector(vec3(0.0, 0.0, 0.0))),
            ("predictionSuppressed", boolean(true)),
        ])])
    }

    fn ctf_source() -> SaveJson {
        obj(vec![
            ("kind", save_str("q2-ctf")),
            ("entities", q2_entities()),
            (
                "states",
                arr(vec![
                    obj(vec![
                        ("actor", write_saved_actor(SavedActorId { slot: 2, generation: 0 })),
                        (
                            "state",
                            obj(vec![
                                ("grapple", write_saved_actor(SavedActorId { slot: 9, generation: 0 })),
                                ("grappleState", save_str("pull")),
                                ("grappleReleaseTime", num(101.0)),
                                ("grappleNoKnockback", boolean(true)),
                            ]),
                        ),
                    ]),
                    obj(vec![
                        ("actor", write_saved_actor(SavedActorId { slot: 3, generation: 1 })),
                        (
                            "state",
                            obj(vec![
                                ("grapple", SaveJson::Null),
                                ("grappleState", save_str("fly")),
                                ("grappleReleaseTime", num(0.0)),
                            ]),
                        ),
                    ]),
                ]),
            ),
        ])
    }

    fn checkpoint(source: SaveJson) -> SaveJson {
        obj(vec![
            ("version", int(2)),
            ("weaponAnimations", animations()),
            ("controls", controls()),
            ("random", glibc_random()),
            ("source", source),
        ])
    }

    fn read(json: &SaveJson) -> Result<GrappleRuntimeCheckpoint, WorldError> {
        read_grapple_runtime_checkpoint(SaveReader::new(json), &|_| panic!("no qvm component expected"))
    }

    #[test]
    fn reads_ctf_checkpoint() {
        let json = checkpoint(ctf_source());
        let checkpoint = read(&json).expect("read");
        assert_eq!(checkpoint.version, 2);
        assert_eq!(checkpoint.weapon_animations.len(), 1);
        let animation = &checkpoint.weapon_animations[0];
        assert_eq!(animation.actor, SavedActorId { slot: 2, generation: 0 });
        assert_eq!(animation.next_frame_at, 100.1);
        assert_eq!(animation.kick_origin, vec3(1.0, 2.0, 3.0));
        assert_eq!(animation.kick_pitch, 4.0);
        assert_eq!(
            checkpoint.controls,
            vec![GrappleControlEntry {
                actor: SavedActorId { slot: 2, generation: 0 },
                teleport_bit: Some(4),
                jump: true,
                held: true,
                pressed: false,
                released: false,
                previous_velocity: vec3(0.0, 0.0, 0.0),
                prediction_suppressed: true,
            }]
        );
        match &checkpoint.source {
            GrappleSourceCheckpoint::Q2Ctf { states, entities } => {
                assert_eq!(entities.next_source_slot, 9);
                assert_eq!(states.len(), 2);
                assert_eq!(states[0].actor, SavedActorId { slot: 2, generation: 0 });
                assert_eq!(states[0].state.grapple, Some(SavedActorId { slot: 9, generation: 0 }));
                assert_eq!(states[0].state.grapple_state, CtfGrapplePhase::Pull);
                assert_eq!(states[0].state.grapple_release_time, 101.0);
                assert_eq!(states[0].state.grapple_no_knockback, Some(true));
                assert_eq!(states[1].state.grapple, None);
                assert_eq!(states[1].state.grapple_state, CtfGrapplePhase::Fly);
                assert_eq!(states[1].state.grapple_no_knockback, None);
            }
            _ => panic!("ctf source"),
        }
    }

    #[test]
    fn reads_lmctf_states() {
        let json = checkpoint(obj(vec![
            ("kind", save_str("q2-lmctf")),
            ("entities", q2_entities()),
            (
                "states",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(SavedActorId { slot: 2, generation: 0 })),
                    (
                        "state",
                        obj(vec![
                            ("hook", write_saved_actor(SavedActorId { slot: 9, generation: 0 })),
                            ("hookState", int(2)),
                            ("hookLength", num(64.0)),
                            ("hookHeld", boolean(true)),
                        ]),
                    ),
                ])]),
            ),
        ]));
        let checkpoint = read(&json).expect("read");
        match &checkpoint.source {
            GrappleSourceCheckpoint::Q2Lmctf { states, .. } => {
                assert_eq!(states.len(), 1);
                assert_eq!(states[0].state.hook, Some(SavedActorId { slot: 9, generation: 0 }));
                assert_eq!(states[0].state.hook_state, 2);
                assert_eq!(states[0].state.hook_length, 64.0);
                assert!(states[0].state.hook_held);
            }
            _ => panic!("lmctf source"),
        }
    }

    #[test]
    fn reads_q1_source() {
        let json = checkpoint(obj(vec![
            ("kind", save_str("q1-threewave")),
            ("entities", q1_entities()),
        ]));
        let checkpoint = read(&json).expect("read");
        assert!(matches!(checkpoint.source, GrappleSourceCheckpoint::Q1Threewave { .. }));
    }

    #[test]
    fn delegates_qvm_component() {
        let json = checkpoint(obj(vec![
            ("kind", save_str("q3-qvm")),
            ("component", obj(vec![("version", int(1))])),
            (
                "holstered",
                arr(vec![write_saved_actor(SavedActorId { slot: 2, generation: 0 })]),
            ),
        ]));
        let checkpoint = read_grapple_runtime_checkpoint(SaveReader::new(&json), &|reader| {
            reader.field("version").literal_i64(1)?;
            Ok(QvmGrappleSourceCheckpoint {
                version: 1,
                grapple: super::super::grapple_runtime::QvmGrappleCheckpoint {
                    version: 1,
                    profile: "test".to_string(),
                    module: qa_guest::checkpoint::GuestCheckpoint::Qvm {
                        module: qa_guest::checkpoint::ModuleIdentity {
                            id: "q3:test".to_string(),
                            artifact_path: "test.qvm".to_string(),
                            digest: "sha256:00".to_string(),
                            revision: "1".to_string(),
                        },
                        random: Vec::new(),
                        callbacks: Vec::new(),
                        api: qa_guest::checkpoint::GameApi::Q2ClassicGame,
                        abi_profile: "test".to_string(),
                        data: Vec::new(),
                        instruction_index: 0,
                        program_stack: 0,
                        operand_stack: Vec::new(),
                        host_state: qa_guest::checkpoint::GuestPrivateState {
                            module: qa_guest::checkpoint::ModuleIdentity {
                                id: "q3:test".to_string(),
                                artifact_path: "test.qvm".to_string(),
                                digest: "sha256:00".to_string(),
                                revision: "1".to_string(),
                            },
                            format: "q3:test".to_string(),
                            bytes: Vec::new(),
                        },
                    },
                    owners: Vec::new(),
                },
                bindings: Vec::new(),
                tethers: Vec::new(),
            })
        })
        .expect("read");
        match &checkpoint.source {
            GrappleSourceCheckpoint::Q3Qvm { holstered, .. } => {
                assert_eq!(holstered, &vec![SavedActorId { slot: 2, generation: 0 }]);
            }
            _ => panic!("qvm source"),
        }
    }

    #[test]
    fn rejects_foreign_random_stream() {
        let mut json = checkpoint(ctf_source());
        let SaveJson::Object(entries) = &mut json else {
            panic!("checkpoint object");
        };
        for (key, value) in entries.iter_mut() {
            if key == "random" {
                *value = obj(vec![("kind", save_str("q3-lcg")), ("seed", int(1)), ("draws", int(0))]);
            }
        }
        let error = read(&json).expect_err("foreign stream");
        assert!(error.to_string().contains("Unsupported grapple random stream"));
    }

    #[test]
    fn rejects_bad_source_kind() {
        let json = checkpoint(obj(vec![("kind", save_str("q9-grapple"))]));
        assert!(read(&json).is_err());
    }

    #[test]
    fn rejects_bad_hook_state() {
        let json = checkpoint(obj(vec![
            ("kind", save_str("q2-lmctf")),
            ("entities", q2_entities()),
            (
                "states",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(SavedActorId { slot: 2, generation: 0 })),
                    (
                        "state",
                        obj(vec![
                            ("hook", SaveJson::Null),
                            ("hookState", int(3)),
                            ("hookLength", num(64.0)),
                            ("hookHeld", boolean(false)),
                        ]),
                    ),
                ])]),
            ),
        ]));
        assert!(read(&json).is_err());
    }

    #[test]
    fn rejects_bad_teleport_bit() {
        let json = checkpoint(ctf_source());
        let SaveJson::Object(entries) = &json else {
            panic!("checkpoint object");
        };
        let mut modified = entries.clone();
        for (key, value) in modified.iter_mut() {
            if key == "controls" {
                *value = arr(vec![obj(vec![
                    ("actor", write_saved_actor(SavedActorId { slot: 2, generation: 0 })),
                    ("teleportBit", int(2)),
                    ("jump", boolean(false)),
                    ("held", boolean(false)),
                    ("pressed", boolean(false)),
                    ("released", boolean(false)),
                    ("previousVelocity", write_vector(vec3(0.0, 0.0, 0.0))),
                    ("predictionSuppressed", boolean(false)),
                ])]);
            }
        }
        let json = SaveJson::Object(modified);
        assert!(read(&json).is_err());
    }
}
