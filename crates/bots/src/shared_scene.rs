//! Shared scene queries: one session owner over geometry collision plus
//! linked actors. Model targets bypass actors.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/index.ts`
//! (`SharedSceneQueries`, `createSceneQueries`, `sweptBounds`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_world::body::{BodyState, BodyTable, LinkedBody};
use qa_world::collision::q3::{CollisionMapSettings, SourceClipModels};
use qa_world::collision::{actor_contents, swept_bounds};
use qa_world::registry::ActorRegistry;
use qa_world::spatial::{
    ActorCollision, CollisionFamily, CollisionRole, CollisionShape, QueryRole, SpatialActor, SpatialIndex,
};
use qa_world::WorldError;

use crate::actor_body::trace_actor_body;
use crate::collision_support::{adapt_point_contents, adapt_trace_result};
use crate::q1_collision::{create_shared_q1_collision, Q1Collision, Q1CollisionGeometry};
use crate::q2_collision::{Q2Collision, Q2CollisionGeometry};
use crate::q3_collision::Q3Collision;
use crate::scene::{
    LeafContents, LeafQueryResult, PointContentsQuery, PointContentsResult, Q1MoveRule, QueryTarget, SceneQueries,
    TraceDetail, TraceHit, TracePolicy, TraceQuery, TraceResult, TraceShape, VisibilityKind,
};
use qa_world::collision::q3::Q3CollisionGeometry;

/// Decoded world geometry feeding [`SharedSceneQueries`].
#[derive(Debug, Clone, PartialEq)]
pub enum DecodedCollisionWorld {
    /// Quake I BSP geometry.
    Q1(Q1CollisionGeometry),
    /// Quake II BSP geometry.
    Q2(Q2CollisionGeometry),
    /// Quake III BSP geometry.
    Q3(Q3CollisionGeometry),
}

enum GeometryCollision {
    Q1(Box<Q1Collision>),
    Q2(Q2Collision),
    Q3(Q3Collision),
}

/// Shared actor-state reader.
pub type ActorStateReader = Rc<dyn Fn(&ActorId) -> Option<BodyState>>;

/// Shared actor-collision reader.
pub type ActorCollisionReader = Rc<dyn Fn(&ActorId) -> Option<ActorCollision>>;

/// Actor-state source: a shared body table or a read callback.
#[derive(Clone)]
pub enum ActorStateSource {
    /// Shared body table plus its actor registry.
    Table {
        /// Body table.
        table: Rc<BodyTable>,
        /// Actor registry.
        registry: Rc<ActorRegistry>,
    },
    /// Read callback.
    Callback(ActorStateReader),
}

impl ActorStateSource {
    /// Read state for an actor.
    #[must_use]
    pub fn read(&self, actor: &ActorId) -> Option<BodyState> {
        match self {
            ActorStateSource::Table { table, registry } => table.read(registry, actor),
            ActorStateSource::Callback(read) => read(actor),
        }
    }
}

/// Quake II area-portal contribution: the primary switch plus held counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PortalContribution {
    primary: bool,
    count: i32,
}

fn collision_contents(collision: &ActorCollision, to: CollisionFamily) -> i32 {
    actor_contents(
        collision.family,
        collision.shape,
        collision.contents,
        collision.dead_monster,
        to,
    )
}

fn world_policy(policy: &TracePolicy) -> qa_world::collision::TracePolicy {
    use qa_world::collision::{LeafContents as WorldLeaf, Q1Move, TracePolicy as WorldPolicy};
    match policy {
        TracePolicy::Q1 { move_rule, .. } => WorldPolicy::Q1 {
            movement: match move_rule {
                Q1MoveRule::Normal => Q1Move::Normal,
                Q1MoveRule::NoMonsters => Q1Move::NoMonsters,
                Q1MoveRule::Missile => Q1Move::Missile,
            },
        },
        TracePolicy::Q2 {
            contents_mask,
            leaf_contents,
        } => WorldPolicy::Q2 {
            contents_mask: *contents_mask,
            leaf_contents: match leaf_contents {
                LeafContents::Stored => WorldLeaf::Stored,
                LeafContents::Merged => WorldLeaf::Merged,
            },
        },
        TracePolicy::Q3 { contents_mask, .. } => WorldPolicy::Q3 {
            contents_mask: *contents_mask,
        },
    }
}

fn swept_query_bounds(query: &TraceQuery) -> Bounds {
    let shape = match &query.shape {
        TraceShape::Point => None,
        TraceShape::Box { bounds } | TraceShape::Capsule { bounds } => Some(bounds),
    };
    swept_bounds(query.start, query.end, shape, world_policy(&query.policy))
}

fn missile_bounds() -> Bounds {
    Bounds {
        min: vec3(-15.0, -15.0, -15.0),
        max: vec3(15.0, 15.0, 15.0),
    }
}

/// Shared scene queries over one world's geometry and actors
/// (`SharedSceneQueries`).
pub struct SharedSceneQueries {
    geometry: DecodedCollisionWorld,
    provider: GeometryCollision,
    spatial: SpatialIndex,
    actor_states: Option<ActorStateSource>,
    read_actor_collision: Option<ActorCollisionReader>,
    q2_portal_contributions: RefCell<HashMap<i32, PortalContribution>>,
}

impl SharedSceneQueries {
    /// Build shared queries over decoded geometry.
    pub fn new(world: DecodedCollisionWorld) -> Result<Self, WorldError> {
        let provider = match &world {
            DecodedCollisionWorld::Q1(geometry) => {
                GeometryCollision::Q1(Box::new(create_shared_q1_collision(geometry.clone())))
            }
            DecodedCollisionWorld::Q2(geometry) => GeometryCollision::Q2(Q2Collision::new(geometry.clone())),
            DecodedCollisionWorld::Q3(geometry) => GeometryCollision::Q3(Q3Collision::new(geometry)?),
        };
        let bounds = match &provider {
            GeometryCollision::Q1(queries) => queries.model_bounds(0)?,
            GeometryCollision::Q2(queries) => queries.model_bounds(0)?,
            GeometryCollision::Q3(queries) => queries.model_bounds(0)?,
        };
        Ok(Self {
            geometry: world,
            provider,
            spatial: SpatialIndex::new(&bounds),
            actor_states: None,
            read_actor_collision: None,
            q2_portal_contributions: RefCell::new(HashMap::new()),
        })
    }

