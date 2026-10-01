//! Quake II scene-collision adapter: brush tracing and area connectivity.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q2.ts`
//! (id Software `cmodel.c`, q2repro `cmodel.c`).
//!
//! The adapter consumes a collision-subset geometry mirroring the donor's
//! `Q2WorldGeometry` BSP records; navigation geometries omit them. The scene
//! subset drops the secondary impact plane, so runner-up tracking is omitted.

use std::cell::RefCell;
use std::collections::HashSet;

use qa_core::math::{js_max, js_min, Bounds, Vec3};
use qa_core::numeric::NumericOps;
use qa_world::collision::{convert_contents, trace_brush_media, MediumBrush, TraceMedia};
use qa_world::spatial::CollisionFamily;
use qa_world::WorldError;

use crate::collision_support::{geometry_mask, select_numeric};
use crate::scene::{
    BspPlane, LeafQueryResult, PointContentsQuery, PointContentsResult, Q2SecondaryImpact, Q2SurfaceInfo, QueryTarget,
    SceneQueries, TraceContact, TraceDetail, TraceHit, TracePolicy, TraceQuery, TraceResult, TraceShape,
    VisibilityKind,
};

/// BSP child reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2BspChild {
    /// Leaf index.
    Leaf(usize),
    /// Node index.
    Node(usize),
}

/// Contiguous record span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2IndexRange {
    /// First record.
    pub first: usize,
    /// Record count.
    pub count: usize,
}

/// Collision model: bounds plus head node (`-1 - leaf` for bare leaves).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CollisionModel {
    /// Model bounds.
    pub bounds: Bounds,
    /// Head node.
    pub headnode: i32,
}

/// Collision BSP node.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CollisionNode {
    /// Splitting plane.
    pub plane: usize,
    /// Front/back children.
    pub children: [Q2BspChild; 2],
}

/// Collision leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2CollisionLeaf {
    /// Visibility cluster.
    pub cluster: i32,
    /// Area.
    pub area: i32,
    /// Stored contents.
    pub contents: i32,
    /// Merged contents.
    pub merged_contents: i32,
    /// Leaf brushes.
    pub brushes: Q2IndexRange,
}

/// Collision brush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2CollisionBrush {
    /// Brush contents.
    pub contents: i32,
    /// Brush sides.
    pub sides: Q2IndexRange,
}

/// Collision brush side (`-1` texture info means no surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2CollisionBrushSide {
    /// Side plane.
    pub plane: usize,
    /// Texture info, or `-1`.
    pub texture_info: i32,
}

/// Collision area portal span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2CollisionArea {
    /// Area portals.
    pub portals: Q2IndexRange,
}

/// Area portal link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2CollisionAreaPortal {
    /// Portal identifier.
    pub portal: i32,
    /// Area on the far side.
    pub other_area: usize,
}

/// Visibility row offsets for one cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2VisibilityCluster {
    /// PVS row offset, or `-1` for all-visible.
    pub pvs_offset: i32,
    /// PHS row offset, or `-1` for all-audible.
    pub phs_offset: i32,
}

/// Compressed cluster visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2CollisionVisibility {
    /// Per-cluster row offsets.
    pub clusters: Vec<Q2VisibilityCluster>,
    /// Compressed rows.
    pub compressed: Vec<u8>,
}

/// Collision-subset Quake II geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CollisionGeometry {
    /// Models.
    pub models: Vec<Q2CollisionModel>,
    /// BSP nodes.
    pub nodes: Vec<Q2CollisionNode>,
    /// Planes.
    pub planes: Vec<BspPlane>,
    /// Leaves.
    pub leaves: Vec<Q2CollisionLeaf>,
    /// Brushes.
    pub brushes: Vec<Q2CollisionBrush>,
    /// Brush sides.
    pub brush_sides: Vec<Q2CollisionBrushSide>,
    /// Texture info records.
    pub texture_info: Vec<Q2SurfaceInfo>,
    /// Leaf brush indexes.
    pub leaf_brushes: Vec<usize>,
    /// Areas.
    pub areas: Vec<Q2CollisionArea>,
    /// Area portals.
    pub area_portals: Vec<Q2CollisionAreaPortal>,
    /// Cluster visibility, when shipped.
    pub visibility: Option<Q2CollisionVisibility>,
}

/// Brush clip epsilon.
const EPSILON: f64 = 0.03125;

/// Indexed access with the donor's range error.
fn at<T>(values: &[T], index: i32) -> Result<&T, WorldError> {
    if index < 0 {
        return Err(WorldError::BadCollisionRecord(format!("Q2 collision index {index}")));
    }
    values
        .get(index as usize)
        .ok_or_else(|| WorldError::BadCollisionRecord(format!("Q2 collision index {index}")))
}

fn dot(a: Vec3, b: Vec3, n: &NumericOps) -> f64 {
    n.add(
        n.add(
            n.mul(f64::from(a.x), f64::from(b.x)),
            n.mul(f64::from(a.y), f64::from(b.y)),
        ),
        n.mul(f64::from(a.z), f64::from(b.z)),
    )
}

fn subtract(a: Vec3, b: Vec3, n: &NumericOps) -> Vec3 {
    Vec3 {
        x: n.sub(f64::from(a.x), f64::from(b.x)) as f32,
        y: n.sub(f64::from(a.y), f64::from(b.y)) as f32,
        z: n.sub(f64::from(a.z), f64::from(b.z)) as f32,
    }
}

fn lerp(a: Vec3, b: Vec3, f: f64, n: &NumericOps) -> Vec3 {
    Vec3 {
        x: n.add(f64::from(a.x), n.mul(f, n.sub(f64::from(b.x), f64::from(a.x)))) as f32,
        y: n.add(f64::from(a.y), n.mul(f, n.sub(f64::from(b.y), f64::from(a.y)))) as f32,
        z: n.add(f64::from(a.z), n.mul(f, n.sub(f64::from(b.z), f64::from(a.z)))) as f32,
    }
}

