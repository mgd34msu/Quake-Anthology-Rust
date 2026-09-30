//! Q1 weapon behavior (`src/content/q1/foundation/weapons.ts`).
//!
//! weapons.qc/player.qc, Copyright (C) 1996-2022 id Software LLC.
//! GPL-2.0-or-later.
//!
//! Aim selection ports `aimQ1` (donor
//! `src/world/gameplay/q1-aim.ts`) over game-provided eligibility,
//! bodies, and traces.

use qa_core::identity::{same_actor, ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::contract::ItemId;

use super::entity::Q1ProjectileKind;
use super::entity_services::{ammo_item, Q1DamageParams, Q1EntityServices, Q1WeaponPurpose};
use super::gameplay::TouchSurface;
use super::types::{
    dot, is_q1_base_weapon, length, normalize, vadd, vscale, vsub, Q1CharacterAttack, Q1Effect, Q1Event, Q1MoveType,
    Q1Solid, Q1SoundChannel, Q1TraceRequest, Q1Weapon, POINT, ZERO,
};
use crate::q1::{q1_error, Q1Error};

/// Default best-weapon order (donor `bestWeapon` fallback; the
/// launchers never win automatically).
const BEST_WEAPON_ORDER: [Q1Weapon; 6] = [
    Q1Weapon::Lightning,
    Q1Weapon::Supernailgun,
    Q1Weapon::Supershotgun,
    Q1Weapon::Nailgun,
    Q1Weapon::Shotgun,
    Q1Weapon::Axe,
];

/// Base weapon view model (`weaponModel`).
pub fn weapon_model(weapon: Q1Weapon) -> Result<String, Q1Error> {
    if !is_q1_base_weapon(weapon) {
        return Err(q1_error(format!(
            "Use registered Q1 weapon model for {}",
            weapon.as_str()
        )));
    }
    Ok(String::from(match weapon {
        Q1Weapon::Axe => "progs/v_axe.mdl",
        Q1Weapon::Shotgun => "progs/v_shot.mdl",
        Q1Weapon::Supershotgun => "progs/v_shot2.mdl",
        Q1Weapon::Nailgun => "progs/v_nail.mdl",
        Q1Weapon::Supernailgun => "progs/v_nail2.mdl",
        Q1Weapon::Grenadelauncher => "progs/v_rock.mdl",
        Q1Weapon::Rocketlauncher => "progs/v_rock2.mdl",
        Q1Weapon::Lightning => "progs/v_light.mdl",
        _ => unreachable!("base weapon checked above"),
    }))
}

/// Best available weapon (`bestWeapon`).
pub fn best_weapon(
    game: &mut Q1EntityServices,
    actor: &OwnedActor,
    ammo_count: Option<&dyn Fn(&ItemId) -> f64>,
) -> Result<Q1Weapon, Q1Error> {
    let order = game.weapon_order.clone().unwrap_or_else(|| BEST_WEAPON_ORDER.to_vec());
    if game.player_ref(actor.id()).is_none() {
        return Ok(Q1Weapon::Axe);
    }
    for weapon in order {
        if game.weapon_available(actor.id(), weapon, Q1WeaponPurpose::Best, ammo_count)? {
            return Ok(weapon);
        }
    }
    Ok(Q1Weapon::Axe)
}

/// PF_aim selection with donor `aimQ1` semantics.
pub fn aim(game: &mut Q1EntityServices, actor: &OwnedActor, forward: Vec3) -> Vec3 {
    let body = match game.host.bodies.read(actor.id()) {
        Some(body) => body,
        None => return forward,
    };
    let team = game.host.combat.read(actor.id()).and_then(|combat| combat.team.clone());
    let threshold = game.options().aim_threshold.unwrap_or(0.93);
    let origin = body.origin;
    let start = Vec3 {
        x: origin.x,
        y: origin.y,
        z: origin.z + 20.0,
    };
    let far = vadd(start, vscale(forward, 2048.0));
    let straight = game.host.trace(&Q1TraceRequest {
        start,
        end: far,
        bounds: POINT,
        ignore: Some(actor.id().clone()),
        monsters: true,
        missile: false,
    });
    if let Some(hit) = straight.actor.as_ref() {
        if aim_eligible(game, actor, team.as_deref(), hit) {
            return forward;
        }
    }
    let targets: Vec<ActorId> = game
        .host
        .actors
        .observations()
        .into_iter()
        .map(|observation| observation.id)
        .collect();
    let mut best = threshold;
    let mut selected: Option<Vec3> = None;
    for target in &targets {
        if !aim_eligible(game, actor, team.as_deref(), target) {
            continue;
        }
        let body = match game.host.bodies.read(target) {
            Some(body) => body,
            None => continue,
        };
        let center = Vec3 {
            x: body.origin.x + 0.5 * (body.bounds.min.x + body.bounds.max.x),
            y: body.origin.y + 0.5 * (body.bounds.min.y + body.bounds.max.y),
            z: body.origin.z + 0.5 * (body.bounds.min.z + body.bounds.max.z),
        };
        let delta = vsub(center, start);
        let size = length(delta);
        if size == 0.0 {
            continue;
        }
        let direction = Vec3 {
            x: delta.x / size,
            y: delta.y / size,
            z: delta.z / size,
        };
        let alignment = f64::from(dot(direction, forward));
        if alignment < best {
            continue;
        }
        let hit = game.host.trace(&Q1TraceRequest {
            start,
            end: center,
            bounds: POINT,
            ignore: Some(actor.id().clone()),
            monsters: true,
            missile: false,
        });
        if hit.actor.as_ref().is_some_and(|hit| same_actor(hit, target)) {
            best = alignment;
            selected = Some(body.origin);
        }
    }
    let selected = match selected {
        Some(selected) => selected,
        None => return forward,
    };
    let delta = vsub(selected, origin);
    let along = dot(delta, forward);
    let mut corrected = Vec3 {
        x: forward.x * along,
        y: forward.y * along,
        z: delta.z,
    };
    let size = length(corrected);
    if size != 0.0 {
        corrected = Vec3 {
            x: corrected.x / size,
            y: corrected.y / size,
            z: corrected.z / size,
        };
    }
    corrected
}

fn aim_eligible(game: &mut Q1EntityServices, actor: &OwnedActor, team: Option<&str>, target: &ActorId) -> bool {
    let traits = game.source_target(target);
    let state = game.host.combat.read(target);
    let teamplay = game.options().teamplay.unwrap_or(0);
    match state {
        Some(state) => {
            !same_actor(target, actor.id())
                && (traits.aimed_damage || traits.player)
                && state.can_take_damage
                && !(teamplay != 0 && team.is_some() && team == state.team.as_deref())
        }
        None => false,
    }
}

/// MultiDamage-style pellet spread (`fireBullets`). Damage flushes
/// when the next pellet changes target, preserving intervening
/// reactions.
#[allow(clippy::too_many_arguments)]
pub fn fire_bullets(
    game: &mut Q1EntityServices,
    shooter: &OwnedActor,
    direction: Vec3,
    view_angles: Vec3,
    count: i32,
    spread_x: f64,
    spread_y: f64,
    weapon: Option<Q1Weapon>,
) {
    let body = match game.host.bodies.read(shooter.id()) {
        Some(body) => body,
        None => return,
    };
    let basis = game.make_vectors(view_angles);
    let source = Vec3 {
        x: body.origin.x + basis.forward.x * 10.0,
        y: body.origin.y + basis.forward.y * 10.0,
        z: (f64::from(body.origin.z)
            + f64::from(body.bounds.min.z)
            + (f64::from(body.bounds.max.z) - f64::from(body.bounds.min.z)) * 0.7) as f32,
    };
    let mut pending: Option<ActorId> = None;
    let mut total = 0.0;
    let flush = |game: &mut Q1EntityServices, pending: &mut Option<ActorId>, total: &mut f64| {
        if let Some(target) = pending.take() {
            if game.host.actors.is_live(&target) {
                game.damage(
                    &target,
                    Some(shooter.id()),
                    Some(shooter.id()),
                    *total,
                    &Q1DamageParams {
                        weapon,
                        ..Default::default()
                    },
                );
            }
        }
        *total = 0.0;
    };
    for _ in 0..count {
        let basis = game.basis;
        let rx = game.host.random() * 2.0 - 1.0;
        let ry = game.host.random() * 2.0 - 1.0;
        let ray = vadd(
            vadd(direction, vscale(basis.right, rx * spread_x)),
            vscale(basis.up, ry * spread_y),
        );
        let trace = game.host.trace(&Q1TraceRequest {
            start: source,
            end: vadd(source, vscale(ray, 2048.0)),
            bounds: POINT,
            ignore: Some(shooter.id().clone()),
            monsters: true,
            missile: false,
        });
        if trace.fraction == 1.0 {
            continue;
        }
        let trigger = trace.actor.as_ref().is_some_and(|actor| {
            game.entity_ref(actor)
                .is_some_and(|entity| entity.solid == Q1Solid::Trigger)
        });
        if !trace.sky && (trace.actor.is_none() || !trigger) {
            if let Some(impact) = game.host.weapon_impact.as_mut() {
                impact(shooter.id(), trace.end);
            }
        }
        let damageable = trace.actor.as_ref().is_some_and(|actor| {
            game.host
                .combat
                .read(actor)
                .is_some_and(|combat| combat.can_take_damage)
        });
        if trace.actor.is_some() && damageable {
            let target = trace.actor.clone().expect("target");
            let origin = vsub(trace.end, vscale(ray, 4.0));
            game.effect(Q1Effect::Blood, origin, Some(&target), 4);
            if pending.as_ref().is_none_or(|pending| !same_actor(pending, &target)) {
                flush(game, &mut pending, &mut total);
                pending = Some(target);
            }
            total += 4.0;
        } else {
            let origin = vsub(trace.end, vscale(ray, 4.0));
            let actor = trace.actor.clone();
            game.effect(Q1Effect::Gunshot, origin, actor.as_ref(), 1);
        }
    }
    flush(game, &mut pending, &mut total);
}

fn spawn_projectile(
    game: &mut Q1EntityServices,
    player: &ActorId,
    kind: Q1ProjectileKind,
    velocity: Vec3,
    origin: Vec3,
) -> Result<ActorId, Q1Error> {
    let classname = match kind {
        Q1ProjectileKind::Rocket => "missile",
        Q1ProjectileKind::Grenade => "grenade",
        Q1ProjectileKind::Spike => "spike",
        Q1ProjectileKind::Superspike => "superspike",
    };
    let states = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let id = game.create(classname, None, None)?;
    game.update_entity(&id, |entity| {
        entity.projectile = Some(kind);
        entity.projectile_weapon = Some(states.weapon);
        entity.owner = Some(player.clone());
        entity.movement = if kind == Q1ProjectileKind::Grenade {
            Q1MoveType::Bounce
        } else {
            Q1MoveType::Flymissile
        };
        entity.solid = super::types::Q1Solid::Bbox;
        entity.model = String::from(match kind {
            Q1ProjectileKind::Rocket => "progs/missile.mdl",
            Q1ProjectileKind::Grenade => "progs/grenade.mdl",
            Q1ProjectileKind::Superspike => "progs/s_spike.mdl",
            Q1ProjectileKind::Spike => "progs/spike.mdl",
        });
        if kind == Q1ProjectileKind::Grenade {
            entity.angular_velocity = Vec3 {
                x: 300.0,
                y: 300.0,
                z: 300.0,
            };
        }
    })?;
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            ..Default::default()
        },
    )?;
    game.link(&id)?;
    let touch = game.named.touch("projectile_touch")?;
    game.update_entity(&id, |entity| entity.touch = Some(touch))?;
    let lifetime = match kind {
        Q1ProjectileKind::Grenade => 2.5,
        Q1ProjectileKind::Rocket => 5.0,
        _ => 6.0,
    };
    let removal = match kind {
        Q1ProjectileKind::Grenade => "GrenadeExplode",
        _ => "SUB_Remove",
    };
    game.schedule(&id, lifetime, removal)?;
    let role = match kind {
        Q1ProjectileKind::Rocket => crate::contract::ProjectileRole::Rocket,
        Q1ProjectileKind::Grenade => crate::contract::ProjectileRole::Grenade,
        _ => crate::contract::ProjectileRole::Nail,
    };
    game.launch_projectile_behavior(&id, player, states.weapon, role)?;
    Ok(id)
}

