//! Windowed scene presentation: live world state into draw batches.
//!
//! Donor provenance: `src/render/scene/world.ts` (`WorldScene.prepareView`:
//! world operations plus inline models plus extra input operations finished
//! into one ordered view) and `src/render/scene/models/renderer.ts`
//! (`SceneModelRenderer.prepare`: model/sprite entities into ordered draw
//! groups). The windowed composition keeps the decoded BSP world in a
//! [`WorldScene`], the map's model-bearing entity records as [`SceneEntity`]
//! values, and brush-model (`*N`) records as [`InlineModel`] entries; each
//! frame prepares the world view at the live camera and appends the model
//! draw groups' batches as one trailing [`RenderOperation::Draw`].
//!
//! Textures resolve through the installed product's mounts
//! ([`InstalledCatalog::read`]); model skins resolve through the world's own
//! texture loader ([`WindowedSkinProvider`](super::windowed_skins::WindowedSkinProvider)),
//! so skin registrations share the world image registry and their uploads
//! drain with the first frame view's image operations.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use qa_client::render::scene::models::light_sampler::ModelLightSampler;
use qa_client::render::scene::models::renderer::{
    ModelMaterialProvider, ModelViewInput, RenderFamily, SceneModelRenderer,
};
use qa_client::render::scene::models::types::{
    EntityFlags, EntityTransform, ModelPalette, ModelResource, ModelSourceOptions, SceneEntity, SceneModel, ScenePose,
};
use qa_client::render::scene::resources::SceneImageRegistry;
use qa_client::render::scene::shaders::SceneShaderRegistry;
use qa_client::render::scene::textures::{
    DecodedSceneImage, RgbaLevel, SceneAsset, SceneAssetReader, SceneImageDecoder, SceneTextureLoader,
};
use qa_client::render::scene::world::{InlineModel, WorldScene, WorldSceneOptions, WorldViewInput};
use qa_client::render::types::{
    DrawBatch, ImageResourceOperation, ImageSource, Palette as ClientPalette, RenderImage, RenderOperation, RenderView,
    RendererImage, ResourceOwner, SourceTime, TextureSampling, ViewTarget,
};
use qa_client::render::RenderError;
use qa_client::view::{ModelTransform, SceneCamera};
use qa_content::bsp::{read_q1_bsp, Q1BspOptions};
use qa_content::bsp2::decode_q2_map;
use qa_content::bsp3::decode_q3_world;
use qa_content::catalog::InstalledCatalog;
use qa_content::contract::{create_mount_plan_id, ResolvedMountPlan};
use qa_content::images::bmp::decode_bmp;
use qa_content::images::gif::decode_gif;
use qa_content::images::indexed::decode_pcx;
use qa_content::images::jpeg::decode_jpeg;
use qa_content::images::palette::decode_palette;
use qa_content::images::png::decode_png;
use qa_content::md2::parse_md2;
use qa_content::md3::parse_md3;
use qa_content::mdl::parse_mdl;
use qa_content::mounts::{open_mount_plan, MountedContent, OpenMountOptions};
use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::base::shared::items::item_list;
use qa_content::q3scene::to_scene_md3;
use qa_content::spr::{parse_sp2, parse_spr};
use qa_content::{classify_bsp, BspKind};
use qa_core::math::{angles_to_axis, vec3, vec4, Bounds, Vec3};

use super::play_world::PlayWorldError;
use super::windowed_shaders::{load_registry_scripts, read_shader_scripts, ShaderImageIndex};
use super::windowed_skins::WindowedSkinProvider;

/// Open one installed product's mounts once for a whole presentation.
///
/// Mount opening digests every archive, so the windowed run opens once
/// and reads textures and models through the opened mounts instead of
/// paying per-read opens through [`InstalledCatalog::read`].
pub(crate) fn open_product_mounts(
    catalog: &InstalledCatalog,
    content: &str,
    map: &str,
) -> Result<MountedContent, PlayWorldError> {
    let mounts = catalog.mounts_for(content).map_err(|error| PlayWorldError::MapUnread {
        content: content.to_string(),
        map: map.to_string(),
        reason: error.to_string(),
    })?;
    let plan = ResolvedMountPlan {
        id: create_mount_plan_id("windowed", content).map_err(|error| PlayWorldError::MapUnread {
            content: content.to_string(),
            map: map.to_string(),
            reason: error.to_string(),
        })?,
        mounts: mounts.clone(),
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        prefix_orders: Vec::new(),
    };
    open_mount_plan(&plan, OpenMountOptions::default()).map_err(|error| PlayWorldError::MapUnread {
        content: content.to_string(),
        map: map.to_string(),
        reason: error.to_string(),
    })
}

/// Texture reader over once-opened product mounts.
///
/// Missing paths read as absent (the loader falls back to its missing
/// image) so one absent texture never fails the windowed run.
struct CatalogSceneReader {
    mounts: MountedContent,
}

impl SceneAssetReader for CatalogSceneReader {
    fn read(&self, path: &str) -> Result<Option<SceneAsset>, RenderError> {
        let asset = self
            .mounts
            .open(path, |_| true)
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        Ok(asset.map(|asset| SceneAsset {
            bytes: asset.bytes,
            source: ImageSource::Resource {
                requested_path: asset.reference.requested_path,
            },
        }))
    }
}

