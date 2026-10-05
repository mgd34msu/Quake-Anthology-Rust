//! Q2 rerelease lights (`src/content/q2/rerelease/lights.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_core::numeric::{native_atof, native_atoi};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{Q2GameServices, Q2Think, Q2Use};

use super::types::{q2_is_n64, Q2RereleaseEvent, Q2RereleaseHooks};

/// Parse a rerelease light color (`q2RereleaseColor`).
///
/// ED_LoadColor accepts packed integers, float RGBA, or byte RGBA.
pub fn q2_rerelease_color(value: &str) -> i32 {
    if !value.contains(' ') {
        return native_atoi(value);
    }
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let mut components = [0.0, 0.0, 0.0, 1.0];
    for (index, component) in components.iter_mut().enumerate() {
        if let Some(token) = tokens.get(index) {
            *component = native_atof(token);
        }
    }
    let multiplier = if components.iter().any(|component| *component > 1.0) {
        1.0
    } else {
        255.0
    };
    let channel = |index: usize| (components[index] * multiplier).trunc() as i32;
    channel(3) | channel(2) << 8 | channel(1) << 16 | channel(0) << 24
}

/// Rerelease lights (`Q2RereleaseLights`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleaseLights {
    /// Session hooks.
    pub hooks: Q2RereleaseHooks,
}

/// Rerelease light callbacks (`Q2RereleaseLights::callbacks`).
pub fn rerelease_light_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("rr.target_light_think", rerelease_light_think as Q2Think);
    callbacks
        .think
        .insert("rr.target_light_flicker_think", rerelease_light_flicker as Q2Think);
    callbacks
        .use_
        .insert("rr.target_light_use", rerelease_light_use as Q2Use);
    callbacks
}

/// Spawn rerelease lights (`Q2RereleaseLights::spawn`).
pub fn rerelease_light_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    Q2RereleaseLights {
        hooks: super::rerelease_hooks(game),
    }
    .spawn(entity, game)
}

/// Light use (`use`).
fn rerelease_light_use(
    entity: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    Q2RereleaseLights {
        hooks: super::rerelease_hooks(game),
    }
    .use_light(entity, game);
}

/// Light flicker think (`flicker`).
fn rerelease_light_flicker(entity: ActorId, game: &mut Q2GameServices) {
    Q2RereleaseLights {
        hooks: super::rerelease_hooks(game),
    }
    .flicker(entity, game);
}

/// Light style think (`think`).
fn rerelease_light_think(entity: ActorId, game: &mut Q2GameServices) {
    Q2RereleaseLights {
        hooks: super::rerelease_hooks(game),
    }
    .think(entity, game);
}

