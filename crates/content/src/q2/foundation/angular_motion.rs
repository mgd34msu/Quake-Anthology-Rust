//! Q2 angular motion (`src/content/q2/foundation/angular-motion.ts`).
//!
//! AngleMove_Calc from game/g_func.c and rerelease g_func.cpp.

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{Vec3, length3, scale3, sub3, vec3};

use super::callbacks::Q2CallbackDefinitions;
use super::checkpoint::restore_q2_actor;
use super::host::{Q2Edition, Q2GameServices};

/// Angular move state (`Q2AngularMotion` entry).
#[derive(Debug, Clone)]
pub struct AngularMoveState {
    /// Destination angles.
    pub destination: Vec3,
    /// Current speed.
    pub speed: f64,
    /// Completion callback.
    pub done: super::host::Q2Think,
}

/// Angular move checkpoint entry (`Q2AngularMotionCheckpoint` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct AngularMoveCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Destination angles.
    pub destination: Vec3,
    /// Current speed.
    pub speed: f64,
    /// Completion callback name.
    pub done: String,
}

/// Angular motion checkpoint (`Q2AngularMotionCheckpoint`).
pub type Q2AngularMotionCheckpoint = Vec<AngularMoveCheckpoint>;

/// Angular motion callbacks (`Q2AngularMotion[callbacks]`).
///
/// The donor parameterizes the callback prefix, but the only instantiation
/// uses `q2:foundation/angular`, so the names are fixed here.
pub fn angular_motion_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("q2:foundation/angular/AngleMove_Done", angle_move_done);
    callbacks.think.insert("q2:foundation/angular/AngleMove_Final", angle_move_final);
    callbacks.think.insert("q2:foundation/angular/AngleMove_Begin", angle_move_begin);
    callbacks
}

/// Capture angular motion (`Q2AngularMotion[capture]`).
pub fn capture_angular_motion(game: &mut Q2GameServices) -> Q2AngularMotionCheckpoint {
    let mut entries = Vec::new();
    let actors: Vec<ActorId> = game.entities.keys().cloned().collect();
    for actor in actors {
        let Some(state) = game.movers.angular_moves.get(&actor) else {
            continue;
        };
        let (destination, speed, done) = (state.destination, state.speed, state.done);
        let Some(done) = game.source_callbacks.think_name(Some(done)) else {
            panic!("Q2 angular move checkpoint has no end function");
        };
        entries.push(AngularMoveCheckpoint {
            actor: SavedActorId::from(&actor),
            destination,
            speed,
            done: done.to_string(),
        });
    }
    entries
}

/// Restore angular motion (`Q2AngularMotion[restore]`).
pub fn restore_angular_motion(game: &mut Q2GameServices, checkpoint: &Q2AngularMotionCheckpoint) {
    game.movers.angular_moves = HashMap::new();
    for saved in checkpoint {
        let owned = restore_q2_actor(game, saved.actor.clone());
        let done = game.source_callbacks.resolve_think(Some(&saved.done));
        let Some(done) = done.filter(|_| game.entity(owned.id()).is_some()) else {
            panic!("Q2 angular move checkpoint has no actor or end function");
        };
        game.movers.angular_moves.insert(
            owned.id().clone(),
            AngularMoveState {
                destination: saved.destination,
                speed: saved.speed,
                done,
            },
        );
    }
}

/// Move to angles (`Q2AngularMotion[moveTo]`).
pub fn angular_move_to(
    game: &mut Q2GameServices,
    actor: ActorId,
    destination: Vec3,
    done: super::host::Q2Think,
) {
    let callbacks = angular_motion_callbacks();
    game.source_callbacks.register(&callbacks);
    let entity = game.require_entity(&actor);
    let (accel, speed) = (entity.accel, entity.speed);
    let start = if game.options.edition == Q2Edition::Rerelease && accel != speed {
        0.0
    } else {
        speed
    };
    game.movers.angular_moves.insert(
        actor.clone(),
        AngularMoveState {
            destination,
            speed: start,
            done,
        },
    );
    game.require_entity_mut(&actor).angular_velocity = vec3(0.0, 0.0, 0.0);
    let motion = game.require_entity(&actor).motion;
    game.set_motion_kind(actor.clone(), motion);
    let team_master = game.require_entity(&actor).team_master.clone().unwrap_or_else(|| actor.clone());
    if game.current_actor == Some(team_master) {
        angle_move_begin(actor, game);
    } else {
        let frame_seconds = game.host.frame_seconds();
        game.schedule(actor, frame_seconds, angle_move_begin);
    }
}

/// Active angular move state (`Q2AngularMotion[state]`).
fn angular_move_state(game: &mut Q2GameServices, actor: &ActorId) -> AngularMoveState {
    game.movers.angular_moves.get(actor).cloned().unwrap_or_else(|| {
        panic!("Q2 angular callback has no active move");
    })
}

/// Angle move done (`AngleMove_Done`).
fn angle_move_done(actor: ActorId, game: &mut Q2GameServices) {
    let state = angular_move_state(game, &actor);
    game.movers.angular_moves.remove(&actor);
    game.require_entity_mut(&actor).angular_velocity = vec3(0.0, 0.0, 0.0);
    let motion = game.require_entity(&actor).motion;
    game.set_motion_kind(actor.clone(), motion);
    (state.done)(actor, game);
}

/// Angle move final (`AngleMove_Final`).
fn angle_move_final(actor: ActorId, game: &mut Q2GameServices) {
    let state = angular_move_state(game, &actor);
    let delta = sub3(state.destination, game.body_of(actor.clone()).angles);
    if delta.x == 0.0 && delta.y == 0.0 && delta.z == 0.0 {
        return angle_move_done(actor, game);
    }
    let frame_seconds = game.host.frame_seconds();
    game.require_entity_mut(&actor).angular_velocity = scale3(delta, (1.0 / frame_seconds) as f32);
    let motion = game.require_entity(&actor).motion;
    game.set_motion_kind(actor.clone(), motion);
    game.schedule(actor, frame_seconds, angle_move_done);
}

/// Angle move begin (`AngleMove_Begin`).
fn angle_move_begin(actor: ActorId, game: &mut Q2GameServices) {
    let mut state = angular_move_state(game, &actor);
    let entity = game.require_entity(&actor);
    let (speed, accel) = (entity.speed, entity.accel);
    if state.speed < speed {
        state.speed = speed.min(state.speed + accel);
        game.movers.angular_moves.insert(actor.clone(), state.clone());
    }
    let delta = sub3(state.destination, game.body_of(actor.clone()).angles);
    let time = f64::from(length3(delta)) / state.speed;
    let frame_seconds = game.host.frame_seconds();
    if time < frame_seconds {
        return angle_move_final(actor, game);
    }
    game.require_entity_mut(&actor).angular_velocity = scale3(delta, (1.0 / time) as f32);
    let motion = game.require_entity(&actor).motion;
    game.set_motion_kind(actor.clone(), motion);
    if state.speed >= speed {
        game.schedule(
            actor,
            (time / frame_seconds).floor() * frame_seconds,
            angle_move_final,
        );
    } else {
        game.schedule(actor, frame_seconds, angle_move_begin);
    }
}
