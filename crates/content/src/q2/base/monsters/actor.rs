//! Actor monster (`src/content/q2/base/monsters/actor.ts`).
//!
//! Quake II m_actor.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{add3, normalize3, scale3, sub3, vec3, Bounds, Vec3};

use super::common::{damaged_skin, finish_corpse_default, monster_muzzle, move_handler, HUMANOID_BOUNDS};
use super::tables::actor::{actor_frame, actor_moves};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::{integer_field, movedir, number_field};
use crate::q2::foundation::host::{
    Q2Entity, Q2GameServices, Q2PresentationEvent, Q2PrintLevel, Q2Solid, Q2SpawnFn, Q2Touch, Q2Use, SpawnModule,
};
use crate::q2::foundation::monsters::ai::{angles_vectors, enemy_body, health, project_flash, vector_angles};
use crate::q2::foundation::monsters::gibs::{throw_gib, throw_head, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{record_at, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction, TouchContact};

/// Actor names (`actorNames`).
const ACTOR_NAMES: [&str; 8] = [
    "Hellrot",
    "Tokay",
    "Killme",
    "Disruptor",
    "Adrianator",
    "Rambear",
    "Titus",
    "Bitterman",
];

/// Taunt messages.
const TAUNTS: [&str; 4] = ["Watch it", "#$@*&", "Idiot", "Check your targets"];

/// Actor name (`actorName`).
pub fn actor_name(entity: &Q2Entity, game: &mut Q2GameServices) -> String {
    let slot = game
        .host
        .actors()
        .source_of(entity.actor.id())
        .map(|(_, slot)| slot)
        .unwrap_or_else(|| entity.actor.id().slot());
    record_at(&ACTOR_NAMES, (slot % 8) as usize).to_string()
}

/// Stand (`stand`).
fn actor_stand(context: &mut MonsterContext) {
    context.set_move("actor_move_stand", true);
    if context.game.host.now() < 1.0 {
        let span = actor_frame::STAND140 - actor_frame::STAND101 + 1;
        let frame = actor_frame::STAND101 + (context.game.random() * f64::from(span)).floor() as i32;
        context.entity_mut().frame = frame;
    }
}

/// Run (`run`).
fn actor_run(context: &mut MonsterContext) {
    if context.game.host.now() < context.state().pain_time && context.entity().enemy.is_none() {
        if context.state().move_target.is_some() {
            context.set_move("actor_move_walk", true);
        } else {
            actor_stand(context);
        }
        return;
    }
    if context.state().stand_ground {
        actor_stand(context);
    } else {
        context.set_move("actor_move_run", true);
    }
}

/// Initialize (`initialize`).
fn actor_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let (targetname, target) = {
        let entity = context.entity();
        (entity.targetname.clone(), entity.target.clone())
    };
    if targetname.is_empty() || target.is_empty() {
        let origin = context.game.body_of(actor.clone()).origin;
        context.game.host.diagnostic(&format!(
            "misc_actor requires target and targetname at {{\"x\":{},\"y\":{},\"z\":{}}}",
            origin.x, origin.y, origin.z
        ));
        context.game.remove_actor(actor);
        return;
    }
    context.state_mut().good_guy = true;
    let spawn = context.entity().spawn.clone();
    let raw = integer_field(&spawn, "health", 0);
    let max_health = if raw == 0 { 100.0 } else { f64::from(raw) };
    context.entity_mut().max_health = max_health;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_health(&owned, max_health);
    let use_callback = context.game.source_callbacks.resolve_use(Some("actor_use"));
    context.entity_mut().use_ = use_callback;
}

/// Attack (`attack`).
fn actor_attack(context: &mut MonsterContext) {
    context.set_move("actor_move_attack", true);
    let pause = context.game.host.now() + ((context.game.random() * 16.0).floor() + 10.0) * 0.1;
    context.state_mut().pause_time = pause;
}

