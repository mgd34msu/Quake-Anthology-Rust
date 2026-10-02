//! Quake II operator console over the simulation's source registry.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q2-console.ts`
//! (`q2OperatorPlayerName`, `ApplicationQ2ConsoleOptions`, `ApplicationQ2Console`, plus the
//! donor's re-export of `LMCTF_CONSOLE_NAMES`). The simulation runtime
//! (`./simulation/runtime.ts`, out of scope) arrives through the [`Q2ConsoleHost`] seam,
//! which keeps every donor behavior here: the constructor guards, `quit`/`map`/`say`
//! forwarding, the `status`/`dumpuser` reports, shared-name selection, and the maplist
//! load with its line parsing. Content mounts and LMCTF match rules are the canonical
//! `super::content::RemoteContentMounts` and
//! `qa_content::q2::multiplayer::lmctf::runtime::Q2Lmctf` ports, held host-side; the
//! donor's async file open is sync through the host. `bindLmctfConsoleRules` is the
//! canonical [`bind_lmctf_console_rules`], re-exported below for host seam
//! implementations.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::cmd_buffer::{BufferError, CommandBuffer, CommandContext, Invocation};
use qa_core::cvar::CvarRegistry;
use thiserror::Error;

pub use crate::settings::q2_owner::bind_lmctf_console_rules;
pub use crate::settings::server::LMCTF_CONSOLE_NAMES;

/// Parse a Q2 userinfo string into ordered pairs (donor `q2Userinfo`).
///
/// One leading backslash is stripped, the rest splits into key/value pairs, the first
/// occurrence of a key wins, and a dangling trailing token is dropped.
fn q2_userinfo_pairs(source: &str) -> Vec<(String, String)> {
    let body = source.strip_prefix('\\').unwrap_or(source);
    let tokens: Vec<&str> = body.split('\\').collect();
    let mut seen = HashMap::new();
    let mut pairs = Vec::new();
    let mut index = 0;
    while index + 1 < tokens.len() {
        let (key, value) = (tokens[index], tokens[index + 1]);
        if !seen.contains_key(key) {
            seen.insert(key.to_string(), ());
            pairs.push((key.to_string(), value.to_string()));
        }
        index += 2;
    }
    pairs
}

/// Look up one userinfo key (donor `q2Userinfo(...).get(...)`).
fn q2_userinfo_value(source: &str, key: &str) -> Option<String> {
    q2_userinfo_pairs(source)
        .into_iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// One connected Q2 player as the console reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ConsolePlayerView {
    /// Client slot.
    pub slot: u32,
    /// Score.
    pub score: i32,
    /// Ping in milliseconds.
    pub ping: i32,
    /// Whether the slot is connected.
    pub connected: bool,
    /// Raw userinfo string.
    pub userinfo: String,
    /// Engine player name fallback.
    pub name: String,
}

/// Operator-facing player name (donor `q2OperatorPlayerName`).
#[must_use]
pub fn q2_operator_player_name(player: &Q2ConsolePlayerView) -> String {
    q2_userinfo_value(&player.userinfo, "name").unwrap_or_else(|| player.name.clone())
}

/// Pad a dump key the way the donor's `padEnd(20)` does.
fn pad_dump_key(key: &str) -> String {
    let width = key.chars().count();
    if width >= 20 {
        key.to_string()
    } else {
        format!("{key}{}", " ".repeat(20 - width))
    }
}

