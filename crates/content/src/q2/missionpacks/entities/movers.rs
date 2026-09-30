//! Rogue movers (`src/content/q2/missionpacks/entities/movers.ts`).
//!
//! Rogue g_func.c plat2 and g_newfnc.c secret doors/force walls
//! (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, scale3, sub3, vec3};

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{
    Q2Blocked, Q2Die, Q2Edition, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent,
    Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2Think, Q2Touch, Q2Use,
};
use crate::q2::foundation::motion::{linear_move_to, LinearMotionScope};
use crate::q2::foundation::scenery::kill_q2_box;
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::{CombatState, DeathReaction, TouchContact};

use super::super::projectiles::common::explode;
use super::super::projectiles::mission_projectiles;
use super::types::{mission_entity_hooks, Q2MissionPackEntityEvent, Q2MissionPackEntityHooks};

/// Plat2 phase (`platformState` phase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2Plat2Phase {
    /// At the top.
    Top,
    /// At the bottom.
    Bottom,
    /// Moving up.
    Up,
    /// Moving down.
    Down,
}

/// Rogue mover callbacks (`Q2RogueMovers::callbacks`).
pub fn rogue_mover_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("plat2_go_up", plat2_go_up as Q2Think);
    callbacks.think.insert("plat2_go_down", plat2_go_down as Q2Think);
    callbacks.think.insert("plat2_hit_top", plat2_hit_top as Q2Think);
    callbacks.think.insert("plat2_hit_bottom", plat2_hit_bottom as Q2Think);
    callbacks.think.insert("fd_secret_move1", fd_secret_move1 as Q2Think);
    callbacks.think.insert("fd_secret_move2", fd_secret_move2 as Q2Think);
    callbacks.think.insert("fd_secret_move3", fd_secret_move3 as Q2Think);
    callbacks.think.insert("fd_secret_move4", fd_secret_move4 as Q2Think);
    callbacks.think.insert("fd_secret_move5", fd_secret_move5 as Q2Think);
    callbacks.think.insert("fd_secret_move6", fd_secret_move6 as Q2Think);
    callbacks.think.insert("fd_secret_done", fd_secret_done as Q2Think);
    callbacks.think.insert("force_wall_think", force_wall_think as Q2Think);
    callbacks.use_.insert("Use_Plat2", use_plat2 as Q2Use);
    callbacks.use_.insert("plat2_activate", plat2_activate as Q2Use);
    callbacks.use_.insert("fd_secret_use", fd_secret_use as Q2Use);
    callbacks.use_.insert("force_wall_use", force_wall_use as Q2Use);
    callbacks.touch.insert("Touch_Plat_Center2", touch_plat_center2 as Q2Touch);
    callbacks.touch.insert("secret_touch", secret_touch as Q2Touch);
    callbacks.die.insert("fd_secret_killed", fd_secret_killed as Q2Die);
    callbacks.blocked.insert("plat2_blocked", plat2_blocked as Q2Blocked);
    callbacks.blocked.insert("secret_blocked", secret_blocked as Q2Blocked);
    callbacks
}

/// Rogue movers (`Q2RogueMovers`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RogueMovers {
    /// Entity hooks.
    pub hooks: Q2MissionPackEntityHooks,
}

impl Q2RogueMovers {
    /// Spawn a Rogue mover (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        game.source_callbacks.register(&rogue_mover_callbacks());
        let classname = game.require_entity(&entity).classname.clone();
        if classname == "func_plat2" {
            spawn_platform(entity, game);
        } else if classname == "func_door_secret2" {
            spawn_secret(entity, game);
        } else if classname == "func_force_wall" {
            spawn_force_wall(entity, game);
        } else {
            return false;
        }
        true
    }

    /// Read platform state (`platformState`).
    pub fn platform_state(
        &self,
        entity: &ActorId,
        game: &Q2GameServices,
    ) -> Option<(Vec3, Vec3, Q2Plat2Phase)> {
        let _ = self;
        let record = game.require_entity(entity);
        if record.classname != "func_plat2" {
            return None;
        }
        let phase = if record.style == 0 {
            Q2Plat2Phase::Top
        } else if record.style == 1 {
            Q2Plat2Phase::Bottom
        } else if record.style == 2 {
            Q2Plat2Phase::Up
        } else {
            Q2Plat2Phase::Down
        };
        Some((record.pos1, record.pos2, phase))
    }
}

