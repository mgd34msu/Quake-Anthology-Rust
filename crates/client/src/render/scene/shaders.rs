//! Scene shader registry: script parsing, implicit materials, pictures.
//!
//! Donor provenance: `src/render/scene/shaders.ts`. Registration stays
//! synchronous: authored scripts parse through `inspect_shader_script`,
//! explicit shaders register through a [`ShaderRegistrationHost`] over the
//! texture loader, and unknown names fall back to implicit materials. Named
//! shaders register separately per lightmap binding, as `R_FindShader` does.

use std::collections::{HashMap, HashSet};

use crate::materials::compile::{
    compile_implicit_material, default_shader_profile, register_definition, shader_render_material,
    FinishedImagePlayback, FinishedStageBinding, ImageWrap, RegisteredImage, RegisteredShaderVideo, RegisteredStage,
    ShaderRegistrationHost, SourceImageRequest,
};
use crate::materials::finish::{
    finish_shader, FinishImplicitShaderInput, FinishShaderInput, FinishShaderProfile, ImplicitShaderKind,
};
use crate::materials::material::{
    inspect_shader_script, normalize_shader_name, RegisteredSun, ShaderDefinition, ShaderEntryResult, ShaderMap,
};
use crate::materials::sky::SkyBuilder;
use crate::render::types::{RenderImage, RendererImage};
use crate::render::RenderError;
use crate::text::draw2d::MaterialPicture;

use super::image_policy::ImageUsage;
use super::material_registrations::{admit_material, RegisteredSceneMaterial, ShaderRegistration, ShaderWorldIdentity};
use super::textures::{SceneTexture, SceneTextureLoadOptions, SceneTextureLoader, TextureFamily};

/// Cinematic player: resolves a video shader by name during registration.
pub type CinematicPlayer = Box<dyn FnMut(&str) -> Option<RegisteredShaderVideo>>;

/// Shader binding: unlit picture semantics or a world lightmap binding.
// World bindings carry their textures inline; groups are short-lived.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum SceneShaderBinding {
    /// Unlit binding (-1 dynamic, -2 white, -3 vertex, -4 picture).
    Unlit {
        /// Lightmap index.
        lightmap_index: i32,
        /// Mipmaps wanted.
        mipmap: bool,
    },
    /// World binding with an optional lightmap and base texture.
    World {
        /// World identity.
        world: ShaderWorldIdentity,
        /// Lightmap index.
        lightmap_index: i32,
        /// Lightmap image.
        lightmap: Option<RendererImage>,
        /// Base texture.
        base_texture: Option<SceneTexture>,
    },
}

impl SceneShaderBinding {
    /// Binding lightmap index.
    #[must_use]
    pub const fn lightmap_index(&self) -> i32 {
        match self {
            Self::Unlit { lightmap_index, .. } | Self::World { lightmap_index, .. } => *lightmap_index,
        }
    }
}

/// Built-in source materials.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSceneMaterials {
    /// Default material.
    pub default: RegisteredSceneMaterial,
    /// Stencil shadow material.
    pub stencil_shadow: RegisteredSceneMaterial,
    /// Projection shadow material.
    pub projection_shadow: RegisteredSceneMaterial,
    /// Flare material.
    pub flare: RegisteredSceneMaterial,
    /// Sun material.
    pub sun: RegisteredSceneMaterial,
}

/// Prepared remap batch.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSceneRemap {
    /// Prepared materials, one per binding.
    pub materials: Vec<RegisteredSceneMaterial>,
    /// Whether every binding resolved to an authored or implicit shader.
    pub accepted: bool,
}

