//! Widow helpers (`src/content/q2/missionpacks/monsters/widow/common.ts`).
//!
//! Original Rogue m_widow.c and g_newai.c. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, scale3, vec3, Bounds, Vec3};

use super::power_armor::{monster_power_armor, restore_monster_power_armor, PowerArmorKind};
use super::spawn::{create_rogue_ground_monster, find_rogue_spawn_point, rogue_spawn_grow};
use super::state::rogue_state;
use super::types::mission_services;
use crate::q2::foundation::host::{Q2GameServices, Q2Mode};
use crate::q2::foundation::monsters::ai::{angles_vectors, health, visible};
use crate::q2::foundation::monsters::perception::found_target;
use crate::q2::foundation::monsters::types::{record_at, MonsterContext, MonsterSpawner};
use crate::q2::support::contracts::CombatTraitChanges;

/// Stalker bounds (`stalkerBounds`).
const STALKER_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -28.0,
        y: -28.0,
        z: -18.0,
    },
    max: Vec3 {
        x: 28.0,
        y: 28.0,
        z: 18.0,
    },
};

/// Widow project (`widowProject`).
pub fn widow_project(actor: &ActorId, game: &mut Q2GameServices, offset: Vec3) -> Vec3 {
    let body = game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    add3(
        body.origin,
        add3(
            scale3(axes.forward, offset.x),
            add3(scale3(axes.right, offset.y), scale3(axes.up, offset.z)),
        ),
    )
}

/// Widow slots (`widowSlots`).
pub fn widow_slots(context: &mut MonsterContext) {
    let skill = context.game.options.skill;
    let mut slots = if skill < 2 {
        3
    } else if skill == 2 {
        4
    } else {
        6
    };
    if context.game.options.mode == Q2Mode::Coop {
        let players = context
            .game
            .host
            .players()
            .into_iter()
            .filter(|actor| context.game.host.actors().is_live(actor) && context.game.host.is_player(actor))
            .count();
        slots = (slots + i32::from(skill) * (players as i32 - 1)).min(6);
    }
    context.state_mut().monster_slots = slots;
}

/// Widow slots left (`widowSlotsLeft`).
pub fn widow_slots_left(context: &mut MonsterContext) -> i32 {
    context.state().monster_slots - context.state().monster_used
}

/// Coop target (`coopTarget`).
fn widow_coop_target(context: &mut MonsterContext) -> Option<ActorId> {
    let candidates: Vec<ActorId> = context
        .game
        .host
        .players()
        .into_iter()
        .filter(|actor| {
            context.game.host.actors().is_live(actor)
                && context.game.host.is_player(actor)
                && visible(context, Some(actor))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let pick = (context.game.random() * candidates.len() as f64).floor() as usize;
    Some(record_at(&candidates, pick.min(candidates.len() - 1)).clone())
}

/// Widow summon (`widowSummon`).
pub fn widow_summon(context: &mut MonsterContext, second: bool, grow: bool) {
    let actor = context.actor().clone();
    for side in [1.0, -1.0] {
        let anchor = widow_project(
            &actor,
            &mut *context.game,
            vec3(
                30.0,
                side * if second { 135.0 } else { 100.0 },
                if second { 0.0 } else { 16.0 },
            ),
        );
        let point = find_rogue_spawn_point(&mut *context.game, anchor, STALKER_BOUNDS, 64.0);
        let Some(point) = point else {
            continue;
        };
        if grow {
            rogue_spawn_grow(&mut *context.game, point, 1);
            continue;
        }
        let angles = context.game.body_of(actor.clone()).angles;
        let child = create_rogue_ground_monster(
            &mut *context.game,
            point,
            angles,
            STALKER_BOUNDS,
            "monster_stalker",
            256.0,
        );
        let Some(child) = child else {
            continue;
        };
        if !context.game.monsters.states.contains_key(&child) {
            panic!("Widow child lacks its shared monster controller");
        }
        context.state_mut().monster_used += 1;
        if let Some(state) = context.game.monsters.states.get_mut(&child) {
            state.commander = Some(actor.clone());
        }
        let now = context.game.host.now();
        context.game.require_entity_mut(&child).next_think = Some(now);
        let think = context.game.require_entity(&child).think;
        if let Some(think) = think {
            think(child.clone(), &mut *context.game);
        }
        if let Some(state) = context.game.monsters.states.get_mut(&child) {
            state.spawned_by = MonsterSpawner::Widow;
            state.do_not_count = true;
            state.ignore_shots = true;
        }
        let mut target = context.entity().enemy.clone();
        if context.game.options.mode == Q2Mode::Coop {
            let mut child_context = MonsterContext::new(child.clone(), &mut *context.game);
            target = widow_coop_target(&mut child_context);
            let self_enemy = child_context.entity().enemy.clone();
            if target.is_some() && target == self_enemy {
                target = widow_coop_target(&mut child_context);
            }
            if target.is_none() {
                target = self_enemy;
            }
        }
        let live = target.as_ref().is_some_and(|target| {
            context.game.host.actors().is_live(target) && health(&mut *context.game, Some(target)) > 0.0
        });
        if live {
            let target = target.expect("widow summon target");
            context.game.require_entity_mut(&child).enemy = Some(target);
            let mut child_context = MonsterContext::new(child.clone(), &mut *context.game);
            found_target(&mut child_context);
            child_context.attack();
        }
    }
}

/// Widow clear powerups (`widowClearPowerups`).
pub fn widow_clear_powerups(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let power = rogue_state(&mut *context.game, &actor);
    power.widow_quad_until = 0.0;
    power.widow_double_until = 0.0;
    power.widow_invulnerable_until = 0.0;
    context.entity_mut().effects &= !(32768 | 65536 | 134217728);
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            invulnerable: Some(false),
            ..CombatTraitChanges::default()
        },
    );
}

