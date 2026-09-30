//! Q1 mg3 orb (`src/content/q1/addons/monsters/bosses/orb.ts`).
//!
//! `quakec_mg3/monsters/mg3_orb.qc`. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{addon_emit, set_addon_number, Q1AddonEvent};
use crate::q1::addons::monsters::ai::{register_mg3_monster_source, Mg3ActionHandler, Mg3Monster, Mg3SourceHooks};
use crate::q1::addons::monsters::startup::{mg3_use_mapped, start_mg3_monster};
use crate::q1::base::animation::MonsterAi;
use crate::q1::base::creatures::{monster_controller, store_monster_controller};
use crate::q1::base::monsters::BaseMonsterState;
use crate::q1::base::projectiles::{launch_spike, SpikeKind};
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity::{Q1AttackState, Q1MonsterSpecies};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, yaw_for, Q1Effect, Q1SoundChannel, Q1TraceRequest, POINT, ZERO,
};
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::Q1Error;

use super::effects::pain_lightning;
use super::frames::orb::frames;
use super::registry::register_boss_controllers;

/// Orb callback prefix.
const ORB_PREFIX: &str = "mg3:orb";

/// Orb spawn defaults (`spec`).
const ORB_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Wizard,
    kill_string: None,
    classnames: &["monster_orb"],
    model: "teleporter_eye_blink",
    head: None,
    health: 300.0,
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
    stand: "orb_stand1",
    walk: "orb_walk1",
    run: "orb_run1",
    sight: "",
    missile: Some("orb_fast1"),
    melee: false,
    movement: MonsterMovement::Fly,
};

fn orb_idle_sound(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let waitmin = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("waitmin"))
        .unwrap_or(0.0);
    if waitmin < monster.monster.game.time {
        let time = monster.monster.game.time;
        let delay = monster.monster.game.host.random() * 10.0;
        set_addon_number(monster.monster.game, &id, "waitmin", time + 15.0 + delay)?;
        monster
            .monster
            .game
            .sound(&id, "boss2/sight.wav", Q1SoundChannel::Voice, 2.0, 1.0)?;
    }
    Ok(())
}

