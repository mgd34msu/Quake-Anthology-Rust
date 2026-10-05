//! Ordered renderer wire protocol between the frontend and render worker.
//!
//! Donor provenance: `src/render/worker-protocol.ts` in full. The donor
//! crosses the worker boundary with `structuredClone`; Rust has no shared
//! heap here, so the encoder snapshots every value into owned [`WireValue`]
//! trees and the decoder rebuilds canonical receiver identities from them.
//! Wire keys and variant names match the donor exactly (`camelCase` maps,
//! `kebab-case` kinds). Registry semantics are identical: image ordinals map
//! to one identity per lifetime, dynamic sources travel as numeric tokens,
//! and every encoded image operation carries an acknowledgment id that the
//! decoder echoes back.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use qa_core::math::{Mat4, Plane, Vec2, Vec3, Vec4};

use super::error::RenderError;
use super::types::{
    AlphaTest, BatchFog, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthAtlasDraw,
    DepthAtlasPass, DepthImageLevel, DepthTest, DrawBatch, DrawBuffer, DynamicImageSource, FogEffect, ImageLevel,
    ImageResourceOperation, ImageSource, LevelContent, MultitextureVertex, PairEnvironment, Palette,
    PaletteTransparency, PolygonOffset, Q2Fog, Q2FogOperation, Q2FragmentLight, Q2HeightFog, Q2HeightStop, Q2LightCone,
    Q2LightPass, Q2ModelFragmentLight, Q2ModelShadowLight, Q2ShadowAtlas, Q2ShadowProjection, Rect, RenderCamera,
    RenderCommand, RenderImage, RenderOperation, RenderState, RenderVertex, RenderView, RenderViewState, RendererImage,
    ResourceOwner, RetainedBatch, RetainedDraw, RetainedId, RetainedPassAttrs, RetainedSlice, RetainedSurfaceData,
    SkyVertex, SourceTime, TextureBinding, TextureBundle, TextureFilter, TextureRect, TextureSampling, ViewClear,
    ViewClip, ViewTarget,
};

/// Owned snapshot of one wire node.
///
/// `Bytes` carries RGBA8 pixels, palette colors, and translation tables;
/// `Floats` carries depth pixels. Maps hold `camelCase` donor keys.
#[derive(Debug, Clone, PartialEq)]
pub enum WireValue {
    /// Explicit null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer (ordinals, sizes, tokens, acknowledgments, captures).
    Int(i64),
    /// Single-precision float.
    Float(f32),
    /// UTF-8 string.
    Str(String),
    /// Byte blob.
    Bytes(Vec<u8>),
    /// Float blob.
    Floats(Vec<f32>),
    /// Ordered list.
    List(Vec<WireValue>),
    /// Keyed record.
    Map(Vec<(String, WireValue)>),
}

impl WireValue {
    /// Build a map from static keys.
    #[must_use]
    pub fn object(entries: Vec<(&str, WireValue)>) -> Self {
        Self::Map(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    /// Build a list.
    #[must_use]
    pub fn list(items: Vec<WireValue>) -> Self {
        Self::List(items)
    }

    /// Build a string.
    #[must_use]
    pub fn text(value: &str) -> Self {
        Self::Str(value.to_string())
    }
}

fn bad_wire(detail: impl Into<String>) -> RenderError {
    RenderError::BadWire(detail.into())
}

fn record(value: &WireValue) -> Result<&[(String, WireValue)], RenderError> {
    match value {
        WireValue::Map(entries) => {
            if let Some((_, failure)) = entries.iter().find(|(key, _)| key == "workerFailure") {
                let message = match failure {
                    WireValue::Str(message) => message.clone(),
                    _ => "renderer encoding failed".to_string(),
                };
                return Err(RenderError::Worker(message));
            }
            Ok(entries)
        }
        _ => Err(bad_wire("invalid renderer wire record")),
    }
}

fn field<'a>(entries: &'a [(String, WireValue)], key: &str) -> Result<&'a WireValue, RenderError> {
    entries
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value)
        .ok_or_else(|| bad_wire(format!("renderer wire record lacks `{key}`")))
}

fn optional<'a>(entries: &'a [(String, WireValue)], key: &str) -> Option<&'a WireValue> {
    entries
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value)
}

fn number(value: &WireValue) -> Result<f32, RenderError> {
    match value {
        WireValue::Float(value) => Ok(*value),
        WireValue::Int(value) => Ok(*value as f32),
        _ => Err(bad_wire("invalid renderer wire number")),
    }
}

fn integer(value: &WireValue) -> Result<u32, RenderError> {
    match value {
        WireValue::Int(value) if *value >= 0 && *value <= i64::from(u32::MAX) => Ok(*value as u32),
        WireValue::Float(value) if value.is_finite() && *value >= 0.0 && value.fract() == 0.0 => Ok(*value as u32),
        _ => Err(bad_wire("invalid renderer wire integer")),
    }
}

fn byte(value: &WireValue) -> Result<u8, RenderError> {
    let value = integer(value)?;
    u8::try_from(value).map_err(|_| bad_wire("invalid renderer wire byte"))
}

fn boolean(value: &WireValue) -> Result<bool, RenderError> {
    match value {
        WireValue::Bool(value) => Ok(*value),
        _ => Err(bad_wire("invalid renderer wire boolean")),
    }
}

fn string(value: &WireValue) -> Result<&str, RenderError> {
    match value {
        WireValue::Str(value) => Ok(value),
        _ => Err(bad_wire("invalid renderer wire string")),
    }
}

fn list(value: &WireValue) -> Result<&[WireValue], RenderError> {
    match value {
        WireValue::List(items) => Ok(items),
        _ => Err(bad_wire("invalid renderer wire list")),
    }
}

fn choice<'a>(value: &'a WireValue, choices: &[&str]) -> Result<&'a str, RenderError> {
    let text = string(value)?;
    if choices.contains(&text) {
        Ok(text)
    } else {
        Err(bad_wire("invalid renderer wire variant"))
    }
}

fn vec2(value: &WireValue) -> Result<Vec2, RenderError> {
    let map = record(value)?;
    Ok(Vec2 {
        x: number(field(map, "x")?)?,
        y: number(field(map, "y")?)?,
    })
}

fn vec3(value: &WireValue) -> Result<Vec3, RenderError> {
    let map = record(value)?;
    Ok(Vec3 {
        x: number(field(map, "x")?)?,
        y: number(field(map, "y")?)?,
        z: number(field(map, "z")?)?,
    })
}

fn vec4(value: &WireValue) -> Result<Vec4, RenderError> {
    let map = record(value)?;
    Ok(Vec4 {
        x: number(field(map, "x")?)?,
        y: number(field(map, "y")?)?,
        z: number(field(map, "z")?)?,
        w: number(field(map, "w")?)?,
    })
}

fn rect(value: &WireValue) -> Result<Rect, RenderError> {
    let map = record(value)?;
    Ok(Rect {
        x: number(field(map, "x")?)?,
        y: number(field(map, "y")?)?,
        width: number(field(map, "width")?)?,
        height: number(field(map, "height")?)?,
    })
}

fn pair(value: &WireValue) -> Result<[f32; 2], RenderError> {
    let items = list(value)?;
    if items.len() != 2 {
        return Err(bad_wire("invalid renderer pair"));
    }
    Ok([number(&items[0])?, number(&items[1])?])
}

fn matrix(value: &WireValue) -> Result<Mat4, RenderError> {
    let items = list(value)?;
    if items.len() != 16 {
        return Err(bad_wire("invalid renderer matrix"));
    }
    let mut out = [0.0f32; 16];
    for (slot, item) in out.iter_mut().zip(items.iter()) {
        *slot = number(item)?;
    }
    Ok(out)
}

fn offset(value: &WireValue) -> Result<Option<PolygonOffset>, RenderError> {
    if matches!(value, WireValue::Null) {
        return Ok(None);
    }
    let map = record(value)?;
    Ok(Some(PolygonOffset {
        factor: number(field(map, "factor")?)?,
        units: number(field(map, "units")?)?,
    }))
}

fn cull(value: &WireValue) -> Result<CullFace, RenderError> {
    match choice(value, &["none", "back", "front"])? {
        "none" => Ok(CullFace::None),
        "back" => Ok(CullFace::Back),
        _ => Ok(CullFace::Front),
    }
}

fn blend(value: &WireValue) -> Result<BlendFactor, RenderError> {
    match choice(
        value,
        &[
            "zero",
            "one",
            "src-color",
            "one-minus-src-color",
            "dst-color",
            "one-minus-dst-color",
            "src-alpha",
            "one-minus-src-alpha",
            "dst-alpha",
            "one-minus-dst-alpha",
            "src-alpha-saturate",
        ],
    )? {
        "zero" => Ok(BlendFactor::Zero),
        "one" => Ok(BlendFactor::One),
        "src-color" => Ok(BlendFactor::SrcColor),
        "one-minus-src-color" => Ok(BlendFactor::OneMinusSrcColor),
        "dst-color" => Ok(BlendFactor::DstColor),
        "one-minus-dst-color" => Ok(BlendFactor::OneMinusDstColor),
        "src-alpha" => Ok(BlendFactor::SrcAlpha),
        "one-minus-src-alpha" => Ok(BlendFactor::OneMinusSrcAlpha),
        "dst-alpha" => Ok(BlendFactor::DstAlpha),
        "one-minus-dst-alpha" => Ok(BlendFactor::OneMinusDstAlpha),
        _ => Ok(BlendFactor::SrcAlphaSaturate),
    }
}

fn filter(value: &WireValue) -> Result<TextureFilter, RenderError> {
    match choice(
        value,
        &[
            "nearest",
            "linear",
            "nearest-mipmap-nearest",
            "linear-mipmap-nearest",
            "nearest-mipmap-linear",
            "linear-mipmap-linear",
        ],
    )? {
        "nearest" => Ok(TextureFilter::Nearest),
        "linear" => Ok(TextureFilter::Linear),
        "nearest-mipmap-nearest" => Ok(TextureFilter::NearestMipmapNearest),
        "linear-mipmap-nearest" => Ok(TextureFilter::LinearMipmapNearest),
        "nearest-mipmap-linear" => Ok(TextureFilter::NearestMipmapLinear),
        _ => Ok(TextureFilter::LinearMipmapLinear),
    }
}

fn state(value: &WireValue) -> Result<RenderState, RenderError> {
    let map = record(value)?;
    let blend_map = record(field(map, "blend")?)?;
    Ok(RenderState {
        blend: (
            blend(field(blend_map, "source")?)?,
            blend(field(blend_map, "destination")?)?,
        ),
        depth_test: match choice(field(map, "depthTest")?, &["less-equal", "equal", "always"])? {
            "less-equal" => DepthTest::LessEqual,
            "equal" => DepthTest::Equal,
            _ => DepthTest::Always,
        },
        depth_write: boolean(field(map, "depthWrite")?)?,
        alpha_test: match choice(field(map, "alphaTest")?, &["none", "gt0", "lt128", "ge128"])? {
            "none" => AlphaTest::None,
            "gt0" => AlphaTest::GreaterZero,
            "lt128" => AlphaTest::Less128,
            _ => AlphaTest::GreaterEqual128,
        },
        cull: cull(field(map, "cull")?)?,
        depth_range: pair(field(map, "depthRange")?)?,
        polygon_offset: offset(field(map, "polygonOffset")?)?,
    })
}

fn vertex(value: &WireValue) -> Result<RenderVertex, RenderError> {
    let map = record(value)?;
    Ok(RenderVertex {
        position: vec4(field(map, "position")?)?,
        tex_coord: vec2(field(map, "texCoord")?)?,
        color: vec4(field(map, "color")?)?,
    })
}

fn fog(value: &WireValue) -> Result<BatchFog, RenderError> {
    let map = record(value)?;
    let color = vec3(field(map, "color")?)?;
    match string(field(map, "kind")?)? {
        "constant" => Ok(BatchFog::Constant {
            color,
            amount: number(field(map, "amount")?)?,
        }),
        "exp2" => {
            let effect = match optional(map, "effect") {
                None => FogEffect::Color,
                Some(raw) => match choice(raw, &["color", "none", "rgb", "alpha", "rgba", "overlay"])? {
                    "color" => FogEffect::Color,
                    "none" => FogEffect::None,
                    "rgb" => FogEffect::Rgb,
                    "alpha" => FogEffect::Alpha,
                    "rgba" => FogEffect::Rgba,
                    _ => FogEffect::Overlay,
                },
            };
            Ok(BatchFog::Exp2 {
                color,
                density: number(field(map, "density")?)?,
                effect,
            })
        }
        _ => Err(bad_wire("invalid renderer fog")),
    }
}

fn shadow(value: &WireValue) -> Result<Q2ShadowProjection, RenderError> {
    let map = record(value)?;
    match string(field(map, "kind")?)? {
        "none" => Ok(Q2ShadowProjection::None),
        "point" => Ok(Q2ShadowProjection::Point {
            atlas_rect: vec4(field(map, "atlasRect")?)?,
        }),
        "cone" => Ok(Q2ShadowProjection::Cone {
            matrix: matrix(field(map, "matrix")?)?,
            atlas_rect: vec4(field(map, "atlasRect")?)?,
        }),
        _ => Err(bad_wire("invalid renderer shadow projection")),
    }
}

fn light(value: &WireValue) -> Result<Q2FragmentLight, RenderError> {
    let map = record(value)?;
    let cone_raw = field(map, "cone")?;
    let cone = if matches!(cone_raw, WireValue::Null) {
        None
    } else {
        let cone_map = record(cone_raw)?;
        Some(Q2LightCone {
            direction: vec3(field(cone_map, "direction")?)?,
            cos_half_angle: number(field(cone_map, "cosHalfAngle")?)?,
        })
    };
    Ok(Q2FragmentLight {
        origin: vec3(field(map, "origin")?)?,
        radius: number(field(map, "radius")?)?,
        color: vec3(field(map, "color")?)?,
        scale: number(field(map, "scale")?)?,
        cone,
        shadow: shadow(field(map, "shadow")?)?,
    })
}

