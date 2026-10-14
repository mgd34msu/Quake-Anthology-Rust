use qa_core::primitives::{
    AngleOffset, AngleOffsets, Body, MovementMode, MovementTimer, PlayerState, PredictionPolicy,
    RuleSetId, UserCmd, Vec3,
};
use qa_movement::TraceServices;
use qa_session::prediction::Prediction;
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};

struct NoQueries;
impl TraceServices for NoQueries {
    fn trace(&mut self, _: TraceQuery) -> Result<Trace, TraceError> {
        panic!("nonpositional prediction must not trace")
    }
    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        panic!("nonpositional prediction must not sample contents")
    }
}

fn body_bits(body: Body) -> [[u32; 3]; 4] {
    [body.position, body.velocity, body.mins, body.maxs].map(|v| v.0.map(f32::to_bits))
}

#[test]
fn angles_only_prediction_uses_native_unclamped_command_delta_without_movement() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake2] {
        for command_angles in [Vec3([179.0, -0.0, 360.0]), Vec3([-270.0, 721.0, -17.25])] {
            let mut prediction = Prediction::default();
            let player = &mut prediction.player;
            player.movement_rules = rules;
            player.trace_rules = RuleSetId::QuakeWorld;
            player.prediction = PredictionPolicy {
                position: false,
                angles: true,
            };
            player.body.position = Vec3([17.125, -0.0, 137.0]);
            player.body.velocity = Vec3([3.0, 7.0, -1.0]);
            player.view_angles = Vec3([-19.0, 117.0, 7.0]);
            player.movement.delta_angles = AngleOffsets([
                AngleOffset::Units(8192),
                AngleOffset::Units(-32768),
                AngleOffset::Units(2048),
            ]);
            player.movement.timer = MovementTimer::TELEPORT;
            player.movement.remaining_ms = 17;
            player.movement.command_time_ms = 73;
            let body = body_bits(player.body);
            let movement = player.movement;
            // Original Q2 cl_pred.c:216-223. This branch neither quantizes the
            // command nor applies Pmove's pitch/teleport handling.
            let expected = command_angles + player.movement.delta_angles.degrees();
            let result = prediction
                .advance(
                    UserCmd {
                        duration_ms: 16,
                        server_time_ms: 100,
                        movement: [127.0, 64.0, 32.0],
                        view_angles: command_angles,
                        ..Default::default()
                    },
                    &mut NoQueries,
                )
                .unwrap();
            assert_eq!(
                (result.steps, result.traces, result.contact_count),
                (0, 0, 0)
            );
            assert_eq!(body_bits(prediction.player.body), body);
            assert_eq!(prediction.player.movement, movement);
            assert_eq!(
                prediction.player.view_angles.0.map(f32::to_bits),
                expected.0.map(f32::to_bits)
            );
        }
    }
}

#[test]
fn foreign_no_position_permission_retains_q3_angle_rounding_and_pitch_delta_adjustment() {
    let mut prediction = Prediction::default();
    let player = &mut prediction.player;
    player.health = 100;
    player.movement_rules = RuleSetId::Quake3;
    player.trace_rules = RuleSetId::Quake2;
    player.prediction = PredictionPolicy {
        position: false,
        angles: true,
    };
    player.body.position = Vec3([13.0, -0.0, 27.0]);
    player.movement.delta_angles = AngleOffsets([AngleOffset::Units(8192); 3]);
    player.movement.command_time_ms = 1000;
    player.movement.remaining_ms = 19;
    player.movement.timer = MovementTimer::TELEPORT;
    let body = body_bits(player.body);
    let command = UserCmd {
        duration_ms: 16,
        server_time_ms: 16,
        view_angles: Vec3([179.0, 11.25, -0.0]),
        ..Default::default()
    };
    prediction.advance(command, &mut NoQueries).unwrap();
    assert_eq!(body_bits(prediction.player.body), body);
    assert_eq!(prediction.player.movement.command_time_ms, 1000);
    assert_eq!(prediction.player.movement.remaining_ms, 19);
    assert_eq!(prediction.player.movement.timer, MovementTimer::TELEPORT);
    assert_eq!(
        prediction.player.view_angles,
        Vec3([qa_core::math::short_to_angle(-16000), 56.25, 45.0])
    );
    assert_eq!(
        prediction.player.movement.delta_angles.0[0],
        AngleOffset::Units(
            -16000
                - i32::from(qa_core::math::angle_to_short(
                    command.view_angles.0[0],
                    qa_core::math::AngleShortForm::Factored
                ) as u16)
        )
    );
}

#[test]
fn disabled_angles_move_using_the_accepted_view_without_mutating_delta_pitch() {
    let mut prediction = Prediction::default();
    let player = &mut prediction.player;
    player.health = 100;
    player.movement_rules = RuleSetId::Quake3;
    player.trace_rules = RuleSetId::Quake;
    player.movement.mode = MovementMode::Noclip;
    player.prediction = PredictionPolicy {
        position: true,
        angles: false,
    };
    player.view_angles = Vec3([-0.0, 90.0, 0.0]);
    player.movement.delta_angles = AngleOffsets([AngleOffset::Units(17000); 3]);
    let accepted = player.view_angles;
    let delta = player.movement.delta_angles;
    let command = UserCmd {
        duration_ms: 16,
        server_time_ms: 16,
        movement: [127.0, 0.0, 0.0],
        view_angles: Vec3([180.0, 0.0, 17.0]),
        ..Default::default()
    };
    let result = prediction.advance(command, &mut NoQueries).unwrap();
    assert!(result.steps > 0);
    assert!(prediction.player.body.position.0[1] > 0.0);
    assert_eq!(
        prediction.player.view_angles.0.map(f32::to_bits),
        accepted.0.map(f32::to_bits)
    );
    assert_eq!(prediction.player.movement.delta_angles, delta);

    // SERVER uses the same mover with its ordinary invocation policy; CLIENT's
    // prediction permission must never become a server movement suppression.
    qa_movement::pmove(
        UserCmd {
            server_time_ms: 32,
            ..command
        },
        &mut prediction.player,
        &mut NoQueries,
        Default::default(),
        qa_movement::no_boundary,
    )
    .unwrap();
    assert_ne!(
        prediction.player.view_angles.0.map(f32::to_bits),
        accepted.0.map(f32::to_bits)
    );
}

#[test]
fn both_disabled_preserves_every_hot_field_even_with_a_nonzero_command() {
    let mut prediction = Prediction::default();
    prediction.player.prediction = PredictionPolicy::DISABLED;
    prediction.player.view_angles = Vec3([-0.0, 133.0, -19.0]);
    let view = prediction.player.view_angles;
    let body = body_bits(prediction.player.body);
    let movement = prediction.player.movement;
    prediction
        .advance(
            UserCmd {
                duration_ms: 100,
                server_time_ms: 200,
                movement: [127.0; 3],
                view_angles: Vec3([90.0; 3]),
                ..Default::default()
            },
            &mut NoQueries,
        )
        .unwrap();
    assert_eq!(prediction.player.movement, movement);
    assert_eq!(body_bits(prediction.player.body), body);
    assert_eq!(
        prediction.player.view_angles.0.map(f32::to_bits),
        view.0.map(f32::to_bits)
    );
}
