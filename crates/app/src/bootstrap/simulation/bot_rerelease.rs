//! Native rerelease bot transport over the shared simulation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/bot-rerelease.ts`.
//!
//! Missing siblings (host seams, implemented post-merge by their partitions):
//! - `simulation/runtime.ts` (`SharedSimulation`): [`RereleaseBotSimulation`].
//! - `world/session/session.ts` (`EngineSession`): [`RereleaseBotSession`].
//! - `simulation/navigation.ts` (`ApplicationBotNavigation`):
//!   [`RereleaseBotNav`]. The ported `RereleaseNavigation` graph and the
//!   generic `NavigationRuntime` have no bridge yet, so the rerelease-used
//!   surface (checkpoint, restore, per-client graph) is seamed directly.
//! - `bots/behavior/rerelease/profile.ts` (`RereleaseBotBehavior`):
//!   [`RereleaseBotBehavior`]. Only the checkpoint and chat-text pieces are
//!   ported (`qa_bots::behavior::rerelease`); the driver is seamed.
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
use qa_bots::behavior::rerelease::chat_text::q1_bot_chat_text;
use qa_bots::behavior::rerelease::data::botdata::CharacterEntry;
use qa_bots::behavior::rerelease::data::knowledge::{BotGameModeT, BotKnowledge};
use qa_bots::behavior::rerelease::nav::RereleaseNavigation;
use qa_bots::behavior::rerelease::rng::BotRandomT;
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
use qa_world::save::value::{arr, int, num, obj, str, SaveJson};
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
/// Seam over `RereleaseBotBehavior` from donor
/// `src/bots/behavior/rerelease/profile.ts` (canonical home:
/// `qa_bots::behavior::rerelease`); the partition implements it post-merge.
/// The checkpoint image format is owned by the seam: JSON in, JSON out.
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
    /// Behavior factory.
    pub behavior_factory: RereleaseBehaviorFactory,
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
            behavior_factory: options.behavior_factory,
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
    use qa_bots::behavior::rerelease::data::botdata::BotSkillSettings;
    use qa_bots::behavior::rerelease::nav::{BotTransportStep, NavGraphLinkT, NavGraphNodeT, NavPathT, NavPlanOptions};
    use qa_bots::behavior::rerelease::world::empty_usercmd;
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
                behavior_factory: Rc::new(|_params| Ok(Box::new(StubBehavior) as Box<dyn RereleaseBotBehavior>)),
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
                behavior_factory: Rc::new(|_params| Ok(Box::new(StubBehavior) as Box<dyn RereleaseBotBehavior>)),
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
}
