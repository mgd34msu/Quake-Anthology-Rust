//! Quake II rerelease movement.
//!
//! Donor provenance: `src/movement/q2/rerelease.ts` (ported through
//! quake-2-re-ts and checked against rerelease `p_move.cpp`).

use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::numeric::{Arithmetic, NumericOps};

use super::super::swept_body::SweepStop;
use super::dimensions::{accept_body_bounds, character_height};
use super::math::{Q2Math, Q2MathEdition};
use super::swept::{sweep_q2_body, Q2SweepBody};
use super::types::{
    button, kex_pm_type, pm_flags, refdef_flags, search_initial_snap_position, water_level, CSurface, KexPmove,
    KexTouchList, PmConfig, SrcVec3, StuckResult, TraceT, AXES, CONTENTS_CURRENT_0, CONTENTS_CURRENT_180,
    CONTENTS_CURRENT_270, CONTENTS_CURRENT_90, CONTENTS_CURRENT_DOWN, CONTENTS_CURRENT_UP, CONTENTS_LAVA,
    CONTENTS_NONE, CONTENTS_NO_WATERJUMP, CONTENTS_SLIME, CONTENTS_SOLID, CONTENTS_WATER, MASK_CURRENT,
    MASK_PLAYERSOLID, MASK_SOLID, MASK_WATER, MAXTOUCH, PITCH, Q2_INITIAL_SNAP_OFFSETS, STEPSIZE, SURF_SLICK,
};
use super::view::rerelease_view_angles;

/// Four-argument source trace callback.
pub type TraceFn<'a> = dyn FnMut(SrcVec3, SrcVec3, SrcVec3, SrcVec3) -> TraceT + 'a;

/// The game DLL shares `pml` between all Pmove and server fly-move calls.
/// Prediction owns a separate context. Pmove resets it; server
/// duplicate-plane recovery does not.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseMovementContext {
    /// Shared pml origin.
    pub pml_origin: SrcVec3,
}

impl Q2RereleaseMovementContext {
    /// Create a zeroed context.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pml_origin: [0.0, 0.0, 0.0],
        }
    }

    /// Capture the shared origin as a scene vector.
    #[must_use]
    pub fn capture(&self) -> Vec3 {
        vec3(
            self.pml_origin[0] as f32,
            self.pml_origin[1] as f32,
            self.pml_origin[2] as f32,
        )
    }

    /// Restore the shared origin.
    pub fn restore(&mut self, origin: Vec3) {
        self.pml_origin[0] = f64::from(origin.x);
        self.pml_origin[1] = f64::from(origin.y);
        self.pml_origin[2] = f64::from(origin.z);
    }
}

impl Default for Q2RereleaseMovementContext {
    fn default() -> Self {
        Self::new()
    }
}

struct SideCheck {
    normal: SrcVec3,
    mins: SrcVec3,
    maxs: SrcVec3,
}

const SIDE_CHECKS: [SideCheck; 6] = [
    SideCheck {
        normal: [0.0, 0.0, 1.0],
        mins: [-1.0, -1.0, 0.0],
        maxs: [1.0, 1.0, 0.0],
    },
    SideCheck {
        normal: [0.0, 0.0, -1.0],
        mins: [-1.0, -1.0, 0.0],
        maxs: [1.0, 1.0, 0.0],
    },
    SideCheck {
        normal: [1.0, 0.0, 0.0],
        mins: [0.0, -1.0, -1.0],
        maxs: [0.0, 1.0, 1.0],
    },
    SideCheck {
        normal: [-1.0, 0.0, 0.0],
        mins: [0.0, -1.0, -1.0],
        maxs: [0.0, 1.0, 1.0],
    },
    SideCheck {
        normal: [0.0, 1.0, 0.0],
        mins: [-1.0, 0.0, -1.0],
        maxs: [1.0, 0.0, 1.0],
    },
    SideCheck {
        normal: [0.0, -1.0, 0.0],
        mins: [-1.0, 0.0, -1.0],
        maxs: [1.0, 0.0, 1.0],
    },
];

fn source_float_value(n: NumericOps, value: f64) -> f64 {
    if matches!(n.profile.arithmetic, Arithmetic::DonorBinary64(_)) {
        value
    } else {
        f64::from(n.store(value))
    }
}

/// Generic stuck-object fix over any trace callback.
pub fn fix_stuck_object(
    n: NumericOps,
    math: Q2Math,
    origin: &mut SrcVec3,
    own_mins: SrcVec3,
    own_maxs: SrcVec3,
    trace: &mut TraceFn<'_>,
) -> StuckResult {
    if !trace(*origin, own_mins, own_maxs, *origin).startsolid {
        return StuckResult::GoodPosition;
    }
    let mut good_positions: Vec<(f64, SrcVec3)> = Vec::new();
    for (sn, side) in SIDE_CHECKS.iter().enumerate() {
        let mut start = *origin;
        let mut mins = math.vec3(0.0, 0.0, 0.0);
        let mut maxs = math.vec3(0.0, 0.0, 0.0);
        for axis in AXES {
            if side.normal[axis] < 0.0 {
                start[axis] = f64::from(n.store(n.add(start[axis], own_mins[axis])));
            } else if side.normal[axis] > 0.0 {
                start[axis] = f64::from(n.store(n.add(start[axis], own_maxs[axis])));
            }
            if side.mins[axis] == -1.0 {
                mins[axis] = f64::from(n.store(own_mins[axis]));
            } else if side.mins[axis] == 1.0 {
                mins[axis] = f64::from(n.store(own_maxs[axis]));
            }
            if side.maxs[axis] == -1.0 {
                maxs[axis] = f64::from(n.store(own_mins[axis]));
            } else if side.maxs[axis] == 1.0 {
                maxs[axis] = f64::from(n.store(own_maxs[axis]));
            }
        }
        let mut tr = trace(start, mins, maxs, start);
        let mut needed_epsilon_fix: Option<usize> = None;
        let mut needed_epsilon_dir = 0.0;
        if tr.startsolid {
            for e in AXES {
                if side.normal[e] != 0.0 {
                    continue;
                }
                let mut ep_start = start;
                ep_start[e] = f64::from(n.store(n.add(ep_start[e], 1.0)));
                tr = trace(ep_start, mins, maxs, ep_start);
                if !tr.startsolid {
                    start = ep_start;
                    needed_epsilon_fix = Some(e);
                    needed_epsilon_dir = 1.0;
                    break;
                }
                ep_start[e] = f64::from(n.store(n.sub(ep_start[e], 2.0)));
                tr = trace(ep_start, mins, maxs, ep_start);
                if !tr.startsolid {
                    start = ep_start;
                    needed_epsilon_fix = Some(e);
                    needed_epsilon_dir = -1.0;
                    break;
                }
            }
        }
        if tr.startsolid {
            continue;
        }
        let mut opposite_start = *origin;
        let other_side = &SIDE_CHECKS[sn ^ 1];
        for axis in AXES {
            if other_side.normal[axis] < 0.0 {
                opposite_start[axis] = f64::from(n.store(n.add(opposite_start[axis], own_mins[axis])));
            } else if other_side.normal[axis] > 0.0 {
                opposite_start[axis] = f64::from(n.store(n.add(opposite_start[axis], own_maxs[axis])));
            }
        }
        if let Some(e) = needed_epsilon_fix {
            opposite_start[e] = f64::from(n.store(n.add(opposite_start[e], needed_epsilon_dir)));
        }
        tr = trace(start, mins, maxs, opposite_start);
        if tr.startsolid {
            continue;
        }
        let mut end = tr.endpos;
        for axis in AXES {
            end[axis] = f64::from(n.store(n.add(end[axis], n.mul(side.normal[axis], source_float_value(n, 0.125)))));
        }
        let delta = math.sub(end, opposite_start);
        let mut new_origin = math.add(*origin, delta);
        if let Some(e) = needed_epsilon_fix {
            new_origin[e] = f64::from(n.store(n.add(new_origin[e], needed_epsilon_dir)));
        }
        tr = trace(new_origin, own_mins, own_maxs, new_origin);
        if tr.startsolid {
            continue;
        }
        good_positions.push((math.length_squared(delta), new_origin));
    }
    if !good_positions.is_empty() {
        let count = good_positions.len();
        if count > 1 {
            // Source sorts every candidate but the last.
            let mut sortable = good_positions[..count - 1].to_vec();
            sortable.sort_by(|a, b| n.sub(a.0, b.0).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
            for (i, entry) in sortable.into_iter().enumerate() {
                good_positions[i] = entry;
            }
        }
        *origin = good_positions[0].1;
        return StuckResult::Fixed;
    }
    StuckResult::NoGoodPosition
}

/// Record a touch unless the list is full or the entity repeats.
pub fn record_touch(touch: &mut KexTouchList, trace: &TraceT) {
    if touch.num == MAXTOUCH {
        return;
    }
    for i in 0..touch.num {
        if touch.traces.get(i).is_some_and(|prior| prior.ent == trace.ent) {
            return;
        }
    }
    if touch.num >= touch.traces.len() {
        touch.traces.resize(touch.num + 1, trace.clone());
    }
    touch.traces[touch.num] = trace.clone();
    touch.num += 1;
}

struct RereleaseSweepBody<'a> {
    origin: SrcVec3,
    velocity: SrcVec3,
    mins: SrcVec3,
    maxs: SrcVec3,
    trace: &'a mut TraceFn<'a>,
    touch: &'a mut KexTouchList,
    recover_target: Option<SrcVec3>,
    working_is_pml: bool,
    n: NumericOps,
    math: Q2Math,
    overbounce: f64,
    duplicate_threshold: f64,
    fix_nudge: f64,
}