/// Projectile impact dispatch (`projectileTouch`).
pub fn projectile_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    _normal: Vec3,
    surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if other.is_some_and(|other| entity.owner.as_ref().is_some_and(|owner| same_actor(other, owner))) {
        return Ok(());
    }
    if other.is_some_and(|other| {
        game.entity_ref(other)
            .is_some_and(|entity| entity.solid == Q1Solid::Trigger)
    }) {
        return Ok(());
    }
    if entity.projectile != Some(Q1ProjectileKind::Grenade)
        && (surface.is_some_and(|surface| surface.native_flags & 4 != 0)
            || game.host.contents(game.body(&id).map(|body| body.origin)?) == super::host::Q1Contents::Sky)
    {
        return game.remove(&id);
    }
    match entity.projectile {
        Some(Q1ProjectileKind::Rocket) => explode(game, &id, other, true),
        Some(Q1ProjectileKind::Grenade) => {
            if let Some(other) = other {
                let traits = game.source_target(other);
                if traits.aimed_damage || traits.player {
                    return explode(game, &id, None, false);
                }
            }
            game.sound(&id, "weapons/bounce.wav", Q1SoundChannel::Weapon, 1.0, 1.0)
        }
        Some(Q1ProjectileKind::Spike) | Some(Q1ProjectileKind::Superspike) => {
            let owner = entity.owner.clone();
            let origin = game.body(&id).map(|body| body.origin)?;
            if let (Some(owner), Some(impact)) = (owner.as_ref(), game.host.weapon_impact.as_mut()) {
                impact(owner, origin);
            }
            let amount = if entity.projectile == Some(Q1ProjectileKind::Spike) {
                9.0
            } else {
                18.0
            };
            let damageable = other.is_some_and(|other| {
                game.host
                    .combat
                    .read(other)
                    .is_some_and(|combat| combat.can_take_damage)
            });
            if let Some(other) = other {
                if damageable {
                    let origin = game.body(&id).map(|body| body.origin)?;
                    game.effect(Q1Effect::Blood, origin, Some(other), amount as i32);
                    game.damage(
                        other,
                        Some(&id),
                        owner.as_ref(),
                        amount,
                        &Q1DamageParams {
                            weapon: entity.projectile_weapon,
                            ..Default::default()
                        },
                    );
                } else {
                    let effect = if entity.classname == "wizard_spike" {
                        Q1Effect::WizardSpike
                    } else if entity.classname == "knight_spike" {
                        Q1Effect::KnightSpike
                    } else if entity.projectile == Some(Q1ProjectileKind::Superspike) {
                        Q1Effect::Superspike
                    } else {
                        Q1Effect::Spike
                    };
                    let origin = game.body(&id).map(|body| body.origin)?;
                    game.effect(effect, origin, None, 1);
                }
            } else {
                let effect = if entity.classname == "wizard_spike" {
                    Q1Effect::WizardSpike
                } else if entity.classname == "knight_spike" {
                    Q1Effect::KnightSpike
                } else if entity.projectile == Some(Q1ProjectileKind::Superspike) {
                    Q1Effect::Superspike
                } else {
                    Q1Effect::Spike
                };
                let origin = game.body(&id).map(|body| body.origin)?;
                game.effect(effect, origin, None, 1);
            }
            game.remove(&id)
        }
        None => Ok(()),
    }
}