impl Q2RereleaseLights {
    /// Spawn a light.
    fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        if game.require_entity(&entity).classname != "target_light" {
            return false;
        }
        let radius = number_field(&game.require_entity(&entity).spawn, "radius", 0.0);
        let skin = game.require_entity(&entity).skin;
        let target = game.require_entity(&entity).target.clone();
        let start_on = game.require_entity(&entity).spawnflags & 1 != 0;
        {
            let record = game.require_entity_mut(&entity);
            record.visible = false;
            record.server_flags |= 1;
            record.frame = if radius == 0.0 { 150 } else { radius as i32 };
            record.count = skin;
        }
        game.rerelease.light_active.insert(entity.clone(), false);
        if !target.is_empty() {
            let chain = game.pick_target(&target);
            game.require_entity_mut(&entity).chain = chain;
        }
        game.require_entity_mut(&entity).use_ = Some(rerelease_light_use as Q2Use);
        if start_on {
            self.use_light(entity.clone(), game);
        }
        let speed = game.require_entity(&entity).speed;
        game.require_entity_mut(&entity).speed = if speed == 0.0 { 1.0 } else { 0.1 / speed };
        if q2_is_n64(game) {
            game.require_entity_mut(&entity).style += 10;
        }
        game.link_actor(entity.clone());
        self.show(&entity, game);
        true
    }

    /// Present a light.
    fn show(&self, entity: &ActorId, game: &mut Q2GameServices) {
        let visible = game.require_entity(entity).server_flags & 1 == 0;
        game.require_entity_mut(entity).visible = visible;
        let origin = game.body_of(entity.clone()).origin;
        let record = game.require_entity(entity);
        let radius = f64::from(record.frame);
        let skin = record.skin as u32;
        (self.hooks.emit)(
            game,
            Q2RereleaseEvent::DynamicLight {
                actor: entity.clone(),
                origin,
                radius,
                color: Vec3 {
                    x: ((skin >> 24) & 255) as f32 / 255.0,
                    y: ((skin >> 16) & 255) as f32 / 255.0,
                    z: ((skin >> 8) & 255) as f32 / 255.0,
                },
                visible,
            },
        );
    }

    /// Toggle a light.
    fn use_light(&self, entity: ActorId, game: &mut Q2GameServices) {
        let active = !game.rerelease.light_active.get(&entity).copied().unwrap_or(false);
        game.rerelease.light_active.insert(entity.clone(), active);
        if active {
            game.require_entity_mut(&entity).server_flags &= !1;
        } else {
            game.require_entity_mut(&entity).server_flags |= 1;
        }
        self.show(&entity, game);
        if !active {
            game.cancel_actor(entity);
            return;
        }
        if game.require_entity(&entity).chain.is_some() {
            game.schedule(entity, 0.1, rerelease_light_think as Q2Think);
        } else if game.require_entity(&entity).spawnflags & 4 != 0 {
            game.schedule(entity, 0.1, rerelease_light_flicker as Q2Think);
        }
    }

    /// Flicker a light.
    fn flicker(&self, entity: ActorId, game: &mut Q2GameServices) {
        if game.random() < 0.5 {
            game.require_entity_mut(&entity).server_flags ^= 1;
        }
        self.show(&entity, game);
        game.schedule(entity, 0.1, rerelease_light_flicker as Q2Think);
    }

    /// Animate a light from its source style.
    fn think(&self, entity: ActorId, game: &mut Q2GameServices) {
        if game.require_entity(&entity).spawnflags & 4 != 0 && game.random() < 0.5 {
            game.require_entity_mut(&entity).server_flags ^= 1;
        }
        let style_number = game.require_entity(&entity).style;
        let style = (self.hooks.light_style)(game, style_number);
        if style.is_empty() {
            panic!("target_light requires source lightstyle {style_number}");
        }
        let chain = game.require_entity(&entity).chain.clone();
        let target = chain.as_ref().and_then(|chain| game.entity(chain));
        let Some(target) = target else {
            panic!("target_light lost its source color target");
        };
        let target_skin = target.skin as u32;
        let speed = game.require_entity(&entity).speed;
        let record = game.require_entity_mut(&entity);
        record.delay += speed;
        let delay = record.delay;
        let pattern = style.as_bytes();
        let index = (delay.trunc() as usize) % pattern.len();
        let current = (f64::from(pattern[index]) - 97.0) / 25.0;
        let fraction = delay.fract();
        let next = (f64::from(pattern[(index + 1) % pattern.len()]) - 97.0) / 25.0;
        let lerp = if record.spawnflags & 2 != 0 {
            current
        } else {
            next * fraction + current * (1.0 - fraction)
        };
        let count = record.count as u32;
        let channel = |shift: u32| {
            (((target_skin >> shift) & 255) as f64 * lerp + ((count >> shift) & 255) as f64 * (1.0 - lerp)).trunc()
                as i32
        };
        record.skin = channel(8) << 8 | channel(16) << 16 | channel(24) << 24;
        self.show(&entity, game);
        game.schedule(entity, 0.1, rerelease_light_think as Q2Think);
    }
}
