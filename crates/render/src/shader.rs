//! Load-only shader definitions for the one shared material table.
//! Native grammar: qsrc quake-iii-arena/code/renderer/tr_shader.c.
mod lexer;
mod parse;
pub use crate::assets::DepthFunc;
pub use parse::parse_sources;
pub use qa_core::names::canonical_path;

/// Native shader lookup strips at the first dot, before folded name lookup.
/// Separators remain exact bytes; path normalization belongs to image lookup.
pub fn registration_key(raw: &[u8]) -> &[u8] {
    &raw[..raw
        .iter()
        .position(|&byte| byte == b'.')
        .unwrap_or(raw.len())]
}

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
    pub ops: Box<[ShaderOp]>,
}
impl ShaderDef {
    pub fn has_unsupported_runtime(&self) -> bool {
        self.ops.iter().any(|op| {
            matches!(&op.action,
            ShaderAction::Header(HeaderOp::Unsupported(declaration))
            | ShaderAction::Stage(StageOp::Unsupported(declaration))
            if declaration.effect == DeclarationEffect::Runtime)
        })
    }
    pub fn is_complete(&self) -> bool {
        self.ops
            .last()
            .is_some_and(|op| matches!(op.action, ShaderAction::EndShader))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderOp {
    pub line: usize,
    pub action: ShaderAction,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ShaderAction {
    Header(HeaderOp),
    BeginStage,
    Stage(StageOp),
    EndStage,
    EndShader,
    Abort(ShaderFailure),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderFailure {
    Native,
    Limit(LimitKind),
    InvalidData(&'static str),
}
#[derive(Clone, Debug, PartialEq)]
pub enum HeaderOp {
    Cull(Cull),
    Sort(f32),
    SurfaceParm { surface: u32, contents: u32 },
    SkyOuter(Option<String>),
    SkyHeight(f32),
    SkyInner(Option<String>),
    FogColor([Option<f32>; 3]),
    FogDepth(f32),
    Sun(SunParms),
    NoMipmaps,
    NoPicmip,
    PolygonOffset,
    EntityMergable,
    Portal,
    ClampTime(f32),
    Deform(Option<Deform>),
    Unsupported(UnsupportedDeclaration),
}
#[derive(Clone, Debug, PartialEq)]
pub enum ShaderMap {
    Image { name: String, clamp: bool },
    White,
    Lightmap,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaveUpdate {
    pub function: Option<WaveFunction>,
    pub values: [Option<f32>; 4],
}
#[derive(Clone, Debug, PartialEq)]
pub enum StageOp {
    Map(ShaderMap),
    AnimationSpeed(f32),
    AnimationFrame(String),
    BlendSource(BlendFactor),
    BlendDestination(BlendFactor),
    BlendComplete,
    RgbGen(RgbGen),
    RgbWave(WaveUpdate),
    AlphaGen(AlphaGen),
    AlphaWave(WaveUpdate),
    TcGen(TexCoordGen),
    TcVector([[Option<f32>; 3]; 2]),
    TexMod(Option<TexMod>),
    AlphaFunc(AlphaFunc),
    DepthFunc(DepthFunc),
    DepthWrite,
    Detail,
    Unsupported(UnsupportedDeclaration),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cull {
    #[default]
    Front,
    Back,
    None,
}
impl Cull {
    pub fn for_view(self, reflected: bool) -> Self {
        if reflected {
            match self {
                Self::Front => Self::Back,
                Self::Back => Self::Front,
                Self::None => Self::None,
            }
        } else {
            self
        }
    }
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
impl SunParms {
    pub fn direction(self) -> qa_core::primitives::Vec3 {
        // ParseShader divides in float, multiplies by double M_PI, stores float
        // radians, then multiplies double trig results before the final store.
        let [azimuth, elevation] = [self.azimuth_degrees, self.elevation_degrees]
            .map(|degrees| f64::from((f64::from(degrees / 180.) * std::f64::consts::PI) as f32));
        qa_core::primitives::Vec3([
            (azimuth.cos() * elevation.cos()) as f32,
            (azimuth.sin() * elevation.cos()) as f32,
            elevation.sin() as f32,
        ])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageBlend {
    pub source: BlendFactor,
    pub destination: BlendFactor,
}
impl StageBlend {
    pub const ALPHA: Self = Self {
        source: BlendFactor::SourceAlpha,
        destination: BlendFactor::OneMinusSourceAlpha,
    };
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
