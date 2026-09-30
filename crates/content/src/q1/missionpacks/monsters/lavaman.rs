//! Rogue lava man (`src/content/q1/missionpacks/monsters/lavaman.ts`).

use std::sync::Arc;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, Q1Effect, Q1MoveType, Q1Powerup, Q1Solid, Q1SoundChannel, Q1TraceRequest,
    POINT,
};
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::Q1Error;

use super::helpers::{drop_to_floor, missile};
use super::runtime::MissionMonster;
use super::tables::lavaman::FRAMES;
use super::types::{MissionAction, MissionCheckAttack, MissionDie, MissionPain, MissionUse, PackMonsterDefinition};

/// Check for a fireball attack (`checkAttack`).
fn check_attack(monster: &mut MissionMonster) -> bool {
    monster.face();
    let id = monster.entity.actor.id().clone();
    let Some(target) = monster.target else {
        monster.refresh();
        return false;
    };
    let trace = monster.game.host.trace(&Q1TraceRequest {
        start: vadd(
            monster.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 64.0,
            },
        ),
        end: target,
        bounds: POINT,
        ignore: Some(id),
        monsters: true,
        missile: false,
    });
    if trace.actor != monster.enemy
        || trace.in_open && trace.in_water
        || monster.game.time < monster.state.attack_finished
    {
        monster.refresh();
        return false;
    }
    monster.play("lavaman_fire1");
    let delay = 1.0 + monster.game.host.random();
    monster.attack_finished(delay);
    true
}

/// Acquire the next visible player (`hunt`).
fn hunt(monster: &mut MissionMonster) {
    monster.sync();
    if monster.enemy.is_none()
        || monster
            .enemy
            .clone()
            .is_some_and(|enemy| monster.game.health(&enemy) <= 0.0)
    {
        let players = (monster.game.host.players)();
        let index = monster
            .enemy
            .clone()
            .and_then(|enemy| players.iter().position(|player| player == &enemy))
            .map(|index| index as i64)
            .unwrap_or(-1);
        let candidate = players.get((index + 1) as usize).cloned();
        let body = candidate
            .as_ref()
            .and_then(|candidate| monster.game.host.bodies.read(candidate));
        if let (Some(candidate), Some(body)) = (candidate, body) {
            let world = monster.game.world.clone();
            if monster
                .game
                .host
                .trace(&Q1TraceRequest {
                    start: vadd(
                        monster.origin,
                        Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 96.0,
                        },
                    ),
                    end: body.origin,
                    bounds: POINT,
                    ignore: world,
                    monsters: false,
                    missile: false,
                })
                .fraction
                == 1.0
            {
                monster.enemy = Some(candidate);
            }
        }
    }
    if monster.enemy.is_some() {
        monster.face();
    } else {
        monster.refresh();
    }
}

/// Shared locomotion (`locomotion`).
fn locomotion(monster: &mut MissionMonster, mode: MonsterAi, distance: f64) {
    if monster.enemy.is_some() {
        check_attack(monster);
    } else {
        hunt(monster);
    }
    if matches!(mode, MonsterAi::Walk) {
        if let Some(enemy) = monster.enemy.clone() {
            monster.find_target();
            let owned = monster.entity.actor.clone();
            monster.game.host.move_to_goal(&owned, &enemy, distance, None);
            monster.refresh();
            return;
        }
    }
    monster.ai(mode, distance);
}

/// Throw a lava ball (`fire`).
fn fire(monster: &mut MissionMonster, side: i32) {
    let id = monster.entity.actor.id().clone();
    let Some(target) = monster.target else {
        monster.refresh();
        return;
    };
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let basis = monster.game.make_vectors(body.angles);
    let origin = vadd(
        vadd(
            vadd(monster.origin, vscale(basis.forward, 40.0)),
            vscale(basis.right, if side == 1 { 65.0 } else { -75.0 }),
        ),
        vscale(basis.up, if side == 1 { 130.0 } else { 125.0 }),
    );
    let direction = normalize(vsub(target, origin));
    let flight = (f64::from(length(vsub(target, origin))) / 380.0).clamp(1.0, 1.75);
    let ball = missile(
        monster.game,
        &id,
        "lavaman_ball",
        "progs/lavaball.mdl",
        origin,
        vadd(
            vscale(direction, 600.0 * flight),
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: (200.0 * flight) as f32,
            },
        ),
        "rogue:lavaman_touch",
        6.0,
    );
    let _ = monster.game.update_entity(&ball, |entity| {
        entity.movement = Q1MoveType::Bounce;
        entity.angular_velocity = Vec3 {
            x: 200.0,
            y: 100.0,
            z: 300.0,
        };
    });
    let _ = monster.game.set_body(
        &ball,
        &BodyPatch {
            angles: Some(velocity_angles(direction)),
            ..Default::default()
        },
    );
    let _ = monster
        .game
        .sound(&id, "boss1/throw.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    if monster.enemy.is_none()
        || monster
            .enemy
            .clone()
            .is_some_and(|enemy| monster.game.health(&enemy) <= 0.0)
    {
        monster.play("lavaman_idle1");
    } else {
        monster.refresh();
    }
}

