//! Quake III scene-collision adapter over the ported runtime.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/index.ts`.
//!
//! The adapter consumes the world's collision-subset geometry
//! ([`Q3CollisionGeometry`]) instead of the full scene contract: navigation
//! geometries omit the BSP records collision needs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::math::{Bounds, Vec3};
use qa_world::collision::q3::{
    decoded_collision_map, CollisionBoxModel, CollisionCounters, CollisionMapResource, CollisionWorld,
    HunkAccountingProfile, Q3CollisionGeometry, SourceTraceResult,
};
use qa_world::collision::TraceMedia;
use qa_world::spatial::CollisionFamily;
use qa_world::WorldError;

use crate::collision_support::{geometry_mask, select_numeric};
use crate::scene::{
    BspPlane, LeafQueryResult, PointContentsQuery, PointContentsResult, QueryTarget, SceneQueries, TraceContact,
    TraceDetail, TraceHit, TracePolicy, TraceQuery, TraceResult, VisibilityKind,
};
use qa_world::collision::q3::{TraceQuery as RuntimeTraceQuery, TraceShape as RuntimeTraceShape};

/// Scene adapter over a Quake III collision world.
pub struct Q3Collision {
    world: CollisionWorld,
    phs: RefCell<HashMap<i32, Vec<u8>>>,
}

impl Q3Collision {
    /// Decode collision geometry into an adapter.
    pub fn new(geometry: &Q3CollisionGeometry) -> Result<Self, WorldError> {
        let map = decoded_collision_map(geometry, None)?;
        Ok(Self::from_world(CollisionWorld::new(
            Rc::new(map),
            qa_world::collision::q3::CollisionWorldProfile::Disabled,
            Rc::new(CollisionCounters::new()),
        )))
    }

    /// Wrap an existing runtime world (donor union constructor).
    #[must_use]
    pub fn from_world(world: CollisionWorld) -> Self {
        Self {
            world,
            phs: RefCell::new(HashMap::new()),
        }
    }

    /// Borrow the runtime world.
    #[must_use]
    pub fn world(&self) -> &CollisionWorld {
        &self.world
    }

    /// Leaf containing a point.
    pub fn point_leaf(&self, point: Vec3) -> Result<i32, WorldError> {
        Ok(self.world.point_leafnum(point)? as i32)
    }

    /// Leaf area.
    pub fn leaf_area(&self, index: i32) -> Result<i32, WorldError> {
        self.world.leaf_area(index)
    }

    /// Leaf cluster.
    pub fn leaf_cluster(&self, index: i32) -> Result<i32, WorldError> {
        self.world.leaf_cluster(index)
    }

    /// Model bounds.
    pub fn model_bounds(&self, index: i32) -> Result<Bounds, WorldError> {
        self.world.model_bounds(index)
    }

    /// Leaves touched by bounds.
    pub fn box_leaves(&self, bounds: &Bounds, limit: usize) -> Result<LeafQueryResult, WorldError> {
        let result = self.world.box_leafnums(bounds, limit)?;
        Ok(LeafQueryResult {
            leaves: result.leaves.into_iter().map(|leaf| leaf as i32).collect(),
            topnode: result.topnode.map(|node| node as i32),
            overflow: result.overflowed,
        })
    }

    /// Area connectivity.
    pub fn areas_connected(&self, first: i32, second: i32) -> Result<bool, WorldError> {
        self.world.areas_connected(first, second)
    }

    /// Area visibility bits.
    pub fn area_bits(&self, area: i32) -> Result<Vec<u8>, WorldError> {
        self.world.area_bits(area)
    }

