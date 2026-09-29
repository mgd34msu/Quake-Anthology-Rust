//! Command dispatch for input: registry, origins, and client commands.
//!
//! Donor provenance: `src/input/client-commands.ts`
//! (`ClientCommandBindings`) plus the `CommandBuffer` surface it borrows
//! (`register`, `registerEngine`, `unregister`, `exists`, `append`).
//! The application owns script execution; input only registers names
//! and appends binding text through [`CommandRegistry`].

use std::collections::{HashMap, HashSet};

use qa_core::cmd::{ascii_fold, Dialect};
use qa_core::identity::SeatId;
use thiserror::Error;

/// Command origin chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOrigin {
    /// A local seat.
    LocalSeat(SeatId),
    /// The local console.
    LocalConsole,
    /// The server console.
    ServerConsole,
    /// A script frame; the caller owns the command.
    Script {
        /// Calling origin.
        caller: Box<CommandOrigin>,
    },
    /// Any other producer (remote, demo, VM).
    Other,
}

/// Client-module producer tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientModuleProducer {
    /// Owning instance token.
    pub instance: u64,
}

/// One command invocation.
#[derive(Debug, Clone)]
pub struct CommandInvocation {
    /// Argument vector; `argv[0]` is the command name.
    pub argv: Vec<String>,
    /// Invocation origin.
    pub origin: CommandOrigin,
    /// Client-module producer, when the call comes from a guest module.
    pub producer: Option<ClientModuleProducer>,
    /// Command dialect.
    pub dialect: Dialect,
}

impl CommandInvocation {
    /// New invocation.
    #[must_use]
    pub fn new(argv: Vec<String>, origin: CommandOrigin, dialect: Dialect) -> Self {
        Self {
            argv,
            origin,
            producer: None,
            dialect,
        }
    }

    /// Origin with script frames unwound.
    #[must_use]
    pub fn root_origin(&self) -> &CommandOrigin {
        let mut origin = &self.origin;
        while let CommandOrigin::Script { caller } = origin {
            origin = caller;
        }
        origin
    }
}

/// Command handler result.
pub type HandlerResult = Result<(), CommandsError>;

/// Registered command handler.
pub type CommandHandler = Box<dyn FnMut(&CommandInvocation) -> HandlerResult>;

/// Application command buffer surface used by input.
pub trait CommandRegistry {
    /// Register an engine command; returns false when the name is taken.
    fn register_engine(&mut self, name: &str, handler: CommandHandler) -> bool;
    /// Register a game command; returns false when the name is taken.
    fn register(&mut self, name: &str, handler: CommandHandler) -> bool;
    /// Remove a command.
    fn unregister(&mut self, name: &str);
    /// Whether a command exists.
    fn exists(&self, name: &str) -> bool;
    /// Queue binding text for a seat.
    fn append(&mut self, text: &str, seat: &SeatId);
}

/// Input command error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandsError {
    /// Client commands need a published local seat.
    #[error("Client commands require a local seat")]
    NoLocalSeat,
    /// The owner was retired.
    #[error("Client command owner is retired")]
    OwnerRetired,
    /// Several components claim one command.
    #[error("Ambiguous component client command {name}: {labels}")]
    AmbiguousCommand {
        /// Command name.
        name: String,
        /// Claiming labels.
        labels: String,
    },
    /// An impulse must fit a byte.
    #[error("Input impulse must fit a byte")]
    BadImpulse,
    /// A button command timestamp is not finite.
    #[error("Invalid button command timestamp")]
    BadTimestamp,
}

/// One guest instance's client-command handler.
pub struct ClientCommandHandler {
    /// Owning instance token.
    pub instance: u64,
    /// Component label for ambiguity reports.
    pub label: String,
    /// Handler body.
    pub execute: Box<dyn FnMut(&CommandInvocation)>,
}

/// Owner handle from [`ClientCommandBindings::create_owner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientCommandOwner(pub u64);

struct OwnerState {
    seat: SeatId,
    names: HashSet<String>,
    handler: Option<ClientCommandHandler>,
}

/// Guest command claims multiplexed onto shared dispatchers.
///
/// Guest instances own claims; one published input owns the shared
/// dispatchers. Installed names dispatch through
/// [`ClientCommandBindings::dispatch`], which the application calls
/// from each installed command.
pub struct ClientCommandBindings {
    owners: HashMap<ClientCommandOwner, OwnerState>,
    installed: HashSet<String>,
    seats: Vec<SeatId>,
    next_owner: u64,
    active: bool,
}

impl ClientCommandBindings {
    /// Bindings for the published seats.
    #[must_use]
    pub fn new(seats: Vec<SeatId>) -> Self {
        Self {
            owners: HashMap::new(),
            installed: HashSet::new(),
            seats,
            next_owner: 1,
            active: false,
        }
    }

