//! Runtime routing, estimates, and rerelease-path integration tests.

#[path = "common/mod.rs"]
mod common;

use std::collections::HashSet;

use common::{linked_aas, linked_graph, FixtureWorld};
use qa_bots::estimate_aas::aas_estimate_area_time;
use qa_bots::estimates::{Eligibility, NavigationEstimates};
use qa_bots::rerelease_path::{rerelease_path_to_goal, RereleasePathRequest};
use qa_bots::runtime::{NavigationRouteQuery, NavigationRuntime};
use qa_bots::types::{NavigationEstimateQuery, NavigationEstimateResult, NavigationRouteResult};
use qa_core::math::{vec3, Bounds};

fn query() -> NavigationRouteQuery {
    NavigationRouteQuery {
        start: vec3(32.0, 32.0, 32.0),
        goal: vec3(32.0, -32.0, 32.0),
        start_node: None,
        goal_node: None,
        travel_flags: None,
        disabled_areas: HashSet::new(),
        edge_filter: None,
        maximum_searches: None,
    }
}

fn runtime(world: &FixtureWorld) -> NavigationRuntime<'_> {
    NavigationRuntime::new(linked_graph(world), world).unwrap()
}

#[test]
fn route_links_both_areas() {
    let world = FixtureWorld::new();
    let mut runtime = runtime(&world);
    let result = runtime.route(&query()).unwrap();
    let NavigationRouteResult::Route { route } = result else {
        panic!("expected a route");
    };
    assert_eq!(route.nodes, vec![1, 2]);
    assert_eq!(route.edges.len(), 1);
    assert!(route.travel_seconds > 0.0);
    assert!(!route.points.is_empty());
}

#[test]
fn route_respects_disabled_areas() {
    let world = FixtureWorld::new();
    let mut runtime = runtime(&world);
    assert!(runtime.enable_area(2, false).unwrap());
    let result = runtime.route(&query()).unwrap();
    assert!(matches!(result, NavigationRouteResult::Unreachable { .. }));
}

#[test]
fn route_respects_blocked_edges() {
    let world = FixtureWorld::new();
    let mut runtime = runtime(&world);
    let edge = runtime.graph.edges[0].id;
    runtime.block_edge(edge, Some("test")).unwrap();
    let result = runtime.route(&query()).unwrap();
    assert!(matches!(result, NavigationRouteResult::Unreachable { .. }));
}

#[test]
fn route_still_valid_tracks_topology() {
    let world = FixtureWorld::new();
    let mut runtime = runtime(&world);
    let NavigationRouteResult::Route { route } = runtime.route(&query()).unwrap() else {
        panic!("expected a route");
    };
    assert!(runtime.route_still_valid(&route));
    let edge = runtime.graph.edges[0].id;
    runtime.block_edge(edge, None).unwrap();
    assert!(!runtime.route_still_valid(&route));
}

#[test]
fn checkpoint_restores_disabled_areas() {
    let world = FixtureWorld::new();
    let mut runtime = runtime(&world);
    let saved = runtime.checkpoint().to_save_value();
    runtime.enable_area(2, false).unwrap();
    assert!(matches!(
        runtime.route(&query()).unwrap(),
        NavigationRouteResult::Unreachable { .. }
    ));
    runtime.restore_checkpoint(&saved).unwrap();
    assert!(matches!(
        runtime.route(&query()).unwrap(),
        NavigationRouteResult::Route { .. }
    ));
}

#[test]
fn runtime_queries_cover_lookup_helpers() {
    let world = FixtureWorld::new();
    let mut runtime = runtime(&world);
    assert_eq!(runtime.area_at(vec3(32.0, 32.0, 32.0)).unwrap(), Some(1));
    assert_eq!(runtime.area_at(vec3(32.0, -32.0, 32.0)).unwrap(), Some(2));
    let nearest = runtime.nearest(vec3(30.0, 30.0, 30.0), 100.0).unwrap();
    assert_eq!(nearest.id, 1);
    let bounds = Bounds {
        min: vec3(0.0, -64.0, 0.0),
        max: vec3(64.0, 64.0, 64.0),
    };
    let areas = runtime.bbox_areas(&bounds).unwrap();
    assert!(areas.contains(&1) && areas.contains(&2));
    let crossings = runtime
        .trace_areas(vec3(32.0, 32.0, 32.0), vec3(32.0, -32.0, 32.0), 20)
        .unwrap();
    assert!(crossings.iter().any(|crossing| crossing.area == 2));
    assert!(runtime.node(1).is_some());
    assert_eq!(runtime.outgoing(1).len(), 1);
    let lines = runtime.debug_lines();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|line| line.enabled));
}

#[test]
fn estimates_price_linked_areas() {
    let world = FixtureWorld::new();
    let graph = linked_graph(&world);
    let mut estimates = NavigationEstimates::new(&graph).unwrap();
    let allowed: Box<Eligibility<'_>> = Box::new(|_, _| true);
    let result = estimates
        .estimate(
            &NavigationEstimateQuery {
                start_node: 1,
                goal_node: 2,
                origin: None,
                travel_flags: None,
            },
            &allowed,
        )
        .unwrap();
    let NavigationEstimateResult::Estimate {
        travel_time,
        first_edge,
    } = result
    else {
        panic!("expected an estimate");
    };
    assert!(travel_time >= 1);
    assert!(first_edge.is_none());
    let missing = estimates
        .estimate(
            &NavigationEstimateQuery {
                start_node: 1,
                goal_node: 99,
                origin: None,
                travel_flags: None,
            },
            &allowed,
        )
        .unwrap();
    assert_eq!(missing, NavigationEstimateResult::Unreachable);
}

#[test]
fn aas_area_time_scales_with_distance() {
    let asset = linked_aas();
    let near = aas_estimate_area_time(&asset.settings[1], vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0)).unwrap();
    let far = aas_estimate_area_time(&asset.settings[1], vec3(0.0, 0.0, 0.0), vec3(100.0, 0.0, 0.0)).unwrap();
    assert!(near >= 1);
    assert!(far > near);
}

#[test]
fn rerelease_path_reports_source_codes() {
    let request = RereleasePathRequest {
        start: vec3(32.0, 32.0, 32.0),
        goal: vec3(32.0, -32.0, 32.0),
        flags: 3,
        move_distance: 8.0,
        ignore_node_flags: false,
        min_height: 0.0,
        max_height: 100.0,
        radius: 200.0,
        drop_height: 64.0,
        jump_height: 32.0,
    };
    let missing = rerelease_path_to_goal(None, &request).unwrap();
    assert_eq!(missing.code, 8);

    let world = FixtureWorld::new();
    let runtime = runtime(&world);
    let mut blocked = request;
    blocked.flags = 0;
    assert_eq!(rerelease_path_to_goal(Some(&runtime), &blocked).unwrap().code, 12);

    let info = rerelease_path_to_goal(Some(&runtime), &request).unwrap();
    assert_eq!(info.code, 4);
    assert!(!info.points.is_empty());
}
