//! Shared model lighting values, resolved before either drawing backend.
use crate::{
    scene::{Light, SceneEntity},
    stage::EntityLighting,
};
use qa_core::{math, primitives::Vec3};

struct GridPoint {
    ambient: [u8; 3],
    directed: [u8; 3],
    direction: Vec3,
}
pub struct LightGrid {
    origin: Vec3,
    inverse_size: Vec3,
    bounds: [usize; 3],
    samples: Box<[GridPoint]>,
}
impl LightGrid {
    pub fn load(
        bounds: qa_core::primitives::Bounds,
        size: Vec3,
        samples: impl ExactSizeIterator<Item = qa_formats::bsp::LightGridPoint>,
        shift: i8,
    ) -> Result<Self, &'static str> {
        if !bounds.is_valid()
            || !size.is_finite()
            || size.0.iter().any(|&value| value <= 0.)
            || !(-8..=8).contains(&shift)
        {
            return Err("invalid light-grid dimensions or shift");
        }
        let origin = Vec3(std::array::from_fn(|axis| {
            size.0[axis] * (bounds.mins.0[axis] / size.0[axis]).ceil()
        }));
        let end = Vec3(std::array::from_fn(|axis| {
            size.0[axis] * (bounds.maxs.0[axis] / size.0[axis]).floor()
        }));
        let counts = std::array::from_fn::<_, 3, _>(|axis| {
            ((end.0[axis] - origin.0[axis]) / size.0[axis]) + 1.
        });
        if counts
            .iter()
            .any(|&value| !value.is_finite() || value < 1. || value > usize::MAX as f32)
        {
            return Err("invalid light-grid bounds");
        }
        let bounds = counts.map(|value| value as usize);
        let count = bounds
            .into_iter()
            .try_fold(1usize, usize::checked_mul)
            .ok_or("light-grid layout overflow")?;
        if count != samples.len() {
            return Err("light-grid sample count does not match world bounds");
        }
        let table = math::sine_table();
        let samples = samples
            .map(|point| {
                let latitude = usize::from(point.latitude) * 4;
                let longitude = usize::from(point.longitude) * 4;
                GridPoint {
                    ambient: crate::lightmap::color_shift_rgb(point.ambient, shift),
                    directed: crate::lightmap::color_shift_rgb(point.directed, shift),
                    direction: Vec3([
                        table[(latitude + 256) & 1023] * table[longitude],
                        table[latitude] * table[longitude],
                        table[(longitude + 256) & 1023],
                    ]),
                }
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            origin,
            inverse_size: Vec3(size.0.map(|value| 1. / value)),
            bounds,
            samples,
        })
    }
    pub fn sample(&self, point: Vec3, ambient_scale: f32, directed_scale: f32) -> EntityLighting {
        let relative = point - self.origin;
        let coordinates =
            std::array::from_fn::<_, 3, _>(|axis| relative.0[axis] * self.inverse_size.0[axis]);
        let positions = coordinates.map(|value| value.floor() as i32);
        let fractions =
            std::array::from_fn::<_, 3, _>(|axis| coordinates[axis] - positions[axis] as f32);
        let positions = std::array::from_fn::<_, 3, _>(|axis| {
            (positions[axis].max(0) as usize).min(self.bounds[axis] - 1)
        });
        let mut ambient = Vec3::default();
        let mut directed = Vec3::default();
        let mut direction = Vec3::default();
        let mut total = 0.;
        for corner in 0..8 {
            let mut factor = 1.;
            let coordinates = std::array::from_fn::<_, 3, _>(|axis| {
                let upper = (corner & (1 << axis)) != 0;
                factor *= if upper {
                    fractions[axis]
                } else {
                    1. - fractions[axis]
                };
                positions[axis] + usize::from(upper)
            });
            let index = coordinates[0]
                + self.bounds[0] * (coordinates[1] + self.bounds[1] * coordinates[2]);
            // Preserve source's in-allocation row aliases. Only an undefined
            // read past the allocation uses TypeScript's replicated-edge rule.
            let sample = self.samples.get(index).unwrap_or_else(|| {
                let clamped = std::array::from_fn::<_, 3, _>(|axis| {
                    coordinates[axis].min(self.bounds[axis] - 1)
                });
                &self.samples
                    [clamped[0] + self.bounds[0] * (clamped[1] + self.bounds[1] * clamped[2])]
            });
            if sample.ambient == [0; 3] {
                continue;
            }
            total += factor;
            ambient = ambient + Vec3(sample.ambient.map(f32::from)) * factor;
            directed = directed + Vec3(sample.directed.map(f32::from)) * factor;
            direction = direction + sample.direction * factor;
        }
        if total > 0. && total < 0.99 {
            let inverse = 1. / total;
            ambient = ambient * inverse;
            directed = directed * inverse;
        }
        EntityLighting {
            ambient: (ambient * ambient_scale).0,
            directed: (directed * directed_scale).0,
            direction: math::normalized_or_zero(direction),
        }
    }
    pub fn load_map(
        map: &qa_formats::bsp::Map<'_>,
        shift: i8,
    ) -> Result<Option<Self>, &'static str> {
        if map.light_grid().len() == 0 {
            return Ok(None);
        }
        let mut size = Vec3([64., 64., 128.]);
        let mut tokens = qa_formats::entities::EntityTokens::new(
            map.entity_text(),
            qa_core::primitives::Lineage::Quake3,
        );
        if tokens
            .next_token()
            .map_err(|_| "invalid worldspawn grid metadata")?
            == Some(b"{")
        {
            loop {
                let Some(key) = tokens
                    .next_token()
                    .map_err(|_| "invalid worldspawn grid metadata")?
                else {
                    break;
                };
                if key == b"}" {
                    break;
                }
                let value = tokens
                    .next_token()
                    .map_err(|_| "invalid worldspawn grid metadata")?
                    .ok_or("incomplete worldspawn grid metadata")?;
                if qa_core::names::compare_folded(key, b"gridsize").is_eq() {
                    let mut remaining = value;
                    for axis in 0..3 {
                        let (value, consumed) = qa_core::text::atof_prefix_bytes(remaining);
                        if consumed == 0 {
                            break;
                        }
                        size.0[axis] = value as f32;
                        remaining = &remaining[consumed..];
                    }
                }
            }
        }
        let bounds = map
            .models
            .first()
            .ok_or("light-grid world model missing")?
            .bounds;
        Self::load(bounds, size, map.light_grid(), shift).map(Some)
    }
}

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
