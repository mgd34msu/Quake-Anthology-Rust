//! Initial configuration script ordering and scoped script reads.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/startup-config.ts`
//! (`StartupScriptScope`, `StartupConfigOptions`, `StartupConfig`,
//! `StartupScriptReaderOptions`, `createStartupScriptReader`). Owns initial
//! script ordering; the application supplies live owners and scoped reads.
//! Sync port: the donor's async reads and frame pump become sync closures
//! plus the injected [`StartupCommandQueue`] seam. Dialects, origins,
//! completions, content paths, and Latin-1 reads reuse the ported core.

use std::path::PathBuf;

use qa_content::paths::find_content_path;
use qa_content::paths::PathComparison;
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::CommandContext;
use qa_core::cmd_buffer::CommandOrigin;
use qa_core::cmd_buffer::ScriptCompletion;
use qa_core::cmd_buffer::ScriptResult;
use thiserror::Error;

/// Startup configuration failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StartupConfigError {
    /// Scripts ended before default and archived stages ran.
    #[error("Startup script did not reach default and archived configuration stages")]
    StagesUnreached,
    /// A script read failed.
    #[error("Startup script read failed: {0}")]
    Read(String),
    /// A script execution failed.
    #[error("{0}")]
    Script(String),
}

/// Where a startup script is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartupScriptScope {
    /// Mounted content.
    Mounted,
    /// User configuration.
    User,
    /// Loose base files.
    BaseLoose,
    /// Loose game files.
    GameLoose,
    /// Loose files, game roots before base roots.
    Loose,
    /// Seat settings.
    Seat,
}

/// Configuration owner scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartupConfigScope {
    /// Source (server/world) configuration.
    #[default]
    Source,
    /// Seat configuration.
    Seat,
}

/// One ordered startup script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupScript {
    /// Script file name.
    pub name: &'static str,
    /// Read scope.
    pub scope: StartupScriptScope,
}

/// Script depth of an origin (donor `scriptDepth`).
fn script_depth(mut origin: &CommandOrigin) -> usize {
    let mut depth = 0;
    while let CommandOrigin::Script { caller, .. } = origin {
        depth += 1;
        origin = caller;
    }
    depth
}

/// Ordered scripts for a dialect (donor `startupScripts`).
fn startup_scripts(dialect: Dialect, has_mod: bool, dedicated_quakeworld: bool) -> Vec<StartupScript> {
    if dedicated_quakeworld {
        return vec![StartupScript {
            name: "server.cfg",
            scope: StartupScriptScope::User,
        }];
    }
    match dialect {
        Dialect::Q1Netquake | Dialect::Q1Quakeworld => {
            vec![StartupScript {
                name: "quake.rc",
                scope: StartupScriptScope::Mounted,
            }]
        }
        Dialect::Q3 => vec![
            StartupScript {
                name: "default.cfg",
                scope: StartupScriptScope::Mounted,
            },
            StartupScript {
                name: "q3config.cfg",
                scope: StartupScriptScope::User,
            },
            StartupScript {
                name: "autoexec.cfg",
                scope: StartupScriptScope::User,
            },
        ],
        Dialect::Q2Rerelease => {
            let mut scripts = vec![
                StartupScript {
                    name: "default.cfg",
                    scope: StartupScriptScope::Mounted,
                },
                StartupScript {
                    name: "config.cfg",
                    scope: StartupScriptScope::Loose,
                },
            ];
            if has_mod {
                scripts.push(StartupScript {
                    name: "autoexec.cfg",
                    scope: StartupScriptScope::BaseLoose,
                });
            }
            scripts.push(StartupScript {
                name: "autoexec.cfg",
                scope: StartupScriptScope::GameLoose,
            });
            scripts.push(StartupScript {
                name: "postexec.cfg",
                scope: StartupScriptScope::Loose,
            });
            scripts
        }
        Dialect::Q2Classic => vec![
            StartupScript {
                name: "default.cfg",
                scope: StartupScriptScope::Mounted,
            },
            StartupScript {
                name: "config.cfg",
                scope: StartupScriptScope::User,
            },
            StartupScript {
                name: "autoexec.cfg",
                scope: StartupScriptScope::GameLoose,
            },
        ],
    }
}

