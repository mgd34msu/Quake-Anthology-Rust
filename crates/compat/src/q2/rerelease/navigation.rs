//! Q2 rerelease bot navigation imports and advisory paths.
//!
//! Donor: `src/compat/q2/rerelease/navigation.ts` — bridges the `game.h`
//! bot movement imports and `PathRequest`/`PathInfo` structs.

use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

/// Goal status: 0 failed, 1 active, 2 arrived, 3 invalid.
pub type GoalStatus = u8;

/// Navigation failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NavigationError {
    /// Nonfinite navigation point.
    #[error("Nonfinite Q2 navigation point")]
    NonFinitePoint,
    /// Invalid bot movement tolerance.
    #[error("Invalid Q2 bot movement tolerance")]
    BadTolerance,
    /// Nonfinite navigation parameter.
    #[error("Nonfinite Q2 navigation parameter")]
    NonFiniteParameter,
    /// Invalid navigation point buffer.
    #[error("Invalid Q2 navigation point buffer")]
    BadBuffer,
    /// Unknown navigation import.
    #[error("Unknown Q2 navigation import {0}")]
    UnknownImport(String),
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Advisory path request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathRequest {
    /// Start point.
    pub start: Vec3,
    /// Goal point.
    pub goal: Vec3,
    /// Movement flags.
    pub flags: u32,
    /// Arrival distance.
    pub move_distance: f32,
    /// Ignore node flags.
    pub ignore_node_flags: bool,
    /// Minimum node height.
    pub min_height: f32,
    /// Maximum node height.
    pub max_height: f32,
    /// Search radius.
    pub radius: f32,
    /// Drop height.
    pub drop_height: f32,
    /// Jump height.
    pub jump_height: f32,
}

/// Advisory path result.
#[derive(Debug, Clone, PartialEq)]
pub struct PathInfo {
    /// Result code.
    pub code: i32,
    /// Squared path distance.
    pub distance_squared: f32,
    /// Path points.
    pub points: Vec<Vec3>,
    /// First traversal point.
    pub first: Vec3,
    /// Second traversal point.
    pub second: Vec3,
    /// Link type.
    pub link_type: i32,
}

/// Navigation graph node.
#[derive(Debug, Clone, PartialEq)]
pub struct NavNode {
    /// Node id.
    pub id: u32,
    /// Origin.
    pub origin: Vec3,
    /// Node flags.
    pub flags: u32,
    /// Node radius.
    pub radius: f32,
    /// Whether the node comes from a nav3 source.
    pub nav3: bool,
}

/// Navigation edge mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    /// Walk.
    Walk,
    /// Crouch.
    Crouch,
    /// Swim.
    Swim,
    /// Drop.
    Drop,
    /// Jump.
    Jump,
    /// Mover.
    Mover,
    /// Teleport.
    Teleport,
}

/// Navigation graph edge.
#[derive(Debug, Clone, PartialEq)]
pub struct NavEdge {
    /// Source node.
    pub from: u32,
    /// Destination node.
    pub to: u32,
    /// Edge mode.
    pub mode: EdgeMode,
    /// Source travel type.
    pub source_travel_type: i32,
    /// Traversal hint: start/end when present.
    pub hint: Option<(Vec3, Vec3)>,
    /// Edge start.
    pub start: Vec3,
    /// Edge end.
    pub end: Vec3,
    /// Bound entity, if any.
    pub entity: Option<u32>,
}

/// Headless navigation runtime: live graph plus scripted trace answers.
#[derive(Debug, Clone, PartialEq)]
pub struct NavRuntime {
    /// Graph nodes.
    pub nodes: Vec<NavNode>,
    /// Graph edges.
    pub edges: Vec<NavEdge>,
    /// Whether point traces report start-solid.
    pub start_solid: bool,
    /// Whether segment traces are blocked.
    pub blocked: bool,
}

impl NavRuntime {
    /// Outgoing edges from a node.
    #[must_use]
    pub fn outgoing(&self, node: u32) -> Vec<&NavEdge> {
        self.edges.iter().filter(|edge| edge.from == node).collect()
    }

