//! Rogue base variants (`src/content/q2/missionpacks/monsters/rogue-variants.ts`).
//!
//! Original Rogue changes to the base species. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, length3, normalize3, scale3, sub3};

use super::rogue_common::{
    rogue_blocked_check_shot, rogue_duck_down, rogue_duck_hold, rogue_duck_up,
    rogue_monster_dodge,
};
use super::rogue_infantry::create_rogue_infantry_definition;
use super::rogue_soldier::create_rogue_soldier_definitions;
use super::tables::rogue_brain::{brain_frame, brain_moves};
use super::tables::rogue_float::{float_frame, float_moves};
use crate::q2::base::monsters::boss2::boss2_definition;
use crate::q2::base::monsters::boss_common::{boss_check_attack, with_boss_explosion_callbacks};
use crate::q2::base::monsters::brain::brain_definition;
use crate::q2::base::monsters::common::{damaged_skin, monster_shot};
use crate::q2::base::monsters::floater::floater_definition;
use crate::q2::base::monsters::gladiator::gladiator_definition;
use crate::q2::base::monsters::jorg::{create_jorg_definition, jorg_initialize};
use crate::q2::base::monsters::makron::{makron_definition, with_makron_spawn_callbacks};
use crate::q2::base::monsters::supertank::supertank_definition;
use crate::q2::base::monsters::tables::supertank::supertank_frame;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, enemy_eye, project_flash,
};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::rerelease::monsters::common::{blocked_check_platform, monster_flash};
use crate::q2::support::contracts::{PainReaction, TraceResult};

/// Rogue shot-blocked check (`shotBlocked`).
fn rogue_shot_blocked(context: &mut MonsterContext) -> bool {
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    rogue_blocked_check_shot(context, chance)
}

/// Rogue blocked (`blocked`).
fn rogue_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    rogue_shot_blocked(context) || blocked_check_platform(context, distance)
}

/// Brain duck (`brainDuck`).
fn rogue_brain_duck_inner(context: &mut MonsterContext, eta: f64) {
    rogue_duck_down(context);
    let skill = context.game.options.skill;
    let now = context.game.host.now();
    context.state_mut().duck_wait = now + eta
        + if skill == 0 {
            1.0
        } else {
            0.1 * f64::from(3 - skill)
        };
    context.state_mut().next_frame = brain_frame::DUCK01;
    context.set_move("brain_move_duck", false);
}

/// Brain pain (`pain`).
fn rogue_brain_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let random = context.game.random();
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if random < 0.33 || random >= 0.66 {
            "brain/brnpain1.wav"
        } else {
            "brain/brnpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if random < 0.33 {
            "brain_move_pain1"
        } else if random < 0.66 {
            "brain_move_pain2"
        } else {
            "brain_move_pain3"
        },
        false,
    );
    if context.state().ducked {
        rogue_duck_up(context);
    }
}

/// Brain dodge (`dodge`).
fn rogue_brain_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    trace: Option<&TraceResult>,
    _direct: bool,
) {
    rogue_monster_dodge(
        context,
        attacker,
        eta,
        trace,
        Some(rogue_brain_duck_inner),
        None,
    );
}

/// Brain duck slot (`duck`).
fn rogue_brain_duck(context: &mut MonsterContext, eta: f64) -> bool {
    rogue_brain_duck_inner(context, eta);
    true
}

/// Floater attack (`attack`).
fn rogue_floater_attack(context: &mut MonsterContext) {
    let skill = context.game.options.skill;
    let chance = if skill == 0 {
        0.0
    } else {
        1.0 - 0.5 / f64::from(skill)
    };
    if context.game.random() > chance {
        context.state_mut().attack_state = MonsterAttackState::Straight;
        context.set_move("floater_move_attack1", false);
        return;
    }
    if context.game.random() <= 0.5 {
        let lefty = context.state().lefty;
        context.state_mut().lefty = !lefty;
    }
    context.state_mut().attack_state = MonsterAttackState::Sliding;
    context.set_move("floater_move_attack1a", false);
}

/// Floater fire blaster (`floater_fire_blaster`).
fn rogue_floater_fire_blaster(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 82, 0.0) else {
        return;
    };
    let frame = context.entity().frame;
    let effects = if frame == float_frame::ATTAK104 || frame == float_frame::ATTAK107 {
        64
    } else {
        0
    };
    let fire_blaster = context.weapons.fire_blaster;
    let actor = context.actor().clone();
    fire_blaster(
        actor,
        &mut *context.game,
        start,
        direction,
        1.0,
        1000.0,
        effects,
        false,
        crate::q2::foundation::weapons::types::Mod::BLASTER,
    );
    monster_flash(context, 82, start, direction);
}

