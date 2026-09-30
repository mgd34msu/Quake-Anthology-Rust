//! Rogue floating sword (`src/content/q1/missionpacks/monsters/sword.ts`).

use std::rc::Rc;

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::types::Q1SoundChannel;

use super::helpers::HUMAN_BOUNDS;
use super::runtime::MissionMonster;
use super::tables::invis_sw::FRAMES;
use super::types::{MissionAction, MissionDie, MissionFound, MissionPain, PackMonsterDefinition};

/// Rogue floating-sword definition (`swordDefinition`).
pub fn sword_definition() -> PackMonsterDefinition {
    let sword_pause: MissionAction = Rc::new(|monster| {
        let delay = monster.entity.delay;
        monster.entity.delay = 0.0;
        monster.next_frame = "sword_run1".to_string();
        monster.entity.fields.insert("sword:awakened".to_string(), "1".to_string());
        monster.flush_entity();
        let _ = monster.delay(delay);
    });
    let sword_run: MissionAction = Rc::new(|monster| {
        monster.entity.effects = 8;
        monster.flush_entity();
        let _ = monster.ai(MonsterAi::Run, 14.0);
    });
    let sword_attack: MissionAction = Rc::new(|monster| {
        let id = monster.entity.actor.id.clone();
        let _ = monster.game.sound(&id, "knight/sword1.wav", Q1SoundChannel::Auto, 1.0, 1.0);
        let _ = monster.ai(MonsterAi::Charge, 14.0);
    });
    let sword_die_sound: MissionAction = Rc::new(|monster| {
        let id = monster.entity.actor.id.clone();
        let _ = monster.game.sound(&id, "player/axhit2.wav", Q1SoundChannel::Weapon, 1.0, 0.5);
    });
    let spawn: MissionAction = Rc::new(|monster| {
        if monster.entity.delay == 0.0 {
            monster.entity.delay = 10.0;
        }
        monster.flush_entity();
        let _ = monster.spawn_default();
    });
    let found: MissionFound = Rc::new(|monster, target| {
        let _ = monster.found_default(target);
        if monster.entity.number("sword:awakened") == 0.0 {
            monster.next_frame = "sword_pause".to_string();
        }
    });
    let pain: MissionPain = Rc::new(|monster, _attacker, _damage| {
        if monster.entity.number("sword:pain-disabled") != 0.0 {
            return;
        }
        monster.entity.fields.insert("sword:pain-disabled".to_string(), "1".to_string());
        monster.entity.fields.insert("sword:awakened".to_string(), "1".to_string());
        monster.entity.delay = 0.0;
        monster.next_frame = "sword_run1".to_string();
        monster.flush_entity();
        let _ = monster.delay(0.1);
    });
    let melee: MissionAction = Rc::new(|monster| {
        let _ = monster.play("sword_atk1");
    });
    let die: MissionDie = Rc::new(|monster, _attacker| {
        monster.entity.effects = 0;
        monster.flush_entity();
        let frame = if monster.game.host.random() < 0.5 {
            "sword_die1"
        } else {
            "sword_dieb1"
        };
        let _ = monster.play(frame);
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Sword,
            kill_string: None,
            classnames: &["monster_sword"],
            model: "sword",
            head: None,
            health: 150.0,
            gib_health: f64::NEG_INFINITY,
            gibs: &[],
            bounds: HUMAN_BOUNDS,
            stand: "sword_stand1",
            walk: "sword_stand1",
            run: "sword_run1",
            sight: "knight/ksight.wav",
            missile: None,
            melee: true,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions: vec![
            ("sword_pause", sword_pause),
            ("invis_sw:sword_run1", sword_run),
            ("invis_sw:sword_atk1", sword_attack),
            ("invis_sw:sword_die7", sword_die_sound),
        ],
        callbacks: Vec::new(),
        spawn: Some(spawn),
        start: None,
        pain,
        die,
        melee: Some(melee),
        check_attack: None,
        found: Some(found),
        ai: None,
        use_: None,
    }
}

#[cfg(test)]
mod tests {
    use super::sword_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{Q1MissionPack, test_game};

    #[test]
    fn sword_registers_and_pauses() {
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
                .expect("runtime");
        let definition = sword_definition();
        assert_eq!(definition.spec.classnames, &["monster_sword"]);
        assert_eq!(definition.spec.model, "sword");
        assert_eq!(definition.spec.health, 150.0);
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 4);
        let pause = definition
            .actions
            .iter()
            .find(|(name, _)| *name == "sword_pause")
            .map(|(_, action)| action.clone())
            .expect("pause");
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_sword", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "sword");
        pause(&mut monster);
        assert_eq!(
            monster.entity.fields.get("sword:awakened").map(String::as_str),
            Some("1")
        );
        assert_eq!(monster.next_frame, "sword_run1");
    }
}
