//! Q2 base scenery (`src/content/q2/base/entities/scenery.ts`).
//!
//! Quake II g_misc.c clocks, decorative models and teleporters.

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{add3, scale3, vec3, Bounds};

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::checkpoint::restore_q2_actor;
use crate::q2::foundation::fields::integer_field;
use crate::q2::foundation::host::{
    Q2Die, Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop,
    Q2Think, Q2TraceRequest,
};
use crate::q2::foundation::scenery::kill_q2_box;
use crate::q2::foundation::weapons::vectors::vector_angles;
use crate::q2::support::contracts::{
    AttackProvenance, CombatState, DamageDelivery, DamageRequest, TouchContact, TraceHit,
};

use super::types::Q2BaseEntityHooks;

/// Scenery animation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2AnimationState {
    /// First frame.
    pub first: i32,
    /// End frame (exclusive wrap).
    pub end: i32,
}

/// Clock state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2ClockState {
    /// Current value.
    pub value: i32,
}

/// Animation checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2AnimationEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// First frame.
    pub first: i32,
    /// End frame.
    pub end: i32,
}

/// Clock checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ClockEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Current value.
    pub value: i32,
}

/// Base scenery checkpoint (`Q2BaseSceneryCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BaseSceneryCheckpoint {
    /// Animations.
    pub animations: Vec<Q2AnimationEntry>,
    /// Clocks.
    pub clocks: Vec<Q2ClockEntry>,
}

/// Format clock text (`q2ClockText`).
pub fn q2_clock_text(seconds: i32, style: i32) -> String {
    let sec = format!("{:02}", seconds % 60);
    if style == 0 {
        return format!("{:>2}", seconds);
    }
    if style == 1 {
        return format!("{:>2}:{sec}", seconds / 60);
    }
    let hours = seconds / 3600;
    let minutes = (seconds - hours * 3600) / 60;
    format!("{:>2}:{:02}:{sec}", hours, minutes)
}

/// Read registered hooks.
fn hooks(game: &Q2GameServices) -> Q2BaseEntityHooks {
    game.base_entities
        .hooks
        .expect("Q2 base entity hooks are not registered")
}

/// Set a decorative model (`model`).
fn set_model(actor: ActorId, game: &mut Q2GameServices, path: &str, bounds: Bounds, solid: Q2Solid) {
    game.require_entity_mut(&actor).model = path.to_string();
    let mut moved = game.body_of(actor.clone());
    moved.bounds = bounds;
    game.write_body(actor.clone(), &moved, false);
    game.set_solid(actor.clone(), solid);
    game.show(actor);
}

/// Start an animation (`animate`).
fn animate(actor: ActorId, game: &mut Q2GameServices, first: i32, end: i32, initial_delay: f64) {
    game.base_entities
        .animations
        .insert(actor.clone(), Q2AnimationState { first, end });
    game.require_entity_mut(&actor).frame = first;
    game.show(actor.clone());
    game.schedule(actor, initial_delay, animate_step as _);
}

/// Animation step (`animateStep`).
fn animate_step(actor: ActorId, game: &mut Q2GameServices) {
    let state = game
        .base_entities
        .animations
        .get(&actor)
        .copied()
        .expect("Missing saved scenery animation");
    {
        let entity = game.require_entity_mut(&actor);
        entity.frame += 1;
        if entity.frame >= state.end {
            entity.frame = state.first;
        }
    }
    game.show(actor.clone());
    let frame_seconds = game.host.frame_seconds();
    game.schedule(actor, frame_seconds, animate_step as _);
}

/// Reset a clock (`resetClock`).
fn reset_clock(actor: ActorId, game: &mut Q2GameServices) {
    let (spawnflags, count) = {
        let entity = game.require_entity(&actor);
        (entity.spawnflags, entity.count)
    };
    game.require_entity_mut(&actor).activator = None;
    if spawnflags & 1 != 0 {
        game.base_entities
            .clocks
            .get_mut(&actor)
            .expect("Missing saved Q2 clock state")
            .value = 0;
        game.require_entity_mut(&actor).wait = f64::from(count);
    } else if spawnflags & 2 != 0 {
        game.base_entities
            .clocks
            .get_mut(&actor)
            .expect("Missing saved Q2 clock state")
            .value = count;
        game.require_entity_mut(&actor).wait = 0.0;
    }
}

