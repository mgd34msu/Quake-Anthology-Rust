//! Application console cvar routing.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/console.ts`
//! (`q1ConsoleServer`, `ApplicationConsoleRouting`).
//! The Rust [`CvarRegistry`](qa_core::cvar::CvarRegistry) carries no session
//! context (upstream follow-up), so each registry handle is paired with its
//! declaring session and origin in [`ConsoleRegistry`]; the routing
//! precedence, dialect guards, and seat-ownership checks otherwise match the
//! donor. `frameTimeCvarNames` is absorbed from `frame-time.ts`.
//! Seat and client handles include their session token in equality, so the
//! donor's `origin.seat.session !== source.session` checks are covered by
//! the seat-identity comparisons; handles expose no separate session
//! projection to compare against the command session directly.

use qa_core::cmd::{source_command_text, CmdError, Dialect};
use qa_core::cmd_buffer::{CommandContext, CommandOrigin};
use qa_core::cvar::{CvarError, CvarRegistry};
use qa_core::identity::{SeatId, SessionId};
use thiserror::Error;

/// Console routing failure.
#[derive(Debug, Error)]
pub enum ConsoleError {
    /// Console dialect must match the source game.
    #[error("Console dialect must match the source game")]
    DialectMismatch,
    /// Command belongs to another session.
    #[error("Console command belongs to another session")]
    ForeignSession,
    /// Source dialect changed under the owner.
    #[error("Changing source command dialect requires a new console owner")]
    DialectChanged,
    /// Remote clients need an explicit client owner.
    #[error("Remote client cvars require an explicit client owner")]
    RemoteClient,
    /// Movement cvars belong to another session.
    #[error("Movement cvars belong to another session")]
    ForeignMovement,
    /// Registry has another session or dialect.
    #[error("Console cvar registry has another session or source dialect")]
    ForeignRegistry,
    /// Mouse settings require a local seat owner.
    #[error("Mouse settings require a local seat owner")]
    InputNotSeat,
    /// Registry belongs to another seat.
    #[error("Console cvar registry belongs to another seat")]
    ForeignSeatRegistry,
    /// Seat and movement both declare a name.
    #[error("Console cvar {0} has conflicting seat and movement declarations")]
    SeatMovementConflict(String),
    /// Console owner is closed.
    #[error("Application console owner is closed")]
    Closed,
    /// Name text is not source bytes.
    #[error(transparent)]
    Command(#[from] CmdError),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

fn caller(origin: &CommandOrigin) -> &CommandOrigin {
    let mut current = origin;
    while let CommandOrigin::Script { caller, .. } = current {
        current = caller;
    }
    current
}

/// Frame-time cvar names shared with client mirrors, by dialect
/// (`frameTimeCvarNames` from `frame-time.ts`).
#[must_use]
pub fn frame_time_cvar_names(dialect: Dialect) -> &'static [&'static str] {
    if dialect.is_q1() {
        &["timescale", "host_framerate"]
    } else if dialect.is_q2() {
        &["timescale", "fixedtime"]
    } else {
        &["timescale", "fixedtime", "com_cameraMode"]
    }
}

/// One registry plus the session context the Rust core does not carry.
#[derive(Clone)]
pub struct ConsoleRegistry<'a> {
    /// Registry handle.
    pub registry: &'a CvarRegistry,
    /// Declaring session.
    pub session: SessionId,
    /// Declared owner origin.
    pub origin: CommandOrigin,
}

/// Server console plus the names a client may mirror
/// (`ApplicationConsoleServer`).
#[derive(Clone)]
pub struct ApplicationConsoleServer<'a> {
    /// Server registry.
    pub cvars: ConsoleRegistry<'a>,
    /// Names declared by the source game that a client may also register
    /// as mirrors.
    pub shared_names: Vec<String>,
}

/// Q1 console sources (`q1ConsoleServer` simulation reads).
#[derive(Clone)]
pub struct Q1ConsoleSources<'a> {
    /// Q1 source registry, if any.
    pub q1: Option<ConsoleRegistry<'a>>,
    /// QuakeC source registry, if any.
    pub quakec: Option<ConsoleRegistry<'a>>,
}

