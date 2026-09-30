//! Gremlin stolen-weapon attacks (`src/content/q1/missionpacks/monsters/gremlin-weapons.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::contract::InventoryEntry;
use crate::q1::base::projectiles::launch_ogre_grenade;
use crate::q1::base::projectiles::{drop_backpack, BackpackDrop};
use crate::q1::base::projectiles::{launch_spike, SpikeKind};
use crate::q1::foundation::entity_services::Q1DamageParams;
use crate::q1::foundation::gameplay::DamageDelivery;
use crate::q1::foundation::types::{
    normalize, vadd, vscale, vsub, Q1BeamStyle, Q1Edition, Q1Effect, Q1Event, Q1SoundChannel, Q1TraceRequest, Q1Weapon,
    POINT, ZERO,
};
use crate::q1::foundation::weapons::fire_bullets;
use crate::q1::missionpacks::hipnotic_weapons::{launch_hipnotic_laser, launch_hipnotic_proximity};
use crate::q1::missionpacks::types::velocity_angles;

use super::gremlin_ai::gremlin_find_victim;
use super::helpers::{missile, number};
use super::runtime::MissionMonster;

/// Weapons a gremlin can steal (`weapons`).
const WEAPONS: [Q1Weapon; 11] = [
    Q1Weapon::Axe,
    Q1Weapon::Shotgun,
    Q1Weapon::Supershotgun,
    Q1Weapon::Nailgun,
    Q1Weapon::Supernailgun,
    Q1Weapon::Grenadelauncher,
    Q1Weapon::Rocketlauncher,
    Q1Weapon::Lightning,
    Q1Weapon::HipnoticLaser,
    Q1Weapon::HipnoticProximity,
    Q1Weapon::HipnoticMjolnir,
];

/// Stolen weapon carried by the gremlin (`gremlinWeapon`).
pub fn gremlin_weapon(monster: &MissionMonster) -> Option<Q1Weapon> {
    let value = monster.entity.fields.get("gremlin:weapon")?.as_str();
    WEAPONS.into_iter().find(|weapon| weapon.as_str() == value)
}

/// Spend stolen ammunition (`ammo`).
fn ammo(monster: &mut MissionMonster, item: &str, used: f64) {
    let owned = monster.entity.actor.clone();
    let item = item.into();
    let count = monster
        .game
        .host
        .inventory
        .adjust_source_counter(&owned, &item, -used)
        .unwrap_or(0.0);
    number(monster, "currentammo", count);
}

/// Whether the gremlin still has stolen ammunition (`gremlinHasAmmo`).
pub fn gremlin_has_ammo(monster: &mut MissionMonster) -> bool {
    if monster.entity.number("currentammo") > 0.0 {
        return true;
    }
    number(monster, "stoleweapon", 0.0);
    false
}

