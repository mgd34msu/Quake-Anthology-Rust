//! Q1 mg3 infected controller (`src/content/q1/addons/monsters/infected/controller.ts`).
//!
//! `quakec_mg3/monsters/mg3_*_infected.qc` and `monsters.qc`.
//! GPL-2.0-or-later.
//!
//! The donor replaces the whole controller object on transformation
//! and resurrection; the replacement restores the captured state
//! wholesale, so the only observable change is the resolved spec.
//! The port therefore swaps the spec in place.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::{same_actor, ActorId};

use crate::q1::addons::context::{set_addon_number, Q1AddonContext, Q1AddonProgram};
use crate::q1::addons::monsters::ordinary::army::{army_actions, army_pain};
use crate::q1::addons::monsters::ordinary::attack::mg3_ordinary_attack;
use crate::q1::base::animation::MonsterAi;
use crate::q1::base::monsters::{BaseMonster, BaseMonsterState};
use crate::q1::base::projectiles::{throw_gib, throw_head};
use crate::q1::base::provider::register_kill_count_rule;
use crate::q1::base::species::MonsterSpecies;
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1StateExtension};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Event, Q1Solid, Q1SoundChannel};
use crate::q1::{q1_error, Q1Error};

use super::frames::infected_frames;
use super::species::{infected_kind, infected_kind_for_classname, infected_species, InfectedKind};
use crate::q1::addons::monsters::ai::{
    clone_monster_controller, register_mg3_monster_callbacks, register_mg3_monster_source, Mg3ActionHandler,
    Mg3Monster, Mg3SourceHooks,
};
use crate::q1::addons::monsters::startup::{
    init_mg3_monster, mg3_use_mapped, register_mg3_monster_startup, start_mg3_monster,
};

/// Infected callback prefix (`infectedPrefix`).
pub const INFECTED_PREFIX: &str = "mg3:infected";

/// Infected frame actions (`actions`).
fn infected_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn corpse_hold(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.delay(9999.0)
        }
        fn test_rise(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster.monster.game.update_entity(&id, |entity| {
                entity.solid = Q1Solid::Slidebox;
            })?;
            let owned = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.actor.clone())
                .ok_or_else(|| q1_error("Missing Q1 entity"))?;
            if !monster.monster.game.host.walk_move(&owned, 0.0, 0.0) {
                monster.monster.game.update_entity(&id, |entity| {
                    entity.solid = Q1Solid::None;
                })?;
                monster.monster.game.link(&id)?;
                monster.monster.controller.next_frame = monster.monster.controller.current_frame.clone();
                return monster.monster.delay(5.0);
            }
            monster.monster.game.link(&id)?;
            monster
                .monster
                .game
                .sound(&id, "infected/death1_rev.wav", Q1SoundChannel::Voice, 1.0, 1.0)
        }
        fn rise_pain(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.monster.pain_finished = monster.monster.game.time + 1.5;
            Ok(())
        }
        fn resurrect(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .invoke_action(&id, &format!("{INFECTED_PREFIX}:resurrect"))
        }
        let mut actions = HashMap::new();
        actions.extend(army_actions().iter().map(|(name, action)| (name.clone(), *action)));
        actions.extend([
            (String::from("infected_corpse_hold"), corpse_hold as Mg3ActionHandler),
            (String::from("infected_test_rise"), test_rise as Mg3ActionHandler),
            (String::from("infected_rise_pain"), rise_pain as Mg3ActionHandler),
            (String::from("infected_resurrect"), resurrect as Mg3ActionHandler),
        ]);
        actions
    })
}

fn infected_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = crate::q1::base::creatures::monster_controller(game, id, classname)?;
    let spec = infected_species(game, id)?;
    Ok((controller, spec))
}

fn infected_start(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let context = Q1AddonContext::new(Q1AddonProgram::Mg3);
    start_mg3_monster(monster, &context)
}

fn infected_try_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    mg3_ordinary_attack(monster)
}

fn infected_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
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