fn camera(value: &WireValue) -> Result<RenderCamera, RenderError> {
    let map = record(value)?;
    let axes = list(field(map, "axis")?)?;
    if axes.len() != 3 {
        return Err(bad_wire("invalid renderer axes"));
    }
    let clip_map = record(field(map, "clip")?)?;
    let clip = match string(field(clip_map, "kind")?)? {
        "none" => ViewClip::None,
        "portal" => {
            let plane_map = record(field(clip_map, "plane")?)?;
            ViewClip::Portal {
                plane: Plane {
                    normal: vec3(field(plane_map, "normal")?)?,
                    distance: number(field(plane_map, "distance")?)?,
                },
                mirror: boolean(field(clip_map, "mirror")?)?,
            }
        }
        _ => return Err(bad_wire("invalid renderer camera clip")),
    };
    Ok(RenderCamera {
        origin: vec3(field(map, "origin")?)?,
        axis: [vec3(&axes[0])?, vec3(&axes[1])?, vec3(&axes[2])?],
        projection: matrix(field(map, "projection")?)?,
        viewport: rect(field(map, "viewport")?)?,
        clip,
    })
}

fn level(value: &WireValue) -> Result<LevelContent, RenderError> {
    let map = record(value)?;
    let width = integer(field(map, "width")?)?;
    let height = integer(field(map, "height")?)?;
    let pixels = field(map, "pixels")?;
    let samples = u64::from(width) * u64::from(height);
    match pixels {
        WireValue::Bytes(bytes) if u64::try_from(bytes.len()).unwrap_or(u64::MAX) == samples * 4 => {
            Ok(LevelContent::Rgba(ImageLevel {
                width,
                height,
                pixels: bytes.clone(),
            }))
        }
        WireValue::Floats(samples_out) if u64::try_from(samples_out.len()).unwrap_or(u64::MAX) == samples => {
            Ok(LevelContent::Depth(DepthImageLevel {
                width,
                height,
                pixels: samples_out.clone(),
            }))
        }
        _ => Err(bad_wire("invalid renderer image pixels")),
    }
}

/// Decode one RGBA8 readback level, checking the pixel length exactly.
pub fn decode_image_level(value: &WireValue) -> Result<ImageLevel, RenderError> {
    let map = record(value)?;
    let width = integer(field(map, "width")?)?;
    let height = integer(field(map, "height")?)?;
    match field(map, "pixels")? {
        WireValue::Bytes(pixels)
            if u64::try_from(pixels.len()).unwrap_or(u64::MAX) == u64::from(width) * u64::from(height) * 4 =>
        {
            Ok(ImageLevel {
                width,
                height,
                pixels: pixels.clone(),
            })
        }
        _ => Err(bad_wire("invalid renderer RGBA readback")),
    }
}

fn encode_vec2(value: Vec2) -> WireValue {
    WireValue::object(vec![("x", WireValue::Float(value.x)), ("y", WireValue::Float(value.y))])
}

fn encode_vec3(value: Vec3) -> WireValue {
    WireValue::object(vec![
        ("x", WireValue::Float(value.x)),
        ("y", WireValue::Float(value.y)),
        ("z", WireValue::Float(value.z)),
    ])
}

fn encode_vec4(value: Vec4) -> WireValue {
    WireValue::object(vec![
        ("x", WireValue::Float(value.x)),
        ("y", WireValue::Float(value.y)),
        ("z", WireValue::Float(value.z)),
        ("w", WireValue::Float(value.w)),
    ])
}

fn encode_rect(value: Rect) -> WireValue {
    WireValue::object(vec![
        ("x", WireValue::Float(value.x)),
        ("y", WireValue::Float(value.y)),
        ("width", WireValue::Float(value.width)),
        ("height", WireValue::Float(value.height)),
    ])
}

fn encode_pair(value: [f32; 2]) -> WireValue {
    WireValue::list(vec![WireValue::Float(value[0]), WireValue::Float(value[1])])
}

fn encode_matrix(value: &Mat4) -> WireValue {
    WireValue::list(value.iter().map(|cell| WireValue::Float(*cell)).collect())
}

fn encode_offset(value: Option<PolygonOffset>) -> WireValue {
    match value {
        None => WireValue::Null,
        Some(offset) => WireValue::object(vec![
            ("factor", WireValue::Float(offset.factor)),
            ("units", WireValue::Float(offset.units)),
        ]),
    }
}

fn encode_cull(value: CullFace) -> WireValue {
    WireValue::text(match value {
        CullFace::None => "none",
        CullFace::Back => "back",
        CullFace::Front => "front",
    })
}

fn encode_blend(value: BlendFactor) -> WireValue {
    WireValue::text(match value {
        BlendFactor::Zero => "zero",
        BlendFactor::One => "one",
        BlendFactor::SrcColor => "src-color",
        BlendFactor::OneMinusSrcColor => "one-minus-src-color",
        BlendFactor::DstColor => "dst-color",
        BlendFactor::OneMinusDstColor => "one-minus-dst-color",
        BlendFactor::SrcAlpha => "src-alpha",
        BlendFactor::OneMinusSrcAlpha => "one-minus-src-alpha",
        BlendFactor::DstAlpha => "dst-alpha",
        BlendFactor::OneMinusDstAlpha => "one-minus-dst-alpha",
        BlendFactor::SrcAlphaSaturate => "src-alpha-saturate",
    })
}

fn encode_filter(value: TextureFilter) -> WireValue {
    WireValue::text(match value {
        TextureFilter::Nearest => "nearest",
        TextureFilter::Linear => "linear",
        TextureFilter::NearestMipmapNearest => "nearest-mipmap-nearest",
        TextureFilter::LinearMipmapNearest => "linear-mipmap-nearest",
        TextureFilter::NearestMipmapLinear => "nearest-mipmap-linear",
        TextureFilter::LinearMipmapLinear => "linear-mipmap-linear",
    })
}

fn encode_state(value: &RenderState) -> WireValue {
    WireValue::object(vec![
        (
            "blend",
            WireValue::object(vec![
                ("source", encode_blend(value.blend.0)),
                ("destination", encode_blend(value.blend.1)),
            ]),
        ),
        (
            "depthTest",
            WireValue::text(match value.depth_test {
                DepthTest::LessEqual => "less-equal",
                DepthTest::Equal => "equal",
                DepthTest::Always => "always",
            }),
        ),
        ("depthWrite", WireValue::Bool(value.depth_write)),
        (
            "alphaTest",
            WireValue::text(match value.alpha_test {
                AlphaTest::None => "none",
                AlphaTest::GreaterZero => "gt0",
                AlphaTest::Less128 => "lt128",
                AlphaTest::GreaterEqual128 => "ge128",
            }),
        ),
        ("cull", encode_cull(value.cull)),
        ("depthRange", encode_pair(value.depth_range)),
        ("polygonOffset", encode_offset(value.polygon_offset)),
    ])
}

fn encode_vertex(value: &RenderVertex) -> WireValue {
    WireValue::object(vec![
        ("position", encode_vec4(value.position)),
        ("texCoord", encode_vec2(value.tex_coord)),
        ("color", encode_vec4(value.color)),
    ])
}

fn encode_fog(value: &BatchFog) -> WireValue {
    match value {
        BatchFog::Constant { color, amount } => WireValue::object(vec![
            ("kind", WireValue::text("constant")),
            ("color", encode_vec3(*color)),
            ("amount", WireValue::Float(*amount)),
        ]),
        BatchFog::Exp2 { color, density, effect } => WireValue::object(vec![
            ("kind", WireValue::text("exp2")),
            ("color", encode_vec3(*color)),
            ("density", WireValue::Float(*density)),
            (
                "effect",
                WireValue::text(match effect {
                    FogEffect::Color => "color",
                    FogEffect::None => "none",
                    FogEffect::Rgb => "rgb",
                    FogEffect::Alpha => "alpha",
                    FogEffect::Rgba => "rgba",
                    FogEffect::Overlay => "overlay",
                }),
            ),
        ]),
    }
}

fn encode_shadow(value: &Q2ShadowProjection) -> WireValue {
    match value {
        Q2ShadowProjection::None => WireValue::object(vec![("kind", WireValue::text("none"))]),
        Q2ShadowProjection::Point { atlas_rect } => WireValue::object(vec![
            ("kind", WireValue::text("point")),
            ("atlasRect", encode_vec4(*atlas_rect)),
        ]),
        Q2ShadowProjection::Cone { matrix, atlas_rect } => WireValue::object(vec![
            ("kind", WireValue::text("cone")),
            ("atlasRect", encode_vec4(*atlas_rect)),
            ("matrix", encode_matrix(matrix)),
        ]),
    }
}

fn encode_light(value: &Q2FragmentLight) -> WireValue {
    WireValue::object(vec![
        ("origin", encode_vec3(value.origin)),
        ("radius", WireValue::Float(value.radius)),
        ("color", encode_vec3(value.color)),
        ("scale", WireValue::Float(value.scale)),
        (
            "cone",
            match value.cone {
                None => WireValue::Null,
                Some(cone) => WireValue::object(vec![
                    ("direction", encode_vec3(cone.direction)),
                    ("cosHalfAngle", WireValue::Float(cone.cos_half_angle)),
                ]),
            },
        ),
        ("shadow", encode_shadow(&value.shadow)),
    ])
}

fn encode_camera(value: &RenderCamera) -> WireValue {
    WireValue::object(vec![
        ("origin", encode_vec3(value.origin)),
        (
            "axis",
            WireValue::list(value.axis.iter().map(|axis| encode_vec3(*axis)).collect()),
        ),
        ("projection", encode_matrix(&value.projection)),
        ("viewport", encode_rect(value.viewport)),
        (
            "clip",
            match value.clip {
                ViewClip::None => WireValue::object(vec![("kind", WireValue::text("none"))]),
                ViewClip::Portal { plane, mirror } => WireValue::object(vec![
                    ("kind", WireValue::text("portal")),
                    ("mirror", WireValue::Bool(mirror)),
                    (
                        "plane",
                        WireValue::object(vec![
                            ("normal", encode_vec3(plane.normal)),
                            ("distance", WireValue::Float(plane.distance)),
                        ]),
                    ),
                ]),
            },
        ),
    ])
}

fn encode_level_content(value: &LevelContent) -> WireValue {
    match value {
        LevelContent::Rgba(level) => WireValue::object(vec![
            ("width", WireValue::Int(i64::from(level.width))),
            ("height", WireValue::Int(i64::from(level.height))),
            ("pixels", WireValue::Bytes(level.pixels.clone())),
        ]),
        LevelContent::Depth(level) => WireValue::object(vec![
            ("width", WireValue::Int(i64::from(level.width))),
            ("height", WireValue::Int(i64::from(level.height))),
            ("pixels", WireValue::Floats(level.pixels.clone())),
        ]),
    }
}

fn encode_levels(value: &RenderImage) -> WireValue {
    match value {
        RenderImage::Indexed8 { levels, .. } | RenderImage::Rgba8 { levels, .. } => WireValue::list(
            levels
                .iter()
                .map(|level| {
                    WireValue::object(vec![
                        ("width", WireValue::Int(i64::from(level.width))),
                        ("height", WireValue::Int(i64::from(level.height))),
                        ("pixels", WireValue::Bytes(level.pixels.clone())),
                    ])
                })
                .collect(),
        ),
        RenderImage::Depth32f { levels } => WireValue::list(
            levels
                .iter()
                .map(|level| {
                    WireValue::object(vec![
                        ("width", WireValue::Int(i64::from(level.width))),
                        ("height", WireValue::Int(i64::from(level.height))),
                        ("pixels", WireValue::Floats(level.pixels.clone())),
                    ])
                })
                .collect(),
        ),
    }
}

fn encode_content(value: &RenderImage) -> WireValue {
    match value {
        RenderImage::Indexed8 {
            palette,
            transparency,
            fullbright,
            translation,
            ..
        } => WireValue::object(vec![
            ("kind", WireValue::text("indexed8")),
            ("levels", encode_levels(value)),
            (
                "palette",
                WireValue::object(vec![("colors", WireValue::Bytes(palette.colors.clone()))]),
            ),
            (
                "transparency",
                match transparency {
                    PaletteTransparency::Opaque => WireValue::object(vec![("kind", WireValue::text("opaque"))]),
                    PaletteTransparency::Index(index) => WireValue::object(vec![
                        ("kind", WireValue::text("index")),
                        ("index", WireValue::Int(i64::from(*index))),
                    ]),
                    PaletteTransparency::Q1Fence => WireValue::object(vec![
                        ("kind", WireValue::text("q1-fence")),
                        ("index", WireValue::Int(255)),
                    ]),
                },
            ),
            (
                "fullbright",
                match fullbright {
                    None => WireValue::Null,
                    Some((first, last)) => WireValue::object(vec![
                        ("first", WireValue::Int(i64::from(*first))),
                        ("last", WireValue::Int(i64::from(*last))),
                    ]),
                },
            ),
            (
                "translation",
                match translation {
                    None => WireValue::Null,
                    Some(table) => WireValue::Bytes(table.clone()),
                },
            ),
        ]),
        RenderImage::Rgba8 { border_color, .. } => WireValue::object(vec![
            ("kind", WireValue::text("rgba8")),
            ("levels", encode_levels(value)),
            ("borderColor", encode_vec4(*border_color)),
        ]),
        RenderImage::Depth32f { .. } => WireValue::object(vec![
            ("kind", WireValue::text("depth32f")),
            ("levels", encode_levels(value)),
        ]),
    }
}

fn encode_sampling(value: &TextureSampling) -> WireValue {
    WireValue::object(vec![
        ("wrap", WireValue::text(if value.repeat { "repeat" } else { "clamp" })),
        ("filter", encode_filter(value.filter)),
    ])
}

/// Snapshot values without invoking dynamic sources or serializing frontend identities.
pub struct WireEncoder {
    owner: ResourceOwner,
    images: HashMap<u32, RendererImage>,
    tokens: HashMap<usize, u32>,
    sources: HashMap<u32, Arc<dyn DynamicImageSource>>,
    next_token: u32,
    next_acknowledgment: u32,
    acknowledgments: HashMap<u32, ImageResourceOperation>,
}

impl WireEncoder {
    /// Bind the encoder to one renderer lifetime.
    #[must_use]
    pub fn new(owner: ResourceOwner) -> Self {
        Self {
            owner,
            images: HashMap::new(),
            tokens: HashMap::new(),
            sources: HashMap::new(),
            next_token: 1,
            next_acknowledgment: 1,
            acknowledgments: HashMap::new(),
        }
    }

