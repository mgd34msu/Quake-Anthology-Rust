//! Quake III base: world adapter.
//!
//! Donor provenance: `src/content/q3/base/world-adapter.ts`.

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::numeric::{NumericProfile, Q3_BINARY32_PROFILE};
use qa_world::body::{BodyState, LinkedBody};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::records::*;
use crate::q3::base::shared::entity_shared::*;
use crate::q3::base::shared::player_state::*;
use crate::q3::base::world::*;

// ---------------------------------------------------------------------------
// world-adapter.ts
// ---------------------------------------------------------------------------

/// Collision shape word (`ActorCollision` shape layer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorCollisionShape {
    /// Inline model.
    InlineModel {
        /// Model index.
        model: i32,
    },
    /// Bounding box.
    Box,
    /// Capsule.
    Capsule,
}

/// Actor collision record (`ActorCollision`, minimal).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorCollision {
    /// Shape.
    pub shape: ActorCollisionShape,
    /// Contents mask.
    pub contents: i32,
    /// Owner actor.
    pub owner: Option<ActorId>,
    /// Collision role.
    pub role: ActorCollisionRole,
    /// Monster.
    pub monster: bool,
    /// Dead monster.
    pub dead_monster: bool,
}

/// Actor collision role word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorCollisionRole {
    /// Trigger.
    Trigger,
    /// Solid.
    Solid,
}

/// Q3 trace query over the shared scene (`TraceQuery`, Q3 layer).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3TraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Actor to pass through.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
    /// Curve collision.
    pub curves: bool,
    /// Player curve clipping.
    pub player_curve_clip: bool,
    /// Numeric profile.
    pub numeric: NumericProfile,
}

/// Q3 trace hit word.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3TraceHit {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Actor hit.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// Q3 trace result (`TraceResult`, Q3 layer).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3TraceResult {
    /// Fraction traveled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit record.
    pub hit: Q3TraceHit,
    /// Contact.
    pub contact: TraceContact,
    /// Started solid.
    pub start_solid: bool,
    /// Entirely solid.
    pub all_solid: bool,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

/// World adapter host services (`Q3WorldAdapterHost`).
pub trait Q3WorldAdapterHost {
    /// Trace the shared scene.
    fn trace_scene(&self, query: &Q3TraceQuery) -> Q3TraceResult;
    /// Contents at a point.
    fn point_contents_scene(&self, query: &Q3TraceQuery, point: Vec3) -> i32;
    /// Actors overlapping bounds.
    fn query_actors(&self, bounds: Bounds) -> Vec<ActorId>;
    /// Spatial collision record.
    fn spatial_collision(&self, actor: &ActorId) -> Option<ActorCollision>;
    /// Body state.
    fn body_state(&self, actor: &ActorId) -> Option<BodyState>;
    /// Linked body.
    fn linked_body(&self, actor: &ActorId) -> Option<LinkedBody>;
    /// Update collision metadata before the link hook publishes it.
    fn set_collision(&self, actor: &OwnedActor, collision: ActorCollision);
    /// Link a body, optionally at a snapped origin.
    fn link_body(&self, actor: &OwnedActor, origin: Option<Vec3>);
    /// Unlink a body.
    fn unlink_body(&self, actor: &OwnedActor);
    /// Curve collision.
    fn curves(&self) -> bool;
    /// Player curve clipping.
    fn player_curve_clip(&self) -> bool;
    /// Model geometry trace start-solid word.
    fn geometry_trace_start_solid(&self, query: &Q3TraceQuery, model: i32, origin: Vec3, angles: Vec3) -> bool;
    /// Actor body trace start-solid word (`traceActorBody` layer).
    fn body_trace_start_solid(&self, query: &Q3TraceQuery, body: &BodyState, collision: &ActorCollision) -> bool;
}

/// Q3 source query words over the shared geometry and actor index
/// (`Q3WorldAdapter`).
#[derive(Clone)]
pub struct Q3WorldAdapter {
    host: Rc<dyn Q3WorldAdapterHost>,
    records: Q3EntityRecords,
}

impl std::fmt::Debug for Q3WorldAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3WorldAdapter").finish()
    }
}

impl Q3WorldAdapter {
    /// Adapter over a host and records.
    #[must_use]
    pub fn new(host: Rc<dyn Q3WorldAdapterHost>, records: Q3EntityRecords) -> Self {
        Self { host, records }
    }

