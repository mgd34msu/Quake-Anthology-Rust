//! Prepared world-space decal projection over the shared render geometry.
use crate::{
    Assets,
    affine::{self, EdgeOrder, PolygonSide},
    world::{World, WorldPresentation, WorldView, geometry::GeometryKind},
};
use qa_core::{
    math::{cross, normalized_fast, normalized_or_zero},
    primitives::{Bounds, IndexSpan, Plane, Vec3},
    stamps::StampSet,
};
use qa_world::leaves::LeafScratch;

const POLYGON_POINTS: usize = 64;
const CANDIDATE_SURFACES: usize = 64;

#[derive(Debug)]
pub enum MarkError<E> {
    Boundary(E),
    Invalid,
    Scratch,
}

/// All input reads finish before the caller lends its output memory.
pub struct MarkProjection {
    bounds: Bounds,
    direction: Vec3,
    planes: [Plane; POLYGON_POINTS + 2],
    count: usize,
}
impl MarkProjection {
    pub fn prepare<E>(
        point_count: usize,
        projection: Vec3,
        mut point: impl FnMut(usize) -> Result<Vec3, E>,
    ) -> Result<Self, MarkError<E>> {
        if point_count == 0 || !projection.is_finite() {
            return Err(MarkError::Invalid);
        }
        let direction = normalized_or_zero(projection);
        let mut points = [Vec3::default(); POLYGON_POINTS];
        let mut bounds = Bounds::empty();
        for index in 0..point_count {
            let point = point(index).map_err(MarkError::Boundary)?;
            let projected = point + projection;
            let front = point + direction * -20.0;
            if !point.is_finite() || !projected.is_finite() || !front.is_finite() {
                return Err(MarkError::Invalid);
            }
            bounds.add_point(point);
            bounds.add_point(projected);
            bounds.add_point(front);
            if let Some(slot) = points.get_mut(index) {
                *slot = point;
            }
        }
        let count = point_count.min(POLYGON_POINTS);
        let mut planes = [Plane::oriented(Vec3::default(), 0.0); POLYGON_POINTS + 2];
        for index in 0..count {
            let first = points[index];
            let edge = points[(index + 1) % count] - first;
            // Preserve native subtraction after adding the projection.
            let back = first - (first + projection);
            let normal = normalized_fast(cross(edge, back));
            let distance = normal.dot(first);
            if !normal.is_finite() || !distance.is_finite() {
                return Err(MarkError::Invalid);
            }
            planes[index] = Plane::oriented(normal, distance);
        }
        planes[count] = Plane::oriented(direction, direction.dot(points[0]) - 32.0);
        planes[count + 1] = Plane::oriented(-direction, (-direction).dot(points[0]) - 20.0);
        if planes[count..count + 2]
            .iter()
            .any(|plane| !plane.distance.is_finite())
        {
            return Err(MarkError::Invalid);
        }
        Ok(Self {
            bounds,
            direction,
            planes,
            count: count + 2,
        })
    }
}

