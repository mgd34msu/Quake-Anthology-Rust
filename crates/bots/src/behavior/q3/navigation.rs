//! Source bot navigation from `src/bots/behavior/q3/navigation.ts`.
//!
//! Adapts the shared [`NavigationRuntime`](crate::runtime::NavigationRuntime)
//! to the [`BotNavigation`] query surface: area lookups, travel times,
//! routes, route prediction, alternative goals, and movement driving.
//! Source areas are 1-based node ids (node + 1); area 0 means none.

use std::collections::HashSet;

use qa_core::math::{Bounds, Vec3};

use crate::behavior::library::goals::BotGoal;
use crate::behavior::q3::movement_state::{BotMoveResult, BotMoveStateStore};
use crate::behavior::q3::navigation_types::{
    AlternativeGoal, AlternativeRouteQuery, AreaTravelTimeQuery, BotMovementPrediction, BotNavigation,
    BotNavigationArea, PredictRouteQuery, PredictedRoute, RouteQuery, RouteResult, RouteStopEvent,
};
use crate::behavior::q3::travel::controller::{move_to_goal, TravelStep};
use crate::behavior::q3::travel::routing::{movement_view_target, predict_visible_position};
use crate::behavior::q3::travel::special::MoverObservation;
use crate::runtime::{NavigationRouteQuery, NavigationRuntime};
use crate::types::{NavigationEstimateQuery, NavigationEstimateResult, NavigationRouteResult};

/// Source area offset: source area = node + 1.
pub const SOURCE_AREA_OFFSET: i32 = 1;

/// Convert a source area to a node id.
#[must_use]
pub fn node_for_area(area: i32) -> Option<i32> {
    if area <= 0 {
        None
    } else {
        Some(area - SOURCE_AREA_OFFSET)
    }
}

/// Convert a node id to a source area.
#[must_use]
pub fn area_for_node(node: i32) -> i32 {
    node + SOURCE_AREA_OFFSET
}

/// Source bot navigation over a shared runtime.
pub struct SourceBotNavigation<'w> {
    runtime: NavigationRuntime<'w>,
    move_states: BotMoveStateStore,
    time_seconds: f32,
}

impl<'w> SourceBotNavigation<'w> {
    /// New navigation over a runtime.
    pub fn new(runtime: NavigationRuntime<'w>) -> Self {
        Self {
            runtime,
            move_states: BotMoveStateStore::new(),
            time_seconds: 0.0,
        }
    }

    /// Borrow the runtime.
    #[must_use]
    pub fn runtime(&self) -> &NavigationRuntime<'w> {
        &self.runtime
    }

    /// Mutably borrow the runtime.
    pub fn runtime_mut(&mut self) -> &mut NavigationRuntime<'w> {
        &mut self.runtime
    }

    /// Borrow the move states.
    #[must_use]
    pub fn move_states(&self) -> &BotMoveStateStore {
        &self.move_states
    }

    /// Mutably borrow the move states.
    pub fn move_states_mut(&mut self) -> &mut BotMoveStateStore {
        &mut self.move_states
    }

    /// Set the navigation clock.
    pub fn set_time(&mut self, time_seconds: f32) {
        self.time_seconds = time_seconds;
    }

    fn estimate(&mut self, query: &AreaTravelTimeQuery) -> i32 {
        let (Some(start), Some(goal)) = (node_for_area(query.area), node_for_area(query.goal_area)) else {
            return 0;
        };
        let estimate = self.runtime.estimate(&NavigationEstimateQuery {
            start_node: start,
            goal_node: goal,
            origin: query.origin,
            travel_flags: Some(query.travel_flags),
        });
        match estimate {
            Ok(NavigationEstimateResult::Estimate { travel_time, .. }) => travel_time.max(0),
            _ => 0,
        }
    }

    fn route_inner(&mut self, query: &RouteQuery) -> RouteResult {
        let (Some(start), Some(goal)) = (node_for_area(query.area), node_for_area(query.goal_area)) else {
            return RouteResult::Unreachable;
        };
        let outcome = self.runtime.route(&NavigationRouteQuery {
            start: query.origin,
            goal: query.origin,
            start_node: Some(start),
            goal_node: Some(goal),
            travel_flags: Some(query.travel_flags),
            disabled_areas: HashSet::new(),
            edge_filter: None,
            maximum_searches: None,
        });
        match outcome {
            Ok(NavigationRouteResult::Route { route, .. }) => {
                let travel_time = (route.travel_seconds * 100.0) as i32;
                let next = route.edges.first().map(|edge| edge.id).unwrap_or(0);
                RouteResult::Found {
                    travel_time,
                    next_reachability: next,
                }
            }
            _ => RouteResult::Unreachable,
        }
    }
}

