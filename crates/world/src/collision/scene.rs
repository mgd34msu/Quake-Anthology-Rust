use super::EntityTraceFlags;
use super::{
    CollisionStore, Contents, EntityTracePolicy, QuakeTraceKind, Trace, TraceQuery, TraceScratch,
    WorldEntityRule, boxes::trace_box,
};
use crate::{area::AreaGrid, entities::EntityTable};
use qa_core::primitives::{
    Body, Bounds, CollisionOwner, CollisionShape, CollisionTags, EntityId, EntityPose, GeometryId,
    NativeEntity, Vec3,
};

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
        }
    }

    pub fn trace(&mut self, mut query: TraceQuery) -> Trace {
        query.pass = query.pass.or(self.pass);
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
        let pass_slot = query.pass.and_then(|id| self.entities.resolve(id));
        let role = query.entity_rules.link_role;
        for linked in self.area.query(self.entities, bounds, role) {
            if result.all_solid {
                break;
            }
            let id = linked.id;
            let slot = id.slot as usize;
            if query.pass == Some(id) || query.excluded.contains(&id) {
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
            if query
                .entity_rules
                .filtering
                .contains(EntityTraceFlags::DEAD_MONSTER_MASK)
                && columns.collision_tags[slot].0 & CollisionTags::DEAD_MONSTER.0 != 0
                && !query.mask.intersects(Contents::CORPSE)
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
            if let Some(pass) = pass_slot {
                let pass_id = query.pass;
                if self.owner_is(columns.collision_owner[slot], pass_id) {
                    continue;
                }
                if query
                    .entity_rules
                    .filtering
                    .contains(EntityTraceFlags::SHARED_OWNER)
                {
                    let pass_owner = self.arena_pass_owner(pass);
                    if self.same_owner(columns.collision_owner[slot], pass_owner) {
                        continue;
                    }
                } else if self.owner_is(columns.collision_owner[pass], Some(id)) {
                    continue;
                }
            }
            let target_query =
                if missile && columns.collision_tags[slot].0 & CollisionTags::MONSTER.0 != 0 {
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

    fn owner_is(&self, owner: CollisionOwner, target: Option<EntityId>) -> bool {
        let Some(target) = target else { return false };
        match owner {
            CollisionOwner::None => false,
            CollisionOwner::Lifetime(id) => id == target,
            CollisionOwner::Native(native) => {
                self.entities.columns.native_entity[target.slot as usize] == Some(native)
            }
        }
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

    fn arena_pass_owner(&self, pass: usize) -> CollisionOwner {
        let columns = &self.entities.columns;
        match columns.collision_owner[pass] {
            CollisionOwner::Native(NativeEntity { module, slot: 1023 }) => {
                CollisionOwner::Native(NativeEntity { module, slot: -1 })
            }
            CollisionOwner::None => CollisionOwner::Native(NativeEntity {
                module: columns.native_entity[pass]
                    .map_or(columns.owner[pass], |native| native.module),
                slot: -1,
            }),
            owner => owner,
        }
    }
}
