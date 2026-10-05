//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/monster-navigation.ts`.
//!
//! MG3 monster navigation: preload mounted navigation assets before the
//! synchronous source constructor, then install a per-actor
//! [`NavigationRuntime`](qa_bots::NavigationRuntime) supplier into the Q1
//! source world (`registerMg3MonsterNavigation`).
//!
//! Donor behavior preserved:
//!
//! * [`preload_monster_navigation`] returns `None` for non-MG3 entities
//!   (donor `content.split(":")[2] !== "mg3"` gate).
//! * [`ApplicationMonsterNavigation::install`] panics with the donor
//!   messages when the Q1 source world is absent (`MG3 navigation requires
//!   the Q1 source world`) or the selected movement is not Q1
//!   (`MG3 monsters require Q1 source movement`).
//! * Graphs load once per monster shape key and are shared across actors;
//!   each actor gets a world whose revision tracks simulation time.
//! * Route prediction walks a detached monster copy through the live Q1
//!   movement provider (`walkMove` stepping with the donor constants).
//!
//! Rust adaptations (all donor-equivalent):
//!
//! * The donor caches one `NavigationRuntime` per actor in a `WeakMap`.
//!   A runtime borrows its world, so a self-referential cache is
//!   unrepresentable; this port caches the expensive graph per shape key
//!   and the cheap world per actor, and builds a fresh runtime per
//!   `for_actor` call. Fresh runtimes are equivalent here because the MG3
//!   host never blocks nodes or edges (blocking state is the only
//!   per-runtime memory beyond the graph).
//! * The scene adapters answer empty-world collision until the collision
//!   lane wires [`SharedSimulation`](crate::bootstrap::simulation::runtime::SharedSimulation)
//!   scene queries through. This is the same collision-lane seam as the
//!   player-movement empty-world traces.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::rc::Rc;

use qa_bots::scene::{
    BodyShape, DecodedWorld, LeafQueryResult, PointContentsQuery, PointContentsResult, Q1MoveRule, QueryTarget,
    SceneQueries, TraceContact, TraceDetail, TraceHit, TracePolicy, TraceQuery, TraceResult, VisibilityKind, WorldKind,
};
use qa_bots::NavigationResources;
use qa_bots::{
    load_prepared_navigation, preload_navigation, ContentId, MovementKind, MovementProfile, NavigationConstruction,
    NavigationEdge, NavigationGraph, NavigationMapIdentity, NavigationProfile, NavigationRoute,
    NavigationRoutePrediction, NavigationRouteQuery, NavigationRouteResult, NavigationRuntime, NavigationWorld,
    OpenedResource, PreloadOptions, PreparedNavigation, ResourceReference, TravelMode, TraversalAdmission,
    TraversalRequest,
};
use qa_content::contract::{ContentId as ContentIdentity, ResolvedResourceReference, ResourceIdentity};
use qa_content::mounts::MountedContent;
use qa_content::q1::addons::monsters::ai::path::{
    register_mg3_monster_navigation, Mg3MonsterNavigationHost, Mg3NavigationEdge, Mg3NavigationRoute,
    Mg3NavigationRuntime, Mg3RouteRequest, Mg3TravelMode,
};
use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Plane, Vec3};
use qa_core::numeric::NumericOps;
use qa_world::movement::q1::monsters::{
    create_q1_monster_movement, Q1MonsterMoveServices, Q1MonsterMoveState, Q1MonsterMovement, Q1MonsterTarget,
};
use qa_world::movement::q1::types::Q1Trace;

use super::player_input_application::MovementProfile as AppMovementProfile;
use crate::bootstrap::simulation::runtime::{movement_profile, SharedSimulation};

/// Zero vector for the model-zero topology target.
const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

/// Preload inputs for MG3 monster navigation (donor
/// `preloadApplicationMonsterNavigation(content)` arguments, restricted to
/// the fields the donor reads).
pub struct MonsterNavigationPreload {
    /// Entities provider content markers (`content.recipe.map.entities.content`).
    pub entities_content: String,
    /// Selected map geometry resource.
    pub geometry: ResolvedResourceReference,
    /// Decoded world BSP family (`content.world.kind`).
    pub world_kind: WorldKind,
    /// Decoded map geometry (`content.world`).
    pub world: Rc<DecodedWorld>,
    /// Geometry content's mounted resources (`content.forContent(...)`).
    pub resources: MountedContent,
    /// Selected map bytes for AAS checksum verification.
    pub map_bytes: Vec<u8>,
    /// Required NAV content owner (geometry provenance mount content).
    pub navigation_content: ContentIdentity,
}

