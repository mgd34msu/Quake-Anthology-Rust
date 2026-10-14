//! Shared model lighting values, resolved before either drawing backend.
use crate::{
    scene::{Light, SceneEntity},
    stage::EntityLighting,
};
use qa_core::{math, primitives::Vec3};

/// R_SetupEntityLighting's NOWORLDMODEL / missing-grid sample.
pub fn unlit_sample(identity_light: f32, sun_direction: Vec3) -> EntityLighting {
    let value = identity_light * 150.0;
    EntityLighting {
        ambient: [value; 3],
        directed: [value; 3],
        direction: sun_direction,
    }
}

/// A grid or legacy point-light sampler supplies world-space lighting. Keep
/// native minimum-light, dynamic-light and local-axis operation order here.
pub fn setup_entity(
    entity: &SceneEntity,
    sample: EntityLighting,
    identity_light: f32,
    lights: &[Light],
) -> EntityLighting {
    let minimum = identity_light * 32.0;
    let mut ambient = Vec3(sample.ambient) + Vec3([minimum; 3]);
    let mut directed = Vec3(sample.directed);
    let mut direction = sample.direction * math::length(directed);
    let origin = entity.lighting_origin.unwrap_or(entity.origin);
    for light in lights {
        let mut relative = light.origin - origin;
        let distance = math::normalize(&mut relative).max(16.0);
        let power = 16.0 * (light.radius * light.radius);
        let attenuation = power / (distance * distance);
        directed = directed + Vec3(light.color) * attenuation;
        direction = direction + relative * attenuation;
    }
    let maximum = (identity_light * 255.0) as i32 as f32;
    for value in &mut ambient.0 {
        if *value > maximum {
            *value = maximum;
        }
    }
    math::normalize(&mut direction);
    EntityLighting {
        ambient: ambient.0,
        directed: directed.0,
        direction: Vec3(entity.axes.map(|axis| direction.dot(axis))),
    }
}
