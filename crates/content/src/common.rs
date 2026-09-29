//! Shared helpers for Quake model and sprite formats.
//!
//! Donor provenance: `src/formats/q12-model/common.ts` (failure,
//! counting, version, vector, sync/group, interval, timed-frame,
//! packed-vertex, and bounds helpers). All format parsers report
//! [`qa_core::binary::BinaryError`], exactly like the donor.

use qa_core::binary::{BinaryError, BinaryReader};

/// Build a [`BinaryError`] at the reader's current offset.
pub fn fail<T>(reader: &BinaryReader<'_>, source: &str, message: String) -> Result<T, BinaryError> {
    Err(BinaryError {
        input: source.to_string(),
        offset: reader.offset(),
        message,
    })
}

/// Read an `i32` count that must reach `minimum` (`count`).
pub fn count(reader: &mut BinaryReader<'_>, source: &str, label: &str, minimum: i32) -> Result<i32, BinaryError> {
    let value = reader.i32()?;
    if value < minimum {
        return fail(reader, source, format!("{label} {value} is below {minimum}"));
    }
    Ok(value)
}

/// Check a format version (`version`).
pub fn version(reader: &mut BinaryReader<'_>, source: &str, expected: i32) -> Result<(), BinaryError> {
    let actual = reader.i32()?;
    if actual != expected {
        return fail(reader, source, format!("version {actual}, expected {expected}"));
    }
    Ok(())
}

/// Read three finite `f32` values (`vector`).
pub fn vector(reader: &mut BinaryReader<'_>) -> Result<[f32; 3], BinaryError> {
    Ok([reader.finite_f32()?, reader.finite_f32()?, reader.finite_f32()?])
}

/// Frame synchronization type (`syncType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncType {
    /// Frames play in order.
    Synchronized,
    /// Frames play in random order.
    Random,
}

/// Read a synchronization type (`syncType`).
pub fn sync_type(reader: &mut BinaryReader<'_>, source: &str) -> Result<SyncType, BinaryError> {
    match reader.i32()? {
        0 => Ok(SyncType::Synchronized),
        1 => Ok(SyncType::Random),
        value => fail(reader, source, format!("invalid synchronization type {value}")),
    }
}

/// Single-or-group marker (`groupType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupType {
    /// One frame.
    Single,
    /// Timed group.
    Group,
}

/// Read a single-or-group marker (`groupType`).
pub fn group_type(reader: &mut BinaryReader<'_>, source: &str) -> Result<GroupType, BinaryError> {
    match reader.i32()? {
        0 => Ok(GroupType::Single),
        1 => Ok(GroupType::Group),
        value => fail(reader, source, format!("invalid frame/skin type {value}")),
    }
}

/// Read positive group interval endpoints (`intervals`).
pub fn intervals(reader: &mut BinaryReader<'_>, source: &str, frame_count: usize) -> Result<Vec<f32>, BinaryError> {
    let mut result = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let endpoint = reader.finite_f32()?;
        if endpoint <= 0.0 {
            return fail(reader, source, "group interval must be positive".to_string());
        }
        result.push(endpoint);
    }
    Ok(result)
}

/// One timed frame.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedFrame<T> {
    /// Interval in seconds.
    pub interval_seconds: f32,
    /// Frame payload.
    pub frame: T,
}

/// Single frame or timed group (`TimedFrames`).
#[derive(Debug, Clone, PartialEq)]
pub enum TimedFrames<T> {
    /// One frame.
    Single(T),
    /// Timed group.
    Group(Vec<TimedFrame<T>>),
}

/// Read a single frame or timed group (`readTimed`).
pub fn read_timed<T>(
    reader: &mut BinaryReader<'_>,
    source: &str,
    mut read_frame: impl FnMut(&mut BinaryReader<'_>) -> Result<T, BinaryError>,
) -> Result<TimedFrames<T>, BinaryError> {
    if group_type(reader, source)? == GroupType::Single {
        return Ok(TimedFrames::Single(read_frame(reader)?));
    }
    let frame_count = count(reader, source, "group frames", 1)?;
    let endpoints = intervals(reader, source, frame_count as usize)?;
    let mut frames = Vec::with_capacity(endpoints.len());
    for interval_seconds in endpoints {
        frames.push(TimedFrame {
            interval_seconds,
            frame: read_frame(reader)?,
        });
    }
    Ok(TimedFrames::Group(frames))
}

/// Packed alias vertex: three position bytes plus a normal index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedVertex {
    /// Compressed position.
    pub position: [u8; 3],
    /// Normal table index.
    pub normal_index: u8,
}

/// Read a packed vertex (`packedVertex`).
pub fn packed_vertex(reader: &mut BinaryReader<'_>) -> Result<PackedVertex, BinaryError> {
    Ok(PackedVertex {
        position: [reader.u8()?, reader.u8()?, reader.u8()?],
        normal_index: reader.u8()?,
    })
}

/// Expand a position by scale and translation (`expandPosition`).
///
/// `f32` arithmetic rounds after each operation, matching the donor's
/// nested `Math.fround` calls.
#[must_use]
pub fn expand_position(position: [f32; 3], scale: [f32; 3], translate: [f32; 3]) -> [f32; 3] {
    [
        position[0] * scale[0] + translate[0],
        position[1] * scale[1] + translate[1],
        position[2] * scale[2] + translate[2],
    ]
}

/// Expand a packed vertex position (`packedPosition`).
#[must_use]
pub fn packed_position(vertex: PackedVertex, scale: [f32; 3], translate: [f32; 3]) -> [f32; 3] {
    expand_position(
        [
            f32::from(vertex.position[0]),
            f32::from(vertex.position[1]),
            f32::from(vertex.position[2]),
        ],
        scale,
        translate,
    )
}

/// Axis-aligned bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Minimum corner.
    pub min: [f32; 3],
    /// Maximum corner.
    pub max: [f32; 3],
}

/// Bound a point set (`boundsFromPoints`).
#[must_use]
pub fn bounds_from_points(points: &[[f32; 3]]) -> Bounds {
    let first = points[0];
    let mut min = first;
    let mut max = first;
    for point in points {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Bounds { min, max }
}

/// Union bounds (`unionBounds`).
#[must_use]
pub fn union_bounds(bounds: &[Bounds]) -> Bounds {
    let mut points = Vec::with_capacity(bounds.len() * 2);
    for bound in bounds {
        points.push(bound.min);
        points.push(bound.max);
    }
    bounds_from_points(&points)
}

/// Validate an index (`index`).
pub fn check_index(
    reader: &BinaryReader<'_>,
    source: &str,
    value: i32,
    limit: i32,
    label: &str,
) -> Result<u32, BinaryError> {
    if value < 0 || value >= limit {
        return fail(
            reader,
            source,
            format!("{label} index {value} outside 0..{}", limit - 1),
        );
    }
    Ok(value as u32)
}