/// Installed MG3 monster navigation (donor `ApplicationMonsterNavigation`).
pub struct ApplicationMonsterNavigation {
    /// Prepared navigation asset shared across monster shape graphs.
    prepared: PreparedNavigation,
    /// Decoded map geometry for graph construction.
    geometry: Rc<DecodedWorld>,
}

/// Preload mounted MG3 navigation assets (donor
/// `preloadApplicationMonsterNavigation`). Returns `None` for non-MG3
/// entity content.
#[must_use]
pub fn preload_monster_navigation(bundle: &MonsterNavigationPreload) -> Option<ApplicationMonsterNavigation> {
    if bundle.entities_content.split(':').nth(2) != Some("mg3") {
        return None;
    }
    let resources = MonsterNavigationResources {
        mounted: &bundle.resources,
    };
    let map = NavigationMapIdentity {
        name: bundle.geometry.requested_path.clone(),
        format: bundle.world_kind,
        identity: bundle.geometry.identity,
    };
    let navigation_content = ContentId::new(&bundle.navigation_content.to_string());
    let prepared = preload_navigation(&PreloadOptions {
        map: &map,
        resources: &resources,
        map_bytes: &bundle.map_bytes,
        navigation_content: Some(&navigation_content),
    })
    .expect("preload MG3 navigation");
    Some(ApplicationMonsterNavigation {
        prepared,
        geometry: Rc::clone(&bundle.world),
    })
}

/// [`NavigationResources`] over a mounted content view.
struct MonsterNavigationResources<'m> {
    /// Mounted resources to open navigation assets from.
    mounted: &'m MountedContent,
}

impl NavigationResources for MonsterNavigationResources<'_> {
    fn open(&self, path: &str) -> Option<OpenedResource> {
        let opened = self.mounted.open(path, |_| true).ok()??;
        let mount_content = match &opened.reference.provenance {
            qa_content::contract::ResourceProvenance::Archive { mount, .. } => mount.identity.content.to_string(),
            qa_content::contract::ResourceProvenance::Loose { mount, .. } => mount.identity.content.to_string(),
        };
        Some(OpenedResource {
            reference: ResourceReference {
                requested_path: opened.reference.requested_path.clone(),
                provenance: qa_bots::ResourceProvenance {
                    mount_content: ContentId::new(&mount_content),
                },
                identity: opened.reference.identity,
                byte_length: opened.reference.byte_length as usize,
            },
            bytes: opened.bytes,
        })
    }
}

impl ApplicationMonsterNavigation {
    /// Install the per-actor navigation supplier into the Q1 source world
    /// (donor `install(simulation, movement)`).
    ///
    /// # Panics
    ///
    /// Panics with the donor messages when the Q1 source world is absent
    /// or the selected movement is not Q1 source movement, and when
    /// registration or graph loading fails (donor throws propagate the
    /// same way).
    pub fn install<S: Q1MonsterMoveServices + 'static>(
        &self,
        simulation: &SharedSimulation,
        movement: Rc<RefCell<Q1MonsterMovement<S>>>,
    ) {
        let mut recipe = simulation.recipe();
        recipe.movement = recipe.map.entities.clone();
        let app_selected = movement_profile(&recipe).expect("MG3 movement profile");
        if !matches!(
            app_selected,
            AppMovementProfile::Q1Netquake(_) | AppMovementProfile::Q1Quakeworld(_)
        ) {
            panic!("MG3 monsters require Q1 source movement");
        }
        let selected = bots_movement_profile(&app_selected);
        let registered = simulation.with_q1_source(|view| {
            let mut services = view.services.borrow_mut();
            register_mg3_monster_navigation(
                &mut services,
                Box::new(Mg3MonsterInstallation {
                    simulation: simulation.clone(),
                    movement: Rc::clone(&movement),
                    selected,
                    geometry: Rc::clone(&self.geometry),
                    prepared: self.prepared.clone(),
                    graphs: HashMap::new(),
                    actors: HashMap::new(),
                }),
            )
        });
        registered
            .expect("MG3 navigation requires the Q1 source world")
            .expect("register MG3 monster navigation");
    }
}

