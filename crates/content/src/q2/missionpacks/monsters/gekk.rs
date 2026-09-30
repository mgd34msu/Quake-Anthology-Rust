//! Xatrix gekk (`src/content/q2/missionpacks/monsters/gekk.ts`).
//!
//! Quake II xatrix/m_gekk.c and acid gibs from g_misc.c.
//! ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, length3, normalize3, scale3, sub3, vec3};

use super::tables::xatrix_gekk::{gekk_frame, gekk_moves};
use crate::q2::base::monsters::common::{
    alive_enemy, finish_corpse_default, move_handler, sound_handler,
};
use crate::q2::foundation::callbacks::{Q2CallbackDefinitions, free_q2_entity};
use crate::q2::foundation::host::{
    Q2GameServices, Q2MotionKind, Q2Solid, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{
    MASK_SHOT, angles_vectors, check_bottom, enemy_body, enemy_eye, health,
    project_flash, run_ai, set_duck, target_distance, trace_ground_actor,
    vector_angles,
};
use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAttackState, MonsterContext, MonsterHandler,
    MonsterLocomotion, Q2MonsterDefinition,
};
use crate::q2::support::contracts::{
    CombatTraitChanges, DeathReaction, PainReaction, TouchContact, TraceResult,
};

/// Acid gib die (`gibDie`).
fn acid_gib_die(actor: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    game.remove_actor(actor);
}

/// Loogie touch (`loogieTouch`).
fn loogie_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let entity = game.require_entity(&actor).clone();
    if Some(&contact.other) == entity.owner.as_ref() {
        return;
    }
    if contact.surface.as_ref().map(|surface| surface.native_flags).unwrap_or(0) & 4 != 0 {
        game.remove_actor(actor);
        return;
    }
    let damageable = game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|state| state.can_take_damage);
    if damageable {
        let body = game.body_of(actor.clone());
        game.damage(
            contact.other,
            actor.clone(),
            entity.owner,
            entity.damage,
            1.0,
            body.velocity,
            body.origin,
            contact.plane.map(|plane| plane.normal).unwrap_or(vec3(0.0, 0.0, 0.0)),
            38,
            4,
            None,
        );
    }
    game.remove_actor(actor);
}

/// Fire a gekk loogie (`fireGekkLoogie`).
pub fn fire_gekk_loogie(
    owner: &ActorId,
    game: &mut Q2GameServices,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    speed: f64,
) -> ActorId {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("G_FreeEdict", free_q2_entity);
    callbacks.touch.insert("loogie_touch", loogie_touch);
    game.source_callbacks.register(&callbacks);
    let projectile = game.create("loogie", BTreeMap::new());
    let aim = normalize3(direction);
    {
        let entity = game.require_entity_mut(&projectile);
        entity.owner = Some(owner.clone());
        entity.model = "models/objects/loogy/tris.md2".to_string();
        entity.effects |= 8;
        entity.damage = damage;
        entity.clip_mask = MASK_SHOT;
        entity.projectile = true;
    }
    let mut moved = game.body_of(projectile.clone());
    moved.origin = start;
    moved.angles = vector_angles(aim);
    moved.velocity = scale3(aim, speed as f32);
    moved.bounds = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    game.write_body(projectile.clone(), &moved, false);
    game.require_entity_mut(&projectile).touch = Some(loogie_touch);
    game.set_solid(projectile.clone(), Q2Solid::Box);
    game.set_motion_kind(projectile.clone(), Q2MotionKind::FlyMissile);
    game.schedule(projectile.clone(), 2.0, free_q2_entity);
    game.show(projectile.clone());
    let origin = game.body_of(owner.clone()).origin;
    let trace = game.host.trace(&Q2TraceRequest {
        start: origin,
        end: start,
        bounds: None,
        ignore: Some(projectile.clone()),
        mask: MASK_SHOT,
        exclude: Vec::new(),
    });
    if trace.fraction < 1.0 {
        let mut moved = game.body_of(projectile.clone());
        moved.origin = add3(start, scale3(aim, -10.0));
        game.write_body(projectile.clone(), &moved, true);
        if let Some(other) = trace_ground_actor(&trace, game) {
            let owned = game.owned_of(projectile.clone());
            loogie_touch(
                projectile.clone(),
                game,
                TouchContact {
                    this: owned,
                    other,
                    plane: None,
                    surface: None,
                    source_trace: None,
                },
            );
        }
    }
    projectile
}

