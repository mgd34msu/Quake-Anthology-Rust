//! Application Quake III snapshot visibility selection.
//!
//! Port of `src/app/bootstrap/q3-client/visibility.ts`
//! (`selectApplicationQ3Snapshot`). Entity and player state reuse
//! [`Q3EntityState`](qa_net::q3_net::Q3EntityState) and
//! [`Q3PlayerState`](qa_net::q3_net::Q3PlayerState); selection reuses
//! [`select_q3_snapshot_entities`](qa_net::q3_visibility::select_q3_snapshot_entities)
//! through a local bindings adapter. Collision queries are a minimal
//! absorbed `SharedSceneQueries` pick (see [`ApplicationQ3SceneQueries`]);
//! the player input is the full [`Q3PlayerState`] (the donor reads only
//! `clientNum`/`origin`/`viewheight` from it, and selection reads no more).

use std::cell::Cell;
use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3};
use qa_net::q3_net::{Q3EntityState, Q3NetError, Q3PlayerState};
use qa_net::q3_visibility::{
    select_q3_snapshot_entities, Q3VisibilityBindings, Q3VisibilityEntity, Q3VisibilityLink, Q3VisibilityWorld,
    Q3VisibleEntities,
};
use thiserror::Error;

/// Absorbed `SharedSceneQueries` pick used by application Q3 selection
/// (donor `Pick<SharedSceneQueries, ...>`).
pub trait ApplicationQ3SceneQueries {
    /// Leaf containing a point (donor `pointLeaf`).
    fn point_leaf(&self, point: Vec3) -> i32;
    /// Cluster of a leaf (donor `leafCluster`).
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Area of a leaf (donor `leafArea`).
    fn leaf_area(&self, leaf: i32) -> i32;
    /// Area visibility bits (donor `areaBits`).
    fn area_bits(&self, area: i32) -> Vec<u8>;
    /// Leaves overlapping bounds, up to `limit` (donor `boxLeaves`).
    fn box_leaves(&self, bounds: Bounds, limit: i32) -> Vec<i32>;
    /// Whether a cluster is PVS-visible from another cluster (donor
    /// `clusterVisible(from, to, "pvs")`; the call site always passes
    /// `"pvs"`, so the absorbed query drops the kind).
    fn cluster_visible(&self, from: i32, cluster: i32) -> bool;
    /// Whether two areas connect (donor `areasConnected`).
    fn areas_connected(&self, first: i32, second: i32) -> bool;
}

/// Application Q3 source entity (donor inline source row).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationQ3SourceEntity {
    /// Entity state.
    pub state: Q3EntityState,
    /// Whether the entity is linked.
    pub linked: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Single-client target.
    pub single_client: i32,
}

/// Error for application Quake III snapshot selection.
#[derive(Debug, Error)]
pub enum ApplicationQ3VisibilityError {
    /// A linked source entity lost its body bounds.
    #[error("Linked Q3 source entity {0} lost its shared body bounds")]
    MissingBounds(i32),
    /// Area bits exceed the 32-byte mask storage.
    #[error("Selected world exceeds source Q3 area mask storage")]
    AreaMaskStorage,
    /// Snapshot selection failure.
    #[error(transparent)]
    Snapshot(#[from] Q3NetError),
}

/// Bindings adapter over absorbed scene queries.
struct ApplicationQ3Bindings<'q, 'p> {
    queries: &'q dyn ApplicationQ3SceneQueries,
    entities: HashMap<i32, Q3VisibilityEntity>,
    links: HashMap<i32, Q3VisibilityLink>,
    entity_count: i32,
    print: &'p mut dyn FnMut(&str),
    area_overflow: Cell<bool>,
}

impl Q3VisibilityWorld for ApplicationQ3Bindings<'_, '_> {
    fn point_leafnum(&self, point: Vec3) -> i32 {
        self.queries.point_leaf(point)
    }

    fn leaf_area(&self, leaf: i32) -> i32 {
        self.queries.leaf_area(leaf)
    }

