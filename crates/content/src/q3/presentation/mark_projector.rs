//! Quake III presentation: mark projector.
//!
//! Donor provenance: `src/content/q3/presentation/mark-projector.ts`.

use crate::md3::normalize_fast3;
use qa_core::math::{
    add3, box_on_plane_side, cross3, dot3, normalize3_or_zero, scale3, sub3, vec3, Bounds, Plane, Vec3,
};
use std::collections::HashSet;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_scene::*;
use crate::q3::presentation::ref_entity::*;

// ---------------------------------------------------------------------------
// mark-projector.ts
// ---------------------------------------------------------------------------

/// Mark surface (`MarkSurface`).
#[derive(Debug, Clone, PartialEq)]
pub enum MarkSurface {
    /// Skipped.
    Skip,
    /// Planar face.
    Face {
        /// Surface flags.
        surface_flags: i32,
        /// Content flags.
        content_flags: i32,
        /// Plane.
        plane: Plane,
        /// Vertices.
        vertices: Vec<MarkVertex>,
        /// Indices.
        indices: Vec<usize>,
    },
    /// Patch grid.
    Grid {
        /// Surface flags.
        surface_flags: i32,
        /// Content flags.
        content_flags: i32,
        /// Mesh.
        mesh: PresentPatchMesh,
    },
}

/// Mark vertex (`MaterialVertex`, minimal mirror: position only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkVertex {
    /// Position.
    pub position: Vec3,
}

/// Patch mesh (`PatchMesh`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentPatchMesh {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Row-major vertices.
    pub vertices: Vec<PatchVertex>,
}

/// Patch vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatchVertex {
    /// Position.
    pub position: Vec3,
    /// Normal.
    pub normal: Vec3,
}

/// BSP child reference (`BspNode.children` entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkChild {
    /// Whether the child is a node (`true`) or leaf (`false`).
    pub is_node: bool,
    /// Index.
    pub index: usize,
}

/// BSP node (`BspNode`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkNode {
    /// Plane index.
    pub plane: usize,
    /// Children.
    pub children: [MarkChild; 2],
}

/// BSP leaf surface range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkLeaf {
    /// First surface.
    pub first_surface: usize,
    /// Surface count.
    pub surface_count: usize,
}

/// Mark geometry map (`MarkGeometry.map`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkMap {
    /// Nodes.
    pub nodes: Vec<MarkNode>,
    /// Planes.
    pub planes: Vec<Plane>,
    /// Leaves.
    pub leaves: Vec<MarkLeaf>,
    /// Leaf surfaces.
    pub leaf_surfaces: Vec<usize>,
    /// Surface count.
    pub surface_count: usize,
}

/// Mark geometry (`MarkGeometry`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkGeometry {
    /// Map.
    pub map: MarkMap,
    /// Surfaces.
    pub surfaces: Vec<MarkSurface>,
}

/// Mark fragment (`MarkFragment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkFragment {
    /// First point.
    pub first_point: usize,
    /// Point count.
    pub point_count: usize,
}

/// Mark fragments (`MarkFragments`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkFragments {
    /// Points.
    pub points: Vec<Vec3>,
    /// Fragments.
    pub fragments: Vec<MarkFragment>,
}

/// Mark projection query (`MarkProjection`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkProjection {
    /// Points.
    pub points: Vec<Vec3>,
    /// Projection.
    pub projection: Vec3,
    /// Maximum points.
    pub max_points: usize,
    /// Maximum fragments.
    pub max_fragments: usize,
}

/// Borrowed mark projection with output stores (`SourceMarkProjection`).
pub trait SourceMarkProjection {
    /// Input point count.
    fn point_count(&self) -> usize;
    /// Maximum points.
    fn max_points(&self) -> usize;
    /// Maximum fragments.
    fn max_fragments(&self) -> usize;
    /// Read an input point.
    fn read_point(&self, index: usize) -> PresentResult<Vec3>;
    /// Read the projection vector.
    fn read_projection(&self) -> Vec3;
    /// Write a fragment.
    fn write_fragment(&mut self, index: usize, fragment: MarkFragment);
    /// Write points.
    fn write_points(&mut self, first_point: usize, points: &[Vec3]);
}

