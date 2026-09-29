//! Seat console: buffer, fields, history, submit, and key input.
//!
//! Donor provenance: `src/console/session.ts` (`SeatConsole`). Same submit
//! rules (explicit `/` prefix, known-command check, QuakeWorld/Q3 chat
//! fallback), staged pre-publication output, toggle suppression, history
//! browsing, completion, and scroll keys. Seat identity and focus plumbing
//! collapse to a single headless seat: [`ConsoleFocus`] replaces the UI
//! focus contract and [`SeatConsoleServices`] replaces the per-seat host
//! callbacks.

use std::collections::HashSet;

use qa_client::input::KeyCode;
use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::cvar::CvarRegistry;

use super::buffer::ConsoleBuffer;
use super::commands::{ConsoleCommandServices, ConsoleCommands};
use super::discovery::{query_console_entries, ConsoleDiscoveryEntry};
use super::field::{ConsoleField, ConsoleHistory, KEYPAD_DOWN, KEYPAD_ENTER, KEYPAD_UP};
use super::ConsoleError;

/// Input focus for console key handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleFocus {
    /// Game input.
    Game,
    /// Console input.
    Console,
    /// Chat input.
    Chat {
        /// Whether the message targets the team.
        team: bool,
    },
}

/// Headless console input event.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleInputEvent {
    /// Text input.
    Text(String),
    /// Key press or release.
    Key {
        /// Key code.
        code: i32,
        /// Press (`true`) or release (`false`).
        down: bool,
        /// Auto-repeat.
        repeat: bool,
    },
    /// Mouse wheel scroll.
    MouseWheel {
        /// Scroll delta in lines.
        delta: f64,
    },
    /// Focus loss clears held keys and toggle suppression.
    Focus {
        /// Whether the seat gained focus.
        focused: bool,
    },
}

/// Host services behind one seat console.
pub trait SeatConsoleServices {
    /// Current time in milliseconds.
    fn now_ms(&mut self) -> i64;
    /// Whether chat has an active connection.
    fn connected(&self) -> bool;
    /// Read the clipboard, if any.
    fn clipboard(&mut self) -> Option<String>;
    /// Move input focus.
    fn set_focus(&mut self, focus: ConsoleFocus);
    /// Send a chat message.
    fn chat(&mut self, text: &str, team: bool, target: Option<i32>);
}

#[derive(Debug, Clone)]
struct StagedOutput {
    text: String,
    time_ms: i64,
    dialect: Dialect,
}

/// Single-seat headless console.
pub struct SeatConsole {
    /// Console edit field.
    pub field: ConsoleField,
    /// Chat edit field.
    pub chat_field: ConsoleField,
    /// Submit history.
    pub history: ConsoleHistory,
    /// Scrollback buffer.
    pub buffer: ConsoleBuffer,
    keys: HashSet<i32>,
    chat_target: Option<i32>,
    fraction: f64,
    opened: bool,
    suppress_toggle_text: bool,
    staged: Option<Vec<StagedOutput>>,
    dialect: Dialect,
}

impl SeatConsole {
    /// Open a console, staging pre-publication output when `staged`.
    pub fn new(dialect: Dialect, staged: bool) -> Self {
        Self {
            field: ConsoleField::new(),
            chat_field: ConsoleField::with_capacity(1023, 30),
            history: ConsoleHistory::new(),
            buffer: ConsoleBuffer::new(dialect),
            keys: HashSet::new(),
            chat_target: None,
            fraction: 0.0,
            opened: false,
            suppress_toggle_text: false,
            staged: staged.then(Vec::new),
            dialect,
        }
    }

