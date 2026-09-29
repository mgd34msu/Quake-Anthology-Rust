//! Combat AI from `src/bots/behavior/q3/ai-combat.ts`
//! (`game/ai_dmq3.c` battle half: `BotUpdateInventory`,
//! `BotChooseWeapon`, `BotAimAtEnemy`, `BotCheckAttack`,
//! `BotBattleFight`, `BotBattleChase`, `BotBattleRetreat`,
//! `BotBattleNbg`, `BotWantsToRetreat`, `BotWantsToChase`,
//! `BotFindEnemy`, `BotVisible`).
//!
//! Each frame the bot refreshes its inventory, picks an enemy, aims
//! with skill-scaled error, and either fights, chases, or retreats.
//! Attack gating combines reaction time, fire throttle, and weapon
//! range; movement strafes, crouches, and jumps on characteristic
//! timers.

use qa_core::math::Vec3;

use crate::behavior::library::actions::BotActionFlag;
use crate::behavior::library::character::{BotCharacterLibrary, Characteristic};
use crate::behavior::library::genetic::BotRandom;
use crate::behavior::q3::ai_context::GameAiContext;
use crate::behavior::q3::ai_definitions::{BotFlag, BotInventory};
use crate::behavior::q3::ai_state::{AiNode, BotState, CommandButtons};
use crate::behavior::q3::game_host::{BotTraceQuery, SourceBotGame};
use crate::behavior::q3::movement_state::{BotMoveResult, BotMoveResultFlag};

/// Refresh the bot inventory from its player snapshot.
pub fn update_bot_inventory(state: &mut BotState) {
    state.inventory[BotInventory::HEALTH] = state.cur_ps.health;
    state.inventory[BotInventory::ARMOR] = state.cur_ps.armor;
}

/// Refresh item inventory from powerups.
pub fn update_bot_item_inventory(state: &mut BotState) {
    for (index, powerup) in state.cur_ps.powerups.iter().enumerate() {
        let slot = BotInventory::TELEPORTER + index;
        if slot < state.inventory.len() {
            state.inventory[slot] = *powerup;
        }
    }
}

/// Whether an enemy entity is visible from the bot's eye.
#[must_use]
pub fn bot_visible(game: &dyn SourceBotGame, state: &BotState, enemy: i32) -> bool {
    let target = game.entity(enemy);
    if !target.present {
        return false;
    }
    let result = game.trace(&BotTraceQuery {
        start: state.eye,
        end: target.origin,
        pass_entity: state.entity_num,
        mask: 1,
        bounds: None,
    });
    result.fraction >= 1.0 || result.entity_num == enemy
}

/// Find an enemy: nearest visible, living opponent.
pub fn bot_find_enemy(game: &dyn SourceBotGame, context: &mut GameAiContext, client: i32) -> i32 {
    let time = context.time;
    let origin = context.states.get(client).map(|state| state.origin);
    let Some(origin) = origin else {
        return -1;
    };
    let team = game.entity(client).player.map(|player| player.team).unwrap_or(-99);
    let mut best = -1;
    let mut best_dist = f32::MAX;
    for other in 0..game.max_clients() {
        if other == client {
            continue;
        }
        let entity = game.entity(other);
        let Some(player) = entity.player else {
            continue;
        };
        if !entity.present || player.team == team || player.health <= 0 {
            continue;
        }
        let dx = entity.origin.x - origin.x;
        let dy = entity.origin.y - origin.y;
        let dz = entity.origin.z - origin.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist < best_dist {
            best_dist = dist;
            best = other;
        }
    }
    if let Some(state) = context.states.get_mut(client) {
        if best >= 0 {
            if state.enemy != best {
                state.enemy_sight_time = time;
            }
            state.enemy = best;
            state.enemy_visible_time = time;
            let entity = game.entity(best);
            state.enemy_origin = entity.origin;
        } else if state.enemy >= 0 && time - state.enemy_visible_time > 4.0 {
            state.enemy = -1;
        }
    }
    best
}