impl BotNavigation for SourceBotNavigation<'_> {
    fn ready(&self) -> bool {
        self.runtime.node(0).is_some()
    }

    fn point_area(&self, origin: Vec3) -> i32 {
        self.runtime.area_at(origin).ok().flatten().map_or(0, area_for_node)
    }

    fn reachability_area(&self, origin: Vec3, _client: i32) -> i32 {
        self.point_area(origin)
    }

    fn fuzzy_point_reachability_area(&self, origin: Vec3) -> i32 {
        if let Ok(Some(area)) = self.runtime.area_at(origin) {
            return area_for_node(area);
        }
        self.runtime
            .nearest(origin, 64.0)
            .map_or(0, |node| area_for_node(node.id))
    }

    fn area(&self, number: i32) -> BotNavigationArea {
        let node = node_for_area(number).and_then(|id| self.runtime.node(id));
        BotNavigationArea {
            contents: 0,
            flags: 0,
            presence_type: 0,
            cluster: node.map_or(-1, |_| 0),
            reachable_area_count: node.map_or(0, |node| self.runtime.outgoing(node.id).len() as i32),
        }
    }

    fn trace_areas(&self, start: Vec3, end: Vec3, maximum: usize) -> Vec<(i32, Vec3)> {
        self.runtime
            .trace_areas(start, end, maximum)
            .unwrap_or_default()
            .iter()
            .map(|crossing| (area_for_node(crossing.area), crossing.point))
            .collect()
    }

    fn bbox_areas(&self, bounds: &Bounds) -> Vec<i32> {
        self.runtime
            .bbox_areas(bounds)
            .unwrap_or_default()
            .iter()
            .map(|node| area_for_node(*node))
            .collect()
    }

    fn set_area_enabled(&mut self, area: i32, enabled: bool) {
        if let Some(node) = node_for_area(area) {
            let _ = self.runtime.enable_area(node, enabled);
        }
    }

    fn area_travel_time_to_goal(&mut self, query: &AreaTravelTimeQuery) -> i32 {
        self.estimate(query)
    }

    fn route(&mut self, query: &RouteQuery) -> RouteResult {
        self.route_inner(query)
    }

    fn predict_route(&mut self, query: &PredictRouteQuery) -> PredictedRoute {
        let route = self.route(&RouteQuery {
            area: query.area,
            origin: query.origin,
            goal_area: query.goal_area,
            travel_flags: query.travel_flags,
        });
        match route {
            RouteResult::Found { travel_time, .. } => PredictedRoute {
                succeeded: true,
                stop_event: RouteStopEvent::NONE,
                end_area: query.goal_area,
                end_contents: 0,
                end_travel_flags: query.travel_flags,
                end_position: query.origin,
                time: travel_time.min(query.maximum_time),
            },
            RouteResult::Unreachable => PredictedRoute {
                succeeded: false,
                stop_event: RouteStopEvent::NO_ROUTE,
                end_area: query.area,
                end_contents: 0,
                end_travel_flags: 0,
                end_position: query.origin,
                time: 0,
            },
        }
    }

    fn alternative_route_goals(&mut self, query: &AlternativeRouteQuery) -> Vec<AlternativeGoal> {
        let start_time = self.area_travel_time_to_goal(&AreaTravelTimeQuery {
            area: query.start_area,
            origin: None,
            goal_area: query.goal_area,
            travel_flags: query.travel_flags,
        });
        if start_time <= 0 || query.maximum_goals <= 0 {
            return Vec::new();
        }
        vec![AlternativeGoal {
            origin: query.start,
            area: query.start_area,
            start_travel_time: 0,
            goal_travel_time: start_time,
            extra_travel_time: 0,
        }]
    }

    fn move_to_goal(&mut self, result: &mut BotMoveResult, move_state: i32, goal: &BotGoal, _travel_flags: i32) {
        let mover = MoverObservation {
            model: None,
            on_mover: false,
            mover_down: false,
        };
        let time = self.time_seconds;
        let origin = self.move_states.get(move_state).map(|state| state.origin);
        let Some(origin) = origin else {
            result.failure = true;
            return;
        };
        let step = self.runtime.route(&NavigationRouteQuery {
            start: origin,
            goal: goal.origin,
            start_node: None,
            goal_node: node_for_area(goal.area),
            travel_flags: None,
            disabled_areas: HashSet::new(),
            edge_filter: None,
            maximum_searches: None,
        });
        let travel_step = match step {
            Ok(NavigationRouteResult::Route { route, .. }) => route.edges.first().map(|edge| {
                let end = route.points.get(1).copied().unwrap_or(goal.origin);
                TravelStep {
                    travel_type: edge.source_travel_type,
                    start: origin,
                    end,
                    number: edge.id,
                    entity: 0,
                    travel_time: (route.travel_seconds * 100.0) as i32,
                }
            }),
            _ => None,
        };
        if let Some(state) = self.move_states.get_mut(move_state) {
            let _ = move_to_goal(state, goal, travel_step, &mover, || 0, &[], time, result);
        } else {
            result.failure = true;
        }
    }

    fn move_in_direction(&mut self, move_state: i32, direction: Vec3, speed: f32, _move_type: i32) -> bool {
        let mut result = BotMoveResult::default();
        let Some(state) = self.move_states.get(move_state) else {
            return false;
        };
        super::travel::routing::move_in_direction(state, direction, speed, &mut result)
    }

    fn movement_view_target(
        &self,
        move_state: i32,
        goal: &BotGoal,
        _travel_flags: i32,
        look_ahead: f32,
    ) -> Option<Vec3> {
        let state = self.move_states.get(move_state)?;
        movement_view_target(state.origin, goal, look_ahead)
    }

    fn predict_visible_position(&self, origin: Vec3, _area: i32, goal: &BotGoal, _travel_flags: i32) -> Option<Vec3> {
        predict_visible_position(origin, goal, &|_, _| true)
    }

    fn swimming(&self, _origin: Vec3) -> bool {
        false
    }

    fn presence_bounds(&self, _presence: i32) -> Bounds {
        Bounds {
            min: Vec3 {
                x: -15.0,
                y: -15.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 15.0,
                y: 15.0,
                z: 32.0,
            },
        }
    }
}

impl SourceBotNavigation<'_> {
    /// Client movement prediction query (navigation half): reports the
    /// query origin as the move end; full projection lives in
    /// [`crate::behavior::prediction`].
    #[must_use]
    pub fn predict_client_movement(&self, query: &BotMovementPrediction) -> Vec3 {
        query.origin
    }
}