/// Owned startup buffer (donor `Pick<CommandBuffer, "append" | "executeScriptsAsync">`).
pub trait StartupCommandQueue {
    /// Queue `exec <script>` for the startup context.
    fn append(&mut self, text: &str, source: &CommandContext);
    /// Dispatch until the queue parks; false means a native wait left work.
    fn execute_scripts(&mut self, after_dispatch: &mut dyn FnMut(), should_continue: Option<&dyn Fn() -> bool>);
}

/// Scoped script read (donor `StartupConfigOptions["read"]`).
pub type StartupScriptRead = Box<dyn FnMut(&str, &CommandContext, StartupScriptScope) -> Option<String>>;

/// Startup configuration owners and scoped reads (donor `StartupConfigOptions`).
pub struct StartupConfigOptions {
    /// Command dialect.
    pub dialect: Dialect,
    /// Startup context.
    pub context: CommandContext,
    /// Whether a mod is selected.
    pub has_mod: bool,
    /// Owner scope.
    pub scope: StartupConfigScope,
    /// Skip `q3config.cfg` (Q3 safe mode).
    pub safe_mode: bool,
    /// Scoped script read.
    pub read: StartupScriptRead,
    /// Apply selected defaults after `default.cfg`.
    pub apply_selected_defaults: Box<dyn FnMut()>,
    /// Apply archived configuration after `config.cfg`/`q3config.cfg`.
    pub apply_archive: Box<dyn FnMut()>,
    /// Apply launch options after the last script.
    pub apply_launch_options: Box<dyn FnMut()>,
    /// Replay startup variables (Q3 `autoexec.cfg`, Q2 `config.cfg`).
    pub replay_startup_variables: Option<Box<dyn FnMut()>>,
}

/// Owns initial script ordering.
pub struct StartupConfig {
    options: StartupConfigOptions,
    scripts: Vec<StartupScript>,
    caller_depth: usize,
    index: usize,
    active: Option<StartupScript>,
    completed: bool,
    failure: Option<String>,
    dedicated_quakeworld: bool,
    defaults_applied: bool,
    archive_applied: bool,
}

impl StartupConfig {
    /// Build ordering for the given owners.
    #[must_use]
    pub fn new(options: StartupConfigOptions) -> Self {
        let mut origin = &options.context.origin;
        while let CommandOrigin::Script { caller, .. } = origin {
            origin = caller;
        }
        let dedicated_quakeworld =
            options.dialect == Dialect::Q1Quakeworld && matches!(origin, CommandOrigin::ServerConsole);
        let scripts = if options.scope == StartupConfigScope::Seat {
            vec![
                StartupScript {
                    name: if options.dialect == Dialect::Q3 {
                        "q3config.cfg"
                    } else {
                        "config.cfg"
                    },
                    scope: StartupScriptScope::Seat,
                },
                StartupScript {
                    name: "autoexec.cfg",
                    scope: StartupScriptScope::Seat,
                },
            ]
        } else {
            startup_scripts(options.dialect, options.has_mod, dedicated_quakeworld)
        };
        let defaults_applied = options.scope == StartupConfigScope::Seat;
        let caller_depth = script_depth(&options.context.origin);
        Self {
            options,
            scripts,
            caller_depth,
            index: 0,
            active: None,
            completed: false,
            failure: None,
            dedicated_quakeworld,
            defaults_applied,
            archive_applied: false,
        }
    }

    /// Whether a source belongs to the active script.
    #[must_use]
    pub fn owns_source(&self, source: &CommandContext) -> bool {
        let Some(active) = self.active else {
            return false;
        };
        if source.session != self.options.context.session {
            return false;
        }
        let mut root = &source.origin;
        let mut depth = script_depth(root);
        while let CommandOrigin::Script { caller, .. } = root {
            if depth <= self.caller_depth + 1 {
                break;
            }
            root = caller;
            depth -= 1;
        }
        let CommandOrigin::Script { name, caller } = root else {
            return false;
        };
        if depth != self.caller_depth + 1 || name != active.name {
            return false;
        }
        let mut caller = caller.as_ref();
        let mut expected = &self.options.context.origin;
        while let (
            CommandOrigin::Script {
                name: left,
                caller: next_left,
            },
            CommandOrigin::Script {
                name: right,
                caller: next_right,
            },
        ) = (caller, expected)
        {
            if left != right {
                return false;
            }
            caller = next_left;
            expected = next_right;
        }
        match (caller, expected) {
            (CommandOrigin::LocalConsole, CommandOrigin::LocalConsole)
            | (CommandOrigin::ServerConsole, CommandOrigin::ServerConsole) => true,
            (
                CommandOrigin::LocalSeat { seat, client },
                CommandOrigin::LocalSeat {
                    seat: other_seat,
                    client: other_client,
                },
            ) => seat == other_seat && client == other_client,
            (CommandOrigin::RemoteClient { client }, CommandOrigin::RemoteClient { client: other }) => client == other,
            _ => false,
        }
    }

