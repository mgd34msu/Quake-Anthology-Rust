//! Q1 mg3 lava man (`src/content/q1/addons/monsters/heavy/lava-man.ts`).
//!
//! `quakec_mg3/monsters/mg3_lavaman.qc`. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{addon_cvar, set_addon_vector, Q1AddonContext};
use crate::q1::base::projectiles::sprite_explosion;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1TouchHandler, Q1UseHandler};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, Q1Effect, Q1Event, Q1MoveType, Q1Powerup, Q1Solid, Q1SoundChannel,
    Q1TraceRequest, POINT, ZERO,
};
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::{q1_error, Q1Error};

use super::runtime::{heavy_install_callbacks, HEAVY_PREFIX};
use crate::q1::addons::monsters::ai::{Mg3ActionHandler, Mg3Monster};

/// Lava-man spawn defaults (`lavaManDefinition.spec`).
pub const LAVA_MAN_SPEC: MonsterSpecies = MonsterSpecies {
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
};

fn lava_check_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    let id = monster.monster.id.clone();
    monster.monster.face()?;
    let origin = monster.monster.origin()?;
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let trace = monster.monster.game.host.trace(&Q1TraceRequest {
        start: vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 64.0,
            },
        ),
        end: target,
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    let enemy = monster.monster.monster.enemy.clone();
    if trace.in_open && trace.in_water
        || enemy.is_none()
        || trace
            .actor
            .as_ref()
            .is_none_or(|actor| !same_actor(actor, enemy.as_ref().expect("enemy")))
        || monster.monster.game.time < monster.monster.monster.attack_finished
    {
        return Ok(false);
    }
    monster.play("lavaman_fire1")?;
    let delay = 1.0 + monster.monster.game.host.random();
    monster.attack_finished(delay);
    Ok(true)
}

fn hunt(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let enemy = monster.monster.monster.enemy.clone();
    if enemy
        .as_ref()
        .is_none_or(|enemy| monster.monster.game.health(enemy) <= 0.0)
    {
        let actors = monster.monster.game.host.actors.observations();
        let current = enemy
            .as_ref()
            .and_then(|enemy| actors.iter().position(|actor| same_actor(&actor.id, enemy)));
        let player = actors
            .iter()
            .skip(current.map(|index| index + 1).unwrap_or(0))
            .find(|actor| monster.monster.game.is_player(&actor.id))
            .map(|actor| actor.id.clone())
            .or_else(|| monster.monster.game.world.clone());
        let target = player
            .as_ref()
            .and_then(|player| monster.monster.game.host.bodies.read(player).map(|body| body.origin))
            .unwrap_or(ZERO);
        let origin = monster.monster.origin()?;
        let trace = monster.monster.game.host.trace(&Q1TraceRequest {
            start: vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 96.0,
                },
            ),
            end: target,
            bounds: POINT,
            ignore: monster.monster.game.world.clone(),
            monsters: false,
            missile: false,
        });
        if trace.fraction == 1.0 {
            monster.monster.monster.enemy = player;
        }
    }
    if let Some(enemy) = monster.monster.monster.enemy.clone() {
        monster.monster.face()?;
        monster.monster.game.update_entity(&id, |entity| {
            entity
                .references
                .insert(String::from("movetarget"), Some(enemy.clone()));
            entity.references.insert(String::from("goalentity"), Some(enemy));
        })?;
    }
    Ok(())
}

fn think(monster: &mut Mg3Monster, mode: crate::q1::base::animation::MonsterAi) -> Result<(), Q1Error> {
    if monster.monster.monster.enemy.is_some() {
        lava_check_attack(monster)?;
    } else {
        hunt(monster)?;
    }
    monster.ai(
        mode,
        if mode == crate::q1::base::animation::MonsterAi::Stand {
            0.0
        } else {
            2.0
        },
    )
}

