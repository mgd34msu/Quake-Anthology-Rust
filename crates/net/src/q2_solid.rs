//! Quake II solid-box encoding.
//!
//! Donor provenance: `q2SolidEncoding`, `packQ2Solid`, and `unpackQ2Solid`
//! in `src/network/q2/solid.ts` (q2proto_solid.c behavior). Protocol
//! identity reuses [`ProtocolIdentity`](crate::protocol::ProtocolIdentity)
//! and bounds reuse [`Bounds`](qa_core::math::Bounds); arithmetic follows
//! the donor's operation order in `f64`, including its single-precision
//! rounding of the packed height bias.

use qa_core::math::{Bounds, vec3};

use crate::protocol::q2;
use crate::protocol::ProtocolIdentity;

/// Solid-box encoding selected by protocol (`Q2SolidEncoding`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2SolidEncoding {
    /// Classic 16-bit short encoding.
    Short,
    /// R1Q2 32-bit encoding.
    R1q2,
    /// Q2Pro v2 32-bit encoding.
    Q2proV2,
}

/// Select the solid encoding for a protocol (`q2SolidEncoding`).
///
/// Returns `None` for non-Q2 identities, which the donor's input type
/// excludes; every Q2 identity maps to an encoding.
#[must_use]
pub fn q2_solid_encoding(protocol: ProtocolIdentity, extended_game: bool) -> Option<Q2SolidEncoding> {
    match protocol {
        ProtocolIdentity::Q2Classic => Some(Q2SolidEncoding::Short),
        ProtocolIdentity::Q2R1q2 { revision } => Some(if revision >= q2::PROTOCOL_VERSION_R1Q2_LONG_SOLID {
            Q2SolidEncoding::R1q2
        } else {
            Q2SolidEncoding::Short
        }),
        ProtocolIdentity::Q2Q2pro { .. } => Some(if extended_game {
            Q2SolidEncoding::Q2proV2
        } else {
            Q2SolidEncoding::R1q2
        }),
        ProtocolIdentity::Q2Rerelease
        | ProtocolIdentity::Q2PrivateClassic
        | ProtocolIdentity::Q2Kex
        | ProtocolIdentity::Q2KexDemo => Some(Q2SolidEncoding::Q2proV2),
        _ => None,
    }
}

/// Truncate and clamp into `[minimum, maximum]` (`clamp`).
///
/// Non-finite inputs follow the donor's bitwise coercion: NaN contributes
/// zero, while infinities saturate at the range ends.
fn clamp(value: f64, minimum: i32, maximum: i32) -> u32 {
    let truncated = value.trunc();
    if truncated.is_nan() {
        return 0;
    }
    truncated.clamp(f64::from(minimum), f64::from(maximum)) as i32 as u32
}

/// Pack bounds into a solid (`packQ2Solid`).
#[must_use]
pub fn pack_q2_solid(bounds: &Bounds, encoding: Q2SolidEncoding) -> u32 {
    let min_z = f64::from(bounds.min.z);
    let max_x = f64::from(bounds.max.x);
    let max_y = f64::from(bounds.max.y);
    let max_z = f64::from(bounds.max.z);
    match encoding {
        Q2SolidEncoding::Short => {
            clamp(max_x / 8.0, 1, 31) | clamp(-min_z / 8.0, 1, 31) << 5 | clamp((max_z + 32.0) / 8.0, 1, 63) << 10
        }
        Q2SolidEncoding::R1q2 => {
            clamp(max_x, 1, 255) | clamp(-min_z, 0, 255) << 8 | clamp(f64::from((max_z + 32768.0) as f32), 0, 65535) << 16
        }
        Q2SolidEncoding::Q2proV2 => {
            clamp(max_x, 1, 255)
                | clamp(max_y, 1, 255) << 8
                | clamp(-min_z, 0, 255) << 16
                | clamp(f64::from((max_z + 32.0) as f32), 0, 255) << 24
        }
    }
}

