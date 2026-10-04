//! Quake I collision provider: native hull traces plus derived-cell sweeps
//! for arbitrary shapes.
//!
//! Donor provenance:
//! - `/home/buzzkill/Projects/quake-typescript/src/world/collision/q1/index.ts`
//!   (`Q1Collision`, `createQ1Collision`, hull selection, clip-cell cache,
//!   PVS/PHS visibility)
//! - `/home/buzzkill/Projects/quake-typescript/src/world/collision/q1/hull.ts`
//!   (`createQ1Hulls`, `Q1_HULL_BOUNDS`; traversal lives in
//!   `qa_world::hull`)
//! - `/home/buzzkill/Projects/quake-typescript/src/formats/q1-map/queries.ts`
//!   (`q1FaceAtContact`, `q1LeafPvs` over the collision geometry view)

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use qa_content::bspx::decompress_q1_pvs;
use qa_content::materials::{q1_surface_kind, Q1SurfaceKind};
use qa_core::math::{angle_vectors, vec3, Bounds, Plane, Vec3};
use qa_core::numeric::NumericOps;
use qa_world::geometry::clipspace::{derive_q1_clip_solids, Q1ClipHull};
use qa_world::geometry::polyhedron::{
    add, box_cell, clip_cell, dot, lerp as dlerp, scale, sub, ConvexCell, DBounds, DPlane, DVec3, AXES,
};
use qa_world::geometry::solid_space::{
    Q1SolidGeometry, Q1SolidLeaf, Q1SolidModel, Q1SolidNode, Q1SolidSpace, SolidChild,
};
use qa_world::geometry::sweep::{shape_support, sweep_box_cell, sweep_capsule_cell, CellShape, SweepInterval};
use qa_world::hull::{ClipChild, ClipNode, Hull};
use qa_world::WorldError;

use crate::collision_support::{blocks_q1_contents, select_numeric};
use crate::scene::{
    scene_expect, BspEdge, BspPlane, IndexRange, LeafQueryResult, PointContentsQuery, PointContentsResult, QueryTarget,
    SceneQueries, TraceContact, TraceDetail, TraceHit, TracePolicy, TraceQuery, TraceResult, TraceShape,
    VisibilityKind, WorldKind,
};

/// Native hull bounds: point, player, shambler (`Q1_HULL_BOUNDS`).
const HULL_BOUNDS: [Bounds; 3] = [
    Bounds {
        min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    },
    Bounds {
        min: Vec3 {
            x: -16.0,
            y: -16.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 16.0,
            y: 16.0,
            z: 32.0,
        },
    },
    Bounds {
        min: Vec3 {
            x: -32.0,
            y: -32.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 32.0,
            y: 32.0,
            z: 64.0,
        },
    },
];

/// Hull plane distance epsilon (`Q1_DISTANCE_EPSILON`).
const DISTANCE_EPSILON: f64 = 1.0 / 32.0;

/// Derived clip-cell cache limits.
const CLIP_CACHE_ENTRIES: usize = 128;
const CLIP_CACHE_CELLS: usize = 1024;
const CLIP_CACHE_FACES: usize = 4096;
const CLIP_CACHE_VERTICES: usize = 16384;

/// Contents-blocking rule: donor `Q1CollisionOptions.blocksContents`.
pub type BlocksContents = fn(i32, &TracePolicy) -> bool;

/// Default rule: only solid blocks.
fn default_blocks(contents: i32, _policy: &TracePolicy) -> bool {
    contents == -2
}

fn to_vec3(value: DVec3) -> Vec3 {
    vec3(value.x as f32, value.y as f32, value.z as f32)
}

fn to_plane(value: DPlane) -> Plane {
    Plane {
        normal: to_vec3(value.normal),
        distance: value.distance as f32,
    }
}

fn to_dbounds(value: &Bounds) -> DBounds {
    DBounds {
        min: DVec3::from(value.min),
        max: DVec3::from(value.max),
    }
}

/// BSP child reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1BspChild {
    /// Node index.
    Node(usize),
    /// Leaf index.
    Leaf(usize),
}

/// Drawing BSP node view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1CollisionNode {
    /// Plane index.
    pub plane: usize,
    /// Front and back children.
    pub children: [Q1BspChild; 2],
}

/// Leaf view: contents plus visibility offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1CollisionLeaf {
    /// Leaf contents.
    pub contents: i32,
    /// Visibility lump offset; leaf zero sees everything.
    pub visibility_offset: Option<u32>,
}

/// Model view: bounds, hull roots, face span, visible-leaf count.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1CollisionModel {
    /// Model bounds.
    pub bounds: Bounds,
    /// Head node per hull; index zero is the drawing hull.
    pub headnodes: Vec<i32>,
    /// Visible leaf count for PVS rows.
    pub visible_leaves: i32,
    /// Authored face span.
    pub faces: IndexRange,
}

/// Face view for contact resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1CollisionFace {
    /// Plane index.
    pub plane: usize,
    /// Winding faces the plane back side.
    pub back: bool,
    /// Surface-edge span.
    pub edges: IndexRange,
    /// Texture-info index.
    pub texture_info: usize,
}

/// Texture-info view: only the texture index matters for sky flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1CollisionTextureInfo {
    /// Texture index.
    pub texture: usize,
}

/// Authored brush-list brush.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1CollisionBrush {
    /// Brush bounds.
    pub bounds: Bounds,
    /// Brush contents.
    pub contents: i32,
    /// Clipping planes.
    pub planes: Vec<Plane>,
}

/// BSPX brush-list entry for one model.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1BrushListEntry {
    /// Model index.
    pub model: usize,
    /// Authored brushes.
    pub brushes: Vec<Q1CollisionBrush>,
}

/// Collision subset of a decoded Quake I world.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1CollisionGeometry {
    /// Models; index zero is the world model.
    pub models: Vec<Q1CollisionModel>,
    /// Drawing BSP nodes.
    pub nodes: Vec<Q1CollisionNode>,
    /// Clip nodes for hulls one and two.
    pub clipnodes: Vec<ClipNode>,
    /// Planes.
    pub planes: Vec<BspPlane>,
    /// Leaves.
    pub leaves: Vec<Q1CollisionLeaf>,
    /// Faces.
    pub faces: Vec<Q1CollisionFace>,
    /// Texture infos.
    pub texture_info: Vec<Q1CollisionTextureInfo>,
    /// Texture names; missing slots are `None`.
    pub textures: Vec<Option<String>>,
    /// Vertices.
    pub vertices: Vec<Vec3>,
    /// Edges.
    pub edges: Vec<BspEdge>,
    /// Signed surface-edge references.
    pub surface_edges: Vec<i32>,
    /// Compressed visibility lump.
    pub visibility: Vec<u8>,
    /// Optional BSPX brush lists.
    pub brush_list: Vec<Q1BrushListEntry>,
}

