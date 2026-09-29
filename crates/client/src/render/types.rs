//! Ordered renderer contract types.
//!
//! Donor provenance: `src/contracts/render.ts` in full (image levels,
//! palettes, resource owners, blend state, shader stages, batches, lights,
//! operations, views, commands, frames, and the backend interface).
//! `f32` storage follows the workspace math library; donor wire `number`
//! fields that count pixels or ordinals use `u32`/`i32` here.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use qa_core::identity::{SeatId, SessionId};
use qa_core::math::{Axis, Mat4, Plane, Vec2, Vec3, Vec4};

use super::error::RenderError;

static NEXT_OWNER_IDENTITY: AtomicU64 = AtomicU64::new(1);

/// Mint a fresh renderer-lifetime identity.
#[must_use]
pub fn fresh_owner_identity() -> u64 {
    NEXT_OWNER_IDENTITY.fetch_add(1, Ordering::SeqCst)
}

/// One RGBA8 mipmap level.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageLevel {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes, `width * height * 4` long.
    pub pixels: Vec<u8>,
}

/// One 32-bit float depth mipmap level.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthImageLevel {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major depth samples, `width * height` long.
    pub pixels: Vec<f32>,
}

/// Indexed-color palette plus its content source path.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    /// 768 RGB bytes.
    pub colors: Vec<u8>,
    /// Content path the palette was resolved from.
    pub source: String,
}

/// Indexed transparency rule.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteTransparency {
    /// No transparent index.
    Opaque,
    /// One transparent index.
    Index(u8),
    /// Q1 fence transparency (index 255).
    Q1Fence,
}

/// Uploaded image content: indexed, true-color, or depth.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderImage {
    /// 8-bit indexed color.
    Indexed8 {
        /// Mipmap levels, base first.
        levels: Vec<ImageLevel>,
        /// Palette.
        palette: Palette,
        /// Transparency rule.
        transparency: PaletteTransparency,
        /// Fullbright index range, inclusive.
        fullbright: Option<(u8, u8)>,
        /// Optional colormap translation table.
        translation: Option<Vec<u8>>,
    },
    /// True-color RGBA.
    Rgba8 {
        /// Mipmap levels, base first.
        levels: Vec<ImageLevel>,
        /// Border color.
        border_color: Vec4,
    },
    /// 32-bit float depth.
    Depth32f {
        /// Mipmap levels, base first.
        levels: Vec<DepthImageLevel>,
    },
}

impl RenderImage {
    /// Image encoding name for journals and diagnostics.
    #[must_use]
    pub const fn encoding(&self) -> &'static str {
        match self {
            Self::Indexed8 { .. } => "indexed8",
            Self::Rgba8 { .. } => "rgba8",
            Self::Depth32f { .. } => "depth32f",
        }
    }

    /// Number of mipmap levels.
    #[must_use]
    pub fn mip_levels(&self) -> usize {
        match self {
            Self::Indexed8 { levels, .. } | Self::Rgba8 { levels, .. } => levels.len(),
            Self::Depth32f { levels } => levels.len(),
        }
    }
}

/// Texture minification/magnification filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFilter {
    /// Nearest.
    Nearest,
    /// Linear.
    Linear,
    /// Nearest mipmap nearest.
    NearestMipmapNearest,
    /// Linear mipmap nearest.
    LinearMipmapNearest,
    /// Nearest mipmap linear.
    NearestMipmapLinear,
    /// Linear mipmap linear.
    LinearMipmapLinear,
}

impl TextureFilter {
    /// Whether the filter samples mipmaps.
    #[must_use]
    pub const fn uses_mipmap(self) -> bool {
        !matches!(self, Self::Nearest | Self::Linear)
    }

    /// Whether the filter linearly interpolates within a level.
    #[must_use]
    pub const fn is_linear(self) -> bool {
        matches!(
            self,
            Self::Linear | Self::LinearMipmapNearest | Self::LinearMipmapLinear
        )
    }
}

/// Texture wrap and filter sampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureSampling {
    /// Whether sampling repeats or clamps.
    pub repeat: bool,
    /// Minification/magnification filter.
    pub filter: TextureFilter,
}

/// Renderer lifetime that owns images and frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceOwner {
    /// Backend instance identity.
    pub identity: u64,
    /// Owning session.
    pub session: SessionId,
    /// Backend generation (bumps on replacement/resize).
    pub generation: u64,
}