/// Host image decoder over the ported content decoders (PNG, JPEG, BMP,
/// GIF); TGA and indexed formats keep the loader's local decoders.
struct WindowedImageDecoder;

impl SceneImageDecoder for WindowedImageDecoder {
    fn decode(&self, bytes: &[u8], path: &str) -> Result<DecodedSceneImage, RenderError> {
        let failed = |error: qa_content::images::ContentError| RenderError::Backend(error.to_string());
        let suffix = path.rsplit('.').next().unwrap_or("").to_lowercase();
        match suffix.as_str() {
            "png" => {
                let image = decode_png(bytes, path).map_err(failed)?;
                Ok(DecodedSceneImage::Rgba(RgbaLevel {
                    width: image.width,
                    height: image.height,
                    pixels: image.pixels,
                }))
            }
            "jpg" | "jpeg" => {
                let image = decode_jpeg(bytes, path).map_err(failed)?;
                Ok(DecodedSceneImage::Rgba(RgbaLevel {
                    width: image.width,
                    height: image.height,
                    pixels: image.pixels,
                }))
            }
            "bmp" => {
                let image = decode_bmp(bytes, path).map_err(failed)?;
                Ok(DecodedSceneImage::Rgba(RgbaLevel {
                    width: image.width,
                    height: image.height,
                    pixels: image.pixels,
                }))
            }
            "gif" => {
                let image = decode_gif(bytes, path).map_err(failed)?;
                Ok(DecodedSceneImage::Animated(
                    image
                        .frames
                        .into_iter()
                        .map(|frame| RgbaLevel {
                            width: frame.image.width,
                            height: frame.image.height,
                            pixels: frame.image.pixels,
                        })
                        .collect(),
                ))
            }
            _ => Err(RenderError::Backend(format!("Unsupported scene image format: {path}"))),
        }
    }
}

/// Prepare-time model material provider over the world's shared
/// white/missing handles and content palette.
///
/// Materials are preloaded through the world's texture loader before the
/// first frame ([`WindowedSkinProvider`](super::windowed_skins::WindowedSkinProvider)),
/// so the prepare path only reads the cached materials plus this
/// provider's family, palette, and fallbacks; the load entry points below
/// are unreachable after that preload and keep the white/missing fallbacks.
pub struct WindowedModelProvider {
    family: RenderFamily,
    palette: Option<ModelPalette>,
    white: RendererImage,
    missing: RendererImage,
}

impl ModelMaterialProvider for WindowedModelProvider {
    fn family(&self) -> RenderFamily {
        self.family
    }

    fn palette(&self) -> Option<&ModelPalette> {
        self.palette.as_ref()
    }

    fn white_image(&self) -> RendererImage {
        self.white.clone()
    }

    fn missing_image(&self) -> RendererImage {
        self.missing.clone()
    }

    fn register_indexed(
        &mut self,
        _name: &str,
        _image: RenderImage,
        _sampling: TextureSampling,
    ) -> Result<RendererImage, RenderError> {
        Ok(self.white.clone())
    }

    fn load_external(&mut self, _path: &str, _sprite: bool) -> Result<Option<RendererImage>, RenderError> {
        Ok(Some(self.white.clone()))
    }

    fn shader_image(&mut self, _name: &str) -> Result<Option<RendererImage>, RenderError> {
        Ok(Some(self.white.clone()))
    }
}

/// Player spawn point: camera origin plus pitch/yaw/roll angles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnPoint {
    /// Camera origin at eye height (feet origin plus the family eye offset).
    pub origin: Vec3,
    /// Camera angles (pitch, yaw, roll) in degrees.
    pub angles: Vec3,
}

/// Standing eye height above the spawn feet origin, in map units (donor
/// `viewOffset.z` 22 for Quake, `viewHeight` 22 for Quake II,
/// `standingViewHeight` 26 for Quake III).
fn eye_height(kind: BspKind) -> f32 {
    match kind {
        BspKind::Q1 | BspKind::Q2 => 22.0,
        BspKind::Q3 => 26.0,
    }
}

/// One model-bearing record that did not become a scene entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedModel {
    /// Zero-based record index in the parsed entity string.
    pub index: usize,
    /// Record classname.
    pub classname: String,
    /// Record `model` value.
    pub model: String,
    /// Why the record produced no scene entity.
    pub reason: String,
}

/// Windowed presentation: decoded world plus model-bearing map entities.
pub struct PlayPresentation {
    scene: WorldScene,
    models: SceneModelRenderer<WindowedModelProvider>,
    entities: Vec<SceneEntity>,
    inline_models: Vec<InlineModel>,
    spawn: Option<SpawnPoint>,
    q3_world: bool,
    q2_world: bool,
    skipped_models: Vec<SkippedModel>,
}

impl std::fmt::Debug for PlayPresentation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlayPresentation")
            .field("surfaces", &self.scene.surfaces().len())
            .field("entities", &self.entities.len())
            .field("inline_models", &self.inline_models.len())
            .field("spawn", &self.spawn)
            .field("skipped_models", &self.skipped_models)
            .finish()
    }
}

