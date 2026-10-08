//! Developer-only native linked-hit merge comparison. Build this example
//! separately, then run tools/check_linked_merge.py with the resulting binary.
//! Cold fixture files avoid consuming a terminal/platform input stream. Only
//! merge_linked calls and fixed result writes enter the allocation scope.
use qa_core::primitives::{Axis, EntityId, Plane, SurfaceFlags, Vec3};
use qa_world::collision::{Contents, EntityTraceRules, Trace};
use std::hint::black_box;

#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;

const WORDS: usize = 15;

fn word(data: &mut &[u8]) -> Result<u32, &'static str> {
    let (head, tail) = data
        .split_first_chunk::<4>()
        .ok_or("truncated merge fixture")?;
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

fn input_trace(data: &mut &[u8]) -> Result<Trace, &'static str> {
    let fraction = f32::from_bits(word(data)?);
    let end = vector(data)?;
    let normal = vector(data)?;
    let distance = f32::from_bits(word(data)?);
    let axis = match word(data)? {
        0 => Some(Axis::X),
        1 => Some(Axis::Y),
        2 => Some(Axis::Z),
        u32::MAX => None,
        _ => return Err("invalid fixture plane axis"),
    };
    let flags = word(data)?;
    let contents = u64::from(word(data)?) | u64::from(word(data)?) << 32;
    let slot = word(data)?;
    let generation = word(data)?;
    let surface = SurfaceFlags(word(data)?);
    if !fraction.is_finite()
        || !(0.0..=1.0).contains(&fraction)
        || !end.0.iter().chain(&normal.0).all(|value| value.is_finite())
        || !distance.is_finite()
        || flags & !31 != 0
        || (slot == u32::MAX && generation != 0)
        || (slot != u32::MAX && slot > i32::MAX as u32)
    {
        return Err("invalid normalized merge fixture");
    }
    Ok(Trace {
        fraction,
        end,
        plane: Plane {
            normal,
            distance,
            axis,
        },
        start_solid: flags & 1 != 0,
        all_solid: flags & 2 != 0,
        in_open: flags & 4 != 0,
        in_water: flags & 8 != 0,
        contents: Contents(contents),
        entity: (slot != u32::MAX).then_some(EntityId { slot, generation }),
        surface,
        brush_solid: flags & 16 != 0,
    })
}

fn output_trace(trace: Trace) -> [u32; WORDS] {
    let (slot, generation) = trace
        .entity
        .map_or((u32::MAX, 0), |id| (id.slot, id.generation));
    [
        trace.fraction.to_bits(),
        trace.end.0[0].to_bits(),
        trace.end.0[1].to_bits(),
        trace.end.0[2].to_bits(),
        trace.plane.normal.0[0].to_bits(),
        trace.plane.normal.0[1].to_bits(),
        trace.plane.normal.0[2].to_bits(),
        trace.plane.distance.to_bits(),
        match trace.plane.axis {
            Some(Axis::X) => 0,
            Some(Axis::Y) => 1,
            Some(Axis::Z) => 2,
            None => u32::MAX,
        },
        u32::from(trace.start_solid)
            | u32::from(trace.all_solid) << 1
            | u32::from(trace.in_open) << 2
            | u32::from(trace.in_water) << 3
            | u32::from(trace.brush_solid) << 4,
        trace.contents.0 as u32,
        (trace.contents.0 >> 32) as u32,
        slot,
        generation,
        trace.surface.0,
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected q1|q2|q3, merge fixture, and result path".into());
    }
    let rules = match args[1].as_str() {
        "q1" => EntityTraceRules::QUAKE,
        "q2" => EntityTraceRules::QUAKE2,
        "q3" => EntityTraceRules::ARENA,
        _ => return Err("invalid linked merge rule".into()),
    };
    let payload = std::fs::read(&args[2])?;
    let mut data = payload.as_slice();
    if word(&mut data)? != 0x474d4b4c || word(&mut data)? != 1 {
        return Err("unsupported linked merge fixture".into());
    }
    let count = word(&mut data)? as usize;
    if !(1..=50_000).contains(&count) {
        return Err("merge fixture count out of bounds".into());
    }
    let mut cases = Vec::with_capacity(count);
    for _ in 0..count {
        cases.push((input_trace(&mut data)?, input_trace(&mut data)?));
    }
    if !data.is_empty() {
        return Err("trailing linked merge fixture bytes".into());
    }
    let mut results = vec![[0u32; WORDS]; count];
    let mut changed = 0usize;
    allocation_counter::start();
    for (&(current, incoming), output) in cases.iter().zip(&mut results) {
        let mut merged = black_box(current);
        merged.merge_linked(black_box(incoming), rules);
        *output = output_trace(black_box(merged));
        changed += usize::from(*output != output_trace(current));
    }
    let allocations = allocation_counter::stop();
    let mut output = Vec::with_capacity(count * WORDS * 4);
    for row in results {
        for value in row {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(&args[3], output)?;
    println!(
        "{{\"scope\":\"headless linked merge statement blocks; not SV_Trace/gameplay/performance\",\"rule\":\"{}\",\"rows\":{count},\"changed_rows\":{changed},\"rust_calling_thread_alloc_or_realloc\":{allocations}}}",
        args[1]
    );
    if allocations != 0 {
        return Err("linked merge allocation gate failed".into());
    }
    Ok(())
}
