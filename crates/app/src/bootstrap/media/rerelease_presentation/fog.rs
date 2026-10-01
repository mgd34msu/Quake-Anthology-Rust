//! Q2 rerelease fog transitions (`P_ForceFogTransition`, q2repro `V_FogParamsChanged`).
//!
//! Port of donor `src/app/bootstrap/rerelease-presentation/fog.ts`
//! (GPL-2.0-or-later).
//!
//! Local events carry game floats; the wire message carries bytes and
//! unscaled signed height coordinates.

use qa_client::render::types::{Q2Fog, Q2HeightFog, Q2HeightStop};
use qa_content::q2::rerelease::types::Q2FogState;
use qa_core::math::{vec3, Vec3};
use qa_core::numeric::float_to_wrapped_i32;
use qa_net::q2_variants::{fog_bits, FogData};

/// Quantize a float to a byte-valued color fraction.
fn fraction(value: f64) -> f64 {
    let truncated = f64::from((value * 255.0) as f32);
    f64::from((float_to_wrapped_i32(truncated) & 255) as f32) / 255.0
}

/// Quantize a color to byte-valued fractions.
fn color(value: Vec3) -> Vec3 {
    vec3(
        fraction(f64::from(value.x)) as f32,
        fraction(f64::from(value.y)) as f32,
        fraction(f64::from(value.z)) as f32,
    )
}

/// Round a game float to wire (`f32`) precision.
fn wire_float(value: f64) -> f64 {
    f64::from(value as f32)
}

/// Convert a game-float fog state to its wire representation.
#[must_use]
pub fn q2_fog_from_source(value: &Q2FogState) -> Q2FogState {
    Q2FogState {
        fog: qa_content::q2::rerelease::types::Q2Fog {
            density: wire_float(value.fog.density),
            color: color(value.fog.color),
            sky_factor: fraction(value.fog.sky_factor),
        },
        height_fog: qa_content::q2::rerelease::types::Q2HeightFog {
            start_color: color(value.height_fog.start_color),
            start_distance: f64::from(float_to_wrapped_i32(value.height_fog.start_distance)),
            end_color: color(value.height_fog.end_color),
            end_distance: f64::from(float_to_wrapped_i32(value.height_fog.end_distance)),
            falloff: wire_float(value.height_fog.falloff),
            density: wire_float(value.height_fog.density),
        },
    }
}

/// Rerelease fog interpolation (`RereleaseFog`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RereleaseFog {
    start: Q2FogState,
    target: Q2FogState,
    starts: f64,
    duration: f64,
}

impl RereleaseFog {
    /// Empty fog with no transition.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Receive a fog state with a blend duration in milliseconds.
    pub fn receive(&mut self, value: &Q2FogState, duration_milliseconds: f64, seconds: f64) {
        if duration_milliseconds != 0.0 {
            self.start = self.target;
            self.starts = seconds * 1000.0;
        }
        self.duration = duration_milliseconds;
        self.target = q2_fog_from_source(value);
    }

    /// Interpolate the fog for the current frame.
    #[must_use]
    pub fn current(&self, seconds: f64) -> Q2Fog {
        let elapsed = seconds * 1000.0 - self.starts;
        let front = if self.duration == 0.0 || elapsed > self.duration {
            1.0
        } else {
            elapsed / self.duration
        };
        let back = 1.0 - front;
        let mix = |a: f64, b: f64| (a * back + b * front) as f32;
        let mix_color = |a: Vec3, b: Vec3| {
            vec3(
                (f64::from(a.x) * back + f64::from(b.x) * front) as f32,
                (f64::from(a.y) * back + f64::from(b.y) * front) as f32,
                (f64::from(a.z) * back + f64::from(b.z) * front) as f32,
            )
        };
        let (a, b) = (&self.start, &self.target);
        Q2Fog {
            color: mix_color(a.fog.color, b.fog.color),
            density: mix(a.fog.density, b.fog.density),
            sky_factor: mix(a.fog.sky_factor, b.fog.sky_factor),
            height: Q2HeightFog {
                start: Q2HeightStop {
                    color: mix_color(a.height_fog.start_color, b.height_fog.start_color),
                    distance: mix(a.height_fog.start_distance, b.height_fog.start_distance),
                },
                end: Q2HeightStop {
                    color: mix_color(a.height_fog.end_color, b.height_fog.end_color),
                    distance: mix(a.height_fog.end_distance, b.height_fog.end_distance),
                },
                density: mix(a.height_fog.density, b.height_fog.density),
                falloff: mix(a.height_fog.falloff, b.height_fog.falloff),
            },
        }
    }
}