    /// Image previously encoded under `ordinal`.
    pub fn original_image(&self, ordinal: u32) -> Result<RendererImage, RenderError> {
        self.images
            .get(&ordinal)
            .cloned()
            .ok_or(RenderError::UnknownImage(ordinal))
    }

    /// Dynamic source previously encoded under `token`.
    pub fn source(&self, token: u32) -> Result<Arc<dyn DynamicImageSource>, RenderError> {
        self.sources
            .get(&token)
            .cloned()
            .ok_or_else(|| bad_wire("unknown renderer dynamic token"))
    }

    /// Forget dynamic tokens. The caller must synchronize and retain tokens
    /// while direct prepared draws remain active.
    pub fn clear_sources(&mut self) {
        self.tokens.clear();
        self.sources.clear();
    }

    /// Forget images, sources, and acknowledgments.
    pub fn clear(&mut self) {
        self.images.clear();
        self.clear_sources();
        self.acknowledgments.clear();
    }

    /// Take the retained operation for an acknowledgment id.
    pub fn acknowledge(&mut self, token: &WireValue) -> Result<ImageResourceOperation, RenderError> {
        let id = integer(token)?;
        self.acknowledgments
            .remove(&id)
            .ok_or_else(|| bad_wire("unknown renderer image acknowledgment"))
    }

    /// Encode an image reference, registering its ordinal identity.
    pub fn image(&mut self, image: &RendererImage) -> Result<WireValue, RenderError> {
        self.owner.require(&image.owner, "image")?;
        if let Some(previous) = self.images.get(&image.ordinal) {
            if previous != image {
                return Err(RenderError::ImageConflict(image.ordinal));
            }
        } else {
            self.images.insert(image.ordinal, image.clone());
        }
        Ok(WireValue::object(vec![
            ("ordinal", WireValue::Int(i64::from(image.ordinal))),
            ("width", WireValue::Int(i64::from(image.width))),
            ("height", WireValue::Int(i64::from(image.height))),
            ("name", WireValue::Str(image.source.display_name().to_string())),
        ]))
    }

    /// Encode a texture binding, minting a token for dynamic sources.
    pub fn binding(&mut self, binding: &TextureBinding) -> Result<WireValue, RenderError> {
        match binding {
            TextureBinding::BindImage(image) => Ok(WireValue::object(vec![
                ("kind", WireValue::text("bind-image")),
                ("image", self.image(image)?),
            ])),
            TextureBinding::RetainCurrentTexture => Ok(WireValue::object(vec![(
                "kind",
                WireValue::text("retain-current-texture"),
            )])),
            TextureBinding::DynamicImage(source) => {
                let key = Arc::as_ptr(source) as *const () as usize;
                let token = match self.tokens.get(&key) {
                    Some(token) => *token,
                    None => {
                        let token = self.next_token;
                        self.next_token += 1;
                        self.tokens.insert(key, token);
                        self.sources.insert(token, Arc::clone(source));
                        token
                    }
                };
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("dynamic-image")),
                    ("token", WireValue::Int(i64::from(token))),
                ]))
            }
        }
    }

    /// Encode an image operation, retaining a clone under its acknowledgment.
    pub fn image_operation(&mut self, operation: &ImageResourceOperation) -> Result<WireValue, RenderError> {
        let acknowledgment = self.next_acknowledgment;
        self.next_acknowledgment += 1;
        self.acknowledgments.insert(acknowledgment, operation.clone());
        let ack = WireValue::Int(i64::from(acknowledgment));
        match operation {
            ImageResourceOperation::TextureMode { filter } => Ok(WireValue::object(vec![
                ("kind", WireValue::text("texture-mode")),
                ("filter", encode_filter(*filter)),
                ("acknowledgment", ack),
            ])),
            ImageResourceOperation::ReleaseImage { image } => {
                let encoded = self.image(image)?;
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("release-image")),
                    ("image", encoded),
                    ("acknowledgment", ack),
                ]))
            }
            ImageResourceOperation::UpdateImage { image, level, content } => {
                let encoded = self.image(image)?;
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("update-image")),
                    ("image", encoded),
                    ("level", WireValue::Int(i64::from(*level))),
                    ("content", encode_level_content(content)),
                    ("acknowledgment", ack),
                ]))
            }
            ImageResourceOperation::CreateImage {
                image,
                content,
                sampling,
            } => {
                let encoded = self.image(image)?;
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("create-image")),
                    ("image", encoded),
                    ("content", encode_content(content)),
                    ("sampling", encode_sampling(sampling)),
                    ("acknowledgment", ack),
                ]))
            }
        }
    }

    fn atlas(&mut self, atlas: &Q2ShadowAtlas) -> Result<WireValue, RenderError> {
        Ok(WireValue::object(vec![
            ("image", self.image(&atlas.image)?),
            ("texelSize", WireValue::Float(atlas.texel_size)),
            ("nearPlane", WireValue::Float(atlas.near_plane)),
        ]))
    }

    fn lighting(&mut self, lighting: &BatchLighting) -> Result<WireValue, RenderError> {
        match lighting {
            BatchLighting::Vertex => Ok(WireValue::object(vec![("kind", WireValue::text("vertex"))])),
            BatchLighting::Q2World {
                world_positions,
                normals,
                atlas,
                pass,
            } => {
                let mut entries = vec![
                    ("kind", WireValue::text("q2-world")),
                    (
                        "worldPositions",
                        WireValue::list(world_positions.iter().map(|pos| encode_vec3(*pos)).collect()),
                    ),
                    (
                        "normals",
                        WireValue::list(normals.iter().map(|normal| encode_vec3(*normal)).collect()),
                    ),
                    (
                        "atlas",
                        match atlas {
                            None => WireValue::Null,
                            Some(atlas) => self.atlas(atlas)?,
                        },
                    ),
                ];
                match pass {
                    Q2LightPass::Lightmap { lights }
                    | Q2LightPass::Texture { lights }
                    | Q2LightPass::MaterialLightmap { lights } => {
                        let name = match pass {
                            Q2LightPass::Lightmap { .. } => "lightmap",
                            Q2LightPass::Texture { .. } => "texture",
                            _ => "material-lightmap",
                        };
                        entries.push(("pass", WireValue::text(name)));
                        entries.push(("lights", WireValue::list(lights.iter().map(encode_light).collect())));
                    }
                    Q2LightPass::Model { lights, shade_scale } => {
                        entries.push(("pass", WireValue::text("model")));
                        entries.push((
                            "lights",
                            WireValue::list(
                                lights
                                    .iter()
                                    .map(|entry| {
                                        let mut pairs = match encode_light(&entry.light) {
                                            WireValue::Map(pairs) => pairs,
                                            _ => Vec::new(),
                                        };
                                        pairs.push(("fraction".to_string(), encode_vec3(entry.fraction)));
                                        WireValue::Map(pairs)
                                    })
                                    .collect(),
                            ),
                        ));
                        entries.push((
                            "shadeScale",
                            match shade_scale {
                                None => WireValue::Null,
                                Some(scale) => WireValue::Float(*scale),
                            },
                        ));
                    }
                }
                Ok(WireValue::Map(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.to_string(), value))
                        .collect(),
                ))
            }
            BatchLighting::Q2ModelShadow {
                world_positions,
                lights,
                shade_scale,
                atlas,
            } => {
                let encoded_atlas = self.atlas(atlas)?;
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("q2-model-shadow")),
                    (
                        "worldPositions",
                        WireValue::list(world_positions.iter().map(|pos| encode_vec3(*pos)).collect()),
                    ),
                    (
                        "lights",
                        WireValue::list(
                            lights
                                .iter()
                                .map(|light| {
                                    WireValue::object(vec![
                                        ("origin", encode_vec3(light.origin)),
                                        ("radius", WireValue::Float(light.radius)),
                                        ("fraction", encode_vec3(light.fraction)),
                                        ("shadow", encode_shadow(&light.shadow)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    ("shadeScale", WireValue::Float(*shade_scale)),
                    ("atlas", encoded_atlas),
                ]))
            }
        }
    }

    /// Encode one draw batch.
    pub fn batch(&mut self, batch: &DrawBatch) -> Result<WireValue, RenderError> {
        let lighting = self.lighting(&batch.lighting)?;
        let texture = self.binding(&batch.texture)?;
        let mut entries: Vec<(String, WireValue)> = Vec::new();
        match batch.primitive {
            BatchPrimitive::Triangles => {
                entries.push(("primitive".to_string(), WireValue::text("triangles")));
            }
            BatchPrimitive::Lines { line_width } => {
                entries.push(("primitive".to_string(), WireValue::text("lines")));
                entries.push(("lineWidth".to_string(), WireValue::Float(line_width)));
            }
        }
        entries.push((
            "indices".to_string(),
            WireValue::list(
                batch
                    .indices
                    .iter()
                    .map(|index| WireValue::Int(i64::from(*index)))
                    .collect(),
            ),
        ));
        entries.push(("texture".to_string(), texture));
        entries.push(("state".to_string(), encode_state(&batch.state)));
        entries.push(("lighting".to_string(), lighting));
        if let Some(fog) = &batch.fog {
            entries.push(("fog".to_string(), encode_fog(fog)));
        }
        if batch.luminance_alpha {
            entries.push(("textureEffect".to_string(), WireValue::text("luminance-alpha")));
        }
        match &batch.vertices {
            BatchVertices::Single(vertices) => {
                entries.push(("texturing".to_string(), WireValue::text("single")));
                entries.push((
                    "vertices".to_string(),
                    WireValue::list(vertices.iter().map(encode_vertex).collect()),
                ));
            }
            BatchVertices::Pair {
                vertices,
                second_texture,
            } => {
                entries.push(("texturing".to_string(), WireValue::text("pair")));
                entries.push((
                    "vertices".to_string(),
                    WireValue::list(
                        vertices
                            .iter()
                            .map(|vertex| {
                                WireValue::Map(vec![
                                    ("position".to_string(), encode_vec4(vertex.base.position)),
                                    ("texCoord".to_string(), encode_vec2(vertex.base.tex_coord)),
                                    ("color".to_string(), encode_vec4(vertex.base.color)),
                                    ("texCoord2".to_string(), encode_vec2(vertex.tex_coord2)),
                                ])
                            })
                            .collect(),
                    ),
                ));
                let binding = self.binding(&second_texture.binding)?;
                entries.push((
                    "secondTexture".to_string(),
                    WireValue::object(vec![
                        (
                            "environment",
                            WireValue::text(match second_texture.environment {
                                PairEnvironment::Modulate => "modulate",
                                PairEnvironment::Add => "add",
                                PairEnvironment::Replace => "replace",
                            }),
                        ),
                        ("binding", binding),
                    ]),
                ));
            }
        }
        Ok(WireValue::Map(entries))
    }

    fn depth_pass(&mut self, pass: &DepthAtlasPass) -> Result<WireValue, RenderError> {
        Ok(WireValue::object(vec![
            ("viewport", encode_rect(pass.viewport)),
            (
                "clearDepth",
                match pass.clear_depth {
                    None => WireValue::Null,
                    Some(depth) => WireValue::Float(depth),
                },
            ),
            (
                "draws",
                WireValue::list(
                    pass.draws
                        .iter()
                        .map(|draw| {
                            WireValue::object(vec![
                                (
                                    "positions",
                                    WireValue::list(draw.positions.iter().map(|pos| encode_vec4(*pos)).collect()),
                                ),
                                (
                                    "indices",
                                    WireValue::list(
                                        draw.indices
                                            .iter()
                                            .map(|index| WireValue::Int(i64::from(*index)))
                                            .collect(),
                                    ),
                                ),
                                ("cull", encode_cull(draw.cull)),
                                ("polygonOffset", encode_offset(draw.polygon_offset)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]))
    }

    /// Encode one render operation.
    pub fn operation(&mut self, operation: &RenderOperation) -> Result<WireValue, RenderError> {
        match operation {
            RenderOperation::Draw(batches) => {
                let mut encoded = Vec::with_capacity(batches.len());
                for batch in batches {
                    encoded.push(self.batch(batch)?);
                }
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("draw")),
                    ("batches", WireValue::list(encoded)),
                ]))
            }
            RenderOperation::ObjectOpacity { opacity, batches } => {
                let mut encoded = Vec::with_capacity(batches.len());
                for batch in batches {
                    encoded.push(self.batch(batch)?);
                }
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("object-opacity")),
                    ("opacity", WireValue::Float(*opacity)),
                    ("batches", WireValue::list(encoded)),
                ]))
            }
            RenderOperation::DepthAtlas { image, passes } => {
                let encoded_image = self.image(image)?;
                let mut encoded = Vec::with_capacity(passes.len());
                for pass in passes {
                    encoded.push(self.depth_pass(pass)?);
                }
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("depth-atlas")),
                    ("image", encoded_image),
                    ("passes", WireValue::list(encoded)),
                ]))
            }
            RenderOperation::SkySide { image, color, strips } => {
                let encoded_image = self.image(image)?;
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("sky-side")),
                    ("image", encoded_image),
                    ("color", encode_vec4(*color)),
                    (
                        "strips",
                        WireValue::list(
                            strips
                                .iter()
                                .map(|strip| {
                                    WireValue::list(
                                        strip
                                            .iter()
                                            .map(|vertex| {
                                                WireValue::object(vec![
                                                    ("position", encode_vec4(vertex.position)),
                                                    ("texCoord", encode_vec2(vertex.tex_coord)),
                                                ])
                                            })
                                            .collect(),
                                    )
                                })
                                .collect(),
                        ),
                    ),
                ]))
            }
            RenderOperation::ShadowVolume {
                positions,
                indices,
                mirror,
                white_image,
            } => {
                let encoded = self.image(white_image)?;
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("shadow-volume")),
                    (
                        "positions",
                        WireValue::list(positions.iter().map(|pos| encode_vec4(*pos)).collect()),
                    ),
                    (
                        "indices",
                        WireValue::list(indices.iter().map(|index| WireValue::Int(i64::from(*index))).collect()),
                    ),
                    ("mirror", WireValue::Bool(*mirror)),
                    ("whiteImage", encoded),
                ]))
            }
            RenderOperation::ShadowFinish { positions, white_image } => {
                let encoded = self.image(white_image)?;
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("shadow-finish")),
                    (
                        "positions",
                        WireValue::list(positions.iter().map(|pos| encode_vec4(*pos)).collect()),
                    ),
                    ("whiteImage", encoded),
                ]))
            }
            RenderOperation::Q2Fog(operation) => Ok(WireValue::object(vec![
                ("kind", WireValue::text("q2-fog")),
                ("camera", encode_camera(&operation.camera)),
                ("farDepth", WireValue::Float(operation.far_depth)),
                ("skyDrawn", WireValue::Bool(operation.sky_drawn)),
                ("fog", encode_q2_fog(&operation.fog)),
            ])),
            RenderOperation::DepthRange(range) => Ok(WireValue::object(vec![
                ("kind", WireValue::text("depth-range")),
                ("range", encode_pair(*range)),
            ])),
            RenderOperation::Cull(face) => Ok(WireValue::object(vec![
                ("kind", WireValue::text("cull")),
                ("cull", encode_cull(*face)),
            ])),
            RenderOperation::PolygonOffset(value) => Ok(WireValue::object(vec![
                ("kind", WireValue::text("polygon-offset")),
                ("value", encode_offset(*value)),
            ])),
            RenderOperation::DisablePortalClip => Ok(WireValue::object(vec![(
                "kind",
                WireValue::text("disable-portal-clip"),
            )])),
            RenderOperation::RetainedDraw(draw) => {
                let mut batches = Vec::with_capacity(draw.batches.len());
                for batch in &draw.batches {
                    batches.push(self.retained_batch(batch)?);
                }
                let passes = draw
                    .surface
                    .passes
                    .iter()
                    .map(|pass| {
                        WireValue::object(vec![
                            (
                                "texCoords",
                                WireValue::list(pass.tex_coords.iter().map(|uv| encode_vec2(*uv)).collect()),
                            ),
                            (
                                "texCoords2",
                                WireValue::list(pass.tex_coords2.iter().map(|uv| encode_vec2(*uv)).collect()),
                            ),
                            (
                                "colors",
                                WireValue::list(pass.colors.iter().map(|color| encode_vec4(*color)).collect()),
                            ),
                        ])
                    })
                    .collect();
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("retained-draw")),
                    ("surface", WireValue::Int(i64::from(draw.surface.id.surface))),
                    ("generation", WireValue::Int(draw.surface.id.generation as i64)),
                    (
                        "eye",
                        WireValue::list(draw.eye.iter().map(|row| encode_vec4(*row)).collect()),
                    ),
                    ("projection", encode_matrix(&draw.projection)),
                    (
                        "positions",
                        WireValue::list(draw.surface.positions.iter().map(|point| encode_vec3(*point)).collect()),
                    ),
                    (
                        "indices",
                        WireValue::list(
                            draw.surface
                                .indices
                                .iter()
                                .map(|index| WireValue::Int(i64::from(*index)))
                                .collect(),
                        ),
                    ),
                    ("passes", WireValue::list(passes)),
                    ("batches", WireValue::list(batches)),
                ]))
            }
        }
    }

    /// Encode one retained batch's per-frame parameters.
    fn retained_batch(&mut self, batch: &RetainedBatch) -> Result<WireValue, RenderError> {
        let lighting = self.lighting(&batch.lighting)?;
        let texture = self.binding(&batch.texture)?;
        let mut entries: Vec<(String, WireValue)> = Vec::new();
        match batch.primitive {
            BatchPrimitive::Triangles => {
                entries.push(("primitive".to_string(), WireValue::text("triangles")));
            }
            BatchPrimitive::Lines { line_width } => {
                entries.push(("primitive".to_string(), WireValue::text("lines")));
                entries.push(("lineWidth".to_string(), WireValue::Float(line_width)));
            }
        }
        entries.push((
            "range".to_string(),
            WireValue::object(vec![
                ("start", WireValue::Int(i64::from(batch.range.start))),
                ("count", WireValue::Int(i64::from(batch.range.count))),
            ]),
        ));
        entries.push(("pass".to_string(), WireValue::Int(i64::from(batch.pass))));
        entries.push(("texture".to_string(), texture));
        if let Some(second) = &batch.second_texture {
            let binding = self.binding(&second.binding)?;
            entries.push((
                "secondTexture".to_string(),
                WireValue::object(vec![
                    (
                        "environment",
                        WireValue::text(match second.environment {
                            PairEnvironment::Modulate => "modulate",
                            PairEnvironment::Add => "add",
                            PairEnvironment::Replace => "replace",
                        }),
                    ),
                    ("binding", binding),
                ]),
            ));
        }
        entries.push(("state".to_string(), encode_state(&batch.state)));
        entries.push(("lighting".to_string(), lighting));
        if let Some(fog) = &batch.fog {
            entries.push(("fog".to_string(), encode_fog(fog)));
        }
        if batch.luminance_alpha {
            entries.push(("textureEffect".to_string(), WireValue::text("luminance-alpha")));
        }
        Ok(WireValue::Map(entries))
    }

    /// Encode one render command. `captures` rides with swap-buffers.
    pub fn command(&mut self, command: &RenderCommand, captures: &[u32]) -> Result<WireValue, RenderError> {
        match command {
            RenderCommand::View(view) => {
                let mut before = Vec::with_capacity(view.before_view.len());
                for operation in &view.before_view {
                    before.push(self.operation(operation)?);
                }
                let mut operations = Vec::with_capacity(view.operations.len());
                for operation in &view.operations {
                    operations.push(self.operation(operation)?);
                }
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("view")),
                    (
                        "view",
                        WireValue::object(vec![
                            ("viewport", encode_rect(view.state.viewport)),
                            ("clear", encode_clear(view.state.clear)),
                            (
                                "clipPlane",
                                match view.state.clip_plane {
                                    None => WireValue::Null,
                                    Some(plane) => encode_vec4(plane),
                                },
                            ),
                            ("beforeView", WireValue::list(before)),
                            ("operations", WireValue::list(operations)),
                        ]),
                    ),
                ]))
            }
            RenderCommand::ImageResource(operation) => Ok(WireValue::object(vec![
                ("kind", WireValue::text("image-resource")),
                ("operation", self.image_operation(operation)?),
            ])),
            RenderCommand::StretchPic { rect, uv, image } => {
                let encoded = self.image(image)?;
                Ok(WireValue::object(vec![
                    ("kind", WireValue::text("stretch-pic")),
                    ("rect", encode_rect(*rect)),
                    (
                        "uv",
                        WireValue::object(vec![
                            ("s1", WireValue::Float(uv.s1)),
                            ("t1", WireValue::Float(uv.t1)),
                            ("s2", WireValue::Float(uv.s2)),
                            ("t2", WireValue::Float(uv.t2)),
                        ]),
                    ),
                    ("image", encoded),
                ]))
            }
            RenderCommand::SwapBuffers => Ok(WireValue::object(vec![
                ("kind", WireValue::text("swap-buffers")),
                (
                    "captures",
                    WireValue::list(
                        captures
                            .iter()
                            .map(|capture| WireValue::Int(i64::from(*capture)))
                            .collect(),
                    ),
                ),
            ])),
            RenderCommand::DrawBuffer { buffer, clear } => Ok(WireValue::object(vec![
                ("kind", WireValue::text("draw-buffer")),
                (
                    "buffer",
                    WireValue::text(match buffer {
                        DrawBuffer::Front => "front",
                        DrawBuffer::Back => "back",
                        DrawBuffer::BackLeft => "back-left",
                        DrawBuffer::BackRight => "back-right",
                    }),
                ),
                ("clear", WireValue::Bool(*clear)),
            ])),
            RenderCommand::SetColor(color) => Ok(WireValue::object(vec![
                ("kind", WireValue::text("set-color")),
                ("color", encode_vec4(*color)),
            ])),
        }
    }
}

