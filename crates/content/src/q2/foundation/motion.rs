//! Q2 linear motion (`src/content/q2/foundation/motion.ts`).
//!
//! Linear Move_Calc from game/g_func.c and accelerated platform movement.

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{add3, length3, scale3, sub3, vec3, Vec3};

use super::callbacks::Q2CallbackDefinitions;
use super::checkpoint::restore_q2_actor;
use super::host::{Q2Edition, Q2GameServices, Q2Think};

/// Linear move curve (`LinearMove["curve"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct LinearMoveCurve {
    /// Sampled positions.
    pub positions: Vec<f32>,
    /// Current frame.
    pub frame: usize,
    /// Current subframe.
    pub subframe: usize,
    /// Subframe count.
    pub subframes: f64,
}

/// Linear move state (`LinearMove`).
#[derive(Debug, Clone)]
pub struct LinearMoveState {
    /// Direction.
    pub direction: Vec3,
    /// Destination.
    pub destination: Vec3,
    /// Reference origin.
    pub reference: Vec3,
    /// Remaining distance.
    pub remaining: f64,
    /// Current speed.
    pub current_speed: f64,
    /// Move speed.
    pub move_speed: f64,
    /// Next speed.
    pub next_speed: f64,
    /// Deceleration distance.
    pub decel_distance: f64,
    /// Completion callback.
    pub done: Q2Think,
    /// Rerelease curve.
    pub curve: Option<LinearMoveCurve>,
}

/// Linear move checkpoint entry (`Q2LinearMotionCheckpoint` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct LinearMoveCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Direction.
    pub direction: Vec3,
    /// Destination.
    pub destination: Vec3,
    /// Reference origin.
    pub reference: Vec3,
    /// Remaining distance.
    pub remaining: f64,
    /// Current speed.
    pub current_speed: f64,
    /// Move speed.
    pub move_speed: f64,
    /// Next speed.
    pub next_speed: f64,
    /// Deceleration distance.
    pub decel_distance: f64,
    /// Completion callback name.
    pub done: String,
    /// Rerelease curve.
    pub curve: Option<LinearMoveCurve>,
}

/// Linear motion checkpoint (`Q2LinearMotionCheckpoint`).
pub type Q2LinearMotionCheckpoint = Vec<LinearMoveCheckpoint>;

/// Linear motion instance scope (`Q2LinearMotion` namespace).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinearMotionScope {
    /// Foundation movers (`q2:foundation/linear`).
    Foundation,
    /// Base entities (`q2:base/linear`).
    Base,
}

/// Read the scoped move map.
fn linear_moves(game: &Q2GameServices, scope: LinearMotionScope) -> &HashMap<ActorId, LinearMoveState> {
    match scope {
        LinearMotionScope::Foundation => &game.movers.linear_moves,
        LinearMotionScope::Base => &game.base_entities.linear_moves,
    }
}

/// Mutably read the scoped move map.
fn linear_moves_mut(game: &mut Q2GameServices, scope: LinearMotionScope) -> &mut HashMap<ActorId, LinearMoveState> {
    match scope {
        LinearMotionScope::Foundation => &mut game.movers.linear_moves,
        LinearMotionScope::Base => &mut game.base_entities.linear_moves,
    }
}

/// Resolve the scope holding an actor's move (foundation wins ties).
fn linear_scope_for(game: &Q2GameServices, actor: &ActorId) -> LinearMotionScope {
    if game.movers.linear_moves.contains_key(actor) {
        LinearMotionScope::Foundation
    } else {
        LinearMotionScope::Base
    }
}

/// Linear motion callbacks (`Q2LinearMotion[callbacks]`).
///
/// The donor parameterizes the callback prefix, but the only instantiation
/// uses `q2:foundation/linear`, so the names are fixed here.
pub fn linear_motion_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("q2:foundation/linear/Move_Done", linear_move_done);
    callbacks
        .think
        .insert("q2:foundation/linear/Move_Final", linear_move_final);
    callbacks
        .think
        .insert("q2:foundation/linear/Move_Begin", linear_move_begin);
    callbacks
        .think
        .insert("q2:foundation/linear/Think_AccelMove", linear_move_accelerate);
    callbacks
        .think
        .insert("q2:foundation/linear/Move_Accel_Curve", linear_move_curve);
    callbacks
}

/// Base linear motion callbacks (`q2:base/linear`).
pub fn base_linear_motion_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("q2:base/linear/Move_Done", linear_move_done);
    callbacks.think.insert("q2:base/linear/Move_Final", linear_move_final);
    callbacks.think.insert("q2:base/linear/Move_Begin", linear_move_begin);
    callbacks
        .think
        .insert("q2:base/linear/Think_AccelMove", linear_move_accelerate);
    callbacks
        .think
        .insert("q2:base/linear/Move_Accel_Curve", linear_move_curve);
    callbacks
}

