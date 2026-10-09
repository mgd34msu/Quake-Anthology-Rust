use qa_core::primitives::{RuleSetId, UserCmd};
use qa_movement::prepare_command;

#[test]
fn original_duration_rules_keep_server_time_and_command_fields() {
    // qsrc Q1 Host_FilterTime, QW/Q2 CL_FinishMove, Q3 PmoveSingle.
    for (raw, expected) in [
        (0, [1, 0, 0, 0, 1]),
        (50, [50; 5]),
        (200, [100, 200, 200, 200, 200]),
        (250, [100, 250, 250, 250, 200]),
        (251, [100, 100, 100, 251, 200]),
        (1000, [100, 100, 100, 255, 200]),
    ] {
        for (rule, value) in [
            RuleSetId::Quake,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake2,
            RuleSetId::Quake2Rerelease,
            RuleSetId::Quake3,
        ]
        .into_iter()
        .zip(expected)
        {
            let command = prepare_command(
                rule,
                UserCmd {
                    duration_ms: raw,
                    server_time_ms: 5500,
                    movement: [200, -150, 10],
                    buttons: 129,
                    ..UserCmd::default()
                },
            );
            assert_eq!(command.duration_ms, value);
            assert_eq!(command.server_time_ms, 5500);
            assert_eq!(command.movement, [200, -150, 10]);
            assert_eq!(command.buttons, 129);
        }
    }
}
