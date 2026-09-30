//! Q2 CTF grapple (`src/content/q2/equipment/ctf-grapple.ts`).
//!
//! Classic CTF 1.09b g_ctf.c and rerelease ctf/g_ctf.cpp grapple
//! (GPL-2.0-or-later).

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, length3, normalize3, scale3, sub3, vec3};

use crate::contract::{ArmorState, PoweredProtectionState, ProjectileRole, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2Die, Q2Edition, Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid,
    Q2SoundEvent, Q2SoundLoop, Q2Touch, Q2TraceRequest,
};
use crate::q2::foundation::weapons::projection::{project_q2_actor, q2_actor_shot_mask, Q2ActorView};
use crate::q2::foundation::weapons::types::WeaponHand;
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use qa_core::math::Plane;
use crate::q2::support::contracts::{
    AttackCause, BodyAttachment, BodyFollow, CombatState, CombatTraitChanges, DeathReaction,
    EnvironmentHazard, Q2BspPlane, TouchContact, TouchSurface, TraceFamily, TraceHit,
    TraceResult, WeaponBehaviorLaunch,
};

use super::grapple_services::{
    grapple_body, grapple_velocity, CtfGrapplePhase, CtfGrappleState, GrappleAnchor, GrappleCableEvent,
    GrappleHand, GrappleHooks, GrappleNoise,
};
use super::{ctf_handle, ctf_state_mut};

/// Grapple means of death.
const MOD_GRAPPLE: i32 = 34;

/// CTF grapple settings (`CtfGrappleSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CtfGrappleSettings {
    /// Fly speed.
    pub fly_speed: f64,
    /// Pull speed.
    pub pull_speed: f64,
    /// Damage.
    pub damage: f64,
    /// Whether players collide.
    pub players_collide: bool,
}

/// Default CTF grapple settings.
pub fn default_ctf_grapple_settings() -> CtfGrappleSettings {
    CtfGrappleSettings {
        fly_speed: 650.0,
        pull_speed: 650.0,
        damage: 10.0,
        players_collide: true,
    }
}

/// Whether a death reaction is a crush (`isCrush`).
fn is_crush(reaction: &DeathReaction) -> bool {
    let Some(attack) = reaction.attack() else {
        return false;
    };
    match &attack.cause {
        AttackCause::Q1 { death_type, .. } => death_type == "crush",
        AttackCause::Q2 { means_of_death, .. } => *means_of_death == 20,
        AttackCause::Q3 { means_of_death, .. } => *means_of_death == 17,
        AttackCause::Environment { hazard } => *hazard == EnvironmentHazard::Crush,
    }
}

/// CTF grapple equipment (`Q2CtfGrappleEquipment`).
///
/// Copy handle over arena state; `bind` registers the hooks that the
/// hook callbacks and release fan-out resolve from the runtime.
#[derive(Debug, Clone, Copy)]
pub struct Q2CtfGrappleEquipment {
    /// Grapple hooks.
    pub hooks: GrappleHooks,
    /// Damage eligibility.
    pub can_damage: fn(ActorId, ActorId, &mut Q2GameServices) -> bool,
    /// Settings provider.
    pub settings: fn(ActorId, &mut Q2GameServices) -> CtfGrappleSettings,
}

/// Allow any damage (`canDamage` default).
pub fn ctf_grapple_can_damage(_owner: ActorId, _target: ActorId, _game: &mut Q2GameServices) -> bool {
    true
}

/// Default settings provider.
pub fn ctf_grapple_settings(_actor: ActorId, _game: &mut Q2GameServices) -> CtfGrappleSettings {
    default_ctf_grapple_settings()
}

impl Q2CtfGrappleEquipment {
    /// Build equipment with default damage and settings.
    pub fn new(hooks: GrappleHooks) -> Self {
        Q2CtfGrappleEquipment {
            hooks,
            can_damage: ctf_grapple_can_damage,
            settings: ctf_grapple_settings,
        }
    }

    /// Bind equipment to a game (`bind`).
    pub fn bind(&self, game: &mut Q2GameServices) {
        game.equipment.ctf = Some(*self);
    }

