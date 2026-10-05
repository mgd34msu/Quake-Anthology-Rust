//! Native rerelease bot transport over the shared simulation.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/bot-rerelease.ts`.
//!
//! Sibling homes (narrow host seams over live siblings):
//! - [`SharedSimulation`](super::runtime::SharedSimulation)
//!   (`simulation/runtime.ts` port): [`RereleaseBotSimulation`].
//! - [`EngineSession`](qa_world::session::EngineSession)
//!   (`world/session/session.ts` port): [`RereleaseBotSession`].
//! - [`ApplicationBotNavigation`](super::navigation::ApplicationBotNavigation)
//!   (`simulation/navigation.ts` port): [`RereleaseBotNav`]. The ported
//!   `RereleaseNavigation` graph and the generic `NavigationRuntime`
//!   stay separate; the rerelease-used surface (checkpoint, restore,
//!   per-client graph) is seamed directly.
//!
//! Production behavior driver ([`ApplicationRereleaseBehavior`], built by
//! [`create_rerelease_behavior`]): port of the donor
//! `src/bots/behavior/rerelease/profile.ts` class over the live
//! `qa_bots::behavior::rerelease` pieces (`brain`, `rng`, `nav` path
//! vocabulary). Think runs the pre-think hook, `brain.think`, the
//! post-think hook, then the due pending chats (donor `think`,
//! profile.ts:69-78); the delegating methods, checkpoint, and restore
//! follow the donor class (profile.ts:79-94). The checkpoint image stays
//! seam-owned JSON in the donor `RereleaseBehaviorCheckpoint` shape
//! (profile.ts:44-51). Hosts may still supply custom behaviors through
//! [`RereleaseBehaviorFactory`].
//!
//! Like its donor, the factory returns the Q3 transport for non-rerelease
//! assets, except the two transports take different Rust options types, so
//! Q3 assets are rejected with the Q3 constructor named. `CommandSource::Bot`
//! carries no provider tag (see `bots.rs`); every command here is implicitly
//! `q1:bot` or `q2:bot` by asset source. The donor passes the movement
//! profile kind straight into `rereleaseBotCommand` as the dialect; the
//! ported [`MovementKind`] has no `q1-rerelease` member, so Q1 sources map to
//! `Q1Netquake`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_bots::behavior::assets::{BotAssetFiles, BotSourceFiles};
use qa_bots::behavior::rerelease::aim::BotAimStateT;
use qa_bots::behavior::rerelease::brain::{
    BotBrain, BotBrainCheckpoint, BotBrainConfig, BotBrainMemory, BotBrainMovementGeom, BotChatEventT,
    ExplicitGoalKind, ExplicitGoalOwner, ExplicitGoalT,
};
use qa_bots::behavior::rerelease::chat_text::q1_bot_chat_text;
use qa_bots::behavior::rerelease::data::botdata::CharacterEntry;
use qa_bots::behavior::rerelease::data::knowledge::{BotGameModeT, BotKnowledge};
use qa_bots::behavior::rerelease::nav::{
    NavEntityBounds, NavGraphLinkT, NavLinkType, NavPathT, NavTraversalT, RereleaseNavigation,
};
use qa_bots::behavior::rerelease::path_follow::BotPathStateT;
use qa_bots::behavior::rerelease::rng::{BotRandomT, Xorshift32};
use qa_bots::behavior::rerelease::senses::BotAwarenessT;
use qa_bots::behavior::rerelease::world::{BotSoundT, BotUsercmdT, BotWorldT};
use qa_bots::movement_contract::MovementKind;
use qa_bots::BotsError;
use qa_client::text::localization::LocalizationTable;
use qa_content::q2::foundation::host::Q2PresentationEvent;
use qa_core::identity::{ActorId, ClientId, OwnedActor, SavedActorId};
use qa_core::math::{Bounds, Vec3};
use qa_net::common::commands::{ActorCommand, ArsenalIntent, CommandSource};
use qa_net::q3_net::{ServerReliableCommands, MAX_RELIABLE_COMMANDS};
use qa_world::save::records::write_saved_actor;
use qa_world::save::value::{arr, boolean, int, num, obj, str, SaveJson};
use qa_world::session::SessionClient;
use qa_world::WorldError;

use super::bot_assets::{ApplicationBotAssets, RereleaseBotSource};
use super::bot_commands::rerelease_bot_command;
use super::bot_objectives::{rerelease_bot_objectives, BotObjectiveSimulation, RereleaseBotObjectivesImpl};
use super::bot_rerelease_world::{
    native_weapon_item, RereleaseBotNavigation, RereleaseBotObjectives, RereleaseBotWorld, RereleaseBotWorldHost,
    RereleaseBotWorldSimulation, RereleaseWorldOptions,
};
use super::bots::{
    restored_bot_reliable_commands, ApplicationBotClient, ApplicationBotError, ApplicationBotService,
    ApplicationBotsRestore, BotClientSnapshot, BotConnection, RereleaseGoalStatus,
};
use super::types::SimulationPresentationEvent;

/// Chat event delivered to the behavior chat callback (donor inline
/// `{ locstring, teamOnly }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseChatEvent {
    /// Localization key.
    pub locstring: String,
    /// Whether team-only.
    pub team_only: bool,
}

/// Behavior movement parameters (donor `movement` params).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RereleaseBehaviorMovement {
    /// Gravity.
    pub gravity: f64,
    /// Jump velocity.
    pub jump_velocity: f64,
    /// Jump air seconds.
    pub jump_air_seconds: f64,
    /// Maximum landing rise.
    pub maximum_landing_rise: f64,
    /// Start-above height.
    pub start_above: f64,
    /// Body mins.
    pub body_mins: Vec3,
    /// Body maxs.
    pub body_maxs: Vec3,
}

/// Behavior callbacks (donor `callbacks` params).
#[derive(Clone)]
pub struct RereleaseBehaviorCallbacks {
    /// Simulation time in seconds.
    pub time: Rc<dyn Fn() -> f64>,
    /// Pre-think hook.
    pub pre_think: Rc<dyn Fn()>,
    /// Post-think hook.
    pub post_think: Rc<dyn Fn()>,
    /// Chat hook. The donor reads `connection.behavior.random`
    /// reentrantly; the seam lends its RNG to the callback instead, which
    /// also makes the donor "lost its admitted client" throw unreachable.
    pub chat: RereleaseChatFn,
    /// Weapon selection hook.
    pub select_weapon: Rc<dyn Fn(i32)>,
    /// Weapon impulse hook.
    pub weapon_impulse: Rc<dyn Fn(i32) -> i32>,
    /// Nearby human teammate probe.
    pub human_teammate_near: Rc<dyn Fn() -> bool>,
}

/// Chat hook (donor `callbacks.chat`).
pub type RereleaseChatFn = Rc<dyn Fn(RereleaseChatEvent, &mut dyn BotRandomT)>;

/// Behavior construction parameters (donor `RereleaseBotBehavior` params).
pub struct RereleaseBehaviorParams {
    /// Asset provenance definition.
    pub definition: String,
    /// Source family.
    pub source: RereleaseBotSource,
    /// Parsed knowledge.
    pub knowledge: Rc<BotKnowledge>,
    /// Skill name.
    pub skill: String,
    /// Random seed.
    pub seed: i32,
    /// Game mode.
    pub game_mode: BotGameModeT,
    /// Character entry, when the name matches.
    pub character: Option<CharacterEntry>,
    /// Maximum health.
    pub max_health: f64,
    /// Run speed.
    pub run_speed: f64,
    /// Walk speed.
    pub walk_speed: f64,
    /// Movement parameters.
    pub movement: RereleaseBehaviorMovement,
    /// Callbacks.
    pub callbacks: RereleaseBehaviorCallbacks,
}

/// Rerelease behavior driver.
///
/// Port of `RereleaseBotBehavior` from donor
/// `src/bots/behavior/rerelease/profile.ts`; the production driver is
/// [`ApplicationRereleaseBehavior`]. The checkpoint image format is owned
/// by the seam: JSON in, JSON out.
pub trait RereleaseBotBehavior: std::fmt::Debug {
    /// Think one frame.
    fn think(&mut self, world: &mut dyn BotWorldT) -> BotUsercmdT;
    /// Set the game mode (donor `brain.setGameMode`).
    fn set_game_mode(&mut self, mode: BotGameModeT);
    /// Set the objective goal (donor `setObjectiveGoal`).
    fn set_objective_goal(&mut self, goal: Option<Vec3>);
    /// Current goal status.
    fn goal_status(&self) -> i32;
    /// Request a move-to-point goal.
    fn request_move_to_point(&mut self, point: Vec3);
    /// Request a follow-entity goal.
    fn request_follow_entity(&mut self, id: i32, origin: Vec3);
    /// Capture the checkpoint image (donor `checkpoint`).
    fn checkpoint_json(&self) -> SaveJson;
    /// Restore a checkpoint image (donor `restore`); the message propagates
    /// like the donor throw.
    fn restore_json(&mut self, image: &SaveJson) -> Result<(), String>;
}

/// Behavior factory (donor `new RereleaseBotBehavior`); the message
/// propagates like the donor throw.
pub type RereleaseBehaviorFactory =
    Rc<dyn Fn(RereleaseBehaviorParams) -> Result<Box<dyn RereleaseBotBehavior>, String>>;

/// Pending behavior chat (donor `pendingChats` entry, profile.ts:57).
#[derive(Debug, Clone)]
struct PendingBehaviorChat {
    /// Delivery time in seconds.
    time: f64,
    /// Brain chat event.
    event: BotChatEventT,
}

/// Production rerelease behavior driver (donor `RereleaseBotBehavior`,
/// profile.ts:53-95).
///
/// The brain owns the behavior's single random stream
/// (`BotBrain::new` seeds it; `rng_state`/`restore_rng` checkpoint it),
/// matching the donor constructor (profile.ts:58-68), which hands its
/// `Xorshift32` to the brain config. The chat callback lends that stream
/// by round-tripping the state word through a local generator, so chat
/// line selection advances the same stream the donor reads reentrantly.
pub struct ApplicationRereleaseBehavior {
    /// Asset provenance definition.
    definition: String,
    /// Source family.
    source: RereleaseBotSource,
    /// Behavior brain.
    brain: BotBrain,
    /// Scheduled chats shared with the brain `on_chat` sink.
    pending: Rc<RefCell<Vec<PendingBehaviorChat>>>,
    /// Behavior callbacks.
    callbacks: RereleaseBehaviorCallbacks,
}

impl std::fmt::Debug for ApplicationRereleaseBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplicationRereleaseBehavior")
            .field("definition", &self.definition)
            .field("source", &self.source)
            .field("pending", &self.pending.borrow().len())
            .finish_non_exhaustive()
    }
}

/// Build the production behavior driver (donor `new RereleaseBotBehavior`,
/// profile.ts:58-68); the message propagates like the donor throw.
pub fn create_rerelease_behavior(params: RereleaseBehaviorParams) -> Result<Box<dyn RereleaseBotBehavior>, String> {
    Ok(Box::new(ApplicationRereleaseBehavior::new(params)?))
}

/// Production behavior factory (donor `new RereleaseBotBehavior`).
///
/// Options that omit the factory build the production driver; tests
/// inject their own factories.
pub fn default_behavior_factory() -> RereleaseBehaviorFactory {
    Rc::new(create_rerelease_behavior)
}

impl ApplicationRereleaseBehavior {
    /// Build the driver (donor constructor).
    fn new(params: RereleaseBehaviorParams) -> Result<Self, String> {
        let RereleaseBehaviorParams {
            definition,
            source,
            knowledge,
            skill,
            seed,
            game_mode,
            character,
            max_health,
            run_speed,
            walk_speed,
            movement,
            callbacks,
        } = params;
        let pending: Rc<RefCell<Vec<PendingBehaviorChat>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&pending);
        let clock = Rc::clone(&callbacks.time);
        let teammate = Rc::clone(&callbacks.human_teammate_near);
        let select = Rc::clone(&callbacks.select_weapon);
        let impulse = Rc::clone(&callbacks.weapon_impulse);
        let weapon_impulse: Option<Box<dyn Fn(i32) -> i32>> = match source {
            RereleaseBotSource::Q1 => Some(Box::new(move |number: i32| impulse(number)) as Box<dyn Fn(i32) -> i32>),
            RereleaseBotSource::Q2 => None,
        };
        let on_weapon_select: Option<Box<dyn FnMut(i32)>> = match source {
            RereleaseBotSource::Q1 => None,
            RereleaseBotSource::Q2 => Some(Box::new(move |number: i32| select(number)) as Box<dyn FnMut(i32)>),
        };
        let config = BotBrainConfig {
            knowledge: (*knowledge).clone(),
            skill,
            game_mode,
            character,
            max_health: max_health as f32,
            run_speed: run_speed as f32,
            walk_speed: walk_speed as f32,
            movement: BotBrainMovementGeom {
                gravity: movement.gravity as f32,
                jump_velocity: movement.jump_velocity as f32,
                jump_air_seconds: movement.jump_air_seconds as f32,
                maximum_landing_rise: movement.maximum_landing_rise as f32,
                start_above: movement.start_above as f32,
                body_mins: movement.body_mins,
                body_maxs: movement.body_maxs,
            },
            on_chat: Some(Box::new(move |event: BotChatEventT| {
                sink.borrow_mut().push(PendingBehaviorChat {
                    time: clock() + f64::from(event.delay_ms) / 1000.0,
                    event,
                });
            }) as Box<dyn FnMut(BotChatEventT)>),
            weapon_impulse,
            on_weapon_select,
            human_teammate_near: Box::new(move || teammate()) as Box<dyn Fn() -> bool>,
        };
        let brain = BotBrain::new(config, seed).map_err(|error| error.to_string())?;
        Ok(ApplicationRereleaseBehavior {
            definition,
            source,
            brain,
            pending,
            callbacks,
        })
    }
}

impl RereleaseBotBehavior for ApplicationRereleaseBehavior {
    fn think(&mut self, world: &mut dyn BotWorldT) -> BotUsercmdT {
        (self.callbacks.pre_think)();
        let command = self.brain.think(world);
        (self.callbacks.post_think)();
        let now = (self.callbacks.time)();
        let ready: Vec<PendingBehaviorChat> = {
            let mut pending = self.pending.borrow_mut();
            let (ready, waiting): (Vec<_>, Vec<_>) = pending.drain(..).partition(|chat| chat.time <= now);
            *pending = waiting;
            ready
        };
        if !ready.is_empty() {
            // Lend the behavior stream to the chat callback: the state word
            // round-trips through a local generator (xorshift32 never holds
            // the rejected zero word from a seeded stream).
            let mut random = Xorshift32::new(0);
            random
                .restore(self.brain.rng_state())
                .expect("rerelease behavior RNG state round-trips");
            for chat in &ready {
                (self.callbacks.chat)(
                    RereleaseChatEvent {
                        locstring: chat.event.locstring.clone(),
                        team_only: chat.event.team_only,
                    },
                    &mut random,
                );
            }
            self.brain
                .restore_rng(random.peek())
                .expect("rerelease behavior RNG state round-trips");
        }
        command
    }

    fn set_game_mode(&mut self, mode: BotGameModeT) {
        self.brain.set_game_mode(mode);
    }

    fn set_objective_goal(&mut self, goal: Option<Vec3>) {
        self.brain.set_objective_goal(goal);
    }

    fn goal_status(&self) -> i32 {
        self.brain.goal_status()
    }

    fn request_move_to_point(&mut self, point: Vec3) {
        self.brain.request_move_to_point(point);
    }

    fn request_follow_entity(&mut self, id: i32, origin: Vec3) {
        self.brain.request_follow_entity(id, origin);
    }

