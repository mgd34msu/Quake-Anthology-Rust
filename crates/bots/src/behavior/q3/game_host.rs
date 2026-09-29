//! Source bot game host from `src/bots/behavior/q3/game-host.ts`.
//!
//! The game host is the observation boundary: the brain reads detached
//! entity/player snapshots and pickup observations, and every mutation
//! is an explicit call to the owning system. Arsenal knowledge (weapon
//! selection, pickup utility, tactics) is selected per game.

use qa_core::math::{Bounds, Vec3};

use crate::behavior::library::character::BotCharacterLibrary;
use crate::behavior::library::genetic::BotRandom;
use crate::behavior::q3::ai_state::BotState;

/// Bot product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotProduct {
    /// Base Quake III.
    BaseQ3,
    /// Team Arena.
    MissionPack,
}

/// Game type numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameType;

impl GameType {
    /// Free for all.
    pub const FFA: i32 = 0;
    /// Tournament.
    pub const TOURNAMENT: i32 = 1;
    /// Single player.
    pub const SINGLE_PLAYER: i32 = 2;
    /// Team deathmatch.
    pub const TEAM: i32 = 3;
    /// Capture the flag.
    pub const CTF: i32 = 4;
    /// One-flag CTF.
    pub const ONE_FLAG_CTF: i32 = 5;
    /// Obelisk.
    pub const OBELISK: i32 = 6;
    /// Harvester.
    pub const HARVESTER: i32 = 7;
}

/// Team numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Team;

impl Team {
    /// Free.
    pub const FREE: i32 = 0;
    /// Red.
    pub const RED: i32 = 1;
    /// Blue.
    pub const BLUE: i32 = 2;
    /// Spectator.
    pub const SPECTATOR: i32 = 3;
}

/// Detached player observation. The player-state cell belongs to the
/// brain, never to gameplay.
#[derive(Debug, Clone, PartialEq)]
pub struct BotObservedPlayer {
    /// Health.
    pub health: i32,
    /// Connected.
    pub connected: bool,
    /// Team.
    pub team: i32,
    /// Name.
    pub name: String,
    /// Last hurt client.
    pub last_hurt_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Current weapon.
    pub weapon: i32,
    /// Powerups.
    pub powerups: [i32; 16],
}

/// Detached entity observation.
#[derive(Debug, Clone, PartialEq)]
pub struct BotObservedEntity {
    /// Generation.
    pub generation: i32,
    /// Present.
    pub present: bool,
    /// Linked.
    pub linked: bool,
    /// Hidden (no-client).
    pub hidden: bool,
    /// Is a bot.
    pub bot: bool,
    /// Entity type.
    pub entity_type: i32,
    /// Model index.
    pub model_index: i32,
    /// Weapon.
    pub weapon: i32,
    /// Event.
    pub event: i32,
    /// Frame.
    pub frame: i32,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Contents.
    pub contents: i32,
    /// Inline model, when set.
    pub inline_model: Option<i32>,
    /// Classname.
    pub classname: Option<String>,
    /// Event time.
    pub event_time: i32,
    /// Proximity trigger.
    pub proximity_trigger: bool,
    /// Player observation, when a player.
    pub player: Option<BotObservedPlayer>,
}

impl BotObservedEntity {
    /// Absent entity.
    #[must_use]
    pub fn absent() -> Self {
        Self {
            generation: 0,
            present: false,
            linked: false,
            hidden: false,
            bot: false,
            entity_type: 0,
            model_index: 0,
            weapon: 0,
            event: 0,
            frame: 0,
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            bounds: Bounds {
                min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            },
            contents: 0,
            inline_model: None,
            classname: None,
            event_time: 0,
            proximity_trigger: false,
            player: None,
        }
    }
}

/// Pickup availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickupAvailability {
    /// Ready to take.
    Ready,
    /// Waiting respawn.
    Waiting,
    /// Disabled.
    Disabled,
}

/// Observed pickup.
#[derive(Debug, Clone, PartialEq)]
pub struct BotObservedPickup {
    /// Availability.
    pub availability: PickupAvailability,
    /// Eligible for this client.
    pub eligible: bool,
    /// Entity number.
    pub entity: i32,
    /// Origin.
    pub origin: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Item name.
    pub name: String,
    /// Item index.
    pub item_index: i32,
    /// Respawn delay seconds.
    pub respawn_delay: f32,
}

/// Pickup observations for one game.
pub trait BotPickupObservations {
    /// Whether the game owns an item goal for a client/entity pair.
    fn owns_item_goal(&self, _client: i32, _entity: i32) -> bool {
        false
    }

    /// Pickup candidates visible to a client.
    fn candidates(&self, client: i32) -> Vec<BotObservedPickup>;
}

