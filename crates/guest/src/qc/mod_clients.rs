//! QuakeC mod-client bindings: reserved edicts for canonical clients.
//!
//! Ported from `src/compat/qc/mod-clients.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `ModClientServices` mirrors the client services from
//! `src/world/session/mod-clients.ts`; `ModClientApplication` and
//! `client_input_values` mirror the application and validated values from
//! `src/world/session/mod-clients.ts` and
//! `src/world/session/mod-client-input.ts`; `info_value_for_key` mirrors
//! `infoValueForKey` from `src/core/info-string.ts`.
//!
//! Adaptation: the donor subscribes to client events with a closure; this
//! port receives events through `deliver` and input dispatch through
//! `input_open`/`input_invoke`/`input_output`, because a self-referential
//! subscription closure cannot be expressed safely here.

use std::collections::HashMap;

use qa_core::identity::{ActorId, ClientId};
use qa_core::math::Vec3;
use qa_core::time::FrameContext;

use super::mod_provider::{
    ModCallbackInput, ModClientDeclaration, ModClientInput, ModClientInputOutput, ModQcInputOutput, ModRuntimeValue,
    ModSourceCall,
};
use crate::error::GuestError;

/// Admitted client slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcModClientSlot {
    /// Client actor.
    pub actor: ActorId,
    /// Reserved source slot.
    pub slot: u32,
    /// Whether admit calls ran.
    pub admitted: bool,
}

/// Client lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModClientEvent {
    /// Client admitted.
    Admitted {
        /// Client actor.
        actor: ActorId,
    },
    /// Client userinfo changed.
    Userinfo {
        /// Client actor.
        actor: ActorId,
    },
    /// Client is leaving.
    Gone {
        /// Client actor.
        actor: ActorId,
    },
}

/// Canonical client reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModClientRef {
    /// Client handle.
    pub client: ClientId,
    /// Client actor.
    pub actor: ActorId,
}

/// Canonical client services.
pub trait ModClientServices {
    /// Client handle for an actor.
    fn for_actor(&self, actor: &ActorId) -> Option<ClientId>;
    /// Actor for a client handle.
    fn actor(&self, client: &ClientId) -> Option<ActorId>;
    /// Live clients.
    fn clients(&self) -> Vec<ModClientRef>;
    /// Client userinfo string.
    fn userinfo(&self, client: &ClientId) -> String;
    /// Assign a client userinfo string.
    fn set_userinfo(&mut self, client: &ClientId, info: String);
}

/// Client binding release outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientBindingRelease {
    /// Bindings released.
    Released,
    /// Release deferred.
    Deferred,
}

/// Validated client-input value.
#[derive(Debug, Clone, PartialEq)]
pub enum ModInputValue {
    /// Scalar value.
    Float(f64),
    /// Vector value.
    Vector(Vec3),
}

/// Client input application.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientApplication {
    /// Client actor.
    pub actor: ActorId,
    /// Client handle.
    pub client: ClientId,
    /// Validated input values.
    pub values: HashMap<ModClientInput, ModInputValue>,
}

/// Validated input values as callback inputs.
#[must_use]
pub fn client_input_values(application: &ModClientApplication) -> HashMap<ModCallbackInput, ModRuntimeValue> {
    application
        .values
        .iter()
        .map(|(input, value)| {
            let runtime = match value {
                ModInputValue::Float(value) => ModRuntimeValue::Float(*value),
                ModInputValue::Vector(value) => ModRuntimeValue::Vector(*value),
            };
            (input.as_callback_input(), runtime)
        })
        .collect()
}

