//! Monster gibs (`src/content/q2/foundation/monsters/gibs.ts`).
//!
//! Quake II `g_misc.c` / rerelease `g_misc.cpp` organic and metallic
//! gibs (id Software, GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::ai::{angles_vectors, vector_angles};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2MotionKind};
use crate::q2::support::contracts::{AttackCause, CombatTraitChanges, DeathReaction, TouchContact};

/// Gib throw options (`Q2GibOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Q2GibOptions {
    /// Reuse the dying actor.
    pub head: bool,
    /// Metallic gib physics.
    pub metallic: bool,
    /// Inherit the dying skin.
    pub skinned: bool,
    /// Stay upright on landing.
    pub upright: bool,
}

/// Free a gib (`freeGib`).
pub fn free_gib(actor: ActorId, game: &mut Q2GameServices) {
    game.remove_actor(actor);
}

/// Animate a meat gib (`animateMeat`).
pub fn animate_meat(actor: ActorId, game: &mut Q2GameServices) {
    let frame = {
        let entity = game.require_entity_mut(&actor);
        entity.frame += 1;
        entity.frame
    };
    game.show(actor.clone());
    if frame == 10 {
        let delay = 8.0 + game.random() * 10.0;
        game.schedule(actor, delay, free_gib as crate::q2::foundation::host::Q2Think);
    } else {
        game.schedule(actor, 0.1, animate_meat as crate::q2::foundation::host::Q2Think);
    }
}

/// Gib death reaction (`gibDie`).
pub fn gib_die(actor: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    let cause = game.require_entity(&actor).last_attack.as_ref().map(|attack| attack.cause.clone());
    let removed = game.options.edition == Q2Edition::Classic
        || matches!(cause, Some(AttackCause::Q2 { means_of_death: 20, .. }));
    if removed {
        game.remove_actor(actor);
    }
}

/// Upright gib landing (`gib_touch_upright`).
pub fn upright_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let Some(plane) = contact.plane else { return };
    if plane.normal.z > 0.7 {
        let mut body = game.body_of(actor.clone());
        body.angles.x = body.angles.x.clamp(-5.0, 5.0);
        body.angles.z = body.angles.z.clamp(-5.0, 5.0);
        game.write_body(actor, &body, false);
    }
}

/// Organic gib landing (`gib_touch`).
pub fn organic_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if game.body_of(actor.clone()).ground.is_none() {
        return;
    }
    game.require_entity_mut(&actor).touch = None;
    if let Some(plane) = contact.plane {
        game.sound(&actor, "misc/fhit3.wav", 2, 1.0, 1.0);
        let angles = vector_angles(angles_vectors(vector_angles(plane.normal)).right);
        let mut body = game.body_of(actor.clone());
        body.angles = angles;
        game.write_body(actor.clone(), &body, false);
        if game.require_entity(&actor).model == "models/objects/gibs/sm_meat/tris.md2" {
            game.require_entity_mut(&actor).frame += 1;
            game.show(actor.clone());
            game.schedule(actor, 0.1, animate_meat as crate::q2::foundation::host::Q2Think);
        }
    }
}

/// Named gib callbacks (`q2GibCallbacks`).
pub fn q2_gib_callbacks() -> Q2CallbackDefinitions {
    Q2CallbackDefinitions {
        trajectory: Vec::new(),
        think: HashMap::from([
            ("gib_free", free_gib as crate::q2::foundation::host::Q2Think),
            ("gib_think", animate_meat as crate::q2::foundation::host::Q2Think),
        ]),
        use_: HashMap::new(),
        touch: HashMap::from([
            ("gib_touch", organic_touch as crate::q2::foundation::host::Q2Touch),
            ("gib_touch_upright", upright_touch as crate::q2::foundation::host::Q2Touch),
        ]),
        pain: HashMap::new(),
        die: HashMap::from([("gib_die", gib_die as crate::q2::foundation::host::Q2Die)]),
        blocked: HashMap::new(),
    }
}

