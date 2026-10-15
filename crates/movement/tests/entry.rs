mod support;
use qa_core::primitives::{PlayerState, RuleSetId, UserCmd, Vec3};
use qa_movement::{pmove, set_bounds};
fn player(rules: RuleSetId) -> PlayerState {
    let mut state = PlayerState {
        movement_rules: rules,
        trace_rules: rules,
        health: 100,
        weapon_state: None,
        view_weapon: qa_core::primitives::ViewWeapon {
            frame: 17,
            ..Default::default()
        },
        ..Default::default()
    };
    set_bounds(&mut state);
    state.body.position = Vec3([0.0, 0.0, 24.0]);
    state.movement.grounded = true;
    state
}

#[test]
fn movement_preserves_module_owned_weapon_readiness_for_every_movement_role() {
    use qa_core::primitives::{ModuleId, ResourceNamespace, WeaponReadiness, WeaponState};
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        for readiness in [
            WeaponReadiness::DeadlineSeconds(4.25),
            WeaponReadiness::RemainingMilliseconds(-197_000_071),
        ] {
            let mut state = player(rules);
            let owned = Some(WeaponState {
                source: RuleSetId::QuakeWorld,
                namespace: ResourceNamespace::Module(ModuleId(17)),
                readiness,
            });
            state.weapon_state = owned;
            let mut world = support::FixtureWorld::default();
            pmove(
                UserCmd {
                    units: qa_gameplay::rules::command_units(rules),
                    duration_ms: 16,
                    server_time_ms: 16,
                    movement: [127.0, 0.0, 0.0],
                    ..Default::default()
                },
                &mut state,
                &mut world,
                Default::default(),
                qa_movement::no_boundary,
            )
            .expect("valid movement query");
            assert_eq!(state.weapon_state, owned);
        }
    }
}
#[test]
fn q3_view_angles_use_native_signed_shorts_and_pitch_delta_clamp() {
    use qa_core::primitives::MovementMode;
    let mut world = support::FixtureWorld::default();
    for (pitch, yaw, roll, expected, delta) in [
        (0., 180., 270., [0., -180., -90.], 0.),
        (0., 450., -450., [0., 90., -90.], 0.),
        (90., 0., 0., [16000. * (360. / 65536.), 0., 0.], -384.),
        (-90., 0., 0., [-16000. * (360. / 65536.), 0., 0.], -65152.),
    ] {
        let mut state = player(RuleSetId::Quake3);
        state.movement.mode = MovementMode::Frozen;
        let command = UserCmd {
            server_time_ms: 16,
            view_angles: Vec3([pitch, yaw, roll]),
            ..Default::default()
        };
        pmove(
            command,
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert_eq!(state.view_angles, Vec3(expected));
        assert_eq!(
            state.movement.delta_angles.0[0].degrees(),
            delta * (360. / 65536.)
        );
        // Native clamping adjusts delta_angles; the same command remains stable.
        pmove(
            UserCmd {
                server_time_ms: 32,
                ..command
            },
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert_eq!(state.view_angles, Vec3(expected));
    }
}

#[test]
fn native_delta_words_wrap_before_short_narrowing_and_only_clamped_pitch_changes() {
    use qa_core::primitives::{AngleOffset, AngleOffsets, MovementMode};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake3] {
        let mut state = player(rules);
        state.movement.mode = MovementMode::Frozen;
        let native = [i32::MIN + 1, i32::MAX - 1, i32::MIN + 1];
        state.movement.delta_angles = AngleOffsets(native.map(AngleOffset::Units));
        pmove(
            UserCmd {
                duration_ms: 16,
                server_time_ms: 16,
                view_angles: Vec3([0.0, 90.0, 0.0]),
                ..Default::default()
            },
            &mut state,
            &mut support::FixtureWorld::default(),
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        // Native short temp retains low16 of (cmd + delta), including an
        // overflowing signed32 addition in yaw; f32 cannot retain these words.
        assert_eq!(
            state.view_angles.0.map(f32::to_bits),
            Vec3([
                1.0 * (360.0 / 65536.0),
                16382.0 * (360.0 / 65536.0),
                1.0 * (360.0 / 65536.0)
            ])
            .0
            .map(f32::to_bits)
        );
        assert_eq!(
            state.movement.delta_angles.0,
            native.map(AngleOffset::Units)
        );
    }
    let mut state = player(RuleSetId::Quake3);
    state.movement.mode = MovementMode::Frozen;
    state.movement.delta_angles = AngleOffsets([
        AngleOffset::Units(0),
        AngleOffset::Units(i32::MAX - 1),
        AngleOffset::Degrees(-0.0),
    ]);
    pmove(
        UserCmd {
            server_time_ms: 16,
            view_angles: Vec3([90.0, 0.0, 0.0]),
            ..Default::default()
        },
        &mut state,
        &mut support::FixtureWorld::default(),
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert_eq!(state.movement.delta_angles.0[0], AngleOffset::Units(-384));
    assert_eq!(
        state.movement.delta_angles.0[1],
        AngleOffset::Units(i32::MAX - 1)
    );
    assert_eq!(
        state.movement.delta_angles.0[2].degrees().to_bits(),
        0x8000_0000
    );
}
#[test]
fn native_command_scheduling_preserves_qw_odd_halves_and_q3_absolute_time() {
    let mut world = support::FixtureWorld::default();
    let mut qw = player(RuleSetId::QuakeWorld);
    let result = pmove(
        UserCmd {
            units: qa_gameplay::rules::command_units(RuleSetId::QuakeWorld),
            duration_ms: 101,
            movement: [320.0, 0.0, 0.0],
            ..Default::default()
        },
        &mut qw,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    let mut expected = player(RuleSetId::QuakeWorld);
    for _ in 0..2 {
        pmove(
            UserCmd {
                units: qa_gameplay::rules::command_units(RuleSetId::QuakeWorld),
                duration_ms: 50,
                movement: [320.0, 0.0, 0.0],
                ..Default::default()
            },
            &mut expected,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
    }
    assert_eq!(result.steps, 2);
    assert_eq!(qw.body.position, expected.body.position);
    assert_eq!(qw.body.velocity, expected.body.velocity);
    let mut q3 = player(RuleSetId::Quake3);
    assert_eq!(
        pmove(
            UserCmd {
                server_time_ms: 200,
                ..Default::default()
            },
            &mut q3,
            &mut world,
            Default::default(),
            qa_movement::no_boundary
        )
        .expect("valid movement query")
        .steps,
        4
    );
    assert_eq!(q3.movement.command_time_ms, 200);
    assert_eq!(
        pmove(
            UserCmd {
                server_time_ms: 199,
                ..Default::default()
            },
            &mut q3,
            &mut world,
            Default::default(),
            qa_movement::no_boundary
        )
        .expect("valid movement query")
        .steps,
        0
    );
    assert_eq!(
        pmove(
            UserCmd {
                server_time_ms: 2000,
                ..Default::default()
            },
            &mut q3,
            &mut world,
            Default::default(),
            qa_movement::no_boundary
        )
        .expect("valid movement query")
        .steps,
        16
    );
}
#[test]
fn netquake_uses_precise_duration_and_module_owns_jump() {
    let mut state = player(RuleSetId::Quake);
    let mut world = support::FixtureWorld::default();
    let command = UserCmd {
        units: qa_gameplay::rules::command_units(RuleSetId::Quake),
        duration_ms: 11,
        duration_ns: 11_764_705,
        movement: [320.0, 0.0, 200.0],
        ..Default::default()
    };
    pmove(
        command,
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    let speed = (10.0f64 * 0.011764705 * 320.0) as f32;
    assert_eq!(state.body.velocity.0[0], speed);
    assert_eq!(state.body.position.0[0], speed * 0.011764705f32);
    assert_eq!(state.body.position.0[2], 24.0);
    assert_eq!(state.weapon_state, None);
    assert_eq!(state.view_weapon.frame, 17);
}
#[test]
fn classic_quantizes_while_rerelease_keeps_floating_state() {
    let mut world = support::FixtureWorld::default();
    let mut classic = player(RuleSetId::Quake2);
    let mut rr = player(RuleSetId::Quake2Rerelease);
    classic.body.position.0[0] = 0.06;
    rr.body.position.0[0] = 0.06;
    let command = UserCmd {
        units: qa_gameplay::rules::command_units(RuleSetId::Quake2),
        duration_ms: 16,
        movement: [300.0, 0.0, 0.0],
        ..Default::default()
    };
    pmove(
        command,
        &mut classic,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    pmove(
        command,
        &mut rr,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert_eq!(classic.body.position.0[0], 0.75);
    assert!((rr.body.position.0[0] - 0.828).abs() < 0.000001);
    assert_eq!(classic.body.mins.0[2], rr.body.mins.0[2]);
}
