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
    /// The caller lends native bytes only after parking its owned child.
    /// Failed exports do not enter this successful-return publication phase.
    pub fn publish(
        &mut self,
        memory: &mut ModuleMemory<'_>,
        services: &mut EngineServices<'_>,
        context: CallContext,
        pmove: Option<&mut crate::pmove::Pmove>,
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
                pmove,
                time,
                clock,
            )?;
            if let Some(frame) = &mut self.frame_observation {
                frame.last = Some(clock);
            }
        }
        Ok(())
    }
}
