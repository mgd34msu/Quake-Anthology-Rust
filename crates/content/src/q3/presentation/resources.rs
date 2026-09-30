//! Quake III presentation: resources.
//!
//! Donor provenance: `src/content/q3/presentation/resources.ts`.

use qa_core::math::{dot3, Bounds, Plane, Vec3};
use std::collections::HashMap;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_client::*;
use crate::q3::presentation::retail_snapshot::*;

// ---------------------------------------------------------------------------
// Renderer resources (resources.ts)
// ---------------------------------------------------------------------------

/// Asset reader (`AssetReader`).
pub trait AssetReader {
    /// Read an asset's bytes.
    fn read_asset(&mut self, path: &str) -> PresentResult<Vec<u8>>;
    /// Whether an asset exists.
    fn has_asset(&self, path: &str) -> bool;
    /// List asset paths under a prefix.
    fn list_assets(&self, prefix: Option<&str>) -> Vec<String>;
}

/// Sound asset reader (`SoundAssetReader`).
pub trait SoundAssetReader: AssetReader {
    /// Byte length of a sound asset.
    fn read_file_length(&mut self, path: &str) -> PresentResult<usize>;
    /// Synchronously read a preloaded sound asset.
    fn read_asset_sync(&self, path: &str) -> PresentResult<Vec<u8>>;
}

/// Client sound bank (`ClientSoundBank`).
pub trait ClientSoundBank {
    /// Register a sound path.
    fn register_bank_sound(&mut self, path: Option<&str>, compressed: bool) -> PresentResult<Option<PcmSound>>;
    /// Bank index for a sound handle.
    fn index_for_sound(&self, sound: Option<PcmSound>) -> i32;
}

/// Loaded world scene (`WorldScene`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldScene {
    /// Inline model bounds.
    pub model_bounds: Vec<Bounds>,
}

/// Resource BSP child (`BspChild`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceBspChild {
    /// Leaf child.
    Leaf {
        /// Leaf index.
        index: usize,
    },
    /// Node child.
    Node {
        /// Node index.
        index: usize,
    },
}

/// Resource BSP node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBspNode {
    /// Plane index.
    pub plane: usize,
    /// Children (`[front, back]`).
    pub children: [ResourceBspChild; 2],
}

/// Resource BSP leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBspLeaf {
    /// PVS cluster.
    pub cluster: i32,
}

/// Resource world map (`Pick<Q3WorldGeometry, ...>`).
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceWorldMap {
    /// Entity lump text.
    pub entities: String,
    /// BSP nodes.
    pub nodes: Vec<ResourceBspNode>,
    /// BSP leaves.
    pub leaves: Vec<ResourceBspLeaf>,
    /// BSP planes.
    pub planes: Vec<Plane>,
}

/// Resource collision world (`Q3ResourceWorld`).
///
/// The donor's `clusterPVS` object folds into one byte lookup; `inPVS` only
/// ever reads a single visibility byte.
pub trait ResourceWorld {
    /// World map geometry.
    fn resource_map(&self) -> &ResourceWorldMap;
    /// Visibility byte for a cluster PVS row.
    fn cluster_pvs_byte(&self, cluster: i32, offset: usize) -> u8;
}

/// Renderer-owned resources (`RendererResources`).
pub trait RendererResources {
    /// Register a model path.
    fn register_model(&mut self, path: Option<&str>) -> PresentResult<SceneModel>;
    /// Register a skin path.
    fn register_skin(&mut self, path: &str) -> PresentResult<Option<SceneSkin>>;
    /// Register a mipmapped shader.
    fn register_shader(&mut self, path: &str) -> PresentResult<Option<SceneShader>>;
    /// Register an unmipmapped shader.
    fn register_shader_no_mip(&mut self, path: Option<&str>) -> PresentResult<Option<SceneShader>>;
    /// Resolve a shader's material picture.
    fn picture(&self, shader: Option<&SceneShader>) -> PresentResult<MaterialPicture>;
    /// Integer handle for a model.
    fn model_handle(&self, model: &SceneModel) -> PresentResult<i32>;
    /// Model for an integer handle.
    fn model_for_handle(&self, handle: i32) -> PresentResult<SceneModel>;
    /// Shader for an integer handle.
    fn shader_for_handle(&self, handle: i32) -> PresentResult<Option<SceneShader>>;
    /// Clear the scene.
    fn clear_scene(&mut self);
    /// Submit a scene entity.
    fn add_ref_entity(&mut self, entity: RefEntity);
    /// Submit a scene polygon.
    fn add_poly(&mut self, poly: RefPoly);
    /// Submit a dynamic light.
    fn add_light(&mut self, light: DynamicLight);
    /// Remap a shader.
    fn remap_shader(&mut self, original: &str, replacement: &str, offset: &str) -> PresentResult<()>;
    /// Render a view definition.
    fn render_scene(&mut self, refdef: &Refdef);
    /// Load a world scene.
    fn load_world(&mut self, path: &str) -> PresentResult<WorldScene>;
}

