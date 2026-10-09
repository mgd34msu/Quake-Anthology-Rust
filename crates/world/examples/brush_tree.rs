//! Developer-only native brush-tree comparison. Build this example separately
//! and use tools/check_brush_tree.py. Cold fixture transport is outside the
//! allocation scope. Trees are synthetic, ordered and brush-only; this does not
//! qualify retail gameplay, patches, transformed bodies or native wire formats.
use qa_world::collision::TraceRules;
use std::hint::black_box;

#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;
#[path = "../../../tools/probes/brush_tree_fixture.rs"]
mod fixture;
use fixture::{Fixture, WORDS, load, result};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected q2|q3, tree fixture and result file".into());
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
        _ => return Err("invalid tree caller rules".into()),
    };
    let payload = std::fs::read(&args[2])?;
    let Fixture {
        store,
        geometries,
        queries,
    } = load(&payload, rules, entity_rules)?;
    let mut scratch = store.scratch();
    let mut results = vec![[0u32; WORDS]; queries.len()];
    allocation_counter::start();
    let positive = black_box(Vec::<u8>::with_capacity(black_box(128)));
    let positive_allocations = allocation_counter::stop();
    drop(positive);
    if positive_allocations == 0 {
        return Err("tree allocation positive control did not detect an allocation".into());
    }
    allocation_counter::start();
    for (query, output) in queries.iter().zip(&mut results) {
        let trace = store.trace_model(
            geometries[query.map],
            query.model as u32,
            black_box(query.trace),
            &mut scratch,
        );
        let point = store.point_contents_model(
            geometries[query.map],
            query.model as u32,
            black_box(query.point),
            entity_rules,
        );
        *output = result(black_box(trace), black_box(point));
    }
    let allocations = allocation_counter::stop();
    let contacts = results
        .iter()
        .filter(|row| f32::from_bits(row[0]) < 1.0)
        .count();
    let enclosed = results.iter().filter(|row| row[9] & 2 != 0).count();
    let mut output = Vec::with_capacity(queries.len() * WORDS * 4);
    for row in results {
        for value in row {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(&args[3], output)?;
    println!(
        "{{\"scope\":\"synthetic native brush-tree/leaf/point fixtures; no patches/gameplay/wire/performance\",\"rule\":\"{}\",\"rows\":{},\"maps\":{},\"contacts\":{contacts},\"enclosed\":{enclosed},\"rust_calling_thread_alloc_or_realloc\":{allocations},\"allocation_positive_control\":{positive_allocations}}}",
        args[1],
        queries.len(),
        geometries.len()
    );
    if allocations != 0 {
        return Err("brush-tree allocation gate failed".into());
    }
    Ok(())
}
