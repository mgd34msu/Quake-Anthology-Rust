//! Voice chat orders from `src/bots/behavior/q3/ai-voice.ts`
//! (`game/ai_vcmd.c`: `BotVoiceChat_GetFlag`, `BotVoiceChat_Offense`,
//! `BotVoiceChat_Defend`, `BotVoiceChat_Patrol`, `BotVoiceChat_Camp`,
//! `BotVoiceChat_FollowMe`, ...).
//!
//! Voice commands from the team leader (or any teammate) translate to
//! long-term goals with the standard durations.

use crate::behavior::q3::ai_context::GameAiContext;
use crate::behavior::q3::ai_definitions::{
    BotLongTermGoal, CTF_GETFLAG_TIME, TEAM_ACCOMPANY_TIME, TEAM_ATTACKENEMYBASE_TIME, TEAM_CAMP_TIME,
    TEAM_DEFENDKEYAREA_TIME, TEAM_HARVEST_TIME,
};
use crate::behavior::q3::ai_orders::{
    bot_get_alternate_route_goal, bot_opposite_team, bot_order_ltg, bot_remember_last_ordered_task,
    bot_set_team_status, bot_team, easy_client_name,
};
use crate::behavior::q3::game_host::{GameType, SourceBotGame, Team};

fn locate_requester(context: &mut GameAiContext, game: &dyn SourceBotGame, client: i32, requester: i32) -> bool {
    let info = game.entity(requester);
    let Some(state) = context.states.get_mut(client) else {
        return false;
    };
    state.team_goal.entity = -1;
    if info.present {
        state.team_goal.entity = requester;
        state.team_goal.origin = info.origin;
        return true;
    }
    let _ = easy_client_name(game, requester, 36);
    false
}

/// Voice order: get the flag.
pub fn bot_voice_chat_get_flag(context: &mut GameAiContext, game: &dyn SourceBotGame, client: i32, requester: i32) {
    if context.game_type() == GameType::CTF {
        bot_order_ltg(context, client, requester, BotLongTermGoal::GetFlag, CTF_GETFLAG_TIME);
        let team = context
            .states
            .get(client)
            .map(|state| bot_opposite_team(game, state))
            .unwrap_or(Team::FREE);
        bot_get_alternate_route_goal(context, client, team);
        bot_set_team_status(context, client);
        bot_remember_last_ordered_task(context, client);
    }
}

/// Voice order: offense.
pub fn bot_voice_chat_offense(context: &mut GameAiContext, game: &dyn SourceBotGame, client: i32, requester: i32) {
    if context.game_type() == GameType::CTF {
        bot_voice_chat_get_flag(context, game, client, requester);
        return;
    }
    if context.game_type() == GameType::HARVESTER {
        bot_order_ltg(context, client, requester, BotLongTermGoal::Harvest, TEAM_HARVEST_TIME);
    } else {
        bot_order_ltg(
            context,
            client,
            requester,
            BotLongTermGoal::AttackEnemyBase,
            TEAM_ATTACKENEMYBASE_TIME,
        );
    }
    bot_set_team_status(context, client);
    bot_remember_last_ordered_task(context, client);
}

/// Voice order: defend.
pub fn bot_voice_chat_defend(context: &mut GameAiContext, game: &dyn SourceBotGame, client: i32, requester: i32) {
    let team = context
        .states
        .get(client)
        .map(|state| bot_team(game, state))
        .unwrap_or(Team::FREE);
    if team != Team::RED && team != Team::BLUE {
        return;
    }
    bot_order_ltg(
        context,
        client,
        requester,
        BotLongTermGoal::DefendKeyArea,
        TEAM_DEFENDKEYAREA_TIME,
    );
    if let Some(state) = context.states.get_mut(client) {
        state.defend_away_time = 0.0;
    }
    bot_set_team_status(context, client);
    bot_remember_last_ordered_task(context, client);
}

/// Voice order: defend the flag.
pub fn bot_voice_chat_defend_flag(context: &mut GameAiContext, game: &dyn SourceBotGame, client: i32, requester: i32) {
    bot_voice_chat_defend(context, game, client, requester);
}

/// Voice order: patrol.
pub fn bot_voice_chat_patrol(context: &mut GameAiContext, client: i32, requester: i32) {
    if let Some(state) = context.states.get_mut(client) {
        state.decisionmaker = requester;
        state.ltg_type = BotLongTermGoal::None as i32;
        state.lead_time = 0.0;
        state.last_goal_ltg_type = 0;
    }
    bot_set_team_status(context, client);
}

/// Voice order: camp at the requester.
pub fn bot_voice_chat_camp(context: &mut GameAiContext, game: &dyn SourceBotGame, client: i32, requester: i32) {
    if !locate_requester(context, game, client, requester) {
        return;
    }
    bot_order_ltg(context, client, requester, BotLongTermGoal::CampOrder, TEAM_CAMP_TIME);
    if let Some(state) = context.states.get_mut(client) {
        state.teammate = requester;
        state.arrive_time = 0.0;
    }
    bot_set_team_status(context, client);
    bot_remember_last_ordered_task(context, client);
}

/// Voice order: follow me.
pub fn bot_voice_chat_follow_me(context: &mut GameAiContext, game: &dyn SourceBotGame, client: i32, requester: i32) {
    if !locate_requester(context, game, client, requester) {
        return;
    }
    bot_order_ltg(
        context,
        client,
        requester,
        BotLongTermGoal::TeamAccompany,
        TEAM_ACCOMPANY_TIME,
    );
    if let Some(state) = context.states.get_mut(client) {
        state.teammate = requester;
        state.teammate_visible_time = context.time;
        state.formation_dist = 112.0;
        state.arrive_time = 0.0;
    }
    bot_set_team_status(context, client);
}

/// Dispatch a voice command by name. Returns whether it was handled.
pub fn bot_voice_chat_command(
    context: &mut GameAiContext,
    game: &dyn SourceBotGame,
    client: i32,
    requester: i32,
    command: &str,
) -> bool {
    match command {
        "getflag" => bot_voice_chat_get_flag(context, game, client, requester),
        "offense" => bot_voice_chat_offense(context, game, client, requester),
        "defend" | "defendflag" => bot_voice_chat_defend(context, game, client, requester),
        "patrol" => bot_voice_chat_patrol(context, client, requester),
        "camp" => bot_voice_chat_camp(context, game, client, requester),
        "followme" => bot_voice_chat_follow_me(context, game, client, requester),
        _ => return false,
    }
    true
}
