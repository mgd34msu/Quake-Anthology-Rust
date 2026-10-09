//! Original command arithmetic and mixed human/bot builder qualification.
use qa_core::{
    primitives::{CommandIntent, RuleSetId},
    sys_events::EventTime,
};
use qa_input::{InputPolicy, UserCmdBuilder};
use std::{hint::black_box, time::Duration};
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

fn next(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed
}
fn intent(seed: &mut u32) -> CommandIntent {
    let fractions: [f32; 8] = std::array::from_fn(|_| (next(seed) % 1025) as f32 / 1024.0);
    CommandIntent {
        movement: [
            [fractions[6], fractions[7]],
            [fractions[2], fractions[3]],
            [fractions[4], fractions[5]],
        ],
        strafe: [fractions[0], fractions[1]],
        speed_modifier: next(seed) & 1 != 0,
        ..Default::default()
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).ok_or("native rule id or bench")?;
    if arg == "bench" {
        return bench();
    }
    let index: usize = arg.parse()?;
    let rules = *RuleSetId::ALL.get(index).ok_or("native rule id")?;
    let policy = InputPolicy::native(rules);
    let mut seed = 0x434d4449;
    for _ in 0..10000 {
        let command = UserCmdBuilder::build(
            Duration::from_millis(16),
            EventTime(16_000_000),
            intent(&mut seed),
            policy,
        );
        println!(
            "{:08x} {:08x} {:08x} {}",
            command.movement[0].to_bits(),
            command.movement[1].to_bits(),
            command.movement[2].to_bits(),
            command.buttons
        );
    }
    Ok(())
}
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn bench() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::{
        Stopwatch,
        allocations::{begin_frame, end_frame},
    };
    begin_frame();
    let control = black_box(vec![0u8; 128]);
    let positive = end_frame();
    drop(control);
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut seed = 0x434d4449;
    let intents: [CommandIntent; 64] = std::array::from_fn(|_| intent(&mut seed));
    let policies: [InputPolicy; 64] =
        std::array::from_fn(|index| InputPolicy::native(RuleSetId::ALL[index % 5]));
    let mut samples = [0u64; 600];
    let mut allocations = 0;
    let mut bytes = 0;
    let mut checksum = 0u64;
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        for index in 0..64 {
            let command = black_box(UserCmdBuilder::build(
                Duration::from_nanos(16_666_667),
                EventTime(frame * 16_666_667),
                black_box(intents[index]),
                policies[index],
            ));
            checksum = checksum.wrapping_add(u64::from(command.movement[0].to_bits()));
        }
        let ns = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= 60 {
            samples[(frame - 60) as usize] = ns;
            allocations += counts.allocations + counts.reallocations;
            bytes += counts.requested_bytes;
        }
    }
    if allocations != 0 || bytes != 0 {
        return Err("mixed command allocation gate".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"one mixed-rule intent builder; no gameplay or wire framing\",\"clients\":64,\"warmup\":60,\"frames\":600,\"allocations_or_reallocations\":{allocations},\"requested_bytes\":{bytes},\"checksum\":{checksum},\"median_ns\":{},\"p99_ns\":{}}}",
        (samples[299] + samples[300]) as f64 * 0.5,
        samples[593]
    );
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn bench() -> Result<(), Box<dyn std::error::Error>> {
    Err("bench requires allocation-tracking".into())
}
