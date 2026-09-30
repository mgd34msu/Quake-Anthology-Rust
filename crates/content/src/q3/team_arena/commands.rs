//! Quake III team-arena: commands.
//!
//! Donor provenance: `src/content/q3/team-arena/commands.ts`.

use qa_core::identity::ActorId;
use qa_core::math::vec3;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::mirrors::*;
use crate::q3::team_arena::r#match::*;
use crate::q3::team_arena::session::*;
use crate::q3::team_arena::team::*;

// ---------------------------------------------------------------------------
// commands.ts
// ---------------------------------------------------------------------------

/// Chat mode (`SayMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SayMode {
    /// Everyone.
    All,
    /// Team only.
    Team,
    /// Tell.
    Tell,
}

/// Arsenal category for selected-arsenal grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArsenalCategory {
    /// Weapons.
    Weapons,
    /// Ammo.
    Ammo,
}

/// Cheat toggle selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleCommand {
    /// God mode.
    God,
    /// Notarget.
    Notarget,
    /// Noclip.
    Noclip,
}

/// Command settings (`CommandSettings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSettings {
    /// Game type code.
    pub game_type: i32,
    /// Cheats enabled.
    pub cheats: bool,
    /// Force team balance.
    pub team_force_balance: bool,
    /// Maximum game clients.
    pub max_game_clients: i32,
    /// Dedicated server.
    pub dedicated: bool,
    /// Voting allowed.
    pub allow_vote: bool,
}

/// Engine imports (`GameCommandImports`).
pub trait CommandImports {
    /// Send a server command.
    fn send_server_command(&self, client_num: i32, text: &str);
    /// Write a config string.
    fn set_configstring(&self, index: i32, text: &str);
    /// Queue a console command.
    fn append_console_command(&self, text: &str);
    /// Read a cvar.
    fn get_cvar(&self, name: &str) -> String;
    /// Read raw userinfo.
    fn get_userinfo(&self, client_num: usize) -> String;
    /// Write raw userinfo.
    fn set_userinfo(&self, client_num: usize, text: &str);
    /// Log a line.
    fn log(&self, text: &str);
    /// Print a line.
    fn print(&self, text: &str);
}

/// Game-command services (`GameCommandHost`).
pub trait GameCommandHost {
    /// Grant a selected arsenal, if one is installed.
    fn grant_selected_arsenal(&self, actor: &ActorId, category: ArsenalCategory) -> bool {
        let _ = (actor, category);
        false
    }

    /// Grant a selected item, if one is installed.
    fn give_selected_item(&self, actor: &ActorId, args: &[String]) -> bool {
        let _ = (actor, args);
        false
    }

    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Match state.
    fn match_state(&self) -> &MatchStateRef;
    /// Shared team scores.
    fn team_scores(&self) -> SharedSlots;
    /// Command settings.
    fn settings(&self) -> CommandSettings;
    /// Engine imports.
    fn imports(&self) -> Rc<dyn CommandImports>;
    /// Location message for an entity.
    fn team_location_message(&self, entity: &EntityRef, capacity: usize) -> Option<String>;
    /// Death services.
    fn death(&self) -> Rc<dyn DeathHost>;
    /// Copy a corpse into the queue.
    fn copy_to_body_queue(&self, entity: &EntityRef) -> Option<EntityRef>;
    /// Begin a client.
    fn admission_begin(&self, client_num: usize);
    /// Refresh client userinfo.
    fn admission_userinfo_changed(&self, client_num: usize);
    /// Begin intermission.
    fn match_begin_intermission(&self);
    /// Set the team leader.
    fn match_set_leader(&self, team_code: i32, client_num: usize);
    /// Ensure a team leader.
    fn match_check_team_leader(&self, team_code: i32);
    /// Item services.
    fn items(&self) -> Rc<dyn ItemHost>;
    /// Teleport services.
    fn teleport(&self) -> Rc<dyn TeleportHost>;
}

pub(crate) const CMD_EF_VOTED: i32 = 0x4000;

pub(crate) const CMD_EF_TEAMVOTED: i32 = 0x80000;

pub(crate) const MAX_VOTE_COUNT: i32 = 3;

pub(crate) const MAX_STRING_CHARS: usize = 1024;

pub(crate) const MAX_SAY_TEXT: usize = 150;

pub(crate) const MOD_GAUNTLET: i32 = 2;

pub(crate) const MOD_SUICIDE: i32 = 20;

pub(crate) const ORDERS: &[&str] = &[
    "hold your position",
    "hold this position",
    "come here",
    "cover me",
    "guard location",
    "search and destroy",
    "report",
];

pub(crate) const GAME_NAMES: &[&str] = &[
    "Free For All",
    "Tournament",
    "Single Player",
    "Team Deathmatch",
    "Capture the Flag",
    "One Flag CTF",
    "Overload",
    "Harvester",
];

pub(crate) fn bounded(text: &str, capacity: usize) -> String {
    let end = text.find('\0').unwrap_or(text.len());
    let result: String = text[..end].chars().take(capacity - 1).collect();
    for ch in result.chars() {
        if ch as u32 > 255 {
            panic!("Game commands require byte strings");
        }
    }
    result
}

/// Command arguments (`Arguments`).
pub struct CommandArguments {
    /// Bounded argument values.
    pub values: Vec<String>,
}

impl CommandArguments {
    /// Fresh arguments.
    pub fn new(values: &[String]) -> Self {
        if values.len() > 1024 {
            panic!("Command exceeds MAX_STRING_TOKENS");
        }
        Self {
            values: values.iter().map(|value| bounded(value, MAX_STRING_CHARS)).collect(),
        }
    }

    /// Argument count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether there are no arguments.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Bounded argument read.
    #[must_use]
    pub fn at(&self, index: usize, capacity: usize) -> String {
        match self.values.get(index) {
            Some(value) => bounded(value, capacity),
            None => String::new(),
        }
    }

    /// Concatenate from an offset (`concat`).
    #[must_use]
    pub fn concat(&self, start: usize) -> String {
        let mut result = String::new();
        for index in start..self.values.len() {
            let value = self.at(index, MAX_STRING_CHARS);
            if result.len() + value.len() >= MAX_STRING_CHARS - 1 {
                break;
            }
            result += &value;
            if index != self.values.len() - 1 {
                result += " ";
            }
        }
        result
    }
}

/// Concatenate command arguments (`concatCommandArgs`).
#[must_use]
pub fn concat_command_args(argv: &[String], start: i32) -> String {
    if start < 0 {
        panic!("ConcatArgs start must be a nonnegative integer");
    }
    CommandArguments::new(argv).concat(start as usize)
}

pub(crate) fn cmd_lower(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_uppercase() {
                ch.to_ascii_lowercase()
            } else {
                ch
            }
        })
        .collect()
}

/// Recognize ESC pairs, not caret colors (`sanitize`).
pub(crate) fn sanitize(text: &str) -> String {
    let mut result = String::new();
    let input = bounded(text, MAX_STRING_CHARS);
    let bytes: Vec<u8> = input.chars().map(|ch| ch as u8).collect();
    let mut index = 0;
    while index < bytes.len() {
        let code = bytes[index];
        if code == 27 {
            if index + 1 == bytes.len() {
                panic!("SanitizeString ESC pair crosses the terminating NUL");
            }
            index += 2;
        } else {
            if (32..128).contains(&code) {
                let folded = if code.is_ascii_uppercase() { code + 32 } else { code };
                result.push(folded as char);
            }
            index += 1;
        }
    }
    result
}

