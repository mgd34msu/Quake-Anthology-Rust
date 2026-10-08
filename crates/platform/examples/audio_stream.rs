//! Private SDL3 PCM transport probe, not game cue or mixer qualification.
use qa_platform::{AudioStream, Stopwatch, pause};
use std::{num::NonZeroU32, time::Duration};
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{begin_frame, end_frame};
    // Refuse to start this developer probe on any default or physical sink.
    if !matches!(
        std::env::var("SDL_AUDIO_DRIVER")
            .or_else(|_| std::env::var("SDL_AUDIODRIVER"))
            .as_deref(),
        Ok("disk" | "dummy")
    ) {
        return Err("probe requires an explicit private audio driver".into());
    }
    let mut stream = AudioStream::open(NonZeroU32::new(48000).unwrap(), 2)?;
    let mut pcm = [0i16; 960];
    for (i, frame) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let sample = ((i as f32 * std::f32::consts::TAU / 120.0).sin() * 2048.0) as i16;
        frame.fill(sample);
    }
    let mut samples = [0u64; 600];
    let mut peak = 0;
    let mut maximum_allocations = 0;
    let mut maximum_bytes = 0;
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        let written = stream.write(&pcm);
        let queued = stream.queued_bytes();
        let ns = timer.elapsed().as_nanos() as u64;
        let allocations = end_frame();
        if !written || queued.is_none() {
            return Err("stream delivery failed".into());
        }
        peak = peak.max(queued.unwrap());
        if frame >= 60 {
            samples[frame - 60] = ns;
            maximum_allocations =
                maximum_allocations.max(allocations.allocations + allocations.reallocations);
            maximum_bytes = maximum_bytes.max(allocations.requested_bytes);
        }
        // Keep mixahead bounded; this is outside the measured transport stage.
        while stream.queued_bytes().is_some_and(|bytes| bytes > 1920 * 4) {
            pause(Duration::from_millis(2));
        }
        pause(Duration::from_millis(10));
    }
    pause(Duration::from_millis(100));
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"private SDL3 PCM transport only\",\"warmup\":60,\"frames\":600,\"median_ns\":{},\"p99_ns\":{},\"maximum_allocations\":{maximum_allocations},\"maximum_requested_bytes\":{maximum_bytes},\"peak_queued_bytes\":{peak}}}",
        (samples[299] + samples[300]) / 2,
        samples[593]
    );
    if maximum_allocations != 0 || maximum_bytes != 0 {
        return Err("Rust allocation gate failed".into());
    }
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() {
    eprintln!("build this developer probe with allocation-tracking");
}
