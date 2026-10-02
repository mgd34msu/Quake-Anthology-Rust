//! Port of Quake-Anthology-TS `src/app/bootstrap/assets.ts`
//!
//! Application asset cache: per-content scene providers, world scenes,
//! console fonts, menu typography, image-refresh rebinding, and model
//! assets (`ProviderSceneAssets`, `PreparedApplicationImageBinding`,
//! `PreparedApplicationImages`, `ModelAsset`, `ApplicationAssets`).
//!
//! Sync port: the donor's promises become cached values and its async
//! loads become sync host calls, so the provider/model failure paths
//! that delete a rejected cache entry collapse to never inserting. The
//! scene graph behind providers (texture loaders, shader registries,
//! world scenes, material movies, remap tables) arrives through the
//! [`AssetScene`] backend: the render-scene ports take texture readers
//! as `'static` and registries/images by value, which does not admit
//! the donor's shared per-content registry graph, so the host owns
//! that graph while every construction, refresh, and close step keeps
//! the donor's order and error text here. Consequences of that split:
//!
//! - Material movies resolve host-side during shader compilation
//!   (donor `materialMovie`); the host dedupes by `content\0path`,
//!   plays silent loops, and releases movies with its shaders.
//! - There is no shared image registry and no media clock: registry
//!   forks and clocked animations were not carried over by the scene
//!   ports.
//! - [`ModelAsset`] brush models carry the submodel index only; the
//!   decoded geometry lives in the retained brush scene.
//! - Model variants key by model cache key (donor keys by variants
//!   identity; each cached model owns at most one live variants
//!   object), and worlds track their provider content (donor compares
//!   shader-registry identity; each provider owns exactly one live
//!   registry).
//!
//! Fonts arrive through `menu-font`'s [`MenuCharsetImages`],
//! [`MountedMenuFonts`], and [`TypographyMounts`] hosts. `commit` and
//! `discard` take the assets back explicitly (donor closures).

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use qa_bots::entities::{parse_entities, EntityError};
use qa_client::render::scene::image_policy::ImagePolicy;
use qa_client::render::scene::models::replacements::{
    load_model_replacement, ModelReplacementPolicy, DEFAULT_MODEL_REPLACEMENT_POLICY,
};
use qa_client::text::atlas::TextFontSelection;
use qa_content::bsp::{read_q1_bsp, Q1BspOptions, Q1Map};
use qa_content::bsp2::{read_q2_bsp, to_q2_world_geometry, Q2DecodedMap};
use qa_content::bsp3::{decode_q3_world, Q3DecodedWorld};
use qa_content::catalog::CatalogError;
use qa_content::contract::{ContentId, GameFamily, ResolvedResourceReference, ResourceProvenance};
use qa_content::images::indexed::decode_pcx;
use qa_content::images::palette::decode_palette;
use qa_content::images::Palette;
use qa_content::mounts::{MountError, MountedContent, OpenedResource};
use qa_content::replacements::QFamily;
use qa_content::{classify_bsp, BspKind};
use qa_core::binary::BinaryError;
use thiserror::Error;

use super::content::{ApplicationContentPreparer, ApplicationWorld, ContentError, LoadedApplicationContent};
use super::menu_font::{
    load_menu_font, load_menu_typography, LoadedMenuFont, MenuCharsetImages, MenuFontError, MenuTypography,
    MountedMenuFonts, TypographyMounts,
};
use super::model_loader::{
    load_application_model, ApplicationModelProvider, ApplicationModelVariants, LoadedModel, ModelLoaderError,
    ModelMounts, ModelTextureLoader, ReplacementCommit,
};

/// Menu typography with a boxed closer.
pub type BoxedMenuTypography = MenuTypography<Box<dyn FnOnce()>>;

/// Live/staged shader-registry pairs for remap resolution.
pub type ShaderRegistryPairs<'p, 't, S> = Vec<(&'p <S as AssetScene>::Shaders, &'t <S as AssetScene>::Shaders)>;

