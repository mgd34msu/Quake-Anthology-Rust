//! Hipnotic scourge (`src/content/q1/missionpacks/monsters/scourge.ts`).

use std::sync::Arc;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::projectiles::{launch_spike, spawn_meat_spray, throw_gib, throw_head, SpikeKind};
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::{Q1AttackState, Q1MonsterSpecies};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::TouchSurface;
use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, yaw_for, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, POINT,
};
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::Q1Error;

use super::helpers::{eye, number};
use super::runtime::{mission_pack_monsters, MissionMonster, Q1MissionPackMonsters};
use super::tables::hipscrge::FRAMES;
use super::types::{MissionAction, MissionDie, MissionPain, PackMonsterDefinition};

/// Maintain the dodge trigger and walk loop (`think`).
fn think(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    if monster.entity.number("scourge:state") == 0.0 {
        let Ok(trigger) = monster.game.create("scourge_trigger", None, None) else {
            monster.refresh();
            return;
        };
        let _ = monster.game.update_entity(&trigger, |entity| {
            entity.solid = Q1Solid::Trigger;
            entity.references.insert("lastvictim".to_string(), Some(id.clone()));
        });
        let _ = monster.game.set_damageable(&trigger, false);
        let _ = monster.game.set_bounds(
            &trigger,
            Bounds {
                min: Vec3 {
                    x: -64.0,
                    y: -64.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 64.0,
                    y: 64.0,
                    z: 64.0,
                },
            },
        );
        monster
            .entity
            .references
            .insert("lastvictim".to_string(), Some(trigger.clone()));
        monster.flush_entity();
        if let Ok(touch) = monster.game.named.touch("hipnotic:ScourgeTriggerTouch") {
            let _ = monster.game.update_entity(&trigger, |entity| {
                entity.touch = Some(touch);
            });
        }
        if let Ok(think) = monster.game.named.action("hipnotic:ScourgeTriggerThink") {
            let delay = 0.1 + monster.game.host.random();
            let _ = monster.game.schedule(&trigger, delay, &think);
        }
        let origin = monster.origin;
        let _ = monster.game.set_origin(&trigger, origin);
        number(monster, "scourge:state", 1.0);
    }
    let silent = monster.entity.number("spawnsilent");
    let multi = monster.entity.number("spawnmulti");
    if silent == 0.0 && multi == 1.0 {
        let _ = monster.game.sound(&id, "misc/null.wav", Q1SoundChannel::Body, 2.0, 1.0);
    } else if silent == 1.0 && multi == 0.0 {
        let _ = monster
            .game
            .sound(&id, "scourge/walk.wav", Q1SoundChannel::Body, 2.0, 1.0);
    }
    number(monster, "spawnmulti", silent);
}

/// Strafe sideways (`side`).
fn side(monster: &mut MissionMonster, right: bool, distance: f64) {
    let id = monster.entity.actor.id().clone();
    let owned = monster.entity.actor.clone();
    let yaw = monster
        .game
        .body(&id)
        .map(|body| f64::from(body.angles.y))
        .unwrap_or(0.0);
    monster
        .game
        .host
        .walk_move(&owned, yaw + if right { 90.0 } else { 270.0 }, distance);
    monster.refresh();
}

/// Fire a spike from a wing mount (`fire`).
fn fire(monster: &mut MissionMonster, offset: f64) {
    monster.face();
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
            vadd(
                monster.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -19.0,
                },
            ),
            vscale(basis.right, offset),
        ),
        vscale(basis.forward, 14.0),
    );
    let _ = monster
        .game
        .sound(&id, "weapons/rocket1i.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let _ = launch_spike(
        monster.game,
        Some(&id),
        origin,
        vscale(
            normalize(vsub(vadd(target, vscale(basis.forward, 200.0)), origin)),
            1000.0,
        ),
        SpikeKind::Spike,
    );
    let time = monster.game.time;
    monster.state.attack_finished = time + 0.2;
    monster.refresh();
}

/// Flash the muzzle and fire (`flash`).
fn flash(monster: &mut MissionMonster, offset: f64) {
    monster.entity.effects |= 2;
    monster.flush_entity();
    fire(monster, offset);
}

