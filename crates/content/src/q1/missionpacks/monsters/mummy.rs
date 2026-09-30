//! Rogue mummy, including delayed wake and blocked stand-up (`src/content/q1/missionpacks/monsters/mummy.ts`).

use std::sync::Arc;

use qa_core::math::{Bounds, Vec3};

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::projectiles::{throw_gib, throw_head};
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{normalize, vadd, vscale, vsub, Q1MoveType, Q1Solid, Q1SoundChannel, POINT, ZERO};
use crate::q1::Q1Error;

use super::helpers::{number, HULL_BOUNDS};
use super::runtime::MissionMonster;
use super::tables::mummy::FRAMES;
use super::types::{MissionAction, MissionDie, MissionFound, MissionPain, PackMonsterDefinition};

/// Throw one bouncing flesh grenade (`fire`).
fn fire(monster: &mut MissionMonster, offset: Vec3) {
    monster.face();
    let target = match monster.target {
        Some(target) => target,
        None => return,
    };
    let id = monster.entity.actor.id().clone();
    let origin = monster.origin;
    let _ = monster
        .game
        .sound(&id, "zombie/z_shot1.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let basis = monster.game.basis;
    let grenade_origin = vadd(
        vadd(
            vadd(origin, vscale(basis.forward, f64::from(offset.x))),
            vscale(basis.right, f64::from(offset.y)),
        ),
        vscale(basis.up, f64::from(offset.z) - 24.0),
    );
    let grenade = match monster.game.create("mummy_grenade", None, None) {
        Ok(grenade) => grenade,
        Err(_) => return,
    };
    let _ = monster.game.update_entity(&grenade, |entity| {
        entity.owner = Some(id.clone());
        entity.movement = Q1MoveType::Bounce;
        entity.solid = Q1Solid::Bbox;
        entity.model = "progs/zom_gib.mdl".to_string();
    });
    let angles = match monster.game.body(&id) {
        Ok(body) => body.angles,
        Err(_) => return,
    };
    let _ = monster.game.make_vectors(angles);
    let mut velocity = vscale(normalize(vsub(target, grenade_origin)), 600.0);
    velocity.z = 200.0;
    let _ = monster.game.set_body(
        &grenade,
        &BodyPatch {
            origin: Some(grenade_origin),
            velocity: Some(velocity),
            bounds: Some(POINT),
            ..Default::default()
        },
    );
    let _ = monster.game.update_entity(&grenade, |entity| {
        entity.angular_velocity = Vec3 {
            x: 3000.0,
            y: 1000.0,
            z: 2000.0,
        };
    });
    if let Ok(touch) = monster.game.named.touch("rogue:mummyGrenadeTouch") {
        let _ = monster.game.update_entity(&grenade, |entity| {
            entity.touch = Some(touch);
        });
    }
    if let Ok(remove) = monster.game.named.action("SUB_Remove") {
        let _ = monster.game.schedule(&grenade, 2.5, &remove);
    }
    let _ = monster.game.link(&grenade);
}

/// Wake a sleeping mummy (`wake`).
fn wake(monster: &mut MissionMonster) {
    number(monster, "mummy:asleep", 0.0);
    monster.play("mummy_paine12");
}

/// Detonate a mummy grenade on touch (`mummyGrenadeTouch`).
fn mummy_grenade_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let owner = game.entity(id).and_then(|entity| entity.owner.clone());
    if owner.as_ref() == Some(other) {
        return Ok(());
    }
    let damageable = game
        .host
        .combat
        .read(other)
        .is_some_and(|combat| combat.can_take_damage);
    if damageable {
        let amount = 15.0 + game.host.random() * 15.0;
        let _ = game.damage(other, Some(id), owner.as_ref(), amount, &Q1DamageParams::default());
        let _ = game.sound(id, "zombie/z_hit.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
        return game.remove(id);
    }
    let _ = game.sound(id, "zombie/z_miss.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let _ = game.set_body(
        id,
        &BodyPatch {
            velocity: Some(ZERO),
            ..Default::default()
        },
    );
    let remove_touch = game.named.touch("rogue:mummyGrenadeRemove").ok();
    let _ = game.update_entity(id, |entity| {
        entity.angular_velocity = ZERO;
        if let Some(touch) = remove_touch {
            entity.touch = Some(touch);
        }
    });
    Ok(())
}

/// Remove a spent mummy grenade on touch (`mummyGrenadeRemove`).
fn mummy_grenade_remove(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    game.remove(id)
}

/// Rogue mummy definition (`mummyDefinition`).
pub fn mummy_definition() -> PackMonsterDefinition {
    let run_step: MissionAction = Arc::new(|monster| {
        monster.ai(MonsterAi::Run, 2.0);
        monster.controller.in_pain = 0.0;
    });
    let fire_a: MissionAction = Arc::new(|monster| {
        fire(
            monster,
            Vec3 {
                x: -10.0,
                y: -22.0,
                z: 30.0,
            },
        );
    });
    let fire_b: MissionAction = Arc::new(|monster| {
        fire(
            monster,
            Vec3 {
                x: -10.0,
                y: -24.0,
                z: 29.0,
            },
        );
    });
    let fire_c: MissionAction = Arc::new(|monster| {
        fire(
            monster,
            Vec3 {
                x: -12.0,
                y: -19.0,
                z: 29.0,
            },
        );
    });
    let wake_delay: MissionAction = Arc::new(|monster| {
        let wait = monster.entity.next_think - monster.game.time + 5.0;
        monster.delay(wait);
    });
    let stand_up: MissionAction = Arc::new(|monster| {
        let id = monster.entity.actor.id().clone();
        let _ = monster
            .game
            .sound(&id, "zombie/z_idle.wav", Q1SoundChannel::Voice, 2.0, 1.0);
        let _ = monster.game.set_bounds(&id, HULL_BOUNDS);
        monster.entity.solid = Q1Solid::Slidebox;
        monster.flush_entity();
        let actor = monster.entity.actor.clone();
        if !monster.game.host.walk_move(&actor, 0.0, 0.0) {
            monster.next_frame = "mummy_paine11".to_string();
            monster.entity.solid = Q1Solid::None;
            monster.flush_entity();
        }
        let _ = monster.game.link(&id);
    });
    let wake_up: MissionAction = Arc::new(wake);
    let missile: MissionAction = Arc::new(|monster| {
        if monster.entity.number("mummy:asleep") != 0.0 {
            wake(monster);
            return;
        }
        let rolled = monster.game.host.random();
        let frame = if rolled < 0.3 {
            "mummy_atta1"
        } else if rolled < 0.6 {
            "mummy_attb1"
        } else {
            "mummy_attc1"
        };
        monster.play(frame);
    });
    let spawn: MissionAction = Arc::new(|monster| {
        monster.spawn_default();
        let id = monster.entity.actor.id().clone();
        if monster.entity.spawnflags & 4 != 0 {
            monster.entity.max_health = 1000.0;
            monster.flush_entity();
            let actor = monster.entity.actor.clone();
            let _ = monster.game.host.combat.set_health(&actor, 1000.0);
        }
        if monster.entity.spawnflags & 2 != 0 {
            number(monster, "mummy:asleep", 1.0);
            let _ = monster.game.set_bounds(
                &id,
                Bounds {
                    min: HULL_BOUNDS.min,
                    max: Vec3 {
                        x: 16.0,
                        y: 16.0,
                        z: -16.0,
                    },
                },
            );
            monster.entity.solid = Q1Solid::None;
            monster.flush_entity();
        }
    });
    let start: MissionAction = Arc::new(|monster| {
        monster.start_default();
        if monster.entity.number("mummy:asleep") != 0.0 {
            monster.next_frame = if monster.state.path.is_empty() {
                "mummy_sleep".to_string()
            } else {
                "mummy_wake".to_string()
            };
        }
    });
    let found: MissionFound = Arc::new(|monster, target| {
        monster.found_default(target);
        if monster.entity.number("mummy:asleep") != 0.0 {
            monster.next_frame = "mummy_wake".to_string();
        }
    });
    let pain: MissionPain = Arc::new(|monster, _attacker, _damage| {
        if monster.entity.number("mummy:asleep") != 0.0 {
            wake(monster);
            return;
        }
        if monster.state.pain_finished > monster.game.time {
            return;
        }
        let rolled = monster.game.host.random();
        if rolled > 0.24 {
            return;
        }
        let time = monster.game.time;
        monster.state.pain_finished = time + 2.5;
        let frame = if rolled < 0.06 {
            "mummy_paina1"
        } else if rolled < 0.12 {
            "mummy_painb1"
        } else if rolled < 0.18 {
            "mummy_painc1"
        } else {
            "mummy_paind1"
        };
        monster.play(frame);
    });
    let die: MissionDie = Arc::new(|monster, _attacker| {
        let id = monster.entity.actor.id().clone();
        let origin = monster.origin;
        let actor = monster.entity.actor.clone();
        let _ = monster.game.host.combat.set_health(&actor, -35.0);
        let _ = monster
            .game
            .sound(&id, "zombie/z_gib.wav", Q1SoundChannel::Voice, 1.0, 1.0);
        let _ = throw_head(monster.game, &id, "h_zombie", -35.0);
        for model in ["gib1", "gib2", "gib3"] {
            let _ = throw_gib(monster.game, origin, model, -35.0);
        }
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Mummy,
            kill_string: None,
            classnames: &["monster_mummy"],
            model: "mummy",
            head: Some("h_zombie"),
            health: 500.0,
            gib_health: 0.0,
            gibs: &["gib1", "gib2", "gib3"],
            bounds: HULL_BOUNDS,
            stand: "mummy_stand1",
            walk: "mummy_walk1",
            run: "mummy_run1",
            sight: "zombie/z_idle.wav",
            missile: Some("mummy_missile"),
            melee: false,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions: vec![
            ("mummy:mummy_run1", run_step),
            ("mummy:mummy_atta13", fire_a),
            ("mummy:mummy_attb14", fire_b),
            ("mummy:mummy_attc12", fire_c),
            ("mummy:mummy_paine11", wake_delay),
            ("mummy:mummy_paine12", stand_up),
            ("mummy_wake", wake_up),
            ("mummy_missile", missile),
        ],
        callbacks: vec![
            (
                "mummyGrenadeTouch",
                Q1CallbackHandlers {
                    touch: Some(mummy_grenade_touch),
                    ..Default::default()
                },
            ),
            (
                "mummyGrenadeRemove",
                Q1CallbackHandlers {
                    touch: Some(mummy_grenade_remove),
                    ..Default::default()
                },
            ),
        ],
        spawn: Some(spawn),
        start: Some(start),
        pain,
        die,
        melee: None,
        check_attack: None,
        found: Some(found),
        ai: None,
        use_: None,
    }
}

use qa_core::identity::ActorId;

#[cfg(test)]
mod tests {
    use super::mummy_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn mummy_registers_with_grenade_callbacks() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
            .expect("runtime");
        let definition = mummy_definition();
        assert_eq!(definition.spec.classnames, &["monster_mummy"]);
        assert_eq!(definition.spec.model, "mummy");
        assert_eq!(definition.spec.health, 500.0);
        assert_eq!(definition.spec.missile, Some("mummy_missile"));
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 8);
        assert_eq!(definition.callbacks.len(), 2);
        assert!(definition
            .callbacks
            .iter()
            .any(|(name, _)| *name == "mummyGrenadeTouch"));
        assert!(definition
            .callbacks
            .iter()
            .any(|(name, _)| *name == "mummyGrenadeRemove"));
        let wait = definition
            .actions
            .iter()
            .find(|(name, _)| *name == "mummy:mummy_paine11")
            .map(|(_, action)| action.clone())
            .expect("wake delay");
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_mummy", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "mummy");
        wait(&mut monster);
    }
}
