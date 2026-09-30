//! Hipnotic weapons (`src/content/q1/missionpacks/hipnotic-weapons.ts`).

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::contract::{ItemId, ProjectileRole};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::host::{Q1Contents, Q1TrajectoryUpdate};
use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, Q1BeamStyle, Q1Effect, Q1Event, Q1MoveType, Q1Powerup, Q1Solid,
    Q1SoundChannel, Q1TraceRequest, Q1Weapon, POINT, ZERO,
};
use crate::q1::foundation::weapons::aim;
use crate::q1::{q1_error, Q1Error};

use super::types::{
    fround, grenade_velocity, mission_reference, move_missile, set_mission_number, set_mission_reference,
    velocity_angles,
};

/// Finish a Hipnotic weapon firing frame (`finish`).
fn finish(
    game: &mut Q1EntityServices,
    player: &ActorId,
    delay: f64,
    frame: i32,
    model: &str,
    punch: f64,
) -> Result<bool, Q1Error> {
    let time = game.time;
    let attack_finished = fround(time + game.weapon_attack_delay(player, delay)?);
    let hostile_until = fround(time + 1.0);
    game.update_player(player, |state| {
        state.attack_finished = attack_finished;
        state.next_weapon_frame = attack_finished;
        state.weapon_frame = frame;
        state.hostile_until = hostile_until;
    })?;
    game.weapon_punch(player, punch)?;
    let (weapon, actor) = game
        .player_ref(player)
        .map(|state| (state.weapon, state.actor.id().clone()))
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    game.host.emit(Q1Event::Weapon {
        player: actor.clone(),
        weapon,
        view_model: model.to_string(),
        frame,
        punch: punch as i32,
        attack: None,
    });
    if let Some(body) = game.host.bodies.read(&actor) {
        game.effect(Q1Effect::Muzzleflash, body.origin, Some(&actor), 1);
    }
    Ok(true)
}

/// Hipnotic laser damage profile (`HipnoticLaserProfile`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HipnoticLaserProfile {
    /// Firing weapon.
    pub weapon: Q1Weapon,
    /// Direct damage.
    pub damage: f64,
    /// Light-bolt damage.
    pub light_damage: f64,
}

/// Default Hipnotic laser profile (`hipnoticLaser`).
pub const HIPNOTIC_LASER: HipnoticLaserProfile = HipnoticLaserProfile {
    weapon: Q1Weapon::HipnoticLaser,
    damage: 18.0,
    light_damage: 25.0,
};

/// Launch a Hipnotic laser bolt (`launchHipnoticLaser`).
pub fn launch_hipnotic_laser(
    game: &mut Q1EntityServices,
    shooter: &ActorId,
    origin: Vec3,
    direction: Vec3,
    light: bool,
    profile: Option<&HipnoticLaserProfile>,
) -> Result<ActorId, Q1Error> {
    let profile = profile.copied().unwrap_or(HIPNOTIC_LASER);
    let velocity = vscale(normalize(direction), 1000.0);
    let id = game.create("hiplaser", None, None)?;
    let time = game.time;
    let damage = if light { profile.light_damage } else { profile.damage };
    game.update_entity(&id, |laser| {
        laser.owner = Some(shooter.clone());
        laser.activator = Some(shooter.clone());
        laser.movement = Q1MoveType::Flymissile;
        laser.solid = Q1Solid::Bbox;
        laser.model = String::from("progs/lasrspik.mdl");
        laser.effects = if light { 8 } else { 0 };
        laser.speed = 1000.0;
        laser.damage = damage;
        laser.angular_velocity = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 400.0,
        };
        laser.movedir = velocity;
        laser.attack_finished = fround(time + 5.0);
        laser.projectile_weapon = Some(profile.weapon);
    })?;
    let touch = game.named.touch("hipnotic:laser-touch")?;
    game.update_entity(&id, |laser| laser.touch = Some(touch))?;
    game.set_bounds(&id, POINT)?;
    game.set_body(
        &id,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            angles: Some(velocity_angles(velocity)),
            ..Default::default()
        },
    )?;
    game.link(&id)?;
    let think = game.named.action("hipnotic:laser-think")?;
    game.schedule(&id, 0.0, &think)?;
    if let Some(owner) = game.host.actors.resolve_owned(shooter) {
        game.sound(owner.id(), "hipweap/laserg.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    }
    game.launch_projectile_behavior(&id, shooter, profile.weapon, ProjectileRole::Bolt)?;
    Ok(id)
}

/// Fire the Hipnotic laser cannon (`fireHipnoticLaser`).
pub fn fire_hipnotic_laser(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if !game
        .host
        .inventory
        .consume(&state.actor, &ItemId::from("q1:ammo/cells"), 1.0)
    {
        return Ok(false);
    }
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(false),
    };
    let basis = game.make_vectors(state.view_angles);
    let outward = normalize(Vec3 {
        x: basis.forward.x,
        y: basis.forward.y,
        z: 0.0,
    });
    let mut origin = vadd(vadd(body.origin, vscale(basis.up, 6.0)), vscale(outward, 12.0));
    let direction = aim(game, &state.actor, basis.forward);
    let paired = !state.continuous_firing || state.weapon_frame == 4;
    if paired {
        let offset = 6.0 * 0.707;
        origin = vsub(vadd(origin, vscale(basis.right, offset)), vscale(basis.up, offset));
        launch_hipnotic_laser(game, player, origin, direction, false, None)?;
        launch_hipnotic_laser(
            game,
            player,
            vsub(origin, vscale(basis.right, offset * 2.0)),
            direction,
            false,
            None,
        )?;
    } else {
        let light = game.host.random() < 0.1;
        launch_hipnotic_laser(
            game,
            player,
            vadd(origin, vscale(basis.up, 6.0)),
            direction,
            light,
            None,
        )?;
    }
    game.update_player(player, |state| {
        state.continuous_firing = true;
        state.weapon_animation_at = -1.0;
    })?;
    finish(
        game,
        player,
        0.1,
        if paired { 1 } else { 4 },
        "progs/v_laserg.mdl",
        -1.0,
    )
}

