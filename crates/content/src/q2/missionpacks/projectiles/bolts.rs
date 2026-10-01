//! Mission-pack bolts (`src/content/q2/missionpacks/projectiles/bolts.ts`).
//!
//! Xatrix g_weapon.c and Rogue g_newweap.c projectile callbacks
//! (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, dot3, length3, normalize3, scale3, sub3, vec3};

use crate::contract::ProjectileRole;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2Edition, Q2GameServices, Q2Think, Q2Touch, Q2TouchProject, Q2TraceRequest,
};
use crate::q2::foundation::weapons::ballistics::{
    check_dodge, fire_blaster, weapon_player_noise, weapon_player_noise_for_actor, NoiseKind,
};
use crate::q2::foundation::weapons::player::weapon_emit;
use crate::q2::foundation::weapons::types::{Q2WeaponEvent, WeaponBeamEffect};
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use crate::q2::support::contracts::{
    TouchContact, TraceContact, TraceFamily, TraceHit, WeaponTrajectoryUpdate,
};

use super::common::{
    effect, explode, free_projectile, projectile, projectile_mask, publish_projectile, sight,
    velocity,
};
use super::super::types::{Q2_MISSION_PACK_DAMAGE, Q2MissionPackPlayerEffect};
use super::Q2MissionPackProjectiles;

/// Steering projection (`projectSteering`).
fn project_steering(entity: ActorId, game: &mut Q2GameServices, update: &WeaponTrajectoryUpdate) {
    let record = game.require_entity_mut(&entity);
    record.movedir = normalize3(update.velocity);
    record.speed = f64::from(length3(update.velocity));
}

/// Bolt callbacks (`Q2MissionPackBolts::callbacks`).
pub fn bolt_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.trajectory = vec![
        crate::q2::foundation::callbacks::TrajectoryProjection {
            touch: ion_touch as Q2Touch,
            project: project_steering as Q2TouchProject,
        },
        crate::q2::foundation::callbacks::TrajectoryProjection {
            touch: plasma_touch as Q2Touch,
            project: project_steering as Q2TouchProject,
        },
        crate::q2::foundation::callbacks::TrajectoryProjection {
            touch: flechette_touch as Q2Touch,
            project: project_steering as Q2TouchProject,
        },
        crate::q2::foundation::callbacks::TrajectoryProjection {
            touch: tracker_touch as Q2Touch,
            project: project_steering as Q2TouchProject,
        },
    ];
    callbacks.think.insert("missionpack.free", free_projectile);
    callbacks.think.insert("ionripper_sparks", ion_sparks as Q2Think);
    callbacks.think.insert("heat_think", heat_think as Q2Think);
    callbacks
        .think
        .insert("rerelease/heat_think", heat_think_rerelease as Q2Think);
    callbacks.think.insert("tracker_fly", tracker_fly as Q2Think);
    callbacks
        .think
        .insert("tracker_pain_daemon_think", tracker_pain as Q2Think);
    callbacks.touch.insert("ionripper_touch", ion_touch as Q2Touch);
    callbacks.touch.insert("plasma_touch", plasma_touch as Q2Touch);
    callbacks
        .touch
        .insert("flechette_touch", flechette_touch as Q2Touch);
    callbacks.touch.insert("blaster2_touch", green_touch as Q2Touch);
    callbacks
        .touch
        .insert("tracker_touch", tracker_touch as Q2Touch);
    callbacks
}

impl Q2MissionPackProjectiles {
    /// Register bolt callbacks (`register`).
    fn register_bolts(&self, game: &mut Q2GameServices) {
        game.source_callbacks.register(&bolt_callbacks());
    }

    /// Clear player collision for non-colliding owners (`playerCollision`).
    pub fn player_collision(&self, owner: &ActorId, game: &mut Q2GameServices, bolt: &ActorId) {
        if game.options.edition == Q2Edition::Rerelease
            && game.host.is_player(owner)
            && game
                .weapons
                .inputs
                .get(owner)
                .is_some_and(|input| !input.players_collide)
        {
            game.require_entity_mut(bolt).clip_mask &= !0x40000000;
        }
    }

    /// Emit impact noise for the owner (`impactNoise`).
    fn impact_noise(&self, entity: &ActorId, game: &mut Q2GameServices) {
        if let Some(owner) = game.require_entity(entity).owner.clone() {
            let origin = game.body_of(entity.clone()).origin;
            weapon_player_noise_for_actor(game, owner, origin, NoiseKind::Impact);
        }
    }

