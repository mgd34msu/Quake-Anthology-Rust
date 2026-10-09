//! Ordinary output dispatch, native peer decode and ACK ingress, with no games.
use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, ModuleId, PlayerTail, PrintKind},
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

fn protocol(slot: usize) -> Protocol {
    [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
    ][slot % 3]
}
struct Source {
    peers: Box<[Channel]>,
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
            for print in Prints::new(protocol(slot), body) {
                let Ok(print) = print else {
                    self.bad = true;
                    return false;
                };
                let decoded = std::str::from_utf8(print.text)
                    .ok()
                    .and_then(|s| s.strip_prefix("native "))
                    .and_then(|s| s.split_once(':'))
                    .and_then(|(client, frame)| {
                        Some((client.parse::<usize>().ok()?, frame.parse::<u64>().ok()?))
                    });
                self.bad |= print.kind != PrintKind::Center || decoded != Some((slot, self.frame));
                self.prints += 1;
            }
        }
        let Ok(Some(packet)) = self.peers[slot].prepare(None, time) else {
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
fn main() -> Result<(), String> {
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
                    channel: Channel::load(protocol(slot).channel(), Endpoint::Server, 8192, 16)
                        .map_err(|e| e.to_string())?,
                    output: *consumer,
                    commands: Some(Commands::load(protocol(slot))),
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
            .map(|slot| Channel::load(protocol(slot).channel(), Endpoint::Client, 8192, 16))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
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
        "{{\"scope\":\"ordinary host output ring through sixteen NQ/QW/Q2 native channels, peer print decode and real native ACK ingress; no game or socket syscall\",\"warmup\":60,\"frames\":600,\"measured_server_ticks\":{ticks},\"measured_output_packets\":{packets},\"prints\":{},\"native_receipts\":{},\"intake_calls\":{},\"median_ns\":{},\"p99_ns\":{},\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{}}}",
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