struct PreparedSceneMaterial {
    compiled: crate::materials::compile::CompiledMaterial,
    defaulted: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct SourceMaterialRecord {
    name: String,
    order: usize,
    kind: SourceMaterialKind,
    binding: SceneShaderBinding,
    material: RegisteredSceneMaterial,
    defaulted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceMaterialKind {
    Default,
    StencilShadow,
    Ordinary,
}

fn key(name: &str) -> String {
    normalize_shader_name(name)
}

fn client_error(error: crate::ClientError) -> RenderError {
    RenderError::Backend(error.to_string())
}

struct Host<'a> {
    textures: &'a mut SceneTextureLoader,
    family: TextureFamily,
    picture: bool,
    base_texture: Option<SceneTexture>,
    white: u32,
    missing: u32,
    lightmap: u32,
    sun: &'a mut Option<RegisteredSun>,
    sky: &'a mut SkyBuilder,
    warnings: &'a mut Vec<String>,
    cinematic: &'a mut dyn FnMut(&str) -> Option<RegisteredShaderVideo>,
    error: &'a mut Option<RenderError>,
}

impl ShaderRegistrationHost for Host<'_> {
    fn white_image(&self) -> RegisteredImage {
        RegisteredImage {
            image: self.white,
            tmu: 0,
        }
    }

    fn default_image(&self) -> RegisteredImage {
        RegisteredImage {
            image: self.missing,
            tmu: 0,
        }
    }

    fn lightmap_image(&self) -> RegisteredImage {
        RegisteredImage {
            image: self.lightmap,
            tmu: 1,
        }
    }

    fn find_image(&mut self, request: &SourceImageRequest) -> Option<RegisteredImage> {
        if self.error.is_some() {
            return None;
        }
        if let Some(base) = &self.base_texture {
            if key(&request.name) == key(&base.name) || key(&request.name) == key(&format!("textures/{}", base.name)) {
                return match self
                    .textures
                    .sample_surface(base, request.mipmap, request.wrap == ImageWrap::Repeat)
                {
                    Ok(sampled) => Some(RegisteredImage {
                        image: sampled.image.ordinal,
                        tmu: 0,
                    }),
                    Err(error) => {
                        *self.error = Some(error);
                        None
                    }
                };
            }
        }
        let options = SceneTextureLoadOptions {
            mipmap: request.mipmap,
            repeat: request.wrap == ImageWrap::Repeat,
            family: self.family,
            usage: Some(if self.picture {
                ImageUsage::Picture
            } else {
                ImageUsage::Wall
            }),
        };
        match self.textures.load(&request.name, &options) {
            Ok(Some(texture)) => Some(RegisteredImage {
                image: texture.image.ordinal,
                tmu: 0,
            }),
            Ok(None) => None,
            Err(error) => {
                *self.error = Some(error);
                None
            }
        }
    }

    fn play_shader_cinematic(&mut self, name: &str) -> Option<RegisteredShaderVideo> {
        (self.cinematic)(name)
    }

    fn apply_sun(&mut self, sun: RegisteredSun) {
        *self.sun = Some(sun);
    }

    fn initialize_sky_tex_coords(&mut self, height: f32) {
        if !(height as f64).is_finite() || self.sky.initialize_cloud_coordinates(height).is_err() {
            *self.error = Some(RenderError::Backend(
                "Sky cloud height must be finite float32".to_string(),
            ));
        }
    }

    fn print_warning(&mut self, message: &str) {
        self.warnings.push(message.to_string());
    }
}

/// Scene shader registry over a texture loader.
pub struct SceneShaderRegistry {
    textures: SceneTextureLoader,
    profile: FinishShaderProfile,
    family: TextureFamily,
    play_cinematic: Option<CinematicPlayer>,
    programs: HashMap<String, ShaderDefinition>,
    cinematic_programs: HashSet<String>,
    compiled: HashMap<String, RegisteredSceneMaterial>,
    defaulted: HashSet<String>,
    bindings: HashMap<ShaderRegistration, (RegisteredSceneMaterial, SceneShaderBinding)>,
    picture_orders: HashMap<ShaderRegistration, u32>,
    generated_pictures: HashMap<String, RenderImage>,
    generated_textures: HashMap<String, SceneTexture>,
    image_requests: HashMap<String, (String, i32, bool)>,
    source_records: Vec<SourceMaterialRecord>,
    source_default: Option<RegisteredSceneMaterial>,
    source_materials: Option<SourceSceneMaterials>,
    /// Sky builder shared by sky shaders.
    pub sky: SkyBuilder,
    /// Shader warnings.
    pub warnings: Vec<String>,
    /// Registered sun.
    pub sun: Option<RegisteredSun>,
}

