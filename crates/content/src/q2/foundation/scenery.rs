//! Q2 scenery (`src/content/q2/foundation/scenery.ts`).
//!
//! Breakable scenery and props adapted from Quake II game/g_misc.c.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{add3, normalize3, scale3, sub3, vec3, Vec3};

use super::callbacks::{free_q2_entity, free_q2_entity_die, Q2CallbackDefinitions};
use super::fields::{integer_field, number_field};
use super::host::{
    Q2Die, Q2EffectEvent, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SpawnFn,
    Q2TraceRequest, SpawnModule,
};
use super::monsters::gibs::{throw_gib, throw_head, Q2GibOptions};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, TouchContact, TraceHit};

/// Throw debris (`throwQ2Debris`).
pub fn throw_q2_debris(this: ActorId, game: &mut Q2GameServices, model: &str, speed: f64, origin: Vec3) -> ActorId {
    let chunk = game.create("debris", BTreeMap::new());
    game.require_entity_mut(&chunk).model = model.to_string();
    let random_velocity = vec3(
        (100.0 * (game.host.random() * 2.0 - 1.0)) as f32,
        (100.0 * (game.host.random() * 2.0 - 1.0)) as f32,
        (100.0 + 100.0 * (game.host.random() * 2.0 - 1.0)) as f32,
    );
    let mut moved = game.body_of(chunk.clone());
    moved.origin = origin;
    moved.velocity = add3(game.body_of(this).velocity, scale3(random_velocity, speed as f32));
    game.write_body(chunk.clone(), &moved, false);
    game.require_entity_mut(&chunk).angular_velocity = vec3(
        (game.host.random() * 600.0) as f32,
        (game.host.random() * 600.0) as f32,
        (game.host.random() * 600.0) as f32,
    );
    let owned = game.owned_of(chunk.clone());
    game.create_combat(&owned, 0.0, 0.0, true);
    game.require_entity_mut(&chunk).die = Some(free_q2_entity_die);
    game.set_motion_kind(chunk.clone(), Q2MotionKind::Bounce);
    game.set_solid(chunk.clone(), Q2Solid::None);
    game.show(chunk.clone());
    let delay = 5.0 + game.host.random() * 5.0;
    game.schedule(chunk.clone(), delay, free_q2_entity);
    chunk
}

/// Explode (`explode`).
fn scenery_explode(game: &mut Q2GameServices, actor: ActorId, kind: i32) {
    let origin = game.body_of(actor.clone()).origin;
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: format!("q2:explosion{kind}"),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    game.remove_actor(actor);
}

/// Kill box (`killQ2Box`).
pub fn kill_q2_box(game: &mut Q2GameServices, actor: ActorId) -> bool {
    loop {
        let body = game.body_of(actor.clone());
        let trace = game.host.trace(&Q2TraceRequest {
            start: body.origin,
            end: body.origin,
            bounds: Some(body.bounds),
            ignore: Some(actor.clone()),
            mask: 0x201_0003,
            exclude: Vec::new(),
        });
        let TraceHit::Actor { actor: victim } = &trace.hit else {
            return !trace.start_solid && !trace.all_solid;
        };
        let victim = victim.clone();
        if game.host.combat().read(&victim).is_none() {
            return false;
        }
        game.damage(
            victim.clone(),
            actor.clone(),
            Some(actor.clone()),
            100_000.0,
            0.0,
            vec3(0.0, 0.0, 0.0),
            body.origin,
            vec3(0.0, 0.0, 0.0),
            32,
            21,
            None,
        );
        if game.entity(&victim).is_none() {
            let next = game.host.trace(&Q2TraceRequest {
                start: body.origin,
                end: body.origin,
                bounds: Some(body.bounds),
                ignore: Some(actor.clone()),
                mask: 0x201_0003,
                exclude: Vec::new(),
            });
            return !matches!(&next.hit, TraceHit::Actor { actor: other } if other == &victim);
        }
        if game.require_entity(&victim).solid != Q2Solid::None {
            return false;
        }
    }
}

