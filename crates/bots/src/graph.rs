//! Travel vocabulary from id Software `be_aas.h`, Q1 NAV2, and q2repro
//! `inc/server/nav.h`, ported from `src/bots/navigation/graph.ts`.
//! Copyright (C) 1999-2005 Id Software, Inc.

use qa_core::math::Vec3;

use crate::aas::AasAreaSettings;
use crate::error::BotsError;
use crate::helpers::{at, clear, contents, crouched_profile, distance, trace, translated, validate_profile};
use crate::scene::QueryTarget;
use crate::types::{
    KexGeneration, NavigationAsset, NavigationEdge, NavigationEntityBinding, NavigationGraph, NavigationMapIdentity,
    NavigationNode, NavigationProfile, NavigationSource, NavigationWorld, RejectedSource, TravelMode,
};

const AAS_MODES: [TravelMode; 20] = [
    TravelMode::Unknown,
    TravelMode::Unknown,
    TravelMode::Walk,
    TravelMode::Crouch,
    TravelMode::Jump,
    TravelMode::Jump,
    TravelMode::Ladder,
    TravelMode::Drop,
    TravelMode::Swim,
    TravelMode::WaterJump,
    TravelMode::Teleport,
    TravelMode::Mover,
    TravelMode::RocketJump,
    TravelMode::BfgJump,
    TravelMode::Grapple,
    TravelMode::DoubleJump,
    TravelMode::RampJump,
    TravelMode::StrafeJump,
    TravelMode::JumpPad,
    TravelMode::Mover,
];

const KEX_MODES: [TravelMode; 15] = [
    TravelMode::Walk,
    TravelMode::Jump,
    TravelMode::Teleport,
    TravelMode::Drop,
    TravelMode::JumpPad,
    TravelMode::Jump,
    TravelMode::Mover,
    TravelMode::Mover,
    TravelMode::Jump,
    TravelMode::Crouch,
    TravelMode::Ladder,
    TravelMode::Jump,
    TravelMode::Jump,
    TravelMode::RocketJump,
    TravelMode::Unknown,
];

/// Travel mode for a source AAS travel type.
#[must_use]
pub fn aas_travel_mode(travel_type: i32) -> TravelMode {
    let index = (travel_type as u32 & 0x00ff_ffff) as usize;
    AAS_MODES.get(index).copied().unwrap_or(TravelMode::Unknown)
}

/// Travel mode for a source Kex link type.
#[must_use]
pub fn kex_travel_mode(link_type: i32) -> TravelMode {
    if link_type < 0 {
        return TravelMode::Unknown;
    }
    KEX_MODES
        .get(link_type as usize)
        .copied()
        .unwrap_or(TravelMode::Unknown)
}

/// Source travel flags preserve the unused bit between LADDER and
/// WALKOFFLEDGE.
#[must_use]
pub fn aas_travel_flag(travel_type: i32) -> i32 {
    let index = (travel_type as u32 & 0x00ff_ffff) as i32;
    if !(2..=19).contains(&index) {
        1
    } else if index == 19 {
        0x0100_0000
    } else if index >= 7 {
        1 << index
    } else {
        1 << (index - 1)
    }
}

/// Travel flags implied by AAS area contents and flags.
#[must_use]
pub fn aas_area_travel_flags(setting: &AasAreaSettings) -> i32 {
    let value = setting.contents;
    (if (value & 1) != 0 {
        0x0010_0000
    } else if (value & 4) != 0 {
        0x0020_0000
    } else if (value & 2) != 0 {
        0x0040_0000
    } else {
        0x0008_0000
    }) | (if (value & 256) != 0 { 0x0080_0000 } else { 0 })
        | (if (value & 2048) != 0 { 0x0800_0000 } else { 0 })
        | (if (value & 4096) != 0 { 0x1000_0000 } else { 0 })
        | (if (setting.flags & 16) != 0 { 0x0400_0000 } else { 0 })
}