/// Renderer-resource host (`Q3ResourceHost`).
///
/// The donor's `scene` recorder folds into direct scene methods; the mount
/// plan and shared renderer behind them stay engine-owned.
pub trait Q3ResourceHost {
    /// Zero (missing-shader) picture.
    fn zero_picture(&self) -> MaterialPicture;
    /// Load a model from the shared renderer.
    fn load_model(&mut self, path: &str) -> PresentResult<SceneModel>;
    /// Load a skin from the shared renderer.
    fn load_skin(&mut self, path: &str) -> PresentResult<Option<SceneSkin>>;
    /// Load a shader picture from the shared renderer.
    fn load_shader(&mut self, path: &str, mip: bool) -> PresentResult<Option<MaterialPicture>>;
    /// Load a world scene.
    fn load_world_scene(&mut self, requested_path: &str) -> PresentResult<WorldScene>;
    /// Remap a shader.
    fn remap_shader(&mut self, original: &str, replacement: &str, offset: &str) -> PresentResult<()>;
    /// Clear the scene.
    fn clear_scene(&mut self);
    /// Submit a scene entity.
    fn add_ref_entity(&mut self, entity: RefEntity);
    /// Submit a scene polygon.
    fn add_poly(&mut self, poly: RefPoly);
    /// Submit a dynamic light.
    fn add_light(&mut self, light: DynamicLight);
    /// Render a view definition.
    fn render_scene(&mut self, refdef: &Refdef);
}

/// Shader registration record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceShaderRequest {
    /// Request path.
    pub path: String,
    /// Mipmapped.
    pub mip: bool,
    /// Assigned handle.
    pub handle: i32,
}

/// Registered model row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredModel {
    /// Source path.
    pub path: String,
    /// Integer handle.
    pub handle: i32,
    /// Model.
    pub model: SceneModel,
}

/// Registered skin row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSkin {
    /// Source path.
    pub path: String,
    /// Integer handle.
    pub handle: i32,
    /// Skin.
    pub skin: SceneSkin,
}

/// Model checkpoint row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceModelCheckpoint {
    /// Source path.
    pub path: String,
    /// Integer handle.
    pub handle: i32,
    /// Loaded resource id (`None` for the default model).
    pub resource: Option<u32>,
}

/// Skin checkpoint row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSkinCheckpoint {
    /// Source path.
    pub path: String,
    /// Integer handle.
    pub handle: i32,
    /// Surface bindings.
    pub surfaces: Option<Vec<SkinSurface>>,
}

/// Shader checkpoint row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceShaderCheckpoint {
    /// Request path.
    pub path: String,
    /// Mipmapped.
    pub mip: bool,
    /// Integer handle.
    pub handle: i32,
    /// Resolved picture name.
    pub name: Option<String>,
}

/// Shader alias checkpoint row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceBindingCheckpoint {
    /// Alias name.
    pub name: String,
    /// Integer handle.
    pub handle: i32,
}

/// Saved entity-parser state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityParserSave {
    /// Current token.
    pub token: String,
    /// Current line.
    pub line: i32,
    /// Current name.
    pub name: String,
}

/// Renderer-resource checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCheckpoint {
    /// Model rows.
    pub models: Vec<ResourceModelCheckpoint>,
    /// Skin rows.
    pub skins: Vec<ResourceSkinCheckpoint>,
    /// Shader rows.
    pub shaders: Vec<ResourceShaderCheckpoint>,
    /// Alias rows.
    pub bindings: Vec<ResourceBindingCheckpoint>,
    /// World loaded flag.
    pub world_loaded: bool,
    /// Entity cursor source.
    pub entity_source: String,
    /// Entity cursor offset.
    pub entity_offset: Option<usize>,
    /// Entity parser state.
    pub entity_parser: EntityParserSave,
}

