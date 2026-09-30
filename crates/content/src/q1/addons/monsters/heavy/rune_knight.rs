//! Q1 mg3 rune knight (`src/content/q1/addons/monsters/heavy/rune-knight.ts`).
//!
//! `quakec_mg3/monsters/mg3_rknight.qc`. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::campaign::BLOODY_NIGHTMARE_ACTIVE;
use crate::q1::addons::context::set_addon_number;
use crate::q1::base::projectiles::{throw_gib, throw_head};
use crate::q1::base::provider::campaign_read_flags;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{length, normalize, vadd, vscale, vsub, Q1SoundChannel, ZERO};
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::Q1Error;

use super::projectiles::heavy_spike;
use super::runtime::{heavy_initialize, heavy_melee_cycle, set_heavy_melee_cycle};
use crate::q1::addons::monsters::ai::{Mg3ActionHandler, Mg3Monster};

/// Rune-knight spawn defaults (`runeKnightDefinition.spec`).
pub const RUNE_KNIGHT_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Hellknight,
    kill_string: None,
    classnames: &["monster_ranged_knight"],
    model: "rknight",
    head: Some("h_hellkn"),
    health: 250.0,
    gib_health: -40.0,
    gibs: &["gib1", "gib2", "gib3"],
    bounds: Bounds {
        min: Vec3 {
            x: -16.0,
            y: -16.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 16.0,
            y: 16.0,
            z: 40.0,
        },
    },
    stand: "rknight_stand1",
    walk: "rknight_walk1",
    run: "rknight_run",
    sight: "",
    missile: Some("rknight_magic"),
    melee: false,
    movement: MonsterMovement::Walk,
};

fn shot(monster: &mut Mg3Monster, offset: f64, variant: u32) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let body = monster.monster.game.body(&id)?;
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = monster.monster.origin()?;
    let delta = vsub(target, origin);
    let (muzzle, direction) = if variant == 0 {
        let angles = velocity_angles(delta);
        let forward = monster
            .monster
            .game
            .make_vectors(Vec3 {
                x: angles.x,
                y: angles.y + offset as f32 * 6.0,
                z: angles.z,
            })
            .forward;
        let muzzle = vadd(
            vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5)),
            vscale(forward, 20.0),
        );
        let mut direction = normalize(forward);
        direction = Vec3 {
            x: direction.x,
            y: direction.y,
            z: -direction.z + (monster.monster.game.host.random() - 0.5) as f32 * 0.1,
        };
        (muzzle, direction)
    } else {
        let forward = monster.monster.game.make_vectors(body.angles).forward;
        let mut muzzle = vadd(
            vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5)),
            vscale(forward, 10.0),
        );
        let mut direction = normalize(delta);
        let nudge = 0.017 * offset * 6.0;
        if variant == 1 {
            muzzle = vadd(
                muzzle,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 10.0,
                },
            );
            direction = Vec3 {
                x: direction.x,
                y: direction.y,
                z: direction.z + nudge as f32,
            };
            direction = Vec3 {
                x: direction.x,
                y: direction.y + (monster.monster.game.host.random() - 0.5) as f32 * 0.2,
                z: direction.z,
            };
        } else {
            muzzle = vadd(
                muzzle,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 14.0,
                },
            );
            let spread = (offset + 2.0) as f32;
            direction = Vec3 {
                x: direction.x,
                y: direction.y + (monster.monster.game.host.random() - 0.5) as f32 * 0.1 * spread,
                z: direction.z,
            };
            direction = Vec3 {
                x: direction.x,
                y: direction.y,
                z: direction.z + (monster.monster.game.host.random() - 0.5) as f32 * 0.1 * spread,
            };
        }
        (muzzle, normalize(direction))
    };
    let missile = heavy_spike(monster.monster.game, &id, muzzle, vscale(direction, 1000.0))?;
    let nightmare = campaign_read_flags(monster.monster.game)? & BLOODY_NIGHTMARE_ACTIVE != 0;
    monster.monster.game.update_entity(&missile, |entity| {
        entity.model = String::from("progs/diamond_trail.mdl");
    })?;
    monster.monster.game.set_body(
        &missile,
        &BodyPatch {
            velocity: Some(vscale(direction, if nightmare { 600.0 } else { 400.0 })),
            ..Default::default()
        },
    )?;
    monster.monster.game.update_entity(&missile, |entity| {
        entity.effects = 64;
    })?;
    monster.monster.game.link(&missile)?;
    monster
        .monster
        .game
        .sound(&id, "hknight/attack1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)
}

