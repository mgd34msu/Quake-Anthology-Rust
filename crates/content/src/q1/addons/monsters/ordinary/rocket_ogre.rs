//! Q1 mg3 rocket ogre (`src/content/q1/addons/monsters/ordinary/rocket-ogre.ts`).
//!
//! `quakec_mg3/monsters/ogre.qc` rocket variant. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::same_actor;
use qa_core::math::Vec3;

use crate::q1::addons::context::Q1AddonContext;
use crate::q1::base::animation::MonsterFrame;
use crate::q1::base::frames::monster_frame;
use crate::q1::base::projectiles::{create_missile, sprite_explosion};
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{length, normalize, vadd, vscale, vsub, Q1Effect, Q1SoundChannel};
use crate::q1::{q1_error, Q1Error};

use crate::q1::addons::monsters::ai::Mg3Monster;

/// Rocket-ogre frame overrides (`rocketOgreFrames`). Sound and action
/// operations are filtered out; the play hook replays the rocket
/// behavior instead.
pub fn rocket_ogre_frames() -> &'static HashMap<String, MonsterFrame> {
    static FRAMES: OnceLock<HashMap<String, MonsterFrame>> = OnceLock::new();
    FRAMES.get_or_init(|| {
        let mut frames = HashMap::new();
        for name in [
            "ogre_stand5",
            "ogre_walk3",
            "ogre_run1",
            "ogre_nail1",
            "ogre_nail4",
            "ogre_nail5",
            "ogre_die3",
            "ogre_bdie3",
        ] {
            let frame = monster_frame(name).unwrap_or_else(|| panic!("Missing ogre frame {name}"));
            let operations: Vec<_> = frame
                .operations
                .iter()
                .filter(|op| {
                    !matches!(
                        op,
                        crate::q1::base::animation::MonsterOperation::Sound { .. }
                            | crate::q1::base::animation::MonsterOperation::Action { .. }
                    )
                })
                .copied()
                .collect();
            frames.insert(
                String::from(name),
                MonsterFrame {
                    frame: frame.frame,
                    next: frame.next,
                    operations: Box::leak(operations.into_boxed_slice()),
                },
            );
        }
        frames
    })
}

/// Run rocket-ogre frame behavior (`rocketOgreFrame`).
pub fn rocket_ogre_frame(monster: &mut Mg3Monster, name: &str) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    if name == "ogre_nail1" || name == "ogre_nail5" {
        let max = monster
            .monster
            .game
            .entity_ref(&id)
            .map(|entity| entity.number("projectiles_max"))
            .unwrap_or(0.0);
        monster.monster.game.update_entity(&id, |entity| {
            entity.fields.insert(String::from("projectiles"), max.to_string());
        })?;
    }
    if name == "ogre_nail4" {
        let snapshot = BaseMonsterSnapshot::capture(monster.monster.game, &id)?;
        fire_rocket(monster.monster.game, &snapshot, &id)?;
        let (remaining, max) = monster
            .monster
            .game
            .entity_ref(&id)
            .map(|entity| (entity.number("projectiles") - 1.0, entity.number("projectiles_max")))
            .unwrap_or((0.0, 0.0));
        monster.monster.game.update_entity(&id, |entity| {
            entity.fields.insert(String::from("projectiles"), remaining.to_string());
        })?;
        if max != 0.0 && remaining > 0.0 {
            monster.monster.controller.next_frame = String::from("ogre_nail2");
        }
    }
    if (name == "ogre_stand5" || name == "ogre_walk3" || name == "ogre_run1")
        && monster.monster.game.host.random() < 0.2
    {
        let base = if name == "ogre_walk3" { 3 } else { 1 };
        let index = base + if monster.monster.game.host.random() < 0.5 { 0 } else { 1 };
        monster.monster.game.sound(
            &id,
            &format!("armagon/idle{index}.wav"),
            Q1SoundChannel::Voice,
            2.0,
            1.0,
        )?;
    }
    Ok(())
}

/// Snapshot of the fields rocket fire reads, so the mutable game can
/// be shared with the missile spawn.
struct BaseMonsterSnapshot {
    enemy: Option<qa_core::identity::ActorId>,
    target: Option<Vec3>,
    origin: Vec3,
    basis_right: Vec3,
    basis_forward: Vec3,
}

