//! QVM render record codecs (`refEntity_t`, `refdef_t`, polygons, orientation).
//!
//! Provenance: `src/compat/qvm/render-record.ts` (QVM layouts from id
//! Software's `code/cgame/tr_types.h` and `code/game/q_shared.h`).
//!
//! [`QvmRefEntity`], [`QvmRefdef`], and [`QvmPolyVertex`] mirror
//! `src/content/q3/presentation/ref-entity.ts` (`SourceRefEntityRecord`) and
//! `src/content/q3/presentation/refdef.ts` (`Refdef`); model/shader/skin
//! handles stay numeric (the donor copies the record without resolving
//! handles).

use qa_core::math::{vec2, vec3, Axis, Vec2, Vec3};

use crate::error::GuestError;

/// `refEntity_t` size in bytes.
pub const QVM_REF_ENTITY_BYTES: usize = 140;
/// `refdef_t` size in bytes.
pub const QVM_REFDEF_BYTES: usize = 368;
/// `polyVert_t` size in bytes.
pub const QVM_POLY_VERTEX_BYTES: usize = 24;
/// `orientation_t` size in bytes.
pub const QVM_ORIENTATION_BYTES: usize = 48;

/// Reference-entity presentation kind (source `reType_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmRefEntityKind {
    /// Model entity.
    Model,
    /// Polygon entity.
    Poly,
    /// Sprite entity.
    Sprite,
    /// Beam entity.
    Beam,
    /// Rail core entity.
    RailCore,
    /// Rail rings entity.
    RailRings,
    /// Lightning entity.
    Lightning,
    /// Portal surface entity.
    PortalSurface,
}

impl QvmRefEntityKind {
    /// Decode a source entity-type tag.
    pub fn decode(value: i32) -> Result<Self, GuestError> {
        match value {
            0 => Ok(Self::Model),
            1 => Ok(Self::Poly),
            2 => Ok(Self::Sprite),
            3 => Ok(Self::Beam),
            4 => Ok(Self::RailCore),
            5 => Ok(Self::RailRings),
            6 => Ok(Self::Lightning),
            7 => Ok(Self::PortalSurface),
            _ => Err(GuestError::invalid(format!(
                "QVM refEntity_t unsupported entity type {value}"
            ))),
        }
    }
}

/// Owned `refEntity_t` record with numeric resource handles.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmRefEntity {
    /// Presentation kind.
    pub kind: QvmRefEntityKind,
    /// Render flags.
    pub render_flags: i32,
    /// Model handle.
    pub model: i32,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Orientation axes.
    pub axis: Axis,
    /// Whether axes are non-normalized.
    pub non_normalized_axes: bool,
    /// Origin.
    pub origin: Vec3,
    /// Frame.
    pub frame: i32,
    /// Old origin.
    pub old_origin: Vec3,
    /// Old frame.
    pub old_frame: i32,
    /// Back-lerp fraction.
    pub back_lerp: f32,
    /// Skin number.
    pub skin_num: i32,
    /// Custom skin handle.
    pub custom_skin: i32,
    /// Custom shader handle.
    pub custom_shader: i32,
    /// Shader RGBA byte channels.
    pub shader_rgba: [u8; 4],
    /// Shader texture coordinates.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Radius.
    pub radius: f32,
    /// Rotation.
    pub rotation: f32,
}

/// Owned `refdef_t` record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmRefdef {
    /// Viewport X.
    pub x: i32,
    /// Viewport Y.
    pub y: i32,
    /// Viewport width.
    pub width: i32,
    /// Viewport height.
    pub height: i32,
    /// Horizontal field of view.
    pub fov_x: f32,
    /// Vertical field of view.
    pub fov_y: f32,
    /// View origin.
    pub view_origin: Vec3,
    /// View axes.
    pub view_axis: Axis,
    /// Time.
    pub time: i32,
    /// Render flags.
    pub render_flags: i32,
    /// Area mask bytes.
    pub area_mask: [u8; 32],
    /// Eight 32-byte text rows, including bytes after NUL.
    pub text: [String; 8],
}

