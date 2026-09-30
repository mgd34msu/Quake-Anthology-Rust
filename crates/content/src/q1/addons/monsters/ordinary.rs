//! Q1 addon ordinary monsters (`src/content/q1/addons/monsters/ordinary`, barrel `index.ts`).
//!
//! `quakec_{mg1,mg3}/monsters/*.qc` native monster admission.
//! GPL-2.0-or-later.

pub mod army;
pub mod attack;
pub mod rocket_ogre;

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{set_addon_number, Q1AddonContext, Q1AddonProgram};
use crate::q1::base::animation::{MonsterAi, MonsterFrame};
use crate::q1::base::frames::monster_frame;
use crate::q1::base::monsters::{BaseMonster, BaseMonsterState};
use crate::q1::base::species::{species_by_classname, MonsterSpecies, BASE_SPECIES};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1MoveType, Q1Solid};
use crate::q1::{q1_error, Q1Error};

use super::ai::{
    clone_monster_controller, register_mg3_monster_callbacks, register_mg3_monster_source, Mg3ActionHandler,
    Mg3Monster, Mg3SourceHooks,
};
use super::startup::{init_mg3_monster, mg3_use_mapped, register_mg3_monster_startup, start_mg3_monster};
use attack::mg3_ordinary_attack;
use rocket_ogre::{register_rocket_ogre, rocket_ogre_frame, rocket_ogre_frames};

const SMALL: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 40.0,
    },
};

/// Ordinary extra frames (`extraFrames`).
fn ordinary_frames() -> &'static HashMap<String, MonsterFrame> {
    static FRAMES: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    FRAMES.get_or_init(|| {
        let hang = monster_frame("zombie_paine1").expect("Missing native zombie hang model frame");
        HashMap::from([(
            String::from("zombie_hang1"),
            MonsterFrame {
                frame: hang.frame,
                next: "zombie_hang1",
                operations: &[],
            },
        )])
    })
}

/// Ordinary frame actions. The donor passes no actions map; frame
/// actions resolve through the shared fallback.
fn ordinary_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(HashMap::new)
}

/// Mg3 zombie spec with the missile alias (`sourceSpec`).
fn zombie_mg3_spec() -> &'static MonsterSpecies {
    static SPEC: OnceLock<MonsterSpecies> = OnceLock::new();
    SPEC.get_or_init(|| {
        let base = species_by_classname("monster_zombie").expect("Missing native addon monster monster_zombie");
        MonsterSpecies {
            missile: Some("zombie_missile"),
            ..*base
        }
    })
}

/// Mg3 zombie spec without missiles (`sourceSpec`).
fn zombie_hang_spec() -> &'static MonsterSpecies {
    static SPEC: OnceLock<MonsterSpecies> = OnceLock::new();
    SPEC.get_or_init(|| {
        let base = species_by_classname("monster_zombie").expect("Missing native addon monster monster_zombie");
        MonsterSpecies {
            missile: None,
            melee: true,
            ..*base
        }
    })
}

/// Rocket-ogre spec (`sourceSpec`).
fn rocket_ogre_spec() -> &'static MonsterSpecies {
    static SPEC: OnceLock<MonsterSpecies> = OnceLock::new();
    SPEC.get_or_init(|| {
        let base = species_by_classname("monster_ogre").expect("Missing native addon monster monster_ogre_rocket");
        MonsterSpecies {
            model: "ogre_rocket",
            sight: "armagon/sight.wav",
            ..*base
        }
    })
}

/// Ordinary fish spec with small bounds (`sourceSpec`).
fn fish_ordinary_spec() -> &'static MonsterSpecies {
    static SPEC: OnceLock<MonsterSpecies> = OnceLock::new();
    SPEC.get_or_init(|| {
        let base = species_by_classname("monster_fish").expect("Missing native addon monster monster_fish");
        MonsterSpecies { bounds: SMALL, ..*base }
    })
}

fn ordinary_program(game: &Q1EntityServices, id: &ActorId) -> Result<Q1AddonProgram, Q1Error> {
    match game
        .entity_ref(id)
        .map(|entity| entity.text("source.monsterCallbackPrefix"))
        .unwrap_or_default()
        .as_str()
    {
        "dopa:ordinary" => Ok(Q1AddonProgram::Dopa),
        "mg1:ordinary" => Ok(Q1AddonProgram::Mg1),
        "mg3:ordinary" => Ok(Q1AddonProgram::Mg3),
        _ => Err(q1_error("Missing ordinary addon source")),
    }
}