impl BaseMonsterSnapshot {
    fn capture(game: &mut Q1EntityServices, id: &qa_core::identity::ActorId) -> Result<Self, Q1Error> {
        let enemy = game
            .entity_ref(id)
            .and_then(|entity| entity.monster.as_ref())
            .and_then(|monster| monster.enemy.clone());
        let target = enemy
            .as_ref()
            .and_then(|enemy| game.host.bodies.read(enemy).map(|body| body.origin));
        let origin = game.body(id).map(|body| body.origin)?;
        let angles = game.body(id).map(|body| body.angles)?;
        let basis = game.make_vectors(angles);
        Ok(Self {
            enemy,
            target,
            origin,
            basis_right: basis.right,
            basis_forward: basis.forward,
        })
    }
}

/// Fire a rocket-ogre missile (`fireRocket`).
fn fire_rocket(
    game: &mut Q1EntityServices,
    monster: &BaseMonsterSnapshot,
    id: &qa_core::identity::ActorId,
) -> Result<(), Q1Error> {
    let (Some(target), Some(enemy)) = (monster.target, monster.enemy.clone()) else {
        return Ok(());
    };
    let Some(body) = game.host.bodies.read(&enemy) else {
        return Ok(());
    };
    let (max, count) = game
        .entity_ref(id)
        .map(|entity| (entity.number("projectiles_max"), entity.number("projectiles")))
        .unwrap_or((0.0, 0.0));
    let offset = (max - count) * if game.host.random() > 0.5 { -64.0 } else { 64.0 };
    let flat = Vec3 {
        x: body.velocity.x,
        y: body.velocity.y,
        z: 0.0,
    };
    let lead = vscale(flat, f64::from(length(vsub(target, monster.origin))) / 1200.0);
    let aim = vadd(
        vsub(
            vadd(
                vadd(
                    target,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: -8.0,
                    },
                ),
                lead,
            ),
            vadd(
                monster.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 16.0,
                },
            ),
        ),
        vscale(monster.basis_right, offset),
    );
    game.update_entity(id, |entity| {
        entity.effects |= 2;
    })?;
    game.sound(id, "weapons/sgun1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let origin = vadd(
        vadd(monster.origin, vscale(monster.basis_forward, 8.0)),
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let missile = create_missile(
        game,
        Some(id),
        "ogre_missile",
        "missile",
        origin,
        vscale(normalize(aim), 600.0),
        5.0,
    )?;
    let touch = game.named.touch("mg3:ordinary:T_OgreMissileTouch")?;
    let time = game.time;
    game.update_entity(&missile, |entity| {
        entity
            .fields
            .insert(String::from("ammo_nails"), (time + 0.2).to_string());
        entity.touch = Some(touch);
    })?;
    Ok(())
}

fn ogre_missile_touch(
    game: &mut Q1EntityServices,
    id: &qa_core::identity::ActorId,
    other: &qa_core::identity::ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let missile = id.clone();
    let other = other.clone();
    let entity = game
        .entity_ref(&missile)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.owner.as_ref().is_some_and(|owner| same_actor(owner, &other)) {
        return Ok(());
    }
    let body = game.body(&missile)?;
    let origin = body.origin;
    if game.host.contents(origin) == Q1Contents::Sky {
        return game.remove(&missile);
    }
    if entity.number("ammo_nails") > game.time {
        game.effect(Q1Effect::Blood, origin, Some(&other), 18);
        game.damage(
            &other,
            Some(&missile),
            entity.owner.as_ref(),
            20.0,
            &Q1DamageParams::default(),
        );
        game.effect(Q1Effect::KnightSpike, origin, None, 1);
        return game.remove(&missile);
    }
    if game.health(&other) != 0.0 {
        let classname = game
            .entity_ref(&other)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        game.damage(
            &other,
            Some(&missile),
            entity.owner.as_ref(),
            if classname == "monster_shambler" {
                20.0
            } else if classname == "monster_zombie" {
                60.0
            } else {
                40.0
            },
            &Q1DamageParams::default(),
        );
    }
    game.radius_damage(&missile, entity.owner.as_ref(), 40.0, Some(&other), None, "");
    game.set_origin(&missile, vsub(origin, vscale(normalize(body.velocity), 8.0)))?;
    sprite_explosion(game, &missile)
}

/// Register the rocket-ogre missile touch (`registerRocketOgre`).
pub fn register_rocket_ogre(_context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "mg3:ordinary:T_OgreMissileTouch",
        Q1CallbackHandlers {
            touch: Some(ogre_missile_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    Ok(())
}
