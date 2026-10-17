use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
    modules::Driver,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, RuleSetId, Vec3},
    sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{
        connection::Commands,
        packet::Protocol,
        startup::{NqControl, read_nq_control, write_nq_control},
    },
    headers::{self, Direction},
    ingress::{ConnectionPhase, Peer},
};
use qa_platform::EventPump;
use qa_session::timing::TickRate;
use std::{net::SocketAddr, ops::ControlFlow, time::Duration};

#[path = "support/client_asset_frames.rs"]
mod asset_frames;
#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod support;

struct Source {
    pump: EventPump,
    time: EventTime,
    reject_connect: bool,
    rejected_connects: usize,
    reject_assets: bool,
    rejected_assets: usize,
    prepare_hud: bool,
    backend: Option<asset_frames::Source>,
}
impl qa_app::renderer::AssetPreparer for Source {
    fn target_size(&self) -> Option<[u32; 2]> {
        self.prepare_hud
            .then(|| self.backend.as_ref().unwrap().cpu.dimensions())
    }
    fn prepare_assets(
        &mut self,
        assets: &qa_render::Assets,
        scene: Option<qa_render::Limits>,
    ) -> Result<(), String> {
        if self.reject_assets {
            self.rejected_assets += 1;
            return Err("owned UDP fixture backend rejection".into());
        }
        qa_app::renderer::AssetPreparer::prepare_assets(
            self.backend.as_mut().unwrap(),
            assets,
            scene,
        )
    }
}
impl FrameSource for Source {
    fn driver(&mut self) -> Driver<'_> {
        let preparer = self
            .backend
            .as_mut()
            .map(|backend| backend as &mut dyn qa_app::renderer::AssetPreparer);
        Driver::with_connector(preparer, &mut self.pump)
    }
    fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
        self.backend.as_ref()?;
        Some(self)
    }
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.pump.poll_network(queue);
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
    fn sound(&mut self, _: Option<&qa_audio::Bank>, _: qa_core::primitives::SoundEvent) -> bool {
        true
    }
    fn send_packet(&mut self, socket: u16, to: SocketAddr, bytes: &[u8]) -> bool {
        if self.reject_connect && bytes.starts_with(b"\xff\xff\xff\xffconnect ") {
            self.rejected_connects += 1;
            return false;
        }
        self.pump.send_udp(socket, to, bytes).is_ok()
    }
}
fn context() -> Context {
    Context {
        source: RuleSetId::Quake,
        ..Context::default()
    }
}
fn host(capacity: usize) -> FrameHost {
    FrameHost::load(
        Console::new(context()).unwrap(),
        Runtime::load(capacity, []).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap()
}
fn queued(host: &mut FrameHost, source: &mut Source, text: &str, milliseconds: u64) {
    source.time = EventTime::from_milliseconds(milliseconds);
    host.queue
        .push(SysEvent {
            time: source.time,
            kind: EventKind::ConsoleLine(text),
        })
        .unwrap();
    host.frame(source, true);
}
fn datagrams(pump: &mut EventPump) -> Vec<(Peer, Vec<u8>)> {
    let mut queue = SysEventQueue::load(64, 32_768).unwrap();
    pump.poll_network(&mut queue);
    let mut result = Vec::new();
    while let Some(event) = queue.pop() {
        let EventKind::Packet { from, bytes, .. } = event.kind else {
            panic!("UDP fixture packet")
        };
        result.push((from, bytes.to_vec()));
    }
    result
}

#[test]
fn queued_connect_reconnect_and_disconnect_use_one_owned_udp_channel() {
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let mut source = Source {
        pump: EventPump::new(),
        time: EventTime(0),
        reject_connect: false,
        rejected_connects: 0,
        reject_assets: false,
        rejected_assets: 0,
        prepare_hud: false,
        backend: None,
    };
    let mut host = host(3);
    let healthy = host
        .runtime
        .connect_local(
            SeatId::new(1).unwrap(),
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                client: RuleSetId::Quake3,
                input: RuleSetId::QuakeWorld,
                movement: RuleSetId::Quake2,
                trace: RuleSetId::Quake,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    host.local_clients[1] = Some(healthy);
    let healthy_entity = host.runtime.server.clients[healthy.0 as usize].entity;
    queued(
        &mut host,
        &mut source,
        &format!("alias connect quit; connect {address}\n"),
        0,
    );
    assert!(!host.runtime.quit);
    let client = host.local_clients[0].unwrap();
    let connection = host.runtime.network.get(client, Endpoint::Client).unwrap();
    assert!(matches!(connection.phase, ConnectionPhase::Connecting(_)));
    assert!(host.runtime.network.get(client, Endpoint::Server).is_none());
    let token = connection.route.socket;
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .udp_socket(),
        Some(token)
    );
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    assert!(matches!(
        read_nq_control(&packets[0].1).unwrap(),
        NqControl::Connect
    ));
    let Peer::Socket(client_address) = packets[0].0 else {
        panic!("socket source")
    };
    let mut bytes = [0; 1400];
    let length = write_nq_control(
        NqControl::Accept {
            port: address.port(),
        },
        &mut bytes,
    )
    .unwrap();
    server
        .send_udp(socket, client_address, &bytes[..length])
        .unwrap();
    source.time = EventTime::from_milliseconds(10);
    host.frame(&mut source, true);
    assert!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
    let mut info = vec![11];
    info.extend(15u32.to_le_bytes());
    info.extend([2, 0]);
    info.extend(b"new level\0\0\0");
    info.extend([5, 1, 0, 25, 4]);
    let length = headers::encode(
        headers::NETQUAKE,
        Direction::ToClient,
        headers::Header {
            datagram_flags: headers::datagram::DATA | headers::datagram::EOM,
            ..Default::default()
        },
        &info,
        &mut bytes,
    )
    .unwrap();
    server
        .send_udp(socket, client_address, &bytes[..length])
        .unwrap();
    source.time = EventTime::from_milliseconds(20);
    host.frame(&mut source, true);
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(4)
    );
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .udp_socket(),
        Some(token)
    );
    datagrams(&mut server);
    let before = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .send_state();
    queued(&mut host, &mut source, "reconnect\n", 30);
    let connection = host.runtime.network.get(client, Endpoint::Client).unwrap();
    assert_eq!(connection.route.socket, token);
    assert_eq!(connection.channel.send_state(), before);
    assert_eq!(
        connection
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        0
    );
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(0)
    );
    queued(&mut host, &mut source, "disconnect\n", 40);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    let (header, body) =
        headers::decode(headers::NETQUAKE, Direction::ToServer, &packets[0].1).unwrap();
    assert_eq!(header.datagram_flags, headers::datagram::UNRELIABLE);
    assert_eq!(body, [2]);
    assert!(host.local_clients[0].is_none());
    assert!(host.runtime.network.get(client, Endpoint::Client).is_none());
    assert!(source.pump.send_udp(token, address, b"closed").is_err());
    assert_eq!(host.local_clients[1], Some(healthy));
    assert_eq!(
        host.runtime.server.clients[healthy.0 as usize].entity,
        healthy_entity
    );
    assert!(
        host.runtime
            .network
            .get(healthy, Endpoint::Server)
            .is_some()
    );
}

