//! Quake II rerelease fly-move sliding.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q2-rerelease-slide.ts`
//! (`createQ2RereleaseFlyMove`; rerelease `g_phys.cpp` `SV_FlyMove`).
//! Movement clipping is shared with the existing `p_move.cpp` implementation
//! through [`create_rerelease_movement`](qa_world::movement::q2::rerelease::create_rerelease_movement).
//!
//! The donor factory returns a closure over one movement runner. The Rust
//! runner borrows its [`Q2RereleaseMovementContext`] mutably, so the port
//! holds the numeric profile in [`Q2RereleaseFlyMove`] and builds the runner
//! per call with the donor defaults (`flight = false`,
//! `speedMultiplier = 1`); the sweep owns progress internally, and the trace
//! adapter publishes the sweep entry state to nested body reads.

use std::cell::RefCell;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{vec3, Bounds, Plane, Vec3};
use qa_core::numeric::NumericOps;
use qa_world::body::BodyState;
use qa_world::movement::q2::rerelease::{create_rerelease_movement, Q2RereleaseMovementContext};
use qa_world::movement::q2::types::{plane, CPlane, CSurface, KexTouchList, Q2Surface, SrcVec3, TraceT};
use qa_world::movement::types::{TraceContact, TraceHit};
use thiserror::Error;

use super::physics::TraceResult;

/// Engine services backing one rerelease fly-move.
///
/// Mirrors the donor `Q2RereleaseFlyMoveServices` interface
/// (`q2-rerelease-slide.ts`); the `actors`/`bodies` members surface as the
/// operations the sweep calls.
pub trait RereleaseFlyMoveServices {
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Read a body record.
    fn read_body(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body record.
    fn write_body(&mut self, actor: &OwnedActor, state: BodyState);
    /// Trace with the caller's source clipmask and ignored mover.
    fn trace(&mut self, start: Vec3, end: Vec3, bounds: Bounds) -> TraceResult;
    /// Actor hit by a trace, if any.
    fn hit_actor(&self, trace: &TraceResult) -> Option<ActorId>;
    /// Dispatch a post-sweep impact.
    fn impact(&mut self, trace: &TraceResult);
    /// Read and clear only `FL_KILL_VELOCITY` on the live source actor.
    fn take_kill_velocity(&mut self) -> bool;
}

/// Rerelease sliding failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2SlideError {
    /// A trace callback returned non-Q2 trace fields.
    #[error("rerelease sliding requires Q2 trace fields")]
    NonQ2Trace,
    /// A recorded contact lost its source trace.
    #[error("rerelease contact lost its source trace")]
    LostSourceTrace,
}

/// Rerelease fly-move over one shared movement context.
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleaseFlyMove {
    numeric: NumericOps,
}

fn to_vec3(value: SrcVec3) -> Vec3 {
    vec3(value[0] as f32, value[1] as f32, value[2] as f32)
}

fn to_src(value: Vec3) -> SrcVec3 {
    [f64::from(value.x), f64::from(value.y), f64::from(value.z)]
}

fn to_cplane(value: &qa_world::movement::q2::types::Q2TracePlane) -> CPlane {
    CPlane {
        normal: to_src(value.normal),
        dist: value.dist,
        plane_type: value.plane_type,
        signbits: value.signbits,
    }
}

fn to_q2_plane(value: &CPlane) -> qa_world::movement::q2::types::Q2TracePlane {
    qa_world::movement::q2::types::Q2TracePlane {
        normal: to_vec3(value.normal),
        dist: value.dist,
        plane_type: value.plane_type,
        signbits: value.signbits,
    }
}

impl Q2RereleaseFlyMove {
    /// Bind the numeric profile once per physics owner.
    #[must_use]
    pub fn new(numeric: NumericOps) -> Self {
        Self { numeric }
    }

