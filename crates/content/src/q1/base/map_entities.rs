//! Remaining map entities (`src/content/q1/base/map-entities.ts`).
//!
//! misc.qc/plats.qc/triggers.qc/items.qc. Copyright (C) 1996-2022 id
//! Software LLC. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::projectiles::{create_missile, launch_laser, launch_spike, SpikeKind};
use crate::q1::foundation::entity::move_direction;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    vadd, vscale, vsub, Q1Edition, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, ZERO,
};
use crate::q1::{q1_error, Q1Error};

/// Remaining map classnames in donor order (`remainingMapClassnames`).
pub const REMAINING_MAP_CLASSNAMES: &[&str] = &[
    "func_train",
    "misc_teleporttrain",
    "func_illusionary",
    "func_episodegate",
    "func_bossgate",
    "item_sigil",
    "event_lightning",
    "testplayerstart",
    "trigger_changelevel",
    "trigger_setskill",
    "trigger_onlyregistered",
    "trigger_monsterjump",
    "trap_spikeshooter",
    "trap_shooter",
    "misc_fireball",
    "air_bubbles",
    "light_globe",
    "light_torch_small_walltorch",
    "light_flame_large_yellow",
    "light_flame_small_yellow",
    "light_flame_small_white",
    "ambient_suck_wind",
    "ambient_flouro_buzz",
    "ambient_drip",
    "ambient_thunder",
    "ambient_light_buzz",
    "ambient_swamp1",
    "ambient_swamp2",
    "viewthing",
    "misc_noisemaker",
];

fn init_trigger(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let angles = game.body(&id).map(|body| body.angles)?;
    let movedir = move_direction(angles, Some(&mut *game));
    game.update_entity(&id, |entity| {
        entity.movedir = movedir;
        entity.solid = Q1Solid::Trigger;
        entity.model = String::new();
    })?;
    game.set_body(
        &id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )
}

fn later(game: &mut Q1EntityServices, id: &ActorId, delay: f64, name: &str) -> Result<(), Q1Error> {
    game.schedule(id, delay, name)
}

/// Emit an entity as a static model and remove it (`makeStatic`).
pub fn make_static(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let body = game.body(&id)?;
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.host.emit(Q1Event::StaticModel {
        path: entity.model.clone(),
        frame: entity.frame,
        color_map: entity.number("colormap") as i32,
        skin: entity.skin,
        origin: body.origin,
        angles: body.angles,
    });
    game.remove(&id)
}

fn train_next(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    game.update_entity(&id, |entity| entity.activated = true)?;
    let target = game
        .entity_ref(&id)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    let corner = game
        .find(&target)
        .first()
        .cloned()
        .ok_or_else(|| q1_error(format!("Train target not found: {target}")))?;
    let next = game
        .entity_ref(&corner)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    if next.is_empty() {
        return Err(q1_error("train_next: no next target"));
    }
    let wait = game.entity_ref(&corner).map(|entity| entity.wait).unwrap_or(0.0);
    game.update_entity(&id, |entity| {
        entity.target = next;
        entity.wait = wait;
    })?;
    let sounds = game.entity_ref(&id).map(|entity| entity.sounds).unwrap_or(0);
    game.sound_simple(
        &id,
        if sounds == 1 {
            "plats/train1.wav"
        } else {
            "misc/null.wav"
        },
    )?;
    let destination = vsub(
        game.body(&corner).map(|body| body.origin)?,
        game.body(&id).map(|body| body.bounds.min)?,
    );
    let speed = game.entity_ref(&id).map(|entity| entity.speed).unwrap_or(0.0);
    game.calc_move(&id, destination, speed, "base:train_wait")
}

