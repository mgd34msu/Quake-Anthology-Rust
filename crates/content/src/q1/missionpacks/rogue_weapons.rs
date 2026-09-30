//! Rogue weapons (src/content/q1/missionpacks/rogue-weapons.ts).

use qa_core::identity::{ActorId, same_actor};
use qa_core::math::Vec3;

use crate::contract::{ItemId, ProjectileRole};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, Q1ArmorEffect, TouchSurface};
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{
    POINT, Q1BeamStyle, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest,
    Q1Weapon, ZERO, length, normalize, vadd, vscale, vsub,
};
use crate::q1::foundation::weapons::aim;
use crate::q1::{Q1Error, q1_error};

use super::types::{
    fround, grenade_velocity, mission_reference, move_missile, set_mission_reference,
    velocity_angles,
};

/// Finish a Rogue weapon firing frame (`finish`).
fn finish_rogue_fire(
    game: &mut Q1EntityServices,
    player: &ActorId,
    delay: f64,
    model: &str,
    repeating: bool,
) -> Result<bool, Q1Error> {
    let delay = game.weapon_attack_delay(player, delay)?;
    let frame_delay = if repeating {
        game.weapon_frame_delay(player, 0.1)?
    } else {
        delay
    };
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let time = game.time;
    let weapon = state.weapon;
    let frame = state.weapon_frame;
    game.update_player(player, |state| {
        state.attack_finished = fround(time + delay);
        state.next_weapon_frame = fround(time + frame_delay);
        state.continuous_firing = repeating;
        state.weapon_frame = if repeating {
            frame % 8 + 1
        } else if weapon == Q1Weapon::RoguePlasma {
            0
        } else {
            1
        };
        state.weapon_animation_at = if repeating || weapon == Q1Weapon::RoguePlasma {
            -1.0
        } else {
            time
        };
        state.weapon_animation_base = 1;
        state.hostile_until = fround(time + 1.0);
    })?;
    game.weapon_punch(player, -2.0)?;
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    game.host.emit(Q1Event::Weapon {
        player: state.actor.id().clone(),
        weapon,
        view_model: model.to_string(),
        frame: state.weapon_frame,
        punch: -2,
        attack: None,
    });
    if let Some(body) = game.host.bodies.read(player) {
        game.effect(Q1Effect::Muzzleflash, body.origin, Some(player), 1);
    }
    Ok(true)
}

/// Spawn a Rogue missile (`missile`).
fn spawn_missile(
    game: &mut Q1EntityServices,
    classname: &str,
    owner: &ActorId,
    origin: Vec3,
    velocity: Vec3,
    model: &str,
) -> Result<ActorId, Q1Error> {
    let entity = game.create(classname, None, None)?;
    let owner = owner.clone();
    let model = model.to_string();
    game.update_entity(&entity, |entity| {
        entity.owner = Some(owner);
        entity.solid = Q1Solid::Bbox;
        entity.movement = Q1MoveType::Flymissile;
        entity.model = model;
    })?;
    game.set_bounds(&entity, POINT)?;
    game.set_body(
        &entity,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            angles: Some(velocity_angles(velocity)),
            ..Default::default()
        },
    )?;
    game.link(&entity)?;
    Ok(entity)
}

/// Whether a touch should be ignored (`ignoreTouch`).
fn ignore_touch(game: &Q1EntityServices, entity: &ActorId, other: &ActorId) -> bool {
    game.entity_ref(entity)
        .and_then(|entity| entity.owner.clone())
        .is_some_and(|owner| same_actor(&owner, other))
        || game
            .entity_ref(other)
            .is_some_and(|entity| entity.solid == Q1Solid::Trigger)
}

/// Launch a Rogue lava spike (`launchRogueLavaSpike`).
pub fn launch_rogue_lava_spike(
    game: &mut Q1EntityServices,
    owner: &ActorId,
    origin: Vec3,
    direction: Vec3,
    powered: bool,
) -> Result<ActorId, Q1Error> {
    let entity = spawn_missile(
        game,
        "lava_spike",
        owner,
        origin,
        vscale(direction, 1000.0),
        "progs/lspike.mdl",
    )?;
    let weapon = if powered {
        Q1Weapon::RogueLavaSupernailgun
    } else {
        Q1Weapon::RogueLavaNailgun
    };
    game.update_entity(&entity, |entity| {
        entity.count = if powered { 1.0 } else { 0.0 };
        entity.projectile_weapon = Some(weapon);
    })?;
    let touch = game.named.touch("rogue:lava-touch")?;
    game.update_entity(&entity, |entity| entity.touch = Some(touch))?;
    game.schedule(&entity, 6.0, "SUB_Remove")?;
    game.launch_projectile_behavior(&entity, owner, weapon, ProjectileRole::Nail)?;
    Ok(entity)
}

