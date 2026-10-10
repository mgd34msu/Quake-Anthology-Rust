//! Native SERVER route fixtures and heap-only socket-address routing checks.
use qa_core::{
    loopback::Endpoint,
    primitives::ClientId,
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::{self, Channel},
    headers::{self, Direction, Format, Header, QPort},
    ingress::{Connection, Connections, Incoming, Route},
    message::{Encoding, Reader},
};
use qa_platform::allocations;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr},
};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn policy(mode: u8) -> Result<(channel::Policy, Format), String> {
    Ok(match mode {
        0 => (channel::QUAKEWORLD, headers::QUAKEWORLD),
        1 => (channel::QUAKE2, headers::QUAKE2),
        2 => (channel::QUAKE3, headers::QUAKE3),
        3 => (channel::q2_new(true), headers::q2_new(true)),
        4 => (channel::q2_new(false), headers::q2_new(false)),
        5 => (channel::q2_old(QPort::Byte), headers::q2_old(QPort::Byte)),
        6 => (channel::q2_old(QPort::None), headers::q2_old(QPort::None)),
        _ => return Err("fixture policy".into()),
    })
}
fn bind(
    peers: &mut Connections,
    mode: u8,
    slot: usize,
    address: SocketAddr,
    qport: u16,
) -> Result<(), String> {
    let mut channel =
        Channel::load(policy(mode)?.0, Endpoint::Server, 8192, 16).map_err(|e| e.to_string())?;
    channel.set_qport(qport);
    peers
        .bind(
            ClientId(slot as u32),
            Endpoint::Server,
            Connection {
                route: Route {
                    socket: 7,
                    peer: Peer::Socket(address),
                },
                channel,
                commands: None,
                output: None,
            },
        )
        .map_err(|e| format!("bind {e:?}"))
}
fn ip(reader: &mut Reader<'_>) -> Result<Ipv4Addr, String> {
    let mut bytes = [0; 4];
    reader.read_data(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.into())
}
fn oracle() -> Result<(), String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let mut reader = Reader::new(&bytes, Encoding::Bytes);
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut cases = 0;
    while reader.byte_position() < bytes.len() {
        let mode = reader.read_bits(8).map_err(|e| e.to_string())? as u8;
        let addresses = [ip(&mut reader)?, ip(&mut reader)?];
        let ports = [
            reader.read_bits(16).map_err(|e| e.to_string())? as u16,
            reader.read_bits(16).map_err(|e| e.to_string())? as u16,
        ];
        let qports = [
            reader.read_bits(16).map_err(|e| e.to_string())? as u16,
            reader.read_bits(16).map_err(|e| e.to_string())? as u16,
        ];
        let from = ip(&mut reader)?;
        let from_port = reader.read_bits(16).map_err(|e| e.to_string())? as u16;
        let length = reader.read_bits(16).map_err(|e| e.to_string())? as usize;
        let mut packet = [0; 1400];
        reader
            .read_data(packet.get_mut(..length).ok_or("packet length")?)
            .map_err(|e| e.to_string())?;
        let mut peers = Connections::load(2);
        for slot in 0..2 {
            bind(
                &mut peers,
                mode,
                slot,
                SocketAddr::from((addresses[slot], ports[slot])),
                qports[slot],
            )?;
        }
        let mut selected = u32::MAX;
        allocations::begin_frame();
        peers.receive(
            7,
            SocketAddr::from((from, from_port)),
            &packet[..length],
            EventTime(1),
            |client, _, incoming| {
                if matches!(incoming, Incoming::Payload(_)) {
                    selected = client.0;
                }
            },
        );
        let heap = allocations::end_frame();
        if heap.allocations != 0 || heap.reallocations != 0 || heap.requested_bytes != 0 {
            return Err("routing heap".into());
        }
        out.write_all(&selected.to_le_bytes())
            .map_err(|e| e.to_string())?;
        for slot in 0..2 {
            let Peer::Socket(address) = peers
                .get(ClientId(slot), Endpoint::Server)
                .ok_or("peer")?
                .route
                .peer
            else {
                return Err("socket".into());
            };
            out.write_all(&address.port().to_le_bytes())
                .map_err(|e| e.to_string())?;
        }
        cases += 1;
    }
    out.flush().map_err(|e| e.to_string())?;
    eprintln!("{{\"cases\":{cases},\"allocations\":0,\"timing_run\":false}}");
    Ok(())
}
fn heap() -> Result<(), String> {
    let mut peers = Connections::load(14);
    for mode in 0..7 {
        for seat in 0..2 {
            let qport = if matches!(mode, 4 | 6) {
                0
            } else {
                17 + seat as u16
            };
            bind(
                &mut peers,
                mode,
                mode as usize * 2 + seat,
                SocketAddr::from(([127, 0, 0, mode + 1], 20000 + seat as u16)),
                qport,
            )?;
        }
    }
    allocations::begin_frame();
    let positive = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&positive);
    drop(positive);
    if allocations::end_frame().allocations != 1 {
        return Err("allocator positive control".into());
    }
    let mut totals = allocations::Counts::default();
    let mut delivered = 0;
    for frame in 0..660 {
        allocations::begin_frame();
        for mode in 0..7 {
            for seat in 0..2 {
                let slot = mode as usize * 2 + seat;
                let qport = if matches!(mode, 4 | 6) {
                    0
                } else {
                    17 + seat as u16
                };
                let port = if matches!(mode, 4 | 6) {
                    20000 + seat as u16
                } else {
                    21000 + frame as u16
                };
                let mut packet = [0; 1400];
                let length = headers::encode(
                    policy(mode)?.1,
                    Direction::ToServer,
                    Header {
                        sequence: frame + 1,
                        qport,
                        ..Default::default()
                    },
                    b"native",
                    &mut packet,
                )
                .map_err(|e| e.to_string())?;
                let mut matched = None;
                peers.receive(
                    7,
                    SocketAddr::from(([127, 0, 0, mode + 1], port)),
                    &packet[..length],
                    EventTime(frame as u64),
                    |client, _, incoming| {
                        if matches!(incoming, Incoming::Payload(_)) {
                            matched = Some(client);
                        }
                    },
                );
                if matched != Some(ClientId(slot as u32)) {
                    return Err("route identity".into());
                }
                delivered += 1;
            }
        }
        let counts = allocations::end_frame();
        if frame >= 60 {
            totals.allocations += counts.allocations;
            totals.reallocations += counts.reallocations;
            totals.requested_bytes += counts.requested_bytes;
        }
    }
    if totals.allocations != 0 || totals.reallocations != 0 || totals.requested_bytes != 0 {
        return Err("measured routing heap".into());
    }
    println!(
        "{{\"scope\":\"SERVER socket address/qport routing over 14 persistent peers, no OS sockets/gameplay/native signon; caller Rust heap\",\"warmup\":60,\"frames\":600,\"delivered_including_warmup\":{delivered},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"timing_run\":false}}",
        totals.allocations, totals.reallocations, totals.requested_bytes
    );
    Ok(())
}
fn main() -> Result<(), String> {
    if std::env::args().any(|arg| arg == "--heap-only") {
        heap()
    } else {
        oracle()
    }
}