fn train(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let (classname, target, sounds) = game
        .entity_ref(&id)
        .map(|entity| (entity.classname.clone(), entity.target.clone(), entity.sounds))
        .unwrap_or_default();
    let teleport = classname == "misc_teleporttrain";
    if target.is_empty() {
        return Err(q1_error(format!("{classname} without a target")));
    }
    let paths: &[&str] = if teleport || sounds == 0 {
        &["misc/null.wav", "misc/null.wav"]
    } else if sounds == 1 {
        &["plats/train2.wav", "plats/train1.wav"]
    } else {
        &[]
    };
    for path in paths {
        if game.uses_id1_precaches() {
            game.precache_sound(path)?;
        }
    }
    if game.uses_id1_precaches() && teleport {
        game.precache_model("progs/teleport.mdl")?;
    }
    game.update_entity(&id, |entity| {
        if entity.speed == 0.0 {
            entity.speed = 100.0;
        }
        if entity.damage == 0.0 {
            entity.damage = 2.0;
        }
        entity.movement = Q1MoveType::Push;
        entity.solid = if teleport { Q1Solid::None } else { Q1Solid::Bsp };
        if teleport {
            entity.model = String::from("progs/teleport.mdl");
            entity.angular_velocity = Vec3 {
                x: 100.0,
                y: 200.0,
                z: 300.0,
            };
        } else {
            entity.classname = String::from("train");
        }
        entity.state = crate::q1::foundation::entity::Q1MoverState::Bottom;
        entity.activated = false;
    })?;
    let use_callback = game.named.use_callback("base:train_use")?;
    let blocked = game.named.blocked("base:train_blocked")?;
    game.update_entity(&id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.blocked = Some(blocked);
    })?;
    let at = f64::from((game.entity_ref(&id).map(|entity| entity.number("ltime")).unwrap_or(0.0) + 0.1) as f32);
    game.schedule_at(&id, at, "base:train_find")
}

fn sigil_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (entity, other) = (id.clone(), other.clone());
    if game
        .entity_ref(&entity)
        .is_none_or(|entity| entity.solid != Q1Solid::Trigger)
        || !game.is_player(&other)
        || game.health(&other) <= 0.0
    {
        return Ok(());
    }
    game.message(
        Some(&other),
        if game.options().edition == Q1Edition::Classic {
            "You got the rune!"
        } else {
            "$qc_got_rune"
        },
        true,
        Vec::new(),
    );
    if let Some(player) = game.host.actors.resolve_owned(&other) {
        game.sound(player.id(), "misc/runekey.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    }
    let origin = game.body(&entity).map(|body| body.origin)?;
    game.effect(Q1Effect::Pickup, origin, Some(&other), 1);
    game.update_entity(&entity, |entity| {
        entity.solid = Q1Solid::None;
        entity.model = String::new();
        entity.touch = None;
    })?;
    game.link(&entity)?;
    let flags = super::provider::campaign_read_flags(game)?;
    let bits = game.entity_ref(&entity).map(|entity| entity.spawnflags).unwrap_or(0) & 15;
    super::provider::campaign_write_flags(game, flags | bits)?;
    game.update_entity(&entity, |entity| entity.classname = String::new())?;
    game.use_targets(&entity, Some(&other))
}

fn sigil(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let bits = game.entity_ref(&id).map(|entity| entity.spawnflags).unwrap_or(0) & 15;
    if bits == 0 {
        return Err(q1_error("item_sigil has no episode spawnflags"));
    }
    if game.uses_id1_precaches() {
        game.precache_sound("misc/runekey.wav")?;
    }
    for episode in 1..=4 {
        if game.uses_id1_precaches() && bits & (1 << (episode - 1)) != 0 {
            game.precache_model(&format!("progs/end{episode}.mdl"))?;
        }
    }
    let number = if bits & 8 != 0 {
        4
    } else if bits & 4 != 0 {
        3
    } else if bits & 2 != 0 {
        2
    } else {
        1
    };
    game.update_entity(&id, |entity| {
        entity.model = format!("progs/end{number}.mdl");
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Toss;
    })?;
    game.set_bounds(
        &id,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 32.0,
            },
        },
    )?;
    let touch = game.named.touch("base:sigil_touch")?;
    game.update_entity(&id, |entity| entity.touch = Some(touch))?;
    later(game, &id, 0.2, "base:sigil_place")
}

fn shooter_fire(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.spawnflags & 2 != 0 {
        game.sound_simple(&id, "enforcer/enfire.wav")?;
        let laser = launch_laser(game, Some(&id), game.body(&id).map(|body| body.origin)?, entity.movedir)?;
        if entity.classname == "trap_shooter" {
            game.set_body(
                &laser,
                &BodyPatch {
                    velocity: Some(vscale(entity.movedir, 500.0)),
                    ..Default::default()
                },
            )?;
        }
    } else {
        game.sound_simple(&id, "weapons/spike2.wav")?;
        launch_spike(
            game,
            Some(&id),
            game.body(&id).map(|body| body.origin)?,
            vscale(entity.movedir, 500.0),
            if entity.spawnflags & 1 != 0 {
                SpikeKind::Superspike
            } else {
                SpikeKind::Spike
            },
        )?;
    }
    Ok(())
}