/// Play a platform sound (`sound`).
fn plat_sound(entity: &ActorId, game: &mut Q2GameServices, start: bool) {
    if game.require_entity(entity).flags & 1024 != 0 {
        return;
    }
    game.sound(
        entity,
        if start { "plats/pt1_strt.wav" } else { "plats/pt1_end.wav" },
        10,
        1.0,
        3.0,
    );
    game.require_entity_mut(entity).sound = if start {
        "plats/pt1_mid.wav".to_string()
    } else {
        String::new()
    };
    let origin = game.body_of(entity.clone()).origin;
    game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(entity.clone()),
        origin,
        path: "plats/pt1_mid.wav".to_string(),
        channel: 0,
        volume: 1.0,
        attenuation: 3.0,
        reliable: false,
        loop_: if start { Q2SoundLoop::Start } else { Q2SoundLoop::Stop },
        loop_owner: None,
    }));
}

/// Run a platform endpoint (`hit`).
fn plat_hit(entity: ActorId, game: &mut Q2GameServices, top: bool) {
    plat_sound(&entity, game, false);
    game.require_entity_mut(&entity).style = if top { 0 } else { 1 };
    let returning = if top { plat2_go_down as Q2Think } else { plat2_go_up as Q2Think };
    if game.require_entity(&entity).count & 1 != 0 {
        game.require_entity_mut(&entity).count = 4;
        if game.require_entity(&entity).spawnflags & 2 == 0 {
            game.schedule(entity.clone(), 5.0, returning);
        }
        let offset = if game.options.mode == Q2Mode::Deathmatch { 1.0 } else { 2.0 };
        game.require_entity_mut(&entity).timestamp = game.host.now() - offset;
    } else {
        game.require_entity_mut(&entity).count = 0;
        game.require_entity_mut(&entity).timestamp = game.host.now();
        if (game.require_entity(&entity).spawnflags & 4 != 0) != top
            && game.require_entity(&entity).spawnflags & 2 == 0
        {
            game.schedule(entity.clone(), 2.0, returning);
        }
    }
    if !top {
        let areas: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        for area in areas {
            let record = game.require_entity(&area);
            if record.classname == "bad_area" && record.owner == Some(entity.clone()) {
                game.remove_actor(area);
            }
        }
    }
    let authored = game.require_entity(&entity).authored_target();
    let id = entity.clone();
    game.use_targets(&authored, Some(&id), false);
}

/// Platform hit top (`top`).
fn plat2_hit_top(entity: ActorId, game: &mut Q2GameServices) {
    plat_hit(entity, game, true);
}

/// Platform hit bottom (`bottom`).
fn plat2_hit_bottom(entity: ActorId, game: &mut Q2GameServices) {
    plat_hit(entity, game, false);
}

/// Platform go down (`down`).
fn plat2_go_down(entity: ActorId, game: &mut Q2GameServices) {
    plat_sound(&entity, game, true);
    game.require_entity_mut(&entity).style = 3;
    let count = game.require_entity(&entity).count;
    game.require_entity_mut(&entity).count = count | 2;
    let destination = game.require_entity(&entity).pos2;
    linear_move_to(game, LinearMotionScope::Base, entity, destination, plat2_hit_bottom as Q2Think);
}

