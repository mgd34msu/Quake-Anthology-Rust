//! Native public player metadata traverses the normal queued snapshot consumer.
use super::*;
use qa_app::{client_policy::ClientPolicy, map::SpawnAnchor};
use qa_core::loopback::Endpoint;
use qa_formats::program::qvm::Opcode as Op;
use qa_network::{commands::packet::Protocol, snapshots::ReceivedFrame, states};
use qa_session::clients::Connection;

const MODULE: ModuleId = ModuleId(1);
const ENTITIES: i32 = 4096;
const PLAYERS: i32 = 16384;
const ENTITY_STRIDE: i32 = 608;
const PLAYER_STRIDE: i32 = 512;
const THINK_CLIENT: i32 = 64;
const THINK_COUNT: i32 = 68;
const RESIDUAL: u32 = 512 | 1024;
const HOT: u32 = 1 | 2 | 32 | 64 | 256;

fn program(gravity: i32, speed: i32, followed_client: i32) -> qa_compat::qvm::Vm {
    let mut code = vec![(Op::Enter, 64)];
    let mut branches = Vec::new();
    for export in [0, 7, 9] {
        code.extend([
            (Op::Local, 72),
            (Op::Load4, 0),
            (Op::Const, export),
            (Op::Eq, 0),
        ]);
        branches.push(code.len() - 1);
    }
    code.extend([(Op::Const, 0), (Op::Leave, 64)]);
    code[branches[0]].1 = code.len() as i32;
    for (index, value) in [ENTITIES, 2, ENTITY_STRIDE, PLAYERS, PLAYER_STRIDE]
        .into_iter()
        .enumerate()
    {
        code.extend([(Op::Const, value), (Op::Arg, 8 + index as i32 * 4)]);
    }
    code.extend([
        (Op::Const, -16), // G_LOCATE_GAME_DATA
        (Op::Call, 0),
        (Op::Pop, 0),
        (Op::Const, 0),
        (Op::Leave, 64),
    ]);
    code[branches[1]].1 = code.len() as i32;
    code.extend([
        (Op::Const, THINK_CLIENT),
        (Op::Local, 76),
        (Op::Load4, 0),
        (Op::Store4, 0),
        (Op::Const, THINK_COUNT),
        (Op::Const, THINK_COUNT),
        (Op::Load4, 0),
        (Op::Const, 1),
        (Op::Add, 0),
        (Op::Store4, 0),
        (Op::Const, 0),
        (Op::Leave, 64),
    ]);
    code[branches[2]].1 = code.len() as i32;
    code.extend([
        (Op::Local, 76),
        (Op::Load4, 0),
        (Op::Load4, 0),
        (Op::Leave, 64),
    ]);
    let mut data = vec![0; PLAYERS as usize + PLAYER_STRIDE as usize * 2];
    for index in 0..2 {
        let at = ENTITIES as usize + index * ENTITY_STRIDE as usize;
        data[at..at + 4].copy_from_slice(&(index as u32).to_le_bytes());
        data[at + 512..at + 516].copy_from_slice(&1023i32.to_le_bytes());
    }
    for (offset, word) in [
        (4, 1), // PM_NOCLIP; this fixture does not pretend to execute stock Pmove.
        (12, (RESIDUAL | HOT) as i32),
        (20, 32.0f32.to_bits() as i32),
        (44, -123),
        (48, gravity),
        (52, speed),
        (68, 1022), // native world contact is grounded without a common entity id.
        (140, followed_client),
        (144, 4), // native WP_GRENADE_LAUNCHER
        (184, 90),
    ] {
        let at = PLAYERS as usize + offset;
        data[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
    let mut vm = service_program::program(&code, &data);
    vm.configure_q3_game();
    vm
}

fn fixture(gravity: i32, speed: i32, followed_client: i32) -> (Box<FrameHost>, ClientId) {
    let mut runtime = Runtime::load(2, []).unwrap();
    assert_eq!(
        runtime.server.connect(Connection::Bot, ModuleId(2), None),
        Some(ClientId(0))
    );
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                client: RuleSetId::Quake3,
                movement: RuleSetId::QuakeWorld,
                trace: RuleSetId::Quake2,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    assert_eq!(client, ClientId(1));
    let tuning = &mut runtime.server.clients[1].player.movement.tuning;
    tuning.gravity_multiplier = 2.0;
    tuning.speed_multiplier = 0.5;
    let tuning = &mut runtime.prediction[0].player.movement.tuning;
    tuning.gravity_multiplier = 2.5;
    tuning.speed_multiplier = 0.0;
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
            module: MODULE,
            clock: ThinkTime::Milliseconds(0),
            server_frame: 0,
            console: context,
            allocation: AllocationPolicy::q3(0),
            link_order: LinkOrder::Head,
        },
        timing_rules: RuleSetId::Quake3,
        phase: Phase::Server,
        rate: TickRate::fixed(50).unwrap(),
        anchor,
        program: Program::Qvm {
            vm: Box::new(program(gravity, speed, followed_client)),
            imports: &Q3_SERVER,
        },
        entries: (0..=9).collect(),
        frame: Export::clocked(CallbackId(8)),
        prepare: vec![],
        initialize: vec![Export::words(CallbackId(0), &[])],
        api: None,
        shutdown: vec![],
        instruction_budget: 1000,
        configstrings: 0,
        files: 0,
        entity_source: None,
    };
    let mut host = Box::new(
        FrameHost::load_modules(
            console,
            runtime,
            TickRate::fixed(50).unwrap(),
            vec![request],
        )
        .unwrap(),
    );
    host.initialize_modules(Phase::Server);
    assert_eq!(
        host.connect_module_client(MODULE, client, b"\\name\\metadata")
            .unwrap(),
        0
    );
    (host, client)
}