    fn checkpoint_json(&self) -> SaveJson {
        obj(vec![
            ("version", int(1)),
            ("source", str(self.source.as_str())),
            ("definition", str(&self.definition)),
            ("rng", int(i64::from(self.brain.rng_state()))),
            ("brain", encode_brain_checkpoint(&self.brain.checkpoint())),
            (
                "pendingChats",
                arr(self.pending.borrow().iter().map(encode_pending_chat).collect()),
            ),
        ])
    }

    fn restore_json(&mut self, image: &SaveJson) -> Result<(), String> {
        let version = read_behavior_int(image, "version")?;
        let source = read_behavior_str(image, "source")?;
        let definition = read_behavior_str(image, "definition")?;
        if version != 1 || source != self.source.as_str() || definition != self.definition {
            return Err("Rerelease behavior checkpoint belongs to another mounted source definition".to_string());
        }
        let rng = read_behavior_i32(image, "rng")?;
        let brain = image.get("brain").ok_or_else(|| behavior_missing("brain"))?;
        let checkpoint = decode_brain_checkpoint(brain)?;
        let chats = image
            .get("pendingChats")
            .ok_or_else(|| behavior_missing("pendingChats"))?;
        let pending = decode_pending_chats(chats)?;
        self.brain.restore(&checkpoint).map_err(|error| error.to_string())?;
        self.brain.restore_rng(rng).map_err(|error| error.to_string())?;
        *self.pending.borrow_mut() = pending;
        Ok(())
    }
}

/// Missing behavior checkpoint member.
fn behavior_missing(key: &str) -> String {
    format!("rerelease behavior checkpoint lacks {key}")
}

/// Malformed behavior checkpoint member.
fn behavior_malformed(key: &str) -> String {
    format!("rerelease behavior checkpoint has malformed {key}")
}

/// Read a required string member.
fn read_behavior_str(value: &SaveJson, key: &str) -> Result<String, String> {
    match value.get(key) {
        Some(SaveJson::String(text)) => Ok(text.clone()),
        _ => Err(behavior_missing(key)),
    }
}

/// Read a required integer member.
fn read_behavior_int(value: &SaveJson, key: &str) -> Result<i64, String> {
    match value.get(key) {
        Some(SaveJson::Number(number)) if number.fract() == 0.0 => Ok(*number as i64),
        _ => Err(behavior_missing(key)),
    }
}

/// Read a required 32-bit integer member.
fn read_behavior_i32(value: &SaveJson, key: &str) -> Result<i32, String> {
    let number = read_behavior_int(value, key)?;
    i32::try_from(number).map_err(|_| behavior_malformed(key))
}

/// Read a required number member.
fn read_behavior_num(value: &SaveJson, key: &str) -> Result<f64, String> {
    match value.get(key) {
        Some(SaveJson::Number(number)) => Ok(*number),
        _ => Err(behavior_missing(key)),
    }
}

/// Read a required single-precision member.
fn read_behavior_f32(value: &SaveJson, key: &str) -> Result<f32, String> {
    read_behavior_num(value, key).map(|number| number as f32)
}

/// Read a required boolean member.
fn read_behavior_bool(value: &SaveJson, key: &str) -> Result<bool, String> {
    match value.get(key) {
        Some(SaveJson::Bool(flag)) => Ok(*flag),
        _ => Err(behavior_missing(key)),
    }
}

/// Read a required object member.
fn read_behavior_obj<'v>(value: &'v SaveJson, key: &str) -> Result<&'v SaveJson, String> {
    match value.get(key) {
        Some(object @ SaveJson::Object(_)) => Ok(object),
        _ => Err(behavior_missing(key)),
    }
}

/// Read a required array member.
fn read_behavior_arr<'v>(value: &'v SaveJson, key: &str) -> Result<&'v Vec<SaveJson>, String> {
    match value.get(key) {
        Some(SaveJson::Array(items)) => Ok(items),
        _ => Err(behavior_missing(key)),
    }
}

/// Read a required nullable member.
fn read_behavior_opt<T>(
    value: &SaveJson,
    key: &str,
    read: impl Fn(&SaveJson) -> Result<T, String>,
) -> Result<Option<T>, String> {
    match value.get(key) {
        None => Err(behavior_missing(key)),
        Some(SaveJson::Null) => Ok(None),
        Some(item) => read(item).map(Some),
    }
}

/// Encode a vector.
fn vec3_json(vector: Vec3) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(vector.x))),
        ("y", num(f64::from(vector.y))),
        ("z", num(f64::from(vector.z))),
    ])
}

/// Decode a vector.
fn vec3_from_json(value: &SaveJson) -> Result<Vec3, String> {
    Ok(Vec3 {
        x: read_behavior_f32(value, "x")?,
        y: read_behavior_f32(value, "y")?,
        z: read_behavior_f32(value, "z")?,
    })
}

/// Encode an optional value.
fn opt_json<T>(value: Option<&T>, encode: impl Fn(&T) -> SaveJson) -> SaveJson {
    value.map(encode).unwrap_or(SaveJson::Null)
}

/// Encode the game mode.
fn encode_game_mode(mode: &BotGameModeT) -> SaveJson {
    obj(vec![
        ("gameType", str(&mode.game_type)),
        ("weaponStay", boolean(mode.weapon_stay)),
        ("hasTeams", opt_json(mode.has_teams.as_ref(), |flag| boolean(*flag))),
        ("teamDamage", opt_json(mode.team_damage.as_ref(), |flag| boolean(*flag))),
    ])
}

/// Decode the game mode.
fn decode_game_mode(value: &SaveJson) -> Result<BotGameModeT, String> {
    Ok(BotGameModeT {
        game_type: read_behavior_str(value, "gameType")?,
        weapon_stay: read_behavior_bool(value, "weaponStay")?,
        has_teams: read_behavior_opt(value, "hasTeams", |item| match item {
            SaveJson::Bool(flag) => Ok(*flag),
            _ => Err(behavior_malformed("gameMode.hasTeams")),
        })?,
        team_damage: read_behavior_opt(value, "teamDamage", |item| match item {
            SaveJson::Bool(flag) => Ok(*flag),
            _ => Err(behavior_malformed("gameMode.teamDamage")),
        })?,
    })
}

/// Encode the aim state.
fn encode_aim(aim: &BotAimStateT) -> SaveJson {
    obj(vec![
        ("pitch", num(f64::from(aim.pitch))),
        ("yaw", num(f64::from(aim.yaw))),
        ("pitchVelocity", num(f64::from(aim.pitch_velocity))),
        ("yawVelocity", num(f64::from(aim.yaw_velocity))),
        ("modifierUntil", num(f64::from(aim.modifier_until))),
    ])
}

/// Decode the aim state.
fn decode_aim(value: &SaveJson) -> Result<BotAimStateT, String> {
    Ok(BotAimStateT {
        pitch: read_behavior_f32(value, "pitch")?,
        yaw: read_behavior_f32(value, "yaw")?,
        pitch_velocity: read_behavior_f32(value, "pitchVelocity")?,
        yaw_velocity: read_behavior_f32(value, "yawVelocity")?,
        modifier_until: read_behavior_f32(value, "modifierUntil")?,
    })
}

/// Encode a user command.
fn encode_usercmd(command: &BotUsercmdT) -> SaveJson {
    obj(vec![
        ("forwardmove", num(f64::from(command.forwardmove))),
        ("sidemove", num(f64::from(command.sidemove))),
        ("upmove", num(f64::from(command.upmove))),
        ("buttons", int(i64::from(command.buttons))),
        ("impulse", int(i64::from(command.impulse))),
        ("viewAngles", vec3_json(command.view_angles)),
    ])
}

/// Decode a user command.
fn decode_usercmd(value: &SaveJson) -> Result<BotUsercmdT, String> {
    Ok(BotUsercmdT {
        forwardmove: read_behavior_f32(value, "forwardmove")?,
        sidemove: read_behavior_f32(value, "sidemove")?,
        upmove: read_behavior_f32(value, "upmove")?,
        buttons: read_behavior_i32(value, "buttons")?,
        impulse: read_behavior_i32(value, "impulse")?,
        view_angles: vec3_from_json(read_behavior_obj(value, "viewAngles")?)?,
    })
}

/// Encode one awareness record.
fn encode_awareness(awareness: &BotAwarenessT) -> SaveJson {
    obj(vec![
        ("id", int(i64::from(awareness.id))),
        ("sight", num(f64::from(awareness.sight))),
        ("weapon", num(f64::from(awareness.weapon))),
        ("lastContact", num(f64::from(awareness.last_contact))),
        ("lastSeen", num(f64::from(awareness.last_seen))),
        ("lastHeard", num(f64::from(awareness.last_heard))),
        ("lastKnownOrigin", vec3_json(awareness.last_known_origin)),
    ])
}

/// Decode one awareness record.
fn decode_awareness(value: &SaveJson) -> Result<BotAwarenessT, String> {
    Ok(BotAwarenessT {
        id: read_behavior_i32(value, "id")?,
        sight: read_behavior_f32(value, "sight")?,
        weapon: read_behavior_f32(value, "weapon")?,
        last_contact: read_behavior_f32(value, "lastContact")?,
        last_seen: read_behavior_f32(value, "lastSeen")?,
        last_heard: read_behavior_f32(value, "lastHeard")?,
        last_known_origin: vec3_from_json(read_behavior_obj(value, "lastKnownOrigin")?)?,
    })
}

/// Encode a pending chat.
fn encode_pending_chat(chat: &PendingBehaviorChat) -> SaveJson {
    obj(vec![
        ("time", num(chat.time)),
        (
            "event",
            obj(vec![
                ("locstring", str(&chat.event.locstring)),
                ("chatType", str(&chat.event.chat_type)),
                ("delayMs", num(f64::from(chat.event.delay_ms))),
                ("teamOnly", boolean(chat.event.team_only)),
            ]),
        ),
    ])
}

/// Decode pending chats.
fn decode_pending_chats(value: &SaveJson) -> Result<Vec<PendingBehaviorChat>, String> {
    let SaveJson::Array(items) = value else {
        return Err(behavior_malformed("pendingChats"));
    };
    items
        .iter()
        .map(|item| {
            let event = read_behavior_obj(item, "event")?;
            Ok(PendingBehaviorChat {
                time: read_behavior_num(item, "time")?,
                event: BotChatEventT {
                    locstring: read_behavior_str(event, "locstring")?,
                    chat_type: read_behavior_str(event, "chatType")?,
                    delay_ms: read_behavior_f32(event, "delayMs")?,
                    team_only: read_behavior_bool(event, "teamOnly")?,
                },
            })
        })
        .collect()
}

/// Encode a traversal funnel.
fn encode_traversal(traversal: &NavTraversalT) -> SaveJson {
    obj(vec![
        ("funnel", vec3_json(traversal.funnel)),
        ("start", vec3_json(traversal.start)),
        ("end", vec3_json(traversal.end)),
    ])
}

/// Decode a traversal funnel.
fn decode_traversal(value: &SaveJson) -> Result<NavTraversalT, String> {
    Ok(NavTraversalT {
        funnel: vec3_from_json(read_behavior_obj(value, "funnel")?)?,
        start: vec3_from_json(read_behavior_obj(value, "start")?)?,
        end: vec3_from_json(read_behavior_obj(value, "end")?)?,
    })
}

/// Encode entity bounds.
fn encode_entity_bounds(bounds: &NavEntityBounds) -> SaveJson {
    obj(vec![("mins", vec3_json(bounds.mins)), ("maxs", vec3_json(bounds.maxs))])
}

/// Decode entity bounds.
fn decode_entity_bounds(value: &SaveJson) -> Result<NavEntityBounds, String> {
    Ok(NavEntityBounds {
        mins: vec3_from_json(read_behavior_obj(value, "mins")?)?,
        maxs: vec3_from_json(read_behavior_obj(value, "maxs")?)?,
    })
}

/// Encode one graph link.
fn encode_link(link: &NavGraphLinkT) -> SaveJson {
    obj(vec![
        ("from", int(i64::from(link.from))),
        ("to", int(i64::from(link.to))),
        ("linkType", int(i64::from(link.link_type as i32))),
        ("traversal", opt_json(link.traversal.as_ref(), encode_traversal)),
        (
            "entityBounds",
            opt_json(link.entity_bounds.as_ref(), encode_entity_bounds),
        ),
    ])
}

/// Decode one graph link.
fn decode_link(value: &SaveJson) -> Result<NavGraphLinkT, String> {
    let link_type = read_behavior_i32(value, "linkType")?;
    Ok(NavGraphLinkT {
        from: read_behavior_i32(value, "from")?,
        to: read_behavior_i32(value, "to")?,
        link_type: NavLinkType::from_i32(link_type).ok_or_else(|| behavior_malformed("path.links.linkType"))?,
        traversal: read_behavior_opt(value, "traversal", decode_traversal)?,
        entity_bounds: read_behavior_opt(value, "entityBounds", decode_entity_bounds)?,
    })
}

/// Encode a navigation path.
fn encode_path(path: &NavPathT) -> SaveJson {
    obj(vec![
        (
            "nodes",
            arr(path.nodes.iter().map(|node| int(i64::from(*node))).collect()),
        ),
        (
            "points",
            arr(path.points.iter().map(|point| vec3_json(*point)).collect()),
        ),
        (
            "links",
            arr(path
                .links
                .iter()
                .map(|link| opt_json(link.as_ref(), encode_link))
                .collect()),
        ),
        ("cost", num(path.cost)),
        ("generation", int(path.generation)),
        ("mapIdentity", str(&path.map_identity)),
    ])
}

/// Decode a navigation path.
fn decode_path(value: &SaveJson) -> Result<NavPathT, String> {
    let nodes = read_behavior_arr(value, "nodes")?;
    let points = read_behavior_arr(value, "points")?;
    let links = read_behavior_arr(value, "links")?;
    Ok(NavPathT {
        nodes: nodes
            .iter()
            .map(|node| match node {
                SaveJson::Number(number) if number.fract() == 0.0 => {
                    i32::try_from(*number as i64).map_err(|_| behavior_malformed("path.nodes"))
                }
                _ => Err(behavior_malformed("path.nodes")),
            })
            .collect::<Result<Vec<_>, _>>()?,
        points: points.iter().map(vec3_from_json).collect::<Result<Vec<_>, _>>()?,
        links: links
            .iter()
            .map(|link| match link {
                SaveJson::Null => Ok(None),
                item => decode_link(item).map(Some),
            })
            .collect::<Result<Vec<_>, _>>()?,
        cost: read_behavior_num(value, "cost")?,
        generation: read_behavior_int(value, "generation")?,
        map_identity: read_behavior_str(value, "mapIdentity")?,
    })
}

/// Encode the path state.
fn encode_path_state(state: &BotPathStateT) -> SaveJson {
    obj(vec![
        ("path", opt_json(state.path.as_ref(), encode_path)),
        ("index", int(state.index as i64)),
        ("stuckOrigin", vec3_json(state.stuck_origin)),
        ("stuckSince", num(f64::from(state.stuck_since))),
        ("stuckCount", int(i64::from(state.stuck_count))),
        ("liftWaitSince", num(f64::from(state.lift_wait_since))),
        ("liftWaitZ", num(f64::from(state.lift_wait_z))),
        ("jumpReadyAt", num(f64::from(state.jump_ready_at))),
        ("plannedAt", num(f64::from(state.planned_at))),
    ])
}

