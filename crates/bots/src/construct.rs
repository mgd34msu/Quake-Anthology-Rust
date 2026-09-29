//! Navigation construction from decoded geometry, ported from
//! `src/bots/navigation/construct.ts`. Ground-face sampling follows AAS
//! reachability's floor-normal and step clearance tests. All occupancy
//! and support decisions use shared collision; movement admission
//! happens in routing.

use std::collections::{HashMap, HashSet};

use qa_core::math::{Bounds, Vec3};

use crate::error::BotsError;
use crate::graph::navigation_clusters;
use crate::helpers::{at, clear, contents, crouched_profile, distance, midpoint, trace, translated, validate_profile};
use crate::scene::{DecodedWorld, Q3SurfaceKind, QueryTarget};
use crate::types::{
    NavigationEdge, NavigationGraph, NavigationMapIdentity, NavigationNode, NavigationProfile, NavigationSource,
    NavigationWorld, RejectedSource, TravelMode, TraversalHint,
};

/// Source connection between constructed samples. Endpoints come from
/// spawned map entities and their actual movement/trigger authority.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationConnection {
    /// Traversal start.
    pub from: Vec3,
    /// Traversal end.
    pub to: Vec3,
    /// Travel mode.
    pub mode: TravelMode,
    /// Traversal hint.
    pub hint: Option<TraversalHint>,
    /// Entity binding.
    pub entity: Option<crate::types::NavigationEntityBinding>,
    /// Source entity ordinal or host-owned connection identifier,
    /// retained in diagnostics.
    pub id: i32,
    /// Source travel type.
    pub source_travel_type: i32,
    /// Travel seconds.
    pub travel_seconds: f64,
}

/// Navigation construction options.
pub struct NavigationConstruction<'a> {
    /// Decoded map geometry.
    pub geometry: &'a DecodedWorld,
    /// Map identity.
    pub map: &'a NavigationMapIdentity,
    /// Traversal profile.
    pub profile: &'a NavigationProfile,
    /// Shared world.
    pub world: &'a dyn NavigationWorld,
    /// Sample spacing in units.
    pub spacing: Option<f64>,
    /// Link distance in units.
    pub link_distance: Option<f64>,
    /// Node cap.
    pub maximum_nodes: Option<usize>,
    /// Source connections.
    pub connections: Option<&'a [NavigationConnection]>,
}

fn map_target() -> QueryTarget {
    QueryTarget::Model {
        model: 0,
        origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    }
}

fn triangle_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ax, ay, az) = (f64::from(a.x), f64::from(a.y), f64::from(a.z));
    let (bx, by, bz) = (f64::from(b.x), f64::from(b.y), f64::from(b.z));
    let (cx, cy, cz) = (f64::from(c.x), f64::from(c.y), f64::from(c.z));
    let x = (by - ay) * (cz - az) - (bz - az) * (cy - ay);
    let y = (bz - az) * (cx - ax) - (bx - ax) * (cz - az);
    let z = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
    let length = x.hypot(y).hypot(z);
    if length == 0.0 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    } else {
        Vec3 {
            x: (x / length) as f32,
            y: (y / length) as f32,
            z: (z / length) as f32,
        }
    }
}

fn inside(point: Vec3, polygon: &[Vec3]) -> bool {
    let (px, py) = (f64::from(point.x), f64::from(point.y));
    let mut sign = 0;
    for (i, a) in polygon.iter().enumerate() {
        let b = &polygon[(i + 1) % polygon.len()];
        let cross = (f64::from(b.x) - f64::from(a.x)) * (py - f64::from(a.y))
            - (f64::from(b.y) - f64::from(a.y)) * (px - f64::from(a.x));
        if cross.abs() < 0.001 {
            continue;
        }
        let current = if cross > 0.0 { 1 } else { -1 };
        if sign != 0 && current != sign {
            return false;
        }
        sign = current;
    }
    true
}