/// Scoped linear motion callbacks.
fn scoped_linear_motion_callbacks(scope: LinearMotionScope) -> Q2CallbackDefinitions {
    match scope {
        LinearMotionScope::Foundation => linear_motion_callbacks(),
        LinearMotionScope::Base => base_linear_motion_callbacks(),
    }
}

/// Linear move destination (`Q2LinearMotion[destination]`).
pub fn linear_move_destination(game: &mut Q2GameServices, scope: LinearMotionScope, actor: &ActorId) -> Option<Vec3> {
    linear_moves(game, scope).get(actor).map(|state| state.destination)
}

/// Capture linear motion (`Q2LinearMotion[capture]`).
pub fn capture_linear_motion(game: &mut Q2GameServices, scope: LinearMotionScope) -> Q2LinearMotionCheckpoint {
    let mut entries = Vec::new();
    let actors: Vec<ActorId> = game.entities.keys().cloned().collect();
    for actor in actors {
        let Some(state) = linear_moves(game, scope).get(&actor) else {
            continue;
        };
        let state = state.clone();
        let Some(done) = game.source_callbacks.think_name(Some(state.done)) else {
            panic!("Q2 linear move checkpoint has no end function");
        };
        entries.push(LinearMoveCheckpoint {
            actor: SavedActorId::from(&actor),
            direction: state.direction,
            destination: state.destination,
            reference: state.reference,
            remaining: state.remaining,
            current_speed: state.current_speed,
            move_speed: state.move_speed,
            next_speed: state.next_speed,
            decel_distance: state.decel_distance,
            done: done.to_string(),
            curve: state.curve,
        });
    }
    entries
}

/// Restore linear motion (`Q2LinearMotion[restore]`).
pub fn restore_linear_motion(
    game: &mut Q2GameServices,
    scope: LinearMotionScope,
    checkpoint: &Q2LinearMotionCheckpoint,
) {
    *linear_moves_mut(game, scope) = HashMap::new();
    for saved in checkpoint {
        let owned = restore_q2_actor(game, saved.actor);
        let done = game.source_callbacks.resolve_think(Some(&saved.done));
        let Some(done) = done.filter(|_| game.entity(owned.id()).is_some()) else {
            panic!("Q2 linear move checkpoint has no actor or end function");
        };
        linear_moves_mut(game, scope).insert(
            owned.id().clone(),
            LinearMoveState {
                direction: saved.direction,
                destination: saved.destination,
                reference: saved.reference,
                remaining: saved.remaining,
                current_speed: saved.current_speed,
                move_speed: saved.move_speed,
                next_speed: saved.next_speed,
                decel_distance: saved.decel_distance,
                done,
                curve: saved.curve.clone(),
            },
        );
    }
}

/// Move to a destination (`Q2LinearMotion[moveTo]`).
pub fn linear_move_to(
    game: &mut Q2GameServices,
    scope: LinearMotionScope,
    actor: ActorId,
    destination: Vec3,
    done: Q2Think,
) {
    let callbacks = scoped_linear_motion_callbacks(scope);
    game.source_callbacks.register(&callbacks);
    let origin = game.body_of(actor.clone()).origin;
    let delta = sub3(destination, origin);
    let distance = f64::from(length3(delta));
    let mut state = LinearMoveState {
        direction: if distance == 0.0 {
            vec3(0.0, 0.0, 0.0)
        } else {
            scale3(delta, (1.0 / distance) as f32)
        },
        remaining: distance,
        destination,
        reference: origin,
        current_speed: 0.0,
        move_speed: 0.0,
        next_speed: 0.0,
        decel_distance: 0.0,
        done,
        curve: None,
    };
    linear_moves_mut(game, scope).insert(actor.clone(), state.clone());
    linear_move_velocity(game, &actor, vec3(0.0, 0.0, 0.0));
    let entity = game.require_entity(&actor);
    let (speed, accel, decel) = (entity.speed, entity.accel, entity.decel);
    if speed == accel && speed == decel {
        let team_master = game
            .require_entity(&actor)
            .team_master
            .clone()
            .unwrap_or_else(|| actor.clone());
        if game.current_actor == Some(team_master) {
            return linear_move_begin(actor, game);
        }
        let frame_seconds = game.host.frame_seconds();
        return game.schedule(actor, frame_seconds, linear_move_begin);
    }
    let frame_seconds = game.host.frame_seconds();
    if game.options.edition == Q2Edition::Rerelease && frame_seconds != 0.1 {
        let subframes = 0.1 / frame_seconds - 1.0;
        let mut positions: Vec<f32> = if subframes != 0.0 { vec![0.0] } else { Vec::new() };
        while state.remaining != 0.0 {
            if state.current_speed == 0.0 {
                calculate_acceleration(&mut state, speed, accel, decel);
            }
            accelerate(&mut state, speed, accel, decel);
            if state.remaining <= state.current_speed {
                break;
            }
            state.remaining -= state.current_speed;
            positions.push((distance - state.remaining) as f32);
        }
        if subframes != 0.0 {
            positions.push(distance as f32);
        }
        state.curve = Some(LinearMoveCurve {
            positions,
            frame: if subframes != 0.0 { 1 } else { 0 },
            subframe: 0,
            subframes,
        });
        linear_moves_mut(game, scope).insert(actor.clone(), state);
        return game.schedule(actor, frame_seconds, linear_move_curve);
    }
    game.schedule(actor, frame_seconds, linear_move_accelerate);
}

