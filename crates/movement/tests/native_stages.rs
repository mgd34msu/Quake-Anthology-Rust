use qa_core::{
    math,
    primitives::{
        MotionWrite, MovementMode, MovementTimer, PlayerState, RuleSetId, ThinkTime, UserCmd, Vec3,
        buttons,
    },
};
use qa_movement::{MoveOptions, MovePhase, TraceServices, no_boundary, pmove};
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};

struct NoTrace;
impl TraceServices for NoTrace {
    fn trace(&mut self, _: TraceQuery) -> Result<Trace, TraceError> {
        panic!("staged/direct NOCLIP must not trace");
    }
    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        panic!("staged/direct NOCLIP must not query contents");
    }
}
fn player(rules: RuleSetId) -> PlayerState {
    let mut player = PlayerState {
        movement_rules: rules,
        trace_rules: RuleSetId::Quake2,
        health: 100,
        ..Default::default()
    };
    player.movement.mode = MovementMode::Noclip;
    player.body.position = Vec3([3.0, 4.0, 5.0]);
    player.body.velocity = Vec3([2.0, 3.0, 4.0]);
    player.view_angles = Vec3([123.5, -14.25, 6.0]);
    player.view_offset = Vec3([1.0, 2.0, 3.0]);
    player.movement.timer = MovementTimer::TELEPORT;
    player.movement.remaining_ms = 123;
    player
}
fn control(angles: Vec3, clock: f64) -> MoveOptions {
    MoveOptions {
        phase: MovePhase::Control {
            body_angles: angles,
            clock: ThinkTime::Seconds(clock),
        },
        ..Default::default()
    }
}
fn integrate(seconds: f64) -> MoveOptions {
    MoveOptions {
        phase: MovePhase::Integrate { seconds },
        ..Default::default()
    }
}

#[test]
fn complete_and_native_control_share_raw_and_capped_velocity_and_scalar_order() {
    for (axes, expected) in [
        ([1.0, -1.0, 0.0], Vec3([1.0, 1.0, 0.0])),
        ([300.0, -400.0, 0.0], Vec3([192.0, 256.0, 0.0])),
    ] {
        let mut complete = player(RuleSetId::Quake);
        let mut native = player(RuleSetId::Quake);
        let cmd = UserCmd {
            duration_ms: 16,
            duration_ns: 16_000_000,
            movement: axes,
            ..Default::default()
        };
        let mut complete_writes = Vec::new();
        let complete_result = pmove(
            cmd,
            &mut complete,
            &mut NoTrace,
            MoveOptions::default(),
            |_: &mut PlayerState, field| {
                complete_writes.push(field);
                Ok::<_, TraceError>(())
            },
        )
        .unwrap();
        let mut native_writes = Vec::new();
        let native_result = pmove(
            cmd,
            &mut native,
            &mut NoTrace,
            control(Vec3::default(), 7.0),
            |_: &mut PlayerState, field| {
                native_writes.push(field);
                Ok::<_, TraceError>(())
            },
        )
        .unwrap();
        assert_eq!(
            complete.body.velocity.0.map(f32::to_bits),
            expected.0.map(f32::to_bits)
        );
        assert_eq!(
            native.body.velocity.0.map(f32::to_bits),
            expected.0.map(f32::to_bits)
        );
        assert_eq!(
            complete_writes,
            [
                MotionWrite::Velocity(0),
                MotionWrite::Velocity(1),
                MotionWrite::Velocity(2),
                MotionWrite::Origin(0),
                MotionWrite::Origin(1),
                MotionWrite::Origin(2)
            ]
        );
        assert_eq!(
            native_writes,
            [
                MotionWrite::Velocity(0),
                MotionWrite::Velocity(1),
                MotionWrite::Velocity(2)
            ]
        );
        assert_eq!(complete_result.steps, 1);
        assert_eq!(native_result.steps, 0);
        let native_view = native.view_angles;
        pmove(
            cmd,
            &mut native,
            &mut NoTrace,
            integrate(0.016),
            no_boundary,
        )
        .unwrap();
        assert_eq!(
            complete.body.position.0.map(f32::to_bits),
            native.body.position.0.map(f32::to_bits)
        );
        assert_eq!(native.view_angles, native_view);
        assert_eq!(native.movement.remaining_ms, 123);
        assert_eq!(native.trace_rules, RuleSetId::Quake2);
    }
}