    /// Whether dispatchers are installed.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Installed command names.
    #[must_use]
    pub fn installed(&self) -> Vec<String> {
        let mut names: Vec<String> = self.installed.iter().cloned().collect();
        names.sort();
        names
    }

    /// Republish seats, retiring owners on removed seats.
    pub fn publish_seats(&mut self, commands: &mut dyn CommandRegistry, seats: Vec<SeatId>) {
        let retired: Vec<ClientCommandOwner> = self
            .owners
            .iter()
            .filter(|(_, owner)| !seats.contains(&owner.seat))
            .map(|(id, _)| *id)
            .collect();
        for id in retired {
            self.close_owner(commands, id);
        }
        self.seats = seats;
    }

    /// Create a claim owner for a local seat.
    pub fn create_owner(
        &mut self,
        seat: SeatId,
        handler: Option<ClientCommandHandler>,
    ) -> Result<ClientCommandOwner, CommandsError> {
        if !self.seats.contains(&seat) {
            return Err(CommandsError::NoLocalSeat);
        }
        let id = ClientCommandOwner(self.next_owner);
        self.next_owner += 1;
        self.owners.insert(
            id,
            OwnerState {
                seat,
                names: HashSet::new(),
                handler,
            },
        );
        Ok(id)
    }

    /// Claim a command name for an owner.
    pub fn owner_register(
        &mut self,
        commands: &mut dyn CommandRegistry,
        owner: ClientCommandOwner,
        name: &str,
    ) -> Result<(), CommandsError> {
        let key = ascii_fold(name);
        {
            let state = self.owners.get_mut(&owner).ok_or(CommandsError::OwnerRetired)?;
            state.names.insert(key.clone());
        }
        if self.active {
            self.install(commands, &key);
        }
        Ok(())
    }

    /// Release one claim.
    pub fn owner_remove(&mut self, commands: &mut dyn CommandRegistry, owner: ClientCommandOwner, name: &str) {
        let key = ascii_fold(name);
        if self.owners.get_mut(&owner).is_some_and(|state| state.names.remove(&key)) {
            self.remove_unused(commands, &key);
        }
    }

    /// Retire an owner and release its claims.
    pub fn close_owner(&mut self, commands: &mut dyn CommandRegistry, owner: ClientCommandOwner) {
        let Some(state) = self.owners.remove(&owner) else {
            return;
        };
        for name in state.names {
            self.remove_unused(commands, &name);
        }
    }

    /// Install dispatchers for every claim.
    pub fn activate(&mut self, commands: &mut dyn CommandRegistry) {
        if self.active {
            return;
        }
        self.active = true;
        let names: Vec<String> = self.owners.values().flat_map(|owner| owner.names.iter().cloned()).collect();
        for name in names {
            self.install(commands, &name);
        }
    }

    /// Remove every dispatcher.
    pub fn deactivate(&mut self, commands: &mut dyn CommandRegistry) {
        self.active = false;
        for name in std::mem::take(&mut self.installed) {
            commands.unregister(&name);
        }
    }

