//! Rogue shared routines (`src/content/q2/missionpacks/monsters/rogue-common.ts`).

use qa_core::identity::ActorId;
use qa_core::math::{dot3, sub3, vec3};

use super::state::rogue_state;
use crate::q2::base::monsters::parasite::parasite_drain_reachable;
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{angles_vectors, enemy_body, finish_dodge, health, project_flash, visible};
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext};
use crate::q2::support::contracts::{CombatTraitChanges, TraceHit, TraceResult};

/// Whether a trace hit only the world (`sourceTraceWorld`).
pub fn source_trace_world(game: &mut Q2GameServices, trace: &TraceResult) -> bool {
    match &trace.hit {
        TraceHit::None => true,
        TraceHit::World { .. } => true,
        TraceHit::Actor { actor } => *actor == game.host.world_actor(),
    }
}

/// Monster mass (`monsterMass`).
pub fn monster_mass(context: &mut MonsterContext) -> f64 {
    let actor = context.actor().clone();
    context
        .game
        .host
        .combat()
        .read(&actor)
        .unwrap_or_else(|| panic!("Source monster has no shared combat state"))
        .mass
}

/// Rogue parasite drain trace (`rogueParasiteDrainTrace`).
pub fn rogue_parasite_drain_trace(context: &mut MonsterContext) -> Option<TraceResult> {
    let enemy = enemy_body(context)?;
    let start = project_flash(context, vec3(24.0, 0.0, 6.0), None);
    let top = vec3(
        enemy.origin.x,
        enemy.origin.y,
        enemy.origin.z + enemy.bounds.max.z - 8.0,
    );
    let bottom = vec3(
        enemy.origin.x,
        enemy.origin.y,
        enemy.origin.z + enemy.bounds.min.z + 8.0,
    );
    if !parasite_drain_reachable(start, enemy.origin)
        && !parasite_drain_reachable(start, top)
        && !parasite_drain_reachable(start, bottom)
    {
        return None;
    }
    let actor = context.actor().clone();
    Some(context.game.host.trace(&Q2TraceRequest {
        start,
        end: enemy.origin,
        bounds: None,
        ignore: Some(actor),
        mask: 0x6000003,
        exclude: Vec::new(),
    }))
}

/// Rogue blocked shot check (`rogueBlockedCheckShot`).
pub fn rogue_blocked_check_shot(context: &mut MonsterContext, chance: f64) -> bool {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    let Some(enemy) = enemy else { return false };
    if !context.game.host.is_player(&enemy) || context.game.random() < chance {
        return false;
    }
    if context.entity().classname == "monster_parasite" {
        let trace = rogue_parasite_drain_trace(context);
        let Some(trace) = trace else { return false };
        if !matches!(&trace.hit, TraceHit::Actor { actor: hit } if *hit == enemy) {
            rogue_state(&mut *context.game, &actor).blocked = true;
            context.attack();
            rogue_state(&mut *context.game, &actor).blocked = false;
            return true;
        }
    }
    let tesla = if context.game.options.edition == Q2Edition::Rerelease {
        "tesla_mine"
    } else {
        "tesla"
    };
    let enemy_class = context.game.entities.get(&enemy).map(|entity| entity.classname.clone());
    if !visible(context, None) || enemy_class.as_deref() != Some(tesla) {
        return false;
    }
    rogue_state(&mut *context.game, &actor).blocked = true;
    context.attack();
    rogue_state(&mut *context.game, &actor).blocked = false;
    true
}

/// Rogue duck down (`rogueDuckDown`).
pub fn rogue_duck_down(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    context.state_mut().ducked = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor.clone());
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    let now = context.game.host.now();
    if context.state().duck_wait < now {
        context.state_mut().duck_wait = now + 1.0;
    }
    let normal_height = context.state().normal_height;
    let mut moved = body;
    moved.bounds.max.z = (normal_height - 32.0) as f32;
    context.game.write_body(actor, &moved, true);
}

/// Rogue duck hold (`rogueDuckHold`).
pub fn rogue_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().duck_wait;
    context.state_mut().hold_frame = hold;
}

/// Rogue duck up (`rogueDuckUp`).
pub fn rogue_duck_up(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    context.state_mut().ducked = false;
    context.state_mut().can_take_damage = true;
    let now = context.game.host.now();
    context.state_mut().next_duck_time = now + 0.5;
    let owned = context.game.owned_of(actor.clone());
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    let normal_height = context.state().normal_height;
    let mut moved = body;
    moved.bounds.max.z = normal_height as f32;
    context.game.write_body(actor, &moved, true);
}

/// Rogue dodge (`rogueMonsterDodge`).
pub fn rogue_monster_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    trace: Option<&TraceResult>,
    duck: Option<fn(&mut MonsterContext, f64)>,
    sidestep: Option<fn(&mut MonsterContext)>,
) {
    let actor = context.actor().clone();
    let random = context.game.random();
    if health(&mut *context.game, Some(&actor)) < 1.0
        || duck.is_none() && (sidestep.is_none() || context.state().stand_ground)
    {
        return;
    }
    if context.entity().enemy.is_none() {
        context.entity_mut().enemy = Some(attacker.clone());
        crate::q2::foundation::monsters::perception::found_target(context);
    }
    let skill = context.game.options.skill;
    if eta < 0.1 || eta > 5.0 || random > 0.25 * f64::from(skill + 1) || trace.is_none() {
        return;
    }
    let trace = trace.expect("rogue dodge trace");
    let body = context.game.body_of(actor);
    let height = body.origin.z + body.bounds.max.z - if duck.is_none() { 0.0 } else { 33.0 };
    let dodger = sidestep.is_some() && !context.state().stand_ground;
    if duck.is_some() && !dodger && (trace.end.z <= height || context.state().ducked) {
        return;
    }
    if dodger {
        if context.state().dodging {
            return;
        }
        if trace.end.z <= height || context.state().ducked {
            let right = angles_vectors(body.angles).right;
            let lefty = dot3(right, sub3(trace.end, body.origin)) >= 0.0;
            context.state_mut().lefty = lefty;
            if duck.is_some() && context.state().ducked {
                rogue_duck_up(context);
            }
            context.state_mut().dodging = true;
            context.state_mut().attack_state = MonsterAttackState::Sliding;
            if let Some(sidestep) = sidestep {
                sidestep(context);
            }
            return;
        }
    }
    if duck.is_some() && context.state().next_duck_time <= context.game.host.now() {
        finish_dodge(context);
        context.state_mut().ducked = true;
        if let Some(duck) = duck {
            duck(context, eta);
        }
    }
}
