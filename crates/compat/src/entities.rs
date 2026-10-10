//! Native entity-address projection onto common lifetime handles.
use crate::{
    memory::ModuleMemory,
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices},
};
use qa_core::primitives::{EntityId, ModuleId, NativeEntity};
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

pub struct EntityProjection {
    table_offset: usize,
    wide_stride: bool,
    linked: Option<usize>,
    owner: Option<ModuleId>,
    bindings: Box<[Option<EntityId>]>,
}
impl EntityProjection {
    pub(crate) fn load(table_offset: usize, wide_stride: bool, linked: Option<usize>) -> Self {
        Self {
            table_offset,
            wide_stride,
            linked,
            owner: None,
            bindings: vec![None; MAX_ENTITIES].into_boxed_slice(),
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
                *row = Some(entity);
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
            || (address != 0 && stride == 0)
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
        let offset = address
            .checked_sub(entities.address)
            .ok_or(CallError::Entity)?;
        if address == 0
            || entities.address == 0
            || entities.stride == 0
            || offset % entities.stride != 0
        {
            return Err(CallError::Entity);
        }
        let slot = usize::try_from(offset / entities.stride).map_err(|_| CallError::Entity)?;
        if slot >= entities.capacity as usize || self.owner != Some(context.module) {
            return Err(CallError::Entity);
        }
        let native = NativeEntity {
            module: context.module,
            slot: slot as i32,
        };
        let bound = self.bindings[slot].filter(|&entity| {
            services.server.entities.resolve(entity).is_some()
                && services.server.entities.columns.native_entity[entity.slot as usize]
                    == Some(native)
        });
        if bound.is_none() {
            self.bindings[slot] = None;
        }
        Ok((entities, slot, bound))
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
}