    /// Run the spawn touch (`initialTouch`).
    fn initial_touch(&self, owner: &ActorId, game: &mut Q2GameServices, bolt: &ActorId) {
        let start = game.body_of(owner.clone()).origin;
        let end = game.body_of(bolt.clone()).origin;
        let mask = game.require_entity(bolt).clip_mask;
        let trace = game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(bolt.clone()),
            mask,
            exclude: Vec::new(),
        });
        if trace.fraction == 1.0 || game.require_entity(bolt).touch.is_none() {
            return;
        }
        let rerelease = game.options.edition == Q2Edition::Rerelease;
        let plane = match &trace.contact {
            TraceContact::Plane { plane } => Some(*plane),
            _ => None,
        };
        let mut moved = game.body_of(bolt.clone());
        moved.origin = if rerelease {
            add3(
                trace.end,
                plane.map(|plane| plane.normal).unwrap_or_default(),
            )
        } else {
            add3(
                moved.origin,
                scale3(game.require_entity(bolt).movedir, -10.0),
            )
        };
        game.write_body(bolt.clone(), &moved, true);
        let other = match &trace.hit {
            TraceHit::Actor { actor } => actor.clone(),
            _ => game.host.world_actor(),
        };
        let surface = if rerelease {
            match &trace.family {
                TraceFamily::Q2(fields) => fields.surface.as_ref().map(|surface| {
                    crate::q2::support::contracts::TouchSurface {
                        name: surface.name.clone(),
                        native_flags: surface.flags,
                        native_value: surface.value,
                    }
                }),
                _ => None,
            }
        } else {
            None
        };
        let touch = game
            .require_entity(bolt)
            .touch
            .expect("mission-pack bolt lost its touch callback");
        let owned = game.owned_of(bolt.clone());
        touch(
            bolt.clone(),
            game,
            TouchContact {
                this: owned,
                other,
                plane: if rerelease { plane } else { None },
                surface,
                source_trace: None,
            },
        );
    }

    /// Fire an ion ripper (`fireIonRipper`).
    pub fn fire_ion_ripper(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        effects: i64,
    ) -> ActorId {
        self.register_bolts(game);
        let bolt = projectile(
            &owner,
            game,
            "ion",
            start,
            normalize3(direction),
            speed,
            "models/objects/boomrang/tris.md2",
            crate::q2::foundation::host::Q2MotionKind::WallBounce,
            effects,
        );
        self.player_collision(&owner, game, &bolt);
        {
            let entity = game.require_entity_mut(&bolt);
            entity.damage = damage;
            entity.damage_radius = 100.0;
            entity.render_flags = 8;
            entity.touch = Some(ion_touch as Q2Touch);
        }
        game.schedule(bolt.clone(), 3.0, ion_sparks as Q2Think);
        publish_projectile(
            bolt.clone(),
            game,
            "misc/lasfly.wav",
            Some(("q2:weapon_boomer", ProjectileRole::Bolt)),
        );
        let movedir = game.require_entity(&bolt).movedir;
        check_dodge(game, &owner, start, movedir, speed);
        self.initial_touch(&owner, game, &bolt);
        bolt
    }

    /// Fire a blue blaster (`fireBlueBlaster`).
    pub fn fire_blue_blaster(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        effects: i64,
    ) -> ActorId {
        let bolt = fire_blaster(
            owner,
            game,
            start,
            direction,
            damage,
            speed,
            effects,
            false,
            if game.options.edition == Q2Edition::Rerelease {
                58
            } else {
                1
            },
        );
        if game.host.actors().is_live(&bolt) {
            game.require_entity_mut(&bolt).model = "models/objects/blaser/tris.md2".to_string();
            game.show(bolt.clone());
        }
        bolt
    }

    /// Fire a heat rocket (`fireHeatRocket`).
    pub fn fire_heat_rocket(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        radius: f64,
        radius_damage: f64,
        turn_fraction: f64,
    ) -> ActorId {
        self.register_bolts(game);
        let rocket = crate::q2::foundation::weapons::ballistics::fire_rocket(
            owner,
            game,
            start,
            direction,
            damage,
            speed,
            radius,
            radius_damage,
        );
        if game.host.actors().is_live(&rocket) {
            {
                let entity = game.require_entity_mut(&rocket);
                entity.accel = turn_fraction;
                entity.speed = speed;
                entity.movedir = direction;
            }
            let rerelease = game.options.edition == Q2Edition::Rerelease;
            game.schedule(
                rocket.clone(),
                if rerelease {
                    game.host.frame_seconds()
                } else {
                    0.1
                },
                if rerelease {
                    heat_think_rerelease as Q2Think
                } else {
                    heat_think as Q2Think
                },
            );
        }
        rocket
    }

    /// Fire plasma (`firePlasma`).
    pub fn fire_plasma(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        radius: f64,
        radius_damage: f64,
    ) -> ActorId {
        self.register_bolts(game);
        let bolt = projectile(
            &owner,
            game,
            "plasma",
            start,
            direction,
            speed,
            "sprites/s_photon.sp2",
            crate::q2::foundation::host::Q2MotionKind::FlyMissile,
            0x1000000 | 0x2000,
        );
        self.player_collision(&owner, game, &bolt);
        {
            let entity = game.require_entity_mut(&bolt);
            entity.damage = damage;
            entity.damage_radius = radius;
            entity.radius_damage = radius_damage;
            entity.touch = Some(plasma_touch as Q2Touch);
        }
        game.schedule(bolt.clone(), 8000.0 / speed, free_projectile);
        publish_projectile(
            bolt.clone(),
            game,
            "weapons/rockfly.wav",
            Some(("q2:weapon_phalanx", ProjectileRole::Plasma)),
        );
        check_dodge(game, &owner, start, direction, speed);
        bolt
    }

    /// Fire a flechette (`fireFlechette`).
    pub fn fire_flechette(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        kick: f64,
    ) -> ActorId {
        self.register_bolts(game);
        let bolt = projectile(
            &owner,
            game,
            "flechette",
            start,
            normalize3(direction),
            speed,
            "models/proj/flechette/tris.md2",
            crate::q2::foundation::host::Q2MotionKind::FlyMissile,
            0,
        );
        self.player_collision(&owner, game, &bolt);
        {
            let entity = game.require_entity_mut(&bolt);
            entity.damage = damage;
            entity.damage_radius = kick;
            entity.render_flags = 8;
            entity.touch = Some(flechette_touch as Q2Touch);
        }
        game.schedule(bolt.clone(), 8000.0 / speed, free_projectile);
        publish_projectile(
            bolt.clone(),
            game,
            "",
            Some(("q2:weapon_etf_rifle", ProjectileRole::Nail)),
        );
        if game.options.edition == Q2Edition::Rerelease {
            self.initial_touch(&owner, game, &bolt);
        } else {
            let movedir = game.require_entity(&bolt).movedir;
            check_dodge(game, &owner, start, movedir, speed);
        }
        bolt
    }

    /// Fire a blaster2 bolt (`fireBlaster2`).
    pub fn fire_blaster2(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        effects: i64,
    ) -> ActorId {
        self.register_bolts(game);
        let bolt = projectile(
            &owner,
            game,
            "bolt",
            start,
            normalize3(direction),
            speed,
            "models/proj/laser2/tris.md2",
            crate::q2::foundation::host::Q2MotionKind::FlyMissile,
            effects | if effects == 0 { 0 } else { 0x4000000 },
        );
        self.player_collision(&owner, game, &bolt);
        if game.options.edition == Q2Edition::Rerelease {
            let entity = game.require_entity_mut(&bolt);
            entity.model = "models/objects/laser/tris.md2".to_string();
            entity.skin = 2;
            entity.scale = 2.5;
        }
        {
            let entity = game.require_entity_mut(&bolt);
            entity.damage = damage;
            entity.damage_radius = 128.0;
            entity.touch = Some(green_touch as Q2Touch);
        }
        game.schedule(bolt.clone(), 2.0, free_projectile);
        publish_projectile(bolt.clone(), game, "", None);
        let movedir = game.require_entity(&bolt).movedir;
        check_dodge(game, &owner, start, movedir, speed);
        self.initial_touch(&owner, game, &bolt);
        bolt
    }

    /// Fire a tracker (`fireTracker`).
    pub fn fire_tracker(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        enemy: Option<ActorId>,
    ) -> ActorId {
        self.register_bolts(game);
        let bolt = projectile(
            &owner,
            game,
            "tracker",
            start,
            normalize3(direction),
            speed,
            "models/proj/disintegrator/tris.md2",
            crate::q2::foundation::host::Q2MotionKind::FlyMissile,
            0x4000000,
        );
        self.player_collision(&owner, game, &bolt);
        {
            let entity = game.require_entity_mut(&bolt);
            entity.damage = damage;
            entity.enemy = enemy.clone();
            entity.touch = Some(tracker_touch as Q2Touch);
        }
        game.schedule(
            bolt.clone(),
            if enemy.is_none() { 10.0 } else { 0.1 },
            if enemy.is_none() {
                free_projectile
            } else {
                tracker_fly as Q2Think
            },
        );
        publish_projectile(
            bolt.clone(),
            game,
            "weapons/disrupt.wav",
            Some(("q2:weapon_disintegrator", ProjectileRole::Energy)),
        );
        let movedir = game.require_entity(&bolt).movedir;
        check_dodge(game, &owner, start, movedir, speed);
        self.initial_touch(&owner, game, &bolt);
        bolt
    }

    /// Fire a heat beam (`fireHeatBeam`).
    pub fn fire_heat_beam(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        _offset: Vec3,
        damage: f64,
        kick: f64,
    ) {
        let water_mask = 56;
        let end = add3(start, scale3(normalize3(direction), 8192.0));
        let underwater = game.host.point_contents(start) & water_mask != 0;
        let mut mask = projectile_mask(game);
        if game.options.edition == Q2Edition::Rerelease
            && game.host.is_player(&owner)
            && game
                .weapons
                .inputs
                .get(&owner)
                .is_some_and(|input| !input.players_collide)
        {
            mask &= !0x40000000;
        }
        let mut water_start = start;
        let mut water = false;
        let mut trace = game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(owner.clone()),
            mask: mask | if underwater { 0 } else { water_mask },
            exclude: Vec::new(),
        });
        if !matches!(trace.family, TraceFamily::Q1 { .. })
            && trace_contents(&trace) & water_mask != 0
        {
            water = true;
            water_start = trace.end;
            if length3(sub3(start, water_start)) != 0.0 {
                game.host.emit(crate::q2::foundation::host::Q2PresentationEvent::Effect(
                    crate::q2::foundation::host::Q2EffectEvent {
                        effect: "q2:heatbeam_sparks".to_string(),
                        origin: water_start,
                        direction: match &trace.contact {
                            TraceContact::Plane { plane } => plane.normal,
                            _ => Vec3::default(),
                        },
                        count: 0,
                        color: 0,
                    },
                ));
            }
            trace = game.host.trace(&Q2TraceRequest {
                start: water_start,
                end,
                bounds: None,
                ignore: Some(owner.clone()),
                mask,
                exclude: Vec::new(),
            });
        }
        let normal = match &trace.contact {
            TraceContact::Plane { plane } => plane.normal,
            _ => Vec3::default(),
        };
        let sky = match &trace.family {
            TraceFamily::Q2(fields) => {
                fields.surface.as_ref().map(|surface| surface.flags).unwrap_or(0) & 4 != 0
            }
            TraceFamily::Q3 { surface_flags, .. } => surface_flags & 4 != 0,
            TraceFamily::Q1 { .. } => false,
        };
        let actor = match &trace.hit {
            TraceHit::Actor { actor } => Some(actor.clone()),
            _ => None,
        };
        if !sky && trace.fraction < 1.0 {
            if actor.as_ref().is_some_and(|actor| {
                game.host
                    .combat()
                    .read(actor)
                    .is_some_and(|combat| combat.can_take_damage)
            }) {
                let target = actor.clone().expect("heatbeam target is missing");
                game.damage(
                    target,
                    owner.clone(),
                    Some(owner.clone()),
                    if water { (damage / 2.0).trunc() } else { damage },
                    kick,
                    direction,
                    trace.end,
                    normal,
                    Q2_MISSION_PACK_DAMAGE.heatbeam,
                    4,
                    Some("q2:weapon_plasmabeam".to_string()),
                );
            } else if !water
                && !matches!(&trace.family, TraceFamily::Q2(fields) if fields.surface.as_ref().is_some_and(|surface| surface.name.starts_with("sky")))
            {
                game.host.emit(crate::q2::foundation::host::Q2PresentationEvent::Effect(
                    crate::q2::foundation::host::Q2EffectEvent {
                        effect: "q2:heatbeam_steam".to_string(),
                        origin: trace.end,
                        direction: normal,
                        count: 0,
                        color: 0,
                    },
                ));
                weapon_player_noise(game, &owner, trace.end, NoiseKind::Impact);
            }
        }
        if water || underwater {
            let pos = add3(
                trace.end,
                scale3(normalize3(sub3(trace.end, water_start)), -2.0),
            );
            let water_end = if game.host.point_contents(pos) & 56 != 0 {
                pos
            } else {
                game.host
                    .trace(&Q2TraceRequest {
                        start: pos,
                        end: water_start,
                        bounds: None,
                        ignore: actor.clone(),
                        mask: 56,
                        exclude: Vec::new(),
                    })
                    .end
            };
            weapon_emit(
                game,
                &Q2WeaponEvent::Beam {
                    effect: WeaponBeamEffect::BubbleTrail,
                    actor: Some(owner.clone()),
                    start: water_start,
                    end: water_end,
                    duration: 0.0,
                },
            );
        }
        let heat_player = game.host.is_player(&owner);
        weapon_emit(
            game,
            &Q2WeaponEvent::Beam {
                effect: if heat_player {
                    WeaponBeamEffect::Heatbeam
                } else {
                    WeaponBeamEffect::MonsterHeatbeam
                },
                actor: Some(owner),
                start,
                end: trace.end,
                duration: 0.0,
            },
        );
    }
}

