//! Foundation-module integration tests: errors, scene, content, behavior,
//! entities, saves, helpers, types, digests, and movement contracts.

#[path = "common/mod.rs"]
mod common;

use common::{q1_world, q2_world, q3_world, test_body, test_profile, FloorScene};
use qa_bots::content::{ContentDigest, ContentId};
use qa_bots::entities::parse_entities;
use qa_bots::helpers::{distance, midpoint, translated, validate_profile};
use qa_bots::md4::{block_checksum, md4};
use qa_bots::movement_contract::{MovementKind, MovementState};
use qa_bots::save::{SaveReader, SaveValue};
use qa_bots::scene::{
    DecodedWorld, PointContentsQuery, QueryTarget, SceneQueries, TracePolicy, TraceQuery, TraceShape, VisibilityKind,
    WorldKind,
};
use qa_bots::types::TravelMode;
use qa_bots::BotsError;
use qa_core::math::{vec3, Bounds};
use qa_core::numeric::Q3_BINARY32_PROFILE;

#[test]
fn errors_render_and_wrap() {
    assert!(!BotsError::BadResourcePath.to_string().is_empty());
    assert!(!BotsError::MapMismatch.to_string().is_empty());
    let wrapped: BotsError = qa_core::binary::BinaryError::custom("test", 0, "boom").into();
    assert!(wrapped.to_string().contains("boom"));
}

#[test]
fn floor_scene_trace_hits_and_misses() {
    let scene = FloorScene { floor_z: 0.0 };
    let policy = TracePolicy::Q3 {
        contents_mask: -1,
        curves: true,
        player_curve_clip: true,
    };
    let falling = scene.trace(&TraceQuery {
        start: vec3(0.0, 0.0, 64.0),
        end: vec3(0.0, 0.0, -64.0),
        shape: TraceShape::Point,
        target: QueryTarget::World,
        policy,
        numeric: Q3_BINARY32_PROFILE,
        pass_actor: None,
    });
    assert!(falling.fraction < 1.0);
    assert_eq!(falling.end.z, 0.0);
    assert!(!falling.start_solid);

    let airborne = scene.trace(&TraceQuery {
        start: vec3(0.0, 0.0, 64.0),
        end: vec3(0.0, 0.0, 32.0),
        shape: TraceShape::Point,
        target: QueryTarget::World,
        policy,
        numeric: Q3_BINARY32_PROFILE,
        pass_actor: None,
    });
    assert_eq!(airborne.fraction, 1.0);
}

#[test]
fn floor_scene_answers_auxiliary_queries() {
    let scene = FloorScene { floor_z: 0.0 };
    let contents = scene.point_contents(&PointContentsQuery {
        point: vec3(0.0, 0.0, 8.0),
        target: QueryTarget::World,
        policy: TracePolicy::Q3 {
            contents_mask: -1,
            curves: true,
            player_curve_clip: true,
        },
        numeric: Q3_BINARY32_PROFILE,
        pass_actor: None,
    });
    assert!(format!("{contents:?}").contains('0'));
    let bounds = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(1.0, 1.0, 1.0),
    };
    let leaves = scene.box_leaves(&bounds, 8);
    assert!(leaves.leaves.is_empty());
    assert!(scene.areas_connected(1, 2));
    assert!(scene.cluster_visible(1, 2, VisibilityKind::Pvs));
}

#[test]
fn decoded_worlds_report_kind_and_entities() {
    assert_eq!(q1_world().kind(), WorldKind::Q1Bsp);
    assert_eq!(q2_world().kind(), WorldKind::Q2Bsp);
    assert_eq!(q3_world().kind(), WorldKind::Q3Bsp);
    let DecodedWorld::Q3(world) = q3_world() else {
        panic!("expected Q3 geometry");
    };
    assert_eq!(world.surfaces.len(), 1);
}

#[test]
fn content_identities_roundtrip() {
    assert_eq!(ContentId::new("a").text, "a");
    assert_eq!(ContentDigest::new("sha256:x").text, "sha256:x");
}

#[test]
fn entities_parse_records() {
    let parsed = parse_entities(
        "{ \"classname\" \"worldspawn\" }\n{ \"classname\" \"info_player_start\" \"origin\" \"1 2 3\" }",
        "fixture",
    )
    .unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[1].get("origin").unwrap(), "1 2 3");
}

#[test]
fn saves_read_typed_fields() {
    let value = SaveValue::map(vec![
        ("name", SaveValue::Str("nav".to_string())),
        ("areas", SaveValue::Int(2)),
        ("enabled", SaveValue::Bool(true)),
    ]);
    let reader = SaveReader::new(&value, "root");
    assert_eq!(reader.field("name").unwrap().string().unwrap(), "nav");
    assert_eq!(reader.field("areas").unwrap().integer(0).unwrap(), 2);
    assert!(reader.field("enabled").unwrap().boolean().unwrap());
    assert!(reader.field("missing").is_err());
}

#[test]
fn helpers_measure_space() {
    assert_eq!(distance(vec3(0.0, 0.0, 0.0), vec3(3.0, 4.0, 0.0)), 5.0);
    assert_eq!(midpoint(vec3(0.0, 0.0, 0.0), vec3(2.0, 2.0, 2.0)), vec3(1.0, 1.0, 1.0));
    let bounds = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(1.0, 1.0, 1.0),
    };
    let moved = translated(vec3(5.0, 0.0, 0.0), bounds);
    assert_eq!(moved.min, vec3(5.0, 0.0, 0.0));
    validate_profile(&test_profile()).unwrap();
    assert_eq!(test_body().bounds().max.z, 32.0);
}

#[test]
fn travel_modes_name_themselves() {
    assert_eq!(TravelMode::Walk.as_str(), "walk");
    assert_eq!(TravelMode::WaterJump.as_str(), "water-jump");
    assert_eq!(TravelMode::Unknown.as_str(), "unknown");
}

#[test]
fn digests_are_deterministic() {
    assert_eq!(md4(b"abc").unwrap(), md4(b"abc").unwrap());
    assert_eq!(block_checksum(b"").unwrap(), 0xc6f6_40b7);
    assert_ne!(md4(b"a").unwrap(), md4(b"b").unwrap());
}

#[test]
fn movement_states_report_kind_and_origin() {
    let state = MovementState::Q3 {
        origin: vec3(1.0, 2.0, 3.0),
    };
    assert_eq!(state.kind(), MovementKind::Q3);
    assert_eq!(state.origin(), vec3(1.0, 2.0, 3.0));
    assert_eq!(MovementKind::Q3.family(), WorldKind::Q3Bsp);
    assert_eq!(MovementKind::Q1Netquake.family(), WorldKind::Q1Bsp);
    assert_eq!(MovementKind::Q2Classic.family(), WorldKind::Q2Bsp);
}
