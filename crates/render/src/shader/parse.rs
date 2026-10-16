use super::lexer::{self, Kind, Token};
use super::*;

/// Sources follow native FS_ListFiles order. The first matching definition is
/// retained, as in FindShaderInShaderText's ordered hash-bucket search. Sorting
/// the finished table changes lookup cost, not duplicate precedence.
pub fn parse_sources(sources: &[ShaderSource<'_>]) -> ShaderCatalog {
    let mut definitions = Vec::new();
    let mut diagnostics = Vec::new();
    for source in sources {
        let lexed = lexer::lex(source.bytes);
        let mut parser = Parser {
            source: source.name,
            tokens: &lexed.tokens,
            at: 0,
            newline_consumed: false,
            depth: 0,
            shader: None,
            diagnostics: &mut diagnostics,
        };
        while let Some(name) = parser.next(true) {
            if name.kind != Kind::Text {
                parser.report(
                    name.line,
                    Severity::Error,
                    DiagnosticKind::Expected {
                        expected: "shader name",
                        found: name.text.clone(),
                    },
                );
                continue;
            }
            parser.shader = Some(canonical_path(&name.text));
            let Some(open) = parser.next(true) else {
                parser.report(
                    name.line,
                    Severity::Error,
                    DiagnosticKind::Expected {
                        expected: "{",
                        found: String::new(),
                    },
                );
                break;
            };
            if open.kind != Kind::Open {
                parser.report(
                    open.line,
                    Severity::Error,
                    DiagnosticKind::Expected {
                        expected: "{",
                        found: open.text.clone(),
                    },
                );
                continue;
            }
            definitions.push(parser.definition(name));
        }
        if let Some(error) = lexed.error {
            diagnostics.push(Diagnostic {
                source: source.name.into(),
                shader: None,
                line: error.line,
                severity: Severity::Error,
                kind: error.kind,
            });
        }
    }
    definitions.sort_by(|a, b| a.name.cmp(&b.name));
    let mut unique: Vec<ShaderDef> = Vec::with_capacity(definitions.len());
    for definition in definitions {
        if let Some(winner) = unique
            .last()
            .filter(|winner| winner.name == definition.name)
        {
            diagnostics.push(Diagnostic {
                source: definition.source,
                shader: Some(definition.name),
                line: definition.line,
                severity: Severity::Warning,
                kind: DiagnosticKind::Duplicate {
                    winner_source: winner.source.clone(),
                },
            });
        } else {
            unique.push(definition)
        }
    }
    ShaderCatalog {
        definitions: unique.into_boxed_slice(),
        diagnostics: diagnostics.into_boxed_slice(),
    }
}

struct Parser<'t, 's, 'd> {
    source: &'s str,
    tokens: &'t [Token],
    at: usize,
    newline_consumed: bool,
    depth: usize,
    shader: Option<String>,
    diagnostics: &'d mut Vec<Diagnostic>,
}
impl<'t> Parser<'t, '_, '_> {
    fn next(&mut self, line_breaks: bool) -> Option<&'t Token> {
        let token = self.tokens.get(self.at)?;
        if !line_breaks && token.newline_before && !self.newline_consumed {
            self.newline_consumed = true;
            return None;
        }
        self.at += 1;
        self.newline_consumed = false;
        match token.kind {
            Kind::Open => self.depth += 1,
            Kind::Close => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        (!token.text.is_empty()).then_some(token)
    }
    fn report(&mut self, line: usize, severity: Severity, kind: DiagnosticKind) {
        self.diagnostics.push(Diagnostic {
            source: self.source.into(),
            shader: self.shader.clone(),
            line,
            severity,
            kind,
        });
    }
    fn argument(&mut self, directive: &str, line: usize) -> Option<&'t Token> {
        let token = self.next(false);
        if token.is_none() {
            self.report(
                line,
                Severity::Error,
                DiagnosticKind::MissingArgument(directive.into()),
            )
        }
        token
    }
    fn number(&mut self, directive: &str, line: usize) -> Option<f32> {
        let token = self.argument(directive, line)?;
        let (value, converted) = native_number(&token.text);
        if converted {
            self.fallback(token, directive);
        }
        Some(value)
    }
    fn numbers<const N: usize>(&mut self, directive: &str, line: usize) -> Option<[f32; N]> {
        let mut values = [0.0; N];
        for value in &mut values {
            *value = self.number(directive, line)?
        }
        Some(values)
    }
    fn line_end(&self) -> usize {
        let mut end = self.at;
        while let Some(token) = self.tokens.get(end) {
            if token.newline_before || matches!(token.kind, Kind::Open | Kind::Close) {
                break;
            }
            end += 1;
        }
        end
    }
    fn rest_of_line(&mut self) -> Box<[String]> {
        let end = self.line_end();
        let arguments = self.tokens[self.at..end]
            .iter()
            .map(|token| token.text.clone())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.at = end;
        arguments
    }
    fn unsupported(&mut self, token: &Token, effect: DeclarationEffect) -> UnsupportedDeclaration {
        let declaration = UnsupportedDeclaration {
            keyword: token.text.clone(),
            arguments: self.rest_of_line(),
            line: token.line,
            effect,
        };
        if effect != DeclarationEffect::CompileOnly {
            self.report(
                token.line,
                if effect == DeclarationEffect::Unknown {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                DiagnosticKind::Unsupported(token.text.clone()),
            );
        }
        declaration
    }
    fn fallback(&mut self, token: &Token, directive: &str) {
        self.report(
            token.line,
            Severity::Warning,
            DiagnosticKind::NativeFallback {
                directive: directive.into(),
                value: token.text.clone(),
            },
        );
    }
    fn emit(ops: &mut Vec<ShaderOp>, line: usize, action: ShaderAction) {
        ops.push(ShaderOp { line, action });
    }
    fn recover(&mut self, _depth: usize) {
        while self.depth != 0 && self.at < self.tokens.len() {
            self.next(true);
        }
    }
    fn definition(&mut self, name: &Token) -> ShaderDef {
        let mut ops = Vec::new();
        let mut stages = 0;
        let mut deforms = 0;
        loop {
            let Some(token) = self.next(true) else {
                self.report(
                    name.line,
                    Severity::Error,
                    DiagnosticKind::Expected {
                        expected: "}",
                        found: String::new(),
                    },
                );
                Self::emit(
                    &mut ops,
                    name.line,
                    ShaderAction::Abort(ShaderFailure::Native),
                );
                break;
            };
            let line = token.line;
            if token.kind == Kind::Close {
                Self::emit(&mut ops, line, ShaderAction::EndShader);
                break;
            }
            if token.kind == Kind::Open {
                if stages == MAX_STAGES {
                    self.report(
                        line,
                        Severity::Error,
                        DiagnosticKind::LimitExceeded(LimitKind::Stages),
                    );
                    Self::emit(
                        &mut ops,
                        line,
                        ShaderAction::Abort(ShaderFailure::Limit(LimitKind::Stages)),
                    );
                    self.recover(2);
                    break;
                }
                stages += 1;
                Self::emit(&mut ops, line, ShaderAction::BeginStage);
                if let Err((failure, depth)) = self.stage(&mut ops, line) {
                    Self::emit(&mut ops, line, ShaderAction::Abort(failure));
                    self.recover(depth);
                    break;
                }
                continue;
            }
            let key = token.text.to_ascii_lowercase();
            let mut emit = |op| Self::emit(&mut ops, line, ShaderAction::Header(op));
            match key.as_str() {
                "cull" => {
                    if let Some(value) = self.argument(&key, line) {
                        match value.text.to_ascii_lowercase().as_str() {
                            "none" | "twosided" | "disable" => emit(HeaderOp::Cull(Cull::None)),
                            "back" | "backside" | "backsided" => emit(HeaderOp::Cull(Cull::Back)),
                            _ => self.fallback(value, &key),
                        }
                    }
                }
                "sort" => {
                    if let Some(value) = self.argument(&key, line) {
                        let (parsed, fallback) = sort_value(&value.text);
                        if fallback {
                            self.fallback(value, &key)
                        }
                        emit(HeaderOp::Sort(parsed));
                    }
                }
                "surfaceparm" => {
                    if let Some(value) = self.argument(&key, line) {
                        if let Some((surface, contents)) = surface_parm(&value.text) {
                            emit(HeaderOp::SurfaceParm { surface, contents })
                        } else {
                            self.fallback(value, &key)
                        }
                    }
                }
                "skyparms" => {
                    if let Some(outer) = self.argument(&key, line) {
                        emit(HeaderOp::SkyOuter(
                            (outer.text != "-").then(|| canonical_path(&outer.text)),
                        ));
                        if let Some(height) = self.number(&key, line) {
                            emit(HeaderOp::SkyHeight(if height == 0. {
                                512.
                            } else {
                                height
                            }));
                            if let Some(inner) = self.argument(&key, line) {
                                emit(HeaderOp::SkyInner(
                                    (inner.text != "-").then(|| canonical_path(&inner.text)),
                                ))
                            }
                        }
                    }
                }
                "fogparms" => {
                    let (color, complete) = self.vector_update(&key, line);
                    emit(HeaderOp::FogColor(color));
                    if !complete {
                        Self::emit(&mut ops, line, ShaderAction::Abort(ShaderFailure::Native));
                        self.recover(1);
                        break;
                    }
                    if let Some(depth) = self.number(&key, line) {
                        emit(HeaderOp::FogDepth(depth));
                        self.rest_of_line();
                    }
                }
                "q3map_sun" => {
                    let values: [f32; 6] =
                        std::array::from_fn(|_| self.number(&key, line).unwrap_or(0.));
                    let [r, g, b, intensity, azimuth_degrees, elevation_degrees] = values;
                    emit(HeaderOp::Sun(SunParms {
                        color: [r, g, b],
                        intensity,
                        azimuth_degrees,
                        elevation_degrees,
                    }));
                }
                "nomipmaps" => emit(HeaderOp::NoMipmaps),
                "nopicmip" => emit(HeaderOp::NoPicmip),
                "polygonoffset" => emit(HeaderOp::PolygonOffset),
                "entitymergable" => emit(HeaderOp::EntityMergable),
                "portal" => emit(HeaderOp::Portal),
                "clamptime" => {
                    if let Some(value) = self.number(&key, line) {
                        emit(HeaderOp::ClampTime(value))
                    }
                }
                "deformvertexes" => {
                    if deforms == MAX_DEFORMS {
                        self.report(
                            line,
                            Severity::Warning,
                            DiagnosticKind::LimitExceeded(LimitKind::Deforms),
                        );
                        self.argument(&key, line);
                    } else {
                        let before = self.at;
                        let deform = self.deform(line);
                        if self.at != before {
                            deforms += 1;
                            emit(HeaderOp::Deform(deform));
                        }
                    }
                }
                "light" => {
                    self.argument(&key, line);
                }
                "tesssize" => emit(HeaderOp::Unsupported(
                    self.unsupported(token, DeclarationEffect::CompileOnly),
                )),
                _ if key.starts_with("qer") || key.starts_with("q3map") => emit(
                    HeaderOp::Unsupported(self.unsupported(token, DeclarationEffect::CompileOnly)),
                ),
                _ => {
                    self.report(
                        line,
                        Severity::Error,
                        DiagnosticKind::Unsupported(token.text.clone()),
                    );
                    Self::emit(&mut ops, line, ShaderAction::Abort(ShaderFailure::Native));
                    self.recover(1);
                    break;
                }
            }
        }
        ShaderDef {
            name: canonical_path(&name.text),
            source: self.source.into(),
            line: name.line,
            ops: ops.into_boxed_slice(),
        }
    }
    fn stage(
        &mut self,
        ops: &mut Vec<ShaderOp>,
        line: usize,
    ) -> Result<(), (ShaderFailure, usize)> {
        let mut texmods = 0;
        let mut animations = 0;
        loop {
            let Some(token) = self.next(true) else {
                self.report(
                    line,
                    Severity::Error,
                    DiagnosticKind::Expected {
                        expected: "stage }",
                        found: String::new(),
                    },
                );
                return Err((ShaderFailure::Native, 0));
            };
            let line = token.line;
            if token.kind == Kind::Close {
                Self::emit(ops, line, ShaderAction::EndStage);
                return Ok(());
            }
            if token.kind != Kind::Text {
                return Err((
                    ShaderFailure::Native,
                    if token.kind == Kind::Open { 3 } else { 2 },
                ));
            }
            let key = token.text.to_ascii_lowercase();
            let mut emit = |op| Self::emit(ops, line, ShaderAction::Stage(op));
            match key.as_str() {
                "map" | "clampmap" => {
                    let Some(value) = self.argument(&key, line) else {
                        return Err((ShaderFailure::Native, 2));
                    };
                    emit(StageOp::Map(
                        if key == "map" && value.text.eq_ignore_ascii_case("$whiteimage") {
                            ShaderMap::White
                        } else if key == "map" && value.text.eq_ignore_ascii_case("$lightmap") {
                            ShaderMap::Lightmap
                        } else {
                            ShaderMap::Image {
                                name: canonical_path(&value.text),
                                clamp: key == "clampmap",
                            }
                        },
                    ));
                }
                "animmap" => {
                    let Some(speed) = self.number(&key, line) else {
                        return Err((ShaderFailure::Native, 2));
                    };
                    emit(StageOp::AnimationSpeed(speed));
                    let images = self.rest_of_line();
                    for image in images.iter() {
                        if animations < MAX_ANIMATIONS {
                            emit(StageOp::AnimationFrame(canonical_path(image)));
                            animations += 1
                        } else {
                            self.report(
                                line,
                                Severity::Warning,
                                DiagnosticKind::LimitExceeded(LimitKind::AnimationFrames),
                            );
                            break;
                        }
                    }
                }
                "videomap" => {
                    if let Some(value) = self.argument(&key, line) {
                        emit(StageOp::Unsupported(UnsupportedDeclaration {
                            keyword: token.text.clone(),
                            arguments: vec![value.text.clone()].into_boxed_slice(),
                            line,
                            effect: DeclarationEffect::Runtime,
                        }));
                    } else {
                        return Err((ShaderFailure::Native, 2));
                    }
                }
                "blendfunc" => {
                    if let Some(source) = self.argument(&key, line) {
                        let blend = match source.text.to_ascii_lowercase().as_str() {
                            "add" => Some(StageBlend {
                                source: BlendFactor::One,
                                destination: BlendFactor::One,
                            }),
                            "filter" => Some(StageBlend {
                                source: BlendFactor::DestinationColor,
                                destination: BlendFactor::Zero,
                            }),
                            "blend" => Some(StageBlend::ALPHA),
                            _ => None,
                        };
                        if let Some(blend) = blend {
                            emit(StageOp::BlendSource(blend.source));
                            emit(StageOp::BlendDestination(blend.destination));
                            emit(StageOp::BlendComplete)
                        } else {
                            emit(StageOp::BlendSource(self.blend_factor(source, true)));
                            if let Some(destination) = self.argument(&key, line) {
                                emit(StageOp::BlendDestination(
                                    self.blend_factor(destination, false),
                                ));
                                emit(StageOp::BlendComplete)
                            }
                        }
                    }
                }
                "rgbgen" => {
                    if let Some(value) = self.argument(&key, line) {
                        let generated = match value.text.to_ascii_lowercase().as_str() {
                            "identity" => Some(RgbGen::Identity),
                            "identitylighting" => Some(RgbGen::IdentityLighting),
                            "entity" => Some(RgbGen::Entity),
                            "oneminusentity" => Some(RgbGen::OneMinusEntity),
                            "vertex" => Some(RgbGen::Vertex),
                            "exactvertex" => Some(RgbGen::ExactVertex),
                            "oneminusvertex" => Some(RgbGen::OneMinusVertex),
                            "lightingdiffuse" => Some(RgbGen::LightingDiffuse),
                            "wave" => {
                                emit(StageOp::RgbWave(self.wave_update(&key, line)));
                                None
                            }
                            "const" => {
                                let (values, _) = self.vector_update(&key, line);
                                if values.iter().any(Option::is_none) {
                                    return Err((
                                        ShaderFailure::InvalidData("incomplete constant color"),
                                        2,
                                    ));
                                }
                                Some(RgbGen::Const(values.map(|v| v.unwrap_or(0.))))
                            }
                            _ => {
                                self.fallback(value, &key);
                                None
                            }
                        };
                        if let Some(value) = generated {
                            emit(StageOp::RgbGen(value))
                        }
                    }
                }
                "alphagen" => {
                    if let Some(value) = self.argument(&key, line) {
                        let generated = match value.text.to_ascii_lowercase().as_str() {
                            "identity" => Some(AlphaGen::Identity),
                            "entity" => Some(AlphaGen::Entity),
                            "oneminusentity" => Some(AlphaGen::OneMinusEntity),
                            "vertex" => Some(AlphaGen::Vertex),
                            "oneminusvertex" => Some(AlphaGen::OneMinusVertex),
                            "lightingspecular" => Some(AlphaGen::LightingSpecular),
                            "wave" => {
                                emit(StageOp::AlphaWave(self.wave_update(&key, line)));
                                None
                            }
                            "const" => Some(AlphaGen::Const(self.number(&key, line).unwrap_or(0.))),
                            "portal" => {
                                Some(AlphaGen::Portal(self.number(&key, line).unwrap_or(256.)))
                            }
                            _ => {
                                self.fallback(value, &key);
                                None
                            }
                        };
                        if let Some(value) = generated {
                            emit(StageOp::AlphaGen(value))
                        }
                    }
                }
                "tcgen" | "texgen" => {
                    if let Some(value) = self.argument(&key, line) {
                        let generated = match value.text.to_ascii_lowercase().as_str() {
                            "base" | "texture" => Some(TexCoordGen::Texture),
                            "lightmap" => Some(TexCoordGen::Lightmap),
                            "environment" => Some(TexCoordGen::Environment),
                            "vector" => {
                                let (s, _) = self.vector_update(&key, line);
                                let (t, _) = self.vector_update(&key, line);
                                emit(StageOp::TcVector([s, t]));
                                None
                            }
                            _ => {
                                self.fallback(value, &key);
                                None
                            }
                        };
                        if let Some(value) = generated {
                            emit(StageOp::TcGen(value))
                        }
                    }
                }
                "tcmod" => {
                    if texmods == MAX_TEXMODS {
                        self.report(
                            line,
                            Severity::Error,
                            DiagnosticKind::LimitExceeded(LimitKind::TexMods),
                        );
                        return Err((ShaderFailure::Limit(LimitKind::TexMods), 2));
                    }
                    texmods += 1;
                    emit(StageOp::TexMod(self.texmod(line)));
                }
                "alphafunc" => {
                    let Some(value) = self.argument(&key, line) else {
                        return Err((ShaderFailure::Native, 2));
                    };
                    emit(StageOp::AlphaFunc(
                        match value.text.to_ascii_lowercase().as_str() {
                            "gt0" => AlphaFunc::GreaterZero,
                            "lt128" => AlphaFunc::LessThanHalf,
                            "ge128" => AlphaFunc::AtLeastHalf,
                            _ => {
                                self.fallback(value, &key);
                                AlphaFunc::None
                            }
                        },
                    ));
                }
                "depthfunc" => {
                    let Some(value) = self.argument(&key, line) else {
                        return Err((ShaderFailure::Native, 2));
                    };
                    match value.text.to_ascii_lowercase().as_str() {
                        "lequal" => emit(StageOp::DepthFunc(DepthFunc::Lequal)),
                        "equal" => emit(StageOp::DepthFunc(DepthFunc::Equal)),
                        _ => self.fallback(value, &key),
                    }
                }
                "depthwrite" => emit(StageOp::DepthWrite),
                "detail" => emit(StageOp::Detail),
                _ => {
                    self.report(
                        line,
                        Severity::Error,
                        DiagnosticKind::Unsupported(token.text.clone()),
                    );
                    return Err((ShaderFailure::Native, 2));
                }
            }
        }
    }
    fn vector_update(&mut self, directive: &str, line: usize) -> ([Option<f32>; 3], bool) {
        let mut values = [None; 3];
        let Some(open) = self.next(false) else {
            self.report(
                line,
                Severity::Error,
                DiagnosticKind::MissingArgument(directive.into()),
            );
            return (values, false);
        };
        if open.kind != Kind::LeftParen {
            self.report(
                line,
                Severity::Error,
                DiagnosticKind::Expected {
                    expected: "(",
                    found: open.text.clone(),
                },
            );
            return (values, false);
        }
        for value in &mut values {
            let Some(token) = self.next(false) else {
                self.report(
                    line,
                    Severity::Error,
                    DiagnosticKind::MissingArgument(directive.into()),
                );
                return (values, false);
            };
            let (number, converted) = native_number(&token.text);
            if converted {
                self.fallback(token, directive)
            }
            *value = Some(number);
        }
        let complete = self
            .next(false)
            .is_some_and(|token| token.kind == Kind::RightParen);
        if !complete {
            self.report(
                line,
                Severity::Error,
                DiagnosticKind::Expected {
                    expected: ")",
                    found: String::new(),
                },
            )
        }
        (values, complete)
    }
    fn wave_update(&mut self, directive: &str, line: usize) -> WaveUpdate {
        let mut update = WaveUpdate::default();
        let Some(function) = self.argument(directive, line) else {
            return update;
        };
        update.function = Some(match function.text.to_ascii_lowercase().as_str() {
            "sin" => WaveFunction::Sin,
            "square" => WaveFunction::Square,
            "triangle" => WaveFunction::Triangle,
            "sawtooth" => WaveFunction::Sawtooth,
            "inversesawtooth" => WaveFunction::InverseSawtooth,
            "noise" => WaveFunction::Noise,
            _ => {
                self.fallback(function, directive);
                WaveFunction::Sin
            }
        });
        for value in &mut update.values {
            let Some(number) = self.number(directive, line) else {
                break;
            };
            *value = Some(number)
        }
        update
    }

    fn deform(&mut self, line: usize) -> Option<Deform> {
        let function = self.argument("deformVertexes", line)?;
        match function.text.to_ascii_lowercase().as_str() {
            "projectionshadow" => Some(Deform::ProjectionShadow),
            "autosprite" => Some(Deform::AutoSprite),
            "autosprite2" => Some(Deform::AutoSprite2),
            "bulge" => self
                .numbers("deformVertexes bulge", line)
                .map(|[width, height, speed]| Deform::Bulge {
                    width,
                    height,
                    speed,
                }),
            "normal" => {
                self.numbers("deformVertexes normal", line)
                    .map(|[amplitude, frequency]| Deform::Normal {
                        amplitude,
                        frequency,
                    })
            }
            "move" => self
                .numbers("deformVertexes move", line)
                .and_then(|vector| {
                    self.wave("deformVertexes move", line)
                        .map(|wave| Deform::Move { vector, wave })
                }),
            "wave" => {
                let token = self.argument("deformVertexes wave", line)?;
                let (divisor, converted) = native_double(&token.text);
                let reciprocal = (1.0 / divisor) as f32;
                let spread = if divisor == 0.0 || !reciprocal.is_finite() {
                    self.fallback(token, "deformVertexes wave spread");
                    100.0
                } else {
                    reciprocal
                };
                if converted {
                    self.fallback(token, "deformVertexes wave spread")
                }
                self.wave("deformVertexes wave", line)
                    .map(|wave| Deform::Wave { spread, wave })
            }
            name if name.starts_with("text") => {
                let number = function
                    .text
                    .as_bytes()
                    .get(4)
                    .copied()
                    .filter(|byte| (b'0'..=b'7').contains(byte))
                    .map_or(0, |byte| byte - b'0');
                Some(Deform::Text(number))
            }
            _ => {
                self.report(
                    function.line,
                    Severity::Error,
                    DiagnosticKind::Unsupported(function.text.clone()),
                );
                None
            }
        }
    }

    fn blend_factor(&mut self, token: &Token, source: bool) -> BlendFactor {
        let factor = match token.text.to_ascii_lowercase().as_str() {
            "gl_zero" => Some(BlendFactor::Zero),
            "gl_one" => Some(BlendFactor::One),
            "gl_src_alpha" => Some(BlendFactor::SourceAlpha),
            "gl_one_minus_src_alpha" => Some(BlendFactor::OneMinusSourceAlpha),
            "gl_dst_alpha" => Some(BlendFactor::DestinationAlpha),
            "gl_one_minus_dst_alpha" => Some(BlendFactor::OneMinusDestinationAlpha),
            "gl_dst_color" if source => Some(BlendFactor::DestinationColor),
            "gl_one_minus_dst_color" if source => Some(BlendFactor::OneMinusDestinationColor),
            "gl_src_alpha_saturate" if source => Some(BlendFactor::SourceAlphaSaturate),
            "gl_src_color" if !source => Some(BlendFactor::SourceColor),
            "gl_one_minus_src_color" if !source => Some(BlendFactor::OneMinusSourceColor),
            _ => None,
        };
        factor.unwrap_or_else(|| {
            self.fallback(token, "blendFunc");
            BlendFactor::One
        })
    }
    fn wave(&mut self, directive: &str, line: usize) -> Option<Waveform> {
        let update = self.wave_update(directive, line);
        let values = update.values.map(|value| value.unwrap_or(0.));
        Some(Waveform {
            function: update.function.unwrap_or(WaveFunction::Sin),
            base: values[0],
            amplitude: values[1],
            phase: values[2],
            frequency: values[3],
        })
    }
    fn texmod(&mut self, line: usize) -> Option<TexMod> {
        let end = self.line_end();
        let function = self.argument("tcMod", line)?;
        let modifier = match function.text.to_ascii_lowercase().as_str() {
            "scale" => self.numbers("tcMod scale", line).map(TexMod::Scale),
            "scroll" => self.numbers("tcMod scroll", line).map(TexMod::Scroll),
            "rotate" => self.number("tcMod rotate", line).map(TexMod::Rotate),
            "stretch" => {
                let update = self.wave_update("tcMod stretch", line);
                if update.function.is_some() && update.values.iter().all(Option::is_some) {
                    Some(TexMod::Stretch(Waveform {
                        function: update.function.unwrap_or(WaveFunction::Sin),
                        base: update.values[0].unwrap_or(0.),
                        amplitude: update.values[1].unwrap_or(0.),
                        phase: update.values[2].unwrap_or(0.),
                        frequency: update.values[3].unwrap_or(0.),
                    }))
                } else {
                    None
                }
            }
            "turb" => {
                self.numbers("tcMod turb", line)
                    .map(|[base, amplitude, phase, frequency]| TexMod::Turbulent {
                        base,
                        amplitude,
                        phase,
                        frequency,
                    })
            }
            "transform" => self
                .numbers("tcMod transform", line)
                .map(|[a, b, c, d, s, t]| TexMod::Transform {
                    matrix: [[a, b], [c, d]],
                    translate: [s, t],
                }),
            "entitytranslate" => Some(TexMod::EntityTranslate),
            _ => {
                self.report(
                    function.line,
                    Severity::Error,
                    DiagnosticKind::Unsupported(function.text.clone()),
                );
                None
            }
        };
        self.at = end;
        modifier
    }
}

