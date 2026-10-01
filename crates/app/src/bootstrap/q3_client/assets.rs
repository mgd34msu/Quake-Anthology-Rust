//! Quake III application assets: retained mounts, sounds, fonts, and the renderer resource host.
//!
//! Port of `src/app/bootstrap/q3-client/assets.ts` (`registerQ3ModelRequest`,
//! `registerQ3ShaderRequest`, `ApplicationQ3Assets`). The donor is async over
//! `ApplicationAssets`/`ProviderSceneAssets` (`../assets.ts`, outside this wave); here content,
//! mounts, models, and the world arrive through injected sync seams while this module keeps the
//! donor's normalization fallbacks, census loops, retention rules, content routing, and host
//! callbacks. Sound state shared with bank closures lives behind `Rc<RefCell<..>>` because the
//! donor's loaders borrow the asset owner.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use qa_client::audio::bank::{OpenedSound, SoundBank, SoundContent};
use qa_client::audio::streams::decode_sound_bytes;
use qa_client::audio::types::{SharedPcm, SoundAsset};
use qa_client::audio::SoundFamily;
use qa_client::render::scene::shaders::SceneShaderRegistry;
use qa_client::render::types::{ImageLevel, RenderImage};
use qa_client::text::draw2d::{
    FontAssetServices, FontFileReader, MaterialPicture as DrawMaterialPicture, PictureAsset, RetainedFontFile,
};
use qa_client::text::q3_font::{LegacyFonts, RegisteredFont};
use qa_client::text::q3_font_registry::{
    FontGenerationServices, FontRegistrationHost, FontRegistryCheckpoint, RendererFontRegistry,
};
use qa_client::ClientError;
use qa_content::contract::{ContentId, GameFamily};
use qa_content::md3::parse_skin;
use qa_content::paths::normalize_resource_path;
use qa_content::q3::presentation::audio::{
    pcm_sound, Q3PresentationSoundBank, SoundAsset as PresentationSoundAsset, SoundBank as EngineSoundBank,
};
use qa_content::q3::presentation::ref_entity::{
    PresentResource, PresentWorld, Q3AdmittedRefEntity, Q3DecodedModel, RefEntity, RefModelEntity, RefPoly,
    RefSpriteEntity, SceneInlineModel, SceneLoadedModel, SceneModel, SceneShader, SceneSkin, ShadedFields, SkinMapping,
};
use qa_content::q3::presentation::refdef::Refdef as ContentRefdef;
use qa_content::q3::presentation::resources::{AssetReader, Q3ResourceHost, SoundAssetReader, WorldScene};
use qa_content::q3::presentation::retail_snapshot::{
    DynamicLight as RetailDynamicLight, RefEntity as RetailRefEntity, RefModelEntity as RetailModelEntity,
    RefPoly as RetailRefPoly, RefSpriteEntity as RetailSpriteEntity, Refdef as RetailRefdef,
    SceneModel as RetailSceneModel, SceneShader as RetailSceneShader, SceneSkin as RetailSceneSkin,
    SkinSurface as RetailSkinSurface,
};
use qa_content::q3::presentation::scene::{Q3FogSelection, Q3SceneRecorder};
use qa_content::q3::presentation::state::{PresentClientError, PresentResult};
use qa_core::math::{vec2, Bounds, Vec4};
use qa_platform::files::writable::{UserFileStore, WritableFileMode};
use thiserror::Error;

/// Shared print callback (`media.print`).
pub type SharedPrint = Rc<RefCell<Box<dyn FnMut(&str)>>>;

/// Shared sound bank (`media.bank`).
pub type SharedSoundBank = Rc<RefCell<Q3PresentationSoundBank>>;

/// Shared asset seams and load tables.
pub type SharedAssetSeams = Rc<RefCell<Q3AssetShared>>;

/// Quake III asset failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3AssetError {
    /// A mount or loader failed.
    #[error("Quake III asset load failed: {0}")]
    Load(String),
    /// The loose-file census failed.
    #[error("Quake III asset census failed: {0}")]
    Census(String),
    /// An inline model has no bounds row.
    #[error("Missing inline model bounds {0}")]
    MissingBounds(String),
    /// A shader remap belongs to a retired source request.
    #[error("Shader remap belongs to a retired source request")]
    RetiredRemap,
}

impl From<Q3AssetError> for PresentClientError {
    fn from(error: Q3AssetError) -> Self {
        match error {
            Q3AssetError::Load(message) | Q3AssetError::Census(message) => Self::State(message),
            Q3AssetError::MissingBounds(path) => Self::State(format!("Missing inline model bounds {path}")),
            Q3AssetError::RetiredRemap => Self::State("Shader remap belongs to a retired source request".to_string()),
        }
    }
}

impl From<Q3AssetError> for qa_content::q3::presentation::ref_entity::PresentError {
    fn from(error: Q3AssetError) -> Self {
        Self::state(error.to_string())
    }
}

/// Adapt a decoded client sound to the presentation bank's identity handle.
fn presentation_sound(sound: &SoundAsset) -> PresentationSoundAsset {
    PresentationSoundAsset {
        pcm: pcm_sound(&sound.name),
        resource: Some(sound.resource.clone()),
    }
}

/// Opened mount bytes with their resource identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedQ3Asset {
    /// Resource id.
    pub id: String,
    /// File bytes.
    pub bytes: Vec<u8>,
}

/// Content-routed mount surface behind the asset census and retention.
pub trait Q3AssetMounts {
    /// Open a content path, or `None` when absent.
    fn open(&mut self, content: &ContentId, path: &str) -> Result<Option<OpenedQ3Asset>, Q3AssetError>;
    /// Whether a content path resolves.
    fn resolve(&mut self, content: &ContentId, path: &str) -> bool;
    /// Catalog archive entries as `(archive path, entry paths)` across products.
    fn catalog_archives(&self) -> Vec<(String, Vec<String>)>;
    /// Archive paths admitted by a content's mount plan.
    fn plan_archives(&self, content: &ContentId) -> Vec<String>;
    /// Loose roots admitted by a content's mount plan.
    fn loose_roots(&self, content: &ContentId) -> Vec<PathBuf>;
}

/// A decoded model load: drawable payload or brush reference.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3AssetModel {
    /// Decoded model payload.
    Decoded(Q3DecodedModel),
    /// Brush reference into a world.
    Brush {
        /// World handle.
        world: PresentWorld,
        /// Submodel index.
        model: usize,
    },
}

/// A loaded model asset with its resource identity.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3LoadedModel {
    /// Model payload.
    pub model: Q3AssetModel,
    /// Resource identity.
    pub resource: PresentResource,
}

/// Content-routed model and skin loading.
pub trait Q3AssetModels {
    /// Load a model asset; the caller retains provider routing.
    fn load_model(&mut self, content: &ContentId, path: &str) -> Result<Q3LoadedModel, Q3AssetError>;
    /// Open raw skin bytes, or `None` when absent.
    fn open_skin(&mut self, content: &ContentId, path: &str) -> Result<Option<Vec<u8>>, Q3AssetError>;
    /// Inline bounds for a world submodel, or `None` when the row is missing.
    fn inline_bounds(&mut self, world: &PresentWorld, index: usize) -> Option<Bounds>;
}

/// Catalog data behind model content routing (donor `contentFor`).
pub trait Q3AssetCatalog {
    /// Appearance content for player models.
    fn character_content(&self) -> ContentId;
    /// Weapon contents in recipe order.
    fn weapon_contents(&self) -> Vec<ContentId>;
    /// Product family of a content.
    fn family_of(&self, content: &ContentId) -> GameFamily;
}

/// Shader-remap outcome (donor `"stale"` union).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3RemapOutcome {
    /// The remap applied.
    Applied,
    /// The remap belongs to a retired source request.
    Stale,
}

/// Loaded-world services: map bounds, shader remaps, and fog.
pub trait Q3AssetWorld {
    /// World submodel bounds for the resource host's map row.
    fn world_model_bounds(&self) -> Vec<Bounds>;
    /// Remap a shader unless `current` reports a retired request.
    fn remap_shader(
        &mut self,
        original: &str,
        replacement: &str,
        time_offset: f32,
        current: &dyn Fn() -> bool,
    ) -> Q3RemapOutcome;
    /// Fog selections for admission and procedural fog.
    fn fog_selections(&self) -> Vec<Q3FogSelection>;
}

/// Asset scope (donor `"selected" | "source"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q3AssetScope {
    /// Route player and weapon models through the recipe.
    #[default]
    Selected,
    /// Load every model from the source content.
    Source,
}

