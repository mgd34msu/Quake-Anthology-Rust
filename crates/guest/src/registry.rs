//! Game-module registry: registration-order dispatch with invocation
//! tracking. Mirrors the donor `ActorCallbackTable`
//! (`src/world/actors/callbacks.ts`): each invocation pushes
//! `{self, kind, parent}` so nested callbacks observe their chain, dead
//! actors decline dispatch, and dispatch returns whether any module
//! handled the call.

use qa_core::identity::{ActorId, SavedActorId};

use crate::error::GuestError;
use crate::fields::FieldTable;
use crate::traits::{CallbackKind, GameContext, GameEvent, GameModule, GameReaction, GameTouch, UseRequest};

/// Per-module invocation callback.
type ModuleCall<'a> = dyn FnMut(&mut Box<dyn GameModule>, &mut GameContext) -> Result<bool, GuestError> + 'a;

/// Maximum nested dispatch depth.
pub const MAX_DISPATCH_DEPTH: usize = 32;

/// One live invocation on the dispatch stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// Invoked entity.
    pub target: ActorId,
    /// Callback kind.
    pub kind: CallbackKind,
    /// Parent stack depth (0 for outermost).
    pub parent_depth: usize,
}

/// Game-module registry with sync dispatch.
#[derive(Default)]
pub struct ModuleRegistry {
    modules: Vec<Box<dyn GameModule>>,
    stack: Vec<Invocation>,
    closed: bool,
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModuleRegistry")
            .field("modules", &self.module_names())
            .field("depth", &self.stack.len())
            .finish()
    }
}