/// Trace contents flags.
fn trace_contents(trace: &crate::q2::support::contracts::TraceResult) -> i32 {
    match &trace.family {
        TraceFamily::Q2(fields) => fields.contents,
        TraceFamily::Q3 { contents, .. } => *contents,
        TraceFamily::Q1 { .. } => 0,
    }
}

/// Ion sparks think (`ionSparks`).
fn ion_sparks(entity: ActorId, game: &mut Q2GameServices) {
    let color = 0xe4 + (game.host.random() * 4.0).floor() as i32;
    effect(&entity, game, "welding_sparks", Vec3::default(), 0, color);
    game.remove_actor(entity);
}

/// Ion touch (`ionTouch`).
fn ion_touch(bolt: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if Some(&contact.other) == game.require_entity(&bolt).owner.as_ref() {
        return;
    }
    if contact.surface.as_ref().map(|surface| surface.native_flags).unwrap_or(0) & 4 != 0 {
        game.remove_actor(bolt);
        return;
    }
    super::mission_projectiles(game).impact_noise(&bolt, game);
    if !game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        return;
    }
    let body = game.body_of(bolt.clone());
    game.damage(
        contact.other.clone(),
        bolt.clone(),
        game.require_entity(&bolt).owner.clone(),
        game.require_entity(&bolt).damage,
        1.0,
        body.velocity,
        body.origin,
        contact.plane.map(|plane| plane.normal).unwrap_or_default(),
        Q2_MISSION_PACK_DAMAGE.ripper,
        4,
        Some("q2:weapon_boomer".to_string()),
    );
    game.remove_actor(bolt);
}