/// Commander-body use (`commanderUse`).
fn commander_body_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    game.sound(&actor, "tank/pain.wav", 4, 1.0, 1.0);
    let frame_seconds = game.host.frame_seconds();
    game.schedule(actor, frame_seconds, commander_body_collapse as _);
}

/// Remove use (`removeUse`).
fn scenery_remove_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    game.remove_actor(actor);
}

/// Remove die (`removeDie`).
fn scenery_remove_die(
    actor: ActorId,
    game: &mut Q2GameServices,
    _reaction: crate::q2::support::contracts::DeathReaction,
) {
    game.remove_actor(actor);
}

/// Remove think (`removeThink`).
fn scenery_remove_think(actor: ActorId, game: &mut Q2GameServices) {
    game.remove_actor(actor);
}

/// Clock tick (`clockTick`).
fn func_clock_tick(actor: ActorId, game: &mut Q2GameServices) {
    let enemy = game.require_entity(&actor).enemy.clone();
    let mut display = enemy.as_ref().and_then(|enemy| game.entity(enemy));
    if display.is_none() {
        let target = game.require_entity(&actor).target.clone();
        let found = game.targets(&target).first().cloned();
        game.require_entity_mut(&actor).enemy = found.clone();
        display = found.as_ref().and_then(|found| game.entity(found));
    }
    let Some(display) = display else { return };
    let display_actor = display.actor.id().clone();
    let spawnflags = game.require_entity(&actor).spawnflags;
    if spawnflags & 1 != 0 {
        let value = game
            .base_entities
            .clocks
            .get(&actor)
            .copied()
            .expect("Missing saved Q2 clock state")
            .value;
        let style = integer_field(&game.require_entity(&actor).spawn.clone(), "style", 0);
        game.require_entity_mut(&actor).message = q2_clock_text(value, style);
        if let Some(entry) = game.base_entities.clocks.get_mut(&actor) {
            entry.value += 1;
        }
    } else if spawnflags & 2 != 0 {
        let value = game
            .base_entities
            .clocks
            .get(&actor)
            .copied()
            .expect("Missing saved Q2 clock state")
            .value;
        let style = integer_field(&game.require_entity(&actor).spawn.clone(), "style", 0);
        game.require_entity_mut(&actor).message = q2_clock_text(value, style);
        if let Some(entry) = game.base_entities.clocks.get_mut(&actor) {
            entry.value -= 1;
        }
    } else {
        let time = (hooks(game).local_time)();
        game.require_entity_mut(&actor).message = format!("{:>2}:{:02}:{:02}", time.hour, time.minute, time.second);
    }
    let message: String = game.require_entity(&actor).message.chars().take(15).collect();
    game.require_entity_mut(&actor).message = message.clone();
    game.require_entity_mut(&display_actor).message = message;
    game.dispatch_use(display_actor, Some(actor.clone()), Some(actor.clone()));
    let value = game
        .base_entities
        .clocks
        .get(&actor)
        .copied()
        .expect("Missing saved Q2 clock state")
        .value;
    let wait = game.require_entity(&actor).wait;
    if (spawnflags & 1 != 0 && f64::from(value) > wait) || (spawnflags & 2 != 0 && f64::from(value) < wait) {
        let path_target = game
            .require_entity(&actor)
            .spawn
            .values
            .get("pathtarget")
            .cloned()
            .unwrap_or_default();
        if !path_target.is_empty() {
            let entity = game.require_entity(&actor);
            let previous_target = entity.target.clone();
            let previous_message = entity.message.clone();
            let activator = entity.activator.clone();
            let mut authored = entity.authored_target();
            authored.target = path_target;
            authored.message = String::new();
            game.use_targets(&authored, activator.as_ref(), false);
            let entity = game.require_entity_mut(&actor);
            entity.target = previous_target;
            entity.message = previous_message;
        }
        if spawnflags & 8 == 0 || !game.host.actors().is_live(&actor) {
            return;
        }
        reset_clock(actor.clone(), game);
        if spawnflags & 4 != 0 {
            return;
        }
    }
    game.schedule(actor, 1.0, func_clock_tick as _);
}