impl ResourceOwner {
    /// Build an owner handle.
    #[must_use]
    pub const fn new(identity: u64, session: SessionId, generation: u64) -> Self {
        Self {
            identity,
            session,
            generation,
        }
    }

    /// Check that `other` names this same lifetime.
    pub fn require(&self, other: &Self, what: &str) -> Result<(), RenderError> {
        if self.identity == other.identity
            && self.session == other.session
            && self.generation == other.generation
        {
            Ok(())
        } else {
            Err(RenderError::ForeignOwner(what.to_string()))
        }
    }
}

/// Where a renderer image came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    /// Loaded from a content resource path.
    Resource {
        /// Requested content path.
        requested_path: String,
    },
    /// Generated by the renderer (lightmaps, cinematics, scratch).
    Generated {
        /// Generated name.
        name: String,
    },
}

impl ImageSource {
    /// Display name: generated name or resource path.
    #[must_use]
    pub fn display_name(&self) -> &str {
        match self {
            Self::Resource { requested_path } => requested_path,
            Self::Generated { name } => name,
        }
    }
}

/// A renderer image handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererImage {
    /// Owning renderer lifetime.
    pub owner: ResourceOwner,
    /// Per-owner ordinal.
    pub ordinal: u32,
    /// Image origin.
    pub source: ImageSource,
    /// Base-level width.
    pub width: u32,
    /// Base-level height.
    pub height: u32,
}

/// Image resource lifecycle operation.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageResourceOperation {
    /// Create and upload an image.
    CreateImage {
        /// Target image.
        image: RendererImage,
        /// Uploaded content.
        content: RenderImage,
        /// Sampling state.
        sampling: TextureSampling,
    },
    /// Replace one mipmap level; encoding stays as created.
    UpdateImage {
        /// Target image.
        image: RendererImage,
        /// Mipmap level.
        level: u32,
        /// Replacement pixels.
        content: LevelContent,
    },
    /// Release an image.
    ReleaseImage {
        /// Target image.
        image: RendererImage,
    },
    /// Global texture filter mode change.
    TextureMode {
        /// New filter.
        filter: TextureFilter,
    },
}

/// Replacement level pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum LevelContent {
    /// RGBA8 pixels.
    Rgba(ImageLevel),
    /// Depth pixels.
    Depth(DepthImageLevel),
}

/// Blend factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendFactor {
    /// Zero.
    Zero,
    /// One.
    One,
    /// Source color.
    SrcColor,
    /// One minus source color.
    OneMinusSrcColor,
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
}

/// Depth test function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthTest {
    /// Less or equal.
    LessEqual,
    /// Equal.
    Equal,
    /// Always pass.
    Always,
}

/// Alpha test function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaTest {
    /// No alpha test.
    None,
    /// Greater than zero.
    GreaterZero,
    /// Less than 128/255.
    Less128,
    /// Greater or equal to 128/255.
    GreaterEqual128,
}

/// Face culling mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullFace {
    /// No culling.
    None,
    /// Cull back faces.
    Back,
    /// Cull front faces.
    Front,
}

/// Polygon offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolygonOffset {
    /// Scale factor.
    pub factor: f32,
    /// Depth units.
    pub units: f32,
}

/// Per-batch pipeline state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderState {
    /// Blend factors.
    pub blend: (BlendFactor, BlendFactor),
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
    /// Opaque defaults: blending off, less-equal depth, write on.
    #[must_use]
    pub const fn opaque(cull: CullFace) -> Self {
        Self {
            blend: (BlendFactor::One, BlendFactor::Zero),
            depth_test: DepthTest::LessEqual,
            depth_write: true,
            alpha_test: AlphaTest::None,
            cull,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
        }
    }
}

/// Shader waveform shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveKind {
    /// No wave.
    None,
    /// Sine.
    Sin,
    /// Square.
    Square,
    /// Triangle.
    Triangle,
    /// Sawtooth.
    Sawtooth,
    /// Inverse sawtooth.
    InverseSawtooth,
    /// Noise.
    Noise,
}

/// Shader waveform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waveform {
    /// Shape.
    pub kind: WaveKind,
    /// Base value.
    pub base: f32,
    /// Amplitude.
    pub amplitude: f32,
    /// Phase.
    pub phase: f32,
    /// Frequency.
    pub frequency: f32,
}

/// RGB generator.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorGen {
    /// Named source generator.
    Named(ColorGenKind),
    /// Constant color.
    Const(Vec3),
    /// Waveform color.
    Wave(Waveform),
}