    /// Whether shared configuration is restricted to the seat script.
    #[must_use]
    pub fn restrict_shared_configuration(&self) -> bool {
        self.options.scope == StartupConfigScope::Seat
            && matches!(
                self.active.map(|script| script.name),
                Some("config.cfg" | "q3config.cfg")
            )
    }

    /// Read a script with the donor's scope routing.
    pub fn read_script(&mut self, name: &str, source: &CommandContext) -> Option<String> {
        let depth = script_depth(&source.origin);
        let active = self.active;
        let direct = active.is_some_and(|script| depth == self.caller_depth && name == script.name);
        let q1_default = active.is_some_and(|script| script.name == "quake.rc")
            && depth == self.caller_depth + 1
            && matches!(&source.origin, CommandOrigin::Script { name, .. } if name == "quake.rc")
            && name == "default.cfg";
        let scope = if direct {
            active.expect("direct implies active").scope
        } else if q1_default {
            StartupScriptScope::Mounted
        } else if self.options.scope == StartupConfigScope::Seat {
            StartupScriptScope::Seat
        } else {
            StartupScriptScope::User
        };
        (self.options.read)(name, source, scope)
    }

    /// Fold a script completion into stage tracking.
    pub fn on_script_complete(&mut self, event: &ScriptCompletion) -> Result<(), StartupConfigError> {
        let Some(active) = self.active else {
            return Ok(());
        };
        let depth = script_depth(&event.source.origin);
        let direct = depth == self.caller_depth + 1 && event.name == active.name;
        let q1_child = active.name == "quake.rc"
            && depth == self.caller_depth + 2
            && matches!(&event.source.origin, CommandOrigin::Script { caller, .. }
                if matches!(caller.as_ref(), CommandOrigin::Script { name, .. } if name == "quake.rc"));
        if let ScriptResult::Failed(message) = &event.result {
            return Err(StartupConfigError::Script(message.clone()));
        }
        if !direct && !q1_child {
            return Ok(());
        }
        if event.name == "default.cfg" && !self.defaults_applied {
            self.defaults_applied = true;
            (self.options.apply_selected_defaults)();
        }
        if (event.name == "config.cfg" || event.name == "q3config.cfg") && !self.archive_applied {
            self.archive_applied = true;
            (self.options.apply_archive)();
        }
        if (self.options.dialect == Dialect::Q3 && direct && event.name == "autoexec.cfg")
            || (self.options.dialect.is_q2() && direct && event.name == "config.cfg")
        {
            if let Some(replay) = self.options.replay_startup_variables.as_mut() {
                replay();
            }
        }
        if direct {
            self.active = None;
        }
        Ok(())
    }

    /// Advance one frame; false means a native wait left work for another frame.
    pub fn execute_frame(
        &mut self,
        commands: &mut dyn StartupCommandQueue,
        after_dispatch: &mut dyn FnMut(),
        should_continue: Option<&dyn Fn() -> bool>,
    ) -> Result<bool, StartupConfigError> {
        if let Some(failure) = &self.failure {
            return Err(StartupConfigError::Script(failure.clone()));
        }
        let result = self.advance_frame(commands, after_dispatch, should_continue);
        if let Err(error) = &result {
            self.failure = Some(error.to_string());
        }
        result
    }

    fn advance_frame(
        &mut self,
        commands: &mut dyn StartupCommandQueue,
        after_dispatch: &mut dyn FnMut(),
        should_continue: Option<&dyn Fn() -> bool>,
    ) -> Result<bool, StartupConfigError> {
        if self.completed {
            return Ok(true);
        }
        if self.dedicated_quakeworld && !self.archive_applied {
            self.defaults_applied = true;
            (self.options.apply_selected_defaults)();
            self.archive_applied = true;
            (self.options.apply_archive)();
        }
        loop {
            if self.active.is_none() {
                let Some(script) = self.scripts.get(self.index).copied() else {
                    if !self.defaults_applied || !self.archive_applied {
                        return Err(StartupConfigError::StagesUnreached);
                    }
                    self.completed = true;
                    (self.options.apply_launch_options)();
                    return Ok(true);
                };
                self.index += 1;
                if self.options.dialect == Dialect::Q3 && self.options.safe_mode && script.name == "q3config.cfg" {
                    self.archive_applied = true;
                    continue;
                }
                self.active = Some(script);
                commands.append(&format!("exec {}\n", script.name), &self.options.context);
            }
            commands.execute_scripts(after_dispatch, should_continue);
            if self.active.is_some() || should_continue.is_some_and(|check| !check()) {
                return Ok(false);
            }
        }
    }
}

