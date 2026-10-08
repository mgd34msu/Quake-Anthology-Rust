//! Headless developer comparison fixture, never linked into the engine.
//! Build with `cargo build --release -p qa-world --example brush_trace`, then
//! run through tools/check_brush_trace.py. File transport is cold; the measured
//! allocation scope contains only analytic traces and fixed result writes.
use qa_core::primitives::{Axis, Plane, SurfaceFlags, Vec3};
use qa_world::collision::brushes::{Brush, BrushMap};
use qa_world::collision::{CollisionWorld, Contents, TraceQuery, TraceRules};
use std::hint::black_box;

#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;

#[derive(Clone, Copy)]
struct Query {
    map: usize,
    trace: TraceQuery,
}

fn word(data: &mut &[u8]) -> Result<u32, &'static str> {
    let (head, tail) = data.split_first_chunk::<4>().ok_or("truncated fixture")?;
    *data = tail;
    Ok(u32::from_le_bytes(*head))
}

fn vector(data: &mut &[u8]) -> Result<Vec3, &'static str> {
    Ok(Vec3([
        f32::from_bits(word(data)?),
        f32::from_bits(word(data)?),
        f32::from_bits(word(data)?),
    ]))
}

fn finite(vector: Vec3) -> bool {
    vector.0.iter().all(|value| value.is_finite())
}

fn contents(raw: u32) -> Result<Contents, &'static str> {
    // These fixtures use common exact bits, without claiming file/wire conversion.
    if raw & !(1 | 8 | 16 | 32 | 0x10000 | 0x20000) != 0 {
        return Err("unsupported fixture contents bit");
    }
    Ok(Contents(u64::from(raw)))
}

fn load(
    mut data: &[u8],
    rules: TraceRules,
) -> Result<(Vec<CollisionWorld>, Vec<Query>), &'static str> {
    if word(&mut data)? != 0x48535242 || word(&mut data)? != 1 {
        return Err("unsupported brush fixture");
    }
    let map_count = word(&mut data)? as usize;
    let query_count = word(&mut data)? as usize;
    if !(1..=32).contains(&map_count) || !(10_000..=50_000).contains(&query_count) {
        return Err("fixture count out of bounds");
    }
    let mut worlds = Vec::with_capacity(map_count);
    for _ in 0..map_count {
        let brush_count = word(&mut data)? as usize;
        if !(1..=8).contains(&brush_count) {
            return Err("brush count out of bounds");
        }
        let mut planes = Vec::new();
        let mut surfaces = Vec::new();
        let mut brushes = Vec::with_capacity(brush_count);
        for _ in 0..brush_count {
            let plane_count = word(&mut data)?;
            let brush_contents = contents(word(&mut data)?)?;
            let mins = vector(&mut data)?;
            let maxs = vector(&mut data)?;
            if !(6..=64).contains(&plane_count)
                || !finite(mins)
                || !finite(maxs)
                || mins.0.iter().zip(maxs.0).any(|(min, max)| *min > max)
            {
                return Err("invalid fixture brush bounds");
            }
            brushes.push(Brush {
                first_plane: planes.len() as u32,
                plane_count,
                contents: brush_contents,
            });
            for _ in 0..plane_count {
                let normal = vector(&mut data)?;
                let distance = f32::from_bits(word(&mut data)?);
                let axis = match word(&mut data)? {
                    0 => Some(Axis::X),
                    1 => Some(Axis::Y),
                    2 => Some(Axis::Z),
                    3 => None,
                    _ => return Err("invalid plane axis"),
                };
                planes.push(Plane {
                    normal,
                    distance,
                    axis,
                });
                surfaces.push(SurfaceFlags(word(&mut data)?));
            }
        }
        worlds.push(CollisionWorld::Brushes(
            BrushMap::load_surfaces(planes, brushes, surfaces)
                .map_err(|_| "invalid fixture geometry")?,
        ));
    }
    let mut queries = Vec::with_capacity(query_count);
    for _ in 0..query_count {
        let map = word(&mut data)? as usize;
        let mask = contents(word(&mut data)?)?;
        let start = vector(&mut data)?;
        let end = vector(&mut data)?;
        let mins = vector(&mut data)?;
        let maxs = vector(&mut data)?;
        if map >= worlds.len()
            || ![start, end, mins, maxs].into_iter().all(finite)
            || mins.0.iter().zip(maxs.0).any(|(min, max)| *min > max)
        {
            return Err("invalid fixture query");
        }
        queries.push(Query {
            map,
            trace: TraceQuery {
                start,
                end,
                mins,
                maxs,
                mask,
                rules,
            },
        });
    }
    if !data.is_empty() {
        return Err("trailing fixture bytes");
    }
    Ok((worlds, queries))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected q2|q3, brush fixture, and result path".into());
    }
    let rules = match args[1].as_str() {
        "q2" => TraceRules::LEGACY,
        "q3" => TraceRules::ARENA,
        _ => return Err("expected q2 or q3 rules".into()),
    };
    let payload = std::fs::read(&args[2])?;
    let (mut worlds, queries) = load(&payload, rules)?;
    let mut results = vec![[0u32; 11]; queries.len()];
    let mut contacts = 0usize;
    let mut start_solid = 0usize;
    let mut all_solid = 0usize;
    allocation_counter::start();
    for (query, row) in queries.iter().zip(&mut results) {
        let trace = black_box(worlds[query.map].trace(black_box(query.trace)));
        contacts += usize::from(trace.fraction < 1.0);
        start_solid += usize::from(trace.start_solid);
        all_solid += usize::from(trace.all_solid);
        *row = [
            trace.fraction.to_bits(),
            trace.end.0[0].to_bits(),
            trace.end.0[1].to_bits(),
            trace.end.0[2].to_bits(),
            trace.plane.normal.0[0].to_bits(),
            trace.plane.normal.0[1].to_bits(),
            trace.plane.normal.0[2].to_bits(),
            trace.plane.distance.to_bits(),
            u32::from(trace.start_solid) | u32::from(trace.all_solid) << 1,
            trace.contents.0 as u32,
            trace.surface.0,
        ];
    }
    let allocations = allocation_counter::stop();
    let mut output = Vec::with_capacity(results.len() * 44);
    for row in results {
        for value in row {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(&args[3], output)?;
    println!(
        "{{\"scope\":\"headless convex brush kernels; no BSP traversal/gameplay/install\",\"rule\":\"{}\",\"rows\":{},\"maps\":{},\"contacts\":{contacts},\"start_solid\":{start_solid},\"all_solid\":{all_solid},\"rust_calling_thread_alloc_or_realloc\":{allocations}}}",
        args[1],
        queries.len(),
        worlds.len()
    );
    if allocations != 0 {
        return Err("brush trace allocation gate failed".into());
    }
    Ok(())
}