/// Native hull with the actor box it was compiled for.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeHull {
    /// Clip tree.
    pub hull: Hull,
    /// Actor box.
    pub clip_bounds: Bounds,
}

/// How arbitrary shapes collide for a model (`geometryCoverage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbitraryShapeSource {
    /// BSPX authored brushes.
    BspxBrushes,
    /// Drawing-BSP derived cells.
    DrawingBspCells,
}

/// How clip-only solid collides for a model (`geometryCoverage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipOnlySource {
    /// BSPX authored brushes.
    Brushes,
    /// Derived native clipspace.
    DerivedNativeClipspace,
}

/// Coverage pair for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeometryCoverage {
    /// Arbitrary-shape source.
    pub arbitrary_shapes: ArbitraryShapeSource,
    /// Clip-only source.
    pub clip_only: ClipOnlySource,
}

/// Cached derived clip cells with accounting.
#[derive(Debug, Clone, PartialEq)]
struct CachedClipCells {
    cells: Vec<ConvexCell>,
    faces: usize,
    vertices: usize,
}

#[derive(Debug, Default)]
struct Caches {
    hulls: HashMap<i32, Vec<NativeHull>>,
    contact_faces: HashMap<i32, HashMap<String, Vec<usize>>>,
    pvs: HashMap<i32, Vec<u8>>,
    phs: HashMap<i32, Vec<u8>>,
    clip_order: VecDeque<String>,
    clip_cells: HashMap<String, CachedClipCells>,
    clip_cell_count: usize,
    clip_face_count: usize,
    clip_vertex_count: usize,
}

/// Quake I collision provider (`Q1Collision`).
#[derive(Debug)]
pub struct Q1Collision {
    geometry: Q1CollisionGeometry,
    solid_space: Q1SolidSpace,
    blocks: BlocksContents,
    caches: RefCell<Caches>,
}

fn solid_view(geometry: &Q1CollisionGeometry) -> Q1SolidGeometry {
    Q1SolidGeometry {
        models: geometry
            .models
            .iter()
            .map(|model| Q1SolidModel {
                headnodes: model.headnodes.clone(),
            })
            .collect(),
        nodes: geometry
            .nodes
            .iter()
            .map(|node| Q1SolidNode {
                plane: node.plane,
                children: node.children.map(|child| match child {
                    Q1BspChild::Node(index) => SolidChild::Node(index),
                    Q1BspChild::Leaf(index) => SolidChild::Leaf(index),
                }),
            })
            .collect(),
        planes: geometry
            .planes
            .iter()
            .map(|plane| DPlane {
                normal: DVec3::from(plane.normal),
                distance: f64::from(plane.distance),
            })
            .collect(),
        leaves: geometry
            .leaves
            .iter()
            .map(|leaf| Q1SolidLeaf {
                contents: leaf.contents,
            })
            .collect(),
    }
}

fn at<'a, T>(items: &'a [T], index: i32, what: &str) -> Result<&'a T, WorldError> {
    if index < 0 {
        return Err(WorldError::BadCollisionRecord(format!(
            "{what}:0: index {index} outside {} records",
            items.len()
        )));
    }
    items.get(index as usize).ok_or_else(|| {
        WorldError::BadCollisionRecord(format!("{what}:0: index {index} outside {} records", items.len()))
    })
}

fn basis(angles: DVec3) -> [DVec3; 3] {
    if angles.x == 0.0 && angles.y == 0.0 && angles.z == 0.0 {
        return AXES;
    }
    let vectors = angle_vectors(to_vec3(angles));
    [
        DVec3::from(vectors.forward),
        scale(DVec3::from(vectors.right), -1.0),
        DVec3::from(vectors.up),
    ]
}

fn to_local(value: DVec3, axis: &[DVec3; 3]) -> DVec3 {
    DVec3 {
        x: dot(value, axis[0]),
        y: dot(value, axis[1]),
        z: dot(value, axis[2]),
    }
}

fn from_local(value: DVec3, axis: &[DVec3; 3]) -> DVec3 {
    add(
        add(scale(axis[0], value.x), scale(axis[1], value.y)),
        scale(axis[2], value.z),
    )
}

fn world_plane(plane: DPlane, axis: &[DVec3; 3], origin: DVec3) -> DPlane {
    let normal = from_local(plane.normal, axis);
    DPlane {
        normal,
        distance: plane.distance + dot(normal, origin),
    }
}

fn dimensions(bounds: &DBounds) -> DVec3 {
    sub(bounds.max, bounds.min)
}

fn equal(a: DVec3, b: DVec3) -> bool {
    a.x == b.x && a.y == b.y && a.z == b.z
}

impl Q1Collision {
    /// Build a provider with the default solid-blocking rule.
    #[must_use]
    pub fn new(geometry: Q1CollisionGeometry) -> Self {
        Self::with_blocks(geometry, default_blocks)
    }

    /// Build a provider with a contents-blocking rule.
    #[must_use]
    pub fn with_blocks(geometry: Q1CollisionGeometry, blocks: BlocksContents) -> Self {
        let solid_space = Q1SolidSpace::new(solid_view(&geometry));
        Self {
            geometry,
            solid_space,
            blocks,
            caches: RefCell::new(Caches::default()),
        }
    }

    /// Borrow the collision geometry.
    #[must_use]
    pub fn geometry(&self) -> &Q1CollisionGeometry {
        &self.geometry
    }

