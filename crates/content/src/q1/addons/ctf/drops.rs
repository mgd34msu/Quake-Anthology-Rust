//! Q1 CTF explicit impulse 20/21 drops (src/content/q1/addons/ctf/drops.ts).

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::ctf::state::{ctf_body, ctf_grant, ctf_owner, with_ctf_services};
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::gameplay::TouchSurface;
use crate::q1::foundation::types::{
    vadd, vectors, vscale, Q1Effect, Q1MessageArg, Q1MoveType, Q1Solid, Q1SoundChannel,
};
use crate::q1::foundation::weapons::aim;
use crate::q1::Q1Error;

/// Ammunition family eligible for backpack drops.
struct AmmoType {
    /// Ammunition item.
    item: &'static str,
    /// Maximum dropped count.
    limit: f64,
    /// Weapons drawing this ammunition.
    weapons: &'static [&'static str],
}

const AMMO_TYPES: [AmmoType; 4] = [
    AmmoType {
        item: "q1:ammo/shells",
        limit: 20.0,
        weapons: &["q1:weapon/shotgun", "q1:weapon/supershotgun"],
    },
    AmmoType {
        item: "q1:ammo/nails",
        limit: 20.0,
        weapons: &["q1:weapon/nailgun", "q1:weapon/supernailgun"],
    },
    AmmoType {
        item: "q1:ammo/rockets",
        limit: 10.0,
        weapons: &["q1:weapon/grenadelauncher", "q1:weapon/rocketlauncher"],
    },
    AmmoType {
        item: "q1:ammo/cells",
        limit: 20.0,
        weapons: &["q1:weapon/lightning"],
    },
];

/// Weapon eligible for impulse 21 drops.
struct WeaponDef {
    /// Weapon item.
    item: &'static str,
    /// Dropped classname.
    classname: &'static str,
    /// World model stem.
    model: &'static str,
    /// Pickup message name.
    name: &'static str,
}

const WEAPON_DEFS: [WeaponDef; 6] = [
    WeaponDef {
        item: "q1:weapon/supershotgun",
        classname: "weapon_supershotgun",
        model: "g_shot",
        name: "Double-barrelled Shotgun",
    },
    WeaponDef {
        item: "q1:weapon/nailgun",
        classname: "weapon_nailgun",
        model: "g_nail",
        name: "nailgun",
    },
    WeaponDef {
        item: "q1:weapon/supernailgun",
        classname: "weapon_supernailgun",
        model: "g_nail2",
        name: "Super Nailgun",
    },
    WeaponDef {
        item: "q1:weapon/grenadelauncher",
        classname: "weapon_grenadelauncher",
        model: "g_rock",
        name: "Grenade Launcher",
    },
    WeaponDef {
        item: "q1:weapon/rocketlauncher",
        classname: "weapon_rocketlauncher",
        model: "g_rock2",
        name: "Rocket Launcher",
    },
    WeaponDef {
        item: "q1:weapon/lightning",
        classname: "weapon_lightning",
        model: "g_light",
        name: "Thunderbolt",
    },
];

/// Tossed item bounds.
const DROP_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: 0.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 56.0,
    },
};

/// Launch a tossed item forward from the actor (`launch`).
fn launch(game: &mut Q1EntityServices, actor: &ActorId, item: &ActorId) -> Result<(), Q1Error> {
    let owner = ctf_owner(game, actor)?;
    let body = ctf_body(game, actor)?;
    let input = with_ctf_services(game, |services| services.input(actor))?;
    let forward = vectors(input.view_angles).forward;
    let velocity = vscale(aim(game, &owner, forward), 500.0);
    game.update_entity(item, |entity| {
        entity.owner = Some(actor.clone());
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Bounce;
        entity.movement_flags |= 256;
    })?;
    game.set_body(
        item,
        &BodyPatch {
            origin: Some(vadd(
                body.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 16.0,
                },
            )),
            velocity: Some(velocity),
            bounds: Some(DROP_BOUNDS),
            ..Default::default()
        },
    )?;
    game.link(item)?;
    game.schedule(item, 120.0, "SUB_Remove")
}