fn idle(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    if monster.monster.game.host.random() >= 0.2 {
        return Ok(());
    }
    let rolled = monster.monster.game.host.random();
    monster.monster.game.sound_simple(
        &monster.monster.id.clone(),
        if rolled < 0.3 {
            "rknight/idle_02.wav"
        } else if rolled < 0.6 {
            "rknight/idle_03.wav"
        } else {
            "rknight/idle_05.wav"
        },
    )
}

fn pain_sound(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let rolled = monster.monster.game.host.random();
    monster.monster.game.sound_simple(
        &monster.monster.id.clone(),
        if rolled < 0.3 {
            "rknight/pain_01.wav"
        } else if rolled < 0.6 {
            "rknight/pain_02.wav"
        } else {
            "rknight/pain_03.wav"
        },
    )
}

fn magic(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = monster.monster.origin()?;
    let delta = vsub(target, origin);
    if monster.monster.game.host.random() < if f64::from(length(delta)) > 300.0 { 0.6 } else { 0.2 } {
        return monster.play("rknight_magicb1");
    }
    let threshold = if delta.z > 100.0 { 0.7 } else { 0.5 };
    let rolled = monster.monster.game.host.random();
    monster.play(if rolled > threshold {
        "rknight_magica1"
    } else {
        "rknight_magicc1"
    })
}

fn run(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let dash = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.spawnflags)
        .unwrap_or(0)
        & 2
        != 0;
    monster.play(if dash { "rknight_runb1" } else { "rknight_run1" })
}

fn knight_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let cycle = heavy_melee_cycle(monster.monster.game)? + 1.0;
    set_heavy_melee_cycle(monster.monster.game, cycle)?;
    monster.monster.game.sound(
        &monster.monster.id.clone(),
        "hknight/slash1.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    if cycle == 1.0 {
        return monster.play("rknight_slice1");
    }
    if cycle == 2.0 {
        return monster.play("rknight_smash1");
    }
    if cycle == 3.0 {
        monster.play("rknight_watk1")?;
        set_heavy_melee_cycle(monster.monster.game, 0.0)?;
    }
    Ok(())
}

/// Rune-knight frame actions (`runeKnightDefinition.actions`).
pub fn rune_knight_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn magicb6(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            let count = (monster.monster.game.host.random() * 8.0 + 0.5).floor();
            set_addon_number(monster.monster.game, &monster.monster.id.clone(), "ammo_nails", count)
        }
        fn magicb12(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, 3.0, 2)?;
            let id = monster.monster.id.clone();
            let remaining = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.number("ammo_nails"))
                .unwrap_or(0.0)
                - 1.0;
            set_addon_number(monster.monster.game, &id, "ammo_nails", remaining)?;
            if remaining > 0.0 {
                monster.monster.controller.next_frame = String::from("rknight_magicb12");
            }
            Ok(())
        }
        fn magica8(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, 1.5, 1)
        }
        fn magica9(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, 0.5, 1)
        }
        fn magica10(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, -0.5, 1)
        }
        fn magica11(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, -1.5, 1)
        }
        fn magica12(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, -2.5, 1)
        }
        fn magicb7(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, -2.0, 2)
        }
        fn magicb8(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, -1.0, 2)
        }
        fn magicb9(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, 0.0, 2)
        }
        fn magicb10(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, 1.0, 2)
        }
        fn magicb11(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            shot(monster, 2.0, 2)
        }
        fn magicc6(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            shot(monster, -2.0, 0)
        }
        fn magicc7(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            shot(monster, -1.0, 0)
        }
        fn magicc8(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            shot(monster, 0.0, 0)
        }
        fn magicc9(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            shot(monster, 1.0, 0)
        }
        fn magicc10(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            shot(monster, 2.0, 0)
        }
        fn magicc11(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            shot(monster, 3.0, 0)
        }
        HashMap::from([
            (String::from("rk_idle_sound"), idle as Mg3ActionHandler),
            (String::from("rknight_pain_sound"), pain_sound as Mg3ActionHandler),
            (String::from("rknight_magic"), magic as Mg3ActionHandler),
            (String::from("rknight_run"), run as Mg3ActionHandler),
            (String::from("rknight_melee"), knight_melee as Mg3ActionHandler),
            (String::from("mg3_rknight:rknight_magicb6"), magicb6 as Mg3ActionHandler),
            (
                String::from("mg3_rknight:rknight_magicb12"),
                magicb12 as Mg3ActionHandler,
            ),
            (String::from("mg3_rknight:rknight_magica8"), magica8 as Mg3ActionHandler),
            (String::from("mg3_rknight:rknight_magica9"), magica9 as Mg3ActionHandler),
            (
                String::from("mg3_rknight:rknight_magica10"),
                magica10 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_rknight:rknight_magica11"),
                magica11 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_rknight:rknight_magica12"),
                magica12 as Mg3ActionHandler,
            ),
            (String::from("mg3_rknight:rknight_magicb7"), magicb7 as Mg3ActionHandler),
            (String::from("mg3_rknight:rknight_magicb8"), magicb8 as Mg3ActionHandler),
            (String::from("mg3_rknight:rknight_magicb9"), magicb9 as Mg3ActionHandler),
            (
                String::from("mg3_rknight:rknight_magicb10"),
                magicb10 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_rknight:rknight_magicb11"),
                magicb11 as Mg3ActionHandler,
            ),
            (String::from("mg3_rknight:rknight_magicc6"), magicc6 as Mg3ActionHandler),
            (String::from("mg3_rknight:rknight_magicc7"), magicc7 as Mg3ActionHandler),
            (String::from("mg3_rknight:rknight_magicc8"), magicc8 as Mg3ActionHandler),
            (String::from("mg3_rknight:rknight_magicc9"), magicc9 as Mg3ActionHandler),
            (
                String::from("mg3_rknight:rknight_magicc10"),
                magicc10 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_rknight:rknight_magicc11"),
                magicc11 as Mg3ActionHandler,
            ),
        ])
    })
}

