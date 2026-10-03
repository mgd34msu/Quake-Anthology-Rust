//! Port of Quake-Anthology-TS `src/content/composition/q2/save.ts`
//!
//! Q2 product restore bridge: decode the `q2:*` provider records from a
//! simulation save image and convert each persistence checkpoint into the
//! content [`Q2ProductCheckpoint`] that `restore_q2_product` consumes.
//! Mirrors donor `restoreQ2Product` (save.ts): the `q2:composition`
//! record is validated against the live runtime first (fail closed
//! instead of the content panics), then every module record is decoded
//! and converted, then the assembled checkpoint restores through the
//! normal content path.
//!
//! The foundation module is converted by the caller with the existing
//! `saved_to_foundation` helper and passed in; every other module is
//! decoded and converted here. Modules whose donor readers return the
//! content shape directly (rerelease, CTF, LMCTF) are read straight into
//! the content checkpoints with `SaveReader`; twin-shaped modules go
//! through the persistence decoders plus a converter, following the
//! `q2_character_bridge.rs` pattern.
//!
//! Numeric policy: donor `number()` fields convert with `as` truncation
//! (donor values are integral; matches the character bridge); donor
//! `integer()` fields convert with `try_from` and fail closed on range
//! errors. Record versions are set literally because the decoders
//! already validate them.

use qa_content::q2::composition::product::Q2ProductRuntime;
use qa_content::q2::composition::save::{
    Q2ProductCheckpoint, Q2ProductExpansionCheckpoint, Q2ProductMatchCheckpoint, Q2ProductRereleaseCheckpoint,
};
use qa_content::q2::composition::types::Q2MatchSelection;
use qa_content::q2::foundation::checkpoint::Q2FoundationCheckpoint as ContentFoundationCheckpoint;
use qa_content::q2::foundation::host::Q2Edition;
use qa_content::q2::missionpacks::types::Q2MissionPack;
use qa_world::save::records::read_saved_actor;
use qa_world::save::shared::read_vector;
use qa_world::save::value::{decode_checkpoint_value, SaveReader};
use qa_world::WorldError;

use super::save::SimulationSaveImage;

/// Runtime configuration the product restore branches on (donor
/// `source.configuration`, `source.match.selection`, and the optional
/// module handles).
#[derive(Debug, Clone)]
pub struct Q2ProductRestoreConfig {
    /// Composition program name.
    pub program: String,
    /// Match selection.
    pub selection: Q2MatchSelection,
    /// Whether the mission-pack armory is bound.
    pub has_armory: bool,
    /// Bound expansion packs.
    pub expansions: Vec<Q2MissionPack>,
    /// Whether the rerelease slice is bound.
    pub has_rerelease: bool,
}

/// Read the restore configuration from a live product runtime.
pub fn restore_config(runtime: &Q2ProductRuntime) -> Q2ProductRestoreConfig {
    Q2ProductRestoreConfig {
        program: runtime.program.clone(),
        selection: runtime.selection.clone(),
        has_armory: runtime.armory.is_some(),
        expansions: runtime.expansions.iter().map(|expansion| expansion.pack).collect(),
        has_rerelease: runtime.rerelease.is_some(),
    }
}

/// Invalid-save error.
fn bad_save(error: impl ToString) -> WorldError {
    WorldError::BadSave(error.to_string())
}

/// Read a required provider record's bytes (donor `read`).
fn required_bytes<'a>(image: &'a SimulationSaveImage, schema: &str) -> Result<&'a [u8], WorldError> {
    super::save::simulation_provider_checkpoint(image, schema)
        .map(|record| record.bytes.as_slice())
        .map_err(bad_save)
}

/// Range-checked `i64` to `i32` conversion.
fn to_i32(value: i64, what: &str) -> Result<i32, WorldError> {
    i32::try_from(value).map_err(|_| WorldError::BadSave(format!("{what} is out of range")))
}

/// Range-checked `u64` to `i32` conversion.
fn u64_to_i32(value: u64, what: &str) -> Result<i32, WorldError> {
    i32::try_from(value).map_err(|_| WorldError::BadSave(format!("{what} is out of range")))
}

/// Range-checked `i64` to `u8` conversion.
fn to_u8(value: i64, what: &str) -> Result<u8, WorldError> {
    u8::try_from(value).map_err(|_| WorldError::BadSave(format!("{what} is out of range")))
}

/// Parsed `q2:composition` record (donor `selection`).
struct Q2Composition {
    /// Saved edition.
    edition: Q2Edition,
    /// Saved program.
    program: String,
    /// Saved match selection.
    selection: Q2MatchSelection,
    /// Saved deathmatch flags.
    deathmatch_flags: i32,
}

/// Read and validate the `q2:composition` record (donor save.ts 44-55).
fn read_composition(
    image: &SimulationSaveImage,
    edition: Q2Edition,
    config: &Q2ProductRestoreConfig,
) -> Result<Q2Composition, WorldError> {
    let bytes = required_bytes(image, "q2:composition")?;
    let payload = decode_checkpoint_value(bytes)?;
    let reader = SaveReader::at(&payload, "q2-composition");
    let saved_edition = match reader.field("edition").choice_str(&["classic", "rerelease"])?.as_str() {
        "classic" => Q2Edition::Classic,
        _ => Q2Edition::Rerelease,
    };
    let program = reader
        .field("program")
        .choice_str(&["baseq2", "xatrix", "rogue", "mg2", "n64"])?;
    let saved = reader.field("match");
    let kind = saved
        .field("kind")
        .choice_str(&["standard", "tag", "deathball", "ctf", "lmctf"])?;
    let selection = match kind.as_str() {
        "tag" => Q2MatchSelection::Tag,
        "deathball" => Q2MatchSelection::Deathball {
            team1_skin: saved.field("team1Skin").string()?,
            team2_skin: saved.field("team2Skin").string()?,
            goal_limit: saved.field("goalLimit").number()?,
        },
        "ctf" => Q2MatchSelection::Ctf,
        "lmctf" => Q2MatchSelection::Lmctf { travel: None },
        _ => Q2MatchSelection::Standard,
    };
    if saved_edition != edition || program != config.program {
        return Err(WorldError::BadSave(
            "Q2 checkpoint belongs to a different source product".to_string(),
        ));
    }
    if selection.kind() != config.selection.kind() {
        return Err(WorldError::BadSave(
            "Q2 checkpoint belongs to a different source match mode".to_string(),
        ));
    }
    if let (
        Q2MatchSelection::Deathball {
            team1_skin,
            team2_skin,
            goal_limit,
        },
        Q2MatchSelection::Deathball {
            team1_skin: saved_team1,
            team2_skin: saved_team2,
            goal_limit: saved_goal,
        },
    ) = (&config.selection, &selection)
    {
        if saved_team1 != team1_skin || saved_team2 != team2_skin || saved_goal != goal_limit {
            return Err(WorldError::BadSave(
                "Q2 checkpoint belongs to different DeathBall rules".to_string(),
            ));
        }
    }
    Ok(Q2Composition {
        edition: saved_edition,
        program,
        selection,
        deathmatch_flags: to_i32(reader.field("deathmatchFlags").integer(0)?, "deathmatch flags")?,
    })
}

/// Expansion pack schema infix (donor `Q2MissionPack`, `xatrix` | `rogue`).
fn pack_name(pack: Q2MissionPack) -> &'static str {
    match pack {
        Q2MissionPack::Xatrix => "xatrix",
        Q2MissionPack::Rogue => "rogue",
    }
}

/// Build the content product checkpoint from the save providers (donor
/// `restoreQ2Product` record reads, save.ts 56-91).
///
/// The caller passes the converted foundation checkpoint plus the live
/// composition edition; identity is validated before any module state
/// is returned, so a mismatch fails closed without touching the game.
pub fn read_q2_product_checkpoint(
    image: &SimulationSaveImage,
    config: &Q2ProductRestoreConfig,
    edition: Q2Edition,
    foundation: ContentFoundationCheckpoint,
) -> Result<Q2ProductCheckpoint, WorldError> {
    let composition = read_composition(image, edition, config)?;
    let items = {
        let bytes = required_bytes(image, "q2:items")?;
        let saved = crate::persistence::q2::items::decode_q2_items_checkpoint(bytes).map_err(bad_save)?;
        convert_persistence_q2_items(&saved)?
    };
    let movers = {
        let bytes = required_bytes(image, "q2:movers")?;
        let saved = crate::persistence::q2::movers::decode_q2_movers_checkpoint(bytes).map_err(bad_save)?;
        convert_persistence_q2_movers(&saved)?
    };
    let monsters = {
        let bytes = required_bytes(image, "q2:monsters")?;
        let saved = crate::persistence::q2::monsters::decode_q2_monsters_checkpoint(bytes).map_err(bad_save)?;
        convert_persistence_q2_monsters(&saved)?
    };
    let weapons = {
        let bytes = required_bytes(image, "q2:weapons")?;
        let saved = crate::persistence::q2::weapons::decode_q2_weapons_checkpoint(bytes).map_err(bad_save)?;
        convert_persistence_q2_weapons(&saved)?
    };
    let players = {
        let bytes = required_bytes(image, "q2:players")?;
        let saved = crate::persistence::q2::players::decode_q2_players_checkpoint(bytes).map_err(bad_save)?;
        convert_persistence_q2_players(&saved)?
    };
    let base_entities = {
        let bytes = required_bytes(image, "q2:base-entities")?;
        let saved =
            crate::persistence::q2::base_entities::decode_q2_base_entities_checkpoint(bytes).map_err(bad_save)?;
        convert_persistence_q2_base_entities(&saved)?
    };
    let missionpack_items = if config.has_armory {
        let bytes = required_bytes(image, "q2:missionpack-items")?;
        let saved =
            crate::persistence::q2::missionpacks::decode_q2_mission_pack_items_checkpoint(bytes).map_err(bad_save)?;
        Some(convert_persistence_q2_missionpack_items(&saved))
    } else {
        None
    };
    let mut expansions = Vec::new();
    for pack in &config.expansions {
        let monsters = {
            let bytes = required_bytes(image, &format!("q2:{}-monsters", pack_name(*pack)))?;
            let saved = crate::persistence::q2::missionpacks::decode_q2_mission_pack_monsters_checkpoint(bytes)
                .map_err(bad_save)?;
            convert_persistence_q2_missionpack_monsters(&saved)?
        };
        let entities = {
            let bytes = required_bytes(image, &format!("q2:{}-entities", pack_name(*pack)))?;
            let saved =
                crate::persistence::q2::missionpacks::decode_q2_rogue_entities_checkpoint(bytes).map_err(bad_save)?;
            convert_persistence_q2_rogue_entities(&saved)
        };
        expansions.push(Q2ProductExpansionCheckpoint {
            pack: *pack,
            monsters,
            entities,
        });
    }
    let rerelease = if config.has_rerelease {
        let players = {
            let bytes = required_bytes(image, "q2:rerelease-players")?;
            let payload = decode_checkpoint_value(bytes)?;
            read_q2_rerelease_players_checkpoint(SaveReader::at(&payload, "q2-rerelease-players"))?
        };
        let entities = {
            let bytes = required_bytes(image, "q2:rerelease-entities")?;
            let payload = decode_checkpoint_value(bytes)?;
            read_q2_rerelease_module_checkpoint(SaveReader::at(&payload, "q2-rerelease-module"))?
        };
        Some(Q2ProductRereleaseCheckpoint { players, entities })
    } else {
        None
    };
    let match_state = match &config.selection {
        Q2MatchSelection::Standard => None,
        Q2MatchSelection::Tag => {
            let bytes = required_bytes(image, "q2:match")?;
            let saved = crate::persistence::q2::missionpacks::decode_q2_tag_checkpoint(bytes).map_err(bad_save)?;
            Some(Q2ProductMatchCheckpoint::Tag(convert_persistence_q2_tag(&saved)))
        }
        Q2MatchSelection::Deathball { .. } => {
            let bytes = required_bytes(image, "q2:match")?;
            let saved =
                crate::persistence::q2::missionpacks::decode_q2_death_ball_checkpoint(bytes).map_err(bad_save)?;
            Some(Q2ProductMatchCheckpoint::Deathball(convert_persistence_q2_deathball(
                &saved,
            )))
        }
        Q2MatchSelection::Ctf => {
            let bytes = required_bytes(image, "q2:match")?;
            let payload = decode_checkpoint_value(bytes)?;
            Some(Q2ProductMatchCheckpoint::Ctf(read_q2_ctf_checkpoint(SaveReader::at(
                &payload, "q2-ctf",
            ))?))
        }
        Q2MatchSelection::Lmctf { .. } => {
            let bytes = required_bytes(image, "q2:match")?;
            let payload = decode_checkpoint_value(bytes)?;
            Some(Q2ProductMatchCheckpoint::Lmctf(read_q2_lmctf_checkpoint(
                SaveReader::at(&payload, "q2-lmctf"),
            )?))
        }
    };
    Ok(Q2ProductCheckpoint {
        edition: composition.edition,
        program: composition.program,
        match_selection: composition.selection,
        deathmatch_flags: composition.deathmatch_flags,
        foundation,
        items,
        movers,
        monsters,
        weapons,
        players,
        base_entities,
        missionpack_items,
        expansions,
        rerelease,
        match_state,
    })
}

/// Convert a persistence Q2 items checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
pub fn convert_persistence_q2_items(
    checkpoint: &crate::persistence::q2::items::Q2ItemsCheckpoint,
) -> Result<qa_content::q2::foundation::items::Q2ItemsCheckpoint, WorldError> {
    use qa_content::q2::foundation::items::{
        Q2ItemsCheckpoint as Content, Q2PickupCheckpoint as ContentPickup, Q2PlayerPowerups,
        Q2PowerCheckpoint as ContentPower,
    };
    Ok(Content {
        power_cube_count: checkpoint.power_cube_count as i32,
        pickups: checkpoint
            .pickups
            .iter()
            .map(|pickup| ContentPickup {
                actor: pickup.actor,
                classname: pickup.classname.clone(),
                targets_used: pickup.targets_used,
                retained: pickup.retained,
                expires_at: pickup.expires_at,
            })
            .collect(),
        powers: checkpoint
            .powers
            .iter()
            .map(|power| ContentPower {
                actor: power.actor,
                state: Q2PlayerPowerups {
                    quad_until: power.quad_until,
                    invulnerability_until: power.invulnerability_until,
                    breather_until: power.breather_until,
                    enviro_until: power.enviro_until,
                },
            })
            .collect(),
        power_armor_bindings: checkpoint.power_armor_bindings.clone(),
    })
}

/// Convert a persistence door phase word (donor `phase` choice, q2-movers.ts 18).
fn convert_door_phase(phase: &str) -> Result<qa_content::q2::foundation::movers::DoorPhase, WorldError> {
    use qa_content::q2::foundation::movers::DoorPhase;
    match phase {
        "bottom" => Ok(DoorPhase::Bottom),
        "up" => Ok(DoorPhase::Up),
        "top" => Ok(DoorPhase::Top),
        "down" => Ok(DoorPhase::Down),
        _ => Err(WorldError::BadSave(format!("unknown Q2 door phase {phase:?}"))),
    }
}

/// Convert a persistence motion curve into the content shape.
#[allow(clippy::cast_possible_truncation)]
fn convert_motion_curve(
    curve: &crate::persistence::q2::movers::Q2MotionCurve,
) -> qa_content::q2::foundation::motion::LinearMoveCurve {
    qa_content::q2::foundation::motion::LinearMoveCurve {
        positions: curve.positions.iter().map(|position| *position as f32).collect(),
        frame: curve.frame as usize,
        subframe: curve.subframe as usize,
        subframes: curve.subframes,
    }
}

/// Convert persistence linear-motion entries into the content checkpoint.
fn convert_linear_motion(
    entries: &[crate::persistence::q2::movers::Q2LinearMotionEntry],
) -> qa_content::q2::foundation::motion::Q2LinearMotionCheckpoint {
    use qa_content::q2::foundation::motion::LinearMoveCheckpoint;
    entries
        .iter()
        .map(|entry| LinearMoveCheckpoint {
            actor: entry.actor,
            direction: entry.state.direction,
            destination: entry.state.destination,
            reference: entry.state.reference,
            remaining: entry.state.remaining,
            current_speed: entry.state.current_speed,
            move_speed: entry.state.move_speed,
            next_speed: entry.state.next_speed,
            decel_distance: entry.state.decel_distance,
            done: entry.state.done.clone(),
            curve: entry.state.curve.as_ref().map(convert_motion_curve),
        })
        .collect()
}

/// Convert persistence angular-motion entries into the content checkpoint.
fn convert_angular_motion(
    entries: &[crate::persistence::q2::movers::Q2AngularMotionEntry],
) -> qa_content::q2::foundation::angular_motion::Q2AngularMotionCheckpoint {
    use qa_content::q2::foundation::angular_motion::AngularMoveCheckpoint;
    entries
        .iter()
        .map(|entry| AngularMoveCheckpoint {
            actor: entry.actor,
            destination: entry.destination,
            speed: entry.speed,
            done: entry.done.clone(),
        })
        .collect()
}

/// Convert a persistence Q2 movers checkpoint into the content shape.
pub fn convert_persistence_q2_movers(
    checkpoint: &crate::persistence::q2::movers::Q2MoversCheckpoint,
) -> Result<qa_content::q2::foundation::movers::Q2MoversCheckpoint, WorldError> {
    use qa_content::q2::foundation::movers::{
        DoorCheckpoint, DoorCheckpointState, Q2MoversCheckpoint as Content, TrainCheckpoint,
    };
    let mut doors = Vec::with_capacity(checkpoint.doors.len());
    for door in &checkpoint.doors {
        doors.push(DoorCheckpoint {
            actor: door.actor,
            state: DoorCheckpointState {
                start: door.state.start,
                end: door.state.end,
                distance: door.state.distance,
                button: door.state.button,
                angular: door.state.angular,
                water: door.state.water,
                safe_direction: door.state.safe_direction,
                water_divisor: door.state.water_divisor,
                reversed: door.state.reversed,
                activated: door.state.activated,
                phase: convert_door_phase(&door.state.phase)?,
                debounce: door.state.debounce,
            },
            master: door.master,
            team: door.team.clone(),
        });
    }
    Ok(Content {
        doors,
        trains: checkpoint
            .trains
            .iter()
            .map(|train| TrainCheckpoint {
                actor: train.actor,
                destination: train.destination,
                debounce: train.debounce,
                ship: train.ship,
            })
            .collect(),
        linear: convert_linear_motion(&checkpoint.linear),
        angular: convert_angular_motion(&checkpoint.angular),
    })
}

/// Convert a persistence monster weapon word (donor `weapon` choice, q2-monsters.ts 18).
fn convert_monster_weapon(
    weapon: &str,
) -> Result<qa_content::q2::foundation::monsters::types::MonsterWeapon, WorldError> {
    use qa_content::q2::foundation::monsters::types::MonsterWeapon;
    match weapon {
        "blaster" => Ok(MonsterWeapon::Blaster),
        "shotgun" => Ok(MonsterWeapon::Shotgun),
        "machinegun" => Ok(MonsterWeapon::Machinegun),
        _ => Err(WorldError::BadSave(format!("unknown Q2 monster weapon {weapon:?}"))),
    }
}

/// Convert a persistence monster locomotion word (donor `locomotion` choice).
fn convert_monster_locomotion(
    locomotion: &str,
) -> Result<qa_content::q2::foundation::monsters::types::MonsterLocomotion, WorldError> {
    use qa_content::q2::foundation::monsters::types::MonsterLocomotion;
    match locomotion {
        "walk" => Ok(MonsterLocomotion::Walk),
        "fly" => Ok(MonsterLocomotion::Fly),
        "swim" => Ok(MonsterLocomotion::Swim),
        "stationary" => Ok(MonsterLocomotion::Stationary),
        _ => Err(WorldError::BadSave(format!(
            "unknown Q2 monster locomotion {locomotion:?}"
        ))),
    }
}

/// Convert a persistence monster spawner word (donor `spawnedBy` choice).
fn convert_monster_spawner(
    spawner: &str,
) -> Result<qa_content::q2::foundation::monsters::types::MonsterSpawner, WorldError> {
    use qa_content::q2::foundation::monsters::types::MonsterSpawner;
    match spawner {
        "none" => Ok(MonsterSpawner::None),
        "carrier" => Ok(MonsterSpawner::Carrier),
        "medic" => Ok(MonsterSpawner::Medic),
        "widow" => Ok(MonsterSpawner::Widow),
        _ => Err(WorldError::BadSave(format!("unknown Q2 monster spawner {spawner:?}"))),
    }
}