fn shooter(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    game.update_entity(&id, |entity| {
        entity
            .fields
            .insert(String::from("killstring"), String::from("$qc_ks_spiked"));
    })?;
    let angles = game.body(&id).map(|body| body.angles)?;
    let movedir = move_direction(angles, Some(&mut *game));
    game.update_entity(&id, |entity| entity.movedir = movedir)?;
    game.set_body(
        &id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let use_callback = game.named.use_callback("base:shooter_fire")?;
    game.update_entity(&id, |entity| entity.use_callback = Some(use_callback))?;
    let (spawnflags, classname) = game
        .entity_ref(&id)
        .map(|entity| (entity.spawnflags, entity.classname.clone()))
        .unwrap_or_default();
    if spawnflags & 2 != 0 {
        if game.uses_id1_precaches() {
            game.precache_model("progs/laser.mdl")?;
        }
        if game.uses_id1_precaches() {
            game.precache_sound("enforcer/enfire.wav")?;
        }
        if game.uses_id1_precaches() {
            game.precache_sound("enforcer/enfstop.wav")?;
        }
    } else if game.uses_id1_precaches() {
        game.precache_sound("weapons/spike2.wav")?;
    }
    if classname == "trap_spikeshooter" {
        return Ok(());
    }
    game.update_entity(&id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 1.0;
        }
    })?;
    let delay = game
        .entity_ref(&id)
        .map(|entity| entity.number("nextthink") + entity.wait)
        .unwrap_or(0.0);
    later(game, &id, delay, "base:shooter_think")
}

/// Spawn an air bubble (`spawnBubble`).
pub fn spawn_bubble(
    game: &mut Q1EntityServices,
    origin: Vec3,
    velocity: Vec3,
    split: bool,
) -> Result<ActorId, Q1Error> {
    let bubble = game.create("bubble", None, None)?;
    game.update_entity(&bubble, |entity| {
        entity.model = String::from("progs/s_bubble.spr");
        entity.movement = Q1MoveType::Noclip;
        entity.frame = if split { 1 } else { 0 };
        entity.count = if split { 10.0 } else { 0.0 };
    })?;
    game.set_body(
        &bubble,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            bounds: Some(Bounds {
                min: Vec3 {
                    x: -8.0,
                    y: -8.0,
                    z: -8.0,
                },
                max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
            }),
            ..Default::default()
        },
    )?;
    game.link(&bubble)?;
    later(game, &bubble, 0.5, "base:bubble_bob")?;
    Ok(bubble)
}

fn bubble_bob(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let body = game.body(&id)?;
    game.update_entity(&id, |entity| entity.count += 1.0)?;
    if game.entity_ref(&id).map(|entity| entity.count).unwrap_or(0.0) == 4.0 {
        spawn_bubble(game, body.origin, body.velocity, true)?;
        game.update_entity(&id, |entity| {
            entity.frame = 1;
            entity.count = 10.0;
        })?;
    }
    let contents = game.host.contents(body.origin);
    let count = game.entity_ref(&id).map(|entity| entity.count).unwrap_or(0.0);
    if count >= 20.0
        || !matches!(
            contents,
            crate::q1::foundation::host::Q1Contents::Water
                | crate::q1::foundation::host::Q1Contents::Slime
                | crate::q1::foundation::host::Q1Contents::Lava
        )
    {
        return game.remove(&id);
    }
    let x = body.velocity.x - 10.0 + game.host.random() as f32 * 20.0;
    let y = body.velocity.y - 10.0 + game.host.random() as f32 * 20.0;
    let z = body.velocity.z + 10.0 + game.host.random() as f32 * 10.0;
    game.set_body(
        &id,
        &BodyPatch {
            velocity: Some(Vec3 {
                x: if x > 10.0 {
                    5.0
                } else if x < -10.0 {
                    -5.0
                } else {
                    x
                },
                y: if y > 10.0 {
                    5.0
                } else if y < -10.0 {
                    -5.0
                } else {
                    y
                },
                z: if z > 30.0 {
                    25.0
                } else if z < 10.0 {
                    15.0
                } else {
                    z
                },
            }),
            ..Default::default()
        },
    )?;
    later(game, &id, 0.5, "base:bubble_bob")
}

