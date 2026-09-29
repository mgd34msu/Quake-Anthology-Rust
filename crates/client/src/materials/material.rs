//! Q3 shader parsing and texture evaluation (`tr_shader.c`).
//!
//! Donor provenance: `src/materials/material.ts` (translated from
//! `renderer/tr_shader.c`, `tr_shade_calc.c`, `tr_init.c`). Covers shader
//! script parsing (`ParseShader`/`ParseStage`), shader-name lookup
//! (`COM_StripExtension`, `Q_stricmp`, `generateHashValue`), waveform
//! evaluation (`EvalWaveForm`), and texture-coordinate evaluation
//! (`RB_CalcTexCoords` family).
//!
//! Registration against an image host lives in
//! [`crate::materials::compile`]; this module parses scripts into
//! [`ShaderDefinition`] values. The donor's async generator plumbing is
//! folded into a direct recursive parser: observable behavior (parsed
//! definitions, warnings, error sites and messages, request order) is
//! preserved. Floats use `f32` storage with the donor's operation order.

use qa_content::md3::renderer_sine;
use qa_core::math::{dot3, normalize3, scale3, sub3, Vec2, Vec3};
use qa_core::numeric::native_atof;

use super::state::bits as state_bits;
use super::state::{
    source_state_bits, AlphaTest, Blend, BlendFactor, CullFace, DepthTest, PolygonMode, RenderState, SourceStateInput,
    ADDITIVE_BLEND, FILTER_BLEND, OPAQUE_BLEND,
};
use crate::ClientError;

/// Waveform kind (`genFunc_t` names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveKind {
    /// None (invalid for evaluation).
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
    /// Noise (needs the renderer noise context).
    Noise,
}

/// Waveform (`Waveform`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waveform {
    /// Wave kind.
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

impl Waveform {
    /// Zero waveform of a kind.
    #[must_use]
    pub const fn zero(kind: WaveKind) -> Self {
        Self {
            kind,
            base: 0.0,
            amplitude: 0.0,
            phase: 0.0,
            frequency: 0.0,
        }
    }
}

/// Numeric wave function (`SourceWaveFunction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SourceWaveFunc {
    /// None.
    None = 0,
    /// Sine.
    Sin = 1,
    /// Square.
    Square = 2,
    /// Triangle.
    Triangle = 3,
    /// Sawtooth.
    Sawtooth = 4,
    /// Inverse sawtooth.
    InverseSawtooth = 5,
    /// Noise.
    Noise = 6,
}

impl SourceWaveFunc {
    /// Convert a waveform kind.
    #[must_use]
    pub const fn of(kind: WaveKind) -> Self {
        match kind {
            WaveKind::None => Self::None,
            WaveKind::Sin => Self::Sin,
            WaveKind::Square => Self::Square,
            WaveKind::Triangle => Self::Triangle,
            WaveKind::Sawtooth => Self::Sawtooth,
            WaveKind::InverseSawtooth => Self::InverseSawtooth,
            WaveKind::Noise => Self::Noise,
        }
    }
}

/// Numeric color generator (`SourceColorGenerator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SourceColorGen {
    /// Bad/uninitialized.
    Bad = 0,
    /// Identity lighting.
    IdentityLighting = 1,
    /// Identity.
    Identity = 2,
    /// Entity.
    Entity = 3,
    /// One minus entity.
    OneMinusEntity = 4,
    /// Exact vertex.
    ExactVertex = 5,
    /// Vertex.
    Vertex = 6,
    /// One minus vertex.
    OneMinusVertex = 7,
    /// Waveform.
    Waveform = 8,
    /// Lighting diffuse.
    LightingDiffuse = 9,
    /// Fog.
    Fog = 10,
    /// Constant.
    Const = 11,
}

/// Numeric alpha generator (`SourceAlphaGenerator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SourceAlphaGen {
    /// Identity.
    Identity = 0,
    /// Skip.
    Skip = 1,
    /// Entity.
    Entity = 2,
    /// One minus entity.
    OneMinusEntity = 3,
    /// Vertex.
    Vertex = 4,
    /// One minus vertex.
    OneMinusVertex = 5,
    /// Lighting specular.
    LightingSpecular = 6,
    /// Waveform.
    Waveform = 7,
    /// Portal.
    Portal = 8,
    /// Constant.
    Const = 9,
}

/// Numeric texture-coordinate generator (`SourceTexCoordGenerator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SourceTcGen {
    /// Bad/uninitialized.
    Bad = 0,
    /// Identity.
    Identity = 1,
    /// Lightmap.
    Lightmap = 2,
    /// Texture.
    Texture = 3,
    /// Environment mapped.
    EnvironmentMapped = 4,
    /// Fog.
    Fog = 5,
    /// Vector.
    Vector = 6,
}

/// Actual `waveForm_t` storage (`SourceWaveStorage`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceWaveStorage {
    /// Numeric function.
    pub func: SourceWaveFunc,
    /// Base value.
    pub base: f32,
    /// Amplitude.
    pub amplitude: f32,
    /// Phase.
    pub phase: f32,
    /// Frequency.
    pub frequency: f32,
}

impl SourceWaveStorage {
    /// Convert a waveform.
    #[must_use]
    pub const fn of(wave: &Waveform) -> Self {
        Self {
            func: SourceWaveFunc::of(wave.kind),
            base: wave.base,
            amplitude: wave.amplitude,
            phase: wave.phase,
            frequency: wave.frequency,
        }
    }

    /// Zero storage.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            func: SourceWaveFunc::None,
            base: 0.0,
            amplitude: 0.0,
            phase: 0.0,
            frequency: 0.0,
        }
    }
}

/// Authoritative per-stage source state (`SourceShaderStageState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceStageState {
    /// Stage is active.
    pub active: bool,
    /// Authoritative `GL_State` input.
    pub state_bits: u32,
    /// Numeric color generator.
    pub rgb_gen: SourceColorGen,
    /// Numeric alpha generator.
    pub alpha_gen: SourceAlphaGen,
    /// Numeric coordinate generator.
    pub tc_gen: SourceTcGen,
    /// Retained RGB wave storage.
    pub rgb_wave: SourceWaveStorage,
    /// Retained alpha wave storage.
    pub alpha_wave: SourceWaveStorage,
    /// Set by `$lightmap`; never cleared.
    pub is_lightmap: bool,
    /// Present in `textureBundle_t` but never assigned.
    pub vertex_lightmap: bool,
}

/// RGB generator (`ColorGenerator`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorGen {
    /// Identity.
    Identity,
    /// Identity lighting.
    IdentityLighting,
    /// Entity.
    Entity,
    /// One minus entity.
    OneMinusEntity,
    /// Vertex.
    Vertex,
    /// Exact vertex.
    ExactVertex,
    /// Lighting diffuse.
    LightingDiffuse,
    /// One minus vertex.
    OneMinusVertex,
    /// Constant color.
    Const(Vec3),
    /// Waveform.
    Wave(Waveform),
}

impl ColorGen {
    /// Numeric generator.
    #[must_use]
    pub const fn source(&self) -> SourceColorGen {
        match self {
            Self::IdentityLighting => SourceColorGen::IdentityLighting,
            Self::Identity => SourceColorGen::Identity,
            Self::Entity => SourceColorGen::Entity,
            Self::OneMinusEntity => SourceColorGen::OneMinusEntity,
            Self::ExactVertex => SourceColorGen::ExactVertex,
            Self::Vertex => SourceColorGen::Vertex,
            Self::OneMinusVertex => SourceColorGen::OneMinusVertex,
            Self::Wave(_) => SourceColorGen::Waveform,
            Self::LightingDiffuse => SourceColorGen::LightingDiffuse,
            Self::Const(_) => SourceColorGen::Const,
        }
    }
}

/// Alpha generator (`AlphaGenerator`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaGen {
    /// Identity.
    Identity,
    /// Entity.
    Entity,
    /// One minus entity.
    OneMinusEntity,
    /// Vertex.
    Vertex,
    /// Lighting specular.
    LightingSpecular,
    /// One minus vertex.
    OneMinusVertex,
    /// Constant alpha.
    Const(f32),
    /// Waveform.
    Wave(Waveform),
    /// Portal with range.
    Portal(f32),
}

impl AlphaGen {
    /// Numeric generator.
    #[must_use]
    pub const fn source(&self) -> SourceAlphaGen {
        match self {
            Self::Identity => SourceAlphaGen::Identity,
            Self::Entity => SourceAlphaGen::Entity,
            Self::OneMinusEntity => SourceAlphaGen::OneMinusEntity,
            Self::Vertex => SourceAlphaGen::Vertex,
            Self::OneMinusVertex => SourceAlphaGen::OneMinusVertex,
            Self::LightingSpecular => SourceAlphaGen::LightingSpecular,
            Self::Wave(_) => SourceAlphaGen::Waveform,
            Self::Portal(_) => SourceAlphaGen::Portal,
            Self::Const(_) => SourceAlphaGen::Const,
        }
    }
}

/// Texture-coordinate generator (`TexCoordGenerator`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TexGen {
    /// Texture.
    Texture,
    /// Lightmap.
    Lightmap,
    /// Environment.
    Environment,
    /// Projected vectors.
    Vector {
        /// S projection.
        s: Vec3,
        /// T projection.
        t: Vec3,
    },
}