/// Directed components support drops and teleports without falsely
/// declaring reverse reachability.
pub fn navigation_clusters(nodes: &[NavigationNode], edges: &[NavigationEdge]) -> Vec<Vec<i32>> {
    use std::collections::{HashMap, HashSet};
    let mut outgoing: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut incoming: HashMap<i32, Vec<i32>> = HashMap::new();
    for node in nodes {
        outgoing.insert(node.id, Vec::new());
        incoming.insert(node.id, Vec::new());
    }
    for edge in edges {
        if let Some(list) = outgoing.get_mut(&edge.from) {
            list.push(edge.to);
        }
        if let Some(list) = incoming.get_mut(&edge.to) {
            list.push(edge.from);
        }
    }
    let mut seen = HashSet::new();
    let mut order = Vec::new();
    for node in nodes {
        let mut stack = vec![(node.id, false)];
        while let Some((id, exit)) = stack.pop() {
            if exit {
                order.push(id);
                continue;
            }
            if seen.contains(&id) {
                continue;
            }
            seen.insert(id);
            stack.push((id, true));
            if let Some(next) = outgoing.get(&id) {
                for target in next {
                    if !seen.contains(target) {
                        stack.push((*target, false));
                    }
                }
            }
        }
    }
    seen.clear();
    let mut clusters = Vec::new();
    order.reverse();
    for start in order {
        if seen.contains(&start) {
            continue;
        }
        let mut cluster = Vec::new();
        let mut stack = vec![start];
        while let Some(id) = stack.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.insert(id);
            cluster.push(id);
            if let Some(next) = incoming.get(&id) {
                for target in next {
                    if !seen.contains(target) {
                        stack.push(*target);
                    }
                }
            }
        }
        clusters.push(cluster);
    }
    clusters
}