/// Toss an ammunition backpack for impulse 20 (`tossAmmo`).
pub fn toss_ammo(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let selected = with_ctf_services(game, |services| services.selected_weapon(&id))?;
    let ammo = with_ctf_services(game, |services| services.selected_ammo(&id))?;
    let Some(ammo) = ammo else {
        return Ok(());
    };
    if game.host.inventory.count(&id, &ammo) <= 0.0 {
        return Ok(());
    }
    let item = game.create("ctf_backpack", None, None)?;
    game.update_entity(&item, |entity| {
        entity.model = String::from("progs/backpack.mdl");
    })?;
    let owner = ctf_owner(game, &id)?;
    for kind in AMMO_TYPES.iter() {
        let selected_here = selected
            .as_deref()
            .is_some_and(|selected| kind.weapons.contains(&selected));
        if !selected_here
            && kind
                .weapons
                .iter()
                .any(|weapon| game.host.inventory.count(&id, &weapon.to_string()) != 0.0)
        {
            continue;
        }
        let count = kind.limit.min(game.host.inventory.count(&id, &kind.item.to_string()));
        game.host.inventory.consume(&owner, &kind.item.to_string(), count);
        crate::q1::addons::context::set_addon_number(game, &item, kind.item, count)?;
    }
    let touch = game.named.touch("ctf:backpack_touch")?;
    game.update_entity(&item, |entity| {
        entity.touch = Some(touch);
    })?;
    launch(game, &id, &item)?;
    with_ctf_services(game, |services| services.weapon_changed(&id, None))?;
    Ok(())
}

/// Toss the selected weapon for impulse 21 (`tossWeapon`).
pub fn toss_weapon(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    if game.options().deathmatch != 1 {
        return Ok(());
    }
    let id = actor.clone();
    let selected = with_ctf_services(game, |services| services.selected_weapon(&id))?;
    let Some(definition) = WEAPON_DEFS
        .iter()
        .find(|weapon| Some(weapon.item) == selected.as_deref())
    else {
        return Ok(());
    };
    let item = game.create(definition.classname, None, None)?;
    let touch = game.named.touch("ctf:weapon_touch")?;
    game.update_entity(&item, |entity| {
        entity.model = format!("progs/{}.mdl", definition.model);
        entity
            .fields
            .insert(String::from("ctf.weapon"), definition.item.to_string());
        entity.message = definition.name.to_string();
        entity.touch = Some(touch);
    })?;
    let owner = ctf_owner(game, &id)?;
    game.host.inventory.consume(&owner, &definition.item.to_string(), 1.0);
    launch(game, &id, &item)?;
    with_ctf_services(game, |services| services.weapon_changed(&id, None))?;
    Ok(())
}

