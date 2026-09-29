//! Source game adapter from `src/bots/behavior/q3/source-game.ts`.
//!
//! Reads detach source state; every mutation is an explicit call to
//! its existing owner. `ScriptedBotGame` is the synthetic game used
//! by behavior tests and headless hosts: scripted entities, picks,
//! and clock with in-memory engine services.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3};

use crate::behavior::library::genetic::{BotRandom, Xorshift32};
use crate::behavior::q3::ai_state::BotState;
use crate::behavior::q3::arsenal_knowledge::DefaultArsenalKnowledge;
use crate::behavior::q3::game_host::{
    BotArsenalKnowledge, BotGameClock, BotObservedEntity, BotObservedPickup, BotProduct, BotTraceQuery, BotTraceResult,
    SourceBotEngine, SourceBotGame,
};

/// In-memory engine services.
#[derive(Debug, Default)]
pub struct MemoryBotEngine {
    /// Printed lines.
    pub printed: Vec<String>,
    /// Userinfo by client.
    pub userinfo: HashMap<i32, String>,
    /// Server commands by client.
    pub server_commands: HashMap<i32, Vec<String>>,
    /// Dropped clients with reasons.
    pub dropped: Vec<(i32, String)>,
    /// Console commands.
    pub console_commands: Vec<String>,
}

impl SourceBotEngine for MemoryBotEngine {
    fn print(&mut self, text: &str) {
        self.printed.push(text.to_owned());
    }

    fn get_userinfo(&self, client: i32) -> String {
        self.userinfo.get(&client).cloned().unwrap_or_default()
    }

    fn set_userinfo(&mut self, client: i32, userinfo: &str) {
        self.userinfo.insert(client, userinfo.to_owned());
    }

    fn send_server_command(&mut self, client: i32, command: &str) {
        self.server_commands.entry(client).or_default().push(command.to_owned());
    }

    fn drop_client(&mut self, client: i32, reason: &str) {
        self.dropped.push((client, reason.to_owned()));
    }

    fn insert_console_command(&mut self, command: &str) {
        self.console_commands.insert(0, command.to_owned());
    }

    fn append_console_command(&mut self, command: &str) {
        self.console_commands.push(command.to_owned());
    }
}

/// Scripted game for tests and headless hosts.
pub struct ScriptedBotGame {
    /// Product.
    pub product: BotProduct,
    /// Game type.
    pub game_type: i32,
    /// Maximum clients.
    pub max_clients: i32,
    /// Entities.
    pub entities: Vec<BotObservedEntity>,
    /// Pickups.
    pub pickups: Vec<BotObservedPickup>,
    /// Clock.
    pub clock: BotGameClock,
    /// Engine.
    pub engine: MemoryBotEngine,
    /// Random state.
    pub random_state: Xorshift32,
    knowledge: DefaultArsenalKnowledge,
    teams: HashMap<i32, i32>,
    blocked_traces: Vec<(Vec3, Vec3)>,
}

impl ScriptedBotGame {
    /// New scripted game.
    #[must_use]
    pub fn new(product: BotProduct, game_type: i32, max_clients: i32) -> Self {
        Self {
            product,
            game_type,
            max_clients,
            entities: vec![BotObservedEntity::absent(); max_clients.max(0) as usize + 64],
            pickups: Vec::new(),
            clock: BotGameClock {
                time: 0,
                start_time: 0,
                intermission_time: 0,
            },
            engine: MemoryBotEngine::default(),
            random_state: Xorshift32::new(0x1234_5678),
            knowledge: DefaultArsenalKnowledge::new(),
            teams: HashMap::new(),
            blocked_traces: Vec::new(),
        }
    }

    /// Set an entity observation.
    pub fn set_entity(&mut self, number: i32, entity: BotObservedEntity) {
        let index = number as usize;
        if index >= self.entities.len() {
            self.entities.resize(index + 1, BotObservedEntity::absent());
        }
        self.entities[index] = entity;
    }

