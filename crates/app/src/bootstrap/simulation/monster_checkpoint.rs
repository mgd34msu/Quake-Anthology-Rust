//! Selected-monster checkpoint records.
//!
//! Absolute donor:
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/monster-checkpoint.ts`
//!
//! The saved authored-monster and per-source checkpoint shapes plus their
//! reader. Entity, monster, mover, and pack checkpoints reuse the real
//! persistence readers; the RNG stream reuses the equipment checkpoint's
//! narrowed random reader. Save field names stay donor camelCase, matching
//! the other Rust save schemas.

use qa_content::contract::{ContentId, MonsterDefinitionReference, ProviderReference};
use qa_content::q2::missionpacks::monsters::types::Q2MonsterMissionPack;
use qa_core::identity::{ProviderId, SavedActorId};
use qa_core::math::Vec3;
use qa_core::time::FrameContext;
use qa_world::save::records::read_saved_actor;
use qa_world::save::shared::{read_frame, read_provider_ref, read_vector, ProviderRef};
use qa_world::save::value::SaveReader;
use qa_world::WorldError;

use super::equipment_checkpoint::read_source_random_checkpoint;
use super::random::RandomCheckpoint;
use crate::persistence::q1::foundation::{read_q1_foundation_checkpoint, Q1FoundationCheckpoint};
use crate::persistence::q2::foundation::{read_q2_foundation_checkpoint, Q2FoundationCheckpoint};
use crate::persistence::q2::missionpacks::{read_q2_mission_pack_monsters_checkpoint, Q2MissionPackMonstersCheckpoint};
use crate::persistence::q2::monsters::{read_q2_monsters_checkpoint, Q2MonstersCheckpoint};
use crate::persistence::q2::movers::{read_q2_movers_checkpoint, Q2MoversCheckpoint};

/// Saved map-script fields (donor flat `AuthoredMonster` target fields).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedAuthoredTarget {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Entity classname.
    pub classname: String,
    /// This entity's target name.
    pub targetname: String,
    /// Trigger target.
    pub target: String,
    /// Kill target.
    pub killtarget: String,
    /// Center-print message.
    pub message: String,
    /// Trigger delay in seconds.
    pub delay: f64,
}

/// Saved spawn placement with checkpoint actor references.
#[derive(Debug, Clone, PartialEq)]
pub enum SavedMonsterPlacement {
    /// Placed and ready.
    Ready,
    /// Teleporting to an origin.
    Teleport {
        /// Teleport destination.
        origin: Vec3,
    },
    /// Waiting on barriers.
    Waiting {
        /// Pinning barriers.
        barriers: Vec<SavedMonsterBarrier>,
        /// Entity that will release the monster.
        activator: Option<SavedActorId>,
    },
}

/// Saved placement barrier with a checkpoint actor reference.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedMonsterBarrier {
    /// Barrier actor.
    pub actor: SavedActorId,
    /// Barrier origin.
    pub origin: Vec3,
}

/// Saved activation state with checkpoint actor references.
#[derive(Debug, Clone, PartialEq)]
pub enum SavedMonsterActivation {
    /// Active in the world.
    Active,
    /// Dormant until used.
    Dormant,
    /// Scheduled to activate at a time.
    Scheduled {
        /// Activation time in seconds.
        at: f64,
        /// Scheduling activator.
        activator: Option<SavedActorId>,
    },
}

/// Saved authored monster (donor `SavedAuthoredMonster`).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedAuthoredMonster {
    /// Shared map-script fields.
    pub target: SavedAuthoredTarget,
    /// Selected definition.
    pub definition: MonsterDefinitionReference,
    /// Source entity ordinal.
    pub source_ordinal: u32,
    /// Spawn flags bit field.
    pub spawnflags: u32,
    /// Target fired on death.
    pub death_target: String,
    /// Item classname dropped on death.
    pub drop_item: String,
    /// Patrol route name.
    pub route: String,
    /// Resolved patrol goal.
    pub route_goal: Option<SavedActorId>,
    /// Whether the patrol goal resolved.
    pub route_resolved: bool,
    /// Whether the death counted toward the kill total.
    pub counted_death: bool,
    /// Combat target name.
    pub combat_target: String,
    /// Resolved combat goal.
    pub combat_goal: Option<SavedActorId>,
    /// Whether the monster holds its ground in combat.
    pub stand_ground: bool,
    /// Spawn placement.
    pub placement: SavedMonsterPlacement,
    /// Activation state.
    pub activation: SavedMonsterActivation,
}

/// Saved blaster cause (donor `captureProjectiles` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBlasterCause {
    /// Hit actor.
    pub actor: SavedActorId,
    /// Means of death.
    pub means_of_death: i32,
}

/// Saved monster ballistics (donor `Q2Ballistics["captureProjectiles"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedMonsterBallistics {
    /// Blaster causes.
    pub blaster_causes: Vec<SavedBlasterCause>,
}

/// Saved mission-pack monsters (donor `packs` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedMonsterPackCheckpoint {
    /// Selected pack.
    pub pack: Q2MonsterMissionPack,
    /// Pack state.
    pub state: Q2MissionPackMonstersCheckpoint,
}

