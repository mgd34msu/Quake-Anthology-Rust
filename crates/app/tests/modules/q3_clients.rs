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
const GAME_USERINFO: i32 = 768;
const CHANGED_USERINFO: &[u8] = b"\\name\\GameName\\skin\\game/blue\0";
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

fn fixture_program(denial: i32, fail_begin: bool, userinfo_mode: i32) -> qa_compat::qvm::Vm {
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
                if userinfo_mode != 0 {
                    import(&mut code, 21, &[0, GAME_USERINFO], true);
                    code.push((Op::Pop, 0));
                }
                if userinfo_mode < 0 {
                    code.extend([(Op::Const, 1), (Op::Const, 0), (Op::DivI, 0), (Op::Pop, 0)]);
                }
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
    data[GAME_USERINFO as usize..GAME_USERINFO as usize + CHANGED_USERINFO.len()]
        .copy_from_slice(CHANGED_USERINFO);
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
    fixture_extra(
        denial,
        phase,
        missing_export,
        fail_begin,
        bot,
        Protocol::Quake3_68,
        0,
        |_, _| Vec::new(),
    )
}
fn fixture_extra(
    denial: i32,
    phase: Phase,
    missing_export: Option<u32>,
    fail_begin: bool,
    bot: bool,
    protocol: Protocol,
    userinfo_mode: i32,
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
            Vec::new(),
            SurfaceTable::flags(vec![]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![CollisionLeaf {
                    stored_contents: None,
                    brushes: IndexSpan { first: 0, count: 0 },
                    patches: Default::default(),
                }],
                leaf_brushes: vec![],
                models: vec![ModelRoot::Leaf(0)],
                leaf_patches: Vec::new(),
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
                protocol,
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
    console
        .append("sv_maxclients 2\nsv_pure 0\n", context)
        .unwrap();
    console.execute_frame(&mut runtime);
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), MODULE, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    let request = ModuleRequest {
        search: None,
        q3_image: None,
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
            vm: Box::new(fixture_program(denial, fail_begin, userinfo_mode)),
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

fn enter_world(host: &mut FrameHost, client: ClientId) {
    tick(host, 0);
    assert!(host.runtime.send_client_command(
        client,
        &UserCmd::default(),
        EventTime(0),
        &mut Source {
            time: EventTime(0),
            polls: 0
        },
    ));
    host.drain(&mut Default::default());
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
    assert_eq!((count(&mut host, 2), count(&mut host, 3)), (1, 0));
    enter_world(&mut host, client);
    assert_eq!((count(&mut host, 3), count(&mut host, 7)), (1, 0));
    // The zero-time command enters before old-usercmd rejection, and is not
    // itself a ClientThink. Repeated zero-time commands cannot begin twice.
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .command
            .server_time_ms,
        0
    );
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
        assert!(host.runtime.send_client_command(
            client,
            &UserCmd {
                server_time_ms: time,
                duration_ms: 20,
                movement: [forward, 0.0, 0.0],
                units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
                buttons: buttons::ATTACK,
                ..Default::default()
            },
            EventTime::from_milliseconds(time as u64),
            &mut Source {
                time: EventTime(0),
                polls: 0
            }
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
            qa_core::primitives::PatchClip::All,
            None,
            None,
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
    assert!(host.userinfo_module_client(client, b"\\name").is_err());
    assert_eq!(count(&mut host, 4), 1);
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

fn send_qw_command(host: &mut FrameHost, client: ClientId, text: &[u8]) -> Vec<u8> {
    use qa_core::loopback::Endpoint;
    let runtime = &mut host.runtime;
    let channel = &mut runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap()
        .channel;
    channel
        .queue_client_command(Protocol::QuakeWorld28, text)
        .unwrap();
    channel.prepare(None, EventTime(0)).unwrap().unwrap();
    let bytes = channel.pending_packet().unwrap().bytes.to_vec();
    assert!(
        channel
            .submit_with(EventTime(0), |packet| runtime
                .loopback
                .send(Endpoint::Client, client, packet)
                .is_ok())
            .unwrap()
            .is_some()
    );
    host.drain(&mut Default::default());
    bytes
}

#[test]
fn qw_wire_notifies_the_loaded_q3_module_once_and_retains_both_callback_edits() {
    use qa_core::loopback::Endpoint;
    let (mut host, client, _) = fixture_extra(
        0,
        Phase::Server,
        None,
        false,
        false,
        Protocol::QuakeWorld28,
        1,
        |_, _| Vec::new(),
    );
    host.connect_module_client(MODULE, client, b"\\name\\Tester\\skin\\original")
        .unwrap();
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    assert_eq!(host.client_info(MODULE).unwrap().rules, RuleSetId::Quake3);
    assert_eq!(
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .protocol,
        Protocol::QuakeWorld28
    );
    let packet = send_qw_command(&mut host, client, b"setinfo skin wire/red");
    assert_eq!(count(&mut host, 4), 1);
    assert_eq!(count(&mut host, 6), 0);
    assert_eq!(read(&mut host, USERINFO + 6), u32::from_le_bytes(*b"Test"));
    assert_eq!(
        host.runtime.server.userinfo(client).unwrap(),
        &CHANGED_USERINFO[..CHANGED_USERINFO.len() - 1]
    );
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].entity,
        entity
    );
    host.runtime
        .loopback
        .send(Endpoint::Client, client, &packet)
        .unwrap();
    host.drain(&mut Default::default());
    assert_eq!(
        count(&mut host, 4),
        1,
        "stale reliable replay must not invoke the module again"
    );
    tick(&mut host, 0);
    // QW emits a reply only after another client packet opens that request.
    send_qw_command(&mut host, client, b"");
    tick(&mut host, 20);
    let peer = host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .peer(0)
        .unwrap();
    for (key, value) in [
        (b"name".as_slice(), b"GameName".as_slice()),
        (b"skin", b"game/blue"),
    ] {
        let range = qa_core::text::info_value(peer.text, key, false)
            .unwrap_or_else(|| panic!("missing {key:?} from {:?}", peer.text));
        assert_eq!(&peer.text[range], value);
    }
    assert_eq!(count(&mut host, 4), 1);
    assert!(!host.runtime.quit);
}

#[test]
fn mixed_wire_userinfo_callback_error_keeps_accepted_effects_and_other_clients() {
    let (mut host, client, _) = fixture_extra(
        0,
        Phase::Server,
        None,
        false,
        false,
        Protocol::QuakeWorld28,
        -1,
        |_, _| Vec::new(),
    );
    host.connect_module_client(MODULE, client, b"\\name\\Tester\\skin\\original")
        .unwrap();
    let other = host.runtime.server.clients[0].entity;
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    send_qw_command(&mut host, client, b"setinfo skin wire/red");
    assert_eq!(host.module_counts(MODULE).unwrap().traps, 1);
    assert_eq!(count(&mut host, 4), 1);
    assert_eq!(
        host.runtime.server.userinfo(client).unwrap(),
        &CHANGED_USERINFO[..CHANGED_USERINFO.len() - 1]
    );
    assert_eq!(host.runtime.server.clients[0].entity, other);
    assert!(host.runtime.server.clients[0].is_active());
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].entity,
        entity
    );
    assert!(host.runtime.server.clients[client.0 as usize].is_active());
    assert!(!host.runtime.quit);
}

