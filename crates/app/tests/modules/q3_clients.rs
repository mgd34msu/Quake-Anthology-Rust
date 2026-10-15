//! Synthetic public-QVM lifecycle coverage, not stock compiled Pmove acceptance.
use super::*;
use qa_app::{WorldCollision, client_policy::ClientPolicy, map::SpawnAnchor};
use qa_formats::program::qvm::Opcode as Op;
use qa_network::commands::packet::Protocol;
use qa_session::clients::Connection;
use qa_world::collision::{
    brushes::{BrushTree, CollisionLeaf, ModelRoot},
    surfaces::SurfaceTable,
};

const MODULE: ModuleId = ModuleId(1);
const COUNTERS: i32 = 128;
const CONNECT_CLIENT: i32 = 176;
const FIRST_TIME: i32 = 180;
const IS_BOT: i32 = 184;
const THINK_CLIENT: i32 = 188;
const COMMAND_ARGC: i32 = 192;
const COMMAND_ARGV: i32 = 256;
const USERINFO: i32 = 384;
const COMMAND: i32 = 512;
const ENTITIES: i32 = 4096;
// Public sharedEntity_t is 516 bytes, distinct from the 272-byte vmCvar_t.
const ENTITY_STRIDE: i32 = 608;
const PLAYERS: i32 = 16384;
const PLAYER_STRIDE: i32 = 512;
const PLAYER_ORIGIN: i32 = PLAYERS + 20;

type Code = Vec<(Op, i32)>;
fn word(code: &mut Code, address: i32, value: i32) {
    code.extend([(Op::Const, address), (Op::Const, value), (Op::Store4, 0)]);
}
fn parameter(code: &mut Code, address: i32, index: i32) {
    code.extend([
        (Op::Const, address),
        (Op::Local, 76 + index * 4),
        (Op::Load4, 0),
        (Op::Store4, 0),
    ]);
}
fn increment(code: &mut Code, address: i32) {
    code.extend([
        (Op::Const, address),
        (Op::Const, address),
        (Op::Load4, 0),
        (Op::Const, 1),
        (Op::Add, 0),
        (Op::Store4, 0),
    ]);
}
fn import(code: &mut Code, number: i32, arguments: &[i32], client: bool) {
    for (index, &value) in arguments.iter().enumerate() {
        if client && index == 0 {
            code.extend([(Op::Local, 76), (Op::Load4, 0)]);
        } else {
            code.push((Op::Const, value));
        }
        code.push((Op::Arg, 8 + index as i32 * 4));
    }
    code.extend([(Op::Const, -number - 1), (Op::Call, 0)]);
}
fn finish(code: &mut Code, value: i32) {
    code.extend([(Op::Const, value), (Op::Leave, 64)]);
}