/// Fire Rogue lava nailguns (`fireRogueLava`).
pub fn fire_rogue_lava(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let nails = game
        .host
        .inventory
        .count(player, &ItemId::from("rogue:ammo/lava-nails"));
    let powered = state.weapon == Q1Weapon::RogueLavaSupernailgun && nails >= 2.0;
    if !game.host.inventory.consume(
        &state.actor,
        &ItemId::from("rogue:ammo/lava-nails"),
        if powered { 2.0 } else { 1.0 },
    ) {
        return Ok(false);
    }
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(false),
    };
    let basis = game.make_vectors(state.view_angles);
    let origin = vadd(
        vadd(
            body.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 16.0,
            },
        ),
        vscale(
            basis.right,
            if powered { 0.0 } else { state.nail_side * 4.0 },
        ),
    );
    game.sound(
        player,
        if powered {
            "weapons/spike2.wav"
        } else {
            "weapons/rocket1i.wav"
        },
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    launch_rogue_lava_spike(
        game,
        player,
        origin,
        aim(game, &state.actor, basis.forward),
        powered,
    )?;
    game.update_player(player, |state| state.nail_side *= -1.0)?;
    finish_rogue_fire(
        game,
        player,
        0.2,
        if state.weapon == Q1Weapon::RogueLavaSupernailgun {
            "progs/v_lava2.mdl"
        } else {
            "progs/v_lava.mdl"
        },
        true,
    )
}

/// Lava spike impact (`lavaTouch`).
fn lava_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if ignore_touch(game, id, other) {
        return Ok(());
    }
    let origin = game.body(id)?.origin;
    if game.host.contents(origin) == Q1Contents::Sky {
        return game.remove(id);
    }
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let powered = entity.count == 1.0;
    if game
        .host
        .combat
        .read(other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        game.effect(
            Q1Effect::Blood,
            origin,
            Some(other),
            if powered { 18 } else { 9 },
        );
        if game.host.classname(other) != "monster_lava_man" {
            let player = game.is_player(other);
            let damage = if player {
                if powered { 18.0 } else { 9.0 }
            } else if powered {
                30.0
            } else {
                15.0
            };
            let params = Q1DamageParams {
                weapon: entity.projectile_weapon,
                death_type: String::from(if powered {
                    "rogue:super-lava"
                } else {
                    "rogue:lava"
                }),
                armor_effect: if player {
                    Some(if powered {
                        Q1ArmorEffect::HalfEffectiveness
                    } else {
                        Q1ArmorEffect::Bypass
                    })
                } else {
                    None
                },
                ..Default::default()
            };
            let _ = game.damage(other, Some(id), entity.owner.as_ref(), damage, &params);
        }
    } else {
        game.effect(
            if powered {
                Q1Effect::Superspike
            } else {
                Q1Effect::Spike
            },
            origin,
            None,
            1,
        );
    }
    game.remove(id)
}

/// Launch a Rogue multi grenade (`launchRogueMultiGrenade`).
pub fn launch_rogue_multi_grenade(
    game: &mut Q1EntityServices,
    owner: &ActorId,
    origin: Vec3,
    velocity: Vec3,
    angles: Vec3,
) -> Result<ActorId, Q1Error> {
    let grenade = spawn_missile(
        game,
        "MultiGrenade",
        owner,
        origin,
        velocity,
        "progs/mervup.mdl",
    )?;
    game.update_entity(&grenade, |grenade| {
        grenade.movement = Q1MoveType::Bounce;
        grenade.mangle = angles;
        grenade.angular_velocity = Vec3 {
            x: 300.0,
            y: 300.0,
            z: 300.0,
        };
        grenade.projectile_weapon = Some(Q1Weapon::RogueMultiGrenade);
    })?;
    let touch = game.named.touch("rogue:multi-grenade-touch")?;
    game.update_entity(&grenade, |grenade| grenade.touch = Some(touch))?;
    let split = game.named.action("rogue:multi-grenade-split")?;
    game.schedule(&grenade, 1.0, &split)?;
    game.launch_projectile_behavior(
        &grenade,
        owner,
        Q1Weapon::RogueMultiGrenade,
        ProjectileRole::Grenade,
    )?;
    Ok(grenade)
}

