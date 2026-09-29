//! Source bot director from `src/bots/behavior/director.ts`.
//!
//! Source arena/AI scheduling generates commands for the existing
//! shared player pipeline. The director owns the game AI, bot
//! catalog, navigation, and BSP entities for one map; each source
//! frame it thinks every bot and encodes user commands into ordinary
//! client commands with per-actor sequences. Scripted orders
//! (move-to-point, follow-entity) override fuzzy goals per bot.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_world::client::ClientCommand;

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::library::bsp_entities::AasBspEntities;
use crate::behavior::library::genetic::Xorshift32;
use crate::behavior::orders::{
    bot_order_active, bot_order_status, same_bot_order, BotGoalStatus, BotOrder, BotOrderProgress, BotOrderState,
    BOT_GOAL_NONE,
};
use crate::behavior::population::BotActorCommand;
use crate::behavior::q3::ai_navigation::bot_clear_activate_goal_stack;
use crate::behavior::q3::ai_state::{AiNode, BotSettings, BotState, BotUserCommand};
use crate::behavior::q3::catalog::{GameBotCatalog, GameBotCatalogHost};
use crate::behavior::q3::game_host::SourceBotGame;
use crate::behavior::q3::library::BotLibrary;
use crate::behavior::q3::navigation_types::BotNavigation;
use crate::behavior::q3::{ai_main::GameAi, game_host::BotProduct};
use crate::error::BotsError;

/// Director host: session bindings owned by the application.
pub trait SourceBotDirectorHost {
    /// Allocate a session player and source client binding.
    fn allocate_client(&mut self) -> Option<i32>;
    /// Admitted actor and registry slot for a client.
    fn actor(&self, client: i32) -> Option<(ActorId, u32)>;
    /// Encode a brain user command into a client command.
    fn encode_command(&self, client: i32, command: &BotUserCommand) -> ClientCommand;
    /// Snapshot entity for a client sequence.
    fn snapshot_entity(&self, client: i32, sequence: i32) -> i32;
    /// Console message for a client.
    fn console_message(&self, client: i32) -> Option<String>;
    /// Point contents.
    fn point_contents(&self, point: Vec3) -> i32;
    /// Print engine text.
    fn print(&mut self, text: &str);
    /// Current time milliseconds.
    fn time_ms(&self) -> i32;
    /// Whether a name is in use.
    fn name_in_use(&self, name: &str) -> bool;
}

/// Roster entry: admitted actor, source client, settings snapshot.
#[derive(Debug, Clone)]
pub struct SourceBotRosterEntry {
    /// Admitted actor.
    pub actor: ActorId,
    /// Registry slot.
    pub slot: u32,
    /// Source client.
    pub source_client: i32,
    /// Bot settings.
    pub settings: BotSettings,
}

/// Director round phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundPhase {
    Active,
    Closed,
}

/// Source bot director.
pub struct SourceBotDirector<'a> {
    host: Box<dyn SourceBotDirectorHost + 'a>,
    game: Box<dyn SourceBotGame + 'a>,
    navigation: Box<dyn BotNavigation + 'a>,
    ai: GameAi<'a>,
    catalog: GameBotCatalog,
    /// Parsed BSP entities.
    pub bsp_entities: AasBspEntities,
    files: &'a dyn BotSourceFiles,
    sequences: HashMap<(u32, u32), u32>,
    random: Xorshift32,
    item_config: String,
    gametype: i32,
    loaded: bool,
    running: bool,
    phase: RoundPhase,
}

/// Constructor parameters for [`SourceBotDirector`].
pub struct SourceBotDirectorParams<'a> {
    pub host: Box<dyn SourceBotDirectorHost + 'a>,
    pub game: Box<dyn SourceBotGame + 'a>,
    pub navigation: Box<dyn BotNavigation + 'a>,
    pub files: &'a dyn BotSourceFiles,
    pub entities: String,
    pub item_config: String,
    pub gametype: i32,
    pub max_clients: i32,
    pub debug: bool,
}