impl TexGen {
    /// Numeric generator.
    #[must_use]
    pub const fn source(&self) -> SourceTcGen {
        match self {
            Self::Texture => SourceTcGen::Texture,
            Self::Lightmap => SourceTcGen::Lightmap,
            Self::Environment => SourceTcGen::EnvironmentMapped,
            Self::Vector { .. } => SourceTcGen::Vector,
        }
    }
}

/// Texture-coordinate modifier (`TexCoordModifier`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TexMod {
    /// Scale.
    Scale(Vec2),
    /// Scroll.
    Scroll(Vec2),
    /// Stretch.
    Stretch(Waveform),
    /// Turbulent.
    Turb(Waveform),
    /// Rotate degrees per second.
    Rotate(f32),
    /// Matrix transform.
    Transform {
        /// Matrix row 0.
        m00: f32,
        /// Matrix row 0 col 1.
        m01: f32,
        /// Matrix row 1 col 0.
        m10: f32,
        /// Matrix row 1 col 1.
        m11: f32,
        /// Translation.
        translation: Vec2,
    },
    /// Entity translate.
    EntityTranslate,
    /// No-op (stops modifier evaluation, retains slot).
    None,
}

/// Stage image map (`ShaderMap`).
#[derive(Debug, Clone, PartialEq)]
pub enum ShaderMap {
    /// Named image.
    Image {
        /// Image name.
        name: String,
        /// Clamp wrapping.
        clamp: bool,
    },
    /// Lightmap.
    Lightmap,
    /// White image.
    WhiteImage,
    /// Frame animation.
    Animation {
        /// Frames per second factor.
        frequency: f32,
        /// Frame image names.
        frames: Vec<String>,
    },
    /// Video map.
    Video {
        /// Video name.
        name: String,
    },
    /// No map.
    None,
}

/// Parsed shader stage (`ShaderStage`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderStage {
    /// Image map.
    pub map: ShaderMap,
    /// Blend equation.
    pub blend: Blend,
    /// Depth test.
    pub depth_func: DepthTest,
    /// Depth write.
    pub depth_write: bool,
    /// Alpha test.
    pub alpha_func: AlphaTest,
    /// Detail stage.
    pub detail: bool,
    /// RGB generator.
    pub rgb_gen: ColorGen,
    /// Alpha generator.
    pub alpha_gen: AlphaGen,
    /// Coordinate generator.
    pub tc_gen: TexGen,
    /// Coordinate modifiers.
    pub tc_mods: Vec<TexMod>,
}

/// A stage from `ParseStage`, before `FinishShader` (`ParsedShaderStage`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedStage {
    /// Semantic stage.
    pub stage: ShaderStage,
    /// Source state.
    pub source_state: SourceStageState,
}

/// Vertex deformation (`VertexDeformation`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VertexDeformation {
    /// Projection shadow.
    ProjectionShadow,
    /// Autosprite.
    Autosprite,
    /// Autosprite 2.
    Autosprite2,
    /// None.
    None,
    /// Text from render-text row.
    Text {
        /// Row index.
        index: u8,
    },
    /// Wave deform.
    Wave {
        /// Spread (reciprocal divisor).
        spread: f32,
        /// Wave.
        wave: Waveform,
    },
    /// Normal perturb.
    Normal {
        /// Amplitude.
        amplitude: f32,
        /// Frequency.
        frequency: f32,
    },
    /// Move along a direction.
    Move {
        /// Direction.
        direction: Vec3,
        /// Wave.
        wave: Waveform,
    },
    /// Bulge.
    Bulge {
        /// Width.
        width: f32,
        /// Height.
        height: f32,
        /// Speed.
        speed: f32,
    },
}

/// Sky parameters (`sky` definition field).
#[derive(Debug, Clone, PartialEq)]
pub struct SkyParms {
    /// Outer box base name.
    pub outer_box: Option<String>,
    /// Cloud height.
    pub cloud_height: f32,
    /// Inner box base name.
    pub inner_box: Option<String>,
}

/// Fog parameters (`fog` definition field).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogParms {
    /// Fog color.
    pub color: Vec3,
    /// Depth for opaque.
    pub depth_for_opaque: f32,
}

/// Sun parameters (`sun` definition field).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunParms {
    /// Sun color.
    pub color: Vec3,
    /// Intensity.
    pub intensity: f32,
    /// Azimuth degrees.
    pub azimuth: f32,
    /// Elevation degrees.
    pub elevation: f32,
}

/// Registered sun vectors (`RegisteredSun`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegisteredSun {
    /// Scaled light color.
    pub light: Vec3,
    /// Direction.
    pub direction: Vec3,
}

/// Compute registered sun vectors (`sunDirective` tail).
#[must_use]
pub fn sun_registration(parms: &SunParms) -> RegisteredSun {
    let light = scale3(normalize3(parms.color), parms.intensity);
    let azimuth = parms.azimuth / 180.0 * core::f32::consts::PI;
    let elevation = parms.elevation / 180.0 * core::f32::consts::PI;
    RegisteredSun {
        light,
        direction: qa_core::math::vec3(
            azimuth.cos() * elevation.cos(),
            azimuth.sin() * elevation.cos(),
            elevation.sin(),
        ),
    }
}

/// A retained compiler directive (`qer*`/`q3map*`/`tesssize`/`light`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompilerDirective {
    /// Directive name.
    pub name: String,
    /// Line arguments.
    pub arguments: Vec<String>,
}

/// A shader warning (`ShaderDiagnostic`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderDiagnostic {
    /// Source name.
    pub source: String,
    /// Line (1-based).
    pub line: usize,
    /// Column (1-based).
    pub column: usize,
    /// Message.
    pub message: String,
}

/// A parsed shader (`ShaderDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderDefinition {
    /// Shader name.
    pub name: String,
    /// Parsed stages.
    pub stages: Vec<ParsedStage>,
    /// Surface parameters.
    pub surface_parms: Vec<String>,
    /// Face culling.
    pub cull: CullFace,
    /// Sort key override.
    pub sort: Option<f32>,
    /// Sky parameters.
    pub sky: Option<SkyParms>,
    /// Fog parameters.
    pub fog: Option<FogParms>,
    /// Sun parameters.
    pub sun: Option<SunParms>,
    /// Vertex deformations.
    pub deforms: Vec<VertexDeformation>,
    /// Polygon offset.
    pub polygon_offset: bool,
    /// No mipmaps.
    pub no_mipmaps: bool,
    /// No picmip.
    pub no_picmip: bool,
    /// Entity mergable.
    pub entity_mergable: bool,
    /// Shader-wide portal range (last `alphaGen portal` wins).
    pub portal_range: f32,
    /// Clamp time.
    pub clamp_time: f32,
    /// Warnings.
    pub warnings: Vec<ShaderDiagnostic>,
    /// Retained compiler directives.
    pub compiler_directives: Vec<CompilerDirective>,
}

/// `q_shared.c COM_StripExtension` (stops at the first dot or NUL).
#[must_use]
pub fn strip_shader_extension(name: &str) -> &str {
    match name.find(['.', '\0']) {
        Some(end) => &name[..end],
        None => name,
    }
}

/// `Q_stricmp` lookup key (ASCII lowercase; separators stay distinct).
#[must_use]
pub fn normalize_shader_name(name: &str) -> String {
    strip_shader_extension(name).to_ascii_lowercase()
}

/// `tr_shader.c generateHashValue` (folds separators for buckets only).
#[must_use]
pub fn shader_name_hash(name: &str, size: u32) -> u32 {
    debug_assert!(size == 1024 || size == 2048);
    let mut hash: i32 = 0;
    for (index, letter) in name.bytes().enumerate() {
        let mut letter = i32::from(letter);
        if letter == 0 || letter == 46 {
            break;
        }
        if (65..=90).contains(&letter) {
            letter += 32;
        }
        if letter == 92 {
            letter = 47;
        }
        if (128..=255).contains(&letter) {
            letter -= 256;
        }
        hash = hash.wrapping_add(letter.wrapping_mul(index as i32 + 119));
    }
    (hash ^ (hash >> 10) ^ (hash >> 20)) as u32 & (size - 1)
}

/// `q_shared.c Q_stricmp` through NUL, ASCII-only folding.
#[must_use]
pub fn same_shader_name(first: &str, second: &str) -> bool {
    let first = first.as_bytes();
    let second = second.as_bytes();
    for index in 0..99999 {
        let mut a = if index < first.len() {
            i32::from(first[index])
        } else {
            0
        };
        let mut b = if index < second.len() {
            i32::from(second[index])
        } else {
            0
        };
        if (65..=90).contains(&a) {
            a += 32;
        }
        if (65..=90).contains(&b) {
            b += 32;
        }
        if a != b {
            return false;
        }
        if a == 0 {
            return true;
        }
    }
    true
}

/// Normalize an image name (`*` names pass through).
#[must_use]
pub fn normalize_image_name(name: &str) -> String {
    if name.starts_with('*') {
        name.to_string()
    } else {
        name.replace('\\', "/").to_ascii_lowercase()
    }
}

