//! Quake III presentation: console.
//!
//! Donor provenance: `src/content/q3/presentation/console.ts`.

use qa_core::cvar::{CvarRegistry, CvarSnapshot};
use qa_core::numeric::qvm_float_to_int;
use std::cell::Cell;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::{game_format_bounded, GameFormatArgument};
use crate::q3::base::game::numeric::{game_atof, game_atoi};
use crate::q3::base::shared::definitions::*;
use crate::q3::presentation::audio::PcmSound;
use crate::q3::presentation::client_info::ClientInfoStore;
use crate::q3::presentation::hud::WeaponService;
use crate::q3::presentation::hud::{same, Shared};
use crate::q3::presentation::mission_hud::CapturedMenu;
use crate::q3::presentation::state::*;

/// Rejection-style failure (donor promise rejection / `HudError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudError {
    /// Donor message text.
    pub message: String,
}

impl HudError {
    /// Build a rejection carrying the donor message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for HudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HudError {}

/// Available console HUD (`ClientConsoleHud` available arm).
pub trait ConsoleHud {
    /// Reset strings.
    fn reset_strings(&mut self);
    /// Reset menus.
    fn reset_menus(&mut self);
    /// Load menus.
    fn load_menus(&mut self, path: &str);
    /// Clear the scoreboard.
    fn clear_scoreboard(&mut self);
    /// Menu scoreboard.
    fn menu_scoreboard(&self) -> Option<CapturedMenu>;
    /// Scroll a feeder.
    fn scroll_feeder(&mut self, menu: &CapturedMenu, feeder: i32, down: bool);
}

/// Console HUD access (`ClientConsoleHud`).
#[derive(Clone)]
pub enum ConsoleHudAccess {
    /// Unavailable with a reason.
    Unavailable {
        /// Reason.
        reason: String,
    },
    /// Available HUD.
    Available(Shared<dyn ConsoleHud>),
}

/// Available team orders (`ClientConsoleTeamOrders` available arm).
pub trait ConsoleOrders {
    /// Select the next player.
    fn select_next_player(&mut self);
    /// Select the previous player.
    fn select_previous_player(&mut self);
    /// Whether the other team has the flag.
    fn other_team_has_flag(&self) -> bool;
    /// Whether your team has the flag.
    fn your_team_has_flag(&self) -> bool;
}

/// Team orders access (`ClientConsoleTeamOrders`).
#[derive(Clone)]
pub enum ConsoleOrdersAccess {
    /// Unavailable with a reason.
    Unavailable {
        /// Reason.
        reason: String,
    },
    /// Available orders.
    Available(Shared<dyn ConsoleOrders>),
}

/// Console host services (`ClientConsoleHost`).
pub trait ClientConsoleHost {
    /// Cvar registry.
    fn cvars(&self) -> Shared<CvarRegistry>;
    /// View runtime.
    fn view(&self) -> Shared<dyn ViewService>;
    /// Weapons.
    fn weapons(&self) -> Shared<dyn WeaponService>;
    /// Client store.
    fn clients(&self) -> Shared<dyn ClientInfoStore>;
    /// Server commands.
    fn server_commands(&self) -> Shared<dyn ServerCommandService>;
    /// HUD access.
    fn hud(&self) -> ConsoleHudAccess;
    /// Team orders access.
    fn team_orders(&self) -> ConsoleOrdersAccess;
    /// Read a cached VM cvar.
    fn read_vm_cvar(&self, name: &str) -> CvarSnapshot;
    /// Reset a player entity.
    fn reset_player_entity(&mut self, entity: &mut ClientEntity);
    /// Register a command name.
    fn add_command(&mut self, name: &str);
    /// Send a client command.
    fn send_client_command(&mut self, text: &str);
    /// Send a console command.
    fn send_console_command(&mut self, text: &str);
    /// Print.
    fn print(&mut self, text: &str);
    /// Center print.
    fn center_print(&mut self, text: &str, y: i32, char_width: i32);
    /// Menu end sound.
    fn sound(&self, name: MenuEndSound) -> Option<PcmSound>;
    /// Buffer a sound.
    fn add_buffered_sound(&mut self, sound: Option<PcmSound>);
}

/// Common console commands (`COMMON_COMMANDS`).
pub const COMMON_COMMANDS: [&str; 21] = [
    "testgun",
    "testmodel",
    "nextframe",
    "prevframe",
    "nextskin",
    "prevskin",
    "viewpos",
    "+scores",
    "-scores",
    "+zoom",
    "-zoom",
    "sizeup",
    "sizedown",
    "weapnext",
    "weapprev",
    "weapon",
    "tell_target",
    "tell_attacker",
    "vtell_target",
    "vtell_attacker",
    "tcmd",
];

/// Mission console commands (`MISSION_COMMANDS`).
pub const MISSION_COMMANDS: [&str; 24] = [
    "loadhud",
    "nextTeamMember",
    "prevTeamMember",
    "nextOrder",
    "confirmOrder",
    "denyOrder",
    "taskOffense",
    "taskDefense",
    "taskPatrol",
    "taskCamp",
    "taskFollow",
    "taskRetrieve",
    "taskEscort",
    "taskSuicide",
    "taskOwnFlag",
    "tauntKillInsult",
    "tauntPraise",
    "tauntTaunt",
    "tauntDeathInsult",
    "tauntGauntlet",
    "spWin",
    "spLose",
    "scoresDown",
    "scoresUp",
];

/// Forwarded console commands (`FORWARDED_COMMANDS`).
pub const FORWARDED_COMMANDS: [&str; 27] = [
    "kill",
    "say",
    "say_team",
    "tell",
    "vsay",
    "vsay_team",
    "vtell",
    "vtaunt",
    "vosay",
    "vosay_team",
    "votell",
    "give",
    "god",
    "notarget",
    "noclip",
    "team",
    "follow",
    "levelshot",
    "addbot",
    "setviewpos",
    "callvote",
    "vote",
    "callteamvote",
    "teamvote",
    "stats",
    "teamtask",
    "loaddefered",
];

/// Local command names (`localCommandNames`).
#[must_use]
pub fn local_command_names(product: Product) -> Vec<String> {
    let mut names: Vec<String> = COMMON_COMMANDS.iter().map(ToString::to_string).collect();
    if product == Product::Missionpack {
        names.extend(MISSION_COMMANDS.iter().map(ToString::to_string));
    }
    names.push("startOrbit".to_string());
    names.push("loaddeferred".to_string());
    names
}

/// All console command names (`clientConsoleCommandNames`).
#[must_use]
pub fn client_console_command_names(product: Product) -> Vec<String> {
    let mut names = local_command_names(product);
    names.extend(FORWARDED_COMMANDS.iter().map(ToString::to_string));
    names
}

/// ASCII fold (`fold`).
pub(crate) fn fold_command(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_uppercase() {
                (ch as u8 + 32) as char
            } else {
                ch
            }
        })
        .collect()
}