fn axis(angles: Vec3, n: &NumericOps) -> [Vec3; 3] {
    let yaw = f64::from(angles.y) * std::f64::consts::PI / 180.0;
    let pitch = f64::from(angles.x) * std::f64::consts::PI / 180.0;
    let roll = f64::from(angles.z) * std::f64::consts::PI / 180.0;
    let sy = f64::from(n.store(yaw.sin()));
    let cy = f64::from(n.store(yaw.cos()));
    let sp = f64::from(n.store(pitch.sin()));
    let cp = f64::from(n.store(pitch.cos()));
    let sr = f64::from(n.store(roll.sin()));
    let cr = f64::from(n.store(roll.cos()));
    [
        Vec3 {
            x: n.mul(cp, cy) as f32,
            y: n.mul(cp, sy) as f32,
            z: (-sp) as f32,
        },
        Vec3 {
            x: n.sub(n.mul(n.mul(sr, sp), cy), n.mul(cr, sy)) as f32,
            y: n.add(n.mul(n.mul(sr, sp), sy), n.mul(cr, cy)) as f32,
            z: n.mul(sr, cp) as f32,
        },
        Vec3 {
            x: n.add(n.mul(n.mul(cr, sp), cy), n.mul(sr, sy)) as f32,
            y: n.sub(n.mul(n.mul(cr, sp), sy), n.mul(sr, cy)) as f32,
            z: n.mul(cr, cp) as f32,
        },
    ]
}

fn rotate(point: Vec3, basis: &[Vec3; 3], n: &NumericOps) -> Vec3 {
    Vec3 {
        x: dot(point, basis[0], n) as f32,
        y: dot(point, basis[1], n) as f32,
        z: dot(point, basis[2], n) as f32,
    }
}

fn plane_distance(point: Vec3, plane: &BspPlane, n: &NumericOps) -> f64 {
    let axial = if plane.plane_type == 0 {
        f64::from(point.x)
    } else if plane.plane_type == 1 {
        f64::from(point.y)
    } else if plane.plane_type == 2 {
        f64::from(point.z)
    } else {
        dot(point, plane.normal, n)
    };
    n.sub(axial, f64::from(plane.distance))
}

fn shape_bounds(shape: &TraceShape) -> Bounds {
    match shape {
        TraceShape::Point => Bounds {
            min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        },
        TraceShape::Box { bounds } | TraceShape::Capsule { bounds } => *bounds,
    }
}

fn expand(plane: &BspPlane, shape: &TraceShape, n: &NumericOps) -> f64 {
    let bounds = shape_bounds(shape);
    let normal = plane.normal;
    if matches!(shape, TraceShape::Capsule { .. }) {
        let center = Vec3 {
            x: n.mul(n.add(f64::from(bounds.min.x), f64::from(bounds.max.x)), 0.5) as f32,
            y: n.mul(n.add(f64::from(bounds.min.y), f64::from(bounds.max.y)), 0.5) as f32,
            z: n.mul(n.add(f64::from(bounds.min.z), f64::from(bounds.max.z)), 0.5) as f32,
        };
        let halfheight = n.mul(n.sub(f64::from(bounds.max.z), f64::from(bounds.min.z)), 0.5);
        let radius = js_min(
            n.mul(n.sub(f64::from(bounds.max.x), f64::from(bounds.min.x)), 0.5),
            halfheight,
        );
        return n.sub(
            n.add(radius, n.mul(f64::from(normal.z.abs()), n.sub(halfheight, radius))),
            dot(center, normal, n),
        );
    }
    -dot(
        Vec3 {
            x: if normal.x < 0.0 { bounds.max.x } else { bounds.min.x },
            y: if normal.y < 0.0 { bounds.max.y } else { bounds.min.y },
            z: if normal.z < 0.0 { bounds.max.z } else { bounds.min.z },
        },
        normal,
        n,
    )
}

struct Work {
    fraction: f64,
    start_solid: bool,
    all_solid: bool,
    contents: i32,
    plane: BspPlane,
    surface: Option<Q2SurfaceInfo>,
    secondary: Option<Q2SecondaryImpact>,
}

fn no_plane() -> BspPlane {
    BspPlane {
        normal: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        distance: 0.0,
        plane_type: 0,
        signbits: 0,
    }
}

/// Scene adapter over Quake II collision geometry.
pub struct Q2Collision {
    geometry: Q2CollisionGeometry,
    open_portals: RefCell<HashSet<i32>>,
    flood: RefCell<Vec<i32>>,
}

impl Q2Collision {
    /// Wrap geometry and flood the area graph.
    #[must_use]
    pub fn new(geometry: Q2CollisionGeometry) -> Self {
        let flood = vec![0; geometry.areas.len()];
        let collision = Self {
            geometry,
            open_portals: RefCell::new(HashSet::new()),
            flood: RefCell::new(flood),
        };
        collision.flood_areas();
        collision
    }

    /// Borrow the geometry.
    #[must_use]
    pub fn geometry(&self) -> &Q2CollisionGeometry {
        &self.geometry
    }

    /// Model bounds.
    pub fn model_bounds(&self, index: i32) -> Result<Bounds, WorldError> {
        Ok(at(&self.geometry.models, index)?.bounds)
    }

    /// Leaf containing a point inside a model.
    pub fn point_leaf(&self, point: Vec3, model: i32) -> Result<i32, WorldError> {
        let headnode = at(&self.geometry.models, model)?.headnode;
        self.walk_leaf(point, headnode, None)
    }

