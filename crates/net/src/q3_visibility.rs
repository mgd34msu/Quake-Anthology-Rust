//! Quake III server snapshot entity visibility.
//!
//! Donor provenance: `Q3ServerEntityFlags`, `Q3VisibilityEntity`,
//! `Q3VisibilityLink`, `Q3VisibilityWorld`, `Q3VisibilityBindings`,
//! `Q3VisibleEntities`, and `selectQ3SnapshotEntities` in
//! `src/network/q3/visibility.ts` (`SV_AddEntitiesVisibleFromPoint` and
//! `SV_BuildClientSnapshot` in `sv_snapshot.c`). Entity and player state
//! reuse [`Q3EntityState`](crate::q3_net::Q3EntityState) and
//! [`Q3PlayerState`](crate::q3_net::Q3PlayerState); vector math reuses
//! `qa-core` with the donor's single-precision rounding.
//!
//! The donor returns host-borrowed states; Rust callers receive owned
//! clones instead. Cluster lists are dense `Vec<i32>`, so the donor's
//! hole check is unrepresentable and omitted.

use std::collections::HashSet;

use qa_core::math::{dot3, sub3, vec3, Vec3};

use crate::q3_net::{Q3EntityState, Q3NetError, Q3PlayerState};

/// Server entity flags (`Q3ServerEntityFlags`).
pub struct Q3ServerEntityFlags;

impl Q3ServerEntityFlags {
    /// Never send.
    pub const NO_CLIENT: i32 = 1;
    /// Bitmask send.
    pub const CLIENT_MASK: i32 = 2;
    /// Bot.
    pub const BOT: i32 = 8;
    /// Always send.
    pub const BROADCAST: i32 = 32;
    /// Portal camera.
    pub const PORTAL: i32 = 64;
    /// Use current origin.
    pub const USE_CURRENT_ORIGIN: i32 = 128;
    /// Single client.
    pub const SINGLE_CLIENT: i32 = 256;
    /// Omit from server info.
    pub const NO_SERVER_INFO: i32 = 512;
    /// Every client but one.
    pub const NOT_SINGLE_CLIENT: i32 = 2048;
}

/// Visibility entity (`Q3VisibilityEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3VisibilityEntity {
    /// Entity state.
    pub state: Q3EntityState,
    /// Whether the entity is linked.
    pub linked: bool,
    /// Entity flags.
    pub flags: i32,
    /// Single-client target.
    pub single_client: i32,
}

/// Visibility link (`Q3VisibilityLink`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3VisibilityLink {
    /// Area number.
    pub areanum: i32,
    /// Second area number.
    pub areanum2: i32,
    /// Clusters.
    pub clusters: Vec<i32>,
    /// Last cluster of a range scan.
    pub last_cluster: i32,
}

/// Collision world queries (`Q3VisibilityWorld`).
///
/// `cluster_pvs_byte` flattens the donor's `clusterPVS(cluster).byteAt`
/// view into one call; reads are equivalent.
pub trait Q3VisibilityWorld {
    /// Leaf containing a point.
    fn point_leafnum(&self, point: Vec3) -> i32;
    /// Area of a leaf.
    fn leaf_area(&self, leaf: i32) -> i32;
    /// Cluster of a leaf.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Write area bits; returns the byte count.
    fn write_area_bits(&self, bytes: &mut [u8; 32], area: i32) -> i32;
    /// PVS byte for a cluster and byte index.
    fn cluster_pvs_byte(&self, cluster: i32, index: i32) -> u8;
    /// Whether two areas connect.
    fn areas_connected(&self, first: i32, second: i32) -> bool;
}

/// Visibility bindings (`Q3VisibilityBindings`).
pub trait Q3VisibilityBindings {
    /// Collision world.
    fn collision(&self) -> &dyn Q3VisibilityWorld;
    /// Entity count.
    fn entity_count(&self) -> i32;
    /// Whether the server is dead.
    fn dead(&self) -> bool;
    /// Read an entity snapshot.
    fn entity(&self, number: i32) -> Q3VisibilityEntity;
    /// Repair a mismatched entity number in host state.
    fn fix_entity_number(&mut self, number: i32);
    /// Read an entity link.
    fn link(&self, number: i32) -> Option<Q3VisibilityLink>;
    /// Print.
    fn print(&mut self, text: &str);
}