/// Acid gib (`acidGib`).
fn acid_gib(context: &mut MonsterContext, part: &str, damage: f64, head: bool) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let gib = if head {
        actor.clone()
    } else {
        context.game.create("acid_gib", BTreeMap::new())
    };
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("G_FreeEdict", free_q2_entity);
    callbacks.die.insert("acid_gib_die", acid_gib_die);
    context.game.source_callbacks.register(&callbacks);
    let half = scale3(sub3(body.bounds.max, body.bounds.min), 0.5);
    let center = add3(
        add3(body.origin, body.bounds.min),
        add3(half, vec3(-1.0, -1.0, -1.0)),
    );
    let origin = if head {
        body.origin
    } else {
        add3(
            center,
            vec3(
                (context.game.random() * 2.0 - 1.0) as f32 * half.x,
                (context.game.random() * 2.0 - 1.0) as f32 * half.y,
                (context.game.random() * 2.0 - 1.0) as f32 * half.z,
            ),
        )
    };
    {
        let entity = context.game.require_entity_mut(&gib);
        entity.model = format!("models/objects/gekkgib/{part}/tris.md2");
        entity.clip_mask = MASK_SHOT;
        entity.flags |= 2048;
        entity.damage = 2.0;
        if head {
            entity.skin = 0;
            entity.frame = 0;
            entity.model2 = String::new();
            entity.effects = (entity.effects | 0x200000 | 8) & !0x4000;
            entity.server_flags &= !4;
        } else {
            entity.effects |= 0x200000;
            entity.render_flags |= 8;
        }
    }
    if head {
        context.game.host_emit(
            crate::q2::foundation::host::Q2PresentationEvent::Sound(
                crate::q2::foundation::host::Q2SoundEvent {
                    actor: Some(gib.clone()),
                    origin,
                    path: String::new(),
                    channel: 0,
                    volume: 0.0,
                    attenuation: 1.0,
                    reliable: false,
                    loop_: crate::q2::foundation::host::Q2SoundLoop::Stop,
                    loop_owner: None,
                },
            ),
        );
    }
    let scale = if damage < 50.0 { 0.7 } else { 1.2 };
    let impulse = scale3(
        vec3(
            (100.0 * (context.game.random() * 2.0 - 1.0)) as f32,
            (100.0 * (context.game.random() * 2.0 - 1.0)) as f32,
            (200.0 + 100.0 * context.game.random()) as f32,
        ),
        scale as f32,
    );
    let velocity = add3(body.velocity, scale3(impulse, if head { 0.5 } else { 3.0 }));
    let spin = if head {
        let yaw = (context.game.random() * 2.0 - 1.0) * 600.0;
        let entity = context.game.require_entity(&gib);
        vec3(entity.angular_velocity.x, yaw as f32, entity.angular_velocity.z)
    } else {
        vec3(
            (context.game.random() * 600.0) as f32,
            (context.game.random() * 600.0) as f32,
            (context.game.random() * 600.0) as f32,
        )
    };
    context.game.require_entity_mut(&gib).angular_velocity = spin;
    let mut moved = context.game.body_of(gib.clone());
    moved.origin = origin;
    moved.velocity = vec3(
        velocity.x.clamp(-300.0, 300.0),
        velocity.y.clamp(-300.0, 300.0),
        velocity.z.clamp(200.0, 500.0),
    );
    moved.bounds = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    context.game.write_body(gib.clone(), &moved, false);
    if context.game.host.combat().read(&gib).is_none() {
        let owned = context.game.owned_of(gib.clone());
        context.game.create_combat(&owned, 0.0, 0.0, true);
    } else {
        let owned = context.game.owned_of(gib.clone());
        context.game.set_combat_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(true),
                ..CombatTraitChanges::default()
            },
        );
    }
    context.game.require_entity_mut(&gib).die = Some(acid_gib_die);
    context.game.set_motion_kind(gib.clone(), Q2MotionKind::Toss);
    context.game.set_solid(gib.clone(), Q2Solid::Box);
    let lifetime = 10.0 + context.game.random() * 10.0;
    context.game.schedule(gib.clone(), lifetime, free_q2_entity);
    context.game.show(gib);
}