/// Named RGB generator kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorGenKind {
    /// Identity.
    Identity,
    /// Identity lighting.
    IdentityLighting,
    /// Entity color.
    Entity,
    /// One minus entity.
    OneMinusEntity,
    /// Vertex color.
    Vertex,
    /// Exact vertex color.
    ExactVertex,
    /// Lighting diffuse.
    LightingDiffuse,
    /// One minus vertex.
    OneMinusVertex,
}

/// Alpha generator.
#[derive(Debug, Clone, PartialEq)]
pub enum AlphaGen {
    /// Named source generator.
    Named(AlphaGenKind),
    /// Constant alpha.
    Const(f32),
    /// Waveform alpha.
    Wave(Waveform),
    /// Portal-range alpha.
    Portal(f32),
}

/// Named alpha generator kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaGenKind {
    /// Identity.
    Identity,
    /// Entity alpha.
    Entity,
    /// One minus entity.
    OneMinusEntity,
    /// Vertex alpha.
    Vertex,
    /// Lighting specular.
    LightingSpecular,
    /// One minus vertex.
    OneMinusVertex,
}

/// Texture-coordinate generator.
#[derive(Debug, Clone, PartialEq)]
pub enum TexCoordGen {
    /// Texture coordinates.
    Texture,
    /// Lightmap coordinates.
    Lightmap,
    /// Environment coordinates.
    Environment,
    /// Vector projection.
    Vector {
        /// S axis.
        s: Vec3,
        /// T axis.
        t: Vec3,
    },
}

/// Texture-coordinate modifier.
#[derive(Debug, Clone, PartialEq)]
pub enum TexCoordModifier {
    /// Scale.
    Scale(Vec2),
    /// Scroll.
    Scroll(Vec2),
    /// Stretch wave.
    Stretch(Waveform),
    /// Turbulent wave.
    Turbulent(Waveform),
    /// Rotate degrees per second.
    Rotate(f32),
    /// 2D transform.
    Transform {
        /// Row-major 2x2 matrix.
        matrix: [f32; 4],
        /// Translation.
        translation: Vec2,
    },
    /// Entity translation.
    EntityTranslate,
    /// No modification.
    None,
}

/// Shader stage image source.
#[derive(Debug, Clone, PartialEq)]
pub enum ShaderMap {
    /// Named image.
    Image {
        /// Content name.
        name: String,
        /// Clamp sampling.
        clamp: bool,
    },
    /// Lightmap.
    Lightmap,
    /// White image.
    WhiteImage,
    /// No image.
    None,
    /// Animated frames.
    Animation {
        /// Frames per second.
        frequency: f32,
        /// Frame names.
        frames: Vec<String>,
    },
    /// Video stream.
    Video {
        /// Content name.
        name: String,
    },
}

/// Original parser fields kept across repeated directives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageSourceState {
    /// Stage active.
    pub active: bool,
    /// Packed state bits.
    pub state_bits: u32,
    /// Packed rgbGen.
    pub rgb_gen: u32,
    /// Packed alphaGen.
    pub alpha_gen: u32,
    /// Packed tcGen.
    pub tc_gen: u32,
    /// RGB wave.
    pub rgb_wave: Waveform,
    /// Alpha wave.
    pub alpha_wave: Waveform,
    /// Stage is a lightmap.
    pub is_lightmap: bool,
    /// Vertex-lit lightmap.
    pub vertex_lightmap: bool,
}

/// One Q3 shader stage.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderStage {
    /// Image source.
    pub map: ShaderMap,
    /// Blend factors.
    pub blend: (BlendFactor, BlendFactor),
    /// Depth test.
    pub depth_test: DepthTest,
    /// Depth write.
    pub depth_write: bool,
    /// Alpha test.
    pub alpha_test: AlphaTest,
    /// Detail stage.
    pub detail: bool,
    /// RGB generator.
    pub color: ColorGen,
    /// Alpha generator.
    pub alpha: AlphaGen,
    /// Coordinate generator.
    pub coordinates: TexCoordGen,
    /// Coordinate modifiers.
    pub modifiers: Vec<TexCoordModifier>,
    /// Original parser fields.
    pub source_state: StageSourceState,
}

