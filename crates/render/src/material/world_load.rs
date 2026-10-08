//! File-boundary conversion into the one runtime world/material table.
use super::{
    load_catalog,
    resources::{
        ImageRole, ImageSettings, ImageUse, Images, PaletteSource, ResolvedImage, ResourceError,
        load_palette,
    },
};
use crate::{
    Assets, BlendPhase, CpuPresentation, LightStyle, PaletteOperation, PaletteTransform,
    PerspectiveStep, Refdef,
    assets::{
        CubeSkyParams, DepthFunc, Filter, Flow, ImageId, MaterialId, MaterialSettings, Sampler,
        Sky, SkyDistance, Stage, StageTexture, TcMod, TextureIntensity, UvWarp, Wrap, upload,
    },
    lightmap::{
        AtlasBuilder, AtlasRegion, LightmapError, Q1GlScale, build_quake_rgb, build_quake2_rgb,
        shift_quake3_rgb,
    },
    shader::{
        AlphaFunc, AlphaGen, BlendFactor, Cull, Deform, RgbGen, ShaderDef, StageBlend, TexCoordGen,
        TextureMap, canonical_name,
    },
    sky::{CloudSphere, LayeredSphere, split_layered_sky},
    surface_cache::{IndexedLighting, IndexedTexture},
    world::{
        SurfaceMaterial, WorldId,
        geometry::{
            GeometryError, GeometryOptions, LightSource, WorldGeometry,
            grid::{GridFan, GridOptions, subdivide_surface},
            load_geometry,
        },
        load_visibility,
    },
};
use qa_content::vfs::Vfs;
use qa_formats::{bsp::Map, image::RasterPolicy};