    /// Native hulls for a model (`nativeHulls` + `createQ1Hulls`).
    pub fn native_hulls(&self, model: i32) -> Result<Vec<NativeHull>, WorldError> {
        if let Some(hulls) = self.caches.borrow().hulls.get(&model) {
            return Ok(hulls.clone());
        }
        let entry = self
            .geometry
            .models
            .get(model as usize)
            .filter(|_| model >= 0)
            .ok_or(WorldError::UnknownQ1Model(model))?;
        let mut drawing = Vec::with_capacity(self.geometry.nodes.len());
        for node in &self.geometry.nodes {
            let mut children = [ClipChild::Contents(-1), ClipChild::Contents(-1)];
            for (slot, child) in children.iter_mut().zip(node.children.iter()) {
                *slot = match child {
                    Q1BspChild::Node(index) => ClipChild::Node(*index),
                    Q1BspChild::Leaf(index) => {
                        let leaf = self
                            .geometry
                            .leaves
                            .get(*index)
                            .ok_or_else(|| WorldError::Hull("Unknown Quake hull-zero leaf".to_string()))?;
                        ClipChild::Contents(leaf.contents)
                    }
                };
            }
            drawing.push(ClipNode {
                plane: node.plane,
                children: [children[0], children[1]],
            });
        }
        let planes: Vec<qa_world::hull::BspPlane> = self
            .geometry
            .planes
            .iter()
            .map(|plane| qa_world::hull::BspPlane {
                normal: plane.normal,
                distance: plane.distance,
                plane_type: plane.plane_type as u8,
                signbits: plane.signbits as u8,
            })
            .collect();
        let mut hulls = Vec::with_capacity(HULL_BOUNDS.len());
        for (index, clip_bounds) in HULL_BOUNDS.iter().enumerate() {
            let mut first = *entry
                .headnodes
                .get(index)
                .ok_or_else(|| WorldError::Hull(format!("Missing Quake hull {index}")))?;
            if index == 0 && first < 0 {
                let leaf = self
                    .geometry
                    .leaves
                    .get((-1 - first) as usize)
                    .filter(|_| first < 0)
                    .ok_or_else(|| WorldError::Hull("Unknown Quake hull root leaf".to_string()))?;
                first = leaf.contents;
            }
            let clipnodes = if index == 0 {
                drawing.clone()
            } else {
                self.geometry.clipnodes.clone()
            };
            #[allow(clippy::cast_possible_wrap)]
            let last = clipnodes.len() as i32 - 1;
            hulls.push(NativeHull {
                hull: Hull {
                    planes: planes.clone(),
                    clipnodes,
                    first,
                    last,
                },
                clip_bounds: *clip_bounds,
            });
        }
        self.caches.borrow_mut().hulls.insert(model, hulls.clone());
        Ok(hulls)
    }

    /// Coverage sources for a model (`geometryCoverage`).
    #[must_use]
    pub fn geometry_coverage(&self, model: usize) -> GeometryCoverage {
        if self.geometry.brush_list.iter().any(|entry| entry.model == model) {
            GeometryCoverage {
                arbitrary_shapes: ArbitraryShapeSource::BspxBrushes,
                clip_only: ClipOnlySource::Brushes,
            }
        } else {
            GeometryCoverage {
                arbitrary_shapes: ArbitraryShapeSource::DrawingBspCells,
                clip_only: ClipOnlySource::DerivedNativeClipspace,
            }
        }
    }
}

impl Q1Collision {
    /// Sweep a query through Quake I collision (`trace`).
    pub fn trace(&self, query: &TraceQuery) -> Result<TraceResult, WorldError> {
        let numeric = select_numeric(&query.numeric)?;
        let (model, origin, axis) = match &query.target {
            QueryTarget::World => (0, DVec3 { x: 0.0, y: 0.0, z: 0.0 }, AXES),
            QueryTarget::Model { model, origin, angles } => (*model, DVec3::from(*origin), basis(DVec3::from(*angles))),
        };
        let bounds = match &query.shape {
            TraceShape::Point => HULL_BOUNDS[0],
            TraceShape::Box { bounds } | TraceShape::Capsule { bounds } => *bounds,
        };
        let mut hull_index = matches!(query.shape, TraceShape::Point).then_some(0);
        if let TracePolicy::Q1 { hull, .. } = &query.policy {
            if let Some(forced) = hull {
                hull_index = Some(*forced);
            } else if let TraceShape::Box { .. } = &query.shape {
                let size = dimensions(&to_dbounds(&bounds));
                if let Some(index) = HULL_BOUNDS
                    .iter()
                    .position(|candidate| equal(dimensions(&to_dbounds(candidate)), size))
                {
                    hull_index = Some(index as i32);
                }
            }
        }
        if let Some(index) = hull_index {
            return self.native_trace(query, model, index, &bounds, &origin, &axis, &numeric);
        }
        self.arbitrary_trace(query, model, &bounds, &origin, &axis)
    }

    /// Trace through one native hull.
    #[allow(clippy::too_many_arguments)]
    fn native_trace(
        &self,
        query: &TraceQuery,
        model: i32,
        hull_index: i32,
        bounds: &Bounds,
        origin: &DVec3,
        axis: &[DVec3; 3],
        numeric: &NumericOps,
    ) -> Result<TraceResult, WorldError> {
        let hulls = self.native_hulls(model)?;
        if hull_index < 0 {
            return Err(WorldError::Hull(format!("Unsupported Quake native hull {hull_index}")));
        }
        let native = hulls
            .get(hull_index as usize)
            .ok_or_else(|| WorldError::Hull(format!("Unsupported Quake native hull {hull_index}")))?;
        let offset = add(
            *origin,
            sub(to_dbounds(&native.clip_bounds).min, to_dbounds(bounds).min),
        );
        let start = to_vec3(to_local(sub(DVec3::from(query.start), offset), axis));
        let end = to_vec3(to_local(sub(DVec3::from(query.end), offset), axis));
        let blocks = self.blocks;
        let policy = &query.policy;
        let trace =
            qa_world::hull::trace_hull(&native.hull, start, end, numeric, &|contents| blocks(contents, policy))?;
        let plane = world_plane(DPlane::from(trace.plane), axis, offset);
        let hit = trace.fraction < 1.0 || trace.start_solid;
        let surface_flags = if hull_index == 0 && trace.fraction < 1.0 && !trace.all_solid {
            self.surface_flags(model, DVec3::from(trace.end), &DPlane::from(trace.plane))?
        } else {
            None
        };
        let end = if trace.fraction == 1.0 {
            query.end
        } else {
            to_vec3(add(from_local(DVec3::from(trace.end), axis), offset))
        };
        Ok(TraceResult {
            fraction: trace.fraction,
            end,
            start_solid: trace.start_solid,
            all_solid: trace.all_solid,
            contact: if trace.fraction < 1.0 {
                TraceContact::Plane { plane: to_plane(plane) }
            } else {
                TraceContact::None
            },
            hit: if hit { TraceHit::World { model } } else { TraceHit::None },
            detail: TraceDetail::Q1 {
                in_open: trace.in_open,
                in_water: trace.in_water,
                source_plane: Plane {
                    normal: to_vec3(plane.normal),
                    distance: trace.plane.distance,
                },
                surface_flags,
                contents: Some(trace.contents),
            },
        })
    }

