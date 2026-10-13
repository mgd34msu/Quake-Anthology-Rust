//! Cold model registration. Native config ordinals resolve through filenames;
//! inline ordinals retain their explicit world and existing surface ranges.
use qa_compat::services::{CallError, ModelLoader, ModelLookup};
use qa_content::vfs::Vfs;
use qa_core::names::{NameTable, canonical_path};
use qa_core::primitives::{Bounds, IndexSpan, NameId, RuleSetId};
use qa_formats::archive::ArchiveReader;
use qa_formats::model::{Image, Mesh, Model, ModelFormat};
use qa_gameplay::rules::appearance::{self, AliasPose, ModelPresentation, ModelSource};
use qa_render::assets::{MaterialSettings, SoftwareAlias, Stage, StageTexture};
use qa_render::material::resources::{ImageSettings, ImageUse, Images, ImagesState, IndexedTarget};
use qa_render::shader::{
    AlphaFunc, AlphaGen, Cull, DepthFunc, RgbGen, ShaderCatalog, ShaderDef, ShaderStage,
    StageBlend, TexCoordGen, TextureMap,
};
use qa_render::surface_cache::IndexedTexture;
use qa_render::world::WorldId;
use qa_render::{Assets, MaterialId, ModelId, PaletteId, Vertex};

#[derive(Clone, Copy, Debug)]
pub struct ModelRequest<'a> {
    pub path: &'a str,
    /// File decoding follows the resource's provenance, independently of upload.
    pub source_rules: RuleSetId,
    /// Image upload policy is independent of model format and map geometry.
    pub rules: RuleSetId,
    pub source_palette: Option<PaletteId>,
    pub palette: Option<PaletteId>,
    pub image_settings: ImageSettings,
    /// Unsupported optional precaches are explicit diagnostics, never a mesh
    /// placeholder. A required asset instead rejects the load.
    pub required: bool,
}
impl<'a> ModelRequest<'a> {
    pub fn native(path: &'a str, rules: RuleSetId, palette: Option<PaletteId>) -> Self {
        Self {
            path,
            source_rules: rules,
            rules,
            source_palette: palette,
            palette,
            image_settings: ImageSettings::native(rules),
            required: true,
        }
    }
    pub fn profile(self) -> ModelProfile {
        ModelProfile {
            source_rules: self.source_rules,
            rules: self.rules,
            source_palette: self.source_palette,
            palette: self.palette,
            image_settings: self.image_settings,
        }
    }
}

/// File provenance and presentation are explicit registration choices. They
/// never alter the filename's exact NameId or the shared decoded mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelProfile {
    pub source_rules: RuleSetId,
    pub rules: RuleSetId,
    pub source_palette: Option<PaletteId>,
    pub palette: Option<PaletteId>,
    pub image_settings: ImageSettings,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelRegistrationId(u32);

