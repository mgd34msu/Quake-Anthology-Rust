//! Q2 triggers and point targets (`src/content/q2/foundation/targets.ts`).
//!
//! Adapted from game/g_trigger.c, g_target.c, g_misc.c and their rerelease variants.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{dot3, sub3, vec3, Vec3};

use super::callbacks::{free_q2_entity, Q2CallbackDefinitions};
use super::fields::{integer_field, movedir, number_field};
use super::host::{
    Q2Edition, Q2EffectEvent, Q2GameServices, Q2ItemNameFn, Q2LandmarkCarry, Q2Mode, Q2PresentationEvent, Q2Solid,
    Q2SoundEvent, Q2SoundLoop, Q2SpawnFn, SpawnModule,
};
use super::shadow_lights::{dynamic_light_use, spawn_q2_shadow_light};
use crate::q2::support::contracts::{TouchContact, TransitionIntent};

/// Multi wait think (`multi_wait`).
fn multi_wait(this: ActorId, game: &mut Q2GameServices) {
    game.cancel_actor(this);
}

/// Fire a multi trigger (`multi`).
fn trigger_multi(this: ActorId, game: &mut Q2GameServices, activator: Option<ActorId>) {
    if game.require_entity(&this).next_think.is_some() {
        return;
    }
    game.require_entity_mut(&this).activator = activator.clone();
    let authored = game.require_entity(&this).authored_target();
    game.use_targets(&authored, activator.as_ref(), false);
    if !game.host.actors().is_live(&this) {
        return;
    }
    let wait = game.require_entity(&this).wait;
    if wait > 0.0 {
        game.schedule(this, wait, multi_wait);
    } else {
        game.require_entity_mut(&this).touch = None;
        let frame_seconds = game.host.frame_seconds();
        game.schedule(this, frame_seconds, free_q2_entity);
    }
}

/// Trigger multiple spawn (`triggerMultiple`).
fn trigger_multiple(this: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&this).classname == "trigger_once" {
        let entity = game.require_entity_mut(&this);
        if entity.spawnflags & 1 != 0 {
            entity.spawnflags = entity.spawnflags & !1 | 4;
        }
        entity.wait = -1.0;
    } else if game.require_entity(&this).wait == 0.0 {
        game.require_entity_mut(&this).wait = 0.2;
    }
    let angles = game.body_of(this.clone()).angles;
    if angles.x != 0.0 || angles.y != 0.0 || angles.z != 0.0 {
        let moved = movedir(angles);
        game.require_entity_mut(&this).movedir = moved;
    }
    let entity = game.require_entity_mut(&this);
    entity.visible = false;
    entity.touch = Some(touch_multi);
    entity.use_ = Some(use_multi);
    let solid = if entity.spawnflags & 4 != 0 {
        Q2Solid::None
    } else {
        Q2Solid::Trigger
    };
    game.set_solid(this, solid);
}

/// Target speaker spawn (`speaker`).
fn target_speaker(this: ActorId, game: &mut Q2GameServices) {
    let spawn = game.require_entity(&this).spawn.clone();
    let Some(noise) = spawn.values.get("noise").cloned() else {
        game.host.diagnostic("target_speaker has no noise");
        return;
    };
    let noise = if noise.contains(".wav") {
        noise
    } else {
        format!("{noise}.wav")
    };
    let volume = number_field(&spawn, "volume", 0.0);
    let volume = if volume == 0.0 { 1.0 } else { volume };
    let authored = number_field(&spawn, "attenuation", 0.0);
    let rerelease_loop = game.options.edition == Q2Edition::Rerelease && game.require_entity(&this).spawnflags & 3 != 0;
    let attenuation = if authored == -1.0 {
        if rerelease_loop {
            -1.0
        } else {
            0.0
        }
    } else if authored == 0.0 {
        if rerelease_loop {
            3.0
        } else {
            1.0
        }
    } else {
        authored
    };
    let entity = game.require_entity_mut(&this);
    entity.noise.clone_from(&noise);
    entity.volume = volume;
    entity.attenuation = attenuation;
    entity.sound = if entity.spawnflags & 1 != 0 {
        noise
    } else {
        String::new()
    };
    if !game.require_entity(&this).sound.is_empty() {
        emit_speaker(this.clone(), game, Q2SoundLoop::Start);
    }
    game.require_entity_mut(&this).use_ = Some(use_target_speaker);
    game.link_actor(this);
}