/// Per-actor navigation supplier installed into the Q1 source world (donor
/// `install` closure with its `graphs` and `actors` caches).
struct Mg3MonsterInstallation<S: Q1MonsterMoveServices> {
    /// Owning simulation for scene access and time.
    simulation: SharedSimulation,
    /// Live Q1 monster movement shared with the simulation.
    movement: Rc<RefCell<Q1MonsterMovement<S>>>,
    /// Selected Q1 movement profile.
    selected: MovementProfile,
    /// Decoded map geometry for graph construction.
    geometry: Rc<DecodedWorld>,
    /// Prepared navigation asset shared across graphs.
    prepared: PreparedNavigation,
    /// Loaded graphs by monster shape key.
    graphs: HashMap<String, NavigationGraph>,
    /// Per-actor worlds by actor.
    actors: HashMap<OwnedActor, MonsterActorEntry<S>>,
}

/// Cached per-actor navigation state (donor `actors` entry).
struct MonsterActorEntry<S: Q1MonsterMoveServices> {
    /// Shape key the cached world was built for.
    key: String,
    /// Actor navigation world.
    world: MonsterWorld<S>,
}

impl<S: Q1MonsterMoveServices + 'static> Mg3MonsterNavigationHost for Mg3MonsterInstallation<S> {
    fn for_actor(&mut self, actor: &OwnedActor) -> Option<Box<dyn Mg3NavigationRuntime + '_>> {
        let state = self.movement.borrow_mut().services_mut().read(actor.id())?;
        let key = shape_key(&state);
        let entry = self.actors.entry(actor.clone()).or_insert_with(|| MonsterActorEntry {
            key: String::new(),
            world: MonsterWorld {
                scene: MonsterScene,
                simulation: self.simulation.clone(),
                movement: Rc::clone(&self.movement),
                actor: actor.clone(),
            },
        });
        if entry.key != key {
            entry.key = key.clone();
            if !self.graphs.contains_key(&key) {
                let profile = monster_profile(&self.selected, &state);
                let topology = TopologyWorld {
                    inner: &entry.world,
                    scene: TopologyScene {
                        inner: &entry.world.scene,
                    },
                };
                let construction = NavigationConstruction {
                    geometry: &self.geometry,
                    map: &self.prepared.map,
                    profile: &profile,
                    world: &topology,
                    spacing: None,
                    link_distance: None,
                    maximum_nodes: None,
                    connections: None,
                };
                let loaded =
                    load_prepared_navigation(&construction, self.prepared.clone()).expect("load MG3 navigation graph");
                self.graphs.insert(key.clone(), loaded.runtime.graph);
            }
        }
        let graph = self.graphs.get(&key).expect("MG3 navigation graph").clone();
        let world = &self.actors.get(actor).expect("MG3 monster world").world;
        let runtime = NavigationRuntime::new(graph, world).expect("build MG3 navigation runtime");
        Some(Box::new(MonsterNavigationRuntime {
            runtime: RefCell::new(runtime),
            map_identity: self.prepared.map.identity.canonical(),
            pass_actor: Some(actor.id().clone()),
        }))
    }
}

/// Monster shape cache key (donor `JSON.stringify([state.bounds,
/// state.flags & (1 | 2)])`).
fn shape_key(state: &Q1MonsterMoveState) -> String {
    format!(
        "[{},{},{},{},{},{},{}]",
        state.bounds.min.x,
        state.bounds.min.y,
        state.bounds.min.z,
        state.bounds.max.x,
        state.bounds.max.y,
        state.bounds.max.z,
        state.flags & (1 | 2),
    )
}

/// Project the rich application movement profile onto the navigation-visible
/// movement profile (donor passes the rich profile straight through; the
/// Rust navigation contract only needs family, provider, and numerics).
fn bots_movement_profile(selected: &AppMovementProfile) -> MovementProfile {
    let (kind, id, numeric) = match selected {
        AppMovementProfile::Q1Netquake(profile) => (MovementKind::Q1Netquake, &profile.id, &profile.numeric),
        AppMovementProfile::Q1Quakeworld(profile) => (MovementKind::Q1Quakeworld, &profile.id, &profile.numeric),
        AppMovementProfile::Q2Classic(profile) => (MovementKind::Q2Classic, &profile.id, &profile.numeric),
        AppMovementProfile::Q2Rerelease(profile) => (MovementKind::Q2Rerelease, &profile.id, &profile.numeric),
        AppMovementProfile::Q3(profile) => (MovementKind::Q3, &profile.id, &profile.numeric),
    };
    MovementProfile {
        kind,
        id: id.clone(),
        numeric: *numeric,
    }
}