    /// Sweep an arbitrary shape through derived cells.
    fn arbitrary_trace(
        &self,
        query: &TraceQuery,
        model: i32,
        bounds: &Bounds,
        origin: &DVec3,
        axis: &[DVec3; 3],
    ) -> Result<TraceResult, WorldError> {
        let shape_bounds = to_dbounds(bounds);
        let center_offset = scale(add(shape_bounds.min, shape_bounds.max), 0.5);
        let extents = scale(dimensions(&shape_bounds), 0.5);
        let center_start = to_local(sub(add(DVec3::from(query.start), center_offset), *origin), axis);
        let center_end = to_local(sub(add(DVec3::from(query.end), center_offset), *origin), axis);
        let local_axes = [
            to_local(AXES[0], axis),
            to_local(AXES[1], axis),
            to_local(AXES[2], axis),
        ];
        let radius = extents.x.min(extents.y).min(extents.z);
        let shape = match &query.shape {
            TraceShape::Capsule { .. } => CellShape::Capsule {
                axis: local_axes[2],
                radius,
                half_segment: (extents.z - radius).max(0.0),
            },
            _ => CellShape::Box {
                axes: local_axes,
                extents,
            },
        };
        let pad = DVec3 {
            x: shape_support(shape, AXES[0]) + 1.0,
            y: shape_support(shape, AXES[1]) + 1.0,
            z: shape_support(shape, AXES[2]) + 1.0,
        };
        let envelope_to = |end: DVec3| DBounds {
            min: DVec3 {
                x: center_start.x.min(end.x) - pad.x,
                y: center_start.y.min(end.y) - pad.y,
                z: center_start.z.min(end.z) - pad.z,
            },
            max: DVec3 {
                x: center_start.x.max(end.x) + pad.x,
                y: center_start.y.max(end.y) + pad.y,
                z: center_start.z.max(end.z) + pad.z,
            },
        };
        let envelope = envelope_to(center_end);
        let mut intervals: Vec<(SweepInterval, i32)> = Vec::new();
        for solid in self.cells(&envelope, model, &query.policy)? {
            let interval = match shape {
                CellShape::Box { axes, extents } => {
                    sweep_box_cell(&solid.cell, center_start, center_end, axes, extents, DISTANCE_EPSILON)
                }
                CellShape::Capsule {
                    axis,
                    radius,
                    half_segment,
                } => sweep_capsule_cell(
                    &solid.cell,
                    center_start,
                    center_end,
                    axis,
                    radius,
                    half_segment,
                    DISTANCE_EPSILON,
                )?,
            };
            if let Some(hit) = interval {
                intervals.push((hit, solid.contents));
            }
        }
        let has_brushes = self.geometry.brush_list.iter().any(|entry| entry.model as i32 == model);
        if !has_brushes && (self.blocks)(-2, &query.policy) {
            let drawing_count = intervals.len();
            let mut first = 1.0f64;
            for (interval, _) in &intervals {
                first = first.min(interval.enter);
            }
            let prefix = if first > 0.0 && first < 1.0 {
                envelope_to(dlerp(center_start, center_end, first))
            } else {
                envelope
            };
            let full_prefix = first <= 0.0 || first >= 1.0;
            let collect = |bounds: &DBounds, intervals: &mut Vec<(SweepInterval, i32)>| -> Result<bool, WorldError> {
                let mut starts_solid = false;
                for cell in self.derived_clip_cells(model, bounds)? {
                    let interval = match shape {
                        CellShape::Box { axes, extents } => {
                            sweep_box_cell(&cell, center_start, center_end, axes, extents, DISTANCE_EPSILON)
                        }
                        CellShape::Capsule {
                            axis,
                            radius,
                            half_segment,
                        } => sweep_capsule_cell(
                            &cell,
                            center_start,
                            center_end,
                            axis,
                            radius,
                            half_segment,
                            DISTANCE_EPSILON,
                        )?,
                    };
                    if let Some(hit) = interval {
                        starts_solid |= hit.enter <= 0.0;
                        intervals.push((hit, -2));
                    }
                }
                Ok(starts_solid)
            };
            if collect(&prefix, &mut intervals)? && !full_prefix {
                intervals.truncate(drawing_count);
                collect(&envelope, &mut intervals)?;
            }
        }
        intervals.sort_by(|a, b| a.0.enter.partial_cmp(&b.0.enter).unwrap_or(std::cmp::Ordering::Equal));
        let mut start_solid = false;
        let mut covered = f64::NEG_INFINITY;
        let mut fraction = 1.0;
        let mut plane = DPlane {
            normal: DVec3 { x: 0.0, y: 0.0, z: 0.0 },
            distance: 0.0,
        };
        let mut contents = -1;
        for (interval, hit_contents) in &intervals {
            if interval.enter <= 0.0 {
                start_solid = true;
                contents = *hit_contents;
                covered = covered.max(interval.exit);
                continue;
            }
            if start_solid && interval.enter <= covered + 1e-8 {
                covered = covered.max(interval.exit);
                continue;
            }
            fraction = interval.contact.max(0.0);
            plane = interval.plane;
            contents = *hit_contents;
            break;
        }
        let hulls = self.native_hulls(model)?;
        let point_hull = hulls
            .first()
            .ok_or_else(|| WorldError::Hull("Missing Quake point hull".to_string()))?;
        let numeric = select_numeric(&query.numeric)?;
        let blocks = self.blocks;
        let policy = &query.policy;
        let stop = dlerp(DVec3::from(query.start), DVec3::from(query.end), fraction);
        let environment = qa_world::hull::trace_hull(
            &point_hull.hull,
            to_vec3(to_local(sub(DVec3::from(query.start), *origin), axis)),
            to_vec3(to_local(sub(stop, *origin), axis)),
            &numeric,
            &|value| blocks(value, policy),
        )?;
        let transformed = world_plane(plane, axis, *origin);
        let end = if fraction == 1.0 {
            query.end
        } else {
            to_vec3(dlerp(DVec3::from(query.start), DVec3::from(query.end), fraction))
        };
        Ok(TraceResult {
            fraction,
            end,
            start_solid,
            all_solid: start_solid && covered >= 1.0,
            contact: if fraction < 1.0 {
                TraceContact::Plane {
                    plane: to_plane(transformed),
                }
            } else {
                TraceContact::None
            },
            hit: if fraction < 1.0 || start_solid {
                TraceHit::World { model }
            } else {
                TraceHit::None
            },
            detail: TraceDetail::Q1 {
                in_open: environment.in_open,
                in_water: environment.in_water,
                source_plane: Plane {
                    normal: to_vec3(transformed.normal),
                    distance: plane.distance as f32,
                },
                surface_flags: None,
                contents: Some(contents),
            },
        })
    }

    /// Solid cells under an envelope: authored brushes win over derived
    /// drawing cells.
    fn cells(&self, envelope: &DBounds, model: i32, policy: &TracePolicy) -> Result<Vec<Q1CellHit>, WorldError> {
        if let Some(authored) = self
            .geometry
            .brush_list
            .iter()
            .find(|entry| entry.model as i32 == model)
        {
            let mut hits = Vec::new();
            for brush in &authored.brushes {
                if !(self.blocks)(brush.contents, policy) {
                    continue;
                }
                let mut cell = Some(box_cell(&to_dbounds(&brush.bounds)));
                for plane in &brush.planes {
                    cell = cell.and_then(|current| clip_cell(&current, DPlane::from(*plane)));
                    if cell.is_none() {
                        break;
                    }
                }
                if let Some(cell) = cell {
                    hits.push(Q1CellHit {
                        cell,
                        contents: brush.contents,
                    });
                }
            }
            return Ok(hits);
        }
        let model_index = usize::try_from(model).map_err(|_| WorldError::UnknownQ1Model(model))?;
        let blocks = self.blocks;
        let found = self
            .solid_space
            .cells(envelope, model_index, &|contents| blocks(contents, policy))?;
        Ok(found
            .into_iter()
            .map(|solid| Q1CellHit {
                cell: solid.cell,
                contents: solid.contents,
            })
            .collect())
    }