    fn walk_leaf(&self, point: Vec3, headnode: i32, numeric: Option<&NumericOps>) -> Result<i32, WorldError> {
        let mut child = if headnode < 0 {
            Q2BspChild::Leaf((-1 - headnode) as usize)
        } else {
            Q2BspChild::Node(headnode as usize)
        };
        while let Q2BspChild::Node(index) = child {
            let node = at(&self.geometry.nodes, index as i32)?;
            let plane = at(&self.geometry.planes, node.plane as i32)?;
            let distance = match numeric {
                None => {
                    f64::from(point.x) * f64::from(plane.normal.x)
                        + f64::from(point.y) * f64::from(plane.normal.y)
                        + f64::from(point.z) * f64::from(plane.normal.z)
                        - f64::from(plane.distance)
                }
                Some(n) => plane_distance(point, plane, n),
            };
            child = node.children[usize::from(distance < 0.0)];
        }
        match child {
            Q2BspChild::Leaf(index) => Ok(index as i32),
            Q2BspChild::Node(_) => unreachable!("walked past leaves"),
        }
    }

    /// Leaf cluster.
    pub fn leaf_cluster(&self, index: i32) -> Result<i32, WorldError> {
        Ok(at(&self.geometry.leaves, index)?.cluster)
    }

    /// Leaf area.
    pub fn leaf_area(&self, index: i32) -> Result<i32, WorldError> {
        Ok(at(&self.geometry.leaves, index)?.area)
    }

    /// Sample contents at a point.
    pub fn point_contents(&self, query: &PointContentsQuery) -> Result<PointContentsResult, WorldError> {
        let n = select_numeric(&query.numeric)?;
        let mut point = query.point;
        let mut model = 0;
        if let QueryTarget::Model {
            model: target,
            origin,
            angles,
        } = &query.target
        {
            model = *target;
            point = rotate(subtract(point, *origin, &n), &axis(*angles, &n), &n);
        }
        let headnode = at(&self.geometry.models, model)?.headnode;
        let leaf = at(&self.geometry.leaves, self.walk_leaf(point, headnode, Some(&n))?)?;
        Ok(PointContentsResult::Q2 {
            stored: leaf.contents,
            merged: leaf.merged_contents,
        })
    }

    /// Classify the media along a sweep.
    pub fn trace_media(&self, query: &TraceQuery, fraction: f64) -> Result<TraceMedia, WorldError> {
        let numeric = select_numeric(&query.numeric)?;
        let model = match &query.target {
            QueryTarget::World => 0,
            QueryTarget::Model { model, .. } => *model,
        };
        let rotation = match &query.target {
            QueryTarget::World => None,
            QueryTarget::Model { angles, .. } => Some(axis(*angles, &numeric)),
        };
        let local = |point: Vec3| -> Vec3 {
            match (&query.target, &rotation) {
                (QueryTarget::Model { origin, .. }, Some(basis)) => {
                    rotate(subtract(point, *origin, &numeric), basis, &numeric)
                }
                _ => point,
            }
        };
        let start = local(query.start);
        let end = local(query.end);
        let bounds = shape_bounds(&query.shape);
        let reached = lerp(start, end, fraction, &numeric);
        let envelope = Bounds {
            min: Vec3 {
                x: (js_min(f64::from(start.x), f64::from(reached.x)) + f64::from(bounds.min.x) - 1.0) as f32,
                y: (js_min(f64::from(start.y), f64::from(reached.y)) + f64::from(bounds.min.y) - 1.0) as f32,
                z: (js_min(f64::from(start.z), f64::from(reached.z)) + f64::from(bounds.min.z) - 1.0) as f32,
            },
            max: Vec3 {
                x: (js_max(f64::from(start.x), f64::from(reached.x)) + f64::from(bounds.max.x) + 1.0) as f32,
                y: (js_max(f64::from(start.y), f64::from(reached.y)) + f64::from(bounds.max.y) + 1.0) as f32,
                z: (js_max(f64::from(start.z), f64::from(reached.z)) + f64::from(bounds.max.z) + 1.0) as f32,
            },
        };
        let headnode = at(&self.geometry.models, model)?.headnode;
        let leaves = self.box_leaves(&envelope, self.geometry.leaves.len(), headnode)?;
        let mut seen = HashSet::new();
        let mut brushes = Vec::new();
        for leaf_index in &leaves.leaves {
            let leaf = at(&self.geometry.leaves, *leaf_index)?;
            for offset in 0..leaf.brushes.count {
                let brush_index = *at(&self.geometry.leaf_brushes, (leaf.brushes.first + offset) as i32)?;
                if !seen.insert(brush_index) {
                    continue;
                }
                let brush = at(&self.geometry.brushes, brush_index as i32)?;
                if convert_contents(brush.contents, CollisionFamily::Q2, CollisionFamily::Q1) == -1 {
                    continue;
                }
                let mut distances = Vec::with_capacity(brush.sides.count);
                for side_offset in 0..brush.sides.count {
                    let side = at(&self.geometry.brush_sides, (brush.sides.first + side_offset) as i32)?;
                    let plane = at(&self.geometry.planes, side.plane as i32)?;
                    let grown = numeric.add(f64::from(plane.distance), expand(plane, &query.shape, &numeric));
                    distances.push((
                        numeric.sub(dot(start, plane.normal, &numeric), grown),
                        numeric.sub(dot(end, plane.normal, &numeric), grown),
                    ));
                }
                brushes.push(MediumBrush {
                    contents: brush.contents,
                    distances,
                });
            }
        }
        Ok(trace_brush_media(&brushes, CollisionFamily::Q2, fraction))
    }