fn changelevel_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (entity, other) = (id.clone(), other.clone());
    if !game.is_player(&other) {
        return Ok(());
    }
    if game.options().no_exit == Some(1) || game.options().no_exit == Some(2) && game.map_name != "start" {
        game.damage(
            &other,
            Some(&entity),
            Some(&entity),
            50000.0,
            &Q1DamageParams {
                death_type: String::from("exit"),
                ..Default::default()
            },
        );
        return Ok(());
    }
    let map = game
        .entity_ref(&entity)
        .map(|entity| entity.text("map"))
        .unwrap_or_default();
    super::provider::level_changelevel_touched(game, &entity, &other)?;
    if let Some(exited) = super::provider::player_exited_observer(game)? {
        exited(other.clone());
    }
    game.use_targets(&entity, Some(&other))?;
    if game
        .entity_ref(&entity)
        .is_some_and(|entity| entity.spawnflags & 1 != 0)
        && game.options().deathmatch == 0
    {
        let same = super::provider::same_level_probe(game)?.is_some_and(|same| same());
        let destination = if same { game.map_name.clone() } else { map };
        return super::provider::level_travel_to(game, &destination, Some(&other));
    }
    game.update_entity(&entity, |entity| {
        entity.touch = None;
        entity.activator = Some(other);
    })?;
    later(game, &entity, 0.1, "base:execute_changelevel")
}

fn fireball_fly(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let (origin, speed) = (
        game.body(&id).map(|body| body.origin)?,
        game.entity_ref(&id).map(|entity| entity.speed).unwrap_or(0.0),
    );
    let (jx, jy, jz) = (game.host.random(), game.host.random(), game.host.random());
    let missile = create_missile(
        game,
        Some(&id),
        "fireball",
        "lavaball",
        origin,
        Vec3 {
            x: jx as f32 * 100.0 - 50.0,
            y: jy as f32 * 100.0 - 50.0,
            z: speed as f32 + jz as f32 * 200.0,
        },
        5.0,
    )?;
    let touch = game.named.touch("base:fireball_touch")?;
    game.update_entity(&missile, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Toss;
        entity.touch = Some(touch);
    })?;
    let delay = game.host.random() * 5.0 + 3.0;
    later(game, &id, delay, "base:fireball_fly")
}

fn train_wait(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let (wait, sounds) = game
        .entity_ref(&id)
        .map(|entity| (entity.wait, entity.sounds))
        .unwrap_or_default();
    if wait != 0.0 {
        game.sound_simple(
            &id,
            if sounds == 1 {
                "plats/train2.wav"
            } else {
                "misc/null.wav"
            },
        )?;
    }
    let at = f64::from(
        (game.entity_ref(&id).map(|entity| entity.number("ltime")).unwrap_or(0.0)
            + if wait == 0.0 { 0.1 } else { wait }) as f32,
    );
    game.schedule_at(&id, at, "base:train_next")
}

fn train_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let ready = game
        .entity_ref(&id)
        .is_some_and(|entity| entity.state == crate::q1::foundation::entity::Q1MoverState::Top && !entity.activated);
    if ready {
        return train_next(game, &id);
    }
    Ok(())
}

fn train_blocked(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let (id, other) = (id.clone(), other.clone());
    if game.time < game.entity_ref(&id).map(|entity| entity.attack_finished).unwrap_or(0.0) {
        return Ok(());
    }
    let damage = game.entity_ref(&id).map(|entity| entity.damage).unwrap_or(0.0);
    let until = game.time + 0.5;
    game.update_entity(&id, |entity| entity.attack_finished = until)?;
    game.damage(
        &other,
        Some(&id),
        Some(&id),
        damage,
        &Q1DamageParams {
            death_type: String::from("crush"),
            ..Default::default()
        },
    );
    Ok(())
}

fn train_find(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let target = game
        .entity_ref(&id)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    let corner = game
        .find(&target)
        .first()
        .cloned()
        .ok_or_else(|| q1_error(format!("Train first target not found: {target}")))?;
    let next = game
        .entity_ref(&corner)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    game.update_entity(&id, |entity| entity.target = next)?;
    let at = vsub(
        game.body(&corner).map(|body| body.origin)?,
        game.body(&id).map(|body| body.bounds.min)?,
    );
    game.set_origin(&id, at)?;
    game.update_entity(&id, |entity| {
        entity.state = crate::q1::foundation::entity::Q1MoverState::Top
    })?;
    if game.entity_ref(&id).is_some_and(|entity| entity.targetname.is_empty()) {
        let when = f64::from((game.entity_ref(&id).map(|entity| entity.number("ltime")).unwrap_or(0.0) + 0.1) as f32);
        return game.schedule_at(&id, when, "base:train_next");
    }
    Ok(())
}

fn sigil_place(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let body = game.body(&id)?;
    let start = vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 6.0 });
    let floor = game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(
            start,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -256.0,
            },
        ),
        bounds: body.bounds,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if floor.all_solid || floor.fraction == 1.0 {
        return game.remove(&id);
    }
    game.set_body(
        &id,
        &BodyPatch {
            origin: Some(floor.end),
            velocity: Some(ZERO),
            ground: Some(floor.actor),
            ..Default::default()
        },
    )?;
    game.link(&id)
}

