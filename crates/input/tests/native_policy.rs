//! Original input references: NQ/QW cl_input.c CL_KeyState/CL_BaseMove,
//! Q2 client/cl_input.c CL_BaseMove/CL_ClampPitch, Q3 client/cl_input.c
//! CL_KeyMove/CL_MouseMove/CL_CreateCmd and game/q_math.c ClampChar.
use qa_core::{
    primitives::{RuleSetId, UserCmd, Vec3, buttons},
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent},
};
use qa_input::{Action, Input, InputPolicy, Target};

struct Sink;
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(&mut self, _: SeatId, _: EventTime, _: &str) {}
}
fn held(input: &mut Input, action: Action, down: bool, ms: u64) {
    input.button(
        SeatId::FIRST,
        action,
        down,
        Some(action as u16),
        EventTime(ms * 1_000_000),
    );
}
fn frame(input: &mut Input, ms: u64, policy: InputPolicy) -> UserCmd {
    input.build_frame_with_policy(EventTime(ms * 1_000_000), &[policy; SeatId::COUNT])[0]
}
fn mouse(input: &mut Input, ms: u64, dx: i32, dy: i32) {
    input.dispatch(
        SysEvent {
            time: EventTime(ms * 1_000_000),
            kind: EventKind::Mouse {
                device: DeviceId::Mouse(0),
                dx,
                dy,
            },
        },
        &mut Sink,
    );
}
fn input() -> Input {
    let mut input = Input::load();
    input.seed(EventTime(0));
    input
}

#[test]
fn quake_keyboard_transitions_remain_native_at_arbitrary_event_times() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut input = input();
        let policy = InputPolicy::native(rules);
        held(&mut input, Action::Forward, true, 80);
        assert_eq!(frame(&mut input, 100, policy).movement[0], 100);
        assert_eq!(frame(&mut input, 200, policy).movement[0], 200);
        held(&mut input, Action::Forward, false, 250);
        assert_eq!(frame(&mut input, 300, policy).movement[0], 0);
        held(&mut input, Action::Forward, true, 310);
        held(&mut input, Action::Forward, false, 320);
        assert_eq!(frame(&mut input, 400, policy).movement[0], 50);
        held(&mut input, Action::Forward, true, 410);
        held(&mut input, Action::Forward, false, 420);
        held(&mut input, Action::Forward, true, 430);
        assert_eq!(frame(&mut input, 500, policy).movement[0], 150);
    }
}

#[test]
fn quake2_and_quake3_keep_elapsed_time_key_fractions() {
    for (rules, expected) in [(RuleSetId::Quake2, 40), (RuleSetId::Quake3, 25)] {
        let mut input = input();
        held(&mut input, Action::Forward, true, 80);
        assert_eq!(
            frame(&mut input, 100, InputPolicy::native(rules)).movement[0],
            expected
        );
    }
}

#[test]
fn independent_forward_back_speeds_and_combined_strafe_are_not_normalized() {
    let mut input = input();
    let mut policy = InputPolicy::native(RuleSetId::Quake);
    policy.speed[0] = 400.0;
    held(&mut input, Action::Back, true, 0);
    assert_eq!(frame(&mut input, 100, policy).movement[0], -100);
    held(&mut input, Action::Forward, true, 120);
    assert_eq!(frame(&mut input, 200, policy).movement[0], 0);
    held(&mut input, Action::Strafe, true, 210);
    held(&mut input, Action::TurnRight, true, 220);
    held(&mut input, Action::Right, true, 230);
    frame(&mut input, 300, policy);
    assert_eq!(frame(&mut input, 400, policy).movement[1], 700);
}

#[test]
fn released_modifiers_do_not_change_native_movement_or_mouse_mode() {
    let mut input = input();
    let mut policy = InputPolicy::native(RuleSetId::Quake2);
    policy.freelook = false;
    policy.mouse_forward = 1.0;
    held(&mut input, Action::Forward, true, 0);
    for action in [Action::Walk, Action::Strafe, Action::MouseLook] {
        held(&mut input, action, true, 10);
        held(&mut input, action, false, 20);
    }
    mouse(&mut input, 30, 10, 10);
    let command = frame(&mut input, 100, policy);
    assert_eq!(command.movement, [170, 0, 0]);
    assert!((command.view_angles.0[1] + 0.66).abs() < 0.00001);
    assert_eq!(command.view_angles.0[0], 0.0);
}

