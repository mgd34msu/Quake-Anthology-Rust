//! Xatrix base variants (`src/content/q2/missionpacks/monsters/xatrix-variants.ts`).
//!
//! Quake II Xatrix base monster variants. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Vec3};

use super::dabeam::{monster_dabeam, monster_dabeam_callbacks};
use super::power_armor::{monster_power_armor, restore_monster_power_armor, PowerArmorKind};
use super::tables::xatrix_brain::{brain_frame, brain_moves};
use super::tables::xatrix_infantry::{infantry_frame, infantry_moves};
use crate::q2::base::monsters::boss_common::with_boss_explosion_callbacks;
use crate::q2::base::monsters::brain::brain_definition;
use crate::q2::base::monsters::common::{damaged_skin, move_handler, HUMANOID_BOUNDS};
use crate::q2::base::monsters::supertank::supertank_definition;
use crate::q2::foundation::host::Q2TraceRequest;
use crate::q2::foundation::host::{Q2MonsterBeam, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, health, project_flash, target_distance, vector_angles, visible,
};
use crate::q2::foundation::monsters::infantry::{
    infantry_attack, infantry_callbacks, infantry_die, infantry_pain, infantry_run, infantry_sight, infantry_stand,
    infantry_walk, machine_gun,
};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{record_at, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::rerelease::monsters::common::monster_flash;
use crate::q2::support::contracts::{PainReaction, TraceHit};

/// Xatrix brain right eye offsets (`xatrixBrainRightEye`).
pub const XATRIX_BRAIN_RIGHT_EYE: [Vec3; 11] = [
    Vec3 {
        x: 0.7467,
        y: 0.23837,
        z: 34.16769,
    },
    Vec3 {
        x: -1.07639,
        y: 0.23837,
        z: 33.386372,
    },
    Vec3 {
        x: -1.3355,
        y: 5.3343,
        z: 32.17717,
    },
    Vec3 {
        x: -0.17536,
        y: 8.84637,
        z: 30.635479,
    },
    Vec3 {
        x: -2.75759,
        y: 7.80461,
        z: 30.15086,
    },
    Vec3 {
        x: -5.57509,
        y: 5.15284,
        z: 30.05616,
    },
    Vec3 {
        x: -7.01755,
        y: 3.26247,
        z: 30.552521,
    },
    Vec3 {
        x: -7.91574,
        y: 0.6388,
        z: 33.176189,
    },
    Vec3 {
        x: -3.91539,
        y: 8.28573,
        z: 33.976349,
    },
    Vec3 {
        x: -0.91354,
        y: 10.93303,
        z: 34.141811,
    },
    Vec3 {
        x: -0.3699,
        y: 8.9239,
        z: 34.189079,
    },
];

/// Xatrix brain left eye offsets (`xatrixBrainLeftEye`).
pub const XATRIX_BRAIN_LEFT_EYE: [Vec3; 11] = [
    Vec3 {
        x: -3.36471,
        y: 0.32775,
        z: 33.938381,
    },
    Vec3 {
        x: -5.14045,
        y: 0.49348,
        z: 32.659851,
    },
    Vec3 {
        x: -5.34198,
        y: 5.64698,
        z: 31.277901,
    },
    Vec3 {
        x: -4.13448,
        y: 9.27744,
        z: 29.925621,
    },
    Vec3 {
        x: -6.59834,
        y: 6.81509,
        z: 29.32262,
    },
    Vec3 {
        x: -8.61084,
        y: 2.52965,
        z: 29.251591,
    },
    Vec3 {
        x: -9.23136,
        y: 0.09328,
        z: 29.747959,
    },
    Vec3 {
        x: -11.00411,
        y: 1.93693,
        z: 32.39526,
    },
    Vec3 {
        x: -7.87831,
        y: 7.64819,
        z: 33.148151,
    },
    Vec3 {
        x: -4.94737,
        y: 11.43005,
        z: 33.31361,
    },
    Vec3 {
        x: -4.33282,
        y: 9.44457,
        z: 33.52634,
    },
];

/// Whether a tongue shot is allowed (`tongueAllowed`).
fn tongue_allowed(start: Vec3, end: Vec3) -> bool {
    let direction = sub3(start, end);
    if length3(direction) > 512.0 {
        return false;
    }
    let mut pitch = vector_angles(direction).x;
    if pitch < -180.0 {
        pitch += 360.0;
    }
    pitch.abs() <= 30.0
}

/// Brain attack (`attack`).
fn xatrix_brain_attack(context: &mut MonsterContext) {
    if context.game.random() >= 0.8 {
        return;
    }
    let distance = target_distance(context);
    if distance >= 80.0 && distance < 500.0 {
        if context.game.random() < 0.5 {
            context.set_move("brain_move_attack3", true);
        } else {
            context.set_move("brain_move_attack4", true);
        }
    } else if distance >= 500.0 {
        context.set_move("brain_move_attack4", true);
    }
}

/// Brain pain (`pain`).
fn xatrix_brain_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
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
}