/// Decode the path state.
fn decode_path_state(value: &SaveJson) -> Result<BotPathStateT, String> {
    let index = read_behavior_int(value, "index")?;
    Ok(BotPathStateT {
        path: read_behavior_opt(value, "path", decode_path)?,
        index: usize::try_from(index).map_err(|_| behavior_malformed("pathState.index"))?,
        stuck_origin: vec3_from_json(read_behavior_obj(value, "stuckOrigin")?)?,
        stuck_since: read_behavior_f32(value, "stuckSince")?,
        stuck_count: read_behavior_i32(value, "stuckCount")?,
        lift_wait_since: read_behavior_f32(value, "liftWaitSince")?,
        lift_wait_z: read_behavior_f32(value, "liftWaitZ")?,
        jump_ready_at: read_behavior_f32(value, "jumpReadyAt")?,
        planned_at: read_behavior_f32(value, "plannedAt")?,
    })
}

/// Encode an explicit goal.
fn encode_explicit_goal(goal: &ExplicitGoalT) -> SaveJson {
    obj(vec![
        (
            "owner",
            str(match goal.owner {
                ExplicitGoalOwner::External => "external",
                ExplicitGoalOwner::Objective => "objective",
            }),
        ),
        (
            "kind",
            str(match goal.kind {
                ExplicitGoalKind::Point => "point",
                ExplicitGoalKind::Entity => "entity",
            }),
        ),
        ("point", vec3_json(goal.point)),
        ("entityId", int(i64::from(goal.entity_id))),
    ])
}

/// Decode an explicit goal.
fn decode_explicit_goal(value: &SaveJson) -> Result<ExplicitGoalT, String> {
    let owner = match read_behavior_str(value, "owner")?.as_str() {
        "external" => ExplicitGoalOwner::External,
        "objective" => ExplicitGoalOwner::Objective,
        _ => return Err(behavior_malformed("explicitGoal.owner")),
    };
    let kind = match read_behavior_str(value, "kind")?.as_str() {
        "point" => ExplicitGoalKind::Point,
        "entity" => ExplicitGoalKind::Entity,
        _ => return Err(behavior_malformed("explicitGoal.kind")),
    };
    Ok(ExplicitGoalT {
        owner,
        kind,
        point: vec3_from_json(read_behavior_obj(value, "point")?)?,
        entity_id: read_behavior_i32(value, "entityId")?,
    })
}

/// Encode the brain memory. Keys mirror the `BotBrainMemory` field names;
/// maps and sets encode sorted so images are deterministic.
fn encode_brain_memory(memory: &BotBrainMemory) -> SaveJson {
    let mut awareness: Vec<(&i32, &BotAwarenessT)> = memory.awareness.iter().collect();
    awareness.sort_by_key(|(id, _)| *id);
    let mut unreachable: Vec<(&i32, &f32)> = memory.unreachable_until.iter().collect();
    unreachable.sort_by_key(|(id, _)| *id);
    let mut homes: Vec<(&i32, &Vec3)> = memory.objective_home.iter().collect();
    homes.sort_by_key(|(id, _)| *id);
    let mut said: Vec<&String> = memory.said_this_level.iter().collect();
    said.sort();
    obj(vec![
        ("triggerWeapon", int(i64::from(memory.trigger_weapon))),
        ("triggerHeldSince", num(f64::from(memory.trigger_held_since))),
        ("triggerReadyAt", num(f64::from(memory.trigger_ready_at))),
        ("aim", encode_aim(&memory.aim)),
        ("pathState", encode_path_state(&memory.path_state)),
        (
            "awareness",
            arr(awareness
                .iter()
                .map(|(id, record)| obj(vec![("key", int(i64::from(**id))), ("value", encode_awareness(record))]))
                .collect()),
        ),
        ("targetId", int(i64::from(memory.target_id))),
        (
            "goalPoint",
            opt_json(memory.goal_point.as_ref(), |point| vec3_json(*point)),
        ),
        ("goalEntityId", int(i64::from(memory.goal_entity_id))),
        (
            "unreachableUntil",
            arr(unreachable
                .iter()
                .map(|(id, until)| obj(vec![("key", int(i64::from(**id))), ("value", num(f64::from(**until)))]))
                .collect()),
        ),
        ("stuckTrips", int(i64::from(memory.stuck_trips))),
        ("goalIsLive", boolean(memory.goal_is_live)),
        ("unstickUntil", num(f64::from(memory.unstick_until))),
        ("pressUntil", num(f64::from(memory.press_until))),
        ("unstickSide", num(f64::from(memory.unstick_side))),
        (
            "explicitGoal",
            opt_json(memory.explicit_goal.as_ref(), encode_explicit_goal),
        ),
        ("explicitGoalDone", boolean(memory.explicit_goal_done)),
        ("explicitGoalFailed", boolean(memory.explicit_goal_failed)),
        (
            "wedgeOrigin",
            opt_json(memory.wedge_origin.as_ref(), |point| vec3_json(*point)),
        ),
        ("wedgeSince", num(f64::from(memory.wedge_since))),
        (
            "lastSafeOrigin",
            opt_json(memory.last_safe_origin.as_ref(), |point| vec3_json(*point)),
        ),
        (
            "restPoint",
            opt_json(memory.rest_point.as_ref(), |point| vec3_json(*point)),
        ),
        ("restUntil", num(f64::from(memory.rest_until))),
        ("guardRefusals", int(i64::from(memory.guard_refusals))),
        ("lastGuardRefused", boolean(memory.last_guard_refused)),
        ("gapJumps", int(i64::from(memory.gap_jumps))),
        ("hazardFrames", int(i64::from(memory.hazard_frames))),
        (
            "objectiveHome",
            arr(homes
                .iter()
                .map(|(id, home)| obj(vec![("key", int(i64::from(**id))), ("value", vec3_json(**home))]))
                .collect()),
        ),
        (
            "ownObjectiveHome",
            opt_json(memory.own_objective_home.as_ref(), |point| vec3_json(*point)),
        ),
        (
            "enemyObjectiveHome",
            opt_json(memory.enemy_objective_home.as_ref(), |point| vec3_json(*point)),
        ),
        ("objectiveRole", str(&memory.objective_role)),
        ("touchGoal", boolean(memory.touch_goal)),
        ("holdPosition", boolean(memory.hold_position)),
        (
            "gateShootAt",
            opt_json(memory.gate_shoot_at.as_ref(), |point| vec3_json(*point)),
        ),
        ("gateFiredAt", num(f64::from(memory.gate_fired_at))),
        ("coopRegrouping", boolean(memory.coop_regrouping)),
        ("coopRegroupAt", num(f64::from(memory.coop_regroup_at))),
        ("coopRegroupUntil", num(f64::from(memory.coop_regroup_until))),
        ("saidThisLevel", arr(said.iter().map(|line| str(line)).collect())),
        ("levelStarted", boolean(memory.level_started)),
        ("checkSixUntil", num(f64::from(memory.check_six_until))),
        ("checkSixNextAt", num(f64::from(memory.check_six_next_at))),
        (
            "roamPoint",
            opt_json(memory.roam_point.as_ref(), |point| vec3_json(*point)),
        ),
        ("roamUntil", num(f64::from(memory.roam_until))),
        ("lastCmd", encode_usercmd(&memory.last_cmd)),
        ("spawnedOnce", boolean(memory.spawned_once)),
        ("lastWeaponNumber", int(i64::from(memory.last_weapon_number))),
        ("deadSince", num(f64::from(memory.dead_since))),
        ("respawnWait", num(f64::from(memory.respawn_wait))),
        ("respawnPress", boolean(memory.respawn_press)),
    ])
}

/// Decode one keyed map entry.
fn decode_map_entry<T>(
    item: &SaveJson,
    key: &str,
    decode: impl Fn(&SaveJson) -> Result<T, String>,
) -> Result<(i32, T), String> {
    let id = read_behavior_i32(item, "key")?;
    let value = item.get("value").ok_or_else(|| behavior_missing(key))?;
    decode(value).map(|value| (id, value))
}

/// Decode the brain memory.
fn decode_brain_memory(value: &SaveJson) -> Result<BotBrainMemory, String> {
    let awareness = read_behavior_arr(value, "awareness")?;
    let unreachable = read_behavior_arr(value, "unreachableUntil")?;
    let homes = read_behavior_arr(value, "objectiveHome")?;
    let said = read_behavior_arr(value, "saidThisLevel")?;
    Ok(BotBrainMemory {
        trigger_weapon: read_behavior_i32(value, "triggerWeapon")?,
        trigger_held_since: read_behavior_f32(value, "triggerHeldSince")?,
        trigger_ready_at: read_behavior_f32(value, "triggerReadyAt")?,
        aim: decode_aim(read_behavior_obj(value, "aim")?)?,
        path_state: decode_path_state(read_behavior_obj(value, "pathState")?)?,
        awareness: awareness
            .iter()
            .map(|item| decode_map_entry(item, "awareness.value", decode_awareness))
            .collect::<Result<HashMap<_, _>, _>>()?,
        target_id: read_behavior_i32(value, "targetId")?,
        goal_point: read_behavior_opt(value, "goalPoint", vec3_from_json)?,
        goal_entity_id: read_behavior_i32(value, "goalEntityId")?,
        unreachable_until: unreachable
            .iter()
            .map(|item| {
                decode_map_entry(item, "unreachableUntil.value", |entry| match entry {
                    SaveJson::Number(number) => Ok(*number as f32),
                    _ => Err(behavior_malformed("unreachableUntil.value")),
                })
            })
            .collect::<Result<HashMap<_, _>, _>>()?,
        stuck_trips: read_behavior_i32(value, "stuckTrips")?,
        goal_is_live: read_behavior_bool(value, "goalIsLive")?,
        unstick_until: read_behavior_f32(value, "unstickUntil")?,
        press_until: read_behavior_f32(value, "pressUntil")?,
        unstick_side: read_behavior_f32(value, "unstickSide")?,
        explicit_goal: read_behavior_opt(value, "explicitGoal", decode_explicit_goal)?,
        explicit_goal_done: read_behavior_bool(value, "explicitGoalDone")?,
        explicit_goal_failed: read_behavior_bool(value, "explicitGoalFailed")?,
        wedge_origin: read_behavior_opt(value, "wedgeOrigin", vec3_from_json)?,
        wedge_since: read_behavior_f32(value, "wedgeSince")?,
        last_safe_origin: read_behavior_opt(value, "lastSafeOrigin", vec3_from_json)?,
        rest_point: read_behavior_opt(value, "restPoint", vec3_from_json)?,
        rest_until: read_behavior_f32(value, "restUntil")?,
        guard_refusals: read_behavior_i32(value, "guardRefusals")?,
        last_guard_refused: read_behavior_bool(value, "lastGuardRefused")?,
        gap_jumps: read_behavior_i32(value, "gapJumps")?,
        hazard_frames: read_behavior_i32(value, "hazardFrames")?,
        objective_home: homes
            .iter()
            .map(|item| decode_map_entry(item, "objectiveHome.value", vec3_from_json))
            .collect::<Result<HashMap<_, _>, _>>()?,
        own_objective_home: read_behavior_opt(value, "ownObjectiveHome", vec3_from_json)?,
        enemy_objective_home: read_behavior_opt(value, "enemyObjectiveHome", vec3_from_json)?,
        objective_role: read_behavior_str(value, "objectiveRole")?,
        touch_goal: read_behavior_bool(value, "touchGoal")?,
        hold_position: read_behavior_bool(value, "holdPosition")?,
        gate_shoot_at: read_behavior_opt(value, "gateShootAt", vec3_from_json)?,
        gate_fired_at: read_behavior_f32(value, "gateFiredAt")?,
        coop_regrouping: read_behavior_bool(value, "coopRegrouping")?,
        coop_regroup_at: read_behavior_f32(value, "coopRegroupAt")?,
        coop_regroup_until: read_behavior_f32(value, "coopRegroupUntil")?,
        said_this_level: said
            .iter()
            .map(|line| match line {
                SaveJson::String(text) => Ok(text.clone()),
                _ => Err(behavior_malformed("saidThisLevel")),
            })
            .collect::<Result<HashSet<_>, _>>()?,
        level_started: read_behavior_bool(value, "levelStarted")?,
        check_six_until: read_behavior_f32(value, "checkSixUntil")?,
        check_six_next_at: read_behavior_f32(value, "checkSixNextAt")?,
        roam_point: read_behavior_opt(value, "roamPoint", vec3_from_json)?,
        roam_until: read_behavior_f32(value, "roamUntil")?,
        last_cmd: decode_usercmd(read_behavior_obj(value, "lastCmd")?)?,
        spawned_once: read_behavior_bool(value, "spawnedOnce")?,
        last_weapon_number: read_behavior_i32(value, "lastWeaponNumber")?,
        dead_since: read_behavior_f32(value, "deadSince")?,
        respawn_wait: read_behavior_f32(value, "respawnWait")?,
        respawn_press: read_behavior_bool(value, "respawnPress")?,
    })
}

/// Encode a brain checkpoint.
fn encode_brain_checkpoint(checkpoint: &BotBrainCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(i64::from(checkpoint.version))),
        ("skill", str(&checkpoint.skill)),
        ("gameMode", encode_game_mode(&checkpoint.game_mode)),
        ("memory", encode_brain_memory(&checkpoint.memory)),
    ])
}

/// Decode a brain checkpoint.
fn decode_brain_checkpoint(value: &SaveJson) -> Result<BotBrainCheckpoint, String> {
    let version = read_behavior_int(value, "version")?;
    Ok(BotBrainCheckpoint {
        version: u32::try_from(version).map_err(|_| behavior_malformed("brain.version"))?,
        skill: read_behavior_str(value, "skill")?,
        game_mode: decode_game_mode(read_behavior_obj(value, "gameMode")?)?,
        memory: decode_brain_memory(read_behavior_obj(value, "memory")?)?,
    })
}

/// Movement player read (donor `movementPlayer` used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct BotMovementView {
    /// Owning client.
    pub client: ClientId,
    /// Movement profile kind.
    pub profile_kind: String,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Arsenal provider.
    pub arsenal_provider: String,
}

/// Source player identity (donor `setSourcePlayerIdentity` payload).
#[derive(Debug, Clone, PartialEq)]
pub struct BotPlayerIdentity {
    /// Player name.
    pub name: String,
    /// Skin path, when cosmetic.
    pub skin: Option<String>,
    /// Shirt color, when cosmetic.
    pub shirt: Option<f64>,
    /// Pants color, when cosmetic.
    pub pants: Option<f64>,
}