/// Clock use (`clockUse`).
fn func_clock_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    {
        let entity = game.require_entity_mut(&actor);
        if entity.spawnflags & 8 == 0 {
            entity.use_ = None;
        }
        if entity.activator.is_some() {
            return;
        }
        entity.activator = activator;
    }
    func_clock_tick(actor, game);
}

/// Teleporter touch (`teleporterTouch`).
fn teleporter_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let zero = vec3(0.0, 0.0, 0.0);
    let other = contact.other.clone();
    if !game.host.is_player(&other) {
        return;
    }
    let target = game.require_entity(&actor).target.clone();
    let destination = game.targets(&target).first().cloned();
    let Some(destination) = destination else {
        game.host
            .diagnostic(&format!("Teleporter destination missing: {target}"));
        return;
    };
    let owned = game.host.actors().resolve_owned(&other);
    let body = game.host.bodies().read(&other);
    let (Some(owned), Some(body)) = (owned, body) else {
        return;
    };
    let target_body = game.body_of(destination);
    let origin = add3(target_body.origin, vec3(0.0, 0.0, 10.0));
    game.host.bodies().unlink(&owned);
    let mut moved = body.clone();
    moved.origin = origin;
    moved.velocity = zero;
    moved.angles = zero;
    game.host.bodies().write(&owned, &moved);
    (hooks(game).teleport_player)(other.clone(), game, target_body.origin, target_body.angles);
    let owner = game.require_entity(&actor).owner.clone();
    let source_origin = owner
        .as_ref()
        .and_then(|owner| game.host.bodies().read(owner))
        .map_or_else(|| game.body_of(actor.clone()).origin, |owner_body| owner_body.origin);
    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:player-teleport".to_string(),
        origin: source_origin,
        direction: zero,
        count: 1,
        color: 0,
    }));
    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:player-teleport".to_string(),
        origin,
        direction: zero,
        count: 1,
        color: 0,
    }));
    if game.entity(&other).is_some() {
        kill_q2_box(game, other.clone());
    } else {
        loop {
            let trace = game.host.trace(&Q2TraceRequest {
                start: origin,
                end: origin,
                bounds: Some(body.bounds),
                ignore: Some(other.clone()),
                mask: 0x2010003,
                exclude: Vec::new(),
            });
            let victim = match &trace.hit {
                TraceHit::Actor { actor } => actor.clone(),
                _ => break,
            };
            let mut attack: AttackProvenance = game.attack(actor.clone(), Some(other.clone()), 21, 32, None);
            attack.inflictor = Some(other.clone());
            game.host.combat().apply(&DamageRequest {
                attack,
                target: victim.clone(),
                amount: 100000.0,
                knockback: 0.0,
                direction: zero,
                point: origin,
                normal: zero,
                delivery: DamageDelivery::Direct,
            });
            let check = game.host.trace(&Q2TraceRequest {
                start: origin,
                end: origin,
                bounds: Some(body.bounds),
                ignore: Some(other.clone()),
                mask: 0x2010003,
                exclude: Vec::new(),
            });
            if matches!(&check.hit, TraceHit::Actor { actor } if *actor == victim) {
                break;
            }
        }
    }
    game.host.bodies().link(&owned, None);
}

/// Viper bomb prethink (`bombPrethink`).
fn misc_viper_bomb_prethink(actor: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&actor);
    let (timestamp, movedir) = (entity.timestamp, entity.movedir);
    let now = game.host.now();
    let diff = (timestamp - now).max(-1.0);
    let angles = vector_angles(vec3(
        movedir.x * (1.0 + diff) as f32,
        movedir.y * (1.0 + diff) as f32,
        diff as f32,
    ));
    let current_z = game.body_of(actor.clone()).angles.z;
    let mut moved = game.body_of(actor.clone());
    moved.ground = None;
    moved.angles = vec3(angles.x, angles.y, current_z + 10.0);
    game.write_body(actor, &moved, false);
}

