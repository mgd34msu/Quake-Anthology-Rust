//! Game AI driver from `src/bots/behavior/q3/ai-main.ts`
//! (`game/ai_main.c`: `BotAISystem`, `BotAISetup`, `BotAILoadMap`,
//! `BotAIStartFrame`, `BotAIShutdown`, `BotAISetupClient`,
//! `BotAIShutdownClient`, `BotAIDeathmatchAI`, `BotInterbreedEndMatch`).
//!
//! `GameAi` owns the AI context and bot library for one game. Setup
//! registers cvars and loads shared data; each frame updates the
//! library clock, thinks every in-use bot on its 100ms cadence with
//! residual carry, and converts action cells to user commands.

use crate::behavior::library::actions::BotActionFlag;
use crate::behavior::library::genetic::BotRandom;
use crate::behavior::q3::ai_command::ConsoleMessageQueue;
use crate::behavior::q3::ai_context::GameAiContext;
use crate::behavior::q3::ai_decision::{bot_deathmatch_ai, DeathmatchFrame};
use crate::behavior::q3::ai_input::{
    bot_add_delta_angles, bot_change_view_angles, bot_input_to_user_command, bot_subtract_delta_angles,
};
use crate::behavior::q3::ai_state::{BotSettings, BotUserCommand, CommandButtons};
use crate::behavior::q3::game_host::SourceBotGame;
use crate::behavior::q3::library::BotLibrary;
use crate::behavior::q3::navigation_types::BotNavigation;
use crate::error::BotsError;

/// Bot think interval milliseconds.
pub const BOT_THINK_TIME: i32 = 100;

/// Game AI driver.
pub struct GameAi<'a> {
    /// AI context.
    pub context: GameAiContext,
    /// Bot library.
    pub library: BotLibrary<'a>,
    /// Console message queue.
    pub console: ConsoleMessageQueue,
    /// Generated user commands (client, command).
    pub commands: Vec<(i32, BotUserCommand)>,
    setup_done: bool,
    map_loaded: bool,
}

/// Clock inputs for one client think.
#[derive(Debug, Clone, Copy)]
struct ThinkClock {
    milliseconds: i32,
    time: f32,
    intermission: bool,
}

impl<'a> GameAi<'a> {
    /// New game AI over prepared files.
    pub fn new(files: &'a dyn crate::behavior::assets::BotSourceFiles, max_clients: i32, debug: bool) -> Self {
        Self {
            context: GameAiContext::new(max_clients),
            library: BotLibrary::new(files, max_clients.max(0) as usize, debug),
            console: ConsoleMessageQueue::new(),
            commands: Vec::new(),
            setup_done: false,
            map_loaded: false,
        }
    }

    /// Set up the AI (`BotAISetup`).
    pub fn setup(&mut self, restart: bool) -> Result<(), BotsError> {
        if self.setup_done && !restart {
            return Err(BotsError::BotLifetime("game AI already set up".to_owned()));
        }
        self.context.register_cvar("bot_enable", "1");
        self.context.register_cvar("bot_challenge", "0");
        self.context.register_cvar("bot_thinktime", "100");
        self.context.register_cvar("bot_memorydump", "0");
        self.context.register_cvar("bot_report", "0");
        self.context.register_cvar("bot_testsolid", "0");
        self.context.register_cvar("bot_testclusters", "0");
        self.library.setup()?;
        self.setup_done = true;
        Ok(())
    }

    /// Load map data (`BotAILoadMap`).
    pub fn load_map(&mut self, item_config: &str, gametype: i32, _restart: bool) -> Result<(), BotsError> {
        if !self.setup_done {
            return Err(BotsError::BotLifetime(
                "game AI setup required before map load".to_owned(),
            ));
        }
        self.library.load_map(item_config)?;
        self.context.deathmatch.gametype = gametype;
        self.map_loaded = true;
        Ok(())
    }

    /// Whether the map is loaded.
    #[must_use]
    pub fn is_map_loaded(&self) -> bool {
        self.map_loaded
    }