/// Simulation surface used by this transport.
///
/// Seam over `SharedSimulation` from donor
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); the partition implements it post-merge. `S` is a cheap
/// shared handle (`Clone` shares live state): worlds and objectives hold
/// clones, so every method takes `&self` and the handle owns interior
/// mutability.
pub trait RereleaseBotSimulation: Clone {
    /// Donor `options.configuration ?? q1Source()?.cvars ??
    /// q2ServerCvars()`, in that order.
    fn bot_configuration(&self) -> Option<Rc<RefCell<qa_core::cvar::CvarRegistry>>>;
    /// Donor `options.skill`.
    fn bot_skill(&self) -> i32;
    /// Donor `options.maxClients`.
    fn bot_max_clients(&self) -> u32;
    /// Donor `players()`.
    fn bot_players(&self) -> Vec<ActorId>;
    /// Donor `movementPlayer`.
    fn bot_movement(&self, actor: &ActorId) -> Option<BotMovementView>;
    /// Donor `timeSeconds`.
    fn bot_time_seconds(&self) -> f64;
    /// Donor `physics.gravity`.
    fn bot_gravity(&self) -> f64;
    /// Donor `random.nextInteger`.
    fn bot_random_integer(&self) -> i32;
    /// Donor `bodies.read(actor)?.origin`.
    fn bot_body_origin(&self, actor: &ActorId) -> Option<Vec3>;
    /// Donor `clientIdentities()`.
    fn bot_client_identities(&self) -> Vec<ClientId>;
    /// Donor `actors.resolveOwned`.
    fn bot_resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Donor `actors.referenceSaved`.
    fn bot_reference_saved(&self, saved: SavedActorId) -> ActorId;
    /// Donor `actors.resolveSaved`.
    fn bot_resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor>;
    /// Donor `actors.isLive`.
    fn bot_is_live(&self, actor: &ActorId) -> bool;
    /// Donor `admitPlayer`; the message propagates like the donor throw.
    fn bot_admit_player(&self, client: &ClientId) -> Result<ActorId, String>;
    /// Donor `disconnectPlayer`.
    fn bot_disconnect_player(&self, actor: &ActorId);
    /// Donor `playerCommand`.
    fn bot_player_command(&self, actor: &ActorId, name: &str, args: &[String]);
    /// Donor `setSourcePlayerIdentity`.
    fn bot_set_player_identity(&self, actor: &ActorId, identity: &BotPlayerIdentity);
    /// Donor `notifyClientEvent`.
    fn bot_notify_client_event(&self, kind: &str, actor: &ActorId);
    /// Donor `botServices.attachTransport`.
    fn attach_bot_transport(&self, transport: Rc<RefCell<dyn ApplicationBotService>>);
    /// Donor `botServices.detachTransport`.
    fn detach_bot_transport(&self);
}

/// Session surface used by this transport.
///
/// Seam over `EngineSession` from donor `src/world/session/session.ts`
/// (canonical home: `qa_world::session`); the partition implements it
/// post-merge.
pub trait RereleaseBotSession {
    /// Donor `session.createClient`.
    fn create_bot_client(&mut self, slot: u32) -> Result<SessionClient, WorldError>;
    /// Donor `session.closeClient`.
    fn close_bot_client(&mut self, client: &ClientId);
}

/// Navigation surface used by this transport.
///
/// Seam over `ApplicationBotNavigation` from donor
/// `src/app/bootstrap/simulation/navigation.ts` (canonical home: the
/// navigation partition); the partition implements it post-merge.
pub trait RereleaseBotNav {
    /// Donor `navigation.checkpoint()`.
    fn nav_checkpoint_json(&self) -> SaveJson;
    /// Donor `navigation.restoreCheckpoint()`; the message propagates like
    /// the donor throw.
    fn nav_restore_json(&mut self, image: &SaveJson) -> Result<(), String>;
    /// Donor `navigation.forClient(slot)` graph.
    fn nav_for_client(&self, slot: u32) -> Box<dyn RereleaseNavigation>;
}

impl<N: RereleaseBotNav> RereleaseBotNavigation for Rc<RefCell<N>> {
    fn for_client(&self, slot: u32) -> Box<dyn RereleaseNavigation> {
        self.borrow().nav_for_client(slot)
    }
}

/// Rerelease bots options (donor `ApplicationBotsOptions` used surface).
pub struct ApplicationRereleaseBotsOptions<S, E, N> {
    /// Engine session.
    pub session: E,
    /// Shared simulation handle.
    pub simulation: S,
    /// Shared navigation.
    pub navigation: N,
    /// Shared configuration registry.
    pub configuration: Option<Rc<RefCell<qa_core::cvar::CvarRegistry>>>,
    /// Restore input.
    pub restore: Option<ApplicationBotsRestore>,
    /// Preserved clients.
    pub clients: Vec<ApplicationBotClient>,
    /// Whether services frame this transport.
    pub automatic_frame: bool,
    /// Behavior factory; `None` builds the production driver via
    /// [`default_behavior_factory`].
    pub behavior_factory: Option<RereleaseBehaviorFactory>,
    /// Print sink.
    pub print: Rc<dyn Fn(&str)>,
}

/// Shared rerelease host state behind world closures.
#[derive(Debug, Default)]
struct SharedRereleaseState {
    /// Observation numbers by actor number.
    observations: HashMap<i32, ActorId>,
    /// Next observation number.
    next_observation: i32,
    /// Bot-controlled actors.
    bot_actors: HashSet<ActorId>,
    /// Elapsed frame milliseconds.
    elapsed_ms: f64,
    /// Noises heard since the last frame.
    heard: Vec<BotSoundT>,
}

/// World host closures (donor `identify`/`isBot`/`elapsed`/`sounds`).
#[derive(Debug, Clone)]
struct RereleaseWorldHost {
    /// Shared state.
    shared: Rc<RefCell<SharedRereleaseState>>,
}

impl RereleaseBotWorldHost for RereleaseWorldHost {
    fn is_bot(&self, actor: &ActorId) -> bool {
        self.shared.borrow().bot_actors.contains(actor)
    }

    fn identify(&self, actor: &ActorId) -> i32 {
        let mut shared = self.shared.borrow_mut();
        if let Some(number) = shared
            .observations
            .iter()
            .find(|(_, existing)| *existing == actor)
            .map(|(number, _)| *number)
        {
            return number;
        }
        let number = shared.next_observation;
        shared.next_observation += 1;
        shared.observations.insert(number, actor.clone());
        number
    }

    fn elapsed_ms(&self) -> u64 {
        self.shared.borrow().elapsed_ms as u64
    }

    fn sounds(&self) -> Vec<BotSoundT> {
        self.shared.borrow().heard.clone()
    }
}

/// Native rerelease bot connection (donor `Connection`).
struct RereleaseConnection<S> {
    /// Live client binding.
    bot: Rc<RefCell<BotConnection>>,
    /// Client userinfo.
    userinfo: Option<String>,
    /// Behavior driver.
    behavior: Box<dyn RereleaseBotBehavior>,
    /// World projection.
    world: RereleaseBotWorld<S, RereleaseBotObjectivesImpl<S>, RereleaseWorldHost>,
    /// Bot name.
    name: String,
    /// Skill name.
    skill: String,
    /// Selected weapon number.
    selection: Rc<RefCell<i32>>,
    /// Command sequence.
    sequence: u64,
}

/// Shared rerelease bot transport handle.
pub type ApplicationRereleaseBotsHandle<S, E, N> = Rc<RefCell<ApplicationRereleaseBots<S, E, N>>>;

/// Native rerelease bot transport (donor `ApplicationRereleaseBots`).
pub struct ApplicationRereleaseBots<S, E, N> {
    /// Engine session.
    session: E,
    /// Shared simulation handle.
    simulation: S,
    /// Shared navigation.
    navigation: Rc<RefCell<N>>,
    /// Bot objectives.
    objectives: RereleaseBotObjectivesImpl<S>,
    /// Shared configuration registry.
    configuration: Rc<RefCell<qa_core::cvar::CvarRegistry>>,
    /// Behavior factory.
    behavior_factory: RereleaseBehaviorFactory,
    /// Print sink.
    print: Rc<dyn Fn(&str)>,
    /// Asset source family.
    assets_source: RereleaseBotSource,
    /// Bot source files.
    asset_files: BotAssetFiles,
    /// Parsed knowledge.
    knowledge: Rc<BotKnowledge>,
    /// Server localization.
    localization: LocalizationTable,
    /// Shared host state.
    shared: Rc<RefCell<SharedRereleaseState>>,
    /// Connections by client slot.
    connections: HashMap<u32, RereleaseConnection<S>>,
    /// Whether services frame this transport.
    automatic_frame: bool,
    /// Whether the transport is closed.
    closed: bool,
}

impl<S, E, N> std::fmt::Debug for ApplicationRereleaseBots<S, E, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplicationRereleaseBots")
            .field("assets_source", &self.assets_source)
            .field("connections", &self.connections.keys().collect::<Vec<_>>())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

/// Create the bot transport for an asset family (donor
/// `createApplicationBots`).
pub fn create_application_bots<S, E, N>(
    options: ApplicationRereleaseBotsOptions<S, E, N>,
    assets: ApplicationBotAssets,
) -> Result<ApplicationRereleaseBotsHandle<S, E, N>, ApplicationBotError>
where
    S: RereleaseBotSimulation + RereleaseBotWorldSimulation + BotObjectiveSimulation + 'static,
    E: RereleaseBotSession + 'static,
    N: RereleaseBotNav + 'static,
{
    match assets {
        ApplicationBotAssets::Rerelease { .. } => ApplicationRereleaseBots::new(options, assets),
        ApplicationBotAssets::Q3 { .. } => Err(ApplicationBotError::Admission(
            "Q3 bot assets require the Q3 bot constructor (bots::create_application_bots)".to_string(),
        )),
    }
}

impl<S, E, N> ApplicationRereleaseBots<S, E, N>
where
    S: RereleaseBotSimulation + RereleaseBotWorldSimulation + BotObjectiveSimulation + 'static,
    E: RereleaseBotSession + 'static,
    N: RereleaseBotNav + 'static,
{
    /// Create and attach the transport (donor constructor).
    pub fn new(
        options: ApplicationRereleaseBotsOptions<S, E, N>,
        assets: ApplicationBotAssets,
    ) -> Result<ApplicationRereleaseBotsHandle<S, E, N>, ApplicationBotError> {
        let ApplicationBotAssets::Rerelease {
            source,
            files,
            knowledge,
            localization,
        } = assets
        else {
            return Err(ApplicationBotError::Admission(
                "Q3 bot assets require the Q3 bot constructor (bots::create_application_bots)".to_string(),
            ));
        };
        let configuration = options
            .configuration
            .or_else(|| options.simulation.bot_configuration())
            .ok_or(ApplicationBotError::MissingConfiguration)?;
        // The `RereleaseBotNav` seam contract guarantees the shared
        // checkpoint lifetime, so the donor `"checkpoint" in navigation`
        // check is vacuous.
        let knowledge = Rc::new(*knowledge);
        let objectives = rerelease_bot_objectives(options.simulation.clone(), Rc::clone(&knowledge))
            .map_err(|error| ApplicationBotError::Admission(format!("{error:?}")))?;
        configuration
            .borrow_mut()
            .register("bot_minplayers", "0", 0)
            .map_err(|error| ApplicationBotError::Admission(format!("{error:?}")))?;
        configuration
            .borrow_mut()
            .register("bot_enable", "1", 0)
            .map_err(|error| ApplicationBotError::Admission(format!("{error:?}")))?;
        let transport = Rc::new(RefCell::new(Self {
            session: options.session,
            simulation: options.simulation,
            navigation: Rc::new(RefCell::new(options.navigation)),
            objectives,
            configuration,
            behavior_factory: options.behavior_factory.unwrap_or_else(default_behavior_factory),
            print: options.print,
            assets_source: source,
            asset_files: files,
            knowledge,
            localization,
            shared: Rc::new(RefCell::new(SharedRereleaseState {
                next_observation: 1,
                ..SharedRereleaseState::default()
            })),
            connections: HashMap::new(),
            automatic_frame: options.automatic_frame,
            closed: false,
        }));
        if let Some(restore) = options.restore {
            transport.borrow_mut().restore(&restore)?;
        } else {
            for saved in options.clients {
                let name = transport.borrow().name(&saved);
                let skill = transport.borrow().default_skill()?;
                transport.borrow_mut().connect(saved, name, skill, None)?;
            }
        }
        transport
            .borrow()
            .simulation
            .attach_bot_transport(transport.clone() as Rc<RefCell<dyn ApplicationBotService>>);
        Ok(transport)
    }

    /// Default skill name (donor `defaultSkill`).
    fn default_skill(&self) -> Result<String, ApplicationBotError> {
        let skills = &self.knowledge.skills;
        let index = (skills.len() as i32 - 1).min(self.simulation.bot_skill() + 1);
        let selected = (index >= 0)
            .then(|| skills.get(index as usize))
            .flatten()
            .or_else(|| skills.first());
        selected.map(|skill| skill.skill.clone()).ok_or_else(|| {
            ApplicationBotError::Admission("Native bot assets contain no source skill settings".to_string())
        })
    }

    /// Bot name for a preserved client (donor `name`).
    fn name(&self, saved: &ApplicationBotClient) -> String {
        let fields: Vec<&str> = saved.userinfo.as_deref().unwrap_or("").split('\\').collect();
        match fields.iter().position(|field| *field == "name") {
            Some(index) => fields
                .get(index + 1)
                .map(|name| name.to_string())
                .unwrap_or_else(|| "Bot".to_string()),
            None => self
                .knowledge
                .characters
                .first()
                .map(|character| character.name.clone())
                .unwrap_or_else(|| "Bot".to_string()),
        }
    }

    /// Stable observation number for an actor (donor `identify`).
    fn identify(&self, actor: &ActorId) -> i32 {
        RereleaseWorldHost {
            shared: Rc::clone(&self.shared),
        }
        .identify(actor)
    }

    /// Connect a bot client (donor `connect`).
    fn connect(
        &mut self,
        saved: ApplicationBotClient,
        name: String,
        skill: String,
        restored: Option<OwnedActor>,
    ) -> Result<&mut RereleaseConnection<S>, ApplicationBotError> {
        if saved.client.is_closed() || self.connections.contains_key(&saved.client.id().slot()) {
            return Err(ApplicationBotError::Admission(
                "Native bot client is closed or already admitted".to_string(),
            ));
        }
        let existing = self.simulation.bot_players().into_iter().find(|actor| {
            self.simulation
                .bot_movement(actor)
                .is_some_and(|movement| movement.client == *saved.client.id())
        });
        let admitted = match existing {
            Some(actor) => actor,
            None => self
                .simulation
                .bot_admit_player(saved.client.id())
                .map_err(ApplicationBotError::Admission)?,
        };
        let fresh = restored.is_none();
        let actor = match restored {
            Some(actor) => actor,
            None => self.simulation.bot_resolve_owned(&admitted).ok_or_else(|| {
                ApplicationBotError::Admission("Native bot admission has no shared actor".to_string())
            })?,
        };
        let player = self
            .simulation
            .bot_movement(actor.id())
            .ok_or_else(|| ApplicationBotError::Admission("Native bot admission lost selected movement".to_string()))?;
        let character = self
            .knowledge
            .characters
            .iter()
            .find(|character| character.name.to_lowercase() == name.to_lowercase())
            .cloned();
        let selection = Rc::new(RefCell::new(0));
        let definition = self
            .asset_files
            .provenance()
            .ok_or_else(|| ApplicationBotError::Admission("Native bot assets require saved provenance".to_string()))?;
        let callbacks = self.callbacks(&actor, &selection)?;
        let gravity = self.simulation.bot_gravity();
        let behavior = (self.behavior_factory)(RereleaseBehaviorParams {
            definition,
            source: self.assets_source,
            knowledge: Rc::clone(&self.knowledge),
            skill: skill.clone(),
            seed: if fresh { self.simulation.bot_random_integer() } else { 0 },
            game_mode: self.objectives.mode(),
            character: character.clone(),
            max_health: 100.0,
            run_speed: if player.profile_kind.starts_with("q2") {
                300.0
            } else {
                320.0
            },
            walk_speed: 160.0,
            movement: RereleaseBehaviorMovement {
                gravity,
                jump_velocity: 270.0,
                jump_air_seconds: 540.0 / gravity,
                maximum_landing_rise: 18.0,
                start_above: 56.0,
                body_mins: player.standing_bounds.min,
                body_maxs: player.standing_bounds.max,
            },
            callbacks,
        })
        .map_err(ApplicationBotError::Admission)?;
        let objectives = rerelease_bot_objectives(self.simulation.clone(), Rc::clone(&self.knowledge))
            .map_err(|error| ApplicationBotError::Admission(format!("{error:?}")))?;
        let world = RereleaseBotWorld::new(
            RereleaseWorldOptions {
                simulation: self.simulation.clone(),
                navigation: Rc::clone(&self.navigation),
                knowledge: Rc::clone(&self.knowledge),
                objectives,
                host: RereleaseWorldHost {
                    shared: Rc::clone(&self.shared),
                },
            },
            actor.id().clone(),
        )
        .map_err(|error| ApplicationBotError::Admission(format!("{error:?}")))?;
        let slot = saved.client.id().slot();
        let userinfo = Some(saved.userinfo.clone().unwrap_or_else(|| format!("\\name\\{name}")));
        self.shared.borrow_mut().bot_actors.insert(actor.id().clone());
        self.connections.insert(
            slot,
            RereleaseConnection {
                bot: Rc::new(RefCell::new(BotConnection {
                    client: saved.client,
                    actor,
                    reliable: saved.reliable,
                })),
                userinfo,
                behavior,
                world,
                name: name.clone(),
                skill,
                selection,
                sequence: 0,
            },
        );
        if fresh {
            let actor_id = self.connections[&slot].bot.borrow().actor.id().clone();
            let cosmetic = !self.objectives.mode().has_teams.unwrap_or(false) && character.is_some();
            let q2 = self.assets_source == RereleaseBotSource::Q2;
            let q1 = self.assets_source == RereleaseBotSource::Q1;
            self.simulation.bot_set_player_identity(
                &actor_id,
                &BotPlayerIdentity {
                    name,
                    skin: if cosmetic && q2 {
                        character
                            .as_ref()
                            .and_then(|character| (!character.skin.is_empty()).then(|| character.skin.clone()))
                    } else {
                        None
                    },
                    shirt: if cosmetic && q1 {
                        character.as_ref().map(|character| character.shirt_color)
                    } else {
                        None
                    },
                    pants: if cosmetic && q1 {
                        character.as_ref().map(|character| character.pants_color)
                    } else {
                        None
                    },
                },
            );
            self.objectives.admit(&actor_id);
        }
        Ok(self.connections.get_mut(&slot).expect("connected bot slot"))
    }

    /// Behavior callbacks for a connection (donor `callbacks`).
    fn callbacks(
        &self,
        actor: &OwnedActor,
        selection: &Rc<RefCell<i32>>,
    ) -> Result<RereleaseBehaviorCallbacks, ApplicationBotError> {
        let time_simulation = self.simulation.clone();
        let teammate_simulation = self.simulation.clone();
        let teammate_objectives = rerelease_bot_objectives(self.simulation.clone(), Rc::clone(&self.knowledge))
            .map_err(|error| ApplicationBotError::Admission(format!("{error:?}")))?;
        let teammate_shared = Rc::clone(&self.shared);
        let own = actor.id().clone();
        let chat_simulation = self.simulation.clone();
        let chat_actor = actor.id().clone();
        let chat_source = self.assets_source;
        let chat_localization = self.localization.clone();
        let select = Rc::clone(selection);
        let impulse = Rc::clone(selection);
        Ok(RereleaseBehaviorCallbacks {
            time: Rc::new(move || time_simulation.bot_time_seconds()),
            pre_think: Rc::new(|| {}),
            post_think: Rc::new(|| {}),
            chat: Rc::new(move |event, random| {
                let lookup = |key: &str| chat_localization.lookup(key, &[]);
                let text = if chat_source == RereleaseBotSource::Q1 {
                    q1_bot_chat_text(&event.locstring, &lookup, random)
                } else {
                    lookup(&event.locstring).unwrap_or(event.locstring)
                };
                chat_simulation.bot_player_command(
                    &chat_actor,
                    if event.team_only { "say_team" } else { "say" },
                    &[text],
                );
            }),
            select_weapon: Rc::new(move |number| {
                *select.borrow_mut() = number;
            }),
            weapon_impulse: Rc::new(move |number| {
                *impulse.borrow_mut() = number;
                0
            }),
            human_teammate_near: Rc::new(move || {
                teammate_simulation.bot_players().into_iter().any(|other| {
                    !teammate_shared.borrow().bot_actors.contains(&other)
                        && teammate_objectives.team(&other) == teammate_objectives.team(&own)
                        && teammate_simulation
                            .bot_body_origin(&other)
                            .zip(teammate_simulation.bot_body_origin(&own))
                            .is_some_and(|(body, home)| {
                                ((f64::from(body.x) - f64::from(home.x)).powi(2)
                                    + (f64::from(body.y) - f64::from(home.y)).powi(2)
                                    + (f64::from(body.z) - f64::from(home.z)).powi(2))
                                .sqrt()
                                    < 256.0
                            })
                })
            }),
        })
    }
}

