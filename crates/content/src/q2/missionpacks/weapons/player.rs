//! Mission-pack player weapons (`src/content/q2/missionpacks/weapons/player.ts`).
//!
//! Original Xatrix/Rogue p_weapon.c source callbacks over the shared
//! Weapon_Generic.

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};

use crate::q2::foundation::host::{
    Q2Edition, Q2EffectEvent, Q2GameServices, Q2Mode, Q2PresentationEvent, Q2Solid, Q2TraceRequest,
};
use crate::q2::foundation::weapons::ballistics::{weapon_player_noise, NoiseKind};
use crate::q2::foundation::weapons::player::{
    animate_player, attack_animation, register_weapon_extension, set_fallback_order, weapon_ammo, weapon_consume,
    weapon_consume_infinite, weapon_continues_attack, weapon_emit, weapon_firing_interval, weapon_flash,
    weapon_generic_classic, weapon_generic_rerelease, weapon_kick, weapon_lag_begin, weapon_lag_end, weapon_multiplier,
    weapon_no_ammo, weapon_powerup_sound, weapon_project, weapon_set_loop, weapon_throw_classic,
    weapon_throw_rerelease, Q2ThrowDefinition, Q2WeaponContext, Q2WeaponExtension, Q2WeaponSelectionRule,
};
use crate::q2::foundation::weapons::types::{
    PlayerAnimationPriority, PrimaryHandoff, Q2WeaponDefinition, Q2WeaponEvent, Q2WeaponName, Q2WeaponOwner,
    Q2WeaponPhase, Q2WeaponState, WeaponHand,
};
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::{TraceContact, TraceHit};

use super::super::projectiles::common::velocity;
use super::super::projectiles::{mission_hooks, mission_projectiles, Q2MissionPackProjectiles};
use super::super::types::{Q2MissionPack, Q2_MISSION_PACK_DAMAGE};
use super::definitions::{rogue_weapon_definitions, xatrix_weapon_definitions};

/// Clamp a point into bounds (`closest`).
fn closest(point: Vec3, min: Vec3, max: Vec3) -> Vec3 {
    vec3(
        point.x.max(min.x).min(max.x),
        point.y.max(min.y).min(max.y),
        point.z.max(min.z).min(max.z),
    )
}

/// Ionripper selection (`choose`, classic Xatrix over the hyperblaster).
fn choose_ionripper(owner: &Q2WeaponOwner, game: &mut Q2GameServices, state: &Q2WeaponState) -> bool {
    if game
        .host
        .inventory()
        .count(owner.actor.id(), &"q2:weapon_boomer".to_string())
        == 0.0
    {
        return false;
    }
    state.weapon.as_deref() == Some("hyperblaster")
}

/// Phalanx selection (`choose`, classic Xatrix over the railgun).
fn choose_phalanx(owner: &Q2WeaponOwner, game: &mut Q2GameServices, state: &Q2WeaponState) -> bool {
    if game
        .host
        .inventory()
        .count(owner.actor.id(), &"q2:weapon_phalanx".to_string())
        == 0.0
    {
        return false;
    }
    if game
        .host
        .inventory()
        .count(owner.actor.id(), &"q2:ammo_slugs".to_string())
        == 0.0
    {
        return game
            .host
            .inventory()
            .count(owner.actor.id(), &"q2:ammo_magslug".to_string())
            > 0.0;
    }
    state.weapon.as_deref() == Some("railgun")
}

/// Throw entry for the shared throw drivers (`throwing.fire`).
fn throw_fire(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState, held: bool) {
    let projectiles = mission_projectiles(game);
    throw_mission_pack_weapon(&projectiles, context, game, state, held);
}

