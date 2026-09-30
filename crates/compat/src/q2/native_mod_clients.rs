//! Port of `src/compat/q2/native-mod-clients.ts`.
//! Bridges component clients: source row numbers stay private while admission,
//! frames, commands, and input dispatch resolve live destination generations.

use std::collections::{HashMap, HashSet};

use thiserror::Error;

/// Failures in the native client bridge.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClientError {
    /// A client identity is no longer live.
    #[error("native component client identity is no longer live")]
    IdentityStale,
    /// A rejected client lost its reserved source row.
    #[error("rejected native client lost its reserved source row")]
    ReservedRowLost,
    /// Source client capacity is exceeded.
    #[error("native component source client capacity exceeded")]
    CapacityExceeded,
    /// Admission needs a live client.
    #[error("native component admission requires a live client")]
    NoLiveClient,
    /// A native command names no live client.
    #[error("native component command client is no longer live")]
    CommandClientGone,
    /// A pickup recipient is retired (input dispatch raced removal).
    #[error("native client recipient is retired")]
    RecipientRetired,
    /// A save cannot capture a rejected-but-connected client.
    #[error("cannot save a native component before its rejected client disconnects")]
    PendingRejection,
    /// A saved client is unavailable.
    #[error("saved native component client is unavailable")]
    SavedUnavailable,
    /// Cleanup aggregated failures.
    #[error("native component client cleanup failed: {0:?}")]
    Cleanup(Vec<String>),
}

/// Generational actor handle local to the client bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeActorId {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Generational client handle local to the client bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeClientId {
    /// Client slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Client lifecycle event (polled; the donor subscribes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    /// A client was admitted.
    Admitted {
        /// Admitted actor.
        actor: NativeActorId,
    },
    /// A userinfo string changed.
    Userinfo {
        /// Affected actor.
        actor: NativeActorId,
    },
    /// A client left.
    Removed {
        /// Removed actor.
        actor: NativeActorId,
    },
}

/// Destination client directory surface.
pub trait ClientDirectory {
    /// Client handle for an actor, if connected.
    fn for_actor(&self, actor: NativeActorId) -> Option<NativeClientId>;
    /// Actor for a client handle, if bound.
    fn actor_of(&self, client: NativeClientId) -> Option<NativeActorId>;
    /// All connected client identities.
    fn clients(&self) -> Vec<(NativeActorId, NativeClientId)>;
    /// Userinfo string for a client.
    fn userinfo(&self, client: NativeClientId) -> String;
    /// Store a userinfo string for a client.
    fn set_userinfo(&mut self, client: NativeClientId, value: String);
    /// Drop a client with a reason.
    fn drop_client(&mut self, client: NativeClientId, reason: String);
    /// Drain pending lifecycle events.
    fn take_events(&mut self) -> Vec<ClientEvent>;
}

/// How a source call result is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallAccepts {
    /// Any result (including void-equivalent zero) accepts.
    Always,
    /// Zero rejects.
    NonZero,
}

/// Declared source call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCallDecl {
    /// Call id.
    pub id: String,
    /// Acceptance rule.
    pub accepts: CallAccepts,
}

/// Declared input binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBindingDecl {
    /// Binding id.
    pub id: String,
    /// Calls to run for the binding.
    pub calls: Vec<SourceCallDecl>,
}

/// Declared component clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientsDeclaration {
    /// Maximum source rows.
    pub maximum: usize,
    /// Admission calls.
    pub admit: Vec<SourceCallDecl>,
    /// Userinfo calls.
    pub userinfo: Vec<SourceCallDecl>,
    /// Disconnect calls.
    pub disconnect: Vec<SourceCallDecl>,
    /// Command calls.
    pub command: Vec<SourceCallDecl>,
    /// Per-frame calls.
    pub frame: Vec<SourceCallDecl>,
    /// End-of-frame calls.
    pub end_frame: Vec<SourceCallDecl>,
    /// Input bindings.
    pub input: Vec<InputBindingDecl>,
}

/// Client input application for one dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientApplication {
    /// Acting actor.
    pub actor: NativeActorId,
    /// Acting client.
    pub client: NativeClientId,
}