/// Heat think (`heatThink`).
fn heat_think(entity: ActorId, game: &mut Q2GameServices) {
    let mut nearest: Option<ActorId> = None;
    let mut nearest_distance = 0.0;
    let body = game.body_of(entity.clone());
    let origin = body.origin;
    let forward = angle_vectors(body.angles).forward;
    for actor in game.host.nearby(origin, 1024.0) {
        if Some(&actor) == game.require_entity(&entity).owner.as_ref()
            || !game.host.is_player(&actor)
            || game.host.combat().read(&actor).map(|combat| combat.health).unwrap_or(0.0) <= 0.0
            || !sight(game, &entity, &actor)
        {
            continue;
        }
        let Some(target) = game.host.bodies().read(&actor) else {
            continue;
        };
        let delta = sub3(target.origin, origin);
        let distance = f64::from(length3(delta).trunc());
        if dot3(normalize3(delta), forward) <= 0.3 {
            continue;
        }
        if nearest.is_none() || distance < nearest_distance {
            nearest = Some(actor);
            nearest_distance = distance;
        }
    }
    let target = nearest.as_ref().and_then(|actor| game.host.bodies().read(actor));
    if let Some(target) = target {
        game.require_entity_mut(&entity).enemy = nearest;
        if !matches!(game.host.weapon_behavior(), Some(port) if port.controls_trajectory(&entity)) {
            let movedir = normalize3(sub3(target.origin, origin));
            game.require_entity_mut(&entity).movedir = movedir;
            let mut moved = game.body_of(entity.clone());
            moved.angles = vector_angles(movedir);
            moved.velocity = scale3(movedir, 500.0);
            game.write_body(entity.clone(), &moved, true);
            let motion = game.require_entity(&entity).motion;
            game.set_motion_kind(entity.clone(), motion);
        }
    }
    game.schedule(entity, 0.1, heat_think as Q2Think);
}

