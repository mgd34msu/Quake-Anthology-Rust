//! Q1 CTF spawn arsenal and rune weapon rules (src/content/q1/addons/ctf/arsenal.ts).

use qa_core::identity::ActorId;

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q1::addons::ctf::runes::{ctf_haste_interval, CTF_HASTE_NAIL_SPEED};
use crate::q1::addons::ctf::state::rune_item;
use crate::q1::addons::ctf::state::{
    ctf_grant, ctf_number, ctf_rune, ctf_set, ctf_start_map, ctf_teamplay_bits, native_grapple_enabled,
    with_ctf_services,
};
use crate::q1::addons::ctf::types::{CtfFlags, CtfRune, CTF_RUNES};
use crate::q1::equipment::grapple::grapple_state;
use crate::q1::equipment::weapon::{
    register_threewave_weapon, threewave_character_pose, weapon_animate, weapon_attack, ThreewaveWeaponHooks,
};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::extensions::{Q1PickupRules, Q1WeaponDefinition, Q1WeaponRules};
use crate::q1::foundation::types::{Q1AutoSwitch, Q1BaseWeapon, Q1Event, Q1Powerup, Q1SoundChannel, Q1Weapon, WEAPONS};
use crate::q1::Q1Error;

/// Character pose contribution (`CtfCharacterPose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtfCharacterPose {
    /// Pose uses the axe animation.
    pub axe_pose: bool,
    /// Override frame, if any.
    pub frame: Option<i32>,
}

/// Q1 model frames only; foreign character controllers use their own
/// grapple animation mapping (`characterPose`).
pub fn character_pose(game: &mut Q1EntityServices, actor: &ActorId) -> Result<CtfCharacterPose, Q1Error> {
    let selected = with_ctf_services(game, |services| services.input(actor).grapple_selected)?;
    if !native_grapple_enabled(game)? || !selected {
        let axe =
            with_ctf_services(game, |services| services.selected_weapon(actor))?.as_deref() == Some("q1:weapon/axe");
        return Ok(CtfCharacterPose {
            axe_pose: axe,
            frame: None,
        });
    }
    let pose = threewave_character_pose(game, actor)?;
    Ok(CtfCharacterPose {
        axe_pose: pose.axe_pose,
        frame: pose.frame,
    })
}

/// Reset powerups, keys, runes, weapons, ammunition, armor and the
/// selected weapon (`spawnArsenal`).
pub fn spawn_arsenal(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let owned = match game.player_owned(actor) {
        Some(owned) => owned,
        None => return Ok(()),
    };
    let id = owned.id().clone();
    let powerups: Vec<Q1Powerup> = game
        .players
        .get(&id)
        .map(|player| player.powerups.keys().copied().collect())
        .unwrap_or_default();
    for kind in &powerups {
        game.host.powerup(&owned, *kind, 0.0);
    }
    if let Some(player) = game.players.get_mut(&id) {
        player.powerups.clear();
    }
    ctf_grant(game, &id, "q1:key/silver", 0.0, 1.0)?;
    ctf_grant(game, &id, "q1:key/gold", 0.0, 1.0)?;
    for rune in CTF_RUNES {
        ctf_grant(game, &id, &rune_item(rune), 0.0, 1.0)?;
    }
    let start = ctf_start_map(game);
    for weapon in WEAPONS {
        let item = game.weapon_item(Q1Weapon::from(weapon));
        let count = if weapon == Q1BaseWeapon::Axe || weapon == Q1BaseWeapon::Shotgun && !start {
            1.0
        } else {
            0.0
        };
        ctf_grant(game, &id, &item, count, 1.0)?;
    }
    ctf_grant(game, &id, "q1:ammo/shells", if start { 0.0 } else { 40.0 }, 100.0)?;
    ctf_grant(game, &id, "q1:ammo/nails", 0.0, 200.0)?;
    ctf_grant(game, &id, "q1:ammo/rockets", 0.0, 100.0)?;
    ctf_grant(game, &id, "q1:ammo/cells", 0.0, 100.0)?;
    let grapple = native_grapple_enabled(game)? && !start && ctf_teamplay_bits(game)? & CtfFlags::DISABLE_GRAPPLE == 0;
    ctf_grant(game, &id, "q1:ctf/weapon/grapple", f64::from(u8::from(grapple)), 1.0)?;
    game.host.combat.set_armor(
        &owned,
        &ArmorState {
            regular: if start {
                RegularArmorState::None
            } else {
                RegularArmorState::Q1 {
                    points: 50.0,
                    absorption: 0.3,
                    item: String::from("q1:item_armor1"),
                }
            },
            powered: PoweredProtectionState::None,
        },
    )?;
    game.select_weapon(&owned, if start { Q1Weapon::Axe } else { Q1Weapon::Shotgun })?;
    Ok(())
}