impl<S, E, N> ApplicationRereleaseBots<S, E, N>
where
    S: RereleaseBotSimulation + RereleaseBotWorldSimulation + BotObjectiveSimulation + 'static,
    E: RereleaseBotSession + 'static,
    N: RereleaseBotNav + 'static,
{
    /// Admit a named bot (donor `add`).
    fn add(&mut self, name: String, skill: String) -> Result<(), ApplicationBotError> {
        let occupied: HashSet<u32> = self
            .simulation
            .bot_client_identities()
            .iter()
            .map(ClientId::slot)
            .collect();
        for slot in 0..self.simulation.bot_max_clients() {
            if occupied.contains(&slot) {
                continue;
            }
            let client = self.session.create_bot_client(slot)?;
            let id = client.id().clone();
            match self.connect(
                ApplicationBotClient {
                    client,
                    reliable: ServerReliableCommands::new(),
                    userinfo: None,
                },
                name,
                skill,
                None,
            ) {
                Ok(_) => return Ok(()),
                Err(error) => {
                    // The seams are infallible, so cleanup cannot fail and the
                    // donor `AggregateError` path is unreachable.
                    if let Some(actor) = self.simulation.bot_players().into_iter().find(|actor| {
                        self.simulation
                            .bot_movement(actor)
                            .is_some_and(|movement| movement.client == id)
                    }) {
                        self.simulation.bot_disconnect_player(&actor);
                        self.shared.borrow_mut().bot_actors.remove(&actor);
                    }
                    self.connections.remove(&id.slot());
                    self.session.close_bot_client(&id);
                    return Err(error);
                }
            }
        }
        (self.print)("Unable to add bot: server is full\n");
        Ok(())
    }

    /// Translate a behavior command (donor `command`).
    fn translate(
        &self,
        source: &BotUsercmdT,
        actor: &ActorId,
    ) -> Result<qa_net::common::commands::UserCommand, ApplicationBotError> {
        let player = self
            .simulation
            .bot_movement(actor)
            .ok_or_else(|| ApplicationBotError::Admission("Native bot lost command player".to_string()))?;
        let dialect = if player.profile_kind.starts_with("q2") {
            MovementKind::Q2Rerelease
        } else {
            MovementKind::Q1Netquake
        };
        Ok(rerelease_bot_command(
            source,
            dialect,
            self.shared.borrow().elapsed_ms,
            (self.simulation.bot_time_seconds() * 1000.0).trunc(),
        ))
    }

    /// Capture the checkpoint image (donor `checkpoint`).
    fn capture(&self) -> Result<SaveJson, ApplicationBotError> {
        let assets = self.asset_files.provenance().ok_or_else(|| {
            ApplicationBotError::RestorationRejected("Native bot assets lack save provenance".to_string())
        })?;
        let mut slots: Vec<u32> = self.connections.keys().copied().collect();
        slots.sort_unstable();
        let transport_connections = slots
            .iter()
            .map(|slot| {
                let connection = &self.connections[slot];
                let bot = connection.bot.borrow();
                obj(vec![
                    (
                        "client",
                        obj(vec![
                            ("slot", int(i64::from(bot.client.id().slot()))),
                            ("generation", int(i64::from(bot.client.id().generation()))),
                        ]),
                    ),
                    ("actor", write_saved_actor(SavedActorId::from(bot.actor.id()))),
                    (
                        "reliable",
                        obj(vec![
                            ("sequence", int(i64::from(bot.reliable.sequence()))),
                            ("acknowledge", int(i64::from(bot.reliable.acknowledge()))),
                            (
                                "slots",
                                arr((0..MAX_RELIABLE_COMMANDS as i32)
                                    .map(|index| str(&bot.reliable.lookup_masked(index)))
                                    .collect()),
                            ),
                        ]),
                    ),
                ])
            })
            .collect();
        let director_clients = slots
            .iter()
            .map(|slot| {
                let connection = &self.connections[slot];
                obj(vec![
                    ("slot", int(i64::from(*slot))),
                    ("name", str(&connection.name)),
                    ("skill", str(&connection.skill)),
                    ("sequence", int(connection.sequence as i64)),
                    ("selection", int(i64::from(*connection.selection.borrow()))),
                    ("behavior", connection.behavior.checkpoint_json()),
                ])
            })
            .collect();
        let shared = self.shared.borrow();
        let mut numbers: Vec<i32> = shared.observations.keys().copied().collect();
        numbers.sort_unstable();
        Ok(obj(vec![
            ("version", int(1)),
            (
                "transport",
                obj(vec![
                    ("version", int(1)),
                    ("elapsedMilliseconds", num(shared.elapsed_ms)),
                    ("snapshots", arr(Vec::new())),
                    ("connections", arr(transport_connections)),
                ]),
            ),
            (
                "director",
                obj(vec![
                    ("version", int(1)),
                    ("kind", str("rerelease")),
                    ("assets", str(&assets)),
                    ("nextObservation", int(i64::from(shared.next_observation))),
                    (
                        "heard",
                        arr(shared
                            .heard
                            .iter()
                            .map(|sound| {
                                obj(vec![
                                    (
                                        "origin",
                                        obj(vec![
                                            ("x", num(f64::from(sound.origin.x))),
                                            ("y", num(f64::from(sound.origin.y))),
                                            ("z", num(f64::from(sound.origin.z))),
                                        ]),
                                    ),
                                    ("sourceId", int(i64::from(sound.source_id))),
                                    ("time", num(f64::from(sound.time))),
                                    ("loudness", num(f64::from(sound.loudness))),
                                ])
                            })
                            .collect()),
                    ),
                    ("clients", arr(director_clients)),
                ]),
            ),
            ("navigation", self.navigation.borrow().nav_checkpoint_json()),
            ("knowledge", SaveJson::Null),
            ("sharedWorld", SaveJson::Null),
            (
                "observations",
                arr(numbers
                    .iter()
                    .map(|number| {
                        obj(vec![
                            ("number", int(i64::from(*number))),
                            (
                                "actor",
                                write_saved_actor(SavedActorId::from(&shared.observations[number])),
                            ),
                        ])
                    })
                    .collect()),
            ),
        ]))
    }

    /// Restore a checkpoint image (donor `restore`).
    fn restore(&mut self, restore: &ApplicationBotsRestore) -> Result<(), ApplicationBotError> {
        let image = &restore.image;
        let director = &image.director;
        let version = get_int(director, "version")?;
        let kind = get_str(director, "kind")?;
        if version != 1 || kind != "rerelease" {
            return Err(ApplicationBotError::RestorationRejected(
                "Saved native bot population differs".to_string(),
            ));
        }
        if get_str(director, "assets")? != self.asset_files.provenance().unwrap_or_default() {
            return Err(ApplicationBotError::RestorationRejected(
                "Saved native bot assets differ from mounted definitions".to_string(),
            ));
        }
        self.navigation
            .borrow_mut()
            .nav_restore_json(&image.navigation)
            .map_err(ApplicationBotError::RestorationRejected)?;
        let next_observation = get_int_min(director, "nextObservation", 1)?;
        let heard = get_list(director, "heard")?
            .iter()
            .map(|sound| {
                let origin = get_object(sound, "origin")?;
                Ok(BotSoundT {
                    origin: Vec3 {
                        x: get_finite(origin, "x")? as f32,
                        y: get_finite(origin, "y")? as f32,
                        z: get_finite(origin, "z")? as f32,
                    },
                    source_id: get_int(sound, "sourceId")? as i32,
                    time: get_finite(sound, "time")? as f32,
                    loudness: get_finite(sound, "loudness")? as f32,
                })
            })
            .collect::<Result<Vec<_>, ApplicationBotError>>()?;
        {
            let mut shared = self.shared.borrow_mut();
            shared.next_observation = next_observation as i32;
            shared.heard = heard;
            for entry in &image.observations {
                if entry.number < 1
                    || entry.number >= i64::from(shared.next_observation)
                    || shared.observations.contains_key(&(entry.number as i32))
                {
                    return Err(ApplicationBotError::RestorationRejected(
                        "Invalid native bot observation sequence".to_string(),
                    ));
                }
                let actor = self.simulation.bot_reference_saved(entry.actor);
                shared.observations.insert(entry.number as i32, actor);
            }
        }
        let clients = get_list(director, "clients")?
            .iter()
            .map(|entry| {
                Ok(RestoredDirectorClient {
                    slot: get_int_min(entry, "slot", 0)? as u32,
                    name: get_str(entry, "name")?,
                    skill: get_str(entry, "skill")?,
                    sequence: get_int_min(entry, "sequence", 0)? as u64,
                    selection: get_int_min(entry, "selection", 0)? as i32,
                    behavior: entry.get("behavior").cloned().unwrap_or(SaveJson::Null),
                })
            })
            .collect::<Result<Vec<_>, ApplicationBotError>>()?;
        let mut seen = HashSet::new();
        if clients.len() != image.transport.connections.len() || !clients.iter().all(|client| seen.insert(client.slot))
        {
            return Err(ApplicationBotError::RestorationRejected(
                "Saved native bot decisions and transport clients differ".to_string(),
            ));
        }
        for entry in &image.transport.connections {
            let client = (restore.resolve_client)(entry.client).ok_or_else(|| {
                ApplicationBotError::RestorationRejected("Saved native bot lacks its original client/actor".to_string())
            })?;
            let actor = self.simulation.bot_resolve_saved(entry.actor).ok_or_else(|| {
                ApplicationBotError::RestorationRejected("Saved native bot lacks its original client/actor".to_string())
            })?;
            let state = clients
                .iter()
                .find(|state| state.slot == entry.client.slot)
                .ok_or_else(|| {
                    ApplicationBotError::RestorationRejected(
                        "Saved native bot lacks its original client/actor".to_string(),
                    )
                })?;
            if !self
                .simulation
                .bot_movement(actor.id())
                .is_some_and(|movement| movement.client == *client.id())
            {
                return Err(ApplicationBotError::RestorationRejected(
                    "Saved native bot actor belongs to a different client".to_string(),
                ));
            }
            let reliable = restored_bot_reliable_commands(&entry.reliable)?;
            let connection = self.connect(
                ApplicationBotClient {
                    client,
                    reliable,
                    userinfo: None,
                },
                state.name.clone(),
                state.skill.clone(),
                Some(actor),
            )?;
            connection.sequence = state.sequence;
            *connection.selection.borrow_mut() = state.selection;
            connection
                .behavior
                .restore_json(&state.behavior)
                .map_err(ApplicationBotError::RestorationRejected)?;
        }
        self.shared.borrow_mut().elapsed_ms = image.transport.elapsed_milliseconds;
        Ok(())
    }
}

/// Restored director client row (donor `restore` client record).
struct RestoredDirectorClient {
    /// Client slot.
    slot: u32,
    /// Bot name.
    name: String,
    /// Skill name.
    skill: String,
    /// Command sequence.
    sequence: u64,
    /// Selected weapon number.
    selection: i32,
    /// Behavior image.
    behavior: SaveJson,
}

/// Read a required string member.
fn get_str(value: &SaveJson, key: &str) -> Result<String, ApplicationBotError> {
    match value.get(key) {
        Some(SaveJson::String(text)) => Ok(text.clone()),
        _ => Err(ApplicationBotError::RestorationRejected(format!(
            "Saved native bot population lacks {key}"
        ))),
    }
}

