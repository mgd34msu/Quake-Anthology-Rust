//! Mission-pack mines (`src/content/q2/missionpacks/projectiles/mines.ts`).
//!
//! Rogue g_newweap.c proximity mines and Tesla coils (GPL-2.0-or-later).

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, scale3, sub3, vec3, Vec3};

use crate::contract::{ArmorState, PoweredProtectionState, ProjectileRole, RegularArmorState};
use crate::q2::foundation::callbacks::{free_q2_entity, Q2CallbackDefinitions};
use crate::q2::foundation::host::{
    Q2Die, Q2Edition, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2Think, Q2Touch,
    Q2TraceRequest,
};
use crate::q2::foundation::weapons::ballistics::{weapon_player_noise_for_actor, NoiseKind};
use crate::q2::foundation::weapons::player::{weapon_can_target, weapon_emit};
use crate::q2::foundation::weapons::types::{Q2WeaponEvent, WeaponBeamEffect};
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use crate::q2::support::contracts::{CombatState, CombatTraitChanges, DeathReaction, TouchContact, TraceContact};

use super::super::types::Q2_MISSION_PACK_DAMAGE;
use super::common::{projectile, projectile_mask, publish_projectile, sight};
use super::Q2MissionPackProjectiles;

/// Mine life for a multiplier (`mineLife`).
fn mine_life(multiplier: f64) -> f64 {
    if multiplier == 2.0 {
        30.0
    } else if multiplier == 4.0 {
        15.0
    } else if multiplier == 8.0 {
        10.0
    } else {
        45.0
    }
}

/// Whether an entity is a player start (`playerStart`).
fn player_start(entity: &crate::q2::foundation::host::Q2Entity, game: &Q2GameServices) -> bool {
    if game.options.edition == Q2Edition::Rerelease {
        entity.classname.starts_with("info_player_")
            || entity.classname == "misc_teleporter_dest"
            || entity.classname.starts_with("item_flag_")
    } else {
        matches!(
            entity.classname.as_str(),
            "info_player_deathmatch" | "info_player_start" | "info_player_coop" | "misc_teleporter_dest"
        )
    }
}

/// Mine callbacks (merged over the bolt callbacks).
pub fn mine_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = super::bolts::bolt_callbacks();
    callbacks.think.insert("Prox_Explode", prox_explode as Q2Think);
    callbacks.think.insert("prox_open", prox_open as Q2Think);
    callbacks.think.insert("prox_seek", prox_seek as Q2Think);
    callbacks.think.insert("Prox_Think", prox_flight as Q2Think);
    callbacks.think.insert("tesla_think", tesla_open as Q2Think);
    callbacks.think.insert("tesla_activate", tesla_activate as Q2Think);
    callbacks.think.insert("tesla_think_active", tesla_active as Q2Think);
    callbacks.touch.insert("prox_land", prox_land as Q2Touch);
    callbacks.touch.insert("Prox_Field_Touch", prox_field as Q2Touch);
    callbacks.touch.insert("tesla_lava", tesla_lava as Q2Touch);
    callbacks.touch.insert("badarea_touch", bad_area_touch as Q2Touch);
    callbacks.die.insert("prox_die", prox_die as Q2Die);
    callbacks.die.insert("tesla_die", tesla_die as Q2Die);
    callbacks
}

impl Q2MissionPackProjectiles {
    /// Spawn a bad area (`spawnBadArea`).
    pub fn spawn_bad_area(
        &self,
        game: &mut Q2GameServices,
        min: Vec3,
        max: Vec3,
        lifespan: f64,
        owner: Option<ActorId>,
    ) -> ActorId {
        game.source_callbacks.register(&mine_callbacks());
        let area = game.create("bad_area", BTreeMap::new());
        let origin = scale3(add3(min, max), 0.5);
        {
            let entity = game.require_entity_mut(&area);
            entity.touch = Some(bad_area_touch as Q2Touch);
            entity.owner = owner;
        }
        let mut moved = game.body_of(area.clone());
        moved.origin = origin;
        moved.bounds.min = sub3(min, origin);
        moved.bounds.max = sub3(max, origin);
        game.write_body(area.clone(), &moved, true);
        game.set_solid(area.clone(), Q2Solid::Trigger);
        game.set_motion_kind(area.clone(), Q2MotionKind::Stationary);
        if lifespan != 0.0 {
            game.schedule(area.clone(), lifespan, free_q2_entity);
        }
        area
    }

