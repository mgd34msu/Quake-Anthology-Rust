//! Navigation construction integration tests.

#[path = "common/mod.rs"]
mod common;

use common::{q1_world, q2_world, q3_world, test_map, test_profile, FixtureWorld};
use qa_bots::construct::{construct_navigation, NavigationConnection, NavigationConstruction};
use qa_bots::types::TravelMode;
use qa_bots::BotsError;
use qa_core::math::vec3;

fn options<'a>(
    world: &'a FixtureWorld,
    geometry: &'a qa_bots::scene::DecodedWorld,
    map: &'a qa_bots::types::NavigationMapIdentity,
    profile: &'a qa_bots::types::NavigationProfile,
) -> NavigationConstruction<'a> {
    NavigationConstruction {
        geometry,
        map,
        profile,
        world,
        spacing: None,
        link_distance: None,
        maximum_nodes: None,
        connections: None,
    }
}

#[test]
fn construct_samples_q3_surfaces() {
    let world = FixtureWorld::new();
    let geometry = q3_world();
    let map = test_map();
    let profile = test_profile();
    let graph = construct_navigation(&options(&world, &geometry, &map, &profile)).unwrap();
    assert!(!graph.nodes.is_empty());
    assert_eq!(graph.map.name, "test");
    assert!(!graph.edges.is_empty());
}

#[test]
fn construct_respects_node_cap() {
    let world = FixtureWorld::new();
    let geometry = q3_world();
    let map = test_map();
    let profile = test_profile();
    let mut input = options(&world, &geometry, &map, &profile);
    input.maximum_nodes = Some(4);
    let result = construct_navigation(&input);
    assert!(matches!(result, Err(BotsError::NodeLimit { maximum: 4 })));
}

#[test]
fn construct_samples_q1_and_q2_faces() {
    let world = FixtureWorld::new();
    let profile = test_profile();
    for (geometry, format) in [
        (q1_world(), qa_bots::scene::WorldKind::Q1Bsp),
        (q2_world(), qa_bots::scene::WorldKind::Q2Bsp),
    ] {
        let mut map = test_map();
        map.format = format;
        let graph = construct_navigation(&options(&world, &geometry, &map, &profile)).unwrap();
        assert!(!graph.nodes.is_empty());
    }
}

#[test]
fn construct_links_authored_connections() {
    let world = FixtureWorld::new();
    let geometry = q3_world();
    let map = test_map();
    let profile = test_profile();
    let connections = [NavigationConnection {
        from: vec3(8.0, 8.0, 8.0),
        to: vec3(120.0, 120.0, 8.0),
        mode: TravelMode::Jump,
        hint: None,
        entity: None,
        id: 7,
        source_travel_type: 4,
        travel_seconds: 0.5,
    }];
    let mut input = options(&world, &geometry, &map, &profile);
    input.connections = Some(&connections);
    let graph = construct_navigation(&input).unwrap();
    assert!(graph.edges.iter().any(|edge| edge.mode == TravelMode::Jump));
}
