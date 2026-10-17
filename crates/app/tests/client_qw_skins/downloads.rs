use super::*;
use qa_content::vfs::{MountId, MountKind};
use qa_network::downloads::{Chunk, write_chunk};

fn root(host: &FrameHost) -> (MountId, std::path::PathBuf) {
    let mount = host
        .runtime
        .vfs
        .mounts()
        .find(|mount| mount.kind == MountKind::Directory)
        .unwrap();
    (mount.id, mount.path.clone())
}

fn configure(host: &mut FrameHost, client: ClientId, maximum_bytes: u64) {
    let (mount, _) = root(host);
    host.configure_client_downloads(client, mount, mount, maximum_bytes, 64)
        .unwrap();
}

fn command(host: &mut FrameHost, source: &mut Source, seat: SeatId, text: &str) {
    let client = host.local_clients[seat.index()].unwrap();
    let source_rules = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .protocol
        .rule_set();
    host.console
        .append(
            text,
            Context {
                source: source_rules,
                seat,
                ..Default::default()
            },
        )
        .unwrap();
    host.execute_commands_with_driver(&mut qa_app::modules::Driver::cache_only());
    step(host, source);
}

fn step(host: &mut FrameHost, source: &mut Source) {
    assert_eq!(host.frame(source, true).drains, 2);
}

fn received(server: &NativeServer, text: &[u8]) -> usize {
    server
        .received_commands
        .iter()
        .filter(|command| command.as_slice() == text)
        .count()
}

fn chunk(host: &mut FrameHost, source: &mut Source, server: usize, chunk: Chunk<'_>) {
    let protocol = source.servers[server].commands.protocol;
    let mut bytes = [0; 1100];
    let length = write_chunk(protocol, chunk, &mut bytes).unwrap();
    send(host, &mut source.servers[server], &bytes[..length]);
}

fn file(host: &FrameHost, path: &[u8]) -> Option<Vec<u8>> {
    let reference = host.runtime.vfs.open(path)?;
    Some(
        host.runtime
            .vfs
            .read_to_vec(
                &reference,
                1024 * 1024,
                &mut qa_formats::archive::ArchiveReader::default(),
            )
            .unwrap(),
    )
}

fn skin_files(host: &FrameHost) -> Vec<std::ffi::OsString> {
    qa_platform::files::directory_entries(root(host).1.join("skins"))
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

#[test]
fn qw_download_publishes_only_the_complete_file_and_refreshes_the_selected_skin() {
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(false), &[], false);
    colored_peers(&mut host, &mut source);
    prepare(&mut host, &mut source);
    configure(&mut host, CLIENT, 4096);
    command(&mut host, &mut source, SeatId::FIRST, "allskins remote\n");
    let before = draw(&host, true);
    assert_eq!((before[0].first, before[1].first), (48, 80));

    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/remote.pcx\n",
    );
    assert_eq!(
        received(&source.servers[0], b"download skins/remote.pcx"),
        1
    );
    let bytes = support::pcx_dimensions(4, 2, 18);
    assert!(bytes.len() > 768 && bytes.len() <= 2 * 768);
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 50,
            bytes: Some(&bytes[..768]),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(received(&source.servers[0], b"nextdl"), 1);
    assert!(file(&host, b"skins/remote.pcx").is_none());
    assert!(qa_platform::files::read(root(&host).1.join("skins/remote.pcx")).is_err());
    assert_eq!(draw(&host, true), before);

    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(&bytes[768..]),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(file(&host, b"skins/remote.pcx").unwrap(), bytes);
    let after = draw(&host, true);
    assert_eq!((after[0].first, after[1].first), (34, 66));
    assert_eq!(after[0].size, [320, 200]);
    assert_eq!(after[2], before[2]);
    assert_eq!(received(&source.servers[0], b"nextdl"), 1);
    assert_eq!(
        received(&source.servers[0], b"download skins/remote.pcx"),
        1
    );
}