/// Vertex deformation.
#[derive(Debug, Clone, PartialEq)]
pub enum VertexDeformation {
    /// Projection shadow.
    ProjectionShadow,
    /// Autosprite.
    Autosprite,
    /// Autosprite2.
    Autosprite2,
    /// No deformation.
    None,
    /// Text deformation.
    Text(u32),
    /// Wave deformation.
    Wave {
        /// Spread.
        spread: f32,
        /// Wave.
        wave: Waveform,
    },
    /// Normal deformation.
    Normal {
        /// Amplitude.
        amplitude: f32,
        /// Frequency.
        frequency: f32,
    },
    /// Move deformation.
    Move {
        /// Direction.
        direction: Vec3,
        /// Wave.
        wave: Waveform,
    },
    /// Bulge deformation.
    Bulge {
        /// Width.
        width: f32,
        /// Height.
        height: f32,
        /// Speed.
        speed: f32,
    },
}

/// Surface lighting source.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceLighting {
    /// Unlit.
    Unlit,
    /// Vertex-lit.
    Vertex,
    /// Lightmapped.
    Lightmap {
        /// Lightmap image.
        image: RendererImage,
        /// Light styles.
        styles: Vec<u32>,
    },
}

/// Q1 surface kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1Surface {
    /// Ordinary.
    Ordinary,
    /// Sky.
    Sky,
    /// Water.
    Water,
    /// Slime.
    Slime,
    /// Lava.
    Lava,
    /// Teleport.
    Teleport,
    /// Fence.
    Fence,
}

/// One Q1 texture-animation frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1AnimFrame {
    /// Frame image.
    pub image: RendererImage,
    /// Start time in tenths of a second.
    pub start_tenths: u32,
    /// End time in tenths of a second.
    pub end_tenths: u32,
}

/// A render material for one source family.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderMaterial {
    /// Q1 mipmap material.
    Q1 {
        /// Material name.
        name: String,
        /// Base texture.
        texture: RendererImage,
        /// Lighting source.
        lighting: SurfaceLighting,
        /// Primary animation.
        animation: Vec<Q1AnimFrame>,
        /// Alternate animation.
        alternate_animation: Vec<Q1AnimFrame>,
        /// Surface kind.
        surface: Q1Surface,
        /// Opacity.
        alpha: f32,
    },
    /// Q2 material.
    Q2 {
        /// Material name.
        name: String,
        /// Animation frames.
        frames: Vec<RendererImage>,
        /// Lighting source.
        lighting: SurfaceLighting,
        /// Surface flags.
        surface_flags: u32,
        /// Material name for footsteps.
        material: String,
        /// Flowing scroll.
        flowing: bool,
        /// Turbulent warp.
        warp: bool,
        /// Opacity.
        alpha: f32,
    },
    /// Q3 shader.
    Q3 {
        /// Shader name.
        name: String,
        /// Stages.
        stages: Vec<ShaderStage>,
        /// Vertex deformations.
        deformations: Vec<VertexDeformation>,
        /// Surface parameters.
        surface_parameters: Vec<String>,
        /// Explicit sort rank.
        sort: Option<i32>,
        /// Face culling.
        cull: CullFace,
        /// Polygon offset enabled.
        polygon_offset: bool,
        /// No mipmaps.
        no_mipmaps: bool,
        /// No picmip.
        no_picmip: bool,
        /// Entity-mergeable.
        entity_mergeable: bool,
        /// Portal range.
        portal_range: f32,
        /// Clamp time.
        clamp_time: f32,
        /// Sky parameters.
        sky: Option<SkyParams>,
        /// Fog parameters.
        fog: Option<ShaderFogParams>,
        /// Sun parameters.
        sun: Option<SunParams>,
    },
}

/// Q3 sky shader parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyParams {
    /// Outer box name.
    pub outer_box: Option<String>,
    /// Inner box name.
    pub inner_box: Option<String>,
    /// Cloud height.
    pub cloud_height: f32,
}

/// Q3 fog shader parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderFogParams {
    /// Fog color.
    pub color: Vec3,
    /// Depth for opaque.
    pub depth_for_opaque: f32,
}

/// Q3 sun shader parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SunParams {
    /// Sun color.
    pub color: Vec3,
    /// Intensity.
    pub intensity: f32,
    /// Azimuth degrees.
    pub azimuth: f32,
    /// Elevation degrees.
    pub elevation: f32,
}

/// One projected batch vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderVertex {
    /// Clip-space position.
    pub position: Vec4,
    /// Texture coordinates.
    pub tex_coord: Vec2,
    /// Vertex color.
    pub color: Vec4,
}

/// One projected dual-textured batch vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MultitextureVertex {
    /// Base vertex.
    pub base: RenderVertex,
    /// Second texture coordinates.
    pub tex_coord2: Vec2,
}

