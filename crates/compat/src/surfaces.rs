//! Load-built native surface views; collision and names remain store-owned.
use crate::{memory::ModuleMemory, services::CallError};
use qa_core::primitives::{GeometryId, SurfaceId};
use qa_world::collision::CollisionStore;

struct Range {
    geometry: GeometryId,
    first: usize,
    count: u32,
}
pub struct NativeSurfaces {
    address: u64,
    stride: usize,
    ranges: Box<[Range]>,
}
impl NativeSurfaces {
    pub fn byte_length(store: &CollisionStore, wide: bool) -> Option<usize> {
        store
            .geometries()
            .try_fold(1usize, |total, id| {
                total.checked_add(store.surface_count(id)? as usize)
            })?
            .checked_mul(if wide { 60 } else { 24 })
    }
    pub fn load(
        address: u64,
        store: &CollisionStore,
        wide: bool,
        memory: &mut ModuleMemory<'_>,
    ) -> Result<Self, CallError> {
        let stride = if wide { 60 } else { 24 };
        let mut ranges = Vec::new();
        let mut first = 1;
        memory.read_mut(address, stride)?.fill(0); // native CM nullsurface
        for geometry in store.geometries() {
            let count = store.surface_count(geometry).ok_or(CallError::Geometry)?;
            ranges.push(Range {
                geometry,
                first,
                count,
            });
            for index in 0..count {
                let row = store
                    .surface(SurfaceId { geometry, index })
                    .ok_or(CallError::Geometry)?;
                let at = address
                    .checked_add((first * stride) as u64)
                    .ok_or(CallError::Memory)?;
                let bytes = memory.read_mut(at, stride)?;
                bytes.fill(0);
                let name_bytes = if wide { 32 } else { 16 };
                let length = row.name.len().min(name_bytes - 1);
                bytes[..length].copy_from_slice(&row.name[..length]);
                bytes[name_bytes..name_bytes + 4].copy_from_slice(&row.flags.to_q2().to_le_bytes());
                bytes[name_bytes + 4..name_bytes + 8].copy_from_slice(&row.value.to_le_bytes());
                if wide {
                    bytes[40..44].copy_from_slice(&row.source_index.to_le_bytes());
                    let length = row.material.len().min(15);
                    bytes[44..44 + length].copy_from_slice(&row.material[..length]);
                }
                first += 1;
            }
        }
        Ok(Self {
            address,
            stride,
            ranges: ranges.into_boxed_slice(),
        })
    }
    pub fn address(&self, surface: Option<SurfaceId>) -> Result<u64, CallError> {
        let Some(surface) = surface else {
            return Ok(self.address);
        };
        let index = self
            .ranges
            .binary_search_by_key(&surface.geometry.slot, |range| range.geometry.slot)
            .map_err(|_| CallError::Geometry)?;
        let range = &self.ranges[index];
        if range.geometry != surface.geometry || surface.index >= range.count {
            return Err(CallError::Geometry);
        }
        self.address
            .checked_add(((range.first + surface.index as usize) * self.stride) as u64)
            .ok_or(CallError::Memory)
    }
}