fn infected_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
    if monster.monster.spec.species == Q1MonsterSpecies::Enforcer {
        let attacker = attacker.cloned();
        monster.retaliate(attacker.as_ref())?;
        let rolled = monster.monster.game.host.random();
        if monster.monster.monster.pain_finished > monster.monster.game.time
            || monster.monster.game.options().skill > 2 && monster.monster.game.host.random() * 200.0 > damage
        {
            return Ok(());
        }
        let id = monster.monster.id.clone();
        monster.monster.game.sound(
            &id,
            if rolled < 0.5 {
                "enforcer/pain1.wav"
            } else {
                "enforcer/pain2.wav"
            },
            Q1SoundChannel::Voice,
            1.0,
            1.0,
        )?;
        monster.monster.monster.pain_finished = monster.monster.game.time + if rolled < 0.7 { 1.0 } else { 2.0 };
        return monster.play(if rolled < 0.2 {
            "enf_paina1"
        } else if rolled < 0.4 {
            "enf_painb1"
        } else if rolled < 0.7 {
            "enf_painc1"
        } else {
            "enf_paind1"
        });
    }
    if monster.monster.spec.species == Q1MonsterSpecies::Army {
        return army_pain(monster, attacker, damage, true);
    }
    let attacker = attacker.cloned();
    monster.pain_default(attacker.as_ref(), damage)
}

fn infected_die(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    if monster.monster.controller.counted_death {
        return Ok(());
    }
    let id = monster.monster.id.clone();
    let transformed = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("infected.transformed"))
        .unwrap_or(0.0)
        != 0.0;
    if transformed && monster.monster.spec.species != Q1MonsterSpecies::Zombie {
        let attacker = attacker.cloned();
        return monster.monster.die(attacker.as_ref());
    }
    if monster.monster.game.health(&id) < -99.0 {
        monster.monster.game.set_health(&id, -99.0)?;
    }
    monster.monster.monster.enemy = attacker.cloned();
    monster.monster.game.set_damageable(&id, false)?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.touch = None;
    })?;
    monster.monster.count_kill()?;
    if transformed {
        monster
            .monster
            .game
            .sound(&id, "zombie/z_gib.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
        let health = monster.monster.game.health(&id);
        throw_head(monster.monster.game, &id, "h_zombie", health)?;
        for model in ["gib1", "gib2", "gib3"] {
            let origin = monster.monster.origin()?;
            throw_gib(monster.monster.game, origin, model, health)?;
        }
        return Ok(());
    }
    monster
        .monster
        .game
        .sound(&id, "player/udeath.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    let health = monster.monster.game.health(&id);
    if infected_kind(monster.monster.game, &id)? == InfectedKind::Army {
        let origin = monster.monster.origin()?;
        throw_gib(monster.monster.game, origin, "h_guard", health)?;
    }
    for model in ["gib1", "gib2", "gib3"] {
        let origin = monster.monster.origin()?;
        throw_gib(monster.monster.game, origin, model, health)?;
    }
    set_addon_number(monster.monster.game, &id, "infected.transformed", 1.0)?;
    let spec = infected_species(monster.monster.game, &id)?;
    monster.monster.spec = spec;
    monster.monster.monster.species = spec.species;
    monster.monster.controller.counted_death = false;
    let model = format!("progs/{}.mdl", spec.model);
    let noise = if spec.species == Q1MonsterSpecies::Zombie {
        "zombie/z_idle.wav"
    } else {
        "demon/sight2.wav"
    };
    monster.monster.game.update_entity(&id, |entity| {
        entity.model = model;
        entity.fields.insert(String::from("noise"), noise.to_string());
    })?;
    monster.monster.game.set_health(&id, spec.health)?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.max_health = spec.health;
    })?;
    let pain = monster
        .monster
        .game
        .named
        .pain(&format!("{INFECTED_PREFIX}:monster_pain"))?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.pain = Some(pain);
        entity.aimed_damage = true;
    })?;
    monster.monster.game.set_damageable(&id, true)?;
    if spec.species == Q1MonsterSpecies::Zombie {
        monster.monster.game.update_entity(&id, |entity| {
            entity.spawnflags = 128;
        })?;
    } else {
        set_addon_number(monster.monster.game, &id, "combat_style", 2.0)?;
        monster.monster.monster.pain_finished = monster.monster.game.time + 1.0;
        monster.monster.monster.attack_finished = 0.0;
    }
    monster.monster.game.update_entity(&id, |entity| {
        entity.classname = if spec.species == Q1MonsterSpecies::Zombie {
            String::from("monster_zombie")
        } else {
            String::from("monster_demon1")
        };
    })?;
    infected_retarget(monster)?;
    let owned = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if monster.monster.game.host.walk_move(&owned, 0.0, 0.0) {
        return monster.play(if spec.species == Q1MonsterSpecies::Zombie {
            "zombie_paina1"
        } else {
            "demon1_pain1"
        });
    }
    monster.monster.game.killed_monsters += 1;
    let (total, found) = (
        monster.monster.game.total_monsters,
        monster.monster.game.killed_monsters,
    );
    monster.monster.game.host.emit(Q1Event::MonsterKilled {
        actor: id.clone(),
        total,
        found,
    });
    monster.monster.controller.counted_death = true;
    monster.monster.game.set_health(&id, -100.0)?;
    let zombie = spec.species == Q1MonsterSpecies::Zombie;
    monster.monster.game.sound(
        &id,
        if zombie {
            "zombie/z_gib.wav"
        } else {
            "player/udeath.wav"
        },
        Q1SoundChannel::Voice,
        1.0,
        1.0,
    )?;
    if zombie {
        throw_head(monster.monster.game, &id, "h_zombie", -100.0)?;
        for model in spec.gibs {
            let origin = monster.monster.origin()?;
            throw_gib(monster.monster.game, origin, model, -100.0)?;
        }
        return Ok(());
    }
    for model in spec.gibs {
        let origin = monster.monster.origin()?;
        throw_gib(monster.monster.game, origin, model, -100.0)?;
    }
    throw_head(monster.monster.game, &id, "h_demon", -100.0)?;
    Ok(())
}