fn transmit(
    host: &mut FrameHost,
    client: ClientId,
    milliseconds: u64,
) -> [u32; states::PLAYER_WORDS] {
    let time = EventTime::from_milliseconds(milliseconds);
    assert!(host.runtime.send_snapshot(
        client,
        time,
        None,
        &mut Source { time, polls: 0 },
        |_, _| None
    ));
    // Ordinary loopback packets enter the system-event queue before CLIENT import.
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut Default::default());
    let frame = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .current_snapshot()
        .unwrap();
    let ReceivedFrame::Quake3(frame) = frame else {
        panic!("expected Q3 native frame")
    };
    *frame.player
}
fn read(host: &mut FrameHost, address: i32) -> u32 {
    host.call_module_export(
        MODULE,
        Export::words(CallbackId(9), &[address as u64]),
        ThinkTime::Milliseconds(0),
    )
    .unwrap();
    let Some(ModuleResult::Qvm(value)) = host.module_counts(MODULE).unwrap().last_result else {
        panic!("expected QVM getter")
    };
    value as u32
}

fn presented_entity(host: &FrameHost, client: ClientId) -> Option<u32> {
    let frame = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .current_snapshot();
    host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .view_entity(frame)
}

fn connect_followed_client(host: &mut FrameHost) {
    assert!(host.runtime.server.bind_client_native(
        ClientId(0),
        NativeEntity {
            module: MODULE,
            slot: 1,
        }
    ));
    assert_eq!(
        host.connect_module_client(MODULE, ClientId(0), b"\\name\\followed")
            .unwrap(),
        1
    );
}

#[test]
fn q3_native_metadata_survives_ordinary_snapshot_and_client_import_with_hot_flag_overlay() {
    let (mut host, client) = fixture(1200, 250, 0);
    let expected_weapon = host
        .runtime
        .catalog
        .registry
        .native_weapon(RuleSetId::Quake3, 4)
        .unwrap();
    let player = &mut host.runtime.server.clients[1].player;
    assert_eq!(player.native.unwrap().gravity, Some(1200));
    assert_eq!(player.native.unwrap().speed, Some(250));
    assert_eq!(player.inventory.weapon, expected_weapon);
    assert_eq!(
        player.movement.tuning.gravity.unwrap().effective(2.0),
        1200.0
    );
    assert_eq!(
        player.movement.tuning.max_speed.unwrap().effective(0.5),
        250.0
    );
    player.movement.ducked = false;
    player.movement.jump_held = true;
    player.movement.timer = MovementTimer(MovementTimer::KNOCKBACK.0);
    let words = transmit(&mut host, client, 100);
    assert_eq!(
        (words[24], words[25], words[40], words[41]),
        (1200, 250, 0, 4)
    );
    assert_eq!(words[19], RESIDUAL | 2 | 64);
    let player = &host.runtime.prediction[0].player;
    let tail = player.native.unwrap();
    assert_eq!(tail.namespace, ResourceNamespace::Client(client));
    assert_eq!(
        (tail.gravity, tail.speed, tail.weapon_number),
        (Some(1200), Some(250), Some(4))
    );
    assert_eq!(tail.flags, RESIDUAL | 2 | 64);
    assert_eq!(player.inventory.weapon, expected_weapon);
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    assert_eq!(
        player.movement.tuning.gravity.unwrap().effective(2.5),
        1200.0
    );
    assert_eq!(
        player.movement.tuning.max_speed.unwrap().effective(0.0),
        250.0
    );
    assert_eq!(host.runtime.snapshots[1].as_ref().unwrap().applied, 1);
    assert!(host.runtime.prediction[1].player.native.is_none());
}

