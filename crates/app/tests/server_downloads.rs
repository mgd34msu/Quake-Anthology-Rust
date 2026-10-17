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
    ticks: u64,
    drop: Option<ClientId>,
    blocks: [Vec<Block>; 2],
    numbered: [Vec<qa_core::primitives::FileChunk<Vec<u8>>>; 2],
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
        let frame = host.frame(self, true);
        self.ticks += frame.server_ticks;
        assert_eq!(frame.drains, 2);
        for packet in std::mem::take(&mut self.transport.packets) {
            if !packet.admitted {
                continue;
            }
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
                        Incoming::Download(block) => match block.unwrap() {
                            qa_network::downloads::Chunk::Sequential { percent, bytes, .. } => {
                                self.blocks[client.0 as usize].push(Block {
                                    percent,
                                    bytes: bytes.map(<[u8]>::to_vec),
                                });
                            }
                            chunk => {
                                self.numbered[client.0 as usize].push(chunk.map(<[u8]>::to_vec))
                            }
                        },
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

#[test]
fn numbered_ring_blocks_share_q3_commands_fragments_and_best_effort_retirement() {
    use qa_core::primitives::FileChunk;
    let (_files, mut host, mut source) = fixture([Protocol::Quake3_68; 2], false);
    let client = ClientId(0);
    host.runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel
        .queue_reliable(b"print \"download control\"\n")
        .unwrap();
    let command_sequence = host
        .runtime
        .network
        .get(client, Endpoint::Server)
        .unwrap()
        .channel
        .command_state()
        .unwrap()
        .queued;
    let entity = host.runtime.server.clients[0].entity;
    let owner = host.runtime.server.clients[0].output.unwrap();
    let bytes: Vec<_> = (0..2048).map(|index| (index % 251) as u8).collect();
    let record = host
        .runtime
        .server
        .events
        .file_chunk(
            client,
            entity,
            FileChunk::Block {
                number: 0,
                total: Some(bytes.len() as u32),
                bytes: &bytes,
            },
        )
        .unwrap();
    source.transport.reject = Some(address(client));
    source.advance(&mut host, 2);
    assert!(source.numbered[0].is_empty());
    let mut batch = host.runtime.server.events.batch(owner).unwrap();
    assert_eq!(
        host.runtime
            .server
            .events
            .next(&mut batch)
            .unwrap()
            .sequence,
        record
    );
    assert!(
        host.runtime
            .network
            .get(client, Endpoint::Server)
            .unwrap()
            .channel
            .pending_packet()
            .is_some()
    );
    // A stalled fragmented download must not stop a healthy peer's stream.
    host.runtime
        .server
        .events
        .file_chunk(
            ClientId(1),
            host.runtime.server.clients[1].entity,
            FileChunk::Block {
                number: 0,
                total: Some(1),
                bytes: &[99],
            },
        )
        .unwrap();
    source.step(&mut host);
    assert_eq!(
        source.numbered[1],
        vec![FileChunk::Block {
            number: 0,
            total: Some(1),
            bytes: vec![99]
        }]
    );
    source.transport.reject = None;
    source.advance(&mut host, 3);
    assert_eq!(
        source.numbered[0],
        vec![FileChunk::Block {
            number: 0,
            total: Some(2048),
            bytes
        }]
    );
    let mut batch = host.runtime.server.events.batch(owner).unwrap();
    assert!(host.runtime.server.events.next(&mut batch).is_none());
    assert!(
        host.runtime
            .network
            .get(client, Endpoint::Server)
            .unwrap()
            .pending_output
            .is_none()
    );
    assert_eq!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .server_command(command_sequence)
            .unwrap(),
        b"print \"download control\"\n"
    );
    assert_eq!(
        host.runtime
            .network
            .get(client, Endpoint::Server)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .submitted,
        command_sequence
    );
    // A record addressed to a previous slot lifetime cannot reach this connection.
    let stale = qa_core::primitives::EntityId {
        generation: entity.generation.wrapping_sub(1),
        ..entity
    };
    host.runtime
        .server
        .events
        .file_chunk(
            client,
            stale,
            FileChunk::Block {
                number: 7,
                total: None,
                bytes: &[88],
            },
        )
        .unwrap();
    host.runtime
        .server
        .events
        .file_chunk(
            client,
            entity,
            FileChunk::Block {
                number: 1,
                total: None,
                bytes: &[],
            },
        )
        .unwrap();
    host.runtime
        .server
        .events
        .file_chunk(
            client,
            entity,
            FileChunk::Rejected {
                size: -1,
                message: b"file unavailable",
            },
        )
        .unwrap();
    source.advance(&mut host, 3);
    assert_eq!(
        source.numbered[0][1..],
        [
            FileChunk::Block {
                number: 1,
                total: None,
                bytes: vec![]
            },
            FileChunk::Rejected {
                size: -1,
                message: b"file unavailable".to_vec()
            }
        ]
    );
}

