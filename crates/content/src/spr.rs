//! Quake sprite (SPR v1 / SP2 v2) parsers.
//!
//! Donor provenance: `parseSpr` and `parseSp2` in
//! `src/formats/q12-model/sprite.ts`, with shared readers from
//! `src/formats/q12-model/common.ts` (see [`crate::common`]).
//!
//! Frame pixels are owned; the table of contents is read sequentially,
//! borrowing nothing past validation.

use qa_core::binary::{BinaryError, BinaryReader};

use crate::common::{count, fail, read_timed, sync_type, union_bounds, version, Bounds, SyncType, TimedFrames};

/// Sprite orientation (`orientation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpriteOrientation {
    /// Parallel upright.
    ParallelUpright = 0,
    /// Facing upright.
    FacingUpright = 1,
    /// Parallel.
    Parallel = 2,
    /// Oriented.
    Oriented = 3,
    /// Parallel oriented.
    ParallelOriented = 4,
}

fn orientation(reader: &mut BinaryReader<'_>, source: &str) -> Result<SpriteOrientation, BinaryError> {
    match reader.i32()? {
        0 => Ok(SpriteOrientation::ParallelUpright),
        1 => Ok(SpriteOrientation::FacingUpright),
        2 => Ok(SpriteOrientation::Parallel),
        3 => Ok(SpriteOrientation::Oriented),
        4 => Ok(SpriteOrientation::ParallelOriented),
        value => fail(reader, source, format!("invalid sprite orientation {value}")),
    }
}

/// One sprite frame (`SpriteFrame`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteFrame {
    /// Origin X.
    pub origin_x: i32,
    /// Origin Y.
    pub origin_y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Pixels.
    pub pixels: Vec<u8>,
}

fn sprite_frame(reader: &mut BinaryReader<'_>, source: &str) -> Result<SpriteFrame, BinaryError> {
    let origin_x = reader.i32()?;
    let origin_y = reader.i32()?;
    let width = count(reader, source, "sprite width", 1)?;
    let height = count(reader, source, "sprite height", 1)?;
    let pixels = reader.bytes(width as usize * height as usize)?;
    Ok(SpriteFrame {
        origin_x,
        origin_y,
        width,
        height,
        pixels,
    })
}

/// Parsed Q1 sprite (`Q1SpriteModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct SprModel {
    /// Orientation.
    pub orientation: SpriteOrientation,
    /// Bounding radius.
    pub bounding_radius: f32,
    /// Maximum width.
    pub max_width: i32,
    /// Maximum height.
    pub max_height: i32,
    /// Beam length.
    pub beam_length: f32,
    /// Synchronization type.
    pub sync: SyncType,
    /// Frames.
    pub frames: Vec<TimedFrames<SpriteFrame>>,
    /// Bounds.
    pub bounds: Bounds,
}

/// Parse an SPR v1 sprite (`parseSpr`).
pub fn parse_spr(data: &[u8], source: &str) -> Result<SprModel, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    reader.expect_magic("IDSP")?;
    version(&mut reader, source, 1)?;
    let orientation = orientation(&mut reader, source)?;
    let bounding_radius = reader.finite_f32()?;
    let max_width = count(&mut reader, source, "sprite maximum width", 1)?;
    let max_height = count(&mut reader, source, "sprite maximum height", 1)?;
    let frame_count = count(&mut reader, source, "sprite frames", 1)?;
    let beam_length = reader.finite_f32()?;
    let sync = sync_type(&mut reader, source)?;
    if bounding_radius < 0.0 {
        return fail(&reader, source, "negative sprite radius".to_string());
    }
    let mut frames = Vec::with_capacity(frame_count as usize);
    for _ in 0..frame_count {
        frames.push(read_timed(&mut reader, source, |reader| sprite_frame(reader, source))?);
    }
    let bounds = Bounds {
        min: [
            -max_width as f32 / 2.0,
            -max_width as f32 / 2.0,
            -max_height as f32 / 2.0,
        ],
        max: [max_width as f32 / 2.0, max_width as f32 / 2.0, max_height as f32 / 2.0],
    };
    Ok(SprModel {
        orientation,
        bounding_radius,
        max_width,
        max_height,
        beam_length,
        sync,
        frames,
        bounds,
    })
}

/// One Q2 sprite frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sp2Frame {
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Origin X.
    pub origin_x: i32,
    /// Origin Y.
    pub origin_y: i32,
    /// Image name.
    pub image: String,
}

fn sprite_bounds(frame: &Sp2Frame) -> Bounds {
    let right = (frame.origin_x.abs().max((frame.width - frame.origin_x).abs())) as f32;
    let up = (frame.origin_y.abs().max((frame.height - frame.origin_y).abs())) as f32;
    let radius = right.hypot(up);
    Bounds {
        min: [-radius, -radius, -radius],
        max: [radius, radius, radius],
    }
}

