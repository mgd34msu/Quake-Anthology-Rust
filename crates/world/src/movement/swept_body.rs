//! Shared swept-body solver.
//!
//! Donor provenance: `src/movement/swept-body.ts`.
//!
//! Source adapters own state access, contact effects, and arithmetic through
//! [`SweptBodyServices`]; the solver owns only the bump/plane iteration.

use qa_core::math::{Vec3, vec3};

pub use super::{MAX_BUMPS, MAX_PLANES};

/// Body state visible to the solver.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SweptBodyState {
    /// Current origin.
    pub origin: Vec3,
    /// Current velocity.
    pub velocity: Vec3,
}

/// Minimal trace view the solver needs. Native traces implement this.
pub trait SweptBodyTrace: Clone {
    /// Travel fraction consumed.
    fn fraction(&self) -> f64;
    /// Trace end position.
    fn end(&self) -> Vec3;
    /// Entire sweep inside solid.
    fn all_solid(&self) -> bool;
    /// Sweep start inside solid.
    fn start_solid(&self) -> bool;
}

/// How the solver treats repeated contacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionOriginalVelocity {
    /// Always clip the frame's initial velocity.
    Initial,
    /// Clip the velocity at each bump's progress point.
    LastProgress,
}

/// Which velocity feeds sequential plane candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateVelocity {
    /// Original velocity for every candidate.
    Original,
    /// Last candidate feeds the next plane.
    Sequential,
}

/// Which velocity feeds the crease direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreaseVelocity {
    /// Last rejected candidate.
    LastCandidate,
    /// Current velocity.
    Current,
}

/// What an all-solid stop writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllSolidVelocity {
    /// Zero the whole velocity.
    Zero,
    /// Zero only the vertical component.
    ZeroZ,
}

/// Collision policy knobs, mirroring donor `collisionPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionPolicy {
    /// Stop on start-solid even without all-solid.
    pub stop_on_start_solid: bool,
    /// Which velocity seeds plane candidates.
    pub original_velocity: CollisionOriginalVelocity,
    /// What an all-solid stop writes.
    pub all_solid_velocity: AllSolidVelocity,
    /// Which velocity feeds sequential candidates.
    pub candidate_velocity: CandidateVelocity,
    /// Which velocity feeds the crease direction.
    pub crease_velocity: CreaseVelocity,
}

impl Default for CollisionPolicy {
    fn default() -> Self {
        Self {
            stop_on_start_solid: false,
            original_velocity: CollisionOriginalVelocity::LastProgress,
            all_solid_velocity: AllSolidVelocity::Zero,
            candidate_velocity: CandidateVelocity::Original,
            crease_velocity: CreaseVelocity::Current,
        }
    }
}

/// Solver stop reason, mirroring donor `SweepStop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepStop {
    /// Sweep completed its travel.
    Complete,
    /// The body was removed mid-sweep.
    Removed,
    /// The body wedged solid.
    Solid,
    /// Plane budget exhausted.
    PlaneLimit,
    /// Crease blocked by a third plane.
    CreaseBlocked,
    /// Result reversed against the initial velocity.
    Reversed,
}

/// Source-owned sweep services: state access, traces, contacts, arithmetic.
pub trait SweptBodyServices {
    /// Native trace type.
    type Trace: SweptBodyTrace;