    /// Cluster visibility through the PVS, or the cached PHS closure.
    pub fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> Result<bool, WorldError> {
        if kind == VisibilityKind::Pvs {
            return self.world.cluster_visible(from, to);
        }
        if to < 0 || to >= self.world.cluster_count() {
            return Ok(false);
        }
        if !self.phs.borrow().contains_key(&from) {
            let mut row = vec![0u8; ((self.world.cluster_count() + 7) >> 3) as usize];
            let source = self.world.cluster_pvs(from);
            for (index, byte) in row.iter_mut().enumerate() {
                *byte = source.byte_at(index as i64)?;
            }
            for cluster in 0..self.world.cluster_count() {
                if !self.world.cluster_visible(from, cluster)? {
                    continue;
                }
                let adjacent = self.world.cluster_pvs(cluster);
                for (index, byte) in row.iter_mut().enumerate() {
                    *byte |= adjacent.byte_at(index as i64)?;
                }
            }
            self.phs.borrow_mut().insert(from, row);
        }
        let rows = self.phs.borrow();
        let row = rows.get(&from).expect("cached phs row");
        Ok(row[(to >> 3) as usize] & (1 << (to & 7)) != 0)
    }

    /// Adjust area portal state.
    pub fn adjust_area_portal_state(&self, first: i32, second: i32, open: bool) -> Result<(), WorldError> {
        self.world.adjust_area_portal_state(first, second, open)
    }

    /// Sample contents at a point.
    pub fn point_contents(&self, query: &PointContentsQuery) -> Result<PointContentsResult, WorldError> {
        select_numeric(&query.numeric)?;
        let contents = match &query.target {
            QueryTarget::World => self.world.point_contents(query.point, 0)?,
            QueryTarget::Model { model, origin, angles } => {
                self.world
                    .transformed_point_contents(query.point, *model, *origin, *angles)?
            }
        };
        Ok(PointContentsResult::Q3 { contents })
    }

    /// Sweep a body through the world.
    pub fn trace(&self, query: &TraceQuery) -> Result<TraceResult, WorldError> {
        select_numeric(&query.numeric)?;
        let model = match &query.target {
            QueryTarget::World => 0,
            QueryTarget::Model { model, .. } => *model,
        };
        let local = self.local_query(query, model);
        let result = match &query.target {
            QueryTarget::World => self.world.trace_source(&local)?,
            QueryTarget::Model { origin, angles, .. } => {
                self.world.transformed_trace_source(&local, *origin, *angles)?
            }
        };
        Ok(scene_result(&result, model))
    }

    /// Classify the media along a sweep.
    pub fn trace_media(&self, query: &TraceQuery, fraction: f64) -> Result<TraceMedia, WorldError> {
        let model = match &query.target {
            QueryTarget::World => 0,
            QueryTarget::Model { model, .. } => *model,
        };
        let local = RuntimeTraceQuery {
            start: query.start,
            end: query.end,
            mask: geometry_mask(&query.policy, CollisionFamily::Q3),
            model_index: Some(model),
            shape: runtime_shape(query),
            curves: None,
            player_curve_clip: None,
        };
        let target = match &query.target {
            QueryTarget::World => None,
            QueryTarget::Model { origin, angles, .. } => Some(qa_world::collision::q3::ModelTransform {
                origin: *origin,
                angles: *angles,
            }),
        };
        self.world.trace_media(&local, fraction, target.as_ref())
    }

    fn local_query(&self, query: &TraceQuery, model: i32) -> RuntimeTraceQuery {
        let _ = self;
        let clip = !matches!(query.policy, TracePolicy::Q3 { .. });
        let (curves, player_curve_clip) = match &query.policy {
            TracePolicy::Q3 {
                curves,
                player_curve_clip,
                ..
            } => (Some(*curves), Some(*player_curve_clip)),
            _ => (Some(clip), Some(clip)),
        };
        RuntimeTraceQuery {
            start: query.start,
            end: query.end,
            mask: geometry_mask(&query.policy, CollisionFamily::Q3),
            model_index: Some(model),
            shape: runtime_shape(query),
            curves,
            player_curve_clip,
        }
    }
}

