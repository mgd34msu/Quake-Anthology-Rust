//! Console LLM commands: `llm_ask`, `llm_exec`, `llm_cancel`.
//!
//! Donor provenance: `src/console/llm.ts` (`registerLlmCommands`).
//! Same direct-input gate, same prompt limits (nonempty, 4096 chars),
//! same started/cancelled/failed print texts, same whole-batch
//! validate-print-execute flow for `llm_exec`.
//!
//! The donor is async with per-seat pending requests, a 120-second timer,
//! and disposal that aborts transport; this synchronous single-seat host
//! runs each request inline, so at most one request is ever pending (the
//! slot guards reentrant input), the requester owns the timeout (the
//! settings service defaults to the same 120 seconds), and disposal is
//! plain unregistration. Batch execution and registry snapshots arrive
//! through [`ConsoleCommandServices`](super::commands::ConsoleCommandServices)
//! because handlers cannot borrow the registry they run inside.

use std::cell::RefCell;
use std::rc::Rc;

use crate::llm::errors::LlmError;
use crate::llm::request::{CancelToken, LlmRequestInput};
use crate::llm::settings::LlmSettingsService;

use super::commands::{CommandDocumentation, ConsoleCommands};
use super::llm_batch::{llm_console_catalog, llm_exec_instructions, validate_llm_batch, CatalogMode};