/// Canonical MG3 traversal profile for a monster shape (donor `profile`).
fn monster_profile(selected: &MovementProfile, state: &Q1MonsterMoveState) -> NavigationProfile {
    NavigationProfile {
        movement: selected.clone(),
        shape: BodyShape::Box(state.bounds),
        crouched_shape: None,
        policy: TracePolicy::Q1 {
            move_rule: Q1MoveRule::Normal,
            hull: None,
        },
        capabilities: HashSet::from([TravelMode::Walk, TravelMode::Drop, TravelMode::Swim]),
        maximum_step: 18.0,
        minimum_floor_normal: 0.7,
        maximum_drop: 18.0,
        team: None,
        monster: true,
    }
}

/// Actor navigation world (donor `world`).
struct MonsterWorld<S: Q1MonsterMoveServices> {
    /// Shared collision queries.
    scene: MonsterScene,
    /// Owning simulation for revision time.
    simulation: SharedSimulation,
    /// Live Q1 monster movement for detached prediction sessions.
    movement: Rc<RefCell<Q1MonsterMovement<S>>>,
    /// Navigating actor.
    actor: OwnedActor,
}

impl<S: Q1MonsterMoveServices> MonsterWorld<S> {
    /// Begin a detached route-prediction session (donor `begin()`).
    fn begin_session(&self) -> MonsterRouteSession<S> {
        MonsterRouteSession {
            predictor: create_q1_monster_movement(DetachedMonsterServices {
                live: Rc::clone(&self.movement),
                actor: self.actor.id().clone(),
                detached: None,
            }),
            actor: self.actor.clone(),
        }
    }
}

impl<S: Q1MonsterMoveServices + 'static> NavigationWorld for MonsterWorld<S> {
    fn scene(&self) -> &dyn SceneQueries {
        &self.scene
    }

    fn pass_actor(&self) -> Option<ActorId> {
        Some(self.actor.id().clone())
    }

    fn revision(&self) -> i64 {
        (self.simulation.time_seconds() * 1000.0) as i64
    }

    fn admit(&self, request: &TraversalRequest, _profile: &NavigationProfile) -> TraversalAdmission {
        self.begin_session().admit_inner(request)
    }

    fn begin_route(&self, _profile: &NavigationProfile) -> Box<dyn NavigationRoutePrediction> {
        Box::new(self.begin_session())
    }

    fn entity(&self, _binding: &qa_bots::NavigationEntityBinding) -> Option<qa_bots::NavigationEntityState> {
        None
    }

    fn hazard(&self, _bounds: &Bounds) -> bool {
        false
    }
}

/// Empty-world collision queries. Collision-lane seam: answers as if the
/// world held no geometry until the simulation scene queries are wired
/// through (same seam as the player-movement empty-world traces).
struct MonsterScene;

impl SceneQueries for MonsterScene {
    fn trace(&self, query: &TraceQuery) -> TraceResult {
        TraceResult {
            fraction: 1.0,
            end: query.end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            detail: TraceDetail::Q1 {
                in_open: true,
                in_water: false,
                source_plane: Plane {
                    normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                    distance: 0.0,
                },
                surface_flags: None,
                contents: None,
            },
        }
    }

    fn point_contents(&self, _query: &PointContentsQuery) -> PointContentsResult {
        PointContentsResult::Q1 { contents: 0 }
    }

    fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> LeafQueryResult {
        LeafQueryResult {
            leaves: Vec::new(),
            topnode: None,
            overflow: false,
        }
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        first == second
    }

    fn cluster_visible(&self, _from: i32, _to: i32, _kind: VisibilityKind) -> bool {
        true
    }
}

/// Stable-topology world: model zero of the same scene (donor `topology`).
/// Live movement checks actors and movers; the topology scene pins the
/// trace target so construction sees stable geometry.
struct TopologyWorld<'w, S: Q1MonsterMoveServices> {
    /// Wrapped live world.
    inner: &'w MonsterWorld<S>,
    /// Target-pinned scene.
    scene: TopologyScene<'w>,
}