fn runtime_shape(query: &TraceQuery) -> RuntimeTraceShape {
    match &query.shape {
        crate::scene::TraceShape::Point => RuntimeTraceShape::Point,
        crate::scene::TraceShape::Box { bounds } => RuntimeTraceShape::Box {
            mins: bounds.min,
            maxs: bounds.max,
        },
        crate::scene::TraceShape::Capsule { bounds } => RuntimeTraceShape::Capsule {
            mins: bounds.min,
            maxs: bounds.max,
        },
    }
}

fn scene_result(result: &SourceTraceResult, model: i32) -> TraceResult {
    TraceResult {
        fraction: f64::from(result.fraction),
        end: result.end,
        start_solid: result.start_solid,
        all_solid: result.all_solid,
        contact: if result.fraction == 1.0 || result.start_solid {
            TraceContact::None
        } else {
            TraceContact::Plane {
                plane: qa_core::math::Plane {
                    normal: result.plane.normal,
                    distance: result.plane.distance,
                },
            }
        },
        hit: if result.fraction < 1.0 || result.start_solid {
            TraceHit::World { model }
        } else {
            TraceHit::None
        },
        detail: TraceDetail::Q3 {
            contents: result.contents,
            surface_flags: result.surface_flags,
            source_plane: BspPlane {
                normal: result.plane.normal,
                distance: result.plane.distance,
                plane_type: result.plane.plane_type,
                signbits: result.plane.signbits,
            },
        },
    }
}

impl SceneQueries for Q3Collision {
    fn trace(&self, query: &TraceQuery) -> TraceResult {
        self.trace(query).expect("q3 scene trace")
    }

    fn point_contents(&self, query: &PointContentsQuery) -> PointContentsResult {
        self.point_contents(query).expect("q3 scene contents")
    }

    fn box_leaves(&self, bounds: &Bounds, limit: usize) -> LeafQueryResult {
        self.box_leaves(bounds, limit).expect("q3 scene leaves")
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        self.areas_connected(first, second).expect("q3 scene areas")
    }

    fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> bool {
        self.cluster_visible(from, to, kind).expect("q3 scene visibility")
    }
}

/// Decode collision geometry into an adapter.
pub fn create_q3_collision(geometry: &Q3CollisionGeometry) -> Result<Q3Collision, WorldError> {
    Q3Collision::new(geometry)
}