fn encode_clear(value: Option<ViewClear>) -> WireValue {
    match value {
        None => WireValue::Null,
        Some(clear) => WireValue::object(vec![
            ("depth", WireValue::Float(clear.depth)),
            (
                "color",
                match clear.color {
                    None => WireValue::Null,
                    Some(color) => encode_vec4(color),
                },
            ),
            ("stencil", WireValue::Bool(clear.stencil)),
        ]),
    }
}

fn encode_q2_fog(value: &Q2Fog) -> WireValue {
    let stop = |stop: &Q2HeightStop| {
        WireValue::object(vec![
            ("color", encode_vec3(stop.color)),
            ("distance", WireValue::Float(stop.distance)),
        ])
    };
    WireValue::object(vec![
        ("kind", WireValue::text("q2")),
        ("color", encode_vec3(value.color)),
        ("density", WireValue::Float(value.density)),
        ("skyFactor", WireValue::Float(value.sky_factor)),
        (
            "height",
            WireValue::object(vec![
                ("start", stop(&value.height.start)),
                ("end", stop(&value.height.end)),
                ("density", WireValue::Float(value.height.density)),
                ("falloff", WireValue::Float(value.height.falloff)),
            ]),
        ),
    ])
}

/// Callback invoked when a decoded dynamic binding resolves: uploads through
/// `apply`, then returns the image to bind.
pub type ReachedDynamicCallback =
    Arc<Mutex<Box<dyn FnMut(u32, &mut dyn FnMut(ImageResourceOperation)) -> RendererImage + Send>>>;

struct TokenSource {
    token: u32,
    reached: ReachedDynamicCallback,
}

impl DynamicImageSource for TokenSource {
    fn resolve(&self, apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage {
        let mut reached = self.reached.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        reached(self.token, apply)
    }
}

/// Reconstruct canonical receiver identities from wire snapshots.
pub struct WireDecoder {
    owner: ResourceOwner,
    images: HashMap<u32, RendererImage>,
    sources: HashMap<u32, Arc<dyn DynamicImageSource>>,
    acknowledgments: Vec<(ImageResourceOperation, u32)>,
    reached: ReachedDynamicCallback,
    palette_source: String,
}

impl WireDecoder {
    /// Bind the decoder to the receiver lifetime. `palette_source` fills the
    /// palette origin the wire strips from indexed uploads; `reached_dynamic`
    /// runs when a decoded dynamic binding resolves.
    pub fn new(
        owner: ResourceOwner,
        palette_source: String,
        reached_dynamic: impl FnMut(u32, &mut dyn FnMut(ImageResourceOperation)) -> RendererImage + Send + 'static,
    ) -> Self {
        Self {
            owner,
            images: HashMap::new(),
            sources: HashMap::new(),
            acknowledgments: Vec::new(),
            reached: Arc::new(Mutex::new(Box::new(reached_dynamic))),
            palette_source,
        }
    }

    /// Forget dynamic sources. The caller must finish submitted work and all
    /// direct prepared draws before clearing.
    pub fn clear_sources(&mut self) {
        self.sources.clear();
    }

    /// Decode an image reference, minting one canonical identity per ordinal.
    pub fn image(&mut self, value: &WireValue) -> Result<RendererImage, RenderError> {
        let map = record(value)?;
        let ordinal = integer(field(map, "ordinal")?)?;
        let width = integer(field(map, "width")?)?;
        let height = integer(field(map, "height")?)?;
        let name = string(field(map, "name")?)?.to_string();
        if let Some(existing) = self.images.get(&ordinal) {
            let matches = existing.width == width
                && existing.height == height
                && matches!(&existing.source, ImageSource::Generated { name: known } if *known == name);
            if !matches {
                return Err(bad_wire("renderer image metadata changed"));
            }
            return Ok(existing.clone());
        }
        let image = RendererImage {
            owner: self.owner.clone(),
            ordinal,
            width,
            height,
            source: ImageSource::Generated { name },
        };
        self.images.insert(ordinal, image.clone());
        Ok(image)
    }

    /// Decode a texture binding.
    pub fn binding(&mut self, value: &WireValue) -> Result<TextureBinding, RenderError> {
        let map = record(value)?;
        match string(field(map, "kind")?)? {
            "bind-image" => Ok(TextureBinding::BindImage(self.image(field(map, "image")?)?)),
            "retain-current-texture" => Ok(TextureBinding::RetainCurrentTexture),
            "dynamic-image" => {
                let token = integer(field(map, "token")?)?;
                if let Some(source) = self.sources.get(&token) {
                    return Ok(TextureBinding::DynamicImage(Arc::clone(source)));
                }
                let source: Arc<dyn DynamicImageSource> = Arc::new(TokenSource {
                    token,
                    reached: Arc::clone(&self.reached),
                });
                self.sources.insert(token, Arc::clone(&source));
                Ok(TextureBinding::DynamicImage(source))
            }
            _ => Err(bad_wire("invalid renderer texture binding")),
        }
    }

    fn content(&self, value: &WireValue) -> Result<RenderImage, RenderError> {
        let map = record(value)?;
        let levels = list(field(map, "levels")?)?;
        if string(field(map, "kind")?)? == "depth32f" {
            let mut decoded = Vec::with_capacity(levels.len());
            for entry in levels {
                match level(entry)? {
                    LevelContent::Depth(depth) => decoded.push(depth),
                    LevelContent::Rgba(_) => return Err(bad_wire("invalid depth pixels")),
                }
            }
            if decoded.is_empty() {
                return Err(bad_wire("image has no levels"));
            }
            return Ok(RenderImage::Depth32f { levels: decoded });
        }
        let mut decoded = Vec::with_capacity(levels.len());
        for entry in levels {
            match level(entry)? {
                LevelContent::Rgba(color) => decoded.push(color),
                LevelContent::Depth(_) => return Err(bad_wire("invalid color pixels")),
            }
        }
        if decoded.is_empty() {
            return Err(bad_wire("image has no levels"));
        }
        match string(field(map, "kind")?)? {
            "rgba8" => Ok(RenderImage::Rgba8 {
                levels: decoded,
                border_color: vec4(field(map, "borderColor")?)?,
            }),
            "indexed8" => {
                let palette_map = record(field(map, "palette")?)?;
                let colors = match field(palette_map, "colors")? {
                    WireValue::Bytes(colors) if colors.len() == 768 => colors.clone(),
                    _ => return Err(bad_wire("invalid indexed palette")),
                };
                let translation = match field(map, "translation")? {
                    WireValue::Null => None,
                    WireValue::Bytes(table) => Some(table.clone()),
                    _ => return Err(bad_wire("invalid indexed palette")),
                };
                let fullbright_raw = field(map, "fullbright")?;
                let fullbright = if matches!(fullbright_raw, WireValue::Null) {
                    None
                } else {
                    let bright = record(fullbright_raw)?;
                    Some((byte(field(bright, "first")?)?, byte(field(bright, "last")?)?))
                };
                let transparency_map = record(field(map, "transparency")?)?;
                let transparency = match string(field(transparency_map, "kind")?)? {
                    "opaque" => PaletteTransparency::Opaque,
                    "index" => PaletteTransparency::Index(byte(field(transparency_map, "index")?)?),
                    "q1-fence" => {
                        if integer(field(transparency_map, "index")?)? != 255 {
                            return Err(bad_wire("invalid palette transparency"));
                        }
                        PaletteTransparency::Q1Fence
                    }
                    _ => return Err(bad_wire("invalid palette transparency")),
                };
                Ok(RenderImage::Indexed8 {
                    levels: decoded,
                    palette: Palette {
                        colors,
                        source: self.palette_source.clone(),
                    },
                    transparency,
                    fullbright,
                    translation,
                })
            }
            _ => Err(bad_wire("invalid renderer image encoding")),
        }
    }