#[test]
fn q3_followed_player_number_does_not_replace_the_native_callback_identity() {
    let (mut host, client) = fixture(1200, 250, 1);
    connect_followed_client(&mut host);
    assert!(host.runtime.send_local_command(
        client,
        &UserCmd {
            server_time_ms: 50,
            duration_ms: 50,
            ..Default::default()
        },
        EventTime::from_milliseconds(50)
    ));
    host.drain(&mut Default::default());
    assert_eq!(read(&mut host, THINK_CLIENT), 0);
    assert_eq!(read(&mut host, THINK_COUNT), 1);
    assert_eq!(
        host.runtime.snapshots[1].as_ref().unwrap().native_client(),
        0
    );
    assert_eq!(
        host.runtime.server.module_moves(client),
        Some(NativeEntity {
            module: MODULE,
            slot: 0
        })
    );
    let words = transmit(&mut host, client, 100);
    assert_eq!(words[40], 1);
    assert_eq!(presented_entity(&host, client), Some(1));
    assert_eq!(
        host.runtime.snapshots[1].as_ref().unwrap().native_client(),
        0
    );
    assert_eq!(
        host.runtime.prediction[0]
            .player
            .native
            .unwrap()
            .client_number,
        Some(1)
    );
}

#[test]
fn q3_signed_native_magnitudes_keep_their_exact_low_wire_bits_above_binary32_integer_precision() {
    let gravity = -0x0100_1235i32;
    let speed = 0x0100_4321i32;
    let (mut host, client) = fixture(gravity, speed, 0);
    let tail = host.runtime.server.clients[1].player.native.unwrap();
    assert_eq!((tail.gravity, tail.speed), (Some(gravity), Some(speed)));
    let words = transmit(&mut host, client, 100);
    assert_eq!(
        (words[24], words[25]),
        (gravity as u32 & 65535, speed as u32 & 65535)
    );
    let tail = host.runtime.prediction[0].player.native.unwrap();
    assert_eq!(
        (tail.gravity, tail.speed),
        (
            Some((gravity as u32 & 65535) as i32),
            Some((speed as u32 & 65535) as i32)
        )
    );
}

#[test]
fn q3_snapshot_does_not_borrow_another_namespace_or_source_tail_and_shared_reset_clears_it() {
    for (source, namespace) in [
        (RuleSetId::Quake3, ResourceNamespace::Module(ModuleId(9))),
        (
            RuleSetId::Quake2Rerelease,
            ResourceNamespace::Module(MODULE),
        ),
    ] {
        let (mut host, client) = fixture(1200, 250, 1);
        let player = &mut host.runtime.server.clients[1].player;
        player.native = Some(NativePlayerTail {
            flags: RESIDUAL,
            client_number: Some(1),
            weapon_number: Some(99),
            gravity: Some(666),
            speed: Some(777),
            ..NativePlayerTail::new(source, namespace)
        });
        let words = transmit(&mut host, client, 100);
        assert_eq!(
            (words[24], words[25], words[40], words[41]),
            (1200, 250, 0, 4)
        );
        assert_eq!(words[19], HOT);
        assert_eq!(presented_entity(&host, client), Some(0));
        host.runtime.server.clients[1].player.reset();
        assert!(host.runtime.server.clients[1].player.native.is_none());
        let words = transmit(&mut host, client, 200);
        assert_eq!((words[19], words[40], words[41]), (0, 0, 0));
        assert_eq!(
            host.runtime.prediction[0].player.inventory.weapon,
            WeaponId::default()
        );
        assert!(host.runtime.server.clients[0].player.native.is_none());
    }
}

#[test]
fn q3_native_world_contact_survives_the_queue_until_the_common_grounded_flag_clears() {
    let (mut host, client) = fixture(1200, 250, 0);
    let player = &host.runtime.server.clients[1].player;
    assert!(player.movement.grounded);
    assert!(player.movement.ground.is_none());
    let words = transmit(&mut host, client, 100);
    assert_eq!(words[20], 1022);
    let player = &host.runtime.prediction[0].player;
    assert!(player.movement.grounded);
    assert!(player.movement.ground.is_none());
    host.runtime.server.clients[1].player.movement.grounded = false;
    let words = transmit(&mut host, client, 200);
    assert_eq!(words[20], 1023);
    let player = &host.runtime.prediction[0].player;
    assert!(!player.movement.grounded);
    assert!(player.movement.ground.is_none());
}

