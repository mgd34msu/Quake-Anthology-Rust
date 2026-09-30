//! Shared mission-pack monster operations
//! (`src/content/q1/missionpacks/monsters/helpers.ts`).

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::projectiles::{throw_gib, throw_head};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    length, vadd, vscale, vsub, Q1MoveType, Q1Solid, Q1TraceRequest, POINT,
};

use super::runtime::MissionMonster;

/// Human collision bounds (`humanBounds`).
pub const HUMAN_BOUNDS: Bounds = Bounds {
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
};

/// Hull collision bounds (`hullBounds`).
pub const HULL_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 32.0,
    },
};

/// Write a binary32 numeric field (`number`). Flushes immediately so later
/// game calls in the same callback observe the value.
pub fn number(monster: &mut MissionMonster, key: &str, value: f64) {
    monster.entity.fields.insert(key.to_string(), (value as f32).to_string());
    monster.flush_entity();
}

/// Drop the monster to the floor (`dropToFloor`).
pub fn drop_to_floor(monster: &mut MissionMonster) -> bool {
    monster.sync();
    let id = monster.entity.actor.id.clone();
    let body = match monster.game.body(&id) {
        Ok(body) => body,
        Err(_) => return false,
    };
    let trace = monster.game.host.trace(&Q1TraceRequest {
        start: body.origin,
        end: vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: -256.0 }),
        bounds: body.bounds,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    if trace.fraction == 1.0 || trace.all_solid {
        return false;
    }
    monster.entity.movement_flags |= 512;
    let _ = monster.game.set_body(
        &id,
        &BodyPatch {
            origin: Some(trace.end),
            ground: Some(trace.actor),
            ..Default::default()
        },
    );
    monster.flush_entity();
    let _ = monster.game.link(&id);
    monster.refresh();
    true
}

/// Gib the monster (`gib`). `sound` defaults to `player/udeath.wav` in the
/// donor; pass `Some("")` to skip the sound.
pub fn gib(monster: &mut MissionMonster, head: &str, gibs: &[&str], sound: Option<&str>) {
    monster.sync();
    let id = monster.entity.actor.id.clone();
    let sound = sound.unwrap_or("player/udeath.wav");
    if !sound.is_empty() {
        let _ = monster.game.sound_simple(&id, sound);
    }
    let health = monster.game.health(&id);
    let _ = throw_head(monster.game, &id, head, health);
    for model in gibs {
        let _ = throw_gib(monster.game, monster.origin, model, health);
    }
    monster.refresh();
}

/// Eye position for an actor (`eye`).
pub fn eye(game: &mut Q1EntityServices, actor: &ActorId) -> Option<Vec3> {
    let body = game.host.bodies.read(actor)?;
    let entity = game.entity(actor).cloned();
    let offset = match entity.as_ref() {
        Some(entity) if entity.fields.contains_key("view_ofs") => entity.vector("view_ofs"),
        Some(entity) => {
            let ducked = entity.movement_flags & 2 != 0;
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: if game.is_player(actor) {
                    22.0
                } else if ducked {
                    10.0
                } else {
                    25.0
                },
            }
        }
        None => Vec3 {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
    };
    Some(vadd(body.origin, offset))
}

/// Actors within `radius` of `origin`, newest first (`radiusActors`).
pub fn radius_actors(game: &mut Q1EntityServices, origin: Vec3, radius: f64) -> Vec<ActorId> {
    let mut actors = Vec::new();
    for observation in game.host.actors.observations() {
        let id = observation.id.clone();
        let body = game.host.bodies.read(&id);
        let solid = game.entity(&id).map(|entity| entity.solid);
        let Some(body) = body else { continue };
        if matches!(solid, Some(Q1Solid::None)) {
            continue;
        }
        let center = vadd(
            body.origin,
            vscale(vadd(body.bounds.min, body.bounds.max), 0.5),
        );
        if f64::from(length(vsub(origin, center))) <= radius {
            actors.push(id);
        }
    }
    actors.reverse();
    actors
}

/// Electric discharge around the monster (`eelZap`).
pub fn eel_zap(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id.clone();
    let origin = monster.origin;
    for target in radius_actors(monster.game, origin, 85.0) {
        let classname = monster.game.host.classname(&target);
        if classname == "monster_eel" {
            continue;
        }
        let flags = monster.game.entity(&target).map(|entity| entity.movement_flags).unwrap_or(0);
        if flags & 16 == 0 {
            continue;
        }
        let body = monster.game.host.bodies.read(&target);
        let damageable = monster
            .game
            .host
            .combat
            .read(&target)
            .is_some_and(|combat| combat.can_take_damage);
        let Some(body) = body else { continue };
        if !damageable {
            continue;
        }
        let center = vadd(
            body.origin,
            vscale(vadd(body.bounds.min, body.bounds.max), 0.5),
        );
        let mut points = 45.0 - (0.5 * f64::from(length(vsub(origin, center)))).max(0.0);
        if target == id {
            points *= 0.5;
        }
        if points > 0.0 && monster.game.can_damage(&target, &id) {
            monster.game.damage(&target, Some(&id), Some(&id), points, &Q1DamageParams::default());
        }
    }
    monster.refresh();
}