/// `Math.fround(nativeAtof(value))`.
fn source_atof(value: &str) -> f32 {
    native_atof(value).unwrap_or(0.0) as f32
}

/// A script token with source position.
#[derive(Debug, Clone, PartialEq)]
struct Token {
    value: String,
    line: usize,
    column: usize,
}

/// Tokenize a shader script: whitespace split, `//` and `/* */`
/// comments, double-quoted strings kept whole (quotes stripped).
fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut line = 1usize;
    let mut column = 1usize;
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\n' {
            line += 1;
            column = 1;
            index += 1;
            continue;
        }
        if ch.is_whitespace() {
            column += 1;
            index += 1;
            continue;
        }
        if ch == '/' && chars.get(index + 1) == Some(&'/') {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if ch == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            column += 2;
            while index < chars.len() && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/')) {
                if chars[index] == '\n' {
                    line += 1;
                    column = 1;
                } else {
                    column += 1;
                }
                index += 1;
            }
            index = (index + 2).min(chars.len());
            continue;
        }
        if ch == '"' {
            let start_column = column;
            let start_line = line;
            index += 1;
            column += 1;
            let mut value = String::new();
            while index < chars.len() && chars[index] != '"' && chars[index] != '\n' {
                value.push(chars[index]);
                index += 1;
                column += 1;
            }
            if chars.get(index) == Some(&'"') {
                index += 1;
                column += 1;
            }
            tokens.push(Token {
                value,
                line: start_line,
                column: start_column,
            });
            continue;
        }
        let start_column = column;
        let mut value = String::new();
        while index < chars.len()
            && !chars[index].is_whitespace()
            && chars[index] != '"'
            && !(chars[index] == '/' && chars.get(index + 1).is_some_and(|next| *next == '/' || *next == '*'))
        {
            value.push(chars[index]);
            index += 1;
            column += 1;
        }
        tokens.push(Token {
            value,
            line,
            column: start_column,
        });
    }
    tokens
}

/// Parse failure with source position.
#[derive(Debug, Clone)]
struct ParseFail {
    line: usize,
    column: usize,
    message: String,
}

/// Recursive shader-script parser (`ShaderParser`, plain-text path).
struct ShaderParser {
    source: String,
    tokens: Vec<Token>,
    cursor: usize,
    pending: Option<Token>,
    depth: i32,
    location: Token,
    warnings: Vec<ShaderDiagnostic>,
    portal_range: f32,
    shader_name: String,
    drop_message: Option<String>,
}

impl ShaderParser {
    fn new(text: &str, source: &str) -> Self {
        Self {
            source: source.to_string(),
            tokens: tokenize(text),
            cursor: 0,
            pending: None,
            depth: 0,
            location: Token {
                value: String::new(),
                line: 1,
                column: 1,
            },
            warnings: Vec::new(),
            portal_range: 0.0,
            shader_name: String::new(),
            drop_message: None,
        }
    }

    fn next(&mut self, allow_line_breaks: bool) -> Option<Token> {
        if let Some(token) = self.pending.take() {
            self.location = token.clone();
            return Some(token);
        }
        let token = self.tokens.get(self.cursor)?.clone();
        if !allow_line_breaks && self.cursor > 0 && token.line != self.location.line {
            return None;
        }
        self.cursor += 1;
        if token.value.starts_with('{') {
            self.depth += 1;
        }
        if token.value.starts_with('}') {
            self.depth -= 1;
        }
        self.location = token.clone();
        Some(token)
    }

    fn fail(&self, message: String) -> ParseFail {
        ParseFail {
            line: self.location.line,
            column: self.location.column,
            message,
        }
    }

    fn warn(&mut self, message: String) {
        self.warnings.push(ShaderDiagnostic {
            source: self.source.clone(),
            line: self.location.line,
            column: self.location.column,
            message,
        });
    }

    fn reject(&mut self, message: String) -> ParseFail {
        self.warn(message.clone());
        self.fail(message)
    }

    fn error(&self, fail: &ParseFail) -> ClientError {
        ClientError::BadShader(format!(
            "{}:{}:{}: {}",
            self.source, fail.line, fail.column, fail.message
        ))
    }

    fn stage_parameter(&mut self, keyword: &str) -> Result<String, ParseFail> {
        match self.next(false) {
            Some(token) => Ok(token.value),
            None => Err(self.reject(format!(
                "WARNING: missing parameter for '{keyword}' keyword in shader '{}'\n",
                self.shader_name
            ))),
        }
    }

    fn required(&mut self, allow_line_breaks: bool) -> Result<String, ParseFail> {
        match self.next(allow_line_breaks) {
            Some(token) => Ok(token.value),
            None => Err(self.fail("Missing shader parameter".to_string())),
        }
    }

    fn number(&mut self) -> Result<f32, ParseFail> {
        let value = self.required(false)?;
        Ok(source_atof(&value))
    }

    fn expect(&mut self, value: &str, allow_line_breaks: bool) -> Result<(), ParseFail> {
        let actual = self.required(allow_line_breaks)?;
        let matches = if value == "{" {
            actual.starts_with(value)
        } else {
            actual == value
        };
        if !matches {
            return Err(self.fail(format!("Expected '{value}', received '{actual}'")));
        }
        Ok(())
    }

    fn vector(&mut self) -> Result<Vec3, ParseFail> {
        self.expect("(", false)?;
        let result = qa_core::math::vec3(self.number()?, self.number()?, self.number()?);
        self.expect(")", false)?;
        Ok(result)
    }

    fn source_vector(&mut self, previous: Vec3) -> (Vec3, bool) {
        if self.next(false).as_ref().map(|token| token.value.as_str()) != Some("(") {
            self.warn(format!(
                "WARNING: missing parenthesis in shader '{}'\n",
                self.shader_name
            ));
            return (previous, false);
        }
        let mut value = previous;
        for field in ["x", "y", "z"] {
            match self.next(false) {
                Some(token) => {
                    let parsed = source_atof(&token.value);
                    match field {
                        "x" => value.x = parsed,
                        "y" => value.y = parsed,
                        _ => value.z = parsed,
                    }
                }
                None => {
                    self.warn(format!(
                        "WARNING: missing vector element in shader '{}'\n",
                        self.shader_name
                    ));
                    return (value, false);
                }
            }
        }
        if self.next(false).as_ref().map(|token| token.value.as_str()) != Some(")") {
            self.warn(format!(
                "WARNING: missing parenthesis in shader '{}'\n",
                self.shader_name
            ));
            return (value, false);
        }
        (value, true)
    }

    fn line_arguments(&mut self) -> Vec<String> {
        let mut args = Vec::new();
        loop {
            match self.next(false) {
                None => return args,
                Some(token) => {
                    if token.value == "}" || token.value == "{" {
                        self.pending = Some(token);
                        return args;
                    }
                    args.push(token.value);
                }
            }
        }
    }

    fn source_line_value(&mut self) -> Option<String> {
        self.next(false).map(|token| token.value)
    }

    fn sun_directive(&mut self) -> (SunParms, RegisteredSun) {
        let red = source_atof(&self.source_line_value().unwrap_or_default());
        let green = source_atof(&self.source_line_value().unwrap_or_default());
        let blue = source_atof(&self.source_line_value().unwrap_or_default());
        let intensity = source_atof(&self.source_line_value().unwrap_or_default());
        let azimuth = source_atof(&self.source_line_value().unwrap_or_default());
        let elevation = source_atof(&self.source_line_value().unwrap_or_default());
        let parms = SunParms {
            color: qa_core::math::vec3(red, green, blue),
            intensity,
            azimuth,
            elevation,
        };
        (parms, sun_registration(&parms))
    }

    fn sky_directive(&mut self, previous: Option<SkyParms>) -> Option<SkyParms> {
        let outer_token = match self.next(false) {
            Some(token) => token,
            None => {
                self.warn(format!(
                    "WARNING: 'skyParms' missing parameter in shader '{}'\n",
                    self.shader_name
                ));
                return previous;
            }
        };
        let outer_box = if outer_token.value == "-" {
            previous.as_ref().and_then(|sky| sky.outer_box.clone())
        } else {
            Some(normalize_image_name(&outer_token.value))
        };
        let height_token = match self.next(false) {
            Some(token) => token,
            None => {
                self.warn(format!(
                    "WARNING: 'skyParms' missing parameter in shader '{}'\n",
                    self.shader_name
                ));
                return previous.map(|sky| SkyParms {
                    outer_box: outer_box.clone(),
                    ..sky
                });
            }
        };
        let parsed_height = source_atof(&height_token.value);
        let cloud_height = if parsed_height == 0.0 { 512.0 } else { parsed_height };
        let inner_token = match self.next(false) {
            Some(token) => token,
            None => {
                self.warn(format!(
                    "WARNING: 'skyParms' missing parameter in shader '{}'\n",
                    self.shader_name
                ));
                return previous.map(|sky| SkyParms {
                    outer_box: outer_box.clone(),
                    cloud_height,
                    ..sky
                });
            }
        };
        let inner_box = if inner_token.value == "-" {
            previous.as_ref().and_then(|sky| sky.inner_box.clone())
        } else {
            Some(normalize_image_name(&inner_token.value))
        };
        Some(SkyParms {
            outer_box,
            cloud_height,
            inner_box,
        })
    }

