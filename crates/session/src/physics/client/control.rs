use super::ClientHost;
use crate::physics::{MotionFrame, MotionWrite, PhysicsError};
use qa_core::{
    math,
    primitives::{EntityId, RuleSetId, ThinkTime, Vec3},
};
use qa_gameplay::rules::motion::ClientMotionRules;
use qa_movement::{
    ControlStart,
    control::{self, ControlWish},
};

fn velocity<H: ClientHost>(host: &mut H, entity: EntityId) -> Result<Vec3, H::Error> {
    let entities = host.entities();
    let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
    Ok(entities.columns.velocity[slot])
}

fn scale_velocity<H: ClientHost>(
    host: &mut H,
    entity: EntityId,
    scale: f32,
) -> Result<(), H::Error> {
    for axis in 0..3 {
        let value = velocity(host, entity)?.0[axis] * scale;
        host.write_motion(entity, MotionWrite::Velocity(axis), value)?;
    }
    Ok(())
}

fn accelerate<H: ClientHost>(
    host: &mut H,
    entity: EntityId,
    direction: Vec3,
    amount: f32,
) -> Result<(), H::Error> {
    for axis in 0..3 {
        let value = velocity(host, entity)?.0[axis] + amount * direction.0[axis];
        host.write_motion(entity, MotionWrite::Velocity(axis), value)?;
    }
    Ok(())
}

/// Native SV_ClientThink physical control. Presentation has already captured
/// onground and completed its tracked punch/angle stores.
pub fn client_control<H: ClientHost>(
    host: &mut H,
    entity: EntityId,
    start: ControlStart,
    frame: MotionFrame,
    clock: ThinkTime,
    rules: &ClientMotionRules,
) -> Result<(), H::Error> {
    host.prepare_motion(entity)?;
    let flags = host
        .state(entity)?
        .flags
        .ok_or(PhysicsError::MissingField)?;
    if flags & rules.water_jump_flag != 0 {
        if clock.seconds() > f64::from(host.teleport_time(entity)?)
            || host.water_level(entity)? == 0.0
        {
            let flags = host
                .state(entity)?
                .flags
                .ok_or(PhysicsError::MissingField)?;
            host.write_flags(entity, flags & !rules.water_jump_flag)?;
            host.write_teleport_time(entity, 0.0)?;
        }
        for axis in 0..2 {
            let value = host.move_direction(entity, axis)?;
            host.write_motion(entity, MotionWrite::Velocity(axis), value)?;
        }
        return Ok(());
    }
    let command = host
        .command(entity)?
        .in_units(qa_gameplay::rules::command_units(RuleSetId::Quake));
    let tuning = host.tuning(entity)?;
    if host.water_level(entity)? >= 2.0 && host.state(entity)?.move_type != rules.noclip_move_type {
        let wish = control::water_wish(host.view_angles(entity)?, command, tuning.max_speed);
        let speed = math::length(velocity(host, entity)?);
        let next = if speed != 0.0 {
            let next = control::friction_speed(speed, speed, tuning.friction, frame.elapsed);
            scale_velocity(host, entity, next / speed)?;
            next
        } else {
            0.0
        };
        if let Some(amount) =
            control::water_acceleration(wish.speed, next, tuning.accelerate, frame.elapsed)
        {
            accelerate(host, entity, wish.direction, amount)?;
        }
        return Ok(());
    }
    let reverse = clock.seconds() < f64::from(host.teleport_time(entity)?);
    let walking = math::native_integer(host.state(entity)?.move_type)
        == math::native_integer(rules.walk_move_type);
    let wish = control::air_wish(
        start.body_angles,
        command,
        walking,
        reverse,
        tuning.max_speed,
    );
    if host.state(entity)?.move_type == rules.noclip_move_type {
        for axis in 0..3 {
            host.write_motion(entity, MotionWrite::Velocity(axis), wish.velocity.0[axis])?;
        }
        return Ok(());
    }
    if start.on_ground {
        let initial_velocity = velocity(host, entity)?;
        let speed = math::length(Vec3([initial_velocity.0[0], initial_velocity.0[1], 0.0]));
        if speed != 0.0 {
            let entities = host.entities();
            let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
            let mut probe = entities.columns.position[slot];
            probe.0[0] += initial_velocity.0[0] / speed * 16.0;
            probe.0[1] += initial_velocity.0[1] / speed * 16.0;
            probe.0[2] += entities.columns.mins[slot].0[2];
            let mut end = probe;
            end.0[2] -= 34.0;
            let trace = host.trace_point(entity, probe, end)?;
            let friction = if trace.fraction == 1.0 {
                tuning.friction * tuning.edge_friction
            } else {
                tuning.friction
            };
            scale_velocity(
                host,
                entity,
                control::friction_scale(speed, tuning.stop_speed, friction, frame.elapsed),
            )?;
        }
        if let Some(amount) = control::acceleration(
            velocity(host, entity)?.dot(wish.direction),
            wish.speed,
            tuning.accelerate,
            frame.elapsed,
            false,
        ) {
            accelerate(host, entity, wish.direction, amount)?;
        }
    } else {
        let ControlWish {
            velocity: mut direction,
            speed,
            ..
        } = wish;
        let air_speed = math::normalize(&mut direction);
        // The air target comes from the second normalization, while its
        // acceleration product retains the separately capped global wishspeed.
        if let Some(amount) = control::air_acceleration(
            velocity(host, entity)?.dot(direction),
            air_speed,
            speed,
            tuning.accelerate,
            frame.elapsed,
        ) {
            accelerate(host, entity, direction, amount)?;
        }
    }
    Ok(())
}
