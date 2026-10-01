//! Monster and player ballistics (`src/content/q2/foundation/weapons/ballistics.ts`).
//!
//! Adapted from id Software's Quake II `g_weapon.c` and rerelease
//! `g_weapon.cpp` (GPL-2.0-or-later). All damage is admitted by the
//! session combat authority.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, Vec3};

use super::super::callbacks::{free_q2_entity, Q2CallbackDefinitions};
use super::super::checkpoint::restore_q2_actor;
use super::super::host::{Q2Edition, Q2GameServices, Q2Mode, Q2MotionKind, Q2Solid, Q2TraceRequest};
use super::super::monsters::perception::report_noise;
use super::projection::q2_actor_shot_mask;
use super::types::{
    Mod, Q2GrenadeAdjustment, Q2NoiseRecord, Q2WeaponEvent, WeaponBeamEffect, PLAYER_CONTENTS, WATER_MASK,
};
use super::vectors::{angle_vectors, lerp_angle, vector_angles};
use crate::contract::ItemId;
use crate::q2::support::contracts::{Q2TraceFields, TouchContact, TraceContact, TraceFamily, TraceHit, TraceResult};

/// Contact normal or zero (`normal`).
fn trace_normal(trace: &TraceResult) -> Vec3 {
    match &trace.contact {
        TraceContact::Plane { plane } => plane.normal,
        TraceContact::None => vec3(0.0, 0.0, 0.0),
    }
}

/// Trace contents (`contents`).
fn trace_contents(trace: &TraceResult) -> i32 {
    match &trace.family {
        TraceFamily::Q1 { .. } => 0,
        TraceFamily::Q2(fields) => fields.contents,
        TraceFamily::Q3 { contents, .. } => *contents,
    }
}

/// Whether a trace hit sky (`sky`).
fn trace_sky(trace: &TraceResult) -> bool {
    match &trace.family {
        TraceFamily::Q2(Q2TraceFields { surface, .. }) => surface
            .as_ref()
            .is_some_and(|surface| (surface.flags & 4) != 0 || surface.name.starts_with("sky")),
        TraceFamily::Q3 { surface_flags, .. } => (surface_flags & 4) != 0,
        TraceFamily::Q1 { .. } => false,
    }
}

/// Trace actor hit (`hit`).
fn trace_hit(trace: &TraceResult) -> Option<ActorId> {
    match &trace.hit {
        TraceHit::Actor { actor } => Some(actor.clone()),
        _ => None,
    }
}

/// Whether an actor can be hurt (`canHurt`).
fn can_hurt(game: &mut Q2GameServices, actor: Option<&ActorId>) -> bool {
    let Some(actor) = actor else { return false };
    game.host
        .combat()
        .read(actor)
        .is_some_and(|state| state.can_take_damage)
}

/// Body centroid (`centroid`).
fn centroid(game: &mut Q2GameServices, actor: &ActorId) -> Option<Vec3> {
    let body = game.host.bodies().read(actor)?;
    Some(add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5)))
}

/// Emit an effect (`effect`).
fn emit_effect(game: &mut Q2GameServices, name: &str, origin: Vec3, direction: Vec3, count: i32, color: i32) {
    game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::Effect(
        crate::q2::foundation::host::Q2EffectEvent {
            effect: name.to_string(),
            origin,
            direction,
            count,
            color,
        },
    ));
}

/// Weapon item for a cause (`weaponForMod`).
fn weapon_for_mod(means_of_death: i32) -> Option<ItemId> {
    match means_of_death {
        Mod::BLASTER => Some("q2:weapon_blaster".to_string()),
        Mod::SHOTGUN => Some("q2:weapon_shotgun".to_string()),
        Mod::SUPERSHOTGUN => Some("q2:weapon_supershotgun".to_string()),
        Mod::MACHINEGUN => Some("q2:weapon_machinegun".to_string()),
        Mod::CHAINGUN => Some("q2:weapon_chaingun".to_string()),
        Mod::HYPERBLASTER => Some("q2:weapon_hyperblaster".to_string()),
        _ => None,
    }
}

/// Hand grenade launch spec (`Q2HandGrenadeLaunch`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2HandGrenadeLaunch {
    /// Launch origin.
    pub start: Vec3,
    /// Launch direction.
    pub direction: Vec3,
    /// Damage.
    pub damage: f64,
    /// Speed.
    pub speed: f64,
    /// Fuse timer.
    pub timer: f64,
    /// Damage radius.
    pub radius: f64,
    /// Whether held.
    pub held: bool,
    /// Gravity.
    pub gravity: f64,
    /// Whether players collide.
    pub players_collide: bool,
}

/// Named ballistics callbacks (`Q2Ballistics["callbacks"]`).
pub fn q2_ballistics_callbacks() -> Q2CallbackDefinitions {
    Q2CallbackDefinitions {
        trajectory: Vec::new(),
        think: HashMap::from([
            ("G_FreeEdict", free_q2_entity as crate::q2::foundation::host::Q2Think),
            (
                "Grenade_Explode",
                grenade_explode as crate::q2::foundation::host::Q2Think,
            ),
            ("grenade_think", grenade_update as crate::q2::foundation::host::Q2Think),
            ("bfg_think", bfg_think as crate::q2::foundation::host::Q2Think),
            ("bfg_explode", bfg_explode as crate::q2::foundation::host::Q2Think),
        ]),
        use_: HashMap::new(),
        touch: HashMap::from([
            ("blaster_touch", blaster_touch as crate::q2::foundation::host::Q2Touch),
            ("Grenade_Touch", grenade_touch as crate::q2::foundation::host::Q2Touch),
            ("rocket_touch", rocket_touch as crate::q2::foundation::host::Q2Touch),
            ("bfg_touch", bfg_touch as crate::q2::foundation::host::Q2Touch),
        ]),
        pain: HashMap::new(),
        die: HashMap::from([(
            "q2_weapon_debris_die",
            crate::q2::foundation::entity_services::free_q2_entity_die as crate::q2::foundation::host::Q2Die,
        )]),
        blocked: HashMap::new(),
    }
}

/// Register ballistics callbacks (`registerCallbacks`).
pub fn register_ballistics_callbacks(game: &mut Q2GameServices) {
    game.source_callbacks.register(&q2_ballistics_callbacks());
}

/// Capture projectile state (`captureProjectiles`).
pub fn capture_projectiles(game: &Q2GameServices) -> Vec<super::checkpoint::Q2BlasterCauseEntry> {
    let mut entries: Vec<super::checkpoint::Q2BlasterCauseEntry> = game
        .weapons
        .blaster_causes
        .iter()
        .map(|(actor, means_of_death)| super::checkpoint::Q2BlasterCauseEntry {
            actor: super::super::checkpoint::save_q2_actor(Some(actor)).expect("live actor"),
            means_of_death: *means_of_death,
        })
        .collect();
    entries.sort_by_key(|entry| (entry.actor.slot, entry.actor.generation));
    entries
}

/// Restore projectile state (`restoreProjectiles`).
pub fn restore_projectiles(game: &mut Q2GameServices, checkpoint: &[super::checkpoint::Q2BlasterCauseEntry]) {
    register_ballistics_callbacks(game);
    game.weapons.blaster_causes.clear();
    for saved in checkpoint {
        let owner = restore_q2_actor(game, saved.actor);
        game.weapons
            .blaster_causes
            .insert(owner.id().clone(), saved.means_of_death);
    }
}

/// Silencer charges (`silencerShots`).
pub fn silencer_shots(game: &Q2GameServices, actor: &ActorId) -> i64 {
    game.weapons.silencer_charges.get(actor).copied().unwrap_or(0)
}