/// Tail swipe (`tail`).
fn tail(monster: &mut MissionMonster) {
    monster.face();
    let id = monster.entity.actor.id().clone();
    let Some(enemy) = monster.enemy.clone() else {
        monster.refresh();
        return;
    };
    if monster.distance() > 100.0 || !monster.game.can_damage(&enemy, &id) {
        monster.refresh();
        return;
    }
    let rolls = monster.game.host.random() + monster.game.host.random() + monster.game.host.random();
    monster
        .game
        .damage(&enemy, Some(&id), Some(&id), rolls * 40.0, &Q1DamageParams::default());
    let _ = monster
        .game
        .sound(&id, "shambler/smack.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let basis = monster.game.basis;
    let spray = vscale(basis.right, (monster.game.host.random() * 2.0 - 1.0) * 50.0);
    let origin = vadd(monster.origin, vscale(basis.forward, 16.0));
    let _ = spawn_meat_spray(monster.game, &id, origin, spray);
    monster.refresh();
}

/// Turn in place toward the enemy (`turn`).
fn turn(monster: &mut MissionMonster) {
    monster.delay(0.1);
    let Some(target) = monster.target else { return };
    let id = monster.entity.actor.id().clone();
    let yaw = monster
        .game
        .body(&id)
        .map(|body| f64::from(body.angles.y))
        .unwrap_or(0.0);
    if (yaw - yaw_for(vsub(target, monster.origin))).abs() > 10.0 {
        monster.face();
        return;
    }
    monster.next_frame = monster.spec.run.to_string();
}

/// Check for a melee or missile attack (`checkAttack`).
fn check_attack(monster: &mut MissionMonster) -> bool {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let target = monster.enemy.clone().and_then(|enemy| eye(monster.game, &enemy));
    let Some(target) = target else {
        monster.refresh();
        return false;
    };
    let start = vadd(
        monster.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
    );
    let delta = vsub(target, start);
    let distance = f64::from(length(delta));
    if distance <= 100.0
        && monster
            .enemy
            .clone()
            .is_some_and(|enemy| monster.game.can_damage(&enemy, &id))
    {
        monster.entity.attack_state = Q1AttackState::Melee;
        monster.flush_entity();
        return true;
    }
    if monster.game.time < monster.state.attack_finished
        || !monster.visible(None)
        || delta.z > 64.0
        || delta.z < -200.0
        || distance > 1000.0
        || distance < 150.0
    {
        monster.refresh();
        return false;
    }
    let trace = monster.game.host.trace(&Q1TraceRequest {
        start,
        end: target,
        bounds: POINT,
        ignore: Some(id),
        monsters: true,
        missile: false,
    });
    if trace.actor != monster.enemy || trace.in_open && trace.in_water {
        monster.refresh();
        return false;
    }
    monster.entity.attack_state = Q1AttackState::Missile;
    monster.flush_entity();
    let delay = 2.0 + 2.0 * monster.game.host.random();
    monster.attack_finished(delay);
    true
}

/// Track the owner ahead of the scourge (`ScourgeTriggerThink`).
fn scourge_trigger_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let owner = game
        .entity(&id)
        .and_then(|entity| entity.references.get("lastvictim").cloned().flatten());
    let target = owner.as_ref().and_then(|owner| game.entity(owner).cloned());
    let Some(target) = target else {
        return game.remove(&id);
    };
    if game.health(target.actor.id()) <= 0.0 {
        return game.remove(&id);
    }
    let body = game.body(target.actor.id())?;
    let forward = game.make_vectors(body.angles).forward;
    game.set_origin(&id, vadd(body.origin, vscale(forward, 300.0)))?;
    let again = game.named.action("hipnotic:ScourgeTriggerThink")?;
    game.schedule(&id, 0.1, &again)
}