/// Resolve an ordinary spawn spec (`sourceSpec`).
fn ordinary_spec(game: &Q1EntityServices, id: &ActorId, classname: &str) -> Result<&'static MonsterSpecies, Q1Error> {
    let entity = game.entity_ref(id).ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let class = entity.text("addon.monsterClass");
    let class = if class.is_empty() { classname } else { &class };
    let program = ordinary_program(game, id)?;
    let rocket = program == Q1AddonProgram::Mg3 && class == "monster_ogre_rocket";
    let base = species_by_classname(if rocket { "monster_ogre" } else { class })
        .ok_or_else(|| q1_error(format!("Missing native addon monster {class}")))?;
    if rocket {
        return Ok(rocket_ogre_spec());
    }
    if base.species == Q1MonsterSpecies::Zombie && program == Q1AddonProgram::Mg3 {
        if entity.spawnflags & 128 != 0 {
            return Ok(zombie_hang_spec());
        }
        return Ok(zombie_mg3_spec());
    }
    if base.species == Q1MonsterSpecies::Fish {
        return Ok(fish_ordinary_spec());
    }
    Ok(base)
}

fn ordinary_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = crate::q1::base::creatures::monster_controller(game, id, classname)?;
    let spec = ordinary_spec(game, id, classname)?;
    Ok((controller, spec))
}

fn base_ai(monster: &mut Mg3Monster, mode: MonsterAi, distance: f64) -> Result<(), Q1Error> {
    monster.monster.ai(mode, distance)
}

fn base_run(monster: &mut Mg3Monster, distance: f64) -> Result<(), Q1Error> {
    monster.monster.run(distance)
}

fn base_find_target(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    monster.monster.find_target()
}

fn base_found(monster: &mut Mg3Monster, target: &ActorId) -> Result<(), Q1Error> {
    let target = target.clone();
    monster.monster.found(&target)
}

fn base_try_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    monster.monster.try_attack()
}

fn base_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    monster.monster.pain(attacker.as_ref(), damage)
}

fn ordinary_start(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let context = Q1AddonContext::new(ordinary_program(monster.monster.game, &monster.monster.id.clone())?);
    start_mg3_monster(monster, &context)
}

fn ordinary_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    if monster.monster.spec.species != Q1MonsterSpecies::Zombie {
        return monster.monster.melee_attack();
    }
    let rolled = monster.monster.game.host.random();
    monster.play(if rolled < 0.3 {
        "zombie_atta1"
    } else if rolled < 0.6 {
        "zombie_attb1"
    } else {
        "zombie_attc1"
    })
}

fn ordinary_mg3_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
    if monster.monster.spec.model != "ogre_rocket" {
        let attacker = attacker.cloned();
        return monster.pain_default(attacker.as_ref(), damage);
    }
    if monster.monster.monster.pain_finished > monster.monster.game.time
        || monster.monster.game.host.random() * 200.0 > damage
    {
        return Ok(());
    }
    monster
        .monster
        .game
        .sound_simple(&monster.monster.id.clone(), "armagon/pain.wav")?;
    let rolled = monster.monster.game.host.random();
    monster.monster.monster.pain_finished = monster.monster.game.time + if rolled < 0.75 { 3.0 } else { 4.0 };
    monster.play(if rolled < 0.25 {
        "ogre_pain1"
    } else if rolled < 0.5 {
        "ogre_painb1"
    } else if rolled < 0.75 {
        "ogre_painc1"
    } else if rolled < 0.88 {
        "ogre_paind1"
    } else {
        "ogre_paine1"
    })
}

fn ordinary_mg3_die(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    if monster.monster.spec.model != "ogre_rocket" || monster.monster.game.health(&monster.monster.id.clone()) < -80.0 {
        let attacker = attacker.cloned();
        return monster.monster.die(attacker.as_ref());
    }
    if monster.monster.controller.counted_death {
        return Ok(());
    }
    monster.monster.monster.enemy = attacker.cloned();
    monster
        .monster
        .game
        .set_damageable(&monster.monster.id.clone(), false)?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.touch = None;
        })?;
    monster.monster.count_kill()?;
    monster
        .monster
        .game
        .sound_simple(&monster.monster.id.clone(), "armagon/sight2.wav")?;
    let rolled = monster.monster.game.host.random();
    monster.play(if rolled < 0.5 { "ogre_die1" } else { "ogre_bdie1" })
}

