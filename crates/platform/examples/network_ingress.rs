//! Two queued-memory packet drains, native channels and output ACK retirement.
use qa_core::{
    events::{EventRing, OutputSubmission, OutputTarget},
    loopback::{Endpoint, Loopback, LoopbackLimits},
    primitives::{ClientId, PrintKind},
    sys_events::{EventKind, EventTime, Peer, SysEventQueue},
};
use qa_network::{
    channel::{self, Channel},
    ingress::{Connection, Connections, Incoming, Route},
};
use qa_platform::{Stopwatch, allocations};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn send(
    peers: &mut Connections,
    local: &mut Loopback,
    client: ClientId,
    endpoint: Endpoint,
    body: Option<&[u8]>,
    time: EventTime,
) -> Result<(), String> {
    let channel = &mut peers.get_mut(client, endpoint).ok_or("channel")?.channel;
    let packet = channel
        .prepare(body, time)
        .map_err(|e| e.to_string())?
        .ok_or("packet")?;
    local
        .send(endpoint, client, packet.bytes)
        .map_err(|e| format!("local {e:?}"))?;
    channel.submitted(time).map_err(|e| e.to_string())
}
fn main() -> Result<(), String> {
    let mut peers = Connections::load(16);
    let mut outputs = EventRing::load(128, 16, 256, 64).map_err(|e| format!("output {e:?}"))?;
    let mut consumers = Vec::with_capacity(16);
    for slot in 0..16 {
        let client = ClientId(slot);
        let output = outputs
            .bind(OutputTarget::Client(client))
            .ok_or("consumer")?;
        consumers.push(output);
        let policy = match slot % 5 {
            0 => channel::NETQUAKE,
            1 => channel::QUAKEWORLD,
            2 => channel::QUAKE2,
            3 => channel::q2_new(true),
            _ => channel::QUAKE3,
        };
        for endpoint in [Endpoint::Client, Endpoint::Server] {
            peers
                .bind(
                    client,
                    endpoint,
                    Connection {
                        route: Route {
                            socket: endpoint.socket(),
                            peer: Peer::Loopback(client),
                        },
                        channel: Channel::load(policy, endpoint, 8192, 16)
                            .map_err(|e| e.to_string())?,
                        output: (endpoint == Endpoint::Server).then_some(output),
                    },
                )
                .map_err(|e| format!("bind {e:?}"))?;
        }
    }
    let mut local = Loopback::load(
        [LoopbackLimits {
            maximum_message: 1400,
            payload_bytes: 5600,
            messages: 4,
        }; 16],
    )
    .map_err(|e| format!("local {e:?}"))?;
    let mut queue = SysEventQueue::load(128, 128 * 1400).map_err(|e| format!("events {e:?}"))?;
    let body = b"native channel payload\0";
    let mut samples = [0u64; 600];
    let mut counts = allocations::Counts::default();
    let mut payloads = 0;
    let mut retired = 0;
    allocations::begin_frame();
    let positive = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&positive);
    drop(positive);
    if allocations::end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    for frame in 0..660 {
        allocations::begin_frame();
        let watch = Stopwatch::start();
        let time = EventTime(frame as u64 * 10_000_000);
        for (slot, &consumer) in consumers.iter().enumerate() {
            let client = ClientId(slot as u32);
            let q3 = slot % 5 == 4;
            if !q3 {
                let sequence = outputs
                    .print(Some(client), PrintKind::Center, format_args!("retained"))
                    .map_err(|e| format!("output {e:?}"))?;
                let receipt = peers
                    .get_mut(client, Endpoint::Server)
                    .ok_or("channel")?
                    .channel
                    .queue_reliable(body)
                    .map_err(|e| e.to_string())?;
                if !outputs.submit(consumer, sequence, OutputSubmission::Reliable(receipt)) {
                    return Err("output submission".into());
                }
            }
            send(
                &mut peers,
                &mut local,
                client,
                Endpoint::Server,
                q3.then_some(body),
                time,
            )?;
        }
        for phase in 0..2 {
            local.enqueue(&mut queue, time);
            let mut bad = false;
            while let Some(event) = queue.pop() {
                if let EventKind::Packet {
                    socket,
                    from,
                    bytes,
                } = event.kind
                {
                    peers.receive(
                        socket,
                        from,
                        std::hint::black_box(bytes),
                        event.time,
                        |_, endpoint, incoming| match incoming {
                            Incoming::Payload(bytes) if !bytes.is_empty() => {
                                bad |= endpoint != Endpoint::Client || bytes != body;
                                payloads += 1;
                            }
                            Incoming::Acknowledged {
                                receipt,
                                output: Some(consumer),
                            } => {
                                let n = outputs.acknowledge(consumer, receipt);
                                bad |= endpoint != Endpoint::Server || n != 1;
                                retired += n;
                            }
                            _ => {}
                        },
                    );
                }
            }
            if bad {
                return Err("native route/receipt fidelity".into());
            }
            if phase == 0 {
                for slot in 0..16 {
                    if slot % 5 != 4 {
                        send(
                            &mut peers,
                            &mut local,
                            ClientId(slot),
                            Endpoint::Client,
                            None,
                            time,
                        )?;
                    }
                }
            }
        }
        let elapsed = watch.elapsed().as_nanos() as u64;
        let actual = allocations::end_frame();
        if !outputs.is_empty() {
            return Err("unretired native output".into());
        }
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            counts.allocations += actual.allocations;
            counts.reallocations += actual.reallocations;
            counts.requested_bytes += actual.requested_bytes;
        }
    }
    if (
        peers.packets,
        payloads,
        retired,
        peers.unrouted,
        peers.malformed,
    ) != (19140, 10560, 8580, 0, 0)
        || counts != allocations::Counts::default()
    {
        return Err(format!(
            "ingress gate: packets={} payloads={payloads} retired={retired} unrouted={} malformed={} allocations={counts:?}",
            peers.packets, peers.unrouted, peers.malformed
        ));
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"16 mixed peers, two queued-memory drains, prepare/submit, SysEventQueue dispatch and native NQ/QW/Q2 output ACK retirement; Q3 framing only; no workers, physical intake or gameplay\",\"warmup\":60,\"frames\":600,\"packets\":{},\"payloads\":{payloads},\"retired_records\":{retired},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
        peers.packets,
        counts.allocations,
        counts.reallocations,
        counts.requested_bytes,
        (samples[299] as f64 + samples[300] as f64) * 0.5,
        samples[593]
    );
    Ok(())
}
