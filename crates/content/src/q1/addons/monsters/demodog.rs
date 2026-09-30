//! Q1 mg3 demodog (`src/content/q1/addons/monsters/demodog.ts`).
//!
//! `quakec_mg3/monsters/mg3_demodog.qc` and `monsters.qc`.
//! GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{set_addon_number, set_addon_vector, Q1AddonContext};
use crate::q1::base::animation::MonsterAi;
use crate::q1::base::monsters::BaseMonsterState;
use crate::q1::base::projectiles::{throw_gib, throw_head};
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1StateExtension, Q1TouchHandler};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity::{Q1MonsterSpecies, Q1ProjectileKind};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    length, vadd, vscale, yaw_for, Q1Effect, Q1MoveType, Q1Solid, Q1SoundChannel, POINT, ZERO,
};
use crate::q1::{q1_error, Q1Error};

use super::ai::{
    clone_monster_controller, register_mg3_monster_callbacks, register_mg3_monster_source, Mg3ActionHandler,
    Mg3Monster, Mg3SourceHooks,
};
use super::demodog_frames::demodog_frames;
use super::startup::{init_mg3_monster, mg3_use_mapped, register_mg3_monster_startup, start_mg3_monster};

/// Demodog callback prefix.
const DEMODOG_PREFIX: &str = "mg3:demodog";

/// Demodog spawn defaults (`spec`).
const DEMODOG_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Dog,
    kill_string: None,
    classnames: &["monster_demodog"],
    model: "dog_explosive",
    head: Some("h_dog"),
    health: 25.0,
    gib_health: -35.0,
    gibs: &["gib3", "gib3", "gib3"],
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
    stand: "demodog_stand1",
    walk: "demodog_walk1",
    run: "demodog_run1",
    sight: "dog/dsight.wav",
    missile: Some("demodog_leap1"),
    melee: true,
    movement: MonsterMovement::Walk,
};

/// Demodog frame actions (`actions`).
fn demodog_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn bite(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            if monster.monster.monster.enemy.is_none() {
                return Ok(());
            }
            monster.ai(MonsterAi::Charge, 10.0)?;
            monster.monster.melee(100.0, 8.0, 3, true)?;
            Ok(())
        }
        fn jump(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let body = monster.monster.game.body(&id)?;
            let touch = monster
                .monster
                .game
                .named
                .touch(&format!("{DEMODOG_PREFIX}:jump_touch"))?;
            monster.monster.game.update_entity(&id, |entity| {
                entity.touch = Some(touch);
                entity.movement_flags &= !512;
            })?;
            let forward = monster.monster.game.make_vectors(body.angles).forward;
            monster.monster.game.set_body(
                &id,
                &BodyPatch {
                    origin: Some(vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 })),
                    velocity: Some(vadd(
                        vscale(forward, 300.0),
                        Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 200.0,
                        },
                    )),
                    ground: Some(None),
                    ..Default::default()
                },
            )
        }
        HashMap::from([
            (String::from("demodog_bite"), bite as Mg3ActionHandler),
            (String::from("demodog_jump"), jump as Mg3ActionHandler),
        ])
    })
}

fn demodog_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    _classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = crate::q1::base::creatures::monster_controller(game, id, "monster_demodog")?;
    Ok((controller, &DEMODOG_SPEC))
}

fn demodog_start(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let context = Q1AddonContext::new(crate::q1::addons::context::Q1AddonProgram::Mg3);
    start_mg3_monster(monster, &context)
}

