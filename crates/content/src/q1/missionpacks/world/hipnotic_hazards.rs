//! Hipnotic mines, lightning, tesla, and gravity hazards
//! (`src/content/q1/missionpacks/world/hipnotic-hazards.ts`).
//!
//! hipitems.qc hazards.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, DamageDelivery, TouchSurface};
use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, Q1BeamStyle, Q1Effect, Q1Event, Q1MoveType, Q1Powerup, Q1Solid,
    Q1TraceRequest, POINT, ZERO,
};
use crate::q1::{q1_error, Q1Error};

use super::common::{later, number, vector};

/// Whether a mjolnir/tesla bolt currently tracks an actor (`isStruckByMjolnir`).
pub fn is_struck_by_mjolnir(game: &Q1EntityServices, actor: &ActorId) -> bool {
    game.entities.values().any(|entity| {
        (entity.classname == "hipnotic_mjolnir_lightning" || entity.classname == "hipnotic_tesla_lightning")
            && entity.count == 1.0
            && entity
                .references
                .get("hipnotic:enemy")
                .cloned()
                .flatten()
                .is_some_and(|enemy| same_actor(actor, &enemy))
    })
}

/// Whether a hazard has line of sight to an actor (`visible`).
fn visible(game: &mut Q1EntityServices, id: &ActorId, actor: &ActorId) -> bool {
    let body = game.host.bodies.read(actor);
    let Some(body) = body else {
        return false;
    };
    let origin = game.body(id).map(|body| body.origin);
    let view_ofs = game.entity(id).map(|entity| entity.vector("view_ofs")).unwrap_or(ZERO);
    let Ok(origin) = origin else {
        return false;
    };
    let eye = if game.is_player(actor) {
        22.0
    } else {
        game.entity(actor)
            .map(|target| target.vector("view_ofs").z)
            .unwrap_or(0.0)
    };
    let trace = game.host.trace(&Q1TraceRequest {
        start: vadd(origin, view_ofs),
        end: vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: eye }),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    trace.fraction == 1.0 && !(trace.in_open && trace.in_water)
}

/// Scan for damageable actors in radius (`scan`).
fn scan(game: &mut Q1EntityServices, id: &ActorId, radius: f64, include_monsters: bool) -> Vec<ActorId> {
    let Ok(origin) = game.body(id).map(|body| body.origin) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for observation in game.host.actors.observations().iter().rev() {
        let actor = observation.id.clone();
        let flags = game.entity(&actor).map(|target| target.movement_flags).unwrap_or(0);
        let body = game.host.bodies.read(&actor);
        let Some(body) = body else {
            continue;
        };
        if flags & 128 != 0 {
            continue;
        }
        if !game.is_player(&actor) && (!include_monsters || flags & 32 == 0) {
            continue;
        }
        let center = vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5));
        if f64::from(length(vsub(center, origin))) <= radius && game.health(&actor) > 0.0 && visible(game, id, &actor) {
            found.push(actor);
        }
    }
    found
}

/// Damage actors along a lightning bolt (`lightningDamage`).
fn lightning_damage(
    game: &mut Q1EntityServices,
    id: &ActorId,
    start: Vec3,
    end: Vec3,
    from: Option<&ActorId>,
    amount: f64,
) -> Result<(), Q1Error> {
    let inflictor = from.cloned().or_else(|| game.world.clone());
    let Some(inflictor) = inflictor else {
        return Err(q1_error("Lightning damage requires worldspawn"));
    };
    let side = Vec3 {
        x: -(end.y - start.y) * 16.0,
        y: -(end.y - start.y) * 16.0,
        z: 0.0,
    };
    let mut hit: Vec<ActorId> = Vec::new();
    for offset in [ZERO, side, vscale(side, -1.0)] {
        let trace = game.host.trace(&Q1TraceRequest {
            start: vadd(start, offset),
            end: vadd(end, offset),
            bounds: POINT,
            ignore: Some(id.clone()),
            monsters: true,
            missile: false,
        });
        let Some(actor) = trace.actor.clone() else {
            continue;
        };
        if hit.iter().any(|other| same_actor(&actor, other)) {
            continue;
        }
        hit.push(actor.clone());
        if !game
            .host
            .combat
            .read(&actor)
            .is_some_and(|combat| combat.can_take_damage)
            || game.powerup_expires(&actor, Q1Powerup::HipnoticWetsuit) != 0.0
        {
            continue;
        }
        game.host.emit(Q1Event::Particles {
            origin: trace.end,
            direction: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 100.0,
            },
            color: 225,
            count: (amount * 4.0) as i32,
        });
        game.damage(
            &actor,
            Some(&inflictor),
            from,
            amount,
            &Q1DamageParams {
                delivery: DamageDelivery::Direct,
                death_type: "electric".to_string(),
                ..Default::default()
            },
        );
    }
    Ok(())
}

