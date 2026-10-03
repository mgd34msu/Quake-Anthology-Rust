//! Q2 persistence-to-content character checkpoint bridge.
//!
//! Converts the save-side
//! [`Q2CharacterCheckpoint`](crate::persistence::q2::players::Q2CharacterCheckpoint)
//! (flat numerics, `qa-world` inventory/armor twins) into the content
//! [`Q2CharacterCheckpoint`](qa_content::q2::base::player::checkpoint::Q2CharacterCheckpoint)
//! that
//! [`Q2CharacterActor::restore`](qa_content::q2::base::player::character::Q2CharacterActor::restore)
//! consumes. The embedded state's chase target travels alongside as a
//! saved id (content checkpoint contract); the caller resolves it
//! through the actor registry.

use qa_content::contract::{
    ArmorState as ContentArmor, InventoryCountPolicy as ContentCountPolicy, InventoryEntry as ContentEntry,
    PoweredProtectionState as ContentPowered, RegularArmorState as ContentRegular,
    SourceCounterArithmetic as ContentArithmetic,
};
use qa_content::q2::base::player::checkpoint::{
    Q2CharacterCheckpoint as ContentCheckpoint, Q2CharacterEntityFields as ContentEntity,
    Q2PlayerStateCheckpoint as ContentState,
};
use qa_content::q2::base::player::types::{
    Q2PlayerCarry as ContentCarry, Q2PlayerGender, Q2PlayerHand, Q2PlayerRules as ContentRules,
    Q2PlayerState as ContentPlayerState,
};
use qa_content::q2::foundation::checkpoint::Q2AttackCheckpoint as ContentAttack;
use qa_content::q2::support::contracts::{
    AttackCause as ContentCause, AttackProvenance as ContentProvenance, EnvironmentHazard as ContentHazard,
    Q1ArmorEffect as ContentArmorEffect, Q2NativeCause as ContentNative, Q2NativeGame as ContentNativeGame,
};
use qa_core::identity::ProviderId;
use qa_world::combat::{ArmorState as SavedArmor, PoweredProtection as SavedPowered, RegularArmor as SavedRegular};
use qa_world::inventory::{
    CountArithmetic as SavedArithmetic, CountPolicy as SavedPolicy, InventoryEntry as SavedEntry,
};
use qa_world::WorldError;

use crate::persistence::q2::foundation::{Q2AttackCause as SavedCause, Q2NativeCause as SavedNative};
use crate::persistence::q2::players::{
    Q2CharacterCheckpoint as SavedCheckpoint, Q2CharacterEntity as SavedEntity, Q2PlayerCarry as SavedCarry,
    Q2PlayerRules as SavedRules, Q2PlayerStateCheckpoint as SavedState,
};

/// Convert a persistence Q2 character checkpoint into the content shape.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_lines)]
pub fn convert_persistence_q2_character(checkpoint: &SavedCheckpoint) -> Result<ContentCheckpoint, WorldError> {
    Ok(ContentCheckpoint {
        version: 1,
        pain_index: checkpoint.pain_index as i32,
        death_index: checkpoint.death_index as i32,
        state: convert_state(&checkpoint.state)?,
        rules: convert_rules(&checkpoint.rules),
        entity: convert_entity(&checkpoint.entity),
        last_attack: checkpoint.last_attack.as_ref().map(convert_attack).transpose()?,
    })
}

/// Convert a persistence player state checkpoint.
#[allow(clippy::cast_possible_truncation)]
fn convert_state(saved: &SavedState) -> Result<ContentState, WorldError> {
    Ok(ContentState {
        state: ContentPlayerState {
            slot: i32::try_from(saved.slot)
                .map_err(|_| WorldError::BadSave("Q2 player slot is out of range".to_string()))?,
            entered_at: saved.entered_at,
            use_q2_weapons: saved.use_q2_weapons,
            use_q2_inventory: saved.use_q2_inventory,
            spawn_inventory: saved.spawn_inventory.iter().map(convert_entry).collect(),
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
            coop_respawn: saved.coop_respawn.as_ref().map(convert_carry),
            flood_times: saved.flood_times.clone(),
            flood_lock_until: saved.flood_lock_until,
        },
        chase_target: saved.chase_target,
    })
}

