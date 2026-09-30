//! Q2 monster runner (`src/content/q2/foundation/monsters`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

/// Arena runtime state for this module.
#[derive(Debug, Default)]
pub struct MonsterRuntime;

impl MonsterRuntime {
    /// Drop monster state after an actor release.
    pub fn on_actor_released(&mut self, _actor: &qa_core::identity::ActorId) {
    }
}