pub(crate) struct MarkScratch {
    leaves: LeafScratch,
    seen: StampSet,
    candidates: [u32; CANDIDATE_SURFACES],
    clips: [[Vec3; POLYGON_POINTS]; 2],
    distances: [f32; POLYGON_POINTS],
}
impl MarkScratch {
    pub(crate) fn load(world: &World) -> Self {
        Self {
            leaves: LeafScratch::new(world.visibility().node_count() + 1),
            seen: StampSet::new(world.geometry().surfaces.len()),
            candidates: [0; CANDIDATE_SURFACES],
            clips: [[Vec3::default(); POLYGON_POINTS]; 2],
            distances: [0.; POLYGON_POINTS],
        }
    }
}
fn side(distance: f32) -> PolygonSide {
    if distance > 0.5 {
        PolygonSide::Front
    } else if distance < -0.5 {
        PolygonSide::Back
    } else {
        PolygonSide::On
    }
}
fn clip<E>(
    input: &[Vec3],
    output: &mut [Vec3],
    distances: &mut [f32],
    plane: Plane,
) -> Result<usize, MarkError<E>> {
    // Native reserves two vertices before either all-front/all-on shortcut.
    if input.len() >= POLYGON_POINTS - 2 {
        return Ok(0);
    }
    let mut front = 0;
    let mut back = 0;
    for (distance, &point) in distances.iter_mut().zip(input) {
        *distance = plane.geometric_distance(point);
        if !distance.is_finite() {
            return Err(MarkError::Invalid);
        }
        front += usize::from(side(*distance) == PolygonSide::Front);
        back += usize::from(side(*distance) == PolygonSide::Back);
    }
    if front == 0 {
        return Ok(0);
    }
    if back == 0 {
        output[..input.len()].copy_from_slice(input);
        return Ok(input.len());
    }
    let mut count = 0;
    affine::polygon_edges(
        input,
        |index, _| distances[index],
        |distance| Some(side(distance)),
        EdgeOrder::CurrentNext,
        |point| {
            let Some(slot) = output.get_mut(count) else {
                return false;
            };
            *slot = point;
            count += 1;
            true
        },
        |_| true,
        |first, second, from, to| {
            let denominator = from - to;
            let fraction = if denominator == 0.0 {
                0.0
            } else {
                from / denominator
            };
            let point = first.lerp(second, fraction);
            point.is_finite().then_some(point)
        },
    )
    .ok_or(MarkError::Scratch)?;
    Ok(count)
}
impl WorldView {
    #[allow(clippy::too_many_arguments)]
    pub fn mark_fragments<E>(
        &mut self,
        world: &World,
        presentation: &WorldPresentation,
        assets: &Assets,
        projection: &MarkProjection,
        max_points: usize,
        max_fragments: usize,
        emit: &mut dyn FnMut(IndexSpan, &[Vec3]) -> Result<(), E>,
    ) -> Result<usize, MarkError<E>> {
        if max_points == 0 || max_fragments == 0 {
            return Ok(0);
        }
        let geometry = world.geometry();
        let Some(model) = geometry.models.first() else {
            return Ok(0);
        };
        let world_surfaces = model.surfaces.range();
        if presentation.bindings().len() != geometry.surfaces.len()
            || self.marks.seen.len() != geometry.surfaces.len()
        {
            return Err(MarkError::Scratch);
        }
        let MarkScratch {
            leaves,
            seen,
            candidates,
            clips,
            distances,
        } = &mut self.marks;
        seen.begin();
        let mut candidate_count = 0;
        let mut invalid = false;
        world
            .visibility()
            .visit_box_surfaces(projection.bounds, leaves, |id| {
                if !world_surfaces.contains(&(id as usize)) {
                    return true;
                }
                if seen.test_and_set(id as usize) {
                    return true;
                }
                let surface = &geometry.surfaces[id as usize];
                let Some(material) = assets.material(presentation.bindings()[id as usize].material)
                else {
                    invalid = true;
                    return false;
                };
                let settings = material.settings;
                // These are compiled native shader capabilities, never legacy BSP flags.
                if settings.surface_flags & (0x10 | 0x20) != 0 || settings.content_flags & 64 != 0 {
                    return true;
                }
                match surface.kind {
                    GeometryKind::Polygon => {
                        let Some(plane) = surface.plane else {
                            return true;
                        };
                        if qa_world::leaves::box_sides(projection.bounds, plane) != 3
                            || plane.normal.dot(projection.direction) > -0.5
                        {
                            return true;
                        }
                    }
                    GeometryKind::Patch => {}
                    GeometryKind::Triangles | GeometryKind::Flare => return true,
                }
                candidates[candidate_count] = id;
                candidate_count += 1;
                candidate_count < candidates.len()
            })
            .map_err(|_| MarkError::Scratch)?;
        if invalid {
            return Err(MarkError::Invalid);
        }
        let mut returned_points = 0usize;
        let mut returned_fragments = 0usize;
        for &id in &candidates[..candidate_count] {
            let surface = &geometry.surfaces[id as usize];
            for (index, triangle) in geometry.indices[surface.indices.range()]
                .chunks_exact(3)
                .enumerate()
            {
                for (slot, &vertex) in clips[0].iter_mut().zip(triangle) {
                    let vertex = &geometry.vertices[vertex as usize].vertex;
                    let normal = surface.plane.map_or(vertex.normal, |plane| plane.normal);
                    // MARKER_OFFSET is zero; retain VectorMA's signed-zero arithmetic.
                    *slot = vertex.position + normal * 0.0;
                }
                if surface.kind == GeometryKind::Patch {
                    let triangle = &clips[0];
                    let normal = normalized_fast(cross(
                        triangle[0] - triangle[1],
                        triangle[2] - triangle[1],
                    ));
                    let threshold = if index & 1 == 0 { -0.1 } else { -0.05 };
                    if !(f64::from(normal.dot(projection.direction)) < threshold) {
                        continue;
                    }
                }
                let mut count = 3;
                let mut active = 0;
                for &plane in &projection.planes[..projection.count] {
                    let (first, second) = clips.split_at_mut(1);
                    let (input, output) = if active == 0 {
                        (&first[0], &mut second[0])
                    } else {
                        (&second[0], &mut first[0])
                    };
                    count = clip(&input[..count], output, distances, plane)?;
                    active ^= 1;
                    if count == 0 {
                        break;
                    }
                }
                if count == 0 || count > max_points.saturating_sub(returned_points) {
                    continue;
                }
                let first = u32::try_from(returned_points).map_err(|_| MarkError::Scratch)?;
                let count_u32 = u32::try_from(count).map_err(|_| MarkError::Scratch)?;
                emit(
                    IndexSpan {
                        first,
                        count: count_u32,
                    },
                    &clips[active][..count],
                )
                .map_err(MarkError::Boundary)?;
                returned_points += count;
                returned_fragments += 1;
                if returned_fragments == max_fragments {
                    return Ok(returned_fragments);
                }
            }
        }
        Ok(returned_fragments)
    }
}
