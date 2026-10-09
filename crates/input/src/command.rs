use crate::InputPolicy;
use qa_core::{
    primitives::{CommandIntent, UserCmd, buttons},
    sys_events::EventTime,
};

/// One intent conversion for human and bot clients. Native wire projections
/// remain in network; this table describes command construction arithmetic.
pub struct UserCmdBuilder;
impl UserCmdBuilder {
    pub fn build(
        duration: std::time::Duration,
        time: EventTime,
        intent: CommandIntent,
        policy: InputPolicy,
    ) -> UserCmd {
        let rules = policy.command_rules();
        let running = intent.speed_modifier != policy.always_run;
        let key_speed = rules.key_scale.map(|scales| scales[usize::from(running)]);
        let speeds = key_speed.map_or(policy.speed, |speed| [speed; 3]);
        let back_speed = key_speed.unwrap_or(policy.back_speed);
        let add = |value: f32, amount: f32| rules.accumulation.narrow(value + amount);
        let mut movement = [0.0; 3];
        movement[1] = add(movement[1], speeds[1] * intent.strafe[0]);
        movement[1] = add(movement[1], -speeds[1] * intent.strafe[1]);
        movement[1] = add(movement[1], speeds[1] * intent.movement[1][0]);
        movement[1] = add(movement[1], -speeds[1] * intent.movement[1][1]);
        let up = if rules.vertical_actions {
            intent.movement[2][0].max(intent.vertical_actions[0])
        } else {
            intent.movement[2][0]
        };
        let down = if rules.vertical_actions {
            intent.movement[2][1].max(intent.vertical_actions[1])
        } else {
            intent.movement[2][1]
        };
        movement[2] = add(movement[2], speeds[2] * up);
        movement[2] = add(movement[2], -speeds[2] * down);
        movement[0] = add(movement[0], speeds[0] * intent.movement[0][0]);
        movement[0] = add(movement[0], -back_speed * intent.movement[0][1]);
        let multiplier = if key_speed.is_none() && running {
            policy.move_multiplier
        } else {
            1.0
        };
        if key_speed.is_none() && running {
            movement
                .iter_mut()
                .for_each(|value| *value = rules.accumulation.narrow(*value * multiplier));
        }
        if let Some([low, high]) = rules.key_limit {
            movement
                .iter_mut()
                .for_each(|value| *value = value.clamp(low, high));
        }
        movement[1] = add(movement[1], intent.mouse_movement[0] * policy.mouse_side);
        if let Some([low, high]) = rules.key_limit {
            movement[1] = movement[1].clamp(low, high);
        }
        movement[0] = add(
            movement[0],
            -intent.mouse_movement[1] * policy.mouse_forward,
        );
        if let Some([low, high]) = rules.key_limit {
            movement[0] = movement[0].clamp(low, high);
        }
        for axes in intent.axes.iter().take(usize::from(intent.axis_count)) {
            movement[0] = add(movement[0], axes[0] * speeds[0] * multiplier);
            movement[1] = add(movement[1], axes[1] * speeds[1] * multiplier);
        }
        let movement = std::array::from_fn(|axis| {
            let [low, high] = rules.key_limit.unwrap_or_else(|| {
                if policy.rules.is_none() {
                    [-policy.speed[axis], policy.speed[axis]]
                } else {
                    [f32::MIN, f32::MAX]
                }
            });
            let value = movement[axis].clamp(low, high);
            if policy.rules.is_none() {
                (value as i32) as f32
            } else {
                value
            }
        });
        let mut buttons = intent.buttons;
        if rules.key_scale.is_some() {
            if running {
                buttons &= !buttons::WALK;
            } else {
                buttons |= buttons::WALK;
            }
        }
        UserCmd {
            duration_ms: duration.as_millis().min(u128::from(u16::MAX)) as u16,
            duration_ns: duration.as_nanos().min(u128::from(u64::MAX)) as u64,
            server_time_ms: time.milliseconds() as i32,
            view_angles: intent.view_angles,
            movement,
            buttons,
            impulse: intent.impulse,
            weapon: intent.weapon,
            light_level: intent.light_level,
        }
    }
}