/// Detonate a spike mine (`mineExplode`).
fn mine_explode(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id_copy = id.clone();
    game.radius_damage(&id_copy, Some(&id_copy), 110.0, None, None, "");
    game.sound(
        id,
        "weapons/r_exp3.wav",
        crate::q1::foundation::types::Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    let origin = game.body(id)?.origin;
    game.effect_simple(Q1Effect::Explosion, origin);
    game.sound_simple(id, "misc/null.wav")?;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(ZERO),
            ..Default::default()
        },
    )?;
    game.update_entity(id, |entity| {
        entity.touch = None;
        entity.model = "progs/s_explod.spr".to_string();
        entity.solid = Q1Solid::None;
        entity.frame = 0;
    })?;
    game.link(id)?;
    later(game, id, 0.1, "base:explosion_frame")
}

/// Home a spike mine toward its enemy (`mineHome`).
fn mine_home(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.frame = (entity.frame + 1) % 9)?;
    later(game, id, 0.2, "hip:mine_home")?;
    let search_time = game
        .entity(id)
        .map(|entity| entity.number("search_time"))
        .unwrap_or(0.0);
    if search_time < game.time {
        let mut distance = 2000.0;
        let mut selected: Option<ActorId> = None;
        for actor in scan(game, id, 2000.0, false) {
            let Some(body) = game.host.bodies.read(&actor) else {
                continue;
            };
            let origin = game.body(id)?.origin;
            let next = f64::from(length(vsub(body.origin, origin)));
            if next < distance {
                distance = next;
                selected = Some(actor);
            }
        }
        if selected.is_some() {
            game.sound_simple(id, "hipitems/spikmine.wav")?;
        }
        let time = game.time;
        game.update_entity(id, |entity| {
            entity.references.insert("enemy".to_string(), selected);
            number(entity, "search_time", time + 1.3);
        })?;
    }
    let enemy = game
        .entity(id)
        .and_then(|entity| entity.references.get("enemy").cloned().flatten());
    let target = enemy.as_ref().and_then(|enemy| game.host.bodies.read(enemy));
    let Some(target) = target else {
        game.sound_simple(id, "misc/null.wav")?;
        return game.set_body(
            id,
            &BodyPatch {
                velocity: Some(ZERO),
                ..Default::default()
            },
        );
    };
    let body = game.body(id)?;
    let direction = normalize(vsub(
        vadd(
            target.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 10.0,
            },
        ),
        body.origin,
    ));
    let in_front = dot(
        normalize(vsub(target.origin, body.origin)),
        game.make_vectors(body.angles).forward,
    ) > 0.3;
    let skill = f64::from(game.options().skill);
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(vscale(direction, skill * 50.0 + if in_front { 50.0 } else { 150.0 })),
            ..Default::default()
        },
    )
}

/// Step a lightning bolt (`lightningThink`).
fn lightning_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (delay, damage) = game
        .entity(id)
        .map(|entity| (entity.delay, entity.damage))
        .unwrap_or((0.0, 0.0));
    if game.time > delay {
        return game.remove(id);
    }
    let start = game.body(id)?.origin;
    let end = game.entity(id).map(|entity| entity.vector("oldorigin")).unwrap_or(ZERO);
    let owned = game.entity(id).map(|entity| entity.actor.clone());
    if owned
        .as_ref()
        .is_some_and(|owned| game.host.check_client(owned).is_some())
    {
        game.host.emit(Q1Event::Beam {
            style: Q1BeamStyle::Lightning2,
            actor: id.clone(),
            start,
            end,
        });
    }
    let from = game
        .entity(id)
        .and_then(|entity| entity.references.get("lastvictim").cloned().flatten());
    lightning_damage(game, id, start, end, from.as_ref(), damage)?;
    later(game, id, 0.1, "hip:lightning_bolt")
}

