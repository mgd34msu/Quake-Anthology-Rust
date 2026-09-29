//! AAS estimate routing from id Software's `be_aas_route.c`,
//! ported from `src/bots/navigation/estimate-aas.ts`. Source
//! centiseconds are estimates, never movement admission.
//! Copyright (C) 1999-2005 Id Software, Inc.

use std::collections::{HashMap, VecDeque};

use qa_core::math::Vec3;

use crate::aas::{AasAreaSettings, AasAsset};
use crate::error::{indexed, BotsError};
use crate::estimates::Eligibility;
use crate::graph::{aas_area_travel_flags, aas_travel_flag};
use crate::types::{NavigationEdge, NavigationEstimateQuery, NavigationEstimateResult, NavigationGraph};

const DEFAULT_TRAVEL_FLAGS: i32 = 0x011c_0fbe;
const CACHE_BUDGET: usize = 16 * 1024 * 1024;

fn metadata<T>(values: &[T], index: i64) -> Result<&T, BotsError> {
    indexed(values, index, "Missing AAS estimate metadata")
}

/// Source intra-area travel-time estimate.
pub fn aas_estimate_area_time(setting: &AasAreaSettings, start: Vec3, end: Vec3) -> Result<u16, BotsError> {
    let x = start.x - end.x;
    let y = start.y - end.y;
    let z = start.z - end.z;
    let distance = f64::from((x * x + y * y) + z * z).sqrt() as f32;
    let factor: f32 = if (setting.presence & 2) == 0 {
        1.3
    } else if (setting.flags & 4) != 0 {
        1.0
    } else {
        0.33
    };
    let value = f64::from((f64::from(distance) * f64::from(factor)) as f32);
    if !value.is_finite() || value > 2_147_483_647.0 {
        return Err(BotsError::EstimateRange);
    }
    Ok((value.trunc().max(1.0) as u32 & 0xffff) as u16)
}

#[derive(Debug, Clone, Copy)]
struct Incoming {
    area: i32,
    reach: i32,
}

struct Topology {
    incoming: Vec<Vec<Incoming>>,
    crossings: Vec<Vec<Vec<u16>>>,
    portal_maxima: Vec<u16>,
}

#[derive(Debug, Clone)]
struct Cache {
    times: Vec<u16>,
    reaches: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Update {
    area: i32,
    cluster: i32,
    time: u16,
    row: Vec<u16>,
    queued: bool,
}

fn build_topology(asset: &AasAsset) -> Result<Topology, BotsError> {
    let mut incoming: Vec<Vec<Incoming>> = vec![Vec::new(); asset.areas.len()];
    for area in 1..asset.areas.len() as i64 {
        let setting = metadata(&asset.settings, area)?;
        for offset in 0..setting.reach_count.min(128) {
            let reach = setting.first_reach + offset;
            let target = metadata(&asset.reachability, i64::from(reach))?.area;
            metadata(&incoming, i64::from(target))?;
            incoming[target as usize].push(Incoming {
                area: area as i32,
                reach,
            });
        }
    }
    for links in &mut incoming {
        links.reverse();
    }
    let mut crossings: Vec<Vec<Vec<u16>>> = Vec::with_capacity(asset.settings.len());
    for (area, setting) in asset.settings.iter().enumerate() {
        let mut rows = Vec::with_capacity(setting.reach_count.max(0) as usize);
        for offset in 0..setting.reach_count {
            let start = metadata(&asset.reachability, i64::from(setting.first_reach + offset))?.start;
            let mut row = Vec::with_capacity(incoming[area].len());
            for link in &incoming[area] {
                row.push(aas_estimate_area_time(
                    setting,
                    metadata(&asset.reachability, i64::from(link.reach))?.end,
                    start,
                )?);
            }
            rows.push(row);
        }
        crossings.push(rows);
    }
    let mut portal_maxima = Vec::with_capacity(asset.portals.len());
    for portal in &asset.portals {
        let mut maximum = 0u16;
        for row in metadata(&crossings, i64::from(portal.area))? {
            for time in row {
                maximum = maximum.max(*time);
            }
        }
        portal_maxima.push(maximum);
    }
    Ok(Topology {
        incoming,
        crossings,
        portal_maxima,
    })
}

/// AAS routing estimates. The donor holds the graph and asset by
/// reference and memoizes topologies per asset in a module `WeakMap`;
/// this port clones the metadata it relaxes and computes one topology
/// per estimator, which is behaviorally identical because topology is
/// a pure function of the asset. Static eligibility arrives per query
/// instead of through a stored closure so runtimes own their
/// estimators.
pub struct AasNavigationEstimates {
    asset: AasAsset,
    edges: HashMap<i32, NavigationEdge>,
    topology: Topology,
    caches: HashMap<String, Cache>,
    cache_order: VecDeque<String>,
    cache_bytes: usize,
}

impl AasNavigationEstimates {
    /// Build estimators over an AAS graph.
    pub fn new(graph: &NavigationGraph, asset: &AasAsset) -> Result<Self, BotsError> {
        let mut edges = HashMap::new();
        for edge in &graph.edges {
            edges.insert(edge.id, edge.clone());
        }
        Ok(Self {
            asset: asset.clone(),
            edges,
            topology: build_topology(asset)?,
            caches: HashMap::new(),
            cache_order: VecDeque::new(),
            cache_bytes: 0,
        })
    }