impl SceneShaderRegistry {
    /// Build a registry over a texture loader.
    pub fn new(
        textures: SceneTextureLoader,
        profile: FinishShaderProfile,
        family: TextureFamily,
        play_cinematic: Option<CinematicPlayer>,
    ) -> Self {
        Self {
            textures,
            profile,
            family,
            play_cinematic,
            programs: HashMap::new(),
            cinematic_programs: HashSet::new(),
            compiled: HashMap::new(),
            defaulted: HashSet::new(),
            bindings: HashMap::new(),
            picture_orders: HashMap::new(),
            generated_pictures: HashMap::new(),
            generated_textures: HashMap::new(),
            image_requests: HashMap::new(),
            source_records: Vec::new(),
            source_default: None,
            source_materials: None,
            sky: SkyBuilder::new(),
            warnings: Vec::new(),
            sun: None,
        }
    }

    /// Build a registry with the default shader profile.
    pub fn with_defaults(textures: SceneTextureLoader) -> Self {
        Self::new(textures, default_shader_profile(), TextureFamily::Q3, None)
    }

    /// Borrow the texture loader.
    #[must_use]
    pub fn textures(&self) -> &SceneTextureLoader {
        &self.textures
    }

    /// Mutably borrow the texture loader.
    pub fn textures_mut(&mut self) -> &mut SceneTextureLoader {
        &mut self.textures
    }

    /// Built-in source materials after initialization.
    pub fn source_materials(&self) -> Result<&SourceSceneMaterials, RenderError> {
        self.source_materials
            .as_ref()
            .ok_or_else(|| RenderError::Backend("Source materials have not finished initialization".to_string()))
    }

    /// Map a defaulted source material to the source default.
    #[must_use]
    pub fn source_world_material(&self, material: &RegisteredSceneMaterial) -> RegisteredSceneMaterial {
        let defaulted = self
            .source_records
            .iter()
            .find(|record| record.material.registration == material.registration)
            .is_some_and(|record| record.defaulted);
        if defaulted {
            if let Some(source) = &self.source_materials {
                return source.default.clone();
            }
        }
        material.clone()
    }

    /// Parse shader scripts and publish the built-in source materials.
    pub fn initialize_source_materials(&mut self, scripts: &[(&str, &str)]) -> Result<(), RenderError> {
        if self.source_materials.is_some() {
            return Ok(());
        }
        for (text, source) in scripts {
            self.add_script(text, source)?;
        }
        let missing = self.textures.missing().image.ordinal;
        let stage = RegisteredStage::Loaded {
            tmu: 0,
            binding: FinishedStageBinding::Images {
                playback: FinishedImagePlayback::Single { image: missing },
            },
        };
        let compile = |kind| {
            compile_implicit_material(&FinishImplicitShaderInput {
                name: String::new(),
                base_image: stage.clone(),
                profile: self.profile,
                kind,
            })
        };
        let default = self.publish_compiled(compile(ImplicitShaderKind::Default).map_err(client_error)?)?;
        self.source_records.push(SourceMaterialRecord {
            name: "<default>".to_string(),
            order: 0,
            kind: SourceMaterialKind::Default,
            binding: SceneShaderBinding::Unlit {
                lightmap_index: -1,
                mipmap: true,
            },
            material: default.clone(),
            defaulted: false,
        });
        self.source_default = Some(default.clone());
        let stencil = self.publish_compiled(compile(ImplicitShaderKind::StencilShadow).map_err(client_error)?)?;
        self.source_records.push(SourceMaterialRecord {
            name: "<stencil shadow>".to_string(),
            order: 1,
            kind: SourceMaterialKind::StencilShadow,
            binding: SceneShaderBinding::Unlit {
                lightmap_index: -1,
                mipmap: true,
            },
            material: stencil.clone(),
            defaulted: false,
        });
        let projection_shadow = self.register(
            "projectionShadow",
            SceneShaderBinding::Unlit {
                lightmap_index: -1,
                mipmap: true,
            },
        )?;
        let flare = self.register(
            "flareShader",
            SceneShaderBinding::Unlit {
                lightmap_index: -1,
                mipmap: true,
            },
        )?;
        let sun = self.register(
            "sun",
            SceneShaderBinding::Unlit {
                lightmap_index: -1,
                mipmap: true,
            },
        )?;
        self.source_materials = Some(SourceSceneMaterials {
            default,
            stencil_shadow: stencil,
            projection_shadow,
            flare,
            sun,
        });
        Ok(())
    }