    /// Leaves touched by bounds under a head node. The limit is a `usize`,
    /// so the donor's negative-limit range error is unrepresentable.
    pub fn box_leaves(&self, bounds: &Bounds, limit: usize, headnode: i32) -> Result<LeafQueryResult, WorldError> {
        let mut leaves = Vec::new();
        let mut topnode = None;
        let mut overflow = false;
        let mut stack = vec![if headnode < 0 {
            Q2BspChild::Leaf((-1 - headnode) as usize)
        } else {
            Q2BspChild::Node(headnode as usize)
        }];
        while let Some(child) = stack.pop() {
            match child {
                Q2BspChild::Leaf(index) => {
                    if leaves.len() == limit {
                        overflow = true;
                    } else {
                        leaves.push(index as i32);
                    }
                }
                Q2BspChild::Node(index) => {
                    let node = at(&self.geometry.nodes, index as i32)?;
                    let plane = at(&self.geometry.planes, node.plane as i32)?;
                    let normal = plane.normal;
                    let far = (if normal.x < 0.0 {
                        f64::from(bounds.min.x)
                    } else {
                        f64::from(bounds.max.x)
                    }) * f64::from(normal.x)
                        + (if normal.y < 0.0 {
                            f64::from(bounds.min.y)
                        } else {
                            f64::from(bounds.max.y)
                        }) * f64::from(normal.y)
                        + (if normal.z < 0.0 {
                            f64::from(bounds.min.z)
                        } else {
                            f64::from(bounds.max.z)
                        }) * f64::from(normal.z);
                    let near = (if normal.x < 0.0 {
                        f64::from(bounds.max.x)
                    } else {
                        f64::from(bounds.min.x)
                    }) * f64::from(normal.x)
                        + (if normal.y < 0.0 {
                            f64::from(bounds.max.y)
                        } else {
                            f64::from(bounds.min.y)
                        }) * f64::from(normal.y)
                        + (if normal.z < 0.0 {
                            f64::from(bounds.max.z)
                        } else {
                            f64::from(bounds.min.z)
                        }) * f64::from(normal.z);
                    if far >= f64::from(plane.distance) && near < f64::from(plane.distance) && topnode.is_none() {
                        topnode = Some(index as i32);
                    }
                    if near < f64::from(plane.distance) {
                        stack.push(node.children[1]);
                    }
                    if far >= f64::from(plane.distance) {
                        stack.push(node.children[0]);
                    }
                }
            }
        }
        Ok(LeafQueryResult {
            leaves,
            topnode,
            overflow,
        })
    }

