#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;
#[path = "support/service_program.rs"]
mod service_program;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult},
    map::SpawnAnchor,
    modules::{Argument, Export, ModuleRequest, ModuleResult, Phase, Program, State},
};
use qa_compat::{abi::Q3_CLIENT, services::CallContext};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::*,
    sys_events::{EventTime, SeatId},
};
use qa_formats::program::qvm::Opcode;
use qa_network::{
    commands::packet::Protocol,
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use qa_session::timing::TickRate;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

const SNAPSHOT_BYTES: usize = 53772;
const QVM_OUTPUT: u64 = 1024;
const GET_GAME_STATE: u32 = 7;
const SELECT_COMMAND: u32 = 8;
const READ_BYTE: u32 = 9;

fn qvm() -> (Program, u64) {
    use Opcode::*;
    let mut operations = vec![(Enter, 64)];
    let mut branches = Vec::new();
    for command in 0..10 {
        operations.extend([(Local, 72), (Load4, 0), (Const, command), (Eq, 0)]);
        branches.push(operations.len() - 1);
    }
    operations.extend([(Const, -1), (Leave, 64)]);
    for (command, branch) in branches.into_iter().enumerate() {
        operations[branch].1 = operations.len() as i32;
        match command {
            0 | 1 => {
                operations.extend([
                    (Const, QVM_OUTPUT as i32),
                    (Arg, 8),
                    (Const, QVM_OUTPUT as i32 + if command == 0 { 4 } else { 0 }),
                    (Arg, 12),
                    (Const, -52),
                    (Call, 0),
                    (Pop, 0),
                    (Const, QVM_OUTPUT as i32),
                    (Load4, 0),
                ]);
            }
            2 | 5 => {
                operations.extend([
                    (Local, 76),
                    (Load4, 0),
                    (Arg, 8),
                    (Local, 80),
                    (Load4, 0),
                    (Arg, 12),
                    (Const, if command == 2 { -53 } else { -52 }),
                    (Call, 0),
                ]);
            }
            3 => operations.extend([(Local, 76), (Load4, 0), (Load4, 0)]),
            4 => operations.extend([
                (Local, 76),
                (Load4, 0),
                (Arg, 8),
                (Local, 80),
                (Load4, 0),
                (Arg, 12),
                (Local, 84),
                (Load4, 0),
                (Arg, 16),
                (Const, -101),
                (Call, 0),
            ]),
            6 => operations.extend([(Local, 76), (Load4, 0)]),
            7 | 8 => operations.extend([
                (Local, 76),
                (Load4, 0),
                (Arg, 8),
                (Const, if command == 7 { -51 } else { -54 }),
                (Call, 0),
            ]),
            9 => operations.extend([(Local, 76), (Load4, 0), (Load1, 0)]),
            _ => unreachable!(),
        }
        operations.push((Leave, 64));
    }
    (
        Program::Qvm {
            vm: Box::new(service_program::program(&operations, &[0; 256])),
            imports: &Q3_CLIENT,
        },
        QVM_OUTPUT,
    )
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native() -> (Program, u64) {
    use native_image::*;
    use qa_compat::native::{NamedExport, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    let mut file = pe_export_fixture(64);
    put(&mut file, 392 + 8, 65536, 4);
    put(&mut file, 392 + 16, 65536, 4);
    put(&mut file, 208, 69632, 4);
    file.resize(65536 + 512, 0);
    pe_rva(&mut file, 0x1140, 0x1800, 4);
    pe_text(&mut file, 0x1800, &[0xc3]);
    let mut vm = Vm::map_image(
        Image::parse(&file, None, LoadRole::Library).unwrap(),
        &(0..10)
            .map(|command| NamedExport {
                name: b"GetGameAPI",
                command: Some(command),
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            })
            .collect::<Vec<_>>(),
        &[],
        std::time::Duration::from_secs(3),
    )
    .unwrap();
    let output = vm.process.base() + 0x2000;
    let callback = vm.import_callback();
    let mut code = Vec::new();
    for command in 0..10u8 {
        code.extend([0x83, 0xf9, command, 0x0f, 0x85]);
        let branch = code.len();
        code.extend([0; 4]);
        if command == 3 {
            code.extend([0x8b, 0x02, 0xc3]); // mov eax,[rdx]; ret
        } else if command == 6 {
            code.extend([0x89, 0xd0, 0xc3]); // mov eax,edx; ret
        } else if command == 9 {
            code.extend([0x0f, 0xb6, 0x02, 0xc3]); // movzx eax,byte [rdx]; ret
        } else {
            code.extend([0x48, 0x83, 0xec, 0x28]);
            if matches!(command, 0 | 1) {
                code.extend([0x48, 0xba]);
                code.extend(output.to_le_bytes());
                code.extend([0x49, 0xb8]);
                code.extend((output + if command == 0 { 4 } else { 0 }).to_le_bytes());
            }
            let ordinal = match command {
                2 => 52,
                4 => 100,
                7 => 50,
                8 => 53,
                _ => 51,
            };
            code.extend([0xb9, ordinal, 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0]);
            if matches!(command, 0 | 1) {
                code.extend([0x48, 0xb8]);
                code.extend(output.to_le_bytes());
                code.extend([0x8b, 0x00]);
            }
            code.extend([0x48, 0x83, 0xc4, 0x28, 0xc3]);
        }
        let next = code.len();
        code[branch..branch + 4].copy_from_slice(&((next - branch - 4) as i32).to_le_bytes());
    }
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(code.len() < 0x800);
    vm.process.memory_mut().unwrap()[0x1800..0x1800 + code.len()].copy_from_slice(&code);
    (
        Program::Native {
            vm: Box::new(vm),
            imports: &Q3_CLIENT,
        },
        output,
    )
}

fn host(factory: fn() -> (Program, u64), clients: usize) -> (FrameHost, Vec<ClientId>, u64) {
    host_with_startup(factory, clients, None)
}

fn host_with_startup(
    factory: fn() -> (Program, u64),
    clients: usize,
    startup: Option<Argument>,
) -> (FrameHost, Vec<ClientId>, u64) {
    let mut runtime = Runtime::load(clients, []).unwrap();
    let mut requests = Vec::new();
    let mut ids = Vec::new();
    let mut output = 0;
    for seat in 0..clients {
        let client = runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake,
                    trace: RuleSetId::QuakeWorld,
                },
                Protocol::Quake3_68,
                seat as u32,
            )
            .unwrap();
        let module = ModuleId(seat as u16 + 1);
        let anchor = runtime
            .server
            .entities
            .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
            .unwrap()
            .id;
        let (program, address) = factory();
        if output != 0 {
            assert_eq!(address, output);
        }
        output = address;
        requests.push(ModuleRequest {
            context: CallContext {
                module,
                clock: ThinkTime::Milliseconds(9000),
                server_frame: 0,
                native_rules: RuleSetId::Quake3,
                trace_rules: RuleSetId::Quake2,
                sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::QuakeWorld),
                console: Context {
                    source: RuleSetId::Quake2,
                    seat: SeatId::new(seat as u8).unwrap(),
                    ..Context::default()
                },
                allocation: AllocationPolicy::q3(0),
                link_order: LinkOrder::Head,
            },
            phase: Phase::Client,
            timing_rules: RuleSetId::QuakeWorld,
            rate: TickRate::FrameDriven,
            anchor,
            program,
            entries: (0..10).collect(),
            frame: Export {
                client: Some(client),
                ..Export::clocked(CallbackId(6))
            },
            prepare: vec![],
            initialize: startup
                .into_iter()
                .map(|argument| {
                    let mut export = Export::words(CallbackId(6), &[]);
                    export.client = Some(client);
                    export.arguments[0] = argument;
                    export
                })
                .collect(),
            api: None,
            shutdown: vec![],
            instruction_budget: 10000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        });
        ids.push(client);
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    for (seat, &id) in ids.iter().enumerate() {
        host.local_clients[seat] = Some(id);
    }
    (host, ids, output)
}

fn call(host: &mut FrameHost, seat: usize, command: u32, words: &[u64]) -> Result<u32, ()> {
    let module = ModuleId(seat as u16 + 1);
    host.call_module_export(
        module,
        Export {
            client: Some(ClientId(seat as u32)),
            ..Export::words(CallbackId(command), words)
        },
        ThinkTime::Milliseconds(9000),
    )
    .map_err(|_| ())?;
    Ok(
        match host.module_counts(module).unwrap().last_result.unwrap() {
            ModuleResult::Qvm(value) => value as u32,
            ModuleResult::Native(value) => value as u32,
            _ => unreachable!(),
        },
    )
}
fn word(host: &mut FrameHost, seat: usize, address: u64) -> u32 {
    call(host, seat, 3, &[address]).unwrap()
}
fn fill(host: &mut FrameHost, seat: usize, address: u64) {
    call(host, seat, 4, &[address, 0xa5, SNAPSHOT_BYTES as u64]).unwrap();
}

fn config_commands(host: &mut FrameHost, client: ClientId, rows: &[(u32, &[u8])]) -> (u32, u32) {
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    let first = channel.command_state().unwrap().queued + 1;
    for &(index, text) in rows {
        let mut sent = 0;
        loop {
            let mut bytes = [0; 1024];
            let chunk = qa_network::outputs::config_command(index, text, sent, &mut bytes).unwrap();
            channel.queue_reliable(&bytes[..chunk.length]).unwrap();
            if chunk.complete {
                break;
            }
            sent = chunk.next;
        }
    }
    let last = channel.command_state().unwrap().queued;
    let time = EventTime::from_milliseconds(100);
    loop {
        let Some(packet) = channel.prepare_output(time).unwrap() else {
            break;
        };
        host.runtime
            .loopback
            .send(Endpoint::Server, client, packet.bytes)
            .unwrap();
        channel.submitted(time).unwrap();
        if !channel.pending_fragments() {
            break;
        }
    }
    host.time = time;
    host.drain(&mut FrameResult::default());
    (first, last)
}

fn select_configs(host: &mut FrameHost, seat: usize, range: (u32, u32)) {
    for sequence in range.0..=range.1 {
        call(host, seat, SELECT_COMMAND, &[u64::from(sequence)]).unwrap();
    }
}

fn assert_game_state(host: &mut FrameHost, seat: usize, output: u64, rows: &[(u32, &[u8])]) {
    use qa_compat::configstrings::{
        Q3_GAME_STATE_BYTES, Q3_GAME_STATE_COUNT_OFFSET, Q3_GAME_STATE_DATA_OFFSET,
        Q3_GAME_STATE_ROWS,
    };
    fill(host, seat, output);
    assert_eq!(call(host, seat, GET_GAME_STATE, &[output]), Ok(0));
    let mut expected = vec![0; Q3_GAME_STATE_BYTES];
    let mut position = 1usize;
    let mut checks = vec![
        0,
        8,
        19 * 4,
        22 * 4,
        31 * 4,
        33 * 4,
        511 * 4,
        1023 * 4,
        Q3_GAME_STATE_DATA_OFFSET,
        Q3_GAME_STATE_COUNT_OFFSET - 4,
        Q3_GAME_STATE_COUNT_OFFSET,
    ];
    for index in 0..Q3_GAME_STATE_ROWS {
        let Some((_, text)) = rows.iter().find(|(row, _)| *row as usize == index) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        expected[index * 4..index * 4 + 4].copy_from_slice(&(position as i32).to_le_bytes());
        checks.push(index * 4);
        let first = Q3_GAME_STATE_DATA_OFFSET + position;
        expected[first..first + text.len()].copy_from_slice(text);
        checks.extend((first..first + text.len() + 1).step_by(4));
        position += text.len() + 1;
    }
    expected[Q3_GAME_STATE_COUNT_OFFSET..].copy_from_slice(&(position as i32).to_le_bytes());
    for offset in checks {
        let bytes = &expected[offset..offset + 4];
        assert_eq!(
            word(host, seat, output + offset as u64),
            u32::from_le_bytes(bytes.try_into().unwrap()),
            "gameState byte {offset}"
        );
    }
    assert_eq!(
        word(host, seat, output + Q3_GAME_STATE_BYTES as u64),
        0xa5a5_a5a5
    );
}

fn accepted_client_game_state(factory: fn() -> (Program, u64)) {
    let (mut host, ids, output) = host(factory, 2);
    assert_game_state(&mut host, 0, output, &[]);
    let rows: [(u32, &[u8]); 5] = [
        (32, b"models/raw\x80%.md3"),
        (20, b"baseq3-1"),
        (0, b"\\mapname\\q3dm1"),
        (21, b"1000"),
        (1, b"\\sv_serverid\\42"),
    ];
    let accepted: [(u32, &[u8]); 5] = [
        (32, b"models/raw...md3"),
        rows[1],
        rows[2],
        rows[3],
        rows[4],
    ];
    let commands = config_commands(&mut host, ids[0], &rows);
    // Receipt retains commands; selecting them owns config mutation.
    assert_game_state(&mut host, 0, output, &[]);
    select_configs(&mut host, 0, commands);
    assert_game_state(&mut host, 0, output, &accepted);
    let commands = config_commands(&mut host, ids[1], &[(20, b"other seat")]);
    select_configs(&mut host, 1, commands);
    assert_game_state(&mut host, 1, output, &[(20, b"other seat")]);
    assert_game_state(&mut host, 0, output, &accepted);

    let revision = host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .configs
        .config(32)
        .unwrap()
        .revision;
    let commands = config_commands(&mut host, ids[0], &[(32, rows[0].1)]);
    select_configs(&mut host, 0, commands);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .unwrap()
            .revision,
        revision
    );
    let commands = config_commands(&mut host, ids[0], &[(20, b"replacement"), (32, b"")]);
    assert_game_state(&mut host, 0, output, &accepted);
    select_configs(&mut host, 0, commands);
    let changed: [(u32, &[u8]); 4] = [
        (0, rows[2].1),
        (1, rows[4].1),
        (20, b"replacement"),
        (21, rows[3].1),
    ];
    assert_game_state(&mut host, 0, output, &changed);
    // Packing itself preserves accepted raw bytes. Q3 wire sanitation above
    // is a separate boundary and must not be repeated by the guest getter.
    assert!(host.runtime.client_resources[0].as_mut().unwrap().apply(
        8,
        b"local\x80\xff%",
        None,
        None
    ));
    let mut with_raw = changed.to_vec();
    with_raw.push((8, b"local\x80\xff%"));
    assert_game_state(&mut host, 0, output, &with_raw);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[0]), Err(()));
    assert_game_state(&mut host, 1, output, &[(20, b"other seat")]);
}