fn shooter_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    shooter_fire(game, &id, None, None)?;
    let wait = game.entity_ref(&id).map(|entity| entity.wait).unwrap_or(0.0);
    later(game, &id, wait, "base:shooter_think")
}

fn execute_changelevel(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let map = game
        .entity_ref(&id)
        .map(|entity| entity.text("map"))
        .unwrap_or_default();
    let activator = game.entity_ref(&id).and_then(|entity| entity.activator.clone());
    super::provider::level_begin(game, &map, activator.as_ref())
}

fn wall_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.frame = 1 - entity.frame)
}

fn donor_number(text: &str) -> f64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

fn setskill_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    let message = game
        .entity_ref(id)
        .map(|entity| entity.message.clone())
        .unwrap_or_default();
    let value = donor_number(&message).floor().clamp(0.0, 3.0);
    if value == 0.0 || value == 1.0 || value == 2.0 || value == 3.0 {
        super::provider::campaign_set_skill(game, value as i32)?;
    }
    Ok(())
}

fn registered_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (entity, other) = (id.clone(), other.clone());
    if !game.is_player(&other)
        || game.time
            < game
                .entity_ref(&entity)
                .map(|entity| entity.attack_finished)
                .unwrap_or(0.0)
    {
        return Ok(());
    }
    let until = game.time + 2.0;
    game.update_entity(&entity, |entity| entity.attack_finished = until)?;
    if super::provider::base_registered_flag(game)? {
        game.update_entity(&entity, |entity| entity.message = String::new())?;
        game.use_targets(&entity, Some(&other))?;
        return game.remove(&entity);
    }
    let message = game
        .entity_ref(&entity)
        .map(|entity| entity.message.clone())
        .unwrap_or_default();
    if !message.is_empty() {
        game.message(Some(&other), &message, true, Vec::new());
        if let Some(player) = game.host.actors.resolve_owned(&other) {
            game.sound(player.id(), "misc/talk.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
        }
    }
    Ok(())
}

fn monsterjump_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (entity, other) = (id.clone(), other.clone());
    let Some(monster) = game.entity_ref(&other).cloned() else {
        return Ok(());
    };
    let Some(body) = game.host.bodies.read(&other) else {
        return Ok(());
    };
    if monster.monster.is_none() || monster.movement_flags & 3 != 0 {
        return Ok(());
    }
    let trigger = game
        .entity_ref(&entity)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let height = trigger.number("height");
    game.set_body(
        &other,
        &BodyPatch {
            velocity: Some(Vec3 {
                x: trigger.movedir.x * trigger.speed as f32,
                y: trigger.movedir.y * trigger.speed as f32,
                z: if body.ground.is_none() {
                    body.velocity.z
                } else if height == 0.0 {
                    200.0
                } else {
                    height as f32
                },
            }),
            ground: Some(None),
            ..Default::default()
        },
    )?;
    if body.ground.is_some() {
        game.update_entity(&other, |entity| entity.movement_flags &= !512)?;
    }
    Ok(())
}

fn fireball_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let (entity, other) = (id.clone(), other.clone());
    game.damage_direct(&other, Some(&entity), Some(&entity), 20.0);
    game.remove(&entity)
}

fn make_bubbles(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let origin = game.body(&id).map(|body| body.origin)?;
    spawn_bubble(
        game,
        origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 15.0,
        },
        false,
    )?;
    let delay = game.host.random() + 0.5;
    later(game, &id, delay, "base:make_bubbles")
}

fn noisemaker(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let sounds: &[(&str, Q1SoundChannel)] = &[
        ("enfire", Q1SoundChannel::Weapon),
        ("enfstop", Q1SoundChannel::Voice),
        ("sight1", Q1SoundChannel::Item),
        ("sight2", Q1SoundChannel::Body),
        ("sight3", Q1SoundChannel::Raw(5)),
        ("sight4", Q1SoundChannel::Raw(6)),
        ("pain1", Q1SoundChannel::Raw(7)),
    ];
    for (path, channel) in sounds {
        game.sound(&id, &format!("enforcer/{path}.wav"), *channel, 1.0, 1.0)?;
    }
    later(game, &id, 0.5, "base:noisemaker")
}