    /// Whether the console is open.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.opened
    }

    /// Current open fraction (console slide animation).
    #[must_use]
    pub const fn fraction(&self) -> f64 {
        self.fraction
    }

    /// Discovery entry for the completion currently previewed.
    #[must_use]
    pub fn selected_completion_entry(
        &self,
        commands: &ConsoleCommands,
        cvars: &CvarRegistry,
    ) -> Option<ConsoleDiscoveryEntry> {
        let selected = self.field.selected_completion()?;
        query_console_entries(commands, cvars)
            .into_iter()
            .find(|entry| entry.name == selected)
    }

    /// Publish the console at a focus, dropping staged retention.
    pub fn publish(&mut self, focus: ConsoleFocus) {
        self.staged = None;
        self.opened = focus == ConsoleFocus::Console;
        self.keys.clear();
        self.suppress_toggle_text = false;
    }

    /// Adopt a staged candidate's output, then publish both.
    pub fn adopt(&mut self, candidate: &mut SeatConsole, focus: ConsoleFocus) -> Result<(), ConsoleError> {
        let Some(staged) = candidate.staged.take() else {
            return Err(ConsoleError::BadCommand(
                "Console candidate already published".to_string(),
            ));
        };
        for output in staged {
            self.buffer.set_dialect(output.dialect);
            self.buffer.print(&output.text, output.time_ms)?;
        }
        self.buffer.set_dialect(self.dialect);
        candidate.publish(focus);
        self.publish(focus);
        Ok(())
    }

    /// Print output, retaining it while staged.
    pub fn print(&mut self, text: &str, services: &mut dyn SeatConsoleServices) -> Result<(), ConsoleError> {
        let time_ms = services.now_ms();
        if let Some(staged) = self.staged.as_mut() {
            staged.push(StagedOutput {
                text: text.to_string(),
                time_ms,
                dialect: self.dialect,
            });
        }
        self.buffer.set_dialect(self.dialect);
        self.buffer.print(text, time_ms)?;
        Ok(())
    }

    /// Open the console.
    pub fn open(&mut self, services: &mut dyn SeatConsoleServices) {
        self.opened = true;
        self.field.clear();
        self.buffer.clear_notify();
        services.set_focus(ConsoleFocus::Console);
    }

    /// Close the console.
    pub fn close(&mut self, services: &mut dyn SeatConsoleServices) {
        self.opened = false;
        self.field.clear();
        self.buffer.clear_notify();
        services.set_focus(ConsoleFocus::Game);
    }

    /// Toggle the console.
    pub fn toggle(&mut self, services: &mut dyn SeatConsoleServices) {
        if self.opened {
            self.close(services);
        } else {
            self.open(services);
        }
    }

    /// Toggle from the console key, suppressing its text once.
    pub fn toggle_from_key(&mut self, repeat: bool, services: &mut dyn SeatConsoleServices) {
        self.suppress_toggle_text = true;
        if !repeat {
            self.toggle(services);
        }
    }

    /// Open message mode for an optional directed target.
    pub fn message(&mut self, team: bool, target: Option<i32>, services: &mut dyn SeatConsoleServices) {
        self.chat_field.clear();
        self.chat_target = target;
        services.set_focus(ConsoleFocus::Chat { team });
    }

    /// Step the open fraction toward its target; returns the fraction.
    pub fn animate(&mut self, open: bool, elapsed_ms: f64, speed: f64, target_fraction: f64) -> f64 {
        let target = if open { target_fraction } else { 0.0 };
        let step = speed * elapsed_ms / 1000.0;
        self.fraction = if self.fraction < target {
            (self.fraction + step).min(target)
        } else {
            (self.fraction - step).max(target)
        };
        self.fraction
    }

    /// Submit the field: chat fallback or command dispatch.
    pub fn submit(
        &mut self,
        commands: &mut ConsoleCommands,
        cvars: &mut CvarRegistry,
        console: &mut dyn ConsoleCommandServices,
        seat: &mut dyn SeatConsoleServices,
    ) -> Result<(), ConsoleError> {
        let text = self.field.text();
        if text.is_empty() {
            return Ok(());
        }
        self.print(&format!("]{text}\n"), seat)?;
        let trimmed = text.trim_start().to_string();
        let explicit = trimmed.starts_with('/') || trimmed.starts_with('\\');
        let probe = if explicit {
            trimmed[1..].to_string()
        } else {
            trimmed.clone()
        };
        let first = tokenize_command(&probe, self.dialect, TextMode::Source)?
            .argv
            .first()
            .cloned()
            .unwrap_or_default();
        let known = query_console_entries(commands, cvars)
            .iter()
            .any(|entry| entry.name.to_lowercase() == first.to_lowercase());
        let chat = seat.connected() && matches!(self.dialect, Dialect::Q1Quakeworld | Dialect::Q3);
        if !explicit && !known && chat {
            seat.chat(&text, false, None);
        } else {
            let line = if explicit { trimmed[1..].to_string() } else { trimmed };
            commands.execute(&format!("{line}\n"), self.dialect, TextMode::Console, cvars, console)?;
        }
        self.history.add(&text);
        self.field.clear();
        self.buffer.bottom();
        Ok(())
    }

    /// Handle one input event; returns whether the console consumed it.
    #[allow(clippy::too_many_arguments)]
    pub fn input(
        &mut self,
        event: &ConsoleInputEvent,
        focus: ConsoleFocus,
        commands: &mut ConsoleCommands,
        cvars: &mut CvarRegistry,
        console: &mut dyn ConsoleCommandServices,
        seat: &mut dyn SeatConsoleServices,
    ) -> Result<bool, ConsoleError> {
        if let ConsoleInputEvent::Focus { focused } = event {
            if !focused {
                self.keys.clear();
                self.suppress_toggle_text = false;
            }
            return Ok(true);
        }
        if let ConsoleInputEvent::Text(text) = event {
            if self.suppress_toggle_text {
                self.suppress_toggle_text = false;
                if text == "`" || text == "~" {
                    return Ok(true);
                }
            }
        }
        if let ConsoleInputEvent::Key { code, down: true, .. } = event {
            if *code != 96 && *code != 126 {
                self.suppress_toggle_text = false;
            }
        }
        if !matches!(focus, ConsoleFocus::Console | ConsoleFocus::Chat { .. }) {
            return Ok(false);
        }
        let chat = matches!(focus, ConsoleFocus::Chat { .. });
        if let ConsoleInputEvent::Text(text) = event {
            if chat {
                self.chat_field.insert(text);
            } else {
                self.field.insert(text);
            }
            return Ok(true);
        }
        if let ConsoleInputEvent::MouseWheel { delta } = event {
            let step = if self.keys.contains(&(KeyCode::Control as i32)) {
                6.0
            } else {
                2.0
            };
            self.buffer.scroll((delta * step).trunc() as i64);
            return Ok(true);
        }
        let ConsoleInputEvent::Key { code, down, repeat } = event else {
            return Ok(false);
        };
        let (code, down, repeat) = (*code, *down, *repeat);
        if down {
            self.keys.insert(code);
        } else {
            self.keys.remove(&code);
            return Ok(true);
        }
        let control = self.keys.contains(&(KeyCode::Control as i32));
        let shift = self.keys.contains(&(KeyCode::Shift as i32));
        if code == 96 || code == 126 {
            self.toggle_from_key(repeat, seat);
            return Ok(true);
        }
        if code == KeyCode::Escape as i32 {
            self.close(seat);
            return Ok(true);
        }
        if code == KeyCode::Enter as i32 || code == KEYPAD_ENTER {
            if let ConsoleFocus::Chat { team } = focus {
                if !self.chat_field.text().is_empty() {
                    let text = self.chat_field.text();
                    seat.chat(&text, team, self.chat_target);
                }
                self.chat_field.clear();
                seat.set_focus(ConsoleFocus::Game);
            } else {
                self.submit(commands, cvars, console, seat)?;
            }
            return Ok(true);
        }
        if !chat {
            if code == KeyCode::Tab as i32 {
                let names: Vec<String> = query_console_entries(commands, cvars)
                    .into_iter()
                    .map(|entry| entry.name)
                    .collect();
                self.field.complete(&names, shift);
                return Ok(true);
            }
            if code == KeyCode::Up as i32 || code == KEYPAD_UP || (control && code == 112) {
                let text = self.field.text();
                let previous = self.history.previous(&text);
                self.field.set_text(&previous);
                return Ok(true);
            }
            if code == KeyCode::Down as i32 || code == KEYPAD_DOWN || (control && code == 110) {
                let next = self.history.next();
                self.field.set_text(&next);
                return Ok(true);
            }
            if code == KeyCode::PageUp as i32 || code == KeyCode::PageDown as i32 {
                self.buffer.scroll(if code == KeyCode::PageUp as i32 { 2 } else { -2 });
                return Ok(true);
            }
            if control && code == KeyCode::Home as i32 {
                self.buffer.top();
                return Ok(true);
            }
            if control && code == KeyCode::End as i32 {
                self.buffer.bottom();
                return Ok(true);
            }
        }
        let field = if chat { &mut self.chat_field } else { &mut self.field };
        Ok(field.key(code, control, shift, &mut || seat.clipboard()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::commands::register_console_commands;
    use qa_core::cmd::Dialect;

    struct SeatFixture {
        now: i64,
        connected: bool,
        focus: ConsoleFocus,
        chats: Vec<(String, bool, Option<i32>)>,
    }

    impl SeatConsoleServices for SeatFixture {
        fn now_ms(&mut self) -> i64 {
            self.now
        }

        fn connected(&self) -> bool {
            self.connected
        }

        fn clipboard(&mut self) -> Option<String> {
            None
        }

        fn set_focus(&mut self, focus: ConsoleFocus) {
            self.focus = focus;
        }

        fn chat(&mut self, text: &str, team: bool, target: Option<i32>) {
            self.chats.push((text.to_string(), team, target));
        }
    }

    struct ConsoleFixture {
        printed: Vec<String>,
    }

    impl ConsoleCommandServices for ConsoleFixture {
        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }

        fn forward_to_server(&mut self, _line: &str) {}
        fn toggle_console(&mut self) {}
        fn clear_console(&mut self) {}
        fn message_mode(&mut self, _team: bool) {}
        fn can_chat(&self) -> bool {
            true
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
    }

    fn harness(dialect: Dialect) -> (SeatConsole, ConsoleCommands, CvarRegistry, SeatFixture, ConsoleFixture) {
        let console = SeatConsole::new(dialect, true);
        let mut commands = ConsoleCommands::new();
        register_console_commands(&mut commands);
        let cvars = CvarRegistry::new(dialect);
        (
            console,
            commands,
            cvars,
            SeatFixture {
                now: 1000,
                connected: true,
                focus: ConsoleFocus::Game,
                chats: Vec::new(),
            },
            ConsoleFixture { printed: Vec::new() },
        )
    }

    #[test]
    fn submits_commands_and_falls_back_to_chat() {
        let (mut console, mut commands, mut cvars, mut seat, mut services) = harness(Dialect::Q3);
        console.field.set_text("clear");
        console
            .submit(&mut commands, &mut cvars, &mut services, &mut seat)
            .unwrap();
        assert_eq!(console.history.lines(), vec!["clear".to_string()]);
        console.field.set_text("hello team");
        console
            .submit(&mut commands, &mut cvars, &mut services, &mut seat)
            .unwrap();
        assert_eq!(seat.chats.len(), 1);
        console.field.set_text("/hello team");
        console
            .submit(&mut commands, &mut cvars, &mut services, &mut seat)
            .unwrap();
        assert_eq!(seat.chats.len(), 1);
    }

    #[test]
    fn handles_toggle_history_and_scroll_keys() {
        let (mut console, mut commands, mut cvars, mut seat, mut services) = harness(Dialect::Q2Classic);
        console.open(&mut seat);
        assert!(console.is_open());
        console
            .input(
                &ConsoleInputEvent::Key {
                    code: 96,
                    down: true,
                    repeat: false,
                },
                ConsoleFocus::Console,
                &mut commands,
                &mut cvars,
                &mut services,
                &mut seat,
            )
            .unwrap();
        assert!(!console.is_open());
        assert!(console
            .input(
                &ConsoleInputEvent::Text("`".to_string()),
                ConsoleFocus::Console,
                &mut commands,
                &mut cvars,
                &mut services,
                &mut seat,
            )
            .unwrap());
        assert_eq!(console.field.text(), "");
        console.field.set_text("clear");
        console
            .submit(&mut commands, &mut cvars, &mut services, &mut seat)
            .unwrap();
        console
            .input(
                &ConsoleInputEvent::Key {
                    code: KeyCode::Up as i32,
                    down: true,
                    repeat: false,
                },
                ConsoleFocus::Console,
                &mut commands,
                &mut cvars,
                &mut services,
                &mut seat,
            )
            .unwrap();
        assert_eq!(console.field.text(), "clear");
        assert!((console.animate(true, 500.0, 3.0, 0.5) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn adopts_staged_output() {
        let (mut console, _, _, _, _) = harness(Dialect::Q3);
        let mut staged = SeatConsole::new(Dialect::Q3, true);
        let mut seat = SeatFixture {
            now: 50,
            connected: false,
            focus: ConsoleFocus::Game,
            chats: Vec::new(),
        };
        staged.print("early\n", &mut seat).unwrap();
        console.adopt(&mut staged, ConsoleFocus::Console).unwrap();
        assert!(console.is_open());
        assert!(console.buffer.dump().contains("early"));
        assert!(console.adopt(&mut staged, ConsoleFocus::Console).is_err());
    }
}