    /// Read or create owner state (`state`).
    pub fn state_snapshot(&self, game: &mut Q2GameServices, actor: ActorId) -> CtfGrappleState {
        ctf_state_mut(game, actor).clone()
    }

    /// Play a grapple sound (`sound`).
    pub fn sound(
        &self,
        entity: ActorId,
        owner: ActorId,
        game: &mut Q2GameServices,
        file: &str,
        reliable: bool,
    ) {
        let origin = grapple_body(entity.clone(), game).origin;
        let volume = (self.hooks.volume)(owner, game);
        game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(entity),
            origin,
            path: format!("weapons/grapple/{file}.wav"),
            channel: 1,
            volume,
            attenuation: 1.0,
            reliable: game.options.edition == Q2Edition::Classic && reliable,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }

    /// Reset a player's grapple (`reset`).
    pub fn reset(&self, player: ActorId, game: &mut Q2GameServices) {
        let hook = game
            .equipment
            .ctf_states
            .get(&player)
            .and_then(|source| source.grapple.clone())
            .and_then(|grapple| {
                game.entity(&grapple)
                    .map(|entity| entity.actor.id().clone())
            });
        if let Some(hook) = hook {
            self.reset_hook(hook, game);
            return;
        }
        let Some(source) = game.equipment.ctf_states.get_mut(&player) else {
            return;
        };
        if source.grapple.is_none() {
            return;
        }
        source.grapple = None;
        source.grapple_state = CtfGrapplePhase::Fly;
        source.grapple_release_time =
            game.host.now() + if game.options.edition == Q2Edition::Rerelease { 1.0 } else { 0.0 };
        Self::restore_knockback(player.clone(), game);
        if game.options.edition == Q2Edition::Classic {
            (self.hooks.set_grapple_prediction)(player, false, game);
        }
    }

    /// Reset a hook (`resetHook`).
    pub fn reset_hook(&self, hook: ActorId, game: &mut Q2GameServices) {
        let owner = game.require_entity(&hook).owner.clone();
        let Some(owner) = owner else {
            game.remove_actor(hook);
            return;
        };
        let live = game
            .equipment
            .ctf_states
            .get(&owner)
            .is_some_and(|source| source.grapple.is_some());
        if !live {
            game.remove_actor(hook);
            return;
        }
        if game.host.actors().is_live(&owner) && game.host.bodies().read(&owner).is_some() {
            self.sound(owner.clone(), owner.clone(), game, "grreset", true);
        }
        let source = game
            .equipment
            .ctf_states
            .get_mut(&owner)
            .expect("Q2 CTF grapple state is missing");
        source.grapple = None;
        source.grapple_state = CtfGrapplePhase::Fly;
        source.grapple_release_time =
            game.host.now() + if game.options.edition == Q2Edition::Rerelease { 1.0 } else { 0.0 };
        Self::restore_knockback(owner.clone(), game);
        set_hook_loop(hook.clone(), game, "");
        if game.options.edition == Q2Edition::Classic {
            (self.hooks.set_grapple_prediction)(owner, false, game);
        }
        let owned = game.owned_of(hook.clone());
        game.host.bodies().detach(&owned);
        game.remove_actor(hook);
    }

    /// Restore saved knockback immunity (`restoreKnockback`).
    fn restore_knockback(owner: ActorId, game: &mut Q2GameServices) {
        let saved = game
            .equipment
            .ctf_states
            .get_mut(&owner)
            .and_then(|state| state.grapple_no_knockback.take());
        let Some(saved) = saved else {
            return;
        };
        if let Some(owned) = game.host.actors().resolve_owned(&owner) {
            if game.host.combat().read(&owner).is_some() {
                game.host.combat().set_traits(
                    &owned,
                    &CombatTraitChanges {
                        can_take_damage: None,
                        mass: None,
                        invulnerable: None,
                        team: None,
                        no_knockback: Some(saved),
                    },
                );
            }
        }
    }