    fn wave(&mut self, previous: Option<Waveform>, missing: &str) -> Result<Waveform, ParseFail> {
        let token = match self.next(false) {
            Some(token) => token,
            None => {
                self.warn(missing.to_string());
                if let Some(previous) = previous {
                    return Ok(previous);
                }
                return Err(self.fail(missing.to_string()));
            }
        };
        let mut result = previous.unwrap_or(Waveform::zero(WaveKind::Sin));
        result.kind = match token.value.to_ascii_lowercase().as_str() {
            "sin" => WaveKind::Sin,
            "square" => WaveKind::Square,
            "triangle" => WaveKind::Triangle,
            "sawtooth" => WaveKind::Sawtooth,
            "inversesawtooth" => WaveKind::InverseSawtooth,
            "noise" => WaveKind::Noise,
            _ => {
                self.warn(format!(
                    "WARNING: invalid genfunc name '{}' in shader '{}'\n",
                    token.value, self.shader_name
                ));
                WaveKind::Sin
            }
        };
        for field in ["base", "amplitude", "phase", "frequency"] {
            match self.next(false) {
                Some(value) => {
                    let parsed = source_atof(&value.value);
                    match field {
                        "base" => result.base = parsed,
                        "amplitude" => result.amplitude = parsed,
                        "phase" => result.phase = parsed,
                        _ => result.frequency = parsed,
                    }
                }
                None => {
                    self.warn(missing.to_string());
                    if previous.is_none() {
                        return Err(self.fail(missing.to_string()));
                    }
                    return Ok(result);
                }
            }
        }
        Ok(result)
    }

    fn parse_rgb(&mut self, previous_wave: Waveform) -> Option<ColorGen> {
        let token = match self.next(false) {
            Some(token) => token,
            None => {
                self.warn(format!(
                    "WARNING: missing parameters for rgbGen in shader '{}'\n",
                    self.shader_name
                ));
                return None;
            }
        };
        match token.value.to_ascii_lowercase().as_str() {
            "identity" => Some(ColorGen::Identity),
            "identitylighting" => Some(ColorGen::IdentityLighting),
            "entity" => Some(ColorGen::Entity),
            "oneminusentity" => Some(ColorGen::OneMinusEntity),
            "vertex" => Some(ColorGen::Vertex),
            "exactvertex" => Some(ColorGen::ExactVertex),
            "lightingdiffuse" => Some(ColorGen::LightingDiffuse),
            "oneminusvertex" => Some(ColorGen::OneMinusVertex),
            "const" => self.vector().ok().map(ColorGen::Const),
            "wave" => {
                let missing = format!("WARNING: missing waveform parm in shader '{}'\n", self.shader_name);
                self.wave(Some(previous_wave), &missing).ok().map(ColorGen::Wave)
            }
            _ => {
                self.warn(format!(
                    "WARNING: unknown rgbGen parameter '{}' in shader '{}'\n",
                    token.value, self.shader_name
                ));
                None
            }
        }
    }