    /// Parse one shader script; first definition wins per name.
    pub fn add_script(&mut self, text: &str, source: &str) -> Result<(), RenderError> {
        for entry in inspect_shader_script(text, source).map_err(client_error)? {
            let name = key(&entry.name);
            if self.programs.contains_key(&name) {
                continue;
            }
            if let ShaderEntryResult::Accepted(definition) = &entry.result {
                if definition
                    .stages
                    .iter()
                    .any(|stage| matches!(stage.stage.map, ShaderMap::Video { .. }))
                {
                    self.cinematic_programs.insert(name.clone());
                }
                self.programs.insert(name, (**definition).clone());
            }
        }
        Ok(())
    }

    /// Whether a name has an authored script.
    #[must_use]
    pub fn has_authored(&self, name: &str) -> bool {
        self.programs.contains_key(&key(name))
    }

    /// Whether a name has a cinematic (video) program.
    #[must_use]
    pub fn has_cinematic(&self, name: &str) -> bool {
        self.cinematic_programs.contains(&key(name))
    }

    /// Registered material bindings.
    #[must_use]
    pub fn material_bindings(&self) -> Vec<(RegisteredSceneMaterial, SceneShaderBinding)> {
        self.bindings.values().cloned().collect()
    }

    /// Register a shader under a binding, admitting it to the scene table.
    pub fn register(
        &mut self,
        name: &str,
        binding: SceneShaderBinding,
    ) -> Result<RegisteredSceneMaterial, RenderError> {
        if (name.is_empty() || name.starts_with('\0')) && self.source_default.is_some() {
            return Ok(self.source_default.clone().expect("checked source default"));
        }
        if self.source_default.is_some() {
            return self.register_source(name, binding);
        }
        let (lightmap, base_texture, lightmap_index, mipmap) = binding_parts(&binding);
        let cache_key = format!(
            "{}\0{lightmap_index}\0{}\0{}",
            key(name),
            lightmap.as_ref().map(|image| image.ordinal).unwrap_or(u32::MAX),
            base_texture
                .as_ref()
                .map(|texture| texture.image.ordinal)
                .unwrap_or(u32::MAX),
        );
        if let Some(material) = self.compiled.get(&cache_key) {
            return Ok(material.clone());
        }
        if matches!(binding, SceneShaderBinding::Unlit { .. }) {
            self.image_requests
                .insert(cache_key.clone(), (name.to_string(), lightmap_index, mipmap));
        }
        let prepared = self.compile(name, lightmap, lightmap_index, base_texture.as_ref(), mipmap)?;
        if prepared.defaulted {
            self.defaulted.insert(cache_key.clone());
        } else {
            self.defaulted.remove(&cache_key);
        }
        let material = self.publish_compiled(prepared.compiled)?;
        self.compiled.insert(cache_key, material.clone());
        self.bindings.insert(material.registration, (material.clone(), binding));
        Ok(material)
    }

    /// Prepare remap materials for one name across bindings.
    pub fn prepare_remap_materials(
        &mut self,
        name: &str,
        bindings: &[SceneShaderBinding],
    ) -> Result<PreparedSceneRemap, RenderError> {
        let mut materials = Vec::with_capacity(bindings.len());
        let mut accepted = true;
        for binding in bindings {
            let (lightmap, base_texture, lightmap_index, mipmap) = binding_parts(binding);
            let prepared = self.compile(name, lightmap, lightmap_index, base_texture.as_ref(), mipmap)?;
            accepted &= !prepared.defaulted;
            let material = self.publish_compiled(prepared.compiled)?;
            self.bindings
                .insert(material.registration, (material.clone(), binding.clone()));
            materials.push(material);
        }
        Ok(PreparedSceneRemap { materials, accepted })
    }