/// Build a navigation graph from a parsed asset.
pub fn navigation_from_asset(
    map: NavigationMapIdentity,
    asset: NavigationAsset,
    profile: NavigationProfile,
    world: &dyn NavigationWorld,
) -> Result<NavigationGraph, BotsError> {
    validate_profile(&profile)?;
    let mut nodes: Vec<NavigationNode> = Vec::new();
    let mut edges: Vec<NavigationEdge> = Vec::new();
    let target = QueryTarget::World;
    match &asset {
        NavigationAsset::Aas(asset) => {
            for number in 1..asset.areas.len() as i32 {
                let area = *at(&asset.areas, number)?;
                let setting = *at(&asset.settings, number)?;
                let posture = if (setting.presence & 2) == 0 && (setting.presence & 4) != 0 {
                    crouched_profile(&profile).unwrap_or_else(|| profile.clone())
                } else {
                    profile.clone()
                };
                let mut origin = area.center;
                if (setting.flags & 1) != 0 {
                    let floor = trace(
                        world,
                        &posture,
                        area.center,
                        Vec3 {
                            x: area.center.x,
                            y: area.center.y,
                            z: (f64::from(area.bounds.min.z) + f64::from(posture.shape.bounds().min.z)
                                - profile.maximum_step) as f32,
                        },
                        &target,
                    );
                    let grounded = matches!(
                        floor.contact,
                        crate::scene::TraceContact::Plane { plane }
                            if plane.normal.z as f64 >= profile.minimum_floor_normal
                    );
                    if !floor.start_solid && !floor.all_solid && floor.fraction < 1.0 && grounded {
                        origin = floor.end;
                    } else {
                        for index in setting.first_reach..setting.first_reach + setting.reach_count {
                            let reach = at(&asset.reachability, index)?;
                            if clear(world, &posture, reach.start, reach.start, &target) {
                                origin = reach.start;
                                break;
                            }
                        }
                    }
                }
                nodes.push(NavigationNode {
                    id: number,
                    origin,
                    bounds: area.bounds,
                    radius: 0.0,
                    contents: contents(world, &profile, origin, &target),
                    flags: setting.flags,
                    presence: setting.presence,
                    source_cluster: Some(setting.cluster),
                    source: NavigationSource::Aas {
                        area: number,
                        reachability: None,
                    },
                });
                for index in setting.first_reach..setting.first_reach + setting.reach_count {
                    let reach = *at(&asset.reachability, index)?;
                    let mode = aas_travel_mode(reach.travel_type);
                    if reach.area == 0 {
                        continue;
                    }
                    let moving =
                        mode == TravelMode::Mover || mode == TravelMode::Teleport || mode == TravelMode::JumpPad;
                    let model = if mode == TravelMode::Mover {
                        Some(reach.face & 0xffff)
                    } else {
                        None
                    };
                    edges.push(NavigationEdge {
                        id: index,
                        from: number,
                        to: reach.area,
                        mode,
                        start: reach.start,
                        end: reach.end,
                        travel_seconds: f64::from(reach.travel_time.max(1)) / 100.0,
                        source_travel_type: reach.travel_type,
                        source_flags: 0,
                        hint: None,
                        entity: if moving {
                            Some(NavigationEntityBinding {
                                model,
                                bounds: area.bounds,
                                raw: vec![reach.face, reach.edge],
                            })
                        } else {
                            None
                        },
                        source: NavigationSource::Aas {
                            area: number,
                            reachability: Some(index),
                        },
                    });
                }
            }
        }
        NavigationAsset::Kex(asset) => {
            let rise = -profile.shape.bounds().min.z;
            let lift = |point: Vec3| Vec3 {
                x: point.x,
                y: point.y,
                z: point.z + rise,
            };
            for (number, node) in asset.nodes.iter().enumerate() {
                let number = number as i32;
                let origin = lift(node.origin);
                nodes.push(NavigationNode {
                    id: number,
                    origin,
                    bounds: translated(origin, profile.shape.bounds()),
                    radius: f64::from(node.radius),
                    flags: node.flags,
                    contents: contents(world, &profile, origin, &target),
                    presence: 0,
                    source_cluster: None,
                    source: NavigationSource::Kex {
                        generation: asset.kind,
                        node: number,
                        link: None,
                    },
                });
            }
            let entity_links: std::collections::HashMap<i32, &crate::nav::KexEntity> =
                asset.entities.iter().map(|entity| (entity.link, entity)).collect();
            for (number, node) in asset.nodes.iter().enumerate() {
                let number = number as i32;
                for index in node.first_link..node.first_link + node.link_count {
                    let link = *at(&asset.links, index)?;
                    let entity = entity_links.get(&index);
                    let hint = match link.traversal {
                        None => None,
                        Some(traversal) => {
                            let raw = at(&asset.traversals, traversal)?;
                            Some(crate::types::TraversalHint {
                                funnel: lift(raw.funnel),
                                start: lift(raw.start),
                                end: lift(raw.end),
                                ladder_plane: raw.ladder_plane,
                            })
                        }
                    };
                    let start = hint.map_or(at(&nodes, number)?.origin, |hint| hint.start);
                    let end = hint.map_or(at(&nodes, link.target)?.origin, |hint| hint.end);
                    let mode = kex_travel_mode(link.link_type);
                    edges.push(NavigationEdge {
                        id: index,
                        from: number,
                        to: link.target,
                        mode,
                        start,
                        end,
                        source_travel_type: link.link_type,
                        source_flags: link.flags,
                        travel_seconds: if mode == TravelMode::Teleport {
                            0.01
                        } else {
                            (distance(start, end) * asset.heuristic / 320.0).max(0.01)
                        },
                        hint,
                        entity: match entity {
                            None if mode == TravelMode::Teleport
                                || mode == TravelMode::JumpPad
                                || mode == TravelMode::Mover =>
                            {
                                Some(NavigationEntityBinding {
                                    model: None,
                                    bounds: at(&nodes, number)?.bounds,
                                    raw: Vec::new(),
                                })
                            }
                            None => None,
                            Some(entity) => Some(NavigationEntityBinding {
                                model: match (asset.kind, entity.model) {
                                    (KexGeneration::Nav3, Some(model)) if model <= 1 || model == 255 => None,
                                    (KexGeneration::Nav3, Some(model)) => Some(model - i32::from(model > 255) - 1),
                                    (_, model) => model,
                                },
                                bounds: entity.bounds,
                                raw: entity.tail.clone(),
                            }),
                        },
                        source: NavigationSource::Kex {
                            generation: asset.kind,
                            node: number,
                            link: Some(index),
                        },
                    });
                }
            }
        }
    }
    let rejected = edges
        .iter()
        .filter(|edge| edge.mode == TravelMode::Unknown)
        .map(|edge| RejectedSource {
            source: edge.source,
            reason: format!("Unsupported source travel type {}", edge.source_travel_type),
        })
        .collect();
    let clusters = navigation_clusters(&nodes, &edges);
    Ok(NavigationGraph {
        map,
        profile,
        asset: Some(asset),
        nodes,
        edges,
        clusters,
        rejected,
    })
}

/// Travel-flag mask for an edge.
#[must_use]
pub fn navigation_edge_travel_flag(edge: &NavigationEdge) -> i32 {
    if let NavigationSource::Aas { .. } = edge.source {
        return aas_travel_flag(edge.source_travel_type);
    }
    match edge.mode {
        TravelMode::Walk => 2,
        TravelMode::Crouch => 4,
        TravelMode::Jump => 16,
        TravelMode::Drop => 128,
        TravelMode::Swim => 256,
        TravelMode::WaterJump => 512,
        TravelMode::Ladder => 32,
        TravelMode::Teleport => 1024,
        TravelMode::Mover => 16_779_264,
        TravelMode::JumpPad => 262_144,
        TravelMode::RocketJump => 4096,
        TravelMode::BfgJump => 8192,
        TravelMode::Grapple => 16384,
        TravelMode::DoubleJump => 32768,
        TravelMode::RampJump => 65536,
        TravelMode::StrafeJump => 131_072,
        TravelMode::Unknown => 1,
    }
}
