use super::{
    Contents, EntityTraceRules, Trace, TraceQuery,
    brushes::{Brush, BrushMap, BrushTree, GeometryError, ModelRoot},
    hulls::{HullError, HullModel, HullScratch, Q1Hulls},
    tree::BrushScratch,
};
use qa_core::{
    math::{AngleBasis, angle_vectors_radians, radians_from_degrees, radians_from_degrees_f32},
    primitives::{
        Bounds, ClipNode, GeometryId, ModelRotation, ModelRules, Plane, SurfaceFlags, Vec3,
    },
};
use std::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    Hull(HullError),
    Brush(GeometryError),
    Models,
    Bounds,
    Capacity,
}

enum Geometry {
    Hulls(Q1Hulls),
    Brushes(BrushMap),
}

#[derive(Clone, Copy)]
enum Root {
    Hull(HullModel),
    Brush(ModelRoot),
}

struct Model {
    root: Root,
    bounds: Bounds,
}

struct Resource {
    geometry: Geometry,
    models: Range<usize>,
}

struct Slot {
    generation: u32,
    resource: Option<Resource>,
}

/// All loaded resources share one flat model table. Native model ordinals stay
/// local to the generation-checked resource, regardless of geometry topology.
#[derive(Default)]
pub struct CollisionStore {
    slots: Vec<Slot>,
    models: Vec<Model>,
}

/// Each caller owns both workspaces, sized once for all registered resources.
pub struct TraceScratch {
    hulls: HullScratch,
    brushes: BrushScratch,
}

fn validate_bounds(count: usize, bounds: &[Bounds]) -> Result<(), StoreError> {
    if count == 0 || bounds.len() != count {
        return Err(StoreError::Models);
    }
    if bounds.iter().any(|bounds| {
        (0..3).any(|axis| {
            !bounds.mins.0[axis].is_finite()
                || !bounds.maxs.0[axis].is_finite()
                || bounds.mins.0[axis] > bounds.maxs.0[axis]
        })
    }) {
        return Err(StoreError::Bounds);
    }
    Ok(())
}

