//! Native original-function fixtures and a pinned 16-peer header probe.
use qa_network::{
    headers::{self, Direction, Format, Fragment, Header, QPort},
    message::{Encoding, Reader, Writer},
};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

#[derive(Clone, Copy)]
struct Case {
    mode: u8,
    role: u8,
    header: Header,
    length: usize,
    payload: [u8; 1450],
}
fn next(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}
fn case(mode: u8, seed: u32) -> Case {
    let mut state = seed + 1;
    let sequence = next(&mut state);
    let ack = next(&mut state) & 0x1fff_ffff;
    let reliable = next(&mut state) & 1 != 0;
    let rack = next(&mut state) & 1 != 0;
    let port = if seed.is_multiple_of(7) || mode == 9 {
        0
    } else {
        next(&mut state) as u16
    };
    let mut length = [0, 1, 3, 17, 255, 511, 1000, 1024][seed as usize % 8];
    if mode == 2 {
        length = 1024;
    } else if mode == 3 {
        length = 0;
    } else if mode >= 16 {
        length = [0, 1, 99, 1299, 1300][seed as usize % 5];
    }
    if mode == 4 && seed == 128 {
        length = 1450;
    }
    let more = next(&mut state) & 1 != 0 && length >= 512;
    let offset = if mode >= 12 {
        ([0, 1300, 2600, 8192])[seed as usize % 4]
    } else {
        0
    };
    let fragment = match mode {
        12 | 13 => Some(Fragment { offset, more }),
        16 | 17 => Some(Fragment {
            offset,
            more: length == 1300,
        }),
        _ => None,
    };
    Case {
        mode,
        role: match mode {
            0..=3 => (seed % 2) as u8,
            5 | 7 | 11 | 13 | 15 | 17 => 1,
            _ => 0,
        },
        length,
        header: Header {
            sequence: if mode < 4 {
                sequence
            } else {
                sequence & 0x1fff_ffff
            },
            acknowledgement: ack,
            reliable: reliable && !(mode == 4 && seed == 128),
            reliable_ack: rack,
            qport: port,
            fragment,
            datagram_flags: match mode {
                0 => headers::datagram::UNRELIABLE,
                1 => headers::datagram::DATA | headers::datagram::EOM,
                2 => headers::datagram::DATA,
                3 => headers::datagram::ACK,
                _ => 0,
            },
        },
        payload: std::array::from_fn(|_| next(&mut state) as u8),
    }
}
fn format(case: &Case) -> Format {
    match case.mode {
        0..=3 => headers::NETQUAKE,
        4 | 5 => headers::QUAKEWORLD,
        6 | 7 => headers::QUAKE2,
        8 => headers::q2_old(if case.header.qport == 0 {
            QPort::None
        } else {
            QPort::Byte
        }),
        9 => headers::q2_old(QPort::None),
        10..=13 => headers::q2_new(case.header.qport != 0),
        _ => headers::QUAKE3,
    }
}
fn direction(case: &Case) -> Direction {
    if case.role == 0 {
        Direction::ToServer
    } else {
        Direction::ToClient
    }
}
fn payload(case: &Case, bytes: &mut [u8; 1451]) -> usize {
    let prefix = usize::from((4..=11).contains(&case.mode) && case.header.reliable);
    if prefix == 1 {
        bytes[0] = 0xfd;
    }
    bytes[prefix..prefix + case.length].copy_from_slice(&case.payload[..case.length]);
    prefix + case.length
}
fn fixture() -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    for mode in 0..18 {
        for seed in 1..=128 {
            let c = case(mode, seed);
            let mut bytes = [0; 19];
            let mut w = Writer::new(&mut bytes, Encoding::Bytes);
            for (value, width) in [
                (u32::from(c.mode), 8),
                (u32::from(c.role), 8),
                (c.header.sequence, 32),
                (c.header.acknowledgement, 32),
                (u32::from(c.header.reliable), 8),
                (u32::from(c.header.reliable_ack), 8),
                (u32::from(c.header.qport), 16),
                (u32::from(c.header.fragment.map_or(0, |f| f.offset)), 16),
                (u32::from(c.header.fragment.is_some_and(|f| f.more)), 8),
                (c.length as u32, 16),
            ] {
                w.write_bits(value, width).map_err(|e| e.to_string())?;
            }
            out.write_all(w.bytes())
                .and_then(|_| out.write_all(&c.payload[..c.length]))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn encode_fixture() -> Result<(), String> {
    let mut data = Vec::new();
    std::io::stdin()
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    let mut input = Reader::new(&data, Encoding::Bytes);
    let mut out = std::io::stdout().lock();
    while input.byte_position() < data.len() {
        let mode = input.read_bits(8).map_err(|e| e.to_string())? as u8;
        let role = input.read_bits(8).map_err(|e| e.to_string())? as u8;
        let sequence = input.read_bits(32).map_err(|e| e.to_string())?;
        let acknowledgement = input.read_bits(32).map_err(|e| e.to_string())?;
        let reliable = input.read_bits(8).map_err(|e| e.to_string())? != 0;
        let reliable_ack = input.read_bits(8).map_err(|e| e.to_string())? != 0;
        let qport = input.read_bits(16).map_err(|e| e.to_string())? as u16;
        let offset = input.read_bits(16).map_err(|e| e.to_string())? as u16;
        let more = input.read_bits(8).map_err(|e| e.to_string())? != 0;
        let length = input.read_bits(16).map_err(|e| e.to_string())? as usize;
        if mode >= 18 || role > 1 || length > 1450 {
            return Err("invalid native fixture".into());
        }
        let mut c = case(mode, 1);
        c.role = role;
        c.length = length;
        c.header = Header {
            sequence,
            acknowledgement,
            reliable,
            reliable_ack,
            qport,
            datagram_flags: c.header.datagram_flags,
            fragment: if matches!(mode, 12 | 13 | 16 | 17) {
                Some(Fragment { offset, more })
            } else {
                None
            },
        };
        input
            .read_data(&mut c.payload[..length])
            .map_err(|e| e.to_string())?;
        let mut body = [0; 1451];
        let size = payload(&c, &mut body);
        let mut packet = [0; 1464];
        let size = headers::encode(
            format(&c),
            direction(&c),
            c.header,
            &body[..size],
            &mut packet,
        )
        .map_err(|e| e.to_string())?;
        validate(&c, &packet[..size])?;
        out.write_all(&(size as u32).to_le_bytes())
            .and_then(|_| out.write_all(&packet[..size]))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn validate(case: &Case, packet: &[u8]) -> Result<(), String> {
    let (read, body) =
        headers::decode(format(case), direction(case), packet).map_err(|e| e.to_string())?;
    let toggle = (4..=13).contains(&case.mode);
    let port = if case.role != 0 || case.mode < 4 || case.mode == 9 {
        0
    } else if matches!(case.mode, 8 | 10..=13) {
        u16::from(case.header.qport as u8)
    } else {
        case.header.qport
    };
    let expected = Header {
        sequence: case.header.sequence,
        acknowledgement: if toggle {
            case.header.acknowledgement
        } else {
            0
        },
        reliable: toggle && case.header.reliable,
        reliable_ack: toggle && case.header.reliable_ack,
        qport: port,
        datagram_flags: case.header.datagram_flags,
        fragment: case.header.fragment,
    };
    let mut payload_bytes = [0; 1451];
    let length = payload(case, &mut payload_bytes);
    if read != expected || body != &payload_bytes[..length] {
        return Err(format!(
            "native decoded fixture differs, mode {}",
            case.mode
        ));
    }
    Ok(())
}
fn timing() -> Result<(), String> {
    let modes = [0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 17];
    let cases = std::array::from_fn::<_, 16, _>(|peer| case(modes[peer], peer as u32 + 1));
    let mut packets = [[0; 1464]; 16];
    let mut bodies = [[0; 1451]; 16];
    let lengths = std::array::from_fn::<_, 16, _>(|peer| payload(&cases[peer], &mut bodies[peer]));
    let mut samples = [0u64; 600];
    let mut counts = allocations::Counts::default();
    allocations::begin_frame();
    let control = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&control);
    drop(control);
    if allocations::end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut packet_bytes = 0;
    let mut decoded = 0;
    for frame in 0..660 {
        allocations::begin_frame();
        let watch = Stopwatch::start();
        let mut total = 0;
        for peer in 0..16 {
            let c = std::hint::black_box(&cases[peer]);
            let size = headers::encode(
                format(c),
                direction(c),
                c.header,
                &bodies[peer][..lengths[peer]],
                &mut packets[peer],
            )
            .map_err(|e| e.to_string())?;
            validate(c, std::hint::black_box(&packets[peer][..size]))?;
            total += size;
            decoded += 1;
        }
        let elapsed = watch.elapsed().as_nanos() as u64;
        let current = allocations::end_frame();
        if frame == 0 {
            packet_bytes = total;
        }
        if packet_bytes != total {
            return Err("packet workload changed".into());
        }
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            counts.allocations += current.allocations;
            counts.reallocations += current.reallocations;
            counts.requested_bytes += current.requested_bytes;
        }
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"16 mixed native header encode/decode peers, no channel state, workers or gameplay\",\"warmup\":60,\"frames\":600,\"decoded_packets\":{decoded},\"bytes_per_frame\":{packet_bytes},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
        counts.allocations,
        counts.reallocations,
        counts.requested_bytes,
        (samples[299] as f64 + samples[300] as f64) / 2.0,
        samples[593]
    );
    if counts.allocations != 0 || counts.reallocations != 0 || counts.requested_bytes != 0 {
        return Err("header frame allocation".into());
    }
    Ok(())
}
fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref() {
        Some("--fixture") => fixture(),
        Some("--encode") => encode_fixture(),
        None => timing(),
        _ => Err("use --fixture, --encode or no arguments for timing".into()),
    }
}