/// Visible entities (`Q3VisibleEntities`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3VisibleEntities {
    /// Area mask.
    pub area_mask: Vec<u8>,
    /// Selected entity states.
    pub entities: Vec<Q3EntityState>,
}

fn to_vec3(value: &[f32; 3]) -> Vec3 {
    vec3(value[0], value[1], value[2])
}

/// Selection cursor.
struct Select<'b> {
    bindings: &'b mut dyn Q3VisibilityBindings,
    client_num: i32,
    selected: Vec<i32>,
    visited: HashSet<i32>,
    area_bits: [u8; 32],
    area_bytes: i32,
}

impl Select<'_> {
    /// Whether a cluster is visible.
    fn visible(&self, pvs_cluster: i32, cluster: i32) -> bool {
        let byte = self.bindings.collision().cluster_pvs_byte(pvs_cluster, cluster >> 3);
        i32::from(byte) & (1 << (cluster & 7)) != 0
    }

    /// Add an entity (`add`).
    fn add(&mut self, number: i32) -> Result<(), Q3NetError> {
        if !self.visited.insert(number) {
            return Ok(());
        }
        if self.selected.len() == 256 {
            return Ok(());
        }
        if number < 0 || number >= 1024 {
            return Err(Q3NetError::Range("Server snapshot entity outside source storage"));
        }
        self.selected.push(number);
        Ok(())
    }

    /// Collect entities visible from a point (`visibleFrom`).
    fn visible_from(&mut self, origin: Vec3) -> Result<(), Q3NetError> {
        if self.bindings.dead() {
            return Ok(());
        }
        let leaf = self.bindings.collision().point_leafnum(origin);
        let area = self.bindings.collision().leaf_area(leaf);
        self.area_bytes = self.bindings.collision().write_area_bits(&mut self.area_bits, area);
        let pvs_cluster = self.bindings.collision().leaf_cluster(leaf);
        let entity_count = self.bindings.entity_count();
        for number in 0..entity_count {
            let mut entity = self.bindings.entity(number);
            if !entity.linked {
                continue;
            }
            if entity.state.number != number {
                self.bindings.print("FIXING ENT->S.NUMBER!!!\n");
                self.bindings.fix_entity_number(number);
                entity.state.number = number;
            }
            if entity.flags & Q3ServerEntityFlags::NO_CLIENT != 0 {
                continue;
            }
            if entity.flags & Q3ServerEntityFlags::SINGLE_CLIENT != 0 && entity.single_client != self.client_num {
                continue;
            }
            if entity.flags & Q3ServerEntityFlags::NOT_SINGLE_CLIENT != 0
                && entity.single_client == self.client_num
            {
                continue;
            }
            if entity.flags & Q3ServerEntityFlags::CLIENT_MASK != 0 {
                if self.client_num >= 32 {
                    return Err(Q3NetError::Drop {
                        kind: "drop",
                        message: "SVF_CLIENTMASK: cientNum > 32\n".to_owned(),
                    });
                }
                if !entity.single_client & (1 << self.client_num) != 0 {
                    continue;
                }
            }
            if self.visited.contains(&number) {
                continue;
            }
            if entity.flags & Q3ServerEntityFlags::BROADCAST != 0 {
                self.add(number)?;
                continue;
            }
            let Some(link) = self.bindings.link(number) else {
                continue;
            };
            if (!self.bindings.collision().areas_connected(area, link.areanum)
                && !self.bindings.collision().areas_connected(area, link.areanum2))
                || link.clusters.is_empty()
            {
                continue;
            }
            let mut cluster = 0;
            let mut index = 0;
            while index < link.clusters.len() {
                cluster = link.clusters[index];
                if self.visible(pvs_cluster, cluster) {
                    break;
                }
                index += 1;
            }
            if index == link.clusters.len() {
                if link.last_cluster == 0 {
                    continue;
                }
                // The scan runs in 64 bits so a maximal range cannot
                // overflow; the donor's equality quirk is preserved.
                let mut scan = i64::from(cluster);
                while scan <= i64::from(link.last_cluster) {
                    if self.visible(pvs_cluster, scan as i32) {
                        break;
                    }
                    scan += 1;
                }
                if scan == i64::from(link.last_cluster) {
                    continue;
                }
            }
            self.add(number)?;
            if entity.flags & Q3ServerEntityFlags::PORTAL != 0 {
                if entity.state.generic1 != 0 {
                    let delta = sub3(to_vec3(&entity.state.origin), origin);
                    let range = entity.state.generic1 as f32 * entity.state.generic1 as f32;
                    if dot3(delta, delta) > range {
                        continue;
                    }
                }
                self.visible_from(to_vec3(&entity.state.origin2))?;
            }
        }
        Ok(())
    }
}