/// Q1 console server from the first available Q1 source
/// (`q1ConsoleServer`).
#[must_use]
pub fn q1_console_server<'a>(sources: &Q1ConsoleSources<'a>) -> Option<ApplicationConsoleServer<'a>> {
    let source = sources.q1.clone().or_else(|| sources.quakec.clone())?;
    let shared_names = frame_time_cvar_names(source.registry.dialect())
        .iter()
        .map(ToString::to_string)
        .collect();
    Some(ApplicationConsoleServer {
        cvars: source,
        shared_names,
    })
}

/// Current server lookup.
pub type ServerLookup<'a> = Box<dyn Fn() -> Option<ApplicationConsoleServer<'a>> + 'a>;
/// Seat registry lookup.
pub type SeatLookup<'a> = Box<dyn Fn(&SeatId) -> Option<ConsoleRegistry<'a>> + 'a>;
/// Input registry lookup; a null id selects the primary input seat.
pub type InputLookup<'a> = Box<dyn Fn(Option<&SeatId>) -> Option<ConsoleRegistry<'a>> + 'a>;
/// Movement registry lookup.
pub type MovementLookup<'a> = Box<dyn Fn() -> Option<ConsoleRegistry<'a>> + 'a>;
/// Shared registry lookup.
pub type SharedLookup<'a> = Box<dyn Fn() -> Option<&'a CvarRegistry> + 'a>;

/// Console routing options (`ApplicationConsoleRoutingOptions`).
pub struct ApplicationConsoleRoutingOptions<'a> {
    /// Fallback registry.
    pub fallback: ConsoleRegistry<'a>,
    /// Source game dialect.
    pub source_dialect: Dialect,
    /// Current server, if any.
    pub server: ServerLookup<'a>,
    /// Seat registry lookup.
    pub seat: SeatLookup<'a>,
    /// Input registry lookup.
    pub input: Option<InputLookup<'a>>,
    /// Movement registry lookup.
    pub movement: Option<MovementLookup<'a>>,
    /// Shared registry lookup.
    pub shared: Option<SharedLookup<'a>>,
}

struct CvarOwners<'a> {
    server: Option<ApplicationConsoleServer<'a>>,
    seat: Option<ConsoleRegistry<'a>>,
    input: Option<ConsoleRegistry<'a>>,
    movement: Option<ConsoleRegistry<'a>>,
    origin: CommandOrigin,
}

/// Cvar routing contract (`CommandCvarRouting`).
pub trait ConsoleCvarRouting<'a> {
    /// Owning registry for a name under a command context.
    fn owner(&self, name: &str, source: &CommandContext) -> Result<&'a CvarRegistry, ConsoleError>;
    /// Registries visible to a command context.
    fn visible(&self, source: &CommandContext) -> Result<Vec<&'a CvarRegistry>, ConsoleError>;
}

/// Session-separated console cvar routing (`ApplicationConsoleRouting`).
pub struct ApplicationConsoleRouting<'a> {
    options: ApplicationConsoleRoutingOptions<'a>,
    closed: bool,
}

impl<'a> ApplicationConsoleRouting<'a> {
    /// Build routing; the console dialect must match the fallback dialect.
    pub fn new(options: ApplicationConsoleRoutingOptions<'a>) -> Result<Self, ConsoleError> {
        if options.source_dialect != options.fallback.registry.dialect() {
            return Err(ConsoleError::DialectMismatch);
        }
        Ok(Self { options, closed: false })
    }

