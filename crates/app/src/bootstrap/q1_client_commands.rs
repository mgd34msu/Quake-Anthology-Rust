//! Quake client cheat/lifecycle commands and host actor routing.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q1-client-commands.ts`
//! (`registerQ1ClientCommands`, `resolveQ1HostCommandActor`). View commands come from the
//! in-scope sibling [`crate::bootstrap::q1_client_settings`]. The merged
//! [`CommandBuffer`](qa_core::cmd_buffer::CommandBuffer) needs the live registry for
//! cvar-name conflicts, so registration takes it explicitly and reports
//! [`BufferError`](qa_core::cmd_buffer::BufferError); the release closure becomes a guard
//! that unregisters both tables.

use std::rc::Rc;

use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{
    BufferError, CommandBuffer, CommandContext, CommandDocumentation, CommandOrigin, Invocation,
};
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, ClientId, SeatId};
use thiserror::Error;

use crate::bootstrap::q1_client_settings::{register_q1_view_commands, Q1ViewCommandGuard};

/// Host callback for client commands (donor `execute`).
pub type Q1ClientCommandExecute = Rc<dyn Fn(&str, &[String], Option<SeatId>, &CommandContext)>;

/// Registered client-command names with their release (donor return closure).
pub struct Q1ClientCommandGuard {
    names: Vec<String>,
    view: Q1ViewCommandGuard,
}

impl Q1ClientCommandGuard {
    /// Client-command names this guard releases (excluding view commands).
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Unregister every command this guard owns, including view commands.
    pub fn release(self, commands: &mut CommandBuffer) {
        self.view.release(commands);
        for name in &self.names {
            commands.unregister(name);
        }
    }
}

/// Unwrap script origins to the commanding origin.
fn root_origin(mut origin: &CommandOrigin) -> &CommandOrigin {
    while let CommandOrigin::Script { caller, .. } = origin {
        origin = caller;
    }
    origin
}

/// Always-registered client commands with their donor summaries.
const Q1_CLIENT_COMMANDS: &[(&str, &str)] = &[
    ("god", "Toggle god mode; multiplayer authority controls cheat access."),
    (
        "notarget",
        "Toggle monster targeting immunity; multiplayer authority controls cheat access.",
    ),
    (
        "noclip",
        "Toggle movement through walls; multiplayer authority controls cheat access.",
    ),
    (
        "give",
        "Give all, health, armor, weapons, ammo, keys, or a named item; source authority controls cheat access.",
    ),
    (
        "giveall",
        "Give the source all grant, including the currently selected arsenal.",
    ),
    ("kill", "Suicide through the source game's player lifecycle."),
    ("suicide", "Alias for kill."),
    (
        "fly",
        "Toggle flying with collision; source authority controls cheat access.",
    ),
];

/// NetQuake/QuakeWorld-only commands with their donor summaries.
const Q1_NET_COMMANDS: &[(&str, &str)] = &[
    ("pause", "Toggle server pause when pausable permits it."),
    ("status", "Show the current source server and connected players."),
    ("ping", "Show measured client round trip times."),
];

/// Register the Q1 client commands (donor `registerQ1ClientCommands`).
pub fn register_q1_client_commands(
    commands: &mut CommandBuffer,
    dialect: Dialect,
    cvars: &CvarRegistry,
    execute: Q1ClientCommandExecute,
) -> Result<Q1ClientCommandGuard, BufferError> {
    let view = register_q1_view_commands(commands, cvars)?;
    let mut table: Vec<(&str, &str)> = Q1_CLIENT_COMMANDS.to_vec();
    if dialect == Dialect::Q1Netquake || dialect == Dialect::Q1Quakeworld {
        table.extend(Q1_NET_COMMANDS.iter().copied());
    }
    let mut names = Vec::new();
    for (name, summary) in table {
        if commands.exists(name) {
            continue;
        }
        let execute = Rc::clone(&execute);
        let owned = name.to_string();
        let usage = if name == "give" {
            "give [client slot: server console only] <all|health|armor|weapons|ammo|keys|item> [amount]".to_string()
        } else {
            format!("{name} [client slot: server console only]")
        };
        let examples = if name == "give" {
            vec!["give all".to_string(), "give health 100".to_string()]
        } else {
            vec![name.to_string()]
        };
        let registered = commands.register(
            name,
            Some(Rc::new(move |invocation: &mut Invocation| {
                let origin = root_origin(&invocation.source.origin);
                let args: Vec<String> = if owned == "giveall" {
                    if matches!(origin, CommandOrigin::ServerConsole) {
                        invocation
                            .args()
                            .iter()
                            .cloned()
                            .chain(std::iter::once("all".to_string()))
                            .collect()
                    } else {
                        vec!["all".to_string()]
                    }
                } else {
                    invocation.args().to_vec()
                };
                let forwarded = if owned == "giveall" {
                    "give"
                } else if owned == "suicide" {
                    "kill"
                } else {
                    owned.as_str()
                };
                let seat = match origin {
                    CommandOrigin::LocalSeat { seat, .. } => Some(seat.clone()),
                    _ => None,
                };
                execute(forwarded, &args, seat, &invocation.source);
            })),
            Some(CommandDocumentation {
                summary: summary.to_string(),
                usage,
                examples,
                allowed_values: None,
            }),
            cvars,
        )?;
        if registered {
            names.push(name.to_string());
        }
    }
    Ok(Q1ClientCommandGuard { names, view })
}

