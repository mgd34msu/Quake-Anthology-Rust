use super::{MotionFrame, MotionHost, MotionTrace, MotionWrite, PhysicsError};
use qa_core::primitives::{Body, EntityId, NativeEntityRef, Vec3};
use qa_gameplay::rules::{motion::StepRules, sweep::LegacySweepRules};
use qa_movement::SweepHost;
use qa_world::collision::{QuakeTraceKind, Trace};

/// STEP falls before its native think phase. The provider runs the existing
/// think/rebind dispatcher afterward, then the surviving row's water phase,
/// including when ONGROUND/FLY/SWIM suppress this falling branch.
pub fn step_fall<H: MotionHost>(
    host: &mut H,
    entity: EntityId,
    frame: MotionFrame,
    rules: &StepRules,
    sweep_rules: &LegacySweepRules,
) -> Result<Option<EntityId>, H::Error> {
    let flags = host
        .state(entity)?
        .flags
        .ok_or(PhysicsError::MissingField)?;
    if !rules.falls(flags) {
        return Ok(Some(entity));
    }
    let entities = host.entities();
    let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
    let hit_sound = rules.landing_sound(entities.columns.velocity[slot].0[2], frame.gravity);
    super::add_gravity(host, entity, frame, &rules.gravity)?;
    host.check_velocity(entity, frame.max_velocity)?;
    let mut falling = Falling {
        host,
        current: Some(entity),
        rules,
    };
    qa_movement::sweep(&mut falling, frame.elapsed as f32, sweep_rules)?;
    let Some(entity) = falling.current else {
        return Ok(None);
    };
    let current = falling.host.relink(entity)?;
    if let Some(entity) = current
        && hit_sound
        && falling
            .host
            .state(entity)?
            .flags
            .ok_or(PhysicsError::MissingField)?
            & rules.ground_flag
            != 0
    {
        falling.host.sound(entity, b"demon/dland2.wav")?;
    }
    Ok(current)
}

struct Falling<'a, H> {
    host: &'a mut H,
    current: Option<EntityId>,
    rules: &'a StepRules,
}

impl<H: MotionHost> SweepHost for Falling<'_, H> {
    type Error = H::Error;
    type Hit = Option<NativeEntityRef>;

    fn read(&mut self) -> Result<Option<Body>, Self::Error> {
        let Some(entity) = self.current else {
            return Ok(None);
        };
        let entities = self.host.entities();
        let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        Ok(Some(entities.columns.body(slot)))
    }

    fn write(&mut self, field: MotionWrite, value: f32) -> Result<(), Self::Error> {
        let entity = self.current.ok_or(PhysicsError::Entity)?;
        self.host.write_motion(entity, field, value)
    }

    fn trace(&mut self, _: Vec3, end: Vec3) -> Result<(Trace, Self::Hit), Self::Error> {
        let entity = self.current.ok_or(PhysicsError::Entity)?;
        let end = super::positional_end(self.host.entities(), entity, end)?;
        let trace = self.host.trace(entity, end, QuakeTraceKind::Normal)?;
        Ok((trace.trace, trace.ground))
    }

    fn contact(
        &mut self,
        trace: &Trace,
        ground: Self::Hit,
        hit_bits: u8,
    ) -> Result<(), Self::Error> {
        let entity = self.current.ok_or(PhysicsError::Entity)?;
        if trace.entity.is_none() || ground.is_none() {
            return Err(PhysicsError::Entity.into());
        }
        if hit_bits & 1 != 0 && trace.brush_solid {
            let flags = self
                .host
                .state(entity)?
                .flags
                .ok_or(PhysicsError::MissingField)?;
            self.host
                .write_flags(entity, flags | self.rules.ground_flag)?;
            self.host.ground(entity, ground)?;
        }
        let trace = MotionTrace {
            trace: *trace,
            ground,
        };
        self.current = self.host.impact(Some(entity), &trace)?;
        Ok(())
    }

    fn recover_position(&mut self, _: Vec3) -> Result<Option<Vec3>, Self::Error> {
        Ok(None)
    }
}