/// Convert a persistence monster attack-state word (donor `attackState` choice).
fn convert_monster_attack_state(
    state: &str,
) -> Result<qa_content::q2::foundation::monsters::types::MonsterAttackState, WorldError> {
    use qa_content::q2::foundation::monsters::types::MonsterAttackState;
    match state {
        "straight" => Ok(MonsterAttackState::Straight),
        "sliding" => Ok(MonsterAttackState::Sliding),
        "melee" => Ok(MonsterAttackState::Melee),
        "missile" => Ok(MonsterAttackState::Missile),
        "blind" => Ok(MonsterAttackState::Blind),
        _ => Err(WorldError::BadSave(format!(
            "unknown Q2 monster attack state {state:?}"
        ))),
    }
}

/// Convert a persistence monster power-armor word (donor `initialPowerArmorType` choice).
fn convert_monster_power_armor(
    armor: &str,
) -> Result<qa_content::q2::foundation::monsters::types::MonsterPowerArmor, WorldError> {
    use qa_content::q2::foundation::monsters::types::MonsterPowerArmor;
    match armor {
        "none" => Ok(MonsterPowerArmor::None),
        "screen" => Ok(MonsterPowerArmor::Screen),
        "shield" => Ok(MonsterPowerArmor::Shield),
        _ => Err(WorldError::BadSave(format!("unknown Q2 monster power armor {armor:?}"))),
    }
}

/// Convert a persistence monster state into the content checkpoint.
///
/// Move names and saved actor handles travel at the checkpoint level;
/// content `restore_monsters` resolves them against the species tables
/// and the actor registry, so the embedded live state carries
/// stand-ins for the current/next move and the live actor handles.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_lines)]
fn convert_monster_state(
    saved: &crate::persistence::q2::monsters::Q2MonsterStateCheckpoint,
) -> Result<qa_content::q2::foundation::monsters::checkpoint::Q2MonsterStateCheckpoint, WorldError> {
    use qa_content::q2::foundation::monsters::checkpoint::{Q2MonsterStateCheckpoint as Content, SavedSoundTarget};
    use qa_content::q2::foundation::monsters::types::{MonsterMove, MonsterPathing, MonsterState};
    Ok(Content {
        movement: saved.move_name.clone(),
        next_move: saved.next_move.clone(),
        sound_target: saved.sound_target.as_ref().map(|target| SavedSoundTarget {
            actor: target.actor,
            owner: target.owner,
            origin: target.origin,
            time: target.time,
        }),
        old_enemy: saved.old_enemy,
        move_target: saved.move_target,
        commander: saved.commander,
        state: MonsterState {
            kind: saved.kind.clone(),
            weapon: convert_monster_weapon(&saved.weapon)?,
            locomotion: convert_monster_locomotion(&saved.locomotion)?,
            has_melee: saved.has_melee,
            has_ranged_attack: saved.has_ranged_attack,
            has_idle: saved.has_idle,
            has_search: saved.has_search,
            blind_fire: saved.blind_fire,
            good_guy: saved.good_guy,
            target_anger: saved.target_anger,
            ignore_shots: saved.ignore_shots,
            do_not_count: saved.do_not_count,
            spawned_by: convert_monster_spawner(&saved.spawned_by)?,
            commander: None,
            monster_slots: saved.monster_slots as i32,
            monster_used: saved.monster_used as i32,
            brutal: saved.brutal,
            medic: saved.medic,
            resurrecting: saved.resurrecting,
            current_move: MonsterMove::default(),
            next_move: None,
            next_frame: saved.next_frame as i32,
            next_move_time: saved.next_move_time,
            scale: saved.scale,
            gib_health: saved.gib_health,
            initial_power_armor: convert_monster_power_armor(&saved.initial_power_armor_type)?,
            max_power_armor_power: saved.max_power_armor_power,
            base_health: saved.base_health,
            health_scaling: f64::from(to_i32(saved.health_scaling, "monster health scaling")?),
            can_take_damage: saved.can_take_damage,
            dead: saved.dead,
            corpse: saved.corpse,
            gibbed: saved.gibbed,
            stand_ground: saved.stand_ground,
            temporary_stand_ground: saved.temporary_stand_ground,
            hold_frame: saved.hold_frame,
            ducked: saved.ducked,
            dodging: saved.dodging,
            charging: saved.charging,
            manual_steering: saved.manual_steering,
            combat_point: saved.combat_point,
            attack_state: convert_monster_attack_state(&saved.attack_state)?,
            lefty: saved.lefty,
            ideal_yaw: saved.ideal_yaw,
            yaw_speed: saved.yaw_speed,
            pause_time: saved.pause_time,
            idle_time: saved.idle_time,
            pain_time: saved.pain_time,
            fire_wait: saved.fire_wait,
            duck_wait: saved.duck_wait,
            next_duck_time: saved.next_duck_time,
            dodge_time: saved.dodge_time,
            attack_finished: saved.attack_finished,
            check_attack_time: saved.check_attack_time,
            strafe_time: saved.strafe_time,
            had_visibility: saved.had_visibility,
            close_sight_tripped: saved.close_sight_tripped,
            melee_time: saved.melee_time,
            search_time: saved.search_time,
            trail_time: saved.trail_time,
            show_hostile: saved.show_hostile,
            last_sighting: saved.last_sighting,
            saved_goal: saved.saved_goal,
            lost_sight: saved.lost_sight,
            pursue_next: saved.pursue_next,
            pursue_temporary: saved.pursue_temporary,
            pursuit_last_seen: saved.pursuit_last_seen,
            blind_fire_target: saved.blind_fire_target,
            blind_fire_delay: saved.blind_fire_delay,
            sound_target: None,
            old_enemy: None,
            move_target: None,
            combat_target: saved.combat_target.clone(),
            cocked: saved.cocked,
            force_refire: saved.force_refire,
            normal_height: saved.normal_height,
            water_level: to_u8(saved.water_level, "monster water level")?,
            water_type: saved.water_type as i32,
            last_link_count: saved.last_link_count as i32,
            air_finished: saved.air_finished,
            environmental_damage_time: saved.environmental_damage_time,
            jump_time: saved.jump_time,
            flies_time: saved.flies_time,
            alternate_fly: saved.fly.alternate_fly,
            fly_min_distance: saved.fly.fly_min_distance,
            fly_max_distance: saved.fly.fly_max_distance,
            fly_acceleration: saved.fly.fly_acceleration,
            fly_speed: saved.fly.fly_speed,
            fly_ideal_position: saved.fly.fly_ideal_position,
            fly_position_time: saved.fly.fly_position_time,
            fly_buzzard: saved.fly.fly_buzzard,
            fly_above: saved.fly.fly_above,
            fly_pinned: saved.fly.fly_pinned,
            fly_thrusters: saved.fly.fly_thrusters,
            fly_recovery_time: saved.fly.fly_recovery_time,
            fly_recovery_direction: saved.fly.fly_recovery_direction,
            hint_path: saved.fly.hint_path,
            pathing: saved.fly.pathing.as_ref().map(|pathing| MonsterPathing {
                first_move_point: pathing.first_move_point,
                second_move_point: pathing.second_move_point,
                traversal_pending: pathing.traversal_pending,
            }),
        },
    })
}

/// Convert a `namespace:name` provider reference (donor `namespaced`).
fn convert_provider(text: &str) -> Result<qa_core::identity::ProviderId, WorldError> {
    use qa_core::identity::ProviderId;
    text.split_once(':')
        .map(|(namespace, name)| ProviderId::new(namespace, name))
        .ok_or_else(|| WorldError::BadSave(format!("expected namespace:name, found {text:?}")))
}

/// Convert a persistence native cause.
fn convert_native(
    saved: &crate::persistence::q2::foundation::Q2NativeCause,
) -> Result<qa_content::q2::support::contracts::Q2NativeCause, WorldError> {
    use crate::persistence::q2::foundation::Q2NativeCause as Saved;
    use qa_content::q2::support::contracts::{Q2NativeCause as Content, Q2NativeGame as ContentGame};
    match saved {
        Saved::Classic { game, value } => Ok(Content::Classic {
            game: match game.as_str() {
                "base" => ContentGame::Base,
                "xatrix" => ContentGame::Xatrix,
                "rogue" => ContentGame::Rogue,
                "ctf" => ContentGame::Ctf,
                _ => return Err(WorldError::BadSave(format!("unknown native attack game {game:?}"))),
            },
            value: to_i32(*value, "native attack value")?,
        }),
        Saved::Rerelease {
            id,
            friendly_fire,
            no_point_loss,
        } => Ok(Content::Rerelease {
            id: i32::try_from(*id)
                .map_err(|_| WorldError::BadSave("native attack cause id is out of range".to_string()))?,
            friendly_fire: *friendly_fire,
            no_point_loss: *no_point_loss,
        }),
    }
}

/// Convert a persistence damage cause.
fn convert_cause(
    saved: &crate::persistence::q2::foundation::Q2AttackCause,
) -> Result<qa_content::q2::support::contracts::AttackCause, WorldError> {
    use crate::persistence::q2::foundation::Q2AttackCause as Saved;
    use qa_content::q2::support::contracts::{
        AttackCause as Content, EnvironmentHazard as ContentHazard, Q1ArmorEffect as ContentArmor,
    };
    match saved {
        Saved::Q1 {
            death_type,
            armor_effect,
        } => Ok(Content::Q1 {
            death_type: death_type.clone(),
            armor_effect: armor_effect
                .as_deref()
                .map(|effect| match effect {
                    "bypass" => Ok(ContentArmor::Bypass),
                    "half-effectiveness" => Ok(ContentArmor::HalfEffectiveness),
                    _ => Err(WorldError::BadSave(format!("unknown attack armor effect {effect:?}"))),
                })
                .transpose()?,
        }),
        Saved::Q2 {
            means_of_death,
            damage_flags,
            native,
        } => Ok(Content::Q2 {
            means_of_death: to_i32(*means_of_death, "attack means of death")?,
            damage_flags: to_i32(*damage_flags, "attack damage flags")?,
            native: native.as_ref().map(convert_native).transpose()?,
        }),
        Saved::Q3 {
            means_of_death,
            damage_flags,
        } => Ok(Content::Q3 {
            means_of_death: to_i32(*means_of_death, "attack means of death")?,
            damage_flags: to_i32(*damage_flags, "attack damage flags")?,
        }),
        Saved::Environment { hazard } => Ok(Content::Environment {
            hazard: match hazard.as_str() {
                "fall" => ContentHazard::Fall,
                "drown" => ContentHazard::Drown,
                "lava" => ContentHazard::Lava,
                "slime" => ContentHazard::Slime,
                "crush" => ContentHazard::Crush,
                "trigger" => ContentHazard::Trigger,
                _ => return Err(WorldError::BadSave(format!("unknown attack hazard {hazard:?}"))),
            },
        }),
    }
}

/// Convert a persistence attack checkpoint into the content shape.
///
/// Live actor handles stay `None`; content `restore_q2_attack` resolves
/// the saved ids through the caller's resolver. Mirrors the attack
/// conversion in `q2_character_bridge.rs`.
fn convert_attack(
    saved: &crate::persistence::q2::foundation::Q2AttackCheckpoint,
) -> Result<qa_content::q2::foundation::checkpoint::Q2AttackCheckpoint, WorldError> {
    use qa_content::q2::foundation::checkpoint::Q2AttackCheckpoint as Content;
    use qa_content::q2::support::contracts::AttackProvenance as ContentProvenance;
    Ok(Content {
        attacker: saved.attacker,
        inflictor: saved.inflictor,
        originating_projectile: saved.originating_projectile,
        attack: ContentProvenance {
            sequence: saved.sequence,
            time: saved.time,
            attacker: None,
            inflictor: None,
            originating_projectile: None,
            weapon: saved.weapon.clone(),
            weapon_provider: convert_provider(&saved.weapon_provider)?,
            damage_powerup_owner: saved
                .damage_powerup_owner
                .as_deref()
                .map(convert_provider)
                .transpose()?,
            combat_provider: convert_provider(&saved.combat_provider)?,
            inventory_provider: convert_provider(&saved.inventory_provider)?,
            movement_provider: convert_provider(&saved.movement_provider)?,
            cause: convert_cause(&saved.cause)?,
        },
    })
}

/// Convert persistence monster perception into the content shape.
fn convert_monster_perception(
    saved: &crate::persistence::q2::monsters::Q2MonsterPerception,
) -> qa_content::q2::foundation::monsters::checkpoint::Q2MonsterPerceptionCheckpoint {
    use qa_content::q2::foundation::monsters::checkpoint::{
        Q2MonsterPerceptionCheckpoint as Content, SavedNoise, SavedSighting,
    };
    use qa_content::q2::foundation::monsters::perception::TrailPoint;
    Content {
        sight_client: saved.sight_client,
        sight: saved.sight.as_ref().map(|sighting| SavedSighting {
            actor: sighting.actor,
            time: sighting.time,
        }),
        alerted: saved
            .alerted
            .iter()
            .map(|(actor, sighting)| {
                (
                    *actor,
                    SavedSighting {
                        actor: sighting.actor,
                        time: sighting.time,
                    },
                )
            })
            .collect(),
        primary: saved.primary.as_ref().map(|noise| SavedNoise {
            actor: noise.actor,
            time: noise.time,
            owner: noise.owner,
            origin: noise.origin,
        }),
        secondary: saved.secondary.as_ref().map(|noise| SavedNoise {
            actor: noise.actor,
            time: noise.time,
            owner: noise.owner,
            origin: noise.origin,
        }),
        noises: saved.noises.clone(),
        trails: saved
            .trails
            .iter()
            .map(|(actor, points)| {
                (
                    *actor,
                    points
                        .iter()
                        .map(|(origin, time, yaw)| TrailPoint {
                            origin: *origin,
                            time: *time,
                            yaw: *yaw,
                        })
                        .collect(),
                )
            })
            .collect(),
        player_origins: saved.player_origins.clone(),
        hostile: saved
            .hostile
            .iter()
            .map(|sighting| (sighting.actor, sighting.time))
            .collect(),
        last_frame: saved.last_frame,
    }
}

/// Convert a persistence Q2 monsters checkpoint into the content shape.
pub fn convert_persistence_q2_monsters(
    checkpoint: &crate::persistence::q2::monsters::Q2MonstersCheckpoint,
) -> Result<qa_content::q2::foundation::monsters::checkpoint::Q2MonstersCheckpoint, WorldError> {
    use qa_content::q2::foundation::monsters::checkpoint::{
        Q2MonsterActorCheckpoint, Q2MonsterDamageCheckpoint, Q2MonstersCheckpoint as Content,
    };
    let mut actors = Vec::with_capacity(checkpoint.actors.len());
    for actor in &checkpoint.actors {
        actors.push(Q2MonsterActorCheckpoint {
            actor: actor.actor,
            definition: actor.definition.clone(),
            state: convert_monster_state(&actor.state)?,
            pending_damage: actor
                .pending_damage
                .as_ref()
                .map(|pending| {
                    Ok::<_, WorldError>(Q2MonsterDamageCheckpoint {
                        damage: pending.damage,
                        kick: pending.kick,
                        point: pending.point,
                        attacker: pending.attacker,
                        inflictor: pending.inflictor,
                        attack: pending.attack.as_ref().map(convert_attack).transpose()?,
                    })
                })
                .transpose()?,
        });
    }
    Ok(Content {
        version: 1,
        actors,
        perception: convert_monster_perception(&checkpoint.perception),
    })
}

/// Convert a persistence weapon hand word (donor `hand` choice, q2-weapons.ts 11).
fn convert_weapon_hand(hand: &str) -> Result<qa_content::q2::foundation::weapons::types::WeaponHand, WorldError> {
    use qa_content::q2::foundation::weapons::types::WeaponHand;
    match hand {
        "right" => Ok(WeaponHand::Right),
        "left" => Ok(WeaponHand::Left),
        "center" => Ok(WeaponHand::Center),
        _ => Err(WorldError::BadSave(format!("unknown Q2 weapon hand {hand:?}"))),
    }
}

/// Convert a persistence weapon phase word (donor `phase` choice).
fn convert_weapon_phase(phase: &str) -> Result<qa_content::q2::foundation::weapons::types::Q2WeaponPhase, WorldError> {
    use qa_content::q2::foundation::weapons::types::Q2WeaponPhase;
    match phase {
        "activating" => Ok(Q2WeaponPhase::Activating),
        "ready" => Ok(Q2WeaponPhase::Ready),
        "firing" => Ok(Q2WeaponPhase::Firing),
        "dropping" => Ok(Q2WeaponPhase::Dropping),
        _ => Err(WorldError::BadSave(format!("unknown Q2 weapon phase {phase:?}"))),
    }
}

/// Convert a persistence primary-handoff word (donor `primaryHandoff` choice).
fn convert_primary_handoff(
    handoff: &str,
) -> Result<qa_content::q2::foundation::weapons::types::PrimaryHandoff, WorldError> {
    use qa_content::q2::foundation::weapons::types::PrimaryHandoff;
    match handoff {
        "active" => Ok(PrimaryHandoff::Active),
        "holstering" => Ok(PrimaryHandoff::Holstering),
        "holstered" => Ok(PrimaryHandoff::Holstered),
        _ => Err(WorldError::BadSave(format!("unknown Q2 primary handoff {handoff:?}"))),
    }
}

/// Convert a persistence weapon source-rules word (donor `sourceRules` choice).
fn convert_weapon_source_rules(
    rules: &str,
) -> Result<qa_content::q2::foundation::weapons::WeaponSourceRules, WorldError> {
    use qa_content::q2::foundation::weapons::WeaponSourceRules;
    match rules {
        "base" => Ok(WeaponSourceRules::Base),
        "ctf" => Ok(WeaponSourceRules::Ctf),
        "lmctf" => Ok(WeaponSourceRules::Lmctf),
        _ => Err(WorldError::BadSave(format!("unknown Q2 weapon source rules {rules:?}"))),
    }
}

/// Convert a persistence weapon input into the content shape.
///
/// The save carries no view height (donor `readQ2WeaponInput`); the
/// content field is currently write-only, so the bridge uses the same
/// 22.0 classic eye-height default the selected arsenal uses for
/// constructed inputs.
fn convert_weapon_input(
    saved: &crate::persistence::q2::weapons::Q2WeaponInput,
) -> Result<qa_content::q2::foundation::weapons::types::Q2WeaponInput, WorldError> {
    use qa_content::q2::foundation::weapons::types::Q2WeaponInput as Content;
    Ok(Content {
        attack: saved.attack,
        latched_attack: saved.latched_attack,
        holster: saved.holster,
        angles: saved.angles,
        ducked: saved.ducked,
        spectator: saved.spectator,
        notarget: saved.notarget,
        hand: convert_weapon_hand(&saved.hand)?,
        animate_player: saved.animate_player,
        quad_until: saved.quad_until,
        double_until: saved.double_until,
        quad_fire_until: saved.quad_fire_until,
        haste: saved.haste,
        no_stack_double: saved.no_stack_double,
        instant_switch: saved.instant_switch,
        quick_switch: saved.quick_switch,
        infinite_ammo: saved.infinite_ammo,
        players_collide: saved.players_collide,
        gravity: saved.gravity,
        weapon_thunk: saved.weapon_thunk,
        view_height: 22.0,
    })
}

