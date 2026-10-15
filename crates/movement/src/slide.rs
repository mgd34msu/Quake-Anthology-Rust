//! Shared bounded plane clipping; native differences are selected as rules data.
use crate::physics::Step;
use qa_core::{
    math,
    primitives::{Body, MotionWrite, RuleSetId, Vec3},
};
use qa_gameplay::rules::sweep::{LegacySweepRules, player_sweep};
use qa_world::collision::{Trace, TraceError};

/// Compute the native plane response before any aliased output stores.
/// Q3 divides nonnegative dot products; legacy overbounce always multiplies.
pub fn clip_backoff(
    velocity: Vec3,
    normal: Vec3,
    overbounce: f32,
    reciprocal_on_positive: bool,
) -> f32 {
    let dot = velocity.dot(normal);
    if reciprocal_on_positive {
        if dot < 0.0 {
            dot * overbounce
        } else {
            dot / overbounce
        }
    } else {
        dot * overbounce
    }
}

pub(crate) fn clip(velocity: Vec3, normal: Vec3, rules: RuleSetId) -> Vec3 {
    let arena = rules == RuleSetId::Quake3;
    let overbounce = if arena {
        1.001
    } else if matches!(rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease) {
        1.01
    } else {
        1.0
    };
    clip_vector(velocity, normal, overbounce, arena, (!arena).then_some(0.1))
}

fn clip_vector(
    velocity: Vec3,
    normal: Vec3,
    overbounce: f32,
    reciprocal_on_positive: bool,
    stop_epsilon: Option<f32>,
) -> Vec3 {
    let backoff = clip_backoff(velocity, normal, overbounce, reciprocal_on_positive);
    Vec3(std::array::from_fn(|axis| {
        math::clip_component(velocity.0[axis], normal.0[axis], backoff, stop_epsilon)
    }))
}

pub trait SweepHost {
    type Error;
    type Hit: Copy;

