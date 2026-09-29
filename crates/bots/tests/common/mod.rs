//! Shared synthetic fixtures for navigation integration tests.
#![allow(dead_code)]

use qa_bots::content::ContentDigest;
use qa_bots::movement_contract::{MovementKind, MovementProfile};
use qa_bots::scene::{
    BodyShape, BspEdge, BspFace, BspPlane, DecodedWorld, IndexRange, LeafQueryResult, PointContentsQuery,
    PointContentsResult, Q1WorldGeometry, Q1WorldModel, Q2Leaf, Q2WorldGeometry, Q2WorldModel, Q3BspLeaf, Q3BspModel,
    Q3BspSurface, Q3BspVertex, Q3SurfaceKind, Q3WorldGeometry, SceneQueries, TraceContact, TraceDetail, TraceHit,
    TracePolicy, TraceQuery, TraceResult, TraceShape, VisibilityKind, WorldKind,
};
use qa_bots::types::{
    NavigationAsset, NavigationEntityBinding, NavigationEntityState, NavigationGraph, NavigationMapIdentity,
    NavigationProfile, NavigationRoutePrediction, NavigationWorld, TravelMode, TraversalAdmission, TraversalRequest,
};
use qa_bots::{aas::*, graph};
use qa_core::identity::{ActorId, IdentityOwner, ProviderId};
use qa_core::math::{vec3, Bounds, Plane, Vec3};
use qa_core::numeric::Q3_BINARY32_PROFILE;

pub fn test_identity() -> IdentityOwner {
    IdentityOwner::create("test").unwrap()
}

pub fn test_actor(owner: &IdentityOwner) -> ActorId {
    owner.actor(1, 1)
}

pub fn test_movement_profile() -> MovementProfile {
    MovementProfile {
        kind: MovementKind::Q3,
        id: ProviderId::new("test", "movement"),
        numeric: Q3_BINARY32_PROFILE,
    }
}

pub fn test_body() -> BodyShape {
    BodyShape::Box(Bounds {
        min: vec3(-16.0, -16.0, -24.0),
        max: vec3(16.0, 16.0, 32.0),
    })
}

pub fn test_profile() -> NavigationProfile {
    NavigationProfile {
        movement: test_movement_profile(),
        shape: test_body(),
        crouched_shape: Some(BodyShape::Box(Bounds {
            min: vec3(-16.0, -16.0, -24.0),
            max: vec3(16.0, 16.0, 16.0),
        })),
        policy: TracePolicy::Q3 {
            contents_mask: -1,
            curves: true,
            player_curve_clip: true,
        },
        capabilities: [
            TravelMode::Walk,
            TravelMode::Crouch,
            TravelMode::Jump,
            TravelMode::Drop,
            TravelMode::Swim,
            TravelMode::WaterJump,
            TravelMode::Ladder,
            TravelMode::Teleport,
            TravelMode::Mover,
            TravelMode::JumpPad,
            TravelMode::RocketJump,
            TravelMode::BfgJump,
            TravelMode::Grapple,
            TravelMode::DoubleJump,
            TravelMode::RampJump,
            TravelMode::StrafeJump,
        ]
        .into_iter()
        .collect(),
        maximum_step: 18.0,
        minimum_floor_normal: 0.7,
        maximum_drop: 64.0,
        team: None,
        monster: false,
    }
}

pub fn test_map() -> NavigationMapIdentity {
    NavigationMapIdentity {
        name: "test".to_string(),
        format: WorldKind::Q3Bsp,
        digest: ContentDigest::new("test"),
    }
}

/// Open scene with a solid floor plane at z = 0.
#[derive(Debug, Clone, Copy)]
pub struct FloorScene {
    pub floor_z: f32,
}