#[test]
fn rejected_client_import_keeps_termination_and_dispatches_the_server_disconnect_once() {
    const CGAME: ModuleId = ModuleId(3);
    let (mut host, client, _) = fixture_extra(
        0,
        Phase::Server,
        None,
        false,
        false,
        Protocol::Quake3_68,
        0,
        |runtime, client| {
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
                search: None,
                q3_image: None,
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
        },
    );
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
    enter_world(&mut host, client);
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
    host.connect_module_client(MODULE, client, b"\\name\\failed begin")
        .unwrap();
    assert_eq!(count(&mut host, 3), 0);
    enter_world(&mut host, client);
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

#[test]
fn accepted_userinfo_survives_text_pressure_without_repeating_the_module_callback() {
    let (mut host, client, _) = fixture_extra(
        0,
        Phase::Server,
        None,
        false,
        false,
        Protocol::QuakeWorld28,
        1,
        |_, _| Vec::new(),
    );
    host.connect_module_client(MODULE, client, b"\\name\\Tester\\skin\\original")
        .unwrap();
    let mut held = Vec::new();
    while let Some(lease) = host
        .runtime
        .server
        .events
        .texts
        .insert(b"independent lease")
    {
        held.push(lease);
    }
    let rejected = host.runtime.server.events.rejected_publications();
    send_qw_command(&mut host, client, b"setinfo skin wire/red");
    assert_eq!(count(&mut host, 4), 1);
    tick(&mut host, 0);
    assert!(host.runtime.server.events.rejected_publications() > rejected);
    assert!(host.runtime.server.userinfo_record(client).unwrap().dirty());
    assert_eq!(
        host.runtime.server.userinfo(client).unwrap(),
        &CHANGED_USERINFO[..CHANGED_USERINFO.len() - 1]
    );
    for lease in held {
        host.runtime.server.events.texts.release(lease);
    }
    for tick_number in 1..=8 {
        send_qw_command(&mut host, client, b"");
        tick(&mut host, tick_number * 20);
    }
    let peer = host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .peer(0)
        .unwrap();
    for (key, value) in [
        (b"name".as_slice(), b"GameName".as_slice()),
        (b"skin", b"game/blue"),
    ] {
        let span = qa_core::text::info_value(peer.text, key, false).unwrap();
        assert_eq!(&peer.text[span], value);
    }
    assert_eq!(count(&mut host, 4), 1);
    assert!(!host.runtime.server.userinfo_record(client).unwrap().dirty());
    assert!(host.runtime.server.clients[0].is_active());
    assert!(!host.runtime.quit);
}

#[test]
fn q3_gamestate_resend_and_pure_commands_share_native_ack_and_client_lifetimes() {
    use qa_core::events::{OutputSubmission, OutputTarget};
    use qa_core::loopback::Endpoint;
    use qa_network::{
        channel::{Channel, Delivery, commands::ServerOutput},
        commands::connection::Commands,
        ingress::Route,
    };
    struct Wire {
        time: EventTime,
        packets: Vec<(std::net::SocketAddr, Vec<u8>)>,
    }
    impl FrameSource for Wire {
        fn begin_frame(&mut self) -> Duration {
            Duration::from_nanos(self.time.0)
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            queue
                .push(SysEvent {
                    time: self.time,
                    kind: EventKind::Time,
                })
                .unwrap();
        }
        fn wait_time(&mut self, _: Duration) -> Duration {
            Duration::from_nanos(self.time.0)
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
        fn send_packet(&mut self, _: u16, peer: std::net::SocketAddr, bytes: &[u8]) -> bool {
            self.packets.push((peer, bytes.to_vec()));
            true
        }
    }
    fn states(
        host: &mut FrameHost,
        observer: qa_core::events::OutputConsumerId,
    ) -> Vec<(ClientId, u64)> {
        let mut found = Vec::new();
        qa_core::events::dispatch_consumer(
            &mut host.runtime.server.events,
            observer,
            &mut |record: qa_core::events::OutputRecord, _: &mut qa_core::events::TextStore| {
                if matches!(record.event, FrameEvent::GameState { .. })
                    && let Some(NativeDelivery {
                        target: NativeTarget::Client(client),
                        ..
                    }) = record.delivery
                {
                    found.push((client, record.sequence));
                }
                OutputSubmission::BestEffort
            },
        );
        found
    }
    fn receive(
        wire: &mut Wire,
        client: &mut Channel,
        peer: std::net::SocketAddr,
    ) -> Vec<Vec<(u32, Vec<u8>)>> {
        let mut states = Vec::new();
        for (to, packet) in std::mem::take(&mut wire.packets) {
            if to != peer {
                continue;
            } // The second peer deliberately withholds its ACK.
            let received = client.receive(&packet, wire.time).unwrap();
            let sequence = received.header.sequence;
            if let Delivery::Payload(body) = received.delivery {
                let mut body = body.to_vec();
                client
                    .decode_server_output(&mut body, sequence, |output| {
                        if let ServerOutput::Gamestate(state) = output {
                            let mut rows = Vec::new();
                            state
                                .for_each_config(|row| rows.push((row.index, row.text.to_vec())))
                                .unwrap();
                            states.push(rows);
                        }
                    })
                    .unwrap();
            }
        }
        states
    }
    fn command(
        host: &mut FrameHost,
        wire: &mut Wire,
        peer: std::net::SocketAddr,
        client: &mut Channel,
        text: Option<&[u8]>,
    ) -> Vec<u8> {
        if let Some(text) = text {
            client
                .queue_client_command(Protocol::Quake3_68, text)
                .unwrap();
        }
        let codec = Commands::load(Protocol::Quake3_68, Protocol::Quake3_68.message_bytes());
        let mut bytes = vec![0; Protocol::Quake3_68.message_bytes()];
        let length = codec
            .encode(
                &UserCmd {
                    server_time_ms: wire.time.milliseconds() as i32,
                    ..Default::default()
                },
                |_| 0,
                client,
                &mut bytes,
            )
            .unwrap();
        let bytes = client
            .prepare_move(&bytes[..length], wire.time, None, None)
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        client.submitted(wire.time).unwrap();
        host.queue
            .push(SysEvent {
                time: wire.time,
                kind: EventKind::Packet {
                    socket: 37,
                    from: Peer::Socket(peer),
                    bytes: &bytes,
                },
            })
            .unwrap();
        host.drain(&mut Default::default());
        bytes
    }
    let (mut host, client_id, _) = fixture(0, Phase::Server, None);
    host.connect_module_client(MODULE, client_id, b"\\name\\Downloader")
        .unwrap();
    // This fixture uses a separate native peer, without a local presentation seat.
    host.runtime.server.clients[client_id.0 as usize].connection = Some(Connection::Remote);
    let peer = ([127, 0, 0, 1], 29000).into();
    let connection = host
        .runtime
        .network
        .get_mut(client_id, Endpoint::Server)
        .unwrap();
    connection.route = Route {
        socket: 37,
        peer: Peer::Socket(peer),
    };
    let output = connection.output.unwrap();
    let mut client = Channel::load(
        Protocol::Quake3_68.channel(),
        Endpoint::Client,
        Protocol::Quake3_68.message_bytes(),
        8,
    )
    .unwrap();
    client.set_qport(connection.channel.qport());
    client
        .set_command_context(connection.channel.command_context().unwrap())
        .unwrap();
    let stalled = host
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
            1,
        )
        .unwrap();
    host.connect_module_client(MODULE, stalled, b"\\name\\Stalled")
        .unwrap();
    host.runtime.server.clients[stalled.0 as usize].connection = Some(Connection::Remote);
    host.runtime
        .network
        .get_mut(stalled, Endpoint::Server)
        .unwrap()
        .route = Route {
        socket: 37,
        peer: Peer::Socket(([127, 0, 0, 1], 29001).into()),
    };
    let stalled_output = host.runtime.server.clients[stalled.0 as usize]
        .output
        .unwrap();
    let observer = host
        .runtime
        .server
        .events
        .bind(OutputTarget::Module(ModuleId(99)))
        .unwrap();
    let mut wire = Wire {
        time: EventTime::from_milliseconds(50),
        packets: Vec::new(),
    };
    host.frame(&mut wire, true);
    let initial_states = states(&mut host, observer);
    assert_eq!(initial_states.len(), 2);
    let initial = initial_states
        .iter()
        .copied()
        .filter(|(id, _)| *id == client_id)
        .collect::<Vec<_>>();
    let stalled_state = initial_states
        .iter()
        .find(|(id, _)| *id == stalled)
        .unwrap()
        .1;
    assert_eq!(receive(&mut wire, &mut client, peer).len(), 1);
    assert!(
        host.runtime
            .server
            .events
            .retained_for(output, initial[0].1)
    );
    let old_ack = command(&mut host, &mut wire, peer, &mut client, None);
    assert!(
        !host
            .runtime
            .server
            .events
            .retained_for(output, initial[0].1)
    );
    let context = Context {
        source: RuleSetId::Quake3,
        side: qa_console::catalog::Scope::Server,
        ..Default::default()
    };
    let hostname = host.console.cvars.bind("hostname", context).unwrap();
    host.console
        .cvars
        .force_write(hostname, "after-download")
        .unwrap();
    let module_commands = count(&mut host, 6);
    command(&mut host, &mut wire, peer, &mut client, Some(b"donedl"));
    assert_eq!(count(&mut host, 6), module_commands);
    wire.time = EventTime::from_milliseconds(100);
    host.frame(&mut wire, true);
    let refreshed = states(&mut host, observer);
    assert_eq!(refreshed.len(), 1);
    assert_eq!(refreshed[0].0, client_id);
    assert!(refreshed[0].1 > initial[0].1);
    assert!(
        host.runtime
            .server
            .events
            .retained_for(stalled_output, stalled_state)
    );
    let state = receive(&mut wire, &mut client, peer);
    assert_eq!(state.len(), 1);
    assert!(state[0].iter().any(|(_, text)| {
        text.windows(b"after-download".len())
            .any(|bytes| bytes == b"after-download")
    }));
    assert!(
        host.runtime
            .server
            .events
            .retained_for(output, refreshed[0].1)
    );
    host.queue
        .push(SysEvent {
            time: wire.time,
            kind: EventKind::Packet {
                socket: 37,
                from: Peer::Socket(peer),
                bytes: &old_ack,
            },
        })
        .unwrap();
    host.drain(&mut Default::default());
    assert!(
        host.runtime
            .server
            .events
            .retained_for(output, refreshed[0].1)
    );
    command(&mut host, &mut wire, peer, &mut client, None);
    assert!(
        !host
            .runtime
            .server
            .events
            .retained_for(output, refreshed[0].1)
    );
    assert!(
        host.runtime
            .server
            .events
            .retained_for(stalled_output, stalled_state)
    );
    assert!(
        host.runtime.server.clients[stalled.0 as usize]
            .connection
            .is_some()
    );
    #[path = "../model_assets/support.rs"]
    #[allow(dead_code)]
    mod pure_files;
    #[path = "../../../formats/tests/support/stored_zip.rs"]
    mod zip;
    let files = pure_files::Fixture::new(&[(
        "pak0.pk3",
        zip::stored_zip(&[
            (b"vm/cgame.qvm", b"server expected cgame"),
            (b"vm/ui.qvm", b"server expected ui"),
        ]),
    )]);
    let path = files
        .vfs
        .directory_path(files.vfs.mounts().next().unwrap().id)
        .unwrap()
        .join("pak0.pk3");
    let archive = std::sync::Arc::new(
        qa_formats::archive::Archive::parse(std::sync::Arc::new(
            qa_platform::files::ReadFile::open(&path).unwrap(),
        ))
        .unwrap(),
    );
    let feed = client.command_context().unwrap().checksum_feed;
    let expected = archive.q3_checksums().unwrap().pure_checksum(feed);
    let regular = archive.q3_checksums().unwrap().checksum();
    host.runtime.vfs.mount_archive(&path, 0, archive).unwrap();
    for name in [
        "sv_paks",
        "sv_pakNames",
        "sv_referencedPaks",
        "sv_referencedPakNames",
    ] {
        let view = host.console.cvars.bind(name, context).unwrap();
        host.console
            .cvars
            .force_write(view, "stale-advertisement")
            .unwrap();
    }
    let pure = host.console.cvars.bind("sv_pure", context).unwrap();
    host.console.cvars.force_write(pure, "1").unwrap();
    wire.time = EventTime::from_milliseconds(200);
    command(&mut host, &mut wire, peer, &mut client, Some(b"donedl"));
    host.frame(&mut wire, true);
    let states = receive(&mut wire, &mut client, peer);
    assert_eq!(states.len(), 1);
    let info = &states[0]
        .iter()
        .find(|(index, _)| *index == qa_core::configstrings::ConfigLayout::Q3_SYSTEM_INFO as u32)
        .unwrap()
        .1;
    let value = |name: &[u8]| {
        let range = qa_core::text::info_value(info, name, true).unwrap();
        &info[range]
    };
    assert_eq!(value(b"sv_paks"), format!("{} ", regular as i32).as_bytes());
    assert_eq!(value(b"sv_pakNames"), b"pak0");
    assert_eq!(
        value(b"sv_referencedPaks"),
        format!("{} ", regular as i32).as_bytes()
    );
    assert_eq!(
        value(b"sv_referencedPakNames"),
        format!(
            "{}/pak0",
            path.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
        )
        .as_bytes()
    );
    let server_id = client.command_context().unwrap().server_id;
    let thinks = count(&mut host, 7);
    wire.time = EventTime::from_milliseconds(210);
    command(&mut host, &mut wire, peer, &mut client, None);
    assert_eq!(count(&mut host, 7), thinks, "no move before native cp");
    wire.time = EventTime::from_milliseconds(220);
    command(
        &mut host,
        &mut wire,
        peer,
        &mut client,
        Some(format!("cp {} malformed", server_id - 1).as_bytes()),
    );
    assert_eq!(count(&mut host, 7), thinks);
    let cp = format!(
        "cp {server_id} {} {} @ {} {}",
        expected as i32,
        expected as i32,
        expected as i32,
        (feed ^ expected ^ 1) as i32
    );
    wire.time = EventTime::from_milliseconds(220);
    command(&mut host, &mut wire, peer, &mut client, Some(cp.as_bytes()));
    assert_eq!(
        count(&mut host, 7),
        thinks,
        "the authenticated first command enters the new world without a second move"
    );
    let begins = count(&mut host, 3);
    wire.time = EventTime::from_milliseconds(230);
    command(&mut host, &mut wire, peer, &mut client, None);
    assert_eq!(count(&mut host, 7), thinks + 1);
    assert_eq!(count(&mut host, 3), begins);
    wire.time = EventTime::from_milliseconds(240);
    command(&mut host, &mut wire, peer, &mut client, Some(b"vdr"));
    assert_eq!(count(&mut host, 7), thinks + 1);
    host.frame(&mut wire, true);
    assert_eq!(receive(&mut wire, &mut client, peer).len(), 1);
    wire.time = EventTime::from_milliseconds(250);
    command(&mut host, &mut wire, peer, &mut client, Some(cp.as_bytes()));
    assert_eq!(count(&mut host, 7), thinks + 1);
    assert_eq!(count(&mut host, 3), begins + 1);
    wire.time = EventTime::from_milliseconds(255);
    command(&mut host, &mut wire, peer, &mut client, None);
    assert_eq!(count(&mut host, 7), thinks + 2);
    let game_commands = count(&mut host, 6);
    let disconnects = count(&mut host, 5);
    let ticks = count(&mut host, 8);
    wire.time = EventTime::from_milliseconds(260);
    command(
        &mut host,
        &mut wire,
        peer,
        &mut client,
        Some(format!("cp {server_id} 0 0 @ 0").as_bytes()),
    );
    assert_eq!(
        count(&mut host, 6),
        game_commands,
        "cp/vdr do not reach GAME_CLIENT_COMMAND"
    );
    assert_eq!(count(&mut host, 5), disconnects + 1);
    let qa_session::clients::ClientPhase::Closing(closing) =
        host.runtime.server.clients[client_id.0 as usize].phase
    else {
        panic!("invalid pure peer must retain native close output");
    };
    let (_, sequence) = closing.final_output.unwrap();
    assert!(host.runtime.server.events.retained_for(output, sequence));
    assert!(host.runtime.server.clients[stalled.0 as usize].is_active());
    wire.time = EventTime::from_milliseconds(300);
    host.frame(&mut wire, true);
    receive(&mut wire, &mut client, peer);
    let state = client.command_state().unwrap();
    assert!(
        (1..=state.received).any(
            |sequence| client.server_command(sequence).is_ok_and(|text| text
                .starts_with(b"disconnect \"")
                && text
                    .windows(qa_network::pure::REJECTION.len())
                    .any(|part| part == qa_network::pure::REJECTION))
        )
    );
    assert!(count(&mut host, 8) > ticks);
    assert!(host.runtime.server.clients[stalled.0 as usize].is_active());
    assert!(!host.runtime.quit);
    wire.time = EventTime::from_milliseconds(310);
    command(&mut host, &mut wire, peer, &mut client, None);
    assert!(
        host.runtime.server.clients[client_id.0 as usize]
            .connection
            .is_none(),
        "only the native ACK may retire the closing client's output"
    );
    assert_eq!(count(&mut host, 5), disconnects + 1);
    assert!(host.runtime.server.clients[stalled.0 as usize].is_active());
}

#[test]
fn q3_native_snapshot_is_inactive_until_the_first_zero_time_move() {
    use qa_core::loopback::Endpoint;
    let (mut host, id, _) = fixture(0, Phase::Server, None);
    host.connect_module_client(MODULE, id, b"\\name\\Primed")
        .unwrap();
    tick(&mut host, 0);
    assert_eq!(count(&mut host, 3), 0);
    // Exercise the shared SERVER serializer independently of the host's
    // gamestate-ACK scheduling gate, before a usercmd has entered the player.
    let snapshot = |host: &mut FrameHost, milliseconds: u64| {
        assert!(host.runtime.send_snapshot(
            id,
            EventTime::from_milliseconds(milliseconds),
            None,
            &mut Source {
                time: EventTime(0),
                polls: 0
            },
            |_, _| None,
            |_, _, _, _| None,
        ));
    };
    snapshot(&mut host, 50);
    let flags = |host: &FrameHost| {
        let frame = host
            .runtime
            .network
            .get(id, Endpoint::Server)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        let qa_network::snapshots::ReceivedFrame::Quake3(frame) = frame else {
            panic!("expected Q3 snapshot")
        };
        frame.flags
    };
    assert_eq!(flags(&host), qa_network::snapshots::Q3_NOT_ACTIVE);
    assert_eq!(count(&mut host, 3), 0);
    assert!(host.runtime.send_client_command(
        id,
        &UserCmd::default(),
        EventTime::from_milliseconds(50),
        &mut Source {
            time: EventTime(0),
            polls: 0
        }
    ));
    host.drain(&mut Default::default());
    assert_eq!((count(&mut host, 3), count(&mut host, 7)), (1, 0));
    snapshot(&mut host, 100);
    assert_eq!(flags(&host), 0);
    assert!(host.runtime.send_client_command(
        id,
        &UserCmd::default(),
        EventTime::from_milliseconds(100),
        &mut Source {
            time: EventTime(0),
            polls: 0
        }
    ));
    host.drain(&mut Default::default());
    assert_eq!((count(&mut host, 3), count(&mut host, 7)), (1, 0));
}