/// Func timer spawn (`timer`).
fn func_timer(this: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&this).wait == 0.0 {
        game.require_entity_mut(&this).wait = 1.0;
    }
    let spawn = game.require_entity(&this).spawn.clone();
    let wait = game.require_entity(&this).wait;
    let frame_seconds = game.host.frame_seconds();
    let random = number_field(&spawn, "random", 0.0).min(wait - frame_seconds);
    let entity = game.require_entity_mut(&this);
    entity.random = random;
    entity.use_ = Some(func_timer_use);
    if game.require_entity(&this).spawnflags & 1 != 0 {
        let delay = game.require_entity(&this).delay;
        let wait = game.require_entity(&this).wait;
        let random = game.require_entity(&this).random;
        let pausetime = number_field(&spawn, "pausetime", 0.0);
        game.require_entity_mut(&this).activator = Some(this.clone());
        let jitter = (game.host.random() * 2.0 - 1.0) * random;
        game.schedule(this, 1.0 + pausetime + delay + wait + jitter, func_timer_think);
    }
}

/// Target changelevel spawn (`changeLevel`).
fn target_changelevel(this: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&this).map.is_empty() {
        game.host.diagnostic("target_changelevel has no map");
        game.remove_actor(this);
        return;
    }
    let entity = game.require_entity(&this);
    if game.options.map_name.to_lowercase() == "fact1" && entity.map.to_lowercase() == "fact3" {
        game.require_entity_mut(&this).map = "fact3$secret1".to_string();
    }
    game.require_entity_mut(&this).use_ = Some(use_target_changelevel);
}

/// Unrotate a landmark-relative vector (`unrotateQ2Landmark`).
///
/// Rerelease landmark rotation order is X(pitch), Y(roll), then Z(yaw).
pub fn unrotate_q2_landmark(vector: Vec3, angles: Vec3) -> Vec3 {
    let pitch = -angles.x * std::f32::consts::PI / 180.0;
    let roll = -angles.z * std::f32::consts::PI / 180.0;
    let yaw = -angles.y * std::f32::consts::PI / 180.0;
    let x = vec3(
        vector.x,
        vector.y * pitch.cos() - vector.z * pitch.sin(),
        vector.y * pitch.sin() + vector.z * pitch.cos(),
    );
    let y = vec3(
        x.x * roll.cos() + x.z * roll.sin(),
        x.y,
        -x.x * roll.sin() + x.z * roll.cos(),
    );
    vec3(
        y.x * yaw.cos() - y.y * yaw.sin(),
        y.x * yaw.sin() + y.y * yaw.cos(),
        y.z,
    )
}