impl SceneQueries for FloorScene {
    fn trace(&self, query: &TraceQuery) -> TraceResult {
        // Sweep the body bottom against the floor plane, like a real box
        // trace: contact is reported at the origin whose bottom rests on
        // the plane, with the rest height exact so resting bodies are not
        // stuck by rounding.
        let offset = match query.shape {
            TraceShape::Point => 0.0,
            TraceShape::Box { bounds } | TraceShape::Capsule { bounds } => f64::from(bounds.min.z),
        };
        let start_z = f64::from(query.start.z) + offset;
        let end_z = f64::from(query.end.z) + offset;
        let floor = f64::from(self.floor_z);
        let rest_z = (floor - offset) as f32;
        let (fraction, end, start_solid, all_solid, contact) = if start_z < floor && end_z < floor {
            (0.0, query.start, true, true, TraceContact::None)
        } else if start_z < floor {
            (0.0, query.start, true, false, TraceContact::None)
        } else if end_z < floor {
            let fraction = (start_z - floor) / (start_z - end_z);
            (
                fraction,
                Vec3 {
                    x: (f64::from(query.start.x) + (f64::from(query.end.x) - f64::from(query.start.x)) * fraction)
                        as f32,
                    y: (f64::from(query.start.y) + (f64::from(query.end.y) - f64::from(query.start.y)) * fraction)
                        as f32,
                    z: rest_z,
                },
                false,
                false,
                TraceContact::Plane {
                    plane: Plane {
                        normal: vec3(0.0, 0.0, 1.0),
                        distance: self.floor_z,
                    },
                },
            )
        } else {
            (1.0, query.end, false, false, TraceContact::None)
        };
        TraceResult {
            fraction,
            end,
            start_solid,
            all_solid,
            contact,
            hit: TraceHit::None,
            detail: TraceDetail::Q3 {
                contents: 0,
                surface_flags: 0,
                source_plane: BspPlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                    plane_type: 2,
                    signbits: 0,
                },
            },
        }
    }

    fn point_contents(&self, query: &PointContentsQuery) -> PointContentsResult {
        let _ = query;
        PointContentsResult::Q3 { contents: 0 }
    }

    fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> LeafQueryResult {
        LeafQueryResult {
            leaves: Vec::new(),
            topnode: None,
            overflow: false,
        }
    }

    fn areas_connected(&self, _first: i32, _second: i32) -> bool {
        true
    }

    fn cluster_visible(&self, _from: i32, _to: i32, _kind: VisibilityKind) -> bool {
        true
    }
}

/// Prediction session that admits straight-line walks.
pub struct AllAdmitPrediction;

impl NavigationRoutePrediction for AllAdmitPrediction {
    fn admit(&mut self, request: &TraversalRequest) -> Result<TraversalAdmission, qa_bots::BotsError> {
        let dx = f64::from(request.to.x) - f64::from(request.from.x);
        let dy = f64::from(request.to.y) - f64::from(request.from.y);
        let dz = f64::from(request.to.z) - f64::from(request.from.z);
        Ok(TraversalAdmission::Admitted {
            seconds: (dx.hypot(dy).hypot(dz) / 320.0).max(0.01),
            trajectory: vec![request.from, request.to],
        })
    }
}

/// World over [`FloorScene`]; admits every traversal.
pub struct FixtureWorld {
    pub scene: FloorScene,
    pub revision: i64,
}

impl FixtureWorld {
    pub fn new() -> Self {
        Self {
            scene: FloorScene { floor_z: 0.0 },
            revision: 0,
        }
    }
}

impl Default for FixtureWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl NavigationWorld for FixtureWorld {
    fn scene(&self) -> &dyn SceneQueries {
        &self.scene
    }

    fn pass_actor(&self) -> Option<ActorId> {
        None
    }

    fn revision(&self) -> i64 {
        self.revision
    }

    fn admit(&self, request: &TraversalRequest, _profile: &NavigationProfile) -> TraversalAdmission {
        AllAdmitPrediction.admit(request).unwrap()
    }

    fn begin_route(&self, _profile: &NavigationProfile) -> Box<dyn NavigationRoutePrediction> {
        Box::new(AllAdmitPrediction)
    }

    fn entity(&self, _binding: &NavigationEntityBinding) -> Option<NavigationEntityState> {
        None
    }

    fn hazard(&self, _bounds: &Bounds) -> bool {
        false
    }
}