    /// Read current body state; `None` removes the body.
    fn read(&mut self) -> Option<SweptBodyState>;
    /// Write origin after progress.
    fn write_origin(&mut self, origin: Vec3);
    /// Write velocity after clipping.
    fn write_velocity(&mut self, velocity: Vec3);
    /// Zero only the vertical velocity (Q2 all-solid response).
    fn write_velocity_zero_z(&mut self) {
        if let Some(state) = self.read() {
            self.write_velocity(vec3(state.velocity.x, state.velocity.y, 0.0));
        }
    }
    /// Trace from `start` to `end`.
    fn trace(&mut self, start: Vec3, end: Vec3) -> Self::Trace;
    /// Rewrite a trace before progress is consumed (Q2 secondary plane).
    fn prepare_trace(&mut self, _trace: &mut Self::Trace) {}
    /// All-solid contact hook.
    fn solid(&mut self, _trace: &Self::Trace) {}
    /// Blocked-trace contact hook.
    fn touch(&mut self, _trace: &Self::Trace) {}
    /// Contact normal for a blocked trace.
    fn normal(&mut self, trace: &Self::Trace) -> Vec3;
    /// Impact hook after the normal resolves.
    fn impact(&mut self, _trace: &Self::Trace, _normal: Vec3) {}
    /// Stop early when the body is still (Q1 fly-move).
    fn stop_when_still(&self) -> bool;
    /// Collision policy knobs.
    fn collision_policy(&self) -> CollisionPolicy {
        CollisionPolicy::default()
    }
    /// Paired (Q3 gravity) plane response: seeds plus enter threshold.
    fn paired_response(&self) -> Option<PairedResponse> {
        None
    }
    /// Current paired end velocity (Q3 gravity copy).
    fn paired_end_velocity(&mut self) -> Vec3 {
        vec3(0.0, 0.0, 0.0)
    }
    /// Write the paired end velocity.
    fn write_paired_end_velocity(&mut self, _velocity: Vec3) {}
    /// Normalize a crease direction (Q3 normalize keeps zero input).
    fn normalize(&mut self, direction: Vec3) -> Vec3 {
        direction
    }
    /// Record impact speed against a plane (Q3 landing damage).
    fn impact_speed(&mut self, _speed: f32) {}
    /// Duplicate-plane dot threshold, when the source recovers from them.
    fn duplicate_threshold(&self) -> Option<f64> {
        None
    }
    /// Recover from a duplicate plane (Q3 nudges velocity along it).
    fn recover_duplicate(&mut self, _normal: Vec3) {}
    /// Plane identity for candidate acceptance.
    fn same_plane(&mut self, first: Vec3, second: Vec3) -> bool;

    /// Advance a point by time along a velocity.
    fn advance(&mut self, origin: Vec3, time: f64, velocity: Vec3) -> Vec3;
    /// Remaining time after consuming a fraction.
    fn remaining(&mut self, time: f64, fraction: f64) -> f64;
    /// Clip a velocity against a plane normal.
    fn clip(&mut self, velocity: Vec3, normal: Vec3) -> Vec3;
    /// Dot product.
    fn dot(&mut self, first: Vec3, second: Vec3) -> f64;
    /// Cross product.
    fn cross(&mut self, first: Vec3, second: Vec3) -> Vec3;
    /// Scale a vector.
    fn scale(&mut self, vector: Vec3, amount: f64) -> Vec3;
}

/// Paired (Q3 gravity) plane-response configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct PairedResponse {
    /// Seed planes (ground plus normalized velocity).
    pub seeds: Vec<Vec3>,
    /// Enter threshold for plane penetration.
    pub enter_threshold: f64,
}