/// Saved selected-monster source (donor `MonsterSourceCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub enum MonsterSourceCheckpoint {
    /// Quake source.
    Q1 {
        /// Defining provider.
        reference: ProviderReference,
        /// Source frame.
        frame: FrameContext,
        /// Source RNG stream.
        random: RandomCheckpoint,
        /// Entity checkpoint.
        entities: Q1FoundationCheckpoint,
    },
    /// Quake II source.
    Q2 {
        /// Defining provider.
        reference: ProviderReference,
        /// Source frame.
        frame: FrameContext,
        /// Source RNG stream.
        random: RandomCheckpoint,
        /// Entity checkpoint.
        entities: Q2FoundationCheckpoint,
        /// Monster checkpoint.
        monsters: Q2MonstersCheckpoint,
        /// Ballistics checkpoint.
        ballistics: SavedMonsterBallistics,
        /// Mover checkpoint, when the source ran movers.
        movers: Option<Q2MoversCheckpoint>,
        /// Pack checkpoints.
        packs: Vec<SavedMonsterPackCheckpoint>,
    },
}

/// Saved selected monsters (donor `SelectedMonstersCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedMonstersCheckpoint {
    /// Schema version (always 2).
    pub version: u32,
    /// Authored monsters.
    pub authored: Vec<SavedAuthoredMonster>,
    /// Per-source checkpoints.
    pub sources: Vec<MonsterSourceCheckpoint>,
}

/// Parse a `namespace:name` provider the way content catalogs do (donor
/// `readProvider`; parse mirrors the private `provider_id` helper in
/// content `monsters.rs`).
fn provider_reference(reference: ProviderRef) -> ProviderReference {
    let (namespace, name) = match reference.provider.split_once(':') {
        Some((namespace, name)) => (namespace, name),
        None => ("", reference.provider.as_str()),
    };
    ProviderReference {
        provider: ProviderId::new(namespace, name),
        content: ContentId(reference.content),
    }
}

fn read_u32(reader: SaveReader<'_>, minimum: i64) -> Result<u32, WorldError> {
    let value = reader.integer(minimum)?;
    u32::try_from(value).map_err(|_| reader.fail("expected an integer in range"))
}

fn read_authored_target(value: &SaveReader<'_>) -> Result<SavedAuthoredTarget, WorldError> {
    Ok(SavedAuthoredTarget {
        actor: read_saved_actor(value.field("actor"))?,
        classname: value.field("classname").string()?,
        targetname: value.field("targetname").string()?,
        target: value.field("target").string()?,
        killtarget: value.field("killtarget").string()?,
        message: value.field("message").string()?,
        delay: value.field("delay").finite()?,
    })
}