/// Gibfest (`gibfest`).
fn gibfest_inner(context: &mut MonsterContext, damage: f64) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
    for part in ["pelvis", "arm", "arm", "torso", "claw", "leg", "leg"] {
        acid_gib(context, part, damage, false);
    }
    acid_gib(context, "head", damage, true);
    context.state_mut().dead = true;
    context.state_mut().gibbed = true;
}

/// Wet (`wet`).
fn gekk_wet(context: &mut MonsterContext, other: Option<&ActorId>) -> bool {
    match other {
        None => context.state().water_level > 0,
        Some(other) if other == context.actor() => context.state().water_level > 0,
        Some(other) => {
            let body = context.game.body_of(other.clone());
            context.game.host.point_contents(vec3(
                body.origin.x,
                body.origin.y,
                body.origin.z + body.bounds.min.z + 1.0,
            )) & 56
                != 0
        }
    }
}

/// Water to land (`waterToLand`).
fn gekk_water_to_land(context: &mut MonsterContext) {
    context.entity_mut().flags &= !2;
    context.state_mut().locomotion = MonsterLocomotion::Walk;
    context.state_mut().yaw_speed = 20.0;
    context.entity_mut().view_height = 25;
    let actor = context.actor().clone();
    let mut body = context.game.body_of(actor.clone());
    body.bounds = Bounds {
        min: vec3(-24.0, -24.0, -24.0),
        max: vec3(24.0, 24.0, 24.0),
    };
    context.game.write_body(actor, &body, false);
    context.set_move("gekk_move_leapatk2", false);
}

/// Land to water (`landToWater`).
fn gekk_land_to_water(context: &mut MonsterContext) {
    context.entity_mut().flags |= 2;
    context.state_mut().locomotion = MonsterLocomotion::Swim;
    context.state_mut().yaw_speed = 10.0;
    context.entity_mut().view_height = 10;
    let actor = context.actor().clone();
    let mut body = context.game.body_of(actor.clone());
    body.bounds = Bounds {
        min: vec3(-24.0, -24.0, -24.0),
        max: vec3(24.0, 24.0, 16.0),
    };
    context.game.write_body(actor, &body, false);
    context.set_move("gekk_move_swim_start", false);
}

/// Check jump (`checkJump`).
fn gekk_check_jump(context: &mut MonsterContext) -> bool {
    let Some(enemy) = enemy_body(context) else {
        return false;
    };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    let minimum = enemy.origin.z + enemy.bounds.min.z;
    let size = enemy.bounds.max.z - enemy.bounds.min.z;
    if body.origin.z + body.bounds.min.z > minimum + 0.75 * size
        || body.origin.z + body.bounds.max.z < minimum + 0.25 * size
    {
        return false;
    }
    let distance = f32::hypot(body.origin.x - enemy.origin.x, body.origin.y - enemy.origin.y);
    !(distance < 100.0 || distance > 100.0 && context.game.random() < 0.9)
}

/// Run (`run`).
fn gekk_run(context: &mut MonsterContext) {
    let wet = gekk_wet(context, None);
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if wet {
            "gekk_move_swim_start"
        } else if stand_ground {
            "gekk_move_stand"
        } else {
            "gekk_move_run"
        },
        false,
    );
}

/// Run start (`runStart`).
fn gekk_run_start(context: &mut MonsterContext) {
    let wet = gekk_wet(context, None);
    context.set_move(
        if wet {
            "gekk_move_swim_start"
        } else {
            "gekk_move_run_start"
        },
        false,
    );
}