/// Heat think rerelease (`heatThinkRerelease`).
fn heat_think_rerelease(entity: ActorId, game: &mut Q2GameServices) {
    let mut acquire: Option<ActorId> = None;
    let mut old_dot = 1.0;
    let mut old_distance = 0.0;
    let body = game.body_of(entity.clone());
    let origin = body.origin;
    let forward = angle_vectors(body.angles).forward;
    for actor in game.host.nearby(origin, 1024.0) {
        if Some(&actor) == game.require_entity(&entity).owner.as_ref()
            || !game.host.is_player(&actor)
            || game.host.combat().read(&actor).map(|combat| combat.health).unwrap_or(0.0) <= 0.0
            || !sight(game, &entity, &actor)
        {
            continue;
        }
        let Some(target) = game.host.bodies().read(&actor) else {
            continue;
        };
        let delta = sub3(origin, target.origin);
        let distance = f64::from(length3(delta));
        let alignment = dot3(normalize3(delta), forward);
        if alignment >= old_dot {
            continue;
        }
        if acquire.is_none() || alignment < old_dot || distance < old_distance {
            acquire = Some(actor);
            old_dot = alignment;
            old_distance = distance;
        }
    }
    let target = acquire.as_ref().and_then(|actor| game.host.bodies().read(actor));
    if target.is_none() {
        game.require_entity_mut(&entity).enemy = None;
    } else {
        let target = target.expect("heat target is missing");
        if !matches!(game.host.weapon_behavior(), Some(port) if port.controls_trajectory(&entity)) {
            let mut desired = normalize3(sub3(target.origin, origin));
            let movedir = game.require_entity(&entity).movedir;
            let alignment = dot3(movedir, desired);
            if alignment < 0.45 && alignment > -0.45 {
                desired = scale3(desired, -1.0);
            }
            let movedir = game.require_entity(&entity).movedir;
            let cosine = f64::from(dot3(movedir, desired));
            let angle = cosine.acos();
            let sine = angle.sin();
            let accel = game.require_entity(&entity).accel;
            let from = if cosine.abs() > 0.9995 {
                1.0 - accel
            } else {
                ((1.0 - accel) * angle).sin() / sine
            };
            let to = if cosine.abs() > 0.9995 {
                accel
            } else {
                (accel * angle).sin() / sine
            };
            let movedir = game.require_entity(&entity).movedir;
            let blended = normalize3(add3(scale3(movedir, from as f32), scale3(desired, to as f32)));
            game.require_entity_mut(&entity).movedir = blended;
            let mut moved = game.body_of(entity.clone());
            moved.angles = vector_angles(blended);
            game.write_body(entity.clone(), &moved, true);
        }
        if game.require_entity(&entity).enemy.is_none() {
            game.sound(&entity, "weapons/railgr1a.wav", 1, 1.0, 0.25);
            game.require_entity_mut(&entity).enemy = acquire;
        }
    }
    if !matches!(game.host.weapon_behavior(), Some(port) if port.controls_trajectory(&entity)) {
        let movedir = game.require_entity(&entity).movedir;
        let speed = game.require_entity(&entity).speed;
        let mut moved = game.body_of(entity.clone());
        moved.velocity = scale3(movedir, speed as f32);
        game.write_body(entity.clone(), &moved, true);
        let motion = game.require_entity(&entity).motion;
        game.set_motion_kind(entity.clone(), motion);
    }
    let frame_seconds = game.host.frame_seconds();
    game.schedule(entity, frame_seconds, heat_think_rerelease as Q2Think);
}

