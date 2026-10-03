//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/runtime.ts`
//!
//! Q1 persistence-to-content checkpoint bridge.
//!
//! Converts the save-side
//! [`Q1FoundationCheckpoint`](crate::persistence::q1::foundation::Q1FoundationCheckpoint)
//! (string numerics, tuple rows) into the content
//! [`Q1FoundationCheckpoint`](qa_content::q1::foundation::checkpoint::Q1FoundationCheckpoint)
//! (typed enums, `i32`/`u32` counters) that
//! [`Q1EntityServices::restore`](qa_content::q1::foundation::entity_services::Q1EntityServices::restore)
//! consumes. Every mapping is total over reader-validated saves: the
//! persistence reader already narrows enums to `choice_str` sets and the
//! bridge re-parses them into content enums, failing closed with
//! [`WorldError::BadSave`](qa_world::WorldError::BadSave) otherwise.

use qa_content::q1::foundation::checkpoint::{
    Q1EntitySourceState as ContentSourceState, Q1FoundationCheckpoint as ContentCheckpoint,
    Q1SavedCallbacks as ContentCallbacks, Q1SavedEntity as ContentEntity, Q1SavedExtension as ContentExtension,
    Q1SavedField as ContentField, Q1SavedIntermission as ContentIntermission, Q1SavedMonster as ContentMonster,
    Q1SavedMove as ContentMove, Q1SavedPlayer as ContentPlayer, Q1SavedPlayerState as ContentPlayerState,
    Q1SavedPowerup as ContentPowerup, Q1SavedReference as ContentReference,
};
use qa_content::q1::foundation::entity::{
    Q1AttackState, Q1MonsterMode, Q1MonsterSpecies, Q1MoverState, Q1ProjectileKind,
};
use qa_content::q1::foundation::types::{
    Q1AutoSwitch, Q1Basis, Q1Edition, Q1MoveType, Q1Powerup, Q1PrecachePhase, Q1PrecacheTables, Q1Solid, Q1Weapon,
};
use qa_core::identity::ProviderId;
use qa_world::WorldError;

use crate::persistence::q1::foundation::{
    Q1EntityCallbacks as SavedCallbacks, Q1EntitySourceState as SavedSourceState,
    Q1FoundationCheckpoint as SavedCheckpoint, Q1Intermission as SavedIntermission, Q1MonsterState as SavedMonster,
    Q1PlayerState as SavedPlayerState, Q1SavedEntity as SavedEntity, Q1SavedPlayer as SavedPlayer,
};

/// Convert a persistence Q1 foundation checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_lines)]
pub fn convert_persistence_q1_checkpoint(checkpoint: &SavedCheckpoint) -> Result<ContentCheckpoint, WorldError> {
    Ok(ContentCheckpoint {
        format: "q1-foundation".to_string(),
        provider: convert_provider(&checkpoint.provider)?,
        version: 5,
        precaches: Q1PrecacheTables {
            phase: match checkpoint.precache_phase.as_str() {
                "loading" => Q1PrecachePhase::Loading,
                "frozen" => Q1PrecachePhase::Frozen,
                other => return Err(WorldError::BadSave(format!("unknown Q1 precache phase {other:?}"))),
            },
            models: checkpoint.precache_models.clone(),
            sounds: checkpoint.precache_sounds.clone(),
        },
        edition: match checkpoint.edition.as_str() {
            "classic" => Q1Edition::Classic,
            "rerelease" => Q1Edition::Rerelease,
            other => return Err(WorldError::BadSave(format!("unknown Q1 edition {other:?}"))),
        },
        time: checkpoint.time,
        frame_seconds: checkpoint.frame_seconds,
        force_retouch: checkpoint.force_retouch as i32,
        basis: Q1Basis {
            forward: checkpoint.basis.0,
            right: checkpoint.basis.1,
            up: checkpoint.basis.2,
        },
        sequence: checkpoint.sequence,
        next_dynamic_slot: u32::try_from(checkpoint.next_dynamic_slot)
            .map_err(|_| WorldError::BadSave("Q1 next dynamic slot is out of range".to_string()))?,
        total_secrets: checkpoint.total_secrets as i32,
        found_secrets: checkpoint.found_secrets as i32,
        total_monsters: checkpoint.total_monsters as i32,
        killed_monsters: checkpoint.killed_monsters as i32,
        world_type: checkpoint.world_type as i32,
        map_name: checkpoint.map_name.clone(),
        world: checkpoint.world,
        sight_entity: checkpoint.sight_entity,
        sight_time: checkpoint.sight_time,
        intermission: checkpoint.intermission.as_ref().map(convert_intermission),
        entities: checkpoint
            .entities
            .iter()
            .map(convert_entity)
            .collect::<Result<Vec<_>, _>>()?,
        players: checkpoint
            .players
            .iter()
            .map(convert_player)
            .collect::<Result<Vec<_>, _>>()?,
        extensions: checkpoint
            .extensions
            .iter()
            .map(|(id, bytes)| ContentExtension {
                id: id.clone(),
                bytes: bytes.clone(),
            })
            .collect(),
    })
}

