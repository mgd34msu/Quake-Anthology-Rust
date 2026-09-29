//! Wire angle/coordinate scalar encodings ported from
//! `src/network/q1/message.ts`, `src/network/q2/message.ts` (+`ANGLE2SHORT`
//! in `src/network/q2/state.ts`), and `angleInteger` in
//! `src/network/q3/message.ts`.

use thiserror::Error;

use qa_core::numeric::float_to_wrapped_i32;

use crate::protocol::q1;

/// Error for wire scalar conversions outside their defined range.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AngleError {
    /// Q3 message angles need a defined float-to-int conversion.
    #[error("Undefined native message angle float-to-int conversion")]
    UndefinedConversion,
}

/// Round half away from zero (`Q_rint`).
#[must_use]
pub fn q_rint(value: f64) -> f64 {
    if value > 0.0 {
        (value + 0.5).trunc()
    } else {
        (value - 0.5).trunc()
    }
}

/// NetQuake `MSG_WriteAngle` value: truncate the degrees, scale to a byte.
#[must_use]
pub fn q1_byte_angle(value: f64) -> u8 {
    float_to_wrapped_i32((value.trunc() * 256.0 / 360.0).trunc()) as u8
}

/// NetQuake `MSG_ReadAngle`: signed byte scaled back to degrees.
#[must_use]
pub fn q1_byte_angle_to_degrees(byte: u8) -> f64 {
    f64::from(byte as i8) * 360.0 / 256.0
}

/// NetQuake `MSG_WriteCoord` value: eighths of a unit, truncated.
#[must_use]
pub fn q1_coord_to_short(value: f64) -> i16 {
    float_to_wrapped_i32((value * 8.0).trunc()) as i16
}

/// NetQuake `MSG_ReadCoord`.
#[must_use]
pub fn q1_short_to_coord(short: i16) -> f64 {
    f64::from(short) / 8.0
}

/// FitzQuake/RMQ coordinate encoding under `PRFL_` flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q1CoordEncoding {
    /// Eighth-unit short (`Q_rint(n * 8)`).
    Short(i16),
    /// Short plus fractional byte (`trunc(n * 255) % 255`).
    ShortByte {
        /// Whole part.
        whole: i16,
        /// Fractional byte.
        fraction: u8,
    },
    /// 32-bit integer sixteenths (`Q_rint(n * 16)`).
    Long(i32),
    /// Raw float.
    Float(f32),
}

/// Encode a coordinate under FitzQuake/RMQ flags.
#[must_use]
pub fn q1_encode_coord_flags(value: f64, flags: u32) -> Q1CoordEncoding {
    if (flags & q1::PRFL_FLOATCOORD) != 0 {
        Q1CoordEncoding::Float(value as f32)
    } else if (flags & q1::PRFL_INT32COORD) != 0 {
        Q1CoordEncoding::Long(float_to_wrapped_i32(q_rint(value * 16.0)))
    } else if (flags & q1::PRFL_24BITCOORD) != 0 {
        Q1CoordEncoding::ShortByte {
            whole: float_to_wrapped_i32(value.trunc()) as i16,
            fraction: (float_to_wrapped_i32((value * 255.0).trunc()) % 255) as u8,
        }
    } else {
        Q1CoordEncoding::Short(float_to_wrapped_i32(q_rint(value * 8.0)) as i16)
    }
}

/// FitzQuake/RMQ angle encoding under `PRFL_` flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q1AngleEncoding {
    /// Byte angle (`Q_rint(n * 256 / 360)`).
    Byte(u8),
    /// Short angle (`Q_rint(n * 65536 / 360)`).
    Short(i16),
    /// Raw float.
    Float(f32),
}

/// Encode an angle under FitzQuake/RMQ flags.
#[must_use]
pub fn q1_encode_angle_flags(value: f64, flags: u32) -> Q1AngleEncoding {
    if (flags & q1::PRFL_FLOATANGLE) != 0 {
        Q1AngleEncoding::Float(value as f32)
    } else if (flags & q1::PRFL_SHORTANGLE) != 0 {
        Q1AngleEncoding::Short(float_to_wrapped_i32(q_rint(value * 65_536.0 / 360.0)) as i16)
    } else {
        Q1AngleEncoding::Byte(float_to_wrapped_i32(q_rint(value * 256.0 / 360.0)) as u8)
    }
}

