//! Render-state values and source state bits (`GL_State`).
//!
//! Donor provenance: `src/contracts/render.ts` (`RenderState`,
//! `BlendFactor`) and `src/materials/source-state.ts` (`SourceStateBit`,
//! `sourceStateBits`, `sourceStateChanges`, from `tr_backend.c`).
//!
//! Data level only: these types describe pipeline state for material
//! evaluation. No GPU calls are made from this crate.

use crate::ClientError;

/// Blend factor (`BlendFactor` contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendFactor {
    /// Zero.
    Zero,
    /// One.
    One,
    /// Destination color.
    DstColor,
    /// One minus destination color.
    OneMinusDstColor,
    /// Source alpha.
    SrcAlpha,
    /// One minus source alpha.
    OneMinusSrcAlpha,
    /// Destination alpha.
    DstAlpha,
    /// One minus destination alpha.
    OneMinusDstAlpha,
    /// Source alpha saturate.
    SrcAlphaSaturate,
    /// Source color.
    SrcColor,
    /// One minus source color.
    OneMinusSrcColor,
}

/// Depth test (`RenderState["depthTest"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthTest {
    /// Less-or-equal.
    LessEqual,
    /// Equal.
    Equal,
    /// Always pass.
    Always,
}

/// Alpha test (`RenderState["alphaTest"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaTest {
    /// Disabled.
    None,
    /// Greater than zero.
    Gt0,
    /// Less than 128.
    Lt128,
    /// Greater-or-equal 128.
    Ge128,
}

/// Face culling (`RenderState["cull"]`, the GL face to discard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullFace {
    /// No culling.
    None,
    /// Cull back faces.
    Back,
    /// Cull front faces.
    Front,
}

/// Blend equation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blend {
    /// Source factor.
    pub source: BlendFactor,
    /// Destination factor.
    pub destination: BlendFactor,
}

/// Opaque blend (source one, destination zero).
pub const OPAQUE_BLEND: Blend = Blend {
    source: BlendFactor::One,
    destination: BlendFactor::Zero,
};

/// Filter blend (source dst-color, destination zero).
pub const FILTER_BLEND: Blend = Blend {
    source: BlendFactor::DstColor,
    destination: BlendFactor::Zero,
};

/// Additive blend (source one, destination one).
pub const ADDITIVE_BLEND: Blend = Blend {
    source: BlendFactor::One,
    destination: BlendFactor::One,
};

/// Polygon offset (`RenderState["polygonOffset"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolygonOffset {
    /// Scale factor.
    pub factor: f32,
    /// Depth units.
    pub units: f32,
}

/// Pipeline state (`RenderState` contract).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderState {
    /// Blend equation.
    pub blend: Blend,
    /// Depth test.
    pub depth_test: DepthTest,
    /// Depth write.
    pub depth_write: bool,
    /// Alpha test.
    pub alpha_test: AlphaTest,
    /// Face culling.
    pub cull: CullFace,
    /// Depth range.
    pub depth_range: [f32; 2],
    /// Polygon offset.
    pub polygon_offset: Option<PolygonOffset>,
}

impl RenderState {
    /// Default state: opaque, less-equal, write, no cull override.
    #[must_use]
    pub const fn opaque(cull: CullFace) -> Self {
        Self {
            blend: OPAQUE_BLEND,
            depth_test: DepthTest::LessEqual,
            depth_write: true,
            alpha_test: AlphaTest::None,
            cull,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
        }
    }
}

