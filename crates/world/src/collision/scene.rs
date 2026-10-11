use super::EntityTraceFlags;
use super::{
    CollisionStore, Contents, EntityTracePolicy, QuakeTraceKind, Trace, TraceQuery, TraceScratch,
    WorldEntityRule, boxes::trace_box,
};
use crate::{area::AreaGrid, entities::EntityTable};
use qa_core::primitives::{
    Body, Bounds, CollisionOwner, CollisionShape, CollisionTags, EntityId, EntityPose, GeometryId,
    ModuleId, NativeEntity, Vec3,
};

/// A stopped module's current ownership/classification fields. Geometry and
/// narrow-phase clipping remain in this shared trace implementation.
pub trait NativeTraceEntities {
    fn module(&self) -> ModuleId;
    fn fields(&self, slot: i32) -> Option<(CollisionOwner, CollisionTags)>;
}

/// A frozen world and its directly borrowed hot entity columns. A caller's
/// scratch and pass entity are independent of every other trace caller.
pub struct WorldTrace<'a> {
    store: &'a CollisionStore,
    geometry: GeometryId,
    model: u32,
    entities: &'a EntityTable,
    area: &'a AreaGrid,
    scratch: &'a mut TraceScratch,
    pass: Option<EntityId>,
    native_entities: Option<(ModuleId, &'a dyn NativeTraceEntities)>,
}

impl<'a> WorldTrace<'a> {
    pub fn new(
        store: &'a CollisionStore,
        geometry: GeometryId,
        model: u32,
        entities: &'a EntityTable,
        area: &'a AreaGrid,
        scratch: &'a mut TraceScratch,
        pass: Option<EntityId>,
    ) -> Self {
        Self {
            store,
            geometry,
            model,
            entities,
            area,
            scratch,
            pass,
            native_entities: None,
        }
    }

    pub fn with_native_entities(mut self, view: Option<&'a dyn NativeTraceEntities>) -> Self {
        self.native_entities = view.map(|view| (view.module(), view));
        self
    }

