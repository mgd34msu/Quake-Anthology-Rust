//! QVM `trace_t` record codec.
//!
//! Provenance: `src/compat/qvm/trace-record.ts` (`trace_t` and `cplane_t`
//! from id Software's `code/game/q_shared.h`).
//!
//! [`QvmTraceRecord`] mirrors the donor's `QvmTraceRecord`
//! (`SourceTraceResult` plus `entityNum`); the source trace work starts with
//! zeroed padding, which [`write_qvm_trace`] reproduces exactly.

use qa_core::math::{vec3, Vec3};

use crate::error::GuestError;

/// `trace_t` size in bytes.
pub const QVM_TRACE_BYTES: usize = 56;

/// Collision plane carried by a trace record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmTracePlane {
    /// Plane normal.
    pub normal: Vec3,
    /// Plane distance.
    pub distance: f32,
    /// Plane type.
    pub plane_type: u8,
    /// Plane sign bits.
    pub signbits: u8,
}

/// Owned `trace_t` record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmTraceRecord {
    /// Everything solid.
    pub all_solid: bool,
    /// Started solid.
    pub start_solid: bool,
    /// Completed fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Impact plane (carried even when invalid).
    pub plane: QvmTracePlane,
    /// Surface flags.
    pub surface_flags: i32,
    /// Contents mask.
    pub contents: i32,
    /// Hit entity number.
    pub entity_num: i32,
}

impl Default for QvmTraceRecord {
    fn default() -> Self {
        Self {
            all_solid: false,
            start_solid: false,
            fraction: 1.0,
            end: vec3(0.0, 0.0, 0.0),
            plane: QvmTracePlane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
                plane_type: 2,
                signbits: 0,
            },
            surface_flags: 0,
            contents: 0,
            entity_num: 1023,
        }
    }
}

fn check_record(bytes: &[u8]) -> Result<(), GuestError> {
    if bytes.len() < QVM_TRACE_BYTES {
        return Err(GuestError::invalid(format!(
            "QVM trace_t record requires {QVM_TRACE_BYTES} bytes, received {}",
            bytes.len()
        )));
    }
    Ok(())
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    i32::from_le_bytes(word)
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    f32::from_le_bytes(word)
}

fn write_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Write a resolved trace into its record, zeroing padding first.
pub fn write_qvm_trace(bytes: &mut [u8], value: &QvmTraceRecord) -> Result<(), GuestError> {
    check_record(bytes)?;
    write_i32(bytes, 0, i32::from(value.all_solid));
    write_i32(bytes, 4, i32::from(value.start_solid));
    write_f32(bytes, 8, value.fraction);
    write_f32(bytes, 12, value.end.x);
    write_f32(bytes, 16, value.end.y);
    write_f32(bytes, 20, value.end.z);
    write_f32(bytes, 24, value.plane.normal.x);
    write_f32(bytes, 28, value.plane.normal.y);
    write_f32(bytes, 32, value.plane.normal.z);
    write_f32(bytes, 36, value.plane.distance);
    bytes[40] = value.plane.plane_type;
    bytes[41] = value.plane.signbits;
    bytes[42] = 0;
    bytes[43] = 0;
    write_i32(bytes, 44, value.surface_flags);
    write_i32(bytes, 48, value.contents);
    write_i32(bytes, 52, value.entity_num);
    Ok(())
}

/// Read a trace record back.
pub fn read_qvm_trace(bytes: &[u8]) -> Result<QvmTraceRecord, GuestError> {
    check_record(bytes)?;
    Ok(QvmTraceRecord {
        all_solid: read_i32(bytes, 0) != 0,
        start_solid: read_i32(bytes, 4) != 0,
        fraction: read_f32(bytes, 8),
        end: vec3(
            read_f32(bytes, 12),
            read_f32(bytes, 16),
            read_f32(bytes, 20),
        ),
        plane: QvmTracePlane {
            normal: vec3(
                read_f32(bytes, 24),
                read_f32(bytes, 28),
                read_f32(bytes, 32),
            ),
            distance: read_f32(bytes, 36),
            plane_type: bytes[40],
            signbits: bytes[41],
        },
        surface_flags: read_i32(bytes, 44),
        contents: read_i32(bytes, 48),
        entity_num: read_i32(bytes, 52),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_matches_source_byte_layout() {
        let value = QvmTraceRecord {
            all_solid: true,
            start_solid: false,
            fraction: 0.5,
            end: vec3(1.0, 2.0, 3.0),
            plane: QvmTracePlane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 64.0,
                plane_type: 2,
                signbits: 1,
            },
            surface_flags: 7,
            contents: 9,
            entity_num: 42,
        };
        let mut bytes = vec![0xFFu8; QVM_TRACE_BYTES];
        write_qvm_trace(&mut bytes, &value).unwrap();
        assert_eq!(read_i32(&bytes, 0), 1);
        assert_eq!(read_i32(&bytes, 4), 0);
        assert_eq!(read_f32(&bytes, 8), 0.5);
        assert_eq!((bytes[40], bytes[41], bytes[42], bytes[43]), (2, 1, 0, 0));
        assert_eq!(read_i32(&bytes, 52), 42);
        assert_eq!(read_qvm_trace(&bytes).unwrap(), value);
    }

    #[test]
    fn short_records_rejected_before_mutation() {
        let mut short = vec![1u8; 8];
        assert!(write_qvm_trace(&mut short, &QvmTraceRecord::default()).is_err());
        assert_eq!(short, vec![1u8; 8]);
        assert!(read_qvm_trace(&short).is_err());
    }
}
