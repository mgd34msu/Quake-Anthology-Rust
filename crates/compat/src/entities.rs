//! Native entity-address projection onto common lifetime handles.
use crate::{
    memory::{ModuleMemory, Width},
    services::{CallContext, CallError, ClipTarget, ENGINE_CALLS, EngineServices},
};
use qa_core::primitives::{
    Body, Bounds, CollisionOwner, CollisionShape, CollisionTags, EntityAppearance, EntityId,
    ModelRef, ModelRules, ModuleId, NativeAppearanceTail, NativeEntity, ResourceNamespace,
    RuleSetId, ThinkTime, Vec3,
};
use qa_world::area::LinkFlags;
use qa_world::collision::Contents;
use qa_world::entities::{EntityTable, MAX_ENTITIES};

mod quakec;
pub(crate) use quakec::QuakeCLayout;

/// Published ABI data is read only while the owned child is stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entities {
    pub address: u64,
    pub stride: u64,
    pub count: u32,
    pub capacity: u32,
    pub server_flags: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct EntityLayout {
    pub source: RuleSetId,
    pub bytes: usize,
    pub in_use: (usize, bool),
    pub linked: Option<usize>,
    pub link_count: usize,
    pub flags: usize,
    pub player_flag: u32,
    pub projectile_flag: u32,
    pub mins: usize,
    pub maxs: usize,
    pub abs_min: usize,
    pub abs_max: usize,
    pub size: usize,
    pub solid: (usize, bool),
    pub owner: usize,
    pub area: usize,
    pub area2: usize,
    pub clusters: Option<(usize, usize, usize)>,
    pub network_solid: usize,
    pub appearance: AppearanceLayout,
    pub model_rules: ModelRules,
}
#[derive(Clone, Copy)]
pub(crate) struct AppearanceLayout {
    pub effects_wide: bool,
    pub render_flags: usize,
    pub loop_sound: usize,
    pub event: (usize, bool),
    pub extended: Option<ExtendedAppearance>,
}
#[derive(Clone, Copy)]
pub(crate) struct ExtendedAppearance {
    pub alpha: usize,
    pub scale: usize,
    pub instance_bits: usize,
    pub loop_volume: usize,
    pub loop_attenuation: usize,
    pub owner: usize,
    pub old_frame: usize,
}
impl EntityLayout {
    fn tags(self, flags: u32) -> CollisionTags {
        CollisionTags(
            u8::from(flags & 4 != 0)
                | (u8::from(flags & 2 != 0) << 1)
                | (u8::from(flags & self.player_flag != 0) << 2)
                | (u8::from(flags & self.projectile_flag != 0) << 3),
        )
    }
}

/// Borrowed only while the native child is parked at an import boundary.
pub struct NativeTraceView<'a, 'memory> {
    memory: &'a ModuleMemory<'memory>,
    entities: Entities,
    module: ModuleId,
    layout: EntityLayout,
}
impl NativeTraceView<'_, '_> {
    pub fn pass(&self, address: u64) -> Result<CollisionOwner, CallError> {
        if address == 0 {
            return Ok(CollisionOwner::None);
        }
        let native = native_identity(self.entities, self.module, address)?;
        // Owner fields remain observable even for an inactive passed edict.
        read_owner(
            self.memory,
            self.entities,
            self.module,
            self.layout,
            address,
        )?;
        Ok(CollisionOwner::Native(native))
    }
    pub fn address(&self, entities: &EntityTable, hit: Option<EntityId>) -> Result<u64, CallError> {
        // A protocol cannot expose another module's pointer. A blocking foreign
        // body is represented by the caller's native world edict.
        if self.entities.capacity == 0 {
            return Err(CallError::Entity);
        }
        Ok(hit
            .and_then(|id| native_address(self.entities, self.module, entities, id))
            .unwrap_or(self.entities.address))
    }
}