/// Fire a lightning trap (`lightningUse`).
fn lightning_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (pausetime, spawnflags, classname, target) = game
        .entity(id)
        .map(|entity| {
            (
                entity.number("pausetime"),
                entity.spawnflags,
                entity.classname.clone(),
                entity.target.clone(),
            )
        })
        .unwrap_or((0.0, 0, String::new(), String::new()));
    if game.time >= pausetime {
        game.sound_simple(
            id,
            if spawnflags & 2 != 0 {
                "weapons/lstart.wav"
            } else {
                "weapons/lhit.wav"
            },
        )?;
        if classname == "trap_lightning_triggered" {
            let time = game.time;
            game.update_entity(id, |entity| number(entity, "pausetime", time + 0.1))?;
        }
    }
    let mut start = game.body(id)?.origin;
    let mut end: Vec3;
    if !target.is_empty() {
        let enemy = game
            .entity(id)
            .and_then(|entity| entity.references.get("enemy").cloned().flatten())
            .and_then(|enemy| game.entity(&enemy).map(|_| enemy))
            .or_else(|| game.world.clone());
        let Some(enemy) = enemy else {
            return Err(q1_error("Lightning requires worldspawn"));
        };
        end = game.body(&enemy)?.origin;
    } else {
        let body = game.body(id)?;
        let movedir = game.make_vectors(body.angles).forward;
        game.update_entity(id, |entity| entity.movedir = movedir)?;
        end = game
            .host
            .trace(&Q1TraceRequest {
                start,
                end: vadd(start, vscale(movedir, 600.0)),
                bounds: POINT,
                ignore: Some(id.clone()),
                monsters: false,
                missile: false,
            })
            .end;
    }
    let direction = normalize(vsub(end, start));
    let distance = f64::from(length(vsub(end, start))) / 30.0;
    let remainder = distance - distance.floor();
    if remainder > 0.0 {
        start = vadd(start, vscale(direction, (remainder - 1.0) * 15.0));
        end = vsub(end, vscale(direction, (remainder - 1.0) * 15.0));
    }
    let (duration, damage) = game
        .entity(id)
        .map(|entity| (entity.number("duration"), entity.damage))
        .unwrap_or((0.0, 0.0));
    if duration > 0.1 {
        let bolt = game.create("hipnotic_lightning", None, None)?;
        game.set_origin(&bolt, start)?;
        let owner = id.clone();
        let time = game.time;
        game.update_entity(&bolt, |bolt| {
            vector(bolt, "oldorigin", end);
            bolt.references.insert("lastvictim".to_string(), Some(owner));
            bolt.damage = damage;
            bolt.delay = time + duration;
        })?;
        return lightning_think(game, &bolt);
    }
    let owned = game.entity(id).map(|entity| entity.actor.clone());
    if owned
        .as_ref()
        .is_some_and(|owned| game.host.check_client(owned).is_some())
    {
        game.host.emit(Q1Event::Beam {
            style: Q1BeamStyle::Lightning2,
            actor: id.clone(),
            start,
            end,
        });
    }
    let from = id.clone();
    lightning_damage(game, id, start, end, Some(&from), damage)
}

/// Scan for tesla targets, skipping already-struck actors (`teslaScan`).
fn tesla_scan(game: &mut Q1EntityServices, id: &ActorId) -> Vec<ActorId> {
    let (distance, spawnflags, count) = game
        .entity(id)
        .map(|entity| (entity.number("distance"), entity.spawnflags, entity.count))
        .unwrap_or((0.0, 0, 0.0));
    let mut targets = Vec::new();
    for actor in scan(game, id, distance, spawnflags & 1 != 0) {
        if is_struck_by_mjolnir(game, &actor) {
            continue;
        }
        targets.push(actor);
        if targets.len() as f64 == count {
            break;
        }
    }
    targets
}