fn polygon_candidates(
    polygon: &[Vec3],
    normal: Vec3,
    spacing: f64,
    source: NavigationSource,
    publish: &mut impl FnMut(Vec3, NavigationSource),
) {
    if polygon.len() < 3 || normal.z <= 0.0 {
        return;
    }
    let count = polygon.len() as f64;
    let mut center = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    for point in polygon {
        center.x += (f64::from(point.x) / count) as f32;
        center.y += (f64::from(point.y) / count) as f32;
        center.z += (f64::from(point.z) / count) as f32;
    }
    publish(center, source);
    for vertex in polygon {
        publish(midpoint(center, *vertex), source);
    }
    let first = polygon[0];
    let plane = f64::from(first.x) * f64::from(normal.x)
        + f64::from(first.y) * f64::from(normal.y)
        + f64::from(first.z) * f64::from(normal.z);
    let min_x = polygon
        .iter()
        .map(|point| f64::from(point.x))
        .fold(f64::INFINITY, f64::min);
    let max_x = polygon
        .iter()
        .map(|point| f64::from(point.x))
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = polygon
        .iter()
        .map(|point| f64::from(point.y))
        .fold(f64::INFINITY, f64::min);
    let max_y = polygon
        .iter()
        .map(|point| f64::from(point.y))
        .fold(f64::NEG_INFINITY, f64::max);
    let mut x = (min_x / spacing).ceil() * spacing;
    while x <= max_x {
        let mut y = (min_y / spacing).ceil() * spacing;
        while y <= max_y {
            let point = Vec3 {
                x: x as f32,
                y: y as f32,
                z: ((plane - x * f64::from(normal.x) - y * f64::from(normal.y)) / f64::from(normal.z)) as f32,
            };
            if inside(point, polygon) {
                publish(point, source);
            }
            y += spacing;
        }
        x += spacing;
    }
}

fn geometry_candidates(
    world: &DecodedWorld,
    spacing: f64,
    slope: f64,
    publish: &mut impl FnMut(Vec3, NavigationSource),
) -> Result<(), BotsError> {
    match world {
        DecodedWorld::Q1(world) => {
            let model = world.models.first().ok_or(BotsError::MissingWorldModel)?;
            for index in model.faces.first..model.faces.first + model.faces.count {
                let face = crate::error::indexed(&world.faces, index as i64, "Navigation record")?;
                let plane = crate::error::indexed(&world.planes, face.plane as i64, "Navigation record")?;
                let sign = if face.back { -1.0 } else { 1.0 };
                let normal = Vec3 {
                    x: (f64::from(plane.normal.x) * sign) as f32,
                    y: (f64::from(plane.normal.y) * sign) as f32,
                    z: (f64::from(plane.normal.z) * sign) as f32,
                };
                if f64::from(normal.z) < slope {
                    continue;
                }
                let mut polygon = Vec::new();
                for edge_index in face.edges.first..face.edges.first + face.edges.count {
                    let signed = crate::error::indexed(&world.surface_edges, edge_index as i64, "Navigation record")?;
                    let edge = crate::error::indexed(&world.edges, signed.unsigned_abs() as i64, "Navigation record")?;
                    polygon.push(*crate::error::indexed(
                        &world.vertices,
                        i64::from(edge.vertices[usize::from(*signed < 0)]),
                        "Navigation record",
                    )?);
                }
                polygon_candidates(
                    &polygon,
                    normal,
                    spacing,
                    NavigationSource::Constructed {
                        surface: Some(index as i32),
                        leaf: None,
                    },
                    publish,
                );
            }
        }
        DecodedWorld::Q2(world) => {
            let model = world.models.first().ok_or(BotsError::MissingWorldModel)?;
            for index in model.faces.first..model.faces.first + model.faces.count {
                let face = crate::error::indexed(&world.faces, index as i64, "Navigation record")?;
                let plane = crate::error::indexed(&world.planes, face.plane as i64, "Navigation record")?;
                let sign = if face.back { -1.0 } else { 1.0 };
                let normal = Vec3 {
                    x: (f64::from(plane.normal.x) * sign) as f32,
                    y: (f64::from(plane.normal.y) * sign) as f32,
                    z: (f64::from(plane.normal.z) * sign) as f32,
                };
                if f64::from(normal.z) < slope {
                    continue;
                }
                let mut polygon = Vec::new();
                for edge_index in face.edges.first..face.edges.first + face.edges.count {
                    let signed = crate::error::indexed(&world.surface_edges, edge_index as i64, "Navigation record")?;
                    let edge = crate::error::indexed(&world.edges, signed.unsigned_abs() as i64, "Navigation record")?;
                    polygon.push(*crate::error::indexed(
                        &world.vertices,
                        i64::from(edge.vertices[usize::from(*signed < 0)]),
                        "Navigation record",
                    )?);
                }
                polygon_candidates(
                    &polygon,
                    normal,
                    spacing,
                    NavigationSource::Constructed {
                        surface: Some(index as i32),
                        leaf: None,
                    },
                    publish,
                );
            }
        }
        DecodedWorld::Q3(world) => {
            let model = world.models.first().ok_or(BotsError::MissingWorldModel)?;
            for index in model.surfaces.first..model.surfaces.first + model.surfaces.count {
                let surface = crate::error::indexed(&world.surfaces, index as i64, "Navigation record")?;
                let source = NavigationSource::Constructed {
                    surface: Some(index as i32),
                    leaf: None,
                };
                if surface.kind == Q3SurfaceKind::Flare {
                    continue;
                }
                if surface.kind == Q3SurfaceKind::Patch {
                    for row in 0..surface.height.saturating_sub(1) {
                        for column in 0..surface.width.saturating_sub(1) {
                            let first = surface.vertices.first + row * surface.width + column;
                            let polygon = [
                                crate::error::indexed(&world.vertices, first as i64, "Navigation record")?.position,
                                crate::error::indexed(&world.vertices, (first + 1) as i64, "Navigation record")?
                                    .position,
                                crate::error::indexed(
                                    &world.vertices,
                                    (first + surface.width + 1) as i64,
                                    "Navigation record",
                                )?
                                .position,
                                crate::error::indexed(
                                    &world.vertices,
                                    (first + surface.width) as i64,
                                    "Navigation record",
                                )?
                                .position,
                            ];
                            let mut normal = triangle_normal(polygon[0], polygon[1], polygon[2]);
                            let authored =
                                crate::error::indexed(&world.vertices, first as i64, "Navigation record")?.normal;
                            if f64::from(normal.x) * f64::from(authored.x)
                                + f64::from(normal.y) * f64::from(authored.y)
                                + f64::from(normal.z) * f64::from(authored.z)
                                < 0.0
                            {
                                normal = Vec3 {
                                    x: -normal.x,
                                    y: -normal.y,
                                    z: -normal.z,
                                };
                            }
                            if f64::from(normal.z) >= slope {
                                polygon_candidates(&polygon, normal, spacing, source, publish);
                            }
                        }
                    }
                } else {
                    let mut offset = surface.indices.first;
                    while offset + 2 < surface.indices.first + surface.indices.count {
                        let corner = |step: usize| -> Result<Vec3, BotsError> {
                            let index =
                                *crate::error::indexed(&world.indices, (offset + step) as i64, "Navigation record")?;
                            Ok(crate::error::indexed(
                                &world.vertices,
                                surface.vertices.first as i64 + i64::from(index),
                                "Navigation record",
                            )?
                            .position)
                        };
                        let polygon = [corner(0)?, corner(1)?, corner(2)?];
                        let mut normal = triangle_normal(polygon[0], polygon[1], polygon[2]);
                        let first = *crate::error::indexed(&world.indices, offset as i64, "Navigation record")?;
                        let authored = crate::error::indexed(
                            &world.vertices,
                            surface.vertices.first as i64 + i64::from(first),
                            "Navigation record",
                        )?
                        .normal;
                        if f64::from(normal.x) * f64::from(authored.x)
                            + f64::from(normal.y) * f64::from(authored.y)
                            + f64::from(normal.z) * f64::from(authored.z)
                            < 0.0
                        {
                            normal = Vec3 {
                                x: -normal.x,
                                y: -normal.y,
                                z: -normal.z,
                            };
                        }
                        if f64::from(normal.z) >= slope {
                            polygon_candidates(&polygon, normal, spacing, source, publish);
                        }
                        offset += 3;
                    }
                }
            }
        }
    }
    Ok(())
}

