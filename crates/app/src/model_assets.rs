//! Cold model registration. Native config ordinals resolve through filenames;
//! inline ordinals retain their explicit world and existing surface ranges.
use qa_compat::services::{CallError, ModelLoader, ModelLookup};
use qa_content::vfs::Vfs;
use qa_core::names::{NameTable, canonical_path};
use qa_core::primitives::{Bounds, IndexSpan, NameId, RuleSetId};
use qa_formats::archive::ArchiveReader;
use qa_formats::model::{Image, Mesh, Model, ModelFormat};
use qa_render::assets::{MaterialSettings, SoftwareAlias, Stage, StageTexture};
use qa_render::material::resources::{ImageSettings, ImageUse, Images, ImagesState, IndexedTarget};
use qa_render::shader::{AlphaGen, RgbGen, StageBlend};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelBinding {
    pub model: ModelId,
    pub material: MaterialId,
    /// Native model presentation chosen at registration, independent of wire.
    pub rules: RuleSetId,
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
    Failed(FileError),
}
struct ProfileImages {
    profile: ModelProfile,
    // Temporarily taken only while Images borrows the separately owned Assets.
    state: Option<Result<ImagesState, String>>,
}

pub struct ModelCatalog {
    names: NameTable,
    files: Box<[ModelFile]>,
    models: Box<[IndexSpan]>,
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

    fn ensure_file(&mut self, vfs: &Vfs, path: &str) -> Result<ModelMetadata, FileError> {
        if let Some(name) = self.names.find(path.as_bytes()) {
            return match &self.files[name.0 as usize] {
                ModelFile::Ready(metadata) => Ok(*metadata),
                ModelFile::Failed(error) => Err(error.clone()),
                ModelFile::Empty => Err(FileError {
                    kind: CallError::File,
                    message: "empty model filename".to_owned(),
                }),
            };
        }
        let next = self
            .registrations
            .len()
            .checked_add(self.profiles.len())
            .filter(|&count| count <= self.limits.models && count <= u32::MAX as usize)
            .ok_or_else(|| FileError::capacity("model registration capacity"))?;
        let name = self
            .names
            .intern(path.as_bytes())
            .map_err(|_| FileError::capacity("model name capacity"))?;
        let span = IndexSpan {
            first: self.registrations.len() as u32,
            count: (next - self.registrations.len()) as u32,
        };
        self.models.push(span);
        self.capacity.names = self.names.len();
        self.capacity.name_bytes += path.len();
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
        self.files.push(match &metadata {
            Ok(metadata) => ModelFile::Ready(*metadata),
            Err(error) => ModelFile::Failed(error.clone()),
        });
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
        metadata
    }

    pub fn finish(self) -> (Assets, ModelCatalog) {
        let catalog = ModelCatalog {
            names: self.names,
            files: self.files.into_boxed_slice(),
            models: self.models.into_boxed_slice(),
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

fn metadata_lookup(names: &NameTable, files: &[ModelFile], path: &[u8]) -> Option<ModelMetadata> {
    let name = names.find_path(std::str::from_utf8(path).ok()?)?;
    match files.get(name.0 as usize)? {
        ModelFile::Ready(metadata) => Some(*metadata),
        ModelFile::Empty | ModelFile::Failed(_) => None,
    }
}
impl ModelLookup for ModelCatalogBuilder {
    fn bounds(&self, path: &[u8], _source: RuleSetId) -> Option<Bounds> {
        metadata_lookup(&self.names, &self.files, path).map(|metadata| metadata.bounds)
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
            .map(|metadata| metadata.bounds)
            .map_err(|error| error.kind)
    }
}
impl ModelLookup for ModelCatalog {
    fn bounds(&self, path: &[u8], _source: RuleSetId) -> Option<Bounds> {
        self.metadata(path).map(|metadata| metadata.bounds)
    }
}
impl ModelCatalog {
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
        metadata_lookup(&self.names, &self.files, path)
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
        let span = *self.worlds.get(world.0 as usize)?;
        if index >= span.count {
            return None;
        }
        self.inline.get((span.first + index) as usize).copied()
    }
    pub fn capacity(&self) -> ModelCapacity {
        self.capacity
    }
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
                if matches!(
                    options.source_rules,
                    RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
                ) && qa_formats::image::ImageFormat::from_path(path.as_bytes())
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
                    software_alias: matches!(
                        options.rules,
                        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
                    )
                    .then_some(SoftwareAlias {
                        fullbright_light: policy.fullbright.then_some([128.0, 64.0]),
                        alpha_cutoffs: policy.translucent.then_some([0.33, 0.66]),
                    }),
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
                rules: options.rules,
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