/// Unpack a solid into bounds (`unpackQ2Solid`).
#[must_use]
pub fn unpack_q2_solid(solid: u32, encoding: Q2SolidEncoding) -> Bounds {
    let x = if encoding == Q2SolidEncoding::Short {
        (solid & 31) * 8
    } else {
        solid & 255
    };
    let y = if encoding == Q2SolidEncoding::Q2proV2 {
        (solid >> 8) & 255
    } else {
        x
    };
    let down = if encoding == Q2SolidEncoding::Short {
        ((solid >> 5) & 31) * 8
    } else if encoding == Q2SolidEncoding::R1q2 {
        (solid >> 8) & 255
    } else {
        (solid >> 16) & 255
    };
    let up = if encoding == Q2SolidEncoding::Short {
        ((solid >> 10) & 63) as i32 * 8 - 32
    } else if encoding == Q2SolidEncoding::R1q2 {
        ((solid >> 16) & 65535) as i32 - 32768
    } else {
        ((solid >> 24) & 255) as i32 - 32
    };
    Bounds {
        min: vec3(-(x as f32), -(y as f32), -(down as f32)),
        max: vec3(x as f32, y as f32, up as f32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(min: [f32; 3], max: [f32; 3]) -> Bounds {
        Bounds {
            min: vec3(min[0], min[1], min[2]),
            max: vec3(max[0], max[1], max[2]),
        }
    }

    #[test]
    fn encoding_follows_protocol() {
        assert_eq!(
            q2_solid_encoding(ProtocolIdentity::Q2Classic, false),
            Some(Q2SolidEncoding::Short)
        );
        assert_eq!(
            q2_solid_encoding(ProtocolIdentity::Q2R1q2 { revision: 1904 }, false),
            Some(Q2SolidEncoding::Short)
        );
        assert_eq!(
            q2_solid_encoding(ProtocolIdentity::Q2R1q2 { revision: 1905 }, false),
            Some(Q2SolidEncoding::R1q2)
        );
        assert_eq!(
            q2_solid_encoding(ProtocolIdentity::Q2Q2pro { revision: 1026 }, false),
            Some(Q2SolidEncoding::R1q2)
        );
        assert_eq!(
            q2_solid_encoding(ProtocolIdentity::Q2Q2pro { revision: 1026 }, true),
            Some(Q2SolidEncoding::Q2proV2)
        );
        for protocol in [
            ProtocolIdentity::Q2Rerelease,
            ProtocolIdentity::Q2PrivateClassic,
            ProtocolIdentity::Q2Kex,
            ProtocolIdentity::Q2KexDemo,
        ] {
            assert_eq!(q2_solid_encoding(protocol, false), Some(Q2SolidEncoding::Q2proV2));
        }
        assert_eq!(q2_solid_encoding(ProtocolIdentity::Q1Netquake, false), None);
        assert_eq!(q2_solid_encoding(ProtocolIdentity::Q3, true), None);
    }

    #[test]
    fn short_vectors_match_donor() {
        let packed = pack_q2_solid(&bounds([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0]), Q2SolidEncoding::Short);
        assert_eq!(packed, 2 | 2 << 5 | 6 << 10);
        let unpacked = unpack_q2_solid(packed, Q2SolidEncoding::Short);
        assert_eq!(unpacked, bounds([-16.0, -16.0, -16.0], [16.0, 16.0, 16.0]));
    }

    #[test]
    fn r1q2_vectors_match_donor() {
        let packed = pack_q2_solid(&bounds([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]), Q2SolidEncoding::R1q2);
        assert_eq!(packed, 1 | 1 << 8 | 32769 << 16);
        let unpacked = unpack_q2_solid(packed, Q2SolidEncoding::R1q2);
        assert_eq!(unpacked, bounds([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]));
    }

    #[test]
    fn q2pro_v2_vectors_match_donor() {
        let packed = pack_q2_solid(&bounds([-2.0, -3.0, -4.0], [5.0, 6.0, 7.0]), Q2SolidEncoding::Q2proV2);
        assert_eq!(packed, 5 | 6 << 8 | 4 << 16 | 39 << 24);
        let unpacked = unpack_q2_solid(packed, Q2SolidEncoding::Q2proV2);
        assert_eq!(unpacked, bounds([-5.0, -6.0, -4.0], [5.0, 6.0, 7.0]));
    }

    #[test]
    fn packing_clamps_and_round_trips() {
        let packed = pack_q2_solid(&bounds([-500.0, -500.0, -500.0], [9000.0, 9000.0, 9000.0]), Q2SolidEncoding::Short);
        assert_eq!(packed, 31 | 31 << 5 | 63 << 10);
        let packed = pack_q2_solid(&bounds([0.0, 0.0, 0.0], [0.0, 0.0, -40.0]), Q2SolidEncoding::Q2proV2);
        assert_eq!(packed, 1 | 1 << 8 | 0 << 16 | 0 << 24);
    }
}
