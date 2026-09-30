//! Mission-pack doppleganger (`src/content/q2/missionpacks/doppleganger.ts`).
//!
//! Rogue g_newdm.c/g_items.c decoy creation, animation, and sphere
//! retaliation (GPL-2.0-or-later).

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, length3, scale3, sub3, vec3};

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2Die, Q2GameServices, Q2MotionKind, Q2Pain, Q2Solid, Q2Think,
};
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use crate::q2::support::contracts::{CombatState, DeathReaction, PainReaction};

use super::monsters::spawn::{
    check_rogue_ground_spawn_point, find_rogue_spawn_point, rogue_spawn_callbacks,
    rogue_spawn_grow,
};
use super::projectiles::common::explode;
use super::projectiles::{mission_projectiles, Q2MissionPackProjectiles};
use super::spheres::{mission_spheres, Q2SphereKind};

/// Doppleganger callbacks (`Q2MissionPackDoppleganger::callbacks`).
pub fn doppleganger_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = rogue_spawn_callbacks();
    callbacks.think.insert("doppleganger_timeout", doppleganger_timeout as Q2Think);
    callbacks.think.insert("body_think", doppleganger_body_think as Q2Think);
    callbacks.pain.insert("doppleganger_pain", doppleganger_pain as Q2Pain);
    callbacks.die.insert("doppleganger_die", doppleganger_die as Q2Die);
    callbacks
}

/// Mission-pack doppleganger (`Q2MissionPackDoppleganger`).
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackDoppleganger {
    /// Projectile driver.
    pub projectiles: Q2MissionPackProjectiles,
}

/// Doppleganger driver bound to the session hooks.
pub fn mission_doppleganger(game: &Q2GameServices) -> Q2MissionPackDoppleganger {
    Q2MissionPackDoppleganger {
        projectiles: mission_projectiles(game),
    }
}

impl Q2MissionPackDoppleganger {
    /// Use a doppleganger item (`use`).
    pub fn use_doppleganger(&self, owner: &ActorId, game: &mut Q2GameServices) -> bool {
        let body = game.body_of(owner.clone());
        let angles = game
            .weapons
            .inputs
            .get(owner)
            .map(|input| input.angles)
            .unwrap_or(body.angles);
        let forward = angle_vectors(vec3(0.0, angles.y, 0.0)).forward;
        let point = find_rogue_spawn_point(
            game,
            add3(body.origin, scale3(forward, 48.0)),
            body.bounds,
            32.0,
        );
        let Some(point) = point else {
            return false;
        };
        if !check_rogue_ground_spawn_point(game, point, body.bounds, 64.0, -1.0) {
            return false;
        }
        let owned = game.owned_of(owner.clone());
        if !game.host.inventory().consume(&owned, &"q2:item_doppleganger".to_string(), 1.0) {
            return false;
        }
        rogue_spawn_grow(game, point, 0);
        self.fire(owner, game, point, forward);
        true
    }

    /// Fire a doppleganger decoy (`fire`).
    pub fn fire(
        &self,
        owner: &ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
    ) -> ActorId {
        game.source_callbacks.register(&doppleganger_callbacks());
        let base = game.create("doppleganger", BTreeMap::new());
        let angles = vector_angles(direction);
        {
            let record = game.require_entity_mut(&base);
            record.team_master = Some(owner.clone());
            record.render_flags = 0x8000;
            record.damageable_target = true;
            record.pain = Some(doppleganger_pain as Q2Pain);
            record.die = Some(doppleganger_die as Q2Die);
        }
        let mut moved = game.body_of(base.clone());
        moved.origin = start;
        moved.angles = vec3(0.0, angles.y, angles.z);
        moved.velocity = Vec3::default();
        moved.bounds.min = vec3(-16.0, -16.0, -24.0);
        moved.bounds.max = vec3(16.0, 16.0, 32.0);
        game.write_body(base.clone(), &moved, true);
        game.set_motion_kind(base.clone(), Q2MotionKind::Toss);
        game.set_solid(base.clone(), Q2Solid::Box);
        let owned = game.owned_of(base.clone());
        game.host.combat().create(
            &owned,
            &CombatState {
                health: 30.0,
                armor: ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
                mass: 0.0,
                can_take_damage: true,
                invulnerable: false,
                no_knockback: false,
                team: None,
            },
        );
        game.schedule(base.clone(), 30.0, doppleganger_timeout as Q2Think);
        let decoy = game.create("doppleganger_body", BTreeMap::new());
        {
            let owner_record = game.require_entity(owner);
            let model = owner_record.model.clone();
            let model2 = owner_record.model2.clone();
            let model3 = owner_record.model3.clone();
            let model4 = owner_record.model4.clone();
            let frame = owner_record.frame;
            let old_frame = owner_record.old_frame;
            let skin = owner_record.skin;
            let effects = owner_record.effects;
            let render_flags = owner_record.render_flags;
            let scale = owner_record.scale;
            let record = game.require_entity_mut(&decoy);
            record.model = model;
            record.model2 = model2;
            record.model3 = model3;
            record.model4 = model4;
            record.frame = frame;
            record.old_frame = old_frame;
            record.skin = skin;
            record.effects = effects;
            record.render_flags = render_flags;
            record.scale = scale;
            record.team_master = Some(base.clone());
            record.speed = 30.0;
        }
        let mut moved = game.body_of(decoy.clone());
        moved.origin = add3(start, vec3(0.0, 0.0, 8.0));
        moved.angles = game.body_of(owner.clone()).angles;
        game.write_body(decoy.clone(), &moved, true);
        game.show(decoy.clone());
        let frame_seconds = game.host.frame_seconds();
        game.schedule(decoy.clone(), frame_seconds, doppleganger_body_think as Q2Think);
        game.require_entity_mut(&base).team_chain = Some(decoy);
        base
    }
}