/// Asset preload mode (donor `"source-sync" | "guest-async"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q3AssetMode {
    /// Retain synchronous scripts and sounds up front.
    #[default]
    SourceSync,
    /// Load everything on demand.
    GuestAsync,
}

/// Normalize a model request, defaulting invalid paths (donor `registerQ3ModelRequest`).
pub fn register_q3_model_request<E>(
    path: &str,
    load: impl FnOnce(String) -> Result<SceneModel, E>,
) -> Result<SceneModel, E> {
    if !path.starts_with('*') {
        let normalized = match normalize_resource_path(path) {
            Ok(normalized) => normalized,
            Err(_) => return Ok(SceneModel::default_model()),
        };
        return load(normalized);
    }
    load(path.to_string())
}

/// Normalize a shader request, rejecting invalid paths (donor `registerQ3ShaderRequest`).
pub fn register_q3_shader_request<T, E>(
    path: &str,
    load: impl FnOnce(String) -> Result<Option<T>, E>,
) -> Result<Option<T>, E> {
    let normalized = match normalize_resource_path(path) {
        Ok(normalized) => normalized,
        Err(_) => return Ok(None),
    };
    load(normalized)
}

/// JavaScript `Number.parseFloat` prefix semantics for shader-remap offsets.
fn parse_float_prefix(text: &str) -> f64 {
    let bytes = text.trim_start().as_bytes();
    let mut index = 0;
    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        index += 1;
    }
    if bytes[index..].starts_with(b"Infinity") {
        return if bytes[0] == b'-' {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
    }
    if index > start && index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
        let mut exponent = index + 1;
        if exponent < bytes.len() && (bytes[exponent] == b'+' || bytes[exponent] == b'-') {
            exponent += 1;
        }
        let digits = exponent;
        while exponent < bytes.len() && bytes[exponent].is_ascii_digit() {
            exponent += 1;
        }
        if exponent > digits {
            index = exponent;
        }
    }
    if index == start {
        return f64::NAN;
    }
    text.trim_start()[..index].parse::<f64>().unwrap_or(f64::NAN)
}

/// Whether a census name is retained for synchronous source reads (donor `create` filter).
fn retain_sync_name(path: &str) -> bool {
    if let Some(rest) = path.strip_prefix("sound/") {
        return rest.ends_with(".wav") || rest.ends_with(".ogg");
    }
    if !path.contains('.') {
        return false;
    }
    matches!(
        path.rsplit('.').next(),
        Some("menu" | "hud" | "txt" | "cfg" | "voice" | "vc" | "h")
    )
}

/// Walk one loose root into the census, skipping a missing root (donor `readdir`).
fn walk_loose(root: &Path, names: &mut BTreeSet<String>) -> Result<(), Q3AssetError> {
    let base = match root.canonicalize() {
        Ok(base) => base,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Q3AssetError::Census(format!(
                "cannot census {}: {error}",
                root.display()
            )))
        }
    };
    let mut stack = vec![base.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Q3AssetError::Census(format!(
                    "cannot census {}: {error}",
                    dir.display()
                )))
            }
        };
        for entry in entries {
            let entry =
                entry.map_err(|error| Q3AssetError::Census(format!("cannot census {}: {error}", dir.display())))?;
            let kind = entry
                .file_type()
                .map_err(|error| Q3AssetError::Census(format!("cannot census {}: {error}", dir.display())))?;
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                let full = entry.path();
                let relative = full.strip_prefix(&base).map_err(|_| {
                    Q3AssetError::Census(format!("cannot census {}: entry escapes root", dir.display()))
                })?;
                let mut text = relative.to_string_lossy().replace('\\', "/");
                text.make_ascii_lowercase();
                names.insert(text);
            }
        }
    }
    Ok(())
}

/// Retained mount bytes shared with the sound bank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetainedQ3Asset {
    /// Resource id.
    resource: String,
    /// File bytes.
    bytes: Vec<u8>,
}

/// Retained bytes and decoded sounds behind the bank closures.
#[derive(Debug, Default)]
pub(crate) struct SoundCaches {
    /// Retained opens by lowercase path.
    pub(crate) retained: HashMap<String, RetainedQ3Asset>,
    /// Decoded sounds by lowercase path.
    pub(crate) sounds: HashMap<String, SoundAsset>,
}

impl SoundCaches {
    /// Decode (and memoize) a retained sound (donor `sound`).
    fn sound(&mut self, name: &str) -> Option<SoundAsset> {
        let path = if let Some(stripped) = name.strip_prefix('#') {
            stripped.to_string()
        } else if name.starts_with("sound/") {
            name.to_string()
        } else {
            format!("sound/{name}")
        }
        .to_lowercase();
        if let Some(prior) = self.sounds.get(&path) {
            return Some(prior.clone());
        }
        let opened = self.retained.get(&path)?;
        let pcm = match decode_sound_bytes(&opened.bytes, &path) {
            Ok(pcm) => pcm,
            Err(_) => return None,
        };
        let sound = SoundAsset {
            resource: opened.resource.clone(),
            name: path.clone(),
            pcm: SharedPcm::new(pcm),
        };
        self.sounds.insert(path, sound.clone());
        Some(sound)
    }
}

/// Shared sound caches behind bank closures.
pub(crate) type SharedSoundCaches = Rc<RefCell<SoundCaches>>;

/// [`SoundContent`] over shared mounts for the engine bank.
#[derive(Clone)]
struct RetainedSoundContent {
    /// Shared seams.
    shared: SharedAssetSeams,
    /// Source content.
    content: ContentId,
}

impl SoundContent for RetainedSoundContent {
    fn open(&mut self, path: &str) -> Option<OpenedSound> {
        let opened = self
            .shared
            .borrow_mut()
            .mounts
            .open(&self.content, &path.to_lowercase())
            .ok()??;
        Some(OpenedSound {
            id: opened.id,
            content: String::new(),
            bytes: opened.bytes,
        })
    }
}

/// Engine bank adapter over retained content (donor `new SoundBank(provider.mounts)`).
struct RetainedEngineBank {
    /// Retained-content bank.
    bank: SoundBank<RetainedSoundContent>,
    /// Shared caches.
    caches: SharedSoundCaches,
}

impl RetainedEngineBank {
    /// Bank over shared mounts and caches.
    fn new(caches: SharedSoundCaches, shared: SharedAssetSeams, content: ContentId) -> Self {
        Self {
            bank: SoundBank::new(RetainedSoundContent { shared, content }),
            caches,
        }
    }
}

impl EngineSoundBank for RetainedEngineBank {
    fn register(&mut self, path: &str, family: &str) -> Option<PresentationSoundAsset> {
        let family = match family {
            "q1" => SoundFamily::Q1,
            "q2" => SoundFamily::Q2,
            "q3" => SoundFamily::Q3,
            _ => return None,
        };
        let sound = self.bank.register(path, family).ok()??;
        // Keep the decoded client asset reachable for service playback by bank name.
        self.caches
            .borrow_mut()
            .sounds
            .insert(sound.name.to_lowercase(), sound.clone());
        Some(presentation_sound(&sound))
    }
}

/// Shared mount/model/catalog/world seams plus load tables.
pub struct Q3AssetShared {
    /// Mounts.
    pub mounts: Box<dyn Q3AssetMounts>,
    /// Models.
    pub models: Box<dyn Q3AssetModels>,
    /// Catalog.
    pub catalog: Box<dyn Q3AssetCatalog>,
    /// World.
    pub world: Box<dyn Q3AssetWorld>,
    /// Contents by loaded model (donor `modelContents`).
    pub model_contents: Vec<(SceneModel, ContentId)>,
    /// Provider slots by loaded model (donor `modelProviders`).
    pub model_slots: Vec<(SceneModel, usize)>,
    /// Contents by provider slot.
    pub provider_contents: Vec<ContentId>,
    /// Loaded models by retail id minus one.
    pub loaded_models: Vec<SceneModel>,
    /// Loaded skins as `(path, surfaces)` by retail id minus one.
    pub loaded_skins: Vec<(String, Vec<SkinMapping>)>,
    /// Whether the owner closed.
    pub closed: bool,
}