/// Failure of application asset loading.
#[derive(Debug, Error)]
pub enum AssetsError<E> {
    /// Assets are closed.
    #[error("{0}")]
    Closed(String),
    /// World presentation has not loaded.
    #[error("World presentation has not loaded")]
    WorldNotLoaded,
    /// Refresh cannot start or finish.
    #[error("{0}")]
    Refresh(String),
    /// Content has no palette or colormap entry.
    #[error("{0}")]
    Palette(String),
    /// Model path or submodel is invalid.
    #[error("{0}")]
    Model(String),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Content failure.
    #[error(transparent)]
    Content(#[from] ContentError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Decode failure (maps, palettes, and images).
    #[error(transparent)]
    Binary(#[from] BinaryError),
    /// Model loading failure.
    #[error(transparent)]
    ModelLoad(#[from] ModelLoaderError),
    /// Font loading failure.
    #[error(transparent)]
    Font(#[from] MenuFontError),
    /// Entity parse failure.
    #[error(transparent)]
    Entities(#[from] EntityError),
    /// Filesystem failure scanning loose shader scripts.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Scene backend failure.
    #[error(transparent)]
    Scene(E),
}

/// Borrowed decoded world geometry ready for scene loading.
pub enum WorldGeometry<'g> {
    /// Quake map.
    Q1(&'g Q1Map<'g>),
    /// Quake II world geometry.
    Q2(&'g Q2DecodedMap<'g>),
    /// Quake III world.
    Q3(&'g Q3DecodedWorld),
}

/// Scene-graph backend behind [`ApplicationAssets`] (donor render/scene
/// plus media surfaces: texture loaders, shader registries, world
/// scenes, material movies, and remap tables).
pub trait AssetScene {
    /// Per-provider texture loader (donor `SceneTextureLoader`).
    type Textures;
    /// Per-provider shader registry (donor `SceneShaderRegistry`).
    type Shaders;
    /// Loaded world scene (donor `WorldScene`).
    type World;
    /// Prepared remap refresh (donor `prepareRemapRefresh` handle).
    type Remaps;
    /// Backend failure.
    type Error: std::error::Error + 'static;

    /// Create a texture loader over mounts, palette, and policy.
    fn create_textures(
        &mut self,
        mounts: &MountedContent,
        palette: Option<&Palette>,
        policy: Option<&ImagePolicy>,
    ) -> Result<Self::Textures, Self::Error>;
    /// Stage `replacement` from `current` (donor `prepareReplacement`).
    fn prepare_texture_replacement(
        &mut self,
        current: &Self::Textures,
        replacement: &mut Self::Textures,
    ) -> Result<(), Self::Error>;
    /// Dispose a loader's images (donor `disposeImages`).
    fn dispose_texture_images(&mut self, textures: &mut Self::Textures);
    /// Close a loader (donor `close`).
    fn close_textures(&mut self, textures: &mut Self::Textures);
    /// Load one model skin texture (donor loader `load`).
    fn load_model_texture(
        &mut self,
        textures: &mut Self::Textures,
        name: &str,
        family: GameFamily,
        usage: &'static str,
    ) -> Result<(), Self::Error>;

    /// Create a shader registry over a texture loader (donor
    /// constructor; movies resolve host-side during compilation).
    fn create_shaders(&mut self, textures: &Self::Textures, family: GameFamily) -> Result<Self::Shaders, Self::Error>;
    /// Add one shader script (donor `addScript`).
    fn add_shader_script(&mut self, shaders: &mut Self::Shaders, text: &str, path: &str) -> Result<(), Self::Error>;
    /// Initialize source materials from loaded scripts (donor
    /// `initializeSourceMaterials`).
    fn initialize_source_materials(
        &mut self,
        shaders: &mut Self::Shaders,
        scripts: &[(&str, &str)],
    ) -> Result<(), Self::Error>;
    /// Wrap staged textures in a replacement registry (donor
    /// `replacement`).
    fn replace_shaders(&mut self, current: &Self::Shaders, textures: &Self::Textures) -> Self::Shaders;
    /// Stage a replacement registry (donor `prepareReplacement`).
    fn prepare_shader_replacement(
        &mut self,
        current: &Self::Shaders,
        replacement: &mut Self::Shaders,
    ) -> Result<(), Self::Error>;
    /// Validate a staged registry (donor `validateReplacement`).
    fn validate_shader_replacement(
        &self,
        current: &Self::Shaders,
        replacement: &Self::Shaders,
    ) -> Result<(), Self::Error>;
    /// Commit a staged registry (donor `commitReplacement`; the
    /// caller swaps and retires the loaders itself).
    fn commit_shader_replacement(&mut self, current: &mut Self::Shaders, replacement: Self::Shaders);
    /// Discard a staged registry and dispose its staged loader (donor
    /// `discardReplacement` plus `disposeImages`).
    fn discard_shader_replacement(&mut self, replacement: &mut Self::Shaders, textures: &mut Self::Textures);
    /// Close a registry, releasing its movies (donor movie loop).
    fn close_shaders(&mut self, shaders: &mut Self::Shaders);

    /// Prepare a remap refresh over `(current, replacement)` registry
    /// pairs (donor `prepareRemapRefresh`).
    fn prepare_remap_refresh(
        &mut self,
        pairs: &[(&Self::Shaders, &Self::Shaders)],
    ) -> Result<Self::Remaps, Self::Error>;
    /// Validate prepared remaps (donor `validate`).
    fn validate_remaps(&self, remaps: &Self::Remaps) -> Result<(), Self::Error>;
    /// Commit prepared remaps (donor `commit`).
    fn commit_remaps(&mut self, remaps: &mut Self::Remaps);

    /// Load a world scene (donor `WorldScene.load`).
    fn load_world_scene(
        &mut self,
        geometry: &WorldGeometry<'_>,
        shaders: &Self::Shaders,
        q2_sky: Option<&str>,
        q2_light_modulate: f32,
    ) -> Result<Self::World, Self::Error>;
    /// Prepare replacement world images; `staged` carries every
    /// `(current, replacement)` registry pair so the host can resolve
    /// remapped materials (donor resolver closure).
    fn prepare_world_images(
        &mut self,
        world: &Self::World,
        shaders: &Self::Shaders,
        staged: &[(&Self::Shaders, &Self::Shaders)],
    ) -> Result<Self::World, Self::Error>;
    /// Validate replacement world images (donor `validateImages`).
    fn validate_world_images(&self, world: &Self::World, replacement: &Self::World) -> Result<(), Self::Error>;
    /// Commit replacement world images (donor `commitImages`).
    fn commit_world_images(&mut self, world: &mut Self::World, replacement: Self::World);
    /// Close a world scene (donor `close`).
    fn close_world(&mut self, world: &mut Self::World);
    /// Leaf count of a loaded world (donor `world.map.leaves.length`).
    fn world_leaf_count(&self, world: &Self::World) -> usize;
}

/// Per-content scene provider (donor `ProviderSceneAssets`).
pub struct ProviderSceneAssets<S: AssetScene> {
    /// Live model replacement policy (donor getter over the assets).
    pub model_policy: ModelReplacementPolicy,
    /// Product family.
    pub family: GameFamily,
    /// Content mounts.
    pub mounts: Rc<MountedContent>,
    /// Source palette, unless Quake III.
    pub palette: Option<Palette>,
    /// Texture loader.
    pub textures: S::Textures,
    /// Shader registry over the loader.
    pub shaders: S::Shaders,
}

impl<S: AssetScene> ProviderSceneAssets<S> {
    /// Borrow the provider as a scene view.
    pub fn view(&self) -> ProviderSceneView<'_, S> {
        ProviderSceneView {
            model_policy: self.model_policy.clone(),
            family: self.family,
            mounts: Rc::clone(&self.mounts),
            palette: self.palette.clone(),
            textures: &self.textures,
            shaders: &self.shaders,
        }
    }
}

/// Borrowed provider scene (donor `ProviderSceneAssets` spread).
pub struct ProviderSceneView<'s, S: AssetScene> {
    /// Model replacement policy.
    pub model_policy: ModelReplacementPolicy,
    /// Product family.
    pub family: GameFamily,
    /// Content mounts.
    pub mounts: Rc<MountedContent>,
    /// Source palette, unless Quake III.
    pub palette: Option<Palette>,
    /// Texture loader (staged during refresh).
    pub textures: &'s S::Textures,
    /// Shader registry (staged during refresh).
    pub shaders: &'s S::Shaders,
}

/// Decoded model payload (donor `DecodedModel`).
#[derive(Debug, Clone)]
pub enum AssetModel {
    /// Decoded alias or mesh model.
    Alias(LoadedModel),
    /// Brush submodel; geometry lives in the retained brush scene.
    Brush {
        /// Submodel index.
        model: usize,
    },
}

/// Loaded model asset (donor `ModelAsset`).
pub struct ModelAsset<'s, S: AssetScene> {
    /// Source resource.
    pub resource: &'s ResolvedResourceReference,
    /// Decoded model.
    pub model: &'s AssetModel,
    /// Owning provider.
    pub provider: &'s ProviderSceneAssets<S>,
    /// Retaining brush scene, for brush models.
    pub brush_scene: Option<&'s S::World>,
}

/// Which retained scene backs a cached brush model.
#[derive(Debug, Clone, Copy)]
enum BrushSceneRef {
    /// Current world scene.
    Current,
    /// One staged brush scene.
    Stored(usize),
}

/// Cached model asset with its variants and scene routing.
struct CachedModel {
    /// Source resource.
    resource: ResolvedResourceReference,
    /// Decoded model.
    model: AssetModel,
    /// Replacement variants, for matching-family alias models.
    variants: Option<ApplicationModelVariants>,
    /// Owning provider content.
    provider: ContentId,
    /// Retaining brush scene, for brush models.
    brush: Option<BrushSceneRef>,
}

/// Loaded world scene with its provider content.
struct WorldEntry<S: AssetScene> {
    /// Scene handle.
    scene: S::World,
    /// Provider content that built it.
    provider: ContentId,
}

/// Donor family name for error text.
fn family_name(family: GameFamily) -> &'static str {
    match family {
        GameFamily::Q1 => "q1",
        GameFamily::Q2 => "q2",
        GameFamily::Q3 => "q3",
    }
}

/// Whether the loader reads enhanced replacements for a family (donor
/// `loadModelReplacement`; Quake III has no replacements).
fn replacement_enabled(family: GameFamily, policy: &ModelReplacementPolicy) -> bool {
    match family {
        GameFamily::Q1 => load_model_replacement(QFamily::Q1, policy),
        GameFamily::Q2 => load_model_replacement(QFamily::Q2, policy),
        GameFamily::Q3 => false,
    }
}

/// Mounted content behind a resource reference.
fn provenance_content(provenance: &ResourceProvenance) -> &ContentId {
    match provenance {
        ResourceProvenance::Archive { mount, .. } => &mount.identity.content,
        ResourceProvenance::Loose { mount, .. } => &mount.identity.content,
    }
}

/// Submodel count of a decoded world (donor `world.models.length`).
fn submodel_count(world: &ApplicationWorld) -> usize {
    match world {
        ApplicationWorld::Q1(map) => map.models.len(),
        ApplicationWorld::Q2(map) => map.models.len(),
        ApplicationWorld::Q3(world) => world.models.len(),
    }
}

/// Parse an inline `*N` model path the way `Number` plus
/// `isSafeInteger` does (donor `model`).
fn inline_model_index(path: &str) -> Option<usize> {
    let digits = path.strip_prefix('*')?;
    let trimmed = digits.trim();
    if trimmed.is_empty() {
        return Some(0);
    }
    let value = if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()? as f64
    } else {
        trimmed.parse::<f64>().ok()?
    };
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > 9_007_199_254_740_991.0 {
        return None;
    }
    Some(value as usize)
}

