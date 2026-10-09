//! RuleSetId views select defaults and conversions, never a second cvar table.
use crate::catalog::Scope;

pub use qa_core::primitives::RuleSetId;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Engine,
    Game,
    Cgame,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub source: RuleSetId,
    pub side: Scope,
    pub role: Role,
    pub dedicated: bool,
    pub seat: qa_core::sys_events::SeatId,
    pub event_time: Option<qa_core::sys_events::EventTime>,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            source: RuleSetId::Quake3,
            side: Scope::Client,
            role: Role::Engine,
            dedicated: false,
            seat: qa_core::sys_events::SeatId::FIRST,
            event_time: None,
        }
    }
}