/// Boss rockets (`bossRockets`).
fn boss_rockets(context: &mut MonsterContext, predictive: bool) {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let Some(enemy) = enemy else { return };
    let right = angles_vectors(context.game.body_of(actor).angles).right;
    if predictive {
        context.game.host.diagnostic("predictive fire");
    }
    for index in 0..4usize {
        let flash = 78 + index;
        let edition = context.game.options.edition;
        let start = project_flash(context, muzzle_offset(edition, flash), None);
        let direction = if predictive {
            let flight = f64::from(length3(sub3(enemy.origin, start))) / 750.0 - 0.3
                + index as f64 * 0.15;
            normalize3(sub3(
                add3(enemy.origin, scale3(enemy.velocity, flight as f32)),
                start,
            ))
        } else {
            let lowered = Vec3 {
                x: enemy.origin.x,
                y: enemy.origin.y,
                z: enemy.origin.z - if index == 0 || index == 3 { 15.0 } else { 0.0 },
            };
            normalize3(add3(
                normalize3(sub3(lowered, start)),
                scale3(right, [0.4f32, 0.025, -0.025, -0.4][index]),
            ))
        };
        let fire_rocket = context.weapons.fire_rocket;
        let actor = context.actor().clone();
        fire_rocket(
            actor,
            &mut *context.game,
            start,
            direction,
            50.0,
            if predictive { 750.0 } else { 500.0 },
            70.0,
            50.0,
        );
        monster_flash(context, flash as i32, start, direction);
    }
}

/// Boss bullet (`bossBullet`).
fn boss_bullet(context: &mut MonsterContext, right: bool) {
    let flash = if right { 133usize } else { 73 };
    let Some((start, direction)) = monster_shot(context, flash, if right { 0.2 } else { -0.2 })
    else {
        return;
    };
    let fire_bullet = context.weapons.fire_bullet;
    let actor = context.actor().clone();
    fire_bullet(
        actor,
        &mut *context.game,
        start,
        direction,
        6.0,
        4.0,
        900.0,
        500.0,
        0,
    );
    monster_flash(context, flash as i32, start, direction);
}

/// Predictive rockets (`Boss2PredictiveRocket`).
fn boss2_predictive_rocket(context: &mut MonsterContext) {
    boss_rockets(context, true);
}

/// Rockets (`Boss2Rocket`).
fn boss2_rocket(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let predictive = enemy.as_ref().is_some_and(|enemy| context.game.host.is_player(enemy))
        && context.game.random() < 0.9;
    boss_rockets(context, predictive);
}

/// Fire bullet right (`boss2_firebullet_right`).
fn boss2_firebullet_right(context: &mut MonsterContext) {
    boss_bullet(context, true);
}

/// Fire bullet left (`boss2_firebullet_left`).
fn boss2_firebullet_left(context: &mut MonsterContext) {
    boss_bullet(context, false);
}

/// Machine gun (`Boss2MachineGun`).
fn boss2_machine_gun(context: &mut MonsterContext) {
    boss_bullet(context, false);
    boss_bullet(context, true);
}

/// Boss2 check attack (`checkAttack`).
fn rogue_boss2_check_attack(context: &mut MonsterContext) -> bool {
    boss_check_attack(context, true, true)
}

/// Supertank rocket (`supertankRocket`).
fn rogue_supertank_rocket(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = if frame == supertank_frame::ATTAK2_8 {
        70usize
    } else if frame == supertank_frame::ATTAK2_11 {
        71
    } else {
        72
    };
    let Some((start, direction)) = monster_shot(context, flash, 0.0) else {
        return;
    };
    let fire_rocket = context.weapons.fire_rocket;
    let actor = context.actor().clone();
    fire_rocket(
        actor,
        &mut *context.game,
        start,
        direction,
        50.0,
        500.0,
        70.0,
        50.0,
    );
    monster_flash(context, flash as i32, start, direction);
}

/// Supertank machine gun (`supertankMachineGun`).
fn rogue_supertank_machine_gun(context: &mut MonsterContext) {
    let eye = enemy_eye(context);
    let Some(eye) = eye else { return };
    let actor = context.actor().clone();
    let frame = context.entity().frame;
    let flash = 64 + frame - supertank_frame::ATTAK1_1;
    let yaw = context.game.body_of(actor).angles.y;
    let edition = context.game.options.edition;
    let start = project_flash(
        context,
        muzzle_offset(edition, flash as usize),
        Some(Vec3 { x: 0.0, y: yaw, z: 0.0 }),
    );
    let direction = normalize3(sub3(eye, start));
    let fire_bullet = context.weapons.fire_bullet;
    let actor = context.actor().clone();
    fire_bullet(
        actor,
        &mut *context.game,
        start,
        direction,
        6.0,
        4.0,
        300.0,
        500.0,
        0,
    );
    monster_flash(context, flash, start, direction);
}