/// Fire Rogue multi grenades (`fireRogueMultiGrenade`).
pub fn fire_rogue_multi_grenade(
    game: &mut Q1EntityServices,
    player: &ActorId,
) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if !game
        .host
        .inventory
        .consume(&state.actor, &ItemId::from("rogue:ammo/multi-rockets"), 1.0)
    {
        return Ok(false);
    }
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(false),
    };
    let forward = game.make_vectors(state.view_angles).forward;
    let velocity = grenade_velocity(game, state.view_angles, aim(game, &state.actor, forward));
    launch_rogue_multi_grenade(game, player, body.origin, velocity, ZERO)?;
    game.sound(
        player,
        "weapons/grenade.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    finish_rogue_fire(game, player, 0.6, "progs/v_multi.mdl", false)
}

/// Detonate a multi grenade (`grenadeExplode`).
fn grenade_explode(
    game: &mut Q1EntityServices,
    grenade: &ActorId,
    mini: bool,
) -> Result<(), Q1Error> {
    let owner = game
        .entity_ref(grenade)
        .and_then(|grenade| grenade.owner.clone());
    let player = owner.as_ref().is_some_and(|owner| game.is_player(owner));
    game.radius_damage(
        grenade,
        owner.as_ref(),
        if mini {
            if player { 90.0 } else { 60.0 }
        } else {
            120.0
        },
        None,
        Some(Q1Weapon::RogueMultiGrenade),
        "",
    );
    game.effect(Q1Effect::Explosion, game.body(grenade)?.origin, None, 1);
    game.remove(grenade)
}

/// Split a multi grenade (`splitGrenade`).
fn split_grenade(game: &mut Q1EntityServices, grenade: &ActorId) -> Result<(), Q1Error> {
    let record = game
        .entity_ref(grenade)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let owner = match record.owner.clone() {
        Some(owner) => owner,
        None => return game.remove(grenade),
    };
    for offset in [0.0, 72.0, 144.0, 216.0, 288.0] {
        let mangle = record.mangle;
        let angles = Vec3 {
            x: mangle.x,
            y: mangle.y + offset,
            z: mangle.z,
        };
        let basis = game.make_vectors(angles);
        let mut velocity = vadd(vscale(basis.forward, 100.0), vscale(basis.up, 400.0));
        velocity = vadd(
            velocity,
            vscale(
                basis.forward,
                (game.host.random() * 2.0 - 1.0) * 60.0 - 30.0,
            ),
        );
        velocity = vadd(
            velocity,
            vscale(basis.right, (game.host.random() * 2.0 - 1.0) * 40.0 - 20.0),
        );
        velocity = vadd(
            velocity,
            vscale(basis.up, (game.host.random() * 2.0 - 1.0) * 60.0 - 30.0),
        );
        let mini = spawn_missile(
            game,
            "MiniGrenade",
            &owner,
            game.body(grenade)?.origin,
            velocity,
            "progs/mervup.mdl",
        )?;
        game.update_entity(&mini, |mini| {
            mini.movement = Q1MoveType::Bounce;
            mini.mangle = angles;
            mini.angular_velocity = Vec3 {
                x: 300.0,
                y: 300.0,
                z: 300.0,
            };
            mini.projectile_weapon = Some(Q1Weapon::RogueMultiGrenade);
        })?;
        let touch = game.named.touch("rogue:multi-grenade-touch")?;
        game.update_entity(&mini, |mini| mini.touch = Some(touch))?;
        game.launch_projectile_behavior(
            &mini,
            &owner,
            Q1Weapon::RogueMultiGrenade,
            ProjectileRole::Grenade,
        )?;
        let explode = game.named.action("rogue:mini-grenade-explode")?;
        game.schedule(
            &mini,
            1.0 + (game.host.random() * 2.0 - 1.0) * 0.5,
            &explode,
        )?;
    }
    game.remove(grenade)
}