/// Failure to resolve a host-command actor, with donor messages.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q1HostCommandError {
    /// Server-console slot selection was malformed.
    #[error("Usage: {name} <client slot>; connected slots: {slots}")]
    Usage {
        /// Command name.
        name: String,
        /// Connected slots, or `"none"`.
        slots: String,
    },
    /// The selected slot is not on the server.
    #[error("Client slot {slot} is not on the server")]
    UnknownSlot {
        /// Requested slot text.
        slot: String,
    },
    /// Only the server console may select a client slot.
    #[error("{name}: only the server console may select a client slot")]
    ConsoleOnly {
        /// Command name.
        name: String,
    },
    /// The commanding client is not admitted.
    #[error("Command requires an admitted client")]
    NoAdmittedClient,
}

/// One connected player (donor `resolveQ1HostCommandActor` players entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1HostPlayer {
    /// Player actor.
    pub actor: ActorId,
    /// Connected client.
    pub client: ClientId,
}

/// Route a host command to its actor (donor `resolveQ1HostCommandActor`).
pub fn resolve_q1_host_command_actor(
    name: &str,
    args: &[String],
    source: Option<&CommandContext>,
    players: &[Q1HostPlayer],
    local_actor: impl FnOnce() -> ActorId,
    takes_arguments: bool,
) -> Result<ActorId, Q1HostCommandError> {
    let mut origin = source.map(|context| &context.origin);
    while let Some(CommandOrigin::Script { caller, .. }) = origin {
        origin = Some(caller);
    }
    if matches!(origin, Some(CommandOrigin::ServerConsole)) {
        let usage = || {
            let slots = players
                .iter()
                .map(|player| player.client.slot().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Q1HostCommandError::Usage {
                name: name.to_string(),
                slots: if slots.is_empty() { "none".to_string() } else { slots },
            }
        };
        let Some(target) = args.first() else {
            return Err(usage());
        };
        let numeric = !target.is_empty() && target.bytes().all(|byte| byte.is_ascii_digit());
        if (!takes_arguments && args.len() != 1) || !numeric {
            return Err(usage());
        }
        let found = target
            .parse::<u32>()
            .ok()
            .and_then(|slot| players.iter().find(|player| player.client.slot() == slot));
        return match found {
            Some(player) => Ok(player.actor.clone()),
            None => Err(Q1HostCommandError::UnknownSlot { slot: target.clone() }),
        };
    }
    if !takes_arguments && !args.is_empty() {
        return Err(Q1HostCommandError::ConsoleOnly { name: name.to_string() });
    }
    if let Some(CommandOrigin::RemoteClient { client } | CommandOrigin::LocalSeat { client, .. }) = origin {
        return match players.iter().find(|player| &player.client == client) {
            Some(player) => Ok(player.actor.clone()),
            None => Err(Q1HostCommandError::NoAdmittedClient),
        };
    }
    Ok(local_actor())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd_buffer::{BufferOptions, CommandOrigin};
    use qa_core::identity::{IdentityOwner, SessionId};
    use std::cell::RefCell;

    fn buffer(dialect: Dialect) -> (CommandBuffer, CvarRegistry, IdentityOwner) {
        let owner = IdentityOwner::create("q1-commands").unwrap();
        let context = CommandContext::new(owner.session().clone(), CommandOrigin::LocalConsole);
        let buffer = CommandBuffer::new(dialect, context, BufferOptions::default()).unwrap();
        (buffer, CvarRegistry::new(dialect), owner)
    }

    fn session(owner: &IdentityOwner) -> SessionId {
        owner.session().clone()
    }

    #[test]
    fn registers_q1_tables_and_releases() {
        let (mut commands, cvars, _) = buffer(Dialect::Q1Netquake);
        let calls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&calls);
        let guard = register_q1_client_commands(
            &mut commands,
            Dialect::Q1Netquake,
            &cvars,
            Rc::new(move |name, _, _, _| seen.borrow_mut().push(name.to_string())),
        )
        .unwrap();
        assert!(commands.exists("god"));
        assert!(commands.exists("pause"));
        assert!(commands.exists("sizeup"));
        assert_eq!(guard.names().len(), 11);
        guard.release(&mut commands);
        assert!(!commands.exists("god"));
        assert!(!commands.exists("sizeup"));
    }

    #[test]
    fn non_q1_skips_net_commands() {
        let (mut commands, cvars, _) = buffer(Dialect::Q3);
        let guard = register_q1_client_commands(&mut commands, Dialect::Q3, &cvars, Rc::new(|_, _, _, _| {})).unwrap();
        assert!(commands.exists("god"));
        assert!(!commands.exists("pause"));
        assert_eq!(guard.names().len(), 8);
    }

    #[test]
    fn server_console_selects_slot() {
        let owner = IdentityOwner::create("q1-host").unwrap();
        let actor = owner.actor(3, 1);
        let client = owner.client(3, 1);
        let players = vec![Q1HostPlayer {
            actor: actor.clone(),
            client,
        }];
        let source = CommandContext::new(session(&owner), CommandOrigin::ServerConsole);
        let resolved = resolve_q1_host_command_actor(
            "god",
            &[String::from("3")],
            Some(&source),
            &players,
            || owner.actor(9, 9),
            false,
        )
        .unwrap();
        assert_eq!(resolved, actor);
        let missing = resolve_q1_host_command_actor(
            "god",
            &[String::from("4")],
            Some(&source),
            &players,
            || owner.actor(9, 9),
            false,
        )
        .unwrap_err();
        assert_eq!(missing, Q1HostCommandError::UnknownSlot { slot: "4".to_string() });
        let usage = resolve_q1_host_command_actor("god", &[], Some(&source), &players, || owner.actor(9, 9), false)
            .unwrap_err();
        assert!(matches!(usage, Q1HostCommandError::Usage { .. }));
    }

    #[test]
    fn seats_route_to_their_client() {
        let owner = IdentityOwner::create("q1-seat").unwrap();
        let seat = owner.seat(0);
        let client = owner.client(1, 1);
        let actor = owner.actor(1, 1);
        let players = vec![Q1HostPlayer {
            actor: actor.clone(),
            client: client.clone(),
        }];
        let source = CommandContext::new(session(&owner), CommandOrigin::LocalSeat { seat, client });
        let resolved =
            resolve_q1_host_command_actor("kill", &[], Some(&source), &players, || owner.actor(9, 9), false).unwrap();
        assert_eq!(resolved, actor);
        let forbidden = resolve_q1_host_command_actor(
            "kill",
            &[String::from("1")],
            Some(&source),
            &players,
            || owner.actor(9, 9),
            false,
        )
        .unwrap_err();
        assert!(matches!(forbidden, Q1HostCommandError::ConsoleOnly { .. }));
    }

    #[test]
    fn unadmitted_and_console_fall_through() {
        let owner = IdentityOwner::create("q1-fallthrough").unwrap();
        let source = CommandContext::new(
            session(&owner),
            CommandOrigin::RemoteClient {
                client: owner.client(7, 1),
            },
        );
        let denied =
            resolve_q1_host_command_actor("god", &[], Some(&source), &[], || owner.actor(9, 9), false).unwrap_err();
        assert_eq!(denied, Q1HostCommandError::NoAdmittedClient);
        let local = resolve_q1_host_command_actor("god", &[], None, &[], || owner.actor(9, 9), false).unwrap();
        assert_eq!(local, owner.actor(9, 9));
    }
}