/// Route a model or skin path to its content (donor `contentFor`).
fn route_content(shared: &Q3AssetShared, scope: Q3AssetScope, content: &ContentId, path: &str) -> ContentId {
    if scope == Q3AssetScope::Source {
        return content.clone();
    }
    if path.starts_with("models/players/") {
        let character = shared.catalog.character_content();
        if shared.catalog.family_of(&character) == GameFamily::Q3 {
            return character;
        }
    }
    if path.starts_with("models/weapons2/") || path.starts_with("models/weaphits/") || path.starts_with("models/ammo/")
    {
        for weapon in shared.catalog.weapon_contents() {
            if shared.catalog.family_of(&weapon) == GameFamily::Q3 {
                return weapon;
            }
        }
    }
    content.clone()
}

/// Font files over shared mounts (donor `MountedFontReader(provider.mounts)`).
struct Q3FontFiles {
    /// Shared seams.
    shared: Rc<RefCell<Q3AssetShared>>,
    /// Source content.
    content: ContentId,
}

impl FontFileReader for Q3FontFiles {
    fn read_file_length(&mut self, path: &str) -> i64 {
        match self.shared.borrow_mut().mounts.open(&self.content, path) {
            Ok(Some(opened)) => opened.bytes.len() as i64,
            _ => -1,
        }
    }

    fn read_file_retained(&mut self, path: &str) -> Option<RetainedFontFile> {
        let opened = self.shared.borrow_mut().mounts.open(&self.content, path).ok()??;
        Some(RetainedFontFile {
            length: opened.bytes.len(),
            bytes: opened.bytes,
        })
    }

    fn free_file(&mut self, _file: &RetainedFontFile) {}
}

/// Font picture host over shared shaders (donor `registerPicture` closure).
struct Q3FontHost {
    /// Shared shaders.
    shaders: Rc<RefCell<SceneShaderRegistry>>,
    /// Diagnostics buffer flushed by the font call.
    print: Rc<RefCell<Vec<String>>>,
}

impl Q3FontHost {
    /// Fallback picture when registration fails.
    fn fallback(&self, name: &str, error: String) -> DrawMaterialPicture {
        self.print
            .borrow_mut()
            .push(format!("Q3 font picture {name} failed: {error}"));
        DrawMaterialPicture { order: 0 }
    }
}

impl FontRegistrationHost for Q3FontHost {
    fn register_picture(&mut self, shader_name: &str) -> DrawMaterialPicture {
        match self.shaders.borrow_mut().register_picture(shader_name, false) {
            Ok(picture) => picture,
            Err(error) => self.fallback(shader_name, error.to_string()),
        }
    }
}

/// Font generation over shared shaders and the user store (donor generation closures).
struct Q3FontGeneration {
    /// Shared shaders.
    shaders: Rc<RefCell<SceneShaderRegistry>>,
    /// Whether generated DAT/TGA files are written back.
    save_font_data: Rc<dyn Fn() -> bool>,
    /// User content root for font export, if any.
    user_root: Option<PathBuf>,
    /// Diagnostics buffer flushed by the font call.
    print: Rc<RefCell<Vec<String>>>,
}

impl FontGenerationServices for Q3FontGeneration {
    fn register_image(&mut self, name: &str, rgba: Vec<u8>) -> DrawMaterialPicture {
        let image = RenderImage::Rgba8 {
            levels: vec![ImageLevel {
                width: 256,
                height: 256,
                pixels: rgba,
            }],
            border_color: Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
        };
        match self.shaders.borrow_mut().register_generated_picture(name, image) {
            Ok(picture) => picture,
            Err(error) => {
                self.print
                    .borrow_mut()
                    .push(format!("Q3 generated font picture {name} failed: {error}"));
                DrawMaterialPicture { order: 0 }
            }
        }
    }

    fn save_font_data(&self) -> bool {
        (self.save_font_data)()
    }

    fn write_file(&mut self, name: &str, bytes: Vec<u8>) {
        let Some(root) = &self.user_root else {
            self.print
                .borrow_mut()
                .push("Font export requires a content user directory".to_string());
            return;
        };
        let file = UserFileStore::new(root.clone()).open(name, WritableFileMode::Write, |_| {});
        let mut file = match file {
            Ok(Some(file)) => file,
            _ => {
                self.print
                    .borrow_mut()
                    .push(format!("Cannot export generated font {name}"));
                return;
            }
        };
        let complete = file.write(&bytes).is_ok_and(|written| written == bytes.len());
        file.close();
        if !complete {
            self.print
                .borrow_mut()
                .push(format!("Incomplete generated font export {name}"));
        }
    }
}

/// Synchronous source script/sound assets over the mount plan (donor `ApplicationQ3Assets`).
pub struct ApplicationQ3Assets {
    /// Shared seams and load tables.
    shared: Rc<RefCell<Q3AssetShared>>,
    /// Shared shader registry.
    shaders: Rc<RefCell<SceneShaderRegistry>>,
    /// Source content.
    content: ContentId,
    /// Asset scope.
    scope: Q3AssetScope,
    /// Print callback.
    print: SharedPrint,
    /// Whether generated font data is written back.
    save_font_data: Rc<dyn Fn() -> bool>,
    /// User content root for font export, if any.
    user_root: Option<PathBuf>,
    /// Census names (donor `names`).
    names: BTreeSet<String>,
    /// Retained bytes and decoded sounds.
    caches: SharedSoundCaches,
    /// Presentation sound bank.
    bank: SharedSoundBank,
    /// Font registry checkpoint across font calls.
    font_checkpoint: Option<FontRegistryCheckpoint>,
}

/// Options for [`ApplicationQ3Assets::create`] (donor `create` arguments).
pub struct ApplicationQ3AssetOptions {
    /// Source content.
    pub content: ContentId,
    /// Mounts.
    pub mounts: Box<dyn Q3AssetMounts>,
    /// Models.
    pub models: Box<dyn Q3AssetModels>,
    /// Catalog.
    pub catalog: Box<dyn Q3AssetCatalog>,
    /// World.
    pub world: Box<dyn Q3AssetWorld>,
    /// Shader registry.
    pub shaders: SceneShaderRegistry,
    /// Print callback.
    pub print: Box<dyn FnMut(&str)>,
    /// Whether generated font data is written back.
    pub save_font_data: Box<dyn Fn() -> bool>,
    /// User content root for font export, if any.
    pub user_root: Option<PathBuf>,
    /// Asset scope.
    pub scope: Q3AssetScope,
    /// Preload mode.
    pub mode: Q3AssetMode,
}

impl ApplicationQ3Assets {
    /// Census the mounts and retain synchronous source assets (donor `create`).
    pub fn create(mut options: ApplicationQ3AssetOptions) -> Result<Self, Q3AssetError> {
        let mut names = BTreeSet::new();
        let archives: std::collections::HashSet<String> =
            options.mounts.plan_archives(&options.content).into_iter().collect();
        for (path, entries) in options.mounts.catalog_archives() {
            if archives.contains(&path) {
                for entry in entries {
                    names.insert(entry.to_lowercase());
                }
            }
        }
        for root in options.mounts.loose_roots(&options.content) {
            walk_loose(&root, &mut names)?;
        }
        let caches: SharedSoundCaches = Rc::new(RefCell::new(SoundCaches::default()));
        if options.mode == Q3AssetMode::SourceSync {
            let synchronous: Vec<String> = names.iter().filter(|path| retain_sync_name(path)).cloned().collect();
            for path in synchronous {
                if let Some(opened) = options.mounts.open(&options.content, &path)? {
                    caches.borrow_mut().retained.insert(
                        path,
                        RetainedQ3Asset {
                            resource: opened.id,
                            bytes: opened.bytes,
                        },
                    );
                }
            }
        }
        let shared: SharedAssetSeams = Rc::new(RefCell::new(Q3AssetShared {
            mounts: options.mounts,
            models: options.models,
            catalog: options.catalog,
            world: options.world,
            model_contents: Vec::new(),
            model_slots: Vec::new(),
            provider_contents: Vec::new(),
            loaded_models: Vec::new(),
            loaded_skins: Vec::new(),
            closed: false,
        }));
        let bank_caches = caches.clone();
        let bank = Rc::new(RefCell::new(Q3PresentationSoundBank::new(
            Rc::new(RefCell::new(RetainedEngineBank::new(
                caches.clone(),
                shared.clone(),
                options.content.clone(),
            ))),
            None,
            Box::new(move |path, _compressed| bank_caches.borrow_mut().sound(path).as_ref().map(presentation_sound)),
        )));
        Ok(Self {
            shared,
            shaders: Rc::new(RefCell::new(options.shaders)),
            content: options.content,
            scope: options.scope,
            print: Rc::new(RefCell::new(options.print)),
            save_font_data: options.save_font_data.into(),
            user_root: options.user_root,
            names,
            caches,
            bank,
            font_checkpoint: None,
        })
    }