fn sort_value(value: &str) -> (f32, bool) {
    let named = match value.to_ascii_lowercase().as_str() {
        "portal" => 1.0,
        "sky" => 2.0,
        "opaque" => 3.0,
        "decal" => 4.0,
        "seethrough" => 5.0,
        "banner" => 6.0,
        "underwater" => 8.0,
        "additive" => 10.0,
        "nearest" => 16.0,
        _ => return native_number(value),
    };
    (named, false)
}

/// qsrc uses atof, not a whole-token Rust float parser: nonnumeric tokens are
/// zero and trailing bytes do not erase the numeric prefix. Keep that behavior
/// at this file boundary, while retaining a warning and bounding nonfinite data.
fn native_number(text: &str) -> (f32, bool) {
    let (number, converted) = native_double(text);
    let number = number as f32;
    if number.is_finite() {
        (number, converted)
    } else {
        (0.0, true)
    }
}
fn native_double(text: &str) -> (f64, bool) {
    let text = text.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (number, consumed) = qa_core::text::atof_prefix(text);
    if number.is_finite() {
        (number, consumed == 0 || consumed != text.len())
    } else {
        (0.0, true)
    }
}
fn surface_parm(value: &str) -> Option<(u32, u32)> {
    Some(match value.to_ascii_lowercase().as_str() {
        "water" => (0, 32),
        "slime" => (0, 16),
        "lava" => (0, 8),
        "playerclip" => (0, 0x10000),
        "monsterclip" => (0, 0x20000),
        "nodrop" => (0, 0x80000000),
        "nonsolid" => (0x4000, 0),
        "origin" => (0, 0x1000000),
        "trans" => (0, 0x20000000),
        "detail" => (0, 0x8000000),
        "structural" => (0, 0x10000000),
        "areaportal" => (0, 0x8000),
        "clusterportal" => (0, 0x100000),
        "donotenter" => (0, 0x200000),
        "fog" => (0, 64),
        "sky" => (4, 0),
        "lightfilter" => (0x8000, 0),
        "alphashadow" => (0x10000, 0),
        "hint" => (0x100, 0),
        "slick" => (2, 0),
        "noimpact" => (0x10, 0),
        "nomarks" => (0x20, 0),
        "ladder" => (8, 0),
        "nodamage" => (1, 0),
        "metalsteps" => (0x1000, 0),
        "flesh" => (0x40, 0),
        "nosteps" => (0x2000, 0),
        "nodraw" => (0x80, 0),
        "pointlight" => (0x800, 0),
        "nolightmap" => (0x400, 0),
        "nodlight" => (0x20000, 0),
        "dust" => (0x40000, 0),
        _ => return None,
    })
}