    /// Touch a hook (`touch`).
    pub fn touch(&self, hook: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
        let owner = game.require_entity(&hook).owner.clone();
        let Some(owner) = owner else {
            self.reset_hook(hook, game);
            return;
        };
        if !game.host.actors().is_live(&owner) || game.host.bodies().read(&owner).is_none() {
            self.reset_hook(hook, game);
            return;
        }
        let source = ctf_state_mut(game, owner.clone()).clone();
        if contact.other == owner || source.grapple_state != CtfGrapplePhase::Fly {
            return;
        }
        if contact
            .surface
            .as_ref()
            .is_some_and(|surface| surface.native_flags & 4 != 0)
        {
            self.reset_hook(hook, game);
            return;
        }
        let mut moved = game.body_of(hook.clone());
        moved.velocity = Vec3::default();
        game.write_body(hook.clone(), &moved, true);
        let origin = game.body_of(hook.clone()).origin;
        (self.hooks.noise)(owner.clone(), game, origin, GrappleNoise::Impact);
        if game
            .host
            .combat()
            .read(&contact.other)
            .is_some_and(|combat| combat.can_take_damage)
        {
            let damage = game.require_entity(&hook).damage;
            if game.options.edition == Q2Edition::Classic || damage != 0.0 {
                let origin = game.body_of(hook.clone()).origin;
                game.damage(
                    contact.other.clone(),
                    hook.clone(),
                    Some(owner.clone()),
                    damage,
                    1.0,
                    Vec3::default(),
                    origin,
                    contact_normal(&contact),
                    MOD_GRAPPLE,
                    0,
                    Some("q2:weapon_grapple".to_string()),
                );
            }
            if !game.host.actors().is_live(&hook) || !game.host.actors().is_live(&owner) {
                return;
            }
            self.reset_hook(hook, game);
            return;
        }
        ctf_state_mut(game, owner.clone()).grapple_state = CtfGrapplePhase::Pull;
        game.require_entity_mut(&hook).enemy = Some(contact.other.clone());
        game.set_solid(hook.clone(), Q2Solid::None);
        let anchor = (self.hooks.anchor)(contact.other.clone(), game);
        let body = game.host.bodies().read(&contact.other);
        let Some(body) = body else {
            self.reset_hook(hook, game);
            return;
        };
        if anchor == GrappleAnchor::None {
            self.reset_hook(hook, game);
            return;
        }
        let follow = match anchor {
            GrappleAnchor::Box | GrappleAnchor::Player | GrappleAnchor::Corpse => BodyFollow::Center,
            _ => BodyFollow::Translation {
                offset: sub3(game.body_of(hook.clone()).origin, body.origin),
            },
        };
        let owned = game.owned_of(hook.clone());
        game.host.bodies().attach(
            &owned,
            &BodyAttachment {
                anchor: contact.other.clone(),
                follow,
            },
        );
        if game.options.edition == Q2Edition::Classic {
            self.sound(owner.clone(), owner.clone(), game, "grpull", true);
        }
        self.sound(hook.clone(), owner, game, "grhit", false);
        if game.options.edition == Q2Edition::Rerelease {
            set_hook_loop(hook.clone(), game, "weapons/grapple/grpull.wav");
        }
        let origin = game.body_of(hook).origin;
        game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
            effect: "sparks".to_string(),
            origin,
            direction: contact_normal(&contact),
            count: 0,
            color: 0,
        }));
    }

    /// Fire or release the offhand grapple (`offhand`).
    pub fn offhand(&self, player: ActorId, game: &mut Q2GameServices, pressed: bool) {
        if !pressed {
            self.reset(player, game);
            return;
        }
        if ctf_state_mut(game, player.clone()).grapple.is_some() {
            return;
        }
        self.fire_from_pose(player, game);
    }

    /// Fire from the owner pose (`fireFromPose`).
    pub fn fire_from_pose(&self, player: ActorId, game: &mut Q2GameServices) {
        if ctf_state_mut(game, player.clone()).grapple_state != CtfGrapplePhase::Fly {
            return;
        }
        let pose = (self.hooks.pose)(player.clone(), game);
        let settings = (self.settings)(player.clone(), game);
        let (start, direction) = project_q2_actor(
            &player,
            game,
            &Q2ActorView {
                hand: grapple_hand(pose.hand),
                view_height: pose.view_height,
                players_collide: settings.players_collide,
            },
            pose.angles,
            vec3(24.0, 8.0, -6.0),
        );
        if game.options.edition == Q2Edition::Classic {
            self.sound(player.clone(), player.clone(), game, "grfire", true);
        }
        let launched = self.fire_grapple(
            player.clone(),
            game,
            start,
            direction,
            if game.options.edition == Q2Edition::Classic {
                10.0
            } else {
                settings.damage
            },
            if game.options.edition == Q2Edition::Classic {
                650.0
            } else {
                settings.fly_speed
            },
            0,
        );
        if !game.host.actors().is_live(&player) {
            return;
        }
        if game.options.edition == Q2Edition::Rerelease && launched {
            self.sound(player.clone(), player.clone(), game, "grfire", false);
        }
        (self.hooks.noise)(player, game, start, GrappleNoise::Weapon);
    }

    /// Fire a grapple hook (`fireGrapple`).
    #[allow(clippy::too_many_arguments)]
    pub fn fire_grapple(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        effects: i64,
    ) -> bool {
        self.bind(game);
        if ctf_state_mut(game, owner.clone()).grapple.is_some() {
            return false;
        }
        let hook = game.create("grapple", BTreeMap::new());
        let normalized = normalize3(direction);
        let players_collide = (self.settings)(owner.clone(), game).players_collide;
        let mask = q2_actor_shot_mask(game, players_collide);
        game.require_entity_mut(&hook).clip_mask = mask;
        if game.options.edition == Q2Edition::Rerelease {
            game.require_entity_mut(&hook).flags |= 0x800 | 0x100000;
            game.require_entity_mut(&hook).die = Some(ctf_grapple_die as Q2Die);
            let owned = game.owned_of(hook.clone());
            game.host.combat().create(
                &owned,
                &CombatState {
                    health: 0.0,
                    armor: ArmorState {
                        regular: RegularArmorState::None,
                        powered: PoweredProtectionState::None,
                    },
                    mass: 0.0,
                    can_take_damage: true,
                    invulnerable: false,
                    no_knockback: true,
                    team: None,
                },
            );
        }
        {
            let entity = game.require_entity_mut(&hook);
            entity.projectile = true;
            entity.effects = effects;
            entity.model = "models/weapons/grapple/hook/tris.md2".to_string();
            entity.owner = Some(owner.clone());
            entity.touch = Some(ctf_grapple_touch as Q2Touch);
            entity.damage = damage;
        }
        let mut moved = game.body_of(hook.clone());
        moved.origin = start;
        moved.angles = vector_angles(normalized);
        moved.velocity = scale3(normalized, speed as f32);
        moved.bounds.min = Vec3::default();
        moved.bounds.max = Vec3::default();
        game.write_body(hook.clone(), &moved, false);
        let source = ctf_state_mut(game, owner.clone());
        source.grapple = Some(hook.clone());
        source.grapple_state = CtfGrapplePhase::Fly;
        game.set_solid(hook.clone(), Q2Solid::Box);
        game.set_motion_kind(hook.clone(), Q2MotionKind::FlyMissile);
        let trajectory = launch_trajectory(
            game,
            hook.clone(),
            owner.clone(),
            "q2:weapon_grapple".to_string(),
        );
        if let Some(update) = trajectory.as_ref() {
            game.project_trajectory(hook.clone(), update);
        }
        let launch_origin = game.body_of(hook.clone()).origin;
        game.show(hook.clone());
        let trace_start = grapple_body(owner.clone(), game).origin;
        let trace = game.host.trace(&Q2TraceRequest {
            start: trace_start,
            end: launch_origin,
            bounds: None,
            ignore: Some(hook.clone()),
            mask,
            exclude: Vec::new(),
        });
        if trace.fraction < 1.0 {
            let origin = if game.options.edition == Q2Edition::Classic {
                let back = match trajectory.as_ref() {
                    None => normalized,
                    Some(update) => normalize3(update.velocity),
                };
                add3(launch_origin, scale3(back, -10.0))
            } else {
                let normal = trace
                    .q2()
                    .map(|fields| fields.source_plane.normal)
                    .unwrap_or_default();
                add3(trace.end, normal)
            };
            let mut moved = game.body_of(hook.clone());
            moved.origin = origin;
            game.write_body(hook.clone(), &moved, true);
            let other = match &trace.hit {
                TraceHit::Actor { actor } => actor.clone(),
                _ => game.host.world_actor(),
            };
            let classic = game.options.edition == Q2Edition::Classic;
            let (plane, surface) = if classic {
                (None, None)
            } else {
                (trace_plane(&trace), trace_surface(&trace))
            };
            let owned = game.owned_of(hook.clone());
            self.touch(
                hook,
                game,
                TouchContact {
                    this: owned,
                    other,
                    plane,
                    surface,
                    source_trace: None,
                },
            );
            return false;
        }
        if game.options.edition == Q2Edition::Rerelease {
            set_hook_loop(hook, game, "weapons/grapple/grfly.wav");
        }
        true
    }

    /// Pull the owner toward the hook (`pull`).
    pub fn pull(&self, hook: ActorId, game: &mut Q2GameServices, damage_pulse: bool) {
        let owner = game.require_entity(&hook).owner.clone();
        let Some(owner) = owner else {
            self.reset_hook(hook, game);
            return;
        };
        if !game.host.actors().is_live(&owner) || game.host.bodies().read(&owner).is_none() {
            self.reset_hook(hook, game);
            return;
        }
        let pose = (self.hooks.pose)(owner.clone(), game);
        if let Some(enemy) = game.require_entity(&hook).enemy.clone() {
            let anchor = (self.hooks.anchor)(enemy.clone(), game);
            let body = game.host.bodies().read(&enemy);
            let Some(body) = body else {
                self.reset_hook(hook, game);
                return;
            };
            if anchor == GrappleAnchor::None {
                self.reset_hook(hook, game);
                return;
            }
            let mut moved = game.body_of(hook.clone());
            match anchor {
                GrappleAnchor::Box | GrappleAnchor::Player | GrappleAnchor::Corpse => {
                    moved.origin = add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5));
                }
                _ => {
                    moved.velocity = body.velocity;
                }
            }
            game.write_body(hook.clone(), &moved, true);
            if game.options.edition == Q2Edition::Classic
                && damage_pulse
                && game
                    .host
                    .combat()
                    .read(&enemy)
                    .is_some_and(|combat| combat.can_take_damage)
                && (self.can_damage)(owner.clone(), enemy.clone(), game)
            {
                let velocity = game.body_of(hook.clone()).velocity;
                let origin = game.body_of(hook.clone()).origin;
                game.damage(
                    enemy.clone(),
                    hook.clone(),
                    Some(owner.clone()),
                    1.0,
                    1.0,
                    velocity,
                    origin,
                    Vec3::default(),
                    MOD_GRAPPLE,
                    0,
                    Some("q2:weapon_grapple".to_string()),
                );
                if !game.host.actors().is_live(&hook) || !game.host.actors().is_live(&owner) {
                    return;
                }
                self.sound(hook.clone(), owner.clone(), game, "grhurt", false);
            }
            if (self.hooks.dead)(enemy, game) {
                self.reset_hook(hook, game);
                return;
            }
        }
        self.cable(hook.clone(), owner.clone(), game);
        let source = ctf_state_mut(game, owner.clone()).clone();
        if source.grapple_state == CtfGrapplePhase::Fly {
            return;
        }
        let body = grapple_body(owner.clone(), game);
        let mouth = add3(body.origin, vec3(0.0, 0.0, pose.view_height as f32));
        let direction = sub3(game.body_of(hook.clone()).origin, mouth);
        if source.grapple_state == CtfGrapplePhase::Pull && length3(direction) < 64.0 {
            ctf_state_mut(game, owner.clone()).grapple_state = CtfGrapplePhase::Hang;
            if game.options.edition == Q2Edition::Classic {
                (self.hooks.set_grapple_prediction)(owner.clone(), true, game);
                self.sound(owner.clone(), owner.clone(), game, "grhang", true);
            } else {
                set_hook_loop(hook.clone(), game, "weapons/grapple/grhang.wav");
            }
        }
        if game.options.edition == Q2Edition::Rerelease {
            let actor = game.host.actors().resolve_owned(&owner);
            let combat = game.host.combat().read(&owner);
            if let (Some(actor), Some(combat)) = (actor, combat) {
                let source = ctf_state_mut(game, owner.clone());
                if source.grapple_no_knockback.is_none() {
                    source.grapple_no_knockback = Some(combat.no_knockback);
                }
                game.host.combat().set_traits(
                    &actor,
                    &CombatTraitChanges {
                        can_take_damage: None,
                        mass: None,
                        invulnerable: None,
                        team: None,
                        no_knockback: Some(true),
                    },
                );
            }
        }
        let speed = if game.options.edition == Q2Edition::Classic {
            650.0
        } else {
            (self.settings)(owner.clone(), game).pull_speed
        };
        let pull = scale3(normalize3(direction), speed as f32);
        let gravity = (pose.gravity * (self.hooks.gravity)(game) * game.host.frame_seconds()) as f32;
        grapple_velocity(owner, game, add3(pull, scale3(pose.gravity_vector, gravity)));
    }

    /// Draw the grapple cable (`cable`).
    fn cable(&self, hook: ActorId, owner: ActorId, game: &mut Q2GameServices) {
        let origin = grapple_body(owner.clone(), game).origin;
        let end = game.body_of(hook).origin;
        let pose = (self.hooks.pose)(owner.clone(), game);
        if game.options.edition == Q2Edition::Rerelease {
            if ctf_state_mut(game, owner.clone()).grapple_state == CtfGrapplePhase::Hang {
                return;
            }
            let settings = (self.settings)(owner.clone(), game);
            let (start, _) = project_q2_actor(
                &owner,
                game,
                &Q2ActorView {
                    hand: grapple_hand(pose.hand),
                    view_height: pose.view_height,
                    players_collide: settings.players_collide,
                },
                pose.angles,
                vec3(7.0, 2.0, -9.0),
            );
            (self.hooks.emit)(
                GrappleCableEvent {
                    actor: owner,
                    start,
                    end,
                    offset: Vec3::default(),
                },
                game,
            );
            return;
        }
        let axes = angle_vectors(pose.angles);
        let side = match pose.hand {
            GrappleHand::Left => -16.0,
            GrappleHand::Center => 0.0,
            GrappleHand::Right => 16.0,
        };
        let start = add3(
            add3(
                add3(origin, scale3(axes.forward, 16.0)),
                scale3(axes.right, side),
            ),
            vec3(0.0, 0.0, pose.view_height as f32 - 8.0),
        );
        if length3(sub3(start, end)) < 64.0 {
            return;
        }
        (self.hooks.emit)(
            GrappleCableEvent {
                actor: owner,
                start: origin,
                end,
                offset: sub3(start, origin),
            },
            game,
        );
    }

    /// Run the player frame (`playerFrame`).
    pub fn player_frame(&self, player: ActorId, game: &mut Q2GameServices, damage_pulse: bool) {
        let hook = game
            .equipment
            .ctf_states
            .get(&player)
            .and_then(|state| state.grapple.clone())
            .and_then(|grapple| {
                game.entity(&grapple)
                    .map(|entity| entity.actor.id().clone())
            });
        let Some(hook) = hook else {
            return;
        };
        self.pull(hook, game, damage_pulse);
    }
}

