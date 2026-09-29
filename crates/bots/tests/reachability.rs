//! AAS reachability construction integration tests.

#[path = "common/mod.rs"]
mod common;

use common::{bare_aas, linked_aas, q3_world, test_profile, FixtureWorld};
use qa_bots::aas_reachability::{build_aas_reachability, AasLinkedReachability, AasReachabilityOptions};
use qa_bots::aas_reachability_geometry::{
    aas_area_ground_face_area, aas_area_volume, aas_face_area, aas_face_center, aas_fall_damage_distance,
    aas_fall_delta, aas_max_jump_height,
};
use qa_bots::aas_reachability_spatial::AasReachabilityEntities;
use qa_bots::aas_reachability_types::{init_aas_movement_settings, AasMovementSettings};
use qa_bots::behavior::{BotMovementPrediction, BotTravelPredictionResult};
use qa_core::math::vec3;

fn stub_prediction(prediction: BotMovementPrediction) -> BotTravelPredictionResult {
    BotTravelPredictionResult {
        end: prediction.origin,
        velocity: vec3(0.0, 0.0, 0.0),
        frames: 0,
        stop_event: 0,
        end_area: None,
    }
}

#[test]
fn build_links_walkable_neighbors() {
    let world = FixtureWorld::new();
    let asset = bare_aas();
    let geometry = q3_world();
    let profile = test_profile();
    let built = build_aas_reachability(&AasReachabilityOptions {
        asset: &asset,
        geometry: &geometry,
        scene: &world.scene,
        profile: &profile,
        prediction_client: 0,
        predict_client_movement: &stub_prediction,
        force: true,
        debug: false,
        settings: None,
        variable: None,
        print: None,
        debug_line: None,
    })
    .unwrap();
    assert_eq!(built.areas.len(), asset.areas.len());
    assert!(built.reachability.len() > 1);
    let areas: Vec<i32> = built.reachability.iter().skip(1).map(|link| link.area).collect();
    assert!(areas.contains(&1));
    assert!(areas.contains(&2));
}

#[test]
fn build_keeps_existing_reachability_without_force() {
    let world = FixtureWorld::new();
    let asset = linked_aas();
    let geometry = q3_world();
    let profile = test_profile();
    let kept = build_aas_reachability(&AasReachabilityOptions {
        asset: &asset,
        geometry: &geometry,
        scene: &world.scene,
        profile: &profile,
        prediction_client: 0,
        predict_client_movement: &stub_prediction,
        force: false,
        debug: false,
        settings: None,
        variable: None,
        print: None,
        debug_line: None,
    })
    .unwrap();
    assert_eq!(kept.reachability, asset.reachability);
}

#[test]
fn linked_reachability_truncates_travel_time() {
    let mut link = AasLinkedReachability::zero();
    assert_eq!(link.travel_time, 0);
    link.set_travel_time(150.9).unwrap();
    assert_eq!(link.travel_time, 150);
    assert!(link.set_travel_time(3.0e9).is_err());
    link.clear();
    assert_eq!(link, AasLinkedReachability::zero());
}

#[test]
fn movement_settings_initialize_from_defaults() {
    let mut settings = AasMovementSettings::default();
    let defaults = |_name: &str, default: &str| default.parse::<f64>().unwrap_or(0.0);
    init_aas_movement_settings(&defaults, &mut settings);
    assert_eq!(settings.gravity, 800.0);
    assert_eq!(settings.friction, 6.0);
    assert_eq!(settings.gravity_direction, vec3(0.0, 0.0, -1.0));
}

#[test]
fn reachability_entities_read_source_epairs() {
    let entities =
        AasReachabilityEntities::new("{ \"classname\" \"worldspawn\" \"count\" \"3\" \"origin\" \"1 2 3\" }").unwrap();
    assert_eq!(entities.next_entity(0), 1);
    assert_eq!(entities.next_entity(1), 0);
    assert_eq!(entities.int(1, "count").unwrap(), (true, 3));
    let (found, origin) = entities.vector(1, "origin").unwrap();
    assert!(found);
    assert_eq!(origin, vec3(1.0, 2.0, 3.0));
}

#[test]
fn geometry_measures_ground_faces() {
    let asset = bare_aas();
    let area = aas_face_area(&asset, &asset.faces[1]).unwrap();
    assert!((area - 4096.0).abs() < 1.0);
    let volume = aas_area_volume(&asset, 1).unwrap();
    // The open single-face area has no enclosed volume: the donor's
    // corner fan is coplanar with its only face.
    assert_eq!(volume, 0.0);
    let ground = aas_area_ground_face_area(&asset, 1).unwrap();
    assert!((ground - 4096.0).abs() < 1.0);
    let center = aas_face_center(&asset, 1).unwrap();
    assert!((f64::from(center.x) - 32.0).abs() < 1.0);
    assert!((f64::from(center.y) - 32.0).abs() < 1.0);
}

#[test]
fn fall_and_jump_helpers_follow_settings() {
    let mut settings = AasMovementSettings::default();
    let defaults = |_name: &str, default: &str| default.parse::<f64>().unwrap_or(0.0);
    init_aas_movement_settings(&defaults, &mut settings);
    assert!(aas_fall_damage_distance(&settings).unwrap() > 0);
    assert!(aas_fall_delta(&settings, 100.0) >= 0.0);
    assert!(aas_max_jump_height(&settings, 270.0) > 0.0);
}