/// Func wall (`wall`).
fn scenery_wall(game: &mut Q2GameServices, actor: ActorId) {
    game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
    let entity = game.require_entity_mut(&actor);
    if entity.spawnflags & 8 != 0 {
        entity.effects |= 0x1000;
    }
    if entity.spawnflags & 16 != 0 {
        entity.effects |= 0x2000;
    }
    if entity.spawnflags & 7 == 0 {
        game.set_solid(actor.clone(), Q2Solid::Brush);
        game.show(actor);
        return;
    }
    let entity = game.require_entity_mut(&actor);
    entity.spawnflags |= 1;
    if entity.spawnflags & 4 != 0 {
        entity.spawnflags |= 2;
    }
    let visible = game.require_entity(&actor).spawnflags & 4 != 0;
    game.require_entity_mut(&actor).visible = visible;
    game.set_solid(actor.clone(), if visible { Q2Solid::Brush } else { Q2Solid::None });
    game.show(actor.clone());
    game.require_entity_mut(&actor).use_ = Some(func_wall_use);
}

/// Func explosive (`explosive`).
fn scenery_explosive(game: &mut Q2GameServices, actor: ActorId) {
    if game.options.mode == Q2Mode::Deathmatch {
        game.remove_actor(actor);
        return;
    }
    game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
    let entity = game.require_entity_mut(&actor);
    if entity.spawnflags & 2 != 0 {
        entity.effects |= 0x1000;
    }
    if entity.spawnflags & 4 != 0 {
        entity.effects |= 0x2000;
    }
    if game.require_entity(&actor).spawnflags & 1 != 0 {
        let entity = game.require_entity_mut(&actor);
        entity.visible = false;
        entity.use_ = Some(func_explosive_spawn);
    } else if !game.require_entity(&actor).targetname.is_empty() {
        game.require_entity_mut(&actor).use_ = Some(func_explosive_use);
    }
    if game.require_entity(&actor).spawnflags & 1 != 0 || game.require_entity(&actor).targetname.is_empty() {
        if game.require_entity(&actor).max_health == 0.0 {
            game.require_entity_mut(&actor).max_health = 100.0;
        }
        let spawn = game.require_entity(&actor).spawn.clone();
        let authored = number_field(&spawn, "mass", 0.0);
        let owned = game.owned_of(actor.clone());
        let max_health = game.require_entity(&actor).max_health;
        game.create_combat(&owned, max_health, if authored != 0.0 { authored } else { 75.0 }, true);
        game.require_entity_mut(&actor).die = Some(func_explosive_die);
    }
    let visible = game.require_entity(&actor).visible;
    game.set_solid(actor.clone(), if visible { Q2Solid::Brush } else { Q2Solid::None });
    game.show(actor);
}

/// Explosive barrel (`barrel`).
fn scenery_barrel(game: &mut Q2GameServices, actor: ActorId) {
    if game.options.mode == Q2Mode::Deathmatch {
        game.remove_actor(actor);
        return;
    }
    let entity = game.require_entity_mut(&actor);
    entity.model = "models/objects/barrels/tris.md2".to_string();
    if entity.max_health == 0.0 {
        entity.max_health = 10.0;
    }
    if entity.damage == 0.0 {
        entity.damage = 150.0;
    }
    let spawn = game.require_entity(&actor).spawn.clone();
    let authored = number_field(&spawn, "mass", 0.0);
    let owned = game.owned_of(actor.clone());
    let max_health = game.require_entity(&actor).max_health;
    game.create_combat(&owned, max_health, if authored != 0.0 { authored } else { 400.0 }, true);
    let mut moved = game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, 0.0);
    moved.bounds.max = vec3(16.0, 16.0, 40.0);
    game.write_body(actor.clone(), &moved, false);
    game.set_solid(actor.clone(), Q2Solid::Box);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Step);
    game.show(actor.clone());
    let entity = game.require_entity_mut(&actor);
    entity.die = Some(barrel_delay);
    entity.touch = Some(barrel_touch);
    let frame_seconds = game.host.frame_seconds();
    game.schedule(actor, 2.0 * frame_seconds, barrel_drop_to_floor);
}