    /// Source content.
    #[must_use]
    pub fn content(&self) -> &ContentId {
        &self.content
    }

    /// Shared print callback.
    #[must_use]
    pub fn print_shared(&self) -> SharedPrint {
        self.print.clone()
    }

    /// Shared sound bank.
    #[must_use]
    pub fn bank_shared(&self) -> SharedSoundBank {
        self.bank.clone()
    }

    /// Shared seams and load tables.
    #[must_use]
    pub fn shared(&self) -> SharedAssetSeams {
        self.shared.clone()
    }

    /// Shared sound caches.
    pub(crate) fn sound_caches(&self) -> SharedSoundCaches {
        self.caches.clone()
    }

    /// Shared shader registry.
    #[must_use]
    pub fn shaders_shared(&self) -> Rc<RefCell<SceneShaderRegistry>> {
        self.shaders.clone()
    }

    /// Print a line.
    pub fn print_text(&self, text: &str) {
        (self.print.borrow_mut())(text);
    }

    /// Whether the owner closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shared.borrow().closed
    }

    /// Route a model or skin path to its content.
    #[must_use]
    pub fn content_for(&self, path: &str) -> ContentId {
        let shared = self.shared.borrow();
        route_content(&shared, self.scope, &self.content, path)
    }

    /// Provider slot for a loaded model, if any.
    #[must_use]
    pub fn provider_slot(&self, model: &SceneModel) -> Option<usize> {
        self.shared
            .borrow()
            .model_slots
            .iter()
            .find(|(value, _)| value == model)
            .map(|(_, slot)| *slot)
    }

    /// Content behind a provider slot, if any.
    #[must_use]
    pub fn provider_content(&self, slot: usize) -> Option<ContentId> {
        self.shared.borrow().provider_contents.get(slot).cloned()
    }

    /// Whether a census name exists (donor `has`).
    #[must_use]
    pub fn has(&self, path: &str) -> bool {
        self.names.contains(&path.to_lowercase())
    }

    /// Census names under a prefix (donor `list`).
    #[must_use]
    pub fn list(&self, prefix: &str) -> Vec<String> {
        let prefix = prefix.to_lowercase();
        self.names
            .iter()
            .filter(|path| path.starts_with(&prefix))
            .cloned()
            .collect()
    }

    /// Open (and retain) an asset (donor `open`).
    fn open(&mut self, path: &str) -> Result<Option<Vec<u8>>, Q3AssetError> {
        let key = path.to_lowercase();
        if let Some(prior) = self.caches.borrow().retained.get(&key) {
            return Ok(Some(prior.bytes.clone()));
        }
        let opened = self.shared.borrow_mut().mounts.open(&self.content, path)?;
        let Some(opened) = opened else {
            return Ok(None);
        };
        self.caches.borrow_mut().retained.insert(
            key,
            RetainedQ3Asset {
                resource: opened.id,
                bytes: opened.bytes.clone(),
            },
        );
        Ok(Some(opened.bytes))
    }

    /// Read an asset's bytes, empty when missing (donor `read`).
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, Q3AssetError> {
        Ok(self.open(path)?.unwrap_or_default())
    }

    /// Read an asset's byte length (donor `readFileLength`, missing reads fail).
    pub fn read_file_length(&mut self, path: &str) -> Result<usize, Q3AssetError> {
        match self.open(path)? {
            Some(bytes) => Ok(bytes.len()),
            None => Err(Q3AssetError::Load(format!("Q3 asset is missing: {path}"))),
        }
    }

    /// Read retained bytes, failing for unretained paths (donor `readSync`).
    pub fn read_sync(&self, path: &str) -> Result<Vec<u8>, Q3AssetError> {
        match self.caches.borrow().retained.get(&path.to_lowercase()) {
            Some(opened) => Ok(opened.bytes.clone()),
            None => Err(Q3AssetError::Load(format!(
                "Q3 source requested an unretained synchronous asset: {path}"
            ))),
        }
    }

    /// Decode (and memoize) a retained sound.
    pub fn sound(&mut self, name: &str) -> Option<SoundAsset> {
        self.caches.borrow_mut().sound(name)
    }

    /// Build a renderer resource host over a shared scene recorder (donor `resourceHost`).
    #[must_use]
    pub fn resource_host(&self, scene: Rc<RefCell<Q3SceneRecorder>>) -> Q3AssetResourceHost {
        Q3AssetResourceHost {
            scene,
            shared: self.shared.clone(),
            shaders: self.shaders.clone(),
            content: self.content.clone(),
            scope: self.scope,
            remap_override: None,
        }
    }

    /// Run a closure over the font registry, preserving its checkpoint.
    pub fn with_fonts<R>(&mut self, run: impl FnOnce(&mut RendererFontRegistry) -> R) -> Result<R, ClientError> {
        if self.is_closed() {
            return Err(ClientError::BadFont("Renderer font registry is closed".to_string()));
        }
        let print: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let mut files = Q3FontFiles {
            shared: self.shared.clone(),
            content: self.content.clone(),
        };
        let mut host = Q3FontHost {
            shaders: self.shaders.clone(),
            print: print.clone(),
        };
        let mut generation = Q3FontGeneration {
            shaders: self.shaders.clone(),
            save_font_data: self.save_font_data.clone(),
            user_root: self.user_root.clone(),
            print: print.clone(),
        };
        let mut fonts = RendererFontRegistry::with_generation(&mut files, &mut host, &mut generation);
        if let Some(checkpoint) = &self.font_checkpoint {
            fonts.restore_checkpoint(checkpoint)?;
        }
        let result = run(&mut fonts);
        self.font_checkpoint = Some(fonts.capture_checkpoint()?);
        for line in print.borrow().iter() {
            self.print_text(line);
        }
        Ok(result)
    }

    /// Run a closure over the UI font assets (donor `fontRegistry`).
    pub fn with_ui_fonts<R>(&mut self, run: impl FnOnce(&mut Q3UiFontAssets) -> R) -> Result<R, ClientError> {
        let print: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let buffered = print.clone();
        let shaders = self.shaders.clone();
        let result = self.with_fonts(|fonts| {
            let mut ui = Q3UiFontAssets {
                fonts,
                shaders,
                print: buffered,
            };
            run(&mut ui)
        })?;
        for line in print.borrow().iter() {
            self.print_text(line);
        }
        Ok(result)
    }

    /// Register a font into a caller glyph table.
    pub fn register_font_record(
        &mut self,
        name: Option<&str>,
        point_size: i32,
        table: &mut [u8],
    ) -> Result<Option<RegisteredFont>, ClientError> {
        let mut lines = Vec::new();
        let result = self.with_fonts(|fonts| {
            fonts.register_font_into(
                name,
                point_size as f32,
                &mut |line: &str| lines.push(line.to_string()),
                table,
            )
        })??;
        for line in lines {
            self.print_text(&line);
        }
        Ok(result)
    }

    /// Close the owner, releasing retention (donor `close`).
    pub fn close(&mut self) {
        let mut shared = self.shared.borrow_mut();
        shared.closed = true;
        shared.model_contents.clear();
        shared.model_slots.clear();
        shared.provider_contents.clear();
        shared.loaded_models.clear();
        shared.loaded_skins.clear();
        let mut caches = self.caches.borrow_mut();
        caches.retained.clear();
        caches.sounds.clear();
        self.font_checkpoint = None;
    }
}

impl AssetReader for ApplicationQ3Assets {
    fn read_asset(&mut self, path: &str) -> PresentResult<Vec<u8>> {
        self.read(path).map_err(PresentClientError::from)
    }

    fn has_asset(&self, path: &str) -> bool {
        self.has(path)
    }

    fn list_assets(&self, prefix: Option<&str>) -> Vec<String> {
        self.list(prefix.unwrap_or(""))
    }
}

impl SoundAssetReader for ApplicationQ3Assets {
    fn read_file_length(&mut self, path: &str) -> PresentResult<usize> {
        self.read_file_length(path).map_err(PresentClientError::from)
    }

    fn read_asset_sync(&self, path: &str) -> PresentResult<Vec<u8>> {
        self.read_sync(path).map_err(PresentClientError::from)
    }
}