/// Whether an archive entry is a top-level `.shader` script (donor
/// `/^scripts\/[^/]+\.shader$/i`).
fn is_archive_shader_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let Some(name) = lower.strip_prefix("scripts/") else {
        return false;
    };
    name.len() > ".shader".len() && !name.contains('/') && name.ends_with(".shader")
}

/// Mount adapter from shared content mounts (donor provider mounts).
struct MountAdapter {
    /// Shared mounts.
    mounts: Rc<MountedContent>,
}

impl ModelMounts for MountAdapter {
    fn open(&mut self, path: &str) -> Result<Option<OpenedResource>, ModelLoaderError> {
        Ok(self.mounts.open(path, |_| true)?)
    }
}

/// Model skin texture bridge over a provider loader.
struct TextureBridge<'b, S: AssetScene> {
    /// Scene backend.
    scene: &'b mut S,
    /// Target loader.
    textures: &'b mut S::Textures,
}

impl<S: AssetScene> ModelTextureLoader for TextureBridge<'_, S> {
    fn load(&mut self, name: &str, family: GameFamily, usage: &'static str) -> Result<(), ModelLoaderError> {
        self.scene
            .load_model_texture(self.textures, name, family, usage)
            .map_err(|error| ModelLoaderError::Texture(error.to_string()))
    }
}

/// Constructor options (donor `ApplicationAssets` options; the owner,
/// media clock, and external registry have no scene-port counterpart).
#[derive(Debug, Clone, Default)]
pub struct ApplicationAssetOptions {
    /// Image policy for new loaders.
    pub image_policy: Option<ImagePolicy>,
    /// Model replacement policy.
    pub model_policy: Option<ModelReplacementPolicy>,
}

/// Presentation caches over loaded application content (donor
/// `ApplicationAssets`).
pub struct ApplicationAssets<
    'a,
    P: ApplicationContentPreparer,
    S: AssetScene,
    C: MenuCharsetImages,
    F: MountedMenuFonts,
    M: TypographyMounts,
> {
    /// Loaded application content.
    pub content: LoadedApplicationContent<'a, P>,
    /// Scene backend.
    scene: S,
    /// Font charset images.
    charset_images: C,
    /// Mounted font selection.
    fonts_host: F,
    /// Typography mount opener.
    typography_mounts: M,
    /// Image policy for new loaders.
    image_policy: Option<ImagePolicy>,
    /// Model replacement policy.
    model_policy: ModelReplacementPolicy,
    /// Scene providers by content.
    providers: HashMap<ContentId, ProviderSceneAssets<S>>,
    /// Model assets by `content\0path`.
    models: HashMap<String, CachedModel>,
    /// Current world scene.
    current_world: Option<WorldEntry<S>>,
    /// Retained `.bsp` model scenes.
    brush_scenes: Vec<WorldEntry<S>>,
    /// Loaded console font.
    font: Option<LoadedMenuFont>,
    /// Loaded menu typography.
    typography: Option<BoxedMenuTypography>,
    /// Retired loaders awaiting `finish_image_refresh`.
    retired_textures: Vec<S::Textures>,
    /// Retired fonts awaiting `finish_image_refresh`.
    retired_fonts: Vec<LoadedMenuFont>,
    /// Retired typography awaiting `finish_image_refresh`.
    retired_typography: Vec<BoxedMenuTypography>,
    /// A committed refresh awaits `finish_image_refresh`.
    refresh_pending: bool,
    /// Closed flag.
    closed: bool,
}