/// Throw a trap or tesla (`throw`).
fn throw_mission_pack_weapon(
    projectiles: &Q2MissionPackProjectiles,
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    held: bool,
) {
    let owner = context.owner.actor.id().clone();
    let trap = context.definition.name == "trap";
    if context.rerelease {
        let timer = state.grenade_time - context.now;
        let duration: f64 = if trap { 5.0 } else { 3.0 };
        let minimum: f64 = if trap { 300.0 } else { 400.0 };
        let maximum: f64 = if trap { 700.0 } else { 800.0 };
        let health = game
            .host
            .combat()
            .read(&owner)
            .map(|combat| combat.health)
            .unwrap_or(0.0);
        let speed = if health <= 0.0 {
            minimum
        } else {
            (minimum + (duration - timer) * (maximum - minimum) / duration).min(maximum)
        }
        .trunc();
        let mut angles = context.input.angles;
        angles.x = (-62.5f32).max(angles.x);
        let (start, direction) = weapon_project(
            context,
            game,
            if trap {
                vec3(8.0, 0.0, -8.0)
            } else {
                vec3(0.0, 0.0, -22.0)
            },
            Some(angles),
        );
        state.grenade_time = 0.0;
        let multiplier = weapon_multiplier(context, game);
        if trap {
            projectiles.fire_trap(
                owner,
                game,
                start,
                direction,
                125.0 * multiplier,
                speed,
                1.0,
                165.0,
                held,
            );
        } else {
            projectiles.fire_tesla(owner, game, start, direction, multiplier, speed);
        }
        weapon_consume_infinite(context, game, state, 1.0, false);
        return;
    }
    let timer = state.grenade_time - context.now;
    let charged: f64 = 400.0 + (3.0 - timer) * 400.0 / 3.0;
    let speed = (if trap { charged } else { charged.min(800.0) }).trunc();
    let axes = angle_vectors(context.input.angles);
    let (start, direction) = weapon_project(context, game, vec3(8.0, 8.0, -8.0), None);
    let multiplier = weapon_multiplier(context, game);
    if trap {
        projectiles.fire_trap(
            owner.clone(),
            game,
            start,
            direction,
            125.0 * multiplier,
            speed,
            timer,
            165.0,
            held,
        );
    } else {
        let side = if context.input.hand == WeaponHand::Left {
            4.0
        } else if context.input.hand == WeaponHand::Center {
            0.0
        } else {
            -4.0
        };
        let origin = game.body_of(owner.clone()).origin;
        let start = add3(
            add3(origin, scale3(axes.right, side)),
            scale3(axes.up, (context.owner.view_height - 22.0) as f32),
        );
        projectiles.fire_tesla(owner.clone(), game, start, axes.forward, multiplier, speed);
    }
    weapon_consume_infinite(context, game, state, 1.0, !trap);
    let interval = weapon_firing_interval(game, &owner, 1.0);
    state.grenade_time = context.now + interval;
    if trap && weapon_ammo(context, game) == 0.0 && !held {
        weapon_no_ammo(context, game, state, false);
    }
    if !trap {
        animate_player(
            context,
            game,
            if context.input.ducked {
                PlayerAnimationPriority::Attack
            } else {
                PlayerAnimationPriority::Reverse
            },
            if context.input.ducked { 159 } else { 119 },
            if context.input.ducked { 162 } else { 112 },
        );
    }
}

/// Mission-pack weapon extension (one per registered definition).
pub struct Q2MissionPackWeaponExtension {
    projectiles: Q2MissionPackProjectiles,
    definition: Q2WeaponDefinition,
    selection: Option<Q2WeaponSelectionRule>,
}

impl Q2MissionPackWeaponExtension {
    /// Fire the prox launcher (`prox`).
    fn prox(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let owner = context.owner.actor.id().clone();
        let mut angles = context.input.angles;
        if context.rerelease {
            angles.x = (-62.5f32).max(angles.x);
        }
        let (start, direction) = weapon_project(
            context,
            game,
            vec3(8.0, if context.rerelease { 0.0 } else { 8.0 }, -8.0),
            if context.rerelease {
                Some(angles)
            } else {
                Some(context.input.angles)
            },
        );
        weapon_kick(
            context,
            game,
            state,
            scale3(angle_vectors(context.input.angles).forward, -2.0),
            vec3(-1.0, 0.0, 0.0),
        );
        let multiplier = weapon_multiplier(context, game);
        self.projectiles
            .fire_prox(owner, game, start, direction, multiplier, 600.0);
        weapon_flash(context, game, if context.rerelease { 31 } else { 6 });
        if !context.rerelease {
            state.frame += 1;
        }
        let owner = context.owner.actor.id().clone();
        weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        weapon_consume(context, game, state);
    }