    /// Borrow the decoded geometry.
    #[must_use]
    pub fn geometry(&self) -> &DecodedCollisionWorld {
        &self.geometry
    }

    /// Borrow the spatial index.
    #[must_use]
    pub fn spatial(&self) -> &SpatialIndex {
        &self.spatial
    }

    /// Native Quake III clip models, when the geometry is Quake III.
    #[must_use]
    pub fn native_q3_clip_models(&self) -> Option<SourceClipModels<'_>> {
        match &self.provider {
            GeometryCollision::Q3(queries) => Some(queries.world().source_clip_models()),
            _ => None,
        }
    }

    /// Bind collision settings to a Quake III world; other families ignore it.
    pub fn bind_collision_settings(&self, settings: CollisionMapSettings) -> Result<(), WorldError> {
        if let GeometryCollision::Q3(queries) = &self.provider {
            queries.world().bind_settings(settings)?;
        }
        Ok(())
    }

    /// Link a body snapshot.
    pub fn link(&mut self, body: &LinkedBody, collision: &ActorCollision) {
        self.spatial.link(body, collision);
    }

    /// Unlink an actor.
    pub fn unlink(&mut self, actor: &ActorId) {
        self.spatial.unlink(actor);
    }

    /// Fetch a linked actor with current state and collision.
    #[must_use]
    pub fn linked_actor(&self, actor: &ActorId) -> Option<SpatialActor> {
        self.spatial.get(actor).and_then(|linked| self.current_actor(&linked))
    }

    /// Bind an actor-state source.
    pub fn bind_actor_state(&mut self, source: ActorStateSource) {
        self.actor_states = Some(source);
    }

    /// Bind an actor-collision reader.
    pub fn bind_actor_collision(&mut self, read: impl Fn(&ActorId) -> Option<ActorCollision> + 'static) {
        self.read_actor_collision = Some(Rc::new(read));
    }

    fn current_actor(&self, linked: &SpatialActor) -> Option<SpatialActor> {
        if self.actor_states.is_none() && self.read_actor_collision.is_none() {
            return Some(linked.clone());
        }
        let collision = match &self.read_actor_collision {
            None => linked.collision.clone(),
            Some(read) => read(&linked.body.actor)?,
        };
        self.snapshot_actor(linked, &collision)
    }

    fn snapshot_actor(&self, linked: &SpatialActor, collision: &ActorCollision) -> Option<SpatialActor> {
        let state = match &self.actor_states {
            None => linked.body.state.clone(),
            Some(source) => source.read(&linked.body.actor)?,
        };
        Some(SpatialActor {
            body: LinkedBody {
                state,
                ..linked.body.clone()
            },
            collision: collision.clone(),
        })
    }

    /// Query actors by role with current state and collision.
    #[must_use]
    pub fn query_actors(&self, bounds: &Bounds, role: QueryRole) -> Vec<SpatialActor> {
        let mut actors = Vec::new();
        let stored = self.spatial.query(
            bounds,
            if self.read_actor_collision.is_none() {
                role
            } else {
                QueryRole::Both
            },
        );
        for linked in stored {
            let refresh = self.actor_states.is_some() || self.read_actor_collision.is_some();
            let collision = match &self.read_actor_collision {
                None => linked.collision.clone(),
                Some(read) => match read(&linked.body.actor) {
                    Some(collision) => collision,
                    None => continue,
                },
            };
            if role != QueryRole::Both
                && collision.role
                    != match role {
                        QueryRole::Solid => CollisionRole::Solid,
                        QueryRole::Trigger => CollisionRole::Trigger,
                        QueryRole::Both => unreachable!("both returns above"),
                    }
            {
                continue;
            }
            let actor = if refresh {
                self.snapshot_actor(&linked, &collision)
            } else {
                Some(linked)
            };
            if let Some(actor) = actor {
                actors.push(actor);
            }
        }
        actors
    }

    /// Leaf containing a point.
    pub fn point_leaf(&self, point: Vec3) -> Result<i32, WorldError> {
        match &self.provider {
            GeometryCollision::Q1(queries) => queries.point_leaf(point),
            GeometryCollision::Q2(queries) => queries.point_leaf(point, 0),
            GeometryCollision::Q3(queries) => queries.point_leaf(point),
        }
    }

    /// Cluster for a leaf.
    pub fn leaf_cluster(&self, leaf: i32) -> Result<i32, WorldError> {
        match &self.provider {
            GeometryCollision::Q1(_) => Ok(Q1Collision::leaf_cluster(leaf)),
            GeometryCollision::Q2(queries) => queries.leaf_cluster(leaf),
            GeometryCollision::Q3(queries) => queries.leaf_cluster(leaf),
        }
    }

    /// Area for a leaf.
    pub fn leaf_area(&self, leaf: i32) -> Result<i32, WorldError> {
        match &self.provider {
            GeometryCollision::Q1(queries) => Ok(queries.leaf_area(leaf)),
            GeometryCollision::Q2(queries) => queries.leaf_area(leaf),
            GeometryCollision::Q3(queries) => queries.leaf_area(leaf),
        }
    }

    /// Model bounds.
    pub fn model_bounds(&self, model: i32) -> Result<Bounds, WorldError> {
        match &self.provider {
            GeometryCollision::Q1(queries) => queries.model_bounds(model),
            GeometryCollision::Q2(queries) => queries.model_bounds(model),
            GeometryCollision::Q3(queries) => queries.model_bounds(model),
        }
    }

    /// Leaves touched by bounds.
    pub fn box_leaves(&self, bounds: &Bounds, limit: usize) -> Result<LeafQueryResult, WorldError> {
        match &self.provider {
            GeometryCollision::Q1(queries) => queries.box_leaves(bounds, limit),
            GeometryCollision::Q2(queries) => queries.box_leaves(bounds, limit, 0),
            GeometryCollision::Q3(queries) => queries.box_leaves(bounds, limit),
        }
    }

    /// Area connectivity.
    pub fn areas_connected(&self, first: i32, second: i32) -> Result<bool, WorldError> {
        match &self.provider {
            GeometryCollision::Q1(queries) => Ok(queries.areas_connected(first, second)),
            GeometryCollision::Q2(queries) => queries.areas_connected(first, second),
            GeometryCollision::Q3(queries) => queries.areas_connected(first, second),
        }
    }

    /// Area visibility bits.
    pub fn area_bits(&self, area: i32) -> Result<Vec<u8>, WorldError> {
        match &self.provider {
            GeometryCollision::Q1(queries) => Ok(queries.area_bits(area)),
            GeometryCollision::Q2(queries) => queries.area_bits(area),
            GeometryCollision::Q3(queries) => queries.area_bits(area),
        }
    }

    /// Cluster visibility.
    pub fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> Result<bool, WorldError> {
        match &self.provider {
            GeometryCollision::Q1(queries) => queries.cluster_visible(from, to, kind),
            GeometryCollision::Q2(queries) => queries.cluster_visible(from, to, kind),
            GeometryCollision::Q3(queries) => queries.cluster_visible(from, to, kind),
        }
    }

    /// Set a Quake II area-portal switch.
    pub fn set_area_portal_state(&self, portal: i32, open: bool) -> Result<(), WorldError> {
        let GeometryCollision::Q2(queries) = &self.provider else {
            return Err(WorldError::BadCollisionRecord(
                "Portal identifiers belong to Quake II maps".to_string(),
            ));
        };
        let mut contributions = self.q2_portal_contributions.borrow_mut();
        if let Some(contribution) = contributions.get_mut(&portal) {
            contribution.primary = open;
        }
        let held = contributions.get(&portal).map_or(0, |entry| entry.count);
        queries.set_area_portal_state(portal, open || held > 0)
    }

    /// Hold or release a Quake II area-portal contribution.
    pub fn adjust_area_portal_contribution(&self, portal: i32, delta: i32) -> Result<(), WorldError> {
        let GeometryCollision::Q2(queries) = &self.provider else {
            return Err(WorldError::BadCollisionRecord(
                "Portal identifiers belong to Quake II maps".to_string(),
            ));
        };
        let mut contributions = self.q2_portal_contributions.borrow_mut();
        let mut contribution = contributions.get(&portal).copied().unwrap_or(PortalContribution {
            primary: queries.portal_state().contains(&portal),
            count: 0,
        });
        let count = contribution.count + delta;
        if count < 0 {
            return Err(WorldError::BadCollisionRecord(
                "Area portal contribution underflow".to_string(),
            ));
        }
        queries.set_area_portal_state(portal, contribution.primary || count > 0)?;
        contribution.count = count;
        if count == 0 {
            contributions.remove(&portal);
        } else {
            contributions.insert(portal, contribution);
        }
        Ok(())
    }

    /// Open Quake II area portals.
    pub fn q2_portal_state(&self) -> Result<Vec<i32>, WorldError> {
        let GeometryCollision::Q2(queries) = &self.provider else {
            return Err(WorldError::BadCollisionRecord(
                "Portal identifiers belong to Quake II maps".to_string(),
            ));
        };
        Ok(queries.portal_state())
    }

    /// Set a Quake III area-pair portal switch.
    pub fn adjust_area_portal_state(&self, first: i32, second: i32, open: bool) -> Result<(), WorldError> {
        let GeometryCollision::Q3(queries) = &self.provider else {
            return Err(WorldError::BadCollisionRecord(
                "Area-pair portal references belong to Quake III maps".to_string(),
            ));
        };
        queries.adjust_area_portal_state(first, second, open)
    }

    /// Trace against geometry only, adapted to the query policy.
    pub fn geometry_trace(&self, query: &TraceQuery) -> Result<TraceResult, WorldError> {
        let normalized;
        let query = match &query.shape {
            TraceShape::Box { bounds }
                if bounds.min.x == 0.0
                    && bounds.min.y == 0.0
                    && bounds.min.z == 0.0
                    && bounds.max.x == 0.0
                    && bounds.max.y == 0.0
                    && bounds.max.z == 0.0 =>
            {
                normalized = TraceQuery {
                    shape: TraceShape::Point,
                    ..query.clone()
                };
                &normalized
            }
            _ => query,
        };
        let native = match &self.provider {
            GeometryCollision::Q1(queries) => queries.trace(query)?,
            GeometryCollision::Q2(queries) => queries.trace(query)?,
            GeometryCollision::Q3(queries) => queries.trace(query)?,
        };
        let result = adapt_trace_result(&native, &query.policy);
        if !matches!(result.detail, TraceDetail::Q1 { .. }) || matches!(self.provider, GeometryCollision::Q1(_)) {
            return Ok(result);
        }
        let media = match &self.provider {
            GeometryCollision::Q2(queries) => queries.trace_media(query, result.fraction)?,
            GeometryCollision::Q3(queries) => queries.trace_media(query, result.fraction)?,
            GeometryCollision::Q1(_) => unreachable!("q1 returns above"),
        };
        let mut merged = result;
        if let TraceDetail::Q1 { in_open, in_water, .. } = &mut merged.detail {
            *in_open = media.in_open;
            *in_water = media.in_water;
        }
        Ok(merged)
    }
}