/// Simulation, content, and match-rule seam (donor `ApplicationQ2ConsoleOptions`).
pub trait Q2ConsoleHost {
    /// Whether a Q2 source runtime exists.
    fn has_q2_source(&self) -> bool;
    /// The simulation's source registry, when present.
    fn server_cvars(&mut self) -> Option<&mut CvarRegistry>;
    /// Current map name, or [`None`] when no server runs.
    fn map_name(&self) -> Option<String>;
    /// Every player slot.
    fn players(&self) -> Vec<Q2ConsolePlayerView>;
    /// Whether the match source is LMCTF.
    fn lmctf_active(&self) -> bool;
    /// The configured maplist file name (`maplist_file`).
    fn maplist_file_name(&self) -> String;
    /// Open a match-content file, or [`None`] when absent.
    fn match_file(&mut self, path: &str) -> Option<Vec<u8>>;
    /// Apply the canonical LMCTF console rules to the source registry (hosts call
    /// the re-exported [`bind_lmctf_console_rules`] on their registry and rules).
    fn bind_lmctf_console_rules(&mut self);
    /// Replace the LMCTF map list.
    fn set_lmctf_map_list(&mut self, maps: Vec<String>);
}

/// Failure to build the console, with donor messages.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2ConsoleError {
    /// No Q2 source runtime is present.
    #[error("Q2 console requires a Q2 source runtime")]
    NoSourceRuntime,
    /// The simulation has no source registry.
    #[error("Q2 console requires the simulation's source registry")]
    NoSourceRegistry,
}

/// Console print sink (donor `ApplicationQ2ConsoleOptions["print"]`).
pub type Q2ConsolePrint = Rc<dyn Fn(&str)>;
/// Command forwarding sink (donor `ApplicationQ2ConsoleOptions["execute"]`).
pub type Q2ConsoleExecute = Rc<dyn Fn(&str, &[String], &CommandContext)>;

/// Quake II operator console (donor `ApplicationQ2Console`).
pub struct ApplicationQ2Console<H> {
    host: Rc<RefCell<H>>,
    print: Q2ConsolePrint,
    execute: Q2ConsoleExecute,
}

