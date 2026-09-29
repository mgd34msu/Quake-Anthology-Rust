//! Bot catalog from `src/bots/behavior/q3/catalog.ts`
//! (`game/ai_main.c` spawn half: `G_LoadBots`, `G_GetBotInfoByNumber`,
//! `G_GetBotInfoByName`, `AddBot`, `BotConnect`, `BotCheckSpawn`,
//! `AddRandomBot`, `RemoveRandomBot`, `CheckMinimumPlayers`).
//!
//! Parses `bots.txt`/`arenas.txt` info files, connects named bots with
//! skill and team, and runs the delayed spawn queue plus minimum-player
//! maintenance.

use std::collections::{HashMap, VecDeque};

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::q3::ai_state::BotSettings;

/// Spawn queue depth.
pub const BOT_SPAWN_QUEUE_DEPTH: usize = 64;

/// Bot info file.
pub const BOTS_INFO_PATH: &str = "scripts/bots.txt";
/// Arena info file.
pub const ARENAS_INFO_PATH: &str = "scripts/arenas.txt";

/// Parse `key=value` info text into a map. Supports both `key=value`
/// lines and `{ key value }` blocks.
#[must_use]
pub fn parse_info(text: &str) -> HashMap<String, String> {
    let mut info = HashMap::new();
    let tokens = crate::behavior::library::structure::tokenize_bot_script(text);
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index] == "{" || tokens[index] == "}" {
            index += 1;
            continue;
        }
        if let Some(equals) = tokens[index].find('=') {
            let (key, value) = tokens[index].split_at(equals);
            info.insert(key.to_owned(), value[1..].to_owned());
            index += 1;
            continue;
        }
        if index + 1 < tokens.len() && tokens[index + 1] != "}" {
            info.insert(tokens[index].clone(), tokens[index + 1].clone());
            index += 2;
        } else {
            index += 1;
        }
    }
    info
}

/// Value for an info key.
#[must_use]
pub fn info_value(info: &HashMap<String, String>, key: &str) -> String {
    info.get(key).cloned().unwrap_or_default()
}

/// Queued spawn entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnQueueEntry {
    /// Client number.
    pub client: i32,
    /// Spawn time milliseconds.
    pub spawn_time: i32,
}

/// Bot catalog host: client allocation and AI setup hooks.
pub trait GameBotCatalogHost {
    /// Allocate a client slot.
    fn allocate_client(&mut self) -> Option<i32>;
    /// Set up a client brain.
    fn setup_client(&mut self, client: i32, settings: BotSettings, restart: bool) -> bool;
    /// Shut down a client brain.
    fn shutdown_client(&mut self, client: i32, restart: bool);
    /// Current time milliseconds.
    fn time_ms(&self) -> i32;
    /// Print engine text.
    fn print(&mut self, text: &str);
    /// Client name in use check.
    fn name_in_use(&self, name: &str) -> bool;
    /// Human/bot counts per team (team, humans, bots).
    fn team_counts(&self) -> Vec<(i32, i32, i32)>;
}

/// Game bot catalog.
pub struct GameBotCatalog {
    bots: Vec<HashMap<String, String>>,
    arenas: Vec<HashMap<String, String>>,
    queue: VecDeque<SpawnQueueEntry>,
    minimum_players: i32,
    check_minimum_time: i32,
}

