use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, RuleSetId, ThinkTime},
    sys_events::{EventKind, EventTime, Peer, SeatId, SysEvent, SysEventQueue},
};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection, Route},
    message::{Encoding, Writer},
    snapshots::{Frame, ReceivedFrame},
    states,
};
use qa_session::timing::TickRate;
use std::time::Duration;

#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;

#[derive(Default)]
struct Source {
    time_ms: u64,
    polls: usize,
    packets: Vec<(ClientId, Vec<u8>)>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(self.time_ms)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.polls += 1;
        let time = EventTime::from_milliseconds(self.time_ms);
        for (client, bytes) in self.packets.drain(..) {
            queue
                .push(SysEvent {
                    time,
                    kind: EventKind::Packet {
                        socket: Endpoint::Client.socket(),
                        from: Peer::Loopback(client),
                        bytes: &bytes,
                    },
                })
                .unwrap();
        }
        queue
            .push(SysEvent {
                time,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_millis(self.time_ms)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

fn connect(
    runtime: &mut Runtime,
    seat: SeatId,
    protocol: Protocol,
    client: RuleSetId,
    movement: RuleSetId,
) -> ClientId {
    let id = runtime
        .connect_local(
            seat,
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: client,
                client,
                movement,
                trace: RuleSetId::QuakeWorld,
            },
            protocol,
            0,
        )
        .unwrap();
    // The fixture controls remote SERVER snapshots and packet loss. Keep the
    // normal local-seat input path, but omit a second in-process SERVER sender.
    runtime.network.unbind(id);
    runtime
        .network
        .bind(
            id,
            Endpoint::Client,
            Connection {
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(id),
                },
                channel: Channel::load(protocol.channel(), Endpoint::Client, 32768, 16).unwrap(),
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 32768)),
            },
        )
        .unwrap();
    id
}

fn packet(protocol: Protocol, time_ms: i64) -> Vec<u8> {
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 32768, 16).unwrap();
    channel.configure_snapshots(protocol).unwrap();
    packet_from(&mut channel, protocol, time_ms)
}

fn packet_from(channel: &mut Channel, protocol: Protocol, time_ms: i64) -> Vec<u8> {
    packet_with_flags(channel, protocol, time_ms, 0)
}

fn packet_with_flags(
    channel: &mut Channel,
    protocol: Protocol,
    time_ms: i64,
    flags: u8,
) -> Vec<u8> {
    let mut bytes = [0; 32768];
    let time = ThinkTime::Milliseconds(time_ms);
    let q3_player = [0; states::PLAYER_WORDS];
    let q2_player = [0; states::Q2_PLAYER_WORDS];
    let current = match protocol {
        Protocol::Quake3_68 => ReceivedFrame::Quake3(Frame {
            sequence: channel.send_state().sequence,
            time,
            command: 0,
            flags,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &q3_player,
            entities: &[],
        }),
        Protocol::Quake2_34 => ReceivedFrame::Quake2(Frame {
            sequence: (time_ms / 100) as u32,
            time,
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &q2_player,
            entities: &[],
        }),
        _ => panic!("unsupported clock fixture protocol"),
    };
    let sequence = current.sequence();
    channel.publish_written_snapshot(current, None).unwrap();
    let length = if protocol == Protocol::Quake3_68 {
        channel
            .encode_server_output(&mut bytes, |writer, channel| {
                channel.write_snapshot(writer, sequence, None, 0)
            })
            .unwrap()
    } else {
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        channel
            .write_snapshot(&mut writer, sequence, None, 0)
            .unwrap();
        writer.size()
    };
    let queued = EventTime::from_milliseconds(10_000);
    let packet = channel
        .prepare_snapshot(sequence, None, &bytes[..length], queued)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    channel.submitted(queued).unwrap();
    packet
}

#[test]
fn live_nudge_and_backward_snapshot_failure_remain_connection_scoped() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let clients = [
        connect(
            &mut runtime,
            SeatId::FIRST,
            Protocol::Quake3_68,
            RuleSetId::Quake,
            RuleSetId::Quake2,
        ),
        connect(
            &mut runtime,
            SeatId::new(1).unwrap(),
            Protocol::Quake3_68,
            RuleSetId::Quake2,
            RuleSetId::QuakeWorld,
        ),
    ];
    let mut senders = std::array::from_fn::<_, 2, _>(|_| {
        let mut channel =
            Channel::load(Protocol::Quake3_68.channel(), Endpoint::Server, 32768, 16).unwrap();
        channel.configure_snapshots(Protocol::Quake3_68).unwrap();
        channel
    });
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(clients[0]);
    host.local_clients[1] = Some(clients[1]);
    let mut source = Source {
        time_ms: 10_000,
        packets: clients
            .into_iter()
            .zip([1000, 2000])
            .enumerate()
            .map(|(index, (client, time))| {
                (
                    client,
                    packet_from(&mut senders[index], Protocol::Quake3_68, time),
                )
            })
            .collect(),
        ..Default::default()
    };
    host.frame(&mut source, true);
    host.console
        .append_line("cl_timeNudge -15", Context::default())
        .unwrap();
    source.time_ms = 10_025;
    source.packets.push((
        clients[0],
        packet_from(&mut senders[0], Protocol::Quake3_68, 999),
    ));
    let result = host.frame(&mut source, true);
    assert_eq!(host.local_clients[0], None);
    assert_eq!(host.local_clients[1], Some(clients[1]));
    assert!(
        host.runtime.server.clients[clients[0].0 as usize]
            .connection
            .is_none()
    );
    assert!(
        host.runtime.server.clients[clients[1].0 as usize]
            .connection
            .is_some()
    );
    assert_eq!(result.commands[1].server_time_ms, 2000);
    assert_eq!(
        host.runtime.snapshots[clients[1].0 as usize]
            .as_ref()
            .unwrap()
            .interpolation()
            .unwrap()
            .time,
        ThinkTime::Milliseconds(2040)
    );
    assert_eq!(result.drains, 2);
    assert_eq!(source.polls, 4);
    assert_eq!(host.runtime.network.malformed, 0);
    assert!(!host.runtime.quit);
}