/// Parsed Q2 sprite (`Q2SpriteModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct Sp2Model {
    /// Frames.
    pub frames: Vec<Sp2Frame>,
    /// Bounds.
    pub bounds: Bounds,
}

/// Parse an SP2 v2 sprite (`parseSp2`).
pub fn parse_sp2(data: &[u8], source: &str) -> Result<Sp2Model, BinaryError> {
    let mut reader = BinaryReader::new(data, source);
    reader.expect_magic("IDS2")?;
    version(&mut reader, source, 2)?;
    let frame_count = count(&mut reader, source, "sprite frames", 1)?;
    let mut frames = Vec::with_capacity(frame_count as usize);
    for _ in 0..frame_count {
        let width = count(&mut reader, source, "sprite width", 1)?;
        let height = count(&mut reader, source, "sprite height", 1)?;
        let origin_x = reader.i32()?;
        let origin_y = reader.i32()?;
        let image = reader.fixed_byte_string(64)?;
        frames.push(Sp2Frame {
            width,
            height,
            origin_x,
            origin_y,
            image,
        });
    }
    let bounds = union_bounds(&frames.iter().map(sprite_bounds).collect::<Vec<_>>());
    Ok(Sp2Model { frames, bounds })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::binary::BinaryWriter;

    fn spr_fixture() -> Vec<u8> {
        let mut writer = BinaryWriter::new(256);
        writer.bytes(b"IDSP").unwrap();
        writer.i32(1).unwrap();
        writer.i32(2).unwrap(); // parallel
        writer.f32(8.0).unwrap();
        writer.i32(64).unwrap();
        writer.i32(32).unwrap();
        writer.i32(2).unwrap(); // frames
        writer.f32(0.0).unwrap();
        writer.i32(0).unwrap(); // sync
                                // Single frame 4x2.
        writer.i32(0).unwrap();
        writer.i32(-2).unwrap();
        writer.i32(3).unwrap();
        writer.i32(4).unwrap();
        writer.i32(2).unwrap();
        writer.bytes(&[1u8, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        // Group frame with one member.
        writer.i32(1).unwrap();
        writer.i32(1).unwrap();
        writer.f32(0.2).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.i32(2).unwrap();
        writer.i32(2).unwrap();
        writer.bytes(&[9u8, 9, 9, 9]).unwrap();
        writer.finish()
    }

    fn sp2_fixture() -> Vec<u8> {
        let mut writer = BinaryWriter::new(256);
        writer.bytes(b"IDS2").unwrap();
        writer.i32(2).unwrap();
        writer.i32(1).unwrap();
        writer.i32(16).unwrap();
        writer.i32(16).unwrap();
        writer.i32(8).unwrap();
        writer.i32(8).unwrap();
        let mut name = [0u8; 64];
        name[..10].copy_from_slice(b"sprite.pcx");
        writer.bytes(&name).unwrap();
        writer.finish()
    }

    #[test]
    fn spr_round_trip() {
        let model = parse_spr(&spr_fixture(), "<test>").unwrap();
        assert_eq!(model.orientation, SpriteOrientation::Parallel);
        assert_eq!(model.max_width, 64);
        assert_eq!(model.frames.len(), 2);
        match &model.frames[0] {
            TimedFrames::Single(frame) => {
                assert_eq!((frame.origin_x, frame.origin_y), (-2, 3));
                assert_eq!(frame.pixels, vec![1, 2, 3, 4, 5, 6, 7, 8]);
            }
            TimedFrames::Group(_) => panic!("expected single frame"),
        }
        match &model.frames[1] {
            TimedFrames::Group(frames) => {
                assert_eq!(frames.len(), 1);
                assert_eq!(frames[0].interval_seconds, 0.2);
                assert_eq!(frames[0].frame.pixels, vec![9, 9, 9, 9]);
            }
            TimedFrames::Single(_) => panic!("expected group frame"),
        }
        assert_eq!(model.bounds.min, [-32.0, -32.0, -16.0]);
        assert_eq!(model.bounds.max, [32.0, 32.0, 16.0]);
    }

    #[test]
    fn sp2_round_trip() {
        let model = parse_sp2(&sp2_fixture(), "<test>").unwrap();
        assert_eq!(model.frames.len(), 1);
        assert_eq!(model.frames[0].image, "sprite.pcx");
        let radius = 8.0f32.hypot(8.0);
        assert_eq!(model.bounds.min, [-radius, -radius, -radius]);

        assert!(parse_sp2(b"IDS2", "<test>").is_err());
        let mut bad = sp2_fixture();
        bad[4] = 3;
        assert!(parse_sp2(&bad, "<test>").is_err());
    }

    #[test]
    fn spr_rejects_bad_input() {
        let good = spr_fixture();
        assert!(parse_spr(&good[..8], "<test>").is_err());
        let mut bad_orientation = good.clone();
        bad_orientation[8] = 9;
        assert!(parse_spr(&bad_orientation, "<test>").is_err());
    }
}