/// UI font assets (donor `UiAssetRegistry`).
pub struct Q3UiFontAssets<'a, 'b> {
    /// Font registry.
    fonts: &'a mut RendererFontRegistry<'b>,
    /// Shared shaders for picture registration.
    shaders: Rc<RefCell<SceneShaderRegistry>>,
    /// Diagnostics buffer flushed by the font call.
    print: Rc<RefCell<Vec<String>>>,
}

impl FontAssetServices for Q3UiFontAssets<'_, '_> {
    fn register_picture(&mut self, path: &str, mip: bool) -> PictureAsset {
        match self.shaders.borrow_mut().register_picture(path, mip) {
            Ok(picture) => PictureAsset::Material(picture),
            Err(error) => {
                self.print
                    .borrow_mut()
                    .push(format!("Q3 UI picture {path} failed: {error}"));
                PictureAsset::Material(DrawMaterialPicture { order: 0 })
            }
        }
    }
}

impl Q3UiFontAssets<'_, '_> {
    /// Register a font (donor `registerFont`).
    pub fn register_font(
        &mut self,
        path: Option<&str>,
        point_size: i32,
        print: &mut dyn FnMut(&str),
    ) -> Result<Option<RegisteredFont>, ClientError> {
        self.fonts.register_font(path, point_size as f32, print)
    }

    /// Load the legacy UI fonts (donor `loadLegacyFonts`).
    pub fn load_legacy_fonts(&mut self) -> LegacyFonts {
        LegacyFonts {
            charset: self.register_picture("gfx/2d/bigchars", false),
            proportional: self.register_picture("menu/art/font1_prop.tga", false),
            glow: self.register_picture("menu/art/font1_prop_glo.tga", false),
            banner: self.register_picture("menu/art/font2_prop.tga", false),
        }
    }
}

/// Shader-remap override (donor `remapShader` option).
pub type Q3RemapOverride = Box<dyn FnMut(&str, &str, &str) -> Result<(), Q3AssetError>>;

/// Renderer resource host over a scene recorder (donor `resourceHost` result).
pub struct Q3AssetResourceHost {
    /// Shared scene recorder.
    scene: Rc<RefCell<Q3SceneRecorder>>,
    /// Shared seams and load tables.
    shared: Rc<RefCell<Q3AssetShared>>,
    /// Shared shader registry.
    shaders: Rc<RefCell<SceneShaderRegistry>>,
    /// Source content.
    content: ContentId,
    /// Asset scope.
    scope: Q3AssetScope,
    /// Shader-remap override.
    remap_override: Option<Q3RemapOverride>,
}

impl Q3AssetResourceHost {
    /// Shared scene recorder.
    #[must_use]
    pub fn scene_shared(&self) -> Rc<RefCell<Q3SceneRecorder>> {
        self.scene.clone()
    }

    /// Install the shader-remap override.
    pub fn set_remap_override(&mut self, remap: Q3RemapOverride) {
        self.remap_override = Some(remap);
    }

    /// Route a model or skin path to its content.
    fn content_for(&self, path: &str) -> ContentId {
        let shared = self.shared.borrow();
        route_content(&shared, self.scope, &self.content, path)
    }

    /// Provider slot for a content, allocating when new.
    fn slot_for(&mut self, content: &ContentId) -> usize {
        let mut shared = self.shared.borrow_mut();
        if let Some(slot) = shared.provider_contents.iter().position(|value| value == content) {
            return slot;
        }
        shared.provider_contents.push(content.clone());
        shared.provider_contents.len() - 1
    }

    /// Load a normalized model request (donor `resourceHost.model`).
    fn load_model_inner(&mut self, path: String) -> Result<SceneModel, Q3AssetError> {
        let content = self.content_for(&path);
        if !path.starts_with('*') {
            let resolved = self.shared.borrow_mut().mounts.resolve(&content, &path);
            if !resolved {
                return Ok(SceneModel::default_model());
            }
        }
        let asset = self.shared.borrow_mut().models.load_model(&content, &path)?;
        let model = match asset.model {
            Q3AssetModel::Decoded(decoded) => SceneModel::Loaded(SceneLoadedModel {
                path: path.clone(),
                model: decoded,
                resource: asset.resource,
            }),
            Q3AssetModel::Brush { world, model } => {
                let bounds = self.shared.borrow_mut().models.inline_bounds(&world, model);
                let Some(bounds) = bounds else {
                    return Err(Q3AssetError::MissingBounds(path));
                };
                SceneModel::Inline(SceneInlineModel {
                    path: path.clone(),
                    index: model,
                    geometry: world,
                    resource: asset.resource,
                    bounds,
                })
            }
        };
        let slot = self.slot_for(&content);
        let mut shared = self.shared.borrow_mut();
        shared.model_contents.push((model.clone(), content));
        shared.model_slots.push((model.clone(), slot));
        Ok(model)
    }
}

/// Admit a retail model entity into the recorder.
fn admit_model_entity(shared: &Q3AssetShared, entity: RetailModelEntity) -> RefModelEntity {
    let model = match entity.model {
        RetailSceneModel::Default => SceneModel::default_model(),
        RetailSceneModel::Loaded { id } => shared
            .loaded_models
            .get(id.saturating_sub(1) as usize)
            .cloned()
            .unwrap_or_else(SceneModel::default_model),
    };
    let custom_skin = entity.custom_skin.and_then(|skin| {
        shared
            .loaded_skins
            .get(skin.id.saturating_sub(1) as usize)
            .map(|(path, surfaces)| SceneSkin {
                path: path.clone(),
                surfaces: surfaces.clone(),
            })
    });
    RefModelEntity {
        shading: ShadedFields {
            render_flags: entity.render_flags,
            custom_shader: entity.custom_shader.map(|shader| SceneShader::new(shader.name)),
            shader_rgba: entity.shader_rgba,
            shader_tex_coord: vec2(entity.shader_tex_coord[0], entity.shader_tex_coord[1]),
            shader_time: 0.0,
        },
        model,
        origin: entity.origin,
        old_origin: entity.old_origin,
        axis: entity.axis,
        non_normalized_axes: false,
        lighting_origin: entity.lighting_origin,
        shadow_plane: entity.shadow_plane,
        frame: entity.frame,
        old_frame: entity.old_frame,
        back_lerp: entity.back_lerp,
        skin_num: 0,
        custom_skin,
    }
}

/// Admit a retail sprite entity into the recorder.
fn admit_sprite_entity(entity: RetailSpriteEntity) -> RefSpriteEntity {
    RefSpriteEntity {
        shading: ShadedFields {
            render_flags: entity.render_flags,
            custom_shader: entity.custom_shader.map(|shader| SceneShader::new(shader.name)),
            shader_rgba: entity.shader_rgba,
            shader_tex_coord: vec2(0.0, 0.0),
            shader_time: 0.0,
        },
        origin: entity.origin,
        radius: entity.radius,
        rotation: 0.0,
    }
}

impl Q3ResourceHost for Q3AssetResourceHost {
    fn zero_picture(&self) -> RetailSceneShader {
        let order = self
            .shaders
            .borrow_mut()
            .source_default_picture()
            .map(|picture| picture.order)
            .unwrap_or(0);
        RetailSceneShader {
            id: order,
            name: String::new(),
            material_order: order as i32,
        }
    }

    fn load_model(&mut self, path: &str) -> PresentResult<RetailSceneModel> {
        let model = register_q3_model_request(path, |valid| self.load_model_inner(valid))?;
        if model == SceneModel::default_model() {
            return Ok(RetailSceneModel::Default);
        }
        let mut shared = self.shared.borrow_mut();
        if let Some(index) = shared.loaded_models.iter().position(|value| value == &model) {
            return Ok(RetailSceneModel::Loaded { id: index as u32 + 1 });
        }
        shared.loaded_models.push(model);
        Ok(RetailSceneModel::Loaded {
            id: shared.loaded_models.len() as u32,
        })
    }

