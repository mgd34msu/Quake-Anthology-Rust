#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::native_image;
use super::service_program;
use qa_app::{Runtime, host::FrameSource};
use qa_compat::{
    abi::{Q3_SERVER, QvmCalls, UnknownCalls},
    qvm::Trap,
    services::{CallContext, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::{FrameEvent, OutputSubmission},
    loopback::Endpoint,
    primitives::{ClientId, ModuleId, NativeEntity, RuleSetId, ThinkTime, UserCmd},
    sys_events::{EventKind, EventTime, Peer, SeatId, SysEvent, SysEventQueue},
};
use qa_formats::program::qvm::Opcode;
use qa_network::{
    channel::{Channel, Delivery},
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as NativeConnection, Incoming, Route},
};
use qa_session::clients::Connection;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};
use std::{ops::ControlFlow, time::Duration};

const MODULE: ModuleId = ModuleId(7);
const OTHER_MODULE: ModuleId = ModuleId(9);

fn context() -> CallContext {
    CallContext {
        module: MODULE,
        clock: ThinkTime::Milliseconds(17),
        server_frame: 1,
        native_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake3,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
        console: Context {
            source: RuleSetId::Quake3,
            ..Context::default()
        },
        allocation: AllocationPolicy::q3(0),
        link_order: LinkOrder::Head,
    }
}

fn storage(console: &Console<Runtime>) -> ServiceStorage {
    let mut storage =
        ServiceStorage::load_native(&[(MODULE, 0, RuleSetId::Quake3)], 0, &console.cvars).unwrap();
    storage.bind_client_capacity(MODULE, 16).unwrap();
    storage
}

fn connect_without_channel(runtime: &mut Runtime, module: ModuleId, native_slot: i32) -> ClientId {
    runtime
        .server
        .connect(
            Connection::Remote,
            module,
            Some(NativeEntity {
                module,
                slot: native_slot,
            }),
        )
        .unwrap()
}

fn qvm_send(
    runtime: &mut Runtime,
    console: &mut Console<Runtime>,
    storage: &mut ServiceStorage,
    target: i32,
    text: Option<&[u8]>,
) -> Result<i32, Trap> {
    use Opcode::*;
    let pointer = if text.is_some() { 128 } else { 0 };
    let operations = [
        (Enter, 64),
        (Const, target),
        (Arg, 8),
        (Const, pointer),
        (Arg, 12),
        // QVM negative call targets encode -(system-call ordinal + 1).
        (Const, -18),
        (Call, 0),
        (Leave, 64),
    ];
    let data_bytes = 128 + text.map_or(0, |text| text.len() + 1);
    let mut data = vec![0; (data_bytes + 3) & !3];
    if let Some(text) = text {
        data[128..128 + text.len()].copy_from_slice(text);
    }
    let mut vm = service_program::program(&operations, &data);
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let result = {
        let mut services = runtime.engine_services(console, storage, &mut scratch);
        vm.call(
            &mut QvmCalls {
                services: &mut services,
                table: &Q3_SERVER,
                context: context(),
                platform_time: EventTime(0),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                unknown: &mut unknown,
            },
            [0; 10],
            100,
            false,
        )
    };
    assert_eq!(unknown.calls, 0);
    result
}

fn command(runtime: &Runtime, client: ClientId) -> Option<(u64, FrameEvent, Vec<u8>)> {
    let output = runtime.server.clients[client.0 as usize].output.unwrap();
    let mut batch = runtime.server.events.batch(output).unwrap();
    let record = runtime.server.events.next(&mut batch)?;
    let FrameEvent::ServerCommand { text, .. } = record.event else {
        panic!("server command");
    };
    Some((
        record.sequence,
        record.event,
        runtime.server.events.texts.get(text).unwrap().to_vec(),
    ))
}

fn finish(runtime: &mut Runtime, client: ClientId, sequence: u64) {
    let output = runtime.server.clients[client.0 as usize].output.unwrap();
    assert!(
        runtime
            .server
            .events
            .submit(output, sequence, OutputSubmission::BestEffort)
    );
}