    fn parse_alpha(&mut self, previous_wave: Waveform) -> Result<Option<AlphaGen>, ParseFail> {
        let token = match self.next(false) {
            Some(token) => token,
            None => {
                self.warn(format!(
                    "WARNING: missing parameters for alphaGen in shader '{}'\n",
                    self.shader_name
                ));
                return Ok(None);
            }
        };
        match token.value.to_ascii_lowercase().as_str() {
            "identity" => Ok(Some(AlphaGen::Identity)),
            "entity" => Ok(Some(AlphaGen::Entity)),
            "oneminusentity" => Ok(Some(AlphaGen::OneMinusEntity)),
            "vertex" => Ok(Some(AlphaGen::Vertex)),
            "lightingspecular" => Ok(Some(AlphaGen::LightingSpecular)),
            "oneminusvertex" => Ok(Some(AlphaGen::OneMinusVertex)),
            "const" => {
                let alpha = native_atof(&self.source_line_value().unwrap_or_default()).unwrap_or(0.0);
                let integer = (255.0 * alpha).trunc() as i64;
                if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&integer) {
                    return Err(ParseFail {
                        line: self.location.line,
                        column: self.location.column,
                        message: "alphaGen const reaches undefined source byte conversion".to_string(),
                    });
                }
                #[allow(clippy::cast_possible_truncation)]
                Ok(Some(AlphaGen::Const(alpha as f32)))
            }
            "wave" => {
                let missing = format!("WARNING: missing waveform parm in shader '{}'\n", self.shader_name);
                self.wave(Some(previous_wave), &missing)
                    .map(|wave| Some(AlphaGen::Wave(wave)))
            }
            "portal" => {
                let token = match self.next(false) {
                    Some(token) => token,
                    None => {
                        self.portal_range = 256.0;
                        self.warn(format!(
                            "WARNING: missing range parameter for alphaGen portal in shader '{}', defaulting to 256\n",
                            self.shader_name
                        ));
                        return Ok(Some(AlphaGen::Portal(256.0)));
                    }
                };
                self.pending = Some(token);
                let range = self.number()?;
                self.portal_range = range;
                Ok(Some(AlphaGen::Portal(range)))
            }
            _ => {
                self.warn(format!(
                    "WARNING: unknown alphaGen parameter '{}' in shader '{}'\n",
                    token.value, self.shader_name
                ));
                Ok(None)
            }
        }
    }

    fn parse_tcgen(&mut self, vectors: &mut TexGen) -> Option<TexGen> {
        let token = match self.next(false) {
            Some(token) => token,
            None => {
                self.warn(format!(
                    "WARNING: missing texgen parm in shader '{}'\n",
                    self.shader_name
                ));
                return None;
            }
        };
        match token.value.to_ascii_lowercase().as_str() {
            "environment" => Some(TexGen::Environment),
            "lightmap" => Some(TexGen::Lightmap),
            "texture" => Some(TexGen::Texture),
            "base" => Some(TexGen::Texture),
            "vector" => {
                let TexGen::Vector { s, t } = vectors else {
                    return None;
                };
                let (s, _) = self.source_vector(*s);
                let (t, _) = self.source_vector(*t);
                *vectors = TexGen::Vector { s, t };
                Some(*vectors)
            }
            _ => {
                self.warn(format!(
                    "WARNING: unknown texgen parm in shader '{}'\n",
                    self.shader_name
                ));
                None
            }
        }
    }

    fn parse_tcmod(&mut self) -> Result<TexMod, ParseFail> {
        let token = self.next(false).map(|token| token.value).unwrap_or_default();
        let kind = token.to_ascii_lowercase();
        macro_rules! number {
            ($label:expr) => {{
                match self.next(false) {
                    Some(token) => source_atof(&token.value),
                    None => {
                        let message = format!("WARNING: missing {} in shader '{}'\n", $label, self.shader_name);
                        self.warn(message.clone());
                        return Err(self.fail(message));
                    }
                }
            }};
        }
        match kind.as_str() {
            "scale" => {
                let x = number!("scale parms");
                let y = number!("scale parms");
                Ok(TexMod::Scale(qa_core::math::vec2(x, y)))
            }
            "scroll" => {
                let x = number!("scale scroll parms");
                let y = number!("scale scroll parms");
                Ok(TexMod::Scroll(qa_core::math::vec2(x, y)))
            }
            "stretch" => {
                let missing = format!("WARNING: missing stretch parms in shader '{}'\n", self.shader_name);
                Ok(TexMod::Stretch(self.wave(None, &missing)?))
            }
            "turb" => {
                let mut parms = [0.0f32; 4];
                for (consumed, parm) in parms.iter_mut().enumerate() {
                    match self.next(false) {
                        Some(token) => *parm = source_atof(&token.value),
                        None => {
                            let label = if consumed == 0 {
                                "tcMod turb parms"
                            } else {
                                "tcMod turb"
                            };
                            let message = format!("WARNING: missing {label} in shader '{}'\n", self.shader_name);
                            self.warn(message.clone());
                            return Err(self.fail(message));
                        }
                    }
                }
                let [base, amplitude, phase, frequency] = parms;
                Ok(TexMod::Turb(Waveform {
                    kind: WaveKind::Sin,
                    base,
                    amplitude,
                    phase,
                    frequency,
                }))
            }
            "rotate" => Ok(TexMod::Rotate(number!("tcMod rotate parms"))),
            "transform" => {
                let m00 = number!("transform parms");
                let m01 = number!("transform parms");
                let m10 = number!("transform parms");
                let m11 = number!("transform parms");
                let x = number!("transform parms");
                let y = number!("transform parms");
                Ok(TexMod::Transform {
                    m00,
                    m01,
                    m10,
                    m11,
                    translation: qa_core::math::vec2(x, y),
                })
            }
            "entitytranslate" => Ok(TexMod::EntityTranslate),
            _ => {
                self.warn(format!(
                    "WARNING: unknown tcMod '{token}' in shader '{}'\n",
                    self.shader_name
                ));
                Ok(TexMod::None)
            }
        }
    }

    fn blend_factor(&mut self, value: &str, destination: bool) -> (BlendFactor, u32) {
        let one = if destination {
            (BlendFactor::One, state_bits::DSTBLEND_ONE)
        } else {
            (BlendFactor::One, state_bits::SRCBLEND_ONE)
        };
        match value.to_ascii_lowercase().as_str() {
            "gl_zero" => {
                if destination {
                    (BlendFactor::Zero, state_bits::DSTBLEND_ZERO)
                } else {
                    (BlendFactor::Zero, state_bits::SRCBLEND_ZERO)
                }
            }
            "gl_one" => one,
            "gl_src_alpha" => {
                if destination {
                    (BlendFactor::SrcAlpha, state_bits::DSTBLEND_SRC_ALPHA)
                } else {
                    (BlendFactor::SrcAlpha, state_bits::SRCBLEND_SRC_ALPHA)
                }
            }
            "gl_one_minus_src_alpha" => {
                if destination {
                    (BlendFactor::OneMinusSrcAlpha, state_bits::DSTBLEND_ONE_MINUS_SRC_ALPHA)
                } else {
                    (BlendFactor::OneMinusSrcAlpha, state_bits::SRCBLEND_ONE_MINUS_SRC_ALPHA)
                }
            }
            "gl_dst_alpha" => {
                if destination {
                    (BlendFactor::DstAlpha, state_bits::DSTBLEND_DST_ALPHA)
                } else {
                    (BlendFactor::DstAlpha, state_bits::SRCBLEND_DST_ALPHA)
                }
            }
            "gl_one_minus_dst_alpha" => {
                if destination {
                    (BlendFactor::OneMinusDstAlpha, state_bits::DSTBLEND_ONE_MINUS_DST_ALPHA)
                } else {
                    (BlendFactor::OneMinusDstAlpha, state_bits::SRCBLEND_ONE_MINUS_DST_ALPHA)
                }
            }
            "gl_dst_color" if !destination => (BlendFactor::DstColor, state_bits::SRCBLEND_DST_COLOR),
            "gl_one_minus_dst_color" if !destination => {
                (BlendFactor::OneMinusDstColor, state_bits::SRCBLEND_ONE_MINUS_DST_COLOR)
            }
            "gl_src_alpha_saturate" if !destination => {
                (BlendFactor::SrcAlphaSaturate, state_bits::SRCBLEND_ALPHA_SATURATE)
            }
            "gl_src_color" if destination => (BlendFactor::SrcColor, state_bits::DSTBLEND_SRC_COLOR),
            "gl_one_minus_src_color" if destination => {
                (BlendFactor::OneMinusSrcColor, state_bits::DSTBLEND_ONE_MINUS_SRC_COLOR)
            }
            _ => {
                self.warn(format!(
                    "WARNING: unknown blend mode '{value}' in shader '{}', substituting GL_ONE\n",
                    self.shader_name
                ));
                one
            }
        }
    }

    fn blend(&mut self, previous: Blend, previous_destination_bits: u32) -> Result<(Blend, u32, u32, bool), ParseFail> {
        let value = self.required(false)?;
        match value.to_ascii_lowercase().as_str() {
            "add" => Ok((ADDITIVE_BLEND, state_bits::SRCBLEND_ONE, state_bits::DSTBLEND_ONE, true)),
            "filter" => Ok((
                FILTER_BLEND,
                state_bits::SRCBLEND_DST_COLOR,
                state_bits::DSTBLEND_ZERO,
                true,
            )),
            "blend" => Ok((
                Blend {
                    source: BlendFactor::SrcAlpha,
                    destination: BlendFactor::OneMinusSrcAlpha,
                },
                state_bits::SRCBLEND_SRC_ALPHA,
                state_bits::DSTBLEND_ONE_MINUS_SRC_ALPHA,
                true,
            )),
            _ => {
                let (source, source_bits) = self.blend_factor(&value, false);
                let token = match self.next(false) {
                    Some(token) => token,
                    None => {
                        self.warn(format!(
                            "WARNING: missing parm for blendFunc in shader '{}'\n",
                            self.shader_name
                        ));
                        return Ok((
                            Blend {
                                source,
                                destination: previous.destination,
                            },
                            source_bits,
                            previous_destination_bits,
                            false,
                        ));
                    }
                };
                let (destination, destination_bits) = self.blend_factor(&token.value, true);
                Ok((Blend { source, destination }, source_bits, destination_bits, true))
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn stage(&mut self) -> Result<ParsedStage, ParseFail> {
        let mut map = ShaderMap::None;
        let mut blend = OPAQUE_BLEND;
        let mut source_blend_bits = 0u32;
        let mut destination_blend_bits = 0u32;
        let mut depth_func = DepthTest::LessEqual;
        let mut depth_write = true;
        let mut explicit_depth_write = false;
        let mut alpha_func = AlphaTest::None;
        let mut detail = false;
        let mut rgb_gen: Option<ColorGen> = None;
        let mut alpha_gen = AlphaGen::Identity;
        let mut tc_gen: Option<TexGen> = None;
        let mut tc_vectors = TexGen::Vector {
            s: qa_core::math::vec3(0.0, 0.0, 0.0),
            t: qa_core::math::vec3(0.0, 0.0, 0.0),
        };
        let mut raw_rgb_gen = SourceColorGen::Bad;
        let mut raw_alpha_gen = SourceAlphaGen::Identity;
        let mut raw_tc_gen = SourceTcGen::Bad;
        let mut rgb_wave = Waveform::zero(WaveKind::None);
        let mut alpha_wave = Waveform::zero(WaveKind::None);
        let mut is_lightmap = false;
        let mut tc_mods: Vec<TexMod> = Vec::new();

        macro_rules! snapshot {
            ($completed:expr) => {{
                let mut stage_rgb_gen = rgb_gen.unwrap_or(ColorGen::IdentityLighting);
                let mut stage_raw_rgb_gen = raw_rgb_gen;
                let mut stage_raw_alpha_gen = raw_alpha_gen;
                let mut stage_tc_gen = tc_gen.unwrap_or(if is_lightmap {
                    TexGen::Lightmap
                } else {
                    TexGen::Texture
                });
                let mut stage_blend = blend;
                let mut stage_depth_func = depth_func;
                let mut stage_depth_write = depth_write;
                let mut stage_alpha_func = alpha_func;
                let mut state_bits_value = 0u32;
                if $completed {
                    if rgb_gen.is_none() {
                        stage_rgb_gen = if source_blend_bits == 0
                            || source_blend_bits == state_bits::SRCBLEND_ONE
                            || source_blend_bits == state_bits::SRCBLEND_SRC_ALPHA
                        {
                            ColorGen::IdentityLighting
                        } else {
                            ColorGen::Identity
                        };
                        stage_raw_rgb_gen = stage_rgb_gen.source();
                    }
                    let mut blend_bits = source_blend_bits | destination_blend_bits;
                    if source_blend_bits == state_bits::SRCBLEND_ONE
                        && destination_blend_bits == state_bits::DSTBLEND_ZERO
                    {
                        blend_bits = 0;
                        stage_depth_write = true;
                    }
                    let input = SourceStateInput {
                        depth_test: depth_func,
                        depth_write: stage_depth_write,
                        blend: None,
                        alpha_test: stage_alpha_func,
                    };
                    match source_state_bits(&input, PolygonMode::Fill, true) {
                        Ok(extra) => state_bits_value = blend_bits | extra,
                        Err(_) => {
                            return Err(self.fail("GL_State cannot encode stage state".to_string()));
                        }
                    }
                    // ParseStage compares alphaGen_t to CGEN_IDENTITY (2).
                    if stage_raw_alpha_gen == SourceAlphaGen::Entity
                        && (stage_raw_rgb_gen == SourceColorGen::Identity
                            || stage_raw_rgb_gen == SourceColorGen::LightingDiffuse)
                    {
                        stage_raw_alpha_gen = SourceAlphaGen::Skip;
                    }
                } else {
                    stage_blend = OPAQUE_BLEND;
                    stage_depth_func = DepthTest::LessEqual;
                    stage_depth_write = false;
                    stage_alpha_func = AlphaTest::None;
                }
                let _ = &mut stage_tc_gen;
                ParsedStage {
                    stage: ShaderStage {
                        map: map.clone(),
                        blend: stage_blend,
                        depth_func: stage_depth_func,
                        depth_write: stage_depth_write,
                        alpha_func: stage_alpha_func,
                        detail,
                        rgb_gen: stage_rgb_gen,
                        alpha_gen,
                        tc_gen: stage_tc_gen,
                        tc_mods: tc_mods.clone(),
                    },
                    source_state: SourceStageState {
                        active: true,
                        state_bits: state_bits_value,
                        rgb_gen: stage_raw_rgb_gen,
                        alpha_gen: stage_raw_alpha_gen,
                        tc_gen: raw_tc_gen,
                        rgb_wave: SourceWaveStorage::of(&rgb_wave),
                        alpha_wave: SourceWaveStorage::of(&alpha_wave),
                        is_lightmap,
                        vertex_lightmap: false,
                    },
                }
            }};
        }

        loop {
            let token = match self.next(true) {
                Some(token) => token,
                None => return Err(self.reject("WARNING: no matching '}' found\n".to_string())),
            };
            let keyword = token.value.to_ascii_lowercase();
            if keyword.starts_with('}') {
                return Ok(snapshot!(true));
            }
            match keyword.as_str() {
                "map" | "clampmap" => {
                    let source_name = self.stage_parameter(keyword.as_str())?;
                    let name = normalize_image_name(&source_name);
                    if keyword == "map" && name == "$lightmap" {
                        map = ShaderMap::Lightmap;
                        is_lightmap = true;
                    } else if keyword == "map" && name == "$whiteimage" {
                        map = ShaderMap::WhiteImage;
                    } else {
                        map = ShaderMap::Image {
                            name,
                            clamp: keyword == "clampmap",
                        };
                    }
                }
                "animmap" => {
                    let frequency = source_atof(&self.stage_parameter("animMmap")?);
                    let mut frames: Vec<String> = Vec::new();
                    map = ShaderMap::Animation {
                        frequency,
                        frames: Vec::new(),
                    };
                    while let Some(token) = self.next(false) {
                        if token.value == "}" || token.value == "{" {
                            self.pending = Some(token);
                            break;
                        }
                        if frames.len() < 8 {
                            frames.push(normalize_image_name(&token.value));
                        }
                        map = ShaderMap::Animation {
                            frequency,
                            frames: frames.clone(),
                        };
                    }
                }
                "videomap" => {
                    let source_name = self.stage_parameter("videoMmap")?;
                    map = ShaderMap::Video {
                        name: normalize_image_name(&source_name),
                    };
                }
                "blendfunc" => {
                    let token = match self.next(false) {
                        Some(token) => token,
                        None => {
                            self.warn(format!(
                                "WARNING: missing parm for blendFunc in shader '{}'\n",
                                self.shader_name
                            ));
                            continue;
                        }
                    };
                    self.pending = Some(token);
                    let parsed = self.blend(blend, destination_blend_bits)?;
                    blend = parsed.0;
                    source_blend_bits = parsed.1;
                    destination_blend_bits = parsed.2;
                    if parsed.3 && !explicit_depth_write {
                        depth_write = false;
                    }
                }
                "depthwrite" => {
                    depth_write = true;
                    explicit_depth_write = true;
                }
                "depthfunc" => {
                    let value = self.stage_parameter("depthfunc")?.to_ascii_lowercase();
                    if value == "equal" {
                        depth_func = DepthTest::Equal;
                    } else if value == "lequal" {
                        depth_func = DepthTest::LessEqual;
                    } else {
                        let seen = self.location.value.clone();
                        self.warn(format!(
                            "WARNING: unknown depthfunc '{seen}' in shader '{}'\n",
                            self.shader_name
                        ));
                    }
                }
                "alphafunc" => {
                    let value = self.stage_parameter("alphaFunc")?.to_ascii_lowercase();
                    match value.as_str() {
                        "gt0" => alpha_func = AlphaTest::Gt0,
                        "lt128" => alpha_func = AlphaTest::Lt128,
                        "ge128" => alpha_func = AlphaTest::Ge128,
                        _ => {
                            let seen = self.location.value.clone();
                            self.warn(format!(
                                "WARNING: invalid alphaFunc name '{seen}' in shader '{}'\n",
                                self.shader_name
                            ));
                            alpha_func = AlphaTest::None;
                        }
                    }
                }
                "detail" => detail = true,
                "rgbgen" => {
                    if let Some(parsed) = self.parse_rgb(rgb_wave) {
                        raw_rgb_gen = parsed.source();
                        if parsed == ColorGen::Vertex && raw_alpha_gen == SourceAlphaGen::Identity {
                            alpha_gen = AlphaGen::Vertex;
                            raw_alpha_gen = SourceAlphaGen::Vertex;
                        }
                        if let ColorGen::Wave(wave) = parsed {
                            rgb_wave = wave;
                        }
                        rgb_gen = Some(parsed);
                    }
                }
                "alphagen" => {
                    if let Some(parsed) = self.parse_alpha(alpha_wave)? {
                        raw_alpha_gen = parsed.source();
                        if let AlphaGen::Wave(wave) = parsed {
                            alpha_wave = wave;
                        }
                        alpha_gen = parsed;
                    }
                }
                "tcgen" | "texgen" => {
                    if let Some(parsed) = self.parse_tcgen(&mut tc_vectors) {
                        raw_tc_gen = parsed.source();
                        tc_gen = Some(parsed);
                    }
                }
                "tcmod" => {
                    if tc_mods.len() == 4 {
                        let message = format!("ERROR: too many tcMod stages in shader '{}'\n", self.shader_name);
                        self.drop_message = Some(message);
                        return Err(self.fail("A stage cannot exceed four tcMod directives".to_string()));
                    }
                    tc_mods.push(TexMod::None);
                    let slot = tc_mods.len() - 1;
                    if let Ok(parsed) = self.parse_tcmod() {
                        tc_mods[slot] = parsed;
                    }
                    self.line_arguments();
                }
                _ => {
                    return Err(self.reject(format!(
                        "WARNING: unknown parameter '{}' in shader '{}'\n",
                        token.value, self.shader_name
                    )));
                }
            }
        }
    }

    fn deform_number(&mut self, bulge: bool) -> Option<f32> {
        match self.next(false) {
            Some(token) => Some(source_atof(&token.value)),
            None => {
                self.warn(format!(
                    "WARNING: missing deformVertexes {}parm in shader '{}'\n",
                    if bulge { "bulge " } else { "" },
                    self.shader_name
                ));
                None
            }
        }
    }

    fn deform(&mut self) -> Result<VertexDeformation, ParseFail> {
        let token = self.required(false)?;
        let kind = token.to_ascii_lowercase();
        let zero_wave = Waveform::zero(WaveKind::None);
        let missing = format!("WARNING: missing waveform parm in shader '{}'\n", self.shader_name);
        match kind.as_str() {
            "projectionshadow" => Ok(VertexDeformation::ProjectionShadow),
            "autosprite" => Ok(VertexDeformation::Autosprite),
            "autosprite2" => Ok(VertexDeformation::Autosprite2),
            "bulge" => {
                let (Some(width), Some(height), Some(speed)) = (
                    self.deform_number(true),
                    self.deform_number(true),
                    self.deform_number(true),
                ) else {
                    return Ok(VertexDeformation::None);
                };
                Ok(VertexDeformation::Bulge { width, height, speed })
            }
            "normal" => {
                let (Some(amplitude), Some(frequency)) = (self.deform_number(false), self.deform_number(false)) else {
                    return Ok(VertexDeformation::None);
                };
                Ok(VertexDeformation::Normal { amplitude, frequency })
            }
            "move" => {
                let (Some(x), Some(y), Some(z)) = (
                    self.deform_number(false),
                    self.deform_number(false),
                    self.deform_number(false),
                ) else {
                    return Ok(VertexDeformation::None);
                };
                Ok(VertexDeformation::Move {
                    direction: qa_core::math::vec3(x, y, z),
                    wave: self.wave(Some(zero_wave), &missing)?,
                })
            }
            "wave" => {
                let divisor = match self.deform_number(false) {
                    Some(divisor) => divisor,
                    None => return Ok(VertexDeformation::None),
                };
                if divisor == 0.0 {
                    self.warn(format!(
                        "WARNING: illegal div value of 0 in deformVertexes command for shader '{}'\n",
                        self.shader_name
                    ));
                }
                let spread = if divisor == 0.0 { 100.0 } else { 1.0 / divisor };
                Ok(VertexDeformation::Wave {
                    spread,
                    wave: self.wave(Some(zero_wave), &missing)?,
                })
            }
            _ => {
                if let Some(rest) = kind.strip_prefix("text") {
                    let index = rest
                        .bytes()
                        .next()
                        .map(|code| if (48..=55).contains(&code) { code - 48 } else { 0 });
                    return Ok(VertexDeformation::Text {
                        index: index.unwrap_or(0),
                    });
                }
                self.warn(format!(
                    "WARNING: unknown deformVertexes subtype '{token}' found in shader '{}'\n",
                    self.shader_name
                ));
                Ok(VertexDeformation::None)
            }
        }
    }

    fn sort(&mut self) -> Option<f32> {
        let token = match self.next(false) {
            Some(token) => token,
            None => {
                self.warn(format!(
                    "WARNING: missing sort parameter in shader '{}'\n",
                    self.shader_name
                ));
                return None;
            }
        };
        match token.value.to_ascii_lowercase().as_str() {
            "portal" => Some(1.0),
            "sky" => Some(2.0),
            "opaque" => Some(3.0),
            "decal" => Some(4.0),
            "seethrough" => Some(5.0),
            "banner" => Some(6.0),
            "underwater" => Some(8.0),
            "additive" => Some(10.0),
            "nearest" => Some(16.0),
            _ => Some(source_atof(&token.value)),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn definition(&mut self, name: &str) -> Result<ShaderDefinition, ParseFail> {
        self.warnings = Vec::new();
        self.shader_name.clone_from(&name.to_string());
        self.portal_range = 0.0;
        self.drop_message = None;
        let shader_name = name.to_string();
        let mut stages: Vec<ParsedStage> = Vec::new();
        let mut surface_parms: Vec<String> = Vec::new();
        let mut deforms: Vec<VertexDeformation> = Vec::new();
        let mut compiler_directives: Vec<CompilerDirective> = Vec::new();
        let mut cull = CullFace::Front;
        let mut sort: Option<f32> = None;
        let mut sky: Option<SkyParms> = None;
        let mut fog: Option<FogParms> = None;
        let mut sun: Option<SunParms> = None;
        let mut polygon_offset = false;
        let mut no_mipmaps = false;
        let mut no_picmip = false;
        let mut entity_mergable = false;
        let mut clamp_time = 0.0f32;

        let opening = self.next(true).map(|token| token.value).unwrap_or_default();
        if !opening.starts_with('{') {
            return Err(self.reject(format!(
                "WARNING: expecting '{{', found '{opening}' instead in shader '{}'\n",
                self.shader_name
            )));
        }
        loop {
            let next = match self.next(true) {
                Some(next) => next,
                None => {
                    return Err(self.reject(format!("WARNING: no concluding '}}' in shader {}\n", self.shader_name)));
                }
            };
            let token = next.value.to_ascii_lowercase();
            if token.starts_with('}') {
                break;
            }
            let keyword = if token.starts_with('{') { "{".to_string() } else { token };
            match keyword.as_str() {
                "{" => {
                    if stages.len() == 8 {
                        return Err(ParseFail {
                            line: self.location.line,
                            column: self.location.column,
                            message: "A shader cannot exceed eight stages".to_string(),
                        });
                    }
                    stages.push(self.stage()?);
                }
                "surfaceparm" => {
                    let Some(token) = self.next(false) else {
                        continue;
                    };
                    surface_parms.push(token.value.to_ascii_lowercase());
                }
                "nomipmaps" => {
                    no_mipmaps = true;
                    no_picmip = true;
                }
                "nopicmip" => no_picmip = true,
                "polygonoffset" => polygon_offset = true,
                "entitymergable" => entity_mergable = true,
                "clamptime" => {
                    if let Some(token) = self.next(false) {
                        clamp_time = source_atof(&token.value);
                    }
                }
                "portal" => sort = Some(1.0),
                "sort" => {
                    if let Some(value) = self.sort() {
                        sort = Some(value);
                    }
                }
                "cull" => {
                    let Some(token) = self.next(false) else {
                        self.warn(format!(
                            "WARNING: missing cull parms in shader '{}'\n",
                            self.shader_name
                        ));
                        continue;
                    };
                    match token.value.to_ascii_lowercase().as_str() {
                        "none" | "twosided" | "disable" => cull = CullFace::None,
                        "back" | "backside" | "backsided" => cull = CullFace::Back,
                        _ => self.warn(format!(
                            "WARNING: invalid cull parm '{}' in shader '{}'\n",
                            token.value, self.shader_name
                        )),
                    }
                }
                "deformvertexes" => {
                    let Some(token) = self.next(false) else {
                        self.warn(format!(
                            "WARNING: missing deform parm in shader '{}'\n",
                            self.shader_name
                        ));
                        continue;
                    };
                    if deforms.len() == 3 {
                        self.warn(format!("WARNING: MAX_SHADER_DEFORMS in '{}'\n", self.shader_name));
                        continue;
                    }
                    self.pending = Some(token);
                    deforms.push(self.deform()?);
                }
                "skyparms" => sky = self.sky_directive(sky),
                "fogparms" => {
                    let previous = fog.map(|fog| fog.color).unwrap_or(qa_core::math::vec3(0.0, 0.0, 0.0));
                    let (color, complete) = self.source_vector(previous);
                    fog = Some(FogParms {
                        color,
                        depth_for_opaque: fog.map(|fog| fog.depth_for_opaque).unwrap_or(0.0),
                    });
                    if !complete {
                        return Err(self.fail("Incomplete fogParms color vector".to_string()));
                    }
                    let Some(token) = self.next(false) else {
                        self.warn(format!(
                            "WARNING: missing parm for 'fogParms' keyword in shader '{}'\n",
                            self.shader_name
                        ));
                        continue;
                    };
                    if let Some(fog) = fog.as_mut() {
                        fog.depth_for_opaque = source_atof(&token.value);
                    }
                    self.line_arguments();
                }
                "q3map_sun" => {
                    let (parms, _) = self.sun_directive();
                    sun = Some(parms);
                }
                "light" => {
                    let token = self.next(false);
                    compiler_directives.push(CompilerDirective {
                        name: keyword.clone(),
                        arguments: token.map(|token| vec![token.value]).unwrap_or_default(),
                    });
                }
                _ => {
                    if keyword.starts_with("qer") || keyword.starts_with("q3map") || keyword == "tesssize" {
                        let arguments = self.line_arguments();
                        compiler_directives.push(CompilerDirective {
                            name: keyword.clone(),
                            arguments,
                        });
                    } else {
                        return Err(self.reject(format!(
                            "WARNING: unknown general shader parameter '{}' in '{}'\n",
                            next.value, self.shader_name
                        )));
                    }
                }
            }
        }
        if stages.is_empty() && sky.is_none() && !surface_parms.iter().any(|parm| parm == "fog") {
            return Err(self.fail(
                "Shader has no stages and is neither sky nor fog; source uses an implicit material".to_string(),
            ));
        }
        Ok(ShaderDefinition {
            name: shader_name,
            stages,
            surface_parms,
            cull,
            sort,
            sky,
            fog,
            sun,
            deforms,
            polygon_offset,
            no_mipmaps,
            no_picmip,
            entity_mergable,
            portal_range: self.portal_range,
            clamp_time,
            warnings: core::mem::take(&mut self.warnings),
            compiler_directives,
        })
    }

    fn parse(mut self, recover: bool) -> Result<Vec<ShaderEntry>, ClientError> {
        let mut entries = Vec::new();
        loop {
            let name_token = match self.next(true) {
                Some(token) => token,
                None => return Ok(entries),
            };
            if name_token.value == "{" || name_token.value == "}" {
                return Err(self.error(&self.fail("Expected shader name".to_string())));
            }
            let name = name_token.value.clone();
            let drop_before = self.drop_message.clone();
            match self.definition(&name) {
                Ok(definition) => entries.push(ShaderEntry {
                    name,
                    result: ShaderEntryResult::Accepted(Box::new(definition)),
                }),
                Err(fail) => {
                    if !recover {
                        return Err(self.error(&fail));
                    }
                    let error = self.error(&fail);
                    let drop_message = self.drop_message.clone().or(drop_before);
                    entries.push(ShaderEntry {
                        name,
                        result: ShaderEntryResult::Rejected {
                            message: match &error {
                                ClientError::BadShader(message) => message.clone(),
                                _ => String::new(),
                            },
                            drop_message,
                        },
                    });
                    while self.depth > 0 {
                        if self.next(true).is_none() {
                            return Ok(entries);
                        }
                    }
                }
            }
        }
    }
}

/// One inspected script entry (`ShaderCatalogEntry` text result).
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderEntry {
    /// Shader name.
    pub name: String,
    /// Parse result.
    pub result: ShaderEntryResult,
}

/// Accepted definition or rejection record.
#[derive(Debug, Clone, PartialEq)]
pub enum ShaderEntryResult {
    /// Parsed definition.
    Accepted(Box<ShaderDefinition>),
    /// Rejected with message and optional drop directive.
    Rejected {
        /// Error message.
        message: String,
        /// `ERROR: too many tcMod` drop message when hit.
        drop_message: Option<String>,
    },
}

/// Parse a shader script, failing on the first rejection
/// (`parseShaderScript`).
pub fn parse_shader_script(text: &str, source: &str) -> Result<Vec<ShaderDefinition>, ClientError> {
    let entries = ShaderParser::new(text, source).parse(false)?;
    let mut definitions = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry.result {
            ShaderEntryResult::Accepted(definition) => definitions.push(*definition),
            ShaderEntryResult::Rejected { message, .. } => {
                return Err(ClientError::BadShader(message));
            }
        }
    }
    Ok(definitions)
}

/// Inspect a script, retaining valid siblings (`inspectShaderScript`).
pub fn inspect_shader_script(text: &str, source: &str) -> Result<Vec<ShaderEntry>, ClientError> {
    ShaderParser::new(text, source).parse(true)
}

/// `EvalWaveForm` table index with source range checks.
fn shader_table_index(value: f32) -> Result<i32, ClientError> {
    if !value.is_finite() {
        return Err(ClientError::BadShader(
            "Shader table index reaches undefined source float-to-int conversion".to_string(),
        ));
    }
    let integer = value.trunc() as i64;
    if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&integer) {
        return Err(ClientError::BadShader(
            "Shader table index reaches undefined source float-to-int conversion".to_string(),
        ));
    }
    #[allow(clippy::cast_possible_wrap)]
    Ok((integer as i32) & 1023)
}

/// Evaluate a waveform (`evaluateWaveform` / `EvalWaveForm`).
pub fn evaluate_waveform(wave: &Waveform, time: f32) -> Result<f32, ClientError> {
    let index = shader_table_index((wave.phase + time * wave.frequency) * 1024.0)?;
    let value = match wave.kind {
        WaveKind::None => {
            return Err(ClientError::BadShader(
                "EvalWaveForm does not support GF_NONE".to_string(),
            ));
        }
        WaveKind::Sin => renderer_sine(index),
        WaveKind::Square => {
            if index < 512 {
                1.0
            } else {
                -1.0
            }
        }
        WaveKind::Sawtooth => f32::from(index as u16) / 1024.0,
        WaveKind::InverseSawtooth => 1.0 - f32::from(index as u16) / 1024.0,
        WaveKind::Triangle => {
            if index < 256 {
                f32::from(index as u16) / 256.0
            } else if index < 768 {
                2.0 - f32::from(index as u16) / 256.0
            } else {
                f32::from(index as u16) / 256.0 - 4.0
            }
        }
        WaveKind::Noise => {
            return Err(ClientError::BadShader(
                "Noise waveforms require the renderer's seeded R_NoiseGet4f context; EvalWaveForm does not support GF_NOISE".to_string(),
            ));
        }
    };
    Ok(wave.base + value * wave.amplitude)
}

/// Texture-coordinate evaluation context (`TexCoordContext`).
#[derive(Debug, Clone, Copy, Default)]
pub struct TexCoordContext {
    /// Lightmap coordinates.
    pub lightmap: Option<Vec2>,
    /// View origin in model coordinates.
    pub view_origin: Option<Vec3>,
    /// Entity shader texcoord.
    pub shader_tex_coord: Option<Vec2>,
}

/// Evaluate texture coordinates (`evaluateTexCoords`).
pub fn evaluate_tex_coords(
    stage: &ShaderStage,
    uv: Vec2,
    position: Vec3,
    normal: Vec3,
    time: f32,
    context: &TexCoordContext,
) -> Result<Vec2, ClientError> {
    use qa_core::math::vec2;
    let mut result = match &stage.tc_gen {
        TexGen::Texture => uv,
        TexGen::Lightmap => context
            .lightmap
            .ok_or_else(|| ClientError::BadShader("Lightmap tcGen requires lightmap coordinates".to_string()))?,
        TexGen::Vector { s, t } => vec2(dot3(position, *s), dot3(position, *t)),
        TexGen::Environment => {
            let view_origin = context.view_origin.ok_or_else(|| {
                ClientError::BadShader("Environment tcGen requires the view origin in model coordinates".to_string())
            })?;
            let delta = sub3(view_origin, position);
            let viewer = qa_content::md3::normalize_fast3(delta);
            let d = dot3(normal, viewer);
            let reflected_y = normal.y * 2.0 * d - viewer.y;
            let reflected_z = normal.z * 2.0 * d - viewer.z;
            vec2(0.5 + reflected_y * 0.5, 0.5 - reflected_z * 0.5)
        }
    };
    for modifier in &stage.tc_mods {
        let (s, t) = (result.x, result.y);
        match modifier {
            TexMod::None => return Ok(vec2(result.x, result.y)),
            TexMod::Scale(amount) => result = vec2(s * amount.x, t * amount.y),
            TexMod::Scroll(amount) => {
                let x = amount.x * time;
                let y = amount.y * time;
                result = vec2(s + (x - x.floor()), t + (y - y.floor()));
            }
            TexMod::EntityTranslate => {
                let coord = context.shader_tex_coord.ok_or_else(|| {
                    ClientError::BadShader("entityTranslate requires entity shaderTexCoord".to_string())
                })?;
                let x = coord.x * time;
                let y = coord.y * time;
                result = vec2(s + (x - x.floor()), t + (y - y.floor()));
            }
            TexMod::Transform {
                m00,
                m01,
                m10,
                m11,
                translation,
            } => {
                result = vec2(s * m00 + t * m10 + translation.x, s * m01 + t * m11 + translation.y);
            }
            TexMod::Rotate(degrees_per_second) => {
                let index = shader_table_index(-degrees_per_second * time * (1024.0 / 360.0))?;
                let sin = renderer_sine(index);
                let cos = renderer_sine(index + 256);
                result = vec2(
                    s * cos + t * -sin + (0.5 - 0.5 * cos + 0.5 * sin),
                    s * sin + t * cos + (0.5 - 0.5 * sin - 0.5 * cos),
                );
            }
            TexMod::Stretch(wave) => {
                let scale = 1.0 / evaluate_waveform(wave, time)?;
                let translate = 0.5 - 0.5 * scale;
                result = vec2(s * scale + translate, t * scale + translate);
            }
            TexMod::Turb(wave) => {
                let now = wave.phase + time * wave.frequency;
                let sx = shader_table_index(((position.x + position.z) / 1024.0 + now) * 1024.0)?;
                let sy = shader_table_index((position.y / 1024.0 + now) * 1024.0)?;
                result = vec2(
                    s + renderer_sine(sx) * wave.amplitude,
                    t + renderer_sine(sy) * wave.amplitude,
                );
            }
        }
    }
    Ok(vec2(result.x, result.y))
}

/// Build a stage render state (`stageState`).
#[must_use]
pub fn stage_state(stage: &ShaderStage, cull: CullFace) -> RenderState {
    RenderState {
        blend: stage.blend,
        depth_test: match stage.depth_func {
            DepthTest::Always => DepthTest::LessEqual,
            test => test,
        },
        depth_write: stage.depth_write,
        alpha_test: stage.alpha_func,
        cull,
        depth_range: [0.0, 1.0],
        polygon_offset: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r"
textures/test/rock
{
    surfaceparm stone
    {
        map textures/test/rock.tga
        blendFunc GL_ONE GL_ZERO
        rgbGen identity
    }
}
";

    #[test]
    fn parses_single_stage_shader() {
        let definitions = parse_shader_script(SCRIPT, "<test>").unwrap();
        assert_eq!(definitions.len(), 1);
        let definition = &definitions[0];
        assert_eq!(definition.name, "textures/test/rock");
        assert_eq!(definition.stages.len(), 1);
        assert_eq!(definition.surface_parms, vec!["stone".to_string()]);
        assert!(matches!(definition.stages[0].stage.map, ShaderMap::Image { .. }));
    }

    #[test]
    fn inspect_retains_siblings() {
        let text = format!("{SCRIPT}\nbroken {{\n bogusparam\n}}\n");
        let entries = inspect_shader_script(&text, "<test>").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0].result, ShaderEntryResult::Accepted(_)));
        assert!(matches!(entries[1].result, ShaderEntryResult::Rejected { .. }));
    }

    #[test]
    fn unknown_stage_parameter_rejects() {
        let text = "bad\n{\n {\n map foo\n wat\n }\n}\n";
        let err = parse_shader_script(text, "<test>").unwrap_err();
        assert!(matches!(err, ClientError::BadShader(_)));
    }

    #[test]
    fn name_helpers_match_donor() {
        assert_eq!(strip_shader_extension("a/b.tga"), "a/b");
        assert_eq!(normalize_shader_name("A/B.TGA"), "a/b");
        assert!(same_shader_name("AbC", "aBc"));
        assert!(!same_shader_name("a/b", "a\\b"));
        assert!(shader_name_hash("test", 1024) < 1024);
    }

    #[test]
    fn waveform_sin_at_zero_is_base() {
        let wave = Waveform {
            kind: WaveKind::Sin,
            base: 0.5,
            amplitude: 0.25,
            phase: 0.0,
            frequency: 1.0,
        };
        let value = evaluate_waveform(&wave, 0.0).unwrap();
        assert!((value - 0.5).abs() < 1e-6);
    }

    #[test]
    fn waveform_none_and_noise_are_errors() {
        assert!(evaluate_waveform(&Waveform::zero(WaveKind::None), 0.0).is_err());
        assert!(evaluate_waveform(&Waveform::zero(WaveKind::Noise), 0.0).is_err());
    }

    #[test]
    fn tex_coords_scale_and_scroll() {
        let stage = ShaderStage {
            map: ShaderMap::None,
            blend: OPAQUE_BLEND,
            depth_func: DepthTest::LessEqual,
            depth_write: true,
            alpha_func: AlphaTest::None,
            detail: false,
            rgb_gen: ColorGen::Identity,
            alpha_gen: AlphaGen::Identity,
            tc_gen: TexGen::Texture,
            tc_mods: vec![TexMod::Scale(qa_core::math::vec2(2.0, 3.0))],
        };
        let uv = evaluate_tex_coords(
            &stage,
            qa_core::math::vec2(0.5, 0.25),
            qa_core::math::vec3(0.0, 0.0, 0.0),
            qa_core::math::vec3(0.0, 0.0, 1.0),
            0.0,
            &TexCoordContext::default(),
        )
        .unwrap();
        assert!((uv.x - 1.0).abs() < 1e-6);
        assert!((uv.y - 0.75).abs() < 1e-6);
    }

    #[test]
    fn deform_wave_zero_divisor_warns() {
        let text = "w\n{\n deformVertexes wave 0 sin 0 1 0 1\n {\n map $whiteimage\n }\n}\n";
        let definitions = parse_shader_script(text, "<test>").unwrap();
        assert!(matches!(
            definitions[0].deforms[0],
            VertexDeformation::Wave { spread, .. } if spread == 100.0
        ));
        assert!(!definitions[0].warnings.is_empty());
    }
}
