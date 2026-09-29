//! Deathmatch decision AI from `src/bots/behavior/q3/ai-decision.ts`
//! (`game/ai_dmq3.c`: `BotDeathmatchAI`, `BotBattleAI`,
//! `BotIntermissionAI`, `BotObserverAI`, `BotRespawnAI`, `BotStandAI`,
//! `BotUpdateBattleInventory`, `BotHarvesterAI`, `BotCTFAI`).
//!
//! The per-frame decision: update inventory and enemy, dispatch the AI
//! node (intermission, observer, respawn, stand, seek, battle), and
//! convert the movement result plus combat actions into the action
//! buffer.

use crate::behavior::library::actions::BotActionFlag;
use crate::behavior::library::genetic::BotRandom;
use crate::behavior::q3::ai_combat::{
    bot_apply_move_result, bot_battle_chase, bot_battle_fight, bot_battle_retreat, bot_find_enemy, bot_wants_to_chase,
    bot_wants_to_retreat, update_bot_inventory,
};
use crate::behavior::q3::ai_context::GameAiContext;
use crate::behavior::q3::ai_definitions::BotLongTermGoal;
use crate::behavior::q3::ai_navigation::{bot_seek_activate_entity, bot_seek_ltg, bot_seek_nbg, SeekFrame};
use crate::behavior::q3::ai_state::{AiNode, BotState};
use crate::behavior::q3::game_host::SourceBotGame;
use crate::behavior::q3::library::BotLibrary;
use crate::behavior::q3::movement_state::BotMoveResult;
use crate::behavior::q3::navigation_types::BotNavigation;

/// Intermission: hold still and watch.
pub fn bot_intermission_ai(
    state: &mut BotState,
    actions: &mut crate::behavior::library::actions::BotActionBuffer,
) -> AiNode {
    actions.reset_input(state.client);
    AiNode::Intermission
}

/// Observer: follow the next player.
pub fn bot_observer_ai(context: &mut GameAiContext, game: &dyn SourceBotGame, client: i32) -> AiNode {
    if let Some(state) = context.states.get_mut(client) {
        state.viewangles.y += 1.0;
    }
    let _ = game;
    AiNode::Observer
}

/// Respawn: press attack until alive.
pub fn bot_respawn_ai(context: &mut GameAiContext, library: &mut BotLibrary<'_>, client: i32, alive: bool) -> AiNode {
    if alive {
        if let Some(state) = context.states.get_mut(client) {
            state.respawn_wait = false;
        }
        return AiNode::Stand;
    }
    library
        .actions
        .action(client, BotActionFlag::RESPAWN | BotActionFlag::ATTACK);
    AiNode::Respawn
}

/// Stand: look around and acquire the first enemy.
pub fn bot_stand_ai(
    context: &mut GameAiContext,
    characters: &crate::behavior::library::character::BotCharacterLibrary,
    game: &dyn SourceBotGame,
    client: i32,
    time: f32,
    random: &mut dyn BotRandom,
) -> AiNode {
    let enemy = bot_find_enemy(game, context, client);
    if enemy >= 0 && bot_wants_to_chase(characters, context.states.get(client).expect("client state")) {
        if let Some(state) = context.states.get_mut(client) {
            state.stand_time = time;
        }
        let _ = random;
        return AiNode::BattleChase;
    }
    if let Some(state) = context.states.get_mut(client) {
        state.viewangles.y += random.next_unit() * 4.0 - 2.0;
        if time - state.stand_time > 2.0 {
            state.stand_time = time;
            return AiNode::SeekLtg;
        }
    }
    AiNode::Stand
}

/// Battle dispatch by range and health.
pub fn bot_battle_ai(
    game: &dyn SourceBotGame,
    characters: &crate::behavior::library::character::BotCharacterLibrary,
    state: &mut BotState,
    result: &mut BotMoveResult,
    time: f32,
    random: &mut dyn BotRandom,
) -> AiNode {
    if state.enemy < 0 {
        return AiNode::SeekLtg;
    }
    if bot_wants_to_retreat(characters, state) {
        return bot_battle_retreat(game, characters, state, result, time, random);
    }
    match state.ai_node {
        Some(AiNode::BattleChase) => bot_battle_chase(game, characters, state, result, time, random),
        Some(AiNode::BattleRetreat) => bot_battle_retreat(game, characters, state, result, time, random),
        Some(AiNode::BattleNbg) => AiNode::BattleNbg,
        _ => bot_battle_fight(game, characters, state, result, time, random),
    }
}

/// Per-think scalar frame for [`bot_deathmatch_ai`].
#[derive(Debug, Clone, Copy)]
pub struct DeathmatchFrame {
    pub client: i32,
    pub time: f32,
    pub intermission: bool,
    pub alive: bool,
    pub random_unit: f32,
}

