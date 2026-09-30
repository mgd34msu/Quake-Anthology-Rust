//! Q1 mg3 heavy runtime (`src/content/q1/addons/monsters/heavy/runtime.ts`).
//!
//! `quakec_mg3/monsters/{mg3_super_shambler,mg3_rknight,mg3_lavaman}.qc`
//! shared definition dispatch. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use qa_core::identity::ActorId;

use crate::q1::addons::context::Q1AddonContext;
use crate::q1::base::animation::MonsterFrame;
use crate::q1::base::monsters::BaseMonsterState;
use crate::q1::base::species::MonsterSpecies;
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1StateExtension};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::{q1_error, Q1Error};
use crate::value::{num, obj, SaveReader};

use super::lava_man::{
    lava_man_actions, lava_man_attack, lava_man_die, lava_man_melee, lava_man_pain, lava_man_spawn,
    register_lava_man_callbacks, LAVA_MAN_SPEC,
};
use super::projectiles::register_heavy_projectiles;
use super::rune_knight::{
    rune_knight_actions, rune_knight_die, rune_knight_melee, rune_knight_pain, rune_knight_sight, rune_knight_spawn,
    RUNE_KNIGHT_SPEC,
};
use super::super_shambler::{
    super_shambler_actions, super_shambler_die, super_shambler_melee, super_shambler_pain, super_shambler_spawn,
    SUPER_SHAMBLER_SPEC,
};
use super::tables::{mg3_lavaman, mg3_rknight, mg3_super_shambler};
use crate::q1::addons::monsters::ai::{
    clone_monster_controller, register_mg3_monster_callbacks, register_mg3_monster_source, Mg3ActionHandler,
    Mg3Monster, Mg3SourceHooks,
};
use crate::q1::addons::monsters::startup::{
    init_mg3_monster, mg3_use_mapped, register_mg3_monster_startup, start_mg3_monster,
};

/// Heavy monster callback prefix (`heavyPrefix`).
pub const HEAVY_PREFIX: &str = "mg3:heavy";

/// Heavy monster definition (`HeavyDefinition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeavyDefinitionId {
    /// Super shambler.
    SuperShambler,
    /// Rune knight.
    RuneKnight,
    /// Lava man.
    LavaMan,
}

/// Resolve a heavy definition by classname.
pub fn heavy_definition_id(classname: &str) -> Option<HeavyDefinitionId> {
    match classname {
        "monster_super_shambler" => Some(HeavyDefinitionId::SuperShambler),
        "monster_ranged_knight" => Some(HeavyDefinitionId::RuneKnight),
        "monster_lava_man" => Some(HeavyDefinitionId::LavaMan),
        _ => None,
    }
}

/// Resolve the definition for a heavy monster entity.
fn heavy_definition(monster: &Mg3Monster) -> Result<HeavyDefinitionId, Q1Error> {
    let classname = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    heavy_definition_id(&classname).ok_or_else(|| {
        q1_error(format!(
            "Heavy decision received another source controller: {classname}"
        ))
    })
}

/// Heavy spawn spec by classname.
fn heavy_spec(classname: &str) -> Result<&'static MonsterSpecies, Q1Error> {
    match heavy_definition_id(classname) {
        Some(HeavyDefinitionId::SuperShambler) => Ok(&SUPER_SHAMBLER_SPEC),
        Some(HeavyDefinitionId::RuneKnight) => Ok(&RUNE_KNIGHT_SPEC),
        Some(HeavyDefinitionId::LavaMan) => Ok(&LAVA_MAN_SPEC),
        None => Err(q1_error(format!("Missing heavy addon monster {classname}"))),
    }
}

/// Merged heavy frames.
pub fn heavy_frames() -> &'static HashMap<String, MonsterFrame> {
    static FRAMES: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    FRAMES.get_or_init(|| {
        let mut frames = HashMap::new();
        frames.extend(
            mg3_super_shambler::frames()
                .iter()
                .map(|(name, frame)| (name.clone(), *frame)),
        );
        frames.extend(mg3_rknight::frames().iter().map(|(name, frame)| (name.clone(), *frame)));
        frames.extend(mg3_lavaman::frames().iter().map(|(name, frame)| (name.clone(), *frame)));
        frames
    })
}