/// Swept movement; source adapters own state access, contact effects and
/// arithmetic. Faithful port of donor `sweepBody`.
pub fn sweep_body<S: SweptBodyServices>(services: &mut S, elapsed: f64) -> SweepStop {
    let zero = vec3(0.0, 0.0, 0.0);
    let initial = match services.read() {
        Some(state) => state,
        None => return SweepStop::Removed,
    };
    let paired = services.paired_response();
    let mut planes: Vec<Vec3> = paired.as_ref().map(|p| p.seeds.clone()).unwrap_or_default();
    let primal = initial.velocity;
    let mut original = initial.velocity;
    let mut remaining = elapsed;
    let policy = services.collision_policy();
    for _ in 0..MAX_BUMPS {
        let state = match services.read() {
            Some(state) => state,
            None => return SweepStop::Removed,
        };
        if services.stop_when_still()
            && state.velocity.x == 0.0
            && state.velocity.y == 0.0
            && state.velocity.z == 0.0
        {
            break;
        }
        let target = services.advance(state.origin, remaining, state.velocity);
        let mut trace = services.trace(state.origin, target);
        if trace.all_solid() || (policy.stop_on_start_solid && trace.start_solid()) {
            if policy.all_solid_velocity == AllSolidVelocity::ZeroZ {
                services.write_velocity_zero_z();
            } else {
                services.write_velocity(zero);
            }
            services.solid(&trace);
            return SweepStop::Solid;
        }
        services.prepare_trace(&mut trace);
        if trace.fraction() > 0.0 {
            services.write_origin(trace.end());
            if policy.original_velocity != CollisionOriginalVelocity::Initial {
                original = state.velocity;
            }
            if paired.is_none() {
                planes.clear();
            }
        }
        if trace.fraction() == 1.0 {
            break;
        }
        services.touch(&trace);
        if paired.is_some() {
            remaining = services.remaining(remaining, trace.fraction());
            if planes.len() >= MAX_PLANES {
                services.write_velocity(zero);
                return SweepStop::PlaneLimit;
            }
        }
        let normal = services.normal(&trace);
        services.impact(&trace, normal);
        let current = match services.read() {
            Some(state) => state,
            None => return SweepStop::Removed,
        };
        if paired.is_none() {
            remaining = services.remaining(remaining, trace.fraction());
            if planes.len() >= MAX_PLANES {
                services.write_velocity(zero);
                return SweepStop::PlaneLimit;
            }
        }
        if let Some(threshold) = services.duplicate_threshold() {
            let mut duplicate = false;
            for plane in &planes {
                if services.dot(normal, *plane) > threshold {
                    duplicate = true;
                    break;
                }
            }
            if duplicate {
                services.recover_duplicate(normal);
                continue;
            }
        }
        planes.push(normal);
        let mut velocity: Option<Vec3> = None;
        let mut last_candidate = current.velocity;
        for (index, plane) in planes.clone().iter().enumerate() {
            if let Some(paired) = paired.as_ref() {
                let into = services.dot(current.velocity, *plane);
                if into >= paired.enter_threshold {
                    continue;
                }
                services.impact_speed(-into as f32);
                let mut clipped = services.clip(current.velocity, *plane);
                let end_velocity = services.paired_end_velocity();
                let mut end_clipped = services.clip(end_velocity, *plane);
                for (second_index, second) in planes.clone().iter().enumerate() {
                    if second_index == index || services.dot(clipped, *second) >= paired.enter_threshold {
                        continue;
                    }
                    clipped = services.clip(clipped, *second);
                    end_clipped = services.clip(end_clipped, *second);
                    if services.dot(clipped, *plane) >= 0.0 {
                        continue;
                    }
                    let crease = services.cross(*plane, *second);
                    let direction = services.normalize(crease);
                    let along = services.dot(direction, current.velocity);
                    clipped = services.scale(direction, along);
                    let end_velocity = services.paired_end_velocity();
                    let end_along = services.dot(direction, end_velocity);
                    end_clipped = services.scale(direction, end_along);
                    let mut blocked = false;
                    for (third_index, third) in planes.iter().enumerate() {
                        if third_index != index
                            && third_index != second_index
                            && services.dot(clipped, *third) < paired.enter_threshold
                        {
                            blocked = true;
                            break;
                        }
                    }
                    if blocked {
                        services.write_velocity(zero);
                        return SweepStop::CreaseBlocked;
                    }
                }
                velocity = Some(clipped);
                services.write_paired_end_velocity(end_clipped);
                break;
            }
            let seed = if policy.candidate_velocity == CandidateVelocity::Sequential {
                last_candidate
            } else {
                original
            };
            let candidate = services.clip(seed, *plane);
            last_candidate = candidate;
            let mut accepted = true;
            for other in &planes {
                if services.same_plane(*other, *plane) {
                    continue;
                }
                if services.dot(candidate, *other) < 0.0 {
                    accepted = false;
                    break;
                }
            }
            if accepted {
                velocity = Some(candidate);
                break;
            }
        }
        if paired.is_some() {
            if let Some(velocity) = velocity {
                services.write_velocity(velocity);
            }
            continue;
        }
        let mut velocity = match velocity {
            Some(velocity) => velocity,
            None => {
                if planes.len() != 2 {
                    services.write_velocity(zero);
                    return SweepStop::CreaseBlocked;
                }
                let direction = services.cross(planes[0], planes[1]);
                let seed = if policy.crease_velocity == CreaseVelocity::LastCandidate {
                    last_candidate
                } else {
                    current.velocity
                };
                let along = services.dot(direction, seed);
                services.scale(direction, along)
            }
        };
        let _ = &mut velocity;
        if services.dot(velocity, primal) <= 0.0 {
            services.write_velocity(zero);
            return SweepStop::Reversed;
        }
        services.write_velocity(velocity);
    }
    SweepStop::Complete
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{add3, cross3, dot3, scale3};

    #[derive(Debug, Clone)]
    struct FakeTrace {
        fraction: f64,
        end: Vec3,
        all_solid: bool,
        start_solid: bool,
        normal: Vec3,
    }

    impl SweptBodyTrace for FakeTrace {
        fn fraction(&self) -> f64 {
            self.fraction
        }
        fn end(&self) -> Vec3 {
            self.end
        }
        fn all_solid(&self) -> bool {
            self.all_solid
        }
        fn start_solid(&self) -> bool {
            self.start_solid
        }
    }

    struct Harness {
        state: Option<SweptBodyState>,
        traces: Vec<FakeTrace>,
        touches: usize,
        removed_after_impact: bool,
    }

    impl SweptBodyServices for Harness {
        type Trace = FakeTrace;

        fn read(&mut self) -> Option<SweptBodyState> {
            self.state
        }
        fn write_origin(&mut self, origin: Vec3) {
            if let Some(state) = self.state.as_mut() {
                state.origin = origin;
            }
        }
        fn write_velocity(&mut self, velocity: Vec3) {
            if let Some(state) = self.state.as_mut() {
                state.velocity = velocity;
            }
        }
        fn trace(&mut self, _start: Vec3, _end: Vec3) -> FakeTrace {
            self.traces.remove(0)
        }
        fn touch(&mut self, _trace: &FakeTrace) {
            self.touches += 1;
        }
        fn normal(&mut self, trace: &FakeTrace) -> Vec3 {
            trace.normal
        }
        fn impact(&mut self, _trace: &FakeTrace, _normal: Vec3) {
            if self.removed_after_impact {
                self.state = None;
            }
        }
        fn stop_when_still(&self) -> bool {
            true
        }
        fn same_plane(&mut self, _first: Vec3, _second: Vec3) -> bool {
            false
        }
        fn advance(&mut self, origin: Vec3, time: f64, velocity: Vec3) -> Vec3 {
            add3(origin, scale3(velocity, time as f32))
        }
        fn remaining(&mut self, time: f64, fraction: f64) -> f64 {
            time - time * fraction
        }
        fn clip(&mut self, velocity: Vec3, normal: Vec3) -> Vec3 {
            let backoff = dot3(velocity, normal);
            Vec3 {
                x: velocity.x - normal.x * backoff,
                y: velocity.y - normal.y * backoff,
                z: velocity.z - normal.z * backoff,
            }
        }
        fn dot(&mut self, first: Vec3, second: Vec3) -> f64 {
            f64::from(dot3(first, second))
        }
        fn cross(&mut self, first: Vec3, second: Vec3) -> Vec3 {
            cross3(first, second)
        }
        fn scale(&mut self, vector: Vec3, amount: f64) -> Vec3 {
            scale3(vector, amount as f32)
        }
    }

    fn open_trace(end: Vec3) -> FakeTrace {
        FakeTrace {
            fraction: 1.0,
            end,
            all_solid: false,
            start_solid: false,
            normal: vec3(0.0, 0.0, 1.0),
        }
    }

    #[test]
    fn removed_body_reports_removed() {
        let mut harness = Harness {
            state: None,
            traces: Vec::new(),
            touches: 0,
            removed_after_impact: false,
        };
        assert_eq!(sweep_body(&mut harness, 0.1), SweepStop::Removed);
    }

    #[test]
    fn still_body_completes_without_tracing() {
        let mut harness = Harness {
            state: Some(SweptBodyState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
            }),
            traces: Vec::new(),
            touches: 0,
            removed_after_impact: false,
        };
        assert_eq!(sweep_body(&mut harness, 0.1), SweepStop::Complete);
    }

    #[test]
    fn open_travel_advances_origin() {
        let mut harness = Harness {
            state: Some(SweptBodyState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(10.0, 0.0, 0.0),
            }),
            traces: vec![open_trace(vec3(1.0, 0.0, 0.0))],
            touches: 0,
            removed_after_impact: false,
        };
        assert_eq!(sweep_body(&mut harness, 0.1), SweepStop::Complete);
        assert_eq!(harness.state.unwrap().origin, vec3(1.0, 0.0, 0.0));
    }

    #[test]
    fn solid_stop_zeroes_velocity() {
        let mut harness = Harness {
            state: Some(SweptBodyState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(10.0, 0.0, 0.0),
            }),
            traces: vec![FakeTrace {
                fraction: 0.0,
                end: vec3(0.0, 0.0, 0.0),
                all_solid: true,
                start_solid: true,
                normal: vec3(0.0, 0.0, 1.0),
            }],
            touches: 0,
            removed_after_impact: false,
        };
        assert_eq!(sweep_body(&mut harness, 0.1), SweepStop::Solid);
        assert_eq!(harness.state.unwrap().velocity, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn wall_clip_slides_and_touches() {
        let wall = FakeTrace {
            fraction: 0.5,
            end: vec3(0.5, 0.0, 0.0),
            all_solid: false,
            start_solid: false,
            normal: vec3(-1.0, 0.0, 0.0),
        };
        let mut harness = Harness {
            state: Some(SweptBodyState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(10.0, 5.0, 0.0),
            }),
            traces: vec![wall, open_trace(vec3(0.5, 0.5, 0.0))],
            touches: 0,
            removed_after_impact: false,
        };
        assert_eq!(sweep_body(&mut harness, 0.1), SweepStop::Complete);
        assert_eq!(harness.touches, 1);
        let velocity = harness.state.unwrap().velocity;
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 5.0);
    }

    #[test]
    fn reversed_clip_reports_reversed() {
        let wall = FakeTrace {
            fraction: 0.0,
            end: vec3(0.0, 0.0, 0.0),
            all_solid: false,
            start_solid: false,
            normal: vec3(-1.0, 0.0, 0.0),
        };
        let mut harness = Harness {
            state: Some(SweptBodyState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(10.0, 0.0, 0.0),
            }),
            traces: vec![wall],
            touches: 0,
            removed_after_impact: false,
        };
        assert_eq!(sweep_body(&mut harness, 0.1), SweepStop::Reversed);
        assert_eq!(harness.state.unwrap().velocity, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn impact_removal_reports_removed() {
        let wall = FakeTrace {
            fraction: 0.5,
            end: vec3(0.5, 0.0, 0.0),
            all_solid: false,
            start_solid: false,
            normal: vec3(-1.0, 0.0, 0.0),
        };
        let mut harness = Harness {
            state: Some(SweptBodyState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(10.0, 5.0, 0.0),
            }),
            traces: vec![wall],
            touches: 0,
            removed_after_impact: true,
        };
        assert_eq!(sweep_body(&mut harness, 0.1), SweepStop::Removed);
    }
}