impl<S: Q1MonsterMoveServices + 'static> NavigationWorld for TopologyWorld<'_, S> {
    fn scene(&self) -> &dyn SceneQueries {
        &self.scene
    }

    fn pass_actor(&self) -> Option<ActorId> {
        self.inner.pass_actor()
    }

    fn revision(&self) -> i64 {
        self.inner.revision()
    }

    fn admit(&self, request: &TraversalRequest, profile: &NavigationProfile) -> TraversalAdmission {
        self.inner.admit(request, profile)
    }

    fn begin_route(&self, profile: &NavigationProfile) -> Box<dyn NavigationRoutePrediction> {
        self.inner.begin_route(profile)
    }

    fn entity(&self, binding: &qa_bots::NavigationEntityBinding) -> Option<qa_bots::NavigationEntityState> {
        self.inner.entity(binding)
    }

    fn hazard(&self, bounds: &Bounds) -> bool {
        self.inner.hazard(bounds)
    }
}

/// Scene wrapper pinning trace targets to model zero (donor topology
/// `scene` override).
struct TopologyScene<'w> {
    /// Wrapped scene.
    inner: &'w MonsterScene,
}

impl SceneQueries for TopologyScene<'_> {
    fn trace(&self, query: &TraceQuery) -> TraceResult {
        let mut pinned = query.clone();
        pinned.target = QueryTarget::Model {
            model: 0,
            origin: ZERO,
            angles: ZERO,
        };
        self.inner.trace(&pinned)
    }

    fn point_contents(&self, query: &PointContentsQuery) -> PointContentsResult {
        let mut pinned = query.clone();
        pinned.target = QueryTarget::Model {
            model: 0,
            origin: ZERO,
            angles: ZERO,
        };
        self.inner.point_contents(&pinned)
    }

    fn box_leaves(&self, bounds: &Bounds, limit: usize) -> LeafQueryResult {
        self.inner.box_leaves(bounds, limit)
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        self.inner.areas_connected(first, second)
    }

    fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> bool {
        self.inner.cluster_visible(from, to, kind)
    }
}

/// Detached prediction services (donor `begin()` predictor services
/// override): the prediction actor reads the detached copy and writes
/// land in the detached copy; everything else forwards to live movement.
struct DetachedMonsterServices<S: Q1MonsterMoveServices> {
    /// Live movement shared with the simulation.
    live: Rc<RefCell<Q1MonsterMovement<S>>>,
    /// Prediction actor id.
    actor: ActorId,
    /// Detached prediction copy, seeded on first admission.
    detached: Option<Q1MonsterMoveState>,
}

impl<S: Q1MonsterMoveServices> DetachedMonsterServices<S> {
    /// Read the live snapshot, bypassing the detached copy.
    fn live_read(&mut self, actor: &ActorId) -> Option<Q1MonsterMoveState> {
        self.live.borrow_mut().services_mut().read(actor)
    }
}

impl<S: Q1MonsterMoveServices> Q1MonsterMoveServices for DetachedMonsterServices<S> {
    fn numeric(&self) -> NumericOps {
        self.live.borrow().services().numeric()
    }

    fn trace(&mut self, actor: &ActorId, start: Vec3, end: Vec3, bounds: Option<Bounds>) -> Q1Trace {
        self.live.borrow_mut().services_mut().trace(actor, start, end, bounds)
    }

    fn point_contents(&mut self, actor: &ActorId, point: Vec3) -> i32 {
        self.live.borrow_mut().services_mut().point_contents(actor, point)
    }

    fn next_random(&mut self) -> i32 {
        self.live.borrow_mut().services_mut().next_random()
    }

    fn read(&mut self, actor: &ActorId) -> Option<Q1MonsterMoveState> {
        if *actor == self.actor {
            self.detached.clone()
        } else {
            self.live.borrow_mut().services_mut().read(actor)
        }
    }

    fn read_target(&mut self, actor: &ActorId) -> Option<Q1MonsterTarget> {
        self.live.borrow_mut().services_mut().read_target(actor)
    }

    fn write(&mut self, _actor: &OwnedActor, state: Q1MonsterMoveState) {
        self.detached = Some(state);
    }