/// Viper bomb touch (`bombTouch`).
fn misc_viper_bomb_touch(actor: ActorId, game: &mut Q2GameServices, _contact: TouchContact) {
    let zero = vec3(0.0, 0.0, 0.0);
    let entity = game.require_entity(&actor);
    let authored = entity.authored_target();
    let activator = entity.activator.clone();
    game.use_targets(&authored, activator.as_ref(), false);
    if !game.host.actors().is_live(&actor) {
        return;
    }
    let body = game.body_of(actor.clone());
    let mut moved = body.clone();
    moved.origin = vec3(body.origin.x, body.origin.y, body.origin.z + body.bounds.min.z + 1.0);
    game.write_body(actor.clone(), &moved, false);
    let damage = game.require_entity(&actor).damage;
    game.radius_damage(
        actor.clone(),
        Some(actor.clone()),
        damage,
        None,
        damage + 40.0,
        27,
        0,
        None,
    );
    let origin = game.body_of(actor.clone()).origin;
    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion2".to_string(),
        origin,
        direction: zero,
        count: 1,
        color: 0,
    }));
    game.remove_actor(actor);
}

/// Commander-body collapse (`commanderCollapse`).
fn commander_body_collapse(actor: ActorId, game: &mut Q2GameServices) {
    let frame = {
        let entity = game.require_entity_mut(&actor);
        entity.frame += 1;
        entity.frame
    };
    game.show(actor.clone());
    if frame == 22 {
        game.sound(&actor, "tank/thud.wav", 4, 1.0, 1.0);
    }
    if frame < 24 {
        let frame_seconds = game.host.frame_seconds();
        game.schedule(actor, frame_seconds, commander_body_collapse as _);
    }
}

/// Commander-body release (`commanderRelease`).
fn commander_body_release(actor: ActorId, game: &mut Q2GameServices) {
    let origin = game.body_of(actor.clone()).origin;
    let mut moved = game.body_of(actor.clone());
    moved.origin = add3(origin, vec3(0.0, 0.0, 2.0));
    game.write_body(actor.clone(), &moved, true);
    game.set_motion_kind(actor, Q2MotionKind::Toss);
}

/// Viper bomb use (`bombUse`).
fn misc_viper_bomb_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    let viper = game
        .entities
        .values()
        .find(|candidate| candidate.classname == "misc_viper")
        .map(|candidate| candidate.actor.id().clone());
    let Some(viper) = viper else {
        game.host.diagnostic("misc_viper_bomb has no misc_viper");
        return;
    };
    let direction = hooks(game).movers.train_direction(viper.clone(), game);
    let speed = game.require_entity(&viper).speed;
    let now = game.host.now();
    {
        let entity = game.require_entity_mut(&actor);
        entity.movedir = direction;
        entity.timestamp = now;
        entity.visible = true;
        entity.use_ = None;
        entity.activator = activator;
        entity.effects |= 16;
    }
    let mut moved = game.body_of(actor.clone());
    moved.velocity = scale3(direction, speed as f32);
    game.write_body(actor.clone(), &moved, false);
    game.set_solid(actor.clone(), Q2Solid::Box);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    game.show(actor.clone());
    let entity = game.require_entity_mut(&actor);
    entity.prethink = Some(misc_viper_bomb_prethink as Q2Think);
    entity.touch = Some(misc_viper_bomb_touch as _);
}

/// Target string use (`stringUse`).
fn target_string_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let entity = game.require_entity(&actor);
    let team = entity.spawn.values.get("team").cloned();
    let message: Vec<char> = entity.message.chars().collect();
    let members: Vec<ActorId> = game.entities.keys().cloned().collect();
    for member in members {
        let entity = game.require_entity(&member);
        let count = entity.count;
        if count == 0 {
            continue;
        }
        let same_team = match team.as_ref() {
            None => member == actor,
            Some(team) => entity.spawn.values.get("team") == Some(team),
        };
        if !same_team {
            continue;
        }
        let character = message.get(count as usize - 1).copied().unwrap_or('\0');
        let frame = if character.is_ascii_digit() {
            (character as u32 - 48) as i32
        } else if character == '-' {
            10
        } else if character == ':' {
            11
        } else {
            12
        };
        game.require_entity_mut(&member).frame = frame;
        game.show(member);
    }
}

