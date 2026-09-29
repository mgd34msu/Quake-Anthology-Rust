//! Game AI context from `src/bots/behavior/q3/ai-context.ts`
//! (`game/ai_main.c`, `ai_dmq3.c` globals).
//!
//! Per-game ownership for the source globals: entity observations,
//! bot states, deathmatch bookkeeping (flags, alternate routes,
//! waypoints), team/task caches, chat/command caches, node switches,
//! and the bot cvar cache. Games and the bot library pass through
//! method parameters so ownership stays explicit.

use std::collections::HashMap;

use qa_core::math::Vec3;

use crate::behavior::library::goals::BotGoal;
use crate::behavior::q3::ai_state::{BotStateStore, BotWaypoint};
use crate::behavior::q3::observations::BotEntityObservations;

/// Bot cvar cell: cached registry read with modification tracking.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BotCvar {
    /// Registered name.
    pub name: Option<String>,
    /// String value.
    pub value: String,
    /// Numeric value.
    pub numeric_value: f64,
    /// Integer value.
    pub integer_value: i32,
    /// Modification count.
    pub modification_count: i64,
}

impl BotCvar {
    /// Register the cvar with a default.
    pub fn register(&mut self, name: &str, default_value: &str) {
        self.name = Some(name.to_owned());
        self.value = default_value.to_owned();
        self.numeric_value = default_value.parse::<f64>().unwrap_or(0.0);
        self.integer_value = self.numeric_value as i32;
        self.modification_count = -1;
    }

    /// Overwrite the integer value (tests and game glue).
    pub fn write_integer(&mut self, value: i32) {
        self.integer_value = value;
    }
}

/// Alternate route goal.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AlternateRouteGoal {
    /// Origin.
    pub origin: Vec3,
    /// Area.
    pub area: i32,
    /// Travel time from start.
    pub start_travel_time: i32,
    /// Travel time to goal.
    pub goal_travel_time: i32,
    /// Extra travel time.
    pub extra_travel_time: i32,
}

/// Deathmatch AI globals (`ai_dmq3.c`).
#[derive(Debug, Clone)]
pub struct GameAiDeathmatchState {
    /// Game type.
    pub gametype: i32,
    /// Maximum clients.
    pub maxclients: i32,
    /// Last teleport time.
    pub last_teleport_time: f32,
    /// Maximum BSP model index.
    pub max_bsp_model_index: i32,
    /// Alternate routes set up.
    pub alternate_routes_setup: bool,
    /// CTF red flag.
    pub ctf_red_flag: BotGoal,
    /// CTF blue flag.
    pub ctf_blue_flag: BotGoal,
    /// CTF neutral flag.
    pub ctf_neutral_flag: BotGoal,
    /// Red obelisk.
    pub red_obelisk: BotGoal,
    /// Blue obelisk.
    pub blue_obelisk: BotGoal,
    /// Neutral obelisk.
    pub neutral_obelisk: BotGoal,
    /// Last teleport origin.
    pub last_teleport_origin: Vec3,
    /// Red alternate goals.
    pub red_alternate_goals: Vec<AlternateRouteGoal>,
    /// Blue alternate goals.
    pub blue_alternate_goals: Vec<AlternateRouteGoal>,
    /// Waypoint heap.
    pub waypoints: Vec<BotWaypoint>,
    /// Free waypoint head.
    pub free_waypoints: Option<usize>,
}

impl Default for GameAiDeathmatchState {
    fn default() -> Self {
        Self {
            gametype: 0,
            maxclients: 0,
            last_teleport_time: 0.0,
            max_bsp_model_index: 0,
            alternate_routes_setup: false,
            ctf_red_flag: BotGoal::default(),
            ctf_blue_flag: BotGoal::default(),
            ctf_neutral_flag: BotGoal::default(),
            red_obelisk: BotGoal::default(),
            blue_obelisk: BotGoal::default(),
            neutral_obelisk: BotGoal::default(),
            last_teleport_origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            red_alternate_goals: Vec::new(),
            blue_alternate_goals: Vec::new(),
            waypoints: (0..super::ai_definitions::MAX_WAYPOINTS)
                .map(|_| BotWaypoint::default())
                .collect(),
            free_waypoints: Some(0),
        }
    }
}

