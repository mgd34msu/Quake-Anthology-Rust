//! Rogue combat hooks (`src/content/q2/missionpacks/monsters/combat.ts`).
//!
//! Quake II rogue/g_combat.c and g_newai.c. ZeniMax Media, GPL-2.0-or-later.

use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, dot3, normalize3, scale3, sub3};

use super::state::rogue_state;
use super::types::Q2MissionPackMonsterServices;
use crate::q2::foundation::host::{Q2Edition, Q2Entity, Q2GameServices};
use crate::q2::foundation::monsters::ai::{angles_vectors, health, visible};
use crate::q2::foundation::monsters::perception::found_target;
use crate::q2::foundation::monsters::types::{
    MonsterContext, Q2MonsterSourceCombatHooks, SourceMoveOutcome,
};
use crate::q2::support::contracts::CombatTraitChanges;

/// Tesla classname for the edition.
fn tesla_class(game: &Q2GameServices) -> &'static str {
    if game.options.edition == Q2Edition::Rerelease {
        "tesla_mine"
    } else {
        "tesla"
    }
}

/// Heal effects (`rogueHealEffects`).
pub fn rogue_heal_effects(context: &mut MonsterContext) {
    let resurrecting = context.state().resurrecting;
    {
        let entity = context.entity_mut();
        entity.effects &= !256;
        entity.render_flags &= !(1024 | 2048 | 4096);
        if resurrecting {
            entity.effects |= 256;
            entity.render_flags |= 1024;
        }
    }
    let actor = context.actor().clone();
    context.game.show(actor);
}

/// Clean up a heal target (`cleanupRogueHealTarget`).
pub fn cleanup_rogue_heal_target(game: &mut Q2GameServices, target: &ActorId) {
    rogue_state(game, target).healer = None;
    let owned = game.owned_of(target.clone());
    game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    if game.monsters.states.contains_key(target) {
        let mut patient = MonsterContext::new(target.clone(), game);
        patient.state_mut().can_take_damage = true;
        patient.state_mut().resurrecting = false;
        rogue_heal_effects(&mut patient);
    }
}

/// Clean up the medic target (`cleanup`).
fn cleanup_medic(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    if let Some(enemy) = enemy {
        if context.game.entities.contains_key(&enemy) {
            cleanup_rogue_heal_target(&mut *context.game, &enemy);
        }
    }
    context.state_mut().medic = false;
}

/// Target a tesla (`targetTesla`).
fn target_tesla(context: &mut MonsterContext, tesla: &ActorId) {
    if context.state().medic {
        cleanup_medic(context);
    }
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    if enemy.as_ref().is_some_and(|enemy| context.game.host.is_player(enemy)) {
        rogue_state(&mut *context.game, &actor).last_player_enemy = enemy.clone();
    }
    if enemy.as_ref() == Some(tesla) {
        return;
    }
    context.state_mut().old_enemy = enemy;
    context.entity_mut().enemy = Some(tesla.clone());
    if !context.state().has_ranged_attack {
        found_target(context);
        return;
    }
    if health(&mut *context.game, Some(&actor)) > 0.0 {
        context.attack();
    }
}

/// Rogue combat hooks (`createRogueCombatHooks`).
pub struct RogueCombatHooks {
    /// Monster services.
    services: Rc<dyn Q2MissionPackMonsterServices>,
}

impl RogueCombatHooks {
    /// Build rogue combat hooks.
    pub fn new(services: Rc<dyn Q2MissionPackMonsterServices>) -> Self {
        Self { services }
    }
}

/// Create rogue combat hooks (`createRogueCombatHooks`).
pub fn create_rogue_combat_hooks(
    services: Rc<dyn Q2MissionPackMonsterServices>,
) -> RogueCombatHooks {
    RogueCombatHooks::new(services)
}

impl Q2MonsterSourceCombatHooks for RogueCombatHooks {
    fn is_good_guy(&mut self, game: &mut Q2GameServices, entity: &Q2Entity) -> bool {
        if let Some(state) = game.monsters.states.get(entity.actor.id()) {
            return state.good_guy;
        }
        rogue_state(game, entity.actor.id()).good_guy
    }