/// Grant silencer charges (`grantSilencer`).
pub fn grant_silencer(game: &mut Q2GameServices, actor: ActorId, charges: i64) {
    if !game.host.actors().is_live(&actor) {
        panic!("Silencer owner is not live");
    }
    if charges < 0 {
        panic!("Silencer charges must be a nonnegative integer");
    }
    register_ballistics_callbacks(game);
    let total = silencer_shots(game, &actor) + charges;
    game.weapons.silencer_charges.insert(actor, total);
}

/// Reset silencer charges (`resetSilencer`).
pub fn reset_silencer(game: &mut Q2GameServices, actor: &ActorId) {
    game.weapons.silencer_charges.remove(actor);
}

/// Shot mask for an owner (`shotMask`).
fn shot_mask(game: &mut Q2GameServices, owner: &ActorId) -> i32 {
    let players_collide = game
        .weapons
        .inputs
        .get(owner)
        .map(|input| input.players_collide)
        .unwrap_or(true);
    q2_actor_shot_mask(game, players_collide)
}

/// Player noise kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseKind {
    /// Self noise.
    Myself,
    /// Weapon noise.
    Weapon,
    /// Impact noise.
    Impact,
}

/// Report player noise (`playerNoise`).
pub(crate) fn player_noise(game: &mut Q2GameServices, owner: &ActorId, origin: Vec3, kind: NoiseKind) {
    player_noise_for_actor(game, owner.clone(), origin, kind);
}

/// Report player noise (`playerNoise`, shared with mission packs).
pub fn weapon_player_noise(game: &mut Q2GameServices, owner: &ActorId, origin: Vec3, kind: NoiseKind) {
    player_noise(game, owner, origin, kind);
}

/// Report player noise for an owner (`playerNoiseForActor`, shared with mission packs).
pub fn weapon_player_noise_for_actor(game: &mut Q2GameServices, owner: ActorId, origin: Vec3, kind: NoiseKind) {
    player_noise_for_actor(game, owner, origin, kind);
}

/// Report player noise for an owner (`playerNoiseForActor`).
pub(crate) fn player_noise_for_actor(game: &mut Q2GameServices, owner: ActorId, origin: Vec3, kind: NoiseKind) {
    if !game.host.actors().is_live(&owner) || !game.host.is_player(&owner) {
        return;
    }
    let charges = silencer_shots(game, &owner);
    if kind == NoiseKind::Weapon {
        if game.options.edition == Q2Edition::Rerelease {
            let until = game.host.now() + if charges > 0 { 0.4 } else { 2.0 };
            weapon_emit(
                game,
                &Q2WeaponEvent::InvisibilityReveal {
                    actor: owner.clone(),
                    until,
                },
            );
        }
        if charges > 0 {
            game.weapons.silencer_charges.insert(owner, charges - 1);
            return;
        }
    }
    if game.options.mode == Q2Mode::Deathmatch
        || game.weapons.inputs.get(&owner).is_some_and(|input| input.notarget)
        || game
            .monster_target(Some(&owner))
            .is_some_and(|observed| observed.notarget)
    {
        return;
    }
    let record = Q2NoiseRecord {
        actor: owner.clone(),
        origin,
        time: game.host.now(),
        secondary: kind == NoiseKind::Impact,
    };
    let mut records = game.weapons.noises.get(&owner).cloned().unwrap_or(super::WeaponNoises {
        primary: None,
        secondary: None,
    });
    if kind == NoiseKind::Impact {
        records.secondary = Some(record.clone());
        game.weapons.sound2_entity = Some(record);
    } else {
        records.primary = Some(record.clone());
        game.weapons.sound_entity = Some(record);
    }
    game.weapons.noises.insert(owner.clone(), records);
    report_noise(game, owner, origin, kind == NoiseKind::Impact);
}

/// Report projectile impact noise (`impactNoise`).
fn impact_noise(game: &mut Q2GameServices, projectile: &ActorId) {
    let owner = game.require_entity(projectile).owner.clone();
    let Some(owner) = owner else { return };
    let origin = game.body_of(projectile.clone()).origin;
    player_noise_for_actor(game, owner, origin, NoiseKind::Impact);
}

/// Emit a weapon event through the session engine (`hooks.emit`).
fn weapon_emit(game: &mut Q2GameServices, event: &Q2WeaponEvent) {
    let mut engine = game.weapons.engine.take();
    if let Some(engine) = engine.as_mut() {
        engine.emit(event);
    }
    game.weapons.engine = engine;
}

/// Check BFG target eligibility (`hooks.canTarget`).
fn can_target(game: &mut Q2GameServices, attacker: Option<&ActorId>, target: &ActorId) -> bool {
    let mut engine = game.weapons.engine.take();
    let eligible = engine
        .as_mut()
        .map(|engine| engine.can_target(attacker, target))
        .unwrap_or(true);
    game.weapons.engine = engine;
    eligible
}

/// Check classic dodge (`checkDodge`).
pub fn check_dodge(game: &mut Q2GameServices, owner: &ActorId, start: Vec3, direction: Vec3, speed: f64) {
    if game.options.edition != Q2Edition::Classic || !game.host.is_player(owner) {
        return;
    }
    if game.options.skill == 0 && game.random() > 0.25 {
        return;
    }
    let mask = shot_mask(game, owner);
    let trace = game.host.trace(&Q2TraceRequest {
        start,
        end: add3(start, scale3(direction, 8192.0)),
        bounds: None,
        ignore: Some(owner.clone()),
        mask,
        exclude: Vec::new(),
    });
    // Donor `hit(trace)` borrows the trace while the game is borrowed
    // for combat reads; resolve the actor first.
    let target = trace_hit(&trace);
    let target_health = target
        .as_ref()
        .map(|target| game.host.combat().read(target).map(|state| state.health).unwrap_or(0.0));
    let Some(target) = target else { return };
    if !game.host.is_monster(&target) || target_health.unwrap_or(0.0) <= 0.0 {
        return;
    }
    let Some(body) = game.host.bodies().read(&target) else {
        return;
    };
    let self_origin = game.body_of(owner.clone()).origin;
    if f64::from(dot3(
        normalize3(sub3(self_origin, body.origin)),
        angle_vectors(body.angles).forward,
    )) <= 0.3
    {
        return;
    }
    let eta = (f64::from(length3(sub3(trace.end, start))) - f64::from(body.bounds.max.x)) / speed;
    let mut context = super::super::monsters::types::MonsterContext::new(target, game);
    context.dodge(owner.clone(), eta, Some(trace), false);
}

/// Fire a bullet (`fireBullet`).
#[allow(clippy::too_many_arguments)]
pub fn fire_bullet(
    owner: ActorId,
    game: &mut Q2GameServices,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    kick: f64,
    horizontal_spread: f64,
    vertical_spread: f64,
    means_of_death: i32,
) {
    fire_lead(
        game,
        &owner,
        start,
        direction,
        damage,
        kick,
        horizontal_spread,
        vertical_spread,
        means_of_death,
        "gunshot",
    );
}

/// Fire shotgun pellets (`fireShotgun`).
#[allow(clippy::too_many_arguments)]
pub fn fire_shotgun(
    owner: ActorId,
    game: &mut Q2GameServices,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    kick: f64,
    horizontal_spread: f64,
    vertical_spread: f64,
    count: i32,
    means_of_death: i32,
) {
    for _ in 0..count {
        fire_lead(
            game,
            &owner,
            start,
            direction,
            damage,
            kick,
            horizontal_spread,
            vertical_spread,
            means_of_death,
            "shotgun",
        );
    }
}