pub fn slot_address(entities: Entities, slot: u32) -> Option<u64> {
    (slot < entities.capacity).then_some(())?;
    entities
        .address
        .checked_add(u64::from(slot).checked_mul(entities.stride)?)
}

pub(crate) fn native_address(
    native: Entities,
    module: ModuleId,
    entities: &EntityTable,
    id: EntityId,
) -> Option<u64> {
    let slot = entities.resolve(id)?;
    let identity = entities.columns.native_entity[slot]?;
    if identity.module != module {
        return None;
    }
    let slot = u32::try_from(identity.slot).ok()?;
    slot_address(native, slot)
}
impl qa_world::collision::NativeTraceEntities for NativeTraceView<'_, '_> {
    fn module(&self) -> ModuleId {
        self.module
    }
    fn fields(&self, slot: i32) -> Option<(CollisionOwner, CollisionTags)> {
        let slot = u32::try_from(slot).ok()?;
        let address = slot_address(self.entities, slot)?;
        Some((
            read_owner(
                self.memory,
                self.entities,
                self.module,
                self.layout,
                address,
            )
            .ok()?,
            self.layout.tags(
                self.memory
                    .read_word(address + self.layout.flags as u64)
                    .ok()? as u32,
            ),
        ))
    }
}

fn native_identity(
    entities: Entities,
    module: ModuleId,
    address: u64,
) -> Result<NativeEntity, CallError> {
    let offset = address
        .checked_sub(entities.address)
        .ok_or(CallError::Entity)?;
    if address == 0
        || entities.address == 0
        || entities.stride == 0
        || offset % entities.stride != 0
        || offset / entities.stride >= u64::from(entities.capacity)
    {
        return Err(CallError::Entity);
    }
    Ok(NativeEntity {
        module,
        slot: (offset / entities.stride) as i32,
    })
}
fn read_owner(
    memory: &ModuleMemory<'_>,
    entities: Entities,
    module: ModuleId,
    layout: EntityLayout,
    address: u64,
) -> Result<CollisionOwner, CallError> {
    let owner = memory.read_u64(address + layout.owner as u64)?;
    if owner == 0 {
        return Ok(CollisionOwner::None);
    }
    Ok(CollisionOwner::Native(native_identity(
        entities, module, owner,
    )?))
}
#[derive(Clone, Copy)]
struct Binding {
    entity: EntityId,
    owned: bool,
    published: bool,
    // A native ordinal's reuse delay is independent of its common lifetime.
    freed_at: Option<ThinkTime>,
    think_word: Option<u32>,
}

#[derive(Clone, Copy)]
enum EntitySchema {
    Native(EntityLayout),
    QuakeC(QuakeCLayout),
}