/// Minimal two-area AAS asset without reachability. Areas 1 (north) and
/// 2 (south) share ground edge 1 along y = 0.
pub fn bare_aas() -> AasAsset {
    let bounds = |min: Vec3, max: Vec3| Bounds { min, max };
    AasAsset {
        source: "fixture".to_string(),
        version: 5,
        bsp_checksum: 1234,
        lumps: Vec::new(),
        bboxes: vec![AasBbox {
            presence: 3,
            flags: 0,
            bounds: bounds(vec3(-16.0, -16.0, -24.0), vec3(16.0, 16.0, 32.0)),
        }],
        vertices: vec![
            vec3(0.0, 0.0, 0.0),
            vec3(64.0, 0.0, 0.0),
            vec3(64.0, 64.0, 0.0),
            vec3(0.0, 64.0, 0.0),
            vec3(0.0, -64.0, 0.0),
            vec3(64.0, -64.0, 0.0),
        ],
        planes: vec![
            AasPlane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
                plane_type: 2,
            },
            AasPlane {
                normal: vec3(0.0, 1.0, 0.0),
                distance: 0.0,
                plane_type: 1,
            },
        ],
        edges: vec![
            AasEdge { vertices: [0, 0] },
            AasEdge { vertices: [0, 1] },
            AasEdge { vertices: [1, 2] },
            AasEdge { vertices: [2, 3] },
            AasEdge { vertices: [3, 0] },
            AasEdge { vertices: [4, 5] },
            AasEdge { vertices: [5, 1] },
            AasEdge { vertices: [0, 4] },
        ],
        edge_indexes: vec![1, 2, 3, 4, -1, 6, 5, 7],
        faces: vec![
            AasFace {
                plane: 0,
                flags: 0,
                edge_count: 0,
                first_edge: 0,
                front_area: 0,
                back_area: 0,
            },
            AasFace {
                plane: 0,
                flags: 4,
                edge_count: 4,
                first_edge: 0,
                front_area: 1,
                back_area: 0,
            },
            AasFace {
                plane: 0,
                flags: 4,
                edge_count: 4,
                first_edge: 4,
                front_area: 2,
                back_area: 0,
            },
        ],
        face_indexes: vec![1, 2],
        areas: vec![
            AasArea {
                number: 0,
                face_count: 0,
                first_face: 0,
                bounds: bounds(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)),
                center: vec3(0.0, 0.0, 0.0),
            },
            AasArea {
                number: 1,
                face_count: 1,
                first_face: 0,
                bounds: bounds(vec3(0.0, 0.0, 0.0), vec3(64.0, 64.0, 64.0)),
                center: vec3(32.0, 32.0, 32.0),
            },
            AasArea {
                number: 2,
                face_count: 1,
                first_face: 1,
                bounds: bounds(vec3(0.0, -64.0, 0.0), vec3(64.0, 0.0, 64.0)),
                center: vec3(32.0, -32.0, 32.0),
            },
        ],
        settings: vec![
            AasAreaSettings {
                contents: 0,
                flags: 0,
                presence: 0,
                cluster: 0,
                cluster_area: 0,
                reach_count: 0,
                first_reach: 0,
            },
            AasAreaSettings {
                contents: 0,
                flags: 1,
                presence: 7,
                cluster: 1,
                cluster_area: 0,
                reach_count: 0,
                first_reach: 1,
            },
            AasAreaSettings {
                contents: 0,
                flags: 1,
                presence: 7,
                cluster: 1,
                cluster_area: 1,
                reach_count: 0,
                first_reach: 1,
            },
        ],
        reachability: Vec::new(),
        nodes: vec![
            AasNode {
                plane: 0,
                children: [0, 0],
            },
            AasNode {
                plane: 1,
                children: [-1, -2],
            },
        ],
        portals: Vec::new(),
        portal_indexes: Vec::new(),
        clusters: vec![
            AasCluster {
                area_count: 0,
                reachability_area_count: 0,
                portal_count: 0,
                first_portal: 0,
            },
            AasCluster {
                area_count: 2,
                reachability_area_count: 2,
                portal_count: 0,
                first_portal: 0,
            },
        ],
    }
}