fn q3_package(host: &mut FrameHost) -> Vec<u8> {
    let root = host
        .runtime
        .vfs
        .mounts()
        .find(|mount| mount.kind == qa_content::vfs::MountKind::Directory)
        .unwrap()
        .path
        .clone()
        .join("baseq3");
    qa_platform::files::create_dir_all(&root).unwrap();
    let payload: Vec<u8> = (0..19000).map(|index| (index * 29) as u8).collect();
    let package = zip::stored_zip(&[(b"maps/download.dat", &payload)]);
    qa_platform::files::write(&root.join("custom.pk3"), &package).unwrap();
    qa_platform::files::write(&root.join("empty.pk3"), &[]).unwrap();
    qa_platform::files::write(&root.join("pak0.pk3"), &package).unwrap();
    host.runtime.vfs.mount_directory(&root, 2).unwrap();
    package
}

#[test]
fn q3_server_window_retains_lost_blocks_and_advances_only_on_native_ack() {
    use qa_core::primitives::FileChunk;
    let (_files, mut host, mut source) = fixture([Protocol::Quake3_68; 2], false);
    let package = q3_package(&mut host);
    let client = ClientId(0);
    queue(&mut host, client, b"download baseq3/custom.pk3");
    source.advance(&mut host, 9);
    assert_eq!(source.numbered[0].len(), 8);
    for (number, chunk) in source.numbered[0].iter().enumerate() {
        let FileChunk::Block {
            number: actual,
            total,
            bytes,
        } = chunk
        else {
            panic!("block");
        };
        assert_eq!(*actual, number as i16);
        assert_eq!(*total, (number == 0).then_some(package.len() as u32));
        assert_eq!(bytes, &package[number * 2048..(number + 1) * 2048]);
    }
    // The initial eight blocks are retained until application ACKs arrive,
    // regardless of channel ACKs or new host frames. A healthy peer progresses.
    let ticks = source.ticks;
    queue(&mut host, ClientId(1), b"download baseq3/custom.pk3");
    source.advance(&mut host, 2);
    assert!(source.ticks > ticks);
    assert_eq!(source.numbered[0].len(), 8);
    assert!(!source.numbered[1].is_empty());
    source.advance(&mut host, 9);
    assert!(source.numbered[0].len() > 8);
    assert_eq!(source.numbered[0][8], source.numbered[0][0]);
    assert!(
        source.numbered[0]
            .iter()
            .all(|chunk| matches!(chunk, FileChunk::Block { number: 0..=7, .. }))
    );
    // ACKs for the original window remain valid while its resend cursor runs.
    for number in 0..8 {
        queue(&mut host, client, format!("nextdl {number}").as_bytes());
    }
    source.advance(&mut host, 4);
    let blocks: Vec<_> = source.numbered[0]
        .iter()
        .filter_map(|chunk| match chunk {
            FileChunk::Block { number, bytes, .. } if *number >= 8 => {
                Some((*number, bytes.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        blocks,
        vec![
            (8, package[16384..18432].to_vec()),
            (9, package[18432..].to_vec()),
            (10, vec![])
        ]
    );
    for number in 8..=10 {
        queue(&mut host, client, format!("nextdl {number}").as_bytes());
    }
    source.advance(&mut host, 2);
    let complete = source.numbered[0].len();
    source.advance(&mut host, 15);
    assert_eq!(source.numbered[0].len(), complete);
    assert!(host.runtime.server.clients[0].connection.is_some());
    assert!(host.runtime.server.clients[1].connection.is_some());
}

#[test]
fn q3_server_rejections_stop_and_replacement_are_scoped_to_one_transfer() {
    use qa_core::primitives::FileChunk;
    let (_files, mut host, mut source) = fixture([Protocol::Quake3_68; 2], false);
    q3_package(&mut host);
    let client = ClientId(0);
    for (request, expected) in [
        (b"baseq3/pak0.pk3".as_slice(), "Cannot autodownload id pk3"),
        (b"missionpack/pak1.pk3", "Cannot autodownload Team Arena"),
        (b"baseq3/empty.pk3", "not found"),
        (b"baseq3/no.pk3", "not found"),
        (b"baseq3/../escape.pk3", "not found"),
        (b"baseq3/config.cfg", "not found"),
    ] {
        let previous = source.numbered[0].len();
        queue(
            &mut host,
            client,
            format!("download {}", String::from_utf8_lossy(request)).as_bytes(),
        );
        source.advance(&mut host, 3);
        let FileChunk::Rejected { size, message } = &source.numbered[0][previous] else {
            panic!("rejection");
        };
        assert_eq!(*size, -1);
        assert!(String::from_utf8_lossy(message).contains(expected));
    }
    setting(&mut host, Protocol::Quake3_68, "sv_allowDownload", "0.5");
    queue(&mut host, client, b"download baseq3/custom.pk3");
    source.advance(&mut host, 3);
    assert!(
        matches!(source.numbered[0].last().unwrap(), FileChunk::Rejected { message, .. } if String::from_utf8_lossy(message).contains("disabled"))
    );
    setting(&mut host, Protocol::Quake3_68, "sv_allowDownload", "1");
    // Stall the source before cancellation. Its retained ring records must not
    // be sent as blocks of the later transfer when the sink becomes writable.
    source.transport.reject = Some(address(client));
    queue(&mut host, client, b"download baseq3/custom.pk3");
    source.step(&mut host);
    queue(&mut host, client, b"stopdl");
    source.step(&mut host);
    queue(&mut host, client, b"download baseq3/no.pk3");
    source.step(&mut host);
    let before = source.numbered[0].len();
    source.transport.reject = None;
    source.advance(&mut host, 3);
    assert!(
        source.numbered[0][before..]
            .iter()
            .all(|chunk| matches!(chunk, FileChunk::Rejected { .. }))
    );
    // An out-of-order native block acknowledgement drops only that peer.
    queue(&mut host, client, b"download baseq3/custom.pk3");
    source.step(&mut host);
    queue(&mut host, client, b"nextdl 7");
    source.advance(&mut host, 2);
    assert!(host.runtime.server.clients[0].connection.is_none());
    assert!(host.runtime.server.clients[1].connection.is_some());
}

#[test]
fn q3_download_batches_follow_client_snaps_rate_and_live_server_rate_limit() {
    for (max_rate, expected) in [(0, 8), (1000, 1), (-1, 1)] {
        let (_files, mut host, mut source) = fixture([Protocol::Quake3_68; 2], false);
        q3_package(&mut host);
        host.runtime
            .server
            .replace_userinfo(
                ClientId(0),
                b"\\rate\\90000\\snaps\\1",
                Protocol::Quake3_68.userinfo_rules().unwrap(),
            )
            .unwrap();
        setting(
            &mut host,
            Protocol::Quake3_68,
            "sv_maxRate",
            &max_rate.to_string(),
        );
        queue(&mut host, ClientId(0), b"download baseq3/custom.pk3");
        source.advance(&mut host, 3);
        assert_eq!(source.numbered[0].len(), expected);
        source.advance(&mut host, 3);
        assert_eq!(source.numbered[0].len(), expected);
        if max_rate != 0 {
            setting(&mut host, Protocol::Quake3_68, "sv_maxRate", "0");
            source.advance(&mut host, 8);
            assert_eq!(source.numbered[0].len(), 8);
        }
    }
}