/// Lazily uploaded image resolved at draw time.
pub trait DynamicImageSource: Send + Sync {
    /// Upload (via `apply`) and return the image to bind.
    fn resolve(&self, apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage;
}

/// Texture binding for one unit.
#[derive(Clone)]
pub enum TextureBinding {
    /// Resolve a dynamic image, then bind it.
    DynamicImage(Arc<dyn DynamicImageSource>),
    /// Bind a resident image.
    BindImage(RendererImage),
    /// Keep the currently bound texture.
    RetainCurrentTexture,
}

impl std::fmt::Debug for TextureBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DynamicImage(_) => f.write_str("DynamicImage(..)"),
            Self::BindImage(image) => f.debug_tuple("BindImage").field(image).finish(),
            Self::RetainCurrentTexture => f.write_str("RetainCurrentTexture"),
        }
    }
}

impl PartialEq for TextureBinding {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::BindImage(a), Self::BindImage(b)) => a == b,
            (Self::RetainCurrentTexture, Self::RetainCurrentTexture) => true,
            (Self::DynamicImage(a), Self::DynamicImage(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// Second-texture bundle plus its combine environment.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureBundle {
    /// Binding.
    pub binding: TextureBinding,
    /// Combine environment.
    pub environment: PairEnvironment,
}

/// Dual-texture combine environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairEnvironment {
    /// Modulate.
    Modulate,
    /// Add.
    Add,
    /// Replace.
    Replace,
}

/// Q2 shadow projection. Atlas rectangles are normalized xy origins with zw
/// sizes, as in the donor `gl_shader.ts`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q2ShadowProjection {
    /// No shadow.
    None,
    /// Spot-cone projection.
    Cone {
        /// Shadow matrix.
        matrix: Mat4,
        /// Atlas rectangle.
        atlas_rect: Vec4,
    },
    /// Point (cube) projection.
    Point {
        /// Atlas rectangle.
        atlas_rect: Vec4,
    },
}

/// Q2 shadow depth atlas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ShadowAtlas {
    /// Atlas image.
    pub image: RendererImage,
    /// Normalized texel size.
    pub texel_size: f32,
    /// Shadow near plane.
    pub near_plane: f32,
}

/// Q2 fragment light. A negative red channel retains the fullbright sentinel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2FragmentLight {
    /// World origin.
    pub origin: Vec3,
    /// Radius in world units.
    pub radius: f32,
    /// Light color.
    pub color: Vec3,
    /// Intensity scale.
    pub scale: f32,
    /// Spot cone.
    pub cone: Option<Q2LightCone>,
    /// Shadow projection.
    pub shadow: Q2ShadowProjection,
}

/// Q2 spot-cone parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2LightCone {
    /// Cone direction.
    pub direction: Vec3,
    /// Cosine of the half angle.
    pub cos_half_angle: f32,
}

/// Q2 model shadow light with a precomputed shadow fraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ModelShadowLight {
    /// World origin.
    pub origin: Vec3,
    /// Radius in world units.
    pub radius: f32,
    /// Shadow fraction.
    pub fraction: Vec3,
    /// Shadow projection.
    pub shadow: Q2ShadowProjection,
}

/// Per-batch lighting. World-space arrays share vertex indices with the
/// batch and survive clipping.
#[derive(Debug, Clone, PartialEq)]
pub enum BatchLighting {
    /// Vertex colors only.
    Vertex,
    /// Q2 world lighting.
    Q2World {
        /// World positions per vertex.
        world_positions: Vec<Vec3>,
        /// World normals per vertex.
        normals: Vec<Vec3>,
        /// Shadow atlas.
        atlas: Option<Q2ShadowAtlas>,
        /// Lighting pass.
        pass: Q2LightPass,
    },
    /// Q2 model shadow pass.
    Q2ModelShadow {
        /// World positions per vertex.
        world_positions: Vec<Vec3>,
        /// Shadow lights.
        lights: Vec<Q2ModelShadowLight>,
        /// Shade scale.
        shade_scale: f32,
        /// Shadow atlas.
        atlas: Q2ShadowAtlas,
    },
}

/// Q2 world lighting pass.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2LightPass {
    /// Lightmap pass.
    Lightmap {
        /// Fragment lights.
        lights: Vec<Q2FragmentLight>,
    },
    /// Texture pass.
    Texture {
        /// Fragment lights.
        lights: Vec<Q2FragmentLight>,
    },
    /// Material lightmap pass.
    MaterialLightmap {
        /// Fragment lights.
        lights: Vec<Q2FragmentLight>,
    },
    /// Model pass.
    Model {
        /// Fragment lights with shadow fractions.
        lights: Vec<Q2ModelFragmentLight>,
        /// Shade scale.
        shade_scale: Option<f32>,
    },
}