    /// Drop cached routing tables.
    pub fn invalidate(&mut self) {
        self.caches.clear();
        self.cache_order.clear();
        self.cache_bytes = 0;
    }

    fn remember(&mut self, key: String, cache: Cache) -> Cache {
        self.cache_bytes += cache.times.len() * 2 + cache.reaches.len();
        self.cache_order.push_back(key.clone());
        self.caches.insert(key, cache.clone());
        while self.cache_bytes > CACHE_BUDGET && self.caches.len() > 1 {
            let Some(oldest) = self.cache_order.pop_front() else {
                break;
            };
            if let Some(removed) = self.caches.remove(&oldest) {
                self.cache_bytes -= removed.times.len() * 2 + removed.reaches.len();
            }
        }
        cache
    }

    fn cached(&mut self, key: &str) -> Option<Cache> {
        let cache = self.caches.get(key)?.clone();
        self.cache_order.retain(|entry| entry != key);
        self.cache_order.push_back(key.to_string());
        Some(cache)
    }

    fn cluster_area(&self, cluster: i32, area: i32) -> Result<i32, BotsError> {
        let setting = metadata(&self.asset.settings, i64::from(area))?;
        if setting.cluster > 0 {
            return Ok(setting.cluster_area);
        }
        let portal = metadata(&self.asset.portals, -i64::from(setting.cluster))?;
        Ok(portal.cluster_areas[usize::from(portal.front_cluster != cluster)])
    }

    fn area_cache(
        &mut self,
        cluster: i32,
        goal: i32,
        flags: i32,
        allowed: &Eligibility<'_>,
    ) -> Result<Cache, BotsError> {
        let key = format!("a:{cluster}:{goal}:{flags}");
        if let Some(cached) = self.cached(&key) {
            return Ok(cached);
        }
        let count = metadata(&self.asset.clusters, i64::from(cluster))?.reachability_area_count;
        let cache = Cache {
            times: vec![0; count.max(0) as usize],
            reaches: vec![0; count.max(0) as usize],
        };
        let goal_index = self.cluster_area(cluster, goal)?;
        if goal_index >= count {
            return Ok(self.remember(key, cache));
        }
        let mut cache = cache;
        let mut updates: Vec<Update> = (0..count.max(0))
            .map(|_| Update {
                area: 0,
                cluster: 0,
                time: 0,
                row: Vec::new(),
                queued: false,
            })
            .collect();
        let incoming_len = metadata(&self.topology.incoming, i64::from(goal))?.len();
        let first = crate::error::indexed_mut(&mut updates, i64::from(goal_index), "Missing AAS estimate metadata")?;
        first.area = goal;
        first.time = 1;
        first.row = vec![0; incoming_len];
        *crate::error::indexed_mut(&mut cache.times, i64::from(goal_index), "Missing AAS estimate metadata")? = 1;
        let mut queue = vec![goal_index as usize];
        let mut cursor = 0;
        while cursor < queue.len() {
            let current_index = queue[cursor];
            cursor += 1;
            updates[current_index].queued = false;
            let (area, time, row) = {
                let current = &updates[current_index];
                (current.area, current.time, current.row.clone())
            };
            for (ordinal, link) in metadata(&self.topology.incoming, i64::from(area))?.iter().enumerate() {
                let reach = *metadata(&self.asset.reachability, i64::from(link.reach))?;
                let edge = match self.edges.get(&link.reach) {
                    Some(edge) => edge,
                    None => continue,
                };
                if (aas_travel_flag(reach.travel_type) & !flags) != 0
                    || !allowed(edge, Some(flags))
                    || (aas_area_travel_flags(metadata(&self.asset.settings, i64::from(reach.area))?) & !flags) != 0
                {
                    continue;
                }
                let setting = *metadata(&self.asset.settings, i64::from(link.area))?;
                if setting.cluster > 0 && setting.cluster != cluster {
                    continue;
                }
                let index = self.cluster_area(cluster, link.area)?;
                if index >= count {
                    continue;
                }
                let time =
                    ((u32::from(time) + u32::from(*metadata(&row, ordinal as i64)?) + u32::from(reach.travel_time))
                        & 0xffff) as u16;
                let previous = metadata(&cache.times, i64::from(index))?;
                if *previous != 0 && u32::from(*previous) <= u32::from(time) {
                    continue;
                }
                cache.times[index as usize] = time;
                let offset = link.reach - setting.first_reach;
                cache.reaches[index as usize] = offset as u8;
                let next = &mut updates[index as usize];
                next.area = link.area;
                next.time = time;
                next.row = metadata(
                    metadata(&self.topology.crossings, i64::from(link.area))?,
                    i64::from(offset),
                )?
                .clone();
                if !next.queued {
                    next.queued = true;
                    queue.push(index as usize);
                }
            }
        }
        Ok(self.remember(key, cache))
    }