    fn read(&mut self) -> Result<Option<Body>, Self::Error>;
    fn write(&mut self, field: MotionWrite, value: f32) -> Result<(), Self::Error>;
    fn trace(&mut self, start: Vec3, end: Vec3) -> Result<(Trace, Self::Hit), Self::Error>;
    fn contact(&mut self, trace: &Trace, hit: Self::Hit, bits: u8) -> Result<(), Self::Error>;
    fn recover_position(&mut self, position: Vec3) -> Result<Option<Vec3>, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweepExit {
    Complete,
    Still,
    Reversed,
    AllSolid,
    PlaneCapacity,
    NoCrease,
    Removed,
}
impl SweepExit {
    pub const fn restores_timed_velocity(self) -> bool {
        matches!(self, Self::Complete | Self::Still | Self::Reversed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SweepResult {
    pub blocked: u8,
    pub exit: SweepExit,
}

fn write_vector<H: SweepHost>(
    host: &mut H,
    field: impl Fn(usize) -> MotionWrite,
    value: Vec3,
) -> Result<bool, H::Error> {
    for axis in 0..3 {
        host.write(field(axis), value.0[axis])?;
        if host.read()?.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod clip_tests {
    use super::*;

    #[test]
    fn shared_scalar_clipping_preserves_all_existing_player_policies() {
        let normal = Vec3([1.0, 0.0, 0.0]);
        for (rules, incoming_x, outgoing_x, outgoing_y) in [
            (RuleSetId::Quake, -10.0, 0, 0),
            (RuleSetId::QuakeWorld, -10.0, 0, 0),
            (RuleSetId::Quake2, -10.0, 0x3dcc_cd00, 0),
            (RuleSetId::Quake2Rerelease, -10.0, 0x3dcc_cd00, 0),
            (RuleSetId::Quake3, -10.0, 0x3c23_d800, 0x3d4c_cccd),
            (RuleSetId::Quake2, 10.0, 0xbdcc_cd00, 0),
            (RuleSetId::Quake3, 10.0, 0x3c23_b000, 0x3d4c_cccd),
        ] {
            assert_eq!(
                clip(Vec3([incoming_x, 0.05, 0.1]), normal, rules)
                    .0
                    .map(f32::to_bits),
                [outgoing_x, outgoing_y, 0x3dcc_cccd],
                "{rules:?} incoming_x={incoming_x}"
            );
        }
    }

    #[test]
    fn backoff_preserves_negative_zero_and_nan_reaches_all_components() {
        let input = Vec3([-0.0, -0.0, -0.0]);
        let normal = Vec3([1.0; 3]);
        assert_eq!(
            clip_backoff(input, normal, 1.001, true).to_bits(),
            (-0.0_f32).to_bits()
        );
        let input = Vec3([1.0, f32::NAN, 3.0]);
        assert!(
            clip(input, normal, RuleSetId::Quake3)
                .0
                .into_iter()
                .all(f32::is_nan)
        );
    }
}

/// One bounded sweep over the caller's current body. Native stores and
/// callbacks may refresh aliased fields or retire the captured lifetime.
pub fn sweep<H: SweepHost>(
    host: &mut H,
    dt: f32,
    rules: &LegacySweepRules,
) -> Result<SweepResult, H::Error> {
    let Some(body) = host.read()? else {
        return Ok(SweepResult {
            blocked: 0,
            exit: SweepExit::Removed,
        });
    };
    let primal = body.velocity;
    let mut original = primal;
    let mut planes = [Vec3::default(); 5];
    let mut count = 0;
    let mut remaining = dt;
    let mut blocked = 0;
    let mut exit = SweepExit::Complete;
    for _ in 0..4 {
        let Some(body) = host.read()? else {
            return Ok(SweepResult {
                blocked,
                exit: SweepExit::Removed,
            });
        };
        if rules.stop_when_still && body.velocity == Vec3::default() {
            exit = SweepExit::Still;
            break;
        }
        let start = body.position;
        let end = if rules.time_first {
            Vec3(std::array::from_fn(|axis| {
                start.0[axis] + remaining * body.velocity.0[axis]
            }))
        } else {
            start + body.velocity * remaining
        };
        let (mut tr, hit) = host.trace(start, end)?;
        if tr.all_solid || rules.stop_start_solid && tr.start_solid {
            let live = if rules.vertical_solid_stop {
                host.write(MotionWrite::Velocity(2), 0.0)?;
                host.read()?.is_some()
            } else {
                write_vector(host, MotionWrite::Velocity, Vec3::default())?
            };
            if !live {
                return Ok(SweepResult {
                    blocked: 3,
                    exit: SweepExit::Removed,
                });
            }
            if rules.contact_all_solid {
                host.contact(&tr, hit, 3)?;
                if host.read()?.is_none() {
                    return Ok(SweepResult {
                        blocked: 3,
                        exit: SweepExit::Removed,
                    });
                }
            }
            return Ok(SweepResult {
                blocked: 3,
                exit: SweepExit::AllSolid,
            });
        }
        if rules.secondary_plane
            && let Some(secondary) = tr.secondary_plane
        {
            let Some(body) = host.read()? else {
                return Ok(SweepResult {
                    blocked,
                    exit: SweepExit::Removed,
                });
            };
            let a = clip_vector(
                body.velocity,
                tr.plane.normal,
                rules.overbounce,
                false,
                Some(rules.stop_epsilon),
            );
            let b = clip_vector(
                body.velocity,
                secondary.normal,
                rules.overbounce,
                false,
                Some(rules.stop_epsilon),
            );
            if (0..3).any(|axis| a.0[axis].abs() < b.0[axis].abs()) {
                tr.plane = secondary;
                tr.surface_id = tr.secondary_surface_id;
                // Both KEX contact slots retain the primary surface flags.
            }
        }
        if tr.fraction > 0.0 {
            if !write_vector(host, MotionWrite::Origin, tr.end)? {
                return Ok(SweepResult {
                    blocked,
                    exit: SweepExit::Removed,
                });
            }
            count = 0;
            if rules.reset_original {
                let Some(body) = host.read()? else {
                    return Ok(SweepResult {
                        blocked,
                        exit: SweepExit::Removed,
                    });
                };
                original = body.velocity;
            }
        }
        if tr.fraction == 1.0 {
            break;
        }
        let mut hit_bits = 0;
        if tr.plane.normal.0[2] > 0.7 {
            hit_bits |= 1;
        }
        if tr.plane.normal.0[2] == 0.0 {
            hit_bits |= 2;
        }
        blocked |= hit_bits;
        host.contact(&tr, hit, hit_bits)?;
        let Some(body) = host.read()? else {
            return Ok(SweepResult {
                blocked,
                exit: SweepExit::Removed,
            });
        };
        remaining -= remaining * tr.fraction;
        if count == 5 {
            let live = write_vector(host, MotionWrite::Velocity, Vec3::default())?;
            return Ok(SweepResult {
                blocked: 3,
                exit: if live {
                    SweepExit::PlaneCapacity
                } else {
                    SweepExit::Removed
                },
            });
        }
        if rules.duplicate_plane
            && planes[..count]
                .iter()
                .any(|&p| p.dot(tr.plane.normal) > 0.99)
        {
            for axis in 0..2 {
                let Some(body) = host.read()? else {
                    return Ok(SweepResult {
                        blocked,
                        exit: SweepExit::Removed,
                    });
                };
                host.write(
                    MotionWrite::Origin(axis),
                    body.position.0[axis] + tr.plane.normal.0[axis] * 0.01,
                )?;
                if host.read()?.is_none() {
                    return Ok(SweepResult {
                        blocked,
                        exit: SweepExit::Removed,
                    });
                }
            }
            let Some(body) = host.read()? else {
                return Ok(SweepResult {
                    blocked,
                    exit: SweepExit::Removed,
                });
            };
            if let Some(position) = host.recover_position(body.position)?
                && !write_vector(host, MotionWrite::Origin, position)?
            {
                return Ok(SweepResult {
                    blocked,
                    exit: SweepExit::Removed,
                });
            }
            continue;
        }
        planes[count] = tr.plane.normal;
        count += 1;
        let mut accepted = false;
        let mut candidate = body.velocity;
        for i in 0..count {
            candidate = clip_vector(
                if rules.clip_current {
                    candidate
                } else {
                    original
                },
                planes[i],
                rules.overbounce,
                false,
                Some(rules.stop_epsilon),
            );
            if (0..count).all(|j| {
                i == j || {
                    let dot = candidate.dot(planes[j]);
                    if rules.reject_negative_only {
                        !(dot < 0.0)
                    } else {
                        dot >= 0.0
                    }
                }
            }) {
                accepted = true;
                break;
            }
        }
        if !accepted {
            if count != 2 {
                let live = write_vector(host, MotionWrite::Velocity, Vec3::default())?;
                return Ok(SweepResult {
                    blocked: 7,
                    exit: if live {
                        SweepExit::NoCrease
                    } else {
                        SweepExit::Removed
                    },
                });
            }
            let direction = math::cross(planes[0], planes[1]);
            candidate = direction
                * direction.dot(if rules.crease_current {
                    body.velocity
                } else {
                    candidate
                });
        }
        if !write_vector(host, MotionWrite::Velocity, candidate)? {
            return Ok(SweepResult {
                blocked,
                exit: SweepExit::Removed,
            });
        }
        let Some(body) = host.read()? else {
            return Ok(SweepResult {
                blocked,
                exit: SweepExit::Removed,
            });
        };
        if body.velocity.dot(primal) <= 0.0 {
            if !write_vector(host, MotionWrite::Velocity, Vec3::default())? {
                return Ok(SweepResult {
                    blocked,
                    exit: SweepExit::Removed,
                });
            }
            exit = SweepExit::Reversed;
            break;
        }
    }
    Ok(SweepResult { blocked, exit })
}

struct PlayerSweep<'a, 'state> {
    step: &'a mut Step<'state>,
    ground_on_brush: bool,
}
impl SweepHost for PlayerSweep<'_, '_> {
    type Error = TraceError;
    type Hit = ();

    fn read(&mut self) -> Result<Option<Body>, TraceError> {
        Ok(Some(self.step.player.body))
    }
    fn write(&mut self, field: MotionWrite, value: f32) -> Result<(), TraceError> {
        match field {
            MotionWrite::Origin(axis) => self.step.player.body.position.0[axis] = value,
            MotionWrite::Velocity(axis) => self.step.player.body.velocity.0[axis] = value,
            MotionWrite::Angles(_) | MotionWrite::AngularVelocity(_) => {
                return Err(TraceError::Scratch);
            }
        }
        Ok(())
    }
    fn trace(&mut self, start: Vec3, end: Vec3) -> Result<(Trace, ()), TraceError> {
        self.step.trace(start, end).map(|trace| (trace, ()))
    }
    fn contact(&mut self, trace: &Trace, (): (), bits: u8) -> Result<(), TraceError> {
        self.step.contact(trace);
        if self.ground_on_brush && bits & 1 != 0 && trace.brush_solid {
            self.step.player.movement.grounded = true;
            self.step.player.movement.ground = trace.entity;
            self.step.player.movement.ground_normal = trace.plane.normal;
            self.step.player.movement.ground_surface = trace.surface;
        }
        Ok(())
    }
    fn recover_position(&mut self, position: Vec3) -> Result<Option<Vec3>, TraceError> {
        if self.step.parameters.special.recover_stuck {
            self.step.recover_position(position)
        } else {
            Ok(None)
        }
    }
}

fn legacy_slide(step: &mut Step<'_>, rules: &LegacySweepRules) -> Result<u8, TraceError> {
    let primal = step.player.body.velocity;
    let dt = step.dt;
    let result = sweep(
        &mut PlayerSweep {
            step,
            ground_on_brush: rules.ground_on_brush,
        },
        dt,
        rules,
    )?;
    if result.exit.restores_timed_velocity()
        && (step.qw() && step.player.movement.water_jump_seconds != 0.0
            || !step.nq() && !step.qw() && step.player.movement.remaining_ms != 0)
    {
        step.player.body.velocity = primal;
    }
    Ok(result.blocked)
}
fn arena_slide(step: &mut Step<'_>, gravity: bool) -> Result<u8, TraceError> {
    let mut primal = step.player.body.velocity;
    let mut end_velocity = primal;
    if gravity {
        end_velocity.0[2] -= step.parameters.gravity * step.dt;
        step.player.body.velocity.0[2] = (step.player.body.velocity.0[2] + end_velocity.0[2]) * 0.5;
        primal.0[2] = end_velocity.0[2];
        if step.ground_plane {
            step.player.body.velocity = clip(
                step.player.body.velocity,
                step.ground_normal,
                RuleSetId::Quake3,
            );
        }
    }
    let mut planes = [Vec3::default(); 5];
    let mut count = 0;
    if step.ground_plane {
        planes[count] = step.ground_normal;
        count += 1;
    }
    planes[count] = math::normalized(step.player.body.velocity);
    count += 1;
    let mut remaining = step.dt;
    let mut blocked = 0;
    for _ in 0..4 {
        let start = step.player.body.position;
        let tr = step.trace(start, start + step.player.body.velocity * remaining)?;
        if tr.all_solid {
            step.player.body.velocity.0[2] = 0.0;
            return Ok(3);
        }
        if tr.fraction > 0.0 {
            step.player.body.position = tr.end;
        }
        if tr.fraction == 1.0 {
            break;
        }
        blocked |= 2;
        step.contact(&tr);
        remaining -= remaining * tr.fraction;
        if count == 5 {
            step.player.body.velocity = Vec3::default();
            return Ok(3);
        }
        let normal = tr.plane.normal;
        if planes[..count].iter().any(|&p| normal.dot(p) > 0.99) {
            step.player.body.velocity = step.player.body.velocity + normal;
            continue;
        }
        planes[count] = normal;
        count += 1;
        for i in 0..count {
            if step.player.body.velocity.dot(planes[i]) >= 0.1 {
                continue;
            }
            let mut candidate = clip(step.player.body.velocity, planes[i], RuleSetId::Quake3);
            let mut end_candidate = clip(end_velocity, planes[i], RuleSetId::Quake3);
            for j in 0..count {
                if j == i || candidate.dot(planes[j]) >= 0.1 {
                    continue;
                }
                candidate = clip(candidate, planes[j], RuleSetId::Quake3);
                end_candidate = clip(end_candidate, planes[j], RuleSetId::Quake3);
                if candidate.dot(planes[i]) >= 0.0 {
                    continue;
                }
                let direction = math::normalized(math::cross(planes[i], planes[j]));
                candidate = direction * direction.dot(step.player.body.velocity);
                end_candidate = direction * direction.dot(end_velocity);
                if (0..count).any(|k| k != i && k != j && candidate.dot(planes[k]) < 0.1) {
                    step.player.body.velocity = Vec3::default();
                    return Ok(3);
                }
            }
            step.player.body.velocity = candidate;
            end_velocity = end_candidate;
            break;
        }
    }
    if gravity {
        step.player.body.velocity = end_velocity;
    }
    if step.player.movement.remaining_ms != 0 {
        step.player.body.velocity = primal;
    }
    Ok(blocked)
}
pub(crate) fn slide(step: &mut Step<'_>, gravity: bool) -> Result<u8, TraceError> {
    if let Some(rules) = player_sweep(step.parameters.rules) {
        legacy_slide(step, &rules)
    } else {
        arena_slide(step, gravity)
    }
}
fn horizontal_distance(a: Vec3, b: Vec3) -> f32 {
    let delta = a - b;
    delta.0[0] * delta.0[0] + delta.0[1] * delta.0[1]
}

pub(crate) fn step_slide(step: &mut Step<'_>, gravity: bool) -> Result<(), TraceError> {
    let start = step.player.body.position;
    let start_velocity = step.player.body.velocity;
    let old_ground = step.player.movement.grounded;
    if step.nq() {
        step.player.movement.grounded = false;
        step.player.movement.ground = None;
    }
    if step.qw() && old_ground {
        step.player.body.velocity.0[2] = 0.0;
        let tr = step.trace(start, start + step.player.body.velocity * step.dt)?;
        if tr.fraction == 1.0 {
            step.player.body.position = tr.end;
            return Ok(());
        }
    }
    let blocked = slide(step, gravity)?;
    if step.parameters.no_step || step.qw() && !old_ground {
        return Ok(());
    }
    if step.nq() && (blocked & 2 == 0 || !old_ground && step.player.movement.water_level == 0) {
        return Ok(());
    }
    if step.arena() && blocked == 0 {
        return Ok(());
    }
    if step.arena() {
        let ground = step.trace(start, start + Vec3([0.0, 0.0, -18.0]))?;
        if step.player.body.velocity.0[2] > 0.0
            && (ground.fraction == 1.0 || ground.plane.normal.0[2] < 0.7)
        {
            return Ok(());
        }
    }
    let down = step.player.body.position;
    let down_velocity = step.player.body.velocity;
    let up = start + Vec3([0.0, 0.0, 18.0]);
    let raised = if step.classic() {
        step.trace(up, up)?
    } else {
        step.trace(start, up)?
    };
    if raised.all_solid {
        return Ok(());
    }
    step.player.body.position = if step.classic() { up } else { raised.end };
    step.player.body.velocity = start_velocity;
    if step.nq() {
        step.player.body.velocity.0[2] = 0.0;
    }
    let height = step.player.body.position.0[2] - start.0[2];
    slide(step, gravity)?;
    let mut drop = step.player.body.position;
    drop.0[2] -= if step.nq() {
        18.0 - start_velocity.0[2] * step.dt
    } else if step.classic() || step.qw() {
        18.0
    } else {
        height
    };
    let original_drop = drop;
    if step.parameters.special.lower_step_probe && start.0[2] < drop.0[2] {
        drop.0[2] = start.0[2] - 1.0;
    }
    let dropped = step.trace(step.player.body.position, drop)?;
    if !dropped.all_solid {
        step.player.body.position = if step.parameters.special.lower_step_probe {
            step.result.step_clip |= step.player.body.velocity.0[2] > 0.0;
            step.trace(step.player.body.position, original_drop)?.end
        } else {
            dropped.end
        };
    }
    if step.arena() {
        if dropped.fraction < 1.0 {
            step.player.body.velocity = clip(
                step.player.body.velocity,
                dropped.plane.normal,
                RuleSetId::Quake3,
            );
        }
        return Ok(());
    }
    if dropped.plane.normal.0[2] < 0.7
        || !step.nq()
            && horizontal_distance(down, start)
                > horizontal_distance(step.player.body.position, start)
    {
        step.player.body.position = down;
        step.player.body.velocity = down_velocity;
    } else if !step.nq() {
        step.player.body.velocity.0[2] = down_velocity.0[2];
    }
    if step.parameters.special.settle_ground_step
        && step.player.movement.grounded
        && !step.player.movement.ladder
        && (step.player.movement.water_level < 2
            || step.command.buttons & qa_core::primitives::buttons::JUMP == 0
                && step.player.body.velocity.0[2] <= 0.0)
    {
        let origin = step.player.body.position;
        let trace = step.trace(origin, origin - Vec3([0.0, 0.0, 18.0]))?;
        if trace.fraction < 1.0 {
            step.player.body.position = trace.end;
        }
    }
    Ok(())
}