fn explode(game: &mut Q1EntityServices, id: &ActorId, direct: Option<&ActorId>, rocket: bool) -> Result<(), Q1Error> {
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let impact_origin = game.body(id).map(|body| body.origin)?;
    if let (Some(owner), Some(impact)) = (entity.owner.as_ref(), game.host.weapon_impact.as_mut()) {
        impact(owner, impact_origin);
    }
    if rocket && direct.is_some_and(|direct| game.health(direct) != 0.0) {
        let direct = direct.expect("direct");
        let mut amount = f64::from((100.0 + game.host.random() * 20.0) as f32);
        if game.host.classname(direct) == "monster_shambler" {
            amount *= 0.5;
        }
        game.damage(
            direct,
            Some(id),
            entity.owner.as_ref(),
            amount,
            &Q1DamageParams {
                weapon: entity.projectile_weapon,
                ..Default::default()
            },
        );
    }
    game.radius_damage(id, entity.owner.as_ref(), 120.0, direct, entity.projectile_weapon, "");
    let origin = if rocket {
        let body = game.body(id)?;
        vsub(body.origin, vscale(normalize(body.velocity), 8.0))
    } else {
        game.body(id).map(|body| body.origin)?
    };
    game.effect_simple(Q1Effect::Explosion, origin);
    // BecomeExplosion in the network Quake source removes the missile
    // after emitting TE_EXPLOSION.
    game.remove(id)
}