fn demodog_try_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    let Some(enemy) = monster.monster.monster.enemy.clone() else {
        return Ok(false);
    };
    let Some(target) = monster.monster.game.host.bodies.read(&enemy) else {
        return Ok(false);
    };
    let range = monster
        .monster
        .game
        .world
        .clone()
        .and_then(|world| {
            monster
                .monster
                .game
                .entity_ref(&world)
                .map(|entity| entity.number("enemy_range"))
        })
        .unwrap_or(0.0);
    if range == 0.0 {
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = crate::q1::foundation::entity::Q1AttackState::Melee;
            })?;
        return Ok(true);
    }
    let body = monster.monster.game.body(&monster.monster.id.clone())?;
    let height = f64::from(target.bounds.max.z - target.bounds.min.z);
    if f64::from(body.origin.z + body.bounds.min.z) > f64::from(target.origin.z + target.bounds.min.z) + height * 0.75
        || f64::from(body.origin.z + body.bounds.max.z)
            < f64::from(target.origin.z + target.bounds.min.z) + height * 0.25
    {
        return Ok(false);
    }
    let distance = f64::from(target.origin.x - body.origin.x).hypot(f64::from(target.origin.y - body.origin.y));
    if distance < 80.0 || distance > 150.0 {
        return Ok(false);
    }
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.attack_state = crate::q1::foundation::entity::Q1AttackState::Missile;
        })?;
    Ok(true)
}

fn demodog_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    monster.play("demodog_atta1")
}

fn demodog_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, _damage: f64) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    monster.retaliate(attacker.as_ref())?;
    monster.monster.game.sound(
        &monster.monster.id.clone(),
        "dog/dpain1.wav",
        Q1SoundChannel::Voice,
        1.0,
        1.0,
    )?;
    let rolled = monster.monster.game.host.random();
    monster.play(if rolled > 0.5 {
        "demodog_pain1"
    } else {
        "demodog_painb1"
    })
}

fn grenade(monster: &mut Mg3Monster, vertical: bool) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let mut velocity = vscale(
        Vec3 {
            x: (100.0 * (monster.monster.game.host.random() * 2.0 - 1.0)) as f32,
            y: (100.0 * (monster.monster.game.host.random() * 2.0 - 1.0)) as f32,
            z: (200.0 + 100.0 * monster.monster.game.host.random()) as f32,
        },
        1.5,
    );
    if vertical {
        velocity = Vec3 {
            x: 0.0,
            y: 0.0,
            z: velocity.z,
        };
    }
    let angles = monster.monster.game.body(&id).map(|body| body.angles)?;
    velocity = vadd(
        velocity,
        vscale(monster.monster.game.make_vectors(angles).forward, 100.0),
    );
    monster
        .monster
        .game
        .sound(&id, "weapons/grenade.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let grenade = monster.monster.game.create("grenade", None, None)?;
    monster.monster.game.update_entity(&grenade, |entity| {
        entity.owner = Some(id.clone());
        entity.movement = Q1MoveType::Bounce;
        entity.solid = Q1Solid::Bbox;
        entity.projectile = Some(Q1ProjectileKind::Grenade);
        entity.damage = 60.0;
        entity.model = String::from("progs/grenade.mdl");
        entity.angular_velocity = Vec3 {
            x: 300.0,
            y: 300.0,
            z: 300.0,
        };
    })?;
    set_addon_vector(monster.monster.game, &grenade, "oldorigin", velocity)?;
    let origin = monster.monster.origin()?;
    monster.monster.game.set_body(
        &grenade,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            bounds: Some(POINT),
            angles: Some(Vec3 {
                x: (f64::from(velocity.z).atan2(f64::from(velocity.x).hypot(f64::from(velocity.y))) * 180.0
                    / std::f64::consts::PI) as f32,
                y: yaw_for(velocity) as f32,
                z: 0.0,
            }),
            ..Default::default()
        },
    )?;
    let touch = monster
        .monster
        .game
        .named
        .touch(&format!("{DEMODOG_PREFIX}:grenade_touch"))?;
    monster.monster.game.update_entity(&grenade, |entity| {
        entity.touch = Some(touch);
    })?;
    monster.monster.game.link(&grenade)?;
    let explode = monster
        .monster
        .game
        .named
        .action(&format!("{DEMODOG_PREFIX}:grenade_explode"))?;
    let delay = 2.5 + 0.25 * (monster.monster.game.host.random() * 2.0 - 1.0);
    monster.monster.game.schedule(&grenade, delay, &explode)
}