/// Something that answers one LLM request (donor `LlmCommandRequester`).
pub trait LlmCommandRequester {
    /// Answer `input`, observing `cancel`.
    fn request(&mut self, input: LlmRequestInput<'_>, cancel: &CancelToken) -> Result<String, LlmError>;
}

impl LlmCommandRequester for LlmSettingsService {
    fn request(&mut self, input: LlmRequestInput<'_>, cancel: &CancelToken) -> Result<String, LlmError> {
        self.request(input, cancel)
    }
}

/// Shared requester handle held by the registered commands.
pub type SharedRequester = Rc<RefCell<dyn LlmCommandRequester>>;

/// `llm_ask` system instructions (donor template).
fn ask_instructions(catalog: &str) -> String {
    format!(
        "Answer the user's question concisely, including general questions unrelated to the game. Your answer is displayed in a plain-text game console without Markdown rendering. Use short paragraphs and put command or code examples on their own lines. Do not use Markdown code fences, language headers, inline backticks, headings, or tables. Preserve meaningful code syntax and indentation.\nFor advice about this running engine, use only command and setting names from the registered catalog below. Follow their documented usage and allowed values. Do not assume that commands from another Quake engine exist here. If a requested setting or command is absent, say it is not registered; do not present a guessed name or a set command creating an unregistered variable as a working solution. If usage is undocumented, say so instead of inventing arguments. The catalog describes available commands, not current values or proof that an action is permitted. You have no console history, current settings, files, or game state. You only answer questions; do not claim to have executed commands.\n{catalog}"
    )
}

fn documented(summary: &str, usage: &str, examples: &[&str]) -> Option<CommandDocumentation> {
    Some(CommandDocumentation {
        summary: Some(summary.to_string()),
        usage: Some(usage.to_string()),
        examples: examples.iter().map(|example| (*example).to_string()).collect(),
        allowed_values: None,
    })
}

/// Register the LLM commands; returns names for later removal.
pub fn register_llm_commands(commands: &mut ConsoleCommands, requester: Option<SharedRequester>) -> Vec<String> {
    let mut names = Vec::new();
    let pending: Rc<RefCell<Option<CancelToken>>> = Rc::new(RefCell::new(None));
    {
        let pending = Rc::clone(&pending);
        if commands.register(
            "llm_cancel",
            move |invocation, services| {
                if !invocation.direct {
                    services.print("LLM commands require direct input from a local player console. Scripts, modules, and servers cannot start requests.\n");
                    return Ok(());
                }
                if invocation.argv.len() != 1 {
                    services.print("Usage: llm_cancel\n");
                    return Ok(());
                }
                match pending.borrow_mut().take() {
                    Some(token) => {
                        token.cancel();
                        services.print("LLM request cancelled.\n");
                    }
                    None => services.print("No LLM request is running in this console.\n"),
                }
                Ok(())
            },
            documented("Cancel this console's pending LLM request.", "llm_cancel", &["llm_cancel"]),
        ) {
            names.push("llm_cancel".to_string());
        }
    }
    for definition in [
        (
            "llm_ask",
            "llm_ask <question>",
            "Ask the configured LLM and print its answer in this console.",
            "llm_ask \"How do I change brightness?\"",
        ),
        (
            "llm_exec",
            "llm_exec <request>",
            "Ask the configured LLM for console commands, validate the whole batch, then print and execute it in your context.",
            "llm_exec \"Set brightness to the default\"",
        ),
    ] {
        let (name, usage, summary, example) = definition;
        let pending = Rc::clone(&pending);
        let requester = requester.clone();
        if commands.register(
            name,
            move |invocation, services| {
                if !invocation.direct {
                    services.print("LLM commands require direct input from a local player console. Scripts, modules, and servers cannot start requests.\n");
                    return Ok(());
                }
                let prompt = invocation.argv.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
                let prompt = prompt.trim().to_string();
                if prompt.is_empty() {
                    services.print(&format!("Usage: {usage}\nQuote text containing semicolons.\n"));
                    return Ok(());
                }
                let Some(requester) = requester.as_ref() else {
                    services.print("Configure a provider and model in Options > LLM before sending a request.\n");
                    return Ok(());
                };
                if prompt.len() > 4096 {
                    services.print("LLM prompt exceeds 4096 characters. Ask a shorter question.\n");
                    return Ok(());
                }
                if pending.borrow().is_some() {
                    services.print("An LLM request is already running in this console.\n");
                    return Ok(());
                }
                let token = CancelToken::new();
                *pending.borrow_mut() = Some(token.clone());
                let current = || pending.borrow().as_ref().is_some_and(|held| held.same(&token));
                let outcome: Result<String, String> = (|| {
                    let entries = services.discovery_entries();
                    let registry = services.llm_batch_registry();
                    let instructions = if invocation.argv.first().map_or("", String::as_str) == "llm_exec" {
                        llm_exec_instructions(&entries, &registry, &prompt).map_err(|error| error.to_string())?
                    } else {
                        let catalog =
                            llm_console_catalog(&entries, &prompt, CatalogMode::Ask).map_err(|error| error.to_string())?;
                        ask_instructions(&catalog)
                    };
                    services.print("LLM request started. Use llm_cancel to cancel.\n");
                    requester
                        .borrow_mut()
                        .request(
                            LlmRequestInput {
                                prompt: &prompt,
                                instructions: &instructions,
                                on_text: None,
                            },
                            &token,
                        )
                        .map_err(|error| error.to_string())
                })();
                if !current() {
                    return Ok(());
                }
                *pending.borrow_mut() = None;
                match outcome {
                    Ok(answer) => {
                        if invocation.argv.first().map_or("", String::as_str) == "llm_ask" {
                            services.print(&format!("{answer}\n"));
                            return Ok(());
                        }
                        match validate_llm_batch(&answer, &services.llm_batch_registry()) {
                            Ok(batch) => {
                                services.print(&format!("{}\n", batch.join("; ")));
                                services.execute_llm_batch(&(batch.join("\n") + "\n"));
                            }
                            Err(error) => services.print(&format!("LLM request failed: {error}\n")),
                        }
                    }
                    Err(message) => services.print(&format!("LLM request failed: {message}\n")),
                }
                Ok(())
            },
            documented(summary, usage, &[example]),
        ) {
            names.push(name.to_string());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::super::commands::ConsoleCommandServices;
    use super::super::discovery::{query_console_entries, ConsoleDiscoveryEntry};
    use super::super::llm_batch::LlmBatchRegistry;
    use super::*;
    use qa_core::cmd::{Dialect, TextMode};
    use qa_core::cvar::CvarRegistry;

    struct Fixture {
        output: Vec<String>,
        batches: Vec<String>,
        entries: Vec<ConsoleDiscoveryEntry>,
        registry: LlmBatchRegistry,
    }

    impl ConsoleCommandServices for Fixture {
        fn print(&mut self, text: &str) {
            self.output.push(text.to_string());
        }

        fn forward_to_server(&mut self, _line: &str) {}

        fn toggle_console(&mut self) {}

        fn clear_console(&mut self) {}

        fn message_mode(&mut self, _team: bool) {}

        fn can_chat(&self) -> bool {
            false
        }

        fn console_dump(&self) -> String {
            String::new()
        }

        fn write_file(&mut self, _path: &str, _contents: &str) -> Result<(), String> {
            Ok(())
        }

        fn configuration_text(&mut self, _argv: &[String]) -> String {
            String::new()
        }

        fn map_name(&self) -> String {
            String::new()
        }

        fn discovery_entries(&self) -> Vec<ConsoleDiscoveryEntry> {
            self.entries.clone()
        }

        fn llm_batch_registry(&self) -> LlmBatchRegistry {
            self.registry.clone()
        }

        fn execute_llm_batch(&mut self, batch: &str) {
            self.batches.push(batch.to_string());
        }
    }

    struct StubRequester {
        answers: Vec<String>,
        prompts: Vec<String>,
        instructions: Vec<String>,
    }

    impl LlmCommandRequester for StubRequester {
        fn request(&mut self, input: LlmRequestInput<'_>, _cancel: &CancelToken) -> Result<String, LlmError> {
            self.prompts.push(input.prompt.to_string());
            self.instructions.push(input.instructions.to_string());
            Ok(self.answers.remove(0))
        }
    }

    fn harness(
        dialect: Dialect,
        answers: Vec<&str>,
    ) -> (
        ConsoleCommands,
        CvarRegistry,
        Fixture,
        Rc<RefCell<StubRequester>>,
        SharedRequester,
    ) {
        let mut commands = ConsoleCommands::new();
        commands.register("game_action", |_, _| Ok(()), None);
        let mut cvars = CvarRegistry::new(dialect);
        cvars.register("sensitivity", "3", 0).unwrap();
        let entries = query_console_entries(&commands, &cvars);
        let registry = LlmBatchRegistry::snapshot(&commands, &cvars, dialect);
        let fixture = Fixture {
            output: Vec::new(),
            batches: Vec::new(),
            entries,
            registry,
        };
        let stub = Rc::new(RefCell::new(StubRequester {
            answers: answers.iter().map(|answer| (*answer).to_string()).collect(),
            prompts: Vec::new(),
            instructions: Vec::new(),
        }));
        let requester: SharedRequester = stub.clone();
        (commands, cvars, fixture, stub, requester)
    }

    fn send(commands: &mut ConsoleCommands, cvars: &mut CvarRegistry, services: &mut Fixture, text: &str) {
        send_dialect(commands, cvars, services, Dialect::Q3, text);
    }

    fn send_dialect(
        commands: &mut ConsoleCommands,
        cvars: &mut CvarRegistry,
        services: &mut Fixture,
        dialect: Dialect,
        text: &str,
    ) {
        commands
            .execute(text, dialect, TextMode::Console, cvars, services)
            .unwrap();
    }

    #[test]
    fn ask_prints_answers_without_executing() {
        let (mut commands, mut cvars, mut services, _stub, requester) =
            harness(Dialect::Q3, vec!["That setting is not registered."]);
        register_llm_commands(&mut commands, Some(requester));
        send(
            &mut commands,
            &mut cvars,
            &mut services,
            "llm_ask \"How do I adjust view?\"\n",
        );
        assert_eq!(services.output.len(), 2);
        assert!(services.output[0].contains("llm_cancel"));
        assert_eq!(services.output[1], "That setting is not registered.\n");
        assert!(services.batches.is_empty());
        assert_eq!(cvars.variable_string("sensitivity"), "3");
    }

    #[test]
    fn exec_validates_the_whole_batch_before_any_action() {
        let (mut commands, mut cvars, mut services, _stub, requester) =
            harness(Dialect::Q3, vec!["game_action; imaginary_command"]);
        register_llm_commands(&mut commands, Some(requester));
        send(&mut commands, &mut cvars, &mut services, "llm_exec \"do a thing\"\n");
        assert_eq!(services.output.len(), 2);
        assert!(services.output[1].contains("Unknown command"));
        assert!(services.batches.is_empty());
    }

    #[test]
    fn exec_prints_and_dispatches_valid_batches() {
        let (mut commands, mut cvars, mut services, _stub, requester) =
            harness(Dialect::Q3, vec!["sensitivity 5; game_action"]);
        register_llm_commands(&mut commands, Some(requester));
        send(&mut commands, &mut cvars, &mut services, "llm_exec \"make changes\"\n");
        assert_eq!(services.output[1], "sensitivity 5; game_action\n");
        assert_eq!(services.batches, vec!["sensitivity 5\ngame_action\n".to_string()]);
    }

    #[test]
    fn rejects_alias_origins_usage_errors_and_missing_providers() {
        let (mut commands, mut cvars, mut services, stub, requester) = harness(Dialect::Q3, vec!["late"]);
        let names = register_llm_commands(&mut commands, Some(Rc::clone(&requester)));
        assert_eq!(
            names,
            vec!["llm_cancel".to_string(), "llm_ask".to_string(), "llm_exec".to_string()]
        );
        commands.set_alias("paid", "llm_ask \"question\"\n");
        send_dialect(&mut commands, &mut cvars, &mut services, Dialect::Q1Netquake, "paid\n");
        assert!(services.output[0].contains("require direct input"));
        assert!(stub.borrow().prompts.is_empty());
        send(&mut commands, &mut cvars, &mut services, "llm_ask\n");
        assert!(services.output[1].contains("Usage: llm_ask <question>"));
        send(
            &mut commands,
            &mut cvars,
            &mut services,
            &format!("llm_ask {}\n", "x".repeat(4097)),
        );
        assert!(services.output[2].contains("exceeds 4096"));
        send(&mut commands, &mut cvars, &mut services, "llm_cancel extra\n");
        assert!(services.output[3].contains("Usage: llm_cancel"));
        send(&mut commands, &mut cvars, &mut services, "llm_cancel\n");
        assert!(services.output[4].contains("No LLM request is running"));
        for name in names {
            assert!(commands.unregister(&name));
        }
        assert!(!commands.exists("llm_ask"));

        let (mut commands, mut cvars, mut services, _, _) = harness(Dialect::Q3, vec![]);
        register_llm_commands(&mut commands, None);
        send(&mut commands, &mut cvars, &mut services, "llm_ask \"hi\"\n");
        assert!(services.output[0].contains("Options > LLM"));
    }

    #[test]
    fn ask_instructions_ground_answers_in_the_live_catalog() {
        let (mut commands, mut cvars, mut services, stub, requester) = harness(Dialect::Q3, vec!["ok"]);
        register_llm_commands(&mut commands, Some(Rc::clone(&requester)));
        send(
            &mut commands,
            &mut cvars,
            &mut services,
            "llm_ask \"adjust sensitivity\"\n",
        );
        let stub = stub.borrow();
        let instructions = &stub.instructions;
        assert!(instructions[0].contains("plain-text game console without Markdown rendering"));
        assert!(instructions[0].contains("cvar:sensitivity"));
        assert!(!instructions[0].contains("\"3\""));
    }

    #[test]
    fn request_failures_print_without_executing() {
        struct Failing;
        impl LlmCommandRequester for Failing {
            fn request(&mut self, _: LlmRequestInput<'_>, _: &CancelToken) -> Result<String, LlmError> {
                Err(LlmError::settings("LLM request cancelled."))
            }
        }
        let (mut commands, mut cvars, _, _, _) = harness(Dialect::Q3, vec![]);
        let entries = query_console_entries(&commands, &cvars);
        let registry = LlmBatchRegistry::snapshot(&commands, &cvars, Dialect::Q3);
        let mut services = Fixture {
            output: Vec::new(),
            batches: Vec::new(),
            entries,
            registry,
        };
        register_llm_commands(&mut commands, Some(Rc::new(RefCell::new(Failing))));
        send(&mut commands, &mut cvars, &mut services, "llm_exec \"do it\"\n");
        assert!(services.output[1].contains("LLM request failed: LLM request cancelled."));
        assert!(services.batches.is_empty());
    }
}