    fn actor(&self, number: i32) -> Option<ActorId> {
        if number == ENTITYNUM_WORLD || number == ENTITYNUM_NONE || number < 0 {
            return None;
        }
        let entity = usize::try_from(number).ok().and_then(|slot| self.records.get(slot))?;
        if entity.borrow().inuse() {
            Some(entity.borrow().actor().id().clone())
        } else {
            None
        }
    }

    fn query(&self, input: &ActorTraceQuery) -> Q3TraceQuery {
        Q3TraceQuery {
            start: input.start,
            end: input.end,
            shape: input.shape,
            pass_actor: input.pass_actor.clone(),
            mask: input.mask,
            curves: self.host.curves(),
            player_curve_clip: self.host.player_curve_clip(),
            numeric: Q3_BINARY32_PROFILE,
        }
    }

    /// Unlink an actor, returning a restore callback.
    #[must_use]
    pub fn unlink_actor(&self, actor: &ActorId) -> Option<Box<dyn Fn()>> {
        let owner = self.records.host().actors().resolve_owned(actor)?;
        self.host.linked_body(actor)?;
        let native = self.records.native_by_actor(Some(actor));
        if let Some(native) = &native {
            native.borrow_mut().r.capture_link();
        }
        self.host.unlink_body(&owner);
        let adapter = self.clone();
        let records = self.records.clone();
        let host = self.host.clone();
        Some(Box::new(move || {
            if !records.host().actors().is_live(owner.id()) || records.host().bodies().read(owner.id()).is_none() {
                return;
            }
            if native.as_ref().is_some_and(|native| {
                records
                    .native_by_actor(Some(owner.id()))
                    .as_ref()
                    .is_some_and(|current| Rc::ptr_eq(current, native))
            }) {
                adapter.link(
                    native
                        .clone()
                        .unwrap_or_else(|| panic!("Q3 restore lost its native entity")),
                );
            } else {
                host.link_body(&owner, None);
            }
        }))
    }
}

impl ServerWorld for Q3WorldAdapter {
    fn entity_contact(&self, bounds: Bounds, entity_num: i32, capsule: bool) -> bool {
        let Some(entity) = usize::try_from(entity_num).ok().and_then(|slot| self.records.get(slot)) else {
            return false;
        };
        if !entity.borrow().inuse() {
            return false;
        }
        let actor = entity.borrow().actor().id().clone();
        self.contact_actor(bounds, &actor, capsule)
    }

    fn trace(&self, query: &ServerTraceQuery) -> ServerTraceResult {
        let actor_query = ActorTraceQuery {
            start: query.start,
            end: query.end,
            shape: query.shape,
            pass_actor: self.actor(query.pass_entity_num),
            mask: query.mask,
        };
        let result = self.trace_actor(&actor_query);
        let entity_num = match &result.hit {
            ActorTraceHit::None => ENTITYNUM_NONE,
            ActorTraceHit::World => ENTITYNUM_WORLD,
            ActorTraceHit::Actor { actor } => {
                let Some(entity) = self.records.by_actor(Some(actor)) else {
                    panic!("Shared collision actor has no Q3 source projection");
                };
                let slot = entity.borrow().slot;
                slot as i32
            }
        };
        ServerTraceResult {
            fraction: result.fraction,
            end: result.end,
            entity_num,
            solidity: result.solidity,
            contact: result.contact,
            contents: result.contents,
            surface_flags: result.surface_flags,
        }
    }

    fn area_entities(&self, bounds: Bounds, maximum: usize) -> Vec<i32> {
        let mut output = Vec::new();
        for actor in self.area_actors(bounds, maximum) {
            let Some(entity) = self.records.by_actor(Some(&actor)) else {
                panic!("Shared spatial actor has no Q3 source projection");
            };
            output.push(entity.borrow().slot as i32);
        }
        output
    }

    fn point_contents(&self, point: Vec3, pass_entity_num: i32) -> i32 {
        let query = self.query(&ActorTraceQuery {
            start: point,
            end: point,
            shape: TraceShape::Point,
            pass_actor: self.actor(pass_entity_num),
            mask: -1,
        });
        self.host.point_contents_scene(&query, point)
    }