/// Source state bits (`SourceStateBit`).
pub mod bits {
    /// Source blend zero.
    pub const SRCBLEND_ZERO: u32 = 0x0000_0001;
    /// Source blend one.
    pub const SRCBLEND_ONE: u32 = 0x0000_0002;
    /// Source blend dst color.
    pub const SRCBLEND_DST_COLOR: u32 = 0x0000_0003;
    /// Source blend one minus dst color.
    pub const SRCBLEND_ONE_MINUS_DST_COLOR: u32 = 0x0000_0004;
    /// Source blend src alpha.
    pub const SRCBLEND_SRC_ALPHA: u32 = 0x0000_0005;
    /// Source blend one minus src alpha.
    pub const SRCBLEND_ONE_MINUS_SRC_ALPHA: u32 = 0x0000_0006;
    /// Source blend dst alpha.
    pub const SRCBLEND_DST_ALPHA: u32 = 0x0000_0007;
    /// Source blend one minus dst alpha.
    pub const SRCBLEND_ONE_MINUS_DST_ALPHA: u32 = 0x0000_0008;
    /// Source blend alpha saturate.
    pub const SRCBLEND_ALPHA_SATURATE: u32 = 0x0000_0009;
    /// Source blend mask.
    pub const SRCBLEND_BITS: u32 = 0x0000_000f;
    /// Destination blend zero.
    pub const DSTBLEND_ZERO: u32 = 0x0000_0010;
    /// Destination blend one.
    pub const DSTBLEND_ONE: u32 = 0x0000_0020;
    /// Destination blend src color.
    pub const DSTBLEND_SRC_COLOR: u32 = 0x0000_0030;
    /// Destination blend one minus src color.
    pub const DSTBLEND_ONE_MINUS_SRC_COLOR: u32 = 0x0000_0040;
    /// Destination blend src alpha.
    pub const DSTBLEND_SRC_ALPHA: u32 = 0x0000_0050;
    /// Destination blend one minus src alpha.
    pub const DSTBLEND_ONE_MINUS_SRC_ALPHA: u32 = 0x0000_0060;
    /// Destination blend dst alpha.
    pub const DSTBLEND_DST_ALPHA: u32 = 0x0000_0070;
    /// Destination blend one minus dst alpha.
    pub const DSTBLEND_ONE_MINUS_DST_ALPHA: u32 = 0x0000_0080;
    /// Destination blend mask.
    pub const DSTBLEND_BITS: u32 = 0x0000_00f0;
    /// Depth mask true.
    pub const DEPTHMASK_TRUE: u32 = 0x0000_0100;
    /// Line polygon mode.
    pub const POLYMODE_LINE: u32 = 0x0000_1000;
    /// Depth test disabled.
    pub const DEPTHTEST_DISABLE: u32 = 0x0001_0000;
    /// Equal depth function.
    pub const DEPTHFUNC_EQUAL: u32 = 0x0002_0000;
    /// Alpha test greater than zero.
    pub const ATEST_GT_0: u32 = 0x1000_0000;
    /// Alpha test less than 128.
    pub const ATEST_LT_80: u32 = 0x2000_0000;
    /// Alpha test greater-or-equal 128.
    pub const ATEST_GE_80: u32 = 0x4000_0000;
    /// Alpha test mask.
    pub const ATEST_BITS: u32 = 0x7000_0000;
    /// Default state bits.
    pub const DEFAULT: u32 = DEPTHMASK_TRUE;
    /// Combined blend mask.
    pub const BLEND_MASK: u32 = SRCBLEND_BITS | DSTBLEND_BITS;
}

/// Input for [`source_state_bits`] (`SourceStateInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceStateInput {
    /// Depth test (less-equal or equal only).
    pub depth_test: DepthTest,
    /// Depth write.
    pub depth_write: bool,
    /// Blend, or `None` for disabled blending.
    pub blend: Option<Blend>,
    /// Alpha test.
    pub alpha_test: AlphaTest,
}

/// Polygon mode for [`source_state_bits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolygonMode {
    /// Fill.
    Fill,
    /// Line.
    Line,
}

fn source_blend_bits(factor: BlendFactor) -> Result<u32, ClientError> {
    use BlendFactor as F;
    use bits as B;
    match factor {
        F::Zero => Ok(B::SRCBLEND_ZERO),
        F::One => Ok(B::SRCBLEND_ONE),
        F::DstColor => Ok(B::SRCBLEND_DST_COLOR),
        F::OneMinusDstColor => Ok(B::SRCBLEND_ONE_MINUS_DST_COLOR),
        F::SrcAlpha => Ok(B::SRCBLEND_SRC_ALPHA),
        F::OneMinusSrcAlpha => Ok(B::SRCBLEND_ONE_MINUS_SRC_ALPHA),
        F::DstAlpha => Ok(B::SRCBLEND_DST_ALPHA),
        F::OneMinusDstAlpha => Ok(B::SRCBLEND_ONE_MINUS_DST_ALPHA),
        F::SrcAlphaSaturate => Ok(B::SRCBLEND_ALPHA_SATURATE),
        F::SrcColor | F::OneMinusSrcColor => Err(ClientError::BadMaterial(
            "GL_State cannot encode a destination-only blend factor as source".to_string(),
        )),
    }
}

