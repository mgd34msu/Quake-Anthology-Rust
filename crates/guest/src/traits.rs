//! Game-module interface: the QC-style gamecode contract as idiomatic
//! Rust traits. Donor `ActorCallbacks` (`src/contracts/world.ts`: sync
//! `think`/`touch`/`use`/`pain`/`die`, never promises) plus the spawn,
//! field-access, and save/restore surface the engine offers game modules
//! (`src/compat/qc/entity-host.ts`, `src/compat/qc/builtins.ts`).

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;
use qa_core::time::{FrameContext, SourceTime};

use crate::error::GuestError;
use crate::fields::{FieldLayout, FieldTable, FieldValue};

/// Callback kind for invocation tracking (donor `ActorInvocation.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackKind {
    /// Periodic think.
    Think,
    /// Trigger or entity touch.
    Touch,
    /// Use on another entity.
    Use,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Die,
}

/// Touch contact delivered to game modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameTouch {
    /// Touched entity.
    pub target: ActorId,
    /// Touching entity.
    pub other: ActorId,
}

/// Use request delivered to game modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UseRequest<'a> {
    /// Used entity.
    pub target: &'a ActorId,
    /// Other entity, if any.
    pub other: Option<&'a ActorId>,
    /// Activator, if any.
    pub activator: Option<&'a ActorId>,
}

/// Pain/death reaction delivered to game modules.
#[derive(Debug, Clone, PartialEq)]
pub struct GameReaction {
    /// Reacting entity.
    pub target: ActorId,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Knockback kick.
    pub kick: f64,
    /// Damage applied.
    pub damage: f64,
    /// Death inflictor (die only).
    pub inflictor: Option<ActorId>,
    /// Death location (die only).
    pub point: Option<Vec3>,
}

/// Game-module event sink entries.
#[derive(Debug, Clone, PartialEq)]
pub enum GameEvent {
    /// Entity spawned.
    Spawned {
        /// Spawned actor.
        actor: SavedActorId,
        /// Classname.
        classname: String,
    },
    /// Think ran.
    Think {
        /// Thinking actor.
        actor: SavedActorId,
    },
    /// Message from gamecode.
    Message(String),
}

/// Engine services offered to one game module invocation.
pub struct GameContext<'a> {
    /// Entity fields.
    pub fields: &'a mut FieldTable,
    /// Pending events.
    pub events: &'a mut Vec<GameEvent>,
    /// Current frame.
    pub frame: FrameContext,
    /// Current source time.
    pub now: SourceTime,
}

impl GameContext<'_> {
    /// Read an entity field.
    pub fn field(&self, actor: &ActorId, name: &str) -> Result<&FieldValue, GuestError> {
        self.fields.get(actor, name)
    }

    /// Write an entity field.
    pub fn set_field(&mut self, actor: &ActorId, name: &str, value: FieldValue) -> Result<(), GuestError> {
        self.fields.set(actor, name, value)
    }

    /// Emit a game event.
    pub fn emit(&mut self, event: GameEvent) {
        self.events.push(event);
    }
}

/// QC-style game module: spawn/think/touch/use/pain/die dispatch over
/// named entity fields. All callbacks are synchronous and return whether
/// they handled the invocation (donor `ModOperation` composition).
pub trait GameModule {
    /// Module name (`namespace:name`).
    fn name(&self) -> &str;

    /// Entity field layout this module allocates.
    fn layout(&self) -> FieldLayout {
        FieldLayout::qc_entity()
    }

    /// Spawn an entity of `classname` with raw fields.
    fn spawn(
        &mut self,
        context: &mut GameContext,
        actor: &ActorId,
        classname: &str,
        fields: &[(&str, &str)],
    ) -> Result<bool, GuestError>;

    /// Periodic think.
    fn think(&mut self, _context: &mut GameContext, _actor: &ActorId) -> Result<bool, GuestError> {
        Ok(false)
    }

    /// Touch dispatch.
    fn touch(&mut self, _context: &mut GameContext, _contact: &GameTouch) -> Result<bool, GuestError> {
        Ok(false)
    }

    /// Use dispatch.
    fn use_on(
        &mut self,
        _context: &mut GameContext,
        _target: &ActorId,
        _other: Option<&ActorId>,
        _activator: Option<&ActorId>,
    ) -> Result<bool, GuestError> {
        Ok(false)
    }

    /// Pain reaction.
    fn pain(&mut self, _context: &mut GameContext, _reaction: &GameReaction) -> Result<bool, GuestError> {
        Ok(false)
    }

    /// Death reaction.
    fn die(&mut self, _context: &mut GameContext, _reaction: &GameReaction) -> Result<bool, GuestError> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_core::time::FramePhase;

    struct Echo {
        handled: bool,
    }

    impl GameModule for Echo {
        fn name(&self) -> &str {
            "test:echo"
        }

        fn spawn(
            &mut self,
            context: &mut GameContext,
            actor: &ActorId,
            classname: &str,
            _fields: &[(&str, &str)],
        ) -> Result<bool, GuestError> {
            context.fields.allocate(actor, &Self::qc_layout())?;
            context.set_field(actor, "origin", FieldValue::Vector(vec3(1.0, 0.0, 0.0)))?;
            context.emit(GameEvent::Spawned {
                actor: SavedActorId::from(actor),
                classname: classname.to_string(),
            });
            Ok(self.handled)
        }
    }

    impl Echo {
        fn qc_layout() -> FieldLayout {
            FieldLayout::qc_entity()
        }
    }

    #[test]
    fn module_spawn_allocates_fields_and_emits() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut fields = FieldTable::new();
        let mut events = Vec::new();
        let frame = FrameContext {
            frame: 0,
            time: SourceTime::Seconds(0.0),
            elapsed: SourceTime::Seconds(0.0),
            phase: FramePhase::FrameEntry,
        };
        let mut module = Echo { handled: true };
        let handled = module
            .spawn(
                &mut GameContext {
                    fields: &mut fields,
                    events: &mut events,
                    frame,
                    now: SourceTime::Seconds(0.0),
                },
                &actor,
                "test:thing",
                &[],
            )
            .unwrap();
        assert!(handled);
        assert!(fields.is_allocated(&actor));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], GameEvent::Think { .. } | GameEvent::Spawned { .. }));
    }

    #[test]
    fn default_callbacks_decline() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut fields = FieldTable::new();
        let mut events = Vec::new();
        let frame = FrameContext {
            frame: 0,
            time: SourceTime::Seconds(0.0),
            elapsed: SourceTime::Seconds(0.0),
            phase: FramePhase::FrameEntry,
        };
        let mut context = GameContext {
            fields: &mut fields,
            events: &mut events,
            frame,
            now: SourceTime::Seconds(0.0),
        };
        let mut module = Echo { handled: false };
        assert!(!module.think(&mut context, &actor).unwrap());
    }
}
