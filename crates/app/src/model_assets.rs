//! Cold model registration. Native config ordinals resolve through filenames;
//! inline ordinals retain their explicit world and existing surface ranges.
use qa_compat::services::{
    CallError, LoadedWorldBinding, ModelLoader, ModelLookup, RendererRegistration, RendererRequest,
};
use qa_content::vfs::Vfs;
use qa_core::names::{NameTable, canonical_path};
use qa_core::primitives::{Bounds, IndexSpan, NameId, Orientation, RuleSetId};
use qa_formats::archive::ArchiveReader;
use qa_formats::model::{Image, Mesh, Model, ModelFormat};
use qa_gameplay::rules::appearance::{self, AliasPose, ModelPresentation, ModelSource};
use qa_render::assets::{
    AssetCounts, ImageId, MaterialSettings, SoftwareAlias, Stage, StageTexture,
};
use qa_render::material::resources::{
    ImageRegistrationMark, ImageSettings, ImageUse, Images, ImagesState, IndexedTarget,
    ResourceError,
};
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

/// Cold world policies are explicit and independent of BSP decoding.
pub struct WorldRegistration {
    pub profile: ModelProfile,
    pub source_palette: Option<PaletteId>,
    pub presentation: qa_render::material::world_load::WorldPresentationProfile,
    pub options: qa_render::material::world_load::WorldLoadOptions,
    pub trace_rules: qa_world::collision::TraceRules,
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
                .map_or(MaterialId(0), |index| {
                    custom[index].materials[self.policy.index()]
                });
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
    state: RegistrationState<IndexSpan>,
}
struct RegisteredShader {
    name: NameId,
    profiles: IndexSpan,
}
struct ShaderProfileBinding {
    profile: ModelProfile,
    state: RegistrationState<MaterialId>,
}
enum RegistrationState<T> {
    Unattempted,
    Failed(String),
    Ready {
        value: T,
        required_assets: Option<AssetCounts>,
    },
}
impl<T> RegistrationState<T> {
    fn ready(&self) -> Option<(&T, Option<AssetCounts>)> {
        match self {
            Self::Ready {
                value,
                required_assets,
            } => Some((value, *required_assets)),
            Self::Unattempted | Self::Failed(_) => None,
        }
    }
    fn cached(&self, handle: i32, admitted: Option<AssetCounts>) -> Option<i32> {
        match self {
            Self::Unattempted => None,
            Self::Failed(_) => Some(0),
            Self::Ready {
                required_assets, ..
            } => required_assets
                .is_none_or(|required| admitted.is_some_and(|counts| counts.contains(required)))
                .then_some(handle),
        }
    }
    fn rollback(&mut self, mode: RegistrationRollback) {
        if matches!(mode, RegistrationRollback::Retry) || !matches!(self, Self::Failed(_)) {
            *self = Self::Unattempted;
        }
    }
    fn required_assets_mut(&mut self) -> Option<&mut Option<AssetCounts>> {
        match self {
            Self::Ready {
                required_assets, ..
            } => Some(required_assets),
            Self::Unattempted | Self::Failed(_) => None,
        }
    }
}
#[derive(Clone, Copy)]
enum RegistrationRollback {
    Retry,
    Source,
}
struct RegisteredSkin {
    profiles: Vec<SkinProfile>,
}
fn source_handle<'a, T: 'a>(
    states: impl Iterator<Item = &'a RegistrationState<T>>,
    handle: i32,
) -> Option<i32> {
    let mut ready = false;
    let mut configured = false;
    for state in states {
        configured = true;
        match state {
            RegistrationState::Unattempted => return None,
            RegistrationState::Ready { .. } => ready = true,
            RegistrationState::Failed(_) => {}
        }
    }
    configured.then_some(if ready { handle } else { 0 })
}
impl RegisteredSkin {
    fn source_handle(&self, handle: i32, source: RuleSetId) -> Option<i32> {
        source_handle(
            self.profiles
                .iter()
                .filter(|entry| entry.profile.source_rules == source)
                .map(|entry| &entry.state),
            handle,
        )
    }
}

struct SkinProfile {
    profile: ModelProfile,
    state: RegistrationState<Box<[SkinMapping]>>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct SkinMapping {
    surface: NameId,
    materials: [MaterialId; 4],
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
#[derive(Clone)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    fn with_images<T>(
        &mut self,
        vfs: &Vfs,
        assets: &mut Assets,
        compile: impl FnOnce(&mut Images<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let Some(state) = self.state.take() else {
            return Err("shader image state already borrowed".to_owned());
        };
        match state {
            Ok(state) => {
                let mut images = Images::from_state(vfs, assets, state);
                let result = compile(&mut images);
                self.state = Some(Ok(images.into_state()));
                result
            }
            Err(error) => {
                self.state = Some(Err(error.clone()));
                Err(error)
            }
        }
    }
    fn compile(
        &mut self,
        vfs: &Vfs,
        assets: &mut Assets,
        definition: Result<&ShaderDef, String>,
    ) -> Result<MaterialId, String> {
        self.with_images(vfs, assets, |images| {
            definition.and_then(|definition| compile_shader(definition, images))
        })
    }
}

/// The session's one model store and retained registration state.
pub struct ModelCatalog {
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
    worlds: Vec<IndexSpan>,
    inline: Vec<RegisteredInline>,
    capacity: ModelCapacity,
    limits: ModelLimits,
    pub diagnostics: Vec<ModelDiagnostic>,
    admitted_assets: Option<AssetCounts>,
}

/// Owns render resources and their numeric model catalog through registration
/// and presentation. Parsed model bytes remain local to one registration.
pub struct ModelAssets {
    assets: Assets,
    catalog: ModelCatalog,
    initial_registration: bool,
}

#[derive(Clone, Copy)]
pub struct ModelView<'a> {
    pub assets: &'a Assets,
    pub catalog: &'a ModelCatalog,
}