/// Client input consumption or update produced by source outputs.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientInputOutput {
    /// Consume inputs.
    Consume {
        /// Consumed inputs.
        inputs: Vec<String>,
    },
    /// Set an input value.
    Set {
        /// Input name.
        input: String,
        /// New value.
        value: f64,
    },
}

/// Command origin chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOrigin {
    /// Local seat command.
    LocalSeat {
        /// Issuing client.
        client: NativeClientId,
    },
    /// Remote client command.
    RemoteCommand {
        /// Issuing client.
        client: NativeClientId,
    },
    /// Script command with a caller origin.
    Script {
        /// Caller origin.
        caller: Box<CommandOrigin>,
    },
    /// Any other origin.
    Other,
}

/// Command invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInvocation {
    /// Argument vector.
    pub argv: Vec<String>,
    /// Command origin.
    pub origin: CommandOrigin,
}

/// Component client operations: projection plus source execution.
pub trait ClientOperations {
    /// Directory type.
    type Directory: ClientDirectory;

    /// Mutable directory access.
    fn directory(&mut self) -> &mut Self::Directory;
    /// Shared directory access.
    fn directory_ref(&self) -> &Self::Directory;
    /// Client declaration.
    fn declaration(&self) -> &ClientsDeclaration;
    /// Project a client actor into source rows.
    fn project(&mut self, actor: NativeActorId);
    /// Hook after admission completes.
    fn admitted_hook(&mut self, actor: NativeActorId);
    /// Release a client actor.
    fn release(&mut self, actor: NativeActorId) -> Result<(), ClientError>;
    /// Invoke a source call; `None` means retired.
    fn invoke(&mut self, call: &SourceCallDecl, actor: NativeActorId) -> Option<i32>;
    /// Open an input application; returns a session token.
    fn open_input(&mut self, application: &ClientApplication) -> u64;
    /// Close an input session token.
    fn close_input(&mut self, token: u64);
    /// Invoke an input call for an application.
    fn invoke_input(&mut self, call: &SourceCallDecl, application: &ClientApplication);
    /// Run source outputs around a closure.
    fn input_output<R>(
        &mut self,
        outputs: &[String],
        application: &ClientApplication,
        run: impl FnOnce(&mut Self) -> R,
    ) -> (R, Vec<ClientInputOutput>);
    /// Establish a command; returns a session token.
    fn begin_command(&mut self, command: &CommandInvocation) -> u64;
    /// Release a command session token.
    fn end_command(&mut self, token: u64);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    client: NativeClientId,
    slot: usize,
    admitted: bool,
}

/// Saved client slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeModClientSlot {
    /// Client actor.
    pub actor: NativeActorId,
    /// Source slot.
    pub slot: usize,
    /// Whether admission completed.
    pub admitted: bool,
}

/// Read one `\key\value` userinfo field.
pub fn userinfo_value(info: &str, key: &str) -> Option<String> {
    let mut parts = info.split('\\').filter(|part| !part.is_empty());
    while let (Some(name), Some(value)) = (parts.next(), parts.next()) {
        if name == key {
            return Some(value.to_string());
        }
    }
    None
}

/// Component client binding over synthetic operations.
pub struct NativeModClientsBinding<O: ClientOperations> {
    operations: O,
    entries: HashMap<NativeActorId, Entry>,
    denied: HashSet<NativeActorId>,
    started: bool,
}

impl<O: ClientOperations> NativeModClientsBinding<O> {
    /// Build the binding.
    pub fn new(operations: O) -> Self {
        Self {
            operations,
            entries: HashMap::new(),
            denied: HashSet::new(),
            started: false,
        }
    }

    /// Borrow the operations (for fixtures).
    #[must_use]
    pub fn operations(&self) -> &O {
        &self.operations
    }

    /// Mutably borrow the operations (for fixtures).
    pub fn operations_mut(&mut self) -> &mut O {
        &mut self.operations
    }

    fn require(&self, actor: NativeActorId) -> Result<&Entry, ClientError> {
        let entry = self.entries.get(&actor).ok_or(ClientError::IdentityStale)?;
        let directory = self.operations.directory_ref();
        let client = directory.for_actor(actor).ok_or(ClientError::IdentityStale)?;
        if client != entry.client || directory.actor_of(client) != Some(actor) {
            return Err(ClientError::IdentityStale);
        }
        Ok(entry)
    }

