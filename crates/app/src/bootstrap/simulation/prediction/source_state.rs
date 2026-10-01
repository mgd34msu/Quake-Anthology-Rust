//! Prediction snapshots across the Q2/Q3 source state boundary.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/prediction/source-state.ts`
//! (`PredictionSourceEntities`, `predictionSourceHit`, `predictionSourceNumber`,
//! `q2PredictionSnapshot`, `readPredictionSourceState`, `writePredictionSourceState`).

use qa_content::q3::base::shared::definitions::{stat_schema, Powerup, Product, StatSchema};
use qa_content::q3::base::shared::items::item_at;
use qa_content::q3::base::shared::player_state::SourcePlayerState;
use qa_content::q3::foundation::arsenal::{q3_weapon_item, Q3_WEAPON_ITEMS};
use qa_core::identity::ActorId;
use qa_core::math::{vec3, Vec3};
use qa_net::q2_adapters::{Q2Player, Q2Vec3};
use qa_world::movement::q2::types::{Q2MovementState, Q2RereleaseMovementState};
use qa_world::movement::q3::constants::move_flags::{RESPAWNED, USE_ITEM_HELD};
use qa_world::movement::q3::weapon::Q3ExternalWeaponSlot;
use qa_world::movement::types::{TraceHit, WeaponState};

use super::super::arsenal::selected::MovementState;
use super::step::copy_prediction_snapshot;
use super::types::MovementPredictionSnapshot;

/// Source entity number mapping for prediction snapshots.
pub trait PredictionSourceEntities {
    /// Actor at a source entity number.
    fn actor_at(&self, number: i32) -> Option<ActorId>;
    /// Source entity number of an actor.
    fn number_of(&self, actor: &ActorId) -> Option<i32>;
}

/// Errors reading source state into a prediction snapshot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceStateError {
    /// Item index out of range.
    #[error(transparent)]
    Items(#[from] qa_content::q3::base::shared::items::ItemsError),
}

/// Hit for a source entity number.
pub fn prediction_source_hit(number: i32, entities: &dyn PredictionSourceEntities) -> TraceHit {
    if number == 1022 {
        return TraceHit::World { model: 0 };
    }
    if number == 1023 {
        return TraceHit::None;
    }
    match entities.actor_at(number) {
        None => TraceHit::None,
        Some(actor) => TraceHit::Actor { actor },
    }
}

/// Source entity number for a hit.
pub fn prediction_source_number(hit: &TraceHit, entities: &dyn PredictionSourceEntities) -> i32 {
    match hit {
        TraceHit::World { .. } => 1022,
        TraceHit::None => 1023,
        TraceHit::Actor { actor } => entities.number_of(actor).unwrap_or(1023),
    }
}

fn vec3_of(value: &Q2Vec3) -> Vec3 {
    vec3(value.x as f32, value.y as f32, value.z as f32)
}

/// Q2 rerelease float movement never passes through the classic eighth-unit
/// representation.
pub fn q2_prediction_snapshot(
    base: &MovementPredictionSnapshot,
    player: &Q2Player,
    sequence: i64,
    command_time_milliseconds: f64,
) -> MovementPredictionSnapshot {
    let (movement, view_angles, view_offset, view_height, health) = match player {
        Q2Player::Classic(player) => (
            MovementState::Q2Classic(Q2MovementState {
                move_type: i32::from(player.movement.move_type),
                origin_eighths: player.movement.origin_eighths,
                velocity_eighths: player.movement.velocity_eighths,
                flags: player.movement.flags,
                time_eight_milliseconds: player.movement.time,
                gravity: f64::from(player.movement.gravity),
                delta_angle_shorts: [
                    i32::from(player.movement.delta_angle_shorts[0]),
                    i32::from(player.movement.delta_angle_shorts[1]),
                    i32::from(player.movement.delta_angle_shorts[2]),
                ],
            }),
            vec3_of(&player.view.view_angles),
            vec3_of(&player.view.view_offset),
            player.view.view_offset.z,
            f64::from(player.view.stats.get(1).copied().unwrap_or(0)),
        ),
        Q2Player::Rerelease(player) => (
            MovementState::Q2Rerelease(Q2RereleaseMovementState {
                move_type: i32::from(player.movement.move_type),
                origin: vec3_of(&player.movement.origin),
                velocity: vec3_of(&player.movement.velocity),
                flags: player.movement.flags,
                time_milliseconds: player.movement.time,
                gravity: f64::from(player.movement.gravity),
                delta_angles: vec3_of(&player.movement.delta_angles),
                view_height: f64::from(player.movement.view_height),
            }),
            vec3_of(&player.view.view_angles),
            vec3_of(&player.view.view_offset),
            f64::from(player.movement.view_height),
            f64::from(player.view.stats.get(1).copied().unwrap_or(0)),
        ),
    };
    let copy = copy_prediction_snapshot(base);
    copy_prediction_snapshot(&MovementPredictionSnapshot {
        sequence,
        command_time_milliseconds,
        state: movement,
        view_angles,
        view_offset,
        view_height,
        environment: qa_world::movement::types::MovementEnvironment {
            health,
            ..copy.environment
        },
        ..copy
    })
}

