//! Widow death debris (`src/content/q2/missionpacks/monsters/widow/death.ts`).
//!
//! Original Rogue m_widow2.c and g_spawn.c death debris.
//! ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, scale3, vec3};

use super::widow_common::widow_project;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent,
    Q2Solid,
};
use crate::q2::foundation::monsters::gibs::q2_gib_callbacks;
use crate::q2::foundation::monsters::resume_monster;
use crate::q2::foundation::monsters::types::MonsterContext;
use crate::q2::support::contracts::{DeathReaction, TouchContact};

/// Meat gib model.
const MEAT: &str = "models/objects/gibs/sm_meat/tris.md2";
/// Metal gib model.
const METAL: &str = "models/objects/gibs/sm_metal/tris.md2";

/// Gib free (`free`).
fn widow_gib_free(actor: ActorId, game: &mut Q2GameServices) {
    game.remove_actor(actor);
}

/// Gib die (`die`).
fn widow_gib_die(actor: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    game.remove_actor(actor);
}

/// Gib touch (`touch`).
fn widow_gib_touch(actor: ActorId, game: &mut Q2GameServices, _contact: TouchContact) {
    game.set_solid(actor.clone(), Q2Solid::None);
    game.require_entity_mut(&actor).touch = None;
    game.require_entity_mut(&actor).angular_velocity = vec3(0.0, 0.0, 0.0);
    let mut moved = game.body_of(actor.clone());
    moved.angles = vec3(0.0, moved.angles.y, 0.0);
    game.write_body(actor.clone(), &moved, true);
    let motion = game.require_entity(&actor).motion;
    game.set_motion_kind(actor.clone(), motion);
    let noise = game.require_entity(&actor).noise.clone();
    if !noise.is_empty() {
        game.sound(&actor, &noise, 2, 1.0, 1.0);
    }
}

/// Clip (`clip`).
fn clip_velocity(velocity: Vec3) -> Vec3 {
    vec3(
        velocity.x.clamp(-300.0, 300.0),
        velocity.y.clamp(-300.0, 300.0),
        velocity.z.clamp(200.0, 500.0),
    )
}