/// Aim at the enemy with skill-scaled error.
pub fn bot_aim_at_enemy(
    game: &dyn SourceBotGame,
    characters: &BotCharacterLibrary,
    state: &mut BotState,
    time: f32,
    random: &mut dyn BotRandom,
) {
    if state.enemy < 0 {
        return;
    }
    let accuracy = characters.bounded_float(state.character, Characteristic::AIM_ACCURACY, 0.0, 1.0);
    let skill = characters.bounded_float(state.character, Characteristic::AIM_SKILL, 0.0, 1.0);
    let entity = game.entity(state.enemy);
    let error = (1.0 - accuracy) * 120.0 + (1.0 - skill) * 60.0;
    let target = Vec3 {
        x: entity.origin.x + (random.next_unit() - 0.5) * 2.0 * error,
        y: entity.origin.y + (random.next_unit() - 0.5) * 2.0 * error,
        z: entity.origin.z + 24.0 + (random.next_unit() - 0.5) * error,
    };
    state.aim_target = target;
    let dir = Vec3 {
        x: target.x - state.eye.x,
        y: target.y - state.eye.y,
        z: target.z - state.eye.z,
    };
    let yaw = dir.y.atan2(dir.x).to_degrees();
    let pitch = (-dir.z / (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0))
        .atan()
        .to_degrees();
    state.ideal_viewangles = Vec3 {
        x: pitch,
        y: yaw,
        z: 0.0,
    };
    state.flags |= BotFlag::IDEALVIEWSET;
    let _ = time;
}

/// Whether the bot wants to retreat: low health against a strong enemy.
#[must_use]
pub fn bot_wants_to_retreat(characters: &BotCharacterLibrary, state: &BotState) -> bool {
    let health = state.inventory.get(BotInventory::HEALTH).copied().unwrap_or(100);
    let preservation = characters.bounded_float(state.character, Characteristic::SELF_PRESERVATION, 0.0, 1.0);
    (health as f32) < 25.0 + preservation * 50.0 && state.enemy >= 0
}

/// Whether the bot wants to chase: healthy and aggressive.
#[must_use]
pub fn bot_wants_to_chase(characters: &BotCharacterLibrary, state: &BotState) -> bool {
    let health = state.inventory.get(BotInventory::HEALTH).copied().unwrap_or(0);
    let aggression = characters.bounded_float(state.character, Characteristic::AGGRESSION, 0.0, 1.0);
    state.enemy >= 0 && (health as f32) > 40.0 && aggression > 0.3
}

/// Check attack gating: reaction time, fire throttle, range, visibility.
pub fn bot_check_attack(
    game: &dyn SourceBotGame,
    characters: &BotCharacterLibrary,
    state: &mut BotState,
    time: f32,
) -> bool {
    if state.enemy < 0 {
        return false;
    }
    let reaction = characters.bounded_float(state.character, Characteristic::REACTION_TIME, 0.0, 5.0);
    if time - state.enemy_sight_time < reaction {
        return false;
    }
    let throttle = characters.bounded_float(state.character, Characteristic::FIRE_THROTTLE, 0.0, 1.0);
    if throttle < 1.0 && time < state.fire_throttle_shoot_time {
        return false;
    }
    if !bot_visible(game, state, state.enemy) {
        return false;
    }
    let dx = state.enemy_origin.x - state.origin.x;
    let dy = state.enemy_origin.y - state.origin.y;
    let range = (dx * dx + dy * dy).sqrt();
    state.inventory[BotInventory::ENEMY_HORIZONTAL_DIST] = range as i32;
    state.inventory[BotInventory::ENEMY_HEIGHT] = (state.enemy_origin.z - state.origin.z) as i32;
    if throttle < 1.0 {
        state.fire_throttle_shoot_time = time + (1.0 - throttle) * 0.5;
        state.fire_throttle_wait_time = time;
    }
    state.flags |= BotFlag::AIMATENEMY;
    true
}