impl<H: Q2ConsoleHost + 'static> ApplicationQ2Console<H> {
    /// Build the console; fails without a Q2 source runtime.
    pub fn new(host: H, print: Q2ConsolePrint, execute: Q2ConsoleExecute) -> Result<Self, Q2ConsoleError> {
        if !host.has_q2_source() {
            return Err(Q2ConsoleError::NoSourceRuntime);
        }
        Ok(Self {
            host: Rc::new(RefCell::new(host)),
            print,
            execute,
        })
    }

    /// Shared access to the host (covers the donor `cvars` getter via
    /// [`Q2ConsoleHost::server_cvars`]).
    #[must_use]
    pub fn host(&self) -> Rc<RefCell<H>> {
        Rc::clone(&self.host)
    }

    /// Register the console commands (donor `bind`).
    pub fn bind(&mut self, commands: &mut CommandBuffer, cvars: &CvarRegistry) -> Result<Q2ConsoleGuard, BufferError> {
        let mut names = Vec::new();
        for name in ["quit", "map", "say"] {
            let execute = Rc::clone(&self.execute);
            let owned = name.to_string();
            if commands.register(
                name,
                Some(Rc::new(move |invocation: &mut Invocation| {
                    execute(&owned, invocation.args(), &invocation.source);
                })),
                None,
                cvars,
            )? {
                names.push(name.to_string());
            }
        }
        {
            let host = Rc::clone(&self.host);
            let print = Rc::clone(&self.print);
            if commands.register(
                "status",
                Some(Rc::new(move |_invocation: &mut Invocation| {
                    let host = host.borrow();
                    let Some(map) = host.map_name() else {
                        print("No server running.\n");
                        return;
                    };
                    print(&format!("map              : {map}\nnum score ping name\n"));
                    for player in host.players() {
                        if player.connected {
                            let name = q2_operator_player_name(&player);
                            print(&format!("{} {} {} {name}\n", player.slot, player.score, player.ping));
                        }
                    }
                })),
                None,
                cvars,
            )? {
                names.push("status".to_string());
            }
        }
        {
            let host = Rc::clone(&self.host);
            let print = Rc::clone(&self.print);
            if commands.register(
                "dumpuser",
                Some(Rc::new(move |invocation: &mut Invocation| {
                    let target = invocation.args().first();
                    if target.is_none() || invocation.args().len() != 1 {
                        print("Usage: dumpuser <player name|slot>\n");
                        return;
                    }
                    let target = target.map_or("", String::as_str);
                    let host = host.borrow();
                    let player = host.players().into_iter().find(|player| {
                        player.connected
                            && if !target.is_empty() && target.bytes().all(|byte| byte.is_ascii_digit()) {
                                target.parse::<u32>().is_ok_and(|slot| player.slot == slot)
                            } else {
                                q2_operator_player_name(player) == target
                            }
                    });
                    let Some(player) = player else {
                        print(&format!("Player {target} is not on the server\n"));
                        return;
                    };
                    print("userinfo\n--------\n");
                    for (key, value) in q2_userinfo_pairs(&player.userinfo) {
                        print(&format!("{}{value}\n", pad_dump_key(&key)));
                    }
                })),
                None,
                cvars,
            )? {
                names.push("dumpuser".to_string());
            }
        }
        Ok(Q2ConsoleGuard { names })
    }

    /// Cvar names shared with the server (donor `sharedNames`).
    #[must_use]
    pub fn shared_names(&self) -> Vec<&'static str> {
        let mut names = vec!["dmflags", "timelimit", "fraglimit", "capturelimit"];
        if self.host.borrow().lmctf_active() {
            names.extend(LMCTF_CONSOLE_NAMES.iter().copied());
        }
        names
    }

    /// Bind the current match mode's console rules (donor `initialize`/`bindCurrent`).
    pub fn bind_current(&mut self) {
        if !self.host.borrow().lmctf_active() {
            return;
        }
        self.host.borrow_mut().bind_lmctf_console_rules();
        let configured = self.host.borrow().maplist_file_name();
        let mut host = self.host.borrow_mut();
        let file = host.match_file(&configured).or_else(|| host.match_file("maplist.txt"));
        if let Some(bytes) = file {
            let text = String::from_utf8_lossy(&bytes);
            let mut maps = Vec::new();
            for line in text.split('\n') {
                let line = line.strip_suffix('\r').unwrap_or(line);
                if let Some(name) = line.split_whitespace().next() {
                    if !name.is_empty() {
                        maps.push(name.to_lowercase());
                    }
                }
            }
            host.set_lmctf_map_list(maps);
        }
    }
}

/// Registered console commands with their release (donor `bind` cleanup).
#[derive(Debug, Default)]
pub struct Q2ConsoleGuard {
    names: Vec<String>,
}

impl Q2ConsoleGuard {
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

/// Re-export surface check: the donor re-exports these console names.
#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::cmd_buffer::{BufferOptions, CommandOrigin};
    use qa_core::identity::IdentityOwner;

    struct Stub {
        cvars: CvarRegistry,
        map: Option<String>,
        players: Vec<Q2ConsolePlayerView>,
        lmctf: bool,
        files: HashMap<String, Vec<u8>>,
        map_list: Vec<String>,
        bound: bool,
    }

