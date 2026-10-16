#[path = "../../compat/tests/support/quakec_program.rs"]
mod quakec_program;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
    modules::{Export, ModuleRequest, Phase, Program, State},
};
use qa_compat::{
    quakec::{Layout, Vm},
    services::CallContext,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{configstrings::ConfigLayout, loopback::Endpoint, primitives::*, sys_events::*};
use qa_formats::program::quakec::{Image, Opcode::*};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as NativeConnection, Route},
};
use qa_session::{clients::Connection, timing::TickRate};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};
use std::{num::NonZeroU32, time::Duration};

fn module(runtime: &mut Runtime, id: ModuleId, x: f32, rate: u32) -> ModuleRequest {
    let operations = [
        (StoreS, 60, 4, 0),
        (Call1, 61, 0, 0),
        (StoreV, 64, 4, 0),
        (StoreS, 60, 7, 0),
        (StoreF, 67, 10, 0),
        (StoreF, 68, 13, 0),
        (Call4, 62, 0, 0),
        (Return, 1, 0, 0),
        (Return, 1, 0, 0),
        (StoreV, 70, 4, 0),
        (StoreS, 60, 7, 0),
        (StoreF, 67, 10, 0),
        (StoreF, 68, 13, 0),
        (Call4, 62, 0, 0),
        (Return, 1, 0, 0),
    ];
    let bytes = quakec_program::program_with_definitions(
        &operations,
        &[0, -19, -74, 1, 9, 10],
        &[
            (4, 34, b"self"),
            (2, 35, b"time"),
            (4, 36, b"other"),
            (2, 37, b"frametime"),
        ],
        &[(2, 0, b"nextthink"), (2, 1, b"frame"), (6, 2, b"think")],
        8,
        128,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    vm.image.globals[60] = vm.insert_string(b"loop.wav").unwrap() as u32;
    vm.image.globals[61] = 1;
    vm.image.globals[62] = 2;
    vm.image.globals[64..67].copy_from_slice(&[x, 0.0, 0.0].map(f32::to_bits));
    vm.image.globals[67] = 1.0f32.to_bits();
    vm.image.globals[68] = 1.0f32.to_bits();
    vm.image.globals[70..73].copy_from_slice(&[x + 1.0, 0.0, 0.0].map(f32::to_bits));
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.0), id, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[anchor.slot as usize] = Some(NativeEntity {
        module: id,
        slot: 0,
    });
    ModuleRequest {
        context: CallContext {
            module: id,
            clock: ThinkTime::Seconds(0.0),
            server_frame: 0,
            native_rules: RuleSetId::QuakeWorld,
            trace_rules: RuleSetId::Quake2,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake),
            console: Context::default(),
            allocation: AllocationPolicy::EDICT,
            link_order: LinkOrder::Tail,
        },
        phase: Phase::Server,
        timing_rules: RuleSetId::QuakeWorld,
        rate: TickRate::fixed(rate).unwrap(),
        anchor,
        program: Program::quakec(vm, None),
        entries: vec![3, 4, 5],
        frame: Export::clocked(CallbackId(1)),
        prepare: vec![],
        initialize: vec![Export::clocked(CallbackId(0))],
        api: None,
        shutdown: vec![],
        instruction_budget: 1000,
        configstrings: ConfigLayout::for_rules(RuleSetId::QuakeWorld).count,
        files: 0,
        entity_source: None,
    }
}