    /// Find a bad area overlapping an actor (`badAreaEntity`).
    #[allow(unpredictable_function_pointer_comparisons)]
    pub fn bad_area_entity(&self, actor: ActorId, game: &mut Q2GameServices, origin: Option<Vec3>) -> Option<ActorId> {
        let body = game.host.bodies().read(&actor)?;
        let base = origin.unwrap_or(body.origin);
        let min = add3(base, body.bounds.min);
        let max = add3(base, body.bounds.max);
        let mut areas: Vec<ActorId> = game.entities.keys().cloned().collect();
        areas.sort_by_cached_key(|actor| {
            let saved = crate::q2::foundation::checkpoint::save_q2_actor(Some(actor));
            saved.map(|saved| (saved.slot, saved.generation))
        });
        for area in areas {
            let entity = game.require_entity(&area).clone();
            if entity.touch != Some(bad_area_touch as Q2Touch) || entity.solid != Q2Solid::Trigger {
                continue;
            }
            let other = game.body_of(area.clone());
            let area_min = add3(other.origin, other.bounds.min);
            let area_max = add3(other.origin, other.bounds.max);
            if min.x <= area_max.x
                && max.x >= area_min.x
                && min.y <= area_max.y
                && max.y >= area_min.y
                && min.z <= area_max.z
                && max.z >= area_min.z
            {
                return Some(area);
            }
        }
        None
    }

    /// Whether an actor is in a bad area (`badArea`).
    pub fn bad_area(&self, actor: ActorId, game: &mut Q2GameServices) -> bool {
        self.bad_area_entity(actor, game, None).is_some()
    }

    /// Mark a tesla area (`markTeslaArea`).
    pub fn mark_tesla_area(&self, owner: &ActorId, game: &mut Q2GameServices, tesla: &ActorId) -> bool {
        if !game.host.actors().is_live(owner) || !game.host.actors().is_live(tesla) {
            return false;
        }
        let mut tail = tesla.clone();
        let mut next = game.require_entity(tesla).team_chain.clone();
        while let Some(link) = next
            .clone()
            .and_then(|link| game.entity(&link).map(|entity| entity.actor.id().clone()))
        {
            if game.require_entity(&link).classname == "bad_area" {
                return false;
            }
            tail = link.clone();
            next = game.require_entity(&link).team_chain.clone();
        }
        let trigger = game
            .require_entity(tesla)
            .team_chain
            .clone()
            .and_then(|chain| game.entity(&chain).map(|entity| entity.actor.id().clone()));
        let (min, max, origin) = match trigger.as_ref() {
            None => (
                vec3(-128.0, -128.0, game.body_of(tesla.clone()).bounds.min.z),
                vec3(128.0, 128.0, 128.0),
                Vec3::default(),
            ),
            Some(trigger) => {
                let body = game.body_of(trigger.clone());
                (body.bounds.min, body.bounds.max, body.origin)
            }
        };
        // Classic passes absolute air_finished/nextthink as a lifespan, including its extra level-time offset.
        let lifespan = if trigger.is_none() {
            30.0
        } else {
            let entity = game.require_entity(tesla);
            if entity.timestamp != 0.0 {
                entity.timestamp
            } else {
                entity.next_think.unwrap_or(0.0)
            }
        };
        let area = self.spawn_bad_area(
            game,
            add3(origin, min),
            add3(origin, max),
            lifespan,
            Some(tesla.clone()),
        );
        game.require_entity_mut(&tail).team_chain = Some(area);
        true
    }

    /// Throw a mine (`throwMine`).
    pub fn throw_mine(
        &self,
        owner: &ActorId,
        game: &mut Q2GameServices,
        classname: &str,
        start: Vec3,
        direction: Vec3,
        speed: f64,
    ) -> ActorId {
        game.source_callbacks.register(&mine_callbacks());
        let mine = projectile(
            owner,
            game,
            classname,
            start,
            direction,
            speed,
            &format!("models/weapons/g_{classname}/tris.md2"),
            Q2MotionKind::Bounce,
            32,
        );
        let axes = angle_vectors(vector_angles(direction));
        let gravity = if game.options.edition == Q2Edition::Rerelease && classname != "nuke" {
            self.hooks
                .gravity
                .map(|gravity| gravity())
                .or_else(|| game.weapons.inputs.get(owner).map(|input| input.gravity))
                .unwrap_or(800.0)
                / 800.0
        } else {
            1.0
        };
        let up = (200.0 + (game.host.random() * 2.0 - 1.0) * 10.0) * gravity;
        let side = (game.host.random() * 2.0 - 1.0) * 10.0;
        let mut moved = game.body_of(mine.clone());
        moved.velocity = add3(
            add3(scale3(direction, speed as f32), scale3(axes.up, up as f32)),
            scale3(axes.right, side as f32),
        );
        game.write_body(mine.clone(), &moved, false);
        if classname != "nuke" {
            self.player_collision(owner, game, &mine);
        }
        {
            let entity = game.require_entity_mut(&mine);
            entity.render_flags = 0x8000;
            entity.team_master = Some(owner.clone());
            entity.damageable_target = true;
        }
        mine
    }

