//! Q2 LMCTF grapple (`src/content/q2/equipment/lmctf-grapple.ts`).
//!
//! LM_CTF p_weapon.c hook and g_cmds.c offhand controls
//! (GPL-2.0-or-later).

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, length3, normalize3, scale3, sub3, vec3};

use crate::contract::{ArmorState, PoweredProtectionState, ProjectileRole, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2Die, Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent,
    Q2SoundLoop, Q2Think, Q2Touch, Q2TraceRequest,
};
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use crate::q2::support::contracts::{
    BodyAttachment, BodyFollow, CombatState, DeathReaction, TouchContact, TraceHit,
    WeaponBehaviorLaunch,
};

use super::grapple_services::{
    grapple_body, grapple_velocity, GrappleAnchor, GrappleCableEvent, GrappleHand, GrappleHooks,
    LmctfGrappleState,
};
use super::{lmctf_handle, lmctf_state_mut};

/// LMCTF grapple policy (`LmctfGrapplePolicy`).
#[derive(Debug, Clone, Copy)]
pub struct LmctfGrapplePolicy {
    /// Whether the hook may attach.
    pub can_attach: fn(ActorId, ActorId, &mut Q2GameServices) -> bool,
    /// Whether the hook may damage.
    pub can_damage: fn(ActorId) -> bool,
    /// Whether a hit counts as a player hit.
    pub player_hit: fn(ActorId, &mut Q2GameServices) -> bool,
}

/// Attach anywhere (`canAttach` default).
pub fn lmctf_can_attach(_owner: ActorId, _target: ActorId, _game: &mut Q2GameServices) -> bool {
    true
}

/// Damage anything (`canDamage` default).
pub fn lmctf_can_damage(_target: ActorId) -> bool {
    true
}

/// Player hits are players (`playerHit` default).
pub fn lmctf_player_hit(actor: ActorId, game: &mut Q2GameServices) -> bool {
    game.host.is_player(&actor)
}

/// Default LMCTF grapple policy.
pub fn default_lmctf_grapple_policy() -> LmctfGrapplePolicy {
    LmctfGrapplePolicy {
        can_attach: lmctf_can_attach,
        can_damage: lmctf_can_damage,
        player_hit: lmctf_player_hit,
    }
}

/// Ignore a release (`released` default).
pub fn lmctf_released(_actor: ActorId) {}

/// LMCTF grapple equipment (`LmctfGrappleEquipment`).
///
/// Copy handle over arena state; `bind` registers the hooks that the
/// hook callbacks and release fan-out resolve from the runtime.
#[derive(Debug, Clone, Copy)]
pub struct LmctfGrappleEquipment {
    /// Grapple hooks.
    pub hooks: GrappleHooks,
    /// Grapple policy.
    pub policy: LmctfGrapplePolicy,
    /// Release callback.
    pub released: fn(ActorId),
}

impl LmctfGrappleEquipment {
    /// Build equipment with the default policy.
    pub fn new(hooks: GrappleHooks) -> Self {
        LmctfGrappleEquipment {
            hooks,
            policy: default_lmctf_grapple_policy(),
            released: lmctf_released,
        }
    }

    /// Bind equipment to a game (`bind`).
    pub fn bind(&self, game: &mut Q2GameServices) {
        game.equipment.lmctf = Some(*self);
    }

    /// Read or create owner state (`state`).
    pub fn state_snapshot(&self, game: &mut Q2GameServices, actor: ActorId) -> LmctfGrappleState {
        lmctf_state_mut(game, actor).clone()
    }

    /// Abort a player's grapple (`abort`).
    pub fn abort(&self, player: ActorId, game: &mut Q2GameServices) {
        let body = game.host.bodies().read(&player);
        if let Some(body) = body {
            if body.ground.is_some() {
                let previous = (self.hooks.previous_velocity)(player.clone());
                (self.hooks.set_previous_velocity)(
                    player.clone(),
                    vec3(previous.x, previous.y, 0.0),
                );
                grapple_velocity(
                    player.clone(),
                    game,
                    vec3(body.velocity.x, body.velocity.y, 0.0),
                );
            }
        }
        (self.released)(player.clone());
        let hook = {
            let state = lmctf_state_mut(game, player.clone());
            state.hook_state = 0;
            state.hook_length = 0.0;
            state.hook.clone()
        };
        lmctf_state_mut(game, player).hook = None;
        if let Some(hook) = hook.and_then(|hook| {
            game.entity(&hook)
                .map(|entity| entity.actor.id().clone())
        }) {
            game.cancel_actor(hook.clone());
            game.require_entity_mut(&hook).enemy = None;
            let owned = game.owned_of(hook.clone());
            game.host.bodies().detach(&owned);
            game.remove_actor(hook);
        }
    }