fn fixture_program(denial: i32, fail_begin: bool) -> qa_compat::qvm::Vm {
    let mut code = vec![(Op::Enter, 64)];
    let mut branches = Vec::new();
    for export in 0..=9 {
        code.extend([
            (Op::Local, 72),
            (Op::Load4, 0),
            (Op::Const, export),
            (Op::Eq, 0),
        ]);
        branches.push(code.len() - 1);
    }
    finish(&mut code, 0);
    for export in 0..=9 {
        code[branches[export as usize]].1 = code.len() as i32;
        if export != 9 {
            increment(&mut code, COUNTERS + export * 4);
        }
        match export {
            0 => {
                import(
                    &mut code,
                    15,
                    &[ENTITIES, 2, ENTITY_STRIDE, PLAYERS, PLAYER_STRIDE],
                    false,
                );
                code.push((Op::Pop, 0));
            }
            2 => {
                parameter(&mut code, CONNECT_CLIENT, 0);
                parameter(&mut code, FIRST_TIME, 1);
                parameter(&mut code, IS_BOT, 2);
                import(&mut code, 20, &[0, USERINFO, 64], true);
                code.push((Op::Pop, 0));
                finish(&mut code, denial);
                continue;
            }
            3 => {
                if fail_begin {
                    code.extend([(Op::Const, 1), (Op::Const, 0), (Op::DivI, 0), (Op::Pop, 0)]);
                }
                // A module-owned noclip pose makes an accidental host move observable.
                word(&mut code, PLAYERS + 4, 1); // PM_NOCLIP
                word(&mut code, PLAYER_ORIGIN, 10.0f32.to_bits() as i32);
                word(&mut code, ENTITIES + 488, 10.0f32.to_bits() as i32);
                word(&mut code, PLAYERS + 32, 64.0f32.to_bits() as i32);
                word(&mut code, PLAYERS + 48, 800);
                word(&mut code, PLAYERS + 52, 320);
                word(&mut code, PLAYERS + 56, 65536); // native signed32 delta angle
                word(&mut code, PLAYERS + 152, 45.0f32.to_bits() as i32);
                word(&mut code, PLAYERS + 184, 125); // STAT_HEALTH
            }
            4 => {
                import(&mut code, 20, &[0, USERINFO, 64], true);
                code.push((Op::Pop, 0));
            }
            6 => {
                code.push((Op::Const, COMMAND_ARGC));
                import(&mut code, 8, &[], false);
                code.push((Op::Store4, 0));
                import(&mut code, 9, &[1, COMMAND_ARGV, 64], false);
                code.push((Op::Pop, 0));
            }
            7 => {
                parameter(&mut code, THINK_CLIENT, 0);
                import(&mut code, 36, &[0, COMMAND], true);
                code.push((Op::Pop, 0));
                code.extend([
                    (Op::Const, PLAYERS),
                    (Op::Const, COMMAND),
                    (Op::Load4, 0),
                    (Op::Store4, 0),
                    (Op::Const, PLAYER_ORIGIN),
                    (Op::Const, PLAYER_ORIGIN),
                    (Op::Load4, 0),
                    (Op::Const, 1.0f32.to_bits() as i32),
                    (Op::AddF, 0),
                    (Op::Store4, 0),
                    // Collision pose is module-owned independently of playerState.
                    (Op::Const, ENTITIES + 488),
                    (Op::Const, PLAYER_ORIGIN),
                    (Op::Load4, 0),
                    (Op::Store4, 0),
                ]);
            }
            9 => {
                code.extend([
                    (Op::Local, 76),
                    (Op::Load4, 0),
                    (Op::Load4, 0),
                    (Op::Leave, 64),
                ]);
                continue;
            }
            _ => {}
        }
        finish(&mut code, 0);
    }
    let mut data = vec![0; PLAYERS as usize + 2 * PLAYER_STRIDE as usize];
    data[..14].copy_from_slice(b"masked denial\0");
    for index in 0..2 {
        let at = ENTITIES as usize + index * ENTITY_STRIDE as usize;
        data[at..at + 4].copy_from_slice(&(index as i32).to_le_bytes());
        data[at + 512..at + 516].copy_from_slice(&1023i32.to_le_bytes());
        for (offset, values) in [(436, [-16.0f32, -16.0, -24.0]), (448, [16.0, 16.0, 32.0])] {
            for (axis, value) in values.into_iter().enumerate() {
                data[at + offset + axis * 4..at + offset + axis * 4 + 4]
                    .copy_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    let mut vm = service_program::program(&code, &data);
    // load_q3 performs this same cold registration for a real server QVM.
    vm.configure_q3_game(qa_core::primitives::Q3StatLayout::BASE);
    vm
}

fn fixture(
    denial: i32,
    phase: Phase,
    missing_export: Option<u32>,
) -> (Box<FrameHost>, ClientId, GeometryId) {
    fixture_with(denial, phase, missing_export, false, false)
}
fn fixture_with(
    denial: i32,
    phase: Phase,
    missing_export: Option<u32>,
    fail_begin: bool,
    bot: bool,
) -> (Box<FrameHost>, ClientId, GeometryId) {
    fixture_extra(denial, phase, missing_export, fail_begin, bot, |_, _| {
        Vec::new()
    })
}
fn fixture_extra(
    denial: i32,
    phase: Phase,
    missing_export: Option<u32>,
    fail_begin: bool,
    bot: bool,
    extra: impl FnOnce(&mut Runtime, ClientId) -> Vec<ModuleRequest>,
) -> (Box<FrameHost>, ClientId, GeometryId) {
    let mut runtime = Runtime::load(3, []).unwrap();
    // The actual native namespace must not be inferred from this occupied common slot.
    assert_eq!(
        runtime.server.connect(Connection::Bot, ModuleId(2), None),
        Some(ClientId(0))
    );
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            SurfaceTable::flags(vec![]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![CollisionLeaf {
                    stored_contents: None,
                    brushes: IndexSpan { first: 0, count: 0 },
                }],
                leaf_brushes: vec![],
                models: vec![ModelRoot::Leaf(0)],
            },
            vec![Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    let client = if bot {
        let client = runtime
            .server
            .connect(
                Connection::Bot,
                MODULE,
                Some(NativeEntity {
                    module: MODULE,
                    slot: 0,
                }),
            )
            .unwrap();
        let row = &mut runtime.server.clients[client.0 as usize];
        row.player.movement_rules = RuleSetId::Quake;
        row.player.trace_rules = RuleSetId::Quake2;
        qa_movement::set_bounds(&mut row.player);
        runtime
            .server
            .entities
            .columns
            .set_body(row.entity.slot as usize, row.player.body);
        client
    } else {
        runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: RuleSetId::Quake3,
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake,
                    trace: RuleSetId::Quake2,
                },
                Protocol::Quake3_68,
                0,
            )
            .unwrap()
    };
    assert_eq!(client, ClientId(1));
    let context = Context {
        source: RuleSetId::Quake3,
        side: qa_console::catalog::Scope::Server,
        ..Default::default()
    };
    let mut console = Console::new(context).unwrap();
    console.append("sv_maxclients 2\n", context).unwrap();
    console.execute_frame(&mut runtime);
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), MODULE, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    let request = ModuleRequest {
        context: CallContext {
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::Quake2,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
            server_frame: 0,
            module: MODULE,
            clock: ThinkTime::Milliseconds(0),
            console: context,
            allocation: AllocationPolicy::q3(0),
            link_order: LinkOrder::Head,
        },
        timing_rules: RuleSetId::Quake3,
        phase,
        rate: TickRate::fixed(50).unwrap(),
        anchor,
        program: Program::Qvm {
            vm: Box::new(fixture_program(denial, fail_begin)),
            imports: &Q3_SERVER,
        },
        entries: (0..=9)
            .filter(|entry| Some(*entry) != missing_export)
            .collect(),
        frame: Export::clocked(CallbackId(8)),
        prepare: vec![],
        initialize: vec![Export::words(CallbackId(0), &[])],
        api: None,
        shutdown: vec![Export::words(CallbackId(1), &[])],
        instruction_budget: 10_000,
        configstrings: 0,
        files: 0,
        entity_source: None,
    };
    let mut requests = vec![request];
    requests.extend(extra(&mut runtime, client));
    let mut host = Box::new(
        FrameHost::load_modules(console, runtime, TickRate::fixed(50).unwrap(), requests).unwrap(),
    );
    host.initialize_modules(phase);
    (host, client, geometry)
}
fn read(host: &mut FrameHost, address: i32) -> u32 {
    host.call_module_export(
        MODULE,
        Export::words(CallbackId(9), &[address as u64]),
        ThinkTime::Milliseconds(0),
    )
    .unwrap();
    let Some(ModuleResult::Qvm(value)) = host.module_counts(MODULE).unwrap().last_result else {
        panic!("getter did not return a QVM value")
    };
    value as u32
}
fn count(host: &mut FrameHost, export: i32) -> u32 {
    read(host, COUNTERS + export * 4)
}
fn tick(host: &mut FrameHost, milliseconds: u64) {
    host.frame(
        &mut Source {
            time: EventTime::from_milliseconds(milliseconds),
            polls: 0,
        },
        true,
    );
}

#[test]
fn q3_queued_commands_use_native_zero_once_and_import_the_module_pose() {
    let (mut host, client, geometry) = fixture(0, Phase::Server, None);
    assert_eq!(host.client_info(MODULE).unwrap().clients, 2);
    assert_eq!(
        host.connect_module_client(MODULE, client, b"\\name\\Tester")
            .unwrap(),
        0
    );
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    assert_eq!(
        host.runtime.server.module_moves(client),
        Some(NativeEntity {
            module: MODULE,
            slot: 0
        })
    );
    assert_eq!(
        (
            read(&mut host, CONNECT_CLIENT),
            read(&mut host, FIRST_TIME),
            read(&mut host, IS_BOT)
        ),
        (0, 1, 0)
    );
    assert_eq!(read(&mut host, USERINFO + 6), u32::from_le_bytes(*b"Test"));
    assert_eq!((count(&mut host, 2), count(&mut host, 3)), (1, 1));
    let player = &host.runtime.server.clients[client.0 as usize].player;
    assert_eq!(player.body.position, Vec3([10.0, 0.0, 0.0]));
    assert_eq!(player.body.velocity, Vec3([64.0, 0.0, 0.0]));
    assert_eq!(player.health, 125);
    assert_eq!(player.view_angles, Vec3([45.0, 0.0, 0.0]));
    assert_eq!(player.movement.delta_angles.0[0].degrees(), 360.0);
    assert_eq!(player.movement.mode, MovementMode::Noclip);
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::Quake, RuleSetId::Quake2)
    );
    tick(&mut host, 0);
    for (time, forward, expected) in [(20, 50.0, 11.0), (40, 100.0, 12.0)] {
        assert!(host.runtime.send_local_command(
            client,
            &UserCmd {
                server_time_ms: time,
                duration_ms: 20,
                movement: [forward, 0.0, 0.0],
                units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
                buttons: buttons::ATTACK,
                ..Default::default()
            },
            EventTime::from_milliseconds(time as u64)
        ));
        host.drain(&mut Default::default());
        assert_eq!(read(&mut host, COMMAND), time as u32);
        assert_eq!(read(&mut host, COMMAND + 20) >> 8 & 255, forward as u32);
        assert_eq!(read(&mut host, THINK_CLIENT), 0);
        assert_eq!(
            host.runtime.server.clients[client.0 as usize]
                .player
                .body
                .position
                .0[0],
            expected
        );
    }
    assert_eq!(count(&mut host, 7), 2);
    let before = host.runtime.server.clients[client.0 as usize].player.body;
    tick(&mut host, 50);
    tick(&mut host, 100);
    assert_eq!(count(&mut host, 7), 2);
    assert!(count(&mut host, 8) >= 2);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .body
            .position,
        before.position
    );
    assert_eq!(
        host.runtime.server.entities.columns.position[entity.slot as usize],
        before.position
    );
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
    assert_eq!(host.module_counts(MODULE).unwrap().traps, 0);
}

