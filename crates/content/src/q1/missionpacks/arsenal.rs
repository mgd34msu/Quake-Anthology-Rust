//! Mission-pack arsenal (src/content/q1/missionpacks/arsenal.ts).

use qa_core::identity::ActorId;

use crate::contract::ItemId;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::extensions::Q1WeaponDefinition;
use crate::q1::foundation::types::{Q1Event, Q1Weapon, weapon_item};
use crate::q1::missionpacks::hipnotic_weapons::{
    fire_hipnotic_laser, fire_hipnotic_mjolnir, fire_hipnotic_proximity,
    register_hipnotic_weapon_callbacks,
};
use crate::q1::{Q1Error, q1_error};

use super::backpacks::{register_rogue_toss_callbacks, toss_rogue_backpack, toss_rogue_weapon};
use super::grapple::RogueGrapple;
use super::items::{MissionItemServices, register_mission_pack_items};
use super::pickup_rules::register_mission_pack_pickup_rules;
use super::player::MissionPackPlayers;
use super::rogue_weapons::{
    fire_rogue_lava, fire_rogue_multi_grenade, fire_rogue_multi_rocket, fire_rogue_plasma,
    register_rogue_weapon_callbacks,
};
use super::selection::mission_weapon_impulse;
use super::types::{
    MISSION_WEAPONS, MissionWeapon, Q1MissionPack, mission_reference, set_mission_reference,
};

/// World reference holding the horn charmer during `useTargets`.
const HORN_CHARMER_KEY: &str = "missionpack:horn-charmer";

/// Fire hook for a mission-pack weapon definition (`fire`).
fn mission_fire_for(
    weapon: MissionWeapon,
) -> fn(&mut Q1EntityServices, &ActorId) -> Result<bool, Q1Error> {
    match weapon {
        MissionWeapon::HipnoticLaser => fire_hipnotic_laser,
        MissionWeapon::HipnoticMjolnir => fire_hipnotic_mjolnir,
        MissionWeapon::HipnoticProximity => fire_hipnotic_proximity,
        MissionWeapon::RogueLavaNailgun | MissionWeapon::RogueLavaSupernailgun => fire_rogue_lava,
        MissionWeapon::RogueMultiGrenade => fire_rogue_multi_grenade,
        MissionWeapon::RogueMultiRocket => fire_rogue_multi_rocket,
        MissionWeapon::RoguePlasma => fire_rogue_plasma,
    }
}

/// Best-weapon availability for the lava super nailgun (`bestAvailable`).
/// The donor hook only checks ammunition because the donor engine checks
/// ownership first; this engine returns hook results directly, so ownership
/// is included here for the same net behavior.
fn lava_supernailgun_best_available(
    game: &mut Q1EntityServices,
    player: &ActorId,
) -> Result<bool, Q1Error> {
    let owned = game
        .host
        .inventory
        .count(player, &weapon_item(Q1Weapon::RogueLavaSupernailgun))
        > 0.0;
    let ammo = game
        .host
        .inventory
        .count(player, &ItemId::from("rogue:ammo/lava-nails"));
    Ok(owned && ammo >= 2.0)
}

/// Animate a mission-pack weapon (`animate`).
fn mission_weapon_animate(
    game: &mut Q1EntityServices,
    player: &ActorId,
    seconds: f64,
) -> Result<(), Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if state.continuous_firing || state.weapon_animation_at < 0.0 {
        return Ok(());
    }
    let step = ((seconds - state.weapon_animation_at) / 0.1).floor() as i32;
    let frame = if step >= 6 {
        0
    } else if state.weapon == Q1Weapon::HipnoticMjolnir {
        (step + 1).min(4)
    } else {
        step + 1
    };
    if frame != state.weapon_frame {
        let weapon = state.weapon;
        let actor = state.actor.id().clone();
        let view_model = game.weapon_model(weapon, Some(player))?;
        game.update_player(player, |state| state.weapon_frame = frame)?;
        game.host.emit(Q1Event::Weapon {
            player: actor,
            weapon,
            view_model,
            frame,
            punch: 0,
            attack: None,
        });
    }
    if step >= 6 {
        game.update_player(player, |state| state.weapon_animation_at = -1.0)?;
    }
    Ok(())
}

/// Register the weapon definitions and order for a pack
/// (`registerWeaponDefinitions`).
fn register_weapon_definitions(
    game: &mut Q1EntityServices,
    pack: Q1MissionPack,
) -> Result<(), Q1Error> {
    let prefix = pack.as_str();
    for definition in MISSION_WEAPONS
        .iter()
        .filter(|definition| definition.id.as_str().starts_with(prefix))
    {
        let weapon = Q1Weapon::from(definition.id);
        game.register_weapon(Q1WeaponDefinition {
            id: weapon,
            item: None,
            ammo: definition.ammo.map(ItemId::from),
            ammo_per_shot: None,
            model: definition.model.to_string(),
            rank: definition.rank,
            model_for: None,
            available: None,
            best_available: if weapon == Q1Weapon::RogueLavaSupernailgun {
                Some(lava_supernailgun_best_available)
            } else {
                None
            },
            fire: mission_fire_for(definition.id),
            animate: Some(mission_weapon_animate),
        })?;
    }
    let order: Vec<Q1Weapon> = if pack == Q1MissionPack::Hipnotic {
        vec![
            Q1Weapon::Lightning,
            Q1Weapon::HipnoticLaser,
            Q1Weapon::Supernailgun,
            Q1Weapon::Supershotgun,
            Q1Weapon::Nailgun,
            Q1Weapon::Shotgun,
            Q1Weapon::HipnoticMjolnir,
            Q1Weapon::Axe,
        ]
    } else {
        vec![
            Q1Weapon::Lightning,
            Q1Weapon::RogueLavaSupernailgun,
            Q1Weapon::Supernailgun,
            Q1Weapon::RogueLavaNailgun,
            Q1Weapon::Nailgun,
            Q1Weapon::Supershotgun,
            Q1Weapon::Shotgun,
            Q1Weapon::Axe,
        ]
    };
    game.register_weapon_order(&format!("q1:{prefix}"), order)
}