/// Pain (`pain`).
fn actor_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if reaction
        .attacker
        .as_ref()
        .is_some_and(|attacker| context.game.host.is_player(attacker))
        && context.game.random() < 0.4
    {
        let attacker = reaction.attacker.clone().expect("actor pain attacker");
        let actor = context.actor().clone();
        if let Some(other) = context.game.host.bodies().read(&attacker) {
            let origin = context.game.body_of(actor.clone()).origin;
            let yaw = vector_angles(sub3(other.origin, origin)).y;
            context.state_mut().ideal_yaw = f64::from(yaw);
        }
        if context.game.random() < 0.5 {
            context.set_move("actor_move_flipoff", true);
        } else {
            context.set_move("actor_move_taunt", true);
        }
        let name = actor_name(&context.entity().clone(), &mut *context.game);
        // The donor samples only the first three taunts.
        let taunt = record_at(&TAUNTS, (context.game.random() * 3.0).floor() as usize);
        context.game.host_emit(Q2PresentationEvent::Print {
            actor: Some(attacker),
            level: Q2PrintLevel::Chat,
            text: format!("{name}: {taunt}!\n"),
        });
        return;
    }
    let n = (context.game.random() * 3.0).floor() as i32;
    context.set_move(
        if n == 0 {
            "actor_move_pain1"
        } else if n == 1 {
            "actor_move_pain2"
        } else {
            "actor_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn actor_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if health(&mut *context.game, Some(&actor)) <= -80.0 {
        let damage = reaction.pain.damage;
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/bone/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        for _ in 0..4 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        throw_head(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/head2/tris.md2",
            damage,
        );
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    if context.game.random() < 0.5 {
        context.set_move("actor_move_death1", true);
    } else {
        context.set_move("actor_move_death2", true);
    }
}

/// Fire (`actor_fire`).
fn actor_fire(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy_body = enemy_body(context);
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 63), None);
    let body = context.game.body_of(actor.clone());
    let mut direction = angles_vectors(body.angles).forward;
    if let Some(enemy_body) = enemy_body {
        let enemy = context.entity().enemy.clone();
        let target = if health(&mut *context.game, enemy.as_ref()) > 0.0 {
            let view_height = enemy
                .as_ref()
                .and_then(|enemy| context.game.entities.get(enemy))
                .map(|entity| entity.view_height)
                .unwrap_or(22);
            add3(
                add3(enemy_body.origin, scale3(enemy_body.velocity, -0.2)),
                vec3(0.0, 0.0, view_height as f32),
            )
        } else {
            vec3(
                enemy_body.origin.x + enemy_body.bounds.min.x,
                enemy_body.origin.y + enemy_body.bounds.min.y,
                enemy_body.origin.z + (enemy_body.bounds.min.z + enemy_body.bounds.max.z) / 2.0,
            )
        };
        direction = normalize3(sub3(target, start));
    }
    let fire_bullet = context.weapons.fire_bullet;
    fire_bullet(actor, &mut *context.game, start, direction, 3.0, 4.0, 300.0, 500.0, 0);
    monster_muzzle(context, 63, direction, start);
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Actor definition (`actorDefinition`).
pub fn actor_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "misc_actor",
        "actor",
        "players/male/tris.md2",
        100.0,
        -80.0,
        200.0,
        HUMANOID_BOUNDS,
        1.0,
        "actor_move_stand",
        actor_moves(),
        MonsterHandler::Callback(actor_stand),
        move_handler("actor_move_walk"),
        MonsterHandler::Callback(actor_run),
        MonsterHandler::Callback(actor_attack),
        actor_die,
    );
    definition.pain = Some(actor_pain);
    definition.initialize = Some(MonsterHandler::Callback(actor_initialize));
    definition.callbacks = HashMap::from([
        ("actor_run".to_string(), MonsterHandler::Callback(actor_run)),
        (
            "actor_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        ("actor_fire".to_string(), MonsterHandler::Callback(actor_fire)),
    ]);
    definition
}

