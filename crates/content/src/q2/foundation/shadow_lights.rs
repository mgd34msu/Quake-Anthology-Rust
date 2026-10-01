//! Rerelease shadow lights (`src/content/q2/foundation/shadow-lights.ts`).
//!
//! Rerelease `g_misc.cpp` `setup_dynamic_light`/`setup_shadow_lights`
//! (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{normalize3_or_zero, sub3, Vec3};

use super::fields::{integer_field, number_field, ZERO};
use super::host::{Q2GameServices, Q2PresentationEvent, Q2Use};

/// Rerelease shadow light state (`Q2ShadowLightState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ShadowLightState {
    /// Light actor.
    pub actor: ActorId,
    /// Light origin.
    pub origin: Vec3,
    /// Light color.
    pub color: Vec3,
    /// Whether the light is visible.
    pub visible: bool,
    /// Light radius.
    pub radius: f64,
    /// Light intensity.
    pub intensity: f64,
    /// Shadow resolution.
    pub resolution: i32,
    /// Fade start distance.
    pub fade_start: f64,
    /// Fade end distance.
    pub fade_end: f64,
    /// Light style.
    pub lightstyle: i32,
    /// Spot cone.
    pub cone: Option<Q2ShadowCone>,
}

/// Shadow light spot cone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ShadowCone {
    /// Cone direction.
    pub direction: Vec3,
    /// Cosine of the half angle.
    pub cos_half_angle: f64,
}

/// Toggle a dynamic light (`dynamic_light_use`).
pub fn dynamic_light_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let visible = {
        let entity = game.require_entity_mut(&actor);
        entity.server_flags ^= 1;
        entity.visible = entity.server_flags & 1 == 0;
        entity.visible
    };
    let _ = visible;
    emit_q2_shadow_light(actor, game);
}

/// Spawn a shadow light (`spawnQ2ShadowLight`).
pub fn spawn_q2_shadow_light(actor: ActorId, game: &mut Q2GameServices) {
    let has_radius = {
        let entity = game.require_entity(&actor);
        number_field(&entity.spawn, "shadowlightradius", 0.0) > 0.0
    };
    if has_radius {
        game.require_entity_mut(&actor).render_flags = 1 << 14;
        let mut body = game.body_of(actor.clone());
        body.bounds = qa_core::math::Bounds { min: ZERO, max: ZERO };
        game.write_body(actor.clone(), &body, false);
        game.link_actor(actor.clone());
    }
    let (has_targetname, toggled) = {
        let entity = game.require_entity(&actor);
        (entity.targetname.clone(), entity.spawnflags)
    };
    if !has_targetname.is_empty() {
        game.require_entity_mut(&actor).use_ = Some(dynamic_light_use as Q2Use);
    }
    if toggled & 1 != 0 {
        game.require_entity_mut(&actor).server_flags ^= 1;
    }
    let visible = game.require_entity(&actor).server_flags & 1 == 0;
    game.require_entity_mut(&actor).visible = visible;
}

/// Emit one shadow light (`emitQ2ShadowLight`).
pub fn emit_q2_shadow_light(actor: ActorId, game: &mut Q2GameServices) {
    let (target_name, style_target, packed, server_flags) = {
        let entity = game.require_entity(&actor);
        (
            entity.target.clone(),
            entity.spawn.values.get("shadowlightstyletarget").cloned(),
            entity.skin,
            entity.server_flags,
        )
    };
    let target = if target_name.is_empty() {
        None
    } else {
        game.entities
            .values()
            .find(|candidate| candidate.targetname == target_name)
            .map(|entity| entity.actor.id().clone())
    };
    let style = style_target.as_ref().and_then(|name| {
        game.entities
            .values()
            .find(|candidate| &candidate.targetname == name)
            .map(|entity| entity.style)
    });
    let origin = game.body_of(actor.clone()).origin;
    let color = if packed == 0 {
        Vec3 { x: 1.0, y: 1.0, z: 1.0 }
    } else {
        let bits = packed as u32;
        Vec3 {
            x: (bits >> 24 & 255) as f32 / 255.0,
            y: (bits >> 16 & 255) as f32 / 255.0,
            z: (bits >> 8 & 255) as f32 / 255.0,
        }
    };
    let (radius, intensity, resolution, fade_start, fade_end, lightstyle, cone_angle) = {
        let entity = game.require_entity(&actor);
        (
            number_field(&entity.spawn, "shadowlightradius", 0.0),
            number_field(&entity.spawn, "shadowlightintensity", 1.0),
            integer_field(&entity.spawn, "shadowlightresolution", 0),
            number_field(&entity.spawn, "shadowlightstartfadedistance", 0.0),
            number_field(&entity.spawn, "shadowlightendfadedistance", 0.0),
            style.unwrap_or_else(|| integer_field(&entity.spawn, "shadowlightstyle", -1)),
            number_field(&entity.spawn, "shadowlightconeangle", 45.0),
        )
    };
    let cone = target.map(|target| {
        let direction = normalize3_or_zero(sub3(game.body_of(target).origin, origin));
        Q2ShadowCone {
            direction,
            cos_half_angle: (cone_angle * std::f64::consts::PI / 180.0).cos(),
        }
    });
    game.host_emit(Q2PresentationEvent::DynamicLight(Q2ShadowLightState {
        actor,
        origin,
        color,
        visible: server_flags & 1 == 0,
        radius,
        intensity,
        resolution,
        fade_start,
        fade_end,
        lightstyle,
        cone,
    }));
}

/// Emit all shadow lights (`emitQ2ShadowLights`).
pub fn emit_q2_shadow_lights(game: &mut Q2GameServices) {
    let lights: Vec<ActorId> = game
        .entities
        .values()
        .filter(|entity| entity.classname == "dynamic_light")
        .map(|entity| entity.actor.id().clone())
        .collect();
    for actor in lights {
        emit_q2_shadow_light(actor, game);
    }
}