/// Scenery spawn (`createQ2SceneryModule spawn`).
fn spawn_scenery(actor: ActorId, game: &mut Q2GameServices) -> bool {
    match game.require_entity(&actor).classname.as_str() {
        "func_wall" => {
            scenery_wall(game, actor);
            true
        }
        "func_explosive" => {
            scenery_explosive(game, actor);
            true
        }
        "misc_explobox" => {
            scenery_barrel(game, actor);
            true
        }
        "misc_banner" => {
            let frame = (game.host.random() * 16.0).floor() as i32;
            let entity = game.require_entity_mut(&actor);
            entity.model = "models/objects/banner/tris.md2".to_string();
            entity.frame = frame;
            game.show(actor.clone());
            game.link_actor(actor.clone());
            game.schedule(actor, 0.1, banner_think);
            true
        }
        "misc_satellite_dish" => {
            game.require_entity_mut(&actor).model = "models/objects/satellite/tris.md2".to_string();
            let mut moved = game.body_of(actor.clone());
            moved.bounds.min = vec3(-64.0, -64.0, 0.0);
            moved.bounds.max = vec3(64.0, 64.0, 128.0);
            game.write_body(actor.clone(), &moved, false);
            game.require_entity_mut(&actor).use_ = Some(satellite_use);
            game.set_solid(actor.clone(), Q2Solid::Box);
            game.show(actor);
            true
        }
        "misc_deadsoldier" => {
            if game.options.mode == Q2Mode::Deathmatch {
                game.remove_actor(actor);
                return true;
            }
            let spawn = game.require_entity(&actor).spawn.clone();
            let flags = game.require_entity(&actor).spawnflags;
            let frame = if flags & 2 != 0 {
                1
            } else if flags & 4 != 0 {
                2
            } else if flags & 8 != 0 {
                3
            } else if flags & 16 != 0 {
                4
            } else if flags & 32 != 0 {
                5
            } else {
                0
            };
            let entity = game.require_entity_mut(&actor);
            entity.model = "models/deadbods/dude/tris.md2".to_string();
            entity.frame = frame;
            entity.server_flags |= 12;
            let owned = game.owned_of(actor.clone());
            game.create_combat(&owned, f64::from(integer_field(&spawn, "health", 0)), 200.0, true);
            let mut moved = game.body_of(actor.clone());
            moved.bounds.min = vec3(-16.0, -16.0, 0.0);
            moved.bounds.max = vec3(16.0, 16.0, 16.0);
            game.write_body(actor.clone(), &moved, false);
            game.require_entity_mut(&actor).die = Some(misc_deadsoldier_die);
            game.set_solid(actor.clone(), Q2Solid::Box);
            game.show(actor);
            true
        }
        "misc_gib_head" => {
            let spin = vec3(
                (game.host.random() * 200.0) as f32,
                (game.host.random() * 200.0) as f32,
                (game.host.random() * 200.0) as f32,
            );
            let entity = game.require_entity_mut(&actor);
            entity.model = "models/objects/gibs/head/tris.md2".to_string();
            entity.effects |= 2;
            entity.server_flags |= 4;
            entity.angular_velocity = spin;
            let owned = game.owned_of(actor.clone());
            game.create_combat(&owned, 0.0, 0.0, true);
            game.require_entity_mut(&actor).die = Some(free_q2_entity_die);
            game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
            game.set_solid(actor.clone(), Q2Solid::None);
            game.show(actor.clone());
            game.schedule(actor, 30.0, free_q2_entity);
            true
        }
        _ => false,
    }
}

/// Func wall use (`func_wall_use`).
fn func_wall_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let visible = game.require_entity(&this).solid == Q2Solid::None;
    game.require_entity_mut(&this).visible = visible;
    game.set_solid(this.clone(), if visible { Q2Solid::Brush } else { Q2Solid::None });
    if visible {
        kill_q2_box(game, this.clone());
    }
    game.show(this.clone());
    if game.require_entity(&this).spawnflags & 2 == 0 {
        game.require_entity_mut(&this).use_ = None;
    }
}

/// Break apart (`breakApart`).
fn scenery_break_apart(
    this: ActorId,
    game: &mut Q2GameServices,
    inflictor: Option<ActorId>,
    attacker: Option<ActorId>,
) {
    let body = game.body_of(this.clone());
    let size = scale3(sub3(body.bounds.max, body.bounds.min), 0.5);
    let origin = add3(add3(body.origin, body.bounds.min), size);
    let mut moved = game.body_of(this.clone());
    moved.origin = origin;
    game.write_body(this.clone(), &moved, false);
    if game.host.combat().read(&this).is_some() {
        let owned = game.owned_of(this.clone());
        game.set_combat_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(false),
                ..CombatTraitChanges::default()
            },
        );
    }
    let damage = game.require_entity(&this).damage;
    if damage != 0.0 {
        game.radius_damage(this.clone(), attacker.clone(), damage, None, damage + 40.0, 25, 0, None);
    }
    let from = inflictor
        .as_ref()
        .and_then(|inflictor| game.host.bodies().read(inflictor))
        .map(|body| body.origin)
        .unwrap_or(origin);
    let mut moved = game.body_of(this.clone());
    moved.velocity = scale3(normalize3(sub3(origin, from)), 150.0);
    game.write_body(this.clone(), &moved, false);
    let spawn = game.require_entity(&this).spawn.clone();
    let authored = number_field(&spawn, "mass", 0.0);
    let mass = if authored != 0.0 { authored } else { 75.0 };
    for _ in 0..((mass / 100.0).trunc() as i32).min(8) {
        let point = scenery_random_point(game, origin, size, 0.5);
        throw_q2_debris(this.clone(), game, "models/objects/debris1/tris.md2", 1.0, point);
    }
    for _ in 0..((mass / 25.0).trunc() as i32).min(16) {
        let point = scenery_random_point(game, origin, size, 0.5);
        throw_q2_debris(this.clone(), game, "models/objects/debris2/tris.md2", 2.0, point);
    }
    let authored = game.require_entity(&this).authored_target();
    game.use_targets(&authored, attacker.as_ref(), false);
    if damage != 0.0 {
        scenery_explode(game, this, 1);
    } else {
        game.remove_actor(this);
    }
}

