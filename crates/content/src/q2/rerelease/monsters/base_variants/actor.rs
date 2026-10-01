//! Rerelease actor (`src/content/q2/rerelease/monsters/base-variants/actor.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, normalize3, scale3, sub3, vec3};

use super::super::common::monster_flash;
use super::super::tables::actor::actor_moves;
use crate::q2::base::monsters::actor::{actor_definition, actor_name};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::{integer_field, movedir, number_field};
use crate::q2::foundation::host::{
    Q2Edition, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2PrintLevel, Q2Solid, Q2SpawnFn, SpawnModule,
};
use crate::q2::foundation::monsters::ai::{angles_vectors, enemy_body, health, vector_angles};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{record_at, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{PainReaction, TouchContact};

/// Hold forever (`holdForever`).
const HOLD_FOREVER: f64 = (i64::MAX as f64) / 1000.0;

/// Dead (`dead`).
fn actor_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.state_mut().corpse = true;
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, -24.0);
    moved.bounds.max = vec3(16.0, 16.0, -8.0);
    context.game.write_body(actor.clone(), &moved, true);
    context.game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    context.game.cancel_actor(actor);
}

/// Use (`use`).
fn actor_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    if !game.monsters.states.contains_key(&actor) {
        return;
    }
    let target_name = game.require_entity(&actor).target.clone();
    let target = game.pick_target(&target_name);
    game.require_entity_mut(&actor).goal = target.clone();
    if let Some(state) = game.monsters.states.get_mut(&actor) {
        state.move_target = target.clone();
    }
    let bad = match target.as_ref() {
        None => true,
        Some(target) => game.require_entity(target).classname != "target_actor",
    };
    if bad {
        game.host
            .diagnostic(&format!("misc_actor has bad target {target_name}"));
        game.require_entity_mut(&actor).target = String::new();
        let mut context = MonsterContext::new(actor, &mut *game);
        context.state_mut().pause_time = HOLD_FOREVER;
        context.stand();
        return;
    }
    let target = target.expect("actor target");
    let target_origin = game.body_of(target).origin;
    let origin = game.body_of(actor.clone()).origin;
    let yaw = f64::from(vector_angles(sub3(target_origin, origin)).y);
    if let Some(state) = game.monsters.states.get_mut(&actor) {
        state.ideal_yaw = yaw;
    }
    let mut moved = game.body_of(actor.clone());
    moved.angles.y = yaw as f32;
    game.write_body(actor.clone(), &moved, true);
    let mut context = MonsterContext::new(actor.clone(), &mut *game);
    context.walk();
    context.game.require_entity_mut(&actor).target = String::new();
}