/// Platform go up (`up`).
fn plat2_go_up(entity: ActorId, game: &mut Q2GameServices) {
    plat_sound(&entity, game, true);
    game.require_entity_mut(&entity).style = 2;
    let count = game.require_entity(&entity).count;
    game.require_entity_mut(&entity).count = count | 2;
    let bounds = game.body_of(entity.clone()).bounds;
    mission_projectiles(game).spawn_bad_area(
        game,
        bounds.min,
        vec3(bounds.max.x, bounds.max.y, bounds.min.z + 64.0),
        0.0,
        Some(entity.clone()),
    );
    let destination = game.require_entity(&entity).pos1;
    linear_move_to(game, LinearMotionScope::Base, entity, destination, plat2_hit_top as Q2Think);
}

/// Operate a platform from a trigger (`operate`).
fn plat_operate(trigger: ActorId, game: &mut Q2GameServices, actor: ActorId) {
    let platform = game.require_entity(&trigger).enemy.clone();
    let entity = platform.as_ref().and_then(|enemy| game.entity(enemy).map(|entity| entity.actor.id().clone()));
    let other = game.host.bodies().read(&actor);
    let (Some(entity), Some(other)) = (entity, other) else {
        return;
    };
    if game.require_entity(&entity).count & 2 != 0
        || game.require_entity(&entity).timestamp + 2.0 > game.host.now()
    {
        return;
    }
    let bounds = game.body_of(trigger);
    let min = add3(bounds.origin, bounds.bounds.min);
    let max = add3(bounds.origin, bounds.bounds.max);
    let center = (min.z + max.z) / 2.0;
    let style = game.require_entity(&entity).style;
    let spawnflags = game.require_entity(&entity).spawnflags;
    let other_state = if style == 0 {
        if (if spawnflags & 32 != 0 { center } else { max.z }) > other.origin.z {
            1
        } else {
            0
        }
    } else if other.origin.z > center {
        0
    } else {
        1
    };
    game.require_entity_mut(&entity).count = 2;
    let mut pause = if game.options.mode == Q2Mode::Deathmatch { 0.3 } else { 0.5 };
    if style != other_state {
        game.require_entity_mut(&entity).count |= 1;
        pause = 0.1;
    }
    game.require_entity_mut(&entity).timestamp = game.host.now();
    game.schedule(entity, pause, if style == 1 { plat2_go_up as Q2Think } else { plat2_go_down as Q2Think });
}

/// Platform center touch (`platTouch`).
fn touch_plat_center2(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if game.host.combat().read(&contact.other).map(|combat| combat.health).unwrap_or(0.0) <= 0.0
        || !game.host.is_player(&contact.other) && !game.host.is_monster(&contact.other)
    {
        return;
    }
    plat_operate(entity, game, contact.other);
}

/// Platform use (`platUse`).
#[allow(unpredictable_function_pointer_comparisons)]
fn use_plat2(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    if game.require_entity(&entity).style > 1
        || game.require_entity(&entity).timestamp + 2.0 > game.host.now()
        || activator.is_none()
    {
        return;
    }
    let activator = activator.expect("plat activator is missing");
    let triggers: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
    for trigger in triggers {
        if game.require_entity(&trigger).touch == Some(touch_plat_center2 as Q2Touch)
            && game.require_entity(&trigger).enemy == Some(entity.clone())
        {
            plat_operate(trigger, game, activator);
            return;
        }
    }
}

/// Platform activate (`activate`).
fn plat2_activate(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    game.require_entity_mut(&entity).use_ = Some(use_plat2 as Q2Use);
    spawn_plat_trigger(entity.clone(), game);
    plat2_go_down(entity, game);
}