    fn link(&mut self, _actor: &OwnedActor, _touch_triggers: bool) {}
}

/// Detached route-prediction session (donor `begin()` result).
struct MonsterRouteSession<S: Q1MonsterMoveServices> {
    /// Predictor over detached services.
    predictor: Q1MonsterMovement<DetachedMonsterServices<S>>,
    /// Navigating actor.
    actor: OwnedActor,
}

impl<S: Q1MonsterMoveServices> NavigationRoutePrediction for MonsterRouteSession<S> {
    fn admit(&mut self, request: &TraversalRequest) -> Result<TraversalAdmission, qa_bots::BotsError> {
        Ok(self.admit_inner(request))
    }
}

impl<S: Q1MonsterMoveServices> MonsterRouteSession<S> {
    /// Admit one traversal against the detached copy (donor `admit`).
    fn admit_inner(&mut self, request: &TraversalRequest) -> TraversalAdmission {
        if !matches!(request.mode, TravelMode::Walk | TravelMode::Drop | TravelMode::Swim) {
            return TraversalAdmission::Refused {
                reason: "MG3 path walking cannot execute this traversal".to_string(),
            };
        }
        let Some(actual) = self.predictor.services_mut().live_read(self.actor.id()) else {
            return TraversalAdmission::Refused {
                reason: "Monster was removed".to_string(),
            };
        };
        if self.predictor.services_mut().detached.is_none() {
            self.predictor.services_mut().detached = Some(Q1MonsterMoveState {
                origin: request.from,
                ..actual
            });
        }
        let detached_origin = self
            .predictor
            .services_mut()
            .detached
            .as_ref()
            .expect("detached monster")
            .origin;
        if distance(detached_origin, request.from) > 2.0 {
            return TraversalAdmission::Refused {
                reason: "Monster route segments are disconnected".to_string(),
            };
        }
        let mut trajectory = vec![detached_origin];
        let steps = (distance(request.from, request.to) / 8.0).ceil() as usize + 2;
        for _ in 0..steps {
            let origin = self
                .predictor
                .services_mut()
                .detached
                .as_ref()
                .expect("detached monster")
                .origin;
            let dx = f64::from(request.to.x - origin.x);
            let dy = f64::from(request.to.y - origin.y);
            let length = dx.hypot(dy);
            if length <= 1.0 {
                break;
            }
            let yaw = dy.atan2(dx) * 180.0 / PI;
            let moved = self.predictor.walk_move(&self.actor, yaw, length.min(8.0));
            if !matches!(moved, Ok(true)) {
                return TraversalAdmission::Refused {
                    reason: "Source monster walkMove blocked".to_string(),
                };
            }
            let next = self
                .predictor
                .services_mut()
                .detached
                .as_ref()
                .expect("detached monster")
                .origin;
            trajectory.push(next);
        }
        let end = self
            .predictor
            .services_mut()
            .detached
            .as_ref()
            .expect("detached monster")
            .origin;
        if distance(end, request.to) <= 2.0 {
            TraversalAdmission::Admitted {
                seconds: distance(request.from, request.to) / 100.0,
                trajectory,
            }
        } else {
            TraversalAdmission::Refused {
                reason: "Source monster walkMove did not reach the endpoint".to_string(),
            }
        }
    }
}

/// 3D distance (donor `distance`).
fn distance(a: Vec3, b: Vec3) -> f64 {
    let dx = f64::from(a.x - b.x);
    let dy = f64::from(a.y - b.y);
    let dz = f64::from(a.z - b.z);
    dx.hypot(dy).hypot(dz)
}

/// Project a navigation travel mode onto the MG3 mode set.
fn mg3_mode(mode: TravelMode) -> Mg3TravelMode {
    match mode {
        TravelMode::Walk => Mg3TravelMode::Walk,
        TravelMode::Drop => Mg3TravelMode::Drop,
        TravelMode::Swim => Mg3TravelMode::Swim,
        _ => Mg3TravelMode::Other,
    }
}

/// MG3 runtime view over a per-actor navigation runtime. The wrapped
/// runtime needs `&mut self` for routing, so it lives behind a `RefCell`;
/// MG3 callers never reenter.
struct MonsterNavigationRuntime<'w> {
    /// Navigation runtime.
    runtime: RefCell<NavigationRuntime<'w>>,
    /// Route map identity.
    map_identity: String,
    /// Self collision exclusion.
    pass_actor: Option<ActorId>,
}

