//! Rogue electric eel (`src/content/q1/missionpacks/monsters/eel.ts`).

use std::sync::Arc;

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1DamageParams;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{Q1SoundChannel, Q1TraceRequest, POINT};

use super::helpers::{drop_to_floor, eel_zap, gib, number, HULL_BOUNDS};
use super::runtime::MissionMonster;
use super::tables::eel::FRAMES;
use super::types::{MissionAction, MissionDie, MissionPain, PackMonsterDefinition};

/// Out-of-water damage and pitch wobble (`pitch`).
fn pitch(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    if monster.game.host.contents(monster.origin) != Q1Contents::Water {
        drop_to_floor(monster);
        let world = monster.game.world.clone();
        let inflictor = world.clone().unwrap_or_else(|| id.clone());
        let _ = monster
            .game
            .damage(&id, Some(&inflictor), world.as_ref(), 6.0, &Q1DamageParams::default());
        return;
    }
    if monster.game.time < monster.entity.delay {
        return;
    }
    let mut weapon = monster.entity.number("weapon");
    if weapon > 10.0 {
        weapon = -10.0;
    }
    if let Ok(body) = monster.game.body(&id) {
        if weapon != 0.0 {
            let mut angles = body.angles;
            angles.x += if weapon < 0.0 { -1.5 } else { 1.5 };
            let _ = monster.game.set_body(
                &id,
                &BodyPatch {
                    angles: Some(angles),
                    ..Default::default()
                },
            );
        }
    }
    number(monster, "weapon", weapon + 1.0);
}

/// Charging swim step with pitch wobble (`charge`).
fn charge(monster: &mut MissionMonster) {
    monster.ai(MonsterAi::Charge, 8.0);
    pitch(monster);
}

/// Rogue electric-eel definition (`eelDefinition`).
pub fn eel_definition() -> PackMonsterDefinition {
    let pitch_change: MissionAction = Arc::new(pitch);
    let attack_warmup: MissionAction = Arc::new(|monster| {
        monster.entity.effects = 8;
        monster.entity.skin = 1;
        monster.flush_entity();
        charge(monster);
        let id = monster.entity.actor.id().clone();
        let _ = monster
            .game
            .sound(&id, "eel/eatt1.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    });
    let attack_skin2: MissionAction = Arc::new(|monster| {
        monster.entity.skin = 2;
        monster.flush_entity();
        charge(monster);
    });
    let attack_skin3: MissionAction = Arc::new(|monster| {
        monster.entity.skin = 3;
        monster.flush_entity();
        charge(monster);
    });
    let attack_skin4: MissionAction = Arc::new(|monster| {
        monster.entity.effects = 4;
        monster.entity.skin = 4;
        monster.flush_entity();
        charge(monster);
    });
    let attack_zap: MissionAction = Arc::new(|monster| {
        monster.entity.skin = 5;
        monster.flush_entity();
        let id = monster.entity.actor.id().clone();
        let origin = monster.origin;
        let enemy = monster.enemy.clone();
        let target = monster.target;
        if let (Some(enemy), Some(target)) = (enemy, target) {
            let trace = monster.game.host.trace(&Q1TraceRequest {
                start: origin,
                end: target,
                bounds: POINT,
                ignore: Some(id.clone()),
                monsters: true,
                missile: false,
            });
            if trace.actor == Some(enemy) && !(trace.in_open && trace.in_water) {
                eel_zap(monster);
            }
        }
        monster.entity.skin = 0;
        monster.entity.effects = 0;
        monster.flush_entity();
    });
    let death_start: MissionAction = Arc::new(|monster| {
        monster.entity.skin = 0;
        monster.entity.effects = 0;
        monster.flush_entity();
        let id = monster.entity.actor.id().clone();
        let _ = monster
            .game
            .sound(&id, "eel/edie3r.wav", Q1SoundChannel::Voice, 1.0, 1.0);
    });
    let death_sink: MissionAction = Arc::new(|monster| {
        monster.entity.movement_flags -= 2;
        monster.flush_entity();
    });
    let drop: MissionAction = Arc::new(|monster| {
        drop_to_floor(monster);
    });
    let pain_sound: MissionAction = Arc::new(|monster| {
        if monster.state.pain_finished > monster.game.time {
            return;
        }
        let time = monster.game.time;
        monster.state.pain_finished = time + 1.0;
        let id = monster.entity.actor.id().clone();
        let _ = monster
            .game
            .sound(&id, "eel/epain3.wav", Q1SoundChannel::Voice, 1.0, 1.0);
        monster.entity.skin = 0;
        monster.flush_entity();
    });
    let spawn: MissionAction = Arc::new(|monster| {
        let time = monster.game.time;
        let jitter = monster.game.host.random() * 6.0;
        monster.entity.delay = time + jitter;
        number(monster, "weapon", 0.0);
        monster.spawn_default();
    });
    let pain: MissionPain = Arc::new(|monster, _attacker, _damage| {
        monster.play("eel_pain1");
    });
    let melee: MissionAction = Arc::new(|monster| {
        monster.play("eel_attack1");
    });
    let die: MissionDie = Arc::new(|monster, _attacker| {
        monster.entity.movement_flags += 2;
        let id = monster.entity.actor.id().clone();
        monster.flush_entity();
        let _ = monster.game.set_bounds(&id, POINT);
        if monster.game.health(&id) < -12.0 {
            monster.entity.skin = 0;
            monster.entity.effects = 0;
            monster.flush_entity();
            gib(monster, "eelgib", &["gib1", "gib1", "gib1"], Some(""));
        } else {
            monster.play("eel_death1");
        }
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Eel,
            kill_string: None,
            classnames: &["monster_eel"],
            model: "eel2",
            head: Some("eelgib"),
            health: 60.0,
            gib_health: -12.0,
            gibs: &["gib1", "gib1", "gib1"],
            bounds: HULL_BOUNDS,
            stand: "eel_stand1",
            walk: "eel_walk1",
            run: "eel_run1",
            sight: "eel/eelc5.wav",
            missile: None,
            melee: true,
            movement: MonsterMovement::Swim,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions: vec![
            ("eel_pitch_change", pitch_change),
            ("eel:eel_attack8", attack_warmup),
            ("eel:eel_attack9", attack_skin2),
            ("eel:eel_attack10", attack_skin3),
            ("eel:eel_attack11", attack_skin4),
            ("eel:eel_attack12", attack_zap),
            ("eel:eel_death1", death_start),
            ("eel:eel_death11", death_sink),
            ("droptofloor", drop),
            ("eel:eel_pain1", pain_sound),
        ],
        callbacks: Vec::new(),
        spawn: Some(spawn),
        start: None,
        pain,
        die,
        melee: Some(melee),
        check_attack: None,
        found: None,
        ai: None,
        use_: None,
    }
}

#[cfg(test)]
mod tests {
    use super::eel_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn eel_registers_and_sounds_pain() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
            .expect("runtime");
        let definition = eel_definition();
        assert_eq!(definition.spec.classnames, &["monster_eel"]);
        assert_eq!(definition.spec.model, "eel2");
        assert_eq!(definition.spec.health, 60.0);
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 10);
        let pain = definition
            .actions
            .iter()
            .find(|(name, _)| *name == "eel:eel_pain1")
            .map(|(_, action)| action.clone())
            .expect("pain");
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_eel", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "eel2");
        pain(&mut monster);
        assert_eq!(monster.entity.skin, 0);
        assert!(monster.state.pain_finished > 0.0);
    }
}