#[test]
fn q3_client_text_userinfo_and_disconnect_use_the_same_lifecycle_and_lifetime() {
    let (mut host, client, _) = fixture(0, Phase::Server, None);
    host.connect_module_client(MODULE, client, b"\\name\\Tester")
        .unwrap();
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    assert_eq!(
        host.client_string_command(client, RuleSetId::Quake3, b"probe \"two words\"")
            .unwrap(),
        std::ops::ControlFlow::Continue(())
    );
    assert_eq!(count(&mut host, 6), 1);
    assert_eq!(read(&mut host, COMMAND_ARGC), 2);
    assert_eq!(read(&mut host, COMMAND_ARGV), u32::from_le_bytes(*b"two "));
    host.userinfo_module_client(client, b"\\name\\After")
        .unwrap();
    assert_eq!(count(&mut host, 4), 1);
    assert_eq!(read(&mut host, USERINFO + 6), u32::from_le_bytes(*b"Afte"));
    assert_eq!(
        host.runtime.server.userinfo(client),
        Some(b"\\name\\After".as_slice())
    );
    assert_eq!(
        host.client_string_command(client, RuleSetId::Quake3, b"disconnect")
            .unwrap(),
        std::ops::ControlFlow::Break(())
    );
    assert_eq!(count(&mut host, 5), 1);
    assert!(host.runtime.server.entities.resolve(entity).is_none());
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_none()
    );
    assert!(
        host.run_module_client_command(MODULE, client, &UserCmd::default(), 0)
            .is_err()
    );
    assert_eq!(count(&mut host, 7), 0);
}