fn game_state_pointer_and_aggregate_limits() {
    use qa_compat::configstrings::*;
    let (mut host, ids, output) = host(qvm, 1);
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[65000]), Err(()));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[65536]), Ok(0));
    assert_eq!(word(&mut host, 0, Q3_GAME_STATE_COUNT_OFFSET as u64), 1);
    assert_eq!(
        call(&mut host, 0, READ_BYTE, &[Q3_GAME_STATE_DATA_OFFSET as u64]),
        Ok(0)
    );

    let first = vec![b'e'; 7998];
    let second = vec![b'h'; 7999];
    let commands = config_commands(&mut host, ids[0], &[(700, &first), (701, &second)]);
    select_configs(&mut host, 0, commands);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Ok(0));
    assert_eq!(
        word(&mut host, 0, output + Q3_GAME_STATE_COUNT_OFFSET as u64),
        Q3_GAME_STATE_DATA_BYTES as u32
    );
    assert_eq!(word(&mut host, 0, output + 700 * 4), 1);
    assert_eq!(word(&mut host, 0, output + 701 * 4), 8000);
    assert_eq!(
        call(
            &mut host,
            0,
            READ_BYTE,
            &[output + Q3_GAME_STATE_COUNT_OFFSET as u64 - 1]
        ),
        Ok(0)
    );
    let revision = host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .configs
        .config(700)
        .unwrap()
        .revision;
    // Retire the first native command flight through a real queued CLIENT ACK.
    assert!(
        host.runtime
            .send_local_command(ids[0], &UserCmd::default(), host.time)
    );
    host.drain(&mut FrameResult::default());
    let commands = config_commands(&mut host, ids[0], &[(700, &first)]);
    select_configs(&mut host, 0, commands);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(700)
            .unwrap()
            .revision,
        revision
    );

    let commands = config_commands(&mut host, ids[0], &[(702, b"x")]);
    // The same row admission checks the complete native extent before changing
    // the accepted text or its revision. The queued command is consumed below.
    assert!(
        !host.runtime.client_resources[0]
            .as_mut()
            .unwrap()
            .apply(702, b"x", None, None)
    );
    let configs = &host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .configs;
    assert_eq!(configs.config(702).unwrap().text, b"");
    assert_eq!(configs.config(702).unwrap().revision, 0);
    assert_eq!(configs.config(700).unwrap().revision, revision);
    assert_eq!(q3_game_state_data_count(configs, None), Ok(16000));

    // Even an invalid trusted table must fail the guest-copy boundary atomically.
    host.runtime.client_resources[0]
        .as_mut()
        .unwrap()
        .state
        .configs
        .set(702, b"x")
        .unwrap();
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Err(()));
    for offset in [0, 4096, Q3_GAME_STATE_COUNT_OFFSET] {
        assert_eq!(word(&mut host, 0, output + offset as u64), 0xa5a5_a5a5);
    }
    host.runtime.client_resources[0]
        .as_mut()
        .unwrap()
        .state
        .configs
        .set(702, b"")
        .unwrap();
    assert_eq!(
        call(&mut host, 0, SELECT_COMMAND, &[u64::from(commands.0)]),
        Err(())
    );
    assert!(host.runtime.server.clients[0].connection.is_none());
}