/// Spread a hitscan endpoint.
fn lead_spread(
    game: &mut Q2GameServices,
    origin: Vec3,
    aim: Vec3,
    factor: f64,
    horizontal_spread: f64,
    vertical_spread: f64,
) -> Vec3 {
    let axes = angle_vectors(vector_angles(aim));
    let r = (game.random() * 2.0 - 1.0) * horizontal_spread * factor;
    let u = (game.random() * 2.0 - 1.0) * vertical_spread * factor;
    add3(
        add3(add3(origin, scale3(axes.forward, 8192.0)), scale3(axes.right, r as f32)),
        scale3(axes.up, u as f32),
    )
}

/// Fire one hitscan pellet (`fireLead`).
#[allow(clippy::too_many_arguments)]
fn fire_lead(
    game: &mut Q2GameServices,
    owner: &ActorId,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    kick: f64,
    horizontal_spread: f64,
    vertical_spread: f64,
    means_of_death: i32,
    impact: &str,
) {
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    let shot = shot_mask(game, owner);
    let mut water_start = if (game.host.point_contents(start) & WATER_MASK) != 0 {
        Some(start)
    } else {
        None
    };
    let mut mask = shot | if water_start.is_none() { WATER_MASK } else { 0 };
    let mut from = game.body_of(owner.clone()).origin;
    let mut end = start;
    let mut excluded: Vec<ActorId> = Vec::new();
    let mut initial = true;
    let trace = loop {
        let trace = game.host.trace(&Q2TraceRequest {
            start: from,
            end,
            bounds: None,
            ignore: Some(owner.clone()),
            mask: if initial && !rerelease { shot } else { mask },
            exclude: excluded.clone(),
        });
        if trace.fraction == 1.0 && initial {
            initial = false;
            excluded.clear();
            from = start;
            end = lead_spread(game, start, direction, 1.0, horizontal_spread, vertical_spread);
            continue;
        }
        if trace.fraction == 1.0 {
            break trace;
        }
        if (trace_contents(&trace) & WATER_MASK) != 0 && (mask & WATER_MASK) != 0 {
            water_start = Some(trace.end);
            if length3(sub3(start, trace.end)) != 0.0 {
                let contents = trace_contents(&trace);
                let brown = if rerelease { "brwater" } else { "*brwater" };
                let surface = match &trace.family {
                    TraceFamily::Q2(fields) => fields.surface.as_ref(),
                    _ => None,
                };
                let color = if (contents & 32) != 0 {
                    if matches!(&trace.family, TraceFamily::Q2(_))
                        && surface.is_some_and(|surface| surface.name == brown)
                    {
                        3
                    } else {
                        2
                    }
                } else if (contents & 16) != 0 {
                    4
                } else {
                    5
                };
                emit_effect(game, "splash", trace.end, trace_normal(&trace), 8, color);
                end = lead_spread(
                    game,
                    trace.end,
                    sub3(end, start),
                    2.0,
                    horizontal_spread,
                    vertical_spread,
                );
            }
            mask &= !WATER_MASK;
            if !rerelease {
                from = trace.end;
            }
            continue;
        }
        let actor = trace_hit(&trace);
        if (rerelease || !trace_sky(&trace)) && can_hurt(game, actor.as_ref()) {
            let target = actor.clone().expect("damageable actor");
            let weapon = weapon_for_mod(means_of_death);
            game.damage(
                target.clone(),
                owner.clone(),
                Some(owner.clone()),
                damage,
                kick,
                direction,
                trace.end,
                trace_normal(&trace),
                means_of_death,
                16,
                weapon,
            );
            let dead = game
                .host
                .combat()
                .read(&target)
                .is_some_and(|state| state.health <= 0.0);
            if rerelease
                && ((trace_contents(&trace) & 0x4000000) != 0 || game.host.is_monster(&target) && dead)
                && !excluded.contains(&target)
            {
                if excluded.len() == 16 {
                    break trace;
                }
                excluded.push(target);
                continue;
            }
        } else if !trace_sky(&trace) {
            emit_effect(game, impact, trace.end, trace_normal(&trace), 0, 0);
            player_noise(game, owner, trace.end, NoiseKind::Impact);
        }
        break trace;
    };
    if let Some(water_start) = water_start {
        let back = add3(trace.end, scale3(normalize3(sub3(trace.end, water_start)), -2.0));
        let water_end = if (game.host.point_contents(back) & WATER_MASK) != 0 {
            back
        } else {
            game.host
                .trace(&Q2TraceRequest {
                    start: back,
                    end: water_start,
                    bounds: None,
                    ignore: trace_hit(&trace),
                    mask: WATER_MASK,
                    exclude: Vec::new(),
                })
                .end
        };
        weapon_emit(
            game,
            &Q2WeaponEvent::Beam {
                effect: WeaponBeamEffect::BubbleTrail,
                actor: Some(owner.clone()),
                start: water_start,
                end: water_end,
                duration: 0.0,
            },
        );
    }
}

/// Fire a rail slug (`fireRail`).
pub fn fire_rail(owner: ActorId, game: &mut Q2GameServices, start: Vec3, direction: Vec3, damage: f64, kick: f64) {
    let end = add3(start, scale3(direction, 8192.0));
    let mut excluded: Vec<ActorId> = Vec::new();
    let mut from = start;
    let mut ignore = Some(owner.clone());
    let mut water = false;
    let mut mask = shot_mask(game, &owner) | 8 | 16;
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    let trace = loop {
        let trace = game.host.trace(&Q2TraceRequest {
            start: from,
            end,
            bounds: None,
            ignore: ignore.clone(),
            mask,
            exclude: excluded.clone(),
        });
        if trace.fraction == 1.0 {
            break trace;
        }
        if (trace_contents(&trace) & mask & (8 | 16)) != 0 {
            mask &= !(8 | 16);
            water = true;
        } else {
            let actor = trace_hit(&trace);
            let Some(actor) = actor else { break trace };
            if actor != owner && can_hurt(game, Some(&actor)) {
                game.damage(
                    actor.clone(),
                    owner.clone(),
                    Some(owner.clone()),
                    damage,
                    kick,
                    direction,
                    trace.end,
                    trace_normal(&trace),
                    Mod::RAILGUN,
                    0,
                    Some("q2:weapon_railgun".to_string()),
                );
            }
            let target = game.weapon_target(&actor);
            let pierce = game.host.is_monster(&actor)
                || game.host.is_player(&actor)
                || target.as_ref().is_some_and(|target| target.solid == Q2Solid::Box)
                || rerelease
                    && (target.as_ref().is_some_and(|target| target.damageable_target)
                        || target.as_ref().is_some_and(|target| target.solid == Q2Solid::None)
                        || target.as_ref().is_some_and(|target| target.solid == Q2Solid::Trigger)
                        || !game.host.actors().is_live(&actor));
            if !pierce || excluded.contains(&actor) {
                break trace;
            }
            if rerelease && excluded.len() == 16 {
                break trace;
            }
            excluded.push(actor.clone());
            if !rerelease {
                ignore = Some(actor);
            }
        }
        if !rerelease {
            from = trace.end;
        }
    };
    weapon_emit(
        game,
        &Q2WeaponEvent::Beam {
            effect: WeaponBeamEffect::Rail,
            actor: Some(owner.clone()),
            start,
            end: trace.end,
            duration: 0.0,
        },
    );
    if water && !rerelease {
        weapon_emit(
            game,
            &Q2WeaponEvent::Beam {
                effect: WeaponBeamEffect::RailWater,
                actor: Some(owner.clone()),
                start,
                end: trace.end,
                duration: 0.0,
            },
        );
    }
    player_noise(game, &owner, trace.end, NoiseKind::Impact);
}

