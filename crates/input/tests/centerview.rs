//! Native centerview and render-phase pitch drift, through the one input state.
#[path = "support/console.rs"]
mod support;
use qa_core::{
    primitives::{MovementMode, PlayerState, RuleSetId, Vec3},
    sys_events::{EventTime, SeatId},
};
use qa_input::{Action, Input};

#[test]
fn unchanged_drift_transition_preserves_both_native_signed_zero_samples() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut input = Input::load();
        input.set_view_angles(SeatId::FIRST, Vec3([-0.0, 5.0, 2.0]));
        let player = PlayerState {
            view_angles: Vec3([17.0, 95.0, 12.0]),
            ..Default::default()
        };
        let (previous, pitch) = input
            .drift_view(
                SeatId::FIRST,
                EventTime::from_milliseconds(16),
                support::catalog_policy(rules),
                &player,
                0.0,
            )
            .unwrap();
        assert_eq!(
            [previous, pitch].map(f32::to_bits),
            [(-0.0f32).to_bits(); 2]
        );
        assert_eq!(player.view_angles, Vec3([17.0, 95.0, 12.0]));
    }
}

#[test]
fn intermission_cannot_be_replaced_by_quake_render_pitch_drift() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for mode in [
            MovementMode::Intermission,
            MovementMode::SinglePlayerIntermission,
        ] {
            let mut input = Input::load();
            input.seed(EventTime(0));
            input.set_view_angles(SeatId::FIRST, Vec3([40.0, 5.0, 2.0]));
            input.center_view(SeatId::FIRST);
            let policy = support::catalog_policy(rules);
            let time = EventTime::from_milliseconds(16);
            input.build_frame_with_policy(time, &[policy; SeatId::COUNT]);
            let mut player = PlayerState {
                movement_rules: RuleSetId::Quake2Rerelease,
                trace_rules: RuleSetId::Quake3,
                view_angles: Vec3([-17.0, 133.0, 9.0]),
                ideal_pitch: 20.0,
                ..Default::default()
            };
            player.movement.mode = mode;
            player.movement.grounded = true;
            assert_eq!(
                input.drift_view(SeatId::FIRST, time, policy, &player, 0.0),
                None,
            );
            // The rejected render update did not consume the centerview/drift
            // request. Freeze still follows the selected native input policy.
            player.movement.mode = MovementMode::Frozen;
            assert_eq!(
                input
                    .drift_view(SeatId::FIRST, time, policy, &player, 0.0)
                    .unwrap(),
                (40.0, 32.0),
            );
        }
    }
}

#[test]
fn accepted_interpolated_view_is_not_replaced_by_local_quake_pitch_drift() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut input = Input::load();
        input.seed(EventTime(0));
        input.set_view_angles(SeatId::FIRST, Vec3([40.0, 5.0, 2.0]));
        input.center_view(SeatId::FIRST);
        let policy = support::catalog_policy(rules);
        let time = EventTime::from_milliseconds(16);
        input.build_frame_with_policy(time, &[policy; SeatId::COUNT]);
        let mut player = PlayerState {
            prediction: qa_core::primitives::PredictionPolicy::DISABLED,
            view_angles: Vec3([-17.0, 133.0, 9.0]),
            ideal_pitch: 20.0,
            ..Default::default()
        };
        player.movement.grounded = true;
        assert_eq!(
            input.drift_view(SeatId::FIRST, time, policy, &player, 0.0),
            None
        );
        assert_eq!(player.view_angles, Vec3([-17.0, 133.0, 9.0]));
        player.prediction = Default::default();
        assert!(
            input
                .drift_view(SeatId::FIRST, time, policy, &player, 0.0)
                .is_some()
        );
    }
}

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
        let mut policy = support::catalog_policy(rules);
        policy.delta_pitch = 20.0;
        input.center_view(second);
        let cmds = input.build_frame_with_policy(EventTime(16_000_000), &[policy; SeatId::COUNT]);
        assert_eq!(cmds[0].view_angles.0, [40.0, 5.0, 2.0]);
        assert_eq!(cmds[1].view_angles.0, [-20.0, 5.0, 2.0]);
        for seat in [SeatId::FIRST, second] {
            assert_eq!(
                input.drift_view(
                    seat,
                    EventTime(16_000_000),
                    policy,
                    &PlayerState::default(),
                    0.0,
                ),
                None
            );
        }
    }
}

#[test]
fn quake_drift_follows_native_render_phase_and_ground_state() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut input = Input::load();
        input.seed(EventTime(0));
        input.set_view_angles(SeatId::FIRST, Vec3([40.0, 5.0, 0.0]));
        input.center_view(SeatId::FIRST);
        let policy = support::catalog_policy(rules);
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
                .unwrap(),
            (40.0, 32.0)
        );
        player.movement.grounded = false;
        let time = EventTime(32_000_000);
        let command = input.build_frame_with_policy(time, &[policy; SeatId::COUNT])[0];
        assert_eq!(command.view_angles.0[0], 32.0);
        assert_eq!(
            input
                .drift_view(SeatId::FIRST, time, policy, &player, 0.0)
                .unwrap(),
            (32.0, 32.0)
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
                .unwrap(),
            (32.0, expected)
        );
        input.button(
            SeatId::FIRST,
            Action::MouseLook,
            true,
            Some(qa_core::sys_events::KeySource {
                device: qa_core::sys_events::DeviceId::Keyboard(0),
                control: 1,
            }),
            None,
            time,
        );
        input.center_view(SeatId::FIRST);
        let time = EventTime(80_000_000);
        let command = input.build_frame_with_policy(time, &[policy; SeatId::COUNT])[0];
        assert_eq!(command.view_angles.0[0], expected);
        assert_eq!(
            input
                .drift_view(SeatId::FIRST, time, policy, &player, 0.0)
                .unwrap(),
            (expected, expected)
        );
    }
}
