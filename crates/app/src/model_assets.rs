//! Cold model registration. Native config ordinals resolve through filenames;
//! inline ordinals retain their explicit world and existing surface ranges.
use qa_content::vfs::Vfs;
use qa_core::names::{NameTable, canonical_path};
use qa_core::primitives::{IndexSpan, NameId, RuleSetId};
use qa_formats::archive::ArchiveReader;
use qa_formats::model::{Image, Mesh, Model, ModelFormat};
use qa_render::assets::{MaterialSettings, SoftwareAlias, Stage, StageTexture};
use qa_render::material::resources::{ImageSettings, ImageUse, Images};
use qa_render::shader::{AlphaGen, RgbGen, StageBlend};
use qa_render::surface_cache::IndexedTexture;
use qa_render::world::WorldId;
use qa_render::{Assets, MaterialId, ModelId, PaletteId, Vertex};

#[derive(Clone, Copy, Debug)]
pub struct ModelRequest<'a> {
    pub path: &'a str,
    /// Image upload policy is independent of model format and map geometry.
    pub rules: RuleSetId,
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
            rules,
            palette,
            image_settings: ImageSettings::native(rules),
            required: true,
        }
    }
}

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
    pub error: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelCapacity {
    pub names: usize,
    pub name_bytes: usize,
    pub bindings: usize,
    pub skins: usize,
    pub frame_vertices: usize,
    pub indices: usize,
    pub inline_bindings: usize,
}

#[derive(Clone, Copy)]
struct Options {
    rules: RuleSetId,
    palette: Option<PaletteId>,
    image_settings: ImageSettings,
    required: bool,
}
impl Options {
    fn same_images(self, other: Self) -> bool {
        self.rules == other.rules && self.image_settings == other.image_settings
    }
}
struct Registration {
    path: String,
    options: Options,
}