/// Tongue attack (`brain_tounge_attack`).
fn brain_tongue_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let enemy_id = context.entity().enemy.clone();
    let (Some(enemy), Some(enemy_id)) = (enemy, enemy_id) else {
        return;
    };
    let start = project_flash(context, vec3(24.0, 0.0, 16.0), None);
    let top = Vec3 {
        x: enemy.origin.x,
        y: enemy.origin.y,
        z: enemy.origin.z + enemy.bounds.max.z - 8.0,
    };
    let bottom = Vec3 {
        x: enemy.origin.x,
        y: enemy.origin.y,
        z: enemy.origin.z + enemy.bounds.min.z + 8.0,
    };
    if !tongue_allowed(start, enemy.origin) && !tongue_allowed(start, top) && !tongue_allowed(start, bottom) {
        return;
    }
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end: enemy.origin,
        bounds: None,
        ignore: Some(actor.clone()),
        mask: 0x6000003,
        exclude: Vec::new(),
    });
    if !matches!(&trace.hit, TraceHit::Actor { actor } if *actor == enemy_id) {
        return;
    }
    context.game.sound(&actor, "brain/brnatck3.wav", 1, 1.0, 1.0);
    context.game.host_emit(Q2PresentationEvent::MonsterBeam {
        effect: Q2MonsterBeam::Parasite,
        actor: actor.clone(),
        start,
        end: enemy.origin,
    });
    context.game.damage(
        enemy_id.clone(),
        actor.clone(),
        Some(actor.clone()),
        5.0,
        0.0,
        sub3(start, enemy.origin),
        enemy.origin,
        vec3(0.0, 0.0, 0.0),
        36,
        8,
        None,
    );
    let mut body = context.game.body_of(actor.clone());
    body.origin.z += 1.0;
    context.game.write_body(actor, &body, false);
    if let Some(target) = context.game.host.actors().resolve_owned(&enemy_id) {
        let mut shoved = enemy;
        shoved.velocity = scale3(angles_vectors(body.angles).forward, -1200.0);
        context.game.host.bodies().write(&target, &shoved);
    }
}

/// Laser beam (`brain_laserbeam`).
fn brain_laserbeam(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.random() > 0.8 {
        context.game.sound(&actor, "misc/lasfly.wav", 0, 1.0, 3.0);
    }
    let enemy = enemy_body(context);
    let Some(enemy) = enemy else { return };
    let origin = context.game.body_of(actor.clone()).origin;
    let angles = vector_angles(sub3(enemy.origin, origin));
    let basis = angles_vectors(angles);
    let enemy_id = context.entity().enemy.clone();
    for eye in [&XATRIX_BRAIN_RIGHT_EYE, &XATRIX_BRAIN_LEFT_EYE] {
        let offset = record_at(eye, (context.entity().frame - brain_frame::WALK101) as usize);
        let start = add3(
            origin,
            add3(
                scale3(basis.right, offset.x),
                add3(scale3(basis.forward, offset.y), scale3(basis.up, offset.z)),
            ),
        );
        monster_dabeam(&actor, &mut *context.game, enemy_id.clone(), start, angles, 1.0, false);
    }
}

/// Laser beam reattack (`brain_laserbeam_reattack`).
fn brain_laserbeam_reattack(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    if context.game.random() < 0.5 && visible(context, None) && health(&mut *context.game, enemy.as_ref()) > 0.0 {
        context.entity_mut().frame = brain_frame::WALK101;
    }
}

/// Xatrix brain definition (`xatrixBrainDefinition`).
pub fn xatrix_brain_definition() -> Q2MonsterDefinition {
    let mut definition = brain_definition();
    definition.moves = brain_moves();
    definition.has_ranged_attack = true;
    definition.source_callbacks = Some(monster_dabeam_callbacks());
    definition.attack = MonsterHandler::Callback(xatrix_brain_attack);
    definition.pain = Some(xatrix_brain_pain);
    definition.callbacks.insert(
        "brain_tounge_attack".to_string(),
        MonsterHandler::Callback(brain_tongue_attack),
    );
    definition
        .callbacks
        .insert("brain_laserbeam".to_string(), MonsterHandler::Callback(brain_laserbeam));
    definition.callbacks.insert(
        "brain_laserbeam_reattack".to_string(),
        MonsterHandler::Callback(brain_laserbeam_reattack),
    );
    definition
}