    /// Touch a hook (`touch`).
    pub fn touch(&self, hook: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
        let owner = game.require_entity(&hook).owner.clone();
        let Some(owner) = owner else {
            game.remove_actor(hook);
            return;
        };
        if !game.host.actors().is_live(&owner) || game.host.bodies().read(&owner).is_none() {
            self.abort(owner, game);
            return;
        }
        if contact.other == owner {
            return;
        }
        if game
            .require_entity(&hook)
            .enemy
            .as_ref()
            .is_some_and(|enemy| *enemy != contact.other)
        {
            return;
        }
        let anchor = (self.hooks.anchor)(contact.other.clone(), game);
        let sky = contact
            .surface
            .as_ref()
            .map(|surface| surface.native_flags)
            .unwrap_or(0)
            & 4
            != 0;
        if anchor == GrappleAnchor::None
            || anchor == GrappleAnchor::Box
            || sky
            || !(self.policy.can_attach)(owner.clone(), contact.other.clone(), game)
            || (self.hooks.dead)(contact.other.clone(), game)
        {
            self.abort(owner, game);
            return;
        }
        let mut moved = game.body_of(hook.clone());
        moved.velocity = Vec3::default();
        game.write_body(hook.clone(), &moved, true);
        lmctf_state_mut(game, owner.clone()).hook_state = 2;
        if (self.policy.can_damage)(contact.other.clone()) {
            let frame = (game.host.now() * 10.0).round() as i32;
            let repeated = game
                .require_entity(&hook)
                .enemy
                .as_ref()
                == Some(&contact.other);
            if !repeated || frame % 7 == 0 && frame != game.require_entity(&hook).count {
                let amount = if repeated { 1.0 } else { 8.0 };
                if (self.policy.player_hit)(contact.other.clone(), game) {
                    game.sound(
                        &hook,
                        if repeated {
                            "weapons/grapple/gkilling.wav"
                        } else {
                            "weapons/grapple/ghit.wav"
                        },
                        0,
                        1.0,
                        1.0,
                    );
                } else if !repeated {
                    game.sound(&hook, "weapons/grapple/ghitwall.wav", 0, 0.8, 1.0);
                }
                if game
                    .host
                    .combat()
                    .read(&contact.other)
                    .is_some_and(|combat| combat.can_take_damage)
                {
                    let origin = game.body_of(hook.clone()).origin;
                    game.damage(
                        contact.other.clone(),
                        hook.clone(),
                        Some(owner.clone()),
                        amount,
                        amount,
                        Vec3::default(),
                        origin,
                        contact_normal(&contact),
                        60,
                        4,
                        Some("q2:weapon_hook".to_string()),
                    );
                    if !game.host.actors().is_live(&hook) || !game.host.actors().is_live(&owner) {
                        return;
                    }
                }
                if repeated {
                    game.require_entity_mut(&hook).count = frame;
                }
            }
        }
        if (self.hooks.dead)(contact.other.clone(), game) {
            self.abort(owner, game);
            return;
        }
        if game.require_entity(&hook).enemy.is_none() {
            let body = game.host.bodies().read(&contact.other);
            let Some(body) = body else {
                self.abort(owner, game);
                return;
            };
            let offset = sub3(
                game.body_of(hook.clone()).origin,
                add3(body.origin, body.bounds.min),
            );
            game.require_entity_mut(&hook).enemy = Some(contact.other.clone());
            game.require_entity_mut(&hook).pos1 = offset;
            game.set_solid(hook.clone(), Q2Solid::Trigger);
            let owned = game.owned_of(hook.clone());
            game.host.bodies().attach(
                &owned,
                &BodyAttachment {
                    anchor: contact.other.clone(),
                    follow: BodyFollow::BoundsMin { offset },
                },
            );
        }
        let origin = game.body_of(hook).origin;
        game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
            effect: "blaster".to_string(),
            origin,
            direction: contact_normal(&contact),
            count: 0,
            color: 0,
        }));
    }

    /// Launch a hook (`launch`).
    fn launch(
        &self,
        player: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
    ) -> ActorId {
        let hook = game.create("noclass", BTreeMap::new());
        let angles = vector_angles(direction);
        lmctf_state_mut(game, player.clone()).hook = Some(hook.clone());
        {
            let entity = game.require_entity_mut(&hook);
            entity.owner = Some(player.clone());
            entity.touch = Some(lmctf_hook_touch as Q2Touch);
            entity.die = Some(lmctf_hook_die as Q2Die);
            entity.damage = 2.0;
            entity.max_health = 59.0;
            entity.model = "models/objects/ghook/tris.md2".to_string();
            entity.clip_mask = 0x6000003;
        }
        let mut moved = game.body_of(hook.clone());
        moved.origin = start;
        moved.angles = vec3(angles.x + 90.0, angles.y, angles.z);
        moved.velocity = scale3(normalize3(direction), 800.0);
        game.write_body(hook.clone(), &moved, true);
        game.set_solid(hook.clone(), Q2Solid::Box);
        game.set_motion_kind(hook.clone(), Q2MotionKind::FlyMissile);
        let owned = game.owned_of(hook.clone());
        game.host.combat().create(
            &owned,
            &CombatState {
                health: 59.0,
                armor: ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
                mass: 0.0,
                can_take_damage: true,
                invulnerable: false,
                no_knockback: false,
                team: None,
            },
        );
        game.schedule(hook.clone(), 1.0, lmctf_hook_think as Q2Think);
        let origin = grapple_body(player.clone(), game).origin;
        game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(player.clone()),
            origin,
            path: "weapons/grapple/grfire.wav".to_string(),
            channel: 0,
            volume: 0.8,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
        let trajectory = launch_trajectory(game, hook.clone(), player.clone());
        if let Some(update) = trajectory.as_ref() {
            game.project_trajectory(hook.clone(), update);
        }
        let launch_origin = game.body_of(hook.clone()).origin;
        game.show(hook.clone());
        let trace_start = grapple_body(player.clone(), game).origin;
        let trace = game.host.trace(&Q2TraceRequest {
            start: trace_start,
            end: launch_origin,
            bounds: None,
            ignore: Some(player),
            mask: 0x6000003,
            exclude: Vec::new(),
        });
        if trace.fraction < 1.0 {
            let back = match trajectory.as_ref() {
                None => direction,
                Some(update) => normalize3(update.velocity),
            };
            let mut moved = game.body_of(hook.clone());
            moved.origin = add3(launch_origin, scale3(back, -10.0));
            game.write_body(hook.clone(), &moved, true);
            let other = match &trace.hit {
                TraceHit::Actor { actor } => actor.clone(),
                _ => game.host.world_actor(),
            };
            let owned = game.owned_of(hook.clone());
            self.touch(
                hook.clone(),
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
        hook
    }

    /// Fire or steer the hook (`fire`).
    pub fn fire(&self, player: ActorId, game: &mut Q2GameServices) {
        self.bind(game);
        let pose = (self.hooks.pose)(player.clone(), game);
        let body = grapple_body(player.clone(), game);
        let basis = angle_vectors(pose.angles);
        let side = match pose.hand {
            GrappleHand::Left => -8.0,
            GrappleHand::Center => 0.0,
            GrappleHand::Right => 8.0,
        };
        let start = add3(
            add3(
                add3(body.origin, scale3(basis.forward, 8.0)),
                scale3(basis.right, side),
            ),
            vec3(0.0, 0.0, pose.view_height as f32 - 8.0),
        );
        if lmctf_state_mut(game, player.clone()).hook_state == 0 {
            lmctf_state_mut(game, player.clone()).hook_state = 1;
            let hook = self.launch(player.clone(), game, start, basis.forward);
            lmctf_state_mut(game, player.clone()).hook = Some(hook.clone());
            if !game.host.actors().is_live(&hook) {
                let state = lmctf_state_mut(game, player.clone());
                state.hook = None;
                state.hook_state = 0;
                return;
            }
            // The donor draws the fresh cable twice.
            self.draw(player.clone(), start, game.body_of(hook.clone()).origin);
            self.draw(player, start, game.body_of(hook).origin);
            return;
        }
        let hooked = lmctf_state_mut(game, player.clone()).hook.clone();
        let hook =
            hooked.and_then(|hook| game.entity(&hook).map(|entity| entity.actor.id().clone()));
        let Some(hook) = hook else {
            lmctf_state_mut(game, player).hook_state = 0;
            return;
        };
        if lmctf_state_mut(game, player.clone()).hook_state == 1 {
            self.draw(player, start, game.body_of(hook).origin);
            return;
        }
        if let Some(enemy) = game.require_entity(&hook).enemy.clone() {
            if let Some(target) = game.host.bodies().read(&enemy) {
                let offset = game.require_entity(&hook).pos1;
                let mut moved = game.body_of(hook.clone());
                moved.origin = add3(add3(target.origin, target.bounds.min), offset);
                game.write_body(hook.clone(), &moved, true);
            }
        }
        let end = game.body_of(hook).origin;
        self.draw(player.clone(), start, end);
        let distance = length3(sub3(end, start)).trunc();
        lmctf_state_mut(game, player.clone()).hook_length = f64::from(distance);
        let speed = if distance > 120.0 {
            800.0
        } else if distance > 100.0 {
            distance * 5.0
        } else if distance > 80.0 {
            distance * 4.0
        } else if distance > 40.0 {
            distance * 3.0
        } else if distance > 20.0 {
            distance * 2.0
        } else if distance > 10.0 {
            distance
        } else {
            1.0
        };
        let velocity = scale3(normalize3(sub3(end, start)), speed);
        grapple_velocity(player.clone(), game, velocity);
        (self.hooks.set_previous_velocity)(player, velocity);
    }

    /// Draw the hook cable (`draw`).
    fn draw(&self, player: ActorId, start: Vec3, end: Vec3) {
        if length3(sub3(end, start)) > 64.0 {
            (self.hooks.emit)(GrappleCableEvent {
                actor: player,
                start,
                end,
                offset: Vec3::default(),
            });
        }
    }

    /// Read the gravity scale (`gravityScale`).
    pub fn gravity_scale(&self, game: &Q2GameServices, actor: ActorId) -> i32 {
        let pulls = game
            .equipment
            .lmctf_states
            .get(&actor)
            .is_some_and(|state| state.hook_state == 2 && state.hook_length < 50.0);
        if pulls { 0 } else { 1 }
    }
}

/// Release fan-out for an LMCTF owner or hook actor.
pub fn lmctf_actor_released(game: &mut Q2GameServices, actor: &ActorId) {
    let handle = lmctf_handle(game);
    if let Some(owned) = game.equipment.lmctf_states.remove(actor) {
        if let Some(hook) = owned
            .hook
            .and_then(|hook| game.entity(&hook).map(|entity| entity.actor.id().clone()))
        {
            if game.host.actors().is_live(&hook) {
                game.cancel_actor(hook.clone());
                game.remove_actor(hook);
            }
        }
    }
    let owners: Vec<ActorId> = game
        .equipment
        .lmctf_states
        .iter()
        .filter(|(_, state)| state.hook.as_ref() == Some(actor))
        .map(|(owner, _)| owner.clone())
        .collect();
    for owner in owners {
        let source = game
            .equipment
            .lmctf_states
            .get_mut(&owner)
            .expect("Q2 LMCTF grapple state is missing");
        source.hook = None;
        source.hook_state = 0;
        source.hook_length = 0.0;
        if game.host.actors().is_live(&owner) {
            (handle.released)(owner);
        }
    }
}

/// Read a contact plane normal or zero.
fn contact_normal(contact: &TouchContact) -> Vec3 {
    contact.plane.map(|plane| plane.normal).unwrap_or_default()
}

/// Launch a projectile through the weapon behavior port.
fn launch_trajectory(
    game: &mut Q2GameServices,
    projectile: ActorId,
    shooter: ActorId,
) -> Option<crate::q2::support::contracts::WeaponTrajectoryUpdate> {
    if !game.host.is_player(&shooter) {
        return None;
    }
    let owned = game.owned_of(projectile);
    let body = game.body_of(owned.id().clone());
    let input = WeaponBehaviorLaunch {
        projectile: owned,
        shooter,
        weapon: "q2:weapon_hook".to_string(),
        role: ProjectileRole::Grapple,
        time_seconds: game.host.now(),
        body,
    };
    match game.host.weapon_behavior() {
        Some(port) => port.launch(&input),
        None => None,
    }
}

/// LMCTF hook think callback (`lmctf:Grapple_Bolt_Think`).
fn lmctf_hook_think(hook: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&hook).owner.clone();
    let state = owner
        .as_ref()
        .and_then(|owner| game.equipment.lmctf_states.get(owner).cloned());
    let Some(state) = state else {
        game.remove_actor(hook);
        return;
    };
    if state.hook_length <= 126.0 {
        game.cancel_actor(hook);
        return;
    }
    let attached = game.require_entity(&hook).enemy.is_some();
    game.sound(
        &hook,
        if attached {
            "weapons/grapple/gpulling.wav"
        } else {
            "weapons/grapple/gflyair.wav"
        },
        0,
        1.0,
        1.0,
    );
    game.schedule(hook, if attached { 0.8 } else { 0.4 }, lmctf_hook_think as Q2Think);
}

/// LMCTF hook touch callback (`lmctf:hook_touch`).
fn lmctf_hook_touch(hook: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    lmctf_handle(game).touch(hook, game, contact);
}

/// LMCTF hook die callback (`lmctf:hook_die`).
fn lmctf_hook_die(hook: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    let handle = lmctf_handle(game);
    match game.require_entity(&hook).owner.clone() {
        None => game.remove_actor(hook),
        Some(owner) => handle.abort(owner, game),
    }
}

/// LMCTF grapple callbacks (`callbacks`).
pub fn lmctf_grapple_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("lmctf:Grapple_Bolt_Think", lmctf_hook_think as Q2Think);
    callbacks
        .touch
        .insert("lmctf:hook_touch", lmctf_hook_touch as Q2Touch);
    callbacks.die.insert("lmctf:hook_die", lmctf_hook_die as Q2Die);
    callbacks
}
