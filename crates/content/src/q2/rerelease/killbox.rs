//! Q2 rerelease killbox (`src/content/q2/rerelease/killbox.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::add3;

use crate::q2::foundation::fields::ZERO;
use crate::q2::foundation::host::{Q2GameServices, Q2Mode, Q2Solid};

/// Player clip mask bit cleared for coop teammates.
const PLAYER_CLIP_BIT: i32 = 0x40000000;

/// Kill everything intersecting an entity's box (`killQ2RereleaseBox`).
///
/// Rerelease KillBox visits the complete linked overlap set and protects
/// coop teammates.
pub fn kill_q2_rerelease_box(entity: ActorId, game: &mut Q2GameServices, spawning: bool, exact: bool) -> bool {
    if game.players.states.get(&entity).is_some_and(|state| state.noclip) {
        return true;
    }
    let body = game.body_of(entity.clone());
    let min = add3(body.origin, body.bounds.min);
    let max = add3(body.origin, body.bounds.max);
    let model = game.require_entity(&entity).model.clone();
    let hooks = super::rerelease_hooks(game);
    let observations: Vec<ActorId> = game
        .host
        .actors()
        .observations()
        .into_iter()
        .map(|observation| observation.id)
        .collect();
    for actor in observations {
        if actor == entity {
            continue;
        }
        let target = game.entity(&actor).cloned();
        let linked = game.host.bodies().linked(&actor);
        let damageable = game
            .host
            .combat()
            .read(&actor)
            .is_some_and(|combat| combat.can_take_damage);
        if linked.is_none() || !damageable || target.as_ref().is_some_and(|target| target.solid != Q2Solid::Box) {
            continue;
        }
        let bounds = linked
            .map(|linked| linked.absolute_bounds)
            .expect("Q2 rerelease linked body vanished");
        if bounds.max.x < min.x
            || bounds.min.x > max.x
            || bounds.max.y < min.y
            || bounds.min.y > max.y
            || bounds.max.z < min.z
            || bounds.min.z > max.z
        {
            continue;
        }
        if spawning
            && game.options.mode == Q2Mode::Coop
            && !game.rerelease.options.coop_player_collision
            && game.host.is_player(&actor)
        {
            continue;
        }
        if exact && model.starts_with('*') && !(hooks.clip_trigger)(entity.clone(), actor.clone(), game) {
            continue;
        }
        if game.options.mode == Q2Mode::Coop && game.host.is_player(&entity) && game.host.is_player(&actor) {
            game.require_entity_mut(&entity).clip_mask &= !PLAYER_CLIP_BIT;
            if let Some(target) = game.entity_mut(&actor) {
                target.clip_mask &= !PLAYER_CLIP_BIT;
            }
            if let Some(player_collision) = hooks.player_collision {
                player_collision(entity.clone(), game, false);
                player_collision(actor, game, false);
            }
            continue;
        }
        game.damage(
            actor,
            entity.clone(),
            Some(entity.clone()),
            100000.0,
            0.0,
            ZERO,
            body.origin,
            ZERO,
            if spawning { 57 } else { 21 },
            32,
            None,
        );
    }
    true
}