/// Merged heavy actions.
pub fn heavy_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        let mut actions = HashMap::new();
        actions.extend(
            super_shambler_actions()
                .iter()
                .map(|(name, action)| (name.clone(), *action)),
        );
        actions.extend(
            rune_knight_actions()
                .iter()
                .map(|(name, action)| (name.clone(), *action)),
        );
        actions.extend(lava_man_actions().iter().map(|(name, action)| (name.clone(), *action)));
        actions
    })
}

fn heavy_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = crate::q1::base::creatures::monster_controller(game, id, classname)?;
    let spec = heavy_spec(classname)?;
    Ok((controller, spec))
}

/// Install heavy pain, death and route callbacks
/// (`installCallbacks`).
pub fn heavy_install_callbacks(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let pain = monster
        .monster
        .game
        .named
        .pain(&format!("{HEAVY_PREFIX}:monster_pain"))?;
    let die = monster.monster.game.named.die(&format!("{HEAVY_PREFIX}:monster_die"))?;
    let path_end = monster
        .monster
        .game
        .named
        .action(&format!("{HEAVY_PREFIX}:monster_stand"))?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.pain = Some(pain);
        entity.die = Some(die);
        entity.path_end = Some(path_end);
    })
}

/// Install heavy callbacks and initialize (`initialize`).
pub fn heavy_initialize(monster: &mut Mg3Monster, context: &Q1AddonContext, size: u32) -> Result<(), Q1Error> {
    heavy_install_callbacks(monster)?;
    let model = format!("progs/{}.mdl", monster.monster.spec.model);
    init_mg3_monster(monster, context, &model, 1, size)
}

fn heavy_start(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let context = Q1AddonContext::new(crate::q1::addons::context::Q1AddonProgram::Mg3);
    start_mg3_monster(monster, &context)
}

fn heavy_sight(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    match heavy_definition(monster)? {
        HeavyDefinitionId::RuneKnight => rune_knight_sight(monster),
        _ => monster.sight_sound_default(),
    }
}

fn heavy_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    match heavy_definition(monster)? {
        HeavyDefinitionId::SuperShambler => super_shambler_melee(monster),
        HeavyDefinitionId::RuneKnight => rune_knight_melee(monster),
        HeavyDefinitionId::LavaMan => lava_man_melee(monster),
    }
}

fn heavy_try_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    match heavy_definition(monster)? {
        HeavyDefinitionId::LavaMan => lava_man_attack(monster),
        _ => monster.check_attack(),
    }
}

fn heavy_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    monster.retaliate(attacker.as_ref())?;
    match heavy_definition(monster)? {
        HeavyDefinitionId::SuperShambler => super_shambler_pain(monster, attacker.as_ref(), damage),
        HeavyDefinitionId::RuneKnight => rune_knight_pain(monster, attacker.as_ref(), damage),
        HeavyDefinitionId::LavaMan => lava_man_pain(monster, attacker.as_ref(), damage),
    }
}

fn heavy_die(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    if monster.monster.controller.counted_death {
        return Ok(());
    }
    monster.monster.monster.enemy = attacker.cloned();
    let id = monster.monster.id.clone();
    if monster.monster.game.health(&id) < -99.0 {
        monster.monster.game.set_health(&id, -99.0)?;
    }
    monster.monster.game.set_damageable(&id, false)?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.touch = None;
    })?;
    monster.monster.count_kill()?;
    match heavy_definition(monster)? {
        HeavyDefinitionId::SuperShambler => super_shambler_die(monster),
        HeavyDefinitionId::RuneKnight => rune_knight_die(monster),
        HeavyDefinitionId::LavaMan => lava_man_die(monster),
    }
}

fn heavy_source_die(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    match heavy_definition(&monster)? {
        HeavyDefinitionId::SuperShambler => super_shambler_die(&mut monster),
        HeavyDefinitionId::RuneKnight => rune_knight_die(&mut monster),
        HeavyDefinitionId::LavaMan => lava_man_die(&mut monster),
    }?;
    monster.finish()
}

fn spawn_heavy(game: &mut Q1EntityServices, id: &ActorId, classname: &str) -> Result<(), Q1Error> {
    let context = Q1AddonContext::new(crate::q1::addons::context::Q1AddonProgram::Mg3);
    let spec = heavy_spec(classname)?;
    let mut monster = Mg3Monster::spawn_new(game, HEAVY_PREFIX, id, spec)?;
    match heavy_definition_id(classname) {
        Some(HeavyDefinitionId::SuperShambler) => super_shambler_spawn(&mut monster, &context),
        Some(HeavyDefinitionId::RuneKnight) => rune_knight_spawn(&mut monster, &context),
        Some(HeavyDefinitionId::LavaMan) => lava_man_spawn(&mut monster, &context),
        None => Err(q1_error(format!("Missing heavy addon monster {classname}"))),
    }?;
    monster.finish()
}