/// Platform blocked (`platBlocked`).
fn plat2_blocked(entity: ActorId, game: &mut Q2GameServices, other: ActorId) {
    let Some(body) = game.host.bodies().read(&other) else {
        return;
    };
    if !game.host.is_monster(&other) && !game.host.is_player(&other) {
        game.damage(
            other.clone(),
            entity.clone(),
            Some(entity.clone()),
            100000.0,
            1.0,
            Vec3::default(),
            body.origin,
            Vec3::default(),
            20,
            0,
            None,
        );
        if let Some(target) = game.entity(&other).map(|entity| entity.actor.id().clone()) {
            explode(&target, game, "explosion1");
        }
        return;
    }
    if game.host.combat().read(&other).map(|combat| combat.health).unwrap_or(0.0) < 1.0 {
        game.damage(
            other.clone(),
            entity.clone(),
            Some(entity.clone()),
            100.0,
            1.0,
            Vec3::default(),
            body.origin,
            Vec3::default(),
            20,
            0,
            None,
        );
    }
    let damage = game.require_entity(&entity).damage;
    game.damage(
        other,
        entity.clone(),
        Some(entity.clone()),
        damage,
        1.0,
        Vec3::default(),
        body.origin,
        Vec3::default(),
        20,
        0,
        None,
    );
    if game.require_entity(&entity).style == 2 {
        plat2_go_down(entity, game);
    } else if game.require_entity(&entity).style == 3 {
        plat2_go_up(entity, game);
    }
}

/// Spawn a platform trigger (`trigger`).
fn spawn_plat_trigger(entity: ActorId, game: &mut Q2GameServices) {
    let bounds = game.body_of(entity.clone()).bounds;
    let spawn = game.require_entity(&entity).spawn.clone();
    let lip = number_field(&spawn, "lip", 0.0);
    let mut min_x = bounds.min.x + 25.0;
    let mut min_y = bounds.min.y + 25.0;
    let mut max_x = bounds.max.x - 25.0;
    let mut max_y = bounds.max.y - 25.0;
    let record = game.require_entity(&entity);
    let min_z = bounds.max.z + 8.0 - (record.pos1.z - record.pos2.z + lip as f32);
    let max_z = if record.spawnflags & 1 != 0 {
        min_z + 8.0
    } else {
        bounds.max.z + 8.0
    };
    if max_x - min_x <= 0.0 {
        min_x = (bounds.min.x + bounds.max.x) * 0.5;
        max_x = min_x + 1.0;
    }
    if max_y - min_y <= 0.0 {
        min_y = (bounds.min.y + bounds.max.y) * 0.5;
        max_y = min_y + 1.0;
    }
    let trigger = game.create("plat2_trigger", std::collections::BTreeMap::new());
    game.require_entity_mut(&trigger).enemy = Some(entity);
    game.require_entity_mut(&trigger).touch = Some(touch_plat_center2 as Q2Touch);
    game.require_entity_mut(&trigger).visible = false;
    let mut moved = game.body_of(trigger.clone());
    moved.bounds.min = vec3(min_x - 10.0, min_y - 10.0, min_z);
    moved.bounds.max = vec3(max_x + 10.0, max_y + 10.0, max_z);
    game.write_body(trigger.clone(), &moved, true);
    game.set_solid(trigger, Q2Solid::Trigger);
}