/// Random debris point (`breakApart randomPoint`).
fn scenery_random_point(game: &mut Q2GameServices, origin: Vec3, size: Vec3, factor: f32) -> Vec3 {
    add3(
        origin,
        vec3(
            (game.host.random() * 2.0 - 1.0) as f32 * size.x * factor,
            (game.host.random() * 2.0 - 1.0) as f32 * size.y * factor,
            (game.host.random() * 2.0 - 1.0) as f32 * size.z * factor,
        ),
    )
}

/// Func explosive spawn use (`func_explosive_spawn`).
fn func_explosive_spawn(
    this: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let entity = game.require_entity_mut(&this);
    entity.visible = true;
    entity.use_ = None;
    game.set_solid(this.clone(), Q2Solid::Brush);
    kill_q2_box(game, this.clone());
    game.show(this);
}

/// Func explosive use (`func_explosive_use`).
fn func_explosive_use(this: ActorId, game: &mut Q2GameServices, other: Option<ActorId>, _activator: Option<ActorId>) {
    scenery_break_apart(this.clone(), game, Some(this), other);
}

/// Func explosive die (`func_explosive_die`).
fn func_explosive_die(this: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    scenery_break_apart(this, game, reaction.inflictor, reaction.pain.attacker);
}

/// Barrel blast (`blast`).
fn barrel_blast(this: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&this);
    let (activator, damage) = (entity.activator.clone(), entity.damage);
    let attacker = activator.unwrap_or_else(|| this.clone());
    game.radius_damage(this.clone(), Some(attacker), damage, None, damage + 40.0, 26, 0, None);
    let body = game.body_of(this.clone());
    let size = sub3(body.bounds.max, body.bounds.min);
    let low = add3(body.origin, body.bounds.min);
    let center = add3(low, scale3(size, 0.5));
    for _ in 0..2 {
        let point = scenery_random_point(game, center, size, 1.0);
        throw_q2_debris(
            this.clone(),
            game,
            "models/objects/debris1/tris.md2",
            1.5 * damage / 200.0,
            point,
        );
    }
    for point in [
        low,
        add3(low, vec3(size.x, 0.0, 0.0)),
        add3(low, vec3(0.0, size.y, 0.0)),
        add3(low, vec3(size.x, size.y, 0.0)),
    ] {
        throw_q2_debris(
            this.clone(),
            game,
            "models/objects/debris3/tris.md2",
            1.75 * damage / 200.0,
            point,
        );
    }
    for _ in 0..8 {
        let point = scenery_random_point(game, center, size, 1.0);
        throw_q2_debris(
            this.clone(),
            game,
            "models/objects/debris2/tris.md2",
            2.0 * damage / 200.0,
            point,
        );
    }
    let grounded = game.body_of(this.clone()).ground.is_some();
    scenery_explode(game, this, if grounded { 2 } else { 1 });
}

/// Barrel delay (`barrel_delay`).
fn barrel_delay(this: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    let owned = game.owned_of(this.clone());
    game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(false),
            ..CombatTraitChanges::default()
        },
    );
    game.require_entity_mut(&this).activator = reaction.pain.attacker;
    let frame_seconds = game.host.frame_seconds();
    game.schedule(this, 2.0 * frame_seconds, barrel_blast);
}