#[test]
fn rejected_client_import_keeps_termination_and_dispatches_the_server_disconnect_once() {
    const CGAME: ModuleId = ModuleId(3);
    let (mut host, client, _) =
        fixture_extra(0, Phase::Server, None, false, false, |runtime, client| {
            let anchor = runtime
                .server
                .entities
                .allocate(ThinkTime::Milliseconds(0), CGAME, AllocationPolicy::q3(0))
                .unwrap()
                .id;
            let mut code = vec![
                (Op::Enter, 64),
                (Op::Local, 72),
                (Op::Load4, 0),
                (Op::Const, 1),
                (Op::Eq, 7),
                (Op::Const, 0),
                (Op::Leave, 64),
            ];
            // A future GetSnapshot marks this short ClientConnectionView as
            // rejected. The status must survive releasing its service borrow.
            import(&mut code, 52, &[1, 0], false);
            code.push((Op::Pop, 0));
            finish(&mut code, 0);
            vec![ModuleRequest {
                context: CallContext {
                    module: CGAME,
                    clock: ThinkTime::Milliseconds(0),
                    native_rules: RuleSetId::Quake3,
                    trace_rules: RuleSetId::Quake2,
                    sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
                    server_frame: 0,
                    console: Context::default(),
                    allocation: AllocationPolicy::q3(0),
                    link_order: LinkOrder::Head,
                },
                timing_rules: RuleSetId::Quake3,
                phase: Phase::Client,
                rate: TickRate::FrameDriven,
                anchor,
                program: Program::Qvm {
                    vm: Box::new(service_program::program(&code, &[])),
                    imports: &qa_compat::abi::Q3_CLIENT,
                },
                entries: vec![0, 1],
                frame: Export {
                    client: Some(client),
                    ..Export::words(CallbackId(0), &[])
                },
                prepare: vec![],
                initialize: vec![],
                api: None,
                shutdown: vec![],
                instruction_budget: 1000,
                configstrings: 0,
                files: 0,
                entity_source: None,
            }]
        });
    host.connect_module_client(MODULE, client, b"\\name\\RejectedView")
        .unwrap();
    let lifetime = host.runtime.server.clients[client.0 as usize].entity;
    host.client_modules();
    assert_eq!(count(&mut host, 5), 0);
    assert!(
        host.call_module_export(
            CGAME,
            Export::words(CallbackId(1), &[]),
            ThinkTime::Milliseconds(20),
        )
        .is_err()
    );
    assert_eq!(count(&mut host, 5), 1);
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_none()
    );
    assert!(host.runtime.server.entities.resolve(lifetime).is_none());
    assert_eq!(
        host.runtime.server.clients[0].connection,
        Some(Connection::Bot)
    );
    assert_eq!(host.module_counts(MODULE).unwrap().traps, 0);
    host.client_modules();
    assert_eq!(count(&mut host, 5), 1);
}

