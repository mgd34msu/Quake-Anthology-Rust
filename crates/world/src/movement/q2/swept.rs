//! Quake II body sweep over the shared solver.
//!
//! Donor provenance: `src/movement/q2/swept.ts`. Converts source tuple
//! storage, preserves native trace identity, and delegates iteration to the
//! shared solver.
//!
//! All sweep state flows through one [`Q2SweepBody`] so duplicate-plane
//! recovery and origin writes share a single borrow; rerelease recovery
//! addresses player origin directly and server-entity sweeps address the
//! shared `pml` origin, matching the source alias.

use qa_core::math::{vec3, Vec3};
use qa_core::numeric::NumericOps;

use super::super::swept_body::{
    sweep_body, AllSolidVelocity, CandidateVelocity, CollisionOriginalVelocity, CollisionPolicy, CreaseVelocity,
    SweepStop, SweptBodyServices, SweptBodyState,
};
use super::math::{Q2Math, Q2MathEdition};
use super::types::{SrcVec3, TraceT};

/// Scene-space conversion for source tuples.
pub fn scene(value: SrcVec3) -> Vec3 {
    vec3(value[0] as f32, value[1] as f32, value[2] as f32)
}

/// Source-tuple conversion for scene vectors.
pub fn source(value: Vec3) -> SrcVec3 {
    [f64::from(value.x), f64::from(value.y), f64::from(value.z)]
}

/// Q2 sweep body: origin/velocity storage plus trace, clip, contact, and
/// recovery callbacks behind a single borrow.
pub trait Q2SweepBody {
    /// Read the sweep origin.
    fn origin(&self) -> SrcVec3;
    /// Read the sweep velocity.
    fn velocity(&self) -> SrcVec3;
    /// Write the sweep origin.
    fn write_origin(&mut self, origin: SrcVec3);
    /// Write the sweep velocity.
    fn write_velocity(&mut self, velocity: SrcVec3);
    /// Zero only the vertical velocity (all-solid response).
    fn write_velocity_zero_z(&mut self);
    /// Trace from `start` to `end`.
    fn trace(&mut self, start: SrcVec3, end: SrcVec3) -> TraceT;
    /// Clip a velocity against a plane normal.
    fn clip(&mut self, velocity: SrcVec3, normal: SrcVec3) -> SrcVec3;
    /// Blocked-trace contact hook.
    fn touch(&mut self, trace: &TraceT);
    /// Pre-progress trace rewrite (rerelease secondary plane).
    fn prepare_trace(&mut self, _trace: &mut TraceT) {}
    /// All-solid contact hook.
    fn solid(&mut self, _trace: &TraceT) {}
    /// Duplicate-plane dot threshold, when the source recovers from them.
    fn duplicate_threshold(&self) -> Option<f64> {
        None
    }
    /// Recover from a duplicate plane.
    fn recover_duplicate(&mut self, _normal: SrcVec3) {}
}

struct SweepAdapter<'a> {
    body: &'a mut dyn Q2SweepBody,
    numeric: NumericOps,
    math: Q2Math,
}

impl SweptBodyServices for SweepAdapter<'_> {
    type Trace = Q2SweepTrace;

    fn read(&mut self) -> Option<SweptBodyState> {
        Some(SweptBodyState {
            origin: scene(self.body.origin()),
            velocity: scene(self.body.velocity()),
        })
    }
    fn write_origin(&mut self, origin: Vec3) {
        self.body.write_origin(source(origin));
    }
    fn write_velocity(&mut self, velocity: Vec3) {
        self.body.write_velocity(source(velocity));
    }
    fn write_velocity_zero_z(&mut self) {
        self.body.write_velocity_zero_z();
    }
    fn trace(&mut self, start: Vec3, end: Vec3) -> Q2SweepTrace {
        Q2SweepTrace {
            native: self.body.trace(source(start), source(end)),
        }
    }
    fn prepare_trace(&mut self, trace: &mut Q2SweepTrace) {
        self.body.prepare_trace(&mut trace.native);
    }
    fn solid(&mut self, trace: &Q2SweepTrace) {
        self.body.solid(&trace.native);
    }
    fn touch(&mut self, trace: &Q2SweepTrace) {
        self.body.touch(&trace.native);
    }
    fn normal(&mut self, trace: &Q2SweepTrace) -> Vec3 {
        let normal = trace.native.plane.normal;
        scene(self.math.vec3(normal[0], normal[1], normal[2]))
    }
    fn stop_when_still(&self) -> bool {
        false
    }
    fn collision_policy(&self) -> CollisionPolicy {
        CollisionPolicy {
            stop_on_start_solid: false,
            original_velocity: CollisionOriginalVelocity::Initial,
            all_solid_velocity: AllSolidVelocity::ZeroZ,
            candidate_velocity: CandidateVelocity::Sequential,
            crease_velocity: CreaseVelocity::LastCandidate,
        }
    }
    fn duplicate_threshold(&self) -> Option<f64> {
        self.body.duplicate_threshold()
    }
    fn recover_duplicate(&mut self, normal: Vec3) {
        self.body.recover_duplicate(source(normal));
    }
    fn same_plane(&mut self, _first: Vec3, _second: Vec3) -> bool {
        false
    }
    fn advance(&mut self, origin: Vec3, time: f64, velocity: Vec3) -> Vec3 {
        let mut out = self.math.vec3(0.0, 0.0, 0.0);
        self.math.ma(source(origin), time, source(velocity), &mut out);
        scene(out)
    }
    fn remaining(&mut self, time: f64, fraction: f64) -> f64 {
        let n = self.numeric;
        n.sub(time, n.mul(time, fraction))
    }
    fn clip(&mut self, velocity: Vec3, normal: Vec3) -> Vec3 {
        let clipped = self.body.clip(source(velocity), source(normal));
        let mut out = self.math.vec3(0.0, 0.0, 0.0);
        self.math.copy(clipped, &mut out);
        scene(out)
    }
    fn dot(&mut self, first: Vec3, second: Vec3) -> f64 {
        self.math.dot(source(first), source(second))
    }
    fn cross(&mut self, first: Vec3, second: Vec3) -> Vec3 {
        scene(self.math.cross(source(first), source(second)))
    }
    fn scale(&mut self, vector: Vec3, amount: f64) -> Vec3 {
        scene(self.math.muls(source(vector), amount))
    }
}

