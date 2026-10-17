#[path = "support/frame_source.rs"]
mod frame_source;
#[allow(dead_code)]
#[path = "model_assets/support.rs"]
mod support;
#[path = "../../formats/tests/support/stored_zip.rs"]
mod zip;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::{EventRing, OutputTarget},
    loopback::Endpoint,
    primitives::{ClientId, PrintKind, ResourceNamespace, RuleSetId},
    sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_formats::archive::Archive;
use qa_network::{
    channel::Delivery,
    commands::packet::Protocol,
    ingress::{Incoming, Route},
};
use qa_platform::files::ReadFile;
use qa_session::timing::TickRate;
use std::{net::SocketAddr, sync::Arc, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Block {
    percent: u8,
    bytes: Option<Vec<u8>>,
}

#[derive(Default)]
struct Source {
    transport: frame_source::Sink,
    milliseconds: u64,
    drop: Option<ClientId>,
    blocks: [Vec<Block>; 2],
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(self.milliseconds)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(self.milliseconds),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped SERVER download fixture")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn send_packet(&mut self, socket: u16, to: SocketAddr, bytes: &[u8]) -> bool {
        self.transport.send_packet(socket, to, bytes)
    }
}
impl Source {
    fn step(&mut self, host: &mut FrameHost) {
        self.milliseconds += 100;
        self.transport.time = EventTime::from_milliseconds(self.milliseconds);
        assert_eq!(host.frame(self, true).drains, 2);
        for packet in std::mem::take(&mut self.transport.packets) {
            assert!(packet.admitted);
            if packet.socket == 38 {
                let client = ClientId(u32::from(packet.to.port() - 30000));
                host.queue
                    .push(SysEvent {
                        time: self.transport.time,
                        kind: EventKind::Packet {
                            socket: 37,
                            from: qa_core::sys_events::Peer::Socket(address(client)),
                            bytes: &packet.bytes,
                        },
                    })
                    .unwrap();
                continue;
            }
            assert_eq!(packet.socket, 37);
            let client = ClientId(u32::from(packet.to.port() - 29000));
            if self.drop == Some(client) {
                continue;
            }
            let Some(connection) = host.runtime.network.get_mut(client, Endpoint::Client) else {
                continue;
            };
            let received = connection
                .channel
                .receive(&packet.bytes, self.transport.time)
                .unwrap();
            let Delivery::Payload(payload) = received.delivery else {
                continue;
            };
            let sequence = received.header.sequence;
            let commands = connection.commands.as_mut().unwrap();
            let length = commands.stage(payload).unwrap();
            commands
                .decode_output(
                    length,
                    sequence,
                    &mut connection.channel,
                    ResourceNamespace::Client(client),
                    |incoming| match incoming {
                        Incoming::Download(block) => {
                            let block = block.unwrap();
                            self.blocks[client.0 as usize].push(Block {
                                percent: block.percent,
                                bytes: block.bytes.map(<[u8]>::to_vec),
                            });
                        }
                        _ => {}
                    },
                )
                .unwrap();
        }
    }

    fn advance(&mut self, host: &mut FrameHost, frames: usize) {
        for _ in 0..frames {
            self.step(host);
        }
    }

    fn block(&mut self, host: &mut FrameHost, client: ClientId, command: &[u8]) -> Block {
        let previous = self.blocks[client.0 as usize].len();
        queue(host, client, command);
        for _ in 0..20 {
            self.step(host);
            if self.blocks[client.0 as usize].len() > previous {
                assert_eq!(self.blocks[client.0 as usize].len(), previous + 1);
                return self.blocks[client.0 as usize][previous].clone();
            }
        }
        panic!("native download command did not produce a block: {command:?}")
    }
}

fn address(client: ClientId) -> SocketAddr {
    ([127, 0, 0, 1], 29000 + client.0 as u16).into()
}

fn connect(host: &mut FrameHost, seat: SeatId, protocol: Protocol) -> ClientId {
    let client = host
        .runtime
        .connect_local(
            seat,
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: protocol.rule_set(),
                client: protocol.rule_set(),
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake2,
            },
            protocol,
            seat.index() as u32,
        )
        .unwrap();
    assert!(host.runtime.server.module_moves(client).is_none());
    host.runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .route = Route {
        socket: 37,
        peer: qa_core::sys_events::Peer::Socket(address(client)),
    };
    host.runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap()
        .route = Route {
        socket: 38,
        peer: qa_core::sys_events::Peer::Socket(([127, 0, 0, 1], 30000 + client.0 as u16).into()),
    };
    host.local_clients[seat.index()] = Some(client);
    client
}

fn queue(host: &mut FrameHost, client: ClientId, text: &[u8]) {
    let connection = host
        .runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap();
    let protocol = connection.commands.as_ref().unwrap().protocol;
    connection
        .channel
        .queue_client_command(protocol, text)
        .unwrap();
}

