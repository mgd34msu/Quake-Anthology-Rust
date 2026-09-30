//! Monster perception state (`src/content/q2/foundation/monsters/perception.ts`).

/// Monster perception arena runtime.
#[derive(Debug, Default)]
pub struct PerceptionRuntime;

impl PerceptionRuntime {
    /// Drop perception state after an actor release.
    pub fn release(&mut self, _actor: &qa_core::identity::ActorId) {
    }
}