/// Emerge from the lava (`awake`).
fn awake(monster: &mut MissionMonster, activator: Option<&ActorId>) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    monster.entity.solid = Q1Solid::Slidebox;
    monster.entity.movement = Q1MoveType::Step;
    monster.entity.aimed_damage = true;
    monster.entity.movement_flags |= 32;
    if let Ok(body) = monster.game.body(&id) {
        monster.entity.ideal_yaw = f64::from(body.angles.y);
    }
    let yaw_speed = monster.entity.number("yaw_speed");
    monster.entity.yaw_speed = if yaw_speed == 0.0 { 20.0 } else { yaw_speed };
    monster.entity.model = "progs/lavaman.mdl".to_string();
    monster
        .entity
        .fields
        .insert("view_ofs".to_string(), "0 0 48".to_string());
    monster.flush_entity();
    let _ = monster.game.set_bounds(&id, monster.spec.bounds);
    let _ = monster.game.set_damageable(&id, true);
    if let Ok(pain) = monster.game.named.pain("rogue:monster_pain") {
        monster.entity.pain = Some(pain);
    }
    if let Ok(die) = monster.game.named.die("rogue:monster_die") {
        monster.entity.die = Some(die);
    }
    monster.entity.max_health = 1250.0 + 250.0 * monster.game.options().skill as f64;
    monster.flush_entity();
    let owned = monster.entity.actor.clone();
    let _ = monster.game.host.combat.set_health(&owned, monster.entity.max_health);
    monster.game.effect_simple(Q1Effect::LavaSplash, monster.origin);
    if let Some(activator) = activator {
        let invisible_until = monster
            .game
            .player_ref(activator)
            .and_then(|player| player.powerups.get(&Q1Powerup::Invisibility).copied())
            .unwrap_or(0.0);
        if monster.game.is_player(activator)
            && invisible_until <= monster.game.time
            && monster
                .game
                .entity(activator)
                .map(|entity| entity.movement_flags)
                .unwrap_or(0)
                & 128
                == 0
        {
            monster.enemy = Some(activator.clone());
        }
    }
    drop_to_floor(monster);
    if let Some(mission) = monster.game.monster_missions.get_mut(&id) {
        mission.started();
    }
    monster.play("lavaman_rise1");
}

/// Detonate a lava ball (`lavaman_touch`).
fn lavaman_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (id, other) = (id.clone(), other.clone());
    let owner = game.entity(&id).and_then(|entity| entity.owner.clone());
    if Some(&other) == owner.as_ref() {
        return Ok(());
    }
    let body = game.body(&id)?;
    if game.host.contents(body.origin) == Q1Contents::Sky {
        return game.remove(&id);
    }
    if game.health(&other) != 0.0 {
        let damage = if game.host.classname(&other) == "monster_shambler" {
            20.0
        } else {
            40.0
        };
        game.damage(&other, Some(&id), owner.as_ref(), damage, &Q1DamageParams::default());
    }
    game.radius_damage(&id, owner.as_ref(), 40.0, Some(&other), None, "");
    let body = game.body(&id)?;
    game.set_body(
        &id,
        &BodyPatch {
            origin: Some(vsub(body.origin, vscale(normalize(body.velocity), 8.0))),
            ..Default::default()
        },
    )?;
    let origin = game.body(&id).map(|moved| moved.origin).unwrap_or(body.origin);
    game.effect_simple(Q1Effect::Explosion, origin);
    game.effect_simple(Q1Effect::Explosion, origin);
    game.remove(&id)
}

