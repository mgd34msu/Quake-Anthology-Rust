//! Navigation loading integration tests.

#[path = "common/mod.rs"]
mod common;

use common::{linked_aas, q3_world, test_map, test_profile, FixtureWorld};
use qa_bots::aas_write::write_aas;
use qa_bots::construct::NavigationConstruction;
use qa_bots::content::{ContentId, NavigationResources, OpenedResource, ResourceProvenance, ResourceReference};
use qa_bots::load::{load_navigation, preload_navigation, NavigationLoadOptions, PreloadOptions};
use qa_bots::md4::block_checksum;
use qa_content::contract::ResourceIdentity;

const MAP_BYTES: &[u8] = b"fixture bsp bytes";

fn reference(path: &str, content: &str, bytes: &[u8]) -> OpenedResource {
    OpenedResource {
        reference: ResourceReference {
            requested_path: path.to_string(),
            provenance: ResourceProvenance {
                mount_content: ContentId::new(content),
            },
            identity: ResourceIdentity::parse("identity:0:0:16:0").unwrap(),
            byte_length: bytes.len(),
        },
        bytes: bytes.to_vec(),
    }
}

struct EmptyResources;

impl NavigationResources for EmptyResources {
    fn open(&self, _path: &str) -> Option<OpenedResource> {
        None
    }
}

struct AasResources {
    bytes: Vec<u8>,
}

impl AasResources {
    fn new() -> Self {
        let mut asset = linked_aas();
        asset.bsp_checksum = block_checksum(MAP_BYTES).unwrap() as i32;
        Self {
            bytes: write_aas(&asset).unwrap(),
        }
    }
}

impl NavigationResources for AasResources {
    fn open(&self, path: &str) -> Option<OpenedResource> {
        if path == "maps/test.aas" {
            Some(reference(path, "geometry:base:test:q3", &self.bytes))
        } else {
            None
        }
    }
}

struct NavResources {
    content: String,
}

impl NavigationResources for NavResources {
    fn open(&self, path: &str) -> Option<OpenedResource> {
        if path == "bots/navigation/test.nav" {
            let bytes = b"not a nav file".to_vec();
            Some(reference(path, &self.content.clone(), &bytes))
        } else {
            None
        }
    }
}

#[test]
fn preload_selects_aas_with_checksum() {
    let resources = AasResources::new();
    let map = test_map();
    let prepared = preload_navigation(&PreloadOptions {
        map: &map,
        resources: &resources,
        map_bytes: MAP_BYTES,
        navigation_content: None,
    })
    .unwrap();
    assert!(prepared.asset.is_some());
    assert!(prepared.resource.is_some());
}

#[test]
fn preload_rejects_checksum_mismatch() {
    let resources = AasResources::new();
    let map = test_map();
    let result = preload_navigation(&PreloadOptions {
        map: &map,
        resources: &resources,
        map_bytes: b"other bytes",
        navigation_content: None,
    });
    assert!(result.is_err());
}

#[test]
fn preload_skips_foreign_nav_content() {
    let resources = NavResources {
        content: "other:content".to_string(),
    };
    let map = test_map();
    let expected = ContentId::new("geometry:base:test:q3");
    let prepared = preload_navigation(&PreloadOptions {
        map: &map,
        resources: &resources,
        map_bytes: MAP_BYTES,
        navigation_content: Some(&expected),
    })
    .unwrap();
    assert!(prepared.asset.is_none());
}

#[test]
fn preload_rejects_escaping_names() {
    let resources = EmptyResources;
    let mut map = test_map();
    map.name = "maps/../escape".to_string();
    let result = preload_navigation(&PreloadOptions {
        map: &map,
        resources: &resources,
        map_bytes: MAP_BYTES,
        navigation_content: None,
    });
    assert!(result.is_err());
}

#[test]
fn load_constructs_without_assets() {
    let world = FixtureWorld::new();
    let geometry = q3_world();
    let map = test_map();
    let profile = test_profile();
    let resources = EmptyResources;
    let loaded = load_navigation(&NavigationLoadOptions {
        construction: NavigationConstruction {
            geometry: &geometry,
            map: &map,
            profile: &profile,
            world: &world,
            spacing: None,
            link_distance: None,
            maximum_nodes: None,
            connections: None,
        },
        resources: &resources,
        map_bytes: MAP_BYTES,
        navigation_content: None,
    })
    .unwrap();
    assert!(!loaded.runtime.graph.nodes.is_empty());
    assert!(loaded.resource.is_none());
}

#[test]
fn load_uses_stored_aas() {
    let world = FixtureWorld::new();
    let geometry = q3_world();
    let map = test_map();
    let profile = test_profile();
    let resources = AasResources::new();
    let loaded = load_navigation(&NavigationLoadOptions {
        construction: NavigationConstruction {
            geometry: &geometry,
            map: &map,
            profile: &profile,
            world: &world,
            spacing: None,
            link_distance: None,
            maximum_nodes: None,
            connections: None,
        },
        resources: &resources,
        map_bytes: MAP_BYTES,
        navigation_content: None,
    })
    .unwrap();
    assert_eq!(loaded.runtime.graph.nodes.len(), 2);
    assert_eq!(loaded.runtime.graph.edges.len(), 2);
    assert!(loaded.resource.is_some());
}