    /// Fire a prox mine (`fireProx`).
    pub fn fire_prox(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        multiplier: f64,
        speed: f64,
    ) -> ActorId {
        let mine = self.throw_mine(&owner, game, "prox", start, direction, speed);
        let angles = game.body_of(mine.clone()).angles;
        {
            let entity = game.require_entity_mut(&mine);
            entity.clip_mask |= 24;
            entity.flags |= 0x2000;
            entity.damage = 90.0 * multiplier;
            entity.touch = Some(prox_land as Q2Touch);
        }
        let mut moved = game.body_of(mine.clone());
        moved.angles = vec3(angles.x - 90.0, angles.y, angles.z);
        moved.bounds.min = vec3(-6.0, -6.0, -6.0);
        moved.bounds.max = vec3(6.0, 6.0, 6.0);
        game.write_body(mine.clone(), &moved, false);
        if game.options.edition == Q2Edition::Rerelease {
            let now = game.host.now();
            let entity = game.require_entity_mut(&mine);
            entity.classname = "prox_mine".to_string();
            entity.flags = (entity.flags | 0x20000) + 2i64.pow(32);
            entity.timestamp = now + mine_life(multiplier);
            game.schedule(mine.clone(), 0.0, prox_flight as Q2Think);
        } else {
            game.schedule(mine.clone(), mine_life(multiplier), prox_explode as Q2Think);
        }
        publish_projectile(
            mine.clone(),
            game,
            "",
            Some(("q2:weapon_proxlauncher", ProjectileRole::Grenade)),
        );
        mine
    }

    /// Emit a grenade effect (`grenadeEffect`).
    pub fn grenade_effect(&self, entity: &ActorId, game: &mut Q2GameServices) {
        let body = game.body_of(entity.clone());
        let wet = game.host.point_contents(body.origin) & 56 != 0;
        let ground = if body.ground.is_none() { "rocket" } else { "grenade" };
        game.host.emit(Q2PresentationEvent::Effect(
            crate::q2::foundation::host::Q2EffectEvent {
                effect: format!("q2:{ground}_explosion{}", if wet { "_water" } else { "" }),
                origin: add3(body.origin, scale3(body.velocity, -0.02)),
                direction: Vec3::default(),
                count: 0,
                color: 0,
            },
        ));
    }