/// Apply a `svc_fog` update to the prior target: only flagged components
/// change, including byte-valued colors.
#[must_use]
pub fn q2_fog_from_wire(previous: &Q2FogState, value: &FogData) -> Q2FogState {
    let has = |bit: u16| value.bits & bit != 0;
    let byte = |byte: u8| f32::from(byte) / 255.0;
    Q2FogState {
        fog: qa_content::q2::rerelease::types::Q2Fog {
            density: if has(fog_bits::DENSITY) {
                f64::from(value.density)
            } else {
                previous.fog.density
            },
            sky_factor: if has(fog_bits::DENSITY) {
                f64::from(value.skyfactor) / 255.0
            } else {
                previous.fog.sky_factor
            },
            color: vec3(
                if has(fog_bits::R) {
                    byte(value.red)
                } else {
                    previous.fog.color.x
                },
                if has(fog_bits::G) {
                    byte(value.green)
                } else {
                    previous.fog.color.y
                },
                if has(fog_bits::B) {
                    byte(value.blue)
                } else {
                    previous.fog.color.z
                },
            ),
        },
        height_fog: qa_content::q2::rerelease::types::Q2HeightFog {
            falloff: if has(fog_bits::HEIGHTFOG_FALLOFF) {
                f64::from(value.hf_falloff)
            } else {
                previous.height_fog.falloff
            },
            density: if has(fog_bits::HEIGHTFOG_DENSITY) {
                f64::from(value.hf_density)
            } else {
                previous.height_fog.density
            },
            start_distance: if has(fog_bits::HEIGHTFOG_START_DIST) {
                f64::from(value.hf_start_dist)
            } else {
                previous.height_fog.start_distance
            },
            end_distance: if has(fog_bits::HEIGHTFOG_END_DIST) {
                f64::from(value.hf_end_dist)
            } else {
                previous.height_fog.end_distance
            },
            start_color: vec3(
                if has(fog_bits::HEIGHTFOG_START_R) {
                    byte(value.hf_start[0])
                } else {
                    previous.height_fog.start_color.x
                },
                if has(fog_bits::HEIGHTFOG_START_G) {
                    byte(value.hf_start[1])
                } else {
                    previous.height_fog.start_color.y
                },
                if has(fog_bits::HEIGHTFOG_START_B) {
                    byte(value.hf_start[2])
                } else {
                    previous.height_fog.start_color.z
                },
            ),
            end_color: vec3(
                if has(fog_bits::HEIGHTFOG_END_R) {
                    byte(value.hf_end[0])
                } else {
                    previous.height_fog.end_color.x
                },
                if has(fog_bits::HEIGHTFOG_END_G) {
                    byte(value.hf_end[1])
                } else {
                    previous.height_fog.end_color.y
                },
                if has(fog_bits::HEIGHTFOG_END_B) {
                    byte(value.hf_end[2])
                } else {
                    previous.height_fog.end_color.z
                },
            ),
        },
    }
}