    /// Derived clip-only cells under an envelope, through the bounded
    /// recency cache.
    fn derived_clip_cells(&self, model: i32, envelope: &DBounds) -> Result<Vec<ConvexCell>, WorldError> {
        let finite = [
            envelope.min.x,
            envelope.min.y,
            envelope.min.z,
            envelope.max.x,
            envelope.max.y,
            envelope.max.z,
        ]
        .iter()
        .all(|value| value.is_finite());
        if !finite {
            return self.derive_clip_cells(model, envelope);
        }
        let key = format!(
            "{model}:{}, {}, {}, {}, {}, {}",
            envelope_coordinate(envelope.min.x),
            envelope_coordinate(envelope.min.y),
            envelope_coordinate(envelope.min.z),
            envelope_coordinate(envelope.max.x),
            envelope_coordinate(envelope.max.y),
            envelope_coordinate(envelope.max.z)
        );
        let cached = self.caches.borrow_mut().clip_cells.remove(&key);
        if let Some(cached) = cached {
            let cells = cached.cells.clone();
            let mut caches = self.caches.borrow_mut();
            caches.clip_order.retain(|entry| entry != &key);
            caches.clip_order.push_back(key.clone());
            caches.clip_cells.insert(key, cached);
            return Ok(cells);
        }
        let cells = self.derive_clip_cells(model, envelope)?;
        if cells.len() > CLIP_CACHE_CELLS {
            return Ok(cells);
        }
        let mut faces = 0;
        let mut vertices = 0;
        for cell in &cells {
            faces += cell.faces.len();
            if faces > CLIP_CACHE_FACES {
                return Ok(cells);
            }
            for face in &cell.faces {
                vertices += face.vertices.len();
                if vertices > CLIP_CACHE_VERTICES {
                    return Ok(cells);
                }
            }
        }
        let mut caches = self.caches.borrow_mut();
        while caches.clip_cells.len() >= CLIP_CACHE_ENTRIES
            || caches.clip_cell_count + cells.len() > CLIP_CACHE_CELLS
            || caches.clip_face_count + faces > CLIP_CACHE_FACES
            || caches.clip_vertex_count + vertices > CLIP_CACHE_VERTICES
        {
            let oldest = caches.clip_order.pop_front().ok_or_else(|| {
                WorldError::BadCollisionRecord("Quake clip-cell cache accounting is inconsistent".to_string())
            })?;
            if let Some(evicted) = caches.clip_cells.remove(&oldest) {
                caches.clip_cell_count -= evicted.cells.len();
                caches.clip_face_count -= evicted.faces;
                caches.clip_vertex_count -= evicted.vertices;
            }
        }
        caches.clip_order.push_back(key.clone());
        caches.clip_cells.insert(
            key,
            CachedClipCells {
                cells: cells.clone(),
                faces,
                vertices,
            },
        );
        caches.clip_cell_count += cells.len();
        caches.clip_face_count += faces;
        caches.clip_vertex_count += vertices;
        Ok(cells)
    }

    /// Derive clip-only cells without consulting the cache.
    fn derive_clip_cells(&self, model: i32, envelope: &DBounds) -> Result<Vec<ConvexCell>, WorldError> {
        let hulls = self.native_hulls(model)?;
        let views: Vec<Q1ClipHull> = hulls
            .iter()
            .map(|native| Q1ClipHull {
                hull: &native.hull,
                clip_bounds: to_dbounds(&native.clip_bounds),
            })
            .collect();
        derive_q1_clip_solids(&views, envelope)
    }

    /// Surface flags at a hull-zero contact (`surfaceFlags`).
    fn surface_flags(&self, model: i32, point: DVec3, plane: &DPlane) -> Result<Option<i32>, WorldError> {
        let key_of = |value: &DPlane| {
            format!(
                "{},{},{},{}",
                value.normal.x, value.normal.y, value.normal.z, value.distance
            )
        };
        if !self.caches.borrow().contact_faces.contains_key(&model) {
            let entry = self
                .geometry
                .models
                .get(model as usize)
                .filter(|_| model >= 0)
                .ok_or(WorldError::UnknownQ1Model(model))?;
            let mut indexed: HashMap<String, Vec<usize>> = HashMap::new();
            for offset in 0..entry.faces.count {
                let face_index = entry.faces.first + offset;
                let face = self
                    .geometry
                    .faces
                    .get(face_index)
                    .ok_or_else(|| WorldError::Hull("Missing Quake face plane".to_string()))?;
                let authored = self
                    .geometry
                    .planes
                    .get(face.plane)
                    .ok_or_else(|| WorldError::Hull("Missing Quake face plane".to_string()))?;
                let oriented = if face.back {
                    DPlane {
                        normal: scale(DVec3::from(authored.normal), -1.0),
                        distance: -f64::from(authored.distance),
                    }
                } else {
                    DPlane {
                        normal: DVec3::from(authored.normal),
                        distance: f64::from(authored.distance),
                    }
                };
                indexed.entry(key_of(&oriented)).or_default().push(face_index);
            }
            self.caches.borrow_mut().contact_faces.insert(model, indexed);
        }
        let caches = self.caches.borrow();
        let indexed = caches.contact_faces.get(&model);
        let candidates = indexed
            .and_then(|faces| faces.get(&key_of(plane)).cloned())
            .unwrap_or_default();
        drop(caches);
        if candidates.is_empty() {
            return Ok(None);
        }
        let face_index = match self.face_at_contact(&candidates, point, plane)? {
            Some(index) => index,
            None => return Ok(None),
        };
        let face = self
            .geometry
            .faces
            .get(face_index)
            .ok_or_else(|| WorldError::Hull("Missing Quake face plane".to_string()))?;
        let name = self
            .geometry
            .texture_info
            .get(face.texture_info)
            .and_then(|info| self.geometry.textures.get(info.texture))
            .and_then(|slot| slot.as_ref());
        match name {
            Some(texture) if q1_surface_kind(texture) == Q1SurfaceKind::Sky => Ok(Some(4)),
            Some(_) => Ok(Some(0)),
            None => Ok(None),
        }
    }