/// Read a required finite member.
fn get_finite(value: &SaveJson, key: &str) -> Result<f64, ApplicationBotError> {
    match value.get(key) {
        Some(SaveJson::Number(number)) if number.is_finite() => Ok(*number),
        _ => Err(ApplicationBotError::RestorationRejected(format!(
            "Saved native bot population lacks {key}"
        ))),
    }
}

/// Read a required integer member.
fn get_int(value: &SaveJson, key: &str) -> Result<i64, ApplicationBotError> {
    match value.get(key) {
        Some(SaveJson::Number(number)) if number.fract() == 0.0 => Ok(*number as i64),
        _ => Err(ApplicationBotError::RestorationRejected(format!(
            "Saved native bot population lacks {key}"
        ))),
    }
}

/// Read a required integer member with a minimum.
fn get_int_min(value: &SaveJson, key: &str, minimum: i64) -> Result<i64, ApplicationBotError> {
    let number = get_int(value, key)?;
    if number < minimum {
        return Err(ApplicationBotError::RestorationRejected(format!(
            "Saved native bot population lacks {key}"
        )));
    }
    Ok(number)
}

/// Read a required object member.
fn get_object<'a>(value: &'a SaveJson, key: &str) -> Result<&'a SaveJson, ApplicationBotError> {
    match value.get(key) {
        Some(object @ SaveJson::Object(_)) => Ok(object),
        _ => Err(ApplicationBotError::RestorationRejected(format!(
            "Saved native bot population lacks {key}"
        ))),
    }
}

/// Read a required list member.
fn get_list<'a>(value: &'a SaveJson, key: &str) -> Result<&'a [SaveJson], ApplicationBotError> {
    match value.get(key) {
        Some(SaveJson::Array(items)) => Ok(items),
        _ => Err(ApplicationBotError::RestorationRejected(format!(
            "Saved native bot population lacks {key}"
        ))),
    }
}

