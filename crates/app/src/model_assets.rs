//! Cold model registration. Native config ordinals resolve through filenames;
//! inline ordinals retain their explicit world and existing surface ranges.
use qa_compat::services::{
    CallError, LoadedWorldBinding, ModelLoader, ModelLookup, RendererRegistration, RendererRequest,
    ShaderLighting,
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
    ImageRegistrationMark, ImageRole, ImageSearchOrder, ImageSettings, ImageUse, Images,
    ImagesState, IndexedTarget, ResourceError,
};
use qa_render::material::world_load::{
    NamedShaderLighting, WorldPresentationRequest, compile_named_shader, native_default_shader,
};
use qa_render::material::{MaterialFinishPolicy, ShaderCompilePolicy};
use qa_render::shader::reference::{ShaderChoice, ShaderDefault, ShaderOccurrence, ShaderView};
use qa_render::shader::{AlphaFunc, AlphaGen, Cull, RgbGen, ShaderCatalog, StageBlend};
use qa_render::surface_cache::IndexedTexture;
use qa_render::world::geometry::LightSource;
use qa_render::world::{WorldId, WorldPresentationId};
use qa_render::{
    Assets, ModelId, PaletteId, ShaderBinding, ShaderBindingId, ShaderRegistration, Vertex,
};
use std::sync::Arc;

#[cfg(test)]
#[path = "../tests/unit/renderer_registrations.rs"]
mod registration_tests;

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
    pub finish: MaterialFinishPolicy,
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
            finish: MaterialFinishPolicy::default(),
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
            finish: self.finish,
        }
    }
}

pub use qa_render::shader::registration::{ModelProfile, ShaderResource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationVariant {
    Retained,
    Guard(bool),
}
impl RegistrationVariant {
    fn matches(self, profile: ModelProfile, guard: bool) -> bool {
        match self {
            Self::Retained => true,
            Self::Guard(current) => profile.finish.compile(current).collapse_vertex() == guard,
        }
    }
    fn guard(self) -> Result<bool, String> {
        match self {
            Self::Guard(current) => Ok(current),
            Self::Retained => Err("retained renderer resource is unavailable".into()),
        }
    }
}

fn with_image_owner<T>(
    profiles: &mut [ProfileImages],
    index: usize,
    operation: impl FnOnce(&mut ProfileImages) -> T,
) -> T {
    let owner = profiles[index].image_owner;
    if let Some(owner) = owner {
        let (before, current) = profiles.split_at_mut(index);
        std::mem::swap(&mut before[owner].state, &mut current[0].state);
        let result = operation(&mut current[0]);
        std::mem::swap(&mut before[owner].state, &mut current[0].state);
        result
    } else {
        operation(&mut profiles[index])
    }
}

/// Cold world policies are explicit and independent of BSP decoding.
pub struct WorldRegistration {
    pub profile: ModelProfile,
    pub presentation: qa_render::material::world_load::WorldPresentationProfile,
    pub options: qa_render::material::world_load::WorldLoadOptions,
    pub ui_fullscreen: bool,
    pub trace_rules: qa_world::collision::TraceRules,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelRegistrationId(u32);

/// Immutable resource selectors prepared for one image-policy publication.
#[derive(Default)]
pub(crate) struct ImageRefresh {
    pub(crate) profiles: Vec<(ModelProfile, ModelProfile)>,
    pub(crate) worlds: Vec<(
        WorldId,
        WorldPresentationId,
        qa_render::material::world_load::LoadedWorldPresentation,
    )>,
}
impl ImageRefresh {
    pub(crate) fn profile(&self, old: ModelProfile) -> ModelProfile {
        self.profiles
            .iter()
            .find_map(|&(from, to)| (from == old).then_some(to))
            .unwrap_or(old)
    }
    pub(crate) fn presentation(
        &self,
        world: WorldId,
        old: WorldPresentationId,
    ) -> Option<qa_render::material::world_load::LoadedWorldPresentation> {
        self.worlds
            .iter()
            .find_map(|&(owner, from, to)| (owner == world && from == old).then_some(to))
    }
    pub(crate) fn registration(
        &self,
        old: ModelRegistrationId,
        catalog: &ModelCatalog,
    ) -> Result<ModelRegistrationId, String> {
        let entry = catalog
            .registrations
            .get(old.0 as usize)
            .filter(|entry| entry.state.ready().is_some())
            .ok_or("image refresh model registration")?;
        let profile = self.profile(entry.profile);
        if profile == entry.profile {
            return Ok(old);
        }
        catalog
            .registration_name(entry.file, profile)
            .ok_or_else(|| "image refresh model unavailable".to_owned())
    }
}

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
    pub shader: ShaderBindingId,
    /// Native model presentation chosen at registration, independent of wire.
    pub pose: AliasPose,
}
impl ModelBinding {
    pub fn scene_part(self) -> qa_render::scene::ModelPart {
        qa_render::scene::ModelPart {
            model: self.model,
            shader: self.shader.into(),
        }
    }
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
    variants: [ModelChoice; 4],
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct ModelChoice {
    model: ModelId,
    shader: ShaderChoice,
    pose: AliasPose,
}
#[derive(Clone)]
pub struct SelectedModelChoice<'a> {
    pub model: ModelId,
    pub shader: ShaderChoice,
    pub pose: AliasPose,
    pub resource: ShaderResource,
    pub registrations: &'a [ShaderRegistration],
    pub default: ShaderDefault,
}
impl SelectedModelChoice<'_> {
    pub fn physical_binding(&self) -> Option<ModelBinding> {
        let shader = ShaderView::direct()
            .choice(self.shader, self.registrations, None, self.default)?
            .source(&())?;
        Some(ModelBinding {
            model: self.model,
            shader,
            pose: self.pose,
        })
    }
}
#[derive(Clone)]
pub struct ModelChoices<'a>(ModelBindings<'a>);
impl<'a> Iterator for ModelChoices<'a> {
    type Item = SelectedModelChoice<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next_choice()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
impl ExactSizeIterator for ModelChoices<'_> {}
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
    custom: Option<(u32, Option<&'a SkinReady>)>,
    registrations: &'a [ShaderRegistration],
    resource: ShaderResource,
    default_shader: ShaderBindingId,
}
impl<'a> ModelBindings<'a> {
    pub fn choices(self) -> ModelChoices<'a> {
        ModelChoices(self)
    }
    fn next_choice(&mut self) -> Option<SelectedModelChoice<'a>> {
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
        let mut registrations = self.registrations;
        let mut resource = self.resource;
        let mut default = ShaderDefault::Global(Some(self.default_shader));
        if let Some((handle, custom)) = self.custom.filter(|_| !surface.frame_skins) {
            binding.shader = ShaderChoice::Direct(self.default_shader);
            if let Some(custom) = custom {
                let mapping = custom
                    .mappings
                    .binary_search_by_key(&surface.name.0, |entry| entry.surface.0)
                    .ok();
                binding.shader = mapping
                    .map_or(ShaderChoice::Direct(self.default_shader), |index| {
                        custom.mappings[index].shaders[self.policy.index()]
                    });
                registrations = &custom.registrations;
                resource = ShaderResource::Skin(handle);
                default = ShaderDefault::Named;
            }
        }
        Some(SelectedModelChoice {
            model: binding.model,
            shader: binding.shader,
            pose: binding.pose,
            resource,
            registrations,
            default,
        })
    }
}
impl Iterator for ModelBindings<'_> {
    type Item = ModelBinding;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_choice()?.physical_binding()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.surfaces.len(), Some(self.surfaces.len()))
    }
}
impl ExactSizeIterator for ModelBindings<'_> {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelLodInfo<'a> {
    native_count: usize,
    md3: bool,
    radii: &'a [f32],
}
impl ModelLodInfo<'_> {
    pub fn native_count(self) -> usize {
        self.native_count
    }
    pub fn base_frames(self) -> usize {
        self.radii.len()
    }
    pub fn frame_count(self, physical_frames: u32) -> u32 {
        if self.md3 {
            self.radii.len() as u32
        } else {
            physical_frames
        }
    }
    pub fn radius(self, frame: u32) -> Option<f32> {
        self.radii.get(frame as usize).copied()
    }
    pub fn is_md3(self) -> bool {
        self.md3
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ModelLodBindings<'a> {
    levels: [Option<ModelBindings<'a>>; 3],
    info: ModelLodInfo<'a>,
    remaining: usize,
}
impl<'a> ModelLodBindings<'a> {
    pub fn base(&self) -> Option<ModelBindings<'a>> {
        self.levels[0].clone()
    }
    pub fn info(&self) -> ModelLodInfo<'a> {
        self.info
    }
    pub fn native_count(&self) -> usize {
        self.info.native_count()
    }
    pub fn base_frames(&self) -> usize {
        self.info.base_frames()
    }
    pub fn radius(&self, frame: u32) -> Option<f32> {
        self.info.radius(frame)
    }
}
impl<'a> Iterator for ModelLodBindings<'a> {
    type Item = [Option<SelectedModelChoice<'a>>; 3];
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(std::array::from_fn(|index| {
            self.levels[index]
                .as_mut()
                .and_then(ModelBindings::next_choice)
        }))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl ExactSizeIterator for ModelLodBindings<'_> {}
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
    file: NameId,
    profile: ModelProfile,
    vertex_collapse: bool,
    state: RegistrationState<ModelReady>,
}
struct ModelReady {
    surfaces: [Option<IndexSpan>; 3],
    default_shader: ShaderBindingId,
    registrations: Box<[ShaderRegistration]>,
}
struct RegisteredShader {
    name: NameId,
    profiles: Vec<usize>,
    requests: [Option<ShaderRequest>; 5],
}
#[derive(Clone)]
struct ShaderRequest {
    name: qa_core::text::FixedText<64>,
    mipmaps: bool,
}
struct ShaderProfileBinding {
    profile: ModelProfile,
    vertex_collapse: bool,
    lighting: ShaderLighting,
    state: RegistrationState<ShaderRegistration>,
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
    public: impl Fn(&T) -> bool,
) -> Option<i32> {
    let mut ready = false;
    let mut configured = false;
    for state in states {
        configured = true;
        match state {
            RegistrationState::Unattempted => return None,
            RegistrationState::Ready { value, .. } => ready |= public(value),
            RegistrationState::Failed(_) => {}
        }
    }
    configured.then_some(if ready { handle } else { 0 })
}

struct SkinProfile {
    profile: ModelProfile,
    vertex_collapse: bool,
    state: RegistrationState<SkinReady>,
}
#[derive(Debug, PartialEq)]
struct SkinReady {
    mappings: Box<[SkinMapping]>,
    registrations: Box<[ShaderRegistration]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct SkinMapping {
    surface: NameId,
    shaders: [ShaderChoice; 4],
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
    fn tag(message: String) -> Self {
        Self {
            kind: CallError::File,
            message,
        }
    }
}
#[derive(Clone)]
enum ModelFile {
    Empty,
    Ready {
        lods: ModelLods,
        renderer_sources: u8,
    },
    LoadedWorld {
        binding: InlineBinding,
        bounds: Bounds,
        collision: Option<qa_core::primitives::GeometryId>,
    },
    Failed(FileError),
}
#[derive(Clone, Copy)]
enum ModelLodSlot {
    Missing,
    Invalid,
    Source(NameId),
    Alias(u8),
}
#[derive(Clone, Copy)]
struct ModelLods {
    slots: [ModelLodSlot; 3],
    native_count: usize,
    md3: bool,
}
impl ModelLods {
    fn source(self, mut slot: usize) -> Option<NameId> {
        for _ in 0..self.slots.len() {
            match *self.slots.get(slot)? {
                ModelLodSlot::Source(name) => return Some(name),
                ModelLodSlot::Alias(next) if usize::from(next) > slot => slot = usize::from(next),
                _ => return None,
            }
        }
        None
    }
}
#[derive(Clone)]
enum ModelSourceFile {
    Empty,
    Invalid {
        error: FileError,
        md3: bool,
    },
    Ready {
        bytes: Arc<[u8]>,
        metadata: ModelMetadata,
        radii: Arc<[f32]>,
        tags: Option<TagLayout>,
        meshes: Vec<Option<ModelId>>,
    },
}
impl ModelSourceFile {
    fn metadata(&self) -> Option<ModelMetadata> {
        match self {
            Self::Ready { metadata, .. } => Some(*metadata),
            _ => None,
        }
    }
    fn rollback_resources(&mut self, retained: AssetCounts, retained_tags: usize) {
        if let Self::Ready { meshes, tags, .. } = self {
            for mesh in meshes {
                if mesh.is_some_and(|id| id.0 as usize >= retained.models) {
                    *mesh = None;
                }
            }
            if tags.is_some_and(|layout| layout.samples.range().end > retained_tags) {
                *tags = None;
            }
        }
    }
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
    fn bounds(&self, sources: &[ModelSourceFile]) -> Option<Bounds> {
        match self {
            Self::Ready { lods, .. } => sources
                .get(lods.source(0)?.0 as usize)?
                .metadata()
                .map(|metadata| metadata.bounds),
            Self::LoadedWorld { bounds, .. } => Some(*bounds),
            Self::Empty | Self::Failed(_) => None,
        }
    }
}
struct ProfileImages {
    profile: ModelProfile,
    image_owner: Option<usize>,
    model_policy: ModelPolicy,
    // Temporarily taken only while Images borrows the separately owned Assets.
    state: Option<Result<ImagesState, String>>,
    colors: Option<PaletteColors>,
    native_default: Option<(ShaderBindingId, AssetCounts)>,
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
fn model_policy(profile: ModelProfile) -> ModelPolicy {
    ModelPolicy {
        source: appearance::model_source(profile.source_rules),
        presentation: appearance::model_presentation(profile.rules),
    }
}
impl ProfileImages {
    fn new(
        profile: ModelProfile,
        previous: &[Self],
        assets: &mut Assets,
    ) -> Result<(Self, Option<qa_render::assets::PreparedPaletteTranslucency>), String> {
        if let Some((index, owner)) = previous.iter().enumerate().find(|(_, owner)| {
            owner.image_owner.is_none() && owner.profile.image_profile() == profile.image_profile()
        }) {
            return Ok((
                Self {
                    profile,
                    image_owner: Some(index),
                    model_policy: model_policy(profile),
                    state: None,
                    colors: None,
                    native_default: owner.native_default,
                },
                None,
            ));
        }
        let blend = if let Some((source, target)) = profile
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
            assets.prepare_palette_translucency(target)?
        } else {
            None
        };
        let mut state = ImagesState::new(
            profile.source_rules,
            profile.rules,
            profile.image_settings,
            profile.source_palette.zip(profile.palette),
        )
        .map_err(|error| format!("model images: {error:?}"));
        let native_default = if profile.source_rules == RuleSetId::Quake3 {
            match state.as_mut() {
                Ok(state) => {
                    let existing = assets.world_presentations().iter().enumerate().find_map(
                        |(index, presentation)| {
                            let registered = assets.world_presentation_registration(
                                WorldPresentationId(index as u32),
                            )?;
                            (registered.profile.rules == profile.rules
                                && registered.profile.indexed_palette == profile.palette
                                && registered.profile.image_settings == profile.image_settings
                                && registered.profile.finish == profile.finish)
                                .then(|| presentation.native_default())
                                .flatten()
                        },
                    );
                    let shader = native_default_shader(
                        state,
                        assets,
                        profile.palette.map(|palette| IndexedTarget {
                            palette,
                            transparent_index: None,
                        }),
                        existing,
                        profile.finish.compile(false),
                    )
                    .map_err(|error| format!("native default shader: {error:?}"))?;
                    Some((shader, assets.counts()))
                }
                Err(_) => None,
            }
        } else {
            None
        };
        Ok((
            Self {
                profile,
                model_policy: model_policy(profile),
                image_owner: None,
                state: Some(state),
                colors: PaletteColors::load(profile, assets),
                native_default,
            },
            blend,
        ))
    }
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
        catalog: Result<&ShaderCatalog, String>,
        name: &[u8],
        mipmaps: bool,
        lighting: NamedShaderLighting,
        ui_fullscreen: bool,
    ) -> Result<ShaderRegistration, String> {
        let policy = self.profile.finish.compile(ui_fullscreen);
        self.with_images(vfs, assets, |images| {
            compile_named_shader(catalog?, name, mipmaps, lighting, policy, images)
                .map_err(|error| format!("shader material: {error:?}"))
        })
    }
}

/// The session's one model store and retained registration state.
pub struct ModelCatalog {
    reader: ArchiveReader,
    names: NameTable,
    files: Vec<ModelFile>,
    sources: Vec<ModelSourceFile>,
    models: Vec<Vec<usize>>,
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
/// and presentation. Source bytes are retained once for cold profile rebuilds.
pub struct ModelAssets {
    assets: Assets,
    catalog: ModelCatalog,
    palettes: crate::render_settings::PaletteRegistrations,
    initial_registration: bool,
    image_transaction_active: bool,
}

#[derive(Clone, Copy)]
pub struct RendererAdmission<'a> {
    pub resource: Option<ShaderResource>,
    pub registrations: &'a [ShaderRegistration],
    owner: &'a ModelAssets,
    profile: ModelProfile,
}

#[derive(Clone, Copy)]
pub struct ModelView<'a> {
    pub assets: &'a Assets,
    pub catalog: &'a ModelCatalog,
}
impl<'a> RendererAdmission<'a> {
    pub(crate) fn selected_presentation(
        self,
        selected: Option<WorldPresentationId>,
    ) -> Result<Self, String> {
        if let (Some(ShaderResource::World(id)), Some(selected)) = (self.resource, selected) {
            let world = self
                .owner
                .assets
                .world_presentation(WorldPresentationId(id))
                .ok_or("world shader membership")?
                .world();
            self.owner
                .world_admission(world, self.profile, Some(selected))
        } else {
            Ok(self)
        }
    }
    pub(crate) fn assets(self) -> &'a Assets {
        &self.owner.assets
    }
    pub(crate) fn budget(self) -> (usize, usize) {
        self.owner.shader_occurrence_budget()
    }
}
impl<'a> ModelView<'a> {
    pub fn world_registrations(
        self,
        world: WorldId,
        profile: ModelProfile,
        selected: Option<WorldPresentationId>,
    ) -> Option<&'a [ShaderRegistration]> {
        let presentation = self.world_presentation(world, Some(profile), selected)?;
        Some(
            self.assets
                .world_presentation(presentation.id)?
                .shader_registrations(),
        )
    }

    pub fn renderer_registrations(
        self,
        kind: RendererRegistration,
        handle: i32,
        profile: ModelProfile,
        selected: Option<WorldPresentationId>,
    ) -> Option<&'a [ShaderRegistration]> {
        if matches!(kind, RendererRegistration::Model)
            && let Some(inline) = self.catalog.renderer_inline(handle)
        {
            return self.world_registrations(inline.world, profile, selected);
        }
        self.catalog.renderer_registrations(kind, handle, profile)
    }

    pub fn world_presentation(
        self,
        world: WorldId,
        profile: Option<ModelProfile>,
        selected: Option<qa_render::world::WorldPresentationId>,
    ) -> Option<qa_render::material::world_load::LoadedWorldPresentation> {
        self.select_world_presentation(world, profile, selected, self.catalog.admitted_assets)
    }
    fn select_world_presentation(
        self,
        world: WorldId,
        profile: Option<ModelProfile>,
        selected: Option<WorldPresentationId>,
        admitted: Option<AssetCounts>,
    ) -> Option<qa_render::material::world_load::LoadedWorldPresentation> {
        let admitted = admitted?;
        if world.0 as usize >= admitted.worlds {
            return None;
        }
        let world = self.assets.world(world)?;
        let presentation = |id: qa_render::world::WorldPresentationId| {
            if id.0 as usize >= admitted.presentations {
                return None;
            }
            self.assets.world_presentation_registration(id)
        };
        if let Some(selected) = selected {
            return if world.presentations().contains(&selected) {
                presentation(selected)
            } else {
                None
            };
        }
        let profile = profile?;
        world
            .presentations()
            .iter()
            .filter_map(|&id| presentation(id))
            .find(|entry| {
                entry.profile.rules == profile.rules
                    && entry.profile.indexed_palette == profile.palette
                    && entry.profile.image_settings == profile.image_settings
                    && entry.profile.finish == profile.finish
                    && world
                        .render_settings()
                        .is_none_or(|settings| entry.lighting == settings.lighting)
            })
    }
}