    /// Fire the ionripper (`ion`).
    fn ion(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let owner = context.owner.actor.id().clone();
        let mut aim = context.input.angles;
        aim.y += (game.host.random() * 2.0 - 1.0) as f32;
        let (start, direction) = weapon_project(context, game, vec3(16.0, 7.0, -8.0), Some(aim));
        weapon_kick(
            context,
            game,
            state,
            scale3(
                angle_vectors(if context.rerelease { context.input.angles } else { aim }).forward,
                -3.0,
            ),
            vec3(-3.0, 0.0, 0.0),
        );
        let multiplier = weapon_multiplier(context, game);
        let damage = (if game.options.mode == Q2Mode::Deathmatch {
            30.0
        } else {
            50.0
        }) * multiplier;
        self.projectiles
            .fire_ion_ripper(owner, game, start, direction, damage, 500.0, 0x100000);
        weapon_flash(context, game, 16);
        if !context.rerelease {
            state.frame += 1;
        }
        let owner = context.owner.actor.id().clone();
        weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        weapon_consume(context, game, state);
    }

    /// Fire the phalanx (`phalanx`).
    fn phalanx(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let owner = context.owner.actor.id().clone();
        let second = state.frame == 8;
        let multiplier = weapon_multiplier(context, game);
        let damage = (70.0 + (game.host.random() * 10.0).floor()) * multiplier;
        let mut angles = context.input.angles;
        angles.y += if second { -1.5 } else { 1.5 };
        let (start, projected) = weapon_project(
            context,
            game,
            vec3(0.0, 8.0, -8.0),
            Some(if context.rerelease {
                angles
            } else {
                context.input.angles
            }),
        );
        let direction = if context.rerelease {
            projected
        } else {
            angle_vectors(angles).forward
        };
        weapon_kick(
            context,
            game,
            state,
            scale3(angle_vectors(context.input.angles).forward, -2.0),
            vec3(-2.0, 0.0, 0.0),
        );
        self.projectiles.fire_plasma(
            owner.clone(),
            game,
            start,
            direction,
            damage,
            725.0,
            120.0,
            if second { 30.0 } else { 120.0 * multiplier },
        );
        if second {
            weapon_consume(context, game, state);
            if context.rerelease {
                weapon_flash(context, game, 20);
            }
        } else {
            weapon_flash(context, game, 18);
            weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        }
        if !context.rerelease {
            state.frame += 1;
        }
    }

    /// Fire the ETF rifle (`etf`).
    fn etf(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let owner = context.owner.actor.id().clone();
        if context.rerelease {
            if !weapon_continues_attack(context, state) {
                state.frame = 8;
                return;
            }
            state.frame = if state.frame == 6 { 7 } else { 6 };
        }
        if weapon_ammo(context, game) < f64::from(context.definition.quantity) {
            weapon_kick(context, game, state, Vec3::default(), Vec3::default());
            state.frame = 8;
            weapon_no_ammo(context, game, state, true);
            return;
        }
        let kick_ox = ((game.host.random() * 2.0 - 1.0) * 0.85) as f32;
        let kick_ax = ((game.host.random() * 2.0 - 1.0) * 0.85) as f32;
        let kick_oy = ((game.host.random() * 2.0 - 1.0) * 0.85) as f32;
        let kick_ay = ((game.host.random() * 2.0 - 1.0) * 0.85) as f32;
        let kick_oz = ((game.host.random() * 2.0 - 1.0) * 0.85) as f32;
        let kick_az = ((game.host.random() * 2.0 - 1.0) * 0.85) as f32;
        let kick_origin = vec3(kick_ox, kick_oy, kick_oz);
        let kick_angles = vec3(kick_ax, kick_ay, kick_az);
        weapon_kick(context, game, state, kick_origin, kick_angles);
        let axes = angle_vectors(context.input.angles);
        let side = (if state.frame == 6 { 8.0 } else { 6.0 })
            * (if context.input.hand == WeaponHand::Left {
                -1.0
            } else if context.input.hand == WeaponHand::Center {
                0.0
            } else {
                1.0
            });
        let (start, direction) = if context.rerelease {
            weapon_project(
                context,
                game,
                vec3(15.0, if state.frame == 6 { 8.0 } else { 6.0 }, -8.0),
                Some(add3(context.input.angles, kick_angles)),
            )
        } else {
            let origin = game.body_of(owner.clone()).origin;
            let start = add3(
                add3(
                    add3(
                        add3(origin, vec3(0.0, 0.0, context.owner.view_height as f32)),
                        scale3(axes.forward, 15.0),
                    ),
                    scale3(axes.right, side as f32),
                ),
                scale3(axes.up, -8.0),
            );
            (start, axes.forward)
        };
        let multiplier = weapon_multiplier(context, game);
        self.projectiles.fire_flechette(
            owner.clone(),
            game,
            start,
            direction,
            10.0 * multiplier,
            if context.rerelease { 1150.0 } else { 750.0 },
            3.0 * multiplier,
        );
        if context.rerelease {
            weapon_powerup_sound(context, game);
        }
        weapon_flash(
            context,
            game,
            if context.rerelease && state.frame == 7 { 32 } else { 30 },
        );
        weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        if !context.rerelease {
            state.frame += 1;
        }
        weapon_consume_infinite(
            context,
            game,
            state,
            f64::from(context.definition.quantity),
            context.rerelease,
        );
        attack_animation(context, game, 1);
    }

