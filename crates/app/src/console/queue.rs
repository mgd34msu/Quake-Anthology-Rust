//! Buffered console dispatch: the synchronous host driver above the
//! [`qa_core::cmd_buffer::CommandBuffer`] queue.
//!
//! Donor provenance: `src/core/commands/index.ts` (`CommandBuffer`) with the
//! per-frame host drains of `src/app/bootstrap/application.ts`. The queue
//! owns the program (text, aliases, waits, scripts, the Quake II overflow
//! queue, the program revision) while registered console commands stay in
//! [`ConsoleCommands`]. Each host frame calls [`ConsoleQueue::drive_frame`],
//! which drains one frame and flushes prints into console services. Script
//! reads resolve synchronously from preloaded text, like `COM_LoadHunkFile`
//! followed by `Cbuf_InsertText`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{
    BufferError, BufferOptions, BufferServices, CommandBuffer, CommandContext, ForwardedCommand, FrameHooks,
    ScriptCompletion, ScriptRead,
};
use qa_core::cvar::CvarRegistry;

use super::commands::{CommandInvocation, ConsoleCommandServices, ConsoleCommands};
use super::ConsoleError;

impl From<BufferError> for ConsoleError {
    fn from(error: BufferError) -> Self {
        Self::BadCommand(error.to_string())
    }
}

/// Script text held by the queue.
#[derive(Debug, Clone)]
enum QueuedScript {
    /// The read failed; `exec` reports it and continues.
    Failed(String),
    /// Settled text (`None` means missing).
    Ready(Option<String>),
}

struct Bridge<'a> {
    commands: &'a mut ConsoleCommands,
    console: &'a mut dyn ConsoleCommandServices,
    scripts: &'a HashMap<String, QueuedScript>,
    prints: Rc<RefCell<Vec<String>>>,
}

impl BufferServices for Bridge<'_> {
    fn read_script(&mut self, name: &str, _source: &CommandContext) -> ScriptRead {
        match self.scripts.get(name) {
            None | Some(QueuedScript::Ready(None)) => ScriptRead::Ready(None),
            Some(QueuedScript::Ready(Some(text))) => ScriptRead::Ready(Some(text.clone())),
            Some(QueuedScript::Failed(error)) => ScriptRead::Failed(error.clone()),
        }
    }

    fn forward_to_server(&mut self, command: &ForwardedCommand) {
        self.console.forward_to_server(&command.raw);
    }

    fn external_command(&mut self, command: &ForwardedCommand, _registered: bool) -> bool {
        let invocation = CommandInvocation {
            argv: command.argv.clone(),
            args_text: command.args_text.clone(),
            dialect: command.dialect,
            raw: command.raw.clone(),
            direct: command.direct,
        };
        match self.commands.invoke_registered(&invocation, &mut *self.console) {
            None => false,
            Some(Ok(())) => true,
            Some(Err(error)) => {
                self.prints.borrow_mut().push(format!("{error}\n"));
                true
            }
        }
    }
}

/// Buffered console program with its synchronous host driver.
pub struct ConsoleQueue {
    buffer: CommandBuffer,
    scripts: HashMap<String, QueuedScript>,
    prints: Rc<RefCell<Vec<String>>>,
}

impl ConsoleQueue {
    /// Open a queue for a dialect and owning context.
    pub fn new(dialect: Dialect, context: CommandContext) -> Result<Self, ConsoleError> {
        Self::with_options(dialect, context, BufferOptions::new())
    }

    /// Open a queue with buffer options.
    pub fn with_options(
        dialect: Dialect,
        context: CommandContext,
        options: BufferOptions,
    ) -> Result<Self, ConsoleError> {
        let mut buffer = CommandBuffer::new(dialect, context, options)?;
        let prints = Rc::new(RefCell::new(Vec::new()));
        let sink = prints.clone();
        buffer.set_printer(move |text, _| sink.borrow_mut().push(text.to_string()));
        Ok(Self {
            buffer,
            scripts: HashMap::new(),
            prints,
        })
    }

    /// Borrow the program buffer.
    #[must_use]
    pub fn buffer(&self) -> &CommandBuffer {
        &self.buffer
    }

    /// Borrow the program buffer mutably.
    pub fn buffer_mut(&mut self) -> &mut CommandBuffer {
        &mut self.buffer
    }

    /// Queue console text behind the program.
    pub fn submit(&mut self, text: &str) -> Result<(), ConsoleError> {
        self.buffer.append(text, None, None)?;
        Ok(())
    }

    /// Queue console text under another context of the same session.
    pub fn submit_as(&mut self, text: &str, source: &CommandContext) -> Result<(), ConsoleError> {
        self.buffer.append(text, Some(source), None)?;
        Ok(())
    }

    /// Preload script text (`None` means missing) for later `exec` reads.
    pub fn set_script(&mut self, name: &str, text: Option<String>) {
        self.scripts.insert(name.to_string(), QueuedScript::Ready(text));
    }

    /// Preload a script read failure for later `exec` reads.
    pub fn fail_script(&mut self, name: &str, error: &str) {
        self.scripts
            .insert(name.to_string(), QueuedScript::Failed(error.to_string()));
    }

    /// Bind a script-completion listener; returns its binding id.
    pub fn bind_script_completion(&mut self, listener: impl FnMut(&ScriptCompletion) + 'static) -> u64 {
        self.buffer.bind_script_completion(listener)
    }