struct RegistrationCheckpoint {
    assets: AssetCounts,
    admitted_assets: Option<AssetCounts>,
    palettes: [Option<PaletteId>; 5],
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
    files: Vec<ModelCheckpoint>,
    sources: Vec<ModelSourceFile>,
    shader_indices: Vec<(NameId, Option<u32>)>,
    shaders_before: Vec<ShaderCheckpoint>,
    skins_before: Vec<SkinCheckpoint>,
    skin_indices: Vec<(NameId, Option<u32>)>,
    inline_before: Vec<(usize, Option<NameId>)>,
    images: Vec<Option<ImageRegistrationMark>>,
}
struct ShaderCheckpoint {
    index: usize,
    profiles: usize,
    requests: [Option<ShaderRequest>; 5],
    bindings: Vec<usize>,
}
struct ModelCheckpoint {
    name: NameId,
    file: ModelFile,
    profiles: Vec<usize>,
    unattempted: Vec<usize>,
}
struct SkinCheckpoint {
    index: usize,
    profiles: usize,
    unattempted: Vec<usize>,
}
struct ProfileCheckpoint {
    registration: RegistrationCheckpoint,
    profiles: usize,
    models: Vec<usize>,
    shaders: Vec<usize>,
    skins: Vec<usize>,
}
impl ProfileCheckpoint {
    fn capture(owner: &ModelAssets) -> Result<Self, String> {
        Ok(Self {
            registration: RegistrationCheckpoint::base(owner)?,
            profiles: owner.catalog.profiles.len(),
            models: owner.catalog.models.iter().map(Vec::len).collect(),
            shaders: owner
                .catalog
                .shaders
                .iter()
                .map(|shader| shader.profiles.len())
                .collect(),
            skins: owner
                .catalog
                .skins
                .iter()
                .map(|skin| skin.profiles.len())
                .collect(),
        })
    }
    fn rollback(self, owner: &mut ModelAssets) -> Result<(), String> {
        owner.catalog.profiles.truncate(self.profiles);
        for (rows, length) in owner.catalog.models.iter_mut().zip(self.models) {
            rows.truncate(length);
        }
        for (shader, length) in owner.catalog.shaders.iter_mut().zip(self.shaders) {
            shader.profiles.truncate(length);
        }
        for (skin, length) in owner.catalog.skins.iter_mut().zip(self.skins) {
            skin.profiles.truncate(length);
        }
        self.registration
            .rollback(owner, RegistrationRollback::Retry)
    }
}
impl RegistrationCheckpoint {
    fn base(owner: &ModelAssets) -> Result<Self, String> {
        let catalog = &owner.catalog;
        let images = catalog
            .profiles
            .iter()
            .map(|profile| match &profile.state {
                None if profile.image_owner.is_some() => Ok(None),
                Some(Ok(state)) => Ok(Some(state.registration_mark())),
                Some(Err(_)) => Ok(None),
                None => Err("model image state already borrowed".to_owned()),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            assets: owner.assets.counts(),
            admitted_assets: catalog.admitted_assets,
            palettes: owner.palettes.checkpoint(),
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
            files: Vec::new(),
            sources: catalog.sources.clone(),
            shader_indices: Vec::new(),
            shaders_before: Vec::new(),
            skins_before: Vec::new(),
            skin_indices: Vec::new(),
            inline_before: Vec::new(),
            images,
        })
    }
    fn capture<const N: usize>(
        owner: &ModelAssets,
        requests: [&RendererRequest<impl AsRef<[u8]>>; N],
    ) -> Result<Self, String> {
        let mut checkpoint = Self::base(owner)?;
        for request in requests {
            checkpoint.capture_target(owner, request)?;
        }
        Ok(checkpoint)
    }