fn demodog_die(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
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
    let id = monster.monster.id.clone();
    monster.monster.game.set_health(&id, -50.0)?;
    grenade(monster, false)?;
    grenade(monster, false)?;
    grenade(monster, false)?;
    if monster.monster.game.options().skill > 2 && monster.monster.game.host.random() > 0.7 {
        grenade(monster, true)?;
    }
    monster
        .monster
        .game
        .sound(&id, "player/udeath.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    for _ in 0..3 {
        let origin = monster.monster.origin()?;
        throw_gib(monster.monster.game, origin, "gib3", -50.0)?;
    }
    throw_head(monster.monster.game, &id, "h_dog", -50.0)?;
    Ok(())
}

fn demodog_jump_touch(monster: &mut Mg3Monster, other: &ActorId) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    if monster.monster.game.health(&id) <= 0.0 {
        return Ok(());
    }
    if monster
        .monster
        .game
        .host
        .combat
        .read(other)
        .is_some_and(|combat| combat.can_take_damage)
        && monster.monster.monster.attack_finished < monster.monster.game.time
        && f64::from(length(
            monster.monster.game.body(&id).map(|body| body.velocity).unwrap_or(ZERO),
        )) > 300.0
    {
        let damage = 10.0 + 10.0 * monster.monster.game.host.random();
        monster
            .monster
            .game
            .damage(other, Some(&id), Some(&id), damage, &Q1DamageParams::default());
        if monster.monster.game.is_player(other) {
            monster
                .monster
                .game
                .damage(&id, Some(other), Some(other), 200.0, &Q1DamageParams::default());
            return Ok(());
        }
        monster.monster.monster.attack_finished = monster.monster.game.time + 0.5;
    }
    if !monster.monster.game.host.check_bottom(&id) {
        if monster
            .monster
            .game
            .entity_ref(&id)
            .map(|entity| entity.movement_flags)
            .unwrap_or(0)
            & 512
            != 0
        {
            monster.monster.game.update_entity(&id, |entity| {
                entity.touch = None;
            })?;
            monster.monster.controller.next_frame = String::from("demodog_leap1");
            return monster.monster.delay(0.1);
        }
        return Ok(());
    }
    monster.monster.game.update_entity(&id, |entity| {
        entity.wait = 0.0;
        entity.touch = None;
    })?;
    monster.monster.controller.next_frame = monster.monster.spec.run.to_string();
    monster.monster.delay(0.1)
}

fn jump_touch_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let other = other.clone();
    let mut monster = Mg3Monster::load(game, id)?;
    demodog_jump_touch(&mut monster, &other)?;
    monster.finish()
}

fn explode(game: &mut Q1EntityServices, id: &ActorId, ignore: Option<&ActorId>) -> Result<(), Q1Error> {
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.radius_damage(id, entity.owner.as_ref(), entity.damage, ignore, None, "");
    let origin = game.body(id).map(|body| body.origin)?;
    game.effect(Q1Effect::Explosion, origin, None, 1);
    game.remove(id)
}

fn grenade_explode_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let ignore = game.world.clone();
    explode(game, id, ignore.as_ref())
}