pub(crate) fn clean_name(text: &str) -> String {
    let mut result = String::new();
    let input = bounded(text, 36);
    let bytes: Vec<u8> = input.chars().map(|ch| ch as u8).collect();
    let mut index = 0;
    while index < bytes.len() {
        let code = bytes[index];
        if bytes[index] == b'^' && index + 1 < bytes.len() && bytes[index + 1] != b'^' {
            index += 1;
        } else if (32..=126).contains(&code) {
            result.push(code as char);
        }
        index += 1;
    }
    cmd_lower(&result)
}

pub(crate) fn char_at(text: &str, index: usize) -> char {
    text.chars().nth(index).unwrap_or('\0')
}

pub(crate) fn command_client_of(entity: &EntityRef) -> ClientRef {
    match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("Player command requires a client entity"),
    }
}

/// Game-command runtime (`GameCommandRuntime`).
pub struct GameCommandRuntime {
    /// Host services.
    pub host: Rc<dyn GameCommandHost>,
}

impl GameCommandRuntime {
    /// Fresh runtime.
    pub fn new(host: Rc<dyn GameCommandHost>) -> Self {
        if host.team_scores().len() != 4 {
            panic!("Commands require the shared four-slot team score table");
        }
        Self { host }
    }

    fn print(&self, entity: &EntityRef, text: &str) {
        self.host.imports().send_server_command(
            entity.borrow().slot as i32,
            &game_format_default("print \"%s\"", &[FormatArg::Text(text.to_string())]),
        );
    }

    /// Send the scoreboard (`scoreboard`).
    pub fn scoreboard(&self, entity: &EntityRef) {
        let state = self.host.match_state();
        let record = state.borrow();
        let mut text = String::new();
        let mut count = 0;
        while count < record.num_connected_clients {
            let slot = match record.sorted_clients.get(count as usize) {
                Some(slot) => *slot,
                None => panic!("Scoreboard sorted-client prefix exceeds its backing storage"),
            };
            let client = self.host.pool().client_at(slot as usize);
            let info = client.borrow();
            let ping = if info.pers.connected == connection_state::CONNECTING {
                -1
            } else {
                info.ps.ping.min(999)
            };
            let accuracy = if info.accuracy_shots == 0 {
                0
            } else {
                info.accuracy_hits.wrapping_mul(100) / info.accuracy_shots
            };
            let perfect = if info.ps.persistant.get(persistent_index::RANK as usize) == 0
                && info.ps.persistant.get(persistent_index::KILLED as usize) == 0
            {
                1
            } else {
                0
            };
            let entry = game_format(
                " %i %i %i %i %i %i %i %i %i %i %i %i %i %i",
                &[
                    FormatArg::Int(slot),
                    FormatArg::Int(info.ps.persistant.get(persistent_index::SCORE as usize)),
                    FormatArg::Int(ping),
                    FormatArg::Int(record.time.wrapping_sub(info.pers.enter_time) / 60_000),
                    FormatArg::Int(0),
                    FormatArg::Int(self.host.pool().at(slot as usize).borrow().s.powerups),
                    FormatArg::Int(accuracy),
                    FormatArg::Int(info.ps.persistant.get(persistent_index::IMPRESSIVE_COUNT as usize)),
                    FormatArg::Int(info.ps.persistant.get(persistent_index::EXCELLENT_COUNT as usize)),
                    FormatArg::Int(info.ps.persistant.get(persistent_index::GAUNTLET_FRAG_COUNT as usize)),
                    FormatArg::Int(info.ps.persistant.get(persistent_index::DEFEND_COUNT as usize)),
                    FormatArg::Int(info.ps.persistant.get(persistent_index::ASSIST_COUNT as usize)),
                    FormatArg::Int(perfect),
                    FormatArg::Int(info.ps.persistant.get(persistent_index::CAPTURES as usize)),
                ],
                1024,
            );
            drop(info);
            if text.len() + entry.len() > 1024 {
                break;
            }
            text += &entry;
            count += 1;
        }
        drop(record);
        self.host.imports().send_server_command(
            entity.borrow().slot as i32,
            &game_format_default(
                "scores %i %i %i%s",
                &[
                    FormatArg::Int(count),
                    FormatArg::Int(self.host.team_scores().get(team::RED as usize)),
                    FormatArg::Int(self.host.team_scores().get(team::BLUE as usize)),
                    FormatArg::Text(text),
                ],
            ),
        );
    }

    /// Cheat gate (`cheatsOk`).
    pub fn cheats_ok(&self, entity: &EntityRef) -> bool {
        if !self.host.settings().cheats {
            self.print(entity, "Cheats are not enabled on this server.\n");
            return false;
        }
        if entity.borrow().health <= 0 {
            self.print(entity, "You must be alive to use this command.\n");
            return false;
        }
        true
    }

