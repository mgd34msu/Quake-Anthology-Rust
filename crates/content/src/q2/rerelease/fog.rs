//! Q2 rerelease fog (`src/content/q2/rerelease/fog.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::math::Vec3;

use crate::q2::foundation::fields::{number_field, vector_field};
use crate::q2::foundation::host::Q2SpawnFields;

use super::types::{Q2Fog, Q2FogState, Q2HeightFog};

/// Read rerelease fog fields (`q2RereleaseFogFields`).
pub fn q2_rerelease_fog_fields(fields: &Q2SpawnFields, off: bool) -> Q2FogState {
    let suffix = if off { "_off" } else { "" };
    let number = |key: &str| number_field(fields, &format!("{key}{suffix}"), 0.0);
    let vector = |key: &str| vector_field(fields, &format!("{key}{suffix}"));
    Q2FogState {
        fog: Q2Fog {
            density: number("fog_density"),
            color: vector("fog_color"),
            sky_factor: number("fog_sky_factor"),
        },
        height_fog: Q2HeightFog {
            start_color: vector("heightfog_start_color"),
            start_distance: number("heightfog_start_dist"),
            end_color: vector("heightfog_end_color"),
            end_distance: number("heightfog_end_dist"),
            falloff: number("heightfog_falloff"),
            density: number("heightfog_density"),
        },
    }
}

/// Interpolate rerelease fog (`interpolateQ2Fog`).
pub fn interpolate_q2_fog(off: &Q2FogState, on: &Q2FogState, fraction: f64) -> Q2FogState {
    let scalar = |first: f64, last: f64| first + (last - first) * fraction;
    let vector = |first: Vec3, last: Vec3| Vec3 {
        x: scalar(f64::from(first.x), f64::from(last.x)) as f32,
        y: scalar(f64::from(first.y), f64::from(last.y)) as f32,
        z: scalar(f64::from(first.z), f64::from(last.z)) as f32,
    };
    Q2FogState {
        fog: Q2Fog {
            density: scalar(off.fog.density, on.fog.density),
            color: vector(off.fog.color, on.fog.color),
            sky_factor: scalar(off.fog.sky_factor, on.fog.sky_factor),
        },
        height_fog: Q2HeightFog {
            start_color: vector(off.height_fog.start_color, on.height_fog.start_color),
            start_distance: scalar(off.height_fog.start_distance, on.height_fog.start_distance),
            end_color: vector(off.height_fog.end_color, on.height_fog.end_color),
            end_distance: scalar(off.height_fog.end_distance, on.height_fog.end_distance),
            falloff: scalar(off.height_fog.falloff, on.height_fog.falloff),
            density: scalar(off.height_fog.density, on.height_fog.density),
        },
    }
}

/// Compare rerelease fog (`equalQ2Fog`).
pub fn equal_q2_fog(first: &Q2FogState, second: &Q2FogState) -> bool {
    first == second
}