    fn leaf_cluster(&self, leaf: i32) -> i32 {
        self.queries.leaf_cluster(leaf)
    }

    fn write_area_bits(&self, bytes: &mut [u8; 32], area: i32) -> i32 {
        let bits = self.queries.area_bits(area);
        if bits.len() > bytes.len() {
            self.area_overflow.set(true);
            return 0;
        }
        for (index, value) in bits.iter().enumerate() {
            bytes[index] |= *value;
        }
        bits.len() as i32
    }

    fn cluster_pvs_byte(&self, cluster: i32, index: i32) -> u8 {
        let mut byte = 0u8;
        for bit in 0..8 {
            if self.queries.cluster_visible(cluster, index * 8 + bit) {
                byte |= 1 << bit;
            }
        }
        byte
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        self.queries.areas_connected(first, second)
    }
}

impl Q3VisibilityBindings for ApplicationQ3Bindings<'_, '_> {
    fn collision(&self) -> &dyn Q3VisibilityWorld {
        self
    }

    fn entity_count(&self) -> i32 {
        self.entity_count
    }

    fn dead(&self) -> bool {
        false
    }

    fn entity(&self, number: i32) -> Q3VisibilityEntity {
        if let Some(value) = self.entities.get(&number) {
            return value.clone();
        }
        let state = Q3EntityState {
            number,
            ..Q3EntityState::default()
        };
        Q3VisibilityEntity {
            state,
            linked: false,
            flags: 0,
            single_client: 0,
        }
    }

    fn fix_entity_number(&mut self, number: i32) {
        if let Some(entity) = self.entities.get_mut(&number) {
            entity.state.number = number;
        }
    }

    fn link(&self, number: i32) -> Option<Q3VisibilityLink> {
        self.links.get(&number).cloned()
    }

    fn print(&mut self, text: &str) {
        (self.print)(text);
    }
}

/// The local transport runs the same Q3 visibility selection as a network
/// snapshot (donor `selectApplicationQ3Snapshot`).
pub fn select_application_q3_snapshot(
    player: &Q3PlayerState,
    source: &[ApplicationQ3SourceEntity],
    queries: &dyn ApplicationQ3SceneQueries,
    bounds: &dyn Fn(i32) -> Option<Bounds>,
    leaf_count: i32,
    print: &mut dyn FnMut(&str),
) -> Result<Q3VisibleEntities, ApplicationQ3VisibilityError> {
    let mut entities = HashMap::new();
    let mut links = HashMap::new();
    let mut entity_count = 0;
    for row in source {
        let number = row.state.number;
        entity_count = entity_count.max(number + 1);
        entities.insert(
            number,
            Q3VisibilityEntity {
                state: row.state.clone(),
                linked: row.linked,
                flags: row.server_flags,
                single_client: row.single_client,
            },
        );
        if !row.linked {
            continue;
        }
        let shape = bounds(number).ok_or(ApplicationQ3VisibilityError::MissingBounds(number))?;
        let all = queries.box_leaves(shape, leaf_count);
        let leaves: Vec<i32> = all.iter().copied().take(128).collect();
        let mut areanum = -1;
        let mut areanum2 = -1;
        for leaf in &leaves {
            let area = queries.leaf_area(*leaf);
            if area == -1 {
                continue;
            }
            if areanum != -1 && areanum != area {
                areanum2 = area;
            } else {
                areanum = area;
            }
        }
        let mut clusters = Vec::new();
        let mut last_cluster = 0;
        for leaf in &leaves {
            let cluster = queries.leaf_cluster(*leaf);
            if cluster == -1 {
                continue;
            }
            clusters.push(cluster);
            if clusters.len() == 16 {
                if let Some(last_leaf) = all.last() {
                    last_cluster = queries.leaf_cluster(*last_leaf);
                }
                break;
            }
        }
        links.insert(
            number,
            Q3VisibilityLink {
                areanum,
                areanum2,
                clusters,
                last_cluster,
            },
        );
    }
    let mut bindings = ApplicationQ3Bindings {
        queries,
        entities,
        links,
        entity_count,
        print,
        area_overflow: Cell::new(false),
    };
    let selected = select_q3_snapshot_entities(player, &mut bindings);
    if bindings.area_overflow.get() {
        return Err(ApplicationQ3VisibilityError::AreaMaskStorage);
    }
    Ok(selected?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;
    use qa_net::q3_net::Q3Product;
    use qa_net::q3_visibility::Q3ServerEntityFlags;

    struct OpenQueries {
        area_bytes: usize,
    }

    impl ApplicationQ3SceneQueries for OpenQueries {
        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }

        fn area_bits(&self, _area: i32) -> Vec<u8> {
            vec![0x01; self.area_bytes]
        }

        fn box_leaves(&self, _bounds: Bounds, _limit: i32) -> Vec<i32> {
            vec![0]
        }

        fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
            true
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }
    }