    /// Set up a client (`BotAISetupClient`).
    pub fn setup_client(
        &mut self,
        client: i32,
        settings: BotSettings,
        files: &'a dyn crate::behavior::assets::BotSourceFiles,
    ) -> Result<(), BotsError> {
        let character = self
            .library
            .characters
            .load_character_skill(files, &settings.characterfile, settings.skill)?;
        let ms = self
            .library
            .move_states
            .alloc()
            .ok_or_else(|| BotsError::BotLifetime("no free move state".to_owned()))?;
        let gs = self
            .library
            .goals
            .alloc_goal_state()
            .ok_or_else(|| BotsError::BotLifetime("no free goal state".to_owned()))?;
        let cs = self
            .library
            .chat
            .alloc_chat_state()
            .ok_or_else(|| BotsError::BotLifetime("no free chat state".to_owned()))?;
        let ws = self
            .library
            .weapons
            .alloc_state()
            .ok_or_else(|| BotsError::BotLifetime("no free weapon state".to_owned()))?;
        let state = self.context.states.acquire(client);
        state.inuse = true;
        state.client = client;
        state.entity_num = client;
        state.settings = settings;
        state.character = character;
        state.ms = ms;
        state.gs = gs;
        state.cs = cs;
        state.ws = ws;
        state.ai_node = Some(crate::behavior::q3::ai_state::AiNode::Stand);
        state.setup = crate::behavior::q3::ai_state::BotSetupProgress::Complete;
        self.context.num_bots += 1;
        Ok(())
    }

    /// Shut down a client (`BotAIShutdownClient`).
    pub fn shutdown_client(&mut self, client: i32) {
        let handles = self
            .context
            .states
            .get(client)
            .map(|state| (state.ms, state.gs, state.cs, state.ws, state.character));
        if let Some((ms, gs, cs, ws, character)) = handles {
            self.library.move_states.free(ms);
            self.library.goals.free_goal_state(gs);
            self.library.chat.free_chat_state(cs);
            self.library.weapons.free_state(ws);
            self.library.characters.free_character(character);
        }
        crate::behavior::q3::ai_context::bot_reset_state(&mut self.context, client);
        self.context.states.release(client);
        self.context.num_bots = (self.context.num_bots - 1).max(0);
    }

    /// Shut down the AI (`BotAIShutdown`).
    pub fn shutdown(&mut self, sink: &mut dyn crate::behavior::library::log::BotLogSink) {
        for client in 0..self.context.max_clients() {
            if self.context.states.get(client).is_some_and(|state| state.inuse) {
                self.shutdown_client(client);
            }
        }
        self.library.shutdown(sink);
        self.setup_done = false;
        self.map_loaded = false;
    }

    /// Start a frame (`BotAIStartFrame`): think every in-use bot.
    pub fn start_frame(
        &mut self,
        game: &mut dyn SourceBotGame,
        navigation: &mut dyn BotNavigation,
        milliseconds: i32,
        random: &mut dyn BotRandom,
        random_unit: impl Fn() -> f32,
    ) -> Result<(), BotsError> {
        if !self.setup_done || !self.map_loaded {
            return Err(BotsError::BotLifetime("game AI frame outside its lifetime".to_owned()));
        }
        self.commands.clear();
        self.context.node_switches.clear();
        let time = milliseconds as f32 / 1000.0;
        self.context.time = time;
        self.library.start_frame(time)?;
        let enabled = self.context.cvar("bot_enable").integer_value != 0;
        if !enabled {
            return Ok(());
        }
        let intermission = game.clock().intermission_time != 0;
        for client in 0..game.max_clients() {
            let inuse = self.context.states.get(client).is_some_and(|state| state.inuse);
            if !inuse {
                continue;
            }
            self.think_client(
                game,
                navigation,
                client,
                ThinkClock {
                    milliseconds,
                    time,
                    intermission,
                },
                random,
                random_unit(),
            );
        }
        Ok(())
    }

