use super::{MotionWrite, PhysicsError};
use qa_core::{
    math,
    primitives::{EntityId, Vec3},
};
use qa_gameplay::rules::motion::TossRules;
use qa_world::{
    collision::{QuakeTraceKind, Trace},
    entities::{EntityReference, EntityTable},
};

/// Scoped native metadata. Optional fields are required only by executed paths.
#[derive(Clone, Copy, Debug)]
pub struct MotionState {
    pub move_type: f32,
    pub solid: Option<f32>,
    pub flags: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct MotionFrame {
    pub elapsed: f64,
    pub gravity: f32,
    pub max_velocity: f32,
}

/// Capture the hit lifetime before callbacks. Native rows may rebind; anonymous
/// bodies retain their exact generation without acquiring a made-up namespace.
#[derive(Clone, Copy, Debug)]
pub struct MotionTrace {
    pub trace: Trace,
    pub ground: Option<EntityReference>,
}

/// Each method ends its entity/VM borrows before returning. Relink performs
/// trigger callbacks; impact then performs both current solid/touch checks,
/// including the target's touch when a trigger has removed the moving entity.
pub trait MotionHost {
    type Error: From<PhysicsError>;
    fn entities(&mut self) -> &mut EntityTable;
    fn state(&mut self, entity: EntityId) -> Result<MotionState, Self::Error>;
    fn prepare_motion(&mut self, entity: EntityId) -> Result<(), Self::Error>;
    fn check_velocity(&mut self, entity: EntityId, max: f32) -> Result<(), Self::Error>;
    fn write_motion(
        &mut self,
        entity: EntityId,
        field: MotionWrite,
        value: f32,
    ) -> Result<(), Self::Error>;
    fn write_flags(&mut self, entity: EntityId, flags: u32) -> Result<(), Self::Error>;
    fn gravity_scale(&mut self, entity: EntityId) -> Result<Option<f32>, Self::Error>;
    fn trace(
        &mut self,
        entity: EntityId,
        end: Vec3,
        kind: QuakeTraceKind,
    ) -> Result<MotionTrace, Self::Error>;
    fn relink(&mut self, entity: EntityId) -> Result<Option<EntityId>, Self::Error>;
    fn impact(
        &mut self,
        entity: Option<EntityId>,
        trace: &MotionTrace,
    ) -> Result<Option<EntityId>, Self::Error>;
    fn ground(
        &mut self,
        entity: EntityId,
        ground: Option<EntityReference>,
    ) -> Result<(), Self::Error>;
    fn sound(&mut self, entity: EntityId, name: &[u8]) -> Result<(), Self::Error>;
    /// Read contents at the surviving entity's current origin and apply the
    /// shared water_transition decision: sound, type store, then level store.
    fn water_transition(&mut self, entity: EntityId) -> Result<(), Self::Error>;
}

/// The provider has already run this entity's think at its native phase and
/// rebound its current lifetime. Motion classification remains that captured
/// phase; raw movetype is reread for gravity, trace filtering and bounce.
pub fn toss<H: MotionHost>(
    host: &mut H,
    mut entity: EntityId,
    frame: MotionFrame,
    rules: &TossRules,
) -> Result<(), H::Error> {
    if rules.clear_ground_on_rise {
        let entities = host.entities();
        let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        if entities.columns.velocity[slot].0[2] > 0.0 {
            let flags = host
                .state(entity)?
                .flags
                .ok_or(PhysicsError::MissingField)?;
            host.write_flags(entity, flags & !rules.ground_flag)?;
        }
    }
    let flags = host
        .state(entity)?
        .flags
        .ok_or(PhysicsError::MissingField)?;
    if flags & rules.ground_flag != 0 {
        return Ok(());
    }
    host.check_velocity(entity, frame.max_velocity)?;
    let mode = host.state(entity)?.move_type;
    if rules.gravity_applies(mode) {
        super::add_gravity(host, entity, frame, &rules.gravity)?;
    }
    host.prepare_motion(entity)?;
    math::vector_ma(
        host,
        frame.elapsed as f32,
        |host, axis| {
            let entities = host.entities();
            let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
            let rate =
                entities.columns.angular_velocity[slot].ok_or(PhysicsError::AngularVelocity)?;
            Ok::<_, H::Error>((entities.columns.angles[slot].0[axis], rate.0[axis]))
        },
        |host, axis, value| host.write_motion(entity, MotionWrite::Angles(axis), value),
    )?;
    let entities = host.entities();
    let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
    let movement = entities.columns.velocity[slot] * frame.elapsed as f32;
    let (trace, current) = push_entity(host, entity, movement)?;
    if trace.trace.fraction == 1.0 {
        return Ok(());
    }
    let Some(surviving) = current else {
        return Ok(());
    };
    entity = surviving;
    let overbounce = if rules.is_bounce(host.state(entity)?.move_type) {
        1.5
    } else {
        1.0
    };
    let entities = host.entities();
    let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
    let backoff = qa_movement::clip_backoff(
        entities.columns.velocity[slot],
        trace.trace.plane.normal,
        overbounce,
        false,
    );
    for axis in 0..3 {
        let entities = host.entities();
        let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        let value = math::clip_component(
            entities.columns.velocity[slot].0[axis],
            trace.trace.plane.normal.0[axis],
            backoff,
            Some(0.1),
        );
        host.write_motion(entity, MotionWrite::Velocity(axis), value)?;
    }
    if trace.trace.plane.normal.0[2] > 0.7 {
        let entities = host.entities();
        let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        if entities.columns.velocity[slot].0[2] < 60.0
            || !rules.is_bounce(host.state(entity)?.move_type)
        {
            let flags = host
                .state(entity)?
                .flags
                .ok_or(PhysicsError::MissingField)?;
            host.write_flags(entity, flags | rules.ground_flag)?;
            host.ground(entity, trace.ground)?;
            for angular in [false, true] {
                for axis in 0..3 {
                    let field = if angular {
                        MotionWrite::AngularVelocity(axis)
                    } else {
                        MotionWrite::Velocity(axis)
                    };
                    host.write_motion(entity, field, 0.0)?;
                }
            }
        }
    }
    host.water_transition(entity)
}

/// Shared native displacement: trace, scalar pose stores, trigger relink, then
/// impact. The returned lifetime may differ after either callback boundary.
pub fn push_entity<H: MotionHost>(
    host: &mut H,
    entity: EntityId,
    displacement: Vec3,
) -> Result<(MotionTrace, Option<EntityId>), H::Error> {
    let entities = host.entities();
    let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
    let end = super::positional_end(
        entities,
        entity,
        entities.columns.position[slot] + displacement,
    )?;
    let state = host.state(entity)?;
    // These native trace categories are shared by the existing Toss rules.
    let kind = if TossRules::is_missile(state.move_type) {
        QuakeTraceKind::Missile
    } else if TossRules::ignores_boxes(state.solid.ok_or(PhysicsError::MissingField)?) {
        QuakeTraceKind::IgnoreBoxes
    } else {
        QuakeTraceKind::Normal
    };
    let trace = host.trace(entity, end, kind)?;
    for axis in 0..3 {
        host.write_motion(entity, MotionWrite::Origin(axis), trace.trace.end.0[axis])?;
    }
    let mut current = host.relink(entity)?;
    if trace.trace.entity.is_some() {
        current = host.impact(current, &trace)?;
    }
    Ok((trace, current))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterTransition {
    pub splash: bool,
    pub water_type: f32,
    pub water_level: f32,
}

/// Native SV_CheckWaterTransition keeps dry waterlevel equal to the contents
/// value. It is neither a boolean nor the player movement water-depth value.
pub fn water_transition(previous: f32, contents: i32) -> WaterTransition {
    if previous == 0.0 {
        return WaterTransition {
            splash: false,
            water_type: contents as f32,
            water_level: 1.0,
        };
    }
    if contents <= -3 {
        WaterTransition {
            splash: previous == -1.0,
            water_type: contents as f32,
            water_level: 1.0,
        }
    } else {
        WaterTransition {
            splash: previous != -1.0,
            water_type: -1.0,
            water_level: contents as f32,
        }
    }
}