/// Present a grapple view-model frame (`weaponFrame`).
fn weapon_frame(game: &mut Q1EntityServices, actor: &ActorId, frame: i32) -> Result<(), Q1Error> {
    grapple_state(game, actor)?.weapon_frame = frame;
    if let Some(player) = game.players.get_mut(actor) {
        player.weapon_frame = frame;
    }
    game.host.emit(Q1Event::Weapon {
        player: actor.clone(),
        weapon: Q1Weapon::CtfGrapple,
        view_model: String::from("progs/v_star.mdl"),
        frame,
        punch: 0,
        attack: None,
    });
    Ok(())
}

/// Throttled haste loop sound (`hasteSound`).
fn haste_sound(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    if ctf_number(game, actor, "hasteSound")? < game.time {
        ctf_set(game, actor, "hasteSound", game.time + 1.0)?;
        let owner = game
            .host
            .actors
            .resolve_owned(actor)
            .map(|owned| owned.id().clone())
            .unwrap_or_else(|| actor.clone());
        game.sound(&owner, "rune/rune3.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
    }
    Ok(())
}

/// Attack with the grapple, syncing the player attack lock
/// (`grappleAttack`).
pub fn grapple_attack(game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
    if !native_grapple_enabled(game)? || game.threewave_grapple.is_none() || game.threewave_weapon.is_none() {
        return Ok(false);
    }
    if grapple_state(game, actor)?.attack_finished > game.time {
        return Ok(false);
    }
    let fired = weapon_attack(game, actor)?;
    if fired {
        let attack_finished = grapple_state(game, actor)?.attack_finished;
        if let Some(player) = game.players.get_mut(actor) {
            player.attack_finished = attack_finished;
        }
    }
    Ok(fired)
}

/// CTF weapon hooks: launches require teamplay grapple access and a
/// live player; frames present the star view model.
pub(crate) struct CtfWeaponHooks;

impl ThreewaveWeaponHooks for CtfWeaponHooks {
    fn launch(&mut self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<Option<bool>, Q1Error> {
        let available = ctf_teamplay_bits(game)? & CtfFlags::DISABLE_GRAPPLE == 0
            && !with_ctf_services(game, |services| services.observer(actor))?;
        Ok(if available { None } else { Some(false) })
    }

    fn animated(&mut self, game: &mut Q1EntityServices, actor: &ActorId, frame: i32) -> Result<(), Q1Error> {
        weapon_frame(game, actor, frame)
    }
}

fn ctf_grapple_available(game: &mut Q1EntityServices, _player: &ActorId) -> Result<bool, Q1Error> {
    Ok(ctf_teamplay_bits(game)? & CtfFlags::DISABLE_GRAPPLE == 0)
}

fn ctf_grapple_best(_game: &mut Q1EntityServices, _player: &ActorId) -> Result<bool, Q1Error> {
    Ok(false)
}

fn ctf_grapple_fire(game: &mut Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    grapple_attack(game, player)
}

fn ctf_grapple_animate(game: &mut Q1EntityServices, player: &ActorId, _seconds: f64) -> Result<(), Q1Error> {
    weapon_animate(game, player)
}

fn ctf_before_fire(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    if ctf_rune(game, player) != Some(CtfRune::Strength) || ctf_number(game, player, "strengthSound")? >= game.time {
        return Ok(());
    }
    ctf_set(game, player, "strengthSound", game.time + 1.0)?;
    let quad = game
        .player_ref(player)
        .and_then(|player| player.powerups.get(&Q1Powerup::Quad).copied())
        .unwrap_or(0.0);
    let owner = player.clone();
    game.sound(
        &owner,
        if quad > game.time {
            "rune/rune22.wav"
        } else {
            "rune/rune2.wav"
        },
        Q1SoundChannel::Body,
        1.0,
        1.0,
    )
}

fn ctf_attack_delay(game: &mut Q1EntityServices, player: &ActorId, delay: f64) -> Result<f64, Q1Error> {
    if ctf_rune(game, player) != Some(CtfRune::Haste) {
        return Ok(delay);
    }
    let weapon = match game.player_ref(player) {
        Some(player) => player.weapon,
        None => return Ok(delay),
    };
    let item = game.weapon_item(weapon);
    match ctf_haste_interval(&item) {
        Some(source) => {
            haste_sound(game, player)?;
            Ok(source)
        }
        None => Ok(delay),
    }
}

fn ctf_nail_speed(game: &mut Q1EntityServices, player: &ActorId, speed: f64) -> Result<f64, Q1Error> {
    if ctf_rune(game, player) != Some(CtfRune::Haste) {
        return Ok(speed);
    }
    haste_sound(game, player)?;
    Ok(CTF_HASTE_NAIL_SPEED)
}

fn ctf_weapon_ammo_grant(
    game: &mut Q1EntityServices,
    _player: &ActorId,
    _weapon: Q1Weapon,
    amount: f64,
) -> Result<f64, Q1Error> {
    if game.options().coop && ctf_teamplay_bits(game)? & CtfFlags::DROP_ITEMS != 0 {
        Ok(0.0)
    } else {
        Ok(amount)
    }
}

fn ctf_auto_switch(game: &mut Q1EntityServices, player: &ActorId, owned: bool) -> Result<bool, Q1Error> {
    let Some(record) = game.player_ref(player).cloned() else {
        return Ok(false);
    };
    Ok(!with_ctf_services(game, |services| services.is_bot(player))?
        && !(record.weapon == Q1Weapon::CtfGrapple && record.attack_held)
        && record.auto_switch != Q1AutoSwitch::Never
        && (record.auto_switch != Q1AutoSwitch::New || !owned))
}

fn ctf_weapon_rank(weapon: Q1Weapon) -> i32 {
    match weapon {
        Q1Weapon::Lightning => 1,
        Q1Weapon::Rocketlauncher => 2,
        Q1Weapon::Supernailgun => 3,
        Q1Weapon::Grenadelauncher => 4,
        Q1Weapon::Supershotgun => 5,
        Q1Weapon::Nailgun => 6,
        _ => 7,
    }
}

/// Register the grapple weapon, rune weapon rules and pickup rules
/// (`registerArsenal`).
pub fn register_arsenal(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    if native_grapple_enabled(game)? {
        register_threewave_weapon(game, Box::new(CtfWeaponHooks))?;
        game.register_weapon(Q1WeaponDefinition {
            id: Q1Weapon::CtfGrapple,
            item: Some(String::from("q1:ctf/weapon/grapple")),
            ammo: None,
            ammo_per_shot: None,
            model: String::from("progs/v_star.mdl"),
            rank: 0,
            model_for: None,
            available: Some(ctf_grapple_available),
            best_available: Some(ctf_grapple_best),
            fire: ctf_grapple_fire,
            animate: Some(ctf_grapple_animate),
        })?;
    }
    game.register_weapon_rules(Q1WeaponRules {
        id: String::from("ctf:rune_weapons"),
        before_fire: Some(ctf_before_fire),
        attack_delay: Some(ctf_attack_delay),
        nail_speed: Some(ctf_nail_speed),
        ..Default::default()
    })?;
    game.register_pickup_rules(Q1PickupRules {
        id: String::from("ctf:weapon_pickups"),
        weapon_ammo_grant: Some(ctf_weapon_ammo_grant),
        auto_switch: Some(ctf_auto_switch),
        weapon_rank: Some(ctf_weapon_rank),
        ..Default::default()
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::state::register_ctf_state;
    use crate::q1::addons::ctf::types::FakeCtfServices;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), true, false);
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn spawn_arsenal_grants_starter_kit() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        spawn_arsenal(&mut game, &player).expect("arsenal");
        let shells = String::from("q1:ammo/shells");
        assert_eq!(game.host.inventory.count(&player, &shells), 40.0);
        let grapple = String::from("q1:ctf/weapon/grapple");
        assert_eq!(game.host.inventory.count(&player, &grapple), 1.0);
        let armor = game.host.combat.read(&player).expect("combat").armor;
        assert!(matches!(
            armor.regular,
            RegularArmorState::Q1 { points, .. } if points == 50.0
        ));
    }

    #[test]
    fn weapon_rank_matches_donor_order() {
        assert_eq!(ctf_weapon_rank(Q1Weapon::Lightning), 1);
        assert_eq!(ctf_weapon_rank(Q1Weapon::Nailgun), 6);
        assert_eq!(ctf_weapon_rank(Q1Weapon::Axe), 7);
        assert_eq!(ctf_weapon_rank(Q1Weapon::CtfGrapple), 7);
    }
}