/// Steal the player's current weapon (`gremlinSteal`).
pub fn gremlin_steal(monster: &mut MissionMonster) -> bool {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let Some(enemy) = monster.enemy.clone() else {
        monster.refresh();
        return false;
    };
    if monster.entity.number("stoleweapon") != 0.0
        || !monster.game.is_player(&enemy)
        || monster.distance() > 100.0
        || monster.game.host.random() < 0.5
    {
        monster.refresh();
        return false;
    }
    let Some(victim) = monster.game.player_ref(&enemy).cloned() else {
        monster.refresh();
        return false;
    };
    let weapon = victim.weapon;
    if matches!(weapon, Q1Weapon::Axe | Q1Weapon::Shotgun | Q1Weapon::HipnoticMjolnir) {
        monster.refresh();
        return false;
    }
    let weapon_item = monster.game.weapon_item(weapon);
    let _ = monster.game.host.inventory.consume(
        &victim.actor,
        &weapon_item,
        monster.game.host.inventory.count(&enemy, &weapon_item),
    );
    let owned = monster.entity.actor.clone();
    let _ = monster.game.host.inventory.configure(
        &owned,
        &InventoryEntry {
            item: weapon_item,
            count: 1.0,
            capacity: 1.0,
            count_policy: None,
        },
    );
    monster
        .entity
        .fields
        .insert("gremlin:weapon".to_string(), weapon.as_str().to_string());
    monster.flush_entity();
    if let Some(item) = monster.game.weapon_ammo(weapon) {
        let cap = if weapon == Q1Weapon::Supershotgun {
            20.0
        } else if matches!(
            weapon,
            Q1Weapon::Grenadelauncher | Q1Weapon::Rocketlauncher | Q1Weapon::HipnoticProximity
        ) {
            5.0
        } else {
            40.0
        };
        let amount = monster.game.host.inventory.count(&enemy, &item).min(cap);
        let _ = monster.game.host.inventory.consume(&victim.actor, &item, amount);
        let owned = monster.entity.actor.clone();
        let held = monster.game.host.inventory.count(&id, &item) + amount;
        let _ = monster.game.host.inventory.configure(
            &owned,
            &InventoryEntry {
                item: item.clone(),
                count: held,
                capacity: 1_000_000.0,
                count_policy: None,
            },
        );
        number(monster, "currentammo", held);
        let label = match weapon {
            Q1Weapon::Supershotgun => ("$qc_gremlin_ssg", "Gremlin stole your Super Shotgun\n"),
            Q1Weapon::Nailgun => ("$qc_gremlin_ng", "Gremlin stole your Nailgun\n"),
            Q1Weapon::Supernailgun => ("$qc_gremlin_sng", "Gremlin stole your Super Nailgun\n"),
            Q1Weapon::Grenadelauncher => ("$qc_gremlin_gl", "Gremlin stole your Grenade Launcher\n"),
            Q1Weapon::Rocketlauncher => ("$qc_gremlin_rl", "Gremlin stole your Rocket Launcher\n"),
            Q1Weapon::Lightning => ("$qc_gremlin_lg", "Gremlin stole your Lightning Gun\n"),
            Q1Weapon::HipnoticLaser => ("$qc_gremlin_lc", "Gremlin stole your Laser Cannon\n"),
            Q1Weapon::HipnoticProximity => ("$qc_gremlin_prox", "Gremlin stole your Proximity Gun\n"),
            _ => ("", ""),
        };
        if !label.0.is_empty() {
            let text = if monster.game.options().edition == Q1Edition::Rerelease {
                label.0
            } else {
                label.1
            };
            monster.game.message(Some(&enemy), text, false, Vec::new());
        }
    }
    if let Ok(best) = monster.game.choose_best(&victim.actor, None) {
        let _ = monster.game.select_weapon(&victim.actor, best);
    }
    number(monster, "stoleweapon", 1.0);
    let last = if monster.game.host.random() > 0.65 {
        Some(enemy)
    } else {
        Some(id)
    };
    monster.entity.references.insert("lastvictim".to_string(), last);
    monster.flush_entity();
    if let Some(next) = gremlin_find_victim(monster) {
        monster.found(&next);
        let time = monster.game.time;
        monster.state.attack_finished = time;
        monster.state.search_until = time + 1.0;
    }
    monster.refresh();
    true
}

/// Aim a stolen weapon with spread (`aim`).
fn aim(monster: &mut MissionMonster, spread: f64) -> Vec3 {
    let direction = normalize(vsub(monster.target.unwrap_or(ZERO), monster.origin));
    let angles = velocity_angles(direction);
    monster
        .entity
        .fields
        .insert("v_angle".to_string(), format!("{} {} {}", angles.x, angles.y, angles.z));
    monster.flush_entity();
    let basis = monster.game.make_vectors(angles);
    normalize(vadd(
        vadd(
            direction,
            vscale(basis.right, (monster.game.host.random() * 2.0 - 1.0) * spread),
        ),
        vscale(monster.game.basis.up, (monster.game.host.random() * 2.0 - 1.0) * spread),
    ))
}

