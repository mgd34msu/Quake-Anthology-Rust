//! AAS parse/write/query/optimize/cluster/prediction-stop integration tests.

#[path = "common/mod.rs"]
mod common;

use common::linked_aas;
use qa_bots::aas::{aas_bbox_areas, aas_point_area, aas_trace_areas, parse_aas, AasAsset};
use qa_bots::aas_cluster::cluster_aas;
use qa_bots::aas_optimize::optimize_aas;
use qa_bots::aas_prediction_stop::aas_prediction_stop;
use qa_bots::aas_write::write_aas;
use qa_bots::behavior::BotMovementStop;
use qa_bots::md4::block_checksum;
use qa_core::math::{vec3, Bounds};

fn reparsed(asset: &AasAsset) -> AasAsset {
    let bytes = write_aas(asset).unwrap();
    parse_aas(&bytes, "roundtrip", None).unwrap()
}

#[test]
fn write_parse_roundtrip_preserves_topology() {
    let asset = linked_aas();
    let mut decoded = reparsed(&asset);
    assert_eq!(decoded.version, asset.version);
    assert_eq!(decoded.bsp_checksum, asset.bsp_checksum);
    assert_eq!(decoded.areas.len(), asset.areas.len());
    assert_eq!(decoded.settings.len(), asset.settings.len());
    assert_eq!(decoded.reachability.len(), asset.reachability.len());
    assert_eq!(decoded.nodes.len(), asset.nodes.len());
    assert_eq!(decoded.clusters.len(), asset.clusters.len());
    assert_eq!(decoded.source, "roundtrip");
    assert!(!decoded.lumps.is_empty());
    decoded.source = asset.source.clone();
    decoded.lumps.clone_from(&asset.lumps);
    assert_eq!(decoded, asset);
}

#[test]
fn parse_verifies_bsp_checksum() {
    let bytes = write_aas(&linked_aas()).unwrap();
    assert!(parse_aas(&bytes, "checksum", Some(1234)).is_ok());
    assert!(parse_aas(&bytes, "checksum", Some(999)).is_err());
}

#[test]
fn parse_rejects_bad_magic_and_version() {
    let mut bytes = write_aas(&linked_aas()).unwrap();
    bytes[0] = 0x00;
    assert!(parse_aas(&bytes, "magic", None).is_err());

    let mut bytes = write_aas(&linked_aas()).unwrap();
    bytes[4..8].copy_from_slice(&7i32.to_le_bytes());
    assert!(parse_aas(&bytes, "version", None).is_err());
}

#[test]
fn parse_rejects_truncated_input() {
    let bytes = write_aas(&linked_aas()).unwrap();
    assert!(parse_aas(&bytes[..12], "truncated", None).is_err());
}

#[test]
fn point_area_resolves_both_sides() {
    let asset = linked_aas();
    assert_eq!(aas_point_area(&asset, vec3(32.0, 32.0, 32.0)).unwrap(), 1);
    assert_eq!(aas_point_area(&asset, vec3(32.0, -32.0, 32.0)).unwrap(), 2);
}

#[test]
fn bbox_areas_span_both_areas() {
    let asset = linked_aas();
    let bounds = Bounds {
        min: vec3(0.0, -64.0, 0.0),
        max: vec3(64.0, 64.0, 64.0),
    };
    let areas = aas_bbox_areas(&asset, bounds, 16).unwrap();
    assert!(areas.contains(&1));
    assert!(areas.contains(&2));
}

#[test]
fn trace_areas_reports_crossing() {
    let asset = linked_aas();
    let crossings = aas_trace_areas(&asset, vec3(32.0, 32.0, 32.0), vec3(32.0, -32.0, 32.0), 20).unwrap();
    let areas: Vec<i32> = crossings.iter().map(|crossing| crossing.area).collect();
    assert!(areas.contains(&1));
    assert!(areas.contains(&2));
}

#[test]
fn optimize_preserves_areas() {
    let optimized = optimize_aas(&linked_aas()).unwrap();
    assert_eq!(optimized.areas.len(), 3);
    assert_eq!(optimized.settings.len(), 3);
    assert_eq!(optimized.reachability.len(), 3);
}

#[test]
fn cluster_assigns_portals_and_clusters() {
    let clustered = cluster_aas(&linked_aas(), None).unwrap();
    assert_eq!(clustered.settings.len(), 3);
    assert!(!clustered.clusters.is_empty());
    assert_eq!(clustered.areas.len(), 3);
}

#[test]
fn prediction_stop_ignores_unmasked_events() {
    let asset = linked_aas();
    let stop = aas_prediction_stop(&asset, vec3(32.0, 32.0, 32.0), vec3(32.0, -32.0, 32.0), 4, 0, 2).unwrap();
    assert_eq!(stop, None);
}

#[test]
fn prediction_stop_fires_on_stop_area() {
    let asset = linked_aas();
    let stop = aas_prediction_stop(&asset, vec3(32.0, 32.0, 32.0), vec3(32.0, -32.0, 32.0), 4, 512, 2).unwrap();
    assert_eq!(
        stop,
        Some(BotMovementStop {
            events: 512,
            origin: stop.unwrap().origin,
            area: Some(2),
        })
    );
}

#[test]
fn prediction_stop_fires_on_slime_contents() {
    let mut asset = linked_aas();
    asset.settings[2].contents = 64;
    let stop = aas_prediction_stop(&asset, vec3(32.0, 32.0, 32.0), vec3(32.0, -32.0, 32.0), 4, 256, 0).unwrap();
    assert!(stop.is_some());
    assert_eq!(stop.unwrap().events, 256);
}

#[test]
fn block_checksum_matches_writer_input() {
    let bytes = write_aas(&linked_aas()).unwrap();
    assert_eq!(block_checksum(&bytes).unwrap(), block_checksum(&bytes).unwrap());
}
