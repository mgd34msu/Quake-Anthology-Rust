//! RuleSetId views select defaults and conversions, never a second cvar table.
use crate::catalog::Scope;

pub use qa_core::primitives::{CommandProducer, RuleSetId};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Engine,
    Game,
    Cgame,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    /// Captured file selection for deferred module/connection commands.
    pub file_search: Option<qa_core::primitives::FileSearchId>,
    pub source: RuleSetId,
    pub side: Scope,
    pub role: Role,
    pub dedicated: bool,
    pub seat: qa_core::sys_events::SeatId,
    pub event_time: Option<qa_core::sys_events::EventTime>,
    pub key_source: Option<qa_core::sys_events::KeySource>,
    pub producer: Option<CommandProducer>,
    /// Captured CLIENT owner for deferred engine commands from a connection.
    pub client_lifetime: Option<(qa_core::primitives::ClientId, qa_core::primitives::EntityId)>,
    /// Captured role choices for one deferred connection command.
    pub client_policy: Option<qa_core::primitives::ClientPolicy>,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            file_search: None,
            source: RuleSetId::Quake3,
            side: Scope::Client,
            role: Role::Engine,
            dedicated: false,
            seat: qa_core::sys_events::SeatId::FIRST,
            event_time: None,
            key_source: None,
            producer: None,
            client_lifetime: None,
            client_policy: None,
        }
    }
}