/// Shown (`shown`).
fn powerup_shown(until: f64, now: f64) -> bool {
    let remaining = ((until - now) * 10.0).round() as i64;
    remaining > 0 && (remaining > 30 || remaining & 4 != 0)
}

/// Widow armor (`armor`).
fn widow_armor(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let power = context
        .game
        .host
        .inventory()
        .count(&actor, &"q2:monster-power".to_string());
    if power <= 0.0 {
        let skill = f64::from(context.game.options.skill);
        monster_power_armor(context, PowerArmorKind::Shield, 250.0 * skill);
    }
}

/// Respond (`respond`).
fn widow_respond(context: &mut MonsterContext, actor: &ActorId) {
    let skill = context.game.options.skill;
    let other = mission_services(&*context.game).powerups(actor);
    let now = context.game.host.now();
    let self_actor = context.actor().clone();
    if powerup_shown(other.quad_until, now) {
        if skill == 1 {
            rogue_state(&mut *context.game, &self_actor).widow_double_until = other.quad_until;
            context.game.mission_monsters.widow_damage_multiplier = 2;
        } else if skill >= 2 {
            rogue_state(&mut *context.game, &self_actor).widow_quad_until = other.quad_until;
            context.game.mission_monsters.widow_damage_multiplier = 4;
            if skill == 3 {
                widow_armor(context);
            }
        }
    } else if powerup_shown(other.double_until, now) {
        if skill >= 2 {
            rogue_state(&mut *context.game, &self_actor).widow_double_until = other.double_until;
            context.game.mission_monsters.widow_damage_multiplier = 2;
            if skill == 3 {
                widow_armor(context);
            }
        }
    } else {
        context.game.mission_monsters.widow_damage_multiplier = 1;
    }
    if powerup_shown(other.invulnerability_until, now) {
        if skill == 1 {
            widow_armor(context);
        } else if skill >= 2 {
            rogue_state(&mut *context.game, &self_actor).widow_invulnerable_until = other.invulnerability_until;
            if skill == 3 {
                widow_armor(context);
            }
        }
    }
}

/// Widow powerups (`widowPowerups`).
pub fn widow_powerups(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.options.mode != Q2Mode::Coop {
        if let Some(enemy) = context.entity().enemy.clone() {
            widow_respond(context, &enemy);
        }
    } else {
        let players: Vec<ActorId> = context
            .game
            .host
            .players()
            .into_iter()
            .filter(|actor| context.game.host.actors().is_live(actor) && context.game.host.is_player(actor))
            .collect();
        let now = context.game.host.now();
        let services = mission_services(&*context.game);
        for field in ["invulnerability_until", "quad_until", "double_until"] {
            let player = players.iter().find(|actor| {
                let powerups = services.powerups(actor);
                let until = match field {
                    "invulnerability_until" => powerups.invulnerability_until,
                    "quad_until" => powerups.quad_until,
                    _ => powerups.double_until,
                };
                powerup_shown(until, now)
            });
            if let Some(player) = player.cloned() {
                widow_respond(context, &player);
                break;
            }
        }
    }
    let now = context.game.host.now();
    let invulnerable = rogue_state(&mut *context.game, &actor).widow_invulnerable_until > now;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            invulnerable: Some(invulnerable),
            ..CombatTraitChanges::default()
        },
    );
}

/// Widow power think (`widowPowerThink`).
pub fn widow_power_think(actor: ActorId, game: &mut Q2GameServices) {
    if !game.monsters.states.contains_key(&actor) {
        return;
    }
    let (quad, double, invulnerable) = {
        let power = rogue_state(game, &actor);
        (
            power.widow_quad_until,
            power.widow_double_until,
            power.widow_invulnerable_until,
        )
    };
    let now = game.host.now();
    game.require_entity_mut(&actor).effects &= !(32768 | 65536 | 134217728);
    if health(game, Some(&actor)) > 0.0 {
        for (until, flag) in [(quad, 32768), (double, 134217728), (invulnerable, 65536)] {
            let remaining = ((until - now) * 10.0).round() as i64;
            if remaining > 0 && (remaining > 30 || remaining & 4 != 0) {
                game.require_entity_mut(&actor).effects |= flag;
            }
        }
    }
    let owned = game.owned_of(actor.clone());
    game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            invulnerable: Some(invulnerable > now),
            ..CombatTraitChanges::default()
        },
    );
}

/// Widow restore armor (`widowRestoreArmor`).
pub fn widow_restore_armor(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let powered = context
        .game
        .host
        .inventory()
        .entries(&actor)
        .iter()
        .any(|entry| entry.item == "q2:monster-power");
    if powered {
        restore_monster_power_armor(context);
    }
}