fn snapshot(
    host: &mut FrameHost,
    client: ClientId,
    time: i32,
    count: usize,
    queued_command: bool,
) -> u32 {
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    if queued_command {
        channel
            .queue_reliable(b"print \"snapshot command\"")
            .unwrap();
    }
    let sequence = channel.send_state().sequence;
    let mut player = [0; states::PLAYER_WORDS];
    player[0] = 1234;
    player[1] = 17.25f32.to_bits();
    player[3] = 231;
    player[13] = 0x4567;
    player[24] = 777;
    player[35] = 65534;
    player[43] = (-0.0f32).to_bits();
    player[46] = 1022;
    player[47] = 513;
    player[48] = (-13i32) as u32;
    player[80] = (-1i32) as u32;
    player[96] = 0x8123_4567;
    let entities = (0..count)
        .map(|index| {
            let mut words = [0; states::ENTITY_WORDS];
            words[0] = 0x8000_1234;
            words[1] = (index as f32 + 0.25).to_bits();
            words[11] = 7;
            words[18] = 17;
            words[29] = 63;
            words[32] = 133;
            words[38] = 0x8123_4567;
            Entity {
                number: index as u32,
                words,
            }
        })
        .collect::<Vec<_>>();
    let frame = ReceivedFrame::Quake3(Frame {
        sequence,
        time: ThinkTime::Milliseconds(i64::from(time)),
        command: 0,
        flags: 4,
        qw_peers: 0,
        areas: &[0x81, 0x42],
        player: &player,
        entities: &entities,
    });
    let mut bytes = [0; 16384];
    let length = channel
        .encode_server_output(&mut bytes, |writer, channel| {
            channel.write_snapshot_candidate(writer, frame, None, 64)
        })
        .unwrap();
    channel.publish_written_snapshot(frame, None).unwrap();
    let now = EventTime::from_milliseconds(sequence as u64 * 100);
    channel
        .prepare_snapshot(sequence, None, &bytes[..length], now)
        .unwrap()
        .unwrap();
    loop {
        assert!(
            channel
                .submit_with(now, |packet| host
                    .runtime
                    .loopback
                    .send(Endpoint::Server, client, packet)
                    .is_ok())
                .unwrap()
                .is_some()
        );
        if !channel.pending_fragments() {
            break;
        }
        channel.prepare_output(now).unwrap().unwrap();
    }
    host.time = now;
    host.drain(&mut FrameResult::default());
    sequence
}