/// Step a tesla tracking bolt (`teslaBolt`).
fn tesla_bolt(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let owner = game.entity(id).and_then(|entity| entity.owner.clone());
    if let Some(owner) = owner.filter(|owner| game.entity(owner).is_some()) {
        game.update_entity(&owner, |owner| number(owner, "attack_state", 2.0))?;
    }
    let (enemy, delay, damage, distance) = game
        .entity(id)
        .map(|entity| {
            (
                entity.references.get("hipnotic:enemy").cloned().flatten(),
                entity.delay,
                entity.damage,
                entity.number("distance"),
            )
        })
        .unwrap_or((None, 0.0, 0.0, 0.0));
    let target = enemy.as_ref().and_then(|enemy| game.host.bodies.read(enemy));
    let Some(target) = target else {
        return game.remove(id);
    };
    if game.time > delay {
        return game.remove(id);
    }
    let start = game.body(id)?.origin;
    let trace = game.host.trace(&Q1TraceRequest {
        start,
        end: target.origin,
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    let enemy = enemy.filter(|enemy| {
        trace.fraction == 1.0
            && game.health(enemy) > 0.0
            && f64::from(length(vsub(start, target.origin))) <= distance + 10.0
    });
    let Some(enemy) = enemy else {
        return game.remove(id);
    };
    let _ = enemy;
    game.host.emit(Q1Event::Beam {
        style: Q1BeamStyle::Lightning2,
        actor: id.clone(),
        start,
        end: trace.end,
    });
    let from = game
        .entity(id)
        .and_then(|entity| entity.references.get("lastvictim").cloned().flatten());
    lightning_damage(game, id, start, trace.end, from.as_ref(), damage)?;
    later(game, id, 0.1, "hip:tesla_bolt")
}

/// Run the tesla state machine (`teslaThink`).
fn tesla_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.number("hazard_state"))
        .unwrap_or(0.0)
        == 0.0
    {
        return later(game, id, 0.25, "hip:tesla_think");
    }
    let state = game
        .entity(id)
        .map(|entity| entity.number("attack_state"))
        .unwrap_or(0.0);
    if state == 0.0 {
        if !tesla_scan(game, id).is_empty() {
            let wait = game.entity(id).map(|entity| entity.wait).unwrap_or(0.0);
            if wait > 0.0 {
                game.sound_simple(id, "misc/tesla.wav")?;
            }
            game.update_entity(id, |entity| number(entity, "attack_state", 1.0))?;
            return later(game, id, wait, "hip:tesla_think");
        }
        let (delay, search_time) = game
            .entity(id)
            .map(|entity| (entity.delay, entity.number("search_time")))
            .unwrap_or((0.0, 0.0));
        if delay > 0.0 && game.time > search_time {
            game.update_entity(id, |entity| number(entity, "attack_state", 3.0))?;
        }
        return later(game, id, 0.25, "hip:tesla_think");
    }
    if state == 1.0 {
        for actor in tesla_scan(game, id) {
            game.sound_simple(id, "hipweap/mjolhit.wav")?;
            let bolt = game.create("hipnotic_tesla_lightning", None, None)?;
            let (lastvictim, duration, damage, distance, origin) = game
                .entity(id)
                .map(|entity| {
                    (
                        entity.references.get("lastvictim").cloned().flatten(),
                        entity.number("duration"),
                        entity.damage,
                        entity.number("distance"),
                        game.body(id).map(|body| body.origin),
                    )
                })
                .ok_or_else(|| q1_error("Missing Q1 entity"))?;
            let origin = origin?;
            let owner = id.clone();
            let time = game.time;
            game.update_entity(&bolt, |bolt| {
                bolt.count = 1.0;
                bolt.owner = Some(owner);
                bolt.references.insert("hipnotic:enemy".to_string(), Some(actor));
                bolt.references.insert("lastvictim".to_string(), lastvictim);
                bolt.delay = time + if duration > 0.0 { duration } else { 9999.0 };
                bolt.damage = damage;
                number(bolt, "distance", distance);
            })?;
            game.set_origin(&bolt, origin)?;
            later(game, &bolt, 0.0, "hip:tesla_bolt")?;
        }
        game.update_entity(id, |entity| number(entity, "attack_state", 2.0))?;
        return later(game, id, 1.0, "hip:tesla_think");
    }
    if state == 2.0 {
        game.update_entity(id, |entity| number(entity, "attack_state", 3.0))?;
        return later(game, id, 0.2, "hip:tesla_think");
    }
    game.update_entity(id, |entity| number(entity, "attack_state", 0.0))?;
    if game
        .entity(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default()
        == "trap_gods_wrath"
    {
        game.cancel(id);
        return Ok(());
    }
    later(game, id, 0.1, "hip:tesla_think")
}

/// Arm a mine on first think.
fn mine_first(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_name = game.named.use_callback("hip:mine_use")?;
    game.update_entity(id, |entity| {
        number(entity, "search_time", 0.0);
        entity.aimed_damage = true;
        entity.use_callback = Some(use_name);
    })?;
    game.set_damageable(id, true)?;
    later(game, id, 0.1, "hip:mine_home")
}

/// Wake a mine when a visible player uses it.
fn mine_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let Some(activator) = activator.cloned() else {
        return Ok(());
    };
    if game.is_player(&activator) && game.powerup_expires(&activator, Q1Powerup::Invisibility) <= game.time {
        game.update_entity(id, |entity| {
            entity.references.insert("enemy".to_string(), Some(activator));
        })?;
        return later(game, id, 0.1, "hip:mine_home");
    }
    Ok(())
}