#[test]
fn q3_connect_distinguishes_null_from_nonzero_masked_zero_and_never_begins_a_denial() {
    for denial in [64, 65536] {
        let (mut host, client, _) = fixture(denial, Phase::Server, None);
        let error = host
            .connect_module_client(MODULE, client, b"\\name\\denied")
            .unwrap_err();
        assert!(error.starts_with("ClientConnect rejected:"), "{error}");
        if denial == 65536 {
            assert!(error.ends_with("masked denial"), "{error}");
        }
        assert_eq!(count(&mut host, 2), 1);
        assert_eq!(count(&mut host, 3), 0);
        assert!(
            host.runtime.server.clients[client.0 as usize]
                .connection
                .is_none()
        );
        assert_eq!(
            host.runtime.server.clients[0].connection,
            Some(Connection::Bot)
        );
    }
}

#[test]
fn q3_client_registration_requires_server_exports_and_freezes_its_native_capacity() {
    for (phase, missing) in [(Phase::Client, None), (Phase::Server, Some(7))] {
        let (mut host, client, _) = fixture(0, phase, missing);
        assert!(host.client_info(MODULE).is_none());
        assert!(
            host.connect_module_client(MODULE, client, b"\\name\\not a server")
                .is_err()
        );
    }
    let (mut host, client, _) = fixture(0, Phase::Server, None);
    let context = Context {
        source: RuleSetId::Quake3,
        side: qa_console::catalog::Scope::Server,
        ..Default::default()
    };
    host.console.append("sv_maxclients 3\n", context).unwrap();
    host.execute_commands();
    assert_eq!(host.client_info(MODULE).unwrap().clients, 2);
    host.connect_module_client(MODULE, client, b"\\name\\healthy")
        .unwrap();
    let extra = host
        .runtime
        .connect_local(
            SeatId::ALL[1],
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake,
                trace: RuleSetId::Quake2,
            },
            Protocol::Quake3_68,
            2,
        )
        .unwrap();
    assert!(
        host.connect_module_client(MODULE, extra, b"\\name\\outside")
            .unwrap_err()
            .contains("outside module maxclients")
    );
    assert_eq!((count(&mut host, 2), count(&mut host, 3)), (1, 1));
    assert!(host.runtime.server.module_moves(client).is_some());
}