/// Register Hipnotic weapons (`registerHipnoticWeapons`).
pub fn register_hipnotic_weapons(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    register_hipnotic_weapon_callbacks(game)?;
    register_weapon_definitions(game, Q1MissionPack::Hipnotic)
}

/// Mission-pack arsenal (`MissionPackArsenal`).
pub struct MissionPackArsenal {
    /// Mission pack.
    pub pack: Q1MissionPack,
    /// Mission-pack player services.
    pub players: MissionPackPlayers,
}

impl MissionPackArsenal {
    /// Register a mission-pack arsenal on a game.
    pub fn new(game: &mut Q1EntityServices, pack: Q1MissionPack) -> Result<Self, Q1Error> {
        let players = MissionPackPlayers::new(game, pack)?;
        register_mission_pack_pickup_rules(game, pack)?;
        if pack == Q1MissionPack::Hipnotic {
            register_hipnotic_weapons(game)?;
        } else {
            register_rogue_weapon_callbacks(game)?;
            register_rogue_toss_callbacks(game)?;
            let _grapple = RogueGrapple::new(game)?;
            register_weapon_definitions(game, pack)?;
        }
        register_mission_pack_items(game, &MissionItemServices::new(pack))?;
        Ok(Self { pack, players })
    }

    /// Actor that sounded the horn of conjuring, while its targets run
    /// (`hornCharmer`). Stored on the world entity so named callbacks can
    /// reach it without captured state.
    #[must_use]
    pub fn horn_charmer(game: &Q1EntityServices) -> Option<ActorId> {
        let world = game.world.as_ref()?;
        mission_reference(game, world, HORN_CHARMER_KEY)
    }

    /// Fire the horn of conjuring for a player (`horn`).
    pub fn use_horn(
        game: &mut Q1EntityServices,
        item: &ActorId,
        player: &ActorId,
    ) -> Result<(), Q1Error> {
        let Some(world) = game.world.clone() else {
            return game.use_targets(item, Some(player));
        };
        let previous = mission_reference(game, &world, HORN_CHARMER_KEY);
        set_mission_reference(game, &world, HORN_CHARMER_KEY, Some(player))?;
        let result = game.use_targets(item, Some(player));
        set_mission_reference(game, &world, HORN_CHARMER_KEY, previous.as_ref())?;
        result
    }

    /// Handle a weapon impulse (`impulse`).
    pub fn impulse(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        impulse: i32,
    ) -> Result<bool, Q1Error> {
        if game.player_ref(actor).is_none() {
            return Ok(false);
        }
        if self.pack == Q1MissionPack::Rogue && (impulse == 20 || impulse == 21) {
            if impulse == 20 {
                toss_rogue_backpack(game, actor)?;
            } else {
                toss_rogue_weapon(game, actor)?;
            }
            return Ok(true);
        }
        self.players.enable_combos(game, actor)?;
        mission_weapon_impulse(game, actor, self.pack, impulse)
    }
}

/// Register a mission-pack arsenal on a game (`registerMissionPackArsenal`).
pub fn register_mission_pack_arsenal(
    game: &mut Q1EntityServices,
    pack: Q1MissionPack,
) -> Result<MissionPackArsenal, Q1Error> {
    MissionPackArsenal::new(game, pack)
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;

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

    #[test]
    fn hipnotic_arsenal_registers_weapons_and_order() {
        let mut game = test_game();
        let arsenal =
            register_mission_pack_arsenal(&mut game, Q1MissionPack::Hipnotic).expect("arsenal");
        assert_eq!(arsenal.pack, Q1MissionPack::Hipnotic);
        assert!(
            game.registered_weapons
                .contains_key(&Q1Weapon::HipnoticLaser)
        );
        assert!(
            game.registered_weapons
                .contains_key(&Q1Weapon::HipnoticMjolnir)
        );
        assert!(
            game.registered_weapons
                .contains_key(&Q1Weapon::HipnoticProximity)
        );
        assert_eq!(
            game.weapon_order.as_ref().expect("order")[1],
            Q1Weapon::HipnoticLaser
        );
        assert!(MissionPackArsenal::horn_charmer(&game).is_none());
    }

    #[test]
    fn rogue_arsenal_registers_grapple_and_toss_impulses() {
        let mut game = test_game();
        let arsenal =
            register_mission_pack_arsenal(&mut game, Q1MissionPack::Rogue).expect("arsenal");
        assert!(game.registered_weapons.contains_key(&Q1Weapon::RoguePlasma));
        assert!(
            game.registered_weapons
                .contains_key(&Q1Weapon::RogueGrapple)
        );
        assert_eq!(
            game.weapon_order.as_ref().expect("order")[1],
            Q1Weapon::RogueLavaSupernailgun
        );
        let player = attached_player(&mut game);
        assert!(arsenal.impulse(&mut game, &player, 20).expect("toss"));
        assert!(arsenal.impulse(&mut game, &player, 21).expect("toss"));
    }

    #[test]
    fn impulse_rejects_unknown_players() {
        let mut game = test_game();
        let arsenal =
            register_mission_pack_arsenal(&mut game, Q1MissionPack::Hipnotic).expect("arsenal");
        let stranger = game.create("player", None, None).expect("stranger");
        assert!(!arsenal.impulse(&mut game, &stranger, 8).expect("impulse"));
    }
}