impl PlayPresentation {
    /// Prepared world surfaces.
    #[must_use]
    pub fn surface_count(&self) -> usize {
        self.scene.surfaces().len()
    }

    /// Model-bearing scene entities.
    #[must_use]
    pub fn entities(&self) -> &[SceneEntity] {
        &self.entities
    }

    /// Brush-model (`*N`) submissions.
    #[must_use]
    pub fn inline_models(&self) -> &[InlineModel] {
        &self.inline_models
    }

    /// Player spawn point, when a spawn record parsed.
    #[must_use]
    pub fn spawn(&self) -> Option<SpawnPoint> {
        self.spawn
    }

    /// Model-bearing records that produced no scene entity.
    #[must_use]
    pub fn skipped_models(&self) -> &[SkippedModel] {
        &self.skipped_models
    }

    /// Prepare the model/sprite batches for the map's model-bearing
    /// entities at `camera` (donor `SceneModelRenderer.prepare`).
    pub fn prepare_entity_batches(&self, camera: SceneCamera, time: SourceTime) -> Result<Vec<DrawBatch>, String> {
        if self.entities.is_empty() {
            return Ok(Vec::new());
        }
        let time_seconds = match time {
            SourceTime::Seconds(seconds) => seconds,
            SourceTime::Milliseconds(millis) => millis / 1000.0,
        };
        let model_input = ModelViewInput {
            camera,
            time_seconds,
            dynamic_lights: Vec::new(),
            q2_lights: Vec::new(),
            q2_atlas: None,
            q3_lights: Vec::new(),
            identity_light: 1.0,
            q3_world: self.q3_world,
            q2_world: self.q2_world,
        };
        let groups = self
            .models
            .prepare(&self.entities, &model_input, &|_| ModelSourceOptions::default(), None)
            .map_err(|error| error.to_string())?;
        Ok(groups.into_iter().flat_map(|group| group.batches).collect())
    }

    /// Prepare one frame view: the world view at `camera` (donor
    /// `prepareView`: world operations plus inline models, finished) with
    /// the model draw groups' batches appended as one trailing draw, plus
    /// the image uploads the backend must apply before executing the view.
    pub fn prepare_frame_view(
        &mut self,
        camera: SceneCamera,
        target: ViewTarget,
        time: SourceTime,
    ) -> Result<(RenderView, Vec<ImageResourceOperation>), String> {
        let mut input = WorldViewInput::new(camera, target, time);
        input.inline_models.clone_from(&self.inline_models);
        let prepared = self.scene.prepare_view(&mut input).map_err(|error| error.to_string())?;
        let mut view = prepared.view;
        let image_operations = prepared.image_operations;
        let batches = self.prepare_entity_batches(camera, time)?;
        if !batches.is_empty() {
            view.operations.push(RenderOperation::Draw(batches));
        }
        Ok((view, image_operations))
    }
}

/// Parse a space-separated triple into a vector.
fn parse_triple(value: &str) -> Option<Vec3> {
    let mut parts = value.split_whitespace();
    let (x, y, z) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some(vec3(x.parse().ok()?, y.parse().ok()?, z.parse().ok()?))
}

/// Look up one record property.
fn record_get<'a>(record: &'a [(String, String)], key: &str) -> Option<&'a str> {
    record
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// Parse record angles: the `angles` triple, else the `angle` yaw (`-1` up,
/// `-2` down), else facing along +X.
fn record_angles(record: &[(String, String)]) -> Vec3 {
    if let Some(angles) = record_get(record, "angles").and_then(parse_triple) {
        return angles;
    }
    match record_get(record, "angle").and_then(|angle| angle.parse::<f32>().ok()) {
        Some(-1.0) => vec3(-90.0, 0.0, 0.0),
        Some(-2.0) => vec3(90.0, 0.0, 0.0),
        Some(yaw) => vec3(0.0, yaw, 0.0),
        None => vec3(0.0, 0.0, 0.0),
    }
}

/// Parse a brush-model reference (`*N`) into its model index.
fn inline_model_index(model: &str) -> Option<usize> {
    model.strip_prefix('*')?.parse::<usize>().ok()
}

/// Lowercased file extension of a model path.
fn model_extension(model: &str) -> Option<String> {
    model.rsplit('.').next().map(str::to_lowercase)
}

/// Spawn-record priority: starts first, then deathmatch, coop, intermission.
fn spawn_priority(classname: &str) -> Option<u32> {
    match classname {
        "info_player_start" => Some(0),
        "info_player_deathmatch" => Some(1),
        "info_player_coop" => Some(2),
        "info_player_intermission" => Some(3),
        _ => None,
    }
}

/// Read the worldspawn `sky` key naming the Q2 skybox (donor servers
/// pass the worldspawn sky to the world scene).
fn worldspawn_sky(records: &[Vec<(String, String)>]) -> Option<String> {
    records
        .iter()
        .find(|record| record_get(record, "classname") == Some("worldspawn"))
        .and_then(|record| record_get(record, "sky"))
        .map(str::to_string)
}