    /// Sweep a body through the world.
    #[allow(clippy::too_many_lines)]
    pub fn trace(&self, query: &TraceQuery) -> Result<TraceResult, WorldError> {
        let n = select_numeric(&query.numeric)?;
        let model = match &query.target {
            QueryTarget::World => 0,
            QueryTarget::Model { model, .. } => *model,
        };
        let headnode = at(&self.geometry.models, model)?.headnode;
        let transform = match &query.target {
            QueryTarget::World => None,
            QueryTarget::Model { angles, .. } => Some(axis(*angles, &n)),
        };
        let (start, end) = match (&query.target, &transform) {
            (QueryTarget::Model { origin, .. }, Some(basis)) => (
                rotate(subtract(query.start, *origin, &n), basis, &n),
                rotate(subtract(query.end, *origin, &n), basis, &n),
            ),
            _ => (query.start, query.end),
        };
        let stationary = start.x == end.x && start.y == end.y && start.z == end.z;
        let merged = !matches!(
            query.policy,
            TracePolicy::Q2 {
                leaf_contents: crate::scene::LeafContents::Stored,
                ..
            }
        );
        let mask = geometry_mask(&query.policy, CollisionFamily::Q2);
        let bounds = shape_bounds(&query.shape);
        let extents = Vec3 {
            x: js_max(-f64::from(bounds.min.x), f64::from(bounds.max.x)) as f32,
            y: js_max(-f64::from(bounds.min.y), f64::from(bounds.max.y)) as f32,
            z: js_max(-f64::from(bounds.min.z), f64::from(bounds.max.z)) as f32,
        };
        let mut work = Work {
            fraction: 1.0,
            start_solid: false,
            all_solid: false,
            contents: 0,
            plane: no_plane(),
            surface: None,
            secondary: None,
        };
        let mut checked = HashSet::new();
        if stationary {
            let envelope = Bounds {
                min: Vec3 {
                    x: (f64::from(start.x) + f64::from(bounds.min.x) - 1.0) as f32,
                    y: (f64::from(start.y) + f64::from(bounds.min.y) - 1.0) as f32,
                    z: (f64::from(start.z) + f64::from(bounds.min.z) - 1.0) as f32,
                },
                max: Vec3 {
                    x: (f64::from(start.x) + f64::from(bounds.max.x) + 1.0) as f32,
                    y: (f64::from(start.y) + f64::from(bounds.max.y) + 1.0) as f32,
                    z: (f64::from(start.z) + f64::from(bounds.max.z) + 1.0) as f32,
                },
            };
            let leaves = self.box_leaves(&envelope, 1024, headnode)?;
            for leaf in &leaves.leaves {
                self.leaf_trace(
                    *leaf,
                    &mut work,
                    &mut checked,
                    &n,
                    mask,
                    merged,
                    &start,
                    &end,
                    stationary,
                    query,
                )?;
                if work.all_solid {
                    break;
                }
            }
        } else {
            let root = if headnode < 0 {
                Q2BspChild::Leaf((-1 - headnode) as usize)
            } else {
                Q2BspChild::Node(headnode as usize)
            };
            self.walk(
                root,
                0.0,
                1.0,
                start,
                end,
                &start,
                &end,
                &mut work,
                &mut checked,
                &n,
                mask,
                merged,
                &extents,
                query,
            )?;
        }
        let mut plane = work.plane;
        if transform.is_some() && work.fraction != 1.0 {
            if let QueryTarget::Model { angles, .. } = &query.target {
                let inverse = axis(
                    Vec3 {
                        x: -angles.x,
                        y: -angles.y,
                        z: -angles.z,
                    },
                    &n,
                );
                plane.normal = rotate(plane.normal, &inverse, &n);
            }
        }
        Ok(TraceResult {
            fraction: work.fraction,
            end: lerp(query.start, query.end, work.fraction, &n),
            start_solid: work.start_solid,
            all_solid: work.all_solid,
            contact: if work.fraction < 1.0 && !work.all_solid {
                TraceContact::Plane {
                    plane: qa_core::math::Plane {
                        normal: plane.normal,
                        distance: plane.distance,
                    },
                }
            } else {
                TraceContact::None
            },
            hit: if work.fraction < 1.0 || work.start_solid {
                TraceHit::World { model }
            } else {
                TraceHit::None
            },
            detail: TraceDetail::Q2 {
                contents: work.contents,
                surface: work.surface,
                source_plane: plane,
                secondary: work.secondary,
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn leaf_trace(
        &self,
        index: i32,
        work: &mut Work,
        checked: &mut HashSet<usize>,
        n: &NumericOps,
        mask: i32,
        merged: bool,
        start: &Vec3,
        end: &Vec3,
        stationary: bool,
        query: &TraceQuery,
    ) -> Result<(), WorldError> {
        let leaf = at(&self.geometry.leaves, index)?;
        let contents = if merged { leaf.merged_contents } else { leaf.contents };
        if contents & mask == 0 {
            return Ok(());
        }
        for offset in 0..leaf.brushes.count {
            let brush = *at(&self.geometry.leaf_brushes, (leaf.brushes.first + offset) as i32)?;
            self.brush_trace(brush, work, checked, n, mask, merged, start, end, stationary, query)?;
            if work.fraction == 0.0 {
                return Ok(());
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn brush_trace(
        &self,
        index: usize,
        work: &mut Work,
        checked: &mut HashSet<usize>,
        n: &NumericOps,
        mask: i32,
        merged: bool,
        start: &Vec3,
        end: &Vec3,
        stationary: bool,
        query: &TraceQuery,
    ) -> Result<(), WorldError> {
        if !checked.insert(index) {
            return Ok(());
        }
        let brush = *at(&self.geometry.brushes, index as i32)?;
        if brush.contents & mask == 0 || brush.sides.count == 0 {
            return Ok(());
        }
        let mut enter = -1.0;
        let mut enter2 = -1.0;
        let mut leave = 1.0;
        let mut start_out = false;
        let mut get_out = false;
        let mut lead: Option<(BspPlane, Option<Q2SurfaceInfo>)> = None;
        let mut second: Option<BspPlane> = None;
        for offset in 0..brush.sides.count {
            let side = at(&self.geometry.brush_sides, (brush.sides.first + offset) as i32)?;
            let plane = at(&self.geometry.planes, side.plane as i32)?;
            let distance = n.add(f64::from(plane.distance), expand(plane, &query.shape, n));
            let d1 = n.sub(dot(*start, plane.normal, n), distance);
            if stationary {
                if d1 > 0.0 {
                    return Ok(());
                }
                continue;
            }
            let d2 = n.sub(dot(*end, plane.normal, n), distance);
            if d1 > 0.0 {
                start_out = true;
            }
            if d2 > 0.0 {
                get_out = true;
            }
            if d1 > 0.0 && (d2 >= EPSILON || d2 >= d1) {
                return Ok(());
            }
            if d1 <= 0.0 && d2 <= 0.0 {
                continue;
            }
            if d1 > d2 {
                let crossed = js_max(0.0, n.div(n.sub(d1, EPSILON), n.sub(d1, d2)));
                if crossed > enter {
                    enter = crossed;
                    let surface = if side.texture_info < 0 {
                        None
                    } else {
                        Some(at(&self.geometry.texture_info, side.texture_info)?.clone())
                    };
                    lead = Some((*plane, surface));
                } else if crossed > enter2 {
                    enter2 = crossed;
                    second = Some(*plane);
                }
            } else {
                leave = js_min(leave, js_min(1.0, n.div(n.add(d1, EPSILON), n.sub(d1, d2))));
            }
        }
        if !start_out {
            work.start_solid = true;
            if !get_out {
                work.all_solid = true;
                if stationary || merged {
                    work.fraction = 0.0;
                    work.contents = brush.contents;
                }
            }
            return Ok(());
        }
        if enter < leave && enter > -1.0 && enter < work.fraction {
            if let Some((plane, surface)) = lead {
                work.fraction = enter;
                work.plane = plane;
                work.surface = surface.clone();
                work.contents = brush.contents;
                if let Some(runner_up) = second {
                    work.secondary = Some(Q2SecondaryImpact {
                        plane: runner_up,
                        surface,
                    });
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        child: Q2BspChild,
        p1f: f64,
        p2f: f64,
        p1: Vec3,
        p2: Vec3,
        segment_start: &Vec3,
        segment_end: &Vec3,
        work: &mut Work,
        checked: &mut HashSet<usize>,
        n: &NumericOps,
        mask: i32,
        merged: bool,
        extents: &Vec3,
        query: &TraceQuery,
    ) -> Result<(), WorldError> {
        if work.fraction <= p1f {
            return Ok(());
        }
        match child {
            // Leaves clip the full segment; the walk only subdivides.
            Q2BspChild::Leaf(index) => self.leaf_trace(
                index as i32,
                work,
                checked,
                n,
                mask,
                merged,
                segment_start,
                segment_end,
                false,
                query,
            ),
            Q2BspChild::Node(index) => {
                let node = at(&self.geometry.nodes, index as i32)?.clone();
                let plane = *at(&self.geometry.planes, node.plane as i32)?;
                let t1 = plane_distance(p1, &plane, n);
                let t2 = plane_distance(p2, &plane, n);
                let offset = if plane.plane_type == 0 {
                    f64::from(extents.x)
                } else if plane.plane_type == 1 {
                    f64::from(extents.y)
                } else if plane.plane_type == 2 {
                    f64::from(extents.z)
                } else {
                    n.add(
                        n.add(
                            n.mul(f64::from(extents.x), f64::from(plane.normal.x)).abs(),
                            n.mul(f64::from(extents.y), f64::from(plane.normal.y)).abs(),
                        ),
                        n.mul(f64::from(extents.z), f64::from(plane.normal.z)).abs(),
                    )
                };
                if t1 >= offset && t2 >= offset {
                    return self.walk(
                        node.children[0],
                        p1f,
                        p2f,
                        p1,
                        p2,
                        segment_start,
                        segment_end,
                        work,
                        checked,
                        n,
                        mask,
                        merged,
                        extents,
                        query,
                    );
                }
                if t1 < -offset && t2 < -offset {
                    return self.walk(
                        node.children[1],
                        p1f,
                        p2f,
                        p1,
                        p2,
                        segment_start,
                        segment_end,
                        work,
                        checked,
                        n,
                        mask,
                        merged,
                        extents,
                        query,
                    );
                }
                let mut side = 0;
                let (mut f1, mut f2) = (1.0, 0.0);
                if t1 < t2 {
                    let inv = n.div(1.0, n.sub(t1, t2));
                    side = 1;
                    f2 = n.mul(n.add(n.add(t1, offset), EPSILON), inv);
                    f1 = n.mul(n.add(n.sub(t1, offset), EPSILON), inv);
                } else if t1 > t2 {
                    let inv = n.div(1.0, n.sub(t1, t2));
                    f2 = n.mul(n.sub(n.sub(t1, offset), EPSILON), inv);
                    f1 = n.mul(n.add(n.add(t1, offset), EPSILON), inv);
                }
                let f1 = js_max(0.0, js_min(1.0, f1));
                let f2 = js_max(0.0, js_min(1.0, f2));
                self.walk(
                    node.children[side],
                    p1f,
                    n.add(p1f, n.mul(n.sub(p2f, p1f), f1)),
                    p1,
                    lerp(p1, p2, f1, n),
                    segment_start,
                    segment_end,
                    work,
                    checked,
                    n,
                    mask,
                    merged,
                    extents,
                    query,
                )?;
                self.walk(
                    node.children[if side == 0 { 1 } else { 0 }],
                    n.add(p1f, n.mul(n.sub(p2f, p1f), f2)),
                    p2f,
                    lerp(p1, p2, f2, n),
                    p2,
                    segment_start,
                    segment_end,
                    work,
                    checked,
                    n,
                    mask,
                    merged,
                    extents,
                    query,
                )
            }
        }
    }

    /// Open or close an area portal and reflood.
    pub fn set_area_portal_state(&self, portal: i32, open: bool) -> Result<(), WorldError> {
        if !self.geometry.area_portals.iter().any(|link| link.portal == portal) {
            return Err(WorldError::BadCollisionRecord(format!(
                "Unknown Q2 area portal {portal}"
            )));
        }
        if open {
            self.open_portals.borrow_mut().insert(portal);
        } else {
            self.open_portals.borrow_mut().remove(&portal);
        }
        self.flood_areas();
        Ok(())
    }

    /// Open portal identifiers.
    pub fn portal_state(&self) -> Vec<i32> {
        self.open_portals.borrow().iter().copied().collect()
    }

    /// Restore open portals and reflood.
    pub fn restore_portal_state(&self, open: &[i32]) {
        self.open_portals.borrow_mut().clear();
        for id in open {
            self.open_portals.borrow_mut().insert(*id);
        }
        self.flood_areas();
    }

    fn flood_areas(&self) {
        let mut flood = self.flood.borrow_mut();
        flood.fill(0);
        let mut epoch = 0;
        for area in 1..self.geometry.areas.len() {
            if flood[area] != 0 {
                continue;
            }
            epoch += 1;
            let mut stack = vec![area];
            while let Some(current) = stack.pop() {
                if flood[current] != 0 {
                    continue;
                }
                flood[current] = epoch;
                let portals = self.geometry.areas[current].portals;
                for offset in 0..portals.count {
                    let Some(link) = self.geometry.area_portals.get(portals.first + offset) else {
                        continue;
                    };
                    if self.open_portals.borrow().contains(&link.portal) {
                        stack.push(link.other_area);
                    }
                }
            }
        }
    }

    /// Area connectivity through open portals.
    pub fn areas_connected(&self, first: i32, second: i32) -> Result<bool, WorldError> {
        let flood = self.flood.borrow();
        let a = flood
            .get(first as usize)
            .ok_or_else(|| WorldError::BadCollisionRecord(format!("Q2 collision index {first}")))?;
        let b = flood
            .get(second as usize)
            .ok_or_else(|| WorldError::BadCollisionRecord(format!("Q2 collision index {second}")))?;
        Ok(a == b)
    }

    /// Area visibility bits.
    pub fn area_bits(&self, area: i32) -> Result<Vec<u8>, WorldError> {
        let mut bits = vec![0u8; (self.geometry.areas.len() + 7) >> 3];
        for index in 0..self.geometry.areas.len() {
            if area == 0 || self.areas_connected(area, index as i32)? {
                bits[index >> 3] |= 1 << (index & 7);
            }
        }
        Ok(bits)
    }

    /// Cluster visibility through compressed rows.
    pub fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> Result<bool, WorldError> {
        if to < 0 {
            return Ok(false);
        }
        let Some(visibility) = &self.geometry.visibility else {
            return Ok(true);
        };
        if from < 0 || from as usize >= visibility.clusters.len() || to as usize >= visibility.clusters.len() {
            return Ok(false);
        }
        let row = &visibility.clusters[from as usize];
        let offset = if kind == VisibilityKind::Pvs {
            row.pvs_offset
        } else {
            row.phs_offset
        };
        if offset < 0 {
            return Ok(true);
        }
        let mut input = offset as usize;
        let mut output = 0usize;
        let wanted = (to >> 3) as usize;
        while output <= wanted {
            let byte = *visibility
                .compressed
                .get(input)
                .ok_or_else(|| WorldError::BadCollisionRecord("Q2 visibility read outside lump".to_string()))?;
            input += 1;
            if byte != 0 {
                if output == wanted {
                    return Ok(byte & (1 << (to & 7)) != 0);
                }
                output += 1;
            } else {
                let run = *visibility
                    .compressed
                    .get(input)
                    .ok_or_else(|| WorldError::BadCollisionRecord("Invalid Q2 visibility zero run".to_string()))?;
                input += 1;
                if run == 0 {
                    return Err(WorldError::BadCollisionRecord(
                        "Invalid Q2 visibility zero run".to_string(),
                    ));
                }
                if output + run as usize > wanted {
                    return Ok(false);
                }
                output += run as usize;
            }
        }
        Ok(false)
    }
}

impl SceneQueries for Q2Collision {
    fn trace(&self, query: &TraceQuery) -> TraceResult {
        self.trace(query).expect("q2 scene trace")
    }

    fn point_contents(&self, query: &PointContentsQuery) -> PointContentsResult {
        self.point_contents(query).expect("q2 scene contents")
    }

    fn box_leaves(&self, bounds: &Bounds, limit: usize) -> LeafQueryResult {
        let headnode = self.geometry.models.first().map(|model| model.headnode).unwrap_or(0);
        self.box_leaves(bounds, limit, headnode).expect("q2 scene leaves")
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        self.areas_connected(first, second).expect("q2 scene areas")
    }

    fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> bool {
        self.cluster_visible(from, to, kind).expect("q2 scene visibility")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;
    use qa_core::numeric::Q3_BINARY32_PROFILE;

    use crate::scene::{LeafContents, TraceShape as SceneShape};

    fn plane(x: f32, y: f32, z: f32, distance: f32, plane_type: i32, signbits: i32) -> BspPlane {
        BspPlane {
            normal: vec3(x, y, z),
            distance,
            plane_type,
            signbits,
        }
    }

    fn geometry() -> Q2CollisionGeometry {
        Q2CollisionGeometry {
            models: vec![Q2CollisionModel {
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                headnode: 0,
            }],
            nodes: vec![Q2CollisionNode {
                plane: 6,
                children: [Q2BspChild::Leaf(0), Q2BspChild::Leaf(1)],
            }],
            planes: vec![
                plane(1.0, 0.0, 0.0, 32.0, 0, 0),
                plane(-1.0, 0.0, 0.0, 0.0, 3, 1),
                plane(0.0, 1.0, 0.0, 16.0, 1, 0),
                plane(0.0, -1.0, 0.0, 16.0, 4, 2),
                plane(0.0, 0.0, 1.0, 16.0, 2, 0),
                plane(0.0, 0.0, -1.0, 16.0, 5, 4),
                plane(1.0, 0.0, 0.0, 0.0, 0, 0),
            ],
            leaves: vec![
                Q2CollisionLeaf {
                    cluster: 0,
                    area: 0,
                    contents: 1,
                    merged_contents: 1,
                    brushes: Q2IndexRange { first: 0, count: 1 },
                },
                Q2CollisionLeaf {
                    cluster: 1,
                    area: 0,
                    contents: 0,
                    merged_contents: 0,
                    brushes: Q2IndexRange { first: 0, count: 0 },
                },
            ],
            brushes: vec![Q2CollisionBrush {
                contents: 1,
                sides: Q2IndexRange { first: 0, count: 6 },
            }],
            brush_sides: (0..6)
                .map(|plane| Q2CollisionBrushSide { plane, texture_info: 0 })
                .collect(),
            texture_info: vec![Q2SurfaceInfo {
                name: "rock".to_string(),
                flags: 3,
            }],
            leaf_brushes: vec![0],
            areas: vec![
                Q2CollisionArea {
                    portals: Q2IndexRange { first: 0, count: 0 },
                },
                Q2CollisionArea {
                    portals: Q2IndexRange { first: 0, count: 1 },
                },
            ],
            area_portals: vec![Q2CollisionAreaPortal {
                portal: 7,
                other_area: 0,
            }],
            visibility: Some(Q2CollisionVisibility {
                clusters: vec![
                    Q2VisibilityCluster {
                        pvs_offset: 0,
                        phs_offset: 1,
                    },
                    Q2VisibilityCluster {
                        pvs_offset: 2,
                        phs_offset: 2,
                    },
                ],
                compressed: vec![0x02, 0x03, 0x03],
            }),
        }
    }

    fn policy() -> TracePolicy {
        TracePolicy::Q2 {
            contents_mask: 1,
            leaf_contents: LeafContents::Stored,
        }
    }

    fn query(start: Vec3, end: Vec3, shape: TraceShape) -> TraceQuery {
        TraceQuery {
            start,
            end,
            shape,
            target: QueryTarget::World,
            policy: policy(),
            numeric: Q3_BINARY32_PROFILE,
            pass_actor: None,
        }
    }

    #[test]
    fn q2_traces_match_donor() {
        let collision = Q2Collision::new(geometry());
        let hit = collision
            .trace(&query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), SceneShape::Point))
            .expect("trace");
        assert_eq!(hit.fraction, 0.4996874928474426);
        assert_eq!(hit.end.x, -0.03125);
        assert_eq!(hit.hit, TraceHit::World { model: 0 });
        match &hit.detail {
            TraceDetail::Q2 {
                contents,
                surface,
                source_plane,
                secondary,
            } => {
                assert_eq!(*contents, 1);
                assert_eq!(surface.as_ref().expect("surface").flags, 3);
                assert_eq!(source_plane.normal, vec3(-1.0, 0.0, 0.0));
                assert_eq!(source_plane.plane_type, 3);
                assert_eq!(source_plane.signbits, 1);
                assert_eq!(secondary, &None);
            }
            _ => panic!("q2 detail"),
        }
        let small = SceneShape::Box {
            bounds: Bounds {
                min: vec3(-4.0, -4.0, -4.0),
                max: vec3(4.0, 4.0, 4.0),
            },
        };
        let hit = collision
            .trace(&query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), small))
            .expect("trace");
        assert_eq!(hit.fraction, 0.4596875011920929);
        assert_eq!(hit.end.x, -4.03125);
        let capsule = SceneShape::Capsule {
            bounds: Bounds {
                min: vec3(-4.0, -4.0, -8.0),
                max: vec3(4.0, 4.0, 8.0),
            },
        };
        let hit = collision
            .trace(&query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), capsule))
            .expect("trace");
        assert_eq!(hit.fraction, 0.4596875011920929);
        let corner = collision
            .trace(&query(
                vec3(-50.0, -50.0, 0.0),
                vec3(50.0, 50.0, 0.0),
                SceneShape::Point,
            ))
            .expect("trace");
        match &corner.detail {
            TraceDetail::Q2 { secondary, .. } => {
                let runner = secondary.as_ref().expect("runner-up");
                assert_eq!(runner.plane.normal, vec3(0.0, -1.0, 0.0));
                assert_eq!(runner.surface.as_ref().expect("surface").flags, 3);
            }
            _ => panic!("q2 detail"),
        }
        let stuck = collision
            .trace(&query(vec3(16.0, 0.0, 0.0), vec3(16.0, 0.0, 0.0), SceneShape::Point))
            .expect("trace");
        assert_eq!(stuck.fraction, 0.0);
        assert!(stuck.all_solid && stuck.start_solid);
        let free = collision
            .trace(&query(vec3(-50.0, 0.0, 0.0), vec3(-50.0, 0.0, 0.0), SceneShape::Point))
            .expect("trace");
        assert_eq!(free.fraction, 1.0);
        assert_eq!(free.hit, TraceHit::None);
    }

    #[test]
    fn q2_queries_match_donor() {
        let collision = Q2Collision::new(geometry());
        let contents = |point| {
            collision
                .point_contents(&PointContentsQuery {
                    point,
                    target: QueryTarget::World,
                    policy: policy(),
                    numeric: Q3_BINARY32_PROFILE,
                    pass_actor: None,
                })
                .expect("contents")
        };
        assert_eq!(
            contents(vec3(16.0, 0.0, 0.0)),
            PointContentsResult::Q2 { stored: 1, merged: 1 }
        );
        assert_eq!(
            contents(vec3(-50.0, 0.0, 0.0)),
            PointContentsResult::Q2 { stored: 0, merged: 0 }
        );
        let leaves = collision
            .box_leaves(
                &Bounds {
                    min: vec3(-70.0, -70.0, -70.0),
                    max: vec3(70.0, 70.0, 70.0),
                },
                16,
                0,
            )
            .expect("leaves");
        assert_eq!(leaves.leaves, vec![0, 1]);
        assert_eq!(leaves.topnode, Some(0));
        assert!(!leaves.overflow);
        let capped = collision
            .box_leaves(
                &Bounds {
                    min: vec3(-70.0, -70.0, -70.0),
                    max: vec3(70.0, 70.0, 70.0),
                },
                1,
                0,
            )
            .expect("leaves");
        assert_eq!(capped.leaves, vec![0]);
        assert!(capped.overflow);
        assert_eq!(collision.point_leaf(vec3(-50.0, 0.0, 0.0), 0).expect("leaf"), 1);
        assert_eq!(collision.leaf_cluster(0).expect("cluster"), 0);
        assert_eq!(collision.leaf_area(1).expect("area"), 0);
        assert!(collision.areas_connected(0, 0).expect("areas"));
        assert!(!collision.areas_connected(0, 1).expect("areas"));
        collision.set_area_portal_state(7, true).expect("open");
        assert!(collision.areas_connected(0, 1).expect("areas"));
        assert_eq!(collision.portal_state(), vec![7]);
        assert_eq!(collision.area_bits(0).expect("bits"), vec![3]);
        assert_eq!(collision.area_bits(1).expect("bits"), vec![3]);
        assert!(collision.cluster_visible(0, 1, VisibilityKind::Pvs).expect("pvs"));
        assert!(collision.cluster_visible(0, 1, VisibilityKind::Phs).expect("phs"));
        assert!(!collision.cluster_visible(0, 0, VisibilityKind::Pvs).expect("pvs"));
        assert!(collision.cluster_visible(1, 1, VisibilityKind::Phs).expect("phs"));
        let media = collision
            .trace_media(
                &query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), SceneShape::Point),
                1.0,
            )
            .expect("media");
        assert!(media.in_open);
        assert!(!media.in_water);
        let error = collision.model_bounds(5).expect_err("bad model must fail");
        assert_eq!(error.to_string(), "Q2 collision index 5");
        let error = collision
            .set_area_portal_state(9, true)
            .expect_err("bad portal must fail");
        assert_eq!(error.to_string(), "Unknown Q2 area portal 9");
        let tasks: &dyn SceneQueries = &collision;
        assert!(tasks.areas_connected(0, 0));
    }

    #[test]
    fn q2_visibility_errors_match_donor() {
        let mut fixture = geometry();
        fixture.visibility = Some(Q2CollisionVisibility {
            clusters: vec![Q2VisibilityCluster {
                pvs_offset: 99,
                phs_offset: 0,
            }],
            compressed: vec![1],
        });
        let error = Q2Collision::new(fixture)
            .cluster_visible(0, 0, VisibilityKind::Pvs)
            .expect_err("outside read must fail");
        assert_eq!(error.to_string(), "Q2 visibility read outside lump");
        let mut fixture = geometry();
        fixture.visibility = Some(Q2CollisionVisibility {
            clusters: vec![Q2VisibilityCluster {
                pvs_offset: 0,
                phs_offset: 0,
            }],
            compressed: vec![0, 0],
        });
        let error = Q2Collision::new(fixture)
            .cluster_visible(0, 0, VisibilityKind::Pvs)
            .expect_err("zero run must fail");
        assert_eq!(error.to_string(), "Invalid Q2 visibility zero run");
    }
}