impl<
        'a,
        P: ApplicationContentPreparer,
        S: AssetScene,
        C: MenuCharsetImages,
        F: MountedMenuFonts,
        M: TypographyMounts,
    > ApplicationAssets<'a, P, S, C, F, M>
{
    /// New asset cache over loaded content and host backends.
    pub fn new(
        content: LoadedApplicationContent<'a, P>,
        scene: S,
        charset_images: C,
        fonts: F,
        typography_mounts: M,
        options: ApplicationAssetOptions,
    ) -> Self {
        Self {
            content,
            scene,
            charset_images,
            fonts_host: fonts,
            typography_mounts,
            image_policy: options.image_policy,
            model_policy: options.model_policy.unwrap_or(DEFAULT_MODEL_REPLACEMENT_POLICY),
            providers: HashMap::new(),
            models: HashMap::new(),
            current_world: None,
            brush_scenes: Vec::new(),
            font: None,
            typography: None,
            retired_textures: Vec::new(),
            retired_fonts: Vec::new(),
            retired_typography: Vec::new(),
            refresh_pending: false,
            closed: false,
        }
    }

    /// Image policy for new loaders (donor `imagePolicy`).
    pub fn image_policy(&self) -> Option<&ImagePolicy> {
        self.image_policy.as_ref()
    }

    /// Model replacement policy (donor `modelPolicy`).
    pub fn model_policy(&self) -> &ModelReplacementPolicy {
        &self.model_policy
    }

    /// Replace the model policy across the assets and live providers
    /// (donor `setModelPolicy`; providers read the policy live).
    pub fn set_model_policy(&mut self, policy: ModelReplacementPolicy) {
        self.model_policy = policy.clone();
        for provider in self.providers.values_mut() {
            provider.model_policy = policy.clone();
        }
    }

    /// Current world scene (donor `world`).
    pub fn world(&self) -> Result<&S::World, AssetsError<S::Error>> {
        self.current_world
            .as_ref()
            .map(|entry| &entry.scene)
            .ok_or(AssetsError::WorldNotLoaded)
    }

    /// Leaf count of the current world scene (donor
    /// `world.map.leaves.length`).
    pub fn world_leaf_count(&self) -> Result<usize, AssetsError<S::Error>> {
        Ok(self.scene.world_leaf_count(self.world()?))
    }

    /// Scene provider for content, loading and caching it (donor
    /// `provider`).
    pub fn provider(&mut self, content: &ContentId) -> Result<&ProviderSceneAssets<S>, AssetsError<S::Error>> {
        if self.closed {
            return Err(AssetsError::Closed("Application assets are closed".to_string()));
        }
        if !self.providers.contains_key(content) {
            let family = self.content.catalog.product(content.as_str())?.expectation.family;
            let mounts = self.content.for_content(content)?;
            let palette = Self::palette_for(&mounts, family)?;
            let mut textures = self
                .scene
                .create_textures(&mounts, palette.as_ref(), self.image_policy.as_ref())
                .map_err(AssetsError::Scene)?;
            let shaders = match self.build_shaders(&mounts, family, &textures) {
                Ok(shaders) => shaders,
                Err(error) => {
                    self.scene.dispose_texture_images(&mut textures);
                    return Err(error);
                }
            };
            self.providers.insert(
                content.clone(),
                ProviderSceneAssets {
                    model_policy: self.model_policy.clone(),
                    family,
                    mounts,
                    palette,
                    textures,
                    shaders,
                },
            );
        }
        self.providers
            .get(content)
            .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))
    }

    /// Build a provider's shader registry with its scripts loaded
    /// (donor `provider` shader half).
    fn build_shaders(
        &mut self,
        mounts: &MountedContent,
        family: GameFamily,
        textures: &S::Textures,
    ) -> Result<S::Shaders, AssetsError<S::Error>> {
        let mut shaders = self
            .scene
            .create_shaders(textures, family)
            .map_err(AssetsError::Scene)?;
        let scripts = self.load_shader_scripts(mounts)?;
        if family == GameFamily::Q3 {
            let borrowed: Vec<(&str, &str)> = scripts
                .iter()
                .map(|(path, text)| (path.as_str(), text.as_str()))
                .collect();
            self.scene
                .initialize_source_materials(&mut shaders, &borrowed)
                .map_err(AssetsError::Scene)?;
        } else {
            for (path, text) in &scripts {
                self.scene
                    .add_shader_script(&mut shaders, text, path)
                    .map_err(AssetsError::Scene)?;
            }
        }
        Ok(shaders)
    }

    /// Load `.shader` scripts for a provider's mounts (donor
    /// `loadScripts`).
    fn load_shader_scripts(&self, mounts: &MountedContent) -> Result<Vec<(String, String)>, AssetsError<S::Error>> {
        let mut scripts = Vec::new();
        for path in Self::shader_paths(&self.content, mounts)? {
            if let Some(asset) = mounts.open(&path, |_| true)? {
                scripts.push((path, String::from_utf8_lossy(&asset.bytes).into_owned()));
            }
        }
        Ok(scripts)
    }

    /// Sorted shader script paths across archives and loose mounts
    /// (donor `shaderPaths`).
    fn shader_paths(
        content: &LoadedApplicationContent<'a, P>,
        mounts: &MountedContent,
    ) -> Result<Vec<String>, AssetsError<S::Error>> {
        let mut paths = BTreeSet::new();
        let archives: BTreeSet<&str> = mounts
            .plan
            .mounts
            .iter()
            .filter_map(|mount| match mount {
                qa_content::contract::ContentMount::Archive(archive) => Some(archive.archive_path.as_str()),
                qa_content::contract::ContentMount::Loose(_) => None,
            })
            .collect();
        for product in &content.catalog.products {
            for archive in &product.archives {
                if !archives.contains(archive.path.as_str()) {
                    continue;
                }
                for entry in &archive.entries {
                    if is_archive_shader_script(&entry.path) {
                        paths.insert(entry.path.clone());
                    }
                }
            }
        }
        for mount in &mounts.plan.mounts {
            let qa_content::contract::ContentMount::Loose(loose) = mount else {
                continue;
            };
            let directory = std::path::Path::new(&loose.root_path).join("scripts");
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(AssetsError::Io(error)),
            };
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".shader") {
                    paths.insert(format!("scripts/{name}"));
                }
            }
        }
        Ok(paths.into_iter().collect())
    }

    /// Source palette for a family (donor `paletteFor`).
    fn palette_for(mounts: &MountedContent, family: GameFamily) -> Result<Option<Palette>, AssetsError<S::Error>> {
        if family == GameFamily::Q3 {
            return Ok(None);
        }
        let path = if family == GameFamily::Q1 {
            "gfx/palette.lmp"
        } else {
            "pics/colormap.pcx"
        };
        let Some(asset) = mounts.open(path, |_| true)? else {
            return Err(AssetsError::Palette(format!(
                "Selected {} content has no {path}",
                family_name(family)
            )));
        };
        if family == GameFamily::Q1 {
            return Ok(Some(decode_palette(&asset.bytes, &asset.reference.requested_path)?));
        }
        let Some(colors) = decode_pcx(&asset.bytes, path)?.palette else {
            return Err(AssetsError::Palette(format!(
                "Selected Quake II colormap has no palette: {}",
                asset.reference.id
            )));
        };
        Ok(Some(Palette {
            colors,
            source: asset.reference.requested_path.clone(),
        }))
    }

    /// Q2 light modulation for content (donor `q2LightModulate`).
    fn q2_light_modulate(&self, content: &ContentId) -> Result<f32, AssetsError<S::Error>> {
        let family = self.content.catalog.product(content.as_str())?.expectation.family;
        Ok(if family == GameFamily::Q2 { 2.0 } else { 1.0 })
    }

    /// Presentation assets content id (donor
    /// `recipe.presentation.assets`).
    fn presentation_content(&self) -> ContentId {
        self.content.recipe.presentation.assets.clone()
    }

    /// Borrow decoded geometry for scene loading.
    fn geometry<'w, 'd: 'w>(world: &'w ApplicationWorld<'d>) -> WorldGeometry<'w> {
        match world {
            ApplicationWorld::Q1(map) => WorldGeometry::Q1(map),
            ApplicationWorld::Q2(map) => WorldGeometry::Q2(map),
            ApplicationWorld::Q3(world) => WorldGeometry::Q3(world),
        }
    }

    /// Sky name of the first worldspawn entity, defaulting to `unit1_`
    /// (donor `loadWorld` sky lookup).
    fn worldspawn_sky(&self) -> Result<String, AssetsError<S::Error>> {
        let entities = parse_entities(self.content.world.entities(), "world")?;
        for entity in &entities {
            if entity.get("classname").map(String::as_str) != Some("worldspawn") {
                continue;
            }
            if let Some(sky) = entity.get("sky") {
                return Ok(sky.clone());
            }
            break;
        }
        Ok("unit1_".to_string())
    }

    /// Load the current world scene once (donor `loadWorld`).
    pub fn load_world(&mut self) -> Result<&S::World, AssetsError<S::Error>> {
        if self.current_world.is_none() {
            let content = self.presentation_content();
            self.provider(&content)?;
            let geometry_content = provenance_content(&self.content.recipe.map.geometry.provenance).clone();
            let modulate = self.q2_light_modulate(&geometry_content)?;
            let sky = if self.content.world.kind() == "q2-bsp" {
                Some(self.worldspawn_sky()?)
            } else {
                None
            };
            let geometry = Self::geometry(&self.content.world);
            let provider = self
                .providers
                .get(&content)
                .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
            let scene = self
                .scene
                .load_world_scene(&geometry, &provider.shaders, sky.as_deref(), modulate)
                .map_err(AssetsError::Scene)?;
            self.current_world = Some(WorldEntry {
                scene,
                provider: content,
            });
        }
        self.world()
    }

    /// Load the console font once (donor `loadConsoleFont`).
    pub fn load_console_font(&mut self) -> Result<TextFontSelection, AssetsError<S::Error>> {
        if self.font.is_none() {
            let content = self.presentation_content();
            let rerelease = self.content.catalog.product(content.as_str())?.expectation.edition == "rerelease";
            let (family, mounts) = {
                let provider = self.provider(&content)?;
                (provider.family, Rc::clone(&provider.mounts))
            };
            let loaded = load_menu_font(
                &mounts,
                family,
                rerelease,
                &mut self.charset_images,
                &mut self.fonts_host,
            )?;
            self.font = Some(loaded);
        }
        self.font
            .as_ref()
            .map(|loaded| loaded.font.clone())
            .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))
    }

    /// Load menu typography once (donor `loadMenuTypography`).
    pub fn load_menu_typography(&mut self) -> Result<&BoxedMenuTypography, AssetsError<S::Error>> {
        if self.typography.is_none() {
            let font = self.load_console_font()?;
            let typography = load_menu_typography(
                &self.content.catalog,
                font.classic().clone(),
                &mut self.charset_images,
                &mut self.fonts_host,
                &mut self.typography_mounts,
            )?;
            self.typography = Some(typography);
        }
        self.typography
            .as_ref()
            .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))
    }

    /// Stage replacement textures and shaders for one content (donor
    /// `stagedProvider`).
    fn stage_provider(
        &mut self,
        staged: &mut HashMap<ContentId, StagedProvider<S>>,
        policy: &ImagePolicy,
        content: &ContentId,
    ) -> Result<(), AssetsError<S::Error>> {
        if staged.contains_key(content) {
            return Ok(());
        }
        self.provider(content)?;
        let live = self
            .providers
            .get(content)
            .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
        let mut textures = self
            .scene
            .create_textures(&live.mounts, live.palette.as_ref(), Some(policy))
            .map_err(AssetsError::Scene)?;
        self.scene
            .prepare_texture_replacement(&live.textures, &mut textures)
            .map_err(AssetsError::Scene)?;
        let mut shaders = self.scene.replace_shaders(&live.shaders, &textures);
        self.scene
            .prepare_shader_replacement(&live.shaders, &mut shaders)
            .map_err(AssetsError::Scene)?;
        staged.insert(content.clone(), StagedProvider { textures, shaders });
        Ok(())
    }

    /// Prepare an image refresh, staging every replacement (donor
    /// `prepareImageRefresh` staging half).
    pub fn prepare_image_refresh(
        &mut self,
        policy: ImagePolicy,
        model_policy: Option<ModelReplacementPolicy>,
    ) -> Result<PreparedApplicationImages<S>, AssetsError<S::Error>> {
        if self.closed {
            return Err(AssetsError::Closed("Application assets are closed".to_string()));
        }
        if self.refresh_pending {
            return Err(AssetsError::Refresh(
                "Previous image refresh has not finished rebinding".to_string(),
            ));
        }
        let model_policy = model_policy.unwrap_or_else(|| self.model_policy.clone());
        let mut staged = StagedRefresh::empty();
        if let Err(error) = self.stage_refresh_providers(&mut staged, &policy) {
            self.discard_staged(staged);
            return Err(error);
        }
        if let Err(error) = self.stage_refresh_models(&mut staged, &policy, &model_policy) {
            self.discard_staged(staged);
            return Err(error);
        }
        if let Err(error) = self.stage_refresh_worlds(&mut staged) {
            self.discard_staged(staged);
            return Err(error);
        }
        let remaps = match self.prepare_refresh_remaps(&staged) {
            Ok(remaps) => remaps,
            Err(error) => {
                self.discard_staged(staged);
                return Err(error);
            }
        };
        let (staged_fonts, staged_typography) = match self.stage_refresh_fonts() {
            Ok(pair) => pair,
            Err(error) => {
                staged.remaps = Some(remaps);
                self.discard_staged(staged);
                return Err(error);
            }
        };
        Ok(PreparedApplicationImages {
            font: staged_fonts.font.clone(),
            typography: staged_typography,
            policy,
            model_policy,
            staged_providers: staged.providers,
            staged_worlds: staged.worlds,
            staged_models: staged.models,
            remaps,
            staged_fonts,
        })
    }

    /// Stage replacements for every live provider.
    fn stage_refresh_providers(
        &mut self,
        staged: &mut StagedRefresh<S>,
        policy: &ImagePolicy,
    ) -> Result<(), AssetsError<S::Error>> {
        let contents: Vec<ContentId> = self.providers.keys().cloned().collect();
        for content in &contents {
            self.stage_provider(&mut staged.providers, policy, content)?;
        }
        Ok(())
    }

    /// Stage model replacement changes under a new policy.
    fn stage_refresh_models(
        &mut self,
        staged: &mut StagedRefresh<S>,
        policy: &ImagePolicy,
        model_policy: &ModelReplacementPolicy,
    ) -> Result<(), AssetsError<S::Error>> {
        if model_policy.q1_enhanced == self.model_policy.q1_enhanced
            && model_policy.q2_load == self.model_policy.q2_load
        {
            return Ok(());
        }
        let keys: Vec<(String, ContentId)> = self
            .models
            .iter()
            .filter(|(_, cached)| cached.variants.is_some())
            .map(|(key, cached)| (key.clone(), cached.provider.clone()))
            .collect();
        for (key, content) in &keys {
            self.stage_provider(&mut staged.providers, policy, content)?;
            let (family, mounts) = {
                let live = self
                    .providers
                    .get(content)
                    .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
                (live.family, Rc::clone(&live.mounts))
            };
            if replacement_enabled(family, model_policy) == replacement_enabled(family, &self.model_policy) {
                continue;
            }
            let entry = staged
                .providers
                .get_mut(content)
                .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
            let mut bridge = ApplicationModelProvider {
                family,
                mounts: MountAdapter { mounts },
                textures: TextureBridge {
                    scene: &mut self.scene,
                    textures: &mut entry.textures,
                },
            };
            let variants = self
                .models
                .get(key)
                .and_then(|cached| cached.variants.as_ref())
                .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
            let token = variants.prepare_replacement(&mut bridge, replacement_enabled(family, model_policy))?;
            staged.models.push((key.clone(), token));
        }
        Ok(())
    }

    /// Stage replacement images for the current and brush worlds.
    fn stage_refresh_worlds(&mut self, staged: &mut StagedRefresh<S>) -> Result<(), AssetsError<S::Error>> {
        let pairs = Self::staged_pairs(&self.providers, &staged.providers)?;
        if let Some(entry) = &self.current_world {
            let replacement = Self::stage_world_images(
                &mut self.scene,
                &entry.provider,
                &entry.scene,
                &staged.providers,
                &pairs,
            )?;
            staged.worlds.push(StagedWorld {
                slot: WorldSlot::Current,
                replacement,
            });
        }
        for (index, entry) in self.brush_scenes.iter().enumerate() {
            let replacement = Self::stage_world_images(
                &mut self.scene,
                &entry.provider,
                &entry.scene,
                &staged.providers,
                &pairs,
            )?;
            staged.worlds.push(StagedWorld {
                slot: WorldSlot::Brush(index),
                replacement,
            });
        }
        Ok(())
    }

    /// Stage replacement images for one world (donor world loop).
    fn stage_world_images(
        scene: &mut S,
        provider: &ContentId,
        world: &S::World,
        staged: &HashMap<ContentId, StagedProvider<S>>,
        pairs: &[(&S::Shaders, &S::Shaders)],
    ) -> Result<S::World, AssetsError<S::Error>> {
        let entry = staged
            .get(provider)
            .ok_or_else(|| AssetsError::Refresh("World image provider is absent".to_string()))?;
        scene
            .prepare_world_images(world, &entry.shaders, pairs)
            .map_err(AssetsError::Scene)
    }

    /// Collect `(live, staged)` registry pairs for remap resolution.
    fn staged_pairs<'p, 't>(
        providers: &'p HashMap<ContentId, ProviderSceneAssets<S>>,
        staged: &'t HashMap<ContentId, StagedProvider<S>>,
    ) -> Result<ShaderRegistryPairs<'p, 't, S>, AssetsError<S::Error>> {
        staged
            .iter()
            .map(|(content, entry)| {
                providers
                    .get(content)
                    .map(|live| (&live.shaders, &entry.shaders))
                    .ok_or_else(|| AssetsError::Refresh("World image provider is absent".to_string()))
            })
            .collect()
    }

    /// Prepare the remap refresh over staged registries.
    fn prepare_refresh_remaps(&mut self, staged: &StagedRefresh<S>) -> Result<S::Remaps, AssetsError<S::Error>> {
        let pairs = Self::staged_pairs(&self.providers, &staged.providers)?;
        self.scene.prepare_remap_refresh(&pairs).map_err(AssetsError::Scene)
    }

    /// Stage console fonts and typography under the refresh.
    fn stage_refresh_fonts(&mut self) -> Result<(LoadedMenuFont, BoxedMenuTypography), AssetsError<S::Error>> {
        let content = self.presentation_content();
        let rerelease = self.content.catalog.product(content.as_str())?.expectation.edition == "rerelease";
        let (family, mounts) = {
            let provider = self.provider(&content)?;
            (provider.family, Rc::clone(&provider.mounts))
        };
        let fonts = load_menu_font(
            &mounts,
            family,
            rerelease,
            &mut self.charset_images,
            &mut self.fonts_host,
        )?;
        let typography = load_menu_typography(
            &self.content.catalog,
            fonts.font.classic().clone(),
            &mut self.charset_images,
            &mut self.fonts_host,
            &mut self.typography_mounts,
        )?;
        Ok((fonts, typography))
    }

    /// Dispose partial or discarded staging (donor prepare catch and
    /// `discard`).
    fn discard_staged(&mut self, mut staged: StagedRefresh<S>) {
        if let Some(typography) = staged.typography.take() {
            let MenuTypography { closer, .. } = typography;
            closer();
        }
        if let Some(fonts) = staged.fonts.take() {
            fonts.close(&mut self.charset_images, &mut self.fonts_host);
        }
        for world in &mut staged.worlds {
            self.scene.close_world(&mut world.replacement);
        }
        for provider in staged.providers.values_mut() {
            self.scene
                .discard_shader_replacement(&mut provider.shaders, &mut provider.textures);
        }
    }

    /// Retire replaced loaders, fonts, and typography (donor
    /// `finishImageRefresh`).
    pub fn finish_image_refresh(&mut self) {
        for textures in &mut self.retired_textures {
            self.scene.dispose_texture_images(textures);
        }
        self.retired_textures.clear();
        for fonts in self.retired_fonts.drain(..) {
            fonts.close(&mut self.charset_images, &mut self.fonts_host);
        }
        for typography in self.retired_typography.drain(..) {
            let MenuTypography { closer, .. } = typography;
            closer();
        }
        self.refresh_pending = false;
    }

    /// Load a model asset, caching it by `content\0path` (donor `model`).
    pub fn model(&mut self, content: &ContentId, path: &str) -> Result<ModelAsset<'_, S>, AssetsError<S::Error>> {
        let key = format!("{content}\0{path}");
        if !self.models.contains_key(&key) {
            let cached = self.load_model(content, path)?;
            self.models.insert(key.clone(), cached);
        }
        let cached = self
            .models
            .get(&key)
            .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
        let provider = self
            .providers
            .get(&cached.provider)
            .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
        let brush_scene = match cached.brush {
            None => None,
            Some(BrushSceneRef::Current) => Some(self.world()?),
            Some(BrushSceneRef::Stored(index)) => Some(
                &self
                    .brush_scenes
                    .get(index)
                    .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?
                    .scene,
            ),
        };
        Ok(ModelAsset {
            resource: &cached.resource,
            model: &cached.model,
            provider,
            brush_scene,
        })
    }

    /// Load one model asset (donor `model` loader half).
    fn load_model(&mut self, content: &ContentId, path: &str) -> Result<CachedModel, AssetsError<S::Error>> {
        self.provider(content)?;
        if path.starts_with('*') {
            let index = inline_model_index(path).filter(|index| *index < submodel_count(&self.content.world));
            let Some(index) = index else {
                return Err(AssetsError::Model(format!("Missing inline model {path}")));
            };
            if self.current_world.is_none() {
                return Err(AssetsError::WorldNotLoaded);
            }
            return Ok(CachedModel {
                resource: self.content.recipe.map.geometry.clone(),
                model: AssetModel::Brush { model: index },
                variants: None,
                provider: content.clone(),
                brush: Some(BrushSceneRef::Current),
            });
        }
        let asset = {
            let provider = self
                .providers
                .get(content)
                .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
            provider.mounts.open(path, |_| true)?
        };
        let Some(asset) = asset else {
            return Err(AssetsError::Model(format!(
                "Model is absent from selected content: {content}/{path}"
            )));
        };
        if path.to_lowercase().ends_with(".bsp") {
            return self.load_brush_model(content, path, &asset);
        }
        let (family, mounts) = {
            let provider = self
                .providers
                .get(content)
                .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
            (provider.family, Rc::clone(&provider.mounts))
        };
        let live = self
            .providers
            .get_mut(content)
            .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
        let mut bridge = ApplicationModelProvider {
            family,
            mounts: MountAdapter { mounts },
            textures: TextureBridge {
                scene: &mut self.scene,
                textures: &mut live.textures,
            },
        };
        let loaded = load_application_model(&mut bridge, &asset, replacement_enabled(family, &self.model_policy))?;
        Ok(CachedModel {
            resource: loaded.resource,
            model: AssetModel::Alias(loaded.model),
            variants: loaded.variants,
            provider: content.clone(),
            brush: None,
        })
    }

    /// Load a `.bsp` model into a retained brush scene (donor `model`
    /// brush half).
    fn load_brush_model(
        &mut self,
        content: &ContentId,
        path: &str,
        asset: &OpenedResource,
    ) -> Result<CachedModel, AssetsError<S::Error>> {
        let geometry_content = provenance_content(&asset.reference.provenance).clone();
        let modulate = self.q2_light_modulate(&geometry_content)?;
        match classify_bsp(&asset.bytes, path)? {
            BspKind::Q1 => {
                let map = read_q1_bsp(&asset.bytes, path, Q1BspOptions::default())?;
                let geometry = WorldGeometry::Q1(&map);
                self.retain_brush_model(content, asset, &geometry, modulate)
            }
            BspKind::Q2 => {
                let map = to_q2_world_geometry(read_q2_bsp(&asset.bytes, path)?, None)?;
                let geometry = WorldGeometry::Q2(&map);
                self.retain_brush_model(content, asset, &geometry, modulate)
            }
            BspKind::Q3 => {
                let world = decode_q3_world(&asset.bytes, path)?;
                let geometry = WorldGeometry::Q3(&world);
                self.retain_brush_model(content, asset, &geometry, modulate)
            }
        }
    }

    /// Retain a decoded brush world as a model scene.
    fn retain_brush_model(
        &mut self,
        content: &ContentId,
        asset: &OpenedResource,
        geometry: &WorldGeometry<'_>,
        modulate: f32,
    ) -> Result<CachedModel, AssetsError<S::Error>> {
        let provider = self
            .providers
            .get(content)
            .ok_or_else(|| AssetsError::Closed("Application assets are closed".to_string()))?;
        let scene = self
            .scene
            .load_world_scene(geometry, &provider.shaders, None, modulate)
            .map_err(AssetsError::Scene)?;
        let index = self.brush_scenes.len();
        self.brush_scenes.push(WorldEntry {
            scene,
            provider: content.clone(),
        });
        Ok(CachedModel {
            resource: asset.reference.clone(),
            model: AssetModel::Brush { model: 0 },
            variants: None,
            provider: content.clone(),
            brush: Some(BrushSceneRef::Stored(index)),
        })
    }

    /// Close the assets and every retained scene, font, and provider
    /// (donor `close`).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.finish_image_refresh();
        for provider in self.providers.values_mut() {
            self.scene.close_textures(&mut provider.textures);
        }
        for provider in self.providers.values_mut() {
            self.scene.close_shaders(&mut provider.shaders);
        }
        if let Some(typography) = self.typography.take() {
            let MenuTypography { closer, .. } = typography;
            closer();
        }
        if let Some(fonts) = self.font.take() {
            fonts.close(&mut self.charset_images, &mut self.fonts_host);
        }
        if let Some(mut entry) = self.current_world.take() {
            self.scene.close_world(&mut entry.scene);
        }
        for entry in &mut self.brush_scenes {
            self.scene.close_world(&mut entry.scene);
        }
        self.brush_scenes.clear();
        self.providers.clear();
        self.models.clear();
    }
}

