//! Original NetQuake payload comparison and bounded local transport timing.
use qa_core::{
    loopback::{Endpoint, Loopback, LoopbackLimits, SendError},
    primitives::ClientId,
    sys_events::{EventKind, EventTime, Peer, SysEventQueue},
};
use std::{hint::black_box, io::Write};
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("bench") {
        return bench();
    }
    let mut local = Loopback::load(
        [LoopbackLimits {
            maximum_message: 8000,
            payload_bytes: 8192,
            messages: 16,
        }; 4],
    )
    .map_err(|_| "local capacity")?;
    let mut queue = SysEventQueue::load(16, 32768).map_err(|_| "queue capacity")?;
    let mut bytes = [0u8; 8000];
    let mut state = 0x1357_2468u32;
    let mut output = std::io::stdout().lock();
    for case in 0..1000u32 {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let length = match case {
            0 => 0,
            1 => 1,
            2 => 3,
            3 => 1401,
            4 => 8000,
            _ => (state % 8001) as usize,
        };
        for (index, byte) in bytes[..length].iter_mut().enumerate() {
            *byte = case.wrapping_mul(17).wrapping_add(index as u32 * 29) as u8;
        }
        let id = ClientId(case % 4);
        local
            .send(Endpoint::Client, id, &bytes[..length])
            .map_err(|_| "send")?;
        if local.enqueue(&mut queue, EventTime(u64::from(case))) != 1 {
            return Err("local admission".into());
        }
        let event = queue.pop().ok_or("local dispatch")?;
        let EventKind::Packet {
            from: Peer::Loopback(client),
            socket,
            bytes: received,
        } = event.kind
        else {
            return Err("packet identity".into());
        };
        if client != id || socket != Endpoint::Server.socket() || received != &bytes[..length] {
            return Err("packet fidelity".into());
        }
        output.write_all(&case.to_le_bytes())?;
        output.write_all(&id.0.to_le_bytes())?;
        output.write_all(&(length as u32).to_le_bytes())?;
        output.write_all(received)?;
    }
    Ok(())
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn bench() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::{
        Stopwatch,
        allocations::{begin_frame, end_frame},
    };
    let sizes = [8000, 64000, 1400, 8000];
    let mut local = Loopback::load(sizes.map(|size| LoopbackLimits {
        maximum_message: size,
        payload_bytes: size * 2,
        messages: 2,
    }))
    .map_err(|_| "local capacity")?;
    let mut queue = SysEventQueue::load(16, 256000).map_err(|_| "queue capacity")?;
    let bytes: [Box<[u8]>; 4] =
        std::array::from_fn(|index| vec![index as u8 + 1; sizes[index]].into_boxed_slice());
    begin_frame();
    let control = black_box(vec![0u8; 128]);
    let positive = end_frame();
    drop(control);
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut samples = [0u64; 600];
    let mut allocations = 0;
    let mut requested_bytes = 0;
    let mut delivered = 0u64;
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        for (client, payload) in bytes.iter().enumerate() {
            for _ in 0..2 {
                local
                    .send(Endpoint::Client, ClientId(client as u32), payload)
                    .map_err(|_| "send")?;
            }
            if local.send(Endpoint::Client, ClientId(client as u32), b"") != Err(SendError::Full) {
                return Err("bounded admission".into());
            }
        }
        if local.enqueue(&mut queue, EventTime(frame)) != 8 {
            return Err("queue admission".into());
        }
        let mut count = 0;
        while let Some(event) = queue.pop() {
            let EventKind::Packet {
                from: Peer::Loopback(client),
                socket,
                bytes: received,
            } = event.kind
            else {
                return Err("packet identity".into());
            };
            if client.0 as usize != count / 2
                || socket != Endpoint::Server.socket()
                || received != &*bytes[client.0 as usize]
            {
                return Err("payload fidelity".into());
            }
            delivered += received.len() as u64;
            black_box(received);
            count += 1;
        }
        if count != 8 || local.pending(Endpoint::Server) != 0 {
            return Err("transport drain".into());
        }
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= 60 {
            samples[frame as usize - 60] = elapsed;
            allocations += counts.allocations + counts.reallocations;
            requested_bytes += counts.requested_bytes;
        }
    }
    if allocations != 0
        || requested_bytes != 0
        || (0..4).any(|id| local.full(Endpoint::Server, ClientId(id)) != Some(660))
    {
        return Err("transport gate".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"four-client local transport and full-byte fidelity including 8k/64k messages; no native signon\",\"warmup\":60,\"frames\":600,\"messages_per_frame\":8,\"full_per_frame\":4,\"delivered_bytes\":{delivered},\"positive_allocations\":{},\"allocations\":{allocations},\"requested_bytes\":{requested_bytes},\"median_ns\":{},\"p99_ns\":{}}}",
        positive.allocations, samples[300], samples[593]
    );
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn bench() -> Result<(), Box<dyn std::error::Error>> {
    Err("allocation tracking required".into())
}
