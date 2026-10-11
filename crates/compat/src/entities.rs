//! Native entity-address projection onto common lifetime handles.
use crate::{
    memory::ModuleMemory,
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices},
};
use qa_core::primitives::{
    Bounds, CollisionOwner, CollisionShape, CollisionTags, EntityId, ModelRules, ModuleId,
    NativeEntity,
};
use qa_world::area::LinkFlags;
use qa_world::collision::Contents;
use qa_world::entities::{EntityTable, MAX_ENTITIES};

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
    pub model_rules: ModelRules,
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
    if slot >= native.capacity {
        return None;
    }
    native.address.checked_add(u64::from(slot) * native.stride)
}
impl qa_world::collision::NativeTraceEntities for NativeTraceView<'_, '_> {
    fn module(&self) -> ModuleId {
        self.module
    }
    fn fields(&self, slot: i32) -> Option<(CollisionOwner, CollisionTags)> {
        let slot = u32::try_from(slot).ok()?;
        if slot >= self.entities.capacity {
            return None;
        }
        let address = self
            .entities
            .address
            .checked_add(u64::from(slot) * self.entities.stride)?;
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
    let owner = u64::from_le_bytes(
        memory
            .read(address + layout.owner as u64, 8)?
            .try_into()
            .map_err(|_| CallError::Memory)?,
    );
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
}

pub struct EntityProjection {
    table_offset: usize,
    wide_stride: bool,
    layout: EntityLayout,
    models: crate::services::ResourceRange,
    owner: Option<ModuleId>,
    bindings: Box<[Option<Binding>]>,
    leaves: [u32; 128],
}
impl EntityProjection {
    pub(crate) fn load(
        table_offset: usize,
        wide_stride: bool,
        layout: EntityLayout,
        models: crate::services::ResourceRange,
    ) -> Self {
        Self {
            table_offset,
            wide_stride,
            layout,
            models,
            owner: None,
            bindings: vec![None; MAX_ENTITIES].into_boxed_slice(),
            leaves: [0; 128],
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
                let row = self.bindings.get_mut(slot).ok_or(CallError::Entity)?;
                if row.is_some() {
                    return Err(CallError::Entity);
                }
                *row = Some(Binding {
                    entity,
                    owned: false,
                    published: false,
                });
            }
        }
        self.owner = Some(owner);
        Ok(())
    }
    pub(crate) fn read(
        &self,
        memory: &ModuleMemory<'_>,
        table: u64,
    ) -> Result<Entities, CallError> {
        let at = table
            .checked_add(self.table_offset as u64)
            .ok_or(CallError::Memory)?;
        let word = |offset| -> Result<u64, CallError> {
            Ok(u64::from_le_bytes(
                memory
                    .read(at + offset, 8)?
                    .try_into()
                    .map_err(|_| CallError::Memory)?,
            ))
        };
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
            || (address != 0 && stride < self.layout.bytes as u64)
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
        if self.owner != Some(context.module) {
            return Err(CallError::Entity);
        }
        let bound = self.bindings[slot].filter(|binding| {
            let entity = binding.entity;
            services.server.entities.resolve(entity).is_some()
                && services.server.entities.columns.native_entity[entity.slot as usize]
                    == Some(native)
        });
        if bound.is_none() {
            self.bindings[slot] = None;
        }
        Ok((entities, slot, bound.map(|binding| binding.entity)))
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
            layout: self.layout,
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
        if field(memory, address, self.layout.in_use)? == 0 {
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
            .layout
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
        let layout = self.layout;
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
        let origin = memory.read_vec3(address + 4)?;
        let angles = memory.read_vec3(address + 16)?;
        let bounds = Bounds {
            mins: memory.read_vec3(address + layout.mins as u64)?,
            maxs: memory.read_vec3(address + layout.maxs as u64)?,
        };
        if origin.0.iter().chain(&angles.0).any(|v| !v.is_finite()) || !bounds.is_valid() {
            return Err(CallError::Entity);
        }
        let solid = field(memory, address, layout.solid)?;
        if solid > 3 {
            return Err(CallError::Entity);
        }
        let flags = memory.read_word(address + layout.flags as u64)? as u32;
        let link_count = memory.read_word(address + layout.link_count as u64)?;
        let shape = if solid == 3 {
            let model = memory.read_word(address + 40)? as u32;
            if model as usize >= self.models.count {
                return Err(CallError::Geometry);
            }
            let (name, _) = services
                .storage
                .configstring(context.module, self.models.first + model as usize)?;
            Self::inline_model(services, name)?.0
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
            if binding.owned {
                (ENGINE_CALLS.free)(services, context, binding.entity)?;
                self.bindings[slot] = None;
            } else {
                (ENGINE_CALLS.unlink)(services, binding.entity)?;
            }
        }
        Ok(())
    }
    fn inline_model(
        services: &EngineServices<'_>,
        name: &[u8],
    ) -> Result<(CollisionShape, Bounds), CallError> {
        let index = std::str::from_utf8(name.strip_prefix(b"*").ok_or(CallError::Geometry)?)
            .ok()
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|&index| index != 0)
            .ok_or(CallError::Geometry)?;
        let (geometry, _) = services.world.ok_or(CallError::Geometry)?;
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
        let inline = name
            .starts_with(b"*")
            .then(|| Self::inline_model(services, name))
            .transpose()?;
        memory.write_word(address + 40, model as i32)?;
        if let Some((_, bounds)) = inline {
            memory.write_vec3(address + self.layout.mins as u64, bounds.mins)?;
            memory.write_vec3(address + self.layout.maxs as u64, bounds.maxs)?;
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
    let at = address + offset as u64;
    Ok(if byte {
        u32::from(memory.read(at, 1)?[0])
    } else {
        memory.read_word(at)? as u32
    })
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