fn ordinary_mg3_play(monster: &mut Mg3Monster, name: &str) -> Result<(), Q1Error> {
    if name == "zombie_missile" {
        return monster.melee_attack();
    }
    let rocket = monster.monster.spec.model == "ogre_rocket";
    if rocket && name == "ogre_stand5" {
        rocket_ogre_frame(monster, name)?;
    }
    if rocket && rocket_ogre_frames().contains_key(name) {
        if !monster.play_preamble()? {
            let frame = rocket_ogre_frames()[name];
            monster.play_ops(name, frame)?;
        }
    } else {
        monster.play_default(name)?;
    }
    if rocket && name != "ogre_stand5" {
        rocket_ogre_frame(monster, name)?;
    }
    Ok(())
}

/// Spawn an ordinary addon monster (`registerOrdinaryAddonMonsters`
/// spawn closure).
fn spawn_ordinary(
    game: &mut Q1EntityServices,
    id: &ActorId,
    prefix: &'static str,
    program: Q1AddonProgram,
) -> Result<(), Q1Error> {
    let classname = game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    game.update_entity(id, |entity| {
        entity.fields.insert(String::from("addon.monsterClass"), classname);
    })?;
    let context = Q1AddonContext::new(program);
    let spec = ordinary_spec(
        game,
        id,
        &game
            .entity_ref(id)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default(),
    )?;
    let mut monster = Mg3Monster::spawn_new(game, prefix, id, spec)?;
    if program == Q1AddonProgram::Mg3 && monster.monster.spec.model == "ogre_rocket" {
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.classname = String::from("monster_ogre");
                entity.fields.insert(String::from("aflag"), String::from("1"));
                entity.fields.insert(String::from("projectiles_max"), String::from("2"));
                entity.fields.insert(String::from("projectiles"), String::from("2"));
            })?;
    }
    spawn_ordinary_body(&mut monster, &context, prefix)?;
    monster.finish()
}

fn spawn_ordinary_body(monster: &mut Mg3Monster, context: &Q1AddonContext, prefix: &str) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let health = monster.monster.spec.health;
    monster.monster.game.set_health(&id, health)?;
    let pain = monster.monster.game.named.pain(&format!("{prefix}:monster_pain"))?;
    let die = monster.monster.game.named.die(&format!("{prefix}:monster_die"))?;
    let path_end = monster.monster.game.named.action(&format!("{prefix}:monster_stand"))?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.pain = Some(pain);
        entity.die = Some(die);
        entity.path_end = Some(path_end);
    })?;
    let entity = monster
        .monster
        .game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if monster.monster.spec.species == Q1MonsterSpecies::Zombie
        && (entity.spawnflags & 1 != 0 || context.program() == Q1AddonProgram::Mg3 && entity.spawnflags & 8388608 != 0)
    {
        monster.monster.game.update_entity(&id, |entity| {
            entity.solid = Q1Solid::Slidebox;
            entity.movement = Q1MoveType::None;
            entity.model = String::from("progs/zombie.mdl");
        })?;
        monster.monster.game.set_bounds(&id, SMALL)?;
        monster.monster.game.link(&id)?;
        return monster.play(
            if context.program() == Q1AddonProgram::Mg3 && entity.spawnflags & 8388608 != 0 {
                "zombie_hang1"
            } else {
                "zombie_cruc1"
            },
        );
    }
    if monster.monster.spec.species == Q1MonsterSpecies::Fish {
        monster.monster.game.update_entity(&id, |entity| {
            entity.spawnflags |= 16384;
        })?;
    }
    if monster.monster.spec.species != Q1MonsterSpecies::Wizard
        && monster.monster.spec.species != Q1MonsterSpecies::Fish
    {
        set_addon_number(monster.monster.game, &id, "allowPathFind", 1.0)?;
    }
    let style = match monster.monster.spec.species {
        Q1MonsterSpecies::Knight | Q1MonsterSpecies::Demon | Q1MonsterSpecies::Tarbaby | Q1MonsterSpecies::Fish => 2.0,
        Q1MonsterSpecies::Ogre | Q1MonsterSpecies::Hellknight | Q1MonsterSpecies::Shambler => 3.0,
        _ => 1.0,
    };
    set_addon_number(monster.monster.game, &id, "combat_style", style)?;
    let model = format!("progs/{}.mdl", monster.monster.spec.model);
    let kind = if monster.monster.spec.species == Q1MonsterSpecies::Wizard {
        2
    } else if monster.monster.spec.species == Q1MonsterSpecies::Fish {
        3
    } else {
        1
    };
    let size = match monster.monster.spec.species {
        Q1MonsterSpecies::Demon | Q1MonsterSpecies::Ogre | Q1MonsterSpecies::Shambler | Q1MonsterSpecies::Shalrath => 2,
        _ => 1,
    };
    init_mg3_monster(monster, context, &model, kind, size)
}

