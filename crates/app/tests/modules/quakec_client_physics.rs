use super::*;

fn append_begin(vm: &mut Vm, extra: &[(qa_formats::program::quakec::Opcode, i16, i16, i16)]) {
    let first = vm.image.functions[3].first_statement as usize;
    let mut body = vm.image.statements[first..]
        .iter()
        .take_while(|statement| statement.opcode != Return)
        .map(|statement| {
            (
                statement.opcode,
                statement.operands[0],
                statement.operands[1],
                statement.operands[2],
            )
        })
        .collect::<Vec<_>>();
    body.extend_from_slice(extra);
    body.push((Return, 0, 0, 0));
    replace_client_function(vm, 3, &body);
}

fn mode_vm(mode: f32, velocity: Vec3, alive: bool) -> Vm {
    let mut vm = noclip_client_vm(velocity);
    vm.image.globals[126] = mode.to_bits();
    vm.image.globals[83] = if alive { 100.0f32 } else { 0.0f32 }.to_bits();
    vm.image.globals[132..135].copy_from_slice(&[2.0f32, 4.0, 6.0].map(f32::to_bits));
    for entry in [4, 7] {
        replace_client_function(&mut vm, entry, &[(Return, 0, 0, 0)]);
    }
    vm.image.globals[110] = 0;
    replace_client_function(&mut vm, 5, &[(LoadV, 34, 110, 150), (Return, 150, 0, 0)]);
    vm
}

fn queue(host: &mut FrameHost, id: ClientId, movement: [f32; 3]) {
    assert!(host.runtime.send_local_command(
        id,
        &UserCmd {
            movement,
            units: qa_gameplay::rules::command_units(RuleSetId::Quake),
            ..Default::default()
        },
        EventTime::from_milliseconds(20),
    ));
    host.drain(&mut Default::default());
}

fn assert_admitted(host: &FrameHost) {
    let counts = host.module_counts(ModuleId(1)).unwrap();
    assert_eq!(
        (counts.rejected, counts.traps, counts.unsupported_physics),
        (0, 0, 0)
    );
}

#[test]
fn native_provider_publication_sets_current_player_mode_and_preserves_spectator() {
    for (mode, expected) in [
        (0.0, MovementMode::Frozen),
        (3.0, MovementMode::Walk),
        (8.0, MovementMode::Noclip),
    ] {
        let vm = mode_vm(mode, Vec3::default(), true);
        let (mut host, id, _, _, _) = qc_client_host(RuleSetId::Quake, RuleSetId::Quake, vm, &[]);
        host.connect_module_client(ModuleId(1), id, b"\\name\\published mode")
            .unwrap();
        assert_eq!(
            host.runtime.server.clients[id.0 as usize]
                .player
                .movement
                .mode,
            expected
        );
        tick(&mut host, 0);
        tick(&mut host, 20);
        assert_eq!(
            host.runtime.server.clients[id.0 as usize]
                .player
                .movement
                .mode,
            expected
        );
        host.runtime.server.clients[id.0 as usize]
            .player
            .movement
            .mode = MovementMode::Spectator;
        tick(&mut host, 40);
        assert_eq!(
            host.runtime.server.clients[id.0 as usize]
                .player
                .movement
                .mode,
            MovementMode::Spectator
        );
        assert_admitted(&host);
    }
}