/// Release fan-out for a CTF owner or hook actor.
pub fn ctf_actor_released(game: &mut Q2GameServices, actor: &ActorId) {
    let handle = ctf_handle(game);
    if let Some(owned) = game.equipment.ctf_states.remove(actor) {
        if let Some(hook) = owned
            .grapple
            .and_then(|grapple| game.entity(&grapple).map(|entity| entity.actor.id().clone()))
        {
            if game.host.actors().is_live(&hook) {
                set_hook_loop(hook.clone(), game, "");
                game.cancel_actor(hook.clone());
                game.remove_actor(hook);
            }
        }
    }
    let owners: Vec<ActorId> = game
        .equipment
        .ctf_states
        .iter()
        .filter(|(_, state)| state.grapple.as_ref() == Some(actor))
        .map(|(owner, _)| owner.clone())
        .collect();
    for owner in owners {
        let rerelease = game.options.edition == Q2Edition::Rerelease;
        let now = game.host.now();
        let source = game
            .equipment
            .ctf_states
            .get_mut(&owner)
            .expect("Q2 CTF grapple state is missing");
        source.grapple = None;
        source.grapple_state = CtfGrapplePhase::Fly;
        source.grapple_release_time = now + if rerelease { 1.0 } else { 0.0 };
        Q2CtfGrappleEquipment::restore_knockback(owner.clone(), game);
        if !rerelease && game.host.actors().is_live(&owner) {
            (handle.hooks.set_grapple_prediction)(owner, false, game);
        }
    }
}

