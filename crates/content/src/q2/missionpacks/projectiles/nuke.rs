//! Mission-pack nuke (`src/content/q2/missionpacks/projectiles/nuke.ts`).
//!
//! Rogue g_newweap.c / g_combat.c antimatter bomb (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, length3, scale3, sub3, vec3};

use crate::contract::{ArmorState, PoweredProtectionState, ProjectileRole, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2Die, Q2GameServices, Q2Think, Q2Touch, Q2TraceRequest,
};
use crate::q2::foundation::weapons::ballistics::{weapon_player_noise_for_actor, NoiseKind};
use crate::q2::foundation::weapons::player::weapon_emit;
use crate::q2::foundation::weapons::types::Q2WeaponEvent;
use crate::q2::support::contracts::{CombatState, CombatTraitChanges, DeathReaction, TouchContact};

use super::common::{effect, publish_projectile, velocity};
use super::super::types::{Q2_MISSION_PACK_DAMAGE, Q2MissionPackPlayerEffect};
use super::Q2MissionPackProjectiles;

/// Nuke callbacks (merged over the mine callbacks).
pub fn nuke_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = super::mines::mine_callbacks();
    callbacks.think.insert("Nuke_Think", nuke_think as Q2Think);
    callbacks.think.insert("Nuke_Quake", nuke_quake as Q2Think);
    callbacks.touch.insert("nuke_bounce", nuke_bounce as Q2Touch);
    callbacks.die.insert("nuke_die", nuke_die as Q2Die);
    callbacks
}