#[test]
fn unavailable_or_unsupported_connect_preserves_the_current_peer() {
    let mut host = host(1);
    let client = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                client: RuleSetId::Quake,
                input: RuleSetId::QuakeWorld,
                movement: RuleSetId::Quake2,
                trace: RuleSetId::Quake3,
            },
            Protocol::NetQuake15,
            0,
        )
        .unwrap();
    host.local_clients[0] = Some(client);
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    for selected in [RuleSetId::Quake, RuleSetId::Quake3] {
        host.console
            .append(
                "connect 127.0.0.1:26000\n",
                Context {
                    source: selected,
                    ..context()
                },
            )
            .unwrap();
        host.execute_commands();
        assert_eq!(host.local_clients[0], Some(client));
        assert_eq!(
            host.runtime.server.clients[client.0 as usize].entity,
            entity
        );
        assert!(host.runtime.network.get(client, Endpoint::Client).is_some());
    }
    let mut pump = EventPump::new();
    for text in [
        "connect 127.0.0.1:26000 extra\n",
        "connect [::1]:26000 extra\n",
        "connect \"127.0.0.1:26000\" extra\n",
    ] {
        host.console.append(text, context()).unwrap();
        host.execute_commands_with_driver(&mut Driver::with_connector(None, &mut pump));
        assert_eq!(host.local_clients[0], Some(client));
        assert_eq!(
            host.runtime.server.clients[client.0 as usize].entity,
            entity
        );
    }
}

