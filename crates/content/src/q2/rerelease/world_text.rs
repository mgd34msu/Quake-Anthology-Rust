//! Q2 rerelease world text (`src/content/q2/rerelease/world-text.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::math::{Vec3, Vec4};

use crate::q2::support::misc::{WorldTextFont, WorldTextInput, WorldTextOrientation};

/// World text request (`q2WorldText` input).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WorldTextRequest {
    /// Origin.
    pub origin: Vec3,
    /// Fixed angles, or a billboard when absent.
    pub angles: Option<Vec3>,
    /// Text.
    pub text: String,
    /// Color.
    pub color: Vec4,
    /// Size in eight-unit cells.
    pub size: f64,
    /// Whether depth testing applies.
    pub depth_test: bool,
}

/// Build world text (`q2WorldText`).
///
/// Q2's textured debug font uses 127 byte glyphs and eight-unit cells.
pub fn q2_world_text(input: &Q2WorldTextRequest) -> WorldTextInput {
    let text: String = input
        .text
        .encode_utf16()
        .take(127)
        .map(|unit| (unit & 0xFF) as u8 as char)
        .collect();
    WorldTextInput {
        text,
        origin: input.origin,
        color: input.color,
        cell_size: input.size * 8.0,
        distance_cull_factor: Some(0.004),
        orientation: match input.angles {
            None => WorldTextOrientation::Billboard,
            Some(angles) => WorldTextOrientation::Fixed { angles },
        },
        depth_test: input.depth_test,
        font: WorldTextFont::Classic,
    }
}