/// Validate source bytes cut at NUL (`sourceBytes`).
pub(crate) fn console_source_bytes(value: &str) -> Result<String, HudError> {
    let cut = match value.find('\0') {
        Some(end) => &value[..end],
        None => value,
    };
    for ch in cut.chars() {
        if ch as u32 > 255 {
            return Err(HudError::new("Console commands require source byte characters"));
        }
    }
    Ok(cut.to_string())
}

/// Command argument (`argument`).
pub(crate) fn console_argument(argv: &[String], index: usize, size: usize) -> String {
    argv.get(index)
        .map(|value| value.chars().take(size - 1).collect())
        .unwrap_or_default()
}

/// Console runtime (`ClientConsoleRuntime`).
pub struct ClientConsoleRuntime {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Host.
    pub host: Shared<dyn ClientConsoleHost>,
    /// Local command names.
    commands: Vec<String>,
    /// Closed.
    closed: Cell<bool>,
}

impl ClientConsoleRuntime {
    /// Assemble a console runtime.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        host: Shared<dyn ClientConsoleHost>,
    ) -> Self {
        if state.borrow().product != static_state.borrow().product
            || !same(&host.borrow().view().borrow().state_handle(), &state)
            || !same(&host.borrow().weapons().borrow().state_handle(), &state)
        {
            panic!("Console services must share canonical cgame state");
        }
        let commands = local_command_names(state.borrow().product);
        Self {
            state,
            static_state,
            host,
            commands,
            closed: Cell::new(false),
        }
    }

    /// Register command names (`initializeCommands`).
    pub fn initialize_commands(&self) {
        self.open();
        for name in client_console_command_names(self.state.borrow().product) {
            self.host.borrow_mut().add_command(&name);
        }
    }

    /// Dispose (`dispose`).
    pub fn dispose(&self) {
        self.closed.set(true);
        self.host.borrow_mut().view().borrow_mut().clear_test_model();
        self.host.borrow_mut().clients().borrow_mut().reset();
    }

    /// Require an open runtime (`open`).
    fn open(&self) {
        if self.closed.get() {
            panic!("Cgame console runtime is closed");
        }
    }

    /// Whether a name is handled (`handles`).
    pub fn handles(&self, name: &str) -> bool {
        self.open();
        let parsed = console_source_bytes(name);
        let Ok(parsed) = parsed else {
            panic!("Console commands require source byte characters");
        };
        let key = fold_command(&parsed);
        self.commands.iter().any(|command| fold_command(command) == key)
    }

    /// Execute a command (`execute`).
    pub fn execute(&self, argv: &[String]) -> Result<bool, HudError> {
        if argv.len() > 1024 {
            return Err(HudError::new("Console command exceeds MAX_STRING_TOKENS"));
        }
        let mut owned = Vec::with_capacity(argv.len());
        let mut storage = 0usize;
        for value in argv {
            let value = console_source_bytes(value)?;
            storage += value.chars().count() + 1;
            owned.push(value);
        }
        if storage > 8192 + 1024 {
            return Err(HudError::new("Console command exceeds source token storage"));
        }
        self.open();
        let name = fold_command(&console_argument(&owned, 0, 1024));
        if !self.handles(&name) {
            return Ok(false);
        }
        match self.dispatch(&name, &owned) {
            Ok(()) => {
                self.open();
                Ok(true)
            }
            Err(error) => {
                self.dispose();
                Err(error)
            }
        }
    }

    /// Crosshair player (`crosshairPlayer`).
    #[must_use]
    pub fn crosshair_player(&self) -> i32 {
        let state = self.state.borrow();
        if state.time > state.crosshair_client_time.wrapping_add(1000) {
            -1
        } else {
            state.crosshair_client_num
        }
    }

    /// Last attacker (`lastAttacker`).
    pub fn last_attacker(&self) -> Result<i32, HudError> {
        if self.state.borrow().attacker_time == 0 {
            return Ok(-1);
        }
        let snapshot = self.state.borrow().snap.clone();
        let Some(snapshot) = snapshot else {
            return Err(HudError::new("CG_LastAttacker requires cg.snap"));
        };
        Ok(snapshot
            .player_state
            .persistant
            .get(PersistentIndex::PersAttacker as usize))
    }

    /// Set a cvar.
    fn set(&self, name: &str, value: &str) -> Result<(), HudError> {
        self.host
            .borrow_mut()
            .cvars()
            .borrow_mut()
            .set(name, value, true)
            .map(|_| ())
            .map_err(|error| HudError::new(error.to_string()))
    }

    /// Immediate cvar text.
    fn immediate(&self, name: &str) -> Result<String, HudError> {
        let value = self.host.borrow().cvars().borrow().get(name);
        match value {
            None => Ok(String::new()),
            Some(snapshot) => Ok(console_source_bytes(&snapshot.value)?.chars().take(1023).collect()),
        }
    }

    /// Available HUD or rejection.
    fn hud(&self) -> Result<Shared<dyn ConsoleHud>, HudError> {
        match self.host.borrow().hud() {
            ConsoleHudAccess::Available(hud) => Ok(hud),
            ConsoleHudAccess::Unavailable { reason } => Err(HudError::new(format!("Cgame HUD unavailable: {reason}"))),
        }
    }

    /// Available orders or rejection.
    fn orders(&self) -> Result<Shared<dyn ConsoleOrders>, HudError> {
        match self.host.borrow().team_orders() {
            ConsoleOrdersAccess::Available(orders) => Ok(orders),
            ConsoleOrdersAccess::Unavailable { reason } => {
                Err(HudError::new(format!("Cgame team orders unavailable: {reason}")))
            }
        }
    }

    /// Scores down (`scoresDown`).
    fn scores_down(&self) {
        if self.state.borrow().product == Product::Missionpack {
            self.host
                .borrow_mut()
                .server_commands()
                .borrow_mut()
                .build_spectator_string();
        }
        let (request_time, time) = {
            let state = self.state.borrow();
            (state.scores_request_time, state.time)
        };
        if request_time.wrapping_add(2000) < time {
            self.state.borrow_mut().scores_request_time = time;
            self.host.borrow_mut().send_client_command("score");
            if !self.state.borrow().show_scores {
                self.state.borrow_mut().show_scores = true;
                self.state.borrow_mut().num_scores = 0;
            }
        } else {
            self.state.borrow_mut().show_scores = true;
        }
    }

    /// Next order (`nextOrder`).
    fn next_order(&self) -> Result<(), HudError> {
        let snapshot = self.state.borrow().snap.clone();
        let Some(snapshot) = snapshot else {
            return Err(HudError::new("CG_NextOrder requires cg.snap"));
        };
        let client = self
            .static_state
            .borrow()
            .client_info
            .get(snapshot.player_state.client_num as usize)
            .cloned()
            .ok_or_else(|| {
                HudError::new(format!(
                    "Console source array index {} outside 64",
                    snapshot.player_state.client_num
                ))
            })?;
        let selected = self
            .host
            .borrow()
            .read_vm_cvar("cg_currentSelectedPlayer")
            .integer_value;
        let selected_client = self
            .state
            .borrow()
            .sorted_team_players
            .get(selected as usize)
            .copied()
            .ok_or_else(|| HudError::new(format!("Console source array index {selected} outside 8")))?;
        if !client.team_leader && selected_client != snapshot.player_state.client_num {
            return Ok(());
        }
        let current = self.static_state.borrow().current_order;
        if current < 7 {
            let mut next = current + 1;
            if next == 5 && !self.orders()?.borrow().other_team_has_flag() {
                next += 1;
            }
            if next == 6 && !self.orders()?.borrow().your_team_has_flag() {
                next += 1;
            }
            self.static_state.borrow_mut().current_order = next;
        } else {
            self.static_state.borrow_mut().current_order = 1;
        }
        self.static_state.borrow_mut().order_pending = true;
        let time = self.state.borrow().time;
        self.static_state.borrow_mut().order_time = time.wrapping_add(3000);
        Ok(())
    }

    /// Send a team task (`task`).
    fn task(&self, voice: &str, task: i32) {
        self.host
            .borrow_mut()
            .send_console_command(&format!("cmd vsay_team {voice}\n"));
        self.host.borrow_mut().send_client_command(&game_format_bounded(
            "teamtask %d\n",
            &[GameFormatArgument::from(task)],
            1024,
        ));
    }

    /// Dispatch a command (`dispatch`).
    fn dispatch(&self, name: &str, argv: &[String]) -> Result<(), HudError> {
        let game_type = self.static_state.borrow().game_type;
        match name {
            "testgun" => {
                let model = if argv.len() < 2 {
                    None
                } else {
                    Some(console_argument(argv, 1, 1024))
                };
                let param = if argv.len() == 3 {
                    Some(f64::from(
                        game_atof(&console_argument(argv, 2, 1024)).unwrap_or_default(),
                    ))
                } else {
                    None
                };
                self.host.borrow_mut().view().borrow_mut().test_gun(model, param);
            }
            "testmodel" => {
                let model = if argv.len() < 2 {
                    None
                } else {
                    Some(console_argument(argv, 1, 1024))
                };
                let param = if argv.len() == 3 {
                    Some(f64::from(
                        game_atof(&console_argument(argv, 2, 1024)).unwrap_or_default(),
                    ))
                } else {
                    None
                };
                self.host.borrow_mut().view().borrow_mut().test_model(model, param);
            }
            "nextframe" => self.host.borrow_mut().view().borrow_mut().next_model_frame(),
            "prevframe" => self.host.borrow_mut().view().borrow_mut().previous_model_frame(),
            "nextskin" => self.host.borrow_mut().view().borrow_mut().next_model_skin(),
            "prevskin" => self.host.borrow_mut().view().borrow_mut().previous_model_skin(),
            "+zoom" => self.host.borrow_mut().view().borrow_mut().zoom_down(),
            "-zoom" => self.host.borrow_mut().view().borrow_mut().zoom_up(),
            "weapnext" => self.host.borrow_mut().weapons().borrow_mut().next_weapon(),
            "weapprev" => self.host.borrow_mut().weapons().borrow_mut().previous_weapon(),
            "weapon" => {
                let weapon = game_atoi(&console_argument(argv, 1, 1024)).unwrap_or_default();
                self.host.borrow_mut().weapons().borrow_mut().select_weapon(weapon);
            }
            "viewpos" => {
                let state = self.state.borrow();
                let text = game_format_bounded(
                    "(%i %i %i) : %i\n",
                    &[
                        GameFormatArgument::from(qvm_float_to_int(state.refdef.view_origin.x)),
                        GameFormatArgument::from(qvm_float_to_int(state.refdef.view_origin.y)),
                        GameFormatArgument::from(qvm_float_to_int(state.refdef.view_origin.z)),
                        GameFormatArgument::from(qvm_float_to_int(state.refdef_view_angles.y)),
                    ],
                    1024,
                );
                drop(state);
                self.host.borrow_mut().print(&text);
            }
            "sizeup" | "sizedown" => {
                let current = self.host.borrow().read_vm_cvar("cg_viewsize").integer_value;
                let next = current.wrapping_add(if name == "sizeup" { 10 } else { -10 });
                self.set(
                    "cg_viewsize",
                    &game_format_bounded("%i", &[GameFormatArgument::from(next)], 1024),
                )?;
            }
            "+scores" => self.scores_down(),
            "-scores" => {
                if self.state.borrow().show_scores {
                    self.state.borrow_mut().show_scores = false;
                    let time = self.state.borrow().time;
                    self.state.borrow_mut().score_fade_time = time;
                }
            }
            "tcmd" => {
                let target = self.crosshair_player();
                if target != 0 {
                    let text = game_format_bounded(
                        "gc %i %i",
                        &[
                            GameFormatArgument::from(target),
                            GameFormatArgument::from(game_atoi(&console_argument(argv, 1, 4)).unwrap_or_default()),
                        ],
                        1024,
                    );
                    self.host.borrow_mut().send_console_command(&text);
                }
            }
            "tell_target" | "tell_attacker" | "vtell_target" | "vtell_attacker" => {
                let target = if name.ends_with("target") {
                    self.crosshair_player()
                } else {
                    self.last_attacker()?
                };
                if target == -1 {
                    return Ok(());
                }
                let args = argv[1..].join(" ");
                if args.chars().count() >= 1024 {
                    return Err(HudError::new("Cmd_Args exceeds MAX_STRING_CHARS"));
                }
                let command = if name.starts_with('v') { "vtell" } else { "tell" };
                let text = game_format_bounded(
                    "%s %i %s",
                    &[
                        GameFormatArgument::from(command.to_string()),
                        GameFormatArgument::from(target),
                        GameFormatArgument::from(args.chars().take(127).collect::<String>()),
                    ],
                    128,
                );
                self.host.borrow_mut().send_client_command(&text);
            }
            "loaddeferred" => {
                let host = self.host.clone();
                self.host
                    .borrow_mut()
                    .clients()
                    .borrow_mut()
                    .load_deferred_players(&mut |entity| {
                        host.borrow_mut().reset_player_entity(entity);
                    });
            }
            "startorbit" => {
                if game_atoi(&self.immediate("developer")?).unwrap_or_default() == 0 {
                    return Ok(());
                }
                if self.host.borrow().read_vm_cvar("cg_cameraOrbit").numeric_value != 0.0 {
                    self.set("cg_cameraOrbit", "0")?;
                    self.set("cg_thirdPerson", "0")?;
                } else {
                    self.set("cg_cameraOrbit", "5")?;
                    self.set("cg_thirdPerson", "1")?;
                    self.set("cg_thirdPersonAngle", "0")?;
                    self.set("cg_thirdPersonRange", "100")?;
                }
            }
            "loadhud" => {
                let hud = self.hud()?;
                hud.borrow_mut().reset_strings();
                hud.borrow_mut().reset_menus();
                let path = self.immediate("cg_hudFiles")?;
                hud.borrow_mut()
                    .load_menus(if path.is_empty() { "ui/hud.txt" } else { &path });
                self.open();
                hud.borrow_mut().clear_scoreboard();
            }
            "scoresdown" | "scoresup" => {
                let hud = self.hud()?;
                let menu = hud.borrow().menu_scoreboard();
                if let Some(menu) = menu {
                    if self.state.borrow().score_board_showing {
                        for feeder in [11, 5, 6] {
                            hud.borrow_mut().scroll_feeder(&menu, feeder, name == "scoresdown");
                        }
                    }
                }
            }
            "nextteammember" => self.orders()?.borrow_mut().select_next_player(),
            "prevteammember" => self.orders()?.borrow_mut().select_previous_player(),
            "nextorder" => self.next_order()?,
            "confirmorder" | "denyorder" => {
                let yes = name == "confirmorder";
                let cgs = self.static_state.borrow();
                let text = game_format_bounded(
                    "cmd vtell %d %s\n",
                    &[
                        GameFormatArgument::from(cgs.accept_leader),
                        GameFormatArgument::from(if yes { "yes".to_string() } else { "no".to_string() }),
                    ],
                    1024,
                );
                drop(cgs);
                self.host.borrow_mut().send_console_command(&text);
                self.host.borrow_mut().send_console_command(if yes {
                    "+button5; wait; -button5"
                } else {
                    "+button6; wait; -button6"
                });
                let (time, accept_time, accept_task) = {
                    let cgs = self.static_state.borrow();
                    (self.state.borrow().time, cgs.accept_order_time, cgs.accept_task)
                };
                if time < accept_time {
                    if yes {
                        self.host.borrow_mut().send_client_command(&game_format_bounded(
                            "teamtask %d\n",
                            &[GameFormatArgument::from(accept_task)],
                            1024,
                        ));
                    }
                    self.static_state.borrow_mut().accept_order_time = 0;
                }
            }
            "taskoffense" => self.task(
                if game_type == GameType::GtCtf || game_type == GameType::Gt1fctf {
                    "ongetflag"
                } else {
                    "onoffense"
                },
                1,
            ),
            "taskdefense" => self.task("ondefense", 2),
            "taskpatrol" => self.task("onpatrol", 3),
            "taskcamp" => self.task("oncamp", 7),
            "taskfollow" => self.task("onfollow", 4),
            "taskretrieve" => self.task("onreturnflag", 5),
            "taskescort" => self.task("onfollowcarrier", 6),
            "taskownflag" => self.host.borrow_mut().send_console_command("cmd vsay_team ihaveflag\n"),
            "tasksuicide" => {
                let target = self.crosshair_player();
                if target != -1 {
                    self.host.borrow_mut().send_client_command(&game_format_bounded(
                        "tell %i suicide",
                        &[GameFormatArgument::from(target)],
                        128,
                    ));
                }
            }
            "tauntkillinsult" => self.host.borrow_mut().send_console_command("cmd vsay kill_insult\n"),
            "tauntpraise" => self.host.borrow_mut().send_console_command("cmd vsay praise\n"),
            "taunttaunt" => self.host.borrow_mut().send_console_command("cmd vtaunt\n"),
            "tauntdeathinsult" => self.host.borrow_mut().send_console_command("cmd vsay death_insult\n"),
            "tauntgauntlet" => self.host.borrow_mut().send_console_command("cmd vsay kill_guantlet\n"),
            "spwin" | "splose" => {
                let win = name == "spwin";
                self.set("cg_cameraOrbit", "2")?;
                self.set("cg_cameraOrbitDelay", "35")?;
                self.set("cg_thirdPerson", "1")?;
                self.set("cg_thirdPersonAngle", "0")?;
                self.set("cg_thirdPersonRange", "100")?;
                let sound = self
                    .host
                    .borrow()
                    .sound(if win { MenuEndSound::Winner } else { MenuEndSound::Loser });
                self.host.borrow_mut().add_buffered_sound(sound);
                self.host
                    .borrow_mut()
                    .center_print(if win { "YOU WIN!" } else { "YOU LOSE..." }, 144, 0);
            }
            _ => {
                return Err(HudError::new(format!(
                    "Registered cgame console command has no handler: {name}"
                )))
            }
        }
        Ok(())
    }
}