/// Plasma touch (`plasmaTouch`).
fn plasma_touch(bolt: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if Some(&contact.other) == game.require_entity(&bolt).owner.as_ref() {
        return;
    }
    if contact.surface.as_ref().map(|surface| surface.native_flags).unwrap_or(0) & 4 != 0 {
        game.remove_actor(bolt);
        return;
    }
    super::mission_projectiles(game).impact_noise(&bolt, game);
    let body = game.body_of(bolt.clone());
    if game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        game.damage(
            contact.other.clone(),
            bolt.clone(),
            game.require_entity(&bolt).owner.clone(),
            game.require_entity(&bolt).damage,
            0.0,
            body.velocity,
            body.origin,
            contact.plane.map(|plane| plane.normal).unwrap_or_default(),
            Q2_MISSION_PACK_DAMAGE.phalanx,
            0,
            Some("q2:weapon_phalanx".to_string()),
        );
    }
    let (owner, radius_damage, radius) = {
        let entity = game.require_entity(&bolt);
        (entity.owner.clone(), entity.radius_damage, entity.damage_radius)
    };
    game.radius_damage(bolt.clone(), owner, radius_damage, Some(contact.other), radius, Q2_MISSION_PACK_DAMAGE.phalanx, 0, Some("q2:weapon_phalanx".to_string()));
    game.host.emit(crate::q2::foundation::host::Q2PresentationEvent::Effect(
        crate::q2::foundation::host::Q2EffectEvent {
            effect: "q2:plasma_explosion".to_string(),
            origin: add3(body.origin, scale3(body.velocity, -0.02)),
            direction: Vec3::default(),
            count: 0,
            color: 0,
        },
    ));
    game.remove_actor(bolt);
}

/// Flechette touch (`flechetteTouch`).
fn flechette_touch(bolt: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if Some(&contact.other) == game.require_entity(&bolt).owner.as_ref() {
        return;
    }
    if contact.surface.as_ref().map(|surface| surface.native_flags).unwrap_or(0) & 4 != 0 {
        game.remove_actor(bolt);
        return;
    }
    if game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        let body = game.body_of(bolt.clone());
        game.damage(
            contact.other,
            bolt.clone(),
            game.require_entity(&bolt).owner.clone(),
            game.require_entity(&bolt).damage,
            game.require_entity(&bolt).damage_radius,
            body.velocity,
            body.origin,
            contact.plane.map(|plane| plane.normal).unwrap_or_default(),
            Q2_MISSION_PACK_DAMAGE.flechette,
            128,
            Some("q2:weapon_etf_rifle".to_string()),
        );
    } else {
        effect(
            &bolt,
            game,
            "flechette",
            contact.plane.map(|plane| plane.normal).unwrap_or_default(),
            1,
            0,
        );
    }
    game.remove_actor(bolt);
}