impl ModuleRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a module; names must be unique.
    pub fn register(&mut self, module: Box<dyn GameModule>) -> Result<(), GuestError> {
        self.assert_open()?;
        if self.modules.iter().any(|known| known.name() == module.name()) {
            return Err(GuestError::DuplicateModule(module.name().to_string()));
        }
        self.modules.push(module);
        Ok(())
    }

    /// Registered module names in dispatch order.
    #[must_use]
    pub fn module_names(&self) -> Vec<String> {
        self.modules.iter().map(|module| module.name().to_string()).collect()
    }

    /// Current invocation, if dispatching.
    #[must_use]
    pub fn current(&self) -> Option<&Invocation> {
        self.stack.last()
    }

    /// Current dispatch depth.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Whether the registry is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Close the registry; further dispatch fails.
    pub fn close(&mut self) {
        self.closed = true;
        self.modules.clear();
        self.stack.clear();
    }

    /// Dispatch spawn in registration order; first handler wins. Spawn
    /// rides the think stack slot: QC spawn functions run before the first
    /// think with the same invocation shape.
    pub fn dispatch_spawn(
        &mut self,
        fields: &mut FieldTable,
        events: &mut Vec<GameEvent>,
        context: &GameDispatchContext,
        actor: &ActorId,
        classname: &str,
        raw: &[(&str, &str)],
    ) -> Result<bool, GuestError> {
        self.invoke(
            fields,
            events,
            context,
            actor,
            CallbackKind::Think,
            &mut |module, context| module.spawn(context, actor, classname, raw),
        )
    }

    /// Dispatch think; first handler wins.
    pub fn dispatch_think(
        &mut self,
        fields: &mut FieldTable,
        events: &mut Vec<GameEvent>,
        context: &GameDispatchContext,
        actor: &ActorId,
        is_live: &dyn Fn(&ActorId) -> bool,
    ) -> Result<bool, GuestError> {
        if !is_live(actor) {
            return Ok(false);
        }
        self.invoke(
            fields,
            events,
            context,
            actor,
            CallbackKind::Think,
            &mut |module, context| module.think(context, actor),
        )
    }

    /// Dispatch touch; first handler wins. Dead `other` actors decline
    /// unless the touch is explicitly inverted (donor canonical rule).
    pub fn dispatch_touch(
        &mut self,
        fields: &mut FieldTable,
        events: &mut Vec<GameEvent>,
        context: &GameDispatchContext,
        contact: &GameTouch,
        is_live: &dyn Fn(&ActorId) -> bool,
        inverted: bool,
    ) -> Result<bool, GuestError> {
        if !is_live(&contact.target) {
            return Ok(false);
        }
        if !is_live(&contact.other) && !inverted {
            return Ok(false);
        }
        self.invoke(
            fields,
            events,
            context,
            &contact.target,
            CallbackKind::Touch,
            &mut |module, context| module.touch(context, contact),
        )
    }

    /// Dispatch use; first handler wins.
    pub fn dispatch_use(
        &mut self,
        fields: &mut FieldTable,
        events: &mut Vec<GameEvent>,
        context: &GameDispatchContext,
        request: UseRequest,
        is_live: &dyn Fn(&ActorId) -> bool,
    ) -> Result<bool, GuestError> {
        if !is_live(request.target) {
            return Ok(false);
        }
        self.invoke(
            fields,
            events,
            context,
            request.target,
            CallbackKind::Use,
            &mut |module, context| module.use_on(context, request.target, request.other, request.activator),
        )
    }

    /// Dispatch pain; first handler wins.
    pub fn dispatch_pain(
        &mut self,
        fields: &mut FieldTable,
        events: &mut Vec<GameEvent>,
        context: &GameDispatchContext,
        reaction: &GameReaction,
        is_live: &dyn Fn(&ActorId) -> bool,
    ) -> Result<bool, GuestError> {
        if !is_live(&reaction.target) {
            return Ok(false);
        }
        self.invoke(
            fields,
            events,
            context,
            &reaction.target,
            CallbackKind::Pain,
            &mut |module, context| module.pain(context, reaction),
        )
    }

    /// Dispatch death; first handler wins.
    pub fn dispatch_die(
        &mut self,
        fields: &mut FieldTable,
        events: &mut Vec<GameEvent>,
        context: &GameDispatchContext,
        reaction: &GameReaction,
        is_live: &dyn Fn(&ActorId) -> bool,
    ) -> Result<bool, GuestError> {
        if !is_live(&reaction.target) {
            return Ok(false);
        }
        self.invoke(
            fields,
            events,
            context,
            &reaction.target,
            CallbackKind::Die,
            &mut |module, context| module.die(context, reaction),
        )
    }

    fn invoke(
        &mut self,
        fields: &mut FieldTable,
        events: &mut Vec<GameEvent>,
        context: &GameDispatchContext,
        target: &ActorId,
        kind: CallbackKind,
        call: &mut ModuleCall,
    ) -> Result<bool, GuestError> {
        self.assert_open()?;
        self.push(target, kind)?;
        // Borrow modules disjointly from the invocation stack.
        let modules = std::mem::take(&mut self.modules);
        let mut modules = modules;
        let mut fields = fields;
        let mut events = events;
        let mut handled = false;
        let mut error = None;
        for module in &mut modules {
            let mut context = GameContext {
                fields,
                events,
                frame: context.frame,
                now: context.now,
            };
            match call(module, &mut context) {
                Ok(true) => {
                    handled = true;
                    fields = context.fields;
                    events = context.events;
                    break;
                }
                Ok(false) => {
                    fields = context.fields;
                    events = context.events;
                }
                Err(failure) => {
                    error = Some(failure);
                    fields = context.fields;
                    events = context.events;
                    break;
                }
            }
        }
        self.modules = modules;
        self.stack.pop();
        if let Some(failure) = error {
            return Err(failure);
        }
        let _ = (fields, events);
        Ok(handled)
    }

    fn push(&mut self, target: &ActorId, kind: CallbackKind) -> Result<(), GuestError> {
        if self.stack.len() >= MAX_DISPATCH_DEPTH {
            return Err(GuestError::DispatchDepth);
        }
        let parent_depth = self.stack.len();
        self.stack.push(Invocation {
            target: target.clone(),
            kind,
            parent_depth,
        });
        Ok(())
    }

    fn assert_open(&self) -> Result<(), GuestError> {
        if self.closed {
            return Err(GuestError::RegistryClosed);
        }
        Ok(())
    }
}

/// Frame context for one dispatch call.
#[derive(Debug, Clone, Copy)]
pub struct GameDispatchContext {
    /// Current frame.
    pub frame: qa_core::time::FrameContext,
    /// Current source time.
    pub now: qa_core::time::SourceTime,
}

