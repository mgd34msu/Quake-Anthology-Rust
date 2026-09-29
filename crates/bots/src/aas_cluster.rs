//! Source clustering and portal discovery from id Software
//! `be_aas_cluster.c`, ported from `src/bots/navigation/aas-cluster.ts`.
//! Copyright (C) 1999-2005 Id Software, Inc.

use crate::aas::{AasAreaSettings, AasAsset, AasCluster, AasPortal};
use crate::error::{indexed, indexed_mut, BotsError};

const MAX_PORTALS: usize = 65536;
const MAX_PORTAL_INDEX: usize = 65536;
const MAX_CLUSTERS: usize = 65536;
const MAX_PORTAL_AREAS: usize = 1024;
const CLUSTER_PORTAL: i32 = 8;
const ROUTE_PORTAL: i32 = 32;
const VIEW_PORTAL: i32 = 512;
const AREA_GROUNDED: i32 = 1;
const FACE_SOLID: i32 = 1;

fn portal_area_push(values: &mut Vec<i32>, value: i32, allocation: &'static str) -> Result<(), BotsError> {
    if values.len() == MAX_PORTAL_AREAS {
        return Err(BotsError::PortalAreaOverflow { allocation });
    }
    values.push(value);
    Ok(())
}

/// AAS_ConnectedAreas_r and AAS_ConnectedAreas.
fn connected_areas(asset: &AasAsset, area_numbers: &[i32]) -> Result<bool, BotsError> {
    if area_numbers.is_empty() {
        return Ok(false);
    }
    if area_numbers.len() == 1 {
        return Ok(true);
    }
    let mut connected = vec![0u8; MAX_PORTAL_AREAS];
    let mut pending = vec![0usize];
    while let Some(current) = pending.pop() {
        if *indexed(&connected, current as i64, "AAS clustering index")? != 0 {
            continue;
        }
        *indexed_mut(&mut connected, current as i64, "AAS clustering index")? = 1;
        let area_number = *indexed(area_numbers, current as i64, "AAS clustering index")?;
        let area = indexed(&asset.areas, i64::from(area_number), "AAS clustering index")?;
        // Reverse pushes retain the recursive source face order.
        for face_index in (0..area.face_count).rev() {
            let face = indexed(
                &asset.faces,
                indexed(
                    &asset.face_indexes,
                    i64::from(area.first_face + face_index),
                    "AAS clustering index",
                )?
                .unsigned_abs() as i64,
                "AAS clustering index",
            )?;
            if (face.flags & FACE_SOLID) != 0 {
                continue;
            }
            let other_area = if face.front_area != area_number {
                face.front_area
            } else {
                face.back_area
            };
            if let Some(other_index) = area_numbers.iter().position(|area| *area == other_area) {
                if *indexed(&connected, other_index as i64, "AAS clustering index")? == 0 {
                    pending.push(other_index);
                }
            }
        }
    }
    for index in 0..area_numbers.len() {
        if *indexed(&connected, index as i64, "AAS clustering index")? == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

enum FloodFrame {
    Enter(i32),
    Faces {
        area_number: i32,
        face_count: i32,
        first_face: i32,
        index: i32,
    },
    Reachabilities {
        area_number: i32,
        index: i32,
    },
}

/// Borrows the actual map allocations; initialization publishes every
/// source write. The donor splits this into a world record and a
/// clustering pass; the single struct keeps Rust borrow scopes small.
struct AasClustering<'a> {
    asset: &'a AasAsset,
    settings: Vec<AasAreaSettings>,
    portals: Vec<AasPortal>,
    clusters: Vec<AasCluster>,
    portal_indexes: Vec<i32>,
    num_portals: usize,
    num_clusters: usize,
    portal_index_size: usize,
    no_face_flood: bool,
    print: &'a dyn Fn(&str),
}

impl<'a> AasClustering<'a> {
    fn new(asset: &'a AasAsset, print: &'a dyn Fn(&str)) -> Self {
        Self {
            asset,
            settings: asset.settings.clone(),
            portals: Vec::new(),
            clusters: Vec::new(),
            portal_indexes: Vec::new(),
            num_portals: 0,
            num_clusters: 0,
            portal_index_size: 0,
            no_face_flood: true,
            print,
        }
    }

    fn log(&self, text: &str) {
        (self.print)(text);
    }

    fn area_settings(&self, index: i32) -> Result<&AasAreaSettings, BotsError> {
        indexed(&self.settings, i64::from(index), "AAS clustering index")
    }

    fn area_settings_mut(&mut self, index: i32) -> Result<&mut AasAreaSettings, BotsError> {
        indexed_mut(&mut self.settings, i64::from(index), "AAS clustering index")
    }

    fn remove_cluster_areas(&mut self) -> Result<(), BotsError> {
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            self.area_settings_mut(area)?.cluster = 0;
        }
        Ok(())
    }

    /// The donor defines but never calls this pass; keep the port.
    #[allow(dead_code)]
    fn clear_cluster(&mut self, cluster: i32) -> Result<(), BotsError> {
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            let setting = self.area_settings_mut(area)?;
            if setting.cluster == cluster {
                setting.cluster = 0;
            }
        }
        Ok(())
    }

    /// The donor defines but never calls this pass; keep the port.
    #[allow(dead_code)]
    fn remove_portals_cluster_reference(&mut self, cluster: i32) -> Result<(), BotsError> {
        for portal_number in 1..self.num_portals {
            let portal = indexed_mut(&mut self.portals, portal_number as i64, "AAS clustering index")?;
            if portal.front_cluster == cluster {
                portal.front_cluster = 0;
            }
            if portal.back_cluster == cluster {
                portal.back_cluster = 0;
            }
        }
        Ok(())
    }

    fn update_portal(&mut self, area: i32, cluster_number: i32) -> Result<bool, BotsError> {
        let mut portal_number = 1;
        while portal_number < self.num_portals {
            if indexed(&self.portals, portal_number as i64, "AAS clustering index")?.area == area {
                break;
            }
            portal_number += 1;
        }
        if portal_number == self.num_portals {
            self.log(&format!("no portal of area {area}"));
            return Ok(true);
        }
        let portal = indexed_mut(&mut self.portals, portal_number as i64, "AAS clustering index")?;
        if portal.front_cluster == cluster_number || portal.back_cluster == cluster_number {
            return Ok(true);
        }
        if portal.front_cluster == 0 {
            portal.front_cluster = cluster_number;
        } else if portal.back_cluster == 0 {
            portal.back_cluster = cluster_number;
        } else {
            self.area_settings_mut(area)?.contents &= !CLUSTER_PORTAL;
            self.log(&format!("portal area {area} is seperating more than two clusters\r\n"));
            return Ok(false);
        }
        if self.portal_index_size >= MAX_PORTAL_INDEX {
            self.log("AAS_MAX_PORTALINDEXSIZE");
            return Ok(true);
        }
        self.area_settings_mut(area)?.cluster = -(portal_number as i32);
        let cluster = indexed_mut(&mut self.clusters, i64::from(cluster_number), "AAS clustering index")?;
        let slot = cluster.first_portal + cluster.portal_count;
        cluster.portal_count += 1;
        *indexed_mut(&mut self.portal_indexes, i64::from(slot), "AAS clustering index")? = portal_number as i32;
        self.portal_index_size += 1;
        Ok(true)
    }

    /// AAS_FloodClusterAreas_r, with continuations replacing the C call stack.
    fn flood_cluster_areas(&mut self, area_number: i32, cluster_number: i32) -> Result<bool, BotsError> {
        let mut pending = vec![FloodFrame::Enter(area_number)];
        while let Some(frame) = pending.pop() {
            match frame {
                FloodFrame::Enter(area_number) => {
                    if area_number <= 0 || area_number >= self.asset.areas.len() as i32 {
                        self.log("AAS_FloodClusterAreas_r: areanum out of range");
                        return Ok(false);
                    }
                    let cluster = self.area_settings(area_number)?.cluster;
                    if cluster > 0 {
                        if cluster == cluster_number {
                            continue;
                        }
                        self.log(&format!(
                            "cluster {cluster_number} touched cluster {cluster} at area {area_number}\r\n"
                        ));
                        return Ok(false);
                    }
                    if (self.area_settings(area_number)?.contents & CLUSTER_PORTAL) != 0 {
                        if !self.update_portal(area_number, cluster_number)? {
                            return Ok(false);
                        }
                        continue;
                    }
                    let area = *indexed(&self.asset.areas, i64::from(area_number), "AAS clustering index")?;
                    self.area_settings_mut(area_number)?.cluster = cluster_number;
                    let cluster_record =
                        indexed_mut(&mut self.clusters, i64::from(cluster_number), "AAS clustering index")?;
                    let cluster_area = cluster_record.area_count;
                    cluster_record.area_count += 1;
                    self.area_settings_mut(area_number)?.cluster_area = cluster_area;
                    if self.no_face_flood {
                        pending.push(FloodFrame::Reachabilities { area_number, index: 0 });
                    } else {
                        pending.push(FloodFrame::Faces {
                            area_number,
                            face_count: area.face_count,
                            first_face: area.first_face,
                            index: 0,
                        });
                    }
                }
                FloodFrame::Faces {
                    area_number,
                    face_count,
                    first_face,
                    index,
                } => {
                    if index >= face_count {
                        pending.push(FloodFrame::Reachabilities { area_number, index: 0 });
                        continue;
                    }
                    let face = *indexed(
                        &self.asset.faces,
                        indexed(
                            &self.asset.face_indexes,
                            i64::from(first_face + index),
                            "AAS clustering index",
                        )?
                        .unsigned_abs() as i64,
                        "AAS clustering index",
                    )?;
                    pending.push(FloodFrame::Faces {
                        area_number,
                        face_count,
                        first_face,
                        index: index + 1,
                    });
                    let other_area = if face.front_area == area_number {
                        face.back_area
                    } else {
                        face.front_area
                    };
                    if other_area != 0 {
                        pending.push(FloodFrame::Enter(other_area));
                    }
                }
                FloodFrame::Reachabilities { area_number, index } => {
                    let setting = *self.area_settings(area_number)?;
                    if index >= setting.reach_count {
                        continue;
                    }
                    let other_area = indexed(
                        &self.asset.reachability,
                        i64::from(setting.first_reach + index),
                        "AAS clustering index",
                    )?
                    .area;
                    pending.push(FloodFrame::Reachabilities {
                        area_number,
                        index: index + 1,
                    });
                    if other_area != 0 {
                        pending.push(FloodFrame::Enter(other_area));
                    }
                }
            }
        }
        Ok(true)
    }

    fn flood_cluster_areas_using_reachabilities(&mut self, cluster_number: i32) -> Result<bool, BotsError> {
        let areas = self.asset.areas.len() as i32;
        let mut area = 1;
        while area < areas {
            let setting = *self.area_settings(area)?;
            if setting.cluster == 0 && (setting.contents & CLUSTER_PORTAL) == 0 {
                let mut touched = false;
                for index in 0..setting.reach_count {
                    let target = indexed(
                        &self.asset.reachability,
                        i64::from(setting.first_reach + index),
                        "AAS clustering index",
                    )?
                    .area;
                    let other = *self.area_settings(target)?;
                    if (other.contents & CLUSTER_PORTAL) != 0 {
                        continue;
                    }
                    if other.cluster != 0 {
                        if !self.flood_cluster_areas(area, cluster_number)? {
                            return Ok(false);
                        }
                        touched = true;
                        break;
                    }
                }
                if touched {
                    area = 0;
                }
            }
            area += 1;
        }
        Ok(true)
    }

    /// The donor defines but never calls this pass; keep the port.
    #[allow(dead_code)]
    fn number_cluster_portals(&mut self, cluster_number: i32) -> Result<(), BotsError> {
        let cluster = *indexed(&self.clusters, i64::from(cluster_number), "AAS clustering index")?;
        for index in 0..cluster.portal_count {
            let portal_number = *indexed(
                &self.portal_indexes,
                i64::from(cluster.first_portal + index),
                "AAS clustering index",
            )?;
            let portal = indexed_mut(&mut self.portals, i64::from(portal_number), "AAS clustering index")?;
            let side = usize::from(portal.front_cluster != cluster_number);
            let cluster = indexed_mut(&mut self.clusters, i64::from(cluster_number), "AAS clustering index")?;
            portal.cluster_areas[side] = cluster.area_count;
            cluster.area_count += 1;
        }
        Ok(())
    }

    fn number_cluster_areas(&mut self, cluster_number: i32) -> Result<(), BotsError> {
        {
            let cluster = indexed_mut(&mut self.clusters, i64::from(cluster_number), "AAS clustering index")?;
            cluster.area_count = 0;
            cluster.reachability_area_count = 0;
        }
        for with_reachabilities in [true, false] {
            let areas = self.asset.areas.len() as i32;
            for area in 1..areas {
                let setting = *self.area_settings(area)?;
                if setting.cluster != cluster_number || (self.area_reachability(area)? != 0) != with_reachabilities {
                    continue;
                }
                let cluster = indexed_mut(&mut self.clusters, i64::from(cluster_number), "AAS clustering index")?;
                let cluster_area = cluster.area_count;
                cluster.area_count += 1;
                if with_reachabilities {
                    cluster.reachability_area_count += 1;
                }
                self.area_settings_mut(area)?.cluster_area = cluster_area;
            }
            let cluster = *indexed(&self.clusters, i64::from(cluster_number), "AAS clustering index")?;
            for index in 0..cluster.portal_count {
                let portal_number = *indexed(
                    &self.portal_indexes,
                    i64::from(cluster.first_portal + index),
                    "AAS clustering index",
                )?;
                let portal = *indexed(&self.portals, i64::from(portal_number), "AAS clustering index")?;
                if (self.area_reachability(portal.area)? != 0) != with_reachabilities {
                    continue;
                }
                let side = usize::from(portal.front_cluster != cluster_number);
                let cluster = indexed_mut(&mut self.clusters, i64::from(cluster_number), "AAS clustering index")?;
                let cluster_area = cluster.area_count;
                cluster.area_count += 1;
                if with_reachabilities {
                    cluster.reachability_area_count += 1;
                }
                indexed_mut(&mut self.portals, i64::from(portal_number), "AAS clustering index")?.cluster_areas[side] =
                    cluster_area;
            }
        }
        Ok(())
    }

    fn find_clusters(&mut self) -> Result<bool, BotsError> {
        self.remove_cluster_areas()?;
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            let setting = *self.area_settings(area)?;
            if setting.cluster != 0 {
                continue;
            }
            if self.no_face_flood && setting.reach_count == 0 {
                continue;
            }
            if (setting.contents & CLUSTER_PORTAL) != 0 {
                continue;
            }
            if self.num_clusters >= MAX_CLUSTERS {
                self.log("AAS_MAX_CLUSTERS");
                return Ok(false);
            }
            {
                let cluster = indexed_mut(&mut self.clusters, self.num_clusters as i64, "AAS clustering index")?;
                cluster.area_count = 0;
                cluster.reachability_area_count = 0;
                cluster.first_portal = self.portal_index_size as i32;
                cluster.portal_count = 0;
            }
            if !self.flood_cluster_areas(area, self.num_clusters as i32)? {
                return Ok(false);
            }
            if !self.flood_cluster_areas_using_reachabilities(self.num_clusters as i32)? {
                return Ok(false);
            }
            self.number_cluster_areas(self.num_clusters as i32)?;
            self.num_clusters += 1;
        }
        Ok(true)
    }

    fn create_portals(&mut self) -> Result<(), BotsError> {
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            if (self.area_settings(area)?.contents & CLUSTER_PORTAL) == 0 {
                continue;
            }
            if self.num_portals >= MAX_PORTALS {
                self.log("AAS_MAX_PORTALS");
                return Ok(());
            }
            let portal = indexed_mut(&mut self.portals, self.num_portals as i64, "AAS clustering index")?;
            portal.area = area;
            portal.front_cluster = 0;
            portal.back_cluster = 0;
            self.num_portals += 1;
        }
        Ok(())
    }

    /// AAS_GetAdjacentAreasWithLessPresenceTypes_r, with an explicit
    /// stack replacing the C recursion.
    fn adjacent_areas_with_less_presence_types(
        &mut self,
        area_numbers: &mut Vec<i32>,
        area_number: i32,
    ) -> Result<(), BotsError> {
        portal_area_push(area_numbers, area_number, "areanums")?;
        let mut stack = vec![(area_number, 0)];
        while let Some((current, face_index)) = stack.pop() {
            let area = *indexed(&self.asset.areas, i64::from(current), "AAS clustering index")?;
            if face_index < area.face_count {
                stack.push((current, face_index + 1));
                let face = *indexed(
                    &self.asset.faces,
                    indexed(
                        &self.asset.face_indexes,
                        i64::from(area.first_face + face_index),
                        "AAS clustering index",
                    )?
                    .unsigned_abs() as i64,
                    "AAS clustering index",
                )?;
                if (face.flags & FACE_SOLID) != 0 {
                    continue;
                }
                let other_area = if face.front_area != current {
                    face.front_area
                } else {
                    face.back_area
                };
                let presence = self.area_settings(current)?.presence;
                let other_presence = self.area_settings(other_area)?.presence;
                if (presence & !other_presence) == 0
                    || (other_presence & !presence) != 0
                    || area_numbers.contains(&other_area)
                {
                    continue;
                }
                if area_numbers.len() >= MAX_PORTAL_AREAS {
                    self.log("MAX_PORTALAREAS");
                    return Ok(());
                }
                portal_area_push(area_numbers, other_area, "areanums")?;
                stack.push((other_area, 0));
            }
        }
        Ok(())
    }

    fn check_area_for_possible_portals(&mut self, area_number: i32) -> Result<usize, BotsError> {
        let settings = *self.area_settings(area_number)?;
        if (settings.contents & CLUSTER_PORTAL) != 0 || (settings.flags & AREA_GROUNDED) == 0 {
            return Ok(0);
        }
        let mut area_numbers: Vec<i32> = Vec::new();
        let mut front_face_counts = vec![0i32; MAX_PORTAL_AREAS];
        let mut back_face_counts = vec![0i32; MAX_PORTAL_AREAS];
        let mut front_faces: Vec<i32> = Vec::new();
        let mut back_faces: Vec<i32> = Vec::new();
        let mut front_areas: Vec<i32> = Vec::new();
        let mut back_areas: Vec<i32> = Vec::new();
        let mut front_plane = -1;
        let mut back_plane = -1;
        self.adjacent_areas_with_less_presence_types(&mut area_numbers, area_number)?;
        for index in 0..area_numbers.len() {
            let current_area = area_numbers[index];
            let area = *indexed(&self.asset.areas, i64::from(current_area), "AAS clustering index")?;
            for face_index in 0..area.face_count {
                let face_number = indexed(
                    &self.asset.face_indexes,
                    i64::from(area.first_face + face_index),
                    "AAS clustering index",
                )?
                .unsigned_abs() as i32;
                let face = *indexed(&self.asset.faces, i64::from(face_number), "AAS clustering index")?;
                if (face.flags & FACE_SOLID) != 0 {
                    continue;
                }
                if area_numbers.iter().enumerate().any(|(other_index, other)| {
                    other_index != index && (face.front_area == *other || face.back_area == *other)
                }) {
                    continue;
                }
                let other_area = if face.front_area == current_area {
                    face.back_area
                } else {
                    face.front_area
                };
                if (self.area_settings(other_area)?.contents & CLUSTER_PORTAL) != 0 {
                    return Ok(0);
                }
                let plane = face.plane & !1;
                if front_plane < 0 || plane == front_plane {
                    front_plane = plane;
                    portal_area_push(&mut front_faces, face_number, "frontfacenums")?;
                    if !front_areas.contains(&other_area) {
                        portal_area_push(&mut front_areas, other_area, "frontareanums")?;
                    }
                    front_face_counts[index] += 1;
                } else if back_plane < 0 || plane == back_plane {
                    back_plane = plane;
                    portal_area_push(&mut back_faces, face_number, "backfacenums")?;
                    if !back_areas.contains(&other_area) {
                        portal_area_push(&mut back_areas, other_area, "backareanums")?;
                    }
                    back_face_counts[index] += 1;
                } else {
                    return Ok(0);
                }
            }
        }
        for index in 0..area_numbers.len() {
            if front_face_counts[index] == 0 || back_face_counts[index] == 0 {
                return Ok(0);
            }
        }
        if !connected_areas(self.asset, &front_areas)? || !connected_areas(self.asset, &back_areas)? {
            return Ok(0);
        }
        for front_number in &front_faces {
            let front = *indexed(&self.asset.faces, i64::from(*front_number), "AAS clustering index")?;
            for front_edge in 0..front.edge_count {
                let edge_number = indexed(
                    &self.asset.edge_indexes,
                    i64::from(front.first_edge + front_edge),
                    "AAS clustering index",
                )?
                .unsigned_abs();
                for back_number in &back_faces {
                    let back = *indexed(&self.asset.faces, i64::from(*back_number), "AAS clustering index")?;
                    for back_edge in 0..back.edge_count {
                        if edge_number
                            == indexed(
                                &self.asset.edge_indexes,
                                i64::from(back.first_edge + back_edge),
                                "AAS clustering index",
                            )?
                            .unsigned_abs()
                        {
                            return Ok(0);
                        }
                    }
                }
            }
        }
        for &area in &area_numbers {
            let setting = self.area_settings_mut(area)?;
            setting.contents |= CLUSTER_PORTAL;
            setting.contents |= ROUTE_PORTAL;
            self.log(&format!("possible portal: {area}\r\n"));
        }
        Ok(area_numbers.len())
    }

    fn find_possible_portals(&mut self) -> Result<(), BotsError> {
        let mut count = 0i32;
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            count = count.wrapping_add(self.check_area_for_possible_portals(area)? as i32);
        }
        self.log(&format!("\r{count:>6} possible portal areas\n"));
        Ok(())
    }

    /// The donor defines but never calls this pass; keep the port.
    #[allow(dead_code)]
    fn remove_all_portals(&mut self) -> Result<(), BotsError> {
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            self.area_settings_mut(area)?.contents &= !CLUSTER_PORTAL;
        }
        Ok(())
    }

    fn test_portals(&mut self) -> Result<bool, BotsError> {
        for portal_number in 1..self.num_portals {
            let portal = *indexed(&self.portals, portal_number as i64, "AAS clustering index")?;
            if portal.front_cluster == 0 {
                self.area_settings_mut(portal.area)?.contents &= !CLUSTER_PORTAL;
                self.log(&format!("portal area {} has no front cluster\r\n", portal.area));
                return Ok(false);
            }
            if portal.back_cluster == 0 {
                self.area_settings_mut(portal.area)?.contents &= !CLUSTER_PORTAL;
                self.log(&format!("portal area {} has no back cluster\r\n", portal.area));
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn count_forced_cluster_portals(&mut self) -> Result<(), BotsError> {
        let mut count = 0;
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            if (self.area_settings(area)?.contents & CLUSTER_PORTAL) == 0 {
                continue;
            }
            self.log(&format!("area {area} is a forced portal area\r\n"));
            count += 1;
        }
        self.log(&format!("{count:>6} forced portal areas\n"));
        Ok(())
    }

    fn create_view_portals(&mut self) -> Result<(), BotsError> {
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            if (self.area_settings(area)?.contents & CLUSTER_PORTAL) != 0 {
                self.area_settings_mut(area)?.contents |= VIEW_PORTAL;
            }
        }
        Ok(())
    }

    fn set_view_portals_as_cluster_portals(&mut self) -> Result<(), BotsError> {
        let areas = self.asset.areas.len() as i32;
        for area in 1..areas {
            if (self.area_settings(area)?.contents & VIEW_PORTAL) != 0 {
                self.area_settings_mut(area)?.contents |= CLUSTER_PORTAL;
            }
        }
        Ok(())
    }

    /// AAS_InitClustering, called only after the runtime has loaded this
    /// world. The donor forces clustering here; the variable gate is
    /// folded away.
    fn initialize(&mut self) -> Result<(), BotsError> {
        self.set_view_portals_as_cluster_portals()?;
        self.count_forced_cluster_portals()?;
        self.remove_cluster_areas()?;
        self.find_possible_portals()?;
        self.create_view_portals()?;
        self.portals = vec![
            AasPortal {
                area: 0,
                front_cluster: 0,
                back_cluster: 0,
                cluster_areas: [0, 0],
            };
            MAX_PORTALS
        ];
        self.portal_indexes = vec![0; MAX_PORTAL_INDEX];
        self.clusters = vec![
            AasCluster {
                area_count: 0,
                reachability_area_count: 0,
                portal_count: 0,
                first_portal: 0,
            };
            MAX_CLUSTERS
        ];
        let mut removed_portal_areas = 0i32;
        self.log(&format!("\r{removed_portal_areas:>6} removed portal areas"));
        loop {
            self.log(&format!("\r{removed_portal_areas:>6}"));
            self.num_portals = 1;
            self.portal_index_size = 0;
            self.num_clusters = 1;
            self.create_portals()?;
            removed_portal_areas = removed_portal_areas.wrapping_add(1);
            if !self.find_clusters()? {
                continue;
            }
            if !self.test_portals()? {
                continue;
            }
            break;
        }
        self.log("\n");
        for portal in 1..self.num_portals {
            let area = indexed(&self.portals, portal as i64, "AAS clustering index")?.area;
            self.log(&format!("portal {portal}: area {area}\r\n"));
        }
        self.log(&format!("{:>6} portals created\n", self.num_portals));
        self.log(&format!("{:>6} clusters created\n", self.num_clusters));
        for cluster in 1..self.num_clusters {
            let count = indexed(&self.clusters, cluster as i64, "AAS clustering index")?.reachability_area_count;
            self.log(&format!("cluster {cluster} has {count} reachability areas\n"));
        }
        let mut reachability_areas = 0i32;
        let mut total = 0i32;
        for cluster in 0..self.num_clusters {
            let count = indexed(&self.clusters, cluster as i64, "AAS clustering index")?.reachability_area_count;
            reachability_areas = reachability_areas.wrapping_add(count);
            total = total.wrapping_add(count.wrapping_mul(count));
        }
        total = total.wrapping_add(reachability_areas.wrapping_mul(self.num_portals as i32));
        self.log(&format!("{reachability_areas:>6} total reachability areas\n"));
        self.log(&format!(
            "{:>6} AAS memory/CPU usage (the lower the better)\n",
            total.wrapping_mul(3)
        ));
        Ok(())
    }

    fn area_reachability(&self, area: i32) -> Result<i32, BotsError> {
        if area < 0 || area >= self.asset.areas.len() as i32 {
            self.log(&format!("AAS_AreaReachability: areanum {area} out of range"));
            return Ok(0);
        }
        Ok(self.area_settings(area)?.reach_count)
    }
}

/// Rebuild clusters on borrowed area geometry and reachabilities; no
/// engine heap or VFS is introduced.
pub fn cluster_aas(asset: &AasAsset, print: Option<&dyn Fn(&str)>) -> Result<AasAsset, BotsError> {
    let noop = |_: &str| {};
    let print = print.unwrap_or(&noop);
    let mut clustering = AasClustering::new(asset, print);
    clustering.initialize()?;
    Ok(AasAsset {
        lumps: Vec::new(),
        settings: clustering.settings,
        portals: clustering.portals[..clustering.num_portals].to_vec(),
        portal_indexes: clustering.portal_indexes[..clustering.portal_index_size].to_vec(),
        clusters: clustering.clusters[..clustering.num_clusters].to_vec(),
        ..asset.clone()
    })
}