    /// Block traces crossing a segment (occluders).
    pub fn block_trace(&mut self, start: Vec3, end: Vec3) {
        self.blocked_traces.push((start, end));
    }

    /// Advance the clock.
    pub fn advance(&mut self, milliseconds: i32) {
        self.clock.time += milliseconds;
    }

    /// Entity count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether no entities exist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

impl SourceBotGame for ScriptedBotGame {
    fn product(&self) -> BotProduct {
        self.product
    }

    fn game_type(&self) -> i32 {
        self.game_type
    }

    fn max_clients(&self) -> i32 {
        self.max_clients
    }

    fn entity_count(&self) -> i32 {
        self.entities.len() as i32
    }

    fn clock(&self) -> BotGameClock {
        self.clock
    }

    fn entity(&self, number: i32) -> BotObservedEntity {
        self.entities
            .get(number as usize)
            .cloned()
            .unwrap_or_else(BotObservedEntity::absent)
    }

    fn model_index(&self, _name: &str) -> i32 {
        1
    }

    fn trace(&self, query: &BotTraceQuery) -> BotTraceResult {
        for (start, end) in &self.blocked_traces {
            if segments_cross(query.start, query.end, *start, *end) {
                return BotTraceResult {
                    fraction: 0.5,
                    entity_num: 1022,
                    end: query.start,
                };
            }
        }
        BotTraceResult {
            fraction: 1.0,
            entity_num: -1,
            end: query.end,
        }
    }

    fn point_contents(&self, _point: Vec3, _pass_entity: i32) -> i32 {
        0
    }

    fn random(&mut self) -> &mut dyn BotRandom {
        &mut self.random_state
    }

    fn choose_team(&mut self, client: i32) -> i32 {
        let team = self.teams.get(&client).copied().unwrap_or(1);
        self.teams.insert(client, team);
        team
    }

    fn activate_bot(&mut self, client: i32) {
        if let Some(entity) = self.entities.get_mut(client as usize) {
            entity.bot = true;
            entity.present = true;
        }
    }

    fn exit_level(&mut self) {
        self.clock.intermission_time = self.clock.time;
    }

    fn client_userinfo_changed(&mut self, _client: i32) {}

    fn client_connect(&mut self, _client: i32, _first_time: bool, _is_bot: bool) -> Option<String> {
        None
    }

    fn client_begin(&mut self, _client: i32) {}

    fn pickup_candidates(&self, _client: i32) -> Vec<BotObservedPickup> {
        self.pickups.clone()
    }

    fn knowledge(&mut self) -> &mut dyn BotArsenalKnowledge {
        &mut self.knowledge
    }
}

fn segments_cross(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> bool {
    // 2D segment intersection on the XY plane.
    let denominator = (b.x - a.x) * (d.y - c.y) - (b.y - a.y) * (d.x - c.x);
    if denominator.abs() < f32::EPSILON {
        return false;
    }
    let t = ((c.x - a.x) * (d.y - c.y) - (c.y - a.y) * (d.x - c.x)) / denominator;
    let u = ((c.x - a.x) * (b.y - a.y) - (c.y - a.y) * (b.x - a.x)) / denominator;
    t >= 0.0 && t <= 1.0 && u >= 0.0 && u <= 1.0
}

/// Null pickup bounds helper.
#[must_use]
pub fn point_bounds(point: Vec3, radius: f32) -> Bounds {
    Bounds {
        min: Vec3 {
            x: point.x - radius,
            y: point.y - radius,
            z: point.z - radius,
        },
        max: Vec3 {
            x: point.x + radius,
            y: point.y + radius,
            z: point.z + radius,
        },
    }
}

/// Update a bot inventory through knowledge (game glue helper).
pub fn update_knowledge_inventory(knowledge: &mut dyn BotArsenalKnowledge, state: &mut BotState) {
    knowledge.update_inventory(state);
}