    /// Authored face at a hull-zero contact, without retracing
    /// (`q1FaceAtContact`).
    fn face_at_contact(&self, candidates: &[usize], point: DVec3, plane: &DPlane) -> Result<Option<usize>, WorldError> {
        let normal = plane.normal;
        let norm_squared = normal.x * normal.x + normal.y * normal.y + normal.z * normal.z;
        let distance = (point.x * normal.x + point.y * normal.y + point.z * normal.z - plane.distance) / norm_squared;
        let projected = DVec3 {
            x: point.x - normal.x * distance,
            y: point.y - normal.y * distance,
            z: point.z - normal.z * distance,
        };
        for face_index in candidates {
            let vertices = self.face_vertices(*face_index)?;
            let mut positive = false;
            let mut negative = false;
            for (i, a) in vertices.iter().enumerate() {
                let b = vertices[(i + 1) % vertices.len()];
                let edge = sub(b, *a);
                let offset = sub(projected, *a);
                let side = (edge.y * offset.z - edge.z * offset.y) * normal.x
                    + (edge.z * offset.x - edge.x * offset.z) * normal.y
                    + (edge.x * offset.y - edge.y * offset.x) * normal.z;
                positive |= side > 0.0;
                negative |= side < 0.0;
                if positive && negative {
                    break;
                }
            }
            if vertices.len() >= 3 && !(positive && negative) {
                return Ok(Some(*face_index));
            }
        }
        Ok(None)
    }

    /// Vertices of a face (`q1FaceVertices`).
    fn face_vertices(&self, face_index: usize) -> Result<Vec<DVec3>, WorldError> {
        let face = at(&self.geometry.faces, face_index as i32, "Quake face")?;
        let mut vertices = Vec::with_capacity(face.edges.count);
        for offset in 0..face.edges.count {
            let edge_index = self
                .geometry
                .surface_edges
                .get(face.edges.first + offset)
                .copied()
                .ok_or_else(|| WorldError::Hull("Missing Quake surface edge".to_string()))?;
            let edge = self
                .geometry
                .edges
                .get(edge_index.unsigned_abs() as usize)
                .ok_or_else(|| WorldError::Hull("Missing Quake edge".to_string()))?;
            let vertex = self
                .geometry
                .vertices
                .get(edge.vertices[usize::from(edge_index < 0)] as usize)
                .copied()
                .ok_or_else(|| WorldError::Hull("Missing Quake vertex".to_string()))?;
            vertices.push(DVec3::from(vertex));
        }
        Ok(vertices)
    }
}

/// One solid cell with its contents.
struct Q1CellHit {
    cell: ConvexCell,
    contents: i32,
}

/// Cache-key coordinate: negative zero keeps its sign.
fn envelope_coordinate(value: f64) -> String {
    if value == 0.0 && value.is_sign_negative() {
        "-0".to_string()
    } else {
        value.to_string()
    }
}

impl Q1Collision {
    /// Contents at a point (`pointContents`).
    pub fn point_contents(&self, query: &PointContentsQuery) -> Result<PointContentsResult, WorldError> {
        let numeric = select_numeric(&query.numeric)?;
        let (model, point) = match &query.target {
            QueryTarget::World => (0, DVec3::from(query.point)),
            QueryTarget::Model { model, origin, angles } => (
                *model,
                to_local(
                    sub(DVec3::from(query.point), DVec3::from(*origin)),
                    &basis(DVec3::from(*angles)),
                ),
            ),
        };
        let hulls = self.native_hulls(model)?;
        let hull = hulls
            .first()
            .ok_or_else(|| WorldError::Hull("Missing Quake point hull".to_string()))?;
        let contents = qa_world::hull::hull_point_contents(&hull.hull, to_vec3(point), &numeric)?;
        Ok(PointContentsResult::Q1 { contents })
    }

    /// Leaf containing a point (`leafAt`).
    pub fn leaf_at(&self, point: Vec3, model: usize) -> Result<i32, WorldError> {
        let root = self
            .geometry
            .models
            .get(model)
            .and_then(|entry| entry.headnodes.first().copied())
            .ok_or(WorldError::UnknownQ1Model(model as i32))?;
        let point = DVec3::from(point);
        let mut child = if root < 0 {
            Q1BspChild::Leaf((-1 - root) as usize)
        } else {
            Q1BspChild::Node(root as usize)
        };
        for _ in 0..=self.geometry.nodes.len() {
            match child {
                Q1BspChild::Leaf(index) => {
                    return Ok(index as i32);
                }
                Q1BspChild::Node(index) => {
                    let node = self
                        .geometry
                        .nodes
                        .get(index)
                        .ok_or_else(|| WorldError::Hull("Invalid Quake BSP node".to_string()))?;
                    let plane = self
                        .geometry
                        .planes
                        .get(node.plane)
                        .ok_or_else(|| WorldError::Hull("Invalid Quake BSP node".to_string()))?;
                    let normal = DVec3::from(plane.normal);
                    let side = usize::from(
                        point.x * normal.x + point.y * normal.y + point.z * normal.z - f64::from(plane.distance) < 0.0,
                    );
                    child = node.children[side];
                }
            }
        }
        Err(WorldError::Hull("Cycle in Quake BSP".to_string()))
    }

    /// Leaf containing a point in the world model (`pointLeaf`).
    pub fn point_leaf(&self, point: Vec3) -> Result<i32, WorldError> {
        self.leaf_at(point, 0)
    }

    /// Cluster for a leaf (`leafCluster`).
    #[must_use]
    pub fn leaf_cluster(leaf: i32) -> i32 {
        if leaf == 0 {
            -1
        } else {
            leaf - 1
        }
    }

    /// Area for a leaf: Quake I has one area (`leafArea`).
    #[must_use]
    pub fn leaf_area(&self, _leaf: i32) -> i32 {
        0
    }

    /// Model bounds (`modelBounds`).
    pub fn model_bounds(&self, model: i32) -> Result<Bounds, WorldError> {
        self.geometry
            .models
            .get(model as usize)
            .filter(|_| model >= 0)
            .map(|entry| entry.bounds)
            .ok_or(WorldError::UnknownQ1Model(model))
    }

    /// Area visibility bits: Quake I has one visible area (`areaBits`).
    #[must_use]
    pub fn area_bits(&self, _area: i32) -> Vec<u8> {
        vec![1]
    }

    /// Area connectivity: only area zero connects to itself
    /// (`areasConnected`).
    #[must_use]
    pub fn areas_connected(&self, first: i32, second: i32) -> bool {
        first == 0 && second == 0
    }

