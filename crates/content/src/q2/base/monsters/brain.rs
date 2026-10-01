//! Brain monster (`src/content/q2/base/monsters/brain.ts`).
//!
//! Quake II m_brain.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::vec3;

use super::common::{
    HUMANOID_BOUNDS, begin_death, damaged_skin, finish_corpse_default, move_handler, sound_handler,
    standard_gib,
};
use super::tables::brain::brain_moves;
use crate::contract::{InventoryEntry, PoweredProtectionState};
use crate::q2::foundation::monsters::ai::set_duck;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, Q2MonsterDefinition, bind_shared_power_cells,
};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceResult};

/// Bind power-armor cells (`bindPowerArmor`).
fn bind_power_armor(context: &mut MonsterContext) {
    bind_shared_power_cells(context);
}

/// Toggle the power screen (`screen`).
fn screen(context: &mut MonsterContext, active: bool) {
    let actor = context.actor().clone();
    // Combat drains land in the shared cell store; sync them back to
    // the inventory like the donor binding before reading the count.
    if let Some(cells) = context.game.monsters.power_cells.get(&actor) {
        let count = *cells.borrow();
        let owned = context.game.owned_of(actor.clone());
        context.game.host.inventory().configure(
            &owned,
            &InventoryEntry {
                item: "q2:monster-power".to_string(),
                count,
                capacity: 100.0,
                count_policy: None,
            },
        );
    }
    let cells = context.game.host.inventory().count(&actor, &"q2:monster-power".to_string());
    let owned = context.game.owned_of(actor.clone());
    context.game.host.combat().set_powered_protection(
        &owned,
        &if active {
            PoweredProtectionState::Screen { cells }
        } else {
            PoweredProtectionState::None
        },
    );
}

/// Run (`run`).
fn brain_run(context: &mut MonsterContext) {
    screen(context, true);
    if context.state().stand_ground {
        context.set_move("brain_move_stand", true);
    } else {
        context.set_move("brain_move_run", true);
    }
}

/// Melee (`melee`).
fn brain_melee(context: &mut MonsterContext) {
    if context.game.random() <= 0.5 {
        context.set_move("brain_move_attack1", true);
    } else {
        context.set_move("brain_move_attack2", true);
    }
}

/// Initialize (`initialize`).
fn brain_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if !context.game.host.inventory().has(&actor) {
        let owned = context.game.owned_of(actor.clone());
        context.game.host.inventory().create(&owned, &[]);
    }
    let owned = context.game.owned_of(actor.clone());
    context.game.host.inventory().configure(
        &owned,
        &InventoryEntry {
            item: "q2:monster-power".to_string(),
            count: 100.0,
            capacity: 100.0,
            count_policy: None,
        },
    );
    bind_power_armor(context);
    screen(context, true);
}

/// Idle (`idle`).
fn brain_idle(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "brain/brnlens1.wav", 0, 1.0, 2.0);
    context.set_move("brain_move_idle", true);
}

/// Pain (`pain`).
fn brain_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let r = context.game.random();
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if r < 0.33 || r >= 0.66 {
            "brain/brnpain1.wav"
        } else {
            "brain/brnpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if r < 0.33 {
            "brain_move_pain1"
        } else if r < 0.66 {
            "brain_move_pain2"
        } else {
            "brain_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn brain_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    context.entity_mut().effects = 0;
    screen(context, false);
    if standard_gib(
        context,
        reaction,
        2,
        4,
        "models/objects/gibs/head2/tris.md2",
        1.0,
    ) || context.state().dead
    {
        return;
    }
    let first = context.game.random() <= 0.5;
    begin_death(
        context,
        reaction,
        "brain/brndeth1.wav",
        if first {
            "brain_move_death1"
        } else {
            "brain_move_death2"
        },
        2,
        4,
    );
}

/// Dodge (`dodge`).
fn brain_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    _trace: Option<&TraceResult>,
    _direct: bool,
) {
    if context.game.random() > 0.25 {
        return;
    }
    if context.entity().enemy.is_none() {
        context.entity_mut().enemy = Some(attacker.clone());
    }
    let now = context.game.host.now();
    context.state_mut().pause_time = now + eta + 0.5;
    context.set_move("brain_move_duck", true);
}

/// Duck down (`brain_duck_down`).
fn brain_duck_down(context: &mut MonsterContext) {
    if !context.state().ducked {
        set_duck(context, true);
    }
}

/// Duck hold (`brain_duck_hold`).
fn brain_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Duck up (`brain_duck_up`).
fn brain_duck_up(context: &mut MonsterContext) {
    set_duck(context, false);
}

