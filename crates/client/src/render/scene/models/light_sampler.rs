//! Model light sampling (donor `src/render/scene/models/light-sampler.ts`).
//!
//! Q3 grid sampling plus dynamic-light accumulation; legacy BSP tracing is
//! supplied by the host scene through an injectable static sampler.

use qa_core::math::{add3, length3, normalize3_or_zero, scale3, sub3, vec3, Plane, Vec3};

use crate::materials::q3_lighting::{
    light_for_point, setup_entity_lighting, DynamicLight, EntityLighting, EntityLightingState, LightGrid,
    LightingEntity, LightingScales,
};
use crate::render::error::RenderError;

use super::types::{EntityFlags, SceneEntity};

/// Normalized, unclamped light sample with an optional floor hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelLightSample {
    /// Normalized RGB; fullbright maps return one in every channel.
    pub color: Vec3,
    /// Floor hit below the sample point.
    pub floor: Option<LightFloor>,
}

/// Floor plane hit below a light sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightFloor {
    /// Hit point.
    pub point: Vec3,
    /// Hit plane.
    pub plane: Plane,
}

/// Dynamic point light for model sampling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerDynamicLight {
    /// World origin.
    pub origin: Vec3,
    /// Radius in world units.
    pub radius: f32,
    /// Light color.
    pub color: Vec3,
}

/// Lights visible to one sampling pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelLightViewInput {
    /// Legacy dynamic lights.
    pub dynamic: Vec<SamplerDynamicLight>,
    /// Q3 dynamic lights.
    pub q3_dynamic: Vec<DynamicLight>,
    /// Identity-light scale.
    pub identity_light: f32,
}

/// Static BSP light callback supplied by the host scene.
pub type StaticLightSampler = dyn Fn(Vec3) -> ModelLightSample + Send + Sync;

/// Model light sampler over a Q3 grid and host static lighting.
pub struct ModelLightSampler {
    /// Q3 light grid, when the world provides one.
    pub grid: Option<LightGrid>,
    /// Host static-light callback (BSP trace); fullbright when absent.
    pub static_sampler: Option<Box<StaticLightSampler>>,
    /// Q2 light modulation scale.
    pub q2_light_modulate: f32,
    /// Sun direction fallback for entity lighting.
    pub sun_direction: Vec3,
}

impl std::fmt::Debug for ModelLightSampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelLightSampler")
            .field("grid", &self.grid.is_some())
            .field("static_sampler", &self.static_sampler.is_some())
            .field("q2_light_modulate", &self.q2_light_modulate)
            .finish()
    }
}

impl ModelLightSampler {
    /// Sampler with fullbright static lighting.
    #[must_use]
    pub fn fullbright() -> Self {
        Self {
            grid: None,
            static_sampler: None,
            q2_light_modulate: 1.0,
            sun_direction: normalize3_or_zero(vec3(0.45, 0.3, 0.9)),
        }
    }

    /// Sample normalized light at a point, adding dynamic lights on request.
    pub fn sample(
        &self,
        point: Vec3,
        input: &ModelLightViewInput,
        include_dynamic: bool,
    ) -> Result<ModelLightSample, RenderError> {
        let mut sample = if let Some(grid) = &self.grid {
            let light = light_for_point(
                Some(grid),
                &point,
                &LightingScales {
                    ambient_scale: 1.0,
                    directed_scale: 1.0,
                },
            )
            .map_err(|error| RenderError::Backend(error.to_string()))?;
            let color = light.map_or(vec3(1.0, 1.0, 1.0), |sample| {
                scale3(add3(sample.ambient_light, sample.directed_light), 1.0 / 255.0)
            });
            ModelLightSample { color, floor: None }
        } else if let Some(sampler) = &self.static_sampler {
            sampler(point)
        } else {
            ModelLightSample {
                color: vec3(1.0, 1.0, 1.0),
                floor: None,
            }
        };
        if include_dynamic {
            sample.color = self.dynamic(sample.color, point, input);
        }
        Ok(sample)
    }

    fn dynamic(&self, color: Vec3, point: Vec3, input: &ModelLightViewInput) -> Vec3 {
        let mut result = color;
        for light in &input.dynamic {
            let amount = (light.radius - length3(sub3(point, light.origin))) / 256.0;
            if amount > 0.0 {
                result = add3(result, scale3(light.color, amount));
            }
        }
        result
    }