/// Rogue lava-man definition (`lavamanDefinition`).
pub fn lavaman_definition() -> PackMonsterDefinition {
    let use_: MissionUse = Arc::new(awake);
    let check_attack: MissionCheckAttack = Arc::new(check_attack);
    let melee: MissionAction = Arc::new(|monster| {
        monster.play("lavaman_fire1");
    });
    let pain: MissionPain = Arc::new(|monster, _attacker, _damage| {
        if monster.state.pain_finished > monster.game.time || monster.game.host.random() >= 0.05 {
            return;
        }
        let time = monster.game.time;
        monster.state.pain_finished = time + 2.0;
        monster.play("lavaman_shocka1");
    });
    let die: MissionDie = Arc::new(|monster, _attacker| {
        monster.play("lavaman_death1");
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::LavaMan,
            kill_string: None,
            classnames: &["monster_lava_man"],
            model: "lavaman",
            head: None,
            health: 1500.0,
            gib_health: f64::NEG_INFINITY,
            gibs: &[],
            bounds: Bounds {
                min: Vec3 {
                    x: -32.0,
                    y: -32.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 32.0,
                    y: 32.0,
                    z: 64.0,
                },
            },
            stand: "lavaman_idle1",
            walk: "lavaman_walk1",
            run: "lavaman_walk1",
            sight: "",
            missile: Some("lavaman_fire1"),
            melee: true,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions: vec![
            (
                "lavaman_stand",
                Arc::new(|monster| locomotion(monster, MonsterAi::Stand, 0.0)),
            ),
            (
                "lavaman_walk",
                Arc::new(|monster| locomotion(monster, MonsterAi::Walk, 2.0)),
            ),
            (
                "lavaman_run",
                Arc::new(|monster| locomotion(monster, MonsterAi::Run, 2.0)),
            ),
            ("lavaman_missile(1)", Arc::new(|monster| fire(monster, 1))),
            ("lavaman_missile(2)", Arc::new(|monster| fire(monster, 2))),
            (
                "lavaman:lavaman_death9",
                Arc::new(|monster| {
                    let id = monster.entity.actor.id().clone();
                    let _ = monster
                        .game
                        .sound(&id, "boss1/out1.wav", Q1SoundChannel::Body, 1.0, 1.0);
                    let origin = monster.origin;
                    monster.game.effect_simple(Q1Effect::LavaSplash, origin);
                }),
            ),
            (
                "lavaman:lavaman_death10",
                Arc::new(|monster| {
                    let id = monster.entity.actor.id().clone();
                    let _ = monster.game.remove(&id);
                }),
            ),
        ],
        callbacks: vec![(
            "lavaman_touch",
            Q1CallbackHandlers {
                touch: Some(lavaman_touch),
                ..Default::default()
            },
        )],
        spawn: Some(Arc::new(|monster| {
            let id = monster.entity.actor.id().clone();
            if monster.game.monster_missions.contains_key(&id) {
                if let Some(mission) = monster.game.monster_missions.get_mut(&id) {
                    mission.spawned();
                }
            } else {
                monster.game.total_monsters += 1;
            }
            if monster.entity.spawnflags & 2 != 0 {
                if let Ok(use_callback) = monster.game.named.use_callback("rogue:monster_use") {
                    monster.entity.use_callback = Some(use_callback);
                    monster.flush_entity();
                }
                return;
            }
            let activator = monster.entity.activator.clone();
            awake(monster, activator.as_ref());
        })),
        start: None,
        pain,
        die,
        melee: Some(melee),
        check_attack: Some(check_attack),
        found: None,
        ai: None,
        use_: Some(use_),
    }
}

#[cfg(test)]
mod tests {
    use super::lavaman_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn lavaman_registers_with_ball_touch() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
            .expect("runtime");
        let definition = lavaman_definition();
        assert_eq!(definition.spec.classnames, &["monster_lava_man"]);
        assert_eq!(definition.spec.model, "lavaman");
        assert_eq!(definition.spec.health, 1500.0);
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 7);
        assert_eq!(definition.callbacks.len(), 1);
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_lava_man", None, None).expect("create");
        let monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "lavaman");
    }
}