/// Barrel touch (`barrel_touch`).
fn barrel_touch(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = game.host.bodies().read(&contact.other);
    let state = game.host.combat().read(&contact.other);
    let (Some(other), Some(state)) = (other, state) else {
        return;
    };
    if other.ground.is_none() || other.ground == Some(this.clone()) {
        return;
    }
    let body = game.body_of(this.clone());
    let delta = sub3(body.origin, other.origin);
    let direction = normalize3(vec3(delta.x, delta.y, 0.0));
    let mass = game.host.combat().read(&this).map(|state| state.mass).unwrap_or(400.0);
    let end = add3(
        body.origin,
        scale3(direction, (20.0 * state.mass / mass * game.host.frame_seconds()) as f32),
    );
    let trace = game.host.trace(&Q2TraceRequest {
        start: body.origin,
        end,
        bounds: Some(body.bounds),
        ignore: Some(this.clone()),
        mask: 0x201_0003,
        exclude: Vec::new(),
    });
    if trace.fraction == 1.0 {
        let mut moved = game.body_of(this.clone());
        moved.origin = trace.end;
        game.write_body(this, &moved, true);
    }
}

/// Barrel drop to floor (`barrelDropToFloor`).
fn barrel_drop_to_floor(this: ActorId, game: &mut Q2GameServices) {
    let body = game.body_of(this.clone());
    let start = add3(body.origin, vec3(0.0, 0.0, 1.0));
    let trace = game.host.trace(&Q2TraceRequest {
        start,
        end: add3(start, vec3(0.0, 0.0, -256.0)),
        bounds: Some(body.bounds),
        ignore: Some(this.clone()),
        mask: 0x201_0003,
        exclude: Vec::new(),
    });
    if !trace.all_solid && trace.fraction < 1.0 {
        let ground = match &trace.hit {
            TraceHit::Actor { actor } => Some(actor.clone()),
            TraceHit::World { .. } => Some(game.host.world_actor()),
            TraceHit::None => None,
        };
        let mut moved = game.body_of(this.clone());
        moved.origin = trace.end;
        moved.ground = ground;
        game.write_body(this, &moved, true);
    }
}

/// Banner think (`bannerThink`).
fn banner_think(this: ActorId, game: &mut Q2GameServices) {
    let frame = game.require_entity(&this).frame;
    game.require_entity_mut(&this).frame = (frame + 1) % 16;
    game.show(this.clone());
    game.schedule(this, 0.1, banner_think);
}

/// Satellite think (`satelliteThink`).
fn satellite_think(this: ActorId, game: &mut Q2GameServices) {
    let frame = game.require_entity(&this).frame + 1;
    game.require_entity_mut(&this).frame = frame;
    game.show(this.clone());
    if frame < 38 {
        game.schedule(this, 0.1, satellite_think);
    }
}

/// Satellite use (`satellite_use`).
fn satellite_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    game.require_entity_mut(&this).frame = 0;
    game.schedule(this, 0.1, satellite_think);
}

/// Dead soldier die (`misc_deadsoldier_die`).
fn misc_deadsoldier_die(this: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    if game.host.combat().read(&this).map(|state| state.health).unwrap_or(0.0) > -80.0 {
        return;
    }
    game.sound(&this, "misc/udeath.wav", 4, 1.0, 1.0);
    for _ in 0..4 {
        throw_gib(
            this.clone(),
            game,
            "models/objects/gibs/sm_meat/tris.md2",
            reaction.pain.damage,
            Q2GibOptions::default(),
        );
    }
    throw_head(this, game, "models/objects/gibs/head2/tris.md2", reaction.pain.damage);
}

/// Scenery item name (unused).
fn scenery_item_name(_classname: &str) -> Option<String> {
    None
}

/// Create the Q2 scenery module (`createQ2SceneryModule`).
pub fn create_q2_scenery_module() -> SpawnModule {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("barrel_explode", barrel_blast);
    callbacks.think.insert("barrel_drop_to_floor", barrel_drop_to_floor);
    callbacks.think.insert("misc_banner_think", banner_think);
    callbacks.think.insert("misc_satellite_dish_think", satellite_think);
    callbacks.use_.insert("func_wall_use", func_wall_use);
    callbacks.use_.insert("func_explosive_spawn", func_explosive_spawn);
    callbacks.use_.insert("func_explosive_use", func_explosive_use);
    callbacks.use_.insert("satellite_use", satellite_use);
    callbacks.touch.insert("barrel_touch", barrel_touch);
    let die: Q2Die = func_explosive_die;
    callbacks.die.insert("func_explosive_die", die);
    let die: Q2Die = barrel_delay;
    callbacks.die.insert("barrel_delay", die);
    let die: Q2Die = misc_deadsoldier_die;
    callbacks.die.insert("misc_deadsoldier_die", die);
    let spawn: Q2SpawnFn = spawn_scenery;
    SpawnModule {
        spawn,
        item_name: scenery_item_name,
        callbacks,
    }
}