    pub fn trace(&mut self, mut query: TraceQuery) -> Trace {
        if query.pass == CollisionOwner::None {
            query.pass = self
                .pass
                .map_or(CollisionOwner::None, CollisionOwner::Lifetime);
        }
        if let CollisionOwner::Lifetime(id) = query.pass
            && self.entities.resolve(id).is_none()
        {
            query.pass = CollisionOwner::None;
        }
        let mut result = self
            .store
            .trace_model(self.geometry, self.model, query, self.scratch);
        let world_entity = match query.entity_rules.world_entity {
            WorldEntityRule::HitOrStartSolid => result.fraction < 1.0 || result.start_solid,
            WorldEntityRule::Always => true,
            WorldEntityRule::FractionChanged => result.fraction != 1.0,
        };
        if world_entity {
            result.entity = self.entities.id_at(0);
        }
        if result.fraction == 0.0 && query.entity_rules.stop_at_zero() {
            return result;
        }
        let missile = query.entity_rules.quake_kind() == Some(QuakeTraceKind::Missile);
        let (mins, maxs) = if missile {
            (Vec3([-15.0; 3]), Vec3([15.0; 3]))
        } else {
            (query.mins, query.maxs)
        };
        let bounds = Bounds {
            mins: Vec3(std::array::from_fn(|axis| {
                query.start.0[axis].min(query.end.0[axis]) + mins.0[axis] - 1.0
            })),
            maxs: Vec3(std::array::from_fn(|axis| {
                query.start.0[axis].max(query.end.0[axis]) + maxs.0[axis] + 1.0
            })),
        };
        let columns = &self.entities.columns;
        let pass_slot = match query.pass {
            CollisionOwner::Lifetime(id) => self.entities.resolve(id),
            _ => None,
        };
        let pass_owner = self
            .fields(query.pass)
            .map_or(CollisionOwner::None, |fields| fields.0);
        let role = query.entity_rules.link_role;
        for linked in self.area.query(self.entities, bounds, role) {
            if result.all_solid {
                break;
            }
            let id = linked.id;
            let slot = id.slot as usize;
            if self.same_owner(query.pass, CollisionOwner::Lifetime(id))
                || query.excluded.contains(&id)
            {
                continue;
            }
            let shape = columns.collision_shape[slot];
            if shape == CollisionShape::None {
                continue;
            }
            if query.entity_rules.quake_kind() == Some(QuakeTraceKind::IgnoreBoxes)
                && shape == CollisionShape::Box
            {
                continue;
            }
            if query
                .entity_rules
                .filtering
                .contains(EntityTraceFlags::SKIP_POINT_ENTITIES)
                && let Some(pass) = pass_slot
                && columns.maxs[pass].0[0] - columns.mins[pass].0[0] != 0.0
                && linked.maxs.0[0] - linked.mins.0[0] == 0.0
            {
                continue;
            }
            let Some((owner, tags)) = self.fields(CollisionOwner::Lifetime(id)) else {
                continue;
            };
            if query
                .entity_rules
                .filtering
                .contains(EntityTraceFlags::DEAD_MONSTER_MASK)
                && tags.0 & CollisionTags::DEAD_MONSTER.0 != 0
                && !query.mask.intersects(Contents::CORPSE)
            {
                continue;
            }
            if (query
                .entity_rules
                .filtering
                .contains(EntityTraceFlags::PROJECTILE_MASK)
                && tags.0 & CollisionTags::PROJECTILE.0 != 0
                && !query.mask.intersects(Contents::PROJECTILE))
                || (query
                    .entity_rules
                    .filtering
                    .contains(EntityTraceFlags::PLAYER_MASK)
                    && tags.0 & CollisionTags::PLAYER.0 != 0
                    && !query.mask.intersects(Contents::PLAYER))
            {
                continue;
            }
            if query
                .entity_rules
                .filtering
                .contains(EntityTraceFlags::CONTENTS_MASK)
                && !query
                    .mask
                    .intersects(Contents(columns.collision_contents[slot]))
            {
                continue;
            }
            if query.pass != CollisionOwner::None {
                if self.same_owner(owner, query.pass) {
                    continue;
                }
                if query
                    .entity_rules
                    .filtering
                    .contains(EntityTraceFlags::SHARED_OWNER)
                {
                    let pass_owner = self.arena_pass_owner(query.pass, pass_owner);
                    if self.same_owner(owner, pass_owner) {
                        continue;
                    }
                } else if self.same_owner(pass_owner, CollisionOwner::Lifetime(id)) {
                    continue;
                }
            }
            let target_query = if missile && tags.0 & CollisionTags::MONSTER.0 != 0 {
                TraceQuery {
                    mins,
                    maxs,
                    ..query
                }
            } else {
                query
            };
            let incoming = match shape {
                CollisionShape::Box => trace_box(
                    target_query,
                    &Body {
                        position: *linked.position,
                        velocity: *linked.velocity,
                        mins: *linked.mins,
                        maxs: *linked.maxs,
                    },
                    id,
                ),
                CollisionShape::Model { geometry, index } => {
                    // Stale lifetimes and invalid inline ordinals affect only
                    // this candidate, never another geometry in the store.
                    if self.store.model_bounds(geometry, index).is_none() {
                        continue;
                    }
                    let mut incoming = self.store.trace_transformed(
                        geometry,
                        index,
                        target_query,
                        *linked.position,
                        columns.angles[slot],
                        columns.model_rules[slot],
                        self.scratch,
                    );
                    if incoming.fraction < 1.0 || incoming.start_solid || incoming.all_solid {
                        incoming.entity = Some(id);
                    }
                    incoming.brush_solid = true;
                    incoming
                }
                CollisionShape::None => continue,
            };
            result.merge_linked(incoming, query.entity_rules);
        }
        result
    }