fn lightning(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(()),
    };
    let cells = game.host.inventory.count(player, &String::from("q1:ammo/cells"));
    let water_level = game.player_ref(player).map(|player| player.water_level).unwrap_or(0);
    if water_level > 1 {
        if let Some(impact) = game.host.weapon_impact.as_mut() {
            impact(player, body.origin);
        }
        game.consume_weapon_ammo(player, &String::from("q1:ammo/cells"), cells)?;
        game.radius_damage(
            player,
            Some(player),
            35.0 * cells,
            None,
            Some(Q1Weapon::Lightning),
            "discharge",
        );
        return Ok(());
    }
    game.consume_weapon_ammo(player, &String::from("q1:ammo/cells"), 1.0)?;
    let view_angles = game.player_ref(player).map(|player| player.view_angles).unwrap_or(ZERO);
    let forward = game.make_vectors(view_angles).forward;
    let start = vadd(
        body.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let wall = game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(start, vscale(forward, 600.0)),
        bounds: POINT,
        ignore: Some(player.clone()),
        monsters: false,
        missile: false,
    });
    if wall.fraction < 1.0 && !wall.sky {
        if let Some(impact) = game.host.weapon_impact.as_mut() {
            impact(player, wall.end);
        }
    }
    game.host.emit(Q1Event::Beam {
        style: super::types::Q1BeamStyle::Lightning2,
        actor: player.clone(),
        start,
        end: wall.end,
    });
    let end = vadd(wall.end, vscale(forward, 4.0));
    // Preserve the source's discarded normalize return and sequential
    // x/y assignments.
    let delta = vsub(end, body.origin);
    let side = Vec3 {
        x: -delta.y * 16.0,
        y: -delta.y * 16.0,
        z: 0.0,
    };
    let mut hit: Vec<ActorId> = Vec::new();
    for offset in [ZERO, side, vscale(side, -1.0)] {
        let trace = game.host.trace(&Q1TraceRequest {
            start: vadd(body.origin, offset),
            end: vadd(end, offset),
            bounds: POINT,
            ignore: Some(player.clone()),
            monsters: true,
            missile: false,
        });
        let target = trace.actor.clone();
        let fresh = target
            .as_ref()
            .is_some_and(|target| !hit.iter().any(|actor| same_actor(actor, target)));
        let damageable = target.as_ref().is_some_and(|target| {
            game.host
                .combat
                .read(target)
                .is_some_and(|combat| combat.can_take_damage)
        });
        if let Some(target) = target.as_ref() {
            if !fresh || !damageable {
                continue;
            }
            let trigger = game
                .entity_ref(target)
                .is_some_and(|entity| entity.solid == Q1Solid::Trigger);
            if !trace.sky && !trigger {
                if let Some(impact) = game.host.weapon_impact.as_mut() {
                    impact(player, trace.end);
                }
            }
            hit.push(target.clone());
            game.effect(Q1Effect::Blood, trace.end, Some(target), 120);
            game.damage(
                target,
                Some(player),
                Some(player),
                30.0,
                &Q1DamageParams {
                    weapon: Some(Q1Weapon::Lightning),
                    ..Default::default()
                },
            );
        }
    }
    Ok(())
}

