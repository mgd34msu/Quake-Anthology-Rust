//! Rogue wrath, including the overlord-shared missile (`src/content/q1/missionpacks/monsters/wrath.ts`).

use std::rc::Rc;

use qa_core::math::Vec3;

use crate::q1::Q1Error;
use crate::q1::base::projectiles::throw_gib;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{
    Q1Effect, Q1Event, Q1SoundChannel, normalize, vadd, vscale, vsub,
};

use super::helpers::{HULL_BOUNDS, eye, missile};
use super::runtime::MissionMonster;
use super::tables::wrath::FRAMES;
use super::types::{MissionAction, MissionDie, MissionPain, PackMonsterDefinition};

/// Fire one homing wrath missile (`wrathMissile`). Shared with the overlord.
pub fn wrath_missile(monster: &mut MissionMonster, attack: i32) {
    let target = match monster.target {
        Some(target) => target,
        None => return,
    };
    let id = monster.entity.actor.id.clone();
    let origin = monster.origin;
    let aim = vadd(
        target,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 10.0,
        },
    );
    let direction = normalize(vsub(aim, origin));
    let angles = match monster.game.body(&id) {
        Ok(body) => body.angles,
        Err(_) => return,
    };
    let basis = monster.game.make_vectors(angles);
    monster.entity.effects |= 2;
    monster.flush_entity();
    let forward = if attack == 1 || attack == 4 {
        20.0
    } else if attack == 2 {
        18.0
    } else {
        12.0
    };
    let up = if attack == 4 {
        16.0
    } else if attack == 2 {
        10.0
    } else {
        12.0
    };
    let right = if attack == 3 { 20.0 } else { 0.0 };
    let shot_origin = vadd(
        vadd(
            vadd(origin, vscale(basis.forward, forward)),
            vscale(basis.up, up),
        ),
        vscale(basis.right, right),
    );
    let shot = missile(
        monster.game,
        &id,
        "wrath_missile",
        "progs/w_ball.mdl",
        shot_origin,
        vscale(direction, 400.0),
        "rogue:WrathMissileTouch",
        5.0,
    );
    let enemy = monster.enemy.clone();
    let _ = monster.game.update_entity(&shot, |entity| {
        entity.references.insert("enemy".to_string(), enemy);
        entity.angular_velocity = Vec3 {
            x: 300.0,
            y: 300.0,
            z: 300.0,
        };
    });
    if let Ok(home) = monster.game.named.action("rogue:WrathHome") {
        let _ = monster.game.schedule(&shot, 0.1, &home);
    }
    let time = monster.game.time;
    monster.state.attack_finished = time + 2.0;
}

/// Steer a live wrath missile toward its enemy (`WrathHome`).
fn wrath_home(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let enemy = game.entity(id).and_then(|entity| entity.references.get("enemy").cloned().flatten());
    let Some(enemy) = enemy else {
        return game.remove(id);
    };
    if game.health(&enemy) < 1.0 {
        return game.remove(id);
    }
    if let Some(target) = eye(game, &enemy) {
        if let Ok(body) = game.body(id) {
            let speed = if game.options().skill == 3 { 550.0 } else { 400.0 };
            let _ = game.set_body(
                id,
                &BodyPatch {
                    velocity: Some(vscale(normalize(vsub(target, body.origin)), speed)),
                    ..Default::default()
                },
            );
        }
    }
    let home = game.named.action("rogue:WrathHome")?;
    game.schedule(id, 0.1, &home)
}

/// Detonate a wrath missile on touch (`WrathMissileTouch`).
fn wrath_missile_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let classname = game.host.classname(other);
    let owner = game.entity(id).and_then(|entity| entity.owner.clone());
    if owner.as_ref() == Some(other)
        || classname == "monster_wrath"
        || classname == "monster_super_wrath"
    {
        return game.remove(id);
    }
    if classname == "monster_zombie" {
        let _ = game.damage(other, Some(id), Some(id), 110.0, &Q1DamageParams::default());
    }
    let world = game.world.clone();
    game.radius_damage(id, owner.as_ref(), 20.0, world.as_ref(), None, "");
    let _ = game.sound(id, "weapons/r_exp3.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    if let Ok(body) = game.body(id) {
        game.effect_simple(Q1Effect::Explosion, body.origin);
    }
    let _ = game.set_body(
        id,
        &BodyPatch {
            velocity: Some(crate::q1::foundation::types::ZERO),
            ..Default::default()
        },
    );
    let _ = game.update_entity(id, |entity| {
        entity.touch = None;
        entity.model = "progs/s_explod.spr".to_string();
        entity.solid = crate::q1::foundation::types::Q1Solid::None;
        entity.frame = 0;
    });
    let _ = game.link(id);
    let explode = game.named.action("rogue:wrath_explode")?;
    game.schedule(id, 0.1, &explode)
}

/// Advance the wrath explosion sprite (`wrath_explode`).
fn wrath_explode(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let frame = game.entity(id).map(|entity| entity.frame).unwrap_or(0) + 1;
    let _ = game.update_entity(id, |entity| entity.frame = frame);
    if frame > 5 {
        return game.remove(id);
    }
    let again = game.named.action("rogue:wrath_explode")?;
    game.schedule(id, 0.1, &again)
}

