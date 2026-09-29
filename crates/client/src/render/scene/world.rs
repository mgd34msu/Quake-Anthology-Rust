//! Unified world preparation over Q1/Q2 brush surfaces and Q3 BSP.
//!
//! Donor provenance: `src/render/scene/world.ts`. Loading stays synchronous:
//! content maps adapt into owned build data, surfaces build against the
//! scene shader registry, and each view assembles visibility, world and
//! inline-model operations, fog, and image uploads into one ordered
//! [`PreparedWorldView`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use qa_content::bsp::{BspFormat, NodeChild as ContentChild};
use qa_core::math::{cross3, dot3, normalize3, scale3, sub3, vec2, vec3, vec4, Bounds, Plane, Vec3, Vec4};

use crate::materials::deform::{DeformGeometry, DeformView, ProjectionShadowContext, RendererNoise};
use crate::materials::dlight::{
    bmodel_dlight_mask, face_dlight_mask, grid_dlight_mask, project_dlight_texture, receives_projected_dlights,
    transform_dlights,
};
use crate::materials::evaluate::{
    evaluate_material_passes, prepare_material_batches, DynamicLightBatches, FogVolumeInput, MaterialBatch,
    MaterialDrawContext, PairEnv, Q1FogInput, Q2LightPass as EvaluateLightPass, TextureRef, Texturing,
};
use crate::materials::fog::{create_fog_texture, prepare_fog_volume, FogBrushMap, FogCoordinates, FogVolume};
use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
use crate::materials::iterator::MaterialIteratorKind;
use crate::materials::legacy::{
    create_q1_material, create_q2_material, prepare_legacy_material_batches, q1_animated_texture, q1_sky_tex_coords,
    q1_surface_kind, q1_texture_animations, split_q1_sky_texture, IndexedImage, LegacyMaterial,
    LegacyMaterialDrawContext, Q1SkyLayer, Q1Surface,
};
use crate::materials::lighting::{
    build_q1_lightmap, build_q2_lightmap, direct_lightmap_pixels, BspLighting, BuiltLightmap, LightmapFace,
    Q1LightmapEncoding, Q2LightStyle, Q2Mono, SurfaceDynamicLight,
};
use crate::materials::q3_lighting::{DynamicLight, EntityLighting};
use crate::materials::sky::SKY_FACE_SUFFIXES;
use crate::materials::state::{
    AlphaTest as MaterialAlphaTest, BlendFactor as MaterialBlendFactor, CullFace as MaterialCullFace,
    DepthTest as MaterialDepthTest,
};
use crate::render::types::{
    AlphaTest, BatchFog, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch,
    FogEffect, ImageLevel, ImageResourceOperation, ImageSource, LevelContent, MultitextureVertex, PairEnvironment,
    PolygonOffset, Q2Fog, Q2FogOperation, Q2FragmentLight, Q2LightPass, Q2ModelFragmentLight, Q2ShadowAtlas,
    RenderCamera, RenderImage, RenderOperation, RenderState, RenderVertex, RenderView, RenderViewState, RendererImage,
    SceneFog, SkyVertex, SourceTime, TextureBinding, TextureBundle, TextureFilter, TextureSampling, ViewClear,
    ViewClip, ViewTarget,
};
use crate::render::{RenderError, SceneLight};
use crate::view::{
    bounds_in_frustum, camera_frustum, far_clip, local_point, model_scale, portal_clip_plane, world_point,
    world_vector, ModelTransform, SceneCamera,
};

use super::geometry::{geometry_bounds, prepare_brush_face, BrushFace, BrushMapData, BrushTextureInfo};
use super::material_registrations::{
    alloc_world_identity, current_remap, material_revision, publish_remap, remove_remap, MaterialRemap,
    RegisteredSceneMaterial, ShaderWorldIdentity,
};
use super::patch::tessellate_patch;
use super::patch_lod::{create_patch_grid, prepare_patch_grids, select_patch_lod, PatchGrid};
use super::portal::{portal_camera, portal_surface_offscreen, PortalEntity};
use super::q1_fog::fog_scene_operations;
use super::q2_sky::{q2_sky_sides, Q2SkyView};
use super::resources::{rgba_image, SceneImageRegistry};
use super::shaders::{SceneShaderBinding, SceneShaderRegistry};
use super::shadow_geometry::{shadow_material_geometry, ShadowMaterialContext};
use super::shadows::{
    shadow_caster, shadow_mesh, PreparedShadows, Q2ShadowScene, ShadowAtlasOptions, ShadowCaster, ShadowMesh,
    ShadowWorldInput, StaticShadowWorld,
};
use super::submissions::{
    compiled_draw_group, create_source_scene_order, finish_scene_operations, sequence_draw_group, source_draw_group,
    SceneOperation, SequencePhase, SourceEntityOrder, SourceSceneOrder, SourceSurfaceOrder,
};
use super::textures::{SceneTexture, SceneTextureLoadOptions, TextureFamily};
use super::visibility::{
    visible_world, world_point_leaf, BspChild, VisibleWorld, WorldKind, WorldLeaf, WorldMap, WorldNode,
    WorldVisibility, WorldVisibilityOptions,
};

/// Prepared world view: ordered view plus its image uploads.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedWorldView {
    /// Image uploads and releases in order.
    pub image_operations: Vec<ImageResourceOperation>,
    /// Ordered view.
    pub view: RenderView,
}

/// World scene build options.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldSceneOptions {
    /// Patch subdivision threshold.
    pub subdivisions: f32,
    /// Q3 lightmap overbright shift.
    pub q3_lightmap_overbright: u32,
    /// Q1 lightmap encoding.
    pub q1_lightmap_encoding: Q1LightmapEncoding,
    /// Q1 water alpha.
    pub q1_water_alpha: f32,
    /// Q2 sky name.
    pub q2_sky_name: Option<String>,
    /// Q2 light modulation.
    pub q2_light_modulate: f32,
}

impl Default for WorldSceneOptions {
    fn default() -> Self {
        Self {
            subdivisions: 4.0,
            q3_lightmap_overbright: 2,
            q1_lightmap_encoding: Q1LightmapEncoding::Rgb,
            q1_water_alpha: 1.0,
            q2_sky_name: None,
            q2_light_modulate: 1.0,
        }
    }
}

/// Q1 fog parameters for a view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1FogParams {
    /// Fog color.
    pub color: Vec3,
    /// Density.
    pub density: f32,
    /// Sky blend factor.
    pub sky_factor: f32,
}

/// Q2 fragment lighting for a view.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FragmentLighting {
    /// Fragment lights.
    pub lights: Vec<Q2FragmentLight>,
    /// Shadow atlas.
    pub atlas: Option<Q2ShadowAtlas>,
}

/// Inline model submission.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineModel {
    /// Model index.
    pub model: usize,
    /// Model transform.
    pub transform: ModelTransform,
    /// Animation frame override.
    pub animation_frame: Option<f32>,
    /// Alternate animation override.
    pub alternate_animation: Option<bool>,
    /// Whether the model casts shadows.
    pub casts_shadow: bool,
    /// Entity color override.
    pub entity_rgba: Option<[u8; 4]>,
}

/// Flare hook: host turns flare surfaces into operations.
pub type FlareHook = Arc<dyn Fn(&WorldSurface, &SceneCamera) -> Vec<RenderOperation> + Send + Sync>;

/// World view input: visibility options plus surface, light, and overlay state.
#[derive(Clone)]
pub struct WorldViewInput {
    /// Visibility options.
    pub visibility: WorldVisibilityOptions,
    /// Skip world-model surfaces.
    pub no_world_model: bool,
    /// Source admission for sortable views.
    pub source: Option<WorldSurfaceAdmission>,
    /// View camera.
    pub camera: SceneCamera,
    /// Draw target.
    pub target: ViewTarget,
    /// View time.
    pub time: SourceTime,
    /// Clear values override.
    pub clear: Option<ViewClear>,
    /// Q1 light styles (8.8 units, 256 entries).
    pub q1_styles: Vec<i32>,
    /// Q2 light styles (256 entries).
    pub q2_styles: Vec<Q2LightStyle>,
    /// Legacy surface dynamic lights.
    pub lights: Vec<SurfaceDynamicLight>,
    /// Q2 fragment lighting.
    pub q2_fragment_lighting: Option<Q2FragmentLighting>,
    /// Q1 fog.
    pub q1_fog: Option<Q1FogParams>,
    /// Q2 fog.
    pub q2_fog: Option<Q2Fog>,
    /// Q2 sky override.
    pub q2_sky: Option<Q2SkyView>,
    /// Source sky override (also reroutes Q1 skies).
    pub source_sky: Option<Q2SkyView>,
    /// Animation frame override.
    pub animation_frame: Option<f32>,
    /// Alternate Q1 animation.
    pub alternate_animation: bool,
    /// Patch curve error.
    pub curve_error: f32,
    /// Identity light scale.
    pub identity_light: f32,
    /// Render text rows.
    pub render_text: Vec<String>,
    /// Extra operations appended after world and inline models.
    pub operations: Vec<SceneOperation>,
    /// Operations before the view begins.
    pub before_view: Vec<RenderOperation>,
    /// Entity lighting override.
    pub lighting: Option<EntityLighting>,
    /// Entity color.
    pub entity_rgba: [u8; 4],
    /// Projection shadow context.
    pub projection_shadow: Option<ProjectionShadowContext>,
    /// Inline models.
    pub inline_models: Vec<InlineModel>,
    /// Flare hook.
    pub prepare_flare: Option<FlareHook>,
    /// Cinematic image bindings by dynamic handle.
    pub dynamic_images: HashMap<u32, RendererImage>,
}

impl WorldViewInput {
    /// Build a minimal input for a camera, target, and time.
    pub fn new(camera: SceneCamera, target: ViewTarget, time: SourceTime) -> Self {
        Self {
            visibility: WorldVisibilityOptions::default(),
            no_world_model: false,
            source: None,
            camera,
            target,
            time,
            clear: None,
            q1_styles: vec![256; 256],
            q2_styles: vec![
                Q2LightStyle {
                    rgb: vec3(1.0, 1.0, 1.0),
                    white: 3.0,
                };
                256
            ],
            lights: Vec::new(),
            q2_fragment_lighting: None,
            q1_fog: None,
            q2_fog: None,
            q2_sky: None,
            source_sky: None,
            animation_frame: None,
            alternate_animation: false,
            curve_error: 250.0,
            identity_light: 1.0,
            render_text: Vec::new(),
            operations: Vec::new(),
            before_view: Vec::new(),
            lighting: None,
            entity_rgba: [255, 255, 255, 255],
            projection_shadow: None,
            inline_models: Vec::new(),
            prepare_flare: None,
            dynamic_images: HashMap::new(),
        }
    }
}

/// Source admission: one context owns one accepted view.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldSurfaceAdmission {
    /// Source order.
    pub view: SourceSceneOrder,
    /// Submitted surface indices.
    pub submitted_surfaces: HashSet<usize>,
    /// Cached world operations.
    pub world_operations: Option<Vec<SceneOperation>>,
}

/// Build source admission for a view.
#[must_use]
pub fn create_world_surface_admission(view: SourceSceneOrder) -> WorldSurfaceAdmission {
    WorldSurfaceAdmission {
        view,
        submitted_surfaces: HashSet::new(),
        world_operations: None,
    }
}

/// Legacy surface lightmap images.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyLightmap {
    /// Lightmap face description.
    pub face: LightmapFace,
    /// Lightmap image.
    pub image: RendererImage,
    /// Direct-light image for translucent passes.
    pub direct: RendererImage,
    /// Q1 encoding.
    pub encoding: Q1LightmapEncoding,
}

/// Q1 sky layer images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1SkyLayers {
    /// Solid layer.
    pub solid: RendererImage,
    /// Overlay layer.
    pub overlay: RendererImage,
}

/// Q3 surface kind retained for culling and light masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SurfaceKindData {
    /// Planar polygon.
    Planar,
    /// Triangle soup.
    Triangles,
    /// Flare.
    Flare,
    /// Bezier patch.
    Patch,
}

/// World surface material data.
// Surfaces are stored in vectors; inline data keeps cache locality.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum WorldSurfaceData {
    /// Q3 shader surface.
    Q3 {
        /// Registered shader.
        shader: RegisteredSceneMaterial,
        /// Lightmap image.
        lightmap: Option<RendererImage>,
        /// Patch grid.
        grid: Option<PatchGrid>,
        /// Fog volume.
        fog: Option<FogVolume>,
        /// Fog index.
        fog_index: i32,
        /// Surface kind.
        kind: Q3SurfaceKindData,
        /// Flare surface.
        flare: bool,
        /// Skipped patch surface.
        skip: bool,
    },
    /// Legacy brush surface.
    Legacy {
        /// Authored shader override.
        shader: Option<RegisteredSceneMaterial>,
        /// Legacy material.
        material: LegacyMaterial,
        /// Fullbright overlay.
        fullbright: Option<RendererImage>,
        /// Lightmap images.
        lightmap: Option<LegacyLightmap>,
        /// Q1 sky layers.
        q1_sky: Option<Q1SkyLayers>,
    },
}

/// One prepared world surface.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldSurface {
    /// Shader name.
    pub shader_name: String,
    /// Base texture.
    pub base_texture: Option<SceneTexture>,
    /// Surface index.
    pub index: usize,
    /// Surface bounds.
    pub bounds: Bounds,
    /// Surface plane.
    pub plane: Option<Plane>,
    /// Surface geometry.
    pub geometry: MaterialGeometry,
    /// Material data.
    pub data: WorldSurfaceData,
}

impl WorldSurface {
    /// Registered shader, if any.
    #[must_use]
    pub fn shader(&self) -> Option<&RegisteredSceneMaterial> {
        match &self.data {
            WorldSurfaceData::Q3 { shader, .. } => Some(shader),
            WorldSurfaceData::Legacy { shader, .. } => shader.as_ref(),
        }
    }
}

/// Model surface range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildModel {
    /// Model bounds.
    pub bounds: Bounds,
    /// First surface or face.
    pub first: usize,
    /// Surface or face count.
    pub count: usize,
}

/// Raw Q3 vertex retained for overbright shifting at build time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawQ3Vertex {
    /// Position.
    pub position: [f32; 3],
    /// Texture coordinates.
    pub tex_coord: [f32; 2],
    /// Lightmap coordinates.
    pub lightmap_coord: [f32; 2],
    /// Normal.
    pub normal: [f32; 3],
    /// Byte color.
    pub color: [u8; 4],
}

/// Q3 build surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3BuildSurface {
    /// Surface kind.
    pub kind: Q3SurfaceKindData,
    /// Shader index.
    pub shader: usize,
    /// Fog index.
    pub fog: i32,
    /// Vertices.
    pub vertices: Vec<RawQ3Vertex>,
    /// Indices.
    pub indices: Vec<u32>,
    /// Patch control width.
    pub width: i32,
    /// Patch control height.
    pub height: i32,
    /// Lightmap image.
    pub lightmap_image: i32,
    /// Lightmap origin and vectors.
    pub lightmap_origin: [f32; 3],
    /// Lightmap vectors.
    pub lightmap_vectors: [[f32; 3]; 3],
}

/// Q3 build data.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3BuildData {
    /// Shader names and surface flags.
    pub shaders: Vec<(String, i32)>,
    /// Raw lightmap bytes.
    pub lightmaps: Vec<Vec<u8>>,
    /// Fog shader names and brush indices.
    pub fogs: Vec<(String, i32)>,
    /// Fog brush map.
    pub fog_map: FogBrushMap,
    /// Surfaces.
    pub surfaces: Vec<Q3BuildSurface>,
    /// Models.
    pub models: Vec<BuildModel>,
}

/// Embedded Q1 texture pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTexture {
    /// Texture name.
    pub name: String,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Four mip levels of indices.
    pub levels: [Vec<u8>; 4],
}

/// Q1 texture slot: external names load from disk, embedded pixels inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildTexture {
    /// Texture name.
    pub name: String,
    /// Embedded pixels, if any.
    pub embedded: Option<EmbeddedTexture>,
}

/// Unified legacy texture info.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyTexInfo {
    /// S projection.
    pub s: [f32; 4],
    /// T projection.
    pub t: [f32; 4],
    /// Q1 texture index.
    pub texture: i32,
    /// Texture name.
    pub name: String,
    /// Surface flags.
    pub flags: i32,
    /// Material name.
    pub material: String,
    /// Next Q2 animation frame.
    pub next: Option<u32>,
}

/// Unified legacy face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyBuildFace {
    /// Plane index.
    pub plane: usize,
    /// Back side.
    pub back: bool,
    /// First surface edge.
    pub first_edge: usize,
    /// Edge count.
    pub edge_count: usize,
    /// Texture info index.
    pub texture_info: usize,
    /// Light styles.
    pub styles: [u8; 4],
    /// Lighting offset.
    pub lighting_offset: Option<u32>,
}

/// Legacy family tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFamily {
    /// Quake 1.
    Q1,
    /// Quake 2.
    Q2,
}

/// Legacy build data.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyBuildData {
    /// Family.
    pub family: LegacyFamily,
    /// Q1 texture slots.
    pub textures: Vec<Option<BuildTexture>>,
    /// Texture infos.
    pub texture_info: Vec<LegacyTexInfo>,
    /// Faces.
    pub faces: Vec<LegacyBuildFace>,
    /// Shared brush arrays.
    pub brush: BrushMapData,
    /// Models.
    pub models: Vec<BuildModel>,
    /// Q1 lighting sample scale (3 for RGB, else 1).
    pub lighting_scale: i32,
    /// Warp subdivision size.
    pub subdivision: f32,
}