fn destination_blend_bits(factor: BlendFactor) -> Result<u32, ClientError> {
    use BlendFactor as F;
    use bits as B;
    match factor {
        F::Zero => Ok(B::DSTBLEND_ZERO),
        F::One => Ok(B::DSTBLEND_ONE),
        F::SrcColor => Ok(B::DSTBLEND_SRC_COLOR),
        F::OneMinusSrcColor => Ok(B::DSTBLEND_ONE_MINUS_SRC_COLOR),
        F::SrcAlpha => Ok(B::DSTBLEND_SRC_ALPHA),
        F::OneMinusSrcAlpha => Ok(B::DSTBLEND_ONE_MINUS_SRC_ALPHA),
        F::DstAlpha => Ok(B::DSTBLEND_DST_ALPHA),
        F::OneMinusDstAlpha => Ok(B::DSTBLEND_ONE_MINUS_DST_ALPHA),
        F::DstColor | F::OneMinusDstColor | F::SrcAlphaSaturate => Err(ClientError::BadMaterial(
            "GL_State cannot encode a source-only blend factor as destination".to_string(),
        )),
    }
}

fn alpha_test_bits(test: AlphaTest) -> u32 {
    use bits as B;
    match test {
        AlphaTest::None => 0,
        AlphaTest::Gt0 => B::ATEST_GT_0,
        AlphaTest::Lt128 => B::ATEST_LT_80,
        AlphaTest::Ge128 => B::ATEST_GE_80,
    }
}

/// Encode `GL_State` bits (`sourceStateBits`).
///
/// A `None` blend encodes disabled blending; explicit one/zero still
/// enables blending.
pub fn source_state_bits(
    input: &SourceStateInput,
    polygon_mode: PolygonMode,
    depth_test_enabled: bool,
) -> Result<u32, ClientError> {
    use bits as B;
    let mut state = match input.depth_test {
        DepthTest::LessEqual => 0,
        DepthTest::Equal => B::DEPTHFUNC_EQUAL,
        DepthTest::Always => {
            return Err(ClientError::BadMaterial(
                "GL_State cannot encode depth function 'always'".to_string(),
            ));
        }
    };
    if let Some(blend) = input.blend {
        state |= source_blend_bits(blend.source)?;
        state |= destination_blend_bits(blend.destination)?;
    }
    if input.depth_write {
        state |= B::DEPTHMASK_TRUE;
    }
    if polygon_mode == PolygonMode::Line {
        state |= B::POLYMODE_LINE;
    }
    if !depth_test_enabled {
        state |= B::DEPTHTEST_DISABLE;
    }
    Ok(state | alpha_test_bits(input.alpha_test))
}

/// A `GL_State` transition (`SourceStateChange`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceStateChange {
    /// Depth function changed.
    DepthFunction(DepthTest),
    /// Blending disabled.
    BlendDisabled,
    /// Blending enabled.
    BlendEnabled {
        /// Source factor.
        source: BlendFactor,
        /// Destination factor.
        destination: BlendFactor,
    },
    /// Depth write changed.
    DepthWrite(bool),
    /// Polygon mode changed.
    PolygonMode(PolygonMode),
    /// Depth test enable changed.
    DepthTest(bool),
    /// Alpha test changed.
    AlphaTest(AlphaTest),
}

fn decode_source_factor(state: u32) -> Result<BlendFactor, ClientError> {
    use BlendFactor as F;
    use bits as B;
    match state {
        B::SRCBLEND_ZERO => Ok(F::Zero),
        B::SRCBLEND_ONE => Ok(F::One),
        B::SRCBLEND_DST_COLOR => Ok(F::DstColor),
        B::SRCBLEND_ONE_MINUS_DST_COLOR => Ok(F::OneMinusDstColor),
        B::SRCBLEND_SRC_ALPHA => Ok(F::SrcAlpha),
        B::SRCBLEND_ONE_MINUS_SRC_ALPHA => Ok(F::OneMinusSrcAlpha),
        B::SRCBLEND_DST_ALPHA => Ok(F::DstAlpha),
        B::SRCBLEND_ONE_MINUS_DST_ALPHA => Ok(F::OneMinusDstAlpha),
        B::SRCBLEND_ALPHA_SATURATE => Ok(F::SrcAlphaSaturate),
        _ => Err(ClientError::BadMaterial(
            "GL_State: invalid src blend state bits\n".to_string(),
        )),
    }
}

fn decode_destination_factor(state: u32) -> Result<BlendFactor, ClientError> {
    use BlendFactor as F;
    use bits as B;
    match state {
        B::DSTBLEND_ZERO => Ok(F::Zero),
        B::DSTBLEND_ONE => Ok(F::One),
        B::DSTBLEND_SRC_COLOR => Ok(F::SrcColor),
        B::DSTBLEND_ONE_MINUS_SRC_COLOR => Ok(F::OneMinusSrcColor),
        B::DSTBLEND_SRC_ALPHA => Ok(F::SrcAlpha),
        B::DSTBLEND_ONE_MINUS_SRC_ALPHA => Ok(F::OneMinusSrcAlpha),
        B::DSTBLEND_DST_ALPHA => Ok(F::DstAlpha),
        B::DSTBLEND_ONE_MINUS_DST_ALPHA => Ok(F::OneMinusDstAlpha),
        _ => Err(ClientError::BadMaterial(
            "GL_State: invalid dst blend state bits\n".to_string(),
        )),
    }
}