impl SharedSceneQueries {
    /// Trace against geometry and linked actors.
    pub fn trace(&self, query: &TraceQuery) -> Result<TraceResult, WorldError> {
        self.trace_inner(query, &[])
    }

    /// Trace while skipping listed actors.
    pub fn trace_excluding(&self, query: &TraceQuery, excluded: &[ActorId]) -> Result<TraceResult, WorldError> {
        self.trace_inner(query, excluded)
    }

    fn trace_inner(&self, query: &TraceQuery, excluded: &[ActorId]) -> Result<TraceResult, WorldError> {
        let mut result = self.geometry_trace(query)?;
        let model_target = matches!(query.target, QueryTarget::Model { .. });
        let q3_zero = matches!(query.policy, TracePolicy::Q3 { .. }) && result.fraction == 0.0;
        if model_target || result.all_solid || q3_zero {
            return Ok(result);
        }
        let pass = query.pass_actor.as_ref().and_then(|actor| self.spatial.get(actor));
        let pass_collision = match &query.pass_actor {
            None => None,
            Some(actor) => match &self.read_actor_collision {
                None => pass.as_ref().map(|linked| linked.collision.clone()),
                Some(read) => read(actor),
            },
        };
        // WinQuake world.c:843-844 reads `passedict->v.size` for the
        // points rule; unknown pass actors keep the rule off like a
        // NULL `passedict`.
        let pass_size_x = query.pass_actor.as_ref().and_then(|actor| {
            if let Some(source) = &self.actor_states {
                if let Some(state) = source.read(actor) {
                    return Some(state.bounds.max.x - state.bounds.min.x);
                }
            }
            pass.as_ref()
                .map(|linked| linked.body.state.bounds.max.x - linked.body.state.bounds.min.x)
        });
        let pass_has_size = pass_size_x.is_some_and(|size| size != 0.0);
        let mut envelope = swept_query_bounds(query);
        if matches!(
            query.policy,
            TracePolicy::Q1 {
                move_rule: Q1MoveRule::Missile,
                ..
            }
        ) {
            let missile = TraceQuery {
                shape: TraceShape::Box {
                    bounds: missile_bounds(),
                },
                ..query.clone()
            };
            envelope = swept_query_bounds(&missile);
        }
        let stored = self.spatial.query(
            &envelope,
            if self.read_actor_collision.is_none() {
                QueryRole::Solid
            } else {
                QueryRole::Both
            },
        );
        for linked in stored {
            let refresh = self.actor_states.is_some() || self.read_actor_collision.is_some();
            let id = linked.body.actor.clone();
            let collision = match &self.read_actor_collision {
                None => linked.collision.clone(),
                Some(read) => match read(&id) {
                    Some(collision) => collision,
                    None => continue,
                },
            };
            if collision.role != CollisionRole::Solid {
                continue;
            }
            if excluded.iter().any(|actor| same_actor(actor, &id)) {
                continue;
            }
            // WinQuake world.c:843-844: points never interact. A sized
            // mover passes through zero-size touch entities (nails and
            // spikes); a point-sized mover still collides with them.
            if pass_has_size
                && matches!(query.policy, TracePolicy::Q1 { .. })
                && linked.body.state.bounds.max.x - linked.body.state.bounds.min.x == 0.0
            {
                continue;
            }
            if let Some(pass_actor) = &query.pass_actor {
                if same_actor(pass_actor, &id) {
                    continue;
                }
                let raw_pass = pass_collision.as_ref().and_then(|record| record.q3_owner);
                let raw_candidate = collision.q3_owner;
                if matches!(query.policy, TracePolicy::Q3 { .. }) && raw_pass.is_some() && raw_candidate.is_some() {
                    let pass_owner = raw_pass.unwrap_or(qa_world::spatial::Q3Owner {
                        entity_number: 0,
                        owner_number: 0,
                    });
                    let candidate = raw_candidate.unwrap_or(qa_world::spatial::Q3Owner {
                        entity_number: 0,
                        owner_number: 0,
                    });
                    let pass_owner_number = if pass_owner.owner_number == 1023 {
                        -1
                    } else {
                        pass_owner.owner_number
                    };
                    if candidate.owner_number == pass_owner.entity_number || candidate.owner_number == pass_owner_number
                    {
                        continue;
                    }
                } else {
                    if let Some(owner) = &collision.owner {
                        if same_actor(pass_actor, owner) {
                            continue;
                        }
                    }
                    if let Some(owner) = pass_collision.as_ref().and_then(|record| record.owner.as_ref()) {
                        let same_chain = if matches!(query.policy, TracePolicy::Q3 { .. }) {
                            collision
                                .owner
                                .as_ref()
                                .is_some_and(|candidate| same_actor(owner, candidate))
                        } else {
                            same_actor(owner, &id)
                        };
                        if same_chain {
                            continue;
                        }
                    }
                }
            }
            if collision.q1_corpse
                && !matches!(query.shape, TraceShape::Point)
                && match &query.shape {
                    TraceShape::Box { bounds } | TraceShape::Capsule { bounds } => {
                        bounds.min.x != bounds.max.x || bounds.min.y != bounds.max.y || bounds.min.z != bounds.max.z
                    }
                    TraceShape::Point => false,
                }
            {
                continue;
            }
            if matches!(
                query.policy,
                TracePolicy::Q1 {
                    move_rule: Q1MoveRule::NoMonsters,
                    ..
                }
            ) && !matches!(collision.shape, CollisionShape::Model(_))
            {
                continue;
            }
            let blocked = match &query.policy {
                TracePolicy::Q1 { .. } => collision_contents(&collision, CollisionFamily::Q1) != -2,
                TracePolicy::Q2 { contents_mask, .. } => {
                    collision_contents(&collision, CollisionFamily::Q2) & contents_mask == 0
                }
                TracePolicy::Q3 { contents_mask, .. } => {
                    collision_contents(&collision, CollisionFamily::Q3) & contents_mask == 0
                }
            };
            if blocked {
                continue;
            }
            let actor = if refresh {
                match self.snapshot_actor(&linked, &collision) {
                    Some(actor) => actor,
                    None => continue,
                }
            } else {
                linked
            };
            let missile = matches!(
                query.policy,
                TracePolicy::Q1 {
                    move_rule: Q1MoveRule::Missile,
                    ..
                }
            ) && collision.monster;
            let moving;
            let moving_query = if missile {
                moving = TraceQuery {
                    shape: TraceShape::Box {
                        bounds: missile_bounds(),
                    },
                    ..query.clone()
                };
                &moving
            } else {
                query
            };
            let mut hit = match collision.shape {
                CollisionShape::Model(model) => {
                    #[allow(clippy::cast_possible_wrap)]
                    let model = model as i32;
                    let model_query = TraceQuery {
                        target: QueryTarget::Model {
                            model,
                            origin: actor.body.state.origin,
                            angles: actor.body.state.angles,
                        },
                        ..moving_query.clone()
                    };
                    let hit = self.geometry_trace(&model_query)?;
                    if !matches!(hit.hit, TraceHit::None) {
                        TraceResult {
                            hit: TraceHit::Actor { actor: id.clone() },
                            ..hit
                        }
                    } else {
                        hit
                    }
                }
                _ => trace_actor_body(moving_query, &actor)?,
            };
            if matches!(query.policy, TracePolicy::Q3 { .. }) {
                if hit.fraction < result.fraction {
                    hit.start_solid = hit.start_solid || result.start_solid;
                    result = hit;
                } else {
                    result.all_solid = result.all_solid || hit.all_solid;
                    result.start_solid = result.start_solid || (!hit.all_solid && hit.start_solid);
                }
            } else if hit.all_solid
                || hit.fraction < result.fraction
                || matches!(query.policy, TracePolicy::Q1 { .. }) && hit.start_solid
            {
                hit.start_solid = hit.start_solid || result.start_solid;
                result = hit;
            } else if hit.start_solid {
                result.start_solid = true;
            }
            if result.all_solid {
                break;
            }
        }
        Ok(result)
    }

