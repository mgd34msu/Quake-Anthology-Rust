//! Cold model registration. Native config ordinals resolve through filenames;
//! inline ordinals retain their explicit world and existing surface ranges.
use qa_compat::services::{
    CallError, LoadedWorldBinding, ModelLoader, ModelLookup, RendererRegistration, RendererRequest,
    ShaderLighting,
};
use qa_content::vfs::{ReadView, SearchView, SourcePrecedence, Vfs};
use qa_core::names::{NameTable, canonical_path, file_stem};
use qa_core::primitives::{Bounds, IndexSpan, ModelFileId, NameId, Orientation, RuleSetId};
use qa_formats::archive::ArchiveReader;
use qa_formats::model::{Image, Md5Animation, Mesh, Model, ModelFormat};
use qa_gameplay::rules::appearance::{self, AliasPose, ModelPresentation, ModelSource};
use qa_render::assets::{
    AssetCounts, ImageId, MaterialSettings, ModelAnimation, SkeletalMesh, Skeleton, SoftwareAlias,
    Stage, StageTexture, TimedGroup,
};
use qa_render::material::resources::{
    ImageRegistrationMark, ImageReload, ImageReloadScope, ImageRole, ImageSearchOrder,
    ImageSettings, ImageUse, Images, ImagesState, IndexedSkinLayout, IndexedSkinReload,
    IndexedSkinSource, IndexedSkinTranslation, IndexedSkinView, IndexedTarget, ResourceError,
};
use qa_render::material::world_load::{
    NamedShaderLighting, WorldImageReload, WorldPresentationRequest, compile_named_shader,
    native_default_shader,
};
use qa_render::material::{MaterialFinishPolicy, ShaderCompilePolicy};
use qa_render::shader::context::ShaderNodeId;
use qa_render::shader::reference::{
    ShaderChoice, ShaderDefault, ShaderOccurrence, ShaderRef, ShaderView,
};
use qa_render::shader::{AlphaFunc, AlphaGen, Cull, RgbGen, ShaderCatalog, StageBlend};
use qa_render::surface_cache::IndexedTexture;
use qa_render::world::geometry::LightSource;
use qa_render::world::{WorldId, WorldPresentationId};
use qa_render::{
    Assets, ModelId, PaletteId, ShaderBinding, ShaderBindingId, ShaderRegistration, Vertex,
};
use std::sync::Arc;

mod files;
use crate::resource_scopes as scopes;
use files::ModelFiles;
use scopes::ScopedBindings;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RegistrationIndex(u32);
impl qa_core::bindings::BindingId for RegistrationIndex {
    fn index(self) -> usize {
        self.0 as usize
    }
}

mod sources;
use sources::{ModelSourceId, ModelSources};

#[cfg(test)]
#[path = "../tests/unit/renderer_registrations.rs"]
mod registration_tests;

#[cfg(test)]
#[path = "../tests/unit/md5_registration.rs"]
mod md5_registration_tests;

#[cfg(test)]
#[path = "../tests/unit/model_source_scopes.rs"]
mod source_scope_tests;

#[derive(Clone, Copy, Debug)]
pub struct ModelRequest<'a> {
    pub path: &'a str,
    /// File and companion reads share this selection; None uses the owned default.
    pub search: Option<&'a SearchView>,
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
            search: None,
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

struct RegistrationTarget<'a> {
    request: RendererRequest<&'a [u8]>,
    file: Option<ModelFileId>,
}
impl<'a> RegistrationTarget<'a> {
    fn named(request: &'a RendererRequest<impl AsRef<[u8]>>) -> Self {
        Self {
            request: RendererRequest {
                search: request.search.clone(),
                name: request.name.as_ref(),
                source: request.source,
                kind: request.kind,
                world: request.world,
            },
            file: None,
        }
    }
    fn retained(file: ModelFileId, source: RuleSetId) -> Self {
        Self {
            request: RendererRequest {
                search: None,
                name: &[],
                source,
                kind: RendererRegistration::Model,
                world: None,
            },
            file: Some(file),
        }
    }
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
    fn from_collapse(profile: ModelProfile, collapse: bool) -> Result<Self, String> {
        [false, true]
            .into_iter()
            .find(|&guard| profile.finish.compile(guard).collapse_vertex() == collapse)
            .map(Self::Guard)
            .ok_or_else(|| "renderer registration finish variant".to_owned())
    }
}

fn image_profile_index(
    profiles: &[ProfileImages],
    profile: ModelProfile,
    source: &SearchView,
) -> Option<usize> {
    profiles
        .iter()
        .position(|entry| entry.profile == profile && entry.source.same_selection(source))
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
    pub search: Option<SearchView>,
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
    skins: Vec<(SkinSelection, SkinSelection)>,
    pub(crate) worlds: Vec<(
        WorldId,
        WorldPresentationId,
        qa_render::material::world_load::LoadedWorldPresentation,
    )>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageRefreshMode {
    Reconfigure,
    Reload,
    IndexedSkins,
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
}
impl ModelMaterialPolicy {
    const ALL: [Self; 4] = [
        Self {
            translucent: false,
            fullbright: false,
            custom_skin: 0,
        },
        Self {
            translucent: false,
            fullbright: true,
            custom_skin: 0,
        },
        Self {
            translucent: true,
            fullbright: false,
            custom_skin: 0,
        },
        Self {
            translucent: true,
            fullbright: true,
            custom_skin: 0,
        },
    ];
    fn index(self) -> usize {
        usize::from(self.translucent) * 2 + usize::from(self.fullbright)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct ModelMaterials {
    variants: [ModelChoice; 4],
    /// Exact Assets filename whose absence selected this row's alias fallback.
    missing_image: Option<NameId>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct ModelChoice {
    model: ModelId,
    shader: ShaderChoice,
    pose: AliasPose,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum ShaderMembers<'a> {
    Models(&'a [ModelMaterials]),
    Skins(&'a [[ShaderChoice; 4]]),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelShaderMembers<'a> {
    materials: ShaderMembers<'a>,
    policy: usize,
}
impl<'a> ModelShaderMembers<'a> {
    fn range(self, range: std::ops::Range<usize>) -> Option<Self> {
        Some(Self {
            materials: match self.materials {
                ShaderMembers::Models(rows) => ShaderMembers::Models(rows.get(range)?),
                ShaderMembers::Skins(rows) => ShaderMembers::Skins(rows.get(range)?),
            },
            ..self
        })
    }
    fn choice(
        mut self,
        selected: u32,
        timed: Option<(u32, IndexSpan)>,
        wrap: bool,
        frames: bool,
    ) -> Option<ModelShaderChoice<'a>> {
        if self.len() == 0 {
            return None;
        }
        if frames {
            return Some(ModelShaderChoice::Frames { members: self });
        }
        let selected = if let Some((group, range)) = timed {
            self = self.range(range.range())?;
            if range.count > 1 {
                return Some(ModelShaderChoice::SkinGroup {
                    group,
                    members: self,
                });
            }
            0
        } else if wrap {
            selected as usize % self.len()
        } else if (selected as usize) < self.len() {
            selected as usize
        } else {
            0
        };
        Some(ModelShaderChoice::Single(self.nth(selected)?))
    }
}
impl Iterator for ModelShaderMembers<'_> {
    type Item = ShaderChoice;
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.materials {
            ShaderMembers::Models(rows) => {
                let (member, remaining) = rows.split_first()?;
                *rows = remaining;
                Some(member.variants[self.policy].shader)
            }
            ShaderMembers::Skins(rows) => {
                let (member, remaining) = rows.split_first()?;
                *rows = remaining;
                Some(member[self.policy])
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match self.materials {
            ShaderMembers::Models(rows) => rows.len(),
            ShaderMembers::Skins(rows) => rows.len(),
        };
        (len, Some(len))
    }
}
impl ExactSizeIterator for ModelShaderMembers<'_> {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModelShaderChoice<'a> {
    Single(ShaderChoice),
    SkinGroup {
        group: u32,
        members: ModelShaderMembers<'a>,
    },
    Frames {
        members: ModelShaderMembers<'a>,
    },
}
#[derive(Clone, Copy)]
pub struct SelectedModelChoice<'a> {
    pub model: ModelId,
    pub shader: ModelShaderChoice<'a>,
    pub pose: AliasPose,
    pub resource: ShaderResource,
    pub registrations: &'a [ShaderRegistration],
    pub default: ShaderDefault,
    timing_source: Option<ModelId>,
}
impl SelectedModelChoice<'_> {
    pub fn capture<'capture>(
        self,
        capture: &mut qa_render::scene::ModelCapture<'capture>,
        view: ShaderView<'_>,
        nodes: Option<&[ShaderNodeId]>,
        override_shader: Option<ShaderRef>,
    ) -> Option<qa_render::scene::ModelChoice<'capture>> {
        use qa_render::scene::{ModelChoice as Choice, ModelPart, ModelSelector};
        if !self.shaders().all(|shader| {
            override_shader
                .or_else(|| self.resolve_shader(shader, view, nodes))
                .is_some()
        }) {
            return None;
        }
        let leaf = |shader| {
            Some(Choice::part(ModelPart {
                model: self.model,
                shader: override_shader.or_else(|| self.resolve_shader(shader, view, nodes))?,
            }))
        };
        match self.shader {
            ModelShaderChoice::Single(shader) => leaf(shader),
            ModelShaderChoice::SkinGroup { group, members } => capture.select(
                ModelSelector::SkinGroup {
                    timing_source: self.timing_source.unwrap_or(self.model),
                    group,
                },
                members.filter_map(leaf),
            ),
            ModelShaderChoice::Frames { members } => capture.select(
                ModelSelector::SpriteFrame { model: self.model },
                members.filter_map(leaf),
            ),
        }
    }
    pub fn shaders(&self) -> impl Iterator<Item = ShaderChoice> + Clone {
        let (single, members) = match self.shader {
            ModelShaderChoice::Single(shader) => (Some(shader), None),
            ModelShaderChoice::SkinGroup { members, .. }
            | ModelShaderChoice::Frames { members } => (None, Some(members)),
        };
        single.into_iter().chain(members.into_iter().flatten())
    }
    pub fn resolve_shader(
        &self,
        shader: ShaderChoice,
        view: ShaderView<'_>,
        nodes: Option<&[ShaderNodeId]>,
    ) -> Option<ShaderRef> {
        view.choice(shader, self.registrations, nodes, self.default)
    }
    pub fn physical_binding(&self) -> Option<ModelBinding> {
        let ModelShaderChoice::Single(shader) = self.shader else {
            return None;
        };
        let shader = self
            .resolve_shader(shader, ShaderView::direct(), None)?
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
        let count = self.0.surfaces.len();
        (count, Some(count))
    }
}
impl ExactSizeIterator for ModelChoices<'_> {}
#[derive(Clone, Copy, Debug, PartialEq)]
struct SurfaceMaterials {
    name: NameId,
    skins: IndexSpan,
    wrap_skins: bool,
    sprite_frames: bool,
}

/// Numeric, allocation-free iteration over every surface of one registration.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelBindings<'a> {
    surfaces: &'a [SurfaceMaterials],
    materials: &'a [ModelMaterials],
    skin: u32,
    policy: ModelMaterialPolicy,
    profile: ModelProfile,
    custom: Option<(u32, Option<&'a SkinReady>)>,
    registrations: &'a [ShaderRegistration],
    resource: ShaderResource,
    default_shader: ShaderBindingId,
    animation: Option<&'a ModelAnimation>,
    timing_source: Option<ModelId>,
}
impl<'a> ModelBindings<'a> {
    pub fn len(&self) -> usize {
        self.surfaces.len()
    }
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
    pub fn choices(self) -> ModelChoices<'a> {
        ModelChoices(self)
    }
    fn next_choice(&mut self) -> Option<SelectedModelChoice<'a>> {
        let (surface, remaining) = self.surfaces.split_first()?;
        self.surfaces = remaining;
        let selected = self.skin;
        let timed = self
            .animation
            .filter(|_| !surface.sprite_frames)
            .map(|animation| {
                let groups = animation.skin_groups();
                let group = if (selected as usize) < groups.len() {
                    selected
                } else {
                    0
                };
                (group, groups[group as usize].members)
            });
        // Both mesh compilers assign one geometry/pose to all surface skins.
        let binding = self.materials[surface.skins.first as usize].variants[0];
        let mut registrations = self.registrations;
        let mut resource = self.resource;
        let mut default = ShaderDefault::Global(Some(self.default_shader));
        let mut shader = ModelShaderMembers {
            materials: ShaderMembers::Models(&self.materials[surface.skins.range()]),
            policy: self.policy.index(),
        }
        .choice(selected, timed, surface.wrap_skins, surface.sprite_frames)?;
        if let Some((handle, custom)) = self.custom.filter(|_| !surface.sprite_frames) {
            shader = ModelShaderChoice::Single(ShaderChoice::Direct(self.default_shader));
            if let Some(custom) = custom {
                if let Ok(mapping) = custom
                    .mappings
                    .binary_search_by_key(&surface.name.0, |entry| entry.surface.0)
                {
                    let span = custom.mappings[mapping].members;
                    shader = ModelShaderMembers {
                        materials: ShaderMembers::Skins(custom.members.get(span.range())?),
                        policy: self.policy.index(),
                    }
                    .choice(
                        selected,
                        (span.count > 1).then_some(timed).flatten(),
                        surface.wrap_skins,
                        false,
                    )?;
                }
                registrations = &custom.registrations;
                resource = ShaderResource::Skin(handle);
                default = ShaderDefault::Named;
            }
        }
        Some(SelectedModelChoice {
            model: binding.model,
            shader,
            pose: binding.pose,
            resource,
            registrations,
            default,
            timing_source: self.timing_source,
        })
    }
}
impl Iterator for ModelBindings<'_> {
    type Item = ModelBinding;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_choice()?.physical_binding()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.surfaces.len()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelLodInfo<'a> {
    native_count: usize,
    supports_lods: bool,
    radii: &'a [f32],
}
impl ModelLodInfo<'_> {
    pub fn native_count(self) -> usize {
        self.native_count
    }
    pub fn base_frames(self) -> usize {
        self.radii.len()
    }
    pub fn frame_count(self, native_frames: u32) -> u32 {
        if self.supports_lods {
            self.radii.len() as u32
        } else {
            native_frames
        }
    }
    pub fn radius(self, frame: u32) -> Option<f32> {
        self.radii.get(frame as usize).copied()
    }
    pub fn supports_lods(self) -> bool {
        self.supports_lods
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ModelLodBindings<'a> {
    levels: [Option<ModelBindings<'a>>; 3],
    info: ModelLodInfo<'a>,
    remaining: usize,
    replacement: Option<(
        ModelBindings<'a>,
        Option<qa_render::scene::SkeletalFrameMap>,
        RuleSetId,
    )>,
}
impl<'a> ModelLodBindings<'a> {
    pub fn with_skin(
        mut self,
        catalog: &'a ModelCatalog,
        selection: SkinSelection,
    ) -> Option<Self> {
        let skin = catalog.skins.get(selection.skin as usize)?;
        let row = skin.profiles.get(selection.row as usize)?;
        if !matches!(row.request, SkinRequest::ModelColors(_)) {
            return None;
        }
        let SkinSource::AliasModel(source) = skin.source.as_ref()? else {
            return None;
        };
        let (ready, required) = row.state.ready()?;
        if !catalog.admitted(required) {
            return None;
        }
        for level in self
            .levels
            .iter_mut()
            .flatten()
            .chain(self.replacement.iter_mut().map(|(bindings, ..)| bindings))
        {
            if level.profile != row.profile {
                return None;
            }
            let ShaderResource::Model(file) = level.resource else {
                return None;
            };
            let ModelFile::Ready { lods, .. } = catalog.files.get(file)? else {
                return None;
            };
            if lods.source(0) != Some(*source) {
                return None;
            }
            level.custom = Some((selection.skin + 1, Some(ready)));
        }
        Some(self)
    }
    pub fn native_only(mut self) -> Self {
        self.replacement = None;
        self.remaining = self
            .levels
            .iter()
            .flatten()
            .map(ModelBindings::len)
            .max()
            .unwrap_or(0);
        self
    }
    pub fn first_choice(&self) -> Option<SelectedModelChoice<'a>> {
        self.levels
            .iter()
            .flatten()
            .find_map(|bindings| bindings.clone().next_choice())
    }
    pub fn capture(
        mut self,
        frame: &mut qa_render::Frame,
        pose: qa_render::SceneEntity,
        view: ShaderView<'_>,
        nodes: Option<&[ShaderNodeId]>,
        override_shader: Option<ShaderRef>,
    ) -> bool {
        use qa_render::scene::{ModelChoice as Choice, ModelSelector};
        let mut replacement = self.replacement();
        let info = self.info;
        frame.add_model_choices(pose, |capture| {
            while let Some(row) = self.next() {
                let mut children = [Choice::missing(); 3];
                for (target, choice) in children.iter_mut().zip(row) {
                    if let Some(choice) = choice {
                        let Some(node) = choice.capture(capture, view, nodes, override_shader)
                        else {
                            return false;
                        };
                        *target = node;
                    }
                }
                let Some(mut root) = capture.select(
                    ModelSelector::Lod {
                        radius: info.radius(pose.frame),
                    },
                    children.into_iter().take(info.native_count()),
                ) else {
                    return false;
                };
                if let Some((bindings, mapping, rules)) = &mut replacement {
                    let replacement = match bindings.next_choice() {
                        Some(choice) => {
                            let Some(mut node) =
                                choice.capture(capture, view, nodes, override_shader)
                            else {
                                return false;
                            };
                            if let Some(mapping) = mapping {
                                let Some(mapped) = capture.map_frames(*mapping, node) else {
                                    return false;
                                };
                                node = mapped;
                            }
                            node
                        }
                        None => Choice::missing(),
                    };
                    let Some(selected) = capture.select(
                        ModelSelector::Replacement { rules: *rules },
                        [root, replacement],
                    ) else {
                        return false;
                    };
                    root = selected;
                }
                if !capture.submit(root) {
                    return false;
                }
            }
            true
        })
    }
    pub fn replacement(
        &self,
    ) -> Option<(
        ModelBindings<'a>,
        Option<qa_render::scene::SkeletalFrameMap>,
        RuleSetId,
    )> {
        self.replacement.clone()
    }
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
    handle: Option<ModelFileId>,
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
    pub model_choices: usize,
}

struct Registration {
    path: String,
    profile: ModelProfile,
    indexed_palette: Option<PaletteId>,
    required: bool,
}
struct RegisteredProfile {
    file: ModelFileId,
    profile: ModelProfile,
    vertex_collapse: bool,
    state: RegistrationState<ModelReady>,
}
struct ModelReady {
    indexed_palette: Option<PaletteId>,
    surfaces: [Option<IndexSpan>; 3],
    replacement: Option<ReplacementReady>,
    default_shader: ShaderBindingId,
    registrations: Box<[ShaderRegistration]>,
}
struct ReplacementReady {
    surfaces: IndexSpan,
    frames: Option<qa_render::scene::SkeletalFrameMap>,
    rules: RuleSetId,
}
struct RegisteredShader {
    name: NameId,
    search: SearchView,
    profiles: Vec<usize>,
    first_request_rows: [Option<usize>; 5],
}
impl RegisteredShader {
    fn first_request<'a>(
        &self,
        bindings: &'a [ShaderProfileBinding],
        source: RuleSetId,
    ) -> Option<&'a ShaderRequest> {
        bindings
            .get(self.first_request_rows[source as usize]?)?
            .request
            .as_ref()
    }
}
#[derive(Clone)]
struct ShaderRequest {
    name: qa_core::text::FixedText<64>,
    mipmaps: bool,
}
struct ShaderProfileBinding {
    shader: RegistrationIndex,
    profile: ModelProfile,
    vertex_collapse: bool,
    lighting: ShaderLighting,
    request: Option<ShaderRequest>,
    state: RegistrationState<ShaderRegistration>,
}
enum RegistrationState<T> {
    Unattempted,
    Reload,
    Failed(String),
    Ready {
        value: T,
        required_assets: Option<AssetCounts>,
    },
}
impl<T> RegistrationState<T> {
    fn pending(&self) -> bool {
        matches!(self, Self::Unattempted | Self::Reload)
    }
    fn ready(&self) -> Option<(&T, Option<AssetCounts>)> {
        match self {
            Self::Ready {
                value,
                required_assets,
            } => Some((value, *required_assets)),
            Self::Unattempted | Self::Reload | Self::Failed(_) => None,
        }
    }
    fn cached(&self, handle: i32, admitted: Option<AssetCounts>) -> Option<i32> {
        match self {
            Self::Unattempted | Self::Reload => None,
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
            Self::Unattempted | Self::Reload | Self::Failed(_) => None,
        }
    }
}
#[derive(Clone, Copy)]
enum RegistrationRollback {
    Retry,
    Source,
}
#[derive(Clone)]
enum SkinSource {
    AliasModel(ModelSourceId),
    Script(Arc<[u8]>),
    Indexed {
        layout: IndexedSkinLayout,
        source: Arc<IndexedSkinSource>,
    },
    IndexedFailure {
        layout: IndexedSkinLayout,
        missing: bool,
        error: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IndexedSkinRequest {
    layout: IndexedSkinLayout,
    translation: IndexedSkinTranslation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSkinColors {
    pub indexed: [u8; 2],
    pub rgba: [u8; 2],
    pub upload_extent: Option<[u32; 2]>,
    pub rgba_crop: Option<[u32; 2]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkinRequest {
    Named,
    Indexed(IndexedSkinRequest),
    ModelColors(ModelSkinColors),
}
/// Exact derived body under an ordinary, shared skin filename row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkinSelection {
    skin: u32,
    row: u32,
}
#[derive(Debug)]
pub enum IndexedSkinError {
    Missing,
    InvalidSource(String),
    Rejected(String),
}
impl std::fmt::Display for IndexedSkinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("missing indexed skin"),
            Self::InvalidSource(error) | Self::Rejected(error) => f.write_str(error),
        }
    }
}
struct RegisteredSkin {
    name: NameId,
    search: SearchView,
    profiles: Vec<SkinProfile>,
    source: Option<SkinSource>,
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
            RegistrationState::Unattempted | RegistrationState::Reload => return None,
            RegistrationState::Ready { value, .. } => ready |= public(value),
            RegistrationState::Failed(_) => {}
        }
    }
    configured.then_some(if ready { handle } else { 0 })
}

