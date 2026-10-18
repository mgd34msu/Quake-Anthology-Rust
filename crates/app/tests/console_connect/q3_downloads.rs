use super::*;
use qa_network::downloads::{self, Chunk};
#[path = "../../../formats/tests/support/stored_zip.rs"]
pub(super) mod zip;

fn command(host: &mut FrameHost, source: &mut Source, text: &str, time: u64) {
    host.console.append(text, context()).unwrap();
    frame(host, source, time);
}
fn bytes(host: &FrameHost, name: &[u8]) -> Option<Vec<u8>> {
    let file = host.runtime.vfs.open(name)?;
    Some(
        host.runtime
            .vfs
            .read_to_vec(&file, 65536, &mut Default::default())
            .unwrap(),
    )
}

#[test]
fn q3_udp_download_ack_cursors_survive_full_commands_and_publish_only_on_eof() {
    let mut files = support::Fixture::new(&[]);
    let mut host = host(2);
    host.runtime.vfs = std::mem::take(&mut files.vfs);
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    let mut source = source();
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, None, 0);
    let local = peer(&datagrams(&mut server)[0]);
    let mut native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        local,
        0,
    );
    host.configure_client_downloads(client, mount, mount, 8192, 128)
        .unwrap();
    setting(&mut host, "cl_allowDownload", "1");
    command(&mut host, &mut source, "download maps/late.bin\n", 30);
    collect(&mut server, &mut native, 30);
    assert_eq!(native.count(b"download maps/late.bin"), 1);
    let progress = |host: &FrameHost, name: &str| {
        let view = host
            .console
            .cvars
            .bind(
                name,
                Context {
                    role: qa_console::views::Role::Cgame,
                    ..context()
                },
            )
            .unwrap();
        host.console.cvars.read(view).unwrap().as_str().to_owned()
    };
    assert_eq!(progress(&host, "cl_downloadName"), "maps/late.bin");
    assert_eq!(progress(&host, "cl_downloadSize"), "0");
    assert_eq!(progress(&host, "cl_downloadCount"), "0");
    assert_eq!(progress(&host, "cl_downloadTime"), "30");
    let other = host
        .console
        .cvars
        .bind(
            "cl_downloadName",
            Context {
                seat: SeatId::new(1).unwrap(),
                ..context()
            },
        )
        .unwrap();
    assert_eq!(host.console.cvars.read(other).unwrap().as_str(), "");

    // Fill the sole reliable command window. A later block ACK must retain its
    // cursor when queue_client_command reports Full, without another byte queue.
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap()
        .channel;
    let mut held = 0;
    while channel
        .queue_client_command(Q3, format!("echo held{held}").as_bytes())
        .is_ok()
    {
        held += 1;
    }
    assert!(held > 32);
    let data: Vec<u8> = (0..4096).map(|index| (index * 29) as u8).collect();
    let packets = send(&mut server, socket, local, &mut native, 40, |writer, _| {
        for number in 0..4 {
            downloads::write_service(
                Q3,
                Chunk::Block {
                    number,
                    total: (number == 0).then_some(data.len() as u32),
                    bytes: &data[number as usize * 512..(number as usize + 1) * 512],
                },
                writer,
            )
            .unwrap();
        }
        Ok(())
    });
    assert!(packets > 1);
    frame(&mut host, &mut source, 40);
    collect(&mut server, &mut native, 40);
    assert_eq!(native.count(b"nextdl 0"), 1);
    assert_eq!(native.count(b"nextdl 1"), 0);
    assert_eq!(progress(&host, "cl_downloadSize"), "4096");
    assert_eq!(progress(&host, "cl_downloadCount"), "2048");
    assert_eq!(progress(&host, "cl_downloadTime"), "30");

    assert!(bytes(&host, b"maps/late.bin").is_none());
    // Native ACK frees the command window. The remaining ACKs are sent in order.
    send(&mut server, socket, local, &mut native, 50, |_, _| Ok(()));
    frame(&mut host, &mut source, 50);
    collect(&mut server, &mut native, 50);
    for number in 0..4 {
        assert_eq!(native.count(format!("nextdl {number}").as_bytes()), 1);
    }
    send(&mut server, socket, local, &mut native, 60, |writer, _| {
        // Duplicate and future blocks must not modify the file or ACK cursor.
        for number in [0, 7, 4, 5, 6, 7] {
            downloads::write_service(
                Q3,
                Chunk::Block {
                    number,
                    total: (number == 0).then_some(data.len() as u32),
                    bytes: &data[number as usize * 512..(number as usize + 1) * 512],
                },
                writer,
            )
            .unwrap();
        }
        Ok(())
    });
    frame(&mut host, &mut source, 60);
    collect(&mut server, &mut native, 60);
    assert!(bytes(&host, b"maps/late.bin").is_none());
    send(&mut server, socket, local, &mut native, 70, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 8,
                total: None,
                bytes: &[],
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 70);
    collect(&mut server, &mut native, 70);
    for number in 0..=8 {
        assert_eq!(native.count(format!("nextdl {number}").as_bytes()), 1);
    }
    assert_eq!(bytes(&host, b"maps/late.bin"), Some(data));
    assert_eq!(progress(&host, "cl_downloadName"), "");
    assert_eq!(progress(&host, "cl_downloadCount"), "4096");
    assert_eq!(progress(&host, "cl_downloadSize"), "4096");

    send(&mut server, socket, local, &mut native, 75, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 8,
                total: None,
                bytes: &[],
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 75);
    collect(&mut server, &mut native, 75);
    assert_eq!(native.count(b"stopdl"), 0);
    assert_eq!(native.count(b"nextdl 8"), 1);

    // A fresh transfer resets only its block cursors, not connection commands.
    command(&mut host, &mut source, "download maps/empty.bin\n", 80);
    collect(&mut server, &mut native, 80);
    assert_eq!(native.count(b"download maps/empty.bin"), 1);
    send(&mut server, socket, local, &mut native, 90, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 0,
                total: Some(0),
                bytes: &[],
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 90);
    collect(&mut server, &mut native, 90);
    assert_eq!(bytes(&host, b"maps/empty.bin"), Some(vec![]));
    assert_eq!(native.count(b"nextdl 0"), 2);
    assert_eq!(progress(&host, "cl_downloadName"), "");
    assert_eq!(progress(&host, "cl_downloadSize"), "0");
    assert_eq!(progress(&host, "cl_downloadCount"), "0");
    assert_eq!(progress(&host, "cl_downloadTime"), "80");

    command(&mut host, &mut source, "download maps/broken.bin\n", 100);
    collect(&mut server, &mut native, 100);
    send(&mut server, socket, local, &mut native, 110, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 0,
                total: Some(4),
                bytes: b"ab",
            },
            writer,
        )
        .unwrap();
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 1,
                total: None,
                bytes: &[],
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 110);
    collect(&mut server, &mut native, 110);
    assert!(bytes(&host, b"maps/broken.bin").is_none());
    assert_eq!(native.count(b"stopdl"), 1);
    command(&mut host, &mut source, "retrydownload\n", 120);
    collect(&mut server, &mut native, 120);
    assert_eq!(native.count(b"download maps/broken.bin"), 2);
    send(&mut server, socket, local, &mut native, 130, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 0,
                total: Some(4),
                bytes: b"abcd",
            },
            writer,
        )
        .unwrap();
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 1,
                total: None,
                bytes: &[],
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 130);
    collect(&mut server, &mut native, 130);
    assert_eq!(bytes(&host, b"maps/broken.bin"), Some(b"abcd".to_vec()));
    assert_eq!(host.local_clients[0], Some(client));
    let package = zip::stored_zip(&[(b"models/downloaded.bin", b"queued package member")]);
    assert_eq!(native.count(b"donedl"), 0);
    command(&mut host, &mut source, "download packs/late.pk3\n", 140);
    collect(&mut server, &mut native, 140);
    assert!(bytes(&host, b"models/downloaded.bin").is_none());
    send(&mut server, socket, local, &mut native, 150, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 0,
                total: Some(package.len() as u32),
                bytes: &package,
            },
            writer,
        )
        .unwrap();
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 1,
                total: None,
                bytes: &[],
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 150);
    collect(&mut server, &mut native, 150);
    assert_eq!(
        bytes(&host, b"models/downloaded.bin"),
        Some(b"queued package member".to_vec())
    );
    let reference = host.runtime.vfs.open(b"models/downloaded.bin").unwrap();
    let origin = host.runtime.vfs.origin(&reference).unwrap();
    assert_eq!(origin.kind, qa_content::vfs::MountKind::Pk3);
    assert!(origin.path.ends_with("packs/late.pk3"));
    frame(&mut host, &mut source, 155);
    collect(&mut server, &mut native, 155);
    assert_eq!(native.count(b"donedl"), 1);
    let done = native
        .strings
        .iter()
        .position(|text| text.as_slice() == b"donedl")
        .unwrap();
    assert!(
        native.strings[..done]
            .iter()
            .rev()
            .take(2)
            .any(|text| text.as_slice() == b"nextdl 1")
    );

    command(&mut host, &mut source, "download maps/cancel.bin\n", 160);
    collect(&mut server, &mut native, 160);
    assert_eq!(progress(&host, "cl_downloadName"), "maps/cancel.bin");
    send(&mut server, socket, local, &mut native, 170, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 0,
                total: Some(8),
                bytes: b"half",
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 170);
    collect(&mut server, &mut native, 170);
    assert_eq!(progress(&host, "cl_downloadCount"), "4");
    command(&mut host, &mut source, "stopdownload\n", 180);
    collect(&mut server, &mut native, 180);
    assert_eq!(progress(&host, "cl_downloadName"), "");
    assert!(bytes(&host, b"maps/cancel.bin").is_none());
    command(&mut host, &mut source, "retrydownload\n", 190);
    collect(&mut server, &mut native, 190);
    assert_eq!(progress(&host, "cl_downloadName"), "maps/cancel.bin");
    assert_eq!(progress(&host, "cl_downloadCount"), "0");
    assert_eq!(progress(&host, "cl_downloadSize"), "0");
    assert_eq!(progress(&host, "cl_downloadTime"), "190");
    command(&mut host, &mut source, "disconnect\n", 200);
    assert_eq!(progress(&host, "cl_downloadName"), "");
}

