use qa_core::{
    math,
    primitives::{MovementMagnitude, PlayerState, RuleSetId, Vec3},
};
use qa_movement::{
    control::{acceleration, capped_wish, water_speed},
    control_tuning,
};

#[test]
fn native_acceleration_preserves_distinct_ground_and_air_promotion_points() {
    let coefficient = 4.3568854331970215;
    let speed = 222.07086181640625;
    let seconds = 0.09298059400454946;
    assert_eq!(
        acceleration(-100.0, speed, coefficient, seconds, false)
            .unwrap()
            .to_bits(),
        0x42b3eca5,
    );
    assert_eq!(
        acceleration(-100.0, speed, coefficient, seconds, true)
            .unwrap()
            .to_bits(),
        0x42b3eca4,
    );
}

#[test]
fn native_water_speed_promotes_the_original_double_literal() {
    assert_eq!(water_speed(172.83140563964844).to_bits(), 0x42f1f6c7);
}

#[test]
fn capped_air_velocity_requires_the_second_native_normalization() {
    let wish = capped_wish(Vec3([-495.2919006347656, -513.3533325195312, 0.0]), 320.0);
    assert_eq!(wish.direction.0[0].to_bits(), 0xbf31bfd8);
    let mut air_direction = wish.velocity;
    math::normalize(&mut air_direction);
    assert_eq!(air_direction.0[0].to_bits(), 0xbf31bfd9);
}

#[test]
fn control_tuning_uses_selected_movement_defaults_and_explicit_edge_override() {
    let mut player = PlayerState {
        movement_rules: RuleSetId::Quake2,
        trace_rules: RuleSetId::Quake3,
        ..Default::default()
    };
    let defaults = control_tuning(&player);
    assert_eq!(defaults.max_speed, 300.0);
    assert_eq!(defaults.friction, 6.0);
    assert_eq!(defaults.edge_friction, 2.0);
    player.movement.tuning.max_speed = Some(MovementMagnitude::Base(400.0));
    player.movement.tuning.speed_multiplier = 1.5;
    player.movement.tuning.edge_friction = Some(3.25);
    player.movement.tuning.no_step = true;
    let tuned = control_tuning(&player);
    assert_eq!(tuned.max_speed, 600.0);
    assert_eq!(tuned.edge_friction, 3.25);
    assert!(tuned.no_step);
}
