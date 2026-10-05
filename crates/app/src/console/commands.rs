//! Console command dispatch and builtin commands.
//!
//! Donor provenance: `src/console/commands.ts`
//! (`registerConsoleCommands`: `toggleconsole`, `clear`, `messagemode`,
//! `condump`, `writeconfig`, `screenshot`, `levelshot`) with fallback
//! behavior from `src/core/commands/index.ts` (`CommandBuffer` cvar
//! fallback, `Unknown command`, Q2 `ALIAS_LOOP_COUNT`) and camera commands
//! from `src/camera/application.ts` (`loadcamera`, `startcamera`,
//! `stopcamera`, `savecamera`).
//!
//! [`ConsoleCommands::execute`] dispatches multi-command lines inline:
//! text tokenizes through [`qa_core::cmd`], handlers run synchronously,
//! and variables resolve through the caller's
//! [`qa_core::cvar::CvarRegistry`]. The buffered queue in [`super::queue`]
//! layers `wait` countdowns, script origins, per-seat routing, and
//! multi-frame programs above this registry and reaches registered
//! commands through [`ConsoleCommands::invoke_registered`].

use std::collections::HashMap;

use qa_client::capture::ScreenshotFormat;
use qa_core::cmd::{ascii_fold, command_separator_offset, tokenize_command, Dialect, TextMode};
use qa_core::cvar::CvarRegistry;

use super::ConsoleError;

/// Documented command help (donor `CommandDocumentation`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandDocumentation {
    /// One-line summary.
    pub summary: Option<String>,
    /// Usage line.
    pub usage: Option<String>,
    /// Example invocations.
    pub examples: Vec<String>,
    /// Allowed values, when the command takes an enum.
    pub allowed_values: Option<Vec<String>>,
}

/// One tokenized command invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInvocation {
    /// Argument vector.
    pub argv: Vec<String>,
    /// Raw text after the first token.
    pub args_text: String,
    /// Dispatch dialect.
    pub dialect: Dialect,
    /// Raw segment text (for server forwarding).
    pub raw: String,
    /// Whether the line reached dispatch without alias expansion.
    pub direct: bool,
}

/// Host services behind console command handlers.
pub trait ConsoleCommandServices {
    /// Print console output.
    fn print(&mut self, text: &str);
    /// Forward a line to the server (Q2/Q3 unknown commands, Q3 levelshot).
    fn forward_to_server(&mut self, line: &str);
    /// Toggle the console open state.
    fn toggle_console(&mut self);
    /// Clear the console buffer.
    fn clear_console(&mut self);
    /// Open message mode (`team` selects `messagemode2`).
    fn message_mode(&mut self, team: bool);
    /// Whether chat has an active connection.
    fn can_chat(&self) -> bool;
    /// Current scrollback as `condump` text.
    fn console_dump(&self) -> String;
    /// Write a config/dump file under the writable root.
    fn write_file(&mut self, path: &str, contents: &str) -> Result<(), String>;
    /// Render `writeconfig` contents for the invocation.
    fn configuration_text(&mut self, argv: &[String]) -> String;
    /// Current map name for captures.
    fn map_name(&self) -> String;
    /// Whether a seat renderer offers frame readback.
    fn has_capture(&self) -> bool {
        false
    }
    /// Capture a screenshot, returning the written path.
    fn capture_screenshot(&mut self, _format: ScreenshotFormat, _name: Option<&str>) -> Result<String, String> {
        Err("Screenshot requires an active seat renderer".to_string())
    }
    /// Capture a levelshot for `map`, returning the written path.
    fn capture_levelshot(&mut self, _map: &str) -> Result<String, String> {
        Err("Screenshot requires an active seat renderer".to_string())
    }
    /// Load a spline camera, returning the `Loaded camera ...` line.
    fn camera_load(&mut self, _path: &str) -> Result<String, String> {
        Err("No camera loaded".to_string())
    }
    /// Start spline camera playback.
    fn camera_start(&mut self) -> Result<(), String> {
        Err("No camera loaded".to_string())
    }
    /// Stop spline camera playback.
    fn camera_stop(&mut self) {}
    /// Save the loaded spline camera.
    fn camera_save(&mut self, _path: &str) -> Result<(), String> {
        Err("No camera loaded".to_string())
    }
    /// Discovery snapshot for `find`/`help`, refreshed before each line.
    fn discovery_entries(&self) -> Vec<super::discovery::ConsoleDiscoveryEntry> {
        Vec::new()
    }
    /// Registry snapshot for `llm_exec` validation, refreshed before each line.
    fn llm_batch_registry(&self) -> super::llm_batch::LlmBatchRegistry {
        super::llm_batch::LlmBatchRegistry::for_dialect(Dialect::Q3)
    }
    /// Execute an `llm_exec` batch the handler already validated and printed.
    fn execute_llm_batch(&mut self, _batch: &str) {}
}