/// Dodge incoming missiles (`ScourgeTriggerTouch`).
fn scourge_trigger_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (id, other) = (id.clone(), other.clone());
    let shot = game.entity(&other).cloned();
    let owner = game
        .entity(&id)
        .and_then(|entity| entity.references.get("lastvictim").cloned().flatten());
    let Some(shot) = shot else { return Ok(()) };
    if shot.movement_flags & (32 | 8) != 0 || game.is_player(&other) || shot.movement != Q1MoveType::Flymissile {
        return Ok(());
    }
    let mut local: Q1MissionPackMonsters = mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(mut monster) = owner.as_ref().and_then(|owner| local.context(game, owner)) else {
        return Ok(());
    };
    let monster_id = monster.entity.actor.id().clone();
    if monster.game.health(&monster_id) <= 0.0 {
        let _ = monster.game.remove(&id);
        return Ok(());
    }
    let shot_body = monster.game.host.bodies.read(&other);
    let Some(shot_body) = shot_body else { return Ok(()) };
    if f64::from(dot(
        normalize(vsub(monster.origin, shot_body.origin)),
        normalize(shot_body.velocity),
    )) < 0.8
    {
        return Ok(());
    }
    let duration = monster
        .game
        .entity(&id)
        .map(|entity| entity.number("duration"))
        .unwrap_or(0.0);
    if monster.game.time > duration {
        let frame = if monster.game.host.random() < 0.5 {
            "scourge_strafeleft1"
        } else {
            "scourge_straferight1"
        };
        monster.play(frame);
        let time = monster.game.time;
        number(&mut monster, "duration", time + 1.5);
    }
    monster.finish();
    *mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = local;
    Ok(())
}