#[test]
fn bound_cgame_waits_for_an_active_clock_while_unbound_ui_keeps_refreshing() {
    use qa_app::modules::{Export, ModuleRequest, ModuleResult, Phase, Program};
    use qa_compat::{
        abi::{Q3_CLIENT, Q3_UI},
        services::CallContext,
    };
    use qa_core::primitives::{CallbackId, ModuleId};
    use qa_formats::program::qvm::Opcode::*;
    use qa_world::{area::LinkOrder, entities::AllocationPolicy};

    let mut runtime = Runtime::load(2, []).unwrap();
    let client = connect(
        &mut runtime,
        SeatId::FIRST,
        Protocol::Quake3_68,
        RuleSetId::Quake,
        RuleSetId::Quake2,
    );
    let modules = [ModuleId(1), ModuleId(2)];
    let requests = modules
        .into_iter()
        .enumerate()
        .map(|(index, module)| {
            let allocation = AllocationPolicy::q3(0);
            let anchor = runtime
                .server
                .entities
                .allocate(ThinkTime::Milliseconds(0), module, allocation)
                .unwrap()
                .id;
            ModuleRequest {
                search: None,
                source: None,
                q3_image: None,
                context: CallContext {
                    module,
                    clock: ThinkTime::Milliseconds(0),
                    server_frame: 0,
                    native_rules: RuleSetId::Quake3,
                    trace_rules: RuleSetId::QuakeWorld,
                    sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
                    console: Context::default(),
                    allocation,
                    link_order: LinkOrder::Head,
                },
                phase: Phase::Client,
                timing_rules: RuleSetId::Quake3,
                rate: TickRate::FrameDriven,
                anchor,
                program: Program::Qvm {
                    vm: Box::new(service_program::program(
                        &[(Enter, 64), (Local, 76), (Load4, 0), (Leave, 64)],
                        &[0; 16],
                    )),
                    imports: if index == 0 { &Q3_CLIENT } else { &Q3_UI },
                },
                entries: vec![0],
                frame: Export {
                    client: (index == 0).then_some(client),
                    ..Export::clocked(CallbackId(0))
                },
                prepare: vec![],
                initialize: vec![],
                shutdown: vec![],
                api: None,
                instruction_budget: 1000,
                configstrings: 0,
                files: 0,
                entity_source: None,
            }
        })
        .collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    let mut sender =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Server, 32768, 16).unwrap();
    sender.configure_snapshots(Protocol::Quake3_68).unwrap();
    let mut source = Source {
        time_ms: 10_000,
        packets: vec![(
            client,
            packet_with_flags(&mut sender, Protocol::Quake3_68, 1000, 2),
        )],
        ..Default::default()
    };
    for time in [10_000, 10_025] {
        source.time_ms = time;
        host.frame(&mut source, true);
        assert!(!host.runtime.quit);
        assert_eq!(host.module_counts(modules[0]).unwrap().calls, 0);
        assert_eq!(
            host.module_counts(modules[1]).unwrap().last_result,
            Some(ModuleResult::Qvm(time as i32))
        );
        assert!(
            host.runtime.snapshots[client.0 as usize]
                .as_ref()
                .unwrap()
                .interpolation()
                .is_none()
        );
    }
    source.time_ms = 10_050;
    source.packets.push((
        client,
        packet_with_flags(&mut sender, Protocol::Quake3_68, 1100, 0),
    ));
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(modules[0]).unwrap().calls, 1);
    assert_eq!(
        host.module_counts(modules[0]).unwrap().last_result,
        Some(ModuleResult::Qvm(1100))
    );
    assert_eq!(host.module_counts(modules[1]).unwrap().calls, 3);
    assert_eq!(
        host.module_counts(modules[1]).unwrap().last_result,
        Some(ModuleResult::Qvm(10_050))
    );
    assert_eq!(source.polls, 6);
}

