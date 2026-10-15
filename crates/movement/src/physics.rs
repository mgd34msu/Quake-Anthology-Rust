use qa_core::{
    math,
    primitives::{
        AngleOffset, MotionWrite, MovementMagnitude, MovementMode, MovementTimer, MovementTuning,
        PlayerState, RuleSetId, SurfaceFlags, ThinkTime, UserCmd, Vec3, buttons,
    },
    sys_events::EventTime,
};
use qa_gameplay::rules::movement::{
    FreeAcceleration, FreeFinish, FreeFriction, FreeModes, FreeMotion, FreeMovePolicy, FreeWish,
    LandingPolicy, ProbePhase, SpecialMovement, StancePolicy, ViewAnglePolicy, capabilities,
};
use qa_world::collision::{
    Contents, EntityTracePolicy, Trace, TraceError, TraceQuery, TraceRules, TraceScope, WorldTrace,
    trace_policy,
};

pub trait TraceServices {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError>;
    fn point_contents(&self, point: Vec3, rules: EntityTracePolicy)
    -> Result<Contents, TraceError>;
}
impl TraceServices for WorldTrace<'_> {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
        WorldTrace::trace(self, query)
    }
    fn point_contents(
        &self,
        point: Vec3,
        rules: EntityTracePolicy,
    ) -> Result<Contents, TraceError> {
        WorldTrace::point_contents(self, point, rules, &[])
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MovementResult {
    pub steps: u32,
    pub traces: u32,
    pub contacts: [Option<Trace>; 32],
    pub contact_count: usize,
    pub jump_sound: bool,
    pub step_clip: bool,
    pub impact_delta: f32,
}
/// Native providers split control from position integration. Ordinary callers
/// execute the full command, retaining their existing duration policy.
#[derive(Clone, Copy, Debug, Default)]
pub enum MovePhase {
    #[default]
    Complete,
    Control {
        body_angles: Vec3,
        clock: ThinkTime,
    },
    Integrate {
        seconds: f64,
    },
}
/// Inputs belonging to this invocation, never retained as player state.
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveOptions {
    pub initial_snap: bool,
    pub phase: MovePhase,
    /// Position prediction can consume the accepted view without changing it.
    pub keep_view_angles: bool,
}
pub type MoveEntry<E, B> = fn(
    UserCmd,
    &mut PlayerState,
    &mut dyn TraceServices,
    MoveOptions,
    &mut B,
) -> Result<MovementResult, E>;
pub fn entry<E, B>(rules: RuleSetId) -> MoveEntry<E, B>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    match rules {
        RuleSetId::Quake => quake::<E, B>,
        RuleSetId::QuakeWorld => quakeworld::<E, B>,
        RuleSetId::Quake2 => quake2::<E, B>,
        RuleSetId::Quake2Rerelease => rerelease::<E, B>,
        RuleSetId::Quake3 => arena::<E, B>,
    }
}
/// No native memory boundary is needed by ordinary engine callers.
pub fn no_boundary(_: &mut PlayerState, _: MotionWrite) -> Result<(), TraceError> {
    Ok(())
}
pub fn pmove<E, B>(
    command: UserCmd,
    player: &mut PlayerState,
    trace: &mut dyn TraceServices,
    options: MoveOptions,
    mut boundary: B,
) -> Result<MovementResult, E>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    entry(player.movement_rules)(command, player, trace, options, &mut boundary)
}

fn update_view_angles(command: UserCmd, player: &mut PlayerState, policy: ViewAnglePolicy) {
    if !player.movement.mode.allows_view_angles()
        || policy.lock_dead_non_spectator
            && player.health <= 0
            && player.movement.mode != MovementMode::Spectator
    {
        return;
    }
    if policy.short_angles {
        for i in 0..3 {
            let command_angle =
                math::angle_to_short(command.view_angles.0[i], math::AngleShortForm::Factored);
            let mut angle = command_angle.wrapping_add(
                player.movement.delta_angles.0[i].units(math::AngleShortForm::Factored),
            ) as i16;
            if policy.signed_pitch_limit && i == 0 {
                let clamped = angle.clamp(-16000, 16000);
                if clamped != angle {
                    let command_angle = command_angle as u16;
                    player.movement.delta_angles.0[i] =
                        AngleOffset::Units(i32::from(clamped) - i32::from(command_angle));
                    angle = clamped;
                }
            }
            player.view_angles.0[i] = math::short_to_angle(i32::from(angle));
        }
    } else {
        player.view_angles = command.view_angles + player.movement.delta_angles.degrees();
    }
    if policy.circular_pitch {
        let pitch = &mut player.view_angles.0[0];
        if *pitch > 89.0 && *pitch < 180.0 {
            *pitch = 89.0;
        } else if *pitch < 271.0 && *pitch >= 180.0 {
            *pitch = 271.0;
        }
        if player.movement.timer.contains(MovementTimer::TELEPORT) {
            player.view_angles.0[0] = 0.0;
            player.view_angles.0[2] = 0.0;
        }
    }
}

/// Angle-only prediction consumes the player's movement capability without
/// advancing position, timers, traces or native memory boundaries.
pub fn predict_view_angles(command: UserCmd, player: &mut PlayerState) {
    let policy = capabilities(player.movement_rules).view_angles.prediction;
    update_view_angles(command, player, policy);
}

fn with_rollback<E>(
    player: &mut PlayerState,
    phase: MovePhase,
    run: impl FnOnce(&mut PlayerState) -> Result<MovementResult, E>,
) -> Result<MovementResult, E> {
    let saved = (
        player.body,
        player.movement,
        player.view_angles,
        player.view_offset,
    );
    let result = run(player);
    // A staged native boundary already committed its admitted scalar prefix.
    // Restoring only Rust state would diverge from the surviving native row.
    if result.is_err() && matches!(phase, MovePhase::Complete) {
        (
            player.body,
            player.movement,
            player.view_angles,
            player.view_offset,
        ) = saved;
    }
    result
}

fn direct_motion<E, B>(player: &mut PlayerState, dt: f32, boundary: &mut B) -> Result<(), E>
where
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    let mut state = (player, boundary);
    math::vector_ma(
        &mut state,
        dt,
        |(player, _), axis| Ok((player.body.position.0[axis], player.body.velocity.0[axis])),
        |(player, boundary), axis, value| {
            player.body.position.0[axis] = value;
            boundary(player, MotionWrite::Origin(axis))
        },
    )
}

fn copy_body<E, B>(
    player: &mut PlayerState,
    position: Option<Vec3>,
    velocity: Vec3,
    baseline: Option<(Vec3, Vec3)>,
    boundary: &mut B,
) -> Result<(), E>
where
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    for (origin, values) in [(false, Some(velocity)), (true, position)] {
        let Some(values) = values else { continue };
        for axis in 0..3 {
            if baseline.is_some_and(|(position, velocity)| {
                let previous = if origin { position } else { velocity };
                previous.0[axis].to_bits() == values.0[axis].to_bits()
            }) {
                continue;
            }
            // Keep final values in scratch: readback after a native store may
            // replace the hot vectors, including another component's input.
            let field = if origin {
                player.body.position.0[axis] = values.0[axis];
                MotionWrite::Origin(axis)
            } else {
                player.body.velocity.0[axis] = values.0[axis];
                MotionWrite::Velocity(axis)
            };
            boundary(player, field)?;
        }
    }
    Ok(())
}