/// Target spawn (`createQ2TargetModule spawn`).
fn spawn_target(actor: ActorId, game: &mut Q2GameServices) -> bool {
    match game.require_entity(&actor).classname.as_str() {
        "worldspawn" => {
            game.require_entity_mut(&actor).model = "*0".to_string();
            game.set_solid(actor.clone(), Q2Solid::Brush);
            for _ in 0..8 {
                let body = game.create("bodyque", BTreeMap::new());
                let entity = game.require_entity_mut(&body);
                entity.visible = false;
                entity.server_flags = 1;
            }
            let track = game
                .require_entity(&actor)
                .spawn
                .values
                .get("sounds")
                .cloned()
                .unwrap_or_else(|| "0".to_string());
            game.host_emit(Q2PresentationEvent::Music { track });
            true
        }
        "info_player_start"
        | "info_player_coop"
        | "info_player_deathmatch"
        | "info_player_intermission"
        | "info_notnull"
        | "info_landmark"
        | "func_group" => true,
        "info_null" => {
            game.remove_actor(actor);
            true
        }
        "light" => {
            let spawn = game.require_entity(&actor).spawn.clone();
            let style = integer_field(&spawn, "style", 0);
            if game.require_entity(&actor).targetname.is_empty() || game.options.mode == Q2Mode::Deathmatch {
                game.remove_actor(actor);
                return true;
            }
            if style >= 32 {
                game.require_entity_mut(&actor).use_ = Some(light_use);
                let pattern = if game.require_entity(&actor).spawnflags & 1 != 0 {
                    "a"
                } else {
                    "m"
                };
                game.host_emit(Q2PresentationEvent::LightStyle {
                    style,
                    pattern: pattern.to_string(),
                });
            }
            true
        }
        "dynamic_light" => {
            spawn_q2_shadow_light(actor, game);
            true
        }
        "target_speaker" => {
            target_speaker(actor, game);
            true
        }
        "func_timer" => {
            func_timer(actor, game);
            true
        }
        "trigger_once" | "trigger_multiple" => {
            trigger_multiple(actor, game);
            true
        }
        "trigger_relay" => {
            game.require_entity_mut(&actor).use_ = Some(trigger_relay_use);
            true
        }
        "trigger_always" => {
            let delay = game.require_entity(&actor).delay.max(0.2);
            game.require_entity_mut(&actor).delay = delay;
            let authored = game.require_entity(&actor).authored_target();
            game.use_targets(&authored, Some(&actor), false);
            true
        }
        "trigger_counter" => {
            let entity = game.require_entity_mut(&actor);
            entity.wait = -1.0;
            if entity.count == 0 {
                entity.count = 2;
            }
            entity.use_ = Some(trigger_counter_use);
            true
        }
        "trigger_key" => {
            let key_name = game.require_entity(&actor).spawn.values.get("item").cloned();
            let Some(key_name) = key_name else {
                game.host.diagnostic("trigger_key has no item");
                return true;
            };
            let pickup = game.item_name(&key_name);
            if pickup.is_none() || game.require_entity(&actor).target.is_empty() {
                game.host
                    .diagnostic(&format!("Q2 trigger_key has unknown item or no target: {key_name}"));
                return true;
            }
            game.require_entity_mut(&actor).use_ = Some(trigger_key_use);
            true
        }
        "target_help" => {
            if game.options.mode == Q2Mode::Deathmatch || game.require_entity(&actor).message.is_empty() {
                game.remove_actor(actor);
            } else {
                game.require_entity_mut(&actor).use_ = Some(use_target_help);
            }
            true
        }
        "target_secret" | "target_goal" => {
            if game.options.mode == Q2Mode::Deathmatch {
                game.remove_actor(actor);
                return true;
            }
            let secret = game.require_entity(&actor).classname == "target_secret";
            if secret {
                game.counters.total_secrets += 1;
            } else {
                game.counters.total_goals += 1;
            }
            game.require_entity_mut(&actor).use_ = Some(use_target_secret_or_goal);
            true
        }
        "target_changelevel" => {
            target_changelevel(actor, game);
            true
        }
        "target_explosion" => {
            game.require_entity_mut(&actor).use_ = Some(use_target_explosion);
            true
        }
        "target_splash" => {
            if game.require_entity(&actor).count == 0 {
                game.require_entity_mut(&actor).count = 32;
            }
            let moved = movedir(game.body_of(actor.clone()).angles);
            let entity = game.require_entity_mut(&actor);
            entity.movedir = moved;
            entity.use_ = Some(use_target_splash);
            true
        }
        "target_poi" => {
            if game.options.edition != Q2Edition::Rerelease {
                return false;
            }
            game.require_entity_mut(&actor).use_ = Some(use_target_poi);
            true
        }
        "func_areaportal" => {
            game.require_entity_mut(&actor).use_ = Some(use_areaportal);
            true
        }
        _ => false,
    }
}

/// Multi touch (`Touch_Multi`).
fn touch_multi(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if game.require_entity(&this).solid != Q2Solid::Trigger {
        return;
    }
    let spawnflags = game.require_entity(&this).spawnflags;
    if game.host.is_player(&contact.other) {
        if spawnflags & 2 != 0 {
            return;
        }
    } else if game.host.is_monster(&contact.other) {
        if spawnflags & 1 == 0 {
            return;
        }
    } else {
        return;
    }
    if let Some(body) = game.host.bodies().read(&contact.other) {
        let trigger_dir = game.require_entity(&this).movedir;
        if dot3(movedir(body.angles), trigger_dir) < 0.0 {
            return;
        }
    }
    trigger_multi(this, game, Some(contact.other));
}