/// Actor use (`actor_use`).
pub fn actor_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    if !game.monsters.states.contains_key(&actor) {
        panic!("Actor use without restored monster state");
    }
    let target_name = game.require_entity(&actor).target.clone();
    let target = game.pick_target(&target_name);
    let goal = target.clone();
    game.require_entity_mut(&actor).goal = goal;
    if let Some(state) = game.monsters.states.get_mut(&actor) {
        state.move_target = target.clone();
    }
    let bad = target
        .as_ref()
        .is_none_or(|target| game.require_entity(target).classname != "target_actor");
    if bad {
        game.host
            .diagnostic(&format!("misc_actor has bad target {target_name}"));
        game.require_entity_mut(&actor).target = String::new();
        if let Some(state) = game.monsters.states.get_mut(&actor) {
            state.pause_time = 100000000.0;
        }
        let mut context = MonsterContext::new(actor, game);
        actor_stand(&mut context);
        return;
    }
    let target = target.expect("actor target");
    let to = game.body_of(target.clone()).origin;
    let from = game.body_of(actor.clone()).origin;
    let yaw = vector_angles(sub3(to, from)).y;
    if let Some(state) = game.monsters.states.get_mut(&actor) {
        state.ideal_yaw = f64::from(yaw);
    }
    let mut body = game.body_of(actor.clone());
    body.angles.y = yaw;
    game.write_body(actor.clone(), &body, true);
    game.require_entity_mut(&actor).target = String::new();
    let mut context = MonsterContext::new(actor, game);
    context.walk();
}

/// Target actor touch (`target_actor_touch`).
pub fn target_actor_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    let contender = game.entities.get(&other).cloned();
    let state = game.monsters.states.get(&other);
    let (Some(contender), Some(state)) = (contender, state) else {
        return;
    };
    if state.move_target.as_ref() != Some(&actor) || contender.enemy.is_some() {
        return;
    }
    game.require_entity_mut(&other).goal = None;
    if let Some(state) = game.monsters.states.get_mut(&other) {
        state.move_target = None;
    }
    let pad = game.require_entity(&actor).clone();
    if !pad.message.is_empty() {
        let name = actor_name(&contender, game);
        for player in game.host.players() {
            game.host_emit(Q2PresentationEvent::Print {
                actor: Some(player),
                level: Q2PrintLevel::Chat,
                text: format!("{name}: {}\n", pad.message),
            });
        }
    }
    if pad.spawnflags & 1 != 0 {
        let body = game.body_of(other.clone());
        let mut moved = body.clone();
        moved.velocity = vec3(
            pad.movedir.x * pad.speed as f32,
            pad.movedir.y * pad.speed as f32,
            if body.ground.is_some() {
                pad.movedir.z
            } else {
                body.velocity.z
            },
        );
        moved.ground = None;
        game.write_body(other.clone(), &moved, true);
        if body.ground.is_some() {
            game.sound(&other, "player/male/jump1.wav", 2, 1.0, 1.0);
        }
    }
    let path_target = pad.spawn.values.get("pathtarget").cloned().unwrap_or_default();
    if pad.spawnflags & 2 == 0 && pad.spawnflags & 4 != 0 {
        let enemy = game.pick_target(&path_target);
        if let Some(enemy) = enemy {
            game.require_entity_mut(&other).goal = Some(enemy.clone());
            game.require_entity_mut(&other).enemy = Some(enemy);
            if pad.spawnflags & 32 != 0 {
                if let Some(state) = game.monsters.states.get_mut(&other) {
                    state.brutal = true;
                }
            }
            let mut context = MonsterContext::new(other.clone(), game);
            if pad.spawnflags & 16 != 0 {
                context.state_mut().stand_ground = true;
                actor_stand(&mut context);
            } else {
                actor_run(&mut context);
            }
            return;
        }
        return;
    }
    if pad.spawnflags & 6 == 0 && !path_target.is_empty() {
        let original = game.require_entity(&actor).target.clone();
        game.require_entity_mut(&actor).target = path_target;
        let authored = game.require_entity(&actor).clone().authored_target();
        game.use_targets(&authored, Some(&other), false);
        game.require_entity_mut(&actor).target = original;
    }
    let target_name = game.require_entity(&actor).target.clone();
    let next = game.pick_target(&target_name);
    if let Some(state) = game.monsters.states.get_mut(&other) {
        state.move_target = next.clone();
    }
    if game.require_entity(&other).goal.is_none() {
        game.require_entity_mut(&other).goal = next.clone();
    }
    let enemy = game.require_entity(&other).enemy.clone();
    if next.is_none() && enemy.is_none() {
        let pause = game.host.now() + 100000000.0;
        if let Some(state) = game.monsters.states.get_mut(&other) {
            state.pause_time = pause;
        }
        let mut context = MonsterContext::new(other, game);
        actor_stand(&mut context);
        return;
    }
    if let Some(next) = next {
        let goal = game.require_entity(&other).goal.clone();
        if goal.as_ref() == Some(&next) {
            let to = game.body_of(next).origin;
            let from = game.body_of(other.clone()).origin;
            let yaw = vector_angles(sub3(to, from)).y;
            if let Some(state) = game.monsters.states.get_mut(&other) {
                state.ideal_yaw = f64::from(yaw);
            }
        }
    }
}