/// Owned world build data retained for image replacement.
#[derive(Debug, Clone, PartialEq)]
pub enum WorldBuildData {
    /// Q3 data.
    Q3(Q3BuildData),
    /// Legacy data.
    Legacy(LegacyBuildData),
}

fn client_error(error: crate::ClientError) -> RenderError {
    RenderError::Backend(error.to_string())
}

fn maybe_model_scale(model: Option<&ModelTransform>) -> Result<f32, RenderError> {
    match model {
        None => Ok(1.0),
        Some(model) => model_scale(model).map_err(client_error),
    }
}

fn maybe_local_point(point: Vec3, model: Option<&ModelTransform>) -> Result<Vec3, RenderError> {
    match model {
        None => Ok(point),
        Some(model) => local_point(point, model).map_err(client_error),
    }
}

fn maybe_world_point(point: Vec3, model: Option<&ModelTransform>) -> Result<Vec3, RenderError> {
    match model {
        None => Ok(point),
        Some(model) => world_point(point, model).map_err(client_error),
    }
}

fn at<'a, T>(items: &'a [T], index: usize, what: &str) -> Result<&'a T, RenderError> {
    items.get(index).ok_or_else(|| RenderError::BadBatch {
        index,
        detail: format!("Scene world {what} {index} outside {}", items.len()),
    })
}

fn to_vec3(value: [f32; 3]) -> Vec3 {
    vec3(value[0], value[1], value[2])
}

fn to_bounds(min: [f32; 3], max: [f32; 3]) -> Bounds {
    Bounds {
        min: to_vec3(min),
        max: to_vec3(max),
    }
}

fn child_to_bsp(child: ContentChild) -> BspChild {
    match child {
        ContentChild::Node(index) => BspChild::Node(index as usize),
        ContentChild::Leaf(index) => BspChild::Leaf(index as usize),
    }
}

fn mip_texture_name<'a>(texture: &'a qa_content::wad::MipTexture<'_>) -> &'a str {
    match texture {
        qa_content::wad::MipTexture::Embedded { name, .. } | qa_content::wad::MipTexture::External { name, .. } => name,
    }
}

/// Adapt a Q1 map into build data, traversal map, and visibility.
pub fn adapt_q1_build(
    map: &qa_content::bsp::Q1Map<'_>,
) -> Result<(WorldBuildData, WorldMap, WorldVisibility), RenderError> {
    let planes = map
        .planes
        .iter()
        .map(|plane| Plane {
            normal: to_vec3(plane.normal),
            distance: plane.distance,
        })
        .collect::<Vec<_>>();
    let nodes = map
        .nodes
        .iter()
        .map(|node| WorldNode {
            plane: node.plane as usize,
            children: [child_to_bsp(node.children[0]), child_to_bsp(node.children[1])],
            bounds: to_bounds(node.bounds.min, node.bounds.max),
        })
        .collect::<Vec<_>>();
    let visible_leaves = map
        .models
        .first()
        .map(|model| model.visible_leaves.max(0) as usize)
        .unwrap_or(0);
    let leaves = map
        .leaves
        .iter()
        .map(|leaf| WorldLeaf {
            bounds: to_bounds(leaf.bounds.min, leaf.bounds.max),
            cluster: -1,
            area: 0,
            contents: leaf.contents,
            first_surface: leaf.faces.first as usize,
            surface_count: leaf.faces.count as usize,
            visibility_offset: leaf.visibility_offset.map(|offset| offset as i32),
            visible_leaves,
        })
        .collect::<Vec<_>>();
    let world = WorldMap {
        kind: WorldKind::Q1,
        nodes,
        leaves,
        planes: planes.clone(),
        leaf_surfaces: map.leaf_faces.iter().map(|face| *face as usize).collect(),
    };
    let visibility = WorldVisibility::Q1 {
        data: map.visibility.to_vec(),
    };
    let textures = map
        .textures
        .iter()
        .map(|texture| match texture {
            None => None,
            Some(qa_content::wad::MipTexture::External { name, .. }) => Some(BuildTexture {
                name: name.clone(),
                embedded: None,
            }),
            Some(qa_content::wad::MipTexture::Embedded {
                name,
                width,
                height,
                levels,
            }) => Some(BuildTexture {
                name: name.clone(),
                embedded: Some(EmbeddedTexture {
                    name: name.clone(),
                    width: *width,
                    height: *height,
                    levels: [
                        levels[0].to_vec(),
                        levels[1].to_vec(),
                        levels[2].to_vec(),
                        levels[3].to_vec(),
                    ],
                }),
            }),
        })
        .collect::<Vec<_>>();
    let texture_info = map
        .texture_info
        .iter()
        .map(|info| LegacyTexInfo {
            s: info.s,
            t: info.t,
            texture: info.texture,
            name: String::new(),
            flags: info.flags,
            material: String::new(),
            next: None,
        })
        .collect::<Vec<_>>();
    let mut names = texture_info;
    for (info, texture) in names.iter_mut().zip(map.texture_info.iter()) {
        let index = texture.texture;
        if index >= 0 {
            if let Some(Some(slot)) = map.textures.get(index as usize) {
                info.name = mip_texture_name(slot).to_string();
            }
        }
    }
    let faces = map
        .faces
        .iter()
        .map(|face| LegacyBuildFace {
            plane: face.plane as usize,
            back: face.back,
            first_edge: face.edge_first.max(0) as usize,
            edge_count: face.edge_count as usize,
            texture_info: face.texture_info as usize,
            styles: face.styles,
            lighting_offset: face.lighting_offset,
        })
        .collect::<Vec<_>>();
    let brush = BrushMapData {
        texture_info: names
            .iter()
            .map(|info| BrushTextureInfo {
                projection_s: vec4(info.s[0], info.s[1], info.s[2], info.s[3]),
                projection_t: vec4(info.t[0], info.t[1], info.t[2], info.t[3]),
            })
            .collect(),
        planes: planes.clone(),
        surface_edges: map.surface_edges.clone(),
        edges: map.edges.iter().map(|edge| edge.vertices).collect(),
        vertices: map.vertices.iter().map(|vertex| to_vec3(*vertex)).collect(),
        lighting: if map.lighting.is_empty() {
            None
        } else {
            Some(BspLighting::Luminance8 {
                samples: map.lighting.to_vec(),
            })
        },
        decoupled: vec![None; map.faces.len()],
    };
    let models = map
        .models
        .iter()
        .map(|model| BuildModel {
            bounds: to_bounds(model.bounds.min, model.bounds.max),
            first: model.face_first.max(0) as usize,
            count: model.face_count.max(0) as usize,
        })
        .collect::<Vec<_>>();
    let rgb = matches!(map.format, BspFormat::Bsp2 | BspFormat::Psb2);
    Ok((
        WorldBuildData::Legacy(LegacyBuildData {
            family: LegacyFamily::Q1,
            textures,
            texture_info: names,
            faces,
            brush,
            models,
            lighting_scale: if rgb { 3 } else { 1 },
            subdivision: 128.0,
        }),
        world,
        visibility,
    ))
}

/// Adapt a decoded Q2 map into build data, traversal map, and visibility.
pub fn adapt_q2_build(
    map: &qa_content::bsp2::Q2DecodedMap<'_>,
) -> Result<(WorldBuildData, WorldMap, WorldVisibility), RenderError> {
    let planes = map
        .map
        .planes
        .iter()
        .map(|plane| Plane {
            normal: to_vec3(plane.normal),
            distance: plane.distance,
        })
        .collect::<Vec<_>>();
    let nodes = map
        .map
        .nodes
        .iter()
        .map(|node| WorldNode {
            plane: node.plane as usize,
            children: [child_to_bsp(node.children[0]), child_to_bsp(node.children[1])],
            bounds: to_bounds(node.bounds.min, node.bounds.max),
        })
        .collect::<Vec<_>>();
    let leaves = map
        .leaves
        .iter()
        .map(|leaf| WorldLeaf {
            bounds: to_bounds(leaf.bounds.min, leaf.bounds.max),
            cluster: leaf.cluster,
            area: leaf.area as i32,
            contents: leaf.merged_contents,
            first_surface: leaf.faces.first as usize,
            surface_count: leaf.faces.count as usize,
            visibility_offset: None,
            visible_leaves: 0,
        })
        .collect::<Vec<_>>();
    let world = WorldMap {
        kind: WorldKind::Q2,
        nodes,
        leaves,
        planes: planes.clone(),
        leaf_surfaces: map.map.leaf_faces.iter().map(|face| *face as usize).collect(),
    };
    let visibility = match &map.map.visibility {
        None => WorldVisibility::None,
        Some(vis) => WorldVisibility::Q2 {
            compressed: vis.compressed.to_vec(),
            clusters: vis
                .clusters
                .iter()
                .map(|cluster| super::visibility::Q2ClusterVis {
                    pvs_offset: cluster.pvs_offset,
                })
                .collect(),
        },
    };
    let texture_info = map
        .texture_info
        .iter()
        .map(|info| LegacyTexInfo {
            s: info.projection_s,
            t: info.projection_t,
            texture: -1,
            name: info.name.clone(),
            flags: info.flags,
            material: info.material.clone(),
            next: info.next,
        })
        .collect::<Vec<_>>();
    let faces = map
        .faces
        .iter()
        .map(|face| LegacyBuildFace {
            plane: face.plane as usize,
            back: face.back,
            first_edge: face.edges.first as usize,
            edge_count: face.edges.count as usize,
            texture_info: face.texture_info as usize,
            styles: face.styles,
            lighting_offset: face.lighting_offset,
        })
        .collect::<Vec<_>>();
    let mut decoupled = vec![None; faces.len()];
    if let Some(mappings) = &map.decoupled_lightmaps {
        for (slot, mapping) in decoupled.iter_mut().zip(mappings.iter()) {
            *slot = Some(crate::materials::lighting::DecoupledLightmap {
                axes: [to_vec3(mapping.axes[0]), to_vec3(mapping.axes[1])],
                offset: vec2(mapping.offset[0], mapping.offset[1]),
            });
        }
    }
    let brush = BrushMapData {
        texture_info: texture_info
            .iter()
            .map(|info| BrushTextureInfo {
                projection_s: vec4(info.s[0], info.s[1], info.s[2], info.s[3]),
                projection_t: vec4(info.t[0], info.t[1], info.t[2], info.t[3]),
            })
            .collect(),
        planes,
        surface_edges: map.map.surface_edges.clone(),
        edges: map.map.edges.iter().map(|edge| edge.vertices).collect(),
        vertices: map.map.vertices.iter().map(|vertex| to_vec3(*vertex)).collect(),
        lighting: if map.map.lighting.is_empty() {
            None
        } else {
            Some(BspLighting::Rgb8 {
                samples: map.map.lighting.to_vec(),
            })
        },
        decoupled,
    };
    let models = map
        .models
        .iter()
        .map(|model| BuildModel {
            bounds: to_bounds(model.bounds.min, model.bounds.max),
            first: model.faces.first as usize,
            count: model.faces.count as usize,
        })
        .collect::<Vec<_>>();
    Ok((
        WorldBuildData::Legacy(LegacyBuildData {
            family: LegacyFamily::Q2,
            textures: Vec::new(),
            texture_info,
            faces,
            brush,
            models,
            lighting_scale: 1,
            subdivision: 64.0,
        }),
        world,
        visibility,
    ))
}

/// Adapt a decoded Q3 world into build data, traversal map, and visibility.
pub fn adapt_q3_build(
    decoded: &qa_content::bsp3::Q3DecodedWorld,
) -> Result<(WorldBuildData, WorldMap, WorldVisibility), RenderError> {
    let map = &decoded.map;
    let planes = decoded
        .planes
        .iter()
        .map(|plane| Plane {
            normal: to_vec3(plane.normal),
            distance: plane.distance,
        })
        .collect::<Vec<_>>();
    let nodes = decoded
        .nodes
        .iter()
        .map(|node| WorldNode {
            plane: node.plane.max(0) as usize,
            children: [child_to_bsp(node.children[0]), child_to_bsp(node.children[1])],
            bounds: Bounds {
                min: vec3(
                    node.bounds.min[0] as f32,
                    node.bounds.min[1] as f32,
                    node.bounds.min[2] as f32,
                ),
                max: vec3(
                    node.bounds.max[0] as f32,
                    node.bounds.max[1] as f32,
                    node.bounds.max[2] as f32,
                ),
            },
        })
        .collect::<Vec<_>>();
    let leaves = decoded
        .leaves
        .iter()
        .map(|leaf| WorldLeaf {
            bounds: Bounds {
                min: vec3(
                    leaf.bounds.min[0] as f32,
                    leaf.bounds.min[1] as f32,
                    leaf.bounds.min[2] as f32,
                ),
                max: vec3(
                    leaf.bounds.max[0] as f32,
                    leaf.bounds.max[1] as f32,
                    leaf.bounds.max[2] as f32,
                ),
            },
            cluster: leaf.cluster,
            area: leaf.area,
            contents: 0,
            first_surface: leaf.surfaces.first as usize,
            surface_count: leaf.surfaces.count as usize,
            visibility_offset: None,
            visible_leaves: 0,
        })
        .collect::<Vec<_>>();
    let world = WorldMap {
        kind: WorldKind::Q3,
        nodes,
        leaves,
        planes,
        leaf_surfaces: map
            .leaf_surfaces
            .iter()
            .map(|surface| (*surface).max(0) as usize)
            .collect(),
    };
    let visibility = match &map.visibility {
        None => WorldVisibility::None,
        Some(vis) => WorldVisibility::Q3 {
            bits: vis.bits.clone(),
            cluster_count: vis.cluster_count.max(0) as usize,
            bytes_per_cluster: vis.bytes_per_cluster.max(0) as usize,
        },
    };
    let surfaces = decoded
        .surfaces
        .iter()
        .map(|surface| {
            let vertices = map
                .vertices
                .iter()
                .skip(surface.vertices.first as usize)
                .take(surface.vertices.count as usize)
                .map(|vertex| RawQ3Vertex {
                    position: vertex.position,
                    tex_coord: vertex.tex_coord,
                    lightmap_coord: vertex.lightmap_coord,
                    normal: vertex.normal,
                    color: vertex.color,
                })
                .collect::<Vec<_>>();
            let indices = map
                .indices
                .iter()
                .skip(surface.indices.first as usize)
                .take(surface.indices.count as usize)
                .map(|index| (*index).max(0) as u32)
                .collect::<Vec<_>>();
            let kind = match surface.kind {
                qa_content::bsp3::Q3WorldSurfaceKind::Planar => Q3SurfaceKindData::Planar,
                qa_content::bsp3::Q3WorldSurfaceKind::Triangles => Q3SurfaceKindData::Triangles,
                qa_content::bsp3::Q3WorldSurfaceKind::Flare => Q3SurfaceKindData::Flare,
                qa_content::bsp3::Q3WorldSurfaceKind::Patch { .. } => Q3SurfaceKindData::Patch,
            };
            let (width, height) = match surface.kind {
                qa_content::bsp3::Q3WorldSurfaceKind::Patch { width, height } => (width, height),
                _ => (0, 0),
            };
            Q3BuildSurface {
                kind,
                shader: surface.shader.max(0) as usize,
                fog: surface.fog,
                vertices,
                indices,
                width,
                height,
                lightmap_image: surface.lightmap.image,
                lightmap_origin: surface.lightmap.origin,
                lightmap_vectors: surface.lightmap.vectors,
            }
        })
        .collect::<Vec<_>>();
    let fog_map = FogBrushMap {
        fogs: map
            .fogs
            .iter()
            .map(|fog| (fog.brush.max(0) as usize, fog.visible_side))
            .collect(),
        brushes: map
            .brushes
            .iter()
            .map(|brush| (brush.first_side.max(0) as usize, brush.side_count.max(0) as usize))
            .collect(),
        brush_sides: map.brush_sides.iter().map(|side| side.plane.max(0) as usize).collect(),
        planes: map
            .planes
            .iter()
            .map(|plane| Plane {
                normal: to_vec3(plane.normal),
                distance: plane.distance,
            })
            .collect(),
    };
    let data = Q3BuildData {
        shaders: map
            .shaders
            .iter()
            .map(|shader| (shader.name.clone(), shader.surface_flags))
            .collect(),
        lightmaps: map.lightmaps.clone(),
        fogs: map.fogs.iter().map(|fog| (fog.shader.clone(), fog.brush)).collect(),
        fog_map,
        surfaces,
        models: decoded
            .models
            .iter()
            .map(|model| BuildModel {
                bounds: to_bounds(model.bounds.min, model.bounds.max),
                first: model.surfaces.first as usize,
                count: model.surfaces.count as usize,
            })
            .collect(),
    };
    Ok((WorldBuildData::Q3(data), world, visibility))
}

/// Scene material remap for one raw legacy surface.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneMaterialRemap {
    /// Replacement material.
    pub material: RegisteredSceneMaterial,
    /// Shader-clock offset.
    pub time_offset: f32,
}