/// Multi use (`Use_Multi`).
fn use_multi(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    if game.require_entity(&this).solid == Q2Solid::None {
        game.set_solid(this, Q2Solid::Trigger);
        return;
    }
    trigger_multi(this, game, activator);
}

/// Convert source loop distance to the shared Q2 loop coefficient of 0.003
/// (`speakerLoopAttenuation`).
fn speaker_loop_attenuation(this: &ActorId, game: &mut Q2GameServices) -> f64 {
    if game.options.edition == Q2Edition::Classic {
        return 1.0;
    }
    let attenuation = game.require_entity(this).attenuation;
    if attenuation == -1.0 {
        return 0.0;
    }
    if attenuation > 0.0 && attenuation != 3.0 {
        attenuation / 5.0
    } else {
        1.0
    }
}

/// Emit a speaker sound (`emitSpeaker`).
fn emit_speaker(this: ActorId, game: &mut Q2GameServices, operation: Q2SoundLoop) {
    let entity = game.require_entity(&this).clone();
    let origin = game.body_of(this.clone()).origin;
    let once = operation == Q2SoundLoop::Once;
    let attenuation = if once {
        entity.attenuation
    } else {
        speaker_loop_attenuation(entity.actor.id(), game)
    };
    game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(this),
        origin,
        path: entity.noise,
        channel: 2,
        volume: if once { entity.volume } else { 1.0 },
        attenuation,
        reliable: entity.spawnflags & 4 != 0,
        loop_: operation,
        loop_owner: None,
    }));
}

/// Target speaker use (`Use_Target_Speaker`).
fn use_target_speaker(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    if game.require_entity(&this).spawnflags & 3 != 0 {
        let noise = game.require_entity(&this).noise.clone();
        let entity = game.require_entity_mut(&this);
        entity.sound = if entity.sound.is_empty() { noise } else { String::new() };
        let start = !game.require_entity(&this).sound.is_empty();
        emit_speaker(this, game, if start { Q2SoundLoop::Start } else { Q2SoundLoop::Stop });
        return;
    }
    emit_speaker(this, game, Q2SoundLoop::Once);
}

/// Func timer think (`func_timer_think`).
fn func_timer_think(this: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&this).clone();
    game.use_targets(&entity.authored_target(), entity.activator.as_ref(), false);
    if !game.host.actors().is_live(&this) {
        return;
    }
    let entity = game.require_entity(&this).clone();
    let jitter = (game.host.random() * 2.0 - 1.0) * entity.random;
    game.schedule(this, entity.wait + jitter, func_timer_think);
}

/// Func timer use (`func_timer_use`).
fn func_timer_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    game.require_entity_mut(&this).activator = activator;
    if game.require_entity(&this).next_think.is_some() {
        game.cancel_actor(this);
        return;
    }
    if game.require_entity(&this).delay != 0.0 {
        let delay = game.require_entity(&this).delay;
        game.schedule(this, delay, func_timer_think);
    } else {
        func_timer_think(this, game);
    }
}