impl Mg3NavigationRuntime for MonsterNavigationRuntime<'_> {
    fn monster_profile(&self) -> bool {
        self.runtime.borrow().graph.profile.monster
    }

    fn pass_actor(&self) -> Option<ActorId> {
        self.pass_actor.clone()
    }

    fn map_identity(&self) -> &str {
        &self.map_identity
    }

    fn node_known(&self, id: i32) -> bool {
        self.runtime.borrow().node(id).is_some()
    }

    fn edge_by_id(&self, id: i32) -> Option<Mg3NavigationEdge> {
        self.runtime
            .borrow()
            .graph
            .edges
            .iter()
            .find(|edge| edge.id == id)
            .map(|edge| Mg3NavigationEdge {
                id: edge.id,
                mode: mg3_mode(edge.mode),
            })
    }

    fn route_still_valid(&self, route: &Mg3NavigationRoute) -> bool {
        let mut runtime = self.runtime.borrow_mut();
        let Some(edges) = route
            .edges
            .iter()
            .map(|edge| {
                runtime
                    .graph
                    .edges
                    .iter()
                    .find(|graph_edge| graph_edge.id == edge.id)
                    .cloned()
            })
            .collect::<Option<Vec<NavigationEdge>>>()
        else {
            return false;
        };
        let Some(identity) = ResourceIdentity::parse(&route.map_identity) else {
            return false;
        };
        runtime.route_still_valid(&NavigationRoute {
            map: NavigationMapIdentity {
                name: String::new(),
                format: WorldKind::Q1Bsp,
                identity,
            },
            nodes: route.nodes.clone(),
            edges,
            points: route.points.clone(),
            travel_seconds: route.travel_seconds,
            generation: i64::from(route.generation),
        })
    }

    fn route(&self, request: &Mg3RouteRequest) -> Option<Mg3NavigationRoute> {
        let modes = request.edge_modes;
        let query = NavigationRouteQuery {
            start: request.start,
            goal: request.goal,
            start_node: None,
            goal_node: None,
            travel_flags: None,
            disabled_areas: HashSet::new(),
            edge_filter: Some(Rc::new(move |edge: &NavigationEdge| {
                modes.contains(&mg3_mode(edge.mode))
            })),
            maximum_searches: None,
        };
        match self.runtime.borrow_mut().route(&query) {
            Ok(NavigationRouteResult::Route { route }) => Some(Mg3NavigationRoute {
                map_identity: route.map.identity.canonical(),
                nodes: route.nodes.clone(),
                edges: route
                    .edges
                    .iter()
                    .map(|edge| Mg3NavigationEdge {
                        id: edge.id,
                        mode: mg3_mode(edge.mode),
                    })
                    .collect(),
                points: route.points.clone(),
                travel_seconds: route.travel_seconds,
                generation: route.generation as i32,
            }),
            Ok(NavigationRouteResult::Unreachable { .. }) | Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_world::movement::types::TraceHit as WorldTraceHit;

    use super::*;

    /// Monster shape cache key covers bounds and ground/fly flags.
    #[test]
    fn shape_key_covers_bounds_and_flags() {
        let bounds = Bounds {
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
        };
        let state = Q1MonsterMoveState {
            origin: ZERO,
            angles: ZERO,
            bounds,
            absolute_bounds: bounds,
            flags: 1 | 2 | 8,
            ground: WorldTraceHit::None,
            ideal_yaw: 0.0,
            yaw_speed: 0.0,
            enemy: None,
        };
        assert_eq!(shape_key(&state), "[-16,-16,-24,16,16,32,3]");
    }

    /// Travel modes project onto the MG3 mode set.
    #[test]
    fn modes_project_to_mg3() {
        assert!(matches!(mg3_mode(TravelMode::Walk), Mg3TravelMode::Walk));
        assert!(matches!(mg3_mode(TravelMode::Drop), Mg3TravelMode::Drop));
        assert!(matches!(mg3_mode(TravelMode::Swim), Mg3TravelMode::Swim));
        assert!(matches!(mg3_mode(TravelMode::Jump), Mg3TravelMode::Other));
    }
}
