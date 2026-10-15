use qa_core::{
    primitives::{RuleSetId, UserCmd, Vec3, WeaponId},
    sys_events::EventTime,
};
use qa_movement::{prepare_command, quakeworld_steps};

#[test]
fn simulation_duration_rules_keep_server_time_and_command_fields() {
    // qsrc Q1 Host_FilterTime, QW SV_RunCmd, Q2 Pmove and Q3 PmoveSingle.
    for (raw, expected) in [
        (0, [1, 0, 0, 0, 1]),
        (50, [50; 5]),
        (200, [100, 200, 200, 200, 200]),
        (250, [100, 250, 250, 250, 200]),
        (251, [100, 251, 251, 251, 200]),
        (255, [100, 255, 255, 255, 200]),
        (1000, [100, 1000, 1000, 255, 200]),
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
                    movement: [200.0, -150.0, 10.0],
                    buttons: 129,
                    ..UserCmd::default()
                },
            );
            assert_eq!(command.duration_ms, value);
            assert_eq!(command.server_time_ms, 5500);
            assert_eq!(command.movement, [200.0, -150.0, 10.0]);
            assert_eq!(command.buttons, 129);
        }
    }
}

#[test]
fn quakeworld_steps_discard_odd_remainders_at_each_split() {
    for (duration_ms, expected) in [(101, vec![50, 50]), (203, vec![50, 50, 50, 50])] {
        let steps = quakeworld_steps(UserCmd {
            duration_ms,
            duration_ns: u64::MAX,
            ..Default::default()
        })
        .collect::<Vec<_>>();
        assert_eq!(
            steps
                .iter()
                .map(|step| step.duration_ms)
                .collect::<Vec<_>>(),
            expected
        );
        for step in steps {
            assert_eq!(
                step.duration_ns,
                EventTime::from_milliseconds(u64::from(step.duration_ms)).0
            );
        }
    }
}

#[test]
fn quakeworld_steps_keep_impulse_only_on_first_leaf_and_preserve_fields() {
    let command = UserCmd {
        duration_ms: 255,
        duration_ns: 7,
        server_time_ms: -12345,
        view_angles: Vec3([-0.0, 217.5, -90.0]),
        movement: [320.0, -0.0, -125.5],
        buttons: 0x8765_4321,
        impulse: 177,
        light_level: 245,
        weapon: Some(WeaponId(23)),
    };
    let steps = quakeworld_steps(command).collect::<Vec<_>>();
    assert_eq!(steps.len(), 8);
    assert_eq!(
        steps
            .iter()
            .map(|step| u32::from(step.duration_ms))
            .sum::<u32>(),
        248
    );
    for (index, step) in steps.iter().enumerate() {
        assert_eq!(step.duration_ms, 31);
        assert_eq!(step.impulse, if index == 0 { command.impulse } else { 0 });
        assert_eq!(step.server_time_ms, command.server_time_ms);
        assert_eq!(
            step.view_angles.0.map(f32::to_bits),
            command.view_angles.0.map(f32::to_bits)
        );
        assert_eq!(
            step.movement.map(f32::to_bits),
            command.movement.map(f32::to_bits)
        );
        assert_eq!(step.buttons, command.buttons);
        assert_eq!(step.light_level, command.light_level);
        assert_eq!(step.weapon, command.weapon);
    }
}

#[test]
fn quakeworld_steps_keep_zero_and_short_commands_as_single_leaves() {
    for duration_ms in [0, 1, 49, 50] {
        let mut steps = quakeworld_steps(UserCmd {
            duration_ms,
            impulse: 9,
            ..Default::default()
        });
        let step = steps.next().expect("one native leaf");
        assert_eq!(step.duration_ms, duration_ms);
        assert_eq!(step.impulse, 9);
        assert!(steps.next().is_none());
        assert!(steps.next().is_none());
    }
}

#[test]
fn quakeworld_steps_preserve_raw_duration_before_subdivision() {
    for (duration_ms, count, leaf_ms) in [
        (51, 2, 25),
        (100, 2, 50),
        (200, 4, 50),
        (250, 8, 31),
        (251, 8, 31),
        (255, 8, 31),
        (256, 8, 32),
        (u16::MAX, 2048, 31),
    ] {
        let steps = quakeworld_steps(UserCmd {
            duration_ms,
            ..Default::default()
        })
        .collect::<Vec<_>>();
        assert_eq!(steps.len(), count);
        assert!(steps.iter().all(|step| step.duration_ms == leaf_ms));
    }
}

#[test]
fn quakeworld_steps_accept_every_native_byte_duration_with_bounded_leaves() {
    for duration_ms in 0..=u16::from(u8::MAX) {
        let mut count = 0;
        for step in quakeworld_steps(UserCmd {
            duration_ms,
            ..Default::default()
        }) {
            count += 1;
            assert!(step.duration_ms <= 50);
            assert_eq!(
                step.duration_ns,
                EventTime::from_milliseconds(u64::from(step.duration_ms)).0
            );
        }
        assert!((1..=8).contains(&count));
    }
}

struct EmptyWorld;
impl qa_movement::TraceServices for EmptyWorld {
    fn trace(
        &mut self,
        query: qa_world::collision::TraceQuery,
    ) -> Result<qa_world::collision::Trace, qa_world::collision::TraceError> {
        Ok(qa_world::collision::Trace::clear(query.end))
    }
    fn point_contents(
        &self,
        _: Vec3,
        _: qa_world::collision::EntityTracePolicy,
    ) -> Result<qa_world::collision::Contents, qa_world::collision::TraceError> {
        Ok(qa_world::collision::Contents::EMPTY)
    }
}

#[test]
fn classic_pmove_consumes_raw_native_255ms_without_client_finish_move() {
    let mut player = qa_core::primitives::PlayerState {
        movement_rules: RuleSetId::Quake2,
        trace_rules: RuleSetId::Quake2,
        health: 100,
        ..Default::default()
    };
    qa_movement::set_bounds(&mut player);
    player.body.position = Vec3([0.0, 0.0, 1000.0]);
    player.body.velocity = Vec3([100.0, 0.0, 0.0]);
    player.movement.tuning.gravity = Some(0.0.into());
    let result = qa_movement::pmove(
        UserCmd {
            duration_ms: 255,
            ..Default::default()
        },
        &mut player,
        &mut EmptyWorld,
        Default::default(),
        qa_movement::no_boundary,
    )
    .unwrap();
    assert_eq!(result.steps, 1);
    assert_eq!(player.body.position, Vec3([25.5, 0.0, 1000.0]));
    assert_eq!(player.body.velocity, Vec3([100.0, 0.0, 0.0]));
}
