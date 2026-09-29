//! Quake III slide movement (`PM_ClipVelocity`, `PM_SlideMove`).
//!
//! Donor provenance: `src/movement/q3/slide-move.ts` (ported from id
//! Software `code/game/bg_slidemove.c`).

use qa_core::identity::ActorId;
use qa_core::math::{
    Bounds, Vec3, add3, cross3, dot3, normalize3, normalize3_or_zero, scale3, vec3,
};

use super::super::swept_body::{
    PairedResponse, SweepStop, SweptBodyServices, SweptBodyState, sweep_body,
};
use super::super::{Q3_DUPLICATE_PLANE_DOT, Q3_ENTER_THRESHOLD, Q3_MIN_GROUND_NORMAL,
    Q3_STEP_HEIGHT, clip_velocity_q3};
use super::constants::entity_event;
use super::types::{Q3Motion, Q3Trace};

/// Contact, event, and diagnostics sink for a slide.
pub trait SlideSink {
    /// Observe a touch contact.
    fn touch(&mut self, trace: &Q3Trace);
    /// Emit a locomotion event.
    fn event(&mut self, event: i32, motion: &mut Q3Motion);
    /// Diagnostics output.
    fn debug(&mut self, _message: &str) {}
}

/// Slide-move context, mirroring donor `SlideMoveContext`.
pub struct SlideMoveContext<'a> {
    /// Motion work state.
    pub motion: &'a mut Q3Motion,
    /// Frame time in seconds.
    pub frame_time: f32,
    /// Trace bounds.
    pub bounds: Bounds,
    /// Trace mask.
    pub mask: i32,
    /// Trace callback.
    pub trace: &'a mut dyn FnMut(Vec3, Vec3, Bounds, ActorId, i32) -> Q3Trace,
    /// Ground normal, when walking.
    pub ground_normal: Option<Vec3>,
    /// Maximum impact speed.
    pub impact_speed: f32,
    /// Contact/event sink.
    pub sink: &'a mut dyn SlideSink,
}

/// Q3 clip velocity with the source default overbounce.
#[must_use]
pub fn clip_velocity(velocity: Vec3, normal: Vec3) -> Vec3 {
    clip_velocity_q3(velocity, normal, 1.001)
}

/// Q3 clip velocity with an explicit overbounce.
#[must_use]
pub fn clip_velocity_overbounce(velocity: Vec3, normal: Vec3, overbounce: f32) -> Vec3 {
    clip_velocity_q3(velocity, normal, overbounce)
}

struct SlideSweep<'a, 'b> {
    context: &'a mut SlideMoveContext<'b>,
    end_velocity: Vec3,
    contacts: usize,
}

impl SweptBodyServices for SlideSweep<'_, '_> {
    type Trace = Q3Trace;