/// Green touch (`greenTouch`).
fn green_touch(bolt: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if Some(&contact.other) == game.require_entity(&bolt).owner.as_ref() {
        return;
    }
    if contact.surface.as_ref().map(|surface| surface.native_flags).unwrap_or(0) & 4 != 0 {
        game.remove_actor(bolt);
        return;
    }
    super::mission_projectiles(game).impact_noise(&bolt, game);
    let origin = game.body_of(bolt.clone()).origin;
    let hurt = game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|combat| combat.can_take_damage);
    let damage = game.require_entity(&bolt).damage;
    if damage >= 5.0 {
        let radius = game.require_entity(&bolt).damage_radius;
        let rerelease = game.options.edition == Q2Edition::Rerelease;
        for actor in game.host.nearby(origin, radius) {
            let owner = game.require_entity(&bolt).owner.clone();
            if Some(&actor) == owner.as_ref()
                || hurt && actor == contact.other
                || !game
                    .host
                    .combat()
                    .read(&actor)
                    .is_some_and(|combat| combat.can_take_damage)
            {
                continue;
            }
            let Some(body) = game.host.bodies().read(&actor) else {
                continue;
            };
            if !game.can_damage(&actor, &bolt) {
                continue;
            }
            let center = add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5));
            let points = damage * if rerelease { 2.0 } else { 3.0 }
                - 0.5 * f64::from(length3(sub3(center, origin)));
            if points > 0.0 {
                game.damage(
                    actor,
                    bolt.clone(),
                    game.require_entity(&bolt).owner.clone(),
                    points.trunc(),
                    points.trunc(),
                    sub3(body.origin, origin),
                    origin,
                    Vec3::default(),
                    0,
                    if rerelease { 5 } else { 1 },
                    None,
                );
            }
        }
    }
    if hurt {
        let body = game.body_of(bolt.clone());
        let owner = game.require_entity(&bolt).owner.clone();
        let sphere = owner.as_ref().is_some_and(|owner| game.host.is_player(owner));
        game.damage(
            contact.other,
            bolt.clone(),
            game.require_entity(&bolt).owner.clone(),
            game.require_entity(&bolt).damage,
            1.0,
            body.velocity,
            origin,
            contact.plane.map(|plane| plane.normal).unwrap_or_default(),
            if sphere {
                Q2_MISSION_PACK_DAMAGE.defender_sphere
            } else {
                Q2_MISSION_PACK_DAMAGE.blaster2
            },
            4,
            None,
        );
    } else {
        effect(
            &bolt,
            game,
            "blaster2",
            contact.plane.map(|plane| plane.normal).unwrap_or_default(),
            1,
            0,
        );
    }
    game.remove_actor(bolt);
}

/// Tracker fly think (`trackerFly`).
fn tracker_fly(entity: ActorId, game: &mut Q2GameServices) {
    let enemy = game.require_entity(&entity).enemy.clone();
    let target = enemy.as_ref().and_then(|enemy| game.host.bodies().read(enemy));
    let dead = enemy.as_ref().is_none_or(|enemy| {
        !game.host.actors().is_live(enemy)
            || game.host.combat().read(enemy).map(|combat| combat.health).unwrap_or(0.0) < 1.0
    });
    if enemy.is_none() || target.is_none() || dead {
        explode(&entity, game, "tracker_explosion");
        return;
    }
    let enemy = enemy.expect("tracker enemy is missing");
    let target = target.expect("tracker target is missing");
    let min = add3(target.origin, target.bounds.min);
    let max = add3(target.origin, target.bounds.max);
    let destination = if game.host.is_player(&enemy) {
        add3(target.origin, vec3(0.0, 0.0, game.entity(&enemy).map(|entity| entity.view_height as f32).unwrap_or(22.0)))
    } else if length3(min) == 0.0 || length3(max) == 0.0 {
        target.origin
    } else {
        scale3(add3(min, max), 0.5)
    };
    if !matches!(game.host.weapon_behavior(), Some(port) if port.controls_trajectory(&entity)) {
        let movedir = normalize3(sub3(destination, game.body_of(entity.clone()).origin));
        game.require_entity_mut(&entity).movedir = movedir;
        let speed = game.require_entity(&entity).speed;
        let mut moved = game.body_of(entity.clone());
        moved.velocity = scale3(movedir, speed as f32);
        moved.angles = vector_angles(movedir);
        game.write_body(entity.clone(), &moved, true);
        let motion = game.require_entity(&entity).motion;
        game.set_motion_kind(entity.clone(), motion);
    }
    game.schedule(entity, 0.1, tracker_fly as Q2Think);
}