/// Team AI globals (`ai_team.c` caches).
#[derive(Debug, Clone, Default)]
pub struct GameAiTeamState {
    /// Teammate cache capacity.
    pub num_team_mates_max_clients: i32,
    /// Sorted teammate cache capacity.
    pub sort_team_mates_max_clients: i32,
    /// Team orders cache capacity.
    pub team_orders_max_clients: i32,
    /// Name lookup cache capacity.
    pub client_from_name_max_clients: i32,
    /// Same-team name lookup cache capacity.
    pub client_on_same_team_from_name_max_clients: i32,
    /// Task preferences by name.
    pub task_preferences: Vec<TaskPreference>,
}

/// Task preference entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPreference {
    /// Bot name.
    pub name: String,
    /// Preference.
    pub preference: i32,
}

/// Chat AI globals.
#[derive(Debug, Clone, Default)]
pub struct GameAiChatState {
    /// Per-name client cache.
    pub max_clients: HashMap<String, i32>,
}

/// Command AI globals.
#[derive(Debug, Clone, Default)]
pub struct GameAiCommandState {
    /// Not-leader flags per slot.
    pub not_leader: Vec<bool>,
    /// Per-name client cache.
    pub max_clients: HashMap<String, i32>,
}

/// Game AI context: per-game ownership for the source globals.
#[derive(Debug)]
pub struct GameAiContext {
    /// Entity observations.
    pub observations: BotEntityObservations,
    /// Bot states.
    pub states: BotStateStore,
    /// Deathmatch globals.
    pub deathmatch: GameAiDeathmatchState,
    /// Team globals.
    pub team: GameAiTeamState,
    /// Chat globals.
    pub chat: GameAiChatState,
    /// Command globals.
    pub command: GameAiCommandState,
    /// Node switches this frame.
    pub node_switches: Vec<String>,
    /// Bot cvar cache.
    pub cvars: HashMap<String, BotCvar>,
    /// AI time seconds.
    pub time: f32,
    /// Regular update time.
    pub regular_update_time: f32,
    /// Bot count.
    pub num_bots: i32,
    /// Interbreed flag.
    pub interbreed: bool,
    /// Interbreed match count.
    pub interbreed_match_count: i32,
}

impl GameAiContext {
    /// New context for `max_clients` slots.
    #[must_use]
    pub fn new(max_clients: i32) -> Self {
        let mut context = Self {
            observations: BotEntityObservations::default(),
            states: BotStateStore::new(),
            deathmatch: GameAiDeathmatchState::default(),
            team: GameAiTeamState::default(),
            chat: GameAiChatState::default(),
            command: GameAiCommandState::default(),
            node_switches: Vec::new(),
            cvars: HashMap::new(),
            time: 0.0,
            regular_update_time: 0.0,
            num_bots: 0,
            interbreed: false,
            interbreed_match_count: 0,
        };
        context.deathmatch.maxclients = max_clients;
        context.command.not_leader = vec![false; max_clients.max(0) as usize];
        context
    }

    /// Game type.
    #[must_use]
    pub fn game_type(&self) -> i32 {
        self.deathmatch.gametype
    }

    /// Maximum clients.
    #[must_use]
    pub fn max_clients(&self) -> i32 {
        self.deathmatch.maxclients
    }

    /// Fetch (or create) a bot cvar.
    pub fn cvar(&mut self, name: &str) -> &mut BotCvar {
        self.cvars.entry(name.to_owned()).or_default()
    }

    /// Register a bot cvar.
    pub fn register_cvar(&mut self, name: &str, default_value: &str) {
        self.cvar(name).register(name, default_value);
    }

    /// Record a node switch, capped at `MAX_NODESWITCHES`.
    pub fn track_node_switch(&mut self, node: &str) {
        if self.node_switches.len() < super::ai_definitions::MAX_NODESWITCHES {
            self.node_switches.push(node.to_owned());
        }
    }
}

impl Default for GameAiContext {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Reset a bot's decision state (`BotResetState`).
pub fn bot_reset_state(context: &mut GameAiContext, client: i32) {
    if let Some(state) = context.states.get_mut(client) {
        state.free_waypoints(state.checkpoints);
        state.checkpoints = None;
        state.free_waypoints(state.patrol_points);
        state.patrol_points = None;
        state.current_patrol_point = None;
        state.reset_decision_state();
    }
}