    fn before_react(&mut self, context: &mut MonsterContext, attacker: &ActorId) -> bool {
        let actor = context.actor().clone();
        let inflictor = context
            .entity()
            .last_attack
            .clone()
            .and_then(|attack| attack.inflictor);
        let tesla = tesla_class(context.game);
        let tesla_hit = inflictor.as_ref().and_then(|inflictor| context.game.entities.get(inflictor)).is_some_and(|entity| entity.classname == tesla);
        if tesla_hit {
            let inflictor = inflictor.expect("tesla inflictor");
            let marked = self.services.mark_tesla_area(&actor, &inflictor);
            if context.game.options.edition == Q2Edition::Classic {
                if marked {
                    target_tesla(context, &inflictor);
                }
            } else {
                let odd = {
                    let random = context
                        .game
                        .host
                        .rerelease_random()
                        .unwrap_or_else(|| panic!("Rerelease Tesla reaction requires the shared source RNG"));
                    (random.integer_any() & 1) != 0
                };
                let enemy = context.entity().enemy.clone();
                let enemy_is_mine = enemy
                    .as_ref()
                    .and_then(|enemy| context.game.entities.get(enemy))
                    .is_some_and(|entity| entity.classname == "tesla_mine");
                if (marked || odd) && !enemy_is_mine {
                    target_tesla(context, &inflictor);
                }
            }
            return true;
        }
        if attacker == &actor || Some(attacker) == context.entity().enemy.as_ref() {
            return false;
        }
        if context.state().good_guy {
            let is_player = context.game.host.is_player(attacker);
            let other = context.game.entities.get(attacker).cloned();
            let mut other_good = false;
            if let Some(other) = other {
                other_good = self.is_good_guy(&mut *context.game, &other);
            }
            if is_player || other_good {
                return false;
            }
        }
        let max_health = context.entity().max_health;
        let percent = health(&mut *context.game, Some(&actor)) / max_health;
        if context.entity().enemy.is_some() && context.state().target_anger {
            let enemy = context.entity().enemy.clone().expect("angry enemy");
            if context.game.host.actors().is_live(&enemy) && percent > 0.33 {
                return true;
            }
            context.state_mut().target_anger = false;
        }
        if context.game.options.edition == Q2Edition::Rerelease {
            let actor_id = actor.clone();
            let react = rogue_state(&mut *context.game, &actor_id).react_to_damage_time;
            if react > context.game.host.now() {
                return true;
            }
        }
        if context.entity().enemy.is_some() && context.state().medic {
            let enemy = context.entity().enemy.clone().expect("medic enemy");
            if context.game.host.actors().is_live(&enemy) && percent > 0.25 {
                return true;
            }
            cleanup_medic(context);
        }
        if context.game.options.edition == Q2Edition::Rerelease {
            let delay = {
                let random = context
                    .game
                    .host
                    .rerelease_random()
                    .unwrap_or_else(|| panic!("Rerelease damage reaction requires the shared source RNG"));
                random.time_milliseconds(3000, 5000)
            };
            let now = context.game.host.now();
            rogue_state(&mut *context.game, &actor).react_to_damage_time =
                now + delay as f64 / 1000.0;
        }
        false
    }

    fn before_killed(&mut self, context: &mut MonsterContext) {
        if context.state().medic {
            cleanup_medic(context);
        }
    }

    fn recover_enemy(&mut self, context: &mut MonsterContext) -> Option<ActorId> {
        let actor = context.actor().clone();
        let candidate =
            rogue_state(&mut *context.game, &actor).last_player_enemy.clone();
        let Some(candidate) = candidate else { return None };
        if health(&mut *context.game, Some(&candidate)) <= 0.0 {
            return None;
        }
        rogue_state(&mut *context.game, &actor).last_player_enemy = None;
        Some(candidate)
    }