impl CollisionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bounds already contain the native load margin, independently of the
    /// later entity-link expansion. Geometry planes remain authored values.
    pub fn load_hulls(
        &mut self,
        planes: Vec<Plane>,
        drawing: Vec<ClipNode>,
        clips: Vec<ClipNode>,
        models: Vec<HullModel>,
        bounds: Vec<Bounds>,
    ) -> Result<GeometryId, StoreError> {
        validate_bounds(models.len(), &bounds)?;
        let geometry = Q1Hulls::load(planes, drawing, clips, &models).map_err(StoreError::Hull)?;
        let models = models
            .into_iter()
            .zip(bounds)
            .map(|(root, bounds)| Model {
                root: Root::Hull(root),
                bounds,
            })
            .collect();
        self.insert(Geometry::Hulls(geometry), models)
    }

    pub fn load_brushes(
        &mut self,
        side_planes: Vec<Plane>,
        brushes: Vec<Brush>,
        surfaces: Vec<SurfaceFlags>,
        tree: BrushTree,
        bounds: Vec<Bounds>,
    ) -> Result<GeometryId, StoreError> {
        validate_bounds(tree.models.len(), &bounds)?;
        let (geometry, models) =
            BrushMap::load_tree(side_planes, brushes, surfaces, tree).map_err(StoreError::Brush)?;
        let models = models
            .into_iter()
            .zip(bounds)
            .map(|(root, bounds)| Model {
                root: Root::Brush(root),
                bounds,
            })
            .collect();
        self.insert(Geometry::Brushes(geometry), models)
    }

    fn insert(&mut self, geometry: Geometry, models: Vec<Model>) -> Result<GeometryId, StoreError> {
        let first = self.models.len();
        let end = first
            .checked_add(models.len())
            .ok_or(StoreError::Capacity)?;
        if end > u32::MAX as usize {
            return Err(StoreError::Capacity);
        }
        let vacant = self
            .slots
            .iter()
            .position(|slot| slot.resource.is_none() && slot.generation < u32::MAX);
        let slot = if let Some(slot) = vacant {
            slot
        } else {
            if self.slots.len() >= u32::MAX as usize {
                return Err(StoreError::Capacity);
            }
            self.slots.push(Slot {
                generation: 1,
                resource: None,
            });
            self.slots.len() - 1
        };
        self.models.extend(models);
        self.slots[slot].resource = Some(Resource {
            geometry,
            models: first..end,
        });
        Ok(GeometryId {
            slot: slot as u32,
            generation: self.slots[slot].generation,
        })
    }

    fn resource(&self, geometry: GeometryId) -> Option<&Resource> {
        let slot = self.slots.get(geometry.slot as usize)?;
        (slot.generation == geometry.generation)
            .then_some(slot.resource.as_ref())
            .flatten()
    }

    fn resolve(&self, geometry: GeometryId, index: u32) -> Option<(&Geometry, &Model)> {
        let resource = self.resource(geometry)?;
        if index as usize >= resource.models.len() {
            return None;
        }
        Some((
            &resource.geometry,
            &self.models[resource.models.start + index as usize],
        ))
    }

    pub fn model_count(&self, geometry: GeometryId) -> Option<u32> {
        Some(self.resource(geometry)?.models.len() as u32)
    }

    pub fn model_bounds(&self, geometry: GeometryId, index: u32) -> Option<Bounds> {
        Some(self.resolve(geometry, index)?.1.bounds)
    }

    /// Cold removal compacts private rows. No body stores their physical index.
    pub fn remove(&mut self, geometry: GeometryId) -> bool {
        let Some(slot) = self.slots.get_mut(geometry.slot as usize) else {
            return false;
        };
        if slot.generation != geometry.generation {
            return false;
        }
        let Some(resource) = slot.resource.take() else {
            return false;
        };
        slot.generation += 1;
        let first = resource.models.start;
        let count = resource.models.len();
        drop(self.models.drain(resource.models));
        for slot in &mut self.slots {
            if let Some(resource) = &mut slot.resource
                && resource.models.start > first
            {
                resource.models.start -= count;
                resource.models.end -= count;
            }
        }
        true
    }

    fn scratch_dimensions(&self) -> (usize, usize, usize) {
        let mut hull_depth = 0;
        let mut brush_depth = 0;
        let mut brushes = 0;
        for resource in self.slots.iter().filter_map(|slot| slot.resource.as_ref()) {
            match &resource.geometry {
                Geometry::Hulls(geometry) => {
                    hull_depth = hull_depth.max(geometry.scratch_capacity());
                }
                Geometry::Brushes(geometry) => {
                    let (depth, count) = geometry.scratch_capacity();
                    brush_depth = brush_depth.max(depth);
                    brushes = brushes.max(count);
                }
            }
        }
        (hull_depth, brush_depth, brushes)
    }

    fn make_scratch(&self, position_capacity: usize) -> TraceScratch {
        let (hull_depth, brush_depth, brushes) = self.scratch_dimensions();
        TraceScratch {
            hulls: HullScratch::new(hull_depth),
            brushes: BrushScratch::new(brush_depth, brushes, position_capacity),
        }
    }

    pub fn scratch(&self) -> TraceScratch {
        self.make_scratch(1024)
    }

    pub fn scratch_with_position_capacity(
        &self,
        capacity: usize,
    ) -> Result<TraceScratch, StoreError> {
        if capacity > u32::MAX as usize || capacity > isize::MAX as usize / size_of::<u32>() {
            return Err(StoreError::Capacity);
        }
        Ok(self.make_scratch(capacity))
    }

    pub fn trace_model(
        &self,
        geometry: GeometryId,
        index: u32,
        query: TraceQuery,
        scratch: &mut TraceScratch,
    ) -> Trace {
        let Some((geometry, model)) = self.resolve(geometry, index) else {
            return Trace::clear(query.end);
        };
        let offset = match geometry {
            Geometry::Hulls(_) => Some(Q1Hulls::clip_offset(query)),
            Geometry::Brushes(_) => None,
        };
        let local = if let Some(offset) = offset {
            TraceQuery {
                start: query.start - offset,
                end: query.end - offset,
                ..query
            }
        } else {
            query
        };
        let mut trace = trace_native(geometry, model.root, local, scratch);
        if let Some(offset) = offset {
            trace.end = if trace.fraction != 1.0 {
                trace.end + offset
            } else {
                query.end
            };
        }
        trace
    }

    pub fn point_contents_model(
        &self,
        geometry: GeometryId,
        index: u32,
        point: Vec3,
        rules: EntityTraceRules,
    ) -> Contents {
        match self.resolve(geometry, index) {
            Some((
                Geometry::Hulls(hulls),
                Model {
                    root: Root::Hull(model),
                    ..
                },
            )) => hulls.point_contents(*model, point),
            Some((
                Geometry::Brushes(brushes),
                Model {
                    root: Root::Brush(root),
                    ..
                },
            )) => brushes.point_contents_root(*root, point, rules),
            _ => Contents::EMPTY,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Geometry, model, transform and caller rules are independent trace inputs"
    )]
    pub fn trace_transformed(
        &self,
        geometry: GeometryId,
        index: u32,
        query: TraceQuery,
        origin: Vec3,
        angles: Vec3,
        rules: ModelRules,
        scratch: &mut TraceScratch,
    ) -> Trace {
        let Some((geometry, model)) = self.resolve(geometry, index) else {
            return Trace::clear(query.end);
        };
        let mut local = query;
        if rules.rotation == ModelRotation::TransposeBasis {
            for axis in 0..3 {
                let center = (query.mins.0[axis] + query.maxs.0[axis]) * 0.5;
                local.mins.0[axis] = query.mins.0[axis] - center;
                local.maxs.0[axis] = query.maxs.0[axis] - center;
                local.start.0[axis] = query.start.0[axis] + center;
                local.end.0[axis] = query.end.0[axis] + center;
            }
        }
        let offset = match geometry {
            Geometry::Hulls(_) => Q1Hulls::clip_offset(local) + origin,
            Geometry::Brushes(_) => origin,
        };
        local.start = local.start - offset;
        local.end = local.end - offset;
        let rotated = rules.rotation != ModelRotation::TranslationOnly
            && angles.0.iter().any(|&angle| angle != 0.0);
        let matrix = if rotated {
            let basis = basis(angles, rules.rotation);
            let matrix = [basis.forward, Vec3(basis.right.0.map(|v| -v)), basis.up];
            if rules.rotation == ModelRotation::TransposeBasis {
                local.start = rotate(local.start, matrix);
                local.end = rotate(local.end, matrix);
            } else {
                local.start = point_rotate(local.start, basis);
                local.end = point_rotate(local.end, basis);
            }
            Some(matrix)
        } else {
            None
        };
        // The wrapper's symmetric bounds still pass through raw BrushWork
        // centering, including its separate f32 rounding operation.
        let mut trace = trace_native(geometry, model.root, local, scratch);
        if let Some(matrix) = matrix
            && trace.fraction != 1.0
        {
            trace.plane.normal = if rules.rotation == ModelRotation::NegativeEuler {
                point_rotate(
                    trace.plane.normal,
                    basis(Vec3(angles.0.map(|v| -v)), rules.rotation),
                )
            } else {
                let transpose = std::array::from_fn(|row| {
                    Vec3(std::array::from_fn(|column| matrix[column].0[row]))
                });
                rotate(trace.plane.normal, transpose)
            };
        }
        trace.end = if rules.rotation == ModelRotation::TranslationOnly {
            if trace.fraction != 1.0 {
                trace.end + offset
            } else {
                query.end
            }
        } else {
            query.start.lerp(query.end, trace.fraction)
        };
        trace
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Geometry, model, transform and caller rules are independent contents inputs"
    )]
    pub fn point_contents_transformed(
        &self,
        geometry: GeometryId,
        index: u32,
        point: Vec3,
        origin: Vec3,
        angles: Vec3,
        rules: ModelRules,
        caller: EntityTraceRules,
    ) -> Contents {
        let mut local = point - origin;
        if rules.rotation != ModelRotation::TranslationOnly
            && angles.0.iter().any(|&angle| angle != 0.0)
        {
            // Both native point-content wrappers negate the right dot after
            // accumulation, unlike Q3's trace matrix with a negated right row.
            local = point_rotate(local, basis(angles, rules.rotation));
        }
        self.point_contents_model(geometry, index, local, caller)
    }
}