    fn capture_target(
        &mut self,
        owner: &ModelAssets,
        request: &RendererRequest<impl AsRef<[u8]>>,
    ) -> Result<(), String> {
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
                skin_index = find_skin_index(&catalog.names, &catalog.skin_indices, raw);
            }
            RendererRegistration::Shader { .. } => {
                shader_index = find_shader_index(&catalog.names, &catalog.shader_indices, raw);
            }
        }
        if let Some((group, index)) = shader_index
            && !self.shader_indices.iter().any(|&(saved, _)| saved == group)
        {
            self.shader_indices.push((group, index));
        }
        if let Some((_, Some(index))) = shader_index
            && !self
                .shaders_before
                .iter()
                .any(|saved| saved.index == index as usize)
        {
            let index = index as usize;
            let shader = &catalog.shaders[index];
            self.shaders_before.push(ShaderCheckpoint {
                index,
                profiles: shader.profiles.len(),
                requests: shader.requests.clone(),
                bindings: shader
                    .profiles
                    .iter()
                    .copied()
                    .filter(|&index| {
                        let binding = &catalog.shader_bindings[index];
                        matches!(binding.state, RegistrationState::Unattempted)
                    })
                    .collect(),
            });
        }
        if let Some((group, index)) = skin_index
            && !self.skin_indices.iter().any(|&(saved, _)| saved == group)
        {
            self.skin_indices.push((group, index));
        }
        if let Some((_, Some(index))) = skin_index
            && !self
                .skins_before
                .iter()
                .any(|saved| saved.index == index as usize)
        {
            let index = index as usize;
            self.skins_before.push(SkinCheckpoint {
                index,
                profiles: catalog.skins[index].profiles.len(),
                unattempted: catalog.skins[index]
                    .profiles
                    .iter()
                    .enumerate()
                    .filter_map(|(index, profile)| {
                        matches!(profile.state, RegistrationState::Unattempted).then_some(index)
                    })
                    .collect(),
            });
        }
        if let Some(name) = file_name
            && !self.files.iter().any(|saved| saved.name == name)
        {
            let profiles = catalog.models[name.0 as usize].clone();
            self.files.push(ModelCheckpoint {
                name,
                file: catalog.files[name.0 as usize].clone(),
                unattempted: profiles
                    .iter()
                    .copied()
                    .filter(|&index| {
                        matches!(
                            catalog.registrations[index].state,
                            RegistrationState::Unattempted
                        )
                    })
                    .collect(),
                profiles,
            });
        }
        if let Some((slot, handle)) = inline
            && !self.inline_before.iter().any(|&(saved, _)| saved == slot)
        {
            self.inline_before.push((slot, handle));
        }
        Ok(())
    }

    fn capture_all(&mut self, owner: &ModelAssets) -> Result<(), String> {
        let catalog = &owner.catalog;
        for (index, file) in catalog.files.iter().enumerate() {
            if !matches!(file, ModelFile::Empty) || !catalog.models[index].is_empty() {
                self.capture_target(
                    owner,
                    &RendererRequest {
                        name: catalog
                            .names
                            .get(NameId(index as u32))
                            .ok_or("model name")?,
                        source: RuleSetId::Quake3,
                        kind: RendererRegistration::Model,
                        world: None,
                    },
                )?;
            }
        }
        for shader in &catalog.shaders {
            self.capture_target(
                owner,
                &RendererRequest {
                    name: catalog.names.get(shader.name).ok_or("shader name")?,
                    source: RuleSetId::Quake3,
                    kind: RendererRegistration::Shader {
                        mipmaps: true,
                        lighting: ShaderLighting::TwoDimensional,
                    },
                    world: None,
                },
            )?;
        }
        for (index, skin) in catalog.skin_indices.iter().enumerate() {
            if skin.is_some() {
                self.capture_target(
                    owner,
                    &RendererRequest {
                        name: catalog.names.get(NameId(index as u32)).ok_or("skin name")?,
                        source: RuleSetId::Quake3,
                        kind: RendererRegistration::Skin,
                        world: None,
                    },
                )?;
            }
        }
        // Existing unbound name groups can acquire a new cached head inside
        // the transaction, so save those numeric slots as well.
        self.shader_indices = catalog
            .shader_indices
            .iter()
            .copied()
            .enumerate()
            .map(|(index, value)| (NameId(index as u32), value))
            .collect();
        self.skin_indices = catalog
            .skin_indices
            .iter()
            .copied()
            .enumerate()
            .map(|(index, value)| (NameId(index as u32), value))
            .collect();
        self.inline_before = catalog
            .inline
            .iter()
            .enumerate()
            .map(|(index, entry)| (index, entry.handle))
            .collect();
        Ok(())
    }

    fn rollback(self, owner: &mut ModelAssets, mode: RegistrationRollback) -> Result<(), String> {
        owner.assets.discard_registration_suffix(self.assets)?;
        owner.palettes.restore(self.palettes);
        let catalog = &mut owner.catalog;
        if matches!(mode, RegistrationRollback::Retry) {
            catalog.sources = self.sources;
            catalog
                .sources
                .resize(catalog.names.len(), ModelSourceFile::Empty);
        } else {
            for (source, previous) in catalog.sources.iter_mut().zip(self.sources) {
                if !matches!(previous, ModelSourceFile::Empty) {
                    *source = previous;
                }
            }
            for source in &mut catalog.sources {
                source.rollback_resources(self.assets, self.tags);
            }
        }
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
                catalog.models[index].clear();
                catalog.shader_indices[index] = None;
                catalog.skin_indices[index] = None;
            }
            for (group, index) in self.shader_indices {
                catalog.shader_indices[group.0 as usize] = index;
            }
            for (group, index) in self.skin_indices {
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
            for shader in &mut catalog.shaders[self.shaders..] {
                shader.requests = std::array::from_fn(|_| None);
            }
            for skin in &mut catalog.skins[self.skins..] {
                for profile in &mut skin.profiles {
                    profile.state.rollback(mode);
                }
            }
        }
        for file in &self.files {
            for &index in &file.unattempted {
                catalog.registrations[index].state.rollback(mode);
            }
        }
        for shader in self.shaders_before {
            catalog.shaders[shader.index].requests = shader.requests;
            if matches!(mode, RegistrationRollback::Retry) {
                catalog.shaders[shader.index]
                    .profiles
                    .truncate(shader.profiles);
            }
            for index in shader.bindings {
                catalog.shader_bindings[index].state.rollback(mode);
            }
        }
        for skin in self.skins_before {
            if matches!(mode, RegistrationRollback::Retry) {
                catalog.skins[skin.index].profiles.truncate(skin.profiles);
            }
            for index in skin.unattempted {
                catalog.skins[skin.index].profiles[index]
                    .state
                    .rollback(mode);
            }
        }
        if matches!(mode, RegistrationRollback::Source) {
            for file in &self.files {
                if matches!(catalog.files[file.name.0 as usize], ModelFile::Failed(_))
                    && matches!(file.file, ModelFile::Ready { renderer_sources, .. } if renderer_sources != 0)
                {
                    catalog.files[file.name.0 as usize] = file.file.clone();
                }
            }
            for index in self
                .files
                .iter()
                .map(|file| file.name.0 as usize)
                .chain(self.names..catalog.names.len())
            {
                let sources = catalog.models[index]
                    .iter()
                    .copied()
                    .fold(0, |sources, entry| {
                        let profile = &catalog.registrations[entry];
                        sources
                            | if profile.state.ready().is_some() {
                                1 << profile.profile.source_rules as u8
                            } else {
                                0
                            }
                    });
                if let ModelFile::Ready {
                    renderer_sources, ..
                } = &mut catalog.files[index]
                {
                    *renderer_sources = sources;
                }
            }
        } else {
            for file in self.files {
                catalog.files[file.name.0 as usize] = file.file;
                catalog.models[file.name.0 as usize] = file.profiles;
            }
        }
        for (slot, handle) in self.inline_before {
            catalog.inline[slot].handle = handle;
        }
        catalog.admitted_assets = self.admitted_assets;
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
    pub fn set_image_search_order(&mut self, order: ImageSearchOrder) {
        self.assets.set_image_search_order(order);
    }

    /// The startup cache moves into the same owner as its palette resources.
    pub fn adopt_palettes(
        &mut self,
        palettes: crate::render_settings::PaletteRegistrations,
    ) -> Result<(), String> {
        if !self.initial_registration
            || self.catalog.admitted_assets.is_some()
            || !self.palettes.is_empty()
            || !palettes.validate(&self.assets)
        {
            return Err(
                "palette owner adoption requires unpublished registered palettes".to_owned(),
            );
        }
        self.palettes = palettes;
        Ok(())
    }
    /// Startup code can split the unpublished asset and palette owners. Once
    /// published, new palettes require prepare_profile or prepare_world.
    pub fn unpublished_assets(
        &mut self,
    ) -> Result<
        (
            &mut Assets,
            &mut crate::render_settings::PaletteRegistrations,
        ),
        String,
    > {
        if self.catalog.admitted_assets.is_some() {
            return Err("assets already published".to_owned());
        }
        Ok((&mut self.assets, &mut self.palettes))
    }
    pub fn palettes(&self) -> &crate::render_settings::PaletteRegistrations {
        &self.palettes
    }
    /// Pending palette suffixes cannot become presentation bindings before
    /// the backend has admitted their resource prefix.
    pub fn published_palette(&self, source: RuleSetId) -> Option<PaletteId> {
        let id = self.palettes.get(source)?;
        let admitted = self.catalog.admitted_assets?;
        ((id.0 as usize) < admitted.palettes && self.assets.palette(id).is_some()).then_some(id)
    }

    pub(crate) fn graphics_profile(
        &mut self,
        vfs: &Vfs,
        rules: RuleSetId,
        settings: ImageSettings,
        finish: MaterialFinishPolicy,
    ) -> Result<ModelProfile, String> {
        let palette = match crate::render_settings::palette_resource(
            vfs,
            &mut self.assets,
            rules,
            &mut self.palettes,
        ) {
            Ok(palette) => palette,
            Err(ResourceError::Missing(_)) => None,
            Err(error) => return Err(format!("graphic palette: {error:?}")),
        };
        let profile = ModelProfile {
            source_rules: rules,
            rules,
            source_palette: palette,
            palette,
            image_settings: settings,
            finish,
        };
        if !self
            .catalog
            .profiles
            .iter()
            .any(|entry| entry.profile == profile)
        {
            if let Some(blend) = self.append_profile(profile)? {
                self.assets.install_palette_translucency(blend);
            }
        }
        Ok(profile)
    }
    pub(crate) fn graphic_shader(
        &mut self,
        vfs: &Vfs,
        profile: ModelProfile,
        name: &str,
        ui_fullscreen: bool,
    ) -> Result<ShaderRegistration, String> {
        self.admit_shader_profile(
            vfs,
            name.as_bytes(),
            profile.source_rules,
            true,
            ShaderLighting::TwoDimensional,
            self.image_transaction_active.then_some(profile),
            ui_fullscreen,
        )
        .map_err(|error| format!("graphic shader: {error:?}"))?;
        self.catalog
            .shader_outcome_candidate_variant(
                name.as_bytes(),
                profile,
                ShaderLighting::TwoDimensional,
                RegistrationVariant::Guard(ui_fullscreen),
            )
            .map(|(registration, _)| *registration)
            .ok_or_else(|| "graphic shader unavailable".into())
    }
    pub(crate) fn graphic_images<T>(
        &mut self,
        vfs: &Vfs,
        profile: ModelProfile,
        compile: impl FnOnce(&mut Images<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let index = self
            .catalog
            .profiles
            .iter()
            .position(|entry| entry.profile == profile)
            .ok_or("graphic profile unavailable")?;
        with_image_owner(&mut self.catalog.profiles, index, |images| {
            images.with_images(vfs, &mut self.assets, compile)
        })
    }
    pub(crate) fn graphic_default(&self, profile: ModelProfile) -> Option<ShaderBindingId> {
        self.catalog
            .profiles
            .iter()
            .find(|entry| entry.profile == profile)?
            .native_default
            .map(|(id, _)| id)
    }
    fn append_profile(
        &mut self,
        profile: ModelProfile,
    ) -> Result<Option<qa_render::assets::PreparedPaletteTranslucency>, String> {
        let count = self
            .catalog
            .profiles
            .len()
            .checked_add(1)
            .filter(|&count| count <= self.catalog.limits.models)
            .ok_or("model profile capacity")?;
        let maximum = self
            .catalog
            .limits
            .models
            .checked_mul(count)
            .and_then(|count| count.checked_mul(2))
            .filter(|&count| count <= u32::MAX as usize)
            .ok_or("model profile row capacity")?;
        let models = self
            .catalog
            .models
            .iter()
            .zip(&self.catalog.files)
            .filter(|(rows, file)| !rows.is_empty() || matches!(file, ModelFile::Ready { .. }))
            .count();
        self.catalog
            .registrations
            .len()
            .checked_add(models)
            .filter(|&count| count <= maximum)
            .ok_or("model registration capacity")?;
        let shader_maximum = maximum
            .checked_mul(
                self.assets
                    .counts()
                    .presentations
                    .checked_add(2)
                    .ok_or("shader lighting capacity")?,
            )
            .filter(|&count| count <= u32::MAX as usize)
            .ok_or("shader lighting capacity")?;
        self.catalog
            .shader_bindings
            .len()
            .checked_add(self.catalog.shaders.len())
            .filter(|&count| count <= shader_maximum)
            .ok_or("shader registration capacity")?;
        let (images, blend) =
            ProfileImages::new(profile, &self.catalog.profiles, &mut self.assets)?;
        if let Some(Err(error)) = &images.state {
            return Err(error.clone());
        }
        for (name, (rows, file)) in self
            .catalog
            .models
            .iter_mut()
            .zip(&self.catalog.files)
            .enumerate()
        {
            if rows.is_empty() && !matches!(file, ModelFile::Ready { .. }) {
                continue;
            }
            rows.push(self.catalog.registrations.len());
            self.catalog.registrations.push(RegisteredProfile {
                file: NameId(name as u32),
                profile,
                vertex_collapse: profile.finish.compile(false).collapse_vertex(),
                state: RegistrationState::Unattempted,
            });
        }
        for shader in &mut self.catalog.shaders {
            shader.profiles.push(self.catalog.shader_bindings.len());
            self.catalog.shader_bindings.push(ShaderProfileBinding {
                profile,
                vertex_collapse: profile.finish.compile(false).collapse_vertex(),
                lighting: ShaderLighting::TwoDimensional,
                state: RegistrationState::Unattempted,
            });
        }
        for skin in &mut self.catalog.skins {
            skin.profiles.push(SkinProfile {
                profile,
                vertex_collapse: profile.finish.compile(false).collapse_vertex(),
                state: RegistrationState::Unattempted,
            });
        }
        self.catalog.profiles.push(images);
        self.catalog.capacity.registrations = self.catalog.registrations.len();
        Ok(blend)
    }

    /// Admit one exact source/presentation policy and its native palettes as a
    /// complete cold transaction, independently of any later world load.
    pub fn prepare_profile(
        &mut self,
        vfs: &Vfs,
        source_rules: RuleSetId,
        presentation_rules: RuleSetId,
        image_settings: ImageSettings,
        finish: MaterialFinishPolicy,
        prepare: impl FnOnce(&Assets) -> Result<(), String>,
    ) -> Result<(ModelProfile, AssetCounts), String> {
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("profile registration requires a published resource prefix".to_owned());
        }
        let checkpoint = ProfileCheckpoint::capture(self)?;
        let result = (|| {
            let source_palette = crate::render_settings::palette(
                vfs,
                &mut self.assets,
                source_rules,
                &mut self.palettes,
            )?;
            let palette = crate::render_settings::palette(
                vfs,
                &mut self.assets,
                presentation_rules,
                &mut self.palettes,
            )?;
            let profile = ModelProfile {
                source_rules,
                rules: presentation_rules,
                source_palette,
                palette,
                image_settings,
                finish,
            };
            self.prepare_profile_value(profile, prepare)
        })();
        match result {
            Ok(ready) => Ok(ready),
            Err(error) => {
                checkpoint
                    .rollback(self)
                    .map_err(|rollback| format!("{error}; rollback: {rollback}"))?;
                Err(error)
            }
        }
    }

    fn prepare_profile_value(
        &mut self,
        profile: ModelProfile,
        prepare: impl FnOnce(&Assets) -> Result<(), String>,
    ) -> Result<(ModelProfile, AssetCounts), String> {
        if self
            .catalog
            .profiles
            .iter()
            .any(|entry| entry.profile == profile)
        {
            return Ok((profile, self.assets.counts()));
        }
        let blend = self.append_profile(profile)?;
        let counts = self.assets.counts();
        self.validate_admission(counts)
            .map_err(|error| format!("profile publication: {error:?}"))?;
        prepare(&self.assets)?;
        if let Some(blend) = blend {
            self.assets.install_palette_translucency(blend);
        }
        self.apply_admission(counts);
        Ok((profile, counts))
    }

    /// Keep a complete cold image refresh unpublished to live callers until
    /// its backend and frontend candidates have both committed.
    pub(crate) fn image_transaction<T>(
        &mut self,
        world_settings: Option<qa_render::world::WorldRenderSettings>,
        operation: impl FnOnce(&mut Self, &[WorldId]) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.image_transaction_active {
            return Err("image refresh is already active".to_owned());
        }
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("image refresh requires a published resource prefix".to_owned());
        }
        let mut checkpoint = ProfileCheckpoint::capture(self)?;
        checkpoint.registration.capture_all(self)?;
        let mut world_rebuild = None;
        self.image_transaction_active = true;
        let result = (|| {
            if let Some(settings) = world_settings {
                let worlds: Vec<_> = (0..self.assets.worlds().len())
                    .map(|index| WorldId(index as u32))
                    .collect();
                world_rebuild = Some(
                    self.assets
                        .rebuild_worlds(&worlds, settings)
                        .map_err(|error| format!("world render settings: {error}"))?,
                );
            }
            let changed: Vec<WorldId> = world_rebuild
                .as_ref()
                .map(|receipt| receipt.worlds().collect())
                .unwrap_or_default();
            operation(self, &changed)
        })();
        self.image_transaction_active = false;
        match result {
            Ok(ready) => {
                if let Some(receipt) = world_rebuild {
                    receipt.commit();
                }
                Ok(ready)
            }
            Err(mut error) => {
                let world_rollback = world_rebuild.map_or(Ok(()), |receipt| {
                    receipt.rollback(&mut self.assets).map_err(str::to_owned)
                });
                // Existing WorldIds and presentation ranges must be restored
                // before the appended resource suffix is discarded.
                let resource_rollback = checkpoint.rollback(self);
                for rollback in [world_rollback, resource_rollback] {
                    if let Err(rollback) = rollback {
                        error = format!("{error}; rollback: {rollback}");
                    }
                }
                Err(error)
            }
        }
    }

    /// Rebuild only the exact active image policies. Geometry, source names and
    /// prior registration identities remain available throughout staging.
    pub(crate) fn refresh_images(
        &mut self,
        vfs: &Vfs,
        profiles: &[ModelProfile],
        presentations: &[(WorldId, WorldPresentationId)],
        settings: [Option<ImageSettings>; 5],
        finish: MaterialFinishPolicy,
        ui_fullscreen: bool,
        mut prepare: impl FnMut(&Assets) -> Result<(), String>,
    ) -> Result<ImageRefresh, String> {
        if !self.image_transaction_active {
            return Err("image refresh requires its cold transaction".to_owned());
        }
        let mut refresh = ImageRefresh::default();
        for &old in profiles {
            if refresh.profiles.iter().any(|&(from, _)| from == old) {
                continue;
            }
            if !self
                .catalog
                .profiles
                .iter()
                .any(|entry| entry.profile == old)
            {
                return Err("image refresh profile unavailable".to_owned());
            }
            let image_settings = settings[old.rules as usize].unwrap_or(old.image_settings);
            let new = ModelProfile {
                image_settings,
                finish,
                ..old
            };
            self.prepare_profile_value(new, &mut prepare)?;
            refresh.profiles.push((old, new));
        }
        for &(world, old) in presentations {
            if refresh
                .worlds
                .iter()
                .any(|&(owner, from, _)| owner == world && from == old)
            {
                continue;
            }
            let previous = self
                .assets
                .world_presentation_registration(old)
                .filter(|_| {
                    self.assets
                        .world(world)
                        .is_some_and(|entry| entry.presentations().contains(&old))
                })
                .ok_or("image refresh world presentation")?;
            let image_settings = settings[previous.profile.rules as usize]
                .unwrap_or(previous.profile.image_settings);
            let profile = profiles
                .iter()
                .find_map(|&from| {
                    (from.rules == previous.profile.rules
                        && from.palette == previous.profile.indexed_palette
                        && from.image_settings == previous.profile.image_settings
                        && from.finish == previous.profile.finish)
                        .then(|| refresh.profile(from))
                })
                .ok_or("image refresh world profile")?;
            let policy = qa_render::material::world_load::WorldPresentationProfile {
                image_settings,
                finish,
                ..previous.profile
            };
            let (loaded, (), _) = self.prepare_world_presentation(
                vfs,
                world,
                profile,
                policy,
                ui_fullscreen,
                |_, _| Ok(()),
                |assets, ()| prepare(assets),
            )?;
            let selected = *loaded
                .presentations
                .first()
                .ok_or("image refresh world result")?;
            refresh.worlds.push((world, old, selected));
        }
        let mut models = Vec::new();
        for (name, rows) in self.catalog.models.iter().enumerate() {
            for &index in rows {
                let entry = &self.catalog.registrations[index];
                let profile = refresh.profile(entry.profile);
                if profile != entry.profile && entry.state.ready().is_some() {
                    let path = self
                        .catalog
                        .names
                        .get(NameId(name as u32))
                        .ok_or("image refresh model name")?
                        .to_vec();
                    models.push((path, profile));
                }
            }
        }
        for (name, profile) in models {
            let request = RendererRequest {
                name,
                source: profile.source_rules,
                kind: RendererRegistration::Model,
                world: None,
            };
            self.prepare_registration(
                vfs,
                &request,
                profile,
                RegistrationVariant::Guard(ui_fullscreen),
                |_| Ok(()),
                |assets, ()| prepare(assets),
            )?;
        }
        let mut skins = Vec::new();
        for (group, index) in self.catalog.skin_indices.iter().enumerate() {
            let Some(index) = *index else { continue };
            for entry in &self.catalog.skins[index as usize].profiles {
                let profile = refresh.profile(entry.profile);
                if profile != entry.profile && entry.state.ready().is_some() {
                    let name = self
                        .catalog
                        .names
                        .get(NameId(group as u32))
                        .ok_or("image refresh skin name")?
                        .to_vec();
                    skins.push((name, profile));
                }
            }
        }
        for (name, profile) in skins {
            let request = RendererRequest {
                name,
                source: profile.source_rules,
                kind: RendererRegistration::Skin,
                world: None,
            };
            self.prepare_registration(
                vfs,
                &request,
                profile,
                RegistrationVariant::Guard(ui_fullscreen),
                |_| Ok(()),
                |assets, ()| prepare(assets),
            )?;
        }
        Ok(refresh)
    }

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
            let (entry, blend) = ProfileImages::new(profile, &unique, &mut assets)?;
            if let Some(blend) = blend {
                assets.install_palette_translucency(blend);
            }
            unique.push(entry);
        }
        let (worlds, inline) = inline_bindings(&assets, worlds, limits)?;
        let capacity = ModelCapacity {
            names: 1,
            inline_bindings: inline.len(),
            ..ModelCapacity::default()
        };
        Ok(Self {
            assets,
            palettes: Default::default(),
            initial_registration: true,
            image_transaction_active: false,
            catalog: ModelCatalog {
                reader: ArchiveReader::default(),
                names,
                files: vec![ModelFile::Empty],
                sources: vec![ModelSourceFile::Empty],
                models: vec![Vec::new()],
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
                    finish: entry.profile.finish,
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

    fn registration_candidates<const N: usize>(
        &self,
        requests: [&RendererRequest<impl AsRef<[u8]>>; N],
        handles: [i32; N],
        profile: ModelProfile,
        variant: RegistrationVariant,
    ) -> Result<[RendererAdmission<'_>; N], String> {
        let mut candidates = [RendererAdmission {
            resource: None,
            registrations: &[],
            owner: self,
            profile,
        }; N];
        for ((candidate, request), handle) in candidates.iter_mut().zip(requests).zip(handles) {
            if handle <= 0 && matches!(request.kind, RendererRegistration::Model) {
                continue;
            }
            if handle <= 0
                && let RendererRegistration::Shader { lighting, .. } = request.kind
                && self
                    .catalog
                    .shader_outcome_candidate_variant(
                        request.name.as_ref(),
                        profile,
                        lighting,
                        variant,
                    )
                    .is_none()
            {
                continue;
            }
            *candidate = match request.kind {
                RendererRegistration::Model => {
                    if let Some(inline) = self.catalog.renderer_inline(handle) {
                        self.world_admission(inline.world, profile, None)?
                    } else {
                        let id = self
                            .catalog
                            .renderer_model_candidate_variant(handle, profile, variant)
                            .ok_or("model shader membership")?;
                        let (ready, _) = self.catalog.registrations[id.0 as usize]
                            .state
                            .ready()
                            .ok_or("model shader membership")?;
                        RendererAdmission {
                            resource: Some(ShaderResource::Model(
                                self.catalog.registrations[id.0 as usize].file,
                            )),
                            registrations: &ready.registrations,
                            owner: self,
                            profile,
                        }
                    }
                }
                RendererRegistration::Skin => {
                    let index = if handle > 0 {
                        usize::try_from(handle - 1).map_err(|_| "skin shader membership")?
                    } else {
                        let Some((_, Some(index))) = find_skin_index(
                            &self.catalog.names,
                            &self.catalog.skin_indices,
                            request.name.as_ref(),
                        ) else {
                            continue;
                        };
                        index as usize
                    };
                    let row = self
                        .catalog
                        .skins
                        .get(index)
                        .ok_or("skin shader membership")?
                        .profiles
                        .iter()
                        .find(|row| {
                            row.profile == profile
                                && variant.matches(row.profile, row.vertex_collapse)
                                && !matches!(row.state, RegistrationState::Unattempted)
                        })
                        .ok_or("skin shader membership")?;
                    let registrations = match &row.state {
                        RegistrationState::Ready { value, .. } => value.registrations.as_ref(),
                        RegistrationState::Failed(_) => &[],
                        RegistrationState::Unattempted => continue,
                    };
                    RendererAdmission {
                        resource: Some(ShaderResource::Skin(
                            u32::try_from(index)
                                .ok()
                                .and_then(|index| index.checked_add(1))
                                .ok_or("skin shader membership")?,
                        )),
                        registrations,
                        owner: self,
                        profile,
                    }
                }
                RendererRegistration::Shader { lighting, .. } => {
                    let (registration, _) = self
                        .catalog
                        .shader_outcome_candidate_variant(
                            request.name.as_ref(),
                            profile,
                            lighting,
                            variant,
                        )
                        .ok_or("shader membership")?;
                    RendererAdmission {
                        resource: None,
                        registrations: std::slice::from_ref(registration),
                        owner: self,
                        profile,
                    }
                }
            };
        }
        Ok(candidates)
    }
    pub(crate) fn failed_skin_admission(
        &self,
        name: &[u8],
        profile: ModelProfile,
        variant: RegistrationVariant,
    ) -> Option<RendererAdmission<'_>> {
        let (_, Some(index)) =
            find_skin_index(&self.catalog.names, &self.catalog.skin_indices, name)?
        else {
            return None;
        };
        let row = self
            .catalog
            .skins
            .get(index as usize)?
            .profiles
            .iter()
            .find(|row| {
                row.profile == profile && variant.matches(row.profile, row.vertex_collapse)
            })?;
        if !matches!(row.state, RegistrationState::Failed(_)) {
            return None;
        }
        let request = RendererRequest {
            name,
            source: profile.source_rules,
            kind: RendererRegistration::Skin,
            world: None,
        };
        let [candidate] = self
            .registration_candidates([&request], [0], profile, variant)
            .ok()?;
        Some(candidate)
    }
    pub(crate) fn renderer_presentation(
        &self,
        world: WorldId,
        profile: ModelProfile,
        selected: Option<WorldPresentationId>,
    ) -> Option<qa_render::material::world_load::LoadedWorldPresentation> {
        self.view().select_world_presentation(
            world,
            Some(profile),
            selected,
            if self.initial_registration {
                Some(self.assets.counts())
            } else {
                self.catalog.admitted_assets
            },
        )
    }
    pub(crate) fn world_admission(
        &self,
        world: WorldId,
        profile: ModelProfile,
        selected: Option<WorldPresentationId>,
    ) -> Result<RendererAdmission<'_>, String> {
        let id = self
            .renderer_presentation(world, profile, selected)
            .ok_or("world shader membership")?
            .id;
        let presentation = self
            .assets
            .world_presentation(id)
            .ok_or("world shader membership")?;
        Ok(RendererAdmission {
            resource: Some(ShaderResource::World(id.0)),
            registrations: presentation.shader_registrations(),
            owner: self,
            profile,
        })
    }
    pub(crate) fn profile_count(&self) -> usize {
        self.catalog.profiles.len()
    }
    pub(crate) fn shader_occurrence_budget(&self) -> (usize, usize) {
        let occurrences = self
            .assets
            .world_presentations()
            .iter()
            .map(|world| world.shader_registrations().len())
            .sum::<usize>()
            + self
                .catalog
                .registrations
                .iter()
                .filter_map(|row| row.state.ready())
                .map(|(ready, _)| ready.registrations.len())
                .sum::<usize>()
            + self
                .catalog
                .skins
                .iter()
                .flat_map(|skin| &skin.profiles)
                .filter_map(|row| row.state.ready())
                .map(|(ready, _)| ready.registrations.len())
                .sum::<usize>();
        let resources = self.assets.world_presentations().len()
            + self.catalog.registrations.len()
            + self
                .catalog
                .skins
                .iter()
                .map(|skin| skin.profiles.len())
                .sum::<usize>();
        (occurrences, resources)
    }

    /// Register one late native request and publish it only after the backend
    /// accepts the complete appended resource prefix.
    pub fn prepare_registration<T>(
        &mut self,
        vfs: &Vfs,
        request: &RendererRequest<impl AsRef<[u8]>>,
        profile: ModelProfile,
        variant: RegistrationVariant,
        stage: impl FnOnce(RendererAdmission<'_>) -> Result<T, String>,
        prepare: impl FnOnce(&Assets, &T) -> Result<(), String>,
    ) -> Result<(i32, T, AssetCounts), String> {
        self.prepare_registrations(
            vfs,
            [request],
            profile,
            variant,
            |[candidate]| stage(candidate),
            prepare,
        )
        .map(|([handle], staged, counts)| (handle, staged, counts))
    }

    fn prepare_registrations<T, const N: usize>(
        &mut self,
        vfs: &Vfs,
        requests: [&RendererRequest<impl AsRef<[u8]>>; N],
        profile: ModelProfile,
        variant: RegistrationVariant,
        stage: impl FnOnce([RendererAdmission<'_>; N]) -> Result<T, String>,
        prepare: impl FnOnce(&Assets, &T) -> Result<(), String>,
    ) -> Result<([i32; N], T, AssetCounts), String> {
        let initial = self.initial_registration;
        if !initial && self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("late registration requires a published resource prefix".to_owned());
        }
        if !self
            .catalog
            .profiles
            .iter()
            .any(|entry| entry.profile == profile)
        {
            return Err("renderer registration source/profile mismatch".to_owned());
        }
        for request in requests {
            if request.name.as_ref().is_empty() {
                return Err("renderer registration name length".to_owned());
            }
            if profile.source_rules != request.source {
                return Err("renderer registration source/profile mismatch".to_owned());
            }
            if let RendererRegistration::Shader {
                lighting: ShaderLighting::World(Some(presentation)),
                ..
            } = request.kind
            {
                let world = request.world.ok_or("shader registration world")?;
                self.renderer_presentation(world, profile, Some(presentation))
                    .filter(|entry| {
                        entry.profile.rules == profile.rules
                            && entry.profile.indexed_palette == profile.palette
                            && entry.profile.image_settings == profile.image_settings
                            && entry.profile.finish == profile.finish
                    })
                    .ok_or("shader registration presentation")?;
            }
        }
        let cached = requests.map(|request| {
            self.catalog.registration_cached_variant(
                request.kind,
                request.name.as_ref(),
                profile,
                request.world,
                variant,
            )
        });
        let mut handles = [0; N];
        for (handle, cached) in handles.iter_mut().zip(cached) {
            if let Some(cached) = cached {
                *handle = cached;
            }
        }
        if cached.iter().all(Option::is_some) {
            let candidates = self.registration_candidates(requests, handles, profile, variant)?;
            let staged = stage(candidates)?;
            return Ok((handles, staged, self.assets.counts()));
        }
        if requests.iter().zip(cached).any(|(request, cached)| {
            cached == Some(0)
                && match request.kind {
                    RendererRegistration::Shader { lighting, .. } => self
                        .catalog
                        .shader_outcome(request.name.as_ref(), profile, lighting)
                        .is_none(),
                    _ => true,
                }
        }) {
            return Err("renderer registration unavailable for invoking profile".to_owned());
        }
        let ui_fullscreen = variant.guard()?;
        let checkpoint = RegistrationCheckpoint::capture(self, requests)?;
        let mut rollback = RegistrationRollback::Source;
        let result: Result<([i32; N], T, AssetCounts), String> = (|| {
            for (index, request) in requests.into_iter().enumerate() {
                if cached[index].is_some() {
                    continue;
                }
                let raw = request.name.as_ref();
                let selected = self.image_transaction_active.then_some(profile);
                let handle = match request.kind {
                    RendererRegistration::Model if raw.starts_with(b"*") => request
                        .world
                        .zip(qa_core::text::inline_model_index(raw))
                        .ok_or(CallError::File)
                        .and_then(|(world, index)| self.register_inline(world, index)),
                    RendererRegistration::Model => {
                        let path =
                            std::str::from_utf8(raw).map_err(|_| "model name is not UTF-8")?;
                        self.ensure_file_profile(
                            vfs,
                            &canonical_path(path),
                            selected,
                            ui_fullscreen,
                        )
                        .map_err(|error| error.kind)
                        .and_then(|_| {
                            self.catalog
                                .names
                                .find_path_bytes(raw)
                                .and_then(|name| i32::try_from(name.0).ok())
                                .ok_or(CallError::File)
                        })
                    }
                    RendererRegistration::Skin => {
                        self.admit_skin_profile(vfs, raw, request.source, selected, ui_fullscreen)
                    }
                    RendererRegistration::Shader { mipmaps, lighting } => self
                        .admit_shader_profile(
                            vfs,
                            raw,
                            request.source,
                            mipmaps,
                            lighting,
                            selected,
                            ui_fullscreen,
                        ),
                }
                .map_err(|error| format!("renderer registration: {error:?}"))?;
                let candidate = match request.kind {
                    RendererRegistration::Shader { lighting, .. } => self
                        .catalog
                        .shader_outcome_candidate_variant(raw, profile, lighting, variant)
                        .map(|(registration, _)| {
                            if registration.defaulted.is_some() {
                                0
                            } else {
                                handle
                            }
                        }),
                    _ => self
                        .catalog
                        .renderer_candidate_variant(request.kind, handle, profile, variant)
                        .then_some(handle),
                };
                handles[index] =
                    candidate.ok_or("renderer registration unavailable for invoking profile")?;
            }
            let counts = self.assets.counts();
            rollback = RegistrationRollback::Retry;
            if !initial {
                self.validate_admission(counts)
                    .map_err(|error| format!("renderer publication: {error:?}"))?;
            }
            let candidates = self.registration_candidates(requests, handles, profile, variant)?;
            let staged = stage(candidates)?;
            if !initial {
                prepare(&self.assets, &staged)?;
            }
            // The backend has committed this prefix. All owner checks already
            // passed, so publication cannot fail or discard accepted resources.
            if !initial {
                self.apply_admission(counts);
            }
            Ok((handles, staged, counts))
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

    /// Register one CLIENT HUD through the same palette/resource owner. The
    /// frontend owns the returned candidate only after backend admission.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_hud<T>(
        &mut self,
        vfs: &Vfs,
        rules: RuleSetId,
        settings: ImageSettings,
        compile_policy: ShaderCompilePolicy,
        width: u32,
        reuse_font: Option<std::sync::Arc<crate::renderer::console::FontAssets>>,
        stage: impl FnOnce(ModelView<'_>, crate::renderer::hud::HudAssets) -> Result<T, String>,
        prepare: impl FnOnce(&Assets, &T) -> Result<(), String>,
    ) -> Result<(T, AssetCounts), String> {
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("HUD registration requires a published resource prefix".to_owned());
        }
        let mut checkpoint = ProfileCheckpoint::capture(self)?;
        if let Some(request) = crate::renderer::console::FontAssets::registration_request(rules) {
            checkpoint.registration.capture_target(self, &request)?;
        }
        let result = (|| {
            let font = match reuse_font.filter(|font| {
                font.profile.source_rules == rules
                    && font.profile.rules == rules
                    && font.profile.image_settings == settings
                    && font.profile.finish == compile_policy.finish
                    && font.compile_policy.collapse_vertex() == compile_policy.collapse_vertex()
            }) {
                Some(font) => font,
                None => crate::renderer::console::FontAssets::load(
                    vfs,
                    self,
                    rules,
                    settings,
                    compile_policy,
                )?,
            };
            let candidate = crate::renderer::hud::HudAssets::load(vfs, self, rules, font, width)?;
            let staged = stage(self.view(), candidate)?;
            let counts = self.assets.counts();
            self.validate_admission(counts)
                .map_err(|error| format!("HUD publication: {error:?}"))?;
            prepare(&self.assets, &staged)?;
            self.apply_admission(counts);
            Ok((staged, counts))
        })();
        match result {
            Ok(ready) => Ok(ready),
            Err(error) => {
                checkpoint
                    .rollback(self)
                    .map_err(|rollback| format!("{error}; rollback: {rollback}"))?;
                Err(error)
            }
        }
    }

    fn validate_world_profile(
        &self,
        profile: ModelProfile,
        presentation: qa_render::material::world_load::WorldPresentationProfile,
    ) -> Result<(), String> {
        if !self
            .catalog
            .profiles
            .iter()
            .any(|entry| entry.profile == profile)
            || presentation.rules != profile.rules
            || presentation.indexed_palette != profile.palette
            || presentation.image_settings != profile.image_settings
            || presentation.finish != profile.finish
        {
            return Err("world registration source/profile mismatch".to_owned());
        }
        Ok(())
    }

    /// Add one presentation to the same world without revisiting BSP/collision
    /// admission. Caller staging and backend preparation precede publication.
    pub fn prepare_world_presentation<T>(
        &mut self,
        vfs: &Vfs,
        world: WorldId,
        profile: ModelProfile,
        presentation: qa_render::material::world_load::WorldPresentationProfile,
        ui_fullscreen: bool,
        stage: impl FnOnce(
            ModelView<'_>,
            &qa_render::material::world_load::LoadedWorld,
        ) -> Result<T, String>,
        prepare: impl FnOnce(&Assets, &T) -> Result<(), String>,
    ) -> Result<(qa_render::material::world_load::LoadedWorld, T, AssetCounts), String> {
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("world presentation requires a published resource prefix".to_owned());
        }
        self.validate_world_profile(profile, presentation)?;
        let native_default = self.catalog.native_default(profile);
        let checkpoint = RegistrationCheckpoint::base(self)?;
        let result = (|| {
            let loaded = qa_render::material::world_load::append_world_presentation(
                vfs,
                &mut self.assets,
                world,
                WorldPresentationRequest {
                    profile: presentation,
                    native_default,
                },
                ui_fullscreen,
            )
            .map_err(|error| format!("world presentation: {error:?}"))?;
            let staged = stage(self.view(), &loaded)?;
            let counts = self.assets.counts();
            self.validate_admission(counts)
                .map_err(|error| format!("world publication: {error:?}"))?;
            prepare(&self.assets, &staged)?;
            self.apply_admission(counts);
            Ok((loaded, staged, counts))
        })();
        match result {
            Ok(ready) => Ok(ready),
            Err(error) => {
                checkpoint
                    .rollback(self, RegistrationRollback::Retry)
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
            presentation,
            options,
            ui_fullscreen,
            trace_rules,
        } = registration;
        self.validate_world_profile(profile, presentation)?;
        let native_default = self.catalog.native_default(profile);
        let checkpoint = RegistrationCheckpoint::capture(
            self,
            [&RendererRequest {
                name: input.virtual_path.as_bytes(),
                source: profile.source_rules,
                kind: RendererRegistration::Model,
                world: None,
            }],
        )?;
        let hulls = input.native_source.lineage() == qa_formats::entities::EntitySyntax::Quake;
        let mut created = None;
        let result = (|| {
            let source_palette = crate::render_settings::palette(
                vfs,
                &mut self.assets,
                input.native_source,
                &mut self.palettes,
            )?;
            let loaded = input.load(
                vfs,
                &mut self.assets,
                geometry,
                source_palette,
                &[WorldPresentationRequest {
                    profile: presentation,
                    native_default,
                }],
                options,
                ui_fullscreen,
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
            self.catalog.sources.push(ModelSourceFile::Empty);
            self.catalog.models.push(Vec::new());
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
            && !self.catalog.models[name.0 as usize]
                .iter()
                .copied()
                .any(|index| {
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
            .iter()
            .copied()
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
        self.ensure_file_profile(vfs, path, None, false)
    }

    fn model_source(&mut self, vfs: &Vfs, path: &str) -> Result<Option<NameId>, FileError> {
        if let Some(name) = self.catalog.names.find(path.as_bytes())
            && !matches!(
                self.catalog.sources[name.0 as usize],
                ModelSourceFile::Empty
            )
        {
            return Ok(Some(name));
        }
        if vfs.open(path.as_bytes()).is_none() {
            return Ok(None);
        }
        let name = self
            .admit_name(path.as_bytes())
            .map_err(|_| FileError::capacity("model source name capacity"))?;
        let bytes: Arc<[u8]> = vfs
            .read_file(
                path.as_bytes(),
                self.catalog.limits.file_bytes,
                &mut self.catalog.reader,
            )
            .map_err(|error| FileError {
                kind: CallError::File,
                message: error.to_string(),
            })?
            .into();
        let model = match Model::parse(&bytes) {
            Ok(model) => model,
            Err(error) => {
                self.catalog.sources[name.0 as usize] = ModelSourceFile::Invalid {
                    md3: bytes.starts_with(b"IDP3"),
                    error: FileError {
                        kind: CallError::File,
                        message: format!("decode: {error:?}"),
                    },
                };
                return Ok(Some(name));
            }
        };
        let metadata = ModelMetadata {
            format: model.format,
            bounds: model.bounds,
            renderer_bounds: match model.format {
                ModelFormat::Md3 | ModelFormat::Mdc => model.frames[0].bounds,
                _ => model.bounds,
            },
        };
        let radii = model
            .frames
            .iter()
            .map(|frame| frame.bounds.radius())
            .collect::<Arc<[_]>>();
        let tags = self.admit_tags(&model).map_err(FileError::tag)?;
        let meshes = vec![
            None;
            if model.format == ModelFormat::Sp2 {
                model.sprites.len()
            } else {
                model.meshes.len()
            }
        ];
        drop(model);
        self.catalog.sources[name.0 as usize] = ModelSourceFile::Ready {
            bytes,
            metadata,
            radii,
            tags,
            meshes,
        };
        Ok(Some(name))
    }

    fn model_lods(&mut self, vfs: &Vfs, path: &str) -> Result<ModelLods, FileError> {
        let mut lods = ModelLods {
            slots: [ModelLodSlot::Missing; 3],
            native_count: 0,
            md3: true,
        };
        let md3_path = path
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("md3"));
        if !md3_path {
            let name = self.model_source(vfs, path)?.ok_or_else(|| FileError {
                kind: CallError::File,
                message: format!("missing model {path}"),
            })?;
            match &self.catalog.sources[name.0 as usize] {
                ModelSourceFile::Ready { metadata, .. } if metadata.format == ModelFormat::Md3 => {}
                ModelSourceFile::Ready { .. } => {
                    return Ok(ModelLods {
                        slots: [
                            ModelLodSlot::Source(name),
                            ModelLodSlot::Missing,
                            ModelLodSlot::Missing,
                        ],
                        native_count: 1,
                        md3: false,
                    });
                }
                ModelSourceFile::Invalid { error, .. } => {
                    return Err(error.clone());
                }
                ModelSourceFile::Empty => {
                    return Err(FileError::capacity("model source unavailable"));
                }
            }
        }
        let stem = path.rsplit_once('.').map_or(path, |(stem, _)| stem);
        for slot in (0..3).rev() {
            let sibling;
            let filename = if slot == 0 {
                path
            } else {
                sibling = format!("{stem}_{slot}.md3");
                &sibling
            };
            let Some(name) = self.model_source(vfs, filename)? else {
                continue;
            };
            match &self.catalog.sources[name.0 as usize] {
                ModelSourceFile::Ready { metadata, .. } if metadata.format == ModelFormat::Md3 => {
                    lods.slots[slot] = ModelLodSlot::Source(name);
                    lods.native_count += 1;
                }
                ModelSourceFile::Invalid { error, md3: true } if slot != 0 => {
                    lods.slots[slot] = ModelLodSlot::Invalid;
                    if lods.native_count == 0 {
                        return Err(error.clone());
                    }
                    for lower in (0..slot).rev() {
                        lods.slots[lower] = ModelLodSlot::Alias((lower + 1) as u8);
                        lods.native_count += 1;
                    }
                    break;
                }
                ModelSourceFile::Invalid { error, .. } => return Err(error.clone()),
                _ => {
                    return Err(FileError {
                        kind: CallError::File,
                        message: format!("unknown MD3 file identifier: {filename}"),
                    });
                }
            }
        }
        if lods.native_count == 0 {
            return Err(FileError {
                kind: CallError::File,
                message: format!("missing model {path}"),
            });
        }
        Ok(lods)
    }

    fn ensure_file_profile(
        &mut self,
        vfs: &Vfs,
        path: &str,
        selected: Option<ModelProfile>,
        ui_fullscreen: bool,
    ) -> Result<Bounds, FileError> {
        let name = if let Some(name) = self.catalog.names.find(path.as_bytes()) {
            let file = &self.catalog.files[name.0 as usize];
            let unattempted = !self
                .catalog
                .profiles
                .iter()
                .filter(|images| selected.is_none_or(|profile| images.profile == profile))
                .all(|images| {
                    self.catalog.models[name.0 as usize].iter().any(|&row| {
                        let row = &self.catalog.registrations[row];
                        row.profile == images.profile
                            && row.vertex_collapse
                                == row.profile.finish.compile(ui_fullscreen).collapse_vertex()
                    })
                })
                || self.catalog.models[name.0 as usize]
                    .iter()
                    .copied()
                    .any(|index| {
                        matches!(
                            self.catalog.registrations[index].state,
                            RegistrationState::Unattempted
                        ) && self.catalog.registrations[index].vertex_collapse
                            == self.catalog.registrations[index]
                                .profile
                                .finish
                                .compile(ui_fullscreen)
                                .collapse_vertex()
                            && selected.is_none_or(|profile| {
                                self.catalog.registrations[index].profile == profile
                            })
                    });
            if !matches!(file, ModelFile::Empty)
                && !(matches!(file, ModelFile::Ready { .. }) && unattempted)
            {
                return file
                    .bounds(&self.catalog.sources)
                    .or_else(|| matches!(file, ModelFile::Ready { .. }).then(Bounds::default))
                    .ok_or_else(|| match file {
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
        let mut rows = self.catalog.models[name.0 as usize].clone();
        let fresh_profiles = rows.is_empty();
        for images in &self.catalog.profiles {
            if !rows.iter().any(|&row| {
                let row = &self.catalog.registrations[row];
                row.profile == images.profile
                    && row.vertex_collapse
                        == row.profile.finish.compile(ui_fullscreen).collapse_vertex()
            }) {
                if self.catalog.registrations.len()
                    >= self
                        .catalog
                        .limits
                        .models
                        .saturating_mul(self.catalog.profiles.len())
                        .saturating_mul(2)
                {
                    return Err(FileError::capacity("model registration capacity"));
                }
                rows.push(self.catalog.registrations.len());
                self.catalog.registrations.push(RegisteredProfile {
                    file: name,
                    profile: images.profile,
                    vertex_collapse: images
                        .profile
                        .finish
                        .compile(ui_fullscreen)
                        .collapse_vertex(),
                    state: RegistrationState::Unattempted,
                });
            }
        }
        self.catalog.models[name.0 as usize] = rows.clone();
        let lods = match self.catalog.files[name.0 as usize] {
            ModelFile::Ready { lods, .. } => lods,
            _ => match self.model_lods(vfs, path) {
                Ok(lods) => lods,
                Err(error) => return Err(self.fail_file(name, fresh_profiles, error)),
            },
        };
        let mut inputs = Vec::new();
        for slot in (0..3).rev() {
            let ModelLodSlot::Source(source) = lods.slots[slot] else {
                continue;
            };
            let ModelSourceFile::Ready { bytes, meshes, .. } =
                &self.catalog.sources[source.0 as usize]
            else {
                return Err(self.fail_file(
                    name,
                    fresh_profiles,
                    FileError::capacity("model source unavailable"),
                ));
            };
            let bytes = Arc::clone(bytes);
            let meshes = meshes.clone();
            let model = match Model::parse(&bytes) {
                Ok(model) => model,
                Err(error) => {
                    return Err(self.fail_file(
                        name,
                        fresh_profiles,
                        FileError {
                            kind: CallError::File,
                            message: format!("decode: {error:?}"),
                        },
                    ));
                }
            };
            let surfaces = match self.admit_surface_names(&model) {
                Ok(surfaces) => surfaces,
                Err(error) => return Err(self.fail_file(name, fresh_profiles, error)),
            };
            if model.tags_per_frame != 0
                && matches!(
                    self.catalog.sources[source.0 as usize],
                    ModelSourceFile::Ready { tags: None, .. }
                )
            {
                let restored = match self.admit_tags(&model) {
                    Ok(tags) => tags,
                    Err(message) => {
                        return Err(self.fail_file(name, fresh_profiles, FileError::tag(message)));
                    }
                };
                if let ModelSourceFile::Ready { tags, .. } =
                    &mut self.catalog.sources[source.0 as usize]
                {
                    *tags = restored;
                }
            }
            drop(model);
            inputs.push((slot, source, bytes, surfaces, meshes));
        }
        if lods.md3 {
            self.shader_catalog(vfs);
        }
        let mut renderer_sources = match &self.catalog.files[name.0 as usize] {
            ModelFile::Ready {
                renderer_sources, ..
            } => *renderer_sources,
            _ => 0,
        };
        for &row in &rows {
            let entry = &mut self.catalog.registrations[row];
            let profile_index = self
                .catalog
                .profiles
                .iter()
                .position(|images| images.profile == entry.profile)
                .ok_or_else(|| FileError::capacity("model profile"))?;
            if !matches!(entry.state, RegistrationState::Unattempted)
                || entry.vertex_collapse
                    != entry
                        .profile
                        .finish
                        .compile(ui_fullscreen)
                        .collapse_vertex()
                || selected.is_some_and(|profile| entry.profile != profile)
            {
                continue;
            }
            let result = with_image_owner(&mut self.catalog.profiles, profile_index, |images| {
                let registration = Registration {
                    path: path.to_owned(),
                    profile: images.profile,
                    required: false,
                };
                let model_policy = images.model_policy;
                let native_default = images
                    .native_default
                    .map_or(ShaderBindingId(0), |entry| entry.0);
                let compile_policy = images.profile.finish.compile(ui_fullscreen);
                images.with_images(vfs, &mut self.assets, |temporary| {
                    let mut compiled = Vec::new();
                    let mut registrations = Vec::new();
                    for (slot, _, bytes, _, meshes) in &mut inputs {
                        let model =
                            Model::parse(bytes).map_err(|error| format!("decode: {error:?}"))?;
                        validate_model(&model)?;
                        let mut model = register(
                            &model,
                            temporary,
                            &registration,
                            model_policy,
                            native_default,
                            meshes,
                            self.catalog.shader_catalog.as_ref(),
                            &mut self.catalog.capacity,
                            self.catalog.limits,
                            compile_policy,
                        )?;
                        let offset = u32::try_from(registrations.len())
                            .map_err(|_| "model shader occurrence capacity")?;
                        for materials in model.surfaces.iter_mut().flatten() {
                            for choice in &mut materials.variants {
                                choice.shader = match choice.shader {
                                    ShaderChoice::Occurrence(ShaderOccurrence(index)) => {
                                        ShaderChoice::Occurrence(ShaderOccurrence(index + offset))
                                    }
                                    ShaderChoice::Converted {
                                        binding,
                                        occurrence: ShaderOccurrence(index),
                                    } => ShaderChoice::Converted {
                                        binding,
                                        occurrence: ShaderOccurrence(index + offset),
                                    },
                                    direct => direct,
                                };
                            }
                        }
                        registrations.extend(model.registrations);
                        compiled.push((*slot, model.surfaces));
                    }
                    Ok((compiled, registrations))
                })
            });
            let images = &self.catalog.profiles[profile_index];
            entry.state = match result {
                Ok((compiled, registrations)) => {
                    let mut surfaces = [None; 3];
                    for (slot, compiled) in compiled {
                        let first = self.catalog.surfaces.len() as u32;
                        let count = compiled.len() as u32;
                        let names = &inputs
                            .iter()
                            .find(|input| input.0 == slot)
                            .ok_or_else(|| FileError::capacity("model source slot"))?
                            .3;
                        for (index, skins) in compiled.into_iter().enumerate() {
                            self.catalog.surfaces.push(SurfaceMaterials {
                                name: names[index],
                                skins: IndexSpan {
                                    first: self.catalog.bindings.len() as u32,
                                    count: skins.len() as u32,
                                },
                                wrap_skins: lods.md3,
                                frame_skins: !lods.md3
                                    && self.catalog.sources[inputs[0].1.0 as usize]
                                        .metadata()
                                        .is_some_and(|metadata| {
                                            metadata.format == ModelFormat::Sp2
                                        }),
                            });
                            self.catalog.bindings.extend(skins);
                        }
                        surfaces[slot] = Some(IndexSpan { first, count });
                    }
                    for slot in (0..3).rev() {
                        if let ModelLodSlot::Alias(next) = lods.slots[slot] {
                            surfaces[slot] = surfaces[usize::from(next)];
                        }
                    }
                    renderer_sources |= 1 << images.profile.source_rules as u8;
                    RegistrationState::Ready {
                        value: ModelReady {
                            default_shader: images
                                .native_default
                                .map_or(ShaderBindingId(0), |entry| entry.0),
                            surfaces,
                            registrations: registrations.into_boxed_slice(),
                        },
                        required_assets: (!self.initial_registration).then(|| self.assets.counts()),
                    }
                }
                Err(error) => {
                    self.catalog.diagnostics.push(ModelDiagnostic {
                        name,
                        profile: entry.profile,
                        error: error.clone(),
                    });
                    RegistrationState::Failed(error)
                }
            };
        }
        for (_, source, _, _, meshes) in inputs {
            if let ModelSourceFile::Ready {
                meshes: current, ..
            } = &mut self.catalog.sources[source.0 as usize]
            {
                *current = meshes;
            }
        }
        self.catalog.files[name.0 as usize] = ModelFile::Ready {
            lods,
            renderer_sources,
        };
        self.catalog.capacity.registrations = self.catalog.registrations.len();
        self.catalog.capacity.bindings =
            self.catalog.bindings.len() * ModelMaterialPolicy::ALL.len();
        Ok(self.catalog.files[name.0 as usize]
            .bounds(&self.catalog.sources)
            .unwrap_or_default())
    }

    fn fail_file(&mut self, name: NameId, fresh_profiles: bool, error: FileError) -> FileError {
        let rows = &self.catalog.models[name.0 as usize];
        let unused_suffix = fresh_profiles
            && rows.first().is_some_and(|&first| {
                first + rows.len() == self.catalog.registrations.len()
                    && rows
                        .iter()
                        .copied()
                        .enumerate()
                        .all(|(offset, index)| first + offset == index)
            })
            && rows.iter().copied().all(|index| {
                matches!(
                    self.catalog.registrations[index].state,
                    RegistrationState::Unattempted
                )
            });
        for &index in rows {
            let entry = &mut self.catalog.registrations[index];
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
            self.catalog.registrations.truncate(rows[0]);
            self.catalog.models[name.0 as usize].clear();
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

    pub(crate) fn shader_lighting(
        &self,
        lighting: ShaderLighting,
    ) -> Result<NamedShaderLighting, CallError> {
        Ok(match lighting {
            ShaderLighting::TwoDimensional => NamedShaderLighting {
                source: LightSource::TwoDimensional,
                lightmap: ImageId(0),
            },
            ShaderLighting::World(None) => NamedShaderLighting {
                source: LightSource::Vertex,
                lightmap: ImageId(0),
            },
            ShaderLighting::World(Some(presentation)) => match self
                .assets
                .world_presentation(presentation)
                .ok_or(CallError::File)?
                .native_page0()
            {
                Some(lightmap) => NamedShaderLighting {
                    source: LightSource::Page(0),
                    lightmap,
                },
                None => NamedShaderLighting {
                    source: LightSource::Vertex,
                    lightmap: ImageId(0),
                },
            },
        })
    }
    pub(crate) fn native_resource(
        &self,
        kind: RendererRegistration,
        name: &[u8],
    ) -> Option<ShaderResource> {
        match kind {
            RendererRegistration::Model if !name.starts_with(b"*") => self
                .catalog
                .names
                .find_path_bytes(name)
                .map(ShaderResource::Model),
            RendererRegistration::Skin => {
                find_skin_index(&self.catalog.names, &self.catalog.skin_indices, name)
                    .and_then(|(_, index)| index)
                    .map(|index| ShaderResource::Skin(index + 1))
            }
            _ => None,
        }
    }
    fn admit_shader(
        &mut self,
        vfs: &Vfs,
        raw: &[u8],
        source: RuleSetId,
        mipmaps: bool,
        lighting: ShaderLighting,
    ) -> Result<i32, CallError> {
        self.admit_shader_profile(vfs, raw, source, mipmaps, lighting, None, false)
    }

    fn admit_shader_profile(
        &mut self,
        vfs: &Vfs,
        raw: &[u8],
        source: RuleSetId,
        mipmaps: bool,
        lighting: ShaderLighting,
        exact: Option<ModelProfile>,
        ui_fullscreen: bool,
    ) -> Result<i32, CallError> {
        if raw.is_empty() {
            return Ok(0);
        }
        if raw.len() >= 64 {
            return Err(CallError::Text);
        }
        let selected = match lighting {
            ShaderLighting::TwoDimensional | ShaderLighting::World(None) => None,
            ShaderLighting::World(Some(presentation)) => Some(
                self.assets
                    .world_presentation_registration(presentation)
                    .ok_or(CallError::File)?
                    .profile,
            ),
        };
        let matches_profile = |profile: ModelProfile| {
            profile.source_rules == source
                && exact.is_none_or(|exact| profile == exact)
                && selected.is_none_or(|selected| {
                    profile.rules == selected.rules
                        && profile.palette == selected.indexed_palette
                        && profile.image_settings == selected.image_settings
                        && profile.finish == selected.finish
                })
        };
        if !self
            .catalog
            .profiles
            .iter()
            .any(|entry| matches_profile(entry.profile))
        {
            return Err(CallError::File);
        }
        let cached = find_shader_index(&self.catalog.names, &self.catalog.shader_indices, raw)
            .and_then(|(_, index)| index);
        if let Some(index) = cached {
            if let Some(handle) = self.catalog.shader_source_handle(
                index as i32 + 1,
                lighting,
                matches_profile,
                RegistrationVariant::Guard(ui_fullscreen),
            ) {
                return Ok(handle);
            }
        }
        std::str::from_utf8(raw).map_err(|_| CallError::Text)?;
        let maximum = self
            .catalog
            .limits
            .models
            .checked_mul(self.catalog.profiles.len())
            .and_then(|count| count.checked_mul(self.assets.counts().presentations.checked_add(2)?))
            .and_then(|count| count.checked_mul(2))
            .filter(|&n| n <= u32::MAX as usize)
            .ok_or(CallError::Capacity)?;
        let index = match cached {
            Some(index) => index as usize,
            None => {
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
                let name = self.admit_name(qa_render::shader::registration_key(raw))?;
                let group = self.catalog.names.folded(name).ok_or(CallError::Aborted)?;
                let index = self.catalog.shaders.len();
                self.catalog.shader_indices[group.0 as usize] = Some(index as u32);
                self.catalog.shaders.push(RegisteredShader {
                    name,
                    profiles: (self.catalog.shader_bindings.len()..next).collect(),
                    requests: std::array::from_fn(|_| None),
                });
                self.catalog
                    .shader_bindings
                    .extend(self.catalog.profiles.iter().map(|images| {
                        ShaderProfileBinding {
                            vertex_collapse: images
                                .profile
                                .finish
                                .compile(ui_fullscreen)
                                .collapse_vertex(),
                            profile: images.profile,
                            lighting: ShaderLighting::TwoDimensional,
                            state: RegistrationState::Unattempted,
                        }
                    }));
                index
            }
        };
        {
            for images in &self.catalog.profiles {
                if matches_profile(images.profile)
                    && !self.catalog.shaders[index].profiles.iter().any(|&row| {
                        let binding = &self.catalog.shader_bindings[row];
                        binding.profile == images.profile
                            && binding.lighting == lighting
                            && binding.vertex_collapse
                                == binding
                                    .profile
                                    .finish
                                    .compile(ui_fullscreen)
                                    .collapse_vertex()
                    })
                {
                    if self.catalog.shader_bindings.len() >= maximum {
                        return Err(CallError::Capacity);
                    }
                    self.catalog.shaders[index]
                        .profiles
                        .push(self.catalog.shader_bindings.len());
                    self.catalog.shader_bindings.push(ShaderProfileBinding {
                        vertex_collapse: images
                            .profile
                            .finish
                            .compile(ui_fullscreen)
                            .collapse_vertex(),
                        profile: images.profile,
                        lighting,
                        state: RegistrationState::Unattempted,
                    });
                }
            }
        }
        let name = self.catalog.shaders[index].name;
        let ordinary = lighting == ShaderLighting::TwoDimensional;
        let first_request =
            ordinary && self.catalog.shaders[index].requests[source as usize].is_none();
        let request = match ordinary
            .then(|| self.catalog.shaders[index].requests[source as usize].as_ref())
            .flatten()
        {
            Some(request) => request.clone(),
            None => {
                let mut name = qa_core::text::FixedText::default();
                name.set_bytes(raw).map_err(|_| CallError::Text)?;
                let request = ShaderRequest { name, mipmaps };
                if ordinary {
                    self.catalog.shaders[index].requests[source as usize] = Some(request.clone());
                }
                request
            }
        };
        let named_lighting = self.shader_lighting(lighting)?;
        self.shader_catalog(vfs);
        let catalog = self
            .catalog
            .shader_catalog
            .as_ref()
            .ok_or(CallError::Aborted)?
            .as_ref()
            .map_err(Clone::clone);
        let mut ready = false;
        for &row in &self.catalog.shaders[index].profiles {
            let profile = self.catalog.shader_bindings[row].profile;
            let existing_default = self.catalog.shaders[index]
                .profiles
                .iter()
                .map(|&row| &self.catalog.shader_bindings[row])
                .filter(|entry| {
                    entry.profile == profile
                        && entry.vertex_collapse
                            == entry
                                .profile
                                .finish
                                .compile(ui_fullscreen)
                                .collapse_vertex()
                })
                .find_map(|entry| {
                    entry
                        .state
                        .ready()
                        .filter(|(registration, _)| registration.defaulted.is_some())
                })
                .map(|(registration, required)| (*registration, required));
            let binding = &mut self.catalog.shader_bindings[row];
            if !matches_profile(binding.profile)
                || binding.lighting != lighting
                || binding.vertex_collapse
                    != binding
                        .profile
                        .finish
                        .compile(ui_fullscreen)
                        .collapse_vertex()
                || !matches!(binding.state, RegistrationState::Unattempted)
            {
                continue;
            }
            if let Some((registration, required_assets)) = existing_default {
                ready = true;
                binding.state = RegistrationState::Ready {
                    value: registration,
                    required_assets,
                };
                continue;
            }
            let image_index = self
                .catalog
                .profiles
                .iter()
                .position(|images| images.profile == binding.profile)
                .ok_or(CallError::Aborted)?;
            let compiled = with_image_owner(&mut self.catalog.profiles, image_index, |images| {
                images.compile(
                    vfs,
                    &mut self.assets,
                    catalog.clone(),
                    request.name.as_bytes(),
                    request.mipmaps,
                    named_lighting,
                    ui_fullscreen,
                )
            });
            binding.state = match compiled {
                Ok(registration) => {
                    ready = true;
                    RegistrationState::Ready {
                        value: registration,
                        required_assets: (!self.initial_registration).then(|| self.assets.counts()),
                    }
                }
                Err(error) => {
                    self.catalog.diagnostics.push(ModelDiagnostic {
                        name,
                        profile: binding.profile,
                        error: error.clone(),
                    });
                    RegistrationState::Failed(error)
                }
            };
        }
        if ready {
            self.catalog
                .shader_source_handle(
                    index as i32 + 1,
                    lighting,
                    matches_profile,
                    RegistrationVariant::Guard(ui_fullscreen),
                )
                .ok_or(CallError::Aborted)
        } else {
            if first_request {
                self.catalog.shaders[index].requests[source as usize] = None;
            }
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

    pub(crate) fn preload_model(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
        ui_fullscreen: bool,
    ) -> Result<i32, CallError> {
        let variant = RegistrationVariant::Guard(ui_fullscreen);
        if let Some(handle) = self.catalog.model_source_cached(name, source, variant) {
            return Ok(handle);
        }
        let path = std::str::from_utf8(name).map_err(|_| CallError::Text)?;
        if path.is_empty() || path.starts_with('*') {
            return Err(CallError::File);
        }
        self.ensure_file_profile(vfs, &canonical_path(path), None, ui_fullscreen)
            .map_err(|error| error.kind)?;
        self.catalog
            .model_source_cached(name, source, variant)
            .filter(|&handle| handle > 0)
            .ok_or(CallError::File)
    }

    pub(crate) fn preload_skin(
        &mut self,
        vfs: &Vfs,
        raw: &[u8],
        source: RuleSetId,
        ui_fullscreen: bool,
    ) -> Result<i32, CallError> {
        self.admit_skin_profile(vfs, raw, source, None, ui_fullscreen)
    }

    fn admit_skin_profile(
        &mut self,
        vfs: &Vfs,
        raw: &[u8],
        source: RuleSetId,
        selected: Option<ModelProfile>,
        ui_fullscreen: bool,
    ) -> Result<i32, CallError> {
        if raw.is_empty() {
            return Ok(0);
        }
        if raw.len() >= 64 {
            return Err(CallError::Text);
        }
        let path = canonical_path(std::str::from_utf8(raw).map_err(|_| CallError::Text)?);
        let cached = if let Some(profile) = selected {
            self.catalog.registration_cached_variant(
                RendererRegistration::Skin,
                path.as_bytes(),
                profile,
                None,
                RegistrationVariant::Guard(ui_fullscreen),
            )
        } else {
            find_skin_index(
                &self.catalog.names,
                &self.catalog.skin_indices,
                path.as_bytes(),
            )
            .and_then(|(_, index)| index)
            .and_then(|index| {
                self.catalog.skin_source_handle(
                    index as i32 + 1,
                    source,
                    RegistrationVariant::Guard(ui_fullscreen),
                )
            })
        };
        if let Some(handle) = cached {
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
                        vertex_collapse: images
                            .profile
                            .finish
                            .compile(ui_fullscreen)
                            .collapse_vertex(),
                        profile: images.profile,
                        state: RegistrationState::Unattempted,
                    })
                    .collect(),
            });
            index
        };
        for images in &self.catalog.profiles {
            if !self.catalog.skins[index].profiles.iter().any(|row| {
                row.profile == images.profile
                    && row.vertex_collapse
                        == images
                            .profile
                            .finish
                            .compile(ui_fullscreen)
                            .collapse_vertex()
            }) {
                self.catalog.skins[index].profiles.push(SkinProfile {
                    profile: images.profile,
                    vertex_collapse: images
                        .profile
                        .finish
                        .compile(ui_fullscreen)
                        .collapse_vertex(),
                    state: RegistrationState::Unattempted,
                });
            }
        }
        let profile_count = self.catalog.skins[index]
            .profiles
            .iter()
            .filter(|entry| {
                matches!(entry.state, RegistrationState::Unattempted)
                    && entry.vertex_collapse
                        == entry
                            .profile
                            .finish
                            .compile(ui_fullscreen)
                            .collapse_vertex()
                    && selected.is_none_or(|profile| entry.profile == profile)
            })
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
                mappings.push((surface, row.shader.to_vec()));
            }
            Ok(mappings)
        })();
        let mappings = match mappings {
            Ok(mappings) => mappings,
            Err(error) => {
                for profile in &mut self.catalog.skins[index].profiles {
                    if matches!(profile.state, RegistrationState::Unattempted)
                        && RegistrationVariant::Guard(ui_fullscreen)
                            .matches(profile.profile, profile.vertex_collapse)
                        && selected.is_none_or(|selected| selected == profile.profile)
                    {
                        profile.state =
                            RegistrationState::Failed(format!("skin registration: {error:?}"));
                    }
                }
                return Err(error);
            }
        };
        for row in 0..self.catalog.skins[index].profiles.len() {
            let profile = &mut self.catalog.skins[index].profiles[row];
            if !matches!(profile.state, RegistrationState::Unattempted)
                || profile.vertex_collapse
                    != profile
                        .profile
                        .finish
                        .compile(ui_fullscreen)
                        .collapse_vertex()
                || selected.is_some_and(|selected| profile.profile != selected)
            {
                continue;
            }
            let image_index = self
                .catalog
                .profiles
                .iter()
                .position(|images| images.profile == profile.profile)
                .ok_or(CallError::Aborted)?;
            with_image_owner(
                &mut self.catalog.profiles,
                image_index,
                |images| -> Result<(), CallError> {
                    let alias = !skin_file && images.model_policy.source.indexed_external_pcx;
                    let mut compiled = Vec::with_capacity(mappings.len());
                    let mut registrations = Vec::with_capacity(mappings.len());
                    let mut failure = None;
                    for (surface, raw) in &mappings {
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
                                let materials = materials.pop().ok_or("missing skin material")?;
                                let registration =
                                    images.assets.shader_registration(raw, materials[0])?;
                                Ok((materials, registration))
                            })
                        } else {
                            images
                                .compile(
                                    vfs,
                                    &mut self.assets,
                                    self.catalog
                                        .shader_catalog
                                        .as_ref()
                                        .ok_or(CallError::Aborted)?
                                        .as_ref()
                                        .map_err(Clone::clone),
                                    raw,
                                    true,
                                    NamedShaderLighting {
                                        source: LightSource::None,
                                        lightmap: ImageId(0),
                                    },
                                    ui_fullscreen,
                                )
                                .map(|registration| ([registration.shader; 4], registration))
                        };
                        let materials = match result {
                            Ok((materials, registration)) => {
                                let occurrence = ShaderOccurrence(
                                    u32::try_from(registrations.len())
                                        .map_err(|_| CallError::Capacity)?,
                                );
                                registrations.push(registration);
                                std::array::from_fn(|index| {
                                    if index == 0 || !alias {
                                        ShaderChoice::Occurrence(occurrence)
                                    } else {
                                        ShaderChoice::Direct(materials[index])
                                    }
                                })
                            }
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
                                [ShaderChoice::Direct(ShaderBindingId(0)); 4]
                            }
                        };
                        compiled.push(SkinMapping {
                            surface: *surface,
                            shaders: materials,
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
                            value: SkinReady {
                                mappings: compiled.into_boxed_slice(),
                                registrations: registrations.into_boxed_slice(),
                            },
                            required_assets: (!self.initial_registration)
                                .then(|| self.assets.counts()),
                        }
                    };
                    Ok(())
                },
            )?;
        }
        if let Some(selected) = selected {
            self.catalog.skins[index]
                .profiles
                .iter()
                .find(|entry| {
                    entry.profile == selected
                        && entry.vertex_collapse
                            == entry
                                .profile
                                .finish
                                .compile(ui_fullscreen)
                                .collapse_vertex()
                })
                .and_then(|entry| entry.state.ready())
                .map(|_| index as i32 + 1)
                .ok_or(CallError::File)
        } else {
            self.catalog
                .skin_source_handle(
                    index as i32 + 1,
                    source,
                    RegistrationVariant::Guard(ui_fullscreen),
                )
                .ok_or(CallError::Aborted)
        }
    }
}

fn shader_handle(catalog: &ModelCatalog, raw: &[u8], source: RuleSetId) -> Option<i32> {
    if raw.is_empty() {
        return Some(0);
    }
    let (_, index) = find_shader_index(&catalog.names, &catalog.shader_indices, raw)?;
    let index = index?;
    catalog.shader_source_handle(
        index as i32 + 1,
        ShaderLighting::TwoDimensional,
        |profile| profile.source_rules == source,
        RegistrationVariant::Retained,
    )
}

fn find_shader_index(
    names: &NameTable,
    indices: &[Option<u32>],
    raw: &[u8],
) -> Option<(NameId, Option<u32>)> {
    let group = names.find_folded(qa_render::shader::registration_key(raw))?;
    Some((group, indices.get(group.0 as usize).copied().flatten()))
}
fn find_skin_index(
    names: &NameTable,
    indices: &[Option<u32>],
    raw: &[u8],
) -> Option<(NameId, Option<u32>)> {
    let group = names.find_path_group(raw)?;
    Some((group, indices.get(group.0 as usize).copied().flatten()))
}
fn skin_handle(catalog: &ModelCatalog, raw: &[u8], source: RuleSetId) -> Option<i32> {
    if raw.is_empty() {
        return Some(0);
    }
    let (_, index) = find_skin_index(&catalog.names, &catalog.skin_indices, raw)?;
    let handle = i32::try_from(index?.checked_add(1)?).ok()?;
    catalog.skin_source_handle(handle, source, RegistrationVariant::Retained)
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
fn model_bounds(files: &[ModelFile], sources: &[ModelSourceFile], handle: i32) -> Option<Bounds> {
    match renderer_file(files, handle, None)? {
        ModelFile::Ready { lods, .. } => Some(
            lods.source(0)
                .and_then(|source| sources.get(source.0 as usize))
                .and_then(ModelSourceFile::metadata)
                .map_or_else(Bounds::default, |metadata| metadata.renderer_bounds),
        ),
        file => file.bounds(sources),
    }
}
fn model_tags(files: &[ModelFile], sources: &[ModelSourceFile], handle: i32) -> Option<TagLayout> {
    let ModelFile::Ready { lods, .. } = renderer_file(files, handle, None)? else {
        return None;
    };
    match sources.get(lods.source(0)?.0 as usize)? {
        ModelSourceFile::Ready { tags, .. } => *tags,
        _ => None,
    }
}
fn lerp_tag(
    names: &NameTable,
    files: &[ModelFile],
    sources: &[ModelSourceFile],
    tags: &[TaggedOrientation],
    handle: i32,
    start: i32,
    end: i32,
    fraction: f32,
    raw_name: &[u8],
) -> Option<Orientation> {
    let layout = model_tags(files, sources, handle)?;
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
    fn set_image_search_order(&mut self, order: ImageSearchOrder) {
        ModelAssets::set_image_search_order(self, order);
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
            .and_then(|file| file.bounds(&self.catalog.sources))
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
        // Engine precaches capture the non-UI preload policy. Native renderer
        // requests and live CLIENT preparation supply their own guard.
        self.preload_model(vfs, name, source, false)
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
        // The trait is the fixed Engine preload entry; CLIENT passes its
        // captured guard directly to the same admission path.
        self.preload_skin(vfs, name, source, false)
    }
    fn register_shader(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
        mipmaps: bool,
        lighting: qa_compat::services::ShaderLighting,
    ) -> Result<i32, CallError> {
        self.admit_shader(vfs, name, source, mipmaps, lighting)
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
        model_tags(&self.files, &self.sources, handle).is_some()
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
            &self.sources,
            &self.tags,
            handle,
            start,
            end,
            fraction,
            name,
        )
    }
    fn bounds(&self, path: &[u8], _source: RuleSetId) -> Option<Bounds> {
        file_lookup(&self.names, &self.files, path)?.bounds(&self.sources)
    }
    fn skin_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        skin_handle(self, name, source)
    }
    fn shader_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        shader_handle(self, name, source)
    }
    fn model_handle(&self, path: &[u8], source: RuleSetId) -> Option<i32> {
        model_handle(&self.names, &self.files, path, Some(source))
    }
    fn model_bounds(&self, handle: i32) -> Option<Bounds> {
        model_bounds(&self.files, &self.sources, handle)
    }
}
impl ModelCatalog {
    pub fn profiles(&self) -> impl Iterator<Item = ModelProfile> + '_ {
        self.profiles.iter().map(|entry| entry.profile)
    }
    pub(crate) fn registration_profile(&self, id: ModelRegistrationId) -> Option<ModelProfile> {
        self.registrations
            .get(id.0 as usize)
            .map(|entry| entry.profile)
    }

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
        self.registration_cached_variant(kind, name, profile, world, RegistrationVariant::Retained)
    }
    pub fn registration_cached_variant(
        &self,
        kind: RendererRegistration,
        name: &[u8],
        profile: ModelProfile,
        world: Option<WorldId>,
        variant: RegistrationVariant,
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
                        .iter()
                        .copied()
                        .map(|index| &self.registrations[index])
                        .find(|entry| {
                            entry.profile == profile
                                && variant.matches(entry.profile, entry.vertex_collapse)
                                && !matches!(entry.state, RegistrationState::Unattempted)
                        })?
                        .state
                        .cached(handle, self.admitted_assets),
                }
            }
            RendererRegistration::Shader { lighting, .. } => {
                let (_, index) = find_shader_index(&self.names, &self.shader_indices, name)?;
                let handle = i32::try_from(index?.checked_add(1)?).ok()?;
                let entry =
                    self.shader_profile_lighting_variant(handle, profile, lighting, variant)?;
                let public = entry
                    .state
                    .ready()
                    .is_none_or(|(registration, _)| registration.defaulted.is_none());
                entry
                    .state
                    .cached(if public { handle } else { 0 }, self.admitted_assets)
            }
            RendererRegistration::Skin => {
                let (_, index) = find_skin_index(&self.names, &self.skin_indices, name)?;
                let index = index?;
                let handle = i32::try_from(index.checked_add(1)?).ok()?;
                self.skin_profile_variant(handle, profile, variant)?
                    .state
                    .cached(handle, self.admitted_assets)
            }
        }
    }
    fn model_source_cached(
        &self,
        name: &[u8],
        source: RuleSetId,
        variant: RegistrationVariant,
    ) -> Option<i32> {
        let name = self.names.find_path_bytes(name)?;
        let handle = i32::try_from(name.0).ok()?;
        match self.files.get(name.0 as usize)? {
            ModelFile::Failed(_) => return Some(0),
            ModelFile::LoadedWorld { .. } => return Some(handle),
            ModelFile::Empty | ModelFile::Ready { .. } => {}
        }
        source_handle(
            self.models[name.0 as usize]
                .iter()
                .copied()
                .map(|index| &self.registrations[index])
                .filter(|entry| {
                    entry.profile.source_rules == source
                        && variant.matches(entry.profile, entry.vertex_collapse)
                })
                .map(|entry| &entry.state),
            handle,
            |_| true,
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
        self.renderer_candidate_variant(kind, handle, profile, RegistrationVariant::Retained)
    }
    pub fn renderer_candidate_variant(
        &self,
        kind: RendererRegistration,
        handle: i32,
        profile: ModelProfile,
        variant: RegistrationVariant,
    ) -> bool {
        if handle <= 0 {
            return false;
        }
        match kind {
            RendererRegistration::Model => {
                self.renderer_inline(handle).is_some()
                    || self
                        .renderer_model_candidate_variant(handle, profile, variant)
                        .is_some()
            }
            RendererRegistration::Skin => self
                .skin_profile_variant(handle, profile, variant)
                .is_some_and(|entry| entry.state.ready().is_some()),
            RendererRegistration::Shader { lighting, .. } => self
                .shader_profile_lighting_variant(handle, profile, lighting, variant)
                .is_some_and(|entry| matches!(entry.state, RegistrationState::Ready { .. })),
        }
    }

    pub fn color_binding(&self, profile: ModelProfile) -> Option<ModelColorId> {
        let index = self
            .profiles
            .iter()
            .position(|entry| entry.profile == profile)?;
        let owner = self.profiles[index].image_owner.unwrap_or(index);
        self.profiles[owner].colors.as_ref()?;
        Some(ModelColorId(owner as u32))
    }
    pub fn colors(&self, binding: ModelColorId) -> Option<&PaletteColors> {
        self.profiles.get(binding.0 as usize)?.colors.as_ref()
    }
    fn skin_source_handle(
        &self,
        handle: i32,
        source: RuleSetId,
        variant: RegistrationVariant,
    ) -> Option<i32> {
        let missing = RegistrationState::Unattempted;
        source_handle(
            self.profiles
                .iter()
                .filter(|entry| entry.profile.source_rules == source)
                .map(|entry| {
                    self.skin_profile_variant(handle, entry.profile, variant)
                        .map_or(&missing, |entry| &entry.state)
                }),
            handle,
            |_| true,
        )
    }

    fn shader_source_handle(
        &self,
        handle: i32,
        lighting: ShaderLighting,
        matches_profile: impl Fn(ModelProfile) -> bool,
        variant: RegistrationVariant,
    ) -> Option<i32> {
        let missing = RegistrationState::Unattempted;
        source_handle(
            self.profiles
                .iter()
                .filter(|entry| matches_profile(entry.profile))
                .map(|entry| {
                    self.shader_profile_lighting_variant(handle, entry.profile, lighting, variant)
                        .map_or(&missing, |entry| &entry.state)
                }),
            handle,
            |registration| registration.defaulted.is_none(),
        )
    }

    fn shader_outcome_candidate(
        &self,
        name: &[u8],
        profile: ModelProfile,
        lighting: ShaderLighting,
    ) -> Option<(&ShaderRegistration, Option<AssetCounts>)> {
        self.shader_outcome_candidate_variant(
            name,
            profile,
            lighting,
            RegistrationVariant::Retained,
        )
    }
    fn shader_outcome_candidate_variant(
        &self,
        name: &[u8],
        profile: ModelProfile,
        lighting: ShaderLighting,
        variant: RegistrationVariant,
    ) -> Option<(&ShaderRegistration, Option<AssetCounts>)> {
        let (_, index) = find_shader_index(&self.names, &self.shader_indices, name)?;
        let handle = i32::try_from(index?.checked_add(1)?).ok()?;
        self.shader_profile_lighting_variant(handle, profile, lighting, variant)?
            .state
            .ready()
    }

    /// Published native outcome, including a named default whose public handle is zero.
    pub fn shader_outcome(
        &self,
        name: &[u8],
        profile: ModelProfile,
        lighting: ShaderLighting,
    ) -> Option<&ShaderRegistration> {
        let (registration, required) = self.shader_outcome_candidate(name, profile, lighting)?;
        self.admitted(required).then_some(registration)
    }

    pub fn native_default(&self, profile: ModelProfile) -> Option<ShaderBindingId> {
        let (shader, required) = self
            .profiles
            .iter()
            .find(|entry| entry.profile == profile)?
            .native_default?;
        self.admitted(Some(required)).then_some(shader)
    }
    pub fn shader_binding(&self, handle: i32, profile: ModelProfile) -> Option<ShaderBindingId> {
        let entry = self.shader_profile(handle, profile)?;
        let (registration, required_assets) = entry.state.ready()?;
        (self.admitted(required_assets) && registration.defaulted.is_none())
            .then_some(registration.shader)
    }

    fn shader_profile(&self, handle: i32, profile: ModelProfile) -> Option<&ShaderProfileBinding> {
        self.shader_profile_lighting(handle, profile, ShaderLighting::TwoDimensional)
    }
    fn shader_profile_lighting(
        &self,
        handle: i32,
        profile: ModelProfile,
        lighting: ShaderLighting,
    ) -> Option<&ShaderProfileBinding> {
        self.shader_profile_lighting_variant(
            handle,
            profile,
            lighting,
            RegistrationVariant::Retained,
        )
    }
    fn shader_profile_lighting_variant(
        &self,
        handle: i32,
        profile: ModelProfile,
        lighting: ShaderLighting,
        variant: RegistrationVariant,
    ) -> Option<&ShaderProfileBinding> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        let rows = &self.shaders.get(index)?.profiles;
        rows.iter()
            .map(|&i| &self.shader_bindings[i])
            .find(|entry| {
                entry.profile == profile
                    && variant.matches(entry.profile, entry.vertex_collapse)
                    && entry
                        .state
                        .ready()
                        .is_some_and(|(registration, _)| registration.defaulted.is_some())
            })
            .or_else(|| {
                rows.iter()
                    .map(|&i| &self.shader_bindings[i])
                    .find(|entry| {
                        entry.profile == profile
                            && variant.matches(entry.profile, entry.vertex_collapse)
                            && entry.lighting == lighting
                            && !matches!(entry.state, RegistrationState::Unattempted)
                    })
            })
    }

    pub fn renderer_registrations(
        &self,
        kind: RendererRegistration,
        handle: i32,
        profile: ModelProfile,
    ) -> Option<&[ShaderRegistration]> {
        match kind {
            RendererRegistration::Model => {
                let registration = self.renderer_registration(handle, profile)?;
                let (ready, _) = self.registrations[registration.0 as usize].state.ready()?;
                Some(&ready.registrations)
            }
            RendererRegistration::Skin => {
                let (ready, required) = self.skin_profile(handle, profile)?.state.ready()?;
                self.admitted(required)
                    .then_some(ready.registrations.as_ref())
            }
            RendererRegistration::Shader { lighting, .. } => {
                let (ready, required) = self
                    .shader_profile_lighting(handle, profile, lighting)?
                    .state
                    .ready()?;
                self.admitted(required)
                    .then_some(std::slice::from_ref(ready))
            }
        }
    }
    pub fn shader_name(&self, handle: i32) -> Option<&[u8]> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.names.get(self.shaders.get(index)?.name)
    }
    pub fn metadata(&self, path: &[u8]) -> Option<ModelMetadata> {
        match file_lookup(&self.names, &self.files, path)? {
            ModelFile::Ready { lods, .. } => {
                self.sources.get(lods.source(0)?.0 as usize)?.metadata()
            }
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
        self.renderer_model_candidate_variant(handle, profile, RegistrationVariant::Retained)
    }
    fn registration_name(
        &self,
        name: NameId,
        profile: ModelProfile,
    ) -> Option<ModelRegistrationId> {
        self.registration_name_variant(name, profile, RegistrationVariant::Retained)
    }
    fn skin_profile(&self, handle: i32, profile: ModelProfile) -> Option<&SkinProfile> {
        self.skin_profile_variant(handle, profile, RegistrationVariant::Retained)
    }
    fn renderer_model_candidate_variant(
        &self,
        handle: i32,
        profile: ModelProfile,
        variant: RegistrationVariant,
    ) -> Option<ModelRegistrationId> {
        renderer_file(&self.files, handle, Some(profile.source_rules))?;
        self.registration_name_variant(NameId(u32::try_from(handle).ok()?), profile, variant)
    }
    /// A logical skin handle is usable only when its selected presentation
    /// registered successfully. Other profiles cannot supply a fallback here.
    pub fn renderer_skin(&self, handle: i32, profile: ModelProfile) -> bool {
        self.skin_profile(handle, profile)
            .is_some_and(|entry| entry.state.cached(handle, self.admitted_assets) == Some(handle))
    }
    fn skin_profile_variant(
        &self,
        handle: i32,
        profile: ModelProfile,
        variant: RegistrationVariant,
    ) -> Option<&SkinProfile> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        self.skins.get(index)?.profiles.iter().find(|entry| {
            entry.profile == profile
                && variant.matches(entry.profile, entry.vertex_collapse)
                && !matches!(entry.state, RegistrationState::Unattempted)
        })
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

    fn registration_name_variant(
        &self,
        name: NameId,
        profile: ModelProfile,
        variant: RegistrationVariant,
    ) -> Option<ModelRegistrationId> {
        let rows = self.models.get(name.0 as usize)?;
        rows.iter()
            .copied()
            .find(|&index| {
                let entry = &self.registrations[index];
                entry.profile == profile
                    && variant.matches(entry.profile, entry.vertex_collapse)
                    && entry.state.ready().is_some()
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
        self.model_bindings(registration, 0, skin, policy)
    }

    pub fn lod_bindings(
        &self,
        registration: ModelRegistrationId,
        skin: u32,
        policy: ModelMaterialPolicy,
    ) -> Option<ModelLodBindings<'_>> {
        self.published_registration(registration)?;
        let entry = self.registrations.get(registration.0 as usize)?;
        let ready = entry.state.ready()?.0;
        let ModelFile::Ready { lods, .. } = self.files.get(entry.file.0 as usize)? else {
            return None;
        };
        let radii = lods
            .source(0)
            .and_then(|source| self.sources.get(source.0 as usize))
            .and_then(|source| match source {
                ModelSourceFile::Ready { radii, .. } => Some(radii.as_ref()),
                _ => None,
            })
            .unwrap_or(&[]);
        let mut levels = std::array::from_fn(|_| None);
        for (slot, level) in levels.iter_mut().enumerate() {
            if ready.surfaces[slot].is_some() {
                *level = Some(self.model_bindings(registration, slot, skin, policy)?);
            }
        }
        let remaining = levels
            .iter()
            .flatten()
            .map(|level| level.len())
            .max()
            .unwrap_or(0);
        Some(ModelLodBindings {
            levels,
            remaining,
            info: ModelLodInfo {
                native_count: lods.native_count,
                md3: lods.md3,
                radii,
            },
        })
    }

    /// Frame lookup uses only cached numeric registration, skin and policy.
    fn model_bindings(
        &self,
        registration: ModelRegistrationId,
        slot: usize,
        skin: u32,
        policy: ModelMaterialPolicy,
    ) -> Option<ModelBindings<'_>> {
        self.published_registration(registration)?;
        let ready = self
            .registrations
            .get(registration.0 as usize)?
            .state
            .ready()?
            .0;
        let span = *ready.surfaces.get(slot)?.as_ref()?;
        let custom = match policy
            .custom_skin
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok())
            .and_then(|index| self.skins.get(index))
        {
            Some(_) => Some((
                policy.custom_skin as u32,
                match self.skin_profile(
                    policy.custom_skin,
                    self.registrations[registration.0 as usize].profile,
                ) {
                    Some(profile) => match &profile.state {
                        RegistrationState::Ready {
                            value,
                            required_assets,
                        } => {
                            if !self.admitted(*required_assets) {
                                return None;
                            }
                            Some(value)
                        }
                        RegistrationState::Failed(_) => None,
                        RegistrationState::Unattempted => return None,
                    },
                    None => None,
                },
            )),
            None => None,
        };
        Some(ModelBindings {
            surfaces: &self.surfaces[span.range()],
            materials: &self.bindings,
            skin,
            policy,
            custom,
            registrations: &ready.registrations,
            resource: ShaderResource::Model(self.registrations[registration.0 as usize].file),
            default_shader: ready.default_shader,
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

struct CompiledModel {
    surfaces: Vec<Vec<ModelMaterials>>,
    registrations: Vec<ShaderRegistration>,
}

fn register(
    model: &Model<'_>,
    images: &mut Images<'_>,
    registration: &Registration,
    model_policy: ModelPolicy,
    native_default: ShaderBindingId,
    shared_meshes: &mut [Option<ModelId>],
    shader_catalog: Option<&Result<ShaderCatalog, String>>,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
    compile_policy: ShaderCompilePolicy,
) -> Result<CompiledModel, String> {
    if model.format == ModelFormat::Sp2 {
        return register_sprites(
            model,
            images,
            registration,
            model_policy,
            shared_meshes,
            capacity,
            limits,
        );
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
    let mut registrations = Vec::new();
    if let Some(alias) = &alias {
        for (skin, variants) in model.skins.iter().zip(alias) {
            let raw = match skin {
                Image::External(raw) => *raw,
                _ => registration.path.as_bytes(),
            };
            registrations.push(images.assets.shader_registration(raw, variants[0])?);
        }
    }
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
            materials
                .iter()
                .enumerate()
                .map(|(skin, variants)| {
                    std::array::from_fn(|index| {
                        if index == 0 && matches!(model.skins[skin], Image::External(_)) {
                            ShaderChoice::Occurrence(ShaderOccurrence(skin as u32))
                        } else {
                            ShaderChoice::Direct(variants[index])
                        }
                    })
                })
                .collect::<Vec<_>>()
        } else if mesh.shaders.is_empty() {
            vec![[ShaderChoice::Direct(native_default); 4]]
        } else {
            mesh.shaders
                .iter()
                .map(|shader| {
                    let registration = compile_named_shader(
                        shader_catalog
                            .ok_or("missing model shader catalog")?
                            .as_ref()
                            .map_err(Clone::clone)?,
                        shader.name,
                        true,
                        NamedShaderLighting {
                            source: LightSource::None,
                            lightmap: ImageId(0),
                        },
                        compile_policy,
                        images,
                    )
                    .map_err(|error| format!("model shader: {error:?}"))?;
                    let occurrence = ShaderOccurrence(
                        u32::try_from(registrations.len())
                            .map_err(|_| "model shader occurrence capacity")?,
                    );
                    registrations.push(registration);
                    Ok([ShaderChoice::Occurrence(occurrence); 4])
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        let id = if let Some(id) = *shared_mesh {
            id
        } else {
            let native = (model.format == ModelFormat::Md2).then_some(model.frames.as_slice());
            let (vertices, frame_size, indices, packed) =
                mesh_vertices(mesh, native, capacity, limits)?;
            let choice = materials.first().ok_or("model has no skin material")?[0];
            let material = ShaderView::direct()
                .choice(
                    choice,
                    &registrations,
                    None,
                    ShaderDefault::Global(Some(native_default)),
                )
                .and_then(|shader| shader.source(&()))
                .ok_or("model shader choice")?;
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
                    variants: materials.map(|material| ModelChoice {
                        model: id,
                        shader: material,
                        pose: model_policy.presentation.pose,
                    }),
                })
                .collect(),
        );
    }
    capacity.skins = skin_count;
    capacity.surfaces = next_surfaces;
    Ok(CompiledModel {
        surfaces,
        registrations,
    })
}

fn register_sprites(
    model: &Model<'_>,
    images: &mut Images<'_>,
    registration: &Registration,
    policy: ModelPolicy,
    shared_meshes: &mut [Option<ModelId>],
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<CompiledModel, String> {
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
    let mut registrations = Vec::with_capacity(model.sprites.len());
    for (index, (sprite, shared_mesh)) in model.sprites.iter().zip(shared_meshes).enumerate() {
        if !matches!(sprite.image, Image::External(_)) {
            return Err("SP2 frame requires an external image".to_owned());
        }
        let image = model_image(
            sprite.image,
            images,
            registration.profile,
            ImageRole::Sprite,
            true,
            Some(255),
        )
        .map_err(|error| format!("sprite image: {error:?}"))?;
        let mut variants = [ShaderBindingId(0); 4];
        for material_policy in ModelMaterialPolicy::ALL {
            let material = images.assets.register_material(
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
            variants[material_policy.index()] = images
                .assets
                .register_shader_binding(ShaderBinding::material(material, LightSource::None))?;
        }
        if let Image::External(raw) = sprite.image {
            registrations.push(images.assets.shader_registration(raw, variants[0])?);
        }
        let geometry = qa_render::assets::SpriteGeometry {
            extents: [
                -(sprite.origin[0] as f32),
                (i64::from(sprite.width) - i64::from(sprite.origin[0])) as f32,
                (i64::from(sprite.height) - i64::from(sprite.origin[1])) as f32,
                -(sprite.origin[1] as f32),
            ],
        };
        let model = match *shared_mesh {
            Some(model) => model,
            None => {
                let model = images.assets.register_sprite(geometry, variants[0])?;
                *shared_mesh = Some(model);
                model
            }
        };
        frames.push(ModelMaterials {
            variants: std::array::from_fn(|index| ModelChoice {
                model,
                shader: if index == 0 {
                    ShaderChoice::Occurrence(ShaderOccurrence((registrations.len() - 1) as u32))
                } else {
                    ShaderChoice::Direct(variants[index])
                },
                pose: policy.presentation.pose,
            }),
        });
    }
    capacity.surfaces = next_surfaces;
    capacity.skins = next_skins;
    Ok(CompiledModel {
        surfaces: vec![frames],
        registrations,
    })
}

fn model_image(
    image: Image<'_>,
    images: &mut Images<'_>,
    options: ModelProfile,
    role: ImageRole,
    indexed_external_pcx: bool,
    transparent: Option<u8>,
) -> Result<ImageId, ResourceError> {
    let usage = ImageUse {
        role,
        ..ImageUse::default()
    };
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
) -> Result<Vec<[ShaderBindingId; 4]>, String> {
    let options = registration.profile;
    let mut materials = Vec::with_capacity(skins.len());
    let mut default_image = None;
    for (index, skin) in skins.iter().enumerate() {
        let result = model_image(
            *skin,
            images,
            options,
            ImageRole::Skin,
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
        let mut variants = [ShaderBindingId(0); 4];
        for policy in ModelMaterialPolicy::ALL {
            let material = images.assets.register_material(
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
            variants[policy.index()] = images
                .assets
                .register_shader_binding(ShaderBinding::material(material, LightSource::None))?;
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
