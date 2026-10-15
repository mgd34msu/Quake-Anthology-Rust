//! Cold model registration. Native config ordinals resolve through filenames;
//! inline ordinals retain their explicit world and existing surface ranges.
use qa_compat::services::{CallError, LoadedWorldBinding, ModelLoader, ModelLookup};
use qa_content::vfs::Vfs;
use qa_core::names::{NameTable, canonical_path};
use qa_core::primitives::{Bounds, IndexSpan, NameId, Orientation, RuleSetId};
use qa_formats::archive::ArchiveReader;
use qa_formats::model::{Image, Mesh, Model, ModelFormat};
use qa_gameplay::rules::appearance::{self, AliasPose, ModelPresentation, ModelSource};
use qa_render::assets::{ImageId, MaterialSettings, SoftwareAlias, Stage, StageTexture};
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
    pub surfaces: usize,
    pub tags: usize,
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
            surfaces: 65536,
            tags: 1024 * 1024,
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
    pub custom_skin: i32,
    pub frame: u32,
}
impl ModelMaterialPolicy {
    const ALL: [Self; 4] = [
        Self {
            translucent: false,
            fullbright: false,
            custom_skin: 0,
            frame: 0,
        },
        Self {
            translucent: false,
            fullbright: true,
            custom_skin: 0,
            frame: 0,
        },
        Self {
            translucent: true,
            fullbright: false,
            custom_skin: 0,
            frame: 0,
        },
        Self {
            translucent: true,
            fullbright: true,
            custom_skin: 0,
            frame: 0,
        },
    ];
    fn index(self) -> usize {
        usize::from(self.translucent) * 2 + usize::from(self.fullbright)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct ModelMaterials {
    variants: [ModelBinding; 4],
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct SurfaceMaterials {
    name: NameId,
    skins: IndexSpan,
    wrap_skins: bool,
    frame_skins: bool,
}

/// Numeric, allocation-free iteration over every surface of one registration.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelBindings<'a> {
    surfaces: &'a [SurfaceMaterials],
    materials: &'a [ModelMaterials],
    skin: u32,
    policy: ModelMaterialPolicy,
    custom: Option<&'a [SkinMapping]>,
}
impl Iterator for ModelBindings<'_> {
    type Item = ModelBinding;
    fn next(&mut self) -> Option<Self::Item> {
        let (surface, remaining) = self.surfaces.split_first()?;
        self.surfaces = remaining;
        let selected = if surface.frame_skins {
            self.policy.frame
        } else {
            self.skin
        };
        let skin = if surface.wrap_skins || surface.frame_skins {
            selected % surface.skins.count
        } else if selected < surface.skins.count {
            selected
        } else {
            0
        };
        let mut binding =
            self.materials[(surface.skins.first + skin) as usize].variants[self.policy.index()];
        if let Some(custom) = self.custom.filter(|_| !surface.frame_skins) {
            binding.material = custom
                .binary_search_by_key(&surface.name.0, |entry| entry.surface.0)
                .ok()
                .map_or(MaterialId(0), |index| custom[index].material);
        }
        Some(binding)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.surfaces.len(), Some(self.surfaces.len()))
    }
}
impl ExactSizeIterator for ModelBindings<'_> {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineBinding {
    pub world: WorldId,
    pub index: u32,
    pub surfaces: IndexSpan,
}
struct RegisteredInline {
    binding: InlineBinding,
    handle: Option<NameId>,
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
    pub surfaces: usize,
    pub tags: usize,
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
    surfaces: Option<IndexSpan>,
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
struct RegisteredSkin {
    ready: bool,
    profiles: Vec<SkinProfile>,
}
struct SkinProfile {
    profile: ModelProfile,
    mappings: Box<[SkinMapping]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct SkinMapping {
    surface: NameId,
    material: MaterialId,
}
/// File-native metadata survives unsupported renderer features. It is not the
/// bounds of an interpolated mesh or a selected presentation profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelMetadata {
    pub format: ModelFormat,
    pub bounds: Bounds,
    pub renderer_bounds: Bounds,
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
    Ready {
        metadata: ModelMetadata,
        renderer_sources: u8,
        tags: Option<TagLayout>,
    },
    LoadedWorld {
        binding: InlineBinding,
        bounds: Bounds,
        collision: Option<qa_core::primitives::GeometryId>,
    },
    Failed(FileError),
}
#[derive(Clone, Copy)]
struct TagLayout {
    samples: IndexSpan,
    frames: u32,
    tags_per_frame: u32,
}
#[derive(Clone, Copy)]
struct TaggedOrientation {
    name: NameId,
    orientation: Orientation,
}
impl ModelFile {
    fn bounds(&self) -> Option<Bounds> {
        match self {
            Self::Ready { metadata, .. } => Some(metadata.bounds),
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
    colors: Option<PaletteColors>,
}
#[derive(Clone, Copy, Debug)]
pub struct ModelColorId(u32);
pub struct PaletteColors {
    pub source: PaletteId,
    pub target: PaletteId,
    pub indices: [u8; 256],
}
impl PaletteColors {
    fn load(profile: ModelProfile, assets: &Assets) -> Option<Self> {
        let source = profile.source_palette?;
        let target = profile.palette.unwrap_or(source);
        let original = assets.palette(source)?;
        let destination = assets.palette(target)?;
        let indices = original.translation_to(destination, None);
        Some(Self {
            source,
            target,
            indices,
        })
    }
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
impl ProfileImages {
    fn compile(
        &mut self,
        vfs: &Vfs,
        assets: &mut Assets,
        definition: Result<&ShaderDef, String>,
    ) -> Result<MaterialId, String> {
        let Some(state) = self.state.take() else {
            return Err("shader image state already borrowed".to_owned());
        };
        match state {
            Ok(state) => {
                let mut images = Images::from_state(vfs, assets, state);
                let result =
                    definition.and_then(|definition| compile_shader(definition, &mut images));
                self.state = Some(Ok(images.into_state()));
                result
            }
            Err(error) => {
                self.state = Some(Err(error.clone()));
                Err(error)
            }
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
    skin_indices: Box<[Option<u32>]>,
    skins: Box<[RegisteredSkin]>,
    registrations: Box<[RegisteredProfile]>,
    bindings: Box<[ModelMaterials]>,
    surfaces: Box<[SurfaceMaterials]>,
    tags: Box<[TaggedOrientation]>,
    worlds: Box<[IndexSpan]>,
    inline: Box<[RegisteredInline]>,
    capacity: ModelCapacity,
    pub diagnostics: Box<[ModelDiagnostic]>,
    colors: Box<[(ModelProfile, PaletteColors)]>,
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
    skin_indices: Vec<Option<u32>>,
    skins: Vec<RegisteredSkin>,
    shader_catalog: Option<Result<ShaderCatalog, String>>,
    profiles: Vec<ProfileImages>,
    registrations: Vec<RegisteredProfile>,
    bindings: Vec<ModelMaterials>,
    surfaces: Vec<SurfaceMaterials>,
    tags: Vec<TaggedOrientation>,
    worlds: Box<[IndexSpan]>,
    inline: Box<[RegisteredInline]>,
    capacity: ModelCapacity,
    limits: ModelLimits,
    diagnostics: Vec<ModelDiagnostic>,
}
impl ModelCatalogBuilder {
    pub fn new(
        mut assets: Assets,
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
            if let Some((source, target)) = profile
                .source_palette
                .zip(profile.palette)
                .filter(|(source, target)| source != target)
                && matches!(
                    profile.source_rules,
                    RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
                )
                && assets.palette(source).is_some()
                && assets.palette(target).is_some()
            {
                assets.ensure_palette_translucency(target)?;
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
                colors: PaletteColors::load(profile, &assets),
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
            skin_indices: vec![None],
            skins: Vec::new(),
            shader_catalog: None,
            profiles: unique,
            registrations: Vec::new(),
            bindings: Vec::new(),
            surfaces: Vec::new(),
            tags: Vec::new(),
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
            self.skin_indices.push(None);
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
        collision: Option<qa_core::primitives::GeometryId>,
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
                    collision: existing_collision,
                }) if *existing == binding
                    && *bounds == native_bounds
                    && *existing_collision == collision =>
                {
                    Ok(())
                }
                _ => Err("loaded world filename conflict".to_owned()),
            };
        }
        self.files[name.0 as usize] = ModelFile::LoadedWorld {
            binding,
            bounds: native_bounds,
            collision,
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
        if self.models[name.0 as usize].count == 0
            && let Err(error) = admission
        {
            return Err(error.message);
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
                // R_ModelBounds exposes the first MD3 frame, rather than the
                // all-frame box used by model loading and collision adapters.
                renderer_bounds: match model.format {
                    ModelFormat::Md3 | ModelFormat::Mdc => model.frames[0].bounds,
                    _ => model.bounds,
                },
            });
        let first_tag = self.tags.len();
        let tags = parsed
            .as_ref()
            .map_err(|error| error.message.clone())
            .and_then(|model| self.admit_tags(model));
        let surface_names = parsed
            .as_ref()
            .ok()
            .map(|model| self.admit_surface_names(model))
            .transpose();
        let surface_names = match surface_names {
            Ok(names) => names,
            Err(error) => {
                self.tags.truncate(first_tag);
                self.capacity.tags = first_tag;
                self.files[name.0 as usize] = ModelFile::Failed(error.clone());
                return Err(error);
            }
        };
        let mut meshes = vec![None; parsed.as_ref().map_or(0, |model| model.meshes.len())];
        if parsed
            .as_ref()
            .is_ok_and(|model| model.format == ModelFormat::Md3)
        {
            self.shader_catalog(vfs);
        }
        let mut renderer_sources = 0;
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
                            tags.as_ref().map_err(Clone::clone)?;
                            register(
                                model,
                                &mut temporary,
                                &registration,
                                images.model_policy,
                                &mut meshes,
                                self.shader_catalog.as_ref(),
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
            let (surfaces, error) = match result {
                Ok(surfaces) => {
                    let first = self.surfaces.len() as u32;
                    let count = surfaces.len() as u32;
                    for (index, skins) in surfaces.into_iter().enumerate() {
                        self.surfaces.push(SurfaceMaterials {
                            name: surface_names
                                .as_ref()
                                .ok_or_else(|| FileError::capacity("model surface names"))?[index],
                            skins: IndexSpan {
                                first: self.bindings.len() as u32,
                                count: skins.len() as u32,
                            },
                            wrap_skins: parsed
                                .as_ref()
                                .is_ok_and(|model| model.format == ModelFormat::Md3),
                            frame_skins: parsed
                                .as_ref()
                                .is_ok_and(|model| model.format == ModelFormat::Sp2),
                        });
                        self.bindings.extend(skins);
                    }
                    renderer_sources |= 1 << images.profile.source_rules as u8;
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
                surfaces,
                error,
            });
        }
        self.models[name.0 as usize] = span;
        self.files[name.0 as usize] = match &metadata {
            Ok(metadata) => ModelFile::Ready {
                metadata: *metadata,
                renderer_sources,
                tags: tags.as_ref().ok().copied().flatten(),
            },
            Err(error) => ModelFile::Failed(error.clone()),
        };
        self.capacity.registrations = self.registrations.len();
        self.capacity.bindings = self.bindings.len() * ModelMaterialPolicy::ALL.len();
        metadata.map(|metadata| metadata.bounds)
    }

    fn admit_surface_names(&mut self, model: &Model<'_>) -> Result<Vec<NameId>, FileError> {
        let raw: Vec<_> = if model.format == ModelFormat::Sp2 {
            vec![b"sprite".as_slice()]
        } else {
            model
                .meshes
                .iter()
                .map(|mesh| {
                    let raw = mesh.name;
                    if model.format == ModelFormat::Md3
                        && raw.len() > 2
                        && raw[raw.len() - 2] == b'_'
                    {
                        &raw[..raw.len() - 2]
                    } else {
                        raw
                    }
                })
                .collect()
        };
        self.names
            .check_intern_all(raw.iter().copied())
            .map_err(|_| FileError::capacity("model surface names"))?;
        raw.into_iter()
            .map(|raw| {
                let name = self
                    .admit_name(raw)
                    .map_err(|_| FileError::capacity("model surface names"))?;
                self.names
                    .folded(name)
                    .ok_or_else(|| FileError::capacity("model surface names"))
            })
            .collect()
    }

    fn shader_catalog(&mut self, vfs: &Vfs) -> &Result<ShaderCatalog, String> {
        self.shader_catalog.get_or_insert_with(|| {
            qa_render::material::load_catalog(vfs)
                .map_err(|error| format!("shader scripts: {error:?}"))
        })
    }

    fn admit_tags(&mut self, model: &Model<'_>) -> Result<Option<TagLayout>, String> {
        if model.format != ModelFormat::Md3 || model.tags_per_frame == 0 {
            return Ok(None);
        }
        let first = self.tags.len();
        let next = first
            .checked_add(model.tags.len())
            .filter(|&count| count <= self.limits.tags && count <= u32::MAX as usize)
            .ok_or("model tag capacity")?;
        for frame in model.tags.chunks_exact(model.tags_per_frame) {
            let at = self.tags.len();
            for tag in frame {
                let name = match self.admit_name(tag.name) {
                    Ok(name) => name,
                    Err(_) => {
                        self.tags.truncate(first);
                        return Err("model tag name capacity".to_owned());
                    }
                };
                self.tags.push(TaggedOrientation {
                    name,
                    orientation: tag.orientation,
                });
            }
            // Preserve the original first match for duplicate names in a frame.
            self.tags[at..].sort_by_key(|tag| tag.name.0);
        }
        self.capacity.tags = next;
        Ok(Some(TagLayout {
            samples: IndexSpan {
                first: first as u32,
                count: model.tags.len() as u32,
            },
            frames: model.frames.len() as u32,
            tags_per_frame: model.tags_per_frame as u32,
        }))
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
        std::str::from_utf8(raw).map_err(|_| CallError::Text)?;
        let key = shader_key(raw);
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
        let definition = shader_definition(self.shader_catalog(vfs), raw, mipmaps, false);
        let mut ready = false;
        for images in self
            .profiles
            .iter_mut()
            .filter(|entry| entry.profile.source_rules == source)
        {
            let result = images.compile(
                vfs,
                &mut self.assets,
                definition.as_ref().map_err(Clone::clone),
            );
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

    fn admit_inline(&mut self, world: WorldId, index: u32) -> Result<i32, CallError> {
        let slot = inline_slot(&self.worlds, world, index).ok_or(CallError::File)?;
        if let Some(handle) = self.inline[slot].handle {
            return i32::try_from(handle.0).map_err(|_| CallError::Capacity);
        }
        let binding = self.inline[slot].binding;
        let bounds = self
            .assets
            .world(world)
            .and_then(|world| world.geometry().models.get(index as usize))
            .ok_or(CallError::File)?
            .bounds;
        // A native cstring cannot name this private semantic key, so an
        // ordinary file registration cannot overwrite the inline record.
        let key = format!("\0engine:inline/{}/{}", world.0, index);
        let handle = self.admit_name(key.as_bytes())?;
        let native = i32::try_from(handle.0).map_err(|_| CallError::Capacity)?;
        self.files[handle.0 as usize] = ModelFile::LoadedWorld {
            binding,
            bounds,
            collision: None,
        };
        self.inline[slot].handle = Some(handle);
        Ok(native)
    }

    fn admit_skin(&mut self, vfs: &Vfs, raw: &[u8], source: RuleSetId) -> Result<i32, CallError> {
        if raw.is_empty() {
            return Ok(0);
        }
        if raw.len() >= 64 {
            return Err(CallError::Text);
        }
        let path = canonical_path(std::str::from_utf8(raw).map_err(|_| CallError::Text)?);
        if let Some(handle) = skin_handle(
            &self.names,
            &self.skin_indices,
            &self.skins,
            path.as_bytes(),
        ) {
            return Ok(handle);
        }
        if self.skins.len() >= self.limits.models || self.skins.len() >= i32::MAX as usize {
            return Err(CallError::Capacity);
        }
        let name = self.admit_name(path.as_bytes())?;
        let group = self.names.folded(name).ok_or(CallError::Aborted)?;
        let index = self.skins.len() as u32;
        self.skin_indices[group.0 as usize] = Some(index);
        self.skins.push(RegisteredSkin {
            ready: false,
            profiles: Vec::new(),
        });
        let profile_count = self
            .profiles
            .iter()
            .filter(|entry| entry.profile.source_rules == source)
            .count();
        let maximum_rows =
            self.limits.skins.saturating_sub(self.capacity.skins) / profile_count.max(1);
        let skin_file = raw.ends_with(b".skin");
        let bytes = if skin_file {
            vfs.read_file(path.as_bytes(), self.limits.file_bytes, &mut self.reader)
                .map_err(|_| CallError::File)?
        } else {
            Vec::new()
        };
        let rows = if skin_file {
            let mut rows = Vec::new();
            for mapping in qa_formats::model::skin::SkinMappings::new(&bytes) {
                if rows.len() == maximum_rows {
                    return Err(CallError::Capacity);
                }
                rows.push(mapping.map_err(|_| CallError::Text)?);
            }
            rows
        } else {
            vec![qa_formats::model::skin::SkinMapping {
                surface: b"",
                shader: raw,
            }]
        };
        if rows.is_empty() {
            return Err(CallError::File);
        }
        self.capacity.skins = rows
            .len()
            .checked_mul(profile_count)
            .and_then(|count| self.capacity.skins.checked_add(count))
            .filter(|&count| count <= self.limits.skins)
            .ok_or(CallError::Capacity)?;
        self.shader_catalog(vfs);
        let mut mappings = Vec::with_capacity(rows.len());
        for row in rows {
            let surface = self.admit_name(row.surface)?;
            let surface = self.names.folded(surface).ok_or(CallError::Aborted)?;
            let definition = shader_definition(
                self.shader_catalog.as_ref().ok_or(CallError::Aborted)?,
                row.shader,
                true,
                true,
            );
            mappings.push((surface, definition));
        }
        let mut profiles = Vec::with_capacity(profile_count);
        for images in self
            .profiles
            .iter_mut()
            .filter(|entry| entry.profile.source_rules == source)
        {
            let mut compiled = Vec::with_capacity(mappings.len());
            for (surface, definition) in &mappings {
                let material = match images.compile(
                    vfs,
                    &mut self.assets,
                    definition.as_ref().map_err(Clone::clone),
                ) {
                    Ok(material) => material,
                    Err(error) => {
                        self.diagnostics.push(ModelDiagnostic {
                            name,
                            profile: images.profile,
                            error,
                        });
                        MaterialId(0)
                    }
                };
                compiled.push(SkinMapping {
                    surface: *surface,
                    material,
                });
            }
            compiled.sort_by_key(|entry| entry.surface.0);
            compiled.dedup_by_key(|entry| entry.surface.0);
            profiles.push(SkinProfile {
                profile: images.profile,
                mappings: compiled.into_boxed_slice(),
            });
        }
        self.skins[index as usize] = RegisteredSkin {
            ready: true,
            profiles,
        };
        Ok(index as i32 + 1)
    }

    pub fn finish(self) -> (Assets, ModelCatalog) {
        let catalog = ModelCatalog {
            names: self.names,
            files: self.files.into_boxed_slice(),
            models: self.models.into_boxed_slice(),
            shader_indices: self.shader_indices.into_boxed_slice(),
            shaders: self.shaders.into_boxed_slice(),
            shader_bindings: self.shader_bindings.into_boxed_slice(),
            skin_indices: self.skin_indices.into_boxed_slice(),
            skins: self.skins.into_boxed_slice(),
            registrations: self.registrations.into_boxed_slice(),
            bindings: self.bindings.into_boxed_slice(),
            surfaces: self.surfaces.into_boxed_slice(),
            tags: self.tags.into_boxed_slice(),
            worlds: self.worlds,
            inline: self.inline,
            capacity: self.capacity,
            diagnostics: self.diagnostics.into_boxed_slice(),
            colors: self
                .profiles
                .into_iter()
                .filter_map(|entry| entry.colors.map(|colors| (entry.profile, colors)))
                .collect(),
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
fn skin_handle(
    names: &NameTable,
    indices: &[Option<u32>],
    skins: &[RegisteredSkin],
    raw: &[u8],
) -> Option<i32> {
    if raw.is_empty() {
        return Some(0);
    }
    let group = names.find_path_bytes(raw)?;
    let group = names.folded(group)?;
    let index = indices.get(group.0 as usize).copied().flatten()?;
    Some(if skins.get(index as usize)?.ready {
        index as i32 + 1
    } else {
        0
    })
}
fn loaded_world(names: &NameTable, files: &[ModelFile], name: &[u8]) -> Option<LoadedWorldBinding> {
    let ModelFile::LoadedWorld {
        binding, collision, ..
    } = file_lookup(names, files, name)?
    else {
        return None;
    };
    Some(LoadedWorldBinding {
        render: binding.world,
        collision: *collision,
    })
}
fn shader_definition(
    catalog: &Result<ShaderCatalog, String>,
    raw: &[u8],
    mipmaps: bool,
    model: bool,
) -> Result<ShaderDef, String> {
    let image = std::str::from_utf8(raw).map_err(|_| "shader name is not UTF-8")?;
    let display = std::str::from_utf8(shader_key(raw)).map_err(|_| "shader name is not UTF-8")?;
    let mut definition = catalog
        .as_ref()
        .map_err(Clone::clone)?
        .find_canonical(&canonical_path(display))
        .cloned()
        .unwrap_or_else(|| implicit_shader(display, image, mipmaps, model));
    // Negative native lightmap indices resolve explicit $lightmap to white.
    for stage in &mut definition.stages {
        if matches!(stage.map, Some(TextureMap::Lightmap)) {
            stage.map = Some(TextureMap::White);
        }
    }
    Ok(definition)
}
fn compile_shader(definition: &ShaderDef, images: &mut Images<'_>) -> Result<MaterialId, String> {
    if !definition.valid {
        return Err("invalid shader definition".to_owned());
    }
    qa_render::material::world_load::compile_definition(definition, images)
        .map_err(|error| format!("shader material: {error:?}"))
}
fn implicit_shader(display: &str, image: &str, mipmaps: bool, model: bool) -> ShaderDef {
    ShaderDef {
        name: display.to_owned(),
        source: String::new(),
        line: 0,
        valid: true,
        cull: Cull::Front,
        sort: if model { 3.0 } else { 9.0 },
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
            blend: (!model).then_some(StageBlend::ALPHA),
            rgb_gen: if model {
                RgbGen::LightingDiffuse
            } else {
                RgbGen::Vertex
            },
            alpha_gen: if model {
                AlphaGen::Identity
            } else {
                AlphaGen::Vertex
            },
            tc_gen: TexCoordGen::Texture,
            tc_mods: Box::new([]),
            alpha_func: AlphaFunc::None,
            depth_func: if model {
                DepthFunc::Lequal
            } else {
                DepthFunc::Always
            },
            depth_write: model,
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
fn model_handle(
    names: &NameTable,
    files: &[ModelFile],
    path: &[u8],
    source: RuleSetId,
) -> Option<i32> {
    let name = names.find_path(std::str::from_utf8(path).ok()?)?;
    let handle = i32::try_from(name.0).ok()?;
    renderer_file(files, handle, Some(source))?;
    Some(handle)
}
fn renderer_file(
    files: &[ModelFile],
    handle: i32,
    source: Option<RuleSetId>,
) -> Option<&ModelFile> {
    let index = usize::try_from(handle).ok().filter(|&index| index > 0)?;
    let file = files.get(index)?;
    match file {
        ModelFile::LoadedWorld { .. } => Some(file),
        ModelFile::Ready {
            renderer_sources, ..
        } if renderer_sources & source.map_or(u8::MAX, |source| 1 << source as u8) != 0 => {
            Some(file)
        }
        _ => None,
    }
}
fn model_bounds(files: &[ModelFile], handle: i32) -> Option<Bounds> {
    match renderer_file(files, handle, None)? {
        ModelFile::Ready { metadata, .. } => Some(metadata.renderer_bounds),
        file => file.bounds(),
    }
}
fn model_has_tags(files: &[ModelFile], handle: i32) -> bool {
    matches!(renderer_file(files, handle, None), Some(ModelFile::Ready {
        tags: Some(TagLayout { tags_per_frame, .. }), ..
    }) if *tags_per_frame != 0)
}
fn lerp_tag(
    names: &NameTable,
    files: &[ModelFile],
    tags: &[TaggedOrientation],
    handle: i32,
    start: i32,
    end: i32,
    fraction: f32,
    raw_name: &[u8],
) -> Option<Orientation> {
    let ModelFile::Ready {
        tags: Some(layout), ..
    } = renderer_file(files, handle, None)?
    else {
        return None;
    };
    let name = names.find(raw_name)?;
    let sample = |frame: i32| {
        let frame = u32::try_from(frame).ok()?.min(layout.frames - 1);
        let first = layout.samples.first + frame * layout.tags_per_frame;
        let samples = &tags[IndexSpan {
            first,
            count: layout.tags_per_frame,
        }
        .range()];
        let index = samples.partition_point(|tag| tag.name.0 < name.0);
        let tag = samples.get(index)?;
        (tag.name == name).then_some(tag.orientation)
    };
    Some(sample(start)?.interpolate(sample(end)?, fraction))
}
impl ModelLookup for ModelCatalogBuilder {
    fn inline_handle(&self, world: WorldId, index: u32) -> Option<i32> {
        inline_handle(&self.worlds, &self.inline, world, index)
    }
    fn model_has_tags(&self, handle: i32) -> bool {
        model_has_tags(&self.files, handle)
    }
    fn loaded_world(&self, name: &[u8]) -> Option<LoadedWorldBinding> {
        loaded_world(&self.names, &self.files, name)
    }
    fn lerp_tag(
        &self,
        handle: i32,
        start: i32,
        end: i32,
        fraction: f32,
        name: &[u8],
    ) -> Option<Orientation> {
        lerp_tag(
            &self.names,
            &self.files,
            &self.tags,
            handle,
            start,
            end,
            fraction,
            name,
        )
    }
    fn bounds(&self, path: &[u8], _source: RuleSetId) -> Option<Bounds> {
        file_lookup(&self.names, &self.files, path)?.bounds()
    }
    fn skin_handle(&self, name: &[u8]) -> Option<i32> {
        skin_handle(&self.names, &self.skin_indices, &self.skins, name)
    }
    fn shader_handle(&self, name: &[u8]) -> Option<i32> {
        shader_handle(&self.names, &self.shader_indices, &self.shaders, name)
    }
    fn model_handle(&self, path: &[u8], source: RuleSetId) -> Option<i32> {
        model_handle(&self.names, &self.files, path, source)
    }
    fn model_bounds(&self, handle: i32) -> Option<Bounds> {
        model_bounds(&self.files, handle)
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
    fn register_inline(&mut self, world: WorldId, index: u32) -> Result<i32, CallError> {
        self.admit_inline(world, index)
    }
    fn register_skin(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
    ) -> Result<i32, CallError> {
        self.admit_skin(vfs, name, source)
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
    fn inline_handle(&self, world: WorldId, index: u32) -> Option<i32> {
        inline_handle(&self.worlds, &self.inline, world, index)
    }
    fn model_has_tags(&self, handle: i32) -> bool {
        model_has_tags(&self.files, handle)
    }
    fn loaded_world(&self, name: &[u8]) -> Option<LoadedWorldBinding> {
        loaded_world(&self.names, &self.files, name)
    }
    fn lerp_tag(
        &self,
        handle: i32,
        start: i32,
        end: i32,
        fraction: f32,
        name: &[u8],
    ) -> Option<Orientation> {
        lerp_tag(
            &self.names,
            &self.files,
            &self.tags,
            handle,
            start,
            end,
            fraction,
            name,
        )
    }
    fn bounds(&self, path: &[u8], _source: RuleSetId) -> Option<Bounds> {
        file_lookup(&self.names, &self.files, path)?.bounds()
    }
    fn skin_handle(&self, name: &[u8]) -> Option<i32> {
        skin_handle(&self.names, &self.skin_indices, &self.skins, name)
    }
    fn shader_handle(&self, name: &[u8]) -> Option<i32> {
        shader_handle(&self.names, &self.shader_indices, &self.shaders, name)
    }
    fn model_handle(&self, path: &[u8], source: RuleSetId) -> Option<i32> {
        model_handle(&self.names, &self.files, path, source)
    }
    fn model_bounds(&self, handle: i32) -> Option<Bounds> {
        model_bounds(&self.files, handle)
    }
}
impl ModelCatalog {
    pub fn color_binding(&self, profile: ModelProfile) -> Option<ModelColorId> {
        self.colors
            .iter()
            .position(|(key, _)| *key == profile)
            .map(|index| ModelColorId(index as u32))
    }
    pub fn colors(&self, binding: ModelColorId) -> Option<&PaletteColors> {
        self.colors
            .get(binding.0 as usize)
            .map(|(_, colors)| colors)
    }
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
            ModelFile::Ready { metadata, .. } => Some(*metadata),
            ModelFile::Empty | ModelFile::LoadedWorld { .. } | ModelFile::Failed(_) => None,
        }
    }
    /// Resolve both filename and profile only at registration/configuration
    /// boundaries. A profile mismatch never selects the first same-name entry.
    pub fn registration(&self, path: &str, profile: ModelProfile) -> Option<ModelRegistrationId> {
        let name = self.names.find_path(path)?;
        self.registration_name(name, profile)
    }
    /// Native handles already identify the interned file. Resolve their exact
    /// presentation registration without repeating filename conversion/lookup.
    pub fn renderer_registration(
        &self,
        handle: i32,
        profile: ModelProfile,
    ) -> Option<ModelRegistrationId> {
        renderer_file(&self.files, handle, Some(profile.source_rules))?;
        self.registration_name(NameId(u32::try_from(handle).ok()?), profile)
    }
    fn registration_name(
        &self,
        name: NameId,
        profile: ModelProfile,
    ) -> Option<ModelRegistrationId> {
        let span = *self.models.get(name.0 as usize)?;
        span.range()
            .find(|&index| {
                let entry = &self.registrations[index];
                entry.profile == profile && entry.surfaces.is_some()
            })
            .map(|index| ModelRegistrationId(index as u32))
    }
    pub fn name(&self, name: NameId) -> Option<&str> {
        std::str::from_utf8(self.names.get(name)?).ok()
    }
    /// Frame lookup uses only cached numeric registration, skin and policy.
    pub fn bindings(
        &self,
        registration: ModelRegistrationId,
        skin: u32,
        policy: ModelMaterialPolicy,
    ) -> Option<ModelBindings<'_>> {
        let span = self.registrations.get(registration.0 as usize)?.surfaces?;
        Some(ModelBindings {
            surfaces: &self.surfaces[span.range()],
            materials: &self.bindings,
            skin,
            policy,
            custom: policy
                .custom_skin
                .checked_sub(1)
                .and_then(|value| usize::try_from(value).ok())
                .and_then(|index| self.skins.get(index))
                .map(|skin| {
                    skin.profiles
                        .iter()
                        .find(|skin| {
                            skin.profile == self.registrations[registration.0 as usize].profile
                        })
                        .map_or(&[][..], |profile| profile.mappings.as_ref())
                }),
        })
    }
    /// Cold CLIENT registration reads the same already-loaded file record.
    pub fn loaded_collision_model(
        &self,
        path: &[u8],
    ) -> Option<(InlineBinding, Option<qa_core::primitives::GeometryId>)> {
        match file_lookup(&self.names, &self.files, path)? {
            ModelFile::LoadedWorld {
                binding, collision, ..
            } => Some((*binding, *collision)),
            _ => None,
        }
    }
    pub fn collision_world(&self, world: WorldId) -> Option<qa_core::primitives::GeometryId> {
        self.files.iter().find_map(|file| match file {
            ModelFile::LoadedWorld {
                binding, collision, ..
            } if binding.world == world && binding.index == 0 => *collision,
            _ => None,
        })
    }
    pub fn renderer_inline(&self, handle: i32) -> Option<InlineBinding> {
        let ModelFile::LoadedWorld { binding, .. } = renderer_file(&self.files, handle, None)?
        else {
            return None;
        };
        Some(*binding)
    }
    pub fn resolve_inline(&self, world: WorldId, index: u32) -> Option<InlineBinding> {
        inline_binding(&self.worlds, &self.inline, world, index)
    }
    pub fn capacity(&self) -> ModelCapacity {
        self.capacity
    }
}

fn inline_slot(worlds: &[IndexSpan], world: WorldId, index: u32) -> Option<usize> {
    worlds.get(world.0 as usize)?.get(index as usize)
}
fn inline_handle(
    worlds: &[IndexSpan],
    inline: &[RegisteredInline],
    world: WorldId,
    index: u32,
) -> Option<i32> {
    i32::try_from(inline.get(inline_slot(worlds, world, index)?)?.handle?.0).ok()
}
fn inline_binding(
    worlds: &[IndexSpan],
    inline: &[RegisteredInline],
    world: WorldId,
    index: u32,
) -> Option<InlineBinding> {
    Some(inline.get(inline_slot(worlds, world, index)?)?.binding)
}

fn inline_bindings(
    assets: &Assets,
    worlds: impl IntoIterator<Item = WorldId>,
    limits: ModelLimits,
) -> Result<(Box<[IndexSpan]>, Box<[RegisteredInline]>), String> {
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
            inline.push(RegisteredInline {
                binding: InlineBinding {
                    world: id,
                    index: index as u32,
                    surfaces: model.surfaces,
                },
                handle: None,
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
    if !matches!(
        model.format,
        ModelFormat::Mdl | ModelFormat::Md2 | ModelFormat::Md3 | ModelFormat::Sp2
    ) {
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
    if model.format != ModelFormat::Md3
        && model.format != ModelFormat::Sp2
        && model.skins.is_empty()
    {
        return Err("model requires an explicit external skin".to_owned());
    }
    Ok(())
}

fn register(
    model: &Model<'_>,
    images: &mut Images<'_>,
    registration: &Registration,
    model_policy: ModelPolicy,
    shared_meshes: &mut [Option<ModelId>],
    shader_catalog: Option<&Result<ShaderCatalog, String>>,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<Vec<Vec<ModelMaterials>>, String> {
    if model.format == ModelFormat::Sp2 {
        return register_sprites(model, images, registration, model_policy, capacity, limits);
    }
    let next_surfaces = capacity
        .surfaces
        .checked_add(model.meshes.len())
        .filter(|&count| count <= limits.surfaces && count <= u32::MAX as usize)
        .ok_or("model surface capacity")?;
    let alias = if model.format == ModelFormat::Md3 {
        None
    } else {
        Some(alias_materials(
            &model.skins,
            images,
            registration,
            model_policy,
        )?)
    };
    let mut surfaces = Vec::with_capacity(model.meshes.len());
    let mut skin_count = capacity.skins;
    for (mesh, shared_mesh) in model.meshes.iter().zip(shared_meshes) {
        let count = alias
            .as_ref()
            .map_or_else(|| mesh.shaders.len().max(1), Vec::len);
        skin_count = skin_count
            .checked_add(count)
            .filter(|&count| count <= limits.skins && count <= u32::MAX as usize)
            .ok_or("model skin capacity")?;
        let materials = if let Some(materials) = &alias {
            materials.clone()
        } else if mesh.shaders.is_empty() {
            return Err("MD3 surface default shader is not yet registered".to_owned());
        } else {
            mesh.shaders
                .iter()
                .map(|shader| {
                    let definition = shader_definition(
                        shader_catalog.ok_or("missing model shader catalog")?,
                        shader.name,
                        true,
                        true,
                    )?;
                    let material = compile_shader(&definition, images)?;
                    Ok([material; 4])
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        let id = if let Some(id) = *shared_mesh {
            id
        } else {
            let native = (model.format == ModelFormat::Md2).then_some(model.frames.as_slice());
            let (vertices, frame_size, indices, packed) =
                mesh_vertices(mesh, native, capacity, limits)?;
            let material = materials.first().ok_or("model has no skin material")?[0];
            let id = images
                .assets
                .register_animated_model(&vertices, frame_size, &indices, material)?;
            if let Some(frames) = native {
                let frames: Vec<_> = frames
                    .iter()
                    .map(|frame| qa_render::assets::AliasFrame {
                        scale: frame.scale,
                        translation: frame.translation,
                    })
                    .collect();
                images.assets.set_alias_frames(id, &frames, &packed)?;
            }
            *shared_mesh = Some(id);
            id
        };
        surfaces.push(
            materials
                .into_iter()
                .map(|materials| ModelMaterials {
                    variants: materials.map(|material| ModelBinding {
                        model: id,
                        material,
                        pose: model_policy.presentation.pose,
                    }),
                })
                .collect(),
        );
    }
    capacity.skins = skin_count;
    capacity.surfaces = next_surfaces;
    Ok(surfaces)
}

fn register_sprites(
    model: &Model<'_>,
    images: &mut Images<'_>,
    registration: &Registration,
    policy: ModelPolicy,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<Vec<Vec<ModelMaterials>>, String> {
    let next_surfaces = capacity
        .surfaces
        .checked_add(1)
        .filter(|&n| n <= limits.surfaces && n <= u32::MAX as usize)
        .ok_or("model surface capacity")?;
    let next_skins = capacity
        .skins
        .checked_add(model.sprites.len())
        .filter(|&n| n <= limits.skins && n <= u32::MAX as usize)
        .ok_or("model skin capacity")?;
    let mut frames = Vec::with_capacity(model.sprites.len());
    for (index, sprite) in model.sprites.iter().enumerate() {
        if !matches!(sprite.image, Image::External(_)) {
            return Err("SP2 frame requires an external image".to_owned());
        }
        let image = model_image(sprite.image, images, registration.profile, true, Some(255))?;
        let mut variants = [MaterialId(0); 4];
        for material_policy in ModelMaterialPolicy::ALL {
            variants[material_policy.index()] = images.assets.register_material(
                &format!(
                    "{}#sprite{index}:{}",
                    registration.path,
                    material_policy.index()
                ),
                &[Stage {
                    texture: StageTexture::Image(image),
                    rgb_gen: RgbGen::ExactVertex,
                    alpha_gen: AlphaGen::Entity,
                    alpha_test: if material_policy.translucent {
                        AlphaFunc::None
                    } else {
                        AlphaFunc::AtLeastHalf
                    },
                    blend: material_policy.translucent.then_some(StageBlend::ALPHA),
                    depth_write: !material_policy.translucent,
                    ..Stage::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
                    sort: if material_policy.translucent {
                        6.0
                    } else {
                        3.0
                    },
                    ..MaterialSettings::default()
                },
            )?;
        }
        let geometry = qa_render::assets::SpriteGeometry {
            extents: [
                -(sprite.origin[0] as f32),
                (i64::from(sprite.width) - i64::from(sprite.origin[0])) as f32,
                (i64::from(sprite.height) - i64::from(sprite.origin[1])) as f32,
                -(sprite.origin[1] as f32),
            ],
        };
        let model = images.assets.register_sprite(geometry, variants[0])?;
        frames.push(ModelMaterials {
            variants: variants.map(|material| ModelBinding {
                model,
                material,
                pose: policy.presentation.pose,
            }),
        });
    }
    capacity.surfaces = next_surfaces;
    capacity.skins = next_skins;
    Ok(vec![frames])
}

fn model_image(
    image: Image<'_>,
    images: &mut Images<'_>,
    options: ModelProfile,
    indexed_external_pcx: bool,
    transparent: Option<u8>,
) -> Result<ImageId, String> {
    let usage = ImageUse::default();
    Ok(match image {
        Image::Indexed {
            width,
            height,
            pixels,
        } => {
            let palette = options
                .source_palette
                .ok_or("indexed model requires a source palette")?;
            let texture = IndexedTexture::load_base(width, height, pixels, transparent)?;
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
            if indexed_external_pcx
                && qa_formats::image::ImageFormat::from_path(path.as_bytes())
                    == Some(qa_formats::image::ImageFormat::Pcx)
            {
                images
                    .indexed_pcx(
                        path,
                        options
                            .source_palette
                            .ok_or("PCX skin requires a source palette")?,
                        transparent,
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
    })
}

fn alias_materials(
    skins: &[Image<'_>],
    images: &mut Images<'_>,
    registration: &Registration,
    model_policy: ModelPolicy,
) -> Result<Vec<[MaterialId; 4]>, String> {
    let options = registration.profile;
    let mut materials = Vec::with_capacity(skins.len());
    for (index, skin) in skins.iter().enumerate() {
        let image = model_image(
            *skin,
            images,
            options,
            model_policy.source.indexed_external_pcx,
            None,
        )?;
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
    Ok(materials)
}

fn mesh_vertices(
    mesh: &Mesh<'_>,
    native: Option<&[qa_formats::model::Frame<'_>]>,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<(Vec<Vertex>, usize, Vec<u32>, Vec<[u8; 3]>), String> {
    if mesh.vertices_per_frame == 0 {
        return Ok((Vec::new(), 0, Vec::new(), Vec::new()));
    }
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
    let mut packed = Vec::with_capacity(if native.is_some() { vertex_count } else { 0 });
    for (frame_index, frame) in mesh
        .vertices
        .chunks_exact(mesh.vertices_per_frame)
        .enumerate()
    {
        for &(vertex, texcoord) in &pairs {
            if let Some(frames) = native {
                let bytes = &frames[frame_index].packed_vertices[vertex as usize * 4..];
                packed.push([bytes[0], bytes[1], bytes[2]]);
            }
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
    Ok((vertices, pairs.len(), indices, packed))
}