    fn require_mut(&mut self, actor: NativeActorId) -> Result<&mut Entry, ClientError> {
        if !self.entries.contains_key(&actor) {
            return Err(ClientError::IdentityStale);
        }
        let entry = self.entries.get(&actor).ok_or(ClientError::IdentityStale)?;
        let directory = self.operations.directory();
        let client = directory.for_actor(actor).ok_or(ClientError::IdentityStale)?;
        if client != entry.client || directory.actor_of(client) != Some(actor) {
            return Err(ClientError::IdentityStale);
        }
        self.entries.get_mut(&actor).ok_or(ClientError::IdentityStale)
    }

    /// Source slot for an actor, reserving a row on first use.
    pub fn slot(&mut self, actor: NativeActorId) -> Result<Option<usize>, ClientError> {
        if self.entries.contains_key(&actor) {
            return Ok(Some(self.require(actor)?.slot));
        }
        if self.denied.contains(&actor) {
            return Err(ClientError::ReservedRowLost);
        }
        let client = self.operations.directory().for_actor(actor);
        let Some(client) = client else {
            return Ok(None);
        };
        if self.operations.directory().actor_of(client) != Some(actor) {
            return Err(ClientError::IdentityStale);
        }
        let occupied: HashSet<usize> = self.entries.values().map(|entry| entry.slot).collect();
        let mut slot = 0;
        while occupied.contains(&slot) {
            slot += 1;
        }
        if slot >= self.operations.declaration().maximum {
            return Err(ClientError::CapacityExceeded);
        }
        self.entries.insert(
            actor,
            Entry {
                client,
                slot,
                admitted: false,
            },
        );
        Ok(Some(slot))
    }

    /// Whether an actor holds a source row.
    #[must_use]
    pub fn has(&self, actor: NativeActorId) -> bool {
        self.entries.contains_key(&actor)
    }

    /// Whether an actor completed admission.
    pub fn admitted(&self, actor: NativeActorId) -> bool {
        self.entries.contains_key(&actor)
            && self.require(actor).is_ok_and(|entry| entry.admitted)
            && !self.denied.contains(&actor)
    }

    /// Whether an actor was rejected.
    #[must_use]
    pub fn rejects(&self, actor: NativeActorId) -> bool {
        self.denied.contains(&actor)
    }

    fn calls(&mut self, calls: &[SourceCallDecl], actor: NativeActorId) -> Result<(), ClientError> {
        let expected = self.require(actor)?.clone();
        for call in calls {
            if self.entries.get(&actor) != Some(&expected) {
                break;
            }
            self.require(actor)?;
            let call = call.clone();
            if self.operations.invoke(&call, actor).is_none() {
                break;
            }
        }
        Ok(())
    }

    fn admit(&mut self, actor: NativeActorId) -> Result<bool, ClientError> {
        if self.denied.contains(&actor) {
            return Ok(false);
        }
        if self.slot(actor)?.is_none() {
            return Err(ClientError::NoLiveClient);
        }
        let entry = self.require(actor)?.clone();
        self.operations.project(actor);
        if entry.admitted {
            return Ok(true);
        }
        let admit = self.operations.declaration().admit.clone();
        let mut rejected = false;
        let mut failed: Option<ClientError> = None;
        for call in &admit {
            if self.require(actor).is_err() {
                failed = Some(ClientError::IdentityStale);
                break;
            }
            let result = self.operations.invoke(call, actor);
            let Some(result) = result else {
                return Ok(false);
            };
            if call.accepts == CallAccepts::NonZero && result == 0 {
                rejected = true;
                break;
            }
        }
        if rejected {
            let info = self.userinfo(actor)?;
            let reason = userinfo_value(&info, "rejmsg").unwrap_or_else(|| "Connection refused".to_string());
            let client = self.require(actor)?.client;
            self.operations.directory().drop_client(client, reason);
            self.denied.insert(actor);
            return Ok(false);
        }
        if let Some(error) = failed {
            self.operations.release(actor).ok();
            self.entries.remove(&actor);
            return Err(error);
        }
        self.require_mut(actor)?.admitted = true;
        self.operations.admitted_hook(actor);
        Ok(true)
    }