/// Prepared world scene: surfaces, shared images, and shadow state.
pub struct WorldScene {
    build: WorldBuildData,
    map: WorldMap,
    visibility: WorldVisibility,
    models: Vec<BuildModel>,
    surfaces: Vec<WorldSurface>,
    bounds: Bounds,
    fog_image: RendererImage,
    dlight_image: RendererImage,
    noise: RendererNoise,
    fog_selections: Vec<(usize, FogVolume)>,
    material_world: ShaderWorldIdentity,
    shadow_scene: Q2ShadowScene,
    owned: Vec<RendererImage>,
    q2_sky: Vec<RendererImage>,
    fullbright_by_texture: HashMap<u32, Option<RendererImage>>,
    raw_remaps: HashMap<usize, SceneMaterialRemap>,
    closed: bool,
    image_generation: u64,
    prepared_generation: Option<u64>,
    prepared_material_revision: Option<u64>,
    shadow_material_revision: u64,
    static_light_styles: HashMap<usize, Vec<f32>>,
    static_shadow_world: Option<(usize, usize, usize, StaticShadowWorld)>,
    pending_image_operations: Vec<ImageResourceOperation>,
    options: WorldSceneOptions,
    shaders: SceneShaderRegistry,
}

fn q3_lightmap_level(bytes: &[u8], shift: u32) -> Result<ImageLevel, RenderError> {
    if bytes.len() != 128 * 128 * 3 {
        return Err(RenderError::BadDimensions {
            width: 128,
            height: 128,
            detail: "Q3 lightmap must contain 128x128 RGB samples".to_string(),
        });
    }
    let mut pixels = vec![0u8; 128 * 128 * 4];
    for pixel in 0..128 * 128 {
        let mut rgb = [
            u32::from(bytes[pixel * 3]) << shift,
            u32::from(bytes[pixel * 3 + 1]) << shift,
            u32::from(bytes[pixel * 3 + 2]) << shift,
        ];
        let maximum = rgb[0].max(rgb[1]).max(rgb[2]);
        if maximum > 255 {
            for channel in &mut rgb {
                *channel = *channel * 255 / maximum;
            }
        }
        pixels[pixel * 4..pixel * 4 + 4].copy_from_slice(&[rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, 255]);
    }
    Ok(ImageLevel {
        width: 128,
        height: 128,
        pixels,
    })
}

fn overbright_color(color: [u8; 4], shift: u32) -> [u8; 4] {
    let mut rgb = [
        u32::from(color[0]) << shift,
        u32::from(color[1]) << shift,
        u32::from(color[2]) << shift,
    ];
    let maximum = rgb[0].max(rgb[1]).max(rgb[2]);
    if maximum > 255 {
        for channel in &mut rgb {
            *channel = *channel * 255 / maximum;
        }
    }
    [rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, color[3]]
}

fn built_lightmap_image(built: &BuiltLightmap) -> Result<ImageLevel, RenderError> {
    Ok(ImageLevel {
        width: u32::try_from(built.width)
            .map_err(|_| RenderError::BadWire("Lightmap width exceeds u32".to_string()))?,
        height: u32::try_from(built.height)
            .map_err(|_| RenderError::BadWire("Lightmap height exceeds u32".to_string()))?,
        pixels: built.pixels.clone(),
    })
}

impl WorldScene {
    /// Load a Q1 map.
    pub fn load_q1(
        map: &qa_content::bsp::Q1Map<'_>,
        shaders: SceneShaderRegistry,
        options: WorldSceneOptions,
    ) -> Result<Self, RenderError> {
        let (build, traversal, visibility) = adapt_q1_build(map)?;
        Self::build(build, traversal, visibility, shaders, options)
    }

    /// Load a decoded Q2 map.
    pub fn load_q2(
        map: &qa_content::bsp2::Q2DecodedMap<'_>,
        shaders: SceneShaderRegistry,
        options: WorldSceneOptions,
    ) -> Result<Self, RenderError> {
        let (build, traversal, visibility) = adapt_q2_build(map)?;
        Self::build(build, traversal, visibility, shaders, options)
    }

    /// Load a decoded Q3 world.
    pub fn load_q3(
        decoded: &qa_content::bsp3::Q3DecodedWorld,
        shaders: SceneShaderRegistry,
        options: WorldSceneOptions,
    ) -> Result<Self, RenderError> {
        let (build, traversal, visibility) = adapt_q3_build(decoded)?;
        Self::build(build, traversal, visibility, shaders, options)
    }

    /// World bounds.
    #[must_use]
    pub const fn bounds(&self) -> Bounds {
        // Bounds is Copy (Vec3 fields); returning by value keeps callers free
        // of scene borrows during preparation.
        self.bounds
    }

    /// Prepared surfaces.
    #[must_use]
    pub fn surfaces(&self) -> &[WorldSurface] {
        &self.surfaces
    }

    /// Borrow the shader registry.
    #[must_use]
    pub fn shaders(&self) -> &SceneShaderRegistry {
        &self.shaders
    }

    /// Mutably borrow the shader registry.
    pub fn shaders_mut(&mut self) -> &mut SceneShaderRegistry {
        &mut self.shaders
    }

    /// Drain image operations queued outside view preparation.
    pub fn drain_image_operations(&mut self) -> Vec<ImageResourceOperation> {
        let mut operations = self.shaders.textures_mut().images_mut().drain_operations();
        operations.extend(self.shadow_scene.drain_operations());
        operations.append(&mut self.pending_image_operations);
        operations
    }

    fn build(
        build: WorldBuildData,
        traversal: WorldMap,
        visibility: WorldVisibility,
        shaders: SceneShaderRegistry,
        options: WorldSceneOptions,
    ) -> Result<Self, RenderError> {
        let material_world = alloc_world_identity();
        let owner = shaders.textures().images().owner().clone();
        let mut scene = Self {
            build,
            map: traversal,
            visibility,
            models: Vec::new(),
            surfaces: Vec::new(),
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
            fog_image: RendererImage {
                owner: owner.clone(),
                ordinal: u32::MAX,
                source: ImageSource::Generated { name: String::new() },
                width: 0,
                height: 0,
            },
            dlight_image: RendererImage {
                owner: owner.clone(),
                ordinal: u32::MAX,
                source: ImageSource::Generated { name: String::new() },
                width: 0,
                height: 0,
            },
            noise: RendererNoise::new(),
            fog_selections: Vec::new(),
            material_world,
            shadow_scene: Q2ShadowScene::new(owner),
            owned: Vec::new(),
            q2_sky: Vec::new(),
            fullbright_by_texture: HashMap::new(),
            raw_remaps: HashMap::new(),
            closed: false,
            image_generation: 0,
            prepared_generation: None,
            prepared_material_revision: None,
            shadow_material_revision: material_revision(),
            static_light_styles: HashMap::new(),
            static_shadow_world: None,
            pending_image_operations: Vec::new(),
            options,
            shaders,
        };
        // Build may fail partway; release anything registered so far.
        let result = scene.build_surfaces();
        if result.is_err() {
            scene.release_images();
        }
        result?;
        scene.finish_shared_images()?;
        Ok(scene)
    }

    fn generated(
        shaders: &mut SceneShaderRegistry,
        owned: &mut Vec<RendererImage>,
        name: &str,
        level: ImageLevel,
        repeat: bool,
    ) -> Result<RendererImage, RenderError> {
        let texture = shaders.textures_mut().register(
            name,
            rgba_image(level),
            TextureSampling {
                repeat,
                filter: TextureFilter::Linear,
            },
            None,
            None,
        )?;
        owned.push(texture.image.clone());
        Ok(texture.image)
    }

    fn build_surfaces(&mut self) -> Result<(), RenderError> {
        match self.build.clone() {
            WorldBuildData::Q3(data) => self.build_q3_surfaces(&data),
            WorldBuildData::Legacy(data) => self.build_legacy_surfaces(&data),
        }
    }

    fn build_q3_surfaces(&mut self, data: &Q3BuildData) -> Result<(), RenderError> {
        let shift = self.options.q3_lightmap_overbright;
        let mut lightmaps = Vec::with_capacity(data.lightmaps.len());
        for (index, bytes) in data.lightmaps.iter().enumerate() {
            lightmaps.push(Self::generated(
                &mut self.shaders,
                &mut self.owned,
                &format!("*q3-lightmap-{index}"),
                q3_lightmap_level(bytes, shift)?,
                false,
            )?);
        }
        let mut fogs = Vec::with_capacity(data.fogs.len());
        for (shader, brush) in &data.fogs {
            let material = self.shaders.register(
                shader,
                SceneShaderBinding::Unlit {
                    lightmap_index: -1,
                    mipmap: true,
                },
            )?;
            let parms = material.material.fog;
            if *brush < 0 || parms.is_none() {
                fogs.push(None);
                continue;
            }
            let parms = parms.expect("checked fog parms");
            let volume = prepare_fog_volume(&data.fog_map, fogs.len(), parms.color, parms.depth_for_opaque)
                .map_err(client_error)?;
            fogs.push(Some(volume));
        }
        for (index, volume) in fogs.iter().enumerate() {
            if let Some(volume) = volume {
                self.fog_selections.push((index, *volume));
            }
        }
        let mut patch_ordinals = Vec::new();
        let mut patches = Vec::new();
        for (index, surface) in data.surfaces.iter().enumerate() {
            let entry = at(&data.shaders, surface.shader, "shader")?;
            let (shader_name, surface_flags) = (entry.0.clone(), entry.1);
            let shift = self.options.q3_lightmap_overbright;
            let vertices = surface
                .vertices
                .iter()
                .map(|vertex| {
                    MaterialVertex::new(
                        to_vec3(vertex.position),
                        to_vec3(vertex.normal),
                        vec2(vertex.tex_coord[0], vertex.tex_coord[1]),
                        vec2(vertex.lightmap_coord[0], vertex.lightmap_coord[1]),
                        overbright_color(vertex.color, shift),
                    )
                })
                .collect::<Vec<_>>();
            let geometry = MaterialGeometry {
                vertices,
                indices: surface.indices.clone(),
            };
            let mut plane = None;
            let mut grid = None;
            if surface.kind == Q3SurfaceKindData::Patch && surface_flags & 0x80 == 0 {
                let mesh = tessellate_patch(
                    &geometry.vertices,
                    surface.width.max(0) as usize,
                    surface.height.max(0) as usize,
                    self.options.subdivisions,
                )?;
                let vectors = surface.lightmap_vectors;
                grid = Some(create_patch_grid(mesh, [to_vec3(vectors[0]), to_vec3(vectors[1])]));
                patch_ordinals.push(index);
                patches.push(grid.clone().expect("created patch grid"));
            } else if surface.kind == Q3SurfaceKindData::Planar {
                let normal = to_vec3(surface.lightmap_vectors[2]);
                let distance = geometry
                    .vertices
                    .first()
                    .map(|vertex| dot3(vertex.position, normal))
                    .unwrap_or(0.0);
                plane = Some(Plane { normal, distance });
            } else if geometry.vertices.len() >= 3 {
                let a = geometry.vertices[0].position;
                let normal = normalize3(cross3(
                    sub3(geometry.vertices[1].position, a),
                    sub3(geometry.vertices[2].position, a),
                ));
                plane = Some(Plane {
                    normal,
                    distance: dot3(a, normal),
                });
            }
            let lightmap_index =
                if surface.kind == Q3SurfaceKindData::Planar || surface.kind == Q3SurfaceKindData::Patch {
                    surface.lightmap_image
                } else {
                    -3
                };
            let lightmap = if lightmap_index >= 0 {
                lightmaps.get(lightmap_index as usize).cloned()
            } else {
                None
            };
            let material = self.shaders.register(
                &shader_name,
                SceneShaderBinding::World {
                    world: self.material_world,
                    lightmap_index,
                    lightmap: lightmap.clone(),
                    base_texture: None,
                },
            )?;
            let material = self.shaders.source_world_material(&material);
            let actual = grid.as_ref().map(patch_geometry).unwrap_or_else(|| geometry.clone());
            let bounds = geometry_bounds(&actual.vertices);
            self.surfaces.push(WorldSurface {
                shader_name: shader_name.clone(),
                base_texture: None,
                index,
                bounds,
                plane,
                geometry: actual,
                data: WorldSurfaceData::Q3 {
                    shader: material,
                    lightmap,
                    grid,
                    fog: if surface.fog < 0 {
                        None
                    } else {
                        fogs.get(surface.fog as usize).and_then(|fog| *fog)
                    },
                    fog_index: surface.fog,
                    kind: surface.kind,
                    flare: surface.kind == Q3SurfaceKindData::Flare,
                    skip: surface.kind == Q3SurfaceKindData::Patch && surface_flags & 0x80 != 0,
                },
            });
        }
        let stitched = prepare_patch_grids(patches, &mut |_| {}, &mut |_, _| {});
        for (ordinal, grid) in patch_ordinals.into_iter().zip(stitched) {
            let geometry = patch_geometry(&grid);
            let bounds = geometry_bounds(&geometry.vertices);
            if let Some(surface) = self.surfaces.get_mut(ordinal) {
                if let WorldSurfaceData::Q3 { grid: slot, .. } = &mut surface.data {
                    *slot = Some(grid);
                }
                surface.geometry = geometry;
                surface.bounds = bounds;
            }
        }
        self.models = data.models.clone();
        self.update_bounds();
        Ok(())
    }