/// Diff two state words (`sourceStateChanges`).
///
/// `None` previous yields every change. Apply each operation before
/// advancing, then commit next only after completion.
pub fn source_state_changes(
    previous: Option<u32>,
    next: u32,
) -> Result<Vec<SourceStateChange>, ClientError> {
    use bits as B;
    let diff = match previous {
        None => u32::MAX,
        Some(prev) => prev ^ next,
    };
    if diff == 0 {
        return Ok(Vec::new());
    }
    let mut changes = Vec::new();
    if diff & B::DEPTHFUNC_EQUAL != 0 {
        changes.push(SourceStateChange::DepthFunction(
            if next & B::DEPTHFUNC_EQUAL != 0 {
                DepthTest::Equal
            } else {
                DepthTest::LessEqual
            },
        ));
    }
    if diff & (B::SRCBLEND_BITS | B::DSTBLEND_BITS) != 0 {
        if next & (B::SRCBLEND_BITS | B::DSTBLEND_BITS) != 0 {
            changes.push(SourceStateChange::BlendEnabled {
                source: decode_source_factor(next & B::SRCBLEND_BITS)?,
                destination: decode_destination_factor(next & B::DSTBLEND_BITS)?,
            });
        } else {
            changes.push(SourceStateChange::BlendDisabled);
        }
    }
    if diff & B::DEPTHMASK_TRUE != 0 {
        changes.push(SourceStateChange::DepthWrite(
            next & B::DEPTHMASK_TRUE != 0,
        ));
    }
    if diff & B::POLYMODE_LINE != 0 {
        changes.push(SourceStateChange::PolygonMode(
            if next & B::POLYMODE_LINE != 0 {
                PolygonMode::Line
            } else {
                PolygonMode::Fill
            },
        ));
    }
    if diff & B::DEPTHTEST_DISABLE != 0 {
        changes.push(SourceStateChange::DepthTest(
            next & B::DEPTHTEST_DISABLE == 0,
        ));
    }
    if diff & B::ATEST_BITS != 0 {
        match next & B::ATEST_BITS {
            0 => changes.push(SourceStateChange::AlphaTest(AlphaTest::None)),
            B::ATEST_GT_0 => changes.push(SourceStateChange::AlphaTest(AlphaTest::Gt0)),
            B::ATEST_LT_80 => changes.push(SourceStateChange::AlphaTest(AlphaTest::Lt128)),
            B::ATEST_GE_80 => changes.push(SourceStateChange::AlphaTest(AlphaTest::Ge128)),
            // The source default is assert(0), a no-op in NDEBUG release.
            _ => {}
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_encodes_depth_mask_only() {
        let state = source_state_bits(
            &SourceStateInput {
                depth_test: DepthTest::LessEqual,
                depth_write: true,
                blend: None,
                alpha_test: AlphaTest::None,
            },
            PolygonMode::Fill,
            true,
        )
        .unwrap();
        assert_eq!(state, bits::DEFAULT);
    }

    #[test]
    fn blend_round_trips_through_changes() {
        let next = source_state_bits(
            &SourceStateInput {
                depth_test: DepthTest::Equal,
                depth_write: false,
                blend: Some(ADDITIVE_BLEND),
                alpha_test: AlphaTest::Gt0,
            },
            PolygonMode::Fill,
            true,
        )
        .unwrap();
        let changes = source_state_changes(Some(bits::DEFAULT), next).unwrap();
        assert!(changes.contains(&SourceStateChange::DepthFunction(DepthTest::Equal)));
        assert!(changes.contains(&SourceStateChange::BlendEnabled {
            source: BlendFactor::One,
            destination: BlendFactor::One,
        }));
        assert!(changes.contains(&SourceStateChange::DepthWrite(false)));
        assert!(changes.contains(&SourceStateChange::AlphaTest(AlphaTest::Gt0)));
    }

    #[test]
    fn invalid_blend_bits_are_an_error() {
        let err = source_state_changes(Some(0), 0x0000_000f).unwrap_err();
        assert!(matches!(err, ClientError::BadMaterial(_)));
    }

    #[test]
    fn no_diff_yields_no_changes() {
        assert!(source_state_changes(Some(7), 7).unwrap().is_empty());
    }
}