#[test]
fn q3_unsolicited_and_rejected_downloads_stay_scoped_to_the_receiving_client() {
    let mut host = host(2);
    let mut source = source();
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, None, 0);
    let local = peer(&datagrams(&mut server)[0]);
    let mut native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        local,
        0,
    );
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
                client: RuleSetId::Quake2,
                input: RuleSetId::Quake3,
                movement: RuleSetId::Quake,
                trace: RuleSetId::QuakeWorld,
            },
            Protocol::Quake2_34,
            0,
        )
        .unwrap();
    host.local_clients[1] = Some(healthy);
    send(&mut server, socket, local, &mut native, 30, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 0,
                total: Some(3),
                bytes: b"bad",
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 30);
    collect(&mut server, &mut native, 30);
    assert_eq!(native.count(b"stopdl"), 1);
    assert_eq!(host.local_clients[0], Some(client));
    send(&mut server, socket, local, &mut native, 40, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Rejected {
                size: -1,
                message: b"download disabled",
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 40);
    assert!(host.local_clients[0].is_none());
    let size = host
        .console
        .cvars
        .bind("cl_downloadSize", context())
        .unwrap();
    assert_eq!(host.console.cvars.integer_view(size).unwrap(), -1);
    let name = host
        .console
        .cvars
        .bind("cl_downloadName", context())
        .unwrap();
    assert_eq!(host.console.cvars.read(name).unwrap().as_str(), "");

    assert_eq!(host.local_clients[1], Some(healthy));
    assert!(
        host.runtime
            .network
            .get(healthy, Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
}

#[test]
fn q3_separate_remote_and_local_names_survive_cancel_retry_and_package_publication() {
    let mut files = support::Fixture::new(&[]);
    let mut host = host(1);
    host.runtime.vfs = std::mem::take(&mut files.vfs);
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    let mut source = source();
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, None, 0);
    let local = peer(&datagrams(&mut server)[0]);
    let mut native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        local,
        0,
    );
    host.configure_client_downloads(client, mount, mount, 8192, 128)
        .unwrap();
    setting(&mut host, "cl_allowDownload", "1");
    for destination in ["../custom.pk3", "/custom.pk3", "maps//custom.pk3"] {
        command(
            &mut host,
            &mut source,
            &format!("download baseq3/custom.pk3 \"{destination}\"\n"),
            20,
        );
    }
    collect(&mut server, &mut native, 20);
    assert_eq!(native.count(b"download baseq3/custom.pk3"), 0);
    let destination = format!(
        "downloads/{0}/{0}/{0}/custom.12345678.pk3",
        "local".repeat(30)
    );
    assert!(destination.len() > 320);
    command(
        &mut host,
        &mut source,
        &format!("download baseq3/custom.pk3 {destination}\n"),
        30,
    );
    collect(&mut server, &mut native, 30);
    assert_eq!(native.count(b"download baseq3/custom.pk3"), 1);
    let progress = host
        .console
        .cvars
        .bind("cl_downloadName", context())
        .unwrap();
    assert_eq!(
        host.console.cvars.read(progress).unwrap().as_str(),
        "baseq3/custom.pk3"
    );
    let package = zip::stored_zip(&[(b"models/aliased.bin", b"retained package member")]);
    send(&mut server, socket, local, &mut native, 40, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 0,
                total: Some(package.len() as u32),
                bytes: &package[..8],
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 40);
    collect(&mut server, &mut native, 40);
    command(&mut host, &mut source, "downloadstatus\n", 45);
    assert!(
        host.console
            .scrollback
            .chunks(&host.runtime.server.events.texts)
            .any(|text| text
                .windows(destination.len())
                .any(|part| part == destination.as_bytes()))
    );
    command(&mut host, &mut source, "stopdownload\n", 50);
    collect(&mut server, &mut native, 50);
    assert!(bytes(&host, destination.as_bytes()).is_none());
    assert!(bytes(&host, b"baseq3/custom.pk3").is_none());
    command(&mut host, &mut source, "retrydownload\n", 60);
    collect(&mut server, &mut native, 60);
    assert_eq!(native.count(b"download baseq3/custom.pk3"), 2);
    assert_eq!(native.count(b"download custom.pk3"), 0);
    send(&mut server, socket, local, &mut native, 70, |writer, _| {
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 0,
                total: Some(package.len() as u32),
                bytes: &package,
            },
            writer,
        )
        .unwrap();
        downloads::write_service(
            Q3,
            Chunk::Block {
                number: 1,
                total: None,
                bytes: &[],
            },
            writer,
        )
        .unwrap();
        Ok(())
    });
    frame(&mut host, &mut source, 70);
    collect(&mut server, &mut native, 70);
    frame(&mut host, &mut source, 75);
    collect(&mut server, &mut native, 75);
    assert_eq!(bytes(&host, destination.as_bytes()), Some(package.clone()));
    assert_eq!(
        bytes(&host, b"models/aliased.bin"),
        Some(b"retained package member".to_vec())
    );
    assert!(bytes(&host, b"baseq3/custom.pk3").is_none());
    assert_eq!(native.count(b"donedl"), 1);
    assert!(
        host.console
            .cvars
            .read(progress)
            .unwrap()
            .as_str()
            .is_empty()
    );
    command(
        &mut host,
        &mut source,
        &format!("download baseq3/other.pk3 {destination}\n"),
        80,
    );
    collect(&mut server, &mut native, 80);
    assert_eq!(native.count(b"download baseq3/other.pk3"), 0);
    assert_eq!(bytes(&host, destination.as_bytes()), Some(package));
}