/// Fire the player's weapon through overrides or the base attack
/// (`fireWeapon`).
pub fn fire_weapon(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let states = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if states.primary_holstered {
        return Ok(false);
    }
    if game.registered_weapons.contains_key(&states.weapon) || !is_q1_base_weapon(states.weapon) {
        return game.fire_registered_weapon(player);
    }
    fire_base_weapon(game, player)
}

/// An overriding source definition can delegate its unmodified attack
/// without reentering its own registration (`fireBaseWeapon`).
pub fn fire_base_weapon(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let player_id = player.clone();
    let states = game
        .player_ref(&player_id)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if states.primary_holstered {
        return Ok(false);
    }
    if !is_q1_base_weapon(states.weapon) {
        return Err(q1_error("Base Q1 attack requires a base weapon"));
    }
    let repeating = states.continuous_firing;
    if game.health(&player_id) <= 0.0
        || game.time
            < if repeating {
                states.next_weapon_frame
            } else {
                states.attack_finished
            }
    {
        return Ok(false);
    }
    if let Some(ammo) = ammo_item(states.weapon) {
        if game.host.inventory.count(&player_id, &ammo) < 1.0 {
            let best = best_weapon(game, &states.actor, None)?;
            game.select_weapon(&states.actor, best)?;
            return Ok(false);
        }
    }
    let body = match game.host.bodies.read(&player_id) {
        Some(body) => body,
        None => return Ok(false),
    };
    let volume = match game.host.weapon_volume.as_mut() {
        Some(volume) => volume(&player_id),
        None => 1.0,
    };
    if !game.registered_weapons.contains_key(&states.weapon) {
        game.weapon_before_fire(&player_id)?;
    }
    let basis = game.make_vectors(states.view_angles);
    let weapon = states.weapon;
    let mut delay = 0.1;
    let mut punch = -2;
    let mut attack = Q1CharacterAttack::Rocket;
    match weapon {
        Q1Weapon::Shotgun | Q1Weapon::Supershotgun => attack = Q1CharacterAttack::Shotgun,
        Q1Weapon::Lightning => attack = Q1CharacterAttack::Lightning,
        Q1Weapon::Nailgun | Q1Weapon::Supernailgun => attack = Q1CharacterAttack::Nail,
        _ => {}
    }
    let continuous = matches!(weapon, Q1Weapon::Nailgun | Q1Weapon::Supernailgun | Q1Weapon::Lightning);
    game.update_player(&player_id, |player| player.continuous_firing = continuous)?;
    let next_frame = f64::from((game.time + game.weapon_frame_delay(&player_id, 0.1)?) as f32);
    game.update_player(&player_id, |player| player.next_weapon_frame = next_frame)?;
    if !continuous {
        let time = game.time;
        game.update_player(&player_id, |player| {
            player.weapon_animation_at = time;
            player.weapon_animation_base = 1;
        })?;
    }
    let hostile_until = game.time + 1.0;
    game.update_player(&player_id, |player| player.hostile_until = hostile_until)?;
    match weapon {
        Q1Weapon::Axe => {
            delay = 0.5;
            punch = 0;
            game.sound(&player_id, "weapons/ax1.wav", Q1SoundChannel::Weapon, 1.0, volume)?;
            let animation = game.host.random();
            let variant = if animation < 0.25 {
                0
            } else if animation < 0.5 {
                1
            } else if animation < 0.75 {
                2
            } else {
                3
            };
            attack = Q1CharacterAttack::Axe { variant };
            let base = if (0.25..0.5).contains(&animation) || animation >= 0.75 {
                5
            } else {
                1
            };
            game.update_player(&player_id, |player| player.weapon_animation_base = base)?;
            // player_axe3 is the hit frame, two 0.1 second animation
            // steps after attack begins.
            let strike = game.create("axe_strike", None, None)?;
            game.update_entity(&strike, |entity| entity.owner = Some(player_id.clone()))?;
            game.schedule(&strike, 0.2, "player_axe3")?;
        }
        Q1Weapon::Shotgun | Q1Weapon::Supershotgun => {
            let super_shot = weapon == Q1Weapon::Supershotgun
                && game.host.inventory.count(&player_id, &String::from("q1:ammo/shells")) > 1.0;
            game.consume_weapon_ammo(
                &player_id,
                &String::from("q1:ammo/shells"),
                if super_shot { 2.0 } else { 1.0 },
            )?;
            delay = if weapon == Q1Weapon::Supershotgun { 0.7 } else { 0.5 };
            punch = if super_shot { -4 } else { -2 };
            game.sound(
                &player_id,
                if super_shot {
                    "weapons/shotgn2.wav"
                } else {
                    "weapons/guncock.wav"
                },
                Q1SoundChannel::Weapon,
                1.0,
                volume,
            )?;
            let owned = game.player_owned(&player_id).expect("player");
            let view_angles = game
                .player_ref(&player_id)
                .map(|player| player.view_angles)
                .unwrap_or(ZERO);
            let direction = aim(game, &owned, basis.forward);
            fire_bullets(
                game,
                &owned,
                direction,
                view_angles,
                if super_shot { 14 } else { 6 },
                if super_shot { 0.14 } else { 0.04 },
                if super_shot { 0.08 } else { 0.04 },
                Some(weapon),
            );
        }
        Q1Weapon::Nailgun | Q1Weapon::Supernailgun => {
            let super_nail = weapon == Q1Weapon::Supernailgun
                && game.host.inventory.count(&player_id, &String::from("q1:ammo/nails")) >= 2.0;
            delay = 0.2;
            game.consume_weapon_ammo(
                &player_id,
                &String::from("q1:ammo/nails"),
                if super_nail { 2.0 } else { 1.0 },
            )?;
            game.sound(
                &player_id,
                if super_nail {
                    "weapons/spike2.wav"
                } else {
                    "weapons/rocket1i.wav"
                },
                Q1SoundChannel::Weapon,
                1.0,
                volume,
            )?;
            let owned = game.player_owned(&player_id).expect("player");
            let nail_side = game
                .player_ref(&player_id)
                .map(|player| player.nail_side)
                .unwrap_or(1.0);
            let origin = vadd(
                vadd(
                    body.origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 16.0,
                    },
                ),
                vscale(basis.right, if super_nail { 0.0 } else { nail_side * 4.0 }),
            );
            let speed = game.nail_speed(&player_id, 1000.0)?;
            let direction = aim(game, &owned, basis.forward);
            spawn_projectile(
                game,
                &player_id,
                if super_nail {
                    Q1ProjectileKind::Superspike
                } else {
                    Q1ProjectileKind::Spike
                },
                vscale(direction, speed),
                origin,
            )?;
            game.update_player(&player_id, |player| player.nail_side *= -1.0)?;
        }
        Q1Weapon::Grenadelauncher => {
            game.consume_weapon_ammo(&player_id, &String::from("q1:ammo/rockets"), 1.0)?;
            delay = 0.6;
            game.sound(&player_id, "weapons/grenade.wav", Q1SoundChannel::Weapon, 1.0, volume)?;
            let view_angles = game
                .player_ref(&player_id)
                .map(|player| player.view_angles)
                .unwrap_or(ZERO);
            let velocity = if view_angles.x == 0.0 {
                let owned = game.player_owned(&player_id).expect("player");
                let mut velocity = vscale(aim(game, &owned, basis.forward), 600.0);
                velocity.z = 200.0;
                velocity
            } else {
                let rx = game.host.random() * 2.0 - 1.0;
                let ry = game.host.random() * 2.0 - 1.0;
                vadd(
                    vadd(vscale(basis.forward, 600.0), vscale(basis.up, 200.0)),
                    vadd(vscale(basis.right, rx * 10.0), vscale(basis.up, ry * 10.0)),
                )
            };
            spawn_projectile(game, &player_id, Q1ProjectileKind::Grenade, velocity, body.origin)?;
        }
        Q1Weapon::Rocketlauncher => {
            game.consume_weapon_ammo(&player_id, &String::from("q1:ammo/rockets"), 1.0)?;
            delay = 0.8;
            game.sound(&player_id, "weapons/sgun1.wav", Q1SoundChannel::Weapon, 1.0, volume)?;
            let owned = game.player_owned(&player_id).expect("player");
            let direction = aim(game, &owned, basis.forward);
            spawn_projectile(
                game,
                &player_id,
                Q1ProjectileKind::Rocket,
                vscale(direction, 1000.0),
                vadd(
                    vadd(body.origin, vscale(basis.forward, 8.0)),
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 16.0,
                    },
                ),
            )?;
        }
        Q1Weapon::Lightning => {
            delay = if repeating { 0.2 } else { 0.1 };
            let lightning_sound_at = game
                .player_ref(&player_id)
                .map(|player| player.lightning_sound_at)
                .unwrap_or(0.0);
            if lightning_sound_at < game.time {
                game.sound(&player_id, "weapons/lhit.wav", Q1SoundChannel::Weapon, 1.0, volume)?;
                let time = game.time;
                game.update_player(&player_id, |player| player.lightning_sound_at = time + 0.6)?;
            }
            lightning(game, &player_id)?;
            if !repeating {
                game.sound(&player_id, "weapons/lstart.wav", Q1SoundChannel::Auto, 1.0, volume)?;
            }
        }
        _ => {}
    }
    let attack_finished = f64::from((game.time + game.weapon_attack_delay(&player_id, delay)?) as f32);
    let continuous_firing = game
        .player_ref(&player_id)
        .map(|player| player.continuous_firing)
        .unwrap_or(false);
    let weapon_frame_current = game
        .player_ref(&player_id)
        .map(|player| player.weapon_frame)
        .unwrap_or(0);
    let weapon_animation_base = game
        .player_ref(&player_id)
        .map(|player| player.weapon_animation_base)
        .unwrap_or(1);
    game.update_player(&player_id, |player| {
        player.attack_finished = attack_finished;
        player.weapon_frame = if continuous_firing {
            weapon_frame_current % if weapon == Q1Weapon::Lightning { 4 } else { 8 } + 1
        } else {
            weapon_animation_base
        };
    })?;
    game.weapon_punch(&player_id, punch as f64)?;
    let view_model = game.weapon_model(weapon, Some(&player_id))?;
    let weapon_frame = game
        .player_ref(&player_id)
        .map(|player| player.weapon_frame)
        .unwrap_or(0);
    game.host.emit(Q1Event::Weapon {
        player: player_id.clone(),
        weapon,
        view_model,
        frame: weapon_frame,
        punch,
        attack: Some(attack),
    });
    game.effect(Q1Effect::Muzzleflash, body.origin, Some(&player_id), 1);
    Ok(true)
}