/// View runtime (`ViewRuntime`, used surface).
pub trait ViewService {
    /// Canonical frame state.
    fn state_handle(&self) -> Shared<ClientGameState>;
    /// Test gun.
    fn test_gun(&mut self, model: Option<String>, param: Option<f64>);
    /// Test model.
    fn test_model(&mut self, model: Option<String>, param: Option<f64>);
    /// Next model frame.
    fn next_model_frame(&mut self);
    /// Previous model frame.
    fn previous_model_frame(&mut self);
    /// Next model skin.
    fn next_model_skin(&mut self);
    /// Previous model skin.
    fn previous_model_skin(&mut self);
    /// Zoom down.
    fn zoom_down(&mut self);
    /// Zoom up.
    fn zoom_up(&mut self);
    /// Clear the test model.
    fn clear_test_model(&mut self);
}

/// Server command runtime (`ClientServerCommandRuntime`, used surface).
pub trait ServerCommandService {
    /// Build the spectator string.
    fn build_spectator_string(&mut self);
}

/// Client command sender (`sendClientCommand` / `sendConsoleCommand` / `addCommand` / `print`).
pub trait HudCommands {
    /// Send a client command.
    fn send_client_command(&mut self, text: &str);
    /// Send a console command.
    fn send_console_command(&mut self, text: &str);
    /// Register a command name.
    fn add_command(&mut self, name: &str);
    /// Print.
    fn print(&mut self, text: &str);
}