/// Source dispatch for client lifecycle.
pub trait QcClientDispatch {
    /// Project a client actor into source words.
    fn project(&mut self, actor: &ActorId) -> Result<(), GuestError>;
    /// Release a client actor's bindings.
    fn release(&mut self, actor: &ActorId) -> Result<ClientBindingRelease, GuestError>;
    /// Invoke a source call for a client actor.
    fn invoke(&mut self, call: &ModSourceCall, actor: &ActorId, frame: Option<&FrameContext>) -> Result<(), GuestError>;
    /// Run a client think; defaults to no think.
    fn think(&mut self, _actor: &ActorId, _frame: &FrameContext, _live: &dyn Fn() -> bool) -> Result<(), GuestError> {
        Ok(())
    }
    /// Reserve a client actor; defaults to no reservation.
    fn reserve(&mut self, _actor: &ActorId) -> Result<(), GuestError> {
        Ok(())
    }
    /// Observe admission; defaults to no hook.
    fn on_admitted(&mut self, _actor: &ActorId) -> Result<(), GuestError> {
        Ok(())
    }
    /// Whether a source input owner exists.
    fn has_input_owner(&self) -> bool {
        false
    }
    /// Open a client-input scope; returns its closer.
    fn input_open(&mut self, _application: &ModClientApplication) -> Result<Box<dyn FnOnce()>, GuestError> {
        Err(GuestError::invalid("QuakeC client input requires a source application owner"))
    }
    /// Invoke a client-input call.
    fn input_invoke(&mut self, _call: &ModSourceCall, _application: &ModClientApplication) -> Result<(), GuestError> {
        Err(GuestError::invalid("QuakeC client input requires a source application owner"))
    }
    /// Run source with output capture.
    fn input_output(
        &mut self,
        _outputs: &[ModQcInputOutput],
        _application: &ModClientApplication,
        _run: &mut dyn FnMut(),
    ) -> Result<Vec<ModClientInputOutput>, GuestError> {
        Err(GuestError::invalid("QuakeC client input requires a source application owner"))
    }
}

#[derive(Debug, Clone)]
struct Entry {
    actor: ActorId,
    client: Option<ClientId>,
    slot: u32,
    admitted: bool,
}

/// Client bindings over reserved source slots.
pub struct QcModClientBindings<S, O> {
    services: S,
    ops: O,
    declaration: ModClientDeclaration,
    entries: HashMap<ActorId, Entry>,
    started: bool,
}

impl<S: ModClientServices, O: QcClientDispatch> QcModClientBindings<S, O> {
    /// Build over client services and source dispatch.
    pub fn new(services: S, ops: O, declaration: ModClientDeclaration) -> Self {
        Self { services, ops, declaration, entries: HashMap::new(), started: false }
    }

    /// Borrow the services.
    #[must_use]
    pub fn services(&self) -> &S {
        &self.services
    }

    /// Borrow the services mutably.
    pub fn services_mut(&mut self) -> &mut S {
        &mut self.services
    }

    /// Borrow the dispatch.
    #[must_use]
    pub fn ops(&self) -> &O {
        &self.ops
    }

    /// Whether a client actor is admitted.
    #[must_use]
    pub fn admitted(&self, actor: &ActorId) -> bool {
        self.entries.get(actor).is_some_and(|entry| entry.admitted)
    }

    /// Reserved slot for a client actor, allocating when live.
    pub fn slot(&mut self, actor: &ActorId) -> Result<Option<u32>, GuestError> {
        let client = self.services.for_actor(actor);
        if client.is_none() {
            return Ok(self.entries.get(actor).map(|entry| entry.slot));
        }
        if let Some(entry) = self.entries.get(actor) {
            return Ok(Some(entry.slot));
        }
        let used: Vec<u32> = self.entries.values().map(|entry| entry.slot).collect();
        let mut slot: u32 = 1;
        while used.contains(&slot) {
            slot += 1;
        }
        if slot > self.declaration.maximum.max(0) as u32 {
            return Err(GuestError::invalid("QuakeC component source client capacity exceeded"));
        }
        let client = client.ok_or_else(|| GuestError::invalid("QuakeC component admission requires a live client"))?;
        self.entries.insert(actor.clone(), Entry { actor: actor.clone(), client: Some(client), slot, admitted: false });
        Ok(Some(slot))
    }

    /// Require a live client entry, binding its handle.
    fn require(&mut self, actor: &ActorId) -> Result<Entry, GuestError> {
        let client = self.services.for_actor(actor);
        let entry = self.entries.get(actor).cloned().ok_or_else(|| GuestError::invalid("QuakeC component client identity is no longer live"))?;
        let current = client.ok_or_else(|| GuestError::invalid("QuakeC component client identity is no longer live"))?;
        if entry.client.as_ref().is_some_and(|bound| bound != &current)
            || self.services.actor(&current).as_ref() != Some(actor)
        {
            return Err(GuestError::invalid("QuakeC component client identity is no longer live"));
        }
        if entry.client.is_none() {
            let bound = Entry { client: Some(current), ..entry };
            self.entries.insert(actor.clone(), bound.clone());
            return Ok(bound);
        }
        Ok(entry)
    }