fn axe_strike(game: &mut Q1EntityServices, strike: &ActorId) -> Result<(), Q1Error> {
    let strike = strike.clone();
    let owner = game.entity_ref(&strike).and_then(|entity| entity.owner.clone());
    let player = owner
        .as_ref()
        .and_then(|owner| game.player_ref(owner).map(|player| player.actor.clone()));
    let Some(player) = player else {
        return game.remove(&strike);
    };
    let current = match game.host.bodies.read(player.id()) {
        Some(current) => current,
        None => return game.remove(&strike),
    };
    let view_angles = game
        .player_ref(player.id())
        .map(|player| player.view_angles)
        .unwrap_or(ZERO);
    let start = vadd(
        current.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let forward = game.make_vectors(view_angles).forward;
    let trace = game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(start, vscale(forward, 64.0)),
        bounds: POINT,
        ignore: Some(player.id().clone()),
        monsters: true,
        missile: false,
    });
    if trace.fraction < 1.0 {
        let trigger = trace.actor.as_ref().is_some_and(|actor| {
            game.entity_ref(actor)
                .is_some_and(|entity| entity.solid == Q1Solid::Trigger)
        });
        if !trace.sky && (trace.actor.is_none() || !trigger) {
            if let Some(impact) = game.host.weapon_impact.as_mut() {
                impact(player.id(), trace.end);
            }
        }
        let damageable = trace.actor.as_ref().is_some_and(|actor| {
            game.host
                .combat
                .read(actor)
                .is_some_and(|combat| combat.can_take_damage)
        });
        if trace.actor.is_some() && damageable {
            let target = trace.actor.clone().expect("target");
            game.effect(Q1Effect::Blood, trace.end, Some(&target), 20);
            game.damage(
                &target,
                Some(player.id()),
                Some(player.id()),
                20.0,
                &Q1DamageParams {
                    weapon: Some(Q1Weapon::Axe),
                    ..Default::default()
                },
            );
        } else {
            game.sound(player.id(), "player/axhit2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
            game.effect(Q1Effect::Gunshot, trace.end, None, 3);
        }
    }
    game.remove(&strike)
}

/// Register weapon callbacks.
pub fn register_weapon_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "projectile_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 normal: Option<Vec3>,
                 surface: Option<&TouchSurface>| {
                    projectile_touch(game, id, Some(other), normal.unwrap_or(ZERO), surface)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "GrenadeExplode",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| explode(game, id, None, false)),
            ..Default::default()
        },
    )?;
    game.named.register(
        "player_axe3",
        super::callbacks::Q1CallbackHandlers {
            action: Some(axe_strike),
            ..Default::default()
        },
    )?;
    Ok(())
}