/// Laser bolt impact (`laserTouch`).
fn laser_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |laser| {
        laser.owner = None;
        laser.count += 1.0;
    })?;
    let laser = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let origin = game.body(id)?.origin;
    if game.host.contents(origin) == Q1Contents::Sky {
        return game.remove(id);
    }
    let old_direction = normalize(laser.movedir);
    let trace = game.host.trace(&Q1TraceRequest {
        start: vsub(origin, vscale(old_direction, 16.0)),
        end: vadd(origin, vscale(old_direction, 16.0)),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    game.set_origin(id, trace.end)?;
    if game.health(other) != 0.0 {
        let mut damage = laser.damage;
        if laser
            .activator
            .as_ref()
            .is_some_and(|activator| same_actor(activator, other))
        {
            damage = fround(damage / 2.0);
            game.update_entity(id, |laser| laser.damage = damage)?;
        }
        game.effect(Q1Effect::Blood, trace.end, Some(other), damage as i32);
        let params = Q1DamageParams {
            weapon: laser.projectile_weapon,
            ..Default::default()
        };
        let _ = game.damage(other, Some(id), laser.activator.as_ref(), damage, &params);
    } else if laser.count == 3.0 || game.host.random() < 0.15 {
        game.effect(Q1Effect::Gunshot, trace.end, None, 1);
    } else {
        let damage = fround(laser.damage * 0.9);
        let push = if trace.fraction < 1.0 {
            trace.normal
        } else {
            normal.unwrap_or(ZERO)
        };
        let movedir = vscale(normalize(vadd(old_direction, vscale(push, 2.0))), laser.speed);
        game.update_entity(id, |laser| {
            laser.damage = damage;
            laser.movedir = movedir;
        })?;
        move_missile(game, id, movedir)?;
        game.host.random();
        return game.sound(id, "hipweap/laserric.wav", Q1SoundChannel::Weapon, 3.0, 1.0);
    }
    game.sound(id, "enforcer/enfstop.wav", Q1SoundChannel::Weapon, 3.0, 1.0)?;
    game.remove(id)
}

/// Launch a Hipnotic proximity mine (`launchHipnoticProximity`).
pub fn launch_hipnotic_proximity(
    game: &mut Q1EntityServices,
    owner: &ActorId,
    origin: Vec3,
    velocity: Vec3,
) -> Result<ActorId, Q1Error> {
    let id = game.create("proximity_grenade", None, None)?;
    let time = game.time;
    let arming = game.host.random();
    game.update_entity(&id, |mine| {
        mine.owner = Some(owner.clone());
        mine.activator = Some(owner.clone());
        mine.movement = Q1MoveType::Toss;
        mine.solid = Q1Solid::Bbox;
        mine.model = String::from("progs/proxbomb.mdl");
        mine.angular_velocity = Vec3 {
            x: 100.0,
            y: 600.0,
            z: 100.0,
        };
        mine.projectile_weapon = Some(Q1Weapon::HipnoticProximity);
        mine.delay = fround(time + 15.0 + 10.0 * arming);
    })?;
    game.set_damageable(&id, false)?;
    let owned = game
        .entity_ref(&id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.host.combat.set_health(&owned, 5.0)?;
    let touch = game.named.touch("hipnotic:proximity-touch")?;
    let die = game.named.die("hipnotic:proximity-arm-explosion")?;
    game.update_entity(&id, |mine| {
        mine.touch = Some(touch);
        mine.die = Some(die);
    })?;
    game.set_bounds(
        &id,
        Bounds {
            min: Vec3 {
                x: -1.0,
                y: -1.0,
                z: -1.0,
            },
            max: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        },
    )?;
    game.set_body(
        &id,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            angles: Some(velocity_angles(velocity)),
            ..Default::default()
        },
    )?;
    game.link(&id)?;
    let watch = game.named.action("hipnotic:proximity-watch")?;
    game.schedule(&id, 2.0, &watch)?;
    game.launch_projectile_behavior(&id, owner, Q1Weapon::HipnoticProximity, ProjectileRole::Grenade)?;
    Ok(id)
}

/// Fire the Hipnotic proximity gun (`fireHipnoticProximity`).
pub fn fire_hipnotic_proximity(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if !game
        .host
        .inventory
        .consume(&state.actor, &ItemId::from("q1:ammo/rockets"), 1.0)
    {
        return Ok(false);
    }
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(false),
    };
    let basis = game.make_vectors(state.view_angles);
    let aimed = aim(game, &state.actor, basis.forward);
    let velocity = grenade_velocity(game, state.view_angles, aimed);
    launch_hipnotic_proximity(game, player, body.origin, velocity)?;
    game.sound(player, "hipweap/proxbomb.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let time = game.time;
    game.update_player(player, |state| {
        state.continuous_firing = false;
        state.weapon_animation_at = time;
        state.weapon_animation_base = 1;
    })?;
    finish(game, player, 0.6, 1, "progs/v_prox.mdl", -2.0)
}

/// Detonate a proximity mine (`proximityExplode`).
fn proximity_explode(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mine = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let origin = game.body(id)?.origin;
    game.radius_damage(
        id,
        mine.activator.as_ref(),
        95.0,
        None,
        Some(Q1Weapon::HipnoticProximity),
        "",
    );
    game.effect(Q1Effect::Explosion, origin, None, 1);
    game.remove(id)
}

/// Arm a proximity mine explosion (`armProximityExplosion`).
fn arm_proximity_explosion(game: &mut Q1EntityServices, id: &ActorId, delay: f64) -> Result<(), Q1Error> {
    game.set_damageable(id, false)?;
    set_mission_number(game, id, "hipnotic:detonating", 1.0)?;
    let activator = game.entity_ref(id).and_then(|mine| mine.activator.clone());
    game.update_entity(id, |mine| mine.owner = activator)?;
    let explode = game.named.action("hipnotic:proximity-explode")?;
    game.schedule(id, delay, &explode)
}

/// Proximity mine impact (`proximityTouch`).
fn proximity_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let mine = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if same_actor(other, id) || game.host.classname(other) == mine.classname {
        return Ok(());
    }
    game.update_entity(id, |mine| mine.movement = Q1MoveType::Toss)?;
    if mine.count == 1.0 {
        return Ok(());
    }
    let body = game.host.bodies.read(other);
    let moving = body.as_ref().is_some_and(|body| f64::from(length(body.velocity)) > 0.0);
    let aimed = game.entity_ref(other).is_some_and(|entity| entity.aimed_damage);
    if moving || aimed || game.is_player(other) {
        arm_proximity_explosion(game, id, 0.1)?;
        return proximity_explode(game, id);
    }
    game.sound(id, "weapons/bounce.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    game.update_entity(id, |mine| {
        mine.movement = Q1MoveType::None;
        mine.count = 1.0;
    })?;
    set_mission_reference(game, id, "hipnotic:surface", Some(other))?;
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -8.0,
                y: -8.0,
                z: -8.0,
            },
            max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
        },
    )?;
    game.link(id)
}