#[derive(Clone, Copy, Debug)]
pub struct ModelLimits {
    pub models: usize,
    pub name_bytes: usize,
    pub skins: usize,
    pub frame_vertices: usize,
    pub indices: usize,
    pub inline_models: usize,
    pub file_bytes: usize,
}
impl Default for ModelLimits {
    fn default() -> Self {
        Self {
            models: 4096,
            name_bytes: 4 * 1024 * 1024,
            skins: 65536,
            frame_vertices: 16 * 1024 * 1024,
            indices: 16 * 1024 * 1024,
            inline_models: 65536,
            file_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelBinding {
    pub model: ModelId,
    pub material: MaterialId,
    /// Native model presentation chosen at registration, independent of wire.
    pub pose: AliasPose,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelMaterialPolicy {
    pub translucent: bool,
    pub fullbright: bool,
}
impl ModelMaterialPolicy {
    const ALL: [Self; 4] = [
        Self {
            translucent: false,
            fullbright: false,
        },
        Self {
            translucent: false,
            fullbright: true,
        },
        Self {
            translucent: true,
            fullbright: false,
        },
        Self {
            translucent: true,
            fullbright: true,
        },
    ];
    fn index(self) -> usize {
        usize::from(self.translucent) * 2 + usize::from(self.fullbright)
    }
}
#[derive(Clone, Copy)]
struct ModelMaterials {
    variants: [ModelBinding; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineBinding {
    pub world: WorldId,
    pub index: u32,
    pub surfaces: IndexSpan,
}
#[derive(Debug)]
pub struct ModelDiagnostic {
    pub name: NameId,
    pub profile: ModelProfile,
    pub error: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelCapacity {
    pub names: usize,
    pub registrations: usize,
    pub name_bytes: usize,
    pub bindings: usize,
    pub skins: usize,
    pub frame_vertices: usize,
    pub indices: usize,
    pub inline_bindings: usize,
}

struct Registration {
    path: String,
    profile: ModelProfile,
    required: bool,
}
struct RegisteredProfile {
    profile: ModelProfile,
    skins: Option<IndexSpan>,
    error: Option<String>,
}
struct RegisteredShader {
    name: NameId,
    profiles: IndexSpan,
    ready: bool,
}
struct ShaderProfileBinding {
    profile: ModelProfile,
    material: Option<MaterialId>,
}

/// File-native metadata survives unsupported renderer features. It is not the
/// bounds of an interpolated mesh or a selected presentation profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelMetadata {
    pub format: ModelFormat,
    pub bounds: Bounds,
}
#[derive(Clone)]
struct FileError {
    kind: CallError,
    message: String,
}
impl FileError {
    fn capacity(message: &str) -> Self {
        Self {
            kind: CallError::Capacity,
            message: message.to_owned(),
        }
    }
}
enum ModelFile {
    Empty,
    Ready(ModelMetadata),
    LoadedWorld {
        binding: InlineBinding,
        bounds: Bounds,
    },
    Failed(FileError),
}
impl ModelFile {
    fn bounds(&self) -> Option<Bounds> {
        match self {
            Self::Ready(metadata) => Some(metadata.bounds),
            Self::LoadedWorld { bounds, .. } => Some(*bounds),
            Self::Empty | Self::Failed(_) => None,
        }
    }
}
struct ProfileImages {
    profile: ModelProfile,
    model_policy: ModelPolicy,
    // Temporarily taken only while Images borrows the separately owned Assets.
    state: Option<Result<ImagesState, String>>,
}
#[derive(Clone, Copy)]
struct ModelPolicy {
    source: ModelSource,
    presentation: ModelPresentation,
}
impl ModelProfile {
    fn model_policy(self) -> ModelPolicy {
        ModelPolicy {
            source: appearance::model_source(self.source_rules),
            presentation: appearance::model_presentation(self.rules),
        }
    }
}

pub struct ModelCatalog {
    names: NameTable,
    files: Box<[ModelFile]>,
    models: Box<[IndexSpan]>,
    shader_indices: Box<[Option<u32>]>,
    shaders: Box<[RegisteredShader]>,
    shader_bindings: Box<[ShaderProfileBinding]>,
    registrations: Box<[RegisteredProfile]>,
    bindings: Box<[ModelMaterials]>,
    worlds: Box<[IndexSpan]>,
    inline: Box<[InlineBinding]>,
    capacity: ModelCapacity,
    pub diagnostics: Box<[ModelDiagnostic]>,
}

/// Owns cold Assets until every native precache and configured presentation
/// finishes. Parsed model bytes live only within one file's registration.
pub struct ModelCatalogBuilder {
    assets: Assets,
    reader: ArchiveReader,
    names: NameTable,
    files: Vec<ModelFile>,
    models: Vec<IndexSpan>,
    shader_indices: Vec<Option<u32>>,
    shaders: Vec<RegisteredShader>,
    shader_bindings: Vec<ShaderProfileBinding>,
    shader_catalog: Option<Result<ShaderCatalog, String>>,
    profiles: Vec<ProfileImages>,
    registrations: Vec<RegisteredProfile>,
    bindings: Vec<ModelMaterials>,
    worlds: Box<[IndexSpan]>,
    inline: Box<[InlineBinding]>,
    capacity: ModelCapacity,
    limits: ModelLimits,
    diagnostics: Vec<ModelDiagnostic>,
}
impl ModelCatalogBuilder {
    pub fn new(
        assets: Assets,
        profiles: impl IntoIterator<Item = ModelProfile>,
        worlds: impl IntoIterator<Item = WorldId>,
        limits: ModelLimits,
    ) -> Result<Self, String> {
        let names = NameTable::load_reserved([], limits.models, limits.name_bytes)
            .map_err(|error| format!("model names: {error}"))?;
        let mut unique = Vec::<ProfileImages>::new();
        for profile in profiles {
            if unique.iter().any(|entry| entry.profile == profile) {
                continue;
            }
            if unique.len() == limits.models {
                return Err("model registration capacity".to_owned());
            }
            let state = ImagesState::new(
                profile.source_rules,
                profile.rules,
                profile.image_settings,
                profile.source_palette.zip(profile.palette),
            )
            .map_err(|error| format!("model images: {error:?}"));
            unique.push(ProfileImages {
                profile,
                model_policy: profile.model_policy(),
                state: Some(state),
            });
        }
        let (worlds, inline) = inline_bindings(&assets, worlds, limits)?;
        let capacity = ModelCapacity {
            names: 1,
            inline_bindings: inline.len(),
            ..ModelCapacity::default()
        };
        Ok(Self {
            assets,
            reader: ArchiveReader::default(),
            names,
            files: vec![ModelFile::Empty],
            models: vec![IndexSpan::default()],
            shader_indices: vec![None],
            shaders: Vec::new(),
            shader_bindings: Vec::new(),
            shader_catalog: None,
            profiles: unique,
            registrations: Vec::new(),
            bindings: Vec::new(),
            worlds,
            inline,
            capacity,
            limits,
            diagnostics: Vec::new(),
        })
    }

    fn admit_name(&mut self, bytes: &[u8]) -> Result<NameId, CallError> {
        let previous = self.names.len();
        let name = self.names.intern(bytes).map_err(|_| CallError::Capacity)?;
        if self.names.len() != previous {
            self.files.push(ModelFile::Empty);
            self.models.push(IndexSpan::default());
            self.shader_indices.push(None);
            self.capacity.names = self.names.len();
            self.capacity.name_bytes += bytes.len();
        }
        Ok(name)
    }

    /// Admit the already-loaded world as native model metadata. Its surface
    /// binding and collision bounds do not create a mesh or decode another file.
    pub fn bind_loaded_world(
        &mut self,
        path: &str,
        world: WorldId,
        native_bounds: Bounds,
    ) -> Result<(), String> {
        if path.is_empty() || path.starts_with('*') {
            return Err("loaded world filename".to_owned());
        }
        if !native_bounds.is_valid() {
            return Err("loaded world bounds".to_owned());
        }
        let binding =
            inline_binding(&self.worlds, &self.inline, world, 0).ok_or("loaded world model0")?;
        let path = canonical_path(path);
        let name = self
            .admit_name(path.as_bytes())
            .map_err(|_| "model name capacity".to_owned())?;
        if !matches!(self.files[name.0 as usize], ModelFile::Empty) {
            return match self.files.get(name.0 as usize) {
                Some(ModelFile::LoadedWorld {
                    binding: existing,
                    bounds,
                }) if *existing == binding && *bounds == native_bounds => Ok(()),
                _ => Err("loaded world filename conflict".to_owned()),
            };
        }
        self.files[name.0 as usize] = ModelFile::LoadedWorld {
            binding,
            bounds: native_bounds,
        };
        Ok(())
    }

    /// Static optional requests and native synchronous requests share exactly
    /// one file admission, decode and registration path.
    pub fn register_request(&mut self, vfs: &Vfs, request: ModelRequest<'_>) -> Result<(), String> {
        if request.path.is_empty() || request.path.starts_with('*') {
            return Ok(());
        }
        let path = canonical_path(request.path);
        let admission = self.ensure_file(vfs, &path);
        let Some(name) = self.names.find(path.as_bytes()) else {
            return Err(admission
                .err()
                .map_or_else(|| "missing registered model name".to_owned(), |e| e.message));
        };
        if matches!(self.files[name.0 as usize], ModelFile::LoadedWorld { .. }) {
            return Ok(());
        }
        let profile = request.profile();
        let registration = self.models[name.0 as usize]
            .range()
            .map(|index| &self.registrations[index])
            .find(|entry| entry.profile == profile)
            .ok_or("unconfigured model profile")?;
        if request.required
            && let Some(error) = &registration.error
        {
            return Err(if error.starts_with("model images: ") {
                error.clone()
            } else {
                format!("model {path}: {error}")
            });
        }
        Ok(())
    }

    fn ensure_file(&mut self, vfs: &Vfs, path: &str) -> Result<Bounds, FileError> {
        if let Some(name) = self.names.find(path.as_bytes()) {
            let file = &self.files[name.0 as usize];
            if !matches!(file, ModelFile::Empty) {
                return file.bounds().ok_or_else(|| match file {
                    ModelFile::Failed(error) => error.clone(),
                    _ => FileError {
                        kind: CallError::File,
                        message: "empty model filename".to_owned(),
                    },
                });
            }
        }
        let next = self
            .registrations
            .len()
            .checked_add(self.profiles.len())
            .filter(|&count| count <= self.limits.models && count <= u32::MAX as usize)
            .ok_or_else(|| FileError::capacity("model registration capacity"))?;
        let name = self
            .admit_name(path.as_bytes())
            .map_err(|_| FileError::capacity("model name capacity"))?;
        let span = IndexSpan {
            first: self.registrations.len() as u32,
            count: (next - self.registrations.len()) as u32,
        };
        self.models[name.0 as usize] = span;
        let bytes = vfs
            .read_file(path.as_bytes(), self.limits.file_bytes, &mut self.reader)
            .map_err(|error| FileError {
                kind: CallError::File,
                message: error.to_string(),
            });
        let parsed = bytes.as_ref().map_err(Clone::clone).and_then(|bytes| {
            Model::parse(bytes).map_err(|error| FileError {
                kind: CallError::File,
                message: format!("decode: {error:?}"),
            })
        });
        let metadata = parsed
            .as_ref()
            .map_err(Clone::clone)
            .map(|model| ModelMetadata {
                format: model.format,
                bounds: model.bounds,
            });
        self.files[name.0 as usize] = match &metadata {
            Ok(metadata) => ModelFile::Ready(*metadata),
            Err(error) => ModelFile::Failed(error.clone()),
        };
        let mut mesh = None;
        for images in &mut self.profiles {
            let registration = Registration {
                path: path.to_owned(),
                profile: images.profile,
                required: false,
            };
            let result = match images.state.take() {
                Some(Ok(state)) => {
                    let mut temporary = Images::from_state(vfs, &mut self.assets, state);
                    let result = parsed
                        .as_ref()
                        .map_err(|error| error.message.clone())
                        .and_then(|model| {
                            validate_model(model)?;
                            register(
                                model,
                                &mut temporary,
                                &registration,
                                images.model_policy,
                                &mut mesh,
                                &mut self.capacity,
                                self.limits,
                            )
                        });
                    images.state = Some(Ok(temporary.into_state()));
                    result
                }
                Some(Err(error)) => {
                    images.state = Some(Err(error.clone()));
                    Err(error)
                }
                None => {
                    return Err(FileError {
                        kind: CallError::Aborted,
                        message: "model image state already borrowed".to_owned(),
                    });
                }
            };
            let (skins, error) = match result {
                Ok(skins) => {
                    let first = self.bindings.len() as u32;
                    let count = skins.len() as u32;
                    self.bindings.extend(skins);
                    (Some(IndexSpan { first, count }), None)
                }
                Err(error) => {
                    self.diagnostics.push(ModelDiagnostic {
                        name,
                        profile: images.profile,
                        error: error.clone(),
                    });
                    (None, Some(error))
                }
            };
            self.registrations.push(RegisteredProfile {
                profile: images.profile,
                skins,
                error,
            });
        }
        self.capacity.registrations = self.registrations.len();
        self.capacity.bindings = self.bindings.len() * ModelMaterialPolicy::ALL.len();
        metadata.map(|metadata| metadata.bounds)
    }

    fn admit_shader(
        &mut self,
        vfs: &Vfs,
        raw: &[u8],
        source: RuleSetId,
        mipmaps: bool,
    ) -> Result<i32, CallError> {
        if raw.is_empty() {
            return Ok(0);
        }
        if raw.len() >= 64 {
            return Err(CallError::Text);
        }
        if let Some(handle) = shader_handle(&self.names, &self.shader_indices, &self.shaders, raw) {
            return Ok(handle);
        }
        let full_name = std::str::from_utf8(raw).map_err(|_| CallError::Text)?;
        let key = shader_key(raw);
        let display = std::str::from_utf8(key).map_err(|_| CallError::Text)?;
        let count = self
            .profiles
            .iter()
            .filter(|entry| entry.profile.source_rules == source)
            .count();
        let next = self
            .shader_bindings
            .len()
            .checked_add(count)
            .filter(|&n| n <= self.limits.models && n <= u32::MAX as usize)
            .ok_or(CallError::Capacity)?;
        if self.shaders.len() >= self.limits.models || self.shaders.len() >= i32::MAX as usize {
            return Err(CallError::Capacity);
        }
        let name = self.admit_name(key)?;
        let group = self.names.folded(name).ok_or(CallError::Aborted)?;
        let index = self.shaders.len() as u32;
        self.shader_indices[group.0 as usize] = Some(index);
        let span = IndexSpan {
            first: self.shader_bindings.len() as u32,
            count: (next - self.shader_bindings.len()) as u32,
        };
        self.shaders.push(RegisteredShader {
            name,
            profiles: span,
            ready: false,
        });
        let catalog = self.shader_catalog.get_or_insert_with(|| {
            qa_render::material::load_catalog(vfs)
                .map_err(|error| format!("shader scripts: {error:?}"))
        });
        let definition = match catalog {
            Ok(catalog) => Ok(catalog
                .find_canonical(&canonical_path(display))
                .cloned()
                .unwrap_or_else(|| implicit_shader(display, full_name, mipmaps))),
            Err(error) => Err(error.clone()),
        };
        // The same compiler serves world and native LIGHTMAP_2D registrations.
        // Native negative lightmap indices resolve explicit $lightmap to white.
        let definition = definition.map(|mut definition| {
            for stage in &mut definition.stages {
                if matches!(stage.map, Some(TextureMap::Lightmap)) {
                    stage.map = Some(TextureMap::White);
                }
            }
            definition
        });
        let mut ready = false;
        for images in self
            .profiles
            .iter_mut()
            .filter(|entry| entry.profile.source_rules == source)
        {
            let result = match images.state.take() {
                Some(Ok(state)) => {
                    let mut temporary = Images::from_state(vfs, &mut self.assets, state);
                    let result = definition
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|definition| {
                            if !definition.valid {
                                return Err("invalid shader definition".to_owned());
                            }
                            qa_render::material::world_load::compile_definition(
                                definition,
                                &mut temporary,
                            )
                            .map_err(|error| format!("shader material: {error:?}"))
                        });
                    images.state = Some(Ok(temporary.into_state()));
                    result
                }
                Some(Err(error)) => {
                    images.state = Some(Err(error.clone()));
                    Err(error)
                }
                None => Err("shader image state already borrowed".to_owned()),
            };
            let material = match result {
                Ok(material) => {
                    ready = true;
                    Some(material)
                }
                Err(error) => {
                    self.diagnostics.push(ModelDiagnostic {
                        name,
                        profile: images.profile,
                        error,
                    });
                    None
                }
            };
            self.shader_bindings.push(ShaderProfileBinding {
                profile: images.profile,
                material,
            });
        }
        self.shaders[index as usize].ready = ready;
        if ready {
            Ok(index as i32 + 1)
        } else {
            Err(CallError::File)
        }
    }

    pub fn finish(self) -> (Assets, ModelCatalog) {
        let catalog = ModelCatalog {
            names: self.names,
            files: self.files.into_boxed_slice(),
            models: self.models.into_boxed_slice(),
            shader_indices: self.shader_indices.into_boxed_slice(),
            shaders: self.shaders.into_boxed_slice(),
            shader_bindings: self.shader_bindings.into_boxed_slice(),
            registrations: self.registrations.into_boxed_slice(),
            bindings: self.bindings.into_boxed_slice(),
            worlds: self.worlds,
            inline: self.inline,
            capacity: self.capacity,
            diagnostics: self.diagnostics.into_boxed_slice(),
        };
        (self.assets, catalog)
    }
}

fn shader_key(name: &[u8]) -> &[u8] {
    &name[..name
        .iter()
        .position(|&byte| byte == b'.')
        .unwrap_or(name.len())]
}
fn shader_handle(
    names: &NameTable,
    indices: &[Option<u32>],
    shaders: &[RegisteredShader],
    raw: &[u8],
) -> Option<i32> {
    if raw.is_empty() {
        return Some(0);
    }
    let group = names.find_folded(shader_key(raw))?;
    let index = indices.get(group.0 as usize).copied().flatten()?;
    let entry = shaders.get(index as usize)?;
    Some(if entry.ready { index as i32 + 1 } else { 0 })
}
fn implicit_shader(display: &str, image: &str, mipmaps: bool) -> ShaderDef {
    ShaderDef {
        name: display.to_owned(),
        source: String::new(),
        line: 0,
        valid: true,
        cull: Cull::Front,
        sort: 9.0,
        sort_explicit: false,
        surface_flags: 0,
        content_flags: 0,
        sky: None,
        fog: None,
        sun: None,
        polygon_offset: false,
        no_mipmaps: !mipmaps,
        no_picmip: !mipmaps,
        entity_mergable: false,
        portal: false,
        clamp_time: None,
        stages: vec![ShaderStage {
            map: Some(TextureMap::Image {
                name: image.to_owned(),
                clamp: !mipmaps,
            }),
            blend: Some(StageBlend::ALPHA),
            rgb_gen: RgbGen::Vertex,
            alpha_gen: AlphaGen::Vertex,
            tc_gen: TexCoordGen::Texture,
            tc_mods: Box::new([]),
            alpha_func: AlphaFunc::None,
            depth_func: DepthFunc::Always,
            depth_write: false,
            detail: false,
            unsupported: Box::new([]),
        }]
        .into_boxed_slice(),
        deforms: Box::new([]),
        unsupported: Box::new([]),
    }
}

fn file_lookup<'a>(
    names: &NameTable,
    files: &'a [ModelFile],
    path: &[u8],
) -> Option<&'a ModelFile> {
    let name = names.find_path(std::str::from_utf8(path).ok()?)?;
    files.get(name.0 as usize)
}
impl ModelLookup for ModelCatalogBuilder {
    fn bounds(&self, path: &[u8], _source: RuleSetId) -> Option<Bounds> {
        file_lookup(&self.names, &self.files, path)?.bounds()
    }
    fn shader_handle(&self, name: &[u8]) -> Option<i32> {
        shader_handle(&self.names, &self.shader_indices, &self.shaders, name)
    }
}
impl ModelLoader for ModelCatalogBuilder {
    fn precache(
        &mut self,
        vfs: &Vfs,
        path: &[u8],
        _source: RuleSetId,
    ) -> Result<Bounds, CallError> {
        let path = std::str::from_utf8(path).map_err(|_| CallError::Text)?;
        if path.is_empty() || path.starts_with('*') {
            return Err(CallError::File);
        }
        self.ensure_file(vfs, &canonical_path(path))
            .map_err(|error| error.kind)
    }
    fn register_shader(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
        mipmaps: bool,
    ) -> Result<i32, CallError> {
        self.admit_shader(vfs, name, source, mipmaps)
    }
}
impl ModelLookup for ModelCatalog {
    fn bounds(&self, path: &[u8], _source: RuleSetId) -> Option<Bounds> {
        file_lookup(&self.names, &self.files, path)?.bounds()
    }
    fn shader_handle(&self, name: &[u8]) -> Option<i32> {
        shader_handle(&self.names, &self.shader_indices, &self.shaders, name)
    }
}
impl ModelCatalog {
    pub fn shader_binding(&self, handle: i32, profile: ModelProfile) -> Option<MaterialId> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.shaders
            .get(index)?
            .profiles
            .range()
            .map(|i| &self.shader_bindings[i])
            .find(|entry| entry.profile == profile)?
            .material
    }
    pub fn shader_name(&self, handle: i32) -> Option<&[u8]> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.names.get(self.shaders.get(index)?.name)
    }
    /// All requests delegate to the same staged native precache implementation.
    pub fn load<'a>(
        vfs: &Vfs,
        assets: Assets,
        requests: impl IntoIterator<Item = ModelRequest<'a>>,
        worlds: impl IntoIterator<Item = WorldId>,
        limits: ModelLimits,
    ) -> Result<(Assets, Self), String> {
        let mut pending = Vec::<Registration>::new();
        for request in requests {
            if request.path.is_empty() || request.path.starts_with('*') {
                continue;
            }
            if pending.len() == limits.models || pending.len() == u32::MAX as usize {
                return Err("model registration capacity".to_owned());
            }
            pending.push(Registration {
                path: canonical_path(request.path),
                profile: request.profile(),
                required: request.required,
            });
        }
        pending.sort_by(|left, right| left.path.cmp(&right.path));
        let mut builder = ModelCatalogBuilder::new(
            assets,
            pending.iter().map(|entry| entry.profile),
            worlds,
            limits,
        )?;
        for entry in &pending {
            builder.register_request(
                vfs,
                ModelRequest {
                    path: &entry.path,
                    source_rules: entry.profile.source_rules,
                    rules: entry.profile.rules,
                    source_palette: entry.profile.source_palette,
                    palette: entry.profile.palette,
                    image_settings: entry.profile.image_settings,
                    required: entry.required,
                },
            )?;
        }
        Ok(builder.finish())
    }
    pub fn metadata(&self, path: &[u8]) -> Option<ModelMetadata> {
        match file_lookup(&self.names, &self.files, path)? {
            ModelFile::Ready(metadata) => Some(*metadata),
            ModelFile::Empty | ModelFile::LoadedWorld { .. } | ModelFile::Failed(_) => None,
        }
    }
    /// Resolve both filename and profile only at registration/configuration
    /// boundaries. A profile mismatch never selects the first same-name entry.
    pub fn registration(&self, path: &str, profile: ModelProfile) -> Option<ModelRegistrationId> {
        let name = self.names.find_path(path)?;
        let span = self.models[name.0 as usize];
        span.range()
            .find(|&index| {
                let entry = &self.registrations[index];
                entry.profile == profile && entry.skins.is_some()
            })
            .map(|index| ModelRegistrationId(index as u32))
    }
    pub fn name(&self, name: NameId) -> Option<&str> {
        std::str::from_utf8(self.names.get(name)?).ok()
    }
    /// Frame lookup uses only cached numeric registration, skin and policy.
    pub fn binding(
        &self,
        registration: ModelRegistrationId,
        skin: u32,
        policy: ModelMaterialPolicy,
    ) -> Option<ModelBinding> {
        let span = self.registrations.get(registration.0 as usize)?.skins?;
        let skin = if skin < span.count { skin } else { 0 };
        Some(self.bindings.get((span.first + skin) as usize)?.variants[policy.index()])
    }
    pub fn resolve_inline(&self, world: WorldId, index: u32) -> Option<InlineBinding> {
        inline_binding(&self.worlds, &self.inline, world, index)
    }
    pub fn capacity(&self) -> ModelCapacity {
        self.capacity
    }
}