/// Handle owner (`"renderer" | "client"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceHandleOwner {
    /// Renderer material orders.
    Renderer,
    /// Client-assigned handles.
    Client,
}

/// Maximum token characters (`MAX_TOKEN_CHARS`).
pub const MAX_TOKEN_CHARS: usize = 1024;

/// Entity lump parse cursor (`CommonParseCursor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityParseCursor {
    source: Vec<u8>,
    terminator: usize,
    offset: Option<usize>,
}

impl EntityParseCursor {
    /// New cursor over Latin-1 source.
    pub fn new(source: &str) -> PresentResult<Self> {
        for (index, c) in source.chars().enumerate() {
            if c as u32 > 255 {
                return Err(range_msg(format!(
                    "COM_Parse source is not a Latin-1 byte string at {index}"
                )));
            }
        }
        let bytes: Vec<u8> = source.chars().map(|c| c as u8).collect();
        let terminator = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Ok(Self {
            source: bytes,
            terminator,
            offset: Some(0),
        })
    }

    /// Cursor source as Latin-1 text.
    #[must_use]
    pub fn source_text(&self) -> String {
        self.source.iter().map(|b| char::from(*b)).collect()
    }

    /// Current offset.
    #[must_use]
    pub const fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// Set the offset.
    pub fn set_offset(&mut self, value: Option<usize>) -> PresentResult<()> {
        if let Some(offset) = value {
            if offset > self.terminator {
                return Err(range_msg("COM_Parse cursor is outside its C byte string"));
            }
        }
        self.offset = value;
        Ok(())
    }

    fn signed_byte(&self, offset: usize) -> i32 {
        if offset >= self.source.len() {
            return 0;
        }
        let byte = self.source[offset];
        if byte >= 128 {
            i32::from(byte) - 256
        } else {
            i32::from(byte)
        }
    }

    fn char_at(&self, offset: usize) -> char {
        self.source.get(offset).map(|b| char::from(*b)).unwrap_or('\0')
    }
}

/// Entity lump parse state (`CommonParseState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityParseState {
    current_token: String,
    current_line: i32,
    current_name: String,
}

impl EntityParseState {
    /// New parse state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current_token: String::new(),
            current_line: 0,
            current_name: String::new(),
        }
    }

    /// Capture save state.
    #[must_use]
    pub fn capture_save_state(&self) -> EntityParserSave {
        EntityParserSave {
            token: self.current_token.clone(),
            line: self.current_line,
            name: self.current_name.clone(),
        }
    }

    /// Restore save state.
    pub fn restore_save_state(&mut self, save: &EntityParserSave) {
        self.current_token = save.token.clone();
        self.current_line = save.line;
        self.current_name = save.name.clone();
    }

    /// Parse one token (`COM_Parse`).
    pub fn parse(&mut self, cursor: &mut EntityParseCursor) -> PresentResult<String> {
        let mut data = match cursor.offset {
            None => {
                self.current_token.clear();
                return Ok(String::new());
            }
            Some(offset) => offset,
        };
        self.current_token.clear();
        loop {
            loop {
                let c = cursor.signed_byte(data);
                if c > 32 {
                    break;
                }
                if c == 0 {
                    cursor.offset = None;
                    return Ok(String::new());
                }
                if c == 10 {
                    self.current_line = self.current_line.wrapping_add(1);
                }
                data += 1;
            }
            let c = cursor.signed_byte(data);
            if c == 47 && cursor.signed_byte(data + 1) == 47 {
                data += 2;
                while cursor.signed_byte(data) != 0 && cursor.signed_byte(data) != 10 {
                    data += 1;
                }
            } else if c == 47 && cursor.signed_byte(data + 1) == 42 {
                data += 2;
                while cursor.signed_byte(data) != 0
                    && (cursor.signed_byte(data) != 42 || cursor.signed_byte(data + 1) != 47)
                {
                    data += 1;
                }
                if cursor.signed_byte(data) != 0 {
                    data += 2;
                }
            } else {
                break;
            }
        }
        // Quoted tokens allow line breaks: entity lumps always parse with them.
        let c = cursor.signed_byte(data);
        if c == 34 {
            data += 1;
            loop {
                let c = cursor.signed_byte(data);
                data += 1;
                if c == 34 || c == 0 {
                    if self.current_token.len() == MAX_TOKEN_CHARS {
                        return Err(range_msg("COM_Parse quoted token terminator exceeds 1024-byte storage"));
                    }
                    cursor.offset = if c == 0 { None } else { Some(data) };
                    return Ok(std::mem::take(&mut self.current_token));
                }
                if self.current_token.len() < MAX_TOKEN_CHARS {
                    self.current_token.push(cursor.char_at(data - 1));
                }
            }
        }
        loop {
            if self.current_token.len() < MAX_TOKEN_CHARS {
                self.current_token.push(cursor.char_at(data));
            }
            data += 1;
            let c = cursor.signed_byte(data);
            if c == 10 {
                self.current_line = self.current_line.wrapping_add(1);
            }
            if c <= 32 {
                break;
            }
        }
        if self.current_token.len() == MAX_TOKEN_CHARS {
            self.current_token.clear();
        }
        cursor.offset = Some(data);
        Ok(std::mem::take(&mut self.current_token))
    }
}

