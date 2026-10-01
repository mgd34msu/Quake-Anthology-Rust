//! Client demo commands and attract-list cycling.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/demo-commands.ts`
//! (`ClientDemoCommands`, `localWorldDemoCommands`).
//! A client retains its attract list across world/source replacement. The
//! host arrives through [`ClientDemoCommandHost`]; request identity (the
//! donor's `WeakSet`/`WeakMap`) becomes per-value occurrence counts, which
//! keeps identical back-to-back requests distinct. Buffer handlers are
//! infallible, so service failures print through the invocation; release
//! takes the buffer the donor closure captures. The Rust buffer keys
//! registrations by name, so release unregisters by name.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::cmd::ascii_fold;
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{BufferError, CommandBuffer, CommandContext, CommandDocumentation, CommandOrigin};
use qa_core::cvar::{CvarError, CvarRegistry};
use thiserror::Error;

use super::demo_playback::{demo_family, DemoFamily, DemoRequest};

/// Demo command failure.
#[derive(Debug, Error)]
pub enum DemoCommandError {
    /// Demo staging failure.
    #[error("{0}")]
    Stage(String),
    /// Completion command failure.
    #[error("{0}")]
    Completion(String),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Command buffer failure.
    #[error(transparent)]
    Buffer(#[from] BufferError),
}

/// Client demo state (`DemoClientState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemoClientState {
    /// Idle client.
    Idle {
        /// Current family.
        family: DemoFamily,
        /// Explicit startup pending.
        explicit_startup: bool,
    },
    /// Local game.
    Local {
        /// Current family.
        family: DemoFamily,
    },
    /// Network game.
    Network {
        /// Current family.
        family: DemoFamily,
    },
    /// Demo playback.
    Demo {
        /// Current family.
        family: DemoFamily,
        /// Active request.
        request: DemoRequest,
    },
}

impl DemoClientState {
    /// Current family.
    #[must_use]
    pub fn family(&self) -> DemoFamily {
        match self {
            Self::Idle { family, .. }
            | Self::Local { family }
            | Self::Network { family }
            | Self::Demo { family, .. } => *family,
        }
    }
}

/// Client demo intent (`ClientDemoIntent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientDemoIntent {
    /// Start playback.
    Start {
        /// Request.
        request: DemoRequest,
        /// Command source.
        source: CommandContext,
    },
    /// Stop playback.
    Stop {
        /// Command source.
        source: CommandContext,
    },
}

/// Demo completion reason (`DemoCompletion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoCompletion {
    /// End of file.
    Eof,
    /// Terminator.
    Terminator,
    /// Disconnected.
    Disconnected,
    /// Truncated.
    Truncated,
    /// Closed.
    Closed,
}

/// Completion-command family (`takeCompletionCommand` argument).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoCompletionFamily {
    /// Quake II (`nextserver`).
    Q2,
    /// Quake III (`nextdemo`).
    Q3,
}

/// Client demo command host (`ClientDemoCommandHost`).
pub trait ClientDemoCommandHost {
    /// Dedicated server without a local client.
    fn dedicated(&self) -> bool;
    /// Current client state.
    fn current(&self, source: Option<&CommandContext>) -> DemoClientState;
    /// Stage a demo intent.
    fn stage(&mut self, intent: ClientDemoIntent) -> Result<(), DemoCommandError>;
    /// Print console text.
    fn print(&mut self, text: &str);
    /// Append buffer text.
    fn append(&mut self, text: &str, source: &CommandContext);
    /// Read and clear the completion command.
    fn take_completion_command(&mut self, family: DemoCompletionFamily) -> Result<String, DemoCommandError>;
}

fn local(source: &CommandContext) -> bool {
    let mut origin = &source.origin;
    while let CommandOrigin::Script { caller, .. } = origin {
        origin = caller;
    }
    !matches!(origin, CommandOrigin::RemoteClient { .. })
}

struct AttachedBinding {
    names: Vec<String>,
}

/// Attached demo commands release.
pub type DemoCommandRelease = Box<dyn FnOnce(&mut CommandBuffer)>;

/// Client demo commands (`ClientDemoCommands`).
pub struct ClientDemoCommands<H> {
    host: H,
    names: Vec<String>,
    next: i32,
    cycle_source: Option<CommandContext>,
    latest_request: Option<DemoRequest>,
    started: Vec<DemoRequest>,
    finished: Vec<DemoRequest>,
    sources: Vec<(DemoRequest, CommandContext)>,
    bindings: Vec<AttachedBinding>,
}

