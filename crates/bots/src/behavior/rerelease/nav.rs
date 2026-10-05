//! Rerelease navigation vocabulary from `src/bots/behavior/rerelease/nav.ts`.
//!
//! Path vocabulary over [`NavigationRuntime`](crate::runtime::NavigationRuntime):
//! searches and movement admission stay with the runtime; this module
//! maps edges to link types, plans paths with traversal caps, and
//! exposes train transport steps.

use std::collections::HashSet;

use qa_core::math::Vec3;

use crate::behavior::rerelease::math::{bvec, bvec_distance, bvec_normalized, bvec_sub};
use crate::runtime::{NavigationRouteQuery, NavigationRuntime};
use crate::types::{NavigationRouteResult, TravelMode};

/// Navigation link type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavLinkType {
    /// Walk.
    Walk = 0,
    /// Long jump.
    LongJump = 1,
    /// Teleport.
    Teleport = 2,
    /// Walk off ledge.
    WalkOffLedge = 3,
    /// Pusher (jump pad).
    Pusher = 4,
    /// Barrier jump.
    BarrierJump = 5,
    /// Elevator.
    Elevator = 6,
    /// Train.
    Train = 7,
    /// Manual long jump.
    ManualLongJump = 8,
    /// Crouch.
    Crouch = 9,
    /// Ladder.
    Ladder = 10,
}

impl NavLinkType {
    /// Convert from the donor integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Walk),
            1 => Some(Self::LongJump),
            2 => Some(Self::Teleport),
            3 => Some(Self::WalkOffLedge),
            4 => Some(Self::Pusher),
            5 => Some(Self::BarrierJump),
            6 => Some(Self::Elevator),
            7 => Some(Self::Train),
            8 => Some(Self::ManualLongJump),
            9 => Some(Self::Crouch),
            10 => Some(Self::Ladder),
            _ => None,
        }
    }
}

/// Traversal funnel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavTraversalT {
    /// Funnel point.
    pub funnel: Vec3,
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
}

/// Entity bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavEntityBounds {
    /// Mins.
    pub mins: Vec3,
    /// Maxs.
    pub maxs: Vec3,
}

/// Graph link.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavGraphLinkT {
    /// From node.
    pub from: i32,
    /// To node.
    pub to: i32,
    /// Link type.
    pub link_type: NavLinkType,
    /// Traversal.
    pub traversal: Option<NavTraversalT>,
    /// Entity bounds.
    pub entity_bounds: Option<NavEntityBounds>,
}

/// Graph node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavGraphNodeT {
    /// Index.
    pub index: i32,
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Flags.
    pub flags: i32,
}

/// Traversal caps.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NavTraverseCapsT {
    /// Jump links.
    pub jump: bool,
    /// Walk-off-ledge links.
    pub walk_off_ledge: bool,
    /// Entity traversals.
    pub entity_traversal: bool,
    /// Swim.
    pub swim: bool,
    /// Maximum drop.
    pub max_drop: f32,
    /// Maximum jump height.
    pub max_jump_height: f32,
    /// Avoided node ids (lava/slime).
    pub avoid_nodes: Vec<i32>,
}

/// Default traversal caps.
#[must_use]
pub fn default_traverse_caps() -> NavTraverseCapsT {
    NavTraverseCapsT {
        jump: true,
        walk_off_ledge: true,
        entity_traversal: true,
        swim: true,
        max_drop: 0.0,
        max_jump_height: 0.0,
        avoid_nodes: Vec::new(),
    }
}

/// Whether a link type is a jump.
#[must_use]
pub fn nav_link_is_jump(link_type: NavLinkType) -> bool {
    matches!(
        link_type,
        NavLinkType::LongJump | NavLinkType::BarrierJump | NavLinkType::ManualLongJump
    )
}

/// Whether a link type is entity-owned.
#[must_use]
pub fn nav_link_is_entity(link_type: NavLinkType) -> bool {
    matches!(
        link_type,
        NavLinkType::Teleport | NavLinkType::Pusher | NavLinkType::Elevator | NavLinkType::Train
    )
}

/// Start search height above the bot.
pub const PLAN_START_ABOVE: f32 = 56.0;

/// Flat steering direction.
#[must_use]
pub fn steer_direction(from: Vec3, to: Vec3) -> Vec3 {
    let direction = bvec_sub(to, from);
    if direction.x.abs() < 0.001 && direction.y.abs() < 0.001 {
        bvec()
    } else {
        bvec_normalized(Vec3 {
            x: direction.x,
            y: direction.y,
            z: 0.0,
        })
    }
}