/// Native-preserving sweep trace wrapper.
#[derive(Debug, Clone)]
pub struct Q2SweepTrace {
    /// Native source trace.
    pub native: TraceT,
}

impl super::super::swept_body::SweptBodyTrace for Q2SweepTrace {
    fn fraction(&self) -> f64 {
        self.native.fraction
    }
    fn end(&self) -> Vec3 {
        scene(self.native.endpos)
    }
    fn all_solid(&self) -> bool {
        self.native.allsolid
    }
    fn start_solid(&self) -> bool {
        self.native.startsolid
    }
}

/// Sweep a Q2 body through the shared solver.
pub fn sweep_q2_body(body: &mut dyn Q2SweepBody, numeric: NumericOps, elapsed: f64) -> SweepStop {
    let math = Q2Math::new(numeric, Q2MathEdition::Classic);
    let mut adapter = SweepAdapter { body, numeric, math };
    sweep_body(&mut adapter, elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};

    use super::super::types::plane;

    fn open_trace(end: SrcVec3) -> TraceT {
        TraceT {
            allsolid: false,
            startsolid: false,
            fraction: 1.0,
            endpos: end,
            plane: plane(),
            surface: None,
            contents: 0,
            ent: None,
            plane2: plane(),
            surface2: None,
            native: None,
        }
    }

    struct Harness {
        origin: SrcVec3,
        velocity: SrcVec3,
        numeric: NumericOps,
        solid_all: bool,
        wall: bool,
        touches: usize,
        solids: usize,
    }

    impl Q2SweepBody for Harness {
        fn origin(&self) -> SrcVec3 {
            self.origin
        }
        fn velocity(&self) -> SrcVec3 {
            self.velocity
        }
        fn write_origin(&mut self, origin: SrcVec3) {
            self.origin = origin;
        }
        fn write_velocity(&mut self, velocity: SrcVec3) {
            self.velocity = velocity;
        }
        fn write_velocity_zero_z(&mut self) {
            self.velocity[2] = f64::from(self.numeric.store(0.0));
        }
        fn trace(&mut self, start: SrcVec3, end: SrcVec3) -> TraceT {
            if self.solid_all {
                return TraceT {
                    allsolid: true,
                    startsolid: true,
                    fraction: 0.0,
                    endpos: start,
                    plane: plane(),
                    surface: None,
                    contents: 1,
                    ent: None,
                    plane2: plane(),
                    surface2: None,
                    native: None,
                };
            }
            if self.wall && end[0] > 5.0 {
                let mut blocked = plane();
                blocked.normal = [-1.0, 0.0, 0.0];
                return TraceT {
                    allsolid: false,
                    startsolid: false,
                    fraction: 0.5,
                    endpos: [5.0, end[1] / 2.0, 0.0],
                    plane: blocked,
                    surface: None,
                    contents: 0,
                    ent: None,
                    plane2: plane(),
                    surface2: None,
                    native: None,
                };
            }
            open_trace(end)
        }
        fn clip(&mut self, velocity: SrcVec3, normal: SrcVec3) -> SrcVec3 {
            Q2Math::new(self.numeric, Q2MathEdition::Classic).slide_clip_velocity(velocity, normal, 1.01)
        }
        fn touch(&mut self, _trace: &TraceT) {
            self.touches += 1;
        }
        fn solid(&mut self, _trace: &TraceT) {
            self.solids += 1;
        }
    }

    fn harness() -> Harness {
        Harness {
            origin: [0.0, 0.0, 0.0],
            velocity: [100.0, 0.0, 0.0],
            numeric: NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            solid_all: false,
            wall: false,
            touches: 0,
            solids: 0,
        }
    }

    #[test]
    fn open_travel_advances_tuples() {
        let mut body = harness();
        let numeric = body.numeric;
        assert_eq!(sweep_q2_body(&mut body, numeric, 0.1), SweepStop::Complete);
        assert_eq!(body.origin, [10.0, 0.0, 0.0]);
    }

    #[test]
    fn all_solid_only_zeroes_vertical() {
        let mut body = harness();
        body.velocity = [100.0, 50.0, -25.0];
        body.solid_all = true;
        let numeric = body.numeric;
        assert_eq!(sweep_q2_body(&mut body, numeric, 0.1), SweepStop::Solid);
        assert_eq!(body.velocity, [100.0, 50.0, 0.0]);
        assert_eq!(body.solids, 1);
    }

    #[test]
    fn wall_clip_slides_sequentially() {
        let mut body = harness();
        body.velocity = [100.0, 50.0, 0.0];
        body.wall = true;
        let numeric = body.numeric;
        assert_eq!(sweep_q2_body(&mut body, numeric, 0.1), SweepStop::Complete);
        assert!(body.touches >= 1);
        // Q2 sweep clips use overbounce 1.01, so the wall push is -1.
        assert_eq!(body.velocity[0], -1.0);
        assert_eq!(body.velocity[1], 50.0);
    }
}