/// Right hit (`brain_hit_right`).
fn brain_hit_right(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let side = context.game.body_of(actor.clone()).bounds.max.x;
    let damage = 15.0 + (context.game.random() * 5.0).floor();
    if fire_hit(
        actor.clone(),
        &mut *context.game,
        vec3(80.0, side, 8.0),
        damage,
        40.0,
    ) {
        context.game.sound(&actor, "brain/melee3.wav", 1, 1.0, 1.0);
    }
}

/// Left hit (`brain_hit_left`).
fn brain_hit_left(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let side = context.game.body_of(actor.clone()).bounds.min.x;
    let damage = 15.0 + (context.game.random() * 5.0).floor();
    if fire_hit(
        actor.clone(),
        &mut *context.game,
        vec3(80.0, side, 8.0),
        damage,
        40.0,
    ) {
        context.game.sound(&actor, "brain/melee3.wav", 1, 1.0, 1.0);
    }
}

/// Chest open (`brain_chest_open`).
fn brain_chest_open(context: &mut MonsterContext) {
    context.entity_mut().spawnflags &= !65536;
    screen(context, false);
    let actor = context.actor().clone();
    context.game.sound(&actor, "brain/brnatck1.wav", 4, 1.0, 1.0);
}

/// Tentacle attack (`brain_tentacle_attack`).
fn brain_tentacle_attack(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let damage = 10.0 + (context.game.random() * 5.0).floor();
    let hit = fire_hit(
        actor.clone(),
        &mut *context.game,
        vec3(80.0, 0.0, 8.0),
        damage,
        -600.0,
    );
    if hit && context.game.options.skill > 0 {
        context.entity_mut().spawnflags |= 65536;
    }
    context.game.sound(&actor, "brain/brnatck3.wav", 1, 1.0, 1.0);
}

/// Chest closed (`brain_chest_closed`).
fn brain_chest_closed(context: &mut MonsterContext) {
    screen(context, true);
    if context.entity().spawnflags & 65536 != 0 {
        context.entity_mut().spawnflags &= !65536;
        context.set_move("brain_move_attack1", true);
    }
}

/// Brain definition (`brainDefinition`).
pub fn brain_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_brain",
        "brain",
        "models/monsters/brain/tris.md2",
        300.0,
        -150.0,
        400.0,
        HUMANOID_BOUNDS,
        1.0,
        "brain_move_stand",
        brain_moves(),
        move_handler("brain_move_stand"),
        move_handler("brain_move_walk1"),
        MonsterHandler::Callback(brain_run),
        MonsterHandler::Callback(brain_melee),
        brain_die,
    );
    definition.melee = Some(MonsterHandler::Callback(brain_melee));
    definition.sight = Some(sound_handler("brain/brnsght1.wav", 2, 1.0));
    definition.search = Some(sound_handler("brain/brnsrch1.wav", 2, 1.0));
    definition.idle = Some(MonsterHandler::Callback(brain_idle));
    definition.pain = Some(brain_pain);
    definition.dodge = Some(brain_dodge);
    definition.initialize = Some(MonsterHandler::Callback(brain_initialize));
    definition.restore = Some(MonsterHandler::Callback(bind_power_armor));
    definition.callbacks = HashMap::from([
        (
            "brain_stand".to_string(),
            move_handler("brain_move_stand"),
        ),
        ("brain_run".to_string(), MonsterHandler::Callback(brain_run)),
        (
            "brain_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "brain_duck_down".to_string(),
            MonsterHandler::Callback(brain_duck_down),
        ),
        (
            "brain_duck_hold".to_string(),
            MonsterHandler::Callback(brain_duck_hold),
        ),
        (
            "brain_duck_up".to_string(),
            MonsterHandler::Callback(brain_duck_up),
        ),
        (
            "brain_swing_right".to_string(),
            sound_handler("brain/melee1.wav", 4, 1.0),
        ),
        (
            "brain_swing_left".to_string(),
            sound_handler("brain/melee2.wav", 4, 1.0),
        ),
        (
            "brain_hit_right".to_string(),
            MonsterHandler::Callback(brain_hit_right),
        ),
        (
            "brain_hit_left".to_string(),
            MonsterHandler::Callback(brain_hit_left),
        ),
        (
            "brain_chest_open".to_string(),
            MonsterHandler::Callback(brain_chest_open),
        ),
        (
            "brain_tentacle_attack".to_string(),
            MonsterHandler::Callback(brain_tentacle_attack),
        ),
        (
            "brain_chest_closed".to_string(),
            MonsterHandler::Callback(brain_chest_closed),
        ),
    ]);
    definition
}