/// Select snapshot entities (`selectQ3SnapshotEntities`).
pub fn select_q3_snapshot_entities(
    player: &Q3PlayerState,
    bindings: &mut dyn Q3VisibilityBindings,
) -> Result<Q3VisibleEntities, Q3NetError> {
    if player.client_num < 0 || player.client_num >= 1024 {
        return Err(Q3NetError::Drop {
            kind: "drop",
            message: "SV_SvEntityForGentity: bad gEnt".to_owned(),
        });
    }
    let mut select = Select {
        bindings,
        client_num: player.client_num,
        selected: Vec::new(),
        visited: HashSet::from([player.client_num]),
        area_bits: [0u8; 32],
        area_bytes: 0,
    };
    let view = vec3(
        player.origin[0],
        player.origin[1],
        player.origin[2] + player.viewheight as f32,
    );
    select.visible_from(view)?;
    select.selected.sort();
    if select.area_bytes < 0 || select.area_bytes > 32 {
        return Err(Q3NetError::Range("Source area bits exceed 32 bytes"));
    }
    for byte in select.area_bits.iter_mut() {
        *byte ^= 255;
    }
    let area_mask = select.area_bits[..select.area_bytes as usize].to_vec();
    let mut entities = Vec::with_capacity(select.selected.len());
    for number in &select.selected {
        entities.push(select.bindings.entity(*number).state);
    }
    Ok(Q3VisibleEntities { area_mask, entities })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct World;

    impl Q3VisibilityWorld for World {
        fn point_leafnum(&self, point: Vec3) -> i32 {
            i32::from(point.x > 100.0)
        }

        fn leaf_area(&self, leaf: i32) -> i32 {
            leaf
        }

        fn leaf_cluster(&self, leaf: i32) -> i32 {
            leaf
        }

        fn write_area_bits(&self, bytes: &mut [u8; 32], area: i32) -> i32 {
            bytes[0] = area as u8;
            1
        }

        fn cluster_pvs_byte(&self, _cluster: i32, _index: i32) -> u8 {
            0xFF
        }

        fn areas_connected(&self, first: i32, second: i32) -> bool {
            first == second
        }
    }

    struct Fixture {
        entities: Vec<Q3VisibilityEntity>,
        links: Vec<Option<Q3VisibilityLink>>,
        fixed: Vec<i32>,
        printed: Vec<String>,
    }

    impl Q3VisibilityBindings for Fixture {
        fn collision(&self) -> &dyn Q3VisibilityWorld {
            &World
        }

        fn entity_count(&self) -> i32 {
            self.entities.len() as i32
        }

        fn dead(&self) -> bool {
            false
        }

        fn entity(&self, number: i32) -> Q3VisibilityEntity {
            self.entities[number as usize].clone()
        }

        fn fix_entity_number(&mut self, number: i32) {
            self.fixed.push(number);
            self.entities[number as usize].state.number = number;
        }

        fn link(&self, number: i32) -> Option<Q3VisibilityLink> {
            self.links[number as usize].clone()
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_owned());
        }
    }

    fn entity(number: i32, flags: i32) -> Q3VisibilityEntity {
        Q3VisibilityEntity {
            state: Q3EntityState {
                number,
                ..Q3EntityState::default()
            },
            linked: true,
            flags,
            single_client: 0,
        }
    }

    fn link(area: i32) -> Q3VisibilityLink {
        Q3VisibilityLink {
            areanum: area,
            areanum2: area,
            clusters: vec![area],
            last_cluster: 0,
        }
    }

    fn player() -> Q3PlayerState {
        let mut player = Q3PlayerState::new(crate::q3_net::Q3Product::Base);
        player.client_num = 0;
        player.origin = [0.0, 0.0, 0.0];
        player.viewheight = 26;
        player
    }

    #[test]
    fn broadcast_and_visible_entities_are_selected() {
        let mut fixture = Fixture {
            entities: vec![
                entity(0, 0),
                entity(1, Q3ServerEntityFlags::BROADCAST),
                entity(2, 0),
                entity(3, Q3ServerEntityFlags::NO_CLIENT),
            ],
            links: vec![None, None, Some(link(0)), Some(link(0))],
            fixed: Vec::new(),
            printed: Vec::new(),
        };
        let visible = select_q3_snapshot_entities(&player(), &mut fixture).unwrap();
        assert_eq!(visible.area_mask, vec![0xFF ^ 0]);
        assert_eq!(visible.entities.len(), 2);
        assert_eq!(visible.entities[0].number, 1);
        assert_eq!(visible.entities[1].number, 2);
    }

    #[test]
    fn mismatched_numbers_are_fixed() {
        let mut broken = entity(999, Q3ServerEntityFlags::BROADCAST);
        broken.state.number = 999;
        let mut fixture = Fixture {
            entities: vec![entity(0, 0), broken],
            links: vec![None, None],
            fixed: Vec::new(),
            printed: Vec::new(),
        };
        let visible = select_q3_snapshot_entities(&player(), &mut fixture).unwrap();
        assert_eq!(fixture.fixed, vec![1]);
        assert_eq!(fixture.printed, vec!["FIXING ENT->S.NUMBER!!!\n"]);
        assert_eq!(visible.entities[0].number, 1);
    }

    #[test]
    fn client_masks_filter_and_validate() {
        let mut masked = entity(1, Q3ServerEntityFlags::CLIENT_MASK);
        masked.single_client = 0b10;
        let mut fixture = Fixture {
            entities: vec![entity(0, 0), masked],
            links: vec![None, Some(link(0))],
            fixed: Vec::new(),
            printed: Vec::new(),
        };
        let visible = select_q3_snapshot_entities(&player(), &mut fixture).unwrap();
        assert!(visible.entities.is_empty());
        let mut old = player();
        old.client_num = 33;
        assert_eq!(
            select_q3_snapshot_entities(&old, &mut fixture),
            Err(Q3NetError::Drop {
                kind: "drop",
                message: "SVF_CLIENTMASK: cientNum > 32\n".to_owned(),
            })
        );
        let mut bad = player();
        bad.client_num = 1024;
        assert_eq!(
            select_q3_snapshot_entities(&bad, &mut fixture),
            Err(Q3NetError::Drop {
                kind: "drop",
                message: "SV_SvEntityForGentity: bad gEnt".to_owned(),
            })
        );
    }

    #[test]
    fn portals_recurse_into_connected_areas() {
        let mut portal = entity(1, Q3ServerEntityFlags::PORTAL);
        portal.state.origin2 = [200.0, 0.0, 0.0];
        let mut fixture = Fixture {
            entities: vec![entity(0, 0), portal, entity(2, 0)],
            links: vec![None, Some(link(0)), Some(link(1))],
            fixed: Vec::new(),
            printed: Vec::new(),
        };
        let visible = select_q3_snapshot_entities(&player(), &mut fixture).unwrap();
        // The portal opens area 1, selecting the far entity; the last
        // area-bits write wins.
        assert_eq!(visible.entities.len(), 2);
        assert_eq!(visible.entities[0].number, 1);
        assert_eq!(visible.entities[1].number, 2);
        assert_eq!(visible.area_mask, vec![0xFF ^ 1]);
    }

    #[test]
    fn distant_portals_do_not_recurse() {
        let mut portal = entity(1, Q3ServerEntityFlags::PORTAL);
        portal.state.generic1 = 10;
        portal.state.origin = [1000.0, 0.0, 0.0];
        portal.state.origin2 = [200.0, 0.0, 0.0];
        let mut fixture = Fixture {
            entities: vec![entity(0, 0), portal, entity(2, 0)],
            links: vec![None, Some(link(0)), Some(link(1))],
            fixed: Vec::new(),
            printed: Vec::new(),
        };
        let visible = select_q3_snapshot_entities(&player(), &mut fixture).unwrap();
        assert_eq!(visible.entities.len(), 1);
        assert_eq!(visible.entities[0].number, 1);
        assert_eq!(visible.area_mask, vec![0xFF ^ 0]);
    }
}