/// Melee hit (`fireHit`).
pub fn fire_hit(owner: ActorId, game: &mut Q2GameServices, aim: Vec3, damage: f64, kick: f64) -> bool {
    let enemy = game.require_entity(&owner).enemy.clone();
    let Some(enemy) = enemy else {
        panic!("Q2 fireHit requires an enemy")
    };
    let Some(target_body) = game.host.bodies().read(&enemy) else {
        return false;
    };
    let body = game.body_of(owner.clone());
    let delta = sub3(target_body.origin, body.origin);
    let mut range = f64::from(length3(delta));
    let mut side = f64::from(aim.y);
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    let min = add3(target_body.origin, target_body.bounds.min);
    let max = add3(target_body.origin, target_body.bounds.max);
    if rerelease {
        let self_min = add3(body.origin, body.bounds.min);
        let self_max = add3(body.origin, body.bounds.max);
        let axis = |a: f32, b: f32| -> f64 { 0.0f64.max(f64::from(a)).max(f64::from(b)) };
        range = axis(min.x - self_max.x, self_min.x - max.x)
            .hypot(axis(min.y - self_max.y, self_min.y - max.y))
            .hypot(axis(min.z - self_max.z, self_min.z - max.z));
    }
    if range > f64::from(aim.x) {
        return false;
    }
    if side > f64::from(body.bounds.min.x) && side < f64::from(body.bounds.max.x) {
        if !rerelease {
            range -= f64::from(target_body.bounds.max.x);
        }
    } else {
        side = f64::from(if side < 0.0 {
            target_body.bounds.min.x
        } else {
            target_body.bounds.max.x
        });
    }
    let point = if rerelease {
        vec3(
            f64::from(min.x).max(f64::from(max.x).min(f64::from(body.origin.x))) as f32,
            f64::from(min.y).max(f64::from(max.y).min(f64::from(body.origin.y))) as f32,
            f64::from(min.z).max(f64::from(max.z).min(f64::from(body.origin.z))) as f32,
        )
    } else {
        add3(body.origin, scale3(delta, range as f32))
    };
    let mut target = enemy.clone();
    let mask = shot_mask(game, &owner);
    let first = game.host.trace(&Q2TraceRequest {
        start: body.origin,
        end: point,
        bounds: None,
        ignore: Some(owner.clone()),
        mask,
        exclude: Vec::new(),
    });
    let traces = if rerelease {
        let mask = shot_mask(game, &owner);
        vec![
            first,
            game.host.trace(&Q2TraceRequest {
                start: point,
                end: target_body.origin,
                bounds: None,
                ignore: Some(owner.clone()),
                mask,
                exclude: Vec::new(),
            }),
        ]
    } else {
        vec![first]
    };
    for trace in &traces {
        if trace.fraction < 1.0 {
            let actor = trace_hit(trace);
            if !can_hurt(game, actor.as_ref()) {
                return false;
            }
            let hit_actor = actor.clone().expect("damageable actor");
            target = if game.host.is_monster(&hit_actor) || game.host.is_player(&hit_actor) {
                enemy.clone()
            } else {
                hit_actor
            };
        }
    }
    let axes = angle_vectors(body.angles);
    let impact = add3(
        add3(
            add3(body.origin, scale3(axes.forward, range as f32)),
            scale3(axes.right, side as f32),
        ),
        scale3(axes.up, aim.z),
    );
    game.damage(
        target.clone(),
        owner.clone(),
        Some(owner.clone()),
        damage,
        (kick / 2.0).trunc(),
        sub3(impact, target_body.origin),
        impact,
        vec3(0.0, 0.0, 0.0),
        Mod::HIT,
        8,
        None,
    );
    if !game.host.is_monster(&target) && !game.host.is_player(&target) {
        return false;
    }
    let owned = game.host.actors().resolve_owned(&enemy);
    let current = game.host.bodies().read(&enemy);
    if let (Some(owned), Some(current)) = (owned, current) {
        let center = add3(
            current.origin,
            scale3(add3(current.bounds.min, current.bounds.max), 0.5),
        );
        let velocity = add3(current.velocity, scale3(normalize3(sub3(center, impact)), kick as f32));
        let mut moved = current;
        moved.velocity = velocity;
        if velocity.z > 0.0 {
            moved.ground = None;
        }
        game.host.bodies().write(&owned, &moved);
    }
    true
}

/// Spawn a projectile (`projectile`).
fn projectile(
    game: &mut Q2GameServices,
    owner: &ActorId,
    classname: &str,
    start: Vec3,
    direction: Vec3,
    speed: f64,
    model: &str,
    effects: i64,
) -> ActorId {
    let mask = shot_mask(game, owner);
    projectile_for_actor(
        game,
        owner.clone(),
        classname,
        start,
        direction,
        speed,
        model,
        effects,
        mask,
    )
}

/// Spawn a projectile for an owner (`projectileForActor`).
#[allow(clippy::too_many_arguments)]
fn projectile_for_actor(
    game: &mut Q2GameServices,
    owner: ActorId,
    classname: &str,
    start: Vec3,
    direction: Vec3,
    speed: f64,
    model: &str,
    effects: i64,
    clip_mask: i32,
) -> ActorId {
    register_ballistics_callbacks(game);
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    let actor = game.create(classname, std::collections::BTreeMap::new());
    {
        let projectile = game.require_entity_mut(&actor);
        projectile.owner = Some(owner);
        projectile.model = model.to_string();
        projectile.effects = effects;
        projectile.clip_mask = clip_mask;
        projectile.projectile = true;
        projectile.dodgeable = rerelease;
        projectile.movedir = direction;
    }
    let mut moved = game.body_of(actor.clone());
    moved.origin = start;
    moved.angles = vector_angles(direction);
    moved.velocity = scale3(direction, speed as f32);
    moved.bounds = qa_core::math::Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    game.write_body(actor.clone(), &moved, false);
    actor
}

/// Route a projectile through weapon behavior (`launchBehavior`).
fn launch_behavior(
    game: &mut Q2GameServices,
    projectile: &ActorId,
    weapon: ItemId,
    role: crate::contract::ProjectileRole,
) -> Option<crate::q2::support::contracts::WeaponTrajectoryUpdate> {
    let owner = game.require_entity(projectile).owner.clone();
    let Some(owner) = owner else { return None };
    if !game.host.is_player(&owner) {
        return None;
    }
    let owned = game.owned_of(projectile.clone());
    let body = game.body_of(projectile.clone());
    let input = crate::q2::support::contracts::WeaponBehaviorLaunch {
        projectile: owned,
        shooter: owner,
        weapon,
        role,
        time_seconds: game.host.now(),
        body,
    };
    let update = match game.host.weapon_behavior() {
        Some(port) => port.launch(&input),
        None => None,
    };
    if let Some(update) = update.as_ref() {
        game.project_trajectory(projectile.clone(), update);
    }
    update
}

/// Start or stop a projectile loop sound (`loop`).
fn loop_sound(game: &mut Q2GameServices, projectile: &ActorId, path: &str, start: bool) {
    let origin = game.body_of(projectile.clone()).origin;
    game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::Sound(
        crate::q2::foundation::host::Q2SoundEvent {
            actor: Some(projectile.clone()),
            origin,
            path: path.to_string(),
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: if start {
                crate::q2::foundation::host::Q2SoundLoop::Start
            } else {
                crate::q2::foundation::host::Q2SoundLoop::Stop
            },
            loop_owner: None,
        },
    ));
}

