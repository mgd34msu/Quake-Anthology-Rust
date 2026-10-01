//! Rerelease monster registry (`src/content/q2/rerelease/monsters/index.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;

use super::arachnid::arachnid_definition;
use super::base_variants::actor::{rerelease_actor_definition, rerelease_actor_targets};
use super::base_variants::boss2::rerelease_boss2_definition;
use super::base_variants::brain::create_rerelease_brain_definition;
use super::base_variants::chick::create_rerelease_chick_definitions;
use super::base_variants::flipper::rerelease_flipper_definition;
use super::base_variants::floater::rerelease_floater_definition;
use super::base_variants::flyer::create_rerelease_flyer_definition;
use super::base_variants::hover::create_rerelease_hover_definition;
use super::base_variants::insane::rerelease_insane_definition;
use super::base_variants::jorg::create_rerelease_jorg_definition;
use super::base_variants::makron::rerelease_makron_definition;
use super::base_variants::medic::create_rerelease_medic_definitions;
use super::base_variants::mutant::create_rerelease_mutant_definition;
use super::base_variants::parasite::create_rerelease_parasite_definition;
use super::berserk::create_rerelease_berserk_definition;
use super::gladiator::create_rerelease_gladiator_definitions;
use super::guardian::guardian_definition;
use super::guncmdr::create_gun_commander_definition;
use super::gunner::rerelease_gunner_definition;
use super::shambler::shambler_definition;
use super::supertank::create_rerelease_supertank_definitions;
use super::tank::{create_rerelease_tank_definitions, rerelease_tank_stand_module};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, SpawnModule};
use crate::q2::foundation::monsters::register_monster;
use crate::q2::foundation::monsters::types::Q2MonsterDefinition;

/// Ordinary rerelease monster definitions (`ordinaryDefinitions`).
fn ordinary_definitions() -> Vec<Q2MonsterDefinition> {
    vec![
        create_rerelease_berserk_definition(),
        rerelease_gunner_definition(),
        rerelease_flipper_definition(),
        rerelease_floater_definition(),
        create_rerelease_hover_definition(),
        create_rerelease_flyer_definition(),
        create_rerelease_mutant_definition(),
        create_rerelease_parasite_definition(),
        create_rerelease_brain_definition(),
    ]
}

/// Register a definition and its source callbacks.
fn register_rerelease_definition(game: &mut Q2GameServices, definition: Q2MonsterDefinition) {
    if let Some(callbacks) = definition.source_callbacks.clone() {
        game.source_callbacks.register(&callbacks);
    }
    register_monster(game, definition, Some(Q2Edition::Rerelease));
}

/// Register a spawn module and its callbacks.
fn register_spawn_module(game: &mut Q2GameServices, module: SpawnModule) {
    game.source_callbacks.register(&module.callbacks);
    game.modules.push(module);
}

/// Register rerelease ordinary monsters (`registerQ2RereleaseOrdinaryMonsters`).
pub fn register_q2_rerelease_ordinary_monsters(game: &mut Q2GameServices) {
    for definition in ordinary_definitions() {
        register_rerelease_definition(game, definition);
    }
    for definition in [
        create_rerelease_chick_definitions(),
        create_rerelease_tank_definitions(),
        create_rerelease_gladiator_definitions(),
    ]
    .into_iter()
    .flatten()
    {
        if definition.classname == "monster_chick_heat" || definition.classname == "monster_gladb" {
            continue;
        }
        register_rerelease_definition(game, definition);
    }
}

/// Register rerelease monsters (`registerQ2RereleaseMonsters`).
pub fn register_q2_rerelease_monsters(
    game: &mut Q2GameServices,
    is_n64: bool,
    healthbar_transfer: Option<fn(ActorId, ActorId, &mut Q2GameServices)>,
) {
    game.monsters.hooks.healthbar_transfer = healthbar_transfer;
    register_rerelease_definition(game, arachnid_definition());
    register_rerelease_definition(game, create_rerelease_berserk_definition());
    register_rerelease_definition(game, guardian_definition());
    register_rerelease_definition(game, shambler_definition());
    register_rerelease_definition(game, create_gun_commander_definition());
    register_rerelease_definition(game, rerelease_gunner_definition());
    register_rerelease_definition(game, rerelease_flipper_definition());
    register_rerelease_definition(game, rerelease_floater_definition());
    register_rerelease_definition(game, create_rerelease_hover_definition());
    register_rerelease_definition(game, create_rerelease_flyer_definition());
    for definition in create_rerelease_chick_definitions() {
        register_rerelease_definition(game, definition);
    }
    register_rerelease_definition(game, create_rerelease_mutant_definition());
    register_rerelease_definition(game, rerelease_insane_definition());
    register_rerelease_definition(game, rerelease_boss2_definition());
    register_rerelease_definition(game, rerelease_makron_definition());
    register_rerelease_definition(game, create_rerelease_jorg_definition());
    register_rerelease_definition(game, create_rerelease_brain_definition());
    register_rerelease_definition(game, create_rerelease_parasite_definition());
    for definition in create_rerelease_medic_definitions() {
        register_rerelease_definition(game, definition);
    }
    register_rerelease_definition(game, rerelease_actor_definition());
    for definition in create_rerelease_tank_definitions() {
        register_rerelease_definition(game, definition);
    }
    for definition in create_rerelease_gladiator_definitions() {
        register_rerelease_definition(game, definition);
    }
    for definition in create_rerelease_supertank_definitions(is_n64) {
        register_rerelease_definition(game, definition);
    }
    register_spawn_module(game, rerelease_tank_stand_module());
    register_spawn_module(game, rerelease_actor_targets());
}