fn read_authored_monster(value: SaveReader<'_>) -> Result<SavedAuthoredMonster, WorldError> {
    let activation = value.field("activation");
    let activation_kind = activation
        .field("kind")
        .choice_str(&["active", "dormant", "scheduled"])?;
    let placement = value.field("placement");
    let placement_kind = if placement.value.is_none() {
        "ready".to_string()
    } else {
        placement.field("kind").choice_str(&["ready", "waiting", "teleport"])?
    };
    let definition = value.field("definition");
    Ok(SavedAuthoredMonster {
        target: read_authored_target(&value)?,
        definition: MonsterDefinitionReference {
            source: provider_reference(read_provider_ref(definition.field("source"))?),
            classname: definition.field("classname").string()?,
        },
        source_ordinal: read_u32(value.field("sourceOrdinal"), 0)?,
        spawnflags: read_u32(value.field("spawnflags"), i64::MIN)?,
        death_target: value.field("deathTarget").string()?,
        drop_item: value.field("dropItem").string()?,
        route: value.field("route").string()?,
        route_goal: value.field("routeGoal").nullable(read_saved_actor)?,
        route_resolved: value.field("routeResolved").boolean()?,
        counted_death: value.field("countedDeath").boolean()?,
        combat_target: value.field("combatTarget").string()?,
        combat_goal: value.field("combatGoal").nullable(read_saved_actor)?,
        stand_ground: value.field("standGround").boolean()?,
        placement: match placement_kind.as_str() {
            "waiting" => SavedMonsterPlacement::Waiting {
                barriers: placement.field("barriers").list(|barrier| {
                    Ok(SavedMonsterBarrier {
                        actor: read_saved_actor(barrier.field("actor"))?,
                        origin: read_vector(barrier.field("origin"))?,
                    })
                })?,
                activator: placement.field("activator").nullable(read_saved_actor)?,
            },
            "teleport" => SavedMonsterPlacement::Teleport {
                origin: read_vector(placement.field("origin"))?,
            },
            _ => SavedMonsterPlacement::Ready,
        },
        activation: match activation_kind.as_str() {
            "scheduled" => SavedMonsterActivation::Scheduled {
                at: activation.field("at").finite()?,
                activator: activation.field("activator").nullable(read_saved_actor)?,
            },
            "dormant" => SavedMonsterActivation::Dormant,
            _ => SavedMonsterActivation::Active,
        },
    })
}

fn bad_save(error: impl ToString) -> WorldError {
    WorldError::BadSave(error.to_string())
}

fn read_source(value: SaveReader<'_>) -> Result<MonsterSourceCheckpoint, WorldError> {
    let reference = provider_reference(read_provider_ref(value.field("reference"))?);
    let frame = read_frame(value.field("frame"))?;
    let random = read_source_random_checkpoint(value.field("random"))?;
    let kind = value.field("kind").choice_str(&["q1", "q2"])?;
    if kind == "q1" {
        return Ok(MonsterSourceCheckpoint::Q1 {
            reference,
            frame,
            random,
            entities: read_q1_foundation_checkpoint(value.field("entities")).map_err(bad_save)?,
        });
    }
    let movers = if value.field("movers").value.is_none() {
        None
    } else {
        Some(read_q2_movers_checkpoint(value.field("movers")).map_err(bad_save)?)
    };
    let packs = if value.field("packs").value.is_none() {
        Vec::new()
    } else {
        value.field("packs").list(|pack| {
            let name = pack.field("pack").choice_str(&["xatrix", "rogue"])?;
            Ok(SavedMonsterPackCheckpoint {
                pack: if name == "xatrix" {
                    Q2MonsterMissionPack::Xatrix
                } else {
                    Q2MonsterMissionPack::Rogue
                },
                state: read_q2_mission_pack_monsters_checkpoint(pack.field("state")).map_err(bad_save)?,
            })
        })?
    };
    let ballistics = value.field("ballistics");
    Ok(MonsterSourceCheckpoint::Q2 {
        reference,
        frame,
        random,
        entities: read_q2_foundation_checkpoint(value.field("entities")).map_err(bad_save)?,
        monsters: read_q2_monsters_checkpoint(value.field("monsters")).map_err(bad_save)?,
        movers,
        packs,
        ballistics: SavedMonsterBallistics {
            blaster_causes: ballistics.field("blasterCauses").list(|cause| {
                let means = cause.field("meansOfDeath").integer(i64::MIN)?;
                Ok(SavedBlasterCause {
                    actor: read_saved_actor(cause.field("actor"))?,
                    means_of_death: i32::try_from(means)
                        .map_err(|_| cause.field("meansOfDeath").fail("expected an integer in range"))?,
                })
            })?,
        },
    })
}