fn accepted_connection_snapshot_bytes(factory: fn() -> (Program, u64)) {
    let (mut host, ids, output) = host(factory, 2);
    assert_eq!(call(&mut host, 0, 0, &[]), Ok(0));
    assert_eq!(word(&mut host, 0, output + 4), 0);
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 2, &[0, 0]), Ok(0));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    let sequence = snapshot(&mut host, ids[0], -12345, 2, true);
    let other_sequence = snapshot(&mut host, ids[1], 45678, 1, false);
    host.runtime.server.clients[0].player.body.position = Vec3([999., 888., 777.]);
    assert_eq!(call(&mut host, 0, 0, &[]), Ok(sequence));
    assert_eq!(word(&mut host, 0, output + 4), (-12345i32) as u32);
    assert_eq!(call(&mut host, 1, 0, &[]), Ok(other_sequence));
    assert_eq!(word(&mut host, 1, output + 4), 45678);
    host.time = EventTime::from_milliseconds(500000);
    for &id in &ids {
        let channel = &host
            .runtime
            .network
            .get(id, Endpoint::Client)
            .unwrap()
            .channel;
        host.runtime.snapshots[id.0 as usize]
            .as_mut()
            .unwrap()
            .advance_presentation(channel, host.time);
    }
    host.client_modules();
    for (seat, expected) in [(-12345i32) as u32, 45678].into_iter().enumerate() {
        let actual = match host
            .module_counts(ModuleId(seat as u16 + 1))
            .unwrap()
            .last_result
            .unwrap()
        {
            ModuleResult::Qvm(value) => value as u32,
            ModuleResult::Native(value) => value as u32,
            _ => unreachable!(),
        };
        assert_eq!(
            actual, expected,
            "CLIENT frame time is accepted native time"
        );
    }
    assert_eq!(call(&mut host, 0, 1, &[]), Ok((-12345i32) as u32));
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, output]), Ok(1));
    for (offset, value) in [
        (0, 4),
        // Native zero-initialized outgoing metadata matches the nonnegative echo.
        (4, sequence * 100),
        (8, (-12345i32) as u32),
        (12, 0x0000_4281),
        (44, 1234),
        (44 + 8, 231),
        (44 + 20, 17.25f32.to_bits()),
        (44 + 48, 777),
        (44 + 56, 65534),
        (44 + 108, 0x4567),
        (44 + 136, 0),
        (44 + 184, (-13i32) as u32),
        (44 + 376, u32::MAX),
        (44 + 312, 0x8123_4567),
        (44 + 444, 513),
        (44 + 448, 1022),
        (44 + 452, 0),
        (44 + 464, 0),
        (512, 2),
        (516, 0),
        (516 + 4, 7),
        (516 + 16, 0x8000_1234),
        (516 + 24, 0.25f32.to_bits()),
        (516 + 84, 0x8123_4567),
        (516 + 140, 17),
        (516 + 160, 63),
        (516 + 204, 133),
        (516 + 208, 1),
        (516 + 208 + 24, 1.25f32.to_bits()),
        (516 + 2 * 208, 0xa5a5_a5a5),
        (53764, 0xa5a5_a5a5),
        (53768, 1),
    ] {
        assert_eq!(
            word(&mut host, 0, output + offset),
            value,
            "offset {offset}"
        );
    }
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 2, &[u32::MAX as u64, 0]), Ok(0));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    assert_eq!(
        call(&mut host, 0, 2, &[(sequence + 1) as u64, output]),
        Err(())
    );
    assert!(host.runtime.server.clients[0].connection.is_none());
    assert!(host.runtime.server.clients[1].connection.is_some());
    assert_eq!(
        call(&mut host, 1, 2, &[other_sequence as u64, output]),
        Ok(1)
    );
    assert_eq!(word(&mut host, 1, output + 8), 45678);
}

