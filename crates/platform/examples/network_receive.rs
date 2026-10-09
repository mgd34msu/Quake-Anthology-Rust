//! Original native receive transcripts and a fixed 16-peer receive workload.
use qa_core::{loopback::Endpoint, sys_events::EventTime};
use qa_network::{
    channel::{self, Channel, Delivery, Policy},
    headers::{self, Direction, Format, Fragment, Header, QPort, datagram},
    message::{Encoding, Reader},
};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn configuration(mode: u32) -> (Policy, Format, Endpoint, Direction, usize) {
    let endpoint = if mode.is_multiple_of(2) {
        Endpoint::Client
    } else {
        Endpoint::Server
    };
    let direction = if endpoint == Endpoint::Client {
        Direction::ToClient
    } else {
        Direction::ToServer
    };
    let (policy, format, limit) = match mode / 2 {
        0 => (channel::NETQUAKE, headers::NETQUAKE, 8192),
        1 => (channel::QUAKEWORLD, headers::QUAKEWORLD, 1450),
        2 => (channel::QUAKE2, headers::QUAKE2, 1400),
        3 => (
            channel::q2_old(QPort::Byte),
            headers::q2_old(QPort::Byte),
            32768,
        ),
        4 => (
            channel::q2_old(QPort::None),
            headers::q2_old(QPort::None),
            32768,
        ),
        5 => (channel::q2_new(true), headers::q2_new(true), 32768),
        6 => (channel::q2_new(false), headers::q2_new(false), 32768),
        _ => (channel::QUAKE3, headers::QUAKE3, 16384),
    };
    (policy, format, endpoint, direction, limit)
}