/// Detonate a mine on death.
fn mine_die(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    game.set_damageable(id, false)?;
    game.killed_monsters += 1;
    let (total, found) = (game.total_monsters, game.killed_monsters);
    game.host.emit(Q1Event::MonsterKilled {
        actor: id.clone(),
        total,
        found,
    });
    let attacker = attacker.cloned();
    game.use_targets(id, attacker.as_ref())?;
    mine_explode(game, id)
}

/// Detonate a mine on touch, ignoring ordnance.
fn mine_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game.health(id) > 0.0 {
        let classname = game.host.classname(other);
        if ["trap_spike_mine", "missile", "grenade", "hiplaser", "proximity_grenade"].contains(&classname.as_str()) {
            return Ok(());
        }
        let id_copy = id.clone();
        game.damage(
            id,
            Some(&id_copy),
            Some(&id_copy),
            game.health(id) + 10.0,
            &Q1DamageParams::default(),
        );
    }
    mine_explode(game, id)
}

/// Toggle a switched hazard.
fn hazard_switch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let state = game
        .entity(id)
        .map(|entity| entity.number("hazard_state"))
        .unwrap_or(0.0);
    game.update_entity(id, |entity| number(entity, "hazard_state", 1.0 - state))?;
    let (state, think) = game
        .entity(id)
        .map(|entity| (entity.number("hazard_state"), entity.think.clone()))
        .unwrap_or((0.0, None));
    if state == 1.0 {
        if let Some(think) = think {
            let delay = game
                .entity(id)
                .map(|entity| entity.number("huntingcharmer"))
                .unwrap_or(0.0)
                - game.time;
            return game.schedule(id, delay, &think);
        }
    }
    Ok(())
}

/// Run a lightning trap's idle cycle.
fn lightning_think_cycle(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.number("hazard_state"))
        .unwrap_or(0.0)
        != 0.0
    {
        lightning_use(game, id, None, None)?;
    }
    if game.entity(id).map(|entity| entity.number("cnt")).unwrap_or(0.0) == 0.0 {
        let (spawnflags, wait, duration) = game
            .entity(id)
            .map(|entity| (entity.spawnflags, entity.wait, entity.number("duration")))
            .unwrap_or((0, 0.0, 0.0));
        let mut delay = if spawnflags & 1 != 0 {
            wait * game.host.random()
        } else {
            wait
        };
        let time = game.time;
        game.update_entity(id, |entity| {
            number(entity, "cnt", 1.0);
            number(entity, "t_length", time + duration - 0.1);
            number(entity, "pausetime", (time + duration - 0.1).max(time + 0.3));
        })?;
        delay = delay.max(duration);
        let time = game.time;
        game.update_entity(id, |entity| number(entity, "t_width", time + delay))?;
    }
    let (t_length, t_width) = game
        .entity(id)
        .map(|entity| (entity.number("t_length"), entity.number("t_width")))
        .unwrap_or((0.0, 0.0));
    if game.time >= t_length {
        game.update_entity(id, |entity| number(entity, "cnt", 0.0))?;
        return later(game, id, t_width - game.time, "hip:lightning_think");
    }
    later(game, id, 0.2, "hip:lightning_think")
}