fn complete_presentation(host: &mut FrameHost) {
    struct Sink;
    impl qa_app::host::FrameSource for Sink {
        fn begin_frame(&mut self) -> std::time::Duration {
            std::time::Duration::ZERO
        }
        fn poll_events(&mut self, _: &mut qa_core::sys_events::SysEventQueue) {}
        fn wait_time(&mut self, _: std::time::Duration) -> std::time::Duration {
            std::time::Duration::ZERO
        }
        fn elapsed(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }
        fn present(&mut self) {}
        fn sound(&mut self, _: SoundEvent) -> bool {
            true
        }
    }
    // A slot remains reserved until its retirement reaches Presentation.
    qa_app::output::dispatch(
        &mut host.runtime,
        &mut Sink,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.0,
        2.0,
    );
}

fn qvm_pointer_checks_and_retained_window() {
    let (mut host, ids, output) = host(qvm, 1);
    let sequence = snapshot(&mut host, ids[0], 2345, 1, false);
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 5, &[output, 65535]), Err(()));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, 65000]), Err(()));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    assert_eq!(call(&mut host, 0, 5, &[65536, output + 4]), Ok(0));
    assert_eq!(word(&mut host, 0, 0), sequence);
    assert_eq!(word(&mut host, 0, output + 4), 2345);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, 65536]), Ok(1));
    assert_eq!(word(&mut host, 0, 8), 2345);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, 0]), Err(()));
    for _ in 0..32 {
        snapshot(&mut host, ids[0], 2345, 0, false);
    }
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, 0]), Ok(0));
    let lifetime = host.runtime.server.clients[0].entity;
    assert!(host.runtime.disconnect_client(ids[0]));
    complete_presentation(&mut host);
    let replacement = host
        .runtime
        .connect_local(
            SeatId::new(0).unwrap(),
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake2,
                trace: RuleSetId::Quake,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    assert_eq!(replacement, ids[0]);
    assert_ne!(host.runtime.server.clients[0].entity, lifetime);
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    assert_eq!(call(&mut host, 0, 0, &[]), Err(()));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Err(()));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
}