    /// Sample contents at a point, including linked actors.
    pub fn point_contents(&self, query: &PointContentsQuery) -> Result<PointContentsResult, WorldError> {
        let native = match &self.provider {
            GeometryCollision::Q1(queries) => queries.point_contents(query)?,
            GeometryCollision::Q2(queries) => queries.point_contents(query)?,
            GeometryCollision::Q3(queries) => queries.point_contents(query)?,
        };
        let mut result = adapt_point_contents(&native, &query.policy);
        if matches!(query.target, QueryTarget::Model { .. }) || matches!(query.policy, TracePolicy::Q1 { .. }) {
            return Ok(result);
        }
        let point_bounds = Bounds {
            min: query.point,
            max: query.point,
        };
        let stored = self.spatial.query(
            &point_bounds,
            if self.read_actor_collision.is_none() {
                QueryRole::Solid
            } else {
                QueryRole::Both
            },
        );
        for linked in stored {
            if let Some(pass_actor) = &query.pass_actor {
                if same_actor(pass_actor, &linked.body.actor) {
                    continue;
                }
            }
            let actor = match self.current_actor(&linked) {
                Some(actor) => actor,
                None => continue,
            };
            if actor.collision.role != CollisionRole::Solid {
                continue;
            }
            let collision = &actor.collision;
            let state = &actor.body.state;
            let added = match collision.shape {
                CollisionShape::Model(model) => {
                    #[allow(clippy::cast_possible_wrap)]
                    let model = model as i32;
                    let model_query = PointContentsQuery {
                        target: QueryTarget::Model {
                            model,
                            origin: state.origin,
                            angles: state.angles,
                        },
                        ..query.clone()
                    };
                    let native = match &self.provider {
                        GeometryCollision::Q1(queries) => queries.point_contents(&model_query)?,
                        GeometryCollision::Q2(queries) => queries.point_contents(&model_query)?,
                        GeometryCollision::Q3(queries) => queries.point_contents(&model_query)?,
                    };
                    let sample = adapt_point_contents(&native, &query.policy);
                    match sample {
                        PointContentsResult::Q2 { stored, merged } => {
                            if matches!(
                                query.policy,
                                TracePolicy::Q2 {
                                    leaf_contents: LeafContents::Stored,
                                    ..
                                }
                            ) {
                                stored
                            } else {
                                merged
                            }
                        }
                        PointContentsResult::Q1 { contents } | PointContentsResult::Q3 { contents } => contents,
                    }
                }
                _ => {
                    let local = vec3(
                        query.point.x - state.origin.x,
                        query.point.y - state.origin.y,
                        query.point.z - state.origin.z,
                    );
                    let inside = local.x >= state.bounds.min.x
                        && local.y >= state.bounds.min.y
                        && local.z >= state.bounds.min.z
                        && local.x <= state.bounds.max.x
                        && local.y <= state.bounds.max.y
                        && local.z <= state.bounds.max.z;
                    if !inside {
                        continue;
                    }
                    match &query.policy {
                        TracePolicy::Q2 { .. } => collision_contents(collision, CollisionFamily::Q2),
                        _ => collision_contents(collision, CollisionFamily::Q3),
                    }
                }
            };
            match &mut result {
                PointContentsResult::Q1 { contents } => {
                    if added == -2 {
                        *contents = -2;
                    }
                }
                PointContentsResult::Q2 { stored, merged } => {
                    *stored |= added;
                    *merged |= added;
                }
                PointContentsResult::Q3 { contents } => {
                    *contents |= added;
                }
            }
        }
        Ok(result)
    }
}