impl Q2MissionPackProjectiles {
    /// Fire a nuke (`fireNuke`).
    pub fn fire_nuke(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        speed: f64,
        multiplier: f64,
    ) -> ActorId {
        let bomb = self.throw_mine(&owner, game, "nuke", start, direction, speed);
        let mut moved = game.body_of(bomb.clone());
        moved.angles = Vec3::default();
        moved.bounds.min = vec3(-8.0, -8.0, 0.0);
        moved.bounds.max = vec3(8.0, 8.0, 16.0);
        game.write_body(bomb.clone(), &moved, false);
        let fuse = game.host.now() + 10.0;
        {
            let entity = game.require_entity_mut(&bomb);
            entity.damage = 400.0 * multiplier;
            entity.damage_radius = if multiplier == 1.0 {
                512.0
            } else {
                512.0 + 128.0 * multiplier
            };
            entity.wait = fuse;
            entity.die = Some(nuke_die as Q2Die);
            entity.touch = Some(nuke_bounce as Q2Touch);
        }
        let owned = game.owned_of(bomb.clone());
        game.host.combat().create(
            &owned,
            &CombatState {
                health: 10000.0,
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
        let frame_seconds = game.host.frame_seconds();
        game.schedule(bomb.clone(), frame_seconds, nuke_think as Q2Think);
        publish_projectile(
            bomb.clone(),
            game,
            "",
            Some(("q2:ammo_nuke", ProjectileRole::Grenade)),
        );
        bomb
    }

    /// Explode a nuke (`nukeExplode`).
    fn nuke_explode(&self, entity: ActorId, game: &mut Q2GameServices) {
        let origin = game.body_of(entity.clone()).origin;
        let owner = game.require_entity(&entity).team_master.clone();
        let mut blinded: Vec<ActorId> = Vec::new();
        if let Some(owner) = owner.clone() {
            weapon_player_noise_for_actor(game, owner, origin, NoiseKind::Impact);
        }
        let (damage, radius) = {
            let record = game.require_entity(&entity);
            (record.damage, record.damage_radius)
        };
        for actor in game.host.nearby(origin, radius * 2.0) {
            if actor == entity
                || !game.host.actors().is_live(&actor)
                || !game
                    .host
                    .combat()
                    .read(&actor)
                    .is_some_and(|combat| combat.can_take_damage)
            {
                continue;
            }
            let target = game.entity(&actor).map(|entity| entity.actor.id().clone());
            let body = game.host.bodies().read(&actor);
            let Some(body) = body else {
                continue;
            };
            if !game.host.is_player(&actor)
                && !game.host.is_monster(&actor)
                && target.as_ref().is_none_or(|target| !game.require_entity(target).damageable_target)
            {
                continue;
            }
            let center = add3(
                body.origin,
                scale3(add3(body.bounds.min, body.bounds.max), 0.5),
            );
            let distance = f64::from(length3(sub3(origin, center)));
            let points = if distance <= radius {
                10000.0
            } else {
                damage / radius * (2.0 * radius - distance)
            };
            if points <= 0.0 {
                continue;
            }
            if game.host.is_player(&actor) {
                if distance <= radius {
                    if let Some(target) = target {
                        game.require_entity_mut(&target).flags |= 0x10000;
                    }
                }
                (self.hooks.player_effect)(Q2MissionPackPlayerEffect::NukeBlind {
                    actor: actor.clone(),
                    until: game.host.now() + 2.0,
                });
                blinded.push(actor.clone());
            }
            game.damage(
                actor,
                entity.clone(),
                owner.clone(),
                points.trunc(),
                points.trunc(),
                sub3(body.origin, origin),
                origin,
                Vec3::default(),
                Q2_MISSION_PACK_DAMAGE.nuke,
                1,
                Some("q2:ammo_nuke".to_string()),
            );
        }
        for actor in game.host.players() {
            if blinded.contains(&actor) {
                continue;
            }
            let Some(body) = game.host.bodies().read(&actor) else {
                continue;
            };
            let trace = game.host.trace(&Q2TraceRequest {
                start: origin,
                end: body.origin,
                bounds: None,
                ignore: Some(entity.clone()),
                mask: 3,
                exclude: Vec::new(),
            });
            let duration = if trace.fraction == 1.0 {
                2.0
            } else if f64::from(length3(sub3(body.origin, origin))) < 2048.0 {
                1.5
            } else {
                1.0
            };
            (self.hooks.player_effect)(Q2MissionPackPlayerEffect::NukeBlind {
                actor,
                until: game.host.now() + duration,
            });
        }
        if game.require_entity(&entity).damage > 400.0 {
            game.sound(&entity, "items/damage3.wav", 3, 1.0, 1.0);
        }
        game.sound(&entity, "weapons/grenlx1a.wav", 10, 1.0, 0.0);
        effect(&entity, game, "explosion1_big", Vec3::default(), 1, 0);
        effect(&entity, game, "nukeblast", Vec3::default(), 1, 0);
        let quake_until = game.host.now() + 3.0;
        {
            let record = game.require_entity_mut(&entity);
            record.visible = false;
            record.speed = 100.0;
            record.timestamp = quake_until;
            record.delay = 0.0;
        }
        game.show(entity.clone());
        let frame_seconds = game.host.frame_seconds();
        game.schedule(entity, frame_seconds, nuke_quake as Q2Think);
    }
}

/// Nuke bounce touch (`nukeBounce`).
fn nuke_bounce(entity: ActorId, game: &mut Q2GameServices, _contact: TouchContact) {
    let bounce = if game.host.random() > 0.5 {
        "weapons/hgrenb1a.wav"
    } else {
        "weapons/hgrenb2a.wav"
    };
    game.sound(&entity, bounce, 2, 1.0, 1.0);
}

/// Nuke die (`nukeDie`).
fn nuke_die(entity: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    let owned = game.owned_of(entity.clone());
    game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(false),
            mass: None,
            invulnerable: None,
            team: None,
            no_knockback: None,
        },
    );
    let classname = reaction
        .pain
        .attacker
        .clone()
        .and_then(|attacker| game.entity(&attacker).map(|entity| entity.classname.clone()));
    if classname.as_deref() == Some("nuke") {
        game.remove_actor(entity);
    } else {
        super::mission_projectiles(game).nuke_explode(entity, game);
    }
}