/// Planned path.
#[derive(Debug, Clone, PartialEq)]
pub struct NavPathT {
    /// Node ids.
    pub nodes: Vec<i32>,
    /// Steering points.
    pub points: Vec<Vec3>,
    /// Links per point.
    pub links: Vec<Option<NavGraphLinkT>>,
    /// Cost seconds.
    pub cost: f64,
    /// Runtime generation.
    pub generation: i64,
    /// Map identity.
    pub map_identity: String,
}

/// Plan options.
#[derive(Debug, Clone, Default)]
pub struct NavPlanOptions {
    /// Traversal caps.
    pub caps: Option<NavTraverseCapsT>,
    /// Maximum search radius.
    pub max_radius: Option<f32>,
    /// Start search height.
    pub start_above: Option<f32>,
}

/// Transport step for entity links.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BotTransportStep {
    /// Move to a target.
    Move {
        /// Approach or exit stage.
        approach: bool,
        /// Target.
        target: Vec3,
    },
    /// Wait.
    Wait,
    /// Ride.
    Ride,
    /// Unavailable.
    Unavailable,
}

/// Rerelease navigation over a shared runtime.
pub trait RereleaseNavigation {
    /// Node count.
    fn node_count(&self) -> usize;
    /// Node views.
    fn nodes(&self) -> Vec<NavGraphNodeT>;
    /// Plan a path.
    fn plan_path(&mut self, start: Vec3, goal: Vec3, options: &NavPlanOptions) -> Option<NavPathT>;
    /// Whether a path is still valid.
    fn path_valid(&mut self, path: &NavPathT) -> bool;
    /// Transport step for an entity link.
    fn transport(&mut self, link: &NavGraphLinkT, origin: Vec3) -> Option<BotTransportStep>;
}

fn link_type_for(mode: TravelMode, source_travel_type: i32) -> Option<NavLinkType> {
    match mode {
        TravelMode::Walk | TravelMode::Swim => Some(NavLinkType::Walk),
        TravelMode::Crouch => Some(NavLinkType::Crouch),
        TravelMode::Ladder => Some(NavLinkType::Ladder),
        TravelMode::Jump | TravelMode::WaterJump => {
            if source_travel_type == 5 {
                Some(NavLinkType::BarrierJump)
            } else {
                Some(NavLinkType::LongJump)
            }
        }
        TravelMode::Drop => Some(NavLinkType::WalkOffLedge),
        TravelMode::Teleport => Some(NavLinkType::Teleport),
        TravelMode::JumpPad => Some(NavLinkType::Pusher),
        TravelMode::Mover => {
            if source_travel_type == 7 {
                Some(NavLinkType::Train)
            } else {
                Some(NavLinkType::Elevator)
            }
        }
        _ => None,
    }
}

/// Source rerelease navigation: holds only a borrowed runtime.
pub struct SourceRereleaseNavigation<'w> {
    runtime: NavigationRuntime<'w>,
}

impl<'w> SourceRereleaseNavigation<'w> {
    /// New navigation over a runtime.
    pub fn new(runtime: NavigationRuntime<'w>) -> Self {
        Self { runtime }
    }

    /// Borrow the runtime.
    #[must_use]
    pub fn runtime(&self) -> &NavigationRuntime<'w> {
        &self.runtime
    }

    /// Node views.
    #[must_use]
    pub fn nodes(&self) -> Vec<NavGraphNodeT> {
        self.runtime
            .graph
            .nodes
            .iter()
            .map(|node| NavGraphNodeT {
                index: node.id,
                origin: node.origin,
                radius: node.radius as f32,
                flags: node.flags,
            })
            .collect()
    }

    fn nearest(&self, point: Vec3, options: &NavPlanOptions, above: Option<f32>) -> Option<i32> {
        let mut selected = None;
        let mut best = options.max_radius.unwrap_or(512.0);
        for node in &self.runtime.graph.nodes {
            if let Some(above) = above {
                if node.origin.z > point.z + above {
                    continue;
                }
            }
            let distance = bvec_distance(point, node.origin);
            if distance > best {
                continue;
            }
            selected = Some(node.id);
            best = distance;
        }
        selected
    }
}