/// Orb frame actions (`actions`).
fn orb_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn stand1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Stand, 0.0)
        }
        fn walk1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Walk, 8.0)?;
            orb_idle_sound(monster)
        }
        fn side1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            monster.ai(MonsterAi::Run, 16.0)?;
            orb_idle_sound(monster)
        }
        fn run1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 16.0)?;
            orb_idle_sound(monster)
        }
        fn fast1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let count = 1.0 + (monster.monster.game.host.random() * 3.0 + 0.5).floor();
            set_addon_number(monster.monster.game, &id, "ammo_nails", count)?;
            monster.monster.face()
        }
        fn fast3(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.delay(0.3)?;
            monster.monster.face()
        }
        fn fast4(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            orb_blast(monster)?;
            monster.monster.delay(0.2)?;
            let id = monster.monster.id.clone();
            let remaining = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.number("ammo_nails"))
                .unwrap_or(0.0)
                - 1.0;
            set_addon_number(monster.monster.game, &id, "ammo_nails", remaining)?;
            if remaining != 0.0 {
                monster.monster.controller.next_frame = String::from("orb_fast2");
            }
            Ok(())
        }
        fn fast5(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            monster.attack_finished(2.0);
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .update_entity(&id, |entity| entity.attack_state = Q1AttackState::Straight)?;
            let world = monster.monster.game.world.clone();
            let range = world
                .as_ref()
                .and_then(|world| monster.monster.game.entity_ref(world))
                .map(|entity| entity.number("enemy_range"))
                .unwrap_or(0.0);
            let visible = world
                .as_ref()
                .and_then(|world| monster.monster.game.entity_ref(world))
                .map(|entity| entity.number("enemy_visible"))
                .unwrap_or(0.0);
            monster.monster.controller.sliding = range < 2.0 && visible != 0.0;
            monster.monster.controller.next_frame = String::from(if monster.monster.controller.sliding {
                "orb_side1"
            } else {
                "orb_run1"
            });
            Ok(())
        }
        fn pain1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            set_addon_number(monster.monster.game, &id, "ammo_shells", 3.0)?;
            monster.monster.delay(0.2)
        }
        fn pain2(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let context =
                crate::q1::addons::context::Q1AddonContext::new(crate::q1::addons::context::Q1AddonProgram::Mg3);
            pain_lightning(&context, monster.monster.game, &id, ZERO)?;
            monster.monster.delay(0.3)?;
            let remaining = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.number("ammo_shells"))
                .unwrap_or(0.0)
                - 1.0;
            set_addon_number(monster.monster.game, &id, "ammo_shells", remaining)?;
            if remaining != 0.0 {
                monster.monster.controller.next_frame = String::from("orb_pain2");
            }
            Ok(())
        }
        fn death1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let vx = -200.0 + 400.0 * monster.monster.game.host.random();
            let vy = -200.0 + 400.0 * monster.monster.game.host.random();
            let vz = 150.0 + 150.0 * monster.monster.game.host.random();
            monster.monster.game.set_body(
                &id,
                &BodyPatch {
                    velocity: Some(Vec3 {
                        x: vx as f32,
                        y: vy as f32,
                        z: vz as f32,
                    }),
                    ground: Some(None),
                    ..Default::default()
                },
            )?;
            monster.monster.game.update_entity(&id, |entity| {
                entity.movement_flags &= !512;
                entity.solid = crate::q1::foundation::types::Q1Solid::Bbox;
            })?;
            monster.monster.game.set_bounds(&id, POINT)?;
            monster
                .monster
                .game
                .sound(&id, "orb/orb_death.wav", Q1SoundChannel::Voice, 1.0, 1.0)
        }
        fn death3(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let touch = monster.monster.game.named.touch(&format!("{ORB_PREFIX}:death_touch"))?;
            monster
                .monster
                .game
                .update_entity(&id, |entity| entity.touch = Some(touch))
        }
        fn death4(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.delay(9999.0)
        }
        HashMap::from([
            (String::from("orb:orb_stand1"), stand1 as Mg3ActionHandler),
            (String::from("orb:orb_walk1"), walk1 as Mg3ActionHandler),
            (String::from("orb:orb_side1"), side1 as Mg3ActionHandler),
            (String::from("orb:orb_run1"), run1 as Mg3ActionHandler),
            (String::from("orb:orb_fast1"), fast1 as Mg3ActionHandler),
            (String::from("orb:orb_fast3"), fast3 as Mg3ActionHandler),
            (String::from("orb:orb_fast4"), fast4 as Mg3ActionHandler),
            (String::from("orb:orb_fast5"), fast5 as Mg3ActionHandler),
            (String::from("orb:orb_pain1"), pain1 as Mg3ActionHandler),
            (String::from("orb:orb_pain2"), pain2 as Mg3ActionHandler),
            (String::from("orb:orb_death1"), death1 as Mg3ActionHandler),
            (String::from("orb:orb_death3"), death3 as Mg3ActionHandler),
            (String::from("orb:orb_death4"), death4 as Mg3ActionHandler),
        ])
    })
}

fn orb_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    _classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = monster_controller(game, id, "monster_orb")?;
    Ok((controller, &ORB_SPEC))
}

fn orb_start(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let context = crate::q1::addons::context::Q1AddonContext::new(crate::q1::addons::context::Q1AddonProgram::Mg3);
    start_mg3_monster(monster, &context)
}

fn orb_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
    monster.retaliate(attacker)?;
    let id = monster.monster.id.clone();
    if monster.monster.monster.pain_finished > monster.monster.game.time
        || monster.monster.game.host.random() * 200.0 > damage
    {
        return Ok(());
    }
    monster
        .monster
        .game
        .sound(&id, "orb/orb_pain.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    let time = monster.monster.game.time;
    monster.monster.monster.pain_finished = time + 8.0;
    monster.play("orb_pain1")
}

fn orb_die(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
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
    monster.play("orb_death1")
}

fn straighten(monster: &mut Mg3Monster, frame: &str) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let attack_state = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.attack_state)
        .unwrap_or(Q1AttackState::Straight);
    if monster.monster.controller.sliding || attack_state != Q1AttackState::Straight {
        monster.monster.controller.sliding = false;
        monster
            .monster
            .game
            .update_entity(&id, |entity| entity.attack_state = Q1AttackState::Straight)?;
        monster.play(frame)?;
    }
    Ok(())
}