impl GameBotCatalog {
    /// Empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bots: Vec::new(),
            arenas: Vec::new(),
            queue: VecDeque::with_capacity(BOT_SPAWN_QUEUE_DEPTH),
            minimum_players: 0,
            check_minimum_time: 0,
        }
    }

    /// Bot count.
    #[must_use]
    pub fn num_bots(&self) -> usize {
        self.bots.len()
    }

    /// Arena count.
    #[must_use]
    pub fn num_arenas(&self) -> usize {
        self.arenas.len()
    }

    /// Load bot and arena infos from prepared files.
    pub fn load(&mut self, files: &dyn BotSourceFiles) {
        self.bots = load_infos(files, BOTS_INFO_PATH);
        self.arenas = load_infos(files, ARENAS_INFO_PATH);
    }

    /// Bot info by number.
    #[must_use]
    pub fn bot_info_by_number(&self, number: usize) -> Option<&HashMap<String, String>> {
        self.bots.get(number)
    }

    /// Bot info by name.
    #[must_use]
    pub fn bot_info_by_name(&self, name: &str) -> Option<&HashMap<String, String>> {
        self.bots
            .iter()
            .find(|info| info_value(info, "name").eq_ignore_ascii_case(name))
    }

    /// Arena info by map name.
    #[must_use]
    pub fn arena_info_by_map(&self, map: &str) -> Option<&HashMap<String, String>> {
        self.arenas
            .iter()
            .find(|info| info_value(info, "map").eq_ignore_ascii_case(map))
    }

    /// Initialize bots from the `bots` cvar list at map start.
    pub fn initialize_bots(&mut self, host: &mut dyn GameBotCatalogHost, bot_list: &str, base_delay_ms: i32) {
        for name in bot_list.split_whitespace() {
            self.add_bot(host, name, 2.0, "", base_delay_ms, name);
        }
    }

    /// Add a bot by name.
    pub fn add_bot(
        &mut self,
        host: &mut dyn GameBotCatalogHost,
        name: &str,
        skill: f32,
        team: &str,
        delay_ms: i32,
        alternate_name: &str,
    ) -> bool {
        let info = self
            .bot_info_by_name(name)
            .cloned()
            .or_else(|| self.bot_info_by_name(alternate_name).cloned());
        let Some(info) = info else {
            host.print(&format!("bot {name} unknown"));
            return false;
        };
        let name = if host.name_in_use(&info_value(&info, "name")) {
            format!("{} II", info_value(&info, "name"))
        } else {
            info_value(&info, "name")
        };
        let Some(client) = host.allocate_client() else {
            host.print("no free client slot for bot");
            return false;
        };
        let characterfile = info_value(&info, "characterfile");
        let settings = BotSettings {
            characterfile: if characterfile.is_empty() {
                crate::behavior::library::character::DEFAULT_CHARACTER_PATH.to_owned()
            } else {
                characterfile
            },
            skill: skill.clamp(1.0, 5.0),
            team: if team.is_empty() {
                info_value(&info, "team")
            } else {
                team.to_owned()
            },
        };
        let _ = name;
        if !host.setup_client(client, settings, false) {
            return false;
        }
        self.add_to_spawn_queue(client, host.time_ms() + delay_ms);
        true
    }

    /// Connect a client (spawn-queue admission).
    pub fn connect(&mut self, host: &mut dyn GameBotCatalogHost, client: i32, first_time: bool) -> bool {
        let _ = (host, first_time);
        self.queue.iter().any(|entry| entry.client == client) || !first_time
    }

    /// Shut down a client.
    pub fn shutdown_client(&mut self, host: &mut dyn GameBotCatalogHost, client: i32, restart: bool) {
        host.shutdown_client(client, restart);
        self.remove_queued_begin(client);
    }

    /// Queue a delayed spawn.
    pub fn add_to_spawn_queue(&mut self, client: i32, spawn_time: i32) {
        if self.queue.len() >= BOT_SPAWN_QUEUE_DEPTH {
            self.queue.pop_front();
        }
        self.queue.push_back(SpawnQueueEntry { client, spawn_time });
    }

    /// Remove a queued spawn.
    pub fn remove_queued_begin(&mut self, client: i32) {
        self.queue.retain(|entry| entry.client != client);
    }

    /// Run due spawns. Returns begun clients.
    pub fn check_spawn(&mut self, host: &mut dyn GameBotCatalogHost) -> Vec<i32> {
        let now = host.time_ms();
        let mut begun = Vec::new();
        while let Some(entry) = self.queue.front() {
            if entry.spawn_time > now {
                break;
            }
            begun.push(entry.client);
            self.queue.pop_front();
        }
        begun
    }

    /// Handle `addbot`/`bot` console commands. Returns whether handled.
    pub fn console_command(&mut self, host: &mut dyn GameBotCatalogHost, argv: &[String]) -> bool {
        if argv
            .first()
            .is_some_and(|command| command.eq_ignore_ascii_case("addbot"))
        {
            let name = argv.get(1).cloned().unwrap_or_default();
            let skill = argv.get(2).and_then(|text| text.parse::<f32>().ok()).unwrap_or(2.0);
            let team = argv.get(3).cloned().unwrap_or_default();
            let delay = argv.get(4).and_then(|text| text.parse::<i32>().ok()).unwrap_or(0);
            let alt = argv.get(5).cloned().unwrap_or_else(|| name.clone());
            let name = if name.is_empty() { self.random_bot_name() } else { name };
            return self.add_bot(host, &name, skill, &team, delay, &alt);
        }
        if argv.first().is_some_and(|command| command.eq_ignore_ascii_case("bot")) {
            self.list_bots(host);
            return true;
        }
        false
    }

    fn list_bots(&self, host: &mut dyn GameBotCatalogHost) {
        for info in &self.bots {
            host.print(&info_value(info, "name"));
        }
    }

    fn random_bot_name(&self) -> String {
        self.bots
            .first()
            .map(|info| info_value(info, "name"))
            .unwrap_or_else(|| "bot".to_owned())
    }

    /// Add a random bot to a team.
    pub fn add_random_bot(&mut self, host: &mut dyn GameBotCatalogHost, team: i32) {
        let name = self.random_bot_name();
        let team_name = if team == 1 {
            "red"
        } else if team == 2 {
            "blue"
        } else {
            ""
        };
        self.add_bot(host, &name, 2.0, team_name, 0, &name);
    }

    /// Maintain minimum players.
    pub fn check_minimum_players(&mut self, host: &mut dyn GameBotCatalogHost, minimum: i32) {
        self.minimum_players = minimum;
        let now = host.time_ms();
        if now < self.check_minimum_time {
            return;
        }
        self.check_minimum_time = now + 10000;
        let total: i32 = host.team_counts().iter().map(|(_, humans, bots)| humans + bots).sum();
        if total < minimum {
            self.add_random_bot(host, 0);
        }
    }

    /// Count human players on a team.
    #[must_use]
    pub fn count_human_players(&self, host: &dyn GameBotCatalogHost, team: i32) -> i32 {
        count_team(host, team, true)
    }

    /// Count bot players on a team.
    #[must_use]
    pub fn count_bot_players(&self, host: &dyn GameBotCatalogHost, team: i32) -> i32 {
        count_team(host, team, false)
    }

    /// Queued spawn count.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}

impl Default for GameBotCatalog {
    fn default() -> Self {
        Self::new()
    }
}

fn count_team(host: &dyn GameBotCatalogHost, team: i32, humans: bool) -> i32 {
    host.team_counts()
        .iter()
        .filter(|(entry_team, _, _)| team < 0 || *entry_team == team)
        .map(|(_, human_count, bot_count)| if humans { *human_count } else { *bot_count })
        .sum()
}

fn load_infos(files: &dyn BotSourceFiles, path: &str) -> Vec<HashMap<String, String>> {
    let bytes = files.read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    // Split on blank lines; each chunk is one info record.
    text.split("\n\n")
        .map(str::trim)
        .filter(|chunk| !chunk.is_empty())
        .map(parse_info)
        .filter(|info| !info.is_empty())
        .collect()
}
