//! Shared advisory paths for the Quake II rerelease, ported from
//! `src/bots/navigation/rerelease-path.ts`. Source advisory paths share
//! the live graph; native monsters execute their own movement.

use qa_core::math::Vec3;

use crate::error::BotsError;
use crate::helpers::distance;
use crate::runtime::NavigationRuntime;
use crate::scene::LeafContents;
use crate::types::{NavigationEdge, NavigationNode};

/// Rerelease path request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RereleasePathRequest {
    /// Path start.
    pub start: Vec3,
    /// Path goal.
    pub goal: Vec3,
    /// Monster flags.
    pub flags: i32,
    /// Arrival distance.
    pub move_distance: f64,
    /// Ignore node flags.
    pub ignore_node_flags: bool,
    /// Minimum search height.
    pub min_height: f64,
    /// Maximum search height.
    pub max_height: f64,
    /// Search radius.
    pub radius: f64,
    /// Maximum drop.
    pub drop_height: f64,
    /// Maximum jump rise.
    pub jump_height: f64,
}

/// Rerelease path outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleasePathInfo {
    /// Result code.
    pub code: i32,
    /// Walked distance squared.
    pub distance_squared: f64,
    /// Path points.
    pub points: Vec<Vec3>,
    /// First traversal point.
    pub first: Vec3,
    /// Second traversal point.
    pub second: Vec3,
    /// First link type.
    pub link_type: i32,
}

fn zero() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
}

fn failure(code: i32) -> RereleasePathInfo {
    RereleasePathInfo {
        code,
        distance_squared: 0.0,
        points: Vec::new(),
        first: zero(),
        second: zero(),
        link_type: 0,
    }
}

fn node_allowed(node: &NavigationNode, request: &RereleasePathRequest) -> bool {
    if !matches!(
        node.source,
        crate::types::NavigationSource::Kex {
            generation: crate::types::KexGeneration::Nav3,
            ..
        }
    ) {
        return true;
    }
    if (node.flags & 8192) != 0 {
        return false;
    }
    if request.ignore_node_flags {
        return (node.flags & 1024) == 0;
    }
    if (node.flags & (1 | 2 | 8 | 256 | 512)) != 0 {
        return false;
    }
    let medium = request.flags & 3;
    !(medium == 2 && (node.flags & 16) != 0
        || medium == 1 && (node.flags & 16) == 0
        || (request.flags & 32) == 0 && (node.flags & 4) != 0)
}

fn link_type(edge: &NavigationEdge) -> i32 {
    if edge.mode == crate::types::TravelMode::Drop {
        1
    } else if edge.mode == crate::types::TravelMode::Jump {
        if edge.source_travel_type == 5 {
            3
        } else {
            2
        }
    } else if edge.mode == crate::types::TravelMode::Mover {
        4
    } else {
        0
    }
}

fn link_allowed(edge: &NavigationEdge, request: &RereleasePathRequest) -> bool {
    use crate::types::TravelMode;
    if request.ignore_node_flags {
        return true;
    }
    if request.flags == 1 && edge.mode != TravelMode::Swim && edge.mode != TravelMode::Walk {
        return false;
    }
    match edge.mode {
        TravelMode::Walk | TravelMode::Swim => edge.entity.is_none(),
        TravelMode::Drop => {
            (request.flags & 4) != 0
                && (request.drop_height <= 0.0
                    || f64::from(edge.start.z) - f64::from(edge.end.z) <= request.drop_height)
        }
        TravelMode::Jump => {
            (request.flags & if edge.source_travel_type == 5 { 16 } else { 8 }) != 0
                && (edge.source_travel_type != 5
                    || request.jump_height <= 0.0
                    || f64::from(edge.end.z) - f64::from(edge.start.z) <= request.jump_height)
        }
        TravelMode::Mover => (request.flags & 32) != 0,
        _ => false,
    }
}