/// Select the player spawn point: the highest-priority spawn record with a
/// parseable origin (first wins ties). Records without a classname, without
/// a spawn priority, or without a parseable origin are skipped, never
/// fatal. The returned origin is at eye height (`eye_height` above the
/// record feet origin).
///
/// Quake II starts carry a `targetname` naming the re-entry point for
/// travelers arriving from another map; the map-entry spawn is the
/// untargeted `info_player_start` (donor `selectQ2Spawn` with an empty
/// spawn point, source `SelectSpawnPoint`). A targeted start still wins
/// over deathmatch/coop/intermission records when no untargeted start
/// parses, but loses to an untargeted start regardless of record order.
/// Other families ignore `targetname` on starts (first record wins).
pub fn select_spawn(records: &[Vec<(String, String)>], kind: BspKind) -> Option<SpawnPoint> {
    let mut best: Option<(u32, bool, SpawnPoint)> = None;
    for record in records {
        let Some(classname) = record_get(record, "classname") else {
            continue;
        };
        let Some(priority) = spawn_priority(classname) else {
            continue;
        };
        let targeted = matches!(kind, BspKind::Q2)
            && classname == "info_player_start"
            && record_get(record, "targetname").is_some_and(|name| !name.is_empty());
        if best.is_some_and(|(best_priority, best_targeted, _)| (best_priority, best_targeted) <= (priority, targeted))
        {
            continue;
        }
        let Some(feet) = record_get(record, "origin").and_then(parse_triple) else {
            continue;
        };
        let origin = vec3(feet.x, feet.y, feet.z + eye_height(kind));
        best = Some((
            priority,
            targeted,
            SpawnPoint {
                origin,
                angles: record_angles(record),
            },
        ));
    }
    best.map(|(_, _, spawn)| spawn)
}

/// Select the Q1 player spawn: stock `SelectSpawnPoint` (`client.qc:454`):
/// a flagged run (any `serverflags` bit) returns through
/// `info_player_start2` when one parses, otherwise the generic pick.
/// Deathmatch/coop last-spawn cycling lands with the spawn-rotation slice.
pub fn select_q1_spawn(records: &[Vec<(String, String)>], serverflags: i32) -> Option<SpawnPoint> {
    if serverflags != 0 {
        for record in records {
            if record_get(record, "classname") != Some("info_player_start2") {
                continue;
            }
            let Some(feet) = record_get(record, "origin").and_then(parse_triple) else {
                continue;
            };
            return Some(SpawnPoint {
                origin: vec3(feet.x, feet.y, feet.z + eye_height(BspKind::Q1)),
                angles: record_angles(record),
            });
        }
    }
    select_spawn(records, BspKind::Q1)
}

/// Digest model bytes for the scene-entity resource identity.
fn digest_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Read one model file through the opened mounts.
fn read_model_bytes(mounts: &MountedContent, model: &str) -> Result<Vec<u8>, String> {
    match mounts.open(model, |_| true) {
        Ok(Some(asset)) => Ok(asset.bytes),
        Ok(None) => Err(format!("model not installed: {model}")),
        Err(error) => Err(format!("cannot read {model}: {error}")),
    }
}

/// Decode one model path into a scene model by file extension.
fn decode_scene_model(mounts: &MountedContent, model: &str, map: &str) -> Result<(SceneModel, u64), String> {
    let bytes = read_model_bytes(mounts, model)?;
    let source = format!("{map}:{model}");
    let scene_model = match model_extension(model).as_deref() {
        Some("md3") => {
            let decoded = parse_md3(&bytes, &source).map_err(|error| error.to_string())?;
            SceneModel::Q3Md3(to_scene_md3(decoded.model))
        }
        Some("md2") => {
            let decoded = parse_md2(&bytes, &source).map_err(|error| error.to_string())?;
            SceneModel::Q2Md2 {
                model: decoded,
                replacement: None,
            }
        }
        Some("mdl") => {
            let decoded = parse_mdl(&bytes, &source).map_err(|error| error.to_string())?;
            let radius = decoded.bounding_radius.max(1.0);
            SceneModel::Q1Mdl {
                bounds: Bounds {
                    min: vec3(-radius, -radius, -radius),
                    max: vec3(radius, radius, radius),
                },
                flags: decoded.flags,
                model: decoded,
                replacement: None,
            }
        }
        Some("spr") => {
            let decoded = parse_spr(&bytes, &source).map_err(|error| error.to_string())?;
            SceneModel::Q1Spr(decoded)
        }
        Some("sp2") => {
            let decoded = parse_sp2(&bytes, &source).map_err(|error| error.to_string())?;
            SceneModel::Q2Sp2(decoded)
        }
        _ => return Err(format!("unsupported model type: {model}")),
    };
    Ok((scene_model, digest_bytes(&bytes)))
}

