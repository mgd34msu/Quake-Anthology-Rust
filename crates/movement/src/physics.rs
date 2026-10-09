use qa_core::{
    math,
    primitives::{
        EntityId, MovementMode, MovementTimer, PlayerState, RuleSetId, SurfaceFlags, UserCmd, Vec3,
        buttons,
    },
};
use qa_world::collision::{Contents, EntityTraceRules, Trace, TraceQuery, TraceRules, WorldTrace};

pub trait TraceServices {
    fn trace(&mut self, query: TraceQuery) -> Trace;
    fn point_contents(&self, point: Vec3, rules: EntityTraceRules) -> Contents;
}
impl TraceServices for WorldTrace<'_> {
    fn trace(&mut self, query: TraceQuery) -> Trace {
        WorldTrace::trace(self, query)
    }
    fn point_contents(&self, point: Vec3, rules: EntityTraceRules) -> Contents {
        WorldTrace::point_contents(self, point, rules, &[])
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MovementResult {
    pub steps: u32,
    pub traces: u32,
    pub contacts: [Option<EntityId>; 32],
    pub contact_count: usize,
}
pub type MoveEntry = fn(UserCmd, &mut PlayerState, &mut dyn TraceServices) -> MovementResult;
pub fn entry(rules: RuleSetId) -> MoveEntry {
    match rules {
        RuleSetId::Quake => quake,
        RuleSetId::QuakeWorld => quakeworld,
        RuleSetId::Quake2 => quake2,
        RuleSetId::Quake2Rerelease => rerelease,
        RuleSetId::Quake3 => arena,
    }
}
pub fn pmove(
    command: UserCmd,
    player: &mut PlayerState,
    trace: &mut dyn TraceServices,
) -> MovementResult {
    entry(player.movement_rules)(command, player, trace)
}

/// Spawn/stance initialization, independent of the module or map family.
pub fn set_bounds(player: &mut PlayerState) {
    let radius = if player.movement_rules == RuleSetId::Quake3 {
        15.0
    } else {
        16.0
    };
    player.body.mins = Vec3([-radius, -radius, -24.0]);
    player.body.maxs = Vec3([radius, radius, 32.0]);
    player.view_offset = Vec3([
        0.0,
        0.0,
        if player.movement_rules == RuleSetId::Quake3 {
            26.0
        } else {
            22.0
        },
    ]);
}
#[derive(Clone, Copy)]
pub(crate) struct Parameters {
    pub rules: RuleSetId,
    pub trace_rules: TraceRules,
    pub entity_rules: EntityTraceRules,
    pub gravity: f32,
    pub speed: f32,
    pub friction: f32,
    pub stop: f32,
    pub accel: f32,
    pub air: f32,
    pub water_accel: f32,
    pub water_friction: f32,
    pub no_step: bool,
}
impl Parameters {
    fn load(rules: RuleSetId, player: &PlayerState) -> Self {
        let t = player.movement.tuning;
        let legacy = matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld);
        let arena = rules == RuleSetId::Quake3;
        Self {
            rules,
            trace_rules: if arena {
                TraceRules::ARENA
            } else {
                TraceRules::LEGACY
            },
            entity_rules: if legacy {
                EntityTraceRules::QUAKE
            } else if arena {
                EntityTraceRules::ARENA
            } else {
                EntityTraceRules::QUAKE2
            },
            gravity: t.gravity.unwrap_or(800.0) * t.gravity_multiplier,
            speed: t
                .max_speed
                .unwrap_or(if legacy || arena { 320.0 } else { 300.0 })
                * t.speed_multiplier,
            friction: t.friction.unwrap_or(if legacy { 4.0 } else { 6.0 }),
            stop: t.stop_speed.unwrap_or(100.0),
            accel: t.accelerate.unwrap_or(10.0),
            air: t.air_accelerate.unwrap_or(if arena { 1.0 } else { 0.0 }),
            water_accel: t.water_accelerate.unwrap_or(if arena { 4.0 } else { 10.0 }),
            water_friction: t.water_friction.unwrap_or(if arena || legacy {
                if rules == RuleSetId::Quake { 4.0 } else { 1.0 }
            } else {
                1.0
            }),
            no_step: t.no_step,
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
    pub previous_velocity: Vec3,
}
impl Step<'_> {
    pub fn trace(&mut self, start: Vec3, end: Vec3) -> Trace {
        self.trace_bounds(start, end, self.player.body.mins, self.player.body.maxs)
    }
    fn trace_bounds(&mut self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> Trace {
        self.result.traces += 1;
        self.world.trace(TraceQuery {
            start,
            end,
            mins,
            maxs,
            mask: self.mask,
            rules: self.parameters.trace_rules,
            entity_rules: self.parameters.entity_rules,
            pass: None,
            excluded: &[],
        })
    }
    pub fn contact(&mut self, trace: &Trace) {
        if let Some(id) = trace.entity
            && self.result.contact_count < 32
            && (!self.arena()
                || !self.result.contacts[..self.result.contact_count].contains(&Some(id)))
        {
            self.result.contacts[self.result.contact_count] = Some(id);
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
    pub fn accelerate(&mut self, direction: Vec3, speed: f32, accel: f32, air_cap: bool) {
        let target = if air_cap { speed.min(30.0) } else { speed };
        let add = target - self.player.body.velocity.dot(direction);
        if add <= 0.0 {
            return;
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
    }
    fn water_level(&mut self) {
        let body = self.player.body;
        let mut point = body.position;
        point.0[2] += body.mins.0[2] + 1.0;
        self.player.movement.water_level = 0;
        self.player.movement.water_contents = 0;
        let contents = self
            .world
            .point_contents(point, self.parameters.entity_rules);
        if !contents.intersects(Contents::FLUID) {
            return;
        }
        self.player.movement.water_level = 1;
        self.player.movement.water_contents = contents.0;
        let middle = if self.nq() || self.qw() {
            (body.mins.0[2] + body.maxs.0[2]) * 0.5
        } else {
            body.mins.0[2] + ((self.player.view_offset.0[2] - body.mins.0[2]) as i32 / 2) as f32
        };
        point.0[2] = body.position.0[2] + middle;
        if self
            .world
            .point_contents(point, self.parameters.entity_rules)
            .intersects(Contents::FLUID)
        {
            self.player.movement.water_level = 2;
            point.0[2] = body.position.0[2] + self.player.view_offset.0[2];
            if self
                .world
                .point_contents(point, self.parameters.entity_rules)
                .intersects(Contents::FLUID)
            {
                self.player.movement.water_level = 3;
            }
        }
    }
    fn categorize(&mut self) {
        let was_grounded = self.player.movement.grounded;
        self.player.movement.grounded = false;
        self.player.movement.ground = None;
        self.ground_plane = false;
        self.ground_normal = Vec3::default();
        if self.player.body.velocity.0[2] <= 180.0 || self.arena() {
            let start = self.player.body.position;
            let drop = if self.qw() { 1.0 } else { 0.25 };
            let mut end = start;
            end.0[2] -= drop;
            let tr = self.trace(start, end);
            self.contact(&tr);
            let contact = tr.fraction < 1.0 || tr.start_solid;
            self.ground_plane = contact;
            self.ground_normal = tr.plane.normal;
            let kicked = self.arena()
                && self.player.body.velocity.0[2] > 0.0
                && self.player.body.velocity.dot(tr.plane.normal) > 10.0;
            if contact
                && !kicked
                && (tr.plane.normal.0[2] >= 0.7 || tr.start_solid && !self.arena() && !self.qw())
            {
                self.player.movement.grounded = true;
                self.player.movement.ground = tr.entity;
                self.player.movement.ground_normal = tr.plane.normal;
                self.player.movement.ground_surface = tr.surface;
                if self.classic() && !was_grounded && self.player.body.velocity.0[2] < -200.0 {
                    self.player.movement.timer.insert(MovementTimer::LAND);
                    self.player.movement.remaining_ms = if self.player.body.velocity.0[2] < -400.0 {
                        200
                    } else {
                        144
                    };
                }
                if self.arena() && !was_grounded && self.previous_velocity.0[2] < -200.0 {
                    self.player.movement.timer.insert(MovementTimer::LAND);
                    self.player.movement.remaining_ms = 250;
                }
                if self.qw() && !tr.start_solid && !tr.all_solid {
                    self.player.body.position = tr.end;
                }
                if self
                    .player
                    .movement
                    .timer
                    .contains(MovementTimer::WATER_JUMP)
                {
                    self.player.movement.timer = MovementTimer::NONE;
                    self.player.movement.remaining_ms = 0;
                }
                if self.qw() {
                    self.player.movement.water_jump_seconds = 0.0;
                }
            }
        }
        self.water_level();
    }
    fn stance(&mut self) {
        if self.nq() || self.qw() {
            return;
        }
        if self.classic() && self.player.movement.mode == MovementMode::Gib {
            self.player.body.mins.0[2] = 0.0;
            self.player.body.maxs.0[2] = 16.0;
            self.player.view_offset.0[2] = 8.0;
            return;
        }
        self.player.body.mins.0[2] = -24.0;
        let old_ducked = self.player.movement.ducked;
        let duck = self.command.movement[2] < 0 || self.command.buttons & buttons::CROUCH != 0;
        if duck && (!self.classic() || self.player.movement.grounded)
            || self.classic() && self.player.movement.mode == MovementMode::Dead
        {
            self.player.movement.ducked = true;
        } else if old_ducked {
            let mut maxs = self.player.body.maxs;
            maxs.0[2] = 32.0;
            let p = self.player.body.position;
            let tr = self.trace_bounds(p, p, self.player.body.mins, maxs);
            if !tr.all_solid {
                self.player.movement.ducked = false;
            }
        }
        let dead = self.arena()
            && matches!(
                self.player.movement.mode,
                MovementMode::Dead | MovementMode::Gib
            );
        self.player.body.maxs.0[2] = if dead {
            -8.0
        } else if self.player.movement.ducked {
            if self.arena() { 16.0 } else { 4.0 }
        } else {
            32.0
        };
        self.player.view_offset.0[2] = if dead {
            -16.0
        } else if self.player.movement.ducked {
            if self.arena() { 12.0 } else { -2.0 }
        } else if self.arena() {
            26.0
        } else {
            22.0
        };
    }
    fn friction(&mut self) {
        if self.nq() && !self.player.movement.grounded
            || self.qw() && self.player.movement.water_jump_seconds != 0.0
        {
            return;
        }
        let mut vector = self.player.body.velocity;
        if self.arena() && self.player.movement.grounded {
            vector.0[2] = 0.0;
        }
        if self.nq() {
            vector.0[2] = 0.0;
        }
        let speed = math::length(vector);
        if speed < (if self.nq() { 0.0 } else { 1.0 }) || speed == 0.0 {
            if !self.nq() {
                self.player.body.velocity.0[0] = 0.0;
                self.player.body.velocity.0[1] = 0.0;
            }
            return;
        }
        let mut friction = self.parameters.friction;
        if (self.nq() || self.qw()) && self.player.movement.grounded {
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
            );
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
        if (ground && (!self.arena() || self.player.movement.water_level <= 1))
            || !self.arena() && self.player.movement.ladder
        {
            drop += speed.max(self.parameters.stop) * friction * self.dt;
        }
        let water = self.player.movement.water_level;
        if self.nq() {
            let new_speed = (f64::from(speed)
                - self.exact_dt * f64::from(speed.max(self.parameters.stop)) * f64::from(friction))
                as f32;
            self.player.body.velocity = self.player.body.velocity * (new_speed.max(0.0) / speed);
            return;
        } else if self.qw() && water >= 2 {
            drop = speed * self.parameters.water_friction * f32::from(water) * self.dt;
        } else if !self.qw() && water > 0 {
            drop += speed * self.parameters.water_friction * f32::from(water) * self.dt;
        }
        self.player.body.velocity = self.player.body.velocity * ((speed - drop).max(0.0) / speed);
    }
    fn jump(&mut self) -> bool {
        if self.nq() {
            return false;
        } // NetQuake PlayerPreThink/QuakeC owns jump impulse.
        if self.arena() && self.player.movement.water_level >= 2 {
            return false;
        }
        let pressed = self.command.buttons & buttons::JUMP != 0 || self.command.movement[2] >= 10;
        if !pressed {
            self.player.movement.jump_held = false;
            return false;
        }
        if !self.arena() && self.player.movement.timer.contains(MovementTimer::LAND) {
            return false;
        }
        if self.player.movement.jump_held
            || matches!(
                self.player.movement.mode,
                MovementMode::Dead | MovementMode::Gib
            )
            || !self.arena() && self.player.movement.timer.contains(MovementTimer::LAND)
        {
            return false;
        }
        if !self.arena() && self.player.movement.water_level >= 2 {
            self.player.movement.grounded = false;
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
    fn wish(&self, water: bool) -> (Vec3, f32) {
        let mut angles = self.player.view_angles;
        if self.nq() && !water {
            angles.0[0] = -angles.0[0] / 3.0;
            angles.0[2] = 0.0;
        } else if (self.classic() || self.rr()) && !water {
            if angles.0[0] > 180.0 {
                angles.0[0] -= 360.0;
            }
            angles.0[0] /= 3.0;
        }
        let mut basis = if self.arena() {
            math::angle_vectors_radians(math::radians_from_degrees_f32(angles))
        } else {
            math::angle_vectors(angles)
        };
        if (self.qw() || self.arena()) && !water {
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
        let mut move_axes = self.command.movement.map(f32::from);
        if self.nq()
            && f64::from(self.command.server_time_ms) * 0.001
                < self.player.movement.teleport_hold_until
            && move_axes[0] < 0.0
        {
            move_axes[0] = 0.0;
        }
        if self.command.buttons & buttons::JUMP != 0 {
            move_axes[2] = move_axes[2].max(if self.arena() { 127.0 } else { 200.0 });
        }
        if self.command.buttons & buttons::CROUCH != 0 {
            move_axes[2] = move_axes[2].min(if self.arena() { -127.0 } else { -200.0 });
        }
        let mut wish = basis.forward * move_axes[0] + basis.right * move_axes[1];
        if water
            || matches!(
                self.player.movement.mode,
                MovementMode::Noclip | MovementMode::Fly | MovementMode::Spectator
            )
        {
            wish.0[2] += move_axes[2];
            if water && move_axes == [0.0; 3] {
                wish.0[2] -= 60.0;
            }
        } else {
            wish.0[2] = 0.0;
        }
        let total = (move_axes[0] * move_axes[0]
            + move_axes[1] * move_axes[1]
            + move_axes[2] * move_axes[2])
            .sqrt();
        let maximum = move_axes[0]
            .abs()
            .max(move_axes[1].abs())
            .max(move_axes[2].abs());
        let scale = if total > 0.0 {
            self.parameters.speed * maximum / (127.0 * total)
        } else {
            0.0
        };
        if self.arena() && water && scale != 0.0 {
            wish = wish * scale;
        }
        let length = math::normalize(&mut wish);
        let max = if self.player.movement.ducked {
            if self.arena() {
                self.parameters.speed * 0.25
            } else {
                100.0
            }
        } else {
            self.parameters.speed
        };
        let mut speed = length.min(max);
        if self.arena() {
            speed = if water {
                length.min(self.parameters.speed * 0.5)
            } else {
                (length * scale).min(max)
            };
        }
        if water && !self.arena() {
            speed *= if self.nq() || self.qw() { 0.7 } else { 0.5 };
        }
        (wish, speed)
    }
    fn walk(&mut self) {
        let water = self.player.movement.water_level >= 2;
        if self.nq() && water {
            self.quake_water();
            super::slide::step_slide(self, false);
            return;
        }
        self.friction();
        let (direction, speed) = self.wish(water);
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
        } else if self.qw() || self.nq() {
            self.parameters.accel
        } else if self.classic() || self.rr() {
            if self.parameters.air != 0.0 {
                self.parameters.accel
            } else {
                1.0
            }
        } else {
            self.parameters.air
        };
        self.accelerate(
            direction,
            speed,
            accel,
            !ground
                && !water
                && (self.nq()
                    || self.qw()
                    || (self.classic() || self.rr()) && self.parameters.air != 0.0),
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
            super::slide::slide(self, false);
        } else {
            super::slide::step_slide(self, self.arena() && !ground);
        }
    }
    fn quake_water(&mut self) {
        let (direction, wish_speed) = self.wish(true);
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
    }
    fn snap(&mut self, previous: Vec3) {
        if self.arena() {
            self.player.body.velocity.0 = self.player.body.velocity.0.map(f32::round_ties_even);
            return;
        }
        if !self.classic() {
            return;
        }
        self.player.body.velocity.0 = self
            .player
            .body
            .velocity
            .0
            .map(|v| f32::from((v * 8.0) as i32 as i16) * 0.125);
        let position = self.player.body.position;
        let base = Vec3(
            position
                .0
                .map(|v| f32::from((v * 8.0) as i32 as i16) * 0.125),
        );
        let signs =
            std::array::from_fn::<_, 3, _>(|i| if position.0[i] >= 0.0 { 1.0 } else { -1.0 });
        for bits in [0, 4, 1, 2, 3, 5, 6, 7] {
            let next = Vec3(std::array::from_fn(|i| {
                base.0[i]
                    + if bits & (1 << i) != 0 {
                        signs[i] * 0.125
                    } else {
                        0.0
                    }
            }));
            let tr = self.trace(next, next);
            if !tr.all_solid {
                self.player.body.position = next;
                return;
            }
        }
        self.player.body.position = previous;
    }
    fn run(&mut self) {
        self.result.steps += 1;
        let previous = self.player.body.position;
        self.player.view_angles = self.command.view_angles + self.player.movement.delta_angles;
        if self.classic() {
            for i in 0..3 {
                let to_short = |v: f32| (v * (65536.0 / 360.0)) as i32 as i16;
                let angle = to_short(self.command.view_angles.0[i])
                    .wrapping_add(to_short(self.player.movement.delta_angles.0[i]));
                self.player.view_angles.0[i] = f32::from(angle) * (360.0 / 65536.0);
            }
            let pitch = &mut self.player.view_angles.0[0];
            if *pitch > 89.0 && *pitch < 180.0 {
                *pitch = 89.0;
            } else if *pitch < 271.0 && *pitch >= 180.0 {
                *pitch = 271.0;
            }
            if self.player.movement.timer.contains(MovementTimer::TELEPORT) {
                self.player.view_angles.0[0] = 0.0;
                self.player.view_angles.0[2] = 0.0;
            }
        } else if self.arena() {
            self.player.view_angles.0[0] = self.player.view_angles.0[0].clamp(-89.0, 89.0);
        }
        if self.player.movement.mode == MovementMode::Frozen {
            return;
        }
        self.stance();
        if matches!(
            self.player.movement.mode,
            MovementMode::Dead | MovementMode::Gib
        ) {
            self.command.movement = [0; 3];
            self.command.buttons = 0;
        }
        if matches!(
            self.player.movement.mode,
            MovementMode::Noclip | MovementMode::Spectator | MovementMode::Fly
        ) {
            let (direction, speed) = self.wish(false);
            if self.nq() {
                self.player.body.velocity = direction * speed;
            } else {
                self.friction();
                self.accelerate(direction, speed, self.parameters.accel, false);
            }
            if matches!(
                self.player.movement.mode,
                MovementMode::Noclip | MovementMode::Spectator
            ) {
                self.player.body.position =
                    self.player.body.position + self.player.body.velocity * self.dt;
            } else {
                super::slide::step_slide(self, false);
            }
            self.snap(previous);
            return;
        }
        if self.nq() {
            self.water_level();
        } else {
            self.categorize();
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
        if !self.player.movement.timer.contains(MovementTimer::TELEPORT) {
            self.jump();
            self.walk();
        }
        if !self.nq() {
            self.categorize();
        }
        self.snap(previous);
        self.player.movement.previous_position = previous;
    }
}
fn run_step(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
    parameters: Parameters,
    result: &mut MovementResult,
) {
    let exact_dt = if parameters.rules == RuleSetId::Quake {
        command.duration_ns as f64 / 1e9
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
            Contents::BODY
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
        previous_velocity,
    }
    .run();
}
fn quake(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
) -> MovementResult {
    let command = super::prepare_command(RuleSetId::Quake, command);
    let parameters = Parameters::load(RuleSetId::Quake, player);
    let mut result = MovementResult::default();
    run_step(command, player, world, parameters, &mut result);
    result
}
fn quakeworld(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
) -> MovementResult {
    let command = super::prepare_command(RuleSetId::QuakeWorld, command);
    let parameters = Parameters::load(RuleSetId::QuakeWorld, player);
    let mut result = MovementResult::default();
    // SV_RunCmd/CL_PredictUsercmd recursively halve both halves, dropping odd ms.
    let mut durations = [0u16; 8];
    durations[0] = command.duration_ms;
    let mut pending = 1;
    while pending > 0 {
        pending -= 1;
        let ms = durations[pending];
        if ms > 50 {
            durations[pending] = ms / 2;
            durations[pending + 1] = ms / 2;
            pending += 2;
        } else {
            run_step(
                UserCmd {
                    duration_ms: ms,
                    duration_ns: u64::from(ms) * 1_000_000,
                    ..command
                },
                player,
                world,
                parameters,
                &mut result,
            );
        }
    }
    result
}
fn quake2(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
) -> MovementResult {
    // Native pmove_state_t carries signed 16-bit coordinates and velocities
    // in eighth units. Rerelease deliberately keeps its floating ABI.
    player.body.position.0 = player.body.position.0.map(eighth);
    player.body.velocity.0 = player.body.velocity.0.map(eighth);
    let command = super::prepare_command(RuleSetId::Quake2, command);
    let parameters = Parameters::load(RuleSetId::Quake2, player);
    let mut result = MovementResult::default();
    run_step(command, player, world, parameters, &mut result);
    result
}
fn eighth(value: f32) -> f32 {
    f32::from((value * 8.0) as i32 as i16) * 0.125
}
fn rerelease(
    command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
) -> MovementResult {
    let command = super::prepare_command(RuleSetId::Quake2Rerelease, command);
    let parameters = Parameters::load(RuleSetId::Quake2Rerelease, player);
    let mut result = MovementResult::default();
    run_step(command, player, world, parameters, &mut result);
    result
}
fn arena(
    mut command: UserCmd,
    player: &mut PlayerState,
    world: &mut dyn TraceServices,
) -> MovementResult {
    let parameters = Parameters::load(RuleSetId::Quake3, player);
    let mut result = MovementResult::default();
    let end = command.server_time_ms;
    if end < player.movement.command_time_ms {
        return result;
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
        run_step(command, player, world, parameters, &mut result);
        if player.movement.jump_held {
            command.movement[2] = 20;
        }
    }
    result
}
