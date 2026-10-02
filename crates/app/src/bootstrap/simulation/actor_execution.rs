//! Per-actor execution dispatch across families.
//!
//! Absolute donor:
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/actor-execution.ts`
//!
//! Covers motion/collision/flag projection (`actorMotion`, `actorCollision`,
//! `actorFlags`, `writeActorFlags`) and per-frame execution (`executeActor`,
//! `executeQuakeCPhysics`) for QuakeC, Q1, Q2, and Q3 entries. Q1/Q2 entries
//! resolve their entity from the real services tables on every call, matching
//! the donor's live object references; the Q2 monster table is read through
//! the game services the same way. Bodies, scheduler, and the two shared-
//! physics calls arrive through narrow seams so engine tables (which need the
//! actor registry) stay behind the session boundary; the engine passes the
//! real [`SharedPhysics`].

use std::collections::HashSet;

use qa_content::contract::{ContentId, SessionActorRegistry};
use qa_content::q1::foundation::entity_services::Q1EntityServices;
use qa_content::q1::foundation::types::{Q1Edition, Q1MoveType, Q1Solid};
use qa_content::q1::Q1Error;
use qa_content::q2::foundation::host::{Q2GameServices, Q2Motion, Q2MotionKind, Q2Solid};
use qa_content::q2::foundation::monsters::types::MonsterLocomotion;
use qa_core::identity::{same_actor, ActorId, OwnedActor, ProviderId};
use qa_core::math::Vec3;
use qa_core::time::{FrameContext, FramePhase, SourceTime};
use qa_world::body::BodyState;
use qa_world::movement::types::MovementError;
use qa_world::scheduler::{ScheduledThink, ThinkTiming};
use thiserror::Error;

use super::new_toss::NewTossOutcome;
use super::physics::{
    PhysicsError, PhysicsFamily, SharedPhysics, SharedPhysicsFlags, SharedSolid, SolidKind, StepOutcome,
};

/// Live QuakeC source surface consumed by actor execution.
///
/// Mirror of `QuakeCSource` from donor
/// `src/app/bootstrap/simulation/quakec-source.ts` (canonical home: the
/// quakec-source-lane port of that module); unify post-merge. The pusher step
/// runs over the source's own pusher services, matching the donor's
/// `stepQ1Pusher` call with `source.pusherServices`.
pub trait QuakeCSource {
    /// Whether an actor is a reserved client slot.
    fn is_reserved_client(&self, actor: &ActorId) -> bool;
    /// Claim one execution of an actor for this frame.
    fn run_actor_once(&mut self, actor: &ActorId, frame: &FrameContext) -> bool;
    /// Read the source movement type word, if the actor is bound.
    fn read_move_type(&self, actor: &ActorId) -> Option<i32>;
    /// Run the source think for an actor.
    fn run_think(&mut self, actor: &OwnedActor, frame: &FrameContext);
    /// Run the source water-transition check for an actor.
    fn check_water_transition(&mut self, actor: &OwnedActor);
    /// Read live source motion for an actor.
    fn motion(&self, actor: &OwnedActor, body: &BodyState) -> Option<Q2Motion>;
    /// Read live source collision for an actor.
    fn collision(&self, actor: &OwnedActor) -> Option<SharedSolid>;
    /// Read live source physics flags for an actor.
    fn flags(&self, actor: &OwnedActor) -> SharedPhysicsFlags;
    /// Write source physics flags for an actor.
    fn write_flags(&mut self, actor: &OwnedActor, changes: &SharedPhysicsFlags);
    /// Step a source pusher (`stepQ1Pusher` over `pusherServices`).
    fn step_pusher(&mut self, actor: &ActorId, elapsed_seconds: f64) -> Result<(), MovementError>;
}

/// Body table surface used by actor execution.
///
/// Seam over donor `SharedBodyTable` (`src/world/actors/body.ts`); canonical
/// home `qa_world::body::BodyTable`, whose registry-taking API the engine
/// adapts. Unify post-merge.
pub trait ExecutionBodies {
    /// Read a body record.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body record.
    fn write(&mut self, actor: &OwnedActor, state: BodyState);
    /// Link a body after a write.
    fn link(&mut self, actor: &OwnedActor);
}

/// Think scheduler surface used by actor execution.
///
/// Seam over donor `FrameScheduler` (`src/world/scheduler.ts`); canonical
/// home `qa_world::scheduler::Scheduler`, whose registry/resolver API the
/// engine adapts. Unify post-merge.
pub trait ExecutionScheduler {
    /// Run the pending think at the during-physics boundary.
    fn run(&mut self, actor: &ActorId, frame: &FrameContext);
    /// Pending think for an actor.
    fn pending(&self, actor: &ActorId) -> Option<ScheduledThink>;
    /// Schedule a think, replacing any pending think for the slot.
    fn schedule(&mut self, actor: &OwnedActor, callback: &str, timing: ThinkTiming);
}

/// Shared-physics calls used by actor execution.
///
/// Narrowing over [`SharedPhysics`] matching the donor's `Pick` surfaces;
/// the engine passes the real physics.
pub trait ExecutionPhysics {
    /// Step one actor's physics.
    fn step_actor(&mut self, actor: &OwnedActor, elapsed: f64) -> Result<StepOutcome, PhysicsError>;
    /// Push a team of actors, reporting the blocker.
    fn push_execution_team(&mut self, team: &[OwnedActor], elapsed: f64) -> Result<Option<ActorId>, PhysicsError>;
}

impl ExecutionPhysics for SharedPhysics {
    fn step_actor(&mut self, actor: &OwnedActor, elapsed: f64) -> Result<StepOutcome, PhysicsError> {
        self.step(actor, elapsed)
    }

    fn push_execution_team(&mut self, team: &[OwnedActor], elapsed: f64) -> Result<Option<ActorId>, PhysicsError> {
        self.push_team(team, elapsed)
    }
}

/// Per-actor execution entry (`ActorExecution`).
pub enum ActorExecution<'a> {
    /// QuakeC-driven actor.
    QuakeC {
        /// Owning actor handle.
        actor: OwnedActor,
        /// Live source.
        source: Box<dyn QuakeCSource + 'a>,
        /// Content identity.
        content: ContentId,
    },
    /// Native Q1 entity; the entity resolves from services on every call.
    Q1 {
        /// Owning actor handle.
        actor: OwnedActor,
        /// Entity services table.
        services: &'a mut Q1EntityServices,
        /// Content identity.
        content: ContentId,
    },
    /// Native Q2 entity; entity and monster resolve from the game on every call.
    Q2 {
        /// Owning actor handle.
        actor: OwnedActor,
        /// Game services arena.
        game: &'a mut Q2GameServices,
        /// Content identity.
        content: ContentId,
    },
    /// Q3 actor stepped by the Q3 arm.
    Q3 {
        /// Owning actor handle.
        actor: OwnedActor,
        /// Owning actor.
        owner: ActorId,
        /// Owning provider.
        provider: ProviderId,
        /// Content identity.
        content: ContentId,
        /// Millisecond frame step.
        step: Box<dyn FnMut(i64, i64) + 'a>,
    },
    /// Q3-source actor with native motion and collision.
    Q3Source {
        /// Owning actor handle.
        actor: OwnedActor,
        /// Owning provider.
        provider: ProviderId,
        /// Content identity.
        content: ContentId,
        /// Native motion reader.
        motion: Box<dyn Fn(&BodyState) -> Q2Motion + 'a>,
        /// Native collision reader.
        collision: Box<dyn Fn() -> SharedSolid + 'a>,
        /// Millisecond frame step.
        step: Box<dyn FnMut(i64, i64) + 'a>,
    },
}

/// Frame context for actor execution (`ActorExecutionFrame`).
pub struct ActorExecutionFrame<'a> {
    /// Live-actor registry.
    pub actors: &'a dyn SessionActorRegistry,
    /// Body table.
    pub bodies: &'a mut dyn ExecutionBodies,
    /// Shared physics calls.
    pub physics: &'a mut dyn ExecutionPhysics,
    /// Think scheduler.
    pub scheduler: &'a mut dyn ExecutionScheduler,
    /// Current frame.
    pub frame: FrameContext,
    /// Current time in seconds.
    pub time_seconds: f64,
    /// Frame interval in seconds.
    pub elapsed: f64,
    /// Executed team members by actor id. The donor keeps a reference set of
    /// `OwnedActor`; id equality matches `sameActor` and survives re-resolved
    /// handles.
    pub visited: HashSet<ActorId>,
}

/// Narrow context for QuakeC physics (`executeQuakeCPhysics` inputs).
pub struct QuakeCPhysicsContext<'a> {
    /// Live-actor registry.
    pub actors: &'a dyn SessionActorRegistry,
    /// Body table.
    pub bodies: &'a mut dyn ExecutionBodies,
    /// Current frame.
    pub frame: FrameContext,
    /// Frame interval in seconds.
    pub elapsed: f64,
    /// Shared physics calls.
    pub physics: &'a mut dyn ExecutionPhysics,
}

/// Actor execution failures.
#[derive(Debug, Error)]
pub enum ActorExecutionError {
    /// The entry's actor has no execution entity.
    #[error("actor {0:?} has no execution entity")]
    MissingEntity(ActorId),
    /// The source reports no live motion.
    #[error("missing live QC motion")]
    MissingQuakeCMotion,
    /// The source reports no live collision.
    #[error("missing live QC collision")]
    MissingQuakeCCollision,
    /// The source movement type word is not executable.
    #[error("unsupported nonclient QuakeC movetype {0}")]
    UnsupportedMoveType(i32),
    /// Q1 gib movement needs the rerelease physics edition.
    #[error("Q1 gib movement requires rerelease engine behavior")]
    GibNeedsRerelease,
    /// Q3 entries execute through the Q3 arm, never `executeActor`.
    #[error("Q3 entries execute through the Q3 arm, not executeActor")]
    Q3NotExecutable,
    /// Q1 services failure.
    #[error(transparent)]
    Q1(#[from] Q1Error),
    /// Shared physics failure.
    #[error(transparent)]
    Physics(#[from] PhysicsError),
    /// Source pusher failure.
    #[error(transparent)]
    Pusher(#[from] MovementError),
}

const DOWN: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: -1.0,
};
const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

fn brush_model(path: &str) -> Option<u32> {
    let digits = path.strip_prefix('*')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn due_seconds(due: &SourceTime) -> f64 {
    match due {
        SourceTime::Seconds(value) => f64::from(*value),
        SourceTime::Milliseconds(value) => f64::from(*value) / 1000.0,
    }
}

#[allow(clippy::cast_possible_truncation)]
fn bump_due(due: &SourceTime, elapsed: f64) -> SourceTime {
    match due {
        SourceTime::Seconds(value) => SourceTime::Seconds((f64::from(*value) + elapsed) as f32),
        SourceTime::Milliseconds(value) => SourceTime::Milliseconds(value + (elapsed * 1000.0) as i32),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn f32_add(base: f32, delta: f64) -> f32 {
    (f64::from(base) + delta) as f32
}

fn q1_edition(services: &Q1EntityServices) -> Q1Edition {
    let options = services.options();
    options.physics_edition.unwrap_or(options.edition)
}

/// Project an entry's motion (`actorMotion`).
pub fn actor_motion(entry: &ActorExecution, body: &BodyState) -> Result<Q2Motion, ActorExecutionError> {
    match entry {
        ActorExecution::Q3Source { motion, .. } => Ok(motion(body)),
        ActorExecution::QuakeC { actor, source, .. } => source
            .motion(actor, body)
            .ok_or(ActorExecutionError::MissingQuakeCMotion),
        ActorExecution::Q3 { actor, owner, .. } => Ok(Q2Motion {
            actor: actor.clone(),
            velocity: body.velocity,
            angular_velocity: ZERO,
            kind: Q2MotionKind::Stationary,
            gravity: 1.0,
            gravity_vector: DOWN,
            clip_mask: 0x600_0001,
            owner: Some(owner.clone()),
        }),
        ActorExecution::Q2 { actor, game, .. } => {
            let entity = game
                .entity(actor.id())
                .ok_or_else(|| ActorExecutionError::MissingEntity(actor.id().clone()))?;
            Ok(Q2Motion {
                actor: entity.actor.clone(),
                velocity: body.velocity,
                angular_velocity: entity.angular_velocity,
                kind: entity.motion,
                gravity: entity.gravity,
                gravity_vector: entity.gravity_vector,
                clip_mask: entity.clip_mask,
                owner: entity.owner.clone(),
            })
        }
        ActorExecution::Q1 { actor, services, .. } => {
            let entity = services
                .entity(actor.id())
                .ok_or_else(|| ActorExecutionError::MissingEntity(actor.id().clone()))?;
            if entity.movement == Q1MoveType::Gib && q1_edition(services) != Q1Edition::Rerelease {
                return Err(ActorExecutionError::GibNeedsRerelease);
            }
            let kind = match entity.movement {
                Q1MoveType::Gib => Q2MotionKind::Bounce,
                Q1MoveType::Flymissile => Q2MotionKind::FlyMissile,
                Q1MoveType::None | Q1MoveType::Noclip => Q2MotionKind::Stationary,
                Q1MoveType::Push => Q2MotionKind::Push,
                Q1MoveType::Step => Q2MotionKind::Step,
                Q1MoveType::Toss => Q2MotionKind::Toss,
                Q1MoveType::Bounce => Q2MotionKind::Bounce,
                Q1MoveType::Fly => Q2MotionKind::Fly,
            };
            let gravity = entity.number("gravity");
            Ok(Q2Motion {
                actor: entity.actor.clone(),
                velocity: body.velocity,
                angular_velocity: entity.angular_velocity,
                kind,
                gravity: if gravity == 0.0 || gravity.is_nan() {
                    1.0
                } else {
                    gravity
                },
                gravity_vector: DOWN,
                clip_mask: 0x600_0003,
                owner: entity.owner.clone(),
            })
        }
    }
}

/// Project an entry's collision (`actorCollision`).
pub fn actor_collision(entry: &ActorExecution) -> Result<SharedSolid, ActorExecutionError> {
    match entry {
        ActorExecution::Q3Source { collision, .. } => Ok(collision()),
        ActorExecution::QuakeC { actor, source, .. } => source
            .collision(actor)
            .ok_or(ActorExecutionError::MissingQuakeCCollision),
        ActorExecution::Q3 { owner, .. } => Ok(SharedSolid {
            solid: SolidKind::None,
            model: None,
            family: PhysicsFamily::Q3,
            owner: Some(owner.clone()),
            monster: None,
            dead_monster: None,
            q1_corpse: false,
            item: None,
        }),
        ActorExecution::Q1 { actor, services, .. } => {
            let entity = services
                .entity(actor.id())
                .ok_or_else(|| ActorExecutionError::MissingEntity(actor.id().clone()))?;
            if entity.solid == Q1Solid::Corpse {
                let rerelease = q1_edition(services) == Q1Edition::Rerelease;
                return Ok(SharedSolid {
                    solid: if rerelease { SolidKind::Box } else { SolidKind::None },
                    model: None,
                    family: PhysicsFamily::Q1,
                    owner: entity.owner.clone(),
                    monster: None,
                    dead_monster: None,
                    q1_corpse: rerelease,
                    item: None,
                });
            }
            let model = if entity.model.is_empty() {
                &entity.original_model
            } else {
                &entity.model
            };
            Ok(SharedSolid {
                solid: match entity.solid {
                    Q1Solid::None => SolidKind::None,
                    Q1Solid::Trigger => SolidKind::Trigger,
                    Q1Solid::Bsp => SolidKind::Brush,
                    Q1Solid::Bbox | Q1Solid::Slidebox | Q1Solid::Corpse => SolidKind::Box,
                },
                model: brush_model(model),
                family: PhysicsFamily::Q1,
                owner: entity.owner.clone(),
                monster: Some(entity.monster.is_some()),
                dead_monster: None,
                q1_corpse: false,
                item: Some(entity.movement_flags & 256 != 0),
            })
        }
        ActorExecution::Q2 { actor, game, .. } => {
            let entity = game
                .entity(actor.id())
                .ok_or_else(|| ActorExecutionError::MissingEntity(actor.id().clone()))?;
            Ok(SharedSolid {
                solid: match entity.solid {
                    Q2Solid::None => SolidKind::None,
                    Q2Solid::Trigger => SolidKind::Trigger,
                    Q2Solid::Box => SolidKind::Box,
                    Q2Solid::Brush => SolidKind::Brush,
                },
                model: brush_model(&entity.model),
                family: PhysicsFamily::Q2,
                owner: entity.owner.clone(),
                monster: Some(entity.server_flags & 4 != 0),
                dead_monster: Some(entity.server_flags & 2 != 0),
                q1_corpse: false,
                item: None,
            })
        }
    }
}

/// Project an entry's physics flags (`actorFlags`).
pub fn actor_flags(entry: &ActorExecution) -> Result<SharedPhysicsFlags, ActorExecutionError> {
    match entry {
        ActorExecution::QuakeC { actor, source, .. } => Ok(source.flags(actor)),
        ActorExecution::Q3 { .. } | ActorExecution::Q3Source { .. } => Ok(SharedPhysicsFlags::default()),
        ActorExecution::Q1 { actor, services, .. } => {
            let entity = services
                .entity(actor.id())
                .ok_or_else(|| ActorExecutionError::MissingEntity(actor.id().clone()))?;
            Ok(SharedPhysicsFlags {
                fly: Some(entity.movement_flags & 1 != 0),
                swim: Some(entity.movement_flags & 2 != 0),
                partial_ground: Some(entity.movement_flags & 1024 != 0),
                water_level: Some(entity.water_level),
                water_type: Some(entity.water_type),
                enemy: entity.monster.as_ref().and_then(|monster| monster.enemy.clone()),
                ..SharedPhysicsFlags::default()
            })
        }
        ActorExecution::Q2 { actor, game, .. } => {
            let entity = game
                .entity(actor.id())
                .ok_or_else(|| ActorExecutionError::MissingEntity(actor.id().clone()))?;
            let mut flags = SharedPhysicsFlags {
                team_slave: Some(entity.flags & 1024 != 0),
                always_touch: Some(entity.flags & 0x1000_0000 != 0),
                ..SharedPhysicsFlags::default()
            };
            if let Some(monster) = game.monsters.states.get(actor.id()) {
                flags.fly = Some(monster.locomotion == MonsterLocomotion::Fly);
                flags.swim = Some(monster.locomotion == MonsterLocomotion::Swim);
                flags.dead = Some(monster.dead);
                flags.water_level = Some(i32::from(monster.water_level));
                flags.water_type = Some(monster.water_type);
            }
            Ok(flags)
        }
    }
}

/// Write physics flag changes back to an entry (`writeActorFlags`).
pub fn write_actor_flags(entry: &mut ActorExecution, changes: &SharedPhysicsFlags) -> Result<(), ActorExecutionError> {
    match entry {
        ActorExecution::QuakeC { actor, source, .. } => {
            source.write_flags(actor, changes);
            Ok(())
        }
        ActorExecution::Q3 { .. } | ActorExecution::Q3Source { .. } => Ok(()),
        ActorExecution::Q1 { actor, services, .. } => {
            let id = actor.id().clone();
            let water_level = changes.water_level;
            let water_type = changes.water_type;
            services.update_entity(&id, |entity| {
                if let Some(level) = water_level {
                    entity.water_level = level;
                }
                if let Some(kind) = water_type {
                    if matches!(kind, -6..=0) {
                        entity.water_type = kind;
                    }
                }
            })?;
            Ok(())
        }
        ActorExecution::Q2 { actor, game, .. } => {
            if let Some(monster) = game.monsters.states.get_mut(actor.id()) {
                if let Some(level) = changes.water_level.filter(|level| (0..=3).contains(level)) {
                    monster.water_level = u8::try_from(level).unwrap_or(monster.water_level);
                }
                if let Some(kind) = changes.water_type {
                    monster.water_type = kind;
                }
            }
            Ok(())
        }
    }
}

/// Execute one non-Q3 entry (`executeActor`).
pub fn execute_actor(entry: &mut ActorExecution, context: &mut ActorExecutionFrame) -> Result<(), ActorExecutionError> {
    match entry {
        ActorExecution::QuakeC { actor, source, .. } => {
            if source.is_reserved_client(actor.id()) {
                return Ok(());
            }
            if !source.run_actor_once(actor.id(), &context.frame) {
                return Ok(());
            }
            let mut narrow = QuakeCPhysicsContext {
                actors: context.actors,
                bodies: &mut *context.bodies,
                frame: context.frame,
                elapsed: context.elapsed,
                physics: &mut *context.physics,
            };
            execute_quake_c_physics(&mut **source, actor, &mut narrow)
        }
        ActorExecution::Q1 { actor, services, .. } => execute_q1_actor(actor, services, context),
        ActorExecution::Q2 { actor, game, .. } => execute_q2_actor(actor, game, context),
        ActorExecution::Q3 { .. } | ActorExecution::Q3Source { .. } => Err(ActorExecutionError::Q3NotExecutable),
    }
}

fn move_q1_noclip(actor: &OwnedActor, angular_velocity: Vec3, bodies: &mut dyn ExecutionBodies, elapsed: f64) {
    let Some(body) = bodies.read(actor.id()) else { return };
    bodies.write(
        actor,
        BodyState {
            origin: Vec3 {
                x: f32_add(body.origin.x, f64::from(body.velocity.x) * elapsed),
                y: f32_add(body.origin.y, f64::from(body.velocity.y) * elapsed),
                z: f32_add(body.origin.z, f64::from(body.velocity.z) * elapsed),
            },
            angles: Vec3 {
                x: f32_add(body.angles.x, f64::from(angular_velocity.x) * elapsed),
                y: f32_add(body.angles.y, f64::from(angular_velocity.y) * elapsed),
                z: f32_add(body.angles.z, f64::from(angular_velocity.z) * elapsed),
            },
            ..body
        },
    );
    bodies.link(actor);
}

/// Execute QuakeC movement dispatch (`executeQuakeCPhysics`).
pub fn execute_quake_c_physics(
    source: &mut dyn QuakeCSource,
    actor: &OwnedActor,
    context: &mut QuakeCPhysicsContext,
) -> Result<(), ActorExecutionError> {
    let Some(movement) = source.read_move_type(actor.id()) else {
        return Ok(());
    };
    if movement == 7 {
        source.step_pusher(actor.id(), context.elapsed)?;
        return Ok(());
    }
    if movement == 4 {
        context.physics.step_actor(actor, context.elapsed)?;
        if context.actors.is_live(actor.id()) {
            source.run_think(actor, &context.frame);
        }
        if context.actors.is_live(actor.id()) {
            source.check_water_transition(actor);
        }
        return Ok(());
    }
    if !matches!(movement, 0 | 5 | 6 | 8 | 9 | 10) {
        return Err(ActorExecutionError::UnsupportedMoveType(movement));
    }
    source.run_think(actor, &context.frame);
    if !context.actors.is_live(actor.id()) || movement == 0 {
        return Ok(());
    }
    if movement == 8 {
        let body = context.bodies.read(actor.id());
        let motion = body.as_ref().and_then(|body| source.motion(actor, body));
        if let Some(motion) = motion {
            move_q1_noclip(actor, motion.angular_velocity, context.bodies, context.elapsed);
        }
    } else {
        context.physics.step_actor(actor, context.elapsed)?;
    }
    Ok(())
}

fn execute_q1_actor(
    actor: &OwnedActor,
    services: &mut Q1EntityServices,
    context: &mut ActorExecutionFrame,
) -> Result<(), ActorExecutionError> {
    let (movement, move_complete, angular_velocity) = {
        let entity = services
            .entity(actor.id())
            .ok_or_else(|| ActorExecutionError::MissingEntity(actor.id().clone()))?;
        (
            entity.movement,
            entity.move_completion.is_none(),
            entity.angular_velocity,
        )
    };
    let pusher = movement == Q1MoveType::Push;
    let step = movement == Q1MoveType::Step;
    let frame = FrameContext {
        phase: FramePhase::EntityThink,
        ..context.frame
    };
    if !pusher && !step {
        context.scheduler.run(actor.id(), &frame);
    }
    if context.actors.is_live(actor.id()) {
        if movement == Q1MoveType::Noclip {
            move_q1_noclip(actor, angular_velocity, context.bodies, context.elapsed);
        } else if step {
            context.physics.step_actor(actor, context.elapsed)?;
            let grounded = context
                .bodies
                .read(actor.id())
                .is_some_and(|body| body.ground.is_some());
            let id = actor.id().clone();
            services.update_entity(&id, |entity| {
                entity.movement_flags = (entity.movement_flags & !512) | (i32::from(grounded) * 512);
            })?;
        } else if move_complete
            && matches!(
                movement,
                Q1MoveType::Toss | Q1MoveType::Bounce | Q1MoveType::Fly | Q1MoveType::Flymissile
            )
        {
            services.apply_projectile_behavior(actor, context.time_seconds)?;
            context.physics.step_actor(actor, context.elapsed)?;
        } else {
            services.physics_entity(actor, context.time_seconds, context.elapsed)?;
        }
    }
    if step && context.actors.is_live(actor.id()) {
        context.scheduler.run(actor.id(), &frame);
    }
    if step && context.actors.is_live(actor.id()) {
        services.check_water_transition(actor.id())?;
    }
    Ok(())
}

fn execute_q2_actor(
    actor: &OwnedActor,
    game: &mut Q2GameServices,
    context: &mut ActorExecutionFrame,
) -> Result<(), ActorExecutionError> {
    let mut result = Ok(());
    game.run_actor(actor.id().clone(), |game| {
        result = execute_q2_body(actor, game, context);
    });
    result
}

#[allow(clippy::too_many_lines)]
fn execute_q2_body(
    actor: &OwnedActor,
    game: &mut Q2GameServices,
    context: &mut ActorExecutionFrame,
) -> Result<(), ActorExecutionError> {
    game.pre_physics(actor.id().clone());
    if !context.actors.is_live(actor.id()) {
        return Ok(());
    }
    let frame = FrameContext {
        phase: FramePhase::EntityThink,
        ..context.frame
    };
    let motion = game
        .entity(actor.id())
        .ok_or_else(|| ActorExecutionError::MissingEntity(actor.id().clone()))?
        .motion;
    if motion == Q2MotionKind::Push || motion == Q2MotionKind::Stop {
        let team = game.push_team(actor.id());
        if let Some(master) = team.first() {
            if !same_actor(master.id(), actor.id()) {
                return Ok(());
            }
        }
        for member in &team {
            context.visited.insert(member.id().clone());
        }
        let blocked = context.physics.push_execution_team(&team, context.elapsed)?;
        if blocked.is_some() {
            for member in &team {
                if let Some(pending) = context.scheduler.pending(member.id()) {
                    if !context.actors.is_live(member.id()) {
                        continue;
                    }
                    let due = bump_due(&pending.timing.due, context.elapsed);
                    let mut timing = pending.timing.clone();
                    timing.due = due;
                    context.scheduler.schedule(member, &pending.callback, timing);
                    if let Some(part) = game.entity_mut(member.id()) {
                        part.next_think = Some(due_seconds(&due));
                    }
                }
            }
        } else {
            for member in &team {
                if context.actors.is_live(member.id()) {
                    context.scheduler.run(member.id(), &frame);
                }
            }
        }
        for member in &team {
            if context.actors.is_live(member.id()) {
                game.post_physics(member.id().clone());
            }
        }
        return Ok(());
    }
    let after = motion == Q2MotionKind::Step;
    if !after {
        context.scheduler.run(actor.id(), &frame);
    }
    if context.actors.is_live(actor.id()) {
        game.apply_projectile_behavior(actor.id().clone(), context.time_seconds);
    }
    let moved = if context.actors.is_live(actor.id()) {
        Some(context.physics.step_actor(actor, context.elapsed)?)
    } else {
        None
    };
    let new_toss_moved = motion == Q2MotionKind::NewToss && moved == Some(Some(NewTossOutcome::Moved));
    let legacy_toss = matches!(
        motion,
        Q2MotionKind::Toss
            | Q2MotionKind::Bounce
            | Q2MotionKind::Fly
            | Q2MotionKind::FlyMissile
            | Q2MotionKind::WallBounce
    );
    if context.actors.is_live(actor.id()) && (new_toss_moved || legacy_toss) {
        if let Some(body) = context.bodies.read(actor.id()) {
            let mut next = game.entity(actor.id()).and_then(|entity| entity.team_chain.clone());
            while let Some(follower_id) = next {
                let follower = game.entity(&follower_id);
                let Some(follower) = follower else { break };
                let follower_actor = follower.actor.clone();
                next = follower.team_chain.clone();
                if let Some(mut current) = context.bodies.read(follower_actor.id()) {
                    current.origin = body.origin;
                    context.bodies.write(&follower_actor, current);
                    context.bodies.link(&follower_actor);
                }
            }
        }
    }
    if after && context.actors.is_live(actor.id()) {
        context.scheduler.run(actor.id(), &frame);
    }
    if context.actors.is_live(actor.id()) {
        game.post_physics(actor.id().clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, HashMap};
    use std::rc::Rc;

    use qa_content::contract::SessionActorRegistry;
    use qa_content::q1::foundation::entity::{Q1Actor, Q1Monster, Q1MonsterMode, Q1MonsterSpecies};
    use qa_content::q1::foundation::types::Q1FoundationOptions;
    use qa_content::q2::foundation::monsters::types::{
        MonsterAttackState, MonsterMove, MonsterPowerArmor, MonsterSpawner, MonsterState, MonsterWeapon,
    };
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{vec3, Bounds};
    use qa_core::time::SourceTime;
    use qa_world::scheduler::{InvocationOrder, ThinkBoundary, ThinkTiming};

    use super::super::test_hosts::{test_q1_game, test_q1_game_with_options, test_q1_options, test_q2_game};
    use super::*;

    fn content() -> ContentId {
        ContentId("q1:id1:e1m1:1".to_string())
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("actor-execution-test").expect("owner")
    }

    fn frame_context() -> FrameContext {
        FrameContext {
            frame: 1,
            time: SourceTime::Seconds(1.0),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::FrameEntry,
        }
    }

    fn body_state() -> BodyState {
        BodyState {
            origin: vec3(10.0, 20.0, 30.0),
            angles: vec3(0.0, 90.0, 0.0),
            velocity: vec3(100.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            ground: None,
        }
    }

    struct FakeActors {
        live: HashSet<ActorId>,
    }

    impl FakeActors {
        fn new(live: &[ActorId]) -> Self {
            Self {
                live: live.iter().cloned().collect(),
            }
        }
    }

    impl SessionActorRegistry for FakeActors {
        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, _actor: &ActorId) -> Option<OwnedActor> {
            None
        }
    }

    struct FakeBodies {
        bodies: HashMap<ActorId, BodyState>,
        linked: Vec<ActorId>,
    }

    impl FakeBodies {
        fn new() -> Self {
            Self {
                bodies: HashMap::new(),
                linked: Vec::new(),
            }
        }
    }

    impl ExecutionBodies for FakeBodies {
        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: BodyState) {
            self.bodies.insert(actor.id().clone(), state);
        }

        fn link(&mut self, actor: &OwnedActor) {
            self.linked.push(actor.id().clone());
        }
    }

    struct FakeScheduler {
        pending: HashMap<ActorId, ScheduledThink>,
        runs: Vec<(ActorId, FramePhase)>,
        scheduled: Vec<(ActorId, String, ThinkTiming)>,
    }

    impl FakeScheduler {
        fn new() -> Self {
            Self {
                pending: HashMap::new(),
                runs: Vec::new(),
                scheduled: Vec::new(),
            }
        }
    }

    impl ExecutionScheduler for FakeScheduler {
        fn run(&mut self, actor: &ActorId, frame: &FrameContext) {
            self.runs.push((actor.clone(), frame.phase));
        }

        fn pending(&self, actor: &ActorId) -> Option<ScheduledThink> {
            self.pending.get(actor).cloned()
        }

        fn schedule(&mut self, actor: &OwnedActor, callback: &str, timing: ThinkTiming) {
            self.scheduled
                .push((actor.id().clone(), callback.to_string(), timing.clone()));
            self.pending.insert(
                actor.id().clone(),
                ScheduledThink {
                    actor: actor.id().clone(),
                    callback: callback.to_string(),
                    timing,
                },
            );
        }
    }

    struct FakePhysics {
        step_result: StepOutcome,
        push_result: Option<ActorId>,
        steps: Vec<ActorId>,
        pushes: Vec<Vec<ActorId>>,
    }

    impl FakePhysics {
        fn new() -> Self {
            Self {
                step_result: None,
                push_result: None,
                steps: Vec::new(),
                pushes: Vec::new(),
            }
        }
    }

    impl ExecutionPhysics for FakePhysics {
        fn step_actor(&mut self, actor: &OwnedActor, _elapsed: f64) -> Result<StepOutcome, PhysicsError> {
            self.steps.push(actor.id().clone());
            Ok(self.step_result)
        }

        fn push_execution_team(&mut self, team: &[OwnedActor], _elapsed: f64) -> Result<Option<ActorId>, PhysicsError> {
            self.pushes
                .push(team.iter().map(|member| member.id().clone()).collect());
            Ok(self.push_result.clone())
        }
    }

    #[derive(Default)]
    struct QCLog {
        thinks: u32,
        water_checks: u32,
        pusher_steps: Vec<(ActorId, f64)>,
        written: Vec<SharedPhysicsFlags>,
    }

    struct FakeQuakeC {
        reserved: bool,
        run_once: bool,
        move_type: Option<i32>,
        motion: Option<Q2Motion>,
        collision: Option<SharedSolid>,
        flags: SharedPhysicsFlags,
        log: Rc<RefCell<QCLog>>,
    }

    impl FakeQuakeC {
        fn new(move_type: Option<i32>) -> Self {
            Self {
                reserved: false,
                run_once: true,
                move_type,
                motion: None,
                collision: None,
                flags: SharedPhysicsFlags::default(),
                log: Rc::new(RefCell::new(QCLog::default())),
            }
        }
    }

    impl QuakeCSource for FakeQuakeC {
        fn is_reserved_client(&self, _actor: &ActorId) -> bool {
            self.reserved
        }

        fn run_actor_once(&mut self, _actor: &ActorId, _frame: &FrameContext) -> bool {
            self.run_once
        }

        fn read_move_type(&self, _actor: &ActorId) -> Option<i32> {
            self.move_type
        }

        fn run_think(&mut self, _actor: &OwnedActor, _frame: &FrameContext) {
            self.log.borrow_mut().thinks += 1;
        }

        fn check_water_transition(&mut self, _actor: &OwnedActor) {
            self.log.borrow_mut().water_checks += 1;
        }

        fn motion(&self, _actor: &OwnedActor, _body: &BodyState) -> Option<Q2Motion> {
            self.motion.clone()
        }

        fn collision(&self, _actor: &OwnedActor) -> Option<SharedSolid> {
            self.collision.clone()
        }

        fn flags(&self, _actor: &OwnedActor) -> SharedPhysicsFlags {
            self.flags.clone()
        }

        fn write_flags(&mut self, _actor: &OwnedActor, changes: &SharedPhysicsFlags) {
            self.log.borrow_mut().written.push(changes.clone());
        }

        fn step_pusher(&mut self, actor: &ActorId, elapsed_seconds: f64) -> Result<(), MovementError> {
            self.log
                .borrow_mut()
                .pusher_steps
                .push((actor.clone(), elapsed_seconds));
            Ok(())
        }
    }

    fn q1_owner() -> ProviderId {
        ProviderId::new("q1", "game")
    }

    fn admit_q1(game: &mut Q1EntityServices, owner: &IdentityOwner, slot: u32) -> OwnedActor {
        let actor = owner.actor(slot, 0);
        let owned = owner.owned_actor(&actor, q1_owner()).expect("owned");
        game.entities
            .insert(actor, Q1Actor::new(owned.clone(), "monster_demon", Some(1), None));
        owned
    }

    fn admit_q2(game: &mut Q2GameServices) -> OwnedActor {
        let actor = game.create("monster_soldier", BTreeMap::new());
        game.owned_of(actor)
    }

    fn q1_monster(enemy: Option<ActorId>) -> Q1Monster {
        Q1Monster {
            species: Q1MonsterSpecies::Army,
            mode: Q1MonsterMode::Stand,
            frame_index: 0,
            sequence: Vec::new(),
            first_frame: 0,
            enemy,
            old_enemy: None,
            path: String::new(),
            pause_until: 0.0,
            attack_finished: 0.0,
            pain_finished: 0.0,
            search_until: 0.0,
            death_drop: false,
            refired: false,
        }
    }

    fn monster_state() -> MonsterState {
        MonsterState {
            kind: "monster_soldier".to_string(),
            weapon: MonsterWeapon::Blaster,
            locomotion: MonsterLocomotion::Fly,
            has_melee: false,
            has_ranged_attack: true,
            has_idle: false,
            has_search: false,
            blind_fire: false,
            good_guy: false,
            target_anger: false,
            ignore_shots: false,
            do_not_count: false,
            spawned_by: MonsterSpawner::None,
            commander: None,
            monster_slots: 0,
            monster_used: 0,
            brutal: false,
            medic: false,
            resurrecting: false,
            current_move: MonsterMove {
                name: "stand".to_string(),
                first_frame: 0,
                last_frame: 0,
                end: None,
                frames: Vec::new(),
                sidestep_scale: 0.0,
            },
            next_move: None,
            next_frame: 0,
            next_move_time: 0.0,
            scale: 1.0,
            gib_health: -40.0,
            initial_power_armor: MonsterPowerArmor::None,
            max_power_armor_power: 0.0,
            base_health: 30.0,
            health_scaling: 1.0,
            can_take_damage: true,
            dead: false,
            corpse: false,
            gibbed: false,
            stand_ground: false,
            temporary_stand_ground: false,
            hold_frame: false,
            ducked: false,
            dodging: false,
            charging: false,
            manual_steering: false,
            combat_point: false,
            attack_state: MonsterAttackState::Straight,
            lefty: false,
            ideal_yaw: 0.0,
            yaw_speed: 0.0,
            pause_time: 0.0,
            idle_time: 0.0,
            pain_time: 0.0,
            fire_wait: 0.0,
            duck_wait: 0.0,
            next_duck_time: 0.0,
            dodge_time: 0.0,
            attack_finished: 0.0,
            check_attack_time: 0.0,
            strafe_time: 0.0,
            had_visibility: false,
            close_sight_tripped: false,
            melee_time: 0.0,
            search_time: 0.0,
            trail_time: 0.0,
            show_hostile: 0.0,
            last_sighting: vec3(0.0, 0.0, 0.0),
            saved_goal: None,
            lost_sight: false,
            pursue_next: false,
            pursue_temporary: false,
            pursuit_last_seen: false,
            blind_fire_target: vec3(0.0, 0.0, 0.0),
            blind_fire_delay: 0.0,
            sound_target: None,
            old_enemy: None,
            move_target: None,
            combat_target: String::new(),
            cocked: false,
            force_refire: false,
            normal_height: 0.0,
            water_level: 1,
            water_type: -3,
            last_link_count: 0,
            air_finished: 0.0,
            environmental_damage_time: 0.0,
            jump_time: 0.0,
            flies_time: None,
            alternate_fly: false,
            fly_min_distance: 0.0,
            fly_max_distance: 0.0,
            fly_acceleration: 0.0,
            fly_speed: 0.0,
            fly_ideal_position: vec3(0.0, 0.0, 0.0),
            fly_position_time: 0.0,
            fly_buzzard: false,
            fly_above: false,
            fly_pinned: false,
            fly_thrusters: false,
            fly_recovery_time: 0.0,
            fly_recovery_direction: vec3(0.0, 0.0, 0.0),
            hint_path: false,
            pathing: None,
        }
    }

    fn think_timing(actor: &ActorId, owner: &ProviderId, due: SourceTime) -> ThinkTiming {
        ThinkTiming {
            execution_provider: None,
            due,
            boundary: ThinkBoundary::DuringPhysics,
            order: InvocationOrder {
                provider: owner.clone(),
                actor: actor.clone(),
                sequence: 0,
            },
        }
    }

    #[test]
    fn q1_motion_maps_move_types() {
        let owner = owner();
        let (mut game, _handles) = test_q1_game();
        let owned = admit_q1(&mut game, &owner, 3);
        let id = owned.id().clone();
        game.update_entity(&id, |entity| {
            entity.movement = Q1MoveType::Flymissile;
            entity.angular_velocity = vec3(0.0, 0.0, 5.0);
            entity.fields.insert("gravity".to_string(), "2".to_string());
        })
        .expect("tweak");
        let entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        let motion = actor_motion(&entry, &body_state()).expect("motion");
        assert_eq!(motion.kind, Q2MotionKind::FlyMissile);
        assert_eq!(motion.gravity, 2.0);
        assert_eq!(motion.clip_mask, 0x600_0003);
        assert_eq!(motion.gravity_vector, DOWN);
    }

    #[test]
    fn q1_motion_defaults_gravity_and_parks_noclip() {
        let owner = owner();
        let (mut game, _handles) = test_q1_game();
        let owned = admit_q1(&mut game, &owner, 3);
        let id = owned.id().clone();
        game.update_entity(&id, |entity| entity.movement = Q1MoveType::Noclip)
            .expect("tweak");
        let entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        let motion = actor_motion(&entry, &body_state()).expect("motion");
        assert_eq!(motion.kind, Q2MotionKind::Stationary);
        assert_eq!(motion.gravity, 1.0);
    }

    #[test]
    fn q1_gib_follows_physics_edition() {
        let owner = owner();
        let (mut game, _handles) = test_q1_game();
        let owned = admit_q1(&mut game, &owner, 3);
        let id = owned.id().clone();
        game.update_entity(&id, |entity| entity.movement = Q1MoveType::Gib)
            .expect("tweak");
        let entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        assert!(matches!(
            actor_motion(&entry, &body_state()),
            Err(ActorExecutionError::GibNeedsRerelease)
        ));

        let mut options = test_q1_options();
        options.physics_edition = Some(Q1Edition::Rerelease);
        let (mut game, _handles) = test_q1_game_with_options(options);
        let owned = admit_q1(&mut game, &owner, 4);
        let id = owned.id().clone();
        game.update_entity(&id, |entity| entity.movement = Q1MoveType::Gib)
            .expect("tweak");
        let entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        let motion = actor_motion(&entry, &body_state()).expect("rerelease gib");
        assert_eq!(motion.kind, Q2MotionKind::Bounce);
    }

    #[test]
    fn q1_collision_maps_solidity() {
        let owner = owner();
        let (mut game, _handles) = test_q1_game();
        let owned = admit_q1(&mut game, &owner, 3);
        let id = owned.id().clone();
        let enemy = owner.actor(9, 0);
        game.update_entity(&id, |entity| {
            entity.solid = Q1Solid::Bsp;
            entity.model = "*12".to_string();
            entity.movement_flags = 256;
            entity.monster = Some(q1_monster(Some(enemy)));
        })
        .expect("tweak");
        let entry = ActorExecution::Q1 {
            actor: owned.clone(),
            services: &mut game,
            content: content(),
        };
        let solid = actor_collision(&entry).expect("solid");
        assert_eq!(solid.solid, SolidKind::Brush);
        assert_eq!(solid.model, Some(12));
        assert_eq!(solid.monster, Some(true));
        assert_eq!(solid.item, Some(true));

        drop(entry);
        game.update_entity(&id, |entity| {
            entity.solid = Q1Solid::Corpse;
            entity.monster = None;
        })
        .expect("corpse");
        let entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        let solid = actor_collision(&entry).expect("corpse");
        assert_eq!(solid.solid, SolidKind::None);
        assert!(!solid.q1_corpse);
    }

    #[test]
    fn q1_corpse_is_box_on_rerelease() {
        let owner = owner();
        let mut options: Q1FoundationOptions = test_q1_options();
        options.physics_edition = Some(Q1Edition::Rerelease);
        let (mut game, _handles) = test_q1_game_with_options(options);
        let owned = admit_q1(&mut game, &owner, 3);
        let id = owned.id().clone();
        game.update_entity(&id, |entity| entity.solid = Q1Solid::Corpse)
            .expect("tweak");
        let entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        let solid = actor_collision(&entry).expect("corpse");
        assert_eq!(solid.solid, SolidKind::Box);
        assert!(solid.q1_corpse);
    }

    #[test]
    fn q1_flags_and_writes_round_trip() {
        let owner = owner();
        let (mut game, _handles) = test_q1_game();
        let owned = admit_q1(&mut game, &owner, 3);
        let id = owned.id().clone();
        let enemy = owner.actor(9, 0);
        game.update_entity(&id, |entity| {
            entity.movement_flags = 1 | 1024;
            entity.water_level = 2;
            entity.water_type = -3;
            entity.monster = Some(q1_monster(Some(enemy.clone())));
        })
        .expect("tweak");
        let entry = ActorExecution::Q1 {
            actor: owned.clone(),
            services: &mut game,
            content: content(),
        };
        let flags = actor_flags(&entry).expect("flags");
        assert_eq!(flags.fly, Some(true));
        assert_eq!(flags.swim, Some(false));
        assert_eq!(flags.partial_ground, Some(true));
        assert_eq!(flags.water_level, Some(2));
        assert_eq!(flags.enemy, Some(enemy));
        drop(entry);

        let mut entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        write_actor_flags(
            &mut entry,
            &SharedPhysicsFlags {
                water_level: Some(1),
                water_type: Some(-4),
                ..SharedPhysicsFlags::default()
            },
        )
        .expect("write");
        drop(entry);
        let entity = game.entity(&id).expect("entity");
        assert_eq!(entity.water_level, 1);
        assert_eq!(entity.water_type, -4);

        let mut entry = ActorExecution::Q1 {
            actor: owner.owned_actor(&id, q1_owner()).expect("owned"),
            services: &mut game,
            content: content(),
        };
        write_actor_flags(
            &mut entry,
            &SharedPhysicsFlags {
                water_type: Some(5),
                ..SharedPhysicsFlags::default()
            },
        )
        .expect("write");
        drop(entry);
        assert_eq!(game.entity(&id).expect("entity").water_type, -4);
    }

    #[test]
    fn q2_motion_collision_follow_entity() {
        let owner = owner();
        let (mut game, _handles) = test_q2_game();
        let owned = admit_q2(&mut game);
        let actor = owned.id().clone();
        let masters_owner = owner.actor(5, 0);
        game.entity_mut(&actor).expect("entity").motion = Q2MotionKind::Fly;
        game.entity_mut(&actor).expect("entity").model = "*3".to_string();
        game.entity_mut(&actor).expect("entity").solid = Q2Solid::Box;
        game.entity_mut(&actor).expect("entity").server_flags = 6;
        game.entity_mut(&actor).expect("entity").owner = Some(masters_owner.clone());
        let entry = ActorExecution::Q2 {
            actor: owned,
            game: &mut game,
            content: content(),
        };
        let motion = actor_motion(&entry, &body_state()).expect("motion");
        assert_eq!(motion.kind, Q2MotionKind::Fly);
        assert_eq!(motion.owner, Some(masters_owner));
        let solid = actor_collision(&entry).expect("solid");
        assert_eq!(solid.solid, SolidKind::Box);
        assert_eq!(solid.model, Some(3));
        assert_eq!(solid.monster, Some(true));
        assert_eq!(solid.dead_monster, Some(true));
    }

    #[test]
    fn q2_flags_follow_monster_locomotion() {
        let _owner = owner();
        let (mut game, _handles) = test_q2_game();
        let owned = admit_q2(&mut game);
        let actor = owned.id().clone();
        game.entity_mut(&actor).expect("entity").flags = 1024 | 0x1000_0000;
        let entry = ActorExecution::Q2 {
            actor: owned.clone(),
            game: &mut game,
            content: content(),
        };
        let flags = actor_flags(&entry).expect("flags");
        assert_eq!(flags.team_slave, Some(true));
        assert_eq!(flags.always_touch, Some(true));
        assert_eq!(flags.fly, None);
        drop(entry);

        game.monsters.states.insert(actor.clone(), monster_state());
        let entry = ActorExecution::Q2 {
            actor: owned,
            game: &mut game,
            content: content(),
        };
        let flags = actor_flags(&entry).expect("monster flags");
        assert_eq!(flags.fly, Some(true));
        assert_eq!(flags.swim, Some(false));
        assert_eq!(flags.dead, Some(false));
        assert_eq!(flags.water_level, Some(1));
        assert_eq!(flags.water_type, Some(-3));
    }

    #[test]
    fn q2_write_flags_updates_monster_water() {
        let _owner = owner();
        let (mut game, _handles) = test_q2_game();
        let owned = admit_q2(&mut game);
        let actor = owned.id().clone();
        let mut entry = ActorExecution::Q2 {
            actor: owned.clone(),
            game: &mut game,
            content: content(),
        };
        write_actor_flags(
            &mut entry,
            &SharedPhysicsFlags {
                water_level: Some(3),
                ..SharedPhysicsFlags::default()
            },
        )
        .expect("no monster");
        drop(entry);

        game.monsters.states.insert(actor.clone(), monster_state());
        let mut entry = ActorExecution::Q2 {
            actor: owned,
            game: &mut game,
            content: content(),
        };
        write_actor_flags(
            &mut entry,
            &SharedPhysicsFlags {
                water_level: Some(9),
                water_type: Some(-2),
                ..SharedPhysicsFlags::default()
            },
        )
        .expect("write");
        drop(entry);
        let monster = game.monsters.states.get(&actor).expect("monster");
        assert_eq!(monster.water_level, 1);
        assert_eq!(monster.water_type, -2);
    }

    #[test]
    fn q3_arms_project_and_reject_execution() {
        let owner = owner();
        let actor = owner.actor(3, 0);
        let owned = owner.owned_actor(&actor, q1_owner()).expect("owned");
        let provider = ProviderId::new("q3", "game");
        let entry = ActorExecution::Q3 {
            actor: owned,
            owner: actor.clone(),
            provider,
            content: content(),
            step: Box::new(|_, _| {}),
        };
        let motion = actor_motion(&entry, &body_state()).expect("motion");
        assert_eq!(motion.kind, Q2MotionKind::Stationary);
        assert_eq!(motion.clip_mask, 0x600_0001);
        assert_eq!(motion.owner, Some(actor.clone()));
        let solid = actor_collision(&entry).expect("solid");
        assert_eq!(solid.family, PhysicsFamily::Q3);
        assert_eq!(solid.solid, SolidKind::None);
        assert_eq!(actor_flags(&entry).expect("flags"), SharedPhysicsFlags::default());

        let actors = FakeActors::new(std::slice::from_ref(&actor));
        let mut bodies = FakeBodies::new();
        let mut physics = FakePhysics::new();
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q3 {
            actor: owner.owned_actor(&actor, q1_owner()).expect("owned"),
            owner: actor,
            provider: ProviderId::new("q3", "game"),
            content: content(),
            step: Box::new(|_, _| {}),
        };
        assert!(matches!(
            execute_actor(&mut entry, &mut frame),
            Err(ActorExecutionError::Q3NotExecutable)
        ));
    }

    #[test]
    fn q3_source_delegates_motion_and_collision() {
        let owner = owner();
        let actor = owner.actor(3, 0);
        let owned = owner.owned_actor(&actor, q1_owner()).expect("owned");
        let solid = SharedSolid {
            solid: SolidKind::Brush,
            model: Some(4),
            family: PhysicsFamily::Q3,
            owner: None,
            monster: None,
            dead_monster: None,
            q1_corpse: false,
            item: None,
        };
        let moved = solid.clone();
        let entry = ActorExecution::Q3Source {
            actor: owned.clone(),
            provider: ProviderId::new("q3", "game"),
            content: content(),
            motion: Box::new(move |body| Q2Motion {
                actor: owned.clone(),
                velocity: body.velocity,
                angular_velocity: ZERO,
                kind: Q2MotionKind::Fly,
                gravity: 0.0,
                gravity_vector: DOWN,
                clip_mask: 1,
                owner: None,
            }),
            collision: Box::new(move || moved.clone()),
            step: Box::new(|_, _| {}),
        };
        let motion = actor_motion(&entry, &body_state()).expect("motion");
        assert_eq!(motion.kind, Q2MotionKind::Fly);
        assert_eq!(actor_collision(&entry).expect("solid"), solid);
        assert_eq!(actor_flags(&entry).expect("flags"), SharedPhysicsFlags::default());
    }

    #[test]
    fn quakec_missing_projection_fails() {
        let owner = owner();
        let actor = owner.actor(3, 0);
        let owned = owner.owned_actor(&actor, q1_owner()).expect("owned");
        let entry = ActorExecution::QuakeC {
            actor: owned,
            source: Box::new(FakeQuakeC::new(Some(4))),
            content: content(),
        };
        assert!(matches!(
            actor_motion(&entry, &body_state()),
            Err(ActorExecutionError::MissingQuakeCMotion)
        ));
        assert!(matches!(
            actor_collision(&entry),
            Err(ActorExecutionError::MissingQuakeCCollision)
        ));
    }

    #[test]
    fn quakec_step_runs_think_and_water() {
        let owner = owner();
        let actor = owner.actor(3, 0);
        let owned = owner.owned_actor(&actor, q1_owner()).expect("owned");
        let actors = FakeActors::new(std::slice::from_ref(&actor));
        let mut bodies = FakeBodies::new();
        let mut physics = FakePhysics::new();
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let source = FakeQuakeC::new(Some(4));
        let log = source.log.clone();
        let mut entry = ActorExecution::QuakeC {
            actor: owned,
            source: Box::new(source),
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        drop(entry);
        drop(frame);
        assert_eq!(physics.steps, vec![actor]);
        let log = log.borrow();
        assert_eq!(log.thinks, 1);
        assert_eq!(log.water_checks, 1);
    }

    #[test]
    fn quakec_gating_skips_reserved_and_claimed() {
        let owner = owner();
        let actor = owner.actor(3, 0);
        let actors = FakeActors::new(std::slice::from_ref(&actor));
        for (reserved, run_once) in [(true, true), (false, false)] {
            let mut bodies = FakeBodies::new();
            let mut physics = FakePhysics::new();
            let mut scheduler = FakeScheduler::new();
            let mut frame = ActorExecutionFrame {
                actors: &actors,
                bodies: &mut bodies,
                physics: &mut physics,
                scheduler: &mut scheduler,
                frame: frame_context(),
                time_seconds: 1.0,
                elapsed: 0.1,
                visited: HashSet::new(),
            };
            let mut source = FakeQuakeC::new(Some(4));
            source.reserved = reserved;
            source.run_once = run_once;
            let mut entry = ActorExecution::QuakeC {
                actor: owner.owned_actor(&actor, q1_owner()).expect("owned"),
                source: Box::new(source),
                content: content(),
            };
            execute_actor(&mut entry, &mut frame).expect("execute");
            drop(entry);
            drop(frame);
            assert!(physics.steps.is_empty());
        }
    }

    #[test]
    fn quakec_write_flags_delegates() {
        let owner = owner();
        let actor = owner.actor(3, 0);
        let owned = owner.owned_actor(&actor, q1_owner()).expect("owned");
        let source = FakeQuakeC::new(Some(4));
        let log = source.log.clone();
        let mut entry = ActorExecution::QuakeC {
            actor: owned,
            source: Box::new(source),
            content: content(),
        };
        let changes = SharedPhysicsFlags {
            fly: Some(true),
            ..SharedPhysicsFlags::default()
        };
        write_actor_flags(&mut entry, &changes).expect("write");
        assert_eq!(log.borrow().written, vec![changes]);
    }

    #[test]
    fn quakec_physics_dispatches_move_types() {
        let owner = owner();
        let actor = owner.actor(3, 0);
        let owned = owner.owned_actor(&actor, q1_owner()).expect("owned");
        let actors = FakeActors::new(std::slice::from_ref(&actor));
        for (move_type, expect_pusher, expect_steps, expect_thinks) in
            [(Some(7), 1, 0, 0), (Some(0), 0, 0, 1), (None, 0, 0, 0)]
        {
            let mut bodies = FakeBodies::new();
            let mut physics = FakePhysics::new();
            let mut source = FakeQuakeC::new(move_type);
            {
                let mut context = QuakeCPhysicsContext {
                    actors: &actors,
                    bodies: &mut bodies,
                    frame: frame_context(),
                    elapsed: 0.1,
                    physics: &mut physics,
                };
                execute_quake_c_physics(&mut source, &owned, &mut context).expect("execute");
                assert_eq!(source.log.borrow().pusher_steps.len(), expect_pusher);
                assert_eq!(source.log.borrow().thinks, expect_thinks);
            }
            assert_eq!(physics.steps.len(), expect_steps);
        }

        let mut bodies = FakeBodies::new();
        let mut physics = FakePhysics::new();
        let mut source = FakeQuakeC::new(Some(3));
        {
            let mut context = QuakeCPhysicsContext {
                actors: &actors,
                bodies: &mut bodies,
                frame: frame_context(),
                elapsed: 0.1,
                physics: &mut physics,
            };
            assert!(matches!(
                execute_quake_c_physics(&mut source, &owned, &mut context),
                Err(ActorExecutionError::UnsupportedMoveType(3))
            ));
        }
    }

    #[test]
    fn quakec_noclip_advances_body() {
        let owner = owner();
        let actor = owner.actor(3, 0);
        let owned = owner.owned_actor(&actor, q1_owner()).expect("owned");
        let actors = FakeActors::new(std::slice::from_ref(&actor));
        let mut bodies = FakeBodies::new();
        bodies.bodies.insert(actor.clone(), body_state());
        let mut physics = FakePhysics::new();
        let mut source = FakeQuakeC::new(Some(8));
        source.motion = Some(Q2Motion {
            actor: owned.clone(),
            velocity: vec3(0.0, 0.0, 0.0),
            angular_velocity: vec3(0.0, 20.0, 0.0),
            kind: Q2MotionKind::Fly,
            gravity: 0.0,
            gravity_vector: DOWN,
            clip_mask: 0,
            owner: None,
        });
        {
            let mut context = QuakeCPhysicsContext {
                actors: &actors,
                bodies: &mut bodies,
                frame: frame_context(),
                elapsed: 0.5,
                physics: &mut physics,
            };
            execute_quake_c_physics(&mut source, &owned, &mut context).expect("execute");
        }
        let moved = bodies.bodies.get(&actor).expect("body");
        assert_eq!(moved.origin.x, 60.0);
        assert_eq!(moved.angles.y, 100.0);
        assert_eq!(bodies.linked, vec![actor]);
    }

    #[test]
    fn q1_noclip_execution_moves_body() {
        let owner = owner();
        let (mut game, _handles) = test_q1_game();
        let owned = admit_q1(&mut game, &owner, 3);
        let id = owned.id().clone();
        game.update_entity(&id, |entity| {
            entity.movement = Q1MoveType::Noclip;
            entity.angular_velocity = vec3(0.0, 10.0, 0.0);
        })
        .expect("tweak");
        let actors = FakeActors::new(std::slice::from_ref(&id));
        let mut bodies = FakeBodies::new();
        bodies.bodies.insert(id.clone(), body_state());
        let mut physics = FakePhysics::new();
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.5,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        drop(entry);
        drop(frame);
        let moved = bodies.bodies.get(&id).expect("body");
        assert_eq!(moved.origin.x, 60.0);
        assert_eq!(moved.angles.y, 95.0);
        assert!(physics.steps.is_empty());
        assert_eq!(scheduler.runs.len(), 1);
        assert!(matches!(scheduler.runs[0].1, FramePhase::EntityThink));
    }

    #[test]
    fn q1_step_execution_tracks_ground_flag() {
        let owner = owner();
        let (mut game, handles) = test_q1_game();
        let owned = admit_q1(&mut game, &owner, 3);
        let id = owned.id().clone();
        handles
            .bodies
            .admit_linked(&id, vec3(0.0, 0.0, 0.0), body_state().bounds);
        game.update_entity(&id, |entity| entity.movement = Q1MoveType::Step)
            .expect("tweak");
        let actors = FakeActors::new(std::slice::from_ref(&id));
        let mut bodies = FakeBodies::new();
        bodies.bodies.insert(id.clone(), body_state());
        let mut physics = FakePhysics::new();
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q1 {
            actor: owned,
            services: &mut game,
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        drop(entry);
        drop(frame);
        assert_eq!(physics.steps, vec![id.clone()]);
        // Step runs the think after physics, then the water check clears nothing here.
        assert_eq!(scheduler.runs.len(), 1);
        assert_eq!(game.entity(&id).expect("entity").movement_flags & 512, 0);

        let mut bodies = FakeBodies::new();
        let mut grounded = body_state();
        grounded.ground = Some(owner.actor(7, 0));
        bodies.bodies.insert(id.clone(), grounded);
        let mut physics = FakePhysics::new();
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q1 {
            actor: owner.owned_actor(&id, q1_owner()).expect("owned"),
            services: &mut game,
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        drop(entry);
        drop(frame);
        assert_eq!(game.entity(&id).expect("entity").movement_flags & 512, 512);
    }

    #[test]
    fn q2_push_team_runs_and_posts() {
        let _owner = owner();
        let (mut game, _handles) = test_q2_game();
        let master_owned = admit_q2(&mut game);
        let master = master_owned.id().clone();
        let slave_owned = admit_q2(&mut game);
        let slave = slave_owned.id().clone();
        game.entity_mut(&master).expect("master").motion = Q2MotionKind::Push;
        game.entity_mut(&master).expect("master").team_chain = Some(slave.clone());
        game.entity_mut(&slave).expect("slave").team_master = Some(master.clone());
        let actors = FakeActors::new(&[master.clone(), slave.clone()]);
        let mut bodies = FakeBodies::new();
        let mut physics = FakePhysics::new();
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q2 {
            actor: master_owned,
            game: &mut game,
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        assert!(frame.visited.contains(&master));
        assert!(frame.visited.contains(&slave));
        drop(entry);
        drop(frame);
        assert_eq!(physics.pushes.len(), 1);
        assert_eq!(physics.pushes[0], vec![master.clone(), slave.clone()]);
        assert_eq!(scheduler.runs.len(), 2);

        // A slave entry defers to its master without touching physics.
        let mut bodies = FakeBodies::new();
        let mut physics = FakePhysics::new();
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q2 {
            actor: slave_owned,
            game: &mut game,
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        assert!(frame.visited.is_empty());
        drop(entry);
        drop(frame);
        assert!(physics.pushes.is_empty());
    }

    #[test]
    fn q2_blocked_team_defers_thinks() {
        let owner = owner();
        let (mut game, _handles) = test_q2_game();
        let provider = ProviderId::new("q2", "game");
        let master_owned = admit_q2(&mut game);
        let master = master_owned.id().clone();
        game.entity_mut(&master).expect("master").motion = Q2MotionKind::Push;
        let blocker = owner.actor(8, 0);
        let actors = FakeActors::new(std::slice::from_ref(&master));
        let mut bodies = FakeBodies::new();
        let mut physics = FakePhysics::new();
        physics.push_result = Some(blocker);
        let mut scheduler = FakeScheduler::new();
        scheduler.pending.insert(
            master.clone(),
            ScheduledThink {
                actor: master.clone(),
                callback: "think".to_string(),
                timing: think_timing(&master, &provider, SourceTime::Seconds(1.0)),
            },
        );
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q2 {
            actor: master_owned,
            game: &mut game,
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        drop(entry);
        drop(frame);
        assert!(scheduler.runs.is_empty());
        assert_eq!(scheduler.scheduled.len(), 1);
        let due = &scheduler.scheduled[0].2.due;
        assert!((due_seconds(due) - 1.1).abs() < 0.001);
        let entity = game.entity(&master).expect("entity");
        assert!(entity.next_think.is_some_and(|think| (think - 1.1).abs() < 0.001));
    }

    #[test]
    fn q2_step_runs_think_after_physics() {
        let _owner = owner();
        let (mut game, _handles) = test_q2_game();
        let owned = admit_q2(&mut game);
        let actor = owned.id().clone();
        game.entity_mut(&actor).expect("entity").motion = Q2MotionKind::Step;
        let actors = FakeActors::new(std::slice::from_ref(&actor));
        let mut bodies = FakeBodies::new();
        let mut physics = FakePhysics::new();
        physics.step_result = Some(NewTossOutcome::Moved);
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q2 {
            actor: owned,
            game: &mut game,
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        drop(entry);
        drop(frame);
        assert_eq!(physics.steps, vec![actor.clone()]);
        assert_eq!(scheduler.runs.len(), 1);
        assert!(matches!(scheduler.runs[0].1, FramePhase::EntityThink));
    }

    #[test]
    fn q2_toss_drags_team_chain() {
        let _owner = owner();
        let (mut game, _handles) = test_q2_game();
        let leader_owned = admit_q2(&mut game);
        let leader = leader_owned.id().clone();
        let follower_owned = admit_q2(&mut game);
        let follower = follower_owned.id().clone();
        game.entity_mut(&leader).expect("leader").motion = Q2MotionKind::Toss;
        game.entity_mut(&leader).expect("leader").team_chain = Some(follower.clone());
        let actors = FakeActors::new(&[leader.clone(), follower.clone()]);
        let mut bodies = FakeBodies::new();
        bodies.bodies.insert(leader.clone(), body_state());
        let mut follower_body = body_state();
        follower_body.origin = vec3(0.0, 0.0, 0.0);
        bodies.bodies.insert(follower.clone(), follower_body);
        let mut physics = FakePhysics::new();
        let mut scheduler = FakeScheduler::new();
        let mut frame = ActorExecutionFrame {
            actors: &actors,
            bodies: &mut bodies,
            physics: &mut physics,
            scheduler: &mut scheduler,
            frame: frame_context(),
            time_seconds: 1.0,
            elapsed: 0.1,
            visited: HashSet::new(),
        };
        let mut entry = ActorExecution::Q2 {
            actor: leader_owned,
            game: &mut game,
            content: content(),
        };
        execute_actor(&mut entry, &mut frame).expect("execute");
        drop(entry);
        drop(frame);
        let dragged = bodies.bodies.get(&follower).expect("follower");
        assert_eq!(dragged.origin, vec3(10.0, 20.0, 30.0));
        assert!(bodies.linked.contains(&follower));
    }
}