/// Spawn a platform (`spawnPlatform`).
fn spawn_platform(entity: ActorId, game: &mut Q2GameServices) {
    let mut moved = game.body_of(entity.clone());
    moved.angles = Vec3::default();
    game.write_body(entity.clone(), &moved, true);
    game.set_solid(entity.clone(), Q2Solid::Brush);
    game.set_motion_kind(entity.clone(), Q2MotionKind::Push);
    game.require_entity_mut(&entity).blocked = Some(plat2_blocked as Q2Blocked);
    let multiplier = if game.options.mode == Q2Mode::Deathmatch { 2.0 } else { 1.0 };
    {
        let record = game.require_entity_mut(&entity);
        record.speed = (if record.speed == 0.0 { 20.0 } else { record.speed * 0.1 }) * multiplier;
        record.accel = (if record.accel == 0.0 { 5.0 } else { record.accel * 0.1 }) * multiplier;
        record.decel = (if record.decel == 0.0 { 5.0 } else { record.decel * 0.1 }) * multiplier;
        if record.damage == 0.0 {
            record.damage = 2.0;
        }
    }
    let body = game.body_of(entity.clone());
    let spawn = game.require_entity(&entity).spawn.clone();
    let lip = number_field(&spawn, "lip", 0.0);
    let mut height = number_field(&spawn, "height", 0.0);
    if height == 0.0 {
        height = f64::from(body.bounds.max.z - body.bounds.min.z);
    }
    {
        let record = game.require_entity_mut(&entity);
        record.pos1 = body.origin;
        record.pos2 = vec3(body.origin.x, body.origin.y, body.origin.z - height as f32 + lip as f32);
        record.style = 0;
        record.count = 0;
    }
    let rerelease_hold = game.options.edition == Q2Edition::Rerelease
        && game.require_entity(&entity).spawnflags & 8 != 0;
    if !game.require_entity(&entity).targetname.is_empty() && !rerelease_hold {
        game.require_entity_mut(&entity).use_ = Some(plat2_activate as Q2Use);
    } else {
        game.require_entity_mut(&entity).use_ = Some(use_plat2 as Q2Use);
        spawn_plat_trigger(entity.clone(), game);
        if game.require_entity(&entity).spawnflags & 4 == 0 {
            let pos2 = game.require_entity(&entity).pos2;
            let mut moved = game.body_of(entity.clone());
            moved.origin = pos2;
            game.write_body(entity.clone(), &moved, true);
            game.require_entity_mut(&entity).style = 1;
        }
    }
    game.show(entity);
}

/// Secret use (`secretUse`).
fn fd_secret_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    if game.require_entity(&entity).flags & 1024 != 0 {
        return;
    }
    let mut member = Some(entity);
    while let Some(current) = member {
        let destination = game.require_entity(&current).pos1;
        linear_move_to(game, LinearMotionScope::Base, current.clone(), destination, fd_secret_move1 as Q2Think);
        member = game
            .require_entity(&current)
            .team_chain
            .clone()
            .and_then(|chain| game.entity(&chain).map(|entity| entity.actor.id().clone()));
    }
}

/// Secret die (`secretDie`).
fn fd_secret_killed(entity: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    let owned = game.owned_of(entity.clone());
    let max_health = game.require_entity(&entity).max_health;
    game.host.combat().set_health(&owned, max_health);
    let owned = game.owned_of(entity.clone());
    game.host.combat().set_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(false),
            mass: None,
            invulnerable: None,
            team: None,
            no_knockback: None,
        },
    );
    let master = game
        .require_entity(&entity)
        .team_master
        .clone()
        .and_then(|master| game.entity(&master).map(|entity| entity.actor.id().clone()));
    if game.require_entity(&entity).flags & 1024 != 0
        && master.as_ref().is_some_and(|master| {
            game.host.combat().read(master).is_some_and(|combat| combat.can_take_damage)
        })
    {
        let master = master.expect("secret master is missing");
        let owned = game.owned_of(master.clone());
        let mut reaction = reaction;
        reaction.pain.this = owned;
        fd_secret_killed(master, game, reaction);
    } else {
        fd_secret_use(entity, game, reaction.inflictor, reaction.pain.attacker);
    }
}

/// Secret move 1 (`secret1`).
fn fd_secret_move1(entity: ActorId, game: &mut Q2GameServices) {
    game.schedule(entity, 1.0, fd_secret_move2 as Q2Think);
}

/// Secret move 2 (`secret2`).
fn fd_secret_move2(entity: ActorId, game: &mut Q2GameServices) {
    let destination = game.require_entity(&entity).pos2;
    linear_move_to(game, LinearMotionScope::Base, entity, destination, fd_secret_move3 as Q2Think);
}

/// Secret move 3 (`secret3`).
fn fd_secret_move3(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).spawnflags & 1 != 0 {
        return;
    }
    let wait = game.require_entity(&entity).wait;
    game.schedule(entity, wait, fd_secret_move4 as Q2Think);
}

/// Secret move 4 (`secret4`).
fn fd_secret_move4(entity: ActorId, game: &mut Q2GameServices) {
    let destination = game.require_entity(&entity).pos1;
    linear_move_to(game, LinearMotionScope::Base, entity, destination, fd_secret_move5 as Q2Think);
}