    pub fn point_contents(
        &self,
        point: Vec3,
        rules: EntityTracePolicy,
        excluded: &[EntityId],
    ) -> Contents {
        let mut result = self
            .store
            .point_contents_model(self.geometry, self.model, point, rules);
        // Q1 SV_PointContents is the world hull, without linked-body contents.
        if !rules.filtering.contains(EntityTraceFlags::LINKED_CONTENTS) {
            return result;
        }
        let role = rules.link_role;
        for linked in self.area.query(
            self.entities,
            Bounds {
                mins: point,
                maxs: point,
            },
            role,
        ) {
            if rules
                .filtering
                .contains(EntityTraceFlags::EXCLUDE_CONTENTS_PASS)
                && Some(linked.id) == self.pass
                || excluded.contains(&linked.id)
            {
                continue;
            }
            let slot = linked.id.slot as usize;
            let pose = self.entities.columns.point_contents_pose[slot].unwrap_or(EntityPose {
                position: *linked.position,
                angles: self.entities.columns.angles[slot],
            });
            match self.entities.columns.collision_shape[slot] {
                CollisionShape::None => continue,
                CollisionShape::Model { geometry, index } => {
                    result |= self.store.point_contents_transformed(
                        geometry,
                        index,
                        point,
                        pose.position,
                        pose.angles,
                        self.entities.columns.model_rules[slot],
                        rules,
                    );
                    continue;
                }
                CollisionShape::Box => {}
            }
            // CM_TransformedPointContents temporary-box contents are the native
            // BODY/MONSTER brush, independently of an entity's content filter.
            let inside = (0..3).all(|axis| {
                let coordinate = point.0[axis] - pose.position.0[axis];
                coordinate >= linked.mins.0[axis]
                    && if !rules
                        .filtering
                        .contains(EntityTraceFlags::INCLUSIVE_CONTENTS_MAX)
                    {
                        coordinate < linked.maxs.0[axis]
                    } else {
                        coordinate <= linked.maxs.0[axis]
                    }
            });
            if inside {
                result |= Contents::BODY;
            }
        }
        result
    }

    fn fields(&self, entity: CollisionOwner) -> Option<(CollisionOwner, CollisionTags)> {
        let (native, cached) = match entity {
            CollisionOwner::None => return Some((CollisionOwner::None, CollisionTags::default())),
            CollisionOwner::Native(native) => (
                Some(native),
                (CollisionOwner::None, CollisionTags::default()),
            ),
            CollisionOwner::Lifetime(id) => {
                let slot = self.entities.resolve(id)?;
                let columns = &self.entities.columns;
                (
                    columns.native_entity[slot],
                    (columns.collision_owner[slot], columns.collision_tags[slot]),
                )
            }
        };
        if let Some((module, view)) = self.native_entities
            && let Some(native) = native
            && native.module == module
        {
            return view.fields(native.slot);
        }
        Some(cached)
    }

    fn native_owner(&self, owner: CollisionOwner) -> Option<NativeEntity> {
        match owner {
            CollisionOwner::None => None,
            CollisionOwner::Native(native) => Some(native),
            CollisionOwner::Lifetime(id) => self
                .entities
                .resolve(id)
                .and_then(|slot| self.entities.columns.native_entity[slot]),
        }
    }

    fn same_owner(&self, left: CollisionOwner, right: CollisionOwner) -> bool {
        match (left, right) {
            (CollisionOwner::Lifetime(left), CollisionOwner::Lifetime(right)) if left == right => {
                self.entities.resolve(left).is_some()
            }
            _ => self
                .native_owner(left)
                .is_some_and(|left| Some(left) == self.native_owner(right)),
        }
    }

    fn arena_pass_owner(&self, pass: CollisionOwner, owner: CollisionOwner) -> CollisionOwner {
        match owner {
            CollisionOwner::Native(NativeEntity { module, slot: 1023 }) => {
                CollisionOwner::Native(NativeEntity { module, slot: -1 })
            }
            CollisionOwner::None => CollisionOwner::Native(NativeEntity {
                module: self.native_owner(pass).map_or_else(
                    || match pass {
                        CollisionOwner::Lifetime(id) => {
                            self.entities.columns.owner[id.slot as usize]
                        }
                        _ => ModuleId::default(),
                    },
                    |native| native.module,
                ),
                slot: -1,
            }),
            owner => owner,
        }
    }
}