/// Melee (`melee`).
fn gekk_melee(context: &mut MonsterContext) {
    let wet = gekk_wet(context, None);
    let first = context.game.random() > 0.66;
    context.set_move(
        if wet {
            "gekk_move_attack"
        } else if first {
            "gekk_move_attack1"
        } else {
            "gekk_move_attack2"
        },
        false,
    );
}

/// Hit (`hit`).
fn gekk_hit(context: &mut MonsterContext, right: bool) {
    let actor = context.actor().clone();
    let bounds = context.game.body_of(actor.clone()).bounds;
    let fire_hit = context.weapons.fire_hit;
    let damage = 15.0 + (context.game.random() * 5.0).floor();
    let connected = fire_hit(
        actor.clone(),
        &mut *context.game,
        vec3(80.0, if right { bounds.max.x } else { bounds.min.x }, 8.0),
        damage,
        100.0,
    );
    context.game.sound(
        &actor,
        if connected {
            if right {
                "gek/gk_atck3.wav"
            } else {
                "gek/gk_atck2.wav"
            }
        } else {
            "gek/gk_atck1.wav"
        },
        1,
        1.0,
        1.0,
    );
}

/// Search (`search`).
fn gekk_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let mut path = "gek/gk_idle1.wav";
    if context.entity().spawnflags & 8 != 0 {
        let random = context.game.random();
        path = if random < 0.33 {
            "gek/gek_low.wav"
        } else if random < 0.66 {
            "gek/gek_mid.wav"
        } else {
            "gek/gek_high.wav"
        };
    }
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    let max_health = context.entity().max_health;
    let hp = max_health.min(
        (health(&mut *context.game, Some(&actor)) + 10.0 + 10.0 * context.game.random()).trunc(),
    );
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_health(&owned, hp);
    let skin = if hp < max_health / 4.0 {
        2
    } else if hp < max_health / 2.0 {
        1
    } else {
        0
    };
    context.entity_mut().skin = skin;
}

/// Jump touch (`jumpTouch`).
fn gekk_jump_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.monsters.states.contains_key(&actor) {
        panic!("Gekk jump without its source controller");
    }
    if health(game, Some(&actor)) <= 0.0 {
        game.require_entity_mut(&actor).touch = None;
        return;
    }
    let body = game.body_of(actor.clone());
    let damageable = game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|state| state.can_take_damage);
    if damageable && length3(body.velocity) > 200.0 {
        let normal = normalize3(body.velocity);
        let damage = (10.0 + 10.0 * game.random()).trunc();
        let owner = game.require_entity(&actor).owner.clone();
        game.damage(
            contact.other.clone(),
            actor.clone(),
            owner,
            damage,
            damage,
            body.velocity,
            add3(body.origin, scale3(normal, body.bounds.max.x)),
            normal,
            38,
            0,
            None,
        );
    }
    let origin = body.origin;
    let mut context = MonsterContext::new(actor.clone(), game);
    let bottom = check_bottom(&mut context, origin);
    let game = &mut *context.game;
    if !bottom {
        if body.ground.is_some() {
            if let Some(state) = game.monsters.states.get_mut(&actor) {
                state.next_frame = gekk_frame::LEAPATK_11;
            }
            game.require_entity_mut(&actor).touch = None;
        }
        return;
    }
    game.require_entity_mut(&actor).touch = None;
}

/// Takeoff (`takeoff`).
fn gekk_takeoff(context: &mut MonsterContext, from_water: bool) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "gek/gk_sght1.wav", 2, 1.0, 1.0);
    let body = context.game.body_of(actor.clone());
    let enemy = enemy_body(context);
    let origin_z = body.origin.z;
    let mut moved = body;
    moved.origin.z = if from_water {
        enemy.map(|enemy| enemy.origin.z).unwrap_or(origin_z)
    } else {
        origin_z + 1.0
    };
    context.game.write_body(actor.clone(), &moved, false);
    let long = gekk_check_jump(context);
    let speed = if from_water {
        if long { 300.0 } else { 150.0 }
    } else if long {
        700.0
    } else {
        250.0
    };
    let up = if from_water {
        if long { 250.0 } else { 300.0 }
    } else if long {
        250.0
    } else {
        400.0
    };
    let mut moved = context.game.body_of(actor.clone());
    let flat = scale3(angles_vectors(moved.angles).forward, speed);
    moved.velocity = vec3(flat.x, flat.y, up);
    moved.ground = None;
    context.game.write_body(actor.clone(), &moved, false);
    context.state_mut().ducked = true;
    let finished = context.game.host.now() + 3.0;
    context.state_mut().attack_finished = finished;
    context.entity_mut().touch = Some(gekk_jump_touch);
}