    /// Slide one actor for a frame.
    ///
    /// Impacts occur after all bumps, in first-contact order, with the final
    /// body already visible.
    pub fn run<H: RereleaseFlyMoveServices>(
        &self,
        context: &mut Q2RereleaseMovementContext,
        actor: &OwnedActor,
        elapsed: f64,
        host: &mut H,
    ) -> Result<(), Q2SlideError> {
        let id = actor.id().clone();
        let Some(body) = host.read_body(&id) else {
            return Ok(());
        };
        if !host.is_live(&id) {
            return Ok(());
        }
        let mut cleared = body.clone();
        cleared.ground = None;
        host.write_body(actor, cleared);
        let mut origin = to_src(body.origin);
        let mut velocity = to_src(body.velocity);
        let bounds_min = to_src(body.bounds.min);
        let bounds_max = to_src(body.bounds.max);
        // Sweep entry state published to nested body reads; the Rust sweep
        // owns progress internally and reports only start/end per trace.
        let published_origin = origin;
        let published_velocity = velocity;
        let mut touch = KexTouchList {
            num: 0,
            traces: Vec::new(),
        };
        let mut entities: Vec<TraceHit> = Vec::new();
        let error = RefCell::new(None);
        let mut runner = create_rerelease_movement(self.numeric, context, false, 1.0);
        {
            let mut trace_fn = |start: SrcVec3, mins: SrcVec3, maxs: SrcVec3, end: SrcVec3| -> TraceT {
                if let Some(current) = host.read_body(&id) {
                    let mut updated = current;
                    updated.origin = to_vec3(published_origin);
                    updated.velocity = to_vec3(published_velocity);
                    host.write_body(actor, updated);
                }
                let result = host.trace(
                    to_vec3(start),
                    to_vec3(end),
                    Bounds {
                        min: to_vec3(mins),
                        max: to_vec3(maxs),
                    },
                );
                let TraceResult::Q2(q2) = &result else {
                    *error.borrow_mut() = Some(Q2SlideError::NonQ2Trace);
                    return TraceT {
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
                    };
                };
                let ent = match &q2.hit {
                    TraceHit::None => None,
                    hit => {
                        if let Some(existing) = entities.iter().find(|candidate| *candidate == hit) {
                            Some(existing.clone())
                        } else {
                            entities.push(hit.clone());
                            Some(hit.clone())
                        }
                    }
                };
                TraceT {
                    allsolid: q2.all_solid,
                    startsolid: q2.start_solid,
                    fraction: q2.fraction,
                    endpos: to_src(q2.end),
                    plane: to_cplane(&q2.source_plane),
                    surface: q2.surface.as_ref().map(CSurface::from),
                    contents: q2.contents,
                    ent,
                    plane2: q2
                        .secondary
                        .as_ref()
                        .map(|(plane, _)| to_cplane(plane))
                        .unwrap_or_else(plane),
                    surface2: q2
                        .secondary
                        .as_ref()
                        .and_then(|(_, surface)| surface.as_ref().map(CSurface::from)),
                    native: Some(q2.clone()),
                }
            };
            runner.step_slide_move_generic(
                &mut origin,
                &mut velocity,
                f64::from(self.numeric.store(elapsed)),
                bounds_min,
                bounds_max,
                &mut touch,
                false,
                &mut trace_fn,
            );
        }
        if let Some(err) = error.into_inner() {
            return Err(err);
        }
        let Some(moved) = host.read_body(&id) else {
            return Ok(());
        };
        if !host.is_live(&id) {
            return Ok(());
        }
        let mut moved = moved;
        moved.origin = to_vec3(origin);
        moved.velocity = to_vec3(velocity);
        host.write_body(actor, moved);
        for contact in &touch.traces {
            if !host.is_live(&id) {
                return Ok(());
            }
            let Some(original) = contact.native.as_ref() else {
                return Err(Q2SlideError::LostSourceTrace);
            };
            let selected = to_q2_plane(&contact.plane);
            let mut rebuilt = original.clone();
            rebuilt.source_plane = selected;
            rebuilt.surface = contact.surface.as_ref().map(Q2Surface::from);
            rebuilt.contact = match &original.contact {
                TraceContact::None => TraceContact::None,
                TraceContact::Plane(_) => TraceContact::Plane(Plane {
                    normal: selected.normal,
                    distance: selected.dist as f32,
                }),
            };
            let rebuilt = TraceResult::Q2(rebuilt);
            let Some(other) = host.hit_actor(&rebuilt) else {
                continue;
            };
            if !host.is_live(&other) {
                continue;
            }
            let Some(current) = host.read_body(&id) else {
                return Ok(());
            };
            if selected.normal.z > self.numeric.store(0.7) {
                let mut grounded = current;
                grounded.ground = Some(other);
                host.write_body(actor, grounded);
            }
            host.impact(&rebuilt);
            if !host.is_live(&id) {
                return Ok(());
            }
            if host.take_kill_velocity() {
                let Some(after) = host.read_body(&id) else {
                    return Ok(());
                };
                let mut stopped = after;
                stopped.velocity = vec3(0.0, 0.0, 0.0);
                host.write_body(actor, stopped);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::numeric::Q2_DONOR_PROFILE;
    use qa_world::movement::q2::types::{Q2Trace, Q2TracePlane};

    use super::*;

    struct FakeHost {
        live: bool,
        body: Option<BodyState>,
        trace: TraceResult,
        hit: Option<ActorId>,
        impacts: Vec<TraceResult>,
        kill_velocity: bool,
    }

    impl FakeHost {
        fn body() -> BodyState {
            BodyState {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                velocity: vec3(100.0, 0.0, 0.0),
                bounds: Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 32.0),
                },
                ground: None,
            }
        }

        fn clear_trace(end: Vec3) -> TraceResult {
            TraceResult::Q2(Q2Trace {
                fraction: 1.0,
                end,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                contents: 0,
                surface: None,
                source_plane: Q2TracePlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    dist: 0.0,
                    plane_type: 0,
                    signbits: 0,
                },
                secondary: None,
            })
        }
    }

