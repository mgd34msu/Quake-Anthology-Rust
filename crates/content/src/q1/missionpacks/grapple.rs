//! Rogue grapple (src/content/q1/missionpacks/grapple.ts).

use qa_core::identity::{ActorId, same_actor};
use qa_core::math::Vec3;

use crate::contract::InventoryEntry;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::extensions::{Q1PlayerExtension, Q1WeaponDefinition};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{
    POINT, Q1BeamStyle, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1Weapon, ZERO,
    length, normalize, vadd, vscale, vsub, weapon_item,
};
use crate::q1::{Q1Error, q1_error};

use super::types::{fround, mission_reference, set_mission_reference, velocity_angles};

/// Grapple view model.
const GRAPPLE_VIEW_MODEL: &str = "progs/v_grpple.mdl";

/// Find a player's live hook (`hook`).
fn find_hook(game: &Q1EntityServices, owner: &ActorId) -> Option<ActorId> {
    game.entities
        .values()
        .find(|entity| {
            entity.classname == "hook"
                && entity
                    .owner
                    .as_ref()
                    .is_some_and(|hook_owner| same_actor(hook_owner, owner))
        })
        .map(|entity| entity.actor.id().clone())
}

/// Present the grapple view model (`presentation`).
fn present_grapple(
    game: &mut Q1EntityServices,
    player: &ActorId,
    punch: i32,
) -> Result<(), Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    game.weapon_punch(player, f64::from(punch))?;
    game.host.emit(Q1Event::Weapon {
        player: state.actor.id().clone(),
        weapon: Q1Weapon::RogueGrapple,
        view_model: GRAPPLE_VIEW_MODEL.to_string(),
        frame: state.weapon_frame,
        punch,
        attack: None,
    });
    Ok(())
}

/// Fire the grapple (`fire`).
fn fire_grapple(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let attack_finished = fround(game.time + 0.1);
    game.update_player(player, |state| state.attack_finished = attack_finished)?;
    if find_hook(game, player).is_some() {
        game.update_player(player, |state| state.weapon_frame = 2)?;
        present_grapple(game, player, 0)?;
        return Ok(true);
    }
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(false),
    };
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let forward = game.make_vectors(state.view_angles).forward;
    let hook = game.create("hook", None, None)?;
    let owner = player.clone();
    game.update_entity(&hook, |hook| {
        hook.owner = Some(owner);
        hook.movement = Q1MoveType::Flymissile;
        hook.solid = Q1Solid::Bbox;
        hook.model = String::from("progs/hook.mdl");
        hook.frame = 1;
        hook.projectile_weapon = Some(Q1Weapon::RogueGrapple);
    })?;
    let touch = game.named.touch("rogue:grapple-anchor")?;
    game.update_entity(&hook, |hook| hook.touch = Some(touch))?;
    game.set_bounds(&hook, POINT)?;
    game.set_body(
        &hook,
        &BodyPatch {
            origin: Some(vadd(
                vadd(body.origin, vscale(forward, 16.0)),
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 16.0,
                },
            )),
            velocity: Some(vscale(forward, 800.0)),
            angles: Some(velocity_angles(forward)),
            ..Default::default()
        },
    )?;
    game.link(&hook)?;
    let reset = game.named.action("rogue:grapple-reset")?;
    game.schedule(&hook, 2.0, &reset)?;
    game.sound(
        player,
        "weapons/chain1.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    let time = game.time;
    game.update_player(player, |state| {
        state.weapon_frame = 1;
        state.weapon_animation_at = time;
        state.continuous_firing = false;
    })?;
    present_grapple(game, player, -2)?;
    Ok(true)
}

/// Grapple fire animation (`animate`).
fn animate_grapple(
    game: &mut Q1EntityServices,
    player: &ActorId,
    _seconds: f64,
) -> Result<(), Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if find_hook(game, player).is_none()
        || state.weapon_frame != 1
        || game.time < state.weapon_animation_at + 0.1
    {
        return Ok(());
    }
    game.update_player(player, |state| state.weapon_frame = 2)?;
    present_grapple(game, player, 0)
}

/// Admit grapple inventory (`attach`).
fn attach_grapple(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    let owned = game
        .player_owned(player)
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let deathmatch = game.options().deathmatch;
    let teamplay = game.options().teamplay.unwrap_or(0);
    game.host.inventory.configure(
        &owned,
        &InventoryEntry {
            item: weapon_item(Q1Weapon::RogueGrapple),
            count: if deathmatch != 0 && teamplay >= 4 {
                1.0
            } else {
                0.0
            },
            capacity: 1.0,
            count_policy: None,
        },
    )
}