    fn player(client_num: i32) -> Q3PlayerState {
        let mut state = Q3PlayerState::new(Q3Product::Base);
        state.client_num = client_num;
        state
    }

    fn entity(number: i32, linked: bool, flags: i32, single_client: i32) -> ApplicationQ3SourceEntity {
        let state = Q3EntityState {
            number,
            ..Q3EntityState::default()
        };
        ApplicationQ3SourceEntity {
            state,
            linked,
            server_flags: flags,
            single_client,
        }
    }

    fn unit_bounds(number: i32) -> Option<Bounds> {
        if number == 7 {
            return None;
        }
        Some(Bounds {
            min: vec3(-8.0, -8.0, -8.0),
            max: vec3(8.0, 8.0, 8.0),
        })
    }

    #[test]
    fn selects_visible_linked_entities() {
        let queries = OpenQueries { area_bytes: 1 };
        let source = vec![
            entity(5, true, 0, 0),
            entity(6, false, 0, 0),
            entity(8, true, Q3ServerEntityFlags::SINGLE_CLIENT, 1),
        ];
        let mut printed = Vec::new();
        let selected = select_application_q3_snapshot(&player(0), &source, &queries, &unit_bounds, 64, &mut |text| {
            printed.push(text.to_string())
        })
        .unwrap();
        assert_eq!(selected.entities.len(), 1);
        assert_eq!(selected.entities[0].number, 5);
        assert_eq!(selected.area_mask, vec![0xFE]);
        assert!(printed.is_empty());
    }

    #[test]
    fn linked_entity_without_bounds_is_rejected() {
        let queries = OpenQueries { area_bytes: 1 };
        let source = vec![entity(7, true, 0, 0)];
        let mut printed = Vec::new();
        let error = select_application_q3_snapshot(&player(0), &source, &queries, &unit_bounds, 64, &mut |text| {
            printed.push(text.to_string())
        })
        .unwrap_err();
        assert!(matches!(error, ApplicationQ3VisibilityError::MissingBounds(7)));
        assert_eq!(
            error.to_string(),
            "Linked Q3 source entity 7 lost its shared body bounds"
        );
    }

    #[test]
    fn oversized_area_mask_is_rejected() {
        let queries = OpenQueries { area_bytes: 33 };
        let source = vec![entity(5, true, 0, 0)];
        let mut printed = Vec::new();
        let error = select_application_q3_snapshot(&player(0), &source, &queries, &unit_bounds, 64, &mut |text| {
            printed.push(text.to_string())
        })
        .unwrap_err();
        assert!(matches!(error, ApplicationQ3VisibilityError::AreaMaskStorage));
        assert_eq!(error.to_string(), "Selected world exceeds source Q3 area mask storage");
    }

    #[test]
    fn bad_client_number_propagates_snapshot_error() {
        let queries = OpenQueries { area_bytes: 1 };
        let mut printed = Vec::new();
        let error = select_application_q3_snapshot(&player(-1), &[], &queries, &unit_bounds, 64, &mut |text| {
            printed.push(text.to_string())
        })
        .unwrap_err();
        assert!(matches!(error, ApplicationQ3VisibilityError::Snapshot(_)));
    }
}