/// Worldspawn/configstring data copied at the entity boundary. Rendering owns
/// no entity-lump parser and resolves this name only during asset registration.
#[derive(Clone, Copy, Debug)]
pub struct SkyEnvironment {
    name: [u8; 64],
    length: u8,
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
            || name.len() > 63
            || !degrees_per_second.is_finite()
            || !axis.0.iter().all(|component| component.is_finite())
            || std::str::from_utf8(name).is_err()
            || name
                .iter()
                .any(|&byte| byte == 0 || byte == b'/' || byte == b'\\')
        {
            return Err("invalid sky environment");
        }
        let mut result = Self {
            name: [0; 64],
            length: name.len() as u8,
            degrees_per_second,
            axis,
        };
        result.name[..name.len()].copy_from_slice(name);
        Ok(result)
    }
    pub fn name(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.name[..self.length as usize])
    }
}
impl Default for SkyEnvironment {
    fn default() -> Self {
        let mut name = [0; 64];
        name[..6].copy_from_slice(b"unit1_");
        Self {
            name,
            length: 6,
            degrees_per_second: 0.0,
            axis: qa_core::primitives::Vec3::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WorldLoadOptions {
    pub geometry: GeometryOptions,
    /// Explicit RGB/custom presentation; stock Q1/Q2 defaults retain indices.
    pub cpu_rgb: bool,
    pub q1_gl_scale: Q1GlScale,
    pub q2_modulate: f32,
    pub map_overbright: u8,
    pub renderer_overbright: u8,
    pub sky_environment: SkyEnvironment,
    /// None resolves original native defaults at this map-format boundary.
    pub image_settings: Option<ImageSettings>,
}
impl Default for WorldLoadOptions {
    fn default() -> Self {
        Self {
            geometry: GeometryOptions::default(),
            cpu_rgb: false,
            q1_gl_scale: Q1GlScale::OriginalOverbright,
            q2_modulate: 1.0,
            map_overbright: 2,
            renderer_overbright: 0,
            sky_environment: SkyEnvironment::default(),
            image_settings: None,
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
pub struct LoadedWorld {
    pub world: WorldId,
    pub presentation: PresentationDefaults,
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

pub fn load_world(
    vfs: &Vfs,
    map: &Map<'_>,
    assets: &mut Assets,
    mut options: WorldLoadOptions,
) -> Result<LoadedWorld, WorldMaterialError> {
    let family = map.bsp.format.family();
    let image_settings = options
        .image_settings
        .unwrap_or_else(|| ImageSettings::native(family));
    if options.renderer_overbright > 2
        || options.map_overbright > 8
        || options.map_overbright < options.renderer_overbright
    {
        return Err(WorldMaterialError::Boundary("invalid overbright settings"));
    }
    let palette = match family {
        1 => Some(load_palette(
            vfs,
            assets,
            PaletteSource::Lmp {
                colors: b"gfx/palette.lmp",
                shades: b"gfx/colormap.lmp",
            },
        )?),
        2 => Some(load_palette(
            vfs,
            assets,
            PaletteSource::Pcx {
                image: b"pics/colormap.pcx",
            },
        )?),
        _ => None,
    };
    let mut presentation = PresentationDefaults {
        cpu: CpuPresentation::Rgb,
        perspective_step: PerspectiveStep::Eight,
        blend_phase: BlendPhase::AfterView,
        palette_transform: None,
        lightstyles: [LightStyle::default(); 256],
        identity_light: 1.0,
    };
    // Native presentation choices are resolved here, independently of movement,
    // client protocol and module format. There is no family branch in drawing.
    if family == 1 {
        presentation.lightstyles[0].indexed_scale = 264;
        presentation.lightstyles[0].rgb = [264.0 / 256.0; 3];
        presentation.blend_phase = BlendPhase::FinalPalette;
        presentation.palette_transform = Some(PaletteTransform::default());
    } else if family == 2 {
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
        options.geometry.vertex_color_shift =
            (options.map_overbright - options.renderer_overbright) as i8;
        presentation.identity_light = 1.0 / (1u32 << options.renderer_overbright) as f32;
    }
    if !options.cpu_rgb
        && let Some(palette) = palette
    {
        presentation.cpu = CpuPresentation::Indexed {
            palette,
            lighting: if family == 1 {
                IndexedLighting::Gray
            } else {
                IndexedLighting::NativeRgb
            },
            ambient: 0,
            fullbright: false,
        };
    }
    let mut geometry = load_geometry(map, options.geometry)?;
    let visibility =
        load_visibility(map).map_err(|_| WorldMaterialError::Boundary("world visibility"))?;
    let (lightmaps, regions) = prepare_lightmaps(
        &geometry,
        assets,
        options,
        &presentation.lightstyles,
        family,
    )?;
    for surface in &geometry.surfaces {
        if family != 3
            && let Some(region) = regions[surface.source_id as usize]
        {
            for vertex in &mut geometry.vertices[surface.vertices.indices()] {
                vertex.vertex.lightmap_coord =
                    region.uv_from_texture(vertex.vertex.texcoord, surface.texture_minima);
            }
        }
    }
    let catalog = load_catalog(vfs).map_err(WorldMaterialError::Catalog)?;
    let mut images = Images::new(
        vfs,
        assets,
        match family {
            1 => RasterPolicy::Standard,
            2 => RasterPolicy::Quake2,
            _ => RasterPolicy::Quake3,
        },
        image_settings,
    )?;
    let mut diagnostics = Vec::new();
    let mut textures = Vec::with_capacity(map.textures.len());
    let mut layered_skies = Vec::with_capacity(map.textures.len());
    if family == 1 {
        for texture in &map.textures {
            textures.push(match texture {
                // R_InitSky uploads the two split layers directly, rather
                // than uploading the original 256x128 texture as a surface.
                Some(mip) if mip.name.starts_with(b"sky") => ImageId(0),
                Some(mip) => images.embedded(
                    mip,
                    palette.ok_or(WorldMaterialError::Boundary("missing legacy palette"))?,
                    mip.name.starts_with(b"{"),
                )?,
                None => ImageId(0),
            });
            layered_skies.push(match texture {
                Some(mip) if mip.name.starts_with(b"sky") => Some(load_layered_sky(
                    mip,
                    palette.ok_or(WorldMaterialError::Boundary("missing sky palette"))?,
                    &mut images,
                )?),
                _ => None,
            });
        }
    }
    // GPU interpolation matches native pre-fragmented liquid/sky polygons;
    // the original convex boundaries remain the CPU edge-scanner input.
    for source in 0..geometry.surfaces.len() {
        let surface = &geometry.surfaces[source];
        let legacy_name = if family == 1 {
            surface
                .source_texture
                .and_then(|i| map.textures.get(i as usize))
                .and_then(Option::as_ref)
                .map(|mip| mip.name)
        } else {
            None
        };
        let warp = legacy_name.is_some_and(|name| name.starts_with(b"*"))
            || (family == 2 && surface.source_flags & 8 != 0);
        let sky = legacy_name.is_some_and(|name| name.starts_with(b"sky"));
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
                    spacing: if family == 1 { 128.0 } else { 64.0 },
                    fan: if family == 1 {
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
    let mut bindings = Vec::with_capacity(geometry.surfaces.len());
    for surface in &geometry.surfaces {
        if surface.no_draw {
            bindings.push(SurfaceMaterial::default());
            continue;
        }
        let (name, image, mut scale, flags) = if family == 1 {
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
        } else if family == 2 {
            let info = &map.texture_info[surface
                .source_texture_info
                .ok_or(WorldMaterialError::Boundary("missing texture info"))?
                as usize];
            let name = std::str::from_utf8(info.name)
                .map_err(|_| WorldMaterialError::Boundary("invalid texture name"))?
                .to_owned();
            let image = if surface.source_flags & 4 != 0 {
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
            let name = canonical_name(
                std::str::from_utf8(shader.name)
                    .map_err(|_| WorldMaterialError::Boundary("invalid shader name"))?,
            );
            (name, ImageId(0), [1.0; 2], shader.surface_flags as u32)
        };
        let legacy_sky = if family == 1 {
            surface
                .source_texture
                .and_then(|index| layered_skies.get(index as usize))
                .copied()
                .flatten()
        } else if family == 2 && flags & 4 != 0 {
            Some(Sky::Cube {
                outer_box: Some(load_indexed_box(
                    options.sky_environment,
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
                        || image_settings.sky_mip
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
        if (family == 1 && name.starts_with('*')) || (family == 2 && flags & 8 != 0) {
            scale = [1.0 / 64.0; 2];
        }
        let material = if family == 3 {
            if let Some(definition) = catalog.find_canonical(&name).filter(|d| d.valid) {
                compile_definition(definition, &mut images)?
            } else {
                if catalog.find_canonical(&name).is_some() {
                    diagnostics.push(format!("native shader fallback: {name}"));
                }
                let image = images.raster(&name, ImageUse::default())?;
                default_material(&name, image, surface.light_source, &mut images)?
            }
        } else {
            legacy_material(
                &name,
                image,
                flags,
                family,
                surface.light_source,
                legacy_sky,
                &mut images,
            )?
        };
        bindings.push(SurfaceMaterial {
            material,
            lightmap: lightmaps[surface.source_id as usize].unwrap_or(ImageId(0)),
            texture_scale: scale,
        });
    }
    let world = images
        .assets
        .register_world_with_bindings(geometry, visibility, &bindings)
        .map_err(WorldMaterialError::Boundary)?;
    for conflict in &images.conflicts {
        diagnostics.push(format!(
            "native first-image flags retained: {} ({:?} requested {:?})",
            conflict.name, conflict.first, conflict.requested
        ));
    }
    Ok(LoadedWorld {
        world,
        presentation,
        diagnostics: diagnostics.into_boxed_slice(),
    })
}

fn prepare_lightmaps(
    geometry: &WorldGeometry,
    assets: &mut Assets,
    options: WorldLoadOptions,
    styles: &[LightStyle; 256],
    family: u8,
) -> Result<(Vec<Option<ImageId>>, Vec<Option<AtlasRegion>>), WorldMaterialError> {
    let mut atlas = AtlasBuilder::load(128, 4096)?;
    let mut regions = vec![None; geometry.surfaces.len()];
    let mut page_regions: Vec<(u32, AtlasRegion)> = Vec::new();
    for surface in &geometry.surfaces {
        if surface.no_draw {
            continue;
        }
        let rgb: Vec<_> = geometry.light_samples[surface.light_samples.indices()]
            .iter()
            .flatten()
            .copied()
            .collect();
        let region = match surface.light_source {
            LightSource::Page(page) => {
                if let Some((_, region)) = page_regions.iter().find(|(id, _)| *id == page) {
                    *region
                } else {
                    let rgb = shift_quake3_rgb(
                        &rgb,
                        options.map_overbright,
                        options.renderer_overbright,
                    )?;
                    let region = atlas.insert_page(&rgb)?;
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
                let active: Vec<_> = surface
                    .styles
                    .iter()
                    .take_while(|&&s| s != 255)
                    .map(|&s| styles[s as usize])
                    .collect();
                let rgb = if family == 1 {
                    let scales: Vec<_> = active
                        .iter()
                        .map(|style| u32::from(style.indexed_scale))
                        .collect();
                    build_quake_rgb(
                        surface.lightmap_grid[0],
                        surface.lightmap_grid[1],
                        &rgb,
                        &scales,
                        options.q1_gl_scale,
                    )?
                } else {
                    let scales: Vec<_> = active.iter().map(|style| style.rgb).collect();
                    build_quake2_rgb(
                        surface.lightmap_grid[0],
                        surface.lightmap_grid[1],
                        &rgb,
                        &scales,
                        options.q2_modulate,
                    )?
                };
                atlas.insert(surface.lightmap_grid[0], surface.lightmap_grid[1], &rgb)?
            }
            _ => continue,
        };
        regions[surface.source_id as usize] = Some(region);
    }
    let atlas = atlas.finish();
    let mut ids = Vec::with_capacity(atlas.page_count());
    for page in atlas.pages() {
        let rgba: Vec<_> = page
            .rgb
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        let id = assets
            .register_image(128, 128, &rgba)
            .map_err(WorldMaterialError::Boundary)?;
        // Atlas bytes already include native lightstyle/modulate/overbright.
        // Image intensity/gamma and ordinary mip generation do not apply.
        assets
            .prepare_image(id, upload::UploadParams::default())
            .map_err(WorldMaterialError::Boundary)?;
        ids.push(id);
    }
    Ok((
        regions
            .iter()
            .map(|region| region.map(|r| ids[r.page as usize]))
            .collect(),
        regions,
    ))
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

fn compile_definition(
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
    environment: SkyEnvironment,
    palette: crate::PaletteId,
    images: &mut Images<'_>,
) -> Result<[ImageId; 6], WorldMaterialError> {
    let mut result = [ImageId(0); 6];
    for (slot, suffix) in result.iter_mut().zip(["rt", "lf", "bk", "ft", "up", "dn"]) {
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
    for (slot, suffix) in result.iter_mut().zip(["rt", "lf", "bk", "ft", "up", "dn"]) {
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
            )?
            .id;
    }
    Ok(Some(result))
}

fn legacy_material(
    name: &str,
    image: ImageId,
    flags: u32,
    family: u8,
    light: LightSource,
    sky_material: Option<Sky>,
    images: &mut Images<'_>,
) -> Result<MaterialId, WorldMaterialError> {
    let warp = (family == 1 && name.starts_with('*')) || (family == 2 && flags & 8 != 0);
    let sky = (family == 1 && name.starts_with("sky")) || (family == 2 && flags & 4 != 0);
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
                        Some(StageBlend {
                            source: BlendFactor::SourceAlpha,
                            destination: BlendFactor::OneMinusSourceAlpha,
                        })
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
    if family == 1 && name.starts_with('{') {
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
    if family == 2 && flags & 64 != 0 {
        base.tcmods[usize::from(warp)] = Some(TcMod::Flow(Flow {
            speed: if warp { 0.5 } else { 1.0 / 40.0 },
            amplitude: if warp { [-1.0, 0.0] } else { [-64.0, 0.0] },
            cycle_start: if warp { [0.0; 2] } else { [-64.0, 0.0] },
        }));
    }
    let alpha = if family == 2 && flags & 16 != 0 {
        Some(0.33)
    } else if family == 2 && flags & 32 != 0 {
        Some(0.66)
    } else {
        None
    };
    if family == 2 && (warp || alpha.is_some()) {
        base.texture_intensity = TextureIntensity::NeutralizeUpload;
    }
    let mut settings = MaterialSettings::default();
    if let Some(alpha) = alpha {
        base.alpha_gen = AlphaGen::Const(alpha);
        base.blend = Some(StageBlend {
            source: BlendFactor::SourceAlpha,
            destination: BlendFactor::OneMinusSourceAlpha,
        });
        base.depth_write = false;
        settings.sort = 9.0;
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
    let palette_colors = std::array::from_fn(|index| colors.color(index as u8).to_le_bytes());
    let layers =
        split_layered_sky(mip.levels[0], &palette_colors).map_err(WorldMaterialError::Boundary)?;
    let corrected = split_layered_sky(mip.levels[0], &images.corrected_palette(palette)?)
        .map_err(WorldMaterialError::Boundary)?;
    let opaque = IndexedTexture::load_base(128, 128, &layers.opaque_indices, None)
        .map_err(WorldMaterialError::Boundary)?;
    let masked = IndexedTexture::load_base(128, 128, &layers.masked_indices, Some(0))
        .map_err(WorldMaterialError::Boundary)?;
    let ids = [
        images
            .assets
            .register_rgba_with_indexed(128, 128, &layers.opaque_rgba, opaque)
            .map_err(WorldMaterialError::Boundary)?,
        images
            .assets
            .register_rgba_with_indexed(128, 128, &layers.masked_rgba, masked)
            .map_err(WorldMaterialError::Boundary)?,
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
