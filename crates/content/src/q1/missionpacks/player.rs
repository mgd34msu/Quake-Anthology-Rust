//! Mission-pack players (src/content/q1/missionpacks/player.ts).

use qa_core::identity::{ActorId, same_actor};
use qa_core::math::Vec3;

use crate::contract::{InventoryEntry, ItemId};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::extensions::Q1PlayerExtension;
use crate::q1::foundation::gameplay::{
    AttackCause, BodyPatch, DamagePreparation, DamageRequest, TouchSurface,
};
use crate::q1::foundation::types::{
    POINT, Q1Effect, Q1MoveType, Q1Powerup, Q1Solid, Q1SoundChannel, Q1TraceRequest, Q1Weapon,
    ZERO, length, normalize, vadd, vscale, vsub, weapon_item, yaw_for,
};
use crate::q1::{Q1Error, q1_error};

use super::messages::mission_message;
use super::types::{
    MISSION_WEAPONS, Q1MissionPack, fround, mission_reference, move_missile, set_mission_number,
    set_mission_reference,
};

/// Rogue combo weapon (`ComboWeapon`).
struct ComboWeapon {
    base: Q1Weapon,
    powered: Q1Weapon,
    ammo: &'static str,
    message: &'static str,
}

const COMBOS: [ComboWeapon; 5] = [
    ComboWeapon {
        base: Q1Weapon::Nailgun,
        powered: Q1Weapon::RogueLavaNailgun,
        ammo: "rogue:ammo/lava-nails",
        message: "$qc_lava_enabled",
    },
    ComboWeapon {
        base: Q1Weapon::Supernailgun,
        powered: Q1Weapon::RogueLavaSupernailgun,
        ammo: "rogue:ammo/lava-nails",
        message: "$qc_super_lava_enabled",
    },
    ComboWeapon {
        base: Q1Weapon::Grenadelauncher,
        powered: Q1Weapon::RogueMultiGrenade,
        ammo: "rogue:ammo/multi-rockets",
        message: "$qc_multi_gl_enabled",
    },
    ComboWeapon {
        base: Q1Weapon::Rocketlauncher,
        powered: Q1Weapon::RogueMultiRocket,
        ammo: "rogue:ammo/multi-rockets",
        message: "$qc_multi_rl_enabled",
    },
    ComboWeapon {
        base: Q1Weapon::Lightning,
        powered: Q1Weapon::RoguePlasma,
        ammo: "rogue:ammo/plasma",
        message: "$qc_plasma_enabled",
    },
];

/// Mission-pack powerup timer (`timers`).
struct PowerupTimer {
    id: Q1Powerup,
    warn: &'static str,
    lost: &'static str,
    sound: &'static str,
}

const TIMERS: [PowerupTimer; 4] = [
    PowerupTimer {
        id: Q1Powerup::HipnoticWetsuit,
        warn: "$qc_wetsuit_fade",
        lost: "",
        sound: "items/suit2.wav",
    },
    PowerupTimer {
        id: Q1Powerup::HipnoticEmpathy,
        warn: "$qc_empathy_fade",
        lost: "",
        sound: "items/suit2.wav",
    },
    PowerupTimer {
        id: Q1Powerup::RogueShield,
        warn: "$qc_shield_failing",
        lost: "$qc_shield_lost",
        sound: "shield/fadeout.wav",
    },
    PowerupTimer {
        id: Q1Powerup::RogueAntigrav,
        warn: "$qc_antigrav_failing",
        lost: "$qc_antigrav_lost",
        sound: "belt/fadeout.wav",
    },
];

/// Mission-pack player services (`MissionPackPlayers`).
///
/// The donor holds the game and registers game-capturing damage stages.
/// Hooks here are static function pointers, so every method takes the game
/// explicitly and the before/after-quad logic lives in game-aware methods
/// for the session damage path to call.
pub struct MissionPackPlayers {
    /// Mission pack.
    pub pack: Q1MissionPack,
}