#[test]
fn q3_server_follow_target_changes_presentation_only_after_client_packet_acceptance() {
    let (mut host, client) = fixture(1200, 250, 0);
    connect_followed_client(&mut host);
    let words = transmit(&mut host, client, 100);
    assert_eq!(words[40], 0);
    assert_eq!(presented_entity(&host, client), Some(0));
    let applied = host.runtime.snapshots[1].as_ref().unwrap().applied;

    host.runtime.server.clients[1]
        .player
        .native
        .as_mut()
        .unwrap()
        .client_number = Some(1);
    let time = EventTime::from_milliseconds(200);
    assert!(host.runtime.send_snapshot(
        client,
        time,
        None,
        &mut Source { time, polls: 0 },
        |_, _| None
    ));
    // Encoding and submission mutate SERVER state, not the accepted CLIENT view.
    assert_eq!(presented_entity(&host, client), Some(0));
    assert_eq!(host.runtime.snapshots[1].as_ref().unwrap().applied, applied);
    assert_eq!(
        host.runtime.prediction[0]
            .player
            .native
            .unwrap()
            .client_number,
        Some(0)
    );

    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut Default::default());
    assert_eq!(presented_entity(&host, client), Some(1));
    assert_eq!(
        host.runtime.snapshots[1].as_ref().unwrap().applied,
        applied + 1
    );
    assert_eq!(
        host.runtime.prediction[0]
            .player
            .native
            .unwrap()
            .client_number,
        Some(1)
    );
    assert_eq!(
        host.runtime.snapshots[1].as_ref().unwrap().native_client(),
        0
    );
}

struct NoPredictionTrace;
impl qa_movement::TraceServices for NoPredictionTrace {
    fn trace(
        &mut self,
        _: qa_world::collision::TraceQuery,
    ) -> Result<qa_world::collision::Trace, qa_world::collision::TraceError> {
        panic!("accepted FOLLOW player must not enter movement tracing")
    }

    fn point_contents(
        &self,
        _: Vec3,
        _: qa_world::collision::EntityTracePolicy,
    ) -> Result<qa_world::collision::Contents, qa_world::collision::TraceError> {
        panic!("accepted FOLLOW player must not sample movement contents")
    }
}

#[test]
fn q3_accepted_follow_flag_keeps_received_pose_when_prediction_has_a_nonzero_command() {
    let (mut host, client) = fixture(1200, 250, 0);
    host.runtime.server.clients[1]
        .player
        .native
        .as_mut()
        .unwrap()
        .flags |= 4096; // PMF_FOLLOW is native metadata, not an engine movement flag.
    let words = transmit(&mut host, client, 100);
    assert_eq!(words[19] & 4096, 4096);
    let prediction = &mut host.runtime.prediction[0];
    assert!(prediction.player.interpolate_only);
    let before = prediction.player.body;
    let view = prediction.player.view_angles;
    let command_time = prediction.player.movement.command_time_ms;
    let result = prediction
        .advance(
            UserCmd {
                server_time_ms: 200,
                duration_ms: 100,
                movement: [127.0, 64.0, 32.0],
                view_angles: Vec3([45.0, 90.0, 10.0]),
                buttons: buttons::JUMP,
                ..Default::default()
            },
            &mut NoPredictionTrace,
        )
        .unwrap();
    assert_eq!(
        (result.steps, result.traces, result.contact_count),
        (0, 0, 0)
    );
    for (actual, expected) in [
        (prediction.player.body.position, before.position),
        (prediction.player.body.velocity, before.velocity),
        (prediction.player.body.mins, before.mins),
        (prediction.player.body.maxs, before.maxs),
        (prediction.player.view_angles, view),
    ] {
        assert_eq!(actual.0.map(f32::to_bits), expected.0.map(f32::to_bits));
    }
    assert_eq!(prediction.player.movement.command_time_ms, command_time);

    host.runtime.server.clients[1]
        .player
        .native
        .as_mut()
        .unwrap()
        .flags &= !4096;
    let words = transmit(&mut host, client, 200);
    assert_eq!(words[19] & 4096, 0);
    assert!(!host.runtime.prediction[0].player.interpolate_only);
}
