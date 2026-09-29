//! Live routing over shared world state, ported from
//! `src/bots/navigation/runtime.ts`. Movement-admitted segments only;
//! blocked or disabled topology, occupancies, hazards, liquids, and
//! missed mover windows invalidate subsequent candidates through
//! world-revision tracking.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::math::{Bounds, Vec3};

use crate::aas::{aas_bbox_areas, aas_point_area, aas_trace_areas, AasAreaCrossing};
use crate::error::BotsError;
use crate::estimates::NavigationEstimates;
use crate::graph::{aas_area_travel_flags, aas_travel_flag, navigation_edge_travel_flag};
use crate::helpers::{clear, contents, distance, node_profile, trace, translated};
use crate::save::{SaveReader, SaveValue};
use crate::scene::QueryTarget;
use crate::train::navigation_train_ride;
use crate::types::{
    ElevatorPhase, ElevatorState, KexGeneration, NavigationAsset, NavigationEdge, NavigationEntityState,
    NavigationEstimateQuery, NavigationEstimateResult, NavigationGraph, NavigationMapIdentity, NavigationNode,
    NavigationProfile, NavigationRoute, NavigationRoutePrediction, NavigationRouteResult, NavigationSource,
    NavigationWorld, TravelMode, TraversalAdmission, TraversalRequest,
};

/// Shared edge filter for route queries.
pub type NavigationEdgeFilter = Rc<dyn Fn(&NavigationEdge) -> bool>;

/// Route query.
#[derive(Clone)]
pub struct NavigationRouteQuery {
    /// Route start.
    pub start: Vec3,
    /// Route goal.
    pub goal: Vec3,
    /// Explicit start node.
    pub start_node: Option<i32>,
    /// Explicit goal node.
    pub goal_node: Option<i32>,
    /// Travel-flag mask.
    pub travel_flags: Option<i32>,
    /// Disabled areas.
    pub disabled_areas: HashSet<i32>,
    /// Edge filter.
    pub edge_filter: Option<NavigationEdgeFilter>,
    /// Maximum candidate routes.
    pub maximum_searches: Option<usize>,
}

/// Static eligibility view over runtime topology.
#[derive(Clone, Copy)]
pub(crate) struct EligibilityQuery<'q> {
    /// Travel-flag mask.
    pub travel_flags: Option<i32>,
    /// Disabled areas.
    pub disabled_areas: Option<&'q HashSet<i32>>,
    /// Edge filter.
    pub edge_filter: Option<&'q dyn Fn(&NavigationEdge) -> bool>,
}

impl<'q> EligibilityQuery<'q> {
    /// Eligibility from a route query.
    pub fn from_route(query: &'q NavigationRouteQuery) -> Self {
        Self {
            travel_flags: query.travel_flags,
            disabled_areas: Some(&query.disabled_areas),
            edge_filter: query.edge_filter.as_deref(),
        }
    }
}

/// Shared static topology borrowed from a runtime.
#[derive(Clone, Copy)]
struct StaticView<'s> {
    graph: &'s NavigationGraph,
    nodes: &'s HashMap<i32, NavigationNode>,
    enabled: &'s HashMap<i32, bool>,
    blocked: &'s HashMap<i32, String>,
}