    /// Acknowledgment id recorded when `operation` was decoded.
    pub fn acknowledgment(&self, operation: &ImageResourceOperation) -> Result<u32, RenderError> {
        self.acknowledgments
            .iter()
            .rev()
            .find(|(known, _)| known == operation)
            .map(|(_, token)| *token)
            .ok_or_else(|| bad_wire("renderer image operation has no acknowledgment"))
    }

    /// Decode an image operation, recording its acknowledgment.
    pub fn image_operation(&mut self, value: &WireValue) -> Result<ImageResourceOperation, RenderError> {
        let map = record(value)?;
        let token = integer(field(map, "acknowledgment")?)?;
        let operation = match string(field(map, "kind")?)? {
            "texture-mode" => ImageResourceOperation::TextureMode {
                filter: filter(field(map, "filter")?)?,
            },
            "release-image" => ImageResourceOperation::ReleaseImage {
                image: self.image(field(map, "image")?)?,
            },
            "update-image" => ImageResourceOperation::UpdateImage {
                image: self.image(field(map, "image")?)?,
                level: integer(field(map, "level")?)?,
                content: level(field(map, "content")?)?,
            },
            "create-image" => {
                let sampling_map = record(field(map, "sampling")?)?;
                ImageResourceOperation::CreateImage {
                    image: self.image(field(map, "image")?)?,
                    content: self.content(field(map, "content")?)?,
                    sampling: TextureSampling {
                        repeat: choice(field(sampling_map, "wrap")?, &["repeat", "clamp"])? == "repeat",
                        filter: filter(field(sampling_map, "filter")?)?,
                    },
                }
            }
            _ => return Err(bad_wire("invalid renderer resource operation")),
        };
        self.acknowledgments.push((operation.clone(), token));
        Ok(operation)
    }

    fn atlas(&mut self, value: &WireValue) -> Result<Q2ShadowAtlas, RenderError> {
        let map = record(value)?;
        Ok(Q2ShadowAtlas {
            image: self.image(field(map, "image")?)?,
            texel_size: number(field(map, "texelSize")?)?,
            near_plane: number(field(map, "nearPlane")?)?,
        })
    }

    fn lighting(&mut self, value: &WireValue) -> Result<BatchLighting, RenderError> {
        let map = record(value)?;
        match string(field(map, "kind")?)? {
            "vertex" => Ok(BatchLighting::Vertex),
            "q2-model-shadow" => {
                let positions = list(field(map, "worldPositions")?)?;
                let mut world_positions = Vec::with_capacity(positions.len());
                for position in positions {
                    world_positions.push(vec3(position)?);
                }
                let raws = list(field(map, "lights")?)?;
                let mut lights = Vec::with_capacity(raws.len());
                for raw in raws {
                    let entry = record(raw)?;
                    lights.push(Q2ModelShadowLight {
                        origin: vec3(field(entry, "origin")?)?,
                        radius: number(field(entry, "radius")?)?,
                        fraction: vec3(field(entry, "fraction")?)?,
                        shadow: shadow(field(entry, "shadow")?)?,
                    });
                }
                Ok(BatchLighting::Q2ModelShadow {
                    world_positions,
                    lights,
                    shade_scale: number(field(map, "shadeScale")?)?,
                    atlas: self.atlas(field(map, "atlas")?)?,
                })
            }
            "q2-world" => {
                let positions = list(field(map, "worldPositions")?)?;
                let mut world_positions = Vec::with_capacity(positions.len());
                for position in positions {
                    world_positions.push(vec3(position)?);
                }
                let normals = list(field(map, "normals")?)?;
                let mut decoded_normals = Vec::with_capacity(normals.len());
                for normal in normals {
                    decoded_normals.push(vec3(normal)?);
                }
                let atlas_raw = field(map, "atlas")?;
                let atlas = if matches!(atlas_raw, WireValue::Null) {
                    None
                } else {
                    Some(self.atlas(atlas_raw)?)
                };
                let pass_raw = field(map, "pass")?;
                if string(pass_raw)? == "model" {
                    let raws = list(field(map, "lights")?)?;
                    let mut lights = Vec::with_capacity(raws.len());
                    for raw in raws {
                        lights.push(Q2ModelFragmentLight {
                            light: light(raw)?,
                            fraction: vec3(field(record(raw)?, "fraction")?)?,
                        });
                    }
                    let scale_raw = field(map, "shadeScale")?;
                    let shade_scale = if matches!(scale_raw, WireValue::Null) {
                        None
                    } else {
                        Some(number(scale_raw)?)
                    };
                    return Ok(BatchLighting::Q2World {
                        world_positions,
                        normals: decoded_normals,
                        atlas,
                        pass: Q2LightPass::Model { lights, shade_scale },
                    });
                }
                let raws = list(field(map, "lights")?)?;
                let mut lights = Vec::with_capacity(raws.len());
                for raw in raws {
                    lights.push(light(raw)?);
                }
                let pass = match choice(pass_raw, &["lightmap", "texture", "material-lightmap"])? {
                    "lightmap" => Q2LightPass::Lightmap { lights },
                    "texture" => Q2LightPass::Texture { lights },
                    _ => Q2LightPass::MaterialLightmap { lights },
                };
                Ok(BatchLighting::Q2World {
                    world_positions,
                    normals: decoded_normals,
                    atlas,
                    pass,
                })
            }
            _ => Err(bad_wire("invalid batch lighting")),
        }
    }

    /// Decode one draw batch.
    pub fn batch(&mut self, value: &WireValue) -> Result<DrawBatch, RenderError> {
        let map = record(value)?;
        let primitive_raw = field(map, "primitive")?;
        let primitive = if matches!(primitive_raw, WireValue::Str(text) if text == "lines") {
            BatchPrimitive::Lines {
                line_width: number(field(map, "lineWidth")?)?,
            }
        } else {
            choice(primitive_raw, &["triangles"])?;
            BatchPrimitive::Triangles
        };
        let raws = list(field(map, "indices")?)?;
        let mut indices = Vec::with_capacity(raws.len());
        for raw in raws {
            indices.push(integer(raw)?);
        }
        let texture = self.binding(field(map, "texture")?)?;
        let decoded_state = state(field(map, "state")?)?;
        let lighting = self.lighting(field(map, "lighting")?)?;
        let fog = match optional(map, "fog") {
            None => None,
            Some(raw) => Some(fog(raw)?),
        };
        let luminance_alpha = match optional(map, "textureEffect") {
            None => false,
            Some(raw) => {
                choice(raw, &["luminance-alpha"])?;
                true
            }
        };
        match string(field(map, "texturing")?)? {
            "single" => {
                let raws = list(field(map, "vertices")?)?;
                let mut vertices = Vec::with_capacity(raws.len());
                for raw in raws {
                    vertices.push(vertex(raw)?);
                }
                Ok(DrawBatch {
                    fog,
                    luminance_alpha,
                    indices,
                    texture,
                    state: decoded_state,
                    lighting,
                    primitive,
                    vertices: BatchVertices::Single(vertices),
                })
            }
            "pair" => {
                let raws = list(field(map, "vertices")?)?;
                let mut vertices = Vec::with_capacity(raws.len());
                for raw in raws {
                    vertices.push(MultitextureVertex {
                        base: vertex(raw)?,
                        tex_coord2: vec2(field(record(raw)?, "texCoord2")?)?,
                    });
                }
                let second = record(field(map, "secondTexture")?)?;
                let environment = match choice(field(second, "environment")?, &["modulate", "add", "replace"])? {
                    "modulate" => PairEnvironment::Modulate,
                    "add" => PairEnvironment::Add,
                    _ => PairEnvironment::Replace,
                };
                Ok(DrawBatch {
                    fog,
                    luminance_alpha,
                    indices,
                    texture,
                    state: decoded_state,
                    lighting,
                    primitive,
                    vertices: BatchVertices::Pair {
                        vertices,
                        second_texture: TextureBundle {
                            binding: self.binding(field(second, "binding")?)?,
                            environment,
                        },
                    },
                })
            }
            _ => Err(bad_wire("invalid renderer texturing")),
        }
    }

    /// Decode one render operation.
    pub fn operation(&mut self, value: &WireValue) -> Result<RenderOperation, RenderError> {
        let map = record(value)?;
        match string(field(map, "kind")?)? {
            "draw" => {
                let raws = list(field(map, "batches")?)?;
                let mut batches = Vec::with_capacity(raws.len());
                for raw in raws {
                    batches.push(self.batch(raw)?);
                }
                Ok(RenderOperation::Draw(batches))
            }
            "object-opacity" => {
                let raws = list(field(map, "batches")?)?;
                let mut batches = Vec::with_capacity(raws.len());
                for raw in raws {
                    batches.push(self.batch(raw)?);
                }
                Ok(RenderOperation::ObjectOpacity {
                    opacity: number(field(map, "opacity")?)?,
                    batches,
                })
            }
            "disable-portal-clip" => Ok(RenderOperation::DisablePortalClip),
            "depth-range" => Ok(RenderOperation::DepthRange(pair(field(map, "range")?)?)),
            "cull" => Ok(RenderOperation::Cull(cull(field(map, "cull")?)?)),
            "polygon-offset" => Ok(RenderOperation::PolygonOffset(offset(field(map, "value")?)?)),
            "depth-atlas" => {
                let image = self.image(field(map, "image")?)?;
                let raws = list(field(map, "passes")?)?;
                let mut passes = Vec::with_capacity(raws.len());
                for raw in raws {
                    passes.push(depth_pass(raw)?);
                }
                Ok(RenderOperation::DepthAtlas { image, passes })
            }
            "sky-side" => {
                let image = self.image(field(map, "image")?)?;
                let color = vec4(field(map, "color")?)?;
                let raws = list(field(map, "strips")?)?;
                let mut strips = Vec::with_capacity(raws.len());
                for raw in raws {
                    let vertices = list(raw)?;
                    let mut strip = Vec::with_capacity(vertices.len());
                    for entry in vertices {
                        let vertex_map = record(entry)?;
                        strip.push(SkyVertex {
                            position: vec4(field(vertex_map, "position")?)?,
                            tex_coord: vec2(field(vertex_map, "texCoord")?)?,
                        });
                    }
                    strips.push(strip);
                }
                Ok(RenderOperation::SkySide { image, color, strips })
            }
            "shadow-volume" => {
                let positions = list(field(map, "positions")?)?;
                let mut decoded = Vec::with_capacity(positions.len());
                for position in positions {
                    decoded.push(vec4(position)?);
                }
                let raws = list(field(map, "indices")?)?;
                let mut indices = Vec::with_capacity(raws.len());
                for raw in raws {
                    indices.push(integer(raw)?);
                }
                Ok(RenderOperation::ShadowVolume {
                    positions: decoded,
                    indices,
                    mirror: boolean(field(map, "mirror")?)?,
                    white_image: self.image(field(map, "whiteImage")?)?,
                })
            }
            "shadow-finish" => {
                let positions = list(field(map, "positions")?)?;
                if positions.len() != 4 {
                    return Err(bad_wire("invalid shadow finish quad"));
                }
                Ok(RenderOperation::ShadowFinish {
                    positions: [
                        vec4(&positions[0])?,
                        vec4(&positions[1])?,
                        vec4(&positions[2])?,
                        vec4(&positions[3])?,
                    ],
                    white_image: self.image(field(map, "whiteImage")?)?,
                })
            }
            "q2-fog" => {
                let fog_map = record(field(map, "fog")?)?;
                if string(field(fog_map, "kind")?)? != "q2" {
                    return Err(bad_wire("invalid Q2 fog kind"));
                }
                let height = record(field(fog_map, "height")?)?;
                let start = record(field(height, "start")?)?;
                let end = record(field(height, "end")?)?;
                Ok(RenderOperation::Q2Fog(Q2FogOperation {
                    camera: camera(field(map, "camera")?)?,
                    fog: Q2Fog {
                        color: vec3(field(fog_map, "color")?)?,
                        density: number(field(fog_map, "density")?)?,
                        sky_factor: number(field(fog_map, "skyFactor")?)?,
                        height: Q2HeightFog {
                            start: Q2HeightStop {
                                color: vec3(field(start, "color")?)?,
                                distance: number(field(start, "distance")?)?,
                            },
                            end: Q2HeightStop {
                                color: vec3(field(end, "color")?)?,
                                distance: number(field(end, "distance")?)?,
                            },
                            density: number(field(height, "density")?)?,
                            falloff: number(field(height, "falloff")?)?,
                        },
                    },
                    far_depth: number(field(map, "farDepth")?)?,
                    sky_drawn: boolean(field(map, "skyDrawn")?)?,
                }))
            }
            "retained-draw" => {
                let eye_raw = list(field(map, "eye")?)?;
                if eye_raw.len() != 3 {
                    return Err(bad_wire("invalid retained eye rows"));
                }
                let eye = [vec4(&eye_raw[0])?, vec4(&eye_raw[1])?, vec4(&eye_raw[2])?];
                let positions_raw = list(field(map, "positions")?)?;
                let mut positions = Vec::with_capacity(positions_raw.len());
                for raw in positions_raw {
                    positions.push(vec3(raw)?);
                }
                let indices_raw = list(field(map, "indices")?)?;
                let mut indices = Vec::with_capacity(indices_raw.len());
                for raw in indices_raw {
                    indices.push(integer(raw)?);
                }
                let passes_raw = list(field(map, "passes")?)?;
                let mut passes = Vec::with_capacity(passes_raw.len());
                for raw in passes_raw {
                    let pass_map = record(raw)?;
                    let coords_raw = list(field(pass_map, "texCoords")?)?;
                    let mut tex_coords = Vec::with_capacity(coords_raw.len());
                    for entry in coords_raw {
                        tex_coords.push(vec2(entry)?);
                    }
                    let coords2_raw = list(field(pass_map, "texCoords2")?)?;
                    let mut tex_coords2 = Vec::with_capacity(coords2_raw.len());
                    for entry in coords2_raw {
                        tex_coords2.push(vec2(entry)?);
                    }
                    let colors_raw = list(field(pass_map, "colors")?)?;
                    let mut colors = Vec::with_capacity(colors_raw.len());
                    for entry in colors_raw {
                        colors.push(vec4(entry)?);
                    }
                    passes.push(RetainedPassAttrs {
                        tex_coords,
                        tex_coords2,
                        colors,
                    });
                }
                let batches_raw = list(field(map, "batches")?)?;
                let mut batches = Vec::with_capacity(batches_raw.len());
                for raw in batches_raw {
                    batches.push(self.retained_batch(raw)?);
                }
                let generation = match field(map, "generation")? {
                    WireValue::Int(value) if *value >= 0 => *value as u64,
                    _ => return Err(bad_wire("invalid retained generation")),
                };
                Ok(RenderOperation::RetainedDraw(RetainedDraw {
                    surface: Arc::new(RetainedSurfaceData {
                        id: RetainedId {
                            surface: integer(field(map, "surface")?)?,
                            generation,
                        },
                        positions,
                        indices,
                        passes,
                    }),
                    eye,
                    projection: matrix(field(map, "projection")?)?,
                    batches,
                }))
            }
            _ => Err(bad_wire("invalid renderer operation")),
        }
    }