impl<H: ClientDemoCommandHost> ClientDemoCommands<H> {
    /// Build commands over a host.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self {
            host,
            names: Vec::new(),
            next: 0,
            cycle_source: None,
            latest_request: None,
            started: Vec::new(),
            finished: Vec::new(),
            sources: Vec::new(),
            bindings: Vec::new(),
        }
    }

    /// Share commands for buffer attachment.
    #[must_use]
    pub fn shared(host: H) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self::new(host)))
    }

    /// Attach demo commands; the release closure takes the buffer the
    /// donor closure captures (`attach`).
    pub fn attach(
        shared: &Rc<RefCell<Self>>,
        commands: &mut CommandBuffer,
        cvars: &CvarRegistry,
    ) -> Result<DemoCommandRelease, DemoCommandError>
    where
        H: 'static,
    {
        let mut owned: Vec<String> = Vec::new();
        let mut add =
            |commands: &mut CommandBuffer, name: &str, summary: &str, usage: &str| -> Result<(), DemoCommandError> {
                let bound = name.to_string();
                let target = Rc::clone(shared);
                let examples = vec![usage
                    .replace("<name>", "demo1")
                    .replace("[name ...]", "demo1 demo2 demo3")];
                let registered = commands.register(
                    name,
                    Some(Rc::new(move |invocation| {
                        let args = invocation.args().to_vec();
                        let source = invocation.source.clone();
                        if let Err(error) = target.borrow_mut().handle(&bound, &args, &source) {
                            invocation.print(&format!("{error}\n"));
                        }
                    })),
                    Some(CommandDocumentation {
                        summary: summary.to_string(),
                        usage: usage.to_string(),
                        examples,
                        allowed_values: None,
                    }),
                    cvars,
                )?;
                if registered {
                    owned.push(name.to_string());
                } else {
                    shared
                        .borrow_mut()
                        .host
                        .print(&format!("Demo command {name} is already owned by another command.\n"));
                }
                Ok(())
            };
        add(
            commands,
            "playdemo",
            "Play a recording; its file extension selects the game.",
            "playdemo <name>",
        )?;
        add(commands, "demo", "Play a Quake III recording.", "demo <name>")?;
        add(commands, "demomap", "Play a Quake II recording.", "demomap <name>")?;
        add(
            commands,
            "startdemos",
            "Set the Quake attract-mode recording list.",
            "startdemos [name ...]",
        )?;
        add(commands, "demos", "Resume the saved Quake attract list.", "demos")?;
        add(
            commands,
            "stopdemo",
            "Stop the active recording and return to the menu.",
            "stopdemo",
        )?;
        let binding = shared.borrow().bindings.len();
        shared.borrow_mut().bindings.push(AttachedBinding { names: owned });
        Self::refresh_binding(shared, binding, commands, cvars)?;
        let target = Rc::clone(shared);
        Ok(Box::new(move |commands| {
            let mut shared = target.borrow_mut();
            if let Some(binding) = shared.bindings.get_mut(binding) {
                for name in std::mem::take(&mut binding.names) {
                    commands.unregister(&name);
                }
            }
        }))
    }

    fn refresh_binding(
        shared: &Rc<RefCell<Self>>,
        binding: usize,
        commands: &mut CommandBuffer,
        cvars: &CvarRegistry,
    ) -> Result<(), DemoCommandError>
    where
        H: 'static,
    {
        let family = shared.borrow().host.current(None).family();
        let has_timedemo = shared
            .borrow()
            .bindings
            .get(binding)
            .is_some_and(|binding| binding.names.iter().any(|name| name == "timedemo"));
        if family == DemoFamily::Q1 || family == DemoFamily::Qw {
            if !has_timedemo {
                let target = Rc::clone(shared);
                let registered = commands.register(
                    "timedemo",
                    Some(Rc::new(move |invocation| {
                        let args = invocation.args().to_vec();
                        let source = invocation.source.clone();
                        if let Err(error) = target.borrow_mut().handle("timedemo", &args, &source) {
                            invocation.print(&format!("{error}\n"));
                        }
                    })),
                    Some(CommandDocumentation {
                        summary: "Benchmark a Quake recording.".to_string(),
                        usage: "timedemo <name>".to_string(),
                        examples: vec!["timedemo demo1".to_string()],
                        allowed_values: None,
                    }),
                    cvars,
                )?;
                if registered {
                    if let Some(binding) = shared.borrow_mut().bindings.get_mut(binding) {
                        binding.names.push("timedemo".to_string());
                    }
                } else {
                    shared
                        .borrow_mut()
                        .host
                        .print("Demo command timedemo is already owned by another command.\n");
                }
            }
        } else if let Some(binding) = shared.borrow_mut().bindings.get_mut(binding) {
            if let Some(index) = binding.names.iter().position(|name| name == "timedemo") {
                binding.names.remove(index);
                commands.unregister("timedemo");
            }
        }
        Ok(())
    }

    /// Refresh every binding at the world/profile boundary (`refresh`).
    pub fn refresh(
        shared: &Rc<RefCell<Self>>,
        commands: &mut CommandBuffer,
        cvars: &CvarRegistry,
    ) -> Result<(), DemoCommandError>
    where
        H: 'static,
    {
        let count = shared.borrow().bindings.len();
        for binding in 0..count {
            Self::refresh_binding(shared, binding, commands, cvars)?;
        }
        Ok(())
    }

    /// Dispatch one demo command (`handle`).
    pub fn handle(
        &mut self,
        name_input: &str,
        args: &[String],
        source: &CommandContext,
    ) -> Result<bool, DemoCommandError> {
        let name = ascii_fold(name_input);
        match name.as_str() {
            "playdemo" | "demo" | "demomap" | "startdemos" | "demos" | "stopdemo" => {}
            "timedemo" => {
                let family = self.host.current(None).family();
                if family != DemoFamily::Q1 && family != DemoFamily::Qw {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
        if !local(source) {
            self.host.print(&format!("{name} is a local client command.\n"));
            return Ok(true);
        }
        match name.as_str() {
            "playdemo" => self.play(&name, args, source, None, false)?,
            "demo" => self.play(&name, args, source, Some(DemoFamily::Q3), false)?,
            "demomap" => self.play(&name, args, source, Some(DemoFamily::Q2), false)?,
            "timedemo" => self.play(&name, args, source, None, true)?,
            "startdemos" => self.start_demos(args, source)?,
            "demos" => {
                if !self.host.dedicated() {
                    if self.next < 0 {
                        self.next = 1;
                    }
                    self.cycle_source = Some(source.clone());
                    self.next_demo()?;
                }
            }
            "stopdemo" if !self.host.dedicated() && matches!(self.host.current(None), DemoClientState::Demo { .. }) => {
                self.next = -1;
                self.latest_request = None;
                self.host.stage(ClientDemoIntent::Stop { source: source.clone() })?;
            }
            _ => {}
        }
        Ok(true)
    }

    fn play(
        &mut self,
        name: &str,
        args: &[String],
        source: &CommandContext,
        selected: Option<DemoFamily>,
        timedemo: bool,
    ) -> Result<(), DemoCommandError> {
        if self.host.dedicated() && selected != Some(DemoFamily::Q2) {
            return Ok(());
        }
        let name_arg = args.len() == 1;
        let Some(demo) = args.first().filter(|_| name_arg) else {
            self.host.print(&format!("Usage: {name} <name>\n"));
            return Ok(());
        };
        self.next = -1;
        let family = selected.unwrap_or_else(|| demo_family(demo, self.host.current(None).family()));
        self.start(
            DemoRequest {
                family,
                name: demo.clone(),
                timedemo,
            },
            source,
        )
    }

    fn start(&mut self, request: DemoRequest, source: &CommandContext) -> Result<(), DemoCommandError> {
        self.latest_request = Some(request.clone());
        self.sources.push((request.clone(), source.clone()));
        self.started.push(request.clone());
        self.host.stage(ClientDemoIntent::Start {
            request,
            source: source.clone(),
        })
    }

    fn start_demos(&mut self, args: &[String], source: &CommandContext) -> Result<(), DemoCommandError> {
        let current = self.host.current(Some(source));
        if self.host.dedicated() {
            if matches!(
                current,
                DemoClientState::Idle {
                    explicit_startup: false,
                    ..
                }
            ) {
                self.host.append("map start\n", source);
            }
            return Ok(());
        }
        if args.len() > 8 {
            self.host.print("Max 8 demos in demoloop\n");
        }
        self.names = args
            .iter()
            .take(8)
            .map(|name| name.chars().take(15).collect::<String>())
            .collect();
        self.host.print(&format!("{} demo(s) in loop\n", self.names.len()));
        self.cycle_source = Some(source.clone());
        if matches!(
            current,
            DemoClientState::Idle {
                explicit_startup: false,
                ..
            }
        ) && self.next != -1
        {
            self.next = 0;
            self.next_demo()?;
        } else {
            self.next = -1;
        }
        Ok(())
    }

    fn next_demo(&mut self) -> Result<(), DemoCommandError> {
        let Some(source) = self.cycle_source.clone() else {
            return Ok(());
        };
        if self.next < 0 {
            return Ok(());
        }
        if self.next as usize >= self.names.len() || self.names.get(self.next as usize).is_some_and(String::is_empty) {
            self.next = 0;
        }
        let name = self.names.get(self.next as usize).cloned().unwrap_or_default();
        if name.is_empty() {
            self.next = -1;
            self.host.print("No demos listed with startdemos\n");
            return Ok(());
        }
        self.next += 1;
        self.start(
            DemoRequest {
                family: demo_family(&name, DemoFamily::Q1),
                name,
                timedemo: false,
            },
            &source,
        )
    }

    /// Completion applies once, including callback reentry (`complete`).
    pub fn complete(&mut self, request: &DemoRequest, reason: DemoCompletion) -> Result<(), DemoCommandError> {
        let current = self.host.current(None);
        let DemoClientState::Demo { request: active, .. } = &current else {
            return Ok(());
        };
        if active != request || self.latest_request.as_ref() != Some(request) {
            return Ok(());
        }
        let started = self.started.iter().filter(|entry| *entry == request).count();
        let finished = self.finished.iter().filter(|entry| *entry == request).count();
        if finished >= started {
            return Ok(());
        }
        self.finished.push(request.clone());
        if reason == DemoCompletion::Closed || reason == DemoCompletion::Truncated {
            self.next = -1;
            return Ok(());
        }
        if request.family == DemoFamily::Q1 || request.family == DemoFamily::Qw {
            self.next_demo()?;
            return Ok(());
        }
        if reason == DemoCompletion::Disconnected {
            return Ok(());
        }
        let family = match request.family {
            DemoFamily::Q2 => DemoCompletionFamily::Q2,
            _ => DemoCompletionFamily::Q3,
        };
        let text = self.host.take_completion_command(family)?;
        let position = self.sources.iter().position(|(entry, _)| entry == request);
        if !text.is_empty() {
            if let Some(index) = position {
                let (_, source) = self.sources.remove(index);
                self.host.append(&format!("{text}\n"), &source);
            }
        }
        Ok(())
    }

    /// Mark a request failed (`failed`).
    pub fn failed(&mut self, request: &DemoRequest) {
        self.finished.push(request.clone());
        if self.latest_request.as_ref() == Some(request) {
            self.next = -1;
        }
    }

    /// A manual game cancels attract cycling (`manualGame`).
    pub fn manual_game(&mut self) {
        self.next = -1;
        self.latest_request = None;
    }
}

/// Local-world demo source (`localWorldDemoCommands` host source).
pub trait LocalWorldDemoSource {
    /// Console dialect.
    fn dialect(&self) -> Dialect;
    /// Read a cvar string.
    fn variable_string(&self, name: &str) -> String;
    /// Write a cvar.
    fn set(&mut self, name: &str, value: &str, force: bool) -> Result<(), CvarError>;
}

impl LocalWorldDemoSource for CvarRegistry {
    fn dialect(&self) -> Dialect {
        CvarRegistry::dialect(self)
    }

    fn variable_string(&self, name: &str) -> String {
        CvarRegistry::variable_string(self, name)
    }

    fn set(&mut self, name: &str, value: &str, force: bool) -> Result<(), CvarError> {
        CvarRegistry::set(self, name, value, force).map(|_| ())
    }
}

/// Local-world demo host: direct owners retain a local server while the
/// frontend owns recording playback.
pub struct LocalWorldDemoHost<S, F, P> {
    /// Dedicated server without a local client.
    pub dedicated: bool,
    /// Cvar source.
    pub source: S,
    /// Buffer append.
    pub append: F,
    /// Console print.
    pub print: P,
}

impl<S: LocalWorldDemoSource, F: FnMut(&str, &CommandContext), P: FnMut(&str)> ClientDemoCommandHost
    for LocalWorldDemoHost<S, F, P>
{
    fn dedicated(&self) -> bool {
        self.dedicated
    }

    fn current(&self, _source: Option<&CommandContext>) -> DemoClientState {
        let family = match self.source.dialect() {
            Dialect::Q1Netquake => DemoFamily::Q1,
            Dialect::Q1Quakeworld => DemoFamily::Qw,
            Dialect::Q3 => DemoFamily::Q3,
            Dialect::Q2Classic | Dialect::Q2Rerelease => DemoFamily::Q2,
        };
        DemoClientState::Local { family }
    }

    fn stage(&mut self, intent: ClientDemoIntent) -> Result<(), DemoCommandError> {
        Err(DemoCommandError::Stage(match intent {
            ClientDemoIntent::Start { request, .. } => format!(
                "Demo playback ({}) requires the graphical launcher; this application owns a local game.",
                request.name
            ),
            ClientDemoIntent::Stop { .. } => "Stopping demo playback requires its retained client owner.".to_string(),
        }))
    }

    fn print(&mut self, text: &str) {
        (self.print)(text);
    }

    fn append(&mut self, text: &str, source: &CommandContext) {
        (self.append)(text, source);
    }

    fn take_completion_command(&mut self, family: DemoCompletionFamily) -> Result<String, DemoCommandError> {
        let name = match family {
            DemoCompletionFamily::Q2 => "nextserver",
            DemoCompletionFamily::Q3 => "nextdemo",
        };
        let text = self.source.variable_string(name);
        self.source.set(name, "", true)?;
        Ok(text)
    }
}

/// Direct-owner demo commands over a local cvar source
/// (`localWorldDemoCommands`).
pub fn local_world_demo_commands<S, F, P>(
    dedicated: bool,
    source: S,
    append: F,
    print: P,
) -> ClientDemoCommands<LocalWorldDemoHost<S, F, P>>
where
    S: LocalWorldDemoSource,
    F: FnMut(&str, &CommandContext),
    P: FnMut(&str),
{
    ClientDemoCommands::new(LocalWorldDemoHost {
        dedicated,
        source,
        append,
        print,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct StubHost {
        dedicated: bool,
        state: DemoClientState,
        staged: Vec<ClientDemoIntent>,
        printed: Vec<String>,
        appended: Vec<String>,
        completion: String,
    }

    impl ClientDemoCommandHost for StubHost {
        fn dedicated(&self) -> bool {
            self.dedicated
        }
        fn current(&self, _source: Option<&CommandContext>) -> DemoClientState {
            self.state.clone()
        }
        fn stage(&mut self, intent: ClientDemoIntent) -> Result<(), DemoCommandError> {
            self.staged.push(intent);
            Ok(())
        }
        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }
        fn append(&mut self, text: &str, _source: &CommandContext) {
            self.appended.push(text.to_string());
        }
        fn take_completion_command(&mut self, _family: DemoCompletionFamily) -> Result<String, DemoCommandError> {
            Ok(std::mem::take(&mut self.completion))
        }
    }

    fn context() -> (CommandContext, IdentityOwner) {
        let owner = IdentityOwner::create("demo-commands").unwrap();
        let seat = owner.seat(0);
        let client = owner.client(0, 0);
        let context = CommandContext::new(owner.session().clone(), CommandOrigin::LocalSeat { seat, client });
        (context, owner)
    }

    fn host() -> StubHost {
        StubHost {
            dedicated: false,
            state: DemoClientState::Idle {
                family: DemoFamily::Q1,
                explicit_startup: false,
            },
            staged: Vec::new(),
            printed: Vec::new(),
            appended: Vec::new(),
            completion: String::new(),
        }
    }

    #[test]
    fn play_selects_family_and_stages_start() {
        let (source, _owner) = context();
        let mut commands = ClientDemoCommands::new(host());
        assert!(commands
            .handle("playdemo", &["demo1.dm2".to_string()], &source)
            .unwrap());
        assert!(commands.handle("demo", &["q3demo".to_string()], &source).unwrap());
        assert!(commands.handle("demomap", &["q2demo".to_string()], &source).unwrap());
        assert!(!commands.handle("unknown", &[], &source).unwrap());
        assert!(commands.handle("timedemo", &["demo1".to_string()], &source).unwrap());
        assert_eq!(commands.host.staged.len(), 4);
    }

    #[test]
    fn attract_list_cycles_and_stops() {
        let (source, _owner) = context();
        let mut commands = ClientDemoCommands::new(host());
        commands
            .handle("startdemos", &["a".to_string(), "b".to_string()], &source)
            .unwrap();
        assert_eq!(commands.host.printed, vec!["2 demo(s) in loop\n".to_string()]);
        assert_eq!(commands.host.staged.len(), 1);
        commands.handle("demos", &[], &source).unwrap();
        assert_eq!(commands.host.staged.len(), 2);
        commands.host.state = DemoClientState::Demo {
            family: DemoFamily::Q1,
            request: DemoRequest {
                family: DemoFamily::Q1,
                name: "b".to_string(),
                timedemo: false,
            },
        };
        commands.handle("stopdemo", &[], &source).unwrap();
        assert!(matches!(
            commands.host.staged.last(),
            Some(ClientDemoIntent::Stop { .. })
        ));
    }

    #[test]
    fn completion_advances_q1_and_runs_q3_commands() {
        let (source, _owner) = context();
        let mut commands = ClientDemoCommands::new(host());
        commands
            .handle("startdemos", &["a".to_string(), "b".to_string()], &source)
            .unwrap();
        let first = DemoRequest {
            family: DemoFamily::Q1,
            name: "a".to_string(),
            timedemo: false,
        };
        commands.host.state = DemoClientState::Demo {
            family: DemoFamily::Q1,
            request: first.clone(),
        };
        commands.complete(&first, DemoCompletion::Eof).unwrap();
        assert_eq!(commands.host.staged.len(), 2);
        // Completing the superseded value is ignored.
        commands.complete(&first, DemoCompletion::Eof).unwrap();
        assert_eq!(commands.host.staged.len(), 2);
        let second = DemoRequest {
            family: DemoFamily::Q1,
            name: "b".to_string(),
            timedemo: false,
        };
        commands.host.state = DemoClientState::Demo {
            family: DemoFamily::Q1,
            request: second.clone(),
        };
        commands.complete(&second, DemoCompletion::Eof).unwrap();
        assert_eq!(commands.host.staged.len(), 3);
        // Q3 completions run the retained completion command once.
        let mut commands = ClientDemoCommands::new(host());
        commands.host.completion = "nextmap".to_string();
        let request = DemoRequest {
            family: DemoFamily::Q3,
            name: "q3demo".to_string(),
            timedemo: false,
        };
        commands.handle("demo", &["q3demo".to_string()], &source).unwrap();
        commands.host.state = DemoClientState::Demo {
            family: DemoFamily::Q3,
            request: request.clone(),
        };
        commands.complete(&request, DemoCompletion::Eof).unwrap();
        assert_eq!(commands.host.appended, vec!["nextmap\n".to_string()]);
        commands.complete(&request, DemoCompletion::Eof).unwrap();
        assert_eq!(commands.host.appended.len(), 1);
    }

    #[test]
    fn failed_and_manual_games_cancel_cycling() {
        let (source, _owner) = context();
        let mut commands = ClientDemoCommands::new(host());
        commands.handle("startdemos", &["a".to_string()], &source).unwrap();
        let request = DemoRequest {
            family: DemoFamily::Q1,
            name: "a".to_string(),
            timedemo: false,
        };
        commands.failed(&request);
        commands.host.state = DemoClientState::Demo {
            family: DemoFamily::Q1,
            request: request.clone(),
        };
        commands.complete(&request, DemoCompletion::Eof).unwrap();
        assert_eq!(commands.host.staged.len(), 1);
        commands.manual_game();
        // `demos` explicitly resumes the retained attract list.
        commands.handle("demos", &[], &source).unwrap();
        assert_eq!(commands.host.staged.len(), 2);
    }

    #[test]
    fn local_world_host_maps_dialect_and_blocks_staging() {
        let (source, _owner) = context();
        let cvars = CvarRegistry::new(Dialect::Q1Netquake);
        let mut appended = Vec::new();
        let mut printed = Vec::new();
        let mut commands = local_world_demo_commands(
            false,
            cvars,
            |text: &str, _: &CommandContext| appended.push(text.to_string()),
            |text: &str| printed.push(text.to_string()),
        );
        assert!(matches!(
            commands.host.current(None),
            DemoClientState::Local { family: DemoFamily::Q1 }
        ));
        let error = commands
            .handle("playdemo", &["demo1".to_string()], &source)
            .unwrap_err();
        assert!(matches!(error, DemoCommandError::Stage(_)));
    }
}
