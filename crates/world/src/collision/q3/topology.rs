//! BSP topology and area connectivity translated from id Software's
//! `cm_test.c`, `cm_load.c` and `q_math.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/topology.ts`.

use std::cell::Cell;
use std::rc::Rc;

use qa_core::math::{dot3, vec3, Bounds, Vec3};

use super::counters::CollisionCounters;
use super::map_resource::{CollisionMapData, CollisionPlane};
use crate::error::WorldError;
use crate::save::value::{arr, boolean, int, obj, SaveJson, SaveReader};

/// Leaves touched by a bounds query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxLeafList {
    /// Touched leaves, in walk order.
    pub leaves: Vec<usize>,
    /// First node split by the bounds, if the query branched.
    pub topnode: Option<usize>,
    /// Last non-solid leaf visited, including leaves omitted after overflow.
    pub last_leaf: usize,
    /// The leaf list overflowed its capacity.
    pub overflowed: bool,
}

/// Borrowed `CM_ClusterPVS` pointer; offsets may cross rows within stored
/// visibility bytes.
#[derive(Debug, Clone, Copy)]
pub struct SourceClusterPVS<'a> {
    visibility: &'a [u8],
    base_offset: i64,
}

impl SourceClusterPVS<'_> {
    /// Read a visibility byte at `offset` past the cluster row.
    pub fn byte_at(&self, offset: i64) -> Result<u8, WorldError> {
        let index = self.base_offset + offset;
        if index < 0 {
            return Err(WorldError::BadCollisionRecord(format!(
                "PVS byte {index} outside stored visibility allocation of {} bytes",
                self.visibility.len()
            )));
        }
        self.visibility.get(index as usize).copied().ok_or_else(|| {
            WorldError::BadCollisionRecord(format!(
                "PVS byte {index} outside stored visibility allocation of {} bytes",
                self.visibility.len()
            ))
        })
    }
}

fn topo_at<T>(values: &[T], index: usize) -> Result<&T, WorldError> {
    values
        .get(index)
        .ok_or_else(|| WorldError::BadCollisionRecord(format!("collision topology index {index} out of range")))
}