    impl Q2ConsoleHost for Stub {
        fn has_q2_source(&self) -> bool {
            true
        }
        fn server_cvars(&mut self) -> Option<&mut CvarRegistry> {
            Some(&mut self.cvars)
        }
        fn map_name(&self) -> Option<String> {
            self.map.clone()
        }
        fn players(&self) -> Vec<Q2ConsolePlayerView> {
            self.players.clone()
        }
        fn lmctf_active(&self) -> bool {
            self.lmctf
        }
        fn maplist_file_name(&self) -> String {
            self.cvars.variable_string("maplist_file")
        }
        fn match_file(&mut self, path: &str) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }
        fn bind_lmctf_console_rules(&mut self) {
            self.bound = true;
        }
        fn set_lmctf_map_list(&mut self, maps: Vec<String>) {
            self.map_list = maps;
        }
    }

    fn stub() -> Stub {
        Stub {
            cvars: CvarRegistry::new(Dialect::Q2Rerelease),
            map: Some("q2dm1".to_string()),
            players: vec![
                Q2ConsolePlayerView {
                    slot: 0,
                    score: 5,
                    ping: 12,
                    connected: true,
                    userinfo: "\\name\\Ranger\\skin\\male/grunt".to_string(),
                    name: "unnamed".to_string(),
                },
                Q2ConsolePlayerView {
                    slot: 1,
                    score: 3,
                    ping: 40,
                    connected: false,
                    userinfo: String::new(),
                    name: "ghost".to_string(),
                },
            ],
            lmctf: false,
            files: HashMap::new(),
            map_list: Vec::new(),
            bound: false,
        }
    }

    type ConsoleHarness = (
        ApplicationQ2Console<Stub>,
        Rc<RefCell<Vec<String>>>,
        Rc<RefCell<Vec<String>>>,
    );

    fn console(stub: Stub) -> ConsoleHarness {
        let printed: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let forwarded: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&printed);
        let forward = Rc::clone(&forwarded);
        let console = ApplicationQ2Console::new(
            stub,
            Rc::new(move |text| sink.borrow_mut().push(text.to_string())),
            Rc::new(move |name, args, _| {
                forward.borrow_mut().push(format!("{name} {}", args.join(" ")));
            }),
        )
        .unwrap();
        (console, printed, forwarded)
    }

    fn buffer() -> (CommandBuffer, CvarRegistry) {
        let owner = IdentityOwner::create("q2-console").unwrap();
        let context = CommandContext::new(owner.session().clone(), CommandOrigin::LocalConsole);
        let buffer = CommandBuffer::new(Dialect::Q2Rerelease, context, BufferOptions::default()).unwrap();
        (buffer, CvarRegistry::new(Dialect::Q2Rerelease))
    }

    #[test]
    fn operator_name_prefers_userinfo() {
        let player = &stub().players[0];
        assert_eq!(q2_operator_player_name(player), "Ranger");
        let bare = Q2ConsolePlayerView {
            userinfo: String::new(),
            ..player.clone()
        };
        assert_eq!(q2_operator_player_name(&bare), "unnamed");
    }

    #[test]
    fn bind_registers_and_releases() {
        let (console_box, _, _) = console(stub());
        let mut console = console_box;
        let (mut commands, cvars) = buffer();
        let guard = console.bind(&mut commands, &cvars).unwrap();
        for name in ["quit", "map", "say", "status", "dumpuser"] {
            assert!(commands.exists(name), "{name}");
        }
        assert_eq!(guard.names().len(), 5);
        guard.release(&mut commands);
        assert!(!commands.exists("status"));
    }

    #[test]
    fn shared_names_gain_lmctf_entries() {
        let (plain, _, _) = console(stub());
        assert_eq!(plain.shared_names().len(), 4);
        let (lmctf, _, _) = console(Stub { lmctf: true, ..stub() });
        assert_eq!(lmctf.shared_names().len(), 4 + LMCTF_CONSOLE_NAMES.len());
    }

    #[test]
    fn bind_current_loads_maplist() {
        let mut files = HashMap::new();
        files.insert(
            "custom.txt".to_string(),
            b"Q2DM1 frag trivia\r\n  \r\nq2dm3\r\n".to_vec(),
        );
        let mut inner = stub();
        inner.lmctf = true;
        inner.files = files;
        inner.cvars.register("maplist_file", "custom.txt", 0).unwrap();
        let (mut console, _, _) = console(inner);
        console.bind_current();
        let host = console.host();
        let host = host.borrow();
        assert!(host.bound);
        assert_eq!(host.map_list, vec!["q2dm1".to_string(), "q2dm3".to_string()]);
    }

    #[test]
    fn bind_current_skips_without_lmctf() {
        let (mut console, _, _) = console(stub());
        console.bind_current();
        assert!(!console.host().borrow().bound);
    }
}