    /// Fire the disintegrator (`tracker`).
    fn tracker(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let owner = context.owner.actor.id().clone();
        let (start, direction) = weapon_project(context, game, vec3(24.0, 8.0, -8.0), None);
        let end = add3(start, scale3(direction, 8192.0));
        let mut mask = if context.rerelease { 0x46004003 } else { 0x6000003 };
        if context.rerelease && !context.input.players_collide {
            mask &= !0x40000000;
        }
        let token = if context.rerelease {
            weapon_lag_begin(game, &owner, start, direction)
        } else {
            None
        };
        let mut trace = game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(owner.clone()),
            mask,
            exclude: Vec::new(),
        });
        weapon_lag_end(game, token);
        let world = game.host.world_actor();
        let retrace = !matches!(trace.hit, TraceHit::Actor { .. })
            || matches!(&trace.hit, TraceHit::Actor { actor } if *actor == world);
        if retrace {
            trace = game.host.trace(&Q2TraceRequest {
                start,
                end,
                bounds: Some(Bounds {
                    min: vec3(-16.0, -16.0, -16.0),
                    max: vec3(16.0, 16.0, 16.0),
                }),
                ignore: Some(owner.clone()),
                mask,
                exclude: Vec::new(),
            });
        }
        let actor = match &trace.hit {
            TraceHit::Actor { actor } => Some(actor.clone()),
            _ => None,
        };
        let enemy = actor.filter(|actor| {
            (game.host.is_monster(actor)
                || game.host.is_player(actor)
                || game.entity(actor).is_some_and(|entity| entity.damageable_target))
                && game
                    .host
                    .combat()
                    .read(actor)
                    .map(|combat| combat.health)
                    .unwrap_or(0.0)
                    > 0.0
        });
        weapon_kick(
            context,
            game,
            state,
            scale3(angle_vectors(context.input.angles).forward, -2.0),
            vec3(-1.0, 0.0, 0.0),
        );
        let multiplier = weapon_multiplier(context, game);
        let damage = (if context.rerelease {
            if game.options.mode == Q2Mode::Deathmatch {
                45.0
            } else {
                135.0
            }
        } else if game.options.mode == Q2Mode::Deathmatch {
            30.0
        } else {
            45.0
        }) * multiplier;
        self.projectiles
            .fire_tracker(owner.clone(), game, start, direction, damage, 1000.0, enemy);
        weapon_emit(
            game,
            &Q2WeaponEvent::Muzzleflash {
                actor: owner.clone(),
                flash: 35,
                silenced: context.rerelease && context.silenced,
            },
        );
        weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        if !context.rerelease {
            state.frame += 1;
        }
        weapon_consume_infinite(
            context,
            game,
            state,
            f64::from(context.definition.quantity),
            context.rerelease,
        );
    }

    /// Fire the heatbeam (`heat`).
    fn heat(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let owner = context.owner.actor.id().clone();
        let (start, direction) = weapon_project(context, game, vec3(7.0, 2.0, -3.0), None);
        if context.rerelease {
            if !weapon_continues_attack(context, state) || weapon_ammo(context, game) < 2.0 {
                state.frame = 13;
                state.view_skin = 0;
                weapon_set_loop(context, game, state, "");
                if weapon_continues_attack(context, state) {
                    weapon_no_ammo(context, game, state, true);
                }
                return;
            }
            state.frame = if state.frame > 12 || state.frame == 11 {
                8
            } else {
                state.frame + 1
            };
            state.view_skin = 1;
            weapon_set_loop(context, game, state, "weapons/bfg__l1a.wav");
            weapon_powerup_sound(context, game);
        } else {
            state.frame += 1;
            state.view_model = Some("models/weapons/v_beamer2/tris.md2".to_string());
        }
        weapon_kick(context, game, state, Vec3::default(), Vec3::default());
        let token = if context.rerelease {
            weapon_lag_begin(game, &owner, start, direction)
        } else {
            None
        };
        let multiplier = weapon_multiplier(context, game);
        let deathmatch = game.options.mode == Q2Mode::Deathmatch;
        self.projectiles.fire_heat_beam(
            owner.clone(),
            game,
            start,
            direction,
            vec3(2.0, 7.0, -3.0),
            15.0 * multiplier,
            (if deathmatch { 75.0 } else { 30.0 }) * multiplier,
        );
        weapon_lag_end(game, token);
        weapon_flash(context, game, 33);
        weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        weapon_consume(context, game, state);
        attack_animation(context, game, 1);
    }

    /// Fire the rerelease chainfist (`chainfistRerelease`).
    fn chainfist_rerelease(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        let owner = context.owner.actor.id().clone();
        if !weapon_continues_attack(context, state) && (state.frame == 13 || state.frame == 23 || state.frame >= 32) {
            state.frame = 33;
            return;
        }
        let (start, direction) = weapon_project(context, game, vec3(0.0, 0.0, -4.0), None);
        let own = game.body_of(owner.clone());
        let own_min = add3(own.origin, own.bounds.min);
        let own_max = add3(own.origin, own.bounds.max);
        let actors: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        let mut count = 0;
        let mut hit = false;
        for actor in actors {
            if actor == owner
                || !game
                    .host
                    .combat()
                    .read(&actor)
                    .is_some_and(|combat| combat.can_take_damage)
            {
                continue;
            }
            let Some(body) = game.host.bodies().read(&actor) else {
                continue;
            };
            let target = game.entity(&actor).map(|entity| entity.actor.id().clone());
            if target
                .as_ref()
                .is_some_and(|target| matches!(game.require_entity(target).solid, Q2Solid::None | Q2Solid::Trigger))
            {
                continue;
            }
            let min = add3(body.origin, body.bounds.min);
            let max = add3(body.origin, body.bounds.max);
            if min.x > own_max.x + 23.0
                || max.x < own_min.x - 23.0
                || min.y > own_max.y + 23.0
                || max.y < own_min.y - 23.0
                || min.z > own_max.z + 23.0
                || max.z < own_min.z - 23.0
            {
                continue;
            }
            let point = closest(start, min, max);
            let near = closest(point, own_min, own_max);
            if f64::from(length3(sub3(point, near))) > 24.0 {
                continue;
            }
            let intersect = min.x + 2.0 <= own_max.x - 2.0
                && max.x - 2.0 >= own_min.x + 2.0
                && min.y + 2.0 <= own_max.y - 2.0
                && max.y - 2.0 >= own_min.y + 2.0
                && min.z + 2.0 <= own_max.z - 2.0
                && max.z - 2.0 >= own_min.z + 2.0;
            if !intersect && dot3(normalize3(sub3(scale3(add3(min, max), 0.5), start)), direction) < 0.7 {
                continue;
            }
            count += 1;
            if count > 4 {
                break;
            }
            let visible = match target {
                None => game.can_damage(&actor, &owner),
                Some(ref target) => game.can_damage(&owner, target),
            };
            if !visible {
                continue;
            }
            let monster_hook = mission_hooks(game).monster;
            if let Some(mut monster) = monster_hook(actor.clone(), game) {
                let jitter = monster.game.host.random();
                monster.state_mut().pain_time -= 0.005 + jitter * 0.07;
            }
            let multiplier = weapon_multiplier(context, game);
            game.damage(
                actor,
                owner.clone(),
                Some(owner.clone()),
                (if game.options.mode == Q2Mode::Deathmatch {
                    15.0
                } else {
                    7.0
                }) * multiplier,
                50.0,
                direction,
                point,
                scale3(direction, -1.0),
                Q2_MISSION_PACK_DAMAGE.chainfist,
                64 | 8,
                Some("q2:weapon_chainfist".to_string()),
            );
            hit = true;
        }
        if hit && state.empty_sound_time < context.now {
            state.empty_sound_time = context.now + 0.5;
            game.sound(&owner, "weapons/sawslice.wav", 1, 1.0, 1.0);
        }
        weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        state.frame += 1;
        if weapon_continues_attack(context, state) {
            if state.frame == 12 {
                state.frame = 14;
            } else if state.frame == 22 {
                state.frame = 24;
            } else if state.frame >= 32 {
                state.frame = 7;
            }
        }
        attack_animation(context, game, 1);
    }

    /// Fire the chainfist (`chainfist`).
    fn chainfist(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        if context.rerelease {
            self.chainfist_rerelease(context, game, state);
            return;
        }
        let owner = context.owner.actor.id().clone();
        let (start, direction) = weapon_project(context, game, vec3(0.0, 8.0, -4.0), None);
        let axes = angle_vectors(context.input.angles);
        weapon_kick(context, game, state, scale3(axes.forward, -2.0), vec3(-1.0, 0.0, 0.0));
        let trace = game.host.trace(&Q2TraceRequest {
            start,
            end: add3(start, scale3(direction, 64.0)),
            bounds: None,
            ignore: Some(owner.clone()),
            mask: 0x6000003,
            exclude: Vec::new(),
        });
        if trace.fraction < 1.0 {
            let target = match &trace.hit {
                TraceHit::Actor { actor } => Some(actor.clone()),
                _ => None,
            };
            if target.as_ref().is_some_and(|target| {
                game.host
                    .combat()
                    .read(target)
                    .is_some_and(|combat| combat.can_take_damage)
            }) {
                let target = target.expect("chainfist target is missing");
                let body = game.body_of(owner.clone());
                velocity(
                    game,
                    &owner,
                    add3(add3(body.velocity, scale3(axes.forward, 75.0)), scale3(axes.up, 75.0)),
                    false,
                );
                let multiplier = weapon_multiplier(context, game);
                let point = game
                    .host
                    .bodies()
                    .read(&target)
                    .map(|body| body.origin)
                    .unwrap_or(trace.end);
                game.damage(
                    target,
                    owner.clone(),
                    Some(owner.clone()),
                    (if game.options.mode == Q2Mode::Deathmatch {
                        30.0
                    } else {
                        15.0
                    }) * multiplier,
                    50.0,
                    Vec3::default(),
                    point,
                    Vec3::default(),
                    Q2_MISSION_PACK_DAMAGE.chainfist,
                    64 | 8,
                    Some("q2:weapon_chainfist".to_string()),
                );
            } else {
                let direction = match &trace.contact {
                    TraceContact::Plane { plane } => plane.normal,
                    _ => Vec3::default(),
                };
                game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                    effect: "q2:gunshot".to_string(),
                    origin: trace.end,
                    direction,
                    count: 0,
                    color: 0,
                }));
            }
        }
        weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        if !context.rerelease {
            state.frame += 1;
        }
    }

    /// Step the extension frame (`think`).
    fn think_frame(&self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        if context.definition.name == "trap" || context.definition.name == "tesla" {
            let trap = context.definition.name == "trap";
            if !trap && !context.rerelease {
                state.view_model = if state.frame > 1 && state.frame < 9 {
                    Some("models/weapons/v_tesla2/tris.md2".to_string())
                } else {
                    None
                };
            }
            let throwing = Q2ThrowDefinition {
                sound_frame: if trap { 5 } else { 99 },
                hold_frame: if trap { 11 } else { 1 },
                fire_frame: if trap { 12 } else { 2 },
                cock_sound: "weapons/trapcock.wav".to_string(),
                hold_sound: if trap {
                    "weapons/traploop.wav".to_string()
                } else {
                    String::new()
                },
                explode: trap && !context.rerelease,
                wrap_before_pause: !trap,
                release_held: !trap,
                fire: throw_fire,
            };
            if context.rerelease {
                weapon_throw_rerelease(context, game, state, Some(&throwing));
            } else {
                weapon_throw_classic(context, game, state, Some(&throwing));
            }
            return;
        }
        if context.rerelease {
            weapon_generic_rerelease(context, game, state);
            if state.primary_handoff == PrimaryHandoff::Holstered {
                return;
            }
            if context.definition.name == "chainfist" {
                if (state.frame == 42 || state.frame == 51)
                    && (game.host.random() * 8.0).floor() as i32 != 0
                    && context.input.hand != WeaponHand::Center
                    && game.host.random() < 0.4
                {
                    let (start, _) = weapon_project(context, game, vec3(8.0, 8.0, -4.0), None);
                    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                        effect: "q2:chainfist_smoke".to_string(),
                        origin: start,
                        direction: Vec3::default(),
                        count: 0,
                        color: 0,
                    }));
                }
                weapon_set_loop(
                    context,
                    game,
                    state,
                    if state.phase == Q2WeaponPhase::Firing {
                        "weapons/sawhit.wav"
                    } else if state.phase == Q2WeaponPhase::Dropping {
                        ""
                    } else {
                        "weapons/sawidle.wav"
                    },
                );
            }
            return;
        }
        let mut last_sequence = 0;
        if context.definition.name == "chainfist" {
            if state.frame == 13 || state.frame == 23 {
                state.frame = 32;
            } else if (state.frame == 42 || state.frame == 51)
                && (game.host.random() * 8.0).floor() as i32 != 0
                && context.input.hand != WeaponHand::Center
                && game.host.random() < 0.4
            {
                let (start, _) = weapon_project(context, game, vec3(8.0, 8.0, -4.0), None);
                game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                    effect: "q2:chainfist_smoke".to_string(),
                    origin: start,
                    direction: Vec3::default(),
                    count: 0,
                    color: 0,
                }));
            }
            weapon_set_loop(
                context,
                game,
                state,
                if state.phase == Q2WeaponPhase::Firing {
                    "weapons/sawhit.wav"
                } else if state.phase == Q2WeaponPhase::Dropping {
                    ""
                } else {
                    "weapons/sawidle.wav"
                },
            );
        } else if context.definition.name == "etf_rifle"
            && state.phase == Q2WeaponPhase::Firing
            && weapon_ammo(context, game) <= 0.0
        {
            state.frame = 8;
        } else if context.definition.name == "heatbeam" {
            if state.phase == Q2WeaponPhase::Firing {
                weapon_set_loop(context, game, state, "weapons/bfg__l1a.wav");
                if weapon_ammo(context, game) >= 2.0 && weapon_continues_attack(context, state) {
                    if state.frame >= 13 {
                        state.frame = 9;
                    }
                    state.view_model = Some("models/weapons/v_beamer2/tris.md2".to_string());
                } else {
                    state.frame = 13;
                    state.view_model = None;
                }
            } else {
                state.view_model = None;
                weapon_set_loop(context, game, state, "");
            }
        }
        if context.rerelease {
            weapon_generic_rerelease(context, game, state);
        } else {
            weapon_generic_classic(context, game, state);
        }
        if state.primary_handoff == PrimaryHandoff::Holstered {
            return;
        }
        if context.definition.name == "etf_rifle" && state.frame == 8 && weapon_continues_attack(context, state) {
            state.frame = 6;
        }
        if context.definition.name == "chainfist" {
            if weapon_continues_attack(context, state) && (state.frame == 13 || state.frame == 23 || state.frame == 32)
            {
                last_sequence = state.frame;
                state.frame = 6;
            }
            if state.frame == 6 {
                let mut chance = game.host.random();
                if last_sequence == 13 {
                    chance -= 0.34;
                } else if last_sequence == 23 {
                    chance += 0.33;
                } else if last_sequence == 32 && chance >= 0.33 {
                    chance += 0.34;
                }
                if chance < 0.33 {
                    state.frame = 14;
                } else if chance < 0.66 {
                    state.frame = 24;
                }
            }
        }
    }
}

