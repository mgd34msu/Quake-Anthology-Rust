//! File-boundary conversion into the one runtime world/material table.
use super::{
    ShaderCompilePolicy,
    finish::{BundleImage, ShaderSlot, finish_material},
    load_catalog,
    resources::{
        ImageRole, ImageSettings, ImageUse, Images, ImagesState, IndexedTarget, ResolvedImage,
        ResourceError,
    },
};
use crate::{
    Assets, BlendPhase, CpuPresentation, LightStyle, PaletteOperation, PaletteTransform,
    PerspectiveStep, Refdef,
    assets::{
        CubeSkyParams, DepthFunc, Flow, ImageId, MaterialId, MaterialSettings, NativeDefaultKind,
        Sampler, ShaderBinding, ShaderBindingId, ShaderRegistration, Sky, SkyDistance,
        SoftwareTiled, Stage, StageTexture, TcMod, TextureIntensity, UvWarp, Wrap, upload,
    },
    lightmap::{
        Atlas, AtlasBuilder, AtlasRegion, LightmapConversion, LightmapError, Q1GlScale,
        shift_quake3_rgb,
    },
    shader::{
        AlphaFunc, AlphaGen, BlendFactor, Cull, Deform, RgbGen, ShaderCatalog, ShaderDef,
        StageBlend, TexCoordGen, canonical_path,
    },
    sky::{CloudSphere, LayeredSphere, split_layered_sky},
    surface_cache::{IndexedLighting, IndexedTexture},
    world::{
        PreparedShader, SurfaceMaterial, WorldId, WorldLighting, WorldPresentationId,
        WorldPresentationInput, WorldRenderSettings,
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
    pub lighting: WorldLighting,
    pub sky_environment: SkyEnvironment,
}
impl Default for WorldLoadOptions {
    fn default() -> Self {
        Self {
            geometry: GeometryOptions::default(),
            lighting: WorldLighting::default(),
            sky_environment: SkyEnvironment::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationDefaults {
    pub software_waterwarp: bool,
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
            software_waterwarp: view.software_waterwarp,
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
        view.software_waterwarp = self.software_waterwarp;
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
pub struct WorldPresentationRequest {
    pub profile: WorldPresentationProfile,
    pub native_default: Option<ShaderBindingId>,
}
impl From<WorldPresentationProfile> for WorldPresentationRequest {
    fn from(profile: WorldPresentationProfile) -> Self {
        Self {
            profile,
            native_default: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadedWorldPresentation {
    pub id: WorldPresentationId,
    pub profile: WorldPresentationProfile,
    pub defaults: PresentationDefaults,
    pub lighting: WorldLighting,
    pub compile_policy: ShaderCompilePolicy,
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
    Shader(&'static str),
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

struct MaterialMip {
    name: qa_core::primitives::NameId,
    width: u32,
    height: u32,
    levels: [Box<[u8]>; 4],
}
impl MaterialMip {
    fn borrowed<'a>(&'a self, name: &'a [u8]) -> qa_formats::image::MipTexture<'a> {
        qa_formats::image::MipTexture {
            name,
            width: self.width,
            height: self.height,
            shift: 0,
            levels: self.levels.each_ref().map(|level| level.as_ref()),
            animation: &[],
            flags: 0,
            contents: 0,
            value: 0,
        }
    }
}
struct MaterialShader {
    name: qa_core::primitives::NameId,
    flags: u32,
}
struct MaterialSurface {
    source_id: u32,
    source_texture: Option<u32>,
    source_texture_info: Option<u32>,
    source_shader: Option<u32>,
    source_flags: u32,
    light_source: LightSource,
    no_draw: bool,
}
/// Authored cold inputs retained once after the BSP boundary. Geometry can be
/// rebuilt without rereading the BSP; visibility and collision stay separately owned.
pub(crate) struct WorldMaterialSource {
    rules: RuleSetId,
    palette: Option<crate::PaletteId>,
    options: WorldLoadOptions,
    vertex_colors: Option<Box<[[u8; 4]]>>,
    textures: Vec<Option<MaterialMip>>,
    texture_info: Vec<qa_core::primitives::NameId>,
    shaders: Vec<MaterialShader>,
    fogs: Vec<qa_core::primitives::NameId>,
    surfaces: Vec<MaterialSurface>,
    legacy_kinds: Vec<(bool, bool)>,
    layout: LightmapLayout,
    catalog: ShaderCatalog,
}
impl WorldMaterialSource {
    pub(crate) fn geometry_options(&self) -> GeometryOptions {
        self.options.geometry
    }
    pub(crate) fn load_options(&self) -> &WorldLoadOptions {
        &self.options
    }
    pub(crate) fn restore_options(&mut self, options: WorldLoadOptions) {
        self.options = options;
    }
    pub(crate) fn render_settings(&self) -> WorldRenderSettings {
        WorldRenderSettings {
            patch_subdivisions: self.options.geometry.patch_subdivisions,
            lighting: self.options.lighting,
        }
    }
    pub(crate) fn set_render_settings(&mut self, settings: WorldRenderSettings) {
        self.options.geometry.patch_subdivisions = settings.patch_subdivisions;
        self.options.lighting = settings.lighting;
        if self.rules == RuleSetId::Quake3 {
            self.options.geometry.vertex_color_shift = settings.lighting.shift();
        }
    }
    pub(crate) fn vertex_colors(&self) -> Option<&[[u8; 4]]> {
        self.vertex_colors.as_deref()
    }

    fn load(
        vfs: &Vfs,
        map: &Map<'_>,
        assets: &mut Assets,
        palette: Option<crate::PaletteId>,
        options: WorldLoadOptions,
        geometry: &WorldGeometry,
        legacy_kinds: Vec<(bool, bool)>,
        layout: LightmapLayout,
    ) -> Result<Self, WorldMaterialError> {
        let mut name = |raw: &[u8]| {
            assets
                .intern_resource_name(raw)
                .map_err(WorldMaterialError::Boundary)
        };
        let textures = map
            .textures
            .iter()
            .map(|mip| {
                mip.as_ref()
                    .map(|mip| {
                        Ok(MaterialMip {
                            name: name(mip.name)?,
                            width: mip.width,
                            height: mip.height,
                            levels: mip.levels.map(Into::into),
                        })
                    })
                    .transpose()
            })
            .collect::<Result<_, WorldMaterialError>>()?;
        let texture_info = map
            .texture_info
            .iter()
            .map(|info| name(info.name))
            .collect::<Result<_, _>>()?;
        let shaders = map
            .shaders
            .iter()
            .map(|shader| {
                Ok(MaterialShader {
                    name: name(shader.name)?,
                    flags: shader.surface_flags as u32,
                })
            })
            .collect::<Result<_, WorldMaterialError>>()?;
        let fogs = map
            .fogs
            .iter()
            .map(|fog| name(fog.name))
            .collect::<Result<_, _>>()?;
        let surfaces = geometry
            .surfaces
            .iter()
            .map(|surface| MaterialSurface {
                source_id: surface.source_id,
                source_texture: surface.source_texture,
                source_texture_info: surface.source_texture_info,
                source_shader: surface.source_shader,
                source_flags: surface.source_flags,
                light_source: surface.light_source,
                no_draw: surface.no_draw,
            })
            .collect();
        Ok(Self {
            rules: map.bsp.format.rule_set(),
            palette,
            options,
            vertex_colors: (map.bsp.format.rule_set() == RuleSetId::Quake3)
                .then(|| map.vertices.iter().map(|vertex| vertex.color).collect()),
            textures,
            texture_info,
            shaders,
            fogs,
            surfaces,
            legacy_kinds,
            layout,
            catalog: load_catalog(vfs).map_err(WorldMaterialError::Catalog)?,
        })
    }
}

struct CompiledPresentation {
    surfaces: Vec<SurfaceMaterial>,
    registrations: Vec<ShaderRegistration<PreparedShader>>,
    native_page0: Option<ImageId>,
    native_default: Option<ShaderBindingId>,
}
impl CompiledPresentation {
    fn input(&self) -> WorldPresentationInput<'_> {
        WorldPresentationInput {
            surfaces: &self.surfaces,
            registrations: &self.registrations,
            native_page0: self.native_page0,
            native_default: self.native_default,
        }
    }
}

struct PreparedPresentation {
    defaults: PresentationDefaults,
    conversion: LightmapConversion,
    pages: Vec<Vec<u8>>,
}
fn prepare_presentation(
    geometry: &WorldGeometry,
    source: &WorldMaterialSource,
    profile: WorldPresentationProfile,
) -> Result<PreparedPresentation, WorldMaterialError> {
    let defaults = presentation_defaults(profile, &source.options);
    let conversion = if matches!(source.rules, RuleSetId::Quake) {
        LightmapConversion::Scalar(profile.q1_gl_scale)
    } else {
        LightmapConversion::Rgb {
            modulate: profile.q2_modulate,
        }
    };
    let pages = prepare_lightmap_pages(
        geometry,
        &source.layout,
        &source.options,
        &defaults.lightstyles,
        conversion,
    )?;
    Ok(PreparedPresentation {
        defaults,
        conversion,
        pages,
    })
}
fn compile_profile(
    vfs: &Vfs,
    assets: &mut Assets,
    source: &WorldMaterialSource,
    request: WorldPresentationRequest,
    prepared: PreparedPresentation,
    policy: ShaderCompilePolicy,
    diagnostics: &mut Vec<String>,
) -> Result<(CompiledPresentation, PresentationDefaults), WorldMaterialError> {
    let lightmaps = register_lightmap_pages(assets, &source.layout, prepared.pages)?;
    let bindings = compile_presentation(
        vfs,
        source,
        assets,
        request,
        policy,
        &source.layout.regions,
        &lightmaps.surfaces,
        prepared.conversion,
        diagnostics,
    )?;
    Ok((
        CompiledPresentation {
            surfaces: bindings.0,
            registrations: bindings.1,
            native_page0: lightmaps.native_page0,
            native_default: bindings.2,
        },
        prepared.defaults,
    ))
}

fn validate_profiles(
    assets: &Assets,
    geometry_source: RuleSetId,
    source_palette: Option<crate::PaletteId>,
    profiles: &[WorldPresentationRequest],
) -> Result<(), WorldMaterialError> {
    if profiles.is_empty() {
        return Err(WorldMaterialError::Boundary("missing world presentation"));
    }
    if (!matches!(geometry_source, RuleSetId::Quake3) && source_palette.is_none())
        || source_palette.is_some_and(|id| assets.palette(id).is_none())
        || profiles.iter().any(|request| {
            let profile = request.profile;
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
    Ok(())
}

/// Presentation is caller-selected; BSP decoding, flags and coordinates retain
/// their source semantics independently of the selected view/image policy.
pub fn load_world(
    vfs: &Vfs,
    map: &Map<'_>,
    assets: &mut Assets,
    source_palette: Option<crate::PaletteId>,
    profiles: &[WorldPresentationRequest],
    mut options: WorldLoadOptions,
    policy: ShaderCompilePolicy,
) -> Result<LoadedWorld, WorldMaterialError> {
    let geometry_source = map.bsp.format.rule_set();
    validate_profiles(assets, geometry_source, source_palette, profiles)?;
    // Nonlinear Q3 color normalization precedes patch interpolation once. These
    // geometry settings are common to every admitted presentation profile.
    if matches!(geometry_source, RuleSetId::Quake3) {
        options.geometry.vertex_color_shift = options.lighting.shift();
    }
    let mut geometry = load_geometry(map, options.geometry)?;
    let visibility =
        load_visibility(map).map_err(|_| WorldMaterialError::Boundary("world visibility"))?;
    let layout = prepare_lightmap_layout(&geometry, geometry_source == RuleSetId::Quake3)?;
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
    let source = WorldMaterialSource::load(
        vfs,
        map,
        assets,
        source_palette,
        options,
        &geometry,
        legacy_kinds,
        layout,
    )?;
    let mut diagnostics = Vec::new();
    let mut distinct = Vec::<WorldPresentationRequest>::new();
    let mut request_indices = Vec::with_capacity(profiles.len());
    for &request in profiles {
        let index = if let Some(index) = distinct
            .iter()
            .position(|existing| existing.profile == request.profile)
        {
            if distinct[index].native_default.is_some()
                && request.native_default.is_some()
                && distinct[index].native_default != request.native_default
            {
                return Err(WorldMaterialError::Boundary("conflicting native defaults"));
            }
            distinct[index].native_default =
                distinct[index].native_default.or(request.native_default);
            index
        } else {
            let index = distinct.len();
            distinct.push(request);
            index
        };
        request_indices.push(index);
    }
    let mut variants = Vec::with_capacity(distinct.len());
    let mut defaults = Vec::with_capacity(distinct.len());
    for &request in &distinct {
        let prepared = prepare_presentation(&geometry, &source, request.profile)?;
        let (bindings, presentation) = compile_profile(
            vfs,
            assets,
            &source,
            request,
            prepared,
            policy,
            &mut diagnostics,
        )?;
        variants.push(bindings);
        defaults.push(presentation);
    }
    let variant_slices: Vec<_> = variants.iter().map(CompiledPresentation::input).collect();
    let world = assets
        .register_world(geometry, visibility, &variant_slices)
        .map_err(WorldMaterialError::Boundary)?;
    if !matches!(geometry_source, RuleSetId::Quake3) {
        let lighting = crate::lighting::PointLighting::load(
            matches!(geometry_source, RuleSetId::Quake),
            source.legacy_kinds.iter().map(|&(warp, sky)| !warp && !sky),
        );
        assets
            .set_point_lighting(world, lighting)
            .map_err(WorldMaterialError::Boundary)?;
    }
    match crate::lighting::LightGrid::load_map(map, source.options.lighting.shift()) {
        Ok(grid) => assets
            .set_light_grid(world, grid)
            .map_err(WorldMaterialError::Boundary)?,
        Err(error) => diagnostics.push(error.to_owned()),
    }
    let lighting = source.options.lighting;
    let ids = assets
        .world(world)
        .ok_or(WorldMaterialError::Boundary("missing registered world"))?
        .presentations()
        .to_vec();
    for ((&id, request), &defaults) in ids.iter().zip(&distinct).zip(&defaults) {
        assets
            .register_world_presentation_metadata(
                world,
                &[LoadedWorldPresentation {
                    id,
                    profile: request.profile,
                    defaults,
                    lighting,
                    compile_policy: policy,
                }],
            )
            .map_err(WorldMaterialError::Boundary)?;
    }
    assets
        .retain_world_material_source(world, source)
        .map_err(WorldMaterialError::Boundary)?;
    let presentations = request_indices
        .into_iter()
        .map(|index| LoadedWorldPresentation {
            id: ids[index],
            profile: distinct[index].profile,
            defaults: defaults[index],
            lighting,
            compile_policy: policy,
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Ok(LoadedWorld {
        world,
        presentations,
        diagnostics: diagnostics.into_boxed_slice(),
    })
}

/// Append an exact presentation to a previously loaded world. The retained
/// source boundary replaces BSP I/O; geometry/visibility/model IDs stay fixed.
/// The app asset owner wraps this cold append in its admission transaction.
pub fn append_world_presentation(
    vfs: &Vfs,
    assets: &mut Assets,
    world: WorldId,
    request: WorldPresentationRequest,
    policy: ShaderCompilePolicy,
) -> Result<LoadedWorld, WorldMaterialError> {
    let profile = request.profile;
    let row = assets
        .world(world)
        .ok_or(WorldMaterialError::Boundary("missing presentation world"))?;
    let current_lighting = row.render_settings().map(|settings| settings.lighting);
    let existing = row.presentations().iter().find_map(|&id| {
        assets.world_presentation_registration(id).filter(|entry| {
            entry.profile == profile
                && current_lighting.is_none_or(|lighting| entry.lighting == lighting)
                && entry.compile_policy == policy
        })
    });
    if let Some(existing) = existing {
        return Ok(LoadedWorld {
            world,
            presentations: vec![existing].into_boxed_slice(),
            diagnostics: Box::new([]),
        });
    }
    assets.with_world_material_source(world, |assets, source| {
        let lighting = source.render_settings().lighting;
        validate_profiles(assets, source.rules, source.palette, &[request])?;
        let prepared = prepare_presentation(
            assets
                .world(world)
                .ok_or(WorldMaterialError::Boundary("missing presentation world"))?
                .geometry(),
            source,
            profile,
        )?;
        let mut diagnostics = Vec::new();
        let (bindings, defaults) = compile_profile(
            vfs,
            assets,
            source,
            request,
            prepared,
            policy,
            &mut diagnostics,
        )?;
        let id = assets
            .append_world_presentation(world, bindings.input())
            .map_err(WorldMaterialError::Boundary)?;
        assets
            .register_world_presentation_metadata(
                world,
                &[LoadedWorldPresentation {
                    id,
                    profile,
                    defaults,
                    lighting,
                    compile_policy: policy,
                }],
            )
            .map_err(WorldMaterialError::Boundary)?;
        Ok(LoadedWorld {
            world,
            presentations: vec![LoadedWorldPresentation {
                id,
                profile,
                defaults,
                lighting,
                compile_policy: policy,
            }]
            .into_boxed_slice(),
            diagnostics: diagnostics.into_boxed_slice(),
        })
    })
}

fn presentation_defaults(
    profile: WorldPresentationProfile,
    options: &WorldLoadOptions,
) -> PresentationDefaults {
    let presentation_rules = profile.rules;
    let mut presentation = PresentationDefaults {
        software_waterwarp: matches!(presentation_rules, RuleSetId::Quake2),
        ..Default::default()
    };
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
        presentation.identity_light = 1.0 / (1u32 << options.lighting.renderer_overbright()) as f32;
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
    source: &WorldMaterialSource,
    assets: &mut Assets,
    request: WorldPresentationRequest,
    policy: ShaderCompilePolicy,
    regions: &[Option<AtlasRegion>],
    lightmaps: &[Option<ImageId>],
    lightmap_conversion: LightmapConversion,
    diagnostics: &mut Vec<String>,
) -> Result<
    (
        Vec<SurfaceMaterial>,
        Vec<ShaderRegistration<PreparedShader>>,
        Option<ShaderBindingId>,
    ),
    WorldMaterialError,
> {
    let profile = request.profile;
    let geometry_source = source.rules;
    let palette = source.palette;
    let options = &source.options;
    let catalog = &source.catalog;
    // Copy only cold name bytes to end the arena borrow before Images appends.
    let name = |id| {
        assets
            .resource_name(id)
            .map(Vec::from)
            .ok_or(WorldMaterialError::Boundary("missing material source name"))
    };
    let texture_names = source
        .textures
        .iter()
        .map(|mip| mip.as_ref().map(|mip| name(mip.name)).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    let info_names = source
        .texture_info
        .iter()
        .map(|&id| name(id))
        .collect::<Result<Vec<_>, _>>()?;
    let shader_names = source
        .shaders
        .iter()
        .map(|shader| name(shader.name))
        .collect::<Result<Vec<_>, _>>()?;
    let fog_names = source
        .fogs
        .iter()
        .map(|&id| name(id))
        .collect::<Result<Vec<_>, _>>()?;
    let mips = source
        .textures
        .iter()
        .zip(&texture_names)
        .map(|(mip, name)| {
            mip.as_ref()
                .zip(name.as_ref())
                .map(|(mip, name)| mip.borrowed(name))
        })
        .collect::<Vec<_>>();
    // Resolve authored overrides once, before generated resources are admitted.
    // Legacy texture names use the same textures/ namespace as their override lookup.
    let definitions = source
        .surfaces
        .iter()
        .map(|surface| {
            let raw =
                source_surface_name(source, surface, &texture_names, &info_names, &shader_names)
                    .ok()?;
            let raw = std::str::from_utf8(raw).ok()?;
            let name = if geometry_source == RuleSetId::Quake3 {
                canonical_path(raw)
            } else {
                canonical_path(&format!("textures/{raw}"))
            };
            catalog.find_canonical(&name)
        })
        .collect::<Vec<_>>();
    let presentation_rules = profile.rules;
    let mut image_state = ImagesState::new(
        geometry_source,
        presentation_rules,
        profile.image_settings,
        palette.zip(profile.indexed_palette),
    )?;
    image_state.set_raster_target(
        assets,
        profile.indexed_palette.map(|palette| IndexedTarget {
            palette,
            transparent_index: Some(255),
        }),
    )?;
    let native_default = if geometry_source == RuleSetId::Quake3
        || request.native_default.is_some()
        || definitions.iter().any(Option::is_some)
    {
        Some(native_default_shader(
            &mut image_state,
            assets,
            profile.indexed_palette.map(|palette| IndexedTarget {
                palette,
                transparent_index: None,
            }),
            request.native_default,
            policy,
        )?)
    } else {
        None
    };
    let mut images = Images::from_state(vfs, assets, image_state);
    let mut textures = Vec::with_capacity(mips.len());
    let mut layered_skies = Vec::with_capacity(mips.len());
    if matches!(geometry_source, RuleSetId::Quake) {
        for texture in &mips {
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
    let mut bindings = Vec::with_capacity(source.surfaces.len());
    let mut registrations = Vec::with_capacity(source.fogs.len() + source.surfaces.len());
    for raw in &fog_names {
        let registration = optional_named_binding(
            catalog,
            raw,
            NamedShaderLighting {
                source: LightSource::None,
                lightmap: ImageId(0),
            },
            policy,
            &mut images,
            diagnostics,
        )?;
        if let Some(registration) = registration {
            registrations.push(registration.map_shader(|shader| PreparedShader::Binding {
                shader,
                surface: None,
            }));
        }
    }
    for ((surface, &kind), definition) in source
        .surfaces
        .iter()
        .zip(&source.legacy_kinds)
        .zip(definitions)
    {
        let (warp, sky) = kind;
        if surface.no_draw {
            let raw = match source_surface_name(
                source,
                surface,
                &texture_names,
                &info_names,
                &shader_names,
            ) {
                Ok(raw) => raw,
                Err(error) => {
                    diagnostics.push(format!("native unnamed default shader: {error:?}"));
                    bindings.push(SurfaceMaterial::default());
                    continue;
                }
            };
            let registration = optional_named_binding(
                catalog,
                raw,
                NamedShaderLighting {
                    // Native Q3 normalizes a missing page before the hidden
                    // surface draw skip; visible external resources keep their boundary.
                    source: if geometry_source == RuleSetId::Quake3
                        && matches!(surface.light_source, LightSource::ExternalPage(_))
                    {
                        LightSource::Vertex
                    } else {
                        surface.light_source
                    },
                    lightmap: lightmaps[surface.source_id as usize].unwrap_or(ImageId(0)),
                },
                policy,
                &mut images,
                diagnostics,
            )?;
            if let Some(registration) = registration {
                registrations.push(registration.map_shader(|shader| PreparedShader::Binding {
                    shader,
                    surface: None,
                }));
            }
            bindings.push(SurfaceMaterial::default());
            continue;
        }
        let (name, image, mut scale, flags) = if matches!(geometry_source, RuleSetId::Quake) {
            let index = surface.source_texture.ok_or(WorldMaterialError::Boundary(
                "missing embedded texture index",
            ))? as usize;
            let mip = mips
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
            let name = &info_names[surface
                .source_texture_info
                .ok_or(WorldMaterialError::Boundary("missing texture info"))?
                as usize];
            let name = std::str::from_utf8(name)
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
            let shader_index = surface
                .source_shader
                .ok_or(WorldMaterialError::Boundary("missing source shader"))?
                as usize;
            let shader = &source.shaders[shader_index];
            let name = canonical_path(
                std::str::from_utf8(&shader_names[shader_index])
                    .map_err(|_| WorldMaterialError::Boundary("invalid shader name"))?,
            );
            (name, ImageId(0), [1.0; 2], shader.flags)
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
        let raw = source_surface_name(source, surface, &texture_names, &info_names, &shader_names)?;
        let lightmap = lightmaps[surface.source_id as usize].unwrap_or(ImageId(0));
        let mut named_registration = None;
        let converted = geometry_source != RuleSetId::Quake3 && definition.is_none();
        let material = if let Some(definition) = definition {
            let compiled = compile_definition(
                definition,
                &mut images,
                NamedShaderLighting {
                    source: surface.light_source,
                    lightmap,
                },
                policy,
            )?;
            let mut registration = images
                .assets
                .shader_registration_value(
                    raw,
                    ShaderBinding {
                        lightmap,
                        ..ShaderBinding::material(compiled.material, surface.light_source)
                    },
                    compiled.defaulted,
                )
                .map_err(WorldMaterialError::Boundary)?;
            registration.sun = compiled.sun;
            named_registration = Some(registration);
            compiled.material
        } else {
            if matches!(geometry_source, RuleSetId::Quake3) {
                let registration = compile_named_binding(
                    catalog,
                    raw,
                    true,
                    NamedShaderLighting {
                        source: surface.light_source,
                        lightmap,
                    },
                    policy,
                    &mut images,
                )?;
                let material = registration.shader.material;
                named_registration = Some(registration);
                material
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
        let binding = crate::ShaderBinding {
            material,
            light_source: surface.light_source,
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
            styled_surface: None,
        };
        let registration = match named_registration {
            Some(registration) => registration.map_shader(|_| binding),
            None => images
                .assets
                .shader_registration_value(raw, binding, None)
                .map_err(WorldMaterialError::Boundary)?,
        };
        if registration.defaulted.is_some() {
            registrations.push(registration.map_shader(|shader| PreparedShader::Binding {
                shader,
                surface: Some(bindings.len() as u32),
            }));
            let shader = *images
                .assets
                .shader_binding(
                    native_default.ok_or(WorldMaterialError::Boundary("missing native default"))?,
                )
                .ok_or(WorldMaterialError::Boundary("missing native default"))?;
            bindings.push(SurfaceMaterial {
                shader,
                texture_scale: scale,
            });
        } else {
            registrations.push(registration.map_shader(|_| {
                if converted {
                    PreparedShader::ConvertedSurface(surface.source_id)
                } else {
                    PreparedShader::Surface(surface.source_id)
                }
            }));
            bindings.push(SurfaceMaterial {
                shader: binding,
                texture_scale: scale,
            });
        }
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
    Ok((bindings, registrations, native_default))
}

fn source_surface_name<'a>(
    source: &WorldMaterialSource,
    surface: &MaterialSurface,
    textures: &'a [Option<Vec<u8>>],
    infos: &'a [Vec<u8>],
    shaders: &'a [Vec<u8>],
) -> Result<&'a [u8], WorldMaterialError> {
    let raw = match source.rules {
        RuleSetId::Quake => surface
            .source_texture
            .and_then(|i| textures.get(i as usize))
            .and_then(Option::as_ref),
        RuleSetId::Quake2 => surface
            .source_texture_info
            .and_then(|i| infos.get(i as usize)),
        _ => surface.source_shader.and_then(|i| shaders.get(i as usize)),
    };
    raw.map(Vec::as_slice)
        .ok_or(WorldMaterialError::Boundary("missing shader source name"))
}

fn optional_named_binding(
    catalog: &ShaderCatalog,
    raw: &[u8],
    lighting: NamedShaderLighting,
    policy: ShaderCompilePolicy,
    images: &mut Images<'_>,
    diagnostics: &mut Vec<String>,
) -> Result<Option<ShaderRegistration<ShaderBinding>>, WorldMaterialError> {
    match images.try_registration(|images| {
        compile_named_binding(catalog, raw, true, lighting, policy, images)
    }) {
        Ok(registration) => Ok(Some(registration)),
        Err(
            error @ (WorldMaterialError::Shader(_)
            | WorldMaterialError::Resource(ResourceError::Missing(_))),
        ) => {
            // Explicit invalid/unsupported recipes remain optional here, but
            // have no native outcome until parser-prefix admission is supported.
            diagnostics.push(format!(
                "native shader registration omitted: {} ({error:?})",
                String::from_utf8_lossy(raw)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

struct LightmapLayout {
    atlas: Atlas,
    regions: Vec<Option<AtlasRegion>>,
    native_page0: Option<AtlasRegion>,
    native_pages: bool,
}
struct RegisteredLightmaps {
    surfaces: Vec<Option<ImageId>>,
    native_page0: Option<ImageId>,
}

fn prepare_lightmap_layout(
    geometry: &WorldGeometry,
    native_pages: bool,
) -> Result<LightmapLayout, WorldMaterialError> {
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
    // Preserve visible atlas order. Native registers page zero even when no
    // drawable face refers to it, then hidden surfaces can reuse real pages.
    let native_page0 = if native_pages
        && geometry.light_samples.len()
            >= (crate::lightmap::PAGE_SIZE * crate::lightmap::PAGE_SIZE) as usize
    {
        if let Some((_, region)) = page_regions.iter().find(|(page, _)| *page == 0) {
            Some(*region)
        } else {
            let blank = vec![0; crate::lightmap::rgb_bytes(page_size, page_size)?];
            let region = atlas.insert_page(&blank)?;
            page_regions.push((0, region));
            Some(region)
        }
    } else {
        None
    };
    if native_pages {
        for surface in geometry.surfaces.iter().filter(|surface| surface.no_draw) {
            if let LightSource::Page(page) = surface.light_source {
                let region =
                    if let Some((_, region)) = page_regions.iter().find(|(id, _)| *id == page) {
                        *region
                    } else {
                        let blank = vec![0; crate::lightmap::rgb_bytes(page_size, page_size)?];
                        let region = atlas.insert_page(&blank)?;
                        page_regions.push((page, region));
                        region
                    };
                regions[surface.source_id as usize] = Some(region);
            }
        }
    }
    let atlas = atlas.finish();
    let native_page0 = if native_pages {
        native_page0
    } else {
        (!atlas.pages().is_empty()).then_some(AtlasRegion {
            page: 0,
            page_size,
            x: 0,
            y: 0,
            width: page_size,
            height: page_size,
        })
    };
    Ok(LightmapLayout {
        atlas,
        regions,
        native_page0,
        native_pages,
    })
}

fn prepare_lightmap_pages(
    geometry: &WorldGeometry,
    layout: &LightmapLayout,
    options: &WorldLoadOptions,
    styles: &[LightStyle; 256],
    conversion: LightmapConversion,
) -> Result<Vec<Vec<u8>>, WorldMaterialError> {
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
            shift_quake3_rgb(&samples, options.lighting)?
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
    if let Some(region) = layout.native_page0
        && !authored.contains(&region.page)
        && geometry.light_samples.len()
            >= (crate::lightmap::PAGE_SIZE * crate::lightmap::PAGE_SIZE) as usize
        && layout.native_pages
    {
        let samples: Vec<_> = geometry.light_samples
            [..(crate::lightmap::PAGE_SIZE * crate::lightmap::PAGE_SIZE) as usize]
            .iter()
            .flatten()
            .copied()
            .collect();
        let converted = shift_quake3_rgb(&samples, options.lighting)?;
        write_lightmap_region(&mut pages[region.page as usize], region, &converted)?;
    }
    Ok(pages)
}

fn register_lightmap_pages(
    assets: &mut Assets,
    layout: &LightmapLayout,
    pages: Vec<Vec<u8>>,
) -> Result<RegisteredLightmaps, WorldMaterialError> {
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
    Ok(RegisteredLightmaps {
        surfaces: layout
            .regions
            .iter()
            .map(|region| region.map(|r| ids[r.page as usize]))
            .collect(),
        native_page0: layout.native_page0.map(|region| ids[region.page as usize]),
    })
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
    lighting: NamedShaderLighting,
    policy: ShaderCompilePolicy,
    images: &mut Images<'_>,
) -> Result<MaterialId, WorldMaterialError> {
    let light = lighting.source;
    let base = Stage {
        texture: StageTexture::Image(image.id),
        sampler: image.sampler,
        ..Stage::default()
    };
    let stages = match light {
        LightSource::Page(_) | LightSource::ExternalPage(_) | LightSource::Samples => vec![
            Stage {
                texture: StageTexture::Lightmap,
                is_lightmap: true,
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
        LightSource::TwoDimensional => vec![Stage {
            blend: Some(StageBlend::ALPHA),
            rgb_gen: RgbGen::Vertex,
            alpha_gen: AlphaGen::Vertex,
            depth_func: DepthFunc::Always,
            depth_write: false,
            ..base
        }],
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
    finish_material(
        name,
        stages.into_iter().map(ShaderSlot::from_stage).collect(),
        MaterialSettings {
            sort: if light == LightSource::TwoDimensional {
                9.0
            } else {
                3.0
            },
            ..MaterialSettings::default()
        },
        lighting,
        policy,
        images.assets,
    )
}

/// One generated native fallback; its draw body is distinct from named defaults and white helpers.
pub fn native_default_shader(
    state: &mut ImagesState,
    assets: &mut Assets,
    indexed: Option<IndexedTarget>,
    existing: Option<ShaderBindingId>,
    policy: ShaderCompilePolicy,
) -> Result<ShaderBindingId, WorldMaterialError> {
    let image = existing
        .map(|id| {
            let binding = assets
                .shader_binding(id)
                .ok_or(WorldMaterialError::Boundary(
                    "missing native default binding",
                ))?;
            let material =
                assets
                    .material(binding.material)
                    .ok_or(WorldMaterialError::Boundary(
                        "missing native default material",
                    ))?;
            if binding.light_source != LightSource::None
                || assets.name(material.name) != Some("<default>")
                || material.stages.len() != 1
            {
                return Err(WorldMaterialError::Boundary(
                    "invalid native default binding",
                ));
            }
            match material.stages[0].texture {
                StageTexture::Image(image) => Ok(image),
                _ => Err(WorldMaterialError::Boundary("invalid native default image")),
            }
        })
        .transpose()?;
    let image = state.native_default_image(assets, indexed, image)?;
    let material = finish_material(
        "<default>",
        vec![ShaderSlot::from_stage(Stage {
            texture: StageTexture::Image(image.id),
            sampler: image.sampler,
            rgb_gen: RgbGen::IdentityLighting,
            alpha_gen: AlphaGen::Identity,
            ..Stage::default()
        })],
        MaterialSettings::default(),
        NamedShaderLighting {
            source: LightSource::None,
            lightmap: ImageId(0),
        },
        policy,
        assets,
    )?;
    let shader = assets
        .register_shader_binding(ShaderBinding::material(material, LightSource::None))
        .map_err(WorldMaterialError::Boundary)?;
    if existing.is_some_and(|existing| existing != shader) {
        return Err(WorldMaterialError::Boundary(
            "native default policy changed",
        ));
    }
    Ok(shader)
}

/// Resolved lighting resources, independent of the request's scene context.
#[derive(Clone, Copy)]
pub struct NamedShaderLighting {
    pub source: LightSource,
    pub lightmap: ImageId,
}

/// One named shader compiler for native models, skins, UI and world requests.
pub fn compile_named_shader(
    catalog: &ShaderCatalog,
    raw: &[u8],
    mipmaps: bool,
    lighting: NamedShaderLighting,
    policy: ShaderCompilePolicy,
    images: &mut Images<'_>,
) -> Result<ShaderRegistration, WorldMaterialError> {
    let registration = compile_named_binding(catalog, raw, mipmaps, lighting, policy, images)?;
    let shader = images
        .assets
        .register_shader_binding(registration.shader)
        .map_err(WorldMaterialError::Boundary)?;
    Ok(registration.map_shader(|_| shader))
}

fn compile_named_binding(
    catalog: &ShaderCatalog,
    raw: &[u8],
    mipmaps: bool,
    lighting: NamedShaderLighting,
    policy: ShaderCompilePolicy,
    images: &mut Images<'_>,
) -> Result<ShaderRegistration<ShaderBinding>, WorldMaterialError> {
    let image = std::str::from_utf8(raw)
        .map_err(|_| WorldMaterialError::Boundary("invalid shader name"))?;
    let display = std::str::from_utf8(crate::shader::registration_key(raw))
        .map_err(|_| WorldMaterialError::Boundary("invalid shader name"))?;
    let definition = catalog.find_canonical(&canonical_path(display));
    let mut defaulted = None;
    let mut sun = None;
    let material = if let Some(definition) = definition {
        let compiled = compile_definition(definition, images, lighting, policy)?;
        defaulted = compiled.defaulted;
        sun = compiled.sun;
        compiled.material
    } else {
        let image = images.raster(
            image,
            ImageUse {
                sampler: Sampler {
                    wrap: if !mipmaps
                        && matches!(
                            lighting.source,
                            LightSource::None | LightSource::TwoDimensional
                        ) {
                        Wrap::Clamp
                    } else {
                        Wrap::Repeat
                    },
                    mipmaps,
                    ..Sampler::default()
                },
                allow_picmip: mipmaps,
                ..ImageUse::default()
            },
            images.raster_target(),
        );
        match image {
            Ok(image) => default_material(display, image, lighting, policy, images)?,
            Err(ResourceError::Missing(_)) => {
                let kind = NativeDefaultKind::MissingImage;
                defaulted = Some(kind);
                finish_material(
                    display,
                    Vec::new(),
                    MaterialSettings {
                        sort: 7.,
                        ..Default::default()
                    },
                    lighting,
                    policy,
                    images.assets,
                )?
            }
            Err(error) => return Err(error.into()),
        }
    };
    let mut binding = ShaderBinding::material(material, lighting.source);
    binding.lightmap = lighting.lightmap;
    let mut registration = images
        .assets
        .shader_registration_value(raw, binding, defaulted)
        .map_err(WorldMaterialError::Boundary)?;
    registration.sun = sun;
    Ok(registration)
}

fn update_wave(wave: &mut crate::shader::Waveform, update: crate::shader::WaveUpdate) {
    if let Some(function) = update.function {
        wave.function = function
    }
    for (target, value) in [
        &mut wave.base,
        &mut wave.amplitude,
        &mut wave.phase,
        &mut wave.frequency,
    ]
    .into_iter()
    .zip(update.values)
    {
        if let Some(value) = value {
            *target = value
        }
    }
}
pub struct CompiledShader {
    pub material: MaterialId,
    pub defaulted: Option<NativeDefaultKind>,
    pub sun: Option<crate::shader::SunParms>,
}

pub fn compile_definition(
    definition: &ShaderDef,
    images: &mut Images<'_>,
    lighting: NamedShaderLighting,
    policy: ShaderCompilePolicy,
) -> Result<CompiledShader, WorldMaterialError> {
    use crate::shader::{HeaderOp, ShaderAction, ShaderFailure, ShaderMap, StageOp};
    let mut settings = MaterialSettings {
        sort: 0.,
        ..Default::default()
    };
    let mut slots = Vec::<ShaderSlot>::with_capacity(crate::shader::MAX_STAGES);
    let mut no_mips = false;
    let mut no_picmip = false;
    let mut reason = None;
    let mut sun = None;
    let mut deforms = 0;
    let mut outer = None;
    let mut inner = None;
    let mut cloud_height = 512.;
    let mut sky = false;
    for op in &definition.ops {
        match &op.action {
            ShaderAction::Abort(ShaderFailure::Native) => {
                reason = Some(crate::assets::ExplicitDefaultReason::Parse);
                break;
            }
            ShaderAction::Abort(ShaderFailure::Limit(_)) => {
                return Err(WorldMaterialError::Shader("native shader limit exceeded"));
            }
            ShaderAction::Abort(ShaderFailure::InvalidData(error)) => {
                return Err(WorldMaterialError::Shader(error));
            }
            ShaderAction::BeginStage => {
                if slots.len() == crate::shader::MAX_STAGES {
                    return Err(WorldMaterialError::Shader("native shader stage limit"));
                }
                slots.push(ShaderSlot::new());
            }
            ShaderAction::EndStage => slots
                .last_mut()
                .ok_or(WorldMaterialError::Shader("stage close without stage"))?
                .commit()?,
            ShaderAction::EndShader => {
                if slots.is_empty() && !sky && settings.content_flags & 64 == 0 {
                    reason = Some(crate::assets::ExplicitDefaultReason::Parse)
                }
                break;
            }
            ShaderAction::Header(header) => match header {
                HeaderOp::Cull(value) => settings.cull = *value,
                HeaderOp::Sort(value) => settings.sort = *value,
                HeaderOp::SurfaceParm { surface, contents } => {
                    settings.surface_flags |= surface;
                    settings.content_flags |= contents
                }
                HeaderOp::NoMipmaps => {
                    no_mips = true;
                    no_picmip = true
                }
                HeaderOp::NoPicmip => no_picmip = true,
                HeaderOp::PolygonOffset => settings.polygon_offset = true,
                HeaderOp::EntityMergable => {}
                HeaderOp::Portal => {
                    settings.portal = true;
                    settings.sort = 1.
                }
                HeaderOp::ClampTime(value) => settings.clamp_time = Some(*value),
                HeaderOp::Sun(value) => sun = Some(*value),
                HeaderOp::SkyOuter(name) => {
                    if name.is_some() {
                        outer = load_box(name.as_deref(), Wrap::Clamp, images)?;
                    }
                }
                HeaderOp::SkyHeight(value) => cloud_height = *value,
                HeaderOp::SkyInner(name) => {
                    if name.is_some() {
                        inner = load_box(name.as_deref(), Wrap::Repeat, images)?;
                    }
                    sky = true
                }
                HeaderOp::FogColor(values) => {
                    let fog = settings.fog.get_or_insert(crate::shader::FogParms {
                        color: [0.; 3],
                        depth_opaque: 0.,
                    });
                    for (target, value) in fog.color.iter_mut().zip(values) {
                        if let Some(value) = value {
                            *target = *value
                        }
                    }
                }
                HeaderOp::FogDepth(value) => {
                    settings
                        .fog
                        .get_or_insert(crate::shader::FogParms {
                            color: [0.; 3],
                            depth_opaque: 0.,
                        })
                        .depth_opaque = *value
                }
                HeaderOp::Deform(value) => {
                    if deforms == settings.deforms.len() {
                        return Err(WorldMaterialError::Shader("native shader deform limit"));
                    }
                    if value.is_some_and(|value| {
                        matches!(
                            value,
                            Deform::AutoSprite
                                | Deform::AutoSprite2
                                | Deform::ProjectionShadow
                                | Deform::Text(_)
                        )
                    }) {
                        return Err(WorldMaterialError::Shader(
                            "shader topology deformation pending",
                        ));
                    }
                    settings.deforms[deforms] = *value;
                    deforms += 1;
                }
                HeaderOp::Unsupported(declaration) => {
                    if declaration.effect != crate::shader::DeclarationEffect::CompileOnly {
                        return Err(WorldMaterialError::Shader(
                            "unsupported runtime shader declaration",
                        ));
                    }
                }
            },
            ShaderAction::Stage(action) => {
                let slot = slots
                    .last_mut()
                    .ok_or(WorldMaterialError::Shader("shader directive without stage"))?;
                match action {
                    StageOp::Map(map) => {
                        let loaded = match map {
                            ShaderMap::White => Ok((
                                BundleImage::Image(ImageId(0)),
                                Some(Sampler {
                                    mipmaps: false,
                                    ..Sampler::default()
                                }),
                            )),
                            ShaderMap::Lightmap => {
                                slot.stage.is_lightmap = true;
                                Ok((
                                    if matches!(
                                        lighting.source,
                                        LightSource::Page(_)
                                            | LightSource::ExternalPage(_)
                                            | LightSource::Samples
                                    ) {
                                        BundleImage::Lightmap
                                    } else {
                                        BundleImage::Image(ImageId(0))
                                    },
                                    Some(Sampler {
                                        wrap: Wrap::Clamp,
                                        mipmaps: false,
                                        ..Sampler::default()
                                    }),
                                ))
                            }
                            ShaderMap::Image { name, clamp } => images
                                .raster(
                                    name,
                                    ImageUse {
                                        sampler: Sampler {
                                            wrap: if *clamp { Wrap::Clamp } else { Wrap::Repeat },
                                            mipmaps: !no_mips,
                                            ..Default::default()
                                        },
                                        allow_picmip: !no_picmip,
                                        ..Default::default()
                                    },
                                    images.raster_target(),
                                )
                                .map(|image| (BundleImage::Image(image.id), Some(image.sampler))),
                        };
                        match loaded {
                            Ok((image, sampler)) => {
                                slot.images[0] = Some(image);
                                if let Some(sampler) = sampler {
                                    slot.stage.sampler = sampler
                                }
                            }
                            Err(ResourceError::Missing(_)) => {
                                slot.images[0] = None;
                                reason = Some(crate::assets::ExplicitDefaultReason::ImageMissing);
                                break;
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                    StageOp::AnimationSpeed(value) => slot.frequency = *value,
                    StageOp::AnimationFrame(name) => {
                        if slot.count == slot.images.len() {
                            return Err(WorldMaterialError::Shader(
                                "native shader animation limit",
                            ));
                        }
                        let loaded = images.raster(
                            name,
                            ImageUse {
                                sampler: Sampler {
                                    mipmaps: !no_mips,
                                    ..Default::default()
                                },
                                allow_picmip: !no_picmip,
                                ..Default::default()
                            },
                            images.raster_target(),
                        );
                        match loaded {
                            Ok(image) => {
                                slot.images[slot.count] = Some(BundleImage::Image(image.id));
                                slot.count += 1;
                                slot.stage.sampler = image.sampler
                            }
                            Err(ResourceError::Missing(_)) => {
                                slot.images[slot.count] = None;
                                reason = Some(crate::assets::ExplicitDefaultReason::ImageMissing);
                                break;
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                    StageOp::BlendSource(value) => slot.source = Some(*value),
                    StageOp::BlendDestination(value) => slot.destination = Some(*value),
                    StageOp::BlendComplete => {
                        if !slot.depth_explicit {
                            slot.pending_depth = false
                        }
                    }
                    StageOp::RgbGen(value) => {
                        slot.rgb = Some(*value);
                        slot.stage.rgb_gen = *value;
                        if *value == RgbGen::Vertex && slot.stage.alpha_gen == AlphaGen::Identity {
                            slot.stage.alpha_gen = AlphaGen::Vertex
                        }
                    }
                    StageOp::RgbWave(update) => {
                        update_wave(&mut slot.rgb_wave, *update);
                        slot.rgb = Some(RgbGen::Wave(slot.rgb_wave));
                        slot.stage.rgb_gen = RgbGen::Wave(slot.rgb_wave)
                    }
                    StageOp::AlphaGen(value) => slot.stage.alpha_gen = *value,
                    StageOp::AlphaWave(update) => {
                        update_wave(&mut slot.alpha_wave, *update);
                        slot.stage.alpha_gen = AlphaGen::Wave(slot.alpha_wave)
                    }
                    StageOp::TcGen(value) => {
                        slot.stage.texgen = *value;
                        slot.tcgen = true
                    }
                    StageOp::TcVector(update) => {
                        for (target, values) in slot.vectors.iter_mut().zip(update) {
                            for (target, value) in target.iter_mut().zip(values) {
                                if let Some(value) = value {
                                    *target = *value
                                }
                            }
                        }
                        slot.stage.texgen = TexCoordGen::Vector(slot.vectors);
                        slot.tcgen = true
                    }
                    StageOp::TexMod(value) => {
                        let target = slot
                            .stage
                            .tcmods
                            .get_mut(slot.texmods)
                            .ok_or(WorldMaterialError::Shader("native shader texmod limit"))?;
                        *target = value.map(TcMod::Script);
                        slot.texmods += 1
                    }
                    StageOp::AlphaFunc(value) => slot.pending_alpha = *value,
                    StageOp::DepthFunc(value) => slot.pending_func = *value,
                    StageOp::DepthWrite => {
                        slot.pending_depth = true;
                        slot.depth_explicit = true
                    }
                    StageOp::Detail => slot.stage.detail = true,
                    StageOp::Unsupported(_) => {
                        return Err(WorldMaterialError::Shader(
                            "unsupported runtime shader declaration",
                        ));
                    }
                }
            }
        }
    }
    if sky {
        settings.sky = Some(Sky::Cube {
            outer_box: outer,
            inner_box: inner,
            clouds: CloudSphere::native(cloud_height),
            rotation: None,
            params: CubeSkyParams::default(),
        });
    }
    let material = finish_material(
        &definition.name,
        slots,
        settings,
        lighting,
        policy,
        images.assets,
    )?;
    let defaulted = reason
        .map(|reason| images.assets.explicit_default(material, reason))
        .transpose()
        .map_err(WorldMaterialError::Boundary)?;
    Ok(CompiledShader {
        material,
        defaulted,
        sun,
    })
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
        *slot = match images.raster(
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
        ) {
            Ok(image) => image.id,
            Err(ResourceError::Missing(_)) => images.native_default_image()?.id,
            Err(error) => return Err(error.into()),
        };
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
            is_lightmap: true,
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