/// Register map callbacks (`registerMapCallbacks`).
pub fn register_map_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    use crate::q1::foundation::callbacks::{
        Q1ActionHandler, Q1BlockedHandler, Q1CallbackHandlers, Q1TouchHandler, Q1UseHandler,
    };
    let action = |handler: Q1ActionHandler| Q1CallbackHandlers {
        action: Some(handler),
        ..Default::default()
    };
    let touch = |handler: Q1TouchHandler| Q1CallbackHandlers {
        touch: Some(handler),
        ..Default::default()
    };
    let use_callback = |handler: Q1UseHandler| Q1CallbackHandlers {
        use_callback: Some(handler),
        ..Default::default()
    };
    game.named.register("base:train_next", action(train_next))?;
    game.named.register("base:train_wait", action(train_wait))?;
    game.named.register("base:train_use", use_callback(train_use))?;
    game.named.register(
        "base:train_blocked",
        Q1CallbackHandlers {
            blocked: Some(train_blocked as Q1BlockedHandler),
            ..Default::default()
        },
    )?;
    game.named.register("base:train_find", action(train_find))?;
    game.named.register("base:sigil_touch", touch(sigil_touch))?;
    game.named.register("base:sigil_place", action(sigil_place))?;
    game.named.register("base:shooter_fire", use_callback(shooter_fire))?;
    game.named.register("base:shooter_think", action(shooter_think))?;
    game.named.register("base:bubble_bob", action(bubble_bob))?;
    game.named
        .register("base:changelevel_touch", touch(changelevel_touch))?;
    game.named
        .register("base:execute_changelevel", action(execute_changelevel))?;
    game.named.register("base:wall_use", use_callback(wall_use))?;
    game.named.register("base:setskill_touch", touch(setskill_touch))?;
    game.named.register("base:registered_touch", touch(registered_touch))?;
    game.named
        .register("base:monsterjump_touch", touch(monsterjump_touch))?;
    game.named.register("base:fireball_fly", action(fireball_fly))?;
    game.named.register("base:fireball_touch", touch(fireball_touch))?;
    game.named.register("base:make_bubbles", action(make_bubbles))?;
    game.named.register("base:noisemaker", action(noisemaker))?;
    Ok(())
}

