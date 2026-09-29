//! Team AI from `src/bots/behavior/q3/ai-team.ts` (`game/ai_team.c`:
//! `BotNumTeamMates`, `BotSortTeamMatesByBaseTravelTime`,
//! `BotSetTeamMateTaskPreference`, `BotGetTeamMateTaskPreference`,
//! `BotTeamOrders`, `BotClientFromName`, `BotClientOnSameTeamFromName`,
//! `BotSameTeam`, `BotTeamLeader`).
//!
//! The team leader distributes tasks by travel-time-sorted preference:
//! defenders hold key areas while attackers push objectives. Name
//! lookups resolve chat addressees to clients.

use crate::behavior::q3::ai_context::{GameAiContext, TaskPreference};
use crate::behavior::q3::ai_orders::{bot_teammates, client_name, same_team};
use crate::behavior::q3::game_host::SourceBotGame;

/// Number of teammates.
#[must_use]
pub fn bot_num_team_mates(game: &dyn SourceBotGame, context: &GameAiContext, client: i32) -> i32 {
    let Some(state) = context.states.get(client) else {
        return 0;
    };
    bot_teammates(game, state).len() as i32
}

/// Sort teammates by travel time to a goal area.
pub fn bot_sort_team_mates_by_travel_time(
    game: &dyn SourceBotGame,
    context: &GameAiContext,
    client: i32,
    goal_area: i32,
    travel_time: &dyn Fn(i32, i32) -> i32,
) -> Vec<i32> {
    let Some(state) = context.states.get(client) else {
        return Vec::new();
    };
    let mut mates = bot_teammates(game, state);
    mates.sort_by_key(|mate| {
        context
            .states
            .get(*mate)
            .map_or(i32::MAX, |mate_state| travel_time(mate_state.area_num, goal_area))
    });
    mates
}

/// Set a teammate task preference.
pub fn bot_set_team_mate_task_preference(context: &mut GameAiContext, name: &str, preference: i32) {
    if let Some(entry) = context
        .team
        .task_preferences
        .iter_mut()
        .find(|entry| entry.name == name)
    {
        entry.preference = preference;
    } else {
        context.team.task_preferences.push(TaskPreference {
            name: name.to_owned(),
            preference,
        });
    }
}

/// Get a teammate task preference, or 0.
#[must_use]
pub fn bot_get_team_mate_task_preference(context: &GameAiContext, name: &str) -> i32 {
    context
        .team
        .task_preferences
        .iter()
        .find(|entry| entry.name == name)
        .map_or(0, |entry| entry.preference)
}

/// Resolve a client number from a name.
#[must_use]
pub fn bot_client_from_name(game: &dyn SourceBotGame, context: &GameAiContext, name: &str) -> i32 {
    for client in 0..game.max_clients() {
        if client_name(game, client).eq_ignore_ascii_case(name) {
            let _ = context;
            return client;
        }
    }
    -1
}

/// Resolve a same-team client number from a name.
#[must_use]
pub fn bot_client_on_same_team_from_name(
    game: &dyn SourceBotGame,
    context: &GameAiContext,
    client: i32,
    name: &str,
) -> i32 {
    let resolved = bot_client_from_name(game, context, name);
    if resolved >= 0 && same_team(game, client, resolved) {
        resolved
    } else {
        -1
    }
}

/// Whether a client is the team leader.
#[must_use]
pub fn bot_is_team_leader(context: &GameAiContext, game: &dyn SourceBotGame, client: i32) -> bool {
    let Some(state) = context.states.get(client) else {
        return false;
    };
    !state.team_leader.is_empty() && state.team_leader == client_name(game, client)
}

/// Distribute team orders as the leader: attackers push, defenders
/// hold. Returns (client, ltg_type) assignments.
pub fn bot_team_orders(game: &dyn SourceBotGame, context: &mut GameAiContext, leader: i32) -> Vec<(i32, i32)> {
    let Some(leader_state) = context.states.get(leader) else {
        return Vec::new();
    };
    let mates = bot_teammates(game, leader_state);
    let mut assignments = Vec::new();
    for mate in mates {
        let name = client_name(game, mate);
        let preference = bot_get_team_mate_task_preference(context, &name);
        let ltg = if preference == crate::behavior::q3::ai_definitions::BotTeamTaskPreference::Defender as i32 {
            crate::behavior::q3::ai_definitions::BotLongTermGoal::DefendKeyArea as i32
        } else {
            crate::behavior::q3::ai_definitions::BotLongTermGoal::GetFlag as i32
        };
        assignments.push((mate, ltg));
    }
    assignments
}

/// Elect a team leader when none is set: lowest in-use client on the team.
pub fn bot_elect_team_leader(game: &dyn SourceBotGame, context: &mut GameAiContext, client: i32) {
    let Some(state) = context.states.get(client) else {
        return;
    };
    if !state.team_leader.is_empty() {
        return;
    }
    let team = crate::behavior::q3::ai_orders::client_team(game, client);
    let mut leader = client;
    for other in 0..game.max_clients() {
        if other != client
            && crate::behavior::q3::ai_orders::client_team(game, other) == team
            && context.states.get(other).is_some_and(|state| state.inuse)
            && other < leader
        {
            leader = other;
        }
    }
    let name = client_name(game, leader);
    if let Some(state) = context.states.get_mut(client) {
        state.team_leader = name;
    }
}