    impl RereleaseFlyMoveServices for FakeHost {
        fn is_live(&self, _actor: &ActorId) -> bool {
            self.live
        }
        fn read_body(&self, _actor: &ActorId) -> Option<BodyState> {
            self.body.clone()
        }
        fn write_body(&mut self, _actor: &OwnedActor, state: BodyState) {
            self.body = Some(state);
        }
        fn trace(&mut self, _start: Vec3, end: Vec3, _bounds: Bounds) -> TraceResult {
            match &self.trace {
                TraceResult::Q2(q2) if q2.fraction == 1.0 => Self::clear_trace(end),
                other => other.clone(),
            }
        }
        fn hit_actor(&self, _trace: &TraceResult) -> Option<ActorId> {
            self.hit.clone()
        }
        fn impact(&mut self, trace: &TraceResult) {
            self.impacts.push(trace.clone());
        }
        fn take_kill_velocity(&mut self) -> bool {
            std::mem::replace(&mut self.kill_velocity, false)
        }
    }

    fn setup() -> (
        IdentityOwner,
        OwnedActor,
        Q2RereleaseMovementContext,
        Q2RereleaseFlyMove,
    ) {
        let owner = IdentityOwner::create("slide").expect("owner");
        let id = owner.actor(1, 0);
        let actor = owner
            .owned_actor(&id, qa_core::identity::ProviderId::new("q2", "test"))
            .expect("owned");
        let numeric = NumericOps::select(Q2_DONOR_PROFILE).expect("numeric");
        (
            owner,
            actor,
            Q2RereleaseMovementContext::new(),
            Q2RereleaseFlyMove::new(numeric),
        )
    }

    #[test]
    fn clear_travel_moves_full_distance() {
        let (_owner, actor, mut context, fly) = setup();
        let mut host = FakeHost {
            live: true,
            body: Some(FakeHost::body()),
            trace: FakeHost::clear_trace(vec3(10.0, 0.0, 0.0)),
            hit: None,
            impacts: Vec::new(),
            kill_velocity: false,
        };
        fly.run(&mut context, &actor, 0.1, &mut host).expect("slide");
        let body = host.body.expect("body");
        assert!((f64::from(body.origin.x) - 10.0).abs() < 0.01);
        assert!(host.impacts.is_empty());
        assert!(body.ground.is_none());
    }

    #[test]
    fn missing_body_is_a_noop() {
        let (_owner, actor, mut context, fly) = setup();
        let mut host = FakeHost {
            live: true,
            body: None,
            trace: FakeHost::clear_trace(vec3(0.0, 0.0, 0.0)),
            hit: None,
            impacts: Vec::new(),
            kill_velocity: false,
        };
        fly.run(&mut context, &actor, 0.1, &mut host).expect("slide");
        assert!(host.body.is_none());
    }

    #[test]
    fn non_q2_trace_fails() {
        use qa_world::movement::q1::types::Q1Trace;
        let (_owner, actor, mut context, fly) = setup();
        let mut host = FakeHost {
            live: true,
            body: Some(FakeHost::body()),
            trace: TraceResult::Q1(Q1Trace {
                fraction: 1.0,
                end: vec3(0.0, 0.0, 0.0),
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                in_open: true,
                in_water: false,
                source_plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
                surface_flags: None,
            }),
            hit: None,
            impacts: Vec::new(),
            kill_velocity: false,
        };
        assert_eq!(
            fly.run(&mut context, &actor, 0.1, &mut host),
            Err(Q2SlideError::NonQ2Trace)
        );
    }

    #[test]
    fn floor_impact_grounds_and_kills_velocity() {
        let (owner, actor, mut context, fly) = setup();
        let world = owner.actor(0, 0);
        let mut host = FakeHost {
            live: true,
            body: Some(FakeHost::body()),
            trace: TraceResult::Q2(Q2Trace {
                fraction: 0.5,
                end: vec3(5.0, 0.0, 0.0),
                start_solid: false,
                all_solid: false,
                contact: TraceContact::Plane(Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                }),
                hit: TraceHit::World { model: 0 },
                contents: 1,
                surface: None,
                source_plane: Q2TracePlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    dist: 0.0,
                    plane_type: 2,
                    signbits: 0,
                },
                secondary: None,
            }),
            hit: Some(world.clone()),
            impacts: Vec::new(),
            kill_velocity: true,
        };
        fly.run(&mut context, &actor, 0.1, &mut host).expect("slide");
        assert_eq!(host.impacts.len(), 1);
        let body = host.body.expect("body");
        assert_eq!(body.ground, Some(world));
        assert_eq!(body.velocity, vec3(0.0, 0.0, 0.0));
    }
}