/// Stand (`stand`).
fn gekk_stand(context: &mut MonsterContext) {
    let wet = gekk_wet(context, None);
    context.set_move(
        if wet {
            "gekk_move_standunderwater"
        } else {
            "gekk_move_stand"
        },
        false,
    );
}

/// Idle (`idle`).
fn gekk_idle(context: &mut MonsterContext) {
    let wet = gekk_wet(context, None);
    context.set_move(
        if wet {
            "gekk_move_swim_start"
        } else {
            "gekk_move_idle"
        },
        false,
    );
}

/// After spawn (`afterSpawn`).
fn gekk_after_spawn(context: &mut MonsterContext) {
    if context.entity().spawnflags & 8 != 0 {
        context.set_move("gekk_move_chant", false);
    }
}

/// Stand AI (`ai_stand2`).
fn gekk_ai_stand2(context: &mut MonsterContext, distance: f64) {
    if context.entity().spawnflags & 8 == 0 {
        run_ai(context, &MonsterAi::Stand, distance);
        return;
    }
    run_ai(context, &MonsterAi::Move, distance);
    if context.entity().spawnflags & 1 == 0
        && context.game.host.now() > context.state().idle_time
    {
        if context.state().idle_time != 0.0 {
            context.idle();
            let idle = context.game.host.now() + 15.0 + context.game.random() * 15.0;
            context.state_mut().idle_time = idle;
        } else {
            let idle = context.game.host.now() + context.game.random() * 15.0;
            context.state_mut().idle_time = idle;
        }
    }
}

/// Check attack (`checkAttack`).
fn gekk_check_attack(context: &mut MonsterContext) -> bool {
    if !alive_enemy(context) {
        return false;
    }
    if target_distance(context) < 80.0 {
        context.state_mut().attack_state = MonsterAttackState::Melee;
        return true;
    }
    let Some(enemy) = enemy_body(context) else {
        return false;
    };
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor).origin;
    let close = f32::hypot(origin.x - enemy.origin.x, origin.y - enemy.origin.y) >= 100.0
        || origin.z < enemy.origin.z;
    if gekk_check_jump(context) || close && !gekk_wet(context, None) {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    false
}

/// Attack (`attack`).
fn gekk_attack(context: &mut MonsterContext) {
    if context.entity().flags & 2 != 0 || gekk_wet(context, None) {
        return;
    }
    let spit = context.game.random() > 0.5 && target_distance(context) >= 80.0
        || context.game.random() > 0.8;
    context.set_move(
        if spit {
            "gekk_move_spit"
        } else {
            "gekk_move_leapatk"
        },
        false,
    );
}

/// Pain (`pain`).
fn gekk_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    if context.entity().spawnflags & 8 != 0 {
        context.entity_mut().spawnflags &= !8;
        return;
    }
    let actor = context.actor().clone();
    let hp = health(&mut *context.game, Some(&actor));
    let max_health = context.entity().max_health;
    if hp < max_health / 2.0 {
        context.entity_mut().skin = if hp < max_health / 4.0 { 2 } else { 1 };
    }
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    context.game.sound(&actor, "gek/gk_pain1.wav", 2, 1.0, 1.0);
    let wet = gekk_wet(context, None);
    let first = context.game.random() > 0.5;
    context.set_move(
        if wet {
            "gekk_move_pain"
        } else if first {
            "gekk_move_pain1"
        } else {
            "gekk_move_pain2"
        },
        false,
    );
}

