//! Native control arithmetic shared by player movement and scoped body motion.
use qa_core::{
    math,
    primitives::{PlayerState, UserCmd, Vec3},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlStart {
    pub on_ground: bool,
    pub body_angles: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlTuning {
    pub max_speed: f32,
    pub stop_speed: f32,
    pub friction: f32,
    pub accelerate: f32,
    pub edge_friction: f32,
    pub no_step: bool,
}

pub fn control_tuning(player: &PlayerState) -> ControlTuning {
    let parameters = super::physics::Parameters::load(player.movement_rules, player);
    ControlTuning {
        max_speed: parameters.speed,
        stop_speed: parameters.stop,
        friction: parameters.friction,
        accelerate: parameters.accel,
        edge_friction: player.movement.tuning.edge_friction.unwrap_or(2.0),
        no_step: parameters.no_step,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ControlWish {
    pub velocity: Vec3,
    pub direction: Vec3,
    pub speed: f32,
    pub length: f32,
}

pub fn wish_velocity(forward: Vec3, right: Vec3, axes: [f32; 3]) -> Vec3 {
    forward * axes[0] + right * axes[1]
}

/// Direction is normalized before the speed cap; the capped velocity remains
/// separate because native AirAccelerate normalizes that vector a second time.
pub fn capped_wish(velocity: Vec3, maximum: f32) -> ControlWish {
    let mut direction = velocity;
    let length = math::normalize(&mut direction);
    let speed = if length > maximum { maximum } else { length };
    ControlWish {
        velocity: cap_velocity(velocity, length, maximum),
        direction,
        speed,
        length,
    }
}

pub fn cap_velocity(velocity: Vec3, length: f32, maximum: f32) -> Vec3 {
    if length > maximum {
        velocity * (maximum / length)
    } else {
        velocity
    }
}

pub fn air_wish(
    angles: Vec3,
    command: UserCmd,
    walking: bool,
    block_reverse: bool,
    maximum: f32,
) -> ControlWish {
    let basis = math::angle_vectors(angles);
    let mut axes = command.movement;
    if block_reverse && axes[0] < 0.0 {
        axes[0] = 0.0;
    }
    let mut velocity = wish_velocity(basis.forward, basis.right, axes);
    velocity.0[2] = if walking { 0.0 } else { axes[2] };
    capped_wish(velocity, maximum)
}

pub fn water_wish(angles: Vec3, command: UserCmd, maximum: f32) -> ControlWish {
    let basis = math::angle_vectors(angles);
    let axes = command.movement;
    let mut velocity = wish_velocity(basis.forward, basis.right, axes);
    if axes == [0.0; 3] {
        velocity.0[2] -= 60.0;
    } else {
        velocity.0[2] += axes[2];
    }
    let mut wish = capped_wish(velocity, maximum);
    wish.speed = water_speed(wish.speed);
    wish.direction = wish.velocity;
    math::normalize(&mut wish.direction);
    wish
}

pub fn water_speed(speed: f32) -> f32 {
    (f64::from(speed) * 0.7) as f32
}

pub fn friction_speed(speed: f32, control: f32, friction: f32, seconds: f64) -> f32 {
    let mut next = (f64::from(speed) - seconds * f64::from(control) * f64::from(friction)) as f32;
    if next < 0.0 {
        next = 0.0;
    }
    next
}

pub fn friction_scale(speed: f32, stop: f32, friction: f32, seconds: f64) -> f32 {
    let control = if speed < stop { stop } else { speed };
    friction_speed(speed, control, friction, seconds) / speed
}

/// Ground acceleration promotes before multiplying duration and speed; air and
/// water multiply their coefficient and wish speed in binary32 first.
pub fn acceleration(
    current: f32,
    wish_speed: f32,
    coefficient: f32,
    seconds: f64,
    air_cap: bool,
) -> Option<f32> {
    let target = if air_cap && wish_speed > 30.0 {
        30.0
    } else {
        wish_speed
    };
    acceleration_amount(current, target, wish_speed, coefficient, seconds, air_cap)
}

pub fn air_acceleration(
    current: f32,
    direction_speed: f32,
    wish_speed: f32,
    coefficient: f32,
    seconds: f64,
) -> Option<f32> {
    let target = if direction_speed > 30.0 {
        30.0
    } else {
        direction_speed
    };
    acceleration_amount(current, target, wish_speed, coefficient, seconds, true)
}

fn acceleration_amount(
    current: f32,
    target: f32,
    wish_speed: f32,
    coefficient: f32,
    seconds: f64,
    narrow_product: bool,
) -> Option<f32> {
    let add = target - current;
    if add <= 0.0 {
        return None;
    }
    let amount = if narrow_product {
        (f64::from(coefficient * wish_speed) * seconds) as f32
    } else {
        (f64::from(coefficient) * seconds * f64::from(wish_speed)) as f32
    };
    Some(if amount > add { add } else { amount })
}

pub fn water_acceleration(
    wish_speed: f32,
    current_speed: f32,
    coefficient: f32,
    seconds: f64,
) -> Option<f32> {
    if wish_speed == 0.0 {
        return None;
    }
    acceleration_amount(
        current_speed,
        wish_speed,
        wish_speed,
        coefficient,
        seconds,
        true,
    )
}