    /// Start dispatch: admit connected clients; poll with [`Self::poll`].
    pub fn start(&mut self) -> Result<(), ClientError> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        let clients: Vec<NativeActorId> = self
            .operations
            .directory()
            .clients()
            .iter()
            .map(|(actor, _)| *actor)
            .collect();
        for actor in clients {
            self.admit(actor)?;
        }
        Ok(())
    }

    /// Drain directory lifecycle events.
    pub fn poll(&mut self) -> Result<(), ClientError> {
        for event in self.operations.directory().take_events() {
            match event {
                ClientEvent::Admitted { actor } => {
                    self.admit(actor)?;
                }
                ClientEvent::Userinfo { actor } => {
                    if self.admit(actor)? {
                        let calls = self.operations.declaration().userinfo.clone();
                        self.calls(&calls, actor)?;
                    }
                }
                ClientEvent::Removed { actor } => {
                    if self.entries.contains_key(&actor) {
                        self.disconnect(actor)?;
                    }
                    self.denied.remove(&actor);
                }
            }
        }
        Ok(())
    }

    /// Dispatch one input binding call for an application.
    pub fn dispatch_input(
        &mut self,
        binding: &InputBindingDecl,
        application: &ClientApplication,
        call: &SourceCallDecl,
    ) -> Result<(), ClientError> {
        let actor = application.actor;
        if !self.admitted(actor) || self.denied.contains(&actor) {
            return Ok(());
        }
        if !self
            .operations
            .declaration()
            .input
            .iter()
            .any(|candidate| candidate.id == binding.id)
        {
            return Ok(());
        }
        let token = self.operations.open_input(application);
        self.operations.invoke_input(call, application);
        self.operations.close_input(token);
        Ok(())
    }

    /// Run source outputs around a closure for an application.
    pub fn run_input_outputs<R>(
        &mut self,
        outputs: &[String],
        application: &ClientApplication,
        run: impl FnOnce(&mut O) -> R,
    ) -> (R, Vec<ClientInputOutput>) {
        self.operations.input_output(outputs, application, run)
    }

    /// Run frame calls for a 1-based source row; reports row ownership.
    pub fn frame(&mut self, slot: usize) -> Result<bool, ClientError> {
        let found = self
            .entries
            .iter()
            .find(|(_, entry)| entry.slot + 1 == slot)
            .map(|(actor, _)| *actor);
        let Some(actor) = found else {
            return Ok(false);
        };
        let directory = self.operations.directory();
        let live = directory
            .for_actor(actor)
            .is_some_and(|client| directory.actor_of(client) == Some(actor));
        if live && self.admitted(actor) {
            let calls = self.operations.declaration().frame.clone();
            self.calls(&calls, actor)?;
        }
        Ok(true)
    }

    /// Run end-of-frame calls in slot order.
    pub fn end_frame(&mut self) -> Result<(), ClientError> {
        if self.operations.declaration().end_frame.is_empty() {
            return Ok(());
        }
        let mut actors: Vec<(usize, NativeActorId)> =
            self.entries.iter().map(|(actor, entry)| (entry.slot, *actor)).collect();
        actors.sort_unstable();
        for (_, actor) in actors {
            let directory = self.operations.directory();
            let live = directory
                .for_actor(actor)
                .is_some_and(|client| directory.actor_of(client) == Some(actor));
            if live && self.admitted(actor) {
                let calls = self.operations.declaration().end_frame.clone();
                self.calls(&calls, actor)?;
            }
        }
        Ok(())
    }

    fn disconnect(&mut self, actor: NativeActorId) -> Result<(), ClientError> {
        let admitted = self.require(actor).map(|entry| entry.admitted)?;
        if admitted {
            let calls = self.operations.declaration().disconnect.clone();
            let result = self.calls(&calls, actor);
            self.operations.release(actor).ok();
            self.entries.remove(&actor);
            result?;
        } else {
            self.operations.release(actor).ok();
            self.entries.remove(&actor);
        }
        Ok(())
    }

    /// Read a client's userinfo string.
    pub fn userinfo(&mut self, actor: NativeActorId) -> Result<String, ClientError> {
        let client = self.require(actor)?.client;
        Ok(self.operations.directory().userinfo(client))
    }

    /// Store a client's userinfo string.
    pub fn set_userinfo(&mut self, actor: NativeActorId, value: String) -> Result<(), ClientError> {
        let client = self.require(actor)?.client;
        self.operations.directory().set_userinfo(client, value);
        Ok(())
    }

    /// Route a command through client command calls.
    pub fn invoke_command(&mut self, command: &CommandInvocation) -> Result<bool, ClientError> {
        if self.operations.declaration().command.is_empty() {
            return Ok(false);
        }
        let mut origin = &command.origin;
        while let CommandOrigin::Script { caller } = origin {
            origin = caller;
        }
        let client = match origin {
            CommandOrigin::LocalSeat { client } | CommandOrigin::RemoteCommand { client } => *client,
            CommandOrigin::Other | CommandOrigin::Script { .. } => return Ok(false),
        };
        let actor = self
            .operations
            .directory()
            .actor_of(client)
            .ok_or(ClientError::CommandClientGone)?;
        if !self.admit(actor)? {
            return Ok(true);
        }
        let token = self.operations.begin_command(command);
        let calls = self.operations.declaration().command.clone();
        let result = self.calls(&calls, actor);
        self.operations.end_command(token);
        result?;
        Ok(true)
    }

    /// Capture client slots.
    pub fn checkpoint(&mut self) -> Result<Vec<NativeModClientSlot>, ClientError> {
        let stale: Vec<NativeActorId> = self
            .denied
            .iter()
            .copied()
            .filter(|actor| self.operations.directory().for_actor(*actor).is_none())
            .collect();
        for actor in stale {
            self.denied.remove(&actor);
        }
        if !self.denied.is_empty() {
            return Err(ClientError::PendingRejection);
        }
        Ok(self
            .entries
            .iter()
            .map(|(actor, entry)| NativeModClientSlot {
                actor: *actor,
                slot: entry.slot,
                admitted: entry.admitted,
            })
            .collect())
    }

    /// Validate saved slots against live identities.
    pub fn validate_restore(&mut self, entries: &[NativeModClientSlot]) -> Result<(), ClientError> {
        for entry in entries {
            let directory = self.operations.directory();
            let client = directory.for_actor(entry.actor);
            if client.is_none_or(|client| directory.actor_of(client) != Some(entry.actor)) {
                return Err(ClientError::SavedUnavailable);
            }
        }
        Ok(())
    }

    /// Restore client slots.
    pub fn restore(&mut self, entries: &[NativeModClientSlot]) -> Result<(), ClientError> {
        self.validate_restore(entries)?;
        self.entries.clear();
        self.denied.clear();
        for entry in entries {
            let client = self
                .operations
                .directory()
                .for_actor(entry.actor)
                .ok_or(ClientError::SavedUnavailable)?;
            self.entries.insert(
                entry.actor,
                Entry {
                    client,
                    slot: entry.slot,
                    admitted: entry.admitted,
                },
            );
        }
        Ok(())
    }

    /// Forget a row without running disconnect (owner already released it).
    pub fn forget(&mut self, actor: NativeActorId) {
        self.entries.remove(&actor);
    }

    /// Release every entry, aggregating failures.
    pub fn close(&mut self) -> Result<(), ClientError> {
        self.started = false;
        let mut errors = Vec::new();
        for actor in self.entries.keys().copied().collect::<Vec<_>>() {
            if let Err(error) = self.operations.release(actor) {
                errors.push(error.to_string());
            }
        }
        self.entries.clear();
        self.denied.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ClientError::Cleanup(errors))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(slot: u32) -> NativeActorId {
        NativeActorId { slot, generation: 1 }
    }

    fn client(slot: u32) -> NativeClientId {
        NativeClientId { slot, generation: 1 }
    }

    fn call(id: &str, accepts: CallAccepts) -> SourceCallDecl {
        SourceCallDecl {
            id: id.to_string(),
            accepts,
        }
    }

    fn declaration() -> ClientsDeclaration {
        ClientsDeclaration {
            maximum: 2,
            admit: vec![call("admit", CallAccepts::NonZero)],
            userinfo: vec![call("userinfo", CallAccepts::Always)],
            disconnect: vec![call("disconnect", CallAccepts::Always)],
            command: vec![call("command", CallAccepts::Always)],
            frame: vec![call("frame", CallAccepts::Always)],
            end_frame: vec![call("end-frame", CallAccepts::Always)],
            input: vec![InputBindingDecl {
                id: "move".to_string(),
                calls: vec![call("input", CallAccepts::Always)],
            }],
        }
    }

    struct TestDirectory {
        actors: HashMap<NativeActorId, NativeClientId>,
        userinfos: HashMap<NativeClientId, String>,
        drops: Vec<(NativeClientId, String)>,
        events: Vec<ClientEvent>,
    }

    impl ClientDirectory for TestDirectory {
        fn for_actor(&self, actor: NativeActorId) -> Option<NativeClientId> {
            self.actors.get(&actor).copied()
        }

        fn actor_of(&self, client: NativeClientId) -> Option<NativeActorId> {
            self.actors
                .iter()
                .find(|(_, bound)| **bound == client)
                .map(|(actor, _)| *actor)
        }

        fn clients(&self) -> Vec<(NativeActorId, NativeClientId)> {
            self.actors.iter().map(|(actor, client)| (*actor, *client)).collect()
        }

        fn userinfo(&self, client: NativeClientId) -> String {
            self.userinfos.get(&client).cloned().unwrap_or_default()
        }

        fn set_userinfo(&mut self, client: NativeClientId, value: String) {
            self.userinfos.insert(client, value);
        }

        fn drop_client(&mut self, client: NativeClientId, reason: String) {
            self.drops.push((client, reason));
            self.actors.retain(|_, bound| *bound != client);
        }

        fn take_events(&mut self) -> Vec<ClientEvent> {
            std::mem::take(&mut self.events)
        }
    }

    struct TestOps {
        directory: TestDirectory,
        declaration: ClientsDeclaration,
        projects: Vec<NativeActorId>,
        hooks: Vec<NativeActorId>,
        releases: Vec<NativeActorId>,
        invokes: Vec<(String, NativeActorId)>,
        results: HashMap<String, Option<i32>>,
        inputs: Vec<String>,
        commands: Vec<Vec<String>>,
    }

    impl ClientOperations for TestOps {
        type Directory = TestDirectory;

        fn directory(&mut self) -> &mut Self::Directory {
            &mut self.directory
        }

        fn directory_ref(&self) -> &Self::Directory {
            &self.directory
        }

        fn declaration(&self) -> &ClientsDeclaration {
            &self.declaration
        }

        fn project(&mut self, actor: NativeActorId) {
            self.projects.push(actor);
        }

        fn admitted_hook(&mut self, actor: NativeActorId) {
            self.hooks.push(actor);
        }

        fn release(&mut self, actor: NativeActorId) -> Result<(), ClientError> {
            self.releases.push(actor);
            Ok(())
        }

        fn invoke(&mut self, call: &SourceCallDecl, actor: NativeActorId) -> Option<i32> {
            self.invokes.push((call.id.clone(), actor));
            self.results.get(&call.id).copied().unwrap_or(Some(1))
        }

        fn open_input(&mut self, application: &ClientApplication) -> u64 {
            self.inputs.push(format!("open:{}", application.actor.slot));
            7
        }

        fn close_input(&mut self, token: u64) {
            self.inputs.push(format!("close:{token}"));
        }

        fn invoke_input(&mut self, call: &SourceCallDecl, application: &ClientApplication) {
            self.inputs.push(format!("{}:{}", call.id, application.actor.slot));
        }

        fn input_output<R>(
            &mut self,
            outputs: &[String],
            application: &ClientApplication,
            run: impl FnOnce(&mut Self) -> R,
        ) -> (R, Vec<ClientInputOutput>) {
            let result = run(self);
            let produced = outputs
                .iter()
                .map(|output| ClientInputOutput::Set {
                    input: output.clone(),
                    value: f64::from(application.actor.slot),
                })
                .collect();
            (result, produced)
        }

        fn begin_command(&mut self, command: &CommandInvocation) -> u64 {
            self.commands.push(command.argv.clone());
            self.commands.len() as u64
        }

        fn end_command(&mut self, _token: u64) {}
    }

    fn fixture() -> NativeModClientsBinding<TestOps> {
        let directory = TestDirectory {
            actors: [(actor(1), client(1)), (actor(2), client(2))].into_iter().collect(),
            userinfos: [(client(1), "\\name\\a\\".to_string())].into_iter().collect(),
            drops: Vec::new(),
            events: Vec::new(),
        };
        NativeModClientsBinding::new(TestOps {
            directory,
            declaration: declaration(),
            projects: Vec::new(),
            hooks: Vec::new(),
            releases: Vec::new(),
            invokes: Vec::new(),
            results: HashMap::new(),
            inputs: Vec::new(),
            commands: Vec::new(),
        })
    }

    #[test]
    fn admission_frames_and_disconnect_flow() {
        let mut binding = fixture();
        binding.start().unwrap();
        assert!(binding.admitted(actor(1)));
        assert!(binding.admitted(actor(2)));
        assert_eq!(binding.operations().hooks.len(), 2);
        let slot = binding.slot(actor(1)).unwrap().unwrap();
        assert!(binding.frame(slot + 1).unwrap());
        assert!(!binding.frame(99).unwrap());
        binding.end_frame().unwrap();
        let kinds: Vec<&str> = binding.operations().invokes.iter().map(|(id, _)| id.as_str()).collect();
        assert!(kinds.contains(&"admit"));
        assert!(kinds.contains(&"frame"));
        assert!(kinds.contains(&"end-frame"));

        binding
            .dispatch_input(
                &InputBindingDecl {
                    id: "move".to_string(),
                    calls: Vec::new(),
                },
                &ClientApplication {
                    actor: actor(1),
                    client: client(1),
                },
                &call("input", CallAccepts::Always),
            )
            .unwrap();
        assert_eq!(
            binding.operations().inputs,
            vec!["open:1".to_string(), "input:1".to_string(), "close:7".to_string()]
        );

        binding
            .operations_mut()
            .directory
            .events
            .push(ClientEvent::Removed { actor: actor(1) });
        binding.poll().unwrap();
        assert!(!binding.has(actor(1)));
        assert!(binding.operations().releases.contains(&actor(1)));
        binding.close().unwrap();
    }

    #[test]
    fn nonzero_rejection_drops_and_blocks_checkpoint() {
        let mut binding = fixture();
        binding.operations_mut().directory.actors.remove(&actor(2));
        binding.operations_mut().results.insert("admit".to_string(), Some(0));
        binding
            .operations_mut()
            .directory
            .userinfos
            .insert(client(1), "\\rejmsg\\full\\".to_string());
        binding.start().unwrap();
        assert!(binding.rejects(actor(1)));
        assert!(!binding.admitted(actor(1)));
        assert_eq!(
            binding.operations().directory.drops,
            vec![(client(1), "full".to_string())]
        );
        // Rejected but still connected (re-queued event keeps the row denied).
        binding.operations_mut().directory.actors.insert(actor(1), client(1));
        assert_eq!(binding.checkpoint(), Err(ClientError::PendingRejection));
        binding
            .operations_mut()
            .directory
            .events
            .push(ClientEvent::Removed { actor: actor(1) });
        binding.poll().unwrap();
        assert!(!binding.rejects(actor(1)));
        assert!(!binding.has(actor(1)));
    }

    #[test]
    fn capacity_commands_and_checkpoint_roundtrip() {
        let mut tight = fixture();
        tight.operations_mut().declaration.maximum = 0;
        tight
            .operations_mut()
            .directory
            .events
            .push(ClientEvent::Admitted { actor: actor(1) });
        assert_eq!(tight.poll(), Err(ClientError::CapacityExceeded));

        let mut solo = fixture();
        solo.operations_mut().directory.actors.remove(&actor(2));
        solo.start().unwrap();
        let handled = solo
            .invoke_command(&CommandInvocation {
                argv: vec!["say".to_string(), "hi".to_string()],
                origin: CommandOrigin::Script {
                    caller: Box::new(CommandOrigin::LocalSeat { client: client(1) }),
                },
            })
            .unwrap();
        assert!(handled);
        assert_eq!(
            solo.operations().commands,
            vec![vec!["say".to_string(), "hi".to_string()]]
        );
        let ignored = solo
            .invoke_command(&CommandInvocation {
                argv: vec!["say".to_string()],
                origin: CommandOrigin::Other,
            })
            .unwrap();
        assert!(!ignored);

        let saved = solo.checkpoint().unwrap();
        assert_eq!(saved.len(), 1);
        solo.restore(&saved).unwrap();
        assert!(solo.admitted(actor(1)));
        solo.forget(actor(1));
        assert!(!solo.has(actor(1)));
    }
}