/// Convert a `namespace:name` provider reference.
fn convert_provider(text: &str) -> Result<ProviderId, WorldError> {
    text.split_once(':')
        .map(|(namespace, name)| ProviderId::new(namespace, name))
        .ok_or_else(|| WorldError::BadSave(format!("expected namespace:name, found {text:?}")))
}

/// Convert a saved intermission record.
fn convert_intermission(saved: &SavedIntermission) -> ContentIntermission {
    ContentIntermission {
        map: saved.map.clone(),
        cause: saved.cause,
        exit_after: saved.exit_after,
    }
}

/// Convert a saved entity record.
#[allow(clippy::cast_possible_truncation)]
fn convert_entity(saved: &SavedEntity) -> Result<ContentEntity, WorldError> {
    Ok(ContentEntity {
        actor: saved.actor,
        source_slot: saved
            .source_slot
            .map(|slot| {
                u32::try_from(slot).map_err(|_| WorldError::BadSave("Q1 source slot is out of range".to_string()))
            })
            .transpose()?,
        actor_provider: convert_provider(&saved.actor_provider)?,
        classname: saved.classname.clone(),
        source_ordinal: saved
            .source_ordinal
            .map(|ordinal| {
                i32::try_from(ordinal).map_err(|_| WorldError::BadSave("Q1 source ordinal is out of range".to_string()))
            })
            .transpose()?,
        state: convert_source_state(&saved.state)?,
        fields: saved
            .fields
            .iter()
            .map(|(key, value)| ContentField {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        references: saved
            .references
            .iter()
            .map(|(key, actor)| ContentReference {
                key: key.clone(),
                actor: *actor,
            })
            .collect(),
        owner: saved.owner,
        activator: saved.activator,
        door_group: saved.door_group.clone(),
        monster: saved.monster.as_ref().map(convert_monster).transpose()?,
        move_completion: saved.move_target.as_ref().map(|(destination, done)| ContentMove {
            destination: *destination,
            done: done.clone(),
        }),
        callbacks: convert_callbacks(&saved.callbacks),
    })
}

/// Convert saved entity source state.
#[allow(clippy::cast_possible_truncation)]
fn convert_source_state(saved: &SavedSourceState) -> Result<ContentSourceState, WorldError> {
    Ok(ContentSourceState {
        model: saved.model.clone(),
        frame: saved.frame as i32,
        skin: saved.skin as i32,
        effects: saved.effects as i32,
        solid: Q1Solid::parse(&saved.solid).map_err(|error| WorldError::BadSave(error.to_string()))?,
        movement: Q1MoveType::parse(&saved.movement).map_err(|error| WorldError::BadSave(error.to_string()))?,
        target: saved.target.clone(),
        targetname: saved.targetname.clone(),
        killtarget: saved.killtarget.clone(),
        message: saved.message.clone(),
        delay: saved.delay,
        spawnflags: saved.spawnflags as i32,
        sounds: saved.sounds as i32,
        wait: saved.wait,
        speed: saved.speed,
        damage: saved.damage,
        max_health: saved.max_health,
        aimed_damage: saved.aimed_damage,
        next_think: saved.next_think,
        original_model: saved.original_model.clone(),
        pos1: saved.pos1,
        pos2: saved.pos2,
        dest1: saved.dest1,
        dest2: saved.dest2,
        movedir: saved.movedir,
        mangle: saved.mangle,
        state: Q1MoverState::parse(&saved.state).map_err(|error| WorldError::BadSave(error.to_string()))?,
        trigger_bounds: saved.trigger_bounds,
        attack_finished: saved.attack_finished,
        count: saved.count,
        activated: saved.activated,
        projectile: saved
            .projectile
            .as_deref()
            .map(Q1ProjectileKind::parse)
            .transpose()
            .map_err(|error| WorldError::BadSave(error.to_string()))?,
        projectile_weapon: saved
            .projectile_weapon
            .as_deref()
            .map(Q1Weapon::parse)
            .transpose()
            .map_err(|error| WorldError::BadSave(error.to_string()))?,
        angular_velocity: saved.angular_velocity,
        water_level: saved.water_level as i32,
        water_type: i32::try_from(saved.water_type)
            .map_err(|_| WorldError::BadSave("Q1 water type is out of range".to_string()))?,
        movement_flags: saved.movement_flags as i32,
        ideal_yaw: saved.ideal_yaw,
        yaw_speed: saved.yaw_speed,
        attack_state: Q1AttackState::parse(&saved.attack_state)
            .map_err(|error| WorldError::BadSave(error.to_string()))?,
    })
}

/// Convert a saved monster record.
#[allow(clippy::cast_possible_truncation)]
fn convert_monster(saved: &SavedMonster) -> Result<ContentMonster, WorldError> {
    Ok(ContentMonster {
        species: Q1MonsterSpecies::parse(&saved.species).map_err(|error| WorldError::BadSave(error.to_string()))?,
        mode: Q1MonsterMode::parse(&saved.mode).map_err(|error| WorldError::BadSave(error.to_string()))?,
        frame_index: saved.frame_index as usize,
        sequence: saved.sequence.clone(),
        first_frame: saved.first_frame as i32,
        enemy: saved.enemy,
        old_enemy: saved.old_enemy,
        path: saved.path.clone(),
        pause_until: saved.pause_until,
        attack_finished: saved.attack_finished,
        pain_finished: saved.pain_finished,
        search_until: saved.search_until,
        death_drop: saved.death_drop,
        refired: saved.refired,
    })
}

/// Convert saved callback names.
fn convert_callbacks(saved: &SavedCallbacks) -> ContentCallbacks {
    ContentCallbacks {
        think: saved.think.clone(),
        use_callback: saved.use_callback.clone(),
        touch: saved.touch.clone(),
        pain: saved.pain.clone(),
        die: saved.die.clone(),
        blocked: saved.blocked.clone(),
        path_end: saved.path_end.clone(),
    }
}

/// Convert a saved player record.
#[allow(clippy::cast_possible_truncation)]
fn convert_player(saved: &SavedPlayer) -> Result<ContentPlayer, WorldError> {
    Ok(ContentPlayer {
        actor_provider: convert_provider(&saved.actor_provider)?,
        actor: saved.actor,
        state: convert_player_state(&saved.state)?,
        powerups: saved
            .powerups
            .iter()
            .map(|(kind, expires)| {
                Ok(ContentPowerup {
                    kind: Q1Powerup::parse(kind).map_err(|error| WorldError::BadSave(error.to_string()))?,
                    expires: *expires,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?,
    })
}

/// Convert saved player arsenal state.
#[allow(clippy::cast_possible_truncation)]
fn convert_player_state(saved: &SavedPlayerState) -> Result<ContentPlayerState, WorldError> {
    Ok(ContentPlayerState {
        alpha: Some(saved.alpha),
        scale: Some(saved.scale),
        weapon: Q1Weapon::parse(&saved.weapon).map_err(|error| WorldError::BadSave(error.to_string()))?,
        primary_holstered: saved.primary_holstered,
        attack_finished: saved.attack_finished,
        attack_held: saved.attack_held,
        jump_held: saved.jump_held,
        teleport_until: saved.teleport_until,
        weapon_frame: saved.weapon_frame as i32,
        weapon_animation_at: saved.weapon_animation_at,
        weapon_animation_base: saved.weapon_animation_base as i32,
        continuous_firing: saved.continuous_firing,
        next_weapon_frame: saved.next_weapon_frame,
        lightning_sound_at: saved.lightning_sound_at,
        punch_angles: saved.punch_angles,
        nail_side: saved.nail_side,
        max_health: saved.max_health,
        mega_rot_at: saved.mega_rot_at,
        hostile_until: saved.hostile_until,
        view_angles: saved.view_angles,
        water_level: saved.water_level as i32,
        air_finished: saved.air_finished,
        drown_damage: saved.drown_damage,
        drown_at: saved.drown_at,
        hazard_at: saved.hazard_at,
        auto_switch: match saved.auto_switch.as_str() {
            "always" => Q1AutoSwitch::Always,
            "new" => Q1AutoSwitch::New,
            "never" => Q1AutoSwitch::Never,
            other => return Err(WorldError::BadSave(format!("unknown Q1 auto-switch {other:?}"))),
        },
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::SavedActorId;
    use qa_core::math::Vec3;

    use super::*;
    use crate::persistence::q1::foundation::{Q1EntityCallbacks, Q1MonsterState, Q1PlayerState as SavedPlayerState};

    fn vec() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    fn sample_state() -> SavedSourceState {
        SavedSourceState {
            model: "progs/ogre.mdl".to_string(),
            frame: 1.0,
            skin: 0.0,
            effects: 0.0,
            solid: "bbox".to_string(),
            movement: "step".to_string(),
            target: String::new(),
            targetname: String::new(),
            killtarget: String::new(),
            message: String::new(),
            delay: 0.0,
            spawnflags: 0.0,
            sounds: 0.0,
            wait: 0.0,
            speed: 0.0,
            damage: 0.0,
            max_health: 200.0,
            aimed_damage: false,
            next_think: 0.0,
            original_model: String::new(),
            pos1: vec(),
            pos2: vec(),
            dest1: vec(),
            dest2: vec(),
            movedir: vec(),
            mangle: vec(),
            state: "bottom".to_string(),
            trigger_bounds: None,
            attack_finished: 0.0,
            count: 0.0,
            activated: false,
            projectile: None,
            projectile_weapon: None,
            angular_velocity: vec(),
            water_level: 0.0,
            water_type: 0,
            movement_flags: 0.0,
            ideal_yaw: 0.0,
            yaw_speed: 0.0,
            attack_state: "straight".to_string(),
        }
    }

    fn sample_player_state() -> SavedPlayerState {
        SavedPlayerState {
            alpha: 1.0,
            scale: 2.0,
            weapon: "shotgun".to_string(),
            primary_holstered: false,
            attack_held: false,
            jump_held: false,
            teleport_until: 0.0,
            attack_finished: 0.0,
            weapon_frame: 3.0,
            weapon_animation_at: 0.0,
            weapon_animation_base: 4.0,
            continuous_firing: false,
            next_weapon_frame: 0.0,
            lightning_sound_at: 0.0,
            punch_angles: vec(),
            nail_side: 0.0,
            max_health: 100.0,
            mega_rot_at: 0.0,
            hostile_until: 0.0,
            view_angles: vec(),
            water_level: 0.0,
            air_finished: 0.0,
            drown_damage: 0.0,
            drown_at: 0.0,
            hazard_at: 0.0,
            auto_switch: "new".to_string(),
        }
    }

    fn sample() -> SavedCheckpoint {
        SavedCheckpoint {
            provider: "q1:game".to_string(),
            precache_phase: "frozen".to_string(),
            precache_models: vec!["progs/ogre.mdl".to_string()],
            precache_sounds: Vec::new(),
            edition: "classic".to_string(),
            time: 10.0,
            frame_seconds: 0.1,
            force_retouch: 2.0,
            sequence: 4,
            next_dynamic_slot: 8,
            total_secrets: 3.0,
            found_secrets: 1.0,
            total_monsters: 12.0,
            killed_monsters: 2.0,
            world_type: 0.0,
            map_name: "e1m1".to_string(),
            basis: (vec(), vec(), vec()),
            world: Some(SavedActorId { slot: 0, generation: 0 }),
            sight_entity: None,
            sight_time: 0.0,
            intermission: None,
            entities: vec![SavedEntity {
                actor: SavedActorId { slot: 1, generation: 0 },
                actor_provider: "q1:game".to_string(),
                source_slot: Some(1),
                classname: "monster_ogre".to_string(),
                source_ordinal: Some(0),
                state: sample_state(),
                fields: vec![("targetname".to_string(), "ogre1".to_string())],
                references: vec![("enemy".to_string(), None)],
                owner: None,
                activator: None,
                door_group: Vec::new(),
                monster: Some(Q1MonsterState {
                    species: "ogre".to_string(),
                    mode: "stand".to_string(),
                    frame_index: 0.0,
                    sequence: Vec::new(),
                    first_frame: 0.0,
                    enemy: None,
                    old_enemy: None,
                    path: String::new(),
                    pause_until: 0.0,
                    attack_finished: 0.0,
                    pain_finished: 0.0,
                    search_until: 0.0,
                    death_drop: false,
                    refired: false,
                }),
                move_target: Some((vec(), "done".to_string())),
                callbacks: Q1EntityCallbacks {
                    think: Some("think".to_string()),
                    ..Default::default()
                },
            }],
            players: vec![SavedPlayer {
                actor: SavedActorId { slot: 2, generation: 0 },
                actor_provider: "q1:game".to_string(),
                state: sample_player_state(),
                powerups: vec![("quad".to_string(), 9.0)],
            }],
            extensions: vec![("ext".to_string(), vec![1, 2])],
        }
    }

    #[test]
    fn converts_top_level_shape() {
        let converted = convert_persistence_q1_checkpoint(&sample()).expect("convert");
        assert_eq!(converted.format, "q1-foundation");
        assert_eq!(converted.version, 5);
        assert_eq!(converted.provider.namespace, "q1");
        assert_eq!(converted.provider.name, "game");
        assert_eq!(converted.edition, Q1Edition::Classic);
        assert_eq!(converted.precaches.phase, Q1PrecachePhase::Frozen);
        assert_eq!(converted.precaches.models, vec!["progs/ogre.mdl".to_string()]);
        assert_eq!(converted.force_retouch, 2);
        assert_eq!(converted.next_dynamic_slot, 8);
        assert_eq!(converted.total_secrets, 3);
        assert_eq!(converted.extensions.len(), 1);
        assert_eq!(converted.world, Some(SavedActorId { slot: 0, generation: 0 }));
    }

    #[test]
    fn converts_entity_and_player_rows() {
        use qa_content::q1::foundation::entity::{Q1MonsterMode, Q1MonsterSpecies};

        let converted = convert_persistence_q1_checkpoint(&sample()).expect("convert");
        let entity = converted.entities.first().expect("entity");
        assert_eq!(entity.state.frame, 1);
        assert_eq!(entity.state.solid, Q1Solid::Bbox);
        assert_eq!(entity.state.movement, Q1MoveType::Step);
        assert_eq!(entity.state.attack_state, Q1AttackState::Straight);
        assert_eq!(entity.fields.len(), 1);
        assert_eq!(entity.references.len(), 1);
        let monster = entity.monster.as_ref().expect("monster");
        assert_eq!(monster.species, Q1MonsterSpecies::Ogre);
        assert_eq!(monster.mode, Q1MonsterMode::Stand);
        let done = entity.move_completion.as_ref().expect("move");
        assert_eq!(done.done, "done");
        assert_eq!(entity.callbacks.think.as_deref(), Some("think"));
        let player = converted.players.first().expect("player");
        assert_eq!(player.state.alpha, Some(1.0));
        assert_eq!(player.state.weapon, Q1Weapon::Shotgun);
        assert_eq!(player.state.weapon_frame, 3);
        assert_eq!(player.state.auto_switch, Q1AutoSwitch::New);
        assert_eq!(player.powerups.len(), 1);
        assert_eq!(player.powerups[0].kind, Q1Powerup::Quad);
    }

    #[test]
    fn rejects_unknown_enums_and_overflow() {
        let mut bad = sample();
        bad.edition = "x".to_string();
        assert!(convert_persistence_q1_checkpoint(&bad).is_err());
        let mut bad = sample();
        bad.next_dynamic_slot = u64::MAX;
        assert!(convert_persistence_q1_checkpoint(&bad).is_err());
        let mut bad = sample();
        bad.players[0].state.weapon = "x".to_string();
        assert!(convert_persistence_q1_checkpoint(&bad).is_err());
    }
}