pub fn qvm_import_uses_module_native_clients_and_source_string_storage() {
    let mut runtime = Runtime::load(3, []).unwrap();
    let same_number_other_module = connect_without_channel(&mut runtime, OTHER_MODULE, 13);
    let target = connect_without_channel(&mut runtime, MODULE, 13);
    let other = connect_without_channel(&mut runtime, MODULE, 0);
    assert_ne!(target.0, 13);
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = storage(&console);

    // The original boundary silently ignores an invalid native client number
    // before touching its command pointer.
    assert_eq!(
        qvm_send(&mut runtime, &mut console, &mut storage, -2, None),
        Ok(0)
    );
    assert_eq!(
        qvm_send(&mut runtime, &mut console, &mut storage, 16, None),
        Ok(0)
    );
    assert!(command(&runtime, same_number_other_module).is_none());
    assert!(command(&runtime, target).is_none());
    assert!(command(&runtime, other).is_none());

    let source = vec![b'x'; 1030];
    assert_eq!(
        qvm_send(&mut runtime, &mut console, &mut storage, 13, Some(&source),),
        Ok(0)
    );
    assert!(command(&runtime, same_number_other_module).is_none());
    assert!(command(&runtime, other).is_none());
    let (sequence, event, text) = command(&runtime, target).unwrap();
    assert_eq!(text, vec![b'x'; 1023]);
    assert!(matches!(
        event,
        FrameEvent::ServerCommand {
            module: MODULE,
            source: RuleSetId::Quake3,
            ..
        }
    ));
    finish(&mut runtime, target, sequence);
    assert!(runtime.server.events.is_empty());

    assert_eq!(
        qvm_send(
            &mut runtime,
            &mut console,
            &mut storage,
            -1,
            Some(b"print \"broadcast\""),
        ),
        Ok(0)
    );
    for client in [same_number_other_module, target, other] {
        let (sequence, _, text) = command(&runtime, client).unwrap();
        assert_eq!(text, b"print \"broadcast\"");
        finish(&mut runtime, client, sequence);
    }
    assert!(runtime.server.events.is_empty());
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn owned_native_import_uses_the_same_service_and_integer_width() {
    use native_image::*;
    use qa_compat::native::{NamedExport, NativeCalls, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;

    let mut runtime = Runtime::load(2, []).unwrap();
    let wrong_module = connect_without_channel(&mut runtime, OTHER_MODULE, 13);
    let target = connect_without_channel(&mut runtime, MODULE, 13);
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = storage(&console);

    let mut file = pe_export_fixture(64);
    put(&mut file, 208, 12288, 4);
    put(&mut file, 392 + 8, 8192, 4);
    put(&mut file, 392 + 16, 8192, 4);
    pe_rva(&mut file, 0x1140, 0x1200, 4);
    file.resize(8704, 0);
    pe_text(&mut file, 0x1080, &[0xb8, 1, 0, 0, 0, 0xc3]);
    pe_text(&mut file, 0x1200, &[0x31, 0xc0, 0xc3]);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let base = image.base;
    let mut vm = Vm::map_image(
        image,
        &[NamedExport {
            name: b"GetGameAPI",
            command: Some(0),
            parameters: &[NativeScalar::Word],
            result: NativeScalar::Word,
        }],
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    let mut code = vec![0x48, 0x83, 0xec, 0x28, 0xb9, 17, 0, 0, 0, 0x48, 0xba];
    code.extend(0x1234_5678_0000_000du64.to_le_bytes());
    code.extend([0x49, 0xb8]);
    code.extend((base + 0x1800).to_le_bytes());
    code.extend([0x48, 0xb8]);
    code.extend(vm.import_callback().to_le_bytes());
    code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
    {
        let mut memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
        memory.write(base + 0x1200, &code).unwrap();
        memory
            .write(base + 0x1800, b"cp \"owned child\"\0")
            .unwrap();
    }
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let result = vm
            .call(
                &mut NativeCalls {
                    services: &mut services,
                    table: &Q3_SERVER,
                    context: context(),
                    platform_time: EventTime(0),
                    command: Default::default(),
                    argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                    unknown: &mut unknown,
                    native_player: None,
                },
                0,
                &[],
            )
            .unwrap();
        assert_eq!(result, 0);
    }
    assert_eq!(unknown.calls, 0);
    assert!(command(&runtime, wrong_module).is_none());
    let (sequence, _, text) = command(&runtime, target).unwrap();
    assert_eq!(text, b"cp \"owned child\"");
    finish(&mut runtime, target, sequence);
}

#[derive(Default)]
struct Sink {
    packets: Vec<(u16, Vec<u8>)>,
    now: EventTime,
}

impl FrameSource for Sink {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.now.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.now,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_nanos(self.now.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn send_packet(&mut self, _: u16, to: std::net::SocketAddr, bytes: &[u8]) -> bool {
        self.packets.push((to.port(), bytes.to_vec()));
        true
    }
}

fn remote(
    runtime: &mut Runtime,
    module: ModuleId,
    native_slot: i32,
    protocol: Protocol,
    port: u16,
) -> ClientId {
    let id = runtime
        .server
        .connect(Connection::Remote, module, None)
        .unwrap();
    assert!(runtime.server.bind_client_native(
        id,
        NativeEntity {
            module,
            slot: native_slot,
        }
    ));
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
    channel.set_qport(port);
    runtime
        .network
        .bind(
            id,
            Endpoint::Server,
            NativeConnection {
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: 0,
                    peer: Peer::Socket(([127, 0, 0, 1], port).into()),
                },
                channel,
                output: runtime.server.clients[id.0 as usize].output,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
        )
        .unwrap();
    id
}

fn dispatch(runtime: &mut Runtime, sink: &mut Sink) -> qa_app::output::OutputCounts {
    let mut scrollback = qa_console::presentation::Scrollback::default();
    let counts = qa_app::output::dispatch(
        runtime,
        sink,
        &mut scrollback,
        &[None; SeatId::COUNT],
        sink.now,
        3.0,
        2.0,
    );
    scrollback.clear(&mut runtime.server.events.texts);
    counts
}

fn receive_commands(peer: &mut Channel, sink: &Sink, port: u16) -> Vec<Vec<u8>> {
    let mut commands = Vec::new();
    for (_, bytes) in sink.packets.iter().filter(|(to, _)| *to == port) {
        let received = peer.receive(bytes, EventTime(20)).unwrap();
        let sequence = received.header.sequence;
        if let Delivery::Payload(body) = received.delivery {
            let mut body = body.to_vec();
            peer.decode_server_output(&mut body, sequence, |message| {
                if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                    text, ..
                } = message
                {
                    commands.push(text.to_vec());
                }
            })
            .unwrap();
        }
    }
    commands
}

fn acknowledge(runtime: &mut Runtime, peer: &mut Channel, client: ClientId, port: u16) -> usize {
    let time = peer.send_state().sequence.wrapping_add(1) as i32 * 16;
    let command = UserCmd {
        server_time_ms: time,
        duration_ms: 16,
        ..UserCmd::default()
    };
    let mut payload = vec![0; peer.native_payload_capacity().unwrap()];
    let encoder = Commands::load(Protocol::Quake3_68, payload.len());
    let length = encoder.encode(&command, |_| 0, peer, &mut payload).unwrap();
    let packet = peer
        .prepare_move(&payload[..length], EventTime(time as u64), None, None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    peer.submitted(EventTime(time as u64)).unwrap();
    let output = runtime.server.clients[client.0 as usize].output.unwrap();
    let events = &mut runtime.server.events;
    let mut retired = 0;
    runtime.network.receive(
        0,
        Peer::Socket(([127, 0, 0, 1], port).into()),
        &packet,
        EventTime(time as u64 + 1),
        |id, endpoint, incoming| {
            assert_eq!((id, endpoint), (client, Endpoint::Server));
            if let Incoming::Acknowledged {
                receipt,
                output: Some(owner),
            } = incoming
            {
                assert_eq!(owner, output);
                retired += events.acknowledge(owner, receipt);
            }
            ControlFlow::Continue(())
        },
    );
    retired
}

pub fn broadcast_backpressure_uses_each_real_ack_and_drops_foreign_protocols() {
    let mut runtime = Runtime::load(3, []).unwrap();
    let healthy = remote(&mut runtime, MODULE, 0, Protocol::Quake3_68, 2000);
    let stalled = remote(&mut runtime, MODULE, 1, Protocol::Quake3_68, 2001);
    let foreign = remote(&mut runtime, MODULE, 2, Protocol::Quake2_34, 2002);
    let healthy_output = runtime.server.clients[healthy.0 as usize].output.unwrap();
    let stalled_output = runtime.server.clients[stalled.0 as usize].output.unwrap();
    let foreign_output = runtime.server.clients[foreign.0 as usize].output.unwrap();
    for _ in 0..64 {
        runtime
            .network
            .get_mut(stalled, Endpoint::Server)
            .unwrap()
            .channel
            .queue_reliable(b"seed")
            .unwrap();
    }
    let sequence = runtime
        .server
        .events
        .server_command(None, MODULE, RuleSetId::Quake3, b"cp \"ready\"")
        .unwrap();
    let mut sink = Sink {
        now: EventTime(10),
        ..Sink::default()
    };
    let counts = dispatch(&mut runtime, &mut sink);
    assert_eq!(counts.native_unsupported, 1);
    assert!(runtime.server.events.retained_for(healthy_output, sequence));
    assert!(runtime.server.events.retained_for(stalled_output, sequence));
    assert!(!runtime.server.events.retained_for(foreign_output, sequence));

    let mut healthy_peer =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 16).unwrap();
    healthy_peer.set_qport(2000);
    let mut stalled_peer =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 16).unwrap();
    stalled_peer.set_qport(2001);
    assert_eq!(
        receive_commands(&mut healthy_peer, &sink, 2000),
        [b"cp \"ready\"".to_vec()]
    );
    assert_eq!(receive_commands(&mut stalled_peer, &sink, 2001).len(), 64);
    assert_eq!(
        acknowledge(&mut runtime, &mut healthy_peer, healthy, 2000),
        1
    );
    assert_eq!(
        acknowledge(&mut runtime, &mut stalled_peer, stalled, 2001),
        0
    );
    assert!(!runtime.server.events.retained_for(healthy_output, sequence));
    assert!(runtime.server.events.retained_for(stalled_output, sequence));

    sink.now = EventTime(60);
    sink.packets.clear();
    assert_eq!(dispatch(&mut runtime, &mut sink).native_unsupported, 0);
    assert_eq!(
        receive_commands(&mut stalled_peer, &sink, 2001),
        [b"cp \"ready\"".to_vec()]
    );
    assert!(receive_commands(&mut healthy_peer, &sink, 2000).is_empty());
    assert_eq!(
        acknowledge(&mut runtime, &mut stalled_peer, stalled, 2001),
        1
    );
    assert!(runtime.server.events.is_empty());
}

pub fn command_width_and_reconnect_boundaries_do_not_stall_later_commands() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = remote(&mut runtime, MODULE, 0, Protocol::Quake3_68, 2100);
    let output = runtime.server.clients[client.0 as usize].output.unwrap();
    let accepted = runtime
        .server
        .events
        .server_command(Some(client), MODULE, RuleSetId::Quake3, &vec![b'a'; 1022])
        .unwrap();
    let rejected = runtime
        .server
        .events
        .server_command(Some(client), MODULE, RuleSetId::Quake3, &vec![b'b'; 1023])
        .unwrap();
    let after = runtime
        .server
        .events
        .server_command(Some(client), MODULE, RuleSetId::Quake3, b"print \"after\"")
        .unwrap();
    let old_text = {
        let mut batch = runtime.server.events.batch(output).unwrap();
        let record = runtime.server.events.next(&mut batch).unwrap();
        let FrameEvent::ServerCommand { text, .. } = record.event else {
            panic!("server command");
        };
        text
    };
    let mut sink = Sink {
        now: EventTime(10),
        ..Sink::default()
    };
    assert_eq!(dispatch(&mut runtime, &mut sink).native_unsupported, 1);
    assert!(runtime.server.events.retained_for(output, accepted));
    assert!(!runtime.server.events.retained_for(output, rejected));
    assert!(runtime.server.events.retained_for(output, after));
    assert_eq!(
        runtime
            .network
            .get(client, Endpoint::Server)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .queued,
        2
    );

    assert!(runtime.disconnect_client(client));
    assert!(runtime.server.events.is_empty());
    assert!(runtime.server.events.texts.get(old_text).is_none());

    let replacement = remote(&mut runtime, MODULE, 0, Protocol::Quake3_68, 2101);
    assert_eq!(replacement, client);
    sink.now = EventTime(30);
    sink.packets.clear();
    assert_eq!(dispatch(&mut runtime, &mut sink).native_unsupported, 0);
    assert_eq!(
        runtime
            .network
            .get(replacement, Endpoint::Server)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .queued,
        0
    );
    runtime
        .server
        .events
        .server_command(
            Some(replacement),
            MODULE,
            RuleSetId::Quake3,
            b"print \"new lifetime\"",
        )
        .unwrap();
    dispatch(&mut runtime, &mut sink);
    assert_eq!(
        runtime
            .network
            .get(replacement, Endpoint::Server)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .queued,
        1
    );
}