impl Q2WeaponExtension for Q2MissionPackWeaponExtension {
    fn definition(&self) -> &Q2WeaponDefinition {
        &self.definition
    }

    fn fire(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        match context.definition.name.as_str() {
            "ionripper" => self.ion(context, game, state),
            "phalanx" => self.phalanx(context, game, state),
            "etf_rifle" => self.etf(context, game, state),
            "disintegrator" => self.tracker(context, game, state),
            "heatbeam" => self.heat(context, game, state),
            "chainfist" => self.chainfist(context, game, state),
            "proxlauncher" => self.prox(context, game, state),
            name => panic!("No mission-pack fire callback for {name}"),
        }
    }

    fn think(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) -> bool {
        self.think_frame(context, game, state);
        true
    }

    fn held(
        &mut self,
        context: &Q2WeaponContext,
        game: &mut Q2GameServices,
        state: &mut Q2WeaponState,
        held: bool,
    ) -> bool {
        throw_mission_pack_weapon(&self.projectiles, context, game, state, held);
        true
    }

    fn selection(&self) -> Option<Q2WeaponSelectionRule> {
        self.selection.clone()
    }

    fn has_held(&self) -> bool {
        self.definition.name == "trap" || self.definition.name == "tesla"
    }
}

/// Mission-pack weapons (`Q2MissionPackWeapons`).
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackWeapons {
    /// Projectile driver.
    pub projectiles: Q2MissionPackProjectiles,
}