impl Default for EntityParseState {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn shader_key(path: &str) -> String {
    path.to_ascii_lowercase()
}

/// Session renderer resources (`Q3RendererResources`).
pub struct Q3RendererResources<H, W> {
    /// Host services.
    pub host: H,
    world: Option<W>,
    handles: ResourceHandleOwner,
    models: Vec<SceneModel>,
    model_names: Vec<(String, SceneModel)>,
    skin_handles: Vec<Option<SceneSkin>>,
    skins: Vec<(String, Option<SceneSkin>)>,
    pictures: HashMap<String, MaterialPicture>,
    shaders: HashMap<i32, MaterialPicture>,
    picture_handles: Vec<(SceneShader, i32)>,
    shader_requests: Vec<ResourceShaderRequest>,
    operations: u32,
    entity_parser: EntityParseState,
    entity_cursor: EntityParseCursor,
    world_loaded: bool,
}

impl<H: Q3ResourceHost, W: ResourceWorld> Q3RendererResources<H, W> {
    /// New resources with renderer handles and no world.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self {
            host,
            world: None,
            handles: ResourceHandleOwner::Renderer,
            models: vec![default_model()],
            model_names: Vec::new(),
            skin_handles: vec![None],
            skins: Vec::new(),
            pictures: HashMap::new(),
            shaders: HashMap::new(),
            picture_handles: Vec::new(),
            shader_requests: Vec::new(),
            operations: 0,
            entity_parser: EntityParseState::new(),
            entity_cursor: EntityParseCursor {
                source: Vec::new(),
                terminator: 0,
                offset: Some(0),
            },
            world_loaded: false,
        }
    }

    /// New resources with an explicit world and handle owner.
    #[must_use]
    pub fn with_world(host: H, world: Option<W>, handles: ResourceHandleOwner) -> Self {
        let mut resources = Self::new(host);
        resources.world = world;
        resources.handles = handles;
        resources
    }

    /// Capture a checkpoint.
    pub fn capture_checkpoint(&self) -> PresentResult<ResourceCheckpoint> {
        if self.handles != ResourceHandleOwner::Client || self.operations != 0 {
            return Err(state_msg("Resource checkpoint requires an idle client handle owner"));
        }
        let mut models = Vec::with_capacity(self.model_names.len());
        for (path, model) in &self.model_names {
            models.push(ResourceModelCheckpoint {
                path: path.clone(),
                handle: self.model_handle(model)?,
                resource: model.resource_id(),
            });
        }
        let mut skins = Vec::with_capacity(self.skins.len());
        for (path, skin) in &self.skins {
            skins.push(ResourceSkinCheckpoint {
                path: path.clone(),
                handle: self.skin_handle(skin)?,
                surfaces: skin.as_ref().map(|skin| skin.surfaces.clone()),
            });
        }
        let mut shaders = Vec::with_capacity(self.shader_requests.len());
        for row in &self.shader_requests {
            shaders.push(ResourceShaderCheckpoint {
                path: row.path.clone(),
                mip: row.mip,
                handle: row.handle,
                name: self.shaders.get(&row.handle).map(|picture| picture.name.clone()),
            });
        }
        let mut bindings = Vec::with_capacity(self.pictures.len());
        for (name, picture) in &self.pictures {
            bindings.push(ResourceBindingCheckpoint {
                name: name.clone(),
                handle: self.shader_handle(Some(picture))?,
            });
        }
        Ok(ResourceCheckpoint {
            models,
            skins,
            shaders,
            bindings,
            world_loaded: self.world_loaded,
            entity_source: self.entity_cursor.source_text(),
            entity_offset: self.entity_cursor.offset(),
            entity_parser: self.entity_parser.capture_save_state(),
        })
    }

    /// Restore a checkpoint into an empty owner.
    pub fn restore_checkpoint(&mut self, checkpoint: &ResourceCheckpoint) -> PresentResult<()> {
        if self.handles != ResourceHandleOwner::Client
            || self.operations != 0
            || !self.model_names.is_empty()
            || !self.skins.is_empty()
            || !self.pictures.is_empty()
            || self.world_loaded
        {
            return Err(state_msg("Resource restore requires an empty client handle owner"));
        }
        for row in &checkpoint.models {
            if row.handle < 0 {
                return Err(range_msg("Resource checkpoint model handle is negative"));
            }
            let model = self.register_model(Some(&row.path))?;
            if self.model_handle(&model)? != row.handle || model.resource_id() != row.resource {
                return Err(state_msg("Resource checkpoint model binding changed"));
            }
        }
        for row in &checkpoint.skins {
            if row.handle < 0 {
                return Err(range_msg("Resource checkpoint skin handle is negative"));
            }
            let skin = self.register_skin(&row.path)?;
            let surfaces = skin.as_ref().map(|skin| skin.surfaces.clone());
            if self.skin_handle(&skin)? != row.handle || surfaces != row.surfaces {
                return Err(state_msg("Resource checkpoint skin binding changed"));
            }
        }
        for row in &checkpoint.shaders {
            if row.handle < 1 {
                return Err(range_msg("Resource checkpoint shader handle is not positive"));
            }
            let shader = self.shader(&row.path, row.mip)?;
            if self.shader_handle(shader.as_ref())? != row.handle
                || shader.as_ref().map(|shader| shader.name.clone()) != row.name
            {
                return Err(state_msg("Resource checkpoint shader binding changed"));
            }
        }
        if checkpoint.bindings.len() != self.pictures.len() {
            return Err(state_msg("Resource checkpoint shader aliases changed"));
        }
        for row in &checkpoint.bindings {
            if row.handle < 1 {
                return Err(range_msg("Resource checkpoint shader alias is not positive"));
            }
            let picture = self.pictures.get(&row.name);
            let bound = self.shaders.get(&row.handle);
            if picture.is_none() || picture != bound {
                return Err(state_msg("Resource checkpoint shader aliases changed"));
            }
        }
        self.world_loaded = checkpoint.world_loaded;
        let mut cursor = EntityParseCursor::new(&checkpoint.entity_source)?;
        cursor.set_offset(checkpoint.entity_offset)?;
        self.entity_cursor = cursor;
        self.entity_parser.restore_save_state(&checkpoint.entity_parser);
        Ok(())
    }

    /// Registered models with source names.
    pub fn registered_models(&self) -> PresentResult<Vec<RegisteredModel>> {
        let mut rows = Vec::new();
        for (handle, model) in self.models.iter().enumerate() {
            if handle == 0 {
                continue;
            }
            let named = self.model_names.iter().find(|(_, value)| value == model);
            let Some((path, _)) = named else {
                return Err(state_msg("Registered cgame model has no source name"));
            };
            rows.push(RegisteredModel {
                path: path.clone(),
                handle: handle as i32,
                model: model.clone(),
            });
        }
        Ok(rows)
    }

    /// Registered skins with source names.
    pub fn registered_skins(&self) -> PresentResult<Vec<RegisteredSkin>> {
        let mut rows = Vec::new();
        for (handle, skin) in self.skin_handles.iter().enumerate() {
            let Some(skin) = skin else {
                continue;
            };
            let named = self.skins.iter().find(|(_, value)| value.as_ref() == Some(skin));
            let Some((path, _)) = named else {
                return Err(state_msg("Registered cgame skin has no source name"));
            };
            rows.push(RegisteredSkin {
                path: path.clone(),
                handle: handle as i32,
                skin: skin.clone(),
            });
        }
        Ok(rows)
    }

    fn shader(&mut self, path: &str, mip: bool) -> PresentResult<Option<SceneShader>> {
        let key = shader_key(path);
        if let Some(prior) = self.pictures.get(&key) {
            return Ok(Some(prior.clone()));
        }
        self.operations += 1;
        let loaded = self.host.load_shader(path, mip);
        self.operations -= 1;
        let picture = loaded?;
        let Some(picture) = picture else {
            return Ok(None);
        };
        self.pictures.insert(key, picture.clone());
        self.pictures.insert(shader_key(&picture.name), picture.clone());
        let existing = self.picture_handles.iter().find(|(value, _)| value == &picture);
        let handle = existing.map_or_else(
            || {
                if self.handles == ResourceHandleOwner::Client {
                    self.shaders.len() as i32 + 1
                } else {
                    picture.material_order
                }
            },
            |(_, handle)| *handle,
        );
        self.shaders.insert(handle, picture.clone());
        if existing.is_none() {
            self.picture_handles.push((picture, handle));
        }
        self.shader_requests.push(ResourceShaderRequest {
            path: path.to_string(),
            mip,
            handle,
        });
        Ok(self.shaders.get(&handle).cloned())
    }

    /// Integer handle for a shader.
    pub fn shader_handle(&self, shader: Option<&SceneShader>) -> PresentResult<i32> {
        let Some(shader) = shader else {
            return Ok(0);
        };
        let picture = self.picture(Some(shader))?;
        if self.handles == ResourceHandleOwner::Renderer {
            return Ok(picture.material_order);
        }
        let found = self.picture_handles.iter().find(|(value, _)| value == &picture);
        match found {
            Some((_, handle)) => Ok(*handle),
            None => Err(state_msg("Cgame shader belongs to another resource owner")),
        }
    }

    /// Integer handle for a skin.
    pub fn skin_handle(&self, skin: &Option<SceneSkin>) -> PresentResult<i32> {
        match self.skin_handles.iter().position(|value| value == skin) {
            Some(index) => Ok(index as i32),
            None => Err(state_msg("Cgame skin belongs to another resource owner")),
        }
    }

    /// Skin for an integer handle.
    pub fn skin_for_handle(&self, handle: i32) -> PresentResult<Option<SceneSkin>> {
        if handle < 0 {
            return Err(range_msg(format!("Invalid cgame skin handle {handle}")));
        }
        match self.skin_handles.get(handle as usize) {
            Some(skin) => Ok(skin.clone()),
            None => Err(range_msg(format!("Invalid cgame skin handle {handle}"))),
        }
    }

    /// Parse the next entity-lump token.
    pub fn get_entity_token(&mut self, write: &mut dyn FnMut(&str)) -> PresentResult<bool> {
        let token = self.entity_parser.parse(&mut self.entity_cursor)?;
        write(&token);
        if self.entity_cursor.offset().is_none() || token.is_empty() {
            self.entity_cursor.set_offset(Some(0))?;
            return Ok(false);
        }
        Ok(true)
    }

    fn point_cluster(&self, read: &mut dyn FnMut() -> Vec3) -> PresentResult<i32> {
        let Some(world) = self.world.as_ref() else {
            return Err(drop_msg("R_PointInLeaf: bad model"));
        };
        if !self.world_loaded {
            return Err(drop_msg("R_PointInLeaf: bad model"));
        }
        let map = world.resource_map();
        let mut leaf = 0;
        if !map.nodes.is_empty() {
            let point = read();
            let mut index = 0;
            loop {
                let node = map
                    .nodes
                    .get(index)
                    .ok_or_else(|| range_msg("R_PointInLeaf: invalid node"))?;
                let plane = map
                    .planes
                    .get(node.plane)
                    .ok_or_else(|| range_msg("R_PointInLeaf: invalid plane"))?;
                let child = node.children[usize::from(dot3(point, plane.normal) - plane.distance <= 0.0)];
                match child {
                    ResourceBspChild::Leaf { index } => {
                        leaf = index;
                        break;
                    }
                    ResourceBspChild::Node { index: next } => {
                        index = next;
                    }
                }
            }
        }
        let found = map
            .leaves
            .get(leaf)
            .ok_or_else(|| range_msg("R_PointInLeaf: invalid leaf"))?;
        Ok(found.cluster)
    }

    /// Potential visibility between two points.
    pub fn in_pvs(
        &self,
        read_first: &mut dyn FnMut() -> Vec3,
        read_second: &mut dyn FnMut() -> Vec3,
    ) -> PresentResult<bool> {
        let first = self.point_cluster(read_first)?;
        let Some(world) = self.world.as_ref() else {
            return Err(drop_msg("R_PointInLeaf: bad model"));
        };
        let second = self.point_cluster(read_second)?;
        let byte = world.cluster_pvs_byte(first, (second >> 3) as usize);
        Ok((byte & (1u8 << ((second & 7) as u32))) != 0)
    }
}