pub struct ModelCatalog {
    names: NameTable,
    models: Box<[Option<IndexSpan>]>,
    bindings: Box<[ModelMaterials]>,
    worlds: Box<[IndexSpan]>,
    inline: Box<[InlineBinding]>,
    capacity: ModelCapacity,
    pub diagnostics: Box<[ModelDiagnostic]>,
}
impl ModelCatalog {
    /// All I/O, names and mesh/image/material registration finish before the
    /// renderer freezes Assets. The returned catalog never grows at runtime.
    pub fn load<'a>(
        vfs: &Vfs,
        reader: &mut ArchiveReader,
        assets: &mut Assets,
        requests: impl IntoIterator<Item = ModelRequest<'a>>,
        worlds: impl IntoIterator<Item = WorldId>,
        limits: ModelLimits,
    ) -> Result<Self, String> {
        let mut registrations = Vec::<Registration>::new();
        let mut name_bytes = 0usize;
        for request in requests {
            if request.path.is_empty() || request.path.starts_with('*') {
                continue;
            }
            if registrations.len() == limits.models {
                return Err("model registration capacity".to_owned());
            }
            let path = canonical_path(request.path);
            name_bytes = name_bytes
                .checked_add(path.len())
                .filter(|&bytes| bytes <= limits.name_bytes)
                .ok_or("model name capacity")?;
            registrations.push(Registration {
                path,
                options: Options {
                    rules: request.rules,
                    palette: request.palette,
                    image_settings: request.image_settings,
                    required: request.required,
                },
            });
        }
        registrations.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        let mut unique = Vec::<Registration>::with_capacity(registrations.len());
        for registration in registrations {
            if let Some(previous) = unique.last_mut()
                && previous.path == registration.path
            {
                if !previous.options.same_images(registration.options)
                    || previous.options.palette != registration.options.palette
                {
                    return Err(format!(
                        "conflicting model image registration: {}",
                        registration.path
                    ));
                }
                previous.options.required |= registration.options.required;
                continue;
            }
            unique.push(registration);
        }
        let names = NameTable::load(
            unique
                .iter()
                .map(|registration| registration.path.as_bytes()),
        )
        .map_err(|error| format!("model names: {error}"))?;
        let mut models = vec![None; names.len()].into_boxed_slice();
        let mut bindings = Vec::new();
        let mut diagnostics = Vec::new();
        let mut capacity = ModelCapacity {
            names: names.len(),
            name_bytes: unique
                .iter()
                .map(|registration| registration.path.len())
                .sum(),
            ..ModelCapacity::default()
        };
        // Keep one existing image-name/recipe cache per upload profile across
        // the entire catalog. Palettes are already part of its indexed keys.
        let mut groups: Vec<Vec<Registration>> = Vec::new();
        for registration in unique {
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group[0].options.same_images(registration.options))
            {
                group.push(registration);
            } else {
                groups.push(vec![registration]);
            }
        }
        for group in groups {
            let options = group[0].options;
            let mut images = Images::new(vfs, assets, options.rules, options.image_settings)
                .map_err(|error| format!("model images: {error:?}"))?;
            for registration in group {
                let name = names
                    .find(registration.path.as_bytes())
                    .ok_or("missing registered model name")?;
                match register(
                    vfs,
                    reader,
                    &mut images,
                    &registration,
                    &mut capacity,
                    limits,
                ) {
                    Ok(skins) => {
                        let first =
                            u32::try_from(bindings.len()).map_err(|_| "model binding table")?;
                        let count =
                            u32::try_from(skins.len()).map_err(|_| "model binding table")?;
                        bindings.extend(skins);
                        models[name.0 as usize] = Some(IndexSpan { first, count });
                    }
                    Err(error) if registration.options.required => {
                        return Err(format!("model {}: {error}", registration.path));
                    }
                    Err(error) => diagnostics.push(ModelDiagnostic { name, error }),
                }
            }
        }
        let mut world_ranges = vec![IndexSpan::default(); assets.worlds().len()].into_boxed_slice();
        let mut inline = Vec::new();
        for id in worlds {
            let range = world_ranges
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
        capacity.skins = bindings.len();
        capacity.bindings = bindings.len() * ModelMaterialPolicy::ALL.len();
        capacity.inline_bindings = inline.len();
        Ok(Self {
            names,
            models,
            bindings: bindings.into_boxed_slice(),
            worlds: world_ranges,
            inline: inline.into_boxed_slice(),
            capacity,
            diagnostics: diagnostics.into_boxed_slice(),
        })
    }

    /// Only config publication/update resolves a filename. Frame callers keep
    /// the resulting NameId or ModelBinding and perform numeric lookup below.
    pub fn name_for_path(&self, path: &str) -> Option<NameId> {
        self.names.find_path(path)
    }
    pub fn name(&self, name: NameId) -> Option<&str> {
        std::str::from_utf8(self.names.get(name)?).ok()
    }
    pub fn resolve_name(&self, name: NameId, skin: u32) -> Option<ModelBinding> {
        self.resolve_name_with(name, skin, ModelMaterialPolicy::default())
    }
    pub fn resolve_name_with(
        &self,
        name: NameId,
        skin: u32,
        policy: ModelMaterialPolicy,
    ) -> Option<ModelBinding> {
        let span = (*self.models.get(name.0 as usize)?)?;
        // Native R_DrawAliasModel uses skin zero for an out-of-range skinnum.
        let skin = if skin < span.count { skin } else { 0 };
        Some(self.bindings.get((span.first + skin) as usize)?.variants[policy.index()])
    }
    pub fn resolve_path(&self, path: &str, skin: u32) -> Option<ModelBinding> {
        self.resolve_name(self.name_for_path(path)?, skin)
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

fn register(
    vfs: &Vfs,
    reader: &mut ArchiveReader,
    images: &mut Images<'_>,
    registration: &Registration,
    capacity: &mut ModelCapacity,
    limits: ModelLimits,
) -> Result<Vec<ModelMaterials>, String> {
    let bytes = vfs
        .read_file(registration.path.as_bytes(), limits.file_bytes, reader)
        .map_err(|error| error.to_string())?;
    let model = Model::parse(&bytes).map_err(|error| format!("decode: {error:?}"))?;
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
    let skin_count = capacity
        .skins
        .checked_add(model.skins.len())
        .filter(|&count| count <= limits.skins && count <= u32::MAX as usize)
        .ok_or("model skin capacity")?;
    let mesh = model.meshes.first().ok_or("model has no mesh")?;
    let (vertices, frame_size, indices) = mesh_vertices(mesh, capacity, limits)?;
    let options = registration.options;
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
                    .palette
                    .ok_or("indexed model requires a selected palette")?;
                let texture = IndexedTexture::load_base(width, height, pixels, None)?;
                let image = images.assets.register_indexed_image(texture, palette)?;
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
                if qa_formats::image::ImageFormat::from_path(path.as_bytes())
                    == Some(qa_formats::image::ImageFormat::Pcx)
                {
                    images
                        .indexed_pcx(
                            path,
                            options
                                .palette
                                .ok_or("PCX skin requires a selected palette")?,
                            None,
                            None,
                            usage,
                        )
                        .map_err(|error| format!("skin {path}: {error:?}"))?
                } else {
                    images
                        .raster(path, usage)
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
    let material = materials.first().ok_or("model has no skin material")?[0];
    let id = images
        .assets
        .register_animated_model(&vertices, frame_size, &indices, material)?;
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