/// Proximity mine watch think (`proximityWatch`).
fn proximity_watch(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mine = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let mines = game
        .entities
        .values()
        .filter(|entity| entity.classname == mine.classname && entity.number("hipnotic:detonating") == 0.0)
        .count();
    let surface = mission_reference(game, id, "hipnotic:surface");
    let surface_body = surface.as_ref().and_then(|surface| game.host.bodies.read(surface));
    let surface_moving = surface_body
        .as_ref()
        .is_some_and(|body| f64::from(length(body.velocity)) > 0.0);
    if game.time > mine.delay || mines > 15 || surface_moving {
        return proximity_explode(game, id);
    }
    game.update_entity(id, |mine| mine.owner = None)?;
    game.set_damageable(id, true)?;
    let origin = game.body(id)?.origin;
    for observation in game.host.actors.observations().iter().rev() {
        let other = &observation.id;
        let Some(other_body) = game.host.bodies.read(other) else {
            continue;
        };
        let center = vadd(
            other_body.origin,
            vscale(vadd(other_body.bounds.min, other_body.bounds.max), 0.5),
        );
        if f64::from(length(vsub(origin, center))) > 140.0 {
            continue;
        }
        let target = game.entity_ref(other).cloned();
        let target_monster = target.as_ref().is_some_and(|target| target.monster.is_some());
        let target_classname = target.as_ref().map(|target| target.classname.clone());
        let live_enemy = !same_actor(other, id)
            && game.health(other) > 0.0
            && (game.is_player(other) || target_monster)
            && target_classname.as_deref() != Some(mine.classname.as_str());
        let loose_mine = target
            .as_ref()
            .is_some_and(|target| target.classname == mine.classname && target.count == 0.0);
        if !live_enemy && !loose_mine {
            continue;
        }
        let trace = game.host.trace(&Q1TraceRequest {
            start: origin,
            end: other_body.origin,
            bounds: POINT,
            ignore: Some(id.clone()),
            monsters: false,
            missile: false,
        });
        if trace.fraction != 1.0 {
            continue;
        }
        game.sound(id, "hipweap/proxwarn.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        return arm_proximity_explosion(game, id, 0.5);
    }
    let watch = game.named.action("hipnotic:proximity-watch")?;
    game.schedule(id, 0.25, &watch)
}

/// Fire the Hipnotic mjolnir (`fireHipnoticMjolnir`).
pub fn fire_hipnotic_mjolnir(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let strike = game.create("hipnotic_hammer_strike", None, None)?;
    game.update_entity(&strike, |strike| strike.owner = Some(player.clone()))?;
    let action = game.named.action("hipnotic:hammer-strike")?;
    game.schedule(&strike, 0.3, &action)?;
    let time = game.time;
    let cells = game.host.inventory.count(player, &ItemId::from("q1:ammo/cells"));
    game.update_player(player, |state| {
        state.continuous_firing = false;
        state.weapon_animation_at = time;
        state.weapon_animation_base = if cells < 30.0 { 32 } else { 38 };
    })?;
    finish(game, player, 0.8, 1, "progs/v_hammer.mdl", 0.0)
}

/// Apply mjolnir lightning damage across three traces (`hammerDamage`).
fn hammer_damage(
    game: &mut Q1EntityServices,
    from: &ActorId,
    start: Vec3,
    end: Vec3,
    damage: f64,
    weapon: Q1Weapon,
) -> Result<(), Q1Error> {
    let delta = vsub(end, start);
    let side = Vec3 {
        x: -delta.y * 16.0,
        y: -delta.y * 16.0,
        z: 0.0,
    };
    let mut hit: Vec<ActorId> = Vec::new();
    for offset in [ZERO, side, vscale(side, -1.0)] {
        let trace = game.host.trace(&Q1TraceRequest {
            start: vadd(start, offset),
            end: vadd(end, offset),
            bounds: POINT,
            ignore: Some(from.clone()),
            monsters: true,
            missile: false,
        });
        let Some(target) = trace.actor.clone() else {
            continue;
        };
        if hit.iter().any(|actor| same_actor(actor, &target)) {
            continue;
        }
        hit.push(target.clone());
        let can_take = game
            .host
            .combat
            .read(&target)
            .is_some_and(|combat| combat.can_take_damage);
        let wetsuit = game
            .player_ref(&target)
            .and_then(|player| player.powerups.get(&Q1Powerup::HipnoticWetsuit).copied())
            .unwrap_or(0.0);
        if can_take && wetsuit == 0.0 {
            game.host.emit(Q1Event::Particles {
                origin: trace.end,
                direction: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 100.0,
                },
                color: 225,
                count: (damage * 4.0) as i32,
            });
            let params = Q1DamageParams {
                weapon: Some(weapon),
                death_type: String::from("electric"),
                ..Default::default()
            };
            let _ = game.damage(&target, Some(from), Some(from), damage, &params);
        }
    }
    Ok(())
}