/// Xatrix infantry fire (`xatrixInfantryFire`).
fn xatrix_infantry_fire(context: &mut MonsterContext) {
    if context.entity().frame != infantry_frame::ATTAK103 {
        machine_gun(context);
        return;
    }
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 26), None);
    let direction = match enemy {
        None => angles_vectors(context.game.body_of(actor).angles).forward,
        Some(enemy) => {
            let view_height = context
                .entity()
                .enemy
                .as_ref()
                .and_then(|enemy| context.game.entities.get(enemy))
                .map(|entity| entity.view_height)
                .unwrap_or(22);
            normalize3(sub3(
                add3(
                    add3(enemy.origin, scale3(enemy.velocity, -0.2)),
                    vec3(0.0, 0.0, view_height as f32),
                ),
                start,
            ))
        }
    };
    let fire_bullet = context.weapons.fire_bullet;
    let actor = context.actor().clone();
    fire_bullet(actor, &mut *context.game, start, direction, 3.0, 4.0, 300.0, 500.0, 0);
    monster_flash(context, 26, start, direction);
}

/// Xatrix infantry idle (`idle`).
fn xatrix_infantry_idle(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "infantry/infidle1.wav", 2, 1.0, 2.0);
    context.set_move("infantry_move_fidget", true);
}

/// Xatrix infantry dodge (`dodge`).
fn xatrix_infantry_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    _eta: f64,
    _trace: Option<&crate::q2::support::contracts::TraceResult>,
    _direct: bool,
) {
    if context.game.random() > 0.25 {
        return;
    }
    if context.entity().enemy.is_none() {
        context.entity_mut().enemy = Some(attacker.clone());
    }
    context.set_move("infantry_move_duck", true);
}

/// Xatrix infantry set firetime (`infantry_set_firetime`).
fn xatrix_infantry_set_firetime(context: &mut MonsterContext) {
    let pause = context.game.host.now() + ((context.game.random() * 16.0).floor() + 5.0) * 0.1;
    context.state_mut().pause_time = pause;
}

/// Xatrix infantry cock gun (`infantry_cock_gun`).
fn xatrix_infantry_cock_gun(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "infantry/infatck3.wav", 1, 1.0, 1.0);
}

/// Xatrix infantry fire (`infantry_fire`).
fn xatrix_infantry_fire_callback(context: &mut MonsterContext) {
    xatrix_infantry_fire(context);
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Xatrix infantry definition (`xatrixInfantryDefinition`).
pub fn xatrix_infantry_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_infantry",
        "infantry",
        "models/monsters/infantry/tris.md2",
        100.0,
        -40.0,
        200.0,
        HUMANOID_BOUNDS,
        1.0,
        "infantry_move_stand",
        infantry_moves(),
        MonsterHandler::Callback(infantry_stand),
        MonsterHandler::Callback(infantry_walk),
        MonsterHandler::Callback(infantry_run),
        MonsterHandler::Callback(infantry_attack),
        infantry_die,
    );
    definition.sight = Some(MonsterHandler::Callback(infantry_sight));
    definition.pain = Some(infantry_pain);
    definition.idle = Some(MonsterHandler::Callback(xatrix_infantry_idle));
    definition.dodge = Some(xatrix_infantry_dodge);
    let mut callbacks = infantry_callbacks();
    callbacks.insert(
        "InfantryMachineGun".to_string(),
        MonsterHandler::Callback(xatrix_infantry_fire),
    );
    callbacks.insert(
        "infantry_set_firetime".to_string(),
        MonsterHandler::Callback(xatrix_infantry_set_firetime),
    );
    callbacks.insert(
        "infantry_cock_gun".to_string(),
        MonsterHandler::Callback(xatrix_infantry_cock_gun),
    );
    callbacks.insert(
        "infantry_fire".to_string(),
        MonsterHandler::Callback(xatrix_infantry_fire_callback),
    );
    definition.callbacks = callbacks;
    definition
}

/// Xatrix supertank initialize (`initialize`).
fn xatrix_supertank_initialize(context: &mut MonsterContext) {
    if context.entity().spawnflags & 8 != 0 {
        monster_power_armor(context, PowerArmorKind::Shield, 400.0);
    }
}

/// Xatrix supertank restore (`restore`).
fn xatrix_supertank_restore(context: &mut MonsterContext) {
    if context.entity().spawnflags & 8 != 0 {
        restore_monster_power_armor(context);
    }
}

/// Create xatrix base variants (`createXatrixBaseVariants`).
pub fn create_xatrix_base_variants() -> Vec<Q2MonsterDefinition> {
    let mut supertank = supertank_definition();
    supertank.initialize = Some(MonsterHandler::Callback(xatrix_supertank_initialize));
    supertank.restore = Some(MonsterHandler::Callback(xatrix_supertank_restore));
    supertank.stand = move_handler("supertank_move_stand");
    vec![
        xatrix_brain_definition(),
        xatrix_infantry_definition(),
        with_boss_explosion_callbacks(supertank),
    ]
}