impl Q2SweepBody for RereleaseSweepBody<'_> {
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
        self.velocity[2] = f64::from(self.n.store(0.0));
    }
    fn trace(&mut self, start: SrcVec3, end: SrcVec3) -> TraceT {
        (self.trace)(start, self.mins, self.maxs, end)
    }
    fn clip(&mut self, velocity: SrcVec3, normal: SrcVec3) -> SrcVec3 {
        self.math.slide_clip_velocity(velocity, normal, self.overbounce)
    }
    fn touch(&mut self, trace: &TraceT) {
        record_touch(self.touch, trace);
    }
    fn prepare_trace(&mut self, trace: &mut TraceT) {
        if trace.surface2.is_some() {
            let clipped_a = self
                .math
                .slide_clip_velocity(self.velocity, trace.plane.normal, self.overbounce);
            let clipped_b = self
                .math
                .slide_clip_velocity(self.velocity, trace.plane2.normal, self.overbounce);
            if AXES.iter().any(|i| clipped_a[*i].abs() < clipped_b[*i].abs()) {
                trace.plane = trace.plane2.clone();
                trace.surface = trace.surface2.clone();
            }
        }
    }
    fn solid(&mut self, trace: &TraceT) {
        record_touch(self.touch, trace);
    }
    fn duplicate_threshold(&self) -> Option<f64> {
        Some(self.duplicate_threshold)
    }
    fn recover_duplicate(&mut self, normal: SrcVec3) {
        // Donor `duplicatePlane.recover`: nudge x/y by the plane normal,
        // fix stuck in place, and let the sweep continue from there.
        if self.working_is_pml {
            // Player move: the working origin aliases pml; fix it in place.
            for axis in AXES[..2].iter() {
                self.origin[*axis] = f64::from(
                    self.n.store(
                        self.n
                            .add(self.origin[*axis], self.n.mul(normal[*axis], self.fix_nudge)),
                    ),
                );
            }
            let mut fixed = self.origin;
            fix_stuck_object(self.n, self.math, &mut fixed, self.mins, self.maxs, self.trace);
            self.origin = fixed;
            return;
        }
        // Temporary probe: recovery addresses pml, which the post-sweep
        // restore carries back (donor keeps the same source alias).
        let Some(target) = self.recover_target.as_mut() else {
            return;
        };
        for axis in AXES[..2].iter() {
            target[*axis] = f64::from(
                self.n
                    .store(self.n.add(target[*axis], self.n.mul(normal[*axis], self.fix_nudge))),
            );
        }
        let mut fixed = *target;
        fix_stuck_object(self.n, self.math, &mut fixed, self.mins, self.maxs, self.trace);
        *target = fixed;
    }
}

/// Rerelease movement runner: tuning plus the shared pml context.
pub struct RereleaseRunner<'c> {
    n: NumericOps,
    math: Q2Math,
    context: &'c mut Q2RereleaseMovementContext,
    flight: bool,
    speed_multiplier: f64,
    pm_stopspeed: f64,
    pm_maxspeed: f64,
    pm_duckspeed: f64,
    pm_accelerate: f64,
    pm_wateraccelerate: f64,
    pm_friction: f64,
    pm_waterfriction: f64,
    pm_waterspeed: f64,
    pm_laddermod: f64,
    min_step_normal: f64,
}

/// Build rerelease movement over a shared context.
pub fn create_rerelease_movement(
    numeric_ops: NumericOps,
    context: &mut Q2RereleaseMovementContext,
    flight: bool,
    speed_multiplier: f64,
) -> RereleaseRunner<'_> {
    let math = Q2Math::new(numeric_ops, Q2MathEdition::Rerelease);
    let equipment = |value: f64| {
        if speed_multiplier == 1.0 {
            value
        } else {
            numeric_ops.mul(value, speed_multiplier)
        }
    };
    RereleaseRunner {
        n: numeric_ops,
        math,
        context,
        flight,
        speed_multiplier,
        pm_stopspeed: 100.0,
        pm_maxspeed: equipment(300.0),
        pm_duckspeed: equipment(100.0),
        pm_accelerate: 10.0,
        pm_wateraccelerate: 10.0,
        pm_friction: 6.0,
        pm_waterfriction: 1.0,
        pm_waterspeed: 400.0,
        pm_laddermod: source_float_value(numeric_ops, 0.5),
        min_step_normal: source_float_value(numeric_ops, 0.7),
    }
}

impl RereleaseRunner<'_> {
    fn source_float(&self, value: f64) -> f64 {
        source_float_value(self.n, value)
    }

    fn equipment_speed(&self, value: f64) -> f64 {
        if self.speed_multiplier == 1.0 {
            value
        } else {
            self.n.mul(value, self.speed_multiplier)
        }
    }

    /// Generic step-slide move over any trace callback. Native
    /// duplicate-plane recovery addresses the shared pml origin even when
    /// the swept origin is a server entity or a temporary probe.
    #[allow(clippy::too_many_arguments)]
    pub fn step_slide_move_generic(
        &mut self,
        origin: &mut SrcVec3,
        velocity: &mut SrcVec3,
        frametime: f64,
        mins: SrcVec3,
        maxs: SrcVec3,
        touch: &mut KexTouchList,
        has_time: bool,
        trace: &mut TraceFn<'_>,
    ) {
        let primal = *velocity;
        let mut body = RereleaseSweepBody {
            origin: *origin,
            velocity: *velocity,
            mins,
            maxs,
            trace,
            touch,
            recover_target: Some(self.context.pml_origin),
            working_is_pml: false,
            n: self.n,
            math: self.math,
            overbounce: self.source_float(1.01),
            duplicate_threshold: self.source_float(0.99),
            fix_nudge: self.source_float(0.01),
        };
        let stop = sweep_q2_body(&mut body, self.n, frametime);
        *origin = body.origin;
        *velocity = body.velocity;
        if let Some(fixed) = body.recover_target {
            self.context.pml_origin = fixed;
        }
        if stop != SweepStop::Solid && has_time {
            *velocity = primal;
        }
    }

    /// Run rerelease pmove over one command.
    pub fn pmove(&mut self, pm: &mut KexPmove<'_>, config: &PmConfig) {
        RereleasePmove::run(self, pm, config);
    }
}

struct RereleasePml {
    velocity: SrcVec3,
    forward: SrcVec3,
    right: SrcVec3,
    up: SrcVec3,
    frametime: f64,
    groundsurface: Option<CSurface>,
    groundcontents: i32,
    previous_origin: SrcVec3,
    start_velocity: SrcVec3,
}

struct RereleasePmove<'r, 'c, 'p, 'cb> {
    runner: &'r mut RereleaseRunner<'c>,
    pm: &'p mut KexPmove<'cb>,
    pml: RereleasePml,
    config: PmConfig,
    accepted_body_bounds: Option<Bounds>,
    accepted_duck: i32,
    accepted_height: f64,
}

impl<'r, 'c, 'p, 'cb> RereleasePmove<'r, 'c, 'p, 'cb> {
    fn run(runner: &'r mut RereleaseRunner<'c>, pm: &'p mut KexPmove<'cb>, config: &PmConfig) {
        let accepted_body_bounds = pm.previous_bounds;
        let accepted_duck = pm.s.pm_flags & pm_flags::DUCKED;
        let accepted_height = pm.s.viewheight;
        let mut mover = Self {
            runner,
            pm,
            pml: RereleasePml {
                velocity: [0.0, 0.0, 0.0],
                forward: [0.0, 0.0, 0.0],
                right: [0.0, 0.0, 0.0],
                up: [0.0, 0.0, 0.0],
                frametime: 0.0,
                groundsurface: None,
                groundcontents: CONTENTS_NONE,
                previous_origin: [0.0, 0.0, 0.0],
                start_velocity: [0.0, 0.0, 0.0],
            },
            config: *config,
            accepted_body_bounds,
            accepted_duck,
            accepted_height,
        };
        mover.pmove_main();
    }

    fn origin(&self) -> SrcVec3 {
        self.runner.context.pml_origin
    }

    fn set_origin(&mut self, origin: SrcVec3) {
        self.runner.context.pml_origin = origin;
    }

    fn equipment_speed(&self, value: f64) -> f64 {
        self.runner.equipment_speed(value)
    }

    fn source_float(&self, value: f64) -> f64 {
        self.runner.source_float(value)
    }

    fn pm_trace_auto(&mut self, start: SrcVec3, mins: SrcVec3, maxs: SrcVec3, end: SrcVec3) -> TraceT {
        if self.pm.s.pm_type == kex_pm_type::SPECTATOR {
            return (self.pm.clip)(start, mins, maxs, end, MASK_SOLID);
        }
        let mut mask = if self.pm.s.pm_type == kex_pm_type::DEAD || self.pm.s.pm_type == kex_pm_type::GIB {
            super::types::MASK_DEADSOLID
        } else {
            MASK_PLAYERSOLID
        };
        if self.pm.s.pm_flags & pm_flags::IGNORE_PLAYER_COLLISION != 0 {
            mask &= !super::types::CONTENTS_PLAYER;
        }
        let player = self.pm.player.clone();
        (self.pm.trace)(start, mins, maxs, end, player, mask)
    }