    /// Decode one retained batch's per-frame parameters.
    fn retained_batch(&mut self, value: &WireValue) -> Result<RetainedBatch, RenderError> {
        let map = record(value)?;
        let primitive_raw = field(map, "primitive")?;
        let primitive = if matches!(primitive_raw, WireValue::Str(text) if text == "lines") {
            BatchPrimitive::Lines {
                line_width: number(field(map, "lineWidth")?)?,
            }
        } else {
            choice(primitive_raw, &["triangles"])?;
            BatchPrimitive::Triangles
        };
        let range_map = record(field(map, "range")?)?;
        let second_texture = match optional(map, "secondTexture") {
            None => None,
            Some(raw) => {
                let second = record(raw)?;
                let environment = match choice(field(second, "environment")?, &["modulate", "add", "replace"])? {
                    "modulate" => PairEnvironment::Modulate,
                    "add" => PairEnvironment::Add,
                    _ => PairEnvironment::Replace,
                };
                Some(TextureBundle {
                    binding: self.binding(field(second, "binding")?)?,
                    environment,
                })
            }
        };
        let luminance_alpha = match optional(map, "textureEffect") {
            None => false,
            Some(raw) => {
                choice(raw, &["luminance-alpha"])?;
                true
            }
        };
        Ok(RetainedBatch {
            range: RetainedSlice {
                start: integer(field(range_map, "start")?)?,
                count: integer(field(range_map, "count")?)?,
            },
            pass: integer(field(map, "pass")?)?,
            texture: self.binding(field(map, "texture")?)?,
            second_texture,
            state: state(field(map, "state")?)?,
            lighting: self.lighting(field(map, "lighting")?)?,
            fog: match optional(map, "fog") {
                None => None,
                Some(raw) => Some(fog(raw)?),
            },
            primitive,
            luminance_alpha,
        })
    }

    /// Decode shared view state.
    pub fn view_state(value: &WireValue) -> Result<RenderViewState, RenderError> {
        let map = record(value)?;
        let clear_raw = field(map, "clear")?;
        let clear = if matches!(clear_raw, WireValue::Null) {
            None
        } else {
            let clear_map = record(clear_raw)?;
            let color_raw = field(clear_map, "color")?;
            Some(ViewClear {
                depth: number(field(clear_map, "depth")?)?,
                color: if matches!(color_raw, WireValue::Null) {
                    None
                } else {
                    Some(vec4(color_raw)?)
                },
                stencil: boolean(field(clear_map, "stencil")?)?,
            })
        };
        let clip_raw = field(map, "clipPlane")?;
        Ok(RenderViewState {
            viewport: rect(field(map, "viewport")?)?,
            clear,
            clip_plane: if matches!(clip_raw, WireValue::Null) {
                None
            } else {
                Some(vec4(clip_raw)?)
            },
        })
    }

    /// Decode capture ids from a swap-buffers wire value.
    pub fn captures(value: &WireValue) -> Result<Vec<u32>, RenderError> {
        let map = record(value)?;
        if string(field(map, "kind")?)? != "swap-buffers" {
            return Err(bad_wire("invalid renderer command"));
        }
        let raws = list(field(map, "captures")?)?;
        let mut captures = Vec::with_capacity(raws.len());
        for raw in raws {
            captures.push(integer(raw)?);
        }
        Ok(captures)
    }

