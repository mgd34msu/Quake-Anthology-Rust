//! Engine-owned entity motion over the one live entity table. Providers choose
//! its phase; native adapters only synchronize ABI writes and explicit links.
pub use qa_core::primitives::MotionWrite;
use qa_core::{
    math,
    primitives::{EntityId, PlayerState},
};
use qa_world::collision::TraceError;
use qa_world::entities::EntityTable;

mod monster;
mod push;
mod step;
mod support;
mod toss;
pub use monster::{
    GoalCursor, GoalHost, GoalStep, MonsterHost, StepMove, StepPost, finish_step, move_step,
};
pub use push::{PushFrame, PushHost, PushNumber, PushUndo, push_frame};
pub use step::step_fall;
pub use support::{SupportQueries, check_bottom};
pub use toss::{
    MotionFrame, MotionHost, MotionState, MotionTrace, WaterTransition, push_entity, toss,
    water_transition,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicsError {
    Entity,
    AngularVelocity,
    MissingField,
    Capacity,
}

/// Native entity phases use one gravity calculation at their own phase
/// position. Per-entity scale is optional in NQ and never read by QW.
fn add_gravity<H: MotionHost>(
    host: &mut H,
    entity: EntityId,
    frame: MotionFrame,
    rules: &qa_gameplay::rules::motion::GravityRules,
) -> Result<(), H::Error> {
    let scale = if rules.entity_scale {
        host.gravity_scale(entity)?
            .filter(|value| *value != 0.0)
            .unwrap_or(1.0)
    } else {
        1.0
    };
    let entities = host.entities();
    let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
    let velocity = entities.columns.velocity[slot].0[2];
    // The gravity factors multiply in binary32 before double duration promotes
    // the subtraction. The scalar store narrows only once.
    let value = (f64::from(velocity) - f64::from(scale * frame.gravity) * frame.elapsed) as f32;
    host.write_motion(entity, MotionWrite::Velocity(2), value)
}

/// Attachment transport owns position; source motion still owns its angular
/// and contact phases. Returning the stored pose also preserves signed zero.
fn positional_end(
    entities: &EntityTable,
    entity: EntityId,
    end: qa_core::primitives::Vec3,
) -> Result<qa_core::primitives::Vec3, PhysicsError> {
    let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
    Ok(if entities.attachment(entity).is_some() {
        entities.columns.position[slot]
    } else {
        end
    })
}

/// Ordinary authoritative movement has no native scalar synchronization.
pub fn no_body_boundary(
    _: &EntityTable,
    _: &mut PlayerState,
    _: MotionWrite,
) -> Result<(), TraceError> {
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VelocityCheck {
    pub invalid_velocity: u8,
    pub invalid_origin: u8,
}

/// Native velocity checking visits each velocity, origin, and speed limit in
/// that order. A scalar ABI write may change another field through overlap;
/// the next check reads the refreshed columns of the same live entity.
/// The boundary receives `true` for a nonfinite clear so it can publish the
/// native diagnostic before the scalar ABI write changes aliased fields.
pub fn check_velocity<E: From<PhysicsError>>(
    entities: &mut EntityTable,
    entity: EntityId,
    max: f32,
    mut write: impl FnMut(&mut EntityTable, MotionWrite, bool) -> Result<(), E>,
) -> Result<VelocityCheck, E> {
    let mut result = VelocityCheck::default();
    for axis in 0..3 {
        let mut slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        if !entities.columns.velocity[slot].0[axis].is_finite() {
            entities.columns.velocity[slot].0[axis] = 0.0;
            result.invalid_velocity |= 1 << axis;
            write(entities, MotionWrite::Velocity(axis), true)?;
            slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        }
        if !entities.columns.position[slot].0[axis].is_finite() {
            entities.columns.position[slot].0[axis] = 0.0;
            result.invalid_origin |= 1 << axis;
            write(entities, MotionWrite::Origin(axis), true)?;
            slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        }
        let velocity = entities.columns.velocity[slot].0[axis];
        let limited = if velocity > max {
            Some(max)
        } else if velocity < -max {
            Some(-max)
        } else {
            None
        };
        if let Some(limited) = limited {
            entities.columns.velocity[slot].0[axis] = limited;
            write(entities, MotionWrite::Velocity(axis), false)?;
            entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        }
    }
    Ok(result)
}

/// The caller captures NOCLIP before think and resolves the surviving lifetime
/// afterward. A boundary may refresh fields affected by a native scalar write:
/// original VectorMA permits overlapping fields and commits in axis order.
pub fn noclip<E: From<PhysicsError>>(
    entities: &mut EntityTable,
    entity: EntityId,
    step: f32,
    mut write: impl FnMut(&mut EntityTable, MotionWrite) -> Result<(), E>,
) -> Result<(), E> {
    for angles in [true, false] {
        let mut state = (&mut *entities, &mut write);
        math::vector_ma(
            &mut state,
            step,
            |(entities, _), axis| {
                let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
                let columns = &entities.columns;
                if angles {
                    let rate =
                        columns.angular_velocity[slot].ok_or(PhysicsError::AngularVelocity)?;
                    Ok::<_, E>((columns.angles[slot].0[axis], rate.0[axis]))
                } else {
                    Ok((
                        columns.position[slot].0[axis],
                        columns.velocity[slot].0[axis],
                    ))
                }
            },
            |(entities, write), axis, value| {
                let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
                let field = if angles {
                    entities.columns.angles[slot].0[axis] = value;
                    MotionWrite::Angles(axis)
                } else {
                    entities.columns.position[slot].0[axis] = value;
                    MotionWrite::Origin(axis)
                };
                write(entities, field)?;
                entities.resolve(entity).ok_or(PhysicsError::Entity)?;
                Ok(())
            },
        )?;
    }
    Ok(())
}
