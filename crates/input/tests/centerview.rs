//! Native centerview and render-phase pitch drift, through the one input state.
use qa_core::{
    primitives::{PlayerState, RuleSetId, Vec3},
    sys_events::{EventTime, SeatId},
};
use qa_input::{Action, Input, InputPolicy};

#[test]
fn delta_centering_changes_only_the_selected_seat() {
    for rules in [
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let mut input = Input::load();
        input.seed(EventTime(0));
        let second = SeatId::new(1).unwrap();
        for seat in [SeatId::FIRST, second] {
            input.set_view_angles(seat, Vec3([40.0, 5.0, 2.0]));
        }
        let mut policy = InputPolicy::native(rules);
        policy.delta_pitch = 20.0;
        input.center_view(second);
        let cmds = input.build_frame_with_policy(EventTime(16_000_000), &[policy; SeatId::COUNT]);
        assert_eq!(cmds[0].view_angles.0, [40.0, 5.0, 2.0]);
        assert_eq!(cmds[1].view_angles.0, [-20.0, 5.0, 2.0]);
    }
}

#[test]
fn quake_drift_follows_native_render_phase_and_ground_state() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut input = Input::load();
        input.seed(EventTime(0));
        input.set_view_angles(SeatId::FIRST, Vec3([40.0, 5.0, 0.0]));
        input.center_view(SeatId::FIRST);
        let policy = InputPolicy::native(rules);
        let mut player = PlayerState {
            ideal_pitch: 20.0,
            ..Default::default()
        };
        player.movement.grounded = true;
        let time = EventTime(16_000_000);
        let command = input.build_frame_with_policy(time, &[policy; SeatId::COUNT])[0];
        assert_eq!(command.view_angles.0[0], 40.0);
        assert_eq!(
            input
                .drift_view(SeatId::FIRST, time, policy, &player, 0.0)
                .0[0],
            32.0
        );
        player.movement.grounded = false;
        let time = EventTime(32_000_000);
        input.build_frame_with_policy(time, &[policy; SeatId::COUNT]);
        assert_eq!(
            input
                .drift_view(SeatId::FIRST, time, policy, &player, 0.0)
                .0[0],
            32.0
        );
        player.movement.grounded = true;
        input.center_view(SeatId::FIRST);
        let time = EventTime(64_000_000);
        input.build_frame_with_policy(time, &[policy; SeatId::COUNT]);
        let expected = if rules == RuleSetId::Quake {
            20.0
        } else {
            16.0
        };
        assert_eq!(
            input
                .drift_view(SeatId::FIRST, time, policy, &player, 0.0)
                .0[0],
            expected
        );
        input.button(SeatId::FIRST, Action::MouseLook, true, Some(1), time);
        input.center_view(SeatId::FIRST);
        let time = EventTime(80_000_000);
        input.build_frame_with_policy(time, &[policy; SeatId::COUNT]);
        assert_eq!(
            input
                .drift_view(SeatId::FIRST, time, policy, &player, 0.0)
                .0[0],
            expected
        );
    }
}