#[test]
fn native_walk_fly_toss_and_bounce_integrate_after_one_due_think() {
    for (mode, phase, gravity, angular) in [
        (3.0, Motion::Walk, true, false),
        (5.0, Motion::Fly, false, false),
        (6.0, Motion::Toss, true, true),
        (10.0, Motion::Bounce, true, true),
    ] {
        let mut vm = mode_vm(mode, Vec3::default(), false);
        for (at, word) in [
            (111, 34),
            (112, 35),
            (113, 7.0f32.to_bits()),
            (114, 6),
            (115, 29),
            (116, 3),
        ] {
            vm.image.globals[at] = word;
        }
        vm.image.globals[180..183].copy_from_slice(&[10.0f32, 20.0, 30.0].map(f32::to_bits));
        append_begin(
            &mut vm,
            &[
                (Address, 34, 111, 90),
                (StorePF, 113, 90, 0),
                (Address, 34, 112, 90),
                (StorePF, 114, 90, 0),
            ],
        );
        replace_client_function(
            &mut vm,
            6,
            &[
                (LoadF, 34, 115, 91),
                (AddF, 91, 88, 92),
                (Address, 34, 115, 90),
                (StorePF, 92, 90, 0),
                (Address, 34, 116, 90),
                (StorePV, 180, 90, 0),
                (Return, 0, 0, 0),
            ],
        );
        let (mut host, id, geometry, entity, _) =
            qc_client_host(RuleSetId::Quake, RuleSetId::Quake, vm, &[]);
        host.connect_module_client(ModuleId(1), id, b"\\name\\native phases")
            .unwrap();
        tick(&mut host, 0);
        let calls = host.module_counts(ModuleId(1)).unwrap().calls;
        tick(&mut host, 20);
        let velocity = Vec3([10.0, 20.0, if gravity { 14.0 } else { 30.0 }]);
        let position = velocity * 0.02f32;
        let columns = &host.runtime.server.entities.columns;
        assert_eq!(columns.motion[entity.slot as usize], phase);
        assert_eq!(columns.position[entity.slot as usize], position);
        assert_eq!(columns.velocity[entity.slot as usize], velocity);
        assert_eq!(columns.appearance[entity.slot as usize].frame, 1);
        assert_eq!(
            columns.next_think[entity.slot as usize],
            Some(ThinkTime::Seconds(0.0))
        );
        assert_eq!(
            columns.angles[entity.slot as usize],
            if angular {
                Vec3([2.0, 4.0, 6.0]) * 0.02f32
            } else {
                Vec3::default()
            }
        );
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC(position.0.map(f32::to_bits)))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls + 4);
        assert!(host.runtime.server.area.bounds(entity).is_some());
        let world = host.runtime.collision.as_mut().unwrap();
        assert_eq!(
            host.runtime.server.move_pending_clients(
                &host.runtime.geometry,
                geometry,
                0,
                &mut world.scratch,
                |_, error| panic!("{error:?}")
            ),
            0
        );
        tick(&mut host, 40);
        assert_eq!(
            host.runtime.server.entities.columns.appearance[entity.slot as usize].frame,
            1
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls + 7);
        assert_admitted(&host);
    }
}

#[test]
fn native_queued_walk_controls_distinguish_ground_and_air_before_integration() {
    for (flags, speed) in [(512.0f32, 40.0f32), (0.0, 30.0)] {
        let mut vm = mode_vm(3.0, Vec3::default(), true);
        for (at, word) in [(110, 3), (111, 37), (112, 25), (113, flags.to_bits())] {
            vm.image.globals[at] = word;
        }
        vm.image.globals[156] = vm.stride as u32;
        append_begin(&mut vm, &[(Address, 34, 112, 90), (StorePF, 113, 90, 0)]);
        replace_client_function(
            &mut vm,
            7,
            &[
                (LoadV, 156, 110, 150),
                (Address, 156, 111, 90),
                (StorePV, 150, 90, 0),
                (Return, 0, 0, 0),
            ],
        );
        let (mut host, id, _, entity, _) =
            qc_client_host(RuleSetId::Quake, RuleSetId::Quake, vm, &[]);
        host.connect_module_client(ModuleId(1), id, b"\\name\\walk input")
            .unwrap();
        tick(&mut host, 0);
        let calls = host.module_counts(ModuleId(1)).unwrap().calls;
        queue(&mut host, id, [200.0, 0.0, 0.0]);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
        assert_eq!(
            host.runtime.server.entities.columns.position[entity.slot as usize],
            Vec3::default()
        );
        tick(&mut host, 20);
        assert_admitted(&host);
        let columns = &host.runtime.server.entities.columns;
        assert_eq!(
            columns.angular_velocity[entity.slot as usize],
            Some(Vec3([speed, 0.0, 0.0]))
        );
        let velocity = Vec3([speed, 0.0, -16.0]);
        assert_eq!(columns.velocity[entity.slot as usize], velocity);
        assert_eq!(columns.position[entity.slot as usize], velocity * 0.02f32);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC(velocity.0.map(f32::to_bits)))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls + 3);
        assert_admitted(&host);
    }
}