/// Fire a blaster bolt (`fireBlaster`).
#[allow(clippy::too_many_arguments)]
pub fn fire_blaster(
    owner: ActorId,
    game: &mut Q2GameServices,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    speed: f64,
    effects: i64,
    hyper: bool,
    means_of_death: i32,
) -> ActorId {
    let classic = game.options.edition == Q2Edition::Classic;
    let dir = if classic { normalize3(direction) } else { direction };
    let bolt = projectile(
        game,
        &owner,
        "bolt",
        start,
        dir,
        speed,
        "models/objects/laser/tris.md2",
        effects,
    );
    game.require_entity_mut(&bolt).damage = damage;
    game.weapons.blaster_causes.insert(bolt.clone(), means_of_death);
    game.require_entity_mut(&bolt).touch = Some(blaster_touch as crate::q2::foundation::host::Q2Touch);
    game.schedule(
        bolt.clone(),
        2.0,
        free_q2_entity as crate::q2::foundation::host::Q2Think,
    );
    game.set_solid(bolt.clone(), Q2Solid::Box);
    game.set_motion_kind(bolt.clone(), Q2MotionKind::FlyMissile);
    let trajectory = launch_behavior(
        game,
        &bolt,
        if hyper {
            "q2:weapon_hyperblaster".to_string()
        } else {
            "q2:weapon_blaster".to_string()
        },
        crate::contract::ProjectileRole::Bolt,
    );
    let launch_origin = game.body_of(bolt.clone()).origin;
    let launch_direction = trajectory
        .as_ref()
        .map(|update| normalize3(update.velocity))
        .unwrap_or(dir);
    game.show(bolt.clone());
    loop_sound(game, &bolt, "misc/lasfly.wav", true);
    check_dodge(game, &owner, start, dir, speed);
    let clip_mask = game.require_entity(&bolt).clip_mask;
    let self_origin = game.body_of(owner.clone()).origin;
    let trace = game.host.trace(&Q2TraceRequest {
        start: self_origin,
        end: launch_origin,
        bounds: None,
        ignore: Some(bolt.clone()),
        mask: clip_mask,
        exclude: Vec::new(),
    });
    if trace.fraction < 1.0 {
        let mut moved = game.body_of(bolt.clone());
        moved.origin = if classic {
            add3(launch_origin, scale3(launch_direction, -10.0))
        } else {
            add3(trace.end, trace_normal(&trace))
        };
        game.write_body(bolt.clone(), &moved, true);
        let other = trace_hit(&trace);
        let plane = if classic {
            vec3(0.0, 0.0, 0.0)
        } else {
            trace_normal(&trace)
        };
        let sky_hit = !classic && trace_sky(&trace);
        blaster_impact(game, &bolt, other.as_ref(), plane, sky_hit);
    }
    bolt
}

/// Fire a grenade (`fireGrenade`).
#[allow(clippy::too_many_arguments)]
pub fn fire_grenade(
    owner: ActorId,
    game: &mut Q2GameServices,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    speed: f64,
    timer: f64,
    radius: f64,
    hand: bool,
    held: bool,
    monster: bool,
    adjustment: Option<Q2GrenadeAdjustment>,
) -> ActorId {
    let gravity = adjustment
        .map(|adjustment| adjustment.gravity)
        .or_else(|| game.weapons.inputs.get(&owner).map(|input| input.gravity))
        .unwrap_or(800.0);
    let mask = shot_mask(game, &owner);
    launch_grenade(
        game, owner, start, direction, damage, speed, timer, radius, hand, held, monster, gravity, mask, adjustment,
    )
}

/// Fire a hand grenade (`fireHandGrenade`).
pub fn fire_hand_grenade(owner: ActorId, game: &mut Q2GameServices, spec: &Q2HandGrenadeLaunch) -> ActorId {
    let mask = q2_actor_shot_mask(game, spec.players_collide);
    launch_grenade(
        game,
        owner,
        spec.start,
        spec.direction,
        spec.damage,
        spec.speed,
        spec.timer,
        spec.radius,
        true,
        spec.held,
        false,
        spec.gravity,
        mask,
        None,
    )
}

/// Launch a grenade (`launchGrenade`).
#[allow(clippy::too_many_arguments)]
fn launch_grenade(
    game: &mut Q2GameServices,
    owner: ActorId,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    speed: f64,
    timer: f64,
    radius: f64,
    hand: bool,
    held: bool,
    monster: bool,
    owner_gravity: f64,
    clip_mask: i32,
    adjustment: Option<Q2GrenadeAdjustment>,
) -> ActorId {
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    let axes = angle_vectors(vector_angles(direction));
    let model = if hand {
        if rerelease {
            "grenade3"
        } else {
            "grenade2"
        }
    } else if rerelease && !monster {
        "grenade4"
    } else {
        "grenade"
    };
    let classname = if hand {
        if rerelease {
            "hand_grenade"
        } else {
            "hgrenade"
        }
    } else {
        "grenade"
    };
    let grenade = projectile_for_actor(
        game,
        owner.clone(),
        classname,
        start,
        direction,
        speed,
        &format!("models/objects/{model}/tris.md2"),
        32 + if rerelease && monster && !hand { 1i64 << 37 } else { 0 },
        clip_mask,
    );
    {
        let entity = game.require_entity_mut(&grenade);
        entity.damage_radius = radius;
        entity.damage = damage;
        entity.speed = speed;
        entity.spawnflags = if hand {
            if held {
                3
            } else {
                1
            }
        } else {
            0
        };
    }
    let gravity = if rerelease { owner_gravity / 800.0 } else { 1.0 };
    let up = adjustment
        .map(|adjustment| adjustment.up)
        .unwrap_or(200.0 + (game.random() * 2.0 - 1.0) * 10.0)
        * gravity;
    let right = adjustment
        .map(|adjustment| adjustment.right)
        .unwrap_or((game.random() * 2.0 - 1.0) * 10.0);
    let mut moved = game.body_of(grenade.clone());
    moved.velocity = add3(
        add3(scale3(direction, speed as f32), scale3(axes.up, up as f32)),
        scale3(axes.right, right as f32),
    );
    game.write_body(grenade.clone(), &moved, false);
    game.require_entity_mut(&grenade).angular_velocity = if rerelease {
        if hand || monster {
            vec3(
                ((game.random() * 2.0 - 1.0) * 360.0) as f32,
                ((game.random() * 2.0 - 1.0) * 360.0) as f32,
                ((game.random() * 2.0 - 1.0) * 360.0) as f32,
            )
        } else {
            vec3(0.0, 0.0, 0.0)
        }
    } else {
        vec3(300.0, 300.0, 300.0)
    };
    game.require_entity_mut(&grenade).touch = Some(grenade_touch as crate::q2::foundation::host::Q2Touch);
    if rerelease && !hand && !monster {
        let timestamp = game.host.now() + timer;
        game.require_entity_mut(&grenade).timestamp = timestamp;
        game.require_entity_mut(&grenade).render_flags |= 1;
        let mut moved = game.body_of(grenade.clone());
        moved.angles = vector_angles(moved.velocity);
        game.write_body(grenade.clone(), &moved, false);
        let frame = game.host.frame_seconds();
        game.schedule(
            grenade.clone(),
            frame,
            grenade_update as crate::q2::foundation::host::Q2Think,
        );
    } else {
        game.schedule(
            grenade.clone(),
            timer.max(0.0),
            grenade_explode as crate::q2::foundation::host::Q2Think,
        );
    }
    if hand {
        loop_sound(game, &grenade, "weapons/hgrenc1b.wav", true);
    }
    if hand && timer <= 0.0 {
        grenade_explode(grenade.clone(), game);
    } else {
        if hand {
            let body = game
                .host
                .bodies()
                .read(&owner)
                .expect("Q2 grenade thrower has no shared body");
            game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::Sound(
                crate::q2::foundation::host::Q2SoundEvent {
                    actor: Some(owner),
                    origin: body.origin,
                    path: "weapons/hgrent1a.wav".to_string(),
                    channel: 1,
                    volume: 1.0,
                    attenuation: 1.0,
                    reliable: false,
                    loop_: crate::q2::foundation::host::Q2SoundLoop::Once,
                    loop_owner: None,
                },
            ));
        }
        game.set_solid(grenade.clone(), Q2Solid::Box);
        game.set_motion_kind(grenade.clone(), Q2MotionKind::Bounce);
        launch_behavior(
            game,
            &grenade,
            if hand {
                "q2:ammo_grenades".to_string()
            } else {
                "q2:weapon_grenadelauncher".to_string()
            },
            crate::contract::ProjectileRole::Grenade,
        );
        game.show(grenade.clone());
    }
    grenade
}

