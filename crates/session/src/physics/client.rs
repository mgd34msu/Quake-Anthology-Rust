//! Native client phases over the same scoped body, trace and contact services.
use super::{MotionFrame, MotionHost, MotionTrace, MotionWrite, PhysicsError};
use qa_core::{
    math,
    primitives::{Body, EntityId, Motion, UserCmd, Vec3},
};
use qa_gameplay::rules::motion::ClientMotionRules;
use qa_movement::ControlTuning;
use qa_world::collision::{QuakeTraceKind, Trace};

mod control;
pub use control::client_control;

/// Extra client fields are read only when their native phase needs them.
/// Every operation ends native/table borrows before returning to the caller.
pub trait ClientHost: MotionHost {
    fn command(&mut self, entity: EntityId) -> Result<UserCmd, Self::Error>;
    fn view_angles(&mut self, entity: EntityId) -> Result<Vec3, Self::Error>;
    fn view_height(&mut self, entity: EntityId) -> Result<f32, Self::Error>;
    fn water_level(&mut self, entity: EntityId) -> Result<f32, Self::Error>;
    fn write_water_level(&mut self, entity: EntityId, value: f32) -> Result<(), Self::Error>;
    fn write_water_type(&mut self, entity: EntityId, value: f32) -> Result<(), Self::Error>;
    fn old_origin(&mut self, entity: EntityId, axis: usize) -> Result<f32, Self::Error>;
    fn write_old_origin(
        &mut self,
        entity: EntityId,
        axis: usize,
        value: f32,
    ) -> Result<(), Self::Error>;
    fn teleport_time(&mut self, entity: EntityId) -> Result<f32, Self::Error>;
    fn write_teleport_time(&mut self, entity: EntityId, value: f32) -> Result<(), Self::Error>;
    fn move_direction(&mut self, entity: EntityId, axis: usize) -> Result<f32, Self::Error>;
    fn point_contents(&mut self, entity: EntityId, point: Vec3) -> Result<i32, Self::Error>;
    fn trace_point(
        &mut self,
        entity: EntityId,
        start: Vec3,
        end: Vec3,
    ) -> Result<Trace, Self::Error>;
    fn tuning(&mut self, entity: EntityId) -> Result<ControlTuning, Self::Error>;
    fn diagnostic(&mut self, text: &'static [u8]) -> Result<(), Self::Error>;
    /// Resolve the same client authorization after any callback boundary.
    fn current(&mut self) -> Result<Option<EntityId>, Self::Error>;
}

/// The app has completed this captured client phase's one native think.
/// Final client relink and PostThink remain with that caller.
pub fn client_move<H: ClientHost>(
    host: &mut H,
    entity: EntityId,
    phase: Motion,
    frame: MotionFrame,
    rules: &ClientMotionRules,
) -> Result<(), H::Error> {
    match phase {
        Motion::None => Ok(()),
        Motion::Noclip => {
            host.prepare_motion(entity)?;
            math::vector_ma(
                host,
                frame.elapsed as f32,
                |host, axis| {
                    let body = body(host, entity)?;
                    Ok::<_, H::Error>((body.position.0[axis], body.velocity.0[axis]))
                },
                |host, axis, value| {
                    let mut end = body(host, entity)?.position;
                    end.0[axis] = value;
                    let end = super::positional_end(host.entities(), entity, end)?;
                    host.write_motion(entity, MotionWrite::Origin(axis), end.0[axis])
                },
            )
        }
        Motion::Walk => {
            host.prepare_motion(entity)?;
            if !check_water(host, entity)? && flags(host, entity)? & rules.water_jump_flag == 0 {
                super::add_gravity(host, entity, frame, &rules.toss.gravity)?;
            }
            if let Some(entity) = check_stuck(host, entity)? {
                walk(host, entity, frame, rules)?;
            }
            Ok(())
        }
        Motion::Fly => {
            host.prepare_motion(entity)?;
            super::sweep_motion(
                host,
                entity,
                frame.elapsed as f32,
                &rules.sweep,
                rules.step.ground_flag,
            )?;
            let _ = host.current()?;
            Ok(())
        }
        Motion::Toss | Motion::Bounce => {
            super::toss(host, entity, frame, &rules.toss)?;
            let _ = host.current()?;
            Ok(())
        }
        Motion::FlyMissile if rules.allow_missile => {
            super::toss(host, entity, frame, &rules.toss)?;
            let _ = host.current()?;
            Ok(())
        }
        Motion::Step if rules.allow_step => {
            let current = super::step_fall(host, entity, frame, &rules.step, &rules.sweep)?;
            if let Some(entity) = after_callback(host, current)? {
                host.water_transition(entity)?;
            }
            Ok(())
        }
        _ => Err(PhysicsError::MissingField.into()),
    }
}

fn body<H: MotionHost>(host: &mut H, entity: EntityId) -> Result<Body, H::Error> {
    let entities = host.entities();
    let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
    Ok(entities.columns.body(slot))
}

fn flags<H: MotionHost>(host: &mut H, entity: EntityId) -> Result<u32, H::Error> {
    host.state(entity)?
        .flags
        .ok_or(PhysicsError::MissingField.into())
}

fn store_vector<H: MotionHost>(
    host: &mut H,
    entity: EntityId,
    field: impl Fn(usize) -> MotionWrite,
    vector: Vec3,
) -> Result<(), H::Error> {
    for axis in 0..3 {
        host.write_motion(entity, field(axis), vector.0[axis])?;
    }
    Ok(())
}

fn after_callback<H: ClientHost>(
    host: &mut H,
    reported: Option<EntityId>,
) -> Result<Option<EntityId>, H::Error> {
    if reported.is_some() {
        host.current()
    } else {
        Ok(None)
    }
}

fn check_water<H: ClientHost>(host: &mut H, entity: EntityId) -> Result<bool, H::Error> {
    let initial = body(host, entity)?;
    let mut point = initial.position;
    point.0[2] = initial.position.0[2] + initial.mins.0[2] + 1.0;
    host.write_water_level(entity, 0.0)?;
    host.write_water_type(entity, -1.0)?;
    let contents = host.point_contents(entity, point)?;
    if contents <= -3 {
        host.write_water_type(entity, contents as f32)?;
        host.write_water_level(entity, 1.0)?;
        let current = body(host, entity)?;
        point.0[2] = current.position.0[2] + (current.mins.0[2] + current.maxs.0[2]) * 0.5;
        if host.point_contents(entity, point)? <= -3 {
            host.write_water_level(entity, 2.0)?;
            point.0[2] = body(host, entity)?.position.0[2] + host.view_height(entity)?;
            if host.point_contents(entity, point)? <= -3 {
                host.write_water_level(entity, 3.0)?;
            }
        }
    }
    Ok(host.water_level(entity)? > 1.0)
}

fn free_position<H: MotionHost>(host: &mut H, entity: EntityId) -> Result<bool, H::Error> {
    let position = body(host, entity)?.position;
    Ok(!host
        .trace(entity, position, QuakeTraceKind::Normal)?
        .trace
        .start_solid)
}

fn check_stuck<H: ClientHost>(
    host: &mut H,
    entity: EntityId,
) -> Result<Option<EntityId>, H::Error> {
    if free_position(host, entity)? {
        for axis in 0..3 {
            let value = body(host, entity)?.position.0[axis];
            host.write_old_origin(entity, axis, value)?;
        }
        return Ok(Some(entity));
    }
    let original = body(host, entity)?.position;
    for axis in 0..3 {
        let value = host.old_origin(entity, axis)?;
        host.write_motion(entity, MotionWrite::Origin(axis), value)?;
    }
    if free_position(host, entity)? {
        host.diagnostic(b"Unstuck.\n")?;
        let current = host.relink(entity)?;
        return after_callback(host, current);
    }
    for z in 0..18 {
        for x in -1..=1 {
            for y in -1..=1 {
                store_vector(
                    host,
                    entity,
                    MotionWrite::Origin,
                    original + Vec3([x as f32, y as f32, z as f32]),
                )?;
                if free_position(host, entity)? {
                    host.diagnostic(b"Unstuck.\n")?;
                    let current = host.relink(entity)?;
                    return after_callback(host, current);
                }
            }
        }
    }
    store_vector(host, entity, MotionWrite::Origin, original)?;
    host.diagnostic(b"player is stuck.\n")?;
    Ok(Some(entity))
}

fn walk<H: ClientHost>(
    host: &mut H,
    mut entity: EntityId,
    frame: MotionFrame,
    rules: &ClientMotionRules,
) -> Result<(), H::Error> {
    let old_flags = flags(host, entity)?;
    let old_ground = old_flags & rules.step.ground_flag != 0;
    host.write_flags(entity, old_flags & !rules.step.ground_flag)?;
    let old = body(host, entity)?;
    let first = super::sweep_motion(
        host,
        entity,
        frame.elapsed as f32,
        &rules.sweep,
        rules.step.ground_flag,
    )?;
    let Some(current) = after_callback(host, first.current)? else {
        return Ok(());
    };
    entity = current;
    if first.result.blocked & 2 == 0
        || !old_ground && host.water_level(entity)? == 0.0
        || host.state(entity)?.move_type != rules.walk_move_type
        || host.tuning(entity)?.no_step
        || flags(host, entity)? & rules.water_jump_flag != 0
    {
        return Ok(());
    }
    let no_step = body(host, entity)?;
    store_vector(host, entity, MotionWrite::Origin, old.position)?;
    let mut down = Vec3::default();
    down.0[2] =
        (-f64::from(rules.step.height) + f64::from(old.velocity.0[2]) * frame.elapsed) as f32;
    let (_, current) = super::push_entity(host, entity, Vec3([0.0, 0.0, rules.step.height]))?;
    let Some(current) = after_callback(host, current)? else {
        return Ok(());
    };
    entity = current;
    store_vector(
        host,
        entity,
        MotionWrite::Velocity,
        Vec3([old.velocity.0[0], old.velocity.0[1], 0.0]),
    )?;
    let second = super::sweep_motion(
        host,
        entity,
        frame.elapsed as f32,
        &rules.sweep,
        rules.step.ground_flag,
    )?;
    let Some(current) = after_callback(host, second.current)? else {
        return Ok(());
    };
    entity = current;
    let mut blocked = second.result.blocked;
    let position = body(host, entity)?.position;
    if blocked != 0
        && (old.position.0[1] - position.0[1]).abs() < 0.03125
        && (old.position.0[0] - position.0[0]).abs() < 0.03125
    {
        let Some((current, clip)) = try_unstick(host, entity, old.velocity, rules)? else {
            return Ok(());
        };
        entity = current;
        blocked = clip;
    }
    if blocked & 2 != 0
        && let Some(wall) = second.wall.or(first.wall)
    {
        wall_friction(host, entity, &wall)?;
    }
    let (trace, current) = super::push_entity(host, entity, down)?;
    let Some(current) = after_callback(host, current)? else {
        return Ok(());
    };
    entity = current;
    if trace.trace.plane.normal.0[2] > 0.7 {
        if host
            .state(entity)?
            .solid
            .ok_or(PhysicsError::MissingField)?
            == rules.bsp_solid
        {
            let flags = flags(host, entity)?;
            host.write_flags(entity, flags | rules.step.ground_flag)?;
            host.ground(entity, trace.ground)?;
        }
    } else {
        store_vector(host, entity, MotionWrite::Origin, no_step.position)?;
        store_vector(host, entity, MotionWrite::Velocity, no_step.velocity)?;
    }
    Ok(())
}

fn try_unstick<H: ClientHost>(
    host: &mut H,
    mut entity: EntityId,
    old_velocity: Vec3,
    rules: &ClientMotionRules,
) -> Result<Option<(EntityId, u8)>, H::Error> {
    let original = body(host, entity)?.position;
    for [x, y] in [
        [2.0, 0.0],
        [0.0, 2.0],
        [-2.0, 0.0],
        [0.0, -2.0],
        [2.0, 2.0],
        [-2.0, 2.0],
        [2.0, -2.0],
        [-2.0, -2.0],
    ] {
        let (_, current) = super::push_entity(host, entity, Vec3([x, y, 0.0]))?;
        let Some(current) = after_callback(host, current)? else {
            return Ok(None);
        };
        entity = current;
        store_vector(
            host,
            entity,
            MotionWrite::Velocity,
            Vec3([old_velocity.0[0], old_velocity.0[1], 0.0]),
        )?;
        let swept = super::sweep_motion(host, entity, 0.1, &rules.sweep, rules.step.ground_flag)?;
        let Some(current) = after_callback(host, swept.current)? else {
            return Ok(None);
        };
        entity = current;
        let position = body(host, entity)?.position;
        if (original.0[1] - position.0[1]).abs() > 4.0
            || (original.0[0] - position.0[0]).abs() > 4.0
        {
            return Ok(Some((entity, swept.result.blocked)));
        }
        store_vector(host, entity, MotionWrite::Origin, original)?;
    }
    store_vector(host, entity, MotionWrite::Velocity, Vec3::default())?;
    Ok(Some((entity, 7)))
}

fn wall_friction<H: ClientHost>(
    host: &mut H,
    entity: EntityId,
    trace: &MotionTrace,
) -> Result<(), H::Error> {
    let forward = math::angle_vectors(host.view_angles(entity)?).forward;
    let normal = trace.trace.plane.normal;
    let d = normal.dot(forward) + 0.5;
    if d >= 0.0 {
        return Ok(());
    }
    let velocity = body(host, entity)?.velocity;
    let into = normal * normal.dot(velocity);
    let side = velocity - into;
    host.write_motion(entity, MotionWrite::Velocity(0), side.0[0] * (1.0 + d))?;
    host.write_motion(entity, MotionWrite::Velocity(1), side.0[1] * (1.0 + d))
}