/// Throw a gib (`throwGib`).
pub fn throw_gib(
    this: ActorId,
    game: &mut Q2GameServices,
    model: &str,
    damage: f64,
    options: Q2GibOptions,
) -> Option<ActorId> {
    game.source_callbacks.register(&q2_gib_callbacks());
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    let body = game.body_of(this.clone());
    let gib = if options.head {
        this.clone()
    } else {
        game.create("gib", std::collections::BTreeMap::new())
    };
    let half = Vec3 {
        x: (body.bounds.max.x - body.bounds.min.x) * 0.5,
        y: (body.bounds.max.y - body.bounds.min.y) * 0.5,
        z: (body.bounds.max.z - body.bounds.min.z) * 0.5,
    };
    let offset = if rerelease { Vec3 { x: 0.0, y: 0.0, z: 0.0 } } else { Vec3 { x: -1.0, y: -1.0, z: -1.0 } };
    let center = Vec3 {
        x: body.origin.x + body.bounds.min.x + half.x + offset.x,
        y: body.origin.y + body.bounds.min.y + half.y + offset.y,
        z: body.origin.z + body.bounds.min.z + half.z + offset.z,
    };
    let mut origin = body.origin;
    if !options.head || rerelease {
        let mut tries = 0;
        loop {
            origin = Vec3 {
                x: center.x + (game.random() * 2.0 - 1.0) as f32 * half.x,
                y: center.y + (game.random() * 2.0 - 1.0) as f32 * half.y,
                z: center.z + (game.random() * 2.0 - 1.0) as f32 * half.z,
            };
            tries += 1;
            if !(rerelease && tries < 3 && (game.host.point_contents(origin) & 3) != 0) {
                break;
            }
        }
        if rerelease && (game.host.point_contents(origin) & 3) != 0 && !options.head {
            game.remove_actor(gib);
            return None;
        }
    }
    let factor = if damage < 50.0 { 0.7 } else { 1.2 };
    let impulse = Vec3 {
        x: (100.0 * (game.random() * 2.0 - 1.0) * factor) as f32,
        y: (100.0 * (game.random() * 2.0 - 1.0) * factor) as f32,
        z: ((200.0 + 100.0 * game.random()) * factor) as f32,
    };
    let push = if options.metallic { 1.0 } else { 0.5 };
    let velocity = Vec3 {
        x: body.velocity.x + impulse.x * push as f32,
        y: body.velocity.y + impulse.y * push as f32,
        z: body.velocity.z + impulse.z * push as f32,
    };
    let clipped = Vec3 {
        x: velocity.x.clamp(-300.0, 300.0),
        y: velocity.y.clamp(-300.0, 300.0),
        z: velocity.z.clamp(200.0, 500.0),
    };
    let (skin, scale) = {
        let dying = game.require_entity(&this);
        (dying.skin, dying.scale)
    };
    let angular = if options.head && !rerelease {
        let current = game.require_entity(&gib).angular_velocity;
        Vec3 {
            x: current.x,
            y: ((game.random() * 2.0 - 1.0) * 600.0) as f32,
            z: current.z,
        }
    } else {
        Vec3 {
            x: (game.random() * 600.0) as f32,
            y: (game.random() * 600.0) as f32,
            z: (game.random() * 600.0) as f32,
        }
    };
    {
        let gib_entity = game.require_entity_mut(&gib);
        gib_entity.model = model.to_string();
        if rerelease {
            gib_entity.classname = "gib".to_string();
        }
        gib_entity.frame = 0;
        gib_entity.old_frame = -1;
        gib_entity.skin = if options.skinned { skin } else { 0 };
        gib_entity.effects = if rerelease { 2 } else { (gib_entity.effects | 2) & !0x4000 };
        if rerelease {
            gib_entity.render_flags = (1 << 24) | (1 << 13) | (1 << 15);
        }
        gib_entity.flags |= 2048 | if rerelease { (1 << 20) | (1 << 28) } else { 0 };
        gib_entity.server_flags = (gib_entity.server_flags & !4) | if rerelease { 2 } else { 0 };
        gib_entity.clip_mask = 3;
        gib_entity.scale = if rerelease { scale } else { 1.0 };
        gib_entity.angular_velocity = angular;
    }
    let angles = if rerelease {
        Vec3 {
            x: (game.random() * 359.0) as f32,
            y: (game.random() * 359.0) as f32,
            z: (game.random() * 359.0) as f32,
        }
    } else {
        game.body_of(gib.clone()).angles
    };
    let mut moved = game.body_of(gib.clone());
    moved.origin = origin;
    moved.velocity = clipped;
    moved.angles = angles;
    moved.bounds = qa_core::math::Bounds {
        min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    };
    moved.ground = None;
    game.write_body(gib.clone(), &moved, false);
    if game.host.combat().read(&gib).is_none() {
        let owned = game.owned_of(gib.clone());
        game.create_combat(&owned, 0.0, 0.0, true);
    } else {
        let owned = game.owned_of(gib.clone());
        game.set_combat_traits(&owned, &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        });
    }
    {
        let gib_entity = game.require_entity_mut(&gib);
        gib_entity.die = Some(gib_die as crate::q2::foundation::host::Q2Die);
        gib_entity.pain = None;
        gib_entity.use_ = None;
        gib_entity.touch = if rerelease {
            if options.upright {
                Some(upright_touch as crate::q2::foundation::host::Q2Touch)
            } else {
                None
            }
        } else if options.metallic {
            None
        } else {
            Some(organic_touch as crate::q2::foundation::host::Q2Touch)
        };
    }
    game.set_solid(gib.clone(), crate::q2::foundation::host::Q2Solid::None);
    game.set_motion_kind(
        gib.clone(),
        if options.metallic { Q2MotionKind::Bounce } else { Q2MotionKind::Toss },
    );
    let delay = 10.0 + game.random() * 10.0;
    game.schedule(gib.clone(), delay, free_gib as crate::q2::foundation::host::Q2Think);
    game.link_actor(gib.clone());
    game.show(gib.clone());
    Some(gib)
}

/// Throw a head gib (`throwHead`).
pub fn throw_head(
    this: ActorId,
    game: &mut Q2GameServices,
    model: &str,
    damage: f64,
) -> Option<ActorId> {
    throw_gib(this, game, model, damage, Q2GibOptions {
        head: true,
        ..Q2GibOptions::default()
    })
}