/// Q2 fragment light with a model shadow fraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ModelFragmentLight {
    /// Base light.
    pub light: Q2FragmentLight,
    /// Shadow fraction.
    pub fraction: Vec3,
}

/// Per-batch fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BatchFog {
    /// Exponential-squared fog.
    Exp2 {
        /// Fog color.
        color: Vec3,
        /// Density.
        density: f32,
        /// Channel effect.
        effect: FogEffect,
    },
    /// Constant fog blend.
    Constant {
        /// Fog color.
        color: Vec3,
        /// Blend amount.
        amount: f32,
    },
}

/// Fog channel effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FogEffect {
    /// Fog color.
    #[default]
    Color,
    /// No fog.
    None,
    /// RGB only.
    Rgb,
    /// Alpha only.
    Alpha,
    /// RGBA.
    Rgba,
    /// Overlay.
    Overlay,
}

/// Batch primitive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BatchPrimitive {
    /// Triangle list.
    Triangles,
    /// Line list with a line width.
    Lines {
        /// Line width in pixels.
        line_width: f32,
    },
}

/// One ordered draw batch. Lightstyles, palettes, and shader time are
/// resolved before either backend draws.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawBatch {
    /// Optional fragment fog.
    pub fog: Option<BatchFog>,
    /// Luminance-alpha texture effect.
    pub luminance_alpha: bool,
    /// Indices into `vertices`.
    pub indices: Vec<u32>,
    /// Primary texture binding.
    pub texture: TextureBinding,
    /// Pipeline state.
    pub state: RenderState,
    /// Lighting parameters.
    pub lighting: BatchLighting,
    /// Primitive type.
    pub primitive: BatchPrimitive,
    /// Vertex payload.
    pub vertices: BatchVertices,
}

/// Batch vertex payload.
#[derive(Clone, Debug, PartialEq)]
pub enum BatchVertices {
    /// Single-textured vertices.
    Single(Vec<RenderVertex>),
    /// Dual-textured vertices plus the second bundle.
    Pair {
        /// Vertices.
        vertices: Vec<MultitextureVertex>,
        /// Second texture.
        second_texture: TextureBundle,
    },
}

/// Float rectangle: viewports, pictures, atlas passes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// Texture-coordinate rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureRect {
    /// Left S.
    pub s1: f32,
    /// Top T.
    pub t1: f32,
    /// Right S.
    pub s2: f32,
    /// Bottom T.
    pub t2: f32,
}

/// Portal clipping mode for a render camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewClip {
    /// No portal clipping.
    None,
    /// Portal plane clipping.
    Portal {
        /// Portal plane.
        plane: Plane,
        /// Whether the portal mirrors.
        mirror: bool,
    },
}

/// Ordered-pipeline camera (donor `SceneCamera`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderCamera {
    /// Camera origin.
    pub origin: Vec3,
    /// Camera basis (forward, left, up).
    pub axis: Axis,
    /// Projection matrix.
    pub projection: Mat4,
    /// Viewport rectangle.
    pub viewport: Rect,
    /// Clipping mode.
    pub clip: ViewClip,
}

/// Scene fog volume.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneFog {
    /// No fog.
    None,
    /// Q1 depth fog.
    Q1 {
        /// Fog color.
        color: Vec3,
        /// Density.
        density: f32,
        /// Sky blend factor.
        sky_factor: f32,
    },
    /// Q2 global plus height fog.
    Q2(Q2Fog),
    /// Q3 fog volume.
    Q3 {
        /// Fog color.
        color: Vec3,
        /// Depth for opaque.
        depth_for_opaque: f32,
        /// Fog plane.
        plane: Option<Plane>,
    },
}

/// Q2 fog parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2Fog {
    /// Fog color.
    pub color: Vec3,
    /// Density.
    pub density: f32,
    /// Sky blend factor.
    pub sky_factor: f32,
    /// Height fog.
    pub height: Q2HeightFog,
}

/// Q2 height fog parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2HeightFog {
    /// Fog color/distance at the start.
    pub start: Q2HeightStop,
    /// Fog color/distance at the end.
    pub end: Q2HeightStop,
    /// Density.
    pub density: f32,
    /// Falloff.
    pub falloff: f32,
}

