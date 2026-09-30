//! Quake III presentation: refdef.
//!
//! Donor provenance: `src/content/q3/presentation/refdef.ts`.

use qa_core::math::{Axis, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::ref_entity::*;
use crate::q3::presentation::ref_entity::{PresentError, PresentResult};

// ---------------------------------------------------------------------------
// refdef.ts
// ---------------------------------------------------------------------------

/// Render text: eight rows (`RenderText`).
pub type RenderText = [String; 8];

/// Copy render text with source bounds checks (`copyRenderText`).
pub fn copy_render_text(source: &RenderText) -> PresentResult<RenderText> {
    for row in source {
        let bytes = row.as_bytes();
        if bytes.len() > 32 || (bytes.len() == 32 && !bytes.contains(&0)) {
            return Err(PresentError::range(
                "refdef render string requires a NUL within 32 bytes",
            ));
        }
        if !row.is_ascii() {
            return Err(PresentError::range("refdef render strings require byte characters"));
        }
    }
    Ok(source.clone())
}

/// No-world-model render flag.
pub const RDF_NOWORLDMODEL: i32 = 1;

/// Hyperspace render flag.
pub const RDF_HYPERSPACE: i32 = 4;

/// Reference definition (`Refdef`).
#[derive(Debug, Clone, PartialEq)]
pub struct Refdef {
    /// X.
    pub x: i32,
    /// Y.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Horizontal FOV.
    pub fov_x: f32,
    /// Vertical FOV.
    pub fov_y: f32,
    /// View origin.
    pub view_origin: Vec3,
    /// View axis.
    pub view_axis: Axis,
    /// Time.
    pub time: i32,
    /// Render flags.
    pub render_flags: i32,
    /// Area mask (32 bytes).
    pub area_mask: [u8; 32],
    /// Text.
    pub text: RenderText,
}

impl Default for Refdef {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            fov_x: 0.0,
            fov_y: 0.0,
            view_origin: zero_vec3(),
            view_axis: zero_axis(),
            time: 0,
            render_flags: 0,
            area_mask: [0; 32],
            text: ["", "", "", "", "", "", "", ""].map(str::to_string),
        }
    }
}

/// Create a refdef (`createRefdef`).
#[must_use]
pub fn create_refdef() -> Refdef {
    Refdef::default()
}

/// Copy a refdef (`copyRefdef`).
#[must_use]
pub fn copy_refdef(source: &Refdef) -> Refdef {
    source.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_text_bounds() {
        let text: RenderText = ["a", "b", "c", "d", "e", "f", "g", "h"].map(str::to_string);
        assert_eq!(copy_render_text(&text).unwrap(), text);
        let mut bad = text.clone();
        bad[0] = "x".repeat(33);
        assert!(copy_render_text(&bad).is_err());
        let mut bad = text.clone();
        bad[1] = "\u{e9}".to_string();
        assert!(copy_render_text(&bad).is_err());
        let refdef = create_refdef();
        assert_eq!(refdef.area_mask.len(), 32);
        assert_eq!(copy_refdef(&refdef), refdef);
    }
}
