//! Threewave grapple mechanics (`src/content/q1/equipment/threewave-grapple.ts`).
//!
//! Original Threewave 4.00 hook behavior over shared actors: firing,
//! touch attachment, pulling with damage pulses, chain links, and the
//! rerelease beam trail. Equipment owns continuation state; the
//! selected input and map supply only observations and policy through
//! [`ThreewaveGrappleHost`].
//!
//! State lives in a game-owned service table so named callbacks
//! (plain `fn` pointers) can reach it; registration wires the
//! callbacks, the actor-release hook, and the checkpoint extension.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{same_actor, ActorId, OwnedActor, SavedActorId};
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1StateExtension};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyAttachment, BodyFollow, BodyPatch, BodyState, TouchSurface};
use crate::q1::foundation::host::{Q1Contents, Q1ReleaseHook};
use crate::q1::foundation::types::{
    length, normalize, vadd, vectors, vscale, vsub, Q1BeamStyle, Q1Edition, Q1Event, Q1MoveType, Q1Solid,
    Q1SoundChannel, Q1Weapon, POINT, ZERO,
};
use crate::q1::{q1_error, Q1Error};
use crate::value::{arr, boolean, int, num, obj, SaveJson, SaveReader};

/// Grapple input observation (`ThreewaveGrappleInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreewaveGrappleInput {
    /// Fire held.
    pub held: bool,
    /// Release pressed.
    pub release: bool,
    /// Jump held.
    pub jump: bool,
    /// View angles.
    pub view_angles: Vec3,
    /// Teleport lock expiry.
    pub teleport_until: f64,
}

/// Grapple anchor observation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreewaveAnchor {
    /// Anchor is solid.
    pub solid: bool,
    /// Anchor follows the target center.
    pub centered: bool,
    /// Anchor is a player.
    pub player: bool,
}

/// Session policy for the grapple (`ThreewaveGrappleHost`).
pub trait ThreewaveGrappleHost {
    /// Read grapple input for an owner.
    fn input(&self, actor: &ActorId) -> ThreewaveGrappleInput;
    /// Aim the hook from a forward vector.
    fn aim(&self, actor: &ActorId, forward: Vec3) -> Vec3;
    /// Describe a hook anchor.
    fn anchor(&self, actor: &ActorId) -> ThreewaveAnchor;
    /// Whether the hook may attach to a target.
    fn can_attach(&self, owner: &ActorId, target: &ActorId) -> bool;
    /// Whether the hook may pulse damage to a target.
    fn can_pulse(&self, owner: &ActorId, target: &ActorId) -> bool;
    /// Whether the owner may damage a target.
    fn can_damage(&self, target: &ActorId, owner: &ActorId) -> bool;
}

/// Per-owner grapple state (`ThreewaveGrappleState`).
#[derive(Debug, Clone, PartialEq)]
pub struct ThreewaveGrappleState {
    /// Hook is pulling.
    pub pulling: bool,
    /// Weapon animation frame.
    pub weapon_frame: i32,
    /// Attack lock expiry.
    pub attack_finished: f64,
    /// Release deadline.
    pub release_time: f64,
    /// Pending launch animation actor.
    pub animation: Option<ActorId>,
}

/// Game-owned grapple service state.
pub struct ThreewaveGrappleService {
    /// Session policy.
    pub host: Box<dyn ThreewaveGrappleHost>,
    /// Per-owner states.
    pub states: HashMap<ActorId, ThreewaveGrappleState>,
}

fn require_service(game: &Q1EntityServices) -> Result<&ThreewaveGrappleService, Q1Error> {
    game.threewave_grapple
        .as_ref()
        .ok_or_else(|| q1_error("Q1 threewave grapple was not registered"))
}

fn grapple_owner(game: &mut Q1EntityServices, actor: &ActorId) -> Result<OwnedActor, Q1Error> {
    game.host
        .actors
        .resolve_owned(actor)
        .ok_or_else(|| q1_error("Grapple owner is no longer admitted"))
}

fn grapple_body(game: &mut Q1EntityServices, actor: &ActorId) -> Result<BodyState, Q1Error> {
    game.host
        .bodies
        .read(actor)
        .ok_or_else(|| q1_error("Grapple actor has no shared body"))
}