/// Convert a persistence weapon state into the content shape.
#[allow(clippy::cast_possible_truncation)]
fn convert_weapon_state(
    saved: &crate::persistence::q2::weapons::Q2WeaponState,
) -> Result<qa_content::q2::foundation::weapons::types::Q2WeaponState, WorldError> {
    use crate::persistence::q2::weapons::Q2HandReservation as Saved;
    use qa_content::q2::foundation::weapons::types::{Q2HandReservation as Content, Q2WeaponState};
    Ok(Q2WeaponState {
        primary_handoff: convert_primary_handoff(&saved.primary_handoff)?,
        weapon: saved.weapon.clone(),
        last_weapon: saved.last_weapon.clone(),
        pending: saved.pending.clone(),
        phase: convert_weapon_phase(&saved.phase)?,
        frame: saved.frame as i32,
        think_time: saved.think_time,
        fire_finished: saved.fire_finished,
        fire_buffered: saved.fire_buffered,
        latched_attack: saved.latched_attack,
        machinegun_shots: saved.machinegun_shots as i32,
        empty_sound_time: saved.empty_sound_time,
        hand_reservation: match saved.hand_reservation {
            Saved::None => Content::None,
            Saved::Finite => Content::Finite,
            Saved::Infinite => Content::Infinite,
        },
        grenade_time: saved.grenade_time,
        grenade_finished: saved.grenade_finished,
        grenade_blew_up: saved.grenade_blew_up,
        kick_origin: saved.kick_origin,
        kick_angles: saved.kick_angles,
        kick_time: saved.kick_time,
        kick_until: saved.kick_until,
        kick_duration: saved.kick_duration,
        loop_sound: saved.loop_sound.clone(),
        view_model: saved.view_model.clone(),
        view_skin: saved.view_skin as i32,
        last_firing_time: saved.last_firing_time,
        source_firing: saved.source_firing,
        gun_rate: saved.gun_rate,
    })
}

/// Convert a persistence weapon noise into the content shape.
fn convert_weapon_noise(
    saved: &crate::persistence::q2::weapons::Q2NoiseCheckpoint,
) -> qa_content::q2::foundation::weapons::checkpoint::Q2NoiseCheckpoint {
    qa_content::q2::foundation::weapons::checkpoint::Q2NoiseCheckpoint {
        actor: saved.actor,
        origin: saved.origin,
        time: saved.time,
        secondary: saved.secondary,
    }
}