    fn build_legacy_surfaces(&mut self, data: &LegacyBuildData) -> Result<(), RenderError> {
        let mut textures = Vec::with_capacity(match data.family {
            LegacyFamily::Q1 => data.textures.len(),
            LegacyFamily::Q2 => data.texture_info.len(),
        });
        if data.family == LegacyFamily::Q1 {
            for slot in &data.textures {
                let Some(slot) = slot else {
                    textures.push(self.shaders.textures().missing().clone());
                    continue;
                };
                let loaded = self.shaders.textures_mut().load(
                    &format!("textures/{}", slot.name),
                    &SceneTextureLoadOptions {
                        mipmap: true,
                        repeat: true,
                        family: TextureFamily::Q1,
                        usage: None,
                    },
                )?;
                if let Some(loaded) = loaded {
                    textures.push(loaded);
                    continue;
                }
                if let Some(embedded) = &slot.embedded {
                    let content = qa_content::wad::MipTexture::Embedded {
                        name: embedded.name.clone(),
                        width: embedded.width,
                        height: embedded.height,
                        levels: [
                            embedded.levels[0].as_slice(),
                            embedded.levels[1].as_slice(),
                            embedded.levels[2].as_slice(),
                            embedded.levels[3].as_slice(),
                        ],
                    };
                    if let Some(texture) = self.shaders.textures_mut().q1_embedded(&content)? {
                        textures.push(texture);
                        continue;
                    }
                }
                textures.push(self.shaders.textures().missing().clone());
            }
        } else {
            for info in &data.texture_info {
                let loaded = self.shaders.textures_mut().load(
                    &format!("textures/{}", info.name),
                    &SceneTextureLoadOptions {
                        mipmap: true,
                        repeat: true,
                        family: TextureFamily::Q2,
                        usage: Some(super::image_policy::ImageUsage::Wall),
                    },
                )?;
                textures.push(loaded.unwrap_or_else(|| self.shaders.textures().missing().clone()));
            }
        }
        for texture in &textures {
            self.fullbright_by_texture
                .insert(texture.image.ordinal, texture.fullbright.clone());
        }
        let pairs = textures
            .iter()
            .map(|texture| (texture.name.as_str(), texture.image.ordinal))
            .collect::<Vec<_>>();
        let mut sky_layers: HashMap<u32, Q1SkyLayers> = HashMap::new();
        for (index, face) in data.faces.iter().enumerate() {
            let info = at(&data.texture_info, face.texture_info, "texture info")?.clone();
            let texture_index = if data.family == LegacyFamily::Q1 {
                if info.texture < 0 {
                    textures.len()
                } else {
                    info.texture as usize
                }
            } else {
                face.texture_info
            };
            let texture = at(
                &textures,
                texture_index.min(textures.len().saturating_sub(1)),
                "texture",
            )?
            .clone();
            let name = if data.family == LegacyFamily::Q1 && !info.name.is_empty() {
                info.name.clone()
            } else {
                texture.name.clone()
            };
            let short_name = name.rsplit('/').next().unwrap_or(&name).to_string();
            let warp = if data.family == LegacyFamily::Q1 {
                short_name.starts_with('*')
            } else {
                info.flags & 8 != 0
            };
            let sky = if data.family == LegacyFamily::Q1 {
                short_name.starts_with("sky")
            } else {
                info.flags & 4 != 0
            };
            let original = if data.family == LegacyFamily::Q1 && info.texture >= 0 {
                data.textures
                    .get(info.texture as usize)
                    .and_then(|slot| slot.as_ref())
                    .and_then(|slot| slot.embedded.as_ref())
            } else {
                None
            };
            let prepared = prepare_brush_face(
                &data.brush,
                &BrushFace {
                    texture_info: face.texture_info,
                    plane: face.plane,
                    back: face.back,
                    first_edge: face.first_edge,
                    edge_count: face.edge_count,
                    lighting_offset: face.lighting_offset.map(|offset| offset as i32),
                    styles: face.styles.to_vec(),
                },
                index,
                vec2(
                    original
                        .map(|texture| texture.width as f32)
                        .unwrap_or(texture.width as f32),
                    original
                        .map(|texture| texture.height as f32)
                        .unwrap_or(texture.height as f32),
                ),
                warp,
                data.subdivision,
                data.lighting_scale,
            )?;
            let mut lightmap = None;
            if !sky && !warp && prepared.lightmap.lighting.is_some() {
                let built = if data.family == LegacyFamily::Q1 {
                    build_q1_lightmap(
                        &prepared.lightmap,
                        &vec![256; 256],
                        Some(self.options.q1_lightmap_encoding),
                        false,
                        &[],
                    )
                    .map_err(client_error)?
                } else {
                    build_q2_lightmap(
                        &prepared.lightmap,
                        &default_q2_styles(),
                        self.options.q2_light_modulate,
                        Q2Mono::Color,
                        &[],
                    )
                    .map_err(client_error)?
                };
                let kind = if data.family == LegacyFamily::Q1 {
                    "q1-bsp"
                } else {
                    "q2-bsp"
                };
                let image = Self::generated(
                    &mut self.shaders,
                    &mut self.owned,
                    &format!("*{kind}-lightmap-{index}"),
                    built_lightmap_image(&built)?,
                    false,
                )?;
                let direct = Self::generated(
                    &mut self.shaders,
                    &mut self.owned,
                    &format!("*{kind}-direct-lightmap-{index}"),
                    built_lightmap_image(&direct_lightmap_pixels(&built).map_err(client_error)?)?,
                    false,
                )?;
                lightmap = Some(LegacyLightmap {
                    face: prepared.lightmap.clone(),
                    image,
                    direct,
                    encoding: if data.family == LegacyFamily::Q1 {
                        self.options.q1_lightmap_encoding
                    } else {
                        Q1LightmapEncoding::Rgb
                    },
                });
            }
            let (material, q1_sky) = if data.family == LegacyFamily::Q1 {
                let (animation, alternate) = q1_texture_animations(&short_name, &pairs).map_err(client_error)?;
                let material = create_q1_material(
                    &short_name,
                    texture.image.ordinal,
                    lightmap.as_ref().map(|lightmap| lightmap.image.ordinal),
                    false,
                    if warp { self.options.q1_water_alpha } else { 1.0 },
                    animation,
                    alternate,
                );
                let mut q1_sky = None;
                if q1_surface_kind(&short_name) == Q1Surface::Sky {
                    if let RenderImage::Indexed8 { levels, palette, .. } = &texture.content {
                        if let Some(base) = levels.first() {
                            q1_sky = sky_layers.get(&texture.image.ordinal).cloned();
                            if q1_sky.is_none() {
                                let layers = split_q1_sky_texture(&IndexedImage {
                                    width: base.width as usize,
                                    height: base.height as usize,
                                    pixels: base.pixels.clone(),
                                    palette: palette.colors.clone(),
                                })
                                .map_err(client_error)?;
                                let solid = Self::generated(
                                    &mut self.shaders,
                                    &mut self.owned,
                                    &format!("{short_name}:solid"),
                                    ImageLevel {
                                        width: layers.solid_size.0 as u32,
                                        height: layers.solid_size.1 as u32,
                                        pixels: layers.solid,
                                    },
                                    true,
                                )?;
                                let overlay = Self::generated(
                                    &mut self.shaders,
                                    &mut self.owned,
                                    &format!("{short_name}:overlay"),
                                    ImageLevel {
                                        width: layers.overlay_size.0 as u32,
                                        height: layers.overlay_size.1 as u32,
                                        pixels: layers.overlay,
                                    },
                                    true,
                                )?;
                                q1_sky = Some(Q1SkyLayers { solid, overlay });
                                sky_layers.insert(texture.image.ordinal, q1_sky.clone().expect("split sky layers"));
                            }
                        }
                    }
                }
                (LegacyMaterial::Q1(material), q1_sky)
            } else {
                let mut frames = Vec::new();
                let mut visited = HashSet::new();
                let mut next = Some(face.texture_info);
                while let Some(current) = next {
                    if !visited.insert(current) {
                        break;
                    }
                    let frame = at(&textures, current.min(textures.len().saturating_sub(1)), "texture")?;
                    frames.push(frame.image.ordinal);
                    if frames.len() >= 8 {
                        break;
                    }
                    next = at(&data.texture_info, current, "texture info")?
                        .next
                        .map(|next| next as usize);
                }
                if frames.is_empty() {
                    frames.push(texture.image.ordinal);
                }
                let material = create_q2_material(
                    &frames,
                    lightmap.as_ref().map(|lightmap| lightmap.image.ordinal),
                    false,
                    info.flags as u32,
                )
                .map_err(client_error)?;
                (
                    LegacyMaterial::Q2 {
                        name: short_name.clone(),
                        material,
                    },
                    None,
                )
            };
            let shader_name = format!("textures/{short_name}");
            let shader = if self.shaders.has_authored(&shader_name) {
                Some(self.shaders.register(
                    &shader_name,
                    SceneShaderBinding::World {
                        world: self.material_world,
                        lightmap_index: if lightmap.is_none() { -1 } else { index as i32 },
                        lightmap: lightmap.as_ref().map(|lightmap| lightmap.image.clone()),
                        base_texture: Some(texture.clone()),
                    },
                )?)
            } else {
                None
            };
            self.surfaces.push(WorldSurface {
                shader_name,
                base_texture: Some(texture.clone()),
                index,
                bounds: geometry_bounds(&prepared.geometry.vertices),
                plane: Some(prepared.plane),
                geometry: prepared.geometry,
                data: WorldSurfaceData::Legacy {
                    shader,
                    material,
                    fullbright: texture.fullbright.clone(),
                    lightmap,
                    q1_sky,
                },
            });
        }
        if data.family == LegacyFamily::Q2 {
            if let Some(sky_name) = self.options.q2_sky_name.clone() {
                let mut sides = Vec::with_capacity(6);
                for suffix in SKY_FACE_SUFFIXES {
                    let loaded = self.shaders.textures_mut().load(
                        &format!("env/{sky_name}{suffix}"),
                        &SceneTextureLoadOptions {
                            mipmap: false,
                            repeat: false,
                            family: TextureFamily::Q2,
                            usage: Some(super::image_policy::ImageUsage::Sky),
                        },
                    )?;
                    sides.push(
                        loaded
                            .map(|texture| texture.image)
                            .unwrap_or_else(|| self.shaders.textures().missing().image.clone()),
                    );
                }
                self.q2_sky = sides;
            }
        }
        self.models = data.models.clone();
        self.update_bounds();
        Ok(())
    }

    fn update_bounds(&mut self) {
        if let Some(model) = self.models.first() {
            self.bounds = model.bounds;
            return;
        }
        let mut bounds: Option<Bounds> = None;
        for surface in &self.surfaces {
            bounds = Some(match bounds {
                None => surface.bounds,
                Some(current) => Bounds {
                    min: vec3(
                        current.min.x.min(surface.bounds.min.x),
                        current.min.y.min(surface.bounds.min.y),
                        current.min.z.min(surface.bounds.min.z),
                    ),
                    max: vec3(
                        current.max.x.max(surface.bounds.max.x),
                        current.max.y.max(surface.bounds.max.y),
                        current.max.z.max(surface.bounds.max.z),
                    ),
                },
            });
        }
        self.bounds = bounds.unwrap_or(Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(0.0, 0.0, 0.0),
        });
    }

    fn finish_shared_images(&mut self) -> Result<(), RenderError> {
        let fog = create_fog_texture();
        self.fog_image = Self::generated(
            &mut self.shaders,
            &mut self.owned,
            "*fog",
            ImageLevel {
                width: fog.width,
                height: fog.height,
                pixels: fog.pixels,
            },
            false,
        )?;
        let mut dlight = vec![0u8; 16 * 16 * 4];
        for x in 0..16 {
            for y in 0..16 {
                let distance = (7.5 - x as f32) * (7.5 - x as f32) + (7.5 - y as f32) * (7.5 - y as f32);
                let mut brightness = (4000.0 / distance) as i32;
                if brightness > 255 {
                    brightness = 255;
                } else if brightness < 75 {
                    brightness = 0;
                }
                let brightness = brightness as u8;
                dlight[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4]
                    .copy_from_slice(&[brightness, brightness, brightness, 255]);
            }
        }
        self.dlight_image = Self::generated(
            &mut self.shaders,
            &mut self.owned,
            "*dlight",
            ImageLevel {
                width: 16,
                height: 16,
                pixels: dlight,
            },
            false,
        )?;
        Ok(())
    }

    fn release_images(&mut self) {
        self.static_shadow_world = None;
        self.static_light_styles.clear();
        self.shadow_scene.close();
        let owned = std::mem::take(&mut self.owned);
        for image in &owned {
            let _ = self.shaders.textures_mut().images_mut().release(image);
        }
    }

    /// Close the scene, releasing owned images.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.image_generation += 1;
        self.release_images();
    }

    /// Whether the scene is closed.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.closed
    }
}

fn default_q2_styles() -> Vec<Q2LightStyle> {
    vec![
        Q2LightStyle {
            rgb: vec3(1.0, 1.0, 1.0),
            white: 3.0,
        };
        256
    ]
}

fn patch_geometry(grid: &PatchGrid) -> MaterialGeometry {
    MaterialGeometry {
        vertices: grid.mesh.vertices.clone(),
        indices: grid.mesh.indices.clone(),
    }
}

/// Owned material-context inputs shared by every surface in one prepare call.
#[derive(Debug, Clone)]
struct DrawContextData {
    time: f32,
    refdef_time: f32,
    identity_light: f32,
    entity_rgba: [u8; 4],
    lighting: Option<EntityLighting>,
    view_origin: Vec3,
    local_view_origin: Vec3,
    deform_view: DeformView,
    projection_shadow: Option<ProjectionShadowContext>,
    render_text: Vec<String>,
    q1_fog: Option<Q1FogInput>,
    mirror: bool,
}

fn material_context_data(
    camera: &SceneCamera,
    model: Option<&ModelTransform>,
    input: &WorldViewInput,
    white_ordinal: u32,
) -> Result<DrawContextData, RenderError> {
    maybe_model_scale(model)?;
    let time = input.time.as_seconds() as f32;
    Ok(DrawContextData {
        time,
        refdef_time: (time * 1000.0).trunc(),
        identity_light: input.identity_light,
        entity_rgba: input.entity_rgba,
        lighting: input.lighting,
        view_origin: camera.origin,
        local_view_origin: maybe_local_point(camera.origin, model)?,
        deform_view: DeformView {
            axis: camera.axis,
            mirror: matches!(camera.clip, crate::view::CameraClip::Portal { mirror: true, .. }),
            entity_axis: model.map(|model| model.axis),
            non_normalized_axis: None,
        },
        projection_shadow: input.projection_shadow,
        render_text: input.render_text.clone(),
        q1_fog: input.q1_fog.map(|fog| Q1FogInput {
            density: fog.density,
            color: fog.color,
            texture: TextureRef::BindImage(white_ordinal),
        }),
        mirror: matches!(camera.clip, crate::view::CameraClip::Portal { mirror: true, .. }),
    })
}

fn draw_context<'a>(
    data: &'a DrawContextData,
    noise: &'a RendererNoise,
    project: &'a dyn Fn(Vec3) -> Vec4,
    fog: Option<FogVolumeInput<'a>>,
    dynamic_light_batches: DynamicLightBatches<'a>,
    time_offset: f32,
) -> MaterialDrawContext<'a> {
    MaterialDrawContext {
        identity_light: data.identity_light,
        time: data.time,
        time_offset,
        refdef_time: data.refdef_time,
        view_origin: data.view_origin,
        local_view_origin: data.local_view_origin,
        shader_tex_coord: vec2(0.0, 0.0),
        deform_view: data.deform_view,
        projection_shadow: data.projection_shadow,
        render_text: data.render_text.clone(),
        depth_range: [0.0, 1.0],
        polygon_offset: Some(crate::materials::state::PolygonOffset {
            factor: -1.0,
            units: -2.0,
        }),
        noise,
        project,
        fog,
        q1_fog: data.q1_fog,
        dynamic_light_batches,
        dynamic_lights: None,
        lighting: data.lighting,
        entity_rgba: data.entity_rgba,
    }
}

/// Whether a world-space box is fully outside the frustum.
fn local_box_culled(
    bounds: &Bounds,
    camera: &SceneCamera,
    model: Option<&ModelTransform>,
) -> Result<bool, RenderError> {
    let corners = [
        Vec3 {
            x: bounds.min.x,
            y: bounds.min.y,
            z: bounds.min.z,
        },
        Vec3 {
            x: bounds.max.x,
            y: bounds.min.y,
            z: bounds.min.z,
        },
        Vec3 {
            x: bounds.min.x,
            y: bounds.max.y,
            z: bounds.min.z,
        },
        Vec3 {
            x: bounds.max.x,
            y: bounds.max.y,
            z: bounds.min.z,
        },
        Vec3 {
            x: bounds.min.x,
            y: bounds.min.y,
            z: bounds.max.z,
        },
        Vec3 {
            x: bounds.max.x,
            y: bounds.min.y,
            z: bounds.max.z,
        },
        Vec3 {
            x: bounds.min.x,
            y: bounds.max.y,
            z: bounds.max.z,
        },
        Vec3 {
            x: bounds.max.x,
            y: bounds.max.y,
            z: bounds.max.z,
        },
    ];
    for plane in camera_frustum(camera) {
        let mut outside = true;
        for corner in corners {
            let local = maybe_local_point(corner, model)?;
            if dot3(local, plane.normal) - plane.distance >= 0.0 {
                outside = false;
                break;
            }
        }
        if outside {
            return Ok(true);
        }
    }
    Ok(false)
}

impl WorldScene {
    /// Admit a surface index into an admission; false when already present.
    fn admit_surface(admission: &mut WorldSurfaceAdmission, index: usize) -> bool {
        admission.submitted_surfaces.insert(index)
    }

    fn source_surface_order(
        surface: &WorldSurface,
        admission: &WorldSurfaceAdmission,
        entity: SourceEntityOrder,
        mask: u32,
    ) -> SourceSurfaceOrder {
        SourceSurfaceOrder {
            view: admission.view.clone(),
            entity,
            surface: surface.index as u32,
            fog: match &surface.data {
                WorldSurfaceData::Q3 { fog_index, .. } => (*fog_index).max(0) as u32,
                WorldSurfaceData::Legacy { .. } => 0,
            },
            dlight: u32::from(mask & 1 != 0),
        }
    }

    fn frontend_light_mask(surface: &WorldSurface, incoming: u32, lights: &[DynamicLight]) -> u32 {
        match &surface.data {
            WorldSurfaceData::Q3 { kind, .. } => match (kind, surface.plane) {
                (Q3SurfaceKindData::Planar | Q3SurfaceKindData::Triangles, Some(plane)) => {
                    face_dlight_mask(lights, incoming, &plane)
                }
                _ => grid_dlight_mask(lights, incoming, &surface.bounds),
            },
            WorldSurfaceData::Legacy { .. } => incoming,
        }
    }

    /// Q3 surface cull plus its narrowed light mask.
    fn surface_light_mask(
        surface: &WorldSurface,
        model: Option<&ModelTransform>,
        data: &DrawContextData,
        lights: &[DynamicLight],
        incoming: u32,
        frustum: &[Plane],
    ) -> Result<(bool, u32), RenderError> {
        let flare = matches!(&surface.data, WorldSurfaceData::Q3 { flare: true, .. });
        if flare {
            return Ok((false, incoming));
        }
        let grid = matches!(&surface.data, WorldSurfaceData::Q3 { grid: Some(_), .. });
        if model.is_none() && !grid {
            if let Some(plane) = surface.plane {
                if dot3(data.local_view_origin, plane.normal) - plane.distance < -0.1 {
                    return Ok((true, incoming));
                }
            }
        }
        if grid {
            return Ok((false, Self::frontend_light_mask(surface, incoming, lights)));
        }
        let bounds = Bounds {
            min: maybe_world_point(surface.bounds.min, model)?,
            max: maybe_world_point(surface.bounds.max, model)?,
        };
        if !bounds_in_frustum(&bounds, frustum) {
            return Ok((true, incoming));
        }
        Ok((false, Self::frontend_light_mask(surface, incoming, lights)))
    }

    /// Resolve the remap for one surface: raw remaps for unshaded legacy
    /// surfaces, global remaps (registered on demand) otherwise.
    fn remap(&mut self, index: usize) -> Result<Option<(RegisteredSceneMaterial, f32)>, RenderError> {
        let surface = at(&self.surfaces, index, "surface")?.clone();
        if surface.shader().is_none() {
            return Ok(self
                .raw_remaps
                .get(&index)
                .map(|remap| (remap.material.clone(), remap.time_offset)));
        }
        let Some(remap) = current_remap(&surface.shader_name) else {
            return Ok(None);
        };
        let binding = self.remap_binding(&surface);
        let material = self.shaders.register(&remap.material, binding)?;
        Ok(Some((material, remap.time_offset)))
    }

    fn remap_binding(&self, surface: &WorldSurface) -> SceneShaderBinding {
        let (lightmap_index, lightmap) = match &surface.data {
            WorldSurfaceData::Q3 {
                lightmap,
                fog_index,
                kind,
                ..
            } => (
                match lightmap {
                    None => -3,
                    Some(_) => match kind {
                        Q3SurfaceKindData::Patch | Q3SurfaceKindData::Planar => *fog_index,
                        _ => -3,
                    },
                },
                lightmap.clone(),
            ),
            WorldSurfaceData::Legacy { lightmap, .. } => (
                lightmap.as_ref().map(|_| surface.index as i32).unwrap_or(-1),
                lightmap.as_ref().map(|lightmap| lightmap.image.clone()),
            ),
        };
        SceneShaderBinding::World {
            world: self.material_world,
            lightmap_index,
            lightmap,
            base_texture: surface.base_texture.clone(),
        }
    }

