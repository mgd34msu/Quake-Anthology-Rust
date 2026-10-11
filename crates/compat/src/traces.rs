//! Native trace layouts over the one collision result and module byte view.
use crate::{memory::ModuleMemory, services::CallError, surfaces::NativeSurfaces};
use qa_core::primitives::{Plane, Vec3};
use qa_world::collision::Trace;

fn plane(memory: &mut ModuleMemory<'_>, address: u64, value: Plane) -> Result<(), CallError> {
    memory.write_vec3(address, value.normal)?;
    memory.write_word(address + 12, value.distance.to_bits() as i32)?;
    memory.write(address + 16, &value.type_sign())?;
    Ok(())
}

/// API-3/2023 returns these large structs through the native hidden output
/// pointer. The result layout is selected in the import table, never from the map.
pub fn write_q2<const WIDE: bool>(
    memory: &mut ModuleMemory<'_>,
    address: u64,
    trace: Trace,
    surfaces: &NativeSurfaces,
    entity: u64,
) -> Result<u64, CallError> {
    // CM's zeroed trace has a null surface pointer; a box contact instead
    // points to CM nullsurface. Position/allsolid tests do not invent a plane.
    let surface = if trace.plane.normal != Vec3::default() {
        surfaces.address(trace.surface_id)?
    } else {
        0
    };
    let secondary = if WIDE && trace.secondary_plane.is_some() {
        surfaces.address(trace.secondary_surface_id)?
    } else {
        0
    };
    let (bytes, fraction, end, normal, surface_at, contents, entity_at) = if WIDE {
        (96, 4, 8, 20, 40, 48, 56)
    } else {
        (72, 8, 12, 24, 48, 56, 64)
    };
    memory.read_mut(address, bytes)?.fill(0);
    if WIDE {
        memory.write(
            address,
            &[u8::from(trace.all_solid), u8::from(trace.start_solid)],
        )?;
    } else {
        memory.write_word(address, i32::from(trace.all_solid))?;
        memory.write_word(address + 4, i32::from(trace.start_solid))?;
    }
    memory.write_word(address + fraction, trace.fraction.to_bits() as i32)?;
    memory.write_vec3(address + end, trace.end)?;
    plane(memory, address + normal, trace.plane)?;
    memory.write(address + surface_at, &surface.to_le_bytes())?;
    memory.write_word(address + contents, trace.contents.to_q2() as i32)?;
    memory.write(address + entity_at, &entity.to_le_bytes())?;
    if WIDE {
        if let Some(value) = trace.secondary_plane {
            plane(memory, address + 64, value)?;
        }
        memory.write(address + 88, &secondary.to_le_bytes())?;
    }
    Ok(address)
}