/// Resolve a lightning trap's endpoint target.
fn lightning_first(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    if !target.is_empty() {
        let enemy = game.find(&target).first().cloned();
        game.update_entity(id, |entity| {
            entity.references.insert("enemy".to_string(), enemy);
        })?;
    }
    if game
        .entity(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default()
        == "trap_lightning_triggered"
    {
        game.cancel(id);
        return Ok(());
    }
    let (huntingcharmer, wait, ltime) = game
        .entity(id)
        .map(|entity| (entity.number("huntingcharmer"), entity.wait, entity.number("ltime")))
        .unwrap_or((0.0, 0.0, 0.0));
    later(
        game,
        id,
        huntingcharmer + wait + ltime - game.time,
        "hip:lightning_think",
    )
}

/// Spawn a lightning trap variant.
fn spawn_lightning_variant(classname: &'static str) -> fn(&mut Q1EntityServices, &ActorId) -> Result<(), Q1Error> {
    match classname {
        "trap_lightning_triggered" => spawn_lightning_triggered,
        "trap_lightning_switched" => spawn_lightning_switched,
        _ => spawn_lightning,
    }
}

fn lightning_variant(game: &mut Q1EntityServices, id: &ActorId, classname: &str) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 1.0;
        }
        if entity.damage == 0.0 {
            entity.damage = 30.0;
        }
        if entity.number("duration") == 0.0 {
            number(entity, "duration", 0.1);
        }
        number(entity, "cnt", 0.0);
        let state = if classname == "trap_lightning" {
            1.0
        } else {
            entity.number("state")
        };
        number(entity, "hazard_state", state);
        number(entity, "huntingcharmer", entity.number("nextthink"));
    })?;
    let use_name = game.named.use_callback(if classname == "trap_lightning_switched" {
        "hip:hazard_switch"
    } else {
        "hip:lightning_use"
    })?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    later(game, id, 0.25, "hip:lightning_first")
}

fn spawn_lightning(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    lightning_variant(game, id, "trap_lightning")
}
fn spawn_lightning_triggered(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    lightning_variant(game, id, "trap_lightning_triggered")
}
fn spawn_lightning_switched(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    lightning_variant(game, id, "trap_lightning_switched")
}

/// Trigger a god's wrath strike.
fn wrath_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.number("attack_state"))
        .unwrap_or(0.0)
        != 0.0
    {
        return Ok(());
    }
    let delay = game.entity(id).map(|entity| entity.delay).unwrap_or(0.0);
    let time = game.time;
    let activator = activator.cloned();
    game.update_entity(id, |entity| {
        number(entity, "search_time", time + delay);
        entity.references.insert("lastvictim".to_string(), activator);
    })?;
    tesla_think(game, id)
}

/// Spawn a tesla coil or god's wrath trap.
fn spawn_tesla_variant(classname: &'static str) -> fn(&mut Q1EntityServices, &ActorId) -> Result<(), Q1Error> {
    if classname == "trap_gods_wrath" {
        spawn_gods_wrath
    } else {
        spawn_tesla_coil
    }
}

fn tesla_variant(game: &mut Q1EntityServices, id: &ActorId, classname: &str) -> Result<(), Q1Error> {
    let skill = f64::from(game.options().skill);
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 2.0;
        }
        if entity.damage == 0.0 {
            entity.damage = 2.0 + 5.0 * skill;
        }
        if entity.number("duration") == 0.0 {
            number(entity, "duration", -1.0);
        }
        if entity.number("distance") == 0.0 {
            number(entity, "distance", 600.0);
        }
        if entity.delay == 0.0 {
            entity.delay = if classname == "trap_gods_wrath" { 5.0 } else { -1.0 };
        }
        number(entity, "hazard_state", entity.number("state"));
        number(entity, "attack_state", 0.0);
        entity.references.insert("lastvictim".to_string(), None);
    })?;
    let use_name = game.named.use_callback("hip:hazard_switch")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    let delay = game.host.random();
    later(game, id, delay, "hip:tesla_think")?;
    if classname == "trap_gods_wrath" {
        let wrath_name = game.named.use_callback("hip:wrath_use")?;
        game.update_entity(id, |entity| {
            entity.wait = 0.0;
            number(entity, "hazard_state", 1.0);
            entity.use_callback = Some(wrath_name);
        })?;
        game.cancel(id);
    }
    Ok(())
}

fn spawn_tesla_coil(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    tesla_variant(game, id, "trap_tesla_coil")
}
fn spawn_gods_wrath(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    tesla_variant(game, id, "trap_gods_wrath")
}