fn trace_native(
    geometry: &Geometry,
    root: Root,
    query: TraceQuery,
    scratch: &mut TraceScratch,
) -> Trace {
    match (geometry, root) {
        (Geometry::Hulls(hulls), Root::Hull(model)) => {
            hulls.trace_local(model, query, &mut scratch.hulls)
        }
        (Geometry::Brushes(brushes), Root::Brush(root)) => {
            brushes.trace_root(root, query, &mut scratch.brushes)
        }
        _ => Trace::clear(query.end),
    }
}

fn basis(angles: Vec3, rotation: ModelRotation) -> AngleBasis {
    angle_vectors_radians(if rotation == ModelRotation::TransposeBasis {
        radians_from_degrees_f32(angles)
    } else {
        radians_from_degrees(angles)
    })
}

fn point_rotate(point: Vec3, basis: AngleBasis) -> Vec3 {
    Vec3([
        point.dot(basis.forward),
        -point.dot(basis.right),
        point.dot(basis.up),
    ])
}

fn rotate(point: Vec3, matrix: [Vec3; 3]) -> Vec3 {
    Vec3(matrix.map(|row| row.dot(point)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_geometry_generation_retires_without_reuse() -> Result<(), StoreError> {
        let mut store = CollisionStore::new();
        let bounds = vec![Bounds::default()];
        let geometry = store.load_hulls(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![HullModel { roots: [-1; 3] }],
            bounds.clone(),
        )?;
        store.slots[geometry.slot as usize].generation = u32::MAX - 1;
        let terminal = GeometryId {
            generation: u32::MAX - 1,
            ..geometry
        };
        assert!(store.remove(terminal));
        assert_eq!(store.slots[geometry.slot as usize].generation, u32::MAX);
        let next = store.load_hulls(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![HullModel { roots: [-1; 3] }],
            bounds,
        )?;
        assert_ne!(next.slot, terminal.slot);
        assert_eq!(next.generation, 1);
        assert_eq!(store.model_count(terminal), None);
        assert!(!store.remove(GeometryId {
            generation: u32::MAX,
            ..terminal
        }));
        Ok(())
    }
}