#[test]
fn queued_q3_clock_offsets_stamp_prior_commands_without_changing_legacy_seats() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let q3 = connect(
        &mut runtime,
        SeatId::FIRST,
        Protocol::Quake3_68,
        RuleSetId::Quake,
        RuleSetId::Quake2,
    );
    let q2 = connect(
        &mut runtime,
        SeatId::new(1).unwrap(),
        Protocol::Quake2_34,
        RuleSetId::Quake3,
        RuleSetId::QuakeWorld,
    );
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(q3);
    host.local_clients[1] = Some(q2);
    let mut source = Source {
        time_ms: 10_000,
        packets: vec![
            (q3, packet(Protocol::Quake3_68, 1000)),
            (q2, packet(Protocol::Quake2_34, 1000)),
        ],
        ..Default::default()
    };
    for (queued, prior_command, render_time, q2_render) in [
        (10_000, 0, 1000, 900),
        (10_025, 1000, 1025, 925),
        (10_060, 1025, 1060, 960),
        (10_200, 1060, 1200, 1000),
    ] {
        source.time_ms = queued;
        let result = host.frame(&mut source, true);
        assert!(!host.runtime.quit);
        assert_eq!(result.drains, 2);
        assert_eq!(result.commands[0].server_time_ms, prior_command);
        assert_eq!(result.commands[1].server_time_ms, queued as i32);
        assert_eq!(
            host.runtime.snapshots[q3.0 as usize]
                .as_ref()
                .unwrap()
                .interpolation()
                .unwrap()
                .time,
            ThinkTime::Milliseconds(render_time)
        );
        assert_eq!(
            host.runtime.snapshots[q2.0 as usize]
                .as_ref()
                .unwrap()
                .interpolation()
                .unwrap()
                .time,
            ThinkTime::Milliseconds(q2_render)
        );
        for (client, seat) in [(q3, 0), (q2, 1)] {
            let recorded = host
                .runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .commands
                .as_ref()
                .unwrap()
                .history()
                .unwrap()
                .get(0)
                .unwrap();
            assert_eq!(
                recorded.server_time_ms,
                result.commands[seat].server_time_ms
            );
            assert_eq!(recorded.duration_ns, result.commands[seat].duration_ns);
        }
    }
    assert_eq!(source.polls, 8);
    assert_eq!(host.runtime.network.malformed, 0);
    assert_eq!(
        host.runtime.prediction[0].player.movement_rules,
        RuleSetId::Quake2
    );
    assert_eq!(
        host.runtime.prediction[1].player.movement_rules,
        RuleSetId::QuakeWorld
    );
    assert_eq!(
        host.runtime.snapshots[q3.0 as usize]
            .as_ref()
            .unwrap()
            .view_source(),
        Some(RuleSetId::Quake)
    );
    assert_eq!(
        host.runtime.snapshots[q2.0 as usize]
            .as_ref()
            .unwrap()
            .view_source(),
        Some(RuleSetId::Quake3)
    );
}

#[test]
fn quakeworld_queued_pause_and_push_latency_drive_the_same_client_clock() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = connect(
        &mut runtime,
        SeatId::FIRST,
        Protocol::QuakeWorld28,
        RuleSetId::Quake2,
        RuleSetId::Quake3,
    );
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    let packet = |sequence, body: &[u8]| {
        let mut bytes = [0; 1400];
        let length = qa_network::headers::encode(
            qa_network::headers::QUAKEWORLD,
            qa_network::headers::Direction::ToClient,
            qa_network::headers::Header {
                sequence,
                ..Default::default()
            },
            body,
            &mut bytes,
        )
        .unwrap();
        bytes[..length].to_vec()
    };
    let mut source = Source {
        time_ms: 10_000,
        packets: vec![(client, packet(1, &[47, 0, 0]))],
        ..Default::default()
    };
    host.console
        .append_line(
            "pushlatency 500",
            Context {
                source: RuleSetId::QuakeWorld,
                ..Default::default()
            },
        )
        .unwrap();
    let result = host.frame(&mut source, true);
    assert_eq!(result.drains, 2);
    let time = |host: &FrameHost| {
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .interpolation()
            .unwrap()
            .time
    };
    assert_eq!(time(&host), ThinkTime::Seconds(10.0));
    let push = host
        .console
        .cvars
        .bind(
            "pushlatency",
            Context {
                source: RuleSetId::QuakeWorld,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(host.console.cvars.numeric(push).unwrap(), 0.0);
    source.time_ms = 20_000;
    source.packets.push((client, packet(2, &[24, 1])));
    host.frame(&mut source, true);
    assert_eq!(time(&host), ThinkTime::Seconds(10.0));
    source.time_ms = 30_000;
    source.packets.push((client, packet(3, &[24, 0])));
    host.frame(&mut source, true);
    assert_eq!(time(&host), ThinkTime::Seconds(30.0));
    assert_eq!(source.polls, 6);
    assert_eq!(host.runtime.network.command_errors, 0);
    assert_eq!(host.local_clients[0], Some(client));
    assert!(!host.runtime.quit);
}