    /// Full entity lighting: Q3 grid setup, or foreign-lightmap ambient.
    pub fn entity_lighting(
        &self,
        entity: &SceneEntity,
        input: &ModelLightViewInput,
        no_world_model: bool,
    ) -> Result<EntityLighting, RenderError> {
        let identity_light = if input.identity_light == 0.0 {
            1.0
        } else {
            input.identity_light
        };
        let axis = [
            scale3(entity.transform.axis[0], entity.transform.scale.x),
            scale3(entity.transform.axis[1], entity.transform.scale.y),
            scale3(entity.transform.axis[2], entity.transform.scale.z),
        ];
        if self.grid.is_some() || no_world_model {
            let render_flags = match entity.flags {
                EntityFlags::Q3 { bits } => bits,
                _ => 0,
            };
            return setup_entity_lighting(
                &LightingEntity {
                    origin: entity.transform.origin,
                    lighting_origin: entity.lighting_origin,
                    axis,
                    render_flags,
                },
                &EntityLightingState {
                    ambient_scale: 1.0,
                    directed_scale: 1.0,
                    grid: self.grid.clone(),
                    no_world_model,
                    identity_light,
                    identity_light_byte: (identity_light * 255.0).trunc(),
                    sun_direction: self.sun_direction,
                    dynamic_lights: input.q3_dynamic.clone(),
                },
                None,
            )
            .map_err(|error| RenderError::Backend(error.to_string()));
        }
        let point = match entity.flags {
            EntityFlags::Q3 { bits } if bits & 128 != 0 => entity.lighting_origin,
            _ => entity.transform.origin,
        };
        let color = self.sample(point, input, true)?.color;
        let ambient = vec3(
            (identity_light * 255.0).min(color.x * 255.0 + identity_light * 32.0),
            (identity_light * 255.0).min(color.y * 255.0 + identity_light * 32.0),
            (identity_light * 255.0).min(color.z * 255.0 + identity_light * 32.0),
        );
        let packet = |value: f32| (value.trunc().clamp(0.0, 255.0) as u32) & 255;
        Ok(EntityLighting {
            ambient_light: ambient,
            directed_light: vec3(0.0, 0.0, 0.0),
            light_dir: vec3(0.0, 0.0, 1.0),
            ambient_light_int: packet(ambient.x) | (packet(ambient.y) << 8) | (packet(ambient.z) << 16) | 0xff00_0000,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{EntityFlags, EntityTransform, ModelResource, SceneModel, ScenePose};
    use super::*;
    use qa_core::math::{vec3, vec4};

    fn entity() -> SceneEntity {
        SceneEntity {
            resource: ModelResource {
                id: "test".to_string(),
                requested_path: "test".to_string(),
                digest: 0,
            },
            model: SceneModel::BrushModel,
            pose: ScenePose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            transform: EntityTransform::identity(),
            previous_origin: vec3(0.0, 0.0, 0.0),
            lighting_origin: vec3(0.0, 0.0, 0.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            skin: 0,
            shader_time_seconds: 0.0,
            flags: EntityFlags::Q3 { bits: 0 },
            attachments: Vec::new(),
            actor_slot: None,
        }
    }

    #[test]
    fn fullbright_sampler_returns_white() {
        let sampler = ModelLightSampler::fullbright();
        let input = ModelLightViewInput::default();
        let sample = sampler.sample(vec3(0.0, 0.0, 0.0), &input, false).expect("sample");
        assert_eq!(sample.color, vec3(1.0, 1.0, 1.0));
        assert!(sample.floor.is_none());
    }

    #[test]
    fn dynamic_light_falls_off_with_distance() {
        let sampler = ModelLightSampler::fullbright();
        let input = ModelLightViewInput {
            dynamic: vec![SamplerDynamicLight {
                origin: vec3(0.0, 0.0, 0.0),
                radius: 256.0,
                color: vec3(1.0, 0.0, 0.0),
            }],
            ..ModelLightViewInput::default()
        };
        let near = sampler.sample(vec3(0.0, 0.0, 0.0), &input, true).expect("near").color;
        let far = sampler.sample(vec3(200.0, 0.0, 0.0), &input, true).expect("far").color;
        assert!(near.x > far.x);
        let excluded = sampler
            .sample(vec3(0.0, 0.0, 0.0), &input, false)
            .expect("excluded")
            .color;
        assert_eq!(excluded, vec3(1.0, 1.0, 1.0));
    }

    #[test]
    fn static_sampler_supplies_legacy_light() {
        let sampler = ModelLightSampler {
            static_sampler: Some(Box::new(|_| ModelLightSample {
                color: vec3(0.25, 0.5, 1.0),
                floor: None,
            })),
            ..ModelLightSampler::fullbright()
        };
        let sample = sampler
            .sample(vec3(0.0, 0.0, 0.0), &ModelLightViewInput::default(), false)
            .expect("sample");
        assert_eq!(sample.color, vec3(0.25, 0.5, 1.0));
    }

    #[test]
    fn entity_lighting_without_world_model_succeeds() {
        let sampler = ModelLightSampler::fullbright();
        let lighting = sampler
            .entity_lighting(&entity(), &ModelLightViewInput::default(), true)
            .expect("lighting");
        assert!(lighting.ambient_light.x > 0.0);
    }

    #[test]
    fn legacy_entity_lighting_builds_ambient_packet() {
        let sampler = ModelLightSampler {
            static_sampler: Some(Box::new(|_| ModelLightSample {
                color: vec3(0.5, 0.5, 0.5),
                floor: None,
            })),
            ..ModelLightSampler::fullbright()
        };
        let lighting = sampler
            .entity_lighting(&entity(), &ModelLightViewInput::default(), false)
            .expect("lighting");
        assert_eq!(lighting.light_dir, vec3(0.0, 0.0, 1.0));
        assert_eq!(lighting.ambient_light_int & 0xff00_0000, 0xff00_0000);
        let red = (lighting.ambient_light_int & 255) as f32;
        assert!((red - (0.5 * 255.0 + 32.0)).abs() < 1.0);
    }
}
