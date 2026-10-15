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

/// Goal selection adds live native yaw and absolute bounds to the same mover.
/// Turning follows the caller's current self, independently of the mover row.
pub trait GoalHost: MonsterHost {
    fn ideal_yaw(&mut self) -> Result<f32, Self::Error>;
    fn yaw(&mut self) -> Result<f32, Self::Error>;
    fn write_ideal_yaw(&mut self, value: f32) -> Result<(), Self::Error>;
    fn change_yaw(&mut self) -> Result<(), Self::Error>;
    fn enemy_present(&mut self) -> Result<bool, Self::Error>;
    fn absolute_min(&mut self, target: NativeEntityRef, axis: usize) -> Result<f32, Self::Error>;
    fn absolute_max(&mut self, target: NativeEntityRef, axis: usize) -> Result<f32, Self::Error>;
    fn random_word(&mut self) -> u32;
}

#[derive(Debug)]
pub enum GoalStep {
    Rejected,
    Complete,
    Link(GoalCursor),
}

/// Only source locals survive a relink. Entity fields are read from the host
/// again after callbacks, while the actor and goal pointers stay captured.
#[derive(Debug)]
pub struct GoalCursor {
    actor: NativeEntityRef,
    goal: NativeEntityRef,
    distance: f32,
    rules: StepRules,
    phase: GoalPhase,
}
#[derive(Debug)]
enum GoalPhase {
    Initial {
        moved: bool,
    },
    Chase {
        locals: ChaseLocals,
        next: ChasePhase,
        moved: bool,
    },
}
#[derive(Debug)]
struct ChaseLocals {
    old: f32,
    turnaround: f32,
    dx: f32,
    dy: f32,
    first: f32,
    second: f32,
}
#[derive(Debug)]
enum ChasePhase {
    Swap,
    First,
    Second,
    Old,
    Sweep,
    SweepNext { next: i32, increment: i32 },
    Turnaround,
    Finish,
}

impl GoalCursor {
    pub fn actor(&self) -> NativeEntityRef {
        self.actor
    }

    pub fn begin<H: GoalHost>(
        host: &mut H,
        actor: NativeEntityRef,
        goal: NativeEntityRef,
        distance: f32,
        rules: StepRules,
    ) -> Result<GoalStep, H::Error> {
        if host.flags()? & rules.mobility_flags() == 0 {
            return Ok(GoalStep::Rejected);
        }
        if host.enemy_present()? {
            for axis in 0..3 {
                if host.absolute_min(goal, axis)? > host.absolute_max(actor, axis)? + distance
                    || host.absolute_max(goal, axis)? < host.absolute_min(actor, axis)? - distance
                {
                    return Self::start(host, actor, goal, distance, rules);
                }
            }
            return Ok(GoalStep::Complete);
        }
        Self::start(host, actor, goal, distance, rules)
    }

    fn start<H: GoalHost>(
        host: &mut H,
        actor: NativeEntityRef,
        goal: NativeEntityRef,
        distance: f32,
        rules: StepRules,
    ) -> Result<GoalStep, H::Error> {
        let mut run = Self {
            actor,
            goal,
            distance,
            rules,
            phase: GoalPhase::Initial { moved: false },
        };
        if host.random_word() & 3 == 1 {
            run.chase(host)
        } else {
            let yaw = host.ideal_yaw()?;
            let moved = run.step_direction(host, yaw)?;
            run.phase = GoalPhase::Initial { moved };
            Ok(GoalStep::Link(run))
        }
    }

    pub fn resume<H: GoalHost>(mut self, host: &mut H) -> Result<GoalStep, H::Error> {
        let phase = std::mem::replace(&mut self.phase, GoalPhase::Initial { moved: false });
        match phase {
            GoalPhase::Initial { moved: true } | GoalPhase::Chase { moved: true, .. } => {
                Ok(GoalStep::Complete)
            }
            GoalPhase::Initial { moved: false } => self.chase(host),
            GoalPhase::Chase {
                locals,
                next,
                moved: false,
            } => self.advance_chase(host, locals, next),
        }
    }

