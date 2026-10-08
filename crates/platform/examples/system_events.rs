use qa_core::sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEventQueue};
use qa_input::{Input, Target};
use qa_network::ingress::PacketReceiver;
use qa_platform::{EventPump, Stopwatch};
use std::{hint::black_box, net::UdpSocket};

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

#[derive(Default)]
struct Sink {
    chars: u64,
}
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {
        self.chars += 1;
    }
    fn command(&mut self, _: SeatId, _: EventTime, _: &str) {}
}
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{begin_frame, end_frame};
    // Positive control proves the actual allocator instrumentation is active.
    begin_frame();
    let mut positive = Vec::with_capacity(4);
    positive.extend_from_slice(&[1u64; 4]);
    positive.reserve_exact(32);
    black_box(&positive);
    let positive = end_frame();
    if positive.allocations != 1 || positive.reallocations != 1 || positive.requested_bytes != 320 {
        return Err("allocation positive control failed".into());
    }
    let mut input = Input::load();
    input.assign(DeviceId::Controller(42), SeatId::new(1).ok_or("seat")?);
    let mut pump = EventPump::new();
    let (_, address) = pump.bind_udp("127.0.0.1:0".parse()?)?;
    let sender = UdpSocket::bind("127.0.0.1:0")?;
    let mut queue = SysEventQueue::load(1024, 256 * 1024).map_err(|e| format!("{e:?}"))?;
    let mut sink = Sink::default();
    let mut network = PacketReceiver::default();
    let mut samples = [0u128; 600];
    let mut maximum = 0;
    let mut maximum_bytes = 0;
    let mut event_total = 0;
    for frame in 0..660 {
        sender.send_to(b"shared packet", address)?;
        begin_frame();
        let timer = Stopwatch::start();
        for kind in [
            EventKind::Key {
                device: DeviceId::Keyboard,
                code: 26,
                symbol: 119,
                down: true,
                repeat: frame != 0,
            },
            EventKind::ControllerAxis {
                device: DeviceId::Controller(42),
                axis: 0,
                value: 16384,
            },
            EventKind::Char {
                device: DeviceId::Keyboard,
                value: 'λ',
            },
        ] {
            pump.enqueue(&mut queue, kind)
                .map_err(|e| format!("{e:?}"))?;
        }
        pump.poll_network(&mut queue);
        pump.enqueue(&mut queue, EventKind::Time)
            .map_err(|e| format!("{e:?}"))?;
        let mut frame_time = qa_core::sys_events::EventTime::default();
        while let Some(event) = queue.pop() {
            event_total += 1;
            match event.kind {
                EventKind::Time => frame_time = event.time,
                EventKind::Packet {
                    socket,
                    from,
                    bytes,
                } => network.receive(socket, from, bytes, event.time),
                _ => input.dispatch(event, &mut sink),
            }
        }
        let commands = input.build_frame(frame_time, [200; 3], [0.022; 2], [None; 4]);
        black_box(commands);
        let elapsed = timer.elapsed().as_nanos();
        let counts = end_frame();
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            maximum = maximum.max(counts.allocations + counts.reallocations);
            maximum_bytes = maximum_bytes.max(counts.requested_bytes);
        }
        if commands[1].movement != [0, 100, 0] {
            return Err("seat routing failed".into());
        }
    }
    samples.sort_unstable();
    if network.packets != 660
        || sink.chars != 660
        || event_total != 3300
        || maximum != 0
        || maximum_bytes != 0
        || pump.dropped_packets() != 0
        || queue.rejected() != 0
    {
        return Err(format!(
            "event qualification failed: packets={}, chars={}, events={}, allocations={maximum}",
            network.packets, sink.chars, event_total
        )
        .into());
    }
    println!(
        "{{\"scope\":\"headless system-event drain, seats and loopback UDP; not gameplay\",\"warmup\":60,\"frames\":600,\"events_per_frame\":5,\"packets\":{},\"characters\":{},\"positive_allocations\":{},\"positive_reallocations\":{},\"maximum_allocations\":{maximum},\"maximum_requested_bytes\":{maximum_bytes},\"median_ns\":{},\"p99_ns\":{}}}",
        network.packets,
        sink.chars,
        positive.allocations,
        positive.reallocations,
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593]
    );
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    println!("Run with --features allocation-tracking for the measured allocation check.");
}