/// Load or create per-owner grapple state (`ThreewaveGrapple.state`).
pub fn grapple_state<'a>(
    game: &'a mut Q1EntityServices,
    actor: &ActorId,
) -> Result<&'a mut ThreewaveGrappleState, Q1Error> {
    grapple_owner(game, actor)?;
    let service = game
        .threewave_grapple
        .as_mut()
        .ok_or_else(|| q1_error("Q1 threewave grapple was not registered"))?;
    Ok(service
        .states
        .entry(actor.clone())
        .or_insert_with(|| ThreewaveGrappleState {
            pulling: false,
            weapon_frame: 0,
            attack_finished: 0.0,
            release_time: 0.0,
            animation: None,
        }))
}

/// Whether an owner is pulling (`ThreewaveGrapple.pulling`).
#[must_use]
pub fn grapple_pulling(game: &Q1EntityServices, actor: &ActorId) -> bool {
    game.threewave_grapple
        .as_ref()
        .and_then(|service| service.states.get(actor))
        .is_some_and(|state| state.pulling)
}

/// Find an owner's live hook (`ThreewaveGrapple.hook`).
#[must_use]
pub fn grapple_hook(game: &Q1EntityServices, actor: &ActorId) -> Option<ActorId> {
    game.entity_ids().into_iter().find(|id| {
        game.entity_ref(id).is_some_and(|entity| {
            entity.classname == "ctf_hook" && entity.owner.as_ref().is_some_and(|owner| same_actor(owner, actor))
        })
    })
}