fn launch(monster: &mut Mg3Monster, hand: u32) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let basis = monster.make_vectors()?;
    let origin = monster.monster.origin()?;
    let muzzle = vadd(
        vadd(
            vadd(origin, vscale(basis.forward, 40.0)),
            vscale(basis.right, if hand == 1 { 65.0 } else { -65.0 }),
        ),
        vscale(basis.up, 90.0),
    );
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let delta = vsub(target, muzzle);
    let duration = (f64::from(length(delta)) / 380.0).clamp(1.0, 1.75);
    let direction = normalize(delta);
    let ball = monster.monster.game.create("lavaman_ball", None, None)?;
    let touch = monster
        .monster
        .game
        .named
        .touch(&format!("{HEAVY_PREFIX}:lavaman_touch"))?;
    monster.monster.game.update_entity(&ball, |entity| {
        entity.owner = Some(id.clone());
        entity.movement = Q1MoveType::Bounce;
        entity.solid = Q1Solid::Bbox;
        entity.model = String::from("progs/lavaball.mdl");
        entity.touch = Some(touch);
        entity.angular_velocity = Vec3 {
            x: 200.0,
            y: 100.0,
            z: 300.0,
        };
    })?;
    let angles = velocity_angles(direction);
    monster.monster.game.set_body(
        &ball,
        &BodyPatch {
            origin: Some(muzzle),
            angles: Some(angles),
            bounds: Some(POINT),
            velocity: Some(vadd(
                vscale(direction, 600.0 * duration),
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 200.0 * duration as f32,
                },
            )),
            ..Default::default()
        },
    )?;
    monster.monster.game.link(&ball)?;
    let remove = monster.monster.game.named.action("SUB_Remove")?;
    monster.monster.game.schedule(&ball, 6.0, &remove)?;
    monster
        .monster
        .game
        .sound(&id, "boss1/throw.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    if monster
        .monster
        .monster
        .enemy
        .as_ref()
        .is_none_or(|enemy| monster.monster.game.health(enemy) <= 0.0)
    {
        monster.play("lavaman_idle1")?;
    }
    Ok(())
}

fn awake(monster: &mut Mg3Monster, activator: Option<&ActorId>) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Fly;
        entity.aimed_damage = true;
    })?;
    monster.monster.game.set_damageable(&id, true)?;
    let yaw = monster.monster.game.body(&id).map(|body| body.angles.y)?;
    let yaw_speed = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("yaw_speed"))
        .unwrap_or(0.0);
    let bounds = monster.monster.spec.bounds;
    monster.monster.game.update_entity(&id, |entity| {
        entity.movement_flags |= 32;
        entity.ideal_yaw = f64::from(yaw);
        entity.yaw_speed = if yaw_speed == 0.0 { 20.0 } else { yaw_speed };
        entity.model = String::from("progs/lavaman.mdl");
    })?;
    monster.monster.game.set_bounds(&id, bounds)?;
    set_addon_vector(
        monster.monster.game,
        &id,
        "view_ofs",
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 48.0,
        },
    )?;
    let skill = addon_cvar(monster.monster.game, "skill")?;
    monster.monster.game.set_health(&id, 1250.0 + 250.0 * skill)?;
    heavy_install_callbacks(monster)?;
    let force_death = monster
        .monster
        .game
        .named
        .use_callback(&format!("{HEAVY_PREFIX}:lavaman_force_death"))?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.use_callback = Some(force_death);
    })?;
    let origin = monster.monster.origin()?;
    monster.monster.game.effect(
        Q1Effect::LavaSplash,
        vsub(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 50.0,
            },
        ),
        None,
        1,
    );
    if let Some(activator) = activator {
        if monster.monster.game.is_player(activator)
            && monster
                .monster
                .game
                .player_ref(activator)
                .and_then(|player| player.powerups.get(&Q1Powerup::Invisibility).copied())
                .unwrap_or(0.0)
                <= monster.monster.game.time
            && monster
                .monster
                .game
                .entity_ref(activator)
                .map(|entity| entity.movement_flags)
                .unwrap_or(0)
                & 128
                == 0
        {
            monster.monster.monster.enemy = Some(activator.clone());
        }
    }
    monster.play("lavaman_rise1")
}

fn lavaman_awake_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    let mut monster = Mg3Monster::load(game, id)?;
    awake(&mut monster, activator.as_ref())?;
    monster.finish()
}

fn lavaman_dead_use(
    _game: &mut Q1EntityServices,
    _id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    Ok(())
}

fn lavaman_force_death_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    let mut monster = Mg3Monster::load(game, id)?;
    monster
        .monster
        .game
        .set_damageable(&monster.monster.id.clone(), false)?;
    monster.monster.game.set_health(&monster.monster.id.clone(), 0.0)?;
    monster.monster.game.killed_monsters += 1;
    let (total, found) = (
        monster.monster.game.total_monsters,
        monster.monster.game.killed_monsters,
    );
    monster.monster.game.host.emit(Q1Event::MonsterKilled {
        actor: monster.monster.id.clone(),
        total,
        found,
    });
    monster
        .monster
        .game
        .use_targets(&monster.monster.id.clone(), activator.as_ref())?;
    monster.monster.controller.counted_death = true;
    let dead = monster
        .monster
        .game
        .named
        .use_callback(&format!("{HEAVY_PREFIX}:lavaman_dead_use"))?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.use_callback = Some(dead);
        })?;
    monster.play("lavaman_death1")?;
    monster.finish()
}