/// Resolve a delayed mjolnir strike (`hammerStrike`).
fn hammer_strike(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let strike = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let player = strike.owner.as_ref().and_then(|owner| game.player_ref(owner).cloned());
    let body = player
        .as_ref()
        .and_then(|player| game.host.bodies.read(player.actor.id()));
    let (Some(player), Some(body)) = (player, body) else {
        return game.remove(id);
    };
    let player_id = player.actor.id().clone();
    let basis = game.make_vectors(player.view_angles);
    let source = vadd(
        body.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let mut trace = game.host.trace(&Q1TraceRequest {
        start: source,
        end: vadd(source, vscale(basis.forward, 32.0)),
        bounds: POINT,
        ignore: Some(player_id.clone()),
        monsters: true,
        missile: false,
    });
    let cells = game.host.inventory.count(&player_id, &ItemId::from("q1:ammo/cells"));
    let attack_finished = fround(game.time + game.weapon_attack_delay(&player_id, 0.4)?);
    game.update_player(&player_id, |state| state.attack_finished = attack_finished)?;
    if trace.fraction == 1.0 && cells >= 15.0 {
        let start = vadd(source, vscale(basis.forward, 32.0));
        trace = game.host.trace(&Q1TraceRequest {
            start,
            end: vsub(start, vscale(basis.up, 50.0)),
            bounds: POINT,
            ignore: Some(player_id.clone()),
            monsters: true,
            missile: false,
        });
        if trace.fraction > 0.3 && trace.fraction < 1.0 {
            if player.water_level > 1 {
                game.host
                    .inventory
                    .consume(&player.actor, &ItemId::from("q1:ammo/cells"), cells);
                game.radius_damage(
                    &player_id,
                    Some(&player_id),
                    35.0 * cells,
                    None,
                    Some(Q1Weapon::HipnoticMjolnir),
                    "discharge",
                );
            } else {
                game.host
                    .inventory
                    .consume(&player.actor, &ItemId::from("q1:ammo/cells"), 15.0);
                spawn_hipnotic_hammer_base(game, &player_id, trace.end, Q1Weapon::HipnoticMjolnir)?;
            }
            let attack_finished = fround(game.time + game.weapon_attack_delay(&player_id, 1.5)?);
            game.update_player(&player_id, |state| state.attack_finished = attack_finished)?;
            return game.remove(id);
        }
    }
    let origin = vsub(trace.end, vscale(basis.forward, 4.0));
    if let Some(target) = trace.actor.clone() {
        if game
            .host
            .combat
            .read(&target)
            .is_some_and(|combat| combat.can_take_damage)
        {
            let damage = if game.host.classname(&target) == "monster_zombie" {
                70.0
            } else {
                50.0
            };
            game.effect(Q1Effect::Blood, origin, Some(&target), damage as i32);
            let params = Q1DamageParams {
                weapon: Some(Q1Weapon::HipnoticMjolnir),
                ..Default::default()
            };
            let _ = game.damage(&target, Some(&player_id), Some(&player_id), damage, &params);
            return game.remove(id);
        }
    }
    if trace.fraction != 1.0 {
        game.sound(
            player.actor.id(),
            "hipweap/mjoltink.wav",
            Q1SoundChannel::Weapon,
            1.0,
            1.0,
        )?;
        game.effect(Q1Effect::Gunshot, origin, None, 1);
    } else {
        game.sound(player.actor.id(), "knight/sword1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    }
    game.remove(id)
}

/// Spawn a mjolnir lightning base (`spawnHipnoticHammerBase`).
pub fn spawn_hipnotic_hammer_base(
    game: &mut Q1EntityServices,
    player: &ActorId,
    origin: Vec3,
    weapon: Q1Weapon,
) -> Result<(), Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let base = game.create("hipnotic_mjolnir_base", None, None)?;
    let basis = game.make_vectors(state.view_angles);
    game.update_entity(&base, |base| {
        base.owner = Some(player.clone());
        base.movedir = basis.forward;
        base.projectile_weapon = Some(weapon);
    })?;
    game.set_origin(&base, origin)?;
    game.schedule(&base, 1.0, "SUB_Remove")?;
    game.sound(&base, "hipweap/mjolslap.wav", Q1SoundChannel::Auto, 1.0, 1.0)?;
    game.sound(&base, "hipweap/mjolhit.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    for _ in 0..4 {
        let bolt = game.create("hipnotic_mjolnir_lightning", None, None)?;
        let delay = fround(game.time + 0.8);
        let yaw = state.view_angles.y;
        game.update_entity(&bolt, |bolt| {
            bolt.owner = Some(base.clone());
            bolt.activator = Some(player.clone());
            bolt.projectile_weapon = Some(weapon);
            bolt.delay = delay;
            bolt.mangle = Vec3 { x: 0.0, y: yaw, z: 0.0 };
        })?;
        game.set_origin(&bolt, origin)?;
        let action = game.named.action("hipnotic:hammer-lightning")?;
        game.schedule(&bolt, 0.0, &action)?;
    }
    Ok(())
}

/// Mjolnir lightning think (`hammerLightning`).
fn hammer_lightning(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let bolt = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let base = bolt.owner.as_ref().and_then(|owner| game.entity_ref(owner).cloned());
    let (Some(base), Some(activator)) = (base, bolt.activator.clone()) else {
        return game.remove(id);
    };
    if game.time > bolt.delay {
        return game.remove(id);
    }
    let origin = game.body(base.actor.id())?.origin;
    let old_state = bolt.count;
    let mut target = mission_reference(game, id, "hipnotic:enemy");
    if bolt.count == 0.0 {
        target = None;
        let mut best = 350.0;
        let holders: Vec<(ActorId, String, f64)> = game
            .entities
            .values()
            .map(|entity| (entity.actor.id().clone(), entity.classname.clone(), entity.count))
            .collect();
        for observation in game.host.actors.observations().iter().rev() {
            let actor = &observation.id;
            let body = game.host.bodies.read(actor);
            let entity = game.entity_ref(actor).cloned();
            let Some(body) = body else {
                continue;
            };
            if same_actor(actor, &activator) {
                continue;
            }
            if game.health(actor) <= 0.0 {
                continue;
            }
            if entity.as_ref().map(|entity| entity.movement_flags).unwrap_or(0) & 128 != 0 {
                continue;
            }
            if !(game.is_player(actor) || entity.as_ref().is_some_and(|entity| entity.monster.is_some())) {
                continue;
            }
            let struck = holders
                .iter()
                .filter(|(_, classname, count)| {
                    (classname == "hipnotic_mjolnir_lightning" || classname == "hipnotic_tesla_lightning")
                        && *count == 1.0
                })
                .any(|(holder, _, _)| {
                    mission_reference(game, holder, "hipnotic:enemy")
                        .as_ref()
                        .is_some_and(|struck| same_actor(struck, actor))
                });
            if struck {
                continue;
            }
            let distance = f64::from(length(vsub(body.origin, origin)));
            if distance >= best {
                continue;
            }
            let visible = game.host.trace(&Q1TraceRequest {
                start: origin,
                end: body.origin,
                bounds: POINT,
                ignore: Some(id.clone()),
                monsters: false,
                missile: false,
            });
            if visible.fraction != 1.0 || visible.in_open && visible.in_water {
                continue;
            }
            best = distance;
            target = Some(actor.clone());
        }
        set_mission_reference(game, id, "hipnotic:enemy", target.as_ref())?;
        if target.is_none() {
            let basis = game.make_vectors(bolt.mangle);
            let end = vadd(
                vadd(origin, vscale(basis.forward, 200.0)),
                vscale(basis.right, 400.0 * game.host.random() - 200.0),
            );
            let trace = game.host.trace(&Q1TraceRequest {
                start: origin,
                end,
                bounds: POINT,
                ignore: Some(id.clone()),
                monsters: false,
                missile: false,
            });
            game.host.emit(Q1Event::Beam {
                style: Q1BeamStyle::Lightning2,
                actor: id.clone(),
                start: origin,
                end: trace.end,
            });
            let action = game.named.action("hipnotic:hammer-lightning")?;
            return game.schedule(id, 0.1, &action);
        }
        game.update_entity(id, |bolt| bolt.count = 1.0)?;
    }
    let (Some(target), Some(body)) = (
        target.clone(),
        target.as_ref().and_then(|target| game.host.bodies.read(target)),
    ) else {
        game.update_entity(id, |bolt| bolt.count = 0.0)?;
        let action = game.named.action("hipnotic:hammer-lightning")?;
        return game.schedule(id, 0.1, &action);
    };
    let size = vsub(body.bounds.max, body.bounds.min);
    let end = vadd(
        vadd(body.origin, body.bounds.min),
        vscale(size, 0.25 + game.host.random() * 0.5),
    );
    let trace = game.host.trace(&Q1TraceRequest {
        start: origin,
        end,
        bounds: POINT,
        ignore: bolt.activator.clone(),
        monsters: false,
        missile: false,
    });
    if trace.fraction != 1.0 || game.health(&target) <= 0.0 {
        game.update_entity(id, |bolt| bolt.count = 0.0)?;
        let action = game.named.action("hipnotic:hammer-lightning")?;
        return game.schedule(id, 0.1, &action);
    }
    game.host.emit(Q1Event::Beam {
        style: Q1BeamStyle::Lightning2,
        actor: id.clone(),
        start: origin,
        end: trace.end,
    });
    let damage = if old_state == 0.0 { 80.0 } else { 30.0 };
    let facing = f64::from(dot(normalize(vsub(body.origin, origin)), base.movedir));
    let weapon = bolt.projectile_weapon.unwrap_or(Q1Weapon::HipnoticMjolnir);
    hammer_damage(
        game,
        &activator,
        origin,
        trace.end,
        if facing > 0.3 { damage } else { damage * 0.5 },
        weapon,
    )?;
    let action = game.named.action("hipnotic:hammer-lightning")?;
    game.schedule(id, 0.2, &action)
}

/// Track laser bolts through trajectory updates (`laser-touch` trajectory).
fn laser_trajectory(game: &mut Q1EntityServices, id: &ActorId, update: &Q1TrajectoryUpdate) -> Result<(), Q1Error> {
    game.update_entity(id, |laser| {
        laser.movedir = update.velocity;
        laser.speed = f64::from(length(update.velocity));
    })
}

/// Laser bolt lifetime think (`hipnotic:laser-think`).
fn laser_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let laser = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if game.time > laser.attack_finished {
        return game.remove(id);
    }
    let controlled = game
        .host
        .weapon_behavior
        .as_mut()
        .is_some_and(|behavior| behavior.controls_trajectory(id));
    if !controlled {
        move_missile(game, id, laser.movedir)?;
    }
    let think = game.named.action("hipnotic:laser-think")?;
    game.schedule(id, 0.1, &think)
}

/// Register a named bundle, ignoring repeat registration
/// (donor `WeakSet` guards).
fn register_once(game: &mut Q1EntityServices, name: &str, handlers: Q1CallbackHandlers) -> Result<(), Q1Error> {
    match game.named.register(name, handlers) {
        Ok(()) => Ok(()),
        Err(Q1Error::Message(message)) if message.starts_with("Duplicate Q1 callback") => Ok(()),
        Err(other) => Err(other),
    }
}

/// Register Hipnotic laser callbacks (`registerHipnoticLaserCallbacks`).
pub fn register_hipnotic_laser_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    register_once(
        game,
        "hipnotic:laser-touch",
        Q1CallbackHandlers {
            touch: Some(laser_touch),
            trajectory: Some(laser_trajectory),
            ..Default::default()
        },
    )?;
    register_once(
        game,
        "hipnotic:laser-think",
        Q1CallbackHandlers {
            action: Some(laser_think),
            ..Default::default()
        },
    )
}

