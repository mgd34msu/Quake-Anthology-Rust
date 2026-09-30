//! Q1 mg3 weapons (`src/content/q1/addons/items/weapons.ts`).
//!
//! `quakec_mg3/weapon.qc` and `weapons.qc`. GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::addons::context::{
    addon_player_number, addon_player_reference, fround, set_addon_number, set_addon_player_number,
    set_addon_player_reference,
};
use crate::q1::addons::items::pickups::{MG3_BLOODY_SHOTGUN, MG3_BLOODY_SUPER_SHOTGUN};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::extensions::{Q1WeaponDefinition, Q1WeaponRules};
use crate::q1::foundation::types::{
    normalize, vadd, vscale, vsub, Q1Effect, Q1Event, Q1SoundChannel, Q1TraceRequest, Q1Weapon, POINT,
};
use crate::q1::foundation::weapons::{aim, fire_base_weapon, fire_bullets};
use crate::q1::missionpacks::hipnotic_weapons::{
    launch_hipnotic_laser, register_hipnotic_hammer_callbacks, register_hipnotic_laser_callbacks,
    spawn_hipnotic_hammer_base, HipnoticLaserProfile,
};
use crate::q1::Q1Error;

/// Infinite ammo check (`infiniteAmmo`).
fn infinite_ammo(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    Ok(addon_player_number(game, player, "infiniteammo")? != 0.0)
}

/// Emits the weapon view (`emitWeapon`).
fn emit_weapon(game: &mut Q1EntityServices, player: &ActorId, punch: i32) -> Result<(), Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 weapon player"))?;
    let view_model = game.weapon_model(state.weapon, Some(player))?;
    game.host.emit(Q1Event::Weapon {
        player: player.clone(),
        weapon: state.weapon,
        view_model,
        frame: state.weapon_frame,
        punch,
        attack: None,
    });
    Ok(())
}

/// Resolves the mjolnir body frame (`mg3HammerBodyFrame`).
pub fn mg3_hammer_body_frame(game: &mut Q1EntityServices, player: &ActorId) -> Result<Option<i32>, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 hammer player"))?;
    if state.weapon != Q1Weapon::Mg3Mjolnir
        || state.weapon_animation_at < 0.0
        || state.weapon_frame < 1
        || state.weapon_frame > 4
    {
        return Ok(None);
    }
    Ok(Some(
        addon_player_number(game, player, "mg3.hammerBodyBase")? as i32 + state.weapon_frame,
    ))
}

/// Fires the mg3 laser (`fireLaser`).
fn fire_laser(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 laser player"))?;
    if !infinite_ammo(game, player)?
        && !game
            .host
            .inventory
            .consume(&state.actor, &String::from("q1:ammo/cells"), 1.0)
    {
        return Ok(false);
    }
    game.sound(state.actor.id(), "weapons/laserg.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let delay = game.weapon_attack_delay(player, 0.7)?;
    let time = game.time;
    game.update_player(player, |state| {
        state.attack_finished = fround(time + delay);
        state.weapon_frame = 1;
    })?;
    emit_weapon(game, player, -1)?;
    let profile = HipnoticLaserProfile {
        weapon: Q1Weapon::Mg3Laser,
        damage: 15.0,
        light_damage: 20.0,
    };
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(false),
    };
    let basis = game.make_vectors(state.view_angles);
    let outward = normalize(Vec3 {
        x: basis.forward.x,
        y: basis.forward.y,
        z: 0.0,
    });
    let origin = vadd(vadd(body.origin, vscale(basis.up, 6.0)), vscale(outward, 12.0));
    let direction = aim(game, &state.actor, basis.forward);
    let offset = 6.0 * 0.707;
    let first = vsub(vadd(origin, vscale(basis.right, offset)), vscale(basis.up, offset));
    launch_hipnotic_laser(game, player, first, direction, false, Some(&profile))?;
    launch_hipnotic_laser(
        game,
        player,
        vsub(first, vscale(basis.right, offset * 2.0)),
        direction,
        false,
        Some(&profile),
    )?;
    let body = game.host.bodies.read(player);
    if let Some(body) = body {
        game.effect(Q1Effect::Muzzleflash, body.origin, Some(player), 1);
    }
    Ok(true)
}

