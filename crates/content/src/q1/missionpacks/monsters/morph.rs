//! Rogue morph (`src/content/q1/missionpacks/monsters/morph.ts`).

use std::sync::Arc;

use qa_core::math::Vec3;

use crate::q1::base::projectiles::launch_laser;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1DamageParams;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel,
};

use super::helpers::{drop_to_floor, eye, HULL_BOUNDS};
use super::overlord::{is_spawn_point_empty, overlord_destination};
use super::runtime::MissionMonster;
use super::tables::morph::FRAMES;
use super::types::{MissionAction, MissionDie, MissionPain, MissionUse, PackMonsterDefinition};

/// Configure a morph body (`setup`).
fn setup(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let owner = monster.entity.owner.clone();
    monster.entity.solid = Q1Solid::Slidebox;
    monster.entity.movement = Q1MoveType::Step;
    monster.entity.movement_flags |= 32;
    if let Ok(body) = monster.game.body(&id) {
        monster.entity.ideal_yaw = f64::from(body.angles.y);
    }
    if monster.entity.yaw_speed == 0.0 {
        monster.entity.yaw_speed = 20.0;
    }
    monster
        .entity
        .fields
        .insert("view_ofs".to_string(), "0 0 25".to_string());
    monster.entity.frame = FRAMES
        .iter()
        .find(|(name, _)| *name == "morph_wake1")
        .map(|(_, frame)| frame.frame)
        .unwrap_or(0);
    monster.flush_entity();
    let _ = monster.game.set_bounds(&id, HULL_BOUNDS);
    let _ = monster.game.set_damageable(&id, false);
    if let Ok(pain) = monster.game.named.pain("rogue:monster_pain") {
        monster.entity.pain = Some(pain);
    }
    if let Ok(die) = monster.game.named.die("rogue:monster_die") {
        monster.entity.die = Some(die);
    }
    monster.entity.max_health = if owner.is_none() { 2000.0 } else { 200.0 };
    monster.flush_entity();
    let owned = monster.entity.actor.clone();
    let _ = monster.game.host.combat.set_health(&owned, monster.entity.max_health);
    if let Some(owner) = owner {
        let spawnflags = monster.game.entity(&owner).map(|entity| entity.spawnflags);
        monster.entity.effects = 0;
        if let Some(spawnflags) = spawnflags {
            monster.entity.spawnflags = spawnflags;
        }
    } else {
        monster.entity.effects |= 8;
    }
    monster.entity.skin = 2;
    monster.flush_entity();
    let _ = monster.game.link(&id);
    monster.refresh();
}

/// Wake when the spawn point is clear (`wake`).
fn wake(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    if is_spawn_point_empty(monster.game, &id) {
        setup(monster);
        monster.next_frame = "morph_wake1".to_string();
    } else {
        monster.next_frame = "morph_wake".to_string();
    }
    monster.delay(0.1);
}

/// Stab or fire lasers (`stab`).
fn stab(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    let Some(enemy) = monster.enemy.clone() else {
        monster.refresh();
        return;
    };
    if !monster.game.can_damage(&enemy, &id) {
        monster.refresh();
        return;
    }
    monster.face();
    let Some(target) = eye(monster.game, &enemy) else {
        monster.refresh();
        return;
    };
    let delta = vsub(target, monster.origin);
    let distance = f64::from(length(delta));
    let direction = normalize(delta);
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let basis = monster.game.make_vectors(body.angles);
    if distance <= 90.0 {
        let _ = monster
            .game
            .sound(&id, "enforcer/enfstop.wav", Q1SoundChannel::Weapon, 3.0, 1.0);
        let damage = monster.game.host.random() * 10.0 + 20.0;
        monster
            .game
            .damage(&enemy, Some(&id), Some(&id), damage, &Q1DamageParams::default());
        monster.game.host.emit(Q1Event::Particles {
            origin: monster.target.unwrap_or(target),
            direction: vscale(basis.forward, 150.0),
            color: 73,
            count: 14,
        });
    } else {
        monster.entity.effects |= 2;
        monster.flush_entity();
        let origin = vadd(
            vadd(
                vadd(monster.origin, vscale(basis.forward, 80.0)),
                vscale(basis.right, 4.0),
            ),
            Vec3 { x: 0.0, y: 0.0, z: 4.0 },
        );
        let right = monster.game.basis.right;
        let spread = if distance != 0.0 { 0.04 } else { 0.1 };
        let _ = launch_laser(monster.game, Some(&id), origin, direction);
        let _ = launch_laser(monster.game, Some(&id), origin, vadd(direction, vscale(right, spread)));
        let _ = launch_laser(monster.game, Some(&id), origin, vsub(direction, vscale(right, spread)));
    }
    monster.refresh();
}