fn inline_binding(
    worlds: &[IndexSpan],
    inline: &[InlineBinding],
    world: WorldId,
    index: u32,
) -> Option<InlineBinding> {
    let span = *worlds.get(world.0 as usize)?;
    if index >= span.count {
        return None;
    }
    inline.get((span.first + index) as usize).copied()
}

fn inline_bindings(
    assets: &Assets,
    worlds: impl IntoIterator<Item = WorldId>,
    limits: ModelLimits,
) -> Result<(Box<[IndexSpan]>, Box<[InlineBinding]>), String> {
    let mut ranges = vec![IndexSpan::default(); assets.worlds().len()].into_boxed_slice();
    let mut inline = Vec::new();
    for id in worlds {
        let range = ranges
            .get_mut(id.0 as usize)
            .ok_or("inline model world handle")?;
        if range.count != 0 {
            continue;
        }
        let geometry = assets
            .world(id)
            .ok_or("inline model world handle")?
            .geometry();
        let next = inline
            .len()
            .checked_add(geometry.models.len())
            .filter(|&count| count <= limits.inline_models && count <= u32::MAX as usize)
            .ok_or("inline model capacity")?;
        let first = inline.len() as u32;
        for (index, model) in geometry.models.iter().enumerate() {
            inline.push(InlineBinding {
                world: id,
                index: index as u32,
                surfaces: model.surfaces,
            });
        }
        *range = IndexSpan {
            first,
            count: (next - first as usize) as u32,
        };
    }
    Ok((ranges, inline.into_boxed_slice()))
}