/// Load raw clip-map bytes into an adapter.
pub fn create_source_q3_collision(bytes: &[u8], source: &str) -> Result<Q3Collision, WorldError> {
    let mut resource = CollisionMapResource::new(
        source,
        HunkAccountingProfile::Unaccounted,
        None,
        CollisionBoxModel::new(),
    );
    resource.load(bytes)?;
    resource.initialize_box_hull()?;
    let map = resource.into_data()?;
    Ok(Q3Collision::from_world(CollisionWorld::new(
        Rc::new(map),
        qa_world::collision::q3::CollisionWorldProfile::Disabled,
        Rc::new(CollisionCounters::new()),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{vec3, Plane};
    use qa_core::numeric::Q3_BINARY32_PROFILE;

    use crate::scene::TracePolicy as ScenePolicy;
    use qa_world::collision::q3::{
        CollisionShader, IndexRange, Q3BspChild, Q3CollisionBrushInput, Q3CollisionBrushSideInput,
        Q3CollisionLeafInput, Q3CollisionModelInput, Q3CollisionNodeInput,
    };

    fn geometry() -> Q3CollisionGeometry {
        let mut planes = vec![Plane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 0.0,
        }];
        for (normal, distance) in [
            (vec3(1.0, 0.0, 0.0), 64.0),
            (vec3(-1.0, 0.0, 0.0), 64.0),
            (vec3(0.0, 1.0, 0.0), 64.0),
            (vec3(0.0, -1.0, 0.0), 64.0),
            (vec3(0.0, 0.0, 1.0), 64.0),
            (vec3(0.0, 0.0, -1.0), 64.0),
        ] {
            planes.push(Plane { normal, distance });
        }
        Q3CollisionGeometry {
            entities: String::new(),
            shaders: vec![CollisionShader {
                name: "solid".to_string(),
                surface_flags: 1,
                content_flags: 1,
            }],
            planes,
            nodes: vec![Q3CollisionNodeInput {
                plane: 0,
                children: [Q3BspChild::Leaf(0), Q3BspChild::Leaf(1)],
            }],
            leaves: vec![
                Q3CollisionLeafInput {
                    cluster: 0,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 1 },
                    surfaces: IndexRange { first: 0, count: 0 },
                },
                Q3CollisionLeafInput {
                    cluster: 1,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 0 },
                    surfaces: IndexRange { first: 0, count: 0 },
                },
            ],
            leaf_brushes: vec![0],
            leaf_surfaces: Vec::new(),
            models: vec![Q3CollisionModelInput {
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                brushes: IndexRange { first: 0, count: 0 },
                surfaces: IndexRange { first: 0, count: 0 },
            }],
            brushes: vec![Q3CollisionBrushInput {
                shader: 0,
                sides: IndexRange { first: 0, count: 6 },
            }],
            brush_sides: (1..7)
                .map(|plane| Q3CollisionBrushSideInput { plane, shader: 0 })
                .collect(),
            vertices: Vec::new(),
            surfaces: Vec::new(),
            visibility: None,
        }
    }

    fn policy() -> ScenePolicy {
        ScenePolicy::Q3 {
            contents_mask: 1,
            curves: true,
            player_curve_clip: true,
        }
    }

    fn query() -> TraceQuery {
        TraceQuery {
            start: vec3(0.0, 0.0, 100.0),
            end: vec3(0.0, 0.0, -100.0),
            shape: crate::scene::TraceShape::Point,
            target: QueryTarget::World,
            policy: policy(),
            numeric: Q3_BINARY32_PROFILE,
            pass_actor: None,
        }
    }

    #[test]
    fn q3_adapter_matches_donor() {
        let adapter = create_q3_collision(&geometry()).expect("adapter");
        let hit = adapter.trace(&query()).expect("trace");
        assert_eq!(hit.fraction, f64::from(0.17937499284744263f64 as f32));
        assert_eq!(hit.end, vec3(0.0, 0.0, 64.125));
        assert_eq!(hit.hit, TraceHit::World { model: 0 });
        match &hit.detail {
            TraceDetail::Q3 {
                contents,
                surface_flags,
                ..
            } => {
                assert_eq!(*contents, 1);
                assert_eq!(*surface_flags, 1);
            }
            _ => panic!("q3 detail"),
        }
        let contents = adapter
            .point_contents(&PointContentsQuery {
                point: vec3(0.0, 0.0, 0.0),
                target: QueryTarget::World,
                policy: policy(),
                numeric: Q3_BINARY32_PROFILE,
                pass_actor: None,
            })
            .expect("contents");
        assert_eq!(contents, PointContentsResult::Q3 { contents: 1 });
        let leaves = adapter
            .box_leaves(
                &Bounds {
                    min: vec3(-70.0, -70.0, -70.0),
                    max: vec3(70.0, 70.0, 70.0),
                },
                16,
            )
            .expect("leaves");
        assert_eq!(leaves.leaves, vec![0, 1]);
        assert!(!leaves.overflow);
        assert_eq!(adapter.point_leaf(vec3(0.0, 0.0, 100.0)).expect("leaf"), 0);
        assert_eq!(adapter.leaf_cluster(0).expect("cluster"), 0);
        // SceneQueries trait wiring stays infallible for valid queries.
        let tasks: &dyn SceneQueries = &adapter;
        assert_eq!(tasks.trace(&query()).fraction, hit.fraction);
        let error = adapter.leaf_cluster(9).expect_err("unknown leaf must fail");
        assert_eq!(error.to_string(), "CM_LeafCluster: bad number");
    }
}