#[test]
fn native_waterjump_control_uses_live_direction_and_clears_expired_state_next_frame() {
    let mut vm = mode_vm(3.0, Vec3([0.0, 0.0, 40.0]), true);
    for (at, word) in [
        (110, 25),
        (111, 2048.0f32.to_bits()),
        (112, 53),
        (113, 2.0f32.to_bits()),
        (114, 55),
        (115, 1000.0f32.to_bits()),
        (116, 67),
    ] {
        vm.image.globals[at] = word;
    }
    vm.image.globals[180..183].copy_from_slice(&[17.0f32, -9.0, 999.0].map(f32::to_bits));
    append_begin(
        &mut vm,
        &[
            (Address, 34, 110, 90),
            (StorePF, 111, 90, 0),
            (Address, 34, 112, 90),
            (StorePF, 113, 90, 0),
            (Address, 34, 114, 90),
            (StorePF, 115, 90, 0),
            (Address, 34, 116, 90),
            (StorePV, 180, 90, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        5,
        &[
            (LoadF, 34, 110, 150),
            (LoadF, 34, 114, 151),
            (LoadF, 34, 112, 152),
            (Return, 150, 0, 0),
        ],
    );
    let (mut host, id, _, entity, _) = qc_client_host(RuleSetId::Quake, RuleSetId::Quake, vm, &[]);
    host.connect_module_client(ModuleId(1), id, b"\\name\\waterjump")
        .unwrap();
    tick(&mut host, 0);
    queue(&mut host, id, [200.0, 0.0, 200.0]);
    tick(&mut host, 20);
    let first = Vec3([17.0, -9.0, 40.0]) * 0.02f32;
    assert_eq!(
        host.runtime.server.entities.columns.position[entity.slot as usize],
        first
    );
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC(
            [2048.0f32, 1000.0, 0.0].map(f32::to_bits)
        ))
    );
    tick(&mut host, 40);
    let columns = &host.runtime.server.entities.columns;
    assert_eq!(
        columns.velocity[entity.slot as usize],
        Vec3([17.0, -9.0, 24.0])
    );
    assert_eq!(
        columns.position[entity.slot as usize],
        first + Vec3([17.0, -9.0, 24.0]) * 0.02f32
    );
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([0.0f32; 3].map(f32::to_bits)))
    );
    assert_admitted(&host);
}