/// Find or create a player's mission-pack state entity (`state`).
fn player_state_entity(game: &mut Q1EntityServices, player: &ActorId) -> Result<ActorId, Q1Error> {
    if let Some(existing) = game
        .entities
        .values()
        .find(|entity| {
            entity.classname == "missionpack_player_state"
                && entity
                    .owner
                    .as_ref()
                    .is_some_and(|owner| same_actor(owner, player))
        })
        .map(|entity| entity.actor.id().clone())
    {
        return Ok(existing);
    }
    let entity = game.create("missionpack_player_state", None, None)?;
    let owner = player.clone();
    game.update_entity(&entity, |entity| entity.owner = Some(owner))?;
    Ok(entity)
}

/// Admit mission-pack inventory (`attach`).
fn attach_for_pack(
    game: &mut Q1EntityServices,
    pack: Q1MissionPack,
    player: &ActorId,
) -> Result<(), Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let prefix = pack.as_str();
    for weapon in MISSION_WEAPONS
        .iter()
        .filter(|weapon| weapon.id.as_str().starts_with(prefix))
    {
        let item = weapon_item(Q1Weapon::from(weapon.id));
        let count = game.host.inventory.count(player, &item);
        game.host.inventory.configure(
            &state.actor,
            &InventoryEntry {
                item,
                count,
                capacity: 1.0,
                count_policy: None,
            },
        )?;
    }
    if pack == Q1MissionPack::Rogue {
        for (item, capacity) in [
            ("rogue:ammo/lava-nails", 200.0),
            ("rogue:ammo/multi-rockets", 100.0),
            ("rogue:ammo/plasma", 100.0),
            ("rogue:artifact/vengeance", 1.0),
        ] {
            let item = ItemId::from(item);
            let count = game.host.inventory.count(player, &item);
            game.host.inventory.configure(
                &state.actor,
                &InventoryEntry {
                    item,
                    count,
                    capacity,
                    count_policy: None,
                },
            )?;
        }
    }
    player_state_entity(game, player)?;
    Ok(())
}

/// Hipnotic attach hook.
fn attach_hipnotic(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    attach_for_pack(game, Q1MissionPack::Hipnotic, player)
}

/// Rogue attach hook.
fn attach_rogue(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    attach_for_pack(game, Q1MissionPack::Rogue, player)
}