type Handler = Box<dyn FnMut(&CommandInvocation, &mut dyn ConsoleCommandServices) -> Result<(), ConsoleError>>;

struct Entry {
    name: String,
    handler: Handler,
    documentation: Option<CommandDocumentation>,
}

/// Synchronous console command registry with aliases.
#[derive(Default)]
pub struct ConsoleCommands {
    entries: HashMap<String, Entry>,
    order: Vec<String>,
    aliases: HashMap<String, String>,
}

impl ConsoleCommands {
    /// Open an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command; returns false when the name is taken.
    pub fn register(
        &mut self,
        name: &str,
        handler: impl FnMut(&CommandInvocation, &mut dyn ConsoleCommandServices) -> Result<(), ConsoleError> + 'static,
        documentation: Option<CommandDocumentation>,
    ) -> bool {
        let key = ascii_fold(name);
        if self.entries.contains_key(&key) {
            return false;
        }
        self.order.push(key.clone());
        self.entries.insert(
            key,
            Entry {
                name: name.to_string(),
                handler: Box::new(handler),
                documentation,
            },
        );
        true
    }

    /// Remove a command; returns whether one was registered.
    pub fn unregister(&mut self, name: &str) -> bool {
        let key = ascii_fold(name);
        if self.entries.remove(&key).is_none() {
            return false;
        }
        self.order.retain(|order| order != &key);
        true
    }

    /// Whether a command is registered (ASCII-folded).
    #[must_use]
    pub fn exists(&self, name: &str) -> bool {
        self.entries.contains_key(&ascii_fold(name))
    }

    /// Registered command names in registration order.
    #[must_use]
    pub fn registered_names(&self) -> Vec<String> {
        self.order
            .iter()
            .filter_map(|key| self.entries.get(key).map(|entry| entry.name.clone()))
            .collect()
    }

    /// Documentation for a registered command.
    #[must_use]
    pub fn command_documentation(&self, name: &str) -> Option<&CommandDocumentation> {
        self.entries
            .get(&ascii_fold(name))
            .and_then(|entry| entry.documentation.as_ref())
    }

    /// Define an alias.
    pub fn set_alias(&mut self, name: &str, value: &str) {
        self.aliases.insert(ascii_fold(name), value.to_string());
    }

    /// Remove an alias.
    pub fn remove_alias(&mut self, name: &str) -> bool {
        self.aliases.remove(&ascii_fold(name)).is_some()
    }