    fn raw_remap_bindings(&self, name: &str) -> Vec<(usize, SceneShaderBinding)> {
        self.surfaces
            .iter()
            .filter(|surface| surface.shader().is_none() && surface.shader_name == name)
            .map(|surface| (surface.index, self.remap_binding(surface)))
            .collect()
    }

    /// Publish replacement materials for every raw surface under a name.
    pub fn publish_raw_remap(
        &mut self,
        name: &str,
        materials: Vec<RegisteredSceneMaterial>,
        time_offset: f32,
    ) -> Result<(), RenderError> {
        let bindings = self.raw_remap_bindings(name);
        if bindings.len() != materials.len() {
            return Err(RenderError::Backend(format!(
                "Remap for {name} prepared {} of {} raw surfaces",
                materials.len(),
                bindings.len()
            )));
        }
        for ((index, _), material) in bindings.into_iter().zip(materials) {
            self.raw_remaps
                .insert(index, SceneMaterialRemap { material, time_offset });
        }
        Ok(())
    }

    /// Remap a shader name to a replacement (equal names clear the remap).
    pub fn remap_shader(&mut self, original: &str, replacement: &str, time_offset: f32) -> Result<(), RenderError> {
        if crate::materials::material::normalize_shader_name(original)
            == crate::materials::material::normalize_shader_name(replacement)
        {
            remove_remap(original);
            let stale: Vec<usize> = self
                .surfaces
                .iter()
                .filter(|surface| surface.shader_name == original)
                .map(|surface| surface.index)
                .collect();
            for index in stale {
                self.raw_remaps.remove(&index);
            }
            return Ok(());
        }
        publish_remap(
            original,
            MaterialRemap {
                material: replacement.to_string(),
                time_offset,
            },
        );
        let bindings = self.raw_remap_bindings(original);
        let mut materials = Vec::with_capacity(bindings.len());
        for (_, binding) in &bindings {
            materials.push(self.shaders.register(replacement, binding.clone())?);
        }
        self.publish_raw_remap(original, materials, time_offset)
    }

    /// Prepare one model for a view.
    pub fn prepare_model(
        &mut self,
        model_index: usize,
        transform: &ModelTransform,
        input: &mut WorldViewInput,
        entity: Option<SourceEntityOrder>,
    ) -> Result<Vec<SceneOperation>, RenderError> {
        let model = *at(&self.models, model_index, "model")?;
        let kind = self.map.kind;
        let has_source = input.source.is_some() && entity.is_some();
        if kind == WorldKind::Q3 && local_box_culled(&model.bounds, &input.camera, Some(transform))? {
            return Ok(Vec::new());
        }
        let white = self.shaders.textures().white().image.ordinal;
        let camera = input.camera;
        let model_param = if model_index == 0 { None } else { Some(*transform) };
        let data = material_context_data(&camera, model_param.as_ref(), input, white)?;
        let lights = if kind == WorldKind::Q3 {
            transform_dlights(&input.visibility.q3_lights, transform.origin, &transform.axis)
        } else {
            Vec::new()
        };
        let incoming = if kind == WorldKind::Q3 && model_index != 0 {
            bmodel_dlight_mask(&lights, &model.bounds)
        } else if lights.is_empty() {
            0
        } else {
            u32::MAX >> (32 - lights.len())
        };
        let frustum = camera_frustum(&camera);
        let last = model.first + model.count;
        let mut operations = Vec::new();
        for index in model.first..last {
            let surface = at(&self.surfaces, index, "surface")?.clone();
            let mut mask = incoming;
            if kind == WorldKind::Q3 {
                if surface.shader().is_none() {
                    continue;
                }
                let (culled, narrowed) =
                    Self::surface_light_mask(&surface, model_param.as_ref(), &data, &lights, incoming, &frustum)?;
                if culled {
                    continue;
                }
                mask = narrowed;
            }
            let order = if has_source {
                if !Self::admit_surface(input.source.as_mut().expect("checked source admission"), index) {
                    continue;
                }
                Some(Self::source_surface_order(
                    &surface,
                    input.source.as_ref().expect("checked source admission"),
                    entity.expect("checked entity order"),
                    mask,
                ))
            } else {
                None
            };
            operations.extend(self.surface_operations(
                index,
                input,
                &data,
                model_param.as_ref(),
                (mask, lights.clone()),
                order,
            )?);
        }
        Ok(operations)
    }

    /// Prepare world-model operations for a view.
    pub fn prepare_world_operations(
        &mut self,
        input: &mut WorldViewInput,
        visibility: Option<VisibleWorld>,
    ) -> Result<Vec<SceneOperation>, RenderError> {
        if let Some(cached) = input.source.as_ref().and_then(|source| source.world_operations.clone()) {
            return Ok(cached);
        }
        let visible = match visibility {
            Some(visible) => visible,
            None => visible_world(&self.map, &self.visibility, &input.camera, &input.visibility)?,
        };
        let kind = self.map.kind;
        let mut indexes: Vec<usize> = if kind == WorldKind::Q3 {
            visible.surfaces.clone()
        } else {
            let world = *at(&self.models, 0, "model")?;
            visible
                .surfaces
                .iter()
                .copied()
                .filter(|index| *index >= world.first && *index < world.first + world.count)
                .collect()
        };
        if indexes.iter().any(|index| {
            self.surfaces
                .get(*index)
                .is_some_and(|surface| surface.shader().is_none())
        }) {
            let mut orders = Vec::with_capacity(indexes.len());
            for index in &indexes {
                orders.push((*index, self.surface_order(*index)?));
            }
            orders.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            indexes = orders.into_iter().map(|(index, _)| index).collect();
        }
        let white = self.shaders.textures().white().image.ordinal;
        let camera = input.camera;
        let data = material_context_data(&camera, None, input, white)?;
        let lights = input.visibility.q3_lights.clone();
        let frustum = camera_frustum(&camera);
        let mut operations = Vec::new();
        for index in indexes {
            let surface = at(&self.surfaces, index, "surface")?.clone();
            if kind == WorldKind::Q3 {
                if surface.shader().is_none() {
                    continue;
                }
                let mask = *visible.surface_dlight_masks.get(&index).ok_or_else(|| {
                    RenderError::Backend(format!("Surface {} is missing its initial light mask", surface.index))
                })?;
                let (culled, narrowed) = Self::surface_light_mask(&surface, None, &data, &lights, mask, &frustum)?;
                if culled {
                    continue;
                }
                let order = match input.source.as_mut() {
                    None => None,
                    Some(admission) => {
                        if !Self::admit_surface(admission, index) {
                            continue;
                        }
                        Some(Self::source_surface_order(
                            &surface,
                            admission,
                            SourceEntityOrder::World,
                            narrowed,
                        ))
                    }
                };
                operations.extend(self.surface_operations(
                    index,
                    input,
                    &data,
                    None,
                    (narrowed, lights.clone()),
                    order,
                )?);
            } else {
                if !bounds_in_frustum(&surface.bounds, &frustum) {
                    continue;
                }
                let order = match input.source.as_mut() {
                    None => None,
                    Some(admission) => {
                        if !Self::admit_surface(admission, index) {
                            continue;
                        }
                        Some(Self::source_surface_order(
                            &surface,
                            admission,
                            SourceEntityOrder::World,
                            0,
                        ))
                    }
                };
                operations.extend(self.surface_operations(index, input, &data, None, (0, Vec::new()), order)?);
            }
        }
        if let Some(source) = input.source.as_mut() {
            source.world_operations = Some(operations.clone());
        }
        Ok(operations)
    }

    fn surface_order(&mut self, index: usize) -> Result<i32, RenderError> {
        let surface = at(&self.surfaces, index, "surface")?.clone();
        if surface.shader().is_none() {
            let sky = match &surface.data {
                WorldSurfaceData::Legacy { q1_sky: Some(_), .. } => true,
                WorldSurfaceData::Legacy {
                    material: LegacyMaterial::Q2 { material, .. },
                    ..
                } => material.surface_flags & 4 != 0,
                _ => false,
            };
            if sky {
                return Ok(2);
            }
            let alpha = match &surface.data {
                WorldSurfaceData::Legacy { material, .. } => match material {
                    LegacyMaterial::Q1(material) => material.alpha,
                    LegacyMaterial::Q2 { material, .. } => material.alpha,
                },
                _ => 1.0,
            };
            return Ok(if alpha < 1.0 { 9 } else { 3 });
        }
        let resolved = self.remap(index)?;
        Ok(resolved
            .map(|(material, _)| material.finished.sort)
            .unwrap_or_else(|| surface.shader().map(|shader| shader.finished.sort).unwrap_or(3)))
    }

    /// Prepare one ordered world view.
    pub fn prepare_view(&mut self, input: &mut WorldViewInput) -> Result<PreparedWorldView, RenderError> {
        let mut operations = if input.no_world_model {
            Vec::new()
        } else {
            let visibility = visible_world(&self.map, &self.visibility, &input.camera, &input.visibility)?;
            self.prepare_world_operations(input, Some(visibility))?
        };
        for inline in input.inline_models.clone() {
            let mut child = input.clone();
            child.animation_frame = inline.animation_frame.or(child.animation_frame);
            child.alternate_animation = inline.alternate_animation.unwrap_or(child.alternate_animation);
            child.entity_rgba = inline.entity_rgba.unwrap_or(child.entity_rgba);
            operations.extend(self.prepare_model(inline.model, &inline.transform, &mut child, None)?);
        }
        operations.extend(input.operations.clone());
        let mut sky_drawn = false;
        for index in 0..self.surfaces.len() {
            let surface = self.surfaces[index].clone();
            if let Some(shader) = surface.shader().cloned() {
                let resolved = self.remap(index)?;
                let selected = resolved.map(|(material, _)| material).unwrap_or(shader);
                if selected.finished.iterator.kind == MaterialIteratorKind::Sky {
                    sky_drawn = true;
                    break;
                }
            } else if let WorldSurfaceData::Legacy {
                material: LegacyMaterial::Q2 { material, .. },
                ..
            } = &surface.data
            {
                if material.surface_flags & 4 != 0 {
                    sky_drawn = true;
                    break;
                }
            }
        }
        if let Some(fog) = input.q2_fog {
            if !input.no_world_model {
                operations.push(SceneOperation::Operation(RenderOperation::Q2Fog(Q2FogOperation {
                    camera: RenderCamera {
                        origin: input.camera.origin,
                        axis: input.camera.axis,
                        projection: input.camera.projection,
                        viewport: crate::render::types::Rect {
                            x: input.camera.viewport.x as f32,
                            y: input.camera.viewport.y as f32,
                            width: input.camera.viewport.width as f32,
                            height: input.camera.viewport.height as f32,
                        },
                        clip: match input.camera.clip {
                            crate::view::CameraClip::None => ViewClip::None,
                            crate::view::CameraClip::Portal { plane, mirror } => ViewClip::Portal { plane, mirror },
                        },
                    },
                    fog,
                    far_depth: 1.0 - 1e-6,
                    sky_drawn,
                })));
            }
        }
        let finished = finish_scene_operations(operations)?;
        let fog = match (input.q1_fog, input.no_world_model) {
            (Some(fog), false) => SceneFog::Q1 {
                color: fog.color,
                density: fog.density,
                sky_factor: fog.sky_factor,
            },
            _ => SceneFog::None,
        };
        let view = RenderView {
            state: RenderViewState {
                viewport: crate::render::types::Rect {
                    x: input.camera.viewport.x as f32,
                    y: input.camera.viewport.y as f32,
                    width: input.camera.viewport.width as f32,
                    height: input.camera.viewport.height as f32,
                },
                clear: Some(input.clear.unwrap_or(ViewClear {
                    depth: 1.0,
                    color: None,
                    stencil: false,
                })),
                clip_plane: portal_clip_plane(&input.camera),
            },
            target: input.target.clone(),
            time: input.time,
            before_view: input.before_view.clone(),
            operations: fog_scene_operations(finished, &fog),
        };
        Ok(PreparedWorldView {
            image_operations: self.drain_image_operations(),
            view,
        })
    }

    /// Prepare the main view plus one child view per visible portal surface.
    pub fn prepare_views(
        &mut self,
        input: &mut WorldViewInput,
        portals: &[PortalEntity],
    ) -> Result<Vec<PreparedWorldView>, RenderError> {
        let mut views = vec![self.prepare_view(input)?];
        if self.map.kind != WorldKind::Q3 || input.no_world_model {
            return Ok(views);
        }
        let milliseconds = match input.time {
            SourceTime::Seconds(seconds) => seconds * 1000.0,
            SourceTime::Milliseconds(millis) => millis,
        } as f32;
        let candidates: Vec<(usize, Plane, MaterialGeometry)> = self
            .surfaces
            .iter()
            .filter(|surface| {
                matches!(&surface.data, WorldSurfaceData::Q3 { .. })
                    && surface.plane.is_some()
                    && surface.shader().is_some_and(|shader| shader.finished.sort == 1)
            })
            .map(|surface| {
                (
                    surface.index,
                    surface.plane.expect("filtered portal plane"),
                    surface.geometry.clone(),
                )
            })
            .collect();
        for (index, plane, geometry) in candidates {
            let surface = at(&self.surfaces, index, "surface")?.clone();
            let shader = surface.shader().cloned().expect("filtered portal shader");
            let resolved = self.remap(index)?;
            let selected = resolved.map(|(material, _)| material).unwrap_or(shader);
            if selected.material.portal_range <= 0.0 {
                continue;
            }
            let child = portal_camera(&plane, portals, &input.camera, milliseconds, None)?;
            let Some(child) = child else { continue };
            if world_point_leaf(&self.map, child.pvs_origin)? < 0 {
                continue;
            }
            if portal_surface_offscreen(&geometry, &child.camera, selected.material.portal_range, child.mirror)? {
                continue;
            }
            let mut nested = input.clone();
            nested.camera = child.camera;
            nested.visibility.pvs_origin = Some(child.pvs_origin);
            nested.source = input
                .source
                .as_ref()
                .map(|source| create_world_surface_admission(create_source_scene_order(source.view.ranks().to_vec())));
            views.push(self.prepare_view(&mut nested)?);
        }
        Ok(views)
    }

    /// Prepare Q2 shadow lighting, caching the static world across frames.
    pub fn prepare_shadows(
        &mut self,
        lights: &[SceneLight],
        input: &WorldViewInput,
        casters: &[ShadowCaster],
        options: &ShadowAtlasOptions,
    ) -> Result<PreparedShadows, RenderError> {
        let revision = material_revision();
        if revision != self.shadow_material_revision {
            self.shadow_material_revision = revision;
            self.static_shadow_world = None;
        }
        let time = input.time.as_seconds() as f32;
        let data = ShadowContextData {
            time,
            refdef_time: (time * 1000.0).trunc(),
            render_text: input.render_text.clone(),
            deform_view: DeformView {
                axis: input.camera.axis,
                mirror: false,
                entity_axis: None,
                non_normalized_axis: None,
            },
            projection_shadow: input.projection_shadow,
        };
        let mut all_casters = casters.to_vec();
        for inline in &input.inline_models {
            if !inline.casts_shadow {
                continue;
            }
            all_casters.push(self.shadow_model(inline.model, &inline.transform, &data)?);
        }
        let count = self.surfaces.len();
        let world_meshes;
        let mut static_world = None;
        if self.map.kind == WorldKind::Q3 {
            let mut meshes = Vec::new();
            for index in 0..count {
                if let Some(geometry) = self.shadow_surface_geometry(index, None, &data)? {
                    meshes.push(shadow_mesh(&geometry));
                }
            }
            world_meshes = meshes;
        } else {
            let cached = matches!(&self.static_shadow_world, Some((cached_count, first, cached_len, _))
                if *cached_count == count && *first == 0 && *cached_len == count);
            if !cached {
                let mut meshes = Vec::new();
                for index in 0..count {
                    if let Some(geometry) = self.shadow_surface_geometry(index, None, &data)? {
                        meshes.push(shadow_mesh(&geometry));
                    }
                }
                self.static_shadow_world = Some((count, 0, count, StaticShadowWorld::new(meshes)));
            }
            static_world = self.static_shadow_world.as_ref().map(|(_, _, _, world)| world);
            world_meshes = Vec::new();
        }
        let prepared = match static_world {
            Some(world) => self
                .shadow_scene
                .prepare(lights, ShadowWorldInput::Static(world), &all_casters, options),
            None => self
                .shadow_scene
                .prepare(lights, ShadowWorldInput::Meshes(&world_meshes), &all_casters, options),
        };
        self.pending_image_operations
            .extend(self.shadow_scene.drain_operations());
        Ok(prepared)
    }

    fn shadow_model(
        &mut self,
        model_index: usize,
        transform: &ModelTransform,
        data: &ShadowContextData,
    ) -> Result<ShadowCaster, RenderError> {
        let model = *at(&self.models, model_index, "model")?;
        let mut meshes = Vec::new();
        for index in model.first..model.first + model.count {
            let Some(geometry) = self.shadow_surface_geometry(index, Some(transform), data)? else {
                continue;
            };
            let positions = geometry
                .vertices
                .iter()
                .map(|vertex| world_point(vertex.position, transform).map_err(client_error))
                .collect::<Result<Vec<_>, _>>()?;
            meshes.push(ShadowMesh {
                positions,
                indices: geometry.indices.clone(),
            });
        }
        Ok(shadow_caster(transform.origin, meshes))
    }