    /// Stage a replacement registry sharing authored programs.
    #[must_use]
    pub fn replacement(&self, textures: SceneTextureLoader) -> SceneShaderRegistry {
        let mut result = SceneShaderRegistry::new(textures, self.profile, self.family, None);
        result.programs = self.programs.clone();
        result.cinematic_programs = self.cinematic_programs.clone();
        result.generated_pictures = self.generated_pictures.clone();
        result
    }

    /// Replay image requests and source records into a replacement registry.
    pub fn prepare_replacement(&self, replacement: &mut SceneShaderRegistry) -> Result<(), RenderError> {
        let mut requests: Vec<(String, i32, bool)> = self.image_requests.values().cloned().collect();
        requests.sort();
        for (name, lightmap_index, mipmap) in requests {
            replacement.register(&name, SceneShaderBinding::Unlit { lightmap_index, mipmap })?;
        }
        Ok(())
    }

    /// Commit a prepared replacement, swapping textures and compiled caches.
    pub fn commit_replacement(&mut self, mut replacement: SceneShaderRegistry) {
        std::mem::swap(&mut self.textures, &mut replacement.textures);
        self.compiled = replacement.compiled;
        self.defaulted = replacement.defaulted;
        self.image_requests = replacement.image_requests;
        self.bindings = replacement.bindings;
        self.generated_textures = replacement.generated_textures;
        self.sun = replacement.sun;
        self.warnings.extend(replacement.warnings);
    }

    /// Register a picture shader.
    pub fn register_picture(&mut self, name: &str, mipmap: bool) -> Result<MaterialPicture, RenderError> {
        let compiled = self.register(
            name,
            SceneShaderBinding::Unlit {
                lightmap_index: -4,
                mipmap,
            },
        )?;
        Ok(self.picture(&compiled))
    }

    /// Register a generated picture, evicting defaulted entries for the name.
    pub fn register_generated_picture(
        &mut self,
        name: &str,
        image: RenderImage,
    ) -> Result<MaterialPicture, RenderError> {
        let wanted = key(name);
        let evict: Vec<String> = self
            .image_requests
            .iter()
            .filter(|(cache_key, (request, _, _))| key(request) == wanted && self.defaulted.contains(*cache_key))
            .map(|(cache_key, _)| cache_key.clone())
            .collect();
        for cache_key in evict {
            self.compiled.remove(&cache_key);
        }
        self.generated_pictures.entry(wanted).or_insert(image);
        self.register_picture(name, false)
    }

    /// Register a source picture; defaulted shaders return None.
    pub fn register_source_picture(
        &mut self,
        name: &str,
        mipmap: bool,
    ) -> Result<Option<MaterialPicture>, RenderError> {
        let compiled = self.register(
            name,
            SceneShaderBinding::Unlit {
                lightmap_index: -4,
                mipmap,
            },
        )?;
        if self.source_world_material(&compiled).registration == self.source_materials()?.default.registration {
            return Ok(None);
        }
        Ok(Some(self.picture(&compiled)))
    }

    /// Picture handle for the source default material.
    pub fn source_default_picture(&mut self) -> Result<MaterialPicture, RenderError> {
        let default = self.source_materials()?.default.clone();
        Ok(self.picture(&default))
    }

    fn picture(&mut self, compiled: &RegisteredSceneMaterial) -> MaterialPicture {
        let order = match self.picture_orders.get(&compiled.registration) {
            Some(order) => *order,
            None => {
                let order = self.picture_orders.len() as u32 + 1;
                self.picture_orders.insert(compiled.registration, order);
                order
            }
        };
        MaterialPicture { order }
    }