/// FitzQuake/RMQ movement angle: short (or float under `PRFL_FLOATANGLE`).
#[must_use]
pub fn q1_encode_move_angle16(value: f64, flags: u32) -> Q1AngleEncoding {
    if (flags & q1::PRFL_FLOATANGLE) != 0 {
        Q1AngleEncoding::Float(value as f32)
    } else {
        Q1AngleEncoding::Short(float_to_wrapped_i32(q_rint(value * 65_536.0 / 360.0)) as i16)
    }
}

/// Decode a short angle under FitzQuake/RMQ flags.
#[must_use]
pub fn q1_decode_short_angle(short: i16) -> f64 {
    f64::from(short) * 360.0 / 65_536.0
}

/// Quake II `ANGLE2SHORT`: truncate to a 16-bit angle word.
#[must_use]
pub fn q2_angle_to_short(value: f64) -> u16 {
    float_to_wrapped_i32(((value * 65_536.0) / 360.0).trunc()) as u16
}

/// Quake II `SHORT2ANGLE`.
#[must_use]
pub fn q2_short_to_angle(word: i16) -> f64 {
    f64::from(word) * (360.0 / 65_536.0)
}

/// Quake II `MSG_WriteAngle` value: truncated byte.
#[must_use]
pub fn q2_byte_angle(value: f64) -> u8 {
    float_to_wrapped_i32(((value * 256.0) / 360.0).trunc()) as u8
}

/// Quake II `MSG_ReadAngle`: signed byte scaled back to degrees.
#[must_use]
pub fn q2_byte_angle_to_degrees(byte: u8) -> f64 {
    f64::from(byte as i8) * (360.0 / 256.0)
}

/// Quake II coordinate: truncated eighth-unit short.
#[must_use]
pub fn q2_coord_to_short(value: f64) -> i16 {
    float_to_wrapped_i32((value * 8.0).trunc()) as i16
}

/// Quake III message angle integer: binary32 scale-then-divide, truncated.
pub fn q3_angle_integer(value: f64, scale: f64) -> Result<i32, AngleError> {
    let product = value as f32 * scale as f32;
    let scaled = f64::from(product) / 360.0;
    let scaled = f64::from(scaled as f32);
    if !scaled.is_finite() || scaled < -2_147_483_648.0 || scaled >= 2_147_483_648.0 {
        return Err(AngleError::UndefinedConversion);
    }
    Ok(scaled.trunc() as i32)
}

/// Quake III `writeAngle` byte.
pub fn q3_angle_to_byte(value: f64) -> Result<u8, AngleError> {
    Ok(q3_angle_integer(value, 256.0)? as u8)
}

/// Quake III `writeAngle16` word.
pub fn q3_angle_to_short(value: f64) -> Result<u16, AngleError> {
    Ok(q3_angle_integer(value, 65_536.0)? as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q1_scalars_match_donor() {
        assert_eq!(q1_byte_angle(90.0), 64);
        assert_eq!(q1_byte_angle_to_degrees(64), 90.0);
        assert_eq!(q1_coord_to_short(12.5), 100);
        assert_eq!(q1_short_to_coord(100), 12.5);
        // Truncation, not rounding: 359.9 degrees truncates to 359 first.
        assert_eq!(q1_byte_angle(359.9), (359.0f64 * 256.0 / 360.0).trunc() as u8);
        assert_eq!(
            q1_encode_angle_flags(90.0, q1::PRFL_SHORTANGLE),
            Q1AngleEncoding::Short(q_rint(90.0 * 65_536.0 / 360.0) as i32 as i16)
        );
        assert_eq!(q1_decode_short_angle(16_384), 90.0);
    }

    #[test]
    fn q2_scalars_match_donor() {
        assert_eq!(q2_angle_to_short(90.0), 16_384);
        assert_eq!(q2_short_to_angle(16_384), 90.0);
        assert_eq!(q2_byte_angle(90.0), 64);
        assert_eq!(q2_byte_angle_to_degrees(64), 90.0);
        assert_eq!(q2_coord_to_short(-1.5), -12);
    }

    #[test]
    fn q3_angles_match_donor() {
        assert_eq!(q3_angle_to_byte(90.0).unwrap(), 64);
        assert_eq!(q3_angle_to_short(90.0).unwrap(), 16_384);
        assert_eq!(q3_angle_to_byte(0.0).unwrap(), 0);
        assert!(q3_angle_integer(f64::INFINITY, 256.0).is_err());
    }
}
