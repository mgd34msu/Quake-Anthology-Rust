//! Graph vocabulary and asset projection integration tests.

#[path = "common/mod.rs"]
mod common;

use common::{bare_aas, linked_aas, test_map, test_profile, FixtureWorld};
use qa_bots::graph::{
    aas_area_travel_flags, aas_travel_flag, aas_travel_mode, kex_travel_mode, navigation_clusters,
    navigation_edge_travel_flag, navigation_from_asset,
};
use qa_bots::types::{NavigationAsset, TravelMode};

#[test]
fn aas_modes_follow_source_table() {
    assert_eq!(aas_travel_mode(2), TravelMode::Walk);
    assert_eq!(aas_travel_mode(3), TravelMode::Crouch);
    assert_eq!(aas_travel_mode(4), TravelMode::Jump);
    assert_eq!(aas_travel_mode(6), TravelMode::Ladder);
    assert_eq!(aas_travel_mode(7), TravelMode::Drop);
    assert_eq!(aas_travel_mode(8), TravelMode::Swim);
    assert_eq!(aas_travel_mode(10), TravelMode::Teleport);
    assert_eq!(aas_travel_mode(18), TravelMode::JumpPad);
    assert_eq!(aas_travel_mode(0), TravelMode::Unknown);
    assert_eq!(aas_travel_mode(99), TravelMode::Unknown);
}

#[test]
fn kex_modes_follow_source_table() {
    assert_eq!(kex_travel_mode(0), TravelMode::Walk);
    assert_eq!(kex_travel_mode(1), TravelMode::Jump);
    assert_eq!(kex_travel_mode(2), TravelMode::Teleport);
    assert_eq!(kex_travel_mode(3), TravelMode::Drop);
    assert_eq!(kex_travel_mode(-1), TravelMode::Unknown);
    assert_eq!(kex_travel_mode(99), TravelMode::Unknown);
}

#[test]
fn aas_travel_flags_preserve_source_bits() {
    assert_eq!(aas_travel_flag(0), 1);
    assert_eq!(aas_travel_flag(2), 2);
    assert_eq!(aas_travel_flag(7), 1 << 7);
    assert_eq!(aas_travel_flag(19), 0x0100_0000);
    assert_eq!(aas_travel_flag(20), 1);
}

#[test]
fn area_flags_follow_contents() {
    let asset = linked_aas();
    let flags = aas_area_travel_flags(&asset.settings[1]);
    assert_eq!(flags, 0x0008_0000);
}

#[test]
fn bare_asset_projects_nodes_without_edges() {
    let world = FixtureWorld::new();
    let graph = navigation_from_asset(
        test_map(),
        NavigationAsset::Aas(Box::new(bare_aas())),
        test_profile(),
        &world,
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert!(graph.edges.is_empty());
    assert_eq!(graph.clusters.len(), 2);
    assert!(graph.rejected.is_empty());
}

#[test]
fn linked_asset_projects_bidirectional_edges() {
    let world = FixtureWorld::new();
    let graph = navigation_from_asset(
        test_map(),
        NavigationAsset::Aas(Box::new(linked_aas())),
        test_profile(),
        &world,
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 2);
    assert_eq!(graph.edges[0].mode, TravelMode::Walk);
    assert_eq!(graph.clusters.len(), 1);
    assert_eq!(graph.map.name, "test");
}

#[test]
fn edge_flag_matches_source_travel_type() {
    let world = FixtureWorld::new();
    let graph = navigation_from_asset(
        test_map(),
        NavigationAsset::Aas(Box::new(linked_aas())),
        test_profile(),
        &world,
    )
    .unwrap();
    for edge in &graph.edges {
        assert_eq!(
            navigation_edge_travel_flag(edge),
            aas_travel_flag(edge.source_travel_type)
        );
    }
}

#[test]
fn clusters_treat_drops_as_directed() {
    let world = FixtureWorld::new();
    let mut graph = navigation_from_asset(
        test_map(),
        NavigationAsset::Aas(Box::new(linked_aas())),
        test_profile(),
        &world,
    )
    .unwrap();
    graph.edges.truncate(1);
    graph.edges[0].mode = TravelMode::Drop;
    let clusters = navigation_clusters(&graph.nodes, &graph.edges);
    assert_eq!(clusters.len(), 2);
}