#[test]
fn one_slot_replacement_waits_for_presentation_and_preserves_all_role_choices() {
    for startup in [false, true] {
        let mut server = EventPump::new();
        let (_, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
        let mut source = Source {
            pump: EventPump::new(),
            time: EventTime(0),
            reject_connect: false,
            rejected_connects: 0,
            reject_assets: false,
            rejected_assets: 0,
            prepare_hud: false,
            backend: None,
        };
        let mut host = host(1);
        let policy = ClientPolicy {
            client: RuleSetId::Quake2Rerelease,
            input: RuleSetId::QuakeWorld,
            movement: RuleSetId::Quake3,
            trace: RuleSetId::Quake2,
        };
        let old = host
            .runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                policy,
                Protocol::NetQuake15,
                0,
            )
            .unwrap();
        host.local_clients[0] = Some(old);
        let old_lifetime = host.runtime.server.clients[old.0 as usize].entity;
        if startup {
            host.console
                .append(&format!("connect \"{address}\"\n"), context())
                .unwrap();
            host.execute_commands_with_driver(&mut source.driver());
            assert!(
                host.runtime.server.clients[old.0 as usize]
                    .audio_retirement
                    .is_some()
            );
            host.dispatch_output(&mut source, &mut Default::default());
        } else {
            queued(
                &mut host,
                &mut source,
                &format!("connect \"{address}\"\n"),
                0,
            );
        }
        assert!(host.local_clients[0].is_none());
        assert!(
            host.runtime.server.clients[old.0 as usize]
                .audio_retirement
                .is_none()
        );
        assert!(!host.console.idle());
        assert!(datagrams(&mut server).is_empty());
        // EXEC_NOW and both ordinary command drains share the frame fence.
        for _ in 0..4 {
            host.execute_commands_with_driver(&mut source.driver());
            assert!(host.local_clients[0].is_none());
            assert!(datagrams(&mut server).is_empty());
        }
        source.time = EventTime::from_milliseconds(10);
        host.frame(&mut source, true);
        let replacement = host.local_clients[0].unwrap();
        assert_eq!(replacement, old);
        let row = &host.runtime.server.clients[replacement.0 as usize];
        assert_ne!(row.entity, old_lifetime);
        assert_eq!(row.client_rules, policy.client);
        assert_eq!(row.input_rules, policy.input);
        assert_eq!(
            host.runtime.prediction[0].player.movement_rules,
            policy.movement
        );
        assert_eq!(host.runtime.prediction[0].player.trace_rules, policy.trace);
        assert!(matches!(
            host.runtime
                .network
                .get(replacement, Endpoint::Client)
                .unwrap()
                .phase,
            ConnectionPhase::Connecting(_)
        ));
        assert!(
            host.runtime
                .network
                .get(replacement, Endpoint::Server)
                .is_none()
        );
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        assert!(matches!(
            read_nq_control(&packets[0].1).unwrap(),
            NqControl::Connect
        ));
        assert!(host.console.idle());
    }
}

#[path = "console_connect/q2.rs"]
mod q2;
#[path = "console_connect/q3.rs"]
mod q3;
#[path = "console_connect/qw.rs"]
mod qw;

fn source() -> Source {
    Source {
        pump: EventPump::new(),
        time: EventTime::default(),
        reject_connect: false,
        rejected_connects: 0,
        reject_assets: false,
        rejected_assets: 0,
        prepare_hud: false,
        backend: None,
    }
}
fn frame(host: &mut FrameHost, source: &mut Source, milliseconds: u64) {
    source.time = EventTime::from_milliseconds(milliseconds);
    assert_eq!(host.frame(source, true).drains, 2);
}
fn peer(packet: &(Peer, Vec<u8>)) -> SocketAddr {
    let Peer::Socket(address) = packet.0 else {
        panic!("private UDP peer")
    };
    address
}
fn response(pump: &mut EventPump, socket: u16, address: SocketAddr, text: &[u8]) {
    let mut bytes = vec![255; 4];
    bytes.extend(text);
    pump.send_udp(socket, address, &bytes).unwrap();
}
fn history(host: &FrameHost, client: ClientId) -> u64 {
    host.runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .commands
        .as_ref()
        .unwrap()
        .history()
        .unwrap()
        .number()
}