/// Rogue wrath callbacks (`wrathCallbacks`).
pub fn wrath_callbacks() -> Vec<(&'static str, Q1CallbackHandlers)> {
    vec![
        (
            "WrathHome",
            Q1CallbackHandlers {
                action: Some(wrath_home),
                ..Default::default()
            },
        ),
        (
            "WrathMissileTouch",
            Q1CallbackHandlers {
                touch: Some(wrath_missile_touch),
                ..Default::default()
            },
        ),
        (
            "wrath_explode",
            Q1CallbackHandlers {
                action: Some(wrath_explode),
                ..Default::default()
            },
        ),
    ]
}

/// Rogue wrath definition (`wrathDefinition`).
pub fn wrath_definition() -> PackMonsterDefinition {
    let attack: MissionAction = Rc::new(|monster| {
        let rolled = monster.game.host.random();
        let frame = if rolled < 0.25 {
            "wrath_at_a01"
        } else if rolled < 0.65 {
            "wrath_at_b01"
        } else {
            "wrath_at_c01"
        };
        let _ = monster.play(frame);
        let id = monster.entity.actor.id.clone();
        let _ = monster.game.sound(&id, "wrath/watt.wav", Q1SoundChannel::Voice, 1.0, 1.0);
    });
    let missile1: MissionAction = Rc::new(|monster| wrath_missile(monster, 1));
    let missile2: MissionAction = Rc::new(|monster| wrath_missile(monster, 2));
    let missile3: MissionAction = Rc::new(|monster| wrath_missile(monster, 3));
    let death_burst: MissionAction = Rc::new(|monster| {
        let id = monster.entity.actor.id.clone();
        let origin = monster.origin;
        let health = monster.game.health(&id);
        for model in ["wrthgib1", "wrthgib2", "wrthgib3"] {
            let _ = throw_gib(monster.game, origin, model, health);
        }
        let world = monster.game.world.clone();
        monster.game.radius_damage(&id, Some(&id), 80.0, world.as_ref(), None, "");
        let _ = monster.game.set_body(
            &id,
            &BodyPatch {
                origin: Some(vadd(
                    origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 24.0,
                    },
                )),
                ..Default::default()
            },
        );
        monster.game.host.emit(Q1Event::ColoredExplosion {
            origin,
            color_start: 0,
            color_length: 4,
        });
        let _ = monster.game.remove(&id);
    });
    let spawn: MissionAction = Rc::new(|monster| {
        monster.entity.fields.insert("yaw_speed".to_string(), "35".to_string());
        monster.flush_entity();
        let _ = monster.spawn_default();
    });
    let pain: MissionPain = Rc::new(|monster, _attacker, _damage| {
        if monster.state.pain_finished > monster.game.time {
            return;
        }
        let rolled = monster.game.host.random();
        if rolled > 0.1 {
            let time = monster.game.time;
            monster.state.pain_finished = time + 0.5;
            return;
        }
        let frame = if rolled < 0.07 {
            "wrath_pn_a01"
        } else {
            "wrath_pn_b01"
        };
        let _ = monster.play(frame);
        let time = monster.game.time;
        monster.state.pain_finished = time + 3.0;
        let id = monster.entity.actor.id.clone();
        let _ = monster.game.sound(&id, "wrath/wpain.wav", Q1SoundChannel::Voice, 1.0, 1.0);
    });
    let die: MissionDie = Rc::new(|monster, _attacker| {
        let _ = monster.play("wrath_die02");
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Wrath,
            kill_string: None,
            classnames: &["monster_wrath"],
            model: "wrath",
            head: None,
            health: 400.0,
            gib_health: f64::NEG_INFINITY,
            gibs: &["wrthgib1", "wrthgib2", "wrthgib3"],
            bounds: HULL_BOUNDS,
            stand: "wrath_stand1",
            walk: "wrath_walk01",
            run: "wrath_run01",
            sight: "wrath/wsee.wav",
            missile: Some("wrath_attack"),
            melee: false,
            movement: MonsterMovement::Fly,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions: vec![
            ("wrath_attack", attack),
            ("WrathMissile(1)", missile1),
            ("WrathMissile(2)", missile2),
            ("WrathMissile(3)", missile3),
            ("wrath:wrath_die15", death_burst),
        ],
        callbacks: wrath_callbacks(),
        spawn: Some(spawn),
        start: None,
        pain,
        die,
        melee: None,
        check_attack: None,
        found: None,
        ai: None,
        use_: None,
    }
}

use qa_core::identity::ActorId;

#[cfg(test)]
mod tests {
    use super::wrath_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{Q1MissionPack, test_game};

    #[test]
    fn wrath_registers_with_missile_callbacks() {
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
                .expect("runtime");
        let definition = wrath_definition();
        assert_eq!(definition.spec.classnames, &["monster_wrath"]);
        assert_eq!(definition.spec.model, "wrath");
        assert_eq!(definition.spec.health, 400.0);
        assert_eq!(definition.spec.missile, Some("wrath_attack"));
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 5);
        assert_eq!(definition.callbacks.len(), 3);
        assert!(definition.callbacks.iter().any(|(name, _)| *name == "WrathHome"));
        assert!(definition.callbacks.iter().any(|(name, _)| *name == "WrathMissileTouch"));
        assert!(definition.callbacks.iter().any(|(name, _)| *name == "wrath_explode"));
        let fire = definition
            .actions
            .iter()
            .find(|(name, _)| *name == "WrathMissile(1)")
            .map(|(_, action)| action.clone())
            .expect("missile");
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_wrath", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "wrath");
        fire(&mut monster);
    }
}