    fn shadow_surface_geometry(
        &mut self,
        index: usize,
        model: Option<&ModelTransform>,
        data: &ShadowContextData,
    ) -> Result<Option<MaterialGeometry>, RenderError> {
        let surface = at(&self.surfaces, index, "surface")?.clone();
        if matches!(&surface.data, WorldSurfaceData::Q3 { flare: true, .. }) {
            return Ok(None);
        }
        let resolved = self.remap(index)?;
        let selected = resolved
            .as_ref()
            .map(|(material, _)| material.clone())
            .or_else(|| surface.shader().cloned());
        let time_offset = resolved.map(|(_, offset)| offset).unwrap_or(0.0);
        if let Some(shader) = selected {
            return shadow_material_geometry(
                &shader,
                &surface.geometry,
                &ShadowMaterialContext {
                    time: data.time,
                    time_offset,
                    refdef_time: data.refdef_time,
                    render_text: data.render_text.clone(),
                    deform_view: data.deform_view,
                    noise: &self.noise,
                    projection_shadow: data.projection_shadow,
                },
            );
        }
        match &surface.data {
            WorldSurfaceData::Legacy { material, .. } => match material {
                LegacyMaterial::Q1(material) => {
                    if matches!(material.surface, Q1Surface::Ordinary | Q1Surface::Fence) && material.alpha >= 1.0 {
                        Ok(Some(surface.geometry))
                    } else {
                        Ok(None)
                    }
                }
                LegacyMaterial::Q2 { material, .. } => {
                    if material.surface_flags & (4 | 8 | 128) != 0 {
                        return Ok(None);
                    }
                    if model.is_some() && material.surface_flags & (16 | 32) != 0 {
                        return Ok(None);
                    }
                    Ok(Some(surface.geometry))
                }
            },
            WorldSurfaceData::Q3 { .. } => Ok(None),
        }
    }

    /// Rebuild the scene against replacement shaders, replaying raw remaps.
    pub fn prepare_images(&self, shaders: SceneShaderRegistry) -> Result<WorldScene, RenderError> {
        if self.closed {
            return Err(RenderError::OutOfOrder("Scene world is closed".to_string()));
        }
        let mut replacement = Self::build(
            self.build.clone(),
            self.map.clone(),
            self.visibility.clone(),
            shaders,
            self.options.clone(),
        )?;
        let mut names = Vec::new();
        for surface in &self.surfaces {
            if surface.shader().is_none() && self.raw_remaps.contains_key(&surface.index) {
                names.push(surface.shader_name.clone());
            }
        }
        names.sort();
        names.dedup();
        for name in names {
            let Some(remap) = current_remap(&name) else { continue };
            let bindings = replacement.raw_remap_bindings(&name);
            let mut materials = Vec::with_capacity(bindings.len());
            for (_, binding) in &bindings {
                materials.push(replacement.shaders.register(&remap.material, binding.clone())?);
            }
            replacement.publish_raw_remap(&name, materials, remap.time_offset)?;
        }
        replacement.prepared_generation = Some(self.image_generation);
        replacement.prepared_material_revision = Some(material_revision());
        Ok(replacement)
    }

    /// Validate a prepared replacement against this scene.
    pub fn validate_images(&self, replacement: &WorldScene) -> Result<(), RenderError> {
        if self.closed {
            return Err(RenderError::OutOfOrder("Scene world is closed".to_string()));
        }
        if replacement.prepared_generation != Some(self.image_generation)
            || replacement.prepared_material_revision != Some(material_revision())
        {
            return Err(RenderError::OutOfOrder(
                "Scene images changed while preparing a replacement".to_string(),
            ));
        }
        Ok(())
    }

    /// Commit a validated replacement, swapping shaders and surfaces.
    pub fn commit_images(&mut self, mut replacement: WorldScene) -> Result<(), RenderError> {
        self.validate_images(&replacement)?;
        self.release_images();
        self.surfaces = replacement.surfaces;
        self.models = replacement.models;
        self.bounds = replacement.bounds;
        self.fog_image = replacement.fog_image;
        self.dlight_image = replacement.dlight_image;
        self.fog_selections = replacement.fog_selections;
        self.owned = replacement.owned;
        self.q2_sky = replacement.q2_sky;
        self.fullbright_by_texture = replacement.fullbright_by_texture;
        self.raw_remaps = replacement.raw_remaps;
        self.static_light_styles = replacement.static_light_styles;
        self.static_shadow_world = replacement.static_shadow_world;
        self.pending_image_operations = replacement.pending_image_operations;
        std::mem::swap(&mut self.shaders, &mut replacement.shaders);
        self.prepared_generation = None;
        self.prepared_material_revision = None;
        self.image_generation += 1;
        self.shadow_material_revision = material_revision();
        Ok(())
    }
}

/// Shadow-context inputs shared by one shadow prepare call.
#[derive(Debug, Clone)]
struct ShadowContextData {
    time: f32,
    refdef_time: f32,
    render_text: Vec<String>,
    deform_view: DeformView,
    projection_shadow: Option<ProjectionShadowContext>,
}

/// Q2 fragment-lighting context for one shader surface.
#[derive(Debug, Clone)]
struct Q2BatchContext {
    world_positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    atlas: Option<Q2ShadowAtlas>,
    lights: Vec<Q2FragmentLight>,
}

/// Pre-bound cinematic image for dynamic texture handles.
#[derive(Debug, Clone)]
struct BoundDynamicImage {
    image: RendererImage,
}

impl crate::render::types::DynamicImageSource for BoundDynamicImage {
    fn resolve(&self, _apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage {
        self.image.clone()
    }
}

fn map_blend_factor(factor: MaterialBlendFactor) -> BlendFactor {
    match factor {
        MaterialBlendFactor::Zero => BlendFactor::Zero,
        MaterialBlendFactor::One => BlendFactor::One,
        MaterialBlendFactor::DstColor => BlendFactor::DstColor,
        MaterialBlendFactor::OneMinusDstColor => BlendFactor::OneMinusDstColor,
        MaterialBlendFactor::SrcAlpha => BlendFactor::SrcAlpha,
        MaterialBlendFactor::OneMinusSrcAlpha => BlendFactor::OneMinusSrcAlpha,
        MaterialBlendFactor::DstAlpha => BlendFactor::DstAlpha,
        MaterialBlendFactor::OneMinusDstAlpha => BlendFactor::OneMinusDstAlpha,
        MaterialBlendFactor::SrcAlphaSaturate => BlendFactor::SrcAlphaSaturate,
        MaterialBlendFactor::SrcColor => BlendFactor::SrcColor,
        MaterialBlendFactor::OneMinusSrcColor => BlendFactor::OneMinusSrcColor,
    }
}

fn map_depth_test(test: MaterialDepthTest) -> DepthTest {
    match test {
        MaterialDepthTest::LessEqual => DepthTest::LessEqual,
        MaterialDepthTest::Equal => DepthTest::Equal,
        MaterialDepthTest::Always => DepthTest::Always,
    }
}

fn map_alpha_test(test: MaterialAlphaTest) -> AlphaTest {
    match test {
        MaterialAlphaTest::None => AlphaTest::None,
        MaterialAlphaTest::Gt0 => AlphaTest::GreaterZero,
        MaterialAlphaTest::Lt128 => AlphaTest::Less128,
        MaterialAlphaTest::Ge128 => AlphaTest::GreaterEqual128,
    }
}

fn map_cull_face(face: MaterialCullFace) -> CullFace {
    match face {
        MaterialCullFace::None => CullFace::None,
        MaterialCullFace::Back => CullFace::Back,
        MaterialCullFace::Front => CullFace::Front,
    }
}

fn map_render_state(state: &crate::materials::state::RenderState) -> RenderState {
    RenderState {
        blend: (
            map_blend_factor(state.blend.source),
            map_blend_factor(state.blend.destination),
        ),
        depth_test: map_depth_test(state.depth_test),
        depth_write: state.depth_write,
        alpha_test: map_alpha_test(state.alpha_test),
        cull: map_cull_face(state.cull),
        depth_range: state.depth_range,
        polygon_offset: state.polygon_offset.map(|offset| PolygonOffset {
            factor: offset.factor,
            units: offset.units,
        }),
    }
}

fn map_texture_ref(
    reference: &TextureRef,
    registry: &SceneImageRegistry,
    dynamic_images: &HashMap<u32, RendererImage>,
) -> Result<TextureBinding, RenderError> {
    match reference {
        TextureRef::BindImage(ordinal) => Ok(TextureBinding::BindImage(
            registry
                .get(*ordinal)
                .cloned()
                .ok_or(RenderError::UnknownImage(*ordinal))?,
        )),
        TextureRef::DynamicImage(handle) => Ok(TextureBinding::DynamicImage(Arc::new(BoundDynamicImage {
            image: dynamic_images
                .get(handle)
                .cloned()
                .ok_or_else(|| RenderError::Backend(format!("Cinematic image handle {handle} has no bound frame")))?,
        }))),
        TextureRef::RetainCurrentTexture => Ok(TextureBinding::RetainCurrentTexture),
    }
}

fn map_batch_lighting(
    lighting: &crate::materials::evaluate::BatchLighting,
    q2: Option<&Q2BatchContext>,
) -> Result<BatchLighting, RenderError> {
    match lighting {
        crate::materials::evaluate::BatchLighting::Vertex => Ok(BatchLighting::Vertex),
        crate::materials::evaluate::BatchLighting::Q2World { pass } => {
            let q2 =
                q2.ok_or_else(|| RenderError::Backend("Q2 world batch arrived without fragment lighting".to_string()))?;
            let pass = match pass {
                EvaluateLightPass::Model => Q2LightPass::Model {
                    lights: q2
                        .lights
                        .iter()
                        .map(|light| Q2ModelFragmentLight {
                            light: *light,
                            fraction: vec3(1.0, 1.0, 1.0),
                        })
                        .collect(),
                    shade_scale: None,
                },
                EvaluateLightPass::MaterialLightmap => Q2LightPass::MaterialLightmap {
                    lights: q2.lights.clone(),
                },
                EvaluateLightPass::Texture => Q2LightPass::Texture {
                    lights: q2.lights.clone(),
                },
                EvaluateLightPass::Lightmap => Q2LightPass::Lightmap {
                    lights: q2.lights.clone(),
                },
            };
            Ok(BatchLighting::Q2World {
                world_positions: q2.world_positions.clone(),
                normals: q2.normals.clone(),
                atlas: q2.atlas.clone(),
                pass,
            })
        }
    }
}

fn map_batch_fog(fog: &Option<crate::materials::evaluate::BatchFog>) -> Option<BatchFog> {
    fog.as_ref().map(|fog| BatchFog::Exp2 {
        color: fog.color,
        density: fog.density,
        effect: match fog.effect {
            crate::materials::fog::FogAdjustment::None => FogEffect::None,
            crate::materials::fog::FogAdjustment::Rgb => FogEffect::Rgb,
            crate::materials::fog::FogAdjustment::Alpha => FogEffect::Alpha,
            crate::materials::fog::FogAdjustment::Rgba => FogEffect::Rgba,
        },
    })
}

/// Convert one evaluated material batch into a draw batch.
fn draw_batch_from_material(
    registry: &SceneImageRegistry,
    batch: &MaterialBatch,
    q2: Option<&Q2BatchContext>,
    dynamic_images: &HashMap<u32, RendererImage>,
) -> Result<DrawBatch, RenderError> {
    let texture = map_texture_ref(&batch.texture, registry, dynamic_images)?;
    let vertices = match batch.texturing {
        Texturing::Single => BatchVertices::Single(
            batch
                .vertices
                .iter()
                .map(|vertex| RenderVertex {
                    position: vertex.position,
                    tex_coord: vertex.tex_coord,
                    color: vertex.color,
                })
                .collect(),
        ),
        Texturing::Pair => {
            let (second, environment) = batch
                .second_texture
                .as_ref()
                .ok_or_else(|| RenderError::Backend("Paired batch is missing its second texture".to_string()))?;
            let mut vertices = Vec::with_capacity(batch.vertices.len());
            for vertex in &batch.vertices {
                vertices.push(MultitextureVertex {
                    base: RenderVertex {
                        position: vertex.position,
                        tex_coord: vertex.tex_coord,
                        color: vertex.color,
                    },
                    tex_coord2: vertex.tex_coord2.ok_or_else(|| {
                        RenderError::Backend("Paired batch vertex is missing its second coordinates".to_string())
                    })?,
                });
            }
            BatchVertices::Pair {
                vertices,
                second_texture: TextureBundle {
                    binding: map_texture_ref(second, registry, dynamic_images)?,
                    environment: match environment {
                        PairEnv::Modulate => PairEnvironment::Modulate,
                        PairEnv::Add => PairEnvironment::Add,
                    },
                },
            }
        }
    };
    Ok(DrawBatch {
        fog: map_batch_fog(&batch.fog),
        luminance_alpha: false,
        indices: batch.indices.clone(),
        texture,
        state: map_render_state(&batch.state),
        lighting: map_batch_lighting(&batch.lighting, q2)?,
        primitive: BatchPrimitive::Triangles,
        vertices,
    })
}

fn flip_cull(batch: &mut DrawBatch) {
    batch.state.cull = match batch.state.cull {
        CullFace::Front => CullFace::Back,
        CullFace::Back => CullFace::Front,
        CullFace::None => CullFace::None,
    };
}

impl WorldScene {
    #[allow(clippy::too_many_arguments)]
    fn surface_operations(
        &mut self,
        index: usize,
        input: &WorldViewInput,
        data: &DrawContextData,
        model: Option<&ModelTransform>,
        lighting: (u32, Vec<DynamicLight>),
        order: Option<SourceSurfaceOrder>,
    ) -> Result<Vec<SceneOperation>, RenderError> {
        let surface = at(&self.surfaces, index, "surface")?.clone();
        let project =
            |point: Vec3| crate::view::project_point(&input.camera, model, point).unwrap_or(vec4(0.0, 0.0, 0.0, 1.0));
        let project_ref: &dyn Fn(Vec3) -> Vec4 = &project;
        let resolved = self.remap(index)?;
        let selected = resolved
            .as_ref()
            .map(|(material, _)| material.clone())
            .or_else(|| surface.shader().cloned());
        let time_offset = resolved.map(|(_, offset)| offset).unwrap_or(0.0);
        if selected.is_none() {
            let WorldSurfaceData::Legacy {
                material,
                lightmap,
                q1_sky,
                ..
            } = &surface.data
            else {
                return Ok(Vec::new());
            };
            let sky = match material {
                LegacyMaterial::Q1(_) => q1_sky.is_some(),
                LegacyMaterial::Q2 { material, .. } => material.surface_flags & 4 != 0,
            };
            if matches!(material, LegacyMaterial::Q2 { material, .. } if material.surface_flags & 128 != 0 && !sky) {
                return Ok(Vec::new());
            }
            if let Some(plane) = surface.plane {
                if dot3(data.local_view_origin, plane.normal) - plane.distance < -0.01 {
                    return Ok(Vec::new());
                }
            }
            if let Some(layers) = q1_sky {
                if let Some(sky) = input.source_sky.as_ref() {
                    return Ok(self
                        .q2_sky_operations(&surface.geometry, sky, input, project_ref)?
                        .into_iter()
                        .map(SceneOperation::Operation)
                        .collect());
                }
                let batches = self.q1_sky_batches(&surface.geometry, layers, input, data)?;
                return Ok(vec![SceneOperation::Group(sequence_draw_group(
                    SequencePhase::Sky,
                    batches,
                ))]);
            }
            if sky && matches!(material, LegacyMaterial::Q2 { .. }) {
                let fallback = Q2SkyView {
                    images: self.q2_sky.clone(),
                    rotation: 0.0,
                    auto_rotate: false,
                    axis: vec3(0.0, 0.0, 1.0),
                };
                let sky = input.source_sky.as_ref().or(input.q2_sky.as_ref()).unwrap_or(&fallback);
                return Ok(self
                    .q2_sky_operations(&surface.geometry, sky, input, project_ref)?
                    .into_iter()
                    .map(SceneOperation::Operation)
                    .collect());
            }
            if let Some(lightmap) = lightmap {
                self.refresh_lightmap(index, lightmap, material, input, model)?;
            }
            let fullbright = match material {
                LegacyMaterial::Q1(material) => {
                    let animated =
                        q1_animated_texture(material, data.time, input.alternate_animation).map_err(client_error)?;
                    self.fullbright_by_texture
                        .get(&animated)
                        .and_then(|entry| entry.as_ref())
                        .map(|image| image.ordinal)
                }
                LegacyMaterial::Q2 { .. } => match &surface.data {
                    WorldSurfaceData::Legacy { fullbright, .. } => fullbright.as_ref().map(|image| image.ordinal),
                    _ => None,
                },
            };
            let entity = data.entity_rgba;
            let context = LegacyMaterialDrawContext {
                entity_rgba: Some(vec4(
                    f32::from(entity[0]) / 255.0,
                    f32::from(entity[1]) / 255.0,
                    f32::from(entity[2]) / 255.0,
                    f32::from(entity[3]) / 255.0,
                )),
                time: data.time,
                animation_frame: input.animation_frame.unwrap_or((data.time * 2.0).trunc()),
                alternate_animation: input.alternate_animation,
                fullbright,
                q1_fog_active: input.q1_fog.is_some_and(|fog| fog.density > 0.0),
                q1_lightmap_encoding: lightmap
                    .as_ref()
                    .map(|lightmap| lightmap.encoding)
                    .unwrap_or(self.options.q1_lightmap_encoding),
                translucent_lightmap: lightmap.as_ref().map(|lightmap| lightmap.direct.ordinal),
                cull: if data.mirror != model.is_some_and(|model| model_scale(model).is_ok_and(|scale| scale < 0.0)) {
                    MaterialCullFace::Back
                } else {
                    MaterialCullFace::Front
                },
                depth_range: [0.0, 1.0],
                project: project_ref,
            };
            let batches =
                prepare_legacy_material_batches(material, &surface.geometry, &context).map_err(client_error)?;
            let alpha = match material {
                LegacyMaterial::Q1(material) => material.alpha,
                LegacyMaterial::Q2 { material, .. } => material.alpha,
            } * f32::from(entity[3])
                / 255.0;
            let mut converted = Vec::with_capacity(batches.len());
            for batch in &batches {
                converted.push(draw_batch_from_material(
                    self.shaders.textures().images(),
                    batch,
                    None,
                    &input.dynamic_images,
                )?);
            }
            return Ok(vec![SceneOperation::Group(sequence_draw_group(
                if alpha < 1.0 {
                    SequencePhase::Translucent
                } else {
                    SequencePhase::Opaque
                },
                converted,
            ))]);
        }
        let shader = selected.expect("resolved selected shader");
        if matches!(&surface.data, WorldSurfaceData::Q3 { flare: true, .. }) {
            return Ok(input
                .prepare_flare
                .as_ref()
                .map(|hook| hook(&surface, &input.camera))
                .unwrap_or_default()
                .into_iter()
                .map(SceneOperation::Operation)
                .collect());
        }
        if let WorldSurfaceData::Legacy {
            lightmap: Some(lightmap),
            material,
            ..
        } = &surface.data
        {
            self.refresh_lightmap(index, lightmap, material, input, model)?;
        }
        self.shader_operations(
            &surface,
            &shader,
            time_offset,
            input,
            data,
            model,
            &lighting,
            order.as_ref(),
            project_ref,
        )
    }