fn orb_try_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    let id = monster.monster.id.clone();
    let world = monster.monster.game.world.clone();
    let range = world
        .as_ref()
        .and_then(|world| monster.monster.game.entity_ref(world))
        .map(|entity| entity.number("enemy_range"))
        .unwrap_or(0.0);
    let visible = world
        .as_ref()
        .and_then(|world| monster.monster.game.entity_ref(world))
        .map(|entity| entity.number("enemy_visible"))
        .unwrap_or(0.0);
    if monster.monster.game.time < monster.monster.monster.attack_finished || visible == 0.0 {
        return Ok(false);
    }
    if range == 3.0 {
        straighten(monster, "wiz_run1")?;
        return Ok(false);
    }
    let enemy = monster.monster.monster.enemy.clone();
    let target = match enemy.as_ref() {
        Some(enemy) => monster.monster.eye(Some(enemy))?,
        None => None,
    };
    let origin = monster.monster.eye(None)?;
    let (Some(target), Some(origin)) = (target, origin) else {
        return Ok(false);
    };
    let trace = monster.monster.game.host.trace(&Q1TraceRequest {
        start: origin,
        end: target,
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    let hit_enemy = match (&trace.actor, &enemy) {
        (Some(hit), Some(enemy)) => same_actor(hit, enemy),
        _ => false,
    };
    if !hit_enemy {
        straighten(monster, "orb_run1")?;
        return Ok(false);
    }
    if monster.monster.game.host.random()
        < (if range == 0.0 {
            0.9
        } else if range == 1.0 {
            0.6
        } else if range == 2.0 {
            0.2
        } else {
            0.0
        })
    {
        monster.monster.controller.sliding = false;
        monster
            .monster
            .game
            .update_entity(&id, |entity| entity.attack_state = Q1AttackState::Missile)?;
        return Ok(true);
    }
    if range == 2.0 {
        straighten(monster, "orb_run1")?;
    } else if !monster.monster.controller.sliding {
        monster
            .monster
            .game
            .update_entity(&id, |entity| entity.attack_state = Q1AttackState::Straight)?;
        monster.monster.controller.sliding = true;
        monster.play("orb_side1")?;
    }
    Ok(false)
}

/// Fires the orb scatter blast (`blast`).
fn orb_blast(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let skill = monster.monster.game.options().skill;
    let speed = if skill > 2 {
        500.0
    } else if skill > 0 {
        450.0
    } else {
        400.0
    };
    let mut count = 4.0 + (monster.monster.game.host.random() * 2.0 + 0.5).floor();
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = monster.monster.origin()?;
    let basis = monster.monster.game.make_vectors(velocity_angles(vsub(target, origin)));
    let muzzle = vadd(origin, vscale(basis.forward, 15.0));
    let time = f64::from(length(vsub(target, muzzle))) / speed;
    let enemy = monster.monster.monster.enemy.clone();
    let target_velocity = enemy
        .as_ref()
        .and_then(|enemy| monster.monster.game.host.bodies.read(enemy))
        .map(|body| body.velocity)
        .unwrap_or(ZERO);
    let flat = Vec3 {
        x: target_velocity.x,
        y: target_velocity.y,
        z: 0.0,
    };
    let projected = vadd(target, vscale(flat, time / 4.0));
    let direction = normalize(vsub(projected, muzzle));
    monster.monster.game.effect(Q1Effect::Explosion, muzzle, None, 1);
    while count > 0.0 {
        let random_x = monster.monster.game.host.random() * 2.0 - 1.0;
        let random_y = monster.monster.game.host.random() * 2.0 - 1.0;
        let right = monster.monster.game.basis.right;
        let up = monster.monster.game.basis.up;
        let aim = normalize(vadd(
            vadd(direction, vscale(right, random_x * 0.1)),
            vscale(up, random_y * 0.1),
        ));
        let missile = launch_spike(
            monster.monster.game,
            Some(&id),
            vadd(muzzle, vscale(aim, 8.0)),
            vscale(aim, 1000.0),
            SpikeKind::Spike,
        )?;
        let spread = monster.monster.game.host.random() * 2.0 - 1.0;
        let spin_x = monster.monster.game.host.random() * 2.0 - 1.0;
        let spin_y = monster.monster.game.host.random() * 2.0 - 1.0;
        let spin_z = monster.monster.game.host.random() * 2.0 - 1.0;
        monster.monster.game.update_entity(&missile, |entity| {
            entity.classname = String::from("rock");
            entity.model = String::from("progs/rogue/sphere.mdl");
            entity.angular_velocity = Vec3 {
                x: (300.0 * spin_x) as f32,
                y: (300.0 * spin_y) as f32,
                z: (300.0 * spin_z) as f32,
            };
            if count as i64 % 2 == 0 {
                entity.effects |= 64;
            }
        })?;
        if count as i64 % 2 == 0 {
            set_addon_number(monster.monster.game, &missile, "frags", 1.0)?;
        }
        let touch = monster
            .monster
            .game
            .named
            .touch(&format!("{ORB_PREFIX}:missile_touch"))?;
        monster
            .monster
            .game
            .update_entity(&missile, |entity| entity.touch = Some(touch))?;
        monster.monster.game.set_body(
            &missile,
            &BodyPatch {
                velocity: Some(vscale(aim, speed + spread * 100.0)),
                bounds: Some(POINT),
                ..Default::default()
            },
        )?;
        count -= 1.0;
    }
    Ok(())
}

fn orb_death_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let solid = game.entity_ref(other).map(|entity| entity.solid);
    let trigger = solid.is_some_and(|solid| solid == crate::q1::foundation::types::Q1Solid::Trigger);
    let dead_bbox =
        solid.is_some_and(|solid| solid == crate::q1::foundation::types::Q1Solid::Bbox) && game.health(other) == 0.0;
    if trigger || dead_bbox {
        return Ok(());
    }
    let origin = game.body(id)?.origin;
    addon_emit(
        game,
        Q1AddonEvent::ColoredExplosion {
            origin,
            color_start: 244,
            color_length: 3,
        },
    )?;
    let world = game.world.clone();
    game.radius_damage(id, Some(id), 100.0, world.as_ref(), None, "");
    game.remove(id)
}