/// Jorg initialize (`initialize`).
fn rogue_jorg_initialize(context: &mut MonsterContext) {
    jorg_initialize(context);
    context.state_mut().ignore_shots = true;
}

/// Makron initialize (`initialize`).
fn rogue_makron_initialize(context: &mut MonsterContext) {
    context.state_mut().ignore_shots = true;
}

/// Create rogue base variants (`createRogueBaseVariants`).
pub fn create_rogue_base_variants() -> Vec<Q2MonsterDefinition> {
    let mut brain = brain_definition();
    brain.moves = brain_moves();
    brain.pain = Some(rogue_brain_pain);
    brain.dodge = Some(rogue_brain_dodge);
    brain.duck = Some(rogue_brain_duck);
    brain.callbacks.insert(
        "monster_duck_down".to_string(),
        MonsterHandler::Callback(rogue_duck_down),
    );
    brain.callbacks.insert(
        "monster_duck_hold".to_string(),
        MonsterHandler::Callback(rogue_duck_hold),
    );
    brain.callbacks.insert(
        "monster_duck_up".to_string(),
        MonsterHandler::Callback(rogue_duck_up),
    );

    let mut floater = floater_definition();
    floater.moves = float_moves()
        .into_iter()
        .filter(|animation| animation.name != "floater_move_activate")
        .collect();
    floater.blocked = Some(rogue_shot_blocked_fn);
    floater.attack = MonsterHandler::Callback(rogue_floater_attack);
    floater.callbacks.insert(
        "floater_fire_blaster".to_string(),
        MonsterHandler::Callback(rogue_floater_fire_blaster),
    );

    let mut boss2 = boss2_definition();
    boss2.yaw_speed = Some(50.0);
    boss2.check_attack = Some(rogue_boss2_check_attack);
    boss2.callbacks.insert(
        "Boss2PredictiveRocket".to_string(),
        MonsterHandler::Callback(boss2_predictive_rocket),
    );
    boss2.callbacks.insert(
        "Boss2Rocket".to_string(),
        MonsterHandler::Callback(boss2_rocket),
    );
    boss2.callbacks.insert(
        "boss2_firebullet_right".to_string(),
        MonsterHandler::Callback(boss2_firebullet_right),
    );
    boss2.callbacks.insert(
        "boss2_firebullet_left".to_string(),
        MonsterHandler::Callback(boss2_firebullet_left),
    );
    boss2.callbacks.insert(
        "Boss2MachineGun".to_string(),
        MonsterHandler::Callback(boss2_machine_gun),
    );

    let mut supertank = supertank_definition();
    supertank.blocked = Some(rogue_blocked);
    supertank.initialize = Some(MonsterHandler::Callback(rogue_supertank_initialize));
    supertank.callbacks.insert(
        "supertankRocket".to_string(),
        MonsterHandler::Callback(rogue_supertank_rocket),
    );
    supertank.callbacks.insert(
        "supertankMachineGun".to_string(),
        MonsterHandler::Callback(rogue_supertank_machine_gun),
    );

    let mut gladiator = gladiator_definition();
    gladiator.blocked = Some(rogue_blocked);

    let mut jorg = create_jorg_definition();
    jorg.initialize = Some(MonsterHandler::Callback(rogue_jorg_initialize));

    let mut makron = makron_definition();
    makron.initialize = Some(MonsterHandler::Callback(rogue_makron_initialize));

    let mut definitions = create_rogue_soldier_definitions();
    definitions.push(create_rogue_infantry_definition());
    definitions.push(brain);
    definitions.push(floater);
    definitions.push(gladiator);
    definitions.push(with_boss_explosion_callbacks(boss2));
    definitions.push(with_boss_explosion_callbacks(supertank));
    definitions.push(with_boss_explosion_callbacks(jorg));
    definitions.push(with_makron_spawn_callbacks(makron));
    definitions
}

/// Shot-blocked as a blocked callback (`shotBlocked`).
fn rogue_shot_blocked_fn(context: &mut MonsterContext, _distance: f64) -> bool {
    rogue_shot_blocked(context)
}

/// Supertank initialize (`initialize`).
fn rogue_supertank_initialize(context: &mut MonsterContext) {
    context.state_mut().ignore_shots = true;
}
