//! Native send/receive transcripts and a pinned mixed-peer channel workload.
use qa_core::{loopback::Endpoint, sys_events::EventTime};
use qa_network::{
    channel::{self, Channel, Delivery},
    headers::QPort,
    message::{Encoding, Reader},
};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn load(mode: u32) -> Result<Channel, String> {
    let (policy, limit) = match mode / 2 {
        0 => (channel::NETQUAKE, 8192),
        1 => (channel::QUAKEWORLD, 1450),
        2 => (channel::QUAKE2, 1400),
        3 => (channel::q2_old(QPort::Byte), 32768),
        4 => (channel::q2_old(QPort::None), 32768),
        5 => (channel::q2_new(true), 32768),
        6 => (channel::q2_new(false), 32768),
        _ => (channel::QUAKE3, 16384),
    };
    let endpoint = if mode.is_multiple_of(2) {
        Endpoint::Client
    } else {
        Endpoint::Server
    };
    let mut channel = Channel::load(policy, endpoint, limit, 16).map_err(|e| e.to_string())?;
    channel.set_qport(if mode / 2 == 4 || mode / 2 == 6 {
        0
    } else {
        37
    });
    Ok(channel)
}
fn compare() -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    let mut reader = Reader::new(&input, Encoding::Bytes);
    let mut output = std::io::stdout().lock();
    let mut body = [0; 32768];
    let mut emitted = [0; 4096];
    while reader.byte_position() < input.len() {
        let mode = reader.read_bits(8).map_err(|e| e.to_string())?;
        let count = reader.read_bits(16).map_err(|e| e.to_string())?;
        if mode >= 16 || count > 128 {
            return Err("transcript boundary".into());
        }
        let mut channel = load(mode)?;
        for _ in 0..count {
            let operation = reader.read_bits(8).map_err(|e| e.to_string())?;
            let milliseconds = reader.read_bits(32).map_err(|e| e.to_string())?;
            let present = reader.read_bits(8).map_err(|e| e.to_string())? != 0;
            let length = reader.read_bits(16).map_err(|e| e.to_string())? as usize;
            if length > body.len() {
                return Err("transcript body boundary".into());
            }
            reader
                .read_data(&mut body[..length])
                .map_err(|e| e.to_string())?;
            let time = EventTime(u64::from(milliseconds) * 1_000_000);
            let mut emitted_length = 0;
            match operation {
                0 => {
                    channel
                        .queue_reliable(&body[..length])
                        .map_err(|e| e.to_string())?;
                }
                1 => {
                    if let Some(packet) = channel
                        .prepare(present.then_some(&body[..length]), time)
                        .map_err(|e| e.to_string())?
                    {
                        emitted[..4].copy_from_slice(&(packet.bytes.len() as u32).to_le_bytes());
                        emitted[4..4 + packet.bytes.len()].copy_from_slice(packet.bytes);
                        emitted_length = 4 + packet.bytes.len();
                        channel.submitted(time).map_err(|e| e.to_string())?;
                    }
                }
                2 => {
                    channel
                        .receive(&body[..length], time)
                        .map_err(|e| e.to_string())?;
                    if mode < 2
                        && channel.send_state().reliable_bytes != 0
                        && let Some(packet) =
                            channel.prepare(None, time).map_err(|e| e.to_string())?
                    {
                        emitted[..4].copy_from_slice(&(packet.bytes.len() as u32).to_le_bytes());
                        emitted[4..4 + packet.bytes.len()].copy_from_slice(packet.bytes);
                        emitted_length = 4 + packet.bytes.len();
                        channel.submitted(time).map_err(|e| e.to_string())?;
                    }
                }
                _ => return Err("transcript operation".into()),
            }
            let state = channel.send_state();
            for value in [
                state.sequence,
                state.datagram_sequence,
                state.ack_sequence,
                u32::from(state.reliable_sequence),
                state.last_reliable_sequence,
                state.reliable_bytes as u32,
                state.fragment_bytes as u32,
                state.fragment_offset as u32,
                state.packets as u32,
                emitted_length as u32,
            ] {
                output
                    .write_all(&value.to_le_bytes())
                    .map_err(|e| e.to_string())?;
            }
            output
                .write_all(&emitted[..emitted_length])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn timing() -> Result<(), String> {
    let mut senders = (0..16).map(load).collect::<Result<Vec<_>, _>>()?;
    let mut receivers = (0..16)
        .map(|mode| load(mode ^ 1))
        .collect::<Result<Vec<_>, _>>()?;
    let body = [7; 2600];
    let mut packet = [0; 1400];
    let mut reply = [0; 1400];
    let mut samples = [0u64; 600];
    let mut counts = allocations::Counts::default();
    let mut deliveries = 0;
    let mut packets = 0;
    let mut receipts = 0;
    let mut delivered_bytes = 0;
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
        for mode in 0..16 {
            let time = EventTime(frame as u64 * 10_000_000);
            let length = if !(2..10).contains(&mode) { 2600 } else { 256 };
            let receipt = if mode < 14 {
                Some(
                    senders[mode]
                        .queue_reliable(std::hint::black_box(&body[..length]))
                        .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };
            let parts = if !(2..14).contains(&mode) {
                3
            } else if mode >= 10 {
                2
            } else {
                1
            };
            for part in 0..parts {
                let incoming = if mode >= 14 && part == 0 {
                    Some(&body[..length])
                } else {
                    None
                };
                let prepared = senders[mode]
                    .prepare(incoming, time)
                    .map_err(|e| e.to_string())?
                    .ok_or("missing transmit packet")?;
                let packet_length = prepared.bytes.len();
                packet[..packet_length].copy_from_slice(prepared.bytes);
                senders[mode].submitted(time).map_err(|e| e.to_string())?;
                let received = receivers[mode]
                    .receive(std::hint::black_box(&packet[..packet_length]), time)
                    .map_err(|e| e.to_string())?;
                if let Delivery::Payload(bytes) = received.delivery {
                    if part + 1 != parts || bytes != &body[..length] {
                        return Err("native payload differs".into());
                    }
                    deliveries += 1;
                    delivered_bytes += bytes.len();
                } else if part + 1 == parts {
                    return Err("native payload not delivered".into());
                }
                packets += 1;
                if mode < 14 {
                    let prepared = receivers[mode]
                        .prepare(None, time)
                        .map_err(|e| e.to_string())?
                        .ok_or("missing native reply")?;
                    let reply_length = prepared.bytes.len();
                    reply[..reply_length].copy_from_slice(prepared.bytes);
                    receivers[mode].submitted(time).map_err(|e| e.to_string())?;
                    senders[mode]
                        .receive(std::hint::black_box(&reply[..reply_length]), time)
                        .map_err(|e| e.to_string())?;
                    packets += 1;
                    if part + 1 == parts {
                        let actual = senders[mode].reliable_receipts();
                        if actual.len() != 1 || Some(actual[0]) != receipt {
                            return Err("native receipt differs".into());
                        }
                        receipts += 1;
                    } else if !senders[mode].reliable_receipts().is_empty() {
                        return Err("premature receipt".into());
                    }
                }
            }
        }
        let elapsed = watch.elapsed().as_nanos() as u64;
        let current = allocations::end_frame();
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            counts.allocations += current.allocations;
            counts.reallocations += current.reallocations;
            counts.requested_bytes += current.requested_bytes;
        }
    }
    if deliveries != 10560 || receipts != 9240 || delivered_bytes != 15079680 {
        return Err(format!(
            "fixture counts differ {deliveries}/{receipts}/{delivered_bytes}"
        ));
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"16 mixed native channel peers: prepared/submitted packets, receive assembly, NQ/QW/Q2 ACK receipts; Q3 fragmentation only; no workers or gameplay\",\"warmup\":60,\"frames\":600,\"processed_packets\":{packets},\"delivered_messages\":{deliveries},\"delivered_bytes\":{delivered_bytes},\"reliable_receipts\":{receipts},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
        counts.allocations,
        counts.reallocations,
        counts.requested_bytes,
        (samples[299] as f64 + samples[300] as f64) / 2.0,
        samples[593]
    );
    if counts.allocations != 0 || counts.reallocations != 0 || counts.requested_bytes != 0 {
        return Err("channel allocation".into());
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