/// Saved actor handle helper for game events.
#[must_use]
pub fn saved(actor: &ActorId) -> SavedActorId {
    SavedActorId::from(actor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::{FramePhase, SourceTime};

    struct Probe {
        name: String,
        handle_think: bool,
        thinks: usize,
    }

    impl GameModule for Probe {
        fn name(&self) -> &str {
            &self.name
        }

        fn spawn(
            &mut self,
            _context: &mut GameContext,
            _actor: &ActorId,
            _classname: &str,
            _fields: &[(&str, &str)],
        ) -> Result<bool, GuestError> {
            Ok(false)
        }

        fn think(&mut self, _context: &mut GameContext, _actor: &ActorId) -> Result<bool, GuestError> {
            self.thinks += 1;
            Ok(self.handle_think)
        }

        fn touch(&mut self, _context: &mut GameContext, _contact: &GameTouch) -> Result<bool, GuestError> {
            Ok(self.handle_think)
        }
    }

    fn context() -> GameDispatchContext {
        GameDispatchContext {
            frame: qa_core::time::FrameContext {
                frame: 1,
                time: SourceTime::Milliseconds(50),
                elapsed: SourceTime::Milliseconds(50),
                phase: FramePhase::EntityThink,
            },
            now: SourceTime::Milliseconds(50),
        }
    }

    #[test]
    fn dispatch_first_handler_wins_and_tracks_depth() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut registry = ModuleRegistry::new();
        registry
            .register(Box::new(Probe {
                name: "test:first".to_string(),
                handle_think: false,
                thinks: 0,
            }))
            .unwrap();
        registry
            .register(Box::new(Probe {
                name: "test:second".to_string(),
                handle_think: true,
                thinks: 0,
            }))
            .unwrap();
        assert_eq!(registry.module_names(), vec!["test:first", "test:second"]);
        let mut fields = FieldTable::new();
        let mut events = Vec::new();
        let live = |_: &ActorId| true;
        let handled = registry
            .dispatch_think(&mut fields, &mut events, &context(), &actor, &live)
            .unwrap();
        assert!(handled);
        assert_eq!(registry.depth(), 0);
        assert!(registry.current().is_none());
    }

    #[test]
    fn dead_actors_and_inverted_touch_follow_canonical_rules() {
        let owner = IdentityOwner::create("test").unwrap();
        let target = owner.actor(1, 1);
        let other = owner.actor(2, 1);
        let mut registry = ModuleRegistry::new();
        registry
            .register(Box::new(Probe {
                name: "test:probe".to_string(),
                handle_think: true,
                thinks: 0,
            }))
            .unwrap();
        let mut fields = FieldTable::new();
        let mut events = Vec::new();
        let live_target = |actor: &ActorId| *actor == target;
        let contact = GameTouch {
            target: target.clone(),
            other: other.clone(),
        };
        assert!(!registry
            .dispatch_touch(&mut fields, &mut events, &context(), &contact, &live_target, false)
            .unwrap());
        assert!(registry
            .dispatch_touch(&mut fields, &mut events, &context(), &contact, &live_target, true)
            .unwrap());
        let live_none = |_: &ActorId| false;
        assert!(!registry
            .dispatch_think(&mut fields, &mut events, &context(), &target, &live_none)
            .unwrap());
    }

    #[test]
    fn duplicate_modules_and_closed_registry_rejected() {
        let mut registry = ModuleRegistry::new();
        registry
            .register(Box::new(Probe {
                name: "test:probe".to_string(),
                handle_think: false,
                thinks: 0,
            }))
            .unwrap();
        assert_eq!(
            registry.register(Box::new(Probe {
                name: "test:probe".to_string(),
                handle_think: false,
                thinks: 0,
            })),
            Err(GuestError::DuplicateModule("test:probe".to_string()))
        );
        registry.close();
        assert!(registry.is_closed());
        assert_eq!(
            registry.register(Box::new(Probe {
                name: "test:other".to_string(),
                handle_think: false,
                thinks: 0,
            })),
            Err(GuestError::RegistryClosed)
        );
    }
}