impl<'s> StaticView<'s> {
    fn node(&self, id: i32) -> Option<&'s NavigationNode> {
        self.nodes.get(&id)
    }

    fn static_node_allowed(&self, node: &NavigationNode, query: Option<EligibilityQuery<'_>>) -> bool {
        let profile = node_profile(&self.graph.profile, node);
        if profile.is_none() {
            return false;
        }
        if self.enabled.get(&node.id) == Some(&false)
            || query
                .and_then(|query| query.disabled_areas)
                .is_some_and(|areas| areas.contains(&node.id))
        {
            return false;
        }
        if let NavigationSource::Aas { .. } = node.source {
            if (node.flags & 8) != 0 && self.enabled.get(&node.id) != Some(&true) {
                return false;
            }
        }
        if let NavigationSource::Kex {
            generation: KexGeneration::Nav3,
            ..
        } = node.source
        {
            let profile = profile.unwrap_or_else(|| self.graph.profile.clone());
            if (node.flags & 8192) != 0 || profile.monster && (node.flags & 256) != 0 {
                return false;
            }
            if (node.flags & 512) != 0 && !profile.capabilities.contains(&TravelMode::Crouch) {
                return false;
            }
        }
        true
    }

    fn static_edge_allowed(&self, edge: &NavigationEdge, query: Option<EligibilityQuery<'_>>) -> bool {
        let profile = &self.graph.profile;
        if !profile.capabilities.contains(&edge.mode)
            || edge.mode == TravelMode::Unknown
            || self.blocked.contains_key(&edge.id)
        {
            return false;
        }
        if let Some(query) = query {
            if let Some(filter) = query.edge_filter {
                if !filter(edge) {
                    return false;
                }
            }
            if let Some(flags) = query.travel_flags {
                if (navigation_edge_travel_flag(edge) & flags) == 0 {
                    return false;
                }
            }
        }
        let (Some(source), Some(target)) = (self.node(edge.from), self.node(edge.to)) else {
            return false;
        };
        if !self.static_node_allowed(source, query) || !self.static_node_allowed(target, query) {
            return false;
        }
        if edge.mode == TravelMode::Drop && f64::from(edge.start.z) - f64::from(edge.end.z) > profile.maximum_drop {
            return false;
        }
        if let NavigationSource::Aas { .. } = edge.source {
            if let Some(flags) = query.and_then(|query| query.travel_flags) {
                if (aas_travel_flag(edge.source_travel_type) & flags) == 0 {
                    return false;
                }
            }
            if profile.team == Some(crate::types::Team::Red) && (edge.source_travel_type & 0x0100_0000) != 0
                || profile.team == Some(crate::types::Team::Blue) && (edge.source_travel_type & 0x0200_0000) != 0
            {
                return false;
            }
            if let Some(NavigationAsset::Aas(asset)) = &self.graph.asset {
                if let Some(settings) = asset.settings.get(edge.to as usize) {
                    if let Some(flags) = query.and_then(|query| query.travel_flags) {
                        if (aas_area_travel_flags(settings) & !flags) != 0 {
                            return false;
                        }
                    }
                    if profile.team == Some(crate::types::Team::Red) && (settings.contents & 2048) != 0
                        || profile.team == Some(crate::types::Team::Blue) && (settings.contents & 4096) != 0
                    {
                        return false;
                    }
                }
            }
        } else if let NavigationSource::Kex {
            generation: KexGeneration::Nav3,
            ..
        } = edge.source
        {
            if (edge.source_flags & 64) != 0 {
                return false;
            }
            if let Some(team) = profile.team {
                let bit = if team == crate::types::Team::Red { 1 } else { 2 };
                if (edge.source_flags & bit) == 0 {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
struct QueueCost {
    node: i32,
    cost: f64,
}

struct Queue {
    values: Vec<QueueCost>,
}

impl Queue {
    fn push(&mut self, value: QueueCost) {
        let mut index = self.values.len();
        self.values.push(value);
        while index > 0 {
            let parent = (index - 1) / 2;
            if self.values[parent].cost <= self.values[index].cost {
                break;
            }
            self.values.swap(index, parent);
            index = parent;
        }
    }

    fn pop(&mut self) -> Option<QueueCost> {
        let first = self.values.first().cloned()?;
        let last = self.values.pop()?;
        if self.values.is_empty() {
            return Some(last);
        }
        let mut index = 0;
        while index * 2 + 1 < self.values.len() {
            let mut child = index * 2 + 1;
            if child + 1 < self.values.len() && self.values[child + 1].cost < self.values[child].cost {
                child += 1;
            }
            if last.cost <= self.values[child].cost {
                break;
            }
            self.values[index] = self.values[child].clone();
            index = child;
        }
        self.values[index] = last;
        Some(first)
    }
}

struct RouteTraversal {
    prediction: Box<dyn NavigationRoutePrediction>,
    cursor: Vec3,
    seconds: f64,
    points: Vec<Vec3>,
}

/// Elevator boarding state behind a node.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardingElevator {
    /// Boarding edge.
    pub edge: NavigationEdge,
    /// Elevator actor.
    pub actor: qa_core::identity::ActorId,
    /// Elevator platform state.
    pub platform: ElevatorState,
}

/// Train boarding state behind a node.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardingTrain {
    /// Boarding edge.
    pub edge: NavigationEdge,
    /// Train entity state.
    pub state: NavigationEntityState,
    /// Train ride.
    pub ride: crate::train::NavigationTrainRide,
}

/// Train step outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum TrainStep {
    /// Move toward a target.
    Move {
        /// Approach or exit stage.
        stage: TrainStage,
        /// Target point.
        target: Vec3,
    },
    /// Wait for the train.
    Wait,
    /// Ride the train.
    Ride,
    /// No train step.
    Unavailable,
}

/// Train step stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainStage {
    /// Approach the train.
    Approach,
    /// Exit the train.
    Exit,
}

/// Debug line over an edge.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationDebugLine {
    /// Line start.
    pub from: Vec3,
    /// Line end.
    pub to: Vec3,
    /// Edge id.
    pub edge: i32,
    /// Edge currently allowed.
    pub enabled: bool,
    /// Travel mode.
    pub mode: TravelMode,
}

/// Serializable runtime checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationRuntimeCheckpoint {
    /// Checkpoint version (always 1).
    pub version: i32,
    /// Map identity.
    pub map: NavigationMapIdentity,
    /// Area overrides.
    pub enabled: Vec<(i32, bool)>,
    /// Blocked edges with reasons.
    pub blocked: Vec<(i32, String)>,
    /// Measured admission seconds.
    pub admission_seconds: Vec<(i32, f64)>,
    /// World revision.
    pub world_revision: i64,
    /// Runtime generation.
    pub generation: i64,
}

