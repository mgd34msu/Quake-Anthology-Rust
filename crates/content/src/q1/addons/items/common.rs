//! Q1 mg3 shared item helpers (`src/content/q1/addons/items/common.ts`).
//!
//! `quakec_mg3/items.qc` and `subs.qc`. GPL-2.0-or-later.

use qa_core::identity::ActorId;

use crate::q1::addons::context::{removed_for_runes, removed_outside_coop};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1UseHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Effect, Q1MessageArg, Q1Solid, Q1SoundChannel};
use crate::q1::Q1Error;

/// MG3 item callback prefix (`MG3_ITEM_PREFIX`).
pub const MG3_ITEM_PREFIX: &str = "mg3:items:";
/// Spawnflag marking a spawned (regenerating) item (`MG3_SPAWNED_ITEM`).
pub const MG3_SPAWNED_ITEM: i32 = 4;

/// Starts an mg3 item spawn (`startMg3Item`).
pub fn start_mg3_item(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? || removed_for_runes(game, id)? {
        return Ok(());
    }
    let model = game
        .entity_ref(id)
        .map(|entity| entity.model.clone())
        .unwrap_or_default();
    game.update_entity(id, |entity| entity.original_model = model)?;
    if game
        .entity_ref(id)
        .is_some_and(|entity| entity.spawnflags & MG3_SPAWNED_ITEM != 0)
    {
        let regenerate = game.named.use_callback(&format!("{MG3_ITEM_PREFIX}regenerate"))?;
        game.update_entity(id, |entity| entity.use_callback = Some(regenerate))?;
    }
    game.schedule(id, 0.2, &format!("{MG3_ITEM_PREFIX}place"))
}

/// Finishes an mg3 pickup (`finishMg3Pickup`).
pub fn finish_mg3_pickup(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    message: &str,
    sound: &str,
    args: Vec<Q1MessageArg>,
) -> Result<(), Q1Error> {
    let player = game.host.actors.resolve_owned(other);
    let Some(player) = player else {
        return Ok(());
    };
    if !message.is_empty() {
        game.message(Some(other), message, true, args);
    }
    if !game.is_live(id) || !game.host.actors.is_live(other) {
        return Ok(());
    }
    game.sound(player.id(), sound, Q1SoundChannel::Item, 1.0, 1.0)?;
    if !game.is_live(id) || !game.host.actors.is_live(other) {
        return Ok(());
    }
    let origin = game.body(id)?.origin;
    game.effect(Q1Effect::Pickup, origin, Some(other), 1);
    if !game.is_live(id) || !game.host.actors.is_live(other) {
        return Ok(());
    }
    game.update_entity(id, |entity| entity.activator = Some(other.clone()))?;
    game.use_targets(id, Some(other))?;
    if game.is_live(id) {
        game.remove(id)
    } else {
        Ok(())
    }
}

fn mg3_item_place(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.invoke_action(id, "PlaceItem")?;
    if game.is_live(id)
        && game
            .entity_ref(id)
            .is_some_and(|entity| entity.spawnflags & MG3_SPAWNED_ITEM != 0)
    {
        game.update_entity(id, |entity| {
            entity.solid = Q1Solid::None;
            entity.model = String::new();
        })?;
        game.link(id)?;
    }
    Ok(())
}

fn mg3_item_regenerate(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.invoke_action(id, "SUB_regen")
}

/// Registers shared mg3 item callbacks (`registerMg3ItemCallbacks`).
pub fn register_mg3_item_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}place"),
        Q1CallbackHandlers {
            action: Some(mg3_item_place as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}regenerate"),
        Q1CallbackHandlers {
            use_callback: Some(mg3_item_regenerate as Q1UseHandler),
            ..Default::default()
        },
    )
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
        register_mg3_item_callbacks(game).expect("callbacks");
        guard
    }

    #[test]
    fn start_arms_spawned_items_for_regen() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let item = game.create("item_upgrade_health", None, None).expect("item");
        game.update_entity(&item, |entity| {
            entity.model = String::from("progs/item.mdl");
            entity.spawnflags = MG3_SPAWNED_ITEM;
        })
        .expect("flags");
        start_mg3_item(&mut game, &item).expect("start");
        let entity = game.entity_ref(&item).expect("entity");
        assert_eq!(entity.original_model, "progs/item.mdl");
        assert_eq!(entity.use_callback.as_deref(), Some("mg3:items:regenerate"));
    }

    #[test]
    fn finish_pickup_removes_and_fires_targets() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        let item = game.create("item_upgrade_health", None, None).expect("item");
        finish_mg3_pickup(
            &mut game,
            &item,
            &player,
            "$mg3_qc_upgrade_success",
            "weapons/lock4.wav",
            vec![Q1MessageArg::Text(String::from("$mg3_qc_upgrade_health"))],
        )
        .expect("finish");
        assert!(game.entity_ref(&item).is_none());
    }
}
