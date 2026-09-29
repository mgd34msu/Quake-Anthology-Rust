//! Shared bot population from `src/bots/behavior/population.ts`.
//!
//! One decision controller drives all admitted bot actors through
//! ordinary client commands. Goals address actors; the population maps
//! them to source clients, validates liveness, and rejects frames
//! that target foreign actors or double-drive one actor.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_world::client::ClientCommand;

use crate::behavior::director::SourceBotDirector;
use crate::behavior::orders::{BotGoalStatus, BOT_GOAL_NONE};
use crate::error::BotsError;

/// Bot frame clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotFrame {
    /// Time milliseconds.
    pub time_milliseconds: i32,
    /// Elapsed milliseconds.
    pub elapsed_milliseconds: i32,
}

/// One bot-driven client command.
#[derive(Debug, Clone, PartialEq)]
pub struct BotActorCommand {
    /// Target actor.
    pub actor: ActorId,
    /// Registry slot.
    pub slot: u32,
    /// Command sequence.
    pub sequence: u32,
    /// Client command.
    pub command: ClientCommand,
}

/// Shared bot population over a director.
pub struct SharedBotPopulation<'d, 'a> {
    director: &'d mut SourceBotDirector<'a>,
    is_live: Box<dyn Fn(&ActorId) -> bool + 'd>,
}

impl<'d, 'a> SharedBotPopulation<'d, 'a> {
    /// New population over a director with a liveness predicate.
    pub fn new(director: &'d mut SourceBotDirector<'a>, is_live: impl Fn(&ActorId) -> bool + 'd) -> Self {
        Self {
            director,
            is_live: Box::new(is_live),
        }
    }

    fn client_for(&self, actor: &ActorId) -> Option<i32> {
        if !(self.is_live)(actor) {
            return None;
        }
        self.director
            .roster()
            .iter()
            .find(|entry| entry.actor == *actor)
            .map(|entry| entry.source_client)
    }

    /// Request a move-to-point goal for an actor.
    pub fn request_move_to_point(&mut self, actor: &ActorId, point: Vec3) -> BotGoalStatus {
        self.client_for(actor).map_or(BOT_GOAL_NONE, |client| {
            self.director.request_move_to_point(client, point)
        })
    }

    /// Request a follow-entity goal for an actor.
    pub fn request_follow_entity(&mut self, actor: &ActorId, entity: i32) -> BotGoalStatus {
        self.client_for(actor).map_or(BOT_GOAL_NONE, |client| {
            self.director.request_follow_entity(client, entity)
        })
    }

    /// Clear an actor's goal.
    pub fn clear_goal(&mut self, actor: &ActorId) {
        if let Some(client) = self.client_for(actor) {
            self.director.clear_goal(client);
        }
    }

    /// Goal status for an actor.
    pub fn goal_status(&mut self, actor: &ActorId) -> BotGoalStatus {
        self.client_for(actor)
            .map_or(BOT_GOAL_NONE, |client| self.director.goal_status(client))
    }

    /// Adapter driving this population into the server tick.
    #[must_use]
    pub fn command_source(self) -> PopulationCommandSource<'d, 'a> {
        PopulationCommandSource {
            population: self,
            last_time_ms: 0,
        }
    }

    /// Run a bot frame; validates all generated commands.
    pub fn frame(&mut self, frame: BotFrame) -> Result<Vec<BotActorCommand>, BotsError> {
        let commands = self.director.frame(frame.time_milliseconds)?;
        let mut seen = std::collections::HashSet::new();
        for command in &commands {
            if !(self.is_live)(&command.actor) {
                return Err(BotsError::BotLifetime(
                    "bot command targets an actor outside the shared world".to_owned(),
                ));
            }
            if !seen.insert((command.actor.slot(), command.actor.generation())) {
                return Err(BotsError::BotLifetime(
                    "two bot commands target the same actor in one frame".to_owned(),
                ));
            }
        }
        Ok(commands)
    }
}

/// Server tick adapter: polls the population each frame and emits
/// `(slot, command)` pairs for the ordinary client pipeline. A failed
/// bot frame emits nothing rather than stalling the server.
pub struct PopulationCommandSource<'d, 'a> {
    population: SharedBotPopulation<'d, 'a>,
    last_time_ms: i32,
}

impl qa_world::server::BotCommandSource for PopulationCommandSource<'_, '_> {
    fn bot_commands(&mut self, time_ms: i32) -> Vec<(u32, ClientCommand)> {
        let elapsed = time_ms.saturating_sub(self.last_time_ms).max(1);
        self.last_time_ms = time_ms;
        self.population
            .frame(BotFrame {
                time_milliseconds: time_ms,
                elapsed_milliseconds: elapsed,
            })
            .unwrap_or_default()
            .into_iter()
            .map(|command| (command.slot, command.command))
            .collect()
    }
}