#[test]
fn native_control_keeps_raw_under_cap_wish_and_separate_integration() {
    let mut state = player(RuleSetId::Quake);
    state.movement.ducked = true;
    let command = UserCmd {
        movement: [3.0, 2.0, 6.0],
        buttons: buttons::JUMP | buttons::CROUCH,
        duration_ms: 1,
        duration_ns: 1,
        ..Default::default()
    };
    let position = state.body.position;
    let view = state.view_angles;
    let offset = state.view_offset;
    let result = pmove(
        command,
        &mut state,
        &mut NoTrace,
        control(Vec3::default(), 7.0),
        no_boundary,
    )
    .unwrap();
    assert_eq!(result.steps, 0);
    assert_eq!(state.body.position, position);
    assert_eq!(
        state.body.velocity.0.map(f32::to_bits),
        [3.0f32.to_bits(), (-2.0f32).to_bits(), 6.0f32.to_bits()]
    );
    assert_eq!(state.view_angles, view);
    assert_eq!(state.view_offset, offset);
    assert_eq!(state.movement.remaining_ms, 123);
    let result = pmove(
        command,
        &mut state,
        &mut NoTrace,
        integrate(0.333),
        no_boundary,
    )
    .unwrap();
    assert_eq!(result.steps, 1);
    assert_eq!(
        state.body.position.0.map(f32::to_bits),
        std::array::from_fn::<_, 3, _>(
            |i| (position.0[i] + 0.333f32 * state.body.velocity.0[i]).to_bits()
        )
    );
    assert_eq!(state.movement.remaining_ms, 123);
}

#[test]
fn native_control_caps_raw_wish_and_uses_explicit_body_pitch_roll() {
    for (angles, axes) in [
        (Vec3::default(), [3.0, 2.0, 6.0]),
        (Vec3([40.0, 17.0, 33.0]), [11.25, -3.5, -7.125]),
    ] {
        let mut state = player(RuleSetId::Quake);
        state.movement.tuning.max_speed = Some(5.0.into());
        let basis = math::angle_vectors(angles);
        let mut expected = basis.forward * axes[0] + basis.right * axes[1];
        expected.0[2] = axes[2];
        let length = math::length(expected);
        if length > 5.0 {
            expected = expected * (5.0 / length);
        }
        pmove(
            UserCmd {
                movement: axes,
                ..Default::default()
            },
            &mut state,
            &mut NoTrace,
            control(angles, 7.0),
            no_boundary,
        )
        .unwrap();
        assert_eq!(
            state.body.velocity.0.map(f32::to_bits),
            expected.0.map(f32::to_bits)
        );
    }
}

#[test]
fn native_reverse_gate_uses_absolute_module_time_and_strict_tie() {
    for (clock, forward) in [(9.999, 0.0), (10.0, -12.0), (10.001, -12.0)] {
        let mut state = player(RuleSetId::Quake);
        state.movement.teleport_hold_until = 10.0;
        let command = UserCmd {
            movement: [-12.0, 0.0, 4.0],
            server_time_ms: 1_000_000,
            ..Default::default()
        };
        pmove(
            command,
            &mut state,
            &mut NoTrace,
            control(Vec3::default(), clock),
            no_boundary,
        )
        .unwrap();
        assert_eq!(state.body.velocity.0[0], forward);
        assert_eq!(state.body.velocity.0[2], 4.0);
    }
}