/// Knock the enemy back (`smack`).
fn smack(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    let Some(enemy) = monster.enemy.clone() else {
        monster.refresh();
        return;
    };
    if !monster.game.can_damage(&enemy, &id) {
        monster.refresh();
        return;
    }
    monster.face();
    if monster.distance() > 100.0 {
        monster.refresh();
        return;
    }
    let damage = monster.game.host.random() * 10.0 + 10.0;
    monster
        .game
        .damage(&enemy, Some(&id), Some(&id), damage, &Q1DamageParams::default());
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let basis = monster.game.make_vectors(body.angles);
    let owned = monster.game.host.actors.resolve_owned(&enemy);
    let target_body = monster.game.host.bodies.read(&enemy);
    if let (Some(owned), Some(target_body)) = (owned, target_body) {
        let mut moved = target_body.clone();
        moved.velocity = vadd(
            vscale(basis.forward, 100.0),
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 100.0,
            },
        );
        let _ = monster.game.host.bodies.write(&owned, &moved);
    }
    monster.refresh();
}

/// Fire a laser spread (`fire`).
fn fire(monster: &mut MissionMonster) {
    monster.face();
    monster.entity.effects |= 2;
    monster.flush_entity();
    let id = monster.entity.actor.id().clone();
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let basis = monster.game.make_vectors(body.angles);
    let Some(target) = monster.target else {
        monster.refresh();
        return;
    };
    let origin = vadd(
        vadd(
            vadd(monster.origin, vscale(basis.forward, 30.0)),
            vscale(basis.right, 8.5),
        ),
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let direction = normalize(vsub(target, monster.origin));
    let _ = launch_laser(monster.game, Some(&id), origin, direction);
    let spread = if monster.distance() > 400.0 { 0.04 } else { 0.1 };
    let right = monster.game.basis.right;
    let _ = launch_laser(monster.game, Some(&id), origin, vadd(direction, vscale(right, spread)));
    let _ = launch_laser(monster.game, Some(&id), origin, vsub(direction, vscale(right, spread)));
    monster.refresh();
}

/// Summon a child morph (`child`).
fn child(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    if monster.entity.owner.is_some()
        || monster.entity.number("childrenSpawned") > 1.0 + monster.game.options().skill as f64
    {
        monster.refresh();
        return;
    }
    let Some(destination) = overlord_destination(monster.game) else {
        monster.refresh();
        return;
    };
    let mangle = monster
        .game
        .entity(&destination)
        .map(|entity| entity.mangle)
        .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
    let destination_origin =
        monster
            .game
            .body(&destination)
            .map(|body| body.origin)
            .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
    let model = monster.entity.model.clone();
    let Ok(next) = monster.game.create("monster_morph", None, None) else {
        monster.refresh();
        return;
    };
    let _ = monster.game.update_entity(&next, |entity| {
        entity.model = model;
        entity.owner = Some(id.clone());
        entity.mangle = mangle;
    });
    let _ = monster.game.set_body(
        &next,
        &BodyPatch {
            angles: Some(mangle),
            ..Default::default()
        },
    );
    drop_to_floor(monster);
    let enemy = monster.enemy.clone();
    let path = monster.state.path.clone();
    let movetarget = monster.entity.references.get("movetarget").cloned().flatten();
    let goalentity = monster.entity.references.get("goalentity").cloned().flatten();
    let game = &mut *monster.game;
    let runtime = &mut *monster.runtime;
    let Ok(mut controller) = runtime.adopt(game, &next) else {
        return;
    };
    controller.enemy = enemy;
    controller.state.path = path;
    controller
        .entity
        .references
        .insert("movetarget".to_string(), movetarget);
    controller
        .entity
        .references
        .insert("goalentity".to_string(), goalentity);
    controller.flush_entity();
    setup(&mut controller);
    let _ = controller.game.set_origin(&next, destination_origin);
    controller.next_frame = "morph_wake1".to_string();
    controller.delay(0.3);
    controller.finish();
}

/// Rogue morph definition (`morphDefinition`).
pub fn morph_definition() -> PackMonsterDefinition {
    let use_: MissionUse = Arc::new(|monster, _activator| {
        monster.next_frame = "morph_wake".to_string();
        let delay = monster.entity.delay;
        monster.delay(if delay == 0.0 { 0.1 } else { delay });
    });
    let melee: MissionAction = Arc::new(|monster| {
        let rolled = monster.game.host.random();
        monster.play(if rolled < 0.5 {
            "morph_bigattack01"
        } else if rolled < 0.75 {
            "morph_attack01"
        } else {
            "morph_knockback01"
        });
    });
    let pain: MissionPain = Arc::new(|monster, _attacker, _damage| {
        if monster.game.options().skill == 3 {
            if monster.game.host.random() > 0.5 {
                child(monster);
            }
            return;
        }
        if monster.state.pain_finished > monster.game.time || monster.game.host.random() > 0.25 {
            return;
        }
        let rolled = monster.game.host.random();
        let time = monster.game.time;
        monster.state.pain_finished = time + 2.0;
        let id = monster.entity.actor.id().clone();
        let _ = monster.game.sound_simple(&id, "guard/pain1.wav");
        monster.next_frame = if rolled > 0.6 { "morph_painB1" } else { "morph_painA1" }.to_string();
        monster.delay(0.1);
    });
    let die: MissionDie = Arc::new(|monster, _attacker| {
        let id = monster.entity.actor.id().clone();
        let _ = monster.game.sound_simple(&id, "guard/death.wav");
        monster.entity.solid = Q1Solid::None;
        monster.flush_entity();
        monster.next_frame = "morph_die1".to_string();
        let _ = monster.game.link(&id);
        monster.delay(0.1);
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Morph,
            kill_string: None,
            classnames: &["monster_morph"],
            model: "morph_az",
            head: None,
            health: 2000.0,
            gib_health: f64::NEG_INFINITY,
            gibs: &[],
            bounds: HULL_BOUNDS,
            stand: "morph_stand1",
            walk: "morph_walk1",
            run: "morph_run1",
            sight: "",
            missile: Some("morph_fire1"),
            melee: true,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions: vec![
            ("morph_stab2", Arc::new(stab)),
            ("morph_smack", Arc::new(smack)),
            ("morph_fire", Arc::new(fire)),
            ("morph_teleport", Arc::new(child)),
            ("morph_wake", Arc::new(wake)),
            (
                "morph:morph_die9",
                Arc::new(|monster| {
                    monster.entity.skin += 1;
                    monster.flush_entity();
                }),
            ),
            (
                "morph:morph_die21",
                Arc::new(|monster| {
                    let id = monster.entity.actor.id().clone();
                    let _ = monster.game.remove(&id);
                }),
            ),
            (
                "morph:morph_wake1",
                Arc::new(|monster| {
                    let id = monster.entity.actor.id().clone();
                    let _ = monster.game.sound_simple(&id, "guard/see1.wav");
                    if let Some(owner) = monster.entity.owner.clone() {
                        monster.game.total_monsters += 1;
                        let total = monster.game.total_monsters;
                        let _ = monster.game.update_entity(&owner, |entity| {
                            let spawned = entity.number("childrenSpawned") + 1.0;
                            entity
                                .fields
                                .insert("childrenSpawned".to_string(), (spawned as f32).to_string());
                        });
                        monster.game.host.emit(Q1Event::MonsterTotal { total });
                    }
                    monster.refresh();
                }),
            ),
            (
                "morph:morph_wake15",
                Arc::new(|monster| {
                    monster.entity.skin = 1;
                    monster.flush_entity();
                }),
            ),
            (
                "morph:morph_wake31",
                Arc::new(|monster| {
                    let id = monster.entity.actor.id().clone();
                    monster.entity.solid = Q1Solid::Slidebox;
                    monster.entity.aimed_damage = true;
                    monster.entity.skin -= 1;
                    monster.flush_entity();
                    let _ = monster.game.set_damageable(&id, true);
                    if monster.entity.owner.is_some() {
                        monster.next_frame = "morph_run1".to_string();
                        monster.delay(0.1);
                    }
                    let _ = monster.game.link(&id);
                }),
            ),
        ],
        callbacks: Vec::new(),
        spawn: Some(Arc::new(|monster| {
            if monster.entity.spawnflags & 2 != 0 {
                monster.entity.model = "progs/morph_az.mdl".to_string();
            } else if monster.entity.spawnflags & 4 != 0 {
                monster.entity.model = "progs/morph_eg.mdl".to_string();
            } else if monster.entity.spawnflags & 8 != 0 {
                monster.entity.model = "progs/morph_gr.mdl".to_string();
            } else {
                panic!("monster_morph: no skin selection!");
            }
            monster.flush_entity();
            monster.game.total_monsters += 1;
            if !monster.entity.targetname.is_empty() {
                if let Ok(use_callback) = monster.game.named.use_callback("rogue:monster_use") {
                    monster.entity.use_callback = Some(use_callback);
                    monster.flush_entity();
                }
                return;
            }
            wake(monster);
        })),
        start: None,
        pain,
        die,
        melee: Some(melee),
        check_attack: None,
        found: None,
        ai: None,
        use_: Some(use_),
    }
}

#[cfg(test)]
mod tests {
    use super::morph_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn morph_registers_with_wake_frames() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
            .expect("runtime");
        let definition = morph_definition();
        assert_eq!(definition.spec.classnames, &["monster_morph"]);
        assert_eq!(definition.spec.model, "morph_az");
        assert_eq!(definition.spec.health, 2000.0);
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 10);
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_morph", None, None).expect("create");
        let monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "morph_az");
    }
}
