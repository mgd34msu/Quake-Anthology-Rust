//! Mission-pack projectile helpers (`src/content/q2/missionpacks/projectiles/common.ts`).

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{vec3, Vec3};

use crate::contract::ProjectileRole;
use crate::q2::foundation::host::{
    Q2Edition, Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop,
    Q2Think, Q2TraceRequest,
};
use crate::q2::foundation::weapons::vectors::vector_angles;
use crate::q2::support::contracts::WeaponBehaviorLaunch;

/// Projectile shot mask.
pub const SHOT_MASK: i32 = 0x6000003;

/// Projectile mask (`projectileMask`).
pub fn projectile_mask(game: &Q2GameServices) -> i32 {
    if game.options.edition == Q2Edition::Rerelease {
        0x46004003
    } else {
        SHOT_MASK
    }
}

/// Free a projectile (`freeProjectile`).
pub fn free_projectile(projectile: ActorId, game: &mut Q2GameServices) {
    game.remove_actor(projectile);
}

/// Spawn a projectile record (`projectile`).
#[allow(clippy::too_many_arguments)]
pub fn projectile(
    owner: &ActorId,
    game: &mut Q2GameServices,
    classname: &str,
    start: Vec3,
    direction: Vec3,
    speed: f64,
    model: &str,
    motion: Q2MotionKind,
    effects: i64,
) -> ActorId {
    let entity = game.create(classname, BTreeMap::new());
    {
        let record = game.require_entity_mut(&entity);
        record.owner = Some(owner.clone());
        record.projectile = true;
        record.dodgeable = true;
        record.speed = speed;
        record.model = model.to_string();
        record.effects = effects;
        record.motion = motion;
        record.movedir = direction;
    }
    let mask = projectile_mask(game);
    game.require_entity_mut(&entity).clip_mask = mask;
    let mut moved = game.body_of(entity.clone());
    moved.origin = start;
    moved.velocity = vec3(
        direction.x * speed as f32,
        direction.y * speed as f32,
        direction.z * speed as f32,
    );
    moved.angles = vector_angles(direction);
    moved.bounds.min = Vec3::default();
    moved.bounds.max = Vec3::default();
    game.write_body(entity.clone(), &moved, false);
    entity
}

/// Publish a projectile (`publishProjectile`).
pub fn publish_projectile(
    entity: ActorId,
    game: &mut Q2GameServices,
    sound: &str,
    behavior: Option<(&str, ProjectileRole)>,
) {
    game.set_solid(entity.clone(), Q2Solid::Box);
    let motion = game.require_entity(&entity).motion;
    game.set_motion_kind(entity.clone(), motion);
    if let Some((weapon, role)) = behavior {
        let owner = game.require_entity(&entity).owner.clone();
        if let Some(owner) = owner {
            if game.host.is_player(&owner) {
                let owned = game.owned_of(entity.clone());
                let body = game.body_of(entity.clone());
                let input = WeaponBehaviorLaunch {
                    projectile: owned,
                    shooter: owner,
                    weapon: weapon.to_string(),
                    role,
                    time_seconds: game.host.now(),
                    body,
                };
                let update = match game.host.weapon_behavior() {
                    Some(port) => port.launch(&input),
                    None => None,
                };
                if let Some(update) = update {
                    game.project_trajectory(entity.clone(), &update);
                }
            }
        }
    }
    game.show(entity.clone());
    if sound.is_empty() {
        return;
    }
    let origin = game.body_of(entity.clone()).origin;
    game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(entity),
        origin,
        path: sound.to_string(),
        channel: 0,
        volume: 1.0,
        attenuation: 1.0,
        reliable: false,
        loop_: Q2SoundLoop::Start,
        loop_owner: None,
    }));
}

/// Emit an effect at an entity (`effect`).
pub fn effect(entity: &ActorId, game: &mut Q2GameServices, name: &str, direction: Vec3, count: i32, color: i32) {
    let origin = game.body_of(entity.clone()).origin;
    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: format!("q2:{name}"),
        origin,
        direction,
        count,
        color,
    }));
}

/// Explode and remove an entity (`explode`).
pub fn explode(entity: &ActorId, game: &mut Q2GameServices, name: &str) {
    effect(entity, game, name, Vec3::default(), 1, 0);
    game.remove_actor(entity.clone());
}

/// Check line of sight (`sight`).
pub fn sight(game: &mut Q2GameServices, from: &ActorId, target: &ActorId) -> bool {
    let Some(body) = game.host.bodies().read(target) else {
        return false;
    };
    let eye = game
        .entity(target)
        .map(|entity| entity.view_height)
        .unwrap_or(if game.host.is_player(target) { 22 } else { 0 });
    let from_body = game.body_of(from.clone());
    let from_height = game.require_entity(from).view_height;
    let trace = game.host.trace(&Q2TraceRequest {
        start: vec3(
            from_body.origin.x,
            from_body.origin.y,
            from_body.origin.z + from_height as f32,
        ),
        end: vec3(body.origin.x, body.origin.y, body.origin.z + eye as f32),
        bounds: None,
        ignore: Some(from.clone()),
        mask: 25,
        exclude: Vec::new(),
    });
    trace.fraction == 1.0
}

/// Write an actor velocity (`velocity`).
pub fn velocity(game: &mut Q2GameServices, actor: &ActorId, value: Vec3, lift_ground: bool) {
    let body = game.host.bodies().read(actor);
    let owned = game.host.actors().resolve_owned(actor);
    let (Some(body), Some(owned)) = (body, owned) else {
        return;
    };
    let mut next = body;
    next.velocity = value;
    if lift_ground {
        next.ground = None;
    }
    game.host.bodies().write(&owned, &next);
    if let Some(entity) = game.entity(actor) {
        let motion = entity.motion;
        game.set_motion_kind(actor.clone(), motion);
    }
}

/// Free-projectile think callback.
pub const FREE_PROJECTILE: Q2Think = free_projectile;