/// Spawn a rune knight (`runeKnightDefinition.spawn`).
pub fn rune_knight_spawn(
    monster: &mut Mg3Monster,
    context: &crate::q1::addons::context::Q1AddonContext,
) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    monster.monster.game.set_health(&id, 250.0)?;
    set_addon_number(monster.monster.game, &id, "allowPathFind", 1.0)?;
    set_addon_number(monster.monster.game, &id, "combat_style", 1.0)?;
    heavy_initialize(monster, context, 1)
}

/// Rune-knight sight (`runeKnightDefinition.sight`).
pub fn rune_knight_sight(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let rolled = monster.monster.game.host.random();
    let id = monster.monster.id.clone();
    monster.monster.game.sound_simple(
        &id,
        if rolled < 0.5 {
            "rknight/sight_01.wav"
        } else {
            "rknight/sight_03.wav"
        },
    )
}

/// Rune-knight melee (`runeKnightDefinition` melee action).
pub fn rune_knight_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    knight_melee(monster)
}

/// React to rune-knight pain (`runeKnightDefinition.pain`).
pub fn rune_knight_pain(monster: &mut Mg3Monster, _attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
    if monster.monster.monster.pain_finished > monster.monster.game.time {
        return Ok(());
    }
    if monster.monster.game.time - monster.monster.monster.pain_finished > 5.0 {
        monster.play("rknight_pain1")?;
        monster.monster.monster.pain_finished = monster.monster.game.time + 1.0;
        return Ok(());
    }
    if monster.monster.game.host.random() * 30.0 > damage {
        return Ok(());
    }
    monster.monster.monster.pain_finished = monster.monster.game.time + 1.0;
    monster.play("rknight_pain1")
}

/// Die as a rune knight (`runeKnightDefinition.die`).
pub fn rune_knight_die(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let health = monster.monster.game.health(&id);
    if health < -40.0 {
        monster.monster.game.sound_simple(&id, "player/udeath.wav")?;
        throw_head(monster.monster.game, &id, "h_hellkn", health)?;
        for model in ["gib1", "gib2", "gib3"] {
            let origin = monster.monster.origin()?;
            throw_gib(monster.monster.game, origin, model, health)?;
        }
        return Ok(());
    }
    let track = monster.monster.game.host.random();
    monster.monster.game.sound_simple(
        &id,
        if track < 0.5 {
            "rknight/death_01.wav"
        } else {
            "rknight/death_02.wav"
        },
    )?;
    let rolled = monster.monster.game.host.random();
    monster.play(if rolled > 0.5 { "rknight_die1" } else { "rknight_dieb1" })
}
