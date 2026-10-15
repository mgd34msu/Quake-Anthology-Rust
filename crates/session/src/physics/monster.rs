use super::{MotionTrace, PhysicsError, SupportQueries, check_bottom};
use qa_core::primitives::{Bounds, NativeEntityRef, Vec3};
use qa_gameplay::rules::motion::StepRules;
use qa_world::collision::{QuakeTraceKind, Trace};

/// A scoped view of one moving entity. Reads remain live across scalar stores;
/// the host owns its weak identity and the caller's collision policy.
pub trait MonsterHost {
    type Error: From<PhysicsError>;
    fn origin(&mut self) -> Result<Vec3, Self::Error>;
    fn bounds(&mut self) -> Result<Bounds, Self::Error>;
    fn flags(&mut self) -> Result<u32, Self::Error>;
    /// None names the source world's enemy sentinel. Reading a nonworld
    /// reference does not itself require that target's origin or common life.
    fn enemy(&mut self) -> Result<Option<NativeEntityRef>, Self::Error>;
    fn target_origin(&mut self, target: NativeEntityRef) -> Result<Vec3, Self::Error>;
    fn contents(&mut self, point: Vec3) -> Result<i32, Self::Error>;
    fn trace(
        &mut self,
        start: Vec3,
        end: Vec3,
        bounds: Bounds,
        kind: QuakeTraceKind,
    ) -> Result<MotionTrace, Self::Error>;
    fn write_origin(&mut self, axis: usize, value: f32) -> Result<(), Self::Error>;
    fn write_flags(&mut self, flags: u32) -> Result<(), Self::Error>;
    fn write_ground(&mut self, ground: NativeEntityRef) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepPost {
    None,
    ClearGround,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepMove {
    Blocked,
    Moved(StepPost),
}
impl StepMove {
    pub const fn moved(self) -> bool {
        matches!(self, Self::Moved(_))
    }
}

/// Complete the partial-ground/no-floor path after its caller's chosen link
/// phase. WalkMove touches first; StepDirection completes before its own link.
pub fn finish_step<H: MonsterHost>(
    host: &mut H,
    post: StepPost,
    rules: &StepRules,
) -> Result<(), H::Error> {
    if post == StepPost::ClearGround {
        let flags = host.flags()?;
        host.write_flags(flags & !rules.ground_flag)?;
    }
    Ok(())
}

fn write_origin<H: MonsterHost>(host: &mut H, value: Vec3) -> Result<(), H::Error> {
    for axis in 0..3 {
        host.write_origin(axis, value.0[axis])?;
    }
    Ok(())
}

struct Support<'a, H>(&'a mut H);
impl<H: MonsterHost> SupportQueries for Support<'_, H> {
    type Error = H::Error;

    fn solid(&mut self, point: Vec3) -> Result<bool, Self::Error> {
        Ok(self.0.contents(point)? == -2)
    }
    fn trace(&mut self, start: Vec3, end: Vec3) -> Result<Trace, Self::Error> {
        Ok(self
            .0
            .trace(start, end, Bounds::default(), QuakeTraceKind::IgnoreBoxes)?
            .trace)
    }
}

/// Native displacement and support checks, with no linking or callbacks. Its
/// caller executes any returned post effect at the native relink/touch phase.
pub fn move_step<H: MonsterHost>(
    host: &mut H,
    movement: Vec3,
    rules: &StepRules,
) -> Result<StepMove, H::Error> {
    let old_origin = host.origin()?;
    let mut destination = old_origin + movement;
    if host.flags()? & (rules.fly_flag | rules.swim_flag) != 0 {
        for attempt in 0..2 {
            destination = host.origin()? + movement;
            let enemy = host.enemy()?;
            if attempt == 0
                && let Some(enemy) = enemy
            {
                let dz = host.origin()?.0[2] - host.target_origin(enemy)?.0[2];
                if dz > rules.follow_above {
                    destination.0[2] -= rules.follow_adjust;
                }
                if dz < rules.follow_below {
                    destination.0[2] += rules.follow_adjust;
                }
            }
            let start = host.origin()?;
            let bounds = host.bounds()?;
            let hit = host.trace(start, destination, bounds, QuakeTraceKind::Normal)?;
            if hit.trace.fraction == 1.0 {
                if host.flags()? & rules.swim_flag != 0 && host.contents(hit.trace.end)? == -1 {
                    return Ok(StepMove::Blocked);
                }
                write_origin(host, hit.trace.end)?;
                return Ok(StepMove::Moved(StepPost::None));
            }
            if enemy.is_none() {
                break;
            }
        }
        return Ok(StepMove::Blocked);
    }

    destination.0[2] += rules.height;
    let mut end = destination;
    end.0[2] -= rules.height * 2.0;
    let bounds = host.bounds()?;
    let mut hit = host.trace(destination, end, bounds, QuakeTraceKind::Normal)?;
    if hit.trace.all_solid {
        return Ok(StepMove::Blocked);
    }
    if hit.trace.start_solid {
        destination.0[2] -= rules.height;
        let bounds = host.bounds()?;
        hit = host.trace(destination, end, bounds, QuakeTraceKind::Normal)?;
        if hit.trace.all_solid || hit.trace.start_solid {
            return Ok(StepMove::Blocked);
        }
    }
    if hit.trace.fraction == 1.0 {
        if host.flags()? & rules.partial_ground_flag != 0 {
            let destination = host.origin()? + movement;
            write_origin(host, destination)?;
            return Ok(StepMove::Moved(StepPost::ClearGround));
        }
        return Ok(StepMove::Blocked);
    }

    write_origin(host, hit.trace.end)?;
    let origin = host.origin()?;
    let bounds = host.bounds()?;
    if !check_bottom(&mut Support(host), origin, bounds, rules.height)? {
        if host.flags()? & rules.partial_ground_flag != 0 {
            return Ok(StepMove::Moved(StepPost::None));
        }
        write_origin(host, old_origin)?;
        return Ok(StepMove::Blocked);
    }
    let flags = host.flags()?;
    if flags & rules.partial_ground_flag != 0 {
        host.write_flags(flags & !rules.partial_ground_flag)?;
    }
    host.write_ground(hit.ground.ok_or(PhysicsError::Entity)?)?;
    Ok(StepMove::Moved(StepPost::None))
}