fn host(clients: usize) -> FrameHost {
    let mut runtime = Runtime::load(clients, []).unwrap();
    let requests = vec![
        module(&mut runtime, ModuleId(2), 8.0, 20),
        module(&mut runtime, ModuleId(101), 16.0, 40),
    ];
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.initialize_modules(Phase::Server);
    for module in [ModuleId(2), ModuleId(101)] {
        assert_eq!(host.module_state(module), Some(State::Running));
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
    let rate = NonZeroU32::new(44100).unwrap();
    let bank = qa_audio::Bank::load(
        rate,
        vec![(
            "sound/loop.wav".into(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: vec![1000, -1000],
                loop_start: Some(0),
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    host.runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
    host.bind_sounds().unwrap();
    assert!(host.runtime.server.events.is_empty());
    host
}

fn connect(host: &mut FrameHost, seat: SeatId, native_client: u32) -> ClientId {
    let client = host
        .runtime
        .connect_local(
            seat,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake2,
                trace: RuleSetId::Quake,
            },
            Protocol::QuakeWorld28,
            native_client,
        )
        .unwrap();
    assert!(host.runtime.server.bind_client_native(
        client,
        NativeEntity {
            module: ModuleId(2),
            slot: native_client as i32 + 1
        }
    ));
    host.local_clients[seat.index()] = Some(client);
    client
}

#[derive(Default)]
struct Source {
    time: EventTime,
    positions: Vec<(ClientId, f32)>,
    retired: Vec<(ClientId, EntityId)>,
    mixer: Option<qa_audio::Mixer>,
    blocked_packets: usize,
}
impl FrameSource for Source {
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
        panic!("uncapped fixture")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, bank: Option<&qa_audio::Bank>, sound: SoundEvent) -> bool {
        if sound.action == SoundAction::RetireClient {
            self.retired
                .push((sound.client.unwrap(), sound.client_lifetime.unwrap()));
            return self
                .mixer
                .as_mut()
                .is_none_or(|mixer| mixer.sound(bank.unwrap(), sound));
        }
        assert_eq!(sound.action, SoundAction::StartLoop);
        assert!(sound.client_lifetime.is_some());
        assert!(matches!(
            sound.entity,
            Some(SoundEmitter::Static {
                namespace: ResourceNamespace::Client(_),
                ..
            })
        ));
        self.positions
            .push((sound.client.unwrap(), sound.position.0[0]));
        self.mixer
            .as_mut()
            .is_none_or(|mixer| mixer.sound(bank.unwrap(), sound))
    }
    fn send_packet(&mut self, _: u16, _: std::net::SocketAddr, _: &[u8]) -> bool {
        self.blocked_packets += 1;
        false
    }
}
fn frames(host: &mut FrameHost, source: &mut Source, from: u64, count: u64) {
    for frame in from..from + count {
        source.time = EventTime::from_milliseconds(frame * 25);
        let result = host.frame(source, true);
        assert_eq!(result.output.native_unsupported, 0);
    }
}
fn positions(source: &Source, client: ClientId) -> Vec<f32> {
    source
        .positions
        .iter()
        .filter(|(id, _)| *id == client)
        .map(|(_, position)| *position)
        .collect()
}

fn late_connections_and_reused_client_slots_receive_each_module_signon_once() {
    let mut host = host(2);
    let first = connect(&mut host, SeatId::FIRST, 0);
    host.bind_sounds().unwrap();
    let first_output = host.runtime.server.clients[first.0 as usize]
        .output
        .unwrap();
    let old_entity = host.runtime.server.clients[first.0 as usize].entity;
    let sound = host
        .runtime
        .sound_bank()
        .unwrap()
        .find("sound/loop.wav")
        .unwrap();
    // Signon loops precede their configstring admission. A resource wait must
    // let those later configstrings through, without advancing later services.
    assert_eq!(
        host.runtime.client_resources[first.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .sound(1),
        None,
    );
    let mut source = Source::default();
    frames(&mut host, &mut source, 0, 16);
    assert_eq!(
        host.runtime.client_resources[first.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .sound(1),
        Some(sound),
    );
    assert_eq!(positions(&source, first), [8.0, 16.0]);
    assert!(
        host.runtime
            .server
            .events
            .counters(first_output)
            .unwrap()
            .acknowledged_records
            >= 2
    );
    host.call_module_export(
        ModuleId(2),
        Export::words(CallbackId(2), &[]),
        ThinkTime::Seconds(1.0),
    )
    .unwrap();
    frames(&mut host, &mut source, 16, 8);
    assert_eq!(positions(&source, first), [8.0, 16.0]);

    let second = connect(&mut host, SeatId::new(1).unwrap(), 1);
    host.bind_sounds().unwrap();
    frames(&mut host, &mut source, 24, 16);
    assert_eq!(positions(&source, first), [8.0, 16.0]);
    assert_eq!(positions(&source, second), [8.0, 9.0, 16.0]);

    // Deliver the native disconnect through the current channel and sole queue.
    let channel = &mut host
        .runtime
        .network
        .get_mut(first, Endpoint::Client)
        .unwrap()
        .channel;
    channel.queue_reliable(b"\x04disconnect\0").unwrap();
    frames(&mut host, &mut source, 40, 4);
    assert!(
        host.runtime.server.clients[first.0 as usize]
            .connection
            .is_none()
    );
    let replacement = connect(&mut host, SeatId::FIRST, 0);
    assert_eq!(replacement, first);
    assert_ne!(
        host.runtime.server.clients[first.0 as usize].entity,
        old_entity
    );
    host.bind_sounds().unwrap();
    frames(&mut host, &mut source, 44, 16);
    assert_eq!(positions(&source, first), [8.0, 16.0, 8.0, 9.0, 16.0]);
    assert_eq!(positions(&source, second), [8.0, 9.0, 16.0]);
    for module in [ModuleId(2), ModuleId(101)] {
        let counts = host.module_counts(module).unwrap();
        assert!(counts.calls > 10);
        assert_eq!(counts.traps, 0);
        assert_eq!(counts.rejected, 0);
    }
}

fn blocked_native_peer_keeps_its_own_receipts_while_healthy_signon_and_server_ticks_continue() {
    let mut host = host(2);
    let stalled = host
        .runtime
        .server
        .connect(Connection::Remote, ModuleId(2), None)
        .unwrap();
    let protocol = Protocol::QuakeWorld28;
    host.runtime
        .bind_server_connection(
            stalled,
            NativeConnection {
                route: Route {
                    socket: 19,
                    peer: Peer::Socket("127.0.0.1:27511".parse().unwrap()),
                },
                channel: Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap(),
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
            0,
            None,
        )
        .unwrap();
    let output = host.runtime.server.clients[stalled.0 as usize]
        .output
        .unwrap();
    // A QW SERVER replies only after accepting a native request. The peer
    // admits its initial request but never ACKs the resulting blocked flight.
    let mut peer = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
    let request = peer.prepare(None, EventTime(0)).unwrap().unwrap();
    host.queue
        .push(SysEvent {
            time: EventTime(0),
            kind: EventKind::Packet {
                socket: 19,
                from: Peer::Socket("127.0.0.1:27511".parse().unwrap()),
                bytes: request.bytes,
            },
        })
        .unwrap();
    peer.submitted(EventTime(0)).unwrap();
    let healthy = connect(&mut host, SeatId::FIRST, 1);
    host.bind_sounds().unwrap();
    let mut source = Source::default();
    frames(&mut host, &mut source, 0, 64);
    assert!(source.blocked_packets > 0);
    assert_eq!(positions(&source, healthy), [8.0, 16.0]);
    assert!(positions(&source, stalled).is_empty());
    assert_eq!(
        host.runtime
            .server
            .events
            .counters(output)
            .unwrap()
            .acknowledged_records,
        0
    );
    let mut batch = host.runtime.server.events.batch(output).unwrap();
    assert!(host.runtime.server.events.next(&mut batch).is_some());
    assert_eq!(
        host.runtime.server.clients[stalled.0 as usize].connection,
        Some(Connection::Remote)
    );
    for module in [ModuleId(2), ModuleId(101)] {
        assert!(host.module_counts(module).unwrap().calls > 10);
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
}

fn decoded_static_loops_stop_at_disconnect_while_other_clients_and_ticks_continue() {
    let mut host = host(2);
    let first = connect(&mut host, SeatId::FIRST, 0);
    let healthy = connect(&mut host, SeatId::new(1).unwrap(), 1);
    let old = host.runtime.server.clients[first.0 as usize].entity;
    let mut mixer = qa_audio::Mixer::load(host.runtime.sound_bank().unwrap().rate, 16, 1);
    let listen = |client| {
        Some(qa_audio::Listener {
            negate_dot: false,
            client,
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1.0, 0.0, 0.0]),
        })
    };
    mixer.listen(&[listen(first)], 1.0);
    let mut source = Source {
        mixer: Some(mixer),
        ..Source::default()
    };
    frames(&mut host, &mut source, 0, 16);
    let mut pcm = [0; 8];
    assert!(
        source
            .mixer
            .as_mut()
            .unwrap()
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert!(pcm.iter().any(|sample| *sample != 0));
    let before = host.module_counts(ModuleId(2)).unwrap().calls;
    host.runtime
        .network
        .get_mut(first, Endpoint::Client)
        .unwrap()
        .channel
        .queue_reliable(b"\x04disconnect\0")
        .unwrap();
    frames(&mut host, &mut source, 16, 8);
    assert!(
        host.runtime.server.clients[first.0 as usize]
            .connection
            .is_none()
    );
    assert_eq!(source.retired, [(first, old)]);
    assert!(
        host.runtime.server.clients[first.0 as usize]
            .audio_retirement
            .is_none()
    );
    assert!(
        source
            .mixer
            .as_mut()
            .unwrap()
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert_eq!(pcm, [0; 8]);
    source
        .mixer
        .as_mut()
        .unwrap()
        .listen(&[listen(healthy)], 1.0);
    assert!(
        source
            .mixer
            .as_mut()
            .unwrap()
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert!(pcm.iter().any(|sample| *sample != 0));
    assert_eq!(
        host.runtime.server.clients[healthy.0 as usize].connection,
        Some(Connection::Local)
    );
    assert!(host.module_counts(ModuleId(2)).unwrap().calls > before);
}

fn stopping_secondary_module_retires_primary_recipient_without_stopping_other_provider() {
    let mut host = host(1);
    let client = connect(&mut host, SeatId::FIRST, 0);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].module,
        ModuleId(2)
    );
    let lifetime = host.runtime.server.clients[client.0 as usize].entity;
    let mut mixer = qa_audio::Mixer::load(host.runtime.sound_bank().unwrap().rate, 8, 1);
    mixer.listen(
        &[Some(qa_audio::Listener {
            negate_dot: false,
            client,
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1.0, 0.0, 0.0]),
        })],
        1.0,
    );
    let mut source = Source {
        mixer: Some(mixer),
        ..Source::default()
    };
    frames(&mut host, &mut source, 0, 16);
    assert_eq!(positions(&source, client), [8.0, 16.0]);
    let before = host.module_counts(ModuleId(2)).unwrap().calls;
    assert!(host.shutdown_module(ModuleId(101)));
    assert!(!host.shutdown_module(ModuleId(101)));
    assert_eq!(host.module_state(ModuleId(101)), Some(State::Stopped));
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_none()
    );
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .audio_retirement
            .is_some()
    );
    frames(&mut host, &mut source, 16, 8);
    assert_eq!(source.retired, [(client, lifetime)]);
    let mut pcm = [1; 8];
    assert!(
        source
            .mixer
            .as_mut()
            .unwrap()
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert_eq!(pcm, [0; 8]);
    assert!(host.module_counts(ModuleId(2)).unwrap().calls > before);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    // A fresh connection has a new canonical lifetime; the stopped module is
    // not replayed, while the surviving module still publishes its signon.
    let fresh = connect(&mut host, SeatId::FIRST, 0);
    assert_ne!(
        host.runtime.server.clients[fresh.0 as usize].entity,
        lifetime
    );
    frames(&mut host, &mut source, 24, 16);
    assert_eq!(positions(&source, client), [8.0, 16.0, 8.0]);
    assert!(
        source
            .mixer
            .as_mut()
            .unwrap()
            .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
    );
    assert!(pcm.iter().any(|sample| *sample != 0));
}

fn main() {
    late_connections_and_reused_client_slots_receive_each_module_signon_once();
    blocked_native_peer_keeps_its_own_receipts_while_healthy_signon_and_server_ticks_continue();
    decoded_static_loops_stop_at_disconnect_while_other_clients_and_ticks_continue();
    stopping_secondary_module_retires_primary_recipient_without_stopping_other_provider();
    println!("per-client module signon checks passed");
}