/// Target changelevel use (`use_target_changelevel`).
fn use_target_changelevel(
    this: ActorId,
    game: &mut Q2GameServices,
    other: Option<ActorId>,
    activator: Option<ActorId>,
) {
    if game.require_entity(&this).transition_started {
        return;
    }
    let map = game.require_entity(&this).map.clone();
    if game.options.mode == Q2Mode::Singleplayer {
        let player = game.host.players().into_iter().next();
        if let Some(player) = player {
            let health = game
                .host
                .combat()
                .read(&player)
                .map(|state| state.health)
                .unwrap_or(0.0);
            if health <= 0.0 {
                return;
            }
        }
    }
    let gib_intruder = match other.clone() {
        Some(other)
            if game.options.mode == Q2Mode::Deathmatch
                && game.deathmatch_flags() & 4096 == 0
                && game.entity(&other).map(|entity| entity.classname.as_str()) != Some("worldspawn") =>
        {
            Some(other)
        }
        _ => None,
    };
    if let Some(other) = gib_intruder {
        let body = game.host.bodies().read(&other);
        let health = game.host.combat().read(&other);
        if let (Some(body), Some(_)) = (body, health) {
            let max_health = game.entity(&other).map(|entity| entity.max_health).unwrap_or(0.0);
            let max_health = if max_health == 0.0 { 100.0 } else { max_health };
            let zero = vec3(0.0, 0.0, 0.0);
            game.damage(
                other,
                this.clone(),
                Some(this),
                10.0 * max_health,
                1000.0,
                zero,
                body.origin,
                zero,
                28,
                0,
                None,
            );
        }
        return;
    }
    if map.contains('*') {
        game.counters.server_flags &= !255;
    }
    let (destination, spawn_point) = match map.find('$') {
        Some(index) => (map[..index].to_string(), map[index + 1..].to_string()),
        None => (map.clone(), String::new()),
    };
    let target = game.require_entity(&this).target.clone();
    let landmark = if activator.is_none() || game.options.mode == Q2Mode::Deathmatch {
        None
    } else {
        game.pick_target(&target)
    };
    let player_body = activator
        .as_ref()
        .and_then(|activator| game.host.bodies().read(activator));
    let player_view = activator
        .as_ref()
        .and_then(|activator| game.host.player_view_state(activator));
    if let (Some(activator), Some(landmark), Some(player_body), Some(player_view)) =
        (activator.clone(), landmark.clone(), player_body, player_view)
    {
        let reference = game.body_of(landmark.clone());
        let name = game.require_entity(&landmark).targetname.clone();
        let carry = Q2LandmarkCarry {
            player: activator,
            name,
            relative_origin: unrotate_q2_landmark(sub3(player_body.origin, reference.origin), reference.angles),
            relative_velocity: unrotate_q2_landmark(player_view.old_velocity, reference.angles),
            relative_view_angles: sub3(player_view.view_angles, reference.angles),
        };
        let server_flags = game.counters.server_flags;
        game.host.prepare_level_change(&map, Some(&carry), server_flags);
    } else {
        let server_flags = game.counters.server_flags;
        game.host.prepare_level_change(&map, None, server_flags);
    }
    game.require_entity_mut(&this).transition_started = true;
    let campaign = game.options.campaign.clone();
    game.host.transition(TransitionIntent::campaign_level(
        campaign,
        format!("q2:{destination}"),
        spawn_point,
        Vec::new(),
        activator,
    ));
}

/// Light use (`light_use`).
fn light_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let entity = game.require_entity_mut(&this);
    entity.spawnflags ^= 1;
    let (style, pattern) = (entity.style, if entity.spawnflags & 1 != 0 { "a" } else { "m" });
    game.host_emit(Q2PresentationEvent::LightStyle {
        style,
        pattern: pattern.to_string(),
    });
}

/// Trigger relay use (`trigger_relay_use`).
fn trigger_relay_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    let authored = game.require_entity(&this).authored_target();
    game.use_targets(&authored, activator.as_ref(), false);
}

/// Trigger counter use (`trigger_counter_use`).
fn trigger_counter_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    if game.require_entity(&this).count == 0 {
        return;
    }
    game.require_entity_mut(&this).count -= 1;
    let entity = game.require_entity(&this).clone();
    if entity.spawnflags & 1 == 0 && activator.is_some() {
        let activator = activator.clone().expect("counter activator");
        let text = if entity.count != 0 {
            format!("{} more to go...", entity.count)
        } else {
            "Sequence completed!".to_string()
        };
        game.host_emit(Q2PresentationEvent::CenterPrint {
            actor: activator,
            text,
            instant: false,
            duration_seconds: None,
        });
        game.sound(&this, "misc/talk1.wav", 0, 1.0, 1.0);
    }
    if game.require_entity(&this).count == 0 {
        trigger_multi(this, game, activator);
    }
}