/// Multi grenade impact (`multiGrenadeTouch`).
fn multi_grenade_touch(
    game: &mut Q1EntityServices,
    grenade: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let record = game
        .entity_ref(grenade)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if record
        .owner
        .as_ref()
        .is_some_and(|owner| same_actor(owner, other))
    {
        return Ok(());
    }
    if game
        .entity_ref(other)
        .is_some_and(|entity| entity.aimed_damage)
        || game.is_player(other)
    {
        let mini = record.classname == "MiniGrenade"
            || record.owner.is_none()
            || record
                .owner
                .as_ref()
                .is_some_and(|owner| !game.is_player(owner));
        return grenade_explode(game, grenade, mini);
    }
    game.sound(
        grenade,
        "weapons/bounce.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    if game.body(grenade)?.velocity == ZERO {
        game.update_entity(grenade, |grenade| grenade.angular_velocity = ZERO)?;
    }
    Ok(())
}

/// Detonate a multi rocket (`explodeRocket`).
fn explode_rocket(
    game: &mut Q1EntityServices,
    rocket: &ActorId,
    direct: Option<&ActorId>,
) -> Result<(), Q1Error> {
    if let Some(direct) = direct {
        if game.health(direct) != 0.0 {
            let mut damage = 60.0 + game.host.random() * 15.0;
            if game.host.classname(direct) == "monster_shambler"
                || game.host.classname(direct) == "monster_dragon"
            {
                damage *= 0.5;
            }
            let params = Q1DamageParams {
                weapon: Some(Q1Weapon::RogueMultiRocket),
                ..Default::default()
            };
            let owner = game
                .entity_ref(rocket)
                .and_then(|rocket| rocket.owner.clone());
            let _ = game.damage(direct, Some(rocket), owner.as_ref(), damage, &params);
        }
    }
    let owner = game
        .entity_ref(rocket)
        .and_then(|rocket| rocket.owner.clone());
    game.radius_damage(
        rocket,
        owner.as_ref(),
        75.0,
        direct,
        Some(Q1Weapon::RogueMultiRocket),
        "",
    );
    let body = game.body(rocket)?;
    game.effect(
        Q1Effect::Explosion,
        vsub(body.origin, vscale(normalize(body.velocity), 8.0)),
        None,
        1,
    );
    game.remove(rocket)
}

/// Acquire a homing target (`acquireRocket`).
fn acquire_rocket(game: &mut Q1EntityServices, rocket: &ActorId) -> Result<(), Q1Error> {
    let record = game
        .entity_ref(rocket)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if record.delay < game.time {
        return explode_rocket(game, rocket, None);
    }
    let body = game.body(rocket)?;
    let owner = record
        .owner
        .as_ref()
        .and_then(|owner| game.host.actors.resolve_owned(owner));
    let direction = match owner {
        None => normalize(body.velocity),
        Some(_) => {
            let owned = game
                .entity_ref(rocket)
                .map(|rocket| rocket.actor.clone())
                .ok_or_else(|| q1_error("Missing Q1 entity"))?;
            aim(game, &owned, game.make_vectors(record.mangle).forward)
        }
    };
    let trace = game.host.trace(&Q1TraceRequest {
        start: body.origin,
        end: vadd(body.origin, vscale(direction, 1000.0)),
        bounds: POINT,
        ignore: Some(rocket.clone()),
        monsters: true,
        missile: false,
    });
    if trace.actor.as_ref().is_some_and(|actor| {
        game.entity_ref(actor)
            .is_some_and(|entity| entity.monster.is_some())
    }) {
        let target = trace.actor.clone().expect("target");
        set_mission_reference(game, rocket, "rogue:enemy", Some(&target))?;
        return home_rocket(game, rocket);
    }
    game.update_entity(rocket, |rocket| {
        rocket.mangle = velocity_angles(body.velocity)
    })?;
    let acquire = game.named.action("rogue:rocket-acquire")?;
    game.schedule(rocket, 0.2, &acquire)
}