/// Maximum clip vertices.
pub(crate) const MAX_CLIP_VERTICES: usize = 64;

/// Maximum mark surfaces per query.
pub(crate) const MAX_MARK_SURFACES: usize = 64;

/// Clip epsilon.
pub(crate) const CLIP_EPSILON: f32 = 0.5;

pub(crate) fn mark_at<T>(values: &[T], index: usize) -> PresentResult<&T> {
    values
        .get(index)
        .ok_or_else(|| PresentError::range(format!("mark geometry index {index} outside {}", values.len())))
}

pub(crate) fn finite_vector(value: Vec3) -> PresentResult<Vec3> {
    if !(value.x.is_finite() && value.y.is_finite() && value.z.is_finite()) {
        return Err(PresentError::range("mark coordinates must be finite float32 values"));
    }
    Ok(value)
}

pub(crate) fn check_capacity(value: usize) -> PresentResult<()> {
    if value > 1_000_000 {
        return Err(PresentError::range(
            "mark buffer capacity must be an integer in [0, 1000000]",
        ));
    }
    Ok(())
}

pub(crate) fn box_side(bounds: &Bounds, plane: &Plane) -> u32 {
    let axis = if plane.normal.x == 1.0 {
        Some('x')
    } else if plane.normal.y == 1.0 {
        Some('y')
    } else if plane.normal.z == 1.0 {
        Some('z')
    } else {
        None
    };
    if let Some(axis) = axis {
        let (min, max) = match axis {
            'x' => (bounds.min.x, bounds.max.x),
            'y' => (bounds.min.y, bounds.max.y),
            _ => (bounds.min.z, bounds.max.z),
        };
        // q_math.c axial fast path treats max == plane distance as behind.
        if plane.distance <= min {
            return 1;
        }
        if plane.distance >= max {
            return 2;
        }
        return 3;
    }
    box_on_plane_side(*bounds, *plane)
}

/// Chop a polygon behind a plane (`R_ChopPolyBehindPlane`).
pub(crate) fn chop(points: &[Vec3], plane: &Plane) -> Vec<Vec3> {
    if points.len() >= MAX_CLIP_VERTICES - 2 {
        return Vec::new();
    }
    let distances: Vec<f32> = points
        .iter()
        .map(|point| dot3(*point, plane.normal) - plane.distance)
        .collect();
    let sides: Vec<u8> = distances
        .iter()
        .map(|distance| {
            if *distance > CLIP_EPSILON {
                0
            } else if *distance < -CLIP_EPSILON {
                1
            } else {
                2
            }
        })
        .collect();
    if !sides.contains(&0) {
        return Vec::new();
    }
    if !sides.contains(&1) {
        return points.to_vec();
    }
    let mut result = Vec::new();
    for (index, first) in points.iter().enumerate() {
        let side = sides[index];
        let next = (index + 1) % points.len();
        if side == 2 {
            result.push(*first);
            continue;
        }
        if side == 0 {
            result.push(*first);
        }
        let next_side = sides[next];
        if next_side == 2 || next_side == side {
            continue;
        }
        let difference = distances[index] - distances[next];
        let fraction = if difference == 0.0 {
            0.0
        } else {
            distances[index] / difference
        };
        result.push(add3(*first, scale3(sub3(points[next], *first), fraction)));
    }
    result
}

/// BSP mark projector (`BspMarkProjector`).
#[derive(Debug, Clone, PartialEq)]
pub struct BspMarkProjector {
    /// Geometry.
    pub geometry: MarkGeometry,
}

impl BspMarkProjector {
    /// New projector.
    pub fn new(geometry: MarkGeometry) -> PresentResult<Self> {
        if geometry.surfaces.len() != geometry.map.surface_count {
            return Err(PresentError::range("mark geometry must retain every BSP surface index"));
        }
        Ok(Self { geometry })
    }