/// Fire a rocket (`fireRocket`).
#[allow(clippy::too_many_arguments)]
pub fn fire_rocket(
    owner: ActorId,
    game: &mut Q2GameServices,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    speed: f64,
    radius: f64,
    radius_damage: f64,
) -> ActorId {
    let rocket = projectile(
        game,
        &owner,
        "rocket",
        start,
        direction,
        speed,
        "models/objects/rocket/tris.md2",
        16,
    );
    {
        let entity = game.require_entity_mut(&rocket);
        entity.damage = damage;
        entity.damage_radius = radius;
        entity.radius_damage = radius_damage;
        entity.touch = Some(rocket_touch as crate::q2::foundation::host::Q2Touch);
    }
    game.schedule(
        rocket.clone(),
        8000.0 / speed,
        free_q2_entity as crate::q2::foundation::host::Q2Think,
    );
    game.set_solid(rocket.clone(), Q2Solid::Box);
    game.set_motion_kind(rocket.clone(), Q2MotionKind::FlyMissile);
    launch_behavior(
        game,
        &rocket,
        "q2:weapon_rocketlauncher".to_string(),
        crate::contract::ProjectileRole::Rocket,
    );
    game.show(rocket.clone());
    loop_sound(game, &rocket, "weapons/rockfly.wav", true);
    check_dodge(game, &owner, start, direction, speed);
    rocket
}

/// Spawn rocket debris (`debris`).
fn debris(game: &mut Q2GameServices, source: &ActorId) {
    let piece = game.create("debris", std::collections::BTreeMap::new());
    let body = game.body_of(source.clone());
    game.require_entity_mut(&piece).model = "models/objects/debris2/tris.md2".to_string();
    let mut moved = game.body_of(piece.clone());
    moved.origin = body.origin;
    moved.velocity = add3(
        body.velocity,
        scale3(
            vec3(
                (100.0 * (game.random() * 2.0 - 1.0)) as f32,
                (100.0 * (game.random() * 2.0 - 1.0)) as f32,
                (100.0 + 100.0 * (game.random() * 2.0 - 1.0)) as f32,
            ),
            2.0,
        ),
    );
    game.write_body(piece.clone(), &moved, false);
    game.require_entity_mut(&piece).angular_velocity = vec3(
        (game.random() * 600.0) as f32,
        (game.random() * 600.0) as f32,
        (game.random() * 600.0) as f32,
    );
    let owned = game.owned_of(piece.clone());
    game.create_combat(&owned, 0.0, 0.0, true);
    game.require_entity_mut(&piece).die =
        Some(crate::q2::foundation::entity_services::free_q2_entity_die as crate::q2::foundation::host::Q2Die);
    game.set_motion_kind(piece.clone(), Q2MotionKind::Bounce);
    game.set_solid(piece.clone(), Q2Solid::None);
    game.show(piece.clone());
    let delay = 5.0 + game.random() * 5.0;
    game.schedule(piece, delay, free_q2_entity as crate::q2::foundation::host::Q2Think);
}

/// Fire a BFG projectile (`fireBfg`).
pub fn fire_bfg(
    owner: ActorId,
    game: &mut Q2GameServices,
    start: Vec3,
    direction: Vec3,
    damage: f64,
    speed: f64,
    radius: f64,
) -> ActorId {
    let bfg = projectile(
        game,
        &owner,
        "bfg blast",
        start,
        direction,
        speed,
        "sprites/s_bfg1.sp2",
        128 | 8192,
    );
    {
        let entity = game.require_entity_mut(&bfg);
        entity.dodgeable = false;
        entity.damage = damage;
        entity.damage_radius = radius;
        entity.touch = Some(bfg_touch as crate::q2::foundation::host::Q2Touch);
    }
    check_dodge(game, &owner, start, direction, speed);
    let frame = game.host.frame_seconds();
    game.schedule(bfg.clone(), frame, bfg_think as crate::q2::foundation::host::Q2Think);
    game.set_solid(bfg.clone(), Q2Solid::Box);
    game.set_motion_kind(bfg.clone(), Q2MotionKind::FlyMissile);
    launch_behavior(
        game,
        &bfg,
        "q2:weapon_bfg".to_string(),
        crate::contract::ProjectileRole::Energy,
    );
    game.show(bfg.clone());
    loop_sound(game, &bfg, "weapons/bfg__l1a.wav", true);
    bfg
}

/// Explode a grenade (`grenadeExplode`).
pub fn grenade_explode(actor: ActorId, game: &mut Q2GameServices) {
    let spawnflags = game.require_entity(&actor).spawnflags;
    let hand = (spawnflags & 1) != 0;
    let held = (spawnflags & 2) != 0;
    impact_noise(game, &actor);
    let body = game.body_of(actor.clone());
    let direct = game.require_entity(&actor).enemy.clone();
    if let Some(direct) = direct.as_ref() {
        let center = centroid(game, direct);
        let target_body = game.host.bodies().read(direct);
        if let (Some(center), Some(target_body)) = (center, target_body) {
            let damage = game.require_entity(&actor).damage;
            let points = (damage - 0.5 * f64::from(length3(sub3(body.origin, center)))).trunc();
            let owner = game.require_entity(&actor).owner.clone();
            game.damage(
                direct.clone(),
                actor.clone(),
                owner,
                points,
                points,
                sub3(target_body.origin, body.origin),
                body.origin,
                vec3(0.0, 0.0, 0.0),
                if hand { Mod::HAND_GRENADE } else { Mod::GRENADE },
                1,
                Some(if hand {
                    "q2:ammo_grenades".to_string()
                } else {
                    "q2:weapon_grenadelauncher".to_string()
                }),
            );
        }
    }
    let entity = game.require_entity(&actor).clone();
    game.radius_damage(
        actor.clone(),
        entity.owner,
        entity.damage,
        direct,
        entity.damage_radius,
        if held {
            Mod::HELD_GRENADE
        } else if hand {
            Mod::HAND_GRENADE_SPLASH
        } else {
            Mod::GRENADE_SPLASH
        },
        0,
        Some(if hand {
            "q2:ammo_grenades".to_string()
        } else {
            "q2:weapon_grenadelauncher".to_string()
        }),
    );
    let body = game.body_of(actor.clone());
    let wet = (game.host.point_contents(body.origin) & WATER_MASK) != 0;
    let name = format!(
        "{}-explosion{}",
        if body.ground.is_none() { "rocket" } else { "grenade" },
        if wet { "-water" } else { "" }
    );
    emit_effect(
        game,
        &name,
        add3(body.origin, scale3(body.velocity, -0.02)),
        vec3(0.0, 0.0, 0.0),
        0,
        0,
    );
    game.remove_actor(actor);
}

/// Grenade touch (`grenadeTouch`).
pub fn grenade_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let entity = game.require_entity(&actor).clone();
    let hand = (entity.spawnflags & 1) != 0;
    if Some(&contact.other) == entity.owner.as_ref() {
        return;
    }
    if (contact
        .surface
        .as_ref()
        .map(|surface| surface.native_flags)
        .unwrap_or(0)
        & 4)
        != 0
    {
        game.remove_actor(actor);
        return;
    }
    if !can_hurt(game, Some(&contact.other)) {
        let path = if hand {
            if game.random() > 0.5 {
                "weapons/hgrenb1a.wav"
            } else {
                "weapons/hgrenb2a.wav"
            }
        } else {
            "weapons/grenlb1b.wav"
        };
        game.sound(&actor, path, 2, 1.0, 1.0);
        return;
    }
    game.require_entity_mut(&actor).enemy = Some(contact.other);
    grenade_explode(actor, game);
}