/// Menu end sound name (`"winnerSound" | "loserSound"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEndSound {
    /// Winner.
    Winner,
    /// Loser.
    Loser,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::presentation::audio::pcm_sound;
    use crate::q3::presentation::config::HudCvarReader;
    use crate::q3::presentation::hud::tests::*;
    use crate::q3::presentation::hud::{shared, Shared};
    use qa_core::cmd::Dialect;

    /// Canned console host.
    struct FakeConsoleHost {
        /// Cvars.
        cvars: Shared<CvarRegistry>,
        /// View.
        view: Shared<FakeView>,
        /// Weapons.
        weapons: Shared<FakeWeapons>,
        /// Store.
        store: Shared<FakeStore>,
        /// Server.
        server: Shared<FakeServerCommands>,
        /// Commands.
        commands: Shared<FakeCommands>,
        /// Reading cvars.
        reader: Shared<FakeCvars>,
        /// Buffered.
        buffered: Vec<Option<PcmSound>>,
        /// Center prints.
        center: Vec<String>,
    }

    impl ClientConsoleHost for FakeConsoleHost {
        fn cvars(&self) -> Shared<CvarRegistry> {
            self.cvars.clone()
        }
        fn view(&self) -> Shared<dyn ViewService> {
            self.view.clone()
        }
        fn weapons(&self) -> Shared<dyn WeaponService> {
            self.weapons.clone()
        }
        fn clients(&self) -> Shared<dyn ClientInfoStore> {
            self.store.clone()
        }
        fn server_commands(&self) -> Shared<dyn ServerCommandService> {
            self.server.clone()
        }
        fn hud(&self) -> ConsoleHudAccess {
            ConsoleHudAccess::Unavailable {
                reason: "test".to_string(),
            }
        }
        fn team_orders(&self) -> ConsoleOrdersAccess {
            ConsoleOrdersAccess::Unavailable {
                reason: "test".to_string(),
            }
        }
        fn read_vm_cvar(&self, name: &str) -> CvarSnapshot {
            self.reader.borrow().read_vm_cvar(name)
        }
        fn reset_player_entity(&mut self, _entity: &mut ClientEntity) {}
        fn add_command(&mut self, name: &str) {
            self.commands.borrow_mut().add_command(name);
        }
        fn send_client_command(&mut self, text: &str) {
            self.commands.borrow_mut().send_client_command(text);
        }
        fn send_console_command(&mut self, text: &str) {
            self.commands.borrow_mut().send_console_command(text);
        }
        fn print(&mut self, text: &str) {
            self.commands.borrow_mut().print(text);
        }
        fn center_print(&mut self, text: &str, _y: i32, _char_width: i32) {
            self.center.push(text.to_string());
        }
        fn sound(&self, _name: MenuEndSound) -> Option<PcmSound> {
            Some(pcm_sound("winner"))
        }
        fn add_buffered_sound(&mut self, sound: Option<PcmSound>) {
            self.buffered.push(sound);
        }
    }

    /// Build a console runtime.
    fn console_fixture() -> (
        ClientConsoleRuntime,
        Shared<FakeCommands>,
        Shared<FakeWeapons>,
        Shared<FakeView>,
    ) {
        let game = world(Product::Baseq3);
        let view: Shared<FakeView> = shared(FakeView {
            state: game.state.clone(),
            calls: Vec::new(),
        });
        let weapons: Shared<FakeWeapons> = shared(FakeWeapons {
            state: game.state.clone(),
            registry: game.registry.clone(),
            selected: Vec::new(),
        });
        let host: Shared<dyn ClientConsoleHost> = shared(FakeConsoleHost {
            cvars: shared(CvarRegistry::new(Dialect::Q3)),
            view: view.clone(),
            weapons: weapons.clone(),
            store: game.store,
            server: shared(FakeServerCommands::default()),
            commands: game.commands.clone(),
            reader: game.cvars,
            buffered: Vec::new(),
            center: Vec::new(),
        });
        let runtime = ClientConsoleRuntime::new(game.state, game.static_state, host);
        runtime.initialize_commands();
        (runtime, game.commands, weapons, view)
    }

    #[test]
    fn console_command_tables() {
        assert_eq!(local_command_names(Product::Baseq3).len(), 23);
        assert_eq!(local_command_names(Product::Missionpack).len(), 47);
        assert_eq!(client_console_command_names(Product::Baseq3).len(), 50);
        assert_eq!(client_console_command_names(Product::Missionpack).len(), 74);
        assert!(COMMON_COMMANDS.contains(&"tcmd"));
        assert!(MISSION_COMMANDS.contains(&"loadhud"));
        assert!(FORWARDED_COMMANDS.contains(&"teamtask"));
    }
    #[test]
    fn console_execute_paths() {
        let (runtime, commands, weapons, view) = console_fixture();
        assert_eq!(commands.borrow().added.len(), 50);
        assert!(runtime.handles("viewpos"));
        assert!(!runtime.handles("kill"));
        assert!(runtime.execute(&["viewpos".to_string()]).unwrap());
        assert_eq!(commands.borrow().printed.len(), 1);
        assert!(!runtime.execute(&["kill".to_string()]).unwrap());
        assert!(runtime.execute(&["weapon".to_string(), "2".to_string()]).unwrap());
        assert_eq!(weapons.borrow().selected, vec![2]);
        assert!(runtime.execute(&["nextframe".to_string()]).unwrap());
        assert!(view.borrow().calls.contains(&"nextframe".to_string()));
        let big = vec!["x".to_string(); 1025];
        assert!(runtime.execute(&big).is_err());
    }
    #[test]
    fn console_scores_and_tcmd() {
        let (runtime, commands, _, _) = console_fixture();
        runtime.state.borrow_mut().time = 5000;
        assert!(runtime.execute(&["+scores".to_string()]).unwrap());
        assert!(runtime.state.borrow().show_scores);
        assert_eq!(commands.borrow().client, vec!["score".to_string()]);
        assert!(runtime.execute(&["-scores".to_string()]).unwrap());
        assert!(!runtime.state.borrow().show_scores);
        runtime.state.borrow_mut().crosshair_client_num = 3;
        runtime.state.borrow_mut().crosshair_client_time = 5000;
        assert!(runtime.execute(&["tcmd".to_string(), "2".to_string()]).unwrap());
        assert!(commands.borrow().console.iter().any(|line| line.starts_with("gc 3 2")));
    }
}