    /// Fire a tesla mine (`fireTesla`).
    pub fn fire_tesla(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        multiplier: f64,
        speed: f64,
    ) -> ActorId {
        let mine = self.throw_mine(&owner, game, "tesla", start, direction, speed);
        {
            let entity = game.require_entity_mut(&mine);
            entity.damage = 3.0 * multiplier;
            entity.clip_mask |= 24;
            entity.flags |= 0x2000;
            entity.touch = Some(tesla_lava as Q2Touch);
            entity.die = Some(tesla_die as Q2Die);
        }
        if game.options.edition == Q2Edition::Rerelease {
            let entity = game.require_entity_mut(&mine);
            entity.classname = "tesla_mine".to_string();
            entity.clip_mask &= !0x4000000;
            entity.flags = (entity.flags | 0x20000) + 2i64.pow(32);
        }
        let mut moved = game.body_of(mine.clone());
        moved.angles = Vec3::default();
        moved.bounds.min = vec3(-12.0, -12.0, 0.0);
        moved.bounds.max = vec3(12.0, 12.0, 20.0);
        game.write_body(mine.clone(), &moved, false);
        let owned = game.owned_of(mine.clone());
        game.host.combat().create(
            &owned,
            &CombatState {
                health: if game.options.mode == Q2Mode::Deathmatch {
                    20.0
                } else if game.options.edition == Q2Edition::Rerelease {
                    50.0
                } else {
                    30.0
                },
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
        game.require_entity_mut(&mine).wait = game.host.now() + 30.0;
        game.schedule(mine.clone(), 3.0, tesla_open as Q2Think);
        publish_projectile(mine.clone(), game, "", Some(("q2:ammo_tesla", ProjectileRole::Grenade)));
        mine
    }
}

/// Remove a tesla (`removeTesla`).
fn remove_tesla(entity: ActorId, game: &mut Q2GameServices, blow: bool) {
    let owned = game.owned_of(entity.clone());
    game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(false),
            mass: None,
            invulnerable: None,
            team: None,
            no_knockback: None,
        },
    );
    let mut child = game.require_entity(&entity).team_chain.clone();
    while let Some(link) = child
        .clone()
        .and_then(|link| game.entity(&link).map(|entity| entity.actor.id().clone()))
    {
        child = game.require_entity(&link).team_chain.clone();
        game.remove_actor(link);
    }
    let team_master = game.require_entity(&entity).team_master.clone();
    {
        let record = game.require_entity_mut(&entity);
        record.owner = team_master;
        record.enemy = None;
        if blow {
            record.damage *= 50.0;
            record.damage_radius = 200.0;
        }
    }
    let (damage, radius, owner) = {
        let record = game.require_entity(&entity);
        (record.damage, record.damage_radius, record.owner.clone())
    };
    if radius != 0.0 && damage > 150.0 {
        game.sound(&entity, "items/damage3.wav", 3, 1.0, 1.0);
    }
    if let Some(owner) = owner.clone() {
        let origin = game.body_of(entity.clone()).origin;
        weapon_player_noise_for_actor(game, owner, origin, NoiseKind::Impact);
    }
    game.radius_damage(entity.clone(), owner, damage, None, radius, 7, 0, None);
    super::mission_projectiles(game).grenade_effect(&entity, game);
    game.remove_actor(entity);
}

/// Bad area touch (`badAreaTouch`).
fn bad_area_touch(_area: ActorId, _game: &mut Q2GameServices, _contact: TouchContact) {}

/// Prox explode think (`proxExplode`).
fn prox_explode(entity: ActorId, game: &mut Q2GameServices) {
    let field = game
        .require_entity(&entity)
        .team_chain
        .clone()
        .and_then(|chain| game.entity(&chain).map(|entity| entity.actor.id().clone()));
    if let Some(field) = field {
        if game.require_entity(&field).owner.as_ref() == Some(&entity) {
            game.remove_actor(field);
        }
    }
    let team_master = game.require_entity(&entity).team_master.clone();
    let owner = team_master
        .clone()
        .filter(|master| game.host.actors().is_live(master))
        .unwrap_or(entity.clone());
    if let Some(master) = team_master {
        let origin = game.body_of(entity.clone()).origin;
        weapon_player_noise_for_actor(game, master, origin, NoiseKind::Impact);
    }
    if game.require_entity(&entity).damage > 90.0 {
        game.sound(&entity, "items/damage3.wav", 3, 1.0, 1.0);
    }
    if game.host.combat().read(&entity).is_some() {
        let owned = game.owned_of(entity.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(false),
                mass: None,
                invulnerable: None,
                team: None,
                no_knockback: None,
            },
        );
    }
    game.radius_damage(
        entity.clone(),
        Some(owner),
        game.require_entity(&entity).damage,
        Some(entity.clone()),
        192.0,
        Q2_MISSION_PACK_DAMAGE.prox,
        0,
        Some("q2:weapon_proxlauncher".to_string()),
    );
    super::mission_projectiles(game).grenade_effect(&entity, game);
    game.remove_actor(entity);
}

/// Prox die (`proxDie`).
fn prox_die(entity: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    let owned = game.owned_of(entity.clone());
    game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(false),
            mass: None,
            invulnerable: None,
            team: None,
            no_knockback: None,
        },
    );
    let classname = reaction
        .inflictor
        .clone()
        .and_then(|inflictor| game.entity(&inflictor).map(|entity| entity.classname.clone()));
    let expected = if game.options.edition == Q2Edition::Rerelease {
        "prox_mine"
    } else {
        "prox"
    };
    if classname.as_deref() == Some(expected) {
        let frame_seconds = game.host.frame_seconds();
        game.schedule(entity, frame_seconds, prox_explode as Q2Think);
    } else {
        prox_explode(entity, game);
    }
}

