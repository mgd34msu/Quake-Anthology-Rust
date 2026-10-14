//! File-boundary conversion into the one runtime world/material table.
use super::{
    load_catalog,
    resources::{
        ImageRole, ImageSettings, ImageUse, Images, IndexedTarget, ResolvedImage, ResourceError,
    },
};
use crate::{
    Assets, BlendPhase, CpuPresentation, LightStyle, PaletteOperation, PaletteTransform,
    PerspectiveStep, Refdef,
    assets::{
        CubeSkyParams, DepthFunc, Filter, Flow, ImageId, MaterialId, MaterialSettings, Sampler,
        Sky, SkyDistance, SoftwareTiled, Stage, StageTexture, TcMod, TextureIntensity, UvWarp,
        Wrap, upload,
    },
    lightmap::{
        Atlas, AtlasBuilder, AtlasRegion, LightmapConversion, LightmapError, Q1GlScale,
        shift_quake3_rgb,
    },
    shader::{
        AlphaFunc, AlphaGen, BlendFactor, Cull, Deform, RgbGen, ShaderCatalog, ShaderDef,
        StageBlend, TexCoordGen, TextureMap, canonical_path,
    },
    sky::{CloudSphere, LayeredSphere, split_layered_sky},
    surface_cache::{IndexedLighting, IndexedTexture},
    world::{
        SurfaceMaterial, WorldId, WorldPresentationId,
        geometry::{
            GeometryError, GeometryOptions, LightSource, WorldGeometry,
            grid::{GridFan, GridOptions, subdivide_surface},
            load_geometry,
        },
        load_visibility,
    },
};
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_core::text::FixedText;
use qa_formats::bsp::Map;