/// Build shared scene queries over decoded geometry (`createSceneQueries`).
pub fn create_scene_queries(world: DecodedCollisionWorld) -> Result<SharedSceneQueries, WorldError> {
    SharedSceneQueries::new(world)
}

impl SceneQueries for SharedSceneQueries {
    fn trace(&self, query: &TraceQuery) -> TraceResult {
        self.trace(query).expect("shared scene trace")
    }

    fn point_contents(&self, query: &PointContentsQuery) -> PointContentsResult {
        self.point_contents(query).expect("shared scene contents")
    }

    fn box_leaves(&self, bounds: &Bounds, limit: usize) -> LeafQueryResult {
        self.box_leaves(bounds, limit).expect("shared scene leaves")
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        self.areas_connected(first, second).expect("shared scene areas")
    }

    fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> bool {
        self.cluster_visible(from, to, kind).expect("shared scene visibility")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_core::numeric::Q3_BINARY32_PROFILE;
    use qa_world::body::LinkedBody;

    use crate::q1_collision::{Q1BspChild, Q1CollisionGeometry, Q1CollisionLeaf, Q1CollisionModel, Q1CollisionNode};
    use crate::q2_collision::{
        Q2BspChild, Q2CollisionArea, Q2CollisionAreaPortal, Q2CollisionBrush, Q2CollisionBrushSide,
        Q2CollisionGeometry, Q2CollisionLeaf, Q2CollisionModel, Q2CollisionNode, Q2IndexRange,
    };
    use crate::scene::{BspPlane, IndexRange, Q2SurfaceInfo};

    fn test_actor(slot: u32) -> ActorId {
        IdentityOwner::create("test").unwrap().actor(slot, 1)
    }

    fn q1_geometry() -> Q1CollisionGeometry {
        Q1CollisionGeometry {
            models: vec![Q1CollisionModel {
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                headnodes: vec![0, 0, 0],
                visible_leaves: 0,
                faces: IndexRange { first: 0, count: 0 },
            }],
            nodes: vec![Q1CollisionNode {
                plane: 0,
                children: [Q1BspChild::Leaf(0), Q1BspChild::Leaf(1)],
            }],
            clipnodes: vec![qa_world::hull::ClipNode {
                plane: 0,
                children: [
                    qa_world::hull::ClipChild::Contents(-2),
                    qa_world::hull::ClipChild::Contents(-1),
                ],
            }],
            planes: vec![BspPlane {
                normal: vec3(1.0, 0.0, 0.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            }],
            leaves: vec![
                Q1CollisionLeaf {
                    contents: -2,
                    visibility_offset: None,
                },
                Q1CollisionLeaf {
                    contents: -1,
                    visibility_offset: None,
                },
            ],
            faces: vec![],
            texture_info: vec![],
            textures: vec![],
            vertices: vec![],
            edges: vec![],
            surface_edges: vec![],
            visibility: vec![],
            brush_list: vec![],
        }
    }

    fn q2_geometry() -> Q2CollisionGeometry {
        let plane = |x: f32, y: f32, z: f32, distance: f32, plane_type: i32, signbits: i32| BspPlane {
            normal: vec3(x, y, z),
            distance,
            plane_type,
            signbits,
        };
        Q2CollisionGeometry {
            models: vec![Q2CollisionModel {
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                headnode: 0,
            }],
            nodes: vec![Q2CollisionNode {
                plane: 6,
                children: [Q2BspChild::Leaf(0), Q2BspChild::Leaf(1)],
            }],
            planes: vec![
                plane(1.0, 0.0, 0.0, 32.0, 0, 0),
                plane(-1.0, 0.0, 0.0, 0.0, 3, 1),
                plane(0.0, 1.0, 0.0, 16.0, 1, 0),
                plane(0.0, -1.0, 0.0, 16.0, 4, 2),
                plane(0.0, 0.0, 1.0, 16.0, 2, 0),
                plane(0.0, 0.0, -1.0, 16.0, 5, 4),
                plane(1.0, 0.0, 0.0, 0.0, 0, 0),
            ],
            leaves: vec![
                Q2CollisionLeaf {
                    cluster: 0,
                    area: 0,
                    contents: 1,
                    merged_contents: 1,
                    brushes: Q2IndexRange { first: 0, count: 1 },
                },
                Q2CollisionLeaf {
                    cluster: 1,
                    area: 0,
                    contents: 0,
                    merged_contents: 0,
                    brushes: Q2IndexRange { first: 0, count: 0 },
                },
            ],
            brushes: vec![Q2CollisionBrush {
                contents: 1,
                sides: Q2IndexRange { first: 0, count: 6 },
            }],
            brush_sides: (0..6)
                .map(|side| Q2CollisionBrushSide {
                    plane: side,
                    texture_info: 0,
                })
                .collect(),
            texture_info: vec![Q2SurfaceInfo {
                name: "rock".to_string(),
                flags: 3,
            }],
            leaf_brushes: vec![0],
            areas: vec![
                Q2CollisionArea {
                    portals: Q2IndexRange { first: 0, count: 0 },
                },
                Q2CollisionArea {
                    portals: Q2IndexRange { first: 0, count: 1 },
                },
            ],
            area_portals: vec![Q2CollisionAreaPortal {
                portal: 7,
                other_area: 0,
            }],
            visibility: None,
        }
    }

    fn body(actor: ActorId, bounds: Bounds) -> (LinkedBody, ActorCollision) {
        let state = BodyState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds,
            ground: None,
        };
        (
            LinkedBody {
                actor,
                state,
                absolute_bounds: bounds,
                link_count: 1,
            },
            ActorCollision {
                family: CollisionFamily::Q1,
                shape: CollisionShape::Box,
                contents: -2,
                owner: None,
                role: CollisionRole::Solid,
                monster: false,
                dead_monster: false,
                q1_corpse: false,
                q3_owner: None,
            },
        )
    }

    fn q1_query() -> TraceQuery {
        TraceQuery {
            start: vec3(-50.0, 0.0, 0.0),
            end: vec3(50.0, 0.0, 0.0),
            shape: TraceShape::Point,
            target: QueryTarget::World,
            policy: TracePolicy::Q1 {
                move_rule: Q1MoveRule::Normal,
                hull: None,
            },
            numeric: Q3_BINARY32_PROFILE,
            pass_actor: None,
        }
    }

    #[test]
    fn shared_trace_hits_actors() {
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        let geometry = shared.geometry_trace(&q1_query()).unwrap();
        assert_eq!(geometry.fraction, 0.4996874928474426);
        assert_eq!(geometry.hit, TraceHit::World { model: 0 });
        let actor = test_actor(1);
        let (linked, collision) = body(
            actor.clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        shared.link(&linked, &collision);
        assert!(shared.linked_actor(&actor).is_some());
        let hit = shared.trace(&q1_query()).unwrap();
        assert!(hit.fraction < geometry.fraction);
        assert_eq!(hit.hit, TraceHit::Actor { actor: actor.clone() });
        let skipped = shared
            .trace_excluding(&q1_query(), std::slice::from_ref(&actor))
            .unwrap();
        assert_eq!(skipped, geometry);
        let passing = TraceQuery {
            pass_actor: Some(actor.clone()),
            ..q1_query()
        };
        assert_eq!(shared.trace(&passing).unwrap(), geometry);
        let trigger = test_actor(2);
        let (linked, mut collision) = body(
            trigger.clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        collision.role = CollisionRole::Trigger;
        shared.link(&linked, &collision);
        let world = Bounds {
            min: vec3(-64.0, -64.0, -64.0),
            max: vec3(64.0, 64.0, 64.0),
        };
        assert_eq!(shared.query_actors(&world, QueryRole::Solid).len(), 1);
        assert_eq!(shared.query_actors(&world, QueryRole::Both).len(), 2);
        shared.unlink(&actor);
        assert!(shared.linked_actor(&actor).is_none());
        assert_eq!(shared.trace(&q1_query()).unwrap(), geometry);
    }

    #[test]
    fn shared_q1_points_never_interact() {
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        let geometry = shared.geometry_trace(&q1_query()).unwrap();
        // Sized player linked on the trace line; only its size matters.
        let player = test_actor(1);
        let (linked, collision) = body(
            player.clone(),
            Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
        );
        shared.link(&linked, &collision);
        // Zero-size nail sitting on the trace path.
        let nail = test_actor(2);
        let (mut linked, collision) = body(
            nail.clone(),
            Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
        );
        linked.state.origin = vec3(-20.0, 0.0, 0.0);
        linked.absolute_bounds = Bounds {
            min: vec3(-20.0, 0.0, 0.0),
            max: vec3(-20.0, 0.0, 0.0),
        };
        shared.link(&linked, &collision);
        // The point trace still uses the pass entity's size, not the
        // query shape: the sized player passes through the nail.
        let passing = TraceQuery {
            pass_actor: Some(player.clone()),
            ..q1_query()
        };
        let through = shared.trace(&passing).unwrap();
        assert_eq!(through, geometry);
        assert_eq!(through.hit, TraceHit::World { model: 0 });
        // Point-sized speck parked off the trace path.
        let speck = test_actor(3);
        let (mut linked, collision) = body(
            speck.clone(),
            Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
        );
        linked.state.origin = vec3(0.0, 100.0, 0.0);
        linked.absolute_bounds = Bounds {
            min: vec3(0.0, 100.0, 0.0),
            max: vec3(0.0, 100.0, 0.0),
        };
        shared.link(&linked, &collision);
        let blocker = test_actor(4);
        let (linked, collision) = body(
            blocker.clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        shared.link(&linked, &collision);
        // A point-sized mover still collides with sized bodies: the
        // rule needs a sized passer.
        let probing = TraceQuery {
            pass_actor: Some(speck.clone()),
            ..q1_query()
        };
        let hit = shared.trace(&probing).unwrap();
        assert!(hit.fraction < geometry.fraction);
        assert_eq!(hit.hit, TraceHit::Actor { actor: blocker.clone() });
        // A sized candidate still blocks a sized mover: the rule needs
        // a zero-size touch entity.
        let blocked = shared.trace(&passing).unwrap();
        assert!(blocked.fraction < geometry.fraction);
        assert_eq!(blocked.hit, TraceHit::Actor { actor: blocker.clone() });
    }

    #[test]
    fn shared_q1_owner_pass_rules() {
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        let geometry = shared.geometry_trace(&q1_query()).unwrap();
        // Owner parked off the trace path; only its identity matters.
        let player = test_actor(1);
        let (mut linked, collision) = body(
            player.clone(),
            Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
        );
        linked.state.origin = vec3(0.0, -100.0, 0.0);
        linked.absolute_bounds = Bounds {
            min: vec3(-16.0, -116.0, -24.0),
            max: vec3(16.0, -84.0, 32.0),
        };
        shared.link(&linked, &collision);
        // Missile on the trace path owned by the player.
        let missile = test_actor(2);
        let (linked, mut collision) = body(
            missile.clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        collision.owner = Some(player.clone());
        shared.link(&linked, &collision);
        // WinQuake world.c:851-852: a mover never clips against its
        // own missiles.
        let firing = TraceQuery {
            pass_actor: Some(player.clone()),
            ..q1_query()
        };
        assert_eq!(shared.trace(&firing).unwrap(), geometry);
        // Without a pass actor the missile blocks like any body.
        let hit = shared.trace(&q1_query()).unwrap();
        assert!(hit.fraction < geometry.fraction);
        assert_eq!(hit.hit, TraceHit::Actor { actor: missile.clone() });
    }

    #[test]
    fn shared_q1_missile_passes_owner() {
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        let geometry = shared.geometry_trace(&q1_query()).unwrap();
        // Owner on the trace path.
        let owner = test_actor(1);
        let (linked, collision) = body(
            owner.clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        shared.link(&linked, &collision);
        // Missile parked off the path, owned by the blocker.
        let missile = test_actor(2);
        let (mut linked, mut collision) = body(
            missile.clone(),
            Bounds {
                min: vec3(-2.0, -2.0, -2.0),
                max: vec3(2.0, 2.0, 2.0),
            },
        );
        linked.state.origin = vec3(0.0, 100.0, 0.0);
        linked.absolute_bounds = Bounds {
            min: vec3(-2.0, 98.0, -2.0),
            max: vec3(2.0, 102.0, 2.0),
        };
        collision.owner = Some(owner.clone());
        shared.link(&linked, &collision);
        // WinQuake world.c:853-854: a missile never clips against its
        // owner.
        let flying = TraceQuery {
            pass_actor: Some(missile.clone()),
            ..q1_query()
        };
        assert_eq!(shared.trace(&flying).unwrap(), geometry);
        // Without a pass actor the owner blocks like any body.
        let hit = shared.trace(&q1_query()).unwrap();
        assert!(hit.fraction < geometry.fraction);
        assert_eq!(hit.hit, TraceHit::Actor { actor: owner.clone() });
    }

    #[test]
    fn shared_q1_missile_expands_monsters() {
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        let geometry = shared.geometry_trace(&q1_query()).unwrap();
        let near = Bounds {
            min: vec3(-30.0, 0.5, -8.0),
            max: vec3(-8.0, 2.0, 8.0),
        };
        let monster = test_actor(1);
        let (linked, mut collision) = body(monster.clone(), near);
        collision.monster = true;
        shared.link(&linked, &collision);
        let crate_actor = test_actor(2);
        let (linked, collision) = body(crate_actor.clone(), near);
        shared.link(&linked, &collision);
        // A point sweep passes beside both bodies under Normal.
        assert_eq!(shared.trace(&q1_query()).unwrap(), geometry);
        // WinQuake world.c:940-947, 857-860: missiles clip monsters
        // with a ±15 box, and only monsters.
        let missile = TraceQuery {
            policy: TracePolicy::Q1 {
                move_rule: Q1MoveRule::Missile,
                hull: None,
            },
            ..q1_query()
        };
        let hit = shared.trace(&missile).unwrap();
        assert!(hit.fraction < geometry.fraction);
        assert_eq!(hit.hit, TraceHit::Actor { actor: monster.clone() });
        shared.unlink(&monster);
        assert_eq!(shared.trace(&missile).unwrap(), geometry);
    }

    #[test]
    fn shared_q1_nomonsters_skips_boxes() {
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        let geometry = shared.geometry_trace(&q1_query()).unwrap();
        let monster = test_actor(1);
        let (linked, mut collision) = body(
            monster.clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        collision.monster = true;
        shared.link(&linked, &collision);
        let hit = shared.trace(&q1_query()).unwrap();
        assert!(hit.fraction < geometry.fraction);
        assert_eq!(hit.hit, TraceHit::Actor { actor: monster.clone() });
        // WinQuake world.c:832-833: NoMonsters skips non-BSP bodies.
        let ghosts = TraceQuery {
            policy: TracePolicy::Q1 {
                move_rule: Q1MoveRule::NoMonsters,
                hull: None,
            },
            ..q1_query()
        };
        assert_eq!(shared.trace(&ghosts).unwrap(), geometry);
    }

    #[test]
    fn shared_q1_startsolid_merge() {
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        // Body around the sweep start.
        let pocket = test_actor(1);
        let (linked, collision) = body(
            pocket.clone(),
            Bounds {
                min: vec3(-60.0, -8.0, -8.0),
                max: vec3(-40.0, 8.0, 8.0),
            },
        );
        shared.link(&linked, &collision);
        // WinQuake world.c:861-864: a startsolid actor hit is taken.
        // Leaving solid is not a hit, so the fraction stays 1.0.
        let hit = shared.trace(&q1_query()).unwrap();
        assert!(hit.start_solid);
        assert_eq!(hit.fraction, 1.0);
        assert_eq!(hit.hit, TraceHit::Actor { actor: pocket.clone() });
        // A nearer actor hit later in link order keeps the startsolid
        // flag already on the trace (world.c:865-869).
        let snag = test_actor(2);
        let (linked, collision) = body(
            snag.clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        shared.link(&linked, &collision);
        let geometry = shared.geometry_trace(&q1_query()).unwrap();
        let caught = shared.trace(&q1_query()).unwrap();
        assert!(caught.start_solid);
        assert!(caught.fraction < geometry.fraction);
        assert_eq!(caught.hit, TraceHit::Actor { actor: snag.clone() });
    }

    #[test]
    fn shared_state_sources_refresh_actors() {
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        let actor = test_actor(1);
        let (linked, collision) = body(
            actor.clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        shared.link(&linked, &collision);
        let moved = BodyState {
            origin: vec3(200.0, 0.0, 0.0),
            ..linked.state.clone()
        };
        shared.bind_actor_state(ActorStateSource::Callback(Rc::new(move |_| Some(moved.clone()))));
        let miss = shared.trace(&q1_query()).unwrap();
        assert_eq!(miss.hit, TraceHit::World { model: 0 });
        let owner = qa_core::identity::IdentityOwner::create("table").unwrap();
        let mut registry = ActorRegistry::new(owner, 4).unwrap();
        let owned = registry
            .allocate(qa_core::identity::ProviderId::new("q1", "game"), "q1:soldier")
            .unwrap();
        let mut table = BodyTable::default();
        table.write(&registry, &owned, linked.state.clone()).unwrap();
        let mut shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        let (linked, collision) = body(
            owned.id().clone(),
            Bounds {
                min: vec3(-40.0, -16.0, -16.0),
                max: vec3(-8.0, 16.0, 16.0),
            },
        );
        shared.link(&linked, &collision);
        shared.bind_actor_state(ActorStateSource::Table {
            table: Rc::new(table),
            registry: Rc::new(registry),
        });
        let current = shared.linked_actor(owned.id()).unwrap();
        assert_eq!(current.body.state.bounds.min.x, -40.0);
    }

    #[test]
    fn shared_q2_portals_and_media_merge() {
        let shared = SharedSceneQueries::new(DecodedCollisionWorld::Q2(q2_geometry())).unwrap();
        assert!(!shared.areas_connected(0, 1).unwrap());
        shared.adjust_area_portal_contribution(7, 1).unwrap();
        assert!(shared.areas_connected(0, 1).unwrap());
        shared.set_area_portal_state(7, false).unwrap();
        assert!(shared.areas_connected(0, 1).unwrap());
        shared.adjust_area_portal_contribution(7, -1).unwrap();
        assert!(!shared.areas_connected(0, 1).unwrap());
        assert_eq!(
            shared
                .adjust_area_portal_contribution(7, -1)
                .expect_err("underflow")
                .to_string(),
            "Area portal contribution underflow"
        );
        let merged = shared.geometry_trace(&q1_query()).unwrap();
        let provider = Q2Collision::new(q2_geometry());
        let media = provider.trace_media(&q1_query(), merged.fraction).unwrap();
        match merged.detail {
            TraceDetail::Q1 { in_open, in_water, .. } => {
                assert_eq!(in_open, media.in_open);
                assert_eq!(in_water, media.in_water);
            }
            _ => panic!("q1 detail"),
        }
    }

    #[test]
    fn shared_family_errors_match_donor() {
        let shared = SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_geometry())).unwrap();
        assert_eq!(
            shared
                .set_area_portal_state(7, true)
                .expect_err("q1 portals")
                .to_string(),
            "Portal identifiers belong to Quake II maps"
        );
        assert_eq!(
            shared.q2_portal_state().expect_err("q1 portals").to_string(),
            "Portal identifiers belong to Quake II maps"
        );
        assert_eq!(
            shared
                .adjust_area_portal_state(0, 1, true)
                .expect_err("q1 pairs")
                .to_string(),
            "Area-pair portal references belong to Quake III maps"
        );
        assert!(shared.native_q3_clip_models().is_none());
        let tasks: &dyn SceneQueries = &shared;
        assert!(tasks.areas_connected(0, 0));
    }
}
