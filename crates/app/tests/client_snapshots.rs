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
    modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
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

fn qvm() -> (Program, u64) {
    use Opcode::*;
    let mut operations = vec![(Enter, 64)];
    let mut branches = Vec::new();
    for command in 0..7 {
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
        &(0..7)
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
    for command in 0..7u8 {
        code.extend([0x83, 0xf9, command, 0x0f, 0x85]);
        let branch = code.len();
        code.extend([0; 4]);
        if command == 3 {
            code.extend([0x8b, 0x02, 0xc3]); // mov eax,[rdx]; ret
        } else if command == 6 {
            code.extend([0x89, 0xd0, 0xc3]); // mov eax,edx; ret
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
            entries: (0..7).collect(),
            frame: Export {
                client: Some(client),
                ..Export::clocked(CallbackId(6))
            },
            prepare: vec![],
            initialize: vec![],
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

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    accepted_connection_snapshot_bytes(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    accepted_connection_snapshot_bytes(native);
    qvm_pointer_checks_and_retained_window();
    snapshot_entity_count_is_truncated_to_the_native_public_array();
    ui_keeps_its_queued_clock();
    println!("CLIENT snapshot getter fixtures passed");
}