fn setting(host: &mut FrameHost, protocol: Protocol, name: &str, value: &str) {
    let view = host
        .console
        .cvars
        .bind(
            name,
            Context {
                source: protocol.rule_set(),
                side: qa_console::catalog::Scope::Server,
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(view, value).unwrap();
}

fn fixture(protocols: [Protocol; 2], small_ring: bool) -> (support::Fixture, FrameHost, Source) {
    let mut files = support::Fixture::new(&[
        (
            "skins/large.pcx",
            (0..2300).map(|index| (index % 251) as u8).collect(),
        ),
        ("skins/small.pcx", b"raw native source".to_vec()),
        ("skins/empty.pcx", Vec::new()),
        ("maps/loose.bsp", b"loose map bytes".to_vec()),
    ]);
    let root = files.vfs.mounts().next().unwrap().path.clone();
    let packed = zip::stored_zip(&[
        (b"skins/packed.pcx", b"archive bytes, not image pixels"),
        (b"maps/packed.bsp", b"archived map"),
    ]);
    let package = root.join("downloads.pk3");
    qa_platform::files::write(&package, &packed).unwrap();
    files
        .vfs
        .mount_archive(
            &package,
            1,
            Arc::new(Archive::parse(Arc::new(ReadFile::open(&package).unwrap())).unwrap()),
        )
        .unwrap();
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime.vfs = std::mem::take(&mut files.vfs);
    if small_ring {
        runtime.server.events = EventRing::load(8, 4, 40, 8192).unwrap();
        runtime.server.presentation = runtime
            .server
            .events
            .bind(OutputTarget::Presentation)
            .unwrap();
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(20).unwrap(),
        vec![],
    )
    .unwrap();
    for (index, protocol) in protocols.into_iter().enumerate() {
        assert_eq!(
            connect(&mut host, SeatId::ALL[index], protocol),
            ClientId(index as u32)
        );
        for name in [
            "sv_allowDownload",
            "allow_download_skins",
            "allow_download_players",
            "allow_download_maps",
            "allow_download_models",
            "allow_download_sounds",
            "allow_download_textures",
            "allow_download_pics",
            "allow_download_others",
        ] {
            setting(&mut host, protocol, name, "1");
        }
    }
    let mut source = Source::default();
    source.advance(&mut host, 16);
    (files, host, source)
}

#[test]
fn module_free_native_requests_use_live_permissions_and_copy_loose_or_archive_bytes() {
    let (_files, mut host, mut source) = fixture([Protocol::QuakeWorld28; 2], false);
    let client = ClientId(0);
    let packed = source.block(&mut host, client, b"download skins/packed.pcx");
    assert_eq!(
        packed,
        Block {
            percent: 100,
            bytes: Some(b"archive bytes, not image pixels".to_vec())
        }
    );
    setting(
        &mut host,
        Protocol::QuakeWorld28,
        "allow_download_skins",
        "0",
    );
    assert_eq!(
        source
            .block(&mut host, client, b"download skins/small.pcx")
            .bytes,
        None
    );
    setting(
        &mut host,
        Protocol::QuakeWorld28,
        "allow_download_skins",
        "1",
    );
    setting(&mut host, Protocol::QuakeWorld28, "sv_allowDownload", "0");
    assert_eq!(
        source
            .block(&mut host, client, b"download skins/small.pcx")
            .bytes,
        None
    );
    setting(&mut host, Protocol::QuakeWorld28, "sv_allowDownload", "1");
    assert_eq!(
        source
            .block(&mut host, client, b"download skins/small.pcx")
            .bytes
            .unwrap(),
        b"raw native source"
    );
    assert_eq!(
        source
            .block(&mut host, client, b"download maps/packed.bsp")
            .bytes,
        None
    );
    assert_eq!(
        source
            .block(&mut host, client, b"download maps/loose.bsp")
            .bytes
            .unwrap(),
        b"loose map bytes"
    );
    assert_eq!(
        source.block(&mut host, client, b"download skins/empty.pcx"),
        Block {
            percent: 0,
            bytes: Some(Vec::new())
        }
    );
    assert!(host.runtime.server.world_frame > 0);
    assert!(host.runtime.server.module_moves(client).is_none());
}

#[test]
fn q2_native_offsets_are_checked_before_the_shared_raw_block_sender() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let (_files, mut host, mut source) = fixture([protocol; 2], false);
        let client = ClientId(0);
        let expected: Vec<_> = (1200..2300).map(|index| (index % 251) as u8).collect();
        let first = source.block(&mut host, client, b"download skins/large.pcx 1200suffix");
        assert_eq!(first.percent, 96);
        assert_eq!(first.bytes.unwrap(), expected[..1024]);
        let last = source.block(&mut host, client, b"nextdl");
        assert_eq!(last.percent, 100);
        assert_eq!(last.bytes.unwrap(), expected[1024..]);
        assert_eq!(
            source
                .block(&mut host, client, b"download skins/large.pcx -1")
                .bytes,
            None
        );
        assert_eq!(
            source.block(&mut host, client, b"download skins/large.pcx 99999"),
            Block {
                percent: 100,
                bytes: Some(Vec::new())
            }
        );
    }
}