fn snapshot_entity_count_is_truncated_to_the_native_public_array() {
    let (mut host, ids, output) = host(qvm, 1);
    let sequence = snapshot(&mut host, ids[0], 7777, 300, false);
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, output]), Ok(1));
    assert_eq!(word(&mut host, 0, output + 512), 256);
    assert_eq!(word(&mut host, 0, output + 516 + 255 * 208), 255);
    assert_eq!(word(&mut host, 0, output + 53764), 0xa5a5_a5a5);
    assert_eq!(word(&mut host, 0, output + 53768), 0);
}

fn ui_keeps_its_queued_clock() {
    fn ui() -> (Program, u64) {
        let (mut program, output) = qvm();
        let Program::Qvm { imports, .. } = &mut program else {
            unreachable!()
        };
        *imports = &qa_compat::abi::Q3_UI;
        (program, output)
    }
    let (mut host, ids, _) = host(ui, 1);
    snapshot(&mut host, ids[0], 1234, 0, false);
    host.time = EventTime::from_milliseconds(500000);
    let channel = &host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel;
    host.runtime.snapshots[0]
        .as_mut()
        .unwrap()
        .advance_presentation(channel, host.time);
    host.client_modules();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(500000))
    );
}

fn gamestate_packets(
    host: &mut FrameHost,
    client: ClientId,
    native_client: i32,
    rows: &[(u32, &[u8])],
) -> Vec<Vec<u8>> {
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    let command_sequence = channel.command_state().unwrap().queued;
    let mut bytes = [0; 32768];
    let length = channel
        .encode_server_output(&mut bytes, |writer, _| {
            writer.write_bits(2, 8)?;
            writer.write_bits(command_sequence, 32)?;
            for &(index, text) in rows {
                writer.write_bits(3, 8)?;
                writer.write_bits(index, 16)?;
                // Incoming reference bytes exercise ReadBigString's native
                // high-byte preservation and percent conversion separately.
                for &byte in text {
                    writer.write_bits(u32::from(byte), 8)?;
                }
                writer.write_bits(0, 8)?;
            }
            let mut baseline = [0; states::ENTITY_WORDS];
            baseline[0] = 7;
            baseline[1] = 12.5f32.to_bits();
            writer.write_bits(4, 8)?;
            assert!(states::write_q3_entity(
                writer,
                7,
                &[0; states::ENTITY_WORDS],
                Some(&baseline),
                true,
            )?);
            writer.write_bits(8, 8)?;
            writer.write_bits(native_client as u32, 32)?;
            writer.write_bits(0, 32)?;
            Ok(())
        })
        .unwrap();
    let time = EventTime::from_milliseconds(100);
    channel
        .prepare_server_message(&bytes[..length], time)
        .unwrap()
        .unwrap();
    let mut packets = Vec::new();
    loop {
        packets.push(channel.pending_packet().unwrap().bytes.to_vec());
        channel.submitted(time).unwrap();
        if !channel.pending_fragments() {
            break;
        }
        channel.prepare_output(time).unwrap().unwrap();
    }
    packets
}