    /// Decode one render command. The wire drops view targets, view times, and
    /// swap captures (see [`WireDecoder::captures`]); views decode with a
    /// canonical empty preview target and zero time.
    pub fn command(&mut self, value: &WireValue) -> Result<RenderCommand, RenderError> {
        let map = record(value)?;
        match string(field(map, "kind")?)? {
            "draw-buffer" => Ok(RenderCommand::DrawBuffer {
                buffer: match choice(field(map, "buffer")?, &["front", "back", "back-left", "back-right"])? {
                    "front" => DrawBuffer::Front,
                    "back" => DrawBuffer::Back,
                    "back-left" => DrawBuffer::BackLeft,
                    _ => DrawBuffer::BackRight,
                },
                clear: boolean(field(map, "clear")?)?,
            }),
            "set-color" => Ok(RenderCommand::SetColor(vec4(field(map, "color")?)?)),
            "image-resource" => Ok(RenderCommand::ImageResource(
                self.image_operation(field(map, "operation")?)?,
            )),
            "stretch-pic" => {
                let uv_map = record(field(map, "uv")?)?;
                Ok(RenderCommand::StretchPic {
                    rect: rect(field(map, "rect")?)?,
                    uv: TextureRect {
                        s1: number(field(uv_map, "s1")?)?,
                        t1: number(field(uv_map, "t1")?)?,
                        s2: number(field(uv_map, "s2")?)?,
                        t2: number(field(uv_map, "t2")?)?,
                    },
                    image: self.image(field(map, "image")?)?,
                })
            }
            "swap-buffers" => {
                let _ = Self::captures(value)?;
                Ok(RenderCommand::SwapBuffers)
            }
            "view" => {
                let view = record(field(map, "view")?)?;
                let before = list(field(view, "beforeView")?)?;
                let mut before_view = Vec::with_capacity(before.len());
                for raw in before {
                    before_view.push(self.operation(raw)?);
                }
                let raws = list(field(view, "operations")?)?;
                let mut operations = Vec::with_capacity(raws.len());
                for raw in raws {
                    operations.push(self.operation(raw)?);
                }
                Ok(RenderCommand::View(RenderView {
                    state: Self::view_state(&WireValue::Map(view.to_vec()))?,
                    target: ViewTarget::Preview(String::new()),
                    time: SourceTime::Seconds(0.0),
                    before_view,
                    operations,
                }))
            }
            _ => Err(bad_wire("invalid renderer command")),
        }
    }
}

fn depth_pass(value: &WireValue) -> Result<DepthAtlasPass, RenderError> {
    let map = record(value)?;
    let clear_raw = field(map, "clearDepth")?;
    let raws = list(field(map, "draws")?)?;
    let mut draws = Vec::with_capacity(raws.len());
    for raw in raws {
        let entry = record(raw)?;
        let positions = list(field(entry, "positions")?)?;
        let mut decoded = Vec::with_capacity(positions.len());
        for position in positions {
            decoded.push(vec4(position)?);
        }
        let indices = list(field(entry, "indices")?)?;
        let mut decoded_indices = Vec::with_capacity(indices.len());
        for index in indices {
            decoded_indices.push(integer(index)?);
        }
        draws.push(DepthAtlasDraw {
            positions: decoded,
            indices: decoded_indices,
            cull: cull(field(entry, "cull")?)?,
            polygon_offset: offset(field(entry, "polygonOffset")?)?,
        });
    }
    Ok(DepthAtlasPass {
        viewport: rect(field(map, "viewport")?)?,
        clear_depth: if matches!(clear_raw, WireValue::Null) {
            None
        } else {
            Some(number(clear_raw)?)
        },
        draws,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec2, vec3, vec4};

    use super::super::types::fresh_owner_identity;
    use super::*;

    const PALETTE_SOURCE: &str = "test/palette.lmp";

    fn owner() -> ResourceOwner {
        let authority = IdentityOwner::create("wire-test").unwrap();
        ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0)
    }

    fn generated(owner: &ResourceOwner, ordinal: u32) -> RendererImage {
        RendererImage {
            owner: owner.clone(),
            ordinal,
            width: 4,
            height: 2,
            source: ImageSource::Generated {
                name: format!("img{ordinal}"),
            },
        }
    }

    fn codec_pair() -> (WireEncoder, WireDecoder) {
        let shared = owner();
        let decoder_owner = shared.clone();
        let fallback = generated(&decoder_owner, 999);
        (
            WireEncoder::new(shared),
            WireDecoder::new(decoder_owner, PALETTE_SOURCE.to_string(), move |_, _| fallback.clone()),
        )
    }

    struct FixedSource {
        image: RendererImage,
    }

    impl DynamicImageSource for FixedSource {
        fn resolve(&self, _apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage {
            self.image.clone()
        }
    }

    fn vertex() -> RenderVertex {
        RenderVertex {
            position: vec4(1.0, 2.0, 3.0, 1.0),
            tex_coord: vec2(0.25, 0.5),
            color: vec4(1.0, 1.0, 1.0, 1.0),
        }
    }

    fn batch(texture: TextureBinding, lighting: BatchLighting) -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0],
            texture,
            state: RenderState::opaque(CullFace::Back),
            lighting,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![vertex()]),
        }
    }

    fn retained_draw(image: RendererImage) -> RetainedDraw {
        RetainedDraw {
            surface: Arc::new(RetainedSurfaceData {
                id: RetainedId {
                    surface: 7,
                    generation: 3,
                },
                positions: vec![vec3(1.0, 2.0, 3.0), vec3(4.0, 5.0, 6.0)],
                indices: vec![0, 1, 0],
                passes: vec![RetainedPassAttrs {
                    tex_coords: vec![vec2(0.0, 0.0), vec2(1.0, 1.0)],
                    tex_coords2: vec![vec2(0.5, 0.5), vec2(0.25, 0.75)],
                    colors: vec![vec4(1.0, 0.0, 0.0, 1.0), vec4(0.0, 1.0, 0.0, 1.0)],
                }],
            }),
            eye: [
                vec4(1.0, 0.0, 0.0, 0.5),
                vec4(0.0, 1.0, 0.0, -0.5),
                vec4(0.0, 0.0, 1.0, 2.0),
            ],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            batches: vec![RetainedBatch {
                range: RetainedSlice { start: 0, count: 3 },
                pass: 0,
                texture: TextureBinding::BindImage(image.clone()),
                second_texture: Some(TextureBundle {
                    binding: TextureBinding::BindImage(image),
                    environment: PairEnvironment::Modulate,
                }),
                state: RenderState::opaque(CullFace::Back),
                lighting: BatchLighting::Vertex,
                fog: Some(BatchFog::Constant {
                    color: vec3(0.1, 0.2, 0.3),
                    amount: 0.5,
                }),
                primitive: BatchPrimitive::Triangles,
                luminance_alpha: true,
            }],
        }
    }

    fn shadowed_light() -> Q2FragmentLight {
        Q2FragmentLight {
            origin: vec3(1.0, 2.0, 3.0),
            radius: 300.0,
            color: vec3(1.0, 0.5, 0.25),
            scale: 2.0,
            cone: Some(Q2LightCone {
                direction: vec3(0.0, 0.0, -1.0),
                cos_half_angle: 0.5,
            }),
            shadow: Q2ShadowProjection::Cone {
                matrix: [1.0; 16],
                atlas_rect: vec4(0.0, 0.0, 0.5, 0.5),
            },
        }
    }

    fn portal_camera(viewport: Rect) -> RenderCamera {
        RenderCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [0.0; 16],
            viewport,
            clip: ViewClip::Portal {
                plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 5.0,
                },
                mirror: true,
            },
        }
    }

    fn rgba_level(width: u32, height: u32) -> ImageLevel {
        ImageLevel {
            width,
            height,
            pixels: vec![7u8; (width * height * 4) as usize],
        }
    }

    fn depth_level(width: u32, height: u32) -> DepthImageLevel {
        DepthImageLevel {
            width,
            height,
            pixels: vec![0.5; (width * height) as usize],
        }
    }

    fn indexed_image() -> RenderImage {
        RenderImage::Indexed8 {
            levels: vec![rgba_level(2, 1)],
            palette: Palette {
                colors: vec![9u8; 768],
                source: PALETTE_SOURCE.to_string(),
            },
            transparency: PaletteTransparency::Index(3),
            fullbright: Some((1, 2)),
            translation: Some(vec![4u8; 256]),
        }
    }

    fn is_bad_wire(result: &Result<impl Sized, RenderError>) -> bool {
        matches!(result, Err(RenderError::BadWire(_)))
    }

    #[test]
    fn image_round_trip_registers_ordinal() {
        let (mut encoder, mut decoder) = codec_pair();
        let image = generated(&encoder.owner.clone(), 3);
        let wire = encoder.image(&image).unwrap();
        let decoded = decoder.image(&wire).unwrap();
        assert_eq!(decoded, image);
        assert_eq!(encoder.original_image(3).unwrap(), image);
        assert!(matches!(encoder.original_image(4), Err(RenderError::UnknownImage(4))));
    }

    #[test]
    fn image_uses_resource_path_as_name() {
        let (mut encoder, mut decoder) = codec_pair();
        let mut image = generated(&encoder.owner.clone(), 1);
        image.source = ImageSource::Resource {
            requested_path: "textures/e1m1.wad".to_string(),
        };
        let wire = encoder.image(&image).unwrap();
        let map = record(&wire).unwrap();
        assert_eq!(string(field(map, "name").unwrap()).unwrap(), "textures/e1m1.wad");
        let decoded = decoder.image(&wire).unwrap();
        assert_eq!(decoded.ordinal, 1);
        assert_eq!(
            decoded.source,
            ImageSource::Generated {
                name: "textures/e1m1.wad".to_string()
            }
        );
    }

    #[test]
    fn image_rejects_foreign_lifetime_and_conflicts() {
        let (mut encoder, _) = codec_pair();
        let mut foreign = generated(&owner(), 1);
        foreign.owner = owner();
        assert!(matches!(encoder.image(&foreign), Err(RenderError::ForeignOwner(_))));
        let first = generated(&encoder.owner.clone(), 1);
        encoder.image(&first).unwrap();
        let mut clash = generated(&encoder.owner.clone(), 1);
        clash.width = 8;
        assert!(matches!(encoder.image(&clash), Err(RenderError::ImageConflict(1))));
    }

    #[test]
    fn decoder_rejects_metadata_change() {
        let (mut encoder, mut decoder) = codec_pair();
        let image = generated(&encoder.owner.clone(), 1);
        decoder.image(&encoder.image(&image).unwrap()).unwrap();
        let changed = WireValue::object(vec![
            ("ordinal", WireValue::Int(1)),
            ("width", WireValue::Int(8)),
            ("height", WireValue::Int(2)),
            ("name", WireValue::text("img1")),
        ]);
        assert!(is_bad_wire(&decoder.image(&changed)));
    }

    #[test]
    fn bindings_round_trip_with_stable_tokens() {
        let (mut encoder, mut decoder) = codec_pair();
        let image = generated(&encoder.owner.clone(), 1);
        let wire = encoder.binding(&TextureBinding::BindImage(image.clone())).unwrap();
        assert_eq!(
            decoder.binding(&wire).unwrap(),
            TextureBinding::BindImage(image.clone())
        );
        let wire = encoder.binding(&TextureBinding::RetainCurrentTexture).unwrap();
        assert_eq!(decoder.binding(&wire).unwrap(), TextureBinding::RetainCurrentTexture);
        let source: Arc<dyn DynamicImageSource> = Arc::new(FixedSource { image: image.clone() });
        let first = encoder
            .binding(&TextureBinding::DynamicImage(Arc::clone(&source)))
            .unwrap();
        let second = encoder
            .binding(&TextureBinding::DynamicImage(Arc::clone(&source)))
            .unwrap();
        assert_eq!(first, second);
        assert!(Arc::ptr_eq(&encoder.source(1).unwrap(), &source));
        assert!(is_bad_wire(&encoder.source(9).map(|_| ())));
        let TextureBinding::DynamicImage(decoded) = decoder.binding(&first).unwrap() else {
            panic!("expected dynamic binding");
        };
        let resolved = decoded.resolve(&mut |_| {});
        assert_eq!(resolved.ordinal, 999);
        encoder.clear_sources();
        assert!(is_bad_wire(&encoder.source(1).map(|_| ())));
    }

    #[test]
    fn acknowledgments_retain_and_release() {
        let (mut encoder, _) = codec_pair();
        let image = generated(&encoder.owner.clone(), 1);
        let operation = ImageResourceOperation::ReleaseImage { image };
        let wire = encoder.image_operation(&operation).unwrap();
        let ack = field(record(&wire).unwrap(), "acknowledgment").unwrap().clone();
        assert_eq!(encoder.acknowledge(&ack).unwrap(), operation);
        assert!(is_bad_wire(&encoder.acknowledge(&ack)));
        assert!(is_bad_wire(&encoder.acknowledge(&WireValue::text("no"))));
    }

    #[test]
    fn image_operations_round_trip() {
        let shared = owner();
        let image = generated(&shared, 5);
        for operation in [
            ImageResourceOperation::CreateImage {
                image: image.clone(),
                content: indexed_image(),
                sampling: TextureSampling {
                    repeat: true,
                    filter: TextureFilter::LinearMipmapLinear,
                },
            },
            ImageResourceOperation::CreateImage {
                image: image.clone(),
                content: RenderImage::Rgba8 {
                    levels: vec![rgba_level(2, 2), rgba_level(1, 1)],
                    border_color: vec4(0.0, 0.0, 0.0, 1.0),
                },
                sampling: TextureSampling {
                    repeat: false,
                    filter: TextureFilter::Nearest,
                },
            },
            ImageResourceOperation::CreateImage {
                image: image.clone(),
                content: RenderImage::Depth32f {
                    levels: vec![depth_level(2, 2)],
                },
                sampling: TextureSampling {
                    repeat: true,
                    filter: TextureFilter::Linear,
                },
            },
            ImageResourceOperation::UpdateImage {
                image: image.clone(),
                level: 1,
                content: LevelContent::Rgba(rgba_level(1, 1)),
            },
            ImageResourceOperation::UpdateImage {
                image: image.clone(),
                level: 0,
                content: LevelContent::Depth(depth_level(2, 1)),
            },
            ImageResourceOperation::ReleaseImage { image: image.clone() },
            ImageResourceOperation::TextureMode {
                filter: TextureFilter::NearestMipmapLinear,
            },
        ] {
            let (mut encoder, mut decoder) = pair_with(&shared);
            let wire = encoder.image_operation(&operation).unwrap();
            let decoded = decoder.image_operation(&wire).unwrap();
            assert_eq!(decoded, operation);
            let ack = integer(field(record(&wire).unwrap(), "acknowledgment").unwrap()).unwrap();
            assert_eq!(decoder.acknowledgment(&decoded).unwrap(), ack);
        }
    }

    fn pair_with(shared: &ResourceOwner) -> (WireEncoder, WireDecoder) {
        let decoder_owner = shared.clone();
        let fallback = generated(&decoder_owner, 999);
        (
            WireEncoder::new(shared.clone()),
            WireDecoder::new(decoder_owner, PALETTE_SOURCE.to_string(), move |_, _| fallback.clone()),
        )
    }

    #[test]
    fn create_wire_strips_palette_source() {
        let (mut encoder, _) = codec_pair();
        let image = generated(&encoder.owner.clone(), 1);
        let operation = ImageResourceOperation::CreateImage {
            image,
            content: indexed_image(),
            sampling: TextureSampling {
                repeat: true,
                filter: TextureFilter::Linear,
            },
        };
        let wire = encoder.image_operation(&operation).unwrap();
        let map = record(&wire).unwrap();
        let content = record(field(map, "content").unwrap()).unwrap();
        let palette = record(field(content, "palette").unwrap()).unwrap();
        assert_eq!(palette.len(), 1);
        assert!(matches!(
            field(palette, "colors").unwrap(),
            WireValue::Bytes(colors) if colors.len() == 768
        ));
    }

    #[test]
    fn decoder_acknowledgment_requires_decoded_operation() {
        let (_, decoder) = codec_pair();
        assert!(is_bad_wire(&decoder.acknowledgment(
            &ImageResourceOperation::TextureMode {
                filter: TextureFilter::Linear,
            }
        )));
    }

    fn round_trip_batch(batch: &DrawBatch) -> DrawBatch {
        let shared = batch_texture_owner(batch);
        let (mut encoder, mut decoder) = pair_with(&shared);
        let wire = encoder.batch(batch).unwrap();
        decoder.batch(&wire).unwrap()
    }

    fn batch_texture_owner(batch: &DrawBatch) -> ResourceOwner {
        match &batch.texture {
            TextureBinding::BindImage(image) => image.owner.clone(),
            TextureBinding::RetainCurrentTexture => owner(),
            TextureBinding::DynamicImage(_) => owner(),
        }
    }

    #[test]
    fn batches_round_trip_single_and_pair() {
        let shared = owner();
        let image = generated(&shared, 1);
        let second = generated(&shared, 2);
        let single = batch(TextureBinding::BindImage(image.clone()), BatchLighting::Vertex);
        assert_eq!(round_trip_batch(&single), single);
        let mut textured = batch(TextureBinding::RetainCurrentTexture, BatchLighting::Vertex);
        textured.fog = Some(BatchFog::Constant {
            color: vec3(0.1, 0.2, 0.3),
            amount: 0.75,
        });
        textured.luminance_alpha = true;
        textured.primitive = BatchPrimitive::Lines { line_width: 2.0 };
        textured.vertices = BatchVertices::Pair {
            vertices: vec![MultitextureVertex {
                base: vertex(),
                tex_coord2: vec2(0.1, 0.9),
            }],
            second_texture: TextureBundle {
                binding: TextureBinding::BindImage(second),
                environment: PairEnvironment::Add,
            },
        };
        let (mut encoder, mut decoder) = pair_with(&shared);
        let wire = encoder.batch(&textured).unwrap();
        assert_eq!(decoder.batch(&wire).unwrap(), textured);
    }

    #[test]
    fn batches_round_trip_all_lighting() {
        let shared = owner();
        let image = generated(&shared, 1);
        let atlas = Q2ShadowAtlas {
            image: generated(&shared, 2),
            texel_size: 0.01,
            near_plane: 4.0,
        };
        let world = |pass| BatchLighting::Q2World {
            world_positions: vec![vec3(1.0, 2.0, 3.0)],
            normals: vec![vec3(0.0, 0.0, 1.0)],
            atlas: Some(atlas.clone()),
            pass,
        };
        let cases = vec![
            BatchLighting::Vertex,
            world(Q2LightPass::Lightmap {
                lights: vec![shadowed_light()],
            }),
            world(Q2LightPass::Texture { lights: vec![] }),
            BatchLighting::Q2World {
                world_positions: vec![],
                normals: vec![],
                atlas: None,
                pass: Q2LightPass::MaterialLightmap {
                    lights: vec![Q2FragmentLight {
                        cone: None,
                        shadow: Q2ShadowProjection::Point {
                            atlas_rect: vec4(0.0, 0.0, 1.0, 1.0),
                        },
                        ..shadowed_light()
                    }],
                },
            },
            world(Q2LightPass::Model {
                lights: vec![Q2ModelFragmentLight {
                    light: Q2FragmentLight {
                        cone: None,
                        shadow: Q2ShadowProjection::None,
                        ..shadowed_light()
                    },
                    fraction: vec3(0.5, 0.5, 0.5),
                }],
                shade_scale: None,
            }),
            world(Q2LightPass::Model {
                lights: vec![],
                shade_scale: Some(1.5),
            }),
            BatchLighting::Q2ModelShadow {
                world_positions: vec![vec3(0.0, 1.0, 0.0)],
                lights: vec![Q2ModelShadowLight {
                    origin: vec3(1.0, 1.0, 1.0),
                    radius: 100.0,
                    fraction: vec3(1.0, 0.0, 0.0),
                    shadow: Q2ShadowProjection::None,
                }],
                shade_scale: 2.0,
                atlas: atlas.clone(),
            },
        ];
        for lighting in cases {
            let mut probe = batch(TextureBinding::BindImage(image.clone()), lighting);
            probe.fog = Some(BatchFog::Exp2 {
                color: vec3(0.5, 0.5, 0.5),
                density: 0.02,
                effect: FogEffect::Overlay,
            });
            let (mut encoder, mut decoder) = pair_with(&shared);
            let wire = encoder.batch(&probe).unwrap();
            assert_eq!(decoder.batch(&wire).unwrap(), probe);
        }
    }

    #[test]
    fn fog_effect_defaults_to_color_when_absent() {
        let wire = WireValue::object(vec![
            ("kind", WireValue::text("exp2")),
            ("color", encode_vec3(vec3(0.0, 0.0, 0.0))),
            ("density", WireValue::Float(0.1)),
        ]);
        assert!(matches!(
            fog(&wire).unwrap(),
            BatchFog::Exp2 {
                effect: FogEffect::Color,
                ..
            }
        ));
    }

    fn round_trip_operation_value(operation: &RenderOperation, shared: &ResourceOwner) -> RenderOperation {
        let (mut encoder, mut decoder) = pair_with(shared);
        let wire = encoder.operation(operation).unwrap();
        decoder.operation(&wire).unwrap()
    }

    #[test]
    fn operations_round_trip_every_kind() {
        let shared = owner();
        let image = generated(&shared, 1);
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        let simple = batch(TextureBinding::BindImage(image.clone()), BatchLighting::Vertex);
        let cases = vec![
            RenderOperation::Draw(vec![simple.clone()]),
            RenderOperation::ObjectOpacity {
                opacity: 0.5,
                batches: vec![simple.clone()],
            },
            RenderOperation::Q2Fog(Q2FogOperation {
                camera: portal_camera(viewport),
                fog: Q2Fog {
                    color: vec3(0.4, 0.4, 0.4),
                    density: 0.01,
                    sky_factor: 0.9,
                    height: Q2HeightFog {
                        start: Q2HeightStop {
                            color: vec3(0.1, 0.1, 0.1),
                            distance: 10.0,
                        },
                        end: Q2HeightStop {
                            color: vec3(0.9, 0.9, 0.9),
                            distance: 100.0,
                        },
                        density: 0.02,
                        falloff: 1.5,
                    },
                },
                far_depth: 0.999,
                sky_drawn: true,
            }),
            RenderOperation::DepthAtlas {
                image: image.clone(),
                passes: vec![DepthAtlasPass {
                    viewport,
                    clear_depth: Some(1.0),
                    draws: vec![DepthAtlasDraw {
                        positions: vec![vec4(0.0, 0.0, 0.0, 1.0)],
                        indices: vec![0],
                        cull: CullFace::Front,
                        polygon_offset: Some(PolygonOffset {
                            factor: 1.0,
                            units: 2.0,
                        }),
                    }],
                }],
            },
            RenderOperation::DepthRange([0.1, 0.9]),
            RenderOperation::Cull(CullFace::Front),
            RenderOperation::PolygonOffset(Some(PolygonOffset {
                factor: 0.5,
                units: 1.0,
            })),
            RenderOperation::PolygonOffset(None),
            RenderOperation::DisablePortalClip,
            RenderOperation::SkySide {
                image: image.clone(),
                color: vec4(1.0, 0.0, 0.0, 1.0),
                strips: vec![vec![SkyVertex {
                    position: vec4(0.0, 1.0, 0.0, 1.0),
                    tex_coord: vec2(0.0, 1.0),
                }]],
            },
            RenderOperation::ShadowVolume {
                positions: vec![vec4(1.0, 0.0, 0.0, 1.0)],
                indices: vec![0],
                mirror: true,
                white_image: image.clone(),
            },
            RenderOperation::ShadowFinish {
                positions: [vec4(0.0, 0.0, 0.0, 1.0); 4],
                white_image: image.clone(),
            },
            RenderOperation::RetainedDraw(retained_draw(image.clone())),
        ];
        for operation in cases {
            assert_eq!(round_trip_operation_value(&operation, &shared), operation);
        }
    }

    #[test]
    fn commands_round_trip() {
        let shared = owner();
        let image = generated(&shared, 1);
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
        };
        let simple = batch(TextureBinding::BindImage(image.clone()), BatchLighting::Vertex);
        for buffer in [
            DrawBuffer::Front,
            DrawBuffer::Back,
            DrawBuffer::BackLeft,
            DrawBuffer::BackRight,
        ] {
            let command = RenderCommand::DrawBuffer { buffer, clear: true };
            let (mut encoder, mut decoder) = pair_with(&shared);
            let wire = encoder.command(&command, &[]).unwrap();
            assert_eq!(decoder.command(&wire).unwrap(), command);
        }
        let rest = vec![
            RenderCommand::ImageResource(ImageResourceOperation::TextureMode {
                filter: TextureFilter::Linear,
            }),
            RenderCommand::SetColor(vec4(1.0, 0.5, 0.25, 1.0)),
            RenderCommand::StretchPic {
                rect: viewport,
                uv: TextureRect {
                    s1: 0.0,
                    t1: 0.0,
                    s2: 1.0,
                    t2: 1.0,
                },
                image: image.clone(),
            },
        ];
        for command in rest {
            let (mut encoder, mut decoder) = pair_with(&shared);
            let wire = encoder.command(&command, &[]).unwrap();
            assert_eq!(decoder.command(&wire).unwrap(), command);
        }
        let view = RenderCommand::View(RenderView {
            state: RenderViewState {
                viewport,
                clear: Some(ViewClear {
                    depth: 1.0,
                    color: Some(vec4(0.0, 0.0, 0.0, 1.0)),
                    stencil: true,
                }),
                clip_plane: Some(vec4(0.0, 0.0, 1.0, 0.0)),
            },
            target: ViewTarget::Preview("main".to_string()),
            time: SourceTime::Milliseconds(16.0),
            before_view: vec![RenderOperation::DisablePortalClip],
            operations: vec![RenderOperation::Draw(vec![simple])],
        });
        let (mut encoder, mut decoder) = pair_with(&shared);
        let wire = encoder.command(&view, &[]).unwrap();
        let RenderCommand::View(decoded) = decoder.command(&wire).unwrap() else {
            panic!("expected view command");
        };
        let RenderCommand::View(expected) = &view else {
            unreachable!();
        };
        assert_eq!(decoded.state, expected.state);
        assert_eq!(decoded.before_view, expected.before_view);
        assert_eq!(decoded.operations, expected.operations);
        let (mut encoder, mut decoder) = pair_with(&shared);
        let wire = encoder.command(&RenderCommand::SwapBuffers, &[3, 7]).unwrap();
        assert_eq!(decoder.command(&wire).unwrap(), RenderCommand::SwapBuffers);
        assert_eq!(WireDecoder::captures(&wire).unwrap(), vec![3, 7]);
    }

    #[test]
    fn view_state_and_levels_decode() {
        let state = WireValue::object(vec![
            (
                "viewport",
                encode_rect(Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                }),
            ),
            ("clear", WireValue::Null),
            ("clipPlane", WireValue::Null),
        ]);
        let decoded = WireDecoder::view_state(&state).unwrap();
        assert!(decoded.clear.is_none() && decoded.clip_plane.is_none());
        let level = WireValue::object(vec![
            ("width", WireValue::Int(2)),
            ("height", WireValue::Int(1)),
            ("pixels", WireValue::Bytes(vec![0u8; 8])),
        ]);
        assert_eq!(decode_image_level(&level).unwrap().pixels.len(), 8);
        let short = WireValue::object(vec![
            ("width", WireValue::Int(2)),
            ("height", WireValue::Int(1)),
            ("pixels", WireValue::Bytes(vec![0u8; 7])),
        ]);
        assert!(is_bad_wire(&decode_image_level(&short)));
    }

    #[test]
    fn dynamic_resolve_reaches_caller_callback() {
        let shared = owner();
        let uploaded: Arc<Mutex<Vec<ImageResourceOperation>>> = Arc::new(Mutex::new(Vec::new()));
        let seen: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let target = generated(&shared, 42);
        let mut encoder = WireEncoder::new(shared.clone());
        let source: Arc<dyn DynamicImageSource> = Arc::new(FixedSource {
            image: generated(&shared, 1),
        });
        let wire = encoder.binding(&TextureBinding::DynamicImage(source)).unwrap();
        let inner_uploaded = Arc::clone(&uploaded);
        let inner_seen = Arc::clone(&seen);
        let inner_target = target.clone();
        let mut decoder = WireDecoder::new(shared, PALETTE_SOURCE.to_string(), move |token, apply| {
            inner_seen.lock().unwrap().push(token);
            apply(ImageResourceOperation::TextureMode {
                filter: TextureFilter::Linear,
            });
            inner_target.clone()
        });
        let TextureBinding::DynamicImage(decoded) = decoder.binding(&wire).unwrap() else {
            panic!("expected dynamic binding");
        };
        let resolved = decoded.resolve(&mut |operation| {
            uploaded.lock().unwrap().push(operation);
        });
        assert_eq!(resolved, target);
        assert_eq!(*seen.lock().unwrap(), vec![1]);
        assert_eq!(inner_uploaded.lock().unwrap().len(), 1);
    }

    #[test]
    fn worker_failure_maps_to_worker_error() {
        let (_, mut decoder) = codec_pair();
        let failure = WireValue::object(vec![("workerFailure", WireValue::text("boom"))]);
        assert!(matches!(
            decoder.image(&failure),
            Err(RenderError::Worker(message)) if message == "boom"
        ));
    }

    #[test]
    fn bad_variants_fail() {
        let (_, mut decoder) = codec_pair();
        let cases = vec![
            WireValue::object(vec![("kind", WireValue::text("bind-nothing"))]),
            WireValue::Int(3),
            WireValue::Null,
        ];
        for case in cases {
            assert!(decoder.binding(&case).is_err());
        }
        assert!(is_bad_wire(
            &decoder.operation(&WireValue::object(vec![("kind", WireValue::text("explode"))]))
        ));
        assert!(is_bad_wire(
            &decoder.command(&WireValue::object(vec![("kind", WireValue::text("explode"))]))
        ));
        assert!(is_bad_wire(&decoder.image_operation(&WireValue::object(vec![
            ("kind", WireValue::text("explode")),
            ("acknowledgment", WireValue::Int(1)),
        ]))));
        assert!(is_bad_wire(&decoder.batch(&WireValue::object(vec![
            ("primitive", WireValue::text("quads")),
            ("indices", WireValue::list(vec![])),
            (
                "texture",
                WireValue::object(vec![("kind", WireValue::text("retain-current-texture"))])
            ),
            ("state", encode_state(&RenderState::opaque(CullFace::None))),
            ("lighting", WireValue::object(vec![("kind", WireValue::text("vertex"))])),
            ("texturing", WireValue::text("single")),
            ("vertices", WireValue::list(vec![])),
        ]))));
        assert!(is_bad_wire(&fog(&WireValue::object(vec![
            ("kind", WireValue::text("smoke")),
            ("color", encode_vec3(vec3(0.0, 0.0, 0.0))),
        ]))));
        assert!(is_bad_wire(&shadow(&WireValue::object(vec![(
            "kind",
            WireValue::text("blob")
        )]))));
        assert!(is_bad_wire(&blend(&WireValue::text("super"))));
        assert!(is_bad_wire(&filter(&WireValue::text("super"))));
        assert!(is_bad_wire(&cull(&WireValue::text("super"))));
        assert!(is_bad_wire(&pair(&WireValue::list(vec![WireValue::Float(1.0)]))));
        assert!(is_bad_wire(&matrix(&WireValue::list(vec![WireValue::Float(1.0)]))));
        assert!(is_bad_wire(&camera(&WireValue::object(vec![
            ("origin", encode_vec3(vec3(0.0, 0.0, 0.0))),
            ("axis", WireValue::list(vec![encode_vec3(vec3(1.0, 0.0, 0.0))])),
            ("projection", encode_matrix(&[0.0; 16])),
            (
                "viewport",
                encode_rect(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                })
            ),
            ("clip", WireValue::object(vec![("kind", WireValue::text("none"))])),
        ]))));
        assert!(is_bad_wire(&decoder.operation(&WireValue::object(vec![
            ("kind", WireValue::text("shadow-finish")),
            ("positions", WireValue::list(vec![])),
            (
                "whiteImage",
                WireValue::object(vec![
                    ("ordinal", WireValue::Int(1)),
                    ("width", WireValue::Int(4)),
                    ("height", WireValue::Int(2)),
                    ("name", WireValue::text("img1")),
                ])
            ),
        ]))));
        assert!(is_bad_wire(&decoder.operation(&WireValue::object(vec![
            ("kind", WireValue::text("q2-fog")),
            (
                "camera",
                encode_camera(&portal_camera(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                }))
            ),
            ("farDepth", WireValue::Float(1.0)),
            ("skyDrawn", WireValue::Bool(false)),
            ("fog", WireValue::object(vec![("kind", WireValue::text("q1"))])),
        ]))));
        assert!(is_bad_wire(&WireDecoder::captures(&WireValue::object(vec![
            ("kind", WireValue::text("draw-buffer")),
            ("buffer", WireValue::text("front")),
            ("clear", WireValue::Bool(false)),
        ]))));
    }

    #[test]
    fn bad_pixels_fail() {
        let shared = owner();
        let (mut encoder, mut decoder) = pair_with(&shared);
        let image = generated(&shared, 1);
        let wire_image = encoder.image(&image).unwrap();
        let content = |kind: &str, pixels: WireValue| {
            WireValue::object(vec![
                ("kind", WireValue::text(kind)),
                (
                    "levels",
                    WireValue::list(vec![WireValue::object(vec![
                        ("width", WireValue::Int(2)),
                        ("height", WireValue::Int(1)),
                        ("pixels", pixels),
                    ])]),
                ),
                ("borderColor", encode_vec4(vec4(0.0, 0.0, 0.0, 1.0))),
            ])
        };
        for bad in [
            content("rgba8", WireValue::Bytes(vec![0u8; 7])),
            content("rgba8", WireValue::Floats(vec![0.0; 2])),
            content("depth32f", WireValue::Bytes(vec![0u8; 8])),
            content("depth32f", WireValue::Floats(vec![0.0; 3])),
            content("rgba8", WireValue::text("nope")),
        ] {
            let operation = WireValue::object(vec![
                ("kind", WireValue::text("create-image")),
                ("image", wire_image.clone()),
                ("content", bad),
                (
                    "sampling",
                    encode_sampling(&TextureSampling {
                        repeat: true,
                        filter: TextureFilter::Linear,
                    }),
                ),
                ("acknowledgment", WireValue::Int(1)),
            ]);
            assert!(is_bad_wire(&decoder.image_operation(&operation)));
        }
        let mut indexed = match indexed_image() {
            RenderImage::Indexed8 {
                levels,
                palette,
                transparency,
                fullbright,
                translation,
            } => (levels, palette, transparency, fullbright, translation),
            _ => unreachable!(),
        };
        indexed.1.colors = vec![0u8; 100];
        let operation = ImageResourceOperation::CreateImage {
            image: image.clone(),
            content: RenderImage::Indexed8 {
                levels: indexed.0,
                palette: indexed.1,
                transparency: indexed.2,
                fullbright: indexed.3,
                translation: indexed.4,
            },
            sampling: TextureSampling {
                repeat: true,
                filter: TextureFilter::Linear,
            },
        };
        let wire = encoder.image_operation(&operation).unwrap();
        assert!(is_bad_wire(&decoder.image_operation(&wire)));
        let fenced = WireValue::object(vec![
            ("kind", WireValue::text("create-image")),
            ("image", wire_image.clone()),
            (
                "content",
                WireValue::object(vec![
                    ("kind", WireValue::text("indexed8")),
                    (
                        "levels",
                        WireValue::list(vec![WireValue::object(vec![
                            ("width", WireValue::Int(2)),
                            ("height", WireValue::Int(1)),
                            ("pixels", WireValue::Bytes(vec![0u8; 8])),
                        ])]),
                    ),
                    (
                        "palette",
                        WireValue::object(vec![("colors", WireValue::Bytes(vec![0u8; 768]))]),
                    ),
                    (
                        "transparency",
                        WireValue::object(vec![
                            ("kind", WireValue::text("q1-fence")),
                            ("index", WireValue::Int(3)),
                        ]),
                    ),
                    ("fullbright", WireValue::Null),
                    ("translation", WireValue::Null),
                ]),
            ),
            (
                "sampling",
                encode_sampling(&TextureSampling {
                    repeat: true,
                    filter: TextureFilter::Linear,
                }),
            ),
            ("acknowledgment", WireValue::Int(2)),
        ]);
        assert!(is_bad_wire(&decoder.image_operation(&fenced)));
        let empty = WireValue::object(vec![
            ("kind", WireValue::text("create-image")),
            ("image", wire_image),
            (
                "content",
                WireValue::object(vec![
                    ("kind", WireValue::text("rgba8")),
                    ("levels", WireValue::list(vec![])),
                    ("borderColor", encode_vec4(vec4(0.0, 0.0, 0.0, 1.0))),
                ]),
            ),
            (
                "sampling",
                encode_sampling(&TextureSampling {
                    repeat: true,
                    filter: TextureFilter::Linear,
                }),
            ),
            ("acknowledgment", WireValue::Int(3)),
        ]);
        assert!(is_bad_wire(&decoder.image_operation(&empty)));
    }
}