/// One Q2 height-fog gradient stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2HeightStop {
    /// Stop color.
    pub color: Vec3,
    /// Stop distance.
    pub distance: f32,
}

/// One depth-atlas draw.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthAtlasDraw {
    /// Clip-space positions.
    pub positions: Vec<Vec4>,
    /// Indices.
    pub indices: Vec<u32>,
    /// Face culling.
    pub cull: CullFace,
    /// Polygon offset.
    pub polygon_offset: Option<PolygonOffset>,
}

/// One depth-atlas pass.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthAtlasPass {
    /// Pass viewport.
    pub viewport: Rect,
    /// Optional depth clear value.
    pub clear_depth: Option<f32>,
    /// Draws.
    pub draws: Vec<DepthAtlasDraw>,
}

/// Q2 fog operation. Runs once after scene lighting and transparency,
/// before screen blends. Far depth includes the 1e-6 sky threshold;
/// `sky_drawn` distinguishes a sky view from an empty view.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FogOperation {
    /// Fog camera (source symmetric perspective projection).
    pub camera: RenderCamera,
    /// Fog parameters.
    pub fog: Q2Fog,
    /// Far-depth sky threshold.
    pub far_depth: f32,
    /// Whether sky was drawn.
    pub sky_drawn: bool,
}

/// Positions plus texture coordinates for one sky strip vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyVertex {
    /// Clip-space position.
    pub position: Vec4,
    /// Texture coordinates.
    pub tex_coord: Vec2,
}

/// One ordered render operation.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderOperation {
    /// Draw batches in order.
    Draw(Vec<DrawBatch>),
    /// Draw with whole-object opacity against the current backdrop.
    ObjectOpacity {
        /// Opacity in 0..=1.
        opacity: f32,
        /// Batches.
        batches: Vec<DrawBatch>,
    },
    /// Q2 fog pass.
    Q2Fog(Q2FogOperation),
    /// Bind a depth32f image, run passes, restore target and viewport.
    DepthAtlas {
        /// Atlas image.
        image: RendererImage,
        /// Passes.
        passes: Vec<DepthAtlasPass>,
    },
    /// Set the depth range.
    DepthRange([f32; 2]),
    /// Set face culling.
    Cull(CullFace),
    /// Set polygon offset.
    PolygonOffset(Option<PolygonOffset>),
    /// Disable portal clipping.
    DisablePortalClip,
    /// Draw one sky side.
    SkySide {
        /// Sky image.
        image: RendererImage,
        /// Sky color.
        color: Vec4,
        /// Triangle strips.
        strips: Vec<Vec<SkyVertex>>,
    },
    /// Stencil shadow volume.
    ShadowVolume {
        /// Clip-space positions.
        positions: Vec<Vec4>,
        /// Indices.
        indices: Vec<u32>,
        /// Mirrored (portal) view.
        mirror: bool,
        /// White image.
        white_image: RendererImage,
    },
    /// Shadow-volume finish quad.
    ShadowFinish {
        /// Quad corners.
        positions: [Vec4; 4],
        /// White image.
        white_image: RendererImage,
    },
}

/// Viewport, clear, and clip state shared by every view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderViewState {
    /// Viewport rectangle.
    pub viewport: Rect,
    /// Clear values.
    pub clear: Option<ViewClear>,
    /// Clip plane.
    pub clip_plane: Option<Vec4>,
}

/// View clear values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewClear {
    /// Depth clear value.
    pub depth: f32,
    /// Optional color clear.
    pub color: Option<Vec4>,
    /// Clear stencil.
    pub stencil: bool,
}

/// Source clock time; units stay explicit at provider boundaries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceTime {
    /// Seconds.
    Seconds(f64),
    /// Milliseconds.
    Milliseconds(f64),
}

impl SourceTime {
    /// Time in seconds.
    #[must_use]
    pub fn as_seconds(self) -> f64 {
        match self {
            Self::Seconds(value) | Self::Milliseconds(value) => {
                if matches!(self, Self::Seconds(_)) {
                    value
                } else {
                    value / 1000.0
                }
            }
        }
    }

    /// Time in milliseconds.
    #[must_use]
    pub fn as_milliseconds(self) -> f64 {
        match self {
            Self::Seconds(value) => value * 1000.0,
            Self::Milliseconds(value) => value,
        }
    }
}

/// Where a view draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewTarget {
    /// A local seat.
    Seat(SeatId),
    /// A preview surface.
    Preview(String),
}