    fn owners(&self, source: &CommandContext) -> Result<CvarOwners<'a>, ConsoleError> {
        if self.closed {
            return Err(ConsoleError::Closed);
        }
        let dialect = self.options.source_dialect;
        if source.session != self.options.fallback.session {
            return Err(ConsoleError::ForeignSession);
        }
        if dialect != self.options.fallback.registry.dialect() {
            return Err(ConsoleError::DialectChanged);
        }
        let server = (self.options.server)();
        let origin = caller(&source.origin).clone();
        if matches!(origin, CommandOrigin::RemoteClient { .. }) {
            return Err(ConsoleError::RemoteClient);
        }
        let seat = match &origin {
            CommandOrigin::LocalSeat { seat, .. } => (self.options.seat)(seat),
            _ => None,
        };
        let input = match &origin {
            CommandOrigin::ServerConsole => None,
            CommandOrigin::LocalSeat { seat, .. } => self.options.input.as_ref().and_then(|lookup| lookup(Some(seat))),
            _ => self.options.input.as_ref().and_then(|lookup| lookup(None)),
        };
        let movement = self.options.movement.as_ref().and_then(|lookup| lookup());
        if let Some(movement) = &movement {
            if movement.session != source.session {
                return Err(ConsoleError::ForeignMovement);
            }
        }
        for registry in [
            server.as_ref().map(|server| &server.cvars),
            seat.as_ref(),
            input.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if registry.session != source.session || registry.registry.dialect() != dialect {
                return Err(ConsoleError::ForeignRegistry);
            }
        }
        if let Some(input) = &input {
            if !matches!(caller(&input.origin), CommandOrigin::LocalSeat { .. }) {
                return Err(ConsoleError::InputNotSeat);
            }
        }
        if let CommandOrigin::LocalSeat {
            seat: seat_id,
            client: client_id,
        } = &origin
        {
            for registry in [&seat, &input].into_iter().flatten() {
                match caller(&registry.origin) {
                    CommandOrigin::LocalSeat {
                        seat: owner_seat,
                        client: owner_client,
                    } if owner_seat == seat_id && owner_client == client_id => {}
                    _ => return Err(ConsoleError::ForeignSeatRegistry),
                }
            }
        }
        Ok(CvarOwners {
            server,
            seat,
            input,
            movement,
            origin,
        })
    }

    /// Retire the owner (`close`).
    pub fn close(&mut self) {
        self.closed = true;
    }
}