    /// Leaves touched by bounds (`boxLeaves`).
    pub fn box_leaves(&self, bounds: &Bounds, limit: usize) -> Result<LeafQueryResult, WorldError> {
        let root = self
            .geometry
            .models
            .first()
            .and_then(|entry| entry.headnodes.first().copied())
            .ok_or_else(|| WorldError::Hull("Missing Quake world root".to_string()))?;
        let mut leaves = Vec::new();
        let mut seen = HashSet::new();
        let mut topnode: Option<i32> = None;
        let mut overflow = false;
        let mut stack = vec![if root < 0 {
            Q1BspChild::Leaf((-1 - root) as usize)
        } else {
            Q1BspChild::Node(root as usize)
        }];
        let bounds = to_dbounds(bounds);
        while let Some(child) = stack.pop() {
            match child {
                Q1BspChild::Leaf(index) => {
                    if index == 0 || !seen.insert(index) {
                        continue;
                    }
                    if leaves.len() < limit {
                        leaves.push(index as i32);
                    } else {
                        overflow = true;
                    }
                }
                Q1BspChild::Node(index) => {
                    let node = self
                        .geometry
                        .nodes
                        .get(index)
                        .ok_or_else(|| WorldError::Hull("Invalid Quake leaf query node".to_string()))?;
                    let plane = self
                        .geometry
                        .planes
                        .get(node.plane)
                        .ok_or_else(|| WorldError::Hull("Invalid Quake leaf query node".to_string()))?;
                    let normal = DVec3::from(plane.normal);
                    let center = scale(add(bounds.min, bounds.max), 0.5);
                    let extents = scale(sub(bounds.max, bounds.min), 0.5);
                    let d = dot(center, normal) - f64::from(plane.distance);
                    let r = normal.x.abs() * extents.x + normal.y.abs() * extents.y + normal.z.abs() * extents.z;
                    if d >= r {
                        stack.push(node.children[0]);
                    } else if d < -r {
                        stack.push(node.children[1]);
                    } else {
                        if topnode.is_none() {
                            topnode = Some(index as i32);
                        }
                        stack.push(node.children[1]);
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

    /// Decompressed visibility row for a leaf, through the row cache.
    fn visibility(&self, leaf: i32, kind: VisibilityKind) -> Result<Vec<u8>, WorldError> {
        let cached = match kind {
            VisibilityKind::Pvs => self.caches.borrow().pvs.get(&leaf).cloned(),
            VisibilityKind::Phs => self.caches.borrow().phs.get(&leaf).cloned(),
        };
        if let Some(row) = cached {
            return Ok(row);
        }
        let leaf_index = usize::try_from(leaf).map_err(|_| {
            WorldError::BadCollisionRecord(format!(
                "Quake PVS leaf:0: index {leaf} outside {} records",
                self.geometry.leaves.len()
            ))
        })?;
        let entry = self.geometry.leaves.get(leaf_index).ok_or_else(|| {
            WorldError::BadCollisionRecord(format!(
                "Quake PVS leaf:0: index {leaf} outside {} records",
                self.geometry.leaves.len()
            ))
        })?;
        #[allow(clippy::cast_sign_loss)]
        let count = self
            .geometry
            .models
            .first()
            .map_or(self.geometry.leaves.len().saturating_sub(1), |world| {
                (world.visible_leaves.max(0)) as usize
            });
        let offset = if leaf == 0 { None } else { entry.visibility_offset };
        let mut row = decompress_q1_pvs(&self.geometry.visibility, offset, count)
            .map_err(|error| WorldError::BadCollisionRecord(error.to_string()))?;
        if kind == VisibilityKind::Phs {
            let source = row.clone();
            for other in 1..self.geometry.leaves.len() {
                let byte = source.get((other - 1) >> 3).copied().unwrap_or(0);
                if byte & (1 << ((other - 1) & 7)) == 0 {
                    continue;
                }
                let visible = self.visibility(other as i32, VisibilityKind::Pvs)?;
                for (slot, bits) in row.iter_mut().zip(visible.iter()) {
                    *slot |= bits;
                }
            }
        }
        match kind {
            VisibilityKind::Pvs => self.caches.borrow_mut().pvs.insert(leaf, row.clone()),
            VisibilityKind::Phs => self.caches.borrow_mut().phs.insert(leaf, row.clone()),
        };
        Ok(row)
    }

    /// Cluster visibility (`clusterVisible`).
    pub fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> Result<bool, WorldError> {
        if to < 0 {
            return Ok(false);
        }
        if from < 0 {
            return Ok(true);
        }
        let row = self.visibility(from + 1, kind)?;
        #[allow(clippy::cast_sign_loss)]
        let to = to as usize;
        Ok(row.get(to >> 3).copied().unwrap_or(0) & (1 << (to & 7)) != 0)
    }
}

/// Build a Quake I collision provider (`createQ1Collision`).
#[must_use]
pub fn create_q1_collision(geometry: Q1CollisionGeometry) -> Q1Collision {
    Q1Collision::new(geometry)
}

/// Build a provider with the shared contents-blocking rule, as the scene
/// aggregator does.
#[must_use]
pub fn create_shared_q1_collision(geometry: Q1CollisionGeometry) -> Q1Collision {
    Q1Collision::with_blocks(geometry, blocks_q1_contents)
}

impl SceneQueries for Q1Collision {
    fn trace(&self, query: &TraceQuery) -> TraceResult {
        scene_expect(self.trace(query), WorldKind::Q1Bsp, "trace")
    }

    fn point_contents(&self, query: &PointContentsQuery) -> PointContentsResult {
        scene_expect(self.point_contents(query), WorldKind::Q1Bsp, "contents")
    }

    fn box_leaves(&self, bounds: &Bounds, limit: usize) -> LeafQueryResult {
        scene_expect(self.box_leaves(bounds, limit), WorldKind::Q1Bsp, "leaves")
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        self.areas_connected(first, second)
    }

    fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> bool {
        scene_expect(self.cluster_visible(from, to, kind), WorldKind::Q1Bsp, "visibility")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;
    use qa_core::numeric::Q3_BINARY32_PROFILE;

    use crate::scene::{Q1MoveRule, TraceShape as SceneShape};

    fn geometry() -> Q1CollisionGeometry {
        Q1CollisionGeometry {
            models: vec![
                Q1CollisionModel {
                    bounds: Bounds {
                        min: vec3(-64.0, -64.0, -64.0),
                        max: vec3(64.0, 64.0, 64.0),
                    },
                    headnodes: vec![0, 0, 0],
                    visible_leaves: 1,
                    faces: IndexRange { first: 0, count: 1 },
                },
                Q1CollisionModel {
                    bounds: Bounds {
                        min: vec3(-64.0, -64.0, -64.0),
                        max: vec3(64.0, 64.0, 64.0),
                    },
                    headnodes: vec![0, 0, 0],
                    visible_leaves: 1,
                    faces: IndexRange { first: 5, count: 1 },
                },
            ],
            nodes: vec![Q1CollisionNode {
                plane: 0,
                children: [Q1BspChild::Leaf(0), Q1BspChild::Leaf(1)],
            }],
            clipnodes: vec![ClipNode {
                plane: 0,
                children: [ClipChild::Contents(-2), ClipChild::Contents(-1)],
            }],
            planes: vec![BspPlane {
                normal: vec3(1.0, 0.0, 0.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            }],
            leaves: vec![
                Q1CollisionLeaf {
                    contents: -2,
                    visibility_offset: None,
                },
                Q1CollisionLeaf {
                    contents: -1,
                    visibility_offset: Some(0),
                },
            ],
            faces: vec![Q1CollisionFace {
                plane: 0,
                back: true,
                edges: IndexRange { first: 0, count: 4 },
                texture_info: 0,
            }],
            texture_info: vec![Q1CollisionTextureInfo { texture: 0 }],
            textures: vec![Some("sky1".to_string())],
            vertices: vec![
                vec3(0.0, -16.0, -16.0),
                vec3(0.0, 16.0, -16.0),
                vec3(0.0, 16.0, 16.0),
                vec3(0.0, -16.0, 16.0),
            ],
            edges: vec![
                BspEdge { vertices: [0, 1] },
                BspEdge { vertices: [1, 2] },
                BspEdge { vertices: [2, 3] },
                BspEdge { vertices: [3, 0] },
            ],
            surface_edges: vec![0, 1, 2, 3],
            visibility: vec![0x01],
            brush_list: vec![],
        }
    }

    fn policy() -> TracePolicy {
        TracePolicy::Q1 {
            move_rule: Q1MoveRule::Normal,
            hull: None,
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
    fn q1_native_traces_match_donor() {
        let collision = Q1Collision::new(geometry());
        let hit = collision
            .trace(&query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), SceneShape::Point))
            .expect("trace");
        assert_eq!(hit.fraction, 0.4996874928474426);
        assert_eq!(hit.end.x, -0.03125);
        assert_eq!(hit.hit, TraceHit::World { model: 0 });
        match &hit.detail {
            TraceDetail::Q1 {
                in_open,
                in_water,
                source_plane,
                surface_flags,
                contents,
            } => {
                assert!(in_open);
                assert!(!in_water);
                assert_eq!(source_plane.normal, vec3(-1.0, 0.0, 0.0));
                assert_eq!(*surface_flags, Some(4));
                assert_eq!(*contents, Some(-2));
            }
            _ => panic!("q1 detail"),
        }
        let player = SceneShape::Box {
            bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
        };
        let hit = collision
            .trace(&query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), player))
            .expect("trace");
        assert_eq!(hit.fraction, 0.4996874928474426);
        match &hit.detail {
            TraceDetail::Q1 {
                surface_flags,
                contents,
                ..
            } => {
                assert_eq!(*surface_flags, None);
                assert_eq!(*contents, Some(-2));
            }
            _ => panic!("q1 detail"),
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
        assert_eq!(hit.fraction, 0.4596875);
        assert_eq!(hit.end.x, -4.03125);
        match &hit.detail {
            TraceDetail::Q1 { contents, .. } => assert_eq!(*contents, Some(-2)),
            _ => panic!("q1 detail"),
        }
        assert_eq!(
            collision.geometry_coverage(0),
            GeometryCoverage {
                arbitrary_shapes: ArbitraryShapeSource::DrawingBspCells,
                clip_only: ClipOnlySource::DerivedNativeClipspace,
            }
        );
    }

    #[test]
    fn q1_queries_match_donor() {
        let collision = Q1Collision::new(geometry());
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
        assert_eq!(contents(vec3(16.0, 0.0, 0.0)), PointContentsResult::Q1 { contents: -2 });
        assert_eq!(
            contents(vec3(-16.0, 0.0, 0.0)),
            PointContentsResult::Q1 { contents: -1 }
        );
        assert_eq!(collision.point_leaf(vec3(-50.0, 0.0, 0.0)).expect("leaf"), 1);
        assert_eq!(collision.point_leaf(vec3(50.0, 0.0, 0.0)).expect("leaf"), 0);
        assert_eq!(Q1Collision::leaf_cluster(0), -1);
        assert_eq!(Q1Collision::leaf_cluster(1), 0);
        assert_eq!(collision.leaf_area(1), 0);
        assert_eq!(collision.area_bits(0), vec![1]);
        assert!(collision.areas_connected(0, 0));
        assert!(!collision.areas_connected(0, 1));
        let leaves = collision
            .box_leaves(
                &Bounds {
                    min: vec3(-70.0, -70.0, -70.0),
                    max: vec3(70.0, 70.0, 70.0),
                },
                16,
            )
            .expect("leaves");
        assert_eq!(leaves.leaves, vec![1]);
        assert_eq!(leaves.topnode, Some(0));
        assert!(!leaves.overflow);
        assert!(collision.cluster_visible(0, 0, VisibilityKind::Pvs).expect("pvs"));
        assert!(collision.cluster_visible(0, 0, VisibilityKind::Phs).expect("phs"));
        assert!(!collision.cluster_visible(0, -1, VisibilityKind::Pvs).expect("pvs"));
        assert!(collision.cluster_visible(-1, 0, VisibilityKind::Pvs).expect("pvs"));
        let tasks: &dyn SceneQueries = &collision;
        assert!(tasks.areas_connected(0, 0));
    }

    #[test]
    fn q1_errors_match_donor() {
        let collision = Q1Collision::new(geometry());
        assert_eq!(
            collision.model_bounds(5).expect_err("bad model").to_string(),
            "Unknown Quake model 5"
        );
        let bad_model = TraceQuery {
            target: QueryTarget::Model {
                model: 5,
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            },
            ..query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), SceneShape::Point)
        };
        assert_eq!(
            collision.trace(&bad_model).expect_err("bad model").to_string(),
            "Unknown Quake model 5"
        );
        let bad_hull = TraceQuery {
            policy: TracePolicy::Q1 {
                move_rule: Q1MoveRule::Normal,
                hull: Some(9),
            },
            ..query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), SceneShape::Point)
        };
        assert_eq!(
            collision.trace(&bad_hull).expect_err("bad hull").to_string(),
            "Unsupported Quake native hull 9"
        );
        assert_eq!(
            collision
                .cluster_visible(5, 0, VisibilityKind::Pvs)
                .expect_err("bad leaf")
                .to_string(),
            "Quake PVS leaf:0: index 6 outside 2 records"
        );
        let bad_face = TraceQuery {
            target: QueryTarget::Model {
                model: 1,
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            },
            ..query(vec3(-50.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), SceneShape::Point)
        };
        assert_eq!(
            collision.trace(&bad_face).expect_err("bad face").to_string(),
            "Missing Quake face plane"
        );
    }
}
