//! Q1 mg3 lava suit (`src/content/q1/addons/items/lavasuit.ts`).
//!
//! `quakec_mg3/items.qc` `powerup_touch` and `client.qc`
//! `CheckPowerups`. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::addons::context::{addon_player_number, set_addon_player_number};
use crate::q1::addons::items::common::{start_mg3_item, MG3_ITEM_PREFIX};
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Effect, Q1MessageArg, Q1Powerup, Q1SoundChannel};
use crate::q1::Q1Error;

/// Lava suit touch (`lavasuit_touch`).
fn lavasuit_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if game.player_ref(other).is_none() || game.health(other) <= 0.0 {
        return Ok(());
    }
    game.give_powerup(other, Q1Powerup::Mg3Lavasuit, 30.0)?;
    set_addon_player_number(game, other, "lavasuit_time", 1.0)?;
    game.message(
        Some(other),
        "$qc_got_item",
        true,
        vec![Q1MessageArg::Text(String::from("$mg3_qc_lavasuit"))],
    );
    let owned = game
        .player_owned(other)
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 lava suit player"))?;
    game.sound(owned.id(), "items/suit.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    let origin = game.body(id)?.origin;
    game.effect(Q1Effect::Pickup, origin, Some(other), 1);
    game.update_entity(id, |entity| {
        entity.model = String::new();
        entity.solid = crate::q1::foundation::types::Q1Solid::None;
    })?;
    game.link(id)?;
    game.update_entity(id, |entity| entity.activator = Some(other.clone()))?;
    game.use_targets(id, Some(other))?;
    if !game.is_live(id) {
        return Ok(());
    }
    if game.options().coop {
        game.update_entity(id, |entity| entity.target = String::new())?;
    }
    let respawn = if game.options().coop {
        2.5
    } else if game.options().deathmatch != 0 {
        60.0
    } else {
        game.entity_ref(id).map(|entity| entity.wait).unwrap_or(0.0)
    };
    if respawn > 0.0 {
        game.schedule(id, respawn, "SUB_regen")
    } else {
        game.cancel(id);
        Ok(())
    }
}

fn spawn_lavasuit(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.precache_model("progs/lavasuit.mdl")?;
    game.precache_sound("items/suit.wav")?;
    game.precache_sound("items/suit2.wav")?;
    game.update_entity(id, |entity| {
        entity.model = String::from("progs/lavasuit.mdl");
        entity
            .fields
            .insert(String::from("netname"), String::from("$mg3_qc_lavasuit"));
    })?;
    let touch = game.named.touch(&format!("{MG3_ITEM_PREFIX}lavasuit_touch"))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    game.set_bounds(
        id,
        qa_core::math::Bounds {
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
    start_mg3_item(game, id)
}

/// Registers the mg3 lava suit (`registerMg3LavaSuit`).
pub fn register_mg3_lava_suit(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}lavasuit_touch"),
        Q1CallbackHandlers {
            touch: Some(lavasuit_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("item_artifact_lavasuit", spawn_lavasuit)
}

/// Ticks lava suit warnings (`mg3LavaSuitFrame`).
pub fn mg3_lava_suit_frame(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    let expires = game
        .player_ref(player)
        .and_then(|state| state.powerups.get(&Q1Powerup::Mg3Lavasuit).copied())
        .unwrap_or(0.0);
    if expires == 0.0 {
        return Ok(());
    }
    let warning = addon_player_number(game, player, "lavasuit_time")?;
    let body = game.host.bodies.read(player);
    if expires < game.time + 3.0 && warning == 1.0 {
        game.message(Some(player), "$mg3_qc_lavasuit_wearing_out", true, Vec::new());
        let owned = game
            .player_owned(player)
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 lava suit player"))?;
        game.sound(owned.id(), "items/suit2.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
        set_addon_player_number(game, player, "lavasuit_time", game.time + 1.0)?;
        if let Some(body) = body {
            game.effect(Q1Effect::Pickup, body.origin, Some(player), 1);
        }
    } else if expires < game.time + 3.0 && warning < game.time {
        set_addon_player_number(game, player, "lavasuit_time", game.time + 1.0)?;
        if let Some(body) = body {
            game.effect(Q1Effect::Pickup, body.origin, Some(player), 1);
        }
    }
    if expires <= game.time {
        set_addon_player_number(game, player, "lavasuit_time", 0.0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg3);
        crate::q1::addons::items::common::register_mg3_item_callbacks(game).expect("callbacks");
        register_mg3_lava_suit(game).expect("lavasuit");
        guard
    }

    #[test]
    fn spawn_sets_lavasuit_model() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let suit = game.create("item_artifact_lavasuit", None, None).expect("suit");
        game.spawn_entity(&suit, None).expect("spawn");
        let entity = game.entity_ref(&suit).expect("entity");
        assert_eq!(entity.model, "progs/lavasuit.mdl");
        assert_eq!(entity.text("netname"), "$mg3_qc_lavasuit");
    }

    #[test]
    fn touch_grants_timed_powerup() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        let suit = game.create("item_artifact_lavasuit", None, None).expect("suit");
        game.spawn_entity(&suit, None).expect("spawn");
        lavasuit_touch(&mut game, &suit, &player, None, None).expect("touch");
        let state = game.player_ref(&player).expect("state").clone();
        assert!(state.powerups.get(&Q1Powerup::Mg3Lavasuit).copied().unwrap_or(0.0) > 0.0);
        assert_eq!(addon_player_number(&game, &player, "lavasuit_time"), Ok(1.0));
    }

    #[test]
    fn frame_warns_before_expiry() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        game.give_powerup(&player, Q1Powerup::Mg3Lavasuit, 2.0).expect("give");
        set_addon_player_number(&mut game, &player, "lavasuit_time", 1.0).expect("warn");
        game.begin_frame(0.5, 0.1);
        mg3_lava_suit_frame(&mut game, &player).expect("frame");
        assert_eq!(addon_player_number(&game, &player, "lavasuit_time"), Ok(1.5));
    }
}