/// Pull actors into a gravity well.
fn gravity_well_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time))?;
    for actor in tesla_scan(game, id) {
        let owned = game.host.actors.resolve_owned(&actor);
        let body = game.host.bodies.read(&actor);
        let (Some(owned), Some(body)) = (owned, body) else {
            continue;
        };
        let (origin, speed, spawnflags) = game
            .entity(id)
            .map(|entity| (game.body(id).map(|body| body.origin), entity.speed, entity.spawnflags))
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let origin = origin?;
        let factor = if spawnflags & 2 != 0 && game.powerup_expires(&actor, Q1Powerup::HipnoticWetsuit) > game.time {
            0.6
        } else {
            1.0
        };
        let pull = vscale(normalize(vsub(origin, body.origin)), speed * factor);
        let mut next = body.clone();
        next.velocity = vadd(body.velocity, pull);
        game.host.bodies.write(&owned, &next)?;
    }
    later(game, id, 0.1, "hip:gravity_well")
}

/// Crush whoever touches a gravity well.
fn gravity_well_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (attack_finished, damage) = game
        .entity(id)
        .map(|entity| (entity.attack_finished, entity.damage))
        .unwrap_or((0.0, 0.0));
    if attack_finished > game.time
        || !game
            .host
            .combat
            .read(other)
            .is_some_and(|combat| combat.can_take_damage)
    {
        return Ok(());
    }
    let id_copy = id.clone();
    let other = other.clone();
    game.damage(
        &other,
        Some(&id_copy),
        Some(&id_copy),
        damage,
        &Q1DamageParams::default(),
    );
    let time = game.time;
    game.update_entity(id, |entity| entity.attack_finished = time + 0.2)
}

/// Spawn a `trap_gravity_well`.
fn spawn_gravity_well(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::None;
        if entity.damage == 0.0 {
            entity.damage = 10000.0;
        }
        if entity.speed == 0.0 {
            entity.speed = 210.0;
        }
        if entity.number("distance") == 0.0 {
            number(entity, "distance", 600.0);
        }
    })?;
    let touch_name = game.named.touch("hip:gravity_well")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))?;
    game.set_bounds(
        id,
        qa_core::math::Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -16.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 16.0,
            },
        },
    )?;
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time))?;
    later(game, id, 0.1, "hip:gravity_well")
}

/// Spawn a `trap_spike_mine`.
fn spawn_spike_mine(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().deathmatch != 0 {
        return game.remove(id);
    }
    let skill = game.options().skill;
    game.update_entity(id, |entity| {
        entity.model = "progs/spikmine.mdl".to_string();
        entity.solid = Q1Solid::Bbox;
        entity.movement = Q1MoveType::Flymissile;
        entity.angular_velocity = Vec3 {
            x: -50.0,
            y: 100.0,
            z: 150.0,
        };
        entity.max_health = if skill <= 1 { 200.0 } else { 400.0 };
        entity.frame = 0;
        entity.movement_flags |= 32;
    })?;
    let max_health = game.entity(id).map(|entity| entity.max_health).unwrap_or(0.0);
    game.set_health(id, max_health)?;
    game.total_monsters += 1;
    let touch_name = game.named.touch("hip:mine_explode")?;
    let die_name = game.named.die("hip:mine_explode")?;
    game.update_entity(id, |entity| {
        entity.touch = Some(touch_name);
        entity.die = Some(die_name);
    })?;
    later(game, id, 0.2, "hip:mine_first")
}