/// [`bare_aas`] plus a zero record and a walk link in each direction.
pub fn linked_aas() -> AasAsset {
    let mut asset = bare_aas();
    asset.reachability = vec![
        AasReachability {
            area: 0,
            face: 0,
            edge: 0,
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(0.0, 0.0, 0.0),
            travel_type: 0,
            travel_time: 0,
            padding: 0,
        },
        AasReachability {
            area: 2,
            face: 0,
            edge: 1,
            start: vec3(32.0, 4.0, 1.0),
            end: vec3(32.0, -4.0, 1.0),
            travel_type: 2,
            travel_time: 100,
            padding: 0,
        },
        AasReachability {
            area: 1,
            face: 0,
            edge: -1,
            start: vec3(32.0, -4.0, 1.0),
            end: vec3(32.0, 4.0, 1.0),
            travel_type: 2,
            travel_time: 100,
            padding: 0,
        },
    ];
    asset.settings[1].first_reach = 1;
    asset.settings[1].reach_count = 1;
    asset.settings[2].first_reach = 2;
    asset.settings[2].reach_count = 1;
    asset
}

pub fn linked_graph(world: &FixtureWorld) -> NavigationGraph {
    graph::navigation_from_asset(
        test_map(),
        NavigationAsset::Aas(Box::new(linked_aas())),
        test_profile(),
        world,
    )
    .unwrap()
}

pub fn q1_world() -> DecodedWorld {
    DecodedWorld::Q1(Q1WorldGeometry {
        entities: String::new(),
        planes: vec![BspPlane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 0.0,
            plane_type: 2,
            signbits: 0,
        }],
        vertices: vec![
            vec3(0.0, 0.0, 0.0),
            vec3(128.0, 0.0, 0.0),
            vec3(128.0, 128.0, 0.0),
            vec3(0.0, 128.0, 0.0),
        ],
        edges: vec![
            BspEdge { vertices: [0, 1] },
            BspEdge { vertices: [1, 2] },
            BspEdge { vertices: [2, 3] },
            BspEdge { vertices: [3, 0] },
        ],
        surface_edges: vec![0, 1, 2, 3],
        leaves: Vec::new(),
        faces: vec![BspFace {
            plane: 0,
            back: false,
            edges: IndexRange { first: 0, count: 4 },
        }],
        models: vec![Q1WorldModel {
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(128.0, 128.0, 64.0),
            },
            faces: IndexRange { first: 0, count: 1 },
        }],
    })
}

pub fn q2_world() -> DecodedWorld {
    DecodedWorld::Q2(Q2WorldGeometry {
        entities: String::new(),
        planes: vec![BspPlane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 0.0,
            plane_type: 2,
            signbits: 0,
        }],
        vertices: vec![vec3(0.0, 0.0, 0.0), vec3(128.0, 0.0, 0.0), vec3(128.0, 128.0, 0.0)],
        edges: vec![
            BspEdge { vertices: [0, 1] },
            BspEdge { vertices: [1, 2] },
            BspEdge { vertices: [2, 0] },
        ],
        surface_edges: vec![0, 1, 2],
        leaves: vec![Q2Leaf {
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(128.0, 128.0, 64.0),
            },
        }],
        faces: vec![BspFace {
            plane: 0,
            back: false,
            edges: IndexRange { first: 0, count: 3 },
        }],
        models: vec![Q2WorldModel {
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(128.0, 128.0, 64.0),
            },
            faces: IndexRange { first: 0, count: 1 },
        }],
    })
}

pub fn q3_world() -> DecodedWorld {
    DecodedWorld::Q3(Q3WorldGeometry {
        entities: String::new(),
        models: vec![Q3BspModel {
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(128.0, 128.0, 64.0),
            },
            surfaces: IndexRange { first: 0, count: 1 },
        }],
        vertices: vec![
            Q3BspVertex {
                position: vec3(0.0, 0.0, 0.0),
                normal: vec3(0.0, 0.0, 1.0),
            },
            Q3BspVertex {
                position: vec3(128.0, 0.0, 0.0),
                normal: vec3(0.0, 0.0, 1.0),
            },
            Q3BspVertex {
                position: vec3(128.0, 128.0, 0.0),
                normal: vec3(0.0, 0.0, 1.0),
            },
            Q3BspVertex {
                position: vec3(0.0, 128.0, 0.0),
                normal: vec3(0.0, 0.0, 1.0),
            },
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        surfaces: vec![Q3BspSurface {
            kind: Q3SurfaceKind::Planar,
            vertices: IndexRange { first: 0, count: 4 },
            indices: IndexRange { first: 0, count: 6 },
            width: 0,
            height: 0,
        }],
        leaves: vec![Q3BspLeaf {
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(128.0, 128.0, 64.0),
            },
        }],
    })
}