struct SkinProfile {
    profile: ModelProfile,
    request: SkinRequest,
    vertex_collapse: bool,
    state: RegistrationState<SkinReady>,
    reserved: Option<SkinReady>,
}
#[derive(Clone, Debug, PartialEq)]
struct SkinReady {
    mappings: Box<[SkinMapping]>,
    members: Box<[[ShaderChoice; 4]]>,
    registrations: Box<[ShaderRegistration]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct SkinMapping {
    surface: NameId,
    members: IndexSpan,
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
    fn file(message: String) -> Self {
        Self {
            kind: CallError::File,
            message,
        }
    }
}
impl From<ResourceError> for FileError {
    fn from(error: ResourceError) -> Self {
        Self::file(format!("image registration: {error:?}"))
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
        native_checksums: Option<qa_formats::bsp::NativeMapChecksums>,
    },
    Failed(FileError),
}
#[derive(Clone, Copy)]
enum ModelLodSlot {
    Missing,
    Invalid,
    Source(ModelSourceId),
    Alias(u8),
}
#[derive(Clone, Copy)]
struct ModelLods {
    slots: [ModelLodSlot; 3],
    native_count: usize,
    supports_lods: bool,
}
impl ModelLods {
    fn source(self, mut slot: usize) -> Option<ModelSourceId> {
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
struct Md5Source {
    // Keep the accepted pair intact across profile changes and image reloads.
    _animation_bytes: Arc<[u8]>,
    _scale_bytes: Option<Arc<[u8]>>,
    skeleton: Arc<Skeleton>,
    diagnostics: Arc<[String]>,
}
#[derive(Clone)]
struct ReplacementSource {
    source: ModelSourceId,
    skins: Arc<[Box<[NameId]>]>,
}
#[derive(Clone)]
enum ReplacementState {
    Unattempted,
    Unavailable(Option<String>),
    Ready(ReplacementSource),
}
#[derive(Clone)]
enum ModelSourceFile {
    Empty,
    Invalid {
        error: FileError,
        supports_lods: bool,
    },
    Ready {
        bytes: Arc<[u8]>,
        precedence: SourcePrecedence,
        metadata: ModelMetadata,
        skin_extent: Option<[u32; 2]>,
        indexed_skins: Arc<[IndexSpan]>,
        source_images: Arc<[IndexSpan]>,
        animation: Option<Arc<ModelAnimation>>,
        skeletal: Option<Md5Source>,
        replacement: ReplacementState,
        radii: Arc<[f32]>,
        tags: Option<TagLayout>,
        meshes: Vec<Option<ModelId>>,
    },
}
fn retained_model_span(bytes: &[u8], part: &[u8]) -> Result<IndexSpan, FileError> {
    let first = part
        .as_ptr()
        .addr()
        .checked_sub(bytes.as_ptr().addr())
        .filter(|&first| {
            first
                .checked_add(part.len())
                .is_some_and(|end| end <= bytes.len())
        })
        .and_then(|first| u32::try_from(first).ok())
        .ok_or_else(|| FileError::capacity("model source range"))?;
    let count =
        u32::try_from(part.len()).map_err(|_| FileError::capacity("model source length"))?;
    Ok(IndexSpan { first, count })
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
    fn bounds(&self, sources: &ModelSources) -> Option<Bounds> {
        match self {
            Self::Ready { lods, .. } => sources
                .get(lods.source(0)?)?
                .metadata()
                .map(|metadata| metadata.bounds),
            Self::LoadedWorld { bounds, .. } => Some(*bounds),
            Self::Empty | Self::Failed(_) => None,
        }
    }
}
struct ProfileImages {
    profile: ModelProfile,
    source: SearchView,
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
        source: SearchView,
        previous: &[Self],
        assets: &mut Assets,
    ) -> Result<(Self, Option<qa_render::assets::PreparedPaletteTranslucency>), String> {
        if let Some((index, owner)) = previous.iter().enumerate().find(|(_, owner)| {
            owner.image_owner.is_none()
                && owner.profile.image_profile() == profile.image_profile()
                && owner.source.same_selection(&source)
        }) {
            return Ok((
                Self {
                    profile,
                    source,
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
            Some(source.clone()),
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
                source,
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
                let mut images = match Images::from_state(vfs, assets, state) {
                    Ok(images) => images,
                    Err(rejected) => {
                        self.state = Some(Ok(rejected.state));
                        return Err(format!("model image source: {:?}", rejected.error));
                    }
                };
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
    files: ModelFiles,
    registration_vfs: SearchView,
    sources: ModelSources,
    shader_indices: ScopedBindings<RegistrationIndex>,
    shaders: Vec<RegisteredShader>,
    shader_bindings: Vec<ShaderProfileBinding>,
    skin_indices: ScopedBindings<RegistrationIndex>,
    skins: Vec<RegisteredSkin>,
    shader_catalogs: scopes::Selections<Result<ShaderCatalog, String>>,
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
    image_transaction_active: Option<ImageRefreshMode>,
    replacement_load: [bool; 2],
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
                .world_admission(world, self.profile, Some(selected), None)
        } else {
            Ok(self)
        }
    }
    pub(crate) fn assets(self) -> &'a Assets {
        &self.owner.assets
    }
    pub(crate) fn choice_nodes(self) -> usize {
        self.owner.catalog.max_choice_nodes()
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
        let presentation = self.world_presentation(world, Some(profile), selected, None)?;
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
        search: Option<&SearchView>,
    ) -> Option<qa_render::material::world_load::LoadedWorldPresentation> {
        self.select_world_presentation(
            world,
            profile,
            selected,
            search,
            self.catalog.admitted_assets,
        )
    }
    pub(crate) fn presentation_hint(
        self,
        selected: Option<WorldPresentationId>,
        search: Option<&SearchView>,
    ) -> Option<WorldPresentationId> {
        selected.filter(|&id| {
            search.is_none_or(|search| {
                self.assets
                    .world_presentation(id)
                    .is_some_and(|entry| entry.accepts_source(search))
            })
        })
    }
    fn select_world_presentation(
        self,
        world: WorldId,
        profile: Option<ModelProfile>,
        selected: Option<WorldPresentationId>,
        search: Option<&SearchView>,
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
            return if world.presentations().contains(&selected)
                && self.presentation_hint(Some(selected), search).is_some()
            {
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
                self.assets
                    .world_presentation(entry.id)
                    .is_some_and(|entry| {
                        entry.accepts_source(search.unwrap_or(&self.catalog.registration_vfs))
                    })
                    && entry.profile.rules == profile.rules
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
    palettes: crate::render_settings::PaletteRegistrations,
    file_count: usize,
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
    sources: ModelSources,
    shaders_before: Vec<ShaderCheckpoint>,
    skins_before: Vec<SkinCheckpoint>,
    inline_before: Vec<(usize, Option<ModelFileId>)>,
    images: Vec<Option<ImageRegistrationMark>>,
    reload: Option<ReloadCheckpoint>,
}
struct ReloadCheckpoint {
    mode: ImageRefreshMode,
    models: Vec<(usize, RegistrationState<ModelReady>)>,
    shaders: Vec<(usize, RegistrationState<ShaderRegistration>)>,
    skins: Vec<(usize, usize, RegistrationState<SkinReady>)>,
    derived: Option<(Vec<ModelMaterials>, Vec<SurfaceMaterials>)>,
    images: Vec<(usize, ImageReload)>,
    worlds: Option<WorldImageReload>,
}
impl ReloadCheckpoint {
    fn commit(self, owner: &mut ModelAssets) {
        for (index, receipt) in self.images {
            if let Some(Ok(state)) = &mut owner.catalog.profiles[index].state {
                state.commit_reload(receipt);
            }
        }
        if let Some(worlds) = self.worlds {
            worlds.commit(&mut owner.assets);
        }
    }

    fn replay(&self, vfs: &Vfs, owner: &mut ModelAssets) -> Result<(), String> {
        if self.mode == ImageRefreshMode::Reload {
            for (index, receipt) in &self.images {
                owner.catalog.profiles[*index].with_images(vfs, &mut owner.assets, |images| {
                    images
                        .replay_reload(receipt)
                        .map_err(|error| format!("image replay: {error:?}"))
                })?;
            }
        }
        if let Some(worlds) = &self.worlds {
            worlds
                .replay(vfs, &mut owner.assets)
                .map_err(|error| format!("world image replay: {error:?}"))?;
        }
        for &(index, _) in &self.models {
            let entry = &owner.catalog.registrations[index];
            let profile = entry.profile;
            let guard =
                RegistrationVariant::from_collapse(profile, entry.vertex_collapse)?.guard()?;
            let file = entry.file;
            owner
                .ensure_file_record(vfs, file, Some(profile), guard)
                .map_err(|error| format!("model replay: {}", error.message))?;
            if owner.catalog.registrations[index].state.ready().is_none() {
                return Err("model replay did not restore its registered presentation".to_owned());
            }
        }
        for &(skin, row, _) in &self.skins {
            let entry = &owner.catalog.skins[skin];
            let profile = entry.profiles[row].profile;
            let guard =
                RegistrationVariant::from_collapse(profile, entry.profiles[row].vertex_collapse)?
                    .guard()?;
            let name = owner
                .catalog
                .names
                .get(entry.name)
                .ok_or("skin replay filename")?
                .to_vec();
            let request = entry.profiles[row].request;
            let search = entry.search.clone();
            let path = std::str::from_utf8(&name).map_err(|_| "skin replay filename")?;
            if let Err(error) =
                owner.admit_skin_request(vfs, path, profile, guard, request, Some(&search))
            {
                // Source absence or malformed source is a retained native fallback.
                // Material, capacity and backend errors are transaction failures.
                let source_failure = matches!(request, SkinRequest::Indexed(indexed)
                    if matches!(&owner.catalog.skins[skin].source,
                        Some(SkinSource::IndexedFailure { layout, .. }) if *layout == indexed.layout));
                if self.mode == ImageRefreshMode::IndexedSkins && source_failure {
                    continue;
                }
                return Err(format!("skin replay: {error}"));
            }
            if owner.catalog.skins[skin].profiles[row]
                .state
                .ready()
                .is_none()
            {
                return Err("skin replay did not restore its registered presentation".to_owned());
            }
        }
        for &(index, _) in &self.shaders {
            let binding = &owner.catalog.shader_bindings[index];
            let profile = binding.profile;
            let guard =
                RegistrationVariant::from_collapse(profile, binding.vertex_collapse)?.guard()?;
            let lighting = binding.lighting;
            let request = binding.request.clone().ok_or("shader replay request")?;
            let search = owner.catalog.shaders[binding.shader.0 as usize]
                .search
                .clone();
            owner
                .admit_shader_profile(
                    vfs,
                    request.name.as_bytes(),
                    profile.source_rules,
                    request.mipmaps,
                    lighting,
                    Some(profile),
                    guard,
                    Some(&search),
                )
                .map_err(|error| format!("shader replay: {error:?}"))?;
            if owner.catalog.shader_bindings[index].state.ready().is_none() {
                return Err("shader replay did not restore its registered presentation".to_owned());
            }
        }
        // This prefix is visible only inside the callback-free cold transaction.
        // Its caller still prepares and publishes the complete backend candidate.
        let counts = owner.assets.counts();
        owner
            .validate_admission(counts)
            .map_err(|error| format!("image replay admission: {error:?}"))?;
        owner.apply_admission(counts);
        Ok(())
    }

    fn restore(self, owner: &mut ModelAssets) -> Result<(), String> {
        if let Some(worlds) = self.worlds {
            worlds.restore(&mut owner.assets).map_err(str::to_owned)?;
        }
        for (index, receipt) in self.images {
            let Some(Ok(state)) = &mut owner.catalog.profiles[index].state else {
                return Err("image replay owner unavailable".to_owned());
            };
            state.restore_reload(receipt, &mut owner.assets)?;
        }
        if let Some((bindings, surfaces)) = self.derived {
            owner.catalog.bindings = bindings;
            owner.catalog.surfaces = surfaces;
        }
        for (index, state) in self.models {
            owner.catalog.registrations[index].state = state;
        }
        for (index, state) in self.shaders {
            owner.catalog.shader_bindings[index].state = state;
        }
        for (skin, row, state) in self.skins {
            owner.catalog.skins[skin].profiles[row].state = state;
        }
        Ok(())
    }
}
struct ShaderCheckpoint {
    index: usize,
    profiles: usize,
    first_request_rows: [Option<usize>; 5],
    bindings: Vec<(usize, Option<ShaderRequest>)>,
}
struct ModelCheckpoint {
    name: ModelFileId,
    file: ModelFile,
    profiles: Vec<usize>,
    unattempted: Vec<usize>,
}
struct SkinCheckpoint {
    index: usize,
    profiles: usize,
    unattempted: Vec<usize>,
    source: Option<SkinSource>,
    reserved: Vec<Option<SkinReady>>,
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
            models: owner.catalog.files.all_rows().map(Vec::len).collect(),
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
        for (rows, length) in owner.catalog.files.all_rows_mut().zip(self.models) {
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
    fn begin_reload(
        &mut self,
        owner: &mut ModelAssets,
        mode: ImageRefreshMode,
    ) -> Result<(), String> {
        let mut selected = Vec::new();
        let mut ready_skins = 0usize;
        if mode == ImageRefreshMode::IndexedSkins {
            for skin in &owner.catalog.skins {
                for row in &skin.profiles {
                    let SkinRequest::Indexed(request) = row.request else {
                        continue;
                    };
                    if !matches!(
                        row.state,
                        RegistrationState::Ready { .. } | RegistrationState::Failed(_)
                    ) {
                        continue;
                    }
                    ready_skins += usize::from(row.state.ready().is_some());
                    let filename = owner
                        .catalog
                        .names
                        .get(skin.name)
                        .ok_or("indexed reload filename")?;
                    let filename =
                        std::str::from_utf8(filename).map_err(|_| "indexed reload filename")?;
                    if let Some(name) = owner.assets.find_name(filename) {
                        selected.push(IndexedSkinReload {
                            name,
                            source: 0,
                            layout: request.layout,
                            translation: request.translation,
                        });
                    }
                }
            }
        } else if mode != ImageRefreshMode::Reload {
            return Err("invalid image replay mode".to_owned());
        }
        let skin_capacity = if mode == ImageRefreshMode::IndexedSkins {
            owner
                .catalog
                .capacity
                .skins
                .checked_sub(ready_skins)
                .ok_or("indexed reload skin capacity")?
        } else {
            0
        };
        let worlds = if mode == ImageRefreshMode::Reload {
            Some(
                owner
                    .assets
                    .begin_world_image_reload()
                    .map_err(str::to_owned)?,
            )
        } else {
            None
        };
        let catalog = &mut owner.catalog;
        let mut images = Vec::new();
        for (index, profile) in catalog.profiles.iter_mut().enumerate() {
            if profile.image_owner.is_none()
                && let Some(Ok(state)) = &mut profile.state
            {
                let scope = if mode == ImageRefreshMode::Reload {
                    ImageReloadScope::All
                } else {
                    ImageReloadScope::IndexedSkins(&selected)
                };
                images.push((index, state.begin_reload(scope)));
            }
        }
        let mut models = Vec::new();
        let mut shaders = Vec::new();
        if mode == ImageRefreshMode::Reload {
            for (index, entry) in catalog.registrations.iter_mut().enumerate() {
                if entry.state.ready().is_some() {
                    models.push((
                        index,
                        std::mem::replace(&mut entry.state, RegistrationState::Reload),
                    ));
                }
            }
            for (index, entry) in catalog.shader_bindings.iter_mut().enumerate() {
                if entry.state.ready().is_some() {
                    shaders.push((
                        index,
                        std::mem::replace(&mut entry.state, RegistrationState::Reload),
                    ));
                }
            }
        }
        let mut skins = Vec::new();
        for (skin, entry) in catalog.skins.iter_mut().enumerate() {
            let mut clear_source = false;
            for (row, profile) in entry.profiles.iter_mut().enumerate() {
                let selected = if mode == ImageRefreshMode::Reload {
                    profile.state.ready().is_some()
                } else {
                    matches!(profile.request, SkinRequest::Indexed(_))
                        && matches!(
                            profile.state,
                            RegistrationState::Ready { .. } | RegistrationState::Failed(_)
                        )
                };
                if selected {
                    clear_source |= matches!(profile.request, SkinRequest::Indexed(_));
                    if matches!(profile.request, SkinRequest::Indexed(_))
                        && let Some((ready, _)) = profile.state.ready()
                    {
                        profile.reserved = Some(ready.clone());
                    }
                    skins.push((
                        skin,
                        row,
                        std::mem::replace(&mut profile.state, RegistrationState::Reload),
                    ));
                }
            }
            if (mode == ImageRefreshMode::Reload || clear_source)
                && matches!(
                    entry.source,
                    Some(SkinSource::Indexed { .. } | SkinSource::IndexedFailure { .. })
                )
            {
                entry.source = None;
            }
        }
        let derived = if mode == ImageRefreshMode::Reload {
            catalog.capacity.bindings = 0;
            catalog.capacity.surfaces = 0;
            Some((
                std::mem::take(&mut catalog.bindings),
                std::mem::take(&mut catalog.surfaces),
            ))
        } else {
            None
        };
        catalog.capacity.skins = skin_capacity;
        self.reload = Some(ReloadCheckpoint {
            mode,
            models,
            shaders,
            skins,
            derived,
            images,
            worlds,
        });
        Ok(())
    }

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
            file_count: catalog.files.len(),
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
            shaders_before: Vec::new(),
            skins_before: Vec::new(),
            inline_before: Vec::new(),
            images,
            reload: None,
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
        let search = request.search.as_ref();
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
                file_name = catalog.inline[slot].handle;
            }
            RendererRegistration::Model => {
                let path = canonical_path(
                    std::str::from_utf8(raw).map_err(|_| "model name is not UTF-8")?,
                );
                file_name = catalog.file_id(path.as_bytes(), search);
            }
            RendererRegistration::Skin => {
                skin_index = find_skin_index(
                    &catalog.names,
                    &catalog.skin_indices,
                    raw,
                    search.unwrap_or(&catalog.registration_vfs),
                );
            }
            RendererRegistration::Shader { .. } => {
                shader_index = find_shader_index(
                    &catalog.names,
                    &catalog.shader_indices,
                    raw,
                    search.unwrap_or(&catalog.registration_vfs),
                );
            }
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
                first_request_rows: shader.first_request_rows,
                bindings: shader
                    .profiles
                    .iter()
                    .copied()
                    .filter(|&index| {
                        let binding = &catalog.shader_bindings[index];
                        binding.state.pending()
                    })
                    .map(|index| (index, catalog.shader_bindings[index].request.clone()))
                    .collect(),
            });
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
                    .filter_map(|(index, profile)| profile.state.pending().then_some(index))
                    .collect(),
                source: catalog.skins[index].source.clone(),
                reserved: catalog.skins[index]
                    .profiles
                    .iter()
                    .map(|row| row.reserved.clone())
                    .collect(),
            });
        }
        if let Some(file) = file_name {
            self.capture_file(catalog, file);
        }
        if let Some((slot, handle)) = inline
            && !self.inline_before.iter().any(|&(saved, _)| saved == slot)
        {
            self.inline_before.push((slot, handle));
        }
        Ok(())
    }