/// Home a multi rocket (`homeRocket`).
fn home_rocket(game: &mut Q1EntityServices, rocket: &ActorId) -> Result<(), Q1Error> {
    let target = mission_reference(game, rocket, "rogue:enemy");
    let body = target
        .as_ref()
        .and_then(|target| game.host.bodies.read(target));
    match (target, body) {
        (Some(target), Some(body)) if game.health(&target) >= 1.0 => {
            let controlled = game
                .host
                .weapon_behavior
                .as_mut()
                .is_some_and(|behavior| behavior.controls_trajectory(rocket));
            if !controlled {
                move_missile(
                    game,
                    rocket,
                    vscale(
                        normalize(vsub(body.origin, game.body(rocket)?.origin)),
                        1000.0,
                    ),
                )?;
            }
        }
        _ => return game.remove(rocket),
    }
    let home = game.named.action("rogue:rocket-home")?;
    game.schedule(rocket, 0.1, &home)
}

/// Rocket-explode action (`rogue:rocket-explode`).
fn rocket_explode(game: &mut Q1EntityServices, rocket: &ActorId) -> Result<(), Q1Error> {
    explode_rocket(game, rocket, None)
}

/// Mini grenade explode action (`rogue:mini-grenade-explode`).
fn mini_grenade_explode(game: &mut Q1EntityServices, grenade: &ActorId) -> Result<(), Q1Error> {
    grenade_explode(game, grenade, true)
}

/// Multi rocket impact (`rogue:rocket-touch`).
fn rocket_touch(
    game: &mut Q1EntityServices,
    rocket: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game
        .entity_ref(rocket)
        .and_then(|rocket| rocket.owner.clone())
        .is_some_and(|owner| same_actor(&owner, other))
    {
        return Ok(());
    }
    if game.host.contents(game.body(rocket)?.origin) == Q1Contents::Sky {
        return game.remove(rocket);
    }
    explode_rocket(game, rocket, Some(other))
}

/// Fire Rogue multi rockets (`fireRogueMultiRocket`).
pub fn fire_rogue_multi_rocket(
    game: &mut Q1EntityServices,
    player: &ActorId,
) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if !game
        .host
        .inventory
        .consume(&state.actor, &ItemId::from("rogue:ammo/multi-rockets"), 1.0)
    {
        return Ok(false);
    }
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(false),
    };
    game.make_vectors(state.view_angles);
    let coop = game.options().coop;
    let deathmatch = game.options().deathmatch;
    let multiplayer = coop || deathmatch != 0;
    for shot in [(-10.0, 2), (-5.0, 3), (5.0, 0), (10.0, 1)] {
        // The donor reads the saved basis before refreshing it per shot.
        let origin = vadd(
            vadd(body.origin, vscale(game.basis.forward, 8.0)),
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 16.0,
            },
        );
        let angles = Vec3 {
            x: state.view_angles.x,
            y: state.view_angles.y + shot.0 * 0.66,
            z: state.view_angles.z,
        };
        let basis = game.make_vectors(if multiplayer {
            angles
        } else {
            state.view_angles
        });
        let velocity = if multiplayer {
            vscale(aim(game, &state.actor, basis.forward), 1000.0)
        } else {
            vsub(
                vscale(basis.forward, 1000.0),
                vscale(basis.right, shot.0 * 8.0),
            )
        };
        let rocket = spawn_missile(
            game,
            "MultiRocket",
            player,
            origin,
            velocity,
            if multiplayer {
                "progs/rockup_d.mdl"
            } else {
                "progs/rockup.mdl"
            },
        )?;
        let delay = fround(game.time + 4.0);
        let mangle = state.view_angles;
        game.update_entity(&rocket, |rocket| {
            rocket.projectile_weapon = Some(Q1Weapon::RogueMultiRocket);
            rocket.frame = shot.1;
            rocket.delay = delay;
            rocket.mangle = mangle;
        })?;
        let touch = game.named.touch("rogue:rocket-touch")?;
        game.update_entity(&rocket, |rocket| rocket.touch = Some(touch))?;
        game.launch_projectile_behavior(
            &rocket,
            player,
            Q1Weapon::RogueMultiRocket,
            ProjectileRole::Rocket,
        )?;
        if multiplayer {
            let explode = game.named.action("rogue:rocket-explode")?;
            game.schedule(&rocket, 4.0, &explode)?;
        } else {
            let trace = game.host.trace(&Q1TraceRequest {
                start: origin,
                end: vadd(origin, velocity),
                bounds: POINT,
                ignore: Some(player.clone()),
                monsters: true,
                missile: false,
            });
            if trace.actor.as_ref().is_some_and(|actor| {
                game.entity_ref(actor)
                    .is_some_and(|entity| entity.monster.is_some())
            }) {
                // Both source editions return here without assigning nextthink.
                let target = trace.actor.clone().expect("target");
                set_mission_reference(game, &rocket, "rogue:enemy", Some(&target))?;
                let home = game.named.action("rogue:rocket-home")?;
                game.update_entity(&rocket, |rocket| rocket.think = Some(home))?;
            } else {
                let acquire = game.named.action("rogue:rocket-acquire")?;
                game.schedule(&rocket, 0.1, &acquire)?;
            }
        }
    }
    game.sound(
        player,
        "weapons/sgun1.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    finish_rogue_fire(game, player, 0.8, "progs/v_multi2.mdl", false)
}