/// Spawn/stance initialization, independent of the module or map family.
pub fn set_bounds(player: &mut PlayerState) {
    let parameters = Parameters::load(player.movement_rules, player);
    let radius = parameters.radius;
    player.body.mins = Vec3([-radius, -radius, -24.0]);
    player.body.maxs = Vec3([radius, radius, 32.0]);
    player.view_offset = Vec3([0.0, 0.0, parameters.view_height]);
}
#[derive(Clone, Copy)]
enum FrictionContext {
    Native,
    Free(FreeFriction),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectiveTuning {
    pub gravity: f32,
    pub speed: f32,
    pub spectator_speed: f32,
}

/// Movement and snapshots use the same selected defaults and multiplier order.
pub fn effective_tuning(rules: RuleSetId, tuning: MovementTuning) -> EffectiveTuning {
    let legacy = matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld);
    let arena = rules == RuleSetId::Quake3;
    let speed = tuning
        .max_speed
        .unwrap_or(MovementMagnitude::Base(if legacy || arena {
            320.0
        } else {
            300.0
        }))
        .effective(tuning.speed_multiplier);
    EffectiveTuning {
        gravity: tuning
            .gravity
            .unwrap_or(MovementMagnitude::Base(800.0))
            .effective(tuning.gravity_multiplier),
        speed,
        spectator_speed: if let Some(speed) = tuning.spectator_max_speed {
            speed * tuning.speed_multiplier
        } else if rules == RuleSetId::QuakeWorld {
            500.0 * tuning.speed_multiplier
        } else {
            speed
        },
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Parameters {
    pub rules: RuleSetId,
    pub trace_rules: TraceRules,
    pub entity_rules: EntityTracePolicy,
    pub radius: f32,
    pub view_height: f32,
    pub gravity: f32,
    pub speed: f32,
    pub spectator_speed: f32,
    pub friction: f32,
    pub stop: f32,
    pub accel: f32,
    pub air: f32,
    capped_air: bool,
    pub water_accel: f32,
    pub water_friction: f32,
    pub no_step: bool,
    free: FreeModes,
    control: Option<FreeMovePolicy>,
    relative_command_clock: bool,
    pub special: SpecialMovement,
    pub landing: LandingPolicy,
    stance: StancePolicy,
    angles: ViewAnglePolicy,
    fly_accel: f32,
    exclusive_button: Option<u32>,
    dead_friction: Option<f32>,
}
impl Parameters {
    fn load(rules: RuleSetId, player: &PlayerState) -> Self {
        let t = player.movement.tuning;
        let legacy = matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld);
        let arena = rules == RuleSetId::Quake3;
        let tuning = effective_tuning(rules, t);
        let capabilities = capabilities(rules);
        let (trace_rules, entity_rules) = trace_policy(player.trace_rules);
        let accel = t.accelerate.unwrap_or(10.0);
        let configured_air = t.air_accelerate.unwrap_or(if arena { 1.0 } else { 0.0 });
        let (air, capped_air) = match rules {
            RuleSetId::Quake | RuleSetId::QuakeWorld => (accel, true),
            RuleSetId::Quake2 if configured_air != 0.0 => (accel, true),
            RuleSetId::Quake2Rerelease if configured_air != 0.0 => (configured_air, true),
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => (1.0, false),
            RuleSetId::Quake3 => (configured_air, false),
        };
        Self {
            rules,
            trace_rules,
            entity_rules,
            radius: if arena { 15.0 } else { 16.0 },
            view_height: if arena { 26.0 } else { 22.0 },
            gravity: tuning.gravity,
            speed: tuning.speed,
            spectator_speed: tuning.spectator_speed,
            friction: t.friction.unwrap_or(if legacy { 4.0 } else { 6.0 }),
            stop: t.stop_speed.unwrap_or(100.0),
            accel,
            air,
            capped_air,
            water_accel: t.water_accelerate.unwrap_or(if arena { 4.0 } else { 10.0 }),
            water_friction: t.water_friction.unwrap_or(if arena || legacy {
                if legacy { 4.0 } else { 1.0 }
            } else {
                1.0
            }),
            no_step: t.no_step,
            free: capabilities.free,
            control: capabilities.control,
            relative_command_clock: capabilities.relative_command_clock,
            special: capabilities.special,
            landing: capabilities.landing,
            stance: capabilities.stance,
            angles: capabilities.view_angles.movement,
            fly_accel: 8.0,
            exclusive_button: if arena { Some(buttons::TALK) } else { None },
            dead_friction: capabilities.dead_friction,
        }
    }
}
struct Wish {
    direction: Vec3,
    speed: f32,
    vertical: f32,
    raw: Vec3,
    length: f32,
    maximum: f32,
}
impl Wish {
    fn immediate_velocity(self) -> Vec3 {
        if self.length > self.maximum {
            self.raw * (self.maximum / self.length)
        } else {
            self.raw
        }
    }
}
pub(crate) struct Step<'a> {
    pub player: &'a mut PlayerState,
    pub world: &'a mut dyn TraceServices,
    pub result: &'a mut MovementResult,
    pub parameters: Parameters,
    pub command: UserCmd,
    pub dt: f32,
    pub exact_dt: f64,
    pub ground_plane: bool,
    pub ground_normal: Vec3,
    pub mask: Contents,
    pub scope: TraceScope,
    pub previous_velocity: Vec3,
    options: MoveOptions,
}
impl Step<'_> {
    pub fn trace(&mut self, start: Vec3, end: Vec3) -> Result<Trace, TraceError> {
        self.trace_bounds(start, end, self.player.body.mins, self.player.body.maxs)
    }
    fn trace_bounds(
        &mut self,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
    ) -> Result<Trace, TraceError> {
        self.trace_mask(start, end, mins, maxs, self.mask)
    }
    pub(crate) fn trace_mask(
        &mut self,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
        mask: Contents,
    ) -> Result<Trace, TraceError> {
        self.result.traces += 1;
        self.world.trace(TraceQuery {
            capsule: None,
            start,
            end,
            mins,
            maxs,
            mask,
            scope: self.scope,
            rules: self.parameters.trace_rules,
            entity_rules: self.parameters.entity_rules,
            pass: qa_core::primitives::CollisionOwner::None,
            excluded: &[],
        })
    }
    pub fn contact(&mut self, trace: &Trace) {
        if let Some(id) = trace.entity
            && self.result.contact_count < 32
            && (!(self.arena() || self.rr())
                || !self.result.contacts[..self.result.contact_count]
                    .iter()
                    .flatten()
                    .any(|contact| contact.entity == Some(id)))
        {
            self.result.contacts[self.result.contact_count] = Some(*trace);
            self.result.contact_count += 1;
        }
    }
    pub fn classic(&self) -> bool {
        self.parameters.rules == RuleSetId::Quake2
    }
    pub fn arena(&self) -> bool {
        self.parameters.rules == RuleSetId::Quake3
    }
    pub fn nq(&self) -> bool {
        self.parameters.rules == RuleSetId::Quake
    }
    pub fn qw(&self) -> bool {
        self.parameters.rules == RuleSetId::QuakeWorld
    }
    pub fn rr(&self) -> bool {
        self.parameters.rules == RuleSetId::Quake2Rerelease
    }
    pub fn accelerate(&mut self, direction: Vec3, speed: f32, accel: f32, air_cap: bool) -> bool {
        if self.player.movement.water_jump_seconds != 0.0
            && self
                .parameters
                .free
                .get(self.player.movement.mode)
                .is_none()
        {
            return false;
        }
        let target = if air_cap { speed.min(30.0) } else { speed };
        let add = target - self.player.body.velocity.dot(direction);
        if add <= 0.0 {
            return false;
        }
        let amount = if self.nq() {
            if air_cap {
                (f64::from(accel * speed) * self.exact_dt) as f32
            } else {
                (f64::from(accel) * self.exact_dt * f64::from(speed)) as f32
            }
        } else if air_cap {
            accel * speed * self.dt
        } else {
            accel * self.dt * speed
        };
        self.player.body.velocity = self.player.body.velocity + direction * amount.min(add);
        true
    }
    pub(crate) fn water_at(
        &self,
        position: Vec3,
        upper_origin: Vec3,
    ) -> Result<(u8, u64), TraceError> {
        let body = self.player.body;
        let mut point = position;
        point.0[2] += body.mins.0[2] + 1.0;
        let contents = self
            .world
            .point_contents(point, self.parameters.entity_rules)?;
        if !contents.intersects(Contents::FLUID) {
            return Ok((0, 0));
        }
        let mut level = 1;
        let middle = if self.nq() || self.qw() {
            qa_core::math::midpoint(body.mins, body.maxs).0[2]
        } else {
            body.mins.0[2] + ((self.player.view_offset.0[2] - body.mins.0[2]) as i32 / 2) as f32
        };
        point.0[2] = upper_origin.0[2] + middle;
        if self
            .world
            .point_contents(point, self.parameters.entity_rules)?
            .intersects(Contents::FLUID)
        {
            level = 2;
            point.0[2] = upper_origin.0[2] + self.player.view_offset.0[2];
            if self
                .world
                .point_contents(point, self.parameters.entity_rules)?
                .intersects(Contents::FLUID)
            {
                level = 3;
            }
        }
        Ok((level, contents.0))
    }
    fn water_level(&mut self) -> Result<(), TraceError> {
        let position = self.player.body.position;
        let (level, contents) = self.water_at(position, position)?;
        self.player.movement.water_level = level;
        self.player.movement.water_contents = contents;
        Ok(())
    }
    pub(crate) fn view_basis(&self, angles: Vec3) -> math::AngleBasis {
        if self.arena() {
            math::angle_vectors_radians(math::radians_from_degrees_f32(angles))
        } else {
            math::angle_vectors(angles)
        }
    }
    fn categorize(&mut self) -> Result<(), TraceError> {
        let was_grounded = self.player.movement.grounded;
        self.player.movement.grounded = false;
        self.player.movement.ground = None;
        self.ground_plane = false;
        self.ground_normal = Vec3::default();
        self.player.movement.ground_contents = 0;
        if self.player.body.velocity.0[2] <= 180.0 || self.arena() {
            let start = self.player.body.position;
            let drop = if self.qw() { 1.0 } else { 0.25 };
            let mut end = start;
            end.0[2] -= drop;
            let tr = self.trace(start, end)?;
            self.contact(&tr);
            let contact = tr.fraction < 1.0 || tr.start_solid;
            self.ground_plane = contact;
            self.ground_normal = tr.plane.normal;
            let kicked = self.arena()
                && self.player.body.velocity.0[2] > 0.0
                && self.player.body.velocity.dot(tr.plane.normal) > 10.0;
            let mut walkable = tr.plane.normal.0[2] >= 0.7;
            if self.parameters.special.wall_ground && tr.fraction < 1.0 && !walkable {
                // Rerelease PM_CatagorizePosition keeps the ground contact,
                // but admits its steep slope when the normal points into a wall.
                let wall = self.trace(start, start + tr.plane.normal)?;
                walkable = wall.fraction < 1.0 && !wall.start_solid;
            }
            let grounded = if self.parameters.special.wall_ground {
                tr.fraction < 1.0 && (walkable || tr.start_solid)
            } else {
                contact && (walkable || tr.start_solid && !self.arena() && !self.qw())
            };
            if grounded && !kicked {
                self.player.movement.grounded = true;
                self.player.movement.ground = tr.entity;
                self.player.movement.ground_normal = tr.plane.normal;
                self.player.movement.ground_surface = tr.surface;
                self.player.movement.ground_contents = tr.contents.0;
                if self
                    .player
                    .movement
                    .timer
                    .contains(MovementTimer::WATER_JUMP)
                {
                    let mask = self
                        .parameters
                        .special
                        .water_jump
                        .map_or(MovementTimer(u8::MAX), |policy| policy.ground_clear);
                    self.clear_timer_flags(mask);
                }
                if !was_grounded {
                    self.landed(tr.plane.normal);
                }
                if self.qw() && !tr.start_solid && !tr.all_solid {
                    self.player.body.position = tr.end;
                }
                if self.qw() {
                    self.player.movement.water_jump_seconds = 0.0;
                }
            }
        }
        self.water_level()?;
        Ok(())
    }
    fn dead_friction(&mut self) {
        if self.player.movement.mode != MovementMode::Dead || !self.player.movement.grounded {
            return;
        }
        let Some(drop) = self.parameters.dead_friction else {
            return;
        };
        // Native PM_DeadMove precedes ordinary friction and removes a fixed
        // speed per command, independently of the command's frame duration.
        let mut direction = self.player.body.velocity;
        let speed = math::normalize(&mut direction) - drop;
        self.player.body.velocity = if speed <= 0.0 {
            Vec3::default()
        } else {
            direction * speed
        };
    }
    fn landed(&mut self, normal: Vec3) {
        let policy = self.parameters.landing;
        let velocity = self.player.body.velocity;
        if let Some(fall) = policy.fall {
            let z = if fall.previous_velocity {
                self.previous_velocity.0[2]
            } else {
                velocity.0[2]
            };
            if z < fall.threshold {
                let duration = fall
                    .fast
                    .filter(|&(threshold, _)| z < threshold)
                    .map_or(fall.duration, |(_, time)| time);
                self.start_timer(MovementTimer::LAND, duration);
            }
        }
        let n64 = self.player.movement.tuning.n64_physics;
        if !n64
            && !self.player.movement.ducked
            && let Some(trick) = policy.trick
            && velocity.0[2] >= trick.threshold
            && normal.0[2] >= trick.normal
        {
            self.start_timer(MovementTimer::TRICK, trick.duration);
        }
        if policy.impact {
            let clipped = super::slide::clip(velocity, normal, self.parameters.rules);
            self.result.impact_delta = self.previous_velocity.0[2] - clipped.0[2];
        }
        let duration = if n64 {
            policy.n64_duration
        } else if self.player.movement.ducked {
            policy.duck_duration
        } else {
            None
        };
        if let Some(duration) = duration {
            self.start_timer(MovementTimer::LAND, duration);
        }
    }
    fn dimensions(&mut self) {
        if self.nq() || self.qw() {
            return;
        }
        for axis in 0..2 {
            self.player.body.mins.0[axis] = -self.parameters.radius;
            self.player.body.maxs.0[axis] = self.parameters.radius;
        }
        if self.parameters.stance.gib_bounds && self.player.movement.mode == MovementMode::Gib {
            self.player.body.mins.0[2] = 0.0;
            self.player.body.maxs.0[2] = 16.0;
            self.player.view_offset.0[2] = 8.0;
            return;
        }
        self.player.body.mins.0[2] = -24.0;
        let dead = self.arena()
            && matches!(
                self.player.movement.mode,
                MovementMode::Dead | MovementMode::Gib
            );
        self.player.body.maxs.0[2] = if dead {
            -8.0
        } else if self.player.movement.ducked
            || self.parameters.stance.dead_duck && self.player.movement.mode == MovementMode::Dead
        {
            if self.arena() { 16.0 } else { 4.0 }
        } else {
            32.0
        };
        self.player.view_offset.0[2] = if dead {
            -16.0
        } else if self.player.movement.ducked
            || self.parameters.stance.dead_duck && self.player.movement.mode == MovementMode::Dead
        {
            if self.arena() { 12.0 } else { -2.0 }
        } else {
            self.parameters.view_height
        };
    }
    fn above_water(&mut self) -> Result<bool, TraceError> {
        let origin = self.player.body.position;
        let below = origin - Vec3([0.0, 0.0, 8.0]);
        let mins = self.player.body.mins;
        let maxs = self.player.body.maxs;
        if self
            .trace_mask(
                origin,
                below,
                mins,
                maxs,
                Contents::SOLID | Contents::WINDOW,
            )?
            .fraction
            < 1.0
        {
            return Ok(false);
        }
        Ok(self
            .trace_mask(origin, below, mins, maxs, Contents::FLUID)?
            .fraction
            < 1.0)
    }
    fn stance(&mut self) -> Result<bool, TraceError> {
        if self.nq() || self.qw() {
            return Ok(false);
        }
        if self.parameters.stance.gib_bounds && self.player.movement.mode == MovementMode::Gib {
            self.dimensions();
            return Ok(false);
        }
        let old_ducked = self.player.movement.ducked;
        let policy = self.parameters.stance;
        if !policy.after_ground {
            self.dimensions();
        }
        let duck = self.command.buttons & buttons::CROUCH != 0
            || !policy.button_only && self.command.movement[2] < 0.0;
        if policy.dead_duck && self.player.movement.mode == MovementMode::Dead {
            self.player.movement.ducked = true;
        } else if duck
            && (!policy.grounded_duck
                || self.player.movement.grounded
                || policy.above_water
                    && self.player.movement.water_level <= 1
                    && !self.above_water()?)
            && !(policy.forbid_ladder && self.player.movement.ladder)
            && !(policy.forbid_n64 && self.player.movement.tuning.n64_physics)
        {
            if !old_ducked {
                let allowed = if policy.check_duck_hull {
                    let mut maxs = self.player.body.maxs;
                    maxs.0[2] = 4.0;
                    let p = self.player.body.position;
                    !self
                        .trace_bounds(p, p, self.player.body.mins, maxs)?
                        .all_solid
                } else {
                    true
                };
                self.player.movement.ducked = allowed;
            }
        } else if old_ducked {
            let mut maxs = self.player.body.maxs;
            maxs.0[2] = 32.0;
            let p = self.player.body.position;
            if !self
                .trace_bounds(p, p, self.player.body.mins, maxs)?
                .all_solid
            {
                self.player.movement.ducked = false;
            }
        }
        let changed = old_ducked != self.player.movement.ducked;
        if changed {
            self.dimensions();
        }
        Ok(changed)
    }
    fn friction(&mut self, context: FrictionContext) -> Result<(), TraceError> {
        let free = match context {
            FrictionContext::Native | FrictionContext::Free(FreeFriction::Native) => None,
            FrictionContext::Free(policy) => Some(policy),
        };
        let stop_controlled = matches!(free, Some(FreeFriction::StopControlled));
        let environment = free.is_none() || matches!(free, Some(FreeFriction::Flight(_)));
        if environment
            && (self.nq() && !self.player.movement.grounded
                || self.qw() && self.player.movement.water_jump_seconds != 0.0)
        {
            return Ok(());
        }
        let mut vector = self.player.body.velocity;
        if environment && self.arena() && self.player.movement.grounded {
            vector.0[2] = 0.0;
        }
        if environment && self.nq() {
            vector.0[2] = 0.0;
        }
        let speed = math::length(vector);
        if speed < (if self.nq() { 0.0 } else { 1.0 }) || speed == 0.0 {
            if stop_controlled {
                self.player.body.velocity = Vec3::default();
            } else if !self.nq() {
                self.player.body.velocity.0[0] = 0.0;
                self.player.body.velocity.0[1] = 0.0;
            }
            return Ok(());
        }
        let mut friction = self.parameters.friction;
        if stop_controlled {
            friction *= 1.5;
        }
        if environment && (self.nq() || self.qw()) && self.player.movement.grounded {
            let mut start = self.player.body.position;
            start.0[0] += self.player.body.velocity.0[0] / speed * 16.0;
            start.0[1] += self.player.body.velocity.0[1] / speed * 16.0;
            start.0[2] += self.player.body.mins.0[2];
            let mut end = start;
            end.0[2] -= 34.0;
            let tr = self.trace_bounds(
                start,
                end,
                if self.nq() {
                    Vec3::default()
                } else {
                    self.player.body.mins
                },
                if self.nq() {
                    Vec3::default()
                } else {
                    self.player.body.maxs
                },
            )?;
            if tr.fraction == 1.0 {
                friction *= 2.0;
            }
        }
        let ground = self.player.movement.grounded
            && !self
                .player
                .movement
                .ground_surface
                .contains(SurfaceFlags::SLICK)
            && (!self.arena()
                || !self
                    .player
                    .movement
                    .timer
                    .contains(MovementTimer::KNOCKBACK));
        let mut drop = 0.0;
        if stop_controlled
            || environment
                && ((ground && (!self.arena() || self.player.movement.water_level <= 1))
                    || !self.arena() && self.player.movement.ladder)
        {
            drop += speed.max(self.parameters.stop) * friction * self.dt;
        }
        let water = self.player.movement.water_level;
        if environment && self.nq() {
            let new_speed = (f64::from(speed)
                - self.exact_dt * f64::from(speed.max(self.parameters.stop)) * f64::from(friction))
                as f32;
            self.player.body.velocity = self.player.body.velocity * (new_speed.max(0.0) / speed);
            return Ok(());
        } else if environment && self.qw() && water >= 2 {
            drop = speed * self.parameters.water_friction * f32::from(water) * self.dt;
        } else if environment && !self.qw() && water > 0 && !self.player.movement.ladder {
            drop += speed * self.parameters.water_friction * f32::from(water) * self.dt;
        }
        if let Some(FreeFriction::Linear(rate) | FreeFriction::Flight(rate)) = free {
            drop += speed * rate * self.dt;
        }
        self.player.body.velocity = self.player.body.velocity * ((speed - drop).max(0.0) / speed);
        Ok(())
    }
    fn jump(&mut self) -> bool {
        if self.nq() {
            return false;
        } // NetQuake PlayerPreThink/QuakeC owns jump impulse.
        if self.arena() && self.player.movement.water_level >= 2 {
            return false;
        }
        if self.parameters.landing.blocks_jump
            && self.player.movement.timer.contains(MovementTimer::LAND)
        {
            return false;
        }
        let pressed = self.command.buttons & buttons::JUMP != 0 || self.command.movement[2] >= 10.0;
        if !pressed {
            self.player.movement.jump_held = false;
            return false;
        }
        if self.player.movement.water_jump_seconds != 0.0 {
            self.player.movement.water_jump_seconds =
                (self.player.movement.water_jump_seconds - self.dt).max(0.0);
            return false;
        }
        if self.player.movement.jump_held
            || matches!(
                self.player.movement.mode,
                MovementMode::Dead | MovementMode::Gib
            )
        {
            return false;
        }
        if !self.arena() && self.player.movement.water_level >= 2 {
            self.player.movement.grounded = false;
            if !self.parameters.special.swim_jump_impulse {
                return false;
            }
            if self.player.body.velocity.0[2] <= -300.0 {
                return false;
            }
            let water = Contents(self.player.movement.water_contents);
            self.player.body.velocity.0[2] = if water == Contents::WATER {
                100.0
            } else if water == Contents::SLIME {
                80.0
            } else {
                50.0
            };
            return false;
        }
        if !self.player.movement.grounded {
            return false;
        }
        self.player.movement.jump_held = true;
        self.result.jump_sound |= self.parameters.landing.publish_jump;
        self.player.movement.grounded = false;
        self.player.movement.ground = None;
        self.ground_plane = false;
        self.player.body.velocity.0[2] = if self.arena() {
            270.0
        } else if self.qw() {
            self.player.body.velocity.0[2] + 270.0
        } else {
            (self.player.body.velocity.0[2] + 270.0).max(270.0)
        };
        true
    }
    fn wish(&mut self, water: bool, free: Option<FreeMovePolicy>) -> Result<Wish, TraceError> {
        let native_control =
            free.is_some_and(|policy| matches!(policy.wish, FreeWish::NativeControl));
        let full_basis = free.is_some_and(|policy| !matches!(policy.wish, FreeWish::Existing));
        let mut angles = if native_control {
            if let MovePhase::Control { body_angles, .. } = self.options.phase {
                body_angles
            } else {
                self.player.view_angles
            }
        } else {
            self.player.view_angles
        };
        if !full_basis && self.nq() && !water {
            angles.0[0] = -angles.0[0] / 3.0;
            angles.0[2] = 0.0;
        } else if !full_basis && (self.classic() || self.rr()) && !water {
            angles.0[0] = math::wrap_above_180(angles.0[0]);
            angles.0[0] /= 3.0;
        }
        let mut basis = if native_control {
            math::angle_vectors(angles)
        } else {
            self.view_basis(angles)
        };
        if !full_basis && (self.qw() || self.arena()) && !water {
            basis.forward.0[2] = 0.0;
            basis.right.0[2] = 0.0;
            if self.arena() && self.player.movement.grounded {
                basis.forward =
                    super::slide::clip(basis.forward, self.ground_normal, RuleSetId::Quake3);
                basis.right =
                    super::slide::clip(basis.right, self.ground_normal, RuleSetId::Quake3);
            }
            math::normalize(&mut basis.forward);
            math::normalize(&mut basis.right);
        }
        if free.is_some_and(|policy| matches!(policy.wish, FreeWish::Normalized)) {
            math::normalize(&mut basis.forward);
            math::normalize(&mut basis.right);
        }
        let mut move_axes = self.command.movement;
        let clock = if native_control {
            if let MovePhase::Control { clock, .. } = self.options.phase {
                clock.seconds()
            } else {
                f64::from(self.command.server_time_ms) * 0.001
            }
        } else {
            f64::from(self.command.server_time_ms) * 0.001
        };
        if self.nq() && clock < self.player.movement.teleport_hold_until && move_axes[0] < 0.0 {
            move_axes[0] = 0.0;
        }
        let button_vertical = free.is_some_and(|policy| policy.button_vertical);
        let swim_buttons = water && self.parameters.special.swim_buttons;
        if button_vertical || swim_buttons {
            move_axes[2] = 0.0;
        }
        if !native_control
            && !button_vertical
            && !swim_buttons
            && self.command.buttons & buttons::JUMP != 0
        {
            move_axes[2] = move_axes[2].max(if self.arena() { 127.0 } else { 200.0 });
        }
        if !native_control
            && !button_vertical
            && !swim_buttons
            && self.command.buttons & buttons::CROUCH != 0
        {
            move_axes[2] = move_axes[2].min(if self.arena() { -127.0 } else { -200.0 });
        }
        let total = math::length(Vec3(move_axes));
        let maximum = move_axes[0]
            .abs()
            .max(move_axes[1].abs())
            .max(move_axes[2].abs());
        let scale = if total > 0.0 {
            self.parameters.speed * maximum / (127.0 * total)
        } else {
            0.0
        };
        // PM_WaterMove scales each basis before multiplying by its command
        // component. Factoring scale out of the sum changes binary32 results.
        let scaled_basis = free.is_some_and(|policy| matches!(policy.wish, FreeWish::ScaledBasis));
        if self.arena() && (water || scaled_basis) && scale != 0.0 {
            basis.forward = basis.forward * scale;
            basis.right = basis.right * scale;
            move_axes[2] *= scale;
        }
        let mut wish = basis.forward * move_axes[0] + basis.right * move_axes[1];
        if native_control {
            wish.0[2] = move_axes[2];
        } else if water
            || matches!(
                self.player.movement.mode,
                MovementMode::Noclip | MovementMode::Fly | MovementMode::Spectator
            )
        {
            if !button_vertical && !swim_buttons {
                wish.0[2] += move_axes[2];
            }
            if button_vertical {
                if self.command.buttons & buttons::JUMP != 0 {
                    wish.0[2] += 200.0;
                }
                if self.command.buttons & buttons::CROUCH != 0 {
                    wish.0[2] -= 200.0;
                }
            }
            if swim_buttons {
                if move_axes[0] == 0.0
                    && move_axes[1] == 0.0
                    && self.command.buttons & (buttons::JUMP | buttons::CROUCH) == 0
                {
                    if self.parameters.special.sink_on_ground || !self.player.movement.grounded {
                        wish.0[2] -= 60.0;
                    }
                } else if self.command.buttons & buttons::CROUCH != 0 {
                    wish.0[2] -= 200.0;
                } else if self.command.buttons & buttons::JUMP != 0 {
                    wish.0[2] += 200.0;
                }
            } else if water && move_axes == [0.0; 3] {
                wish.0[2] -= 60.0;
            }
        } else {
            wish.0[2] = 0.0;
        }
        if free.is_none() {
            self.adjust_wish(&mut wish)?;
        }
        let raw = wish;
        let vertical = wish.0[2];
        let length = math::normalize(&mut wish);
        let max = if self.player.movement.ducked
            && !full_basis
            && !(water && self.parameters.special.water_standing_cap)
        {
            if self.arena() {
                self.parameters.speed * 0.25
            } else {
                100.0
            }
        } else if free.is_some_and(|policy| policy.spectator_speed) {
            self.parameters.spectator_speed
        } else {
            self.parameters.speed
        };
        let mut speed = length.min(max);
        if self.arena() {
            speed = if scaled_basis {
                length
            } else if full_basis {
                length * scale
            } else if water {
                length.min(self.parameters.speed * 0.5)
            } else {
                (length * scale).min(max)
            };
        }
        if water && !self.arena() {
            speed *= if self.nq() || self.qw() { 0.7 } else { 0.5 };
        }
        if water
            && self.player.movement.ducked
            && let Some(cap) = self.parameters.special.water_duck_cap_after
        {
            speed = speed.min(cap);
        }
        if let Some(policy) = free {
            speed *= policy.speed_multiplier;
        }
        Ok(Wish {
            direction: wish,
            speed,
            vertical,
            raw,
            length,
            maximum: max,
        })
    }
    fn walk(&mut self) -> Result<(), TraceError> {
        let water = self.player.movement.water_level >= 2;
        if self.nq() && water {
            self.quake_water()?;
            super::slide::step_slide(self, false)?;
            return Ok(());
        }
        if water
            && self
                .parameters
                .special
                .water_jump
                .is_some_and(|policy| matches!(policy.phase, ProbePhase::InWaterMovement))
        {
            self.check_water_jump()?;
            if self.water_jump_move()? {
                return Ok(());
            }
        }
        self.friction(FrictionContext::Native)?;
        let Wish {
            direction,
            speed,
            vertical,
            ..
        } = self.wish(water, None)?;
        if !water && self.player.movement.ladder {
            self.accelerate(direction, speed, self.parameters.accel, false);
            if vertical == 0.0 {
                let z = &mut self.player.body.velocity.0[2];
                if *z > 0.0 {
                    *z -= self.parameters.gravity * self.dt;
                    if *z < 0.0 {
                        *z = 0.0;
                    }
                } else {
                    *z += self.parameters.gravity * self.dt;
                    if *z > 0.0 {
                        *z = 0.0;
                    }
                }
            }
            super::slide::step_slide(self, false)?;
            return Ok(());
        }
        let ground = self.player.movement.grounded;
        let accel = if water {
            self.parameters.water_accel
        } else if ground {
            if self.arena()
                && self
                    .player
                    .movement
                    .ground_surface
                    .contains(SurfaceFlags::SLICK)
                || self.arena()
                    && self
                        .player
                        .movement
                        .timer
                        .contains(MovementTimer::KNOCKBACK)
            {
                self.parameters.air
            } else {
                self.parameters.accel
            }
        } else {
            self.parameters.air
        };
        self.accelerate(
            direction,
            speed,
            accel,
            !ground && !water && self.parameters.capped_air,
        );
        if self.arena() && self.ground_plane {
            if ground
                && !water
                && (self
                    .player
                    .movement
                    .ground_surface
                    .contains(SurfaceFlags::SLICK)
                    || self
                        .player
                        .movement
                        .timer
                        .contains(MovementTimer::KNOCKBACK))
            {
                self.player.body.velocity.0[2] -= self.parameters.gravity * self.dt;
            }
            let velocity = self.player.body.velocity;
            if !water || velocity.dot(self.ground_normal) < 0.0 {
                self.player.body.velocity =
                    super::slide::clip(velocity, self.ground_normal, RuleSetId::Quake3);
                if ground || water {
                    math::normalize(&mut self.player.body.velocity);
                    self.player.body.velocity = self.player.body.velocity * math::length(velocity);
                }
            }
        }
        let gravity = !water
            && (self.nq()
                || self.qw()
                || !ground
                || self.arena()
                    && self
                        .player
                        .movement
                        .ground_surface
                        .contains(SurfaceFlags::SLICK));
        if ground && !self.arena() && !self.nq() {
            self.player.body.velocity.0[2] = 0.0;
        }
        if gravity && !self.arena() {
            self.player.body.velocity.0[2] = if self.nq() {
                (f64::from(self.player.body.velocity.0[2])
                    - f64::from(self.parameters.gravity) * self.exact_dt) as f32
            } else {
                self.player.body.velocity.0[2] - self.parameters.gravity * self.dt
            };
        }
        if self.arena() && water {
            super::slide::slide(self, false)?;
        } else {
            super::slide::step_slide(self, self.arena() && !ground)?;
        }
        Ok(())
    }
    fn quake_water(&mut self) -> Result<(), TraceError> {
        let Wish {
            direction,
            speed: wish_speed,
            ..
        } = self.wish(true, None)?;
        let speed = math::length(self.player.body.velocity);
        let new_speed = if speed == 0.0 {
            0.0
        } else {
            let next = (f64::from(speed)
                - self.exact_dt * f64::from(speed) * f64::from(self.parameters.friction))
                as f32;
            let next = next.max(0.0);
            self.player.body.velocity = self.player.body.velocity * (next / speed);
            next
        };
        let add = wish_speed - new_speed;
        if add > 0.0 {
            let amount = (f64::from(self.parameters.accel * wish_speed) * self.exact_dt) as f32;
            self.player.body.velocity = self.player.body.velocity + direction * amount.min(add);
        }
        Ok(())
    }
    fn snap(&mut self, previous: Vec3, test_position: bool) -> Result<(), TraceError> {
        if self.arena() {
            self.player.body.velocity = math::round_vector(self.player.body.velocity);
            return Ok(());
        }
        if !self.classic() {
            if self.parameters.special.recover_stuck
                && self.player.movement.mode != MovementMode::Noclip
            {
                let position = self.player.body.position;
                if self.trace(position, position)?.all_solid {
                    self.player.body.position =
                        self.recover_position(position)?.unwrap_or(previous);
                }
            }
            return Ok(());
        }
        self.player.body.velocity.0 = self.player.body.velocity.0.map(math::narrow_eighth);
        let position = self.player.body.position;
        let base = Vec3(position.0.map(math::narrow_eighth));
        // Native Q2 PM_GoodPosition accepts spectators without tracing.
        if !test_position {
            self.player.body.position = base;
            return Ok(());
        }
        let signs = std::array::from_fn::<_, 3, _>(|i| {
            if base.0[i] == position.0[i] {
                0.0
            } else if position.0[i] >= 0.0 {
                1.0
            } else {
                -1.0
            }
        });
        for bits in [0, 4, 1, 2, 3, 5, 6, 7] {
            let next = Vec3(std::array::from_fn(|i| {
                math::narrow_eighth(
                    base.0[i]
                        + if bits & (1 << i) != 0 {
                            signs[i] * 0.125
                        } else {
                            0.0
                        },
                )
            }));
            let tr = self.trace(next, next)?;
            if !tr.all_solid {
                self.player.body.position = next;
                return Ok(());
            }
        }
        self.player.body.position = previous;
        Ok(())
    }
    pub(crate) fn clear_timer_flags(&mut self, mask: MovementTimer) {
        self.player.movement.timer.0 &= !mask.0;
        self.player.movement.remaining_ms = 0;
    }
    pub(crate) fn start_timer(&mut self, flag: MovementTimer, duration: u32) {
        self.player.movement.timer.insert(flag);
        self.player.movement.remaining_ms = duration;
    }
    fn drop_timers(&mut self) {
        if self.player.movement.remaining_ms == 0 {
            return;
        }
        let elapsed = if self.classic() {
            u32::from((self.command.duration_ms >> 3).max(1)) * 8
        } else {
            u32::from(self.command.duration_ms)
        };
        self.player.movement.remaining_ms =
            self.player.movement.remaining_ms.saturating_sub(elapsed);
        if self.player.movement.remaining_ms == 0 {
            self.player.movement.timer = MovementTimer::NONE;
        }
    }
    fn run_free<E, B>(
        &mut self,
        policy: FreeMovePolicy,
        previous: Vec3,
        boundary: &mut B,
    ) -> Result<(), E>
    where
        E: From<TraceError>,
        B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
    {
        let mut synchronized = (self.player.body.position, self.player.body.velocity);
        if policy.reset_flags {
            self.player.movement.timer = MovementTimer::NONE;
            self.player.movement.ducked = false;
            self.player.movement.jump_held = false;
            self.player.movement.grounded = false;
            self.player.movement.ground = None;
            self.player.movement.ladder = false;
        }
        if policy.clear_ground_reference {
            self.player.movement.ground = None;
        }
        if policy.exclude_body_contents {
            self.mask.0 &= !(Contents::BODY | Contents::PLAYER).0;
        }
        if policy.clear_water_outputs {
            self.player.movement.water_level = 0;
            self.player.movement.water_contents = 0;
        }
        if let Some(bounds) = policy.bounds {
            self.player.body.mins = bounds.mins;
            self.player.body.maxs = bounds.maxs;
        }
        if policy.world_only {
            self.scope = TraceScope::World;
            self.mask = Contents::SOLID | Contents::WINDOW;
        }
        if policy.stance {
            self.stance()?;
        } else {
            self.player.view_offset.0[2] =
                policy.view_height.unwrap_or(self.parameters.view_height);
        }
        if matches!(policy.finish, FreeFinish::Environment) {
            self.categorize()?;
            self.drop_timers();
        }
        if !matches!(policy.acceleration, FreeAcceleration::Immediate) {
            self.friction(FrictionContext::Free(policy.friction))?;
        }
        let wish = self.wish(false, Some(policy))?;
        let accelerated = match policy.acceleration {
            FreeAcceleration::Immediate => {
                self.player.body.velocity = wish.immediate_velocity();
                true
            }
            FreeAcceleration::Normal | FreeAcceleration::Flight => self.accelerate(
                wish.direction,
                wish.speed,
                if matches!(policy.acceleration, FreeAcceleration::Flight) {
                    self.parameters.fly_accel
                } else {
                    self.parameters.accel
                },
                false,
            ),
        };
        // Q2/QW spectator acceleration returns before position integration
        // when addspeed <= 0. Rerelease and Q3 still integrate their velocity.
        if accelerated || !policy.abort_without_acceleration {
            match policy.motion {
                FreeMotion::Direct => {
                    let velocity = self.player.body.velocity;
                    copy_body(self.player, None, velocity, None, boundary)?;
                    direct_motion(self.player, self.dt, boundary)?;
                    synchronized = (self.player.body.position, self.player.body.velocity);
                }
                FreeMotion::StepSlide => super::slide::step_slide(self, false)?,
            }
        }
        match policy.finish {
            FreeFinish::Snap(test_position) => self.snap(previous, test_position)?,
            // Q3 spectator/noclip return before the ordinary SnapVector call.
            FreeFinish::Timers => self.drop_timers(),
            FreeFinish::Environment => {
                self.categorize()?;
                self.snap(previous, true)?;
                self.player.movement.previous_position = previous;
            }
        }
        let position = self.player.body.position;
        let velocity = self.player.body.velocity;
        copy_body(
            self.player,
            Some(position),
            velocity,
            Some(synchronized),
            boundary,
        )?;
        Ok(())
    }
    fn run<E, B>(&mut self, boundary: &mut B) -> Result<(), E>
    where
        E: From<TraceError>,
        B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
    {
        self.result.steps += 1;
        if let Some(exclusive) = self.parameters.exclusive_button
            && self.command.buttons & exclusive != 0
        {
            self.command.buttons = exclusive;
            self.command.movement = [0.0; 3];
        }
        let mut previous = self.player.body.position;
        if !self.options.keep_view_angles {
            update_view_angles(self.command, self.player, self.parameters.angles);
        }
        if self.parameters.special.ladder.is_none() {
            self.player.movement.ladder = false;
        }
        if self.command.buttons & buttons::JUMP == 0
            && self
                .parameters
                .special
                .release_jump_below
                .is_some_and(|threshold| self.command.movement[2] < threshold)
        {
            self.player.movement.jump_held = false;
        }
        if !self.player.movement.mode.allows_movement() {
            return Ok(());
        }
        if let Some(policy) = self.parameters.free.get(self.player.movement.mode) {
            self.run_free(policy, previous, boundary)?;
            return Ok(());
        }
        let stance_after_ground = self.parameters.stance.after_ground;
        if !stance_after_ground {
            self.stance()?;
            if self.options.initial_snap
                && let Some(scale) = self.parameters.stance.initial_snap
            {
                previous = self.initial_snap(previous, scale)?;
            }
        } else {
            self.dimensions();
        }
        if matches!(
            self.player.movement.mode,
            MovementMode::Dead | MovementMode::Gib
        ) {
            self.command.movement = [0.0; 3];
            self.command.buttons &= self.parameters.exclusive_button.unwrap_or(0);
        }
        if self.nq() {
            self.water_level()?;
        } else {
            self.categorize()?;
        }
        if stance_after_ground {
            if self.options.initial_snap
                && let Some(scale) = self.parameters.stance.initial_snap
            {
                previous = self.initial_snap(previous, scale)?;
            }
            if self.stance()? {
                self.categorize()?;
            }
        }
        self.dead_friction();
        self.check_special()?;
        self.drop_timers();
        if !self.player.movement.timer.contains(MovementTimer::TELEPORT)
            && !self.water_jump_move()?
        {
            self.jump();
            self.walk()?;
        }
        if !self.nq() {
            self.categorize()?;
        }
        if self.parameters.landing.recheck_jump
            && self.player.movement.timer.contains(MovementTimer::TRICK)
        {
            self.jump();
        }
        self.snap(previous, true)?;
        self.player.movement.previous_position = previous;
        Ok(())
    }
}
fn run_step<E, B>(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
    parameters: Parameters,
    result: &mut MovementResult,
    options: MoveOptions,
    boundary: &mut B,
) -> Result<(), E>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    let exact_dt = if parameters.rules == RuleSetId::Quake {
        EventTime(command.duration_ns).seconds()
    } else {
        f64::from(command.duration_ms) * 0.001
    };
    let dt = if parameters.rules == RuleSetId::Quake2Rerelease {
        f32::from(command.duration_ms) * 0.001
    } else {
        exact_dt as f32
    };
    let mask = Contents::SOLID
        | Contents::WINDOW
        | Contents::PLAYER_CLIP
        | if matches!(player.movement.mode, MovementMode::Dead | MovementMode::Gib) {
            Contents::EMPTY
        } else {
            Contents::BODY | Contents::PLAYER
        };
    let previous_velocity = player.body.velocity;
    Step {
        player,
        world,
        result,
        parameters,
        command,
        dt,
        exact_dt,
        ground_plane: false,
        ground_normal: Vec3::default(),
        mask,
        scope: TraceScope::Scene,
        previous_velocity,
        options,
    }
    .run(boundary)?;
    Ok(())
}
fn run_entry<E, B>(
    rules: RuleSetId,
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
    options: MoveOptions,
    boundary: &mut B,
    run: impl FnOnce(
        UserCmd,
        &mut PlayerState,
        &mut dyn TraceServices,
        Parameters,
        MoveOptions,
        &mut B,
    ) -> Result<MovementResult, E>,
) -> Result<MovementResult, E>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    with_rollback(player, options.phase, |player| {
        let parameters = Parameters::load(rules, player);
        match options.phase {
            MovePhase::Control { .. } => {
                let mut result = MovementResult::default();
                if let Some(policy) = parameters.control
                    && player.movement.mode.allows_movement()
                {
                    let previous_velocity = player.body.velocity;
                    let mut step = Step {
                        player,
                        world,
                        result: &mut result,
                        parameters,
                        command,
                        dt: 0.0,
                        exact_dt: 0.0,
                        ground_plane: false,
                        ground_normal: Vec3::default(),
                        mask: Contents::EMPTY,
                        scope: TraceScope::Scene,
                        previous_velocity,
                        options,
                    };
                    let wish = step.wish(false, Some(policy))?;
                    let velocity = wish.immediate_velocity();
                    copy_body(step.player, None, velocity, None, boundary)?;
                }
                Ok(result)
            }
            MovePhase::Integrate { seconds } if parameters.control.is_some() => {
                if !player.movement.mode.allows_movement() {
                    return Ok(MovementResult::default());
                }
                direct_motion(player, seconds as f32, boundary)?;
                Ok(MovementResult {
                    steps: 1,
                    ..Default::default()
                })
            }
            MovePhase::Integrate { seconds } => {
                // NQ commands have no native msec or absolute Q3 command clock.
                // Foreign movement consumes the admitted provider duration,
                // then retains its own command subdivision and width policy.
                let mut command = command;
                command.duration_ms = (seconds * 1000.0) as u16;
                command.duration_ns =
                    EventTime::from_milliseconds(u64::from(command.duration_ms)).0;
                if parameters.relative_command_clock {
                    command.server_time_ms = player
                        .movement
                        .command_time_ms
                        .saturating_add(i32::from(command.duration_ms));
                }
                run(command, player, world, parameters, options, boundary)
            }
            MovePhase::Complete => run(command, player, world, parameters, options, boundary),
        }
    })
}
fn quake<E, B>(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
    options: MoveOptions,
    boundary: &mut B,
) -> Result<MovementResult, E>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    run_entry(
        RuleSetId::Quake,
        command,
        player,
        world,
        options,
        boundary,
        |command, player, world, parameters, options, boundary| {
            let command = super::prepare_command(RuleSetId::Quake, command);
            let mut result = MovementResult::default();
            run_step(
                command,
                player,
                world,
                parameters,
                &mut result,
                options,
                boundary,
            )?;
            Ok(result)
        },
    )
}
fn quakeworld<E, B>(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
    options: MoveOptions,
    boundary: &mut B,
) -> Result<MovementResult, E>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    run_entry(
        RuleSetId::QuakeWorld,
        command,
        player,
        world,
        options,
        boundary,
        |command, player, world, parameters, options, boundary| {
            let commands = super::quakeworld_steps(command);
            let mut result = MovementResult::default();
            for command in commands {
                run_step(
                    command,
                    player,
                    world,
                    parameters,
                    &mut result,
                    options,
                    boundary,
                )?;
            }
            Ok(result)
        },
    )
}
fn quake2<E, B>(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
    options: MoveOptions,
    boundary: &mut B,
) -> Result<MovementResult, E>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    run_entry(
        RuleSetId::Quake2,
        command,
        player,
        world,
        options,
        boundary,
        |command, player, world, parameters, options, boundary| {
            // Native pmove_state_t carries signed 16-bit coordinates and velocities
            // in eighth units. Rerelease deliberately keeps its floating ABI.
            player.body.position.0 = player.body.position.0.map(math::narrow_eighth);
            player.body.velocity.0 = player.body.velocity.0.map(math::narrow_eighth);
            let command = super::prepare_command(RuleSetId::Quake2, command);
            let mut result = MovementResult::default();
            run_step(
                command,
                player,
                world,
                parameters,
                &mut result,
                options,
                boundary,
            )?;
            Ok(result)
        },
    )
}
fn rerelease<E, B>(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
    options: MoveOptions,
    boundary: &mut B,
) -> Result<MovementResult, E>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    run_entry(
        RuleSetId::Quake2Rerelease,
        command,
        player,
        world,
        options,
        boundary,
        |command, player, world, parameters, options, boundary| {
            let command = super::prepare_command(RuleSetId::Quake2Rerelease, command);
            let mut result = MovementResult::default();
            run_step(
                command,
                player,
                world,
                parameters,
                &mut result,
                options,
                boundary,
            )?;
            Ok(result)
        },
    )
}
fn arena<E, B>(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
    options: MoveOptions,
    boundary: &mut B,
) -> Result<MovementResult, E>
where
    E: From<TraceError>,
    B: FnMut(&mut PlayerState, MotionWrite) -> Result<(), E>,
{
    run_entry(
        RuleSetId::Quake3,
        command,
        player,
        world,
        options,
        boundary,
        |mut command, player, world, parameters, options, boundary| {
            let mut result = MovementResult::default();
            let end = command.server_time_ms;
            if end < player.movement.command_time_ms {
                return Ok(result);
            }
            if i64::from(end) > i64::from(player.movement.command_time_ms) + 1000 {
                player.movement.command_time_ms = end - 1000;
            }
            let fixed = player.movement.tuning.fixed_step_ms;
            let maximum = if fixed == 0 { 66 } else { fixed.clamp(1, 200) };
            while player.movement.command_time_ms < end {
                let ms = (i64::from(end) - i64::from(player.movement.command_time_ms))
                    .min(i64::from(maximum)) as u16;
                command.duration_ms = ms;
                player.movement.command_time_ms += i32::from(ms);
                command.server_time_ms = player.movement.command_time_ms;
                run_step(
                    command,
                    player,
                    world,
                    parameters,
                    &mut result,
                    options,
                    boundary,
                )?;
                if player.movement.jump_held {
                    command.movement[2] = 20.0;
                }
            }
            Ok(result)
        },
    )
}