/// Prox field touch (`proxField`).
#[allow(unpredictable_function_pointer_comparisons)]
fn prox_field(field: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.host.is_monster(&contact.other) && !game.host.is_player(&contact.other) {
        return;
    }
    let owner = game.require_entity(&field).owner.clone();
    let mine = owner.and_then(|owner| game.entity(&owner).map(|entity| entity.actor.id().clone()));
    let Some(mine) = mine else {
        game.remove_actor(field);
        return;
    };
    let prox_rerelease = game.options.edition == Q2Edition::Rerelease;
    let prox_master = game.require_entity(&mine).team_master.clone();
    let prox_deathmatch = game.options.mode == Q2Mode::Deathmatch;
    let prox_player = game.host.is_player(&contact.other);
    if prox_rerelease
        && (!weapon_can_target(game, prox_master.as_ref(), &contact.other) || !prox_deathmatch && prox_player)
    {
        return;
    }
    if contact.other == mine || game.require_entity(&mine).think == Some(prox_explode as Q2Think) {
        return;
    }
    if game.require_entity(&mine).team_chain.as_ref() != Some(&field) {
        game.remove_actor(field);
        return;
    }
    game.sound(&field, "weapons/proxwarn.wav", 2, 1.0, 1.0);
    game.schedule(mine, 0.5, prox_explode as Q2Think);
}

/// Prox seek think (`proxSeek`).
fn prox_seek(entity: ActorId, game: &mut Q2GameServices) {
    if game.host.now() > game.require_entity(&entity).wait {
        prox_explode(entity, game);
        return;
    }
    {
        let record = game.require_entity_mut(&entity);
        record.frame += 1;
        if record.frame > 13 {
            record.frame = 9;
        }
    }
    game.show(entity.clone());
    game.schedule(entity, 0.1, prox_seek as Q2Think);
}

/// Prox open think (`proxOpen`).
fn prox_open(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).frame != 9 {
        if game.require_entity(&entity).frame == 0 {
            game.sound(&entity, "weapons/proxopen.wav", 2, 1.0, 1.0);
        }
        game.require_entity_mut(&entity).frame += 1;
        game.show(entity.clone());
        let delay = if game.options.edition == Q2Edition::Rerelease {
            0.1
        } else {
            0.05
        };
        game.schedule(entity, delay, prox_open as Q2Think);
        return;
    }
    if game.options.edition == Q2Edition::Classic || game.options.mode == Q2Mode::Deathmatch {
        game.require_entity_mut(&entity).owner = None;
    }
    let motion = game.require_entity(&entity).motion;
    game.set_motion_kind(entity.clone(), motion);
    if let Some(field) = game
        .require_entity(&entity)
        .team_chain
        .clone()
        .and_then(|chain| game.entity(&chain).map(|entity| entity.actor.id().clone()))
    {
        game.require_entity_mut(&field).touch = Some(prox_field as Q2Touch);
    }
    let prox_origin = game.body_of(entity.clone()).origin;
    for actor in game.host.nearby(prox_origin, 202.0) {
        let target = game.entity(&actor).map(|entity| entity.actor.id().clone());
        let rerelease = game.options.edition == Q2Edition::Rerelease;
        let prox_master = game.require_entity(&entity).team_master.clone();
        if rerelease && (actor == entity || !weapon_can_target(game, prox_master.as_ref(), &actor)) {
            continue;
        }
        let newcomer = target
            .as_ref()
            .is_some_and(|target| game.require_entity(target).classname == "prox_mine");
        let living = (game.host.is_monster(&actor)
            || (game.host.is_player(&actor) || rerelease && newcomer)
                && (!rerelease || game.options.mode == Q2Mode::Deathmatch))
            && game
                .host
                .combat()
                .read(&actor)
                .map(|combat| combat.health)
                .unwrap_or(0.0)
                > 0.0;
        if !living
            && !(game.options.mode == Q2Mode::Deathmatch
                && target
                    .as_ref()
                    .is_some_and(|target| player_start(game.require_entity(target), game)))
        {
            continue;
        }
        let seen = match target {
            Some(target) => sight(game, &target, &entity),
            None => sight(game, &entity, &actor),
        };
        if !seen {
            continue;
        }
        game.sound(&entity, "weapons/proxwarn.wav", 2, 1.0, 1.0);
        prox_explode(entity, game);
        return;
    }
    let strong = super::mission_hooks(game).strong_mines;
    let wait = game.host.now()
        + if strong {
            45.0
        } else {
            mine_life(game.require_entity(&entity).damage / 90.0)
        };
    game.require_entity_mut(&entity).wait = wait;
    game.schedule(entity, 0.2, prox_seek as Q2Think);
}