/// Remove a hook and restore the owner (`reset`).
fn reset_hook(game: &mut Q1EntityServices, hook: &ActorId) -> Result<(), Q1Error> {
    let owner = game.entity_ref(hook).and_then(|hook| hook.owner.clone());
    if let Some(owner) = owner {
        if game.player_ref(&owner).is_some() {
            let attack_finished = fround(game.time + 0.25);
            let grapple = game
                .player_ref(&owner)
                .is_some_and(|player| player.weapon == Q1Weapon::RogueGrapple);
            game.update_player(&owner, |state| {
                state.weapon_frame = 0;
                state.attack_finished = attack_finished;
                state.weapon_animation_at = -1.0;
            })?;
            if grapple {
                present_grapple(game, &owner, 0)?;
            }
        }
    }
    game.remove(hook)
}

/// Anchor a hook (`anchor`).
fn anchor_hook(
    game: &mut Q1EntityServices,
    hook: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let owner = game.entity_ref(hook).and_then(|hook| hook.owner.clone());
    let player = owner
        .as_ref()
        .and_then(|owner| game.player_ref(owner).cloned());
    let Some(player) = player else {
        return reset_hook(game, hook);
    };
    let player_id = player.actor.id().clone();
    if same_actor(other, &player_id) {
        return Ok(());
    }
    if game.host.contents(game.body(hook)?.origin) == Q1Contents::Sky {
        return reset_hook(game, hook);
    }
    let params = Q1DamageParams {
        weapon: Some(Q1Weapon::RogueGrapple),
        ..Default::default()
    };
    if game.is_player(other) {
        let target_team = game
            .host
            .combat
            .read(other)
            .and_then(|combat| combat.team.clone());
        let owner_team = game
            .host
            .combat
            .read(&player_id)
            .and_then(|combat| combat.team.clone());
        if target_team == owner_team {
            return reset_hook(game, hook);
        }
        game.sound(hook, "player/axhit1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        let _ = game.damage(other, Some(hook), Some(&player_id), 10.0, &params);
    } else {
        game.sound(hook, "player/axhit2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        if game
            .host
            .combat
            .read(other)
            .is_some_and(|combat| combat.can_take_damage)
        {
            let _ = game.damage(other, Some(hook), Some(&player_id), 1.0, &params);
        }
        game.set_body(
            hook,
            &BodyPatch {
                velocity: Some(ZERO),
                ..Default::default()
            },
        )?;
        game.update_entity(hook, |hook| hook.angular_velocity = ZERO)?;
    }
    game.update_entity(hook, |hook| hook.frame = 2)?;
    game.sound(
        player.actor.id(),
        "weapons/tink1.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    if !player.attack_held {
        return reset_hook(game, hook);
    }
    game.update_entity(hook, |hook| {
        hook.count = 1.0;
        hook.solid = Q1Solid::None;
        hook.touch = None;
    })?;
    set_mission_reference(game, hook, "rogue:target", Some(other))?;
    if let Some(body) = game.host.bodies.read(&player_id) {
        game.host.bodies.write(
            &player.actor,
            &BodyPatch {
                ground: Some(None),
                ..Default::default()
            }
            .apply_to(&body),
        )?;
    }
    game.link(hook)?;
    let track = game.named.action("rogue:grapple-track")?;
    game.schedule(hook, 0.0, &track)
}

/// Track an anchored hook (`track`).
fn track_hook(game: &mut Q1EntityServices, hook: &ActorId) -> Result<(), Q1Error> {
    let owner = game.entity_ref(hook).and_then(|hook| hook.owner.clone());
    let player = owner
        .as_ref()
        .and_then(|owner| game.player_ref(owner).cloned());
    let target = mission_reference(game, hook, "rogue:target");
    let dead_target = target
        .as_ref()
        .is_some_and(|target| game.is_player(target) && game.health(target) <= 0.0);
    if player.is_none() || target.is_none() || dead_target {
        return reset_hook(game, hook);
    }
    let player = player.expect("player");
    let target = target.expect("target");
    if game.health(player.actor.id()) <= 0.0 {
        return reset_hook(game, hook);
    }
    let body = match game.host.bodies.read(&target) {
        Some(body) => body,
        None => return reset_hook(game, hook),
    };
    if game.is_player(&target) {
        if game
            .player_ref(&target)
            .map(|target| target.teleport_until)
            .unwrap_or(0.0)
            > game.time
        {
            return reset_hook(game, hook);
        }
        game.set_origin(hook, body.origin)?;
        game.sound(hook, "pendulum/hit.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        let params = Q1DamageParams {
            weapon: Some(Q1Weapon::RogueGrapple),
            ..Default::default()
        };
        let _ = game.damage(&target, Some(hook), Some(player.actor.id()), 1.0, &params);
        game.host.random();
        game.host.random();
        game.host.random();
        game.effect(Q1Effect::Blood, body.origin, Some(&target), 20);
    }
    if game
        .entity_ref(&target)
        .is_some_and(|entity| entity.solid == Q1Solid::Slidebox)
        || game.is_player(&target)
    {
        game.set_body(
            hook,
            &BodyPatch {
                origin: Some(vadd(
                    body.origin,
                    vscale(vadd(body.bounds.min, body.bounds.max), 0.5),
                )),
                velocity: Some(ZERO),
                ..Default::default()
            },
        )?;
    } else {
        game.set_body(
            hook,
            &BodyPatch {
                velocity: Some(body.velocity),
                ..Default::default()
            },
        )?;
    }
    let track = game.named.action("rogue:grapple-track")?;
    game.schedule(hook, 0.1, &track)
}

/// Pull the owner toward an anchored hook (`service`).
fn service_grapple(
    game: &mut Q1EntityServices,
    player: &ActorId,
    _seconds: f64,
) -> Result<(), Q1Error> {
    let hook = find_hook(game, player);
    let body = game.host.bodies.read(player);
    let state = game.player_ref(player).cloned();
    let (Some(hook), Some(body), Some(state)) = (hook, body, state) else {
        return Ok(());
    };
    let origin = game.body(&hook)?.origin;
    let distance = f64::from(length(vsub(origin, body.origin)));
    let count = game.entity_ref(&hook).map(|hook| hook.count).unwrap_or(0.0);
    if count == 1.0 {
        if !state.attack_held && state.weapon == Q1Weapon::RogueGrapple
            || state.teleport_until > game.time
        {
            return reset_hook(game, &hook);
        }
        let basis = game.make_vectors(body.angles);
        let direction = vsub(
            origin,
            vadd(
                vadd(
                    body.origin,
                    vscale(basis.up, if state.jump_held { 0.0 } else { 16.0 }),
                ),
                vscale(basis.forward, 16.0),
            ),
        );
        let speed = f64::from(length(direction));
        game.host.bodies.write(
            &state.actor,
            &BodyPatch {
                velocity: Some(vscale(
                    normalize(direction),
                    if speed <= 100.0 { speed * 10.0 } else { 1000.0 },
                )),
                ground: Some(None),
                ..Default::default()
            }
            .apply_to(&body),
        )?;
    }
    if distance > 50.0 {
        game.host.emit(Q1Event::Beam {
            style: Q1BeamStyle::Grapple,
            actor: hook.clone(),
            start: origin,
            end: vadd(
                body.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 16.0,
                },
            ),
        });
    }
    Ok(())
}

/// Rogue grapple (`RogueGrapple`).
pub struct RogueGrapple;

impl RogueGrapple {
    /// Register the Rogue grapple on a game.
    pub fn new(game: &mut Q1EntityServices) -> Result<Self, Q1Error> {
        game.register_weapon(Q1WeaponDefinition {
            id: Q1Weapon::RogueGrapple,
            item: None,
            ammo: None,
            ammo_per_shot: None,
            model: GRAPPLE_VIEW_MODEL.to_string(),
            rank: 12,
            model_for: None,
            available: None,
            best_available: None,
            fire: fire_grapple,
            animate: Some(animate_grapple),
        })?;
        game.register_player_extension(Q1PlayerExtension {
            id: String::from("rogue:grapple"),
            attach: Some(attach_grapple),
            frame: Some(service_grapple),
            ..Default::default()
        })?;
        game.named.register(
            "rogue:grapple-reset",
            Q1CallbackHandlers {
                action: Some(reset_hook),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:grapple-anchor",
            Q1CallbackHandlers {
                touch: Some(anchor_hook),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:grapple-track",
            Q1CallbackHandlers {
                action: Some(track_hook),
                ..Default::default()
            },
        )?;
        Ok(Self)
    }

    /// Fire the grapple for a player.
    pub fn fire(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
        fire_grapple(game, player)
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;

    fn attached_player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default())
            .expect("attach");
        player
    }

    #[test]
    fn grapple_registers_weapon_and_callbacks() {
        let mut game = test_game();
        RogueGrapple::new(&mut game).expect("grapple");
        assert!(
            game.registered_weapons
                .contains_key(&Q1Weapon::RogueGrapple)
        );
        assert!(game.named.action("rogue:grapple-reset").is_ok());
        assert!(game.named.touch("rogue:grapple-anchor").is_ok());
        assert!(game.named.action("rogue:grapple-track").is_ok());
    }

    #[test]
    fn grapple_fire_creates_hook() {
        let mut game = test_game();
        RogueGrapple::new(&mut game).expect("grapple");
        let player = attached_player(&mut game);
        assert!(RogueGrapple::fire(&mut game, &player).expect("fire"));
        assert!(find_hook(&game, &player).is_some());
        assert_eq!(game.player_ref(&player).expect("state").weapon_frame, 1);
    }

    #[test]
    fn grapple_reset_clears_hook() {
        let mut game = test_game();
        RogueGrapple::new(&mut game).expect("grapple");
        let player = attached_player(&mut game);
        RogueGrapple::fire(&mut game, &player).expect("fire");
        let hook = find_hook(&game, &player).expect("hook");
        reset_hook(&mut game, &hook).expect("reset");
        assert!(find_hook(&game, &player).is_none());
    }
}