impl RereleaseNavigation for SourceRereleaseNavigation<'_> {
    fn node_count(&self) -> usize {
        self.runtime.graph.nodes.len()
    }

    fn nodes(&self) -> Vec<NavGraphNodeT> {
        SourceRereleaseNavigation::nodes(self)
    }

    fn plan_path(&mut self, start: Vec3, goal: Vec3, options: &NavPlanOptions) -> Option<NavPathT> {
        let caps = options.caps.clone().unwrap_or_else(default_traverse_caps);
        let start_node = self.nearest(start, options, options.start_above)?;
        let goal_node = self.nearest(goal, options, None)?;
        let result = self.runtime.route(&NavigationRouteQuery {
            start,
            goal,
            start_node: Some(start_node),
            goal_node: Some(goal_node),
            travel_flags: None,
            disabled_areas: caps.avoid_nodes.iter().copied().collect::<HashSet<_>>(),
            edge_filter: None,
            maximum_searches: None,
        });
        let NavigationRouteResult::Route { route } = result.ok()? else {
            return None;
        };
        let mut points = Vec::new();
        let mut links: Vec<Option<NavGraphLinkT>> = Vec::new();
        for edge in &route.edges {
            let link_type = link_type_for(edge.mode, edge.source_travel_type)?;
            if !caps.jump && nav_link_is_jump(link_type) {
                return None;
            }
            if !caps.entity_traversal && nav_link_is_entity(link_type) {
                return None;
            }
            if !caps.walk_off_ledge && link_type == NavLinkType::WalkOffLedge {
                return None;
            }
            if !caps.swim && edge.mode == TravelMode::Swim {
                return None;
            }
            if caps.max_drop > 0.0 && edge.mode != TravelMode::Mover && edge.start.z - edge.end.z >= caps.max_drop {
                return None;
            }
            if caps.max_jump_height > 0.0
                && nav_link_is_jump(link_type)
                && edge.end.z - edge.start.z >= caps.max_jump_height
            {
                return None;
            }
            let link = NavGraphLinkT {
                from: edge.from,
                to: edge.to,
                link_type,
                traversal: edge.hint.as_ref().map(|hint| NavTraversalT {
                    funnel: hint.funnel,
                    start: edge.start,
                    end: edge.end,
                }),
                entity_bounds: edge.entity.as_ref().map(|entity| NavEntityBounds {
                    mins: entity.bounds.min,
                    maxs: entity.bounds.max,
                }),
            };
            if link_type != NavLinkType::Walk || edge.entity.is_some() {
                if link.traversal.is_some() {
                    push_point(&mut points, &mut links, edge.start, Some(link));
                    push_point(&mut points, &mut links, edge.end, None);
                } else {
                    push_point(&mut points, &mut links, edge.end, Some(link));
                }
            } else {
                push_point(&mut points, &mut links, edge.end, None);
            }
        }
        if points.is_empty() {
            push_point(&mut points, &mut links, goal, None);
        }
        Some(NavPathT {
            nodes: route.nodes.clone(),
            points,
            links,
            cost: route.travel_seconds,
            generation: route.generation,
            map_identity: route.map.identity.canonical(),
        })
    }

    fn path_valid(&mut self, path: &NavPathT) -> bool {
        path.generation == self.runtime.generation() && path.map_identity == self.runtime.graph.map.identity.canonical()
    }

    fn transport(&mut self, link: &NavGraphLinkT, origin: Vec3) -> Option<BotTransportStep> {
        if link.link_type != NavLinkType::Train {
            return None;
        }
        let edge = self
            .runtime
            .outgoing(link.from)
            .iter()
            .find(|candidate| {
                candidate.to == link.to
                    && link_type_for(candidate.mode, candidate.source_travel_type) == Some(NavLinkType::Train)
            })?
            .clone();
        let step = self.runtime.train_step(&edge, origin, None)?;
        match step {
            crate::runtime::TrainStep::Move { stage, target } => Some(BotTransportStep::Move {
                approach: stage == crate::runtime::TrainStage::Approach,
                target,
            }),
            crate::runtime::TrainStep::Ride => Some(BotTransportStep::Ride),
            crate::runtime::TrainStep::Wait => Some(BotTransportStep::Wait),
            crate::runtime::TrainStep::Unavailable => Some(BotTransportStep::Unavailable),
        }
    }
}

fn push_point(
    points: &mut Vec<Vec3>,
    links: &mut Vec<Option<NavGraphLinkT>>,
    point: Vec3,
    link: Option<NavGraphLinkT>,
) {
    if let Some(previous) = points.last() {
        if bvec_distance(*previous, point) < 1.0 {
            if link.is_some() {
                if let Some(slot) = links.last_mut() {
                    *slot = link;
                }
            }
            return;
        }
    }
    points.push(point);
    links.push(link);
}