    /// Alias names.
    #[must_use]
    pub fn alias_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.aliases.keys().cloned().collect();
        names.sort();
        names
    }

    /// Alias expansion text.
    #[must_use]
    pub fn alias_value(&self, name: &str) -> Option<&str> {
        self.aliases.get(&ascii_fold(name)).map(String::as_str)
    }

    /// Invoke one registered command without alias expansion, cvar
    /// fallback, or server forwarding. Returns `None` when no command owns
    /// the name; the buffered queue owns those layers per the donor
    /// `CommandBuffer` and calls here for registered console commands.
    pub fn invoke_registered(
        &mut self,
        invocation: &CommandInvocation,
        services: &mut dyn ConsoleCommandServices,
    ) -> Option<Result<(), ConsoleError>> {
        let name = invocation.argv.first()?;
        let key = ascii_fold(name);
        let entry = self.entries.get_mut(&key)?;
        Some((entry.handler)(invocation, services))
    }

    /// Execute `;`/newline-separated console text.
    pub fn execute(
        &mut self,
        text: &str,
        dialect: Dialect,
        mode: TextMode,
        cvars: &mut CvarRegistry,
        services: &mut dyn ConsoleCommandServices,
    ) -> Result<(), ConsoleError> {
        let mut rest = qa_core::cmd::source_command_text(text);
        while !rest.is_empty() {
            let offset = command_separator_offset(&rest, dialect);
            let segment = rest[..offset].to_string();
            let skip = rest[offset..].chars().next().map_or(0, |next| next.len_utf8());
            rest = rest[offset + skip..].to_string();
            self.execute_segment(&segment, dialect, mode, cvars, services)?;
        }
        Ok(())
    }

    fn execute_segment(
        &mut self,
        segment: &str,
        dialect: Dialect,
        mode: TextMode,
        cvars: &mut CvarRegistry,
        services: &mut dyn ConsoleCommandServices,
    ) -> Result<(), ConsoleError> {
        let mut line = segment.to_string();
        let mut expansions = 0;
        let tokens = loop {
            let tokens = tokenize_command(&line, dialect, mode)?;
            let Some(name) = tokens.argv.first().cloned() else {
                return Ok(());
            };
            if dialect == Dialect::Q3 {
                break tokens;
            }
            let Some(alias) = self.aliases.get(&ascii_fold(&name)).cloned() else {
                break tokens;
            };
            expansions += 1;
            if dialect.is_q2() && expansions == 16 {
                services.print("ALIAS_LOOP_COUNT\n");
                return Ok(());
            }
            if expansions > 64 {
                services.print("ALIAS_LOOP_COUNT\n");
                return Ok(());
            }
            line = if tokens.args_text.trim().is_empty() {
                alias
            } else {
                format!("{} {}", alias.trim_end(), tokens.args_text)
            };
        };
        let Some(name) = tokens.argv.first().cloned() else {
            return Ok(());
        };
        let invocation = CommandInvocation {
            argv: tokens.argv.clone(),
            args_text: tokens.args_text.clone(),
            dialect,
            raw: segment.to_string(),
            direct: expansions == 0,
        };
        if self.entries.contains_key(&ascii_fold(&name)) {
            let entry = self
                .entries
                .get_mut(&ascii_fold(&name))
                .expect("console command vanished");
            return (entry.handler)(&invocation, services);
        }
        if let Some(variable) = cvars.get(&name) {
            match tokens.argv.get(1) {
                None => {
                    if dialect == Dialect::Q3 {
                        services.print(&format!(
                            "\"{}\" is:\"{}^7\" default:\"{}^7\"\n",
                            variable.name, variable.value, variable.reset_value
                        ));
                        if let Some(latched) = variable.latched_value {
                            services.print(&format!("latched: \"{latched}\"\n"));
                        }
                    } else {
                        services.print(&format!("\"{}\" is \"{}\"\n", variable.name, variable.value));
                    }
                }
                Some(value) => {
                    cvars.set(&variable.name, value, false)?;
                }
            }
            return Ok(());
        }
        if dialect == Dialect::Q3 || dialect.is_q2() {
            services.forward_to_server(segment);
            return Ok(());
        }
        if dialect != Dialect::Q1Quakeworld
            || cvars.get("cl_warncmd").is_some_and(|warn| warn.numeric_value != 0.0)
            || cvars
                .get("developer")
                .is_some_and(|developer| developer.numeric_value != 0.0)
        {
            services.print(&format!("Unknown command \"{name}\"\n"));
        }
        Ok(())
    }
}

