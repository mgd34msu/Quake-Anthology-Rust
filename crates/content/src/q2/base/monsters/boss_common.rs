//! Shared original boss routines (`src/content/q2/base/monsters/boss-common.ts`).
//!
//! Shared original boss routines. id Software Quake II, GPL-2.0-or-later.

use qa_core::math::{add3, sub3, vec3};

use crate::q2::foundation::host::{
    Q2EffectEvent, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{enemy_body, enemy_eye, health, target_distance, vector_angles};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterLocomotion, Q2MonsterDefinition,
};
use crate::q2::support::contracts::TraceHit;

/// Stop a monster loop sound (`stopLoop`).
pub fn stop_loop(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(actor),
        origin,
        path: String::new(),
        channel: 0,
        volume: 0.0,
        attenuation: 1.0,
        reliable: false,
        loop_: Q2SoundLoop::Stop,
        loop_owner: None,
    }));
}

/// Boss attack check (`bossCheckAttack`).
pub fn boss_check_attack(context: &mut MonsterContext, hover: bool, allow_non_solid: bool) -> bool {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let enemy_body = enemy_body(context);
    let eye = enemy_eye(context);
    let enemy = context.entity().enemy.clone();
    let (Some(enemy_body), Some(eye), Some(enemy)) = (enemy_body, eye, enemy.as_ref()) else {
        return false;
    };
    if health(&mut *context.game, Some(enemy)) > 0.0 {
        let view_height = context.entity().view_height;
        let trace = context.game.host.trace(&Q2TraceRequest {
            start: vec3(body.origin.x, body.origin.y, body.origin.z + view_height as f32),
            end: eye,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 1 | 0x2000000 | 16 | 8,
            exclude: Vec::new(),
        });
        let hit_enemy = matches!(&trace.hit, TraceHit::Actor { actor: hit } if hit == enemy);
        let enemy_solid = context
            .game
            .entities
            .get(enemy)
            .is_some_and(|entity| entity.solid != Q2Solid::None);
        if !hit_enemy && (!allow_non_solid || enemy_solid || trace.fraction < 1.0) {
            return false;
        }
    }
    let distance = target_distance(context);
    let yaw = vector_angles(sub3(enemy_body.origin, body.origin)).y;
    context.state_mut().ideal_yaw = f64::from(yaw);
    if distance < 80.0 {
        let melee = context.state().has_melee;
        context.state_mut().attack_state = if melee {
            MonsterAttackState::Melee
        } else {
            MonsterAttackState::Missile
        };
        return true;
    }
    let now = context.game.host.now();
    if !context.state().has_ranged_attack || now < context.state().attack_finished || distance >= 1000.0 {
        return false;
    }
    let chance = if context.state().stand_ground {
        0.4
    } else if hover {
        0.8
    } else if distance < 500.0 {
        0.4
    } else {
        0.2
    };
    let enemy_non_solid = context
        .game
        .entities
        .get(enemy)
        .is_some_and(|entity| entity.solid == Q2Solid::None);
    if context.game.random() < chance || (allow_non_solid && enemy_non_solid) {
        let attack_finished = now + 2.0 * context.game.random();
        context.state_mut().attack_state = MonsterAttackState::Missile;
        context.state_mut().attack_finished = attack_finished;
        return true;
    }
    if context.state().locomotion == MonsterLocomotion::Fly {
        let sliding = context.game.random() < 0.3;
        context.state_mut().attack_state = if sliding {
            MonsterAttackState::Sliding
        } else {
            MonsterAttackState::Straight
        };
    }
    false
}

/// Run one boss explosion step (`advanceBossExplosion`).
fn advance_boss_explosion(context: &mut MonsterContext) {
    const POSITIONS: [(i32, i32); 8] = [
        (-24, -24),
        (24, 24),
        (24, -24),
        (-24, 24),
        (-48, -48),
        (48, 48),
        (-48, 48),
        (48, -48),
    ];
    let height = 24.0 + (context.game.random() * 16.0).floor();
    let index = {
        let entity = context.entity_mut();
        let index = entity.count;
        entity.count += 1;
        index
    };
    if index == 8 {
        stop_loop(context);
        let actor = context.actor().clone();
        for _ in 0..4 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                500.0,
                Q2GibOptions::default(),
            );
        }
        for _ in 0..8 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_metal/tris.md2",
                500.0,
                Q2GibOptions {
                    metallic: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/chest/tris.md2",
            500.0,
            Q2GibOptions::default(),
        );
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/gear/tris.md2",
            500.0,
            Q2GibOptions {
                metallic: true,
                head: true,
                ..Q2GibOptions::default()
            },
        );
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    let (ox, oy) = POSITIONS.get(index as usize).copied().unwrap_or((0, 0));
    let actor = context.actor().clone();
    let origin = add3(
        context.game.body_of(actor.clone()).origin,
        vec3(ox as f32, oy as f32, height as f32),
    );
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    let think = context
        .game
        .source_callbacks
        .resolve_think(Some("q2:BossExplode"))
        .unwrap_or_else(|| panic!("Boss explosion source callback was not registered"));
    context.game.schedule(actor, 0.1, think);
}

/// Fire the boss explosion sequence (`bossExplode`).
pub fn boss_explode(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let think = context
        .game
        .source_callbacks
        .resolve_think(Some("q2:BossExplode"))
        .unwrap_or_else(|| panic!("Boss explosion source callback was not registered"));
    think(actor, &mut *context.game);
}

/// Attach boss explosion callbacks (`withBossExplosionCallbacks`).
pub fn with_boss_explosion_callbacks(mut definition: Q2MonsterDefinition) -> Q2MonsterDefinition {
    let mut callbacks = definition.source_callbacks.take().unwrap_or_default();
    callbacks.think.insert("q2:BossExplode", boss_explosion_think);
    definition.source_callbacks = Some(callbacks);
    definition
}

/// Boss explosion think (`q2:BossExplode`).
pub fn boss_explosion_think(actor: qa_core::identity::ActorId, game: &mut crate::q2::foundation::host::Q2GameServices) {
    if !game.monsters.states.contains_key(&actor) {
        panic!("Boss explosion has no restored source monster context");
    }
    let mut context = MonsterContext::new(actor, game);
    advance_boss_explosion(&mut context);
}
