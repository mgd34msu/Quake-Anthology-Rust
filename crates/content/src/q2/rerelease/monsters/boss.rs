//! Rerelease boss explosion (`src/content/q2/rerelease/monsters/boss.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, vec3};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2EffectEvent, Q2GameServices, Q2PresentationEvent};
use crate::q2::foundation::monsters::types::MonsterContext;

/// Random body point (`randomBodyPoint`).
pub fn random_body_point(actor: &ActorId, game: &mut Q2GameServices) -> Vec3 {
    let body = game.body_of(actor.clone());
    vec3(
        body.origin.x + body.bounds.min.x
            + (game.random() * f64::from(body.bounds.max.x - body.bounds.min.x)) as f32,
        body.origin.y + body.bounds.min.y
            + (game.random() * f64::from(body.bounds.max.y - body.bounds.min.y)) as f32,
        body.origin.z + body.bounds.min.z
            + (game.random() * f64::from(body.bounds.max.z - body.bounds.min.z)) as f32,
    )
}

/// Boss explode think (`bossExplodeThink`).
pub fn boss_explode_think(actor: ActorId, game: &mut Q2GameServices) {
    let (owner, model, view_height) = {
        let entity = game.require_entity(&actor);
        (entity.owner.clone(), entity.model.clone(), entity.view_height)
    };
    let owner_entity = owner.as_ref().and_then(|owner| game.entity(owner)).cloned();
    match owner_entity {
        Some(owner_entity) if owner_entity.model == model => {
            let owner_id = owner_entity.actor.id().clone();
            let point = random_body_point(&owner_id, game);
            game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                effect: if view_height % 3 == 0 {
                    "q2:explosion1"
                } else {
                    "q2:explosion1-nl"
                }
                .to_string(),
                origin: point,
                direction: vec3(0.0, 0.0, 0.0),
                count: 1,
                color: 0,
            }));
            game.require_entity_mut(&actor).view_height += 1;
            let delay = 0.05 + game.random() * 0.15;
            game.schedule(actor, delay, boss_explode_think);
        }
        _ => {
            game.remove_actor(actor);
        }
    }
}

/// Boss explode (`bossExplode`).
pub fn boss_explode(context: &mut MonsterContext) {
    if context.entity().spawnflags & (1 << 16) != 0 {
        return;
    }
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("BossExplode_think", boss_explode_think);
    context.game.source_callbacks.register(&callbacks);
    let actor = context.actor().clone();
    let exploder = context.game.create("boss_exploder", BTreeMap::new());
    let model = context.entity().model.clone();
    {
        let entity = context.game.require_entity_mut(&exploder);
        entity.owner = Some(actor);
        entity.model = model;
        entity.visible = false;
        entity.view_height = 0;
    }
    let delay = 0.075 + context.game.random() * 0.175;
    context.game.schedule(exploder, delay, boss_explode_think);
}