/// Strikes with the mjolnir (`hammerStrike`).
fn hammer_strike(game: &mut Q1EntityServices, strike: &ActorId) -> Result<(), Q1Error> {
    let owner = game.entity_ref(strike).and_then(|entity| entity.owner.clone());
    let Some(owner) = owner else {
        return game.remove(strike);
    };
    let state = game.player_ref(&owner).cloned();
    let body = game.host.bodies.read(&owner);
    let (Some(state), Some(body)) = (state, body) else {
        return game.remove(strike);
    };
    let basis = game.make_vectors(state.view_angles);
    let source = vadd(
        body.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let trace = game.host.trace(&Q1TraceRequest {
        start: source,
        end: vadd(source, vscale(basis.forward, 64.0)),
        bounds: POINT,
        ignore: Some(owner.clone()),
        monsters: true,
        missile: false,
    });
    let target = trace.actor;
    let origin = vsub(trace.end, vscale(basis.forward, 4.0));
    let delay = game.weapon_attack_delay(&owner, 0.4)?;
    let time = game.time;
    game.update_player(&owner, |state| {
        state.attack_finished = fround(time + delay);
    })?;
    let hit = target.clone();
    let damageable = hit.as_ref().is_some_and(|target| {
        game.host
            .combat
            .read(target)
            .is_some_and(|combat| combat.can_take_damage)
    });
    if let Some(target) = hit.filter(|_| damageable) {
        game.sound(&owner, "hipweap/mjolslap.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        game.effect(Q1Effect::Blood, origin, Some(&target), 40);
        set_addon_number(game, &target, "axhitme", 1.0)?;
        let classname = game.host.classname(&target);
        let health = game.health(&target);
        let damage = if classname == "monster_zombie" || classname == "monster_szombie" {
            120.0
        } else if health - 40.0 < 0.0 {
            80.0
        } else {
            40.0
        };
        let last = addon_player_reference(game, &owner, "last_mjolnir_hit")?;
        if addon_player_number(game, &owner, "last_mjolnir_hit_time")? > fround(game.time) {
            if last.as_ref().is_some_and(|last| same_actor(last, &target))
                && state.water_level < 2
                && game.host.inventory.count(&owner, &String::from("q1:ammo/cells")) >= 15.0
            {
                let floor = game.host.trace(&Q1TraceRequest {
                    start: source,
                    end: vsub(
                        source,
                        vscale(basis.up, if state.water_level < 1 { 30.0 } else { 15.0 }),
                    ),
                    bounds: POINT,
                    ignore: Some(owner.clone()),
                    monsters: true,
                    missile: false,
                });
                if !infinite_ammo(game, &owner)? {
                    game.host
                        .inventory
                        .consume(&state.actor, &String::from("q1:ammo/cells"), 15.0);
                }
                spawn_hipnotic_hammer_base(game, &owner, floor.end, Q1Weapon::Mg3Mjolnir)?;
            } else {
                game.damage(
                    &target,
                    Some(&owner),
                    Some(&owner),
                    damage,
                    &Q1DamageParams {
                        weapon: Some(Q1Weapon::Mg3Mjolnir),
                        ..Default::default()
                    },
                );
            }
            set_addon_player_number(game, &owner, "last_mjolnir_hit_time", game.time)?;
            let delay = game.weapon_attack_delay(&owner, 0.5)?;
            let time = game.time;
            game.update_player(&owner, |state| {
                state.attack_finished = fround(time + delay);
            })?;
        } else {
            set_addon_player_number(game, &owner, "last_mjolnir_hit_time", game.time + 0.5)?;
            set_addon_player_reference(game, &owner, "last_mjolnir_hit", Some(&target))?;
            let delay = game.weapon_attack_delay(&owner, 0.2)?;
            let time = game.time;
            game.update_player(&owner, |state| {
                state.attack_finished = fround(time + delay);
            })?;
            if game.host.inventory.count(&owner, &String::from("q1:ammo/cells")) >= 15.0 {
                set_addon_player_number(game, &owner, "mg3.hammerGlow", 1.0)?;
            }
            game.damage(
                &target,
                Some(&owner),
                Some(&owner),
                damage,
                &Q1DamageParams {
                    weapon: Some(Q1Weapon::Mg3Mjolnir),
                    ..Default::default()
                },
            );
        }
    } else {
        if trace.fraction != 1.0 {
            game.sound(&owner, "hipweap/mjoltink.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
            game.effect(Q1Effect::Gunshot, origin, None, 1);
        } else {
            game.sound(&owner, "weapons/ax1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        }
        set_addon_player_reference(game, &owner, "last_mjolnir_hit", None)?;
    }
    emit_weapon(game, &owner, 0)?;
    game.remove(strike)
}

fn hammer_strike_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    hammer_strike(game, id)
}

/// Fires the mjolnir (`fireHammer`).
fn fire_hammer(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let strike = game.create("mg3_hammer_strike", None, None)?;
    game.update_entity(&strike, |entity| entity.owner = Some(player.clone()))?;
    let base = if game.host.inventory.count(player, &String::from("q1:ammo/cells")) < 30.0 {
        31.0
    } else {
        37.0
    };
    set_addon_player_number(game, player, "mg3.hammerBodyBase", base)?;
    game.schedule(&strike, 0.2, "mg3:hammer_strike")?;
    let delay = game.weapon_attack_delay(player, 0.5)?;
    let time = game.time;
    game.update_player(player, |state| {
        state.continuous_firing = false;
        state.weapon_animation_at = time;
        state.weapon_animation_base = 1;
        state.weapon_frame = 1;
        state.attack_finished = fround(time + delay);
        state.hostile_until = fround(time + 1.0);
    })?;
    emit_weapon(game, player, 0)?;
    Ok(true)
}

/// Mjolnir view model (`mjolnirModelFor`).
fn mjolnir_model_for(game: &mut Q1EntityServices, player: &ActorId) -> Result<String, Q1Error> {
    let glow = addon_player_number(game, player, "mg3.hammerGlow")? != 0.0
        && addon_player_number(game, player, "last_mjolnir_hit_time")? > fround(game.time);
    Ok(String::from(if glow {
        "progs/v_hammer_glow.mdl"
    } else {
        "progs/v_hammer.mdl"
    }))
}

/// Mjolnir animation (`mjolnirAnimate`).
fn mjolnir_animate(game: &mut Q1EntityServices, player: &ActorId, seconds: f64) -> Result<(), Q1Error> {
    let started = game
        .player_ref(player)
        .map(|state| state.weapon_animation_at)
        .unwrap_or(-1.0);
    if started < 0.0 {
        return Ok(());
    }
    let step = ((seconds - started) / 0.1).floor() as i32;
    let frame = if step >= 4 { 0 } else { step + 1 };
    let current = game.player_ref(player).map(|state| state.weapon_frame).unwrap_or(0);
    if frame != current {
        game.update_player(player, |state| state.weapon_frame = frame)?;
        emit_weapon(game, player, 0)?;
    }
    if step >= 4 {
        game.update_player(player, |state| state.weapon_animation_at = -1.0)?;
    }
    Ok(())
}

/// Fires shotguns (`fireShotgun`).
fn fire_shotgun(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 shotgun player"))?;
    let count = game.host.inventory.count(player, &String::from("q1:ammo/shells"));
    let bloody = addon_player_number(game, player, "parm15")? as i32;
    if state.weapon == Q1Weapon::Supershotgun && bloody & MG3_BLOODY_SUPER_SHOTGUN != 0 && count > 1.0 {
        let body = match game.host.bodies.read(player) {
            Some(body) => body,
            None => return Ok(false),
        };
        if !infinite_ammo(game, player)? {
            game.host
                .inventory
                .consume(&state.actor, &String::from("q1:ammo/shells"), 2.0);
        }
        game.sound(
            state.actor.id(),
            "weapons/shotgn2.wav",
            Q1SoundChannel::Weapon,
            1.0,
            1.0,
        )?;
        let basis = game.make_vectors(state.view_angles);
        let direction = aim(game, &state.actor, basis.forward);
        fire_bullets(
            game,
            &state.actor,
            direction,
            state.view_angles,
            28,
            0.3,
            0.08,
            Some(Q1Weapon::Supershotgun),
        );
        let delay = game.weapon_attack_delay(player, 0.7)?;
        let time = game.time;
        game.update_player(player, |state| {
            state.continuous_firing = false;
            state.weapon_animation_at = time;
            state.weapon_animation_base = 1;
            state.weapon_frame = 1;
            state.attack_finished = fround(time + delay);
            state.hostile_until = fround(time + 1.0);
        })?;
        emit_weapon(game, player, -4)?;
        game.effect(Q1Effect::Muzzleflash, body.origin, Some(player), 1);
        return Ok(true);
    }
    let fired = fire_base_weapon(game, player)?;
    if fired && state.weapon == Q1Weapon::Shotgun && bloody & MG3_BLOODY_SHOTGUN != 0 {
        let delay = game.weapon_attack_delay(player, 0.28)?;
        let time = game.time;
        game.update_player(player, |state| {
            state.attack_finished = fround(time + delay);
        })?;
    }
    Ok(fired)
}

/// Shotgun availability (`shotgunBestAvailable`).
fn shotgun_best_available(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    let _ = game;
    let _ = player;
    Ok(true)
}

/// Super shotgun availability (`superShotgunBestAvailable`).
fn super_shotgun_best_available(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    Ok(game.host.inventory.count(player, &String::from("q1:ammo/shells")) >= 2.0)
}

/// Shotgun view model (`shotgunModelFor`).
fn shotgun_model_for(game: &mut Q1EntityServices, player: &ActorId) -> Result<String, Q1Error> {
    let bloody = addon_player_number(game, player, "parm15")? as i32;
    Ok(String::from(if bloody & MG3_BLOODY_SHOTGUN != 0 {
        "progs/v_bloodshot.mdl"
    } else {
        "progs/v_shot.mdl"
    }))
}

/// Super shotgun view model (`superShotgunModelFor`).
fn super_shotgun_model_for(game: &mut Q1EntityServices, player: &ActorId) -> Result<String, Q1Error> {
    let bloody = addon_player_number(game, player, "parm15")? as i32;
    Ok(String::from(if bloody & MG3_BLOODY_SUPER_SHOTGUN != 0 {
        "progs/v_bloodshot2.mdl"
    } else {
        "progs/v_shot2.mdl"
    }))
}

/// MG3 ammunition consumption (`mg3ConsumeAmmo`).
fn mg3_consume_ammo(
    game: &mut Q1EntityServices,
    player: &ActorId,
    item: &crate::contract::ItemId,
    amount: f64,
) -> Result<bool, Q1Error> {
    if infinite_ammo(game, player)? {
        return Ok(true);
    }
    let owned = game
        .player_owned(player)
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 ammo player"))?;
    Ok(game.host.inventory.consume(&owned, item, amount))
}

/// Registers mg3 weapons (`registerMg3Weapons`).
pub fn register_mg3_weapons(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.register_weapon_rules(Q1WeaponRules {
        id: String::from("q1:mg3:ammunition"),
        consume_ammo: Some(mg3_consume_ammo),
        ..Default::default()
    })?;
    register_hipnotic_laser_callbacks(game)?;
    register_hipnotic_hammer_callbacks(game)?;
    game.named.register(
        "mg3:hammer_strike",
        crate::q1::foundation::callbacks::Q1CallbackHandlers {
            action: Some(hammer_strike_action as crate::q1::foundation::callbacks::Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.register_weapon(Q1WeaponDefinition {
        id: Q1Weapon::Mg3Laser,
        item: None,
        ammo: Some(String::from("q1:ammo/cells")),
        ammo_per_shot: None,
        model: String::from("progs/v_laserg.mdl"),
        rank: 3,
        model_for: None,
        available: None,
        best_available: None,
        fire: fire_laser,
        animate: None,
    })?;
    game.register_weapon(Q1WeaponDefinition {
        id: Q1Weapon::Mg3Mjolnir,
        item: None,
        ammo: Some(String::from("q1:ammo/cells")),
        ammo_per_shot: Some(0.0),
        model: String::from("progs/v_hammer.mdl"),
        rank: 8,
        model_for: Some(mjolnir_model_for),
        available: None,
        best_available: None,
        fire: fire_hammer,
        animate: Some(mjolnir_animate),
    })?;
    game.replace_weapon(Q1WeaponDefinition {
        id: Q1Weapon::Shotgun,
        item: None,
        ammo: Some(String::from("q1:ammo/shells")),
        ammo_per_shot: Some(1.0),
        model: String::from("progs/v_shot.mdl"),
        rank: 8,
        model_for: Some(shotgun_model_for),
        available: None,
        best_available: Some(shotgun_best_available),
        fire: fire_shotgun,
        animate: None,
    })?;
    game.replace_weapon(Q1WeaponDefinition {
        id: Q1Weapon::Supershotgun,
        item: None,
        ammo: Some(String::from("q1:ammo/shells")),
        ammo_per_shot: Some(1.0),
        model: String::from("progs/v_shot2.mdl"),
        rank: 6,
        model_for: Some(super_shotgun_model_for),
        available: None,
        best_available: Some(super_shotgun_best_available),
        fire: fire_shotgun,
        animate: None,
    })?;
    game.register_weapon_order(
        "q1:mg3",
        vec![
            Q1Weapon::Lightning,
            Q1Weapon::Supernailgun,
            Q1Weapon::Supershotgun,
            Q1Weapon::Nailgun,
            Q1Weapon::Shotgun,
            Q1Weapon::Mg3Mjolnir,
            Q1Weapon::Axe,
        ],
    )
}

/// Ticks mg3 weapon state (`mg3WeaponFrame`).
pub fn mg3_weapon_frame(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    if game.health(player) <= 0.0 {
        return Ok(());
    }
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 weapon frame player"))?;
    if state.weapon == Q1Weapon::Mg3Mjolnir
        && addon_player_number(game, player, "mg3.hammerGlow")? != 0.0
        && addon_player_number(game, player, "last_mjolnir_hit_time")? <= fround(game.time)
    {
        set_addon_player_number(game, player, "mg3.hammerGlow", 0.0)?;
        emit_weapon(game, player, 0)?;
    }
    if game.time <= state.attack_finished || state.weapon == Q1Weapon::Axe || state.weapon == Q1Weapon::Mg3Mjolnir {
        return Ok(());
    }
    let ammo = game.weapon_ammo(state.weapon);
    let Some(ammo) = ammo else {
        return Ok(());
    };
    if game.host.inventory.count(player, &ammo) != 0.0 {
        return Ok(());
    }
    let name = if ammo == "q1:ammo/shells" {
        Some("item_shells")
    } else if ammo == "q1:ammo/nails" {
        Some("item_spikes")
    } else if ammo == "q1:ammo/rockets" {
        Some("item_rockets")
    } else if ammo == "q1:ammo/cells" {
        Some("item_cells")
    } else {
        None
    };
    if let Some(name) = name {
        let ids = game.entity_ids();
        for id in ids {
            let hidden = game.entity_ref(&id).is_some_and(|entity| {
                entity.classname == name
                    && entity.spawnflags & 8 != 0
                    && entity.solid == crate::q1::foundation::types::Q1Solid::None
            });
            if hidden {
                let delay = 0.5 * game.host.random();
                game.schedule(&id, delay, "SUB_regen")?;
            }
        }
    }
    let best = game.choose_best(&state.actor, None)?;
    game.select_weapon(&state.actor, best)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn stub_fire(_game: &mut Q1EntityServices, _player: &ActorId) -> Result<bool, Q1Error> {
        Ok(false)
    }

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg3);
        for (id, model) in [
            (Q1Weapon::Shotgun, "progs/v_shot.mdl"),
            (Q1Weapon::Supershotgun, "progs/v_shot2.mdl"),
        ] {
            game.register_weapon(Q1WeaponDefinition {
                id,
                item: None,
                ammo: Some(String::from("q1:ammo/shells")),
                ammo_per_shot: Some(1.0),
                model: String::from(model),
                rank: 1,
                model_for: None,
                available: None,
                best_available: None,
                fire: stub_fire,
                animate: None,
            })
            .expect("base arsenal");
        }
        register_mg3_weapons(game).expect("weapons");
        guard
    }

    #[test]
    fn hammer_body_frame_tracks_mjolnir() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        assert_eq!(mg3_hammer_body_frame(&mut game, &player).expect("frame"), None);
        game.update_player(&player, |state| {
            state.weapon = Q1Weapon::Mg3Mjolnir;
            state.weapon_animation_at = 0.0;
            state.weapon_frame = 2;
        })
        .expect("weapon");
        set_addon_player_number(&mut game, &player, "mg3.hammerBodyBase", 31.0).expect("base");
        assert_eq!(mg3_hammer_body_frame(&mut game, &player).expect("frame"), Some(33));
    }

    #[test]
    fn laser_consumes_cells() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        assert!(!fire_laser(&mut game, &player).expect("fire"));
        let owned = game.player_owned(&player).expect("owned");
        game.host
            .inventory
            .configure(
                &owned,
                &crate::contract::InventoryEntry {
                    item: String::from("q1:ammo/cells"),
                    count: 10.0,
                    capacity: 100.0,
                    count_policy: None,
                },
            )
            .expect("cells");
        assert!(fire_laser(&mut game, &player).expect("fire"));
        assert_eq!(game.host.inventory.count(&player, &String::from("q1:ammo/cells")), 9.0);
    }
}