impl NavigationRuntimeCheckpoint {
    /// Encode as a checkpoint value.
    #[must_use]
    pub fn to_save_value(&self) -> SaveValue {
        SaveValue::map(vec![
            ("version", SaveValue::Int(i64::from(self.version))),
            (
                "map",
                SaveValue::map(vec![
                    ("name", SaveValue::Str(self.map.name.clone())),
                    ("format", SaveValue::Str(self.map.format.as_str().to_string())),
                    ("digest", SaveValue::Str(self.map.digest.text.clone())),
                ]),
            ),
            (
                "enabled",
                SaveValue::List(
                    self.enabled
                        .iter()
                        .map(|(id, enabled)| {
                            SaveValue::map(vec![
                                ("id", SaveValue::Int(i64::from(*id))),
                                ("enabled", SaveValue::Bool(*enabled)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "blocked",
                SaveValue::List(
                    self.blocked
                        .iter()
                        .map(|(id, reason)| {
                            SaveValue::map(vec![
                                ("id", SaveValue::Int(i64::from(*id))),
                                ("reason", SaveValue::Str(reason.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "admissionSeconds",
                SaveValue::List(
                    self.admission_seconds
                        .iter()
                        .map(|(id, seconds)| {
                            SaveValue::map(vec![
                                ("id", SaveValue::Int(i64::from(*id))),
                                ("seconds", SaveValue::Float(*seconds)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("worldRevision", SaveValue::Int(self.world_revision)),
            ("generation", SaveValue::Int(self.generation)),
        ])
    }
}

/// Live navigation runtime. Methods that admit, estimate, or publish
/// take `&mut self` because admission records durations, refreshes
/// world revisions, and caches estimate tails.
pub struct NavigationRuntime<'w> {
    /// Navigation graph.
    pub graph: NavigationGraph,
    world: &'w dyn NavigationWorld,
    estimates: NavigationEstimates,
    nodes: HashMap<i32, NavigationNode>,
    outgoing: HashMap<i32, Vec<NavigationEdge>>,
    enabled: HashMap<i32, bool>,
    blocked: HashMap<i32, String>,
    admission_seconds: HashMap<i32, f64>,
    world_revision: i64,
    generation: i64,
}

impl<'w> NavigationRuntime<'w> {
    /// Build a runtime over a graph and shared world.
    pub fn new(graph: NavigationGraph, world: &'w dyn NavigationWorld) -> Result<Self, BotsError> {
        let mut nodes = HashMap::new();
        for node in &graph.nodes {
            if nodes.insert(node.id, node.clone()).is_some() {
                return Err(BotsError::DuplicateNode { id: node.id });
            }
        }
        let mut outgoing: HashMap<i32, Vec<NavigationEdge>> = HashMap::new();
        let mut edge_ids = HashSet::new();
        for node in &graph.nodes {
            outgoing.insert(node.id, Vec::new());
        }
        for edge in &graph.edges {
            if !edge_ids.insert(edge.id) || !nodes.contains_key(&edge.from) || !nodes.contains_key(&edge.to) {
                return Err(BotsError::BadEdge);
            }
            if !edge.travel_seconds.is_finite() || edge.travel_seconds < 0.0 {
                return Err(BotsError::BadEdgeCost);
            }
            if let Some(list) = outgoing.get_mut(&edge.from) {
                list.push(edge.clone());
            }
        }
        let estimates = NavigationEstimates::new(&graph)?;
        let world_revision = world.revision();
        Ok(Self {
            graph,
            world,
            estimates,
            nodes,
            outgoing,
            enabled: HashMap::new(),
            blocked: HashMap::new(),
            admission_seconds: HashMap::new(),
            world_revision,
            generation: 0,
        })
    }

    /// Shared world.
    #[must_use]
    pub fn world(&self) -> &dyn NavigationWorld {
        self.world
    }

    fn view(&self) -> StaticView<'_> {
        StaticView {
            graph: &self.graph,
            nodes: &self.nodes,
            enabled: &self.enabled,
            blocked: &self.blocked,
        }
    }

    /// Runtime generation, refreshing world state first.
    pub fn generation(&mut self) -> i64 {
        self.refresh();
        self.generation
    }

    /// Node by id.
    #[must_use]
    pub fn node(&self, id: i32) -> Option<&NavigationNode> {
        self.nodes.get(&id)
    }

    /// Outgoing edges of a node.
    #[must_use]
    pub fn outgoing(&self, id: i32) -> &[NavigationEdge] {
        self.outgoing.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Area containing a point.
    pub fn area_at(&self, point: Vec3) -> Result<Option<i32>, BotsError> {
        if let Some(NavigationAsset::Aas(asset)) = &self.graph.asset {
            let area = aas_point_area(asset, point)?;
            return Ok(self.node(area).map(|_| area));
        }
        Ok(self.nearest(point, 512.0).map(|node| node.id))
    }

    /// Nearest admissible node within a radius.
    pub fn nearest(&self, point: Vec3, radius: f64) -> Option<NavigationNode> {
        let mut selected = None;
        let mut best = radius;
        for node in &self.graph.nodes {
            let d = distance(point, node.origin);
            if d > best {
                continue;
            }
            let Some(profile) = node_profile(&self.graph.profile, node) else {
                continue;
            };
            if !self.node_allowed(node, None, false) {
                continue;
            }
            if !clear(self.world, &profile, point, node.origin, &QueryTarget::World) {
                continue;
            }
            best = d;
            selected = Some(node.clone());
        }
        selected
    }

    /// Areas touched by bounds.
    pub fn bbox_areas(&self, bounds: &Bounds) -> Result<Vec<i32>, BotsError> {
        if let Some(NavigationAsset::Aas(asset)) = &self.graph.asset {
            return aas_bbox_areas(asset, *bounds, asset.areas.len());
        }
        Ok(self
            .graph
            .nodes
            .iter()
            .filter(|node| {
                node.bounds.min.x <= bounds.max.x
                    && node.bounds.max.x >= bounds.min.x
                    && node.bounds.min.y <= bounds.max.y
                    && node.bounds.max.y >= bounds.min.y
                    && node.bounds.min.z <= bounds.max.z
                    && node.bounds.max.z >= bounds.min.z
            })
            .map(|node| node.id)
            .collect())
    }

    /// Areas crossed by a segment.
    pub fn trace_areas(&self, start: Vec3, end: Vec3, maximum: usize) -> Result<Vec<AasAreaCrossing>, BotsError> {
        if let Some(NavigationAsset::Aas(asset)) = &self.graph.asset {
            return aas_trace_areas(asset, start, end, maximum);
        }
        let mut crossed: Vec<(i32, f64, Vec3)> = Vec::new();
        for node in &self.graph.nodes {
            let mut enter = 0.0f64;
            let mut leave = 1.0f64;
            for axis in 0..3 {
                let (s, e, min, max) = match axis {
                    0 => (
                        f64::from(start.x),
                        f64::from(end.x),
                        f64::from(node.bounds.min.x),
                        f64::from(node.bounds.max.x),
                    ),
                    1 => (
                        f64::from(start.y),
                        f64::from(end.y),
                        f64::from(node.bounds.min.y),
                        f64::from(node.bounds.max.y),
                    ),
                    _ => (
                        f64::from(start.z),
                        f64::from(end.z),
                        f64::from(node.bounds.min.z),
                        f64::from(node.bounds.max.z),
                    ),
                };
                let delta = e - s;
                if delta == 0.0 {
                    if s < min || s > max {
                        leave = -1.0;
                    }
                    continue;
                }
                let first = (min - s) / delta;
                let second = (max - s) / delta;
                enter = enter.max(first.min(second));
                leave = leave.min(first.max(second));
            }
            if enter <= leave {
                crossed.push((
                    node.id,
                    enter,
                    Vec3 {
                        x: (f64::from(start.x) + (f64::from(end.x) - f64::from(start.x)) * enter) as f32,
                        y: (f64::from(start.y) + (f64::from(end.y) - f64::from(start.y)) * enter) as f32,
                        z: (f64::from(start.z) + (f64::from(end.z) - f64::from(start.z)) * enter) as f32,
                    },
                ));
            }
        }
        crossed.sort_by(|a, b| a.1.total_cmp(&b.1));
        crossed.truncate(maximum);
        Ok(crossed
            .into_iter()
            .map(|(area, _, point)| AasAreaCrossing { area, point })
            .collect())
    }

    /// Whether a node is statically allowed.
    pub fn static_node_allowed(&self, node: &NavigationNode, query: Option<&NavigationRouteQuery>) -> bool {
        self.view()
            .static_node_allowed(node, query.map(EligibilityQuery::from_route))
    }

    /// Whether an edge is statically allowed.
    pub fn static_edge_allowed(&self, edge: &NavigationEdge, query: Option<&NavigationRouteQuery>) -> bool {
        self.view()
            .static_edge_allowed(edge, query.map(EligibilityQuery::from_route))
    }

    /// Whether a node admits occupancy right now.
    pub fn node_allowed(
        &self,
        node: &NavigationNode,
        query: Option<&NavigationRouteQuery>,
        await_elevator: bool,
    ) -> bool {
        let eligibility = query.map(EligibilityQuery::from_route);
        if !self.view().static_node_allowed(node, eligibility) {
            return false;
        }
        let Some(profile) = node_profile(&self.graph.profile, node) else {
            return false;
        };
        let target = QueryTarget::World;
        let medium = contents(self.world, &profile, node.origin, &target);
        if (medium & 6) != 0 || self.world.hazard(&translated(node.origin, profile.shape.bounds())) {
            return false;
        }
        if !clear(self.world, &profile, node.origin, node.origin, &target) {
            return false;
        }
        if let NavigationSource::Kex {
            generation: KexGeneration::Nav3,
            ..
        } = node.source
        {
            if (node.flags & 64) != 0 {
                let bounds = profile.shape.bounds();
                let result = trace(
                    self.world,
                    &NavigationProfile {
                        shape: crate::scene::BodyShape::Box(Bounds {
                            min: Vec3 {
                                x: bounds.min.x,
                                y: bounds.min.y,
                                z: 0.0,
                            },
                            max: Vec3 {
                                x: bounds.max.x,
                                y: bounds.max.y,
                                z: 0.0,
                            },
                        }),
                        ..profile.clone()
                    },
                    node.origin,
                    Vec3 {
                        x: node.origin.x,
                        y: node.origin.y,
                        z: node.origin.z - 96.0,
                    },
                    &target,
                );
                if result.fraction == 1.0 && !await_elevator {
                    return false;
                }
            }
            if (node.flags & 128) != 0 {
                let bounds = profile.shape.bounds();
                let floor = trace(
                    self.world,
                    &profile,
                    Vec3 {
                        x: node.origin.x,
                        y: node.origin.y,
                        z: node.origin.z + bounds.max.z,
                    },
                    Vec3 {
                        x: node.origin.x,
                        y: node.origin.y,
                        z: node.origin.z - 4096.0,
                    },
                    &target,
                );
                if floor.fraction == 1.0 && !await_elevator {
                    return false;
                }
            }
        }
        true
    }

    /// Whether an edge admits traversal right now.
    pub fn edge_allowed(&self, edge: &NavigationEdge, query: Option<&NavigationRouteQuery>) -> bool {
        self.edge_allowed_with(edge, query, &mut |node, await_elevator| {
            self.node_allowed(node, query, await_elevator)
        })
    }

    fn edge_allowed_with(
        &self,
        edge: &NavigationEdge,
        query: Option<&NavigationRouteQuery>,
        node_allowed: &mut dyn FnMut(&NavigationNode, bool) -> bool,
    ) -> bool {
        if !self
            .view()
            .static_edge_allowed(edge, query.map(EligibilityQuery::from_route))
        {
            return false;
        }
        let target = self.node(edge.to);
        let elevator = self.boarding_elevator(edge.to);
        let train = self.boarding_train(edge.to);
        let Some(target) = target else {
            return false;
        };
        let source_train = if edge.mode == TravelMode::Mover {
            edge.entity.as_ref().and_then(|entity| self.world.entity(entity))
        } else {
            None
        };
        let ride = source_train
            .as_ref()
            .and_then(|state| navigation_train_ride(state, edge, &self.graph.profile));
        let awaiting_train = train.as_ref().is_some_and(|train| {
            train.state.train.as_ref().is_some_and(|platform| {
                distance(platform.origin, train.ride.boarding.origin) > self.graph.profile.maximum_step
            })
        }) || source_train.as_ref().is_some_and(|state| {
            state.train.as_ref().is_some_and(|platform| {
                platform.running
                    && ride.as_ref().is_some_and(|ride| {
                        distance(platform.origin, ride.arrival.origin) > self.graph.profile.maximum_step
                    })
            })
        });
        if awaiting_train
            && !clear(
                self.world,
                &self.graph.profile,
                target.origin,
                target.origin,
                &QueryTarget::World,
            )
        {
            return false;
        }
        if !node_allowed(
            target,
            awaiting_train
                || elevator
                    .as_ref()
                    .is_some_and(|elevator| elevator.platform.phase != ElevatorPhase::Bottom),
        ) {
            return false;
        }
        if let Some(entity) = &edge.entity {
            match self.world.entity(entity) {
                Some(state) if state.enabled && !state.locked => {}
                _ => return false,
            }
        }
        true
    }

    /// Elevator boarding state behind a node.
    pub fn boarding_elevator(&self, node: i32) -> Option<BoardingElevator> {
        for edge in self.outgoing(node) {
            if !matches!(edge.source, NavigationSource::Kex { .. })
                || edge.mode != TravelMode::Mover
                || edge.source_travel_type != 6
                || edge.entity.is_none()
            {
                continue;
            }
            let binding = edge.entity.as_ref()?;
            let state = self.world.entity(binding)?;
            match state.elevator {
                Some(platform) if state.enabled && !state.locked => {
                    return Some(BoardingElevator {
                        edge: edge.clone(),
                        actor: state.actor.clone(),
                        platform,
                    });
                }
                _ => continue,
            }
        }
        None
    }

    /// Train boarding state behind a node.
    pub fn boarding_train(&self, node: i32) -> Option<BoardingTrain> {
        for edge in self.outgoing(node) {
            if edge.mode != TravelMode::Mover || edge.entity.is_none() {
                continue;
            }
            let state = self.world.entity(edge.entity.as_ref()?)?;
            if state.train.as_ref().is_none_or(|train| !train.running) {
                continue;
            }
            let ride = navigation_train_ride(&state, edge, &self.graph.profile)?;
            return Some(BoardingTrain {
                edge: edge.clone(),
                state,
                ride,
            });
        }
        None
    }

    /// Next train step toward an edge destination, or `None` when the
    /// edge is not a mover edge at all.
    pub fn train_step(
        &self,
        edge: &NavigationEdge,
        origin: Vec3,
        ground: Option<&qa_core::identity::ActorId>,
    ) -> Option<TrainStep> {
        if edge.mode != TravelMode::Mover || edge.entity.is_none() {
            return None;
        }
        let state = self.world.entity(edge.entity.as_ref()?)?;
        let train = state.train.as_ref()?;
        let ride = navigation_train_ride(&state, edge, &self.graph.profile)?;
        let aboard = ground == Some(&state.actor);
        if !self.view().static_edge_allowed(edge, None) || !aboard && !train.running {
            return Some(TrainStep::Unavailable);
        }
        if self
            .world
            .hazard(&translated(origin, self.graph.profile.shape.bounds()))
        {
            return Some(TrainStep::Unavailable);
        }
        if aboard && !train.running && distance(train.origin, ride.arrival.origin) > self.graph.profile.maximum_step {
            return Some(TrainStep::Unavailable);
        }
        if aboard && distance(train.origin, ride.arrival.origin) > self.graph.profile.maximum_step {
            return Some(TrainStep::Ride);
        }
        let staging = edge.hint.map_or(edge.start, |hint| hint.funnel);
        let target = if aboard {
            edge.end
        } else if distance(train.origin, ride.boarding.origin) <= self.graph.profile.maximum_step {
            edge.start
        } else {
            staging
        };
        if !aboard && target == staging && distance(origin, staging) <= 8.0 {
            return Some(TrainStep::Wait);
        }
        if self
            .world
            .hazard(&translated(target, self.graph.profile.shape.bounds()))
        {
            return Some(TrainStep::Unavailable);
        }
        let admission = self.world.admit(
            &TraversalRequest {
                from: origin,
                to: target,
                mode: TravelMode::Walk,
                hint: None,
                entity: None,
            },
            &self.graph.profile,
        );
        if admission.admitted() {
            Some(TrainStep::Move {
                stage: if aboard { TrainStage::Exit } else { TrainStage::Approach },
                target,
            })
        } else {
            Some(TrainStep::Unavailable)
        }
    }

    /// Override an area's enabled state, returning the previous value.
    pub fn enable_area(&mut self, area: i32, enabled: bool) -> Result<bool, BotsError> {
        let node = self.node(area).cloned().ok_or(BotsError::UnknownArea { id: area })?;
        let previous = self
            .enabled
            .get(&area)
            .copied()
            .unwrap_or(!(matches!(node.source, NavigationSource::Aas { .. }) && (node.flags & 8) != 0));
        if previous == enabled {
            return Ok(previous);
        }
        self.enabled.insert(area, enabled);
        self.invalidate();
        self.estimates.invalidate();
        Ok(previous)
    }

    /// Block an edge with a reason, or unblock with none.
    pub fn block_edge(&mut self, edge: i32, reason: Option<&str>) -> Result<(), BotsError> {
        if !self.graph.edges.iter().any(|candidate| candidate.id == edge) {
            return Err(BotsError::UnknownEdge { id: edge });
        }
        match reason {
            Some(reason) => {
                self.blocked.insert(edge, reason.to_string());
            }
            None => {
                self.blocked.remove(&edge);
            }
        }
        self.invalidate();
        self.estimates.invalidate();
        Ok(())
    }

    /// Drop measured durations and advance the generation.
    pub fn invalidate(&mut self) {
        self.generation += 1;
        self.admission_seconds.clear();
    }

    /// Resynchronize after world changes.
    pub fn refresh(&mut self) {
        let revision = self.world.revision();
        if revision != self.world_revision {
            self.world_revision = revision;
            self.invalidate();
        }
    }

    fn candidate(
        &self,
        start: i32,
        goal: i32,
        query: &NavigationRouteQuery,
        rejected: &HashSet<i32>,
        node_allowed: &mut dyn FnMut(&NavigationNode, bool) -> bool,
    ) -> Result<Option<Vec<NavigationEdge>>, BotsError> {
        let mut queue = Queue { values: Vec::new() };
        let mut costs: HashMap<i32, f64> = HashMap::from([(start, 0.0)]);
        let mut parents: HashMap<i32, NavigationEdge> = HashMap::new();
        queue.push(QueueCost { node: start, cost: 0.0 });
        while let Some(current) = queue.pop() {
            if Some(current.cost) != costs.get(&current.node).copied() {
                continue;
            }
            if current.node == goal {
                let mut path = Vec::new();
                let mut node = goal;
                while node != start {
                    let edge = parents
                        .get(&node)
                        .ok_or_else(|| BotsError::Internal("Navigation predecessor chain is incomplete".to_string()))?;
                    path.push(edge.clone());
                    node = edge.from;
                }
                path.reverse();
                return Ok(Some(path));
            }
            for edge in self.outgoing(current.node) {
                if rejected.contains(&edge.id) || !self.edge_allowed_with(edge, Some(query), node_allowed) {
                    continue;
                }
                let cost = current.cost
                    + self
                        .admission_seconds
                        .get(&edge.id)
                        .copied()
                        .unwrap_or(edge.travel_seconds);
                if cost >= costs.get(&edge.to).copied().unwrap_or(f64::INFINITY) {
                    continue;
                }
                costs.insert(edge.to, cost);
                parents.insert(edge.to, edge.clone());
                queue.push(QueueCost { node: edge.to, cost });
            }
        }
        Ok(None)
    }

    fn traversal(&self, from: Vec3) -> RouteTraversal {
        RouteTraversal {
            prediction: self.world.begin_route(&self.graph.profile),
            cursor: from,
            seconds: 0.0,
            points: vec![from],
        }
    }

    fn admit_traversal(
        state: &mut RouteTraversal,
        request: &TraversalRequest,
    ) -> Result<TraversalAdmission, BotsError> {
        let result = state.prediction.admit(request)?;
        if let TraversalAdmission::Admitted { seconds, trajectory } = &result {
            let last = trajectory.last();
            if last.is_none() || !seconds.is_finite() || *seconds < 0.0 {
                return Err(BotsError::BadAdmissionTrajectory);
            }
            state.seconds += *seconds;
            state.points.extend_from_slice(&trajectory[1..]);
            state.cursor = *last.unwrap_or(&state.cursor);
        }
        Ok(result)
    }

    fn traverse_edge(&mut self, edge: &NavigationEdge, state: &mut RouteTraversal) -> Result<bool, BotsError> {
        let train = if edge.mode == TravelMode::Mover {
            edge.entity.as_ref().and_then(|entity| self.world.entity(entity))
        } else {
            None
        };
        if train.as_ref().and_then(|train| train.train.as_ref()).is_some() {
            return self.traverse_train_edge(edge, state);
        }
        let mover = if edge.mode == TravelMode::Mover
            && matches!(edge.source, NavigationSource::Kex { .. })
            && edge.source_travel_type == 6
        {
            edge.entity.as_ref().and_then(|entity| self.world.entity(entity))
        } else {
            None
        };
        if let Some(mover) = mover {
            if mover.elevator.is_some() && mover.enabled && !mover.locked {
                return self.traverse_elevator_edge(edge, state, &mover);
            }
        }
        if distance(state.cursor, edge.start) > 1.0
            && !Self::admit_traversal(
                state,
                &TraversalRequest {
                    from: state.cursor,
                    to: edge.start,
                    mode: if edge.mode == TravelMode::Crouch {
                        TravelMode::Crouch
                    } else {
                        TravelMode::Walk
                    },
                    hint: None,
                    entity: None,
                },
            )?
            .admitted()
        {
            return Ok(false);
        }
        let mut landing = edge.end;
        let boarding = if edge.mode == TravelMode::Walk && matches!(edge.source, NavigationSource::Kex { .. }) {
            self.boarding_elevator(edge.to)
        } else {
            None
        };
        let train_boarding = if edge.mode == TravelMode::Walk {
            self.boarding_train(edge.to)
        } else {
            None
        };
        let train_far = train_boarding.as_ref().is_some_and(|train_boarding| {
            train_boarding.state.train.as_ref().is_some_and(|platform| {
                distance(platform.origin, train_boarding.ride.boarding.origin) > self.graph.profile.maximum_step
            })
        });
        if train_far {
            landing = train_boarding
                .as_ref()
                .unwrap_or_else(|| unreachable!("train boarding checked above"))
                .edge
                .hint
                .map_or(edge.start, |hint| hint.funnel);
        } else if let Some(boarding) = &boarding {
            landing = Self::elevator_landing(self, edge, boarding, landing);
        }
        let admission = Self::admit_traversal(
            state,
            &TraversalRequest {
                from: state.cursor,
                to: landing,
                mode: edge.mode,
                hint: edge.hint,
                entity: edge.entity.clone(),
            },
        )?;
        if let TraversalAdmission::Admitted { seconds, .. } = &admission {
            self.admission_seconds.insert(edge.id, *seconds);
        }
        Ok(admission.admitted())
    }

    fn traverse_train_edge(&self, edge: &NavigationEdge, state: &mut RouteTraversal) -> Result<bool, BotsError> {
        let train = edge.entity.as_ref().and_then(|entity| self.world.entity(entity));
        let Some(train) = train else {
            return Ok(false);
        };
        if train.train.as_ref().is_none_or(|platform| !platform.running)
            || navigation_train_ride(&train, edge, &self.graph.profile).is_none()
        {
            return Ok(false);
        }
        let staging = edge.hint.map_or(edge.start, |hint| hint.funnel);
        if distance(state.cursor, staging) > 1.0
            && !Self::admit_traversal(
                state,
                &TraversalRequest {
                    from: state.cursor,
                    to: staging,
                    mode: TravelMode::Walk,
                    hint: None,
                    entity: None,
                },
            )?
            .admitted()
        {
            return Ok(false);
        }
        state.points.push(edge.start);
        state.points.push(edge.end);
        state.cursor = edge.end;
        state.seconds += edge.travel_seconds;
        state.prediction = self.world.begin_route(&self.graph.profile);
        Ok(true)
    }

    fn traverse_elevator_edge(
        &self,
        edge: &NavigationEdge,
        state: &mut RouteTraversal,
        mover: &NavigationEntityState,
    ) -> Result<bool, BotsError> {
        let elevator = mover.elevator.unwrap_or(ElevatorState {
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            bottom: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            top: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            phase: ElevatorPhase::Bottom,
        });
        if elevator.top.z <= elevator.bottom.z
            || elevator.top.x != elevator.bottom.x
            || elevator.top.y != elevator.bottom.y
        {
            return Ok(false);
        }
        let staging = edge.hint.map_or(edge.start, |hint| hint.funnel);
        if distance(state.cursor, edge.start)
            > self
                .graph
                .profile
                .maximum_step
                .max(self.node(edge.from).map_or(0.0, |node| node.radius))
            && distance(state.cursor, staging) > 1.0
            && !Self::admit_traversal(
                state,
                &TraversalRequest {
                    from: state.cursor,
                    to: staging,
                    mode: TravelMode::Walk,
                    hint: None,
                    entity: None,
                },
            )?
            .admitted()
        {
            return Ok(false);
        }
        state.points.push(edge.end);
        state.cursor = edge.end;
        state.seconds += edge.travel_seconds;
        state.prediction = self.world.begin_route(&self.graph.profile);
        Ok(true)
    }

    fn elevator_landing(
        runtime: &NavigationRuntime<'_>,
        edge: &NavigationEdge,
        boarding: &BoardingElevator,
        landing: Vec3,
    ) -> Vec3 {
        if boarding.platform.phase != ElevatorPhase::Bottom {
            return edge.start;
        }
        let floor = trace(
            runtime.world,
            &runtime.graph.profile,
            edge.end,
            Vec3 {
                x: edge.end.x,
                y: edge.end.y,
                z: edge.end.z - 96.0,
            },
            &QueryTarget::World,
        );
        if !floor.start_solid
            && !floor.all_solid
            && matches!(&floor.hit, crate::scene::TraceHit::Actor { actor } if actor == &boarding.actor)
            && matches!(floor.contact, crate::scene::TraceContact::Plane { plane } if f64::from(plane.normal.z) >= runtime.graph.profile.minimum_floor_normal)
        {
            return floor.end;
        }
        landing
    }

    /// Admit one traversal from an origin.
    pub fn admit_edge(&mut self, edge: &NavigationEdge, origin: Vec3) -> Result<TraversalAdmission, BotsError> {
        self.refresh();
        if !self.edge_allowed(edge, None) {
            return Ok(TraversalAdmission::Refused {
                reason: "Selected traversal is disabled or obstructed".to_string(),
            });
        }
        let mut state = self.traversal(origin);
        if !self.traverse_edge(edge, &mut state)? {
            return Ok(TraversalAdmission::Refused {
                reason: "Selected movement cannot perform this traversal".to_string(),
            });
        }
        Ok(TraversalAdmission::Admitted {
            seconds: state.seconds,
            trajectory: state.points,
        })
    }

    /// Estimate a cost query.
    pub fn estimate(&mut self, query: &NavigationEstimateQuery) -> Result<NavigationEstimateResult, BotsError> {
        let view = StaticView {
            graph: &self.graph,
            nodes: &self.nodes,
            enabled: &self.enabled,
            blocked: &self.blocked,
        };
        self.estimates.estimate(query, &|edge, flags| {
            view.static_edge_allowed(
                edge,
                Some(EligibilityQuery {
                    travel_flags: flags,
                    disabled_areas: None,
                    edge_filter: None,
                }),
            )
        })
    }

    /// Publish a movement-admitted route.
    pub fn route(&mut self, query: &NavigationRouteQuery) -> Result<NavigationRouteResult, BotsError> {
        self.refresh();
        let start = match query.start_node {
            Some(node) => Some(node),
            None => self.area_at(query.start)?,
        };
        let goal = match query.goal_node {
            Some(node) => Some(node),
            None => self.area_at(query.goal)?,
        };
        let (Some(start), Some(goal)) = (start, goal) else {
            return Ok(NavigationRouteResult::Unreachable {
                reason: "start or goal has no navigation area".to_string(),
            });
        };
        let start_node = self.node(start).cloned();
        let goal_node = self.node(goal).cloned();
        let (Some(start_node), Some(_goal_node)) = (start_node, goal_node) else {
            return Ok(NavigationRouteResult::Unreachable {
                reason: "start or goal has no navigation area".to_string(),
            });
        };
        if !self.node_allowed(&start_node, Some(query), false) {
            return Ok(NavigationRouteResult::Unreachable {
                reason: "start area is disabled or occupied".to_string(),
            });
        }
        let mut grounded: HashMap<i32, bool> = HashMap::new();
        let mut elevator: HashMap<i32, bool> = HashMap::new();
        let mut rejected: HashSet<i32> = HashSet::new();
        let maximum_searches = query.maximum_searches.unwrap_or(64);
        for _ in 0..maximum_searches {
            let path = self.candidate(start, goal, query, &rejected, &mut |node, await_elevator| {
                let memo = if await_elevator { &mut elevator } else { &mut grounded };
                if let Some(allowed) = memo.get(&node.id) {
                    return *allowed;
                }
                let allowed = self.node_allowed(node, Some(query), await_elevator);
                memo.insert(node.id, allowed);
                allowed
            })?;
            let Some(path) = path else {
                return Ok(NavigationRouteResult::Unreachable {
                    reason: "no route satisfies source flags, character capabilities, and current obstacles"
                        .to_string(),
                });
            };
            let mut traversal = self.traversal(query.start);
            let mut failed = None;
            for edge in &path {
                if !self.traverse_edge(edge, &mut traversal)? {
                    failed = Some(edge.id);
                    break;
                }
            }
            if let Some(id) = failed {
                rejected.insert(id);
                continue;
            }
            let cursor = traversal.cursor;
            if distance(cursor, query.goal) > 1.0
                && !Self::admit_traversal(
                    &mut traversal,
                    &TraversalRequest {
                        from: cursor,
                        to: query.goal,
                        mode: TravelMode::Walk,
                        hint: None,
                        entity: None,
                    },
                )?
                .admitted()
            {
                return Ok(NavigationRouteResult::Unreachable {
                    reason: "selected movement cannot reach the goal within its area".to_string(),
                });
            }
            let mut nodes = vec![start];
            let mut edges = Vec::new();
            for edge in path {
                nodes.push(edge.to);
                edges.push(edge);
            }
            return Ok(NavigationRouteResult::Route {
                route: NavigationRoute {
                    map: self.graph.map.clone(),
                    nodes,
                    edges,
                    points: traversal.points,
                    travel_seconds: traversal.seconds,
                    generation: self.generation,
                },
            });
        }
        Ok(NavigationRouteResult::Unreachable {
            reason: format!("movement admission exhausted {maximum_searches} candidate routes"),
        })
    }

    /// Whether an admitted route is still valid.
    pub fn route_still_valid(&mut self, route: &NavigationRoute) -> bool {
        self.refresh();
        if route.map.digest != self.graph.map.digest || route.generation != self.generation {
            return false;
        }
        route.edges.iter().all(|edge| self.edge_allowed(edge, None))
    }

    /// Debug lines over all edges.
    pub fn debug_lines(&mut self) -> Vec<NavigationDebugLine> {
        self.refresh();
        self.graph
            .edges
            .iter()
            .map(|edge| NavigationDebugLine {
                from: edge.start,
                to: edge.end,
                edge: edge.id,
                enabled: self.edge_allowed(edge, None),
                mode: edge.mode,
            })
            .collect()
    }

    /// Capture a serializable checkpoint.
    #[must_use]
    pub fn checkpoint(&self) -> NavigationRuntimeCheckpoint {
        let mut enabled: Vec<(i32, bool)> = self.enabled.iter().map(|(id, enabled)| (*id, *enabled)).collect();
        enabled.sort_by_key(|(id, _)| *id);
        let mut blocked: Vec<(i32, String)> = self.blocked.iter().map(|(id, reason)| (*id, reason.clone())).collect();
        blocked.sort_by_key(|(id, _)| *id);
        let mut admission_seconds: Vec<(i32, f64)> = self
            .admission_seconds
            .iter()
            .map(|(id, seconds)| (*id, *seconds))
            .collect();
        admission_seconds.sort_by_key(|(id, _)| *id);
        NavigationRuntimeCheckpoint {
            version: 1,
            map: self.graph.map.clone(),
            enabled,
            blocked,
            admission_seconds,
            world_revision: self.world_revision,
            generation: self.generation,
        }
    }

    /// Restore a checkpoint value.
    pub fn restore_checkpoint(&mut self, checkpoint: &SaveValue) -> Result<(), BotsError> {
        let reader = SaveReader::new(checkpoint, "navigation");
        reader.field("version")?.literal_int(1)?;
        let map = reader.field("map")?;
        map.field("name")?.literal_str(&self.graph.map.name)?;
        map.field("format")?.literal_str(self.graph.map.format.as_str())?;
        map.field("digest")?.literal_str(&self.graph.map.digest.text)?;
        let mut enabled = HashMap::new();
        reader.field("enabled")?.list(|entry| {
            let raw = entry.field("id")?.integer(0)?;
            let value = entry.field("enabled")?.boolean()?;
            let Ok(id) = i32::try_from(raw) else {
                return entry.fail("unknown or duplicate navigation identity");
            };
            if !self.nodes.contains_key(&id) || enabled.contains_key(&id) {
                return entry.fail("unknown or duplicate navigation identity");
            }
            enabled.insert(id, value);
            Ok(())
        })?;
        let mut blocked = HashMap::new();
        reader.field("blocked")?.list(|entry| {
            let raw = entry.field("id")?.integer(0)?;
            let reason = entry.field("reason")?.string()?.to_string();
            let Ok(id) = i32::try_from(raw) else {
                return entry.fail("unknown or duplicate navigation identity");
            };
            if !self.graph.edges.iter().any(|edge| edge.id == id) || blocked.contains_key(&id) {
                return entry.fail("unknown or duplicate navigation identity");
            }
            blocked.insert(id, reason);
            Ok(())
        })?;
        let mut admission_seconds = HashMap::new();
        reader.field("admissionSeconds")?.list(|entry| {
            let raw = entry.field("id")?.integer(0)?;
            let seconds = entry.field("seconds")?.finite()?;
            let Ok(id) = i32::try_from(raw) else {
                return entry.fail("unknown or duplicate navigation identity");
            };
            if !self.graph.edges.iter().any(|edge| edge.id == id) || admission_seconds.contains_key(&id) {
                return entry.fail("unknown or duplicate navigation identity");
            }
            if seconds < 0.0 {
                return entry.fail("negative admission duration");
            }
            admission_seconds.insert(id, seconds);
            Ok(())
        })?;
        let world_revision = reader.field("worldRevision")?.finite()? as i64;
        let generation = reader.field("generation")?.integer(0)?;
        self.enabled = enabled;
        self.blocked = blocked;
        self.admission_seconds = admission_seconds;
        self.world_revision = world_revision;
        self.generation = generation;
        self.estimates.invalidate();
        Ok(())
    }
}