/// Retarget a transformed monster away from same-classname rivals
/// (`retarget`).
fn infected_retarget(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let Some(enemy) = monster.monster.monster.enemy.clone() else {
        return Ok(());
    };
    let classname = monster.monster.game.host.classname(&enemy);
    let own = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    if classname != own {
        return Ok(());
    }
    let Some(player) = (monster.monster.game.host.players)()
        .into_iter()
        .find(|player| monster.monster.game.health(player) > 0.0)
    else {
        return Ok(());
    };
    if let Some(rival) = monster
        .monster
        .game
        .entity_ref(&enemy)
        .and_then(|entity| entity.monster.clone())
    {
        if rival
            .enemy
            .as_ref()
            .is_some_and(|rival_enemy| same_actor(rival_enemy, &id))
        {
            monster.monster.game.update_entity(&enemy, |entity| {
                if let Some(monster) = entity.monster.as_mut() {
                    monster.enemy = Some(player.clone());
                }
            })?;
        }
    }
    monster.monster.monster.enemy = Some(player);
    Ok(())
}

/// Resurrect a corpse hell knight (`resurrect`).
fn infected_resurrect(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    set_addon_number(monster.monster.game, &id, "aflag", 0.0)?;
    set_addon_number(monster.monster.game, &id, "infected.risen", 1.0)?;
    let pain = monster
        .monster
        .game
        .named
        .pain(&format!("{INFECTED_PREFIX}:monster_pain"))?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.pain = Some(pain);
    })?;
    monster.ai(MonsterAi::Stand, 0.0)
}

fn resurrect_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    infected_resurrect(&mut monster)?;
    monster.finish()
}

fn infected_kill_rule(monster: &BaseMonster) -> bool {
    if monster.prefix != INFECTED_PREFIX {
        return true;
    }
    // The donor clears the `infected` word on the first counted death
    // and counts once `infected.transformed` is set. Nothing else
    // reads the word, so the rule keys on the transformation
    // directly, which the shared rule signature can observe.
    monster
        .game
        .entity_ref(&monster.id)
        .map(|entity| entity.number("infected.transformed"))
        .unwrap_or(0.0)
        != 0.0
}