/// Trigger key use (`trigger_key_use`).
fn trigger_key_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    let key_name = game
        .require_entity(&this)
        .spawn
        .values
        .get("item")
        .cloned()
        .unwrap_or_default();
    let key = format!("q2:{key_name}");
    let Some(pickup) = game.item_name(&key_name) else {
        panic!("Q2 saved key references missing item {key_name}");
    };
    let Some(activator) = activator else { return };
    if !game.host.is_player(&activator) {
        return;
    }
    if game.host.inventory().count(&activator, &key) == 0.0 {
        let now = game.host.now();
        if now < game.require_entity(&this).timestamp {
            return;
        }
        game.require_entity_mut(&this).timestamp = now + 5.0;
        game.host_emit(Q2PresentationEvent::CenterPrint {
            actor: activator.clone(),
            text: format!("You need the {pickup}"),
            instant: false,
            duration_seconds: None,
        });
        if let Some(body) = game.host.bodies().read(&activator) {
            game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                actor: Some(activator),
                origin: body.origin,
                path: "misc/keytry.wav".to_string(),
                channel: 0,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_: Q2SoundLoop::Once,
                loop_owner: None,
            }));
        }
        return;
    }
    if let Some(body) = game.host.bodies().read(&activator) {
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(activator.clone()),
            origin: body.origin,
            path: "misc/keyuse.wav".to_string(),
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }
    // Key identity is preserved in the shared inventory even for a foreign player provider.
    let cube_key = key_name == "key_power_cube"
        || game.options.edition == Q2Edition::Rerelease && key_name == "key_explosive_charges";
    let mut cube = 0;
    if game.options.mode == Q2Mode::Coop && cube_key {
        let bits = game.entity(&activator).map(|entity| entity.power_cubes).unwrap_or(0);
        while cube < 8 && bits & 1 << cube == 0 {
            cube += 1;
        }
    }
    let players = if game.options.mode == Q2Mode::Coop {
        game.host.players()
    } else {
        vec![activator.clone()]
    };
    for player in players {
        let Some(owner) = game.host.actors().resolve_owned(&player) else {
            continue;
        };
        if game.options.mode == Q2Mode::Coop && cube_key {
            let cubes = game.entity(&player).map(|entity| entity.power_cubes);
            let Some(cubes) = cubes else { continue };
            if cubes & 1 << cube == 0 {
                continue;
            }
            game.require_entity_mut(&player).power_cubes &= !(1 << cube);
            game.host.inventory().consume(&owner, &key, 1.0);
        } else {
            let count = if game.options.mode == Q2Mode::Coop {
                game.host.inventory().count(&player, &key)
            } else {
                1.0
            };
            game.host.inventory().consume(&owner, &key, count);
        }
        game.host.key_consumed(&player);
    }
    let authored = game.require_entity(&this).authored_target();
    game.use_targets(&authored, Some(&activator), false);
    game.require_entity_mut(&this).use_ = None;
}

/// Target help use (`use_target_help`).
fn use_target_help(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let entity = game.require_entity(&this).clone();
    game.host_emit(Q2PresentationEvent::Help {
        slot: if entity.spawnflags & 1 != 0 { 1 } else { 2 },
        text: entity.message,
    });
}

/// Target secret/goal use (`use_target_secret_or_goal`).
fn use_target_secret_or_goal(
    this: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    activator: Option<ActorId>,
) {
    let secret = game.require_entity(&this).classname == "target_secret";
    let noise = game
        .require_entity(&this)
        .spawn
        .values
        .get("noise")
        .cloned()
        .unwrap_or_else(|| "misc/secret.wav".to_string());
    game.sound(&this, &noise, 2, 1.0, 1.0);
    if secret {
        game.counters.found_secrets += 1;
    } else {
        game.counters.found_goals += 1;
    }
    if !secret && game.counters.total_goals == game.counters.found_goals {
        game.host_emit(Q2PresentationEvent::Music { track: "0".to_string() });
    }
    let authored = game.require_entity(&this).authored_target();
    game.use_targets(&authored, activator.as_ref(), false);
    game.remove_actor(this);
}

/// Target explosion use (`use_target_explosion`).
fn use_target_explosion(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    game.require_entity_mut(&this).activator = activator;
    if game.require_entity(&this).delay == 0.0 {
        target_explosion_explode(this, game);
    } else {
        let delay = game.require_entity(&this).delay;
        game.schedule(this, delay, target_explosion_explode);
    }
}