fn grounded(world: &dyn NavigationWorld, profile: &NavigationProfile, point: Vec3) -> Option<Vec3> {
    let height = f64::from(-profile.shape.bounds().min.z);
    let target = map_target();
    let end = Vec3 {
        x: point.x,
        y: point.y,
        z: (f64::from(point.z) + height - profile.maximum_step - 4.0) as f32,
    };
    let mut result = trace(
        world,
        profile,
        Vec3 {
            x: point.x,
            y: point.y,
            z: (f64::from(point.z) + height + profile.maximum_step + 2.0) as f32,
        },
        end,
        &target,
    );
    if result.start_solid {
        result = trace(
            world,
            profile,
            Vec3 {
                x: point.x,
                y: point.y,
                z: (f64::from(point.z) + height + 1.0) as f32,
            },
            end,
            &target,
        );
    }
    if result.start_solid
        || result.all_solid
        || result.fraction == 1.0
        || !matches!(
            result.contact,
            crate::scene::TraceContact::Plane { plane }
                if f64::from(plane.normal.z) >= profile.minimum_floor_normal
        )
    {
        return None;
    }
    Some(result.end)
}

/// Source `Math.round` (half up, negative zero normalized) for grid keys.
fn js_round(value: f64) -> i64 {
    let rounded = (value + 0.5).floor();
    if rounded == 0.0 {
        0
    } else {
        rounded as i64
    }
}