impl<'a> SourceBotDirector<'a> {
    /// New director. `entities` is the BSP entity lump text.
    pub fn new(params: SourceBotDirectorParams<'a>) -> Result<Self, BotsError> {
        let SourceBotDirectorParams {
            host,
            game,
            navigation,
            files,
            entities,
            item_config,
            gametype,
            max_clients,
            debug,
        } = params;
        let mut bsp_entities = AasBspEntities::new();
        bsp_entities.load(&entities)?;
        let mut catalog = GameBotCatalog::new();
        catalog.load(files);
        Ok(Self {
            host,
            game,
            navigation,
            ai: GameAi::new(files, max_clients, debug),
            catalog,
            bsp_entities,
            files,
            sequences: HashMap::new(),
            random: Xorshift32::new(0x9e37_79b9),
            item_config,
            gametype,
            loaded: false,
            running: false,
            phase: RoundPhase::Active,
        })
    }

    /// Borrow the game AI.
    #[must_use]
    pub fn ai(&self) -> &GameAi<'a> {
        &self.ai
    }

    /// Borrow the catalog.
    #[must_use]
    pub fn catalog(&self) -> &GameBotCatalog {
        &self.catalog
    }

    /// Borrow the library.
    #[must_use]
    pub fn library(&self) -> &BotLibrary<'a> {
        &self.ai.library
    }

    /// Product.
    #[must_use]
    pub fn product(&self) -> BotProduct {
        self.game.product()
    }

    /// Load the map lifetime.
    pub fn load(&mut self) -> Result<(), BotsError> {
        if self.phase == RoundPhase::Closed || self.loaded {
            return Err(BotsError::BotLifetime(
                "bot director map lifetime is already loaded or closed".to_owned(),
            ));
        }
        if !self.navigation.ready() {
            return Err(BotsError::BotLifetime(
                "bot arena admission requires completed selected-movement navigation".to_owned(),
            ));
        }
        self.ai.setup(false)?;
        let item_config = self.item_config.clone();
        let gametype = self.gametype;
        self.ai.load_map(&item_config, gametype, false)?;
        self.loaded = true;
        Ok(())
    }

    /// Whether the director is loaded.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Run one source frame; returns encoded client commands.
    pub fn frame(&mut self, milliseconds: i32) -> Result<Vec<BotActorCommand>, BotsError> {
        if !self.loaded || self.phase == RoundPhase::Closed || self.running {
            return Err(BotsError::BotLifetime(
                "bot commands require a live idle director".to_owned(),
            ));
        }
        self.running = true;
        let result = self.frame_inner(milliseconds);
        self.running = false;
        result
    }

    fn frame_inner(&mut self, milliseconds: i32) -> Result<Vec<BotActorCommand>, BotsError> {
        // Expire follow orders whose entity generation changed.
        for client in 0..self.game.max_clients() {
            self.refresh_follow_order(client);
        }
        self.ai.start_frame(
            self.game.as_mut(),
            self.navigation.as_mut(),
            milliseconds,
            &mut self.random,
            || 0.5,
        )?;
        let mut commands = Vec::new();
        for (client, ucmd) in self.ai.drain_commands() {
            let Some((actor, slot)) = self.host.actor(client) else {
                return Err(BotsError::BotLifetime(format!(
                    "bot command has no admitted shared actor for client {client}"
                )));
            };
            let key = (actor.slot(), actor.generation());
            let sequence = self.sequences.get(&key).copied().unwrap_or(0);
            self.sequences.insert(key, sequence + 1);
            commands.push(BotActorCommand {
                actor,
                slot,
                sequence,
                command: self.host.encode_command(client, &ucmd),
            });
        }
        Ok(commands)
    }

    fn refresh_follow_order(&mut self, client: i32) {
        let state = self.ai.context.states.get(client);
        let order = state.and_then(|state| state.scripted_order);
        if let Some(BotOrderState {
            order: BotOrder::Follow { entity },
            progress,
        }) = order
        {
            if progress == BotOrderProgress::InProgress {
                let observed = self.game.entity(entity.number);
                if !observed.present || observed.generation != entity.generation {
                    if let Some(state) = self.ai.context.states.get_mut(client) {
                        state.scripted_order = Some(BotOrderState {
                            order: BotOrder::Follow { entity },
                            progress: BotOrderProgress::Error,
                        });
                    }
                }
            }
        }
    }

    /// Live roster.
    #[must_use]
    pub fn roster(&self) -> Vec<SourceBotRosterEntry> {
        let mut roster = Vec::new();
        for client in 0..self.game.max_clients() {
            let state = self.ai.context.states.get(client);
            let actor = self.host.actor(client);
            if let (Some(state), Some((actor, slot))) = (state, actor) {
                if state.inuse {
                    roster.push(SourceBotRosterEntry {
                        actor,
                        slot,
                        source_client: client,
                        settings: state.settings.clone(),
                    });
                }
            }
        }
        roster
    }

    /// Request a move-to-point goal.
    pub fn request_move_to_point(&mut self, client: i32, point: Vec3) -> BotGoalStatus {
        if ![point.x, point.y, point.z].iter().all(|v| v.is_finite()) {
            return BOT_GOAL_NONE;
        }
        self.set_order(client, BotOrder::Point { point })
    }

    /// Request a follow-entity goal.
    pub fn request_follow_entity(&mut self, client: i32, number: i32) -> BotGoalStatus {
        if number < 0 || number >= self.game.entity_count() {
            return BOT_GOAL_NONE;
        }
        let entity = self.game.entity(number);
        if !entity.present {
            return BOT_GOAL_NONE;
        }
        self.set_order(
            client,
            BotOrder::Follow {
                entity: crate::behavior::orders::BotOrderEntity {
                    number,
                    generation: entity.generation,
                },
            },
        )
    }

    /// Clear a client's scripted goal.
    pub fn clear_goal(&mut self, client: i32) {
        let active = self
            .ai
            .context
            .states
            .get(client)
            .and_then(|state| state.scripted_order)
            .is_some_and(|order| bot_order_active(Some(&order)));
        if active {
            bot_clear_activate_goal_stack(&mut self.ai.context, client);
            if let Some(state) = self.ai.context.states.get_mut(client) {
                if state.ai_node == Some(AiNode::SeekActivateEntity) {
                    state.ai_node = Some(AiNode::SeekLtg);
                }
                state.scripted_order = None;
                let ms = state.ms;
                self.ai.library.move_states.reset_avoid_reach(ms);
            }
        } else if let Some(state) = self.ai.context.states.get_mut(client) {
            state.scripted_order = None;
        }
    }

    /// Goal status for a client.
    pub fn goal_status(&mut self, client: i32) -> BotGoalStatus {
        self.refresh_follow_order(client);
        let order = self
            .ai
            .context
            .states
            .get(client)
            .and_then(|state| state.scripted_order);
        bot_order_status(order.as_ref())
    }

    fn order_state(&self, client: i32) -> Option<&BotState> {
        if !self.loaded || self.phase == RoundPhase::Closed || client < 0 || client >= self.game.max_clients() {
            return None;
        }
        let state = self.ai.context.states.get(client)?;
        if state.inuse && self.host.actor(client).is_some() {
            Some(state)
        } else {
            None
        }
    }

    fn set_order(&mut self, client: i32, order: BotOrder) -> BotGoalStatus {
        let Some(current) = self.order_state(client).and_then(|state| state.scripted_order) else {
            if self.order_state(client).is_none() {
                return BOT_GOAL_NONE;
            }
            return self.install_order(client, order);
        };
        if same_bot_order(&current.order, &order) {
            return self.goal_status(client);
        }
        self.install_order(client, order)
    }

    fn install_order(&mut self, client: i32, order: BotOrder) -> BotGoalStatus {
        bot_clear_activate_goal_stack(&mut self.ai.context, client);
        if let Some(state) = self.ai.context.states.get_mut(client) {
            if state.ai_node == Some(AiNode::SeekActivateEntity) {
                state.ai_node = Some(AiNode::SeekLtg);
            }
            state.scripted_order = Some(BotOrderState {
                order,
                progress: BotOrderProgress::InProgress,
            });
            let ms = state.ms;
            self.ai.library.move_states.reset_avoid_reach(ms);
        }
        crate::behavior::orders::BOT_GOAL_ACTIVE
    }

    fn catalog_host(&mut self) -> (DirectorCatalogHost<'_, 'a>, &mut GameBotCatalog) {
        let files = self.files;
        let host = DirectorCatalogHost {
            host: &mut self.host,
            game: &mut self.game,
            ai: &mut self.ai,
            files,
        };
        (host, &mut self.catalog)
    }

    /// Connect a client through the catalog.
    pub fn connect(&mut self, client: i32, first_time: bool) -> bool {
        let (mut host, catalog) = self.catalog_host();
        catalog.connect(&mut host, client, first_time)
    }

    /// Shut down a client through the catalog.
    pub fn shutdown_client(&mut self, client: i32, restart: bool) {
        let (mut host, catalog) = self.catalog_host();
        catalog.shutdown_client(&mut host, client, restart);
    }

    /// Handle a bot console command.
    pub fn console_command(&mut self, argv: &[String]) -> bool {
        let (mut host, catalog) = self.catalog_host();
        catalog.console_command(&mut host, argv)
    }

    /// Bots named by an arena.
    #[must_use]
    pub fn arena_roster(&self, map: &str) -> Vec<String> {
        let Some(info) = self.catalog.arena_info_by_map(map) else {
            return Vec::new();
        };
        crate::behavior::q3::catalog::info_value(info, "bots")
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    /// Add a bot by name.
    pub fn add_bot(&mut self, name: &str, skill: f32, team: &str, delay_ms: i32) -> bool {
        let (mut host, catalog) = self.catalog_host();
        catalog.add_bot(&mut host, name, skill, team, delay_ms, name)
    }

    /// Close the director.
    pub fn close(&mut self, sink: &mut dyn crate::behavior::library::log::BotLogSink) {
        if self.phase == RoundPhase::Closed {
            return;
        }
        for entry in self.roster() {
            self.ai.shutdown_client(entry.source_client);
        }
        self.ai.shutdown(sink);
        self.sequences.clear();
        self.phase = RoundPhase::Closed;
    }
}