fn orb_missile_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
    if owner.as_ref().is_some_and(|owner| same_actor(owner, other)) {
        return Ok(());
    }
    let classname = game.host.classname(other);
    if classname == "monster_orb" || classname == "monster_lava_man" || classname == "monster_super_shambler" {
        return game.remove(id);
    }
    let own_class = game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    let trigger = game
        .entity_ref(other)
        .is_some_and(|entity| entity.solid == crate::q1::foundation::types::Q1Solid::Trigger);
    if classname == own_class || trigger {
        return Ok(());
    }
    let origin = game.body(id)?.origin;
    if game.host.contents(origin) == Q1Contents::Sky {
        return game.remove(id);
    }
    if game
        .host
        .combat
        .read(other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        game.effect(Q1Effect::Blood, origin, Some(other), 18);
        let attacker = game.entity_ref(id).and_then(|entity| entity.owner.clone());
        game.damage(other, Some(id), attacker.as_ref(), 18.0, &Q1DamageParams::default());
    } else if game.entity_ref(id).map(|entity| entity.number("frags")).unwrap_or(0.0) != 0.0 {
        game.effect(Q1Effect::KnightSpike, origin, None, 1);
    }
    game.remove(id)
}

fn spawn_orb(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::spawn_new(game, ORB_PREFIX, id, &ORB_SPEC)?;
    let owned = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.actor.clone());
    if let Some(owned) = owned {
        monster.monster.game.host.combat.set_health(&owned, 300.0)?;
    }
    let pain = monster.monster.game.named.pain(&format!("{ORB_PREFIX}:monster_pain"))?;
    let die = monster.monster.game.named.die(&format!("{ORB_PREFIX}:monster_die"))?;
    let path_end = monster
        .monster
        .game
        .named
        .action(&format!("{ORB_PREFIX}:monster_stand"))?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.max_health = 300.0;
            entity.pain = Some(pain);
            entity.die = Some(die);
            entity.path_end = Some(path_end);
        })?;
    set_addon_number(monster.monster.game, &monster.monster.id.clone(), "combat_style", 1.0)?;
    let context = crate::q1::addons::context::Q1AddonContext::new(crate::q1::addons::context::Q1AddonProgram::Mg3);
    crate::q1::addons::monsters::startup::init_mg3_monster(
        &mut monster,
        &context,
        "progs/teleporter_eye_blink.mdl",
        2,
        2,
    )?;
    monster.finish()
}

