use super::lexer::{self, Kind, Token};
use super::*;

/// Sources follow native FS_ListFiles order. The first matching definition is
/// retained, as in FindShaderInShaderText's ordered hash-bucket search. Sorting
/// the finished table changes lookup cost, not duplicate precedence.
pub fn parse_sources(sources: &[ShaderSource<'_>]) -> ShaderCatalog {
    let mut definitions = Vec::new();
    let mut diagnostics = Vec::new();
    for source in sources {
        match lexer::lex(source.bytes) {
            Ok(tokens) => {
                let mut parser = Parser {
                    source: source.name,
                    tokens: &tokens,
                    at: 0,
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
                    parser.shader = Some(canonical_name(&name.text));
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
            }
            Err(error) => diagnostics.push(Diagnostic {
                source: source.name.into(),
                shader: None,
                line: error.line,
                severity: Severity::Error,
                kind: error.kind,
            }),
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
    shader: Option<String>,
    diagnostics: &'d mut Vec<Diagnostic>,
}
impl<'t> Parser<'t, '_, '_> {
    fn next(&mut self, line_breaks: bool) -> Option<&'t Token> {
        let token = self.tokens.get(self.at)?;
        if !line_breaks && token.newline_before {
            return None;
        }
        self.at += 1;
        Some(token)
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
        let Some(token) = self.tokens.get(self.at) else {
            self.report(
                line,
                Severity::Error,
                DiagnosticKind::MissingArgument(directive.into()),
            );
            return None;
        };
        if token.newline_before || token.kind != Kind::Text {
            self.report(
                line,
                Severity::Error,
                DiagnosticKind::MissingArgument(directive.into()),
            );
            return None;
        }
        self.at += 1;
        Some(token)
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
    fn vector(&mut self, directive: &str, line: usize) -> Option<[f32; 3]> {
        let Some(token) = self.next(false) else {
            self.report(
                line,
                Severity::Error,
                DiagnosticKind::MissingArgument(directive.into()),
            );
            return None;
        };
        if token.kind != Kind::LeftParen {
            self.report(
                token.line,
                Severity::Error,
                DiagnosticKind::Expected {
                    expected: "(",
                    found: token.text.clone(),
                },
            );
            return None;
        }
        let values = self.numbers(directive, line)?;
        let Some(token) = self.next(false) else {
            self.report(
                line,
                Severity::Error,
                DiagnosticKind::Expected {
                    expected: ")",
                    found: String::new(),
                },
            );
            return None;
        };
        if token.kind != Kind::RightParen {
            self.report(
                token.line,
                Severity::Error,
                DiagnosticKind::Expected {
                    expected: ")",
                    found: token.text.clone(),
                },
            );
            return None;
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
    fn skip_section(&mut self) {
        let mut depth = 1;
        while let Some(token) = self.next(true) {
            match token.kind {
                Kind::Open => depth += 1,
                Kind::Close => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn definition(&mut self, name: &Token) -> ShaderDef {
        let diagnostic_start = self.diagnostics.len();
        let mut definition = ShaderDef {
            name: canonical_name(&name.text),
            source: self.source.into(),
            line: name.line,
            valid: true,
            cull: Cull::Front,
            sort: 0.0,
            sort_explicit: false,
            surface_flags: 0,
            content_flags: 0,
            sky: None,
            fog: None,
            sun: None,
            polygon_offset: false,
            no_mipmaps: false,
            no_picmip: false,
            entity_mergable: false,
            portal: false,
            clamp_time: None,
            stages: Box::new([]),
            deforms: Box::new([]),
            unsupported: Box::new([]),
        };
        let mut stages = Vec::new();
        let mut deforms = Vec::new();
        let mut unsupported = Vec::new();
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
                break;
            };
            match token.kind {
                Kind::Close => break,
                Kind::Open => {
                    if stages.len() == MAX_STAGES {
                        self.report(
                            token.line,
                            Severity::Error,
                            DiagnosticKind::LimitExceeded(LimitKind::Stages),
                        );
                        self.skip_section();
                    } else {
                        stages.push(self.stage(token.line))
                    }
                    continue;
                }
                Kind::Text => {}
                _ => {
                    self.report(
                        token.line,
                        Severity::Error,
                        DiagnosticKind::Expected {
                            expected: "shader declaration",
                            found: token.text.clone(),
                        },
                    );
                    continue;
                }
            }
            let key = token.text.to_ascii_lowercase();
            match key.as_str() {
                "cull" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        definition.cull = match value.text.to_ascii_lowercase().as_str() {
                            "none" | "twosided" | "disable" => Cull::None,
                            "back" | "backside" | "backsided" => Cull::Back,
                            _ => {
                                self.fallback(value, &key);
                                definition.cull
                            }
                        };
                    }
                }
                "sort" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        let (sort, converted) = sort_value(&value.text);
                        if converted {
                            self.fallback(value, &key)
                        }
                        definition.sort = sort;
                        definition.sort_explicit = true;
                    }
                }
                "surfaceparm" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        if let Some((surface, contents)) = surface_parm(&value.text) {
                            definition.surface_flags |= surface;
                            definition.content_flags |= contents;
                        } else {
                            self.fallback(value, &key);
                            unsupported.push(UnsupportedDeclaration {
                                keyword: token.text.clone(),
                                arguments: vec![value.text.clone()].into_boxed_slice(),
                                line: token.line,
                                effect: DeclarationEffect::Unknown,
                            });
                        }
                    }
                }
                "skyparms" => {
                    if let Some(outer) = self.argument(&key, token.line) {
                        if let Some(height) = self.number(&key, token.line) {
                            if let Some(inner) = self.argument(&key, token.line) {
                                definition.sky = Some(SkyParms {
                                    outer_box: (outer.text != "-")
                                        .then(|| canonical_name(&outer.text)),
                                    cloud_height: if height == 0.0 { 512.0 } else { height },
                                    inner_box: (inner.text != "-")
                                        .then(|| canonical_name(&inner.text)),
                                });
                            }
                        }
                    }
                }
                "fogparms" => {
                    if let Some(color) = self.vector(&key, token.line) {
                        if let Some(depth_opaque) = self.number(&key, token.line) {
                            definition.fog = Some(FogParms {
                                color,
                                depth_opaque,
                            });
                        }
                    }
                    // Native old-gradient arguments are ignored, not generators.
                    let ignored = self.rest_of_line();
                    if !ignored.is_empty() {
                        unsupported.push(UnsupportedDeclaration {
                            keyword: "fogParms gradient".into(),
                            arguments: ignored,
                            line: token.line,
                            effect: DeclarationEffect::CompileOnly,
                        });
                    }
                }
                "q3map_sun" => {
                    if let Some([r, g, b, intensity, azimuth_degrees, elevation_degrees]) =
                        self.numbers(&key, token.line)
                    {
                        definition.sun = Some(SunParms {
                            color: [r, g, b],
                            intensity,
                            azimuth_degrees,
                            elevation_degrees,
                        });
                    }
                }
                "nomipmaps" => {
                    definition.no_mipmaps = true;
                    definition.no_picmip = true
                }
                "nopicmip" => definition.no_picmip = true,
                "polygonoffset" => definition.polygon_offset = true,
                "entitymergable" => definition.entity_mergable = true,
                "portal" => {
                    definition.portal = true;
                    definition.sort = 1.0;
                    definition.sort_explicit = true
                }
                "clamptime" => definition.clamp_time = self.number(&key, token.line),
                "deformvertexes" => {
                    let first = self.at;
                    let end = self.line_end();
                    if deforms.len() == MAX_DEFORMS {
                        self.report(
                            token.line,
                            Severity::Warning,
                            DiagnosticKind::LimitExceeded(LimitKind::Deforms),
                        );
                        unsupported.push(UnsupportedDeclaration {
                            keyword: token.text.clone(),
                            arguments: self.rest_of_line(),
                            line: token.line,
                            effect: DeclarationEffect::CompileOnly,
                        });
                    } else if let Some(deform) = self.deform(token.line) {
                        deforms.push(deform);
                    } else {
                        unsupported.push(UnsupportedDeclaration {
                            keyword: token.text.clone(),
                            arguments: self.tokens[first..end]
                                .iter()
                                .map(|argument| argument.text.clone())
                                .collect::<Vec<_>>()
                                .into_boxed_slice(),
                            line: token.line,
                            effect: DeclarationEffect::Unknown,
                        });
                        self.at = end;
                    }
                }
                "tesssize" | "light" => {
                    unsupported.push(self.unsupported(token, DeclarationEffect::CompileOnly))
                }
                _ if key.starts_with("qer") || key.starts_with("q3map") => {
                    unsupported.push(self.unsupported(token, DeclarationEffect::CompileOnly));
                }
                _ => unsupported.push(self.unsupported(token, DeclarationEffect::Unknown)),
            }
        }
        if stages.is_empty() && definition.sky.is_none() && definition.content_flags & 64 == 0 {
            self.report(name.line, Severity::Error, DiagnosticKind::NoStages);
        }
        if definition.sky.is_some() {
            definition.sort = 2.0
        }
        if definition.polygon_offset && definition.sort == 0.0 {
            definition.sort = 4.0
        }
        if definition.sort == 0.0 {
            definition.sort = if stages.first().is_some_and(|stage| stage.blend.is_some()) {
                if stages.first().is_some_and(|stage| stage.depth_write) {
                    5.0
                } else {
                    9.0
                }
            } else {
                3.0
            };
        }
        if stages.is_empty() {
            definition.sort = 7.0
        }
        definition.stages = stages.into_boxed_slice();
        definition.deforms = deforms.into_boxed_slice();
        definition.unsupported = unsupported.into_boxed_slice();
        definition.valid = !self.diagnostics[diagnostic_start..]
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error);
        definition
    }

    fn stage(&mut self, line: usize) -> ShaderStage {
        let mut stage = ShaderStage {
            map: None,
            blend: None,
            rgb_gen: RgbGen::IdentityLighting,
            alpha_gen: AlphaGen::Identity,
            tc_gen: TexCoordGen::Texture,
            tc_mods: Box::new([]),
            alpha_func: AlphaFunc::None,
            depth_func: DepthFunc::Lequal,
            depth_write: true,
            detail: false,
            unsupported: Box::new([]),
        };
        let mut rgb_explicit = false;
        let mut tc_explicit = false;
        let mut depth_explicit = false;
        let mut tc_mods = Vec::new();
        let mut tc_mod_count = 0;
        let mut unsupported = Vec::new();
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
                break;
            };
            if token.kind == Kind::Close {
                break;
            }
            if token.kind != Kind::Text {
                self.report(
                    token.line,
                    Severity::Error,
                    DiagnosticKind::Expected {
                        expected: "stage declaration",
                        found: token.text.clone(),
                    },
                );
                if token.kind == Kind::Open {
                    self.skip_section()
                }
                continue;
            }
            let key = token.text.to_ascii_lowercase();
            match key.as_str() {
                "map" | "clampmap" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        let image = canonical_name(&value.text);
                        stage.map = Some(if key == "map" && image == "$whiteimage" {
                            TextureMap::White
                        } else if key == "map" && image == "$lightmap" {
                            TextureMap::Lightmap
                        } else {
                            TextureMap::Image {
                                name: image,
                                clamp: key == "clampmap",
                            }
                        });
                    }
                }
                "animmap" => {
                    if let Some(frequency) = self.number(&key, token.line) {
                        let images = self.rest_of_line();
                        if images.is_empty() {
                            self.report(token.line, Severity::Error, DiagnosticKind::MissingImage)
                        }
                        if images.len() > MAX_ANIMATIONS {
                            self.report(
                                token.line,
                                Severity::Warning,
                                DiagnosticKind::LimitExceeded(LimitKind::AnimationFrames),
                            );
                        }
                        stage.map = Some(TextureMap::Animation {
                            frequency,
                            images: images
                                .iter()
                                .take(MAX_ANIMATIONS)
                                .map(|image| canonical_name(image))
                                .collect::<Vec<_>>()
                                .into_boxed_slice(),
                        });
                    }
                }
                "videomap" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        stage.map = Some(TextureMap::Video(canonical_name(&value.text)));
                        unsupported.push(UnsupportedDeclaration {
                            keyword: token.text.clone(),
                            arguments: vec![value.text.clone()].into_boxed_slice(),
                            line: token.line,
                            effect: DeclarationEffect::Runtime,
                        });
                        self.report(
                            token.line,
                            Severity::Warning,
                            DiagnosticKind::Unsupported(token.text.clone()),
                        );
                    }
                }
                "blendfunc" => {
                    if let Some(blend) = self.blend(token.line) {
                        stage.blend = Some(blend);
                        if !depth_explicit {
                            stage.depth_write = false
                        }
                    }
                }
                "rgbgen" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        let generated = match value.text.to_ascii_lowercase().as_str() {
                            "identity" => Some(RgbGen::Identity),
                            "identitylighting" => Some(RgbGen::IdentityLighting),
                            "entity" => Some(RgbGen::Entity),
                            "oneminusentity" => Some(RgbGen::OneMinusEntity),
                            "vertex" => {
                                if stage.alpha_gen == AlphaGen::Identity {
                                    stage.alpha_gen = AlphaGen::Vertex
                                }
                                Some(RgbGen::Vertex)
                            }
                            "exactvertex" => Some(RgbGen::ExactVertex),
                            "oneminusvertex" => Some(RgbGen::OneMinusVertex),
                            "lightingdiffuse" => Some(RgbGen::LightingDiffuse),
                            "wave" => self.wave(&key, token.line).map(RgbGen::Wave),
                            "const" => self.vector(&key, token.line).map(RgbGen::Const),
                            _ => {
                                self.fallback(value, &key);
                                None
                            }
                        };
                        if let Some(generated) = generated {
                            stage.rgb_gen = generated;
                            rgb_explicit = true
                        }
                    }
                }
                "alphagen" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        let generated = match value.text.to_ascii_lowercase().as_str() {
                            "identity" => Some(AlphaGen::Identity),
                            "entity" => Some(AlphaGen::Entity),
                            "oneminusentity" => Some(AlphaGen::OneMinusEntity),
                            "vertex" => Some(AlphaGen::Vertex),
                            "oneminusvertex" => Some(AlphaGen::OneMinusVertex),
                            "lightingspecular" => Some(AlphaGen::LightingSpecular),
                            "wave" => self.wave(&key, token.line).map(AlphaGen::Wave),
                            "const" => self.number(&key, token.line).map(AlphaGen::Const),
                            "portal" => {
                                let range = if self.tokens.get(self.at).is_some_and(|next| {
                                    !next.newline_before && next.kind == Kind::Text
                                }) {
                                    self.number(&key, token.line)
                                } else {
                                    Some(256.0)
                                };
                                range.map(AlphaGen::Portal)
                            }
                            _ => {
                                self.fallback(value, &key);
                                None
                            }
                        };
                        if let Some(generated) = generated {
                            stage.alpha_gen = generated
                        }
                    }
                }
                "tcgen" | "texgen" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        let generated = match value.text.to_ascii_lowercase().as_str() {
                            "base" | "texture" => Some(TexCoordGen::Texture),
                            "lightmap" => Some(TexCoordGen::Lightmap),
                            "environment" => Some(TexCoordGen::Environment),
                            "vector" => self.vector(&key, token.line).and_then(|s| {
                                self.vector(&key, token.line)
                                    .map(|t| TexCoordGen::Vector([s, t]))
                            }),
                            _ => {
                                self.fallback(value, &key);
                                None
                            }
                        };
                        if let Some(generated) = generated {
                            stage.tc_gen = generated;
                            tc_explicit = true
                        }
                    }
                }
                "tcmod" => {
                    if tc_mod_count == MAX_TEXMODS {
                        self.report(
                            token.line,
                            Severity::Error,
                            DiagnosticKind::LimitExceeded(LimitKind::TexMods),
                        );
                        self.rest_of_line();
                    } else {
                        tc_mod_count += 1;
                        let first = self.at;
                        let end = self.line_end();
                        if let Some(modifier) = self.texmod(token.line) {
                            tc_mods.push(modifier);
                        } else {
                            unsupported.push(UnsupportedDeclaration {
                                keyword: token.text.clone(),
                                arguments: self.tokens[first..end]
                                    .iter()
                                    .map(|argument| argument.text.clone())
                                    .collect::<Vec<_>>()
                                    .into_boxed_slice(),
                                line: token.line,
                                effect: DeclarationEffect::Unknown,
                            });
                        }
                    }
                }
                "alphafunc" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        stage.alpha_func = match value.text.to_ascii_lowercase().as_str() {
                            "gt0" => AlphaFunc::GreaterZero,
                            "lt128" => AlphaFunc::LessThanHalf,
                            "ge128" => AlphaFunc::AtLeastHalf,
                            _ => {
                                self.fallback(value, &key);
                                AlphaFunc::None
                            }
                        };
                    }
                }
                "depthfunc" => {
                    if let Some(value) = self.argument(&key, token.line) {
                        match value.text.to_ascii_lowercase().as_str() {
                            "lequal" => stage.depth_func = DepthFunc::Lequal,
                            "equal" => stage.depth_func = DepthFunc::Equal,
                            _ => self.fallback(value, &key),
                        }
                    }
                }
                "depthwrite" => {
                    stage.depth_write = true;
                    depth_explicit = true
                }
                "detail" => stage.detail = true,
                _ => unsupported.push(self.unsupported(token, DeclarationEffect::Unknown)),
            }
        }
        if stage.map.is_none() {
            self.report(line, Severity::Error, DiagnosticKind::MissingImage)
        }
        if !rgb_explicit {
            stage.rgb_gen = match stage.blend.map(|blend| blend.source) {
                None | Some(BlendFactor::One | BlendFactor::SourceAlpha) => {
                    RgbGen::IdentityLighting
                }
                _ => RgbGen::Identity,
            };
        }
        if stage.blend
            == Some(StageBlend {
                source: BlendFactor::One,
                destination: BlendFactor::Zero,
            })
        {
            stage.blend = None;
            stage.depth_write = true;
        }
        if !tc_explicit && stage.map == Some(TextureMap::Lightmap) {
            stage.tc_gen = TexCoordGen::Lightmap
        }
        // qsrc ParseStage compares alphaGen against CGEN_IDENTITY (numeric 2),
        // which is AGEN_ENTITY. Retain that observable original enum quirk.
        if stage.alpha_gen == AlphaGen::Entity
            && matches!(stage.rgb_gen, RgbGen::Identity | RgbGen::LightingDiffuse)
        {
            stage.alpha_gen = AlphaGen::Skip;
        }
        stage.tc_mods = tc_mods.into_boxed_slice();
        stage.unsupported = unsupported.into_boxed_slice();
        stage
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

    fn blend(&mut self, line: usize) -> Option<StageBlend> {
        let source = self.argument("blendFunc", line)?;
        let (source_factor, destination_factor) = match source.text.to_ascii_lowercase().as_str() {
            "add" => (BlendFactor::One, BlendFactor::One),
            "filter" => (BlendFactor::DestinationColor, BlendFactor::Zero),
            "blend" => (BlendFactor::SourceAlpha, BlendFactor::OneMinusSourceAlpha),
            _ => {
                let destination = self.argument("blendFunc", line)?;
                (
                    self.blend_factor(source, true),
                    self.blend_factor(destination, false),
                )
            }
        };
        Some(StageBlend {
            source: source_factor,
            destination: destination_factor,
        })
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
        let function = self.argument(directive, line)?;
        let function = match function.text.to_ascii_lowercase().as_str() {
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
        };
        let [base, amplitude, phase, frequency] = self.numbers(directive, line)?;
        Some(Waveform {
            function,
            base,
            amplitude,
            phase,
            frequency,
        })
    }
    fn texmod(&mut self, line: usize) -> Option<TexMod> {
        let end = self.line_end();
        let function = self.argument("tcMod", line)?;
        let modifier = match function.text.to_ascii_lowercase().as_str() {
            "scale" => self.numbers("tcMod scale", line).map(TexMod::Scale),
            "scroll" => self.numbers("tcMod scroll", line).map(TexMod::Scroll),
            "rotate" => self.number("tcMod rotate", line).map(TexMod::Rotate),
            "stretch" => self.wave("tcMod stretch", line).map(TexMod::Stretch),
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
    let bytes = text.as_bytes();
    let sign_at = usize::from(
        bytes
            .first()
            .is_some_and(|byte| matches!(byte, b'+' | b'-')),
    );
    if bytes
        .get(sign_at..)
        .is_some_and(|tail| tail.starts_with(b"0x") || tail.starts_with(b"0X"))
    {
        if let Some((number, end)) = hexadecimal(bytes, sign_at) {
            return if number.is_finite() {
                (number, end != bytes.len())
            } else {
                (0.0, true)
            };
        }
    }
    let mut at = sign_at;
    let mut digits = 0;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
        digits += 1
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
            digits += 1
        }
    }
    if digits == 0 {
        return (0.0, true);
    }
    if bytes
        .get(at)
        .is_some_and(|byte| matches!(byte, b'e' | b'E'))
    {
        let exponent = at;
        at += 1;
        if bytes
            .get(at)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            at += 1
        }
        let first_digit = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1
        }
        if at == first_digit {
            at = exponent
        }
    }
    match text[..at].parse::<f64>() {
        Ok(number) if number.is_finite() => (number, at != bytes.len()),
        _ => (0.0, true),
    }
}
fn hexadecimal(bytes: &[u8], sign_at: usize) -> Option<(f64, usize)> {
    let mut at = sign_at + 2;
    let mut number = 0.0;
    let mut digits = 0;
    while let Some(digit) = bytes.get(at).and_then(|byte| hex_digit(*byte)) {
        number = number * 16.0 + f64::from(digit);
        at += 1;
        digits += 1;
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let mut scale = 1.0 / 16.0;
        while let Some(digit) = bytes.get(at).and_then(|byte| hex_digit(*byte)) {
            number += f64::from(digit) * scale;
            scale /= 16.0;
            at += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if bytes
        .get(at)
        .is_some_and(|byte| matches!(byte, b'p' | b'P'))
    {
        let exponent_start = at;
        at += 1;
        let negative = bytes.get(at) == Some(&b'-');
        if bytes
            .get(at)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            at += 1
        }
        let first_digit = at;
        let mut exponent = 0_i32;
        while let Some(byte) = bytes.get(at).filter(|byte| byte.is_ascii_digit()) {
            exponent = (exponent * 10 + i32::from(*byte - b'0')).min(4096);
            at += 1;
        }
        if at == first_digit {
            at = exponent_start
        } else {
            number *= 2.0_f64.powi(if negative { -exponent } else { exponent })
        }
    }
    if bytes.first() == Some(&b'-') {
        number = -number
    }
    Some((number, at))
}
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
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