/// Release an owner's hook, links, and animation (`ThreewaveGrapple.release`).
pub fn grapple_release(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let animation = game
        .threewave_grapple
        .as_ref()
        .and_then(|service| service.states.get(&id))
        .and_then(|state| state.animation.clone())
        .and_then(|animation| game.entity(&animation).map(|entity| entity.actor.id().clone()));
    if let Some(service) = game.threewave_grapple.as_mut() {
        if let Some(state) = service.states.get_mut(&id) {
            state.animation = None;
            state.pulling = false;
        }
    }
    if let Some(animation) = animation {
        game.remove(&animation)?;
    }
    let hook = match grapple_hook(game, &id) {
        Some(hook) => hook,
        None => return Ok(()),
    };
    if game.options().edition == Q1Edition::Classic && game.host.actors.is_live(&id) {
        let owner = grapple_owner(game, &id)?;
        game.sound(owner.id(), "weapons/bounce2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    }
    let owned = game.entity_ref(&hook).map(|entity| entity.actor.clone()).expect("hook");
    game.host.bodies.detach(&owned)?;
    let links: Vec<ActorId> = game
        .entity_ids()
        .into_iter()
        .filter(|id| {
            game.entity_ref(id).is_some_and(|entity| {
                entity.classname == "ctf_hook_link"
                    && entity.owner.as_ref().is_some_and(|owner| same_actor(owner, &hook))
            })
        })
        .collect();
    for link in &links {
        game.remove(link)?;
    }
    game.remove(&hook)
}

fn grapple_vanish(game: &mut Q1EntityServices, hook: &ActorId) -> Result<(), Q1Error> {
    let hook = hook.clone();
    let owner = game.entity_ref(&hook).and_then(|entity| entity.owner.clone());
    match owner {
        Some(owner) => grapple_release(game, &owner),
        None => game.remove(&hook),
    }
}

fn grapple_pull(game: &mut Q1EntityServices, hook: &ActorId) -> Result<(), Q1Error> {
    let hook = hook.clone();
    let entity = game
        .entity_ref(&hook)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let owner = entity.owner.clone();
    let enemy = entity.references.get("ctf.enemy").cloned().flatten();
    let (owner, enemy) = match (owner, enemy) {
        (Some(owner), Some(enemy)) if game.host.actors.is_live(&owner) && game.host.actors.is_live(&enemy) => {
            (owner, enemy)
        }
        _ => return grapple_vanish(game, &hook),
    };
    let service = require_service(game)?;
    let input = service.host.input(&owner);
    let target = service.host.anchor(&enemy);
    grapple_state(game, &owner)?.pulling = true;
    if input.release || input.teleport_until > game.time || game.health(&owner) <= 0.0 || !target.solid {
        return grapple_vanish(game, &hook);
    }
    let enemy_body = grapple_body(game, &enemy)?;
    let enemy_combat = game.host.combat.read(&enemy);
    let service = require_service(game)?;
    if enemy_combat.is_some_and(|combat| combat.can_take_damage) && service.host.can_pulse(&owner, &enemy) {
        let service = require_service(game)?;
        if !service.host.can_damage(&enemy, &owner) {
            return grapple_vanish(game, &hook);
        }
        game.sound(&hook, "blob/land1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        let params = Q1DamageParams {
            death_type: String::from("ctf:grapple"),
            ..Default::default()
        };
        game.damage(&enemy, Some(&hook), Some(&owner), 1.0, &params);
        if !game.is_live(&hook) || !game.host.actors.is_live(&owner) || !game.host.actors.is_live(&enemy) {
            return grapple_vanish(game, &hook);
        }
        let spray = Vec3 {
            x: (100.0 * (game.host.random() * 2.0 - 1.0)) as f32,
            y: (100.0 * (game.host.random() * 2.0 - 1.0)) as f32,
            z: (100.0 * (game.host.random() * 2.0 - 1.0) + 50.0) as f32,
        };
        let origin = game.body(&hook).map(|body| body.origin)?;
        game.host.emit(Q1Event::Particles {
            origin,
            direction: vscale(spray, 0.1),
            color: 73,
            count: 40,
        });
    }
    if target.centered {
        game.set_body(
            &hook,
            &BodyPatch {
                velocity: Some(ZERO),
                origin: Some(vadd(
                    enemy_body.origin,
                    vscale(vadd(enemy_body.bounds.min, enemy_body.bounds.max), 0.5),
                )),
                ..Default::default()
            },
        )?;
    } else {
        game.set_body(
            &hook,
            &BodyPatch {
                velocity: Some(enemy_body.velocity),
                ..Default::default()
            },
        )?;
    }
    let body = grapple_body(game, &owner)?;
    let basis = vectors(body.angles);
    let hook_origin = game.body(&hook).map(|body| body.origin)?;
    let relative = vsub(
        hook_origin,
        vadd(
            body.origin,
            vadd(
                vscale(basis.up, if input.jump { 0.0 } else { 16.0 }),
                vscale(basis.forward, 16.0),
            ),
        ),
    );
    let distance = length(relative);
    let velocity = vscale(
        normalize(relative),
        if distance <= 100.0 {
            f64::from(distance) * 10.0
        } else {
            1000.0
        },
    );
    let traveled = length(vsub(
        body.origin,
        game.entity_ref(&hook)
            .map(|entity| entity.vector("ctf.lastOrigin"))
            .unwrap_or(ZERO),
    ));
    let style = game
        .entity_ref(&hook)
        .map(|entity| entity.number("style"))
        .unwrap_or(0.0);
    if traveled > 10.0 && style == 3.0 {
        if game.options().edition == Q1Edition::Classic {
            let owner = grapple_owner(game, &owner)?;
            game.sound(owner.id(), "weapons/chain2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        }
        game.update_entity(&hook, |entity| {
            entity.fields.insert(String::from("style"), String::from("2"));
        })?;
    }
    if traveled < 10.0 && style == 2.0 {
        if game.options().edition == Q1Edition::Classic {
            let owner = grapple_owner(game, &owner)?;
            game.sound(owner.id(), "weapons/chain3.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        }
        game.update_entity(&hook, |entity| {
            entity.fields.insert(String::from("style"), String::from("3"));
        })?;
    }
    grapple_owner(game, &owner)?;
    game.set_body(
        &owner,
        &BodyPatch {
            velocity: Some(velocity),
            ..Default::default()
        },
    )?;
    game.link(&owner)?;
    game.update_entity(&hook, |entity| {
        entity.fields.insert(
            String::from("ctf.lastOrigin"),
            format!("{} {} {}", body.origin.x, body.origin.y, body.origin.z),
        );
    })?;
    game.link(&hook)?;
    game.schedule(&hook, 0.1, "ctf:hook_pull")
}

fn grapple_touch(
    game: &mut Q1EntityServices,
    hook: &ActorId,
    other: &ActorId,
    surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let hook = hook.clone();
    let other = other.clone();
    let entity = game
        .entity_ref(&hook)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let owner = match entity.owner.clone() {
        Some(owner) if game.host.actors.is_live(&owner) => owner,
        _ => return grapple_vanish(game, &hook),
    };
    if same_actor(&owner, &other) {
        return Ok(());
    }
    let origin = game.body(&hook).map(|body| body.origin)?;
    if surface.is_some_and(|surface| (surface.native_flags & 4) != 0) || game.host.contents(origin) == Q1Contents::Sky {
        return grapple_vanish(game, &hook);
    }
    let service = require_service(game)?;
    if !service.host.can_attach(&owner, &other) {
        return Ok(());
    }
    let service = require_service(game)?;
    let target = service.host.anchor(&other);
    let combat = game.host.combat.read(&other);
    if combat.is_some_and(|combat| combat.can_take_damage) {
        if !target.player {
            game.sound(&hook, "player/axhit2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        }
        let params = Q1DamageParams {
            weapon: Some(Q1Weapon::CtfGrapple),
            death_type: String::from("ctf:grapple"),
            ..Default::default()
        };
        game.damage(&other, Some(&hook), Some(&owner), 10.0, &params);
        if !game.is_live(&hook) || !game.host.actors.is_live(&owner) || !game.host.actors.is_live(&other) {
            return grapple_vanish(game, &hook);
        }
        let body = game.body(&hook)?;
        game.host.emit(Q1Event::Particles {
            origin: body.origin,
            direction: vscale(body.velocity, 0.1),
            color: 73,
            count: 20,
        });
    } else {
        game.sound(&hook, "player/axhit2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        game.update_entity(&hook, |entity| entity.angular_velocity = ZERO)?;
    }
    let service = require_service(game)?;
    if !service.host.input(&owner).held {
        return grapple_vanish(game, &hook);
    }
    let body = grapple_body(game, &other)?;
    if target.centered {
        game.set_body(
            &hook,
            &BodyPatch {
                origin: Some(vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5))),
                velocity: Some(ZERO),
                ..Default::default()
            },
        )?;
    } else {
        game.set_body(
            &hook,
            &BodyPatch {
                velocity: Some(body.velocity),
                ..Default::default()
            },
        )?;
    }
    let hook_body = game.body(&hook)?;
    let owned = game.entity_ref(&hook).map(|entity| entity.actor.clone()).expect("hook");
    game.host.bodies.attach(
        &owned,
        &BodyAttachment {
            anchor: other.clone(),
            follow: if target.centered {
                BodyFollow::Center
            } else {
                BodyFollow::Translation {
                    offset: vsub(hook_body.origin, body.origin),
                }
            },
        },
    )?;
    if game.options().edition == Q1Edition::Classic {
        let owner = grapple_owner(game, &owner)?;
        game.sound(owner.id(), "weapons/chain2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    }
    game.update_entity(&hook, |entity| {
        entity.references.insert(String::from("ctf.enemy"), Some(other));
        entity.fields.insert(String::from("style"), String::from("2"));
        entity.touch = None;
    })?;
    game.link(&hook)?;
    game.schedule(&hook, 0.1, "ctf:hook_pull")
}

/// Fire an owner's hook (`ThreewaveGrapple.fire`).
pub fn grapple_fire(game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
    let id = actor.clone();
    if grapple_hook(game, &id).is_some() || game.health(&id) <= 0.0 {
        return Ok(false);
    }
    let owner = grapple_owner(game, &id)?;
    let body = grapple_body(game, &id)?;
    let service = require_service(game)?;
    let forward = vectors(service.host.input(&id).view_angles).forward;
    let service = require_service(game)?;
    let direction = service.host.aim(&id, forward);
    grapple_state(game, &id)?;
    let hook = game.create("ctf_hook", None, None)?;
    game.update_entity(&hook, |entity| {
        entity.owner = Some(id.clone());
        entity.movement = Q1MoveType::Fly;
        entity.solid = Q1Solid::Bbox;
        entity.model = String::from("progs/star.mdl");
        entity.angular_velocity = Vec3 {
            x: 0.0,
            y: 0.0,
            z: -500.0,
        };
    })?;
    let touch = game.named.touch("ctf:hook_touch")?;
    let fired = format!("{}", game.time as f32);
    game.update_entity(&hook, |entity| {
        entity.touch = Some(touch);
        entity.fields.insert(String::from("ctf.fired"), fired);
    })?;
    let yaw = f64::from(direction.y).atan2(f64::from(direction.x)) * 180.0 / std::f64::consts::PI;
    let pitch = f64::from(direction.z).atan2(f64::from(direction.x.hypot(direction.y))) * 180.0 / std::f64::consts::PI;
    game.set_body(
        &hook,
        &BodyPatch {
            origin: Some(vadd(
                body.origin,
                vadd(
                    vscale(forward, 16.0),
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 16.0,
                    },
                ),
            )),
            bounds: Some(POINT),
            velocity: Some(vscale(direction, 800.0)),
            angles: Some(Vec3 {
                x: pitch as f32,
                y: (if yaw < 0.0 { yaw + 360.0 } else { yaw }) as f32,
                z: 0.0,
            }),
            ..Default::default()
        },
    )?;
    game.link(&hook)?;
    game.sound(owner.id(), "weapons/chain1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    game.schedule(&hook, 0.1, "ctf:hook_flying")?;
    if game.options().edition == Q1Edition::Classic {
        for number in (1..=3).rev() {
            let link = game.create("ctf_hook_link", None, None)?;
            game.update_entity(&link, |entity| {
                entity.owner = Some(hook.clone());
                entity.movement = Q1MoveType::Noclip;
                entity.solid = Q1Solid::None;
                entity.model = String::from("progs/bit.mdl");
                entity.references.insert(String::from("ctf.tail"), Some(id.clone()));
                entity
                    .fields
                    .insert(String::from("weapon"), format!("{}", f64::from(number) / 4.0));
                entity.angular_velocity = Vec3 {
                    x: 310.0,
                    y: 410.0,
                    z: 510.0,
                };
            })?;
            game.set_body(
                &link,
                &BodyPatch {
                    bounds: Some(POINT),
                    angles: Some(Vec3 {
                        x: (31 * number) as f32,
                        y: (41 * number) as f32,
                        z: (51 * number) as f32,
                    }),
                    ..Default::default()
                },
            )?;
            position_link(game, &link)?;
        }
    }
    Ok(true)
}

/// Position a chain link between hook and owner (`positionLink`).
fn position_link(game: &mut Q1EntityServices, link: &ActorId) -> Result<(), Q1Error> {
    let link = link.clone();
    let entity = game
        .entity_ref(&link)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let head = entity
        .owner
        .clone()
        .and_then(|owner| game.entity_ref(&owner).map(|head| head.actor.id().clone()));
    let tail = entity.references.get("ctf.tail").cloned().flatten();
    let (head, tail) = match (head, tail) {
        (Some(head), Some(tail)) if game.host.actors.is_live(&tail) => (head, tail),
        _ => return game.remove(&link),
    };
    let body = grapple_body(game, &tail)?;
    let basis = vectors(body.angles);
    let service = require_service(game)?;
    let end = vadd(
        body.origin,
        vadd(
            vscale(basis.up, if service.host.input(&tail).jump { 0.0 } else { 16.0 }),
            vscale(basis.forward, 16.0),
        ),
    );
    let origin = game.body(&head).map(|body| body.origin)?;
    let fraction = game
        .entity_ref(&link)
        .map(|entity| entity.number("weapon"))
        .unwrap_or(0.0);
    game.set_origin(&link, vadd(origin, vscale(vsub(end, origin), fraction)))?;
    game.schedule(&link, 0.1, "ctf:hook_link")
}

/// Emit the rerelease beam trail (`ThreewaveGrapple.trail`).
pub fn grapple_trail(game: &mut Q1EntityServices, actor: &ActorId) {
    if game.options().edition == Q1Edition::Classic {
        return;
    }
    let hook = match grapple_hook(game, actor) {
        Some(hook) => hook,
        None => return,
    };
    let body = match game.body(&hook) {
        Ok(body) => body,
        Err(_) => return,
    };
    let offset = vscale(vectors(body.angles).forward, -7.0);
    let origin = match grapple_body(game, actor) {
        Ok(body) => body.origin,
        Err(_) => return,
    };
    game.host.emit(Q1Event::Beam {
        style: Q1BeamStyle::Grapple,
        actor: hook,
        start: vadd(
            body.origin,
            Vec3 {
                x: offset.x,
                y: offset.y,
                z: -offset.z,
            },
        ),
        end: vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 16.0,
            },
        ),
    });
}

/// Actor-release cleanup for grapple state.
struct ThreewaveGrappleRelease;

impl Q1ReleaseHook for ThreewaveGrappleRelease {
    fn on_release(&mut self, game: &mut Q1EntityServices, actor: &OwnedActor) {
        let id = actor.id().clone();
        let _ = grapple_release(game, &id);
        if let Some(service) = game.threewave_grapple.as_mut() {
            service.states.remove(&id);
        }
        let links: Vec<ActorId> = game
            .entity_ids()
            .into_iter()
            .filter(|link| {
                game.entity_ref(link).is_some_and(|entity| {
                    entity.classname == "ctf_hook_link"
                        && entity.owner.as_ref().is_some_and(|owner| same_actor(owner, &id))
                })
            })
            .collect();
        for link in &links {
            let _ = game.remove(link);
        }
        let owners: Vec<ActorId> = game
            .threewave_grapple
            .as_ref()
            .map(|service| service.states.keys().cloned().collect())
            .unwrap_or_default();
        for owner in &owners {
            let hooked = grapple_hook(game, owner).is_some();
            if let Some(service) = game.threewave_grapple.as_mut() {
                if let Some(state) = service.states.get_mut(owner) {
                    if state
                        .animation
                        .as_ref()
                        .is_some_and(|animation| same_actor(animation, &id))
                    {
                        state.animation = None;
                    }
                    if !hooked {
                        state.pulling = false;
                    }
                }
            }
        }
    }
}

/// Checkpoint extension for grapple states.
struct ThreewaveGrappleExtension;

impl Q1StateExtension for ThreewaveGrappleExtension {
    fn id(&self) -> &str {
        "q1:equipment:threewave-grapple"
    }

    fn capture(&self, game: &Q1EntityServices) -> Vec<u8> {
        let states = game
            .threewave_grapple
            .as_ref()
            .map(|service| {
                service
                    .states
                    .iter()
                    .map(|(actor, state)| {
                        obj(vec![
                            (
                                "actor",
                                obj(vec![
                                    ("slot", int(i64::from(actor.slot()))),
                                    ("generation", int(i64::from(actor.generation()))),
                                ]),
                            ),
                            ("pulling", boolean(state.pulling)),
                            ("weaponFrame", num(f64::from(state.weapon_frame))),
                            ("attackFinished", num(state.attack_finished)),
                            ("releaseTime", num(state.release_time)),
                            (
                                "animation",
                                match state.animation.as_ref() {
                                    Some(animation) => obj(vec![
                                        ("slot", int(i64::from(animation.slot()))),
                                        ("generation", int(i64::from(animation.generation()))),
                                    ]),
                                    None => SaveJson::Null,
                                },
                            ),
                        ])
                    })
                    .collect()
            })
            .unwrap_or_default();
        encode_checkpoint_value(&arr(states))
    }

    fn restore(&mut self, game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
        let saved = decode_checkpoint_value(bytes)?;
        let reader = SaveReader::new(&saved);
        if game.threewave_grapple.is_none() {
            return Err(q1_error("Q1 threewave grapple was not registered"));
        }
        let mut states = HashMap::new();
        for entry in reader.list(|reader| restore_state(game, reader))? {
            states.insert(entry.0, entry.1);
        }
        if let Some(service) = game.threewave_grapple.as_mut() {
            service.states = states;
        }
        Ok(())
    }
}

fn restore_state(
    game: &mut Q1EntityServices,
    reader: SaveReader<'_>,
) -> Result<(ActorId, ThreewaveGrappleState), Q1Error> {
    let owner = reader.field("actor");
    let saved = SavedActorId {
        slot: u32::try_from(owner.field("slot").integer(0)?).map_err(|_| reader.fail("missing grapple owner"))?,
        generation: u32::try_from(owner.field("generation").integer(0)?)
            .map_err(|_| reader.fail("missing grapple owner"))?,
    };
    let actor = game
        .host
        .actors
        .resolve_saved(&saved)
        .ok_or_else(|| reader.fail("missing grapple owner"))?;
    let animation = reader.field("animation").nullable(|value| {
        Ok::<_, Q1Error>(game.host.actors.reference_saved(&SavedActorId {
            slot: u32::try_from(value.field("slot").integer(0)?).map_err(|_| value.fail("missing saved actor"))?,
            generation:
                u32::try_from(value.field("generation").integer(0)?).map_err(|_| value.fail("missing saved actor"))?,
        }))
    })?;
    Ok((
        actor.id().clone(),
        ThreewaveGrappleState {
            pulling: reader.field("pulling").boolean()?,
            weapon_frame: reader.field("weaponFrame").number()? as i32,
            attack_finished: reader.field("attackFinished").number()?,
            release_time: reader.field("releaseTime").number()?,
            animation,
        },
    ))
}

/// Register the grapple service, callbacks, release hook, and checkpoint extension.
pub fn register_threewave_grapple(
    game: &mut Q1EntityServices,
    host: Box<dyn ThreewaveGrappleHost>,
) -> Result<(), Q1Error> {
    game.threewave_grapple = Some(ThreewaveGrappleService {
        host,
        states: HashMap::new(),
    });
    game.named.register(
        "ctf:hook_touch",
        Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 surface: Option<&TouchSurface>| { grapple_touch(game, id, other, surface) },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:hook_pull",
        Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| grapple_pull(game, id)),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:hook_link",
        Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| position_link(game, id)),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:hook_flying",
        Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                let owner = entity.owner.clone();
                let expired = owner.as_ref().is_some_and(|owner| {
                    !game.host.actors.is_live(owner) || game.time >= entity.number("ctf.fired") + 5.0
                }) || owner.is_none();
                if expired {
                    return grapple_vanish(game, &id);
                }
                let owner = owner.expect("live owner");
                let service = require_service(game)?;
                if service.host.input(&owner).release {
                    return grapple_vanish(game, &id);
                }
                game.schedule(&id, 0.1, "ctf:hook_flying")
            }),
            ..Default::default()
        },
    )?;
    game.register_release_hook(Rc::new(RefCell::new(ThreewaveGrappleRelease)));
    game.register_state_extension(Box::new(ThreewaveGrappleExtension))
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::super::super::foundation::host::mock::{mock_host, MockEvents};
    use super::super::super::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};
    use super::*;

    struct TestHost;

    impl ThreewaveGrappleHost for TestHost {
        fn input(&self, _actor: &ActorId) -> ThreewaveGrappleInput {
            ThreewaveGrappleInput {
                held: true,
                release: false,
                jump: false,
                view_angles: ZERO,
                teleport_until: 0.0,
            }
        }

        fn aim(&self, _actor: &ActorId, forward: Vec3) -> Vec3 {
            forward
        }

        fn anchor(&self, _actor: &ActorId) -> ThreewaveAnchor {
            ThreewaveAnchor {
                solid: true,
                centered: false,
                player: false,
            }
        }

        fn can_attach(&self, _owner: &ActorId, _target: &ActorId) -> bool {
            true
        }

        fn can_pulse(&self, _owner: &ActorId, _target: &ActorId) -> bool {
            true
        }

        fn can_damage(&self, _target: &ActorId, _owner: &ActorId) -> bool {
            true
        }
    }

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    fn game() -> (Q1EntityServices, std::rc::Rc<std::cell::RefCell<MockEvents>>) {
        let (host, events) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        register_threewave_grapple(&mut game, Box::new(TestHost)).expect("register");
        (game, events)
    }

    #[test]
    fn fire_release_round_trip() {
        let (mut game, _) = game();
        let player = game.create("player", None, None).expect("player");
        game.set_health(&player, 100.0).expect("health");
        assert!(grapple_fire(&mut game, &player).expect("fire"));
        let hook = grapple_hook(&game, &player).expect("hook");
        assert_eq!(
            game.entity_ref(&hook).map(|entity| entity.classname.clone()),
            Some(String::from("ctf_hook"))
        );
        assert!(!grapple_fire(&mut game, &player).expect("second fire"));
        grapple_release(&mut game, &player).expect("release");
        assert!(grapple_hook(&game, &player).is_none());
        assert!(!grapple_pulling(&game, &player));
    }

    #[test]
    fn state_extension_captures_and_restores() {
        let (mut game, _) = game();
        let player = game.create("player", None, None).expect("player");
        grapple_state(&mut game, &player).expect("state").pulling = true;
        let bytes = game
            .state_extensions
            .get("q1:equipment:threewave-grapple")
            .expect("extension")
            .capture(&game);
        game.threewave_grapple.as_mut().expect("service").states.clear();
        assert!(!grapple_pulling(&game, &player));
        let mut extension = game
            .state_extensions
            .remove("q1:equipment:threewave-grapple")
            .expect("extension");
        extension.restore(&mut game, &bytes).expect("restore");
        game.state_extensions
            .insert(String::from("q1:equipment:threewave-grapple"), extension);
        assert!(grapple_pulling(&game, &player));
    }
}