/// Prox land touch (`proxLand`).
fn prox_land(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if contact
        .surface
        .as_ref()
        .map(|surface| surface.native_flags)
        .unwrap_or(0)
        & 4
        != 0
    {
        game.remove_actor(entity);
        return;
    }
    let normal = contact.plane.map(|plane| plane.normal);
    if let Some(normal) = normal {
        let land_origin = game.body_of(entity.clone()).origin;
        if game.host.point_contents(add3(land_origin, scale3(normal, -10.0))) & 24 != 0 {
            prox_explode(entity, game);
            return;
        }
    }
    let other = game.entity(&contact.other).map(|entity| entity.actor.id().clone());
    if game.host.is_monster(&contact.other)
        || game.host.is_player(&contact.other)
        || other
            .as_ref()
            .is_some_and(|other| game.require_entity(other).damageable_target)
    {
        if contact.other
            == game
                .require_entity(&entity)
                .team_master
                .clone()
                .unwrap_or(entity.clone())
        {
            return;
        }
        prox_explode(entity, game);
        return;
    }
    let mut motion = Q2MotionKind::Stationary;
    if contact.other != game.host.world_actor() {
        let Some(normal) = normal else {
            prox_explode(entity, game);
            return;
        };
        let body = game.body_of(entity.clone());
        let out = sub3(body.velocity, scale3(normal, dot3(body.velocity, normal) * 1.5));
        if out.z > 60.0 {
            return;
        }
        let push = other
            .as_ref()
            .is_some_and(|other| game.require_entity(other).motion == Q2MotionKind::Push);
        if !push || normal.z <= 0.7 {
            if normal.z > 0.7 {
                prox_explode(entity, game);
            }
            return;
        }
        motion = Q2MotionKind::Bounce;
    }
    let Some(normal) = normal else {
        prox_explode(entity, game);
        return;
    };
    let field_origin = game.body_of(entity.clone()).origin;
    if game.host.point_contents(field_origin) & 24 != 0 {
        prox_explode(entity, game);
        return;
    }
    let field = game.create("prox_field", BTreeMap::new());
    game.require_entity_mut(&field).owner = Some(entity.clone());
    game.require_entity_mut(&field).team_master = Some(entity.clone());
    let mut moved = game.body_of(field.clone());
    moved.origin = game.body_of(entity.clone()).origin;
    moved.bounds.min = vec3(-96.0, -96.0, -96.0);
    moved.bounds.max = vec3(96.0, 96.0, 96.0);
    game.write_body(field.clone(), &moved, true);
    game.set_solid(field.clone(), Q2Solid::Trigger);
    game.set_motion_kind(field.clone(), Q2MotionKind::Stationary);
    let angles = vector_angles(normal);
    game.require_entity_mut(&entity).angular_velocity = Vec3::default();
    let mut moved = game.body_of(entity.clone());
    moved.velocity = Vec3::default();
    moved.angles = vec3(angles.x + 90.0, angles.y, angles.z);
    game.write_body(entity.clone(), &moved, true);
    game.require_entity_mut(&entity).die = Some(prox_die as Q2Die);
    game.require_entity_mut(&entity).team_chain = Some(field);
    game.require_entity_mut(&entity).touch = None;
    let owned = game.owned_of(entity.clone());
    game.host.combat().create(
        &owned,
        &CombatState {
            health: 20.0,
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
    if game.options.edition == Q2Edition::Rerelease {
        game.require_entity_mut(&entity).projectile = false;
    }
    game.set_motion_kind(entity.clone(), motion);
    let delay = if game.options.edition == Q2Edition::Rerelease {
        0.0
    } else {
        0.05
    };
    game.schedule(entity, delay, prox_open as Q2Think);
}

/// Prox flight think (`proxFlight`).
fn prox_flight(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).timestamp <= game.host.now() {
        prox_explode(entity, game);
        return;
    }
    let angles = vector_angles(game.body_of(entity.clone()).velocity);
    let mut moved = game.body_of(entity.clone());
    moved.angles = vec3(angles.x - 90.0, angles.y, angles.z);
    game.write_body(entity.clone(), &moved, true);
    game.schedule(entity, 0.0, prox_flight as Q2Think);
}

/// Tesla lava touch (`teslaLava`).
fn tesla_lava(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if let Some(plane) = contact.plane {
        let lava_origin = game.body_of(entity.clone()).origin;
        if game.host.point_contents(add3(lava_origin, scale3(plane.normal, -20.0))) & 24 != 0 {
            remove_tesla(entity, game, true);
            return;
        }
    }
    let bounce = if game.host.random() > 0.5 {
        "weapons/hgrenb1a.wav"
    } else {
        "weapons/hgrenb2a.wav"
    };
    game.sound(&entity, bounce, 2, 1.0, 1.0);
}

/// Tesla die (`teslaDie`).
fn tesla_die(entity: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    remove_tesla(entity, game, false);
}

/// Tesla open think (`teslaOpen`).
fn tesla_open(entity: ActorId, game: &mut Q2GameServices) {
    let open_origin = game.body_of(entity.clone()).origin;
    if game.host.point_contents(open_origin) & 24 != 0 {
        remove_tesla(entity, game, false);
        return;
    }
    let mut moved = game.body_of(entity.clone());
    moved.angles = Vec3::default();
    game.write_body(entity.clone(), &moved, true);
    if game.require_entity(&entity).frame == 0 {
        game.sound(&entity, "weapons/teslaopen.wav", 2, 1.0, 1.0);
    }
    game.require_entity_mut(&entity).frame += 1;
    if game.require_entity(&entity).frame > 14 {
        game.require_entity_mut(&entity).frame = 14;
        game.schedule(entity, 0.1, tesla_activate as Q2Think);
        return;
    }
    if game.require_entity(&entity).frame == 10 {
        if let Some(owner) = game.require_entity(&entity).owner.clone() {
            let origin = game.body_of(entity.clone()).origin;
            weapon_player_noise_for_actor(game, owner, origin, NoiseKind::Weapon);
        }
        game.require_entity_mut(&entity).skin = 1;
    } else if game.require_entity(&entity).frame == 12 {
        game.require_entity_mut(&entity).skin = 2;
    } else if game.require_entity(&entity).frame == 14 {
        game.require_entity_mut(&entity).skin = 3;
    }
    game.show(entity.clone());
    game.schedule(entity, 0.1, tesla_open as Q2Think);
}

/// Tesla activate think (`teslaActivate`).
fn tesla_activate(entity: ActorId, game: &mut Q2GameServices) {
    let activate_origin = game.body_of(entity.clone()).origin;
    if game.host.point_contents(activate_origin) & 56 != 0 {
        remove_tesla(entity, game, true);
        return;
    }
    if game.options.mode == Q2Mode::Deathmatch {
        let scan_origin = game.body_of(entity.clone()).origin;
        for actor in game.host.nearby(scan_origin, 192.0) {
            let other = game.entity(&actor).map(|entity| entity.actor.id().clone());
            let other_id = other
                .as_ref()
                .map(|other| game.require_entity(other).actor.id().clone());
            if other
                .as_ref()
                .is_some_and(|other| player_start(game.require_entity(other), game))
                && other_id.as_ref().is_some_and(|other_id| sight(game, other_id, &entity))
            {
                remove_tesla(entity, game, false);
                return;
            }
        }
    }
    let field = game.create("tesla trigger", BTreeMap::new());
    game.require_entity_mut(&field).owner = Some(entity.clone());
    let mut moved = game.body_of(field.clone());
    moved.origin = game.body_of(entity.clone()).origin;
    moved.bounds.min = vec3(-128.0, -128.0, game.body_of(entity.clone()).bounds.min.z);
    moved.bounds.max = vec3(128.0, 128.0, 128.0);
    game.write_body(field.clone(), &moved, true);
    game.set_solid(field.clone(), Q2Solid::Trigger);
    game.set_motion_kind(field.clone(), Q2MotionKind::Stationary);
    let mut moved = game.body_of(entity.clone());
    moved.angles = Vec3::default();
    game.write_body(entity.clone(), &moved, true);
    if game.options.mode == Q2Mode::Deathmatch {
        game.require_entity_mut(&entity).owner = None;
        let motion = game.require_entity(&entity).motion;
        game.set_motion_kind(entity.clone(), motion);
    }
    game.require_entity_mut(&entity).team_chain = Some(field);
    game.require_entity_mut(&entity).timestamp = game.host.now() + 30.0;
    let delay = if game.options.edition == Q2Edition::Rerelease {
        0.1
    } else {
        game.host.frame_seconds()
    };
    game.schedule(entity, delay, tesla_active as Q2Think);
}

/// Tesla active think (`teslaActive`).
fn tesla_active(entity: ActorId, game: &mut Q2GameServices) {
    if game.host.now() > game.require_entity(&entity).timestamp {
        remove_tesla(entity, game, false);
        return;
    }
    let field = game
        .require_entity(&entity)
        .team_chain
        .clone()
        .and_then(|chain| game.entity(&chain).map(|entity| entity.actor.id().clone()))
        .expect("Active source Tesla has no trigger field");
    let field_body = game.body_of(field);
    let min = add3(field_body.origin, field_body.bounds.min);
    let max = add3(field_body.origin, field_body.bounds.max);
    let start = add3(game.body_of(entity.clone()).origin, vec3(0.0, 0.0, 16.0));
    for observation in game.host.actors().observations() {
        let actor = observation.id;
        let body = game.host.bodies().read(&actor);
        let target = game.entity(&actor).map(|entity| entity.actor.id().clone());
        if !game.host.actors().is_live(&entity) {
            return;
        }
        let Some(body) = body else {
            continue;
        };
        if game
            .host
            .combat()
            .read(&actor)
            .map(|combat| combat.health)
            .unwrap_or(0.0)
            < 1.0
        {
            continue;
        }
        if game.host.is_player(&actor) && game.options.mode != Q2Mode::Deathmatch {
            continue;
        }
        if game.options.edition == Q2Edition::Rerelease {
            let prox_master = game.require_entity(&entity).team_master.clone();
            if game.host.is_player(&actor)
                && prox_master.is_some()
                && !weapon_can_target(game, prox_master.as_ref(), &actor)
            {
                continue;
            }
            if game.options.mode != Q2Mode::Deathmatch
                && target.as_ref().is_some_and(|target| {
                    ((game.require_entity(target).flags as f64 / 2f64.powi(32)).trunc() as i64) % 2 != 0
                })
            {
                continue;
            }
        }
        if !game.host.is_player(&actor)
            && !game.host.is_monster(&actor)
            && target
                .as_ref()
                .is_none_or(|target| !game.require_entity(target).damageable_target)
        {
            continue;
        }
        if body.origin.x + body.bounds.max.x < min.x
            || body.origin.x + body.bounds.min.x > max.x
            || body.origin.y + body.bounds.max.y < min.y
            || body.origin.y + body.bounds.min.y > max.y
            || body.origin.z + body.bounds.max.z < min.z
            || body.origin.z + body.bounds.min.z > max.z
        {
            continue;
        }
        let trace = game.host.trace(&Q2TraceRequest {
            start,
            end: body.origin,
            bounds: None,
            ignore: Some(entity.clone()),
            mask: projectile_mask(game),
            exclude: Vec::new(),
        });
        if trace.fraction != 1.0
            && !matches!(&trace.hit, crate::q2::support::contracts::TraceHit::Actor { actor: hit } if hit == &actor)
        {
            continue;
        }
        if game.require_entity(&entity).damage > 3.0 {
            game.sound(&entity, "items/damage3.wav", 3, 1.0, 1.0);
        }
        let knockback = if game.host.is_monster(&actor)
            && target
                .as_ref()
                .map(|target| game.require_entity(target).flags)
                .unwrap_or(0)
                & 3
                == 0
        {
            0.0
        } else {
            8.0
        };
        game.damage(
            actor.clone(),
            entity.clone(),
            game.require_entity(&entity).team_master.clone(),
            game.require_entity(&entity).damage,
            knockback,
            sub3(body.origin, start),
            trace.end,
            match &trace.contact {
                TraceContact::Plane { plane } => plane.normal,
                _ => Vec3::default(),
            },
            Q2_MISSION_PACK_DAMAGE.tesla,
            0,
            None,
        );
        weapon_emit(
            game,
            &Q2WeaponEvent::Beam {
                effect: WeaponBeamEffect::BfgLightning,
                actor: Some(entity.clone()),
                start,
                end: trace.end,
                duration: game.host.frame_seconds(),
            },
        );
    }
    let delay = if game.options.edition == Q2Edition::Rerelease {
        0.1
    } else {
        game.host.frame_seconds()
    };
    game.schedule(entity, delay, tesla_active as Q2Think);
}