/// Touch (`touch`).
fn target_actor_touch(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    if game.entity(&other).is_none() || !game.monsters.states.contains_key(&other) {
        return;
    }
    let move_target = game
        .monsters
        .states
        .get(&other)
        .expect("actor touch")
        .move_target
        .clone();
    if move_target != Some(this.clone()) || game.require_entity(&other).enemy.is_some() {
        return;
    }
    game.require_entity_mut(&other).goal = None;
    if let Some(state) = game.monsters.states.get_mut(&other) {
        state.move_target = None;
    }
    let message = game.require_entity(&this).message.clone();
    if !message.is_empty() {
        let entity = game.require_entity(&other).clone();
        let name = actor_name(&entity, game);
        for player in game.host.players() {
            game.host_emit(Q2PresentationEvent::Print {
                actor: Some(player),
                level: Q2PrintLevel::Chat,
                text: format!("{name}: {message}\n"),
            });
        }
    }
    let flags = game.require_entity(&this).spawnflags;
    if flags & 1 != 0 {
        let body = game.body_of(other.clone());
        let entity = game.require_entity(&this);
        let (movedir, speed) = (entity.movedir, entity.speed);
        let mut moved = body.clone();
        moved.velocity = vec3(
            movedir.x * speed as f32,
            movedir.y * speed as f32,
            if body.ground.is_some() {
                movedir.z
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
    let path_target = game
        .require_entity(&this)
        .spawn
        .values
        .get("pathtarget")
        .cloned()
        .unwrap_or_default();
    let flags = game.require_entity(&this).spawnflags;
    if flags & 2 == 0 && flags & 4 != 0 {
        let enemy = game.pick_target(&path_target);
        game.require_entity_mut(&other).enemy = enemy.clone();
        if let Some(enemy) = enemy {
            game.require_entity_mut(&other).goal = Some(enemy);
            if flags & 32 != 0 {
                if let Some(state) = game.monsters.states.get_mut(&other) {
                    state.brutal = true;
                }
            }
            if flags & 16 != 0 {
                let mut context = MonsterContext::new(other.clone(), &mut *game);
                context.state_mut().stand_ground = true;
                context.stand();
            } else {
                let mut context = MonsterContext::new(other.clone(), &mut *game);
                context.run();
            }
        }
    }
    let flags = game.require_entity(&this).spawnflags;
    if flags & 6 == 0 && !path_target.is_empty() {
        let original = game.require_entity(&this).target.clone();
        game.require_entity_mut(&this).target = path_target;
        let authored = game.require_entity(&this).authored_target();
        game.use_targets(&authored, Some(&other), false);
        game.require_entity_mut(&this).target = original;
    }
    let target_name = game.require_entity(&this).target.clone();
    let goal = game.pick_target(&target_name);
    if let Some(state) = game.monsters.states.get_mut(&other) {
        state.move_target = goal.clone();
    }
    if game.require_entity(&other).goal.is_none() {
        game.require_entity_mut(&other).goal = goal.clone();
    }
    if goal.is_none() && game.require_entity(&other).enemy.is_none() {
        let mut context = MonsterContext::new(other, &mut *game);
        context.state_mut().pause_time = HOLD_FOREVER;
        context.stand();
        return;
    }
    if let Some(goal) = goal {
        let actor_goal = game.require_entity(&other).goal.clone();
        if actor_goal == Some(goal.clone()) {
            let goal_origin = game.body_of(goal).origin;
            let origin = game.body_of(other.clone()).origin;
            let yaw = f64::from(vector_angles(sub3(goal_origin, origin)).y);
            if let Some(state) = game.monsters.states.get_mut(&other) {
                state.ideal_yaw = yaw;
            }
        }
    }
}

/// Initialize (`initialize`).
fn rerelease_actor_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let entity = context.game.require_entity(&actor);
    if entity.targetname.is_empty() || entity.target.is_empty() {
        let origin = context.game.body_of(actor.clone()).origin;
        context.game.host.diagnostic(&format!(
            "misc_actor requires target and targetname at {{\"x\":{},\"y\":{},\"z\":{}}}",
            origin.x, origin.y, origin.z
        ));
        context.game.remove_actor(actor);
        return;
    }
    context.state_mut().good_guy = true;
    let spawn = context.game.require_entity(&actor).spawn.clone();
    let authored = integer_field(&spawn, "health", 0);
    let max_health = if authored != 0 { f64::from(authored) } else { 100.0 };
    context.game.require_entity_mut(&actor).max_health = max_health;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_health(&owned, max_health);
}

/// After spawn (`afterSpawn`).
fn rerelease_actor_after_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).use_ = Some(actor_use);
}

/// Attack (`attack`).
fn rerelease_actor_attack(context: &mut MonsterContext) {
    context.set_move("actor_move_attack", true);
    let now = context.game.host.now();
    context.state_mut().fire_wait = now + 1.0 + context.game.random() * 1.6;
}

/// Pain (`pain`).
fn rerelease_actor_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let attacker = reaction.attacker.clone();
    let attacker = attacker.filter(|attacker| context.game.host.is_player(attacker) && context.game.random() < 0.4);
    if let Some(attacker) = attacker {
        if let Some(other) = context.game.host.bodies().read(&attacker) {
            let origin = context.game.body_of(actor.clone()).origin;
            context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(other.origin, origin)).y);
        }
        let flipoff = context.game.random() < 0.5;
        context.set_move(
            if flipoff {
                "actor_move_flipoff"
            } else {
                "actor_move_taunt"
            },
            true,
        );
        let message = record_at(
            &["Watch it", "#$@*&", "Idiot", "Check your targets"],
            (context.game.random() * 4.0).floor() as usize,
        );
        let entity = context.game.require_entity(&actor).clone();
        let name = actor_name(&entity, &mut *context.game);
        context.game.host_emit(Q2PresentationEvent::Print {
            actor: Some(attacker),
            level: Q2PrintLevel::Chat,
            text: format!("{name}: {message}!\n"),
        });
        return;
    }
    let variant = (context.game.random() * 3.0).floor() as i32 + 1;
    context.set_move(&format!("actor_move_pain{variant}"), true);
}