/// Owned `polyVert_t` record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPolyVertex {
    /// Position.
    pub position: Vec3,
    /// Texture coordinates.
    pub tex_coord: Vec2,
    /// Modulate byte channels.
    pub color: [u8; 4],
}

fn check_bytes(bytes: &[u8], need: usize, what: &str) -> Result<(), GuestError> {
    if bytes.len() < need {
        return Err(GuestError::invalid(format!(
            "{what} record requires {need} bytes, received {}",
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

fn read_vec3(bytes: &[u8], offset: usize) -> Vec3 {
    vec3(
        read_f32(bytes, offset),
        read_f32(bytes, offset + 4),
        read_f32(bytes, offset + 8),
    )
}

fn read_axis(bytes: &[u8], offset: usize) -> Axis {
    [
        read_vec3(bytes, offset),
        read_vec3(bytes, offset + 12),
        read_vec3(bytes, offset + 24),
    ]
}

fn read_color(bytes: &[u8], offset: usize) -> [u8; 4] {
    [
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]
}

/// Copy a source record without resolving its model, shader, or skin handles.
pub fn read_qvm_ref_entity(bytes: &[u8]) -> Result<QvmRefEntity, GuestError> {
    check_bytes(bytes, QVM_REF_ENTITY_BYTES, "QVM refEntity_t")?;
    Ok(QvmRefEntity {
        kind: QvmRefEntityKind::decode(read_i32(bytes, 0))?,
        render_flags: read_i32(bytes, 4),
        model: read_i32(bytes, 8),
        lighting_origin: read_vec3(bytes, 12),
        shadow_plane: read_f32(bytes, 24),
        axis: read_axis(bytes, 28),
        non_normalized_axes: read_i32(bytes, 64) != 0,
        origin: read_vec3(bytes, 68),
        frame: read_i32(bytes, 80),
        old_origin: read_vec3(bytes, 84),
        old_frame: read_i32(bytes, 96),
        back_lerp: read_f32(bytes, 100),
        skin_num: read_i32(bytes, 104),
        custom_skin: read_i32(bytes, 108),
        custom_shader: read_i32(bytes, 112),
        shader_rgba: read_color(bytes, 116),
        shader_tex_coord: vec2(read_f32(bytes, 120), read_f32(bytes, 124)),
        shader_time: read_f32(bytes, 128),
        radius: read_f32(bytes, 132),
        rotation: read_f32(bytes, 136),
    })
}

fn read_text_row(bytes: &[u8], offset: usize) -> Result<String, GuestError> {
    let row = &bytes[offset..offset + 32];
    if !row.contains(&0) {
        return Err(GuestError::invalid(
            "QVM refdef_t render text row has no NUL within 32 bytes",
        ));
    }
    Ok(row.iter().map(|byte| *byte as char).collect())
}

/// Read a `refdef_t`, owning the mask and complete text rows.
pub fn read_qvm_refdef(bytes: &[u8]) -> Result<QvmRefdef, GuestError> {
    check_bytes(bytes, QVM_REFDEF_BYTES, "QVM refdef_t")?;
    let mut area_mask = [0u8; 32];
    area_mask.copy_from_slice(&bytes[80..112]);
    Ok(QvmRefdef {
        x: read_i32(bytes, 0),
        y: read_i32(bytes, 4),
        width: read_i32(bytes, 8),
        height: read_i32(bytes, 12),
        fov_x: read_f32(bytes, 16),
        fov_y: read_f32(bytes, 20),
        view_origin: read_vec3(bytes, 24),
        view_axis: read_axis(bytes, 36),
        time: read_i32(bytes, 72),
        render_flags: read_i32(bytes, 76),
        area_mask,
        text: [
            read_text_row(bytes, 112)?,
            read_text_row(bytes, 144)?,
            read_text_row(bytes, 176)?,
            read_text_row(bytes, 208)?,
            read_text_row(bytes, 240)?,
            read_text_row(bytes, 272)?,
            read_text_row(bytes, 304)?,
            read_text_row(bytes, 336)?,
        ],
    })
}

/// Read polygon vertices; the caller owns admission and grouping.
pub fn read_qvm_poly_vertices(bytes: &[u8], count: usize) -> Result<Vec<QvmPolyVertex>, GuestError> {
    if count > bytes.len() / QVM_POLY_VERTEX_BYTES {
        return Err(GuestError::invalid(format!(
            "QVM polyVert_t invalid vertex count {count} for {} bytes",
            bytes.len()
        )));
    }
    Ok((0..count)
        .map(|index| {
            let offset = index * QVM_POLY_VERTEX_BYTES;
            QvmPolyVertex {
                position: read_vec3(bytes, offset),
                tex_coord: vec2(
                    read_f32(bytes, offset + 12),
                    read_f32(bytes, offset + 16),
                ),
                color: read_color(bytes, offset + 20),
            }
        })
        .collect())
}

/// Write a lerp-tag result into `orientation_t`, checking extent first.
pub fn write_qvm_orientation(
    bytes: &mut [u8],
    origin: &Vec3,
    axes: &Axis,
) -> Result<(), GuestError> {
    check_bytes(bytes, QVM_ORIENTATION_BYTES, "QVM orientation_t")?;
    let mut write = |offset: usize, value: &Vec3| {
        bytes[offset..offset + 4].copy_from_slice(&value.x.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&value.y.to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&value.z.to_le_bytes());
    };
    write(0, origin);
    write(12, &axes[0]);
    write(24, &axes[1]);
    write(36, &axes[2]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_entity_decodes_model_kind_and_handles() {
        let mut bytes = vec![0u8; QVM_REF_ENTITY_BYTES];
        bytes[0..4].copy_from_slice(&0i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&7i32.to_le_bytes());
        bytes[64..68].copy_from_slice(&1i32.to_le_bytes());
        bytes[104..108].copy_from_slice(&3i32.to_le_bytes());
        bytes[112..116].copy_from_slice(&9i32.to_le_bytes());
        bytes[116..120].copy_from_slice(&[255, 128, 0, 64]);
        let entity = read_qvm_ref_entity(&bytes).unwrap();
        assert_eq!(entity.kind, QvmRefEntityKind::Model);
        assert_eq!(entity.model, 7);
        assert!(entity.non_normalized_axes);
        assert_eq!(entity.skin_num, 3);
        assert_eq!(entity.custom_shader, 9);
        assert_eq!(entity.shader_rgba, [255, 128, 0, 64]);
        bytes[0..4].copy_from_slice(&8i32.to_le_bytes());
        assert!(read_qvm_ref_entity(&bytes).is_err());
        assert!(read_qvm_ref_entity(&bytes[..10]).is_err());
    }

    #[test]
    fn refdef_requires_nul_terminated_rows() {
        let mut bytes = vec![0u8; QVM_REFDEF_BYTES];
        bytes[8..12].copy_from_slice(&640i32.to_le_bytes());
        bytes[112] = b'A';
        let view = read_qvm_refdef(&bytes).unwrap();
        assert_eq!(view.width, 640);
        assert!(view.text[0].starts_with('A'));
        bytes[112..144].fill(b'B');
        assert!(read_qvm_refdef(&bytes).is_err());
    }

    #[test]
    fn poly_vertices_and_orientation_round_trip() {
        let mut bytes = vec![0u8; QVM_POLY_VERTEX_BYTES * 2];
        bytes[0..4].copy_from_slice(&1.0f32.to_le_bytes());
        bytes[20..24].copy_from_slice(&[1, 2, 3, 4]);
        let vertices = read_qvm_poly_vertices(&bytes, 2).unwrap();
        assert_eq!(vertices.len(), 2);
        assert_eq!(vertices[0].position.x, 1.0);
        assert_eq!(vertices[0].color, [1, 2, 3, 4]);
        assert!(read_qvm_poly_vertices(&bytes, 3).is_err());

        let mut orientation = vec![0u8; QVM_ORIENTATION_BYTES];
        let axes = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
        write_qvm_orientation(&mut orientation, &vec3(5.0, 6.0, 7.0), &axes).unwrap();
        assert_eq!(read_f32(&orientation, 0), 5.0);
        assert_eq!(read_f32(&orientation, 24), 1.0);
        let mut short = vec![0u8; 8];
        assert!(write_qvm_orientation(&mut short, &vec3(0.0, 0.0, 0.0), &axes).is_err());
    }
}