/// Hipnotic scourge definition (`scourgeDefinition`).
pub fn scourge_definition(runtime: &Q1MissionPackMonsters) -> PackMonsterDefinition {
    debug_assert!(
        matches!(runtime.pack, Q1MissionPack::Hipnotic),
        "scourge is Hipnotic-only"
    );
    let actions: Vec<(&'static str, MissionAction)> = vec![
        ("scourge_think", Arc::new(think)),
        (
            "hipscrge:scourge_stand1",
            Arc::new(|monster| {
                number(monster, "spawnsilent", 0.0);
                monster.ai(MonsterAi::Stand, 0.0);
                think(monster);
            }),
        ),
        (
            "hipscrge:scourge_walk1",
            Arc::new(|monster| {
                if monster.game.host.random() < 0.1 {
                    let id = monster.entity.actor.id().clone();
                    let _ = monster
                        .game
                        .sound(&id, "scourge/idle.wav", Q1SoundChannel::Voice, 2.0, 1.0);
                }
                number(monster, "spawnsilent", 1.0);
                think(monster);
                monster.ai(MonsterAi::Walk, 8.0);
            }),
        ),
        (
            "hipscrge:scourge_run1",
            Arc::new(|monster| {
                if monster.game.host.random() < 0.1 {
                    let id = monster.entity.actor.id().clone();
                    let _ = monster
                        .game
                        .sound(&id, "scourge/idle.wav", Q1SoundChannel::Voice, 2.0, 1.0);
                }
                number(monster, "spawnsilent", 1.0);
                think(monster);
                monster.ai(MonsterAi::Run, 18.0);
            }),
        ),
        (
            "hipscrge:scourge_strafeleft1",
            Arc::new(|monster| {
                number(monster, "spawnsilent", 1.0);
                think(monster);
                side(monster, false, 20.0);
            }),
        ),
        (
            "hipscrge:scourge_straferight1",
            Arc::new(|monster| {
                number(monster, "spawnsilent", 1.0);
                think(monster);
                side(monster, true, 20.0);
            }),
        ),
        (
            "hipscrge:scourge_turn1",
            Arc::new(|monster| {
                number(monster, "spawnsilent", 1.0);
                think(monster);
                turn(monster);
            }),
        ),
        ("ai_left(20)", Arc::new(|monster| side(monster, false, 20.0))),
        ("ai_left(14)", Arc::new(|monster| side(monster, false, 14.0))),
        ("ai_right(20)", Arc::new(|monster| side(monster, true, 20.0))),
        ("ai_right(14)", Arc::new(|monster| side(monster, true, 14.0))),
        ("ai_turn_in_place", Arc::new(turn)),
        (
            "hipscrge:scourge_atk1",
            Arc::new(|monster| {
                number(monster, "spawnsilent", 0.0);
                think(monster);
                flash(monster, 40.0);
            }),
        ),
        ("hipscrge:scourge_atk2", Arc::new(|monster| flash(monster, -56.0))),
        ("hipscrge:scourge_atk3", Arc::new(|monster| flash(monster, -40.0))),
        ("hipscrge:scourge_atk4", Arc::new(|monster| flash(monster, 56.0))),
        ("hipscrge:scourge_atk5", Arc::new(|monster| flash(monster, 40.0))),
        (
            "hipscrge:scourge_atk8",
            Arc::new(|monster| {
                flash(monster, 56.0);
                let delay = 4.0 * monster.game.host.random();
                monster.attack_finished(delay);
            }),
        ),
        (
            "hipscrge:scourge_melee1",
            Arc::new(|monster| {
                number(monster, "spawnsilent", 0.0);
                think(monster);
                monster.ai(MonsterAi::Charge, 3.0);
            }),
        ),
        (
            "hipscrge:scourge_melee11",
            Arc::new(|monster| {
                monster.face();
                if monster.game.options().skill == 3 && !monster.state.refired && monster.visible(None) {
                    monster.state.refired = true;
                    monster.next_frame = "scourge_melee1".to_string();
                }
            }),
        ),
        (
            "hipscrge:scourge_pain1",
            Arc::new(|monster| {
                number(monster, "spawnsilent", 0.0);
                think(monster);
            }),
        ),
        ("Attack_With_Tail", Arc::new(tail)),
    ];
    let spawn: MissionAction = Arc::new(|monster| {
        monster.entity.fields.insert("yaw_speed".to_string(), "60".to_string());
        monster.flush_entity();
        number(monster, "scourge:state", 0.0);
        monster.entity.attack_state = Q1AttackState::Dodging;
        monster.flush_entity();
        monster.spawn_default();
    });
    let melee: MissionAction = Arc::new(|monster| {
        monster.play("scourge_melee1");
        let delay = 2.0 * monster.game.host.random();
        monster.attack_finished(delay);
    });
    let pain: MissionPain = Arc::new(|monster, _attacker, damage| {
        if monster.game.host.random() * 50.0 > damage || monster.state.pain_finished > monster.game.time {
            return;
        }
        let _ = monster.game.host.random();
        let id = monster.entity.actor.id().clone();
        let _ = monster.game.sound_simple(&id, "scourge/pain.wav");
        let time = monster.game.time;
        monster.state.pain_finished = time + 2.0;
        monster.play("scourge_pain1");
    });
    let die: MissionDie = Arc::new(|monster, _attacker| {
        monster.sync();
        let id = monster.entity.actor.id().clone();
        if let Some(trigger) = monster.entity.references.get("lastvictim").cloned().flatten() {
            let _ = monster.game.remove(&trigger);
        }
        number(monster, "spawnsilent", 0.0);
        think(monster);
        let health = monster.game.health(&id);
        if health < -35.0 {
            let _ = monster.game.sound_simple(&id, "player/udeath.wav");
            let _ = throw_head(monster.game, &id, "h_scourg", health);
            for model in ["gib1", "gib2", "gib3"] {
                let _ = throw_gib(monster.game, monster.origin, model, health);
            }
            return;
        }
        let _ = monster.game.sound_simple(&id, "scourge/pain2.wav");
        monster.play("scourge_die1");
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Scourge,
            kill_string: None,
            classnames: &["monster_scourge"],
            model: "scor",
            head: Some("h_scourg"),
            health: 300.0,
            gib_health: -35.0,
            gibs: &["gib1", "gib2", "gib3"],
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
            stand: "scourge_stand1",
            walk: "scourge_walk1",
            run: "scourge_run1",
            sight: "scourge/sight.wav",
            missile: Some("scourge_atk1"),
            melee: true,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions,
        callbacks: vec![
            (
                "ScourgeTriggerThink",
                Q1CallbackHandlers {
                    action: Some(scourge_trigger_think),
                    ..Default::default()
                },
            ),
            (
                "ScourgeTriggerTouch",
                Q1CallbackHandlers {
                    touch: Some(scourge_trigger_touch),
                    ..Default::default()
                },
            ),
        ],
        spawn: Some(spawn),
        start: None,
        pain,
        die,
        melee: Some(melee),
        check_attack: Some(Arc::new(check_attack)),
        found: None,
        ai: None,
        use_: None,
    }
}

#[cfg(test)]
mod tests {
    use super::scourge_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn scourge_registers_with_dodge_trigger() {
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default())
                .expect("runtime");
        let definition = scourge_definition(&runtime);
        assert_eq!(definition.spec.classnames, &["monster_scourge"]);
        assert_eq!(definition.spec.model, "scor");
        assert_eq!(definition.spec.health, 300.0);
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 22);
        assert_eq!(definition.callbacks.len(), 2);
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_scourge", None, None).expect("create");
        let monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "scor");
    }
}
