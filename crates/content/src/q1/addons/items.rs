//! Q1 mg3 items root (`src/content/q1/addons/items/index.ts`).

pub mod commands;
pub mod common;
pub mod lavasuit;
pub mod pickups;
pub mod upgrades;
pub mod weapons;

pub use commands::handle_mg3_item_impulse;
pub use common::{finish_mg3_pickup, register_mg3_item_callbacks, start_mg3_item, MG3_ITEM_PREFIX, MG3_SPAWNED_ITEM};
pub use lavasuit::{mg3_lava_suit_frame, register_mg3_lava_suit};
pub use pickups::{mg3_weapon_rank, register_mg3_pickups, MG3_BLOODY_SHOTGUN, MG3_BLOODY_SUPER_SHOTGUN};
pub use upgrades::{
    capture_mg3_upgrade_travel, give_next_mg3_upgrade, initialize_mg3_capacities, mg3_inventory_capacity,
    mg3_upgrade_flag, mg3_upgraded_maximum, register_mg3_upgrades, restore_mg3_upgrade_travel, Mg3Upgrade,
};
pub use weapons::{mg3_hammer_body_frame, mg3_weapon_frame, register_mg3_weapons};

use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::Q1Error;

/// Registers mg3 items (`registerMg3Items`). Register after the shared
/// base arsenal and before player admission/map spawning.
pub fn register_mg3_items(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    use crate::q1::addons::context::{addon_program, Q1AddonProgram};
    use crate::q1::foundation::extensions::{Q1PickupRules, Q1PlayerExtension};

    if addon_program(game)? != Q1AddonProgram::Mg3 {
        return Err(crate::q1::q1_error("MG3 item registration requires its source program"));
    }
    register_mg3_item_callbacks(game)?;
    register_mg3_upgrades(game)?;
    register_mg3_weapons(game)?;
    register_mg3_pickups(game)?;
    register_mg3_lava_suit(game)?;
    game.register_pickup_rules(Q1PickupRules {
        id: String::from("q1:mg3"),
        weapon_rank: Some(mg3_weapon_rank),
        ..Default::default()
    })?;
    game.register_player_extension(Q1PlayerExtension {
        id: String::from("q1:mg3:items"),
        attach: Some(initialize_mg3_capacities),
        inventory_capacity: Some(mg3_inventory_capacity),
        frame: Some(mg3_items_frame),
        capture_travel: Some(capture_mg3_upgrade_travel),
        restore_travel: Some(restore_mg3_upgrade_travel),
        ..Default::default()
    })
}

fn mg3_items_frame(
    game: &mut Q1EntityServices,
    player: &qa_core::identity::ActorId,
    _seconds: f64,
) -> Result<(), Q1Error> {
    mg3_weapon_frame(game, player)?;
    mg3_lava_suit_frame(game, player)
}