#[test]
fn q3_bad_package_stays_unpublished_and_retry_can_complete_the_same_destination() {
    let mut files = support::Fixture::new(&[]);
    let mut host = host(1);
    host.runtime.vfs = std::mem::take(&mut files.vfs);
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    let root = host
        .runtime
        .vfs
        .directory_path(mount)
        .unwrap()
        .to_path_buf();
    let mut source = source();
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, None, 0);
    let local = peer(&datagrams(&mut server)[0]);
    let mut native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        local,
        0,
    );
    host.configure_client_downloads(client, mount, mount, 8192, 128)
        .unwrap();
    setting(&mut host, "cl_allowDownload", "1");
    command(
        &mut host,
        &mut source,
        "download baseq3/custom.pk3 custom.pk3\n",
        20,
    );
    collect(&mut server, &mut native, 20);
    for (attempt, package) in [
        b"not a native archive".to_vec(),
        zip::stored_zip(&[(b"../outside", b"bad member path")]),
        b"PACK\x0c\0\0\0\0\0\0\0".to_vec(),
        zip::stored_zip(&[(b"models/kept", b"accepted package")]),
    ]
    .into_iter()
    .enumerate()
    {
        let time = 30 + attempt as u64 * 20;
        send(
            &mut server,
            socket,
            local,
            &mut native,
            time,
            |writer, _| {
                downloads::write_service(
                    Q3,
                    Chunk::Block {
                        number: 0,
                        total: Some(package.len() as u32),
                        bytes: &package,
                    },
                    writer,
                )
                .unwrap();
                downloads::write_service(
                    Q3,
                    Chunk::Block {
                        number: 1,
                        total: None,
                        bytes: &[],
                    },
                    writer,
                )
                .unwrap();
                Ok(())
            },
        );
        frame(&mut host, &mut source, time);
        collect(&mut server, &mut native, time);
        if attempt < 3 {
            assert!(!root.join("custom.pk3").exists());
            assert!(bytes(&host, b"custom.pk3").is_none());
            assert!(bytes(&host, b"models/kept").is_none());
            assert_eq!(native.count(b"donedl"), 0);
            assert!(
                qa_platform::files::directory_entries(&root)
                    .unwrap()
                    .is_empty()
            );
            command(&mut host, &mut source, "retrydownload\n", time + 10);
            collect(&mut server, &mut native, time + 10);
        } else {
            frame(&mut host, &mut source, time + 5);
            collect(&mut server, &mut native, time + 5);
            assert_eq!(bytes(&host, b"custom.pk3"), Some(package));
            assert_eq!(
                bytes(&host, b"models/kept"),
                Some(b"accepted package".to_vec())
            );
            assert_eq!(native.count(b"donedl"), 1);
        }
    }
    assert_eq!(native.count(b"download baseq3/custom.pk3"), 4);
}