/// Launch a Rogue plasma ball (`launchRoguePlasma`).
pub fn launch_rogue_plasma(
    game: &mut Q1EntityServices,
    owner: &ActorId,
    origin: Vec3,
    direction: Vec3,
) -> Result<ActorId, Q1Error> {
    let plasma = spawn_missile(
        game,
        "plasma",
        owner,
        origin,
        vscale(direction, 0.01),
        "progs/plasma.mdl",
    )?;
    let coop = game.options().coop;
    let deathmatch = game.options().deathmatch;
    game.update_entity(&plasma, |plasma| {
        plasma.angular_velocity = Vec3 {
            x: 300.0,
            y: 300.0,
            z: 300.0,
        };
        plasma.projectile_weapon = Some(Q1Weapon::RoguePlasma);
        if !coop && deathmatch == 0 {
            plasma.effects = 4;
        }
    })?;
    let touch = game.named.touch("rogue:plasma-touch")?;
    game.update_entity(&plasma, |plasma| plasma.touch = Some(touch))?;
    game.sound(
        &plasma,
        "plasma/flight.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    let launch = game.named.action("rogue:plasma-launch")?;
    game.schedule(&plasma, 0.1, &launch)?;
    game.launch_projectile_behavior(
        &plasma,
        owner,
        Q1Weapon::RoguePlasma,
        ProjectileRole::Plasma,
    )?;
    Ok(plasma)
}

/// Fire the Rogue plasma gun (`fireRoguePlasma`).
pub fn fire_rogue_plasma(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let ammo = game
        .host
        .inventory
        .count(player, &ItemId::from("rogue:ammo/plasma"));
    if ammo < 1.0 {
        return Ok(false);
    }
    if state.water_level > 1 {
        game.host
            .inventory
            .consume(&state.actor, &ItemId::from("rogue:ammo/plasma"), ammo);
        game.radius_damage(
            player,
            Some(player),
            35.0 * ammo,
            None,
            Some(Q1Weapon::RoguePlasma),
            "",
        );
    } else {
        let body = match game.host.bodies.read(player) {
            Some(body) => body,
            None => return Ok(false),
        };
        game.host
            .inventory
            .consume(&state.actor, &ItemId::from("rogue:ammo/plasma"), 1.0);
        let basis = game.make_vectors(state.view_angles);
        let origin = vadd(
            vadd(body.origin, vscale(basis.forward, 24.0)),
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 16.0,
            },
        );
        launch_rogue_plasma(game, player, origin, aim(game, &state.actor, basis.forward))?;
        game.host.emit(Q1Event::Sound {
            origin: None,
            actor: player.clone(),
            path: String::from("plasma/fire.wav"),
            channel: Q1SoundChannel::Weapon,
            attenuation: 1.0,
            volume: 0.5,
        });
    }
    finish_rogue_fire(game, player, 1.0, "progs/v_plasma.mdl", false)
}