/// Active linear move state (`Q2LinearMotion[state]`).
fn linear_move_state(game: &mut Q2GameServices, actor: &ActorId) -> LinearMoveState {
    let scope = linear_scope_for(game, actor);
    linear_moves(game, scope).get(actor).cloned().unwrap_or_else(|| {
        panic!("Q2 mover callback has no active move");
    })
}

/// Set mover velocity (`Q2LinearMotion[velocity]`).
fn linear_move_velocity(game: &mut Q2GameServices, actor: &ActorId, velocity: Vec3) {
    let mut moved = game.body_of(actor.clone());
    moved.velocity = velocity;
    game.write_body(actor.clone(), &moved, false);
    let motion = game.require_entity(actor).motion;
    game.set_motion_kind(actor.clone(), motion);
}

/// Linear move done (`Move_Done`).
fn linear_move_done(actor: ActorId, game: &mut Q2GameServices) {
    linear_move_velocity(game, &actor, vec3(0.0, 0.0, 0.0));
    let state = linear_move_state(game, &actor);
    let scope = linear_scope_for(game, &actor);
    linear_moves_mut(game, scope).remove(&actor);
    (state.done)(actor, game);
}

/// Linear move final (`Move_Final`).
fn linear_move_final(actor: ActorId, game: &mut Q2GameServices) {
    let state = linear_move_state(game, &actor);
    if state.remaining == 0.0 {
        return linear_move_done(actor, game);
    }
    let frame_seconds = game.host.frame_seconds();
    let delta = if game.options.edition == Q2Edition::Rerelease {
        sub3(state.destination, game.body_of(actor.clone()).origin)
    } else {
        scale3(state.direction, state.remaining as f32)
    };
    linear_move_velocity(game, &actor, scale3(delta, (1.0 / frame_seconds) as f32));
    game.schedule(actor, frame_seconds, linear_move_done);
}

/// Linear move begin (`Move_Begin`).
fn linear_move_begin(actor: ActorId, game: &mut Q2GameServices) {
    let mut state = linear_move_state(game, &actor);
    let frame = game.host.frame_seconds();
    let speed = game.require_entity(&actor).speed;
    if speed * frame >= state.remaining {
        return linear_move_final(actor, game);
    }
    linear_move_velocity(game, &actor, scale3(state.direction, speed as f32));
    let frames = (state.remaining / speed / frame).floor();
    state.remaining -= frames * speed * frame;
    let scope = linear_scope_for(game, &actor);
    linear_moves_mut(game, scope).insert(actor.clone(), state);
    game.schedule(actor, frames * frame, linear_move_final);
}

/// Linear move accelerate (`Think_AccelMove`).
fn linear_move_accelerate(actor: ActorId, game: &mut Q2GameServices) {
    let mut state = linear_move_state(game, &actor);
    if game.options.edition == Q2Edition::Rerelease {
        state.remaining = f64::from(length3(sub3(state.destination, game.body_of(actor.clone()).origin)));
    } else {
        state.remaining -= state.current_speed;
    }
    let entity = game.require_entity(&actor);
    let (speed, accel, decel) = (entity.speed, entity.accel, entity.decel);
    if state.current_speed == 0.0 {
        calculate_acceleration(&mut state, speed, accel, decel);
    }
    accelerate(&mut state, speed, accel, decel);
    if state.remaining <= state.current_speed {
        let scope = linear_scope_for(game, &actor);
        linear_moves_mut(game, scope).insert(actor.clone(), state);
        return linear_move_final(actor, game);
    }
    let frame_seconds = game.host.frame_seconds();
    linear_move_velocity(
        game,
        &actor,
        scale3(state.direction, (state.current_speed / frame_seconds) as f32),
    );
    let scope = linear_scope_for(game, &actor);
    linear_moves_mut(game, scope).insert(actor.clone(), state);
    game.schedule(actor, frame_seconds, linear_move_accelerate);
}