struct DirectorCatalogHost<'x, 'a> {
    host: &'x mut Box<dyn SourceBotDirectorHost + 'a>,
    game: &'x mut Box<dyn SourceBotGame + 'a>,
    ai: &'x mut GameAi<'a>,
    files: &'a dyn BotSourceFiles,
}

impl GameBotCatalogHost for DirectorCatalogHost<'_, '_> {
    fn allocate_client(&mut self) -> Option<i32> {
        self.host.allocate_client()
    }

    fn setup_client(&mut self, client: i32, settings: BotSettings, _restart: bool) -> bool {
        self.ai.setup_client(client, settings, self.files).is_ok()
    }

    fn shutdown_client(&mut self, client: i32, _restart: bool) {
        self.ai.shutdown_client(client);
    }

    fn time_ms(&self) -> i32 {
        self.host.time_ms()
    }

    fn print(&mut self, text: &str) {
        self.host.print(text);
    }

    fn name_in_use(&self, name: &str) -> bool {
        self.host.name_in_use(name)
    }

    fn team_counts(&self) -> Vec<(i32, i32, i32)> {
        let mut counts: HashMap<i32, (i32, i32)> = HashMap::new();
        for client in 0..self.game.max_clients() {
            let entity = self.game.entity(client);
            if let Some(player) = entity.player {
                let entry = counts.entry(player.team).or_insert((0, 0));
                if entity.bot {
                    entry.1 += 1;
                } else {
                    entry.0 += 1;
                }
            }
        }
        counts
            .into_iter()
            .map(|(team, (humans, bots))| (team, humans, bots))
            .collect()
    }
}