    fn portal_cache(
        &mut self,
        cluster: i32,
        goal: i32,
        flags: i32,
        allowed: &Eligibility<'_>,
    ) -> Result<Cache, BotsError> {
        let key = format!("p:{goal}:{flags}");
        if let Some(cached) = self.cached(&key) {
            return Ok(cached);
        }
        let count = self.asset.portals.len();
        let mut cache = Cache {
            times: vec![0; count],
            reaches: vec![0; count],
        };
        let mut updates: Vec<Update> = (0..=count)
            .map(|_| Update {
                area: 0,
                cluster: 0,
                time: 0,
                row: Vec::new(),
                queued: false,
            })
            .collect();
        updates[count].cluster = cluster;
        updates[count].area = goal;
        updates[count].time = 1;
        let goal_cluster = metadata(&self.asset.settings, i64::from(goal))?.cluster;
        if goal_cluster < 0 {
            let slot = (-i64::from(goal_cluster)) as usize;
            if slot < cache.times.len() {
                cache.times[slot] = 1;
            }
        }
        let mut queue = vec![count];
        let mut cursor = 0;
        while cursor < queue.len() {
            let current_index = queue[cursor];
            cursor += 1;
            updates[current_index].queued = false;
            let (area, cluster, time) = {
                let current = &updates[current_index];
                (current.area, current.cluster, current.time)
            };
            let record = *metadata(&self.asset.clusters, i64::from(cluster))?;
            let local = self.area_cache(cluster, area, flags, allowed)?;
            for offset in 0..record.portal_count {
                let number = *metadata(&self.asset.portal_indexes, i64::from(record.first_portal + offset))?;
                let portal = *metadata(&self.asset.portals, i64::from(number))?;
                if portal.area == area {
                    continue;
                }
                let index = self.cluster_area(cluster, portal.area)?;
                if index >= record.reachability_area_count {
                    continue;
                }
                let local_time = *metadata(&local.times, i64::from(index))?;
                if local_time == 0 {
                    continue;
                }
                let time = ((u32::from(local_time) + u32::from(time)) & 0xffff) as u16;
                let previous = *metadata(&cache.times, i64::from(number))?;
                if previous != 0 && u32::from(previous) <= u32::from(time) {
                    continue;
                }
                cache.times[number as usize] = time;
                let next = &mut updates[number as usize];
                next.cluster = if portal.front_cluster == cluster {
                    portal.back_cluster
                } else {
                    portal.front_cluster
                };
                next.area = portal.area;
                next.time = ((u32::from(time) + u32::from(*metadata(&self.topology.portal_maxima, i64::from(number))?))
                    & 0xffff) as u16;
                if !next.queued {
                    next.queued = true;
                    queue.push(number as usize);
                }
            }
        }
        Ok(self.remember(key, cache))
    }