struct RegistrationCheckpoint {
    assets: AssetCounts,
    names: usize,
    capacity: ModelCapacity,
    registrations: usize,
    bindings: usize,
    surfaces: usize,
    tags: usize,
    shaders: usize,
    shader_bindings: usize,
    skins: usize,
    diagnostics: usize,
    worlds: usize,
    inline_count: usize,
    file: Option<ModelCheckpoint>,
    shader_index: Option<(NameId, Option<u32>)>,
    shader: Option<ShaderCheckpoint>,
    skin: Option<SkinCheckpoint>,
    skin_index: Option<(NameId, Option<u32>)>,
    inline: Option<(usize, Option<NameId>)>,
    images: Vec<Option<ImageRegistrationMark>>,
}
struct ShaderCheckpoint {
    bindings: Vec<usize>,
}
struct ModelCheckpoint {
    name: NameId,
    file: ModelFile,
    profiles: IndexSpan,
    unattempted: Vec<usize>,
}
struct SkinCheckpoint {
    index: usize,
    unattempted: Vec<usize>,
}
impl RegistrationCheckpoint {
    fn capture(
        owner: &ModelAssets,
        request: &RendererRequest<impl AsRef<[u8]>>,
    ) -> Result<Self, String> {
        let catalog = &owner.catalog;
        let mut file_name = None;
        let mut shader_index = None;
        let mut skin_index = None;
        let mut inline = None;
        let raw = request.name.as_ref();
        match request.kind {
            RendererRegistration::Model if raw.starts_with(b"*") => {
                let world = request.world.ok_or("inline registration world")?;
                let index =
                    qa_core::text::inline_model_index(raw).ok_or("inline registration index")?;
                let slot = inline_slot(&catalog.worlds, world, index)
                    .ok_or("inline registration model")?;
                inline = Some((slot, catalog.inline[slot].handle));
                let key = format!("\0engine:inline/{}/{}", world.0, index);
                file_name = catalog.names.find(key.as_bytes());
            }
            RendererRegistration::Model => {
                let path = canonical_path(
                    std::str::from_utf8(raw).map_err(|_| "model name is not UTF-8")?,
                );
                file_name = catalog.names.find(path.as_bytes());
            }
            RendererRegistration::Skin => {
                let path =
                    canonical_path(std::str::from_utf8(raw).map_err(|_| "skin name is not UTF-8")?);
                skin_index = catalog
                    .names
                    .find_folded(path.as_bytes())
                    .map(|group| (group, catalog.skin_indices[group.0 as usize]));
            }
            RendererRegistration::Shader { .. } => {
                shader_index = find_shader_index(&catalog.names, &catalog.shader_indices, raw);
            }
        }
        let shader = shader_index.and_then(|(_, index)| {
            let index = index? as usize;
            let shader = &catalog.shaders[index];
            Some(ShaderCheckpoint {
                bindings: shader
                    .profiles
                    .range()
                    .filter(|&index| {
                        let binding = &catalog.shader_bindings[index];
                        binding.profile.source_rules == request.source
                            && matches!(binding.state, RegistrationState::Unattempted)
                    })
                    .collect(),
            })
        });
        let skin = skin_index.and_then(|(_, index)| {
            let index = index? as usize;
            Some(SkinCheckpoint {
                index,
                unattempted: catalog.skins[index]
                    .profiles
                    .iter()
                    .enumerate()
                    .filter_map(|(index, profile)| {
                        matches!(profile.state, RegistrationState::Unattempted).then_some(index)
                    })
                    .collect(),
            })
        });
        let images = catalog
            .profiles
            .iter()
            .map(|profile| match &profile.state {
                Some(Ok(state)) => Ok(Some(state.registration_mark())),
                Some(Err(_)) => Ok(None),
                None => Err("model image state already borrowed".to_owned()),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            assets: owner.assets.counts(),
            names: catalog.names.len(),
            capacity: catalog.capacity,
            registrations: catalog.registrations.len(),
            bindings: catalog.bindings.len(),
            surfaces: catalog.surfaces.len(),
            tags: catalog.tags.len(),
            shaders: catalog.shaders.len(),
            shader_bindings: catalog.shader_bindings.len(),
            skins: catalog.skins.len(),
            diagnostics: catalog.diagnostics.len(),
            worlds: catalog.worlds.len(),
            inline_count: catalog.inline.len(),
            file: file_name.map(|name| {
                let profiles = catalog.models[name.0 as usize];
                ModelCheckpoint {
                    name,
                    file: catalog.files[name.0 as usize].clone(),
                    profiles,
                    unattempted: profiles
                        .range()
                        .filter(|&index| {
                            matches!(
                                catalog.registrations[index].state,
                                RegistrationState::Unattempted
                            )
                        })
                        .collect(),
                }
            }),
            shader_index,
            shader,
            skin,
            skin_index,
            inline,
            images,
        })
    }

    fn rollback(self, owner: &mut ModelAssets, mode: RegistrationRollback) -> Result<(), String> {
        owner.assets.discard_registration_suffix(self.assets)?;
        let catalog = &mut owner.catalog;
        for (profile, mark) in catalog.profiles.iter_mut().zip(self.images) {
            if let (Some(Ok(state)), Some(mark)) = (&mut profile.state, mark) {
                state.rollback_registration(mark, self.assets.images);
            }
        }
        catalog.bindings.truncate(self.bindings);
        catalog.surfaces.truncate(self.surfaces);
        catalog.tags.truncate(self.tags);
        catalog.worlds.truncate(self.worlds);
        catalog.inline.truncate(self.inline_count);
        if matches!(mode, RegistrationRollback::Retry) {
            catalog.registrations.truncate(self.registrations);
            catalog.shaders.truncate(self.shaders);
            catalog.shader_bindings.truncate(self.shader_bindings);
            catalog.skins.truncate(self.skins);
            catalog.diagnostics.truncate(self.diagnostics);
            // Interned names retain their identities without rejected bindings.
            for index in self.names..catalog.names.len() {
                catalog.files[index] = ModelFile::Empty;
                catalog.models[index] = IndexSpan::default();
                catalog.shader_indices[index] = None;
                catalog.skin_indices[index] = None;
            }
            if let Some((group, index)) = self.shader_index {
                catalog.shader_indices[group.0 as usize] = index;
            }
            if let Some((group, index)) = self.skin_index {
                catalog.skin_indices[group.0 as usize] = index;
            }
        } else {
            // Keep negative metadata, but no value into a discarded resource suffix.
            for entry in &mut catalog.registrations[self.registrations..] {
                entry.state.rollback(mode);
            }
            for entry in &mut catalog.shader_bindings[self.shader_bindings..] {
                entry.state.rollback(mode);
            }
            for skin in &mut catalog.skins[self.skins..] {
                for profile in &mut skin.profiles {
                    profile.state.rollback(mode);
                }
            }
        }
        if let Some(file) = self.file.as_ref() {
            for &index in &file.unattempted {
                catalog.registrations[index].state.rollback(mode);
            }
        }
        if let Some(shader) = self.shader {
            for index in shader.bindings {
                catalog.shader_bindings[index].state.rollback(mode);
            }
        }
        if let Some(skin) = self.skin {
            for index in skin.unattempted {
                catalog.skins[skin.index].profiles[index]
                    .state
                    .rollback(mode);
            }
        }
        if matches!(mode, RegistrationRollback::Source) {
            if let Some(file) = self.file.as_ref()
                && matches!(catalog.files[file.name.0 as usize], ModelFile::Failed(_))
                && matches!(file.file, ModelFile::Ready { renderer_sources, .. } if renderer_sources != 0)
            {
                catalog.files[file.name.0 as usize] = file.file.clone();
            }
            for index in self
                .file
                .as_ref()
                .map(|file| file.name.0 as usize)
                .into_iter()
                .chain(self.names..catalog.names.len())
            {
                let sources = catalog.models[index].range().fold(0, |sources, entry| {
                    let profile = &catalog.registrations[entry];
                    sources
                        | if profile.state.ready().is_some() {
                            1 << profile.profile.source_rules as u8
                        } else {
                            0
                        }
                });
                if let ModelFile::Ready {
                    renderer_sources,
                    tags,
                    ..
                } = &mut catalog.files[index]
                {
                    *renderer_sources = sources;
                    if tags.is_some_and(|layout| layout.samples.range().end > self.tags) {
                        *tags = None;
                    }
                }
            }
        } else if let Some(file) = self.file {
            catalog.files[file.name.0 as usize] = file.file;
            catalog.models[file.name.0 as usize] = file.profiles;
        }
        if let Some((slot, handle)) = self.inline {
            catalog.inline[slot].handle = handle;
        }
        catalog.capacity = ModelCapacity {
            names: catalog.capacity.names,
            name_bytes: catalog.capacity.name_bytes,
            registrations: catalog.registrations.len(),
            ..self.capacity
        };
        Ok(())
    }
}