/// Apply plasma lightning damage across three traces (`plasmaDamage`).
fn plasma_damage(game: &mut Q1EntityServices, plasma: &ActorId, end: Vec3) -> Result<(), Q1Error> {
    let origin = game.body(plasma)?.origin;
    let delta = vsub(end, origin);
    let side = Vec3 {
        x: -delta.y * 16.0,
        y: -delta.y * 16.0,
        z: 0.0,
    };
    let mut hit: Vec<ActorId> = Vec::new();
    for offset in [ZERO, side, vscale(side, -1.0)] {
        let trace = game.host.trace(&Q1TraceRequest {
            start: vadd(origin, offset),
            end: vadd(end, offset),
            bounds: POINT,
            ignore: Some(plasma.clone()),
            monsters: true,
            missile: false,
        });
        let Some(other) = trace.actor.clone() else {
            continue;
        };
        if hit.iter().any(|actor| same_actor(actor, &other)) {
            continue;
        }
        hit.push(other.clone());
        if game
            .host
            .combat
            .read(&other)
            .is_some_and(|combat| combat.can_take_damage)
        {
            game.host.emit(Q1Event::Particles {
                origin: trace.end,
                direction: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 100.0,
                },
                color: 225,
                count: 200,
            });
            let params = Q1DamageParams {
                weapon: Some(Q1Weapon::RoguePlasma),
                ..Default::default()
            };
            let owner = game
                .entity_ref(plasma)
                .and_then(|plasma| plasma.owner.clone());
            let _ = game.damage(&other, Some(plasma), owner.as_ref(), 50.0, &params);
        }
    }
    Ok(())
}

/// Plasma ball impact (`plasmaTouch`).
fn plasma_touch(
    game: &mut Q1EntityServices,
    plasma: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game
        .entity_ref(plasma)
        .and_then(|plasma| plasma.owner.clone())
        .is_some_and(|owner| same_actor(&owner, other))
    {
        return Ok(());
    }
    let origin = game.body(plasma)?.origin;
    if game.host.contents(origin) == Q1Contents::Sky {
        return game.remove(plasma);
    }
    let mut damage = 80.0 + game.host.random() * 20.0;
    game.sound(
        &plasma.clone(),
        "plasma/explode.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    if game.health(other) != 0.0 {
        if game.host.classname(other) == "monster_shambler" {
            damage *= 0.5;
        }
        let params = Q1DamageParams {
            weapon: Some(Q1Weapon::RoguePlasma),
            ..Default::default()
        };
        let owner = game
            .entity_ref(plasma)
            .and_then(|plasma| plasma.owner.clone());
        let _ = game.damage(other, Some(plasma), owner.as_ref(), damage, &params);
    }
    let owner = game
        .entity_ref(plasma)
        .and_then(|plasma| plasma.owner.clone());
    game.radius_damage(
        plasma,
        owner.as_ref(),
        70.0,
        Some(other),
        Some(Q1Weapon::RoguePlasma),
        "",
    );
    game.effect(Q1Effect::Explosion, origin, None, 1);
    let mut count = 0;
    for observation in game.host.actors.observations().iter().rev() {
        let target = observation.id.clone();
        let body = game.host.bodies.read(&target);
        let entity = game.entity_ref(&target).cloned();
        let Some(body) = body else {
            continue;
        };
        if owner
            .as_ref()
            .is_some_and(|owner| same_actor(owner, &target))
        {
            continue;
        }
        if !(game.is_player(&target)
            || entity
                .as_ref()
                .is_some_and(|entity| entity.monster.is_some()))
        {
            continue;
        }
        let center = vadd(
            body.origin,
            vscale(vadd(body.bounds.min, body.bounds.max), 0.5),
        );
        if f64::from(length(vsub(center, origin))) > 320.0 {
            continue;
        }
        let trace = game.host.trace(&Q1TraceRequest {
            start: origin,
            end: body.origin,
            bounds: POINT,
            ignore: None,
            monsters: false,
            missile: false,
        });
        if trace.fraction != 1.0 {
            continue;
        }
        game.host.emit(Q1Event::Beam {
            style: Q1BeamStyle::Lightning2,
            actor: target.clone(),
            start: body.origin,
            end: origin,
        });
        game.sound(
            &plasma.clone(),
            "weapons/lhit.wav",
            Q1SoundChannel::Voice,
            1.0,
            1.0,
        )?;
        plasma_damage(game, plasma, body.origin)?;
        count += 1;
        if count == 5 {
            break;
        }
    }
    game.remove(plasma)
}