fn spawn_infected(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let context = Q1AddonContext::new(Q1AddonProgram::Mg3);
    let classname = game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    let Some(kind) = infected_kind_for_classname(&classname) else {
        return Err(q1_error(format!("Missing MG3 infected variant on {classname}")));
    };
    game.update_entity(id, |entity| {
        entity.fields.insert(
            String::from("infected.kind"),
            match kind {
                InfectedKind::Army => String::from("army"),
                InfectedKind::Knight => String::from("knight"),
                InfectedKind::Enforcer => String::from("enforcer"),
                InfectedKind::Hellknight => String::from("hellknight"),
            },
        );
    })?;
    let spec = infected_species(game, id)?;
    let mut monster = Mg3Monster::spawn_new(game, INFECTED_PREFIX, id, spec)?;
    let native = spec
        .classnames
        .first()
        .ok_or_else(|| q1_error("Infected source class has no native name"))?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.classname = native.to_string();
        })?;
    set_addon_number(monster.monster.game, &monster.monster.id.clone(), "infected", 1.0)?;
    let owned = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    monster.monster.game.host.combat.set_health(&owned, spec.health)?;
    if !spec.stand.starts_with("hknight_corpse") {
        let pain = monster
            .monster
            .game
            .named
            .pain(&format!("{INFECTED_PREFIX}:monster_pain"))?;
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.pain = Some(pain);
            })?;
    }
    let die = monster
        .monster
        .game
        .named
        .die(&format!("{INFECTED_PREFIX}:monster_die"))?;
    let path_end = monster
        .monster
        .game
        .named
        .action(&format!("{INFECTED_PREFIX}:monster_stand"))?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.die = Some(die);
            entity.path_end = Some(path_end);
        })?;
    if spec.species == Q1MonsterSpecies::Knight || spec.species == Q1MonsterSpecies::Hellknight {
        set_addon_number(monster.monster.game, &monster.monster.id.clone(), "allowPathFind", 1.0)?;
        set_addon_number(
            monster.monster.game,
            &monster.monster.id.clone(),
            "combat_style",
            if spec.species == Q1MonsterSpecies::Knight {
                2.0
            } else {
                3.0
            },
        )?;
    }
    let model = format!("progs/{}.mdl", spec.model);
    let size = if spec.species == Q1MonsterSpecies::Army || spec.species == Q1MonsterSpecies::Knight {
        1
    } else {
        2
    };
    init_mg3_monster(&mut monster, &context, &model, 1, size)?;
    monster.finish()
}

/// Register mg3 infected (`registerMg3Infected`). Controllers persist
/// through the base creature store; the extension only carries the
/// clone hook.
pub fn register_mg3_infected(_context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    struct Extension;

    impl Q1StateExtension for Extension {
        fn id(&self) -> &str {
            INFECTED_PREFIX
        }

        fn capture(&self, _game: &Q1EntityServices) -> Vec<u8> {
            encode_checkpoint_value(&crate::value::arr(Vec::new()))
        }

        fn restore(&mut self, _game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
            let saved = decode_checkpoint_value(bytes)?;
            crate::value::SaveReader::new(&saved).list(|_entry| Ok::<(), Q1Error>(()))?;
            Ok(())
        }

        fn clone_state(
            &mut self,
            game: &mut Q1EntityServices,
            source: &ActorId,
            target: &ActorId,
        ) -> Result<(), Q1Error> {
            clone_monster_controller(game, source, target, INFECTED_PREFIX)
        }
    }

    register_mg3_monster_source(
        INFECTED_PREFIX,
        infected_frames(),
        infected_actions(),
        Mg3SourceHooks {
            start: Some(infected_start),
            use_monster: Some(mg3_use_mapped),
            try_attack: Some(infected_try_attack),
            melee_attack: Some(infected_melee),
            pain: Some(infected_pain),
            die: Some(infected_die),
            ..Default::default()
        },
        infected_load_controller,
        crate::q1::base::creatures::store_monster_controller,
    );
    register_mg3_monster_callbacks(game, INFECTED_PREFIX)?;
    register_mg3_monster_startup(game, INFECTED_PREFIX)?;
    game.named.register(
        &format!("{INFECTED_PREFIX}:resurrect"),
        Q1CallbackHandlers {
            action: Some(resurrect_handler as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    register_kill_count_rule(game, INFECTED_PREFIX, infected_kill_rule)?;
    for classname in [
        "monster_army_infected",
        "monster_knight_infected",
        "monster_enforcer_infected",
        "monster_hell_knight_infected",
    ] {
        game.register_spawn(classname, spawn_infected)?;
    }
    game.register_state_extension(Box::new(Extension))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infected_frame_actions_resolve() {
        for (name, frame) in infected_frames() {
            if frame.next == "hknight_run1" {
                continue;
            }
            assert!(
                infected_frames().contains_key(frame.next),
                "dangling {name} -> {}",
                frame.next
            );
            for op in frame.operations {
                if let crate::q1::base::animation::MonsterOperation::Action { name } = op {
                    assert!(infected_actions().contains_key(*name), "missing action {name}");
                }
            }
        }
    }
}