/// Run mission-pack player prethink (`frame`).
fn frame_for_pack(
    game: &mut Q1EntityServices,
    pack: Q1MissionPack,
    player: &ActorId,
    seconds: f64,
) -> Result<(), Q1Error> {
    let state_entity = player_state_entity(game, player)?;
    MissionPackPlayers::for_pack(pack).enable_combos(game, player)?;
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    for timer in TIMERS.iter() {
        let expires = match state.powerups.get(&timer.id).copied() {
            Some(expires) => expires,
            None => continue,
        };
        let warned_key = format!("{}:warned", timer.id.as_str());
        let flash_key = format!("{}:flash", timer.id.as_str());
        let lost_key = format!("{}:lost", timer.id.as_str());
        let number = |game: &Q1EntityServices, key: &str| {
            game.entity_ref(&state_entity)
                .map(|entity| entity.number(key))
                .unwrap_or(0.0)
        };
        if expires < seconds + 3.0 && number(game, &warned_key) == 0.0 {
            mission_message(game, Some(player), timer.warn);
            game.sound(player, timer.sound, Q1SoundChannel::Auto, 1.0, 1.0)?;
            set_mission_number(game, &state_entity, &warned_key, 1.0)?;
        }
        if expires < seconds + 3.0 && number(game, &flash_key) < seconds {
            if let Some(body) = game.host.bodies.read(player) {
                game.effect(Q1Effect::Pickup, body.origin, Some(player), 1);
            }
            set_mission_number(game, &state_entity, &flash_key, seconds + 1.0)?;
        }
        if expires <= seconds && number(game, &lost_key) == 0.0 {
            mission_message(game, Some(player), timer.lost);
            set_mission_number(game, &state_entity, &lost_key, 1.0)?;
            if timer.id == Q1Powerup::RogueAntigrav {
                game.set_gravity(player, 1.0)?;
            }
        }
    }
    if state
        .powerups
        .get(&Q1Powerup::HipnoticWetsuit)
        .copied()
        .unwrap_or(0.0)
        > seconds
    {
        game.update_player(player, |state| state.air_finished = seconds + 12.0)?;
        if state.water_level >= 2 {
            let scuba = game
                .entity_ref(&state_entity)
                .map(|entity| entity.number("hipnotic:scuba"))
                .unwrap_or(0.0);
            if scuba < seconds {
                game.sound(player, "misc/wetsuit.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
                set_mission_number(game, &state_entity, "hipnotic:scuba", seconds + 7.0)?;
            }
            let scaled_time = game
                .entity_ref(&state_entity)
                .map(|entity| entity.text("hipnotic:scaled-time"))
                .unwrap_or_default();
            if let Some(body) = game.host.bodies.read(player) {
                if scaled_time != seconds.to_string() {
                    let scale = if state.water_level == 2 { 1.25 } else { 1.5 };
                    game.host.bodies.write(
                        &state.actor,
                        &BodyPatch {
                            velocity: Some(vscale(body.velocity, scale)),
                            ..Default::default()
                        }
                        .apply_to(&body),
                    )?;
                    game.update_entity(&state_entity, |entity| {
                        entity
                            .fields
                            .insert("hipnotic:scaled-time".to_string(), seconds.to_string());
                    })?;
                    set_mission_number(
                        game,
                        &state_entity,
                        "hipnotic:scaled-level",
                        state.water_level as f64,
                    )?;
                }
            }
        }
    }
    let empathy = state
        .powerups
        .get(&Q1Powerup::HipnoticEmpathy)
        .copied()
        .unwrap_or(0.0)
        > seconds;
    if game.entity_ref(player).is_some() {
        game.update_entity(player, |entity| {
            entity.effects = if empathy {
                entity.effects | 8
            } else {
                entity.effects & !8
            };
        })?;
    }
    Ok(())
}

/// Hipnotic frame hook.
fn frame_hipnotic(
    game: &mut Q1EntityServices,
    player: &ActorId,
    seconds: f64,
) -> Result<(), Q1Error> {
    frame_for_pack(game, Q1MissionPack::Hipnotic, player, seconds)
}

/// Rogue frame hook.
fn frame_rogue(game: &mut Q1EntityServices, player: &ActorId, seconds: f64) -> Result<(), Q1Error> {
    frame_for_pack(game, Q1MissionPack::Rogue, player, seconds)
}

/// Undo wetsuit velocity scaling after physics (`afterPhysics`).
fn after_physics_player(
    game: &mut Q1EntityServices,
    player: &ActorId,
    seconds: f64,
) -> Result<(), Q1Error> {
    let state_entity = player_state_entity(game, player)?;
    let scaled_time = game
        .entity_ref(&state_entity)
        .map(|entity| entity.text("hipnotic:scaled-time"))
        .unwrap_or_default();
    if scaled_time == seconds.to_string() {
        let state = game
            .player_ref(player)
            .cloned()
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        if let Some(body) = game.host.bodies.read(player) {
            let level = game
                .entity_ref(&state_entity)
                .map(|entity| entity.number("hipnotic:scaled-level"))
                .unwrap_or(0.0);
            game.host.bodies.write(
                &state.actor,
                &BodyPatch {
                    velocity: Some(vscale(body.velocity, if level == 2.0 { 0.8 } else { 0.66 })),
                    ..Default::default()
                }
                .apply_to(&body),
            )?;
        }
        game.update_entity(&state_entity, |entity| {
            entity.fields.remove("hipnotic:scaled-time");
        })?;
    }
    Ok(())
}

/// Flash the Rogue shield on a frontal hit (`shieldHit`).
fn shield_hit(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    let state_entity = player_state_entity(game, player)?;
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(()),
    };
    let death = game
        .entity_ref(&state_entity)
        .map(|entity| entity.number("rogue:shield-death"))
        .unwrap_or(0.0);
    if death <= game.time {
        let shield = game.create("power_shield", None, None)?;
        let delay = fround(game.time + 0.3);
        let owner = player.clone();
        game.update_entity(&shield, |shield| {
            shield.owner = Some(owner);
            shield.model = String::from("progs/p_shield.mdl");
            shield.delay = delay;
        })?;
        game.set_bounds(&shield, POINT)?;
        game.set_body(
            &shield,
            &BodyPatch {
                origin: Some(body.origin),
                angles: Some(body.angles),
                ..Default::default()
            },
        )?;
        game.link(&shield)?;
        set_mission_number(game, &state_entity, "rogue:shield-death", delay)?;
        let think = game.named.action("rogue:shield-think")?;
        game.schedule(&shield, 0.1, &think)?;
    }
    let sounded = game
        .entity_ref(&state_entity)
        .map(|entity| entity.number("rogue:shield-sound"))
        .unwrap_or(0.0);
    if sounded < game.time {
        game.sound(player, "shield/hit.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
        set_mission_number(game, &state_entity, "rogue:shield-sound", game.time + 0.5)?;
    }
    Ok(())
}

/// Step a shield flash (`rogue:shield-think`).
fn shield_think(game: &mut Q1EntityServices, shield: &ActorId) -> Result<(), Q1Error> {
    let record = game
        .entity_ref(shield)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let body = record
        .owner
        .as_ref()
        .and_then(|owner| game.host.bodies.read(owner));
    let Some(body) = body else {
        return game.remove(shield);
    };
    if record.delay < game.time {
        return game.remove(shield);
    }
    if record.delay - 0.25 <= game.time {
        game.update_entity(shield, |shield| shield.model.clear())?;
    } else {
        game.set_body(
            shield,
            &BodyPatch {
                origin: Some(body.origin),
                angles: Some(body.angles),
                ..Default::default()
            },
        )?;
    }
    let think = game.named.action("rogue:shield-think")?;
    game.schedule(shield, 0.05, &think)
}

/// Step a vengeance sphere (`sphereThink`).
fn sphere_think(game: &mut Q1EntityServices, sphere: &ActorId) -> Result<(), Q1Error> {
    let record = game
        .entity_ref(sphere)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let owner = record.owner.clone();
    let player = owner
        .as_ref()
        .and_then(|owner| game.player_ref(owner).cloned());
    let body = owner
        .as_ref()
        .and_then(|owner| game.host.bodies.read(owner));
    let (Some(player), Some(body)) = (player, body) else {
        return game.remove(sphere);
    };
    let player_id = player.actor.id().clone();
    if record.attack_finished < game.time {
        game.sound(sphere, "sphere/sphere.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
        let attack_finished = fround(game.time + 4.0);
        game.update_entity(sphere, |sphere| sphere.attack_finished = attack_finished)?;
    }
    if game.time > record.delay || game.health(&player_id) < 1.0 {
        game.host.inventory.consume(
            &player.actor,
            &ItemId::from("rogue:artifact/vengeance"),
            1.0,
        );
        if game.time > record.delay {
            mission_message(game, Some(&player_id), "$qc_vengeance_lost");
            return game.remove(sphere);
        }
        let state_entity = player_state_entity(game, &player_id)?;
        let mut killer = mission_reference(game, &state_entity, "rogue:killer");
        if killer
            .as_ref()
            .is_some_and(|killer| !game.is_player(killer))
        {
            killer = killer
                .as_ref()
                .and_then(|killer| game.entity_ref(killer))
                .and_then(|entity| entity.owner.clone());
        }
        let killer = match killer {
            Some(killer) if game.is_player(&killer) => killer,
            _ => return game.remove(sphere),
        };
        set_mission_reference(game, sphere, "rogue:enemy", Some(&killer))?;
        return sphere_attack(game, sphere);
    }
    let center = vadd(
        body.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 48.0,
        },
    );
    let source = game.body(sphere)?.origin;
    if record.count < 0.0 || record.count > 3.0 {
        game.update_entity(sphere, |sphere| sphere.count = 0.0)?;
    }
    let count = game
        .entity_ref(sphere)
        .map(|sphere| sphere.count)
        .unwrap_or(0.0);
    let trace = game.host.trace(&Q1TraceRequest {
        start: source,
        end: center,
        bounds: POINT,
        ignore: None,
        monsters: false,
        missile: false,
    });
    if trace.fraction < 1.0 {
        game.set_origin(sphere, center)?;
        game.update_entity(sphere, |sphere| sphere.count += 1.0)?;
    } else {
        let offset = if count == 0.0 {
            Vec3 {
                x: 16.0,
                y: 0.0,
                z: 0.0,
            }
        } else if count == 1.0 {
            Vec3 {
                x: 0.0,
                y: 16.0,
                z: 0.0,
            }
        } else if count == 2.0 {
            Vec3 {
                x: -16.0,
                y: 0.0,
                z: 0.0,
            }
        } else {
            Vec3 {
                x: 0.0,
                y: -16.0,
                z: 0.0,
            }
        };
        let direction = vsub(vadd(center, offset), source);
        let distance = f64::from(length(direction));
        if distance < 8.0 {
            game.update_entity(sphere, |sphere| sphere.count += 1.0)?;
        } else {
            move_missile(
                game,
                sphere,
                vscale(
                    normalize(direction),
                    if distance < 50.0 { 150.0 } else { 500.0 },
                ),
            )?;
        }
    }
    let think = game.named.action("rogue:sphere-think")?;
    game.schedule(sphere, 0.1, &think)
}

/// Steer a vengeance sphere at its killer (`sphereAttack`).
fn sphere_attack(game: &mut Q1EntityServices, sphere: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(sphere, |sphere| sphere.solid = Q1Solid::Trigger)?;
    let touch = game.named.touch("rogue:sphere-impact")?;
    game.update_entity(sphere, |sphere| sphere.touch = Some(touch))?;
    game.link(sphere)?;
    let target = mission_reference(game, sphere, "rogue:enemy");
    let body = target
        .as_ref()
        .and_then(|target| game.host.bodies.read(target));
    match (target, body) {
        (Some(target), Some(body)) if game.health(&target) >= 1.0 => {
            move_missile(
                game,
                sphere,
                vscale(
                    normalize(vsub(
                        vadd(
                            body.origin,
                            Vec3 {
                                x: 0.0,
                                y: 0.0,
                                z: 22.0,
                            },
                        ),
                        game.body(sphere)?.origin,
                    )),
                    650.0,
                ),
            )?;
        }
        _ => {
            let owner = game
                .entity_ref(sphere)
                .and_then(|sphere| sphere.owner.clone());
            mission_message(game, owner.as_ref(), "$qc_you_are_denied_vengeance");
            return game.remove(sphere);
        }
    }
    let attack = game.named.action("rogue:sphere-attack")?;
    game.schedule(sphere, 0.1, &attack)
}

/// Detonate a vengeance sphere (`rogue:sphere-impact`).
fn sphere_impact(
    game: &mut Q1EntityServices,
    sphere: &ActorId,
    target: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game.health(target) != 0.0 {
        let params = Q1DamageParams {
            death_type: String::from("rogue:vengeance"),
            ..Default::default()
        };
        let _ = game.damage(target, Some(sphere), Some(sphere), 1000.0, &params);
    }
    game.radius_damage(
        sphere,
        Some(sphere),
        300.0,
        Some(target),
        None,
        "rogue:vengeance",
    );
    let body = game.body(sphere)?;
    game.effect(
        Q1Effect::Explosion,
        vsub(body.origin, vscale(normalize(body.velocity), 8.0)),
        None,
        1,
    );
    game.remove(sphere)
}

impl MissionPackPlayers {
    /// Register mission-pack player services on a game.
    pub fn new(game: &mut Q1EntityServices, pack: Q1MissionPack) -> Result<Self, Q1Error> {
        game.register_player_extension(Q1PlayerExtension {
            id: format!("q1:{}:players", pack.as_str()),
            attach: Some(if pack == Q1MissionPack::Hipnotic {
                attach_hipnotic
            } else {
                attach_rogue
            }),
            frame: Some(if pack == Q1MissionPack::Hipnotic {
                frame_hipnotic
            } else {
                frame_rogue
            }),
            after_physics: Some(after_physics_player),
            ..Default::default()
        })?;
        game.named.register(
            "rogue:shield-think",
            Q1CallbackHandlers {
                action: Some(shield_think),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:sphere-think",
            Q1CallbackHandlers {
                action: Some(sphere_think),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:sphere-attack",
            Q1CallbackHandlers {
                action: Some(sphere_attack),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:sphere-impact",
            Q1CallbackHandlers {
                touch: Some(sphere_impact),
                ..Default::default()
            },
        )?;
        Ok(Self { pack })
    }

    /// Service view for a pack without registering.
    #[must_use]
    pub fn for_pack(pack: Q1MissionPack) -> Self {
        Self { pack }
    }

    /// Enable Rogue combo weapons (`enableCombos`).
    pub fn enable_combos(
        &self,
        game: &mut Q1EntityServices,
        player: &ActorId,
    ) -> Result<(), Q1Error> {
        if self.pack != Q1MissionPack::Rogue {
            return Ok(());
        }
        let state = game
            .player_ref(player)
            .cloned()
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        for combo in COMBOS.iter() {
            if game
                .host
                .inventory
                .count(player, &weapon_item(combo.powered))
                == 0.0
                && game.host.inventory.count(player, &weapon_item(combo.base)) > 0.0
                && game.host.inventory.count(player, &ItemId::from(combo.ammo)) > 0.0
            {
                game.host
                    .inventory
                    .give(&state.actor, &weapon_item(combo.powered), 1.0);
                mission_message(game, Some(player), combo.message);
            }
        }
        Ok(())
    }

    /// Grant a timed powerup (`powerup`).
    pub fn powerup(
        &self,
        game: &mut Q1EntityServices,
        player: &ActorId,
        powerup: Q1Powerup,
        seconds: f64,
    ) -> Result<(), Q1Error> {
        let state_entity = player_state_entity(game, player)?;
        game.update_entity(&state_entity, |entity| {
            entity
                .fields
                .remove(&format!("{}:warned", powerup.as_str()));
            entity.fields.remove(&format!("{}:lost", powerup.as_str()));
        })?;
        if powerup == Q1Powerup::RogueAntigrav {
            game.set_gravity(player, 0.25)?;
        }
        game.give_powerup(player, powerup, seconds)
    }

    /// Gravity scale for a player (`gravityScale`).
    #[must_use]
    pub fn gravity_scale(&self, game: &Q1EntityServices, player: &ActorId) -> f64 {
        let antigrav = game
            .player_ref(player)
            .and_then(|player| player.powerups.get(&Q1Powerup::RogueAntigrav).copied())
            .unwrap_or(0.0);
        if antigrav > game.time { 0.25 } else { 1.0 }
    }

    /// Quad preparation stage (`beforeQuad`).
    pub fn before_quad(
        &self,
        game: &mut Q1EntityServices,
        request: &DamageRequest,
        damage: f64,
    ) -> DamagePreparation {
        let player = match game.player_ref(&request.target) {
            Some(player) => player.clone(),
            None => return DamagePreparation::Continue { amount: damage },
        };
        let discharge = matches!(&request.attack.cause, AttackCause::Q1 { death_type, .. } if death_type == "discharge");
        if self.pack == Q1MissionPack::Hipnotic
            && discharge
            && player
                .powerups
                .get(&Q1Powerup::HipnoticWetsuit)
                .copied()
                .unwrap_or(0.0)
                != 0.0
        {
            return DamagePreparation::Cancel;
        }
        if self.pack == Q1MissionPack::Rogue
            && player
                .powerups
                .get(&Q1Powerup::RogueShield)
                .copied()
                .unwrap_or(0.0)
                != 0.0
        {
            if let Some(inflictor) = request.attack.inflictor.as_ref() {
                let body = game.host.bodies.read(&request.target);
                let incoming = game.host.bodies.read(inflictor);
                if let (Some(body), Some(incoming)) = (body, incoming) {
                    let hit_angle = yaw_for(vsub(incoming.origin, body.origin))
                        - yaw_for(game.make_vectors(body.angles).forward);
                    if !(hit_angle > 90.0 && hit_angle < 270.0
                        || hit_angle < -90.0 && hit_angle > -270.0)
                    {
                        let lava_spike = game.host.classname(inflictor) == "lava_spike";
                        let _ = shield_hit(game, &request.target);
                        return DamagePreparation::Continue {
                            amount: fround(damage * if lava_spike { 0.7 } else { 0.3 }),
                        };
                    }
                }
            }
        }
        DamagePreparation::Continue { amount: damage }
    }

    /// Post-quad preparation stage (`afterQuad`).
    pub fn after_quad(
        &self,
        game: &mut Q1EntityServices,
        request: &DamageRequest,
        damage: f64,
    ) -> DamagePreparation {
        let player = match game.player_ref(&request.target) {
            Some(player) => player.clone(),
            None => return DamagePreparation::Continue { amount: damage },
        };
        let attacker = request.attack.attacker.clone();
        if attacker.is_some() {
            if let Ok(state_entity) = player_state_entity(game, &request.target) {
                let _ =
                    set_mission_reference(game, &state_entity, "rogue:killer", attacker.as_ref());
            }
        }
        let inflictor = request.attack.inflictor.clone();
        if self.pack == Q1MissionPack::Hipnotic {
            if let Some(attacker) = attacker.as_ref() {
                let empathied = player
                    .powerups
                    .get(&Q1Powerup::HipnoticEmpathy)
                    .copied()
                    .unwrap_or(0.0)
                    != 0.0;
                let inflictor_empathied = inflictor
                    .as_ref()
                    .and_then(|inflictor| game.player_ref(inflictor))
                    .and_then(|player| player.powerups.get(&Q1Powerup::HipnoticEmpathy).copied())
                    .unwrap_or(0.0)
                    != 0.0;
                if !same_actor(attacker, &request.target)
                    && empathied
                    && (inflictor.is_none() || !inflictor_empathied)
                {
                    let reflected = fround(damage / 2.0);
                    let params = Q1DamageParams {
                        death_type: String::from("hipnotic:empathy"),
                        ..Default::default()
                    };
                    let _ = game.damage(
                        attacker,
                        Some(&request.target),
                        Some(&request.target),
                        reflected,
                        &params,
                    );
                    return DamagePreparation::Continue { amount: reflected };
                }
            }
        }
        DamagePreparation::Continue { amount: damage }
    }

    /// Grant a vengeance sphere (`sphere`).
    pub fn sphere(
        &self,
        game: &mut Q1EntityServices,
        item: &ActorId,
        player: &ActorId,
    ) -> Result<bool, Q1Error> {
        let state = game
            .player_ref(player)
            .cloned()
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        if game
            .host
            .inventory
            .give(&state.actor, &ItemId::from("rogue:artifact/vengeance"), 1.0)
            == 0.0
        {
            return Ok(false);
        }
        let sphere = game.create("Vengeance", None, None)?;
        let delay = fround(game.time + 30.0);
        let owner = player.clone();
        game.update_entity(&sphere, |sphere| {
            sphere.owner = Some(owner);
            sphere.model = String::from("progs/sphere.mdl");
            sphere.movement = Q1MoveType::Flymissile;
            sphere.solid = Q1Solid::None;
            sphere.angular_velocity = Vec3 {
                x: 40.0,
                y: 40.0,
                z: 40.0,
            };
            sphere.delay = delay;
        })?;
        game.set_origin(&sphere, game.body(item)?.origin)?;
        let think = game.named.action("rogue:sphere-think")?;
        game.schedule(&sphere, 0.1, &think)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_core::time::SourceTime;

    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;
    use crate::q1::foundation::gameplay::{AttackProvenance, DamageDelivery};

    fn attached_player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default())
            .expect("attach");
        player
    }

    fn damage_request(target: ActorId, death_type: &str) -> DamageRequest {
        DamageRequest {
            attack: AttackProvenance {
                sequence: 0,
                time: SourceTime::Seconds(0.0),
                attacker: None,
                inflictor: None,
                originating_projectile: None,
                weapon: None,
                weapon_provider: ProviderId::new("q1", "campaign"),
                damage_powerup_owner: None,
                combat_provider: ProviderId::new("q1", "combat"),
                inventory_provider: ProviderId::new("q1", "inventory"),
                movement_provider: ProviderId::new("q1", "movement"),
                cause: AttackCause::Q1 {
                    death_type: death_type.to_string(),
                    armor_effect: None,
                },
            },
            target,
            amount: 10.0,
            knockback: 10.0,
            direction: ZERO,
            point: ZERO,
            normal: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            delivery: DamageDelivery::Direct,
        }
    }

    #[test]
    fn players_register_callbacks() {
        let mut game = test_game();
        MissionPackPlayers::new(&mut game, Q1MissionPack::Hipnotic).expect("players");
        assert!(game.named.action("rogue:shield-think").is_ok());
        assert!(game.named.touch("rogue:sphere-impact").is_ok());
    }

    #[test]
    fn combos_grant_powered_weapons() {
        let mut game = test_game();
        let players = MissionPackPlayers::for_pack(Q1MissionPack::Rogue);
        let player = attached_player(&mut game);
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.host
            .inventory
            .give(&owned, &weapon_item(Q1Weapon::Nailgun), 1.0);
        game.host
            .inventory
            .give(&owned, &ItemId::from("rogue:ammo/lava-nails"), 10.0);
        players.enable_combos(&mut game, &player).expect("combos");
        assert_eq!(
            game.host
                .inventory
                .count(&player, &weapon_item(Q1Weapon::RogueLavaNailgun)),
            1.0
        );
    }

    #[test]
    fn wetsuit_cancels_discharge() {
        let mut game = test_game();
        let players = MissionPackPlayers::for_pack(Q1MissionPack::Hipnotic);
        let player = attached_player(&mut game);
        players
            .powerup(&mut game, &player, Q1Powerup::HipnoticWetsuit, 30.0)
            .expect("powerup");
        let request = damage_request(player.clone(), "discharge");
        assert_eq!(
            players.before_quad(&mut game, &request, 10.0),
            DamagePreparation::Cancel
        );
        let request = damage_request(player, "electric");
        assert_eq!(
            players.before_quad(&mut game, &request, 10.0),
            DamagePreparation::Continue { amount: 10.0 }
        );
    }

    #[test]
    fn empathy_reflects_half_damage() {
        let mut game = test_game();
        let players = MissionPackPlayers::for_pack(Q1MissionPack::Hipnotic);
        let target = attached_player(&mut game);
        let attacker = attached_player(&mut game);
        players
            .powerup(&mut game, &target, Q1Powerup::HipnoticEmpathy, 30.0)
            .expect("powerup");
        let mut request = damage_request(target.clone(), "electric");
        request.attack.attacker = Some(attacker);
        assert_eq!(
            players.after_quad(&mut game, &request, 10.0),
            DamagePreparation::Continue {
                amount: fround(10.0 / 2.0)
            }
        );
    }
}