fn lavaman_touch(
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
    if entity.owner.as_ref().is_some_and(|owner| same_actor(owner, other)) {
        return Ok(());
    }
    if game.host.contents(game.body(id).map(|body| body.origin)?) == Q1Contents::Sky {
        return game.remove(id);
    }
    if game.health(other) != 0.0 {
        let shambler = game
            .entity_ref(other)
            .is_some_and(|other| other.classname == "monster_shambler");
        game.damage(
            other,
            Some(id),
            entity.owner.as_ref(),
            if shambler { 20.0 } else { 40.0 },
            &Q1DamageParams::default(),
        );
    }
    game.radius_damage(id, entity.owner.as_ref(), 40.0, Some(other), None, "");
    let body = game.body(id)?;
    game.set_origin(id, vsub(body.origin, vscale(normalize(body.velocity), 8.0)))?;
    sprite_explosion(game, id)
}

/// Register lava-man named callbacks (`lavaManDefinition.callbacks`).
pub fn register_lava_man_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        &format!("{HEAVY_PREFIX}:lavaman_awake"),
        Q1CallbackHandlers {
            use_callback: Some(lavaman_awake_use as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{HEAVY_PREFIX}:lavaman_dead_use"),
        Q1CallbackHandlers {
            use_callback: Some(lavaman_dead_use as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{HEAVY_PREFIX}:lavaman_force_death"),
        Q1CallbackHandlers {
            use_callback: Some(lavaman_force_death_use as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{HEAVY_PREFIX}:lavaman_touch"),
        Q1CallbackHandlers {
            touch: Some(lavaman_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Lava-man frame actions (`lavaManDefinition.actions`).
pub fn lava_man_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        use crate::q1::base::animation::MonsterAi;
        fn stand(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            think(monster, MonsterAi::Stand)
        }
        fn walk(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            think(monster, MonsterAi::Walk)
        }
        fn run(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            think(monster, MonsterAi::Run)
        }
        fn missile1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            launch(monster, 1)
        }
        fn missile2(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            launch(monster, 2)
        }
        fn death9(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.game.sound(
                &monster.monster.id.clone(),
                "boss1/out1.wav",
                Q1SoundChannel::Body,
                1.0,
                1.0,
            )?;
            let origin = monster.monster.origin()?;
            monster.monster.game.effect(
                Q1Effect::LavaSplash,
                vsub(
                    origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 50.0,
                    },
                ),
                None,
                1,
            );
            Ok(())
        }
        fn death10(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster.monster.game.remove(&id)
        }
        HashMap::from([
            (String::from("lavaman_stand"), stand as Mg3ActionHandler),
            (String::from("lavaman_walk"), walk as Mg3ActionHandler),
            (String::from("lavaman_run"), run as Mg3ActionHandler),
            (String::from("lavaman_missile(1)"), missile1 as Mg3ActionHandler),
            (String::from("lavaman_missile(2)"), missile2 as Mg3ActionHandler),
            (String::from("mg3_lavaman:lavaman_death9"), death9 as Mg3ActionHandler),
            (String::from("mg3_lavaman:lavaman_death10"), death10 as Mg3ActionHandler),
        ])
    })
}

/// Spawn a lava man (`lavaManDefinition.spawn`).
pub fn lava_man_spawn(monster: &mut Mg3Monster, _context: &Q1AddonContext) -> Result<(), Q1Error> {
    if monster.monster.game.options().deathmatch != 0 {
        let id = monster.monster.id.clone();
        return monster.monster.game.remove(&id);
    }
    monster.monster.game.total_monsters += 1;
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.spawnflags |= 16384;
    })?;
    let entity = monster
        .monster
        .game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if !entity.targetname.is_empty() {
        let awake = monster
            .monster
            .game
            .named
            .use_callback(&format!("{HEAVY_PREFIX}:lavaman_awake"))?;
        monster.monster.game.update_entity(&id, |entity| {
            entity.use_callback = Some(awake);
        })?;
        return Ok(());
    }
    let activator = entity.activator.clone();
    awake(monster, activator.as_ref())
}

/// Attempt a lava-man attack (`lavaManDefinition.attack`).
pub fn lava_man_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    lava_check_attack(monster)
}

/// Lava-man melee (`lavaManDefinition.melee`).
pub fn lava_man_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    monster.play("lavaman_fire1")
}

/// React to lava-man pain (`lavaManDefinition.pain`).
pub fn lava_man_pain(monster: &mut Mg3Monster, _attacker: Option<&ActorId>, _damage: f64) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    if monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.count)
        .unwrap_or(0.0)
        == 0.0
    {
        monster.monster.game.update_entity(&id, |entity| {
            entity.count += 1.0;
        })?;
        monster.monster.monster.pain_finished = monster.monster.game.time + 2.0;
        return monster.play("lavaman_shocka1");
    }
    if monster.monster.monster.pain_finished > monster.monster.game.time || monster.monster.game.host.random() >= 0.05 {
        return Ok(());
    }
    monster.monster.monster.pain_finished = monster.monster.game.time + 2.0;
    monster.play("lavaman_shocka1")
}

/// Die as a lava man (`lavaManDefinition.die`).
pub fn lava_man_die(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    monster.play("lavaman_death1")
}
