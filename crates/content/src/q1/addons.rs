//! Q1 campaign addon root (`src/content/q1/addons`).
//!
//! Rerelease addon words for the `dopa`, `mg1`, `mg3`, and `ctf`
//! programs. The context owns per-game addon words, player references,
//! and frame ticks; every other module here registers its spawns and
//! named callbacks against it.

pub mod base_triggers;
pub mod brushes;
pub mod campaign;
pub mod commands;
pub mod context;
pub mod corpses;
pub mod ctf;
pub mod effects;
pub mod field_triggers;
pub mod horde;
pub mod items;
pub mod lights;
pub mod monsters;
pub mod rope;
pub mod travel;
pub mod triggers;

use qa_core::identity::ActorId;

use crate::q1::addons::base_triggers::register_addon_base_triggers;
use crate::q1::addons::brushes::register_addon_brushes;
use crate::q1::addons::campaign::register_campaign_addons;
use crate::q1::addons::context::{
    addon_cvar, register_addon_context, set_addon_number, Q1AddonContext, Q1AddonProgram, Q1AddonServices,
};
use crate::q1::addons::corpses::register_addon_corpses;
use crate::q1::addons::effects::{register_addon_effects, register_addon_fog};
use crate::q1::addons::field_triggers::register_addon_field_triggers;
use crate::q1::addons::items::register_mg3_items;
use crate::q1::addons::lights::register_addon_lights;
use crate::q1::addons::monsters::register_addon_monsters;
use crate::q1::addons::rope::register_addon_ropes;
use crate::q1::addons::triggers::register_addon_triggers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::spawns::spawn_map_actor;
use crate::q1::{q1_error, Q1Error};

/// MG3 worldspawn hook: spawn the map actor, then record the horde
/// and cheat words from engine variables.
fn mg3_worldspawn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_map_actor(game, id)?;
    set_addon_number(game, id, "isHordeMode", addon_cvar(game, "horde")?)?;
    set_addon_number(game, id, "cheats_allowed", addon_cvar(game, "sv_cheats")?)?;
    Ok(())
}

/// Register campaign addons (`registerQ1CampaignAddons`). Register
/// before map spawning; call `frame_addons` from the shared Q1 source
/// frame phase.
pub fn register_q1_campaign_addons(
    game: &mut Q1EntityServices,
    program: Q1AddonProgram,
    services: Box<dyn Q1AddonServices>,
) -> Result<Q1AddonContext, Q1Error> {
    if !matches!(
        program,
        Q1AddonProgram::Dopa | Q1AddonProgram::Mg1 | Q1AddonProgram::Mg3
    ) {
        return Err(q1_error("Campaign addons require the dopa, mg1, or mg3 source program"));
    }
    let context = register_addon_context(game, program, services)?;
    if program == Q1AddonProgram::Mg3 {
        game.register_spawn("worldspawn", mg3_worldspawn)?;
    }
    register_campaign_addons(&context, game)?;
    register_addon_triggers(&context, game)?;
    register_addon_base_triggers(&context, game)?;
    register_addon_field_triggers(&context, game)?;
    register_addon_brushes(&context, game)?;
    register_addon_effects(&context, game)?;
    register_addon_fog(&context, game)?;
    register_addon_lights(&context, game)?;
    register_addon_corpses(&context, game)?;
    register_addon_monsters(&context, game)?;
    if program == Q1AddonProgram::Mg3 {
        register_addon_ropes(&context, game)?;
        register_mg3_items(game)?;
    }
    Ok(context)
}