fn receive_gamestate_packets(host: &mut FrameHost, client: ClientId, packets: &[Vec<u8>]) {
    for packet in packets {
        host.runtime
            .loopback
            .send(Endpoint::Server, client, packet)
            .unwrap();
    }
    host.time = EventTime::from_milliseconds(100);
    host.drain(&mut FrameResult::default());
}

fn queued_gamestate_precedes_first_snapshot(factory: fn() -> (Program, u64)) {
    let (mut host, ids, output) = host(factory, 2);
    let packets = gamestate_packets(
        &mut host,
        ids[0],
        13,
        &[(20, b"BASEQ3-1"), (32, b"first model"), (200, b"\x80raw%")],
    );
    let channel = &host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel;
    assert!(channel.q3_startup().is_none());
    assert!(channel.current_snapshot().is_none());
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(20)
            .unwrap()
            .text,
        b"",
    );
    receive_gamestate_packets(&mut host, ids[0], &packets);
    let channel = &host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel;
    let startup = channel.q3_startup().unwrap();
    assert_eq!(startup.client_num, 13);
    assert_eq!(startup.last_executed_server_command, 0);
    assert!(channel.current_snapshot().is_none());
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        13
    );
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Ok(0));
    let offset = word(&mut host, 0, output + 20 * 4);
    assert_eq!(
        word(&mut host, 0, output + 4096 + u64::from(offset)),
        word_bytes(b"BASE")
    );
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(200)
            .unwrap()
            .text,
        b"\x80raw."
    );
    assert!(
        host.runtime
            .network
            .get(ids[1], Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup()
            .is_none()
    );
    assert_eq!(
        host.runtime.client_resources[1]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(20)
            .unwrap()
            .text,
        b""
    );

    let packets = gamestate_packets(&mut host, ids[1], 57, &[(20, b"other seat")]);
    receive_gamestate_packets(&mut host, ids[1], &packets);
    assert_eq!(
        host.runtime.snapshots[1].as_ref().unwrap().native_client(),
        57
    );
    assert_eq!(call(&mut host, 1, GET_GAME_STATE, &[output]), Ok(0));
    let offset = word(&mut host, 1, output + 20 * 4);
    assert_eq!(
        word(&mut host, 1, output + 4096 + u64::from(offset)),
        word_bytes(b"othe")
    );
    assert_eq!(
        host.runtime.prediction[0].player.movement_rules,
        RuleSetId::Quake
    );
    assert_eq!(
        host.runtime.prediction[1].player.trace_rules,
        RuleSetId::QuakeWorld
    );
}

