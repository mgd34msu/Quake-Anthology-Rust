//! Engine-owned entity motion over the one live entity table. Providers choose
//! its phase; native adapters only synchronize ABI writes and explicit links.
use qa_core::primitives::EntityId;
use qa_world::entities::EntityTable;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicsError {
    Entity,
    AngularVelocity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionWrite {
    Angles(usize),
    Origin(usize),
    Velocity(usize),
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
    for axis in 0..3 {
        let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        let columns = &mut entities.columns;
        let angular = columns.angular_velocity[slot].ok_or(PhysicsError::AngularVelocity)?;
        let next = columns.angles[slot] + angular * step;
        columns.angles[slot].0[axis] = next.0[axis];
        write(entities, MotionWrite::Angles(axis))?;
    }
    for axis in 0..3 {
        let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        let columns = &mut entities.columns;
        let next = columns.position[slot] + columns.velocity[slot] * step;
        columns.position[slot].0[axis] = next.0[axis];
        write(entities, MotionWrite::Origin(axis))?;
    }
    Ok(())
}