fn schema_max_health(schema: StatSchema) -> usize {
    match schema {
        StatSchema::Base(layout) => layout.max_health as usize,
        StatSchema::Missionpack(layout) => layout.max_health as usize,
    }
}

fn persistent_powerup_tag(product: Product, ps: &SourcePlayerState) -> Result<i32, SourceStateError> {
    let schema = stat_schema(product);
    match schema {
        StatSchema::Missionpack(layout) => {
            Ok(item_at(product, ps.stats.get(layout.persistent_powerup as usize))?.tag())
        }
        StatSchema::Base(_) => Ok(0),
    }
}

fn holdable_tag(product: Product, holdable_item: i32) -> Result<i32, SourceStateError> {
    Ok(item_at(product, holdable_item)?.tag())
}

/// Read a source player state into a prediction snapshot.
pub fn read_prediction_source_state(
    base: &MovementPredictionSnapshot,
    ps: &SourcePlayerState,
    entities: &dyn PredictionSourceEntities,
) -> Result<MovementPredictionSnapshot, SourceStateError> {
    let source = copy_prediction_snapshot(base);
    let product = ps.product();
    let schema = stat_schema(product);
    let q3_state = qa_world::movement::q3::types::Q3MovementState {
        command_time_milliseconds: ps.command_time,
        movement_type: ps.pm_type,
        bob_cycle: ps.bob_cycle,
        movement_flags: ps.pm_flags,
        movement_time_milliseconds: ps.pm_time,
        origin: ps.origin(),
        velocity: ps.velocity(),
        gravity: f64::from(ps.gravity),
        speed: f64::from(ps.speed),
        delta_angle_words: [
            ps.delta_angles.x as i32,
            ps.delta_angles.y as i32,
            ps.delta_angles.z as i32,
        ],
        ground: prediction_source_hit(ps.ground_entity_num, entities),
        movement_direction: ps.movement_dir,
        grapple_point: ps.grapple_point,
        flags: ps.e_flags,
        view_angles: ps.viewangles,
        view_height: f64::from(ps.viewheight),
        predictable_event_sequence: ps.event_sequence,
        jump_pad: if ps.jumppad_ent == 0 {
            None
        } else {
            entities.actor_at(ps.jumppad_ent)
        },
        movement_frame: ps.pmove_framecount,
        jump_pad_frame: ps.jumppad_frame,
    };
    let arsenal = if !matches!(source.arsenal.state, WeaponState::Q3 { .. }) {
        source.arsenal.clone()
    } else {
        let owned = ps.stats.get(schema.weapons());
        let ammo = Q3_WEAPON_ITEMS
            .iter()
            .filter(|item| product == Product::Missionpack || (item.weapon as i32) < 11)
            .flat_map(|item| {
                let mut entries = vec![qa_world::movement::types::InventoryEntry {
                    item: item.item.clone(),
                    count: if owned & (1 << (item.weapon as i32)) != 0 {
                        1.0
                    } else {
                        0.0
                    },
                }];
                if let Some(ammo) = &item.ammo {
                    entries.push(qa_world::movement::types::InventoryEntry {
                        item: ammo.clone(),
                        count: f64::from(ps.ammo.get(item.weapon as i32 as usize)),
                    });
                }
                entries
            })
            .collect();
        qa_world::movement::types::ArsenalState {
            provider: source.arsenal.provider.clone(),
            active_weapon: q3_weapon_item(ps.weapon).map(|item| item.item.clone()),
            state: WeaponState::Q3 {
                source_weapon: ps.weapon,
                state: ps.weapon_state,
                time_milliseconds: ps.weapon_time,
            },
            ammo,
        }
    };
    let holdable_item = ps.stats.get(schema.holdable_item());
    let state = match &source.state {
        MovementState::Q3(_) => MovementState::Q3(q3_state),
        MovementState::Q2Classic(state) => {
            let mut next = *state;
            next.origin_eighths = [
                (f64::from(ps.origin().x) * 8.0).trunc() as i32,
                (f64::from(ps.origin().y) * 8.0).trunc() as i32,
                (f64::from(ps.origin().z) * 8.0).trunc() as i32,
            ];
            next.velocity_eighths = [
                (f64::from(ps.velocity().x) * 8.0).trunc() as i32,
                (f64::from(ps.velocity().y) * 8.0).trunc() as i32,
                (f64::from(ps.velocity().z) * 8.0).trunc() as i32,
            ];
            MovementState::Q2Classic(next)
        }
        MovementState::Q1Netquake(state) => {
            let mut next = state.clone();
            next.origin = ps.origin();
            next.velocity = ps.velocity();
            MovementState::Q1Netquake(next)
        }
        MovementState::Q1Quakeworld(state) => {
            let mut next = state.clone();
            next.origin = ps.origin();
            next.velocity = ps.velocity();
            MovementState::Q1Quakeworld(next)
        }
        MovementState::Q2Rerelease(state) => {
            let mut next = *state;
            next.origin = ps.origin();
            next.velocity = ps.velocity();
            MovementState::Q2Rerelease(next)
        }
    };
    let animation = match &source.animation.state {
        qa_world::movement::types::AnimationState::Q3 { .. } => {
            let mut animation = source.animation.clone();
            animation.state = qa_world::movement::types::AnimationState::Q3 {
                legs: ps.legs_anim,
                torso: ps.torso_anim,
                legs_timer_milliseconds: ps.legs_timer,
                torso_timer_milliseconds: ps.torso_timer,
            };
            animation
        }
        _ => source.animation.clone(),
    };
    let q3_arsenal = if !matches!(arsenal.state, WeaponState::Q3 { .. }) {
        None
    } else {
        Some(qa_content::q3::foundation::arsenal::Q3ArsenalRuntimeState {
            product,
            max_health: f64::from(ps.stats.get(schema_max_health(schema))),
            spectator: ps.pm_type == 1,
            persistent_powerup_tag: persistent_powerup_tag(product, ps)?,
            holdable_item,
            holdable_tag: holdable_tag(product, holdable_item)?,
            respawned: ps.pm_flags & RESPAWNED != 0,
            use_item_held: ps.pm_flags & USE_ITEM_HELD != 0,
            event_sequence: ps.event_sequence,
            fractional_milliseconds: source
                .q3_arsenal
                .as_ref()
                .map(|arsenal| arsenal.fractional_milliseconds)
                .unwrap_or(0.0),
            external_slot: source
                .q3_arsenal
                .as_ref()
                .map(|arsenal| arsenal.external_slot)
                .unwrap_or(Q3ExternalWeaponSlot::Active),
            requested_weapon: source.q3_arsenal.as_ref().and_then(|arsenal| arsenal.requested_weapon),
        })
    };
    Ok(MovementPredictionSnapshot {
        command_time_milliseconds: f64::from(ps.command_time),
        state,
        arsenal,
        animation,
        view_angles: ps.viewangles,
        view_height: f64::from(ps.viewheight),
        environment: qa_world::movement::types::MovementEnvironment {
            gravity_multiplier: if matches!(source.state, MovementState::Q3(_)) {
                1.0
            } else {
                source.environment.gravity_multiplier
            },
            health: f64::from(ps.health()),
            flight: ps.powerups.get(Powerup::PwFlight as usize) != 0,
            haste: ps.powerups.get(Powerup::PwHaste as usize) != 0,
            invulnerable: product == Product::Missionpack && ps.powerups.get(Powerup::PwInvulnerability as usize) != 0,
            ..source.environment
        },
        q3_arsenal,
        ..source
    })
}