/// Start or stop a hook loop sound (`loop`).
fn set_hook_loop(hook: ActorId, game: &mut Q2GameServices, path: &str) {
    if game.require_entity(&hook).sound == path {
        return;
    }
    let origin = game.body_of(hook.clone()).origin;
    let current = game.require_entity(&hook).sound.clone();
    if !current.is_empty() {
        game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(hook.clone()),
            origin,
            path: current,
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Stop,
            loop_owner: None,
        }));
    }
    game.require_entity_mut(&hook).sound = path.to_string();
    if !path.is_empty() {
        game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(hook),
            origin,
            path: path.to_string(),
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Start,
            loop_owner: None,
        }));
    }
}

/// Map a grapple hand to a weapon hand.
fn grapple_hand(hand: GrappleHand) -> WeaponHand {
    match hand {
        GrappleHand::Left => WeaponHand::Left,
        GrappleHand::Center => WeaponHand::Center,
        GrappleHand::Right => WeaponHand::Right,
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
    weapon: String,
) -> Option<crate::q2::support::contracts::WeaponTrajectoryUpdate> {
    if !game.host.is_player(&shooter) {
        return None;
    }
    let owned = game.owned_of(projectile);
    let body = game.body_of(owned.id().clone());
    let input = WeaponBehaviorLaunch {
        projectile: owned,
        shooter,
        weapon,
        role: ProjectileRole::Grapple,
        time_seconds: game.host.now(),
        body,
    };
    match game.host.weapon_behavior() {
        Some(port) => port.launch(&input),
        None => None,
    }
}

/// Convert a trace source plane (`fireGrapple` contact plane).
fn trace_plane(trace: &TraceResult) -> Option<Plane> {
    match &trace.family {
        TraceFamily::Q1 { source_plane, .. } => Some(*source_plane),
        TraceFamily::Q2(fields) => Some(bsp_plane(&fields.source_plane)),
        TraceFamily::Q3 { source_plane, .. } => Some(bsp_plane(source_plane)),
    }
}

/// Convert a Q2 BSP plane to a contact plane.
fn bsp_plane(plane: &Q2BspPlane) -> Plane {
    Plane {
        normal: plane.normal,
        distance: plane.distance,
    }
}

/// Convert a trace surface (`fireGrapple` contact surface).
fn trace_surface(trace: &TraceResult) -> Option<TouchSurface> {
    match &trace.family {
        TraceFamily::Q2(fields) => fields.surface.as_ref().map(|surface| TouchSurface {
            name: surface.name.clone(),
            native_flags: surface.flags,
            native_value: surface.value,
        }),
        TraceFamily::Q1 { surface_flags, .. } => Some(TouchSurface {
            name: String::new(),
            native_flags: surface_flags.unwrap_or(0),
            native_value: 0,
        }),
        TraceFamily::Q3 { surface_flags, .. } => Some(TouchSurface {
            name: String::new(),
            native_flags: *surface_flags,
            native_value: 0,
        }),
    }
}

/// CTF grapple touch callback (`CTFGrappleTouch`).
fn ctf_grapple_touch(hook: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    ctf_handle(game).touch(hook, game, contact);
}

/// CTF grapple die callback (`grapple_die`).
fn ctf_grapple_die(hook: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    if is_crush(&reaction) {
        ctf_handle(game).reset_hook(hook, game);
    }
}

/// CTF grapple callbacks (`callbacks`).
pub fn ctf_grapple_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .touch
        .insert("CTFGrappleTouch", ctf_grapple_touch as Q2Touch);
    callbacks.die.insert("grapple_die", ctf_grapple_die as Q2Die);
    callbacks
}
