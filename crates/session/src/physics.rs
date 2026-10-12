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