    /// Estimate an AAS cost query.
    pub fn estimate(
        &mut self,
        query: &NavigationEstimateQuery,
        allowed: &Eligibility<'_>,
    ) -> Result<NavigationEstimateResult, BotsError> {
        let (area, goal) = (query.start_node, query.goal_node);
        if area <= 0 || area >= self.asset.areas.len() as i32 || goal <= 0 || goal >= self.asset.areas.len() as i32 {
            return Ok(NavigationEstimateResult::Unreachable);
        }
        if area == goal {
            return Ok(NavigationEstimateResult::Estimate {
                travel_time: 1,
                first_edge: None,
            });
        }
        let start = *metadata(&self.asset.settings, i64::from(area))?;
        let end = *metadata(&self.asset.settings, i64::from(goal))?;
        let mut flags = query.travel_flags.unwrap_or(DEFAULT_TRAVEL_FLAGS);
        if ((start.contents | end.contents) & 256) != 0 {
            flags |= 0x0080_0000;
        }
        let mut cluster = start.cluster;
        let mut goal_cluster = end.cluster;
        if cluster < 0 && goal_cluster > 0 {
            let portal = *metadata(&self.asset.portals, -i64::from(cluster))?;
            if portal.front_cluster == goal_cluster || portal.back_cluster == goal_cluster {
                cluster = goal_cluster;
            }
        } else if cluster > 0 && goal_cluster < 0 {
            let portal = *metadata(&self.asset.portals, -i64::from(goal_cluster))?;
            if portal.front_cluster == cluster || portal.back_cluster == cluster {
                goal_cluster = cluster;
            }
        }
        if cluster > 0 && cluster == goal_cluster {
            let cache = self.area_cache(cluster, goal, flags, allowed)?;
            let index = self.cluster_area(cluster, area)?;
            if index >= metadata(&self.asset.clusters, i64::from(cluster))?.reachability_area_count {
                return Ok(NavigationEstimateResult::Unreachable);
            }
            let time = *metadata(&cache.times, i64::from(index))?;
            if time != 0 {
                let Some(origin) = query.origin else {
                    return Ok(NavigationEstimateResult::Estimate {
                        travel_time: i32::from(time),
                        first_edge: None,
                    });
                };
                let reach = start.first_reach + i32::from(*metadata(&cache.reaches, i64::from(index))?);
                return Ok(NavigationEstimateResult::Estimate {
                    travel_time: i32::from(time)
                        + i32::from(aas_estimate_area_time(
                            &start,
                            origin,
                            metadata(&self.asset.reachability, i64::from(reach))?.start,
                        )?),
                    first_edge: self.edges.get(&reach).cloned(),
                });
            }
        }
        let cluster = start.cluster;
        let mut goal_cluster = end.cluster;
        if goal_cluster < 0 {
            goal_cluster = metadata(&self.asset.portals, -i64::from(goal_cluster))?.front_cluster;
        }
        let portal_cache = self.portal_cache(goal_cluster, goal, flags, allowed)?;
        if cluster < 0 {
            let slot = -i64::from(cluster);
            return Ok(NavigationEstimateResult::Estimate {
                travel_time: i32::from(*metadata(&portal_cache.times, slot)?),
                first_edge: match query.origin {
                    None => None,
                    Some(_) => self
                        .edges
                        .get(&(start.first_reach + i32::from(*metadata(&portal_cache.reaches, slot)?)))
                        .cloned(),
                },
            });
        }
        let mut best = NavigationEstimateResult::Unreachable;
        let mut best_time = 0u32;
        let record = *metadata(&self.asset.clusters, i64::from(cluster))?;
        for offset in 0..record.portal_count {
            let number = *metadata(&self.asset.portal_indexes, i64::from(record.first_portal + offset))?;
            let portal_time = *metadata(&portal_cache.times, i64::from(number))?;
            if portal_time == 0 {
                continue;
            }
            let portal = *metadata(&self.asset.portals, i64::from(number))?;
            let local = self.area_cache(cluster, portal.area, flags, allowed)?;
            let index = self.cluster_area(cluster, area)?;
            if index >= record.reachability_area_count {
                continue;
            }
            let local_time = *metadata(&local.times, i64::from(index))?;
            if local_time == 0 {
                continue;
            }
            let reach = match query.origin {
                None => None,
                Some(_) => Some(start.first_reach + i32::from(*metadata(&local.reaches, i64::from(index))?)),
            };
            let mut time = (u32::from(portal_time) + u32::from(local_time)) & 0xffff;
            time = (time + u32::from(*metadata(&self.topology.portal_maxima, i64::from(number))?)) & 0xffff;
            if let (Some(origin), Some(reach)) = (query.origin, reach) {
                time = (time
                    + u32::from(aas_estimate_area_time(
                        &start,
                        origin,
                        metadata(&self.asset.reachability, i64::from(reach))?.start,
                    )?))
                    & 0xffff;
            }
            if best_time == 0 || time < best_time {
                best_time = time;
                best = NavigationEstimateResult::Estimate {
                    travel_time: time as i32,
                    first_edge: reach.and_then(|reach| self.edges.get(&reach).cloned()),
                };
            }
        }
        Ok(best)
    }
}