/// Worldspawn/configstring data copied at the entity boundary. Rendering owns
/// no entity-lump parser and resolves this name only during asset registration.
#[derive(Clone, Debug)]
pub struct SkyEnvironment {
    name: FixedText<63>,
    pub degrees_per_second: f32,
    pub axis: qa_core::primitives::Vec3,
}
impl SkyEnvironment {
    pub fn new(
        name: &[u8],
        degrees_per_second: f32,
        axis: qa_core::primitives::Vec3,
    ) -> Result<Self, &'static str> {
        if name.is_empty()
            || name.len() > FixedText::<63>::CAPACITY
            || !degrees_per_second.is_finite()
            || !axis.is_finite()
            || std::str::from_utf8(name).is_err()
            || name
                .iter()
                .any(|&byte| byte == 0 || byte == b'/' || byte == b'\\')
        {
            return Err("invalid sky environment");
        }
        let mut text = FixedText::default();
        text.set_bytes(name)
            .map_err(|_| "invalid sky environment")?;
        Ok(Self {
            name: text,
            degrees_per_second,
            axis,
        })
    }
    pub fn name(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.name.as_bytes())
    }
}
impl Default for SkyEnvironment {
    fn default() -> Self {
        let mut name = FixedText::default();
        // The original default is a constant six-byte name within this capacity.
        let _ = name.set_bytes(b"unit1_");
        Self {
            name,
            degrees_per_second: 0.0,
            axis: qa_core::primitives::Vec3::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorldLoadOptions {
    pub geometry: GeometryOptions,
    pub map_overbright: u8,
    pub renderer_overbright: u8,
    pub sky_environment: SkyEnvironment,
}
impl Default for WorldLoadOptions {
    fn default() -> Self {
        Self {
            geometry: GeometryOptions::default(),
            map_overbright: 2,
            renderer_overbright: 0,
            sky_environment: SkyEnvironment::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PresentationDefaults {
    pub cpu: CpuPresentation,
    pub perspective_step: PerspectiveStep,
    pub blend_phase: BlendPhase,
    pub palette_transform: Option<PaletteTransform>,
    pub lightstyles: [LightStyle; 256],
    pub identity_light: f32,
}
impl Default for PresentationDefaults {
    fn default() -> Self {
        let view = Refdef::default();
        Self {
            cpu: view.cpu_presentation,
            perspective_step: view.perspective_step,
            blend_phase: view.blend_phase,
            palette_transform: view.palette_transform,
            lightstyles: view.lightstyles,
            identity_light: view.identity_light,
        }
    }
}
impl PresentationDefaults {
    pub fn apply(&self, view: &mut Refdef) {
        view.cpu_presentation = self.cpu;
        view.perspective_step = self.perspective_step;
        view.blend_phase = self.blend_phase;
        view.palette_transform = self.palette_transform;
        view.lightstyles = self.lightstyles;
        view.identity_light = self.identity_light;
    }
}
/// Exact cold policies. None is an explicit RGB target, never a palette fallback.
#[derive(Clone, Copy, Debug)]
pub struct WorldPresentationProfile {
    pub rules: RuleSetId,
    pub indexed_palette: Option<crate::PaletteId>,
    pub image_settings: ImageSettings,
    pub q1_gl_scale: Q1GlScale,
    pub q2_modulate: f32,
}
impl PartialEq for WorldPresentationProfile {
    fn eq(&self, other: &Self) -> bool {
        self.rules == other.rules
            && self.indexed_palette == other.indexed_palette
            && self.image_settings == other.image_settings
            && self.q1_gl_scale == other.q1_gl_scale
            && self.q2_modulate.to_bits() == other.q2_modulate.to_bits()
    }
}
impl WorldPresentationProfile {
    pub fn native(rules: RuleSetId, indexed_palette: Option<crate::PaletteId>) -> Self {
        Self {
            rules,
            indexed_palette,
            image_settings: ImageSettings::native(rules),
            q1_gl_scale: Q1GlScale::OriginalOverbright,
            q2_modulate: 1.0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct LoadedWorldPresentation {
    pub id: WorldPresentationId,
    pub profile: WorldPresentationProfile,
    pub defaults: PresentationDefaults,
}
pub struct LoadedWorld {
    pub world: WorldId,
    /// Request order is preserved; exact duplicate profiles share one id.
    pub presentations: Box<[LoadedWorldPresentation]>,
    pub diagnostics: Box<[String]>,
}
#[derive(Debug)]
pub enum WorldMaterialError {
    Resource(ResourceError),
    Geometry(GeometryError),
    Lightmap(LightmapError),
    Boundary(&'static str),
    Catalog(super::CatalogLoadError),
}
impl From<ResourceError> for WorldMaterialError {
    fn from(e: ResourceError) -> Self {
        Self::Resource(e)
    }
}
impl From<GeometryError> for WorldMaterialError {
    fn from(e: GeometryError) -> Self {
        Self::Geometry(e)
    }
}
impl From<LightmapError> for WorldMaterialError {
    fn from(e: LightmapError) -> Self {
        Self::Lightmap(e)
    }
}

/// Presentation is caller-selected; BSP decoding, flags and coordinates retain
/// their source semantics independently of the selected view/image policy.
pub fn load_world(
    vfs: &Vfs,
    map: &Map<'_>,
    assets: &mut Assets,
    source_palette: Option<crate::PaletteId>,
    profiles: &[WorldPresentationProfile],
    mut options: WorldLoadOptions,
) -> Result<LoadedWorld, WorldMaterialError> {
    let geometry_source = map.bsp.format.rule_set();
    if profiles.is_empty() {
        return Err(WorldMaterialError::Boundary("missing world presentation"));
    }
    if options.renderer_overbright > 2
        || options.map_overbright > 8
        || options.map_overbright < options.renderer_overbright
    {
        return Err(WorldMaterialError::Boundary("invalid overbright settings"));
    }
    if (!matches!(geometry_source, RuleSetId::Quake3) && source_palette.is_none())
        || source_palette.is_some_and(|id| assets.palette(id).is_none())
        || profiles.iter().any(|profile| {
            !profile.q2_modulate.is_finite()
                || profile
                    .indexed_palette
                    .is_some_and(|id| assets.palette(id).is_none())
        })
    {
        return Err(WorldMaterialError::Boundary(
            "invalid world presentation profile",
        ));
    }
    // Nonlinear Q3 color normalization precedes patch interpolation once. These
    // geometry settings are common to every admitted presentation profile.
    if matches!(geometry_source, RuleSetId::Quake3) {
        options.geometry.vertex_color_shift =
            (options.map_overbright - options.renderer_overbright) as i8;
    }
    let mut geometry = load_geometry(map, options.geometry)?;
    let visibility =
        load_visibility(map).map_err(|_| WorldMaterialError::Boundary("world visibility"))?;
    let layout = prepare_lightmap_layout(&geometry)?;
    for surface in &geometry.surfaces {
        if !matches!(geometry_source, RuleSetId::Quake3)
            && let Some(region) = layout.regions[surface.source_id as usize]
        {
            for vertex in &mut geometry.vertices[surface.vertices.range()] {
                vertex.vertex.lightmap_coord =
                    if let Some(sample) = surface.lightmap_sample(vertex.vertex.position) {
                        region.uv_at_sample(sample)
                    } else {
                        region.uv_from_texture(vertex.vertex.texcoord, surface.texture_minima)
                    };
            }
        }
    }
    // GPU interpolation matches native pre-fragmented liquid/sky polygons;
    // the original convex boundaries remain the CPU edge-scanner input.
    let mut legacy_kinds = Vec::with_capacity(geometry.surfaces.len());
    for source in 0..geometry.surfaces.len() {
        let surface = &geometry.surfaces[source];
        let legacy_name = if matches!(geometry_source, RuleSetId::Quake) {
            surface
                .source_texture
                .and_then(|i| map.textures.get(i as usize))
                .and_then(Option::as_ref)
                .map(|mip| mip.name)
        } else {
            None
        };
        let kind = legacy_kind(
            geometry_source,
            legacy_name.unwrap_or_default(),
            surface.source_flags,
        );
        legacy_kinds.push(kind);
        let (warp, sky) = kind;
        if warp || sky {
            let texture_offset = if warp {
                [
                    surface.texture_projection[0][3],
                    surface.texture_projection[1][3],
                ]
            } else {
                [0.0; 2]
            };
            subdivide_surface(
                &mut geometry,
                source,
                GridOptions {
                    spacing: if matches!(geometry_source, RuleSetId::Quake) {
                        128.0
                    } else {
                        64.0
                    },
                    fan: if matches!(geometry_source, RuleSetId::Quake) {
                        GridFan::PolygonAnchor
                    } else {
                        GridFan::CenterFan
                    },
                    texture_offset,
                    ..GridOptions::default()
                },
            )?;
        }
    }
    let catalog = load_catalog(vfs).map_err(WorldMaterialError::Catalog)?;
    let mut diagnostics = Vec::new();
    let mut distinct = Vec::<WorldPresentationProfile>::new();
    let mut request_indices = Vec::with_capacity(profiles.len());
    for &profile in profiles {
        let index = if let Some(index) = distinct.iter().position(|existing| *existing == profile) {
            index
        } else {
            let index = distinct.len();
            distinct.push(profile);
            index
        };
        request_indices.push(index);
    }
    let mut variants = Vec::with_capacity(distinct.len());
    let mut defaults = Vec::with_capacity(distinct.len());
    for &profile in &distinct {
        let presentation = presentation_defaults(profile, &options);
        let conversion = if matches!(geometry_source, RuleSetId::Quake) {
            LightmapConversion::Scalar(profile.q1_gl_scale)
        } else {
            LightmapConversion::Rgb {
                modulate: profile.q2_modulate,
            }
        };
        let lightmaps = prepare_lightmap_pages(
            &geometry,
            &layout,
            assets,
            &options,
            &presentation.lightstyles,
            conversion,
        )?;
        variants.push(compile_presentation(
            vfs,
            map,
            assets,
            source_palette,
            profile,
            &options,
            &geometry,
            &legacy_kinds,
            &catalog,
            &layout.regions,
            &lightmaps,
            conversion,
            &mut diagnostics,
        )?);
        defaults.push(presentation);
    }
    let variant_slices: Vec<_> = variants.iter().map(Vec::as_slice).collect();
    let world = assets
        .register_world(geometry, visibility, &variant_slices)
        .map_err(WorldMaterialError::Boundary)?;
    if !matches!(geometry_source, RuleSetId::Quake3) {
        let lighting = crate::lighting::PointLighting::load(
            matches!(geometry_source, RuleSetId::Quake),
            legacy_kinds.iter().map(|&(warp, sky)| !warp && !sky),
        );
        assets
            .set_point_lighting(world, lighting)
            .map_err(WorldMaterialError::Boundary)?;
    }
    match crate::lighting::LightGrid::load_map(
        map,
        (options.map_overbright - options.renderer_overbright) as i8,
    ) {
        Ok(grid) => assets
            .set_light_grid(world, grid)
            .map_err(WorldMaterialError::Boundary)?,
        Err(error) => diagnostics.push(error.to_owned()),
    }
    let first = assets
        .world(world)
        .ok_or(WorldMaterialError::Boundary("missing registered world"))?
        .presentations()
        .first;
    let presentations = request_indices
        .into_iter()
        .map(|index| LoadedWorldPresentation {
            id: WorldPresentationId(first + index as u32),
            profile: distinct[index],
            defaults: defaults[index],
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Ok(LoadedWorld {
        world,
        presentations,
        diagnostics: diagnostics.into_boxed_slice(),
    })
}

fn presentation_defaults(
    profile: WorldPresentationProfile,
    options: &WorldLoadOptions,
) -> PresentationDefaults {
    let presentation_rules = profile.rules;
    let mut presentation = PresentationDefaults::default();
    // Native presentation choices are resolved here, independently of movement,
    // client protocol and module format. There is no family branch in drawing.
    if matches!(presentation_rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
        presentation.lightstyles[0].indexed_scale = 264;
        presentation.lightstyles[0].rgb = [264.0 / 256.0; 3];
        presentation.blend_phase = BlendPhase::FinalPalette;
        presentation.palette_transform = Some(PaletteTransform::default());
    } else if matches!(
        presentation_rules,
        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
    ) {
        for style in &mut presentation.lightstyles {
            style.indexed_scale = 384;
        }
        // The original portable Q2 D_DrawSpans16 body subdivides by eight.
        presentation.perspective_step = PerspectiveStep::Eight;
        presentation.blend_phase = BlendPhase::FinalPalette;
        presentation.palette_transform = Some(PaletteTransform {
            operation: PaletteOperation::ScreenBlend,
            ..PaletteTransform::default()
        });
    } else {
        presentation.identity_light = 1.0 / (1u32 << options.renderer_overbright) as f32;
    }
    if let Some(palette) = profile.indexed_palette {
        presentation.cpu = CpuPresentation::Indexed {
            palette,
            lighting: if matches!(presentation_rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                IndexedLighting::Gray
            } else {
                IndexedLighting::NativeRgb
            },
            ambient: 0,
            fullbright: false,
        };
    }
    presentation
}

#[expect(
    clippy::too_many_arguments,
    reason = "One cold converter receives the shared geometry and atlas plus an explicit exact resource profile."
)]
fn compile_presentation(
    vfs: &Vfs,
    map: &Map<'_>,
    assets: &mut Assets,
    palette: Option<crate::PaletteId>,
    profile: WorldPresentationProfile,
    options: &WorldLoadOptions,
    geometry: &WorldGeometry,
    legacy_kinds: &[(bool, bool)],
    catalog: &ShaderCatalog,
    regions: &[Option<AtlasRegion>],
    lightmaps: &[Option<ImageId>],
    lightmap_conversion: LightmapConversion,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<SurfaceMaterial>, WorldMaterialError> {
    let geometry_source = map.bsp.format.rule_set();
    let presentation_rules = profile.rules;
    let mut images = Images::new(
        vfs,
        assets,
        geometry_source,
        presentation_rules,
        profile.image_settings,
        palette.zip(profile.indexed_palette),
    )?;
    images.set_raster_target(profile.indexed_palette.map(|palette| IndexedTarget {
        palette,
        transparent_index: Some(255),
    }))?;
    let mut textures = Vec::with_capacity(map.textures.len());
    let mut layered_skies = Vec::with_capacity(map.textures.len());
    if matches!(geometry_source, RuleSetId::Quake) {
        for texture in &map.textures {
            textures.push(match texture {
                // R_InitSky uploads the two split layers directly, rather
                // than uploading the original 256x128 texture as a surface.
                Some(mip) if legacy_sky_name(&mip.name) => ImageId(0),
                Some(mip) => images.embedded(
                    mip,
                    palette.ok_or(WorldMaterialError::Boundary("missing legacy palette"))?,
                    mip.name.starts_with(b"{"),
                )?,
                None => ImageId(0),
            });
            layered_skies.push(match texture {
                Some(mip) if legacy_sky_name(&mip.name) => Some(load_layered_sky(
                    mip,
                    palette.ok_or(WorldMaterialError::Boundary("missing sky palette"))?,
                    &mut images,
                )?),
                _ => None,
            });
        }
    }
    let mut bindings = Vec::with_capacity(geometry.surfaces.len());
    for (surface, &kind) in geometry.surfaces.iter().zip(legacy_kinds) {
        let (warp, sky) = kind;
        if surface.no_draw {
            bindings.push(SurfaceMaterial::default());
            continue;
        }
        let (name, image, mut scale, flags) = if matches!(geometry_source, RuleSetId::Quake) {
            let index = surface.source_texture.ok_or(WorldMaterialError::Boundary(
                "missing embedded texture index",
            ))? as usize;
            let mip = map
                .textures
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(WorldMaterialError::Boundary("missing embedded texture"))?;
            let name = std::str::from_utf8(mip.name)
                .map_err(|_| WorldMaterialError::Boundary("invalid texture name"))?
                .to_owned();
            (
                name,
                textures[index],
                [1.0 / mip.width as f32, 1.0 / mip.height as f32],
                surface.source_flags,
            )
        } else if matches!(geometry_source, RuleSetId::Quake2) {
            let info = &map.texture_info[surface
                .source_texture_info
                .ok_or(WorldMaterialError::Boundary("missing texture info"))?
                as usize];
            let name = std::str::from_utf8(info.name)
                .map_err(|_| WorldMaterialError::Boundary("invalid texture name"))?
                .to_owned();
            let image = if sky {
                ImageId(0)
            } else {
                images.wal(
                    &format!("textures/{name}.wal"),
                    palette.ok_or(WorldMaterialError::Boundary("missing legacy palette"))?,
                    false,
                )?
            };
            let texture = images
                .assets
                .image(image)
                .ok_or(WorldMaterialError::Boundary("missing registered texture"))?;
            let scale = [1.0 / texture.width as f32, 1.0 / texture.height as f32];
            (name, image, scale, surface.source_flags)
        } else {
            let shader = &map.shaders[surface
                .source_shader
                .ok_or(WorldMaterialError::Boundary("missing source shader"))?
                as usize];
            let name = canonical_path(
                std::str::from_utf8(shader.name)
                    .map_err(|_| WorldMaterialError::Boundary("invalid shader name"))?,
            );
            (name, ImageId(0), [1.0; 2], shader.surface_flags as u32)
        };
        let legacy_sky = if matches!(geometry_source, RuleSetId::Quake) {
            surface
                .source_texture
                .and_then(|index| layered_skies.get(index as usize))
                .copied()
                .flatten()
        } else if matches!(geometry_source, RuleSetId::Quake2) && sky {
            Some(Sky::Cube {
                outer_box: Some(load_indexed_box(
                    &options.sky_environment,
                    palette.ok_or(WorldMaterialError::Boundary("missing sky palette"))?,
                    &mut images,
                )?),
                inner_box: None,
                clouds: CloudSphere::native(512.0),
                rotation: (options.sky_environment.degrees_per_second != 0.0).then_some(
                    crate::sky::Rotation {
                        axis: qa_core::math::normalized(options.sky_environment.axis),
                        degrees_per_second: options.sky_environment.degrees_per_second,
                    },
                ),
                params: CubeSkyParams {
                    distance: SkyDistance::Fixed(2300.0),
                    far_depth: false,
                    texcoord_range: if options.sky_environment.degrees_per_second != 0.0
                        || profile.image_settings.sky_mip
                    {
                        [1.0 / 256.0, 255.0 / 256.0]
                    } else {
                        [1.0 / 512.0, 511.0 / 512.0]
                    },
                    sampler: Sampler {
                        wrap: Wrap::Clamp,
                        mipmaps: false,
                        ..Sampler::default()
                    },
                    snap_bounds: false,
                    cpu_background: true,
                    cpu_rotation: false,
                },
            })
        } else {
            None
        };
        if warp {
            scale = [1.0 / 64.0; 2];
        }
        let legacy_shader_name = (!matches!(geometry_source, RuleSetId::Quake3))
            .then(|| canonical_path(&format!("textures/{name}")));
        let definition = catalog.find_canonical(legacy_shader_name.as_deref().unwrap_or(&name));
        let material = if let Some(definition) = definition.filter(|d| d.valid) {
            compile_definition(definition, &mut images)?
        } else {
            if definition.is_some() {
                diagnostics.push(format!("native shader fallback: {name}"));
            }
            if matches!(geometry_source, RuleSetId::Quake3) {
                let image = images.raster(&name, ImageUse::default(), images.raster_target())?;
                default_material(&name, image, surface.light_source, &mut images)?
            } else {
                legacy_material(
                    &name,
                    image,
                    flags,
                    geometry_source,
                    kind,
                    surface.light_source,
                    legacy_sky,
                    &mut images,
                )?
            }
        };
        bindings.push(SurfaceMaterial {
            material,
            lightmap: lightmaps[surface.source_id as usize].unwrap_or(ImageId(0)),
            // Legacy grids retain face bounds after their UVs were rewritten
            // into the atlas. Authored Page UVs sample the full original page.
            lightmap_region: if surface.light_source == LightSource::Samples {
                regions[surface.source_id as usize]
            } else {
                None
            },
            lightmap_conversion: (surface.light_source == LightSource::Samples
                && regions[surface.source_id as usize].is_some())
            .then_some(lightmap_conversion),
            texture_scale: scale,
        });
    }
    for conflict in images.conflicts() {
        diagnostics.push(format!(
            "native first-image flags retained: {} ({:?} requested {:?})",
            images
                .assets
                .name(conflict.name)
                .unwrap_or("<unknown image>"),
            conflict.first,
            conflict.requested
        ));
    }
    Ok(bindings)
}

struct LightmapLayout {
    atlas: Atlas,
    regions: Vec<Option<AtlasRegion>>,
}

fn prepare_lightmap_layout(geometry: &WorldGeometry) -> Result<LightmapLayout, WorldMaterialError> {
    let decoupled = geometry
        .surfaces
        .iter()
        .any(|surface| surface.lightmap_projection.is_some());
    let mut page_size = if decoupled {
        1024
    } else {
        crate::lightmap::PAGE_SIZE
    };
    if decoupled {
        for surface in &geometry.surfaces {
            if surface.no_draw || surface.light_source != LightSource::Samples {
                continue;
            }
            let required = surface.lightmap_grid[0]
                .checked_add(1)
                .map(|width| width.max(surface.lightmap_grid[1]))
                .ok_or(LightmapError::Dimensions)?;
            page_size = page_size.max(
                required
                    .checked_next_power_of_two()
                    .ok_or(LightmapError::Dimensions)?,
            );
        }
    }
    let mut atlas = AtlasBuilder::load(page_size, 4096)?;
    let mut regions = vec![None; geometry.surfaces.len()];
    let mut page_regions: Vec<(u32, AtlasRegion)> = Vec::new();
    for surface in &geometry.surfaces {
        if surface.no_draw {
            continue;
        }
        let region = match surface.light_source {
            LightSource::Page(page) => {
                if let Some((_, region)) = page_regions.iter().find(|(id, _)| *id == page) {
                    *region
                } else {
                    let blank = vec![0; crate::lightmap::rgb_bytes(page_size, page_size)?];
                    let region = atlas.insert_page(&blank)?;
                    page_regions.push((page, region));
                    region
                }
            }
            LightSource::ExternalPage(_) => {
                return Err(WorldMaterialError::Boundary(
                    "external lightmap resource pending",
                ));
            }
            LightSource::Samples => {
                let [width, height] = surface.lightmap_grid;
                let blank = vec![0; crate::lightmap::rgb_bytes(width, height)?];
                atlas.insert(width, height, &blank)?
            }
            _ => continue,
        };
        regions[surface.source_id as usize] = Some(region);
    }
    Ok(LightmapLayout {
        atlas: atlas.finish(),
        regions,
    })
}

fn prepare_lightmap_pages(
    geometry: &WorldGeometry,
    layout: &LightmapLayout,
    assets: &mut Assets,
    options: &WorldLoadOptions,
    styles: &[LightStyle; 256],
    conversion: LightmapConversion,
) -> Result<Vec<Option<ImageId>>, WorldMaterialError> {
    let mut pages: Vec<_> = layout
        .atlas
        .pages()
        .iter()
        .map(|page| page.rgb.to_vec())
        .collect();
    let mut authored = Vec::new();
    for surface in &geometry.surfaces {
        let Some(region) = layout.regions[surface.source_id as usize] else {
            continue;
        };
        if matches!(surface.light_source, LightSource::Page(_)) {
            if authored.contains(&region.page) {
                continue;
            }
            authored.push(region.page);
        }
        let samples: Vec<_> = geometry.light_samples[surface.light_samples.range()]
            .iter()
            .flatten()
            .copied()
            .collect();
        let converted = if matches!(surface.light_source, LightSource::Page(_)) {
            shift_quake3_rgb(
                &samples,
                options.map_overbright,
                options.renderer_overbright,
            )?
        } else {
            let (active, count) = conversion.referenced_styles(surface.styles, styles);
            let mut rgb = vec![0; crate::lightmap::rgb_bytes(region.width, region.height)?];
            conversion.build_rgb_into(
                region.width,
                region.height,
                &samples,
                &active[..count],
                &mut rgb,
            )?;
            rgb
        };
        write_lightmap_region(&mut pages[region.page as usize], region, &converted)?;
    }
    let mut ids = Vec::with_capacity(pages.len());
    for rgb in pages {
        let rgba: Vec<_> = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        let id = assets
            .register_image(layout.atlas.page_size(), layout.atlas.page_size(), &rgba)
            .map_err(WorldMaterialError::Boundary)?;
        // Lightstyle/modulate/overbright bytes bypass ordinary image upload corrections.
        assets
            .prepare_image(id, upload::UploadParams::default())
            .map_err(WorldMaterialError::Boundary)?;
        ids.push(id);
    }
    Ok(layout
        .regions
        .iter()
        .map(|region| region.map(|r| ids[r.page as usize]))
        .collect())
}

fn write_lightmap_region(
    page: &mut [u8],
    region: AtlasRegion,
    rgb: &[u8],
) -> Result<(), LightmapError> {
    let expected = crate::lightmap::rgb_bytes(region.width, region.height)?;
    if rgb.len() != expected {
        return Err(LightmapError::SampleLength {
            expected,
            actual: rgb.len(),
        });
    }
    for row in 0..region.height as usize {
        let from = row * region.width as usize * 3;
        let to = ((region.y as usize + row) * region.page_size as usize + region.x as usize) * 3;
        page[to..to + region.width as usize * 3]
            .copy_from_slice(&rgb[from..from + region.width as usize * 3]);
    }
    Ok(())
}

fn default_material(
    name: &str,
    image: ResolvedImage,
    light: LightSource,
    images: &mut Images<'_>,
) -> Result<MaterialId, WorldMaterialError> {
    let base = Stage {
        texture: StageTexture::Image(image.id),
        sampler: image.sampler,
        ..Stage::default()
    };
    let stages = match light {
        LightSource::Page(_) | LightSource::ExternalPage(_) | LightSource::Samples => vec![
            Stage {
                texture: StageTexture::Lightmap,
                texgen: TexCoordGen::Lightmap,
                sampler: Sampler {
                    wrap: Wrap::Clamp,
                    mipmaps: false,
                    ..Sampler::default()
                },
                ..Stage::default()
            },
            Stage {
                blend: Some(StageBlend {
                    source: BlendFactor::DestinationColor,
                    destination: BlendFactor::Zero,
                }),
                depth_write: false,
                ..base
            },
        ],
        LightSource::Vertex => vec![Stage {
            rgb_gen: RgbGen::ExactVertex,
            alpha_gen: AlphaGen::Skip,
            ..base
        }],
        LightSource::White => vec![
            Stage {
                rgb_gen: RgbGen::IdentityLighting,
                ..Stage::default()
            },
            Stage {
                blend: Some(StageBlend {
                    source: BlendFactor::DestinationColor,
                    destination: BlendFactor::Zero,
                }),
                depth_write: false,
                ..base
            },
        ],
        _ => vec![Stage {
            rgb_gen: RgbGen::LightingDiffuse,
            ..base
        }],
    };
    images
        .assets
        .register_material(name, &stages, MaterialSettings::default())
        .map_err(WorldMaterialError::Boundary)
}

pub fn compile_definition(
    definition: &ShaderDef,
    images: &mut Images<'_>,
) -> Result<MaterialId, WorldMaterialError> {
    if definition.has_unsupported_runtime() {
        return Err(WorldMaterialError::Boundary(
            "unsupported runtime shader declaration",
        ));
    }
    // Topology-changing native deforms need the shared geometry builder. Fail
    // at the resource boundary instead of registering a draw that disappears.
    if definition.deforms.iter().any(|deform| {
        matches!(
            deform,
            Deform::AutoSprite | Deform::AutoSprite2 | Deform::ProjectionShadow | Deform::Text(_)
        )
    }) {
        return Err(WorldMaterialError::Boundary(
            "shader topology deformation pending",
        ));
    }
    let mut settings = MaterialSettings {
        cull: definition.cull,
        sort: definition.sort,
        polygon_offset: definition.polygon_offset,
        clamp_time: definition.clamp_time,
        fog: definition.fog,
        surface_flags: definition.surface_flags,
        content_flags: definition.content_flags,
        portal: definition.portal,
        ..MaterialSettings::default()
    };
    for (slot, &deform) in settings.deforms.iter_mut().zip(definition.deforms.iter()) {
        *slot = Some(deform);
    }
    if let Some(sky) = &definition.sky {
        settings.sky = Some(Sky::Cube {
            outer_box: load_box(sky.outer_box.as_deref(), Wrap::Clamp, images)?,
            inner_box: load_box(sky.inner_box.as_deref(), Wrap::Repeat, images)?,
            clouds: CloudSphere::native(sky.cloud_height),
            rotation: None,
            params: CubeSkyParams::default(),
        });
    }
    let mut stages = Vec::with_capacity(definition.stages.len());
    for parsed in &definition.stages {
        let mut sampler = Sampler {
            filter: Filter::Linear,
            mipmaps: !definition.no_mipmaps,
            ..Sampler::default()
        };
        let texture = match parsed
            .map
            .as_ref()
            .ok_or(WorldMaterialError::Boundary("shader stage has no map"))?
        {
            TextureMap::Image { name, clamp } => {
                sampler.wrap = if *clamp { Wrap::Clamp } else { Wrap::Repeat };
                let image = images.raster(
                    name,
                    ImageUse {
                        sampler,
                        allow_picmip: !definition.no_picmip,
                        ..ImageUse::default()
                    },
                    images.raster_target(),
                )?;
                sampler = image.sampler;
                StageTexture::Image(image.id)
            }
            TextureMap::White => {
                sampler.mipmaps = false;
                StageTexture::Image(ImageId(0))
            }
            TextureMap::Lightmap => {
                sampler.wrap = Wrap::Clamp;
                sampler.mipmaps = false;
                StageTexture::Lightmap
            }
            TextureMap::Animation {
                frequency,
                images: names,
            } => {
                let mut frames = [ImageId(0); 8];
                for (slot, name) in frames.iter_mut().zip(names.iter()) {
                    let image = images.raster(
                        name,
                        ImageUse {
                            sampler,
                            allow_picmip: !definition.no_picmip,
                            ..ImageUse::default()
                        },
                        images.raster_target(),
                    )?;
                    *slot = image.id;
                }
                StageTexture::Animation {
                    images: frames,
                    count: names.len() as u8,
                    frequency: *frequency,
                }
            }
            TextureMap::Video(_) => {
                return Err(WorldMaterialError::Boundary(
                    "video shader image decoder pending",
                ));
            }
        };
        let mut tcmods = [None; 4];
        for (slot, &modification) in tcmods.iter_mut().zip(parsed.tc_mods.iter()) {
            *slot = Some(TcMod::Script(modification));
        }
        let texgen = match (&definition.sky, parsed.tc_gen) {
            (Some(sky), TexCoordGen::Texture) => TexCoordGen::CloudSky {
                radius: 4096.0,
                height: sky.cloud_height,
            },
            _ => parsed.tc_gen,
        };
        stages.push(Stage {
            texture,
            sampler,
            texture_intensity: TextureIntensity::Preserve,
            blend: parsed.blend,
            rgb_gen: parsed.rgb_gen,
            alpha_gen: parsed.alpha_gen,
            alpha_test: parsed.alpha_func,
            texgen,
            tcmods,
            depth_func: parsed.depth_func,
            depth_write: parsed.depth_write,
            detail: parsed.detail,
        });
    }
    images
        .assets
        .register_material(&definition.name, &stages, settings)
        .map_err(WorldMaterialError::Boundary)
}
fn load_box(
    name: Option<&str>,
    wrap: Wrap,
    images: &mut Images<'_>,
) -> Result<Option<[ImageId; 6]>, WorldMaterialError> {
    load_box_with_suffix(name, "_", wrap, images)
}
fn load_indexed_box(
    environment: &SkyEnvironment,
    palette: crate::PaletteId,
    images: &mut Images<'_>,
) -> Result<[ImageId; 6], WorldMaterialError> {
    let mut result = [ImageId(0); 6];
    for (slot, suffix) in result
        .iter_mut()
        .zip(crate::sky::CubeFace::ALL.map(crate::sky::CubeFace::suffix))
    {
        let base = format!(
            "env/{}{suffix}",
            environment
                .name()
                .map_err(|_| WorldMaterialError::Boundary("invalid sky environment name"))?
        );
        *slot = images.indexed_pcx(
            &format!("{base}.pcx"),
            palette,
            None,
            Some(&format!("{base}.tga")),
            ImageUse {
                role: ImageRole::CubeSky,
                sampler: Sampler {
                    wrap: Wrap::Clamp,
                    mipmaps: false,
                    ..Sampler::default()
                },
                allow_picmip: false,
            },
        )?;
    }
    Ok(result)
}
fn load_box_with_suffix(
    name: Option<&str>,
    separator: &str,
    wrap: Wrap,
    images: &mut Images<'_>,
) -> Result<Option<[ImageId; 6]>, WorldMaterialError> {
    let Some(name) = name else { return Ok(None) };
    let mut result = [ImageId(0); 6];
    for (slot, suffix) in result
        .iter_mut()
        .zip(crate::sky::CubeFace::ALL.map(crate::sky::CubeFace::suffix))
    {
        *slot = images
            .raster(
                &format!("{name}{separator}{suffix}.tga"),
                // Native ParseSkyParms forces mips/picmip for box faces even if
                // the cloud material has nomipmaps/nopicmip.
                ImageUse {
                    role: ImageRole::CubeSky,
                    sampler: Sampler {
                        wrap,
                        ..Sampler::default()
                    },
                    allow_picmip: true,
                },
                images.raster_target(),
            )?
            .id;
    }
    Ok(Some(result))
}

fn legacy_sky_name(name: &[u8]) -> bool {
    name.starts_with(b"sky")
}

fn legacy_kind(rules: RuleSetId, name: &[u8], flags: u32) -> (bool, bool) {
    match rules {
        RuleSetId::Quake | RuleSetId::QuakeWorld => (name.starts_with(b"*"), legacy_sky_name(name)),
        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => (flags & 8 != 0, flags & 4 != 0),
        RuleSetId::Quake3 => (false, false),
    }
}

fn legacy_material(
    name: &str,
    image: ImageId,
    flags: u32,
    rules: RuleSetId,
    kind: (bool, bool),
    light: LightSource,
    sky_material: Option<Sky>,
    images: &mut Images<'_>,
) -> Result<MaterialId, WorldMaterialError> {
    let (warp, sky) = kind;
    if sky {
        let sky =
            sky_material.ok_or(WorldMaterialError::Boundary("missing native sky resources"))?;
        let stages = if let Sky::Layered { images, sphere } = sky {
            sphere
                .scroll_speeds
                .iter()
                .enumerate()
                .map(|(i, &speed)| Stage {
                    texture: StageTexture::Image(images[i]),
                    texgen: TexCoordGen::LayeredSky {
                        flatten_z: sphere.flatten_z,
                        projected_scale: sphere.projected_scale,
                        texture_size: sphere.texture_size,
                        scroll_speed: speed,
                    },
                    blend: if i == 0 {
                        None
                    } else {
                        Some(StageBlend::ALPHA)
                    },
                    depth_write: i == 0,
                    sampler: Sampler {
                        mipmaps: false,
                        ..Sampler::default()
                    },
                    ..Stage::default()
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        return images
            .assets
            .register_material(
                name,
                &stages,
                MaterialSettings {
                    cull: Cull::None,
                    sort: 2.0,
                    sky: Some(sky),
                    ..MaterialSettings::default()
                },
            )
            .map_err(WorldMaterialError::Boundary);
    }
    let mut base = Stage {
        texture: StageTexture::Image(image),
        ..Stage::default()
    };
    if matches!(rules, RuleSetId::Quake) && name.starts_with('{') {
        base.alpha_test = AlphaFunc::GreaterZero;
    }
    if warp {
        base.tcmods[0] = Some(TcMod::Warp(UvWarp {
            texel_scale: [64.0; 2],
            amplitude: [8.0 / 64.0; 2],
            frequency: 0.125,
            time_scale: 1.0,
        }));
    }
    if matches!(rules, RuleSetId::Quake2) && flags & 64 != 0 {
        base.tcmods[usize::from(warp)] = Some(TcMod::Flow(Flow {
            speed: if warp { 0.5 } else { 1.0 / 40.0 },
            amplitude: if warp { [-1.0, 0.0] } else { [-64.0, 0.0] },
            cycle_start: if warp { [0.0; 2] } else { [-64.0, 0.0] },
        }));
    }
    let alpha = if matches!(rules, RuleSetId::Quake2) && flags & 16 != 0 {
        Some(0.33)
    } else if matches!(rules, RuleSetId::Quake2) && flags & 32 != 0 {
        Some(0.66)
    } else {
        None
    };
    if matches!(rules, RuleSetId::Quake2) && (warp || alpha.is_some()) {
        base.texture_intensity = TextureIntensity::NeutralizeUpload;
    }
    let mut settings = MaterialSettings::default();
    if let Some(alpha) = alpha {
        base.alpha_gen = AlphaGen::Const(alpha);
        base.blend = Some(StageBlend::ALPHA);
        base.depth_write = false;
        settings.sort = 9.0;
    } else if warp || matches!(rules, RuleSetId::Quake2) && flags & 64 != 0 {
        settings.software_tiled = Some(SoftwareTiled {
            turbulent: warp,
            flow_speed: (matches!(rules, RuleSetId::Quake2) && flags & 64 != 0)
                .then_some(if warp { 0.25 } else { 0.77 }),
            time_f32: matches!(rules, RuleSetId::Quake2),
        });
    }
    let mut stages = vec![base];
    if !warp && alpha.is_none() && light == LightSource::Samples {
        stages.push(Stage {
            texture: StageTexture::Lightmap,
            texgen: TexCoordGen::Lightmap,
            sampler: Sampler {
                wrap: Wrap::Clamp,
                mipmaps: false,
                ..Sampler::default()
            },
            blend: Some(StageBlend {
                source: BlendFactor::DestinationColor,
                destination: BlendFactor::Zero,
            }),
            depth_func: DepthFunc::Equal,
            depth_write: false,
            ..Stage::default()
        });
    }
    images
        .assets
        .register_material(name, &stages, settings)
        .map_err(WorldMaterialError::Boundary)
}

fn load_layered_sky(
    mip: &qa_formats::image::MipTexture<'_>,
    palette: crate::PaletteId,
    images: &mut Images<'_>,
) -> Result<Sky, WorldMaterialError> {
    if mip.width != 256 || mip.height != 128 {
        return Err(WorldMaterialError::Boundary(
            "native layered sky dimensions",
        ));
    }
    let colors = images
        .assets
        .palette(palette)
        .ok_or(WorldMaterialError::Boundary("missing sky palette"))?;
    let palette_colors = colors.rgba_table();
    let layers =
        split_layered_sky(mip.levels[0], &palette_colors).map_err(WorldMaterialError::Boundary)?;
    let corrected = split_layered_sky(mip.levels[0], &images.corrected_palette(palette)?)
        .map_err(WorldMaterialError::Boundary)?;
    let opaque = IndexedTexture::load_base(128, 128, &layers.opaque_indices, None)
        .map_err(WorldMaterialError::Boundary)?;
    let masked = IndexedTexture::load_base(128, 128, &layers.masked_indices, Some(0))
        .map_err(WorldMaterialError::Boundary)?;
    let ids = [
        images.register_indexed_rgba(128, 128, &layers.opaque_rgba, opaque, palette)?,
        images.register_indexed_rgba(128, 128, &layers.masked_rgba, masked, palette)?,
    ];
    let params = images.upload_params(ImageUse {
        role: ImageRole::LayeredSky,
        sampler: Sampler {
            mipmaps: false,
            ..Sampler::default()
        },
        allow_picmip: false,
    })?;
    images
        .assets
        .prepare_image_with_rgba(ids[0], &corrected.opaque_rgba, params)
        .map_err(WorldMaterialError::Boundary)?;
    images
        .assets
        .prepare_image_with_rgba(ids[1], &corrected.masked_rgba, params)
        .map_err(WorldMaterialError::Boundary)?;
    Ok(Sky::Layered {
        images: ids,
        sphere: LayeredSphere::NATIVE,
    })
}

#[cfg(test)]
#[path = "../../tests/legacy_kind/oracle.rs"]
mod legacy_kind_tests;