/// Arm a proximity mine from its death callback.
fn proximity_arm_die(game: &mut Q1EntityServices, id: &ActorId, _attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    arm_proximity_explosion(game, id, 0.1)
}

/// Register Hipnotic weapon callbacks (`registerHipnoticWeaponCallbacks`).
pub fn register_hipnotic_weapon_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    register_hipnotic_laser_callbacks(game)?;
    game.named.register(
        "hipnotic:proximity-touch",
        Q1CallbackHandlers {
            touch: Some(proximity_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hipnotic:proximity-watch",
        Q1CallbackHandlers {
            action: Some(proximity_watch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hipnotic:proximity-explode",
        Q1CallbackHandlers {
            action: Some(proximity_explode),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hipnotic:proximity-arm-explosion",
        Q1CallbackHandlers {
            die: Some(proximity_arm_die),
            ..Default::default()
        },
    )?;
    register_hipnotic_hammer_callbacks(game)
}

/// Register Hipnotic hammer callbacks (`registerHipnoticHammerCallbacks`).
pub fn register_hipnotic_hammer_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    register_once(
        game,
        "hipnotic:hammer-strike",
        Q1CallbackHandlers {
            action: Some(hammer_strike),
            ..Default::default()
        },
    )?;
    register_once(
        game,
        "hipnotic:hammer-lightning",
        Q1CallbackHandlers {
            action: Some(hammer_lightning),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use qa_core::math::Vec3;

    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::types::{Q1MoveType, Q1Solid, ZERO};

    #[test]
    fn registrations_are_idempotent_where_guarded() {
        let mut game = test_game();
        register_hipnotic_laser_callbacks(&mut game).expect("laser");
        register_hipnotic_laser_callbacks(&mut game).expect("laser again");
        register_hipnotic_hammer_callbacks(&mut game).expect("hammer");
        register_hipnotic_hammer_callbacks(&mut game).expect("hammer again");
        register_hipnotic_weapon_callbacks(&mut game).expect("weapons");
        assert!(game.named.touch("hipnotic:laser-touch").is_ok());
        assert!(game.named.action("hipnotic:proximity-watch").is_ok());
        assert!(game.named.die("hipnotic:proximity-arm-explosion").is_ok());
        assert!(game.named.action("hipnotic:hammer-lightning").is_ok());
    }

    #[test]
    fn laser_launch_matches_donor_profile() {
        let mut game = test_game();
        register_hipnotic_laser_callbacks(&mut game).expect("register");
        let shooter = game.create("player", None, None).expect("shooter");
        let id = launch_hipnotic_laser(&mut game, &shooter, ZERO, Vec3 { x: 1.0, y: 0.0, z: 0.0 }, false, None)
            .expect("launch");
        let laser = game.entity_ref(&id).cloned().expect("laser");
        assert_eq!(laser.model, "progs/lasrspik.mdl");
        assert_eq!(laser.damage, 18.0);
        assert_eq!(laser.speed, 1000.0);
        assert_eq!(laser.effects, 0);
        assert_eq!(laser.movement, Q1MoveType::Flymissile);
        assert_eq!(laser.solid, Q1Solid::Bbox);
        assert_eq!(laser.projectile_weapon, Some(Q1Weapon::HipnoticLaser));
        assert_eq!(laser.touch.as_deref(), Some("hipnotic:laser-touch"));
        let light = launch_hipnotic_laser(&mut game, &shooter, ZERO, Vec3 { x: 1.0, y: 0.0, z: 0.0 }, true, None)
            .expect("light");
        let light = game.entity_ref(&light).cloned().expect("light");
        assert_eq!(light.damage, 25.0);
        assert_eq!(light.effects, 8);
    }

    #[test]
    fn laser_fire_requires_cells() {
        let mut game = test_game();
        register_hipnotic_weapon_callbacks(&mut game).expect("register");
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(
            &owned,
            &crate::q1::foundation::entity_services::Q1AttachOptions::default(),
        )
        .expect("attach");
        assert!(!fire_hipnotic_laser(&mut game, &player).expect("fire"));
        assert!(!fire_hipnotic_proximity(&mut game, &player).expect("fire"));
    }

    #[test]
    fn mjolnir_fire_schedules_strike() {
        let mut game = test_game();
        register_hipnotic_weapon_callbacks(&mut game).expect("register");
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(
            &owned,
            &crate::q1::foundation::entity_services::Q1AttachOptions::default(),
        )
        .expect("attach");
        assert!(fire_hipnotic_mjolnir(&mut game, &player).expect("fire"));
        assert!(game
            .entities
            .values()
            .any(|entity| entity.classname == "hipnotic_hammer_strike"));
    }
}
