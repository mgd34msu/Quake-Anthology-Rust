//! One entity/player publication owner for interpreted and native module memory.
use crate::{
    entities::EntityProjection,
    memory::ModuleMemory,
    players::PlayerValues,
    services::{CallContext, CallError, EngineServices},
};
use qa_core::{
    primitives::{
        ClockObservation, EntityId, ModuleId, NativeEntity, PlayerValueBindings, Q3StatLayout,
    },
    sys_events::EventTime,
};
use qa_world::entities::EntityTable;

pub(crate) struct Projection {
    pub entities: EntityProjection,
    values: PlayerValues,
    frame_observation: Option<FrameObservation>,
}
struct FrameObservation {
    exports: [u32; 2],
    last: Option<ClockObservation>,
}
impl Projection {
    pub fn load_q3(layout: Q3StatLayout) -> Self {
        Self {
            entities: EntityProjection::load_q3(),
            values: PlayerValues::load_q3(layout),
            frame_observation: Some(FrameObservation {
                exports: [0, 8],
                last: None,
            }),
        }
    }
    pub fn load_q2(
        entities: EntityProjection,
        client: &'static crate::q2_abi::ClientLayout,
    ) -> Self {
        Self {
            entities,
            values: PlayerValues::load_q2(client),
            frame_observation: None,
        }
    }
    pub fn bind_values(
        &mut self,
        values: PlayerValueBindings<'_>,
        registry: &qa_gameplay::registry::Registry,
    ) -> Result<(), CallError> {
        self.values.bind(values, registry)
    }
    pub fn stat_layout(&self) -> Q3StatLayout {
        self.values.stat_layout()
    }
    pub fn seed(&mut self, table: &EntityTable, module: ModuleId) -> Result<(), CallError> {
        self.entities.seed(table, module)
    }
    pub fn bind_client(
        &mut self,
        table: &EntityTable,
        native: NativeEntity,
        entity: EntityId,
    ) -> Result<(), CallError> {
        let slot = table.resolve(entity).ok_or(CallError::Entity)?;
        if table.columns.owner[slot] != native.module {
            return Err(CallError::Entity);
        }
        self.entities.bind_client(table, native, entity)
    }

    pub fn prepare(
        &mut self,
        memory: &mut ModuleMemory<'_>,
        services: &mut EngineServices<'_>,
        context: CallContext,
        mut pmove: Option<&mut crate::pmove::Pmove>,
    ) -> Result<(), CallError> {
        self.seed(&services.server.entities, context.module)?;
        if !self.entities.ready() || services.server.entities.attachment_ids().is_empty() {
            return Ok(());
        }
        let table = self.entities.boundary(memory, context)?;
        services.sync_attachment_positions(context.module, |services, entity, native, position| {
            self.write_position(
                memory,
                services,
                context,
                table,
                entity,
                native,
                position,
                pmove.as_deref_mut(),
            )
        })
    }

    fn write_position(
        &self,
        memory: &mut ModuleMemory<'_>,
        services: &mut EngineServices<'_>,
        context: CallContext,
        table: crate::entities::Entities,
        entity: EntityId,
        native: NativeEntity,
        position: qa_core::primitives::Vec3,
        pmove: Option<&mut crate::pmove::Pmove>,
    ) -> Result<(), CallError> {
        self.entities
            .write_position(memory, services.server, context, entity, position)?;
        if let Some(client) = self.values.write_position(
            memory,
            table,
            &self.entities,
            context,
            services.server,
            native,
            pmove,
            position,
        )? {
            services.server.clients[client.0 as usize]
                .player
                .body
                .position = position;
        }
        Ok(())
    }

    pub fn transport(
        &mut self,
        memory: &mut ModuleMemory<'_>,
        services: &mut EngineServices<'_>,
        context: CallContext,
        entity: EntityId,
        position: qa_core::primitives::Vec3,
        pmove: Option<&mut crate::pmove::Pmove>,
        relink: bool,
    ) -> Result<(), CallError> {
        let slot = services
            .server
            .entities
            .resolve(entity)
            .ok_or(CallError::Entity)?;
        let native =
            services.server.entities.columns.native_entity[slot].ok_or(CallError::Entity)?;
        if native.module != context.module
            || self.entities.binding(&services.server.entities, native)? != Some(entity)
        {
            return Err(CallError::Entity);
        }
        let table = self.entities.boundary(memory, context)?;
        let address = crate::entities::native_address(
            table,
            context.module,
            &services.server.entities,
            entity,
        )
        .ok_or(CallError::Entity)?;
        services
            .server
            .entities
            .columns
            .transport_position(slot, position);
        self.write_position(
            memory, services, context, table, entity, native, position, pmove,
        )?;
        if relink {
            self.entities.link(services, memory, context, address)?;
        }
        Ok(())
    }
    /// The caller lends native bytes only after parking its owned child.
    /// Failed exports do not enter this successful-return publication phase.
    pub fn publish(
        &mut self,
        memory: &mut ModuleMemory<'_>,
        services: &mut EngineServices<'_>,
        context: CallContext,
        mut pmove: Option<&mut crate::pmove::Pmove>,
        time: EventTime,
        completed_export: Option<u32>,
    ) -> Result<(), CallError> {
        if self.entities.ready() {
            let table = self.entities.project(services, memory, context)?;
            let proposed = self.values.observe_clock(ClockObservation {
                native: context.clock,
                queued: time,
            })?;
            let clock = self
                .frame_observation
                .as_ref()
                .filter(|frame| {
                    !completed_export.is_some_and(|export| frame.exports.contains(&export))
                })
                .and_then(|frame| frame.last)
                .unwrap_or(proposed);
            self.values.project(
                memory,
                table,
                &self.entities,
                context.module,
                services.registry,
                services.server,
                pmove.as_deref_mut(),
                time,
                clock,
            )?;
            if let Some(frame) = &mut self.frame_observation {
                frame.last = Some(clock);
            }
            self.prepare(memory, services, context, pmove)?;
        }
        Ok(())
    }
}
