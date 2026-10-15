use super::EntityTraceFlags;
use super::store::BodyTraceMode;
use super::{
    CollisionStore, Contents, EntityTracePolicy, QuakeTraceKind, Trace, TraceError, TraceQuery,
    TraceScope, TraceScratch, WorldEntityRule,
};
use crate::{area::AreaGrid, entities::EntityTable};
use qa_core::primitives::{
    Body, Bounds, CollisionOwner, CollisionShape, CollisionTags, EntityId, EntityPose, GeometryId,
    ModelRules, ModuleId, NativeEntity, Vec3,
};

#[derive(Clone, Copy, Default)]
pub struct NativeTraceFields {
    pub tags: CollisionTags,
    pub enabled: bool,
    pub box_target: bool,
    pub contents: Contents,
}

#[derive(Clone, Copy)]
pub struct NativeTraceTarget {
    pub body: Body,
    pub angles: Vec3,
    pub shape: CollisionShape,
    pub rules: ModelRules,
    pub contents_pose: Option<EntityPose>,
}

/// The existing module configstring storage lends names and revisions. This
/// interface neither owns resources nor caches geometry.
pub trait NativeModelNames {
    fn model_name(&self, module: ModuleId, index: u32) -> Option<(&[u8], u64)>;
}

/// A stopped module's current ownership/classification fields. Geometry and
/// narrow-phase clipping remain in this shared trace implementation.
pub trait NativeTraceEntities {
    fn module(&self) -> ModuleId;
    fn owner(&self, slot: i32) -> Option<CollisionOwner>;
    fn fields(&self, slot: i32, point_contents: bool) -> Option<NativeTraceFields>;
    fn bounds(&self, slot: i32, cached: Bounds) -> Option<Bounds>;
    fn target(
        &self,
        slot: i32,
        cached: NativeTraceTarget,
        point_contents: bool,
        store: &CollisionStore,
        geometry: GeometryId,
        models: Option<&dyn NativeModelNames>,
    ) -> Option<NativeTraceTarget>;
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
    pass: CollisionOwner,
    native_entities: Option<(ModuleId, &'a dyn NativeTraceEntities)>,
    native_models: Option<&'a dyn NativeModelNames>,
}

impl<'a> WorldTrace<'a> {
    pub fn new(
        store: &'a CollisionStore,
        geometry: GeometryId,
        model: u32,
        entities: &'a EntityTable,
        area: &'a AreaGrid,
        scratch: &'a mut TraceScratch,
        pass: CollisionOwner,
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
            native_models: None,
        }
    }

    pub fn with_pass(mut self, pass: CollisionOwner) -> Self {
        self.pass = pass;
        self
    }

    pub fn with_native_entities(mut self, view: Option<&'a dyn NativeTraceEntities>) -> Self {
        self.native_entities = view.map(|view| (view.module(), view));
        self
    }

    pub fn with_native_models(mut self, models: Option<&'a dyn NativeModelNames>) -> Self {
        self.native_models = models;
        self
    }