/// Build one scene entity from a model-bearing record.
#[allow(clippy::too_many_arguments)]
fn build_scene_entity(
    mounts: &MountedContent,
    map: &str,
    index: usize,
    record: &[(String, String)],
    model: &str,
    flags: EntityFlags,
) -> Result<SceneEntity, String> {
    let (scene_model, digest) = decode_scene_model(mounts, model, map)?;
    let origin = record_get(record, "origin")
        .and_then(parse_triple)
        .unwrap_or(vec3(0.0, 0.0, 0.0));
    let skin = record_get(record, "skin")
        .and_then(|skin| skin.parse().ok())
        .unwrap_or(0);
    Ok(SceneEntity {
        resource: ModelResource {
            id: format!("windowed:{map}:{index}"),
            requested_path: model.to_string(),
            digest,
        },
        model: scene_model,
        pose: ScenePose::Frame {
            frame: 0,
            previous_frame: 0,
            back_lerp: 0.0,
        },
        transform: EntityTransform {
            origin,
            axis: angles_to_axis(record_angles(record)),
            scale: vec3(1.0, 1.0, 1.0),
        },
        previous_origin: origin,
        lighting_origin: origin,
        color: vec4(1.0, 1.0, 1.0, 1.0),
        skin,
        shader_time_seconds: 0.0,
        flags,
        attachments: Vec::new(),
        actor_slot: None,
    })
}

/// Source palette for a map family (donor `paletteFor`): none for Q3,
/// `gfx/palette.lmp` for Q1, the `pics/colormap.pcx` palette for Q2.
fn load_palette(mounts: &MountedContent, kind: BspKind, map: &str) -> Result<Option<ClientPalette>, PlayWorldError> {
    let failed = |reason: String| PlayWorldError::Presentation {
        map: map.to_string(),
        reason,
    };
    match kind {
        BspKind::Q3 => Ok(None),
        BspKind::Q1 => {
            let path = "gfx/palette.lmp";
            let asset = mounts.open(path, |_| true).map_err(|error| failed(error.to_string()))?;
            let Some(asset) = asset else {
                return Err(failed(format!("Quake content has no {path}")));
            };
            let palette = decode_palette(&asset.bytes, &asset.reference.requested_path)
                .map_err(|error| failed(error.to_string()))?;
            Ok(Some(ClientPalette {
                colors: palette.colors,
                source: palette.source,
            }))
        }
        BspKind::Q2 => {
            let path = "pics/colormap.pcx";
            let asset = mounts.open(path, |_| true).map_err(|error| failed(error.to_string()))?;
            let Some(asset) = asset else {
                return Err(failed(format!("Quake II content has no {path}")));
            };
            let decoded = decode_pcx(&asset.bytes, path).map_err(|error| failed(error.to_string()))?;
            let Some(colors) = decoded.palette else {
                return Err(failed(format!("Quake II colormap has no palette: {path}")));
            };
            Ok(Some(ClientPalette {
                colors,
                source: path.to_string(),
            }))
        }
    }
}

/// Resolve a Q3 item/weapon classname to the item list's primary world
/// model (donor gamecode bind: `G_SpawnItem` assigns `world_model[0]`).
fn q3_item_model(classname: &str) -> Option<&'static str> {
    item_list(Product::Baseq3)
        .iter()
        .find(|item| item.class_name == Some(classname))
        .and_then(|item| item.world_models[0])
}

/// Build scene entities and inline models from parsed entity records.
///
/// Records whose `model` is `*N` (with `N > 0`) become inline-model
/// submissions through the world scene's `prepareModel` path (donor
/// `prepareView`); records naming model files become [`SceneEntity`]
/// values through the scene models machinery. Q3 item/weapon records
/// carry no `model` key (the gamecode bind assigns it), so they resolve
/// through the ported item list instead; other model-less records have no
/// visual and are skipped, matching the donor (only modeled entities
/// submit). `*0` is the world itself and is likewise skipped.
fn build_scene_entities(
    mounts: &MountedContent,
    map: &str,
    records: &[Vec<(String, String)>],
    flags: EntityFlags,
    is_q3: bool,
) -> (Vec<SceneEntity>, Vec<InlineModel>, Vec<SkippedModel>) {
    let mut entities = Vec::new();
    let mut inline_models = Vec::new();
    let mut skipped = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let classname = record_get(record, "classname").unwrap_or("").to_string();
        let Some(model) = record_get(record, "model") else {
            if is_q3 {
                if let Some(item_model) = q3_item_model(&classname) {
                    match build_scene_entity(mounts, map, index, record, item_model, flags) {
                        Ok(entity) => entities.push(entity),
                        Err(reason) => skipped.push(SkippedModel {
                            index,
                            classname,
                            model: item_model.to_string(),
                            reason,
                        }),
                    }
                }
            }
            continue;
        };
        if let Some(inline) = inline_model_index(model) {
            if inline == 0 {
                continue;
            }
            let origin = record_get(record, "origin")
                .and_then(parse_triple)
                .unwrap_or(vec3(0.0, 0.0, 0.0));
            inline_models.push(InlineModel {
                model: inline,
                transform: ModelTransform {
                    origin,
                    axis: angles_to_axis(record_angles(record)),
                    scale: 1.0,
                },
                animation_frame: None,
                alternate_animation: None,
                casts_shadow: false,
                entity_rgba: None,
            });
            continue;
        }
        match build_scene_entity(mounts, map, index, record, model, flags) {
            Ok(entity) => entities.push(entity),
            Err(reason) => skipped.push(SkippedModel {
                index,
                classname,
                model: model.to_string(),
                reason,
            }),
        }
    }
    (entities, inline_models, skipped)
}