    fn publish_compiled(
        &self,
        compiled: crate::materials::compile::CompiledMaterial,
    ) -> Result<RegisteredSceneMaterial, RenderError> {
        admit_material(compiled)
            .ok_or_else(|| RenderError::Backend("Shader registration table is unavailable".to_string()))
    }

    fn register_source(
        &mut self,
        name: &str,
        binding: SceneShaderBinding,
    ) -> Result<RegisteredSceneMaterial, RenderError> {
        let wanted = key(name);
        let mut previous: Option<usize> = None;
        for (index, record) in self.source_records.iter().enumerate() {
            if key(&record.name) != wanted {
                continue;
            }
            if let Some(found) = previous {
                if self.source_records[found].order > record.order {
                    continue;
                }
            }
            let lightmap_index = record.binding.lightmap_index();
            if record.defaulted
                || lightmap_index == binding.lightmap_index()
                    && (lightmap_index < 0 || worlds_match(&record.binding, &binding))
            {
                previous = Some(index);
            }
        }
        if let Some(index) = previous {
            let record = self.source_records[index].clone();
            if record.defaulted && self.generated_pictures.contains_key(&wanted) {
                let (lightmap, base_texture, lightmap_index, mipmap) = binding_parts(&binding);
                let prepared = self.compile(name, lightmap, lightmap_index, base_texture.as_ref(), mipmap)?;
                let material = self.publish_compiled(prepared.compiled)?;
                self.source_records[index] = SourceMaterialRecord {
                    binding,
                    material: material.clone(),
                    defaulted: prepared.defaulted,
                    ..record
                };
                let stored = self.source_records[index].clone();
                self.bindings.insert(
                    stored.material.registration,
                    (stored.material.clone(), stored.binding.clone()),
                );
                return Ok(stored.material);
            }
            return Ok(record.material);
        }
        let (lightmap, base_texture, lightmap_index, mipmap) = binding_parts(&binding);
        let prepared = self.compile(name, lightmap, lightmap_index, base_texture.as_ref(), mipmap)?;
        let material = self.publish_compiled(prepared.compiled)?;
        let order = self.source_records.len();
        self.source_records.push(SourceMaterialRecord {
            name: name.to_string(),
            order,
            kind: SourceMaterialKind::Ordinary,
            binding: binding.clone(),
            material: material.clone(),
            defaulted: prepared.defaulted,
        });
        self.bindings.insert(material.registration, (material.clone(), binding));
        Ok(material)
    }