struct NativePeer {
    channel: Channel,
    commands: Commands,
    strings: Vec<Vec<u8>>,
    userinfos: Vec<Vec<u8>>,
    moves: usize,
    q3_moves: Vec<qa_network::commands::packet::Move>,
}
impl NativePeer {
    fn new(protocol: Protocol, qport: u16) -> Self {
        let mut channel = Channel::load(
            protocol.client_channel(qport),
            Endpoint::Server,
            protocol.message_bytes(),
            16,
        )
        .unwrap();
        channel.configure_snapshots(protocol).unwrap();
        channel.set_qport(qport);
        Self {
            channel,
            commands: Commands::load(protocol, protocol.message_bytes()),
            strings: vec![],
            userinfos: vec![],
            moves: 0,
            q3_moves: vec![],
        }
    }
    fn receive(&mut self, bytes: &[u8], milliseconds: u64) {
        let time = EventTime::from_milliseconds(milliseconds);
        let received = self.channel.receive(bytes, time).unwrap();
        let sequence = received.header.sequence;
        if let Delivery::Payload(payload) = received.delivery {
            let inspected =
                (self.commands.protocol == Protocol::Quake3_68).then(|| payload.to_vec());
            let length = self.commands.stage(payload).unwrap();
            if let Some(mut inspected) = inspected {
                let ack = qa_network::commands::packet::acknowledgements(&inspected).unwrap();
                let key = self.channel.command_key(Some(ack)).unwrap();
                qa_network::commands::packet::read_with_commands(
                    Protocol::Quake3_68,
                    &mut inspected,
                    sequence,
                    key,
                    |record| {
                        if let qa_network::commands::packet::Record::Move(movement) = record {
                            self.q3_moves.push(movement);
                        }
                        Ok(ControlFlow::Continue(()))
                    },
                )
                .unwrap();
            }
            self.commands
                .decode(
                    length,
                    sequence,
                    milliseconds as i32,
                    &mut self.channel,
                    None,
                    |incoming| {
                        match incoming {
                            qa_network::ingress::Incoming::CommandString { text, .. } => {
                                self.strings.push(text.to_vec());
                                if text == b"disconnect" {
                                    return ControlFlow::Break(());
                                }
                            }
                            qa_network::ingress::Incoming::Command { .. } => self.moves += 1,
                            qa_network::ingress::Incoming::Userinfo { text, .. } => {
                                self.userinfos.push(text.to_vec())
                            }
                            _ => {}
                        }
                        ControlFlow::Continue(())
                    },
                )
                .unwrap();
        }
    }
    fn count(&self, text: &[u8]) -> usize {
        self.strings
            .iter()
            .filter(|candidate| candidate.as_slice() == text)
            .count()
    }
    fn reply(&mut self, pump: &mut EventPump, socket: u16, address: SocketAddr, milliseconds: u64) {
        let time = EventTime::from_milliseconds(milliseconds);
        let packet = self.channel.prepare_output(time).unwrap().unwrap();
        pump.send_udp(socket, address, packet.bytes).unwrap();
        self.channel.submitted(time).unwrap();
    }
}

fn wire(write: impl FnOnce(&mut qa_network::message::Writer<'_>)) -> Vec<u8> {
    let mut bytes = [0; 2048];
    let mut writer =
        qa_network::message::Writer::new(&mut bytes, qa_network::message::Encoding::Bytes);
    write(&mut writer);
    writer.bytes().to_vec()
}
fn deliver(
    host: &mut FrameHost,
    source: &mut Source,
    server: &mut EventPump,
    socket: u16,
    local: SocketAddr,
    native: &mut NativePeer,
    bytes: &[u8],
    milliseconds: u64,
) {
    native.channel.queue_reliable(bytes).unwrap();
    native.reply(server, socket, local, milliseconds);
    frame(host, source, milliseconds);
    for (_, bytes) in datagrams(server) {
        native.receive(&bytes, milliseconds);
    }
}