/// Spawn a viper bomb (`viperBomb`).
fn spawn_viper_bomb(actor: ActorId, game: &mut Q2GameServices) {
    {
        let entity = game.require_entity_mut(&actor);
        entity.visible = false;
        if entity.damage == 0.0 {
            entity.damage = 1000.0;
        }
    }
    set_model(
        actor.clone(),
        game,
        "models/objects/bomb/tris.md2",
        Bounds {
            min: vec3(-8.0, -8.0, -8.0),
            max: vec3(8.0, 8.0, 8.0),
        },
        Q2Solid::None,
    );
    game.require_entity_mut(&actor).use_ = Some(misc_viper_bomb_use as _);
}

/// Spawn a clock (`clock`).
fn spawn_clock(actor: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&actor);
    let (target_empty, spawnflags, count) = (entity.target.is_empty(), entity.spawnflags, entity.count);
    if target_empty || (spawnflags & 2 != 0 && count == 0) {
        let missing = if spawnflags & 2 != 0 {
            "count or target"
        } else {
            "target"
        };
        game.host.diagnostic(&format!("func_clock without {missing}"));
        game.remove_actor(actor);
        return;
    }
    if spawnflags & 1 != 0 && count == 0 {
        game.require_entity_mut(&actor).count = 3600;
    }
    game.base_entities
        .clocks
        .insert(actor.clone(), Q2ClockState { value: 0 });
    reset_clock(actor.clone(), game);
    if game.require_entity(&actor).spawnflags & 4 != 0 {
        game.require_entity_mut(&actor).use_ = Some(func_clock_use as _);
    } else {
        game.schedule(actor, 1.0, func_clock_tick as _);
    }
}

/// Spawn a teleporter (`teleporter`).
fn spawn_teleporter(actor: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&actor).target.is_empty() {
        game.host.diagnostic("teleporter without a target");
        game.remove_actor(actor);
        return;
    }
    {
        let entity = game.require_entity_mut(&actor);
        entity.skin = 1;
        entity.effects = 0x20000;
    }
    set_model(
        actor.clone(),
        game,
        "models/objects/dmspot/tris.md2",
        Bounds {
            min: vec3(-32.0, -32.0, -24.0),
            max: vec3(32.0, 32.0, -16.0),
        },
        Q2Solid::Box,
    );
    let origin = game.body_of(actor.clone()).origin;
    game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(actor.clone()),
        origin,
        path: "world/amb10.wav".to_string(),
        channel: 0,
        volume: 1.0,
        attenuation: 3.0,
        reliable: false,
        loop_: Q2SoundLoop::Start,
        loop_owner: None,
    }));
    let target = game.require_entity(&actor).target.clone();
    let trigger = game.create("teleporter_trigger", BTreeMap::new());
    {
        let entity = game.require_entity_mut(&trigger);
        entity.target = target;
        entity.owner = Some(actor.clone());
        entity.visible = false;
    }
    let mut moved = game.body_of(trigger.clone());
    moved.origin = origin;
    moved.bounds.min = vec3(-8.0, -8.0, 8.0);
    moved.bounds.max = vec3(8.0, 8.0, 24.0);
    game.write_body(trigger.clone(), &moved, false);
    game.require_entity_mut(&trigger).touch = Some(teleporter_touch as _);
    game.set_solid(trigger, Q2Solid::Trigger);
}