    /// Invoke lifecycle calls for an actor.
    fn invoke(&mut self, calls: &[ModSourceCall], actor: &ActorId) -> Result<(), GuestError> {
        for call in calls {
            self.require(actor)?;
            let call = call.clone();
            self.ops.invoke(&call, actor, None)?;
        }
        Ok(())
    }

    /// Admit a client actor.
    fn admit(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        if self.slot(actor)?.is_none() {
            return Err(GuestError::invalid("QuakeC component admission requires a live client"));
        }
        let entry = self.require(actor)?;
        self.ops.reserve(actor)?;
        self.ops.project(actor)?;
        if !entry.admitted {
            let admit = self.declaration.admit.clone();
            if let Some(stored) = self.entries.get_mut(actor) {
                stored.admitted = true;
            }
            self.invoke(&admit, actor)?;
        }
        self.ops.on_admitted(actor)
    }

    /// Start lifecycle dispatch and admit live clients.
    pub fn start(&mut self) -> Result<(), GuestError> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        let actors: Vec<ActorId> = self.services.clients().into_iter().map(|client| client.actor).collect();
        for actor in actors {
            self.admit(&actor)?;
        }
        if !self.declaration.input.is_empty() && !self.ops.has_input_owner() {
            return Err(GuestError::invalid("QuakeC client input requires a source application owner"));
        }
        Ok(())
    }

    /// Deliver a client lifecycle event.
    pub fn deliver(&mut self, event: &ModClientEvent) -> Result<(), GuestError> {
        match event {
            ModClientEvent::Admitted { actor } => self.admit(actor),
            ModClientEvent::Userinfo { actor } => {
                self.admit(actor)?;
                let calls = self.declaration.userinfo.clone();
                self.invoke(&calls, actor)
            }
            ModClientEvent::Gone { actor } => {
                if !self.entries.contains_key(actor) {
                    return Ok(());
                }
                self.require(actor)?;
                let calls = self.declaration.disconnect.clone();
                self.invoke(&calls, actor)?;
                if self.ops.release(actor)? == ClientBindingRelease::Released {
                    self.entries.remove(actor);
                }
                Ok(())
            }
        }
    }

    /// Read a userinfo key.
    pub fn userinfo(&mut self, actor: &ActorId, key: &str) -> Result<String, GuestError> {
        let entry = self.require(actor)?;
        let client = entry.client.ok_or_else(|| GuestError::invalid("QuakeC component userinfo requires a restored client"))?;
        Ok(info_value_for_key(&self.services.userinfo(&client), key))
    }

    /// Run one reserved slot for a frame; false when the slot is idle.
    pub fn frame(&mut self, slot: u32, frame: &FrameContext) -> Result<bool, GuestError> {
        if slot < 1 || slot > self.declaration.maximum as u32 {
            return Ok(false);
        }
        let entry = match self.entries.values().find(|entry| entry.slot == slot).cloned() {
            Some(entry) => entry,
            None => return Ok(false),
        };
        let calls = self.declaration.frame.clone();
        let Self { services, entries, ops, .. } = self;
        let (services, entries, ops) = (&*services, &*entries, &mut *ops);
        let live = || entry_is_live(services, entries, &entry);
        if live() {
            ops.think(&entry.actor, frame, &live)?;
        }
        for call in &calls {
            if !live() {
                break;
            }
            ops.invoke(call, &entry.actor, Some(frame))?;
        }
        Ok(true)
    }

    /// Assign a userinfo key.
    pub fn set_userinfo(&mut self, actor: &ActorId, key: &str, value: &str) -> Result<(), GuestError> {
        if value.contains(['\\', '\0']) {
            return Err(GuestError::invalid("QuakeC component userinfo field cannot contain a delimiter or NUL"));
        }
        let entry = self.require(actor)?;
        let client = entry.client.ok_or_else(|| GuestError::invalid("QuakeC component userinfo requires a restored client"))?;
        let source = self.services.userinfo(&client);
        let fields: Vec<&str> = source.split('\\').collect();
        let mut result: Vec<&str> = Vec::new();
        let mut index = if fields.first() == Some(&"") { 1 } else { 0 };
        while index + 1 < fields.len() {
            let name = fields[index];
            let text = fields[index + 1];
            if name != key {
                result.push(name);
                result.push(text);
            }
            index += 2;
        }
        if !value.is_empty() {
            result.push(key);
            result.push(value);
        }
        let info = if result.is_empty() { String::new() } else { format!("\\{}", result.join("\\")) };
        self.services.set_userinfo(&client, info);
        Ok(())
    }

    /// Open a client-input scope.
    pub fn input_open(&mut self, application: &ModClientApplication) -> Result<Box<dyn FnOnce()>, GuestError> {
        if !self.require(&application.actor)?.admitted {
            return Err(GuestError::invalid("QuakeC client input requires source admission"));
        }
        self.ops.input_open(application)
    }

    /// Invoke a client-input call.
    pub fn input_invoke(&mut self, call: &ModSourceCall, application: &ModClientApplication) -> Result<(), GuestError> {
        self.require(&application.actor)?;
        let call = call.clone();
        self.ops.input_invoke(&call, application)
    }

    /// Run source with input-output capture.
    pub fn input_output(
        &mut self,
        outputs: &[ModQcInputOutput],
        application: &ModClientApplication,
        run: &mut dyn FnMut(),
    ) -> Result<Vec<ModClientInputOutput>, GuestError> {
        self.require(&application.actor)?;
        let outputs = outputs.to_vec();
        self.ops.input_output(&outputs, application, run)
    }

    /// Checkpoint client slots.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<QcModClientSlot> {
        let mut slots: Vec<QcModClientSlot> = self
            .entries
            .values()
            .map(|entry| QcModClientSlot { actor: entry.actor.clone(), slot: entry.slot, admitted: entry.admitted })
            .collect();
        slots.sort_by_key(|slot| slot.slot);
        slots
    }

    /// Restore client slots.
    pub fn restore(&mut self, entries: &[QcModClientSlot]) -> Result<(), GuestError> {
        self.entries.clear();
        let mut slots = std::collections::HashSet::new();
        for entry in entries {
            if entry.slot < 1
                || entry.slot > self.declaration.maximum as u32
                || !slots.insert(entry.slot)
                || self.entries.contains_key(&entry.actor)
            {
                return Err(GuestError::invalid("Invalid saved QuakeC component client mapping"));
            }
            let client = self.services.for_actor(&entry.actor);
            self.entries.insert(
                entry.actor.clone(),
                Entry { actor: entry.actor.clone(), client, slot: entry.slot, admitted: entry.admitted },
            );
        }
        Ok(())
    }

    /// Forget an actor's entry.
    pub fn forget(&mut self, actor: &ActorId) {
        self.entries.remove(actor);
    }

    /// Close lifecycle dispatch.
    pub fn close(&mut self) {
        self.started = false;
        self.entries.clear();
    }
}