/// Die (`die`).
fn gekk_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if health(&mut *context.game, Some(&actor)) <= context.state().gib_health {
        let damage = reaction.pain.damage;
        gibfest_inner(context, damage);
        return;
    }
    if context.state().dead {
        return;
    }
    let actor = context.actor().clone();
    context.game.sound(&actor, "gek/gk_deth1.wav", 2, 1.0, 1.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    context.entity_mut().skin = 2;
    let actor = context.actor().clone();
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    if gekk_wet(context, None) {
        context.set_move("gekk_move_wdeath", false);
        return;
    }
    let random = context.game.random();
    context.set_move(
        if random > 0.66 {
            "gekk_move_death1"
        } else if random > 0.33 {
            "gekk_move_death3"
        } else {
            "gekk_move_death4"
        },
        false,
    );
}

/// Dodge (`dodge`).
fn gekk_dodge(
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
    if gekk_wet(context, None) {
        context.set_move("gekk_move_attack", false);
        return;
    }
    if context.game.options.skill == 0 {
        let left = context.game.random() > 0.5;
        context.set_move(
            if left {
                "gekk_move_lduck"
            } else {
                "gekk_move_rduck"
            },
            false,
        );
        return;
    }
    let pause = context.game.host.now() + eta + 0.3;
    context.state_mut().pause_time = pause;
    let skill = context.game.options.skill;
    let random = context.game.random();
    if skill < 3 && random > if skill == 1 { 0.33 } else { 0.66 } {
        let left = context.game.random() > 0.5;
        context.set_move(
            if left {
                "gekk_move_lduck"
            } else {
                "gekk_move_rduck"
            },
            false,
        );
        return;
    }
    let first = context.game.random() > 0.66;
    context.set_move(
        if first {
            "gekk_move_attack1"
        } else {
            "gekk_move_attack2"
        },
        false,
    );
}

/// Stand callback (`gekk_stand`).
fn gekk_stand_callback(context: &mut MonsterContext) {
    context.stand();
}

/// Swim loop (`gekk_swim_loop`).
fn gekk_swim_loop(context: &mut MonsterContext) {
    context.entity_mut().flags |= 2;
    context.state_mut().locomotion = MonsterLocomotion::Swim;
    context.set_move("gekk_move_swim_loop", false);
}

/// Swim (`gekk_swim`).
fn gekk_swim(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let enemy_entity = enemy
        .as_ref()
        .and_then(|enemy| context.game.entity(enemy))
        .cloned();
    let leave = match enemy_entity {
        Some(enemy_entity) => {
            let id = enemy_entity.actor.id().clone();
            !gekk_wet(context, Some(&id)) && context.game.random() > 0.7
        }
        None => false,
    };
    if leave {
        gekk_water_to_land(context);
    } else {
        context.set_move("gekk_move_swim_start", false);
    }
}

/// Check underwater (`gekk_check_underwater`).
fn gekk_check_underwater(context: &mut MonsterContext) {
    if gekk_wet(context, None) {
        gekk_land_to_water(context);
    }
}

/// Idle loop (`gekk_idle_loop`).
fn gekk_idle_loop(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.random() > 0.75
        && health(&mut *context.game, Some(&actor)) < context.entity().max_health
    {
        context.state_mut().next_frame = gekk_frame::IDLE_01;
    }
}

/// Step (`gekk_step`).
fn gekk_step(context: &mut MonsterContext) {
    let n = ((context.game.random() * 3.0).floor() as i32 + 1) % 3;
    let actor = context.actor().clone();
    let path = format!("gek/gk_step{}.wav", n + 1);
    context.game.sound(&actor, &path, 2, 1.0, 1.0);
}

/// Hit left (`gekk_hit_left`).
fn gekk_hit_left(context: &mut MonsterContext) {
    gekk_hit(context, false);
}

/// Hit right (`gekk_hit_right`).
fn gekk_hit_right(context: &mut MonsterContext) {
    gekk_hit(context, true);
}

/// Bite (`gekk_bite`).
fn gekk_bite(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    fire_hit(actor, &mut *context.game, vec3(80.0, 0.0, 0.0), 5.0, 0.0);
}