/// Base scenery callbacks (`Q2BaseScenery[callbacks]`).
pub fn scenery_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("q2_base_scenery_animate", animate_step as _);
    callbacks.think.insert("func_clock_think", func_clock_tick as _);
    callbacks
        .think
        .insert("misc_viper_bomb_prethink", misc_viper_bomb_prethink as _);
    callbacks
        .think
        .insert("commander_body_think", commander_body_collapse as _);
    callbacks
        .think
        .insert("commander_body_drop", commander_body_release as _);
    callbacks
        .think
        .insert("q2_base_scenery_remove", scenery_remove_think as _);
    callbacks.use_.insert("func_clock_use", func_clock_use as _);
    callbacks.use_.insert("misc_viper_bomb_use", misc_viper_bomb_use as _);
    callbacks.use_.insert("commander_body_use", commander_body_use as _);
    callbacks.use_.insert("misc_blackhole_use", scenery_remove_use as _);
    callbacks.use_.insert("target_string_use", target_string_use as _);
    callbacks.touch.insert("teleporter_touch", teleporter_touch as _);
    callbacks
        .touch
        .insert("misc_viper_bomb_touch", misc_viper_bomb_touch as _);
    callbacks.die.insert("q2_base_scenery_die", scenery_remove_die as _);
    callbacks
}

/// Capture base scenery (`Q2BaseScenery[capture]`).
pub fn capture_scenery(game: &mut Q2GameServices) -> Q2BaseSceneryCheckpoint {
    let mut animations = Vec::new();
    let mut clocks = Vec::new();
    let actors: Vec<ActorId> = game.entities.keys().cloned().collect();
    for actor in actors {
        let saved = SavedActorId::from(&actor);
        if let Some(state) = game.base_entities.animations.get(&actor).copied() {
            animations.push(Q2AnimationEntry {
                actor: saved,
                first: state.first,
                end: state.end,
            });
        }
        if let Some(state) = game.base_entities.clocks.get(&actor).copied() {
            clocks.push(Q2ClockEntry {
                actor: saved,
                value: state.value,
            });
        }
    }
    Q2BaseSceneryCheckpoint { animations, clocks }
}

/// Restore base scenery (`Q2BaseScenery[restore]`).
pub fn restore_scenery(game: &mut Q2GameServices, checkpoint: &Q2BaseSceneryCheckpoint) {
    game.base_entities.animations = std::collections::HashMap::new();
    game.base_entities.clocks = std::collections::HashMap::new();
    for entry in &checkpoint.animations {
        let actor = restore_q2_actor(game, entry.actor).id().clone();
        if game.entity(&actor).is_none() {
            panic!("Missing saved Q2 scenery");
        }
        game.base_entities.animations.insert(
            actor,
            Q2AnimationState {
                first: entry.first,
                end: entry.end,
            },
        );
    }
    for entry in &checkpoint.clocks {
        let actor = restore_q2_actor(game, entry.actor).id().clone();
        if game.entity(&actor).is_none() {
            panic!("Missing saved Q2 scenery");
        }
        game.base_entities
            .clocks
            .insert(actor, Q2ClockState { value: entry.value });
    }
}

