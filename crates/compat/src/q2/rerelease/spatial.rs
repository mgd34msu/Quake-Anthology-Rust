//! Q2 rerelease link bounds and network solid packing.
//!
//! Donor: `src/compat/q2/rerelease/spatial.ts` — bridges `q2repro`
//! `SV_LinkEdict` envelopes and `kex_pack_solid` into host helpers.

use qa_core::math::{Bounds, Vec3};

/// Body snapshot needed for link computations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkBody {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Local bounds.
    pub bounds: Bounds,
}

/// `q2repro` `server/world.c` `SV_LinkEdict`: rotated brush envelope plus
/// one-unit padding.
#[must_use]
pub fn rerelease_link_bounds(state: &LinkBody, solid: i32) -> Bounds {
    let (mut min, mut max) = (state.bounds.min, state.bounds.max);
    if solid == 3 && (state.angles.x != 0.0 || state.angles.y != 0.0 || state.angles.z != 0.0)
    {
        let extent = min
            .x
            .abs()
            .max(min.y.abs())
            .max(min.z.abs())
            .max(max.x.abs())
            .max(max.y.abs())
            .max(max.z.abs());
        min = Vec3 {
            x: -extent,
            y: -extent,
            z: -extent,
        };
        max = Vec3 {
            x: extent,
            y: extent,
            z: extent,
        };
    }
    Bounds {
        min: Vec3 {
            x: (state.origin.x + min.x) - 1.0,
            y: (state.origin.y + min.y) - 1.0,
            z: (state.origin.z + min.z) - 1.0,
        },
        max: Vec3 {
            x: (state.origin.x + max.x) + 1.0,
            y: (state.origin.y + max.y) + 1.0,
            z: (state.origin.z + max.z) + 1.0,
        },
    }
}

/// `q2proto_proto_kex.c` `kex_pack_solid` with the shared solid BSP
/// sentinel 31.
#[must_use]
pub fn rerelease_network_solid(bounds: &Bounds, solid: i32, server_flags: i32) -> u32 {
    if solid == 3 {
        return 31;
    }
    if solid != 2
        || (server_flags & 2) != 0
        || (bounds.min.x == bounds.max.x
            && bounds.min.y == bounds.max.y
            && bounds.min.z == bounds.max.z)
    {
        return 0;
    }
    let clamp = |value: f32, minimum: i32| -> u32 {
        (value.trunc() as i32).clamp(minimum, 255) as u32
    };
    let packed = (clamp(bounds.max.z + 32.0, 0) << 24)
        | (clamp(-bounds.min.z, 0) << 16)
        | (clamp(bounds.max.y, 1) << 8)
        | clamp(bounds.max.x, 1);
    if packed == 31 { 0 } else { packed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> LinkBody {
        LinkBody {
            origin: Vec3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
            angles: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            bounds: Bounds {
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
            },
        }
    }

    #[test]
    fn link_bounds_pad_and_rotate_brushes() {
        let plain = rerelease_link_bounds(&body(), 2);
        assert_eq!(plain.min.x, 100.0 - 16.0 - 1.0);
        assert_eq!(plain.max.z, 32.0 + 1.0);
        let rotated = rerelease_link_bounds(
            &LinkBody {
                angles: Vec3 {
                    x: 0.0,
                    y: 45.0,
                    z: 0.0,
                },
                ..body()
            },
            3,
        );
        assert_eq!(rotated.min.x, 100.0 - 32.0 - 1.0);
        assert_eq!(rotated.max.x, 100.0 + 32.0 + 1.0);
        assert_eq!(rotated.min.z, -32.0 - 1.0);
        let unrotated_brush = rerelease_link_bounds(&body(), 3);
        assert_eq!(unrotated_brush.min.x, plain.min.x);
    }

    #[test]
    fn network_solid_packs_and_sentinels() {
        let bounds = Bounds {
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
        assert_eq!(rerelease_network_solid(&bounds, 3, 0), 31);
        assert_eq!(rerelease_network_solid(&bounds, 0, 0), 0);
        assert_eq!(rerelease_network_solid(&bounds, 2, 2), 0);
        let packed = rerelease_network_solid(&bounds, 2, 0);
        assert_eq!(packed, (64 << 24) | (24 << 16) | (16 << 8) | 16);
        let point = Bounds {
            min: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            max: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
        };
        assert_eq!(rerelease_network_solid(&point, 2, 0), 0);
    }
}