#[test]
fn cancel_and_retry_drain_the_old_reply_before_any_new_destination_can_start() {
    let (_fixture, mut host, mut source) = loading();
    configure(&mut host, CLIENT, 4096);
    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/cancelled.pcx\n",
    );
    assert_eq!(
        received(&source.servers[0], b"download skins/cancelled.pcx"),
        1
    );
    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "stopdownload\ndownload skins/new.pcx\nretrydownload\n",
    );
    assert_eq!(received(&source.servers[0], b"download skins/new.pcx"), 0);
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"delayed old reply"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(
        received(&source.servers[0], b"download skins/cancelled.pcx"),
        2
    );
    assert!(file(&host, b"skins/cancelled.pcx").is_none());
    assert!(file(&host, b"skins/new.pcx").is_none());
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"retry contents"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(
        file(&host, b"skins/cancelled.pcx").unwrap(),
        b"retry contents"
    );

    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/new.pcx\n",
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"new destination"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(file(&host, b"skins/new.pcx").unwrap(), b"new destination");
    assert_eq!(received(&source.servers[0], b"download skins/new.pcx"), 1);
}

#[test]
fn a_full_native_reliable_queue_retains_the_download_request_until_acknowledged_space() {
    let (_fixture, mut host, mut source) = loading();
    configure(&mut host, CLIENT, 4096);
    let connection = host
        .runtime
        .network
        .get_mut(CLIENT, Endpoint::Client)
        .unwrap();
    let mut queued = 0;
    while connection
        .channel
        .queue_client_command(Protocol::QuakeWorld28, b"pressure")
        .is_ok()
    {
        queued += 1;
        assert!(queued < 1024);
    }
    assert!(queued > 0);
    let rejected = connection.channel.counts().client_command_rejected;
    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/queued.pcx\n",
    );
    assert!(
        host.runtime
            .network
            .get(CLIENT, Endpoint::Client)
            .unwrap()
            .channel
            .counts()
            .client_command_rejected
            > rejected
    );
    for _ in 0..8 {
        if received(&source.servers[0], b"download skins/queued.pcx") == 1 {
            break;
        }
        step(&mut host, &mut source);
    }
    assert_eq!(received(&source.servers[0], b"pressure"), queued);
    assert_eq!(
        received(&source.servers[0], b"download skins/queued.pcx"),
        1
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"accepted after pressure"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(
        file(&host, b"skins/queued.pcx").unwrap(),
        b"accepted after pressure"
    );
}

#[test]
fn malformed_native_block_cleans_the_partial_sink_and_does_not_stall_either_client() {
    let (_fixture, mut host, mut source) = loading();
    let other = add_q2_client(&mut host, &mut source, Protocol::Quake2_34);
    configure(&mut host, CLIENT, 4096);
    let files = skin_files(&host);
    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/malformed.pcx\n",
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 25,
            bytes: Some(b"partial contents"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(skin_files(&host).len(), files.len() + 1);
    assert!(file(&host, b"skins/malformed.pcx").is_none());

    let mut malformed = [0; 16];
    let length = write_chunk(
        Protocol::QuakeWorld28,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b""),
        },
        &mut malformed,
    )
    .unwrap();
    malformed[length - 1] = 101;
    send(&mut host, &mut source.servers[0], &malformed[..length]);
    step(&mut host, &mut source);
    assert_eq!(
        skin_files(&host),
        files,
        "malformed framing drops its staged inode"
    );
    assert!(file(&host, b"skins/malformed.pcx").is_none());
    assert!(
        host.runtime
            .network
            .get(CLIENT, Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );

    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/recovered.pcx\n",
    );
    assert_eq!(
        received(&source.servers[0], b"download skins/recovered.pcx"),
        1
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"healthy transfer"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(
        file(&host, b"skins/recovered.pcx").unwrap(),
        b"healthy transfer"
    );
    let mut bytes = [0; 64];
    let length =
        outputs::configstring(Protocol::Quake2_34, 0, b"still healthy", &mut bytes).unwrap();
    send(&mut host, &mut source.servers[1], &bytes[..length]);
    assert_eq!(
        host.runtime.client_resources[other.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(0)
            .unwrap()
            .text,
        b"still healthy"
    );
}

fn add_q2_client(host: &mut FrameHost, source: &mut Source, protocol: Protocol) -> ClientId {
    let client = ClientId(1);
    let server = NativeServer::load(protocol, client);
    let mut channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
    channel.set_qport(27001);
    assert_eq!(
        host.runtime
            .connect_client(
                SeatId::ALL[1],
                ClientPolicy {
                    input: protocol.rule_set(),
                    client: protocol.rule_set(),
                    movement: RuleSetId::Quake3,
                    trace: RuleSetId::Quake,
                },
                NativePlayerBinding::Supplied(0),
                Connection {
                    phase: ConnectionPhase::Connected,
                    route: Route {
                        socket: SOCKET,
                        peer: Peer::Socket(server.peer)
                    },
                    channel,
                    output: None,
                    pending_output: None,
                    commands: Some(Commands::load(protocol, 8192)),
                },
                None,
            )
            .unwrap(),
        client
    );
    host.local_clients[1] = Some(client);
    source.servers.push(server);
    client
}

#[test]
fn unavailable_and_oversized_qw_transfers_leave_the_q2_peer_and_empty_completion_healthy() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        mixed_clients(protocol);
    }
}