#[test]
fn native_integrate_rereads_shifted_aliases_without_duration_clamping() {
    let mut state = player(RuleSetId::Quake);
    let mut words = [2.0_f32, 3.0, 4.0, 5.0];
    let mut writes = Vec::new();
    let result = pmove(
        UserCmd {
            duration_ms: 1,
            duration_ns: 1,
            ..Default::default()
        },
        &mut state,
        &mut NoTrace,
        integrate(2.0),
        |player: &mut PlayerState, write| {
            let MotionWrite::Origin(axis) = write else {
                panic!("integrate is position only")
            };
            words[axis + 1] = player.body.position.0[axis];
            player.body.position = Vec3([words[1], words[2], words[3]]);
            player.body.velocity = Vec3([words[0], words[1], words[2]]);
            writes.push(write);
            Ok::<_, TraceError>(())
        },
    )
    .unwrap();
    assert_eq!(words, [2.0, 7.0, 18.0, 41.0]);
    assert_eq!(state.body.position, Vec3([7.0, 18.0, 41.0]));
    assert_eq!(
        writes,
        [
            MotionWrite::Origin(0),
            MotionWrite::Origin(1),
            MotionWrite::Origin(2)
        ]
    );
    assert_eq!(result.steps, 1);
}

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Boundary,
    Trace(TraceError),
}
impl From<TraceError> for Failure {
    fn from(error: TraceError) -> Self {
        Self::Trace(error)
    }
}
#[test]
fn staging_failure_preserves_the_native_admitted_prefix() {
    let mut state = player(RuleSetId::Quake);
    let mut writes = Vec::new();
    let error = pmove(
        UserCmd::default(),
        &mut state,
        &mut NoTrace,
        integrate(2.0),
        |_, write| {
            writes.push(write);
            if write == MotionWrite::Origin(1) {
                Err(Failure::Boundary)
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert_eq!(error, Failure::Boundary);
    assert_eq!(writes, [MotionWrite::Origin(0), MotionWrite::Origin(1)]);
    assert_eq!(state.body.position, Vec3([7.0, 10.0, 5.0]));
}

#[test]
fn foreign_control_is_noop_and_q3_acceleration_survives_native_readback() {
    let command = UserCmd {
        movement: [127.0, -33.0, 17.0],
        view_angles: Vec3([13.0, 11.0, 7.0]),
        server_time_ms: 150,
        duration_ms: 150,
        ..Default::default()
    };
    let mut actual = player(RuleSetId::Quake3);
    let mut expected = player(RuleSetId::Quake3);
    let initial_position = actual.body.position;
    let initial_velocity = actual.body.velocity;
    let result = pmove(
        command,
        &mut actual,
        &mut NoTrace,
        control(Vec3([40.0, 70.0, 12.0]), 99.0),
        |_, _| -> Result<(), TraceError> { panic!("foreign Control is empty") },
    )
    .unwrap();
    assert_eq!(result.steps, 0);
    assert_eq!(actual.body.position, initial_position);
    assert_eq!(actual.body.velocity, initial_velocity);
    let expected_result = pmove(
        command,
        &mut expected,
        &mut NoTrace,
        Default::default(),
        no_boundary,
    )
    .unwrap();
    let mut native_position = actual.body.position;
    let mut native_velocity = actual.body.velocity;
    let mut writes = Vec::new();
    let retained_command = UserCmd {
        server_time_ms: 0,
        duration_ms: 0,
        duration_ns: 0,
        ..command
    };
    let actual_result = pmove(
        retained_command,
        &mut actual,
        &mut NoTrace,
        integrate(0.15),
        |player: &mut PlayerState, write| {
            match write {
                MotionWrite::Velocity(axis) => {
                    native_velocity.0[axis] = player.body.velocity.0[axis]
                }
                MotionWrite::Origin(axis) => native_position.0[axis] = player.body.position.0[axis],
                MotionWrite::Angles(_) => panic!("client NOCLIP never integrates angular velocity"),
            }
            player.body.position = native_position;
            player.body.velocity = native_velocity;
            writes.push(write);
            Ok::<_, TraceError>(())
        },
    )
    .unwrap();
    assert_eq!(actual_result.steps, 3);
    assert_eq!(actual_result.steps, expected_result.steps);
    assert_eq!(
        actual.body.position.0.map(f32::to_bits),
        expected.body.position.0.map(f32::to_bits)
    );
    assert_eq!(
        actual.body.velocity.0.map(f32::to_bits),
        expected.body.velocity.0.map(f32::to_bits)
    );
    assert_eq!(actual.trace_rules, RuleSetId::Quake2);
    for chunk in writes.chunks_exact(6) {
        assert_eq!(
            chunk,
            [
                MotionWrite::Velocity(0),
                MotionWrite::Velocity(1),
                MotionWrite::Velocity(2),
                MotionWrite::Origin(0),
                MotionWrite::Origin(1),
                MotionWrite::Origin(2)
            ]
        );
    }
    assert_eq!(writes.len(), 18);
}

struct ClearTrace;
impl TraceServices for ClearTrace {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
        Ok(Trace::clear(query.end))
    }
    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        Ok(Contents::EMPTY)
    }
}

#[test]
fn foreign_q2_final_snap_is_published_after_direct_native_motion() {
    let mut actual = player(RuleSetId::Quake2);
    actual.body.position = Vec3::default();
    actual.body.velocity = Vec3::default();
    let mut native_position = actual.body.position;
    let mut native_velocity = actual.body.velocity;
    let mut origin_x = Vec::new();
    let result = pmove(
        UserCmd {
            movement: [50.0, 0.0, 0.0],
            ..Default::default()
        },
        &mut actual,
        &mut ClearTrace,
        integrate(0.001),
        |player: &mut PlayerState, write| {
            match write {
                MotionWrite::Velocity(axis) => {
                    native_velocity.0[axis] = player.body.velocity.0[axis]
                }
                MotionWrite::Origin(axis) => {
                    native_position.0[axis] = player.body.position.0[axis];
                    if axis == 0 {
                        origin_x.push(native_position.0[axis]);
                    }
                }
                MotionWrite::Angles(_) => panic!("client motion is position/velocity only"),
            }
            player.body.position = native_position;
            player.body.velocity = native_velocity;
            Ok::<_, TraceError>(())
        },
    )
    .unwrap();
    assert_eq!(result.steps, 1);
    assert_eq!(actual.body.velocity, Vec3([0.5, 0.0, 0.0]));
    assert_eq!(actual.body.position, Vec3::default());
    assert_eq!(native_position, actual.body.position);
    assert_eq!(native_velocity, actual.body.velocity);
    assert_eq!(origin_x.len(), 2);
    // Native PM_Accelerate multiplies accel, dt, then wishspeed before
    // Direct motion. Its pre-snap velocity is one ULP above 0.5 here.
    assert_eq!(
        origin_x[0].to_bits(),
        (0.001f32 * (10.0f32 * 0.001f32 * 50.0f32)).to_bits()
    );
    assert_eq!(origin_x[1].to_bits(), 0.0f32.to_bits());
}

#[test]
fn slide_motion_publishes_its_finished_native_body_through_the_same_boundary() {
    let command = UserCmd {
        movement: [127.0, 0.0, 0.0],
        server_time_ms: 16,
        duration_ms: 16,
        ..Default::default()
    };
    let mut actual = player(RuleSetId::Quake3);
    let mut expected = player(RuleSetId::Quake3);
    actual.movement.mode = MovementMode::Spectator;
    expected.movement.mode = MovementMode::Spectator;
    actual.body.velocity = Vec3::default();
    expected.body.velocity = Vec3::default();
    let expected_result = pmove(
        command,
        &mut expected,
        &mut ClearTrace,
        Default::default(),
        no_boundary,
    )
    .unwrap();
    let mut native_position = actual.body.position;
    let mut native_velocity = actual.body.velocity;
    let mut writes = Vec::new();
    let actual_result = pmove(
        UserCmd {
            server_time_ms: 0,
            duration_ms: 0,
            ..command
        },
        &mut actual,
        &mut ClearTrace,
        integrate(0.016),
        |player: &mut PlayerState, write| {
            match write {
                MotionWrite::Velocity(axis) => {
                    native_velocity.0[axis] = player.body.velocity.0[axis]
                }
                MotionWrite::Origin(axis) => native_position.0[axis] = player.body.position.0[axis],
                MotionWrite::Angles(_) => panic!("client motion is position/velocity only"),
            }
            player.body.position = native_position;
            player.body.velocity = native_velocity;
            writes.push(write);
            Ok::<_, TraceError>(())
        },
    )
    .unwrap();
    assert_eq!(actual_result.steps, expected_result.steps);
    assert_eq!(actual_result.traces, expected_result.traces);
    assert!(actual_result.traces > 0);
    assert_eq!(
        native_position.0.map(f32::to_bits),
        expected.body.position.0.map(f32::to_bits)
    );
    assert_eq!(
        native_velocity.0.map(f32::to_bits),
        expected.body.velocity.0.map(f32::to_bits)
    );
    assert!(writes.contains(&MotionWrite::Velocity(0)));
    assert!(writes.contains(&MotionWrite::Origin(0)));
}