    /// Project mark fragments (`markFragments`).
    pub fn mark_fragments(&self, query: &MarkProjection) -> PresentResult<MarkFragments> {
        struct Owned {
            query: MarkProjection,
            points: Vec<Vec3>,
            fragments: Vec<MarkFragment>,
        }
        impl SourceMarkProjection for Owned {
            fn point_count(&self) -> usize {
                self.query.points.len()
            }
            fn max_points(&self) -> usize {
                self.query.max_points
            }
            fn max_fragments(&self) -> usize {
                self.query.max_fragments
            }
            fn read_point(&self, index: usize) -> PresentResult<Vec3> {
                mark_at(&self.query.points, index).copied()
            }
            fn read_projection(&self) -> Vec3 {
                self.query.projection
            }
            fn write_fragment(&mut self, _index: usize, fragment: MarkFragment) {
                self.fragments.push(fragment);
            }
            fn write_points(&mut self, _first_point: usize, points: &[Vec3]) {
                self.points.extend_from_slice(points);
            }
        }
        let mut owned = Owned {
            query: query.clone(),
            points: Vec::new(),
            fragments: Vec::new(),
        };
        self.mark_fragments_record(&mut owned)?;
        Ok(MarkFragments {
            points: owned.points,
            fragments: owned.fragments,
        })
    }

