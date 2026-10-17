use super::*;
use qa_network::downloads::{self, Chunk};
#[path = "../../../formats/tests/support/stored_zip.rs"]
mod zip;

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