#[test]
fn quake_speed_key_changes_saved_keyboard_speed_and_angle_rate() {
    let mut input = input();
    let mut policy = InputPolicy::native(RuleSetId::Quake);
    policy.speed[0] = 400.0;
    policy.angle_speed[0] = 200.0;
    policy.angle_multiplier = 2.0;
    held(&mut input, Action::Forward, true, 0);
    held(&mut input, Action::TurnLeft, true, 0);
    held(&mut input, Action::Walk, true, 0);
    let command = frame(&mut input, 100, policy);
    assert_eq!(command.movement[0], 400);
    // anglemod(20): floor(20 * 65536 / 360) * 360 / 65536.
    assert_eq!(command.view_angles.0[1], 3640.0 * (360.0 / 65536.0));
    held(&mut input, Action::Strafe, true, 110);
    input.set_view_angles(SeatId::FIRST, Vec3([0.0, 20.12345, 0.0]));
    assert_eq!(frame(&mut input, 200, policy).view_angles.0[1], 20.12345);
}

#[test]
fn quake3_world_unit_settings_do_not_scale_native_byte_commands() {
    let mut input = input();
    let mut policy = InputPolicy::native(RuleSetId::Quake3);
    policy.speed = [10_000.0, 1.0, 32_000.0];
    policy.back_speed = 0.0;
    policy.move_multiplier = 0.0;
    held(&mut input, Action::Forward, true, 0);
    held(&mut input, Action::Right, true, 0);
    held(&mut input, Action::Up, true, 0);
    let run = frame(&mut input, 100, policy);
    assert_eq!(run.movement, [127; 3]);
    assert_eq!(run.buttons & buttons::WALK, 0);
    held(&mut input, Action::Walk, true, 100);
    let walk = frame(&mut input, 200, policy);
    assert_eq!(walk.movement, [64; 3]);
    assert_ne!(walk.buttons & buttons::WALK, 0);
    policy.always_run = false;
    assert_eq!(frame(&mut input, 300, policy).movement, [127; 3]);
}

#[test]
fn quake3_truncates_each_key_contribution_before_mouse_clamping() {
    let mut input = input();
    let policy = InputPolicy::native(RuleSetId::Quake3);
    held(&mut input, Action::Strafe, true, 0);
    held(&mut input, Action::TurnRight, true, 90);
    held(&mut input, Action::Right, true, 90);
    // Native integer side += 127*.1 twice is 12+12, rather than 25.
    assert_eq!(frame(&mut input, 100, policy).movement[1], 24);
    held(&mut input, Action::Right, false, 100);
    held(&mut input, Action::TurnRight, false, 100);
    held(&mut input, Action::TurnLeft, true, 100);
    held(&mut input, Action::Left, true, 100);
    assert_eq!(frame(&mut input, 200, policy).movement[1], -128);
}

#[test]
fn mouse_filter_inversion_and_freelook_use_the_cached_policy() {
    let mut input = input();
    let mut policy = InputPolicy::native(RuleSetId::Quake3);
    policy.sensitivity = 2.0;
    policy.filter = true;
    policy.mouse_scale = [0.1, -0.2];
    mouse(&mut input, 5, 10, 20);
    let first = frame(&mut input, 10, policy);
    assert_eq!(first.view_angles.0[..2], [-4.0, -1.0]);
    let filtered_tail = frame(&mut input, 20, policy);
    assert_eq!(filtered_tail.view_angles.0[..2], [-8.0, -2.0]);
    policy.filter = false;
    policy.freelook = false;
    policy.mouse_forward = 0.25;
    mouse(&mut input, 25, 0, 20);
    let move_mouse = frame(&mut input, 30, policy);
    assert_eq!(move_mouse.movement[0], -10);
    assert_eq!(move_mouse.view_angles.0[0], -8.0);
}

#[test]
fn native_pitch_limits_keep_quake3_accumulation_and_quake2_delta_angles() {
    let mut input = input();
    let mut policy = InputPolicy::native(RuleSetId::Quake3);
    policy.sensitivity = 1.0;
    policy.mouse_scale[1] = 1.0;
    input.set_view_angles(SeatId::FIRST, Vec3([170.0, 0.0, 0.0]));
    mouse(&mut input, 5, 0, 1000);
    assert_eq!(frame(&mut input, 10, policy).view_angles.0[0], 260.0);
    mouse(&mut input, 15, 0, -1000);
    assert_eq!(frame(&mut input, 20, policy).view_angles.0[0], 170.0);
    let mut q2 = InputPolicy::native(RuleSetId::Quake2);
    q2.delta_pitch = 20.0;
    input.set_view_angles(SeatId::FIRST, Vec3([100.0, 0.0, 0.0]));
    assert_eq!(frame(&mut input, 30, q2).view_angles.0[0], 69.0);
    q2.delta_pitch = 350.0;
    input.set_view_angles(SeatId::FIRST, Vec3([100.0, 0.0, 0.0]));
    assert_eq!(frame(&mut input, 40, q2).view_angles.0[0], 99.0);
}