fn grenade_touch_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.owner.as_ref().is_some_and(|owner| same_actor(other, owner)) {
        return Ok(());
    }
    let target = game.entity_ref(other).cloned();
    if target.as_ref().is_some_and(|target| target.aimed_damage) || game.is_player(other) {
        if target
            .as_ref()
            .is_some_and(|target| target.classname == "monster_boss" || target.classname == "monster_oldone_new")
        {
            game.damage(
                other,
                Some(id),
                entity.owner.as_ref(),
                entity.damage,
                &Q1DamageParams::default(),
            );
            return explode(game, id, Some(other));
        }
        let ignore = game.world.clone();
        return explode(game, id, ignore.as_ref());
    }
    if f64::from(length(game.body(id).map(|body| body.velocity).unwrap_or(ZERO))) == 0.0 {
        game.update_entity(id, |entity| {
            entity.angular_velocity = ZERO;
        })?;
    }
    if game
        .entity_ref(id)
        .map(|entity| entity.number("attack_finished"))
        .unwrap_or(0.0)
        < game.time
    {
        game.sound(id, "weapons/bounce.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    }
    let finished = game.time + 0.1;
    set_addon_number(game, id, "attack_finished", finished)
}

fn spawn_demodog(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let context = Q1AddonContext::new(crate::q1::addons::context::Q1AddonProgram::Mg3);
    let mut monster = Mg3Monster::spawn_new(game, DEMODOG_PREFIX, id, &DEMODOG_SPEC)?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.classname = String::from("monster_dog");
            entity.wait = 0.0;
        })?;
    set_addon_number(monster.monster.game, &monster.monster.id.clone(), "aflag", 1.0)?;
    monster.monster.game.set_health(&monster.monster.id.clone(), 25.0)?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.max_health = 25.0;
        })?;
    let pain = monster
        .monster
        .game
        .named
        .pain(&format!("{DEMODOG_PREFIX}:monster_pain"))?;
    let die = monster
        .monster
        .game
        .named
        .die(&format!("{DEMODOG_PREFIX}:monster_die"))?;
    let path_end = monster
        .monster
        .game
        .named
        .action(&format!("{DEMODOG_PREFIX}:monster_stand"))?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.pain = Some(pain);
            entity.die = Some(die);
            entity.path_end = Some(path_end);
        })?;
    set_addon_number(monster.monster.game, &monster.monster.id.clone(), "allowPathFind", 1.0)?;
    set_addon_number(monster.monster.game, &monster.monster.id.clone(), "combat_style", 2.0)?;
    init_mg3_monster(&mut monster, &context, "progs/dog_explosive.mdl", 1, 2)?;
    monster.finish()
}

/// Register mg3 demodogs (`registerMg3Demodog`). Controllers persist
/// through the base creature store; the extension only carries the
/// clone hook.
pub fn register_mg3_demodog(_context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    struct Extension;

    impl Q1StateExtension for Extension {
        fn id(&self) -> &str {
            DEMODOG_PREFIX
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
            clone_monster_controller(game, source, target, DEMODOG_PREFIX)
        }
    }

    register_mg3_monster_source(
        DEMODOG_PREFIX,
        demodog_frames(),
        demodog_actions(),
        Mg3SourceHooks {
            start: Some(demodog_start),
            use_monster: Some(mg3_use_mapped),
            try_attack: Some(demodog_try_attack),
            melee_attack: Some(demodog_melee),
            pain: Some(demodog_pain),
            die: Some(demodog_die),
            ..Default::default()
        },
        demodog_load_controller,
        crate::q1::base::creatures::store_monster_controller,
    );
    register_mg3_monster_callbacks(game, DEMODOG_PREFIX)?;
    game.named.register(
        &format!("{DEMODOG_PREFIX}:jump_touch"),
        Q1CallbackHandlers {
            touch: Some(jump_touch_handler as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    register_mg3_monster_startup(game, DEMODOG_PREFIX)?;
    game.named.register(
        &format!("{DEMODOG_PREFIX}:grenade_explode"),
        Q1CallbackHandlers {
            action: Some(grenade_explode_handler as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{DEMODOG_PREFIX}:grenade_touch"),
        Q1CallbackHandlers {
            touch: Some(grenade_touch_handler as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("monster_demodog", spawn_demodog)?;
    game.register_state_extension(Box::new(Extension))?;
    Ok(())
}