/// Secret move 5 (`secret5`).
fn fd_secret_move5(entity: ActorId, game: &mut Q2GameServices) {
    game.schedule(entity, 1.0, fd_secret_move6 as Q2Think);
}

/// Secret move 6 (`secret6`).
fn fd_secret_move6(entity: ActorId, game: &mut Q2GameServices) {
    let destination = game.require_entity(&entity).movedir;
    linear_move_to(game, LinearMotionScope::Base, entity, destination, fd_secret_done as Q2Think);
}

/// Secret done (`secretDone`).
fn fd_secret_done(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).targetname.is_empty()
        || game.require_entity(&entity).spawnflags & 16 != 0
    {
        let owned = game.owned_of(entity.clone());
        game.host.combat().set_health(&owned, 1.0);
        let owned = game.owned_of(entity.clone());
        game.host.combat().set_traits(
            &owned,
            &crate::q2::support::contracts::CombatTraitChanges {
                can_take_damage: Some(true),
                mass: None,
                invulnerable: None,
                team: None,
                no_knockback: None,
            },
        );
        game.require_entity_mut(&entity).die = Some(fd_secret_killed as Q2Die);
    }
}

/// Secret blocked (`secretBlocked`).
fn secret_blocked(entity: ActorId, game: &mut Q2GameServices, other: ActorId) {
    let body = game.host.bodies().read(&other);
    if game.require_entity(&entity).flags & 1024 == 0 {
        if let Some(body) = body {
            let damage = game.require_entity(&entity).damage;
            game.damage(
                other,
                entity.clone(),
                Some(entity),
                damage,
                0.0,
                Vec3::default(),
                body.origin,
                Vec3::default(),
                20,
                0,
                None,
            );
        }
    }
}

/// Secret touch (`secretTouch`).
fn secret_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.host.is_player(&contact.other)
        || game.host.combat().read(&contact.other).map(|combat| combat.health).unwrap_or(0.0) <= 0.0
        || game.require_entity(&entity).timestamp > game.host.now()
    {
        return;
    }
    game.require_entity_mut(&entity).timestamp = game.host.now() + 2.0;
    if game.require_entity(&entity).message.is_empty() {
        return;
    }
    let message = game.require_entity(&entity).message.clone();
    game.host.emit(Q2PresentationEvent::CenterPrint {
        actor: contact.other,
        text: message,
        instant: false,
        duration_seconds: None,
    });
}