    fn before_move(
        &mut self,
        context: &mut MonsterContext,
        displacement: Vec3,
    ) -> SourceMoveOutcome {
        let actor = context.actor().clone();
        if health(&mut *context.game, Some(&actor)) <= 0.0 {
            return SourceMoveOutcome::Move { displacement };
        }
        let current = self.services.bad_area_entity(&actor, None);
        if let Some(current) = current {
            rogue_state(&mut *context.game, &actor).bad_area = Some(current.clone());
            let tesla = tesla_class(context.game);
            let enemy = context.entity().enemy.clone();
            let enemy_is_tesla = enemy
                .as_ref()
                .and_then(|enemy| context.game.entities.get(enemy))
                .is_some_and(|entity| entity.classname == tesla);
            if enemy_is_tesla {
                let body = context.game.body_of(actor);
                let forward = angles_vectors(body.angles).forward;
                let bad_origin = context.game.body_of(current).origin;
                let bad_dot = dot3(forward, normalize3(sub3(bad_origin, body.origin)));
                let move_dot = dot3(forward, normalize3(displacement));
                if bad_dot < 0.0 && move_dot < 0.0 || bad_dot > 0.0 && move_dot > 0.0 {
                    return SourceMoveOutcome::Move {
                        displacement: scale3(displacement, -1.0),
                    };
                }
            }
        } else {
            let had = rogue_state(&mut *context.game, &actor).bad_area.take().is_some();
            if had {
                let old_enemy = context.state().old_enemy.clone();
                if let Some(old_enemy) = old_enemy {
                    context.entity_mut().enemy = Some(old_enemy.clone());
                    context.entity_mut().goal = Some(old_enemy);
                    found_target(context);
                    return SourceMoveOutcome::Handled;
                }
            }
        }
        SourceMoveOutcome::Move { displacement }
    }

    fn accepts_ground_move(&mut self, context: &mut MonsterContext, origin: Vec3) -> bool {
        let actor = context.actor().clone();
        if health(&mut *context.game, Some(&actor)) <= 0.0
            || rogue_state(&mut *context.game, &actor).bad_area.is_some()
        {
            return true;
        }
        let area = self.services.bad_area_entity(&actor, Some(origin));
        let Some(area) = area else { return true };
        let owner = context
            .game
            .entities
            .get(&area)
            .and_then(|entity| entity.owner.clone());
        let Some(owner) = owner else { return false };
        let owner_class = context
            .game
            .entities
            .get(&owner)
            .map(|entity| entity.classname.clone());
        let tesla = tesla_class(context.game);
        if owner_class.as_deref() == Some(tesla) {
            let enemy = context.entity().enemy.clone();
            let strike = match enemy {
                None => true,
                Some(enemy) => {
                    let class = context
                        .game
                        .entities
                        .get(&enemy)
                        .map(|entity| entity.classname.clone());
                    match class {
                        None => true,
                        Some(class) => {
                            // The donor keeps the original "telsa" typo here.
                            let expected =
                                if context.game.options.edition == Q2Edition::Classic {
                                    "telsa"
                                } else {
                                    "tesla_mine"
                                };
                            class != expected
                                && (!context.game.host.is_player(&enemy)
                                    || !visible(context, None))
                        }
                    }
                }
            };
            if strike {
                target_tesla(context, &owner);
                rogue_state(&mut *context.game, &actor).blocked = true;
            }
        }
        false
    }

    fn consume_blocked(&mut self, context: &mut MonsterContext) -> bool {
        let actor = context.actor().clone();
        let state = rogue_state(&mut *context.game, &actor);
        let blocked = state.blocked;
        state.blocked = false;
        blocked
    }
}

/// Rogue target anger (`rogueTargetAnger`).
pub fn rogue_target_anger(game: &mut Q2GameServices, entity: &ActorId, target: &ActorId) {
    if !game.host.actors().is_live(entity) || !game.host.actors().is_live(target) {
        return;
    }
    if game.monsters.states.contains_key(target) {
        game.monsters.require_state_mut(target).good_guy = true;
    } else {
        rogue_state(game, target).good_guy = true;
    }
    if !game.monsters.states.contains_key(entity) {
        return;
    }
    game.require_entity_mut(entity).enemy = Some(target.clone());
    let mut context = MonsterContext::new(entity.clone(), game);
    context.state_mut().target_anger = true;
    found_target(&mut context);
}