#[test]
fn q3_failed_begin_disconnects_once_and_invalidates_only_that_common_lifetime() {
    let (mut host, client, _) = fixture_with(0, Phase::Server, None, true, false);
    let before = host.runtime.server.clients[client.0 as usize].entity;
    let error = host
        .connect_module_client(MODULE, client, b"\\name\\failed begin")
        .unwrap_err();
    assert!(error.starts_with("ClientBegin:"), "{error}");
    assert_eq!(
        (
            count(&mut host, 2),
            count(&mut host, 3),
            count(&mut host, 5)
        ),
        (1, 1, 1)
    );
    assert_eq!(host.module_counts(MODULE).unwrap().traps, 1);
    assert!(host.runtime.server.entities.resolve(before).is_none());
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_none()
    );
    let reset = host.runtime.server.clients[client.0 as usize].entity;
    assert_ne!(reset, before);
    assert!(host.runtime.server.entities.resolve(reset).is_some());
    assert_eq!(
        host.runtime.server.clients[0].connection,
        Some(Connection::Bot)
    );
    assert_eq!(
        host.client_string_command(client, RuleSetId::Quake3, b"disconnect")
            .unwrap(),
        std::ops::ControlFlow::Break(())
    );
    assert_eq!(count(&mut host, 5), 1);
    assert_eq!(host.runtime.server.clients[client.0 as usize].entity, reset);
}

#[test]
fn q3_bot_uses_its_explicit_native_namespace_and_the_common_command_builder_once() {
    let (mut host, client, _) = fixture_with(0, Phase::Server, None, false, true);
    assert_eq!(
        host.connect_module_client(MODULE, client, b"\\name\\bot")
            .unwrap(),
        0
    );
    assert!(host.runtime.snapshots[client.0 as usize].is_none());
    assert_eq!(
        (
            read(&mut host, CONNECT_CLIENT),
            read(&mut host, FIRST_TIME),
            read(&mut host, IS_BOT)
        ),
        (0, 1, 1)
    );
    assert_eq!(
        host.runtime.server.module_moves(client),
        Some(NativeEntity {
            module: MODULE,
            slot: 0
        })
    );
    host.runtime.server.clients[client.0 as usize]
        .intent
        .movement[0][0] = 1.0;
    tick(&mut host, 0);
    let initial_count = count(&mut host, 7);
    let initial_origin = host.runtime.server.clients[client.0 as usize]
        .player
        .body
        .position
        .0[0];
    tick(&mut host, 50);
    assert_eq!(count(&mut host, 7), initial_count + 1);
    assert_eq!(read(&mut host, THINK_CLIENT), 0);
    assert_eq!(read(&mut host, COMMAND), 50);
    assert_eq!(read(&mut host, COMMAND + 20) >> 8 & 255, 127);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .body
            .position
            .0[0],
        initial_origin + 1.0
    );
    tick(&mut host, 70);
    assert_eq!(count(&mut host, 7), initial_count + 1);
    tick(&mut host, 100);
    assert_eq!(count(&mut host, 7), initial_count + 2);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .body
            .position
            .0[0],
        initial_origin + 2.0
    );
    assert_eq!(host.module_counts(MODULE).unwrap().traps, 0);
}