/// Empty fog state for tests.
#[cfg(test)]
pub(crate) fn test_fog_data() -> FogData {
    FogData {
        bits: 0,
        density: 0.0,
        skyfactor: 0,
        red: 0,
        green: 0,
        blue: 0,
        time: 0,
        hf_falloff: 0.0,
        hf_density: 0.0,
        hf_start: [0; 3],
        hf_start_dist: 0,
        hf_end: [0; 3],
        hf_end_dist: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::q2::rerelease::types::create_q2_fog;

    fn state() -> Q2FogState {
        let mut state = create_q2_fog();
        state.fog.density = 0.5;
        state.fog.color = vec3(1.0, 0.5, 0.25);
        state.fog.sky_factor = 0.75;
        state.height_fog.start_color = vec3(0.1, 0.2, 0.3);
        state.height_fog.start_distance = 100.9;
        state.height_fog.end_color = vec3(0.4, 0.5, 0.6);
        state.height_fog.end_distance = 900.9;
        state.height_fog.falloff = 1.5;
        state.height_fog.density = 0.125;
        state
    }

    #[test]
    fn source_quantizes_colors_and_distances() {
        let converted = q2_fog_from_source(&state());
        assert_eq!(converted.fog.color, vec3(1.0, 127.0 / 255.0, 63.0 / 255.0));
        assert!((converted.fog.sky_factor - 191.0 / 255.0).abs() < 1e-6);
        assert_eq!(converted.height_fog.start_distance, 100.0);
        assert_eq!(converted.height_fog.end_distance, 900.0);
        assert_eq!(converted.height_fog.falloff, 1.5);
    }

    #[test]
    fn instant_receive_applies_immediately() {
        let mut fog = RereleaseFog::new();
        fog.receive(&state(), 0.0, 10.0);
        let current = fog.current(10.0);
        assert_eq!(current.color, vec3(1.0, 127.0 / 255.0, 63.0 / 255.0));
        assert!((current.density - 0.5).abs() < 1e-6);
        assert_eq!(current.height.start.distance, 100.0);
        assert_eq!(current.height.end.distance, 900.0);
    }

    #[test]
    fn timed_receive_blends_from_prior_target() {
        let mut fog = RereleaseFog::new();
        fog.receive(&state(), 0.0, 0.0);
        let mut next = create_q2_fog();
        next.fog.density = 1.0;
        next.fog.color = vec3(0.0, 0.0, 0.0);
        fog.receive(&next, 1000.0, 10.0);
        let mid = fog.current(10.5);
        assert!((mid.density - 0.75).abs() < 1e-6);
        assert!((mid.color.x - 0.5).abs() < 1e-6);
        assert!((mid.color.y - 127.0 / 510.0).abs() < 1e-6);
        assert!((mid.color.z - 63.0 / 510.0).abs() < 1e-6);
        let end = fog.current(11.5);
        assert!((end.density - 1.0).abs() < 1e-6);
        assert_eq!(end.color, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn wire_updates_only_flagged_components() {
        let previous = q2_fog_from_source(&state());
        let mut wire = test_fog_data();
        wire.bits = fog_bits::DENSITY | fog_bits::R | fog_bits::HEIGHTFOG_START_DIST;
        wire.density = 0.25;
        wire.skyfactor = 255;
        wire.red = 0;
        wire.hf_start_dist = 50;
        let merged = q2_fog_from_wire(&previous, &wire);
        assert!((merged.fog.density - 0.25).abs() < 1e-6);
        assert_eq!(merged.fog.sky_factor, 1.0);
        assert_eq!(merged.fog.color.x, 0.0);
        assert_eq!(merged.fog.color.y, previous.fog.color.y);
        assert_eq!(merged.height_fog.start_distance, 50.0);
        assert_eq!(merged.height_fog.end_distance, previous.height_fog.end_distance);
    }

    #[test]
    fn wire_height_colors_map_per_channel() {
        let previous = create_q2_fog();
        let mut wire = test_fog_data();
        wire.bits = fog_bits::HEIGHTFOG_START_G | fog_bits::HEIGHTFOG_END_B;
        wire.hf_start = [10, 20, 30];
        wire.hf_end = [40, 50, 60];
        let merged = q2_fog_from_wire(&previous, &wire);
        assert_eq!(merged.height_fog.start_color, vec3(0.0, 20.0 / 255.0, 0.0));
        assert_eq!(merged.height_fog.end_color, vec3(0.0, 0.0, 60.0 / 255.0));
    }
}