/// Staged replacement textures and shaders for one content.
struct StagedProvider<S: AssetScene> {
    /// Staged loader.
    textures: S::Textures,
    /// Staged registry.
    shaders: S::Shaders,
}

/// Retained world slot.
#[derive(Debug, Clone, Copy)]
enum WorldSlot {
    /// Current world scene.
    Current,
    /// One brush scene.
    Brush(usize),
}

/// Staged replacement world images for one slot.
struct StagedWorld<S: AssetScene> {
    /// Target slot.
    slot: WorldSlot,
    /// Replacement scene.
    replacement: S::World,
}

/// Partial image-refresh staging (donor prepare locals).
struct StagedRefresh<S: AssetScene> {
    /// Staged providers by content.
    providers: HashMap<ContentId, StagedProvider<S>>,
    /// Staged world replacements.
    worlds: Vec<StagedWorld<S>>,
    /// Staged model commits by model cache key.
    models: Vec<(String, ReplacementCommit)>,
    /// Prepared remaps.
    remaps: Option<S::Remaps>,
    /// Staged console fonts.
    fonts: Option<LoadedMenuFont>,
    /// Staged typography.
    typography: Option<BoxedMenuTypography>,
}

impl<S: AssetScene> StagedRefresh<S> {
    /// Empty staging.
    fn empty() -> Self {
        Self {
            providers: HashMap::new(),
            worlds: Vec::new(),
            models: Vec::new(),
            remaps: None,
            fonts: None,
            typography: None,
        }
    }
}