    fn pm_trace_masked(&mut self, start: SrcVec3, mins: SrcVec3, maxs: SrcVec3, end: SrcVec3, mask: i32) -> TraceT {
        if self.pm.s.pm_type == kex_pm_type::SPECTATOR {
            return (self.pm.clip)(start, mins, maxs, end, MASK_SOLID);
        }
        let player = self.pm.player.clone();
        (self.pm.trace)(start, mins, maxs, end, player, mask)
    }

    /// Main sweep over the shared pml origin with the pm touch list.
    fn sweep_main(&mut self) {
        let Self { runner, pm, pml, .. } = self;
        let primal = pml.velocity;
        let frametime = pml.frametime;
        let mins = pm.mins;
        let maxs = pm.maxs;
        let has_time = pm.s.pm_time != 0;
        let pm_type = pm.s.pm_type;
        let pm_flags_value = pm.s.pm_flags;
        let player = pm.player.clone();
        let trace_cb = &mut pm.trace;
        let clip_cb = &mut pm.clip;
        let mut auto = |start: SrcVec3, mins: SrcVec3, maxs: SrcVec3, end: SrcVec3| -> TraceT {
            if pm_type == kex_pm_type::SPECTATOR {
                return clip_cb(start, mins, maxs, end, MASK_SOLID);
            }
            let mut mask = if pm_type == kex_pm_type::DEAD || pm_type == kex_pm_type::GIB {
                super::types::MASK_DEADSOLID
            } else {
                MASK_PLAYERSOLID
            };
            if pm_flags_value & pm_flags::IGNORE_PLAYER_COLLISION != 0 {
                mask &= !super::types::CONTENTS_PLAYER;
            }
            trace_cb(start, mins, maxs, end, player.clone(), mask)
        };
        let touch = &mut pm.touch;
        let n = runner.n;
        let math = runner.math;
        let overbounce = runner.source_float(1.01);
        let duplicate_threshold = runner.source_float(0.99);
        let fix_nudge = runner.source_float(0.01);
        let mut body = RereleaseSweepBody {
            origin: runner.context.pml_origin,
            velocity: pml.velocity,
            mins,
            maxs,
            trace: &mut auto,
            touch,
            // The main sweep has no separate recovery target: duplicate-
            // plane fixes land on the working origin and the sweep
            // continues from there (donor `PM_StepSlideMove_Generic`).
            recover_target: None,
            working_is_pml: true,
            n,
            math,
            overbounce,
            duplicate_threshold,
            fix_nudge,
        };
        let stop = sweep_q2_body(&mut body, n, frametime);
        runner.context.pml_origin = body.origin;
        let mut velocity = body.velocity;
        if stop != SweepStop::Solid && has_time {
            velocity = primal;
        }
        pml.velocity = velocity;
    }

    /// Temporary sweep (water-jump probe): recovery still addresses pml.
    #[allow(clippy::too_many_arguments)]
    fn sweep_temp(
        &mut self,
        origin: &mut SrcVec3,
        velocity: &mut SrcVec3,
        frametime: f64,
        mins: SrcVec3,
        maxs: SrcVec3,
        touch: &mut KexTouchList,
        has_time: bool,
    ) {
        let Self { runner, pm, .. } = self;
        let primal = *velocity;
        let pm_type = pm.s.pm_type;
        let pm_flags_value = pm.s.pm_flags;
        let player = pm.player.clone();
        let trace_cb = &mut pm.trace;
        let clip_cb = &mut pm.clip;
        let mut auto = |start: SrcVec3, mins: SrcVec3, maxs: SrcVec3, end: SrcVec3| -> TraceT {
            if pm_type == kex_pm_type::SPECTATOR {
                return clip_cb(start, mins, maxs, end, MASK_SOLID);
            }
            let mut mask = if pm_type == kex_pm_type::DEAD || pm_type == kex_pm_type::GIB {
                super::types::MASK_DEADSOLID
            } else {
                MASK_PLAYERSOLID
            };
            if pm_flags_value & pm_flags::IGNORE_PLAYER_COLLISION != 0 {
                mask &= !super::types::CONTENTS_PLAYER;
            }
            trace_cb(start, mins, maxs, end, player.clone(), mask)
        };
        let n = runner.n;
        let math = runner.math;
        let overbounce = runner.source_float(1.01);
        let duplicate_threshold = runner.source_float(0.99);
        let fix_nudge = runner.source_float(0.01);
        let mut body = RereleaseSweepBody {
            origin: *origin,
            velocity: *velocity,
            mins,
            maxs,
            trace: &mut auto,
            touch,
            recover_target: Some(runner.context.pml_origin),
            working_is_pml: false,
            n,
            math,
            overbounce,
            duplicate_threshold,
            fix_nudge,
        };
        let stop = sweep_q2_body(&mut body, n, frametime);
        *origin = body.origin;
        *velocity = body.velocity;
        if let Some(fixed) = body.recover_target {
            runner.context.pml_origin = fixed;
        }
        if stop != SweepStop::Solid && has_time {
            *velocity = primal;
        }
    }

    fn step_slide_move(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        let min_step_normal = self.runner.min_step_normal;
        let start_o = self.origin();
        let start_v = self.pml.velocity;
        self.sweep_main();
        let down_o = self.origin();
        let down_v = self.pml.velocity;
        let up = math.add(start_o, [0.0, 0.0, f64::from(STEPSIZE)]);
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        let mut trace = self.pm_trace_auto(start_o, mins, maxs, up);
        if trace.allsolid {
            return;
        }
        let step_size = n.sub(trace.endpos[2], start_o[2]);
        let endpos = trace.endpos;
        self.set_origin(endpos);
        self.pml.velocity = start_v;
        self.sweep_main();
        let origin = self.origin();
        let mut down = math.sub(origin, [0.0, 0.0, step_size]);
        let original_down = down;
        if start_o[2] < down[2] {
            down[2] = f64::from(n.store(n.sub(start_o[2], 1.0)));
        }
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        trace = self.pm_trace_auto(origin, mins, maxs, down);
        if !trace.allsolid {
            let mins = self.pm.mins;
            let maxs = self.pm.maxs;
            let real = self.pm_trace_auto(origin, mins, maxs, original_down);
            self.set_origin(real.endpos);
            if self.pml.velocity[2] > 0.0 {
                self.pm.step_clip = true;
            }
        }
        let up_end = self.origin();
        let down_dist = n.add(
            n.mul(n.sub(down_o[0], start_o[0]), n.sub(down_o[0], start_o[0])),
            n.mul(n.sub(down_o[1], start_o[1]), n.sub(down_o[1], start_o[1])),
        );
        let up_dist = n.add(
            n.mul(n.sub(up_end[0], start_o[0]), n.sub(up_end[0], start_o[0])),
            n.mul(n.sub(up_end[1], start_o[1]), n.sub(up_end[1], start_o[1])),
        );
        if down_dist > up_dist || trace.plane.normal[2] < min_step_normal {
            self.set_origin(down_o);
            self.pml.velocity = down_v;
        } else {
            self.pml.velocity[2] = f64::from(n.store(down_v[2]));
        }
        if self.pm.s.pm_flags & pm_flags::ON_GROUND != 0
            && self.pm.s.pm_flags & pm_flags::ON_LADDER == 0
            && (self.pm.waterlevel < water_level::WAIST
                || (self.pm.cmd.buttons & button::JUMP == 0 && self.pml.velocity[2] <= 0.0))
        {
            let origin = self.origin();
            let stairs = math.sub(origin, [0.0, 0.0, f64::from(STEPSIZE)]);
            let mins = self.pm.mins;
            let maxs = self.pm.maxs;
            let trace = self.pm_trace_auto(origin, mins, maxs, stairs);
            if trace.fraction < 1.0 {
                self.set_origin(trace.endpos);
            }
        }
    }

    fn friction(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        let stopspeed = self.runner.pm_stopspeed;
        let friction = self.runner.pm_friction;
        let waterfriction = self.runner.pm_waterfriction;
        let vel = self.pml.velocity;
        let speed = n.sqrt(n.add(
            n.add(n.mul(vel[0], vel[0]), n.mul(vel[1], vel[1])),
            n.mul(vel[2], vel[2]),
        ));
        if speed < 1.0 {
            self.pml.velocity[0] = f64::from(n.store(0.0));
            self.pml.velocity[1] = f64::from(n.store(0.0));
            return;
        }
        let mut drop = 0.0;
        let slick = self
            .pml
            .groundsurface
            .as_ref()
            .is_some_and(|surface| surface.flags & SURF_SLICK == 0);
        if (self.pm.groundentity.is_some() && slick) || self.pm.s.pm_flags & pm_flags::ON_LADDER != 0 {
            let control = if speed < stopspeed { stopspeed } else { speed };
            drop = n.add(drop, n.mul(n.mul(control, friction), self.pml.frametime));
        }
        if self.pm.waterlevel != 0 && self.pm.s.pm_flags & pm_flags::ON_LADDER == 0 {
            drop = n.add(
                drop,
                n.mul(
                    n.mul(n.mul(speed, waterfriction), self.pm.waterlevel as f64),
                    self.pml.frametime,
                ),
            );
        }
        let mut newspeed = n.sub(speed, drop);
        if newspeed < 0.0 {
            newspeed = 0.0;
        }
        newspeed = n.div(newspeed, speed);
        let scaled = math.muls(vel, newspeed);
        self.pml.velocity = [scaled[0], scaled[1], scaled[2]];
    }