/// Write a prediction snapshot back to a source player state.
pub fn write_prediction_source_state(
    ps: &mut SourcePlayerState,
    output: &MovementPredictionSnapshot,
    entities: &dyn PredictionSourceEntities,
) {
    let state = &output.state;
    ps.command_time = output.command_time_milliseconds as i32;
    let (origin, velocity) = match state {
        MovementState::Q2Classic(state) => (
            vec3(
                state.origin_eighths[0] as f32 / 8.0,
                state.origin_eighths[1] as f32 / 8.0,
                state.origin_eighths[2] as f32 / 8.0,
            ),
            vec3(
                state.velocity_eighths[0] as f32 / 8.0,
                state.velocity_eighths[1] as f32 / 8.0,
                state.velocity_eighths[2] as f32 / 8.0,
            ),
        ),
        MovementState::Q1Netquake(state) => (state.origin, state.velocity),
        MovementState::Q1Quakeworld(state) => (state.origin, state.velocity),
        MovementState::Q2Rerelease(state) => (state.origin, state.velocity),
        MovementState::Q3(state) => (state.origin, state.velocity),
    };
    ps.set_origin(origin);
    ps.set_velocity(velocity);
    ps.viewangles = output.view_angles;
    ps.viewheight = output.view_height as i32;
    if let Some(contact) = &output.contact {
        ps.ground_entity_num = prediction_source_number(&contact.ground, entities);
    }
    if let MovementState::Q3(state) = state {
        ps.command_time = state.command_time_milliseconds;
        ps.pm_type = state.movement_type;
        ps.pm_flags = state.movement_flags;
        ps.pm_time = state.movement_time_milliseconds;
        ps.bob_cycle = state.bob_cycle;
        ps.delta_angles = vec3(
            state.delta_angle_words[0] as f32,
            state.delta_angle_words[1] as f32,
            state.delta_angle_words[2] as f32,
        );
        ps.ground_entity_num = prediction_source_number(&state.ground, entities);
        ps.movement_dir = state.movement_direction;
        ps.e_flags = state.flags;
        ps.pmove_framecount = state.movement_frame;
        ps.jumppad_frame = state.jump_pad_frame;
        ps.jumppad_ent = state
            .jump_pad
            .as_ref()
            .and_then(|actor| entities.number_of(actor))
            .unwrap_or(0);
    }
    if let WeaponState::Q3 {
        source_weapon,
        state: weapon_state,
        time_milliseconds,
    } = &output.arsenal.state
    {
        ps.weapon = *source_weapon;
        ps.weapon_state = *weapon_state;
        ps.weapon_time = *time_milliseconds;
        let mut owned = 0;
        for item in Q3_WEAPON_ITEMS.iter() {
            if output
                .arsenal
                .ammo
                .iter()
                .find(|entry| entry.item == item.item)
                .map(|entry| entry.count)
                .unwrap_or(0.0)
                > 0.0
            {
                owned |= 1 << (item.weapon as i32);
            }
            if let Some(ammo) = &item.ammo {
                ps.ammo.set(
                    item.weapon as i32 as usize,
                    output
                        .arsenal
                        .ammo
                        .iter()
                        .find(|entry| entry.item == *ammo)
                        .map(|entry| entry.count)
                        .unwrap_or(0.0) as i32,
                );
            }
        }
        let schema = stat_schema(ps.product());
        ps.stats.set(schema.weapons(), owned);
        if let Some(q3_arsenal) = &output.q3_arsenal {
            ps.stats.set(schema.holdable_item(), q3_arsenal.holdable_item);
            ps.pm_flags = (ps.pm_flags & !(RESPAWNED | USE_ITEM_HELD))
                | if q3_arsenal.respawned { RESPAWNED } else { 0 }
                | if q3_arsenal.use_item_held { USE_ITEM_HELD } else { 0 };
        }
    }
    if let qa_world::movement::types::AnimationState::Q3 {
        legs,
        torso,
        legs_timer_milliseconds,
        torso_timer_milliseconds,
    } = &output.animation.state
    {
        ps.legs_anim = *legs;
        ps.torso_anim = *torso;
        ps.legs_timer = *legs_timer_milliseconds;
        ps.torso_timer = *torso_timer_milliseconds;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;
    use qa_net::q2_adapters::{Q2PlayerState, Q2PlayerView, Q2Vec3};
    use qa_world::movement::q1::types::QwMovementState;
    use qa_world::movement::types::{ActorAnimationState, AnimationState, ArsenalState, TraceHit, WeaponState};

    use super::super::test_support::standing_bounds;
    use super::*;

    struct MapEntities {
        actors: HashMap<i32, ActorId>,
    }

    impl PredictionSourceEntities for MapEntities {
        fn actor_at(&self, number: i32) -> Option<ActorId> {
            self.actors.get(&number).cloned()
        }

        fn number_of(&self, actor: &ActorId) -> Option<i32> {
            self.actors
                .iter()
                .find(|(_, id)| *id == actor)
                .map(|(number, _)| *number)
        }
    }

    fn snapshot() -> MovementPredictionSnapshot {
        MovementPredictionSnapshot {
            sequence: 0,
            command_time_milliseconds: 0.0,
            state: MovementState::Q1Quakeworld(QwMovementState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                old_buttons: 0,
                water_jump_time_seconds: 0.0,
                dead: false,
                spectator: 0,
                ground: TraceHit::None,
            }),
            arsenal: ArsenalState {
                provider: ProviderId::new("sim", "test"),
                active_weapon: None,
                state: WeaponState::Q1 {
                    frame: 0,
                    attack_finished_seconds: 0.0,
                    source_weapon: 0,
                },
                ammo: Vec::new(),
            },
            animation: ActorAnimationState {
                provider: ProviderId::new("sim", "test"),
                state: AnimationState::Q1 {
                    frame: 0,
                    next_frame_seconds: 0.0,
                },
            },
            environment: qa_world::movement::types::MovementEnvironment::default(),
            bounds: standing_bounds(),
            view_angles: vec3(0.0, 0.0, 0.0),
            view_height: 22.0,
            view_offset: vec3(0.0, 0.0, 22.0),
            contact: None,
            q3_arsenal: None,
        }
    }

    #[test]
    fn entity_numbers_roundtrip() {
        let owner = qa_core::identity::IdentityOwner::create("source-state-test").unwrap();
        let actor = owner.actor(3, 1);
        let entities = MapEntities {
            actors: [(5, actor.clone())].into_iter().collect(),
        };
        let entities: &dyn PredictionSourceEntities = &entities;
        assert_eq!(prediction_source_hit(1022, entities), TraceHit::World { model: 0 });
        assert_eq!(prediction_source_hit(1023, entities), TraceHit::None);
        assert_eq!(
            prediction_source_hit(5, entities),
            TraceHit::Actor { actor: actor.clone() }
        );
        assert_eq!(prediction_source_hit(6, entities), TraceHit::None);
        assert_eq!(prediction_source_number(&TraceHit::World { model: 0 }, entities), 1022);
        assert_eq!(prediction_source_number(&TraceHit::None, entities), 1023);
        assert_eq!(prediction_source_number(&TraceHit::Actor { actor }, entities), 5);
    }

    #[test]
    fn q2_players_replace_movement() {
        let base = snapshot();
        let player = Q2Player::Classic(Q2PlayerState {
            view: Q2PlayerView {
                view_angles: Q2Vec3 {
                    x: 10.0,
                    y: 20.0,
                    z: 30.0,
                },
                view_offset: Q2Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 24.0,
                },
                stats: vec![0, 75],
                ..Default::default()
            },
            ..Default::default()
        });
        let predicted = q2_prediction_snapshot(&base, &player, 9, 900.0);
        assert_eq!(predicted.sequence, 9);
        assert_eq!(predicted.command_time_milliseconds, 900.0);
        assert_eq!(predicted.view_angles, vec3(10.0, 20.0, 30.0));
        assert_eq!(predicted.view_height, 24.0);
        assert_eq!(predicted.environment.health, 75.0);
        assert!(matches!(predicted.state, MovementState::Q2Classic(_)));
    }

    #[test]
    fn source_roundtrip_preserves_pose() {
        let base = snapshot();
        let entities = MapEntities { actors: HashMap::new() };
        let entities: &dyn PredictionSourceEntities = &entities;
        let mut ps = SourcePlayerState::new(Product::Baseq3, None);
        ps.set_origin(vec3(1.0, 2.0, 3.0));
        ps.set_velocity(vec3(4.0, 5.0, 6.0));
        ps.command_time = 120;
        let read = read_prediction_source_state(&base, &ps, entities).unwrap();
        assert_eq!(read.command_time_milliseconds, 120.0);
        match &read.state {
            MovementState::Q1Quakeworld(state) => {
                assert_eq!(state.origin, vec3(1.0, 2.0, 3.0));
                assert_eq!(state.velocity, vec3(4.0, 5.0, 6.0));
            }
            _ => panic!("family changed"),
        }
        let mut back = SourcePlayerState::new(Product::Baseq3, None);
        write_prediction_source_state(&mut back, &read, entities);
        assert_eq!(back.origin(), vec3(1.0, 2.0, 3.0));
        assert_eq!(back.command_time, 120);
        assert_eq!(back.viewheight, 0);
    }
}
