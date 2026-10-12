//! One entity/player publication owner for interpreted and native module memory.
use crate::{
    entities::EntityProjection,
    memory::ModuleMemory,
    players::PlayerValues,
    services::{CallContext, CallError, EngineServices},
};
use qa_core::primitives::{ModuleId, ValueBinding};
use qa_world::entities::EntityTable;

pub(crate) struct Projection {
    pub entities: EntityProjection,
    values: PlayerValues,
}
impl Projection {
    pub fn load_q3() -> Self {
        Self {
            entities: EntityProjection::load_q3(),
            values: PlayerValues::load_q3(),
        }
    }
    pub fn load_q2(
        entities: EntityProjection,
        client: &'static crate::q2_abi::ClientLayout,
    ) -> Self {
        Self {
            entities,
            values: PlayerValues::load_q2(client),
        }
    }
    pub fn bind_values(
        &mut self,
        stats: &[ValueBinding],
        persistent: &[ValueBinding],
    ) -> Result<(), CallError> {
        self.values.bind(stats, persistent)
    }
    pub fn seed(&mut self, table: &EntityTable, module: ModuleId) -> Result<(), CallError> {
        self.entities.seed(table, module)
    }
    /// The caller lends native bytes only after parking its owned child.
    /// Failed exports do not enter this successful-return publication phase.
    pub fn publish(
        &mut self,
        memory: &mut ModuleMemory<'_>,
        services: &mut EngineServices<'_>,
        context: CallContext,
        pmove: Option<&mut crate::pmove::Pmove>,
    ) -> Result<(), CallError> {
        if self.entities.ready() {
            let table = self.entities.project(services, memory, context)?;
            self.values.project(
                memory,
                table,
                &self.entities,
                context.module,
                services.server,
                pmove,
            )?;
        }
        Ok(())
    }
}