fn validate_model(model: &Model<'_>) -> Result<(), String> {
    if !matches!(model.format, ModelFormat::Mdl | ModelFormat::Md2) {
        return Err(format!(
            "model format not yet registered: {:?}",
            model.format
        ));
    }
    if model
        .skin_groups
        .iter()
        .any(|group| group.members.len() != 1)
        || model
            .frame_groups
            .iter()
            .any(|group| group.members.len() != 1)
    {
        return Err("grouped MDL frame/skin timing is not yet registered".to_owned());
    }
    if model.skins.is_empty() {
        return Err("model requires an explicit external skin".to_owned());
    }
    Ok(())
}

fn register(
    model: &Model<'_>,
    images: &mut Images<'_>,
    registration: &Registration,
    model_policy: ModelPolicy,
    shared_mesh: &mut Option<ModelId>,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<Vec<ModelMaterials>, String> {
    let skin_count = capacity
        .skins
        .checked_add(model.skins.len())
        .filter(|&count| count <= limits.skins && count <= u32::MAX as usize)
        .ok_or("model skin capacity")?;
    let options = registration.profile;
    let usage = ImageUse::default();
    let mut materials = Vec::with_capacity(model.skins.len());
    for (index, skin) in model.skins.iter().enumerate() {
        let image = match *skin {
            Image::Indexed {
                width,
                height,
                pixels,
            } => {
                let palette = options
                    .source_palette
                    .ok_or("indexed model requires a source palette")?;
                let texture = IndexedTexture::load_base(width, height, pixels, None)?;
                let image = images
                    .register_indexed(texture, palette)
                    .map_err(|error| format!("indexed skin: {error:?}"))?;
                let params = images
                    .upload_params(usage)
                    .map_err(|error| format!("skin upload: {error:?}"))?;
                images.assets.prepare_image(image, params)?;
                images
                    .assets
                    .set_image_native_sampler(image, usage.sampler)?;
                image
            }
            Image::External(path) => {
                let path = std::str::from_utf8(path).map_err(|_| "skin path is not UTF-8")?;
                if model_policy.source.indexed_external_pcx
                    && qa_formats::image::ImageFormat::from_path(path.as_bytes())
                        == Some(qa_formats::image::ImageFormat::Pcx)
                {
                    images
                        .indexed_pcx(
                            path,
                            options
                                .source_palette
                                .ok_or("PCX skin requires a source palette")?,
                            None,
                            None,
                            usage,
                        )
                        .map_err(|error| format!("skin {path}: {error:?}"))?
                } else {
                    images
                        .raster(
                            path,
                            usage,
                            options.palette.map(|palette| IndexedTarget {
                                palette,
                                transparent_index: Some(255),
                            }),
                        )
                        .map_err(|error| format!("skin {path}: {error:?}"))?
                        .id
                }
            }
        };
        let mut variants = [MaterialId(0); 4];
        for policy in ModelMaterialPolicy::ALL {
            variants[policy.index()] = images.assets.register_material(
                &format!("{}#skin{index}:{}", registration.path, policy.index()),
                &[Stage {
                    texture: StageTexture::Image(image),
                    rgb_gen: if policy.fullbright {
                        RgbGen::Identity
                    } else {
                        RgbGen::LightingDiffuse
                    },
                    alpha_gen: AlphaGen::Entity,
                    blend: policy.translucent.then_some(StageBlend::ALPHA),
                    depth_write: !policy.translucent,
                    ..Stage::default()
                }],
                MaterialSettings {
                    sort: if policy.translucent { 6.0 } else { 3.0 },
                    software_alias: model_policy.presentation.software_alias.then_some(
                        SoftwareAlias {
                            fullbright_light: policy
                                .fullbright
                                .then_some(model_policy.presentation.fullbright_light),
                            alpha_cutoffs: policy
                                .translucent
                                .then_some(model_policy.presentation.alpha_cutoffs),
                        },
                    ),
                    ..MaterialSettings::default()
                },
            )?;
        }
        materials.push(variants);
    }
    let id = if let Some(id) = *shared_mesh {
        id
    } else {
        let mesh = model.meshes.first().ok_or("model has no mesh")?;
        let (vertices, frame_size, indices) = mesh_vertices(mesh, capacity, limits)?;
        let material = materials.first().ok_or("model has no skin material")?[0];
        let id = images
            .assets
            .register_animated_model(&vertices, frame_size, &indices, material)?;
        *shared_mesh = Some(id);
        id
    };
    capacity.skins = skin_count;
    Ok(materials
        .into_iter()
        .map(|materials| ModelMaterials {
            variants: materials.map(|material| ModelBinding {
                model: id,
                material,
                pose: model_policy.presentation.pose,
            }),
        })
        .collect())
}

fn mesh_vertices(
    mesh: &Mesh<'_>,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<(Vec<Vertex>, usize, Vec<u32>), String> {
    let index_count = mesh
        .triangles
        .len()
        .checked_mul(3)
        .ok_or("model index capacity")?;
    let next_indices = capacity
        .indices
        .checked_add(index_count)
        .filter(|&count| count <= limits.indices && count <= u32::MAX as usize)
        .ok_or("model index capacity")?;
    let mut pairs = Vec::with_capacity(index_count);
    for triangle in &mesh.triangles {
        pairs.extend(triangle.vertex.into_iter().zip(triangle.texcoord));
    }
    pairs.sort_unstable();
    pairs.dedup();
    let frame_count = mesh.vertices.len() / mesh.vertices_per_frame;
    let vertex_count = pairs
        .len()
        .checked_mul(frame_count)
        .ok_or("model vertex capacity")?;
    let next_vertices = capacity
        .frame_vertices
        .checked_add(vertex_count)
        .filter(|&count| count <= limits.frame_vertices && count <= u32::MAX as usize)
        .ok_or("model vertex capacity")?;
    let mut indices = Vec::with_capacity(index_count);
    for triangle in &mesh.triangles {
        for pair in triangle.vertex.into_iter().zip(triangle.texcoord) {
            indices.push(
                u32::try_from(
                    pairs
                        .binary_search(&pair)
                        .map_err(|_| "missing model vertex/UV pair")?,
                )
                .map_err(|_| "model index capacity")?,
            );
        }
    }
    let mut vertices = Vec::with_capacity(vertex_count);
    for frame in mesh.vertices.chunks_exact(mesh.vertices_per_frame) {
        for &(vertex, texcoord) in &pairs {
            let vertex = frame[vertex as usize];
            vertices.push(Vertex {
                position: vertex.position,
                normal: vertex.normal,
                texcoord: mesh.texcoords[texcoord as usize],
                ..Vertex::default()
            });
        }
    }
    capacity.frame_vertices = next_vertices;
    capacity.indices = next_indices;
    Ok((vertices, pairs.len(), indices))
}