    fn load_skin(&mut self, path: &str) -> PresentResult<Option<RetailSceneSkin>> {
        let content = self.content_for(path);
        let opened = self.shared.borrow_mut().models.open_skin(&content, path)?;
        let Some(bytes) = opened else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&bytes);
        let surfaces = parse_skin(&text)
            .map_err(|error| Q3AssetError::Load(format!("Q3 skin {path} failed: {error}")))?
            .into_iter()
            .map(|surface| SkinMapping {
                name: surface.name,
                shader: surface.shader,
            })
            .collect::<Vec<_>>();
        let mut shared = self.shared.borrow_mut();
        shared.loaded_skins.push((path.to_string(), surfaces.clone()));
        let id = shared.loaded_skins.len() as u32;
        Ok(Some(RetailSceneSkin {
            id,
            surfaces: surfaces
                .into_iter()
                .map(|surface| RetailSkinSurface {
                    name: surface.name,
                    shader: surface.shader,
                })
                .collect(),
        }))
    }

    fn load_shader(&mut self, path: &str, mip: bool) -> PresentResult<Option<RetailSceneShader>> {
        register_q3_shader_request(path, |valid: String| {
            let picture = self
                .shaders
                .borrow_mut()
                .register_source_picture(&valid, mip)
                .map_err(|error| Q3AssetError::Load(format!("Q3 shader {valid} failed: {error}")))?;
            Ok::<_, Q3AssetError>(picture.map(|picture| RetailSceneShader {
                id: picture.order,
                name: valid,
                material_order: picture.order as i32,
            }))
        })
        .map_err(PresentClientError::from)
    }

    fn load_world_scene(&mut self, _requested_path: &str) -> PresentResult<WorldScene> {
        Ok(WorldScene {
            model_bounds: self.shared.borrow().world.world_model_bounds(),
        })
    }

    fn remap_shader(&mut self, original: &str, replacement: &str, offset: &str) -> PresentResult<()> {
        if let Some(remap) = self.remap_override.as_mut() {
            return remap(original, replacement, offset).map_err(PresentClientError::from);
        }
        let parsed = parse_float_prefix(offset);
        let time_offset = if parsed.is_nan() { 0.0 } else { parsed as f32 };
        // The donor re-reads retirement across an await; the sync call cannot interleave.
        let current = !self.shared.borrow().closed;
        let stale = self
            .shared
            .borrow_mut()
            .world
            .remap_shader(original, replacement, time_offset, &|| current);
        if stale == Q3RemapOutcome::Stale {
            return Err(Q3AssetError::RetiredRemap.into());
        }
        Ok(())
    }

    fn clear_scene(&mut self) {
        self.scene.borrow_mut().clear_scene();
    }

    fn add_ref_entity(&mut self, entity: RetailRefEntity) {
        let admitted = {
            let shared = self.shared.borrow();
            match entity {
                RetailRefEntity::Model(entity) => {
                    Q3AdmittedRefEntity::Entity(RefEntity::Model(admit_model_entity(&shared, entity)))
                }
                RetailRefEntity::Sprite(entity) => {
                    Q3AdmittedRefEntity::Entity(RefEntity::Sprite(admit_sprite_entity(entity)))
                }
            }
        };
        self.scene.borrow_mut().add_ref_entity(&admitted);
    }

    fn add_poly(&mut self, poly: RetailRefPoly) {
        let converted = RefPoly {
            shader: poly.shader.map(|shader| SceneShader::new(shader.name)),
            vertices: poly
                .vertices
                .into_iter()
                .map(|vertex| qa_content::q3::presentation::ref_entity::RefPolyVertex {
                    position: vertex.position,
                    tex_coord: vec2(vertex.tex_coord[0], vertex.tex_coord[1]),
                    color: vertex.color,
                })
                .collect(),
        };
        let _ = self.scene.borrow_mut().add_poly(&converted);
    }

    fn add_light(&mut self, light: RetailDynamicLight) {
        self.scene.borrow_mut().add_light(&light);
    }

    fn render_scene(&mut self, refdef: &RetailRefdef) {
        self.scene.borrow_mut().render_scene(&ContentRefdef {
            x: refdef.x,
            y: refdef.y,
            width: refdef.width,
            height: refdef.height,
            fov_x: refdef.fov_x,
            fov_y: refdef.fov_y,
            view_origin: refdef.view_origin,
            view_axis: refdef.view_axis,
            time: refdef.time,
            render_flags: refdef.render_flags,
            area_mask: refdef.area_mask,
            text: refdef.text.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::render::scene::resources::SceneImageRegistry;
    use qa_client::render::scene::textures::{SceneAsset, SceneAssetReader, SceneTextureLoader};
    use qa_client::render::types::{fresh_owner_identity, ResourceOwner};
    use qa_content::q3::presentation::ref_entity::default_model;
    use qa_content::q3::presentation::resources::{RendererResources, ResourceWorldMap};
    use qa_content::q3::presentation::scene::Q3SceneTarget;
    use qa_core::identity::{ActorId, IdentityOwner, SeatId};
    use qa_core::math::vec3;

    const CONTENT: &str = "q3:classic:baseq3:1";
    const CHARACTER: &str = "q3:classic:missionpack:1";

    fn content(id: &str) -> ContentId {
        ContentId(id.to_string())
    }

    fn wav_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[128u8, 140, 128, 116]);
        bytes
    }

    #[derive(Default)]
    struct StubMounts {
        files: HashMap<(String, String), OpenedQ3Asset>,
        archives: Vec<(String, Vec<String>)>,
        plan: Vec<String>,
        loose: Vec<PathBuf>,
    }

    impl StubMounts {
        fn with_file(mut self, content: &str, path: &str, id: &str, bytes: Vec<u8>) -> Self {
            self.files.insert(
                (content.to_string(), path.to_lowercase()),
                OpenedQ3Asset {
                    id: id.to_string(),
                    bytes,
                },
            );
            self
        }
    }

    impl Q3AssetMounts for StubMounts {
        fn open(&mut self, content: &ContentId, path: &str) -> Result<Option<OpenedQ3Asset>, Q3AssetError> {
            Ok(self.files.get(&(content.to_string(), path.to_lowercase())).cloned())
        }

        fn resolve(&mut self, content: &ContentId, path: &str) -> bool {
            self.files.contains_key(&(content.to_string(), path.to_lowercase()))
        }

        fn catalog_archives(&self) -> Vec<(String, Vec<String>)> {
            self.archives.clone()
        }

        fn plan_archives(&self, _content: &ContentId) -> Vec<String> {
            self.plan.clone()
        }

        fn loose_roots(&self, _content: &ContentId) -> Vec<PathBuf> {
            self.loose.clone()
        }
    }

    #[derive(Default)]
    struct StubModels {
        models: HashMap<(String, String), Q3LoadedModel>,
        skins: HashMap<(String, String), Vec<u8>>,
        bounds: HashMap<(String, usize), Bounds>,
    }

    impl Q3AssetModels for StubModels {
        fn load_model(&mut self, content: &ContentId, path: &str) -> Result<Q3LoadedModel, Q3AssetError> {
            self.models
                .get(&(content.to_string(), path.to_string()))
                .cloned()
                .ok_or_else(|| Q3AssetError::Load(format!("missing model {path}")))
        }

        fn open_skin(&mut self, content: &ContentId, path: &str) -> Result<Option<Vec<u8>>, Q3AssetError> {
            Ok(self.skins.get(&(content.to_string(), path.to_string())).cloned())
        }

        fn inline_bounds(&mut self, world: &PresentWorld, index: usize) -> Option<Bounds> {
            self.bounds.get(&(world.name.clone(), index)).copied()
        }
    }

    struct StubCatalog {
        character: ContentId,
        weapons: Vec<ContentId>,
    }

    impl Q3AssetCatalog for StubCatalog {
        fn character_content(&self) -> ContentId {
            self.character.clone()
        }

        fn weapon_contents(&self) -> Vec<ContentId> {
            self.weapons.clone()
        }

        fn family_of(&self, content: &ContentId) -> GameFamily {
            if content.as_str().contains("missionpack") || content.as_str().contains("baseq3") {
                GameFamily::Q3
            } else {
                GameFamily::Q2
            }
        }
    }

    struct StubWorld {
        bounds: Vec<Bounds>,
        remaps: Vec<(String, String, f32)>,
        outcome: Q3RemapOutcome,
    }

    impl Q3AssetWorld for StubWorld {
        fn world_model_bounds(&self) -> Vec<Bounds> {
            self.bounds.clone()
        }

        fn remap_shader(
            &mut self,
            original: &str,
            replacement: &str,
            time_offset: f32,
            current: &dyn Fn() -> bool,
        ) -> Q3RemapOutcome {
            assert!(current());
            self.remaps
                .push((original.to_string(), replacement.to_string(), time_offset));
            self.outcome
        }

        fn fog_selections(&self) -> Vec<Q3FogSelection> {
            Vec::new()
        }
    }

    struct FakeReader;

    impl SceneAssetReader for FakeReader {
        fn read(&self, _path: &str) -> Result<Option<SceneAsset>, qa_client::render::error::RenderError> {
            Ok(None)
        }
    }

    fn shaders() -> SceneShaderRegistry {
        let session = IdentityOwner::create("q3-assets-test")
            .expect("owner")
            .session()
            .clone();
        let owner = ResourceOwner::new(fresh_owner_identity(), session, 0);
        let images = SceneImageRegistry::new(owner);
        let loader = SceneTextureLoader::new(images, Box::new(FakeReader), None, None, 224).expect("loader");
        SceneShaderRegistry::with_defaults(loader)
    }

    fn options() -> ApplicationQ3AssetOptions {
        let prints: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        ApplicationQ3AssetOptions {
            content: content(CONTENT),
            mounts: Box::new(
                StubMounts::default()
                    .with_file(CONTENT, "sound/test.wav", "resource:sound", wav_bytes())
                    .with_file(CONTENT, "scripts/test.cfg", "resource:cfg", b"seta x 1\n".to_vec()),
            ),
            models: Box::new(StubModels::default()),
            catalog: Box::new(StubCatalog {
                character: content(CHARACTER),
                weapons: vec![content("q2:classic:baseq2:1"), content(CHARACTER)],
            }),
            world: Box::new(StubWorld {
                bounds: Vec::new(),
                remaps: Vec::new(),
                outcome: Q3RemapOutcome::Applied,
            }),
            shaders: shaders(),
            print: Box::new(move |line: &str| prints.borrow_mut().push(line.to_string())),
            save_font_data: Box::new(|| false),
            user_root: None,
            scope: Q3AssetScope::Selected,
            mode: Q3AssetMode::GuestAsync,
        }
    }

    fn assets() -> ApplicationQ3Assets {
        ApplicationQ3Assets::create(options()).expect("assets")
    }

    #[test]
    fn model_request_defaults_invalid_paths() {
        let model = register_q3_model_request("../escape.md3", |_| Ok::<_, Q3AssetError>(SceneModel::default_model()))
            .expect("request");
        assert_eq!(model, SceneModel::default_model());
        let model = register_q3_model_request("*1", |path| {
            assert_eq!(path, "*1");
            Ok::<_, Q3AssetError>(SceneModel::default_model())
        })
        .expect("inline");
        assert_eq!(model, SceneModel::default_model());
    }

    #[test]
    fn shader_request_rejects_invalid_paths() {
        let result: Result<Option<String>, Q3AssetError> =
            register_q3_shader_request("../escape", |_| Ok(Some("loaded".to_string())));
        assert_eq!(result.expect("request"), None);
    }

    #[test]
    fn float_prefix_matches_donor_parse() {
        assert_eq!(parse_float_prefix("0.5"), 0.5);
        assert_eq!(parse_float_prefix("12abc"), 12.0);
        assert!(parse_float_prefix("abc").is_nan());
        assert!(parse_float_prefix("").is_nan());
        assert_eq!(parse_float_prefix("-2.5e2"), -250.0);
        assert_eq!(parse_float_prefix("Infinity"), f64::INFINITY);
    }

    #[test]
    fn sync_filter_matches_donor() {
        assert!(retain_sync_name("ui/main.menu"));
        assert!(retain_sync_name("sound/hit.wav"));
        assert!(retain_sync_name("sound/hit.ogg"));
        assert!(!retain_sync_name("sound/hit.mp3"));
        assert!(!retain_sync_name("models/player.md3"));
        assert!(!retain_sync_name("noextension"));
    }

    #[test]
    fn census_combines_archives_and_loose_roots() {
        let root = std::env::temp_dir().join(format!("qa-q3c-census-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sound")).expect("mkdir");
        std::fs::write(root.join("sound/loose.wav"), [0u8; 4]).expect("write");
        std::fs::write(root.join("top.cfg"), [1u8; 2]).expect("write");
        let mut opts = options();
        opts.mounts = Box::new(StubMounts {
            files: HashMap::new(),
            archives: vec![("pak0.pk3".to_string(), vec!["UI/HUD.MENU".to_string()])],
            plan: vec!["pak0.pk3".to_string()],
            loose: vec![root.clone()],
        });
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        assert!(assets.has("UI/hud.menu"));
        assert!(assets.has("sound/loose.wav"));
        assert!(assets.has("top.cfg"));
        assert_eq!(assets.list("sound/"), vec!["sound/loose.wav".to_string()]);
        assert_eq!(
            assets.list(""),
            vec![
                "sound/loose.wav".to_string(),
                "top.cfg".to_string(),
                "ui/hud.menu".to_string()
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_loose_root_reads_empty() {
        let mut opts = options();
        opts.mounts = Box::new(StubMounts {
            loose: vec![PathBuf::from("/nonexistent-qa-q3c-root-9f8e7d")],
            ..StubMounts::default()
        });
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        assert!(assets.list("").is_empty());
    }

    #[test]
    fn source_sync_retains_scripts_and_sounds() {
        let mut opts = options();
        opts.mode = Q3AssetMode::SourceSync;
        opts.mounts = Box::new(
            StubMounts {
                archives: vec![(
                    "pak0.pk3".to_string(),
                    vec!["sound/test.wav".to_string(), "scripts/test.cfg".to_string()],
                )],
                plan: vec!["pak0.pk3".to_string()],
                ..StubMounts::default()
            }
            .with_file(CONTENT, "sound/test.wav", "resource:sound", wav_bytes())
            .with_file(CONTENT, "scripts/test.cfg", "resource:cfg", b"seta x 1\n".to_vec()),
        );
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        assert!(assets.read_sync("sound/test.wav").is_ok());
        assert!(assets.read_sync("scripts/test.cfg").is_ok());
        assert!(assets.read_sync("sound/other.wav").is_err());
    }

    #[test]
    fn reads_retain_and_report_missing() {
        let mut assets = assets();
        assert_eq!(assets.read("scripts/test.cfg").expect("read"), b"seta x 1\n");
        assert!(assets.read("missing.cfg").expect("missing").is_empty());
        assert_eq!(assets.read_file_length("scripts/test.cfg").expect("length"), 9);
        assert!(assets.read_file_length("missing.cfg").is_err());
        assert!(assets.read_sync("scripts/test.cfg").is_ok());
    }

    #[test]
    fn sounds_decode_and_memoize() {
        let mut assets = assets();
        assert!(assets.read("sound/test.wav").is_ok());
        let first = assets.sound("test.wav").expect("sound");
        assert_eq!(first.resource, "resource:sound");
        assert_eq!(first.name, "sound/test.wav");
        let second = assets.sound("#sound/test.wav").expect("memoized");
        assert_eq!(second.pcm.sample_rate, first.pcm.sample_rate);
        assert!(assets.sound("missing.wav").is_none());
    }

    #[test]
    fn undecodable_sounds_read_missing() {
        let mut opts = options();
        opts.mounts = Box::new(StubMounts::default().with_file(CONTENT, "sound/bad.wav", "resource:bad", vec![0u8; 8]));
        let mut assets = ApplicationQ3Assets::create(opts).expect("assets");
        assert!(assets.read("sound/bad.wav").is_ok());
        assert!(assets.sound("bad.wav").is_none());
    }

    #[test]
    fn content_routing_follows_recipe() {
        let assets = assets();
        assert_eq!(assets.content_for("models/players/sarge/head.md3"), content(CHARACTER));
        assert_eq!(
            assets.content_for("models/weapons2/railgun/rail.md3"),
            content(CHARACTER)
        );
        assert_eq!(assets.content_for("models/mapobjects/chair.md3"), content(CONTENT));
    }

    #[test]
    fn host_loads_models_with_routing() {
        let assets = assets();
        let target = StubTarget::new();
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(target)))));
        let missing = host.load_model("models/missing.md3").expect("missing");
        assert_eq!(missing, RetailSceneModel::Default);
        assert!(assets.provider_slot(&SceneModel::default_model()).is_none());
    }

    #[test]
    fn host_records_provider_slots() {
        let mut opts = options();
        let mut models = StubModels::default();
        models.models.insert(
            (CHARACTER.to_string(), "models/players/sarge/head.md3".to_string()),
            Q3LoadedModel {
                model: Q3AssetModel::Decoded(Q3DecodedModel::Bounded {
                    bounds: Bounds {
                        min: vec3(0.0, 0.0, 0.0),
                        max: vec3(1.0, 1.0, 1.0),
                    },
                }),
                resource: PresentResource::new("models/players/sarge/head.md3"),
            },
        );
        opts.models = Box::new(models);
        opts.mounts = Box::new(StubMounts::default().with_file(
            CHARACTER,
            "models/players/sarge/head.md3",
            "resource:head",
            vec![1],
        ));
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        let loaded = host.load_model("models/players/sarge/head.md3").expect("load");
        assert!(matches!(loaded, RetailSceneModel::Loaded { id: 1 }));
        assert_eq!(assets.provider_content(0), Some(content(CHARACTER)));
    }

    #[test]
    fn host_rejects_missing_inline_bounds() {
        let mut opts = options();
        let mut models = StubModels::default();
        models.models.insert(
            (CONTENT.to_string(), "*3".to_string()),
            Q3LoadedModel {
                model: Q3AssetModel::Brush {
                    world: PresentWorld::new("world"),
                    model: 3,
                },
                resource: PresentResource::new("*3"),
            },
        );
        opts.models = Box::new(models);
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        let error = host.load_model("*3").expect_err("bounds");
        assert!(error.to_string().contains("Missing inline model bounds"));
    }

    #[test]
    fn host_loads_skins() {
        let mut opts = options();
        let mut models = StubModels::default();
        models.skins.insert(
            (CHARACTER.to_string(), "models/players/sarge/head.skin".to_string()),
            b"head,models/players/sarge/head.tga\n".to_vec(),
        );
        opts.models = Box::new(models);
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        assert!(host.load_skin("models/missing.skin").expect("missing").is_none());
        let skin = host
            .load_skin("models/players/sarge/head.skin")
            .expect("skin")
            .expect("some");
        assert_eq!(skin.surfaces.len(), 1);
        assert_eq!(skin.surfaces[0].name, "head");
    }

    #[test]
    fn host_loads_shaders() {
        let assets = assets();
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        assert!(host.load_shader("../escape", true).expect("invalid").is_none());
    }

    #[test]
    fn host_reports_world_bounds() {
        let mut opts = options();
        opts.world = Box::new(StubWorld {
            bounds: vec![Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(8.0, 8.0, 8.0),
            }],
            remaps: Vec::new(),
            outcome: Q3RemapOutcome::Applied,
        });
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        let world = host.load_world_scene("maps/test.bsp").expect("world");
        assert_eq!(world.model_bounds.len(), 1);
    }

    #[test]
    fn host_remaps_through_world_or_override() {
        let assets = assets();
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        host.remap_shader("a", "b", "0.5").expect("remap");
        host.set_remap_override(Box::new(|original, replacement, offset| {
            assert_eq!((original, replacement, offset), ("x", "y", "z"));
            Ok(())
        }));
        host.remap_shader("x", "y", "z").expect("override");
    }

    #[test]
    fn host_rejects_stale_remaps() {
        let mut opts = options();
        opts.world = Box::new(StubWorld {
            bounds: Vec::new(),
            remaps: Vec::new(),
            outcome: Q3RemapOutcome::Stale,
        });
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        let error = host.remap_shader("a", "b", "nan-prefix").expect_err("stale");
        assert!(error.to_string().contains("retired source request"));
    }

    #[test]
    fn host_drives_recorder() {
        let assets = assets();
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        let mut entity = qa_content::q3::presentation::retail_snapshot::create_model_entity();
        entity.origin = vec3(1.0, 2.0, 3.0);
        host.add_ref_entity(RetailRefEntity::Model(entity));
        host.add_light(RetailDynamicLight {
            origin: vec3(0.0, 0.0, 8.0),
            radius: 64.0,
            color: vec3(1.0, 1.0, 1.0),
            additive: false,
        });
        let content = host.scene.borrow().capture();
        assert_eq!(content.admission.entities.len(), 1);
        assert_eq!(content.lights.len(), 1);
        host.clear_scene();
        assert!(host.scene.borrow().capture().admission.entities.is_empty());
    }

    #[test]
    fn host_renders_through_target() {
        let assets = assets();
        let target = StubTarget::new();
        let published = target.published.clone();
        let mut host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(target)))));
        let refdef = qa_content::q3::presentation::retail_snapshot::create_refdef();
        host.render_scene(&refdef);
        assert_eq!(published.borrow().len(), 1);
    }

    #[test]
    fn zero_picture_falls_back_without_sources() {
        let assets = assets();
        let host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        assert_eq!(host.zero_picture().id, 0);
    }

    #[test]
    fn resources_register_through_host() {
        struct StubResourceWorld;
        impl qa_content::q3::presentation::resources::ResourceWorld for StubResourceWorld {
            fn resource_map(&self) -> &ResourceWorldMap {
                static MAP: std::sync::OnceLock<ResourceWorldMap> = std::sync::OnceLock::new();
                MAP.get_or_init(|| ResourceWorldMap {
                    entities: String::new(),
                    nodes: Vec::new(),
                    leaves: Vec::new(),
                    planes: Vec::new(),
                })
            }

            fn cluster_pvs_byte(&self, _cluster: i32, _offset: usize) -> u8 {
                0
            }
        }

        let mut opts = options();
        let mut models = StubModels::default();
        models.models.insert(
            (CONTENT.to_string(), "models/box.md3".to_string()),
            Q3LoadedModel {
                model: Q3AssetModel::Decoded(Q3DecodedModel::Framed {
                    frames: vec![Bounds {
                        min: vec3(0.0, 0.0, 0.0),
                        max: vec3(1.0, 1.0, 1.0),
                    }],
                }),
                resource: PresentResource::new("models/box.md3"),
            },
        );
        opts.models = Box::new(models);
        opts.mounts = Box::new(StubMounts::default().with_file(CONTENT, "models/box.md3", "resource:box", vec![2]));
        let assets = ApplicationQ3Assets::create(opts).expect("assets");
        let host = assets.resource_host(Rc::new(RefCell::new(Q3SceneRecorder::new(Box::new(StubTarget::new())))));
        let mut resources =
            qa_content::q3::presentation::resources::Q3RendererResources::<_, StubResourceWorld>::new(host);
        let model = resources.register_model(Some("models/box.md3")).expect("register");
        assert_eq!(resources.model_handle(&model).expect("handle"), 1);
        assert_eq!(default_model(), SceneModel::default_model());
    }

    #[test]
    fn ui_fonts_load_legacy_pictures() {
        let mut assets = assets();
        let fonts = assets.with_ui_fonts(|ui| ui.load_legacy_fonts()).expect("legacy");
        for picture in [fonts.charset, fonts.proportional, fonts.glow, fonts.banner] {
            assert!(matches!(picture, PictureAsset::Material(material) if material.order >= 1));
        }
    }

    #[test]
    fn font_record_reports_missing() {
        let mut assets = assets();
        let mut table = vec![0u8; 20548];
        let result = assets.register_font_record(None, 12, &mut table);
        assert!(result.is_ok());
    }

    #[test]
    fn closed_assets_reject_fonts() {
        let mut assets = assets();
        assets.close();
        assert!(assets.is_closed());
        assert!(assets.with_fonts(|_| {}).is_err());
        let mut table = vec![0u8; 20548];
        assert!(assets.register_font_record(None, 12, &mut table).is_err());
    }

    #[test]
    fn present_error_mapping() {
        let error: PresentClientError = Q3AssetError::RetiredRemap.into();
        assert!(error.to_string().contains("retired"));
    }

    struct StubTarget {
        published: Rc<RefCell<Vec<qa_content::q3::presentation::scene::Q3PresentedScene>>>,
        seat: SeatId,
    }

    impl StubTarget {
        fn new() -> Self {
            let owner = IdentityOwner::create("q3-assets-target").expect("owner");
            Self {
                published: Rc::new(RefCell::new(Vec::new())),
                seat: owner.seat(0),
            }
        }
    }

    impl Q3SceneTarget for StubTarget {
        fn seat(&self) -> SeatId {
            self.seat.clone()
        }

        fn viewport(&self) -> qa_content::q3::presentation::scene::Rect {
            qa_content::q3::presentation::scene::Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            }
        }

        fn far_clip(&self) -> f32 {
            16384.0
        }

        fn near_clip(&self) -> f32 {
            4.0
        }

        fn rail(&self) -> qa_content::q3::presentation::scene::RailSettings {
            qa_content::q3::presentation::scene::RailSettings
        }

        fn fog_selections(&self) -> Vec<Q3FogSelection> {
            Vec::new()
        }

        fn print(&mut self, _text: &str) {}

        fn actor(&self, _entity: &RefModelEntity) -> Option<ActorId> {
            None
        }

        fn publish(&mut self, scene: qa_content::q3::presentation::scene::Q3PresentedScene) {
            self.published.borrow_mut().push(scene);
        }
    }
}