/// Spawn a target actor (`spawn`).
pub fn spawn_target_actor(actor: ActorId, game: &mut Q2GameServices) -> bool {
    if game.require_entity(&actor).classname != "target_actor" {
        return false;
    }
    if game.require_entity(&actor).targetname.is_empty() {
        let origin = game.body_of(actor.clone()).origin;
        game.host.diagnostic(&format!(
            "target_actor has no targetname at {{\"x\":{},\"y\":{},\"z\":{}}}",
            origin.x, origin.y, origin.z
        ));
    }
    {
        let entity = game.require_entity_mut(&actor);
        entity.server_flags = 1;
        entity.visible = false;
    }
    let mut body = game.body_of(actor.clone());
    body.bounds = Bounds {
        min: Vec3 {
            x: -8.0,
            y: -8.0,
            z: -8.0,
        },
        max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
    };
    game.write_body(actor.clone(), &body, true);
    if game.require_entity(&actor).spawnflags & 1 != 0 {
        if game.require_entity(&actor).speed == 0.0 {
            game.require_entity_mut(&actor).speed = 200.0;
        }
        let angles = game.body_of(actor.clone()).angles;
        let yaw = if angles.y == 0.0 { 360.0 } else { angles.y };
        let spawn = game.require_entity(&actor).spawn.clone();
        let height = number_field(&spawn, "height", 0.0);
        let mut direction = movedir(vec3(angles.x, yaw, angles.z));
        direction.z = if height == 0.0 { 200.0 } else { height as f32 };
        game.require_entity_mut(&actor).movedir = direction;
        let mut body = game.body_of(actor.clone());
        body.angles = vec3(0.0, 0.0, 0.0);
        game.write_body(actor.clone(), &body, true);
    }
    game.set_solid(actor.clone(), Q2Solid::Trigger);
    game.require_entity_mut(&actor).touch = Some(target_actor_touch as Q2Touch);
    true
}

/// Actor target module (`createActorTargetModule`).
pub fn create_actor_target_module() -> SpawnModule {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.use_.insert("actor_use", actor_use as Q2Use);
    callbacks
        .touch
        .insert("target_actor_touch", target_actor_touch as Q2Touch);
    SpawnModule {
        spawn: spawn_target_actor as Q2SpawnFn,
        item_name: |_| None,
        callbacks,
    }
}