    /// Release a script-completion binding.
    pub fn unbind_script_completion(&mut self, id: u64) -> bool {
        self.buffer.unbind_script_completion(id)
    }

    /// Drain one host frame: lines run until the queue empties, a `wait`
    /// boundary lands, or a script read parks. `after_dispatch` runs after
    /// every dispatched line; prints flush into `console` when the frame
    /// drain returns.
    pub fn drive_frame(
        &mut self,
        commands: &mut ConsoleCommands,
        cvars: &mut CvarRegistry,
        console: &mut dyn ConsoleCommandServices,
        after_dispatch: &mut dyn FnMut(),
    ) -> Result<usize, ConsoleError> {
        struct Hooks<'a> {
            inner: &'a mut dyn FnMut(),
        }
        impl FrameHooks for Hooks<'_> {
            fn after_dispatch(&mut self) {
                (self.inner)();
            }
        }
        let mut bridge = Bridge {
            commands,
            console,
            scripts: &self.scripts,
            prints: self.prints.clone(),
        };
        let mut hooks = Hooks { inner: after_dispatch };
        let executed = self.buffer.execute_hooked(cvars, &mut bridge, &mut hooks)?;
        for line in bridge.prints.borrow_mut().drain(..) {
            bridge.console.print(&line);
        }
        Ok(executed)
    }

    /// Drive frames until the program idles or `max_frames` elapses;
    /// returns the total dispatched lines.
    pub fn drive_until_idle(
        &mut self,
        commands: &mut ConsoleCommands,
        cvars: &mut CvarRegistry,
        console: &mut dyn ConsoleCommandServices,
        max_frames: u32,
    ) -> Result<usize, ConsoleError> {
        let mut executed = 0;
        for _ in 0..max_frames {
            if !self.buffer.has_pending_commands() {
                break;
            }
            executed += self.drive_frame(commands, cvars, console, &mut || {})?;
        }
        Ok(executed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd_buffer::CommandOrigin;
    use qa_core::identity::IdentityOwner;

    struct Fixture {
        printed: Vec<String>,
        forwarded: Vec<String>,
        toggles: usize,
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

    fn harness(dialect: Dialect) -> (ConsoleQueue, ConsoleCommands, CvarRegistry, Fixture, CommandContext) {
        let owner = IdentityOwner::create("queue").unwrap();
        let context = CommandContext::new(
            owner.session().clone(),
            CommandOrigin::LocalSeat {
                seat: owner.seat(0),
                client: owner.client(0, 0),
            },
        );
        // The owner only mints handles; the queue keeps the cloned session.
        let _ = owner;
        let queue = ConsoleQueue::new(dialect, context.clone()).unwrap();
        let mut commands = ConsoleCommands::new();
        super::super::commands::register_console_commands(&mut commands);
        let cvars = CvarRegistry::new(dialect);
        (
            queue,
            commands,
            cvars,
            Fixture {
                printed: Vec::new(),
                forwarded: Vec::new(),
                toggles: 0,
            },
            context,
        )
    }

    #[test]
    fn waits_and_scripts_span_frames_with_console_commands() {
        let (mut queue, mut commands, mut cvars, mut services, _) = harness(Dialect::Q3);
        queue.set_script("startup.cfg", Some("toggleconsole\n".to_string()));
        let completed = Rc::new(RefCell::new(Vec::new()));
        let completed_handler = completed.clone();
        queue.bind_script_completion(move |event| {
            completed_handler
                .borrow_mut()
                .push(format!("{}:{:?}", event.name, event.result));
        });
        queue.submit("exec startup; wait 2; echo done\n").unwrap();
        let mut dispatches = 0;
        queue
            .drive_frame(&mut commands, &mut cvars, &mut services, &mut || dispatches += 1)
            .unwrap();
        assert_eq!(services.toggles, 1);
        assert_eq!(dispatches, 3);
        assert_eq!(*completed.borrow(), vec!["startup.cfg:Completed".to_string()]);
        queue
            .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
            .unwrap();
        assert_eq!(services.printed, vec!["execing startup.cfg\n".to_string()]);
        queue
            .drive_frame(&mut commands, &mut cvars, &mut services, &mut || {})
            .unwrap();
        assert_eq!(
            services.printed,
            vec!["execing startup.cfg\n".to_string(), "done \n".to_string()]
        );
    }

    #[test]
    fn preloaded_missing_and_failed_scripts_report() {
        let (mut queue, mut commands, mut cvars, mut services, _) = harness(Dialect::Q2Classic);
        queue.set_script("late.cfg", Some("echo late\n".to_string()));
        queue.fail_script("bad.cfg", "read denied");
        queue.submit("exec late.cfg; echo after\n").unwrap();
        queue
            .drive_until_idle(&mut commands, &mut cvars, &mut services, 8)
            .unwrap();
        assert_eq!(
            services.printed,
            vec![
                "execing late.cfg\n".to_string(),
                "late \n".to_string(),
                "after \n".to_string()
            ]
        );
        queue.submit("exec missing.cfg; exec bad.cfg\n").unwrap();
        queue
            .drive_until_idle(&mut commands, &mut cvars, &mut services, 8)
            .unwrap();
        assert!(services
            .printed
            .iter()
            .any(|line| line == "couldn't exec missing.cfg\n"));
        assert!(services
            .printed
            .iter()
            .any(|line| line == "couldn't exec bad.cfg: read denied\n"));
    }
}