#[test]
fn foreign_walk_uses_one_ordinary_q3_pmove_with_independent_q2_trace() {
    use qa_world::collision::WorldTrace;

    let velocity = Vec3([3.0, 4.0, 5.0]);
    let mut vm = mode_vm(3.0, velocity, true);
    for (at, word) in [
        (111, 25),
        (112, 512.0f32.to_bits()),
        (113, 53),
        (114, 2.0f32.to_bits()),
        (115, 54),
        (116, (-3.0f32).to_bits()),
    ] {
        vm.image.globals[at] = word;
    }
    append_begin(
        &mut vm,
        &[
            (Address, 34, 111, 90),
            (StorePF, 112, 90, 0),
            (Address, 34, 113, 90),
            (StorePF, 114, 90, 0),
            (Address, 34, 115, 90),
            (StorePF, 116, 90, 0),
        ],
    );
    let (mut host, id, geometry, entity, _) =
        qc_client_host(RuleSetId::Quake, RuleSetId::Quake3, vm, &[]);
    host.connect_module_client(ModuleId(1), id, b"\\name\\foreign walk")
        .unwrap();
    tick(&mut host, 0);
    host.console
        .append(
            "host_framerate 0.15\n",
            Context {
                source: RuleSetId::Quake,
                ..Context::default()
            },
        )
        .unwrap();
    host.execute_commands();
    host.runtime.server.clients[id.0 as usize]
        .player
        .trace_rules = RuleSetId::Quake2;
    let player = &host.runtime.server.clients[id.0 as usize].player;
    let mut expected = PlayerState {
        movement_rules: player.movement_rules,
        trace_rules: player.trace_rules,
        movement: player.movement,
        body: player.body,
        view_angles: player.view_angles,
        view_offset: player.view_offset,
        health: player.health,
        ..Default::default()
    };
    expected.movement.mode = MovementMode::Walk;
    expected.movement.tuning = qa_console::movement::MovementHandles::bind(&host.console.cvars)
        .unwrap()
        .resolve(
            &host.console.cvars,
            RuleSetId::Quake3,
            player.movement.tuning,
        );
    let command = UserCmd {
        movement: [127.0, -19.0, 23.0],
        units: qa_gameplay::rules::command_units(RuleSetId::Quake),
        ..Default::default()
    };
    let mut scratch = host.runtime.geometry.scratch();
    let mut trace = WorldTrace::new(
        &host.runtime.geometry,
        geometry,
        0,
        &host.runtime.server.entities,
        &host.runtime.server.area,
        &mut scratch,
        CollisionOwner::Lifetime(entity),
    );
    let result = qa_movement::pmove(
        UserCmd {
            server_time_ms: 150,
            duration_ms: 150,
            ..command
        },
        &mut expected,
        &mut trace,
        Default::default(),
        qa_movement::no_boundary,
    )
    .unwrap();
    assert_eq!(result.steps, 3);
    assert_ne!(expected.body.position, velocity * 0.15f32);
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    queue(&mut host, id, command.movement);
    tick(&mut host, 20);
    assert_admitted(&host);
    let actual = &host.runtime.server.clients[id.0 as usize].player;
    assert_eq!(
        actual.body.position.0.map(f32::to_bits),
        expected.body.position.0.map(f32::to_bits)
    );
    assert_eq!(
        actual.body.velocity.0.map(f32::to_bits),
        expected.body.velocity.0.map(f32::to_bits)
    );
    assert_eq!(actual.movement.command_time_ms, 150);
    assert_eq!(actual.movement.grounded, expected.movement.grounded);
    assert_eq!(actual.movement.ground, expected.movement.ground);
    assert_eq!(actual.movement.water_level, expected.movement.water_level);
    assert_eq!(
        actual.movement.water_contents,
        expected.movement.water_contents
    );
    assert_eq!(actual.view_angles, expected.view_angles);
    assert_eq!(
        (actual.movement_rules, actual.trace_rules),
        (RuleSetId::Quake3, RuleSetId::Quake2)
    );
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC(
            expected.body.position.0.map(f32::to_bits)
        ))
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls + 3);
    let world = host.runtime.collision.as_mut().unwrap();
    assert_eq!(
        host.runtime.server.move_pending_clients(
            &host.runtime.geometry,
            geometry,
            0,
            &mut world.scratch,
            |_, error| panic!("{error:?}")
        ),
        0
    );
    assert_admitted(&host);
}

#[test]
fn foreign_noclip_integrates_captured_phase_after_think_changes_native_mode_to_none() {
    let mut vm = mode_vm(8.0, Vec3([200.0, 0.0, 0.0]), false);
    for (at, word) in [
        (110, 36),
        (111, 0),
        (112, 29),
        (113, 34),
        (114, 35),
        (115, 7.0f32.to_bits()),
        (116, 6),
        (117, 0.0f32.to_bits()),
        (118, 1.0f32.to_bits()),
    ] {
        vm.image.globals[at] = word;
    }
    append_begin(
        &mut vm,
        &[
            (Address, 34, 113, 90),
            (StorePF, 115, 90, 0),
            (Address, 34, 114, 90),
            (StorePF, 116, 90, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        6,
        &[
            (Address, 34, 110, 90),
            (StorePF, 117, 90, 0),
            (Address, 34, 112, 90),
            (StorePF, 118, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        5,
        &[
            (LoadF, 34, 110, 150),
            (LoadF, 34, 111, 151),
            (LoadF, 34, 112, 152),
            (Return, 150, 0, 0),
        ],
    );
    let (mut host, id, _, entity, _) = qc_client_host(RuleSetId::Quake, RuleSetId::Quake3, vm, &[]);
    host.connect_module_client(ModuleId(1), id, b"\\name\\captured foreign")
        .unwrap();
    tick(&mut host, 0);
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    tick(&mut host, 20);
    let position = host.runtime.server.entities.columns.position[entity.slot as usize];
    assert!(position.0[0] > 0.0);
    assert_eq!([position.0[1], position.0[2]], [0.0; 2]);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls + 4);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC(
            [0.0, position.0[0], 1.0].map(f32::to_bits)
        ))
    );
    tick(&mut host, 40);
    assert_eq!(
        host.runtime.server.entities.columns.position[entity.slot as usize],
        position
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls + 7);
    assert_admitted(&host);
}