/// Read a selected-monsters checkpoint.
pub fn read_selected_monsters_checkpoint(reader: SaveReader<'_>) -> Result<SelectedMonstersCheckpoint, WorldError> {
    let version = reader.field("version").literal_i64(2)?;
    Ok(SelectedMonstersCheckpoint {
        version: u32::try_from(version).map_err(|_| reader.field("version").fail("expected an integer in range"))?,
        authored: reader.field("authored").list(read_authored_monster)?,
        sources: reader.field("sources").list(read_source)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::q1::foundation::write_q1_foundation_checkpoint;
    use crate::persistence::q2::foundation::write_q2_foundation_checkpoint;
    use crate::persistence::q2::missionpacks::write_q2_mission_pack_monsters_checkpoint;
    use crate::persistence::q2::monsters::write_q2_monsters_checkpoint;
    use crate::persistence::q2::movers::write_q2_movers_checkpoint;
    use qa_core::time::FramePhase;
    use qa_core::time::SourceTime;
    use qa_world::save::records::write_saved_actor;
    use qa_world::save::shared::{write_frame, write_provider_ref, write_vector};
    use qa_world::save::value::{arr, boolean, int, num, obj, str as save_str, SaveJson};

    fn saved(slot: u32, generation: u32) -> SaveJson {
        write_saved_actor(SavedActorId { slot, generation })
    }

    fn glibc_random() -> SaveJson {
        obj(vec![
            ("kind", save_str("glibc-random")),
            ("draws", int(310)),
            ("words", arr((0..31).map(int).collect())),
            ("front", int(3)),
            ("rear", int(0)),
        ])
    }

    fn provider(provider: &str, content: &str) -> SaveJson {
        write_provider_ref(&ProviderRef {
            provider: provider.to_string(),
            content: content.to_string(),
        })
    }

    fn frame() -> SaveJson {
        write_frame(FrameContext {
            frame: 12,
            time: SourceTime::Seconds(1.5),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::EntityThink,
        })
    }

    fn authored(waiting: bool) -> SaveJson {
        obj(vec![
            ("actor", saved(4, 0)),
            (
                "definition",
                obj(vec![
                    ("source", provider("q1:monsters/classic/id1", "q1:id1:e1m1:1")),
                    ("classname", save_str("monster_knight")),
                ]),
            ),
            ("classname", save_str("monster_knight")),
            ("sourceOrdinal", int(7)),
            ("spawnflags", int(32)),
            ("targetname", save_str("knight1")),
            ("target", save_str("path1")),
            ("killtarget", save_str("")),
            ("message", save_str("")),
            ("delay", num(0.0)),
            ("deathTarget", save_str("")),
            ("dropItem", save_str("")),
            ("route", save_str("path1")),
            ("routeGoal", saved(9, 0)),
            ("routeResolved", boolean(true)),
            ("countedDeath", boolean(false)),
            ("combatTarget", save_str("")),
            ("combatGoal", SaveJson::Null),
            ("standGround", boolean(false)),
            (
                "placement",
                if waiting {
                    obj(vec![
                        ("kind", save_str("waiting")),
                        (
                            "barriers",
                            arr(vec![obj(vec![
                                ("actor", saved(11, 0)),
                                ("origin", write_vector(Vec3 { x: 1.0, y: 2.0, z: 3.0 })),
                            ])]),
                        ),
                        ("activator", SaveJson::Null),
                    ])
                } else {
                    obj(vec![
                        ("kind", save_str("teleport")),
                        ("origin", write_vector(Vec3 { x: 4.0, y: 5.0, z: 6.0 })),
                    ])
                },
            ),
            (
                "activation",
                if waiting {
                    obj(vec![
                        ("kind", save_str("scheduled")),
                        ("at", num(12.5)),
                        ("activator", saved(13, 0)),
                    ])
                } else {
                    obj(vec![("kind", save_str("active"))])
                },
            ),
        ])
    }

    #[test]
    fn reads_q1_checkpoint_with_waiting_monster() {
        let entities = write_q1_foundation_checkpoint(&Q1FoundationCheckpoint {
            provider: "q1:official".to_string(),
            precache_phase: "loading".to_string(),
            precache_models: Vec::new(),
            precache_sounds: Vec::new(),
            edition: "classic".to_string(),
            time: 0.0,
            frame_seconds: 0.1,
            force_retouch: 0.0,
            sequence: 0,
            next_dynamic_slot: 9,
            total_secrets: 0.0,
            found_secrets: 0.0,
            total_monsters: 0.0,
            killed_monsters: 0.0,
            world_type: 0.0,
            map_name: "e1m1".to_string(),
            basis: (
                Vec3 { x: 1.0, y: 0.0, z: 0.0 },
                Vec3 { x: 0.0, y: 1.0, z: 0.0 },
                Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            ),
            world: None,
            sight_entity: None,
            sight_time: 0.0,
            intermission: None,
            entities: Vec::new(),
            players: Vec::new(),
            extensions: Vec::new(),
        });
        let json = obj(vec![
            ("version", int(2)),
            ("authored", arr(vec![authored(true)])),
            (
                "sources",
                arr(vec![obj(vec![
                    ("reference", provider("q1:official", "q1:id1:e1m1:1")),
                    ("frame", frame()),
                    ("random", glibc_random()),
                    ("kind", save_str("q1")),
                    ("entities", entities),
                ])]),
            ),
        ]);
        let checkpoint = read_selected_monsters_checkpoint(SaveReader::new(&json)).expect("read");
        assert_eq!(checkpoint.version, 2);
        assert_eq!(checkpoint.authored.len(), 1);
        let entry = &checkpoint.authored[0];
        assert_eq!(entry.target.actor, SavedActorId { slot: 4, generation: 0 });
        assert_eq!(entry.target.classname, "monster_knight");
        assert_eq!(entry.definition.classname, "monster_knight");
        assert_eq!(entry.source_ordinal, 7);
        assert_eq!(entry.spawnflags, 32);
        assert_eq!(entry.route_goal, Some(SavedActorId { slot: 9, generation: 0 }));
        assert!(entry.route_resolved);
        assert!(matches!(
            entry.placement,
            SavedMonsterPlacement::Waiting { ref barriers, activator: None } if barriers.len() == 1
        ));
        assert!(matches!(
            entry.activation,
            SavedMonsterActivation::Scheduled { at, .. } if at == 12.5
        ));
        assert_eq!(checkpoint.sources.len(), 1);
        let MonsterSourceCheckpoint::Q1 {
            reference,
            frame,
            entities,
            ..
        } = &checkpoint.sources[0]
        else {
            panic!("expected a q1 source");
        };
        assert_eq!(reference.provider, ProviderId::new("q1", "official"));
        assert_eq!(frame.frame, 12);
        assert_eq!(entities.sequence, 0);
    }

    fn q2_source(with_movers: bool, with_packs: bool) -> SaveJson {
        use crate::persistence::q2::foundation::Q2FoundationCounters;
        use crate::persistence::q2::monsters::Q2MonsterPerception;
        let entities = write_q2_foundation_checkpoint(&Q2FoundationCheckpoint {
            next_source_slot: 9,
            sequence: 0,
            freed_slots: Vec::new(),
            counters: Q2FoundationCounters {
                total_secrets: 0.0,
                found_secrets: 0.0,
                total_goals: 0.0,
                found_goals: 0.0,
                total_monsters: 0.0,
                killed_monsters: 0.0,
                server_flags: 0.0,
            },
            entities: Vec::new(),
        });
        let monsters = write_q2_monsters_checkpoint(&Q2MonstersCheckpoint {
            actors: Vec::new(),
            perception: Q2MonsterPerception {
                sight_client: None,
                sight: None,
                alerted: Vec::new(),
                primary: None,
                secondary: None,
                noises: Vec::new(),
                trails: Vec::new(),
                player_origins: Vec::new(),
                hostile: Vec::new(),
                last_frame: None,
            },
        });
        let mut source = vec![
            ("reference", provider("q2:official", "q2:baseq2:base1:1")),
            ("frame", frame()),
            ("random", glibc_random()),
            ("kind", save_str("q2")),
            ("entities", entities),
            ("monsters", monsters),
            (
                "ballistics",
                obj(vec![(
                    "blasterCauses",
                    arr(vec![obj(vec![("actor", saved(21, 0)), ("meansOfDeath", int(9))])]),
                )]),
            ),
        ];
        if with_movers {
            source.push((
                "movers",
                write_q2_movers_checkpoint(&Q2MoversCheckpoint {
                    doors: Vec::new(),
                    trains: Vec::new(),
                    linear: Vec::new(),
                    angular: Vec::new(),
                }),
            ));
        }
        if with_packs {
            let state = Q2MissionPackMonstersCheckpoint {
                flyer_next_move: "none".to_string(),
                hints: None,
                widow_shots_fired: 0.0,
                widow_damage_multiplier: 1,
                actors: Vec::new(),
            };
            source.push((
                "packs",
                arr(vec![obj(vec![
                    ("pack", save_str("xatrix")),
                    ("state", write_q2_mission_pack_monsters_checkpoint(&state)),
                ])]),
            ));
        }
        obj(source)
    }

    #[test]
    fn reads_q2_checkpoint_with_absent_movers_and_packs() {
        let mut entry = authored(false);
        let SaveJson::Object(fields) = &mut entry else {
            panic!("authored object")
        };
        fields.retain(|(key, _)| key != "placement");
        let json = obj(vec![
            ("version", int(2)),
            ("authored", arr(vec![entry])),
            ("sources", arr(vec![q2_source(false, false)])),
        ]);
        let checkpoint = read_selected_monsters_checkpoint(SaveReader::new(&json)).expect("read");
        assert!(matches!(checkpoint.authored[0].placement, SavedMonsterPlacement::Ready));
        assert!(matches!(
            checkpoint.authored[0].activation,
            SavedMonsterActivation::Active
        ));
        let MonsterSourceCheckpoint::Q2 {
            movers,
            packs,
            ballistics,
            random,
            ..
        } = &checkpoint.sources[0]
        else {
            panic!("expected a q2 source");
        };
        assert!(movers.is_none());
        assert!(packs.is_empty());
        assert_eq!(
            ballistics.blaster_causes,
            vec![SavedBlasterCause {
                actor: SavedActorId {
                    slot: 21,
                    generation: 0
                },
                means_of_death: 9
            }]
        );
        assert!(matches!(random, RandomCheckpoint::Glibc(_)));
    }

    #[test]
    fn reads_q2_movers_and_pack_checkpoints() {
        let json = obj(vec![
            ("version", int(2)),
            ("authored", arr(Vec::new())),
            ("sources", arr(vec![q2_source(true, true)])),
        ]);
        let checkpoint = read_selected_monsters_checkpoint(SaveReader::new(&json)).expect("read");
        let MonsterSourceCheckpoint::Q2 { movers, packs, .. } = &checkpoint.sources[0] else {
            panic!("expected a q2 source");
        };
        let movers = movers.as_ref().expect("movers");
        assert!(movers.doors.is_empty());
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].pack, Q2MonsterMissionPack::Xatrix);
        assert_eq!(packs[0].state.widow_damage_multiplier, 1);
    }

    #[test]
    fn rejects_bad_version_and_random_stream() {
        let json = obj(vec![
            ("version", int(1)),
            ("authored", arr(Vec::new())),
            ("sources", arr(Vec::new())),
        ]);
        assert!(read_selected_monsters_checkpoint(SaveReader::new(&json)).is_err());
        let json = obj(vec![
            ("version", int(2)),
            ("authored", arr(Vec::new())),
            (
                "sources",
                arr(vec![obj(vec![
                    ("reference", provider("q1:official", "q1:id1:e1m1:1")),
                    ("frame", frame()),
                    ("random", obj(vec![("kind", save_str("bogus"))])),
                    ("kind", save_str("q1")),
                    ("entities", obj(Vec::new())),
                ])]),
            ),
        ]);
        assert!(read_selected_monsters_checkpoint(SaveReader::new(&json)).is_err());
    }
}