    fn link_state(&self, number: i32) -> Option<LinkState> {
        let actor = self.actor(number);
        let linked = actor.as_ref().and_then(|actor| self.host.linked_body(actor))?;
        Some(LinkState {
            absbounds: linked.absolute_bounds,
            linked: true,
            linkcount: linked.link_count as i32,
        })
    }

    fn link(&self, entity: EntityRef) {
        // NaN truncates through the source's float clamps to a zero
        // bitwise byte.
        let byte = |value: f32| -> i32 {
            if value.is_nan() {
                0
            } else {
                (value.trunc() as i32).clamp(1, 255)
            }
        };
        let (actor, model, contents, mins, maxs, owner_num, current_origin) = {
            let borrowed = entity.borrow();
            (
                borrowed.actor(),
                borrowed.r.model,
                borrowed.r.contents,
                borrowed.r.mins(),
                borrowed.r.maxs(),
                borrowed.r.owner_num,
                borrowed.r.current_origin(),
            )
        };
        let solid = match model {
            EntityCollisionModel::Inline { .. } => 0xffffff,
            EntityCollisionModel::Box | EntityCollisionModel::Capsule => {
                if contents & (1 | 0x2000000) == 0 {
                    0
                } else {
                    (byte(maxs.z + 32.0) << 16) | (byte(-mins.z) << 8) | byte(maxs.x)
                }
            }
        };
        {
            let mut borrowed = entity.borrow_mut();
            borrowed.r.clear_bounds_overrides();
            borrowed.s.solid = solid;
        }
        let shape = match model {
            EntityCollisionModel::Inline { index } => ActorCollisionShape::InlineModel { model: index },
            EntityCollisionModel::Box => ActorCollisionShape::Box,
            EntityCollisionModel::Capsule => ActorCollisionShape::Capsule,
        };
        self.host.set_collision(
            &actor,
            ActorCollision {
                shape,
                contents,
                owner: self.actor(owner_num),
                role: if contents == 0x40000000 {
                    ActorCollisionRole::Trigger
                } else {
                    ActorCollisionRole::Solid
                },
                monster: false,
                dead_monster: false,
            },
        );
        self.host.link_body(&actor, Some(current_origin));
        entity.borrow_mut().r.capture_link();
    }

    fn unlink(&self, number: i32) {
        let entity = usize::try_from(number).ok().and_then(|slot| self.records.get(slot));
        if let Some(entity) = entity {
            if entity.borrow().inuse() {
                let actor = entity.borrow().actor();
                entity.borrow_mut().r.capture_link();
                self.host.unlink_body(&actor);
            }
        }
    }
}

impl ActorSpatialQueries for Q3WorldAdapter {
    fn area_actors(&self, bounds: Bounds, maximum: usize) -> Vec<ActorId> {
        self.host.query_actors(bounds).into_iter().take(maximum).collect()
    }

    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult {
        let result = self.host.trace_scene(&self.query(query));
        ActorTraceResult {
            fraction: result.fraction,
            end: result.end,
            hit: match result.hit {
                Q3TraceHit::None => ActorTraceHit::None,
                Q3TraceHit::World => ActorTraceHit::World,
                Q3TraceHit::Actor { actor } => ActorTraceHit::Actor { actor },
            },
            contact: result.contact,
            solidity: if result.all_solid {
                TraceSolidity::AllSolid
            } else if result.start_solid {
                TraceSolidity::StartSolid
            } else {
                TraceSolidity::Clear
            },
            contents: result.contents,
            surface_flags: result.surface_flags,
        }
    }

    fn contact_actor(&self, bounds: Bounds, actor: &ActorId, capsule: bool) -> bool {
        let (Some(spatial), Some(body)) = (self.host.spatial_collision(actor), self.host.body_state(actor)) else {
            return false;
        };
        let origin = vec3(0.0, 0.0, 0.0);
        let shape = if capsule {
            TraceShape::Capsule {
                mins: bounds.min,
                maxs: bounds.max,
            }
        } else {
            TraceShape::Box {
                mins: bounds.min,
                maxs: bounds.max,
            }
        };
        let query = self.query(&ActorTraceQuery {
            start: origin,
            end: origin,
            shape,
            pass_actor: None,
            mask: -1,
        });
        if let ActorCollisionShape::InlineModel { model } = spatial.shape {
            return self
                .host
                .geometry_trace_start_solid(&query, model, body.origin, body.angles);
        }
        self.host.body_trace_start_solid(&query, &body, &spatial)
    }
}