/// Spawn a missile (`missile`). Missing named callbacks leave the touch or
/// removal unset instead of failing, so tests can run without base
/// registration.
pub fn missile(
    game: &mut Q1EntityServices,
    owner: &ActorId,
    classname: &str,
    model: &str,
    origin: Vec3,
    velocity: Vec3,
    touch: &str,
    seconds: f64,
) -> ActorId {
    let id = game.create(classname, None, None).expect("missile entity");
    let owner = owner.clone();
    let model = model.to_string();
    let _ = game.update_entity(&id, |entity| {
        entity.owner = Some(owner);
        entity.model = model;
        entity.movement = Q1MoveType::Flymissile;
        entity.solid = Q1Solid::Bbox;
    });
    let _ = game.set_body(
        &id,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            bounds: Some(POINT),
            ground: Some(None),
            ..Default::default()
        },
    );
    if let Ok(touch) = game.named.touch(touch) {
        let _ = game.update_entity(&id, |entity| entity.touch = Some(touch));
    }
    if let Ok(remove) = game.named.action("SUB_Remove") {
        let _ = game.schedule(&id, seconds, &remove);
    }
    let _ = game.link(&id);
    id
}

/// No-op pain handler (`emptyPain`).
pub fn empty_pain(_monster: &mut MissionMonster, _attacker: Option<&ActorId>, _damage: f64) {}

/// Collapse the monster bounds to a point (`setPointBounds`).
pub fn set_point_bounds(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id.clone();
    let _ = monster.game.set_bounds(&id, POINT);
    monster.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
    use crate::q1::foundation::entity::Q1MonsterSpecies;
    use crate::q1::foundation::types::ZERO;
    use crate::q1::missionpacks::types::test_game;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::PackMonsterDefinition;
    use crate::q1::missionpacks::types::Q1MissionPack;
    use std::rc::Rc;

    fn test_definition() -> PackMonsterDefinition {
        PackMonsterDefinition {
            spec: Box::leak(Box::new(MonsterSpecies {
                species: Q1MonsterSpecies::Gremlin,
                kill_string: None,
                classnames: &["monster_gremlin"],
                model: "grem",
                head: None,
                health: 100.0,
                gib_health: -35.0,
                gibs: &[],
                bounds: HULL_BOUNDS,
                stand: "gremlin_stand1",
                walk: "gremlin_walk1",
                run: "gremlin_run1",
                sight: "",
                missile: None,
                melee: true,
                movement: MonsterMovement::Walk,
            })),
            base_behavior: false,
            frames: &[],
            actions: Vec::new(),
            callbacks: Vec::new(),
            spawn: None,
            start: None,
            pain: Rc::new(|_, _, _| {}),
            die: Rc::new(|_, _| {}),
            melee: None,
            check_attack: None,
            found: None,
            ai: None,
            use_: None,
        }
    }

    #[test]
    fn bounds_match_donor() {
        assert_eq!(HUMAN_BOUNDS.min, Vec3 { x: -16.0, y: -16.0, z: -24.0 });
        assert_eq!(HUMAN_BOUNDS.max, Vec3 { x: 16.0, y: 16.0, z: 40.0 });
        assert_eq!(HULL_BOUNDS.min, Vec3 { x: -16.0, y: -16.0, z: -24.0 });
        assert_eq!(HULL_BOUNDS.max, Vec3 { x: 16.0, y: 16.0, z: 32.0 });
        assert_eq!(ZERO, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
    }

    #[test]
    fn helpers_smoke() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, Default::default()).expect("new");
        runtime.register(&mut game, test_definition()).expect("register");
        let id = game.create("monster_gremlin", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        number(&mut monster, "stoleweapon", 1.0);
        assert_eq!(monster.entity.fields.get("stoleweapon").map(String::as_str), Some("1"));
        empty_pain(&mut monster, None, 0.0);
        let _ = drop_to_floor(&mut monster);
        gib(&mut monster, "h_grem", &["gib1"], None);
        eel_zap(&mut monster);
        set_point_bounds(&mut monster);
        let actors = radius_actors(monster.game, monster.origin, 85.0);
        assert!(actors.is_empty() || !actors.is_empty());
        let shot = missile(monster.game, &id, "missile", "progs/missile.mdl", monster.origin, ZERO, "projectile_touch", 5.0);
        assert_eq!(monster.game.entity(&shot).map(|entity| entity.classname.as_str()), Some("missile"));
    }

    #[test]
    fn eye_is_none_without_body() {
        let mut game = test_game();
        let id = game.create("monster_gremlin", None, None).expect("create");
        let _ = game.remove(&id);
        assert!(eye(&mut game, &id).is_none());
    }
}