/// Check refire (`gekk_check_refire`).
fn gekk_check_refire(context: &mut MonsterContext) {
    if alive_enemy(context)
        && context.game.random() < f64::from(context.game.options.skill) * 0.1
        && target_distance(context) < 80.0
    {
        if context.entity().frame == gekk_frame::CLAWATK3_09 {
            context.set_move("gekk_move_attack2", false);
        } else if context.entity().frame == gekk_frame::CLAWATK5_09 {
            context.set_move("gekk_move_attack1", false);
        }
    }
}

/// Loogie (`loogie`).
fn gekk_loogie(context: &mut MonsterContext) {
    let Some(eye) = enemy_eye(context) else {
        return;
    };
    if !alive_enemy(context) {
        return;
    }
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let start = add3(
        project_flash(context, vec3(-18.0, -0.8, 24.0), None),
        scale3(angles_vectors(body.angles).up, 2.0),
    );
    fire_gekk_loogie(&actor, &mut *context.game, start, sub3(eye, start), 5.0, 550.0);
}

/// Reloogie (`reloogie`).
fn gekk_reloogie(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.random() > 0.8
        && health(&mut *context.game, Some(&actor)) < context.entity().max_health
    {
        context.set_move("gekk_move_idle2", false);
        return;
    }
    let distance = target_distance(context);
    let enemy = context.entity().enemy.clone();
    if health(&mut *context.game, enemy.as_ref()) >= 0.0
        && context.game.random() > 0.7
        && distance >= 80.0
        && distance < 500.0
    {
        context.set_move("gekk_move_spit", false);
    }
}

/// Jump takeoff (`gekk_jump_takeoff`).
fn gekk_jump_takeoff(context: &mut MonsterContext) {
    gekk_takeoff(context, false);
}

/// Water jump takeoff (`gekk_jump_takeoff2`).
fn gekk_jump_takeoff2(context: &mut MonsterContext) {
    gekk_takeoff(context, true);
}

/// Stop skid (`gekk_stop_skid`).
fn gekk_stop_skid(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.body_of(actor.clone()).ground.is_some() {
        let mut moved = context.game.body_of(actor.clone());
        moved.velocity = vec3(0.0, 0.0, 0.0);
        context.game.write_body(actor, &moved, false);
    }
}

/// Check landing (`gekk_check_landing`).
fn gekk_check_landing(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.body_of(actor.clone()).ground.is_some() {
        context.game.sound(&actor, "mutant/thud1.wav", 1, 1.0, 1.0);
        context.state_mut().attack_finished = 0.0;
        context.state_mut().ducked = false;
        let mut moved = context.game.body_of(actor.clone());
        moved.velocity = vec3(0.0, 0.0, 0.0);
        context.game.write_body(actor, &moved, false);
        return;
    }
    context.state_mut().next_frame = if context.game.host.now() > context.state().attack_finished {
        gekk_frame::LEAPATK_11
    } else {
        gekk_frame::LEAPATK_12
    };
}

/// Preattack (`gekk_preattack`).
fn gekk_preattack(_context: &mut MonsterContext) {}

/// Random gibfest (`isgibfest`).
fn gekk_isgibfest(context: &mut MonsterContext) {
    if context.game.random() > 0.9 {
        gibfest_inner(context, 20.0);
    }
}

/// Gibfest (`gekk_gibfest`).
fn gekk_gibfest(context: &mut MonsterContext) {
    gibfest_inner(context, 20.0);
}

/// Dead (`gekk_dead`).
fn gekk_dead(context: &mut MonsterContext) {
    if !gekk_wet(context, None) {
        finish_corpse_default(context);
    }
}

/// Duck down (`gekk_duck_down`).
fn gekk_duck_down(context: &mut MonsterContext) {
    if context.state().ducked {
        return;
    }
    set_duck(context, true);
    let pause = context.game.host.now() + 1.0;
    context.state_mut().pause_time = pause;
}

/// Duck up (`gekk_duck_up`).
fn gekk_duck_up(context: &mut MonsterContext) {
    set_duck(context, false);
}