/// Target explosion detonation (`explode`).
fn target_explosion_explode(this: ActorId, game: &mut Q2GameServices) {
    let origin = game.body_of(this.clone()).origin;
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    let entity = game.require_entity(&this).clone();
    game.radius_damage(
        this.clone(),
        entity.activator.clone(),
        entity.damage,
        None,
        entity.damage + 40.0,
        25,
        0,
        None,
    );
    game.use_targets(&entity.authored_target(), entity.activator.as_ref(), true);
}

/// Target splash use (`use_target_splash`).
fn use_target_splash(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    let entity = game.require_entity(&this).clone();
    let origin = game.body_of(this.clone()).origin;
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:splash".to_string(),
        origin,
        direction: entity.movedir,
        count: entity.count,
        color: integer_field(&entity.spawn, "sounds", 0),
    }));
    if entity.damage != 0.0 {
        game.radius_damage(
            this.clone(),
            activator.clone(),
            entity.damage,
            None,
            entity.damage + 40.0,
            29,
            0,
            None,
        );
    }
}

/// Target POI use (`use_target_poi`).
fn use_target_poi(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let entity = game.require_entity(&this).clone();
    let origin = game.body_of(this).origin;
    game.host_emit(Q2PresentationEvent::Poi {
        origin,
        message: entity.message,
        fields: entity.spawn.values,
    });
}

/// Func areaportal use (`Use_Areaportal`).
fn use_areaportal(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let entity = game.require_entity_mut(&this);
    entity.count ^= 1;
    let (style, open) = (entity.style, entity.count != 0);
    game.host.set_area_portal(style, open);
}

/// Target item name (unused).
fn target_item_name(_classname: &str) -> Option<String> {
    None
}

/// Create the Q2 target module (`createQ2TargetModule`).
pub fn create_q2_target_module() -> SpawnModule {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("multi_wait", multi_wait);
    callbacks.think.insert("func_timer_think", func_timer_think);
    callbacks
        .think
        .insert("target_explosion_explode", target_explosion_explode);
    callbacks.use_.insert("dynamic_light_use", dynamic_light_use);
    callbacks.use_.insert("Use_Multi", use_multi);
    callbacks.use_.insert("Use_Target_Speaker", use_target_speaker);
    callbacks.use_.insert("func_timer_use", func_timer_use);
    callbacks.use_.insert("use_target_changelevel", use_target_changelevel);
    callbacks.use_.insert("light_use", light_use);
    callbacks.use_.insert("trigger_relay_use", trigger_relay_use);
    callbacks.use_.insert("trigger_counter_use", trigger_counter_use);
    callbacks.use_.insert("trigger_key_use", trigger_key_use);
    callbacks.use_.insert("use_target_help", use_target_help);
    callbacks
        .use_
        .insert("use_target_secret_or_goal", use_target_secret_or_goal);
    callbacks.use_.insert("use_target_explosion", use_target_explosion);
    callbacks.use_.insert("use_target_splash", use_target_splash);
    callbacks.use_.insert("use_target_poi", use_target_poi);
    callbacks.use_.insert("Use_Areaportal", use_areaportal);
    callbacks.touch.insert("Touch_Multi", touch_multi);
    let spawn: Q2SpawnFn = spawn_target;
    let item_name: Q2ItemNameFn = target_item_name;
    SpawnModule {
        spawn,
        item_name,
        callbacks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrotates_landmark_vectors() {
        let vector = vec3(1.0, 2.0, 3.0);
        let identity = unrotate_q2_landmark(vector, vec3(0.0, 0.0, 0.0));
        assert!((identity.x - 1.0).abs() < 1e-6);
        assert!((identity.y - 2.0).abs() < 1e-6);
        assert!((identity.z - 3.0).abs() < 1e-6);
        let yawed = unrotate_q2_landmark(vec3(1.0, 0.0, 0.0), vec3(0.0, 90.0, 0.0));
        assert!(yawed.x.abs() < 1e-6);
        assert!((yawed.y + 1.0).abs() < 1e-6);
        assert!(yawed.z.abs() < 1e-6);
    }
}
