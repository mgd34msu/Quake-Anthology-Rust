//! Classic base monster registry (`src/content/q2/base/monsters/index.ts`).
//!
//! Quake II monster registration. id Software, GPL-2.0-or-later.

use super::actor::{actor_definition, create_actor_target_module};
use super::berserk::berserk_definition;
use super::boss2::boss2_definition;
use super::boss3_stand::boss3_stand_module;
use super::boss_common::with_boss_explosion_callbacks;
use super::brain::brain_definition;
use super::chick::chick_definition;
use super::flipper::flipper_definition;
use super::floater::floater_definition;
use super::flyer::flyer_definition;
use super::gladiator::gladiator_definition;
use super::gunner::gunner_definition;
use super::hover::hover_definition;
use super::insane::insane_definition;
use super::jorg::create_jorg_definition;
use super::makron::{makron_definition, with_makron_spawn_callbacks};
use super::medic::create_medic_definition;
use super::mutant::{MutantSource, create_mutant_definition};
use super::parasite::parasite_definition;
use super::supertank::supertank_definition;
use super::tank::{tank_commander_definition, tank_definition};
use crate::q2::foundation::host::{Q2GameServices, SpawnModule};
use crate::q2::foundation::monsters::register_monster;
use crate::q2::foundation::monsters::types::Q2MonsterDefinition;

/// Classic base monster definitions (`q2ClassicBaseMonsterDefinitions`).
pub fn q2_classic_base_monster_definitions() -> Vec<Q2MonsterDefinition> {
    [
        actor_definition(),
        berserk_definition(),
        boss2_definition(),
        brain_definition(),
        chick_definition(),
        flipper_definition(),
        floater_definition(),
        flyer_definition(),
        gladiator_definition(),
        gunner_definition(),
        hover_definition(),
        insane_definition(),
        create_jorg_definition(),
        with_makron_spawn_callbacks(makron_definition()),
        create_medic_definition(),
        create_mutant_definition(MutantSource::Classic),
        parasite_definition(),
        supertank_definition(),
        tank_definition(),
        tank_commander_definition(),
    ]
    .into_iter()
    .map(with_boss_explosion_callbacks)
    .collect()
}

/// Register a spawn module and its callbacks.
fn register_spawn_module(game: &mut Q2GameServices, module: SpawnModule) {
    game.source_callbacks.register(&module.callbacks);
    game.modules.push(module);
}

/// Register classic base monsters (`registerQ2ClassicBaseMonsters`).
pub fn register_q2_classic_base_monsters(game: &mut Q2GameServices) {
    register_spawn_module(game, create_actor_target_module());
    for definition in q2_classic_base_monster_definitions() {
        if let Some(callbacks) = definition.source_callbacks.clone() {
            game.source_callbacks.register(&callbacks);
        }
        register_monster(game, definition, None);
    }
    register_spawn_module(game, boss3_stand_module());
}
