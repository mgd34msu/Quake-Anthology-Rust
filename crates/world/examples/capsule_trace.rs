//! Original-source collision comparison, without timing or allocation gates.
use qa_core::primitives::{Body, CollisionShape, RuleSetId, Vec3};
use qa_world::collision::{CollisionStore, Contents, TraceQuery, trace_policy};

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
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("expected fixture and result path".into());
    }
    let bytes = std::fs::read(&args[1])?;
    let mut data = bytes.as_slice();
    if word(&mut data)? != 0x4c535043 {
        return Err("invalid capsule fixture".into());
    }
    let count = word(&mut data)?;
    if count > 100_000 {
        return Err("fixture row capacity".into());
    }
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let (rules, entity_rules) = trace_policy(RuleSetId::Quake3);
    let mut output = Vec::with_capacity(count as usize * 44);
    for _ in 0..count {
        let body = Body {
            position: vector(&mut data)?,
            mins: vector(&mut data)?,
            maxs: vector(&mut data)?,
            ..Body::default()
        };
        let mut query = TraceQuery {
            start: vector(&mut data)?,
            end: vector(&mut data)?,
            mins: vector(&mut data)?,
            maxs: vector(&mut data)?,
            mask: Contents::BODY,
            ..TraceQuery::point(Vec3::default(), Vec3::default(), rules, entity_rules)
        };
        if word(&mut data)? != 0 {
            query = query.with_capsule();
        }
        let trace = store.trace_body(
            query,
            CollisionShape::Capsule,
            &body,
            Vec3::default(),
            Default::default(),
            None,
            &mut scratch,
        )?;
        for value in [
            trace.fraction,
            trace.end.0[0],
            trace.end.0[1],
            trace.end.0[2],
            trace.plane.normal.0[0],
            trace.plane.normal.0[1],
            trace.plane.normal.0[2],
            trace.plane.distance,
        ] {
            output.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        for value in [
            u32::from(trace.start_solid) | u32::from(trace.all_solid) << 1,
            trace.contents.to_q3(),
            trace.surface.to_q3(),
        ] {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    if !data.is_empty() {
        return Err("trailing fixture bytes".into());
    }
    std::fs::write(&args[2], output)?;
    println!("{{\"rows\":{count},\"timing_measured\":false,\"allocation_measured\":false}}");
    Ok(())
}