impl<H: Q3ResourceHost, W: ResourceWorld> RendererResources for Q3RendererResources<H, W> {
    fn register_model(&mut self, path: Option<&str>) -> PresentResult<SceneModel> {
        match path {
            None => return Ok(default_model()),
            Some("") => return Ok(default_model()),
            _ => {}
        }
        let path = path.unwrap_or_default();
        if let Some(prior) = self.model_names.iter().find(|(name, _)| name == path) {
            return Ok(prior.1.clone());
        }
        self.operations += 1;
        let loaded = self.host.load_model(path);
        self.operations -= 1;
        let model = loaded?;
        self.model_names.push((path.to_string(), model.clone()));
        if !model.is_default() && !self.models.contains(&model) {
            self.models.push(model.clone());
        }
        Ok(model)
    }

    fn register_skin(&mut self, path: &str) -> PresentResult<Option<SceneSkin>> {
        if let Some(prior) = self.skins.iter().find(|(name, _)| name == path) {
            return Ok(prior.1.clone());
        }
        self.operations += 1;
        let loaded = self.host.load_skin(path);
        self.operations -= 1;
        let skin = loaded?;
        self.skins.push((path.to_string(), skin.clone()));
        if skin.is_some() && !self.skin_handles.contains(&skin) {
            self.skin_handles.push(skin.clone());
        }
        Ok(skin)
    }