/// Convert a persistence Q2 weapons checkpoint into the content shape.
pub fn convert_persistence_q2_weapons(
    checkpoint: &crate::persistence::q2::weapons::Q2WeaponsCheckpoint,
) -> Result<qa_content::q2::foundation::weapons::checkpoint::Q2WeaponsCheckpoint, WorldError> {
    use qa_content::q2::foundation::weapons::checkpoint::{
        Q2BlasterCauseEntry, Q2SilencerEntry, Q2WeaponInputEntry, Q2WeaponNoiseEntry, Q2WeaponStateEntry,
        Q2WeaponsCheckpoint as Content,
    };
    let mut states = Vec::with_capacity(checkpoint.states.len());
    for (actor, state) in &checkpoint.states {
        states.push(Q2WeaponStateEntry {
            actor: *actor,
            state: convert_weapon_state(state)?,
        });
    }
    let mut inputs = Vec::with_capacity(checkpoint.inputs.len());
    for (actor, input) in &checkpoint.inputs {
        inputs.push(Q2WeaponInputEntry {
            actor: *actor,
            input: convert_weapon_input(input)?,
        });
    }
    Ok(Content {
        format_version: 2,
        silencer_charges: checkpoint
            .silencer_charges
            .iter()
            .map(|(actor, charges)| Q2SilencerEntry {
                actor: *actor,
                charges: *charges,
            })
            .collect(),
        source_rules: convert_weapon_source_rules(&checkpoint.source_rules)?,
        registered: checkpoint.registered.clone(),
        fallback_order: checkpoint.fallback_order.clone(),
        states,
        inputs,
        noises: checkpoint
            .noises
            .iter()
            .map(|(actor, primary, secondary)| Q2WeaponNoiseEntry {
                actor: *actor,
                primary: primary.as_ref().map(convert_weapon_noise),
                secondary: secondary.as_ref().map(convert_weapon_noise),
            })
            .collect(),
        sound_entity: checkpoint.sound_entity.as_ref().map(convert_weapon_noise),
        sound2_entity: checkpoint.sound2_entity.as_ref().map(convert_weapon_noise),
        blaster_causes: checkpoint
            .blaster_causes
            .iter()
            .map(|(actor, means)| {
                Ok::<_, WorldError>(Q2BlasterCauseEntry {
                    actor: *actor,
                    means_of_death: to_i32(*means, "blaster means of death")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// Convert a world inventory entry into the content twin.
fn convert_inventory_entry(saved: &qa_world::inventory::InventoryEntry) -> qa_content::contract::InventoryEntry {
    use qa_content::contract::{
        InventoryCountPolicy as ContentPolicy, InventoryEntry as ContentEntry,
        SourceCounterArithmetic as ContentArithmetic,
    };
    use qa_world::inventory::{CountArithmetic as SavedArithmetic, CountPolicy as SavedPolicy};
    ContentEntry {
        item: saved.item.clone(),
        count: saved.count,
        capacity: saved.capacity,
        count_policy: saved.count_policy.map(|policy| match policy {
            SavedPolicy::Stack => ContentPolicy::Stack,
            SavedPolicy::SourceCounter(arithmetic) => ContentPolicy::SourceCounter(match arithmetic {
                SavedArithmetic::Binary32 => ContentArithmetic::Binary32,
                SavedArithmetic::Binary64 => ContentArithmetic::Binary64,
                SavedArithmetic::Int32 => ContentArithmetic::Int32,
            }),
        }),
    }
}

/// Convert world armor state into the content twin.
fn convert_armor_state(saved: &qa_world::combat::ArmorState) -> qa_content::contract::ArmorState {
    use qa_content::contract::{
        ArmorState as Content, PoweredProtectionState as ContentPowered, RegularArmorState as ContentRegular,
    };
    use qa_world::combat::{PoweredProtection as SavedPowered, RegularArmor as SavedRegular};
    Content {
        regular: match &saved.regular {
            SavedRegular::None => ContentRegular::None,
            SavedRegular::Q1 {
                points,
                absorption,
                item,
            } => ContentRegular::Q1 {
                points: *points,
                absorption: *absorption,
                item: item.clone(),
            },
            SavedRegular::Q2 {
                points,
                normal_protection,
                energy_protection,
                item,
            } => ContentRegular::Q2 {
                points: *points,
                normal_protection: *normal_protection,
                energy_protection: *energy_protection,
                item: item.clone(),
            },
            SavedRegular::Q3 { points, protection } => ContentRegular::Q3 {
                points: *points,
                protection: *protection,
            },
            SavedRegular::Source { points, item } => ContentRegular::Source {
                points: *points,
                item: item.clone(),
            },
        },
        powered: match &saved.powered {
            SavedPowered::None => ContentPowered::None,
            SavedPowered::Screen { cells } => ContentPowered::Screen {
                cells: f64::from(*cells),
            },
            SavedPowered::Shield { cells } => ContentPowered::Shield {
                cells: f64::from(*cells),
            },
        },
    }
}

/// Convert a persistence coop-respawn carry record.
#[allow(clippy::cast_possible_truncation)]
fn convert_player_carry(
    saved: &crate::persistence::q2::players::Q2PlayerCarry,
) -> qa_content::q2::base::player::types::Q2PlayerCarry {
    use qa_content::q2::base::player::types::Q2PlayerCarry as Content;
    Content {
        health: saved.health,
        maximum_health: saved.maximum_health,
        armor: convert_armor_state(&saved.armor),
        inventory: saved.inventory.iter().map(convert_inventory_entry).collect(),
        weapon: saved.weapon.clone(),
        selected_item: saved.selected_item.clone(),
        score: saved.score as i32,
        flags: saved.flags as i64,
        power_cubes: saved.power_cubes as i32,
    }
}

/// Convert a persistence player state checkpoint.
///
/// Mirrors the player-state conversion in `q2_character_bridge.rs`;
/// the players module record carries the same state shape per entry.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_lines)]
fn convert_player_state(
    saved: &crate::persistence::q2::players::Q2PlayerStateCheckpoint,
) -> Result<qa_content::q2::base::player::checkpoint::Q2PlayerStateCheckpoint, WorldError> {
    use qa_content::q2::base::player::checkpoint::Q2PlayerStateCheckpoint as Content;
    use qa_content::q2::base::player::types::{Q2PlayerGender, Q2PlayerHand, Q2PlayerState as ContentState};
    Ok(Content {
        state: ContentState {
            slot: u64_to_i32(saved.slot, "Q2 player slot")?,
            entered_at: saved.entered_at,
            use_q2_weapons: saved.use_q2_weapons,
            use_q2_inventory: saved.use_q2_inventory,
            spawn_inventory: saved.spawn_inventory.iter().map(convert_inventory_entry).collect(),
            userinfo: saved.userinfo.clone(),
            name: saved.name.clone(),
            skin: saved.skin.clone(),
            gender: match saved.gender.as_str() {
                "male" => Q2PlayerGender::Male,
                "female" => Q2PlayerGender::Female,
                "neutral" => Q2PlayerGender::Neutral,
                other => return Err(WorldError::BadSave(format!("unknown Q2 gender {other:?}"))),
            },
            fov: saved.fov as i32,
            hand: match saved.hand.as_str() {
                "right" => Q2PlayerHand::Right,
                "left" => Q2PlayerHand::Left,
                "center" => Q2PlayerHand::Center,
                other => return Err(WorldError::BadSave(format!("unknown Q2 hand {other:?}"))),
            },
            spectator: saved.spectator,
            requested_spectator: saved.requested_spectator,
            connected: saved.connected,
            dead: saved.dead,
            gibbed: saved.gibbed,
            noclip: saved.noclip,
            god: saved.god,
            notarget: saved.notarget,
            score: saved.score as i32,
            ping: saved.ping as i32,
            respawn_time: saved.respawn_time,
            air_finished: saved.air_finished,
            next_drown_time: saved.next_drown_time,
            drown_damage: saved.drown_damage,
            old_water_level: saved.old_water_level as i32,
            breather_sound: saved.breather_sound as i32,
            pain_debounce: saved.pain_debounce,
            damage_blood: saved.damage_blood,
            damage_armor: saved.damage_armor,
            damage_power_armor: saved.damage_power_armor,
            damage_knockback: saved.damage_knockback,
            damage_from: saved.damage_from,
            damage_blend: saved.damage_blend,
            damage_alpha: saved.damage_alpha,
            bonus_alpha: saved.bonus_alpha,
            damage_pitch: saved.damage_pitch,
            damage_roll: saved.damage_roll,
            damage_time: saved.damage_time,
            power_armor_time: saved.power_armor_time,
            fall_time: saved.fall_time,
            fall_value: saved.fall_value,
            landmark_free_fall: saved.landmark_free_fall,
            landmark_noise_time: saved.landmark_noise_time,
            old_velocity: saved.old_velocity,
            old_view_angles: saved.old_view_angles,
            killer_yaw: saved.killer_yaw,
            buttons: saved.buttons as i32,
            latched_buttons: saved.latched_buttons as i32,
            weapon_thunk: saved.weapon_thunk,
            bob_time: saved.bob_time,
            bob_move: saved.bob_move,
            event: saved.event.clone(),
            animation_priority: saved.animation_priority as i32,
            animation_end: saved.animation_end as i32,
            animation_duck: saved.animation_duck,
            animation_run: saved.animation_run,
            loop_sound: saved.loop_sound.clone(),
            selected_item: saved.selected_item.clone(),
            show_scores: saved.show_scores,
            show_inventory: saved.show_inventory,
            show_help: saved.show_help,
            chase_target: None,
            coop_respawn: saved.coop_respawn.as_ref().map(convert_player_carry),
            flood_times: saved.flood_times.clone(),
            flood_lock_until: saved.flood_lock_until,
        },
        chase_target: saved.chase_target,
    })
}

/// Convert persistence player rules (mirrors `q2_character_bridge.rs`).
#[allow(clippy::cast_possible_truncation)]
fn convert_player_rules(
    saved: &crate::persistence::q2::players::Q2PlayerRules,
) -> qa_content::q2::base::player::types::Q2PlayerRules {
    use qa_content::q2::base::player::types::Q2PlayerRules as Content;
    Content {
        password: saved.password.clone(),
        spectator_password: saved.spectator_password.clone(),
        max_spectators: saved.max_spectators as i32,
        cheats: saved.cheats,
        time_limit_minutes: saved.time_limit_minutes as i32,
        frag_limit: saved.frag_limit as i32,
        map_list: saved.map_list.clone(),
        map_list_shuffle: saved.map_list_shuffle,
        next_map: saved.next_map.clone(),
        spawn_point: saved.spawn_point.clone(),
        flood_messages: saved.flood_messages as i32,
        flood_seconds: saved.flood_seconds,
        flood_wait_seconds: saved.flood_wait_seconds,
        roll_speed: saved.roll_speed,
        roll_angle: saved.roll_angle,
        run_pitch: saved.run_pitch,
        run_roll: saved.run_roll,
        bob_up: saved.bob_up,
        bob_pitch: saved.bob_pitch,
        bob_roll: saved.bob_roll,
        gun_offset: saved.gun_offset,
    }
}

/// Convert a persistence intermission checkpoint into the content shape.
fn convert_intermission(
    saved: &crate::persistence::q2::players::Q2PlayerIntermission,
) -> qa_content::q2::base::player::checkpoint::Q2PlayerIntermissionCheckpoint {
    use crate::persistence::q2::players::Q2PlayerIntermission as Saved;
    use qa_content::q2::base::player::checkpoint::{
        Q2LandmarkCarryCheckpoint as ContentLandmark, Q2PlayerIntermissionCheckpoint as Content,
    };
    match saved {
        Saved::Playing => Content::Playing,
        Saved::Intermission {
            map,
            started,
            exit,
            landmark,
        } => Content::Intermission {
            map: map.clone(),
            started: *started,
            exit: *exit,
            landmark: landmark.as_ref().map(|landmark| ContentLandmark {
                name: landmark.name.clone(),
                relative_origin: landmark.relative_origin,
                relative_velocity: landmark.relative_velocity,
                relative_view_angles: landmark.relative_view_angles,
                player: landmark.player,
            }),
        },
    }
}

/// Convert a persistence Q2 players checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
pub fn convert_persistence_q2_players(
    checkpoint: &crate::persistence::q2::players::Q2PlayersCheckpoint,
) -> Result<qa_content::q2::base::player::checkpoint::Q2PlayersCheckpoint, WorldError> {
    use qa_content::q2::base::player::checkpoint::{Q2PlayerCheckpointEntry, Q2PlayersCheckpoint as Content};
    let mut players = Vec::with_capacity(checkpoint.players.len());
    for (actor, state) in &checkpoint.players {
        players.push(Q2PlayerCheckpointEntry {
            actor: *actor,
            state: convert_player_state(state)?,
        });
    }
    Ok(Content {
        version: 1,
        corpse_index: u64_to_i32(checkpoint.corpse_index, "Q2 corpse index")?,
        death_animation: checkpoint.death_animation as i32,
        pain_animation: checkpoint.pain_animation as i32,
        rules: convert_player_rules(&checkpoint.rules),
        intermission: convert_intermission(&checkpoint.intermission),
        players,
    })
}

/// Convert a persistence platform phase word (donor `phase` choice).
fn convert_platform_phase(phase: &str) -> Result<qa_content::q2::base::entities::movers::Q2PlatformPhase, WorldError> {
    use qa_content::q2::base::entities::movers::Q2PlatformPhase;
    match phase {
        "top" => Ok(Q2PlatformPhase::Top),
        "bottom" => Ok(Q2PlatformPhase::Bottom),
        "up" => Ok(Q2PlatformPhase::Up),
        "down" => Ok(Q2PlatformPhase::Down),
        _ => Err(WorldError::BadSave(format!("unknown Q2 platform phase {phase:?}"))),
    }
}

/// Convert a persistence Q2 base-entities checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
pub fn convert_persistence_q2_base_entities(
    checkpoint: &crate::persistence::q2::base_entities::Q2BaseEntitiesCheckpoint,
) -> Result<qa_content::q2::base::entities::Q2BaseEntitiesCheckpoint, WorldError> {
    use qa_content::q2::base::entities::movers::{
        Q2BaseMoversCheckpoint, Q2PlatformEntry, Q2PlatformState as ContentPlatform, Q2SecretEntry,
        Q2SecretState as ContentSecret,
    };
    use qa_content::q2::base::entities::scenery::{Q2AnimationEntry, Q2BaseSceneryCheckpoint, Q2ClockEntry};
    use qa_content::q2::base::entities::triggers::Q2WindTimeEntry;
    use qa_content::q2::base::entities::turrets::{
        Q2BreachEntry, Q2BreachState as ContentBreach, Q2DriverEntry, Q2TurretsCheckpoint,
    };
    use qa_content::q2::base::entities::Q2BaseEntitiesCheckpoint as Content;
    let mut platforms = Vec::with_capacity(checkpoint.platforms.len());
    for (actor, state) in &checkpoint.platforms {
        platforms.push(Q2PlatformEntry {
            actor: *actor,
            state: ContentPlatform {
                top: state.top,
                bottom: state.bottom,
                phase: convert_platform_phase(&state.phase)?,
            },
        });
    }
    let mut animations = Vec::with_capacity(checkpoint.animations.len());
    for (actor, first, end) in &checkpoint.animations {
        animations.push(Q2AnimationEntry {
            actor: *actor,
            first: u64_to_i32(*first, "scenery animation first frame")?,
            end: u64_to_i32(*end, "scenery animation end frame")?,
        });
    }
    Ok(Content {
        version: 1,
        movers: Q2BaseMoversCheckpoint {
            platforms,
            secrets: checkpoint
                .secrets
                .iter()
                .map(|(actor, state)| Q2SecretEntry {
                    actor: *actor,
                    state: ContentSecret {
                        first: state.first,
                        second: state.second,
                        home: state.home,
                        shootable: state.shootable,
                        blocked_time: state.blocked_time,
                        message_time: state.message_time,
                    },
                })
                .collect(),
            linear: convert_linear_motion(&checkpoint.linear),
        },
        scenery: Q2BaseSceneryCheckpoint {
            animations,
            clocks: checkpoint
                .clocks
                .iter()
                .map(|(actor, value)| Q2ClockEntry {
                    actor: *actor,
                    value: *value as i32,
                })
                .collect(),
        },
        turrets: Q2TurretsCheckpoint {
            breaches: checkpoint
                .breaches
                .iter()
                .map(|(actor, state)| Q2BreachEntry {
                    actor: *actor,
                    state: ContentBreach {
                        goal: state.goal,
                        muzzle: state.muzzle,
                        pitch_max: state.pitch_max,
                        pitch_min: state.pitch_min,
                        yaw_min: state.yaw_min,
                        yaw_max: state.yaw_max,
                    },
                })
                .collect(),
            drivers: checkpoint
                .drivers
                .iter()
                .map(|driver| Q2DriverEntry {
                    actor: driver.actor,
                    breach: driver.breach,
                    radius: driver.radius,
                    yaw_offset: driver.yaw_offset,
                    height: driver.height,
                    monster_die: driver.monster_die.clone(),
                })
                .collect(),
        },
        wind_times: checkpoint
            .wind_times
            .iter()
            .map(|(actor, until)| Q2WindTimeEntry {
                actor: *actor,
                until: *until,
            })
            .collect(),
    })
}

/// Convert a persistence mission-pack items checkpoint into the content shape.
pub fn convert_persistence_q2_missionpack_items(
    checkpoint: &crate::persistence::q2::missionpacks::Q2MissionPackItemsCheckpoint,
) -> qa_content::q2::missionpacks::items::Q2MissionPackItemsCheckpoint {
    use qa_content::q2::missionpacks::items::{Q2MissionPackItemsCheckpoint as Content, Q2MissionPackPowerups};
    Content {
        powers: checkpoint
            .powers
            .iter()
            .map(|power| {
                (
                    power.actor,
                    Q2MissionPackPowerups {
                        quad_fire_until: power.quad_fire_until,
                        double_until: power.double_until,
                        ir_until: power.ir_until,
                    },
                )
            })
            .collect(),
    }
}

/// Convert a persistence rogue hints checkpoint into the content shape.
fn convert_rogue_hints(
    saved: &crate::persistence::q2::missionpacks::Q2RogueHintsCheckpoint,
) -> Result<qa_content::q2::missionpacks::monsters::hints::RogueHintsCheckpoint, WorldError> {
    use qa_content::q2::missionpacks::monsters::hints::{
        RogueHintsCheckpoint as Content, SavedHintNode, SavedHintPursuer,
    };
    let mut nodes = Vec::with_capacity(saved.nodes.len());
    for node in &saved.nodes {
        nodes.push(SavedHintNode {
            actor: node.actor,
            chain: to_i32(node.chain, "hint chain index")?,
            next: node.next,
        });
    }
    Ok(Content {
        version: 1,
        present: saved.present,
        starts: saved.starts.clone(),
        nodes,
        monsters: saved
            .monsters
            .iter()
            .map(|monster| SavedHintPursuer {
                actor: monster.actor,
                goal: monster.goal,
                last_time: monster.last_time,
            })
            .collect(),
    })
}

/// Convert a persistence mission-pack monsters checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
pub fn convert_persistence_q2_missionpack_monsters(
    checkpoint: &crate::persistence::q2::missionpacks::Q2MissionPackMonstersCheckpoint,
) -> Result<qa_content::q2::missionpacks::monsters::state::MissionPackMonstersCheckpoint, WorldError> {
    use qa_content::q2::missionpacks::monsters::state::{
        MissionPackMonstersCheckpoint as Content, RogueFlyerNext, SavedRogueMonsterState,
    };
    let flyer_next_move = match checkpoint.flyer_next_move.as_str() {
        "none" => RogueFlyerNext::None,
        "run" => RogueFlyerNext::Run,
        other => return Err(WorldError::BadSave(format!("unknown rogue flyer move {other:?}"))),
    };
    let mut actors = Vec::with_capacity(checkpoint.actors.len());
    for (actor, state) in &checkpoint.actors {
        let mut reinforcements = Vec::with_capacity(state.chosen_reinforcements.len());
        for index in &state.chosen_reinforcements {
            reinforcements.push(u64_to_i32(*index, "reinforcement index")?);
        }
        actors.push(SavedRogueMonsterState {
            actor: *actor,
            blocked: state.blocked,
            turret_orientation: state.turret_orientation,
            healer: state.healer,
            bad_medic1: state.bad_medic1,
            bad_medic2: state.bad_medic2,
            medic_tries: state.medic_tries as i32,
            chosen_reinforcements: reinforcements,
            react_to_damage_time: state.react_to_damage_time,
            summon_strength: state.summon_strength as i32,
            last_player_enemy: state.last_player_enemy,
            bad_area: state.bad_area,
            good_guy: state.good_guy,
            widow_quad_until: state.widow_quad_until,
            widow_double_until: state.widow_double_until,
            widow_invulnerable_until: state.widow_invulnerable_until,
        });
    }
    Ok(Content {
        version: 1,
        flyer_next_move,
        widow_shots_fired: checkpoint.widow_shots_fired as i32,
        widow_damage_multiplier: to_u8(checkpoint.widow_damage_multiplier, "widow damage multiplier")?,
        hints: checkpoint.hints.as_ref().map(convert_rogue_hints).transpose()?,
        actors,
    })
}

/// Convert a persistence rogue entities checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
pub fn convert_persistence_q2_rogue_entities(
    checkpoint: &crate::persistence::q2::missionpacks::Q2RogueEntitiesCheckpoint,
) -> qa_content::q2::missionpacks::entities::rogue::Q2RogueEntitiesCheckpoint {
    qa_content::q2::missionpacks::entities::rogue::Q2RogueEntitiesCheckpoint {
        steam_id: checkpoint.steam_id as i32,
    }
}

/// Convert a persistence tag checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
pub fn convert_persistence_q2_tag(
    checkpoint: &crate::persistence::q2::missionpacks::Q2TagCheckpoint,
) -> qa_content::q2::missionpacks::modes::tag::Q2TagCheckpoint {
    qa_content::q2::missionpacks::modes::tag::Q2TagCheckpoint {
        token: checkpoint.token,
        owner: checkpoint.owner,
        count: checkpoint.count as i32,
    }
}

/// Convert a persistence deathball checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
pub fn convert_persistence_q2_deathball(
    checkpoint: &crate::persistence::q2::missionpacks::Q2DeathBallCheckpoint,
) -> qa_content::q2::missionpacks::modes::deathball::Q2DeathBallCheckpoint {
    qa_content::q2::missionpacks::modes::deathball::Q2DeathBallCheckpoint {
        ball: checkpoint.ball,
        starts: checkpoint.starts as i32,
        team1_score: checkpoint.team1_score,
        team2_score: checkpoint.team2_score,
    }
}

/// Read rerelease fog state (donor `readQ2FogState`, q2-rerelease-state.ts 9-14).
fn read_fog_state(reader: SaveReader) -> Result<qa_content::q2::rerelease::types::Q2FogState, WorldError> {
    use qa_content::q2::rerelease::types::{Q2Fog, Q2FogState as Content, Q2HeightFog};
    let fog = reader.field("fog");
    let height = reader.field("heightFog");
    Ok(Content {
        fog: Q2Fog {
            density: fog.field("density").number()?,
            color: read_vector(fog.field("color"))?,
            sky_factor: fog.field("skyFactor").number()?,
        },
        height_fog: Q2HeightFog {
            start_color: read_vector(height.field("startColor"))?,
            start_distance: height.field("startDistance").number()?,
            end_color: read_vector(height.field("endColor"))?,
            end_distance: height.field("endDistance").number()?,
            falloff: height.field("falloff").number()?,
            density: height.field("density").number()?,
        },
    })
}

/// Read a rerelease player state (donor `player`, q2-rerelease-state.ts 15-20).
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_lines)]
fn read_rerelease_player(
    reader: SaveReader,
) -> Result<qa_content::q2::rerelease::types::Q2RereleasePlayerState, WorldError> {
    use qa_content::q2::rerelease::types::{Q2CoopRespawnState, Q2PendingLandmark, Q2RereleasePlayerState as Content};
    let number = |key: &str| reader.field(key).number();
    let flag = |key: &str| reader.field(key).boolean();
    Ok(Content {
        spawned: flag("spawned")?,
        game_help1_changed: number("gameHelp1Changed")? as i32,
        game_help2_changed: number("gameHelp2Changed")? as i32,
        help_changed: number("helpChanged")? as i32,
        help_time: number("helpTime")?,
        invisibility_until: reader.field("invisibilityUntil").finite()?,
        invisibility_fade_until: reader.field("invisibilityFadeUntil").finite()?,
        slime_debounce: number("slimeDebounce")?,
        animation_time: number("animationTime")?,
        flash_time: number("flashTime")?,
        flashes: number("flashes")? as i32,
        last_damage_until: number("lastDamageUntil")?,
        last_firing_until: number("lastFiringUntil")?,
        lives: number("lives")? as i32,
        coop_respawn_state: match reader
            .field("coopRespawnState")
            .choice_str(&["none", "in-combat", "bad-area", "blocked", "waiting", "no-lives"])?
            .as_str()
        {
            "in-combat" => Q2CoopRespawnState::InCombat,
            "bad-area" => Q2CoopRespawnState::BadArea,
            "blocked" => Q2CoopRespawnState::Blocked,
            "waiting" => Q2CoopRespawnState::Waiting,
            "no-lives" => Q2CoopRespawnState::NoLives,
            _ => Q2CoopRespawnState::None,
        },
        flashlight: flag("flashlight")?,
        fog: read_fog_state(reader.field("fog"))?,
        wanted_fog: read_fog_state(reader.field("wantedFog"))?,
        fog_transition: number("fogTransition")?,
        bob_skip: flag("bobSkip")?,
        auto_switch: to_u8(reader.field("autoSwitch").choice_i64(&[0, 1, 2, 3])?, "auto switch")?,
        auto_shield: number("autoShield")? as i32,
        dogtag: reader.field("dogtag").string()?,
        impact_delta: number("impactDelta")?,
        on_ladder: flag("onLadder")?,
        grapple_released_until: number("grappleReleasedUntil")?,
        grapple_attached: flag("grappleAttached")?,
        slow_view_angles: read_vector(reader.field("slowViewAngles"))?,
        quake_time: number("quakeTime")?,
        wind_sound_time: number("windSoundTime")?,
        awaiting_respawn: flag("awaitingRespawn")?,
        respawn_timeout: number("respawnTimeout")?,
        pending_landmark: reader.field("pendingLandmark").nullable(|value| {
            Ok::<_, WorldError>(Q2PendingLandmark {
                name: value.field("name").string()?,
                relative_origin: read_vector(value.field("relativeOrigin"))?,
                relative_velocity: read_vector(value.field("relativeVelocity"))?,
                relative_view_angles: read_vector(value.field("relativeViewAngles"))?,
            })
        })?,
        help_location: read_vector(reader.field("helpLocation"))?,
        help_image: reader.field("helpImage").string()?,
        help_points: reader.field("helpPoints").list(read_vector)?,
        help_index: number("helpIndex")? as i32,
        help_draw_time: number("helpDrawTime")?,
        help_marker_until: if reader.field("helpMarkerUntil").is_missing() {
            0.0
        } else {
            reader.field("helpMarkerUntil").finite()?
        },
        seat: to_i32(reader.field("seat").integer(0)?, "rerelease seat")?,
        social_id: reader.field("socialId").string()?,
    })
}

/// Read a rerelease players checkpoint (donor
/// `readQ2RereleasePlayersCheckpoint`, q2-rerelease-state.ts 21-26).
#[allow(clippy::cast_possible_truncation)]
pub fn read_q2_rerelease_players_checkpoint(
    reader: SaveReader,
) -> Result<qa_content::q2::rerelease::checkpoint::Q2RereleasePlayersCheckpoint, WorldError> {
    use qa_content::q2::rerelease::checkpoint::{
        Q2RereleaseIntermissionCamera, Q2RereleasePlayerCheckpointEntry, Q2RereleasePlayersCheckpoint as Content,
        Q2RereleaseSquadSpawn,
    };
    use qa_content::q2::rerelease::types::Q2RereleaseOptions;
    reader.field("version").literal_i64(1)?;
    let options = reader.field("options");
    let flag = |key: &str| options.field(key).boolean();
    Ok(Content {
        version: 1,
        options: Q2RereleaseOptions {
            coop_squad_respawn: flag("coopSquadRespawn")?,
            coop_instanced_items: flag("coopInstancedItems")?,
            coop_lives: flag("coopLives")?,
            coop_num_lives: options.field("coopNumLives").number()? as i32,
            deathmatch_force_respawn: flag("deathmatchForceRespawn")?,
            deathmatch_no_fall_damage: flag("deathmatchNoFallDamage")?,
            deathmatch_allow_exit: flag("deathmatchAllowExit")?,
            deathmatch_spawn_farthest: flag("deathmatchSpawnFarthest")?,
            deathmatch_force_respawn_time: options.field("deathmatchForceRespawnTime").number()?,
            coop_player_collision: flag("coopPlayerCollision")?,
            auto_save_minimum_time: options.field("autoSaveMinimumTime").number()?,
        },
        coop_restart_time: reader.field("coopRestartTime").number()?,
        deadly_kill_box: reader.field("deadlyKillBox").boolean()?,
        intermission_flags: reader.field("intermissionFlags").number()? as i32,
        intermission_fade_until: reader.field("intermissionFadeUntil").nullable(|value| value.number())?,
        intermission_camera: reader.field("intermissionCamera").nullable(|value| {
            Ok::<_, WorldError>(Q2RereleaseIntermissionCamera {
                origin: read_vector(value.field("origin"))?,
                angles: read_vector(value.field("angles"))?,
            })
        })?,
        intermission_camera_set: reader.field("intermissionCameraSet").boolean()?,
        players: reader.field("players").list(|value| {
            Ok::<_, WorldError>(Q2RereleasePlayerCheckpointEntry {
                actor: read_saved_actor(value.field("actor"))?,
                state: read_rerelease_player(value.field("state"))?,
            })
        })?,
        squad_spawns: reader.field("squadSpawns").list(|value| {
            Ok::<_, WorldError>(Q2RereleaseSquadSpawn {
                actor: read_saved_actor(value.field("actor"))?,
                origin: read_vector(value.field("origin"))?,
                angles: read_vector(value.field("angles"))?,
            })
        })?,
    })
}

/// Read a rerelease Q64 checkpoint (donor
/// `readQ2RereleaseQ64Checkpoint`, q2-rerelease-state.ts 33-40).
fn read_rerelease_q64(
    reader: SaveReader,
) -> Result<qa_content::q2::rerelease::checkpoint::Q2RereleaseQ64Checkpoint, WorldError> {
    use qa_content::q2::rerelease::checkpoint::{
        Q2RereleaseQ64CameraCheckpoint, Q2RereleaseQ64CameraState, Q2RereleaseQ64Checkpoint as Content,
        Q2RereleaseQ64DummyCheckpoint, Q2RereleaseQ64DummyState, Q2RereleaseQ64EyeCheckpoint, Q2RereleaseQ64EyeState,
    };
    Ok(Content {
        eyes: reader.field("eyes").list(|value| {
            let state = value.field("state");
            Ok::<_, WorldError>(Q2RereleaseQ64EyeCheckpoint {
                actor: read_saved_actor(value.field("actor"))?,
                state: Q2RereleaseQ64EyeState {
                    neutral_angles: read_vector(state.field("neutralAngles"))?,
                    eye_position: read_vector(state.field("eyePosition"))?,
                    vision_cone: state.field("visionCone").number()?,
                },
            })
        })?,
        cameras: reader.field("cameras").list(|value| {
            let state = value.field("state");
            Ok::<_, WorldError>(Q2RereleaseQ64CameraCheckpoint {
                actor: read_saved_actor(value.field("actor"))?,
                state: Q2RereleaseQ64CameraState {
                    remaining: state.field("remaining").number()?,
                    distance: state.field("distance").number()?,
                    speed: state.field("speed").number()?,
                    angles: read_vector(state.field("angles"))?,
                },
            })
        })?,
        dummies: reader.field("dummies").list(|value| {
            let state = value.field("state");
            Ok::<_, WorldError>(Q2RereleaseQ64DummyCheckpoint {
                actor: read_saved_actor(value.field("actor"))?,
                state: Q2RereleaseQ64DummyState {
                    fade_remaining: state.field("fadeRemaining").number()?,
                    fade_duration: state.field("fadeDuration").number()?,
                    fading: state.field("fading").boolean()?,
                },
            })
        })?,
    })
}

/// Read a rerelease module checkpoint (donor
/// `readQ2RereleaseModuleCheckpoint`, q2-rerelease-state.ts 27-32).
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_lines)]
pub fn read_q2_rerelease_module_checkpoint(
    reader: SaveReader,
) -> Result<qa_content::q2::rerelease::checkpoint::Q2RereleaseModuleCheckpoint, WorldError> {
    use qa_content::q2::rerelease::campaign::{Q2RereleaseLevelEntry, Q2RereleaseMission};
    use qa_content::q2::rerelease::checkpoint::{
        Q2RereleaseCampaignCheckpoint, Q2RereleaseHealthBarCheckpoint, Q2RereleaseHealthTarget,
        Q2RereleaseLightCheckpoint, Q2RereleaseModuleCheckpoint as Content, Q2RereleasePickupRecord,
        Q2RereleasePoiCheckpoint, Q2RereleaseSkyCheckpoint, Q2RereleaseTriggerSoundTime,
    };
    use qa_content::q2::rerelease::goals::Q2RereleaseGoalsCheckpoint;
    reader.field("version").literal_i64(1)?;
    let sky = reader.field("sky");
    let campaign = reader.field("campaign");
    let mission = campaign.field("mission");
    let goals = reader.field("goals");
    Ok(Content {
        version: 1,
        world_fog: read_fog_state(reader.field("worldFog"))?,
        story: reader.field("story").string()?,
        sky: Q2RereleaseSkyCheckpoint {
            name: sky.field("name").string()?,
            rotation: sky.field("rotation").number()?,
            auto_rotate: sky.field("autoRotate").boolean()?,
            axis: read_vector(sky.field("axis"))?,
        },
        poi: reader.field("poi").nullable(|value| {
            Ok::<_, WorldError>(Q2RereleasePoiCheckpoint {
                actor: read_saved_actor(value.field("actor"))?,
                origin: read_vector(value.field("origin"))?,
                image: value.field("image").string()?,
                dynamic: value.field("dynamic").nullable(read_saved_actor)?,
            })
        })?,
        poi_stage: reader.field("poiStage").number()? as i32,
        last_auto_save: reader.field("lastAutoSave").number()?,
        goals: Q2RereleaseGoalsCheckpoint {
            goals: goals.field("goals").nullable(|value| value.string())?,
            goal_number: goals.field("goalNumber").number()? as i32,
        },
        campaign: Q2RereleaseCampaignCheckpoint {
            mission: Q2RereleaseMission {
                primary: mission.field("primary").string()?,
                secondary: mission.field("secondary").string()?,
                primary_changes: mission.field("primaryChanges").number()? as i32,
                secondary_changes: mission.field("secondaryChanges").number()? as i32,
            },
            cross_unit_flags: campaign.field("crossUnitFlags").number()? as i32,
            visited_maps: campaign.field("visitedMaps").list(|value| value.string())?,
            levels: campaign.field("levels").list(|value| {
                Ok::<_, WorldError>(Q2RereleaseLevelEntry {
                    map: value.field("map").string()?,
                    name: value.field("name").string()?,
                    visit_order: value.field("visitOrder").number()? as i32,
                    total_secrets: value.field("totalSecrets").number()? as i32,
                    found_secrets: value.field("foundSecrets").number()? as i32,
                    total_monsters: value.field("totalMonsters").number()? as i32,
                    killed_monsters: value.field("killedMonsters").number()? as i32,
                    time: value.field("time").number()?,
                })
            })?,
        },
        q64: read_rerelease_q64(reader.field("q64"))?,
        lights: reader.field("lights").list(|value| {
            Ok::<_, WorldError>(Q2RereleaseLightCheckpoint {
                actor: read_saved_actor(value.field("actor"))?,
                active: value.field("active").boolean()?,
            })
        })?,
        health_bars: reader.field("healthBars").list(|value| {
            value.nullable(|bar| {
                Ok::<_, WorldError>(Q2RereleaseHealthBarCheckpoint {
                    controller: read_saved_actor(bar.field("controller"))?,
                    target: read_saved_actor(bar.field("target"))?,
                    dead_until: bar.field("deadUntil").nullable(|time| time.number())?,
                })
            })
        })?,
        health_targets: reader.field("healthTargets").list(|value| {
            Ok::<_, WorldError>(Q2RereleaseHealthTarget {
                controller: read_saved_actor(value.field("controller"))?,
                target: read_saved_actor(value.field("target"))?,
            })
        })?,
        picked_up_by: reader.field("pickedUpBy").list(|value| {
            Ok::<_, WorldError>(Q2RereleasePickupRecord {
                actor: read_saved_actor(value.field("actor"))?,
                slots: value
                    .field("slots")
                    .list(|slot| to_i32(slot.integer(0)?, "pickup slot"))?,
            })
        })?,
        trigger_sound_times: reader.field("triggerSoundTimes").list(|value| {
            Ok::<_, WorldError>(Q2RereleaseTriggerSoundTime {
                actor: read_saved_actor(value.field("actor"))?,
                time: value.field("time").number()?,
            })
        })?,
    })
}

/// Read a CTF player state (donor `readPlayer`, ctf/checkpoint.ts 49-57).
fn read_ctf_player(
    reader: SaveReader,
) -> Result<
    (
        qa_content::q2::multiplayer::ctf::types::Q2CtfPlayerState,
        qa_content::q2::equipment::grapple_services::CtfGrappleCheckpoint,
    ),
    WorldError,
> {
    use qa_content::q2::equipment::grapple_services::{CtfGrappleCheckpoint, CtfGrapplePhase};
    use qa_content::q2::multiplayer::ctf::types::Q2CtfPlayerState as Content;
    let grapple_no_knockback = if reader.field("grappleNoKnockback").is_missing() {
        None
    } else {
        reader.field("grappleNoKnockback").nullable(|value| value.boolean())?
    };
    Ok((
        Content {
            team: to_u8(reader.field("team").choice_i64(&[0, 1, 2])?, "CTF team")?,
            spawn_state: to_i32(reader.field("spawnState").integer(0)?, "CTF spawn state")?,
            last_hurt_carrier: reader.field("lastHurtCarrier").nullable(|value| value.finite())?,
            last_returned_flag: reader.field("lastReturnedFlag").nullable(|value| value.finite())?,
            last_fragged_carrier: reader.field("lastFraggedCarrier").nullable(|value| value.finite())?,
            flag_since: reader.field("flagSince").finite()?,
            voted: reader.field("voted").boolean()?,
            ready: reader.field("ready").boolean()?,
            admin: reader.field("admin").boolean()?,
            id_view: reader.field("idView").boolean()?,
            ghost_code: reader
                .field("ghostCode")
                .nullable(|value| to_i32(value.integer(10000)?, "CTF ghost code"))?,
            regen_time: reader.field("regenTime").finite()?,
            tech_sound_time: reader.field("techSoundTime").finite()?,
            last_tech_message: reader.field("lastTechMessage").finite()?,
            match_respawn_at: reader.field("matchRespawnAt").nullable(|value| value.finite())?,
        },
        CtfGrappleCheckpoint {
            grapple: reader.field("grapple").nullable(read_saved_actor)?,
            grapple_state: match reader
                .field("grappleState")
                .choice_str(&["fly", "pull", "hang"])?
                .as_str()
            {
                "pull" => CtfGrapplePhase::Pull,
                "hang" => CtfGrapplePhase::Hang,
                _ => CtfGrapplePhase::Fly,
            },
            grapple_release_time: reader.field("grappleReleaseTime").finite()?,
            grapple_no_knockback,
        },
    ))
}

/// Read a CTF checkpoint (donor `decodeQ2CtfCheckpoint`, ctf/checkpoint.ts 58-72).
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::too_many_lines)]
pub fn read_q2_ctf_checkpoint(
    reader: SaveReader,
) -> Result<qa_content::q2::multiplayer::ctf::checkpoint::Q2CtfCheckpoint, WorldError> {
    use qa_content::q2::multiplayer::ctf::checkpoint::{
        Q2CtfCheckpoint as Content, Q2CtfElectionCheckpoint, Q2CtfGhostCheckpoint, Q2CtfMatchCheckpoint,
        Q2CtfPlayerCheckpoint, Q2CtfRulesCheckpoint,
    };
    use qa_content::q2::multiplayer::ctf::types::{Q2CtfElectionKind, Q2CtfForceJoin, Q2CtfMatchPhase};
    reader.field("version").literal_i64(1)?;
    let rules = reader.field("rules");
    let saved = reader.field("match");
    Ok(Content {
        version: 1,
        rules: Q2CtfRulesCheckpoint {
            force_join: match rules.field("forceJoin").choice_str(&["", "red", "blue"])?.as_str() {
                "red" => Q2CtfForceJoin::Red,
                "blue" => Q2CtfForceJoin::Blue,
                _ => Q2CtfForceJoin::Any,
            },
            competition: to_i32(rules.field("competition").integer(0)?, "CTF competition")?,
            match_lock: rules.field("matchLock").boolean()?,
            election_percentage: rules.field("electionPercentage").finite()?,
            match_minutes: rules.field("matchMinutes").finite()?,
            setup_minutes: rules.field("setupMinutes").finite()?,
            start_seconds: rules.field("startSeconds").finite()?,
            capture_limit: to_i32(rules.field("captureLimit").integer(0)?, "CTF capture limit")?,
            instant_weapons: rules.field("instantWeapons").boolean()?,
        },
        match_state: Q2CtfMatchCheckpoint {
            team1: to_i32(saved.field("team1").integer(0)?, "CTF team1")?,
            team2: to_i32(saved.field("team2").integer(0)?, "CTF team2")?,
            total1: to_i32(saved.field("total1").integer(i64::MIN)?, "CTF total1")?,
            total2: to_i32(saved.field("total2").integer(i64::MIN)?, "CTF total2")?,
            last_flag_capture: saved.field("lastFlagCapture").nullable(|value| value.finite())?,
            last_capture_team: saved
                .field("lastCaptureTeam")
                .nullable(|value| to_u8(value.choice_i64(&[1, 2])?, "CTF capture team"))?,
            phase: match saved
                .field("phase")
                .choice_str(&["none", "setup", "pregame", "game", "post"])?
                .as_str()
            {
                "setup" => Q2CtfMatchPhase::Setup,
                "pregame" => Q2CtfMatchPhase::Pregame,
                "game" => Q2CtfMatchPhase::Game,
                "post" => Q2CtfMatchPhase::Post,
                _ => Q2CtfMatchPhase::None,
            },
            match_time: saved.field("matchTime").finite()?,
            last_time: saved.field("lastTime").integer(i64::MIN)? as f64,
            election: saved.field("election").nullable(|value| {
                Ok::<_, WorldError>(Q2CtfElectionCheckpoint {
                    kind: match value.field("kind").choice_str(&["match", "admin", "map"])?.as_str() {
                        "admin" => Q2CtfElectionKind::Admin,
                        "map" => Q2CtfElectionKind::Map,
                        _ => Q2CtfElectionKind::Match,
                    },
                    target: read_saved_actor(value.field("target"))?,
                    map: value.field("map").string()?,
                    message: value.field("message").string()?,
                    votes: to_i32(value.field("votes").integer(0)?, "CTF votes")?,
                    needed: to_i32(value.field("needed").integer(1)?, "CTF needed")?,
                    expires: value.field("expires").finite()?,
                })
            })?,
            ghosts: saved.field("ghosts").list(|value| {
                Ok::<_, WorldError>(Q2CtfGhostCheckpoint {
                    code: to_i32(value.field("code").integer(10000)?, "CTF ghost code")?,
                    team: to_u8(value.field("team").choice_i64(&[1, 2])?, "CTF ghost team")?,
                    name: value.field("name").string()?,
                    actor: value.field("actor").nullable(read_saved_actor)?,
                    score: to_i32(value.field("score").integer(i64::MIN)?, "CTF ghost score")?,
                    deaths: to_i32(value.field("deaths").integer(0)?, "CTF ghost deaths")?,
                    kills: to_i32(value.field("kills").integer(0)?, "CTF ghost kills")?,
                    captures: to_i32(value.field("captures").integer(0)?, "CTF ghost captures")?,
                    base_defense: to_i32(value.field("baseDefense").integer(0)?, "CTF base defense")?,
                    carrier_defense: to_i32(value.field("carrierDefense").integer(0)?, "CTF carrier defense")?,
                })
            })?,
        },
        players: reader.field("players").list(|value| {
            let (state, grapple) = read_ctf_player(value.field("state"))?;
            Ok::<_, WorldError>(Q2CtfPlayerCheckpoint {
                actor: read_saved_actor(value.field("actor"))?,
                state,
                grapple,
            })
        })?,
    })
}

/// Read an LMCTF checkpoint (donor `Q2Lmctf::restore`, lmctf/runtime.ts
/// 64-90, plus the match/vote/flags/runes restores).
#[allow(clippy::too_many_lines)]
pub fn read_q2_lmctf_checkpoint(
    reader: SaveReader,
) -> Result<qa_content::q2::multiplayer::lmctf::runtime::LmctfCheckpoint, WorldError> {
    use qa_content::q2::equipment::grapple_services::LmctfGrappleCheckpoint;
    use qa_content::q2::multiplayer::lmctf::flags::{LmctfFlagSlot, LmctfFlagsCheckpoint};
    use qa_content::q2::multiplayer::lmctf::match_::{LmctfMatchCheckpoint, LmctfMatchPhase};
    use qa_content::q2::multiplayer::lmctf::runes::LmctfRunesCheckpoint;
    use qa_content::q2::multiplayer::lmctf::runtime::{
        LmctfCheckpoint as Content, LmctfPlayerCheckpoint, LmctfRulesCheckpoint,
    };
    use qa_content::q2::multiplayer::lmctf::types::LmctfMapChange;
    use qa_content::q2::multiplayer::lmctf::vote::LmctfVoteCheckpoint;
    let rules = reader.field("rules");
    let saved = reader.field("match");
    let pending = saved.field("pendingMap");
    let flags = reader.field("flags");
    Ok(Content {
        rules: LmctfRulesCheckpoint {
            time_limit_minutes: rules.field("timeLimitMinutes").finite()?,
            frag_limit: to_i32(rules.field("fragLimit").integer(i64::MIN)?, "LMCTF frag limit")?,
            map_list: rules.field("mapList").list(|value| value.string())?,
            ctf_flags: to_i32(rules.field("ctfFlags").integer(0)?, "LMCTF CTF flags")?,
            ref_flags: to_i32(rules.field("refFlags").integer(0)?, "LMCTF referee flags")?,
            runes: to_i32(rules.field("runes").integer(0)?, "LMCTF runes")?,
            skin_set: to_i32(rules.field("skinSet").integer(i64::MIN)?, "LMCTF skin set")?,
            flag_init: rules.field("flagInit").boolean()?,
            disabled_weapons: to_i32(rules.field("disabledWeapons").integer(0)?, "LMCTF disabled weapons")?,
            fast_switch: rules.field("fastSwitch").boolean()?,
            auto_lock: rules.field("autoLock").boolean()?,
            countdown_seconds: rules.field("countdownSeconds").finite()?,
            quad_seconds: rules.field("quadSeconds").finite()?,
        },
        match_state: LmctfMatchCheckpoint {
            pending_map: if pending.is_missing() {
                None
            } else {
                pending.nullable(|value| {
                    Ok::<_, WorldError>(LmctfMapChange {
                        map: value.field("map").string()?,
                        countdown: value.field("countdown").boolean()?,
                    })
                })?
            },
            phase: match saved
                .field("phase")
                .choice_str(&["none", "countdown", "inplay", "over"])?
                .as_str()
            {
                "countdown" => LmctfMatchPhase::Countdown,
                "inplay" => LmctfMatchPhase::Inplay,
                "over" => LmctfMatchPhase::Over,
                _ => LmctfMatchPhase::None,
            },
            remaining: to_i32(saved.field("remaining").integer(i64::MIN)?, "LMCTF remaining")?,
            next_think: saved.field("nextThink").finite()?,
            paused: saved.field("paused").boolean()?,
            teams_locked: saved.field("teamsLocked").boolean()?,
        },
        vote: LmctfVoteCheckpoint {
            started_at: reader
                .field("vote")
                .field("startedAt")
                .nullable(|value| value.finite())?,
        },
        plasma_quad: reader.field("plasmaQuad").boolean()?,
        flags: LmctfFlagsCheckpoint {
            flags: flags.field("flags").list(|flag| {
                Ok::<_, WorldError>(LmctfFlagSlot {
                    team: to_u8(flag.field("team").choice_i64(&[1, 2])?, "LMCTF flag team")?,
                    actor: read_saved_actor(flag.field("actor"))?,
                })
            })?,
            last_taken_sound: flags.field("lastTakenSound").finite()?,
        },
        runes: LmctfRunesCheckpoint {
            forward: reader.field("runes").field("forward").boolean()?,
        },
        players: reader.field("players").list(|entry| {
            let saved = entry.field("state");
            Ok::<_, WorldError>(LmctfPlayerCheckpoint {
                actor: read_saved_actor(entry.field("actor"))?,
                plasma_mode: saved.field("plasmaMode").boolean()?,
                team: to_u8(saved.field("team").choice_i64(&[0, 1, 2])?, "LMCTF team")?,
                observer_team: to_u8(
                    saved.field("observerTeam").choice_i64(&[0, 1, 2])?,
                    "LMCTF observer team",
                )?,
                rune: saved.field("rune").nullable(read_saved_actor)?,
                regen_frame: to_i32(saved.field("regenFrame").integer(i64::MIN)?, "LMCTF regen frame")?,
                kill_carrier_time: saved.field("killCarrierTime").finite()?,
                hit_carrier_time: saved.field("hitCarrierTime").finite()?,
                return_flag_time: saved.field("returnFlagTime").finite()?,
                defend_flag_time: saved.field("defendFlagTime").finite()?,
                extra_flags: to_i32(saved.field("extraFlags").integer(i64::MIN)?, "LMCTF extra flags")?,
                spawn_state: to_i32(saved.field("spawnState").integer(0)?, "LMCTF spawn state")?,
                statistics: saved.field("statistics").list(|value| {
                    Ok::<_, WorldError>((value.field("key").string()?, value.field("count").finite()?))
                })?,
                grapple: LmctfGrappleCheckpoint {
                    hook: saved.field("hook").nullable(read_saved_actor)?,
                    hook_state: to_i32(saved.field("hookState").choice_i64(&[0, 1, 2])?, "LMCTF hook state")?,
                    hook_length: saved.field("hookLength").finite()?,
                    hook_held: saved.field("hookHeld").boolean()?,
                },
            })
        })?,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::SavedActorId;
    use qa_core::math::Vec3;
    use qa_world::save::records::write_saved_actor;
    use qa_world::save::shared::write_vector;
    use qa_world::save::value::{arr, boolean, encode_checkpoint_value, int, num, obj, str, SaveJson, SaveReader};

    use super::*;

    fn actor(slot: u32) -> SavedActorId {
        SavedActorId { slot, generation: 0 }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn vec(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 {
            x: x as f32,
            y: y as f32,
            z: z as f32,
        }
    }

    #[test]
    fn items_convert() {
        use crate::persistence::q2::items::{Q2ItemsCheckpoint, Q2PickupCheckpoint, Q2PowerCheckpoint};
        let saved = Q2ItemsCheckpoint {
            power_cube_count: 7.0,
            pickups: vec![Q2PickupCheckpoint {
                actor: actor(1),
                classname: "item_quad".to_string(),
                targets_used: true,
                retained: false,
                expires_at: Some(30.0),
            }],
            powers: vec![Q2PowerCheckpoint {
                actor: actor(2),
                quad_until: 10.0,
                invulnerability_until: 20.0,
                breather_until: 0.0,
                enviro_until: 5.0,
            }],
            power_armor_bindings: vec![actor(2)],
        };
        let converted = convert_persistence_q2_items(&saved).unwrap();
        assert_eq!(converted.power_cube_count, 7);
        assert_eq!(converted.pickups.len(), 1);
        assert_eq!(converted.pickups[0].classname, "item_quad");
        assert!(converted.pickups[0].targets_used);
        assert_eq!(converted.powers[0].actor, actor(2));
        assert_eq!(converted.powers[0].state.quad_until, 10.0);
        assert_eq!(converted.powers[0].state.invulnerability_until, 20.0);
        assert_eq!(converted.powers[0].state.enviro_until, 5.0);
        assert_eq!(converted.power_armor_bindings, vec![actor(2)]);
    }

    #[test]
    fn movers_convert() {
        use crate::persistence::q2::movers::{
            Q2AngularMotionEntry, Q2DoorEntry, Q2DoorState, Q2LinearMotionEntry, Q2LinearMotionState, Q2MotionCurve,
            Q2MoversCheckpoint, Q2TrainEntry,
        };
        use qa_content::q2::foundation::movers::DoorPhase;
        let saved = Q2MoversCheckpoint {
            doors: ["bottom", "up", "top", "down"]
                .iter()
                .enumerate()
                .map(|(index, phase)| Q2DoorEntry {
                    actor: actor(index as u32),
                    master: actor(9),
                    team: vec![actor(9)],
                    state: Q2DoorState {
                        start: vec(1.0, 0.0, 0.0),
                        end: vec(0.0, 1.0, 0.0),
                        distance: 8.0,
                        button: true,
                        angular: false,
                        water: false,
                        safe_direction: vec(0.0, 0.0, 1.0),
                        water_divisor: 2.0,
                        reversed: true,
                        activated: true,
                        phase: (*phase).to_string(),
                        debounce: 3.0,
                    },
                })
                .collect(),
            trains: vec![Q2TrainEntry {
                actor: actor(5),
                destination: Some(actor(6)),
                debounce: 1.5,
                ship: true,
            }],
            linear: vec![Q2LinearMotionEntry {
                actor: actor(7),
                state: Q2LinearMotionState {
                    direction: vec(1.0, 0.0, 0.0),
                    destination: vec(2.0, 0.0, 0.0),
                    reference: vec(0.0, 0.0, 0.0),
                    remaining: 4.0,
                    current_speed: 5.0,
                    move_speed: 6.0,
                    next_speed: 7.0,
                    decel_distance: 8.0,
                    done: "AngleMove_Done".to_string(),
                    curve: Some(Q2MotionCurve {
                        positions: vec![1.0, 2.0],
                        frame: 3.0,
                        subframe: 1.0,
                        subframes: 4.0,
                    }),
                },
            }],
            angular: vec![Q2AngularMotionEntry {
                actor: actor(8),
                destination: vec(0.0, 90.0, 0.0),
                speed: 9.0,
                done: "AngleMove_Done".to_string(),
            }],
        };
        let converted = convert_persistence_q2_movers(&saved).unwrap();
        let phases: Vec<DoorPhase> = converted.doors.iter().map(|door| door.state.phase).collect();
        assert_eq!(
            phases,
            vec![DoorPhase::Bottom, DoorPhase::Up, DoorPhase::Top, DoorPhase::Down]
        );
        assert_eq!(converted.doors[0].master, actor(9));
        assert!(converted.doors[0].state.button);
        assert_eq!(converted.trains[0].destination, Some(actor(6)));
        assert!(converted.trains[0].ship);
        let curve = converted.linear[0].curve.as_ref().unwrap();
        assert_eq!(curve.positions, vec![1.0f32, 2.0f32]);
        assert_eq!(curve.frame, 3);
        assert_eq!(converted.linear[0].done, "AngleMove_Done");
        assert_eq!(converted.angular[0].speed, 9.0);

        let mut bad = saved.doors[0].clone();
        bad.state.phase = "sideways".to_string();
        let bad_checkpoint = Q2MoversCheckpoint {
            doors: vec![bad],
            trains: Vec::new(),
            linear: Vec::new(),
            angular: Vec::new(),
        };
        assert!(convert_persistence_q2_movers(&bad_checkpoint).is_err());
    }

    #[test]
    fn monster_enums_convert() {
        use qa_content::q2::foundation::monsters::types::{
            MonsterAttackState, MonsterLocomotion, MonsterPowerArmor, MonsterSpawner, MonsterWeapon,
        };
        assert_eq!(convert_monster_weapon("blaster").unwrap(), MonsterWeapon::Blaster);
        assert_eq!(convert_monster_weapon("shotgun").unwrap(), MonsterWeapon::Shotgun);
        assert_eq!(convert_monster_weapon("machinegun").unwrap(), MonsterWeapon::Machinegun);
        assert!(convert_monster_weapon("railgun").is_err());
        assert_eq!(convert_monster_locomotion("walk").unwrap(), MonsterLocomotion::Walk);
        assert_eq!(convert_monster_locomotion("fly").unwrap(), MonsterLocomotion::Fly);
        assert_eq!(convert_monster_locomotion("swim").unwrap(), MonsterLocomotion::Swim);
        assert_eq!(
            convert_monster_locomotion("stationary").unwrap(),
            MonsterLocomotion::Stationary
        );
        assert!(convert_monster_locomotion("blink").is_err());
        assert_eq!(convert_monster_spawner("none").unwrap(), MonsterSpawner::None);
        assert_eq!(convert_monster_spawner("carrier").unwrap(), MonsterSpawner::Carrier);
        assert_eq!(convert_monster_spawner("medic").unwrap(), MonsterSpawner::Medic);
        assert_eq!(convert_monster_spawner("widow").unwrap(), MonsterSpawner::Widow);
        assert!(convert_monster_spawner("tank").is_err());
        assert_eq!(
            convert_monster_attack_state("straight").unwrap(),
            MonsterAttackState::Straight
        );
        assert_eq!(
            convert_monster_attack_state("sliding").unwrap(),
            MonsterAttackState::Sliding
        );
        assert_eq!(
            convert_monster_attack_state("melee").unwrap(),
            MonsterAttackState::Melee
        );
        assert_eq!(
            convert_monster_attack_state("missile").unwrap(),
            MonsterAttackState::Missile
        );
        assert_eq!(
            convert_monster_attack_state("blind").unwrap(),
            MonsterAttackState::Blind
        );
        assert!(convert_monster_attack_state("sideways").is_err());
        assert_eq!(convert_monster_power_armor("none").unwrap(), MonsterPowerArmor::None);
        assert_eq!(
            convert_monster_power_armor("screen").unwrap(),
            MonsterPowerArmor::Screen
        );
        assert_eq!(
            convert_monster_power_armor("shield").unwrap(),
            MonsterPowerArmor::Shield
        );
        assert!(convert_monster_power_armor("plate").is_err());
    }

    fn sample_monster_state() -> crate::persistence::q2::monsters::Q2MonsterStateCheckpoint {
        use crate::persistence::q2::monsters::{
            Q2AlternateFlyState, Q2FlyPathing, Q2MonsterStateCheckpoint, Q2SoundTarget,
        };
        Q2MonsterStateCheckpoint {
            initial_power_armor_type: "shield".to_string(),
            max_power_armor_power: 50.0,
            base_health: 100.0,
            health_scaling: 2,
            fly: Q2AlternateFlyState {
                alternate_fly: true,
                fly_min_distance: 1.0,
                fly_max_distance: 2.0,
                fly_acceleration: 3.0,
                fly_speed: 4.0,
                fly_ideal_position: vec(1.0, 2.0, 3.0),
                fly_position_time: 5.0,
                fly_buzzard: true,
                fly_above: false,
                fly_pinned: true,
                fly_thrusters: false,
                fly_recovery_time: 6.0,
                fly_recovery_direction: vec(0.0, 1.0, 0.0),
                hint_path: true,
                pathing: Some(Q2FlyPathing {
                    first_move_point: vec(1.0, 0.0, 0.0),
                    second_move_point: vec(0.0, 1.0, 0.0),
                    traversal_pending: true,
                }),
            },
            kind: "soldier".to_string(),
            weapon: "shotgun".to_string(),
            locomotion: "walk".to_string(),
            has_melee: true,
            has_ranged_attack: true,
            has_idle: false,
            has_search: true,
            blind_fire: false,
            good_guy: false,
            target_anger: true,
            ignore_shots: false,
            do_not_count: true,
            spawned_by: "medic".to_string(),
            commander: Some(actor(3)),
            monster_slots: 4.0,
            monster_used: 1.0,
            brutal: true,
            medic: false,
            resurrecting: true,
            move_name: "run".to_string(),
            next_move: Some("walk".to_string()),
            next_frame: 7.0,
            next_move_time: 8.0,
            scale: 1.5,
            gib_health: -40.0,
            can_take_damage: true,
            dead: false,
            corpse: true,
            gibbed: false,
            stand_ground: true,
            temporary_stand_ground: false,
            hold_frame: true,
            ducked: false,
            dodging: true,
            charging: false,
            manual_steering: true,
            combat_point: false,
            attack_state: "missile".to_string(),
            lefty: true,
            ideal_yaw: 90.0,
            yaw_speed: 10.0,
            pause_time: 11.0,
            idle_time: 12.0,
            pain_time: 13.0,
            fire_wait: 14.0,
            duck_wait: 15.0,
            next_duck_time: 16.0,
            dodge_time: 17.0,
            attack_finished: 18.0,
            check_attack_time: 19.0,
            strafe_time: 20.0,
            had_visibility: true,
            close_sight_tripped: false,
            melee_time: 21.0,
            search_time: 22.0,
            trail_time: 23.0,
            show_hostile: 24.0,
            last_sighting: vec(5.0, 6.0, 7.0),
            saved_goal: Some(vec(8.0, 9.0, 10.0)),
            lost_sight: true,
            pursue_next: false,
            pursue_temporary: true,
            pursuit_last_seen: false,
            blind_fire_target: vec(1.0, 1.0, 1.0),
            blind_fire_delay: 25.0,
            sound_target: Some(Q2SoundTarget {
                actor: actor(4),
                owner: actor(5),
                origin: vec(2.0, 2.0, 2.0),
                time: 26.0,
            }),
            old_enemy: Some(actor(6)),
            move_target: Some(actor(7)),
            combat_target: "player".to_string(),
            cocked: true,
            force_refire: false,
            normal_height: 27.0,
            air_finished: 28.0,
            environmental_damage_time: 29.0,
            water_level: 2,
            water_type: 3.0,
            last_link_count: 30.0,
            jump_time: 31.0,
            flies_time: Some(32.0),
        }
    }

    #[test]
    fn monsters_convert() {
        use crate::persistence::q2::monsters::{Q2MonsterActor, Q2MonsterPerception, Q2MonstersCheckpoint, Q2Sighting};
        use qa_content::q2::foundation::monsters::types::{MonsterAttackState, MonsterPowerArmor, MonsterWeapon};
        let saved = Q2MonstersCheckpoint {
            actors: vec![Q2MonsterActor {
                actor: actor(1),
                definition: "monster_soldier".to_string(),
                state: sample_monster_state(),
                pending_damage: None,
            }],
            perception: Q2MonsterPerception {
                sight_client: Some(actor(2)),
                sight: Some(Q2Sighting {
                    actor: actor(2),
                    time: 1.0,
                }),
                alerted: vec![(
                    actor(2),
                    Q2Sighting {
                        actor: actor(3),
                        time: 2.0,
                    },
                )],
                primary: None,
                secondary: None,
                noises: vec![(actor(2), actor(3), actor(4))],
                trails: vec![(actor(2), vec![(vec(1.0, 0.0, 0.0), 3.0, 90.0)])],
                player_origins: vec![(actor(2), vec(4.0, 5.0, 6.0))],
                hostile: vec![Q2Sighting {
                    actor: actor(3),
                    time: 4.0,
                }],
                last_frame: Some(5.0),
            },
        };
        let converted = convert_persistence_q2_monsters(&saved).unwrap();
        assert_eq!(converted.version, 1);
        let entry = &converted.actors[0];
        assert_eq!(entry.definition, "monster_soldier");
        assert_eq!(entry.state.movement, "run");
        assert_eq!(entry.state.next_move, Some("walk".to_string()));
        assert_eq!(entry.state.commander, Some(actor(3)));
        assert_eq!(entry.state.old_enemy, Some(actor(6)));
        assert_eq!(entry.state.move_target, Some(actor(7)));
        let sound = entry.state.sound_target.as_ref().unwrap();
        assert_eq!(sound.actor, actor(4));
        assert_eq!(sound.time, 26.0);
        let state = &entry.state.state;
        assert_eq!(state.weapon, MonsterWeapon::Shotgun);
        assert_eq!(state.attack_state, MonsterAttackState::Missile);
        assert_eq!(state.initial_power_armor, MonsterPowerArmor::Shield);
        assert_eq!(state.monster_slots, 4);
        assert_eq!(state.next_frame, 7);
        assert_eq!(state.health_scaling, 2.0);
        assert_eq!(state.water_level, 2);
        assert_eq!(state.water_type, 3);
        assert_eq!(state.last_link_count, 30);
        assert!(state.alternate_fly);
        assert!(state.pathing.as_ref().unwrap().traversal_pending);
        assert!(state.commander.is_none());
        assert!(state.sound_target.is_none());
        assert!(state.old_enemy.is_none());
        assert!(state.move_target.is_none());
        assert!(state.next_move.is_none());
        assert_eq!(converted.perception.sight_client, Some(actor(2)));
        assert_eq!(converted.perception.noises.len(), 1);
        assert_eq!(converted.perception.trails[0].1[0].yaw, 90.0);
        assert_eq!(converted.perception.hostile, vec![(actor(3), 4.0)]);
        assert_eq!(converted.perception.last_frame, Some(5.0));
    }

    fn sample_attack(
        cause: crate::persistence::q2::foundation::Q2AttackCause,
    ) -> crate::persistence::q2::foundation::Q2AttackCheckpoint {
        use crate::persistence::q2::foundation::Q2AttackCheckpoint;
        Q2AttackCheckpoint {
            sequence: 9,
            time: qa_core::time::SourceTime::Seconds(1.5),
            attacker: Some(actor(1)),
            inflictor: None,
            originating_projectile: Some(actor(2)),
            damage_powerup_owner: Some("q2:game".to_string()),
            weapon: Some("q2:blaster".to_string()),
            weapon_provider: "q2:game".to_string(),
            combat_provider: "q2:game".to_string(),
            inventory_provider: "q2:game".to_string(),
            movement_provider: "q2:game".to_string(),
            cause,
        }
    }

    #[test]
    fn attacks_convert() {
        use crate::persistence::q2::foundation::{Q2AttackCause, Q2NativeCause};
        use qa_content::q2::support::contracts::{AttackCause as ContentCause, EnvironmentHazard, Q1ArmorEffect};
        let q1 = convert_attack(&sample_attack(Q2AttackCause::Q1 {
            death_type: "shot".to_string(),
            armor_effect: Some("bypass".to_string()),
        }))
        .unwrap();
        assert_eq!(q1.attacker, Some(actor(1)));
        assert_eq!(q1.attack.sequence, 9);
        assert!(q1.attack.attacker.is_none());
        assert_eq!(q1.attack.weapon, Some("q2:blaster".to_string()));
        assert!(matches!(
            q1.attack.cause,
            ContentCause::Q1 {
                armor_effect: Some(Q1ArmorEffect::Bypass),
                ..
            }
        ));
        let classic = convert_attack(&sample_attack(Q2AttackCause::Q2 {
            means_of_death: 1,
            damage_flags: 2,
            native: Some(Q2NativeCause::Classic {
                game: "rogue".to_string(),
                value: 3,
            }),
        }))
        .unwrap();
        assert!(matches!(
            classic.attack.cause,
            ContentCause::Q2 { means_of_death: 1, .. }
        ));
        let rerelease = convert_attack(&sample_attack(Q2AttackCause::Q2 {
            means_of_death: 1,
            damage_flags: 0,
            native: Some(Q2NativeCause::Rerelease {
                id: 4,
                friendly_fire: true,
                no_point_loss: false,
            }),
        }))
        .unwrap();
        assert!(matches!(rerelease.attack.cause, ContentCause::Q2 { .. }));
        let q3 = convert_attack(&sample_attack(Q2AttackCause::Q3 {
            means_of_death: 5,
            damage_flags: 6,
        }))
        .unwrap();
        assert!(matches!(
            q3.attack.cause,
            ContentCause::Q3 {
                means_of_death: 5,
                damage_flags: 6
            }
        ));
        let env = convert_attack(&sample_attack(Q2AttackCause::Environment {
            hazard: "lava".to_string(),
        }))
        .unwrap();
        assert!(matches!(
            env.attack.cause,
            ContentCause::Environment {
                hazard: EnvironmentHazard::Lava
            }
        ));
        assert!(convert_attack(&sample_attack(Q2AttackCause::Environment {
            hazard: "void".to_string()
        }))
        .is_err());
        assert!(convert_attack(&sample_attack(Q2AttackCause::Q1 {
            death_type: "shot".to_string(),
            armor_effect: Some("full".to_string()),
        }))
        .is_err());
        let mut bad_provider = sample_attack(Q2AttackCause::Q3 {
            means_of_death: 0,
            damage_flags: 0,
        });
        bad_provider.weapon_provider = "not-a-provider".to_string();
        assert!(convert_attack(&bad_provider).is_err());
    }

    #[test]
    fn weapons_convert() {
        use crate::persistence::q2::weapons::Q2HandReservation;
        use crate::persistence::q2::weapons::{Q2NoiseCheckpoint, Q2WeaponInput, Q2WeaponState, Q2WeaponsCheckpoint};
        use qa_content::q2::foundation::weapons::types::{PrimaryHandoff, Q2WeaponPhase, WeaponHand};
        use qa_content::q2::foundation::weapons::WeaponSourceRules;
        let saved = Q2WeaponsCheckpoint {
            silencer_charges: vec![(actor(1), 5)],
            source_rules: "lmctf".to_string(),
            registered: vec!["blaster".to_string()],
            fallback_order: Some(vec!["blaster".to_string()]),
            states: vec![(
                actor(1),
                Q2WeaponState {
                    primary_handoff: "holstering".to_string(),
                    hand_reservation: Q2HandReservation::Finite,
                    source_firing: true,
                    weapon: Some("blaster".to_string()),
                    last_weapon: None,
                    pending: Some("shotgun".to_string()),
                    phase: "firing".to_string(),
                    frame: 3.0,
                    think_time: 1.0,
                    fire_finished: 2.0,
                    fire_buffered: true,
                    latched_attack: false,
                    machinegun_shots: 4.0,
                    empty_sound_time: 0.0,
                    grenade_time: 0.0,
                    grenade_finished: 0.0,
                    grenade_blew_up: false,
                    kick_origin: vec(1.0, 0.0, 0.0),
                    kick_angles: vec(0.0, 1.0, 0.0),
                    kick_time: 1.0,
                    kick_until: 2.0,
                    kick_duration: 1.0,
                    loop_sound: String::new(),
                    view_model: Some("models/v_blast.md2".to_string()),
                    view_skin: 2.0,
                    last_firing_time: 3.0,
                    gun_rate: 4.0,
                },
            )],
            inputs: vec![(
                actor(1),
                Q2WeaponInput {
                    attack: true,
                    latched_attack: false,
                    holster: false,
                    angles: vec(0.0, 0.0, 0.0),
                    ducked: true,
                    spectator: false,
                    notarget: true,
                    hand: "left".to_string(),
                    animate_player: true,
                    quad_until: 0.0,
                    double_until: 0.0,
                    quad_fire_until: 0.0,
                    haste: false,
                    no_stack_double: true,
                    instant_switch: false,
                    quick_switch: true,
                    infinite_ammo: false,
                    players_collide: true,
                    gravity: 800.0,
                    weapon_thunk: false,
                },
            )],
            noises: vec![(
                actor(1),
                Some(Q2NoiseCheckpoint {
                    actor: actor(2),
                    origin: vec(1.0, 2.0, 3.0),
                    time: 9.0,
                    secondary: false,
                }),
                None,
            )],
            sound_entity: None,
            sound2_entity: None,
            blaster_causes: vec![(actor(3), 7)],
        };
        let converted = convert_persistence_q2_weapons(&saved).unwrap();
        assert_eq!(converted.format_version, 2);
        assert_eq!(converted.source_rules, WeaponSourceRules::Lmctf);
        assert_eq!(converted.registered, vec!["blaster".to_string()]);
        let state = &converted.states[0].state;
        assert_eq!(state.primary_handoff, PrimaryHandoff::Holstering);
        assert_eq!(state.phase, Q2WeaponPhase::Firing);
        assert_eq!(state.frame, 3);
        assert_eq!(state.machinegun_shots, 4);
        assert_eq!(state.view_skin, 2);
        assert_eq!(state.pending, Some("shotgun".to_string()));
        let input = &converted.inputs[0].input;
        assert_eq!(input.hand, WeaponHand::Left);
        assert_eq!(input.view_height, 22.0);
        assert!(input.ducked);
        assert_eq!(converted.noises[0].actor, actor(1));
        assert_eq!(converted.noises[0].primary.as_ref().unwrap().time, 9.0);
        assert!(converted.noises[0].secondary.is_none());
        assert_eq!(converted.blaster_causes[0].means_of_death, 7);
        assert_eq!(converted.silencer_charges[0].charges, 5);

        let mut bad = saved.clone();
        bad.source_rules = "quake".to_string();
        assert!(convert_persistence_q2_weapons(&bad).is_err());
        let mut bad_means = saved.clone();
        bad_means.blaster_causes = vec![(actor(3), i64::from(i32::MAX) + 1)];
        assert!(convert_persistence_q2_weapons(&bad_means).is_err());
    }

    fn sample_player_state() -> crate::persistence::q2::players::Q2PlayerStateCheckpoint {
        use crate::persistence::q2::players::Q2PlayerStateCheckpoint;
        use qa_world::combat::{ArmorState, PoweredProtection, RegularArmor};
        Q2PlayerStateCheckpoint {
            slot: 1,
            entered_at: 0.0,
            use_q2_weapons: true,
            use_q2_inventory: true,
            spawn_inventory: Vec::new(),
            userinfo: String::new(),
            name: "player".to_string(),
            skin: "male/grunt".to_string(),
            gender: "female".to_string(),
            fov: 90.0,
            hand: "center".to_string(),
            spectator: false,
            requested_spectator: false,
            connected: true,
            dead: false,
            gibbed: false,
            noclip: false,
            god: true,
            notarget: false,
            score: 5.0,
            ping: 50.0,
            respawn_time: 0.0,
            air_finished: 0.0,
            next_drown_time: 0.0,
            drown_damage: 2.0,
            old_water_level: 1.0,
            breather_sound: 0.0,
            pain_debounce: 0.0,
            damage_blood: 0.0,
            damage_armor: 0.0,
            damage_power_armor: 0.0,
            damage_knockback: 0.0,
            damage_from: vec(0.0, 0.0, 0.0),
            damage_blend: vec(0.0, 0.0, 0.0),
            damage_alpha: 0.0,
            bonus_alpha: 0.0,
            damage_pitch: 0.0,
            damage_roll: 0.0,
            damage_time: 0.0,
            power_armor_time: 0.0,
            fall_time: 0.0,
            fall_value: 0.0,
            landmark_free_fall: false,
            landmark_noise_time: 0.0,
            old_velocity: vec(0.0, 0.0, 0.0),
            old_view_angles: vec(0.0, 0.0, 0.0),
            killer_yaw: 0.0,
            buttons: 1.0,
            latched_buttons: 0.0,
            weapon_thunk: false,
            bob_time: 0.0,
            bob_move: 0.0,
            event: String::new(),
            animation_priority: 0.0,
            animation_end: 0.0,
            animation_duck: false,
            animation_run: true,
            loop_sound: String::new(),
            selected_item: None,
            show_scores: false,
            show_inventory: true,
            show_help: false,
            chase_target: Some(actor(9)),
            coop_respawn: Some(crate::persistence::q2::players::Q2PlayerCarry {
                health: 100.0,
                maximum_health: 100.0,
                armor: ArmorState {
                    regular: RegularArmor::None,
                    powered: PoweredProtection::None,
                },
                inventory: Vec::new(),
                weapon: Some("q2:blaster".to_string()),
                selected_item: None,
                score: 1.0,
                flags: 2.0,
                power_cubes: 0.0,
            }),
            flood_times: vec![1.0, 2.0],
            flood_lock_until: 0.0,
        }
    }

    fn sample_player_rules() -> crate::persistence::q2::players::Q2PlayerRules {
        use crate::persistence::q2::players::Q2PlayerRules;
        Q2PlayerRules {
            password: "pw".to_string(),
            spectator_password: String::new(),
            max_spectators: 4.0,
            cheats: true,
            time_limit_minutes: 10.0,
            frag_limit: 20.0,
            map_list: vec!["q2dm1".to_string()],
            map_list_shuffle: false,
            next_map: "q2dm2".to_string(),
            spawn_point: String::new(),
            flood_messages: 4.0,
            flood_seconds: 8.0,
            flood_wait_seconds: 10.0,
            roll_speed: 200.0,
            roll_angle: 2.0,
            run_pitch: 0.002,
            run_roll: 0.005,
            bob_up: 0.005,
            bob_pitch: 0.002,
            bob_roll: 0.002,
            gun_offset: vec(0.0, 0.0, 0.0),
        }
    }

    #[test]
    fn players_convert() {
        use crate::persistence::q2::players::{Q2IntermissionLandmark, Q2PlayerIntermission, Q2PlayersCheckpoint};
        use qa_content::q2::base::player::checkpoint::Q2PlayerIntermissionCheckpoint as ContentIntermission;
        use qa_content::q2::base::player::types::{Q2PlayerGender, Q2PlayerHand};
        let saved = Q2PlayersCheckpoint {
            corpse_index: 3,
            death_animation: 1.0,
            pain_animation: 2.0,
            rules: sample_player_rules(),
            intermission: Q2PlayerIntermission::Intermission {
                map: "q2dm1".to_string(),
                started: 99.0,
                exit: true,
                landmark: Some(Q2IntermissionLandmark {
                    player: actor(1),
                    name: "base1".to_string(),
                    relative_origin: vec(1.0, 0.0, 0.0),
                    relative_velocity: vec(0.0, 1.0, 0.0),
                    relative_view_angles: vec(0.0, 0.0, 1.0),
                }),
            },
            players: vec![(actor(1), sample_player_state())],
        };
        let converted = convert_persistence_q2_players(&saved).unwrap();
        assert_eq!(converted.version, 1);
        assert_eq!(converted.corpse_index, 3);
        assert_eq!(converted.death_animation, 1);
        assert_eq!(converted.rules.max_spectators, 4);
        assert!(converted.rules.cheats);
        assert_eq!(converted.rules.map_list, vec!["q2dm1".to_string()]);
        match &converted.intermission {
            ContentIntermission::Intermission {
                map,
                started,
                exit,
                landmark,
            } => {
                assert_eq!(map, "q2dm1");
                assert_eq!(*started, 99.0);
                assert!(*exit);
                let landmark = landmark.as_ref().unwrap();
                assert_eq!(landmark.player, actor(1));
                assert_eq!(landmark.name, "base1");
            }
            ContentIntermission::Playing => panic!("expected intermission"),
        }
        let entry = &converted.players[0];
        assert_eq!(entry.actor, actor(1));
        assert_eq!(entry.state.state.gender, Q2PlayerGender::Female);
        assert_eq!(entry.state.state.hand, Q2PlayerHand::Center);
        assert_eq!(entry.state.state.slot, 1);
        assert!(entry.state.state.god);
        assert_eq!(entry.state.chase_target, Some(actor(9)));
        assert!(entry.state.state.chase_target.is_none());
        assert_eq!(entry.state.state.flood_times, vec![1.0, 2.0]);
        assert_eq!(entry.state.state.coop_respawn.as_ref().unwrap().score, 1);

        let playing = Q2PlayersCheckpoint {
            intermission: Q2PlayerIntermission::Playing,
            players: Vec::new(),
            ..saved.clone()
        };
        let converted = convert_persistence_q2_players(&playing).unwrap();
        assert!(matches!(converted.intermission, ContentIntermission::Playing));
        assert!(converted.players.is_empty());

        let mut bad_corpse = saved.clone();
        bad_corpse.corpse_index = u64::from(u32::MAX) + 1;
        assert!(convert_persistence_q2_players(&bad_corpse).is_err());
        let mut bad_gender = saved.clone();
        bad_gender.players[0].1.gender = "other".to_string();
        assert!(convert_persistence_q2_players(&bad_gender).is_err());
    }

    #[test]
    fn base_entities_convert() {
        use crate::persistence::q2::base_entities::{
            Q2BaseEntitiesCheckpoint, Q2BreachState, Q2PlatformState, Q2SecretState, Q2TurretDriver,
        };
        use qa_content::q2::base::entities::movers::Q2PlatformPhase;
        let saved = Q2BaseEntitiesCheckpoint {
            platforms: vec![(
                actor(1),
                Q2PlatformState {
                    top: vec(0.0, 0.0, 10.0),
                    bottom: vec(0.0, 0.0, 0.0),
                    phase: "up".to_string(),
                },
            )],
            secrets: vec![(
                actor(2),
                Q2SecretState {
                    first: vec(1.0, 0.0, 0.0),
                    second: vec(2.0, 0.0, 0.0),
                    home: vec(0.0, 0.0, 0.0),
                    shootable: true,
                    blocked_time: 1.0,
                    message_time: 2.0,
                },
            )],
            linear: Vec::new(),
            animations: vec![(actor(3), 4, 9)],
            clocks: vec![(actor(4), 42.0)],
            breaches: vec![(
                actor(5),
                Q2BreachState {
                    goal: vec(0.0, 0.0, 0.0),
                    muzzle: vec(1.0, 1.0, 1.0),
                    pitch_max: 45.0,
                    pitch_min: -45.0,
                    yaw_min: -90.0,
                    yaw_max: 90.0,
                },
            )],
            drivers: vec![Q2TurretDriver {
                actor: actor(6),
                breach: Some(actor(5)),
                radius: 10.0,
                yaw_offset: 5.0,
                height: 20.0,
                monster_die: "infantry_die".to_string(),
            }],
            wind_times: vec![(actor(7), 3.5)],
        };
        let converted = convert_persistence_q2_base_entities(&saved).unwrap();
        assert_eq!(converted.version, 1);
        assert_eq!(converted.movers.platforms[0].state.phase, Q2PlatformPhase::Up);
        assert!(converted.movers.secrets[0].state.shootable);
        assert_eq!(converted.scenery.animations[0].first, 4);
        assert_eq!(converted.scenery.animations[0].end, 9);
        assert_eq!(converted.scenery.clocks[0].value, 42);
        assert_eq!(converted.turrets.breaches[0].state.pitch_max, 45.0);
        assert_eq!(converted.turrets.drivers[0].breach, Some(actor(5)));
        assert_eq!(converted.turrets.drivers[0].monster_die, "infantry_die");
        assert_eq!(converted.wind_times[0].until, 3.5);

        let mut bad = saved.clone();
        bad.platforms[0].1.phase = "sideways".to_string();
        assert!(convert_persistence_q2_base_entities(&bad).is_err());
        let mut bad_anim = saved.clone();
        bad_anim.animations = vec![(actor(3), u64::from(u32::MAX) + 1, 9)];
        assert!(convert_persistence_q2_base_entities(&bad_anim).is_err());
    }

    #[test]
    fn missionpacks_convert() {
        use crate::persistence::q2::missionpacks::{
            Q2DeathBallCheckpoint, Q2HintNode, Q2HintPursuer, Q2MissionPackItemsCheckpoint, Q2MissionPackMonsterState,
            Q2MissionPackMonstersCheckpoint, Q2MissionPackPower, Q2RogueEntitiesCheckpoint, Q2RogueHintsCheckpoint,
            Q2TagCheckpoint,
        };
        use qa_content::q2::missionpacks::monsters::state::RogueFlyerNext;
        let items = convert_persistence_q2_missionpack_items(&Q2MissionPackItemsCheckpoint {
            powers: vec![Q2MissionPackPower {
                actor: actor(1),
                quad_fire_until: 1.0,
                double_until: 2.0,
                ir_until: 3.0,
            }],
        });
        assert_eq!(items.powers[0].1.ir_until, 3.0);
        let monsters = convert_persistence_q2_missionpack_monsters(&Q2MissionPackMonstersCheckpoint {
            flyer_next_move: "run".to_string(),
            hints: Some(Q2RogueHintsCheckpoint {
                present: true,
                starts: vec![actor(1)],
                nodes: vec![Q2HintNode {
                    actor: actor(1),
                    chain: -1,
                    next: Some(actor(2)),
                }],
                monsters: vec![Q2HintPursuer {
                    actor: actor(3),
                    goal: None,
                    last_time: 4.0,
                }],
            }),
            widow_shots_fired: 6.0,
            widow_damage_multiplier: 2,
            actors: vec![(
                actor(4),
                Q2MissionPackMonsterState {
                    blocked: true,
                    turret_orientation: 7.0,
                    healer: Some(actor(5)),
                    bad_medic1: None,
                    bad_medic2: None,
                    medic_tries: 8.0,
                    chosen_reinforcements: vec![1, 2],
                    react_to_damage_time: 9.0,
                    summon_strength: 10.0,
                    last_player_enemy: Some(actor(6)),
                    bad_area: None,
                    good_guy: false,
                    widow_quad_until: 11.0,
                    widow_double_until: 12.0,
                    widow_invulnerable_until: 13.0,
                },
            )],
        })
        .unwrap();
        assert_eq!(monsters.version, 1);
        assert_eq!(monsters.flyer_next_move, RogueFlyerNext::Run);
        assert_eq!(monsters.widow_shots_fired, 6);
        assert_eq!(monsters.widow_damage_multiplier, 2);
        let hints = monsters.hints.as_ref().unwrap();
        assert!(hints.present);
        assert_eq!(hints.nodes[0].chain, -1);
        assert_eq!(hints.monsters[0].last_time, 4.0);
        assert_eq!(monsters.actors[0].medic_tries, 8);
        assert_eq!(monsters.actors[0].chosen_reinforcements, vec![1, 2]);
        assert_eq!(monsters.actors[0].summon_strength, 10);
        assert_eq!(monsters.actors[0].healer, Some(actor(5)));
        let rogue = convert_persistence_q2_rogue_entities(&Q2RogueEntitiesCheckpoint { steam_id: 11.0 });
        assert_eq!(rogue.steam_id, 11);
        let tag = convert_persistence_q2_tag(&Q2TagCheckpoint {
            token: Some(actor(1)),
            owner: None,
            count: 14.0,
        });
        assert_eq!(tag.count, 14);
        assert_eq!(tag.token, Some(actor(1)));
        let ball = convert_persistence_q2_deathball(&Q2DeathBallCheckpoint {
            ball: None,
            starts: 15.0,
            team1_score: 16.0,
            team2_score: 17.0,
        });
        assert_eq!(ball.starts, 15);
        assert_eq!(ball.team2_score, 17.0);

        let bad = convert_persistence_q2_missionpack_monsters(&Q2MissionPackMonstersCheckpoint {
            flyer_next_move: "walk".to_string(),
            hints: None,
            widow_shots_fired: 0.0,
            widow_damage_multiplier: 1,
            actors: Vec::new(),
        });
        assert!(bad.is_err());
    }

    fn fog_json() -> SaveJson {
        obj(vec![
            (
                "fog",
                obj(vec![
                    ("density", num(0.1)),
                    ("color", write_vector(vec(1.0, 1.0, 1.0))),
                    ("skyFactor", num(0.5)),
                ]),
            ),
            (
                "heightFog",
                obj(vec![
                    ("startColor", write_vector(vec(0.0, 0.0, 0.0))),
                    ("startDistance", num(10.0)),
                    ("endColor", write_vector(vec(1.0, 0.0, 0.0))),
                    ("endDistance", num(20.0)),
                    ("falloff", num(1.5)),
                    ("density", num(0.2)),
                ]),
            ),
        ])
    }

    fn rerelease_player_json() -> SaveJson {
        obj(vec![
            ("spawned", boolean(true)),
            ("gameHelp1Changed", num(1.0)),
            ("gameHelp2Changed", num(2.0)),
            ("helpChanged", num(3.0)),
            ("helpTime", num(4.0)),
            ("seat", int(2)),
            ("socialId", str("player-one")),
            ("autoSwitch", int(3)),
            ("autoShield", num(5.0)),
            ("dogtag", str("tag")),
            ("slimeDebounce", num(6.0)),
            ("windSoundTime", num(7.0)),
            ("animationTime", num(8.0)),
            ("flashTime", num(9.0)),
            ("flashes", num(10.0)),
            ("lastDamageUntil", num(11.0)),
            ("lastFiringUntil", num(12.0)),
            ("lives", num(13.0)),
            ("invisibilityUntil", num(14.0)),
            ("invisibilityFadeUntil", num(15.0)),
            ("coopRespawnState", str("waiting")),
            ("flashlight", boolean(true)),
            ("fog", fog_json()),
            ("wantedFog", fog_json()),
            ("fogTransition", num(16.0)),
            ("bobSkip", boolean(false)),
            ("impactDelta", num(17.0)),
            ("onLadder", boolean(true)),
            ("grappleReleasedUntil", num(18.0)),
            ("grappleAttached", boolean(false)),
            ("awaitingRespawn", boolean(true)),
            ("respawnTimeout", num(19.0)),
            (
                "pendingLandmark",
                obj(vec![
                    ("name", str("base1")),
                    ("relativeOrigin", write_vector(vec(1.0, 0.0, 0.0))),
                    ("relativeVelocity", write_vector(vec(0.0, 1.0, 0.0))),
                    ("relativeViewAngles", write_vector(vec(0.0, 0.0, 1.0))),
                ]),
            ),
            ("slowViewAngles", write_vector(vec(0.0, 0.0, 0.0))),
            ("quakeTime", num(20.0)),
            ("helpLocation", write_vector(vec(3.0, 3.0, 3.0))),
            ("helpImage", str("help.pcx")),
            ("helpPoints", arr(vec![write_vector(vec(1.0, 2.0, 3.0))])),
            ("helpIndex", num(21.0)),
            ("helpDrawTime", num(22.0)),
        ])
    }

    #[test]
    fn rerelease_players_read() {
        use qa_content::q2::rerelease::types::Q2CoopRespawnState;
        let payload = encode_checkpoint_value(&obj(vec![
            ("version", int(1)),
            (
                "options",
                obj(vec![
                    ("coopSquadRespawn", boolean(true)),
                    ("coopInstancedItems", boolean(false)),
                    ("coopLives", boolean(true)),
                    ("coopNumLives", num(3.0)),
                    ("deathmatchForceRespawn", boolean(false)),
                    ("deathmatchNoFallDamage", boolean(true)),
                    ("deathmatchAllowExit", boolean(false)),
                    ("deathmatchSpawnFarthest", boolean(true)),
                    ("deathmatchForceRespawnTime", num(5.0)),
                    ("coopPlayerCollision", boolean(false)),
                    ("autoSaveMinimumTime", num(60.0)),
                ]),
            ),
            ("coopRestartTime", num(100.0)),
            ("intermissionFlags", num(7.0)),
            ("intermissionFadeUntil", SaveJson::Null),
            (
                "intermissionCamera",
                obj(vec![
                    ("origin", write_vector(vec(1.0, 2.0, 3.0))),
                    ("angles", write_vector(vec(0.0, 90.0, 0.0))),
                ]),
            ),
            ("intermissionCameraSet", boolean(true)),
            ("deadlyKillBox", boolean(false)),
            (
                "players",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(actor(1))),
                    ("state", rerelease_player_json()),
                ])]),
            ),
            (
                "squadSpawns",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(actor(2))),
                    ("origin", write_vector(vec(4.0, 5.0, 6.0))),
                    ("angles", write_vector(vec(0.0, 0.0, 0.0))),
                ])]),
            ),
        ]));
        let value = decode_checkpoint_value(&payload).unwrap();
        let checkpoint = read_q2_rerelease_players_checkpoint(SaveReader::at(&value, "q2-rerelease-players")).unwrap();
        assert_eq!(checkpoint.version, 1);
        assert!(checkpoint.options.coop_squad_respawn);
        assert_eq!(checkpoint.options.coop_num_lives, 3);
        assert_eq!(checkpoint.coop_restart_time, 100.0);
        assert_eq!(checkpoint.intermission_flags, 7);
        assert!(checkpoint.intermission_fade_until.is_none());
        assert!(checkpoint.intermission_camera_set);
        assert_eq!(checkpoint.squad_spawns[0].actor, actor(2));
        let state = &checkpoint.players[0].state;
        assert!(state.spawned);
        assert_eq!(state.game_help1_changed, 1);
        assert_eq!(state.seat, 2);
        assert_eq!(state.social_id, "player-one");
        assert_eq!(state.auto_switch, 3);
        assert_eq!(state.lives, 13);
        assert_eq!(state.coop_respawn_state, Q2CoopRespawnState::Waiting);
        assert!(state.flashlight);
        assert_eq!(state.fog.fog.density, 0.1);
        assert_eq!(state.wanted_fog.height_fog.falloff, 1.5);
        assert_eq!(state.help_points.len(), 1);
        assert_eq!(state.help_marker_until, 0.0);
        assert_eq!(state.pending_landmark.as_ref().unwrap().name, "base1");
        assert_eq!(state.flashes, 10);
    }

    #[test]
    fn rerelease_module_read() {
        let payload = encode_checkpoint_value(&obj(vec![
            ("version", int(1)),
            ("worldFog", fog_json()),
            ("story", str("intro")),
            (
                "sky",
                obj(vec![
                    ("name", str("unit1_")),
                    ("rotation", num(2.0)),
                    ("autoRotate", boolean(true)),
                    ("axis", write_vector(vec(0.0, 0.0, 1.0))),
                ]),
            ),
            (
                "poi",
                obj(vec![
                    ("actor", write_saved_actor(actor(1))),
                    ("origin", write_vector(vec(1.0, 1.0, 1.0))),
                    ("image", str("poi.pcx")),
                    ("dynamic", SaveJson::Null),
                ]),
            ),
            ("poiStage", num(3.0)),
            ("lastAutoSave", num(99.0)),
            (
                "goals",
                obj(vec![("goals", str("goal-list")), ("goalNumber", num(4.0))]),
            ),
            (
                "campaign",
                obj(vec![
                    (
                        "mission",
                        obj(vec![
                            ("primary", str("find-exit")),
                            ("secondary", str("optional")),
                            ("primaryChanges", num(1.0)),
                            ("secondaryChanges", num(2.0)),
                        ]),
                    ),
                    ("crossUnitFlags", num(5.0)),
                    ("visitedMaps", arr(vec![str("base1")])),
                    (
                        "levels",
                        arr(vec![obj(vec![
                            ("map", str("base1")),
                            ("name", str("Outer Base")),
                            ("visitOrder", num(1.0)),
                            ("totalSecrets", num(2.0)),
                            ("foundSecrets", num(1.0)),
                            ("totalMonsters", num(10.0)),
                            ("killedMonsters", num(7.0)),
                            ("time", num(120.0)),
                        ])]),
                    ),
                ]),
            ),
            (
                "q64",
                obj(vec![
                    (
                        "eyes",
                        arr(vec![obj(vec![
                            ("actor", write_saved_actor(actor(3))),
                            (
                                "state",
                                obj(vec![
                                    ("neutralAngles", write_vector(vec(0.0, 0.0, 0.0))),
                                    ("eyePosition", write_vector(vec(1.0, 1.0, 1.0))),
                                    ("visionCone", num(0.9)),
                                ]),
                            ),
                        ])]),
                    ),
                    ("cameras", arr(Vec::new())),
                    (
                        "dummies",
                        arr(vec![obj(vec![
                            ("actor", write_saved_actor(actor(4))),
                            (
                                "state",
                                obj(vec![
                                    ("fadeRemaining", num(1.0)),
                                    ("fadeDuration", num(2.0)),
                                    ("fading", boolean(true)),
                                ]),
                            ),
                        ])]),
                    ),
                ]),
            ),
            (
                "lights",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(actor(5))),
                    ("active", boolean(false)),
                ])]),
            ),
            (
                "healthBars",
                arr(vec![
                    SaveJson::Null,
                    obj(vec![
                        ("controller", write_saved_actor(actor(6))),
                        ("target", write_saved_actor(actor(7))),
                        ("deadUntil", num(50.0)),
                    ]),
                ]),
            ),
            (
                "healthTargets",
                arr(vec![obj(vec![
                    ("controller", write_saved_actor(actor(6))),
                    ("target", write_saved_actor(actor(7))),
                ])]),
            ),
            (
                "pickedUpBy",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(actor(8))),
                    ("slots", arr(vec![int(0), int(3)])),
                ])]),
            ),
            (
                "triggerSoundTimes",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(actor(9))),
                    ("time", num(60.0)),
                ])]),
            ),
        ]));
        let value = decode_checkpoint_value(&payload).unwrap();
        let checkpoint = read_q2_rerelease_module_checkpoint(SaveReader::at(&value, "q2-rerelease-module")).unwrap();
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.story, "intro");
        assert_eq!(checkpoint.sky.name, "unit1_");
        assert!(checkpoint.sky.auto_rotate);
        assert_eq!(checkpoint.poi.as_ref().unwrap().image, "poi.pcx");
        assert!(checkpoint.poi.as_ref().unwrap().dynamic.is_none());
        assert_eq!(checkpoint.poi_stage, 3);
        assert_eq!(checkpoint.goals.goals, Some("goal-list".to_string()));
        assert_eq!(checkpoint.goals.goal_number, 4);
        assert_eq!(checkpoint.campaign.cross_unit_flags, 5);
        assert_eq!(checkpoint.campaign.visited_maps, vec!["base1".to_string()]);
        assert_eq!(checkpoint.campaign.mission.primary_changes, 1);
        assert_eq!(checkpoint.campaign.levels[0].killed_monsters, 7);
        assert_eq!(checkpoint.campaign.levels[0].time, 120.0);
        assert_eq!(checkpoint.q64.eyes[0].state.vision_cone, 0.9);
        assert!(checkpoint.q64.cameras.is_empty());
        assert!(checkpoint.q64.dummies[0].state.fading);
        assert!(!checkpoint.lights[0].active);
        assert!(checkpoint.health_bars[0].is_none());
        assert_eq!(checkpoint.health_bars[1].as_ref().unwrap().dead_until, Some(50.0));
        assert_eq!(checkpoint.health_targets[0].target, actor(7));
        assert_eq!(checkpoint.picked_up_by[0].slots, vec![0, 3]);
        assert_eq!(checkpoint.trigger_sound_times[0].time, 60.0);
    }

    #[test]
    fn ctf_read() {
        use qa_content::q2::equipment::grapple_services::CtfGrapplePhase;
        use qa_content::q2::multiplayer::ctf::types::{Q2CtfElectionKind, Q2CtfForceJoin, Q2CtfMatchPhase};
        let payload = encode_checkpoint_value(&obj(vec![
            ("version", int(1)),
            (
                "rules",
                obj(vec![
                    ("forceJoin", str("red")),
                    ("competition", int(1)),
                    ("matchLock", boolean(true)),
                    ("electionPercentage", num(0.6)),
                    ("matchMinutes", num(10.0)),
                    ("setupMinutes", num(1.0)),
                    ("startSeconds", num(5.0)),
                    ("captureLimit", int(8)),
                    ("instantWeapons", boolean(false)),
                ]),
            ),
            (
                "match",
                obj(vec![
                    ("team1", int(3)),
                    ("team2", int(4)),
                    ("total1", int(30)),
                    ("total2", int(40)),
                    ("lastFlagCapture", num(500.0)),
                    ("lastCaptureTeam", int(2)),
                    ("phase", str("game")),
                    ("matchTime", num(600.0)),
                    ("lastTime", int(590)),
                    (
                        "election",
                        obj(vec![
                            ("kind", str("map")),
                            ("target", write_saved_actor(actor(1))),
                            ("map", str("q2ctf1")),
                            ("message", str("vote")),
                            ("votes", int(3)),
                            ("needed", int(5)),
                            ("expires", num(700.0)),
                        ]),
                    ),
                    (
                        "ghosts",
                        arr(vec![obj(vec![
                            ("code", int(12345)),
                            ("team", int(1)),
                            ("name", str("ghost")),
                            ("actor", write_saved_actor(actor(2))),
                            ("score", int(10)),
                            ("deaths", int(1)),
                            ("kills", int(5)),
                            ("captures", int(2)),
                            ("baseDefense", int(3)),
                            ("carrierDefense", int(4)),
                        ])]),
                    ),
                ]),
            ),
            (
                "players",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(actor(3))),
                    (
                        "state",
                        obj(vec![
                            ("team", int(2)),
                            ("spawnState", int(1)),
                            ("lastHurtCarrier", SaveJson::Null),
                            ("lastReturnedFlag", num(10.0)),
                            ("lastFraggedCarrier", SaveJson::Null),
                            ("flagSince", num(20.0)),
                            ("voted", boolean(true)),
                            ("ready", boolean(false)),
                            ("admin", boolean(true)),
                            ("idView", boolean(false)),
                            ("ghostCode", int(12345)),
                            ("grapple", SaveJson::Null),
                            ("grappleState", str("hang")),
                            ("grappleReleaseTime", num(30.0)),
                            ("regenTime", num(40.0)),
                            ("techSoundTime", num(50.0)),
                            ("lastTechMessage", num(60.0)),
                            ("matchRespawnAt", SaveJson::Null),
                        ]),
                    ),
                ])]),
            ),
        ]));
        let value = decode_checkpoint_value(&payload).unwrap();
        let checkpoint = read_q2_ctf_checkpoint(SaveReader::at(&value, "q2-ctf")).unwrap();
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.rules.force_join, Q2CtfForceJoin::Red);
        assert_eq!(checkpoint.rules.capture_limit, 8);
        assert!(checkpoint.rules.match_lock);
        assert_eq!(checkpoint.match_state.team1, 3);
        assert_eq!(checkpoint.match_state.total2, 40);
        assert_eq!(checkpoint.match_state.last_capture_team, Some(2));
        assert_eq!(checkpoint.match_state.phase, Q2CtfMatchPhase::Game);
        assert_eq!(checkpoint.match_state.last_time, 590.0);
        let election = checkpoint.match_state.election.as_ref().unwrap();
        assert_eq!(election.kind, Q2CtfElectionKind::Map);
        assert_eq!(election.target, actor(1));
        assert_eq!(election.needed, 5);
        let ghost = &checkpoint.match_state.ghosts[0];
        assert_eq!(ghost.code, 12345);
        assert_eq!(ghost.team, 1);
        assert_eq!(ghost.actor, Some(actor(2)));
        assert_eq!(ghost.carrier_defense, 4);
        let player = &checkpoint.players[0];
        assert_eq!(player.actor, actor(3));
        assert_eq!(player.state.team, 2);
        assert_eq!(player.state.last_returned_flag, Some(10.0));
        assert!(player.state.last_hurt_carrier.is_none());
        assert_eq!(player.state.ghost_code, Some(12345));
        assert_eq!(player.grapple.grapple_state, CtfGrapplePhase::Hang);
        assert!(player.grapple.grapple_no_knockback.is_none());
    }

    #[test]
    fn lmctf_read() {
        use qa_content::q2::multiplayer::lmctf::match_::LmctfMatchPhase;
        let payload = encode_checkpoint_value(&obj(vec![
            (
                "rules",
                obj(vec![
                    ("timeLimitMinutes", num(15.0)),
                    ("fragLimit", int(20)),
                    ("mapList", arr(vec![str("q2dm1")])),
                    ("ctfFlags", int(1)),
                    ("refFlags", int(0)),
                    ("runes", int(3)),
                    ("skinSet", int(2)),
                    ("flagInit", boolean(true)),
                    ("disabledWeapons", int(0)),
                    ("fastSwitch", boolean(false)),
                    ("autoLock", boolean(true)),
                    ("countdownSeconds", num(10.0)),
                    ("quadSeconds", num(60.0)),
                ]),
            ),
            (
                "match",
                obj(vec![
                    (
                        "pendingMap",
                        obj(vec![("map", str("q2dm2")), ("countdown", boolean(true))]),
                    ),
                    ("phase", str("countdown")),
                    ("remaining", int(9)),
                    ("nextThink", num(100.0)),
                    ("paused", boolean(false)),
                    ("teamsLocked", boolean(true)),
                ]),
            ),
            ("vote", obj(vec![("startedAt", num(50.0))])),
            ("plasmaQuad", boolean(true)),
            (
                "flags",
                obj(vec![
                    ("lastTakenSound", num(70.0)),
                    (
                        "flags",
                        arr(vec![obj(vec![
                            ("team", int(1)),
                            ("actor", write_saved_actor(actor(1))),
                        ])]),
                    ),
                ]),
            ),
            ("runes", obj(vec![("forward", boolean(false))])),
            (
                "players",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(actor(2))),
                    (
                        "state",
                        obj(vec![
                            ("plasmaMode", boolean(true)),
                            ("team", int(1)),
                            ("observerTeam", int(0)),
                            ("rune", SaveJson::Null),
                            ("hook", write_saved_actor(actor(3))),
                            ("hookState", int(2)),
                            ("hookLength", num(12.0)),
                            ("hookHeld", boolean(true)),
                            ("regenFrame", int(100)),
                            ("killCarrierTime", num(1.0)),
                            ("hitCarrierTime", num(2.0)),
                            ("returnFlagTime", num(3.0)),
                            ("defendFlagTime", num(4.0)),
                            ("extraFlags", int(64)),
                            ("spawnState", int(1)),
                            (
                                "statistics",
                                arr(vec![obj(vec![("key", str("score")), ("count", num(5.0))])]),
                            ),
                        ]),
                    ),
                ])]),
            ),
        ]));
        let value = decode_checkpoint_value(&payload).unwrap();
        let checkpoint = read_q2_lmctf_checkpoint(SaveReader::at(&value, "q2-lmctf")).unwrap();
        assert_eq!(checkpoint.rules.frag_limit, 20);
        assert_eq!(checkpoint.rules.map_list, vec!["q2dm1".to_string()]);
        assert_eq!(checkpoint.match_state.phase, LmctfMatchPhase::Countdown);
        assert_eq!(checkpoint.match_state.remaining, 9);
        let pending = checkpoint.match_state.pending_map.as_ref().unwrap();
        assert_eq!(pending.map, "q2dm2");
        assert!(pending.countdown);
        assert_eq!(checkpoint.vote.started_at, Some(50.0));
        assert!(checkpoint.plasma_quad);
        assert_eq!(checkpoint.flags.flags[0].team, 1);
        assert_eq!(checkpoint.flags.flags[0].actor, actor(1));
        assert!(!checkpoint.runes.forward);
        let player = &checkpoint.players[0];
        assert!(player.plasma_mode);
        assert_eq!(player.team, 1);
        assert_eq!(player.regen_frame, 100);
        assert_eq!(player.extra_flags, 64);
        assert_eq!(player.statistics, vec![("score".to_string(), 5.0)]);
        assert_eq!(player.grapple.hook, Some(actor(3)));
        assert_eq!(player.grapple.hook_state, 2);
        assert!(player.grapple.hook_held);
    }

    const PROVIDER: &str = "q2:game";

    fn record(schema: &str, bytes: Vec<u8>) -> qa_world::save::ownership::ProviderCheckpoint {
        qa_world::save::ownership::ProviderCheckpoint {
            provider: PROVIDER.to_string(),
            schema: schema.to_string(),
            version: 1,
            bytes,
        }
    }

    fn composition_json(kind: &str) -> SaveJson {
        obj(vec![
            ("edition", str("classic")),
            ("program", str("baseq2")),
            ("match", obj(vec![("kind", str(kind))])),
            ("deathmatchFlags", int(0)),
        ])
    }

    fn minimal_module_records() -> Vec<qa_world::save::ownership::ProviderCheckpoint> {
        use crate::persistence::q2::base_entities::Q2BaseEntitiesCheckpoint;
        use crate::persistence::q2::items::Q2ItemsCheckpoint;
        use crate::persistence::q2::monsters::{Q2MonsterPerception, Q2MonstersCheckpoint};
        use crate::persistence::q2::movers::Q2MoversCheckpoint;
        use crate::persistence::q2::players::{Q2PlayerIntermission, Q2PlayersCheckpoint};
        use crate::persistence::q2::weapons::Q2WeaponsCheckpoint;
        vec![
            record(
                "q2:items",
                crate::persistence::q2::items::encode_q2_items_checkpoint(&Q2ItemsCheckpoint {
                    power_cube_count: 0.0,
                    pickups: Vec::new(),
                    powers: Vec::new(),
                    power_armor_bindings: Vec::new(),
                }),
            ),
            record(
                "q2:movers",
                crate::persistence::q2::movers::encode_q2_movers_checkpoint(&Q2MoversCheckpoint {
                    doors: Vec::new(),
                    trains: Vec::new(),
                    linear: Vec::new(),
                    angular: Vec::new(),
                }),
            ),
            record(
                "q2:monsters",
                crate::persistence::q2::monsters::encode_q2_monsters_checkpoint(&Q2MonstersCheckpoint {
                    actors: Vec::new(),
                    perception: Q2MonsterPerception::default(),
                }),
            ),
            record(
                "q2:weapons",
                crate::persistence::q2::weapons::encode_q2_weapons_checkpoint(&Q2WeaponsCheckpoint {
                    silencer_charges: Vec::new(),
                    source_rules: "base".to_string(),
                    registered: Vec::new(),
                    fallback_order: None,
                    states: Vec::new(),
                    inputs: Vec::new(),
                    noises: Vec::new(),
                    sound_entity: None,
                    sound2_entity: None,
                    blaster_causes: Vec::new(),
                }),
            ),
            record(
                "q2:players",
                crate::persistence::q2::players::encode_q2_players_checkpoint(&Q2PlayersCheckpoint {
                    corpse_index: 0,
                    death_animation: 0.0,
                    pain_animation: 0.0,
                    rules: sample_player_rules(),
                    intermission: Q2PlayerIntermission::Playing,
                    players: Vec::new(),
                }),
            ),
            record(
                "q2:base-entities",
                crate::persistence::q2::base_entities::encode_q2_base_entities_checkpoint(&Q2BaseEntitiesCheckpoint {
                    platforms: Vec::new(),
                    secrets: Vec::new(),
                    linear: Vec::new(),
                    animations: Vec::new(),
                    clocks: Vec::new(),
                    breaches: Vec::new(),
                    drivers: Vec::new(),
                    wind_times: Vec::new(),
                }),
            ),
        ]
    }

    fn image_with(records: Vec<qa_world::save::ownership::ProviderCheckpoint>) -> SimulationSaveImage {
        use super::super::save::{SimSaveClock, SimSaveRandom, SimSavedRecipe};
        let frame_time = num(1.0);
        SimulationSaveImage {
            providers: records,
            recipe: SimSavedRecipe {
                execution: Vec::new(),
                map_entities_provider: PROVIDER.to_string(),
                map_geometry_path: "maps/q2dm1.bsp".to_string(),
            },
            guests: Vec::new(),
            clocks: vec![SimSaveClock {
                provider: PROVIDER.to_string(),
                time: frame_time.clone(),
            }],
            random: vec![SimSaveRandom {
                provider: PROVIDER.to_string(),
                state: qa_world::save::shared::SaveRandomState::GlibcRandom {
                    words: Vec::new(),
                    front: 0,
                    rear: 0,
                    draws: 0,
                },
            }],
            bodies: Vec::new(),
            combat: Vec::new(),
            inventories: Vec::new(),
            configurations: Vec::new(),
            thinks: Vec::new(),
            frame_time,
            mods: None,
            schema_version: 3,
            legacy_armor_layout: false,
            frame: qa_core::time::FrameContext {
                frame: 0,
                time: qa_core::time::SourceTime::Seconds(0.0),
                elapsed: qa_core::time::SourceTime::Seconds(0.0),
                phase: qa_core::time::FramePhase::FrameEntry,
            },
            next_event_sequence: 0,
            actors: Vec::new(),
        }
    }

    fn sample_foundation() -> ContentFoundationCheckpoint {
        ContentFoundationCheckpoint {
            version: 1,
            next_source_slot: 0,
            sequence: 0,
            freed_slots: Vec::new(),
            counters: qa_content::q2::foundation::host::Q2Counters {
                total_secrets: 0,
                found_secrets: 0,
                total_goals: 0,
                found_goals: 0,
                total_monsters: 0,
                killed_monsters: 0,
                server_flags: 0,
            },
            entities: Vec::new(),
        }
    }

    fn standard_config() -> Q2ProductRestoreConfig {
        Q2ProductRestoreConfig {
            program: "baseq2".to_string(),
            selection: Q2MatchSelection::Standard,
            has_armory: false,
            expansions: Vec::new(),
            has_rerelease: false,
        }
    }

    #[test]
    fn product_standard_round_trip() {
        let mut records = vec![record(
            "q2:composition",
            encode_checkpoint_value(&composition_json("standard")),
        )];
        records.extend(minimal_module_records());
        let image = image_with(records);
        let checkpoint =
            read_q2_product_checkpoint(&image, &standard_config(), Q2Edition::Classic, sample_foundation()).unwrap();
        assert_eq!(checkpoint.edition, Q2Edition::Classic);
        assert_eq!(checkpoint.program, "baseq2");
        assert_eq!(checkpoint.match_selection, Q2MatchSelection::Standard);
        assert_eq!(checkpoint.deathmatch_flags, 0);
        assert_eq!(checkpoint.foundation.version, 1);
        assert!(checkpoint.items.pickups.is_empty());
        assert!(checkpoint.movers.doors.is_empty());
        assert!(checkpoint.monsters.actors.is_empty());
        assert!(checkpoint.weapons.states.is_empty());
        assert!(checkpoint.players.players.is_empty());
        assert!(checkpoint.base_entities.wind_times.is_empty());
        assert!(checkpoint.missionpack_items.is_none());
        assert!(checkpoint.expansions.is_empty());
        assert!(checkpoint.rerelease.is_none());
        assert!(checkpoint.match_state.is_none());
    }

    #[test]
    fn product_identity_guards() {
        let config = standard_config();
        let foundation = || sample_foundation();
        let build = |composition: SaveJson| {
            let mut records = vec![record("q2:composition", encode_checkpoint_value(&composition))];
            records.extend(minimal_module_records());
            image_with(records)
        };
        let bad_edition = obj(vec![
            ("edition", str("rerelease")),
            ("program", str("baseq2")),
            ("match", obj(vec![("kind", str("standard"))])),
            ("deathmatchFlags", int(0)),
        ]);
        let error =
            read_q2_product_checkpoint(&build(bad_edition), &config, Q2Edition::Classic, foundation()).unwrap_err();
        assert!(error.to_string().contains("different source product"));
        let bad_program = obj(vec![
            ("edition", str("classic")),
            ("program", str("rogue")),
            ("match", obj(vec![("kind", str("standard"))])),
            ("deathmatchFlags", int(0)),
        ]);
        let error =
            read_q2_product_checkpoint(&build(bad_program), &config, Q2Edition::Classic, foundation()).unwrap_err();
        assert!(error.to_string().contains("different source product"));
        let bad_kind = obj(vec![
            ("edition", str("classic")),
            ("program", str("baseq2")),
            ("match", obj(vec![("kind", str("ctf"))])),
            ("deathmatchFlags", int(0)),
        ]);
        let error =
            read_q2_product_checkpoint(&build(bad_kind), &config, Q2Edition::Classic, foundation()).unwrap_err();
        assert!(error.to_string().contains("different source match mode"));
        let deathball_config = Q2ProductRestoreConfig {
            selection: Q2MatchSelection::Deathball {
                team1_skin: "male/ctf_r".to_string(),
                team2_skin: "male/ctf_b".to_string(),
                goal_limit: 5.0,
            },
            ..standard_config()
        };
        let bad_ball = obj(vec![
            ("edition", str("classic")),
            ("program", str("baseq2")),
            (
                "match",
                obj(vec![
                    ("kind", str("deathball")),
                    ("team1Skin", str("male/ctf_r")),
                    ("team2Skin", str("female/ctf_b")),
                    ("goalLimit", num(5.0)),
                ]),
            ),
            ("deathmatchFlags", int(0)),
        ]);
        let error = read_q2_product_checkpoint(&build(bad_ball), &deathball_config, Q2Edition::Classic, foundation())
            .unwrap_err();
        assert!(error.to_string().contains("different DeathBall rules"));
        let missing = image_with(vec![record(
            "q2:composition",
            encode_checkpoint_value(&composition_json("standard")),
        )]);
        assert!(read_q2_product_checkpoint(&missing, &config, Q2Edition::Classic, foundation()).is_err());
    }

    #[test]
    fn product_optionals_round_trip() {
        use crate::persistence::q2::missionpacks::{
            Q2MissionPackItemsCheckpoint, Q2MissionPackMonstersCheckpoint, Q2RogueEntitiesCheckpoint, Q2TagCheckpoint,
        };
        let config = Q2ProductRestoreConfig {
            program: "baseq2".to_string(),
            selection: Q2MatchSelection::Tag,
            has_armory: true,
            expansions: vec![Q2MissionPack::Rogue],
            has_rerelease: false,
        };
        let mut records = vec![record(
            "q2:composition",
            encode_checkpoint_value(&composition_json("tag")),
        )];
        records.extend(minimal_module_records());
        records.push(record(
            "q2:missionpack-items",
            crate::persistence::q2::missionpacks::encode_q2_mission_pack_items_checkpoint(
                &Q2MissionPackItemsCheckpoint { powers: Vec::new() },
            ),
        ));
        records.push(record(
            "q2:rogue-monsters",
            crate::persistence::q2::missionpacks::encode_q2_mission_pack_monsters_checkpoint(
                &Q2MissionPackMonstersCheckpoint {
                    flyer_next_move: "none".to_string(),
                    hints: None,
                    widow_shots_fired: 0.0,
                    widow_damage_multiplier: 1,
                    actors: Vec::new(),
                },
            ),
        ));
        records.push(record(
            "q2:rogue-entities",
            crate::persistence::q2::missionpacks::encode_q2_rogue_entities_checkpoint(&Q2RogueEntitiesCheckpoint {
                steam_id: 3.0,
            }),
        ));
        records.push(record(
            "q2:match",
            crate::persistence::q2::missionpacks::encode_q2_tag_checkpoint(&Q2TagCheckpoint {
                token: Some(actor(1)),
                owner: None,
                count: 2.0,
            }),
        ));
        let checkpoint =
            read_q2_product_checkpoint(&image_with(records), &config, Q2Edition::Classic, sample_foundation()).unwrap();
        assert!(checkpoint.missionpack_items.is_some());
        assert_eq!(checkpoint.expansions.len(), 1);
        assert_eq!(checkpoint.expansions[0].pack, Q2MissionPack::Rogue);
        assert_eq!(checkpoint.expansions[0].entities.steam_id, 3);
        match checkpoint.match_state.as_ref().unwrap() {
            Q2ProductMatchCheckpoint::Tag(tag) => {
                assert_eq!(tag.token, Some(actor(1)));
                assert_eq!(tag.count, 2);
            }
            _ => panic!("expected tag match state"),
        }

        let mut missing = vec![record(
            "q2:composition",
            encode_checkpoint_value(&composition_json("tag")),
        )];
        missing.extend(minimal_module_records());
        assert!(
            read_q2_product_checkpoint(&image_with(missing), &config, Q2Edition::Classic, sample_foundation()).is_err()
        );
    }
}
