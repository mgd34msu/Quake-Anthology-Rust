//! Named Q1 source continuations (`src/content/q1/foundation/callbacks.ts`).
//!
//! Named QC continuations. Only names are saved; handlers are
//! registered by source modules.
//!
//! Handler functions receive the game plus the subject actor id and
//! resolve live records through the game; this keeps the donor's
//! dispatch order while satisfying Rust borrowing.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::entity_services::Q1EntityServices;
use super::gameplay::TouchSurface;
use super::host::Q1TrajectoryUpdate;
use crate::q1::{q1_error, Q1Error};

/// Named source action handler.
pub type Q1ActionHandler = fn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error>;
/// Named source use handler.
pub type Q1UseHandler = fn(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error>;
/// Named source touch handler.
pub type Q1TouchHandler = fn(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    normal: Option<Vec3>,
    surface: Option<&TouchSurface>,
) -> Result<(), Q1Error>;
/// Named source pain handler.
pub type Q1PainHandler =
    fn(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error>;
/// Named source death handler.
pub type Q1DieHandler =
    fn(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error>;
/// Named source blocked handler.
pub type Q1BlockedHandler = fn(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error>;
/// Named projectile trajectory handler.
pub type Q1TrajectoryHandler =
    fn(game: &mut Q1EntityServices, id: &ActorId, update: &Q1TrajectoryUpdate) -> Result<(), Q1Error>;

/// Named callback handlers (`Q1CallbackHandlers`).
#[derive(Debug, Default)]
pub struct Q1CallbackHandlers {
    /// Trajectory projection handler.
    pub trajectory: Option<Q1TrajectoryHandler>,
    /// Scheduled action handler.
    pub action: Option<Q1ActionHandler>,
    /// Use handler.
    pub use_callback: Option<Q1UseHandler>,
    /// Touch handler.
    pub touch: Option<Q1TouchHandler>,
    /// Pain handler.
    pub pain: Option<Q1PainHandler>,
    /// Death handler.
    pub die: Option<Q1DieHandler>,
    /// Blocked handler.
    pub blocked: Option<Q1BlockedHandler>,
}

/// Source state extension (`Q1StateExtension`).
pub trait Q1StateExtension {
    /// Extension id.
    fn id(&self) -> &str;
    /// Capture extension bytes.
    fn capture(&self) -> Vec<u8>;
    /// Restore extension bytes after all entity/player references exist,
    /// before thinks are scheduled.
    fn restore(&mut self, bytes: &[u8]) -> Result<(), Q1Error>;
    /// Duplicate initialized source state for a cloned entity without
    /// running a spawn function (`SUB_CopyEntity` semantics).
    fn clone_state(
        &mut self,
        _game: &mut Q1EntityServices,
        _source: &ActorId,
        _target: &ActorId,
    ) -> Result<(), Q1Error> {
        Ok(())
    }
}

/// Saved callback name (`callbackName`). Entities store names directly,
/// so this only clones the stored value.
#[must_use]
pub fn callback_name(callback: Option<&String>) -> Option<String> {
    callback.cloned()
}

/// Named callback registry (`Q1CallbackRegistry`).
#[derive(Debug, Default)]
pub struct Q1CallbackRegistry {
    handlers: HashMap<String, Q1CallbackHandlers>,
}

impl Q1CallbackRegistry {
    /// Fresh registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a named handler bundle.
    pub fn register(&mut self, name: &str, handlers: Q1CallbackHandlers) -> Result<(), Q1Error> {
        if self.handlers.contains_key(name) {
            return Err(q1_error(format!("Duplicate Q1 callback: {name}")));
        }
        self.handlers.insert(name.to_string(), handlers);
        Ok(())
    }

    /// Look up a registered bundle.
    fn get(&self, name: &str) -> Result<&Q1CallbackHandlers, Q1Error> {
        self.handlers
            .get(name)
            .ok_or_else(|| q1_error(format!("Unknown Q1 saved callback: {name}")))
    }

    /// Validate that a name resolves to an action handler, returning
    /// the name for storage (`named.action`).
    pub fn action(&self, name: &str) -> Result<String, Q1Error> {
        if self.get(name)?.action.is_none() {
            return Err(q1_error(format!("Q1 callback is not an action: {name}")));
        }
        Ok(name.to_string())
    }

    /// Validate that a name resolves to a use handler, returning the
    /// name for storage (`named.use`).
    pub fn use_callback(&self, name: &str) -> Result<String, Q1Error> {
        if self.get(name)?.use_callback.is_none() {
            return Err(q1_error(format!("Q1 callback is not use: {name}")));
        }
        Ok(name.to_string())
    }

    /// Validate that a name resolves to a touch handler, returning the
    /// name for storage (`named.touch`).
    pub fn touch(&self, name: &str) -> Result<String, Q1Error> {
        if self.get(name)?.touch.is_none() {
            return Err(q1_error(format!("Q1 callback is not touch: {name}")));
        }
        Ok(name.to_string())
    }

    /// Validate that a name resolves to a pain handler, returning the
    /// name for storage (`named.pain`).
    pub fn pain(&self, name: &str) -> Result<String, Q1Error> {
        if self.get(name)?.pain.is_none() {
            return Err(q1_error(format!("Q1 callback is not pain: {name}")));
        }
        Ok(name.to_string())
    }

    /// Validate that a name resolves to a death handler, returning the
    /// name for storage (`named.die`).
    pub fn die(&self, name: &str) -> Result<String, Q1Error> {
        if self.get(name)?.die.is_none() {
            return Err(q1_error(format!("Q1 callback is not die: {name}")));
        }
        Ok(name.to_string())
    }

    /// Validate that a name resolves to a blocked handler, returning
    /// the name for storage (`named.blocked`).
    pub fn blocked(&self, name: &str) -> Result<String, Q1Error> {
        if self.get(name)?.blocked.is_none() {
            return Err(q1_error(format!("Q1 callback is not blocked: {name}")));
        }
        Ok(name.to_string())
    }

    /// Fetch a stored trajectory handler by name, if the bundle
    /// defines one.
    pub(crate) fn trajectory_handler(&self, name: &str) -> Result<Option<Q1TrajectoryHandler>, Q1Error> {
        Ok(self.get(name)?.trajectory)
    }

    /// Fetch a stored action handler by name. Handlers are plain
    /// function pointers so dispatch can copy them out before
    /// re-borrowing the game.
    pub(crate) fn action_handler(&self, name: &str) -> Result<Q1ActionHandler, Q1Error> {
        self.get(name)?
            .action
            .ok_or_else(|| q1_error(format!("Q1 callback is not an action: {name}")))
    }

    /// Fetch a stored use handler by name.
    pub(crate) fn use_handler(&self, name: &str) -> Result<Q1UseHandler, Q1Error> {
        self.get(name)?
            .use_callback
            .ok_or_else(|| q1_error(format!("Q1 callback is not use: {name}")))
    }

    /// Fetch a stored touch handler by name.
    pub(crate) fn touch_handler(&self, name: &str) -> Result<Q1TouchHandler, Q1Error> {
        self.get(name)?
            .touch
            .ok_or_else(|| q1_error(format!("Q1 callback is not touch: {name}")))
    }

    /// Fetch a stored pain handler by name.
    pub(crate) fn pain_handler(&self, name: &str) -> Result<Q1PainHandler, Q1Error> {
        self.get(name)?
            .pain
            .ok_or_else(|| q1_error(format!("Q1 callback is not pain: {name}")))
    }

    /// Fetch a stored death handler by name.
    pub(crate) fn die_handler(&self, name: &str) -> Result<Q1DieHandler, Q1Error> {
        self.get(name)?
            .die
            .ok_or_else(|| q1_error(format!("Q1 callback is not die: {name}")))
    }

    /// Fetch a stored blocked handler by name.
    pub(crate) fn blocked_handler(&self, name: &str) -> Result<Q1BlockedHandler, Q1Error> {
        self.get(name)?
            .blocked
            .ok_or_else(|| q1_error(format!("Q1 callback is not blocked: {name}")))
    }
}

/// Entity callback slot selected for name storage lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1CallbackSlot {
    /// Think slot.
    Think,
    /// Use slot.
    Use,
    /// Touch slot.
    Touch,
    /// Pain slot.
    Pain,
    /// Death slot.
    Die,
    /// Blocked slot.
    Blocked,
    /// Path-end slot.
    PathEnd,
}
