//! Developer-only transformed model fidelity/allocation probe and optional
//! pinned batch timings. No host, SDL, workers, input or gameplay is started.
use qa_platform::Stopwatch;
use std::hint::black_box;

#[path = "../../../tools/probes/model_collision_fixture.rs"]
mod fixture;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

const WARMUP: usize = 60;
const FRAMES: usize = 600;
const QUERIES: usize = 64;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{begin_frame, end_frame};
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 && (args.len() != 8 || args[5] != "--timings") {
        return Err("expected q1|q2|q3, fixture, poses, output, optionally --timings native-rows JSON-output".into());
    }
    let rule = fixture::Rule::parse(&args[1])?;
    let bytes = std::fs::read(&args[2])?;
    let poses = std::fs::read(&args[3])?;
    let fixture = fixture::load(rule, &bytes, &poses)?;
    let words = rule.words();
    let mut scratch = fixture.store.scratch();
    let mut results = vec![[0u32; fixture::WORDS]; fixture.queries.len()];
    let expected = if args.len() == 8 {
        let bytes = std::fs::read(&args[6])?;
        if bytes.len() != results.len() * words * 4 {
            return Err("native model row count differs".into());
        }
        Some(
            bytes
                .chunks_exact(words * 4)
                .map(|row| {
                    let mut result = [0u32; fixture::WORDS];
                    for (index, value) in result[..words].iter_mut().enumerate() {
                        let at = index * 4;
                        *value =
                            u32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]]);
                    }
                    result
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let _ = Stopwatch::start().elapsed();
    begin_frame();
    let positive = black_box(Vec::<u8>::with_capacity(black_box(128)));
    let positive_count = end_frame();
    drop(positive);
    if positive_count.allocations != 1 {
        return Err("model allocation positive control failed".into());
    }
    begin_frame();
    for (index, row) in results.iter_mut().enumerate() {
        *row = black_box(fixture.sample(black_box(index), &mut scratch).1);
    }
    let all_counts = end_frame();
    let mut output = Vec::with_capacity(results.len() * words * 4);
    for row in &results {
        for value in &row[..words] {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(&args[4], output)?;
    let mut timing = String::new();
    if let Some(expected) = expected {
        if results
            .iter()
            .zip(&expected)
            .any(|(a, b)| a[..words] != b[..words])
        {
            return Err("native transformed model mismatch before timing".into());
        }
        let mut samples = [0u64; FRAMES];
        let mut allocations = 0u64;
        let mut reallocations = 0u64;
        let mut requested_bytes = 0u64;
        let mut mismatches = 0u64;
        let mut contacts = 0u64;
        let mut enclosed = 0u64;
        let mut next = 0usize;
        for frame in 0..WARMUP + FRAMES {
            begin_frame();
            let timer = Stopwatch::start();
            let mut frame_mismatches = 0;
            let mut frame_contacts = 0;
            let mut frame_enclosed = 0;
            for _ in 0..QUERIES {
                let (trace, row) = black_box(fixture.sample(black_box(next), &mut scratch));
                frame_mismatches += u64::from(row[..words] != expected[next][..words]);
                frame_contacts += u64::from(trace.fraction < 1.0);
                frame_enclosed += u64::from(trace.all_solid);
                next += 1;
                if next == results.len() {
                    next = 0;
                }
            }
            let nanos = timer.elapsed().as_nanos() as u64;
            let counts = end_frame();
            if frame >= WARMUP {
                samples[frame - WARMUP] = nanos;
                allocations += counts.allocations;
                reallocations += counts.reallocations;
                requested_bytes += counts.requested_bytes;
                mismatches += frame_mismatches;
                contacts += frame_contacts;
                enclosed += frame_enclosed;
            }
        }
        samples.sort_unstable();
        let median = (samples[299] as f64 + samples[300] as f64) * 0.5;
        let p99 = samples[593];
        let query_scope = if rule == fixture::Rule::Quake {
            "trace_only"
        } else {
            "trace_and_point_contents"
        };
        timing = format!(
            ",\"timing\":{{\"warmup\":{WARMUP},\"measured_frames\":{FRAMES},\"queries_per_frame\":{QUERIES},\"query_scope\":\"{query_scope}\",\"median_batch_ns\":{median},\"p99_batch_ns\":{p99},\"median_ns_per_query\":{},\"p99_batch_ns_per_query\":{},\"rust_allocations\":{allocations},\"rust_reallocations\":{reallocations},\"rust_requested_bytes\":{requested_bytes},\"native_row_mismatches\":{mismatches},\"contacts\":{contacts},\"enclosed\":{enclosed}}}",
            median / QUERIES as f64,
            p99 as f64 / QUERIES as f64,
        );
        if allocations != 0 || reallocations != 0 || requested_bytes != 0 || mismatches != 0 {
            return Err("model timing fidelity/allocation gate failed".into());
        }
    }
    let report = format!(
        "{{\"scope\":\"synthetic production transformed models; no gameplay/install/nativeheap/workers; Q1 point fields excluded\",\"rule\":\"{}\",\"rows\":{},\"row_words\":{words},\"geometries\":{},\"rust_allocations\":{},\"rust_reallocations\":{},\"rust_requested_bytes\":{},\"allocation_positive_control\":{}{timing}}}\n",
        args[1],
        results.len(),
        fixture.geometries.len(),
        all_counts.allocations,
        all_counts.reallocations,
        all_counts.requested_bytes,
        positive_count.allocations,
    );
    if args.len() == 8 {
        std::fs::write(&args[7], &report)?;
    }
    print!("{report}");
    if all_counts.allocations != 0
        || all_counts.reallocations != 0
        || all_counts.requested_bytes != 0
    {
        return Err("model fixture hot allocation gate failed".into());
    }
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("build with --features allocation-tracking for the model fidelity/allocation gate".into())
}
