//! Ordinary output dispatch, native peer decode and ACK ingress, with no games.
use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, ModuleId, PlayerTail, PrintKind, UserCmd},
    sys_events::{EventKind, EventTime, Peer, SysEvent, SysEventQueue},
};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as ChannelConnection, Route},
    outputs::Prints,
};
use qa_platform::{Stopwatch, allocations};
use qa_session::{clients::Connection, timing::TickRate};
use std::time::Duration;

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn protocol(slot: usize, q3: bool) -> Protocol {
    let protocols = [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake3_68,
    ];
    protocols[slot % if q3 { 4 } else { 3 }]
}
struct Source {
    peers: Box<[Channel]>,
    commands: Box<[Commands]>,
    pending: Box<[[u8; 1400]]>,
    lengths: [usize; 16],
    frame: u64,
    polls: u64,
    prints: u64,
    bad: bool,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> EventTime {
        EventTime(self.frame * 16_000_000)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.polls += 1;
        let time = EventTime(self.frame * 16_000_000);
        for slot in 0..16 {
            if self.lengths[slot] != 0 {
                let to = std::net::SocketAddr::from(([127, 0, 0, 1], 2000 + slot as u16));
                self.bad |= queue
                    .push(SysEvent {
                        time,
                        kind: EventKind::Packet {
                            socket: 0,
                            from: Peer::Socket(to),
                            bytes: &self.pending[slot][..self.lengths[slot]],
                        },
                    })
                    .is_err();
                self.lengths[slot] = 0;
            }
        }
        self.bad |= queue
            .push(SysEvent {
                time,
                kind: EventKind::Time,
            })
            .is_err();
    }
    fn wait_time(&mut self, _: Duration) -> EventTime {
        EventTime(self.frame * 16_000_000)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn send_packet(&mut self, socket: u16, to: std::net::SocketAddr, bytes: &[u8]) -> bool {
        let slot = usize::from(to.port() - 2000);
        self.bad |= socket != 0;
        let time = EventTime(self.frame * 16_000_000);
        let Ok(received) = self.peers[slot].receive(bytes, time) else {
            self.bad = true;
            return false;
        };
        if let Delivery::Payload(body) = received.delivery {
            if self.commands[slot].protocol == Protocol::Quake3_68 {
                let codec = &mut self.commands[slot];
                let Ok(n) = codec.stage(body) else {
                    self.bad = true;
                    return false;
                };
                let decoded = codec.decode_output(
                    n,
                    received.header.sequence,
                    &mut self.peers[slot],
                    |incoming| {
                        let qa_network::ingress::Incoming::ReliableCommand { text, .. } = incoming
                        else {
                            self.bad = true;
                            return;
                        };
                        self.bad |= text
                            .strip_prefix(b"cp \"")
                            .and_then(|t| t.strip_suffix(b"\""))
                            .and_then(native_position)
                            != Some((slot, self.frame));
                        self.prints += 1;
                    },
                );
                if decoded.is_err() {
                    self.bad = true;
                    return false;
                }
            } else {
                for print in Prints::new(self.commands[slot].protocol, body) {
                    let Ok(print) = print else {
                        self.bad = true;
                        return false;
                    };
                    let decoded = native_position(print.text);
                    self.bad |=
                        print.kind != PrintKind::Center || decoded != Some((slot, self.frame));
                    self.prints += 1;
                }
            }
        } else {
            return true; // Complete fragments before sending the native message ACK.
        }
        let mut bytes = [0; 8192];
        let prepared = if self.commands[slot].protocol == Protocol::Quake3_68 {
            let command = UserCmd {
                server_time_ms: ((self.frame + 1) * 16) as i32,
                duration_ms: 16,
                ..UserCmd::default()
            };
            let Ok(n) = self.commands[slot].encode(&command, &self.peers[slot], &mut bytes) else {
                self.bad = true;
                return false;
            };
            self.peers[slot].prepare_move(&bytes[..n], time, None)
        } else {
            self.peers[slot].prepare(None, time)
        };
        let Ok(Some(packet)) = prepared else {
            self.bad = true;
            return false;
        };
        self.bad |= self.lengths[slot] != 0;
        self.lengths[slot] = packet.bytes.len();
        self.pending[slot][..packet.bytes.len()].copy_from_slice(packet.bytes);
        self.bad |= self.peers[slot].submitted(time).is_err();
        true
    }
}
fn native_position(text: &[u8]) -> Option<(usize, u64)> {
    let (client, frame) = std::str::from_utf8(text)
        .ok()?
        .strip_prefix("native ")?
        .split_once(':')?;
    Some((client.parse().ok()?, frame.parse().ok()?))
}
fn main() -> Result<(), String> {
    let q3 = std::env::args().any(|arg| arg == "--q3");
    let mut runtime = Runtime::load(16, [])?;
    let mut consumers = [None; 16];
    for (slot, consumer) in consumers.iter_mut().enumerate() {
        let client = runtime
            .server
            .connect(Connection::Remote, ModuleId(0), PlayerTail::None, None)
            .ok_or("client")?;
        *consumer = runtime.server.clients[slot].output;
        runtime
            .network
            .bind(
                client,
                Endpoint::Server,
                ChannelConnection {
                    route: Route {
                        socket: 0,
                        peer: Peer::Socket(std::net::SocketAddr::from((
                            [127, 0, 0, 1],
                            2000 + slot as u16,
                        ))),
                    },
                    channel: Channel::load(
                        protocol(slot, q3).channel(),
                        Endpoint::Server,
                        8192,
                        16,
                    )
                    .map_err(|e| e.to_string())?,
                    output: *consumer,
                    commands: Some(Commands::load(protocol(slot, q3))),
                },
            )
            .map_err(|e| format!("bind {e:?}"))?;
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).map_err(|e| e.to_string())?,
        runtime,
        TickRate::FrameDriven,
        vec![],
    )?;
    let mut source = Source {
        peers: (0..16)
            .map(|slot| Channel::load(protocol(slot, q3).channel(), Endpoint::Client, 8192, 16))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        commands: (0..16)
            .map(|slot| Commands::load(protocol(slot, q3)))
            .collect(),
        pending: vec![[0; 1400]; 16].into_boxed_slice(),
        lengths: [0; 16],
        frame: 0,
        polls: 0,
        prints: 0,
        bad: false,
    };
    allocations::begin_frame();
    let positive = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&positive);
    drop(positive);
    if allocations::end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut samples = [0u64; 600];
    let mut total = allocations::Counts::default();
    let mut packets = 0;
    let mut ticks = 0;
    for frame in 0..660 {
        source.frame = frame;
        allocations::begin_frame();
        let watch = Stopwatch::start();
        for slot in 0..16 {
            host.runtime.print_event(
                Some(ClientId(slot)),
                PrintKind::Center,
                format_args!("native {slot}:{frame}"),
            );
        }
        let result = host.frame(&mut source, true);
        let elapsed = watch.elapsed().as_nanos() as u64;
        let counts = allocations::end_frame();
        if source.bad
            || result.drains != 2
            || result.output.native_unsupported
                + result.output.native_blocked
                + result.output.native_disconnected
                != 0
        {
            return Err("native output fidelity".into());
        }
        if frame >= 60 {
            samples[frame as usize - 60] = elapsed;
            packets += result.output.native_packets;
            ticks += result.server_ticks;
            total.allocations += counts.allocations;
            total.reallocations += counts.reallocations;
            total.requested_bytes += counts.requested_bytes;
        }
    }
    if source.prints != 660 * 16
        || host.runtime.network.acknowledged != 659 * 16
        || source.polls != 660 * 2
    {
        return Err("native packet/receipt counts".into());
    }
    for consumer in consumers.into_iter().flatten() {
        let counters = host
            .runtime
            .server
            .events
            .counters(consumer)
            .ok_or("consumer")?;
        if counters.acknowledged_records != 659 || counters.overflow != 0 {
            return Err("native retirement counts".into());
        }
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"ordinary host output ring through sixteen native channels, peer print decode and real native ACK ingress; no game or socket syscall\",\"q3\":{q3},\"warmup\":60,\"frames\":600,\"measured_server_ticks\":{ticks},\"measured_output_packets\":{packets},\"prints\":{},\"native_receipts\":{},\"intake_calls\":{},\"median_ns\":{},\"p99_ns\":{},\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{}}}",
        source.prints,
        host.runtime.network.acknowledged,
        source.polls,
        (samples[299] + samples[300]) / 2,
        samples[593],
        total.allocations,
        total.reallocations,
        total.requested_bytes
    );
    if total.allocations + total.reallocations != 0 {
        return Err("Rust heap activity".into());
    }
    Ok(())
}