#[test]
fn q3_referenced_packages_share_cancel_retry_and_publish_only_the_advertised_versions() {
    let mut files = support::Fixture::new(&[]);
    let parent = files.vfs.mounts().next().unwrap().id;
    let root = files.vfs.directory_path(parent).unwrap().join("baseq3");
    qa_platform::files::create_dir_all(&root).unwrap();
    let original = zip::stored_zip(&[(b"models/original.bin", b"original package")]);
    let loaded = zip::stored_zip(&[(b"models/loaded.bin", b"already loaded")]);
    qa_platform::files::write(root.join("first.pk3"), &original).unwrap();
    qa_platform::files::write(root.join("loaded.pk3"), &loaded).unwrap();
    let game = files.vfs.mount_product(&root, 100).unwrap()[0];
    let checksum = |package: &[u8]| {
        let mut staged =
            qa_platform::files::StagedFile::create(&root, "reference.tmp", 8192, 128).unwrap();
        staged.append(package).unwrap();
        let parsed =
            qa_formats::archive::Archive::parse(std::sync::Arc::new(staged.reader().unwrap()))
                .unwrap();
        parsed.q3_checksums().unwrap().checksum()
    };
    let first = zip::stored_zip(&[(b"models/first.bin", b"first wanted version")]);
    let second = zip::stored_zip(&[(b"models/second.bin", b"second wanted version")]);
    let first_sum = checksum(&first);
    let second_sum = checksum(&second);
    let loaded_sum = checksum(&loaded);
    let mut host = host(1);
    host.runtime.vfs = std::mem::take(&mut files.vfs);
    host.configure_download_roots(game, game, 8192, 128)
        .unwrap();
    setting(&mut host, "cl_allowDownload", "1");
    let mut source = source();
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, None, 0);
    let local = peer(&datagrams(&mut server)[0]);
    let mut native = accept(
        &mut host,
        &mut source,
        SeatId::FIRST,
        &mut server,
        socket,
        local,
        0,
    );
    native
        .channel
        .set_command_context(CommandContext {
            server_id: SERVER_ID,
            challenge: CHALLENGE as u32,
            checksum_feed: FEED,
        })
        .unwrap();
    let info = format!(
        "\\sv_serverid\\{SERVER_ID}\\sv_pure\\0\\sv_referencedPaks\\{} 1 {} {} {}\\sv_referencedPakNames\\baseq3/elsewhere baseq3/pak0 baseq3/first baseq3/second baseq3/first",
        loaded_sum as i32, first_sum as i32, second_sum as i32, first_sum as i32,
    );
    let watermark = native.channel.command_state().unwrap().queued;
    send(&mut server, socket, local, &mut native, 30, |writer, _| {
        write_gamestate(
            writer,
            watermark,
            0,
            FEED,
            [(0, b"\\mapname\\ready".as_slice()), (1, info.as_bytes())],
            std::iter::empty(),
        )?;
        Ok(())
    });
    frame(&mut host, &mut source, 30);
    collect(&mut server, &mut native, 30);
    assert_eq!(native.count(b"download baseq3/first.pk3"), 1);
    assert_eq!(native.count(b"download baseq3/elsewhere.pk3"), 0);
    assert_eq!(native.count(b"download baseq3/pak0.pk3"), 0);
    let mut deliver = |host: &mut FrameHost,
                       source: &mut Source,
                       native: &mut NativePeer,
                       package: &[u8],
                       time| {
        send(&mut server, socket, local, native, time, |writer, _| {
            downloads::write_service(
                Q3,
                Chunk::Block {
                    number: 0,
                    total: Some(package.len() as u32),
                    bytes: package,
                },
                writer,
            )
            .unwrap();
            downloads::write_service(
                Q3,
                Chunk::Block {
                    number: 1,
                    total: None,
                    bytes: &[],
                },
                writer,
            )
            .unwrap();
            Ok(())
        });
        for time in time..time + 3 {
            frame(host, source, time);
            collect(&mut server, native, time);
        }
    };
    // This archive is structurally valid but is not the advertised version.
    deliver(&mut host, &mut source, &mut native, &second, 40);
    let alias = format!("first.{first_sum:08x}.pk3");
    assert!(!root.join(&alias).exists());
    assert!(bytes(&host, b"models/first.bin").is_none());
    assert_eq!(native.count(b"download baseq3/second.pk3"), 0);
    assert_eq!(native.count(b"donedl"), 0);
    command(&mut host, &mut source, "retrydownload\n", 50);
    deliver(&mut host, &mut source, &mut native, &first, 60);
    assert_eq!(bytes(&host, alias.as_bytes()), Some(first.clone()));
    assert_eq!(bytes(&host, b"first.pk3"), Some(original));
    assert_eq!(native.count(b"download baseq3/first.pk3"), 2);
    assert_eq!(native.count(b"download baseq3/second.pk3"), 1);
    assert_eq!(native.count(b"donedl"), 0);
    command(&mut host, &mut source, "stopdownload\n", 70);
    command(&mut host, &mut source, "retrydownload\n", 80);
    deliver(&mut host, &mut source, &mut native, &second, 90);
    assert_eq!(bytes(&host, b"second.pk3"), Some(second));
    assert_eq!(
        bytes(&host, b"models/first.bin"),
        Some(b"first wanted version".to_vec())
    );
    assert_eq!(
        bytes(&host, b"models/second.bin"),
        Some(b"second wanted version".to_vec())
    );
    assert_eq!(native.count(b"download baseq3/second.pk3"), 2);
    assert_eq!(native.count(b"donedl"), 1);
    send(&mut server, socket, local, &mut native, 100, |writer, _| {
        write_gamestate(
            writer,
            watermark,
            0,
            FEED,
            [(0, b"\\mapname\\ready".as_slice()), (1, info.as_bytes())],
            std::iter::empty(),
        )?;
        Ok(())
    });
    frame(&mut host, &mut source, 100);
    collect(&mut server, &mut native, 100);
    assert_eq!(native.count(b"download baseq3/first.pk3"), 2);
    assert_eq!(native.count(b"download baseq3/second.pk3"), 2);
    assert_eq!(native.count(b"donedl"), 1);
    assert_eq!(host.local_clients[0], Some(client));
    assert!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
}