    fn register_shader(&mut self, path: &str) -> PresentResult<Option<SceneShader>> {
        self.shader(path, true)
    }

    fn register_shader_no_mip(&mut self, path: Option<&str>) -> PresentResult<Option<SceneShader>> {
        match path {
            None => Ok(None),
            Some(path) => self.shader(path, false),
        }
    }

    fn picture(&self, shader: Option<&SceneShader>) -> PresentResult<MaterialPicture> {
        let Some(shader) = shader else {
            return Ok(self.host.zero_picture());
        };
        match self.pictures.get(&shader_key(&shader.name)) {
            Some(picture) => Ok(picture.clone()),
            None => Err(state_msg(format!(
                "Cgame shader is not registered in this resource owner: {}",
                shader.name
            ))),
        }
    }

    fn model_handle(&self, model: &SceneModel) -> PresentResult<i32> {
        match self.models.iter().position(|value| value == model) {
            Some(index) => Ok(index as i32),
            None => Err(state_msg("Cgame model belongs to another resource owner")),
        }
    }

    fn model_for_handle(&self, handle: i32) -> PresentResult<SceneModel> {
        if handle < 0 {
            return Err(range_msg(format!("Invalid cgame model handle {handle}")));
        }
        match self.models.get(handle as usize) {
            Some(model) => Ok(model.clone()),
            None => Err(range_msg(format!("Invalid cgame model handle {handle}"))),
        }
    }