#[derive(Clone, Copy)]
struct CachedModel {
    revision: u64,
    geometry: qa_core::primitives::GeometryId,
    shape: CollisionShape,
    bounds: Bounds,
}
pub struct EntityProjection {
    table_offset: usize,
    wide_stride: bool,
    schema: EntitySchema,
    models: qa_core::primitives::IndexSpan,
    model_handles: Box<[Option<CachedModel>]>,
    owner: Option<ModuleId>,
    bindings: Box<[Option<Binding>]>,
    quakec_thinks: Option<quakec::ThinkBindings>,
    observed_count: usize,
    leaves: [u32; 128],
}
impl EntityProjection {
    pub(crate) fn load(
        table_offset: usize,
        wide_stride: bool,
        layout: EntityLayout,
        models: qa_core::primitives::IndexSpan,
    ) -> Self {
        Self {
            table_offset,
            wide_stride,
            schema: EntitySchema::Native(layout),
            models,
            model_handles: vec![None; models.count as usize].into_boxed_slice(),
            owner: None,
            bindings: vec![None; MAX_ENTITIES].into_boxed_slice(),
            quakec_thinks: None,
            observed_count: 0,
            leaves: [0; 128],
        }
    }
    fn native_layout(&self) -> Result<EntityLayout, CallError> {
        match self.schema {
            EntitySchema::Native(layout) => Ok(layout),
            EntitySchema::QuakeC(_) => Err(CallError::Entity),
        }
    }
    pub(crate) fn seed(
        &mut self,
        entities: &EntityTable,
        owner: ModuleId,
    ) -> Result<(), CallError> {
        if let Some(bound) = self.owner {
            return if bound == owner {
                Ok(())
            } else {
                Err(CallError::Entity)
            };
        }
        for entity in entities.active() {
            if let Some(native) = entities.columns.native_entity[entity.slot as usize]
                && native.module == owner
            {
                let slot = usize::try_from(native.slot).map_err(|_| CallError::Entity)?;
                self.borrow_slot(entities, slot, entity)?;
                if matches!(self.schema, EntitySchema::QuakeC(_)) {
                    self.observed_count = self.observed_count.max(slot + 1);
                }
            }
        }
        self.owner = Some(owner);
        Ok(())
    }
    fn borrow_slot(
        &mut self,
        entities: &EntityTable,
        slot: usize,
        entity: EntityId,
    ) -> Result<(), CallError> {
        let row = self.bindings.get_mut(slot).ok_or(CallError::Entity)?;
        if let Some(binding) = *row
            && entities.resolve(binding.entity).is_some()
        {
            return if binding.entity == entity && !binding.owned {
                Ok(())
            } else {
                Err(CallError::Entity)
            };
        }
        *row = Some(Binding {
            entity,
            owned: false,
            published: false,
            freed_at: None,
            think_word: None,
        });
        Ok(())
    }
    pub(crate) fn bind_client(
        &mut self,
        entities: &EntityTable,
        native: NativeEntity,
        entity: EntityId,
    ) -> Result<(), CallError> {
        if self.owner != Some(native.module)
            || entities.resolve(entity).is_none()
            || entities.columns.native_entity[entity.slot as usize] != Some(native)
        {
            return Err(CallError::Entity);
        }
        let slot = usize::try_from(native.slot).map_err(|_| CallError::Entity)?;
        self.borrow_slot(entities, slot, entity)
    }
    pub(crate) fn binding(
        &self,
        entities: &EntityTable,
        native: NativeEntity,
    ) -> Result<Option<EntityId>, CallError> {
        if self.owner != Some(native.module) {
            return Err(CallError::Entity);
        }
        let slot = usize::try_from(native.slot).map_err(|_| CallError::Entity)?;
        let binding = self.bindings.get(slot).ok_or(CallError::Entity)?;
        Ok(binding.and_then(|binding| {
            let entity = binding.entity;
            let slot = entities.resolve(entity)?;
            (entities.columns.native_entity[slot] == Some(native)).then_some(entity)
        }))
    }
    pub(crate) fn read(
        &self,
        memory: &ModuleMemory<'_>,
        table: u64,
    ) -> Result<Entities, CallError> {
        let at = table
            .checked_add(self.table_offset as u64)
            .ok_or(CallError::Memory)?;
        let word = |offset| -> Result<u64, CallError> { Ok(memory.read_u64(at + offset)?) };
        let address = word(0)?;
        let (stride, offset) = if self.wide_stride {
            (word(8)?, 16)
        } else {
            (
                u64::try_from(memory.read_word(at + 8)?).map_err(|_| CallError::Entity)?,
                12,
            )
        };
        let count = memory.read_word(at + offset)? as u32;
        let capacity = memory.read_word(at + offset + 4)? as u32;
        let server_flags = if self.wide_stride {
            memory.read_word(at + offset + 8)? as u32
        } else {
            0
        };
        if count > capacity
            || capacity as usize > self.bindings.len()
            || (address == 0 && count != 0)
            || (address != 0 && stride < self.native_layout()?.bytes as u64)
        {
            return Err(CallError::Entity);
        }
        if address != 0 {
            let bytes = stride
                .checked_mul(u64::from(capacity))
                .and_then(|n| usize::try_from(n).ok())
                .ok_or(CallError::Memory)?;
            memory.read(address, bytes)?;
        }
        Ok(Entities {
            address,
            stride,
            count,
            capacity,
            server_flags,
        })
    }
    fn entity(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<(Entities, usize, Option<EntityId>), CallError> {
        let entities = self.read(memory, table)?;
        let native = native_identity(entities, context.module, address)?;
        let slot = native.slot as usize;
        let bound = self.binding(&services.server.entities, native)?;
        if bound.is_none() {
            self.bindings[slot] = None;
        }
        Ok((entities, slot, bound))
    }
    pub fn trace_view<'a, 'memory>(
        &self,
        memory: &'a ModuleMemory<'memory>,
        context: CallContext,
        table: u64,
    ) -> Result<NativeTraceView<'a, 'memory>, CallError> {
        Ok(NativeTraceView {
            memory,
            entities: self.boundary(memory, context, table)?,
            module: context.module,
            layout: self.native_layout()?,
        })
    }
    pub(crate) fn boundary(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        table: u64,
    ) -> Result<Entities, CallError> {
        if self.owner != Some(context.module) {
            return Err(CallError::Entity);
        }
        self.read(memory, table)
    }
    /// An export has parked its child. Observe every published native lifetime,
    /// including active entities that never called LinkEntity. Presentation
    /// pose edits update the shared columns but never reinsert area bounds;
    /// only the explicit native LinkEntity service owns that operation.
    pub(crate) fn project(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
    ) -> Result<Entities, CallError> {
        let native = self.boundary(memory, context, table)?;
        let count = native.count as usize;
        // A rejected row must not hide earlier admitted lifetimes from the
        // next reconciliation when the module shrinks its published table.
        let previous = std::mem::replace(&mut self.observed_count, count);
        for slot in 1..count.max(previous) {
            if slot >= count {
                self.retire(services, context, slot)?;
                continue;
            }
            let address = slot_address(native, slot as u32).ok_or(CallError::Entity)?;
            if field(memory, address, self.native_layout()?.in_use)? == 0 {
                if let Some(entity) = self.binding(
                    &services.server.entities,
                    NativeEntity {
                        module: context.module,
                        slot: slot as i32,
                    },
                )? {
                    services.server.entities.columns.appearance[entity.slot as usize] =
                        EntityAppearance::default();
                }
                self.retire(services, context, slot)?;
                continue;
            }
            let (body, angles) = self.body(memory, address)?;
            let appearance = self.appearance(memory, address, context.module)?;
            let flags = memory.read_word(address + self.native_layout()?.flags as u64)? as u32;
            let bound = self.binding(
                &services.server.entities,
                NativeEntity {
                    module: context.module,
                    slot: slot as i32,
                },
            )?;
            let entity = self.bind(services, context, slot, bound)?;
            let slot = entity.slot as usize;
            let columns = &mut services.server.entities.columns;
            columns.position[slot] = body.position;
            columns.mins[slot] = body.mins;
            columns.maxs[slot] = body.maxs;
            columns.angles[slot] = angles;
            columns.flags[slot] = flags;
            columns.appearance[slot] = appearance;
        }
        Ok(native)
    }
    fn appearance(
        &self,
        memory: &ModuleMemory<'_>,
        address: u64,
        module: ModuleId,
    ) -> Result<EntityAppearance, CallError> {
        let layout = self.native_layout()?.appearance;
        let mut models = [None; 4];
        for (index, model) in models.iter_mut().enumerate() {
            *model = ModelRef::native(
                ResourceNamespace::Module(module),
                memory.read_word(address + 40 + index as u64 * 4)? as u32,
            );
        }
        let mut native = NativeAppearanceTail {
            effects: if layout.effects_wide {
                memory.read_u64(address + 64)?
            } else {
                u64::from(memory.read_word(address + 64)? as u32)
            },
            render_flags: memory.read_word(address + layout.render_flags as u64)? as u32,
            event: field(memory, address, layout.event)?,
            loop_sound: memory.read_word(address + layout.loop_sound as u64)? as u32,
            solid: memory.read_word(address + self.native_layout()?.network_solid as u64)? as u32,
            ..NativeAppearanceTail::default()
        };
        let (alpha, scale) = if let Some(extended) = layout.extended {
            native.instance_bits = field(memory, address, (extended.instance_bits, true))? as u8;
            native.loop_volume =
                f32::from_bits(memory.read_word(address + extended.loop_volume as u64)? as u32);
            native.loop_attenuation = f32::from_bits(
                memory.read_word(address + extended.loop_attenuation as u64)? as u32,
            );
            native.owner = memory.read_word(address + extended.owner as u64)?;
            native.old_frame = memory.read_word(address + extended.old_frame as u64)?;
            (
                f32::from_bits(memory.read_word(address + extended.alpha as u64)? as u32),
                f32::from_bits(memory.read_word(address + extended.scale as u64)? as u32),
            )
        } else {
            (1.0, 1.0)
        };
        Ok(EntityAppearance {
            source: self.native_layout()?.source,
            models,
            frame: memory.read_word(address + 56)?,
            skin: memory.read_word(address + 60)?,
            old_origin: memory.read_vec3(address + 28)?,
            alpha,
            scale,
            native,
        })
    }
    pub(crate) fn identity(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<NativeEntity, CallError> {
        native_identity(
            self.boundary(memory, context, table)?,
            context.module,
            address,
        )
    }
    fn observe(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<EntityId, CallError> {
        let (entities, slot, bound) = self.entity(services, memory, context, table, address)?;
        // Native world zero is geometry, not an allocated area-index body.
        if slot == 0 || slot >= entities.count as usize {
            return Err(CallError::Entity);
        }
        if field(memory, address, self.native_layout()?.in_use)? == 0 {
            return Err(CallError::Entity);
        }
        self.bind(services, context, slot, bound)
    }
    fn bind(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        slot: usize,
        bound: Option<EntityId>,
    ) -> Result<EntityId, CallError> {
        if let Some(entity) = bound {
            return Ok(entity);
        }
        let entity = (ENGINE_CALLS.spawn)(services, context)?;
        services.server.entities.columns.native_entity[entity.slot as usize] = Some(NativeEntity {
            module: context.module,
            slot: slot as i32,
        });
        self.bindings[slot] = Some(Binding {
            entity,
            owned: true,
            published: false,
            freed_at: None,
            think_word: None,
        });
        Ok(entity)
    }
    pub fn unlink(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<(), CallError> {
        let (entities, slot, bound) = self.entity(services, memory, context, table, address)?;
        let linked_address = self
            .native_layout()?
            .linked
            .filter(|_| slot != 0)
            .map(|linked| {
                if linked as u64 >= entities.stride {
                    return Err(CallError::Entity);
                }
                address.checked_add(linked as u64).ok_or(CallError::Memory)
            })
            .transpose()?;
        // Native world slot zero is not an area-index entity.
        if slot != 0
            && let Some(entity) = bound
        {
            (ENGINE_CALLS.unlink)(services, entity)?;
        }
        // API2023 publishes a linked byte; classic linkage remains engine-owned.
        if let Some(address) = linked_address {
            memory.write(address, &[0])?;
        }
        Ok(())
    }
    pub fn forget_observer(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<(), CallError> {
        let bound = if address == 0 {
            None
        } else {
            self.entity(services, memory, context, table, address)?.2
        };
        (ENGINE_CALLS.bot_registration)(services, bound, false)
    }
    pub fn register_observer(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<(), CallError> {
        let entity = self.observe(services, memory, context, table, address)?;
        (ENGINE_CALLS.bot_registration)(services, Some(entity), true)
    }
    pub fn link(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<(), CallError> {
        let (entities, native_slot, mut bound) =
            self.entity(services, memory, context, table, address)?;
        if native_slot == 0 {
            return Ok(());
        }
        let layout = self.native_layout()?;
        if field(memory, address, layout.in_use)? == 0 {
            self.retire(services, context, native_slot)?;
            if let Some(linked) = layout.linked {
                memory.write(address + linked as u64, &[0])?;
            }
            return Ok(());
        }
        if native_slot >= entities.count as usize {
            return Err(CallError::Entity);
        }
        let (body, angles) = self.body(memory, address)?;
        let origin = body.position;
        let bounds = Bounds {
            mins: body.mins,
            maxs: body.maxs,
        };
        let solid = field(memory, address, layout.solid)?;
        if solid > 3 {
            return Err(CallError::Entity);
        }
        let flags = memory.read_word(address + layout.flags as u64)? as u32;
        let link_count = memory.read_word(address + layout.link_count as u64)?;
        let shape = if solid == 3 {
            self.model(services, memory, context, address)?.0
        } else if solid == 2 {
            CollisionShape::Box
        } else {
            CollisionShape::None
        };
        let owner = read_owner(memory, entities, context.module, layout, address)?;
        // A reset native link count begins a fresh game-owned lifetime. Borrowed
        // client lifetimes remain owned by the session, not by this adapter.
        if link_count == 0 && self.bindings[native_slot].is_some_and(|b| b.owned && b.published) {
            self.retire(services, context, native_slot)?;
            bound = None;
        }
        let entity = self.bind(services, context, native_slot, bound)?;
        let slot = entity.slot as usize;
        let columns = &mut services.server.entities.columns;
        columns.position[slot] = origin;
        columns.angles[slot] = angles;
        columns.mins[slot] = bounds.mins;
        columns.maxs[slot] = bounds.maxs;
        columns.collision_shape[slot] = shape;
        columns.model_rules[slot] = layout.model_rules;
        columns.collision_owner[slot] = owner;
        columns.collision_tags[slot] = layout.tags(flags);
        // Original Q2's temporary box hull carries CONTENTS_MONSTER.
        columns.collision_contents[slot] = if solid == 2 {
            Contents::BODY.0
        } else {
            Contents::SOLID.0
        };
        let link_flags = match solid {
            1 => LinkFlags::TRIGGER,
            2 | 3 => LinkFlags::SOLID,
            _ => LinkFlags::LINKED,
        };
        (ENGINE_CALLS.link)(services, context, entity, link_flags)?;
        let absolute = services
            .server
            .area
            .bounds(entity)
            .ok_or(CallError::Entity)?;
        let result = (ENGINE_CALLS.box_leaves)(services, absolute, &mut self.leaves)?;
        let world = services.visibility.as_ref().ok_or(CallError::Geometry)?.0;
        let mut areas = [0u32; 2];
        let mut clusters = [0u32; 16];
        let mut count = 0;
        for &index in &self.leaves[..result.count] {
            let leaf = world.leaf(index).ok_or(CallError::Geometry)?;
            let area = leaf.area.unwrap_or(0);
            if area != 0 {
                if areas[0] != 0 && areas[0] != area {
                    areas[1] = area;
                } else {
                    areas[0] = area;
                }
            }
            if let Some(cluster) = leaf.selector
                && !clusters[..count.min(16)].contains(&cluster)
            {
                if count < 16 {
                    clusters[count] = cluster;
                }
                count += 1;
            }
        }
        memory.write_vec3(address + layout.abs_min as u64, absolute.mins)?;
        memory.write_vec3(address + layout.abs_max as u64, absolute.maxs)?;
        memory.write_vec3(address + layout.size as u64, bounds.maxs - bounds.mins)?;
        for (offset, value) in [
            (layout.area, areas[0]),
            (layout.area2, areas[1]),
            (layout.link_count, (link_count as u32).wrapping_add(1)),
            (
                layout.network_solid,
                if solid == 3 {
                    31
                } else if solid == 2 && flags & 2 == 0 {
                    pack_solid(bounds, layout.linked.is_some())
                } else {
                    0
                },
            ),
        ] {
            memory.write_word(address + offset as u64, value as i32)?;
        }
        if let Some((number, list, head)) = layout.clusters {
            let overflow = result.count == self.leaves.len() || count > 16;
            memory.write_word(
                address + number as u64,
                if overflow { -1 } else { count as i32 },
            )?;
            for (n, &cluster) in clusters[..count.min(16)].iter().enumerate() {
                memory.write_word(address + list as u64 + n as u64 * 4, cluster as i32)?;
            }
            if overflow {
                memory.write_word(address + head as u64, result.top_node)?;
            }
        }
        if let Some(linked) = layout.linked {
            memory.write(address + linked as u64, &[1])?;
        }
        if link_count == 0
            && (layout.linked.is_none() || memory.read_word(address + 72)? as u32 & 128 == 0)
        {
            memory.write_vec3(address + 28, origin)?;
        }
        if let Some(binding) = self.bindings[native_slot].as_mut() {
            binding.published = true;
        }
        Ok(())
    }
    fn retire(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        slot: usize,
    ) -> Result<(), CallError> {
        if let Some(binding) = self.bindings[slot] {
            if services.server.entities.resolve(binding.entity).is_none() {
                self.bindings[slot] = None;
                return Ok(());
            }
            if binding.owned {
                (ENGINE_CALLS.free)(services, context, binding.entity)?;
                self.bindings[slot] = None;
            } else {
                (ENGINE_CALLS.unlink)(services, binding.entity)?;
            }
        }
        Ok(())
    }
    pub fn sound_source(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<(qa_core::primitives::SoundEmitter, Vec3, bool), CallError> {
        let (_, slot, entity) = self.entity(services, memory, context, table, address)?;
        let (body, _) = self.body(memory, address)?;
        let bsp = field(memory, address, self.native_layout()?.solid)? == 3;
        let position = if bsp { body.center() } else { body.position };
        let positioned =
            bsp || memory.read_word(address + self.native_layout()?.flags as u64)? as u32 & 1 != 0;
        let emitter = entity.map_or(
            qa_core::primitives::SoundEmitter::Native {
                namespace: ResourceNamespace::Module(context.module),
                slot: slot as i32,
            },
            qa_core::primitives::SoundEmitter::Entity,
        );
        Ok((emitter, position, positioned))
    }
    pub(crate) fn body(
        &self,
        memory: &ModuleMemory<'_>,
        address: u64,
    ) -> Result<(Body, Vec3), CallError> {
        let body = Body {
            position: memory.read_vec3(address + 4)?,
            mins: memory.read_vec3(address + self.native_layout()?.mins as u64)?,
            maxs: memory.read_vec3(address + self.native_layout()?.maxs as u64)?,
            ..Body::default()
        };
        let angles = memory.read_vec3(address + 16)?;
        if !body.position.is_finite()
            || !angles.is_finite()
            || !(Bounds {
                mins: body.mins,
                maxs: body.maxs,
            })
            .is_valid()
        {
            return Err(CallError::Entity);
        }
        Ok((body, angles))
    }
    fn model(
        &mut self,
        services: &EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        address: u64,
    ) -> Result<(CollisionShape, Bounds), CallError> {
        let model = memory.read_word(address + 40)? as u32;
        self.cache_model(services, context, model, None)
    }
    fn cache_model(
        &mut self,
        services: &EngineServices<'_>,
        context: CallContext,
        model: u32,
        name: Option<&[u8]>,
    ) -> Result<(CollisionShape, Bounds), CallError> {
        let cached = self
            .model_handles
            .get_mut(model as usize)
            .ok_or(CallError::Geometry)?;
        let (registered, revision) = services.storage.configstring(
            context.module,
            self.models.get(model as usize).ok_or(CallError::Geometry)?,
        )?;
        let (geometry, _) = services.world.ok_or(CallError::Geometry)?;
        if name.is_none()
            && let Some(entry) =
                cached.filter(|entry| entry.revision == revision && entry.geometry == geometry)
        {
            return Ok((entry.shape, entry.bounds));
        }
        let (shape, bounds) = Self::inline_model(services, geometry, name.unwrap_or(registered))?;
        *cached = Some(CachedModel {
            revision,
            geometry,
            shape,
            bounds,
        });
        Ok((shape, bounds))
    }

    pub(crate) fn clip_target(
        &mut self,
        services: &EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
    ) -> Result<ClipTarget, CallError> {
        let identity = self.identity(memory, context, table, address)?;
        if identity.slot == 0 {
            return Ok(ClipTarget::World);
        }
        let (body, angles) = self.body(memory, address)?;
        let solid = field(memory, address, self.native_layout()?.solid)?;
        let shape = match solid {
            3 => self.model(services, memory, context, address)?.0,
            1 => self
                .model(services, memory, context, address)
                .map_or(CollisionShape::Box, |model| model.0),
            0 | 2 => CollisionShape::Box,
            _ => return Err(CallError::Entity),
        };
        Ok(ClipTarget::Body {
            body,
            shape,
            angles,
            rules: self.native_layout()?.model_rules,
        })
    }
    pub(crate) fn inline_model(
        services: &EngineServices<'_>,
        geometry: qa_core::primitives::GeometryId,
        name: &[u8],
    ) -> Result<(CollisionShape, Bounds), CallError> {
        let suffix = name.strip_prefix(b"*").ok_or(CallError::Geometry)?;
        let index = qa_core::text::atoi_prefix(suffix) as u32;
        if index == 0 {
            return Err(CallError::Geometry);
        }
        let bounds = services
            .geometry
            .model_bounds(geometry, index)
            .ok_or(CallError::Geometry)?;
        Ok((CollisionShape::Model { geometry, index }, bounds))
    }
    pub fn set_model(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        table: u64,
        address: u64,
        name: u64,
    ) -> Result<(), CallError> {
        self.entity(services, memory, context, table, address)?;
        let name = memory.cstring(name)?;
        let model = (ENGINE_CALLS.resource_index)(services, context.module, self.models, name)?;
        let inline = if name.starts_with(b"*") {
            Some(self.cache_model(services, context, model, Some(name))?)
        } else {
            *self
                .model_handles
                .get_mut(model as usize)
                .ok_or(CallError::Geometry)? = None;
            None
        };
        memory.write_word(address + 40, model as i32)?;
        if let Some((_, bounds)) = inline {
            memory.write_vec3(address + self.native_layout()?.mins as u64, bounds.mins)?;
            memory.write_vec3(address + self.native_layout()?.maxs as u64, bounds.maxs)?;
            self.link(services, memory, context, table, address)?;
        }
        Ok(())
    }
}

fn field(
    memory: &ModuleMemory<'_>,
    address: u64,
    (offset, byte): (usize, bool),
) -> Result<u32, CallError> {
    Ok(memory.read_field(
        address + offset as u64,
        if byte { Width::Byte } else { Width::Word },
    )?)
}
fn pack_solid(bounds: Bounds, wide: bool) -> u32 {
    if !wide {
        let x = (bounds.maxs.0[0] / 8.0) as i32;
        let down = (-bounds.mins.0[2] / 8.0) as i32;
        let up = ((bounds.maxs.0[2] + 32.0) / 8.0) as i32;
        return (x.clamp(1, 31) | down.clamp(1, 31) << 5 | up.clamp(1, 63) << 10) as u32;
    }
    if bounds.mins == bounds.maxs {
        return 0;
    }
    let byte = |value: f32, min: u32| {
        if value <= min as f32 {
            min
        } else if value >= 255.0 {
            255
        } else {
            value as u32
        }
    };
    let packed = byte(bounds.maxs.0[0], 1)
        | byte(bounds.maxs.0[1], 1) << 8
        | byte(-bounds.mins.0[2], 0) << 16
        | byte(bounds.maxs.0[2] + 32.0, 0) << 24;
    if packed == 31 { 0 } else { packed }
}