/// Widow gib (`widowGib`).
#[allow(clippy::too_many_arguments)]
pub fn widow_gib(
    owner: &ActorId,
    game: &mut Q2GameServices,
    model: &str,
    damage: f64,
    organic: bool,
    origin: Option<Vec3>,
    sized: bool,
    hit_sound: &str,
    fade: bool,
) -> ActorId {
    let gib = game.create("gib", BTreeMap::new());
    let body = game.body_of(owner.clone());
    let half = scale3(
        vec3(
            body.bounds.max.x - body.bounds.min.x,
            body.bounds.max.y - body.bounds.min.y,
            body.bounds.max.z - body.bounds.min.z,
        ),
        0.5,
    );
    let center = add3(
        body.origin,
        add3(body.bounds.min, add3(half, vec3(-1.0, -1.0, -1.0))),
    );
    let point = origin.unwrap_or_else(|| {
        add3(
            center,
            vec3(
                ((game.random() * 2.0 - 1.0) as f32) * half.x,
                ((game.random() * 2.0 - 1.0) as f32) * half.y,
                ((game.random() * 2.0 - 1.0) as f32) * half.z,
            ),
        )
    });
    {
        let entity = game.require_entity_mut(&gib);
        entity.effects = 2;
        entity.flags |= 2048;
        entity.render_flags |= 32768;
        entity.die = Some(widow_gib_die);
    }
    let lifetime = (if fade {
        if sized { 20.0 } else { 5.0 }
    } else if sized {
        60.0
    } else {
        25.0
    }) + game.random() * if sized { 15.0 } else { 10.0 };
    let mut velocity = clip_velocity(add3(
        body.velocity,
        scale3(
            vec3(
                (damage * (game.random() * 2.0 - 1.0)) as f32,
                (damage * (game.random() * 2.0 - 1.0)) as f32,
                (damage * (game.random() * 2.0 - 1.0) + 200.0) as f32,
            ),
            if organic { 0.5 } else { 1.0 },
        ),
    ));
    game.require_entity_mut(&gib).model = model.to_string();
    let mut bounds = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    if sized {
        game.require_entity_mut(&gib).noise = hit_sound.to_string();
        game.require_entity_mut(&gib).angular_velocity = vec3(
            (game.random() * 400.0) as f32,
            (game.random() * 400.0) as f32,
            (game.random() * 200.0) as f32,
        );
        velocity = clip_velocity(vec3(velocity.x * 2.0, velocity.y * 2.0, velocity.z.abs()));
        velocity.z = (350.0 + game.random() * 100.0).max(f64::from(velocity.z)) as f32;
        {
            let entity = game.require_entity_mut(&gib);
            entity.gravity = 0.25;
            entity.touch = Some(widow_gib_touch);
            entity.owner = Some(owner.clone());
        }
        let size = if model == "models/monsters/blackwidow2/gib2/tris.md2" {
            10.0
        } else {
            5.0
        };
        bounds = Bounds {
            min: vec3(-size, -size, 0.0),
            max: vec3(size, size, size),
        };
    } else {
        velocity = vec3(velocity.x * 2.0, velocity.y * 2.0, velocity.z);
        game.require_entity_mut(&gib).angular_velocity = vec3(
            (game.random() * 600.0) as f32,
            (game.random() * 600.0) as f32,
            (game.random() * 600.0) as f32,
        );
        game.require_entity_mut(&gib).touch = if organic {
            game.source_callbacks.resolve_touch(Some("gib_touch"))
        } else {
            None
        };
    }
    let mut moved = game.body_of(gib.clone());
    moved.origin = point;
    moved.velocity = velocity;
    moved.bounds = bounds;
    game.write_body(gib.clone(), &moved, false);
    let owned = game.owned_of(gib.clone());
    game.create_combat(&owned, 0.0, 0.0, true);
    game.set_solid(gib.clone(), if sized { Q2Solid::Box } else { Q2Solid::None });
    game.set_motion_kind(
        gib.clone(),
        if organic {
            Q2MotionKind::Toss
        } else {
            Q2MotionKind::Bounce
        },
    );
    game.schedule(gib.clone(), lifetime, widow_gib_free);
    game.link_actor(gib.clone());
    game.show(gib.clone());
    gib
}

/// Widow effect (`widowEffect`).
pub fn widow_effect(game: &mut Q2GameServices, origin: Vec3, effect: &str, count: i32) {
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: effect.to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count,
        color: 0,
    }));
}

/// Small (`small`).
fn widow_small(owner: &ActorId, game: &mut Q2GameServices, point: Vec3) {
    for _ in 0..2 {
        widow_gib(owner, game, MEAT, 300.0, true, Some(point), false, "", false);
    }
    widow_gib(owner, game, METAL, 300.0, false, Some(point), false, "", false);
    widow_gib(owner, game, METAL, 100.0, false, Some(point), false, "", false);
}

/// More (`more`).
fn widow_more(owner: &ActorId, game: &mut Q2GameServices, point: Vec3) {
    if game.options.mode == Q2Mode::Coop {
        widow_small(owner, game, point);
        return;
    }
    widow_gib(owner, game, MEAT, 300.0, true, Some(point), false, "", false);
    for _ in 0..2 {
        widow_gib(owner, game, METAL, 300.0, false, Some(point), false, "", false);
    }
    for _ in 0..3 {
        widow_gib(owner, game, METAL, 100.0, false, Some(point), false, "", false);
    }
}