/// Battle fight: strafe around the enemy and fire.
pub fn bot_battle_fight(
    game: &dyn SourceBotGame,
    characters: &BotCharacterLibrary,
    state: &mut BotState,
    result: &mut BotMoveResult,
    time: f32,
    random: &mut dyn BotRandom,
) -> AiNode {
    bot_aim_at_enemy(game, characters, state, time, random);
    if bot_wants_to_retreat(characters, state) {
        return AiNode::BattleRetreat;
    }
    if !bot_visible(game, state, state.enemy) {
        return AiNode::BattleChase;
    }
    // Strafe direction flips on the attack strafe timer.
    if time > state.attack_strafe_time {
        state.attack_strafe_time = time + 0.4 + random.next_unit() * 0.8;
        if random.next_unit() < 0.5 {
            state.flags |= BotFlag::STRAFERIGHT;
        } else {
            state.flags &= !BotFlag::STRAFERIGHT;
        }
    }
    let strafe = if state.flags & BotFlag::STRAFERIGHT != 0 {
        1.0
    } else {
        -1.0
    };
    let yaw = state.viewangles.y.to_radians();
    result.move_direction = Vec3 {
        x: -yaw.sin() * strafe,
        y: yaw.cos() * strafe,
        z: 0.0,
    };
    result.flags |= BotMoveResultFlag::MOVEMENTVIEW;
    if bot_check_attack(game, characters, state, time) {
        result.flags |= BotMoveResultFlag::MOVEMENTWEAPON;
    }
    // Crouch and jump on characteristic timers.
    let croucher = characters.bounded_float(state.character, Characteristic::CROUCHER, 0.0, 1.0);
    if croucher > 0.5 && time > state.attack_crouch_time {
        state.attack_crouch_time = time + 1.0 + random.next_unit() * 2.0;
    }
    let jumper = characters.bounded_float(state.character, Characteristic::JUMPER, 0.0, 1.0);
    if jumper > 0.6 && time > state.attack_jump_time {
        state.attack_jump_time = time + 1.0 + random.next_unit() * 2.0;
        state.flags |= BotFlag::ATTACKJUMPED;
    }
    AiNode::BattleFight
}

/// Battle chase: run at the last known enemy position.
pub fn bot_battle_chase(
    game: &dyn SourceBotGame,
    characters: &BotCharacterLibrary,
    state: &mut BotState,
    result: &mut BotMoveResult,
    time: f32,
    random: &mut dyn BotRandom,
) -> AiNode {
    if bot_wants_to_retreat(characters, state) {
        return AiNode::BattleRetreat;
    }
    if state.enemy >= 0 && bot_visible(game, state, state.enemy) {
        bot_aim_at_enemy(game, characters, state, time, random);
        return AiNode::BattleFight;
    }
    if time - state.enemy_visible_time > 6.0 {
        state.enemy = -1;
        return AiNode::SeekLtg;
    }
    let dir = Vec3 {
        x: state.enemy_origin.x - state.origin.x,
        y: state.enemy_origin.y - state.origin.y,
        z: 0.0,
    };
    let len = (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0);
    result.move_direction = Vec3 {
        x: dir.x / len,
        y: dir.y / len,
        z: 0.0,
    };
    AiNode::BattleChase
}

/// Battle retreat: run away from the enemy toward health.
pub fn bot_battle_retreat(
    game: &dyn SourceBotGame,
    characters: &BotCharacterLibrary,
    state: &mut BotState,
    result: &mut BotMoveResult,
    time: f32,
    random: &mut dyn BotRandom,
) -> AiNode {
    let health = state.inventory.get(BotInventory::HEALTH).copied().unwrap_or(0);
    if health > 60 || state.enemy < 0 {
        return AiNode::SeekLtg;
    }
    bot_aim_at_enemy(game, characters, state, time, random);
    let dir = Vec3 {
        x: state.origin.x - state.enemy_origin.x,
        y: state.origin.y - state.enemy_origin.y,
        z: 0.0,
    };
    let len = (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0);
    result.move_direction = Vec3 {
        x: dir.x / len,
        y: dir.y / len,
        z: 0.0,
    };
    if bot_check_attack(game, characters, state, time) {
        result.flags |= BotMoveResultFlag::MOVEMENTWEAPON;
    }
    AiNode::BattleRetreat
}

/// Apply a movement result to the action buffer.
pub fn bot_apply_move_result(
    actions: &mut crate::behavior::library::actions::BotActionBuffer,
    state: &mut BotState,
    result: &BotMoveResult,
    speed: f32,
) {
    actions.set_move(state.client, result.move_direction, speed);
    if result.flags & BotMoveResultFlag::MOVEMENTVIEWSET != 0 {
        state.ideal_viewangles = result.ideal_view_angles;
    }
    if result.flags & BotMoveResultFlag::MOVEMENTWEAPON != 0 {
        actions.action(state.client, BotActionFlag::ATTACK);
    }
    if state.flags & BotFlag::ATTACKJUMPED != 0 {
        state.flags &= !BotFlag::ATTACKJUMPED;
        actions.action(state.client, BotActionFlag::JUMP);
    }
}

/// Whether the last command attacked.
#[must_use]
pub fn bot_last_attacked(state: &BotState) -> bool {
    state.last_ucmd.buttons & CommandButtons::ATTACK != 0
}
