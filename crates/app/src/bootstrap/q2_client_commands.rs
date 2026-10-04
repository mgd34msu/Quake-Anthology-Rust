//! Quake II client command registration over the shared command buffer.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q2-client-commands.ts`
//! (`registerQ2ClientCommands`). Command definitions come from the ported catalog
//! [`Q2_CLIENT_COMMANDS`](qa_content::q2::base::player::commands::Q2_CLIENT_COMMANDS). The
//! merged [`CommandBuffer`](qa_core::cmd_buffer::CommandBuffer) needs the live registry for
//! cvar-name conflicts, so registration takes it explicitly and reports
//! [`BufferError`](qa_core::cmd_buffer::BufferError); the release closure becomes a guard.

use std::rc::Rc;

use qa_content::q2::base::player::commands::Q2_CLIENT_COMMANDS;
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{BufferError, CommandBuffer, CommandContext, CommandDocumentation};
use qa_core::cvar::CvarRegistry;
use qa_core::identity::SeatId;

use super::q1_client_commands::register_forwarded_client_command;

/// Host callback for client commands (donor `execute`).
pub type Q2ClientCommandExecute = Rc<dyn Fn(&str, &[String], Option<SeatId>, &CommandContext)>;

/// Registered command names with their release (donor return closure).
#[derive(Debug, Default)]
pub struct Q2ClientCommandGuard {
    names: Vec<String>,
}

impl Q2ClientCommandGuard {
    /// Names this guard releases.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Unregister every command this guard owns.
    pub fn release(self, commands: &mut CommandBuffer) {
        for name in &self.names {
            commands.unregister(name);
        }
    }
}

/// Register the Q2 client commands (donor `registerQ2ClientCommands`).
pub fn register_q2_client_commands(
    commands: &mut CommandBuffer,
    dialect: Dialect,
    cvars: &CvarRegistry,
    execute: Q2ClientCommandExecute,
) -> Result<Q2ClientCommandGuard, BufferError> {
    let mut names = Vec::new();
    if dialect == Dialect::Q2Classic || dialect == Dialect::Q2Rerelease {
        for definition in Q2_CLIENT_COMMANDS {
            let name = if definition.name == "help" {
                "gamehelp"
            } else {
                definition.name
            };
            if commands.exists(name) {
                continue;
            }
            let documentation = if definition.name == "help" {
                CommandDocumentation {
                    summary: format!(
                        "{} For console command help, use help <name>.",
                        definition.documentation.summary
                    ),
                    usage: "gamehelp".to_string(),
                    examples: vec!["gamehelp".to_string()],
                    allowed_values: None,
                }
            } else {
                CommandDocumentation {
                    summary: definition.documentation.summary.to_string(),
                    usage: definition.documentation.usage.to_string(),
                    examples: definition
                        .documentation
                        .examples
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    allowed_values: None,
                }
            };
            let registered = register_forwarded_client_command(
                commands,
                cvars,
                name,
                definition.name,
                documentation,
                None,
                &execute,
            )?;
            if registered {
                names.push(name.to_string());
            }
        }
    }
    Ok(Q2ClientCommandGuard { names })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd_buffer::{BufferOptions, CommandOrigin};
    use qa_core::identity::IdentityOwner;

    fn buffer(dialect: Dialect) -> (CommandBuffer, CvarRegistry) {
        let owner = IdentityOwner::create("q2-commands").unwrap();
        let context = CommandContext::new(owner.session().clone(), CommandOrigin::LocalConsole);
        let buffer = CommandBuffer::new(dialect, context, BufferOptions::default()).unwrap();
        (buffer, CvarRegistry::new(dialect))
    }

    #[test]
    fn registers_catalog_and_renames_help() {
        let (mut commands, cvars) = buffer(Dialect::Q2Rerelease);
        let guard =
            register_q2_client_commands(&mut commands, Dialect::Q2Rerelease, &cvars, Rc::new(|_, _, _, _| {})).unwrap();
        assert!(commands.exists("gamehelp"));
        assert!(!commands.exists("help"));
        assert!(commands.exists("say"));
        assert_eq!(guard.names().len(), Q2_CLIENT_COMMANDS.len());
        let docs = commands.command_documentation("gamehelp").unwrap();
        assert!(docs.summary.contains("For console command help"), "{}", docs.summary);
        assert_eq!(docs.usage, "gamehelp");
        guard.release(&mut commands);
        assert!(!commands.exists("say"));
    }

    #[test]
    fn non_q2_registers_nothing() {
        let (mut commands, cvars) = buffer(Dialect::Q3);
        let guard = register_q2_client_commands(&mut commands, Dialect::Q3, &cvars, Rc::new(|_, _, _, _| {})).unwrap();
        assert!(guard.names().is_empty());
    }
}