/// Deathmatch AI: one think for a bot.
pub fn bot_deathmatch_ai(
    context: &mut GameAiContext,
    library: &mut BotLibrary<'_>,
    game: &mut dyn SourceBotGame,
    navigation: &mut dyn BotNavigation,
    random: &mut dyn BotRandom,
    frame: DeathmatchFrame,
) -> AiNode {
    let DeathmatchFrame {
        client,
        time,
        intermission,
        alive,
        random_unit,
    } = frame;
    if intermission {
        if let Some(state) = context.states.get_mut(client) {
            state.ai_node = Some(AiNode::Intermission);
        }
        let state = context.states.get_mut(client).expect("client state");
        let actions = &mut library.actions;
        return bot_intermission_ai(state, actions);
    }
    if !alive {
        let node = bot_respawn_ai(context, library, client, false);
        if let Some(state) = context.states.get_mut(client) {
            state.ai_node = Some(node);
        }
        return node;
    }
    if let Some(state) = context.states.get_mut(client) {
        update_bot_inventory(state);
        game.knowledge().update_inventory(state);
    }
    bot_find_enemy(game, context, client);
    let node = context.states.get(client).and_then(|state| state.ai_node);
    let mut result = BotMoveResult::default();
    let next = match node {
        None | Some(AiNode::Stand) => bot_stand_ai(context, &library.characters, game, client, time, random),
        Some(AiNode::Respawn) => bot_respawn_ai(context, library, client, true),
        Some(AiNode::Observer) => bot_observer_ai(context, game, client),
        Some(AiNode::Intermission) => {
            if let Some(state) = context.states.get_mut(client) {
                let actions = &mut library.actions;
                bot_intermission_ai(state, actions)
            } else {
                AiNode::Intermission
            }
        }
        Some(AiNode::SeekLtg) => {
            let enemy = context.states.get(client).map(|state| state.enemy).unwrap_or(-1);
            if enemy >= 0 {
                let state = context.states.get(client).expect("client state");
                if bot_wants_to_chase(&library.characters, state) {
                    let state = context.states.get_mut(client).expect("client state");
                    bot_battle_ai(game, &library.characters, state, &mut result, time, random)
                } else {
                    bot_seek_ltg(
                        context,
                        library,
                        game,
                        navigation,
                        SeekFrame {
                            client,
                            time,
                            random_unit,
                        },
                        &mut result,
                    )
                }
            } else {
                bot_seek_ltg(
                    context,
                    library,
                    game,
                    navigation,
                    SeekFrame {
                        client,
                        time,
                        random_unit,
                    },
                    &mut result,
                )
            }
        }
        Some(AiNode::SeekNbg) => bot_seek_nbg(context, library, navigation, client, &mut result, time),
        Some(AiNode::SeekActivateEntity) => bot_seek_activate_entity(context, library, navigation, client, &mut result),
        Some(AiNode::BattleFight)
        | Some(AiNode::BattleChase)
        | Some(AiNode::BattleRetreat)
        | Some(AiNode::BattleNbg) => {
            let state = context.states.get_mut(client).expect("client state");
            bot_battle_ai(game, &library.characters, state, &mut result, time, random)
        }
    };
    if let Some(state) = context.states.get_mut(client) {
        state.ai_node = Some(next);
        let speed = 400.0;
        bot_apply_move_result(&mut library.actions, state, &result, speed);
    }
    context.track_node_switch(match next {
        AiNode::Intermission => "intermission",
        AiNode::Observer => "observer",
        AiNode::Respawn => "respawn",
        AiNode::Stand => "stand",
        AiNode::SeekActivateEntity => "seek-activate-entity",
        AiNode::SeekNbg => "seek-nbg",
        AiNode::SeekLtg => "seek-ltg",
        AiNode::BattleFight => "battle-fight",
        AiNode::BattleChase => "battle-chase",
        AiNode::BattleRetreat => "battle-retreat",
        AiNode::BattleNbg => "battle-nbg",
    });
    next
}

/// CTF strategy selection: carriers escort, others push or defend.
pub fn bot_ctf_ai(context: &mut GameAiContext, client: i32) {
    let ltg = context.states.get(client).map(|state| state.ltg_type).unwrap_or(0);
    if BotLongTermGoal::from_i32(ltg) == BotLongTermGoal::None {
        if let Some(state) = context.states.get_mut(client) {
            state.ltg_type = BotLongTermGoal::GetFlag as i32;
            state.team_goal_time = context.time + super::ai_definitions::CTF_GETFLAG_TIME;
        }
    }
}

/// Harvester strategy selection.
pub fn bot_harvester_ai(context: &mut GameAiContext, client: i32) {
    let ltg = context.states.get(client).map(|state| state.ltg_type).unwrap_or(0);
    if BotLongTermGoal::from_i32(ltg) == BotLongTermGoal::None {
        if let Some(state) = context.states.get_mut(client) {
            state.ltg_type = BotLongTermGoal::Harvest as i32;
            state.team_goal_time = context.time + super::ai_definitions::TEAM_HARVEST_TIME;
        }
    }
}