impl Q2MissionPackWeapons {
    /// Build the weapons module.
    pub fn new(projectiles: Q2MissionPackProjectiles) -> Self {
        Self { projectiles }
    }

    /// Register the pack weapons (`register`).
    pub fn register(&self, game: &mut Q2GameServices, pack: Q2MissionPack, edition: Q2Edition) {
        let originals = match pack {
            Q2MissionPack::Xatrix => xatrix_weapon_definitions(),
            Q2MissionPack::Rogue => rogue_weapon_definitions(),
        };
        for original in originals {
            let mut definition = original;
            if edition == Q2Edition::Rerelease {
                if definition.name == "ionripper" {
                    definition.activate_last = 5;
                    definition.fire_last = 7;
                    definition.fires = vec![6];
                } else if definition.name == "heatbeam" {
                    definition.repeating = true;
                    definition.idle_last = 42;
                    definition.deactivate_last = 47;
                } else if definition.name == "chainfist" || definition.name == "etf_rifle" {
                    definition.repeating = true;
                }
            }
            let selection =
                if edition == Q2Edition::Classic && pack == Q2MissionPack::Xatrix && definition.name == "ionripper" {
                    Some(Q2WeaponSelectionRule {
                        requested: "hyperblaster".to_string(),
                        choose: choose_ionripper,
                    })
                } else if edition == Q2Edition::Classic && pack == Q2MissionPack::Xatrix && definition.name == "phalanx"
                {
                    Some(Q2WeaponSelectionRule {
                        requested: "railgun".to_string(),
                        choose: choose_phalanx,
                    })
                } else {
                    None
                };
            register_weapon_extension(
                game,
                Box::new(Q2MissionPackWeaponExtension {
                    projectiles: self.projectiles,
                    definition,
                    selection,
                }),
            );
        }
        if pack == Q2MissionPack::Rogue {
            set_fallback_order(
                game,
                [
                    "railgun",
                    "heatbeam",
                    "etf_rifle",
                    "chaingun",
                    "machinegun",
                    "supershotgun",
                    "shotgun",
                    "blaster",
                ]
                .into_iter()
                .map(Q2WeaponName::from)
                .collect(),
            );
        }
    }
}
