//! Pinned developer timing over the same native-compared brush-tree fixture.
//! No window, host, worker, input, audio or installed gameplay is created.
use qa_platform::Stopwatch;
use qa_world::collision::TraceRules;
use std::hint::black_box;

#[path = "../../../tools/probes/brush_tree_fixture.rs"]
mod fixture;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

const WARMUP: usize = 60;
const FRAMES: usize = 600;
const QUERIES_PER_FRAME: usize = 64;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[expect(
    clippy::chunks_exact_to_as_chunks,
    reason = "Retain the independent packed-byte fixture and its incomplete-tail handling"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::allocations::{begin_frame, end_frame};
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err("expected q2|q3, fixture, matching native rows and JSON output".into());
    }
    let (rules, entity_rules) = match args[1].as_str() {
        "q2" => (
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
        ),
        "q3" => (
            TraceRules::ARENA,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        ),
        _ => return Err("unknown caller rule".into()),
    };
    let bytes = std::fs::read(&args[2])?;
    let fixture = fixture::load(&bytes, rules, entity_rules)?;
    let expected_bytes = std::fs::read(&args[3])?;
    if expected_bytes.len() != fixture.queries.len() * fixture::WORDS * 4 {
        return Err("native rows do not match the fixture size".into());
    }
    let expected: Vec<[u32; fixture::WORDS]> = expected_bytes
        .chunks_exact(fixture::WORDS * 4)
        .map(|row| {
            std::array::from_fn(|column| {
                let at = column * 4;
                u32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]])
            })
        })
        .collect();
    let mut scratch = fixture.store.scratch();
    // Warm the platform timer's frequency cache before allocation counting.
    let _ = Stopwatch::start().elapsed();
    begin_frame();
    let positive = black_box(Vec::<u8>::with_capacity(black_box(128)));
    let positive_count = end_frame();
    drop(positive);
    if positive_count.allocations != 1 {
        return Err("allocation positive control failed".into());
    }
    // Verify every supplied row before using the rotating measured subset.
    for (query, expected) in fixture.queries.iter().zip(&expected) {
        let geometry = fixture.geometries[query.map];
        let trace =
            fixture
                .store
                .trace_model(geometry, query.model as u32, query.trace, &mut scratch);
        let point = fixture.store.point_contents_model(
            geometry,
            query.model as u32,
            query.point,
            entity_rules,
        );
        if fixture::result(trace, point) != *expected {
            return Err("native row mismatch before timing".into());
        }
    }
    let mut samples = [0u64; FRAMES];
    let mut allocation_calls = 0;
    let mut reallocation_calls = 0;
    let mut requested_bytes = 0;
    let mut mismatches = 0u64;
    let mut contacts = 0u64;
    let mut enclosed = 0u64;
    let mut next_query = 0;
    for frame in 0..WARMUP + FRAMES {
        begin_frame();
        let timer = Stopwatch::start();
        let mut frame_contacts = 0;
        let mut frame_enclosed = 0;
        let mut frame_mismatches = 0;
        for _ in 0..QUERIES_PER_FRAME {
            let query = &fixture.queries[next_query];
            let geometry = fixture.geometries[query.map];
            let trace = fixture.store.trace_model(
                geometry,
                query.model as u32,
                black_box(query.trace),
                &mut scratch,
            );
            let point = fixture.store.point_contents_model(
                geometry,
                query.model as u32,
                black_box(query.point),
                entity_rules,
            );
            let row = black_box(fixture::result(trace, point));
            frame_mismatches += u64::from(row != expected[next_query]);
            frame_contacts += u64::from(trace.fraction < 1.0);
            frame_enclosed += u64::from(trace.all_solid);
            next_query += 1;
            if next_query == fixture.queries.len() {
                next_query = 0;
            }
        }
        let elapsed = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= WARMUP {
            samples[frame - WARMUP] = elapsed;
            allocation_calls += counts.allocations;
            reallocation_calls += counts.reallocations;
            requested_bytes += counts.requested_bytes;
            mismatches += frame_mismatches;
            contacts += frame_contacts;
            enclosed += frame_enclosed;
        }
    }
    samples.sort_unstable();
    let median = (samples[299] as f64 + samples[300] as f64) * 0.5;
    let p99 = samples[593];
    let report = format!(
        "{{\"scope\":\"synthetic brush-tree trace plus point contents, full native row comparison and sampling; no gameplay/retail/workers\",\"rule\":\"{}\",\"maps\":{},\"fixture_queries\":{},\"warmup\":{WARMUP},\"measured_frames\":{FRAMES},\"queries_per_frame\":{QUERIES_PER_FRAME},\"measured_trace_and_point_pairs\":{},\"median_batch_ns\":{median},\"p99_batch_ns\":{p99},\"median_ns_per_pair\":{},\"p99_batch_ns_per_pair\":{},\"native_row_mismatches\":{mismatches},\"contacts\":{contacts},\"enclosed\":{enclosed},\"rust_allocations\":{allocation_calls},\"rust_reallocations\":{reallocation_calls},\"rust_requested_bytes\":{requested_bytes},\"allocation_positive_control\":{}}}\n",
        args[1],
        fixture.geometries.len(),
        fixture.queries.len(),
        FRAMES * QUERIES_PER_FRAME,
        median / QUERIES_PER_FRAME as f64,
        p99 as f64 / QUERIES_PER_FRAME as f64,
        positive_count.allocations,
    );
    std::fs::write(&args[4], &report)?;
    print!("{report}");
    if mismatches != 0 || allocation_calls != 0 || reallocation_calls != 0 || requested_bytes != 0 {
        return Err("native fidelity or hot allocation gate failed".into());
    }
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("build with --features allocation-tracking for the timing gate".into())
}