/// Prepared image refresh with commit and discard (donor
/// `PreparedApplicationImages`).
pub struct PreparedApplicationImages<S: AssetScene> {
    /// Staged font selection.
    pub font: TextFontSelection,
    /// Staged typography.
    pub typography: BoxedMenuTypography,
    /// Staged image policy.
    pub policy: ImagePolicy,
    /// Staged model policy.
    model_policy: ModelReplacementPolicy,
    /// Staged providers by content.
    staged_providers: HashMap<ContentId, StagedProvider<S>>,
    /// Staged world replacements.
    staged_worlds: Vec<StagedWorld<S>>,
    /// Staged model commits by model cache key.
    staged_models: Vec<(String, ReplacementCommit)>,
    /// Prepared remaps.
    remaps: S::Remaps,
    /// Staged console fonts.
    staged_fonts: LoadedMenuFont,
}

/// Commit and discard surface of a prepared refresh (donor
/// `PreparedApplicationImageBinding`).
pub trait PreparedApplicationImageBinding<P, S, C, F, M>
where
    P: ApplicationContentPreparer,
    S: AssetScene,
    C: MenuCharsetImages,
    F: MountedMenuFonts,
    M: TypographyMounts,
{
    /// Commit the refresh into the assets (donor `commit`).
    fn commit(self, assets: &mut ApplicationAssets<'_, P, S, C, F, M>) -> Result<(), AssetsError<S::Error>>;
    /// Discard the refresh (donor `discard`).
    fn discard(self, assets: &mut ApplicationAssets<'_, P, S, C, F, M>);
}

impl<S: AssetScene> PreparedApplicationImages<S> {
    /// Staged provider view for content (donor prepared `provider`).
    pub fn provider<'s, 'x, 'a, P, C, F, M>(
        &'s mut self,
        assets: &'x mut ApplicationAssets<'a, P, S, C, F, M>,
        content: &ContentId,
    ) -> Result<ProviderSceneView<'s, S>, AssetsError<S::Error>>
    where
        P: ApplicationContentPreparer,
        C: MenuCharsetImages,
        F: MountedMenuFonts,
        M: TypographyMounts,
    {
        assets.stage_provider(&mut self.staged_providers, &self.policy, content)?;
        let (mounts, palette, family, model_policy) = {
            let live = assets
                .providers
                .get(content)
                .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
            (
                Rc::clone(&live.mounts),
                live.palette.clone(),
                live.family,
                live.model_policy.clone(),
            )
        };
        let staged = self
            .staged_providers
            .get(content)
            .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
        Ok(ProviderSceneView {
            model_policy,
            family,
            mounts,
            palette,
            textures: &staged.textures,
            shaders: &staged.shaders,
        })
    }
}

impl<P, S, C, F, M> PreparedApplicationImageBinding<P, S, C, F, M> for PreparedApplicationImages<S>
where
    P: ApplicationContentPreparer,
    S: AssetScene,
    C: MenuCharsetImages,
    F: MountedMenuFonts,
    M: TypographyMounts,
{
    fn commit(self, assets: &mut ApplicationAssets<'_, P, S, C, F, M>) -> Result<(), AssetsError<S::Error>> {
        assets.scene.validate_remaps(&self.remaps).map_err(AssetsError::Scene)?;
        for staged in &self.staged_worlds {
            let live = Self::commit_world(assets, staged.slot)?;
            assets
                .scene
                .validate_world_images(&live.scene, &staged.replacement)
                .map_err(AssetsError::Scene)?;
        }
        for (content, staged) in &self.staged_providers {
            let live = assets
                .providers
                .get(content)
                .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
            assets
                .scene
                .validate_shader_replacement(&live.shaders, &staged.shaders)
                .map_err(AssetsError::Scene)?;
        }
        for (key, token) in self.staged_models {
            let cached = assets
                .models
                .get_mut(&key)
                .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
            let variants = cached
                .variants
                .as_mut()
                .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
            variants.commit(token);
        }
        for (content, mut staged) in self.staged_providers {
            let live = assets
                .providers
                .get_mut(&content)
                .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
            assets
                .scene
                .commit_shader_replacement(&mut live.shaders, staged.shaders);
            std::mem::swap(&mut live.textures, &mut staged.textures);
            assets.retired_textures.push(staged.textures);
        }
        for staged in self.staged_worlds {
            let replacement = staged.replacement;
            match staged.slot {
                WorldSlot::Current => {
                    let live = assets
                        .current_world
                        .as_mut()
                        .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
                    assets.scene.commit_world_images(&mut live.scene, replacement);
                }
                WorldSlot::Brush(index) => {
                    let live = assets
                        .brush_scenes
                        .get_mut(index)
                        .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))?;
                    assets.scene.commit_world_images(&mut live.scene, replacement);
                }
            }
        }
        let mut remaps = self.remaps;
        assets.scene.commit_remaps(&mut remaps);
        if let Some(previous) = assets.font.replace(self.staged_fonts) {
            assets.retired_fonts.push(previous);
        }
        if let Some(previous) = assets.typography.replace(self.typography) {
            assets.retired_typography.push(previous);
        }
        assets.image_policy = Some(self.policy);
        assets.model_policy = self.model_policy.clone();
        for provider in assets.providers.values_mut() {
            provider.model_policy = assets.model_policy.clone();
        }
        assets.refresh_pending = true;
        Ok(())
    }

    fn discard(self, assets: &mut ApplicationAssets<'_, P, S, C, F, M>) {
        assets.discard_staged(StagedRefresh {
            providers: self.staged_providers,
            worlds: self.staged_worlds,
            models: self.staged_models,
            remaps: None,
            fonts: Some(self.staged_fonts),
            typography: Some(self.typography),
        });
    }
}

