//! Q2 CTF native grapple hooks (`src/content/q2/multiplayer/ctf/native-grapple-hooks.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q2::base::player::types::Q2PlayerHand;
use crate::q2::equipment::grapple_services::{
    GrappleAnchor, GrappleCableEvent, GrappleHand, GrappleHooks, GrappleNoise, GrapplePose,
};
use crate::q2::foundation::host::{Q2GameServices, Q2Solid};
use crate::q2::foundation::weapons::ballistics::{silencer_shots, weapon_player_noise, NoiseKind};
use crate::q2::foundation::weapons::types::WeaponHand;

use super::types::Q2CtfEvent;

/// Read the grapple pose (`pose`).
fn grapple_pose(actor: ActorId, game: &mut Q2GameServices, weapon_aim: bool) -> GrapplePose {
    if game.entity(&actor).is_none() {
        panic!("Native grapple binding requires its source player entity");
    }
    let stored = if weapon_aim {
        game.weapons.inputs.get(&actor).map(|input| (input.angles, input.hand))
    } else {
        None
    };
    let angles = match stored {
        Some((angles, _)) => angles,
        None => game
            .host
            .player_view_state(&actor)
            .map(|view| view.view_angles)
            .unwrap_or_else(|| game.body_of(actor.clone()).angles),
    };
    let hand = match stored {
        Some((_, WeaponHand::Left)) => GrappleHand::Left,
        Some((_, WeaponHand::Center)) => GrappleHand::Center,
        Some((_, WeaponHand::Right)) => GrappleHand::Right,
        None => {
            let hooks = super::ctf_hooks(game);
            match (hooks.player)(actor.clone(), game).map(|player| player.hand) {
                Some(Q2PlayerHand::Left) => GrappleHand::Left,
                Some(Q2PlayerHand::Center) => GrappleHand::Center,
                _ => GrappleHand::Right,
            }
        }
    };
    let record = game.require_entity(&actor);
    GrapplePose {
        angles,
        hand,
        view_height: f64::from(record.view_height),
        gravity: record.gravity,
        gravity_vector: record.gravity_vector,
    }
}

/// Read the grapple pose with weapon aim (`pose`).
fn ctf_grapple_pose_aim(actor: ActorId, game: &mut Q2GameServices) -> GrapplePose {
    grapple_pose(actor, game, true)
}

/// Read the grapple pose without weapon aim (`pose`).
fn ctf_grapple_pose(actor: ActorId, game: &mut Q2GameServices) -> GrapplePose {
    grapple_pose(actor, game, false)
}

/// Read the anchor kind (`anchor`).
fn grapple_anchor(actor: ActorId, game: &mut Q2GameServices, weapon_aim: bool) -> GrappleAnchor {
    if actor == game.host.world_actor() {
        return GrappleAnchor::World;
    }
    let (classname, solid) = match game.entity(&actor) {
        Some(entity) => (entity.classname.clone(), Some(entity.solid)),
        None => (String::new(), None),
    };
    if classname == "bodyque" {
        return GrappleAnchor::Corpse;
    }
    if !weapon_aim && (classname.starts_with("info_flag") || classname.starts_with("func")) {
        return GrappleAnchor::Brush;
    }
    if solid == Some(Q2Solid::None) {
        return GrappleAnchor::None;
    }
    if game.host.is_player(&actor) {
        return GrappleAnchor::Player;
    }
    match solid {
        Some(Q2Solid::Box) => GrappleAnchor::Box,
        Some(Q2Solid::Brush) => GrappleAnchor::Brush,
        _ => GrappleAnchor::None,
    }
}

/// Read the anchor kind with weapon aim (`anchor`).
fn ctf_grapple_anchor_aim(actor: ActorId, game: &mut Q2GameServices) -> GrappleAnchor {
    grapple_anchor(actor, game, true)
}

/// Read the anchor kind without weapon aim (`anchor`).
fn ctf_grapple_anchor(actor: ActorId, game: &mut Q2GameServices) -> GrappleAnchor {
    grapple_anchor(actor, game, false)
}

/// Whether the actor is dead (`dead`).
fn ctf_grapple_dead(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let hooks = super::ctf_hooks(game);
    if (hooks.player)(actor.clone(), game).is_some_and(|player| player.dead) {
        return true;
    }
    game.host
        .combat()
        .read(&actor)
        .is_some_and(|combat| combat.can_take_damage && combat.health <= 0.0)
}

/// Read the previous velocity (`previousVelocity`).
fn ctf_grapple_previous_velocity(actor: ActorId, game: &mut Q2GameServices) -> Vec3 {
    let hooks = super::ctf_hooks(game);
    (hooks.player)(actor, game)
        .map(|player| player.old_velocity)
        .unwrap_or_default()
}

/// Write the previous velocity (`setPreviousVelocity`).
fn ctf_grapple_set_previous_velocity(actor: ActorId, velocity: Vec3, game: &mut Q2GameServices) {
    let hooks = super::ctf_hooks(game);
    if let Some(player) = (hooks.player)(actor, game) {
        player.old_velocity = velocity;
    }
}

/// Read the grapple volume (`volume`).
fn ctf_grapple_volume(actor: ActorId, game: &mut Q2GameServices) -> f64 {
    if silencer_shots(game, &actor) > 0 {
        0.2
    } else {
        1.0
    }
}

/// Emit a grapple noise (`noise`).
fn ctf_grapple_noise(actor: ActorId, game: &mut Q2GameServices, origin: Vec3, kind: GrappleNoise) {
    weapon_player_noise(
        game,
        &actor,
        origin,
        match kind {
            GrappleNoise::Weapon => NoiseKind::Weapon,
            GrappleNoise::Impact => NoiseKind::Impact,
        },
    );
}

/// Suppress or restore grapple prediction (`setGrapplePrediction`).
fn ctf_grapple_set_prediction(actor: ActorId, suppressed: bool, game: &mut Q2GameServices) {
    let hooks = super::ctf_hooks(game);
    (hooks.set_grapple_prediction)(actor, game, suppressed);
}

/// Read gravity (`gravity`).
fn ctf_grapple_gravity(game: &mut Q2GameServices) -> f64 {
    let hooks = super::ctf_hooks(game);
    (hooks.gravity)(game)
}

/// Emit a grapple cable (`emit`).
fn ctf_grapple_emit(event: GrappleCableEvent, game: &mut Q2GameServices) {
    let hooks = super::ctf_hooks(game);
    (hooks.emit)(
        game,
        Q2CtfEvent::GrappleCable {
            actor: event.actor,
            start: event.start,
            end: event.end,
            offset: event.offset,
        },
    );
}

/// Build native grapple hooks (`nativeGrappleHooks`).
///
/// The hooks resolve the installed [`Q2CtfHooks`](super::types::Q2CtfHooks)
/// from the arena; weapon aim selects the hook variants.
pub fn native_grapple_hooks(weapon_aim: bool) -> GrappleHooks {
    GrappleHooks {
        pose: if weapon_aim {
            ctf_grapple_pose_aim
        } else {
            ctf_grapple_pose
        },
        anchor: if weapon_aim {
            ctf_grapple_anchor_aim
        } else {
            ctf_grapple_anchor
        },
        dead: ctf_grapple_dead,
        previous_velocity: ctf_grapple_previous_velocity,
        set_previous_velocity: ctf_grapple_set_previous_velocity,
        volume: ctf_grapple_volume,
        noise: ctf_grapple_noise,
        set_grapple_prediction: ctf_grapple_set_prediction,
        gravity: ctf_grapple_gravity,
        emit: ctf_grapple_emit,
    }
}