    fn capture_file(&mut self, catalog: &ModelCatalog, name: ModelFileId) {
        if self.files.iter().any(|saved| saved.name == name) {
            return;
        }
        let profiles = catalog.files.rows(name).clone();
        self.files.push(ModelCheckpoint {
            name,
            file: catalog.files[name].clone(),
            unattempted: profiles
                .iter()
                .copied()
                .filter(|&index| catalog.registrations[index].state.pending())
                .collect(),
            profiles,
        });
    }
    fn capture_all(&mut self, owner: &ModelAssets) -> Result<(), String> {
        let catalog = &owner.catalog;
        for (index, file) in catalog.files.iter().enumerate() {
            let id = ModelFileId(index as u32);
            if !matches!(file, ModelFile::Empty) || !catalog.files.rows(id).is_empty() {
                self.capture_file(catalog, id);
            }
        }
        for shader in &catalog.shaders {
            self.capture_target(
                owner,
                &RendererRequest {
                    search: Some(shader.search.clone()),
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
        for (search, name, _) in catalog.skin_indices.all_entries() {
            self.capture_target(
                owner,
                &RendererRequest {
                    search: Some(search.clone()),
                    name: catalog.names.get(name).ok_or("skin name")?,
                    source: RuleSetId::Quake3,
                    kind: RendererRegistration::Skin,
                    world: None,
                },
            )?;
        }
        self.inline_before = catalog
            .inline
            .iter()
            .enumerate()
            .map(|(index, entry)| (index, entry.handle))
            .collect();
        Ok(())
    }

    fn rollback(
        mut self,
        owner: &mut ModelAssets,
        mode: RegistrationRollback,
    ) -> Result<(), String> {
        if let Some(reload) = self.reload.take() {
            reload.restore(owner)?;
        }
        for (profile, mark) in owner.catalog.profiles.iter_mut().zip(&self.images) {
            if let (Some(Ok(state)), Some(mark)) = (&mut profile.state, mark) {
                state.rollback_registration(*mark, self.assets.images, &mut owner.assets)?;
            }
        }
        owner.catalog.profiles.truncate(self.images.len());
        owner.assets.discard_registration_suffix(self.assets)?;
        owner.palettes.restore(self.palettes);
        let catalog = &mut owner.catalog;
        catalog
            .sources
            .rollback(self.sources, mode, self.assets, self.tags);
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
            for index in self.file_count..catalog.files.len() {
                let id = ModelFileId(index as u32);
                catalog.files[id] = ModelFile::Empty;
                catalog.files.rows_mut(id).clear();
            }
            catalog
                .shader_indices
                .discard_suffix(self.shaders)
                .map_err(|_| "shader binding rollback")?;
            catalog
                .skin_indices
                .discard_suffix(self.skins)
                .map_err(|_| "skin binding rollback")?;
        } else {
            // Keep negative metadata, but no value into a discarded resource suffix.
            for entry in &mut catalog.registrations[self.registrations..] {
                entry.state.rollback(mode);
            }
            for entry in &mut catalog.shader_bindings[self.shader_bindings..] {
                entry.state.rollback(mode);
                if entry.state.pending() {
                    entry.request = None;
                }
            }
            for shader in &mut catalog.shaders[self.shaders..] {
                shader.first_request_rows = [None; 5];
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
            catalog.shaders[shader.index].first_request_rows = shader.first_request_rows;
            if matches!(mode, RegistrationRollback::Retry) {
                catalog.shaders[shader.index]
                    .profiles
                    .truncate(shader.profiles);
            }
            for (index, request) in shader.bindings {
                let binding = &mut catalog.shader_bindings[index];
                binding.state.rollback(mode);
                if binding.state.pending() {
                    binding.request = request;
                }
            }
        }
        for skin in self.skins_before {
            if matches!(mode, RegistrationRollback::Retry) {
                catalog.skins[skin.index].profiles.truncate(skin.profiles);
                catalog.skins[skin.index].source = skin.source;
                for (row, reserved) in catalog.skins[skin.index]
                    .profiles
                    .iter_mut()
                    .zip(skin.reserved)
                {
                    row.reserved = reserved;
                }
            }
            for index in skin.unattempted {
                catalog.skins[skin.index].profiles[index]
                    .state
                    .rollback(mode);
            }
        }
        if matches!(mode, RegistrationRollback::Source) {
            for file in &self.files {
                if matches!(catalog.files[file.name], ModelFile::Failed(_))
                    && matches!(file.file, ModelFile::Ready { renderer_sources, .. } if renderer_sources != 0)
                {
                    catalog.files[file.name] = file.file.clone();
                }
            }
            for index in self
                .files
                .iter()
                .map(|file| file.name.0 as usize)
                .chain(self.file_count..catalog.files.len())
            {
                let index = ModelFileId(index as u32);
                let sources =
                    catalog
                        .files
                        .rows(index)
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
                catalog.files[file.name] = file.file;
                *catalog.files.rows_mut(file.name) = file.profiles;
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
    /// Admit newly available model images before replacing their fallback choices.
    pub(crate) fn downloaded<T>(
        &mut self,
        vfs: &Vfs,
        path: &[u8],
        stage: impl FnOnce(&mut Self, bool) -> Result<Option<T>, String>,
        prepare: impl FnOnce(&Assets, Option<&T>) -> Result<(), String>,
    ) -> Result<(bool, Option<T>), String> {
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        let image = std::str::from_utf8(path)
            .ok()
            .and_then(|path| self.assets.find_name(path));
        let mut targets = Vec::new();
        if let Some(image) = image {
            for (registration, entry) in self.catalog.registrations.iter().enumerate() {
                let Some((ready, _)) = entry.state.ready() else {
                    continue;
                };
                for span in ready
                    .surfaces
                    .into_iter()
                    .flatten()
                    .chain(ready.replacement.as_ref().map(|row| row.surfaces))
                {
                    for surface in &self.catalog.surfaces[span.range()] {
                        for (skin, binding) in surface.skins.range().enumerate() {
                            let materials = self.catalog.bindings[binding];
                            if materials.missing_image != Some(image)
                                || targets.iter().any(|&(_, found, _, _)| found == binding)
                            {
                                continue;
                            }
                            let occurrence = match materials.variants[0].shader {
                                ShaderChoice::Occurrence(occurrence)
                                | ShaderChoice::Converted { occurrence, .. } => occurrence,
                                ShaderChoice::Direct(_) => {
                                    return Err("missing model skin occurrence".to_owned());
                                }
                            };
                            if ready.registrations.get(occurrence.0 as usize).is_none() {
                                return Err("missing model skin receipt".to_owned());
                            }
                            targets.push((registration, binding, skin, occurrence));
                        }
                    }
                }
            }
        }
        let initial = self.initial_registration;
        if !initial && self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("download refresh requires a published resource prefix".to_owned());
        }
        let checkpoint = ProfileCheckpoint::capture(self)?;
        let result: Result<(bool, Option<T>), String> = (|| {
            let image_path = image
                .and_then(|image| self.assets.name(image))
                .map(str::to_owned);
            let mut updates = Vec::with_capacity(targets.len());
            for &(registration, binding, skin, occurrence) in &targets {
                let image_path = image_path.as_deref().ok_or("published model image name")?;
                let entry = &self.catalog.registrations[registration];
                let profile = entry.profile;
                let (ready, _) = entry.state.ready().ok_or("published model registration")?;
                let indexed_palette = ready.indexed_palette;
                let model_path = self
                    .catalog
                    .file_name(entry.file)
                    .ok_or("model filename")?
                    .to_owned();
                let source = self
                    .catalog
                    .files
                    .source_view(self.catalog.registrations[registration].file)
                    .ok_or("model image source")?;
                let image_index = image_profile_index(&self.catalog.profiles, profile, source)
                    .ok_or("model image profile")?;
                let (variants, receipt) =
                    with_image_owner(&mut self.catalog.profiles, image_index, |owner| {
                        let policy = owner.model_policy;
                        owner.with_images(vfs, &mut self.assets, |images| {
                            let image = model_image(
                                Image::External(image_path.as_bytes()),
                                images,
                                profile,
                                indexed_palette,
                                ImageRole::Skin,
                                policy.source.indexed_external_pcx,
                                None,
                            )
                            .map_err(|error| format!("published model skin: {error:?}"))?;
                            let variants = alias_image_materials(
                                images.assets,
                                &model_path,
                                skin,
                                image,
                                policy,
                                false,
                            )?;
                            let receipt = images
                                .assets
                                .shader_registration(image_path.as_bytes(), variants[0])?;
                            Ok((variants, receipt))
                        })
                    })?;
                let mut materials = self.catalog.bindings[binding];
                for (index, choice) in materials.variants.iter_mut().enumerate() {
                    choice.shader = if index == 0 {
                        ShaderChoice::Converted {
                            binding: variants[index],
                            occurrence,
                        }
                    } else {
                        ShaderChoice::Direct(variants[index])
                    };
                }
                materials.missing_image = None;
                updates.push((registration, binding, occurrence, materials, receipt));
            }
            let staged = stage(self, !updates.is_empty())?;
            let counts = self.assets.counts();
            let changed =
                !updates.is_empty() || staged.is_some() || counts != checkpoint.registration.assets;
            if !initial && changed {
                self.validate_admission(counts)
                    .map_err(|error| format!("download publication: {error:?}"))?;
                prepare(&self.assets, staged.as_ref())?;
            }
            // Every fallible check precedes backend admission. Preserve each
            // occurrence so an already active native remap still selects its target.
            for (registration, binding, occurrence, materials, receipt) in updates {
                self.catalog.bindings[binding] = materials;
                if let RegistrationState::Ready { value, .. } =
                    &mut self.catalog.registrations[registration].state
                {
                    value.registrations[occurrence.0 as usize] = receipt;
                }
            }
            if !initial && changed {
                self.apply_admission(counts);
            }
            Ok((changed, staged))
        })();
        match result {
            Ok((changed, staged)) => Ok((self.retry_published_file(path) || changed, staged)),
            Err(error) => {
                checkpoint
                    .rollback(self)
                    .map_err(|rollback| format!("{error}; rollback: {rollback}"))?;
                Err(error)
            }
        }
    }

    fn retry_published_file(&mut self, path: &[u8]) -> bool {
        let Some(name) = self.catalog.names.find_path_bytes(path) else {
            return false;
        };
        let mut changed = false;
        for (file, rows) in self.catalog.files.named_mut(name) {
            if matches!(file, ModelFile::Failed(_)) {
                *file = ModelFile::Empty;
                changed = true;
            }
            for &index in rows {
                let state = &mut self.catalog.registrations[index].state;
                if matches!(state, RegistrationState::Failed(_)) {
                    *state = RegistrationState::Unattempted;
                    changed = true;
                }
            }
        }
        changed |= self.catalog.sources.invalidate(name);
        for index in self.catalog.skin_indices.named(name) {
            let skin = &mut self.catalog.skins[index.0 as usize];
            let mut retry = false;
            for row in &mut skin.profiles {
                if row.request == SkinRequest::Named
                    && matches!(row.state, RegistrationState::Failed(_))
                {
                    row.state = RegistrationState::Unattempted;
                    retry = true;
                }
            }
            if retry {
                if matches!(skin.source, Some(SkinSource::Script(_))) {
                    skin.source = None;
                }
                changed = true;
            }
        }
        changed
    }

    pub fn set_image_search_order(&mut self, order: ImageSearchOrder) {
        self.assets.set_image_search_order(order);
    }

    pub fn set_model_replacement_load(&mut self, q1: bool, q2: bool) {
        self.replacement_load = [q1, q2];
    }

    /// The startup cache moves into the same owner as its palette resources.
    pub fn adopt_palettes(
        &mut self,
        palettes: crate::render_settings::PaletteRegistrations,
    ) -> Result<(), String> {
        if !self.initial_registration
            || self.catalog.admitted_assets.is_some()
            || !self.palettes.is_empty()
            || !palettes.owned_by(&self.catalog.registration_vfs)
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
    pub fn published_palette(
        &self,
        source: RuleSetId,
        search: Option<&SearchView>,
    ) -> Option<PaletteId> {
        let id = self.palettes.get(source, search)?;
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
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        let palette = match crate::render_settings::palette_resource(
            vfs.read_view(None),
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
            self.image_transaction_active.map(|_| profile),
            ui_fullscreen,
            None,
        )
        .map_err(|error| format!("graphic shader: {error:?}"))?;
        self.catalog
            .shader_outcome_candidate_variant(
                name.as_bytes(),
                profile,
                ShaderLighting::TwoDimensional,
                RegistrationVariant::Guard(ui_fullscreen),
                None,
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
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        let index = image_profile_index(
            &self.catalog.profiles,
            profile,
            &self.catalog.registration_vfs,
        )
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
    fn append_image_policy(
        &mut self,
        profile: ModelProfile,
        source: SearchView,
    ) -> Result<Option<qa_render::assets::PreparedPaletteTranslucency>, String> {
        let maximum = self
            .catalog
            .limits
            .models
            .checked_add(1)
            .and_then(|views| views.checked_mul(self.catalog.limits.models))
            .filter(|&count| count <= u32::MAX as usize)
            .ok_or("image source policy capacity")?;
        if self.catalog.profiles.len() >= maximum {
            return Err("image source policy capacity".to_owned());
        }
        self.catalog
            .profiles
            .try_reserve(1)
            .map_err(|_| "image source policy capacity")?;
        let (images, blend) =
            ProfileImages::new(profile, source, &self.catalog.profiles, &mut self.assets)?;
        if let Some(Err(error)) = &images.state {
            return Err(error.clone());
        }
        self.catalog.profiles.push(images);
        Ok(blend)
    }

    fn ensure_source_profiles(
        &mut self,
        source: &SearchView,
        selected: Option<ModelProfile>,
    ) -> Result<(), FileError> {
        let previous = self.catalog.profiles.len();
        for index in 0..previous {
            let profile = self.catalog.profiles[index].profile;
            if selected.is_some_and(|selected| selected != profile)
                || self.catalog.profiles[..index]
                    .iter()
                    .any(|entry| entry.profile == profile)
                || image_profile_index(&self.catalog.profiles, profile, source).is_some()
            {
                continue;
            }
            if let Some(blend) = self
                .append_image_policy(profile, source.clone())
                .map_err(FileError::file)?
            {
                self.assets.install_palette_translucency(blend);
            }
        }
        Ok(())
    }

    fn append_profile(
        &mut self,
        profile: ModelProfile,
    ) -> Result<Option<qa_render::assets::PreparedPaletteTranslucency>, String> {
        if !self.catalog.profiles().any(|current| current == profile)
            && self.catalog.profiles().count() >= self.catalog.limits.models
        {
            return Err("model profile capacity".to_owned());
        }
        let count = self
            .catalog
            .profiles
            .len()
            .checked_add(1)
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
            .files
            .all_rows()
            .zip(self.catalog.files.iter())
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
        let blend = self.append_image_policy(profile, self.catalog.registration_vfs.clone())?;
        for (name, file, rows) in self.catalog.files.entries_mut() {
            if rows.is_empty() && !matches!(file, ModelFile::Ready { .. }) {
                continue;
            }
            rows.push(self.catalog.registrations.len());
            self.catalog.registrations.push(RegisteredProfile {
                file: name,
                profile,
                vertex_collapse: profile.finish.compile(false).collapse_vertex(),
                state: RegistrationState::Unattempted,
            });
        }
        for (index, shader) in self.catalog.shaders.iter_mut().enumerate() {
            shader.profiles.push(self.catalog.shader_bindings.len());
            self.catalog.shader_bindings.push(ShaderProfileBinding {
                shader: RegistrationIndex(index as u32),
                profile,
                vertex_collapse: profile.finish.compile(false).collapse_vertex(),
                lighting: ShaderLighting::TwoDimensional,
                request: None,
                state: RegistrationState::Unattempted,
            });
        }
        for skin in &mut self.catalog.skins {
            skin.profiles.push(SkinProfile {
                request: SkinRequest::Named,
                profile,
                vertex_collapse: profile.finish.compile(false).collapse_vertex(),
                state: RegistrationState::Unattempted,
                reserved: None,
            });
        }
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
        search: Option<&SearchView>,
        prepare: impl FnOnce(&Assets) -> Result<(), String>,
    ) -> Result<(ModelProfile, AssetCounts), String> {
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("profile registration requires a published resource prefix".to_owned());
        }
        let checkpoint = ProfileCheckpoint::capture(self)?;
        let result = (|| {
            let source_palette = crate::render_settings::palette(
                vfs.read_view(search),
                &mut self.assets,
                source_rules,
                &mut self.palettes,
            )?;
            let palette = crate::render_settings::palette(
                vfs.read_view(search),
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
        vfs: &Vfs,
        world_settings: Option<qa_render::world::WorldRenderSettings>,
        mode: ImageRefreshMode,
        operation: impl FnOnce(&mut Self, &[WorldId]) -> Result<T, String>,
    ) -> Result<T, String> {
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        if self.image_transaction_active.is_some() {
            return Err("image refresh is already active".to_owned());
        }
        if mode == ImageRefreshMode::IndexedSkins && world_settings.is_some() {
            return Err("indexed skin refresh cannot change world settings".to_owned());
        }
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("image refresh requires a published resource prefix".to_owned());
        }
        let mut checkpoint = ProfileCheckpoint::capture(self)?;
        checkpoint.registration.capture_all(self)?;
        let mut world_rebuild = None;
        self.image_transaction_active = Some(mode);
        let result = (|| {
            if mode != ImageRefreshMode::Reconfigure {
                checkpoint.registration.begin_reload(self, mode)?;
                checkpoint
                    .registration
                    .reload
                    .as_ref()
                    .ok_or("image reload checkpoint")?
                    .replay(vfs, self)?;
            }
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
        self.image_transaction_active = None;
        match result {
            Ok(ready) => {
                if let Some(reload) = checkpoint.registration.reload.take() {
                    reload.commit(self);
                }
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
        if self.image_transaction_active.is_none() {
            return Err("image refresh requires its cold transaction".to_owned());
        }
        if self.image_transaction_active == Some(ImageRefreshMode::IndexedSkins) {
            return Err("indexed skin refresh cannot reconfigure other resources".to_owned());
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
            let reload =
                (self.image_transaction_active == Some(ImageRefreshMode::Reload)).then_some(old);
            let search = self
                .assets
                .world_presentation(old)
                .and_then(|entry| entry.search())
                .cloned();
            let (loaded, (), _) = self.prepare_world_presentation_value(
                vfs,
                world,
                profile,
                policy,
                ui_fullscreen,
                search.as_ref(),
                reload,
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
        for (name, rows) in self.catalog.files.all_rows().enumerate() {
            for &index in rows {
                let entry = &self.catalog.registrations[index];
                let profile = refresh.profile(entry.profile);
                if profile != entry.profile && entry.state.ready().is_some() {
                    models.push((ModelFileId(name as u32), profile));
                }
            }
        }
        for (file, profile) in models {
            self.prepare_targets(
                vfs,
                [RegistrationTarget::retained(file, profile.source_rules)],
                profile,
                RegistrationVariant::Guard(ui_fullscreen),
                |_| Ok(()),
                |assets, ()| prepare(assets),
            )?;
        }
        let mut skins = Vec::new();
        for (search, name, index) in self.catalog.skin_indices.all_entries() {
            for (row, entry) in self.catalog.skins[index.0 as usize]
                .profiles
                .iter()
                .enumerate()
            {
                let profile = refresh.profile(entry.profile);
                if profile != entry.profile && entry.state.ready().is_some() {
                    let name = self
                        .catalog
                        .names
                        .get(name)
                        .ok_or("image refresh skin name")?
                        .to_vec();
                    skins.push((
                        name,
                        profile,
                        entry.request,
                        SkinSelection {
                            skin: index.0,
                            row: row as u32,
                        },
                        search.clone(),
                    ));
                }
            }
        }
        for (name, profile, mut request, old_selection, search) in skins {
            if request != SkinRequest::Named {
                if let SkinRequest::Indexed(indexed) = &mut request {
                    indexed.translation.target = profile.palette;
                    indexed.translation.palette = profile
                        .source_palette
                        .unwrap_or(indexed.translation.palette);
                }
                let name =
                    std::str::from_utf8(&name).map_err(|_| "derived skin refresh filename")?;
                let selection = self
                    .prepare_skin_request(
                        vfs,
                        name,
                        profile,
                        ui_fullscreen,
                        request,
                        Some(&search),
                        |assets, _| prepare(assets),
                    )
                    .map_err(|error| format!("derived skin refresh: {error}"))?;
                refresh.skins.push((old_selection, selection));
                continue;
            }
            let request = RendererRequest {
                search: Some(search),
                name,
                source: profile.source_rules,
                kind: RendererRegistration::Skin,
                world: None,
            };
            let target = RegistrationTarget::named(&request);
            self.prepare_targets(
                vfs,
                [target],
                profile,
                RegistrationVariant::Guard(ui_fullscreen),
                |_| Ok(()),
                |assets, ()| prepare(assets),
            )?;
        }
        let mut shaders = Vec::new();
        for shader in &self.catalog.shaders {
            for &row in &shader.profiles {
                let binding = &self.catalog.shader_bindings[row];
                let profile = refresh.profile(binding.profile);
                if profile == binding.profile || binding.state.ready().is_none() {
                    continue;
                }
                let request = binding
                    .request
                    .as_ref()
                    .ok_or("image refresh shader request")?
                    .clone();
                let (lighting, world) = match binding.lighting {
                    ShaderLighting::World(Some(previous)) => {
                        let world = self
                            .assets
                            .world_presentation(previous)
                            .ok_or("image refresh shader world")?
                            .world();
                        let Some(next) = refresh.presentation(world, previous) else {
                            continue;
                        };
                        (ShaderLighting::World(Some(next.id)), Some(world))
                    }
                    lighting => (lighting, None),
                };
                shaders.push((
                    RendererRequest {
                        search: Some(shader.search.clone()),
                        name: request.name,
                        source: profile.source_rules,
                        kind: RendererRegistration::Shader {
                            mipmaps: request.mipmaps,
                            lighting,
                        },
                        world,
                    },
                    profile,
                ));
            }
        }
        for (request, profile) in shaders {
            let target = RegistrationTarget::named(&request);
            self.prepare_targets(
                vfs,
                [target],
                profile,
                RegistrationVariant::Guard(ui_fullscreen),
                |_| Ok(()),
                |assets, ()| prepare(assets),
            )?;
        }
        Ok(refresh)
    }

    pub fn new(
        vfs: &Vfs,
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
            let (entry, blend) =
                ProfileImages::new(profile, vfs.default_view(), &unique, &mut assets)?;
            if let Some(blend) = blend {
                assets.install_palette_translucency(blend);
            }
            unique.push(entry);
        }
        let (worlds, inline) = inline_bindings(&assets, worlds, limits)?;
        let sources = ModelSources::load(names.capacity());
        let files = ModelFiles::load(names.capacity(), limits.inline_models)?;
        let shader_indices = ScopedBindings::load(names.capacity(), limits.models);
        let skin_indices = ScopedBindings::load(names.capacity(), limits.models);
        let capacity = ModelCapacity {
            names: 1,
            inline_bindings: inline.len(),
            ..ModelCapacity::default()
        };
        Ok(Self {
            assets,
            palettes: crate::render_settings::PaletteRegistrations::new(vfs),
            initial_registration: true,
            image_transaction_active: None,
            replacement_load: [true; 2],
            catalog: ModelCatalog {
                reader: ArchiveReader::default(),
                names,
                files,
                registration_vfs: vfs.default_view(),
                sources,
                shader_indices,
                shaders: Vec::new(),
                shader_bindings: Vec::new(),
                skin_indices,
                skins: Vec::new(),
                shader_catalogs: scopes::Selections::default(),
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
        let mut pending = Vec::<(Registration, SearchView)>::new();
        for request in requests {
            if request.path.is_empty() || request.path.starts_with('*') {
                continue;
            }
            if pending.len() == limits.models || pending.len() == u32::MAX as usize {
                return Err("model registration capacity".to_owned());
            }
            let search = vfs
                .read_view(request.search)
                .retain()
                .map_err(|error| format!("model source selection: {error:?}"))?;
            pending.push((
                Registration {
                    path: canonical_path(request.path),
                    profile: request.profile(),
                    indexed_palette: request.source_palette,
                    required: request.required,
                },
                search,
            ));
        }
        pending.sort_by(|left, right| left.0.path.cmp(&right.0.path));
        let mut owner = Self::new(
            vfs,
            assets,
            pending.iter().map(|(entry, _)| entry.profile),
            worlds,
            limits,
        )?;
        for (entry, search) in &pending {
            owner.register_request(
                vfs,
                ModelRequest {
                    path: &entry.path,
                    search: Some(search),
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
            let search = request.search.as_ref();
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
                        search,
                    )
                    .is_none()
            {
                continue;
            }
            *candidate = match request.kind {
                RendererRegistration::Model => {
                    if let Some(inline) = self.catalog.renderer_inline(handle) {
                        self.world_admission(inline.world, profile, None, search)?
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
                            search.unwrap_or(&self.catalog.registration_vfs),
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
                                && !row.state.pending()
                        })
                        .ok_or("skin shader membership")?;
                    let registrations = match &row.state {
                        RegistrationState::Ready { value, .. } => value.registrations.as_ref(),
                        RegistrationState::Failed(_) => &[],
                        RegistrationState::Unattempted | RegistrationState::Reload => continue,
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
                            search,
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
        search: Option<&SearchView>,
    ) -> Option<RendererAdmission<'_>> {
        let (_, Some(index)) = find_skin_index(
            &self.catalog.names,
            &self.catalog.skin_indices,
            name,
            search.unwrap_or(&self.catalog.registration_vfs),
        )?
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
            search: search.cloned(),
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
        search: Option<&SearchView>,
    ) -> Option<qa_render::material::world_load::LoadedWorldPresentation> {
        self.view().select_world_presentation(
            world,
            Some(profile),
            self.view().presentation_hint(selected, search),
            search,
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
        search: Option<&SearchView>,
    ) -> Result<RendererAdmission<'_>, String> {
        let id = self
            .renderer_presentation(world, profile, selected, search)
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
        self.prepare_targets(
            vfs,
            requests.map(RegistrationTarget::named),
            profile,
            variant,
            stage,
            prepare,
        )
    }

    fn prepare_targets<T, const N: usize>(
        &mut self,
        vfs: &Vfs,
        targets: [RegistrationTarget<'_>; N],
        profile: ModelProfile,
        variant: RegistrationVariant,
        stage: impl FnOnce([RendererAdmission<'_>; N]) -> Result<T, String>,
        prepare: impl FnOnce(&Assets, &T) -> Result<(), String>,
    ) -> Result<([i32; N], T, AssetCounts), String> {
        let requests = targets.each_ref().map(|target| &target.request);
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
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
        for (request, target) in requests.into_iter().zip(&targets) {
            vfs.read_view(target.request.search.as_ref())
                .retain()
                .map_err(|error| format!("renderer source selection: {error:?}"))?;
            if let Some(file) = target.file {
                self.catalog
                    .files
                    .get(file)
                    .ok_or("renderer model record")?;
            } else if request.name.is_empty() {
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
                self.renderer_presentation(
                    world,
                    profile,
                    Some(presentation),
                    request.search.as_ref(),
                )
                .filter(|entry| {
                    entry.profile.rules == profile.rules
                        && entry.profile.indexed_palette == profile.palette
                        && entry.profile.image_settings == profile.image_settings
                        && entry.profile.finish == profile.finish
                })
                .ok_or("shader registration presentation")?;
            }
        }
        let cached = targets.each_ref().map(|target| match target.file {
            Some(file) => self.catalog.model_cached(file, profile, variant),
            None => self.catalog.registration_cached_variant(
                target.request.kind,
                target.request.name,
                profile,
                target.request.world,
                variant,
                target.request.search.as_ref(),
            ),
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
        if requests
            .iter()
            .zip(cached)
            .enumerate()
            .any(|(index, (request, cached))| {
                cached == Some(0)
                    && match request.kind {
                        RendererRegistration::Shader { lighting, .. } => self
                            .catalog
                            .shader_outcome(
                                request.name.as_ref(),
                                profile,
                                lighting,
                                targets[index].request.search.as_ref(),
                            )
                            .is_none(),
                        _ => true,
                    }
            })
        {
            return Err("renderer registration unavailable for invoking profile".to_owned());
        }
        let ui_fullscreen = variant.guard()?;
        let mut checkpoint = RegistrationCheckpoint::base(self)?;
        for target in &targets {
            match target.file {
                Some(file) => checkpoint.capture_file(&self.catalog, file),
                None => checkpoint.capture_target(self, &target.request)?,
            }
        }
        let mut rollback = RegistrationRollback::Source;
        let result: Result<([i32; N], T, AssetCounts), String> = (|| {
            for (index, request) in requests.into_iter().enumerate() {
                if cached[index].is_some() {
                    continue;
                }
                let raw = request.name.as_ref();
                let selected = self.image_transaction_active.map(|_| profile);
                let handle = match request.kind {
                    RendererRegistration::Model if raw.starts_with(b"*") => request
                        .world
                        .zip(qa_core::text::inline_model_index(raw))
                        .ok_or(CallError::File)
                        .and_then(|(world, index)| self.register_inline(world, index)),
                    RendererRegistration::Model => {
                        if let Some(file) = targets[index].file {
                            self.ensure_file_record(vfs, file, selected, ui_fullscreen)
                                .map_err(|error| error.kind)
                                .and_then(|_| {
                                    i32::try_from(file.0).map_err(|_| CallError::Capacity)
                                })
                        } else {
                            let path =
                                std::str::from_utf8(raw).map_err(|_| "model name is not UTF-8")?;
                            self.ensure_file_profile(
                                vfs,
                                &canonical_path(path),
                                selected,
                                ui_fullscreen,
                                request.search.as_ref(),
                            )
                            .map_err(|error| error.kind)
                            .and_then(|_| {
                                self.catalog
                                    .file_id(raw, request.search.as_ref())
                                    .and_then(|file| i32::try_from(file.0).ok())
                                    .ok_or(CallError::File)
                            })
                        }
                    }
                    RendererRegistration::Skin => self.admit_skin_profile(
                        vfs,
                        raw,
                        request.source,
                        selected,
                        ui_fullscreen,
                        targets[index].request.search.as_ref(),
                    ),
                    RendererRegistration::Shader { mipmaps, lighting } => self
                        .admit_shader_profile(
                            vfs,
                            raw,
                            request.source,
                            mipmaps,
                            lighting,
                            selected,
                            ui_fullscreen,
                            targets[index].request.search.as_ref(),
                        ),
                }
                .map_err(|error| format!("renderer registration: {error:?}"))?;
                let candidate = match request.kind {
                    RendererRegistration::Shader { lighting, .. } => self
                        .catalog
                        .shader_outcome_candidate_variant(
                            raw,
                            profile,
                            lighting,
                            variant,
                            targets[index].request.search.as_ref(),
                        )
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
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
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
        search: Option<&SearchView>,
        stage: impl FnOnce(
            ModelView<'_>,
            &qa_render::material::world_load::LoadedWorld,
        ) -> Result<T, String>,
        prepare: impl FnOnce(&Assets, &T) -> Result<(), String>,
    ) -> Result<(qa_render::material::world_load::LoadedWorld, T, AssetCounts), String> {
        self.prepare_world_presentation_value(
            vfs,
            world,
            profile,
            presentation,
            ui_fullscreen,
            search,
            None,
            stage,
            prepare,
        )
    }

    fn prepare_world_presentation_value<T>(
        &mut self,
        vfs: &Vfs,
        world: WorldId,
        profile: ModelProfile,
        presentation: qa_render::material::world_load::WorldPresentationProfile,
        ui_fullscreen: bool,
        search: Option<&SearchView>,
        reload: Option<WorldPresentationId>,
        stage: impl FnOnce(
            ModelView<'_>,
            &qa_render::material::world_load::LoadedWorld,
        ) -> Result<T, String>,
        prepare: impl FnOnce(&Assets, &T) -> Result<(), String>,
    ) -> Result<(qa_render::material::world_load::LoadedWorld, T, AssetCounts), String> {
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("world presentation requires a published resource prefix".to_owned());
        }
        self.validate_world_profile(profile, presentation)?;
        let native_default = self.catalog.native_default(profile);
        let checkpoint = RegistrationCheckpoint::base(self)?;
        let result = (|| {
            let request = WorldPresentationRequest {
                profile: presentation,
                native_default,
                search,
            };
            let loaded = match reload {
                Some(old) => qa_render::material::world_load::reload_world_presentation(
                    vfs,
                    &mut self.assets,
                    world,
                    old,
                    request,
                    ui_fullscreen,
                ),
                None => qa_render::material::world_load::append_world_presentation(
                    vfs,
                    &mut self.assets,
                    world,
                    request,
                    ui_fullscreen,
                ),
            }
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
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        if self.initial_registration || self.catalog.admitted_assets != Some(self.assets.counts()) {
            return Err("world registration requires a published resource prefix".to_owned());
        }
        let WorldRegistration {
            search,
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
                search: Some(input.search().clone()),
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
                vfs.read_view(Some(input.search())),
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
                    search: search.as_ref(),
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
                vfs,
                &loaded.virtual_path,
                loaded.render.world,
                Some(loaded.collision),
                loaded.collision_bounds,
                Some(loaded.native_checksums),
                Some(&loaded.search),
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
            self.catalog.capacity.names = self.catalog.names.len();
            self.catalog.capacity.name_bytes += bytes.len();
        }
        Ok(name)
    }

    fn check_registration_vfs(&self, vfs: &Vfs) -> Result<(), CallError> {
        vfs.accepts_search(&self.catalog.registration_vfs)
            .then_some(())
            .ok_or(CallError::File)
    }

    fn admit_file(
        &mut self,
        path: &[u8],
        search: Option<&SearchView>,
    ) -> Result<ModelFileId, CallError> {
        let name = self.admit_name(path)?;
        let view = search.unwrap_or(&self.catalog.registration_vfs).clone();
        self.catalog.files.admit(Some((name, view)))
    }
    /// Admit the already-loaded world as native model metadata. Its surface
    /// binding and collision bounds do not create a mesh or decode another file.
    pub fn bind_loaded_world(
        &mut self,
        vfs: &Vfs,
        path: &str,
        world: WorldId,
        collision: Option<qa_core::primitives::GeometryId>,
        native_bounds: Bounds,
        native_checksums: Option<qa_formats::bsp::NativeMapChecksums>,
        search: Option<&SearchView>,
    ) -> Result<(), String> {
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        let search = vfs
            .read_view(search)
            .retain()
            .map_err(|error| format!("world source selection: {error:?}"))?;
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
            .admit_file(path.as_bytes(), Some(&search))
            .map_err(|_| "model name capacity".to_owned())?;
        if !matches!(self.catalog.files[name], ModelFile::Empty) {
            return match self.catalog.files.get(name) {
                Some(ModelFile::LoadedWorld {
                    binding: existing,
                    bounds,
                    collision: existing_collision,
                    native_checksums: existing_checksum,
                }) if *existing == binding
                    && *bounds == native_bounds
                    && *existing_collision == collision
                    && *existing_checksum == native_checksums =>
                {
                    Ok(())
                }
                _ => Err("loaded world filename conflict".to_owned()),
            };
        }
        self.catalog.files[name] = ModelFile::LoadedWorld {
            binding,
            bounds: native_bounds,
            collision,
            native_checksums,
        };
        Ok(())
    }

    /// Static optional requests and native synchronous requests share exactly
    /// one file admission, decode and registration path.
    pub fn register_request(&mut self, vfs: &Vfs, request: ModelRequest<'_>) -> Result<(), String> {
        self.check_registration_vfs(vfs)
            .map_err(|error| format!("renderer VFS owner: {error:?}"))?;
        let search = vfs
            .read_view(request.search)
            .retain()
            .map_err(|error| format!("model source selection: {error:?}"))?;
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
        let name = match self.catalog.file_id(path.as_bytes(), Some(&search)) {
            Some(name) => name,
            None => self
                .admit_file(path.as_bytes(), Some(&search))
                .map_err(|error| format!("model name capacity: {error:?}"))?,
        };
        let admission = self.ensure_file_record(vfs, name, None, false);
        if matches!(self.catalog.files[name], ModelFile::LoadedWorld { .. }) {
            return Ok(());
        }
        if let Err(error) = admission
            && !self
                .catalog
                .files
                .rows(name)
                .iter()
                .copied()
                .any(|index| !self.catalog.registrations[index].state.pending())
        {
            if !request.required && matches!(error.kind, CallError::File) {
                return Ok(());
            }
            return Err(context(&error.message));
        }
        let profile = request.profile();
        let registration = self
            .catalog
            .files
            .rows(name)
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

    fn model_bytes(&mut self, vfs: ReadView<'_>, path: &str) -> Result<Arc<[u8]>, FileError> {
        let reference = vfs.open(path.as_bytes()).ok_or_else(|| FileError {
            kind: CallError::File,
            message: format!("missing model source {path}"),
        })?;
        self.model_bytes_at(vfs, path, &reference)
    }

    fn model_bytes_at(
        &mut self,
        vfs: ReadView<'_>,
        path: &str,
        reference: &qa_content::vfs::FileRef,
    ) -> Result<Arc<[u8]>, FileError> {
        vfs.read_to_vec(
            reference,
            self.catalog.limits.file_bytes,
            &mut self.catalog.reader,
        )
        .map(Arc::from)
        .map_err(|error| FileError {
            kind: CallError::File,
            message: format!("{path}: {error}"),
        })
    }

    fn md5_source(
        &mut self,
        vfs: ReadView<'_>,
        path: &str,
        mesh: &Model<'_>,
    ) -> Result<(Md5Source, Bounds, Arc<[f32]>), FileError> {
        let stem = file_stem(path);
        let animation_path = format!("{stem}.md5anim");
        let animation_bytes = self.model_bytes(vfs, &animation_path)?;
        let scale_path = format!("{stem}.md5scale");
        let mut diagnostics = Vec::new();
        let scale_bytes = if vfs.open(scale_path.as_bytes()).is_some() {
            match self.model_bytes(vfs, &scale_path) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    diagnostics.push(error.message);
                    None
                }
            }
        } else {
            None
        };
        let animation = Md5Animation::parse(&animation_bytes, scale_bytes.as_deref())
            .and_then(|animation| {
                animation.validate_mesh(mesh)?;
                Ok(animation)
            })
            .map_err(|error| FileError {
                kind: CallError::File,
                message: format!("{animation_path}: {error:?}"),
            })?;
        diagnostics.extend(
            animation
                .scale_diagnostics
                .iter()
                .map(|diagnostic| format!("{scale_path}: {diagnostic:?}")),
        );
        let mut bounds = animation.frames[0].bounds;
        for frame in &animation.frames[1..] {
            bounds.add_bounds(frame.bounds);
        }
        let radii = animation
            .frames
            .iter()
            .map(|frame| frame.bounds.radius())
            .collect();
        let skeleton = Skeleton::new(
            animation.hierarchy.len() as u32,
            animation.poses.into_boxed_slice(),
        )
        .map_err(|error| FileError {
            kind: CallError::File,
            message: format!("{animation_path}: {error}"),
        })?;
        Ok((
            Md5Source {
                _animation_bytes: animation_bytes,
                _scale_bytes: scale_bytes,
                skeleton: Arc::new(skeleton),
                diagnostics: diagnostics.into(),
            },
            bounds,
            radii,
        ))
    }

    fn model_source_view(&self, vfs: ReadView<'_>) -> Result<SearchView, FileError> {
        if !vfs.accepts_search(&self.catalog.registration_vfs) {
            return Err(FileError::file(
                "model source VFS owner mismatch".to_owned(),
            ));
        }
        vfs.retain()
            .map_err(|_| FileError::file("model source selection".to_owned()))
    }

    fn cached_model_source(&self, view: &SearchView, path: &[u8]) -> Option<ModelSourceId> {
        let name = self.catalog.names.find_path_bytes(path)?;
        let source = self.catalog.sources.find(view, name)?;
        (!matches!(self.catalog.sources[source], ModelSourceFile::Empty)).then_some(source)
    }
    fn model_source(
        &mut self,
        vfs: ReadView<'_>,
        path: &str,
    ) -> Result<Option<ModelSourceId>, FileError> {
        let view = self.model_source_view(vfs)?;
        if let Some(name) = self.cached_model_source(&view, path.as_bytes()) {
            return Ok(Some(name));
        }
        let Some(reference) = vfs.open(path.as_bytes()) else {
            return Ok(None);
        };
        let precedence = vfs
            .source_precedence(&reference)
            .map_err(|error| FileError {
                kind: CallError::File,
                message: format!("model source origin: {error}"),
            })?;
        let name = self
            .admit_name(path.as_bytes())
            .map_err(|_| FileError::capacity("model source name capacity"))?;
        let name = self.catalog.sources.intern(view, name)?;
        let bytes = self.model_bytes_at(vfs, path, &reference)?;
        let model = match Model::parse(&bytes) {
            Ok(model) => model,
            Err(error) => {
                self.catalog.sources[name] = ModelSourceFile::Invalid {
                    supports_lods: bytes.starts_with(b"IDP3") || bytes.starts_with(b"IDPC"),
                    error: FileError {
                        kind: CallError::File,
                        message: format!("decode: {error:?}"),
                    },
                };
                return Ok(Some(name));
            }
        };
        let (skeletal, bounds, radii) = if model.format == ModelFormat::Md5Mesh {
            match self.md5_source(vfs, path, &model) {
                Ok((skeletal, bounds, radii)) => (Some(skeletal), bounds, radii),
                Err(error) => {
                    self.catalog.sources[name] = ModelSourceFile::Invalid {
                        supports_lods: false,
                        error,
                    };
                    return Ok(Some(name));
                }
            }
        } else {
            (
                None,
                model.bounds,
                model
                    .frames
                    .iter()
                    .map(|frame| frame.bounds.radius())
                    .collect::<Arc<[_]>>(),
            )
        };
        let metadata = ModelMetadata {
            format: model.format,
            bounds,
            renderer_bounds: if supports_lods(model.format) {
                model.frames[0].bounds
            } else {
                bounds
            },
        };
        let animation = model_animation(&model).map_err(|message| FileError {
            kind: CallError::File,
            message,
        })?;
        let tags = self.admit_tags(&model).map_err(FileError::file)?;
        let meshes = vec![
            None;
            if matches!(model.format, ModelFormat::Spr | ModelFormat::Sp2) {
                1
            } else {
                model.meshes.len()
            }
        ];
        let skin_extent = (model.format == ModelFormat::Mdl)
            .then(|| model.skins.first())
            .flatten()
            .and_then(|skin| match skin {
                Image::Indexed { width, height, .. } => Some([*width, *height]),
                Image::External(_) => None,
            });
        let indexed_skins = if model.format == ModelFormat::Mdl {
            model
                .skins
                .iter()
                .map(|skin| {
                    let Image::Indexed { pixels, .. } = skin else {
                        return Err(FileError::capacity("native indexed skin source"));
                    };
                    retained_model_span(&bytes, pixels)
                })
                .collect::<Result<Arc<[_]>, FileError>>()?
        } else {
            Arc::from([])
        };
        let source_images = model
            .skins
            .iter()
            .chain(model.sprites.iter().map(|sprite| &sprite.image))
            .filter_map(|image| match image {
                Image::External(name) => Some(*name),
                Image::Indexed { .. } => None,
            })
            .map(|image| retained_model_span(&bytes, image))
            .collect::<Result<Arc<[_]>, FileError>>()?;
        drop(model);
        self.catalog.sources[name] = ModelSourceFile::Ready {
            bytes,
            precedence,
            metadata,
            skin_extent,
            indexed_skins,
            source_images,
            animation,
            skeletal,
            replacement: ReplacementState::Unattempted,
            radii,
            tags,
            meshes,
        };
        Ok(Some(name))
    }

    pub(crate) fn source_image(
        &mut self,
        vfs: ReadView<'_>,
        path: &[u8],
        ordinal: u32,
    ) -> Result<Option<&[u8]>, String> {
        let view = self.model_source_view(vfs).map_err(|error| error.message)?;
        let name = if let Some(name) = self.cached_model_source(&view, path) {
            name
        } else {
            let path = canonical_path(
                std::str::from_utf8(path).map_err(|_| "model dependency path encoding")?,
            );
            let Some(name) = self
                .model_source(vfs, &path)
                .map_err(|error| error.message)?
            else {
                return Ok(None);
            };
            name
        };
        match &self.catalog.sources[name] {
            ModelSourceFile::Ready {
                bytes,
                source_images,
                ..
            } => Ok(source_images
                .get(ordinal as usize)
                .and_then(|span| bytes.get(span.range()))),
            ModelSourceFile::Invalid { error, .. } => Err(error.message.clone()),
            ModelSourceFile::Empty => Ok(None),
        }
    }

    fn model_lods(&mut self, vfs: ReadView<'_>, path: &str) -> Result<ModelLods, FileError> {
        let stem = file_stem(path);
        let preferred = match path[stem.len()..].strip_prefix('.') {
            Some(extension) if extension.eq_ignore_ascii_case("md3") => "md3",
            Some(extension) if extension.eq_ignore_ascii_case("mdc") => "mdc",
            _ => {
                let name = self.model_source(vfs, path)?.ok_or_else(|| FileError {
                    kind: CallError::File,
                    message: format!("missing model {path}"),
                })?;
                match &self.catalog.sources[name] {
                    ModelSourceFile::Ready { metadata, .. } if supports_lods(metadata.format) => {
                        if metadata.format == ModelFormat::Mdc {
                            "mdc"
                        } else {
                            "md3"
                        }
                    }
                    ModelSourceFile::Ready { .. } => {
                        return Ok(ModelLods {
                            slots: [
                                ModelLodSlot::Source(name),
                                ModelLodSlot::Missing,
                                ModelLodSlot::Missing,
                            ],
                            native_count: 1,
                            supports_lods: false,
                        });
                    }
                    ModelSourceFile::Invalid { error, .. } => return Err(error.clone()),
                    ModelSourceFile::Empty => {
                        return Err(FileError::capacity("model source unavailable"));
                    }
                }
            }
        };
        let alternate = if preferred == "md3" { "mdc" } else { "md3" };
        let mut lods = ModelLods {
            slots: [ModelLodSlot::Missing; 3],
            native_count: 0,
            supports_lods: true,
        };
        for slot in (0..3).rev() {
            let filename = if slot == 0 {
                path.to_owned()
            } else {
                format!("{stem}_{slot}.{preferred}")
            };
            let name = match self.model_source(vfs, &filename)? {
                Some(name) => Some(name),
                None => {
                    let alternate = if slot == 0 {
                        format!("{stem}.{alternate}")
                    } else {
                        format!("{stem}_{slot}.{alternate}")
                    };
                    self.model_source(vfs, &alternate)?
                }
            };
            let Some(name) = name else {
                continue;
            };
            match &self.catalog.sources[name] {
                ModelSourceFile::Ready { metadata, .. } if supports_lods(metadata.format) => {
                    lods.slots[slot] = ModelLodSlot::Source(name);
                    lods.native_count += 1;
                }
                ModelSourceFile::Invalid {
                    error,
                    supports_lods: true,
                } if slot != 0 => {
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
                        message: format!(
                            "unknown vertex model identifier: {}",
                            self.catalog.source_name(name).unwrap_or(&filename)
                        ),
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

    fn replacement_source(
        &mut self,
        vfs: ReadView<'_>,
        source: ModelSourceId,
    ) -> Option<ReplacementSource> {
        let ModelSourceFile::Ready {
            bytes,
            metadata,
            precedence,
            replacement,
            ..
        } = &self.catalog.sources[source]
        else {
            return None;
        };
        let q1 = metadata.format == ModelFormat::Mdl;
        if !matches!(metadata.format, ModelFormat::Mdl | ModelFormat::Md2)
            || !self.replacement_load[usize::from(!q1)]
        {
            return None;
        }
        match replacement {
            ReplacementState::Ready(replacement) => return Some(replacement.clone()),
            ReplacementState::Unavailable(_) => return None,
            _ => (),
        }
        let bytes = Arc::clone(bytes);
        let precedence = precedence.clone();
        let result = (|| -> Result<Option<ReplacementSource>, String> {
            let original =
                Model::parse(&bytes).map_err(|error| format!("replacement source: {error:?}"))?;
            let path = self
                .catalog
                .source_name(source)
                .ok_or("replacement source name")?
                .to_owned();
            let stem = file_stem(&path);
            let candidate = format!(
                "{}.md5mesh",
                if q1 {
                    stem.to_owned()
                } else {
                    md5_directory(stem)
                }
            );
            let Some(candidate) = self
                .model_source(vfs, &candidate)
                .map_err(|error| error.message)?
            else {
                return Ok(None);
            };
            let ModelSourceFile::Ready {
                bytes,
                precedence: candidate_rank,
                metadata,
                ..
            } = &self.catalog.sources[candidate]
            else {
                return Err(match &self.catalog.sources[candidate] {
                    ModelSourceFile::Invalid { error, .. } => error.message.clone(),
                    _ => "replacement source unavailable".to_owned(),
                });
            };
            if !matches!(
                candidate_rank.compare(&precedence),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ) {
                return Ok(None);
            }
            if metadata.format != ModelFormat::Md5Mesh {
                return Err("replacement is not an MD5 mesh".to_owned());
            }
            let candidate_bytes = Arc::clone(bytes);
            let model = Model::parse(&candidate_bytes)
                .map_err(|error| format!("replacement mesh: {error:?}"))?;
            let mut skins = Vec::with_capacity(model.meshes.len());
            for mesh in &model.meshes {
                let mut names = Vec::with_capacity(original.skins.len());
                if q1 {
                    let shader = std::str::from_utf8(
                        mesh.shaders.first().ok_or("replacement mesh shader")?.name,
                    )
                    .map_err(|_| "replacement shader is not UTF-8")?;
                    for (group, description) in original.skin_groups.iter().enumerate() {
                        for member in 0..description.members.len() {
                            let path = format!("progs/{shader}_{group:02}_{member:02}.lmp");
                            let bytes = self
                                .model_bytes(vfs, &path)
                                .map_err(|error| error.message)?;
                            qa_formats::image::decode(
                                &bytes,
                                qa_formats::image::ImageFormat::Lmp,
                                RuleSetId::Quake.lineage(),
                            )
                            .map_err(|error| format!("replacement skin {path}: {error:?}"))?;
                            names.push(
                                self.admit_name(path.as_bytes())
                                    .map_err(|_| "replacement skin name capacity")?,
                            );
                        }
                    }
                } else {
                    for skin in &original.skins {
                        let Image::External(path) = skin else {
                            return Err("MD2 replacement skin source".to_owned());
                        };
                        let path = md5_directory(&canonical_path(
                            std::str::from_utf8(path)
                                .map_err(|_| "replacement skin is not UTF-8")?,
                        ));
                        names.push(
                            self.admit_name(path.as_bytes())
                                .map_err(|_| "replacement skin name capacity")?,
                        );
                    }
                }
                skins.push(names.into_boxed_slice());
            }
            Ok(Some(ReplacementSource {
                source: candidate,
                skins: skins.into(),
            }))
        })();
        let replacement = match result {
            Ok(Some(value)) => ReplacementState::Ready(value),
            Ok(None) => ReplacementState::Unavailable(None),
            Err(error) => ReplacementState::Unavailable(Some(error)),
        };
        let result = match &replacement {
            ReplacementState::Ready(value) => Some(value.clone()),
            _ => None,
        };
        if let ModelSourceFile::Ready {
            replacement: current,
            ..
        } = &mut self.catalog.sources[source]
        {
            *current = replacement;
        }
        result
    }

    fn ensure_file_profile(
        &mut self,
        vfs: &Vfs,
        path: &str,
        selected: Option<ModelProfile>,
        ui_fullscreen: bool,
        search: Option<&SearchView>,
    ) -> Result<Bounds, FileError> {
        self.check_registration_vfs(vfs).map_err(|kind| FileError {
            kind,
            message: "model VFS owner mismatch".to_owned(),
        })?;
        let search = vfs.read_view(search).retain().map_err(|_| FileError {
            kind: CallError::File,
            message: "model source owner mismatch".to_owned(),
        })?;
        let name = match self.catalog.file_id(path.as_bytes(), Some(&search)) {
            Some(name) => name,
            None => self
                .admit_file(path.as_bytes(), Some(&search))
                .map_err(|_| FileError::capacity("model name capacity"))?,
        };
        self.ensure_file_record(vfs, name, selected, ui_fullscreen)
    }

    fn ensure_file_record(
        &mut self,
        vfs: &Vfs,
        name: ModelFileId,
        selected: Option<ModelProfile>,
        ui_fullscreen: bool,
    ) -> Result<Bounds, FileError> {
        self.check_registration_vfs(vfs).map_err(|kind| FileError {
            kind,
            message: "model VFS owner mismatch".to_owned(),
        })?;
        let source_view = self.catalog.files.source_view(name).cloned();
        if matches!(
            self.catalog.files[name],
            ModelFile::Empty | ModelFile::Ready { .. }
        ) && let Some(source) = &source_view
        {
            self.ensure_source_profiles(source, selected)?;
        }
        let file = &self.catalog.files[name];
        let unattempted = !self
            .catalog
            .profiles
            .iter()
            .filter(|images| {
                source_view
                    .as_ref()
                    .is_none_or(|source| images.source.same_selection(source))
            })
            .filter(|images| selected.is_none_or(|profile| images.profile == profile))
            .all(|images| {
                self.catalog.files.rows(name).iter().any(|&row| {
                    let row = &self.catalog.registrations[row];
                    row.profile == images.profile
                        && row.vertex_collapse
                            == row.profile.finish.compile(ui_fullscreen).collapse_vertex()
                })
            })
            || self.catalog.files.rows(name).iter().copied().any(|index| {
                self.catalog.registrations[index].state.pending()
                    && self.catalog.registrations[index].vertex_collapse
                        == self.catalog.registrations[index]
                            .profile
                            .finish
                            .compile(ui_fullscreen)
                            .collapse_vertex()
                    && selected
                        .is_none_or(|profile| self.catalog.registrations[index].profile == profile)
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
        let file_name = self
            .catalog
            .files
            .name(name)
            .ok_or_else(|| FileError::capacity("model file name binding"))?;
        let path = self
            .catalog
            .name(file_name)
            .ok_or_else(|| FileError::capacity("model file name encoding"))?
            .to_owned();
        let source_view =
            source_view.ok_or_else(|| FileError::capacity("model source selection"))?;
        let mut rows = self.catalog.files.rows(name).clone();
        let fresh_profiles = rows.is_empty();
        for images in self
            .catalog
            .profiles
            .iter()
            .filter(|images| images.source.same_selection(&source_view))
        {
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
        *self.catalog.files.rows_mut(name) = rows.clone();
        let lods = match self.catalog.files[name] {
            ModelFile::Ready { lods, .. } => lods,
            _ => match self.model_lods(vfs.read_view(Some(&source_view)), &path) {
                Ok(lods) => lods,
                Err(error) => return Err(self.fail_file(name, fresh_profiles, error)),
            },
        };
        let replacement = lods
            .source(0)
            .and_then(|source| self.replacement_source(vfs.read_view(Some(&source_view)), source));
        let replacement_frames = replacement.as_ref().and_then(|replacement| {
            let original = &self.catalog.sources[lods.source(0)?];
            let ModelSourceFile::Ready {
                metadata,
                radii,
                animation,
                ..
            } = original
            else {
                return None;
            };
            if metadata.format == ModelFormat::Md2 {
                return Some(qa_render::scene::SkeletalFrameMap::Wrap);
            }
            let native = animation
                .as_ref()
                .map_or(radii.len(), |animation| animation.frame_groups().len());
            let ModelSourceFile::Ready {
                skeletal: Some(skeletal),
                ..
            } = &self.catalog.sources[replacement.source]
            else {
                return None;
            };
            (native != skeletal.skeleton.frame_count() as usize)
                .then_some(qa_render::scene::SkeletalFrameMap::Elapsed { rate: 2.0 })
        });
        let mut inputs = Vec::new();
        let source_rows = (0..3)
            .rev()
            .filter_map(|slot| match lods.slots[slot] {
                ModelLodSlot::Source(source) => Some((slot, source)),
                _ => None,
            })
            .chain(
                replacement
                    .as_ref()
                    .map(|replacement| (3, replacement.source)),
            );
        for (slot, source) in source_rows {
            let input = (|| -> Result<_, FileError> {
                let ModelSourceFile::Ready {
                    bytes,
                    meshes,
                    animation,
                    skeletal,
                    ..
                } = &self.catalog.sources[source]
                else {
                    return Err(FileError::capacity("model source unavailable"));
                };
                let bytes = Arc::clone(bytes);
                let animation = animation.clone();
                let skeletal = skeletal.clone();
                let meshes = meshes.clone();
                let replacement_skins = if slot == 3 {
                    Some(
                        replacement
                            .as_ref()
                            .ok_or_else(|| FileError::capacity("replacement source"))?
                            .skins
                            .iter()
                            .map(|names| {
                                names
                                    .iter()
                                    .map(|&name| {
                                        self.catalog.name(name).map(str::to_owned).ok_or_else(
                                            || FileError::capacity("replacement skin name"),
                                        )
                                    })
                                    .collect::<Result<Vec<_>, _>>()
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                } else {
                    None
                };
                let model = match Model::parse(&bytes) {
                    Ok(model) => model,
                    Err(error) => {
                        return Err(FileError::file(format!("decode: {error:?}")));
                    }
                };
                let surfaces = match self.admit_surface_names(&model) {
                    Ok(surfaces) => surfaces,
                    Err(error) => return Err(error),
                };
                if model.tags_per_frame != 0
                    && matches!(
                        self.catalog.sources[source],
                        ModelSourceFile::Ready { tags: None, .. }
                    )
                {
                    let restored = match self.admit_tags(&model) {
                        Ok(tags) => tags,
                        Err(message) => {
                            return Err(FileError::file(message));
                        }
                    };
                    if let ModelSourceFile::Ready { tags, .. } = &mut self.catalog.sources[source] {
                        *tags = restored;
                    }
                }
                drop(model);
                Ok((
                    slot,
                    source,
                    bytes,
                    surfaces,
                    meshes,
                    animation,
                    skeletal,
                    replacement_skins,
                ))
            })();
            match input {
                Ok(input) => inputs.push(input),
                Err(error) if slot == 3 => {
                    if let Some(native) = lods.source(0)
                        && let ModelSourceFile::Ready { replacement, .. } =
                            &mut self.catalog.sources[native]
                    {
                        *replacement = ReplacementState::Unavailable(Some(error.message));
                    }
                }
                Err(error) => return Err(self.fail_file(name, fresh_profiles, error)),
            }
        }
        let shader_catalog = if inputs.iter().any(|input| {
            let source = input.1;
            input.7.is_none()
                && self.catalog.sources[source]
                    .metadata()
                    .is_some_and(|metadata| named_model_shaders(metadata.format))
        }) {
            match self.shader_catalog(vfs, Some(&source_view)) {
                Ok(id) => Some(id),
                Err(kind) => {
                    return Err(self.fail_file(
                        name,
                        fresh_profiles,
                        FileError {
                            kind,
                            message: "model shader catalog source".to_owned(),
                        },
                    ));
                }
            }
        } else {
            None
        };
        let mut renderer_sources = match &self.catalog.files[name] {
            ModelFile::Ready {
                renderer_sources, ..
            } => *renderer_sources,
            _ => 0,
        };
        for &row in &rows {
            let profile_index = image_profile_index(
                &self.catalog.profiles,
                self.catalog.registrations[row].profile,
                &source_view,
            )
            .ok_or_else(|| FileError::capacity("model profile"))?;
            let entry = &mut self.catalog.registrations[row];
            if !entry.state.pending()
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
            for source in inputs.iter().filter_map(|input| input.6.as_ref()) {
                self.catalog
                    .diagnostics
                    .extend(source.diagnostics.iter().map(|error| ModelDiagnostic {
                        name: file_name,
                        profile: entry.profile,
                        error: error.clone(),
                    }));
            }
            if let Some(source) = lods.source(0)
                && let ModelSourceFile::Ready {
                    replacement: ReplacementState::Unavailable(Some(error)),
                    ..
                } = &self.catalog.sources[source]
            {
                self.catalog.diagnostics.push(ModelDiagnostic {
                    name: file_name,
                    profile: entry.profile,
                    error: error.clone(),
                });
            }
            let indexed_palette = if entry.profile.source_palette.is_none()
                && inputs.iter().any(|(_, source, ..)| {
                    self.catalog.sources[*source]
                        .metadata()
                        .is_some_and(|metadata| {
                            matches!(metadata.format, ModelFormat::Mdl | ModelFormat::Spr)
                        })
                }) {
                crate::render_settings::palette_resource(
                    vfs.read_view(None),
                    &mut self.assets,
                    RuleSetId::Quake,
                    &mut self.palettes,
                )
                .map_err(|error| format!("indexed model palette: {error:?}"))
            } else {
                Ok(entry.profile.source_palette)
            };
            let result = indexed_palette.and_then(|indexed_palette| {
                with_image_owner(&mut self.catalog.profiles, profile_index, |images| {
                    let registration = Registration {
                        path: path.to_owned(),
                        profile: images.profile,
                        indexed_palette,
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
                        let mut replacement_error = None;
                        for (slot, _, bytes, _, meshes, animation, skeletal, replacement_skins) in
                            &mut inputs
                        {
                            let model = Model::parse(bytes)
                                .map_err(|error| format!("decode: {error:?}"))?;
                            validate_model(&model)?;
                            let replacement_checkpoint =
                                (*slot == 3).then(|| (self.catalog.capacity, meshes.clone()));
                            let mut compile = |temporary: &mut Images<'_>| {
                                register(
                                    &model,
                                    temporary,
                                    &registration,
                                    model_policy,
                                    native_default,
                                    meshes,
                                    animation.as_ref(),
                                    skeletal.as_ref().map(|source| &source.skeleton),
                                    replacement_skins.as_deref(),
                                    lods.source(0)
                                        .and_then(|source| self.catalog.sources[source].metadata())
                                        .is_some_and(|metadata| {
                                            metadata.format == ModelFormat::Md2
                                        }),
                                    shader_catalog.map(|id| self.catalog.shader_catalogs.get(id)),
                                    &mut self.catalog.capacity,
                                    self.catalog.limits,
                                    compile_policy,
                                )
                                .map_err(FileError::file)
                            };
                            let registered = if replacement_checkpoint.is_some() {
                                temporary.try_registration(compile)
                            } else {
                                compile(temporary)
                            };
                            let mut model = match registered {
                                Ok(model) => model,
                                Err(error) => {
                                    let Some((capacity, old_meshes)) = replacement_checkpoint
                                    else {
                                        return Err(error.message);
                                    };
                                    self.catalog.capacity = capacity;
                                    *meshes = old_meshes;
                                    replacement_error = Some(error.message);
                                    continue;
                                }
                            };
                            let offset = u32::try_from(registrations.len())
                                .map_err(|_| "model shader occurrence capacity")?;
                            for materials in model.surfaces.iter_mut().flatten() {
                                for choice in &mut materials.variants {
                                    choice.shader = match choice.shader {
                                        ShaderChoice::Occurrence(ShaderOccurrence(index)) => {
                                            ShaderChoice::Occurrence(ShaderOccurrence(
                                                index + offset,
                                            ))
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
                        Ok((compiled, registrations, replacement_error, indexed_palette))
                    })
                })
            });
            let images = &self.catalog.profiles[profile_index];
            entry.state = match result {
                Ok((compiled, registrations, replacement_error, indexed_palette)) => {
                    if let Some(error) = replacement_error {
                        self.catalog.diagnostics.push(ModelDiagnostic {
                            name: file_name,
                            profile: entry.profile,
                            error: format!("replacement material: {error}"),
                        });
                    }
                    let mut surfaces = [None; 3];
                    let mut replacement = None;
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
                                wrap_skins: slot != 3 && lods.supports_lods,
                                sprite_frames: !lods.supports_lods
                                    && self.catalog.sources[inputs[0].1].metadata().is_some_and(
                                        |metadata| {
                                            matches!(
                                                metadata.format,
                                                ModelFormat::Spr | ModelFormat::Sp2
                                            )
                                        },
                                    ),
                            });
                            self.catalog.bindings.extend(skins);
                        }
                        if slot == 3 {
                            let native_format = lods
                                .source(0)
                                .and_then(|source| self.catalog.sources[source].metadata())
                                .ok_or_else(|| FileError::capacity("replacement source metadata"))?
                                .format;
                            let rules = match native_format {
                                ModelFormat::Mdl
                                    if entry.profile.source_rules == RuleSetId::QuakeWorld =>
                                {
                                    RuleSetId::QuakeWorld
                                }
                                ModelFormat::Mdl => RuleSetId::Quake,
                                _ if entry.profile.source_rules == RuleSetId::Quake2Rerelease => {
                                    RuleSetId::Quake2Rerelease
                                }
                                _ => RuleSetId::Quake2,
                            };
                            replacement = Some(ReplacementReady {
                                surfaces: IndexSpan { first, count },
                                frames: replacement_frames,
                                rules,
                            });
                        } else {
                            surfaces[slot] = Some(IndexSpan { first, count });
                        }
                    }
                    for slot in (0..3).rev() {
                        if let ModelLodSlot::Alias(next) = lods.slots[slot] {
                            surfaces[slot] = surfaces[usize::from(next)];
                        }
                    }
                    renderer_sources |= 1 << images.profile.source_rules as u8;
                    RegistrationState::Ready {
                        value: ModelReady {
                            indexed_palette,
                            default_shader: images
                                .native_default
                                .map_or(ShaderBindingId(0), |entry| entry.0),
                            surfaces,
                            replacement,
                            registrations: registrations.into_boxed_slice(),
                        },
                        required_assets: (!self.initial_registration).then(|| self.assets.counts()),
                    }
                }
                Err(error) => {
                    self.catalog.diagnostics.push(ModelDiagnostic {
                        name: file_name,
                        profile: entry.profile,
                        error: error.clone(),
                    });
                    RegistrationState::Failed(error)
                }
            };
        }
        for (_, source, _, _, meshes, _, _, _) in inputs {
            if let ModelSourceFile::Ready {
                meshes: current, ..
            } = &mut self.catalog.sources[source]
            {
                *current = meshes;
            }
        }
        self.catalog.files[name] = ModelFile::Ready {
            lods,
            renderer_sources,
        };
        self.catalog.capacity.registrations = self.catalog.registrations.len();
        self.catalog.capacity.bindings =
            self.catalog.bindings.len() * ModelMaterialPolicy::ALL.len();
        self.catalog.capacity.model_choices = self.catalog.calculate_choice_nodes();
        Ok(self.catalog.files[name]
            .bounds(&self.catalog.sources)
            .unwrap_or_default())
    }

    fn fail_file(
        &mut self,
        name: ModelFileId,
        fresh_profiles: bool,
        error: FileError,
    ) -> FileError {
        let Some(file_name) = self.catalog.files.name(name) else {
            return FileError::capacity("model file name binding");
        };
        let rows = self.catalog.files.rows(name);
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
            if entry.state.pending() {
                self.catalog.diagnostics.push(ModelDiagnostic {
                    name: file_name,
                    profile: entry.profile,
                    error: error.message.clone(),
                });
                entry.state = RegistrationState::Failed(error.message.clone());
            }
        }
        if unused_suffix {
            self.catalog.registrations.truncate(rows[0]);
            self.catalog.files.rows_mut(name).clear();
            self.catalog.capacity.registrations = self.catalog.registrations.len();
        }
        self.catalog.files[name] = ModelFile::Failed(error.clone());
        error
    }

    fn admit_surface_names(&mut self, model: &Model<'_>) -> Result<Vec<NameId>, FileError> {
        let generated = (model.format == ModelFormat::Md5Mesh).then(|| {
            (0..model.meshes.len())
                .map(|index| format!("mesh{index}"))
                .collect::<Vec<_>>()
        });
        let raw: Vec<_> = if let Some(names) = &generated {
            names.iter().map(String::as_bytes).collect()
        } else if matches!(model.format, ModelFormat::Spr | ModelFormat::Sp2) {
            vec![b"sprite".as_slice()]
        } else {
            model
                .meshes
                .iter()
                .map(|mesh| {
                    let raw = mesh.name;
                    if supports_lods(model.format) && raw.len() > 2 && raw[raw.len() - 2] == b'_' {
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

    fn shader_catalog(
        &mut self,
        vfs: &Vfs,
        search: Option<&SearchView>,
    ) -> Result<scopes::SelectionId, CallError> {
        self.check_registration_vfs(vfs)?;
        let view = vfs
            .read_view(search)
            .retain()
            .map_err(|_| CallError::File)?;
        self.catalog
            .shader_catalogs
            .intern(view.clone(), self.catalog.limits.models, || {
                qa_render::material::load_catalog(vfs, Some(&view))
                    .map_err(|error| format!("shader scripts: {error:?}"))
            })
            .map_err(|_| CallError::Capacity)
    }

    fn admit_tags(&mut self, model: &Model<'_>) -> Result<Option<TagLayout>, String> {
        if !supports_lods(model.format) || model.tags_per_frame == 0 {
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
        search: Option<&SearchView>,
    ) -> Option<ShaderResource> {
        match kind {
            RendererRegistration::Model if !name.starts_with(b"*") => self
                .catalog
                .file_id(name, search)
                .map(ShaderResource::Model),
            RendererRegistration::Skin => find_skin_index(
                &self.catalog.names,
                &self.catalog.skin_indices,
                name,
                search.unwrap_or(&self.catalog.registration_vfs),
            )
            .and_then(|(_, index)| index)
            .map(|index| ShaderResource::Skin(index + 1)),
            _ => None,
        }
    }
    /// Cold precache through the caller's file selection. Live use still waits
    /// for the shared renderer's resource admission.
    pub fn preload_shader(
        &mut self,
        vfs: &Vfs,
        search: Option<&SearchView>,
        raw: &[u8],
        source: RuleSetId,
        mipmaps: bool,
        lighting: ShaderLighting,
    ) -> Result<i32, CallError> {
        self.admit_shader_profile(vfs, raw, source, mipmaps, lighting, None, false, search)
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
        search: Option<&SearchView>,
    ) -> Result<i32, CallError> {
        self.check_registration_vfs(vfs)?;
        let view = vfs
            .read_view(search)
            .retain()
            .map_err(|_| CallError::File)?;
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
        let cached = find_shader_index(
            &self.catalog.names,
            &self.catalog.shader_indices,
            raw,
            &view,
        )
        .and_then(|(_, index)| index);
        if let Some(index) = cached {
            let reloading = self.catalog.shaders[index as usize]
                .profiles
                .iter()
                .any(|&row| {
                    let entry = &self.catalog.shader_bindings[row];
                    matches_profile(entry.profile)
                        && entry.lighting == lighting
                        && RegistrationVariant::Guard(ui_fullscreen)
                            .matches(entry.profile, entry.vertex_collapse)
                        && matches!(entry.state, RegistrationState::Reload)
                });
            if !reloading
                && let Some(handle) = self.catalog.shader_source_handle(
                    index as i32 + 1,
                    lighting,
                    matches_profile,
                    RegistrationVariant::Guard(ui_fullscreen),
                )
            {
                return Ok(handle);
            }
        }
        std::str::from_utf8(raw).map_err(|_| CallError::Text)?;
        self.ensure_source_profiles(&view, exact)
            .map_err(|error| error.kind)?;
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
                    .checked_add(
                        self.catalog
                            .profiles
                            .iter()
                            .filter(|images| images.source.same_selection(&view))
                            .count(),
                    )
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
                self.catalog
                    .shader_indices
                    .bind(view.clone(), group, RegistrationIndex(index as u32))
                    .map_err(|_| CallError::Capacity)?;
                self.catalog.shaders.push(RegisteredShader {
                    name,
                    search: view.clone(),
                    profiles: (self.catalog.shader_bindings.len()..next).collect(),
                    first_request_rows: [None; 5],
                });
                self.catalog.shader_bindings.extend(
                    self.catalog
                        .profiles
                        .iter()
                        .filter(|images| images.source.same_selection(&view))
                        .map(|images| ShaderProfileBinding {
                            shader: RegistrationIndex(index as u32),
                            vertex_collapse: images
                                .profile
                                .finish
                                .compile(ui_fullscreen)
                                .collapse_vertex(),
                            profile: images.profile,
                            lighting: ShaderLighting::TwoDimensional,
                            request: None,
                            state: RegistrationState::Unattempted,
                        }),
                );
                index
            }
        };
        {
            for images in self
                .catalog
                .profiles
                .iter()
                .filter(|images| images.source.same_selection(&view))
            {
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
                        shader: RegistrationIndex(index as u32),
                        vertex_collapse: images
                            .profile
                            .finish
                            .compile(ui_fullscreen)
                            .collapse_vertex(),
                        profile: images.profile,
                        lighting,
                        request: None,
                        state: RegistrationState::Unattempted,
                    });
                }
            }
        }
        let name = self.catalog.shaders[index].name;
        let ordinary = lighting == ShaderLighting::TwoDimensional;
        let first_request =
            ordinary && self.catalog.shaders[index].first_request_rows[source as usize].is_none();
        let request = if ordinary && !first_request {
            self.catalog.shaders[index]
                .first_request(&self.catalog.shader_bindings, source)
                .cloned()
                .ok_or(CallError::Aborted)?
        } else {
            let mut name = qa_core::text::FixedText::default();
            name.set_bytes(raw).map_err(|_| CallError::Text)?;
            ShaderRequest { name, mipmaps }
        };
        let named_lighting = self.shader_lighting(lighting)?;
        let catalog_id = self.shader_catalog(vfs, Some(&view))?;
        let catalog = self
            .catalog
            .shader_catalogs
            .get(catalog_id)
            .as_ref()
            .map_err(Clone::clone);
        let mut ready = false;
        let mut first_request_row = None;
        for &row in &self.catalog.shaders[index].profiles {
            let profile = self.catalog.shader_bindings[row].profile;
            let existing_default = (!matches!(
                self.catalog.shader_bindings[row].state,
                RegistrationState::Reload
            ))
            .then(|| {
                self.catalog.shaders[index]
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
                    .map(|(registration, required)| (*registration, required))
            })
            .flatten();
            let binding = &mut self.catalog.shader_bindings[row];
            if !matches_profile(binding.profile)
                || binding.lighting != lighting
                || binding.vertex_collapse
                    != binding
                        .profile
                        .finish
                        .compile(ui_fullscreen)
                        .collapse_vertex()
                || !binding.state.pending()
            {
                continue;
            }
            binding.request = Some(request.clone());
            first_request_row.get_or_insert(row);
            if let Some((registration, required_assets)) = existing_default {
                ready = true;
                binding.state = RegistrationState::Ready {
                    value: registration,
                    required_assets,
                };
                continue;
            }
            let image_index = image_profile_index(&self.catalog.profiles, binding.profile, &view)
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
            if first_request {
                self.catalog.shaders[index].first_request_rows[source as usize] =
                    Some(first_request_row.ok_or(CallError::Aborted)?);
            }
            self.catalog
                .shader_source_handle(
                    index as i32 + 1,
                    lighting,
                    matches_profile,
                    RegistrationVariant::Guard(ui_fullscreen),
                )
                .ok_or(CallError::Aborted)
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
        let handle = self.catalog.files.admit(None)?;
        let native = i32::try_from(handle.0).map_err(|_| CallError::Capacity)?;
        self.catalog.files[handle] = ModelFile::LoadedWorld {
            binding,
            bounds,
            collision: None,
            native_checksums: None,
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
        self.check_registration_vfs(vfs)?;
        let variant = RegistrationVariant::Guard(ui_fullscreen);
        if let Some(handle) = self.catalog.model_source_cached(name, source, variant) {
            return Ok(handle);
        }
        let path = std::str::from_utf8(name).map_err(|_| CallError::Text)?;
        if path.is_empty() || path.starts_with('*') {
            return Err(CallError::File);
        }
        self.ensure_file_profile(vfs, &canonical_path(path), None, ui_fullscreen, None)
            .map_err(|error| error.kind)?;
        self.catalog
            .model_source_cached(name, source, variant)
            .filter(|&handle| handle > 0)
            .ok_or(CallError::File)
    }

    pub fn preload_skin(
        &mut self,
        vfs: &Vfs,
        raw: &[u8],
        source: RuleSetId,
        ui_fullscreen: bool,
        search: Option<&SearchView>,
    ) -> Result<i32, CallError> {
        self.admit_skin_profile(vfs, raw, source, None, ui_fullscreen, search)
    }

    fn ensure_skin_row(
        &mut self,
        path: &str,
        ui_fullscreen: bool,
        search: Option<&SearchView>,
    ) -> Result<usize, CallError> {
        let view = search.unwrap_or(&self.catalog.registration_vfs).clone();
        let name = self.admit_name(path.as_bytes())?;
        let group = self.catalog.names.folded(name).ok_or(CallError::Aborted)?;
        if let Some(index) = self.catalog.skin_indices.find(&view, group) {
            return Ok(index.0 as usize);
        }
        if self.catalog.skins.len() >= self.catalog.limits.models
            || self.catalog.skins.len() >= i32::MAX as usize
        {
            return Err(CallError::Capacity);
        }
        let index = self.catalog.skins.len();
        self.catalog
            .skin_indices
            .bind(view.clone(), group, RegistrationIndex(index as u32))
            .map_err(|_| CallError::Capacity)?;
        self.catalog.skins.push(RegisteredSkin {
            name,
            search: view.clone(),
            source: None,
            profiles: self
                .catalog
                .profiles
                .iter()
                .filter(|images| images.source.same_selection(&view))
                .map(|images| SkinProfile {
                    request: SkinRequest::Named,
                    profile: images.profile,
                    vertex_collapse: images
                        .profile
                        .finish
                        .compile(ui_fullscreen)
                        .collapse_vertex(),
                    state: RegistrationState::Unattempted,
                    reserved: None,
                })
                .collect(),
        });
        Ok(index)
    }

    /// Admit a player-derived body without creating another public skin handle.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_indexed_skin(
        &mut self,
        vfs: &Vfs,
        name: &str,
        profile: ModelProfile,
        ui_fullscreen: bool,
        layout: IndexedSkinLayout,
        translation: IndexedSkinTranslation,
        search: Option<&SearchView>,
        prepare: impl FnOnce(&Assets, usize) -> Result<(), String>,
    ) -> Result<SkinSelection, IndexedSkinError> {
        self.prepare_skin_request(
            vfs,
            name,
            profile,
            ui_fullscreen,
            SkinRequest::Indexed(IndexedSkinRequest {
                layout,
                translation,
            }),
            search,
            prepare,
        )
    }

    pub fn prepare_model_colors(
        &mut self,
        vfs: &Vfs,
        registration: ModelRegistrationId,
        colors: ModelSkinColors,
        ui_fullscreen: bool,
        prepare: impl FnOnce(&Assets, usize) -> Result<(), String>,
    ) -> Result<SkinSelection, String> {
        self.catalog
            .published_registration(registration)
            .ok_or("model colors require an admitted model")?;
        let row = &self.catalog.registrations[registration.0 as usize];
        let profile = row.profile;
        let path = self
            .catalog
            .file_name(row.file)
            .ok_or("model color filename")?
            .to_owned();
        let search = self
            .catalog
            .files
            .source_view(row.file)
            .cloned()
            .ok_or("model color source selection")?;
        self.prepare_skin_request(
            vfs,
            &path,
            profile,
            ui_fullscreen,
            SkinRequest::ModelColors(colors),
            Some(&search),
            prepare,
        )
        .map_err(|error| error.to_string())
    }

    fn prepare_skin_request(
        &mut self,
        vfs: &Vfs,
        name: &str,
        profile: ModelProfile,
        ui_fullscreen: bool,
        request: SkinRequest,
        search: Option<&SearchView>,
        prepare: impl FnOnce(&Assets, usize) -> Result<(), String>,
    ) -> Result<SkinSelection, IndexedSkinError> {
        self.check_registration_vfs(vfs).map_err(|error| {
            IndexedSkinError::Rejected(format!("renderer VFS owner: {error:?}"))
        })?;
        let view = vfs.read_view(search).retain().map_err(|error| {
            IndexedSkinError::Rejected(format!("skin source selection: {error:?}"))
        })?;
        let path = canonical_path(name);
        if path.is_empty() || path.len() >= 64 {
            return Err(IndexedSkinError::Rejected(
                "indexed skin filename length".to_owned(),
            ));
        }
        if !self.initial_registration && self.catalog.admitted_assets != Some(self.assets.counts())
        {
            return Err(IndexedSkinError::Rejected(
                "indexed skin requires a published resource prefix".to_owned(),
            ));
        }
        if let Some(selected) =
            self.catalog
                .find_skin_selection(&path, profile, request, ui_fullscreen, Some(&view))
        {
            return Ok(selected);
        }
        let target = RendererRequest {
            search: Some(view.clone()),
            name: path.as_bytes(),
            source: profile.source_rules,
            kind: RendererRegistration::Skin,
            world: None,
        };
        let mut checkpoint =
            RegistrationCheckpoint::base(self).map_err(IndexedSkinError::Rejected)?;
        checkpoint
            .capture_target(self, &target)
            .map_err(IndexedSkinError::Rejected)?;
        if matches!(request, SkinRequest::ModelColors(_)) {
            checkpoint
                .capture_all(self)
                .map_err(IndexedSkinError::Rejected)?;
        }
        let mut rollback = RegistrationRollback::Retry;
        let result = (|| {
            let selection =
                self.admit_skin_request(vfs, &path, profile, ui_fullscreen, request, Some(&view))?;
            let counts = self.assets.counts();
            if !self.initial_registration {
                self.validate_admission(counts).map_err(|error| {
                    IndexedSkinError::Rejected(format!("indexed skin admission: {error:?}"))
                })?;
                if self.image_transaction_active != Some(ImageRefreshMode::IndexedSkins) {
                    prepare(&self.assets, self.catalog.max_choice_nodes())
                        .map_err(IndexedSkinError::Rejected)?;
                }
                self.apply_admission(counts);
            }
            Ok(selection)
        })();
        if result.is_err() {
            if find_skin_index(&self.catalog.names, &self.catalog.skin_indices, path.as_bytes(), &view)
                .and_then(|(_, index)| index).and_then(|index| self.catalog.skins.get(index as usize))
                .is_some_and(|entry| matches!(&entry.source, Some(SkinSource::IndexedFailure { layout: failed, .. }) if matches!(request, SkinRequest::Indexed(indexed) if indexed.layout == *failed))) {
                rollback = RegistrationRollback::Source;
            }
            checkpoint
                .rollback(self, rollback)
                .map_err(IndexedSkinError::Rejected)?;
        }
        result
    }

    fn admit_skin_request(
        &mut self,
        vfs: &Vfs,
        path: &str,
        profile: ModelProfile,
        ui: bool,
        request: SkinRequest,
        search: Option<&SearchView>,
    ) -> Result<SkinSelection, IndexedSkinError> {
        let view = search.unwrap_or(&self.catalog.registration_vfs).clone();
        self.ensure_source_profiles(&view, Some(profile))
            .map_err(|error| IndexedSkinError::Rejected(error.message))?;
        match request {
            SkinRequest::Indexed(request) => {
                self.admit_indexed_skin(vfs, path, profile, ui, request, Some(&view))
            }
            SkinRequest::ModelColors(colors) => self
                .admit_model_colors(vfs, path, profile, ui, colors, Some(&view))
                .map_err(IndexedSkinError::Rejected),
            SkinRequest::Named => {
                let handle = self
                    .admit_skin_profile(
                        vfs,
                        path.as_bytes(),
                        profile.source_rules,
                        Some(profile),
                        ui,
                        Some(&view),
                    )
                    .map_err(|error| {
                        IndexedSkinError::Rejected(format!("skin registration: {error:?}"))
                    })?;
                let skin = handle
                    .checked_sub(1)
                    .ok_or_else(|| IndexedSkinError::Rejected("skin registration failed".into()))?
                    as usize;
                let row = self.catalog.skins[skin]
                    .profiles
                    .iter()
                    .position(|row| {
                        row.request == SkinRequest::Named
                            && row.profile == profile
                            && row.vertex_collapse == profile.finish.compile(ui).collapse_vertex()
                    })
                    .ok_or_else(|| IndexedSkinError::Rejected("skin registered row".into()))?;
                Ok(SkinSelection {
                    skin: skin as u32,
                    row: row as u32,
                })
            }
        }
    }

    fn skin_variant(
        &mut self,
        index: usize,
        profile: ModelProfile,
        ui: bool,
        request: SkinRequest,
    ) -> Result<SkinSelection, String> {
        let collapse = profile.finish.compile(ui).collapse_vertex();
        let row = match self.catalog.skins[index].profiles.iter().position(|row| {
            row.profile == profile && row.vertex_collapse == collapse && row.request == request
        }) {
            Some(row) => row,
            None => {
                let variants = self
                    .catalog
                    .skins
                    .iter()
                    .flat_map(|skin| &skin.profiles)
                    .filter(|row| row.request != SkinRequest::Named)
                    .count();
                if variants >= self.catalog.limits.skins {
                    return Err("derived skin variant capacity".to_owned());
                }
                let rows = &mut self.catalog.skins[index].profiles;
                let row = rows.len();
                rows.push(SkinProfile {
                    profile,
                    vertex_collapse: collapse,
                    request,
                    state: RegistrationState::Unattempted,
                    reserved: None,
                });
                row
            }
        };
        Ok(SkinSelection {
            skin: index as u32,
            row: row as u32,
        })
    }

    fn admit_indexed_skin(
        &mut self,
        vfs: &Vfs,
        path: &str,
        profile: ModelProfile,
        ui_fullscreen: bool,
        request: IndexedSkinRequest,
        search: Option<&SearchView>,
    ) -> Result<SkinSelection, IndexedSkinError> {
        let view = search.unwrap_or(&self.catalog.registration_vfs).clone();
        let image_index =
            image_profile_index(&self.catalog.profiles, profile, &view).ok_or_else(|| {
                IndexedSkinError::Rejected("indexed skin profile unavailable".to_owned())
            })?;
        let index = self
            .ensure_skin_row(path, ui_fullscreen, Some(&view))
            .map_err(|error| IndexedSkinError::Rejected(format!("indexed skin row: {error:?}")))?;
        let selection = self
            .skin_variant(index, profile, ui_fullscreen, SkinRequest::Indexed(request))
            .map_err(IndexedSkinError::Rejected)?;
        let row = selection.row as usize;
        if self.catalog.skins[index].profiles[row]
            .state
            .ready()
            .is_some()
        {
            return Ok(selection);
        }
        let source = match self.skin_source(vfs, path, image_index, index, request.layout) {
            Ok(source) => source,
            Err(error) => {
                self.catalog.skins[index].profiles[row].state =
                    RegistrationState::Failed(error.to_string());
                return Err(error);
            }
        };
        let next = self
            .catalog
            .capacity
            .skins
            .checked_add(1)
            .filter(|&count| count <= self.catalog.limits.skins)
            .ok_or_else(|| IndexedSkinError::Rejected("indexed skin capacity".to_owned()))?;
        let reserved = self.catalog.skins[index].profiles[row].reserved.as_ref();
        let ready = with_image_owner(&mut self.catalog.profiles, image_index, |images| {
            let policy = images.model_policy;
            images.with_images(vfs, &mut self.assets, |images| {
                let image = images
                    .translated_skin(path, 0, source.view(), request.translation)
                    .map_err(|error| format!("indexed skin image: {error:?}"))?
                    .id;
                if let Some(reserved) = reserved {
                    let shader = reserved
                        .members
                        .first()
                        .and_then(|members| match members[0] {
                            ShaderChoice::Direct(shader) => Some(shader),
                            _ => None,
                        })
                        .and_then(|shader| images.assets.shader_binding(shader))
                        .and_then(|shader| images.assets.material(shader.material))
                        .and_then(|material| material.stages.first());
                    if !shader.is_some_and(|stage| stage.texture == StageTexture::Image(image)) {
                        return Err("indexed skin reservation image changed".to_owned());
                    }
                    return Ok(reserved.clone());
                }
                let variants = alias_image_materials(images.assets, path, 0, image, policy, false)?;
                let registration = images
                    .assets
                    .shader_registration(path.as_bytes(), variants[0])?;
                Ok::<_, String>(SkinReady {
                    mappings: vec![SkinMapping {
                        surface: NameId(0),
                        members: IndexSpan { first: 0, count: 1 },
                    }]
                    .into_boxed_slice(),
                    members: vec![variants.map(ShaderChoice::Direct)].into_boxed_slice(),
                    registrations: vec![registration].into_boxed_slice(),
                })
            })
        })
        .map_err(|error| IndexedSkinError::Rejected(format!("indexed skin material: {error:?}")))?;
        self.catalog.capacity.skins = next;
        self.catalog.skins[index].profiles[row].reserved = None;
        self.catalog.skins[index].profiles[row].state = RegistrationState::Ready {
            value: ready,
            required_assets: (!self.initial_registration).then(|| self.assets.counts()),
        };
        Ok(selection)
    }

    fn admit_model_colors(
        &mut self,
        vfs: &Vfs,
        path: &str,
        profile: ModelProfile,
        ui: bool,
        colors: ModelSkinColors,
        search: Option<&SearchView>,
    ) -> Result<SkinSelection, String> {
        let file = self
            .catalog
            .file_id(path.as_bytes(), search)
            .ok_or("model color file")?;
        let view = self
            .catalog
            .files
            .source_view(file)
            .cloned()
            .ok_or("model color source selection")?;
        let registration = self
            .catalog
            .registration_name_variant(file, profile, RegistrationVariant::Guard(ui))
            .or_else(|| self.catalog.registration_name(file, profile))
            .ok_or("model color registration")?;
        let ready = self.catalog.registrations[registration.0 as usize]
            .state
            .ready()
            .ok_or("model color readiness")?
            .0;
        let palette = ready.indexed_palette.ok_or("model color source palette")?;
        let native = ready.surfaces[0].ok_or("model color native surfaces")?;
        let native_surfaces: Vec<_> = self.catalog.surfaces[native.range()]
            .iter()
            .map(|row| row.name)
            .collect();
        let replacement_surfaces: Vec<_> = ready
            .replacement
            .as_ref()
            .map(|row| {
                self.catalog.surfaces[row.surfaces.range()]
                    .iter()
                    .map(|row| row.name)
                    .collect()
            })
            .unwrap_or_default();
        let ModelFile::Ready { lods, .. } = self.catalog.files[file] else {
            return Err("model color source slots".to_owned());
        };
        let source_name = lods.source(0).ok_or("model color native source")?;
        let ModelSourceFile::Ready {
            bytes,
            skin_extent: Some(extent),
            indexed_skins,
            replacement,
            ..
        } = &self.catalog.sources[source_name]
        else {
            return Err("model colors require native indexed MDL skins".to_owned());
        };
        let bytes = Arc::clone(bytes);
        let spans = Arc::clone(indexed_skins);
        let extent = *extent;
        if spans.is_empty() {
            return Err("model color native members".to_owned());
        }
        let replacement_names = if replacement_surfaces.is_empty() {
            None
        } else {
            let ReplacementState::Ready(replacement) = replacement else {
                return Err("model color replacement source".to_owned());
            };
            if replacement.skins.len() != replacement_surfaces.len() {
                return Err("model color replacement surfaces".to_owned());
            }
            Some(Arc::clone(&replacement.skins))
        };
        let image_index = image_profile_index(&self.catalog.profiles, profile, &view)
            .ok_or("model color image owner")?;
        let index = self
            .ensure_skin_row(path, ui, Some(&view))
            .map_err(|error| format!("model color row: {error:?}"))?;
        let selected = self.skin_variant(index, profile, ui, SkinRequest::ModelColors(colors))?;
        if self.catalog.skins[index].profiles[selected.row as usize]
            .state
            .ready()
            .is_some()
        {
            return Ok(selected);
        }
        let mut sidecars = Vec::new();
        if let Some(names) = replacement_names {
            for (surface, names) in replacement_surfaces.into_iter().zip(names.iter()) {
                if names.len() != spans.len() {
                    return Err("model color replacement members".to_owned());
                }
                let mut sources = Vec::with_capacity(names.len());
                for &name in names.iter() {
                    let path = self
                        .catalog
                        .name(name)
                        .ok_or("model color sidecar filename")?
                        .to_owned();
                    let skin = self
                        .ensure_skin_row(&path, ui, Some(&view))
                        .map_err(|error| format!("model color sidecar row: {error:?}"))?;
                    let source = self
                        .skin_source(vfs, &path, image_index, skin, Default::default())
                        .map_err(|error| error.to_string())?;
                    sources.push((path, source));
                }
                sidecars.push((surface, sources));
            }
        }
        let count = spans
            .len()
            .checked_add(
                sidecars
                    .iter()
                    .map(|(_, members)| members.len())
                    .sum::<usize>(),
            )
            .ok_or("model color member count")?;
        let next = self
            .catalog
            .capacity
            .skins
            .checked_add(count)
            .filter(|&count| count <= self.catalog.limits.skins && count <= u32::MAX as usize)
            .ok_or("model color skin capacity")?;
        let compiled = with_image_owner(&mut self.catalog.profiles, image_index, |owner| {
            let policy = owner.model_policy;
            owner.with_images(vfs, &mut self.assets, |images| {
                let mut members = Vec::with_capacity(count);
                let mut registrations = Vec::with_capacity(count);
                let mut mappings = Vec::with_capacity(native_surfaces.len() + sidecars.len());
                let translation = |coordinate_extent, crop| IndexedSkinTranslation {
                    palette,
                    target: profile.palette,
                    indexed_colors: Some(colors.indexed),
                    rgba_colors: Some(colors.rgba),
                    crop,
                    coordinate_extent,
                    upload_extent: colors.upload_extent,
                    usage: ImageUse {
                        role: ImageRole::Skin,
                        sampler: qa_render::assets::Sampler {
                            mipmaps: false,
                            ..Default::default()
                        },
                        allow_picmip: false,
                    },
                };
                let crop = colors
                    .rgba_crop
                    .filter(|crop| crop[0] <= extent[0] && crop[1] <= extent[1])
                    .map(|[width, height]| [0, 0, width, height]);
                for (ordinal, span) in spans.iter().enumerate() {
                    let source = IndexedSkinView::new(
                        extent,
                        bytes.get(span.range()).ok_or("model color member range")?,
                    )
                    .map_err(|error| format!("model color source view: {error:?}"))?;
                    let image = images
                        .translated_skin(path, ordinal as u32, source, translation(extent, crop))
                        .map_err(|error| format!("model color image: {error:?}"))?
                        .id;
                    let shaders =
                        alias_image_materials(images.assets, path, ordinal, image, policy, false)?;
                    registrations.push(
                        images
                            .assets
                            .shader_registration(path.as_bytes(), shaders[0])?,
                    );
                    members.push(shaders.map(ShaderChoice::Direct));
                }
                for surface in native_surfaces {
                    mappings.push(SkinMapping {
                        surface,
                        members: IndexSpan {
                            first: 0,
                            count: spans.len() as u32,
                        },
                    });
                }
                for (surface, sources) in sidecars {
                    let first = members.len() as u32;
                    for (path, source) in sources {
                        let image = images
                            .translated_skin(
                                &path,
                                0,
                                source.view(),
                                translation(source.decoded_extent(), None),
                            )
                            .map_err(|error| format!("model color sidecar image: {error:?}"))?
                            .id;
                        let shaders =
                            alias_image_materials(images.assets, &path, 0, image, policy, false)?;
                        registrations.push(
                            images
                                .assets
                                .shader_registration(path.as_bytes(), shaders[0])?,
                        );
                        members.push(shaders.map(ShaderChoice::Direct));
                    }
                    mappings.push(SkinMapping {
                        surface,
                        members: IndexSpan {
                            first,
                            count: members.len() as u32 - first,
                        },
                    });
                }
                mappings.sort_by_key(|mapping| mapping.surface.0);
                if mappings
                    .windows(2)
                    .any(|rows| rows[0].surface == rows[1].surface)
                {
                    return Err("model color duplicate surface".to_owned());
                }
                Ok(SkinReady {
                    mappings: mappings.into_boxed_slice(),
                    members: members.into_boxed_slice(),
                    registrations: registrations.into_boxed_slice(),
                })
            })
        })?;
        self.catalog.capacity.skins = next;
        self.catalog.skins[index].source = Some(SkinSource::AliasModel(source_name));
        self.catalog.skins[index].profiles[selected.row as usize].state =
            RegistrationState::Ready {
                value: compiled,
                required_assets: (!self.initial_registration).then(|| self.assets.counts()),
            };
        Ok(selected)
    }

    fn skin_source(
        &mut self,
        vfs: &Vfs,
        path: &str,
        image_index: usize,
        index: usize,
        requested_layout: IndexedSkinLayout,
    ) -> Result<Arc<IndexedSkinSource>, IndexedSkinError> {
        let source = match &self.catalog.skins[index].source {
            Some(SkinSource::Indexed { layout, source }) if *layout == requested_layout => {
                Some(Arc::clone(source))
            }
            Some(SkinSource::IndexedFailure {
                layout,
                missing,
                error,
            }) if *layout == requested_layout => {
                return Err(if *missing {
                    IndexedSkinError::Missing
                } else {
                    IndexedSkinError::InvalidSource(error.clone())
                });
            }
            _ => None,
        };
        Ok(match source {
            Some(source) => source,
            None => {
                let decoded = with_image_owner(&mut self.catalog.profiles, image_index, |images| {
                    images.with_images(vfs, &mut self.assets, |images| {
                        Ok(images.indexed_skin_source(path, requested_layout))
                    })
                })
                .map_err(IndexedSkinError::Rejected)?;
                match decoded {
                    Ok(source) => {
                        let source = Arc::new(source);
                        self.catalog.skins[index].source = Some(SkinSource::Indexed {
                            layout: requested_layout,
                            source: Arc::clone(&source),
                        });
                        source
                    }
                    Err(error) => {
                        if matches!(
                            &error,
                            ResourceError::Vfs(
                                qa_content::vfs::VfsError::Allocation
                                    | qa_content::vfs::VfsError::Capacity
                            )
                        ) {
                            return Err(IndexedSkinError::Rejected(format!(
                                "indexed skin source: {error:?}"
                            )));
                        }
                        let missing = matches!(error, ResourceError::Missing(_));
                        let error = format!("indexed skin source: {error:?}");
                        self.catalog.skins[index].source = Some(SkinSource::IndexedFailure {
                            layout: requested_layout,
                            missing,
                            error: error.clone(),
                        });
                        return Err(if missing {
                            IndexedSkinError::Missing
                        } else {
                            IndexedSkinError::InvalidSource(error)
                        });
                    }
                }
            }
        })
    }

    fn admit_skin_profile(
        &mut self,
        vfs: &Vfs,
        raw: &[u8],
        source: RuleSetId,
        selected: Option<ModelProfile>,
        ui_fullscreen: bool,
        search: Option<&SearchView>,
    ) -> Result<i32, CallError> {
        self.check_registration_vfs(vfs)?;
        let view = vfs
            .read_view(search)
            .retain()
            .map_err(|_| CallError::File)?;
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
                Some(&view),
            )
        } else {
            find_skin_index(
                &self.catalog.names,
                &self.catalog.skin_indices,
                path.as_bytes(),
                &view,
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
        self.ensure_source_profiles(&view, selected)
            .map_err(|error| error.kind)?;
        let index = self.ensure_skin_row(&path, ui_fullscreen, Some(&view))?;
        let name = self.catalog.skins[index].name;
        for images in self
            .catalog
            .profiles
            .iter()
            .filter(|images| images.source.same_selection(&view))
        {
            if !self.catalog.skins[index].profiles.iter().any(|row| {
                row.request == SkinRequest::Named
                    && row.profile == images.profile
                    && row.vertex_collapse
                        == images
                            .profile
                            .finish
                            .compile(ui_fullscreen)
                            .collapse_vertex()
            }) {
                self.catalog.skins[index].profiles.push(SkinProfile {
                    request: SkinRequest::Named,
                    profile: images.profile,
                    vertex_collapse: images
                        .profile
                        .finish
                        .compile(ui_fullscreen)
                        .collapse_vertex(),
                    state: RegistrationState::Unattempted,
                    reserved: None,
                });
            }
        }
        let profile_count = self.catalog.skins[index]
            .profiles
            .iter()
            .filter(|entry| {
                entry.request == SkinRequest::Named
                    && entry.state.pending()
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
        let mut retained_source = self.catalog.skins[index].source.clone();
        let mappings = (|| {
            let bytes = if skin_file {
                match &retained_source {
                    Some(SkinSource::Script(bytes)) => Arc::clone(bytes),
                    _ => Arc::from(
                        vfs.read_view(Some(&view))
                            .read_file(
                                path.as_bytes(),
                                self.catalog.limits.file_bytes,
                                &mut self.catalog.reader,
                            )
                            .map_err(|_| CallError::File)?,
                    ),
                }
            } else {
                Arc::from([])
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
            rows.len()
                .checked_mul(profile_count)
                .and_then(|count| self.catalog.capacity.skins.checked_add(count))
                .filter(|&count| count <= self.catalog.limits.skins)
                .ok_or(CallError::Capacity)?;
            let catalog_id = self.shader_catalog(vfs, Some(&view))?;
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
            if skin_file {
                retained_source = Some(SkinSource::Script(bytes));
            }
            Ok((mappings, catalog_id))
        })();
        let (mappings, catalog_id) = match mappings {
            Ok(mappings) => mappings,
            Err(error) => {
                for profile in &mut self.catalog.skins[index].profiles {
                    if profile.request == SkinRequest::Named
                        && profile.state.pending()
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
            if profile.request != SkinRequest::Named
                || !profile.state.pending()
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
            let image_index = image_profile_index(&self.catalog.profiles, profile.profile, &view)
                .ok_or(CallError::Aborted)?;
            let registered = with_image_owner(
                &mut self.catalog.profiles,
                image_index,
                |images| -> Result<usize, CallError> {
                    let alias = !skin_file && images.model_policy.source.indexed_external_pcx;
                    let mut compiled = Vec::with_capacity(mappings.len());
                    let mut members = Vec::with_capacity(mappings.len());
                    let mut registrations = Vec::with_capacity(mappings.len());
                    let mut failure = None;
                    for (surface, raw) in &mappings {
                        let result = if alias {
                            let registration = Registration {
                                path: path.clone(),
                                profile: images.profile,
                                indexed_palette: images.profile.source_palette,
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
                                let materials =
                                    materials.pop().ok_or("missing skin material")?.variants;
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
                                        .shader_catalogs
                                        .get(catalog_id)
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
                            members: IndexSpan {
                                first: members.len() as u32,
                                count: 1,
                            },
                        });
                        members.push(materials);
                    }
                    compiled.sort_by_key(|entry| entry.surface.0);
                    compiled.dedup_by_key(|entry| entry.surface.0);
                    let registered = compiled.len();
                    profile.state = if compiled.is_empty() {
                        RegistrationState::Failed(
                            failure.unwrap_or_else(|| "missing skin mapping".to_owned()),
                        )
                    } else {
                        RegistrationState::Ready {
                            value: SkinReady {
                                mappings: compiled.into_boxed_slice(),
                                members: members.into_boxed_slice(),
                                registrations: registrations.into_boxed_slice(),
                            },
                            required_assets: (!self.initial_registration)
                                .then(|| self.assets.counts()),
                        }
                    };
                    Ok(registered)
                },
            )?;
            self.catalog.capacity.skins = self
                .catalog
                .capacity
                .skins
                .checked_add(registered)
                .ok_or(CallError::Capacity)?;
        }
        if self.catalog.skins[index]
            .profiles
            .iter()
            .any(|entry| entry.state.ready().is_some())
        {
            self.catalog.skins[index].source = retained_source;
        }
        if let Some(selected) = selected {
            self.catalog.skins[index]
                .profiles
                .iter()
                .find(|entry| {
                    entry.request == SkinRequest::Named
                        && entry.profile == selected
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
    let (_, index) = find_shader_index(
        &catalog.names,
        &catalog.shader_indices,
        raw,
        &catalog.registration_vfs,
    )?;
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
    indices: &ScopedBindings<RegistrationIndex>,
    raw: &[u8],
    source: &SearchView,
) -> Option<(NameId, Option<u32>)> {
    let group = names.find_folded(qa_render::shader::registration_key(raw))?;
    Some((group, indices.find(source, group).map(|index| index.0)))
}
fn find_skin_index(
    names: &NameTable,
    indices: &ScopedBindings<RegistrationIndex>,
    raw: &[u8],
    source: &SearchView,
) -> Option<(NameId, Option<u32>)> {
    let group = names.find_path_group(raw)?;
    Some((group, indices.find(source, group).map(|index| index.0)))
}
fn skin_handle(catalog: &ModelCatalog, raw: &[u8], source: RuleSetId) -> Option<i32> {
    if raw.is_empty() {
        return Some(0);
    }
    let (_, index) = find_skin_index(
        &catalog.names,
        &catalog.skin_indices,
        raw,
        &catalog.registration_vfs,
    )?;
    let handle = i32::try_from(index?.checked_add(1)?).ok()?;
    catalog.skin_source_handle(handle, source, RegistrationVariant::Retained)
}

fn loaded_world(
    names: &NameTable,
    files: &ModelFiles,
    name: &[u8],
    search: Option<&SearchView>,
) -> Option<LoadedWorldBinding> {
    let ModelFile::LoadedWorld {
        binding, collision, ..
    } = file_lookup(names, files, name, search)?
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
    files: &'a ModelFiles,
    path: &[u8],
    search: Option<&SearchView>,
) -> Option<&'a ModelFile> {
    let name = names.find_path(std::str::from_utf8(path).ok()?)?;
    files.get(files.find(search?, name)?)
}
fn model_handle(
    names: &NameTable,
    files: &ModelFiles,
    path: &[u8],
    source: Option<RuleSetId>,
    search: Option<&SearchView>,
) -> Option<i32> {
    let name = names.find_path(std::str::from_utf8(path).ok()?)?;
    let file = files.find(search?, name)?;
    let handle = i32::try_from(file.0).ok()?;
    renderer_file(files, handle, source)?;
    Some(handle)
}
fn renderer_file(files: &ModelFiles, handle: i32, source: Option<RuleSetId>) -> Option<&ModelFile> {
    let index = usize::try_from(handle).ok().filter(|&index| index > 0)?;
    let file = files.get(ModelFileId(index as u32))?;
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
fn model_bounds(files: &ModelFiles, sources: &ModelSources, handle: i32) -> Option<Bounds> {
    match renderer_file(files, handle, None)? {
        ModelFile::Ready { lods, .. } => Some(
            lods.source(0)
                .and_then(|source| sources.get(source))
                .and_then(ModelSourceFile::metadata)
                .map_or_else(Bounds::default, |metadata| metadata.renderer_bounds),
        ),
        file => file.bounds(sources),
    }
}
fn model_tags(files: &ModelFiles, sources: &ModelSources, handle: i32) -> Option<TagLayout> {
    let ModelFile::Ready { lods, .. } = renderer_file(files, handle, None)? else {
        return None;
    };
    match sources.get(lods.source(0)?)? {
        ModelSourceFile::Ready { tags, .. } => *tags,
        _ => None,
    }
}
fn lerp_tag(
    names: &NameTable,
    files: &ModelFiles,
    sources: &ModelSources,
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
    fn set_model_replacement_load(&mut self, q1: bool, q2: bool) {
        ModelAssets::set_model_replacement_load(self, q1, q2);
    }
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
        self.check_registration_vfs(vfs)?;
        let path = std::str::from_utf8(path).map_err(|_| CallError::Text)?;
        if path.is_empty() || path.starts_with('*') {
            return Err(CallError::File);
        }
        if let Some(bounds) = file_lookup(
            &self.catalog.names,
            &self.catalog.files,
            path.as_bytes(),
            Some(&self.catalog.registration_vfs),
        )
        .and_then(|file| file.bounds(&self.catalog.sources))
        {
            return Ok(bounds);
        }
        self.ensure_file_profile(vfs, &canonical_path(path), None, false, None)
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
        self.preload_skin(vfs, name, source, false, None)
    }
    fn register_shader(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
        mipmaps: bool,
        lighting: qa_compat::services::ShaderLighting,
    ) -> Result<i32, CallError> {
        self.preload_shader(vfs, None, name, source, mipmaps, lighting)
    }
}
impl ModelLookup for ModelCatalog {
    fn file_handle(&self, path: &[u8]) -> Option<i32> {
        model_handle(
            &self.names,
            &self.files,
            path,
            None,
            Some(&self.registration_vfs),
        )
    }
    fn inline_handle(&self, world: WorldId, index: u32) -> Option<i32> {
        inline_handle(&self.worlds, &self.inline, world, index)
    }
    fn model_has_tags(&self, handle: i32) -> bool {
        model_tags(&self.files, &self.sources, handle).is_some()
    }
    fn loaded_world(
        &self,
        name: &[u8],
        search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<LoadedWorldBinding> {
        loaded_world(
            &self.names,
            &self.files,
            name,
            Some(search.unwrap_or(&self.registration_vfs)),
        )
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
        file_lookup(&self.names, &self.files, path, Some(&self.registration_vfs))?
            .bounds(&self.sources)
    }
    fn skin_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        skin_handle(self, name, source)
    }
    fn shader_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        shader_handle(self, name, source)
    }
    fn model_handle(&self, path: &[u8], source: RuleSetId) -> Option<i32> {
        model_handle(
            &self.names,
            &self.files,
            path,
            Some(source),
            Some(&self.registration_vfs),
        )
    }
    fn model_bounds(&self, handle: i32) -> Option<Bounds> {
        model_bounds(&self.files, &self.sources, handle)
    }
}
impl ModelCatalog {
    pub fn world_native_checksums(
        &self,
        path: &[u8],
    ) -> Option<qa_formats::bsp::NativeMapChecksums> {
        match file_lookup(&self.names, &self.files, path, Some(&self.registration_vfs))? {
            ModelFile::LoadedWorld {
                native_checksums, ..
            } => *native_checksums,
            _ => None,
        }
    }

    pub fn max_choice_nodes(&self) -> usize {
        self.capacity.model_choices
    }
    fn calculate_choice_nodes(&self) -> usize {
        self.registrations
            .iter()
            .filter_map(|entry| {
                let ready = entry.state.ready()?.0;
                let ModelFile::Ready { lods, .. } = self.files.get(entry.file)? else {
                    return None;
                };
                let groups = lods
                    .source(0)
                    .and_then(|source| self.sources.get(source))
                    .and_then(|source| match source {
                        ModelSourceFile::Ready { animation, .. } => animation.as_deref(),
                        _ => None,
                    });
                let skin_members = groups
                    .and_then(|animation| {
                        animation
                            .skin_groups()
                            .iter()
                            .map(|group| group.members.count as usize)
                            .max()
                    })
                    .unwrap_or(1);
                let ordinary = ready
                    .surfaces
                    .iter()
                    .flatten()
                    .flat_map(|range| &self.surfaces[range.range()])
                    .map(|surface| {
                        let members = if surface.sprite_frames {
                            surface.skins.count as usize
                        } else {
                            skin_members
                        };
                        if members > 1 {
                            members.saturating_add(1)
                        } else {
                            0
                        }
                    })
                    .max()
                    .unwrap_or(0);
                let native = if lods.native_count > 1 {
                    ordinary
                        .saturating_mul(lods.native_count)
                        .saturating_add(lods.native_count + 1)
                } else {
                    ordinary
                };
                Some(if let Some(replacement) = &ready.replacement {
                    let mut selected = if skin_members > 1 {
                        skin_members + 1
                    } else {
                        0
                    };
                    if replacement.frames.is_some() {
                        selected += if selected == 0 { 2 } else { 1 };
                    }
                    native.saturating_add(selected).saturating_add(3)
                } else {
                    native
                })
            })
            .max()
            .unwrap_or(0)
    }
    pub fn profiles(&self) -> impl Iterator<Item = ModelProfile> + '_ {
        self.profiles
            .iter()
            .filter(|entry| entry.source.same_selection(&self.registration_vfs))
            .map(|entry| entry.profile)
    }
    pub(crate) fn registration_profile(&self, id: ModelRegistrationId) -> Option<ModelProfile> {
        self.registrations
            .get(id.0 as usize)
            .map(|entry| entry.profile)
    }
    pub(crate) fn registration_source(
        &self,
        id: ModelRegistrationId,
    ) -> Option<(ModelFileId, i32)> {
        self.published_registration(id)?;
        let row = self.registrations.get(id.0 as usize)?;
        let sync = match self.files.get(row.file)? {
            ModelFile::Ready { lods, .. } => lods
                .source(0)
                .and_then(|source| self.sources.get(source))
                .and_then(|source| match source {
                    ModelSourceFile::Ready { animation, .. } => animation.as_deref(),
                    _ => None,
                })
                .map_or(0, ModelAnimation::sync),
            _ => 0,
        };
        Some((row.file, sync))
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
        self.registration_cached_variant(
            kind,
            name,
            profile,
            world,
            RegistrationVariant::Retained,
            None,
        )
    }
    pub fn registration_cached_variant(
        &self,
        kind: RendererRegistration,
        name: &[u8],
        profile: ModelProfile,
        world: Option<WorldId>,
        variant: RegistrationVariant,
        search: Option<&SearchView>,
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
                let name = self.file_id(name, search)?;
                self.model_cached(name, profile, variant)
            }
            RendererRegistration::Shader { lighting, .. } => {
                let (_, index) = find_shader_index(
                    &self.names,
                    &self.shader_indices,
                    name,
                    search.unwrap_or(&self.registration_vfs),
                )?;
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
                let (_, index) = find_skin_index(
                    &self.names,
                    &self.skin_indices,
                    name,
                    search.unwrap_or(&self.registration_vfs),
                )?;
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
        let name = self.file_id(name, None)?;
        let handle = i32::try_from(name.0).ok()?;
        match self.files.get(name)? {
            ModelFile::Failed(_) => return Some(0),
            ModelFile::LoadedWorld { .. } => return Some(handle),
            ModelFile::Empty | ModelFile::Ready { .. } => {}
        }
        source_handle(
            self.files
                .rows(name)
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

    fn model_cached(
        &self,
        file: ModelFileId,
        profile: ModelProfile,
        variant: RegistrationVariant,
    ) -> Option<i32> {
        let handle = i32::try_from(file.0).ok()?;
        match self.files.get(file)? {
            ModelFile::Failed(_) => Some(0),
            ModelFile::LoadedWorld { .. } => Some(handle),
            ModelFile::Empty | ModelFile::Ready { .. } => self
                .files
                .rows(file)
                .iter()
                .copied()
                .map(|index| &self.registrations[index])
                .find(|entry| {
                    entry.profile == profile
                        && variant.matches(entry.profile, entry.vertex_collapse)
                        && !entry.state.pending()
                })?
                .state
                .cached(handle, self.admitted_assets),
        }
    }

    pub fn color_binding(&self, profile: ModelProfile) -> Option<ModelColorId> {
        let index = image_profile_index(&self.profiles, profile, &self.registration_vfs)?;
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
            self.profiles()
                .filter(|profile| profile.source_rules == source)
                .map(|profile| {
                    self.skin_profile_variant(handle, profile, variant)
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
            self.profiles()
                .filter(|profile| matches_profile(*profile))
                .map(|profile| {
                    self.shader_profile_lighting_variant(handle, profile, lighting, variant)
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
        search: Option<&SearchView>,
    ) -> Option<(&ShaderRegistration, Option<AssetCounts>)> {
        self.shader_outcome_candidate_variant(
            name,
            profile,
            lighting,
            RegistrationVariant::Retained,
            search,
        )
    }
    fn shader_outcome_candidate_variant(
        &self,
        name: &[u8],
        profile: ModelProfile,
        lighting: ShaderLighting,
        variant: RegistrationVariant,
        search: Option<&SearchView>,
    ) -> Option<(&ShaderRegistration, Option<AssetCounts>)> {
        let (_, index) = find_shader_index(
            &self.names,
            &self.shader_indices,
            name,
            search.unwrap_or(&self.registration_vfs),
        )?;
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
        search: Option<&SearchView>,
    ) -> Option<&ShaderRegistration> {
        let (registration, required) =
            self.shader_outcome_candidate(name, profile, lighting, search)?;
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
                            && !entry.state.pending()
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
    pub fn metadata(&self, path: &[u8], search: Option<&SearchView>) -> Option<ModelMetadata> {
        match self.files.get(self.file_id(path, search)?)? {
            ModelFile::Ready { lods, .. } => self.sources.get(lods.source(0)?)?.metadata(),
            ModelFile::Empty | ModelFile::LoadedWorld { .. } | ModelFile::Failed(_) => None,
        }
    }
    /// Resolve both filename and profile only at registration/configuration
    /// boundaries. A profile mismatch never selects the first same-name entry.
    pub fn registration(
        &self,
        path: &str,
        profile: ModelProfile,
        search: Option<&SearchView>,
    ) -> Option<ModelRegistrationId> {
        let name = self.file_id(path.as_bytes(), search)?;
        self.published_registration(self.registration_name(name, profile)?)
    }
    /// Native handles already identify the logical model record. Resolve its exact
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
        name: ModelFileId,
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
        self.registration_name_variant(ModelFileId(u32::try_from(handle).ok()?), profile, variant)
    }
    /// A logical skin handle is usable only when its selected presentation
    /// registered successfully. Other profiles cannot supply a fallback here.
    pub fn native_skin(&self, registration: ModelRegistrationId) -> Option<([u32; 2], PaletteId)> {
        self.published_registration(registration)?;
        let row = self.registrations.get(registration.0 as usize)?;
        let palette = row.state.ready()?.0.indexed_palette?;
        let ModelFile::Ready { lods, .. } = self.files.get(row.file)? else {
            return None;
        };
        match self.sources.get(lods.source(0)?)? {
            ModelSourceFile::Ready {
                skin_extent: Some(extent),
                ..
            } => Some((*extent, palette)),
            _ => None,
        }
    }
    fn find_skin_selection(
        &self,
        path: &str,
        profile: ModelProfile,
        request: SkinRequest,
        ui: bool,
        search: Option<&SearchView>,
    ) -> Option<SkinSelection> {
        let (_, skin) = find_skin_index(
            &self.names,
            &self.skin_indices,
            path.as_bytes(),
            search.unwrap_or(&self.registration_vfs),
        )?;
        let skin = skin?;
        let row = self
            .skins
            .get(skin as usize)?
            .profiles
            .iter()
            .position(|entry| {
                entry.profile == profile
                    && entry.request == request
                    && entry.vertex_collapse == profile.finish.compile(ui).collapse_vertex()
                    && entry
                        .state
                        .ready()
                        .is_some_and(|(_, required)| self.admitted(required))
            })?;
        Some(SkinSelection {
            skin,
            row: row as u32,
        })
    }
    pub fn indexed_skin(
        &self,
        selection: SkinSelection,
        policy: ModelMaterialPolicy,
    ) -> Option<ShaderRef> {
        let row = self
            .skins
            .get(selection.skin as usize)?
            .profiles
            .get(selection.row as usize)?;
        if !matches!(row.request, SkinRequest::Indexed(_)) {
            return None;
        }
        let (ready, required) = row.state.ready()?;
        if !self.admitted(required) {
            return None;
        }
        let span = ready.mappings.first()?.members;
        let shader = ready.members.get(span.first as usize)?[policy.index()];
        ShaderView::direct().choice(shader, &ready.registrations, None, ShaderDefault::Named)
    }
    pub(crate) fn refresh_skin(
        &self,
        selection: SkinSelection,
        refresh: &ImageRefresh,
    ) -> Result<SkinSelection, String> {
        if let Some((_, new)) = refresh.skins.iter().find(|(old, _)| *old == selection) {
            return Ok(*new);
        }
        let row = self
            .skins
            .get(selection.skin as usize)
            .and_then(|skin| skin.profiles.get(selection.row as usize))
            .ok_or("indexed skin row")?;
        if refresh.profile(row.profile) == row.profile && row.state.ready().is_some() {
            Ok(selection)
        } else {
            Err("indexed skin refreshed row".to_owned())
        }
    }
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
            entry.request == SkinRequest::Named
                && entry.profile == profile
                && variant.matches(entry.profile, entry.vertex_collapse)
                && !entry.state.pending()
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
        name: ModelFileId,
        profile: ModelProfile,
        variant: RegistrationVariant,
    ) -> Option<ModelRegistrationId> {
        self.files.get(name)?;
        let rows = self.files.rows(name);
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
    fn file_id(&self, path: &[u8], search: Option<&SearchView>) -> Option<ModelFileId> {
        self.files.find(
            search.unwrap_or(&self.registration_vfs),
            self.names.find_path_bytes(path)?,
        )
    }
    fn file_name(&self, file: ModelFileId) -> Option<&str> {
        self.name(self.files.name(file)?)
    }
    fn source_name(&self, source: ModelSourceId) -> Option<&str> {
        self.name(self.sources.name(source)?)
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
        let ModelFile::Ready { lods, .. } = self.files.get(entry.file)? else {
            return None;
        };
        let radii = lods
            .source(0)
            .and_then(|source| self.sources.get(source))
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
        let replacement = if let Some(replacement) = &ready.replacement {
            Some((
                self.model_bindings(registration, 3, skin, policy)?,
                replacement.frames,
                replacement.rules,
            ))
        } else {
            None
        };
        let remaining = levels
            .iter()
            .flatten()
            .map(|level| level.len())
            .max()
            .unwrap_or(0)
            .max(
                replacement
                    .as_ref()
                    .map_or(0, |(bindings, ..)| bindings.len()),
            );
        Some(ModelLodBindings {
            levels,
            remaining,
            replacement,
            info: ModelLodInfo {
                native_count: lods.native_count,
                supports_lods: lods.supports_lods,
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
        let span = if slot == 3 {
            ready.replacement.as_ref()?.surfaces
        } else {
            *ready.surfaces.get(slot)?.as_ref()?
        };
        let animation = match self
            .files
            .get(self.registrations[registration.0 as usize].file)?
        {
            ModelFile::Ready { lods, .. } => lods
                .source(if slot == 3 { 0 } else { slot })
                .and_then(|source| self.sources.get(source))
                .and_then(|source| match source {
                    ModelSourceFile::Ready { animation, .. } => animation.as_deref(),
                    _ => None,
                }),
            _ => None,
        };
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
                        RegistrationState::Unattempted | RegistrationState::Reload => return None,
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
            profile: self.registrations[registration.0 as usize].profile,
            custom,
            registrations: &ready.registrations,
            resource: ShaderResource::Model(self.registrations[registration.0 as usize].file),
            default_shader: ready.default_shader,
            animation,
            timing_source: (slot == 3)
                .then(|| {
                    let native = ready.surfaces[0]?;
                    let surface = self.surfaces.get(native.first as usize)?;
                    Some(self.bindings.get(surface.skins.first as usize)?.variants[0].model)
                })
                .flatten(),
        })
    }
    /// Cold CLIENT registration reads the same already-loaded file record.
    pub fn loaded_collision_model(
        &self,
        path: &[u8],
    ) -> Option<(InlineBinding, Option<qa_core::primitives::GeometryId>)> {
        match file_lookup(&self.names, &self.files, path, Some(&self.registration_vfs))? {
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

fn supports_lods(format: ModelFormat) -> bool {
    matches!(format, ModelFormat::Md3 | ModelFormat::Mdc)
}

fn named_model_shaders(format: ModelFormat) -> bool {
    supports_lods(format) || format == ModelFormat::Md5Mesh
}

fn md5_directory(path: &str) -> String {
    let (directory, name) = path
        .rsplit_once('/')
        .map_or(("", path), |(directory, name)| {
            (&path[..directory.len() + 1], name)
        });
    format!("{directory}md5/{name}")
}

fn model_animation(model: &Model<'_>) -> Result<Option<Arc<ModelAnimation>>, String> {
    if !matches!(model.format, ModelFormat::Mdl | ModelFormat::Spr) {
        return Ok(None);
    }
    let span = |range: &std::ops::Range<usize>| -> Result<IndexSpan, String> {
        Ok(IndexSpan {
            first: u32::try_from(range.start).map_err(|_| "model animation range")?,
            count: u32::try_from(range.len()).map_err(|_| "model animation range")?,
        })
    };
    let groups = |groups: &[qa_formats::model::Group]| -> Result<Box<[TimedGroup]>, String> {
        groups
            .iter()
            .map(|group| {
                Ok(TimedGroup {
                    members: span(&group.members)?,
                    endpoints: span(&group.intervals)?,
                    bounds: group.bounds,
                })
            })
            .collect()
    };
    let animation = ModelAnimation::new(
        groups(&model.frame_groups)?,
        groups(&model.skin_groups)?,
        model.intervals.clone().into_boxed_slice(),
        model.sync,
        u32::try_from(if model.format == ModelFormat::Spr {
            model.sprites.len()
        } else {
            model.frames.len()
        })
        .map_err(|_| "model animation frames")?,
        u32::try_from(model.skins.len()).map_err(|_| "model animation skins")?,
    )?;
    Ok(Some(Arc::new(animation)))
}

fn validate_model(model: &Model<'_>) -> Result<(), String> {
    if !named_model_shaders(model.format)
        && !matches!(
            model.format,
            ModelFormat::Mdl | ModelFormat::Md2 | ModelFormat::Spr | ModelFormat::Sp2
        )
    {
        return Err(format!(
            "model format not yet registered: {:?}",
            model.format
        ));
    }
    if !named_model_shaders(model.format)
        && !matches!(model.format, ModelFormat::Spr | ModelFormat::Sp2)
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
    animation: Option<&Arc<ModelAnimation>>,
    skeleton: Option<&Arc<Skeleton>>,
    replacement_skins: Option<&[Vec<String>]>,
    replacement_missing: bool,
    shader_catalog: Option<&Result<ShaderCatalog, String>>,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
    compile_policy: ShaderCompilePolicy,
) -> Result<CompiledModel, String> {
    if model.format == ModelFormat::Md5Mesh && skeleton.is_none() {
        return Err("MD5 model requires its registered animation".to_owned());
    }
    if matches!(model.format, ModelFormat::Spr | ModelFormat::Sp2) {
        return register_sprites(
            model,
            images,
            registration,
            shared_meshes,
            animation,
            capacity,
            limits,
        );
    }
    let next_surfaces = capacity
        .surfaces
        .checked_add(model.meshes.len())
        .filter(|&count| count <= limits.surfaces && count <= u32::MAX as usize)
        .ok_or("model surface capacity")?;
    let alias = if named_model_shaders(model.format) {
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
    let mut skin_count = capacity.skins;
    for (mesh_index, (mesh, shared_mesh)) in model.meshes.iter().zip(shared_meshes).enumerate() {
        let replacement_images = replacement_skins.map(|skins| {
            skins[mesh_index]
                .iter()
                .map(|path| Image::External(path.as_bytes()))
                .collect::<Vec<_>>()
        });
        let skin_images = replacement_images.as_deref().unwrap_or(&model.skins);
        let replacement_materials = replacement_images
            .as_ref()
            .map(|skins| {
                alias_materials(
                    skins,
                    images,
                    registration,
                    model_policy,
                    replacement_missing,
                )
            })
            .transpose()?;
        let alias = replacement_materials.as_ref().or(alias.as_ref());
        let occurrence_first = registrations.len() as u32;
        if let Some(alias) = alias {
            for (skin, material) in skin_images.iter().zip(alias) {
                let raw = match skin {
                    Image::External(raw) => *raw,
                    _ => registration.path.as_bytes(),
                };
                registrations.push(
                    images
                        .assets
                        .shader_registration(raw, material.variants[0])?,
                );
            }
        }
        let count = alias.map_or_else(|| mesh.shaders.len().max(1), |materials| materials.len());
        skin_count = skin_count
            .checked_add(count)
            .filter(|&count| count <= limits.skins && count <= u32::MAX as usize)
            .ok_or("model skin capacity")?;
        let materials = if let Some(materials) = alias {
            materials
                .iter()
                .enumerate()
                .map(|(skin, material)| {
                    std::array::from_fn(|index| {
                        if index == 0 && matches!(skin_images[skin], Image::External(_)) {
                            ShaderChoice::Occurrence(ShaderOccurrence(
                                occurrence_first + skin as u32,
                            ))
                        } else {
                            ShaderChoice::Direct(material.variants[index])
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
            let packed = mesh_vertices(mesh, native, capacity, limits)?;
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
            let id = images.assets.register_animated_model(
                &packed.vertices,
                packed.frame_size,
                &packed.indices,
                material,
            )?;
            if let Some(animation) = animation {
                images
                    .assets
                    .set_model_animation(id, Arc::clone(animation))?;
            }
            if let Some(skeleton) = skeleton {
                let weights = mesh.weights.clone().into_boxed_slice();
                let vertex_weights = packed
                    .source_vertices
                    .iter()
                    .map(|&vertex| mesh.vertex_weights[vertex as usize])
                    .collect();
                let bind_normals = packed
                    .source_vertices
                    .iter()
                    .map(|&vertex| mesh.bind_normals[vertex as usize])
                    .collect();
                images.assets.set_model_skeleton(
                    id,
                    SkeletalMesh::new(Arc::clone(skeleton), weights, vertex_weights, bind_normals)?,
                )?;
            }
            if let Some(frames) = native {
                let frames: Vec<_> = frames
                    .iter()
                    .map(|frame| qa_render::assets::AliasFrame {
                        scale: frame.scale,
                        translation: frame.translation,
                    })
                    .collect();
                images
                    .assets
                    .set_alias_frames(id, &frames, &packed.alias_positions)?;
            }
            *shared_mesh = Some(id);
            id
        };
        surfaces.push(
            materials
                .into_iter()
                .enumerate()
                .map(|(skin, materials)| ModelMaterials {
                    variants: materials.map(|material| ModelChoice {
                        model: id,
                        shader: material,
                        pose: model_policy.presentation.pose,
                    }),
                    missing_image: alias
                        .and_then(|materials| materials.get(skin))
                        .and_then(|material| material.missing_image),
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
    shared_meshes: &mut [Option<ModelId>],
    animation: Option<&Arc<ModelAnimation>>,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<CompiledModel, String> {
    use qa_render::assets::{SpriteFrame, SpriteGeometry, SpriteOrientation};
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
    let mut geometry = Vec::with_capacity(model.sprites.len());
    let mut registrations = Vec::with_capacity(model.sprites.len());
    let mut first_shader = None;
    for (index, sprite) in model.sprites.iter().enumerate() {
        let image = model_image(
            sprite.image,
            images,
            registration.profile,
            registration.indexed_palette,
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
                    } else if model.format == ModelFormat::Spr {
                        if matches!(
                            registration.profile.rules,
                            RuleSetId::Quake | RuleSetId::QuakeWorld
                        ) {
                            AlphaFunc::GreaterThan0666
                        } else {
                            AlphaFunc::GreaterZero
                        }
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
        first_shader.get_or_insert(variants[0]);
        let shader = if let Image::External(raw) = sprite.image {
            let occurrence = ShaderOccurrence(registrations.len() as u32);
            registrations.push(images.assets.shader_registration(raw, variants[0])?);
            ShaderChoice::Occurrence(occurrence)
        } else {
            ShaderChoice::Direct(variants[0])
        };
        frames.push(std::array::from_fn(|index| {
            if index == 0 {
                shader
            } else {
                ShaderChoice::Direct(variants[index])
            }
        }));
        let x = i64::from(sprite.origin[0]);
        let y = i64::from(sprite.origin[1]);
        let width = i64::from(sprite.width);
        let height = i64::from(sprite.height);
        geometry.push(SpriteFrame {
            extents: if model.format == ModelFormat::Spr {
                [x as f32, (x + width) as f32, y as f32, (y - height) as f32]
            } else {
                [
                    (-x) as f32,
                    (width - x) as f32,
                    (height - y) as f32,
                    (-y) as f32,
                ]
            },
        });
    }
    let shared_mesh = shared_meshes
        .first_mut()
        .ok_or("missing sprite mesh slot")?;
    let model_id = match *shared_mesh {
        Some(model) => model,
        None => {
            let geometry = SpriteGeometry {
                frames: geometry.into_boxed_slice(),
                orientation: if model.format == ModelFormat::Spr {
                    SpriteOrientation::try_from(model.orientation)?
                } else {
                    SpriteOrientation::default()
                },
                beam_length: model.beam_length,
                wrap_frames: model.format == ModelFormat::Sp2,
            };
            let model = images
                .assets
                .register_sprite(geometry, first_shader.ok_or("missing sprite frame")?)?;
            if let Some(animation) = animation {
                images
                    .assets
                    .set_model_animation(model, Arc::clone(animation))?;
            }
            *shared_mesh = Some(model);
            model
        }
    };
    let frames = frames
        .into_iter()
        .map(|shaders: [ShaderChoice; 4]| ModelMaterials {
            variants: shaders.map(|shader| ModelChoice {
                model: model_id,
                shader,
                pose: AliasPose::default(),
            }),
            missing_image: None,
        })
        .collect();
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
    indexed_palette: Option<PaletteId>,
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
            let palette = indexed_palette.ok_or(ResourceError::Asset(
                "indexed model requires a source palette",
            ))?;
            let texture = IndexedTexture::load_base(width, height, pixels, transparent)
                .map_err(ResourceError::Asset)?;
            let image = images.register_indexed(texture, palette, options.palette)?;
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
            } else if qa_formats::image::ImageFormat::from_path(path.as_bytes())
                == Some(qa_formats::image::ImageFormat::Lmp)
            {
                images
                    .indexed_lmp(
                        path,
                        indexed_palette
                            .ok_or(ResourceError::Asset("LMP skin requires a source palette"))?,
                        transparent,
                        usage,
                    )?
                    .id
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

struct AliasMaterial {
    variants: [ShaderBindingId; 4],
    missing_image: Option<NameId>,
}

fn alias_materials(
    skins: &[Image<'_>],
    images: &mut Images<'_>,
    registration: &Registration,
    model_policy: ModelPolicy,
    allow_missing: bool,
) -> Result<Vec<AliasMaterial>, String> {
    let options = registration.profile;
    let mut materials = Vec::with_capacity(skins.len());
    let mut default_image = None;
    for (index, skin) in skins.iter().enumerate() {
        let result = model_image(
            *skin,
            images,
            options,
            registration.indexed_palette,
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
        let variants = alias_image_materials(
            images.assets,
            &registration.path,
            index,
            image,
            model_policy,
            missing_skin,
        )?;
        let missing_image = if missing_skin {
            match skin {
                Image::External(path) => {
                    let path = canonical_path(
                        std::str::from_utf8(path).map_err(|_| "skin path is not UTF-8")?,
                    );
                    Some(images.assets.intern_resource_name(path.as_bytes())?)
                }
                Image::Indexed { .. } => None,
            }
        } else {
            None
        };
        materials.push(AliasMaterial {
            variants,
            missing_image,
        });
    }
    Ok(materials)
}

fn alias_image_materials(
    assets: &mut Assets,
    path: &str,
    index: usize,
    image: ImageId,
    model_policy: ModelPolicy,
    missing_skin: bool,
) -> Result<[ShaderBindingId; 4], String> {
    let mut variants = [ShaderBindingId(0); 4];
    for policy in ModelMaterialPolicy::ALL {
        let material = assets.register_material(
            &format!("{}#skin{index}:{}", path, policy.index()),
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
                software_alias: model_policy
                    .presentation
                    .software_alias
                    .then_some(SoftwareAlias {
                        missing_skin,
                        infrared_map: model_policy.presentation.infrared_map,
                        fullbright_light: policy
                            .fullbright
                            .then_some(model_policy.presentation.fullbright_light),
                        alpha_cutoffs: policy
                            .translucent
                            .then_some(model_policy.presentation.alpha_cutoffs),
                    }),
                ..MaterialSettings::default()
            },
        )?;
        variants[policy.index()] =
            assets.register_shader_binding(ShaderBinding::material(material, LightSource::None))?;
    }
    Ok(variants)
}

struct PackedMesh {
    vertices: Vec<Vertex>,
    frame_size: usize,
    indices: Vec<u32>,
    alias_positions: Vec<[u8; 3]>,
    source_vertices: Vec<u32>,
}

fn mesh_vertices(
    mesh: &Mesh<'_>,
    native: Option<&[qa_formats::model::Frame<'_>]>,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<PackedMesh, String> {
    if mesh.vertices_per_frame == 0 {
        return Ok(PackedMesh {
            vertices: Vec::new(),
            frame_size: 0,
            indices: Vec::new(),
            alias_positions: Vec::new(),
            source_vertices: Vec::new(),
        });
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
    Ok(PackedMesh {
        vertices,
        frame_size: pairs.len(),
        indices,
        alias_positions: packed,
        source_vertices: if mesh.vertex_weights.is_empty() {
            Vec::new()
        } else {
            pairs.into_iter().map(|(vertex, _)| vertex).collect()
        },
    })
}