fn backpack_touch(
    game: &mut Q1EntityServices,
    item: &ActorId,
    actor: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let item = item.clone();
    let id = actor.clone();
    if !game.is_player(&id) || game.health(&id) <= 0.0 {
        return Ok(());
    }
    if with_ctf_services(game, |services| services.observer(&id))? {
        return Ok(());
    }
    let owner = ctf_owner(game, &id)?;
    for kind in AMMO_TYPES.iter() {
        let count = game
            .entity_ref(&item)
            .map(|entity| entity.number(kind.item))
            .unwrap_or(0.0);
        game.host.inventory.give(&owner, &kind.item.to_string(), count);
    }
    game.sound(&id, "weapons/lock4.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    game.effect(Q1Effect::Pickup, ctf_body(game, &id)?.origin, Some(&id), 1);
    with_ctf_services(game, |services| services.weapon_changed(&id, None))?;
    game.remove(&item)
}

fn weapon_touch(
    game: &mut Q1EntityServices,
    item: &ActorId,
    actor: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let item = item.clone();
    let id = actor.clone();
    if !game.is_player(&id) {
        return Ok(());
    }
    if with_ctf_services(game, |services| services.observer(&id))? {
        return Ok(());
    }
    let entity = game
        .entity_ref(&item)
        .cloned()
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 entity"))?;
    if entity.owner.as_ref().is_some_and(|owner| same_actor(owner, &id)) && entity.next_think - game.time > 119.0 {
        return Ok(());
    }
    let kind = entity.text("ctf.weapon");
    let Some(definition) = WEAPON_DEFS.iter().find(|weapon| weapon.item == kind) else {
        return Err(crate::q1::q1_error("CTF dropped weapon lost source kind"));
    };
    ctf_grant(game, &id, definition.item, 1.0, 1.0)?;
    let message = entity.message.clone();
    game.message(Some(&id), "$qc_got_item", false, vec![Q1MessageArg::Text(message)]);
    game.sound(&id, "weapons/pkup.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    game.effect(Q1Effect::Pickup, ctf_body(game, &id)?.origin, Some(&id), 1);
    let acquired = definition.item.to_string();
    with_ctf_services(game, |services| services.weapon_changed(&id, Some(&acquired)))?;
    game.use_targets(&item, Some(&id))?;
    game.remove(&item)
}

/// Register tossed-item touch callbacks (`registerDrops`).
pub fn register_drops(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "ctf:backpack_touch",
        Q1CallbackHandlers {
            touch: Some(backpack_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:weapon_touch",
        Q1CallbackHandlers {
            touch: Some(weapon_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
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

    fn setup(
        game: &mut Q1EntityServices,
    ) -> (
        Q1BaseGuard,
        ActorId,
        std::sync::Arc<std::sync::Mutex<crate::q1::addons::ctf::types::FakeCtfState>>,
    ) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, fake) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), true, false);
        let player = attach_test_player(game);
        (guard, player, fake)
    }

    #[test]
    fn toss_ammo_consumes_and_spawns_backpack() {
        let mut game = test_game();
        let (_guard, player, fake) = setup(&mut game);
        register_drops(&mut game).expect("drops");
        crate::q1::addons::ctf::state::ctf_grant(&mut game, &player, "q1:ammo/shells", 50.0, 100.0).expect("shells");
        crate::q1::addons::ctf::state::ctf_grant(&mut game, &player, "q1:weapon/shotgun", 1.0, 1.0).expect("shotgun");
        {
            let mut fake = fake.lock().expect("fake");
            fake.selected_weapons
                .insert(player.clone(), Some(String::from("q1:weapon/shotgun")));
            fake.selected_ammo
                .insert(player.clone(), Some(String::from("q1:ammo/shells")));
        }
        toss_ammo(&mut game, &player).expect("toss");
        let shells = String::from("q1:ammo/shells");
        assert_eq!(game.host.inventory.count(&player, &shells), 30.0);
        let packs: Vec<ActorId> = game
            .entity_ids()
            .into_iter()
            .filter(|id| {
                game.entity_ref(id)
                    .is_some_and(|entity| entity.classname == "ctf_backpack")
            })
            .collect();
        assert_eq!(packs.len(), 1);
        backpack_touch(&mut game, &packs[0], &player, None, None).expect("touch");
        assert_eq!(game.host.inventory.count(&player, &shells), 50.0);
    }

    #[test]
    fn toss_weapon_requires_deathmatch_one() {
        let mut game = test_game();
        let (_guard, player, _) = setup(&mut game);
        register_drops(&mut game).expect("drops");
        toss_weapon(&mut game, &player).expect("toss");
        assert!(game.entity_ids().into_iter().all(|id| {
            game.entity_ref(&id)
                .map(|entity| !entity.classname.starts_with("weapon_"))
                .unwrap_or(true)
        }));
    }
}