    fn refresh_lightmap(
        &mut self,
        index: usize,
        lightmap: &LegacyLightmap,
        material: &LegacyMaterial,
        input: &WorldViewInput,
        model: Option<&ModelTransform>,
    ) -> Result<(), RenderError> {
        let lights = input
            .lights
            .iter()
            .map(|light| {
                Ok(SurfaceDynamicLight {
                    origin: maybe_world_point(light.origin, model)?,
                    radius: light.radius,
                    minimum: light.minimum,
                    color: light.color,
                })
            })
            .collect::<Result<Vec<_>, RenderError>>()?;
        let styles: Vec<f32> = match material {
            LegacyMaterial::Q1(_) => lightmap
                .face
                .styles
                .iter()
                .take_while(|style| **style != 255)
                .map(|style| {
                    input
                        .q1_styles
                        .get(*style as usize)
                        .copied()
                        .ok_or_else(|| RenderError::BadBatch {
                            index: *style as usize,
                            detail: "Q1 light style is outside the style table".to_string(),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(|style| style as f32)
                .collect(),
            LegacyMaterial::Q2 { .. } => {
                let mut styles = vec![self.options.q2_light_modulate];
                for style in lightmap.face.styles.iter().take_while(|style| **style != 255) {
                    let entry = input
                        .q2_styles
                        .get(*style as usize)
                        .ok_or_else(|| RenderError::BadBatch {
                            index: *style as usize,
                            detail: "Q2 light style is outside the style table".to_string(),
                        })?;
                    styles.extend([entry.rgb.x, entry.rgb.y, entry.rgb.z]);
                }
                styles
            }
        };
        if self.static_light_styles.get(&index) == Some(&styles) {
            return Ok(());
        }
        let built = match material {
            LegacyMaterial::Q1(_) => build_q1_lightmap(
                &lightmap.face,
                &input.q1_styles,
                Some(lightmap.encoding),
                false,
                &lights,
            )
            .map_err(client_error)?,
            LegacyMaterial::Q2 { .. } => build_q2_lightmap(
                &lightmap.face,
                &input.q2_styles,
                self.options.q2_light_modulate,
                Q2Mono::Color,
                &lights,
            )
            .map_err(client_error)?,
        };
        let direct = direct_lightmap_pixels(&built).map_err(client_error)?;
        self.shaders.textures_mut().images_mut().update(
            &lightmap.image,
            0,
            LevelContent::Rgba(built_lightmap_image(&built)?),
        )?;
        self.shaders.textures_mut().images_mut().update(
            &lightmap.direct,
            0,
            LevelContent::Rgba(built_lightmap_image(&direct)?),
        )?;
        self.static_light_styles.insert(index, styles);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn shader_operations(
        &mut self,
        surface: &WorldSurface,
        shader: &RegisteredSceneMaterial,
        time_offset: f32,
        input: &WorldViewInput,
        data: &DrawContextData,
        model: Option<&ModelTransform>,
        lighting: &(u32, Vec<DynamicLight>),
        order: Option<&SourceSurfaceOrder>,
        project: &dyn Fn(Vec3) -> Vec4,
    ) -> Result<Vec<SceneOperation>, RenderError> {
        if shader.finished.iterator.kind == MaterialIteratorKind::Sky {
            return self.sky_operations(surface, shader, input, data, order, project);
        }
        let geometry = match &surface.data {
            WorldSurfaceData::Q3 { grid: Some(grid), .. } => {
                let world_origin = match model {
                    None => grid.lod_origin,
                    Some(model) => world_point(grid.lod_origin, model).map_err(client_error)?,
                };
                let mesh = select_patch_lod(
                    grid,
                    world_origin,
                    data.view_origin,
                    data.deform_view.axis[0],
                    input.curve_error,
                );
                MaterialGeometry {
                    vertices: mesh.vertices,
                    indices: mesh.indices,
                }
            }
            _ => surface.geometry.clone(),
        };
        let fog = match &surface.data {
            WorldSurfaceData::Q3 { fog: Some(fog), .. } => Some(*fog),
            _ => None,
        };
        let dlight_ordinal = self.dlight_image.ordinal;
        let filtered: Vec<DynamicLight> = lighting
            .1
            .iter()
            .enumerate()
            .filter(|(slot, _)| *slot < 32 && lighting.0 & (1 << slot) != 0)
            .map(|(_, light)| *light)
            .collect();
        let dlight_batches = |deformed: &MaterialGeometry| {
            project_dlight_texture(
                &DeformGeometry::from(deformed.clone()),
                lighting.0,
                &filtered,
                dlight_ordinal,
                project,
                MaterialCullFace::Back,
            )
            .unwrap_or_default()
        };
        let hook: DynamicLightBatches = if lighting.0 != 0 && receives_projected_dlights(shader) {
            Some(&dlight_batches)
        } else {
            None
        };
        let fog_image = self.fog_image.ordinal;
        let camera = input.camera;
        let coordinates = fog.map(|fog| FogCoordinates::new(&fog, &camera.origin, &camera.axis[0]));
        let map_fog = |point: Vec3| {
            coordinates
                .as_ref()
                .map(|coordinates| coordinates.coordinates(&point))
                .unwrap_or(vec2(0.0, 0.0))
        };
        let context = draw_context(
            data,
            &self.noise,
            project,
            fog.map(|fog| FogVolumeInput {
                coordinates: &map_fog,
                texture: TextureRef::BindImage(fog_image),
                color: fog.color,
            }),
            hook,
            time_offset,
        );
        let q2 = input.q2_fragment_lighting.as_ref().map(|fragment| {
            let mut lights = fragment
                .lights
                .iter()
                .filter(|light| light.cone.is_none() || (light.radius > 0.0 && light.scale != 0.0))
                .cloned()
                .collect::<Vec<_>>();
            for light in &mut lights {
                light.color = scale3(light.color, self.options.q2_light_modulate);
            }
            let (world_positions, normals) = match model {
                None => (
                    geometry.vertices.iter().map(|vertex| vertex.position).collect(),
                    geometry.vertices.iter().map(|vertex| vertex.normal).collect(),
                ),
                Some(model) => (
                    geometry
                        .vertices
                        .iter()
                        .map(|vertex| world_point(vertex.position, model).unwrap_or(vertex.position))
                        .collect(),
                    geometry
                        .vertices
                        .iter()
                        .map(|vertex| world_vector(vertex.normal, model).unwrap_or(vertex.normal))
                        .collect(),
                ),
            };
            Q2BatchContext {
                world_positions,
                normals,
                atlas: fragment.atlas.clone(),
                lights,
            }
        });
        let batches = prepare_material_batches(shader, &geometry, &context).map_err(client_error)?;
        let mut converted = Vec::with_capacity(batches.len());
        for batch in &batches {
            let mut draw = draw_batch_from_material(
                self.shaders.textures().images(),
                batch,
                q2.as_ref(),
                &input.dynamic_images,
            )?;
            if data.mirror {
                flip_cull(&mut draw);
            }
            converted.push(draw);
        }
        Ok(vec![SceneOperation::Group(match order {
            None => compiled_draw_group(shader.clone(), converted),
            Some(order) => source_draw_group(shader.clone(), order.clone(), converted)?,
        })])
    }

    fn sky_operations(
        &mut self,
        surface: &WorldSurface,
        shader: &RegisteredSceneMaterial,
        input: &WorldViewInput,
        data: &DrawContextData,
        order: Option<&SourceSurfaceOrder>,
        project: &dyn Fn(Vec3) -> Vec4,
    ) -> Result<Vec<SceneOperation>, RenderError> {
        if let Some(sky) = input.source_sky.as_ref() {
            return Ok(self
                .q2_sky_operations(&surface.geometry, sky, input, project)?
                .into_iter()
                .map(SceneOperation::Operation)
                .collect());
        }
        let origin = input.camera.origin;
        self.shaders
            .sky
            .clip(&[DeformGeometry::from(surface.geometry.clone())], origin)
            .map_err(client_error)?;
        let far = far_clip(origin, &self.bounds).max(2048.0);
        let built = self.shaders.sky.build(origin, far).map_err(client_error)?;
        let _ = far;
        let mut operations = vec![RenderOperation::DepthRange([1.0, 1.0])];
        if let Some(outer) = shader.registered.sky.as_ref().and_then(|sky| sky.outer.as_ref()) {
            let light = data.identity_light;
            for face in &built.sky_box {
                let Some(image) = outer
                    .faces
                    .get(face.face)
                    .and_then(|ordinal| self.shaders.textures().images().get(*ordinal).cloned())
                else {
                    continue;
                };
                let mut strips = Vec::with_capacity(face.strips.len());
                for strip in &face.strips {
                    let mut vertices = Vec::with_capacity(strip.len());
                    for index in strip {
                        let vertex =
                            face.geometry
                                .vertices
                                .get(*index as usize)
                                .ok_or_else(|| RenderError::BadBatch {
                                    index: *index as usize,
                                    detail: "Sky strip index is outside its vertices".to_string(),
                                })?;
                        vertices.push(SkyVertex {
                            position: project(vertex.position),
                            tex_coord: vertex.tex_coord,
                        });
                    }
                    strips.push(vertices);
                }
                operations.push(RenderOperation::SkySide {
                    image,
                    color: vec4(light, light, light, 1.0),
                    strips,
                });
            }
        }
        if !built.clouds.vertices.is_empty() {
            let context = draw_context(data, &self.noise, project, None, None, 0.0);
            let batches = evaluate_material_passes(shader, &MaterialGeometry::from(built.clouds.clone()), &context)
                .map_err(client_error)?;
            let mut converted = Vec::with_capacity(batches.len());
            for batch in &batches {
                let mut draw =
                    draw_batch_from_material(self.shaders.textures().images(), batch, None, &input.dynamic_images)?;
                if data.mirror {
                    flip_cull(&mut draw);
                }
                converted.push(draw);
            }
            let group = match order {
                None => compiled_draw_group(shader.clone(), converted),
                Some(order) => source_draw_group(shader.clone(), order.clone(), converted)?,
            };
            return Ok(vec![
                SceneOperation::Operation(operations[0].clone()),
                SceneOperation::Group(group),
                SceneOperation::Operation(RenderOperation::DepthRange([0.0, 1.0])),
            ]);
        }
        operations.push(RenderOperation::DepthRange([0.0, 1.0]));
        Ok(operations.into_iter().map(SceneOperation::Operation).collect())
    }

    fn q2_sky_operations(
        &self,
        geometry: &MaterialGeometry,
        sky: &Q2SkyView,
        input: &WorldViewInput,
        project: &dyn Fn(Vec3) -> Vec4,
    ) -> Result<Vec<RenderOperation>, RenderError> {
        let mut operations = vec![RenderOperation::DepthRange([1.0, 1.0])];
        operations.extend(q2_sky_sides(
            geometry,
            input.camera.origin,
            sky,
            input.time.as_seconds() as f32,
            project,
        )?);
        operations.push(RenderOperation::DepthRange([0.0, 1.0]));
        Ok(operations)
    }

    fn q1_sky_batches(
        &self,
        geometry: &MaterialGeometry,
        layers: &Q1SkyLayers,
        input: &WorldViewInput,
        data: &DrawContextData,
    ) -> Result<Vec<DrawBatch>, RenderError> {
        let fog = if input.q1_fog.is_some_and(|fog| fog.density > 0.0) {
            input.q1_fog.map(|fog| BatchFog::Constant {
                color: fog.color,
                amount: fog.sky_factor,
            })
        } else {
            None
        };
        let mut batches = Vec::with_capacity(2);
        for (image, layer, blend) in [
            (&layers.solid, Q1SkyLayer::Solid, (BlendFactor::One, BlendFactor::Zero)),
            (
                &layers.overlay,
                Q1SkyLayer::Overlay,
                (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
            ),
        ] {
            batches.push(DrawBatch {
                fog,
                luminance_alpha: false,
                indices: geometry.indices.clone(),
                texture: TextureBinding::BindImage(image.clone()),
                state: RenderState {
                    blend,
                    depth_test: DepthTest::LessEqual,
                    depth_write: true,
                    alpha_test: AlphaTest::None,
                    cull: CullFace::None,
                    depth_range: [0.0, 1.0],
                    polygon_offset: None,
                },
                lighting: BatchLighting::Vertex,
                primitive: BatchPrimitive::Triangles,
                vertices: BatchVertices::Single(
                    geometry
                        .vertices
                        .iter()
                        .map(|vertex| RenderVertex {
                            position: vec4(vertex.position.x, vertex.position.y, vertex.position.z, 1.0),
                            tex_coord: q1_sky_tex_coords(vertex.position, data.view_origin, data.time, layer),
                            color: vec4(1.0, 1.0, 1.0, 1.0),
                        })
                        .collect(),
                ),
            });
        }
        Ok(batches)
    }
}

#[cfg(test)]
mod tests {
    use super::super::textures::SceneTextureLoader;
    use super::*;
    use crate::render::types::{fresh_owner_identity, Palette, ResourceOwner};
    use crate::render::LightShadow;
    use crate::view::{CameraClip, Rect};
    use qa_content::bsp::{
        BspFormat, ClipNode, Edge, Face, IndexRange, Leaf, Lump, Node, NodeChild, Plane as ContentPlane, Q1Entity,
        Q1Map, TextureInfo, WorldModel,
    };
    use qa_content::wad::MipTexture;
    use qa_core::identity::IdentityOwner;

    struct FakeReader {
        assets: HashMap<String, Vec<u8>>,
    }

    impl super::super::textures::SceneAssetReader for FakeReader {
        fn read(&self, path: &str) -> Result<Option<super::super::textures::SceneAsset>, RenderError> {
            use super::super::textures::SceneAsset;
            Ok(self.assets.get(path).map(|bytes| SceneAsset {
                bytes: bytes.clone(),
                source: ImageSource::Resource {
                    requested_path: path.to_string(),
                },
            }))
        }
    }

    fn owner(name: &str) -> ResourceOwner {
        let session = IdentityOwner::create(name).expect("session").session().clone();
        ResourceOwner::new(fresh_owner_identity(), session, 0)
    }

    fn palette() -> Palette {
        let mut colors = vec![0u8; 768];
        colors[3] = 255;
        colors[4] = 255;
        colors[5] = 255;
        Palette {
            colors,
            source: "test".to_string(),
        }
    }

    fn registry(name: &str, assets: HashMap<String, Vec<u8>>) -> SceneShaderRegistry {
        let loader = SceneTextureLoader::new(
            SceneImageRegistry::new(owner(name)),
            Box::new(FakeReader { assets }),
            Some(palette()),
            None,
            224,
        )
        .expect("loader");
        SceneShaderRegistry::with_defaults(loader)
    }

    fn camera_at(origin: Vec3) -> SceneCamera {
        SceneCamera {
            origin,
            axis: [vec3(0.0, 0.0, -1.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.0, 0.0, -8.0, 0.0,
            ],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            clip: CameraClip::None,
        }
    }

    fn input(camera: SceneCamera) -> WorldViewInput {
        WorldViewInput::new(
            camera,
            ViewTarget::Preview("test".to_string()),
            SourceTime::Seconds(1.0),
        )
    }

    fn q1_map<'a>(
        visibility: &'a [u8],
        lighting: &'a [u8],
        l0: &'a [u8],
        l1: &'a [u8],
        l2: &'a [u8],
        l3: &'a [u8],
        s0: &'a [u8],
        s1: &'a [u8],
        s2: &'a [u8],
        s3: &'a [u8],
    ) -> Q1Map<'a> {
        let bounds = qa_content::common::Bounds {
            min: [0.0, 0.0, 0.0],
            max: [16.0, 16.0, 64.0],
        };
        Q1Map {
            format: BspFormat::Bsp29,
            source: "test".to_string(),
            version: 29,
            data: &[],
            lumps: Vec::<Lump>::new(),
            entities: String::new(),
            entity_list: Vec::<Q1Entity>::new(),
            planes: vec![
                ContentPlane {
                    normal: [1.0, 0.0, 0.0],
                    distance: 0.0,
                    plane_type: 0,
                    signbits: 0,
                },
                ContentPlane {
                    normal: [0.0, 0.0, 1.0],
                    distance: 0.0,
                    plane_type: 2,
                    signbits: 0,
                },
            ],
            vertices: vec![[0.0, 0.0, 0.0], [16.0, 0.0, 0.0], [16.0, 16.0, 0.0], [0.0, 16.0, 0.0]],
            textures: vec![
                Some(MipTexture::Embedded {
                    name: "rock".to_string(),
                    width: 16,
                    height: 16,
                    levels: [l0, l1, l2, l3],
                }),
                Some(MipTexture::Embedded {
                    name: "sky1".to_string(),
                    width: 256,
                    height: 128,
                    levels: [s0, s1, s2, s3],
                }),
            ],
            texture_offsets: vec![],
            mip_offsets: vec![],
            texture_info: vec![
                TextureInfo {
                    s: [1.0, 0.0, 0.0, 0.0],
                    t: [0.0, 1.0, 0.0, 0.0],
                    texture: 0,
                    flags: 0,
                },
                TextureInfo {
                    s: [1.0, 0.0, 0.0, 0.0],
                    t: [0.0, 1.0, 0.0, 0.0],
                    texture: 1,
                    flags: 0,
                },
            ],
            faces: vec![
                Face {
                    plane: 1,
                    back: false,
                    edge_first: 0,
                    edge_count: 4,
                    texture_info: 0,
                    styles: [0, 255, 255, 255],
                    lighting_offset: Some(0),
                },
                Face {
                    plane: 1,
                    back: false,
                    edge_first: 0,
                    edge_count: 4,
                    texture_info: 1,
                    styles: [0, 255, 255, 255],
                    lighting_offset: None,
                },
            ],
            models: vec![WorldModel {
                bounds,
                origin: [0.0, 0.0, 0.0],
                headnodes: [0, -1, -1, -1],
                visible_leaves: 2,
                face_first: 0,
                face_count: 2,
            }],
            nodes: vec![Node {
                plane: 0,
                children: [NodeChild::Leaf(1), NodeChild::Leaf(2)],
                bounds,
                faces: IndexRange { first: 0, count: 0 },
            }],
            leaves: vec![
                Leaf {
                    contents: -2,
                    visibility_offset: None,
                    bounds: qa_content::common::Bounds {
                        min: [0.0, 0.0, 0.0],
                        max: [0.0, 0.0, 0.0],
                    },
                    faces: IndexRange { first: 0, count: 0 },
                    ambient_sound: [0, 0, 0, 0],
                },
                Leaf {
                    contents: -1,
                    visibility_offset: Some(0),
                    bounds,
                    faces: IndexRange { first: 0, count: 2 },
                    ambient_sound: [0, 0, 0, 0],
                },
                Leaf {
                    contents: -1,
                    visibility_offset: Some(0),
                    bounds,
                    faces: IndexRange { first: 0, count: 0 },
                    ambient_sound: [0, 0, 0, 0],
                },
            ],
            edges: vec![
                Edge { vertices: [0, 1] },
                Edge { vertices: [1, 2] },
                Edge { vertices: [2, 3] },
                Edge { vertices: [3, 0] },
            ],
            clipnodes: Vec::<ClipNode>::new(),
            surface_edges: vec![0, 1, 2, 3],
            leaf_faces: vec![0, 1],
            visibility,
            lighting,
        }
    }

    fn q1_levels() -> (
        [u8; 256],
        [u8; 64],
        [u8; 16],
        [u8; 4],
        [u8; 32768],
        [u8; 8192],
        [u8; 2048],
        [u8; 512],
    ) {
        (
            [1u8; 256],
            [1u8; 64],
            [1u8; 16],
            [1u8; 4],
            [1u8; 32768],
            [1u8; 8192],
            [1u8; 2048],
            [1u8; 512],
        )
    }

    #[test]
    fn q1_load_builds_surfaces_and_lightmaps() {
        let levels = q1_levels();
        let visibility = [0b11u8];
        let lighting = [128u8; 4];
        let map = q1_map(
            &visibility,
            &lighting,
            &levels.0,
            &levels.1,
            &levels.2,
            &levels.3,
            &levels.4,
            &levels.5,
            &levels.6,
            &levels.7,
        );
        let scene = WorldScene::load_q1(&map, registry("world-q1", HashMap::new()), WorldSceneOptions::default())
            .expect("load");
        assert_eq!(scene.surfaces().len(), 2);
        assert!(matches!(
            scene.surfaces()[0].data,
            WorldSurfaceData::Legacy { lightmap: Some(_), .. }
        ));
        assert!(matches!(
            scene.surfaces()[1].data,
            WorldSurfaceData::Legacy { q1_sky: Some(_), .. }
        ));
    }

    #[test]
    fn q1_view_assembles_ordered_operations() {
        let levels = q1_levels();
        let visibility = [0b11u8];
        let lighting = [128u8; 4];
        let map = q1_map(
            &visibility,
            &lighting,
            &levels.0,
            &levels.1,
            &levels.2,
            &levels.3,
            &levels.4,
            &levels.5,
            &levels.6,
            &levels.7,
        );
        let mut scene = WorldScene::load_q1(
            &map,
            registry("world-q1-view", HashMap::new()),
            WorldSceneOptions::default(),
        )
        .expect("load");
        let mut view_input = input(camera_at(vec3(8.0, 8.0, 64.0)));
        let prepared = scene.prepare_view(&mut view_input).expect("view");
        assert_eq!(prepared.view.operations.len(), 2);
        assert!(matches!(prepared.view.operations[0], RenderOperation::Draw(_)));
        assert_eq!(prepared.image_operations.len(), 13);
        let creates = prepared
            .image_operations
            .iter()
            .filter(|operation| matches!(operation, ImageResourceOperation::CreateImage { .. }))
            .count();
        assert_eq!(creates, 11);
        let second = scene.prepare_view(&mut view_input).expect("second view");
        assert!(second.image_operations.is_empty());
    }

    #[test]
    fn empty_view_skips_world_model() {
        let levels = q1_levels();
        let visibility = [0b11u8];
        let lighting = [128u8; 4];
        let map = q1_map(
            &visibility,
            &lighting,
            &levels.0,
            &levels.1,
            &levels.2,
            &levels.3,
            &levels.4,
            &levels.5,
            &levels.6,
            &levels.7,
        );
        let mut scene = WorldScene::load_q1(
            &map,
            registry("world-q1-empty", HashMap::new()),
            WorldSceneOptions::default(),
        )
        .expect("load");
        let mut view_input = input(camera_at(vec3(8.0, 8.0, 64.0)));
        view_input.no_world_model = true;
        let prepared = scene.prepare_view(&mut view_input).expect("view");
        assert!(prepared.view.operations.is_empty());
    }

    #[test]
    fn raw_remap_round_trips_through_shader_names() {
        let levels = q1_levels();
        let visibility = [0b11u8];
        let lighting = [128u8; 4];
        let map = q1_map(
            &visibility,
            &lighting,
            &levels.0,
            &levels.1,
            &levels.2,
            &levels.3,
            &levels.4,
            &levels.5,
            &levels.6,
            &levels.7,
        );
        let mut scene = WorldScene::load_q1(
            &map,
            registry("world-q1-remap", HashMap::new()),
            WorldSceneOptions::default(),
        )
        .expect("load");
        scene
            .remap_shader("textures/rock", "textures/rock", 0.0)
            .expect("clear");
        assert!(current_remap("textures/rock").is_none());
        scene
            .remap_shader("textures/rock", "textures/other", 1.5)
            .expect("remap");
        let remap = current_remap("textures/rock").expect("remap");
        assert_eq!(remap.material, "textures/other");
        assert_eq!(remap.time_offset, 1.5);
        remove_remap("textures/rock");
    }

    #[test]
    fn shadows_prepare_with_point_light() {
        let levels = q1_levels();
        let visibility = [0b11u8];
        let lighting = [128u8; 4];
        let map = q1_map(
            &visibility,
            &lighting,
            &levels.0,
            &levels.1,
            &levels.2,
            &levels.3,
            &levels.4,
            &levels.5,
            &levels.6,
            &levels.7,
        );
        let mut scene = WorldScene::load_q1(
            &map,
            registry("world-q1-shadow", HashMap::new()),
            WorldSceneOptions::default(),
        )
        .expect("load");
        let view_input = input(camera_at(vec3(8.0, 8.0, 64.0)));
        let lights = vec![SceneLight {
            origin: vec3(8.0, 8.0, 32.0),
            color: vec3(1.0, 1.0, 1.0),
            radius: 300.0,
            additive: false,
            profile: crate::render::LightProfile::Q2 {
                scale: 1.0,
                cone: None,
                shadow: LightShadow::Cast { resolution: 128 },
            },
        }];
        let options = ShadowAtlasOptions {
            enabled: true,
            resolution_cap: 512,
        };
        let prepared = scene
            .prepare_shadows(&lights, &view_input, &[], &options)
            .expect("shadows");
        assert_eq!(prepared.lighting.lights.len(), 1);
        assert!(!prepared.operations.is_empty());
        let cached = scene
            .prepare_shadows(&lights, &view_input, &[], &options)
            .expect("shadows");
        assert_eq!(cached.stats.cached_lights, 1);
    }

    #[test]
    fn q2_load_and_view_cover_wal_textures() {
        use qa_content::bsp2::{Q2Bsp, Q2DecodedMap, Q2Face, Q2Format, Q2Leaf, Q2TextureInfo, Q2WorldModel};
        let bounds = qa_content::common::Bounds {
            min: [0.0, 0.0, 0.0],
            max: [16.0, 16.0, 64.0],
        };
        let lighting: [u8; 0] = [];
        let map = Q2Bsp {
            source: "test".to_string(),
            format: Q2Format::Ibsp38,
            version: 38,
            lumps: vec![],
            entities: String::new(),
            entity_bytes: b"",
            planes: vec![ContentPlane {
                normal: [0.0, 0.0, 1.0],
                distance: 0.0,
                plane_type: 2,
                signbits: 0,
            }],
            vertices: vec![[0.0, 0.0, 0.0], [16.0, 0.0, 0.0], [16.0, 16.0, 0.0], [0.0, 16.0, 0.0]],
            edges: vec![
                Edge { vertices: [0, 1] },
                Edge { vertices: [1, 2] },
                Edge { vertices: [2, 3] },
                Edge { vertices: [3, 0] },
            ],
            surface_edges: vec![0, 1, 2, 3],
            nodes: vec![],
            leaves: vec![],
            leaf_faces: vec![0],
            leaf_brushes: vec![],
            texture_info: vec![],
            faces: vec![],
            brushes: vec![],
            brush_sides: vec![],
            models: vec![Q2WorldModel {
                bounds,
                origin: [0.0, 0.0, 0.0],
                headnode: 0,
                faces: IndexRange { first: 0, count: 1 },
            }],
            areas: vec![],
            area_portals: vec![],
            visibility: None,
            lighting: &lighting,
            pop: b"",
            bspx: None,
            diagnostics: vec![],
        };
        let decoded = Q2DecodedMap {
            map,
            leaves: vec![Q2Leaf {
                contents: 0,
                merged_contents: 0,
                cluster: 0,
                area: 0,
                bounds,
                faces: IndexRange { first: 0, count: 1 },
                brushes: IndexRange { first: 0, count: 0 },
            }],
            faces: vec![Q2Face {
                plane: 0,
                back: false,
                edges: IndexRange { first: 0, count: 4 },
                texture_info: 0,
                styles: [0, 255, 255, 255],
                lighting_offset: None,
            }],
            texture_info: vec![Q2TextureInfo {
                projection_s: [1.0, 0.0, 0.0, 0.0],
                projection_t: [0.0, 1.0, 0.0, 0.0],
                flags: 0,
                value: 0,
                name: "rock".to_string(),
                material: String::new(),
                next: None,
            }],
            models: vec![Q2WorldModel {
                bounds,
                origin: [0.0, 0.0, 0.0],
                headnode: 0,
                faces: IndexRange { first: 0, count: 1 },
            }],
            decoupled_lightmaps: None,
            lightgrid: None,
            face_normals: None,
            diagnostics: vec![],
        };
        let mut wal = vec![0u8; 100];
        wal[0..4].copy_from_slice(b"rock");
        wal[32..36].copy_from_slice(&4u32.to_le_bytes());
        wal[36..40].copy_from_slice(&4u32.to_le_bytes());
        wal[40..44].copy_from_slice(&100u32.to_le_bytes());
        wal[44..48].copy_from_slice(&116u32.to_le_bytes());
        wal[48..52].copy_from_slice(&120u32.to_le_bytes());
        wal[52..56].copy_from_slice(&121u32.to_le_bytes());
        wal.extend(vec![1u8; 22]);
        let mut assets = HashMap::new();
        assets.insert("textures/rock.wal".to_string(), wal);
        let mut scene =
            WorldScene::load_q2(&decoded, registry("world-q2", assets), WorldSceneOptions::default()).expect("load");
        assert_eq!(scene.surfaces().len(), 1);
        let mut view_input = input(camera_at(vec3(8.0, 8.0, 64.0)));
        let prepared = scene.prepare_view(&mut view_input).expect("view");
        assert_eq!(prepared.view.operations.len(), 1);
        assert_eq!(prepared.image_operations.len(), 5);
    }

    #[test]
    fn q3_load_and_view_cover_planar_surfaces() {
        use qa_content::bsp3::{
            IntBounds, Q3DecodedWorld, Q3Map, Q3Plane, Q3Shader, Q3Surface, Q3SurfaceLightmap, Q3SurfaceType, Q3Vertex,
            Q3WorldLeaf, Q3WorldModel, Q3WorldPlane, Q3WorldSurface, Q3WorldSurfaceKind,
        };
        let vertex = |x: f32, y: f32| Q3Vertex {
            position: [x, y, 0.0],
            tex_coord: [x / 16.0, y / 16.0],
            lightmap_coord: [0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            color: [255, 255, 255, 255],
        };
        let map = Q3Map {
            entities: String::new(),
            entity_records: vec![],
            shaders: vec![Q3Shader {
                name: "test/rock".to_string(),
                surface_flags: 0,
                content_flags: 0,
            }],
            planes: vec![Q3Plane {
                normal: [0.0, 0.0, 1.0],
                distance: 0.0,
            }],
            nodes: vec![],
            leaves: vec![],
            leaf_surfaces: vec![0],
            leaf_brushes: vec![],
            models: vec![],
            brushes: vec![],
            brush_sides: vec![],
            vertices: vec![vertex(0.0, 0.0), vertex(16.0, 0.0), vertex(0.0, 16.0)],
            indices: vec![0, 1, 2],
            fogs: vec![],
            surfaces: vec![Q3Surface {
                surface_type: Q3SurfaceType::Planar,
                shader: 0,
                fog: -1,
                first_vertex: 0,
                vertex_count: 3,
                first_index: 0,
                index_count: 3,
                lightmap: -3,
                lightmap_rect: [0, 0, 0, 0],
                lightmap_origin: [0.0, 0.0, 0.0],
                lightmap_vectors: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                patch_width: 0,
                patch_height: 0,
            }],
            lightmaps: vec![],
            light_grid: vec![],
            visibility: None,
        };
        let decoded = Q3DecodedWorld {
            map,
            planes: vec![Q3WorldPlane {
                normal: [0.0, 0.0, 1.0],
                distance: 0.0,
                plane_type: 2,
                signbits: 0,
            }],
            nodes: vec![],
            leaves: vec![Q3WorldLeaf {
                cluster: 0,
                area: 0,
                bounds: IntBounds {
                    min: [0, 0, 0],
                    max: [16, 16, 1],
                },
                surfaces: IndexRange { first: 0, count: 1 },
                brushes: IndexRange { first: 0, count: 0 },
            }],
            models: vec![Q3WorldModel {
                bounds: qa_content::common::Bounds {
                    min: [0.0, 0.0, 0.0],
                    max: [16.0, 16.0, 1.0],
                },
                surfaces: IndexRange { first: 0, count: 1 },
                brushes: IndexRange { first: 0, count: 0 },
            }],
            brushes: vec![],
            surfaces: vec![Q3WorldSurface {
                kind: Q3WorldSurfaceKind::Planar,
                shader: 0,
                fog: -1,
                vertices: IndexRange { first: 0, count: 3 },
                indices: IndexRange { first: 0, count: 3 },
                lightmap: Q3SurfaceLightmap {
                    image: -3,
                    rect: [0, 0, 0, 0],
                    origin: [0.0, 0.0, 0.0],
                    vectors: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                },
            }],
        };
        let mut scene = WorldScene::load_q3(
            &decoded,
            registry("world-q3", HashMap::new()),
            WorldSceneOptions::default(),
        )
        .expect("load");
        assert_eq!(scene.surfaces().len(), 1);
        let mut view_input = input(camera_at(vec3(8.0, 8.0, 64.0)));
        let prepared = scene.prepare_view(&mut view_input).expect("view");
        assert_eq!(prepared.view.operations.len(), 1);
        assert!(matches!(prepared.view.operations[0], RenderOperation::Draw(_)));
        let views = scene.prepare_views(&mut view_input, &[]).expect("views");
        assert_eq!(views.len(), 1);
    }
}
