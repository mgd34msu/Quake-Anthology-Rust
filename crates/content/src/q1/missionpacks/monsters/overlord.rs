//! Rogue overlord (`src/content/q1/missionpacks/monsters/overlord.ts`).

use std::sync::Arc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::spawns::{spawn_teledeath, spawn_teleport_fog};
use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, Q1Event, Q1MoveType, Q1SoundChannel, POINT, ZERO,
};
use crate::q1::Q1Error;

use super::helpers::{eye, radius_actors, HULL_BOUNDS};
use super::runtime::MissionMonster;
use super::tables::s_wrath::FRAMES;
use super::types::{MissionDie, MissionPain, PackMonsterDefinition};
use super::wrath::wrath_missile;

/// Whether a destination has no blocking occupants (`isSpawnPointEmpty`).
pub fn is_spawn_point_empty(game: &mut Q1EntityServices, point: &ActorId) -> bool {
    let Ok(body) = game.body(point) else { return false };
    for actor in radius_actors(game, body.origin, 64.0) {
        if actor == *point {
            continue;
        }
        let other = game.entity(&actor).cloned();
        if other.as_ref().map(|entity| entity.movement_flags).unwrap_or(0) & 32 != 0
            || game.host.classname(&actor) == "player"
            || other.as_ref().and_then(|entity| entity.think.clone()).is_some()
        {
            return false;
        }
    }
    true
}

/// Pick an overlord teleport destination (`overlordDestination`).
pub fn overlord_destination(game: &mut Q1EntityServices) -> Option<ActorId> {
    let player = (game.host.players)().first().cloned().or_else(|| game.world.clone());
    let body = player.as_ref().and_then(|player| game.host.bodies.read(player));
    let basis = game.make_vectors(body.as_ref().map(|body| body.angles).unwrap_or(ZERO));
    let origin = body.map(|body| body.origin).unwrap_or(ZERO);
    let mut best = None;
    let mut furthest = None;
    let mut distance = 0.0;
    for id in game.entity_ids() {
        let entity = game.entity(&id).cloned();
        if entity.as_ref().map(|entity| entity.classname.as_str()) != Some("info_overlord_destination")
            || !is_spawn_point_empty(game, &id)
        {
            continue;
        }
        let at = game.body(&id).map(|body| body.origin).unwrap_or(ZERO);
        let delta = vsub(at, origin);
        let current = f64::from(length(delta));
        if f64::from(dot(normalize(delta), basis.forward)) > 0.6 && current > 150.0 {
            best = Some(id.clone());
        }
        if current > distance {
            furthest = Some(id.clone());
            distance = current;
        }
    }
    best.or(furthest)
}

/// Register the overlord destination spawn (`registerOverlordDestination`).
pub fn register_overlord_destination(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.register_spawn("info_overlord_destination", |game, id| {
        let body = game.body(id)?;
        game.update_entity(id, |entity| {
            entity.mangle = body.angles;
            entity.model = String::new();
        })?;
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(ZERO),
                origin: Some(vadd(
                    body.origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 27.0,
                    },
                )),
                ..Default::default()
            },
        )
    })
}

/// Teleport to a destination (`teleport`).
fn teleport(monster: &mut MissionMonster) {
    if monster.entity.spawnflags & 2 == 0 || monster.game.host.random() > 0.75 {
        return;
    }
    let id = monster.entity.actor.id().clone();
    let Some(destination) = overlord_destination(monster.game) else {
        monster.refresh();
        return;
    };
    let _ = spawn_teleport_fog(monster.game, monster.origin);
    let basis = monster
        .game
        .body(&id)
        .ok()
        .map(|body| monster.game.make_vectors(body.angles));
    let origin = monster.game.body(&destination).map(|body| body.origin).unwrap_or(ZERO);
    if let Some(basis) = basis {
        let _ = spawn_teleport_fog(monster.game, vadd(origin, vscale(basis.forward, 32.0)));
    }
    let _ = spawn_teledeath(monster.game, origin, &id);
    let _ = monster.game.set_origin(&id, origin);
    monster.entity.movement_flags &= !512;
    monster.flush_entity();
    monster.refresh();
}

/// Toss one gib (`toss`).
fn toss(monster: &mut MissionMonster, model: &str) {
    let id = monster.entity.actor.id().clone();
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let basis = monster.game.make_vectors(body.angles);
    let velocity = vadd(
        vadd(
            vadd(vscale(basis.forward, 250.0), vscale(basis.up, 300.0)),
            vscale(basis.up, monster.game.host.random() * 100.0 - 50.0),
        ),
        vscale(basis.right, monster.game.host.random() * 200.0 - 100.0),
    );
    let Ok(gib) = monster.game.create("gib", None, None) else {
        monster.refresh();
        return;
    };
    let time = monster.game.time;
    let _ = monster.game.update_entity(&gib, |entity| {
        entity.model = format!("progs/{model}.mdl");
        entity.movement = Q1MoveType::Bounce;
        entity.fields.insert("ltime".to_string(), time.to_string());
    });
    let origin = monster.origin;
    let _ = monster.game.set_body(
        &gib,
        &BodyPatch {
            origin: Some(origin),
            bounds: Some(POINT),
            ..Default::default()
        },
    );
    if let Ok(remove) = monster.game.named.action("SUB_Remove") {
        let lifetime = 10.0 + monster.game.host.random() * 10.0;
        let _ = monster.game.schedule(&gib, lifetime, &remove);
    }
    let _ = monster.game.set_body(
        &gib,
        &BodyPatch {
            velocity: Some(velocity),
            ..Default::default()
        },
    );
    let _ = monster.game.link(&gib);
    monster.refresh();
}