/// Whether an entry still describes a live client.
fn entry_is_live<S: ModClientServices>(services: &S, entries: &HashMap<ActorId, Entry>, entry: &Entry) -> bool {
    let client = services.for_actor(&entry.actor);
    entry.admitted
        && entries.get(&entry.actor).is_some_and(|current| current.slot == entry.slot && current.admitted)
        && client.as_ref().is_some_and(|client| {
            entry.client.as_ref().is_none_or(|bound| bound == client)
                && services.actor(client).as_ref() == Some(&entry.actor)
        })
}

/// Read an info-string value for a key.
#[must_use]
pub fn info_value_for_key(info: &str, key: &str) -> String {
    let fields: Vec<&str> = info.split('\\').collect();
    let mut index = if fields.first() == Some(&"") { 1 } else { 0 };
    while index + 1 < fields.len() {
        if fields[index] == key {
            return fields[index + 1].to_string();
        }
        index += 2;
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::{FramePhase, SourceTime};

    struct FakeServices {
        owner: IdentityOwner,
        by_actor: HashMap<ActorId, ClientId>,
        infos: HashMap<ClientId, String>,
    }

    impl FakeServices {
        fn new() -> Self {
            Self { owner: IdentityOwner::create("clients").unwrap(), by_actor: HashMap::new(), infos: HashMap::new() }
        }

        fn join(&mut self, slot: u32) -> ActorId {
            let actor = self.owner.actor(slot, 1);
            let client = self.owner.client(slot, 1);
            self.by_actor.insert(actor.clone(), client.clone());
            self.infos.insert(client, "\\name\\player".to_string());
            actor
        }
    }

    impl ModClientServices for FakeServices {
        fn for_actor(&self, actor: &ActorId) -> Option<ClientId> {
            self.by_actor.get(actor).cloned()
        }

        fn actor(&self, client: &ClientId) -> Option<ActorId> {
            self.by_actor.iter().find(|(_, bound)| *bound == client).map(|(actor, _)| actor.clone())
        }

        fn clients(&self) -> Vec<ModClientRef> {
            self.by_actor.iter().map(|(actor, client)| ModClientRef { client: client.clone(), actor: actor.clone() }).collect()
        }

        fn userinfo(&self, client: &ClientId) -> String {
            self.infos.get(client).cloned().unwrap_or_default()
        }

        fn set_userinfo(&mut self, client: &ClientId, info: String) {
            self.infos.insert(client.clone(), info);
        }
    }

    #[derive(Default)]
    struct FakeOps {
        events: Vec<String>,
        input_owner: bool,
        deferred: bool,
    }

    impl QcClientDispatch for FakeOps {
        fn project(&mut self, actor: &ActorId) -> Result<(), GuestError> {
            self.events.push(format!("project {}", actor.slot()));
            Ok(())
        }

        fn release(&mut self, actor: &ActorId) -> Result<ClientBindingRelease, GuestError> {
            self.events.push(format!("release {}", actor.slot()));
            Ok(if self.deferred { ClientBindingRelease::Deferred } else { ClientBindingRelease::Released })
        }

        fn invoke(&mut self, call: &ModSourceCall, actor: &ActorId, _frame: Option<&FrameContext>) -> Result<(), GuestError> {
            self.events.push(format!("invoke {} {}", call.function, actor.slot()));
            Ok(())
        }

        fn think(&mut self, actor: &ActorId, _frame: &FrameContext, live: &dyn Fn() -> bool) -> Result<(), GuestError> {
            self.events.push(format!("think {} live={}", actor.slot(), live()));
            Ok(())
        }

        fn reserve(&mut self, actor: &ActorId) -> Result<(), GuestError> {
            self.events.push(format!("reserve {}", actor.slot()));
            Ok(())
        }

        fn on_admitted(&mut self, actor: &ActorId) -> Result<(), GuestError> {
            self.events.push(format!("admitted {}", actor.slot()));
            Ok(())
        }

        fn has_input_owner(&self) -> bool {
            self.input_owner
        }
    }

    fn call(name: &str) -> ModSourceCall {
        ModSourceCall { function: name.to_string(), arguments: vec![], globals: vec![] }
    }

    fn declaration() -> ModClientDeclaration {
        ModClientDeclaration {
            maximum: 4,
            admit: vec![call("admit")],
            userinfo: vec![call("userinfo")],
            disconnect: vec![call("disconnect")],
            frame: vec![call("frame")],
            ..ModClientDeclaration::default()
        }
    }

    fn frame() -> FrameContext {
        FrameContext {
            frame: 1,
            time: SourceTime::Seconds(1.0),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::ClientCommand,
        }
    }

    #[test]
    fn start_admits_and_deliver_disconnects() {
        let mut services = FakeServices::new();
        let actor = services.join(1);
        let mut bindings = QcModClientBindings::new(services, FakeOps::default(), declaration());
        bindings.start().unwrap();
        assert!(bindings.admitted(&actor));
        assert_eq!(bindings.slot(&actor).unwrap(), Some(1));
        assert!(bindings.ops().events.iter().any(|event| event == "invoke admit 1"));
        bindings.deliver(&ModClientEvent::Userinfo { actor: actor.clone() }).unwrap();
        assert!(bindings.ops().events.iter().any(|event| event == "invoke userinfo 1"));
        bindings.deliver(&ModClientEvent::Gone { actor: actor.clone() }).unwrap();
        assert!(bindings.ops().events.iter().any(|event| event == "invoke disconnect 1"));
        assert!(!bindings.admitted(&actor));
    }

    #[test]
    fn deferred_release_keeps_entry() {
        let mut services = FakeServices::new();
        let actor = services.join(1);
        let mut bindings = QcModClientBindings::new(services, FakeOps { deferred: true, ..FakeOps::default() }, declaration());
        bindings.start().unwrap();
        bindings.deliver(&ModClientEvent::Gone { actor: actor.clone() }).unwrap();
        assert!(bindings.admitted(&actor));
    }

    #[test]
    fn capacity_is_enforced() {
        let mut bindings = QcModClientBindings::new(
            FakeServices::new(),
            FakeOps::default(),
            ModClientDeclaration { maximum: 1, ..ModClientDeclaration::default() },
        );
        let actor = bindings.services_mut().join(1);
        bindings.deliver(&ModClientEvent::Admitted { actor: actor.clone() }).unwrap();
        assert_eq!(bindings.slot(&actor).unwrap(), Some(1));
        let other = bindings.services_mut().join(2);
        assert!(bindings.deliver(&ModClientEvent::Admitted { actor: other }).is_err());
    }

    #[test]
    fn frame_runs_think_and_calls() {
        let mut services = FakeServices::new();
        let actor = services.join(1);
        let mut bindings = QcModClientBindings::new(services, FakeOps::default(), declaration());
        bindings.start().unwrap();
        assert!(bindings.frame(1, &frame()).unwrap());
        assert!(bindings.ops().events.iter().any(|event| event == "think 1 live=true"));
        assert!(bindings.ops().events.iter().any(|event| event == "invoke frame 1"));
        assert!(!bindings.frame(2, &frame()).unwrap());
        assert!(!bindings.frame(99, &frame()).unwrap());
    }

    #[test]
    fn userinfo_round_trip() {
        let mut services = FakeServices::new();
        let actor = services.join(1);
        let mut bindings = QcModClientBindings::new(services, FakeOps::default(), declaration());
        bindings.start().unwrap();
        assert_eq!(bindings.userinfo(&actor, "name").unwrap(), "player");
        bindings.set_userinfo(&actor, "team", "red").unwrap();
        assert_eq!(bindings.userinfo(&actor, "team").unwrap(), "red");
        bindings.set_userinfo(&actor, "team", "").unwrap();
        assert_eq!(bindings.userinfo(&actor, "team").unwrap(), "");
        assert!(bindings.set_userinfo(&actor, "team", "a\\b").is_err());
        assert_eq!(info_value_for_key("", "name"), "");
    }

    #[test]
    fn checkpoint_restore_round_trip() {
        let mut services = FakeServices::new();
        let actor = services.join(1);
        let mut bindings = QcModClientBindings::new(services, FakeOps::default(), declaration());
        bindings.start().unwrap();
        let saved = bindings.checkpoint();
        assert_eq!(saved, vec![QcModClientSlot { actor: actor.clone(), slot: 1, admitted: true }]);
        let mut revived = QcModClientBindings::new(FakeServices::new(), FakeOps::default(), declaration());
        let joined = revived.services_mut().join(1);
        assert_eq!(joined.slot(), actor.slot());
        revived.restore(&[QcModClientSlot { actor: joined.clone(), slot: 1, admitted: true }]).unwrap();
        assert!(revived.admitted(&joined));
        assert!(revived.restore(&[QcModClientSlot { actor: joined, slot: 9, admitted: true }]).is_err());
    }

    #[test]
    fn input_requires_owner() {
        use super::super::mod_provider::{ModClientInputBinding, ModInputPhase, ModInputScope};
        let mut declaration = declaration();
        declaration.input.push(ModClientInputBinding {
            scope: ModInputScope::ClientCommand,
            phase: ModInputPhase::Before,
            calls: vec![call("input")],
            outputs: vec![],
        });
        let mut bindings = QcModClientBindings::new(FakeServices::new(), FakeOps::default(), declaration);
        assert!(bindings.start().is_err());
    }

    #[test]
    fn input_values_widen() {
        let owner = IdentityOwner::create("input").unwrap();
        let application = ModClientApplication {
            actor: owner.actor(1, 1),
            client: owner.client(1, 1),
            values: [(ModClientInput::Attack, ModInputValue::Float(1.0))].into_iter().collect(),
        };
        let values = client_input_values(&application);
        assert_eq!(values.get(&ModCallbackInput::Attack), Some(&ModRuntimeValue::Float(1.0)));
    }
}