/// Build a navigation graph by sampling decoded geometry.
pub fn construct_navigation(options: &NavigationConstruction<'_>) -> Result<NavigationGraph, BotsError> {
    let (geometry, map, profile, world) = (options.geometry, options.map, options.profile, options.world);
    validate_profile(profile)?;
    if map.format != geometry.kind() {
        return Err(BotsError::MapMismatch);
    }
    let spacing = options.spacing.unwrap_or(48.0);
    let link_distance = options.link_distance.unwrap_or(spacing * 2.1);
    let maximum_nodes = options.maximum_nodes.unwrap_or(100_000);
    if !spacing.is_finite()
        || spacing < 8.0
        || !link_distance.is_finite()
        || link_distance < spacing
        || maximum_nodes < 1
    {
        return Err(BotsError::ConstructionLimits);
    }
    let mut nodes: Vec<NavigationNode> = Vec::new();
    let mut edges: Vec<NavigationEdge> = Vec::new();
    let mut seen: HashSet<(i64, i64, i64)> = HashSet::new();
    let mut rejected: Vec<RejectedSource> = Vec::new();
    let crouched = crouched_profile(profile);
    let target = map_target();
    let insert = |origin: Vec3,
                  source: NavigationSource,
                  posture: &NavigationProfile,
                  crouch_posture: bool,
                  nodes: &mut Vec<NavigationNode>,
                  seen: &mut HashSet<(i64, i64, i64)>|
     -> Result<(), BotsError> {
        let key = (
            js_round(f64::from(origin.x) / 8.0),
            js_round(f64::from(origin.y) / 8.0),
            js_round(f64::from(origin.z) / 4.0),
        );
        if seen.contains(&key) || !clear(world, posture, origin, origin, &target) {
            return Ok(());
        }
        let medium = contents(world, profile, origin, &target);
        if (medium & 6) != 0 {
            return Ok(());
        }
        if nodes.len() >= maximum_nodes {
            return Err(BotsError::NodeLimit { maximum: maximum_nodes });
        }
        seen.insert(key);
        nodes.push(NavigationNode {
            id: nodes.len() as i32,
            origin,
            bounds: translated(origin, posture.shape.bounds()),
            radius: spacing / 2.0,
            contents: medium,
            flags: 0,
            presence: if crouch_posture { 4 } else { 2 },
            source_cluster: None,
            source,
        });
        Ok(())
    };
    let mut pending: Vec<(Vec3, NavigationSource)> = Vec::new();
    geometry_candidates(geometry, spacing, profile.minimum_floor_normal, &mut |point, source| {
        pending.push((point, source));
    })?;
    for (point, source) in pending {
        match grounded(world, profile, point) {
            Some(origin) => insert(origin, source, profile, false, &mut nodes, &mut seen)?,
            None => {
                if let Some(crouched) = &crouched {
                    if let Some(origin) = grounded(world, crouched, point) {
                        insert(origin, source, crouched, true, &mut nodes, &mut seen)?;
                    }
                }
            }
        }
    }
    let leaf_bounds: Vec<Bounds> = match geometry {
        DecodedWorld::Q1(world) => world.leaves.iter().map(|leaf| leaf.bounds).collect(),
        DecodedWorld::Q2(world) => world.leaves.iter().map(|leaf| leaf.bounds).collect(),
        DecodedWorld::Q3(world) => world.leaves.iter().map(|leaf| leaf.bounds).collect(),
    };
    for (index, bounds) in leaf_bounds.iter().enumerate() {
        let center = midpoint(bounds.min, bounds.max);
        let medium = contents(world, profile, center, &target);
        if (medium & 9) != 0 && (medium & 6) == 0 {
            insert(
                center,
                NavigationSource::Constructed {
                    surface: None,
                    leaf: Some(index as i32),
                },
                profile,
                false,
                &mut nodes,
                &mut seen,
            )?;
        }
    }
    let mut bins: HashMap<(i64, i64), Vec<i32>> = HashMap::new();
    let key = |point: Vec3, dx: i64, dy: i64| {
        (
            (f64::from(point.x) / link_distance).floor() as i64 + dx,
            (f64::from(point.y) / link_distance).floor() as i64 + dy,
        )
    };
    for node in &nodes {
        bins.entry(key(node.origin, 0, 0)).or_default().push(node.id);
    }
    for index in 0..nodes.len() {
        let node = nodes[index].clone();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let neighbors = bins.get(&key(node.origin, dx, dy)).cloned().unwrap_or_default();
                for other_id in neighbors {
                    if node.id == other_id {
                        continue;
                    }
                    let other = at(&nodes, other_id)?.clone();
                    let rise = f64::from(other.origin.z) - f64::from(node.origin.z);
                    let planar = (f64::from(node.origin.x) - f64::from(other.origin.x))
                        .hypot(f64::from(node.origin.y) - f64::from(other.origin.y));
                    if planar > link_distance || rise.abs() > profile.maximum_drop.max(link_distance) {
                        continue;
                    }
                    let mode = if (node.contents & 1) != 0 && (other.contents & 1) != 0 {
                        TravelMode::Swim
                    } else if (node.contents & 8) != 0 && (other.contents & 8) != 0 {
                        TravelMode::Ladder
                    } else if rise > profile.maximum_step {
                        TravelMode::Jump
                    } else if rise < -profile.maximum_step {
                        TravelMode::Drop
                    } else if node.presence == 4 || other.presence == 4 {
                        TravelMode::Crouch
                    } else {
                        TravelMode::Walk
                    };
                    if !profile.capabilities.contains(&mode) || mode == TravelMode::Drop && -rise > profile.maximum_drop
                    {
                        continue;
                    }
                    if mode == TravelMode::Walk || mode == TravelMode::Crouch {
                        let posture = if mode == TravelMode::Crouch {
                            crouched.as_ref().unwrap_or(profile)
                        } else {
                            profile
                        };
                        let up = profile.maximum_step + 1.0;
                        let raised = |point: Vec3| Vec3 {
                            x: point.x,
                            y: point.y,
                            z: (f64::from(point.z) + up) as f32,
                        };
                        if !clear(world, posture, node.origin, other.origin, &target)
                            && !clear(world, posture, raised(node.origin), raised(other.origin), &target)
                        {
                            continue;
                        }
                        let middle = midpoint(node.origin, other.origin);
                        let foot = Vec3 {
                            x: middle.x,
                            y: middle.y,
                            z: middle.z + profile.shape.bounds().min.z,
                        };
                        if grounded(world, posture, foot).is_none() {
                            continue;
                        }
                    } else if mode == TravelMode::Swim && !clear(world, profile, node.origin, other.origin, &target) {
                        continue;
                    }
                    edges.push(NavigationEdge {
                        id: edges.len() as i32,
                        from: node.id,
                        to: other.id,
                        mode,
                        start: node.origin,
                        end: other.origin,
                        travel_seconds: (distance(node.origin, other.origin) / 320.0).max(0.01),
                        source_travel_type: 0,
                        source_flags: 0,
                        hint: None,
                        entity: None,
                        source: node.source,
                    });
                }
            }
        }
    }
    let nearest = |point: Vec3| -> Option<NavigationNode> {
        let mut selected = None;
        let mut best = link_distance * 2.0;
        for node in &nodes {
            let d = distance(node.origin, point);
            if d < best {
                selected = Some(node.clone());
                best = d;
            }
        }
        selected
    };
    for connection in options.connections.unwrap_or(&[]) {
        let from = nearest(connection.from);
        let to = nearest(connection.to);
        match (from, to) {
            (Some(from), Some(to)) => edges.push(NavigationEdge {
                id: edges.len() as i32,
                from: from.id,
                to: to.id,
                mode: connection.mode,
                start: connection.from,
                end: connection.to,
                travel_seconds: connection.travel_seconds,
                source_travel_type: connection.source_travel_type,
                source_flags: 0,
                hint: connection.hint.or(if connection.mode == TravelMode::Mover {
                    Some(TraversalHint {
                        funnel: from.origin,
                        start: connection.from,
                        end: connection.to,
                        ladder_plane: None,
                    })
                } else {
                    None
                }),
                entity: connection.entity.clone(),
                source: NavigationSource::Constructed {
                    surface: None,
                    leaf: None,
                },
            }),
            (from, _to) => rejected.push(RejectedSource {
                source: NavigationSource::Constructed {
                    surface: None,
                    leaf: None,
                },
                reason: format!(
                    "Source connection {} has no nearby navigation {}",
                    connection.id,
                    if from.is_none() { "start" } else { "destination" }
                ),
            }),
        }
    }
    let clusters = navigation_clusters(&nodes, &edges);
    Ok(NavigationGraph {
        map: map.clone(),
        profile: profile.clone(),
        asset: None,
        nodes,
        edges,
        clusters,
        rejected,
    })
}