    /// Dispatch an installed command.
    ///
    /// Returns false when no claim on the invoking seat handles it.
    pub fn dispatch(
        &mut self,
        invocation: &CommandInvocation,
        execute: &mut dyn FnMut(&CommandInvocation, &SeatId),
    ) -> Result<bool, CommandsError> {
        if !self.active {
            return Ok(false);
        }
        let seat = match invocation.root_origin() {
            CommandOrigin::LocalSeat(seat) => Some(seat.clone()),
            CommandOrigin::LocalConsole => self
                .seats
                .iter()
                .find(|seat| self.owners.values().any(|owner| &owner.seat == *seat))
                .cloned(),
            _ => None,
        };
        let Some(seat) = seat else {
            return Ok(false);
        };
        let name = ascii_fold(invocation.argv.first().map_or("", String::as_str));
        let claims: Vec<ClientCommandOwner> = self
            .owners
            .iter()
            .filter(|(_, owner)| owner.seat == seat && owner.names.contains(&name))
            .map(|(id, _)| *id)
            .collect();
        if claims.is_empty() {
            return Ok(false);
        }
        if let Some(producer) = &invocation.producer {
            for claim in claims {
                let matches = self
                    .owners
                    .get(&claim)
                    .and_then(|owner| owner.handler.as_ref())
                    .is_some_and(|handler| handler.instance == producer.instance);
                if matches {
                    let owner = self.owners.get_mut(&claim).expect("claim vanished");
                    let handler = owner.handler.as_mut().expect("claim vanished");
                    (handler.execute)(invocation);
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if claims.iter().any(|claim| self.owners.get(claim).is_some_and(|owner| owner.handler.is_none())) {
            execute(invocation, &seat);
            return Ok(true);
        }
        if claims.len() > 1 {
            let labels = claims
                .iter()
                .filter_map(|claim| self.owners.get(claim)?.handler.as_ref().map(|handler| handler.label.clone()))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(CommandsError::AmbiguousCommand { name, labels });
        }
        let claim = claims[0];
        let owner = self.owners.get_mut(&claim).expect("claim vanished");
        if let Some(handler) = owner.handler.as_mut() {
            (handler.execute)(invocation);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn install(&mut self, commands: &mut dyn CommandRegistry, name: &str) {
        if self.installed.contains(name) || commands.exists(name) {
            return;
        }
        self.installed.insert(name.to_string());
    }

    fn remove_unused(&mut self, commands: &mut dyn CommandRegistry, name: &str) {
        if self.owners.values().any(|owner| owner.names.contains(name)) {
            return;
        }
        if self.installed.remove(name) {
            commands.unregister(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct FakeRegistry {
        commands: HashMap<String, CommandHandler>,
        appended: Vec<(String, SeatId)>,
    }

    impl FakeRegistry {
        fn new() -> Self {
            Self {
                commands: HashMap::new(),
                appended: Vec::new(),
            }
        }
    }

    impl CommandRegistry for FakeRegistry {
        fn register_engine(&mut self, name: &str, handler: CommandHandler) -> bool {
            if self.commands.contains_key(name) {
                return false;
            }
            self.commands.insert(name.to_string(), handler);
            true
        }

        fn register(&mut self, name: &str, handler: CommandHandler) -> bool {
            self.register_engine(name, handler)
        }

        fn unregister(&mut self, name: &str) {
            self.commands.remove(name);
        }

        fn exists(&self, name: &str) -> bool {
            self.commands.contains_key(name)
        }

        fn append(&mut self, text: &str, seat: &SeatId) {
            self.appended.push((text.to_string(), seat.clone()));
        }
    }

    #[test]
    fn claims_dispatch_per_seat() {
        let owner = IdentityOwner::create("test").unwrap();
        let first = owner.seat(0);
        let second = owner.seat(1);
        let mut registry = FakeRegistry::new();
        let mut bindings = ClientCommandBindings::new(vec![first.clone(), second.clone()]);
        let guest = bindings.create_owner(first.clone(), None).unwrap();
        bindings.owner_register(&mut registry, guest, "Fire").unwrap();
        bindings.activate(&mut registry);
        assert_eq!(bindings.installed(), vec!["fire".to_string()]);
        let mut executed = Vec::new();
        let invocation = CommandInvocation::new(vec!["fire".to_string()], CommandOrigin::LocalSeat(first.clone()), Dialect::Q3);
        assert!(bindings.dispatch(&invocation, &mut |command, seat| executed.push((command.argv.clone(), seat.clone()))).unwrap());
        assert_eq!(executed.len(), 1);
        let remote = CommandInvocation::new(
            vec!["fire".to_string()],
            CommandOrigin::LocalSeat(second.clone()),
            Dialect::Q3,
        );
        assert!(!bindings.dispatch(&remote, &mut |_, _| panic!("wrong seat")).unwrap());
        bindings.close_owner(&mut registry, guest);
        assert!(bindings.installed().is_empty());
        assert!(bindings.create_owner(owner.seat(9), None).is_err());
    }

    #[test]
    fn module_producers_route_to_instances() {
        let owner = IdentityOwner::create("test").unwrap();
        let seat = owner.seat(0);
        let mut registry = FakeRegistry::new();
        let mut bindings = ClientCommandBindings::new(vec![seat.clone()]);
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let capture = seen.clone();
        let guest = bindings
            .create_owner(
                seat.clone(),
                Some(ClientCommandHandler {
                    instance: 7,
                    label: "mod".to_string(),
                    execute: Box::new(move |command| capture.borrow_mut().push(command.argv.clone())),
                }),
            )
            .unwrap();
        bindings.owner_register(&mut registry, guest, "zap").unwrap();
        bindings.activate(&mut registry);
        let mut invocation = CommandInvocation::new(vec!["zap".to_string()], CommandOrigin::LocalSeat(seat.clone()), Dialect::Q3);
        invocation.producer = Some(ClientModuleProducer { instance: 7 });
        assert!(bindings.dispatch(&invocation, &mut |_, _| panic!("module owned")).unwrap());
        assert_eq!(seen.borrow().len(), 1);
        invocation.producer = Some(ClientModuleProducer { instance: 8 });
        assert!(!bindings.dispatch(&invocation, &mut |_, _| panic!("foreign instance")).unwrap());
        bindings.deactivate(&mut registry);
        assert!(!bindings.is_active());
    }
}