/// Tracker pain daemon think (`trackerPain`).
fn tracker_pain(entity: ActorId, game: &mut Q2GameServices) {
    let enemy = game.require_entity(&entity).enemy.clone();
    let target = enemy.as_ref().and_then(|enemy| game.entity(enemy).map(|entity| entity.actor.id().clone()));
    let body = enemy.as_ref().and_then(|enemy| game.host.bodies().read(enemy));
    let timestamp = game.require_entity(&entity).timestamp;
    let dead = enemy.as_ref().is_none_or(|enemy| {
        game.host.combat().read(enemy).map(|combat| combat.health).unwrap_or(0.0) <= 0.0
    });
    if enemy.is_none() || body.is_none() || game.host.now() - timestamp > 0.5 || dead {
        if let Some(target) = target {
            if !game.host.is_player(&target) {
                game.require_entity_mut(&target).effects &= !0x80000000u32 as i64;
                game.show(target);
            }
        }
        game.remove_actor(entity);
        return;
    }
    let enemy = enemy.expect("tracker enemy is missing");
    let body = body.expect("tracker body is missing");
    let point = if game.options.edition == Q2Edition::Rerelease {
        add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5))
    } else {
        body.origin
    };
    let interval = if game.options.edition == Q2Edition::Rerelease {
        0.1
    } else {
        game.host.frame_seconds()
    };
    let (owner, damage) = {
        let entity = game.require_entity(&entity);
        (entity.owner.clone(), entity.damage)
    };
    game.damage(
        enemy.clone(),
        entity.clone(),
        owner.clone(),
        damage,
        0.0,
        Vec3::default(),
        point,
        vec3(0.0, 0.0, 1.0),
        Q2_MISSION_PACK_DAMAGE.tracker,
        268,
        Some("q2:weapon_disintegrator".to_string()),
    );
    if !game.host.actors().is_live(&entity) {
        return;
    }
    if game.host.combat().read(&enemy).map(|combat| combat.health).unwrap_or(0.0) < 1.0 {
        let hooks = super::mission_hooks(game);
        let gib = (hooks.monster)(enemy.clone(), game)
            .map(|monster| monster.state().gib_health)
            .unwrap_or(0.0);
        game.damage(
            enemy.clone(),
            entity.clone(),
            owner,
            if gib == 0.0 { 500.0 } else { -gib },
            0.0,
            Vec3::default(),
            point,
            vec3(0.0, 0.0, 1.0),
            Q2_MISSION_PACK_DAMAGE.tracker,
            268,
            Some("q2:weapon_disintegrator".to_string()),
        );
    }
    if game.host.is_player(&enemy) {
        let hooks = super::mission_hooks(game);
        (hooks.player_effect)(Q2MissionPackPlayerEffect::TrackerPain {
            actor: enemy,
            until: game.host.now() + interval,
        });
    } else if let Some(target) = target {
        game.require_entity_mut(&target).effects |= 0x80000000u32 as i64;
        game.show(target);
    }
    game.schedule(entity, interval, tracker_pain as Q2Think);
}

/// Tracker touch (`trackerTouch`).
fn tracker_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if Some(&contact.other) == game.require_entity(&entity).owner.as_ref() {
        return;
    }
    if contact.surface.as_ref().map(|surface| surface.native_flags).unwrap_or(0) & 4 != 0 {
        game.remove_actor(entity);
        return;
    }
    let target = game.host.combat().read(&contact.other);
    let body = game.body_of(entity.clone());
    if target.is_some_and(|target| target.can_take_damage) {
        let creature = game.host.is_monster(&contact.other) || game.host.is_player(&contact.other);
        let health = game.host.combat().read(&contact.other).map(|combat| combat.health).unwrap_or(0.0);
        let live = creature && health > 0.0;
        let damage = game.require_entity(&entity).damage;
        game.damage(
            contact.other.clone(),
            entity.clone(),
            game.require_entity(&entity).owner.clone(),
            if live { 0.0 } else if creature { damage * 4.0 } else { damage },
            damage * 3.0,
            body.velocity,
            body.origin,
            contact.plane.map(|plane| plane.normal).unwrap_or_default(),
            Q2_MISSION_PACK_DAMAGE.tracker,
            260,
            Some("q2:weapon_disintegrator".to_string()),
        );
        if live {
            let target_body = game.host.bodies().read(&contact.other);
            let flags = game.entity(&contact.other).map(|entity| entity.flags).unwrap_or(0);
            if target_body.is_some() && flags & 3 == 0 {
                let target_body = target_body.expect("tracker target body is missing");
                velocity(
                    game,
                    &contact.other,
                    add3(target_body.velocity, vec3(0.0, 0.0, 140.0)),
                    false,
                );
            }
            let owner = game.require_entity(&entity).owner.clone();
            let daemon = game.create("pain daemon", std::collections::BTreeMap::new());
            let damage = game.require_entity(&entity).damage;
            let tracker_rerelease = game.options.edition == Q2Edition::Rerelease;
            let interval = if tracker_rerelease { 0.1 } else { game.host.frame_seconds() };
            let scaled = (damage * interval / 0.5).trunc();
            let now = game.host.now();
            {
                let record = game.require_entity_mut(&daemon);
                record.owner = owner;
                record.enemy = Some(contact.other);
                record.damage = scaled;
                record.timestamp = now;
            }
            let rerelease = game.options.edition == Q2Edition::Rerelease;
            game.schedule(daemon, if rerelease { 0.0 } else { game.host.frame_seconds() }, tracker_pain as Q2Think);
        }
    }
    explode(&entity, game, "tracker_explosion");
}