impl<S: AssetScene> PreparedApplicationImages<S> {
    /// Live world behind a staged slot.
    fn commit_world<'w, P, C, F, M>(
        assets: &'w ApplicationAssets<'_, P, S, C, F, M>,
        slot: WorldSlot,
    ) -> Result<&'w WorldEntry<S>, AssetsError<S::Error>>
    where
        P: ApplicationContentPreparer,
        C: MenuCharsetImages,
        F: MountedMenuFonts,
        M: TypographyMounts,
    {
        match slot {
            WorldSlot::Current => assets.current_world.as_ref(),
            WorldSlot::Brush(index) => assets.brush_scenes.get(index),
        }
        .ok_or_else(|| AssetsError::Refresh("Image refresh resources were not prepared".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_model_numbers_match_donor() {
        assert_eq!(inline_model_index("*0"), Some(0));
        assert_eq!(inline_model_index("*12"), Some(12));
        assert_eq!(inline_model_index("*"), Some(0));
        assert_eq!(inline_model_index("*  7  "), Some(7));
        assert_eq!(inline_model_index("*0x10"), Some(16));
        assert_eq!(inline_model_index("*1e3"), Some(1000));
        assert_eq!(inline_model_index("*1.5"), None);
        assert_eq!(inline_model_index("*-1"), None);
        assert_eq!(inline_model_index("*abc"), None);
        assert_eq!(inline_model_index("*NaN"), None);
        assert_eq!(inline_model_index("*Infinity"), None);
        assert_eq!(inline_model_index("progs/player.mdl"), None);
        assert_eq!(inline_model_index("*9007199254740992"), None);
        assert_eq!(inline_model_index("*9007199254740991"), Some(9_007_199_254_740_991));
        assert_eq!(inline_model_index("*9007199254740990"), Some(9_007_199_254_740_990));
    }

    #[test]
    fn archive_shader_scripts_match_donor_pattern() {
        assert!(is_archive_shader_script("scripts/level.shader"));
        assert!(is_archive_shader_script("SCRIPTS/LEVEL.SHADER"));
        assert!(is_archive_shader_script("Scripts/Mixed.Shader"));
        assert!(!is_archive_shader_script("scripts/nested/dir.shader"));
        assert!(!is_archive_shader_script("scripts/level.txt"));
        assert!(!is_archive_shader_script("scripts/.shader"));
        assert!(!is_archive_shader_script("scripts/"));
        assert!(!is_archive_shader_script("level.shader"));
    }

    #[test]
    fn replacement_policy_matches_donor_selection() {
        let policy = DEFAULT_MODEL_REPLACEMENT_POLICY;
        assert!(replacement_enabled(GameFamily::Q1, &policy));
        assert!(replacement_enabled(GameFamily::Q2, &policy));
        assert!(!replacement_enabled(GameFamily::Q3, &policy));
        let off = ModelReplacementPolicy {
            q1_enhanced: false,
            q2_load: false,
            ..policy
        };
        assert!(!replacement_enabled(GameFamily::Q1, &off));
        assert!(!replacement_enabled(GameFamily::Q2, &off));
    }
}
