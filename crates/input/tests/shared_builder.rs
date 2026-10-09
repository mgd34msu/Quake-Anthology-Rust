use qa_core::{
    primitives::{CommandIntent, RuleSetId, buttons},
    sys_events::{EventTime, SeatId},
};
use qa_input::{Action, Input, InputPolicy, UserCmdBuilder};
use std::time::Duration;

#[test]
fn sampled_human_and_bot_intents_share_native_rule_scales() {
    for rules in RuleSetId::ALL {
        let mut input = Input::load();
        input.seed(EventTime(0));
        input.button(SeatId::FIRST, Action::Forward, true, Some(26), EventTime(0));
        let policy = InputPolicy::native(rules);
        let human =
            input.build_frame_with_policy(EventTime(100_000_000), &[policy; SeatId::COUNT])[0];
        let fraction = if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
            0.5
        } else {
            1.0
        };
        let bot = UserCmdBuilder::build(
            Duration::from_millis(100),
            EventTime(100_000_000),
            CommandIntent::moving([fraction, 0.0, 0.0]),
            policy,
        );
        assert_eq!(
            human.movement.map(f32::to_bits),
            bot.movement.map(f32::to_bits)
        );
        assert_eq!(human.duration_ns, bot.duration_ns);
        assert_eq!(human.server_time_ms, bot.server_time_ms);
    }
}

#[test]
fn float_rules_keep_fractions_and_short_rules_convert_each_contribution() {
    let intent = CommandIntent {
        strafe: [0.101, 0.0],
        movement: [[0.101, 0.0], [0.101, 0.0], [0.0; 2]],
        ..Default::default()
    };
    for (rules, forward, side) in [
        (RuleSetId::Quake, 20.2, 70.7),
        (RuleSetId::QuakeWorld, 20.0, 70.0),
        (RuleSetId::Quake2, 20.0, 40.0),
        (RuleSetId::Quake2Rerelease, 40.4, 80.8),
        (RuleSetId::Quake3, 12.0, 24.0),
    ] {
        let command = UserCmdBuilder::build(
            Duration::from_millis(16),
            EventTime(16_000_000),
            intent,
            InputPolicy::native(rules),
        );
        assert!((command.movement[0] - forward).abs() < 0.00001);
        assert!((command.movement[1] - side).abs() < 0.00001);
    }
}

#[test]
fn crouch_and_holster_aliases_use_the_same_action_storage() {
    assert_eq!(qa_input::bindings::action("crouch"), Some(Action::Crouch));
    assert_eq!(qa_input::bindings::action("duck"), Some(Action::Crouch));
    let mut input = Input::load();
    input.seed(EventTime(0));
    input.button(SeatId::FIRST, Action::Holster, true, Some(17), EventTime(0));
    input.button(SeatId::FIRST, Action::Crouch, true, Some(18), EventTime(0));
    let policy = InputPolicy::native(RuleSetId::Quake3);
    let command =
        input.build_frame_with_policy(EventTime(100_000_000), &[policy; SeatId::COUNT])[0];
    assert_eq!(command.movement[2], -127.0);
    assert_eq!(
        command.buttons & (buttons::HOLSTER | buttons::CROUCH),
        buttons::HOLSTER | buttons::CROUCH
    );
}