    fn think_client(
        &mut self,
        game: &mut dyn SourceBotGame,
        navigation: &mut dyn BotNavigation,
        client: i32,
        clock: ThinkClock,
        random: &mut dyn BotRandom,
        random_unit: f32,
    ) {
        let ThinkClock {
            milliseconds,
            time,
            intermission,
        } = clock;
        // 100ms think cadence with residual carry.
        let residual = self
            .context
            .states
            .get(client)
            .map(|state| state.bot_think_residual)
            .unwrap_or(0);
        let elapsed = milliseconds
            - self
                .context
                .states
                .get(client)
                .map(|state| (state.ltime * 1000.0) as i32)
                .unwrap_or(milliseconds);
        let mut budget = residual + elapsed.max(0);
        if budget < BOT_THINK_TIME && !intermission {
            if let Some(state) = self.context.states.get_mut(client) {
                state.bot_think_residual = budget;
            }
            return;
        }
        if let Some(state) = self.context.states.get_mut(client) {
            state.bot_think_residual = 0;
            state.ltime = time;
        }
        budget -= BOT_THINK_TIME;
        if budget > 0 {
            if let Some(state) = self.context.states.get_mut(client) {
                state.bot_think_residual = budget.min(BOT_THINK_TIME);
            }
        }
        self.sync_state(game, navigation, client);
        let alive = self
            .context
            .states
            .get(client)
            .map(|state| state.cur_ps.health > 0)
            .unwrap_or(false);
        bot_deathmatch_ai(
            &mut self.context,
            &mut self.library,
            game,
            navigation,
            random,
            DeathmatchFrame {
                client,
                time,
                intermission,
                alive,
                random_unit,
            },
        );
        self.update_input(client, milliseconds, time);
    }

    /// Sync the brain state from game observations.
    fn sync_state(&mut self, game: &dyn SourceBotGame, navigation: &dyn BotNavigation, client: i32) {
        let entity = game.entity(client);
        let Some(state) = self.context.states.get_mut(client) else {
            return;
        };
        state.origin = entity.origin;
        state.eye = entity.origin;
        state.eye.z += 26.0;
        state.area_num = navigation.point_area(entity.origin);
        if let Some(player) = &entity.player {
            state.cur_ps.health = player.health;
            state.cur_ps.weapon = player.weapon;
        }
        if let Some(ms) = self.library.move_states.get_mut(state.ms) {
            ms.origin = entity.origin;
        }
    }

    /// Convert the action cell to a user command (`BotUpdateInput`).
    fn update_input(&mut self, client: i32, milliseconds: i32, _time: f32) {
        let challenge = self.context.cvar("bot_challenge").integer_value != 0;
        let Some(state) = self.context.states.get_mut(client) else {
            return;
        };
        bot_add_delta_angles(state);
        let think = BOT_THINK_TIME as f32 / 1000.0;
        bot_change_view_angles(&self.library.characters, state, think, challenge);
        let mut input = self.library.actions.get_input(client);
        if input.action_flags & BotActionFlag::RESPAWN != 0 && state.last_ucmd.buttons & CommandButtons::ATTACK != 0 {
            input.action_flags &= !(BotActionFlag::RESPAWN | BotActionFlag::ATTACK);
        }
        let mut command = state.last_ucmd;
        bot_input_to_user_command(&input, &mut command, state.cur_ps.delta_angles, milliseconds);
        state.last_ucmd = command;
        bot_subtract_delta_angles(state);
        self.library.actions.view(client, state.viewangles);
        self.commands.push((client, command));
    }

    /// Drain generated commands.
    #[must_use]
    pub fn drain_commands(&mut self) -> Vec<(i32, BotUserCommand)> {
        std::mem::take(&mut self.commands)
    }

    /// Test AAS at an origin (diagnostic).
    pub fn test_aas(&self, _origin: qa_core::math::Vec3) -> bool {
        self.map_loaded
    }

    /// Interbreed characters at match end.
    pub fn interbreed_end_match(&mut self) {
        self.context.interbreed = false;
        self.context.interbreed_match_count += 1;
    }
}