    /// Node by id.
    #[must_use]
    pub fn node(&self, id: u32) -> Option<&NavNode> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

fn distance(a: Vec3, b: Vec3) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

fn failure(code: i32) -> PathInfo {
    PathInfo {
        code,
        distance_squared: 0.0,
        points: Vec::new(),
        first: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        second: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        link_type: 0,
    }
}

fn node_allowed(node: &NavNode, request: &PathRequest) -> bool {
    if !node.nav3 {
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

fn link_type(edge: &NavEdge) -> i32 {
    match edge.mode {
        EdgeMode::Drop => 1,
        EdgeMode::Jump => {
            if edge.source_travel_type == 5 {
                3
            } else {
                2
            }
        }
        EdgeMode::Mover => 4,
        _ => 0,
    }
}

fn link_allowed(edge: &NavEdge, request: &PathRequest) -> bool {
    if request.ignore_node_flags {
        return true;
    }
    if request.flags == 1 && edge.mode != EdgeMode::Swim && edge.mode != EdgeMode::Walk {
        return false;
    }
    match edge.mode {
        EdgeMode::Walk | EdgeMode::Swim => edge.entity.is_none(),
        EdgeMode::Drop => {
            (request.flags & 4) != 0 && (request.drop_height <= 0.0 || edge.start.z - edge.end.z <= request.drop_height)
        }
        EdgeMode::Jump => {
            (request.flags & if edge.source_travel_type == 5 { 16 } else { 8 }) != 0
                && (edge.source_travel_type != 5
                    || request.jump_height <= 0.0
                    || edge.end.z - edge.start.z <= request.jump_height)
        }
        EdgeMode::Mover => (request.flags & 32) != 0,
        EdgeMode::Crouch | EdgeMode::Teleport => false,
    }
}

/// Source advisory paths share the live graph; native monsters execute
/// their own movement (`bots/navigation/rerelease-path.ts`).
#[must_use]
pub fn path_to_goal(runtime: Option<&NavRuntime>, request: &PathRequest) -> PathInfo {
    let Some(runtime) = runtime else {
        return failure(8);
    };
    if runtime.nodes.is_empty() {
        return failure(8);
    }
    if (request.flags & 3) == 0 {
        return failure(12);
    }
    let nearest = |point: Vec3| {
        let mut selected: Option<&NavNode> = None;
        let mut best = if request.radius > 0.0 { request.radius } else { 512.0 };
        for node in &runtime.nodes {
            if !node_allowed(node, request)
                || node.origin.z
                    < point.z
                        - if request.min_height > 0.0 {
                            request.min_height
                        } else {
                            64.0
                        }
                || node.origin.z
                    > point.z
                        + if request.max_height > 0.0 {
                            request.max_height
                        } else {
                            64.0
                        }
            {
                continue;
            }
            let d = ((node.origin.x - point.x).powi(2) + (node.origin.y - point.y).powi(2)).sqrt();
            if d > best || runtime.blocked {
                continue;
            }
            selected = Some(node);
            best = d;
        }
        selected
    };
    let Some(start) = nearest(request.start) else {
        return failure(9);
    };
    let Some(goal) = nearest(request.goal) else {
        return failure(10);
    };
    if start.id == goal.id || distance(request.start, goal.origin) <= request.move_distance {
        return failure(0);
    }
    if !request.ignore_node_flags && runtime.start_solid {
        return failure(6);
    }
    let start_id = start.id;
    let goal_id = goal.id;
    let mut costs = std::collections::HashMap::from([(start_id, 0.0f32)]);
    let mut previous: std::collections::HashMap<u32, NavEdge> = std::collections::HashMap::new();
    let mut queue = vec![(start_id, 0.0f32)];
    while let Some((current, cost)) = {
        queue.sort_by(|a, b| b.1.total_cmp(&a.1));
        queue.pop()
    } {
        if costs.get(&current) != Some(&cost) {
            continue;
        }
        if current == goal_id {
            break;
        }
        for edge in runtime.outgoing(current) {
            let Some(node) = runtime.node(edge.to) else {
                continue;
            };
            if !node_allowed(node, request) || !link_allowed(edge, request) {
                continue;
            }
            let step = if edge.mode == EdgeMode::Teleport {
                1.0
            } else {
                distance(
                    runtime.node(edge.from).map_or(edge.start, |node| node.origin),
                    node.origin,
                )
            };
            let next = cost + step;
            if next >= costs.get(&edge.to).copied().unwrap_or(f32::INFINITY) {
                continue;
            }
            costs.insert(edge.to, next);
            previous.insert(edge.to, (*edge).clone());
            queue.push((edge.to, next));
        }
    }
    if !previous.contains_key(&goal_id) {
        return failure(11);
    }
    let mut edges: Vec<NavEdge> = Vec::new();
    let mut nodes: Vec<NavNode> = vec![goal.clone()];
    let mut cursor = goal_id;
    while cursor != start_id {
        let Some(edge) = previous.get(&cursor) else {
            return failure(11);
        };
        if edges.len() >= runtime.nodes.len() {
            return failure(11);
        }
        let Some(node) = runtime.node(edge.from) else {
            return failure(11);
        };
        edges.push(edge.clone());
        nodes.push(node.clone());
        cursor = edge.from;
    }
    nodes.reverse();
    edges.reverse();
    let first_edge = edges.first();
    let mut first = 0usize;
    if !request.ignore_node_flags {
        if let Some(edge) = first_edge {
            if edge.mode == EdgeMode::Walk || edge.mode == EdgeMode::Crouch {
                let dx = request.start.x - start.origin.x;
                let dy = request.start.y - start.origin.y;
                let length = dx.hypot(dy);
                let next = nodes.get(1);
                let aligned = length > 0.0
                    && next.is_some_and(|next| {
                        dx * (next.origin.x - start.origin.x) + dy * (next.origin.y - start.origin.y)
                            > start.radius * length
                    });
                if (length <= start.radius && (request.start.z - start.origin.z).abs() <= 64.0) || aligned {
                    first = 1;
                }
            }
        }
    }
    let selected: Vec<NavNode> = nodes.iter().skip(first).cloned().collect();
    let Some(first_node) = selected.first() else {
        return failure(11);
    };
    let mut walked = 0.0f32;
    let mut at = request.start;
    for node in &selected {
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
    let traversal = !request.ignore_node_flags && first_edge.is_some_and(|edge| edge.hint.is_some());
    let (first_point, second_point) = if request.ignore_node_flags {
        (Vec3 { x: 0.0, y: 0.0, z: 0.0 }, Vec3 { x: 0.0, y: 0.0, z: 0.0 })
    } else if traversal {
        let edge = first_edge.expect("traversal edge");
        (edge.start, edge.end)
    } else {
        (
            first_node.origin,
            selected.get(1).map_or(request.goal, |node| node.origin),
        )
    };
    PathInfo {
        code: if request.ignore_node_flags {
            3
        } else if traversal {
            2
        } else {
            4
        },
        distance_squared: walked * walked,
        points,
        first: first_point,
        second: second_point,
        link_type: first_edge.map_or(0, link_type),
    }
}

/// Navigation services behind the imports.
pub trait NavigationServices {
    /// Live runtime, if navigation is available.
    fn runtime(&self) -> Option<&NavRuntime>;
    /// Move an actor toward a point.
    fn move_to_point(&mut self, actor: u32, point: Vec3, tolerance: f32) -> GoalStatus;
    /// Follow a target actor.
    fn follow_actor(&mut self, actor: u32, target: u32) -> GoalStatus;
}

/// `game.h` API 2023 Microsoft x64: `PathRequest` is 80 bytes and
/// `PathInfo` is 40 bytes.
pub struct RereleaseNavigationImports<S> {
    /// Services.
    pub services: S,
}

impl<S: NavigationServices> RereleaseNavigationImports<S> {
    /// Create over services.
    #[must_use]
    pub fn new(services: S) -> Self {
        Self { services }
    }

    /// Dispatch one navigation import.
    pub fn invoke(
        &mut self,
        memory: &mut SparseGuestMemory,
        name: &str,
        args: &[GuestCallValue],
        actor_of: &dyn Fn(GuestAddress) -> Option<u32>,
    ) -> Result<GuestCallResult, NavigationError> {
        let pointer = |index: usize| -> Option<GuestAddress> {
            match args.get(index) {
                Some(GuestCallValue::Pointer(address)) => *address,
                _ => None,
            }
        };
        let required = |index: usize| -> Result<GuestAddress, NavigationError> {
            pointer(index).ok_or(NavigationError::NonFinitePoint)
        };
        let vector = |memory: &mut SparseGuestMemory, address: GuestAddress| {
            memory
                .read_f32x3(address)
                .map_err(NavigationError::from)
                .and_then(|value| {
                    if [value.x, value.y, value.z].iter().all(|lane| lane.is_finite()) {
                        Ok(value)
                    } else {
                        Err(NavigationError::NonFinitePoint)
                    }
                })
        };
        let finite = |memory: &mut SparseGuestMemory, address: GuestAddress| {
            memory
                .read_f32(address)
                .map_err(NavigationError::from)
                .and_then(|value| {
                    if value.is_finite() {
                        Ok(value)
                    } else {
                        Err(NavigationError::NonFiniteParameter)
                    }
                })
        };
        if name == "Bot_FollowActor" {
            let actor = actor_of(required(0)?);
            let target = actor_of(required(1)?);
            let status = match (actor, target) {
                (Some(actor), Some(target)) => self.services.follow_actor(actor, target),
                _ => 0,
            };
            return Ok(GuestCallResult::Value(GuestCallValue::Int32(i32::from(status))));
        }
        if name == "Bot_MoveToPoint" {
            let actor = actor_of(required(0)?);
            let point = vector(memory, required(1)?)?;
            let tolerance = match args.get(2) {
                Some(GuestCallValue::Float32(value)) if value.is_finite() && *value >= 0.0 => *value,
                _ => return Err(NavigationError::BadTolerance),
            };
            let status = actor.map_or(0, |actor| self.services.move_to_point(actor, point, tolerance));
            return Ok(GuestCallResult::Value(GuestCallValue::Int32(i32::from(status))));
        }
        if name != "GetPathToGoal" {
            return Err(NavigationError::UnknownImport(name.to_string()));
        }
        let request = required(0)?;
        let output = required(1)?;
        memory.check(request, 80, qa_guest::core::contracts::GuestAccess::Read)?;
        memory.check(output, 40, qa_guest::core::contracts::GuestAccess::Write)?;
        let buffer = memory.read_pointer(memory.offset(request, 64)?)?;
        let count = memory.read_i64(memory.offset(request, 72)?)?;
        if !(0..=0x7fff_ffff).contains(&count) || (count > 0 && buffer.is_none()) {
            return Err(NavigationError::BadBuffer);
        }
        if let Some(buffer) = buffer {
            memory.check(
                buffer,
                count as usize * 12,
                qa_guest::core::contracts::GuestAccess::Write,
            )?;
        }
        let result = path_to_goal(
            self.services.runtime(),
            &PathRequest {
                start: vector(memory, request)?,
                goal: vector(memory, memory.offset(request, 12)?)?,
                flags: memory.read_u32(memory.offset(request, 24)?)?,
                move_distance: finite(memory, memory.offset(request, 28)?)?,
                ignore_node_flags: memory.read_u8(memory.offset(request, 36)?)? != 0,
                min_height: finite(memory, memory.offset(request, 40)?)?,
                max_height: finite(memory, memory.offset(request, 44)?)?,
                radius: finite(memory, memory.offset(request, 48)?)?,
                drop_height: finite(memory, memory.offset(request, 52)?)?,
                jump_height: finite(memory, memory.offset(request, 56)?)?,
            },
        );
        let write_vec = |memory: &mut SparseGuestMemory, address: GuestAddress, point: Vec3| {
            memory.write_f32(address, point.x)?;
            memory.write_f32(memory.offset(address, 4)?, point.y)?;
            memory.write_f32(memory.offset(address, 8)?, point.z)?;
            Ok::<(), NavigationError>(())
        };
        let points: Vec<Vec3> = result.points.into_iter().take(count as usize).collect();
        if let Some(buffer) = buffer {
            for (index, point) in points.iter().enumerate() {
                write_vec(memory, memory.offset(buffer, index as i64 * 12)?, *point)?;
            }
        }
        memory.write_i32(output, points.len() as i32)?;
        memory.write_f32(memory.offset(output, 4)?, result.distance_squared)?;
        write_vec(memory, memory.offset(output, 8)?, result.first)?;
        write_vec(memory, memory.offset(output, 20)?, result.second)?;
        memory.write_i32(memory.offset(output, 32)?, result.link_type)?;
        memory.write_i32(memory.offset(output, 36)?, result.code)?;
        Ok(GuestCallResult::Value(GuestCallValue::Uint32(u32::from(
            result.code < 5,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    struct FakeServices {
        runtime: Option<NavRuntime>,
        moves: Vec<(u32, Vec3, f32)>,
    }

    impl NavigationServices for FakeServices {
        fn runtime(&self) -> Option<&NavRuntime> {
            self.runtime.as_ref()
        }
        fn move_to_point(&mut self, actor: u32, point: Vec3, tolerance: f32) -> GoalStatus {
            self.moves.push((actor, point, tolerance));
            1
        }
        fn follow_actor(&mut self, _actor: u32, _target: u32) -> GoalStatus {
            2
        }
    }

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "navigation-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn runtime() -> NavRuntime {
        NavRuntime {
            nodes: vec![
                NavNode {
                    id: 1,
                    origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    flags: 0,
                    radius: 64.0,
                    nav3: true,
                },
                NavNode {
                    id: 2,
                    origin: Vec3 {
                        x: 200.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    flags: 0,
                    radius: 64.0,
                    nav3: true,
                },
            ],
            edges: vec![NavEdge {
                from: 1,
                to: 2,
                mode: EdgeMode::Walk,
                source_travel_type: 0,
                hint: None,
                start: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                end: Vec3 {
                    x: 200.0,
                    y: 0.0,
                    z: 0.0,
                },
                entity: None,
            }],
            start_solid: false,
            blocked: false,
        }
    }

    #[test]
    fn advisory_paths_search_the_live_graph() {
        let runtime = runtime();
        let request = PathRequest {
            start: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            goal: Vec3 {
                x: 200.0,
                y: 0.0,
                z: 0.0,
            },
            flags: 3,
            move_distance: 16.0,
            ignore_node_flags: false,
            min_height: 0.0,
            max_height: 0.0,
            radius: 0.0,
            drop_height: 0.0,
            jump_height: 0.0,
        };
        let info = path_to_goal(Some(&runtime), &request);
        assert_eq!(info.code, 4);
        assert!(!info.points.is_empty());
        assert!(info.distance_squared > 0.0);
        assert_eq!(path_to_goal(None, &request).code, 8);
        let bad_flags = PathRequest { flags: 0, ..request };
        assert_eq!(path_to_goal(Some(&runtime), &bad_flags).code, 12);
        let arrived = PathRequest {
            start: Vec3 {
                x: 200.0,
                y: 0.0,
                z: 0.0,
            },
            ..request
        };
        assert_eq!(path_to_goal(Some(&runtime), &arrived).code, 0);
    }

    #[test]
    fn imports_move_follow_and_fill_path_info() {
        let mut memory = test_memory();
        let mut imports = RereleaseNavigationImports::new(FakeServices {
            runtime: Some(runtime()),
            moves: Vec::new(),
        });
        let space = memory.address_space();
        let actor_of = |address: GuestAddress| (address.offset == 0x700).then_some(11u32);
        let edict = GuestAddress::new(space, 0x700);
        let point = memory.allocate(&GuestAllocationOptions::bytes(12)).expect("alloc");
        memory.write_f32(point, 5.0).expect("x");
        let moved = imports
            .invoke(
                &mut memory,
                "Bot_MoveToPoint",
                &[
                    GuestCallValue::Pointer(Some(edict)),
                    GuestCallValue::Pointer(Some(point)),
                    GuestCallValue::Float32(8.0),
                ],
                &actor_of,
            )
            .expect("move");
        assert_eq!(moved, GuestCallResult::Value(GuestCallValue::Int32(1)));
        let followed = imports
            .invoke(
                &mut memory,
                "Bot_FollowActor",
                &[
                    GuestCallValue::Pointer(Some(edict)),
                    GuestCallValue::Pointer(Some(edict)),
                ],
                &actor_of,
            )
            .expect("follow");
        assert_eq!(followed, GuestCallResult::Value(GuestCallValue::Int32(2)));
        let request = memory.allocate(&GuestAllocationOptions::bytes(80)).expect("alloc");
        let output = memory.allocate(&GuestAllocationOptions::bytes(40)).expect("alloc");
        let points = memory.allocate(&GuestAllocationOptions::bytes(48)).expect("alloc");
        memory
            .write_f32(memory.offset(request, 12).expect("o"), 200.0)
            .expect("goal");
        memory
            .write_u32(memory.offset(request, 24).expect("o"), 3)
            .expect("flags");
        memory
            .write_f32(memory.offset(request, 28).expect("o"), 16.0)
            .expect("dist");
        memory
            .write_pointer(memory.offset(request, 64).expect("o"), Some(points))
            .expect("buf");
        memory
            .write_i64(memory.offset(request, 72).expect("o"), 4)
            .expect("count");
        let ok = imports
            .invoke(
                &mut memory,
                "GetPathToGoal",
                &[
                    GuestCallValue::Pointer(Some(request)),
                    GuestCallValue::Pointer(Some(output)),
                ],
                &actor_of,
            )
            .expect("path");
        assert_eq!(ok, GuestCallResult::Value(GuestCallValue::Uint32(1)));
        assert_eq!(memory.read_i32(memory.offset(output, 36).expect("o")).expect("code"), 4);
        assert!(memory.read_i32(output).expect("n") > 0);
    }
}