/// Weapon tactics for one weapon number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotWeaponTactics {
    /// Melee weapon.
    pub melee: bool,
    /// Maximum range, or `None` for unlimited.
    pub maximum_range: Option<f32>,
    /// Aim accuracy override.
    pub aim_accuracy: Option<f32>,
    /// Aim skill override.
    pub aim_skill: Option<f32>,
    /// Weakness weight.
    pub weakness: f32,
    /// Predict occluded splash.
    pub predict_occluded_splash: bool,
}

impl Default for BotWeaponTactics {
    fn default() -> Self {
        Self {
            melee: false,
            maximum_range: None,
            aim_accuracy: None,
            aim_skill: None,
            weakness: 0.0,
            predict_occluded_splash: false,
        }
    }
}

/// Arsenal knowledge: the selected arsenal supplies inventory and
/// ballistics to the shared decision controller.
pub trait BotArsenalKnowledge {
    /// Fuzzy utility of a pickup preview for a bot.
    fn pickup_utility(&self, characters: &BotCharacterLibrary, state: &BotState, pickup: &BotObservedPickup) -> f32;
    /// Choose a weapon number for a bot.
    fn choose_weapon(&self, characters: &BotCharacterLibrary, state: &BotState) -> i32;
    /// Activation weapon for shootable goals.
    fn activation_weapon(&self, characters: &BotCharacterLibrary, state: &BotState) -> i32;
    /// Tactics for a weapon number.
    fn tactics(&self, weapon: i32) -> BotWeaponTactics;
    /// Aggression 0-1 for a bot.
    fn aggression(&self, state: &BotState) -> f32;
    /// Refresh the bot inventory from the game.
    fn update_inventory(&mut self, state: &mut BotState);
}

/// Engine services borrowed by bots.
pub trait SourceBotEngine {
    /// Print engine text.
    fn print(&mut self, text: &str);
    /// Get client userinfo.
    fn get_userinfo(&self, client: i32) -> String;
    /// Set client userinfo.
    fn set_userinfo(&mut self, client: i32, userinfo: &str);
    /// Send a server command to a client.
    fn send_server_command(&mut self, client: i32, command: &str);
    /// Drop a client.
    fn drop_client(&mut self, client: i32, reason: &str);
    /// Insert a console command.
    fn insert_console_command(&mut self, command: &str);
    /// Append a console command.
    fn append_console_command(&mut self, command: &str);
}

/// Shared game clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotGameClock {
    /// Current time milliseconds.
    pub time: i32,
    /// Start time milliseconds.
    pub start_time: i32,
    /// Intermission time milliseconds.
    pub intermission_time: i32,
}

/// Trace query for bot visibility checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotTraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Pass entity.
    pub pass_entity: i32,
    /// Contents mask.
    pub mask: i32,
    /// Box bounds, or `None` for a point trace.
    pub bounds: Option<Bounds>,
}

/// Trace result for bot visibility checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotTraceResult {
    /// Fraction reached.
    pub fraction: f32,
    /// Hit entity, or -1.
    pub entity_num: i32,
    /// End position.
    pub end: Vec3,
}

/// Source game observed by bots.
pub trait SourceBotGame {
    /// Product.
    fn product(&self) -> BotProduct;
    /// Game type.
    fn game_type(&self) -> i32;
    /// Maximum clients.
    fn max_clients(&self) -> i32;
    /// Entity count.
    fn entity_count(&self) -> i32;
    /// Game clock.
    fn clock(&self) -> BotGameClock;
    /// Observe an entity.
    fn entity(&self, number: i32) -> BotObservedEntity;
    /// Model index for a name.
    fn model_index(&self, name: &str) -> i32;
    /// Trace the world.
    fn trace(&self, query: &BotTraceQuery) -> BotTraceResult;
    /// Point contents.
    fn point_contents(&self, point: Vec3, pass_entity: i32) -> i32;
    /// Random draws.
    fn random(&mut self) -> &mut dyn BotRandom;
    /// Pick a team for a client.
    fn choose_team(&mut self, client: i32) -> i32;
    /// Activate a bot client.
    fn activate_bot(&mut self, client: i32);
    /// Exit the level.
    fn exit_level(&mut self);
    /// Notify userinfo changed.
    fn client_userinfo_changed(&mut self, client: i32);
    /// Connect a client; returns a deny reason or `None`.
    fn client_connect(&mut self, client: i32, first_time: bool, is_bot: bool) -> Option<String>;
    /// Begin a client.
    fn client_begin(&mut self, client: i32);
    /// Pickup candidates for a client.
    fn pickup_candidates(&self, client: i32) -> Vec<BotObservedPickup>;
    /// Arsenal knowledge.
    fn knowledge(&mut self) -> &mut dyn BotArsenalKnowledge;
}