impl<S, E, N> ApplicationBotService for ApplicationRereleaseBots<S, E, N>
where
    S: RereleaseBotSimulation + RereleaseBotWorldSimulation + BotObjectiveSimulation + 'static,
    E: RereleaseBotSession + 'static,
    N: RereleaseBotNav + 'static,
{
    fn configuration(&self) -> Rc<RefCell<qa_core::cvar::CvarRegistry>> {
        Rc::clone(&self.configuration)
    }

    fn automatic_frame(&self) -> bool {
        self.automatic_frame
    }

    fn route_for_client(
        &self,
        _client: i32,
        _start: Vec3,
        _goal: Vec3,
    ) -> Result<qa_bots::types::NavigationRouteResult, BotsError> {
        // Rerelease navigation plans through the behavior nav graph, which
        // has no generic route query; the donor service has no equivalent.
        Err(BotsError::Internal(
            "rerelease bots plan routes through the behavior nav graph".to_string(),
        ))
    }

    fn mover_client_slot(&self, actor: &ActorId) -> Option<i32> {
        self.simulation
            .bot_movement(actor)
            .map(|movement| movement.client.slot() as i32)
    }

    fn move_to_point(&mut self, actor: &ActorId, point: Vec3, tolerance: f64) -> RereleaseGoalStatus {
        let origin = self.simulation.bot_body_origin(actor);
        let connection = self
            .connections
            .values_mut()
            .find(|connection| connection.bot.borrow().actor.id() == actor);
        let (Some(connection), Some(home)) = (connection, origin) else {
            return 0;
        };
        let before = connection.behavior.goal_status();
        connection.behavior.request_move_to_point(point);
        if ((f64::from(home.x) - f64::from(point.x)).powi(2)
            + (f64::from(home.y) - f64::from(point.y)).powi(2)
            + (f64::from(home.z) - f64::from(point.z)).powi(2))
        .sqrt()
            <= tolerance
        {
            return 3;
        }
        let status = connection.behavior.goal_status();
        if status == 0 {
            0
        } else if status == 1 {
            3
        } else if before == 0 {
            1
        } else {
            2
        }
    }

    fn follow_actor(&mut self, actor: &ActorId, target: &ActorId) -> RereleaseGoalStatus {
        let origin = self.simulation.bot_body_origin(target);
        let target_id = self.identify(target);
        let connection = self
            .connections
            .values_mut()
            .find(|connection| connection.bot.borrow().actor.id() == actor);
        let (Some(connection), Some(home)) = (connection, origin) else {
            return 0;
        };
        let before = connection.behavior.goal_status();
        connection.behavior.request_follow_entity(target_id, home);
        let status = connection.behavior.goal_status();
        if status == 0 {
            0
        } else if status == 1 {
            3
        } else if before == 0 {
            1
        } else {
            2
        }
    }

    fn is_bot(&self, actor: &ActorId) -> bool {
        self.shared.borrow().bot_actors.contains(actor)
    }

    fn service_actor(&self, client: &ClientId) -> Option<ActorId> {
        self.connections.get(&client.slot()).and_then(|connection| {
            let bot = connection.bot.borrow();
            (bot.client.id() == client).then(|| bot.actor.id().clone())
        })
    }

    fn frame(
        &mut self,
        _time_milliseconds: f64,
        elapsed_milliseconds: f64,
    ) -> Result<Vec<ActorCommand>, ApplicationBotError> {
        if self.closed {
            return Err(ApplicationBotError::Closed);
        }
        self.shared.borrow_mut().elapsed_ms = elapsed_milliseconds;
        let minimum = self.simulation.bot_max_clients().min(
            self.configuration
                .borrow()
                .variable_value("bot_minplayers")
                .trunc()
                .max(0.0) as u32,
        );
        if self.configuration.borrow().variable_value("bot_enable") != 0.0
            && self.simulation.bot_client_identities().len() < minimum as usize
        {
            let characters = &self.knowledge.characters;
            let name = characters
                .get(self.connections.len() % characters.len().max(1))
                .map(|character| character.name.clone())
                .unwrap_or_else(|| "Bot".to_string());
            self.add(name, self.default_skill()?)?;
        }
        let mut commands = Vec::new();
        let mut slots: Vec<u32> = self.connections.keys().copied().collect();
        slots.sort_unstable();
        for slot in slots {
            let actor = self.connections[&slot].bot.borrow().actor.id().clone();
            let live = self.simulation.bot_movement(&actor).is_some() && self.simulation.bot_is_live(&actor);
            if !live {
                return Err(ApplicationBotError::Admission(
                    "Native bot command targets a retired actor".to_string(),
                ));
            }
            let source = {
                let connection = self.connections.get_mut(&slot).expect("live bot slot");
                connection.behavior.set_game_mode(self.objectives.mode());
                let objective = self.objectives.goal(&actor);
                connection.behavior.set_objective_goal(objective);
                connection.behavior.think(&mut connection.world)
            };
            let command = self.translate(&source, &actor)?;
            let connection = self.connections.get_mut(&slot).expect("live bot slot");
            let selection = *connection.selection.borrow();
            let client = connection.bot.borrow().client.id().clone();
            let weapon = native_weapon_item(&self.simulation, &actor, &self.knowledge, selection);
            let provider = self
                .simulation
                .bot_movement(&actor)
                .map(|movement| movement.arsenal_provider)
                .unwrap_or_default();
            let sequence = connection.sequence;
            connection.sequence += 1;
            commands.push(ActorCommand {
                actor,
                source: CommandSource::Bot { client },
                sequence,
                command,
                arsenal: Some(ArsenalIntent {
                    provider,
                    weapon,
                    use_holdable: false,
                }),
            });
        }
        self.shared.borrow_mut().heard.clear();
        Ok(commands)
    }

    fn receive(&mut self, events: &[SimulationPresentationEvent]) {
        use super::types::SourcePresentationEvent;
        for event in events {
            match &event.event {
                SourcePresentationEvent::Q1(qa_content::q1::foundation::types::Q1Event::Sound {
                    origin,
                    actor,
                    volume,
                    ..
                }) => {
                    if let Some(home) = origin.or_else(|| self.simulation.bot_body_origin(actor)) {
                        let source_id = self.identify(actor);
                        self.shared.borrow_mut().heard.push(BotSoundT {
                            origin: home,
                            source_id,
                            time: event.seconds as f32,
                            loudness: *volume as f32,
                        });
                    }
                }
                SourcePresentationEvent::Q2(Q2PresentationEvent::Sound(sound)) => {
                    let source_id = sound.actor.as_ref().map(|actor| self.identify(actor)).unwrap_or(-1);
                    self.shared.borrow_mut().heard.push(BotSoundT {
                        origin: sound.origin,
                        source_id,
                        time: event.seconds as f32,
                        loudness: sound.volume as f32,
                    });
                }
                _ => {}
            }
        }
    }

    fn clients(&self) -> Vec<BotClientSnapshot> {
        let mut slots: Vec<u32> = self.connections.keys().copied().collect();
        slots.sort_unstable();
        slots
            .iter()
            .map(|slot| {
                let connection = &self.connections[slot];
                BotClientSnapshot {
                    connection: Rc::clone(&connection.bot),
                    userinfo: connection.userinfo.clone(),
                }
            })
            .collect()
    }

    fn console_command(&mut self, argv: &[String]) -> Result<(), ApplicationBotError> {
        let command = argv.first().map(|arg| arg.to_lowercase());
        if command.as_deref() == Some("addbot") {
            let characters = &self.knowledge.characters;
            let name = argv
                .get(1)
                .cloned()
                .or_else(|| {
                    characters
                        .get(self.connections.len() % characters.len().max(1))
                        .map(|character| character.name.clone())
                })
                .unwrap_or_else(|| "Bot".to_string());
            let raw = argv.get(2).cloned();
            let numeric = raw.as_ref().map(|raw| {
                if raw.trim().is_empty() {
                    0.0
                } else {
                    raw.parse::<f64>().unwrap_or(f64::NAN)
                }
            });
            let skill = match numeric {
                Some(number) if number.is_finite() => {
                    let index = (number.trunc() as i32)
                        .max(0)
                        .min(self.knowledge.skills.len() as i32 - 1)
                        .max(0) as usize;
                    match self.knowledge.skills.get(index) {
                        Some(skill) => skill.skill.clone(),
                        None => self.default_skill()?,
                    }
                }
                _ => match raw {
                    Some(raw) => raw,
                    None => self.default_skill()?,
                },
            };
            self.add(name, skill)?;
        } else if command.as_deref() == Some("botlist") {
            for character in &self.knowledge.characters {
                (self.print)(&format!("{}\n", character.name));
            }
        } else {
            return Err(ApplicationBotError::Admission(format!(
                "Unknown native bot command {}",
                command.as_deref().unwrap_or("undefined")
            )));
        }
        Ok(())
    }

    fn disconnect(&mut self, client: i32) -> bool {
        let slot = u32::try_from(client)
            .ok()
            .and_then(|slot| self.connections.remove(&slot));
        match slot {
            Some(connection) => {
                let bot = connection.bot.borrow();
                let actor = bot.actor.id().clone();
                let id = bot.client.id().clone();
                drop(bot);
                self.simulation.bot_disconnect_player(&actor);
                self.shared.borrow_mut().bot_actors.remove(&actor);
                self.session.close_bot_client(&id);
                true
            }
            None => false,
        }
    }

    fn close(&mut self, _restart: bool) {
        if self.closed {
            return;
        }
        self.simulation.detach_bot_transport();
        self.connections.clear();
        self.shared.borrow_mut().bot_actors.clear();
        self.closed = true;
    }

    fn checkpoint(&self) -> Result<SaveJson, ApplicationBotError> {
        self.capture()
    }

    fn begin_round_restart(&mut self) -> Result<Vec<BotClientSnapshot>, ApplicationBotError> {
        Err(ApplicationBotError::RestorationRejected(
            "Native rerelease bots use shared map replacement, not Q3 fast restart".to_string(),
        ))
    }

    fn bind_restarted_round(&mut self) -> Result<(), ApplicationBotError> {
        Err(ApplicationBotError::RestorationRejected(
            "Native rerelease bots cannot bind a Q3 round".to_string(),
        ))
    }

    fn reconnect_restarted_client(&mut self, _client: &ClientId) -> Result<bool, ApplicationBotError> {
        Err(ApplicationBotError::RestorationRejected(
            "Native rerelease bots cannot reconnect a Q3 round".to_string(),
        ))
    }

    fn resume_round_bots(&mut self) -> Result<(), ApplicationBotError> {
        Err(ApplicationBotError::RestorationRejected(
            "Native rerelease bots cannot resume a Q3 round".to_string(),
        ))
    }

    fn director_remove_queued_begin(&mut self, _client: i32) {
        // Rerelease bots have no director backing; nothing to remove.
    }

    fn director_interbreed_end_match(&mut self) {
        // Rerelease bots have no director backing; nothing to interbreed.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::behavior::rerelease::brain::BotGoalStatus;
    use qa_bots::behavior::rerelease::data::botdata::{BotSkillSettings, BotSourceEntry, ChatEntry};
    use qa_bots::behavior::rerelease::nav::{BotTransportStep, NavGraphLinkT, NavGraphNodeT, NavPathT, NavPlanOptions};
    use qa_bots::behavior::rerelease::world::{empty_usercmd, BotEntityT, BotSelfT, BotSoundT, BotTraceT};
    use qa_client::text::localization::LocalizationProfile;
    use qa_core::cmd::Dialect;
    use qa_core::cvar::CvarRegistry;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::numeric::Q1_DONOR_PROFILE;

    use super::super::bot_objectives::{BotMatchKind, BotSimulationMode, BotSourceObjective};
    use super::super::bot_rerelease_world::{
        RereleaseBotBody, RereleaseBotCombat, RereleaseBotEntity, RereleaseBotPlayer, RereleaseBotTrace, RereleaseBotUi,
    };

    #[derive(Default)]
    struct StubInner {
        players: Vec<ActorId>,
        movement: HashMap<ActorId, BotMovementView>,
        origins: HashMap<ActorId, Vec3>,
        identities: Vec<ClientId>,
        configuration: Option<Rc<RefCell<CvarRegistry>>>,
        skill: i32,
        max_clients: u32,
        next_actor: u32,
        attached: bool,
    }

    #[derive(Clone)]
    struct StubSim {
        inner: Rc<RefCell<StubInner>>,
        owner: Rc<IdentityOwner>,
    }

    impl StubSim {
        fn new(owner: Rc<IdentityOwner>) -> Self {
            Self {
                inner: Rc::new(RefCell::new(StubInner {
                    max_clients: 4,
                    ..StubInner::default()
                })),
                owner,
            }
        }
    }

    impl RereleaseBotSimulation for StubSim {
        fn bot_configuration(&self) -> Option<Rc<RefCell<qa_core::cvar::CvarRegistry>>> {
            self.inner.borrow().configuration.clone()
        }

        fn bot_skill(&self) -> i32 {
            self.inner.borrow().skill
        }

        fn bot_max_clients(&self) -> u32 {
            self.inner.borrow().max_clients
        }

        fn bot_players(&self) -> Vec<ActorId> {
            self.inner.borrow().players.clone()
        }

        fn bot_movement(&self, actor: &ActorId) -> Option<BotMovementView> {
            self.inner.borrow().movement.get(actor).cloned()
        }

        fn bot_time_seconds(&self) -> f64 {
            1.0
        }

        fn bot_gravity(&self) -> f64 {
            800.0
        }

        fn bot_random_integer(&self) -> i32 {
            42
        }

        fn bot_body_origin(&self, actor: &ActorId) -> Option<Vec3> {
            self.inner.borrow().origins.get(actor).copied()
        }

        fn bot_client_identities(&self) -> Vec<ClientId> {
            self.inner.borrow().identities.clone()
        }

        fn bot_resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owner.owned_actor(actor, ProviderId::new("q1", "test")).ok()
        }

        fn bot_reference_saved(&self, saved: SavedActorId) -> ActorId {
            self.owner.actor(saved.slot, saved.generation)
        }

        fn bot_resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
            let actor = self.owner.actor(saved.slot, saved.generation);
            self.owner.owned_actor(&actor, ProviderId::new("q1", "test")).ok()
        }

        fn bot_is_live(&self, actor: &ActorId) -> bool {
            self.inner.borrow().players.contains(actor)
        }

        fn bot_admit_player(&self, client: &ClientId) -> Result<ActorId, String> {
            let mut inner = self.inner.borrow_mut();
            let slot = inner.next_actor;
            inner.next_actor += 1;
            let actor = self.owner.actor(100 + slot, 0);
            inner.players.push(actor.clone());
            inner.movement.insert(
                actor.clone(),
                BotMovementView {
                    client: client.clone(),
                    profile_kind: "q1-rerelease".to_string(),
                    standing_bounds: Bounds {
                        min: Vec3 {
                            x: -16.0,
                            y: -16.0,
                            z: -24.0,
                        },
                        max: Vec3 {
                            x: 16.0,
                            y: 16.0,
                            z: 32.0,
                        },
                    },
                    arsenal_provider: "q1:base".to_string(),
                },
            );
            inner.origins.insert(actor.clone(), Vec3::default());
            if !inner.identities.contains(client) {
                inner.identities.push(client.clone());
            }
            Ok(actor)
        }

        fn bot_disconnect_player(&self, actor: &ActorId) {
            let mut inner = self.inner.borrow_mut();
            inner.players.retain(|player| player != actor);
            inner.movement.remove(actor);
        }

        fn bot_player_command(&self, _actor: &ActorId, _name: &str, _args: &[String]) {}

        fn bot_set_player_identity(&self, _actor: &ActorId, _identity: &BotPlayerIdentity) {}

        fn bot_notify_client_event(&self, _kind: &str, _actor: &ActorId) {}

        fn attach_bot_transport(&self, _transport: Rc<RefCell<dyn ApplicationBotService>>) {
            self.inner.borrow_mut().attached = true;
        }

        fn detach_bot_transport(&self) {
            self.inner.borrow_mut().attached = false;
        }
    }

    impl RereleaseBotWorldSimulation for StubSim {
        fn bot_numeric(&self) -> Option<qa_core::numeric::NumericProfile> {
            Some(Q1_DONOR_PROFILE)
        }

        fn time_seconds(&self) -> f32 {
            1.0
        }

        fn movement_player(&self, actor: &ActorId) -> Option<RereleaseBotPlayer> {
            self.inner
                .borrow()
                .movement
                .get(actor)
                .map(|movement| RereleaseBotPlayer {
                    client_slot: movement.client.slot(),
                    view_angles: Vec3::default(),
                    view_height: 22.0,
                    water_level: 0,
                })
        }

        fn body_read(&self, _actor: &ActorId) -> Option<RereleaseBotBody> {
            None
        }

        fn bodies_linked(&self, _actor: &ActorId) -> bool {
            false
        }

        fn player_ui(&self, _actor: &ActorId) -> RereleaseBotUi {
            RereleaseBotUi {
                items: Vec::new(),
                active_weapon: None,
                inventory: Vec::new(),
                health: 100.0,
                armor_points: 0.0,
            }
        }

        fn bot_entity(&self, _actor: &ActorId) -> Option<RereleaseBotEntity> {
            None
        }

        fn combat_read(&self, _actor: &ActorId) -> Option<RereleaseBotCombat> {
            None
        }

        fn inventory_count(&self, _actor: &ActorId, _item: &str) -> i32 {
            0
        }

        fn actor_observations(&self) -> Vec<ActorId> {
            Vec::new()
        }

        fn bot_trace(
            &self,
            _start: Vec3,
            end: Vec3,
            _bounds: Option<Bounds>,
            _pass_actor: &ActorId,
        ) -> RereleaseBotTrace {
            RereleaseBotTrace {
                fraction: 1.0,
                end,
                start_solid: false,
                hit_actor: None,
            }
        }

        fn bot_point_contents(&self, _point: Vec3, _pass_actor: &ActorId) -> Option<i32> {
            None
        }
    }

    impl BotObjectiveSimulation for StubSim {
        fn has_objective_registry(&self) -> bool {
            true
        }

        fn objective_cvar(&self, _name: &str) -> f64 {
            0.0
        }

        fn objective_players(&self) -> Vec<ActorId> {
            Vec::new()
        }

        fn player_team_command(&self, _actor: &ActorId, _team: &str) {}

        fn simulation_mode(&self) -> BotSimulationMode {
            BotSimulationMode::Deathmatch
        }

        fn has_movement_player(&self, _actor: &ActorId) -> bool {
            false
        }

        fn combat_team(&self, _actor: &ActorId) -> Option<String> {
            None
        }

        fn has_q1_ctf(&self) -> bool {
            false
        }

        fn q1_ctf_carried(&self, _actor: &ActorId) -> bool {
            false
        }

        fn match_kind(&self) -> BotMatchKind {
            BotMatchKind::Open
        }

        fn match_team(&self, _actor: &ActorId) -> i32 {
            0
        }

        fn deathball_skin(&self, _actor: &ActorId) -> Option<String> {
            None
        }

        fn deathball_teams(&self) -> Option<(String, String)> {
            None
        }

        fn deathball_ball(&self) -> Option<ActorId> {
            None
        }

        fn lmctf_flag_carried(&self, _actor: &ActorId) -> bool {
            false
        }

        fn tag_owner(&self) -> Option<ActorId> {
            None
        }

        fn source_objectives(&self) -> Vec<BotSourceObjective> {
            Vec::new()
        }

        fn body_origin(&self, _actor: &ActorId) -> Option<Vec3> {
            None
        }

        fn inventory_count(&self, _actor: &ActorId, _item: &str) -> i32 {
            0
        }

        fn rerelease_poi(&self) -> Option<Vec3> {
            None
        }
    }

    #[derive(Debug)]
    struct StubSession {
        owner: Rc<IdentityOwner>,
    }

    impl RereleaseBotSession for StubSession {
        fn create_bot_client(&mut self, slot: u32) -> Result<SessionClient, WorldError> {
            Ok(SessionClient::new(self.owner.client(slot, 0)))
        }

        fn close_bot_client(&mut self, _client: &ClientId) {}
    }

    #[derive(Debug)]
    struct StubGraph;

    impl RereleaseNavigation for StubGraph {
        fn node_count(&self) -> usize {
            0
        }

        fn nodes(&self) -> Vec<NavGraphNodeT> {
            Vec::new()
        }

        fn plan_path(&mut self, _start: Vec3, _goal: Vec3, _options: &NavPlanOptions) -> Option<NavPathT> {
            None
        }

        fn path_valid(&mut self, _path: &NavPathT) -> bool {
            false
        }

        fn transport(&mut self, _link: &NavGraphLinkT, _origin: Vec3) -> Option<BotTransportStep> {
            None
        }
    }

    #[derive(Debug)]
    struct StubNav;

    impl RereleaseBotNav for StubNav {
        fn nav_checkpoint_json(&self) -> SaveJson {
            obj(vec![("stub", str("nav"))])
        }

        fn nav_restore_json(&mut self, _image: &SaveJson) -> Result<(), String> {
            Ok(())
        }

        fn nav_for_client(&self, _slot: u32) -> Box<dyn RereleaseNavigation> {
            Box::new(StubGraph)
        }
    }

    #[derive(Debug, Default)]
    struct StubBehavior;

    impl RereleaseBotBehavior for StubBehavior {
        fn think(&mut self, _world: &mut dyn BotWorldT) -> BotUsercmdT {
            empty_usercmd()
        }

        fn set_game_mode(&mut self, _mode: BotGameModeT) {}

        fn set_objective_goal(&mut self, _goal: Option<Vec3>) {}

        fn goal_status(&self) -> i32 {
            0
        }

        fn request_move_to_point(&mut self, _point: Vec3) {}

        fn request_follow_entity(&mut self, _id: i32, _origin: Vec3) {}

        fn checkpoint_json(&self) -> SaveJson {
            obj(vec![("stub", str("behavior"))])
        }

        fn restore_json(&mut self, _image: &SaveJson) -> Result<(), String> {
            Ok(())
        }
    }

    fn knowledge() -> BotKnowledge {
        let mut knowledge = BotKnowledge::default();
        knowledge.characters = vec![CharacterEntry {
            name: "Crash".to_string(),
            ..CharacterEntry::default()
        }];
        knowledge.skills = vec![BotSkillSettings {
            skill: "novice".to_string(),
            ..BotSkillSettings::default()
        }];
        knowledge
    }

    fn assets() -> ApplicationBotAssets {
        ApplicationBotAssets::Rerelease {
            source: RereleaseBotSource::Q1,
            files: BotAssetFiles::default(),
            knowledge: Box::new(knowledge()),
            localization: LocalizationTable::new(LocalizationProfile::default()),
        }
    }

    type Harness = (
        ApplicationRereleaseBotsHandle<StubSim, StubSession, StubNav>,
        Rc<IdentityOwner>,
        StubSim,
        Rc<RefCell<Vec<String>>>,
    );

    fn harness() -> Harness {
        let owner = Rc::new(IdentityOwner::create("rerelease-test").expect("owner"));
        let sim = StubSim::new(Rc::clone(&owner));
        sim.inner.borrow_mut().configuration = Some(Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q1Netquake))));
        let printed = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&printed);
        let transport = create_application_bots(
            ApplicationRereleaseBotsOptions {
                session: StubSession {
                    owner: Rc::clone(&owner),
                },
                simulation: sim.clone(),
                navigation: StubNav,
                configuration: None,
                restore: None,
                clients: Vec::new(),
                automatic_frame: true,
                behavior_factory: Some(Rc::new(|_params| {
                    Ok(Box::new(StubBehavior) as Box<dyn RereleaseBotBehavior>)
                })),
                print: Rc::new(move |line| sink.borrow_mut().push(line.to_string())),
            },
            assets(),
        )
        .expect("transport builds");
        (transport, owner, sim, printed)
    }

    #[test]
    fn default_skill_reads_simulation_index() {
        let (transport, _, _, _) = harness();
        assert_eq!(transport.borrow().default_skill().expect("skill"), "novice");
    }

    #[test]
    fn names_prefer_userinfo_then_roster() {
        let (transport, owner, _, _) = harness();
        let host = transport.borrow();
        let saved = ApplicationBotClient {
            client: SessionClient::new(owner.client(0, 0)),
            reliable: ServerReliableCommands::new(),
            userinfo: Some("\\name\\Zed\\skin\\base".to_string()),
        };
        assert_eq!(host.name(&saved), "Zed");
        let saved = ApplicationBotClient {
            client: SessionClient::new(owner.client(1, 0)),
            reliable: ServerReliableCommands::new(),
            userinfo: None,
        };
        assert_eq!(host.name(&saved), "Crash");
    }

    #[test]
    fn addbot_admits_lists_and_disconnects() {
        let (transport, _, _, printed) = harness();
        transport
            .borrow_mut()
            .console_command(&["addbot".to_string()])
            .expect("addbot");
        assert_eq!(transport.borrow().clients().len(), 1);
        transport
            .borrow_mut()
            .console_command(&["botlist".to_string()])
            .expect("botlist");
        assert_eq!(printed.borrow().as_slice(), &["Crash\n".to_string()]);
        assert!(transport
            .borrow_mut()
            .console_command(&["frobnicate".to_string()])
            .is_err());
        assert!(transport.borrow_mut().disconnect(0));
        assert!(transport.borrow().clients().is_empty());
    }

    #[test]
    fn identify_numbers_are_stable() {
        let (transport, owner, _, _) = harness();
        let host = transport.borrow();
        let first = owner.actor(1, 0);
        let second = owner.actor(2, 0);
        assert_eq!(host.identify(&first), 1);
        assert_eq!(host.identify(&second), 2);
        assert_eq!(host.identify(&first), 1);
    }

    #[test]
    fn checkpoint_captures_population() {
        let (transport, _, _, _) = harness();
        transport
            .borrow_mut()
            .console_command(&["addbot".to_string()])
            .expect("addbot");
        let image = transport.borrow_mut().checkpoint().expect("checkpoint");
        let director = image.get("director").expect("director");
        assert_eq!(director.get("kind"), Some(&SaveJson::String("rerelease".to_string())));
        let SaveJson::Array(clients) = director.get("clients").expect("clients") else {
            panic!("clients array");
        };
        assert_eq!(clients.len(), 1);
        let transport_image = image.get("transport").expect("transport");
        let SaveJson::Array(connections) = transport_image.get("connections").expect("connections") else {
            panic!("connections array");
        };
        assert_eq!(connections.len(), 1);
    }

    #[test]
    fn restore_round_trip_preserves_sequence() {
        let (transport, owner, sim, _) = harness();
        transport
            .borrow_mut()
            .console_command(&["addbot".to_string()])
            .expect("addbot");
        transport.borrow_mut().frame(0.0, 16.0).expect("frame");
        let image = transport.borrow_mut().checkpoint().expect("checkpoint");
        let restore_owner = Rc::clone(&owner);
        let second = create_application_bots(
            ApplicationRereleaseBotsOptions {
                session: StubSession {
                    owner: Rc::clone(&owner),
                },
                simulation: sim.clone(),
                navigation: StubNav,
                configuration: None,
                restore: Some(ApplicationBotsRestore {
                    image: super::super::bots::decode_application_bots_checkpoint(&image).expect("decode"),
                    resolve_client: Rc::new(move |saved| {
                        Some(SessionClient::new(restore_owner.client(saved.slot, saved.generation)))
                    }),
                }),
                clients: Vec::new(),
                automatic_frame: true,
                behavior_factory: Some(Rc::new(|_params| {
                    Ok(Box::new(StubBehavior) as Box<dyn RereleaseBotBehavior>)
                })),
                print: Rc::new(|_| {}),
            },
            assets(),
        )
        .expect("restore builds");
        assert_eq!(second.borrow().clients().len(), 1);
        let again = second.borrow_mut().checkpoint().expect("checkpoint");
        let director = again.get("director").expect("director");
        let SaveJson::Array(clients) = director.get("clients").expect("clients") else {
            panic!("clients array");
        };
        assert_eq!(clients[0].get("sequence"), Some(&SaveJson::Number(1.0)));
    }

    /// Recorded chat delivery (locstring, team-only).
    struct BehaviorProbes {
        /// Behavior clock in seconds.
        clock: Rc<RefCell<f64>>,
        /// Pre-think calls.
        pre: Rc<RefCell<u32>>,
        /// Post-think calls.
        post: Rc<RefCell<u32>>,
        /// Delivered chats.
        chats: Rc<RefCell<Vec<(String, bool)>>>,
        /// Selected weapons.
        selected: Rc<RefCell<Vec<i32>>>,
        /// Weapon impulses.
        impulses: Rc<RefCell<Vec<i32>>>,
        /// Chat delay milliseconds.
        delay_ms: f64,
    }

    impl BehaviorProbes {
        /// Probes over a 100-second clock.
        fn new() -> Self {
            Self {
                clock: Rc::new(RefCell::new(100.0)),
                pre: Rc::new(RefCell::new(0)),
                post: Rc::new(RefCell::new(0)),
                chats: Rc::new(RefCell::new(Vec::new())),
                selected: Rc::new(RefCell::new(Vec::new())),
                impulses: Rc::new(RefCell::new(Vec::new())),
                delay_ms: 0.0,
            }
        }

        /// Behavior callbacks recording into the probes. The chat sink draws
        /// from the lent RNG like the transport `chats.txt` lookup.
        fn callbacks(&self) -> RereleaseBehaviorCallbacks {
            let clock = Rc::clone(&self.clock);
            let pre = Rc::clone(&self.pre);
            let post = Rc::clone(&self.post);
            let chats = Rc::clone(&self.chats);
            let selected = Rc::clone(&self.selected);
            let impulses = Rc::clone(&self.impulses);
            RereleaseBehaviorCallbacks {
                time: Rc::new(move || *clock.borrow()),
                pre_think: Rc::new(move || *pre.borrow_mut() += 1),
                post_think: Rc::new(move || *post.borrow_mut() += 1),
                chat: Rc::new(move |event, random| {
                    random.next();
                    chats.borrow_mut().push((event.locstring, event.team_only));
                }),
                select_weapon: Rc::new(move |number| selected.borrow_mut().push(number)),
                weapon_impulse: Rc::new(move |number| {
                    impulses.borrow_mut().push(number);
                    0
                }),
                human_teammate_near: Rc::new(|| false),
            }
        }

        /// Driver parameters over the shared knowledge fixture plus two
        /// certain chats (`connected`, `match_start`).
        fn params(&self, source: RereleaseBotSource) -> RereleaseBehaviorParams {
            let mut fixture = knowledge();
            let delay_ms = self.delay_ms;
            fixture.chats = vec![
                ChatEntry {
                    base: BotSourceEntry {
                        source: None,
                        unknown: Vec::new(),
                    },
                    locstring: "hi".to_string(),
                    chat_type: "connected".to_string(),
                    time: delay_ms,
                    chance: 100.0,
                    team: false,
                },
                ChatEntry {
                    base: BotSourceEntry {
                        source: None,
                        unknown: Vec::new(),
                    },
                    locstring: "gl".to_string(),
                    chat_type: "match_start".to_string(),
                    time: delay_ms,
                    chance: 100.0,
                    team: true,
                },
            ];
            RereleaseBehaviorParams {
                definition: "test-definition".to_string(),
                source,
                knowledge: Rc::new(fixture),
                skill: "novice".to_string(),
                seed: 1234,
                game_mode: BotGameModeT {
                    game_type: "dm".to_string(),
                    weapon_stay: false,
                    has_teams: Some(true),
                    team_damage: Some(false),
                },
                character: None,
                max_health: 100.0,
                run_speed: 320.0,
                walk_speed: 160.0,
                movement: RereleaseBehaviorMovement {
                    gravity: 800.0,
                    jump_velocity: 270.0,
                    jump_air_seconds: 0.7,
                    maximum_landing_rise: 18.0,
                    start_above: 56.0,
                    body_mins: Vec3 {
                        x: -16.0,
                        y: -16.0,
                        z: -24.0,
                    },
                    body_maxs: Vec3 {
                        x: 16.0,
                        y: 16.0,
                        z: 32.0,
                    },
                },
                callbacks: self.callbacks(),
            }
        }
    }

    /// Empty behavior world: alive, grounded, clear traces, no entities.
    struct FakeBehaviorWorld {
        /// Server time seconds.
        time: f32,
    }

    impl BotWorldT for FakeBehaviorWorld {
        fn time(&self) -> f32 {
            self.time
        }

        fn frame_time(&self) -> f32 {
            0.05
        }

        fn bot_self(&self) -> BotSelfT {
            BotSelfT {
                id: 1,
                origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                view_angles: Vec3 {
                    x: 0.0,
                    y: 90.0,
                    z: 0.0,
                },
                eye: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 22.0,
                },
                health: 100.0,
                armor: 0.0,
                items: 0,
                ammo: HashMap::new(),
                current_weapon: 0,
                on_ground: true,
                water_level: 0,
                air_seconds: None,
                on_lift: None,
                team: 0,
                dead: false,
                has_protection: false,
                max_armor: None,
                carrying_objective: false,
            }
        }

        fn trace_line(&self, _start: Vec3, end: Vec3) -> BotTraceT {
            BotTraceT {
                fraction: 1.0,
                endpos: end,
                startsolid: false,
                hit_id: -1,
            }
        }

        fn trace_box(&self, _start: Vec3, _mins: Vec3, _maxs: Vec3, end: Vec3) -> BotTraceT {
            BotTraceT {
                fraction: 1.0,
                endpos: end,
                startsolid: false,
                hit_id: -1,
            }
        }

        fn point_contents(&self, _point: Vec3) -> i32 {
            0
        }

        fn entities(&self) -> Vec<BotEntityT> {
            Vec::new()
        }

        fn hearing(&self) -> Vec<BotSoundT> {
            Vec::new()
        }

        fn nav(&mut self) -> Option<&mut dyn RereleaseNavigation> {
            None
        }
    }

    /// Q1 drivers resolve weapon impulses; Q2 drivers select directly.
    #[test]
    fn driver_sources_select_their_weapon_path() {
        let probes = BehaviorProbes::new();
        let q1 = ApplicationRereleaseBehavior::new(probes.params(RereleaseBotSource::Q1)).expect("q1 builds");
        assert!(q1.brain.config.weapon_impulse.is_some());
        assert!(q1.brain.config.on_weapon_select.is_none());
        let q2 = ApplicationRereleaseBehavior::new(probes.params(RereleaseBotSource::Q2)).expect("q2 builds");
        assert!(q2.brain.config.weapon_impulse.is_none());
        assert!(q2.brain.config.on_weapon_select.is_some());
    }

    /// Think runs the hooks and delivers due chats through the lent RNG.
    #[test]
    fn think_runs_hooks_and_delivers_due_chats() {
        let probes = BehaviorProbes::new();
        let mut driver = create_rerelease_behavior(probes.params(RereleaseBotSource::Q1)).expect("builds");
        let mut world = FakeBehaviorWorld { time: 100.0 };
        let command = driver.think(&mut world);
        assert_eq!(*probes.pre.borrow(), 1);
        assert_eq!(*probes.post.borrow(), 1);
        assert_eq!(
            *probes.chats.borrow(),
            vec![("hi".to_string(), false), ("gl".to_string(), true)]
        );
        assert_eq!(command.view_angles.x, 0.0);
        assert_eq!(command.view_angles.y, 90.0);
    }

    /// Future chats wait until their delivery time.
    #[test]
    fn think_defers_future_chats_until_due() {
        let probes = BehaviorProbes {
            delay_ms: 5000.0,
            ..BehaviorProbes::new()
        };
        let mut driver = ApplicationRereleaseBehavior::new(probes.params(RereleaseBotSource::Q2)).expect("builds");
        let mut world = FakeBehaviorWorld { time: 100.0 };
        driver.think(&mut world);
        assert!(probes.chats.borrow().is_empty());
        *probes.clock.borrow_mut() = 106.0;
        driver.think(&mut world);
        assert_eq!(probes.chats.borrow().len(), 2);
    }

    /// Delegating methods drive the brain goal state.
    #[test]
    fn delegating_methods_drive_the_brain() {
        let probes = BehaviorProbes::new();
        let mut driver = ApplicationRereleaseBehavior::new(probes.params(RereleaseBotSource::Q1)).expect("builds");
        assert_eq!(driver.goal_status(), BotGoalStatus::ERROR);
        driver.request_move_to_point(Vec3 {
            x: 64.0,
            y: 0.0,
            z: 0.0,
        });
        assert_eq!(driver.goal_status(), BotGoalStatus::IN_PROGRESS);
        driver.request_follow_entity(
            7,
            Vec3 {
                x: 0.0,
                y: 64.0,
                z: 0.0,
            },
        );
        assert_eq!(driver.goal_status(), BotGoalStatus::IN_PROGRESS);
        driver.set_game_mode(BotGameModeT {
            game_type: "ctf".to_string(),
            weapon_stay: true,
            has_teams: Some(true),
            team_damage: Some(true),
        });
        let image = driver.checkpoint_json();
        let mode = image.get("brain").expect("brain").get("gameMode").expect("mode");
        assert_eq!(mode.get("gameType"), Some(&str("ctf")));
        driver.set_objective_goal(Some(Vec3 { x: 1.0, y: 2.0, z: 3.0 }));
        assert_eq!(driver.goal_status(), BotGoalStatus::IN_PROGRESS);
    }

    /// Checkpoint restores memory, RNG, and pending chats exactly.
    #[test]
    fn checkpoint_restores_memory_rng_and_pending_chats() {
        let probes = BehaviorProbes {
            delay_ms: 5000.0,
            ..BehaviorProbes::new()
        };
        let mut driver = ApplicationRereleaseBehavior::new(probes.params(RereleaseBotSource::Q1)).expect("builds");
        driver.request_move_to_point(Vec3 {
            x: 64.0,
            y: 0.0,
            z: 0.0,
        });
        let mut world = FakeBehaviorWorld { time: 100.0 };
        driver.think(&mut world);
        let before = driver.checkpoint_json();
        let SaveJson::Array(pending) = before.get("pendingChats").expect("pending") else {
            panic!("pending chats array");
        };
        assert_eq!(pending.len(), 2);
        driver.request_follow_entity(
            7,
            Vec3 {
                x: 0.0,
                y: 64.0,
                z: 0.0,
            },
        );
        driver.restore_json(&before).expect("restore");
        let after = driver.checkpoint_json();
        assert_eq!(before, after);
        *probes.clock.borrow_mut() = 200.0;
        driver.think(&mut world);
        assert_eq!(probes.chats.borrow().len(), 2);
    }

    /// Restore rejects foreign checkpoints with the donor message.
    #[test]
    fn restore_rejects_foreign_checkpoints() {
        let probes = BehaviorProbes::new();
        let mut driver = ApplicationRereleaseBehavior::new(probes.params(RereleaseBotSource::Q1)).expect("builds");
        let image = driver.checkpoint_json();
        for (key, value) in [
            ("version", int(2)),
            ("source", str("q2-rerelease")),
            ("definition", str("other-definition")),
        ] {
            let mut foreign = image.clone();
            let SaveJson::Object(members) = &mut foreign else {
                panic!("behavior object");
            };
            let slot = members.iter_mut().find(|(name, _)| name == key).expect("member");
            slot.1 = value;
            assert_eq!(
                driver.restore_json(&foreign),
                Err("Rerelease behavior checkpoint belongs to another mounted source definition".to_string())
            );
        }
    }

    /// The factory reports missing skill settings.
    #[test]
    fn factory_reports_missing_skill_settings() {
        let probes = BehaviorProbes::new();
        let mut params = probes.params(RereleaseBotSource::Q1);
        params.knowledge = Rc::new(BotKnowledge::default());
        let error = create_rerelease_behavior(params).expect_err("skill-less knowledge fails");
        assert!(error.contains("no skill settings"), "unexpected error: {error}");
    }

    /// The default factory builds the production driver (it validates
    /// like production instead of accepting unconditionally).
    #[test]
    fn default_factory_builds_production_behavior() {
        let probes = BehaviorProbes::new();
        let factory = default_behavior_factory();
        factory(probes.params(RereleaseBotSource::Q1)).expect("production builds");
        let mut params = probes.params(RereleaseBotSource::Q1);
        params.knowledge = Rc::new(BotKnowledge::default());
        let error = factory(params).expect_err("skill-less knowledge fails");
        assert!(error.contains("no skill settings"), "unexpected error: {error}");
    }

    /// Replace a nested object member by path.
    fn set_path(image: &mut SaveJson, path: &[&str], value: SaveJson) {
        let (key, rest) = path.split_first().expect("path");
        let SaveJson::Object(members) = image else {
            panic!("behavior object");
        };
        let slot = members.iter_mut().find(|(name, _)| name == key).expect("member");
        if rest.is_empty() {
            slot.1 = value;
        } else {
            set_path(&mut slot.1, rest, value);
        }
    }

    /// Populated maps, sets, paths, and goals survive the image.
    #[test]
    fn populated_memory_decodes_maps_paths_and_goals() {
        let probes = BehaviorProbes::new();
        let mut driver = ApplicationRereleaseBehavior::new(probes.params(RereleaseBotSource::Q1)).expect("builds");
        driver.request_move_to_point(Vec3 {
            x: 64.0,
            y: 0.0,
            z: 0.0,
        });
        let mut image = driver.checkpoint_json();
        let awareness = |id: i32| {
            obj(vec![
                ("key", int(i64::from(id))),
                (
                    "value",
                    encode_awareness(&BotAwarenessT {
                        id,
                        sight: 0.5,
                        weapon: 0.25,
                        last_contact: 91.0,
                        last_seen: 92.0,
                        last_heard: 93.0,
                        last_known_origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                    }),
                ),
            ])
        };
        set_path(
            &mut image,
            &["brain", "memory", "awareness"],
            arr(vec![awareness(3), awareness(1)]),
        );
        set_path(
            &mut image,
            &["brain", "memory", "unreachableUntil"],
            arr(vec![obj(vec![("key", int(5)), ("value", num(12.5))])]),
        );
        set_path(
            &mut image,
            &["brain", "memory", "objectiveHome"],
            arr(vec![obj(vec![
                ("key", int(2)),
                ("value", vec3_json(Vec3 { x: 4.0, y: 5.0, z: 6.0 })),
            ])]),
        );
        set_path(
            &mut image,
            &["brain", "memory", "saidThisLevel"],
            arr(vec![str("b"), str("a")]),
        );
        let path = NavPathT {
            nodes: vec![1, 2],
            points: vec![Vec3 { x: 7.0, y: 8.0, z: 9.0 }],
            links: vec![
                Some(NavGraphLinkT {
                    from: 1,
                    to: 2,
                    link_type: NavLinkType::Ladder,
                    traversal: Some(NavTraversalT {
                        funnel: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                        start: Vec3 { x: 2.0, y: 2.0, z: 2.0 },
                        end: Vec3 { x: 3.0, y: 3.0, z: 3.0 },
                    }),
                    entity_bounds: Some(NavEntityBounds {
                        mins: Vec3 {
                            x: -1.0,
                            y: -1.0,
                            z: -1.0,
                        },
                        maxs: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                    }),
                }),
                None,
            ],
            cost: 1.5,
            generation: 9,
            map_identity: "identity:0:0:0:0".to_string(),
        };
        set_path(
            &mut image,
            &["brain", "memory", "pathState", "path"],
            encode_path(&path),
        );
        driver.restore_json(&image).expect("restore");
        let again = driver.checkpoint_json();
        let memory = again.get("brain").expect("brain").get("memory").expect("memory");
        let SaveJson::Array(records) = memory.get("awareness").expect("awareness") else {
            panic!("awareness array");
        };
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].get("key"), Some(&int(1)));
        assert_eq!(records[1].get("key"), Some(&int(3)));
        let SaveJson::Array(said) = memory.get("saidThisLevel").expect("said") else {
            panic!("said array");
        };
        assert_eq!(said, &vec![str("a"), str("b")]);
        let stored = memory.get("pathState").expect("path state").get("path").expect("path");
        assert_eq!(stored, &encode_path(&path));
        let goal = memory.get("explicitGoal").expect("goal");
        assert_eq!(goal.get("kind"), Some(&str("point")));
        assert_eq!(goal.get("owner"), Some(&str("external")));
        driver.request_follow_entity(
            7,
            Vec3 {
                x: 0.0,
                y: 64.0,
                z: 0.0,
            },
        );
        let followed = driver.checkpoint_json();
        let goal = followed
            .get("brain")
            .expect("brain")
            .get("memory")
            .expect("memory")
            .get("explicitGoal")
            .expect("goal");
        assert_eq!(goal.get("kind"), Some(&str("entity")));
    }
}