    fn accelerate(&mut self, wishdir: SrcVec3, wishspeed: f64, accel: f64) {
        let n = self.runner.n;
        let math = self.runner.math;
        let currentspeed = math.dot(self.pml.velocity, wishdir);
        let addspeed = n.sub(wishspeed, currentspeed);
        if addspeed <= 0.0 {
            return;
        }
        let mut accelspeed = n.mul(n.mul(accel, self.pml.frametime), wishspeed);
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }
        for i in AXES {
            self.pml.velocity[i] = f64::from(n.store(n.add(self.pml.velocity[i], n.mul(accelspeed, wishdir[i]))));
        }
    }

    fn air_accelerate(&mut self, wishdir: SrcVec3, wishspeed: f64, accel: f64) {
        let n = self.runner.n;
        let math = self.runner.math;
        let frametime = self.pml.frametime;
        super::math::q2_air_accelerate(n, math, &mut self.pml.velocity, wishdir, wishspeed, accel, frametime);
    }

    fn add_currents(&mut self, wishvel: &mut SrcVec3) {
        let n = self.runner.n;
        let math = self.runner.math;
        let maxspeed = self.runner.pm_maxspeed;
        let waterspeed = self.runner.pm_waterspeed;
        let laddermod = self.runner.pm_laddermod;
        if self.pm.s.pm_flags & pm_flags::ON_LADDER != 0 {
            if self.pm.cmd.buttons & (button::JUMP | button::CROUCH) != 0 {
                let ladder_speed = if self.pm.waterlevel >= water_level::WAIST {
                    maxspeed
                } else {
                    200.0
                };
                if self.pm.cmd.buttons & button::JUMP != 0 {
                    wishvel[2] = f64::from(n.store(ladder_speed));
                } else if self.pm.cmd.buttons & button::CROUCH != 0 {
                    wishvel[2] = f64::from(n.store(-ladder_speed));
                }
            } else if self.pm.cmd.forwardmove != 0.0 {
                let ladder_speed = math.clamp(self.pm.cmd.forwardmove, -200.0, 200.0);
                if self.pm.cmd.forwardmove > 0.0 {
                    if self.pm.viewangles[PITCH] < 15.0 {
                        wishvel[2] = f64::from(n.store(ladder_speed));
                    } else {
                        wishvel[2] = f64::from(n.store(-ladder_speed));
                    }
                } else if self.pm.cmd.forwardmove < 0.0 {
                    if self.pm.groundentity.is_none() {
                        wishvel[0] = f64::from(n.store(0.0));
                        wishvel[1] = f64::from(n.store(0.0));
                    }
                    wishvel[2] = f64::from(n.store(ladder_speed));
                }
            } else {
                wishvel[2] = f64::from(n.store(0.0));
            }
            if self.pm.groundentity.is_none() {
                if self.pm.cmd.sidemove != 0.0 {
                    let mut ladder_speed = math.clamp(self.pm.cmd.sidemove, -150.0, 150.0);
                    if self.pm.waterlevel < water_level::WAIST {
                        ladder_speed = n.mul(ladder_speed, laddermod);
                    }
                    let mut flatforward = [self.pml.forward[0], self.pml.forward[1], 0.0];
                    math.normalize(&mut flatforward);
                    let spot = math.add(self.origin(), math.muls(flatforward, 1.0));
                    let mins = self.pm.mins;
                    let maxs = self.pm.maxs;
                    let origin = self.origin();
                    let trace = self.pm_trace_masked(origin, mins, maxs, spot, super::types::CONTENTS_LADDER);
                    if trace.fraction != 1.0 && trace.contents & super::types::CONTENTS_LADDER != 0 {
                        let right = math.cross(trace.plane.normal, [0.0, 0.0, 1.0]);
                        wishvel[0] = f64::from(n.store(0.0));
                        wishvel[1] = f64::from(n.store(0.0));
                        let push = math.muls(right, -ladder_speed);
                        let mut out = *wishvel;
                        math.add_eq(&mut out, push);
                        *wishvel = out;
                    }
                } else {
                    wishvel[0] = f64::from(n.store(wishvel[0].clamp(-25.0, 25.0)));
                    wishvel[1] = f64::from(n.store(wishvel[1].clamp(-25.0, 25.0)));
                }
            }
        }
        if self.pm.watertype & MASK_CURRENT != 0 {
            let mut v = math.vec3(0.0, 0.0, 0.0);
            if self.pm.watertype & CONTENTS_CURRENT_0 != 0 {
                v[0] = f64::from(n.store(n.add(v[0], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_90 != 0 {
                v[1] = f64::from(n.store(n.add(v[1], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_180 != 0 {
                v[0] = f64::from(n.store(n.sub(v[0], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_270 != 0 {
                v[1] = f64::from(n.store(n.sub(v[1], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_UP != 0 {
                v[2] = f64::from(n.store(n.add(v[2], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_DOWN != 0 {
                v[2] = f64::from(n.store(n.sub(v[2], 1.0)));
            }
            let mut s = waterspeed;
            if self.pm.waterlevel == water_level::FEET && self.pm.groundentity.is_some() {
                s = n.div(s, 2.0);
            }
            let push = math.muls(v, s);
            let mut out = *wishvel;
            math.add_eq(&mut out, push);
            *wishvel = out;
        }
        if self.pm.groundentity.is_some() {
            let mut v = math.vec3(0.0, 0.0, 0.0);
            if self.pml.groundcontents & CONTENTS_CURRENT_0 != 0 {
                v[0] = f64::from(n.store(n.add(v[0], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_90 != 0 {
                v[1] = f64::from(n.store(n.add(v[1], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_180 != 0 {
                v[0] = f64::from(n.store(n.sub(v[0], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_270 != 0 {
                v[1] = f64::from(n.store(n.sub(v[1], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_UP != 0 {
                v[2] = f64::from(n.store(n.add(v[2], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_DOWN != 0 {
                v[2] = f64::from(n.store(n.sub(v[2], 1.0)));
            }
            let push = math.muls(v, 100.0);
            let mut out = *wishvel;
            math.add_eq(&mut out, push);
            *wishvel = out;
        }
    }

    fn water_move(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        let maxspeed = self.runner.pm_maxspeed;
        let duckspeed = self.runner.pm_duckspeed;
        let waterspeed = self.runner.pm_waterspeed;
        let wateraccelerate = self.runner.pm_wateraccelerate;
        let fmove = self.equipment_speed(self.pm.cmd.forwardmove);
        let smove = self.equipment_speed(self.pm.cmd.sidemove);
        let mut wishvel = math.vec3(
            n.add(n.mul(self.pml.forward[0], fmove), n.mul(self.pml.right[0], smove)),
            n.add(n.mul(self.pml.forward[1], fmove), n.mul(self.pml.right[1], smove)),
            n.add(n.mul(self.pml.forward[2], fmove), n.mul(self.pml.right[2], smove)),
        );
        if self.pm.cmd.forwardmove == 0.0
            && self.pm.cmd.sidemove == 0.0
            && self.pm.cmd.buttons & (button::JUMP | button::CROUCH) == 0
        {
            if self.pm.groundentity.is_none() {
                wishvel[2] = f64::from(n.store(n.sub(wishvel[2], 60.0)));
            }
        } else if self.pm.cmd.buttons & button::CROUCH != 0 {
            wishvel[2] = f64::from(n.store(n.sub(
                wishvel[2],
                self.equipment_speed(n.mul(waterspeed, self.source_float(0.5))),
            )));
        } else if self.pm.cmd.buttons & button::JUMP != 0 {
            wishvel[2] = f64::from(n.store(n.add(
                wishvel[2],
                self.equipment_speed(n.mul(waterspeed, self.source_float(0.5))),
            )));
        }
        self.add_currents(&mut wishvel);
        let mut wishdir = wishvel;
        let mut wishspeed = math.normalize(&mut wishdir);
        if wishspeed > maxspeed {
            math.mul_eq(&mut wishvel, n.div(maxspeed, wishspeed));
            wishspeed = maxspeed;
        }
        wishspeed = n.mul(wishspeed, self.source_float(0.5));
        if self.pm.s.pm_flags & pm_flags::DUCKED != 0 && wishspeed > duckspeed {
            math.mul_eq(&mut wishvel, n.div(duckspeed, wishspeed));
            wishspeed = duckspeed;
        }
        self.accelerate(wishdir, wishspeed, wateraccelerate);
        self.step_slide_move();
    }

    fn air_move(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        let maxspeed = self.runner.pm_maxspeed;
        let duckspeed = self.runner.pm_duckspeed;
        let accelerate = self.runner.pm_accelerate;
        let fmove = self.equipment_speed(self.pm.cmd.forwardmove);
        let smove = self.equipment_speed(self.pm.cmd.sidemove);
        let mut wishvel = math.vec3(
            n.add(n.mul(self.pml.forward[0], fmove), n.mul(self.pml.right[0], smove)),
            n.add(n.mul(self.pml.forward[1], fmove), n.mul(self.pml.right[1], smove)),
            0.0,
        );
        self.add_currents(&mut wishvel);
        let mut wishdir = wishvel;
        let mut wishspeed = math.normalize(&mut wishdir);
        let max = if self.pm.s.pm_flags & pm_flags::DUCKED != 0 {
            duckspeed
        } else {
            maxspeed
        };
        if wishspeed > max {
            math.mul_eq(&mut wishvel, n.div(max, wishspeed));
            wishspeed = max;
        }
        if self.pm.s.pm_flags & pm_flags::ON_LADDER != 0 {
            self.accelerate(wishdir, wishspeed, accelerate);
            if wishvel[2] == 0.0 {
                if self.pml.velocity[2] > 0.0 {
                    self.pml.velocity[2] =
                        f64::from(n.store(n.sub(self.pml.velocity[2], n.mul(self.pm.s.gravity, self.pml.frametime))));
                    if self.pml.velocity[2] < 0.0 {
                        self.pml.velocity[2] = f64::from(n.store(0.0));
                    }
                } else {
                    self.pml.velocity[2] =
                        f64::from(n.store(n.add(self.pml.velocity[2], n.mul(self.pm.s.gravity, self.pml.frametime))));
                    if self.pml.velocity[2] > 0.0 {
                        self.pml.velocity[2] = f64::from(n.store(0.0));
                    }
                }
            }
            self.step_slide_move();
        } else if self.pm.groundentity.is_some() {
            self.pml.velocity[2] = f64::from(n.store(0.0));
            self.accelerate(wishdir, wishspeed, accelerate);
            if self.pm.s.gravity > 0.0 {
                self.pml.velocity[2] = f64::from(n.store(0.0));
            } else {
                self.pml.velocity[2] =
                    f64::from(n.store(n.sub(self.pml.velocity[2], n.mul(self.pm.s.gravity, self.pml.frametime))));
            }
            if self.pml.velocity[0] == 0.0 && self.pml.velocity[1] == 0.0 {
                return;
            }
            self.step_slide_move();
        } else {
            let airaccel = self.config.airaccel;
            if airaccel != 0.0 {
                self.air_accelerate(wishdir, wishspeed, airaccel);
            } else {
                self.accelerate(wishdir, wishspeed, 1.0);
            }
            if self.pm.s.pm_type != kex_pm_type::GRAPPLE {
                self.pml.velocity[2] =
                    f64::from(n.store(n.sub(self.pml.velocity[2], n.mul(self.pm.s.gravity, self.pml.frametime))));
            }
            self.step_slide_move();
        }
    }

    fn get_water_level(&mut self, position: SrcVec3) -> (i32, i32) {
        let n = self.runner.n;
        let math = self.runner.math;
        let mut level = water_level::NONE;
        let mut water_type = CONTENTS_NONE;
        let sample2 = n.sub(self.pm.s.viewheight, self.pm.mins[2]).trunc() as i32;
        let sample1 = n.div(sample2 as f64, 2.0).trunc() as i32;
        let mut point = math.vec3(
            position[0],
            position[1],
            n.add(n.add(position[2], self.pm.mins[2]), 1.0),
        );
        let cont = (self.pm.pointcontents)(point);
        if cont & MASK_WATER != 0 {
            water_type = cont;
            level = water_level::FEET;
            // Later samples reuse pml origin in the source.
            let origin = self.origin();
            point[2] = f64::from(n.store(n.add(n.add(origin[2], self.pm.mins[2]), sample1 as f64)));
            let cont = (self.pm.pointcontents)(point);
            if cont & MASK_WATER != 0 {
                level = water_level::WAIST;
                point[2] = f64::from(n.store(n.add(n.add(origin[2], self.pm.mins[2]), sample2 as f64)));
                let cont = (self.pm.pointcontents)(point);
                if cont & MASK_WATER != 0 {
                    level = water_level::UNDER;
                }
            }
        }
        (level, water_type)
    }

    fn categorize_position(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        let origin = self.origin();
        let point = math.vec3(origin[0], origin[1], n.sub(origin[2], self.source_float(0.25)));
        if self.pml.velocity[2] > 180.0 || self.pm.s.pm_type == kex_pm_type::GRAPPLE {
            self.pm.s.pm_flags &= !pm_flags::ON_GROUND;
            self.pm.groundentity = None;
        } else {
            let mins = self.pm.mins;
            let maxs = self.pm.maxs;
            let trace = self.pm_trace_auto(origin, mins, maxs, point);
            self.pm.groundplane = trace.plane.clone();
            self.pml.groundsurface = trace.surface.clone();
            self.pml.groundcontents = trace.contents;
            let mut slanted = trace.fraction < 1.0 && trace.plane.normal[2] < self.source_float(0.7);
            if slanted {
                let probe = math.add(origin, trace.plane.normal);
                let mins = self.pm.mins;
                let maxs = self.pm.maxs;
                let slant = self.pm_trace_auto(origin, mins, maxs, probe);
                if slant.fraction < 1.0 && !slant.startsolid {
                    slanted = false;
                }
            }
            if trace.fraction == 1.0 || (slanted && !trace.startsolid) {
                self.pm.groundentity = None;
                self.pm.s.pm_flags &= !pm_flags::ON_GROUND;
            } else {
                self.pm.groundentity = trace.ent.clone();
                if self.pm.s.pm_flags & pm_flags::TIME_WATERJUMP != 0 {
                    self.pm.s.pm_flags &= !(pm_flags::TIME_WATERJUMP
                        | pm_flags::TIME_LAND
                        | pm_flags::TIME_TELEPORT
                        | pm_flags::TIME_TRICK);
                    self.pm.s.pm_time = 0;
                }
                if self.pm.s.pm_flags & pm_flags::ON_GROUND == 0 {
                    if !self.config.n64_physics
                        && self.pml.velocity[2] >= 100.0
                        && self.pm.groundplane.normal[2] >= self.source_float(0.9)
                        && self.pm.s.pm_flags & pm_flags::DUCKED == 0
                    {
                        self.pm.s.pm_flags |= pm_flags::TIME_TRICK;
                        self.pm.s.pm_time = 64;
                    }
                    let clipped = math.slide_clip_velocity(
                        self.pml.velocity,
                        self.pm.groundplane.normal,
                        self.source_float(1.01),
                    );
                    self.pm.impact_delta = n.sub(self.pml.start_velocity[2], clipped[2]);
                    self.pm.s.pm_flags |= pm_flags::ON_GROUND;
                    if self.config.n64_physics || self.pm.s.pm_flags & pm_flags::DUCKED != 0 {
                        self.pm.s.pm_flags |= pm_flags::TIME_LAND;
                        self.pm.s.pm_time = 128;
                    }
                }
            }
            let touch = &mut self.pm.touch;
            record_touch(touch, &trace);
        }
        let origin = self.origin();
        let (level, water_type) = self.get_water_level(origin);
        self.pm.waterlevel = level;
        self.pm.watertype = water_type;
    }

    fn check_jump(&mut self) {
        let n = self.runner.n;
        if self.pm.s.pm_flags & pm_flags::TIME_LAND != 0 {
            return;
        }
        if self.pm.cmd.buttons & button::JUMP == 0 {
            self.pm.s.pm_flags &= !pm_flags::JUMP_HELD;
            return;
        }
        if self.pm.s.pm_flags & pm_flags::JUMP_HELD != 0 {
            return;
        }
        if self.pm.s.pm_type == kex_pm_type::DEAD {
            return;
        }
        if self.pm.waterlevel >= water_level::WAIST {
            self.pm.groundentity = None;
            return;
        }
        if self.pm.groundentity.is_none() {
            return;
        }
        self.pm.s.pm_flags |= pm_flags::JUMP_HELD;
        self.pm.jump_sound = true;
        self.pm.groundentity = None;
        self.pm.s.pm_flags &= !pm_flags::ON_GROUND;
        let jump_height = 270.0;
        self.pml.velocity[2] = f64::from(n.store(n.add(self.pml.velocity[2], jump_height)));
        if self.pml.velocity[2] < jump_height {
            self.pml.velocity[2] = f64::from(n.store(jump_height));
        }
    }

    fn check_special_movement(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        if self.pm.s.pm_time != 0 {
            return;
        }
        self.pm.s.pm_flags &= !pm_flags::ON_LADDER;
        let mut flatforward = [self.pml.forward[0], self.pml.forward[1], 0.0];
        math.normalize(&mut flatforward);
        let origin = self.origin();
        let spot = math.add(origin, math.muls(flatforward, 1.0));
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        let mut trace = self.pm_trace_masked(origin, mins, maxs, spot, super::types::CONTENTS_LADDER);
        if trace.fraction < 1.0
            && trace.contents & super::types::CONTENTS_LADDER != 0
            && self.pm.waterlevel < water_level::WAIST
        {
            self.pm.s.pm_flags |= pm_flags::ON_LADDER;
        }
        if self.pm.s.gravity == 0.0 {
            return;
        }
        if self.pm.cmd.buttons & button::JUMP == 0 && self.pm.cmd.forwardmove <= 0.0 {
            return;
        }
        if self.pm.waterlevel != water_level::WAIST || self.pm.watertype & CONTENTS_NO_WATERJUMP != 0 {
            return;
        }
        let probe = math.add(origin, math.muls(flatforward, 40.0));
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        trace = self.pm_trace_masked(origin, mins, maxs, probe, MASK_SOLID);
        if trace.fraction == 1.0 || trace.plane.normal[2] >= self.source_float(0.7) {
            return;
        }
        let mut waterjump_vel = math.muls(flatforward, 50.0);
        waterjump_vel[2] = f64::from(n.store(350.0));
        let mut touches = KexTouchList {
            num: 0,
            traces: Vec::new(),
        };
        let mut waterjump_origin = origin;
        let time = self.source_float(0.1);
        let mut has_time = true;
        let iter_count = (50.0f64).min(n.mul(10.0, n.div(800.0, self.pm.s.gravity)).trunc()) as i32;
        for _ in 0..iter_count {
            waterjump_vel[2] = f64::from(n.store(n.sub(waterjump_vel[2], n.mul(self.pm.s.gravity, time))));
            if waterjump_vel[2] < 0.0 {
                has_time = false;
            }
            let mins = self.pm.mins;
            let maxs = self.pm.maxs;
            self.sweep_temp(
                &mut waterjump_origin,
                &mut waterjump_vel,
                time,
                mins,
                maxs,
                &mut touches,
                has_time,
            );
        }
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        let below = math.sub(waterjump_origin, [0.0, 0.0, 2.0]);
        trace = self.pm_trace_masked(waterjump_origin, mins, maxs, below, MASK_SOLID);
        if trace.fraction == 1.0 || trace.plane.normal[2] < self.source_float(0.7) || trace.endpos[2] < origin[2] {
            return;
        }
        if self.pm.groundentity.is_some() && n.sub(origin[2], trace.endpos[2]).abs() <= f64::from(STEPSIZE) {
            return;
        }
        let (level, _) = self.get_water_level(trace.endpos);
        if level >= water_level::WAIST {
            return;
        }
        self.pml.velocity[0] = f64::from(n.store(n.mul(flatforward[0], 50.0)));
        self.pml.velocity[1] = f64::from(n.store(n.mul(flatforward[1], 50.0)));
        self.pml.velocity[2] = f64::from(n.store(350.0));
        self.pm.s.pm_flags |= pm_flags::TIME_WATERJUMP;
        self.pm.s.pm_time = 2048;
    }

    fn fly_move(&mut self, doclip: bool) {
        let n = self.runner.n;
        let math = self.runner.math;
        let stopspeed = self.runner.pm_stopspeed;
        let maxspeed = self.runner.pm_maxspeed;
        let accelerate = self.runner.pm_accelerate;
        let friction = self.runner.pm_friction;
        let waterspeed = self.runner.pm_waterspeed;
        self.pm.s.viewheight = if doclip { 0.0 } else { 22.0 };
        let speed = math.length(self.pml.velocity);
        if speed < 1.0 {
            self.pml.velocity = [
                f64::from(n.store(0.0)),
                f64::from(n.store(0.0)),
                f64::from(n.store(0.0)),
            ];
        } else {
            let friction = n.mul(friction, 1.5);
            let control = if speed < stopspeed { stopspeed } else { speed };
            let drop = n.mul(n.mul(control, friction), self.pml.frametime);
            let mut newspeed = n.sub(speed, drop);
            if newspeed < 0.0 {
                newspeed = 0.0;
            }
            newspeed = n.div(newspeed, speed);
            math.mul_eq(&mut self.pml.velocity, newspeed);
        }
        let fmove = self.equipment_speed(self.pm.cmd.forwardmove);
        let smove = self.equipment_speed(self.pm.cmd.sidemove);
        math.normalize(&mut self.pml.forward);
        math.normalize(&mut self.pml.right);
        let mut wishvel = math.vec3(
            n.add(n.mul(self.pml.forward[0], fmove), n.mul(self.pml.right[0], smove)),
            n.add(n.mul(self.pml.forward[1], fmove), n.mul(self.pml.right[1], smove)),
            n.add(n.mul(self.pml.forward[2], fmove), n.mul(self.pml.right[2], smove)),
        );
        if self.pm.cmd.buttons & button::JUMP != 0 {
            wishvel[2] = f64::from(n.store(n.add(
                wishvel[2],
                self.equipment_speed(n.mul(waterspeed, self.source_float(0.5))),
            )));
        }
        if self.pm.cmd.buttons & button::CROUCH != 0 {
            wishvel[2] = f64::from(n.store(n.sub(
                wishvel[2],
                self.equipment_speed(n.mul(waterspeed, self.source_float(0.5))),
            )));
        }
        let mut wishdir = wishvel;
        let mut wishspeed = math.normalize(&mut wishdir);
        if wishspeed > maxspeed {
            math.mul_eq(&mut wishvel, n.div(maxspeed, wishspeed));
            wishspeed = maxspeed;
        }
        wishspeed = n.mul(wishspeed, 2.0);
        let currentspeed = math.dot(self.pml.velocity, wishdir);
        let addspeed = n.sub(wishspeed, currentspeed);
        if addspeed > 0.0 {
            let mut accelspeed = n.mul(n.mul(accelerate, self.pml.frametime), wishspeed);
            if accelspeed > addspeed {
                accelspeed = addspeed;
            }
            for i in AXES {
                self.pml.velocity[i] = f64::from(n.store(n.add(self.pml.velocity[i], n.mul(accelspeed, wishdir[i]))));
            }
        }
        if doclip {
            self.step_slide_move();
        } else {
            let origin = self.origin();
            let velocity = self.pml.velocity;
            let frametime = self.pml.frametime;
            let next = math.add(origin, math.muls(velocity, frametime));
            self.set_origin(next);
        }
    }

    fn set_dimensions(&mut self) {
        let n = self.runner.n;
        let character = self.pm.character_bounds;
        self.pm.mins[0] = f64::from(n.store(f64::from(character.min.x)));
        self.pm.mins[1] = f64::from(n.store(f64::from(character.min.y)));
        self.pm.maxs[0] = f64::from(n.store(f64::from(character.max.x)));
        self.pm.maxs[1] = f64::from(n.store(f64::from(character.max.y)));
        if self.pm.s.pm_type == kex_pm_type::GIB {
            self.pm.mins[2] = f64::from(n.store(character_height(&character, 0.0, &n)));
            self.pm.maxs[2] = f64::from(n.store(character_height(&character, 16.0, &n)));
            self.pm.s.viewheight = character_height(&character, 8.0, &n);
            return;
        }
        self.pm.mins[2] = f64::from(n.store(character_height(&character, -24.0, &n)));
        if self.pm.s.pm_flags & pm_flags::DUCKED != 0 || self.pm.s.pm_type == kex_pm_type::DEAD {
            self.pm.maxs[2] = f64::from(n.store(character_height(&character, 4.0, &n)));
            self.pm.s.viewheight = character_height(&character, -2.0, &n);
        } else {
            self.pm.maxs[2] = f64::from(n.store(character_height(&character, 32.0, &n)));
            self.pm.s.viewheight = character_height(&character, 22.0, &n);
        }
        let requested = self.pm.body_bounds.unwrap_or(Bounds {
            min: vec3(self.pm.mins[0] as f32, self.pm.mins[1] as f32, self.pm.mins[2] as f32),
            max: vec3(self.pm.maxs[0] as f32, self.pm.maxs[1] as f32, self.pm.maxs[2] as f32),
        });
        let previous = self.accepted_body_bounds.unwrap_or(requested);
        let origin = self.origin();
        let bounds = accept_body_bounds(&previous, &requested, |bounds| {
            let trace = self.pm_trace_auto(
                origin,
                [
                    f64::from(bounds.min.x),
                    f64::from(bounds.min.y),
                    f64::from(bounds.min.z),
                ],
                [
                    f64::from(bounds.max.x),
                    f64::from(bounds.max.y),
                    f64::from(bounds.max.z),
                ],
                origin,
            );
            !trace.allsolid
        });
        if bounds != requested {
            self.pm.s.pm_flags = (self.pm.s.pm_flags & !pm_flags::DUCKED) | self.accepted_duck;
            self.pm.s.viewheight = self.accepted_height;
        }
        self.accepted_duck = self.pm.s.pm_flags & pm_flags::DUCKED;
        self.accepted_height = self.pm.s.viewheight;
        self.accepted_body_bounds = Some(bounds);
        self.pm.mins = [
            f64::from(bounds.min.x),
            f64::from(bounds.min.y),
            f64::from(bounds.min.z),
        ];
        self.pm.maxs = [
            f64::from(bounds.max.x),
            f64::from(bounds.max.y),
            f64::from(bounds.max.z),
        ];
    }

    fn above_water(&mut self) -> bool {
        let n = self.runner.n;
        let math = self.runner.math;
        let origin = self.origin();
        let below = math.vec3(origin[0], origin[1], n.sub(origin[2], 8.0));
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        let player = self.pm.player.clone();
        let solid_below = (self.pm.trace)(origin, mins, maxs, below, player.clone(), MASK_SOLID).fraction < 1.0;
        if solid_below {
            return false;
        }
        (self.pm.trace)(origin, mins, maxs, below, player, MASK_WATER).fraction < 1.0
    }

    fn check_duck(&mut self) -> bool {
        let n = self.runner.n;
        if self.pm.s.pm_type == kex_pm_type::GIB {
            return false;
        }
        let mut flags_changed = false;
        if self.pm.s.pm_type == kex_pm_type::DEAD {
            if self.pm.s.pm_flags & pm_flags::DUCKED == 0 {
                self.pm.s.pm_flags |= pm_flags::DUCKED;
                flags_changed = true;
            }
        } else if self.pm.cmd.buttons & button::CROUCH != 0
            && (self.pm.groundentity.is_some() || (self.pm.waterlevel <= water_level::FEET && !self.above_water()))
            && self.pm.s.pm_flags & pm_flags::ON_LADDER == 0
            && !self.config.n64_physics
        {
            if self.pm.s.pm_flags & pm_flags::DUCKED == 0 {
                let (check_mins, check_maxs) = match self.pm.body_bounds {
                    Some(body) => (
                        [f64::from(body.min.x), f64::from(body.min.y), f64::from(body.min.z)],
                        [f64::from(body.max.x), f64::from(body.max.y), f64::from(body.max.z)],
                    ),
                    None => {
                        let character = self.pm.character_bounds;
                        (
                            self.pm.mins,
                            [self.pm.maxs[0], self.pm.maxs[1], character_height(&character, 4.0, &n)],
                        )
                    }
                };
                let origin = self.origin();
                let trace = self.pm_trace_auto(origin, check_mins, check_maxs, origin);
                if !trace.allsolid {
                    self.pm.s.pm_flags |= pm_flags::DUCKED;
                    flags_changed = true;
                }
            }
        } else if self.pm.s.pm_flags & pm_flags::DUCKED != 0 {
            let (check_mins, check_maxs) = match self.pm.body_bounds {
                Some(body) => (
                    [f64::from(body.min.x), f64::from(body.min.y), f64::from(body.min.z)],
                    [f64::from(body.max.x), f64::from(body.max.y), f64::from(body.max.z)],
                ),
                None => (
                    self.pm.mins,
                    [
                        self.pm.maxs[0],
                        self.pm.maxs[1],
                        f64::from(self.pm.character_bounds.max.z),
                    ],
                ),
            };
            let origin = self.origin();
            let trace = self.pm_trace_auto(origin, check_mins, check_maxs, origin);
            if !trace.allsolid {
                self.pm.s.pm_flags &= !pm_flags::DUCKED;
                flags_changed = true;
            }
        }
        if !flags_changed {
            return false;
        }
        self.set_dimensions();
        true
    }

    fn dead_move(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        if self.pm.groundentity.is_none() {
            return;
        }
        let mut forward = math.length(self.pml.velocity);
        forward = n.sub(forward, 20.0);
        if forward <= 0.0 {
            self.pml.velocity = [
                f64::from(n.store(0.0)),
                f64::from(n.store(0.0)),
                f64::from(n.store(0.0)),
            ];
        } else {
            math.normalize(&mut self.pml.velocity);
            math.mul_eq(&mut self.pml.velocity, forward);
        }
    }

    fn good_position(&mut self) -> bool {
        if self.pm.s.pm_type == kex_pm_type::NOCLIP {
            return true;
        }
        let origin = self.pm.s.origin;
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        !self.pm_trace_auto(origin, mins, maxs, origin).allsolid
    }

    fn snap_position(&mut self) {
        let math = self.runner.math;
        let velocity = self.pml.velocity;
        let origin = self.origin();
        math.copy(velocity, &mut self.pm.s.velocity);
        math.copy(origin, &mut self.pm.s.origin);
        if self.good_position() {
            return;
        }
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        let n = self.runner.n;
        let pm_type = self.pm.s.pm_type;
        let pm_flags_value = self.pm.s.pm_flags;
        let player = self.pm.player.clone();
        let trace_cb = &mut self.pm.trace;
        let clip_cb = &mut self.pm.clip;
        let mut auto = |start: SrcVec3, mins: SrcVec3, maxs: SrcVec3, end: SrcVec3| -> TraceT {
            if pm_type == kex_pm_type::SPECTATOR {
                return clip_cb(start, mins, maxs, end, MASK_SOLID);
            }
            let mut mask = if pm_type == kex_pm_type::DEAD || pm_type == kex_pm_type::GIB {
                super::types::MASK_DEADSOLID
            } else {
                MASK_PLAYERSOLID
            };
            if pm_flags_value & pm_flags::IGNORE_PLAYER_COLLISION != 0 {
                mask &= !super::types::CONTENTS_PLAYER;
            }
            trace_cb(start, mins, maxs, end, player.clone(), mask)
        };
        if fix_stuck_object(n, math, &mut self.pm.s.origin, mins, maxs, &mut auto) == StuckResult::NoGoodPosition {
            let previous = self.pml.previous_origin;
            math.copy(previous, &mut self.pm.s.origin);
        }
    }

    fn initial_snap_position(&mut self) {
        let n = self.runner.n;
        let base = self.pm.s.origin;
        search_initial_snap_position(|x, y, z| {
            self.pm.s.origin[2] = f64::from(n.store(n.add(base[2], Q2_INITIAL_SNAP_OFFSETS[z])));
            self.pm.s.origin[1] = f64::from(n.store(n.add(base[1], Q2_INITIAL_SNAP_OFFSETS[y])));
            self.pm.s.origin[0] = f64::from(n.store(n.add(base[0], Q2_INITIAL_SNAP_OFFSETS[x])));
            if self.good_position() {
                let origin = self.pm.s.origin;
                self.set_origin(origin);
                self.pml.previous_origin = origin;
                return true;
            }
            false
        });
    }

    fn clamp_angles(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        let angles = self.pm.cmd.angles;
        let delta = self.pm.s.delta_angles;
        let flags = self.pm.s.pm_flags;
        rerelease_view_angles(&mut self.pm.viewangles, angles, delta, flags, &n);
        let viewangles = self.pm.viewangles;
        math.angle_vectors(viewangles, &mut self.pml.forward, &mut self.pml.right, &mut self.pml.up);
    }

    fn screen_effects(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        let origin = self.origin();
        let vieworg = math.vec3(
            n.add(origin[0], self.pm.viewoffset[0]),
            n.add(origin[1], self.pm.viewoffset[1]),
            n.add(n.add(origin[2], self.pm.viewoffset[2]), self.pm.s.viewheight),
        );
        let contents = (self.pm.pointcontents)(vieworg);
        if contents & (CONTENTS_LAVA | CONTENTS_SLIME | CONTENTS_WATER) != 0 {
            self.pm.rdflags |= refdef_flags::UNDERWATER;
        } else {
            self.pm.rdflags &= !refdef_flags::UNDERWATER;
        }
        if contents & (CONTENTS_SOLID | CONTENTS_LAVA) != 0 {
            math.add_blend(
                1.0,
                self.source_float(0.3),
                self.source_float(0.0),
                self.source_float(0.6),
                &mut self.pm.screen_blend,
            );
        } else if contents & CONTENTS_SLIME != 0 {
            math.add_blend(
                self.source_float(0.0),
                self.source_float(0.1),
                self.source_float(0.05),
                self.source_float(0.6),
                &mut self.pm.screen_blend,
            );
        } else if contents & CONTENTS_WATER != 0 {
            math.add_blend(
                self.source_float(0.5),
                self.source_float(0.3),
                self.source_float(0.2),
                self.source_float(0.4),
                &mut self.pm.screen_blend,
            );
        }
    }

    fn pmove_main(&mut self) {
        let n = self.runner.n;
        let math = self.runner.math;
        let flight = self.runner.flight;
        self.pm.touch.num = 0;
        self.pm.viewangles = [
            f64::from(n.store(0.0)),
            f64::from(n.store(0.0)),
            f64::from(n.store(0.0)),
        ];
        self.pm.s.viewheight = 0.0;
        self.pm.groundentity = None;
        self.pm.watertype = CONTENTS_NONE;
        self.pm.waterlevel = water_level::NONE;
        self.pm.screen_blend = [
            f64::from(n.store(0.0)),
            f64::from(n.store(0.0)),
            f64::from(n.store(0.0)),
            f64::from(n.store(0.0)),
        ];
        self.pm.rdflags = refdef_flags::NONE;
        self.pm.jump_sound = false;
        self.pm.step_clip = false;
        self.pm.impact_delta = 0.0;
        let start_origin = self.pm.s.origin;
        self.set_origin(start_origin);
        self.pml.velocity = self.pm.s.velocity;
        self.pml.frametime = n.mul(self.pm.cmd.msec as f64, self.source_float(0.001));
        self.pml.previous_origin = self.pm.s.origin;
        self.pml.start_velocity = self.pm.s.velocity;
        self.clamp_angles();
        if flight && self.pm.s.pm_type == kex_pm_type::NORMAL {
            self.pm.s.pm_flags &= !(pm_flags::ON_GROUND | pm_flags::DUCKED | pm_flags::TIME_WATERJUMP);
            self.pm.s.pm_time = 0;
            self.fly_move(true);
            self.snap_position();
            return;
        }
        if self.pm.s.pm_type == kex_pm_type::SPECTATOR || self.pm.s.pm_type == kex_pm_type::NOCLIP {
            self.pm.s.pm_flags = pm_flags::NONE;
            if self.pm.s.pm_type == kex_pm_type::SPECTATOR {
                let character = self.pm.character_bounds;
                self.pm.mins[0] = n.mul(f64::from(character.min.x), 0.5);
                self.pm.mins[1] = n.mul(f64::from(character.min.y), 0.5);
                self.pm.maxs[0] = n.mul(f64::from(character.max.x), 0.5);
                self.pm.maxs[1] = n.mul(f64::from(character.max.y), 0.5);
                self.pm.mins[2] = f64::from(n.store(character_height(&character, -8.0, &n)));
                self.pm.maxs[2] = f64::from(n.store(character_height(&character, 8.0, &n)));
            }
            let spectator = self.pm.s.pm_type == kex_pm_type::SPECTATOR;
            self.fly_move(spectator);
            self.snap_position();
            return;
        }
        if self.pm.s.pm_type >= kex_pm_type::DEAD {
            self.pm.cmd.forwardmove = 0.0;
            self.pm.cmd.sidemove = 0.0;
            self.pm.cmd.buttons &= !(button::JUMP | button::CROUCH);
        }
        if self.pm.s.pm_type == kex_pm_type::FREEZE {
            return;
        }
        self.set_dimensions();
        self.categorize_position();
        if self.pm.snapinitial {
            self.initial_snap_position();
        }
        if self.check_duck() {
            self.categorize_position();
        }
        if self.pm.s.pm_type == kex_pm_type::DEAD {
            self.dead_move();
        }
        self.check_special_movement();
        if self.pm.s.pm_time != 0 {
            if self.pm.cmd.msec >= self.pm.s.pm_time {
                self.pm.s.pm_flags &=
                    !(pm_flags::TIME_WATERJUMP | pm_flags::TIME_LAND | pm_flags::TIME_TELEPORT | pm_flags::TIME_TRICK);
                self.pm.s.pm_time = 0;
            } else {
                self.pm.s.pm_time = n.sub(self.pm.s.pm_time as f64, self.pm.cmd.msec as f64) as i32;
            }
        }
        if self.pm.s.pm_flags & pm_flags::TIME_TELEPORT != 0 {
        } else if self.pm.s.pm_flags & pm_flags::TIME_WATERJUMP != 0 {
            self.pml.velocity[2] =
                f64::from(n.store(n.sub(self.pml.velocity[2], n.mul(self.pm.s.gravity, self.pml.frametime))));
            if self.pml.velocity[2] < 0.0 {
                self.pm.s.pm_flags &=
                    !(pm_flags::TIME_WATERJUMP | pm_flags::TIME_LAND | pm_flags::TIME_TELEPORT | pm_flags::TIME_TRICK);
                self.pm.s.pm_time = 0;
            }
            self.step_slide_move();
        } else {
            self.check_jump();
            self.friction();
            if self.pm.waterlevel >= water_level::WAIST {
                self.water_move();
            } else {
                let mut angles = self.pm.viewangles;
                if angles[PITCH] > 180.0 {
                    angles[PITCH] = f64::from(n.store(n.sub(angles[PITCH], 360.0)));
                }
                angles[PITCH] = f64::from(n.store(n.div(angles[PITCH], 3.0)));
                math.angle_vectors(angles, &mut self.pml.forward, &mut self.pml.right, &mut self.pml.up);
                self.air_move();
            }
        }
        self.categorize_position();
        if self.pm.s.pm_flags & pm_flags::TIME_TRICK != 0 {
            self.check_jump();
        }
        self.screen_effects();
        self.snap_position();
    }
}

/// Run rerelease pmove over one command.
pub fn pmove_rerelease(
    pm: &mut KexPmove<'_>,
    numeric_ops: NumericOps,
    config: &PmConfig,
    context: &mut Q2RereleaseMovementContext,
    flight: bool,
    speed_multiplier: f64,
) {
    create_rerelease_movement(numeric_ops, context, flight, speed_multiplier).pmove(pm, config);
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};

    use super::super::types::{KexPmoveCmd, KexPmoveState, PM_CONFIG_DEFAULT};

    fn open_trace(end: SrcVec3) -> TraceT {
        TraceT {
            allsolid: false,
            startsolid: false,
            fraction: 1.0,
            endpos: end,
            plane: super::super::types::plane(),
            surface: None,
            contents: 0,
            ent: None,
            plane2: super::super::types::plane(),
            surface2: None,
            native: None,
        }
    }

    fn fixture() -> KexPmove<'static> {
        KexPmove {
            s: KexPmoveState {
                pm_type: kex_pm_type::NORMAL,
                origin: [0.0, 0.0, 100.0],
                velocity: [0.0, 0.0, 0.0],
                pm_flags: pm_flags::ON_GROUND,
                pm_time: 0,
                gravity: 800.0,
                delta_angles: [0.0, 0.0, 0.0],
                viewheight: 22.0,
            },
            cmd: KexPmoveCmd {
                msec: 16,
                angles: [0.0, 90.0, 0.0],
                forwardmove: 400.0,
                sidemove: 0.0,
                buttons: 0,
                server_frame: 1,
            },
            snapinitial: false,
            touch: KexTouchList {
                num: 0,
                traces: Vec::new(),
            },
            viewangles: [0.0, 0.0, 0.0],
            mins: [-16.0, -16.0, -24.0],
            maxs: [16.0, 16.0, 32.0],
            groundentity: None,
            groundplane: super::super::types::plane(),
            watertype: 0,
            waterlevel: 0,
            player: None,
            trace: Box::new(|_start, _mins, _maxs, end, _pass, _mask| open_trace(end)),
            clip: Box::new(|_start, _mins, _maxs, end, _mask| open_trace(end)),
            pointcontents: Box::new(|_| 0),
            viewoffset: [0.0, 0.0, 0.0],
            screen_blend: [0.0, 0.0, 0.0, 0.0],
            rdflags: 0,
            jump_sound: false,
            step_clip: false,
            impact_delta: 0.0,
            character_bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            body_bounds: None,
            previous_bounds: None,
        }
    }

    fn ground_fixture() -> KexPmove<'static> {
        let mut pm = fixture();
        pm.trace = Box::new(|start, _mins, _maxs, end, _pass, _mask| {
            if end[2] < start[2] && start[2] - end[2] <= 1.0 {
                let mut plane = super::super::types::plane();
                plane.normal = [0.0, 0.0, 1.0];
                TraceT {
                    allsolid: false,
                    startsolid: false,
                    fraction: 0.0,
                    endpos: start,
                    plane,
                    surface: None,
                    contents: 0,
                    ent: Some(super::super::super::types::TraceHit::World { model: 0 }),
                    plane2: super::super::types::plane(),
                    surface2: None,
                    native: None,
                }
            } else {
                open_trace(end)
            }
        });
        pm
    }

    #[test]
    fn ground_step_accelerates() {
        let mut pm = ground_fixture();
        let mut context = Q2RereleaseMovementContext::new();
        pmove_rerelease(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            &PM_CONFIG_DEFAULT,
            &mut context,
            false,
            1.0,
        );
        assert_eq!(pm.s.pm_flags & pm_flags::ON_GROUND, pm_flags::ON_GROUND);
        assert!(pm.s.velocity[1] > 0.0);
        assert_eq!(pm.s.viewheight, 22.0);
    }

    #[test]
    fn jump_button_reports_sound() {
        let mut pm = ground_fixture();
        pm.cmd.buttons = button::JUMP;
        let mut context = Q2RereleaseMovementContext::new();
        pmove_rerelease(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            &PM_CONFIG_DEFAULT,
            &mut context,
            false,
            1.0,
        );
        assert!(pm.jump_sound);
        // Jump launches at 270 then air-move gravity applies in the same frame.
        assert!(pm.s.velocity[2] > 200.0);
    }

    #[test]
    fn underwater_view_blends() {
        let mut pm = fixture();
        pm.pointcontents = Box::new(|_| CONTENTS_WATER);
        let mut context = Q2RereleaseMovementContext::new();
        pmove_rerelease(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            &PM_CONFIG_DEFAULT,
            &mut context,
            false,
            1.0,
        );
        assert_eq!(pm.rdflags & refdef_flags::UNDERWATER, refdef_flags::UNDERWATER);
        assert!(pm.screen_blend[3] > 0.0);
    }

    #[test]
    fn stuck_fix_reports_good_position() {
        let numeric = NumericOps::select(Q2_DONOR_PROFILE).unwrap();
        let math = Q2Math::new(numeric, Q2MathEdition::Rerelease);
        let mut origin = [0.0, 0.0, 0.0];
        let mut trace = |start: SrcVec3, _mins: SrcVec3, _maxs: SrcVec3, _end: SrcVec3| -> TraceT { open_trace(start) };
        let result = fix_stuck_object(
            numeric,
            math,
            &mut origin,
            [-16.0, -16.0, -24.0],
            [16.0, 16.0, 32.0],
            &mut trace,
        );
        assert_eq!(result, StuckResult::GoodPosition);
    }

    #[test]
    fn context_captures_and_restores() {
        let mut context = Q2RereleaseMovementContext::new();
        context.pml_origin = [1.0, 2.0, 3.0];
        assert_eq!(context.capture(), vec3(1.0, 2.0, 3.0));
        context.restore(vec3(4.0, 5.0, 6.0));
        assert_eq!(context.pml_origin, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn spectator_flies() {
        let mut pm = fixture();
        pm.s.pm_type = kex_pm_type::SPECTATOR;
        let mut context = Q2RereleaseMovementContext::new();
        pmove_rerelease(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            &PM_CONFIG_DEFAULT,
            &mut context,
            false,
            1.0,
        );
        assert!(pm.s.velocity[1] > 0.0);
    }

    #[test]
    fn walk_advances_origin_in_open_space() {
        let mut pm = fixture();
        let mut context = Q2RereleaseMovementContext::new();
        pmove_rerelease(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            &PM_CONFIG_DEFAULT,
            &mut context,
            false,
            1.0,
        );
        // Open traces, forward command: velocity without displacement
        // means the sweep result was discarded (stale recovery restore).
        assert_ne!(pm.s.origin, [0.0, 0.0, 100.0], "origin never moved");
        assert_eq!(context.pml_origin, pm.s.origin);
    }
}