    fn read(&mut self) -> Option<SweptBodyState> {
        Some(SweptBodyState {
            origin: self.context.motion.origin,
            velocity: self.context.motion.velocity,
        })
    }
    fn write_origin(&mut self, origin: Vec3) {
        self.context.motion.origin = origin;
    }
    fn write_velocity(&mut self, velocity: Vec3) {
        self.context.motion.velocity = velocity;
    }
    fn trace(&mut self, start: Vec3, end: Vec3) -> Q3Trace {
        let actor = self.context.motion.actor.clone();
        let bounds = self.context.bounds;
        let mask = self.context.mask;
        (self.context.trace)(start, end, bounds, actor, mask)
    }
    fn touch(&mut self, trace: &Q3Trace) {
        self.contacts += 1;
        self.context.sink.touch(trace);
    }
    fn normal(&mut self, trace: &Q3Trace) -> Vec3 {
        match trace.contact {
            super::super::types::TraceContact::Plane(plane) => plane.normal,
            super::super::types::TraceContact::None => {
                panic!("Movement impact trace requires a collision plane")
            }
        }
    }
    fn stop_when_still(&self) -> bool {
        false
    }
    fn paired_response(&self) -> Option<PairedResponse> {
        let mut seeds = Vec::new();
        if let Some(ground) = self.context.ground_normal {
            seeds.push(ground);
        }
        seeds.push(normalize3_or_zero(self.context.motion.velocity));
        Some(PairedResponse {
            seeds,
            enter_threshold: f64::from(Q3_ENTER_THRESHOLD),
        })
    }
    fn paired_end_velocity(&mut self) -> Vec3 {
        self.end_velocity
    }
    fn write_paired_end_velocity(&mut self, velocity: Vec3) {
        self.end_velocity = velocity;
    }
    fn normalize(&mut self, direction: Vec3) -> Vec3 {
        normalize3(direction)
    }
    fn impact_speed(&mut self, speed: f32) {
        self.context.impact_speed = self.context.impact_speed.max(speed);
    }
    fn duplicate_threshold(&self) -> Option<f64> {
        Some(f64::from(Q3_DUPLICATE_PLANE_DOT))
    }
    fn recover_duplicate(&mut self, normal: Vec3) {
        let velocity = self.context.motion.velocity;
        self.context.motion.velocity = add3(velocity, normal);
    }
    fn same_plane(&mut self, _first: Vec3, _second: Vec3) -> bool {
        false
    }
    fn advance(&mut self, origin: Vec3, time: f64, velocity: Vec3) -> Vec3 {
        add3(origin, scale3(velocity, time as f32))
    }
    fn remaining(&mut self, time: f64, fraction: f64) -> f64 {
        f64::from((time as f32) - (time as f32) * (fraction as f32))
    }
    fn clip(&mut self, velocity: Vec3, normal: Vec3) -> Vec3 {
        clip_velocity(velocity, normal)
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

/// Slide the motion work state; true reports contacts or an incomplete
/// sweep. Mirrors donor `slideMove`.
pub fn slide_move(context: &mut SlideMoveContext<'_>, gravity: bool) -> bool {
    let mut primal_velocity = context.motion.velocity;
    let mut end_velocity = context.motion.velocity;
    if gravity {
        end_velocity = vec3(
            context.motion.velocity.x,
            context.motion.velocity.y,
            context.motion.velocity.z - context.motion.gravity * context.frame_time,
        );
        context.motion.velocity = vec3(
            context.motion.velocity.x,
            context.motion.velocity.y,
            (context.motion.velocity.z + end_velocity.z) * 0.5,
        );
        primal_velocity = vec3(primal_velocity.x, primal_velocity.y, end_velocity.z);
        if let Some(ground) = context.ground_normal {
            context.motion.velocity = clip_velocity(context.motion.velocity, ground);
        }
    }
    let mut sweep = SlideSweep {
        context,
        end_velocity,
        contacts: 0,
    };
    let frame_time = sweep.context.frame_time;
    let stop = sweep_body(&mut sweep, f64::from(frame_time));
    if stop != SweepStop::Complete {
        return true;
    }
    let contacts = sweep.contacts;
    end_velocity = sweep.end_velocity;
    let context = sweep.context;
    if gravity {
        context.motion.velocity = end_velocity;
    }
    if context.motion.pm_time != 0 {
        context.motion.velocity = primal_velocity;
    }
    contacts != 0
}

/// Step-slide the motion work state. Mirrors donor `stepSlideMove`.
pub fn step_slide_move(context: &mut SlideMoveContext<'_>, gravity: bool) {
    let start_origin = context.motion.origin;
    let start_velocity = context.motion.velocity;
    if !slide_move(context, gravity) {
        return;
    }
    let down = vec3(start_origin.x, start_origin.y, start_origin.z - Q3_STEP_HEIGHT);
    let actor = context.motion.actor.clone();
    let bounds = context.bounds;
    let mask = context.mask;
    let ground = (context.trace)(start_origin, down, bounds, actor, mask);
    if context.motion.velocity.z > 0.0
        && (ground.fraction == 1.0
            || !matches!(ground.contact, super::super::types::TraceContact::Plane(_))
            || ground_normal_z(&ground) < Q3_MIN_GROUND_NORMAL)
    {
        return;
    }
    let up = vec3(start_origin.x, start_origin.y, start_origin.z + Q3_STEP_HEIGHT);
    let actor = context.motion.actor.clone();
    let bounds = context.bounds;
    let mask = context.mask;
    let raised = (context.trace)(start_origin, up, bounds, actor, mask);
    if raised.all_solid {
        context.sink.debug("bend can't step");
        return;
    }
    let step_size = raised.end.z - start_origin.z;
    context.motion.origin = raised.end;
    context.motion.velocity = start_velocity;
    slide_move(context, gravity);
    let origin = context.motion.origin;
    let dropped_end = vec3(origin.x, origin.y, origin.z - step_size);
    let actor = context.motion.actor.clone();
    let bounds = context.bounds;
    let mask = context.mask;
    let dropped = (context.trace)(origin, dropped_end, bounds, actor, mask);
    if !dropped.all_solid {
        context.motion.origin = dropped.end;
    }
    if dropped.fraction < 1.0 {
        if let super::super::types::TraceContact::Plane(plane) = dropped.contact {
            context.motion.velocity = clip_velocity(context.motion.velocity, plane.normal);
        }
    }
    let delta = context.motion.origin.z - start_origin.z;
    if delta > 2.0 {
        let event = if delta < 7.0 {
            entity_event::STEP_4
        } else if delta < 11.0 {
            entity_event::STEP_8
        } else if delta < 15.0 {
            entity_event::STEP_12
        } else {
            entity_event::STEP_16
        };
        context.sink.event(event, &mut *context.motion);
    }
    context.sink.debug("stepped");
}

fn ground_normal_z(trace: &Q3Trace) -> f32 {
    match trace.contact {
        super::super::types::TraceContact::Plane(plane) => plane.normal.z,
        super::super::types::TraceContact::None => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull::BspPlane;
    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::super::constants::move_type;
    use super::super::super::types::{TraceContact, TraceHit};

    fn motion() -> Q3Motion {
        let owner = IdentityOwner::create("q3-slide").unwrap();
        let id = owner.actor(1, 0);
        let _ = owner.owned_actor(&id, ProviderId::new("q3", "test")).unwrap();
        Q3Motion {
            command_time: 0,
            pm_type: move_type::NORMAL,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(100.0, 0.0, 0.0),
            gravity: 800.0,
            speed: 320.0,
            delta_angles: vec3(0.0, 0.0, 0.0),
            ground: TraceHit::None,
            movement_dir: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            e_flags: 0,
            viewangles: vec3(0.0, 0.0, 0.0),
            viewheight: 26.0,
            pmove_framecount: 0,
            event_sequence: 0,
            actor: id,
            health: 100.0,
            flight: false,
            invulnerable: false,
            product: super::super::types::Q3Product::BaseQ3,
        }
    }

    struct RecSink {
        touches: usize,
        events: Vec<i32>,
    }

    impl SlideSink for RecSink {
        fn touch(&mut self, _trace: &Q3Trace) {
            self.touches += 1;
        }
        fn event(&mut self, event: i32, _motion: &mut Q3Motion) {
            self.events.push(event);
        }
    }

    fn open_trace(end: Vec3) -> Q3Trace {
        Q3Trace {
            fraction: 1.0,
            end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            contents: 0,
            surface_flags: 0,
            source_plane: BspPlane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            },
        }
    }

    #[test]
    fn clip_matches_source_overbounce() {
        let clipped = clip_velocity(vec3(100.0, 0.0, -50.0), vec3(0.0, 0.0, 1.0));
        assert_eq!(clipped.x, 100.0);
        assert!(clipped.z.abs() < 0.1);
    }

    #[test]
    fn open_slide_reports_no_contacts() {
        let mut motion = motion();
        let mut trace =
            |_start: Vec3, end: Vec3, _bounds: Bounds, _actor: ActorId, _mask: i32| open_trace(end);
        let mut sink = RecSink {
            touches: 0,
            events: Vec::new(),
        };
        let mut context = SlideMoveContext {
            motion: &mut motion,
            frame_time: 0.05,
            bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            mask: 1,
            trace: &mut trace,
            ground_normal: None,
            impact_speed: 0.0,
            sink: &mut sink,
        };
        assert!(!slide_move(&mut context, false));
        assert_eq!(context.motion.origin, vec3(5.0, 0.0, 0.0));
    }

    #[test]
    fn gravity_slide_averages_velocity() {
        let mut motion = motion();
        motion.velocity = vec3(0.0, 0.0, 0.0);
        let mut trace =
            |_start: Vec3, end: Vec3, _bounds: Bounds, _actor: ActorId, _mask: i32| open_trace(end);
        let mut sink = RecSink {
            touches: 0,
            events: Vec::new(),
        };
        let mut context = SlideMoveContext {
            motion: &mut motion,
            frame_time: 0.05,
            bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            mask: 1,
            trace: &mut trace,
            ground_normal: None,
            impact_speed: 0.0,
            sink: &mut sink,
        };
        assert!(!slide_move(&mut context, true));
        assert_eq!(context.motion.velocity.z, -40.0);
    }

    #[test]
    fn wall_slide_reports_contacts() {
        let mut motion = motion();
        let mut trace = |start: Vec3, end: Vec3, _bounds: Bounds, _actor: ActorId, _mask: i32| {
            if end.x > 2.0 {
                Q3Trace {
                    fraction: 0.4,
                    end: vec3(2.0, end.y * 0.4, end.z * 0.4),
                    start_solid: false,
                    all_solid: false,
                    contact: TraceContact::Plane(qa_core::math::Plane {
                        normal: vec3(-1.0, 0.0, 0.0),
                        distance: 0.0,
                    }),
                    hit: TraceHit::World { model: 0 },
                    contents: 0,
                    surface_flags: 0,
                    source_plane: BspPlane {
                        normal: vec3(-1.0, 0.0, 0.0),
                        distance: 0.0,
                        plane_type: 0,
                        signbits: 0,
                    },
                }
            } else {
                let _ = start;
                open_trace(end)
            }
        };
        let mut sink = RecSink {
            touches: 0,
            events: Vec::new(),
        };
        let mut context = SlideMoveContext {
            motion: &mut motion,
            frame_time: 0.05,
            bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            mask: 1,
            trace: &mut trace,
            ground_normal: None,
            impact_speed: 0.0,
            sink: &mut sink,
        };
        assert!(slide_move(&mut context, false));
        assert!(sink.touches >= 1);
    }
}