/// Register the donor console builtins; returns names for later removal.
pub fn register_console_commands(commands: &mut ConsoleCommands) -> Vec<String> {
    let mut names = Vec::new();
    let add = |commands: &mut ConsoleCommands,
               names: &mut Vec<String>,
               name: &str,
               handler: Handler,
               documentation: Option<CommandDocumentation>| {
        if commands.entries.contains_key(&ascii_fold(name)) {
            return;
        }
        commands.order.push(ascii_fold(name));
        commands.entries.insert(
            ascii_fold(name),
            Entry {
                name: name.to_string(),
                handler,
                documentation,
            },
        );
        names.push(name.to_string());
    };
    add(
        commands,
        &mut names,
        "toggleconsole",
        Box::new(|_, services| {
            services.toggle_console();
            Ok(())
        }),
        None,
    );
    add(
        commands,
        &mut names,
        "clear",
        Box::new(|_, services| {
            services.clear_console();
            Ok(())
        }),
        Some(CommandDocumentation {
            summary: Some("Clear the invoking seat's console output.".to_string()),
            usage: Some("clear".to_string()),
            examples: vec!["clear".to_string()],
            allowed_values: None,
        }),
    );
    for name in ["messagemode", "messagemode2"] {
        let team = name == "messagemode2";
        add(
            commands,
            &mut names,
            name,
            Box::new(move |_, services| {
                if !services.can_chat() {
                    services.print("Chat requires an active connection.\n");
                    return Ok(());
                }
                services.message_mode(team);
                Ok(())
            }),
            None,
        );
    }
    add(
        commands,
        &mut names,
        "condump",
        Box::new(|invocation, services| {
            if invocation.argv.len() != 2 {
                services.print("condump <filename>\n");
                return Ok(());
            }
            let name = invocation.argv[1].clone();
            let path = if invocation.dialect.is_q2() && !name.ends_with(".txt") {
                format!("{name}.txt")
            } else {
                name
            };
            let contents = services.console_dump();
            services
                .write_file(&path, &contents)
                .map_err(ConsoleError::BadCommand)?;
            services.print(&format!("Dumped console text to {path}\n"));
            Ok(())
        }),
        Some(CommandDocumentation {
            summary: Some("Write the invoking seat's console output to a file.".to_string()),
            usage: Some("condump <filename>".to_string()),
            examples: vec!["condump console.txt".to_string()],
            allowed_values: None,
        }),
    );
    add(
        commands,
        &mut names,
        "writeconfig",
        Box::new(|invocation, services| {
            let name = invocation
                .argv
                .get(1)
                .cloned()
                .unwrap_or_else(|| "config.cfg".to_string());
            let path = if name.ends_with(".cfg") {
                name
            } else {
                format!("{name}.cfg")
            };
            let contents = services.configuration_text(&invocation.argv);
            services
                .write_file(&path, &contents)
                .map_err(ConsoleError::BadCommand)?;
            services.print(&format!("Wrote {path}\n"));
            Ok(())
        }),
        Some(CommandDocumentation {
            summary: Some("Save the invoking seat's configuration.".to_string()),
            usage: Some("writeconfig [filename]".to_string()),
            examples: vec!["writeconfig config.cfg".to_string()],
            allowed_values: None,
        }),
    );
    for (name, format) in [
        ("screenshot", ScreenshotFormat::Tga),
        ("screenshotJPEG", ScreenshotFormat::Jpg),
        ("screenshotPNG", ScreenshotFormat::Png),
    ] {
        add(
            commands,
            &mut names,
            name,
            Box::new(move |invocation, services| {
                if !services.has_capture() {
                    services.print("Screenshot requires an active seat renderer\n");
                    return Ok(());
                }
                let map = services.map_name();
                let argument = invocation.argv.get(1).cloned();
                let path = if argument.as_deref() == Some("levelshot") {
                    services.capture_levelshot(&map).map_err(ConsoleError::Capture)?
                } else {
                    let named = match argument.as_deref() {
                        None | Some("silent") => None,
                        Some(name) => Some(name),
                    };
                    services
                        .capture_screenshot(format, named)
                        .map_err(ConsoleError::Capture)?
                };
                if argument.as_deref() != Some("silent") {
                    services.print(&format!("Wrote {path}\n"));
                }
                Ok(())
            }),
            None,
        );
    }
    add(
        commands,
        &mut names,
        "levelshot",
        Box::new(|invocation, services| {
            if invocation.dialect == Dialect::Q3 {
                services.forward_to_server(&invocation.raw);
                return Ok(());
            }
            if services.has_capture() {
                let map = services.map_name();
                let path = services.capture_levelshot(&map).map_err(ConsoleError::Capture)?;
                services.print(&format!("Wrote {path}\n"));
            }
            Ok(())
        }),
        None,
    );
    add(
        commands,
        &mut names,
        "loadcamera",
        Box::new(|invocation, services| {
            if invocation.argv.len() != 2 {
                services.print("Camera: Usage: loadcamera <file.camera>\n");
                return Ok(());
            }
            match services.camera_load(&invocation.argv[1]) {
                Ok(line) => services.print(&line),
                Err(error) => services.print(&format!("Camera: {error}\n")),
            }
            Ok(())
        }),
        Some(CommandDocumentation {
            summary: Some("Control the loaded source spline camera.".to_string()),
            usage: Some("loadcamera <file.camera>".to_string()),
            examples: Vec::new(),
            allowed_values: None,
        }),
    );
    add(
        commands,
        &mut names,
        "startcamera",
        Box::new(|invocation, services| {
            if invocation.argv.len() != 1 {
                services.print("Camera: Usage: startcamera\n");
                return Ok(());
            }
            if let Err(error) = services.camera_start() {
                services.print(&format!("Camera: {error}\n"));
            }
            Ok(())
        }),
        Some(CommandDocumentation {
            summary: Some("Control the loaded source spline camera.".to_string()),
            usage: Some("startcamera".to_string()),
            examples: Vec::new(),
            allowed_values: None,
        }),
    );
    add(
        commands,
        &mut names,
        "stopcamera",
        Box::new(|_, services| {
            services.camera_stop();
            Ok(())
        }),
        Some(CommandDocumentation {
            summary: Some("Control the loaded source spline camera.".to_string()),
            usage: Some("stopcamera".to_string()),
            examples: Vec::new(),
            allowed_values: None,
        }),
    );
    add(
        commands,
        &mut names,
        "savecamera",
        Box::new(|invocation, services| {
            if invocation.argv.len() != 2 {
                services.print("Camera: Usage: savecamera <file.camera>\n");
                return Ok(());
            }
            if let Err(error) = services.camera_save(&invocation.argv[1]) {
                services.print(&format!("Camera: {error}\n"));
            }
            Ok(())
        }),
        Some(CommandDocumentation {
            summary: Some("Control the loaded source spline camera.".to_string()),
            usage: Some("savecamera <file.camera>".to_string()),
            examples: Vec::new(),
            allowed_values: None,
        }),
    );
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        printed: Vec<String>,
        forwarded: Vec<String>,
        toggles: usize,
        clears: usize,
        files: HashMap<String, String>,
    }

    impl ConsoleCommandServices for Fixture {
        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }

        fn forward_to_server(&mut self, line: &str) {
            self.forwarded.push(line.to_string());
        }

        fn toggle_console(&mut self) {
            self.toggles += 1;
        }

        fn clear_console(&mut self) {
            self.clears += 1;
        }

        fn message_mode(&mut self, team: bool) {
            self.printed.push(format!("message:{team}"));
        }

        fn can_chat(&self) -> bool {
            true
        }

        fn console_dump(&self) -> String {
            "dumped\n".to_string()
        }

        fn write_file(&mut self, path: &str, contents: &str) -> Result<(), String> {
            self.files.insert(path.to_string(), contents.to_string());
            Ok(())
        }

        fn configuration_text(&mut self, _argv: &[String]) -> String {
            "unbindall\n".to_string()
        }

        fn map_name(&self) -> String {
            "q3dm1".to_string()
        }

        fn has_capture(&self) -> bool {
            true
        }

        fn capture_screenshot(&mut self, format: ScreenshotFormat, name: Option<&str>) -> Result<String, String> {
            Ok(format!("/shots/{}/{}.png", format.extension(), name.unwrap_or("auto")))
        }

        fn capture_levelshot(&mut self, map: &str) -> Result<String, String> {
            Ok(format!("/levelshots/{map}.tga"))
        }
    }

    fn harness() -> (ConsoleCommands, CvarRegistry, Fixture) {
        let mut commands = ConsoleCommands::new();
        let names = register_console_commands(&mut commands);
        assert!(names.contains(&"condump".to_string()));
        let cvars = CvarRegistry::new(Dialect::Q2Classic);
        (
            commands,
            cvars,
            Fixture {
                printed: Vec::new(),
                forwarded: Vec::new(),
                toggles: 0,
                clears: 0,
                files: HashMap::new(),
            },
        )
    }

    #[test]
    fn dispatches_commands_aliases_and_cvars() {
        let (mut commands, mut cvars, mut services) = harness();
        cvars.register("volume", "0.7", 0).unwrap();
        commands.set_alias("v", "volume");
        commands
            .execute(
                "toggleconsole; v; volume 0.9",
                Dialect::Q2Classic,
                TextMode::Source,
                &mut cvars,
                &mut services,
            )
            .unwrap();
        assert_eq!(services.toggles, 1);
        assert_eq!(services.printed[0], "\"volume\" is \"0.7\"\n");
        assert_eq!(cvars.get("volume").unwrap().value, "0.9");
        commands
            .execute("volume", Dialect::Q3, TextMode::Source, &mut cvars, &mut services)
            .unwrap();
        assert!(services.printed.last().unwrap().contains("default:"));
    }

    #[test]
    fn reports_unknown_commands_and_alias_loops() {
        let (mut commands, mut cvars, mut services) = harness();
        commands
            .execute(
                "bogus",
                Dialect::Q1Netquake,
                TextMode::Source,
                &mut cvars,
                &mut services,
            )
            .unwrap();
        assert_eq!(services.printed, vec!["Unknown command \"bogus\"\n".to_string()]);
        commands.set_alias("loop", "loop");
        commands
            .execute("loop", Dialect::Q2Classic, TextMode::Source, &mut cvars, &mut services)
            .unwrap();
        assert!(services.printed.contains(&"ALIAS_LOOP_COUNT\n".to_string()));
        commands
            .execute("bogus", Dialect::Q2Classic, TextMode::Source, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(services.forwarded, vec!["bogus".to_string()]);
    }

    #[test]
    fn dumps_config_and_captures() {
        let (mut commands, mut cvars, mut services) = harness();
        commands
            .execute(
                "condump session; writeconfig",
                Dialect::Q2Classic,
                TextMode::Source,
                &mut cvars,
                &mut services,
            )
            .unwrap();
        assert_eq!(services.files.get("session.txt").unwrap(), "dumped\n");
        assert_eq!(services.files.get("config.cfg").unwrap(), "unbindall\n");
        assert!(services
            .printed
            .iter()
            .any(|line| line.starts_with("Dumped console text to")));
        commands
            .execute(
                "screenshot silent; levelshot",
                Dialect::Q2Classic,
                TextMode::Source,
                &mut cvars,
                &mut services,
            )
            .unwrap();
        assert_eq!(
            services
                .printed
                .iter()
                .filter(|line| line.starts_with("Wrote "))
                .count(),
            2
        );
        commands
            .execute("levelshot", Dialect::Q3, TextMode::Source, &mut cvars, &mut services)
            .unwrap();
        assert_eq!(services.forwarded.last().unwrap(), "levelshot");
    }
}