    fn chase<H: GoalHost>(mut self, host: &mut H) -> Result<GoalStep, H::Error> {
        let old = qa_core::math::anglemod(
            qa_core::math::native_integer(host.ideal_yaw()? / 45.0).wrapping_mul(45) as f32,
        );
        let turnaround = qa_core::math::anglemod(old - 180.0);
        let target = host.target_origin(self.goal)?;
        let origin = host.origin()?;
        let dx = target.0[0] - origin.0[0];
        let dy = target.0[1] - origin.0[1];
        let first = if dx > 10.0 {
            0.0
        } else if dx < -10.0 {
            180.0
        } else {
            -1.0
        };
        let second = if dy < -10.0 {
            270.0
        } else if dy > 10.0 {
            90.0
        } else {
            -1.0
        };
        let locals = ChaseLocals {
            old,
            turnaround,
            dx,
            dy,
            first,
            second,
        };
        if first != -1.0 && second != -1.0 {
            let diagonal = if first == 0.0 {
                if second == 90.0 { 45.0 } else { 315.0 }
            } else if second == 90.0 {
                135.0
            } else {
                215.0
            };
            if diagonal != turnaround {
                let moved = self.step_direction(host, diagonal)?;
                self.phase = GoalPhase::Chase {
                    locals,
                    next: ChasePhase::Swap,
                    moved,
                };
                return Ok(GoalStep::Link(self));
            }
        }
        self.advance_chase(host, locals, ChasePhase::Swap)
    }

    fn advance_chase<H: GoalHost>(
        mut self,
        host: &mut H,
        mut locals: ChaseLocals,
        mut next: ChasePhase,
    ) -> Result<GoalStep, H::Error> {
        loop {
            let (yaw, following) = match next {
                ChasePhase::Swap => {
                    if host.random_word() & 1 != 0
                        || qa_core::math::native_integer(locals.dy).wrapping_abs()
                            > qa_core::math::native_integer(locals.dx).wrapping_abs()
                    {
                        std::mem::swap(&mut locals.first, &mut locals.second);
                    }
                    next = ChasePhase::First;
                    continue;
                }
                ChasePhase::First => (locals.first, ChasePhase::Second),
                ChasePhase::Second => (locals.second, ChasePhase::Old),
                ChasePhase::Old => {
                    let moved = self.step_direction(host, locals.old)?;
                    self.phase = GoalPhase::Chase {
                        locals,
                        next: ChasePhase::Sweep,
                        moved,
                    };
                    return Ok(GoalStep::Link(self));
                }
                ChasePhase::Sweep => {
                    next = if host.random_word() & 1 != 0 {
                        ChasePhase::SweepNext {
                            next: 0,
                            increment: 45,
                        }
                    } else {
                        ChasePhase::SweepNext {
                            next: 315,
                            increment: -45,
                        }
                    };
                    continue;
                }
                ChasePhase::SweepNext {
                    next: yaw,
                    increment,
                } => {
                    if !(0..=315).contains(&yaw) {
                        next = ChasePhase::Turnaround;
                        continue;
                    }
                    (
                        yaw as f32,
                        ChasePhase::SweepNext {
                            next: yaw + increment,
                            increment,
                        },
                    )
                }
                ChasePhase::Turnaround => (locals.turnaround, ChasePhase::Finish),
                ChasePhase::Finish => {
                    host.write_ideal_yaw(locals.old)?;
                    let origin = host.origin()?;
                    let bounds = host.bounds()?;
                    if !check_bottom(&mut Support(host), origin, bounds, self.rules.height)? {
                        let flags = host.flags()?;
                        host.write_flags(flags | self.rules.partial_ground_flag)?;
                    }
                    return Ok(GoalStep::Complete);
                }
            };
            next = following;
            if yaw != -1.0 && yaw != locals.turnaround {
                let moved = self.step_direction(host, yaw)?;
                self.phase = GoalPhase::Chase {
                    locals,
                    next,
                    moved,
                };
                return Ok(GoalStep::Link(self));
            }
            // The final turnaround is explicitly permitted by the source.
            if matches!(next, ChasePhase::Finish) && yaw != -1.0 {
                let moved = self.step_direction(host, yaw)?;
                self.phase = GoalPhase::Chase {
                    locals,
                    next,
                    moved,
                };
                return Ok(GoalStep::Link(self));
            }
        }
    }

    fn step_direction<H: GoalHost>(&self, host: &mut H, yaw: f32) -> Result<bool, H::Error> {
        host.write_ideal_yaw(yaw)?;
        host.change_yaw()?;
        let movement = qa_movement::yaw_displacement(yaw, self.distance);
        let old_origin = host.origin()?;
        let step = move_step(host, movement, &self.rules)?;
        if let StepMove::Moved(post) = step {
            finish_step(host, post, &self.rules)?;
            let delta = host.yaw()? - host.ideal_yaw()?;
            if delta > 45.0 && delta < 315.0 {
                write_origin(host, old_origin)?;
            }
        }
        Ok(step.moved())
    }
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
