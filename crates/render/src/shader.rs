//! Load-only shader definitions for the one shared material table.
//! Native grammar: qsrc quake-iii-arena/code/renderer/tr_shader.c.
mod lexer;
mod parse;
pub use crate::assets::DepthFunc;
pub use parse::parse_sources;
pub use qa_core::names::canonical_path;

pub const MAX_STAGES: usize = 8;
pub const MAX_TEXMODS: usize = 4;
pub const MAX_ANIMATIONS: usize = 8;
pub const MAX_DEFORMS: usize = 3;

#[derive(Clone, Copy)]
pub struct ShaderSource<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}
pub struct ShaderCatalog {
    /// Sorted canonical names. Registration resolves these to numeric handles.
    pub definitions: Box<[ShaderDef]>,
    pub diagnostics: Box<[Diagnostic]>,
}
impl ShaderCatalog {
    pub fn find_canonical(&self, name: &str) -> Option<&ShaderDef> {
        self.definitions
            .binary_search_by(|definition| definition.name.as_str().cmp(name))
            .ok()
            .map(|index| &self.definitions[index])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShaderDef {
    pub name: String,
    pub source: String,
    pub line: usize,
    /// False means the native grammar/limits were not satisfied. Keep the
    /// definition so a caller can report fallback instead of hiding the failure.
    pub valid: bool,
    pub cull: Cull,
    pub sort: f32,
    pub sort_explicit: bool,
    pub surface_flags: u32,
    pub content_flags: u32,
    pub sky: Option<SkyParms>,
    pub fog: Option<FogParms>,
    pub sun: Option<SunParms>,
    pub polygon_offset: bool,
    pub no_mipmaps: bool,
    pub no_picmip: bool,
    pub entity_mergable: bool,
    pub portal: bool,
    pub clamp_time: Option<f32>,
    pub stages: Box<[ShaderStage]>,
    pub deforms: Box<[Deform]>,
    pub unsupported: Box<[UnsupportedDeclaration]>,
}
impl ShaderDef {
    pub fn has_unsupported_runtime(&self) -> bool {
        self.unsupported
            .iter()
            .chain(
                self.stages
                    .iter()
                    .flat_map(|stage| stage.unsupported.iter()),
            )
            .any(|declaration| declaration.effect != DeclarationEffect::CompileOnly)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cull {
    #[default]
    Front,
    Back,
    None,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SkyParms {
    pub outer_box: Option<String>,
    pub cloud_height: f32,
    pub inner_box: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogParms {
    pub color: [f32; 3],
    pub depth_opaque: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunParms {
    pub color: [f32; 3],
    pub intensity: f32,
    pub azimuth_degrees: f32,
    pub elevation_degrees: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderStage {
    pub map: Option<TextureMap>,
    /// None is native opaque GL_ONE/GL_ZERO with blending disabled.
    pub blend: Option<StageBlend>,
    pub rgb_gen: RgbGen,
    pub alpha_gen: AlphaGen,
    pub tc_gen: TexCoordGen,
    pub tc_mods: Box<[TexMod]>,
    pub alpha_func: AlphaFunc,
    pub depth_func: DepthFunc,
    pub depth_write: bool,
    pub detail: bool,
    pub unsupported: Box<[UnsupportedDeclaration]>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TextureMap {
    Image {
        name: String,
        clamp: bool,
    },
    White,
    Lightmap,
    Animation {
        frequency: f32,
        images: Box<[String]>,
    },
    Video(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageBlend {
    pub source: BlendFactor,
    pub destination: BlendFactor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum BlendFactor {
    Zero = 0,
    One = 1,
    SourceColor = 0x0300,
    OneMinusSourceColor = 0x0301,
    SourceAlpha = 0x0302,
    OneMinusSourceAlpha = 0x0303,
    DestinationAlpha = 0x0304,
    OneMinusDestinationAlpha = 0x0305,
    DestinationColor = 0x0306,
    OneMinusDestinationColor = 0x0307,
    SourceAlphaSaturate = 0x0308,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RgbGen {
    Identity,
    IdentityLighting,
    Entity,
    OneMinusEntity,
    Vertex,
    ExactVertex,
    OneMinusVertex,
    LightingDiffuse,
    Wave(Waveform),
    Const([f32; 3]),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlphaGen {
    Identity,
    Skip,
    Entity,
    OneMinusEntity,
    Vertex,
    OneMinusVertex,
    LightingSpecular,
    Portal(f32),
    Wave(Waveform),
    Const(f32),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TexCoordGen {
    Texture,
    Lightmap,
    Environment,
    Vector([[f32; 3]; 2]),
    LayeredSky {
        flatten_z: f32,
        projected_scale: f32,
        texture_size: f32,
        scroll_speed: f32,
    },
    CloudSky {
        radius: f32,
        height: f32,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaFunc {
    #[default]
    None,
    GreaterZero,
    LessThanHalf,
    AtLeastHalf,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveFunction {
    Sin,
    Square,
    Triangle,
    Sawtooth,
    InverseSawtooth,
    Noise,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waveform {
    pub function: WaveFunction,
    pub base: f32,
    pub amplitude: f32,
    pub phase: f32,
    pub frequency: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deform {
    Wave {
        /// Native deformationSpread is the reciprocal of the script divisor.
        spread: f32,
        wave: Waveform,
    },
    Move {
        vector: [f32; 3],
        wave: Waveform,
    },
    Bulge {
        width: f32,
        height: f32,
        speed: f32,
    },
    /// Native ParseDeform reads amplitude first, then frequency.
    Normal {
        amplitude: f32,
        frequency: f32,
    },
    AutoSprite,
    AutoSprite2,
    ProjectionShadow,
    Text(u8),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TexMod {
    Transform {
        matrix: [[f32; 2]; 2],
        translate: [f32; 2],
    },
    Scale([f32; 2]),
    Scroll([f32; 2]),
    Rotate(f32),
    Stretch(Waveform),
    Turbulent {
        base: f32,
        amplitude: f32,
        phase: f32,
        frequency: f32,
    },
    EntityTranslate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationEffect {
    /// qer/q3map annotations and other declarations qsrc deliberately ignores.
    CompileOnly,
    /// Recognised native runtime behavior not yet converted by this parser.
    Runtime,
    /// qsrc rejects this keyword instead of inventing a runtime effect.
    Unknown,
}
#[derive(Clone, Debug, PartialEq)]
pub struct UnsupportedDeclaration {
    pub keyword: String,
    pub arguments: Box<[String]>,
    pub line: usize,
    pub effect: DeclarationEffect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    Stages,
    TexMods,
    AnimationFrames,
    Deforms,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
    InvalidUtf8,
    EmbeddedNul,
    UnterminatedComment,
    UnterminatedQuote,
    TokenTooLong,
    Expected {
        expected: &'static str,
        found: String,
    },
    MissingArgument(String),
    InvalidNumber(String),
    NativeFallback {
        directive: String,
        value: String,
    },
    Unsupported(String),
    LimitExceeded(LimitKind),
    Duplicate {
        winner_source: String,
    },
    NoStages,
    MissingImage,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: String,
    pub shader: Option<String>,
    pub line: usize,
    pub severity: Severity,
    pub kind: DiagnosticKind,
}