fn spawn_ordinary_dopa(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_ordinary(game, id, "dopa:ordinary", Q1AddonProgram::Dopa)
}

fn spawn_ordinary_mg1(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_ordinary(game, id, "mg1:ordinary", Q1AddonProgram::Mg1)
}

fn spawn_ordinary_mg3(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_ordinary(game, id, "mg3:ordinary", Q1AddonProgram::Mg3)
}

/// Look up a live ordinary (or base) monster controller
/// (`ordinaryAddonMonster`).
pub fn ordinary_addon_monster<'g>(
    game: &'g mut Q1EntityServices,
    id: &ActorId,
) -> Result<Option<BaseMonster<'g>>, Q1Error> {
    if game.entity_ref(id).is_none() {
        return Ok(None);
    }
    match BaseMonster::load(game, id) {
        Ok(monster) => Ok(Some(monster)),
        Err(_) => Ok(None),
    }
}

/// Register ordinary addon monsters (`registerOrdinaryAddonMonsters`).
pub fn register_ordinary_addon_monsters(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let (prefix, spawn): (&'static str, crate::q1::foundation::entity_services::Q1SpawnHandler) =
        match context.program() {
            Q1AddonProgram::Dopa => ("dopa:ordinary", spawn_ordinary_dopa),
            Q1AddonProgram::Mg1 => ("mg1:ordinary", spawn_ordinary_mg1),
            Q1AddonProgram::Mg3 => ("mg3:ordinary", spawn_ordinary_mg3),
            Q1AddonProgram::Ctf => return Err(q1_error("CTF has no ordinary addon monsters")),
        };
    let hooks = if context.program() == Q1AddonProgram::Mg3 {
        Mg3SourceHooks {
            start: Some(ordinary_start),
            use_monster: Some(mg3_use_mapped),
            melee_attack: Some(ordinary_melee),
            try_attack: Some(mg3_ordinary_attack),
            play: Some(ordinary_mg3_play),
            pain: Some(ordinary_mg3_pain),
            die: Some(ordinary_mg3_die),
            ..Default::default()
        }
    } else {
        Mg3SourceHooks {
            ai: Some(base_ai),
            run: Some(base_run),
            find_target: Some(base_find_target),
            found: Some(base_found),
            try_attack: Some(base_try_attack),
            pain: Some(base_pain),
            start: Some(ordinary_start),
            use_monster: Some(mg3_use_mapped),
            melee_attack: Some(ordinary_melee),
            ..Default::default()
        }
    };
    register_mg3_monster_source(
        prefix,
        ordinary_frames(),
        ordinary_actions(),
        hooks,
        ordinary_load_controller,
        crate::q1::base::creatures::store_monster_controller,
    );
    register_mg3_monster_callbacks(game, prefix)?;
    register_mg3_monster_startup(game, prefix)?;
    // Controllers persist through the base creature store; the
    // extension only carries the clone hook.
    struct Extension {
        prefix: &'static str,
    }
    impl crate::q1::foundation::callbacks::Q1StateExtension for Extension {
        fn id(&self) -> &str {
            self.prefix
        }

        fn capture(&self, _game: &Q1EntityServices) -> Vec<u8> {
            crate::q1::foundation::checkpoint::encode_checkpoint_value(&crate::value::arr(Vec::new()))
        }

        fn restore(&mut self, _game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
            let saved = crate::q1::foundation::checkpoint::decode_checkpoint_value(bytes)?;
            crate::value::SaveReader::new(&saved).list(|_entry| Ok::<(), Q1Error>(()))?;
            Ok(())
        }

        fn clone_state(
            &mut self,
            game: &mut Q1EntityServices,
            source: &ActorId,
            target: &ActorId,
        ) -> Result<(), Q1Error> {
            clone_monster_controller(game, source, target, self.prefix)
        }
    }
    game.register_state_extension(Box::new(Extension { prefix }))?;
    for spec in BASE_SPECIES {
        if spec.species == Q1MonsterSpecies::Boss || spec.species == Q1MonsterSpecies::Oldone {
            continue;
        }
        for classname in spec.classnames {
            game.replace_spawn(classname, spawn)?;
        }
    }
    if context.program() == Q1AddonProgram::Mg3 {
        register_rocket_ogre(context, game)?;
        game.register_spawn("monster_ogre_rocket", spawn_ordinary_mg3)?;
    }
    Ok(())
}