/// Scoped reader owners (donor `StartupScriptReaderOptions`).
pub struct StartupScriptReaderOptions<Mounted, User> {
    /// Mounted-content read.
    pub mounted: Mounted,
    /// User-configuration read.
    pub user: User,
    /// Loose base roots, highest priority first.
    pub base_loose_roots: Vec<PathBuf>,
    /// Loose game roots, highest priority first.
    pub game_loose_roots: Vec<PathBuf>,
    /// Seat settings root.
    pub seat_root: Option<PathBuf>,
}

/// Build a scoped script read (roots already ordered, highest priority first).
pub fn create_startup_script_reader<Mounted, User>(
    options: StartupScriptReaderOptions<Mounted, User>,
) -> impl FnMut(&str, &CommandContext, StartupScriptScope) -> Option<String>
where
    Mounted: FnMut(&str) -> Option<Vec<u8>>,
    User: FnMut(&str, &CommandContext) -> Option<String>,
{
    let mut mounted = options.mounted;
    let mut user = options.user;
    let base_loose_roots = options.base_loose_roots;
    let game_loose_roots = options.game_loose_roots;
    let seat_root = options.seat_root;
    move |name, source, scope| {
        if scope == StartupScriptScope::Seat {
            let mut origin = &source.origin;
            while let CommandOrigin::Script { caller, .. } = origin {
                origin = caller;
            }
            let CommandOrigin::LocalSeat { seat, .. } = origin else {
                return None;
            };
            let root = seat_root.as_ref()?;
            let path = find_content_path(
                root,
                &format!("settings/seat-{}/{}", seat.index(), name),
                PathComparison::CaseInsensitive,
            )
            .ok()??;
            return std::fs::read(&path)
                .ok()
                .map(|bytes| bytes.iter().map(|&byte| byte as char).collect());
        }
        if scope == StartupScriptScope::User {
            return user(name, source);
        }
        if scope == StartupScriptScope::Mounted {
            let bytes = mounted(name)?;
            return Some(bytes.iter().map(|&byte| byte as char).collect());
        }
        let roots: Vec<&PathBuf> = match scope {
            StartupScriptScope::BaseLoose => base_loose_roots.iter().collect(),
            StartupScriptScope::GameLoose => game_loose_roots.iter().collect(),
            _ => game_loose_roots.iter().chain(base_loose_roots.iter()).collect(),
        };
        for root in roots {
            let path = find_content_path(root, name, PathComparison::CaseInsensitive).ok()??;
            if let Ok(bytes) = std::fs::read(&path) {
                return Some(bytes.iter().map(|&byte| byte as char).collect());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_core::identity::IdentityOwner;

    use super::*;

    fn owner() -> IdentityOwner {
        IdentityOwner::create("startup-config-test").unwrap()
    }

    fn context(owner: &IdentityOwner, origin: CommandOrigin) -> CommandContext {
        CommandContext::new(owner.session().clone(), origin)
    }

    struct Calls {
        defaults: usize,
        archive: usize,
        launch: usize,
        replay: usize,
        reads: Vec<(String, StartupScriptScope)>,
    }

    fn harness(
        dialect: Dialect,
        scope: StartupConfigScope,
        origin: CommandOrigin,
    ) -> (StartupConfig, Rc<RefCell<Calls>>, IdentityOwner) {
        let owner = owner();
        let calls = Rc::new(RefCell::new(Calls {
            defaults: 0,
            archive: 0,
            launch: 0,
            replay: 0,
            reads: Vec::new(),
        }));
        let reads = Rc::clone(&calls);
        let defaults = Rc::clone(&calls);
        let archive = Rc::clone(&calls);
        let launch = Rc::clone(&calls);
        let replay = Rc::clone(&calls);
        let config = StartupConfig::new(StartupConfigOptions {
            dialect,
            context: context(&owner, origin),
            has_mod: false,
            scope,
            safe_mode: false,
            read: Box::new(move |name, _, scope| {
                reads.borrow_mut().reads.push((name.to_string(), scope));
                Some(format!("// {name}"))
            }),
            apply_selected_defaults: Box::new(move || defaults.borrow_mut().defaults += 1),
            apply_archive: Box::new(move || archive.borrow_mut().archive += 1),
            apply_launch_options: Box::new(move || launch.borrow_mut().launch += 1),
            replay_startup_variables: Some(Box::new(move || replay.borrow_mut().replay += 1)),
        });
        (config, calls, owner)
    }

    struct FakeQueue {
        appended: Vec<String>,
    }

    impl StartupCommandQueue for FakeQueue {
        fn append(&mut self, text: &str, _source: &CommandContext) {
            self.appended.push(text.to_string());
        }

        fn execute_scripts(&mut self, after_dispatch: &mut dyn FnMut(), _should_continue: Option<&dyn Fn() -> bool>) {
            after_dispatch();
        }
    }

    fn completion(owner: &IdentityOwner, name: &str, origin: CommandOrigin) -> ScriptCompletion {
        ScriptCompletion {
            name: name.to_string(),
            source: context(owner, origin),
            result: ScriptResult::Completed,
        }
    }

    fn script_origin(name: &str, caller: CommandOrigin) -> CommandOrigin {
        CommandOrigin::Script {
            name: name.to_string(),
            caller: Box::new(caller),
        }
    }

    #[test]
    fn orders_q3_scripts_and_stages() {
        let (mut config, calls, owner) = harness(Dialect::Q3, StartupConfigScope::Source, CommandOrigin::LocalConsole);
        let mut queue = FakeQueue { appended: Vec::new() };
        assert!(!config.execute_frame(&mut queue, &mut || {}, None).unwrap());
        assert_eq!(queue.appended, vec!["exec default.cfg\n".to_string()]);
        let event = completion(
            &owner,
            "default.cfg",
            script_origin("default.cfg", CommandOrigin::LocalConsole),
        );
        config.on_script_complete(&event).unwrap();
        assert_eq!(calls.borrow().defaults, 1);
        assert!(!config.execute_frame(&mut queue, &mut || {}, None).unwrap());
        assert_eq!(queue.appended.len(), 2);
        let event = completion(
            &owner,
            "q3config.cfg",
            script_origin("q3config.cfg", CommandOrigin::LocalConsole),
        );
        config.on_script_complete(&event).unwrap();
        assert_eq!(calls.borrow().archive, 1);
        assert!(!config.execute_frame(&mut queue, &mut || {}, None).unwrap());
        let event = completion(
            &owner,
            "autoexec.cfg",
            script_origin("autoexec.cfg", CommandOrigin::LocalConsole),
        );
        config.on_script_complete(&event).unwrap();
        assert_eq!(calls.borrow().replay, 1);
        assert!(config.execute_frame(&mut queue, &mut || {}, None).unwrap());
        assert_eq!(calls.borrow().launch, 1);
        assert!(config.execute_frame(&mut queue, &mut || {}, None).unwrap());
    }

    #[test]
    fn q1_uses_quake_rc_with_mounted_default() {
        let (mut config, calls, owner) = harness(
            Dialect::Q1Netquake,
            StartupConfigScope::Source,
            CommandOrigin::LocalConsole,
        );
        let mut queue = FakeQueue { appended: Vec::new() };
        assert!(!config.execute_frame(&mut queue, &mut || {}, None).unwrap());
        assert_eq!(queue.appended, vec!["exec quake.rc\n".to_string()]);
        let nested = script_origin("quake.rc", CommandOrigin::LocalConsole);
        let source = context(&owner, nested);
        config.read_script("default.cfg", &source);
        assert_eq!(
            calls.borrow().reads.last(),
            Some(&("default.cfg".to_string(), StartupScriptScope::Mounted))
        );
        let child = completion(
            &owner,
            "default.cfg",
            script_origin("default.cfg", script_origin("quake.rc", CommandOrigin::LocalConsole)),
        );
        config.on_script_complete(&child).unwrap();
        assert_eq!(calls.borrow().defaults, 1);
        let direct = completion(
            &owner,
            "quake.rc",
            script_origin("quake.rc", CommandOrigin::LocalConsole),
        );
        assert!(config.owns_source(&direct.source));
        let foreign = context(&owner, CommandOrigin::LocalConsole);
        assert!(!config.owns_source(&foreign));
        config.on_script_complete(&direct).unwrap();
        assert_eq!(
            config.execute_frame(&mut queue, &mut || {}, None),
            Err(StartupConfigError::StagesUnreached)
        );
    }

    #[test]
    fn seat_scope_restricts_shared_configuration() {
        let owner = owner();
        let origin = CommandOrigin::LocalSeat {
            seat: owner.seat(0),
            client: owner.client(0, 0),
        };
        let (mut config, _, _) = harness(Dialect::Q3, StartupConfigScope::Seat, origin);
        let mut queue = FakeQueue { appended: Vec::new() };
        assert!(!config.execute_frame(&mut queue, &mut || {}, None).unwrap());
        assert_eq!(queue.appended, vec!["exec q3config.cfg\n".to_string()]);
        assert!(config.restrict_shared_configuration());
    }

    #[test]
    fn q2_replay_and_failure_stick() {
        let (mut config, calls, owner) = harness(
            Dialect::Q2Classic,
            StartupConfigScope::Source,
            CommandOrigin::LocalConsole,
        );
        let mut queue = FakeQueue { appended: Vec::new() };
        for name in ["default.cfg", "config.cfg", "autoexec.cfg"] {
            assert!(!config.execute_frame(&mut queue, &mut || {}, None).unwrap());
            let event = completion(&owner, name, script_origin(name, CommandOrigin::LocalConsole));
            config.on_script_complete(&event).unwrap();
        }
        assert_eq!(calls.borrow().replay, 1);
        assert!(config.execute_frame(&mut queue, &mut || {}, None).unwrap());
        let (mut failing, _, fail_owner) =
            harness(Dialect::Q3, StartupConfigScope::Source, CommandOrigin::LocalConsole);
        let mut fail_queue = FakeQueue { appended: Vec::new() };
        assert!(!failing.execute_frame(&mut fail_queue, &mut || {}, None).unwrap());
        let failed = ScriptCompletion {
            name: "default.cfg".to_string(),
            source: context(&fail_owner, script_origin("default.cfg", CommandOrigin::LocalConsole)),
            result: ScriptResult::Failed("boom".to_string()),
        };
        assert_eq!(
            failing.on_script_complete(&failed),
            Err(StartupConfigError::Script("boom".to_string()))
        );
    }

    #[test]
    fn dedicated_quakeworld_applies_stages_upfront() {
        let (mut config, calls, _) = harness(
            Dialect::Q1Quakeworld,
            StartupConfigScope::Source,
            CommandOrigin::ServerConsole,
        );
        let mut queue = FakeQueue { appended: Vec::new() };
        assert!(!config.execute_frame(&mut queue, &mut || {}, None).unwrap());
        assert_eq!(queue.appended, vec!["exec server.cfg\n".to_string()]);
        assert_eq!(calls.borrow().defaults, 1);
        assert_eq!(calls.borrow().archive, 1);
    }

    #[test]
    fn script_reader_routes_scopes() {
        let root = std::env::temp_dir().join(format!("startup-config-{}", std::process::id()));
        let game = root.join("game");
        let base = root.join("base");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(game.join("autoexec.cfg"), "game-exec").unwrap();
        std::fs::write(base.join("autoexec.cfg"), "base-exec").unwrap();
        let owner = owner();
        let source = context(&owner, CommandOrigin::LocalConsole);
        let mut read = create_startup_script_reader(StartupScriptReaderOptions {
            mounted: |name: &str| {
                if name == "default.cfg" {
                    Some(vec![65, 66])
                } else {
                    None
                }
            },
            user: |name: &str, _: &CommandContext| Some(format!("user:{name}")),
            base_loose_roots: vec![base.clone()],
            game_loose_roots: vec![game.clone()],
            seat_root: None,
        });
        assert_eq!(
            read("default.cfg", &source, StartupScriptScope::Mounted),
            Some("AB".to_string())
        );
        assert_eq!(
            read("q3config.cfg", &source, StartupScriptScope::User),
            Some("user:q3config.cfg".to_string())
        );
        assert_eq!(
            read("autoexec.cfg", &source, StartupScriptScope::Loose),
            Some("game-exec".to_string())
        );
        assert_eq!(
            read("autoexec.cfg", &source, StartupScriptScope::BaseLoose),
            Some("base-exec".to_string())
        );
        assert_eq!(read("missing.cfg", &source, StartupScriptScope::GameLoose), None);
        assert_eq!(read("config.cfg", &source, StartupScriptScope::Seat), None);
        std::fs::remove_dir_all(&root).ok();
    }
}