/// Convert a persistence coop-respawn carry record.
#[allow(clippy::cast_possible_truncation)]
fn convert_carry(saved: &SavedCarry) -> ContentCarry {
    ContentCarry {
        health: saved.health,
        maximum_health: saved.maximum_health,
        armor: convert_armor(&saved.armor),
        inventory: saved.inventory.iter().map(convert_entry).collect(),
        weapon: saved.weapon.clone(),
        selected_item: saved.selected_item.clone(),
        score: saved.score as i32,
        flags: saved.flags as i64,
        power_cubes: saved.power_cubes as i32,
    }
}

/// Convert persistence player rules.
#[allow(clippy::cast_possible_truncation)]
fn convert_rules(saved: &SavedRules) -> ContentRules {
    ContentRules {
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

/// Convert persistence character entity fields.
#[allow(clippy::cast_possible_truncation)]
fn convert_entity(saved: &SavedEntity) -> ContentEntity {
    ContentEntity {
        model: saved.model.clone(),
        model2: saved.model2.clone(),
        model3: saved.model3.clone(),
        model4: saved.model4.clone(),
        skin: saved.skin as i32,
        frame: saved.frame as i32,
        old_frame: saved.old_frame as i32,
        scale: saved.scale,
        effects: saved.effects as i64,
        render_flags: saved.render_flags as i32,
        flags: saved.flags as i64,
        server_flags: saved.server_flags as i32,
        view_height: saved.view_height as i32,
        max_health: saved.max_health,
        sound: saved.sound.clone(),
        visible: saved.visible,
    }
}

/// Convert a persistence attack checkpoint into the content shape.
///
/// Live actor handles stay `None` here; content `restore_q2_attack`
/// resolves the saved ids through the caller's resolver.
fn convert_attack(saved: &crate::persistence::q2::foundation::Q2AttackCheckpoint) -> Result<ContentAttack, WorldError> {
    Ok(ContentAttack {
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

/// Convert a persistence damage cause.
fn convert_cause(saved: &SavedCause) -> Result<ContentCause, WorldError> {
    match saved {
        SavedCause::Q1 {
            death_type,
            armor_effect,
        } => Ok(ContentCause::Q1 {
            death_type: death_type.clone(),
            armor_effect: armor_effect
                .as_deref()
                .map(|effect| match effect {
                    "bypass" => Ok(ContentArmorEffect::Bypass),
                    "half-effectiveness" => Ok(ContentArmorEffect::HalfEffectiveness),
                    _ => Err(WorldError::BadSave(format!("unknown attack armor effect {effect:?}"))),
                })
                .transpose()?,
        }),
        SavedCause::Q2 {
            means_of_death,
            damage_flags,
            native,
        } => Ok(ContentCause::Q2 {
            means_of_death: i32::try_from(*means_of_death)
                .map_err(|_| WorldError::BadSave("attack means of death is out of range".to_string()))?,
            damage_flags: i32::try_from(*damage_flags)
                .map_err(|_| WorldError::BadSave("attack damage flags are out of range".to_string()))?,
            native: native.as_ref().map(convert_native).transpose()?,
        }),
        SavedCause::Q3 {
            means_of_death,
            damage_flags,
        } => Ok(ContentCause::Q3 {
            means_of_death: i32::try_from(*means_of_death)
                .map_err(|_| WorldError::BadSave("attack means of death is out of range".to_string()))?,
            damage_flags: i32::try_from(*damage_flags)
                .map_err(|_| WorldError::BadSave("attack damage flags are out of range".to_string()))?,
        }),
        SavedCause::Environment { hazard } => Ok(ContentCause::Environment {
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

/// Convert a persistence native cause.
fn convert_native(saved: &SavedNative) -> Result<ContentNative, WorldError> {
    match saved {
        SavedNative::Classic { game, value } => Ok(ContentNative::Classic {
            game: match game.as_str() {
                "base" => ContentNativeGame::Base,
                "xatrix" => ContentNativeGame::Xatrix,
                "rogue" => ContentNativeGame::Rogue,
                "ctf" => ContentNativeGame::Ctf,
                _ => return Err(WorldError::BadSave(format!("unknown native attack game {game:?}"))),
            },
            value: i32::try_from(*value)
                .map_err(|_| WorldError::BadSave("native attack value is out of range".to_string()))?,
        }),
        SavedNative::Rerelease {
            id,
            friendly_fire,
            no_point_loss,
        } => Ok(ContentNative::Rerelease {
            id: i32::try_from(*id)
                .map_err(|_| WorldError::BadSave("native attack cause id is out of range".to_string()))?,
            friendly_fire: *friendly_fire,
            no_point_loss: *no_point_loss,
        }),
    }
}

/// Convert a `namespace:name` provider reference.
fn convert_provider(text: &str) -> Result<ProviderId, WorldError> {
    text.split_once(':')
        .map(|(namespace, name)| ProviderId::new(namespace, name))
        .ok_or_else(|| WorldError::BadSave(format!("expected namespace:name, found {text:?}")))
}

/// Convert a world inventory entry into the content twin.
fn convert_entry(saved: &SavedEntry) -> ContentEntry {
    ContentEntry {
        item: saved.item.clone(),
        count: saved.count,
        capacity: saved.capacity,
        count_policy: saved.count_policy.map(|policy| match policy {
            SavedPolicy::Stack => ContentCountPolicy::Stack,
            SavedPolicy::SourceCounter(arithmetic) => ContentCountPolicy::SourceCounter(match arithmetic {
                SavedArithmetic::Binary32 => ContentArithmetic::Binary32,
                SavedArithmetic::Binary64 => ContentArithmetic::Binary64,
                SavedArithmetic::Int32 => ContentArithmetic::Int32,
            }),
        }),
    }
}

/// Convert world armor state into the content twin.
fn convert_armor(saved: &SavedArmor) -> ContentArmor {
    ContentArmor {
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

#[cfg(test)]
mod tests {
    use qa_core::identity::SavedActorId;
    use qa_core::math::Vec3;
    use qa_core::time::SourceTime;
    use qa_world::combat::{ArmorState as SavedArmor, PoweredProtection, RegularArmor};
    use qa_world::inventory::InventoryEntry as SavedInventoryEntry;

    use super::*;
    use crate::persistence::q2::foundation::Q2AttackCheckpoint as SavedAttack;

    fn vec() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    fn sample_state() -> SavedState {
        SavedState {
            slot: 1,
            entered_at: 0.0,
            use_q2_weapons: true,
            use_q2_inventory: true,
            spawn_inventory: vec![SavedInventoryEntry {
                item: "q2:blaster".to_string(),
                count: 1.0,
                capacity: 1.0,
                count_policy: None,
            }],
            userinfo: String::new(),
            name: "player".to_string(),
            skin: "male/grunt".to_string(),
            gender: "male".to_string(),
            fov: 90.0,
            hand: "right".to_string(),
            spectator: false,
            requested_spectator: false,
            connected: true,
            dead: false,
            gibbed: false,
            noclip: false,
            god: false,
            notarget: false,
            score: 5.0,
            ping: 50.0,
            respawn_time: 0.0,
            air_finished: 0.0,
            next_drown_time: 0.0,
            drown_damage: 2.0,
            old_water_level: 0.0,
            breather_sound: 0.0,
            pain_debounce: 0.0,
            damage_blood: 0.0,
            damage_armor: 0.0,
            damage_power_armor: 0.0,
            damage_knockback: 0.0,
            damage_from: vec(),
            damage_blend: vec(),
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
            old_velocity: vec(),
            old_view_angles: vec(),
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
            animation_run: false,
            loop_sound: String::new(),
            selected_item: None,
            show_scores: false,
            show_inventory: false,
            show_help: false,
            chase_target: Some(SavedActorId { slot: 3, generation: 0 }),
            coop_respawn: Some(SavedCarry {
                health: 100.0,
                maximum_health: 100.0,
                armor: SavedArmor {
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
            flood_times: Vec::new(),
            flood_lock_until: 0.0,
        }
    }

    fn sample() -> SavedCheckpoint {
        SavedCheckpoint {
            pain_index: 1.0,
            death_index: 2.0,
            state: sample_state(),
            rules: SavedRules {
                password: String::new(),
                spectator_password: String::new(),
                max_spectators: 4.0,
                cheats: false,
                time_limit_minutes: 0.0,
                frag_limit: 0.0,
                map_list: Vec::new(),
                map_list_shuffle: false,
                next_map: String::new(),
                spawn_point: String::new(),
                flood_messages: 5.0,
                flood_seconds: 1.0,
                flood_wait_seconds: 10.0,
                roll_speed: 0.0,
                roll_angle: 0.0,
                run_pitch: 0.0,
                run_roll: 0.0,
                bob_up: 0.0,
                bob_pitch: 0.0,
                bob_roll: 0.0,
                gun_offset: vec(),
            },
            last_attack: Some(SavedAttack {
                sequence: 7,
                time: SourceTime::Seconds(0.0_f32),
                attacker: None,
                inflictor: None,
                originating_projectile: None,
                damage_powerup_owner: None,
                weapon: None,
                weapon_provider: "q2:game".to_string(),
                combat_provider: "q2:game".to_string(),
                inventory_provider: "q2:game".to_string(),
                movement_provider: "q2:game".to_string(),
                cause: SavedCause::Q2 {
                    means_of_death: 1,
                    damage_flags: 0,
                    native: None,
                },
            }),
            entity: SavedEntity {
                model: "players/male/tris.md2".to_string(),
                model2: String::new(),
                model3: String::new(),
                model4: String::new(),
                skin: 0.0,
                frame: 0.0,
                old_frame: 0.0,
                scale: 1.0,
                effects: 0.0,
                render_flags: 0.0,
                flags: 0.0,
                server_flags: 0.0,
                view_height: 22.0,
                max_health: 100.0,
                sound: String::new(),
                visible: true,
            },
        }
    }

    #[test]
    fn converts_character_shape() {
        let converted = convert_persistence_q2_character(&sample()).expect("convert");
        assert_eq!(converted.version, 1);
        assert_eq!(converted.pain_index, 1);
        assert_eq!(converted.death_index, 2);
        let state = &converted.state.state;
        assert_eq!(state.slot, 1);
        assert_eq!(state.gender, Q2PlayerGender::Male);
        assert_eq!(state.hand, Q2PlayerHand::Right);
        assert_eq!(state.fov, 90);
        assert_eq!(state.score, 5);
        assert_eq!(state.buttons, 1);
        assert_eq!(state.spawn_inventory.len(), 1);
        assert_eq!(state.chase_target, None);
        assert_eq!(
            converted.state.chase_target,
            Some(SavedActorId { slot: 3, generation: 0 })
        );
        let carry = state.coop_respawn.as_ref().expect("carry");
        assert_eq!(carry.score, 1);
        assert_eq!(carry.flags, 2);
        assert_eq!(converted.rules.max_spectators, 4);
        assert_eq!(converted.rules.flood_messages, 5);
        assert_eq!(converted.entity.skin, 0);
        assert_eq!(converted.entity.view_height, 22);
        let attack = converted.last_attack.as_ref().expect("attack");
        assert_eq!(attack.attack.sequence, 7);
        assert_eq!(attack.attack.weapon_provider.name, "game");
        assert!(matches!(
            attack.attack.cause,
            ContentCause::Q2 { means_of_death: 1, .. }
        ));
    }

    #[test]
    fn rejects_unknown_enums_and_overflow() {
        let mut bad = sample();
        bad.state.gender = "x".to_string();
        assert!(convert_persistence_q2_character(&bad).is_err());
        let mut bad = sample();
        bad.state.hand = "x".to_string();
        assert!(convert_persistence_q2_character(&bad).is_err());
        let mut bad = sample();
        bad.state.slot = u64::MAX;
        assert!(convert_persistence_q2_character(&bad).is_err());
    }
}