/// Legs think (`legsThink`).
fn widow_legs_think(actor: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&actor).frame == 17 {
        let point = widow_project(&actor, game, vec3(11.77, -7.24, 23.31));
        widow_effect(game, point, "q2:explosion1", 1);
        widow_small(&actor, game, point);
    }
    if game.require_entity(&actor).frame < 23 {
        game.require_entity_mut(&actor).frame += 1;
        game.show(actor.clone());
        game.schedule(actor, 0.1, widow_legs_think);
        return;
    }
    if game.require_entity(&actor).wait == 0.0 {
        let wait = game.host.now() + 1.0;
        game.require_entity_mut(&actor).wait = wait;
    }
    if game.host.now() > game.require_entity(&actor).wait {
        for (offset, pieces) in [
            (vec3(-65.6, -8.44, 28.59), 2),
            (vec3(-1.04, -51.18, 7.04), 3),
        ] {
            let point = widow_project(&actor, game, offset);
            widow_effect(game, point, "q2:explosion1", 1);
            widow_small(&actor, game, point);
            for i in 1..=pieces {
                let model = format!("models/monsters/blackwidow/gib{i}/tris.md2");
                let damage = 80.0 + (game.random() * 20.0).trunc();
                widow_gib(&actor, game, &model, damage, false, Some(point), true, "", true);
            }
        }
        game.remove_actor(actor);
        return;
    }
    if game.host.now() > game.require_entity(&actor).wait - 0.5
        && game.require_entity(&actor).count == 0
    {
        game.require_entity_mut(&actor).count = 1;
        let first = widow_project(&actor, game, vec3(31.0, -88.7, 10.96));
        widow_effect(game, first, "q2:explosion1", 1);
        let second = widow_project(&actor, game, vec3(-12.67, -4.39, 15.68));
        widow_effect(game, second, "q2:explosion1", 1);
    }
    game.schedule(actor, 0.1, widow_legs_think);
}

/// Spawn widow legs (`spawnWidowLegs`).
pub fn spawn_widow_legs(owner: &ActorId, game: &mut Q2GameServices) {
    let legs = game.create("widowlegs", BTreeMap::new());
    let body = game.body_of(owner.clone());
    {
        let entity = game.require_entity_mut(&legs);
        entity.model = "models/monsters/legs/tris.md2".to_string();
        entity.render_flags = 32768;
    }
    let mut moved = game.body_of(legs.clone());
    moved.origin = body.origin;
    moved.angles = body.angles;
    game.write_body(legs.clone(), &moved, false);
    game.set_solid(legs.clone(), Q2Solid::None);
    game.set_motion_kind(legs.clone(), Q2MotionKind::Stationary);
    game.schedule(legs.clone(), 0.1, widow_legs_think);
    game.link_actor(legs.clone());
    game.show(legs);
}

/// Widow debris callbacks (`widowDebrisCallbacks`).
pub fn widow_debris_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = q2_gib_callbacks();
    callbacks.think.insert("q2:rogue/widow_gib_free", widow_gib_free);
    callbacks.think.insert("q2:rogue/widowlegs_think", widow_legs_think);
    callbacks.touch.insert("q2:rogue/widow_gib_touch", widow_gib_touch);
    callbacks.die.insert("q2:rogue/widow_gib_die", widow_gib_die);
    callbacks
}

/// Widow explosion (`widowExplosion`).
pub fn widow_explosion(context: &mut MonsterContext, offset: Vec3) {
    let actor = context.actor().clone();
    let point = widow_project(&actor, &mut *context.game, offset);
    widow_effect(&mut *context.game, point, "q2:explosion1", 1);
    widow_gib(&actor, &mut *context.game, MEAT, 300.0, true, Some(point), false, "", false);
    widow_gib(&actor, &mut *context.game, METAL, 100.0, false, Some(point), false, "", false);
    for _ in 0..2 {
        widow_gib(&actor, &mut *context.game, METAL, 300.0, false, Some(point), false, "", false);
    }
}

/// Widow explosion leg (`widowExplosionLeg`).
pub fn widow_explosion_leg(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    for (offset, piece, damage) in [
        (vec3(-31.89, -47.86, 67.02), 2, 200.0),
        (vec3(-44.9, -82.14, 54.72), 1, 300.0),
    ] {
        let point = widow_project(&actor, &mut *context.game, offset);
        widow_effect(
            &mut *context.game,
            point,
            if piece == 2 {
                "q2:explosion1_big"
            } else {
                "q2:explosion1"
            },
            1,
        );
        let model = format!("models/monsters/blackwidow2/gib{piece}/tris.md2");
        widow_gib(
            &actor,
            &mut *context.game,
            &model,
            damage,
            false,
            Some(point),
            true,
            "misc/fhit3.wav",
            false,
        );
        widow_gib(&actor, &mut *context.game, MEAT, 300.0, true, Some(point), false, "", false);
        widow_gib(&actor, &mut *context.game, METAL, 100.0, false, Some(point), false, "", false);
    }
}

