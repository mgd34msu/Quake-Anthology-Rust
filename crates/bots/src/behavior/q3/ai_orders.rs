//! Team orders from `src/bots/behavior/q3/ai-orders.ts`
//! (`game/ai_team.c`, `ai_dmq3.c` order helpers: `BotSetTeamStatus`,
//! `BotRememberLastOrderedTask`, `BotGetAlternateRouteGoal`,
//! `ClientName`, `EasyClientName`, `BotTeam`, `BotOppositeTeam`).
//!
//! Orders assign a long-term goal plus a teammate anchor; the ordered
//! bot remembers the task so it can resume after interruptions.

use crate::behavior::library::goals::BotGoal;
use crate::behavior::q3::ai_context::GameAiContext;
use crate::behavior::q3::ai_definitions::BotLongTermGoal;
use crate::behavior::q3::ai_state::BotState;
use crate::behavior::q3::game_host::{SourceBotGame, Team};

/// Team of a bot from the game view.
#[must_use]
pub fn bot_team(game: &dyn SourceBotGame, state: &BotState) -> i32 {
    game.entity(state.client)
        .player
        .map_or(Team::FREE, |player| player.team)
}

/// Team of any client.
#[must_use]
pub fn client_team(game: &dyn SourceBotGame, client: i32) -> i32 {
    game.entity(client).player.map_or(Team::FREE, |player| player.team)
}

/// Opposite team.
#[must_use]
pub fn bot_opposite_team(game: &dyn SourceBotGame, state: &BotState) -> i32 {
    match bot_team(game, state) {
        Team::RED => Team::BLUE,
        Team::BLUE => Team::RED,
        other => other,
    }
}

/// Client display name.
#[must_use]
pub fn client_name(game: &dyn SourceBotGame, client: i32) -> String {
    game.entity(client)
        .player
        .map_or_else(|| format!("client{client}"), |player| player.name.clone())
}

/// Easy (truncated, cleaned) client name for chat.
#[must_use]
pub fn easy_client_name(game: &dyn SourceBotGame, client: i32, max_len: usize) -> String {
    let name = client_name(game, client);
    let cleaned: String = name.chars().filter(|c| !c.is_control()).take(max_len).collect();
    cleaned.trim().to_owned()
}

/// Copy a client name into the team leader slot.
pub fn copy_client_name_to_team_leader(game: &dyn SourceBotGame, client: i32, state: &mut BotState) {
    state.team_leader = easy_client_name(game, client, 32);
}

/// Teammate clients of a bot (excluding self).
#[must_use]
pub fn bot_teammates(game: &dyn SourceBotGame, state: &BotState) -> Vec<i32> {
    let team = bot_team(game, state);
    (0..game.max_clients())
        .filter(|client| *client != state.client && client_team(game, *client) == team)
        .collect()
}

/// Whether two clients share a team.
#[must_use]
pub fn same_team(game: &dyn SourceBotGame, first: i32, second: i32) -> bool {
    client_team(game, first) == client_team(game, second)
}

/// Flag carrier client on a team, or -1.
#[must_use]
pub fn bot_team_flag_carrier(context: &GameAiContext, team: i32) -> i32 {
    for client in 0..context.max_clients() {
        if let Some(state) = context.states.get(client) {
            if !state.inuse {
                continue;
            }
            let carrying = if team == Team::RED {
                state
                    .inventory
                    .get(crate::behavior::q3::ai_definitions::BotInventory::BLUEFLAG)
                    .copied()
                    .unwrap_or(0)
                    > 0
            } else {
                state
                    .inventory
                    .get(crate::behavior::q3::ai_definitions::BotInventory::REDFLAG)
                    .copied()
                    .unwrap_or(0)
                    > 0
            };
            if carrying {
                return client;
            }
        }
    }
    -1
}

/// Publish team status from the current long-term goal.
pub fn bot_set_team_status(context: &mut GameAiContext, client: i32) {
    let ltg = context.states.get(client).map(|state| state.ltg_type).unwrap_or(0);
    let _ = BotLongTermGoal::from_i32(ltg);
    context.track_node_switch("team-status");
}

/// Remember the ordered task so the bot can resume it.
pub fn bot_remember_last_ordered_task(context: &mut GameAiContext, client: i32) {
    if let Some(state) = context.states.get_mut(client) {
        state.last_goal_decisionmaker = state.decisionmaker;
        state.last_goal_ltg_type = state.ltg_type;
        state.last_goal_teammate = state.teammate;
        state.last_goal_team_goal = state.team_goal;
    }
}

/// Set an alternate route goal toward the enemy side.
pub fn bot_get_alternate_route_goal(context: &mut GameAiContext, client: i32, team: i32) {
    let goal = if team == Team::RED {
        context.deathmatch.red_alternate_goals.first().map(|alt| BotGoal {
            origin: alt.origin,
            area: alt.area,
            ..BotGoal::default()
        })
    } else {
        context.deathmatch.blue_alternate_goals.first().map(|alt| BotGoal {
            origin: alt.origin,
            area: alt.area,
            ..BotGoal::default()
        })
    };
    if let (Some(goal), Some(state)) = (goal, context.states.get_mut(client)) {
        state.alt_route_goal = goal;
    }
}

/// Order a bot to a long-term goal with a duration.
pub fn bot_order_ltg(
    context: &mut GameAiContext,
    client: i32,
    decisionmaker: i32,
    ltg: BotLongTermGoal,
    duration: f32,
) {
    // Borrow fix
    let time = context.time;
    if let Some(state) = context.states.get_mut(client) {
        state.decisionmaker = decisionmaker;
        state.ordered = true;
        state.order_time = time;
        state.ltg_type = ltg as i32;
        state.team_goal_time = time + duration;
    }
    bot_remember_last_ordered_task(context, client);
}

/// Print the team goal for debugging; returns the summary line.
#[must_use]
pub fn bot_print_team_goal(context: &GameAiContext, client: i32) -> String {
    let (name, ltg) = context
        .states
        .get(client)
        .map(|state| (state.client, state.ltg_type))
        .unwrap_or((client, 0));
    format!("bot {name} team goal {}", BotLongTermGoal::from_i32(ltg) as i32)
}