/// Fire a stolen nailgun (`gremlinFireNail`).
pub fn gremlin_fire_nail(monster: &mut MissionMonster) {
    ammo(monster, "q1:ammo/nails", 1.0);
    monster.entity.effects |= 2;
    monster.flush_entity();
    let id = monster.entity.actor.id().clone();
    let _ = monster
        .game
        .sound(&id, "weapons/rocket1i.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let direction = aim(monster, 0.1);
    let origin = vadd(
        monster.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let _ = launch_spike(
        monster.game,
        Some(&id),
        origin,
        vscale(direction, 1000.0),
        SpikeKind::Spike,
    );
    monster.refresh();
}

/// Fire a stolen laser cannon (`gremlinFireLaser`).
pub fn gremlin_fire_laser(monster: &mut MissionMonster) {
    ammo(monster, "q1:ammo/cells", 1.0);
    monster.entity.effects |= 2;
    monster.flush_entity();
    let id = monster.entity.actor.id().clone();
    let _ = monster
        .game
        .sound(&id, "weapons/rocket1i.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let direction = aim(monster, 0.1);
    let origin = vadd(
        monster.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let _ = launch_hipnotic_laser(monster.game, &id, origin, direction, false, None);
    monster.refresh();
}

/// Fire a stolen (super) shotgun (`shotgun`).
fn shotgun(monster: &mut MissionMonster, double: bool) {
    ammo(monster, "q1:ammo/shells", if double { 2.0 } else { 1.0 });
    monster.entity.effects |= 2;
    monster.flush_entity();
    let id = monster.entity.actor.id().clone();
    let _ = monster.game.sound(
        &id,
        if double {
            "weapons/shotgn2.wav"
        } else {
            "weapons/guncock.wav"
        },
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    );
    let direction = aim(monster, if double { 0.3 } else { 0.1 });
    let angles = velocity_angles(direction);
    monster
        .entity
        .fields
        .insert("v_angle".to_string(), format!("{} {} {}", angles.x, angles.y, angles.z));
    monster.flush_entity();
    let owned = monster.entity.actor.clone();
    fire_bullets(
        monster.game,
        &owned,
        direction,
        angles,
        if double { 14 } else { 6 },
        if double { 0.14 } else { 0.04 },
        if double { 0.08 } else { 0.04 },
        Some(if double {
            Q1Weapon::Supershotgun
        } else {
            Q1Weapon::Shotgun
        }),
    );
    monster.refresh();
}

/// Fire a stolen rocket launcher (`rocket`).
fn rocket(monster: &mut MissionMonster) {
    ammo(monster, "q1:ammo/rockets", 1.0);
    monster.entity.effects |= 2;
    monster.flush_entity();
    let id = monster.entity.actor.id().clone();
    let _ = monster
        .game
        .sound(&id, "weapons/sgun1.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    monster
        .entity
        .fields
        .insert("punchangle".to_string(), "-2 0 0".to_string());
    monster.flush_entity();
    let direction = aim(monster, 0.1);
    let origin = vadd(
        vadd(monster.origin, vscale(monster.game.basis.forward, 8.0)),
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let shot = missile(
        monster.game,
        &id,
        "missile",
        "progs/missile.mdl",
        origin,
        vscale(direction, 1000.0),
        "projectile_touch",
        5.0,
    );
    let _ = monster.game.update_entity(&shot, |entity| {
        entity.projectile = Some(crate::q1::foundation::entity::Q1ProjectileKind::Rocket);
        entity.projectile_weapon = Some(Q1Weapon::Rocketlauncher);
    });
    if let Ok(body) = monster.game.body(&shot) {
        let angles = velocity_angles(body.velocity);
        let _ = monster.game.set_body(
            &shot,
            &crate::q1::foundation::gameplay::BodyPatch {
                angles: Some(angles),
                ..Default::default()
            },
        );
    }
    monster.refresh();
}

/// Fire a stolen thunderbolt (`gremlinFireLightning`).
pub fn gremlin_fire_lightning(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    if monster.entity.water_type <= -3 {
        let cells = monster.game.host.inventory.count(&id, &"q1:ammo/cells".into());
        let owned = monster.entity.actor.clone();
        let _ = monster.game.host.inventory.configure(
            &owned,
            &InventoryEntry {
                item: "q1:ammo/cells".into(),
                count: 0.0,
                capacity: 1_000_000.0,
                count_policy: None,
            },
        );
        let world = monster.game.world.clone();
        monster.game.radius_damage(
            &id,
            Some(&id),
            35.0 * cells,
            world.as_ref(),
            Some(Q1Weapon::Lightning),
            "discharge",
        );
        monster.refresh();
        return;
    }
    monster.entity.effects |= 2;
    monster.flush_entity();
    monster.face();
    ammo(monster, "q1:ammo/cells", 2.0);
    let start = vadd(
        monster.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 16.0,
        },
    );
    let direction = aim(monster, 0.1);
    let wall = monster.game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(monster.origin, vscale(direction, 600.0)),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    monster.game.host.emit(Q1Event::Beam {
        style: Q1BeamStyle::Lightning2,
        actor: id.clone(),
        start,
        end: wall.end,
    });
    let end = vadd(wall.end, vscale(direction, 4.0));
    let delta = vsub(end, start);
    let side = Vec3 {
        x: -delta.y * 16.0,
        y: -delta.y * 16.0,
        z: 0.0,
    };
    let mut hit: Vec<ActorId> = Vec::new();
    for offset in [ZERO, side, vscale(side, -1.0)] {
        let trace = monster.game.host.trace(&Q1TraceRequest {
            start: vadd(start, offset),
            end: vadd(end, offset),
            bounds: POINT,
            ignore: Some(id.clone()),
            monsters: true,
            missile: false,
        });
        let Some(target) = trace.actor else { continue };
        if hit.contains(&target) {
            continue;
        }
        hit.push(target.clone());
        if monster
            .game
            .host
            .combat
            .read(&target)
            .is_some_and(|combat| combat.can_take_damage)
        {
            monster.game.effect(Q1Effect::Blood, trace.end, Some(&target), 120);
            monster.game.damage(
                &target,
                Some(&id),
                Some(&id),
                30.0,
                &Q1DamageParams {
                    weapon: Some(Q1Weapon::Lightning),
                    delivery: DamageDelivery::Direct,
                    death_type: "electric".to_string(),
                    ..Default::default()
                },
            );
        }
    }
    monster.refresh();
}

/// Fire a stolen proximity gun (`proximity`).
fn proximity(monster: &mut MissionMonster) {
    ammo(monster, "q1:ammo/rockets", 1.0);
    let id = monster.entity.actor.id().clone();
    let _ = monster
        .game
        .sound(&id, "weapons/grenade.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let direction = aim(monster, 0.1);
    let velocity = Vec3 {
        x: vscale(direction, 600.0).x,
        y: vscale(direction, 600.0).y,
        z: 200.0,
    };
    let origin = monster.origin;
    let _ = launch_hipnotic_proximity(monster.game, &id, origin, velocity);
    monster.refresh();
}

/// Attack with the stolen weapon (`gremlinWeaponAttack`).
pub fn gremlin_weapon_attack(monster: &mut MissionMonster) -> bool {
    if !gremlin_has_ammo(monster) {
        return false;
    }
    let time = monster.game.time;
    number(monster, "show_hostile", time + 1.0);
    match gremlin_weapon(monster) {
        Some(Q1Weapon::Shotgun) => {
            monster.play("gremlin_shot1");
            shotgun(monster, false);
            monster.attack_finished(1.0);
        }
        Some(Q1Weapon::Supershotgun) => {
            monster.play("gremlin_shot1");
            shotgun(monster, true);
            monster.attack_finished(1.0);
        }
        Some(Q1Weapon::Nailgun) | Some(Q1Weapon::Supernailgun) => {
            monster.play("gremlin_nail3");
            monster.attack_finished(1.0);
        }
        Some(Q1Weapon::Grenadelauncher) => {
            monster.play("gremlin_rocket1");
            monster.with_base(|base| {
                let _ = launch_ogre_grenade(base);
            });
            ammo(monster, "q1:ammo/rockets", 1.0);
            monster.attack_finished(1.0);
        }
        Some(Q1Weapon::Rocketlauncher) => {
            monster.play("gremlin_rocket1");
            rocket(monster);
            monster.attack_finished(1.0);
        }
        Some(Q1Weapon::Lightning) => {
            monster.play("gremlin_light1");
            monster.attack_finished(1.0);
            let id = monster.entity.actor.id().clone();
            let _ = monster
                .game
                .sound(&id, "weapons/lstart.wav", Q1SoundChannel::Auto, 1.0, 1.0);
        }
        Some(Q1Weapon::HipnoticLaser) => {
            monster.play("gremlin_laser3");
            monster.attack_finished(1.0);
        }
        Some(Q1Weapon::HipnoticProximity) => {
            monster.play("gremlin_rocket1");
            proximity(monster);
            monster.attack_finished(1.0);
        }
        _ => {}
    }
    true
}

/// Drop the stolen weapon as a backpack (`gremlinDropBackpack`).
pub fn gremlin_drop_backpack(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let selected = WEAPONS.into_iter().find(|weapon| {
        monster
            .game
            .host
            .inventory
            .count(&id, &monster.game.weapon_item(*weapon))
            > 0.0
    });
    let shells = monster
        .game
        .host
        .inventory
        .count(&id, &"q1:ammo/shells".into())
        .max(0.0);
    let nails = monster.game.host.inventory.count(&id, &"q1:ammo/nails".into()).max(0.0);
    let rockets = monster
        .game
        .host
        .inventory
        .count(&id, &"q1:ammo/rockets".into())
        .max(0.0);
    let cells = monster.game.host.inventory.count(&id, &"q1:ammo/cells".into()).max(0.0);
    let origin = monster.origin;
    let _ = drop_backpack(
        monster.game,
        origin,
        &BackpackDrop {
            weapon: selected,
            shells,
            nails,
            rockets,
            cells,
            extra: Vec::new(),
            selection: None,
            avoid_underwater_lightning: None,
            owner_pickup_delay: None,
        },
        None,
    );
    monster.refresh();
}

#[cfg(test)]
mod tests {
    use super::{gremlin_drop_backpack, gremlin_has_ammo, gremlin_weapon, WEAPONS};
    use crate::q1::missionpacks::monsters::gremlin::gremlin_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn stolen_weapon_round_trip() {
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default())
                .expect("runtime");
        runtime
            .register(&mut game, gremlin_definition(&runtime))
            .expect("register");
        let id = game.create("monster_gremlin", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        assert!(gremlin_weapon(&monster).is_none());
        assert!(!gremlin_has_ammo(&mut monster));
        assert_eq!(monster.entity.fields.get("stoleweapon").map(String::as_str), Some("0"));
        gremlin_drop_backpack(&mut monster);
        assert_eq!(WEAPONS.len(), 11);
    }
}