/// Spawn a scenery entity (`Q2BaseScenery[spawn]`).
pub fn spawn_scenery(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&actor).classname.clone();
    match classname.as_str() {
        "viewthing" => {
            game.require_entity_mut(&actor).render_flags = 64;
            set_model(
                actor.clone(),
                game,
                "models/objects/banner/tris.md2",
                Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 32.0),
                },
                Q2Solid::Box,
            );
            animate(actor, game, 0, 7, 0.5);
            true
        }
        "misc_blackhole" => {
            {
                let entity = game.require_entity_mut(&actor);
                entity.render_flags = 32;
                entity.use_ = Some(scenery_remove_use as _);
            }
            set_model(
                actor.clone(),
                game,
                "models/objects/black/tris.md2",
                Bounds {
                    min: vec3(-64.0, -64.0, 0.0),
                    max: vec3(64.0, 64.0, 8.0),
                },
                Q2Solid::None,
            );
            animate(actor, game, 0, 19, 0.2);
            true
        }
        "misc_eastertank" => {
            set_model(
                actor.clone(),
                game,
                "models/monsters/tank/tris.md2",
                Bounds {
                    min: vec3(-32.0, -32.0, -16.0),
                    max: vec3(32.0, 32.0, 32.0),
                },
                Q2Solid::Box,
            );
            animate(actor, game, 254, 293, 0.2);
            true
        }
        "misc_easterchick" | "misc_easterchick2" => {
            let first = if classname == "misc_easterchick" { 208 } else { 248 };
            let end = if classname == "misc_easterchick" { 247 } else { 287 };
            set_model(
                actor.clone(),
                game,
                "models/monsters/bitch/tris.md2",
                Bounds {
                    min: vec3(-32.0, -32.0, 0.0),
                    max: vec3(32.0, 32.0, 32.0),
                },
                Q2Solid::Box,
            );
            animate(actor, game, first, end, 0.2);
            true
        }
        "monster_commander_body" => {
            {
                let entity = game.require_entity_mut(&actor);
                entity.render_flags |= 64;
                entity.flags |= 16;
            }
            set_model(
                actor.clone(),
                game,
                "models/monsters/commandr/tris.md2",
                Bounds {
                    min: vec3(-32.0, -32.0, 0.0),
                    max: vec3(32.0, 32.0, 48.0),
                },
                Q2Solid::Box,
            );
            let owned = game.require_entity(&actor).actor.clone();
            game.host.combat().create(
                &owned,
                &CombatState {
                    health: 0.0,
                    armor: ArmorState {
                        regular: RegularArmorState::None,
                        powered: PoweredProtectionState::None,
                    },
                    mass: 200.0,
                    can_take_damage: true,
                    invulnerable: true,
                    no_knockback: false,
                    team: None,
                },
            );
            game.require_entity_mut(&actor).use_ = Some(commander_body_use as _);
            let delay = 5.0 * game.host.frame_seconds();
            game.schedule(actor, delay, commander_body_release as _);
            true
        }
        "misc_bigviper" => {
            set_model(
                actor,
                game,
                "models/ships/bigviper/tris.md2",
                Bounds {
                    min: vec3(-176.0, -120.0, -24.0),
                    max: vec3(176.0, 120.0, 72.0),
                },
                Q2Solid::Box,
            );
            true
        }
        "misc_viper_bomb" => {
            spawn_viper_bomb(actor, game);
            true
        }
        "light_mine1" | "light_mine2" => {
            let model = if classname == "light_mine1" {
                "models/objects/minelite/light1/tris.md2"
            } else {
                "models/objects/minelite/light2/tris.md2"
            };
            game.require_entity_mut(&actor).model = model.to_string();
            game.link_actor(actor.clone());
            game.show(actor);
            true
        }
        "misc_gib_arm" | "misc_gib_leg" => {
            let model = if classname == "misc_gib_arm" {
                "models/objects/gibs/arm/tris.md2"
            } else {
                "models/objects/gibs/leg/tris.md2"
            };
            let spin = vec3(
                (game.host.random() * 200.0) as f32,
                (game.host.random() * 200.0) as f32,
                (game.host.random() * 200.0) as f32,
            );
            {
                let entity = game.require_entity_mut(&actor);
                entity.model = model.to_string();
                entity.effects |= 2;
                entity.server_flags |= 4;
                entity.angular_velocity = spin;
            }
            let owned = game.require_entity(&actor).actor.clone();
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
                    no_knockback: false,
                    team: None,
                },
            );
            game.require_entity_mut(&actor).die = Some(scenery_remove_die as Q2Die);
            game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
            game.set_solid(actor.clone(), Q2Solid::None);
            game.show(actor.clone());
            game.schedule(actor, 30.0, scenery_remove_think as _);
            true
        }
        "target_character" => {
            game.require_entity_mut(&actor).frame = 12;
            game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
            game.set_solid(actor.clone(), Q2Solid::Brush);
            game.show(actor);
            true
        }
        "target_string" => {
            game.require_entity_mut(&actor).use_ = Some(target_string_use as _);
            true
        }
        "func_clock" => {
            spawn_clock(actor, game);
            true
        }
        "misc_teleporter" => {
            spawn_teleporter(actor, game);
            true
        }
        "misc_teleporter_dest" => {
            game.require_entity_mut(&actor).skin = 0;
            set_model(
                actor,
                game,
                "models/objects/dmspot/tris.md2",
                Bounds {
                    min: vec3(-32.0, -32.0, -24.0),
                    max: vec3(32.0, 32.0, -16.0),
                },
                Q2Solid::Box,
            );
            true
        }
        _ => false,
    }
}