/// Spawn a secret door (`spawnSecret`).
fn spawn_secret(entity: ActorId, game: &mut Q2GameServices) {
    let angles = game.body_of(entity.clone()).angles;
    let axes = angle_vectors(angles);
    let mut moved = game.body_of(entity.clone());
    moved.angles = Vec3::default();
    game.write_body(entity.clone(), &moved, true);
    game.set_solid(entity.clone(), Q2Solid::Brush);
    game.set_motion_kind(entity.clone(), Q2MotionKind::Push);
    let body = game.body_of(entity.clone());
    let size = sub3(body.bounds.max, body.bounds.min);
    if ![0.0, 90.0, 180.0, 270.0].contains(&angles.y) {
        game.host.diagnostic("Secret door not at 0,90,180,270!");
        game.remove_actor(entity);
        return;
    }
    let spawnflags = game.require_entity(&entity).spawnflags;
    let forward = scale3(
        axes.forward,
        (if angles.y == 0.0 || angles.y == 180.0 { size.x } else { size.y })
            * (if spawnflags & 64 != 0 { 1.0 } else { -1.0 }),
    );
    let right = scale3(
        axes.right,
        (if angles.y == 0.0 || angles.y == 180.0 { size.y } else { size.x })
            * (if spawnflags & 32 != 0 { 1.0 } else { -1.0 }),
    );
    {
        let record = game.require_entity_mut(&entity);
        record.movedir = body.origin;
        record.pos1 = add3(
            body.origin,
            if spawnflags & 4 != 0 { forward } else { right },
        );
    }
    let pos1 = game.require_entity(&entity).pos1;
    game.require_entity_mut(&entity).pos2 = add3(
        pos1,
        if spawnflags & 4 != 0 { right } else { forward },
    );
    if game.require_entity(&entity).damage == 0.0 {
        game.require_entity_mut(&entity).damage = 2.0;
    }
    if game.require_entity(&entity).wait == 0.0 {
        game.require_entity_mut(&entity).wait = 5.0;
    }
    game.require_entity_mut(&entity).speed = 50.0;
    game.require_entity_mut(&entity).accel = 50.0;
    game.require_entity_mut(&entity).decel = 50.0;
    game.require_entity_mut(&entity).touch = Some(secret_touch as Q2Touch);
    game.require_entity_mut(&entity).blocked = Some(secret_blocked as Q2Blocked);
    game.require_entity_mut(&entity).use_ = Some(fd_secret_use as Q2Use);
    if game.require_entity(&entity).targetname.is_empty()
        || game.require_entity(&entity).spawnflags & 16 != 0
    {
        game.require_entity_mut(&entity).max_health = 1.0;
        game.require_entity_mut(&entity).die = Some(fd_secret_killed as Q2Die);
        let owned = game.owned_of(entity.clone());
        game.host.combat().create(
            &owned,
            &CombatState {
                health: 1.0,
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
    }
    game.show(entity);
}

/// Force wall think (`forceThink`).
fn force_wall_think(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).wait == 0.0 {
        let record = game.require_entity(&entity);
        let (pos1, pos2, style) = (record.pos1, record.pos2, record.style);
        (mission_entity_hooks(game).emit)(Q2MissionPackEntityEvent::ForceWall {
            start: pos1,
            end: pos2,
            color: style,
        });
    }
    game.schedule(entity, 0.1, force_wall_think as Q2Think);
}

/// Force wall use (`forceUse`).
fn force_wall_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    if game.require_entity(&entity).wait == 0.0 {
        game.require_entity_mut(&entity).wait = 1.0;
        game.cancel_actor(entity.clone());
        game.set_solid(entity, Q2Solid::None);
        return;
    }
    game.require_entity_mut(&entity).wait = 0.0;
    game.schedule(entity.clone(), 0.1, force_wall_think as Q2Think);
    game.set_solid(entity.clone(), Q2Solid::Brush);
    kill_q2_box(game, entity.clone());
    game.link_actor(entity);
}

/// Spawn a force wall (`spawnForceWall`).
fn spawn_force_wall(entity: ActorId, game: &mut Q2GameServices) {
    game.set_solid(entity.clone(), Q2Solid::Brush);
    let body = game.body_of(entity.clone());
    let min = add3(body.origin, body.bounds.min);
    let max = add3(body.origin, body.bounds.max);
    let middle = scale3(add3(min, max), 0.5);
    if max.x - min.x > max.y - min.y {
        game.require_entity_mut(&entity).pos1 = vec3(min.x, middle.y, max.z);
        game.require_entity_mut(&entity).pos2 = vec3(max.x, middle.y, max.z);
    } else {
        game.require_entity_mut(&entity).pos1 = vec3(middle.x, min.y, max.z);
        game.require_entity_mut(&entity).pos2 = vec3(middle.x, max.y, max.z);
    }
    if game.require_entity(&entity).style == 0 {
        game.require_entity_mut(&entity).style = 208;
    }
    game.require_entity_mut(&entity).wait = 1.0;
    game.require_entity_mut(&entity).use_ = Some(force_wall_use as Q2Use);
    game.require_entity_mut(&entity).visible = false;
    game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
    if game.require_entity(&entity).spawnflags & 1 != 0 {
        game.schedule(entity.clone(), 0.1, force_wall_think as Q2Think);
    } else {
        game.set_solid(entity.clone(), Q2Solid::None);
    }
    game.show(entity);
}