    fn compile(
        &mut self,
        name: &str,
        lightmap: Option<RendererImage>,
        lightmap_index: i32,
        base_texture: Option<&SceneTexture>,
        mipmap: bool,
    ) -> Result<PreparedSceneMaterial, RenderError> {
        if let Some(generated) = self.generated_pictures.get(&key(name)).cloned() {
            let texture = match self.generated_textures.get(&key(name)).cloned() {
                Some(texture) => texture,
                None => {
                    let texture = self.textures.register(
                        name,
                        generated,
                        crate::render::types::TextureSampling {
                            repeat: false,
                            filter: crate::render::types::TextureFilter::Linear,
                        },
                        None,
                        None,
                    )?;
                    self.generated_textures.insert(key(name), texture.clone());
                    texture
                }
            };
            let compiled = compile_implicit_material(&FinishImplicitShaderInput {
                name: name.to_string(),
                base_image: loaded_stage(texture.image.ordinal),
                profile: self.profile,
                kind: ImplicitShaderKind::Picture,
            })
            .map_err(client_error)?;
            return Ok(PreparedSceneMaterial {
                compiled,
                defaulted: false,
            });
        }
        if let Some(program) = self.programs.get(&key(name)).cloned() {
            let white = self.textures.white().image.ordinal;
            let missing = self.textures.missing().image.ordinal;
            let lightmap_image = lightmap.as_ref().map(|image| image.ordinal).unwrap_or(white);
            let mut error = None;
            let mut noop = |_: &str| None;
            let family = self.family;
            let registered = {
                let Self {
                    textures,
                    sun,
                    sky,
                    warnings,
                    play_cinematic,
                    ..
                } = &mut *self;
                let cinematic: &mut dyn FnMut(&str) -> Option<RegisteredShaderVideo> = match play_cinematic {
                    Some(callback) => callback,
                    None => &mut noop,
                };
                let mut host = Host {
                    textures,
                    family,
                    picture: lightmap_index == -4,
                    base_texture: base_texture.cloned(),
                    white,
                    missing,
                    lightmap: lightmap_image,
                    sun,
                    sky,
                    warnings,
                    cinematic,
                    error: &mut error,
                };
                register_definition(&mut host, &program)
            };
            if let Some(error) = error {
                return Err(error);
            }
            let finished = finish_shader(&FinishShaderInput {
                definition: registered.definition.clone(),
                lightmap_index,
                images: registered.stages.clone(),
                profile: self.profile,
            })
            .map_err(client_error)?;
            let material = shader_render_material(&registered.definition).map_err(client_error)?;
            return Ok(PreparedSceneMaterial {
                compiled: crate::materials::compile::CompiledMaterial {
                    registered,
                    finished,
                    material,
                },
                defaulted: false,
            });
        }
        let loaded = self.textures.load(
            name,
            &SceneTextureLoadOptions {
                mipmap,
                repeat: mipmap,
                family: self.family,
                usage: Some(if lightmap_index == -4 {
                    ImageUsage::Picture
                } else {
                    ImageUsage::Wall
                }),
            },
        )?;
        let texture = loaded.clone().unwrap_or_else(|| self.textures.missing().clone());
        let base_image = loaded_stage(texture.image.ordinal);
        if loaded.is_none() {
            self.warnings.push(format!(
                "{name}: missing shader image, using the source default material"
            ));
            let compiled = compile_implicit_material(&FinishImplicitShaderInput {
                name: name.to_string(),
                base_image,
                profile: self.profile,
                kind: ImplicitShaderKind::Default,
            })
            .map_err(client_error)?;
            return Ok(PreparedSceneMaterial {
                compiled,
                defaulted: true,
            });
        }
        let kind = if let Some(lightmap) = lightmap {
            ImplicitShaderKind::Lightmap {
                lightmap_index,
                lightmap_image: loaded_stage(lightmap.ordinal),
            }
        } else if lightmap_index == -2 {
            ImplicitShaderKind::White {
                white_image: loaded_stage(self.textures.white().image.ordinal),
            }
        } else if lightmap_index == -4 {
            ImplicitShaderKind::Picture
        } else if lightmap_index == -3 {
            ImplicitShaderKind::Vertex
        } else {
            ImplicitShaderKind::Dynamic
        };
        let compiled = compile_implicit_material(&FinishImplicitShaderInput {
            name: name.to_string(),
            base_image,
            profile: self.profile,
            kind,
        })
        .map_err(client_error)?;
        Ok(PreparedSceneMaterial {
            compiled,
            defaulted: false,
        })
    }
}

fn loaded_stage(image: u32) -> RegisteredStage {
    RegisteredStage::Loaded {
        tmu: 0,
        binding: FinishedStageBinding::Images {
            playback: FinishedImagePlayback::Single { image },
        },
    }
}

fn binding_parts(binding: &SceneShaderBinding) -> (Option<RendererImage>, Option<SceneTexture>, i32, bool) {
    match binding {
        SceneShaderBinding::Unlit { lightmap_index, mipmap } => (None, None, *lightmap_index, *mipmap),
        SceneShaderBinding::World {
            lightmap_index,
            lightmap,
            base_texture,
            ..
        } => (
            lightmap.clone(),
            base_texture.clone(),
            *lightmap_index,
            *lightmap_index != -4,
        ),
    }
}