/// Duck hold (`gekk_duck_hold`).
fn gekk_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Create the gekk definition (`createGekkDefinition`).
pub fn create_gekk_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_gekk",
        "gekk",
        "models/monsters/gekk/tris.md2",
        125.0,
        -30.0,
        300.0,
        Bounds {
            min: vec3(-24.0, -24.0, -24.0),
            max: vec3(24.0, 24.0, 24.0),
        },
        1.0,
        "gekk_move_stand",
        gekk_moves(),
        MonsterHandler::Callback(gekk_stand),
        move_handler("gekk_move_walk"),
        MonsterHandler::Callback(gekk_run_start),
        MonsterHandler::Callback(gekk_attack),
        gekk_die,
    );
    definition.melee = Some(MonsterHandler::Callback(gekk_melee));
    definition.sight = Some(sound_handler("gek/gk_sght1.wav", 2, 1.0));
    definition.search = Some(MonsterHandler::Callback(gekk_search));
    definition.idle = Some(MonsterHandler::Callback(gekk_idle));
    definition.after_spawn = Some(MonsterHandler::Callback(gekk_after_spawn));
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("G_FreeEdict", free_q2_entity);
    source_callbacks.touch.insert("gekk_jump_touch", gekk_jump_touch);
    source_callbacks.touch.insert("loogie_touch", loogie_touch);
    source_callbacks.die.insert("acid_gib_die", acid_gib_die);
    definition.source_callbacks = Some(source_callbacks);
    definition.ai.insert("ai_stand2".to_string(), gekk_ai_stand2);
    definition.check_attack = Some(gekk_check_attack);
    definition.pain = Some(gekk_pain);
    definition.dodge = Some(gekk_dodge);
    for (name, handler) in [
        ("gekk_face", move_handler("gekk_move_run")),
        ("gekk_chant", move_handler("gekk_move_chant")),
        ("gekk_run", MonsterHandler::Callback(gekk_run)),
        ("gekk_run_start", MonsterHandler::Callback(gekk_run_start)),
        ("gekk_stand", MonsterHandler::Callback(gekk_stand_callback)),
        ("gekk_search", MonsterHandler::Callback(gekk_search)),
        ("gekk_swim_loop", MonsterHandler::Callback(gekk_swim_loop)),
        ("gekk_swim", MonsterHandler::Callback(gekk_swim)),
        ("gekk_check_underwater", MonsterHandler::Callback(gekk_check_underwater)),
        ("gekk_idle_loop", MonsterHandler::Callback(gekk_idle_loop)),
        ("gekk_step", MonsterHandler::Callback(gekk_step)),
        ("gekk_hit_left", MonsterHandler::Callback(gekk_hit_left)),
        ("gekk_hit_right", MonsterHandler::Callback(gekk_hit_right)),
        ("gekk_bite", MonsterHandler::Callback(gekk_bite)),
        ("gekk_check_refire", MonsterHandler::Callback(gekk_check_refire)),
        ("loogie", MonsterHandler::Callback(gekk_loogie)),
        ("reloogie", MonsterHandler::Callback(gekk_reloogie)),
        ("gekk_jump_takeoff", MonsterHandler::Callback(gekk_jump_takeoff)),
        ("gekk_jump_takeoff2", MonsterHandler::Callback(gekk_jump_takeoff2)),
        ("gekk_stop_skid", MonsterHandler::Callback(gekk_stop_skid)),
        ("gekk_check_landing", MonsterHandler::Callback(gekk_check_landing)),
        ("gekk_preattack", MonsterHandler::Callback(gekk_preattack)),
        ("isgibfest", MonsterHandler::Callback(gekk_isgibfest)),
        ("gekk_gibfest", MonsterHandler::Callback(gekk_gibfest)),
        ("gekk_dead", MonsterHandler::Callback(gekk_dead)),
        ("gekk_duck_down", MonsterHandler::Callback(gekk_duck_down)),
        ("gekk_duck_up", MonsterHandler::Callback(gekk_duck_up)),
        ("gekk_duck_hold", MonsterHandler::Callback(gekk_duck_hold)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