impl<'a> ConsoleCvarRouting<'a> for ApplicationConsoleRouting<'a> {
    fn owner(&self, name_input: &str, source: &CommandContext) -> Result<&'a CvarRegistry, ConsoleError> {
        let name = source_command_text(name_input)?;
        let resolved = self.owners(source)?;
        if let Some(shared) = self.options.shared.as_ref().and_then(|lookup| lookup()) {
            if shared.get(&name).is_some() {
                return Ok(shared);
            }
        }
        if let Some(server) = &resolved.server {
            if server.cvars.registry.get(&name).is_some() {
                return Ok(server.cvars.registry);
            }
        }
        if let Some(input) = &resolved.input {
            if input.registry.get(&name).is_some() {
                return Ok(input.registry);
            }
        }
        let seat_has = resolved
            .seat
            .as_ref()
            .is_some_and(|seat| seat.registry.get(&name).is_some());
        let movement_has = resolved
            .movement
            .as_ref()
            .is_some_and(|movement| movement.registry.get(&name).is_some());
        if seat_has && movement_has {
            let seat_ptr = resolved.seat.as_ref().map(|seat| seat.registry as *const CvarRegistry);
            let movement_ptr = resolved
                .movement
                .as_ref()
                .map(|movement| movement.registry as *const CvarRegistry);
            if seat_ptr != movement_ptr {
                return Err(ConsoleError::SeatMovementConflict(name));
            }
        }
        if let Some(seat) = &resolved.seat {
            if seat_has {
                return Ok(seat.registry);
            }
        }
        if let Some(movement) = &resolved.movement {
            if movement_has {
                return Ok(movement.registry);
            }
        }
        if self.options.fallback.registry.get(&name).is_some() {
            return Ok(self.options.fallback.registry);
        }
        if matches!(resolved.origin, CommandOrigin::ServerConsole) {
            if let Some(server) = &resolved.server {
                return Ok(server.cvars.registry);
            }
        }
        if let Some(seat) = &resolved.seat {
            return Ok(seat.registry);
        }
        Ok(self.options.fallback.registry)
    }

    fn visible(&self, source: &CommandContext) -> Result<Vec<&'a CvarRegistry>, ConsoleError> {
        let resolved = self.owners(source)?;
        let mut registries: Vec<&'a CvarRegistry> = Vec::new();
        let mut push = |registry: &'a CvarRegistry| {
            if !registries.iter().any(|existing| std::ptr::eq(*existing, registry)) {
                registries.push(registry);
            }
        };
        if let Some(shared) = self.options.shared.as_ref().and_then(|lookup| lookup()) {
            push(shared);
        }
        if let Some(input) = &resolved.input {
            push(input.registry);
        }
        if let Some(server) = &resolved.server {
            push(server.cvars.registry);
        }
        if let Some(seat) = &resolved.seat {
            push(seat.registry);
        }
        if let Some(movement) = &resolved.movement {
            push(movement.registry);
        }
        push(self.options.fallback.registry);
        Ok(registries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct Fixture {
        owner: IdentityOwner,
        fallback: CvarRegistry,
        server: CvarRegistry,
        seat: CvarRegistry,
        input: CvarRegistry,
        movement: CvarRegistry,
        shared: CvarRegistry,
    }

    impl Fixture {
        fn new() -> Self {
            let owner = IdentityOwner::create("console").unwrap();
            let mut fallback = CvarRegistry::new(Dialect::Q3);
            fallback.register("fallback_only", "0", 0).unwrap();
            let mut server = CvarRegistry::new(Dialect::Q3);
            server.register("sv_hostname", "host", 0).unwrap();
            server.register("shared_name", "server", 0).unwrap();
            let mut seat = CvarRegistry::new(Dialect::Q3);
            seat.register("seat_only", "1", 0).unwrap();
            seat.register("clash", "seat", 0).unwrap();
            let mut input = CvarRegistry::new(Dialect::Q3);
            input.register("sensitivity", "3", 0).unwrap();
            let mut movement = CvarRegistry::new(Dialect::Q3);
            movement.register("movement_only", "2", 0).unwrap();
            movement.register("clash", "movement", 0).unwrap();
            let mut shared = CvarRegistry::new(Dialect::Q3);
            shared.register("shared_name", "shared", 0).unwrap();
            Self {
                owner,
                fallback,
                server,
                seat,
                input,
                movement,
                shared,
            }
        }

        fn owned<'x>(&self, registry: &'x CvarRegistry, origin: CommandOrigin) -> ConsoleRegistry<'x> {
            ConsoleRegistry {
                registry,
                session: self.owner.session().clone(),
                origin,
            }
        }

        fn seat_origin(&self) -> (CommandOrigin, SeatId) {
            let seat = self.owner.seat(0);
            let client = self.owner.client(0, 0);
            (
                CommandOrigin::LocalSeat {
                    seat: seat.clone(),
                    client,
                },
                seat,
            )
        }
    }

    fn routing<'a>(
        fixture: &'a Fixture,
        _seat_id: SeatId,
        seat_origin: CommandOrigin,
    ) -> ApplicationConsoleRouting<'a> {
        let fallback = fixture.owned(&fixture.fallback, CommandOrigin::ServerConsole);
        let server = fixture.owned(&fixture.server, CommandOrigin::ServerConsole);
        let seat = fixture.owned(&fixture.seat, seat_origin.clone());
        let input = fixture.owned(&fixture.input, seat_origin);
        let movement = fixture.owned(&fixture.movement, CommandOrigin::ServerConsole);
        ApplicationConsoleRouting::new(ApplicationConsoleRoutingOptions {
            fallback,
            source_dialect: Dialect::Q3,
            server: Box::new(move || {
                Some(ApplicationConsoleServer {
                    cvars: server.clone(),
                    shared_names: vec!["timescale".to_string()],
                })
            }),
            seat: Box::new(move |_| Some(seat.clone())),
            input: Some(Box::new(move |_| Some(input.clone()))),
            movement: Some(Box::new(move || Some(movement.clone()))),
            shared: Some(Box::new(|| None)),
        })
        .unwrap()
    }

    #[test]
    fn owner_follows_shared_server_input_seat_movement_fallback() {
        let fixture = Fixture::new();
        let (origin, seat_id) = fixture.seat_origin();
        let routing = routing(&fixture, seat_id, origin.clone());
        let source = CommandContext::new(fixture.owner.session().clone(), origin);
        assert_eq!(
            routing
                .owner("sv_hostname", &source)
                .unwrap()
                .get("sv_hostname")
                .unwrap()
                .value,
            "host"
        );
        assert_eq!(
            routing
                .owner("sensitivity", &source)
                .unwrap()
                .get("sensitivity")
                .unwrap()
                .value,
            "3"
        );
        assert_eq!(
            routing
                .owner("seat_only", &source)
                .unwrap()
                .get("seat_only")
                .unwrap()
                .value,
            "1"
        );
        assert_eq!(
            routing
                .owner("movement_only", &source)
                .unwrap()
                .get("movement_only")
                .unwrap()
                .value,
            "2"
        );
        assert_eq!(
            routing
                .owner("fallback_only", &source)
                .unwrap()
                .get("fallback_only")
                .unwrap()
                .value,
            "0"
        );
        // Unknown names fall back to the seat registry.
        assert!(routing
            .owner("unknown_name", &source)
            .unwrap()
            .get("seat_only")
            .is_some());
    }

    #[test]
    fn shared_registry_wins_and_conflicts_error() {
        let fixture = Fixture::new();
        let (origin, seat_id) = fixture.seat_origin();
        let mut routing = routing(&fixture, seat_id, origin.clone());
        let source = CommandContext::new(fixture.owner.session().clone(), origin);
        // Without a shared hit, the server defines the shared name.
        assert_eq!(
            routing
                .owner("shared_name", &source)
                .unwrap()
                .get("shared_name")
                .unwrap()
                .value,
            "server"
        );
        routing.options.shared = Some(Box::new(|| None));
        // Installing the shared registry reroutes the shared name.
        let shared_registry: &CvarRegistry = &fixture.shared;
        routing.options.shared = Some(Box::new(move || Some(shared_registry)));
        assert_eq!(
            routing
                .owner("shared_name", &source)
                .unwrap()
                .get("shared_name")
                .unwrap()
                .value,
            "shared"
        );
        // Seat and movement both declaring a name is a conflict.
        assert!(matches!(
            routing.owner("clash", &source).err().unwrap(),
            ConsoleError::SeatMovementConflict(name) if name == "clash"
        ));
    }

    #[test]
    fn visible_lists_input_server_seat_movement_fallback_once() {
        let fixture = Fixture::new();
        let (origin, seat_id) = fixture.seat_origin();
        let routing = routing(&fixture, seat_id, origin.clone());
        let source = CommandContext::new(fixture.owner.session().clone(), origin);
        let tag = |registry: &CvarRegistry| {
            if registry.get("sensitivity").is_some() {
                "input"
            } else if registry.get("sv_hostname").is_some() {
                "server"
            } else if registry.get("seat_only").is_some() {
                "seat"
            } else if registry.get("movement_only").is_some() {
                "movement"
            } else {
                "fallback"
            }
        };
        let order: Vec<&str> = routing
            .visible(&source)
            .unwrap()
            .iter()
            .map(|registry| tag(registry))
            .collect();
        assert_eq!(order, vec!["input", "server", "seat", "movement", "fallback"]);
    }

    #[test]
    fn rejects_foreign_sessions_remote_clients_and_closed() {
        let fixture = Fixture::new();
        let (origin, seat_id) = fixture.seat_origin();
        let mut routing = routing(&fixture, seat_id, origin.clone());
        let foreign_owner = IdentityOwner::create("foreign").unwrap();
        let foreign = CommandContext::new(foreign_owner.session().clone(), origin.clone());
        assert!(matches!(
            routing.owner("seat_only", &foreign).err().unwrap(),
            ConsoleError::ForeignSession
        ));
        let remote = CommandContext::new(
            fixture.owner.session().clone(),
            CommandOrigin::RemoteClient {
                client: foreign_owner.client(0, 0),
            },
        );
        assert!(matches!(
            routing.owner("seat_only", &remote).err().unwrap(),
            ConsoleError::RemoteClient
        ));
        routing.close();
        let source = CommandContext::new(fixture.owner.session().clone(), origin);
        assert!(matches!(
            routing.owner("seat_only", &source).err().unwrap(),
            ConsoleError::Closed
        ));
    }

    #[test]
    fn q1_server_prefers_q1_source() {
        let fixture = Fixture::new();
        let (origin, _) = fixture.seat_origin();
        let server = q1_console_server(&Q1ConsoleSources {
            q1: Some(fixture.owned(&fixture.server, origin.clone())),
            quakec: Some(fixture.owned(&fixture.seat, origin)),
        })
        .unwrap();
        assert!(server.cvars.registry.get("sv_hostname").is_some());
        assert_eq!(
            server.shared_names,
            vec![
                "timescale".to_string(),
                "fixedtime".to_string(),
                "com_cameraMode".to_string()
            ]
        );
        assert!(q1_console_server(&Q1ConsoleSources { q1: None, quakec: None }).is_none());
    }
}