/// Rerelease grenade spin (`grenadeUpdate`).
pub fn grenade_update(actor: ActorId, game: &mut Q2GameServices) {
    if game.host.now() >= game.require_entity(&actor).timestamp {
        grenade_explode(actor, game);
        return;
    }
    let body = game.body_of(actor.clone());
    let velocity_squared = f64::from(dot3(body.velocity, body.velocity));
    if velocity_squared != 0.0 {
        let speed = game.require_entity(&actor).speed;
        let fraction = 1.0f64.min(velocity_squared / (speed * speed));
        let angles = vector_angles(body.velocity);
        let mut moved = game.body_of(actor.clone());
        moved.angles = vec3(
            lerp_angle(f64::from(body.angles.x), f64::from(angles.x), fraction) as f32,
            angles.y,
            (f64::from(body.angles.z) + game.host.frame_seconds() * 360.0 * fraction) as f32,
        );
        game.write_body(actor.clone(), &moved, true);
    }
    let frame = game.host.frame_seconds();
    game.schedule(actor, frame, grenade_update as crate::q2::foundation::host::Q2Think);
}

/// Rocket touch (`rocketTouch`).
pub fn rocket_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let entity = game.require_entity(&actor).clone();
    if Some(&contact.other) == entity.owner.as_ref() {
        return;
    }
    if (contact
        .surface
        .as_ref()
        .map(|surface| surface.native_flags)
        .unwrap_or(0)
        & 4)
        != 0
    {
        game.remove_actor(actor);
        return;
    }
    impact_noise(game, &actor);
    let body = game.body_of(actor.clone());
    let plane = contact.plane.map(|plane| plane.normal).unwrap_or(vec3(0.0, 0.0, 0.0));
    if can_hurt(game, Some(&contact.other)) {
        game.damage(
            contact.other.clone(),
            actor.clone(),
            entity.owner.clone(),
            entity.damage,
            0.0,
            body.velocity,
            body.origin,
            plane,
            Mod::ROCKET,
            0,
            Some("q2:weapon_rocketlauncher".to_string()),
        );
    } else if game.options.mode == Q2Mode::Singleplayer
        && contact
            .surface
            .as_ref()
            .is_some_and(|surface| (surface.native_flags & (8 | 16 | 32 | 64)) == 0)
    {
        let count = (game.random() * 5.0).floor() as i32;
        for _ in 0..count {
            debris(game, &actor);
        }
    }
    let entity = game.require_entity(&actor).clone();
    game.radius_damage(
        actor.clone(),
        entity.owner,
        entity.radius_damage,
        Some(contact.other),
        entity.damage_radius,
        Mod::ROCKET_SPLASH,
        0,
        Some("q2:weapon_rocketlauncher".to_string()),
    );
    let body = game.body_of(actor.clone());
    let wet = (game.host.point_contents(body.origin) & WATER_MASK) != 0;
    let name = if wet {
        "rocket-explosion-water"
    } else {
        "rocket-explosion"
    };
    let origin = if game.options.edition == Q2Edition::Classic {
        add3(body.origin, scale3(body.velocity, -0.02))
    } else {
        add3(body.origin, plane)
    };
    emit_effect(game, name, origin, vec3(0.0, 0.0, 0.0), 0, 0);
    game.remove_actor(actor);
}

/// BFG explosion frames (`bfgExplode`).
pub fn bfg_explode(actor: ActorId, game: &mut Q2GameServices) {
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    if rerelease {
        bfg_ambient(game, &actor);
    }
    if game.require_entity(&actor).frame == 0 {
        let origin = game.body_of(actor.clone()).origin;
        let entity = game.require_entity(&actor).clone();
        for target in game.host.nearby(origin, entity.damage_radius) {
            if Some(&target) == entity.owner.as_ref()
                || !can_hurt(game, Some(&target))
                || !game.can_damage(&target, &actor)
            {
                continue;
            }
            if let Some(owner) = entity.owner.as_ref() {
                let resolved = game.host.actors().resolve_owned(owner);
                if resolved
                    .as_ref()
                    .is_some_and(|owned| game.host.bodies().read(owned.id()).is_some())
                    && !game.can_damage(&target, owner)
                {
                    continue;
                }
            }
            if rerelease && (!bfg_target(game, &target) || !can_target(game, entity.owner.as_ref(), &target)) {
                continue;
            }
            let center = centroid(game, &target);
            let body = game.host.bodies().read(&target);
            let (Some(center), Some(body)) = (center, body) else {
                continue;
            };
            let origin = game.body_of(actor.clone()).origin;
            let distance = f64::from(length3(sub3(origin, center)));
            let entity = game.require_entity(&actor).clone();
            let points = (entity.damage * (1.0 - (distance / entity.damage_radius).sqrt())).trunc();
            if !rerelease {
                emit_effect(game, "bfg-explosion", body.origin, vec3(0.0, 0.0, 0.0), 0, 0);
            }
            let velocity = game.body_of(actor.clone()).velocity;
            game.damage(
                target,
                actor.clone(),
                entity.owner,
                points,
                0.0,
                velocity,
                if rerelease { center } else { body.origin },
                vec3(0.0, 0.0, 0.0),
                Mod::BFG_EFFECT,
                4,
                Some("q2:weapon_bfg".to_string()),
            );
            if rerelease {
                let origin = game.body_of(actor.clone()).origin;
                weapon_emit(
                    game,
                    &Q2WeaponEvent::Beam {
                        effect: WeaponBeamEffect::BfgZap,
                        actor: Some(actor.clone()),
                        start: origin,
                        end: center,
                        duration: 0.0,
                    },
                );
            }
        }
    }
    game.require_entity_mut(&actor).frame += 1;
    game.show(actor.clone());
    let frame = game.require_entity(&actor).frame;
    game.schedule(
        actor,
        0.1,
        if frame == 5 {
            free_q2_entity as crate::q2::foundation::host::Q2Think
        } else {
            bfg_explode as crate::q2::foundation::host::Q2Think
        },
    );
}

/// BFG touch (`bfgTouch`).
pub fn bfg_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let entity = game.require_entity(&actor).clone();
    if Some(&contact.other) == entity.owner.as_ref() {
        return;
    }
    if (contact
        .surface
        .as_ref()
        .map(|surface| surface.native_flags)
        .unwrap_or(0)
        & 4)
        != 0
    {
        game.remove_actor(actor);
        return;
    }
    impact_noise(game, &actor);
    let body = game.body_of(actor.clone());
    let energy = if game.options.edition == Q2Edition::Rerelease {
        4
    } else {
        0
    };
    if can_hurt(game, Some(&contact.other)) {
        game.damage(
            contact.other.clone(),
            actor.clone(),
            entity.owner.clone(),
            200.0,
            0.0,
            body.velocity,
            body.origin,
            contact.plane.map(|plane| plane.normal).unwrap_or(vec3(0.0, 0.0, 0.0)),
            Mod::BFG_BLAST,
            energy,
            Some("q2:weapon_bfg".to_string()),
        );
    }
    game.radius_damage(
        actor.clone(),
        entity.owner,
        200.0,
        Some(contact.other.clone()),
        100.0,
        Mod::BFG_BLAST,
        energy,
        Some("q2:weapon_bfg".to_string()),
    );
    game.sound(&actor, "weapons/bfg__x1b.wav", 2, 1.0, 1.0);
    loop_sound(game, &actor, "weapons/bfg__l1a.wav", false);
    {
        let entity = game.require_entity_mut(&actor);
        entity.touch = None;
        entity.enemy = Some(contact.other);
        entity.model = "sprites/s_bfg3.sp2".to_string();
        entity.frame = 0;
        entity.effects &= !8192;
    }
    let frame = game.host.frame_seconds();
    let mut moved = game.body_of(actor.clone());
    moved.origin = add3(body.origin, scale3(body.velocity, -(frame as f32)));
    moved.velocity = vec3(0.0, 0.0, 0.0);
    game.write_body(actor.clone(), &moved, true);
    game.set_solid(actor.clone(), Q2Solid::None);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Stationary);
    game.show(actor.clone());
    let origin = game.body_of(actor.clone()).origin;
    emit_effect(game, "bfg-bigexplosion", origin, vec3(0.0, 0.0, 0.0), 0, 0);
    game.schedule(actor, 0.1, bfg_explode as crate::q2::foundation::host::Q2Think);
}