/// One ordered view: state plus before/after operations.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderView {
    /// Shared view state.
    pub state: RenderViewState,
    /// Draw target.
    pub target: ViewTarget,
    /// View time.
    pub time: SourceTime,
    /// Operations before the view begins.
    pub before_view: Vec<RenderOperation>,
    /// Operations inside the view.
    pub operations: Vec<RenderOperation>,
}

/// Draw buffer selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawBuffer {
    /// Front buffer.
    Front,
    /// Back buffer.
    Back,
    /// Stereo back-left.
    BackLeft,
    /// Stereo back-right.
    BackRight,
}

/// One ordered render command.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderCommand {
    /// Select the draw buffer.
    DrawBuffer {
        /// Buffer.
        buffer: DrawBuffer,
        /// Clear after selecting.
        clear: bool,
    },
    /// Image resource operation.
    ImageResource(ImageResourceOperation),
    /// Set the 2D drawing color.
    SetColor(Vec4),
    /// Stretch a picture.
    StretchPic {
        /// Destination rectangle.
        rect: Rect,
        /// Source coordinates.
        uv: TextureRect,
        /// Picture image.
        image: RendererImage,
    },
    /// Render a view.
    View(RenderView),
    /// Swap front/back buffers.
    SwapBuffers,
}

/// One ordered frame.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderFrame {
    /// Owning renderer lifetime.
    pub owner: ResourceOwner,
    /// Frame sequence number.
    pub sequence: u64,
    /// Commands in order.
    pub commands: Vec<RenderCommand>,
}

/// A prepared draw: begin, bind, draw, release.
pub trait PreparedDraw {
    /// Begin the draw.
    fn begin(&mut self);
    /// Bind a texture unit.
    fn apply_texture(&mut self, unit: u32, binding: &TextureBinding);
    /// Issue the draw.
    fn draw(&mut self);
    /// Release transient state.
    fn cleanup(&mut self);
}

/// CPU and GL implementations consume the same ordered operations
/// synchronously (donor `RendererBackend`).
pub trait OrderedBackend {
    /// Prepared-draw handle type.
    type Prepared<'a>: PreparedDraw
    where
        Self: 'a;

    /// Owning renderer lifetime.
    fn owner(&self) -> &ResourceOwner;
    /// Framebuffer width.
    fn width(&self) -> u32;
    /// Framebuffer height.
    fn height(&self) -> u32;
    /// Stencil bits.
    fn stencil_bits(&self) -> u32;
    /// Apply an image resource operation.
    fn apply_image_resource(&mut self, operation: &ImageResourceOperation);
    /// Select the draw buffer.
    fn select_draw_buffer(&mut self, buffer: DrawBuffer, clear: bool);
    /// Enable or disable overdraw measurement.
    fn set_overdraw_measurement(&mut self, enabled: bool);
    /// Read the stencil overdraw buffer.
    fn read_stencil_overdraw(&self, destination: &mut [u8]);
    /// Read one depth pixel in window coordinates.
    fn read_depth_pixel(&self, window_x: i32, window_y: i32) -> f32;
    /// Begin a view.
    fn begin_view(&mut self, view: &RenderViewState);
    /// Draw with partial object opacity. Partial opacity commits color
    /// only; zero skips the draw, one preserves direct rendering.
    fn with_object_opacity(&mut self, opacity: f32, draw: impl FnOnce(&mut Self));
    /// Draw one immediate (non-batch) operation.
    fn draw_immediate(&mut self, operation: &RenderOperation);
    /// Prepare one batch for drawing.
    fn prepare_geometry(&mut self, batch: &DrawBatch) -> Self::Prepared<'_>;
    /// Draw one batch directly.
    fn draw_batch(&mut self, batch: &DrawBatch) {
        let mut prepared = self.prepare_geometry(batch);
        prepared.begin();
        prepared.apply_texture(0, &batch.texture);
        if let BatchVertices::Pair { second_texture, .. } = &batch.vertices {
            prepared.apply_texture(1, &second_texture.binding);
        }
        prepared.draw();
        prepared.cleanup();
    }
    /// Clear the color buffer.
    fn clear_color_buffer(&mut self);
    /// Draw a debug image rectangle.
    fn draw_show_image(&mut self, image: &RendererImage, rect: &Rect, proportional: bool);
    /// Finish the frame.
    fn finish(&mut self);
    /// Release backend resources. Idempotent.
    fn close(&mut self);
}