/// Build the windowed presentation for one map: decode the BSP world for
/// the map's family into a [`WorldScene`], then map the parsed entity
/// records into scene entities, inline models, and a spawn point. Takes
/// ownership of the once-opened product mounts; the texture reader keeps
/// them for the presentation's lifetime.
pub fn build_presentation(
    mounts: MountedContent,
    map: &str,
    bytes: &[u8],
    records: &[Vec<(String, String)>],
    owner: ResourceOwner,
) -> Result<PlayPresentation, PlayWorldError> {
    let kind = classify_bsp(bytes, map).map_err(|error| PlayWorldError::MapDecode {
        map: map.to_string(),
        reason: error.to_string(),
    })?;
    let (family, flags, q3_world, q2_world) = match kind {
        BspKind::Q1 => (RenderFamily::Q1, EntityFlags::Q1 { bits: 0 }, false, false),
        BspKind::Q2 => (RenderFamily::Q2, EntityFlags::Q2 { bits: 0 }, false, true),
        BspKind::Q3 => (RenderFamily::Q3, EntityFlags::Q3 { bits: 0 }, true, false),
    };
    let (entities, inline_models, skipped_models) =
        build_scene_entities(&mounts, map, records, flags, matches!(kind, BspKind::Q3));
    let palette = load_palette(&mounts, kind, map)?;
    let model_palette = palette.as_ref().map(|palette| ModelPalette {
        colors: palette.colors.clone(),
        source: palette.source.clone(),
    });
    // Authored `.shader` scripts load before the world scene builds, so
    // glow/transparency/anim surfaces register their authored stages
    // instead of implicit materials over the missing handle.
    let scripts = read_shader_scripts(&mounts).map_err(|error| PlayWorldError::Presentation {
        map: map.to_string(),
        reason: error.to_string(),
    })?;
    let skin_index = ShaderImageIndex::build(&scripts).map_err(|error| PlayWorldError::Presentation {
        map: map.to_string(),
        reason: error.to_string(),
    })?;
    let mut loader = SceneTextureLoader::new(
        SceneImageRegistry::new(owner),
        Box::new(CatalogSceneReader { mounts }),
        palette,
        None,
        224,
    )
    .map_err(|error| PlayWorldError::Presentation {
        map: map.to_string(),
        reason: error.to_string(),
    })?;
    loader.set_decoder(Box::new(WindowedImageDecoder));
    let white = loader.white().image.clone();
    let missing = loader.missing().image.clone();
    let mut registry = SceneShaderRegistry::with_defaults(loader);
    load_registry_scripts(&mut registry, &scripts, matches!(kind, BspKind::Q3)).map_err(|error| {
        PlayWorldError::Presentation {
            map: map.to_string(),
            reason: error.to_string(),
        }
    })?;
    let options = WorldSceneOptions {
        q2_sky_name: if matches!(kind, BspKind::Q2) {
            worldspawn_sky(records)
        } else {
            None
        },
        ..WorldSceneOptions::default()
    };
    let mut scene = match kind {
        BspKind::Q1 => {
            let parsed =
                read_q1_bsp(bytes, map, Q1BspOptions::default()).map_err(|error| PlayWorldError::MapDecode {
                    map: map.to_string(),
                    reason: error.to_string(),
                })?;
            WorldScene::load_q1(&parsed, registry, options).map_err(|error| PlayWorldError::Presentation {
                map: map.to_string(),
                reason: error.to_string(),
            })?
        }
        BspKind::Q2 => {
            let parsed = decode_q2_map(bytes, map, None).map_err(|error| PlayWorldError::MapDecode {
                map: map.to_string(),
                reason: error.to_string(),
            })?;
            WorldScene::load_q2(&parsed, registry, options).map_err(|error| PlayWorldError::Presentation {
                map: map.to_string(),
                reason: error.to_string(),
            })?
        }
        BspKind::Q3 => {
            let parsed = decode_q3_world(bytes, map).map_err(|error| PlayWorldError::MapDecode {
                map: map.to_string(),
                reason: error.to_string(),
            })?;
            WorldScene::load_q3(&parsed, registry, options).map_err(|error| PlayWorldError::Presentation {
                map: map.to_string(),
                reason: error.to_string(),
            })?
        }
    };
    let mut models = SceneModelRenderer::new(
        WindowedModelProvider {
            family,
            palette: model_palette,
            white,
            missing,
        },
        ModelLightSampler::fullbright(),
    );
    models.set_world(q3_world, q2_world, 1.0);
    let skin_palette = models.provider().palette().cloned();
    let delegate = WindowedSkinProvider::new(scene.shaders_mut().textures_mut(), family, skin_palette)
        .with_authored_index(skin_index);
    models
        .preload_with(delegate, &entities, &|_| ModelSourceOptions::default())
        .map_err(|error| PlayWorldError::Presentation {
            map: map.to_string(),
            reason: error.to_string(),
        })?;
    Ok(PlayPresentation {
        scene,
        models,
        entities,
        inline_models,
        spawn: select_spawn(records, kind),
        q3_world,
        q2_world,
        skipped_models,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn q1_spawn_prefers_start2_when_flagged() {
        let records = vec![
            record(&[("classname", "worldspawn")]),
            record(&[
                ("classname", "info_player_start"),
                ("origin", "544 288 32"),
                ("angle", "90"),
            ]),
            record(&[
                ("classname", "info_player_start2"),
                ("origin", "544 1536 32"),
                ("angle", "90"),
            ]),
        ];
        let spawn = select_q1_spawn(&records, 0).expect("spawn");
        assert_eq!(spawn.origin, vec3(544.0, 288.0, 54.0));
        let spawn = select_q1_spawn(&records, 1).expect("spawn");
        assert_eq!(spawn.origin, vec3(544.0, 1536.0, 54.0));
        assert_eq!(spawn.angles, vec3(0.0, 90.0, 0.0));
        // Flagged without a start2 falls back to the generic pick.
        let records = vec![
            record(&[("classname", "worldspawn")]),
            record(&[("classname", "info_player_start"), ("origin", "1 2 3")]),
        ];
        let spawn = select_q1_spawn(&records, 8).expect("spawn");
        assert_eq!(spawn.origin, vec3(1.0, 2.0, 25.0));
    }

    #[test]
    fn spawn_prefers_starts_over_deathmatch() {
        let records = vec![
            record(&[("classname", "worldspawn")]),
            record(&[
                ("classname", "info_player_deathmatch"),
                ("origin", "1 2 3"),
                ("angle", "90"),
            ]),
            record(&[
                ("classname", "info_player_start"),
                ("origin", "4 5 6"),
                ("angle", "180"),
            ]),
            record(&[("classname", "info_player_start"), ("origin", "7 8 9")]),
        ];
        let spawn = select_spawn(&records, BspKind::Q1).expect("spawn");
        assert_eq!(spawn.origin, vec3(4.0, 5.0, 6.0 + 22.0));
        assert_eq!(spawn.angles, vec3(0.0, 180.0, 0.0));
    }

    #[test]
    fn spawn_prefers_untargeted_q2_starts() {
        // base1 record order: coop and deathmatch spots first, then the
        // `base2` re-entry start, then the untargeted map-entry start.
        // Picking the first start spawned at the exit end of the map.
        let records = vec![
            record(&[("classname", "worldspawn")]),
            record(&[
                ("classname", "info_player_coop"),
                ("origin", "32 -224 24"),
                ("angle", "90"),
            ]),
            record(&[
                ("classname", "info_player_deathmatch"),
                ("origin", "-392 840 -104"),
                ("angle", "0"),
            ]),
            record(&[
                ("classname", "info_player_start"),
                ("targetname", "base2"),
                ("origin", "-1768 1536 128"),
                ("angle", "0"),
            ]),
            record(&[
                ("classname", "info_player_start"),
                ("origin", "128 -320 32"),
                ("angle", "135"),
            ]),
        ];
        let spawn = select_spawn(&records, BspKind::Q2).expect("spawn");
        assert_eq!(spawn.origin, vec3(128.0, -320.0, 32.0 + 22.0));
        assert_eq!(spawn.angles, vec3(0.0, 135.0, 0.0));
    }

    #[test]
    fn spawn_falls_back_to_targeted_q2_start() {
        let records = vec![
            record(&[
                ("classname", "info_player_start"),
                ("targetname", "base2"),
                ("origin", "-1768 1536 128"),
                ("angle", "0"),
            ]),
            record(&[
                ("classname", "info_player_deathmatch"),
                ("origin", "1 2 3"),
                ("angle", "90"),
            ]),
        ];
        let spawn = select_spawn(&records, BspKind::Q2).expect("spawn");
        assert_eq!(spawn.origin, vec3(-1768.0, 1536.0, 128.0 + 22.0));
        assert_eq!(spawn.angles, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn spawn_ignores_targetname_for_q1_and_q3() {
        let records = vec![
            record(&[
                ("classname", "info_player_start"),
                ("targetname", "other"),
                ("origin", "1 2 3"),
                ("angle", "0"),
            ]),
            record(&[
                ("classname", "info_player_start"),
                ("origin", "4 5 6"),
                ("angle", "180"),
            ]),
        ];
        let q1 = select_spawn(&records, BspKind::Q1).expect("q1 spawn");
        assert_eq!(q1.origin, vec3(1.0, 2.0, 3.0 + 22.0));
        let q3 = select_spawn(&records, BspKind::Q3).expect("q3 spawn");
        assert_eq!(q3.origin, vec3(1.0, 2.0, 3.0 + 26.0));
    }

    #[test]
    fn spawn_eye_height_follows_family() {
        let records = vec![
            record(&[("classname", "worldspawn")]),
            record(&[("classname", "info_player_start"), ("origin", "0 0 100")]),
        ];
        assert_eq!(eye_height(BspKind::Q1), 22.0);
        assert_eq!(eye_height(BspKind::Q2), 22.0);
        assert_eq!(eye_height(BspKind::Q3), 26.0);
        let q1 = select_spawn(&records, BspKind::Q1).expect("q1 spawn");
        assert_eq!(q1.origin, vec3(0.0, 0.0, 122.0));
        let q3 = select_spawn(&records, BspKind::Q3).expect("q3 spawn");
        assert_eq!(q3.origin, vec3(0.0, 0.0, 126.0));
    }

    #[test]
    fn spawn_angles_parse_triples_and_special_yaws() {
        let triple = record(&[("angles", "10 20 30")]);
        assert_eq!(record_angles(&triple), vec3(10.0, 20.0, 30.0));
        let yaw = record(&[("angle", "45")]);
        assert_eq!(record_angles(&yaw), vec3(0.0, 45.0, 0.0));
        let up = record(&[("angle", "-1")]);
        assert_eq!(record_angles(&up), vec3(-90.0, 0.0, 0.0));
        let down = record(&[("angle", "-2")]);
        assert_eq!(record_angles(&down), vec3(90.0, 0.0, 0.0));
        let missing = record(&[("classname", "info_player_start")]);
        assert_eq!(record_angles(&missing), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn spawn_skips_records_without_origins() {
        let records = vec![
            record(&[("classname", "info_player_start")]),
            record(&[("classname", "info_player_deathmatch"), ("origin", "bogus")]),
        ];
        assert!(select_spawn(&records, BspKind::Q2).is_none());
    }

    #[test]
    fn spawn_skips_worldspawn_and_non_spawn_records() {
        // BSP entity strings always lead with worldspawn; non-spawn
        // records must never abort the scan (regression: the windowed
        // camera used to fall back to the world origin on every map).
        let records = vec![
            record(&[("classname", "worldspawn"), ("sky", "unit1_")]),
            record(&[("classname", "light"), ("origin", "9 9 9")]),
            record(&[
                ("classname", "info_player_deathmatch"),
                ("origin", "216 1328 24"),
                ("angle", "270"),
            ]),
            record(&[("classname", "trigger_multiple"), ("origin", "1 1 1")]),
            record(&[("classname", "info_player_deathmatch"), ("origin", "7 8 9")]),
        ];
        let spawn = select_spawn(&records, BspKind::Q3).expect("spawn past worldspawn");
        assert_eq!(spawn.origin, vec3(216.0, 1328.0, 24.0 + 26.0));
        assert_eq!(spawn.angles, vec3(0.0, 270.0, 0.0));
    }

    #[test]
    fn spawn_skips_unparseable_origins_and_continues() {
        let records = vec![
            record(&[("classname", "info_player_deathmatch"), ("origin", "bogus")]),
            record(&[("classname", "info_player_deathmatch")]),
            record(&[("classname", "info_player_deathmatch"), ("origin", "1 2 3")]),
        ];
        let spawn = select_spawn(&records, BspKind::Q1).expect("spawn past bad origins");
        assert_eq!(spawn.origin, vec3(1.0, 2.0, 3.0 + 22.0));
    }

    #[test]
    fn q3_item_classnames_resolve_to_world_models() {
        assert_eq!(
            q3_item_model("weapon_rocketlauncher"),
            Some("models/weapons2/rocketl/rocketl.md3")
        );
        assert_eq!(q3_item_model("ammo_rockets"), Some("models/powerups/ammo/rocketam.md3"));
        assert_eq!(q3_item_model("info_player_start"), None);
        assert_eq!(q3_item_model("light"), None);
    }

    #[test]
    fn worldspawn_sky_reads_the_sky_key() {
        let records = vec![
            record(&[("classname", "worldspawn"), ("sky", "unit1_")]),
            record(&[("classname", "info_player_start"), ("origin", "0 0 0")]),
        ];
        assert_eq!(worldspawn_sky(&records), Some("unit1_".to_string()));
        let none = vec![record(&[("classname", "worldspawn")])];
        assert_eq!(worldspawn_sky(&none), None);
    }

    #[test]
    fn inline_model_references_parse() {
        assert_eq!(inline_model_index("*1"), Some(1));
        assert_eq!(inline_model_index("*0"), Some(0));
        assert_eq!(inline_model_index("models/ammo.md3"), None);
        assert_eq!(inline_model_index("*bogus"), None);
    }

    #[test]
    fn unsupported_models_skip_without_aborting() {
        let mounts = open_mount_plan(
            &ResolvedMountPlan {
                id: create_mount_plan_id("windowed-test", "scene").unwrap(),
                mounts: Vec::new(),
                default_order: Vec::new(),
                prefix_orders: Vec::new(),
            },
            OpenMountOptions::default(),
        )
        .unwrap();
        let records = vec![
            record(&[("classname", "worldspawn"), ("model", "*0")]),
            record(&[("classname", "func_door"), ("model", "*3"), ("origin", "0 0 0")]),
            record(&[("classname", "item_armor"), ("model", "models/armor.md3")]),
            record(&[("classname", "light")]),
        ];
        let (entities, inline_models, skipped) =
            build_scene_entities(&mounts, "maps/q3dm1.bsp", &records, EntityFlags::Q3 { bits: 0 }, true);
        assert!(entities.is_empty());
        assert_eq!(inline_models.len(), 1);
        assert_eq!(inline_models[0].model, 3);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].index, 2);
        assert_eq!(skipped[0].classname, "item_armor");
    }
}