/// Nuke quake think (`nukeQuake`).
fn nuke_quake(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).delay < game.host.now() {
        game.sound(&entity, "world/rumble.wav", 0, 0.75, 0.0);
        game.require_entity_mut(&entity).delay = game.host.now() + 0.5;
    }
    for actor in game.host.players() {
        let Some(body) = game.host.bodies().read(&actor) else {
            continue;
        };
        if body.ground.is_none() {
            continue;
        }
        let mass = game.host.combat().read(&actor).map(|combat| combat.mass).unwrap_or(200.0);
        let speed = game.require_entity(&entity).speed;
        let shake_x = ((game.host.random() * 2.0 - 1.0) * 150.0) as f32;
        let shake_y = ((game.host.random() * 2.0 - 1.0) * 150.0) as f32;
        velocity(
            game,
            &actor,
            vec3(
                body.velocity.x + shake_x,
                body.velocity.y + shake_y,
                (speed * 100.0 / mass) as f32,
            ),
            true,
        );
    }
    if game.host.now() < game.require_entity(&entity).timestamp {
        let frame_seconds = game.host.frame_seconds();
        game.schedule(entity, frame_seconds, nuke_quake as Q2Think);
    } else {
        game.remove_actor(entity);
    }
}

/// Nuke think (`nukeThink`).
fn nuke_think(entity: ActorId, game: &mut Q2GameServices) {
    let damage = game.require_entity(&entity).damage;
    let multiplier = damage / 400.0;
    let divisor = if multiplier == 1.0 {
        1.4
    } else if multiplier == 2.0 {
        2.0
    } else if multiplier == 4.0 {
        3.0
    } else if multiplier == 8.0 {
        5.0
    } else {
        1.0
    };
    let flash = if multiplier == 2.0 {
        37
    } else if multiplier == 4.0 {
        38
    } else if multiplier == 8.0 {
        39
    } else {
        36
    };
    let wait = game.require_entity(&entity).wait;
    if wait < game.host.now() {
        super::mission_projectiles(game).nuke_explode(entity, game);
        return;
    }
    if game.host.now() >= wait - 6.0 {
        {
            let record = game.require_entity_mut(&entity);
            record.frame += 1;
            if record.frame > 11 {
                record.frame = 6;
            }
        }
        let think_origin = game.body_of(entity.clone()).origin;
        if game.host.point_contents(think_origin) & 24 != 0 {
            super::mission_projectiles(game).nuke_explode(entity, game);
            return;
        }
        let owned = game.owned_of(entity.clone());
        game.host.combat().set_health(&owned, 1.0);
        game.require_entity_mut(&entity).owner = None;
        let motion = game.require_entity(&entity).motion;
        game.set_motion_kind(entity.clone(), motion);
        game.show(entity.clone());
        weapon_emit(
            game,
            &Q2WeaponEvent::Muzzleflash {
                actor: entity.clone(),
                flash,
                silenced: false,
            },
        );
        if game.require_entity(&entity).timestamp <= game.host.now() {
            game.sound(&entity, "weapons/nukewarn2.wav", 10, 1.0, 1.8 / divisor);
            let wait = game.require_entity(&entity).wait;
            game.require_entity_mut(&entity).timestamp =
                game.host.now() + if wait - game.host.now() <= 3.0 { 0.3 } else { 0.5 };
        }
        game.schedule(entity, 0.1, nuke_think as Q2Think);
        return;
    }
    if game.require_entity(&entity).timestamp <= game.host.now() {
        game.sound(&entity, "weapons/nukewarn2.wav", 10, 1.0, 1.8 / divisor);
        game.require_entity_mut(&entity).timestamp = game.host.now() + 1.0;
    }
    let frame_seconds = game.host.frame_seconds();
    game.schedule(entity, frame_seconds, nuke_think as Q2Think);
}
