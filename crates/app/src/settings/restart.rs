//! Subsystem restart flow (`vid_restart`, `in_restart`, `snd_restart`).
//!
//! Donor provenance: `src/settings/restart.ts` (`RestartControls`).
//! Resource owners perform their own restart while session state stays
//! outside their lease; each drained restart first applies its latched
//! cvars. Async services become synchronous; the pending queue is shared
//! with the registered command handlers so `vid_restart` typed at the
//! console queues the same restart as an API call.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::cvar::CvarRegistry;

use super::SettingsError;
use crate::console::commands::ConsoleCommands;

/// Restartable subsystem (donor `RestartKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RestartKind {
    /// Video (`vid_restart`).
    Video,
    /// Input (`in_restart`).
    Input,
    /// Audio (`snd_restart`).
    Audio,
}

impl RestartKind {
    #[must_use]
    const fn command(self) -> &'static str {
        match self {
            Self::Video => "vid_restart",
            Self::Input => "in_restart",
            Self::Audio => "snd_restart",
        }
    }
}

/// Restartable resource owners by kind (donor `RestartService` record).
pub trait RestartServices {
    /// Restart one subsystem.
    fn restart(&mut self, kind: RestartKind) -> Result<(), SettingsError>;
}

/// Queued restart controls with per-kind latched cvars.
#[derive(Debug, Default)]
pub struct RestartControls {
    pending: Rc<RefCell<Vec<RestartKind>>>,
    running: bool,
    latched: HashMap<RestartKind, Vec<String>>,
}

impl RestartControls {
    /// Open controls with per-kind latched cvar names.
    #[must_use]
    pub fn new(latched: HashMap<RestartKind, Vec<String>>) -> Self {
        Self {
            pending: Rc::new(RefCell::new(Vec::new())),
            running: false,
            latched,
        }
    }

    /// Queue a restart (each kind runs once per drain).
    pub fn request(&mut self, kind: RestartKind) {
        let mut pending = self.pending.borrow_mut();
        if !pending.contains(&kind) {
            pending.push(kind);
        }
    }

    /// Whether a kind is queued.
    #[must_use]
    pub fn is_pending(&self, kind: RestartKind) -> bool {
        self.pending.borrow().contains(&kind)
    }

    /// Drain the queue: apply latched cvars, then restart each service.
    pub fn drain(&mut self, services: &mut dyn RestartServices, cvars: &mut CvarRegistry) -> Result<(), SettingsError> {
        if self.running {
            return Err(SettingsError::BadValue(
                "Restart controls already have an active operation".to_string(),
            ));
        }
        self.running = true;
        let mut result = Ok(());
        loop {
            let kind = {
                let mut pending = self.pending.borrow_mut();
                if pending.is_empty() {
                    None
                } else {
                    Some(pending.remove(0))
                }
            };
            let Some(kind) = kind else { break };
            let step = (|| -> Result<(), SettingsError> {
                if let Some(names) = self.latched.get(&kind).cloned() {
                    for name in &names {
                        cvars.apply_latched(Some(name))?;
                    }
                }
                services.restart(kind)
            })();
            if let Err(error) = step {
                result = Err(error);
                break;
            }
        }
        self.running = false;
        result
    }

    /// Register the restart commands; returns names for later removal.
    pub fn register(&mut self, commands: &mut ConsoleCommands) -> Vec<String> {
        let mut names = Vec::new();
        for kind in [RestartKind::Video, RestartKind::Input, RestartKind::Audio] {
            let pending = Rc::clone(&self.pending);
            if commands.register(
                kind.command(),
                move |_, _| {
                    if !pending.borrow().contains(&kind) {
                        pending.borrow_mut().push(kind);
                    }
                    Ok(())
                },
                None,
            ) {
                names.push(kind.command().to_string());
            }
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::{Dialect, TextMode};

    struct Services {
        restarts: Vec<RestartKind>,
    }

    impl RestartServices for Services {
        fn restart(&mut self, kind: RestartKind) -> Result<(), SettingsError> {
            self.restarts.push(kind);
            Ok(())
        }
    }

    struct Sink;

    impl crate::console::commands::ConsoleCommandServices for Sink {
        fn print(&mut self, _text: &str) {}
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
    }

    #[test]
    fn queues_once_and_applies_latched() {
        let mut latched = HashMap::new();
        latched.insert(RestartKind::Video, vec!["vid_mode".to_string()]);
        let mut controls = RestartControls::new(latched);
        controls.request(RestartKind::Video);
        controls.request(RestartKind::Video);
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        cvars.register("vid_mode", "3", qa_core::cvar::q2_flags::LATCH).unwrap();
        cvars.set_server_active(true);
        cvars.set("vid_mode", "5", false).unwrap();
        assert_eq!(cvars.get("vid_mode").unwrap().value, "3");
        let mut services = Services { restarts: Vec::new() };
        controls.drain(&mut services, &mut cvars).unwrap();
        assert_eq!(cvars.get("vid_mode").unwrap().value, "5");
        assert_eq!(services.restarts, vec![RestartKind::Video]);
        assert!(!controls.is_pending(RestartKind::Video));
    }

    #[test]
    fn console_commands_queue_restarts() {
        let mut controls = RestartControls::new(HashMap::new());
        let mut commands = ConsoleCommands::new();
        let names = controls.register(&mut commands);
        assert_eq!(
            names,
            vec![
                "vid_restart".to_string(),
                "in_restart".to_string(),
                "snd_restart".to_string()
            ]
        );
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        let mut sink = Sink;
        commands
            .execute(
                "vid_restart",
                Dialect::Q2Classic,
                TextMode::Source,
                &mut cvars,
                &mut sink,
            )
            .unwrap();
        assert!(controls.is_pending(RestartKind::Video));
        let mut services = Services { restarts: Vec::new() };
        controls.drain(&mut services, &mut cvars).unwrap();
        assert_eq!(services.restarts, vec![RestartKind::Video]);
    }
}