    /// Project into borrowed output stores (`markFragmentsRecord`).
    pub fn mark_fragments_record(&self, query: &mut dyn SourceMarkProjection) -> PresentResult<usize> {
        check_capacity(query.max_points())?;
        check_capacity(query.max_fragments())?;
        if query.point_count() < 1 {
            return Err(PresentError::range("mark projection requires at least one input point"));
        }
        let projection = finite_vector(query.read_projection())?;
        let direction = normalize3_or_zero(projection);
        let mut points = Vec::new();
        let mut minimum = vec3(99999.0, 99999.0, 99999.0);
        let mut maximum = vec3(-99999.0, -99999.0, -99999.0);
        for index in 0..query.point_count() {
            let point = finite_vector(query.read_point(index)?)?;
            if index < MAX_CLIP_VERTICES {
                points.push(point);
            }
            for bound_point in [point, add3(point, projection), add3(point, scale3(direction, -20.0))] {
                minimum.x = minimum.x.min(bound_point.x);
                minimum.y = minimum.y.min(bound_point.y);
                minimum.z = minimum.z.min(bound_point.z);
                maximum.x = maximum.x.max(bound_point.x);
                maximum.y = maximum.y.max(bound_point.y);
                maximum.z = maximum.z.max(bound_point.z);
            }
        }
        if query.max_fragments() == 0 || query.max_points() == 0 {
            return Ok(0);
        }
        let count = points.len();
        let mut planes = Vec::new();
        for index in 0..count {
            let point = *mark_at(&points, index)?;
            let edge = sub3(*mark_at(&points, (index + 1) % count)?, point);
            let reverse = sub3(point, add3(point, projection));
            let normal = normalize_fast3(cross3(edge, reverse));
            planes.push(Plane {
                normal,
                distance: dot3(normal, point),
            });
        }
        let first = *mark_at(&points, 0)?;
        let inverse = scale3(direction, -1.0);
        planes.push(Plane {
            normal: direction,
            distance: dot3(direction, first) - 32.0,
        });
        planes.push(Plane {
            normal: inverse,
            distance: dot3(inverse, first) - 20.0,
        });
        let surfaces = self.box_surfaces(
            &Bounds {
                min: minimum,
                max: maximum,
            },
            direction,
        )?;
        let mut returned_points = 0usize;
        let mut returned_fragments = 0usize;
        let max_points = query.max_points();
        let max_fragments = query.max_fragments();
        let append = |triangle: [Vec3; 3],
                      query: &mut dyn SourceMarkProjection,
                      returned_points: &mut usize,
                      returned_fragments: &mut usize| {
            let mut clipped = triangle.to_vec();
            for plane in &planes {
                clipped = chop(&clipped, plane);
                if clipped.is_empty() {
                    return;
                }
            }
            if clipped.len() + *returned_points > max_points {
                return;
            }
            query.write_fragment(
                *returned_fragments,
                MarkFragment {
                    first_point: *returned_points,
                    point_count: clipped.len(),
                },
            );
            query.write_points(*returned_points, &clipped);
            *returned_points += clipped.len();
            *returned_fragments += 1;
        };
        for surface in &surfaces {
            match surface {
                MarkSurface::Skip => {}
                MarkSurface::Face {
                    plane,
                    vertices,
                    indices,
                    ..
                } => {
                    if dot3(plane.normal, direction) > -0.5 {
                        continue;
                    }
                    let mut index = 0;
                    while index < indices.len() {
                        let triangle = [
                            add3(
                                mark_at(vertices, *mark_at(indices, index)?)?.position,
                                scale3(plane.normal, 0.0),
                            ),
                            add3(
                                mark_at(vertices, *mark_at(indices, index + 1)?)?.position,
                                scale3(plane.normal, 0.0),
                            ),
                            add3(
                                mark_at(vertices, *mark_at(indices, index + 2)?)?.position,
                                scale3(plane.normal, 0.0),
                            ),
                        ];
                        append(triangle, &mut *query, &mut returned_points, &mut returned_fragments);
                        if returned_fragments == max_fragments {
                            return Ok(returned_fragments);
                        }
                        index += 3;
                    }
                }
                MarkSurface::Grid { mesh, .. } => {
                    for row in 0..mesh.height.saturating_sub(1) {
                        for column in 0..mesh.width.saturating_sub(1) {
                            let base = row * mesh.width + column;
                            for (indexes, threshold) in [
                                ([base, base + mesh.width, base + 1], -0.1f32),
                                ([base + 1, base + mesh.width, base + mesh.width + 1], -0.05f32),
                            ] {
                                let triangle: Vec<Vec3> = indexes
                                    .iter()
                                    .map(|index| {
                                        mark_at(&mesh.vertices, *index)
                                            .map(|vertex| add3(vertex.position, scale3(vertex.normal, 0.0)))
                                    })
                                    .collect::<PresentResult<_>>()?;
                                let normal = normalize_fast3(cross3(
                                    sub3(triangle[0], triangle[1]),
                                    sub3(triangle[2], triangle[1]),
                                ));
                                if dot3(normal, direction) >= threshold {
                                    continue;
                                }
                                append(
                                    [triangle[0], triangle[1], triangle[2]],
                                    &mut *query,
                                    &mut returned_points,
                                    &mut returned_fragments,
                                );
                                if returned_fragments == max_fragments {
                                    return Ok(returned_fragments);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(returned_fragments)
    }

    /// Surfaces overlapping a bounds box (`boxSurfaces`).
    fn box_surfaces(&self, bounds: &Bounds, direction: Vec3) -> PresentResult<Vec<MarkSurface>> {
        let map = &self.geometry.map;
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut stack: Vec<i64> = if map.nodes.is_empty() {
            if map.leaves.is_empty() {
                Vec::new()
            } else {
                vec![-1]
            }
        } else {
            vec![0]
        };
        while let Some(index) = stack.pop() {
            if index >= 0 {
                let node = mark_at(&map.nodes, index as usize)?;
                let side = box_side(bounds, mark_at(&map.planes, node.plane)?);
                if side & 2 != 0 {
                    let child = node.children[1];
                    stack.push(if child.is_node {
                        child.index as i64
                    } else {
                        -1 - child.index as i64
                    });
                }
                if side & 1 != 0 {
                    let child = node.children[0];
                    stack.push(if child.is_node {
                        child.index as i64
                    } else {
                        -1 - child.index as i64
                    });
                }
                continue;
            }
            let leaf = mark_at(&map.leaves, (-index - 1) as usize)?;
            for offset in 0..leaf.surface_count {
                if result.len() >= MAX_MARK_SURFACES {
                    break;
                }
                let surface_index = *mark_at(&map.leaf_surfaces, leaf.first_surface + offset)?;
                if !visited.insert(surface_index) {
                    continue;
                }
                let surface = mark_at(&self.geometry.surfaces, surface_index)?;
                match surface {
                    MarkSurface::Skip => continue,
                    MarkSurface::Face {
                        surface_flags,
                        content_flags,
                        ..
                    }
                    | MarkSurface::Grid {
                        surface_flags,
                        content_flags,
                        ..
                    } => {
                        if surface_flags & 0x30 != 0 || content_flags & 64 != 0 {
                            continue;
                        }
                    }
                }
                if let MarkSurface::Face { plane, .. } = surface {
                    if box_side(bounds, plane) != 3 || dot3(plane.normal, direction) > -0.5 {
                        continue;
                    }
                }
                result.push(surface.clone());
            }
        }
        Ok(result)
    }
}

/// World surface for mark projection (`worldMarkProjector` input).
#[derive(Debug, Clone, PartialEq)]
pub enum PresentMarkWorldSurface {
    /// Quake III material surface.
    Q3 {
        /// Surface flags.
        surface_flags: i32,
        /// Content flags.
        content_flags: i32,
        /// Prepared grid, when present.
        grid: Option<PresentPatchMesh>,
        /// Whether the source surface is planar.
        planar: bool,
        /// Plane, when present.
        plane: Option<Plane>,
        /// Vertices.
        vertices: Vec<MarkVertex>,
        /// Indices.
        indices: Vec<usize>,
    },
    /// Shared foreign surface.
    Shared {
        /// Plane, when present.
        plane: Option<Plane>,
        /// Vertices.
        vertices: Vec<MarkVertex>,
        /// Indices.
        indices: Vec<usize>,
        /// Excluded by material rules.
        excluded: bool,
    },
}

/// World mark geometry input.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentMarkWorld {
    /// Nodes.
    pub nodes: Vec<MarkNode>,
    /// Planes.
    pub planes: Vec<Plane>,
    /// Leaves.
    pub leaves: Vec<MarkLeaf>,
    /// Leaf surfaces.
    pub leaf_surfaces: Vec<usize>,
    /// Surfaces.
    pub surfaces: Vec<PresentMarkWorldSurface>,
}

/// Build a projector from prepared world geometry (`worldMarkProjector`).
pub fn world_mark_projector(world: &PresentMarkWorld) -> PresentResult<BspMarkProjector> {
    let mut surfaces = Vec::with_capacity(world.surfaces.len());
    for surface in &world.surfaces {
        match surface {
            PresentMarkWorldSurface::Q3 {
                surface_flags,
                content_flags,
                grid,
                planar,
                plane,
                vertices,
                indices,
            } => {
                if let Some(mesh) = grid {
                    surfaces.push(MarkSurface::Grid {
                        surface_flags: *surface_flags,
                        content_flags: *content_flags,
                        mesh: mesh.clone(),
                    });
                } else if !planar || plane.is_none() {
                    surfaces.push(MarkSurface::Skip);
                } else {
                    surfaces.push(MarkSurface::Face {
                        surface_flags: *surface_flags,
                        content_flags: *content_flags,
                        plane: plane.unwrap_or(Plane {
                            normal: zero_vec3(),
                            distance: 0.0,
                        }),
                        vertices: vertices.clone(),
                        indices: indices.clone(),
                    });
                }
            }
            PresentMarkWorldSurface::Shared {
                plane,
                vertices,
                indices,
                excluded,
            } => {
                if plane.is_none() || *excluded {
                    surfaces.push(MarkSurface::Skip);
                } else {
                    surfaces.push(MarkSurface::Face {
                        surface_flags: 0,
                        content_flags: 0,
                        plane: plane.unwrap_or(Plane {
                            normal: zero_vec3(),
                            distance: 0.0,
                        }),
                        vertices: vertices.clone(),
                        indices: indices.clone(),
                    });
                }
            }
        }
    }
    let surface_count = surfaces.len();
    BspMarkProjector::new(MarkGeometry {
        map: MarkMap {
            nodes: world.nodes.clone(),
            planes: world.planes.clone(),
            leaves: world.leaves.clone(),
            leaf_surfaces: world.leaf_surfaces.clone(),
            surface_count,
        },
        surfaces,
    })
}