fn compare() -> Result<(), String> {
    let mut data = Vec::new();
    std::io::stdin()
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    let mut input = Reader::new(&data, Encoding::Bytes);
    let mut output = std::io::stdout().lock();
    let mut packet = [0; 4096];
    let mut delivered = [0; 32768];
    let mut controls = [0; 256];
    while input.byte_position() < data.len() {
        let mode = input.read_bits(8).map_err(|e| e.to_string())?;
        let count = input.read_bits(16).map_err(|e| e.to_string())?;
        if mode >= 16 || count > 128 {
            return Err("native transcript boundary".into());
        }
        let (policy, _, endpoint, _, limit) = configuration(mode);
        let mut channel = Channel::load(policy, endpoint, limit, 32).map_err(|e| e.to_string())?;
        for index in 0..count {
            let length = input.read_bits(16).map_err(|e| e.to_string())? as usize;
            if length > packet.len() {
                return Err("transcript packet too large".into());
            }
            input
                .read_data(&mut packet[..length])
                .map_err(|e| e.to_string())?;
            let received = channel
                .receive(&packet[..length], EventTime(u64::from(index)))
                .map_err(|e| e.to_string())?;
            let mut ready = 0u32;
            let mut body_length = 0;
            if let Delivery::Payload(body) = received.delivery {
                ready = if received.header.datagram_flags & datagram::UNRELIABLE != 0 {
                    2
                } else {
                    1
                };
                body_length = body.len();
                delivered[..body_length].copy_from_slice(body);
            }
            let state = channel.state();
            let mut control_length = 0;
            while let Some(header) = channel.next_control() {
                control_length += headers::encode(
                    headers::NETQUAKE,
                    Direction::ToClient,
                    header,
                    &[],
                    &mut controls[control_length..],
                )
                .map_err(|e| e.to_string())?;
            }
            for value in [
                ready,
                state.sequence,
                state.next_datagram,
                state.acknowledged,
                u32::from(state.reliable_acknowledged),
                u32::from(state.reliable_sequence),
                state.fragment_sequence,
                state.fragment_bytes as u32,
                state.dropped,
                control_length as u32,
                body_length as u32,
            ] {
                output
                    .write_all(&value.to_le_bytes())
                    .map_err(|e| e.to_string())?;
            }
            output
                .write_all(&delivered[..body_length])
                .and_then(|_| output.write_all(&controls[..control_length]))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn timing() -> Result<(), String> {
    let mut channels = (0..16)
        .map(|mode| {
            let (policy, _, endpoint, _, limit) = configuration(mode);
            Channel::load(policy, endpoint, limit, 8).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bodies = std::array::from_fn::<_, 1300, _>(|index| (index.wrapping_mul(37)) as u8);
    let mut packet = [0; 1464];
    let mut samples = [0u64; 600];
    let mut total_counts = allocations::Counts::default();
    let mut delivered = 0u64;
    let mut bytes = 0u64;
    let mut packets = 0u64;
    allocations::begin_frame();
    let control = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&control);
    drop(control);
    if allocations::end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    for frame in 0usize..660 {
        allocations::begin_frame();
        let watch = Stopwatch::start();
        for (mode, channel) in channels.iter_mut().enumerate() {
            let (_, format, _, direction, _) = configuration(mode as u32);
            let sequence = if mode < 2 {
                frame as u32
            } else {
                frame as u32 + 1
            };
            let fragmented = mode >= 10;
            for part in 0..if fragmented { 2 } else { 1 } {
                let fragment = fragmented.then_some(Fragment {
                    offset: if part == 0 { 0 } else { 1300 },
                    more: part == 0,
                });
                let header = Header {
                    sequence,
                    acknowledgement: frame as u32,
                    reliable: frame.is_multiple_of(3),
                    reliable_ack: !frame.is_multiple_of(2),
                    qport: 37,
                    datagram_flags: if mode < 2 {
                        datagram::DATA | datagram::EOM
                    } else {
                        0
                    },
                    fragment,
                };
                let payload = if !fragmented {
                    &bodies[..256]
                } else if part == 0 {
                    &bodies[..]
                } else {
                    &bodies[..127]
                };
                let length = headers::encode(
                    format,
                    direction,
                    std::hint::black_box(header),
                    payload,
                    &mut packet,
                )
                .map_err(|e| e.to_string())?;
                let received = channel
                    .receive(
                        std::hint::black_box(&packet[..length]),
                        EventTime(frame as u64),
                    )
                    .map_err(|e| e.to_string())?;
                let ready = if let Delivery::Payload(body) = received.delivery {
                    let expected = if fragmented { 1427 } else { 256 };
                    if body.len() != expected
                        || body[0] != 0
                        || body[expected - 1] != bodies[if fragmented { 126 } else { 255 }]
                    {
                        return Err("receive body differs".into());
                    }
                    bytes += body.len() as u64;
                    delivered += 1;
                    true
                } else {
                    false
                };
                if ready != (!fragmented || part == 1) {
                    return Err("fragment delivery phase differs".into());
                }
                packets += 1;
                while let Some(control) = channel.next_control() {
                    if control.sequence != sequence || control.datagram_flags != datagram::ACK {
                        return Err("native control differs".into());
                    }
                    std::hint::black_box(control);
                }
            }
        }
        let elapsed = watch.elapsed().as_nanos() as u64;
        let counts = allocations::end_frame();
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            total_counts.allocations += counts.allocations;
            total_counts.reallocations += counts.reallocations;
            total_counts.requested_bytes += counts.requested_bytes;
        }
    }
    if channels
        .iter()
        .any(|channel| channel.counts().delivered != 660 || channel.pending_controls() != 0)
    {
        return Err("peer fixture counts differ".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"16 mixed native peers: header encode, sequence and ordered fragment receive; no transmit ACK validation, workers or gameplay\",\"warmup\":60,\"frames\":600,\"received_packets\":{packets},\"delivered_messages\":{delivered},\"delivered_bytes\":{bytes},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
        total_counts.allocations,
        total_counts.reallocations,
        total_counts.requested_bytes,
        (samples[299] as f64 + samples[300] as f64) / 2.0,
        samples[593]
    );
    if total_counts.allocations != 0
        || total_counts.reallocations != 0
        || total_counts.requested_bytes != 0
    {
        return Err("receive frame allocation".into());
    }
    Ok(())
}
fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref() {
        Some("--compare") => compare(),
        None => timing(),
        _ => Err("use --compare or no arguments for timing".into()),
    }
}