/// Widow explode think (`createWidowExplode`).
pub fn widow_explode_think(actor: ActorId, game: &mut Q2GameServices) {
    if !game.monsters.states.contains_key(&actor) {
        panic!("Widow explosion lost its source controller");
    }
    let origin = game.body_of(actor.clone()).origin;
    let mut point = vec3(
        origin.x,
        origin.y,
        origin.z + 24.0 + ((game.random() * 2147483648.0).floor() as i64 & 15) as f32,
    );
    if game.require_entity(&actor).count < 8 {
        point.z += 24.0 + ((game.random() * 2147483648.0).floor() as i64 & 31) as f32;
    }
    match game.require_entity(&actor).count {
        0 => {
            point = add3(point, vec3(-24.0, -24.0, 0.0));
        }
        1 => {
            point = add3(point, vec3(24.0, 24.0, 0.0));
            widow_small(&actor, game, point);
        }
        2 => {
            point = add3(point, vec3(24.0, -24.0, 0.0));
        }
        3 => {
            point = add3(point, vec3(-24.0, 24.0, 0.0));
            widow_more(&actor, game, point);
        }
        4 => {
            point = add3(point, vec3(-48.0, -48.0, 0.0));
        }
        5 => {
            point = add3(point, vec3(48.0, 48.0, 0.0));
            let arm = widow_project(&actor, game, vec3(65.76, 17.52, 7.56));
            widow_effect(game, arm, "q2:explosion1_big", 1);
            for _ in 0..2 {
                widow_gib(&actor, game, METAL, 100.0, false, Some(arm), false, "", false);
            }
        }
        6 => {
            point = add3(point, vec3(-48.0, 48.0, 0.0));
            let arm = widow_project(&actor, game, vec3(65.76, 17.52, 7.56));
            widow_gib(
                &actor,
                game,
                "models/monsters/blackwidow2/gib4/tris.md2",
                200.0,
                false,
                Some(arm),
                true,
                "misc/fhit3.wav",
                false,
            );
            widow_gib(&actor, game, MEAT, 300.0, true, Some(arm), false, "", false);
        }
        7 => {
            point = add3(point, vec3(48.0, -48.0, 0.0));
            widow_small(&actor, game, point);
        }
        8 => {
            point = add3(origin, vec3(18.0, 18.0, 48.0));
            widow_more(&actor, game, point);
        }
        9 => {
            point = add3(origin, vec3(-18.0, 18.0, 48.0));
        }
        10 => {
            point = add3(origin, vec3(18.0, -18.0, 48.0));
        }
        11 => {
            point = add3(origin, vec3(-18.0, -18.0, 48.0));
        }
        _ => {
            game.require_entity_mut(&actor).sound = String::new();
            widow_gib(&actor, game, MEAT, 400.0, true, None, false, "", true);
            for _ in 0..2 {
                widow_gib(&actor, game, METAL, 100.0, false, None, false, "", true);
            }
            for _ in 0..2 {
                widow_gib(&actor, game, METAL, 400.0, false, None, false, "", true);
            }
            if let Some(state) = game.monsters.states.get_mut(&actor) {
                state.dead = true;
            }
            let mut context = MonsterContext::new(actor.clone(), game);
            context.set_move("widow2_move_dead", true);
            let game = &mut *context.game;
            resume_monster(game, actor);
            return;
        }
    }
    game.require_entity_mut(&actor).count += 1;
    let count = game.require_entity(&actor).count;
    widow_effect(
        game,
        point,
        if (9..=12).contains(&count) {
            "q2:explosion1_big"
        } else if count % 2 == 1 {
            "q2:explosion1"
        } else {
            "q2:explosion1_np"
        },
        1,
    );
    game.schedule(actor, 0.1, widow_explode_think);
}