fn word_bytes(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

fn rejected_gamestate_preserves_accepted_client_and_reuse_starts_empty() {
    let (mut host, ids, _) = host(qvm, 1);
    let packets = gamestate_packets(&mut host, ids[0], 13, &[(20, b"accepted"), (32, b"old")]);
    receive_gamestate_packets(&mut host, ids[0], &packets);
    let previous = host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel
        .q3_startup();
    let errors = host.runtime.network.command_errors;
    let packets = gamestate_packets(&mut host, ids[0], 64, &[(20, b"rejected")]);
    receive_gamestate_packets(&mut host, ids[0], &packets);
    assert_eq!(host.runtime.network.command_errors, errors + 1);
    assert_eq!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup(),
        previous
    );
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(20)
            .unwrap()
            .text,
        b"accepted"
    );
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        13
    );

    let packets = gamestate_packets(&mut host, ids[0], 15, &[(20, b"replacement")]);
    receive_gamestate_packets(&mut host, ids[0], &packets);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .unwrap()
            .text,
        b""
    );
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        15
    );
    assert!(host.runtime.disconnect_client(ids[0]));
    assert!(host.runtime.client_resources[0].is_none());
    assert!(host.runtime.network.get(ids[0], Endpoint::Client).is_none());
    complete_presentation(&mut host);
    assert_eq!(
        host.runtime
            .connect_local(
                SeatId::new(0).unwrap(),
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false
                },
                ClientPolicy {
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake2,
                    trace: RuleSetId::Quake
                },
                Protocol::Quake3_68,
                0,
            )
            .unwrap(),
        ids[0]
    );
    assert!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup()
            .is_none()
    );
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(20)
            .unwrap()
            .text,
        b""
    );
}

fn cgame_startup_waits_for_queued_state_and_resolves_current_arguments(
    factory: fn() -> (Program, u64),
) {
    for (argument, expected) in [
        (Argument::ClientMessageSequence, 2),
        (Argument::ClientExecutedCommand, 2),
        (Argument::ClientNativeNumber, 57),
    ] {
        let (mut host, ids, _) = host_with_startup(factory, 1, Some(argument));
        let module = ModuleId(1);
        assert_eq!(host.module_state(module), Some(State::Pending));
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_state(module), Some(State::Pending));
        assert_eq!(host.module_counts(module).unwrap().calls, 0);
        let packets = gamestate_packets(&mut host, ids[0], 64, &[(20, b"bad state")]);
        receive_gamestate_packets(&mut host, ids[0], &packets);
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_state(module), Some(State::Pending));
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        {
            let channel = &mut host
                .runtime
                .network
                .get_mut(ids[0], Endpoint::Server)
                .unwrap()
                .channel;
            channel.queue_reliable(b"print before").unwrap();
            channel.queue_reliable(b"print second").unwrap();
        }
        let packets = gamestate_packets(&mut host, ids[0], 57, &[(20, b"BASEQ3-1")]);
        receive_gamestate_packets(&mut host, ids[0], &packets);
        host.runtime
            .network
            .get_mut(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .executed_server_command(2)
            .unwrap();
        assert!(
            host.runtime
                .network
                .get(ids[0], Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
                .is_none()
        );
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_state(module), Some(State::Running));
        assert_eq!(host.module_counts(module).unwrap().calls, 1);
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        let actual = match host.module_counts(module).unwrap().last_result.unwrap() {
            ModuleResult::Qvm(value) => value as u64,
            ModuleResult::Native(value) => value,
            ModuleResult::QuakeC(_) => panic!("word module result"),
        };
        assert_eq!(actual, expected);
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_counts(module).unwrap().calls, 1);
    }
}

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    accepted_connection_snapshot_bytes(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    accepted_connection_snapshot_bytes(native);
    accepted_client_game_state(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    accepted_client_game_state(native);
    game_state_pointer_and_aggregate_limits();
    qvm_pointer_checks_and_retained_window();
    snapshot_entity_count_is_truncated_to_the_native_public_array();
    ui_keeps_its_queued_clock();
    queued_gamestate_precedes_first_snapshot(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    queued_gamestate_precedes_first_snapshot(native);
    rejected_gamestate_preserves_accepted_client_and_reuse_starts_empty();
    cgame_startup_waits_for_queued_state_and_resolves_current_arguments(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    cgame_startup_waits_for_queued_state_and_resolves_current_arguments(native);
    println!("CLIENT snapshot getter fixtures passed");
}
