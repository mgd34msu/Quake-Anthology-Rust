//! Fixed-capacity host output with an unacknowledged peer beside a healthy peer.
use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource, Provider},
};
use qa_console::{commands::Console, views::Context};
use qa_core::loopback::Endpoint;
use qa_core::{events::*, primitives::*, sys_events::*};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as ChannelConnection, Route},
    outputs::Prints,
};
use qa_platform::{
    Stopwatch,
    allocations::{CountingAllocator, begin_frame, end_frame},
};
use qa_session::{
    clients::Connection,
    timing::{Tick, TickRate},
};
use std::{net::SocketAddr, time::Duration};
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
struct Source {
    frame: u64,
    healthy: u64,
    stalled: u64,
    peers: [Channel; 2],
    commands: [Commands; 2],
    ack: [u8; 1400],
    ack_length: usize,
    unsent: bool,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> EventTime {
        EventTime(self.frame * 25_000_000)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        if self.ack_length != 0 {
            let _ = queue.push(SysEvent {
                time: self.begin_frame(),
                kind: EventKind::Packet {
                    socket: 0,
                    from: Peer::Socket(SocketAddr::from(([127, 0, 0, 1], 1001))),
                    bytes: &self.ack[..self.ack_length],
                },
            });
            self.ack_length = 0;
        }
        let _ = queue.push(SysEvent {
            time: self.begin_frame(),
            kind: EventKind::Time,
        });
    }
    fn wait_time(&mut self, _: Duration) -> EventTime {
        self.begin_frame()
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn send_packet(&mut self, socket: u16, to: SocketAddr, bytes: &[u8]) -> bool {
        let slot = usize::from(to.port() - 1000);
        if socket != 0 || slot > 1 {
            return false;
        }
        if slot == 0 && self.unsent {
            self.stalled += 1;
            return false;
        }
        let time = EventTime(self.frame * 25_000_000);
        let Ok(received) = self.peers[slot].receive(bytes, time) else {
            return false;
        };
        if let Delivery::Payload(body) = received.delivery {
            if self.commands[slot].protocol == Protocol::Quake3_68 {
                let codec = &mut self.commands[slot];
                let Ok(n) = codec.stage(body) else {
                    return false;
                };
                let mut valid = true;
                if codec
                    .decode_output(
                        n,
                        received.header.sequence,
                        &mut self.peers[slot],
                        |incoming| {
                            let qa_network::ingress::Incoming::ReliableCommand { text, .. } =
                                incoming
                            else {
                                valid = false;
                                return;
                            };
                            valid &= text.starts_with(b"cp \"")
                                && text.ends_with(b"\"")
                                && text.len() > 5;
                            if slot == 1 {
                                self.healthy += 1;
                            } else {
                                self.stalled += 1;
                            }
                        },
                    )
                    .is_err()
                    || !valid
                {
                    return false;
                }
            } else {
                for print in Prints::new(self.commands[slot].protocol, body) {
                    let Ok(print) = print else {
                        return false;
                    };
                    if print.kind != PrintKind::Center || print.text.is_empty() {
                        return false;
                    }
                    if slot == 1 {
                        self.healthy += 1;
                    } else {
                        self.stalled += 1;
                    }
                }
            }
        } else {
            return true;
        }
        if slot == 1 {
            let mut bytes = [0; 8192];
            let prepared = if self.commands[slot].protocol == Protocol::Quake3_68 {
                let command = UserCmd {
                    server_time_ms: ((self.frame + 1) * 25) as i32,
                    duration_ms: 25,
                    ..UserCmd::default()
                };
                let Ok(n) = self.commands[slot].encode(&command, &self.peers[slot], &mut bytes)
                else {
                    return false;
                };
                self.peers[slot].prepare_move(&bytes[..n], time, None)
            } else {
                self.peers[slot].prepare(None, time)
            };
            let Ok(Some(packet)) = prepared else {
                return false;
            };
            self.ack_length = packet.bytes.len();
            self.ack[..self.ack_length].copy_from_slice(packet.bytes);
            if self.peers[slot].submitted(time).is_err() {
                return false;
            }
        }
        true
    }
}
fn emit(host: &mut FrameHost, tick: Tick) {
    let runtime = &mut host.runtime;
    runtime.print_event(
        None,
        PrintKind::Center,
        format_args!("{}:{}", tick.source_slot, tick.index),
    );
}
fn consume(host: &mut FrameHost, tick: Tick, record: OutputRecord) -> OutputSubmission {
    let runtime = &mut host.runtime;
    let player = &mut runtime.server.clients[10 + tick.source_slot].player;
    if let FrameEvent::Print(p) = record.event
        && runtime.server.events.texts.get(p.text).is_none()
    {
        player.armor += 1;
    }
    player.health += 1;
    OutputSubmission::BestEffort
}
fn run(
    unsent: bool,
    protocol: Protocol,
    heap_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::load(64, std::iter::empty())?;
    runtime.server.events = EventRing::load(32, 8, 64, 256).map_err(|_| "events")?;
    runtime.server.presentation = runtime
        .server
        .events
        .bind(OutputTarget::Presentation)
        .ok_or("presentation")?;
    for connection in [Connection::Remote, Connection::Remote, Connection::Local] {
        runtime
            .server
            .connect(connection, ModuleId(0), PlayerTail::None, None)
            .ok_or("clients")?;
    }
    let stalled = runtime.server.clients[0].output.ok_or("stalled cursor")?;
    let healthy = runtime.server.clients[1].output.ok_or("healthy cursor")?;
    for slot in 0..2 {
        let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)?;
        channel.set_qport([101, 211][slot as usize]);
        runtime
            .network
            .bind(
                ClientId(slot),
                Endpoint::Server,
                ChannelConnection {
                    route: Route {
                        socket: 0,
                        peer: Peer::Socket(SocketAddr::from(([127, 0, 0, 1], 1000 + slot as u16))),
                    },
                    channel,
                    output: runtime.server.clients[slot as usize].output,
                    commands: Some(Commands::load(protocol)),
                },
            )
            .map_err(|_| "binding")?;
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).map_err(|e| e.to_string())?,
        runtime,
        TickRate::fixed(25).ok_or("world rate")?,
        [100, 50, 25]
            .into_iter()
            .enumerate()
            .map(|(slot, ms)| Provider {
                module: ModuleId(slot as u16 + 1),
                rate: TickRate::fixed(ms).expect("fixed rate"),
                frame: emit,
                output: Some(consume),
            })
            .collect(),
    )?;
    host.local_clients[0] = Some(ClientId(2));
    let mut source = Source {
        frame: 0,
        healthy: 0,
        stalled: 0,
        peers: std::array::from_fn(|slot| {
            let mut channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16)
                .expect("channel load");
            channel.set_qport([101, 211][slot]);
            channel
        }),
        commands: std::array::from_fn(|_| Commands::load(protocol)),
        ack: [0; 1400],
        ack_length: 0,
        unsent,
    };
    // Positive control and all load-time allocation are outside measured frames.
    begin_frame();
    std::hint::black_box(vec![1u8; 32]);
    if end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut samples = [0u64; 600];
    let mut maximum = 0;
    let mut bytes = 0;
    let mut ticks = 0;
    let mut leased_id = None;
    let mut resync_frame = None;
    let mut continuing_frames = 0;
    let mut disconnected = 0;
    let mut overflow = 0;
    let mut retired = 0;
    for frame in 0..660 {
        source.frame = frame;
        let healthy_before = source.healthy;
        begin_frame();
        let timer = (!heap_only).then(Stopwatch::start);
        let result = host.frame(&mut source, true);
        let elapsed = timer.map_or(0, |timer| timer.elapsed().as_nanos() as u64);
        let counts = end_frame();
        disconnected += result.output.native_disconnected;
        overflow += result.output.native_overflow;
        retired += result.output.native_resync_retired;
        if result.output.native_disconnected != 0 {
            resync_frame.get_or_insert(frame);
        }
        if result.drains != 2 || result.output_drains != 1 {
            return Err("host drain phases".into());
        }
        // Check each complete SERVER -> CLIENT cycle, including the precise
        // overflow frame. Aggregate totals alone could hide a publication wait.
        if frame > 0 {
            let produced = result.server_ticks - 1;
            if host.runtime.server.world_frame != frame
                || source.healthy - healthy_before != produced
                || produced < 1
            {
                return Err(format!("peer stalled host progress at frame {frame}").into());
            }
            if frame >= 60 {
                continuing_frames += 1;
            }
        }
        let line = host.runtime.server.clients[2].hud.centerprint.as_ref();
        if let Some(line) = line {
            if host
                .runtime
                .server
                .events
                .texts
                .get(line.text.id())
                .is_none()
            {
                return Err("HUD lease expired".into());
            }
            leased_id = Some(line.text.id());
        }
        if frame >= 60 {
            samples[frame as usize - 60] = elapsed;
            maximum = maximum.max(counts.allocations + counts.reallocations);
            bytes = bytes.max(counts.requested_bytes);
            ticks += result.server_ticks;
        }
    }
    host.runtime.quit = true;
    host.frame(&mut source, true); // Last real ACK enters at the normal quit drain.
    let healthy_counters = host
        .runtime
        .server
        .events
        .counters(healthy)
        .ok_or("healthy counters")?;
    let modules: [i32; 3] =
        std::array::from_fn(|slot| host.runtime.server.clients[11 + slot].player.health);
    if source.healthy != 1152
        || source.stalled == 0
        || (disconnected, overflow) != (1, 1)
        || retired < 32
        || healthy_counters.acknowledged_records != 1152
        || host.runtime.server.events.counters(stalled).is_some()
        || host.runtime.server.clients[0].connection.is_some()
        || host
            .runtime
            .network
            .get(ClientId(0), Endpoint::Server)
            .is_some()
        || healthy_counters.overflow != 0
        || maximum != 0
        || bytes != 0
        || host.runtime.server.world_frame != 659
        || modules != [1146, 1150, 1152]
        || continuing_frames != 600
        || resync_frame.is_none_or(|frame| frame >= 60)
        || host.runtime.server.clients[11..14]
            .iter()
            .any(|c| c.player.armor != 0)
    {
        return Err(format!("retirement fidelity: healthy={} stalled={} disconnects={disconnected} overflow={overflow} retired={retired} modules={modules:?} allocations={maximum}", source.healthy, source.stalled).into());
    }
    if host
        .runtime
        .server
        .events
        .acknowledge(stalled, NativeReceipt(7))
        != 0
    {
        return Err("stale ACK".into());
    }
    if !host.runtime.server.disconnect(ClientId(2)) {
        return Err("disconnect".into());
    }
    // Module slots may still retain the last message; disconnect must at least
    // release every HUD display lease, independently of module delivery.
    let display_after_disconnect =
        leased_id.is_some_and(|id| host.runtime.server.events.texts.get(id).is_some());
    samples.sort_unstable();
    let median = if heap_only {
        "null".into()
    } else {
        ((samples[299] + samples[300]) as f64 * 0.5).to_string()
    };
    let p99 = if heap_only {
        "null".into()
    } else {
        samples[593].to_string()
    };
    println!(
        "{{\"scope\":\"headless Com_Frame native print ACK retirement; no sign-on or gameplay\",\"protocol\":\"{protocol:?}\",\"stalled_delivery\":\"{}\",\"warmup\":60,\"frames\":600,\"continuous_healthy_and_server_frames\":{continuing_frames},\"disconnect_frame\":{},\"server_ticks\":{ticks},\"world_frame\":659,\"module_hz\":[10,20,40],\"module_deliveries\":{modules:?},\"healthy_prints\":{},\"healthy_native_acked_records\":{},\"stalled_attempts\":{},\"retired_on_resync\":{retired},\"stalled_overflow\":{overflow},\"disconnected\":{disconnected},\"healthy_overflow\":0,\"stale_texts\":0,\"payload_retained_for_slower_module_after_hud_disconnect\":{display_after_disconnect},\"maximum_allocations\":{maximum},\"maximum_requested_bytes\":{bytes},\"median_ns\":{},\"p99_ns\":{},\"timing_run\":{}}}",
        if unsent {
            "unsent"
        } else {
            "reliable_without_ack"
        },
        resync_frame.ok_or("missing resync")?,
        source.healthy,
        healthy_counters.acknowledged_records,
        source.stalled,
        median,
        p99,
        !heap_only
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let heap_only = std::env::args().any(|arg| arg == "--heap-only");
    let protocol = if std::env::args().any(|arg| arg == "--q3") {
        Protocol::Quake3_68
    } else {
        Protocol::QuakeWorld28
    };
    for unsent in [false, true] {
        run(unsent, protocol, heap_only)?;
    }
    Ok(())
}