fn spawn_super_shambler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_heavy(game, id, "monster_super_shambler")
}

fn spawn_rune_knight(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_heavy(game, id, "monster_ranged_knight")
}

fn spawn_lava_man(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_heavy(game, id, "monster_lava_man")
}

fn heavy_games() -> &'static Mutex<HashMap<usize, f64>> {
    static GAMES: OnceLock<Mutex<HashMap<usize, f64>>> = OnceLock::new();
    GAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn heavy_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

/// Read the rune-knight melee cycle (`runeKnightMeleeCycle`).
pub fn heavy_melee_cycle(game: &Q1EntityServices) -> Result<f64, Q1Error> {
    heavy_games()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&heavy_key(game))
        .copied()
        .ok_or_else(|| q1_error("MG3 heavy monsters are not registered"))
}

/// Write the rune-knight melee cycle.
pub fn set_heavy_melee_cycle(game: &Q1EntityServices, cycle: f64) -> Result<(), Q1Error> {
    heavy_games()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_mut(&heavy_key(game))
        .map(|slot| *slot = cycle)
        .ok_or_else(|| q1_error("MG3 heavy monsters are not registered"))
}

/// Register mg3 heavy monsters (`registerMg3Heavy`). Monster
/// controllers persist through the base creature store; only the
/// rune-knight melee cycle needs its own checkpoint word.
pub fn register_mg3_heavy(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    struct Extension;

    impl Q1StateExtension for Extension {
        fn id(&self) -> &str {
            "mg3:heavy"
        }

        fn capture(&self, game: &Q1EntityServices) -> Vec<u8> {
            let cycle = heavy_melee_cycle(game).unwrap_or(0.0);
            encode_checkpoint_value(&obj(vec![("runeKnightMeleeCycle", num(cycle))]))
        }

        fn restore(&mut self, game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
            let saved = decode_checkpoint_value(bytes)?;
            let cycle = SaveReader::new(&saved).field("runeKnightMeleeCycle").number()?;
            set_heavy_melee_cycle(game, cycle)
        }

        fn clone_state(
            &mut self,
            game: &mut Q1EntityServices,
            source: &ActorId,
            target: &ActorId,
        ) -> Result<(), Q1Error> {
            clone_monster_controller(game, source, target, HEAVY_PREFIX)
        }
    }

    heavy_games()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(heavy_key(game), 0.0);
    register_mg3_monster_source(
        HEAVY_PREFIX,
        heavy_frames(),
        heavy_actions(),
        Mg3SourceHooks {
            start: Some(heavy_start),
            use_monster: Some(mg3_use_mapped),
            sight_sound: Some(heavy_sight),
            melee_attack: Some(heavy_melee),
            try_attack: Some(heavy_try_attack),
            pain: Some(heavy_pain),
            die: Some(heavy_die),
            ..Default::default()
        },
        heavy_load_controller,
        crate::q1::base::creatures::store_monster_controller,
    );
    register_mg3_monster_callbacks(game, HEAVY_PREFIX)?;
    register_mg3_monster_startup(game, HEAVY_PREFIX)?;
    register_heavy_projectiles(game)?;
    register_lava_man_callbacks(game)?;
    game.named.register(
        &format!("{HEAVY_PREFIX}:source_die"),
        Q1CallbackHandlers {
            action: Some(heavy_source_die as crate::q1::foundation::callbacks::Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.register_state_extension(Box::new(Extension))?;
    game.register_spawn("monster_super_shambler", spawn_super_shambler)?;
    game.register_spawn("monster_ranged_knight", spawn_rune_knight)?;
    game.register_spawn("monster_lava_man", spawn_lava_man)?;
    let _ = context;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::lava_man::lava_man_actions;
    use super::super::rune_knight::rune_knight_actions;
    use super::super::super_shambler::super_shambler_actions;
    use super::*;

    #[test]
    fn heavy_tables_merge_without_collisions() {
        let merged = heavy_frames().len();
        let total = mg3_super_shambler::frames().len() + mg3_rknight::frames().len() + mg3_lavaman::frames().len();
        assert_eq!(merged, total);
        let actions = heavy_actions().len();
        let total = super_shambler_actions().len() + rune_knight_actions().len() + lava_man_actions().len();
        assert_eq!(actions, total);
    }
}