    fn shader_for_handle(&self, handle: i32) -> PresentResult<Option<SceneShader>> {
        if handle == 0 {
            return Ok(None);
        }
        match self.shaders.get(&handle) {
            Some(shader) => Ok(Some(shader.clone())),
            None => Err(range_msg(format!("Invalid cgame shader handle {handle}"))),
        }
    }

    fn clear_scene(&mut self) {
        self.host.clear_scene();
    }

    fn add_ref_entity(&mut self, entity: RefEntity) {
        self.host.add_ref_entity(entity);
    }

    fn add_poly(&mut self, poly: RefPoly) {
        self.host.add_poly(poly);
    }

    fn add_light(&mut self, light: DynamicLight) {
        self.host.add_light(light);
    }

    fn remap_shader(&mut self, original: &str, replacement: &str, offset: &str) -> PresentResult<()> {
        self.host.remap_shader(original, replacement, offset)
    }

    fn render_scene(&mut self, refdef: &Refdef) {
        self.host.render_scene(refdef);
    }

    fn load_world(&mut self, path: &str) -> PresentResult<WorldScene> {
        let scene = self.host.load_world_scene(path)?;
        let entities = self
            .world
            .as_ref()
            .map(|world| world.resource_map().entities.clone())
            .unwrap_or_default();
        self.entity_cursor = EntityParseCursor::new(&entities)?;
        self.world_loaded = true;
        Ok(scene)
    }
}