/// Explosion flash plus gibs (`burst`).
fn burst(monster: &mut MissionMonster, models: &[&str]) {
    monster.game.host.emit(Q1Event::ColoredExplosion {
        origin: monster.origin,
        color_start: 0,
        color_length: 4,
    });
    for model in models {
        toss(monster, model);
    }
}

/// Smash the enemy (`smash`).
fn smash(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    let Some(enemy) = monster.enemy.clone() else {
        monster.refresh();
        return;
    };
    if !monster.game.can_damage(&enemy, &id) {
        monster.refresh();
        return;
    }
    monster.ai(MonsterAi::Charge, 10.0);
    let Some(target) = monster.target else {
        monster.refresh();
        return;
    };
    if monster.distance() > 100.0 {
        monster.refresh();
        return;
    }
    let damage = 20.0 + monster.game.host.random() * 10.0;
    let _ = monster
        .game
        .sound(&id, "s_wrath/smash.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    monster
        .game
        .damage(&enemy, Some(&id), Some(&id), damage, &Q1DamageParams::default());
    let face = eye(monster.game, &enemy).unwrap_or(target);
    let direction = normalize(vsub(face, monster.origin));
    monster.game.host.emit(Q1Event::Particles {
        origin: vsub(target, vscale(direction, 30.0)),
        direction: vscale(direction, -100.0),
        color: 73,
        count: damage as i32,
    });
    monster.refresh();
}

/// Choose a melee sequence (`melee`).
fn melee(monster: &mut MissionMonster) {
    let rolled = monster.game.host.random();
    monster.play(if rolled < 0.33 {
        "overlord_at_a01"
    } else if rolled < 0.66 {
        "overlord_at_b01"
    } else {
        "overlord_at_c01"
    });
}

/// Rogue overlord definition (`overlordDefinition`).
pub fn overlord_definition() -> PackMonsterDefinition {
    let pain: MissionPain = Arc::new(|monster, _attacker, _damage| {
        if monster.state.pain_finished > monster.game.time {
            return;
        }
        let rolled = monster.game.host.random();
        if rolled > 0.2 {
            return;
        }
        monster.play(if rolled < 0.15 {
            "overlord_pn_a01"
        } else {
            "overlord_pn_b01"
        });
        let time = monster.game.time;
        monster.state.pain_finished = time + 2.0;
        let id = monster.entity.actor.id().clone();
        let _ = monster.game.sound_simple(&id, "wrath/wpain.wav");
    });
    let die: MissionDie = Arc::new(|monster, _attacker| {
        monster.play("overlord_die02");
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::SuperWrath,
            kill_string: None,
            classnames: &["monster_super_wrath"],
            model: "s_wrath",
            head: None,
            health: 1000.0,
            gib_health: f64::NEG_INFINITY,
            gibs: &[],
            bounds: HULL_BOUNDS,
            stand: "overlord_stand1",
            walk: "overlord_walk01",
            run: "overlord_run01",
            sight: "",
            missile: Some("overlord_missile"),
            melee: true,
            movement: MonsterMovement::Fly,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions: vec![
            ("overlord_smash", Arc::new(smash)),
            ("overlord_melee", Arc::new(melee)),
            ("overlord_teleport", Arc::new(teleport)),
            (
                "overlord_missile",
                Arc::new(|monster| {
                    let _ = monster.game.host.random();
                    monster.play("overlord_msl_a01");
                }),
            ),
            ("WrathMissile(4)", Arc::new(|monster| wrath_missile(monster, 4))),
            ("s_wrath:overlord_die01", Arc::new(|monster| monster.delay(0.05))),
            (
                "s_wrath:overlord_die02",
                Arc::new(|monster| {
                    monster.entity.movement_flags |= 1;
                    monster.flush_entity();
                    monster.delay(0.05);
                }),
            ),
            (
                "s_wrath:overlord_die17",
                Arc::new(|monster| {
                    monster.entity.model = String::new();
                    monster.flush_entity();
                    burst(monster, &["s_wrtgb2", "s_wrtgb3", "wrthgib1", "wrthgib2", "wrthgib3"]);
                    monster.delay(0.1);
                }),
            ),
            (
                "s_wrath:overlord_die18",
                Arc::new(|monster| {
                    burst(monster, &["gib1", "gib2", "gib3", "gib1", "gib2", "gib3"]);
                    monster.delay(0.1);
                }),
            ),
            (
                "s_wrath:overlord_die19",
                Arc::new(|monster| {
                    burst(monster, &["gib1", "gib2", "gib3", "gib1", "gib2", "gib3"]);
                    let id = monster.entity.actor.id().clone();
                    let _ = monster.game.remove(&id);
                }),
            ),
        ],
        callbacks: Vec::new(),
        spawn: None,
        start: None,
        pain,
        die,
        melee: Some(Arc::new(melee)),
        check_attack: None,
        found: None,
        ai: None,
        use_: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{overlord_definition, overlord_destination, register_overlord_destination};
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn overlord_registers_and_picks_destinations() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
            .expect("runtime");
        let definition = overlord_definition();
        assert_eq!(definition.spec.classnames, &["monster_super_wrath"]);
        assert_eq!(definition.spec.model, "s_wrath");
        assert_eq!(definition.spec.health, 1000.0);
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 10);
        runtime.register(&mut game, definition).expect("register");
        register_overlord_destination(&mut game).expect("destinations");
        let id = game.create("monster_super_wrath", None, None).expect("create");
        let monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "s_wrath");
        assert!(overlord_destination(&mut game).is_none());
    }
}