/// Linear move curve (`Move_Accel_Curve`).
fn linear_move_curve(actor: ActorId, game: &mut Q2GameServices) {
    let mut state = linear_move_state(game, &actor);
    let Some(curve) = state.curve.clone() else {
        panic!("Q2 rerelease accelerated move has no curve");
    };
    let mut curve = curve;
    if curve.subframes != 0.0 && curve.subframe as f64 == curve.subframes + 1.0 {
        curve.subframe = 0;
        curve.frame += 1;
    }
    if curve.frame == curve.positions.len() {
        state.curve = Some(curve);
        let scope = linear_scope_for(game, &actor);
        linear_moves_mut(game, scope).insert(actor.clone(), state);
        return linear_move_final(actor, game);
    }
    let distance: f64;
    if curve.subframes != 0.0 {
        let from = curve.positions.get(curve.frame - 1).copied();
        let to = curve.positions.get(curve.frame).copied();
        let (Some(from), Some(to)) = (from, to) else {
            panic!("Q2 accelerated move references a missing curve sample");
        };
        distance = f64::from(from)
            + (f64::from(to) - f64::from(from)) * (curve.subframe as f64 + 1.0) / (curve.subframes + 1.0);
        curve.subframe += 1;
    } else {
        let sample = curve.positions.get(curve.frame).copied();
        let Some(sample) = sample else {
            panic!("Q2 accelerated move references a missing curve sample");
        };
        curve.frame += 1;
        distance = f64::from(sample);
    }
    let target = add3(state.reference, scale3(state.direction, distance as f32));
    let frame_seconds = game.host.frame_seconds();
    let origin = game.body_of(actor.clone()).origin;
    linear_move_velocity(game, &actor, scale3(sub3(target, origin), (1.0 / frame_seconds) as f32));
    state.curve = Some(curve);
    let scope = linear_scope_for(game, &actor);
    linear_moves_mut(game, scope).insert(actor.clone(), state);
    game.schedule(actor, frame_seconds, linear_move_curve);
}

/// Acceleration distance (`accelerationDistance`).
fn acceleration_distance(target: f64, rate: f64) -> f64 {
    target * (target / rate + 1.0) / 2.0
}

/// Calculate acceleration (`calculateAcceleration`).
fn calculate_acceleration(state: &mut LinearMoveState, speed: f64, accel: f64, decel: f64) {
    state.move_speed = speed;
    if state.remaining < accel {
        state.current_speed = state.remaining;
        return;
    }
    let accel_distance = acceleration_distance(speed, accel);
    let mut decel_distance = acceleration_distance(speed, decel);
    if state.remaining - accel_distance - decel_distance < 0.0 {
        let factor = (accel + decel) / (accel * decel);
        state.move_speed = (-2.0 + (4.0 + 8.0 * factor * state.remaining).sqrt()) / (2.0 * factor);
        decel_distance = acceleration_distance(state.move_speed, decel);
    }
    state.decel_distance = decel_distance;
}

/// Accelerate (`accelerate`).
fn accelerate(state: &mut LinearMoveState, speed: f64, accel: f64, decel: f64) {
    if state.remaining <= state.decel_distance {
        if state.remaining < state.decel_distance {
            if state.next_speed != 0.0 {
                state.current_speed = state.next_speed;
                state.next_speed = 0.0;
                return;
            }
            if state.current_speed > decel {
                state.current_speed -= decel;
            }
        }
        return;
    }
    if state.current_speed == state.move_speed && state.remaining - state.current_speed < state.decel_distance {
        let first_distance = state.remaining - state.decel_distance;
        let second_distance = state.move_speed * (1.0 - first_distance / state.move_speed);
        state.current_speed = state.move_speed;
        state.next_speed = state.move_speed - decel * second_distance / (first_distance + second_distance);
        return;
    }
    if state.current_speed < speed {
        let old_speed = state.current_speed;
        state.current_speed = (state.current_speed + accel).min(speed);
        if state.remaining - state.current_speed >= state.decel_distance {
            return;
        }
        let first_distance = state.remaining - state.decel_distance;
        let first_speed = (old_speed + state.move_speed) / 2.0;
        let second_distance = state.move_speed * (1.0 - first_distance / first_speed);
        let distance = first_distance + second_distance;
        state.current_speed = first_speed * first_distance / distance + state.move_speed * second_distance / distance;
        state.next_speed = state.move_speed - decel * second_distance / distance;
    }
}
