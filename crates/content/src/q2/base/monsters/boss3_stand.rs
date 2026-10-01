//! Boss3 stand-in (`src/content/q2/base/monsters/boss3-stand.ts`).
//!
//! Quake II m_boss3.c. id Software, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, vec3};

use super::tables::boss32::boss32_frame;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid,
    Q2SpawnFn, Q2Think, Q2Use, SpawnModule,
};

/// Boss3 stand think (`think`).
pub fn boss3_stand_think(actor: ActorId, game: &mut Q2GameServices) {
    let frame = game.require_entity(&actor).frame;
    game.require_entity_mut(&actor).frame = if frame == boss32_frame::STAND260 {
        boss32_frame::STAND201
    } else {
        frame + 1
    };
    game.show(actor.clone());
    game.schedule(actor, 0.1, boss3_stand_think as Q2Think);
}

/// Boss3 stand use (`use`).
pub fn boss3_stand_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let origin = game.body_of(actor.clone()).origin;
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:boss-teleport".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    game.remove_actor(actor);
}

/// Spawn a boss3 stand-in (`spawn`).
pub fn spawn_boss3_stand(actor: ActorId, game: &mut Q2GameServices) -> bool {
    if game.require_entity(&actor).classname != "monster_boss3_stand" {
        return false;
    }
    if game.options.mode == Q2Mode::Deathmatch {
        game.remove_actor(actor);
        return true;
    }
    {
        let entity = game.require_entity_mut(&actor);
        entity.model = "models/monsters/boss3/rider/tris.md2".to_string();
        entity.frame = boss32_frame::STAND201;
    }
    let mut body = game.body_of(actor.clone());
    body.bounds = Bounds {
        min: Vec3 {
            x: -32.0,
            y: -32.0,
            z: 0.0,
        },
        max: Vec3 {
            x: 32.0,
            y: 32.0,
            z: 90.0,
        },
    };
    game.write_body(actor.clone(), &body, true);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Step);
    game.set_solid(actor.clone(), Q2Solid::Box);
    game.show(actor.clone());
    game.require_entity_mut(&actor).use_ = Some(boss3_stand_use as Q2Use);
    game.schedule(actor, 0.1, boss3_stand_think as Q2Think);
    true
}

/// Boss3 stand module (`q2Boss3StandModule`).
pub fn boss3_stand_module() -> SpawnModule {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert(
        "q2:base/Think_Boss3Stand",
        boss3_stand_think as Q2Think,
    );
    callbacks
        .use_
        .insert("q2:base/Use_Boss3", boss3_stand_use as Q2Use);
    SpawnModule {
        spawn: spawn_boss3_stand as Q2SpawnFn,
        item_name: |_| None,
        callbacks,
    }
}