/// Spawn a remaining map actor (`spawnRemainingMapActor`).
pub fn spawn_remaining_map_actor(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let classname = game
        .entity_ref(&id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    match classname.as_str() {
        "testplayerstart" => Ok(()),
        "trigger_changelevel" => {
            if game
                .entity_ref(&id)
                .map(|entity| entity.text("map"))
                .unwrap_or_default()
                .is_empty()
            {
                return Err(q1_error("changelevel trigger doesn't have map"));
            }
            game.update_entity(&id, |entity| {
                entity
                    .fields
                    .insert(String::from("killstring"), String::from("$qc_ks_tried_leave"));
            })?;
            init_trigger(game, &id)?;
            let touch = game.named.touch("base:changelevel_touch")?;
            game.update_entity(&id, |entity| entity.touch = Some(touch))
        }
        "func_train" | "misc_teleporttrain" => train(game, &id),
        "item_sigil" => sigil(game, &id),
        "event_lightning" => super::provider::spawn_lightning(game, &id),
        "func_episodegate" | "func_bossgate" => {
            let flags = super::provider::campaign_read_flags(game)?;
            let (classname, spawnflags) = game
                .entity_ref(&id)
                .map(|entity| (entity.classname.clone(), entity.spawnflags))
                .unwrap_or_default();
            if classname == "func_episodegate" && flags & spawnflags == 0
                || classname == "func_bossgate" && flags & 15 == 15
            {
                game.update_entity(&id, |entity| entity.model = String::new())?;
                return Ok(());
            }
            game.update_entity(&id, |entity| {
                entity.solid = Q1Solid::Bsp;
                entity.movement = Q1MoveType::Push;
            })?;
            game.set_body(
                &id,
                &BodyPatch {
                    angles: Some(ZERO),
                    ..Default::default()
                },
            )?;
            let use_callback = game.named.use_callback("base:wall_use")?;
            game.update_entity(&id, |entity| entity.use_callback = Some(use_callback))
        }
        "func_illusionary" => {
            game.set_body(
                &id,
                &BodyPatch {
                    angles: Some(ZERO),
                    ..Default::default()
                },
            )?;
            game.update_entity(&id, |entity| {
                entity.solid = Q1Solid::None;
                entity.movement = Q1MoveType::None;
            })?;
            make_static(game, &id)
        }
        "trigger_setskill" => {
            init_trigger(game, &id)?;
            let touch = game.named.touch("base:setskill_touch")?;
            game.update_entity(&id, |entity| entity.touch = Some(touch))
        }
        "trigger_onlyregistered" => {
            if game.uses_id1_precaches() {
                game.precache_sound("misc/talk.wav")?;
            }
            init_trigger(game, &id)?;
            let touch = game.named.touch("base:registered_touch")?;
            game.update_entity(&id, |entity| entity.touch = Some(touch))
        }
        "trigger_monsterjump" => {
            if game.body(&id).map(|body| body.angles.y).unwrap_or(0.0) == 0.0 {
                game.set_body(
                    &id,
                    &BodyPatch {
                        angles: Some(Vec3 {
                            x: 0.0,
                            y: 360.0,
                            z: 0.0,
                        }),
                        ..Default::default()
                    },
                )?;
            }
            init_trigger(game, &id)?;
            game.update_entity(&id, |entity| {
                if entity.speed == 0.0 {
                    entity.speed = 200.0;
                }
            })?;
            let touch = game.named.touch("base:monsterjump_touch")?;
            game.update_entity(&id, |entity| entity.touch = Some(touch))
        }
        "trap_spikeshooter" | "trap_shooter" => shooter(game, &id),
        "misc_fireball" => {
            if game.uses_id1_precaches() {
                game.precache_model("progs/lavaball.mdl")?;
            }
            game.update_entity(&id, |entity| {
                entity.classname = String::from("fireball");
                entity
                    .fields
                    .insert(String::from("killstring"), String::from("$qc_ks_lavaball"));
                if entity.speed == 0.0 {
                    entity.speed = 1000.0;
                }
            })?;
            let delay = game.host.random() * 5.0;
            later(game, &id, delay, "base:fireball_fly")
        }
        "air_bubbles" => {
            if game.options().deathmatch != 0 {
                return game.remove(&id);
            }
            if game.uses_id1_precaches() {
                game.precache_model("progs/s_bubble.spr")?;
            }
            later(game, &id, 1.0, "base:make_bubbles")
        }
        "light_globe" => {
            if game.uses_id1_precaches() {
                game.precache_model("progs/s_light.spr")?;
            }
            game.update_entity(&id, |entity| entity.model = String::from("progs/s_light.spr"))?;
            make_static(game, &id)
        }
        "light_torch_small_walltorch"
        | "light_flame_large_yellow"
        | "light_flame_small_yellow"
        | "light_flame_small_white" => {
            let classname = game
                .entity_ref(&id)
                .map(|entity| entity.classname.clone())
                .unwrap_or_default();
            game.update_entity(&id, |entity| {
                entity.model = if classname == "light_torch_small_walltorch" {
                    String::from("progs/flame.mdl")
                } else {
                    String::from("progs/flame2.mdl")
                };
                entity.frame = if classname == "light_flame_large_yellow" { 1 } else { 0 };
            })?;
            let model = game
                .entity_ref(&id)
                .map(|entity| entity.model.clone())
                .unwrap_or_default();
            if game.uses_id1_precaches() {
                game.precache_model(&model)?;
            }
            if game.uses_id1_precaches() {
                game.precache_sound("ambience/fire1.wav")?;
            }
            let origin = game.body(&id).map(|body| body.origin)?;
            game.host.emit(Q1Event::Ambient {
                origin,
                path: String::from("ambience/fire1.wav"),
                volume: 0.5,
                attenuation: 3.0,
            });
            make_static(game, &id)
        }
        "ambient_suck_wind"
        | "ambient_flouro_buzz"
        | "ambient_drip"
        | "ambient_thunder"
        | "ambient_light_buzz"
        | "ambient_swamp1"
        | "ambient_swamp2" => {
            let name = classname.clone();
            let path = if name == "ambient_suck_wind" {
                "suck1"
            } else if name == "ambient_flouro_buzz" {
                "buzz1"
            } else if name == "ambient_drip" {
                "drip1"
            } else if name == "ambient_thunder" {
                "thunder1"
            } else if name == "ambient_light_buzz" {
                "fl_hum1"
            } else if name == "ambient_swamp1" {
                "swamp1"
            } else {
                "swamp2"
            };
            if game.uses_id1_precaches() {
                game.precache_sound(&format!("ambience/{path}.wav"))?;
            }
            let origin = game.body(&id).map(|body| body.origin)?;
            game.host.emit(Q1Event::Ambient {
                origin,
                path: format!("ambience/{path}.wav"),
                volume: if name == "ambient_suck_wind" || name == "ambient_flouro_buzz" {
                    1.0
                } else {
                    0.5
                },
                attenuation: 3.0,
            });
            Ok(())
        }
        "viewthing" => {
            if game.uses_id1_precaches() {
                game.precache_model("progs/player.mdl")?;
            }
            game.update_entity(&id, |entity| entity.model = String::from("progs/player.mdl"))
        }
        "misc_noisemaker" => {
            for path in [
                "enfire", "enfstop", "sight1", "sight2", "sight3", "sight4", "pain1", "pain2", "death1", "idle1",
            ] {
                if game.uses_id1_precaches() {
                    game.precache_sound(&format!("enforcer/{path}.wav"))?;
                }
            }
            let delay = 0.1 + game.host.random();
            later(game, &id, delay, "base:noisemaker")
        }
        _ => Err(q1_error(format!("Unknown base Q1 map class {classname}"))),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::base::provider::{campaign_read_flags, Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::entity_services::Q1AttachOptions;
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

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

    #[test]
    fn remaining_classnames_spawn() {
        let (host, events) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        assert_eq!(REMAINING_MAP_CLASSNAMES.len(), 30);
        for classname in REMAINING_MAP_CLASSNAMES {
            let entity = game
                .create(classname, None, None)
                .unwrap_or_else(|_| panic!("create {classname}"));
            match *classname {
                "trigger_changelevel" => {
                    game.update_entity(&entity, |entity| {
                        entity.fields.insert(String::from("map"), String::from("e1m2"));
                    })
                    .expect("map");
                }
                "item_sigil" => {
                    game.update_entity(&entity, |entity| entity.spawnflags = 1)
                        .expect("flags");
                }
                "func_train" | "misc_teleporttrain" => {
                    let corner = game.create("path_corner", None, None).expect("corner");
                    game.update_entity(&corner, |entity| {
                        entity.targetname = String::from("corner");
                        entity.target = String::from("corner");
                    })
                    .expect("corner");
                    game.update_entity(&entity, |entity| entity.target = String::from("corner"))
                        .expect("target");
                }
                _ => {}
            }
            game.spawn_entity(&entity, None)
                .unwrap_or_else(|_| panic!("spawn {classname}"));
        }
        assert!(events
            .borrow()
            .events
            .iter()
            .any(|event| matches!(event, Q1Event::StaticModel { .. })));
        let unknown = game.create("func_door", None, None).expect("door");
        let error = spawn_remaining_map_actor(&mut game, &unknown).expect_err("unknown class");
        assert!(error.to_string().contains("Unknown base Q1 map class"));
    }

    #[test]
    fn sigil_sets_campaign_and_changelevel_begins() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        game.set_health(&player, 100.0).expect("health");
        let sigil = game.create("item_sigil", None, None).expect("sigil");
        game.update_entity(&sigil, |entity| entity.spawnflags = 2)
            .expect("flags");
        game.spawn_entity(&sigil, None).expect("spawn");
        game.invoke_touch(&sigil, &player, None, None).expect("touch");
        assert_eq!(campaign_read_flags(&game).expect("flags"), 2);
        let change = game.create("trigger_changelevel", None, None).expect("change");
        game.update_entity(&change, |entity| {
            entity.fields.insert(String::from("map"), String::from("e1m2"));
        })
        .expect("map");
        game.spawn_entity(&change, None).expect("spawn");
        let spot = game.create("info_intermission", None, None).expect("spot");
        game.update_entity(&spot, |entity| entity.targetname = String::from("info_intermission"))
            .expect("name");
        game.invoke_touch(&change, &player, None, None).expect("touch");
        game.invoke_action(&change, "base:execute_changelevel")
            .expect("execute");
        assert!(game.intermission.is_some());
    }

    #[test]
    fn gates_skill_and_jump_follow_donor() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let gate = game.create("func_episodegate", None, None).expect("gate");
        game.update_entity(&gate, |entity| entity.spawnflags = 1)
            .expect("flags");
        game.spawn_entity(&gate, None).expect("spawn");
        assert_eq!(
            game.entity_ref(&gate).map(|entity| entity.model.clone()),
            Some(String::new())
        );
        let skill = game.create("trigger_setskill", None, None).expect("skill");
        game.update_entity(&skill, |entity| entity.message = String::from("3"))
            .expect("message");
        game.spawn_entity(&skill, None).expect("spawn");
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        game.invoke_touch(&skill, &player, None, None).expect("touch");
        assert!(donor_number("abc").is_nan());
        assert_eq!(donor_number(""), 0.0);
        assert_eq!(donor_number(" 2 "), 2.0);
    }
}