fn mixed_clients(protocol: Protocol) {
    let (_fixture, mut host, mut source) = loading();
    let other = add_q2_client(&mut host, &mut source, protocol);
    let [allow_download, _] = ["cl_allowDownload", "allow_download_players"].map(|name| {
        let view = host
            .console
            .cvars
            .bind(
                name,
                Context {
                    source: protocol.rule_set(),
                    side: qa_console::catalog::Scope::Client,
                    seat: SeatId::ALL[1],
                    ..Default::default()
                },
            )
            .unwrap();
        host.console.cvars.force_write(view, "1").unwrap();
        view
    });
    configure(&mut host, CLIENT, 4);
    configure(&mut host, other, 4);
    command(
        &mut host,
        &mut source,
        SeatId::ALL[1],
        "download skins/family.pcx\n",
    );
    assert_eq!(
        received(&source.servers[1], b"download skins/family.pcx"),
        1
    );
    chunk(
        &mut host,
        &mut source,
        1,
        Chunk {
            compressed: false,
            percent: 50,
            bytes: Some(b"12"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(received(&source.servers[1], b"nextdl"), 1);
    assert!(file(&host, b"skins/family.pcx").is_none());
    chunk(
        &mut host,
        &mut source,
        1,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"34"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(file(&host, b"skins/family.pcx").unwrap(), b"1234");
    command(
        &mut host,
        &mut source,
        SeatId::ALL[1],
        "download skins/denied-retry.pcx\n",
    );
    chunk(
        &mut host,
        &mut source,
        1,
        Chunk {
            compressed: false,
            percent: 0,
            bytes: None,
        },
    );
    step(&mut host, &mut source);
    host.console.cvars.force_write(allow_download, "0").unwrap();
    command(&mut host, &mut source, SeatId::ALL[1], "retrydownload\n");
    assert_eq!(
        received(&source.servers[1], b"download skins/denied-retry.pcx"),
        1
    );
    assert!(file(&host, b"skins/denied-retry.pcx").is_none());
    host.console.cvars.force_write(allow_download, "1").unwrap();
    command(&mut host, &mut source, SeatId::ALL[1], "retrydownload\n");
    assert_eq!(
        received(&source.servers[1], b"download skins/denied-retry.pcx"),
        2
    );
    chunk(
        &mut host,
        &mut source,
        1,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"okay"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(file(&host, b"skins/denied-retry.pcx").unwrap(), b"okay");
    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/unavailable.pcx\n",
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 0,
            bytes: None,
        },
    );
    step(&mut host, &mut source);
    assert!(file(&host, b"skins/unavailable.pcx").is_none());
    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/too-large.pcx\n",
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"12345"),
        },
    );
    step(&mut host, &mut source);
    assert!(file(&host, b"skins/too-large.pcx").is_none());
    assert!(
        host.runtime
            .network
            .get(CLIENT, Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );

    command(
        &mut host,
        &mut source,
        SeatId::ALL[1],
        "download skins/empty.pcx\n",
    );
    assert_eq!(received(&source.servers[1], b"download skins/empty.pcx"), 1);
    chunk(
        &mut host,
        &mut source,
        1,
        Chunk {
            compressed: false,
            percent: 0,
            bytes: Some(b""),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(file(&host, b"skins/empty.pcx").unwrap(), b"");
    assert_eq!(received(&source.servers[1], b"nextdl"), 1);
    let mut bytes = [0; 64];
    let length = outputs::configstring(protocol, 0, b"healthy peer", &mut bytes).unwrap();
    send(&mut host, &mut source.servers[1], &bytes[..length]);
    assert_eq!(
        host.runtime.client_resources[other.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(0)
            .unwrap()
            .text,
        b"healthy peer"
    );
    assert!(
        host.runtime
            .network
            .get(other, Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
}

fn set_pair_skin(host: &mut FrameHost, source: &mut Source, skin: &[u8]) {
    peers(
        host,
        source,
        &[
            PeerUpdate::SetInfo {
                slot: 0,
                key: b"skin",
                value: skin,
            },
            PeerUpdate::SetInfo {
                slot: 1,
                key: b"skin",
                value: skin,
            },
        ],
    );
}

#[test]
fn automatic_skin_pass_preserves_manual_priority_deduplicates_peers_and_reuses_refresh() {
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(false), &[], false);
    colored_peers(&mut host, &mut source);
    prepare(&mut host, &mut source);
    configure(&mut host, CLIENT, 4096);
    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/manual.bin\n",
    );
    assert_eq!(
        received(&source.servers[0], b"download skins/manual.bin"),
        1
    );
    set_pair_skin(&mut host, &mut source, b"automatic");
    step(&mut host, &mut source);
    assert_eq!(
        received(&source.servers[0], b"download skins/automatic.pcx"),
        0
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(b"manual completion"),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(
        received(&source.servers[0], b"download skins/automatic.pcx"),
        1
    );
    let before = draw(&host, true);
    assert_eq!((before[0].first, before[1].first), (48, 80));
    let bytes = support::pcx_dimensions(4, 2, 18);
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 50,
            bytes: Some(&bytes[..768]),
        },
    );
    step(&mut host, &mut source);
    assert!(file(&host, b"skins/automatic.pcx").is_none());
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 100,
            bytes: Some(&bytes[768..]),
        },
    );
    step(&mut host, &mut source);
    assert_eq!(file(&host, b"skins/automatic.pcx").unwrap(), bytes);
    let after = draw(&host, true);
    assert_eq!((after[0].first, after[1].first), (34, 66));
    assert_eq!(after[2], before[2]);
    for _ in 0..4 {
        step(&mut host, &mut source);
    }
    assert_eq!(
        received(&source.servers[0], b"download skins/automatic.pcx"),
        1
    );
    assert_eq!(received(&source.servers[0], b"nextdl"), 1);
}

#[test]
fn automatic_refusals_wait_for_an_explicit_pass_and_native_noskins_controls_requests() {
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(false), &[], false);
    colored_peers(&mut host, &mut source);
    prepare(&mut host, &mut source);
    configure(&mut host, CLIENT, 4096);
    set_pair_skin(&mut host, &mut source, b"refused");
    step(&mut host, &mut source);
    assert_eq!(
        received(&source.servers[0], b"download skins/refused.pcx"),
        1
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 0,
            bytes: None,
        },
    );
    for _ in 0..4 {
        step(&mut host, &mut source);
    }
    assert_eq!(
        received(&source.servers[0], b"download skins/refused.pcx"),
        1
    );
    command(&mut host, &mut source, SeatId::FIRST, "skins\n");
    assert_eq!(
        received(&source.servers[0], b"download skins/refused.pcx"),
        2
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 0,
            bytes: None,
        },
    );
    setting(&mut host, "cl_noskins", "1");
    set_pair_skin(&mut host, &mut source, b"blocked");
    step(&mut host, &mut source);
    assert_eq!(
        received(&source.servers[0], b"download skins/blocked.pcx"),
        0
    );
    setting(&mut host, "cl_noskins", "2");
    step(&mut host, &mut source);
    assert_eq!(
        received(&source.servers[0], b"download skins/blocked.pcx"),
        0
    );
    setting(&mut host, "cl_noskins", "0");
    step(&mut host, &mut source);
    assert_eq!(
        received(&source.servers[0], b"download skins/blocked.pcx"),
        1
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 0,
            bytes: None,
        },
    );
}

#[test]
fn explicit_skin_pass_can_supersede_a_terminal_manual_failure() {
    let (_fixture, mut host, mut source) = loading_model(RULES, colored_mdl(false), &[], false);
    colored_peers(&mut host, &mut source);
    prepare(&mut host, &mut source);
    configure(&mut host, CLIENT, 4096);
    command(
        &mut host,
        &mut source,
        SeatId::FIRST,
        "download skins/manual-miss.bin\n",
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 0,
            bytes: None,
        },
    );
    set_pair_skin(&mut host, &mut source, b"explicit");
    step(&mut host, &mut source);
    assert_eq!(
        received(&source.servers[0], b"download skins/explicit.pcx"),
        0,
        "passive peer changes retain the manual retry target"
    );
    command(&mut host, &mut source, SeatId::FIRST, "skins\n");
    assert_eq!(
        received(&source.servers[0], b"download skins/explicit.pcx"),
        1
    );
    chunk(
        &mut host,
        &mut source,
        0,
        Chunk {
            compressed: false,
            percent: 0,
            bytes: None,
        },
    );
}