fn finite(point: Vec3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn ordered(bounds: &Bounds) -> bool {
    finite(bounds.min)
        && finite(bounds.max)
        && bounds.min.x <= bounds.max.x
        && bounds.min.y <= bounds.max.y
        && bounds.min.z <= bounds.max.z
}

fn box_side(bounds: &Bounds, plane: &CollisionPlane) -> u8 {
    let normal = plane.normal;
    if plane.plane_type == 0 {
        return if plane.distance <= bounds.min.x {
            1
        } else if plane.distance >= bounds.max.x {
            2
        } else {
            3
        };
    }
    if plane.plane_type == 1 {
        return if plane.distance <= bounds.min.y {
            1
        } else if plane.distance >= bounds.max.y {
            2
        } else {
            3
        };
    }
    if plane.plane_type == 2 {
        return if plane.distance <= bounds.min.z {
            1
        } else if plane.distance >= bounds.max.z {
            2
        } else {
            3
        };
    }
    let signs = plane.signbits;
    let far = if signs < 8 {
        dot3(
            normal,
            vec3(
                if signs & 1 != 0 { bounds.min.x } else { bounds.max.x },
                if signs & 2 != 0 { bounds.min.y } else { bounds.max.y },
                if signs & 4 != 0 { bounds.min.z } else { bounds.max.z },
            ),
        )
    } else {
        0.0
    };
    let near = if signs < 8 {
        dot3(
            normal,
            vec3(
                if signs & 1 != 0 { bounds.max.x } else { bounds.min.x },
                if signs & 2 != 0 { bounds.max.y } else { bounds.min.y },
                if signs & 4 != 0 { bounds.max.z } else { bounds.min.z },
            ),
        )
    } else {
        0.0
    };
    if far >= plane.distance {
        if near < plane.distance {
            3
        } else {
            1
        }
    } else if near < plane.distance {
        2
    } else {
        0
    }
}

/// BSP topology over borrowed collision records.
#[derive(Debug)]
pub struct CollisionTopology {
    map: Rc<CollisionMapData>,
    counters: Rc<CollisionCounters>,
    /// Area count.
    pub area_count: usize,
    /// Cluster count.
    pub cluster_count: i32,
    flood_valid: Cell<i32>,
    no_areas: Cell<bool>,
}

impl CollisionTopology {
    /// Borrow a map and flood its areas.
    #[must_use]
    pub fn new(map: Rc<CollisionMapData>, counters: Rc<CollisionCounters>) -> Self {
        let topology = Self {
            area_count: map.areas.len(),
            cluster_count: map.cluster_count,
            map,
            counters,
            flood_valid: Cell::new(0),
            no_areas: Cell::new(false),
        };
        topology.flood_areas().expect("fresh areas always flood");
        topology
    }

    /// Capture the portal/area checkpoint.
    pub fn capture_portal_checkpoint(&self) -> SaveJson {
        obj(vec![
            ("version", int(1)),
            ("floodValid", int(self.flood_valid.get() as i64)),
            ("noAreas", boolean(self.no_areas.get())),
            (
                "areas",
                arr(self
                    .map
                    .areas
                    .iter()
                    .map(|area| {
                        obj(vec![
                            ("flood", int(area.flood.get() as i64)),
                            ("floodValid", int(area.flood_valid.get() as i64)),
                        ])
                    })
                    .collect()),
            ),
            (
                "portals",
                arr((0..self.area_count * self.area_count)
                    .map(|index| int(self.map.portals[index].get() as i64))
                    .collect()),
            ),
        ])
    }

    /// Restore a portal/area checkpoint.
    pub fn restore_portal_checkpoint(&self, value: &SaveJson) -> Result<(), WorldError> {
        let reader = SaveReader::at(value, "q3.collision.portals");
        reader.field("version").literal_i64(1)?;
        let word = |field: SaveReader| -> Result<i32, WorldError> {
            let value = field.integer(-0x8000_0000)?;
            if value > 0x7fff_ffff {
                return Err(field.fail("expected a signed 32-bit source word"));
            }
            Ok(value as i32)
        };
        let flood_valid = word(reader.field("floodValid"))?;
        let no_areas = reader.field("noAreas").boolean()?;
        let areas = reader.field("areas").list(|area| -> Result<(i32, i32), WorldError> {
            Ok((word(area.field("flood"))?, word(area.field("floodValid"))?))
        })?;
        let portals = reader.field("portals").list(word)?;
        if areas.len() != self.area_count || portals.len() != self.area_count * self.area_count {
            return Err(reader.fail("portal checkpoint belongs to another area table"));
        }
        for first in 0..self.area_count {
            for second in first + 1..self.area_count {
                if portals[first * self.area_count + second] != portals[second * self.area_count + first] {
                    return Err(reader.fail("asymmetric source portal references"));
                }
            }
        }
        for (index, (flood, valid)) in areas.iter().enumerate() {
            let target = topo_at(&self.map.areas, index)?;
            target.flood.set(*flood);
            target.flood_valid.set(*valid);
        }
        for (index, count) in portals.iter().enumerate() {
            self.map.portal_set(index, *count)?;
        }
        self.flood_valid.set(flood_valid);
        self.no_areas.set(no_areas);
        Ok(())
    }

    /// Leaf containing a point.
    pub fn point_leafnum(&self, point: Vec3) -> Result<usize, WorldError> {
        if !finite(point) {
            return Err(WorldError::BadCollisionRecord(
                "point leaf query requires finite coordinates".to_string(),
            ));
        }
        if self.map.nodes.is_empty() {
            return Ok(0);
        }
        let mut index: i32 = 0;
        while index >= 0 {
            let node = topo_at(&self.map.nodes, index as usize)?;
            let plane = topo_at(&self.map.planes, node.plane)?;
            let along = if plane.plane_type == 0 {
                point.x
            } else if plane.plane_type == 1 {
                point.y
            } else if plane.plane_type == 2 {
                point.z
            } else {
                dot3(point, plane.normal)
            };
            let distance = along - plane.distance;
            index = node.children[if distance < 0.0 { 1 } else { 0 }];
        }
        CollisionCounters::bump(&self.counters.c_pointcontents);
        Ok((-1 - index) as usize)
    }

    /// Leaves touched by bounds, up to `max_leaves`.
    pub fn box_leafnums(&self, bounds: &Bounds, max_leaves: usize) -> Result<BoxLeafList, WorldError> {
        if !ordered(bounds) {
            return Err(WorldError::BadCollisionRecord(
                "box leaf query requires finite ordered bounds".to_string(),
            ));
        }
        CollisionCounters::bump(&self.map.check_count);
        let mut leaves = Vec::new();
        let mut last_leaf = 0;
        let mut overflowed = false;
        let topnode = self.visit_leaves(bounds, &mut |leafnum| {
            if topo_at(&self.map.leaves, leafnum)?.cluster != -1 {
                last_leaf = leafnum;
            }
            if leaves.len() == max_leaves {
                overflowed = true;
            } else {
                leaves.push(leafnum);
            }
            Ok(())
        })?;
        Ok(BoxLeafList {
            leaves,
            topnode,
            last_leaf,
            overflowed,
        })
    }

    /// `CM_BoxBrushes` returns brush indexes, including their mutable
    /// check counts on the map.
    pub fn box_brushes(&self, bounds: &Bounds, max_brushes: i32) -> Result<Vec<usize>, WorldError> {
        CollisionCounters::bump(&self.map.check_count);
        let query = Bounds {
            min: vec3(bounds.min.x, bounds.min.y, bounds.min.z),
            max: vec3(bounds.max.x, bounds.max.y, bounds.max.z),
        };
        let mut brushes = Vec::new();
        self.visit_leaves(&query, &mut |leafnum| {
            let leaf = topo_at(&self.map.leaves, leafnum)?;
            for index in 0..leaf.brush_count {
                let brush_index = self.map.leaf_brush_at(leaf.first_brush + index)?;
                let brush_number = usize::try_from(brush_index).map_err(|_| {
                    WorldError::BadCollisionRecord(format!("collision topology index {brush_index} out of range"))
                })?;
                let brush = topo_at(&self.map.brushes, brush_number)?;
                if brush.check_count.get() == self.map.check_count.get() {
                    continue;
                }
                brush.check_count.set(self.map.check_count.get());
                let brush_bounds = self.map.brush_bounds(brush_number)?;
                if brush_bounds.min.x >= query.max.x
                    || brush_bounds.max.x <= query.min.x
                    || brush_bounds.min.y >= query.max.y
                    || brush_bounds.max.y <= query.min.y
                    || brush_bounds.min.z >= query.max.z
                    || brush_bounds.max.z <= query.min.z
                {
                    continue;
                }
                // CM_StoreBrushes stops only this leaf; the BSP walker
                // continues its siblings.
                if brushes.len() as i64 >= max_brushes as i64 {
                    return Ok(());
                }
                brushes.push(brush_number);
            }
            Ok(())
        })?;
        Ok(brushes)
    }

    /// `CM_BoxLeafnums_r` dispatches either `CM_StoreLeafs` or
    /// `CM_StoreBrushes` at each leaf.
    pub fn visit_leaves(
        &self,
        bounds: &Bounds,
        store_leaf: &mut dyn FnMut(usize) -> Result<(), WorldError>,
    ) -> Result<Option<usize>, WorldError> {
        let mut topnode = None;
        let mut pending = vec![if self.map.nodes.is_empty() { -1 } else { 0 }];
        while let Some(index) = pending.pop() {
            if index < 0 {
                store_leaf((-1 - index) as usize)?;
                continue;
            }
            let node = topo_at(&self.map.nodes, index as usize)?;
            let side = box_side(bounds, topo_at(&self.map.planes, node.plane)?);
            if side != 1 && side != 2 && topnode.is_none() {
                topnode = Some(index as usize);
            }
            if side != 1 {
                pending.push(node.children[1]);
            }
            if side != 2 {
                pending.push(node.children[0]);
            }
        }
        Ok(topnode)
    }

    /// Area containing a leaf.
    pub fn leaf_area(&self, index: i32) -> Result<i32, WorldError> {
        if index < 0 || index as usize >= self.map.leaves.len() {
            return Err(WorldError::BadCollisionRecord("CM_LeafArea: bad number".to_string()));
        }
        Ok(topo_at(&self.map.leaves, index as usize)?.area)
    }

    /// Cluster containing a leaf.
    pub fn leaf_cluster(&self, index: i32) -> Result<i32, WorldError> {
        if index < 0 || index as usize >= self.map.leaves.len() {
            return Err(WorldError::BadCollisionRecord("CM_LeafCluster: bad number".to_string()));
        }
        Ok(topo_at(&self.map.leaves, index as usize)?.cluster)
    }

    /// Borrow a cluster's PVS row. `CM_ClusterPVS` returns the allocation
    /// start for invalid clusters or no vis.
    pub fn cluster_pvs(&self, cluster: i32) -> SourceClusterPVS<'_> {
        let base_offset = match self.map.visibility_row_bytes {
            None => 0,
            Some(row) if cluster < 0 || cluster >= self.cluster_count => {
                let _ = row;
                0
            }
            Some(row) => cluster as i64 * row as i64,
        };
        SourceClusterPVS {
            visibility: &self.map.visibility,
            base_offset,
        }
    }

    /// Test cluster visibility through the PVS.
    pub fn cluster_visible(&self, from: i32, to: i32) -> Result<bool, WorldError> {
        if to < 0 || to >= self.cluster_count {
            return Ok(false);
        }
        let byte = self.cluster_pvs(from).byte_at((to >> 3) as i64)?;
        Ok(byte & (1 << (to & 7)) != 0)
    }

    /// Override area connectivity directly (unshared worlds only).
    pub fn set_no_areas(&self, enabled: bool) {
        self.no_areas.set(enabled);
    }

    fn check_area(&self, area: i32) -> Result<(), WorldError> {
        if area < 0 || area as usize >= self.area_count {
            return Err(WorldError::BadCollisionRecord(format!("invalid area {area}")));
        }
        Ok(())
    }

    /// Adjust the portal reference count between two areas and reflood.
    pub fn adjust_area_portal_state(&self, area1: i32, area2: i32, open: bool) -> Result<(), WorldError> {
        if area1 < 0 || area2 < 0 {
            return Ok(());
        }
        if area1 as usize >= self.area_count || area2 as usize >= self.area_count {
            return Err(WorldError::BadCollisionRecord(
                "CM_ChangeAreaPortalState: bad area number".to_string(),
            ));
        }
        let (area1, area2) = (area1 as usize, area2 as usize);
        let index1 = area1 * self.area_count + area2;
        let index2 = area2 * self.area_count + area1;
        let amount = if open { 1 } else { -1 };
        self.map
            .portal_set(index1, self.map.portal_at(index1)?.wrapping_add(amount))?;
        self.map
            .portal_set(index2, self.map.portal_at(index2)?.wrapping_add(amount))?;
        if !open && self.map.portal_at(index2)? < 0 {
            return Err(WorldError::BadCollisionRecord(
                "CM_AdjustAreaPortalState: negative reference count".to_string(),
            ));
        }
        self.flood_areas()
    }

    fn flood_areas(&self) -> Result<(), WorldError> {
        self.flood_valid.set(self.flood_valid.get().wrapping_add(1));
        let mut flood = 0i32;
        for area in 0..self.area_count {
            if topo_at(&self.map.areas, area)?.flood_valid.get() == self.flood_valid.get() {
                continue;
            }
            flood = flood.wrapping_add(1);
            let mut pending = vec![area];
            while let Some(current) = pending.pop() {
                let cell = topo_at(&self.map.areas, current)?;
                if cell.flood_valid.get() == self.flood_valid.get() {
                    if cell.flood.get() != flood {
                        return Err(WorldError::BadCollisionRecord("FloodArea_r: reflooded".to_string()));
                    }
                    continue;
                }
                cell.flood.set(flood);
                cell.flood_valid.set(self.flood_valid.get());
                for adjacent in (0..self.area_count).rev() {
                    if self.map.portal_at(current * self.area_count + adjacent)? > 0 {
                        pending.push(adjacent);
                    }
                }
            }
        }
        Ok(())
    }

    /// Test whether two areas connect. `None` reads the world's own flag.
    pub fn areas_connected(&self, area1: i32, area2: i32, no_areas: Option<bool>) -> Result<bool, WorldError> {
        if no_areas.unwrap_or_else(|| self.no_areas.get()) {
            return Ok(true);
        }
        if area1 < 0 || area2 < 0 {
            return Ok(false);
        }
        if area1 as usize >= self.area_count || area2 as usize >= self.area_count {
            return Err(WorldError::BadCollisionRecord("area >= cm.numAreas".to_string()));
        }
        Ok(topo_at(&self.map.areas, area1 as usize)?.flood.get()
            == topo_at(&self.map.areas, area2 as usize)?.flood.get())
    }

    /// Write area visibility bits; returns the byte count.
    pub fn write_area_bits(&self, buffer: &mut [u8], area: i32, no_areas: Option<bool>) -> Result<usize, WorldError> {
        let bytes = (self.area_count + 7) >> 3;
        if buffer.len() < bytes {
            return Err(WorldError::BadCollisionRecord(format!("area bits need {bytes} bytes")));
        }
        if no_areas.unwrap_or_else(|| self.no_areas.get()) || area == -1 {
            buffer[..bytes].fill(255);
            return Ok(bytes);
        }
        self.check_area(area)?;
        let flood = topo_at(&self.map.areas, area as usize)?.flood.get();
        for other in 0..self.area_count {
            if topo_at(&self.map.areas, other)?.flood.get() != flood {
                continue;
            }
            let index = other >> 3;
            buffer[index] |= 1 << (other & 7);
        }
        Ok(bytes)
    }

    /// Fresh area visibility bits.
    pub fn area_bits(&self, area: i32, no_areas: Option<bool>) -> Result<Vec<u8>, WorldError> {
        let mut bits = vec![0u8; (self.area_count + 7) >> 3];
        self.write_area_bits(&mut bits, area, no_areas)?;
        Ok(bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::Plane;

    use super::super::map_resource::{
        decoded_collision_map, CollisionShader, IndexRange, Q3BspChild, Q3CollisionBrushInput,
        Q3CollisionBrushSideInput, Q3CollisionGeometry, Q3CollisionLeafInput, Q3CollisionModelInput,
        Q3CollisionNodeInput,
    };

    fn shader() -> CollisionShader {
        CollisionShader {
            name: "solid".to_string(),
            surface_flags: 0,
            content_flags: 1,
        }
    }

    fn two_leaf_geometry() -> Q3CollisionGeometry {
        Q3CollisionGeometry {
            entities: String::new(),
            shaders: vec![shader()],
            planes: vec![Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            }],
            nodes: vec![Q3CollisionNodeInput {
                plane: 0,
                children: [Q3BspChild::Leaf(0), Q3BspChild::Leaf(1)],
            }],
            leaves: vec![
                Q3CollisionLeafInput {
                    cluster: 0,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 0 },
                    surfaces: IndexRange { first: 0, count: 0 },
                },
                Q3CollisionLeafInput {
                    cluster: 1,
                    area: 1,
                    brushes: IndexRange { first: 0, count: 0 },
                    surfaces: IndexRange { first: 0, count: 0 },
                },
            ],
            leaf_brushes: Vec::new(),
            leaf_surfaces: Vec::new(),
            models: vec![Q3CollisionModelInput {
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                brushes: IndexRange { first: 0, count: 0 },
                surfaces: IndexRange { first: 0, count: 0 },
            }],
            brushes: Vec::new(),
            brush_sides: Vec::new(),
            vertices: Vec::new(),
            surfaces: Vec::new(),
            visibility: None,
        }
    }

    fn two_leaf_topology() -> CollisionTopology {
        let map = decoded_collision_map(&two_leaf_geometry(), None).expect("decode");
        CollisionTopology::new(Rc::new(map), Rc::new(CollisionCounters::new()))
    }

    #[test]
    fn point_leafnum_walks_the_split() {
        let topology = two_leaf_topology();
        assert_eq!(topology.point_leafnum(vec3(0.0, 0.0, 5.0)).expect("leaf"), 0);
        assert_eq!(topology.point_leafnum(vec3(0.0, 0.0, -5.0)).expect("leaf"), 1);
        let error = topology
            .point_leafnum(vec3(f32::NAN, 0.0, 0.0))
            .expect_err("NaN must fail");
        assert_eq!(error.to_string(), "point leaf query requires finite coordinates");
    }

    #[test]
    fn box_leafnums_reports_branching_and_overflow() {
        let topology = two_leaf_topology();
        let spanning = Bounds {
            min: vec3(-8.0, -8.0, -8.0),
            max: vec3(8.0, 8.0, 8.0),
        };
        let list = topology.box_leafnums(&spanning, 1024).expect("leaves");
        assert_eq!(list.leaves.len(), 2);
        assert_eq!(list.topnode, Some(0));
        assert!(!list.overflowed);
        let upper = Bounds {
            min: vec3(-8.0, -8.0, 1.0),
            max: vec3(8.0, 8.0, 8.0),
        };
        let list = topology.box_leafnums(&upper, 1024).expect("leaves");
        assert_eq!(list.leaves, [0]);
        assert_eq!(list.topnode, None);
        let list = topology.box_leafnums(&spanning, 1).expect("leaves");
        assert_eq!(list.leaves.len(), 1);
        assert!(list.overflowed);
        assert_eq!(list.last_leaf, 1);
        let bad = Bounds {
            min: vec3(8.0, 0.0, 0.0),
            max: vec3(-8.0, 0.0, 0.0),
        };
        let error = topology.box_leafnums(&bad, 8).expect_err("unordered must fail");
        assert_eq!(error.to_string(), "box leaf query requires finite ordered bounds");
    }

    #[test]
    fn leaf_queries_validate() {
        let topology = two_leaf_topology();
        assert_eq!(topology.leaf_area(1).expect("area"), 1);
        assert_eq!(topology.leaf_cluster(0).expect("cluster"), 0);
        let error = topology.leaf_area(2).expect_err("leaf 2 is missing");
        assert_eq!(error.to_string(), "CM_LeafArea: bad number");
        let error = topology.leaf_cluster(-1).expect_err("leaf -1 is missing");
        assert_eq!(error.to_string(), "CM_LeafCluster: bad number");
    }

    #[test]
    fn cluster_visibility_reads_pvs_rows() {
        let topology = two_leaf_topology();
        assert!(topology.cluster_visible(0, 1).expect("visible"));
        assert!(topology.cluster_visible(1, 0).expect("visible"));
        assert!(!topology.cluster_visible(0, 2).expect("outside"));
        assert!(!topology.cluster_visible(0, -1).expect("negative"));
        let error = topology
            .cluster_pvs(0)
            .byte_at(1_000_000)
            .expect_err("far offset must fail");
        assert!(error.to_string().starts_with("PVS byte 1000000 outside"), "{error}");
        let error = topology
            .cluster_pvs(-1)
            .byte_at(-1)
            .expect_err("negative index must fail");
        assert!(error.to_string().starts_with("PVS byte -1 outside"), "{error}");
    }

    #[test]
    fn portals_flood_and_reflood() {
        let topology = two_leaf_topology();
        assert!(!topology.areas_connected(0, 1, None).expect("separate"));
        topology.adjust_area_portal_state(0, 1, true).expect("open");
        assert!(topology.areas_connected(0, 1, None).expect("joined"));
        topology.adjust_area_portal_state(0, 1, false).expect("close");
        assert!(!topology.areas_connected(0, 1, None).expect("separate"));
        let error = topology
            .adjust_area_portal_state(0, 1, false)
            .expect_err("double close must fail");
        assert_eq!(error.to_string(), "CM_AdjustAreaPortalState: negative reference count");
        topology
            .adjust_area_portal_state(-1, 0, true)
            .expect("negative is a no-op");
        let error = topology
            .adjust_area_portal_state(0, 2, true)
            .expect_err("area 2 is missing");
        assert_eq!(error.to_string(), "CM_ChangeAreaPortalState: bad area number");
        let error = topology.areas_connected(0, 2, None).expect_err("area 2 is missing");
        assert_eq!(error.to_string(), "area >= cm.numAreas");
        assert!(!topology.areas_connected(-1, 0, None).expect("negative area"));
        assert!(topology.areas_connected(0, 1, Some(true)).expect("override"));
        topology.set_no_areas(true);
        assert!(topology.areas_connected(0, 1, None).expect("flag"));
    }

    #[test]
    fn portal_checkpoint_round_trips() {
        let topology = two_leaf_topology();
        topology.adjust_area_portal_state(0, 1, true).expect("open");
        let checkpoint = topology.capture_portal_checkpoint();
        topology.adjust_area_portal_state(0, 1, false).expect("close");
        assert!(!topology.areas_connected(0, 1, None).expect("separate"));
        topology.restore_portal_checkpoint(&checkpoint).expect("restore");
        assert!(topology.areas_connected(0, 1, None).expect("joined"));

        let mut asymmetric = checkpoint.clone();
        if let SaveJson::Object(members) = &mut asymmetric {
            for (name, value) in members.iter_mut() {
                if name == "portals" {
                    *value = arr(vec![int(0), int(1), int(0), int(0)]);
                }
            }
        }
        let error = topology
            .restore_portal_checkpoint(&asymmetric)
            .expect_err("asymmetric portals must fail");
        assert!(error.to_string().contains("asymmetric source portal"), "{error}");

        let short = obj(vec![
            ("version", int(1)),
            ("floodValid", int(1)),
            ("noAreas", boolean(false)),
            ("areas", arr(vec![])),
            ("portals", arr(vec![])),
        ]);
        let error = topology
            .restore_portal_checkpoint(&short)
            .expect_err("short checkpoint must fail");
        assert!(error.to_string().contains("another area table"), "{error}");

        let bad_version = obj(vec![
            ("version", int(2)),
            ("floodValid", int(1)),
            ("noAreas", boolean(false)),
            ("areas", arr(vec![])),
            ("portals", arr(vec![])),
        ]);
        assert!(topology.restore_portal_checkpoint(&bad_version).is_err());
    }

    #[test]
    fn area_bits_cover_floods_and_overrides() {
        let topology = two_leaf_topology();
        assert_eq!(topology.area_bits(0, None).expect("bits"), [0b01]);
        topology.adjust_area_portal_state(0, 1, true).expect("open");
        assert_eq!(topology.area_bits(0, None).expect("bits"), [0b11]);
        assert_eq!(topology.area_bits(-1, None).expect("all"), [0xff]);
        let mut short = Vec::new();
        let error = topology
            .write_area_bits(&mut short, 0, None)
            .expect_err("short buffer must fail");
        assert_eq!(error.to_string(), "area bits need 1 bytes");
        let error = topology.area_bits(7, None).expect_err("area 7 is missing");
        assert_eq!(error.to_string(), "invalid area 7");
    }

    #[test]
    fn box_brushes_dedup_and_filter() {
        let mut geometry = two_leaf_geometry();
        for (normal, distance) in [
            (vec3(1.0, 0.0, 0.0), 64.0),
            (vec3(-1.0, 0.0, 0.0), 64.0),
            (vec3(0.0, 1.0, 0.0), 64.0),
            (vec3(0.0, -1.0, 0.0), 64.0),
            (vec3(0.0, 0.0, 1.0), 64.0),
            (vec3(0.0, 0.0, -1.0), 64.0),
        ] {
            geometry.planes.push(Plane { normal, distance });
        }
        geometry.brushes.push(Q3CollisionBrushInput {
            shader: 0,
            sides: IndexRange { first: 0, count: 6 },
        });
        for plane in 1..7 {
            geometry
                .brush_sides
                .push(Q3CollisionBrushSideInput { plane, shader: 0 });
        }
        geometry.leaves[0].brushes = IndexRange { first: 0, count: 1 };
        geometry.leaf_brushes = vec![0, 0];
        geometry.leaves[1].brushes = IndexRange { first: 1, count: 1 };
        let map = decoded_collision_map(&geometry, None).expect("decode");
        let topology = CollisionTopology::new(Rc::new(map), Rc::new(CollisionCounters::new()));
        let spanning = Bounds {
            min: vec3(-80.0, -80.0, -80.0),
            max: vec3(80.0, 80.0, 80.0),
        };
        assert_eq!(topology.box_brushes(&spanning, 1024).expect("brushes"), [0]);
        let far = Bounds {
            min: vec3(1000.0, 1000.0, 1000.0),
            max: vec3(1008.0, 1008.0, 1008.0),
        };
        assert!(topology.box_brushes(&far, 1024).expect("brushes").is_empty());
        assert!(topology.box_brushes(&spanning, 0).expect("brushes").is_empty());
    }
}