/// Shared advisory path to a goal.
pub fn rerelease_path_to_goal(
    runtime: Option<&NavigationRuntime<'_>>,
    request: &RereleasePathRequest,
) -> Result<RereleasePathInfo, BotsError> {
    let Some(runtime) = runtime else {
        return Ok(failure(8));
    };
    if runtime.graph.nodes.is_empty() {
        return Ok(failure(8));
    }
    if (request.flags & 3) == 0 {
        return Ok(failure(12));
    }
    let world = runtime.world();
    let trace = |start: Vec3, end: Vec3| {
        world.scene().trace(&crate::scene::TraceQuery {
            start,
            end,
            shape: crate::scene::TraceShape::Point,
            target: crate::scene::QueryTarget::World,
            policy: crate::scene::TracePolicy::Q2 {
                contents_mask: 0x30003,
                leaf_contents: LeafContents::Merged,
            },
            numeric: runtime.graph.profile.movement.numeric,
            pass_actor: world.pass_actor(),
        })
    };
    let nearest = |point: Vec3| {
        let mut selected = None;
        let mut best = if request.radius > 0.0 { request.radius } else { 512.0 };
        for node in &runtime.graph.nodes {
            if !node_allowed(node, request)
                || f64::from(node.origin.z)
                    < f64::from(point.z)
                        - if request.min_height > 0.0 {
                            request.min_height
                        } else {
                            64.0
                        }
                || f64::from(node.origin.z)
                    > f64::from(point.z)
                        + if request.max_height > 0.0 {
                            request.max_height
                        } else {
                            64.0
                        }
            {
                continue;
            }
            let d =
                (f64::from(node.origin.x) - f64::from(point.x)).hypot(f64::from(node.origin.y) - f64::from(point.y));
            if d > best {
                continue;
            }
            let result = trace(
                point,
                Vec3 {
                    x: node.origin.x,
                    y: node.origin.y,
                    z: node.origin.z + 32.0,
                },
            );
            if result.fraction < 1.0 || result.start_solid || result.all_solid {
                continue;
            }
            selected = Some(node);
            best = d;
        }
        selected
    };
    let Some(start) = nearest(request.start) else {
        return Ok(failure(9));
    };
    let Some(goal) = nearest(request.goal) else {
        return Ok(failure(10));
    };
    if start.id == goal.id || distance(request.start, goal.origin) <= request.move_distance {
        return Ok(failure(0));
    }
    if !request.ignore_node_flags {
        if trace(request.start, request.start).start_solid {
            return Ok(failure(6));
        }
        if trace(request.goal, request.goal).start_solid {
            return Ok(failure(7));
        }
    }
    let mut costs = std::collections::HashMap::from([(start.id, 0.0)]);
    let mut previous: std::collections::HashMap<i32, NavigationEdge> = std::collections::HashMap::new();
    let mut queue = vec![(start.id, 0.0)];
    while !queue.is_empty() {
        queue.sort_by(|a: &(i32, f64), b: &(i32, f64)| a.1.total_cmp(&b.1));
        let (node, cost) = queue.remove(0);
        if Some(cost) != costs.get(&node).copied() {
            continue;
        }
        if node == goal.id {
            break;
        }
        for edge in runtime.outgoing(node) {
            let Some(node) = runtime.node(edge.to) else {
                continue;
            };
            if !node_allowed(node, request) || !link_allowed(edge, request) || !runtime.edge_allowed(edge, None) {
                continue;
            }
            let cost = cost
                + if edge.mode == crate::types::TravelMode::Teleport {
                    1.0
                } else {
                    distance(
                        runtime.node(edge.from).map_or(edge.start, |node| node.origin),
                        node.origin,
                    )
                };
            if cost >= costs.get(&edge.to).copied().unwrap_or(f64::INFINITY) {
                continue;
            }
            costs.insert(edge.to, cost);
            previous.insert(edge.to, edge.clone());
            queue.push((edge.to, cost));
        }
    }
    if !previous.contains_key(&goal.id) {
        return Ok(failure(11));
    }
    let mut edges: Vec<NavigationEdge> = Vec::new();
    let mut nodes: Vec<&NavigationNode> = vec![goal];
    let mut cursor = goal.id;
    while cursor != start.id {
        let Some(edge) = previous.get(&cursor) else {
            return Err(BotsError::Internal(
                "Invalid shared navigation predecessor chain".to_string(),
            ));
        };
        if edges.len() >= runtime.graph.nodes.len() {
            return Err(BotsError::Internal(
                "Invalid shared navigation predecessor chain".to_string(),
            ));
        }
        let Some(node) = runtime.node(edge.from) else {
            return Err(BotsError::Internal("Missing shared navigation predecessor".to_string()));
        };
        edges.push(edge.clone());
        nodes.push(node);
        cursor = edge.from;
    }
    nodes.reverse();
    edges.reverse();
    let edge = edges.first();
    let mut first = 0;
    if !request.ignore_node_flags {
        if let Some(edge) = edge {
            if edge.mode == crate::types::TravelMode::Walk || edge.mode == crate::types::TravelMode::Crouch {
                let dx = f64::from(request.start.x) - f64::from(start.origin.x);
                let dy = f64::from(request.start.y) - f64::from(start.origin.y);
                let length = dx.hypot(dy);
                let next = nodes.get(1);
                if length <= start.radius && (f64::from(request.start.z) - f64::from(start.origin.z)).abs() <= 64.0
                    || length > 0.0
                        && next.is_some_and(|next| {
                            dx * (f64::from(next.origin.x) - f64::from(start.origin.x))
                                + dy * (f64::from(next.origin.y) - f64::from(start.origin.y))
                                > start.radius * length
                        })
                {
                    first = 1;
                }
            }
        }
    }
    let selected = &nodes[first..];
    let Some(first_node) = selected.first() else {
        return Err(BotsError::Internal("Empty shared navigation result".to_string()));
    };
    let mut walked = 0.0;
    let mut at = request.start;
    for node in selected {
        walked += distance(at, node.origin);
        at = node.origin;
    }
    walked += distance(at, request.goal);
    let mut points: Vec<Vec3> = selected.iter().map(|node| node.origin).collect();
    if distance(request.start, first_node.origin) > 64.0 {
        points.insert(0, request.start);
    }
    if distance(goal.origin, request.goal) > 64.0 {
        points.push(request.goal);
    }
    let traversal = !request.ignore_node_flags && edge.is_some_and(|edge| edge.hint.is_some());
    Ok(RereleasePathInfo {
        code: if request.ignore_node_flags {
            3
        } else if traversal {
            2
        } else {
            4
        },
        distance_squared: walked * walked,
        points,
        first: if request.ignore_node_flags {
            zero()
        } else if traversal {
            edge.map_or_else(zero, |edge| edge.start)
        } else {
            first_node.origin
        },
        second: if request.ignore_node_flags {
            zero()
        } else if traversal {
            edge.map_or_else(zero, |edge| edge.end)
        } else {
            selected.get(1).map_or(request.goal, |node| node.origin)
        },
        link_type: edge.map_or(0, link_type),
    })
}