    pub fn trace(&mut self, mut query: TraceQuery) -> Result<Trace, TraceError> {
        if query.pass == CollisionOwner::None {
            query.pass = self.pass;
        }
        if let CollisionOwner::Lifetime(id) = query.pass
            && self.entities.resolve(id).is_none()
        {
            query.pass = CollisionOwner::None;
        }
        let mut result = self
            .store
            .trace_model(self.geometry, self.model, query, self.scratch)?;
        let world_entity = match query.entity_rules.world_entity {
            WorldEntityRule::HitOrStartSolid => result.fraction < 1.0 || result.start_solid,
            WorldEntityRule::Always => true,
            WorldEntityRule::FractionChanged => result.fraction != 1.0,
        };
        if world_entity {
            result.entity = self.entities.id_at(0);
        }
        if query.scope == TraceScope::World
            || result.fraction == 0.0 && query.entity_rules.stop_at_zero()
        {
            return Ok(result);
        }
        let missile = query.entity_rules.quake_kind() == Some(QuakeTraceKind::Missile);
        let (mins, maxs) = if missile {
            (Vec3([-15.0; 3]), Vec3([15.0; 3]))
        } else {
            (query.mins, query.maxs)
        };
        let bounds = TraceQuery {
            mins,
            maxs,
            ..query
        }
        .swept_bounds(1.0);
        let pass_owner = self.owner(query.pass).unwrap_or(CollisionOwner::None);
        let role = query.entity_rules.link_role;
        for linked in self.area.query(self.entities, bounds, role) {
            if result.all_solid {
                break;
            }
            let id = linked.id;
            if self.same_owner(query.pass, CollisionOwner::Lifetime(id))
                || query.excluded.contains(&id)
            {
                continue;
            }
            let Some(fields) = self.fields(CollisionOwner::Lifetime(id), false) else {
                continue;
            };
            if !fields.enabled {
                continue;
            }
            if query.entity_rules.quake_kind() == Some(QuakeTraceKind::IgnoreBoxes)
                && fields.box_target
            {
                continue;
            }
            if query
                .entity_rules
                .filtering
                .contains(EntityTraceFlags::SKIP_POINT_ENTITIES)
                && self
                    .owner_bounds(query.pass)
                    .is_some_and(|bounds| bounds.maxs.0[0] - bounds.mins.0[0] != 0.0)
                && self
                    .bounds(id)
                    .is_some_and(|bounds| bounds.maxs.0[0] - bounds.mins.0[0] == 0.0)
            {
                continue;
            }
            let tags = fields.tags;
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
                && !query.mask.intersects(fields.contents)
            {
                continue;
            }
            if query.pass != CollisionOwner::None {
                let Some(owner) = self.owner(CollisionOwner::Lifetime(id)) else {
                    continue;
                };
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
            let Some(target) = self.target(id, false) else {
                continue;
            };
            let incoming = self.store.trace_body(
                target_query,
                target.shape,
                &target.body,
                target.angles,
                target.rules,
                BodyTraceMode::Transformed,
                Some(id),
                self.scratch,
            )?;
            result.merge_linked(incoming, query.entity_rules);
        }
        Ok(result)
    }

    pub fn point_contents(
        &self,
        point: Vec3,
        rules: EntityTracePolicy,
        excluded: &[EntityId],
    ) -> Result<Contents, TraceError> {
        let mut result =
            self.store
                .point_contents_model(self.geometry, self.model, point, rules)?;
        // Q1 SV_PointContents is the world hull, without linked-body contents.
        if !rules.filtering.contains(EntityTraceFlags::LINKED_CONTENTS) {
            return Ok(result);
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
                && self.same_owner(CollisionOwner::Lifetime(linked.id), self.pass)
                || excluded.contains(&linked.id)
            {
                continue;
            }
            if !self
                .fields(CollisionOwner::Lifetime(linked.id), true)
                .is_some_and(|fields| fields.enabled)
            {
                continue;
            }
            let Some(target) = self.target(linked.id, true) else {
                continue;
            };
            let pose = target.contents_pose.unwrap_or(EntityPose {
                position: target.body.position,
                angles: target.angles,
            });
            let mut body = target.body;
            body.position = pose.position;
            result |= self.store.point_contents_body(
                point,
                target.shape,
                &body,
                pose.angles,
                target.rules,
                rules,
                BodyTraceMode::Transformed,
            )?;
        }
        Ok(result)
    }

    fn owner(&self, entity: CollisionOwner) -> Option<CollisionOwner> {
        let (native, cached) = match entity {
            CollisionOwner::None => return None,
            CollisionOwner::Native(native) => (Some(native), CollisionOwner::None),
            CollisionOwner::Lifetime(id) => {
                let slot = self.entities.resolve(id)?;
                (
                    self.entities.columns.native_entity[slot],
                    self.entities.columns.collision_owner[slot],
                )
            }
        };
        if let Some((module, view)) = self.native_entities
            && let Some(native) = native
            && native.module == module
        {
            return view.owner(native.slot);
        }
        Some(cached)
    }

    fn fields(&self, entity: CollisionOwner, point_contents: bool) -> Option<NativeTraceFields> {
        let (native, cached) = match entity {
            CollisionOwner::None => return None,
            CollisionOwner::Native(native) => (
                Some(native),
                NativeTraceFields {
                    tags: CollisionTags::default(),
                    enabled: false,
                    box_target: false,
                    contents: Contents::EMPTY,
                },
            ),
            CollisionOwner::Lifetime(id) => {
                let slot = self.entities.resolve(id)?;
                let columns = &self.entities.columns;
                (
                    columns.native_entity[slot],
                    NativeTraceFields {
                        tags: columns.collision_tags[slot],
                        enabled: columns.collision_shape[slot] != CollisionShape::None,
                        box_target: matches!(
                            columns.collision_shape[slot],
                            CollisionShape::Box | CollisionShape::Capsule
                        ),
                        contents: Contents(columns.collision_contents[slot]),
                    },
                )
            }
        };
        if let Some((module, view)) = self.native_entities
            && let Some(native) = native
            && native.module == module
        {
            return view.fields(native.slot, point_contents);
        }
        Some(cached)
    }

    fn native(&self, id: EntityId) -> Option<(NativeEntity, &'a dyn NativeTraceEntities)> {
        let slot = self.entities.resolve(id)?;
        let native = self.entities.columns.native_entity[slot]?;
        let (module, view) = self.native_entities?;
        (native.module == module).then_some((native, view))
    }

    fn owner_bounds(&self, owner: CollisionOwner) -> Option<Bounds> {
        match owner {
            CollisionOwner::None => None,
            CollisionOwner::Lifetime(id) => self.bounds(id),
            CollisionOwner::Native(native) => {
                let (module, view) = self.native_entities?;
                (native.module == module)
                    .then(|| view.bounds(native.slot, Bounds::default()))
                    .flatten()
            }
        }
    }

    fn bounds(&self, id: EntityId) -> Option<Bounds> {
        let slot = self.entities.resolve(id)?;
        let cached = Bounds {
            mins: self.entities.columns.mins[slot],
            maxs: self.entities.columns.maxs[slot],
        };
        if let Some((native, view)) = self.native(id) {
            view.bounds(native.slot, cached)
        } else {
            Some(cached)
        }
    }

    fn target(&self, id: EntityId, point_contents: bool) -> Option<NativeTraceTarget> {
        let slot = self.entities.resolve(id)?;
        let columns = &self.entities.columns;
        let cached = NativeTraceTarget {
            body: columns.body(slot),
            angles: columns.angles[slot],
            shape: columns.collision_shape[slot],
            rules: columns.model_rules[slot],
            contents_pose: columns.point_contents_pose[slot],
        };
        if let Some((native, view)) = self.native(id) {
            view.target(
                native.slot,
                cached,
                point_contents,
                self.store,
                self.geometry,
                self.native_models,
            )
        } else {
            Some(cached)
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