impl ModelAssets {
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
            initial_registration: true,
            catalog: ModelCatalog {
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
                admitted_assets: None,
            },
        })
    }

    /// All requests delegate to the same staged native precache implementation.
    pub fn load<'a>(
        vfs: &Vfs,
        assets: Assets,
        requests: impl IntoIterator<Item = ModelRequest<'a>>,
        worlds: impl IntoIterator<Item = WorldId>,
        limits: ModelLimits,
    ) -> Result<Self, String> {
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
        let mut owner = Self::new(
            assets,
            pending.iter().map(|entry| entry.profile),
            worlds,
            limits,
        )?;
        for entry in &pending {
            owner.register_request(
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
        Ok(owner)
    }
    pub fn view(&self) -> ModelView<'_> {
        ModelView {
            assets: &self.assets,
            catalog: &self.catalog,
        }
    }
    pub fn assets(&self) -> &Assets {
        &self.assets
    }
    pub fn assets_mut(&mut self) -> &mut Assets {
        &mut self.assets
    }
    pub fn catalog(&self) -> &ModelCatalog {
        &self.catalog
    }
    pub fn initial_registration(&self) -> bool {
        self.initial_registration
    }
    pub fn end_initial_registration(&mut self) {
        self.initial_registration = false;
    }

    /// Register one late native request and publish it only after the backend
    /// accepts the complete appended resource prefix.
    pub fn prepare_registration(
        &mut self,
        vfs: &Vfs,
        request: &RendererRequest<impl AsRef<[u8]>>,
        profile: ModelProfile,
        prepare: impl FnOnce(&Assets) -> Result<(), String>,
    ) -> Result<(i32, AssetCounts), String> {
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("late registration requires a published resource prefix".to_owned());
        }
        if request.name.as_ref().is_empty() {
            return Err("renderer registration name length".to_owned());
        }
        if profile.source_rules != request.source
            || !self
                .catalog
                .profiles
                .iter()
                .any(|entry| entry.profile == profile)
        {
            return Err("renderer registration source/profile mismatch".to_owned());
        }
        if let Some(handle) = self.catalog.registration_cached(
            request.kind,
            request.name.as_ref(),
            profile,
            request.world,
        ) {
            return Ok((handle, self.assets.counts()));
        }
        let checkpoint = RegistrationCheckpoint::capture(self, request)?;
        let mut rollback = RegistrationRollback::Source;
        let result = (|| {
            let raw = request.name.as_ref();
            let handle = match request.kind {
                RendererRegistration::Model if raw.starts_with(b"*") => request
                    .world
                    .zip(qa_core::text::inline_model_index(raw))
                    .ok_or(CallError::File)
                    .and_then(|(world, index)| self.register_inline(world, index)),
                RendererRegistration::Model => self.register_model(vfs, raw, request.source),
                RendererRegistration::Skin => self.register_skin(vfs, raw, request.source),
                RendererRegistration::Shader { mipmaps } => {
                    self.register_shader(vfs, raw, request.source, mipmaps)
                }
            }
            .map_err(|error| format!("renderer registration: {error:?}"))?;
            if !self
                .catalog
                .renderer_candidate(request.kind, handle, profile)
            {
                return Err("renderer registration unavailable for invoking profile".to_owned());
            }
            let counts = self.assets.counts();
            rollback = RegistrationRollback::Retry;
            self.validate_admission(counts)
                .map_err(|error| format!("renderer publication: {error:?}"))?;
            prepare(&self.assets)?;
            // The backend has committed this prefix. All owner checks already
            // passed, so publication cannot fail or discard accepted resources.
            self.apply_admission(counts);
            Ok((handle, counts))
        })();
        match result {
            Ok(ready) => Ok(ready),
            Err(error) => {
                let error = self.catalog.diagnostics[checkpoint.diagnostics..]
                    .iter()
                    .find(|entry| entry.profile == profile)
                    .map_or(error.clone(), |entry| format!("{error}: {}", entry.error));
                checkpoint
                    .rollback(self, rollback)
                    .map_err(|rollback| format!("{error}; rollback: {rollback}"))?;
                Err(error)
            }
        }
    }

    /// Load one world through the shared BSP boundary and stage caller-owned
    /// cold state before the backend commits its appended asset prefix.
    pub fn prepare_world<T>(
        &mut self,
        vfs: &Vfs,
        geometry: &mut qa_world::collision::CollisionStore,
        input: crate::map::MapInput,
        registration: WorldRegistration,
        stage: impl FnOnce(
            ModelView<'_>,
            &qa_world::collision::CollisionStore,
            &crate::map::LoadedMap,
        ) -> Result<T, String>,
        prepare: impl FnOnce(&Assets, &T) -> Result<(), String>,
    ) -> Result<(crate::map::LoadedMap, T, AssetCounts), String> {
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("world registration requires a published resource prefix".to_owned());
        }
        let WorldRegistration {
            profile,
            source_palette,
            presentation,
            options,
            trace_rules,
        } = registration;
        if !self
            .catalog
            .profiles
            .iter()
            .any(|entry| entry.profile == profile)
            || presentation.rules != profile.rules
            || presentation.indexed_palette != profile.palette
            || presentation.image_settings != profile.image_settings
        {
            return Err("world registration source/profile mismatch".to_owned());
        }
        let checkpoint = RegistrationCheckpoint::capture(
            self,
            &RendererRequest {
                name: input.virtual_path.as_bytes(),
                source: profile.source_rules,
                kind: RendererRegistration::Model,
                world: None,
            },
        )?;
        let hulls = input.native_source.lineage() == qa_formats::entities::EntitySyntax::Quake;
        let mut created = None;
        let result = (|| {
            let loaded = input.load(
                vfs,
                &mut self.assets,
                geometry,
                source_palette,
                &[presentation],
                options,
            )?;
            created = Some(loaded.collision);
            if hulls && trace_rules.hull_shape == qa_world::collision::HullShape::ExactBox {
                geometry
                    .prepare_hull_positions(loaded.collision, Default::default())
                    .map_err(|error| format!("world trace capability: {error:?}"))?;
            }
            append_inline_bindings(
                &self.assets,
                &mut self.catalog.worlds,
                &mut self.catalog.inline,
                loaded.render.world,
                self.catalog.limits,
            )?;
            self.catalog.capacity.inline_bindings = self.catalog.inline.len();
            self.bind_loaded_world(
                &loaded.virtual_path,
                loaded.render.world,
                Some(loaded.collision),
                loaded.collision_bounds,
            )?;
            let staged = stage(self.view(), geometry, &loaded)?;
            let counts = self.assets.counts();
            self.validate_admission(counts)
                .map_err(|error| format!("world publication: {error:?}"))?;
            prepare(&self.assets, &staged)?;
            // Backend success is irreversible. Every fallible owner and caller
            // check has completed before this infallible publication.
            self.apply_admission(counts);
            Ok((loaded, staged, counts))
        })();
        match result {
            Ok(ready) => Ok(ready),
            Err(error) => {
                let rolled_back = checkpoint.rollback(self, RegistrationRollback::Retry);
                if let Some(created) = created {
                    geometry.remove(created);
                }
                rolled_back.map_err(|rollback| format!("{error}; rollback: {rollback}"))?;
                Err(error)
            }
        }
    }

    /// Publish only the prefix reported by a successfully prepared backend.
    /// Registration and source metadata alone do not establish readiness.
    pub fn publish_admission(&mut self, counts: AssetCounts) -> Result<(), CallError> {
        self.validate_admission(counts)?;
        self.apply_admission(counts);
        Ok(())
    }

    fn validate_admission(&self, counts: AssetCounts) -> Result<(), CallError> {
        if !self.assets.counts().contains(counts)
            || self
                .catalog
                .admitted_assets
                .is_some_and(|previous| !counts.contains(previous))
        {
            return Err(CallError::Geometry);
        }
        Ok(())
    }

    fn apply_admission(&mut self, counts: AssetCounts) {
        self.catalog.admitted_assets = Some(counts);
        for required in self
            .catalog
            .registrations
            .iter_mut()
            .filter_map(|entry| entry.state.required_assets_mut())
            .chain(
                self.catalog
                    .shader_bindings
                    .iter_mut()
                    .filter_map(|entry| entry.state.required_assets_mut()),
            )
            .chain(
                self.catalog
                    .skins
                    .iter_mut()
                    .flat_map(|skin| skin.profiles.iter_mut())
                    .filter_map(|entry| entry.state.required_assets_mut()),
            )
        {
            if required.is_some_and(|required| counts.contains(required)) {
                *required = None;
            }
        }
    }

    fn admit_name(&mut self, bytes: &[u8]) -> Result<NameId, CallError> {
        let previous = self.catalog.names.len();
        let name = self
            .catalog
            .names
            .intern(bytes)
            .map_err(|_| CallError::Capacity)?;
        if self.catalog.names.len() != previous {
            self.catalog.files.push(ModelFile::Empty);
            self.catalog.models.push(IndexSpan::default());
            self.catalog.shader_indices.push(None);
            self.catalog.skin_indices.push(None);
            self.catalog.capacity.names = self.catalog.names.len();
            self.catalog.capacity.name_bytes += bytes.len();
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
        let binding = inline_binding(&self.catalog.worlds, &self.catalog.inline, world, 0)
            .ok_or("loaded world model0")?;
        let path = canonical_path(path);
        let name = self
            .admit_name(path.as_bytes())
            .map_err(|_| "model name capacity".to_owned())?;
        if !matches!(self.catalog.files[name.0 as usize], ModelFile::Empty) {
            return match self.catalog.files.get(name.0 as usize) {
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
        self.catalog.files[name.0 as usize] = ModelFile::LoadedWorld {
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
        let context = |error: &str| {
            if error.starts_with("model images: ") {
                error.to_owned()
            } else {
                format!("model {path}: {error}")
            }
        };
        let admission = self.ensure_file(vfs, &path);
        let Some(name) = self.catalog.names.find(path.as_bytes()) else {
            return Err(admission.err().map_or_else(
                || "missing registered model name".to_owned(),
                |e| context(&e.message),
            ));
        };
        if matches!(
            self.catalog.files[name.0 as usize],
            ModelFile::LoadedWorld { .. }
        ) {
            return Ok(());
        }
        if let Err(error) = admission
            && !self.catalog.models[name.0 as usize].range().any(|index| {
                !matches!(
                    self.catalog.registrations[index].state,
                    RegistrationState::Unattempted
                )
            })
        {
            if !request.required && matches!(error.kind, CallError::File) {
                return Ok(());
            }
            return Err(context(&error.message));
        }
        let profile = request.profile();
        let registration = self.catalog.models[name.0 as usize]
            .range()
            .map(|index| &self.catalog.registrations[index])
            .find(|entry| entry.profile == profile)
            .ok_or("unconfigured model profile")?;
        if request.required
            && let RegistrationState::Failed(error) = &registration.state
        {
            return Err(context(error));
        }
        Ok(())
    }

    fn ensure_file(&mut self, vfs: &Vfs, path: &str) -> Result<Bounds, FileError> {
        let name = if let Some(name) = self.catalog.names.find(path.as_bytes()) {
            let file = &self.catalog.files[name.0 as usize];
            let unattempted = self.catalog.models[name.0 as usize].range().any(|index| {
                matches!(
                    self.catalog.registrations[index].state,
                    RegistrationState::Unattempted
                )
            });
            if !matches!(file, ModelFile::Empty)
                && !(matches!(file, ModelFile::Ready { .. }) && unattempted)
            {
                return file.bounds().ok_or_else(|| match file {
                    ModelFile::Failed(error) => error.clone(),
                    _ => FileError {
                        kind: CallError::File,
                        message: "empty model filename".to_owned(),
                    },
                });
            }
            name
        } else {
            self.admit_name(path.as_bytes())
                .map_err(|_| FileError::capacity("model name capacity"))?
        };
        let mut span = self.catalog.models[name.0 as usize];
        let fresh_profiles = span.count == 0;
        if span.count == 0 {
            let maximum = self
                .catalog
                .limits
                .models
                .checked_mul(self.catalog.profiles.len())
                .filter(|&count| count <= u32::MAX as usize)
                .ok_or_else(|| FileError::capacity("model registration capacity"))?;
            let next = self
                .catalog
                .registrations
                .len()
                .checked_add(self.catalog.profiles.len())
                .filter(|&count| count <= maximum)
                .ok_or_else(|| FileError::capacity("model registration capacity"))?;
            span = IndexSpan {
                first: self.catalog.registrations.len() as u32,
                count: (next - self.catalog.registrations.len()) as u32,
            };
            self.catalog
                .registrations
                .extend(
                    self.catalog
                        .profiles
                        .iter()
                        .map(|images| RegisteredProfile {
                            profile: images.profile,
                            state: RegistrationState::Unattempted,
                        }),
                );
            self.catalog.models[name.0 as usize] = span;
            self.catalog.capacity.registrations = next;
        }
        let bytes = vfs
            .read_file(
                path.as_bytes(),
                self.catalog.limits.file_bytes,
                &mut self.catalog.reader,
            )
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
        if let Err(error) = &metadata {
            return Err(self.fail_file(name, fresh_profiles, error.clone()));
        }
        let first_tag = self.catalog.tags.len();
        let previous_tags = match &self.catalog.files[name.0 as usize] {
            ModelFile::Ready { tags, .. } => *tags,
            _ => None,
        };
        let tags = if previous_tags.is_some() {
            Ok(previous_tags)
        } else {
            parsed
                .as_ref()
                .map_err(|error| error.message.clone())
                .and_then(|model| self.admit_tags(model))
        };
        if let Err(error) = &tags {
            self.catalog.tags.truncate(first_tag);
            self.catalog.capacity.tags = first_tag;
            return Err(self.fail_file(
                name,
                fresh_profiles,
                FileError {
                    kind: CallError::File,
                    message: error.clone(),
                },
            ));
        }
        let surface_names = parsed
            .as_ref()
            .ok()
            .map(|model| self.admit_surface_names(model))
            .transpose();
        let surface_names = match surface_names {
            Ok(names) => names,
            Err(error) => {
                self.catalog.tags.truncate(first_tag);
                self.catalog.capacity.tags = first_tag;
                return Err(self.fail_file(name, fresh_profiles, error));
            }
        };
        let mut meshes = vec![None; parsed.as_ref().map_or(0, |model| model.meshes.len())];
        if parsed
            .as_ref()
            .is_ok_and(|model| model.format == ModelFormat::Md3)
        {
            self.shader_catalog(vfs);
        }
        let mut renderer_sources = match &self.catalog.files[name.0 as usize] {
            ModelFile::Ready {
                renderer_sources, ..
            } => *renderer_sources,
            _ => 0,
        };
        for (images, entry) in self
            .catalog
            .profiles
            .iter_mut()
            .zip(&mut self.catalog.registrations[span.range()])
            .filter(|(_, entry)| matches!(entry.state, RegistrationState::Unattempted))
        {
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
                                self.catalog.shader_catalog.as_ref(),
                                &mut self.catalog.capacity,
                                self.catalog.limits,
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
            entry.state = match result {
                Ok(surfaces) => {
                    let first = self.catalog.surfaces.len() as u32;
                    let count = surfaces.len() as u32;
                    for (index, skins) in surfaces.into_iter().enumerate() {
                        self.catalog.surfaces.push(SurfaceMaterials {
                            name: surface_names
                                .as_ref()
                                .ok_or_else(|| FileError::capacity("model surface names"))?[index],
                            skins: IndexSpan {
                                first: self.catalog.bindings.len() as u32,
                                count: skins.len() as u32,
                            },
                            wrap_skins: parsed
                                .as_ref()
                                .is_ok_and(|model| model.format == ModelFormat::Md3),
                            frame_skins: parsed
                                .as_ref()
                                .is_ok_and(|model| model.format == ModelFormat::Sp2),
                        });
                        self.catalog.bindings.extend(skins);
                    }
                    renderer_sources |= 1 << images.profile.source_rules as u8;
                    RegistrationState::Ready {
                        value: IndexSpan { first, count },
                        required_assets: (!self.initial_registration).then(|| self.assets.counts()),
                    }
                }
                Err(error) => {
                    self.catalog.diagnostics.push(ModelDiagnostic {
                        name,
                        profile: images.profile,
                        error: error.clone(),
                    });
                    RegistrationState::Failed(error)
                }
            };
        }
        self.catalog.models[name.0 as usize] = span;
        self.catalog.files[name.0 as usize] = match &metadata {
            Ok(metadata) => ModelFile::Ready {
                metadata: *metadata,
                renderer_sources,
                tags: tags.as_ref().ok().copied().flatten(),
            },
            Err(error) => ModelFile::Failed(error.clone()),
        };
        self.catalog.capacity.registrations = self.catalog.registrations.len();
        self.catalog.capacity.bindings =
            self.catalog.bindings.len() * ModelMaterialPolicy::ALL.len();
        metadata.map(|metadata| metadata.bounds)
    }

    fn fail_file(&mut self, name: NameId, fresh_profiles: bool, error: FileError) -> FileError {
        let span = self.catalog.models[name.0 as usize];
        let unused_suffix = fresh_profiles
            && span.range().end == self.catalog.registrations.len()
            && span.range().all(|index| {
                matches!(
                    self.catalog.registrations[index].state,
                    RegistrationState::Unattempted
                )
            });
        for entry in &mut self.catalog.registrations[span.range()] {
            if matches!(entry.state, RegistrationState::Unattempted) {
                self.catalog.diagnostics.push(ModelDiagnostic {
                    name,
                    profile: entry.profile,
                    error: error.message.clone(),
                });
                entry.state = RegistrationState::Failed(error.message.clone());
            }
        }
        if unused_suffix {
            self.catalog.registrations.truncate(span.first as usize);
            self.catalog.models[name.0 as usize] = IndexSpan::default();
            self.catalog.capacity.registrations = self.catalog.registrations.len();
        }
        self.catalog.files[name.0 as usize] = ModelFile::Failed(error.clone());
        error
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
        self.catalog
            .names
            .check_intern_all(raw.iter().copied())
            .map_err(|_| FileError::capacity("model surface names"))?;
        raw.into_iter()
            .map(|raw| {
                let name = self
                    .admit_name(raw)
                    .map_err(|_| FileError::capacity("model surface names"))?;
                self.catalog
                    .names
                    .folded(name)
                    .ok_or_else(|| FileError::capacity("model surface names"))
            })
            .collect()
    }

    fn shader_catalog(&mut self, vfs: &Vfs) -> &Result<ShaderCatalog, String> {
        self.catalog.shader_catalog.get_or_insert_with(|| {
            qa_render::material::load_catalog(vfs)
                .map_err(|error| format!("shader scripts: {error:?}"))
        })
    }

    fn admit_tags(&mut self, model: &Model<'_>) -> Result<Option<TagLayout>, String> {
        if model.format != ModelFormat::Md3 || model.tags_per_frame == 0 {
            return Ok(None);
        }
        let first = self.catalog.tags.len();
        let next = first
            .checked_add(model.tags.len())
            .filter(|&count| count <= self.catalog.limits.tags && count <= u32::MAX as usize)
            .ok_or("model tag capacity")?;
        for frame in model.tags.chunks_exact(model.tags_per_frame) {
            let at = self.catalog.tags.len();
            for tag in frame {
                let name = match self.admit_name(tag.name) {
                    Ok(name) => name,
                    Err(_) => {
                        self.catalog.tags.truncate(first);
                        return Err("model tag name capacity".to_owned());
                    }
                };
                self.catalog.tags.push(TaggedOrientation {
                    name,
                    orientation: tag.orientation,
                });
            }
            // Preserve the original first match for duplicate names in a frame.
            self.catalog.tags[at..].sort_by_key(|tag| tag.name.0);
        }
        self.catalog.capacity.tags = next;
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
        if !self
            .catalog
            .profiles
            .iter()
            .any(|entry| entry.profile.source_rules == source)
        {
            return Err(CallError::File);
        }
        let cached = find_shader_index(&self.catalog.names, &self.catalog.shader_indices, raw)
            .and_then(|(_, index)| index);
        if let Some(index) = cached {
            let span = self.catalog.shaders[index as usize].profiles;
            if let Some(handle) = source_handle(
                span.range()
                    .map(|index| &self.catalog.shader_bindings[index])
                    .filter(|binding| binding.profile.source_rules == source)
                    .map(|binding| &binding.state),
                index as i32 + 1,
            ) {
                return Ok(handle);
            }
        }
        std::str::from_utf8(raw).map_err(|_| CallError::Text)?;
        let index = match cached {
            Some(index) => index as usize,
            None => {
                let maximum = self
                    .catalog
                    .limits
                    .models
                    .checked_mul(self.catalog.profiles.len())
                    .filter(|&n| n <= u32::MAX as usize)
                    .ok_or(CallError::Capacity)?;
                let next = self
                    .catalog
                    .shader_bindings
                    .len()
                    .checked_add(self.catalog.profiles.len())
                    .filter(|&n| n <= maximum)
                    .ok_or(CallError::Capacity)?;
                if self.catalog.shaders.len() >= self.catalog.limits.models
                    || self.catalog.shaders.len() >= i32::MAX as usize
                {
                    return Err(CallError::Capacity);
                }
                let name = self.admit_name(shader_key(raw))?;
                let group = self.catalog.names.folded(name).ok_or(CallError::Aborted)?;
                let index = self.catalog.shaders.len();
                self.catalog.shader_indices[group.0 as usize] = Some(index as u32);
                let span = IndexSpan {
                    first: self.catalog.shader_bindings.len() as u32,
                    count: (next - self.catalog.shader_bindings.len()) as u32,
                };
                self.catalog.shaders.push(RegisteredShader {
                    name,
                    profiles: span,
                });
                self.catalog
                    .shader_bindings
                    .extend(
                        self.catalog
                            .profiles
                            .iter()
                            .map(|images| ShaderProfileBinding {
                                profile: images.profile,
                                state: RegistrationState::Unattempted,
                            }),
                    );
                index
            }
        };
        let shader = &self.catalog.shaders[index];
        let name = shader.name;
        let span = shader.profiles;
        let definition = shader_definition(self.shader_catalog(vfs), raw, mipmaps, false);
        let mut ready = false;
        for (images, binding) in self
            .catalog
            .profiles
            .iter_mut()
            .zip(&mut self.catalog.shader_bindings[span.range()])
            .filter(|(images, binding)| {
                images.profile.source_rules == source
                    && matches!(binding.state, RegistrationState::Unattempted)
            })
        {
            binding.state = match images.compile(
                vfs,
                &mut self.assets,
                definition.as_ref().map_err(Clone::clone),
            ) {
                Ok(material) => {
                    ready = true;
                    RegistrationState::Ready {
                        value: material,
                        required_assets: (!self.initial_registration).then(|| self.assets.counts()),
                    }
                }
                Err(error) => {
                    self.catalog.diagnostics.push(ModelDiagnostic {
                        name,
                        profile: images.profile,
                        error: error.clone(),
                    });
                    RegistrationState::Failed(error)
                }
            };
        }
        if ready {
            Ok(index as i32 + 1)
        } else {
            Err(CallError::File)
        }
    }

    fn admit_inline(&mut self, world: WorldId, index: u32) -> Result<i32, CallError> {
        let slot = inline_slot(&self.catalog.worlds, world, index).ok_or(CallError::File)?;
        if let Some(handle) = self.catalog.inline[slot].handle {
            return i32::try_from(handle.0).map_err(|_| CallError::Capacity);
        }
        let binding = self.catalog.inline[slot].binding;
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
        self.catalog.files[handle.0 as usize] = ModelFile::LoadedWorld {
            binding,
            bounds,
            collision: None,
        };
        self.catalog.inline[slot].handle = Some(handle);
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
            &self.catalog.names,
            &self.catalog.skin_indices,
            &self.catalog.skins,
            path.as_bytes(),
            source,
        ) {
            return Ok(handle);
        }
        let name = self.admit_name(path.as_bytes())?;
        let group = self.catalog.names.folded(name).ok_or(CallError::Aborted)?;
        let index = if let Some(index) = self.catalog.skin_indices[group.0 as usize] {
            index as usize
        } else {
            if self.catalog.skins.len() >= self.catalog.limits.models
                || self.catalog.skins.len() >= i32::MAX as usize
            {
                return Err(CallError::Capacity);
            }
            let index = self.catalog.skins.len();
            self.catalog.skin_indices[group.0 as usize] = Some(index as u32);
            self.catalog.skins.push(RegisteredSkin {
                profiles: self
                    .catalog
                    .profiles
                    .iter()
                    .map(|images| SkinProfile {
                        profile: images.profile,
                        state: RegistrationState::Unattempted,
                    })
                    .collect(),
            });
            index
        };
        let profile_count = self.catalog.skins[index]
            .profiles
            .iter()
            .filter(|entry| matches!(entry.state, RegistrationState::Unattempted))
            .count();
        let maximum_rows = self
            .catalog
            .limits
            .skins
            .saturating_sub(self.catalog.capacity.skins)
            / profile_count.max(1);
        let skin_file = raw.ends_with(b".skin");
        let mappings = (|| {
            let bytes = if skin_file {
                vfs.read_file(
                    path.as_bytes(),
                    self.catalog.limits.file_bytes,
                    &mut self.catalog.reader,
                )
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
            self.catalog.capacity.skins = rows
                .len()
                .checked_mul(profile_count)
                .and_then(|count| self.catalog.capacity.skins.checked_add(count))
                .filter(|&count| count <= self.catalog.limits.skins)
                .ok_or(CallError::Capacity)?;
            self.shader_catalog(vfs);
            let mut mappings = Vec::with_capacity(rows.len());
            for row in rows {
                let surface = self.admit_name(row.surface)?;
                let surface = self
                    .catalog
                    .names
                    .folded(surface)
                    .ok_or(CallError::Aborted)?;
                let definition = shader_definition(
                    self.catalog
                        .shader_catalog
                        .as_ref()
                        .ok_or(CallError::Aborted)?,
                    row.shader,
                    true,
                    true,
                );
                mappings.push((surface, definition));
            }
            Ok(mappings)
        })();
        let mappings = match mappings {
            Ok(mappings) => mappings,
            Err(error) => {
                for profile in &mut self.catalog.skins[index].profiles {
                    if matches!(profile.state, RegistrationState::Unattempted) {
                        profile.state =
                            RegistrationState::Failed(format!("skin registration: {error:?}"));
                    }
                }
                return Err(error);
            }
        };
        for (images, profile) in self
            .catalog
            .profiles
            .iter_mut()
            .zip(&mut self.catalog.skins[index].profiles)
            .filter(|(_, profile)| matches!(profile.state, RegistrationState::Unattempted))
        {
            let alias = !skin_file && images.model_policy.source.indexed_external_pcx;
            let mut compiled = Vec::with_capacity(mappings.len());
            let mut failure = None;
            for (surface, definition) in &mappings {
                let result = if alias {
                    let registration = Registration {
                        path: path.clone(),
                        profile: images.profile,
                        required: false,
                    };
                    let model_policy = images.model_policy;
                    images.with_images(vfs, &mut self.assets, |images| {
                        let mut materials = alias_materials(
                            &[Image::External(path.as_bytes())],
                            images,
                            &registration,
                            model_policy,
                            false,
                        )?;
                        materials
                            .pop()
                            .ok_or_else(|| "missing skin material".to_owned())
                    })
                } else {
                    images
                        .compile(
                            vfs,
                            &mut self.assets,
                            definition.as_ref().map_err(Clone::clone),
                        )
                        .map(|material| [material; 4])
                };
                let materials = match result {
                    Ok(materials) => materials,
                    Err(error) => {
                        self.catalog.diagnostics.push(ModelDiagnostic {
                            name,
                            profile: images.profile,
                            error: error.clone(),
                        });
                        failure = Some(error);
                        if alias {
                            continue;
                        }
                        [MaterialId(0); 4]
                    }
                };
                compiled.push(SkinMapping {
                    surface: *surface,
                    materials,
                });
            }
            compiled.sort_by_key(|entry| entry.surface.0);
            compiled.dedup_by_key(|entry| entry.surface.0);
            profile.state = if compiled.is_empty() {
                RegistrationState::Failed(
                    failure.unwrap_or_else(|| "missing skin mapping".to_owned()),
                )
            } else {
                RegistrationState::Ready {
                    value: compiled.into_boxed_slice(),
                    required_assets: (!self.initial_registration).then(|| self.assets.counts()),
                }
            };
        }
        self.catalog.skins[index]
            .source_handle(index as i32 + 1, source)
            .ok_or(CallError::Aborted)
    }
}

fn shader_key(name: &[u8]) -> &[u8] {
    &name[..name
        .iter()
        .position(|&byte| byte == b'.')
        .unwrap_or(name.len())]
}
fn shader_handle(catalog: &ModelCatalog, raw: &[u8], source: RuleSetId) -> Option<i32> {
    if raw.is_empty() {
        return Some(0);
    }
    let (_, index) = find_shader_index(&catalog.names, &catalog.shader_indices, raw)?;
    let index = index?;
    let entry = catalog.shaders.get(index as usize)?;
    source_handle(
        entry
            .profiles
            .range()
            .map(|index| &catalog.shader_bindings[index])
            .filter(|entry| entry.profile.source_rules == source)
            .map(|entry| &entry.state),
        index as i32 + 1,
    )
}

fn find_shader_index(
    names: &NameTable,
    indices: &[Option<u32>],
    raw: &[u8],
) -> Option<(NameId, Option<u32>)> {
    let group = names.find_folded(shader_key(raw))?;
    Some((group, indices.get(group.0 as usize).copied().flatten()))
}
fn skin_handle(
    names: &NameTable,
    indices: &[Option<u32>],
    skins: &[RegisteredSkin],
    raw: &[u8],
    source: RuleSetId,
) -> Option<i32> {
    if raw.is_empty() {
        return Some(0);
    }
    let group = names.find_path_bytes(raw)?;
    let group = names.folded(group)?;
    let index = indices.get(group.0 as usize).copied().flatten()?;
    let skin = skins.get(index as usize)?;
    skin.source_handle(index as i32 + 1, source)
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
    source: Option<RuleSetId>,
) -> Option<i32> {
    let name = names.find_path(std::str::from_utf8(path).ok()?)?;
    let handle = i32::try_from(name.0).ok()?;
    renderer_file(files, handle, source)?;
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
impl ModelLoader for ModelAssets {
    fn lookup(&self) -> &dyn ModelLookup {
        &self.catalog
    }
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
        if let Some(bounds) = file_lookup(&self.catalog.names, &self.catalog.files, path.as_bytes())
            .and_then(ModelFile::bounds)
        {
            return Ok(bounds);
        }
        self.ensure_file(vfs, &canonical_path(path))
            .map_err(|error| error.kind)
    }
    fn register_model(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
    ) -> Result<i32, CallError> {
        if let Some(handle) = self.catalog.model_source_cached(name, source) {
            return Ok(handle);
        }
        let path = std::str::from_utf8(name).map_err(|_| CallError::Text)?;
        if path.is_empty() || path.starts_with('*') {
            return Err(CallError::File);
        }
        self.ensure_file(vfs, &canonical_path(path))
            .map_err(|error| error.kind)?;
        self.catalog
            .model_handle(name, source)
            .ok_or(CallError::File)
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
    fn file_handle(&self, path: &[u8]) -> Option<i32> {
        model_handle(&self.names, &self.files, path, None)
    }
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
    fn skin_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        skin_handle(&self.names, &self.skin_indices, &self.skins, name, source)
    }
    fn shader_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        shader_handle(self, name, source)
    }
    fn model_handle(&self, path: &[u8], source: RuleSetId) -> Option<i32> {
        model_handle(&self.names, &self.files, path, Some(source))
    }
    fn model_bounds(&self, handle: i32) -> Option<Bounds> {
        model_bounds(&self.files, handle)
    }
}
impl ModelCatalog {
    fn admitted(&self, required: Option<AssetCounts>) -> bool {
        required.is_none_or(|required| {
            self.admitted_assets
                .is_some_and(|admitted| admitted.contains(required))
        })
    }

    /// None requests preparation; zero is an attempted native failure.
    pub fn registration_cached(
        &self,
        kind: RendererRegistration,
        name: &[u8],
        profile: ModelProfile,
        world: Option<WorldId>,
    ) -> Option<i32> {
        if name.is_empty() {
            return Some(0);
        }
        match kind {
            RendererRegistration::Model if name.starts_with(b"*") => {
                let Some((world, index)) = world.zip(qa_core::text::inline_model_index(name))
                else {
                    return Some(0);
                };
                if self.resolve_inline(world, index).is_none() {
                    return Some(0);
                }
                self.inline_handle(world, index)
            }
            RendererRegistration::Model => {
                let name = self.names.find_path_bytes(name)?;
                let handle = i32::try_from(name.0).ok()?;
                match self.files.get(name.0 as usize)? {
                    ModelFile::Failed(_) => Some(0),
                    ModelFile::LoadedWorld { .. } => Some(handle),
                    ModelFile::Empty | ModelFile::Ready { .. } => self.models[name.0 as usize]
                        .range()
                        .map(|index| &self.registrations[index])
                        .find(|entry| entry.profile == profile)?
                        .state
                        .cached(handle, self.admitted_assets),
                }
            }
            RendererRegistration::Shader { .. } => {
                let (_, index) = find_shader_index(&self.names, &self.shader_indices, name)?;
                let handle = i32::try_from(index?.checked_add(1)?).ok()?;
                self.shader_profile(handle, profile)?
                    .state
                    .cached(handle, self.admitted_assets)
            }
            RendererRegistration::Skin => {
                let name = self.names.find_path_bytes(name)?;
                let group = self.names.folded(name)?;
                let index = self.skin_indices[group.0 as usize]?;
                let handle = i32::try_from(index.checked_add(1)?).ok()?;
                self.skin_profile(handle, profile)?
                    .state
                    .cached(handle, self.admitted_assets)
            }
        }
    }
    fn model_source_cached(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        let name = self.names.find_path_bytes(name)?;
        let handle = i32::try_from(name.0).ok()?;
        match self.files.get(name.0 as usize)? {
            ModelFile::Failed(_) => return Some(0),
            ModelFile::LoadedWorld { .. } => return Some(handle),
            ModelFile::Empty | ModelFile::Ready { .. } => {}
        }
        source_handle(
            self.models[name.0 as usize]
                .range()
                .map(|index| &self.registrations[index])
                .filter(|entry| entry.profile.source_rules == source)
                .map(|entry| &entry.state),
            handle,
        )
    }

    /// Exact-profile decoded candidates are checked before backend preparation.
    /// This does not publish them to rendering consumers.
    pub fn renderer_candidate(
        &self,
        kind: RendererRegistration,
        handle: i32,
        profile: ModelProfile,
    ) -> bool {
        if handle <= 0 {
            return false;
        }
        match kind {
            RendererRegistration::Model => {
                self.renderer_inline(handle).is_some()
                    || self.renderer_model_candidate(handle, profile).is_some()
            }
            RendererRegistration::Skin => self
                .skin_profile(handle, profile)
                .is_some_and(|entry| entry.state.ready().is_some()),
            RendererRegistration::Shader { .. } => self
                .shader_profile(handle, profile)
                .is_some_and(|entry| matches!(entry.state, RegistrationState::Ready { .. })),
        }
    }

    pub fn color_binding(&self, profile: ModelProfile) -> Option<ModelColorId> {
        self.profiles
            .iter()
            .position(|entry| entry.profile == profile && entry.colors.is_some())
            .map(|index| ModelColorId(index as u32))
    }
    pub fn colors(&self, binding: ModelColorId) -> Option<&PaletteColors> {
        self.profiles.get(binding.0 as usize)?.colors.as_ref()
    }
    pub fn shader_binding(&self, handle: i32, profile: ModelProfile) -> Option<MaterialId> {
        let entry = self.shader_profile(handle, profile)?;
        let (material, required_assets) = entry.state.ready()?;
        self.admitted(required_assets).then_some(*material)
    }

    fn shader_profile(&self, handle: i32, profile: ModelProfile) -> Option<&ShaderProfileBinding> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.shaders
            .get(index)?
            .profiles
            .range()
            .map(|i| &self.shader_bindings[i])
            .find(|entry| entry.profile == profile)
    }
    pub fn shader_name(&self, handle: i32) -> Option<&[u8]> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.names.get(self.shaders.get(index)?.name)
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
        self.published_registration(self.registration_name(name, profile)?)
    }
    /// Native handles already identify the interned file. Resolve their exact
    /// presentation registration without repeating filename conversion/lookup.
    pub fn renderer_registration(
        &self,
        handle: i32,
        profile: ModelProfile,
    ) -> Option<ModelRegistrationId> {
        self.published_registration(self.renderer_model_candidate(handle, profile)?)
    }
    fn renderer_model_candidate(
        &self,
        handle: i32,
        profile: ModelProfile,
    ) -> Option<ModelRegistrationId> {
        renderer_file(&self.files, handle, Some(profile.source_rules))?;
        self.registration_name(NameId(u32::try_from(handle).ok()?), profile)
    }
    /// A logical skin handle is usable only when its selected presentation
    /// registered successfully. Other profiles cannot supply a fallback here.
    pub fn renderer_skin(&self, handle: i32, profile: ModelProfile) -> bool {
        self.skin_profile(handle, profile)
            .is_some_and(|entry| entry.state.cached(handle, self.admitted_assets) == Some(handle))
    }
    fn skin_profile(&self, handle: i32, profile: ModelProfile) -> Option<&SkinProfile> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.skins
            .get(index)?
            .profiles
            .iter()
            .find(|entry| entry.profile == profile)
    }
    fn published_registration(
        &self,
        registration: ModelRegistrationId,
    ) -> Option<ModelRegistrationId> {
        let (_, required_assets) = self
            .registrations
            .get(registration.0 as usize)?
            .state
            .ready()?;
        self.admitted(required_assets).then_some(registration)
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
                entry.profile == profile && entry.state.ready().is_some()
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
        self.published_registration(registration)?;
        let span = *self
            .registrations
            .get(registration.0 as usize)?
            .state
            .ready()?
            .0;
        let custom = match policy
            .custom_skin
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok())
            .and_then(|index| self.skins.get(index))
        {
            Some(skin) => Some(
                match skin.profiles.iter().find(|skin| {
                    skin.profile == self.registrations[registration.0 as usize].profile
                }) {
                    Some(profile) => match &profile.state {
                        RegistrationState::Ready {
                            value,
                            required_assets,
                        } => {
                            if !self.admitted(*required_assets) {
                                return None;
                            }
                            value.as_ref()
                        }
                        RegistrationState::Failed(_) => &[][..],
                        RegistrationState::Unattempted => return None,
                    },
                    None => &[][..],
                },
            ),
            None => None,
        };
        Some(ModelBindings {
            surfaces: &self.surfaces[span.range()],
            materials: &self.bindings,
            skin,
            policy,
            custom,
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
) -> Result<(Vec<IndexSpan>, Vec<RegisteredInline>), String> {
    let mut ranges = vec![IndexSpan::default(); assets.worlds().len()];
    let mut inline = Vec::new();
    for id in worlds {
        append_inline_bindings(assets, &mut ranges, &mut inline, id, limits)?;
    }
    Ok((ranges, inline))
}

fn append_inline_bindings(
    assets: &Assets,
    ranges: &mut Vec<IndexSpan>,
    inline: &mut Vec<RegisteredInline>,
    id: WorldId,
    limits: ModelLimits,
) -> Result<(), String> {
    let geometry = assets
        .world(id)
        .ok_or("inline model world handle")?
        .geometry();
    if ranges
        .get(id.0 as usize)
        .is_some_and(|range| range.count != 0)
    {
        return Ok(());
    }
    let next = inline
        .len()
        .checked_add(geometry.models.len())
        .filter(|&count| count <= limits.inline_models && count <= u32::MAX as usize)
        .ok_or("inline model capacity")?;
    ranges.resize(assets.worlds().len(), IndexSpan::default());
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
    ranges[id.0 as usize] = IndexSpan {
        first,
        count: (next - first as usize) as u32,
    };
    Ok(())
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
            model.format == ModelFormat::Md2,
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
        let image = model_image(sprite.image, images, registration.profile, true, Some(255))
            .map_err(|error| format!("sprite image: {error:?}"))?;
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
) -> Result<ImageId, ResourceError> {
    let usage = ImageUse::default();
    Ok(match image {
        Image::Indexed {
            width,
            height,
            pixels,
        } => {
            let palette = options.source_palette.ok_or(ResourceError::Asset(
                "indexed model requires a source palette",
            ))?;
            let texture = IndexedTexture::load_base(width, height, pixels, transparent)
                .map_err(ResourceError::Asset)?;
            let image = images.register_indexed(texture, palette)?;
            let params = images.upload_params(usage)?;
            images
                .assets
                .prepare_image(image, params)
                .map_err(ResourceError::Asset)?;
            images
                .assets
                .set_image_native_sampler(image, usage.sampler)
                .map_err(ResourceError::Asset)?;
            image
        }
        Image::External(path) => {
            let path = std::str::from_utf8(path)
                .map_err(|_| ResourceError::Asset("skin path is not UTF-8"))?;
            if indexed_external_pcx
                && qa_formats::image::ImageFormat::from_path(path.as_bytes())
                    == Some(qa_formats::image::ImageFormat::Pcx)
            {
                images.indexed_pcx(
                    path,
                    options
                        .source_palette
                        .ok_or(ResourceError::Asset("PCX skin requires a source palette"))?,
                    transparent,
                    None,
                    usage,
                )?
            } else {
                images
                    .raster(
                        path,
                        usage,
                        options.palette.map(|palette| IndexedTarget {
                            palette,
                            transparent_index: Some(255),
                        }),
                    )?
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
    allow_missing: bool,
) -> Result<Vec<[MaterialId; 4]>, String> {
    let options = registration.profile;
    let mut materials = Vec::with_capacity(skins.len());
    let mut default_image = None;
    for (index, skin) in skins.iter().enumerate() {
        let result = model_image(
            *skin,
            images,
            options,
            model_policy.source.indexed_external_pcx,
            None,
        );
        let missing_skin = matches!(&result, Err(ResourceError::Missing(_)));
        let image = match result {
            Ok(image) => image,
            Err(ResourceError::Missing(_)) if allow_missing => match default_image {
                Some(image) => image,
                None => images
                    .missing_skin(options.source_palette.or(options.palette))
                    .map_err(|error| format!("missing skin image: {error:?}"))?,
            },
            Err(error) => return Err(format!("skin: {error:?}")),
        };
        if index == 0 {
            default_image = Some(image);
        }
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
                            missing_skin,
                            infrared_map: model_policy.presentation.infrared_map,
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