/// Registers orbs (`registerOrb`).
pub fn register_orb(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    register_mg3_monster_source(
        ORB_PREFIX,
        frames(),
        orb_actions(),
        Mg3SourceHooks {
            start: Some(orb_start),
            use_monster: Some(mg3_use_mapped),
            try_attack: Some(orb_try_attack),
            pain: Some(orb_pain),
            die: Some(orb_die),
            ..Default::default()
        },
        orb_load_controller,
        store_monster_controller,
    );
    register_boss_controllers(game, ORB_PREFIX, "monster_orb", spawn_orb)?;
    game.named.register(
        &format!("{ORB_PREFIX}:death_touch"),
        Q1CallbackHandlers {
            touch: Some(orb_death_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{ORB_PREFIX}:missile_touch"),
        Q1CallbackHandlers {
            touch: Some(orb_missile_touch as Q1TouchHandler),
            ..Default::default()
        },
    )
}

/// Steers an orb missile at the monster enemy (`launchOrbMissile`).
pub fn launch_orb_missile(
    monster: &mut Mg3Monster,
    missile: &ActorId,
    speed: f64,
    accuracy: f64,
) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let enemy = monster.monster.monster.enemy.clone();
    let target = enemy
        .as_ref()
        .and_then(|enemy| monster.monster.game.host.bodies.read(enemy));
    let angles = monster.monster.game.body(&id)?.angles;
    let basis = monster.monster.game.make_vectors(angles);
    let origin = monster.monster.game.body(missile)?.origin;
    let target_origin = match target.as_ref() {
        Some(target) => vadd(
            vadd(target.origin, target.bounds.min),
            vscale(vsub(target.bounds.max, target.bounds.min), 0.7),
        ),
        None => ZERO,
    };
    let delta = vsub(target_origin, origin);
    let travel = f64::from(length(delta)) / speed;
    let velocity = match target.as_ref() {
        Some(target) => Vec3 {
            x: target.velocity.x,
            y: target.velocity.y,
            z: 0.0,
        },
        None => ZERO,
    };
    let jitter_up = accuracy * (monster.monster.game.host.random() - 0.5);
    let jitter_right = accuracy * (monster.monster.game.host.random() - 0.5);
    let right = monster.monster.game.basis.right;
    let direction = vadd(
        vadd(
            normalize(vadd(delta, vscale(velocity, travel))),
            vscale(basis.up, jitter_up),
        ),
        vscale(right, jitter_right),
    );
    monster.monster.game.set_body(
        missile,
        &BodyPatch {
            velocity: Some(vscale(direction, speed)),
            angles: Some(Vec3 {
                x: 0.0,
                y: yaw_for(direction) as f32,
                z: 0.0,
            }),
            ..Default::default()
        },
    )?;
    monster.monster.game.schedule(missile, 5.0, "SUB_Remove")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg3);
        register_orb(game).expect("orb");
        guard
    }

    #[test]
    fn spawn_sets_orb_defaults() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let orb = game.create("monster_orb", None, None).expect("orb");
        game.spawn_entity(&orb, None).expect("spawn");
        let entity = game.entity_ref(&orb).expect("entity");
        assert_eq!(entity.max_health, 300.0);
        assert_eq!(entity.number("combat_style"), 1.0);
        assert!(entity.pain.is_some() && entity.die.is_some());
    }

    #[test]
    fn missile_touch_spares_its_owner() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let orb = game.create("monster_orb", None, None).expect("orb");
        game.spawn_entity(&orb, None).expect("spawn");
        let rock = game.create("rock", None, None).expect("rock");
        game.update_entity(&rock, |entity| entity.owner = Some(orb.clone()))
            .expect("owner");
        orb_missile_touch(&mut game, &rock, &orb, None, None).expect("touch");
        assert!(game.entity_ref(&rock).is_some());
    }

    #[test]
    fn launch_orb_missile_aims_at_enemy() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let orb = game.create("monster_orb", None, None).expect("orb");
        game.spawn_entity(&orb, None).expect("spawn");
        let missile = game.create("rock", None, None).expect("missile");
        game.set_body(
            &missile,
            &BodyPatch {
                origin: Some(Vec3 {
                    x: 100.0,
                    y: 0.0,
                    z: 0.0,
                }),
                ..Default::default()
            },
        )
        .expect("missile origin");
        let mut monster = Mg3Monster::load(&mut game, &orb).expect("load");
        launch_orb_missile(&mut monster, &missile, 500.0, 0.0).expect("launch");
        monster.finish().expect("finish");
        let body = game.body(&missile).expect("body");
        assert_eq!(body.velocity.x, -500.0);
        assert_eq!(body.velocity.y, 0.0);
    }
}