/// Register Hipnotic hazard entities (`registerHipnoticHazards`).
pub fn register_hipnotic_hazards(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "hip:mine_home",
        Q1CallbackHandlers {
            action: Some(mine_home),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:mine_first",
        Q1CallbackHandlers {
            action: Some(mine_first),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:mine_use",
        Q1CallbackHandlers {
            use_callback: Some(mine_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:mine_explode",
        Q1CallbackHandlers {
            die: Some(mine_die),
            touch: Some(mine_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trap_spike_mine", spawn_spike_mine)?;
    game.named.register(
        "hip:lightning_bolt",
        Q1CallbackHandlers {
            action: Some(lightning_think),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:lightning_use",
        Q1CallbackHandlers {
            use_callback: Some(lightning_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:hazard_switch",
        Q1CallbackHandlers {
            use_callback: Some(hazard_switch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:lightning_think",
        Q1CallbackHandlers {
            action: Some(lightning_think_cycle),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:lightning_first",
        Q1CallbackHandlers {
            action: Some(lightning_first),
            ..Default::default()
        },
    )?;
    for classname in ["trap_lightning", "trap_lightning_triggered", "trap_lightning_switched"] {
        game.register_spawn(classname, spawn_lightning_variant(classname))?;
    }
    game.named.register(
        "hip:tesla_bolt",
        Q1CallbackHandlers {
            action: Some(tesla_bolt),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:tesla_think",
        Q1CallbackHandlers {
            action: Some(tesla_think),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:wrath_use",
        Q1CallbackHandlers {
            use_callback: Some(wrath_use),
            ..Default::default()
        },
    )?;
    for classname in ["trap_tesla_coil", "trap_gods_wrath"] {
        game.register_spawn(classname, spawn_tesla_variant(classname))?;
    }
    game.named.register(
        "hip:gravity_well",
        Q1CallbackHandlers {
            action: Some(gravity_well_action),
            touch: Some(gravity_well_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trap_gravity_well", spawn_gravity_well)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    fn with_player(game: &mut Q1EntityServices, origin: Vec3) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        game.set_origin(&player, origin).expect("origin");
        game.set_health(&player, 100.0).expect("health");
        let watch = player.clone();
        game.host.players = Box::new(move || vec![watch.clone()]);
        player
    }

    #[test]
    fn mine_arms_and_homes_onto_players() {
        let mut game = test_game();
        register_hipnotic_hazards(&mut game).expect("register");
        let mine = game.create("trap_spike_mine", None, None).expect("mine");
        game.spawn_entity(&mine, None).expect("spawn");
        assert_eq!(game.total_monsters, 1);
        game.invoke_action(&mine, "hip:mine_first").expect("arm");
        assert!(game.is_damageable(&mine));
        let player = with_player(
            &mut game,
            Vec3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        );
        game.time = 0.5;
        game.invoke_action(&mine, "hip:mine_home").expect("home");
        let enemy = game
            .entity(&mine)
            .and_then(|entity| entity.references.get("enemy").cloned().flatten());
        assert_eq!(enemy.as_ref(), Some(&player));
        assert_ne!(game.body(&mine).expect("body").velocity, ZERO);
    }

    #[test]
    fn tesla_charges_then_fires_tracking_bolt() {
        let mut game = test_game();
        register_hipnotic_hazards(&mut game).expect("register");
        let coil = game.create("trap_tesla_coil", None, None).expect("coil");
        game.spawn_entity(&coil, None).expect("spawn");
        game.update_entity(&coil, |entity| {
            number(entity, "hazard_state", 1.0);
            entity.count = 1.0;
        })
        .expect("arm");
        with_player(
            &mut game,
            Vec3 {
                x: 50.0,
                y: 0.0,
                z: 0.0,
            },
        );
        game.invoke_action(&coil, "hip:tesla_think").expect("charge");
        assert_eq!(game.entity(&coil).expect("coil").number("attack_state"), 1.0);
        game.invoke_action(&coil, "hip:tesla_think").expect("fire");
        assert_eq!(game.entity(&coil).expect("coil").number("attack_state"), 2.0);
        let bolts = game.entity_ids().into_iter().filter(|id| {
            game.entity(id)
                .is_some_and(|entity| entity.classname == "hipnotic_tesla_lightning")
        });
        assert_eq!(bolts.count(), 1);
    }

    #[test]
    fn gravity_well_pulls_actors_inward() {
        let mut game = test_game();
        register_hipnotic_hazards(&mut game).expect("register");
        let well = game.create("trap_gravity_well", None, None).expect("well");
        game.spawn_entity(&well, None).expect("spawn");
        let player = with_player(
            &mut game,
            Vec3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        );
        game.invoke_action(&well, "hip:gravity_well").expect("pull");
        let velocity = game.host.bodies.read(&player).expect("body").velocity;
        assert!(f64::from(velocity.x) < 0.0, "velocity {velocity:?}");
    }

    #[test]
    fn mjolnir_tracking_marks_struck_actors() {
        let mut game = test_game();
        register_hipnotic_hazards(&mut game).expect("register");
        let player = with_player(&mut game, ZERO);
        assert!(!is_struck_by_mjolnir(&game, &player));
        let bolt = game.create("hipnotic_tesla_lightning", None, None).expect("bolt");
        game.update_entity(&bolt, |entity| {
            entity.count = 1.0;
            entity
                .references
                .insert("hipnotic:enemy".to_string(), Some(player.clone()));
        })
        .expect("track");
        assert!(is_struck_by_mjolnir(&game, &player));
    }
}