#[test]
fn unacknowledged_file_chunk_stays_reliable_while_another_client_and_ticks_continue() {
    let (_files, mut host, mut source) = fixture([Protocol::QuakeWorld28; 2], false);
    let client = ClientId(0);
    let healthy = ClientId(1);
    let output = host.runtime.server.clients[0].output.unwrap();
    let before = host.runtime.server.events.counters(output).unwrap();
    source.drop = Some(client);
    queue(&mut host, client, b"download skins/large.pcx");
    source.advance(&mut host, 3);
    let waiting = host.runtime.server.events.counters(output).unwrap();
    assert!(waiting.submissions > before.submissions);
    assert_eq!(waiting.acknowledged_records, before.acknowledged_records);
    assert!(source.blocks[0].is_empty());
    let tick = host.runtime.server.world_frame;
    let healthy_output = host.runtime.server.clients[healthy.0 as usize]
        .output
        .unwrap();
    let healthy_acknowledged = host
        .runtime
        .server
        .events
        .counters(healthy_output)
        .unwrap()
        .acknowledged_records;
    assert_eq!(
        source
            .block(&mut host, healthy, b"download skins/small.pcx")
            .bytes
            .unwrap(),
        b"raw native source"
    );
    source.advance(&mut host, 4);
    assert!(
        host.runtime
            .server
            .events
            .counters(healthy_output)
            .unwrap()
            .acknowledged_records
            > healthy_acknowledged
    );
    assert!(host.runtime.server.world_frame > tick);
    source.drop = None;
    for _ in 0..20 {
        source.step(&mut host);
        if !source.blocks[0].is_empty() {
            break;
        }
    }
    assert_eq!(source.blocks[0].len(), 1);
    assert_eq!(source.blocks[0][0].bytes.as_ref().unwrap().len(), 768);
    source.advance(&mut host, 3);
    assert!(
        host.runtime
            .server
            .events
            .counters(output)
            .unwrap()
            .acknowledged_records
            > before.acknowledged_records
    );
    let second = source.block(&mut host, client, b"nextdl");
    assert_eq!(
        second.bytes.unwrap(),
        (768..1536)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>()
    );
}

#[test]
fn resync_and_disconnect_retire_the_old_source_before_continuation_or_slot_reuse() {
    let (_files, mut host, mut source) = fixture([Protocol::QuakeWorld28; 2], true);
    let client = ClientId(0);
    let first = source.block(&mut host, client, b"download skins/large.pcx");
    assert_eq!(first.bytes.unwrap().len(), 768);
    source.advance(&mut host, 3);
    let count = source.blocks[0].len();
    queue(&mut host, client, b"new");
    source.advance(&mut host, 3);
    queue(&mut host, client, b"nextdl");
    source.advance(&mut host, 3);
    assert_eq!(source.blocks[0].len(), count);
    assert_eq!(
        source
            .block(&mut host, client, b"download skins/large.pcx")
            .bytes
            .unwrap()
            .len(),
        768
    );
    source.advance(&mut host, 3);
    let old_output = host.runtime.server.clients[0].output.unwrap();
    for _ in 0..16 {
        host.runtime
            .server
            .events
            .print(
                Some(client),
                PrintKind::Console,
                format_args!("force native resync"),
            )
            .unwrap();
    }
    assert!(host.runtime.server.events.needs_resync(old_output));
    source.advance(&mut host, 12);
    assert_ne!(host.runtime.server.clients[0].output, Some(old_output));
    let count = source.blocks[0].len();
    queue(&mut host, client, b"nextdl");
    source.advance(&mut host, 3);
    assert_eq!(source.blocks[0].len(), count);
    assert_eq!(
        source
            .block(&mut host, client, b"download skins/large.pcx")
            .bytes
            .unwrap()
            .len(),
        768
    );
    let lifetime = host.runtime.server.clients[0].entity;
    assert!(host.runtime.disconnect_client(client));
    host.local_clients[0] = None;
    source.advance(&mut host, 4);
    assert_eq!(
        connect(&mut host, SeatId::FIRST, Protocol::QuakeWorld28),
        client
    );
    assert_ne!(host.runtime.server.clients[0].entity, lifetime);
    source.advance(&mut host, 4);
    let count = source.blocks[0].len();
    queue(&mut host, client, b"nextdl");
    source.advance(&mut host, 3);
    assert_eq!(source.blocks[0].len(), count);
    assert_eq!(
        source
            .block(&mut host, client, b"download skins/small.pcx")
            .bytes
            .unwrap(),
        b"raw native source"
    );
}