/// Fire (`actor_fire`).
fn actor_fire(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let offset = muzzle_offset(Q2Edition::Rerelease, 63);
    let enemy = enemy_body(context);
    let start = add3(
        add3(
            add3(body.origin, scale3(axes.forward, offset.x)),
            scale3(axes.right, offset.y),
        ),
        vec3(0.0, 0.0, offset.z),
    );
    let mut direction = axes.forward;
    if let Some(enemy) = enemy {
        let enemy_id = context.entity().enemy.clone();
        let target = if health(&mut *context.game, enemy_id.as_ref()) > 0.0 {
            let view_height = enemy_id
                .as_ref()
                .and_then(|enemy| context.game.entity(enemy))
                .map(|target| target.view_height)
                .unwrap_or(22);
            add3(
                add3(enemy.origin, scale3(enemy.velocity, -0.2)),
                vec3(0.0, 0.0, view_height as f32),
            )
        } else {
            vec3(
                enemy.origin.x + enemy.bounds.min.x,
                enemy.origin.y + enemy.bounds.min.y,
                enemy.origin.z + (enemy.bounds.min.z + enemy.bounds.max.z) / 2.0 + 1.0,
            )
        };
        direction = normalize3(sub3(target, start));
    }
    let fire_bullet = context.weapons.fire_bullet;
    fire_bullet(actor, &mut *context.game, start, direction, 3.0, 4.0, 300.0, 500.0, 0);
    monster_flash(context, 63, start, direction);
    context.state_mut().hold_frame = context.game.host.now() < context.state().fire_wait;
}

/// Create the rerelease actor definition (`createRereleaseActorModule definition`).
pub fn rerelease_actor_definition() -> Q2MonsterDefinition {
    let mut definition = actor_definition();
    definition.moves = actor_moves();
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.use_.insert("rerelease.actor.actor_use", actor_use);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_actor_initialize));
    definition.after_spawn = Some(MonsterHandler::Callback(rerelease_actor_after_spawn));
    definition.attack = MonsterHandler::Callback(rerelease_actor_attack);
    definition.pain = Some(rerelease_actor_pain);
    for (name, handler) in [
        ("actor_dead", MonsterHandler::Callback(actor_dead)),
        ("actor_fire", MonsterHandler::Callback(actor_fire)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}

/// Target actor spawn (`targets spawn`).
fn spawn_target_actor(actor: ActorId, game: &mut Q2GameServices) -> bool {
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
    let entity = game.require_entity_mut(&actor);
    entity.server_flags = 1;
    entity.visible = false;
    entity.touch = Some(target_actor_touch);
    let mut moved = game.body_of(actor.clone());
    moved.bounds.min = vec3(-8.0, -8.0, -8.0);
    moved.bounds.max = vec3(8.0, 8.0, 8.0);
    game.write_body(actor.clone(), &moved, true);
    if game.require_entity(&actor).spawnflags & 1 != 0 {
        if game.require_entity(&actor).speed == 0.0 {
            game.require_entity_mut(&actor).speed = 200.0;
        }
        let angles = game.body_of(actor.clone()).angles;
        let spawn = game.require_entity(&actor).spawn.clone();
        let facing = vec3(angles.x, if angles.y != 0.0 { angles.y } else { 360.0 }, angles.z);
        let height = number_field(&spawn, "height", 0.0);
        let mut movedir = movedir(facing);
        movedir.z = if height != 0.0 { height as f32 } else { 200.0 };
        game.require_entity_mut(&actor).movedir = movedir;
        let mut moved = game.body_of(actor.clone());
        moved.angles = vec3(0.0, 0.0, 0.0);
        game.write_body(actor.clone(), &moved, true);
    }
    game.set_solid(actor, Q2Solid::Trigger);
    true
}

/// Target actor item name (unused).
fn target_actor_item_name(_classname: &str) -> Option<String> {
    None
}

/// Create the rerelease actor targets module (`createRereleaseActorModule targets`).
pub fn rerelease_actor_targets() -> SpawnModule {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .touch
        .insert("rerelease.actor.target_actor_touch", target_actor_touch);
    let spawn: Q2SpawnFn = spawn_target_actor;
    SpawnModule {
        spawn,
        item_name: target_actor_item_name,
        callbacks,
    }
}