fn worlds_match(first: &SceneShaderBinding, second: &SceneShaderBinding) -> bool {
    match (first, second) {
        (SceneShaderBinding::World { world: a, .. }, SceneShaderBinding::World { world: b, .. }) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::scene::resources::SceneImageRegistry;
    use crate::render::types::{fresh_owner_identity, ImageSource, ResourceOwner};
    use qa_core::identity::IdentityOwner;
    use std::collections::HashMap;

    struct FakeReader {
        assets: HashMap<String, Vec<u8>>,
    }

    impl super::super::textures::SceneAssetReader for FakeReader {
        fn read(&self, path: &str) -> Result<Option<super::super::textures::SceneAsset>, RenderError> {
            use super::super::textures::SceneAsset;
            Ok(self.assets.get(path).map(|bytes| SceneAsset {
                bytes: bytes.clone(),
                source: ImageSource::Resource {
                    requested_path: path.to_string(),
                },
            }))
        }
    }

    fn registry() -> SceneShaderRegistry {
        let session = IdentityOwner::create("shaders-test")
            .expect("session")
            .session()
            .clone();
        let owner = ResourceOwner::new(fresh_owner_identity(), session, 0);
        let images = SceneImageRegistry::new(owner);
        let mut header = vec![0u8; 18];
        header[2] = 2;
        header[12..14].copy_from_slice(&1u16.to_le_bytes());
        header[14..16].copy_from_slice(&1u16.to_le_bytes());
        header[16] = 24;
        header.extend([5u8, 6, 7]);
        let mut assets = HashMap::new();
        assets.insert("textures/rock.tga".to_string(), header);
        let loader = SceneTextureLoader::new(images, Box::new(FakeReader { assets }), None, None, 224).expect("loader");
        SceneShaderRegistry::with_defaults(loader)
    }

    #[test]
    fn implicit_registration_caches_by_binding() {
        let mut registry = registry();
        let binding = SceneShaderBinding::Unlit {
            lightmap_index: -1,
            mipmap: true,
        };
        let first = registry.register("textures/rock", binding.clone()).expect("register");
        let second = registry.register("textures/rock", binding).expect("cached");
        assert_eq!(first.registration, second.registration);
        assert!(!registry.material_bindings().is_empty());
    }

    #[test]
    fn missing_shader_defaults_with_warning() {
        let mut registry = registry();
        let material = registry
            .register(
                "textures/absent",
                SceneShaderBinding::Unlit {
                    lightmap_index: -1,
                    mipmap: true,
                },
            )
            .expect("register");
        assert_eq!(material.material.name, "textures/absent");
        assert!(!registry.warnings.is_empty());
    }

    #[test]
    fn script_registration_marks_cinematics() {
        let mut registry = registry();
        registry
            .add_script("textures/movie { { map video.roq } }", "test.shader")
            .expect("script");
        assert!(registry.has_authored("textures/movie"));
    }

    #[test]
    fn source_initialization_publishes_builtins() {
        let mut registry = registry();
        registry.initialize_source_materials(&[]).expect("source");
        let materials = registry.source_materials().expect("materials");
        assert_ne!(materials.default.registration, materials.stencil_shadow.registration);
        let picture = registry.source_default_picture().expect("picture");
        assert_eq!(picture.order, 1);
        assert_eq!(
            registry
                .register_source_picture("textures/absent", false)
                .expect("none"),
            None
        );
    }

    #[test]
    fn remap_preparation_reports_acceptance() {
        let mut registry = registry();
        let prepared = registry
            .prepare_remap_materials(
                "textures/rock",
                &[SceneShaderBinding::Unlit {
                    lightmap_index: -1,
                    mipmap: true,
                }],
            )
            .expect("remap");
        assert!(prepared.accepted);
        assert_eq!(prepared.materials.len(), 1);
        let missing = registry
            .prepare_remap_materials(
                "textures/absent",
                &[SceneShaderBinding::Unlit {
                    lightmap_index: -1,
                    mipmap: true,
                }],
            )
            .expect("remap");
        assert!(!missing.accepted);
    }

    #[test]
    fn picture_orders_are_stable() {
        let mut registry = registry();
        let first = registry.register_picture("textures/rock", false).expect("picture");
        let second = registry.register_picture("textures/rock", false).expect("cached");
        assert_eq!(first, second);
    }
}