/// Launch a plasma ball to full speed (`rogue:plasma-launch`).
fn plasma_launch(game: &mut Q1EntityServices, plasma: &ActorId) -> Result<(), Q1Error> {
    let controlled = game
        .host
        .weapon_behavior
        .as_mut()
        .is_some_and(|behavior| behavior.controls_trajectory(plasma));
    if !controlled {
        move_missile(
            game,
            plasma,
            vscale(normalize(game.body(plasma)?.velocity), 1250.0),
        )?;
    }
    game.schedule(plasma, 5.0, "SUB_Remove")
}

/// Register Rogue weapon callbacks (`registerRogueWeaponCallbacks`).
pub fn register_rogue_weapon_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "rogue:lava-touch",
        Q1CallbackHandlers {
            touch: Some(lava_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:multi-grenade-touch",
        Q1CallbackHandlers {
            touch: Some(multi_grenade_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:multi-grenade-split",
        Q1CallbackHandlers {
            action: Some(split_grenade),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:mini-grenade-explode",
        Q1CallbackHandlers {
            action: Some(mini_grenade_explode),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:rocket-explode",
        Q1CallbackHandlers {
            action: Some(rocket_explode),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:rocket-acquire",
        Q1CallbackHandlers {
            action: Some(acquire_rocket),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:rocket-home",
        Q1CallbackHandlers {
            action: Some(home_rocket),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:rocket-touch",
        Q1CallbackHandlers {
            touch: Some(rocket_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plasma-touch",
        Q1CallbackHandlers {
            touch: Some(plasma_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plasma-launch",
        Q1CallbackHandlers {
            action: Some(plasma_launch),
            ..Default::default()
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;
    use crate::q1::foundation::types::{Q1MoveType, Q1Solid};

    fn attached_player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default())
            .expect("attach");
        player
    }

    #[test]
    fn callbacks_register() {
        let mut game = test_game();
        register_rogue_weapon_callbacks(&mut game).expect("register");
        assert!(game.named.touch("rogue:lava-touch").is_ok());
        assert!(game.named.action("rogue:multi-grenade-split").is_ok());
        assert!(game.named.action("rogue:rocket-acquire").is_ok());
        assert!(game.named.action("rogue:rocket-home").is_ok());
        assert!(game.named.touch("rogue:plasma-touch").is_ok());
        assert!(game.named.action("rogue:plasma-launch").is_ok());
    }

    #[test]
    fn lava_launch_matches_donor() {
        let mut game = test_game();
        register_rogue_weapon_callbacks(&mut game).expect("register");
        let shooter = game.create("player", None, None).expect("shooter");
        let id = launch_rogue_lava_spike(
            &mut game,
            &shooter,
            ZERO,
            Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            true,
        )
        .expect("launch");
        let spike = game.entity_ref(&id).cloned().expect("spike");
        assert_eq!(spike.model, "progs/lspike.mdl");
        assert_eq!(spike.count, 1.0);
        assert_eq!(spike.movement, Q1MoveType::Flymissile);
        assert_eq!(spike.solid, Q1Solid::Bbox);
        assert_eq!(
            spike.projectile_weapon,
            Some(Q1Weapon::RogueLavaSupernailgun)
        );
        assert_eq!(spike.touch.as_deref(), Some("rogue:lava-touch"));
    }

    #[test]
    fn fire_requires_ammo() {
        let mut game = test_game();
        register_rogue_weapon_callbacks(&mut game).expect("register");
        let player = attached_player(&mut game);
        assert!(!fire_rogue_lava(&mut game, &player).expect("lava"));
        assert!(!fire_rogue_multi_grenade(&mut game, &player).expect("grenade"));
        assert!(!fire_rogue_multi_rocket(&mut game, &player).expect("rocket"));
        assert!(!fire_rogue_plasma(&mut game, &player).expect("plasma"));
    }

    #[test]
    fn plasma_fire_consumes_and_launches() {
        let mut game = test_game();
        register_rogue_weapon_callbacks(&mut game).expect("register");
        let player = attached_player(&mut game);
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.host
            .inventory
            .give(&owned, &ItemId::from("rogue:ammo/plasma"), 5.0);
        assert!(fire_rogue_plasma(&mut game, &player).expect("fire"));
        assert_eq!(
            game.host
                .inventory
                .count(&player, &ItemId::from("rogue:ammo/plasma")),
            4.0
        );
        assert!(
            game.entities
                .values()
                .any(|entity| entity.classname == "plasma")
        );
    }
}