/// Doppleganger timeout (`timeout`).
fn doppleganger_timeout(entity: ActorId, game: &mut Q2GameServices) {
    if let Some(body) = game
        .require_entity(&entity)
        .team_chain
        .clone()
        .and_then(|chain| game.entity(&chain).map(|entity| entity.actor.id().clone()))
    {
        explode(&body, game, "explosion1");
    }
    explode(&entity, game, "explosion1");
}

/// Doppleganger pain (`pain`).
fn doppleganger_pain(entity: ActorId, game: &mut Q2GameServices, reaction: PainReaction) {
    game.require_entity_mut(&entity).enemy = reaction.attacker;
}

/// Doppleganger die (`die`).
fn doppleganger_die(entity: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    let enemy = game.require_entity(&entity).enemy.clone();
    let body = enemy.as_ref().and_then(|enemy| game.host.bodies().read(enemy));
    if let (Some(enemy), Some(body)) = (enemy, body) {
        if Some(&enemy) != game.require_entity(&entity).team_master.as_ref() {
            let distance = f64::from(length3(sub3(body.origin, game.body_of(entity.clone()).origin)));
            let sphere = mission_spheres(game).launch(
                &entity,
                game,
                if distance > 768.0 {
                    Q2SphereKind::Hunter
                } else {
                    Q2SphereKind::Vengeance
                },
                true,
            );
            if let Some(pain) = game.require_entity(&sphere).pain {
                let owned = game.owned_of(sphere.clone());
                pain(
                    sphere,
                    game,
                    PainReaction {
                        attack: reaction.pain.attack.clone(),
                        this: owned,
                        attacker: reaction.pain.attacker.clone(),
                        damage: 0.0,
                        kick: 0.0,
                    },
                );
            }
        }
    }
    doppleganger_timeout(entity, game);
}

/// Doppleganger body think (`bodyThink`).
fn doppleganger_body_think(entity: ActorId, game: &mut Q2GameServices) {
    let angles = game.body_of(entity.clone()).angles;
    let yaw = (f64::from(angles.y) * 65536.0 / 360.0).trunc() as i64 & 65535;
    let yaw = yaw as f64 * (360.0 / 65536.0);
    let pos_y = f64::from(game.require_entity(&entity).pos1.y);
    if (pos_y - yaw).abs() < 2.0 {
        if game.require_entity(&entity).timestamp < game.host.now() && game.host.random() < 0.1 {
            game.require_entity_mut(&entity).pos1.y = (game.host.random() * 350.0) as f32;
            game.require_entity_mut(&entity).timestamp = game.host.now() + 1.0;
        }
    } else {
        let mut travel = pos_y - yaw;
        if pos_y > yaw && travel >= 180.0 {
            travel -= 360.0;
        } else if pos_y <= yaw && travel <= -180.0 {
            travel += 360.0;
        }
        let speed = game.require_entity(&entity).speed;
        let mut moved = game.body_of(entity.clone());
        moved.angles = vec3(
            angles.x,
            ((yaw + travel.max(-speed).min(speed) + 360.0) % 360.0) as f32,
            angles.z,
        );
        game.write_body(entity.clone(), &moved, true);
    }
    {
        let record = game.require_entity_mut(&entity);
        record.frame += 1;
        if record.frame > 39 {
            record.frame = 0;
        }
    }
    game.show(entity.clone());
    game.schedule(entity, 0.1, doppleganger_body_think as Q2Think);
}
