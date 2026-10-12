//! Native ladder and water-jump capabilities over the shared movement executor.
use crate::{physics::Step, slide};
use qa_core::{
    math,
    primitives::{MovementTimer, Vec3, buttons},
};
use qa_gameplay::rules::movement::{
    JumpTime, LadderControl, ProbePhase, WaterJumpMotion, WaterJumpPolicy, WaterJumpProbe,
};
use qa_world::collision::Contents;

impl Step<'_> {
    fn flat_forward(&self) -> Vec3 {
        let mut forward = self.view_basis(self.player.view_angles).forward;
        forward.0[2] = 0.0;
        math::normalize(&mut forward);
        forward
    }

    fn ladder_trace(&mut self, mask: Contents) -> qa_world::collision::Trace {
        let start = self.player.body.position;
        let end = start + self.flat_forward();
        self.trace_mask(
            start,
            end,
            self.player.body.mins,
            self.player.body.maxs,
            mask,
        )
    }

    pub(crate) fn check_special(&mut self) {
        if let Some(policy) = self.parameters.special.ladder {
            // Classic pml.ladder is cleared at every Pmove; the rerelease flag
            // remains published when an active native timer skips this check.
            if policy.transient {
                self.player.movement.ladder = false;
            }
            if self.player.movement.remaining_ms == 0 {
                self.player.movement.ladder = false;
                let mask = if policy.ladder_only_mask {
                    Contents::LADDER
                } else {
                    self.mask
                };
                let trace = self.ladder_trace(mask);
                self.player.movement.ladder = trace.fraction < 1.0
                    && trace.contents.intersects(Contents::LADDER)
                    && self.player.movement.water_level <= policy.max_water_level;
            }
        }
        if self
            .parameters
            .special
            .water_jump
            .is_some_and(|policy| matches!(policy.phase, ProbePhase::BeforeTimers))
        {
            self.check_water_jump();
        }
        if self.player.body.velocity.0[2] < 0.0 {
            self.player.movement.water_jump_seconds = 0.0;
        }
    }

    pub(crate) fn adjust_wish(&mut self, wish: &mut Vec3) {
        if self.player.movement.ladder
            && let Some(policy) = self.parameters.special.ladder
        {
            let forward = self.command.movement[0];
            let up = self.command.movement[2];
            match policy.control {
                LadderControl::PitchVertical => {
                    if self.player.body.velocity.0[2].abs() <= 200.0 {
                        let pitch = math::wrap_above_180(self.player.view_angles.0[0]);
                        wish.0[2] = if pitch <= -15.0 && forward > 0.0 {
                            200.0
                        } else if pitch >= 15.0 && forward > 0.0 {
                            -200.0
                        } else if up > 0.0 || self.command.buttons & buttons::JUMP != 0 {
                            200.0
                        } else if up < 0.0 || self.command.buttons & buttons::CROUCH != 0 {
                            -200.0
                        } else {
                            0.0
                        };
                        wish.0[0] = wish.0[0].clamp(-25.0, 25.0);
                        wish.0[1] = wish.0[1].clamp(-25.0, 25.0);
                    }
                }
                LadderControl::ProjectedStrafe => {
                    let vertical_buttons = self.command.buttons & (buttons::JUMP | buttons::CROUCH);
                    wish.0[2] = if vertical_buttons != 0 {
                        let speed = if self.player.movement.water_level >= 2 {
                            self.parameters.speed
                        } else {
                            200.0
                        };
                        if vertical_buttons & buttons::JUMP != 0 {
                            speed
                        } else {
                            -speed
                        }
                    } else if forward > 0.0 {
                        let speed = forward.clamp(-200.0, 200.0);
                        if self.player.view_angles.0[0] < 15.0 {
                            speed
                        } else {
                            -speed
                        }
                    } else if forward < 0.0 {
                        if !self.player.movement.grounded {
                            wish.0[0] = 0.0;
                            wish.0[1] = 0.0;
                        }
                        forward.clamp(-200.0, 200.0)
                    } else {
                        0.0
                    };
                    if !self.player.movement.grounded {
                        if self.command.movement[1] != 0.0 {
                            let mut speed = self.command.movement[1].clamp(-150.0, 150.0);
                            if self.player.movement.water_level < 2 {
                                speed *= 0.5;
                            }
                            let trace = self.ladder_trace(Contents::LADDER);
                            if trace.fraction != 1.0 && trace.contents.intersects(Contents::LADDER)
                            {
                                let right = math::cross(trace.plane.normal, Vec3([0.0, 0.0, 1.0]));
                                wish.0[0] = 0.0;
                                wish.0[1] = 0.0;
                                *wish = *wish + right * -speed;
                            }
                        } else {
                            wish.0[0] = wish.0[0].clamp(-25.0, 25.0);
                            wish.0[1] = wish.0[1].clamp(-25.0, 25.0);
                        }
                    }
                }
            }
        }
        if let Some(speed) = self.parameters.special.current_speed {
            let water_speed =
                if self.player.movement.water_level == 1 && self.player.movement.grounded {
                    speed / 2.0
                } else {
                    speed
                };
            let contents = Contents(self.player.movement.water_contents);
            if contents.intersects(Contents::CURRENT) {
                *wish = *wish + contents.current() * water_speed;
            }
            if self.player.movement.grounded {
                *wish = *wish + Contents(self.player.movement.ground_contents).current() * 100.0;
            }
        }
    }

    pub(crate) fn check_water_jump(&mut self) {
        let Some(policy) = self.parameters.special.water_jump else {
            return;
        };
        if self.player.movement.remaining_ms != 0
            || self.player.movement.water_jump_seconds != 0.0
            || self.player.movement.water_level != 2
        {
            return;
        }
        let forward = self.flat_forward();
        let can_jump = match policy.probe {
            WaterJumpProbe::Points {
                ahead,
                lower,
                upper,
                down_limit,
            } => {
                if down_limit.is_some_and(|limit| self.player.body.velocity.0[2] < limit) {
                    return;
                }
                let mut point = self.player.body.position + forward * ahead;
                point.0[2] += lower;
                let low = self
                    .world
                    .point_contents(point, self.parameters.entity_rules);
                if !low.intersects(Contents::SOLID) {
                    return;
                }
                point.0[2] += upper;
                self.world
                    .point_contents(point, self.parameters.entity_rules)
                    == Contents::EMPTY
            }
            WaterJumpProbe::Ballistic => self.ballistic_water_jump(forward, policy),
        };
        if !can_jump {
            return;
        }
        self.player.body.velocity = if policy.full_forward {
            self.view_basis(self.player.view_angles).forward * policy.horizontal
        } else {
            forward * policy.horizontal
        };
        self.player.body.velocity.0[2] = policy.vertical;
        match policy.duration {
            JumpTime::Milliseconds(ms) => {
                self.start_timer(MovementTimer::WATER_JUMP, ms);
            }
            JumpTime::Seconds(seconds) => self.player.movement.water_jump_seconds = seconds,
        }
        if policy.mark_jump_held {
            self.player.movement.jump_held = true;
        }
    }

    fn ballistic_water_jump(&mut self, forward: Vec3, policy: WaterJumpPolicy) -> bool {
        // An inverted-gravity ballistic preview has no finite native loop bound.
        // Zero/negative gravity does not initiate this ordinary upward escape.
        if self.parameters.gravity <= 0.0
            || self.command.buttons & buttons::JUMP == 0 && self.command.movement[0] <= 0.0
            || Contents(self.player.movement.water_contents).intersects(Contents::NO_WATERJUMP)
        {
            return false;
        }
        let start = self.player.body.position;
        let solid = Contents::SOLID | Contents::WINDOW;
        let trace = self.trace_mask(
            start,
            start + forward * 40.0,
            self.player.body.mins,
            self.player.body.maxs,
            solid,
        );
        if trace.fraction == 1.0 || trace.plane.normal.0[2] >= 0.7 {
            return false;
        }

        // Reuse the one slide executor with copied hot state; contacts from this
        // bounded query never become live contacts or mutate the area index.
        let body = self.player.body;
        let movement = self.player.movement;
        let result = *self.result;
        let dt = self.dt;
        self.dt = 0.1;
        self.player.body.velocity = forward * policy.horizontal;
        self.player.body.velocity.0[2] = policy.vertical;
        let mut has_time = true;
        let iterations = (10.0 * (800.0 / self.parameters.gravity)) as i32;
        for _ in 0..iterations.clamp(0, 50) {
            self.player.body.velocity.0[2] -= self.parameters.gravity * self.dt;
            if self.player.body.velocity.0[2] < 0.0 {
                has_time = false;
            }
            self.player.movement.remaining_ms = u32::from(has_time);
            slide::slide(self, false);
        }
        let destination = self.player.body.position;
        self.player.body = body;
        self.player.movement = movement;
        self.dt = dt;
        let traces = self.result.traces;
        *self.result = result;
        self.result.traces = traces;

        let trace = self.trace_mask(
            destination,
            destination - Vec3([0.0, 0.0, 2.0]),
            body.mins,
            body.maxs,
            solid,
        );
        if trace.fraction == 1.0
            || trace.plane.normal.0[2] < 0.7
            || trace.end.0[2] < start.0[2]
            || movement.grounded && (start.0[2] - trace.end.0[2]).abs() <= 18.0
        {
            return false;
        }
        // Keep the original rerelease helper's upper-sample origin/order.
        self.water_at(trace.end, start).0 < 2
    }

    pub(crate) fn water_jump_move(&mut self) -> bool {
        let Some(policy) = self.parameters.special.water_jump else {
            return false;
        };
        if !self
            .player
            .movement
            .timer
            .contains(MovementTimer::WATER_JUMP)
        {
            return false;
        }
        match policy.motion {
            WaterJumpMotion::Existing => return false,
            WaterJumpMotion::PreGravity => {
                self.player.body.velocity.0[2] -= self.parameters.gravity * self.dt;
                self.clear_falling_water_jump();
                slide::step_slide(self, false);
            }
            WaterJumpMotion::StepGravityThenGravity => {
                slide::step_slide(self, true);
                self.player.body.velocity.0[2] -= self.parameters.gravity * self.dt;
                self.clear_falling_water_jump();
            }
        }
        true
    }

    fn clear_falling_water_jump(&mut self) {
        if self.player.body.velocity.0[2] < 0.0 {
            self.clear_timer_flags(MovementTimer(u8::MAX));
        }
    }
}