    /// Resolve a client reference (`clientNumberFromString`).
    pub fn client_number_from_string(&self, entity: &EntityRef, text: &str) -> Option<usize> {
        let input = bounded(text, MAX_STRING_CHARS);
        let first = char_at(&input, 0);
        if first.is_ascii_digit() {
            let slot = game_atoi(&input);
            if slot < 0 || slot as usize >= self.host.pool().max_clients() {
                self.print(
                    entity,
                    &game_format_default("Bad client slot: %i\n", &[FormatArg::Int(slot)]),
                );
                return None;
            }
            if self.host.pool().client_at(slot as usize).borrow().pers.connected != connection_state::CONNECTED {
                self.print(
                    entity,
                    &game_format_default("Client %i is not active\n", &[FormatArg::Int(slot)]),
                );
                return None;
            }
            return Some(slot as usize);
        }
        let name = sanitize(&input);
        for slot in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(slot);
            let record = client.borrow();
            if record.pers.connected == connection_state::CONNECTED && sanitize(&record.pers.netname) == name {
                return Some(slot);
            }
        }
        self.print(
            entity,
            &game_format_default("User %s is not on the server\n", &[FormatArg::Text(input)]),
        );
        None
    }

    fn give(&self, entity: &EntityRef, args: &CommandArguments) {
        if !self.cheats_ok(entity) {
            return;
        }
        let name = args.concat(1);
        let key = cmd_lower(&name);
        let all = key == "all";
        let client = command_client_of(entity);
        let product = client.borrow().ps.product;
        let schema = stat_schema(product);
        if all || key == "health" {
            let max = client.borrow().ps.stats.get(schema.max_health);
            entity.borrow_mut().health = max;
            if !all {
                return;
            }
        }
        if all || key == "weapons" {
            let actor = entity.borrow().actor.clone();
            if !self.host.grant_selected_arsenal(&actor, ArsenalCategory::Weapons) {
                client.borrow_mut().ps.stats.set(
                    schema.weapons,
                    (1i32 << weapon_count(product)) - 1 - (1 << weapon::GRAPPLING_HOOK) - (1 << weapon::NONE),
                );
            }
            if !all {
                return;
            }
        }
        if all || key == "ammo" {
            let actor = entity.borrow().actor.clone();
            if !self.host.grant_selected_arsenal(&actor, ArsenalCategory::Ammo) {
                for index in 0..16 {
                    client.borrow_mut().ps.ammo.set(index, 999);
                }
            }
            if !all {
                return;
            }
        }
        if all || key == "armor" {
            client.borrow_mut().ps.stats.set(schema.armor, 200);
            if !all {
                return;
            }
        }
        let award = if key == "excellent" {
            Some(persistent_index::EXCELLENT_COUNT)
        } else if key == "impressive" {
            Some(persistent_index::IMPRESSIVE_COUNT)
        } else if key == "gauntletaward" {
            Some(persistent_index::GAUNTLET_FRAG_COUNT)
        } else if key == "defend" {
            Some(persistent_index::DEFEND_COUNT)
        } else if key == "assist" {
            Some(persistent_index::ASSIST_COUNT)
        } else {
            None
        };
        if let Some(award) = award {
            let record = client.borrow_mut();
            let count = record.ps.persistant.get(award as usize);
            record.ps.persistant.set(award as usize, count + 1);
            return;
        }
        if all {
            return;
        }
        let item = match self.host.items().find_item(product, &name) {
            Some(item) => item,
            None => {
                let actor = entity.borrow().actor.clone();
                self.host
                    .give_selected_item(&actor, &args.values[1.min(args.values.len())..]);
                return;
            }
        };
        let temporary = self.host.pool().spawn();
        temporary.borrow_mut().s.origin = entity.borrow().r.current_origin();
        temporary.borrow_mut().set_classname(Some(item.class_name.clone()));
        let disabled = game_atoi(&self.host.imports().get_cvar(&format!("disable_{}", item.class_name))) != 0;
        self.host
            .items()
            .spawn_item(&temporary, &item, &SpawnVariables::new(vec![]), disabled);
        self.host.items().finish_spawning_item(&temporary);
        let other = DamageParticipant::Entity(entity.clone());
        self.host.items().touch_item(&temporary, &other, &TouchContact);
        if temporary.borrow().inuse {
            self.host.pool().free(&temporary);
        }
    }

    fn toggle(&self, entity: &EntityRef, command: ToggleCommand) {
        if !self.cheats_ok(entity) {
            return;
        }
        let client = command_client_of(entity);
        let enabled = match command {
            ToggleCommand::Noclip => {
                let mut record = client.borrow_mut();
                record.noclip = !record.noclip;
                record.noclip
            }
            ToggleCommand::God | ToggleCommand::Notarget => {
                let flag = if command == ToggleCommand::God {
                    game_flags::GODMODE
                } else {
                    game_flags::NOTARGET
                };
                entity.borrow_mut().flags ^= flag;
                entity.borrow().flags & flag != 0
            }
        };
        let label = match command {
            ToggleCommand::God => "godmode",
            ToggleCommand::Notarget => "notarget",
            ToggleCommand::Noclip => "noclip",
        };
        self.print(entity, &format!("{label} {}\n", if enabled { "ON" } else { "OFF" }));
    }

    fn kill(&self, entity: &EntityRef) {
        let client = command_client_of(entity);
        if client.borrow().sess.session_team == team::SPECTATOR || entity.borrow().health <= 0 {
            return;
        }
        entity.borrow_mut().flags &= !game_flags::GODMODE;
        entity.borrow_mut().health = -999;
        client.borrow_mut().ps.set_health(-999);
        let participant = DamageParticipant::Entity(entity.clone());
        self.host
            .death()
            .player_die(entity, Some(&participant), Some(&participant), 100_000, MOD_SUICIDE);
    }

    fn level_shot(&self, entity: &EntityRef) {
        if !self.cheats_ok(entity) {
            return;
        }
        if self.host.settings().game_type != game_type::FFA {
            self.print(entity, "Must be in g_gametype 0 for levelshot\n");
            return;
        }
        self.host.match_begin_intermission();
        self.host
            .imports()
            .send_server_command(entity.borrow().slot as i32, "clientLevelShot");
    }

    fn team_task(&self, entity: &EntityRef, args: &CommandArguments) {
        if args.len() != 2 {
            return;
        }
        let userinfo = bounded(
            &self.host.imports().get_userinfo(entity.borrow().slot),
            MAX_STRING_CHARS,
        );
        let task = game_format_default("%d", &[FormatArg::Int(game_atoi(&args.at(1, MAX_STRING_CHARS)))]);
        let updated = set_info_value(&userinfo, "teamtask", &task, 1024, &|text| {
            self.host.imports().print(text);
        });
        self.host.imports().set_userinfo(entity.borrow().slot, &updated);
        self.host.admission_userinfo_changed(entity.borrow().slot);
    }

    /// Announce a team change (`broadcastTeamChange`).
    pub fn broadcast_team_change(&self, client_num: usize, old_team: i32) {
        let client = self.host.pool().client_at(client_num);
        let team_code = client.borrow().sess.session_team;
        let netname = client.borrow().pers.netname.clone();
        let announcement = if team_code == team::RED {
            Some("joined the red team.")
        } else if team_code == team::BLUE {
            Some("joined the blue team.")
        } else if team_code == team::SPECTATOR && old_team != team::SPECTATOR {
            Some("joined the spectators.")
        } else if team_code == team::FREE {
            Some("joined the battle.")
        } else {
            None
        };
        if let Some(announcement) = announcement {
            self.host.imports().send_server_command(
                -1,
                &game_format_default(
                    "cp \"%s^7 %s\n\"",
                    &[FormatArg::Text(netname), FormatArg::Text(announcement.to_string())],
                ),
            );
        }
    }

    /// Move a client to a team (`setTeam`).
    pub fn set_team(&self, entity: &EntityRef, request: &str) {
        let client = command_client_of(entity);
        let key = cmd_lower(&bounded(request, MAX_STRING_CHARS));
        let settings = self.host.settings();
        let (team_code, spectator_state_code, spectator_client) = if key == "scoreboard" || key == "score" {
            (team::SPECTATOR, spectator_state::SCOREBOARD, 0)
        } else if key == "follow1" || key == "follow2" {
            (
                team::SPECTATOR,
                spectator_state::FOLLOW,
                if key == "follow1" { -1 } else { -2 },
            )
        } else if key == "spectator" || key == "s" {
            (team::SPECTATOR, spectator_state::FREE, 0)
        } else if settings.game_type >= game_type::TEAM {
            let team_code = if key == "red" || key == "r" {
                team::RED
            } else if key == "blue" || key == "b" {
                team::BLUE
            } else {
                pick_team(
                    self.host.pool().clients(),
                    self.host.pool().max_clients(),
                    &self.host.team_scores(),
                    entity.borrow().slot as i32,
                )
            };
            if settings.team_force_balance {
                let ignore = client.borrow().ps.client_num;
                let red = team_count(
                    self.host.pool().clients(),
                    self.host.pool().max_clients(),
                    ignore,
                    team::RED,
                );
                let blue = team_count(
                    self.host.pool().clients(),
                    self.host.pool().max_clients(),
                    ignore,
                    team::BLUE,
                );
                if team_code == team::RED && red - blue > 1 {
                    self.host
                        .imports()
                        .send_server_command(client.borrow().ps.client_num, "cp \"Red team has too many players.\n\"");
                    return;
                }
                if team_code == team::BLUE && blue - red > 1 {
                    self.host.imports().send_server_command(
                        client.borrow().ps.client_num,
                        "cp \"Blue team has too many players.\n\"",
                    );
                    return;
                }
            }
            (team_code, spectator_state::NOT, 0)
        } else {
            (team::FREE, spectator_state::NOT, 0)
        };
        let mut team_code = team_code;
        let non_spectators = self.host.match_state().borrow().num_non_spectator_clients;
        if (settings.game_type == game_type::TOURNAMENT && non_spectators >= 2)
            || (settings.max_game_clients > 0 && non_spectators >= settings.max_game_clients)
        {
            team_code = team::SPECTATOR;
        }
        let old_team = client.borrow().sess.session_team;
        if team_code == old_team && team_code != team::SPECTATOR {
            return;
        }
        if client.borrow().ps.health() <= 0 {
            self.host.copy_to_body_queue(entity);
        }
        client.borrow_mut().pers.team_state.state = team_state::BEGIN;
        if old_team != team::SPECTATOR {
            entity.borrow_mut().flags &= !game_flags::GODMODE;
            entity.borrow_mut().health = 0;
            client.borrow_mut().ps.set_health(0);
            let participant = DamageParticipant::Entity(entity.clone());
            self.host
                .death()
                .player_die(entity, Some(&participant), Some(&participant), 100_000, MOD_SUICIDE);
        }
        if team_code == team::SPECTATOR {
            client.borrow_mut().sess.spectator_time = self.host.match_state().borrow().time;
        }
        {
            let mut record = client.borrow_mut();
            record.sess.session_team = team_code;
            record.sess.spectator_state = spectator_state_code;
            record.sess.spectator_client = spectator_client;
            record.sess.team_leader = 0;
        }
        if team_code == team::RED || team_code == team::BLUE {
            let mut leader: Option<EntityRef> = None;
            for slot in 0..self.host.pool().max_clients() {
                let candidate = self.host.pool().client_at(slot);
                let record = candidate.borrow();
                if record.pers.connected != connection_state::DISCONNECTED
                    && record.sess.session_team == team_code
                    && record.sess.team_leader != 0
                {
                    leader = Some(self.host.pool().at(slot));
                    break;
                }
            }
            let replace = match &leader {
                None => true,
                Some(leader) => {
                    entity.borrow().r.sv_flags & server_entity_flags::BOT == 0
                        && leader.borrow().r.sv_flags & server_entity_flags::BOT != 0
                }
            };
            if replace {
                self.host.match_set_leader(team_code, entity.borrow().slot);
            }
        }
        if old_team == team::RED || old_team == team::BLUE {
            self.host.match_check_team_leader(old_team);
        }
        self.broadcast_team_change(entity.borrow().slot, old_team);
        self.host.admission_userinfo_changed(entity.borrow().slot);
        self.host.admission_begin(entity.borrow().slot);
    }

    /// Stop following (`stopFollowing`).
    pub fn stop_following(&self, entity: &EntityRef) {
        let client = command_client_of(entity);
        let mut record = client.borrow_mut();
        record
            .ps
            .persistant
            .set(persistent_index::TEAM as usize, team::SPECTATOR);
        record.sess.session_team = team::SPECTATOR;
        record.sess.spectator_state = spectator_state::FREE;
        record.ps.pm_flags &= !move_flags::FOLLOW;
        drop(record);
        entity.borrow_mut().r.sv_flags &= !server_entity_flags::BOT;
        client.borrow_mut().ps.client_num = entity.borrow().slot as i32;
    }

    fn team_command(&self, entity: &EntityRef, args: &CommandArguments) {
        let client = command_client_of(entity);
        if args.len() != 2 {
            let team_code = client.borrow().sess.session_team;
            let label = if team_code == team::RED {
                "Red"
            } else if team_code == team::BLUE {
                "Blue"
            } else if team_code == team::FREE {
                "Free"
            } else {
                "Spectator"
            };
            self.print(entity, &format!("{label} team\n"));
            return;
        }
        if client.borrow().switch_team_time > self.host.match_state().borrow().time {
            self.print(entity, "May not switch teams more than once per 5 seconds.\n");
            return;
        }
        if self.host.settings().game_type == game_type::TOURNAMENT && client.borrow().sess.session_team == team::FREE {
            let losses = client.borrow().sess.losses;
            client.borrow_mut().sess.losses = losses.wrapping_add(1);
        }
        self.set_team(entity, &args.at(1, MAX_STRING_CHARS));
        client.borrow_mut().switch_team_time = self.host.match_state().borrow().time.wrapping_add(5000);
    }

    fn follow(&self, entity: &EntityRef, args: &CommandArguments) {
        let client = command_client_of(entity);
        if args.len() != 2 {
            if client.borrow().sess.spectator_state == spectator_state::FOLLOW {
                self.stop_following(entity);
            }
            return;
        }
        let target = match self.client_number_from_string(entity, &args.at(1, MAX_STRING_CHARS)) {
            Some(target) => target,
            None => return,
        };
        if target == entity.borrow().slot
            || self.host.pool().client_at(target).borrow().sess.session_team == team::SPECTATOR
        {
            return;
        }
        if self.host.settings().game_type == game_type::TOURNAMENT && client.borrow().sess.session_team == team::FREE {
            let losses = client.borrow().sess.losses;
            client.borrow_mut().sess.losses = losses.wrapping_add(1);
        }
        if client.borrow().sess.session_team != team::SPECTATOR {
            self.set_team(entity, "spectator");
        }
        client.borrow_mut().sess.spectator_state = spectator_state::FOLLOW;
        client.borrow_mut().sess.spectator_client = target as i32;
    }

    /// Cycle the follow target (`followCycle`).
    pub fn follow_cycle(&self, entity: &EntityRef, direction: i32) {
        let client = command_client_of(entity);
        if self.host.settings().game_type == game_type::TOURNAMENT && client.borrow().sess.session_team == team::FREE {
            let losses = client.borrow().sess.losses;
            client.borrow_mut().sess.losses = losses.wrapping_add(1);
        }
        if client.borrow().sess.spectator_state == spectator_state::NOT {
            self.set_team(entity, "spectator");
        }
        let original = client.borrow().sess.spectator_client;
        let mut slot = original;
        for _ in 0..self.host.pool().max_clients() {
            slot += direction;
            if slot >= self.host.pool().max_clients() as i32 {
                slot = 0;
            }
            if slot < 0 {
                slot = self.host.pool().max_clients() as i32 - 1;
            }
            let target = self.host.pool().client_at(slot as usize);
            let record = target.borrow();
            if record.pers.connected == connection_state::CONNECTED && record.sess.session_team != team::SPECTATOR {
                drop(record);
                client.borrow_mut().sess.spectator_client = slot;
                client.borrow_mut().sess.spectator_state = spectator_state::FOLLOW;
                return;
            }
            if slot == original {
                return;
            }
        }
        panic!("Cmd_FollowCycle_f cannot terminate from an automatic-follow sentinel without an active player");
    }

    fn say_to(&self, entity: &EntityRef, target: &EntityRef, mode: SayMode, color: i32, name: &str, text: &str) {
        let body = target.borrow();
        let live = body.inuse;
        let client = body.client.clone();
        drop(body);
        let Some(client) = client else {
            return;
        };
        if !live || client.borrow().pers.connected != connection_state::CONNECTED {
            return;
        }
        if mode == SayMode::Team && !on_same_team(self.host.settings().game_type, entity, target) {
            return;
        }
        if self.host.settings().game_type == game_type::TOURNAMENT
            && client.borrow().sess.session_team == team::FREE
            && command_client_of(entity).borrow().sess.session_team != team::FREE
        {
            return;
        }
        self.host.imports().send_server_command(
            target.borrow().slot as i32,
            &game_format_default(
                "%s \"%s%c%c%s\"",
                &[
                    FormatArg::Text(if mode == SayMode::Team { "tchat" } else { "chat" }.to_string()),
                    FormatArg::Text(name.to_string()),
                    FormatArg::Int(94),
                    FormatArg::Int(color),
                    FormatArg::Text(text.to_string()),
                ],
            ),
        );
    }

    /// Chat dispatch (`say`).
    pub fn say(&self, entity: &EntityRef, target: Option<&EntityRef>, mode: SayMode, chat_text: &str) {
        let client = command_client_of(entity);
        let game_type = self.host.settings().game_type;
        let mut mode = mode;
        if game_type < game_type::TEAM && mode == SayMode::Team {
            mode = SayMode::All;
        }
        let netname = client.borrow().pers.netname.clone();
        let (name, color) = if mode == SayMode::All {
            self.host.imports().log(&game_format_default(
                "say: %s: %s\n",
                &[FormatArg::Text(netname.clone()), FormatArg::Text(chat_text.to_string())],
            ));
            (game_format("%s^7\x19: ", &[FormatArg::Text(netname)], 64), 50)
        } else if mode == SayMode::Team {
            self.host.imports().log(&game_format_default(
                "sayteam: %s: %s\n",
                &[FormatArg::Text(netname.clone()), FormatArg::Text(chat_text.to_string())],
            ));
            let location = self.host.team_location_message(entity, 64);
            let name = match location {
                None => game_format("\x19(%s^7\x19)\x19: ", &[FormatArg::Text(netname)], 64),
                Some(location) => game_format(
                    "\x19(%s^7\x19) (%s)\x19: ",
                    &[FormatArg::Text(netname), FormatArg::Text(location)],
                    64,
                ),
            };
            (name, 53)
        } else {
            let same_team = match target {
                Some(target) => {
                    game_type >= game_type::TEAM
                        && command_client_of(target).borrow().sess.session_team == client.borrow().sess.session_team
                }
                None => false,
            };
            let location = if same_team {
                self.host.team_location_message(entity, 64)
            } else {
                None
            };
            let name = match location {
                None => game_format("\x19[%s^7\x19]\x19: ", &[FormatArg::Text(netname)], 64),
                Some(location) => game_format(
                    "\x19[%s^7\x19] (%s)\x19: ",
                    &[FormatArg::Text(netname), FormatArg::Text(location)],
                    64,
                ),
            };
            (name, 54)
        };
        let text = bounded(chat_text, MAX_SAY_TEXT);
        if let Some(target) = target {
            self.say_to(entity, target, mode, color, &name, &text);
            return;
        }
        if self.host.settings().dedicated {
            self.host.imports().print(&game_format_default(
                "%s%s\n",
                &[FormatArg::Text(name.clone()), FormatArg::Text(text.clone())],
            ));
        }
        for slot in 0..self.host.pool().max_clients() {
            let target = self.host.pool().at(slot);
            self.say_to(entity, &target, mode, color, &name, &text);
        }
    }

    fn say_command(&self, entity: &EntityRef, args: &CommandArguments, mode: SayMode, arg0: bool) {
        if args.len() < 2 && !arg0 {
            return;
        }
        self.say(entity, None, mode, &args.concat(if arg0 { 0 } else { 1 }));
    }

    fn tell(&self, entity: &EntityRef, args: &CommandArguments) {
        if args.len() < 2 {
            return;
        }
        let slot = game_atoi(&args.at(1, MAX_STRING_CHARS));
        if slot < 0 || slot as usize >= self.host.pool().max_clients() {
            return;
        }
        let target = self.host.pool().at(slot as usize);
        if !target.borrow().inuse || target.borrow().client.is_none() {
            return;
        }
        let text = args.concat(2);
        let from = command_client_of(entity).borrow().pers.netname.clone();
        let to = command_client_of(&target).borrow().pers.netname.clone();
        self.host.imports().log(&game_format_default(
            "tell: %s to %s: %s\n",
            &[
                FormatArg::Text(from),
                FormatArg::Text(to),
                FormatArg::Text(text.clone()),
            ],
        ));
        self.say(entity, Some(&target), SayMode::Tell, &text);
        if !Rc::ptr_eq(entity, &target) && entity.borrow().r.sv_flags & server_entity_flags::BOT == 0 {
            self.say(entity, Some(entity), SayMode::Tell, &text);
        }
    }

    fn voice_to(&self, entity: &EntityRef, target: &EntityRef, mode: SayMode, id: &str, voice_only: bool) {
        if !target.borrow().inuse || target.borrow().client.is_none() {
            return;
        }
        if mode == SayMode::Team && !on_same_team(self.host.settings().game_type, entity, target) {
            return;
        }
        if self.host.settings().game_type == game_type::TOURNAMENT {
            return;
        }
        let color = if mode == SayMode::Team {
            53
        } else if mode == SayMode::Tell {
            54
        } else {
            50
        };
        let command = if mode == SayMode::Team {
            "vtchat"
        } else if mode == SayMode::Tell {
            "vtell"
        } else {
            "vchat"
        };
        self.host.imports().send_server_command(
            target.borrow().slot as i32,
            &game_format_default(
                "%s %d %d %d %s",
                &[
                    FormatArg::Text(command.to_string()),
                    FormatArg::Int(if voice_only { 1 } else { 0 }),
                    FormatArg::Int(entity.borrow().s.number),
                    FormatArg::Int(color),
                    FormatArg::Text(id.to_string()),
                ],
            ),
        );
    }

    /// Voice dispatch (`voice`).
    pub fn voice(&self, entity: &EntityRef, target: Option<&EntityRef>, mode: SayMode, id: &str, voice_only: bool) {
        let mut mode = mode;
        if self.host.settings().game_type < game_type::TEAM && mode == SayMode::Team {
            mode = SayMode::All;
        }
        if let Some(target) = target {
            self.voice_to(entity, target, mode, id, voice_only);
            return;
        }
        if self.host.settings().dedicated {
            let netname = command_client_of(entity).borrow().pers.netname.clone();
            self.host.imports().print(&game_format_default(
                "voice: %s %s\n",
                &[FormatArg::Text(netname), FormatArg::Text(id.to_string())],
            ));
        }
        for slot in 0..self.host.pool().max_clients() {
            let target = self.host.pool().at(slot);
            self.voice_to(entity, &target, mode, id, voice_only);
        }
    }

    fn voice_command(&self, entity: &EntityRef, args: &CommandArguments, mode: SayMode, voice_only: bool) {
        if args.len() < 2 {
            return;
        }
        self.voice(entity, None, mode, &args.concat(1), voice_only);
    }

    fn voice_tell(&self, entity: &EntityRef, args: &CommandArguments, voice_only: bool) {
        if args.len() < 2 {
            return;
        }
        let slot = game_atoi(&args.at(1, MAX_STRING_CHARS));
        if slot < 0 || slot as usize >= self.host.pool().max_clients() {
            return;
        }
        let target = self.host.pool().at(slot as usize);
        if !target.borrow().inuse || target.borrow().client.is_none() {
            return;
        }
        let id = args.concat(2);
        let from = command_client_of(entity).borrow().pers.netname.clone();
        let to = command_client_of(&target).borrow().pers.netname.clone();
        self.host.imports().log(&game_format_default(
            "vtell: %s to %s: %s\n",
            &[FormatArg::Text(from), FormatArg::Text(to), FormatArg::Text(id.clone())],
        ));
        self.voice(entity, Some(&target), SayMode::Tell, &id, voice_only);
        if !Rc::ptr_eq(entity, &target) && entity.borrow().r.sv_flags & server_entity_flags::BOT == 0 {
            self.voice(entity, Some(entity), SayMode::Tell, &id, voice_only);
        }
    }

    fn voice_taunt(&self, entity: &EntityRef) {
        let client = command_client_of(entity);
        let enemy = entity.borrow().enemy.clone();
        if let Some(enemy) = enemy {
            let number = entity.borrow().s.number;
            let insult = match enemy.borrow().client.clone() {
                Some(enemy_client) => enemy_client.borrow().last_killed_client == number,
                None => false,
            };
            if insult {
                self.taunt_pair(entity, &enemy, "death_insult");
                entity.borrow_mut().enemy = None;
                return;
            }
        }
        let last_killed = client.borrow().last_killed_client;
        if last_killed >= 0 && last_killed != entity.borrow().s.number {
            let target = self.host.pool().at(last_killed as usize);
            if target.borrow().client.is_some() {
                let gauntlet = command_client_of(&target).borrow().last_hurt_mod == MOD_GAUNTLET;
                self.taunt_pair(entity, &target, if gauntlet { "kill_gauntlet" } else { "kill_insult" });
                client.borrow_mut().last_killed_client = -1;
                return;
            }
        }
        if self.host.settings().game_type >= game_type::TEAM {
            for slot in 0..MAX_CLIENTS {
                let target = self.host.pool().at(slot);
                let candidate = target.borrow().client.clone();
                match candidate {
                    Some(candidate)
                        if !Rc::ptr_eq(&target, entity)
                            && candidate.borrow().sess.session_team == client.borrow().sess.session_team
                            && candidate.borrow().reward_time > self.host.match_state().borrow().time =>
                    {
                        self.taunt_pair(entity, &target, "praise");
                        return;
                    }
                    _ => {}
                }
            }
        }
        self.voice(entity, None, SayMode::All, "taunt", false);
    }

    fn taunt_pair(&self, entity: &EntityRef, target: &EntityRef, id: &str) {
        if target.borrow().r.sv_flags & server_entity_flags::BOT == 0 {
            self.voice(entity, Some(target), SayMode::Tell, id, false);
        }
        if entity.borrow().r.sv_flags & server_entity_flags::BOT == 0 {
            self.voice(entity, Some(entity), SayMode::Tell, id, false);
        }
    }

    fn game_command(&self, entity: &EntityRef, args: &CommandArguments) {
        let slot = game_atoi(&args.at(1, MAX_STRING_CHARS));
        let order = game_atoi(&args.at(2, MAX_STRING_CHARS));
        if slot < 0 || slot as usize >= MAX_CLIENTS || order < 0 || order > ORDERS.len() as i32 {
            return;
        }
        let text = match ORDERS.get(order as usize) {
            Some(text) => *text,
            None => panic!("Cmd_GameCommand_f order 7 reads beyond the source order table"),
        };
        let target = self.host.pool().at(slot as usize);
        self.say(entity, Some(&target), SayMode::Tell, text);
        self.say(entity, Some(entity), SayMode::Tell, text);
    }

    fn call_vote(&self, entity: &EntityRef, args: &CommandArguments) {
        let client = command_client_of(entity);
        if !self.host.settings().allow_vote {
            self.print(entity, "Voting not allowed here.\n");
            return;
        }
        if self.host.match_state().borrow().vote.time != 0 {
            self.print(entity, "A vote is already in progress.\n");
            return;
        }
        if client.borrow().pers.vote_count >= MAX_VOTE_COUNT {
            self.print(entity, "You have called the maximum number of votes.\n");
            return;
        }
        if client.borrow().sess.session_team == team::SPECTATOR {
            self.print(entity, "Not allowed to call a vote as spectator.\n");
            return;
        }
        let command = args.at(1, MAX_STRING_CHARS);
        let parameter = args.at(2, MAX_STRING_CHARS);
        let key = cmd_lower(&command);
        if command.contains(';') || parameter.contains(';') {
            self.print(entity, "Invalid vote string.\n");
            return;
        }
        if ![
            "map_restart",
            "nextmap",
            "map",
            "g_gametype",
            "kick",
            "clientkick",
            "g_dowarmup",
            "timelimit",
            "fraglimit",
        ]
        .contains(&key.as_str())
        {
            self.print(entity, "Invalid vote string.\n");
            self.print(
                entity,
                "Vote commands are: map_restart, nextmap, map <mapname>, g_gametype <n>, kick <player>, clientkick <clientnum>, g_doWarmup, timelimit <time>, fraglimit <frags>.\n",
            );
            return;
        }
        if self.host.match_state().borrow().vote.execute_time != 0 {
            self.host.match_state().borrow_mut().vote.execute_time = 0;
            let string = self.host.match_state().borrow().vote.string.clone();
            self.host
                .imports()
                .append_console_command(&game_format_default("%s\n", &[FormatArg::Text(string)]));
        }
        if key == "g_gametype" {
            let gametype = game_atoi(&parameter);
            if gametype == game_type::SINGLE_PLAYER || !(game_type::FFA..game_type::MAX_GAME_TYPE).contains(&gametype) {
                self.print(entity, "Invalid gametype.\n");
                return;
            }
            let name = match GAME_NAMES.get(gametype as usize) {
                Some(name) => *name,
                None => panic!("Vote gametype has no source display name"),
            };
            let mut state = self.host.match_state().borrow_mut();
            state.vote.string = game_format(
                "%s %d",
                &[FormatArg::Text(command.clone()), FormatArg::Int(gametype)],
                MAX_STRING_CHARS,
            );
            state.vote.display_string = game_format(
                "%s %s",
                &[FormatArg::Text(command), FormatArg::Text(name.to_string())],
                MAX_STRING_CHARS,
            );
        } else if key == "map" {
            let nextmap = bounded(&self.host.imports().get_cvar("nextmap"), MAX_STRING_CHARS);
            let mut state = self.host.match_state().borrow_mut();
            state.vote.string = if nextmap.is_empty() {
                game_format(
                    "%s %s",
                    &[FormatArg::Text(command), FormatArg::Text(parameter)],
                    MAX_STRING_CHARS,
                )
            } else {
                game_format(
                    "%s %s; set nextmap \"%s\"",
                    &[
                        FormatArg::Text(command),
                        FormatArg::Text(parameter),
                        FormatArg::Text(nextmap),
                    ],
                    MAX_STRING_CHARS,
                )
            };
            state.vote.display_string = state.vote.string.clone();
        } else if key == "nextmap" {
            if bounded(&self.host.imports().get_cvar("nextmap"), MAX_STRING_CHARS).is_empty() {
                self.print(entity, "nextmap not set.\n");
                return;
            }
            let mut state = self.host.match_state().borrow_mut();
            state.vote.string = "vstr nextmap".to_string();
            state.vote.display_string = state.vote.string.clone();
        } else {
            let mut state = self.host.match_state().borrow_mut();
            state.vote.string = game_format(
                "%s \"%s\"",
                &[FormatArg::Text(command), FormatArg::Text(parameter)],
                MAX_STRING_CHARS,
            );
            state.vote.display_string = state.vote.string.clone();
        }
        let netname = client.borrow().pers.netname.clone();
        self.host.imports().send_server_command(
            -1,
            &game_format_default("print \"%s called a vote.\n\"", &[FormatArg::Text(netname)]),
        );
        {
            let mut state = self.host.match_state().borrow_mut();
            state.vote.time = state.time;
            state.vote.yes = 1;
            state.vote.no = 0;
        }
        for slot in 0..self.host.pool().max_clients() {
            self.host.pool().client_at(slot).borrow_mut().ps.e_flags &= !CMD_EF_VOTED;
        }
        client.borrow_mut().ps.e_flags |= CMD_EF_VOTED;
        let time = self.host.match_state().borrow().vote.time;
        let display = self.host.match_state().borrow().vote.display_string.clone();
        self.host
            .imports()
            .set_configstring(8, &game_format_default("%i", &[FormatArg::Int(time)]));
        self.host.imports().set_configstring(9, &display);
        self.host
            .imports()
            .set_configstring(10, &game_format_default("%i", &[FormatArg::Int(1)]));
        self.host
            .imports()
            .set_configstring(11, &game_format_default("%i", &[FormatArg::Int(0)]));
    }

    fn vote(&self, entity: &EntityRef, args: &CommandArguments) {
        let client = command_client_of(entity);
        if self.host.match_state().borrow().vote.time == 0 {
            self.print(entity, "No vote in progress.\n");
            return;
        }
        if client.borrow().ps.e_flags & CMD_EF_VOTED != 0 {
            self.print(entity, "Vote already cast.\n");
            return;
        }
        if client.borrow().sess.session_team == team::SPECTATOR {
            self.print(entity, "Not allowed to vote as spectator.\n");
            return;
        }
        self.print(entity, "Vote cast.\n");
        client.borrow_mut().ps.e_flags |= CMD_EF_VOTED;
        let text = args.at(1, 64);
        // Both vote handlers test lowercase y at index zero and uppercase Y
        // and digit 1 at index one.
        if char_at(&text, 0) == 'y' || char_at(&text, 1) == 'Y' || char_at(&text, 1) == '1' {
            let mut state = self.host.match_state().borrow_mut();
            state.vote.yes = state.vote.yes.wrapping_add(1);
            let yes = state.vote.yes;
            drop(state);
            self.host
                .imports()
                .set_configstring(10, &game_format_default("%i", &[FormatArg::Int(yes)]));
        } else {
            let mut state = self.host.match_state().borrow_mut();
            state.vote.no = state.vote.no.wrapping_add(1);
            let no = state.vote.no;
            drop(state);
            self.host
                .imports()
                .set_configstring(11, &game_format_default("%i", &[FormatArg::Int(no)]));
        }
    }

    fn call_team_vote(&self, entity: &EntityRef, args: &CommandArguments) {
        let client = command_client_of(entity);
        let team_code = client.borrow().sess.session_team;
        let offset = if team_code == team::RED {
            Some(0)
        } else if team_code == team::BLUE {
            Some(1)
        } else {
            None
        };
        let Some(offset) = offset else {
            return;
        };
        if !self.host.settings().allow_vote {
            self.print(entity, "Voting not allowed here.\n");
            return;
        }
        if self.host.match_state().borrow().team_votes[offset].time != 0 {
            self.print(entity, "A team vote is already in progress.\n");
            return;
        }
        if client.borrow().pers.team_vote_count >= MAX_VOTE_COUNT {
            self.print(entity, "You have called the maximum number of team votes.\n");
            return;
        }
        let command = args.at(1, MAX_STRING_CHARS);
        let mut parameter = String::new();
        for index in 2..args.len() {
            if index > 2 {
                parameter += " ";
            }
            if parameter.len() >= MAX_STRING_CHARS {
                panic!("Team vote arguments overflow the source string buffer");
            }
            parameter += &args.at(index, MAX_STRING_CHARS - parameter.len());
        }
        if command.contains(';') || parameter.contains(';') {
            self.print(entity, "Invalid vote string.\n");
            return;
        }
        if cmd_lower(&command) != "leader" {
            self.print(entity, "Invalid vote string.\n");
            self.print(entity, "Team vote commands are: leader <player>.\n");
            return;
        }
        let mut target = client.borrow().ps.client_num;
        if !parameter.is_empty() {
            let mut digits = 0;
            while digits < 3 {
                let ch = char_at(&parameter, digits);
                if !ch.is_ascii_digit() {
                    break;
                }
                digits += 1;
            }
            if digits >= 3 || char_at(&parameter, digits) == '\0' {
                target = game_atoi(&parameter);
                if target < 0 || target as usize >= self.host.pool().max_clients() {
                    self.print(
                        entity,
                        &game_format_default("Bad client slot: %i\n", &[FormatArg::Int(target)]),
                    );
                    return;
                }
                if !self.host.pool().at(target as usize).borrow().inuse {
                    self.print(
                        entity,
                        &game_format_default("Client %i is not active\n", &[FormatArg::Int(target)]),
                    );
                    return;
                }
            } else {
                let name = clean_name(&parameter);
                let mut found = self.host.pool().max_clients() as i32;
                for slot in 0..self.host.pool().max_clients() {
                    let candidate = self.host.pool().client_at(slot);
                    let record = candidate.borrow();
                    if record.pers.connected != connection_state::DISCONNECTED
                        && record.sess.session_team == team_code
                        && clean_name(&record.pers.netname) == name
                    {
                        found = slot as i32;
                        break;
                    }
                }
                target = found;
                if target as usize >= self.host.pool().max_clients() {
                    self.print(
                        entity,
                        &game_format_default(
                            "%s is not a valid player on your team.\n",
                            &[FormatArg::Text(parameter)],
                        ),
                    );
                    return;
                }
            }
        }
        let netname = client.borrow().pers.netname.clone();
        self.host.match_state().borrow_mut().team_votes[offset].string = game_format(
            "%s %d",
            &[FormatArg::Text(command), FormatArg::Int(target)],
            MAX_STRING_CHARS,
        );
        for slot in 0..self.host.pool().max_clients() {
            let candidate = self.host.pool().client_at(slot);
            if candidate.borrow().pers.connected != connection_state::DISCONNECTED
                && candidate.borrow().sess.session_team == team_code
            {
                self.host.imports().send_server_command(
                    slot as i32,
                    &game_format_default(
                        "print \"%s called a team vote.\n\"",
                        &[FormatArg::Text(netname.clone())],
                    ),
                );
            }
        }
        {
            let mut state = self.host.match_state().borrow_mut();
            state.team_votes[offset].time = state.time;
            state.team_votes[offset].yes = 1;
            state.team_votes[offset].no = 0;
        }
        for slot in 0..self.host.pool().max_clients() {
            let candidate = self.host.pool().client_at(slot);
            if candidate.borrow().sess.session_team == team_code {
                candidate.borrow_mut().ps.e_flags &= !CMD_EF_TEAMVOTED;
            }
        }
        client.borrow_mut().ps.e_flags |= CMD_EF_TEAMVOTED;
        let (time, string) = {
            let state = self.host.match_state().borrow();
            (state.team_votes[offset].time, state.team_votes[offset].string.clone())
        };
        self.host
            .imports()
            .set_configstring(12 + offset as i32, &game_format_default("%i", &[FormatArg::Int(time)]));
        self.host.imports().set_configstring(14 + offset as i32, &string);
        self.host
            .imports()
            .set_configstring(16 + offset as i32, &game_format_default("%i", &[FormatArg::Int(1)]));
        self.host
            .imports()
            .set_configstring(18 + offset as i32, &game_format_default("%i", &[FormatArg::Int(0)]));
    }

    fn team_vote(&self, entity: &EntityRef, args: &CommandArguments) {
        let client = command_client_of(entity);
        let team_code = client.borrow().sess.session_team;
        let offset = if team_code == team::RED {
            Some(0)
        } else if team_code == team::BLUE {
            Some(1)
        } else {
            None
        };
        let Some(offset) = offset else {
            return;
        };
        if self.host.match_state().borrow().team_votes[offset].time == 0 {
            self.print(entity, "No team vote in progress.\n");
            return;
        }
        if client.borrow().ps.e_flags & CMD_EF_TEAMVOTED != 0 {
            self.print(entity, "Team vote already cast.\n");
            return;
        }
        self.print(entity, "Team vote cast.\n");
        client.borrow_mut().ps.e_flags |= CMD_EF_TEAMVOTED;
        let text = args.at(1, 64);
        if char_at(&text, 0) == 'y' || char_at(&text, 1) == 'Y' || char_at(&text, 1) == '1' {
            let mut state = self.host.match_state().borrow_mut();
            state.team_votes[offset].yes = state.team_votes[offset].yes.wrapping_add(1);
            let yes = state.team_votes[offset].yes;
            drop(state);
            self.host
                .imports()
                .set_configstring(16 + offset as i32, &game_format_default("%i", &[FormatArg::Int(yes)]));
        } else {
            let mut state = self.host.match_state().borrow_mut();
            state.team_votes[offset].no = state.team_votes[offset].no.wrapping_add(1);
            let no = state.team_votes[offset].no;
            drop(state);
            self.host
                .imports()
                .set_configstring(18 + offset as i32, &game_format_default("%i", &[FormatArg::Int(no)]));
        }
    }

    fn set_view_position(&self, entity: &EntityRef, args: &CommandArguments) {
        if !self.host.settings().cheats {
            self.print(entity, "Cheats are not enabled on this server.\n");
            return;
        }
        if args.len() != 5 {
            self.print(entity, "usage: setviewpos x y z yaw\n");
            return;
        }
        self.host.teleport().teleport_player(
            entity,
            vec3(
                game_atof(&args.at(1, MAX_STRING_CHARS)),
                game_atof(&args.at(2, MAX_STRING_CHARS)),
                game_atof(&args.at(3, MAX_STRING_CHARS)),
            ),
            vec3(0.0, game_atof(&args.at(4, MAX_STRING_CHARS)), 0.0),
        );
    }

    /// Dispatch a client command (`dispatch`).
    pub fn dispatch(&self, client_num: usize, argv: &[String]) {
        let entity = self.host.pool().at(client_num);
        if entity.borrow().client.is_none() {
            return;
        }
        let args = CommandArguments::new(argv);
        let command = args.at(0, MAX_STRING_CHARS);
        let key = cmd_lower(&command);
        match key.as_str() {
            "say" => {
                self.say_command(&entity, &args, SayMode::All, false);
                return;
            }
            "say_team" => {
                self.say_command(&entity, &args, SayMode::Team, false);
                return;
            }
            "tell" => {
                self.tell(&entity, &args);
                return;
            }
            "vsay" => {
                self.voice_command(&entity, &args, SayMode::All, false);
                return;
            }
            "vsay_team" => {
                self.voice_command(&entity, &args, SayMode::Team, false);
                return;
            }
            "vtell" => {
                self.voice_tell(&entity, &args, false);
                return;
            }
            "vosay" => {
                self.voice_command(&entity, &args, SayMode::All, true);
                return;
            }
            "vosay_team" => {
                self.voice_command(&entity, &args, SayMode::Team, true);
                return;
            }
            "votell" => {
                self.voice_tell(&entity, &args, true);
                return;
            }
            "vtaunt" => {
                self.voice_taunt(&entity);
                return;
            }
            "score" => {
                self.scoreboard(&entity);
                return;
            }
            _ => {}
        }
        if self.host.match_state().borrow().intermission_time != 0 {
            self.say_command(&entity, &args, SayMode::All, true);
            return;
        }
        match key.as_str() {
            "give" => self.give(&entity, &args),
            "god" => self.toggle(&entity, ToggleCommand::God),
            "notarget" => self.toggle(&entity, ToggleCommand::Notarget),
            "noclip" => self.toggle(&entity, ToggleCommand::Noclip),
            "kill" => self.kill(&entity),
            "teamtask" => self.team_task(&entity, &args),
            "levelshot" => self.level_shot(&entity),
            "follow" => self.follow(&entity, &args),
            "follownext" => self.follow_cycle(&entity, 1),
            "followprev" => self.follow_cycle(&entity, -1),
            "team" => self.team_command(&entity, &args),
            "where" => {
                let origin = entity.borrow().s.origin;
                self.print(
                    &entity,
                    &game_format_default("%s\n", &[FormatArg::Text(self.host.pool().vtos(origin))]),
                );
            }
            "callvote" => self.call_vote(&entity, &args),
            "vote" => self.vote(&entity, &args),
            "callteamvote" => self.call_team_vote(&entity, &args),
            "teamvote" => self.team_vote(&entity, &args),
            "gc" => self.game_command(&entity, &args),
            "setviewpos" => self.set_view_position(&entity, &args),
            // Cmd_Stats_f is an empty source body.
            "stats" => {}
            _ => self.print(
                &entity,
                &game_format_default("unknown cmd %s\n", &[FormatArg::Text(command)]),
            ),
        }
    }
}