/// BFG laser think (`bfgThink`).
pub fn bfg_think(actor: ActorId, game: &mut Q2GameServices) {
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    if rerelease {
        bfg_ambient(game, &actor);
    }
    let origin = game.body_of(actor.clone()).origin;
    for target in game.host.nearby(origin, 256.0) {
        let entity = game.require_entity(&actor).clone();
        if target == actor
            || Some(&target) == entity.owner.as_ref()
            || !can_hurt(game, Some(&target))
            || !bfg_target(game, &target)
        {
            continue;
        }
        if rerelease && !can_target(game, entity.owner.as_ref(), &target) {
            continue;
        }
        let Some(center) = centroid(game, &target) else {
            continue;
        };
        if rerelease
            && game
                .host
                .trace(&Q2TraceRequest {
                    start: origin,
                    end: center,
                    bounds: None,
                    ignore: None,
                    mask: 3,
                    exclude: Vec::new(),
                })
                .fraction
                < 1.0
        {
            continue;
        }
        let direction = normalize3(sub3(center, origin));
        let end = add3(origin, scale3(direction, 2048.0));
        let mut excluded: Vec<ActorId> = Vec::new();
        let mut from = origin;
        let mut ignore = Some(actor.clone());
        let trace = loop {
            let trace = game.host.trace(&Q2TraceRequest {
                start: from,
                end,
                bounds: None,
                ignore: ignore.clone(),
                mask: 1 | 0x2000000 | 0x4000000 | if rerelease { PLAYER_CONTENTS } else { 0 },
                exclude: excluded.clone(),
            });
            if trace.fraction == 1.0 {
                break trace;
            }
            let entity = game.require_entity(&actor).clone();
            let hit_actor = trace_hit(&trace);
            if hit_actor.as_ref() != entity.owner.as_ref()
                && can_hurt(game, hit_actor.as_ref())
                && !game
                    .weapon_target(hit_actor.as_ref().expect("damageable actor"))
                    .is_some_and(|target| target.laser_immune)
            {
                let zap = hit_actor.clone().expect("damageable actor");
                game.damage(
                    zap,
                    actor.clone(),
                    entity.owner,
                    if game.options.mode == Q2Mode::Deathmatch {
                        5.0
                    } else {
                        10.0
                    },
                    1.0,
                    direction,
                    trace.end,
                    vec3(0.0, 0.0, 0.0),
                    Mod::BFG_LASER,
                    4,
                    Some("q2:weapon_bfg".to_string()),
                );
            }
            let stop = match hit_actor.as_ref() {
                None => true,
                Some(hit_actor) => {
                    !game.host.is_monster(hit_actor)
                        && !game.host.is_player(hit_actor)
                        && !(rerelease
                            && game
                                .weapon_target(hit_actor)
                                .is_some_and(|target| target.damageable_target))
                }
            };
            if stop {
                emit_effect(game, "laser-sparks", trace.end, trace_normal(&trace), 4, entity.skin);
                break trace;
            }
            let hit_actor = hit_actor.clone().expect("piercing actor");
            if excluded.contains(&hit_actor) || rerelease && excluded.len() == 16 {
                break trace;
            }
            excluded.push(hit_actor.clone());
            if !rerelease {
                from = trace.end;
                ignore = Some(hit_actor);
            }
        };
        weapon_emit(
            game,
            &Q2WeaponEvent::Beam {
                effect: WeaponBeamEffect::BfgLaser,
                actor: Some(actor.clone()),
                start: origin,
                end: trace.end,
                duration: 0.0,
            },
        );
    }
    game.schedule(actor, 0.1, bfg_think as crate::q2::foundation::host::Q2Think);
}

/// Blaster impact (`blasterImpact`).
fn blaster_impact(game: &mut Q2GameServices, entity: &ActorId, other: Option<&ActorId>, plane: Vec3, sky_hit: bool) {
    let means_of_death = game
        .weapons
        .blaster_causes
        .get(entity)
        .copied()
        .expect("Q2 blaster projectile lost its source cause");
    let bolt = game.require_entity(entity).clone();
    if other == bolt.owner.as_ref() {
        return;
    }
    if sky_hit {
        game.remove_actor(entity.clone());
        return;
    }
    impact_noise(game, entity);
    if can_hurt(game, other) {
        let target = other.cloned().expect("damageable actor");
        let body = game.body_of(entity.clone());
        game.damage(
            target,
            entity.clone(),
            bolt.owner,
            bolt.damage,
            1.0,
            body.velocity,
            body.origin,
            plane,
            means_of_death,
            4,
            if means_of_death == Mod::HYPERBLASTER {
                Some("q2:weapon_hyperblaster".to_string())
            } else if means_of_death == Mod::BLASTER {
                Some("q2:weapon_blaster".to_string())
            } else {
                None
            },
        );
    } else {
        let origin = game.body_of(entity.clone()).origin;
        emit_effect(game, "blaster", origin, plane, 0, 0);
    }
    game.remove_actor(entity.clone());
}

/// Blaster touch (`blasterTouch`).
pub fn blaster_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let plane = contact.plane.map(|plane| plane.normal).unwrap_or(vec3(0.0, 0.0, 0.0));
    let sky_hit = (contact
        .surface
        .as_ref()
        .map(|surface| surface.native_flags)
        .unwrap_or(0)
        & 4)
        != 0;
    blaster_impact(game, &actor, Some(&contact.other), plane, sky_hit);
}

/// Whether an actor is a BFG target (`bfgTarget`).
fn bfg_target(game: &mut Q2GameServices, actor: &ActorId) -> bool {
    game.host.is_monster(actor)
        || game.host.is_player(actor)
        || game.weapon_target(actor).is_some_and(|target| target.bfg_explobox)
        || game.options.edition == Q2Edition::Rerelease
            && game.weapon_target(actor).is_some_and(|target| target.damageable_target)
}

/// Rerelease BFG lightning (`bfgAmbient`).
fn bfg_ambient(game: &mut Q2GameServices, entity: &ActorId) {
    let theta = game.random() * 2.0 * std::f64::consts::PI;
    let phi = (game.random() * 2.0 - 1.0).acos();
    let direction = vec3(
        (phi.sin() * theta.cos()) as f32,
        (phi.sin() * theta.sin()) as f32,
        phi.cos() as f32,
    );
    let start = game.body_of(entity.clone()).origin;
    let trace = game.host.trace(&Q2TraceRequest {
        start,
        end: add3(start, scale3(direction, 256.0)),
        bounds: None,
        ignore: Some(entity.clone()),
        mask: 1 | 16 | 8,
        exclude: Vec::new(),
    });
    if trace.fraction < 1.0 {
        weapon_emit(
            game,
            &Q2WeaponEvent::Beam {
                effect: WeaponBeamEffect::BfgLightning,
                actor: Some(entity.clone()),
                start,
                end: trace.end,
                duration: 0.3,
            },
        );
    }
}

/// Projectile contact (`Q2ProjectileContact`).
pub type Q2ProjectileContact = TouchContact;
