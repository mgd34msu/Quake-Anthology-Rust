//! Selected-arsenal weapon HUD pictures.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/weapon-hud.ts`
//! (`ApplicationWeaponHudAssets`). The donor is async over `ApplicationAssets`
//! (`./assets.ts`, outside this wave); here providers, mounts, shaders,
//! textures, and the image registry arrive through the injected sync
//! [`WeaponHudBackend`] seam while this module keeps the donor's resource-id
//! scheme, WAD/QPIC decode chain, texture bookkeeping, and aspect fallbacks.
//! Icon resolution (`weaponHudIcons` over the content catalog) arrives through
//! [`HudIconResolver`]: the ported catalog icons (`qa_content`) expose no
//! field accessors outside their crate, so the catalog call shape (source,
//! product, item) is preserved as a seam rather than a direct call.
//! The donor's `pending` promise map (concurrent-load dedup) becomes the
//! `loaded` key set: sync loads cannot overlap, and completed keys are never
//! reloaded, matching the donor's memoized return.

use std::collections::{HashMap, HashSet};

use qa_content::contract::GameFamily;
use qa_content::images::indexed::decode_qpic;
use qa_content::images::palette::{indexed_render_image, IndexedRenderImage, Palette, PaletteTransparency};
use qa_content::images::wad::decode_wad;
use thiserror::Error;

/// Weapon HUD asset failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WeaponHudError {
    /// A Q1 mount open failed or the provider has no palette.
    #[error("Weapon HUD picture missing: {key}")]
    Missing {
        /// Donor load key.
        key: String,
    },
    /// A WAD lump is absent from its archive.
    #[error("Weapon HUD WAD lump missing: {key}")]
    WadLumpMissing {
        /// Donor load key.
        key: String,
    },
    /// A texture load returned null.
    #[error("Weapon HUD image missing: {key}")]
    ImageMissing {
        /// Donor load key.
        key: String,
    },
    /// QPIC/WAD/palette decoding failed.
    #[error("Weapon HUD decode failed for {key}: {message}")]
    Decode {
        /// Donor load key.
        key: String,
        /// Decoder message.
        message: String,
    },
    /// The injected backend failed.
    #[error("{0}")]
    Backend(String),
}

/// Absorbed donor `WeaponHudIcon` (field accessors are unavailable on the
/// ported catalog type, so the icon shape is mirrored locally).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HudIcon {
    /// WAD picture resource plus lump name.
    WadPicture {
        /// Content identity.
        content: String,
        /// Resource path.
        path: String,
        /// Lump name.
        lump: String,
    },
    /// Plain image resource.
    Image {
        /// Content identity.
        content: String,
        /// Resource path.
        path: String,
    },
    /// Named shader on a content.
    Shader {
        /// Content identity.
        content: String,
        /// Shader name.
        name: String,
    },
}

impl HudIcon {
    /// Donor load key: `content/name` for shaders,
    /// `content/path/lump` for WAD pictures, `content/path/` for images.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Shader { content, name } => format!("{content}/{name}"),
            Self::WadPicture { content, path, lump } => format!("{content}/{path}/{lump}"),
            Self::Image { content, path } => format!("{content}/{path}/"),
        }
    }

    /// Content the icon loads from.
    #[must_use]
    pub fn content(&self) -> &str {
        match self {
            Self::WadPicture { content, .. } | Self::Image { content, .. } | Self::Shader { content, .. } => content,
        }
    }
}

/// Absorbed donor `WeaponHudIcons` triple.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HudIconSet {
    /// Unselected weapon icon.
    pub weapon: Option<HudIcon>,
    /// Selected weapon icon.
    pub selected_weapon: Option<HudIcon>,
    /// Ammo icon.
    pub ammo: Option<HudIcon>,
}

/// Absorbed donor `WeaponHudStatus` fields read by `prepare` (source, item).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudStatusRef {
    /// Source provider.
    pub source_provider: String,
    /// Source content.
    pub source_content: String,
    /// Weapon item id.
    pub item: String,
}

/// Loaded weapon/ammo resource ids (donor `prepare` result).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreparedHudIcons {
    /// Weapon picture resource, if any.
    pub weapon: Option<String>,
    /// Ammo picture resource, if any.
    pub ammo: Option<String>,
}

/// One absorbed material stage: only loaded image bindings matter to
/// `aspect` (donor `stage.kind === "loaded"`, `binding.kind === "images"`,
/// single image or first frame).
#[derive(Debug, Clone, PartialEq)]
pub enum HudMaterialStage {
    /// Loaded image binding with its display dimensions.
    LoadedImages {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// Any other stage.
    Other,
}

/// Absorbed donor `PictureAsset` down to what `aspect` reads.
#[derive(Debug, Clone, PartialEq)]
pub enum WeaponHudPicture {
    /// Plain image picture.
    Image {
        /// Donor picture name (the load key).
        name: String,
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// Shader material picture.
    Material {
        /// Compiled stages in order.
        stages: Vec<HudMaterialStage>,
    },
}

/// Opened mount bytes with their resource reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponHudMount {
    /// File bytes.
    pub bytes: Vec<u8>,
    /// Resource reference for image registration.
    pub reference: String,
}

/// Loaded texture: donor keeps `texture.image` as the picture and
/// `texture.width / texture.height` as the logical aspect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponHudTexture {
    /// Picture image width.
    pub image_width: u32,
    /// Picture image height.
    pub image_height: u32,
    /// Texture logical width.
    pub width: u32,
    /// Texture logical height.
    pub height: u32,
}

/// Texture wrap mode (donor always passes `"clamp"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudTextureWrap {
    /// Clamp to edge.
    Clamp,
}

/// Texture load options (donor `{ family, mipmap: false, wrap: "clamp" }`;
/// family is implied by the content argument).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudTextureRequest {
    /// Generate mipmaps (always false).
    pub mipmap: bool,
    /// Wrap mode (always clamp).
    pub wrap: HudTextureWrap,
}

/// Donor texture request constants.
pub const HUD_TEXTURE_REQUEST: HudTextureRequest = HudTextureRequest {
    mipmap: false,
    wrap: HudTextureWrap::Clamp,
};

/// Absorbed `ApplicationAssets` surface: providers, mounts, palettes,
/// shaders, textures, and the image registry.
pub trait WeaponHudBackend {
    /// Product family of a content.
    fn family(&mut self, content: &str) -> GameFamily;
    /// Provider palette bytes (768), or `None` when absent.
    fn palette(&mut self, content: &str) -> Option<Vec<u8>>;
    /// Open a mount path, or `None` when absent.
    fn open_mount(&mut self, content: &str, path: &str) -> Result<Option<WeaponHudMount>, WeaponHudError>;
    /// Register a shader picture by name.
    fn register_shader_picture(&mut self, content: &str, name: &str) -> Result<WeaponHudPicture, WeaponHudError>;
    /// Load a texture, or `None` when absent.
    fn load_texture(
        &mut self,
        content: &str,
        path: &str,
        request: HudTextureRequest,
    ) -> Result<Option<WeaponHudTexture>, WeaponHudError>;
    /// Register a decoded indexed image under its load key.
    fn register_indexed_image(
        &mut self,
        key: &str,
        image: &IndexedRenderImage,
        resource: &str,
    ) -> Result<(), WeaponHudError>;
}

/// Absorbed `weaponHudIcons(status.source, catalog.product(...).expectation,
/// status.item)` resolution.
pub trait HudIconResolver {
    /// Resolve HUD icons for a weapon item, or `None` when unknown.
    fn resolve(&mut self, source_provider: &str, source_content: &str, item: &str) -> Option<HudIconSet>;
}

impl<F> HudIconResolver for F
where
    F: FnMut(&str, &str, &str) -> Option<HudIconSet>,
{
    fn resolve(&mut self, source_provider: &str, source_content: &str, item: &str) -> Option<HudIconSet> {
        self(source_provider, source_content, item)
    }
}

/// Pending image-refresh commit (donor `prepareImageRefresh` closure).
#[derive(Debug, Default)]
pub struct WeaponHudImageRefresh {
    pictures: HashMap<String, WeaponHudPicture>,
    aspects: HashMap<String, f64>,
}

impl WeaponHudImageRefresh {
    /// Commit refreshed pictures and aspects into the owner.
    pub fn commit(self, assets: &mut ApplicationWeaponHudAssets) {
        for (id, picture) in self.pictures {
            assets.pictures.insert(id, picture);
        }
        for (id, aspect) in self.aspects {
            assets.logical_aspects.insert(id, aspect);
        }
    }
}

/// Selected-arsenal weapon HUD pictures (donor `ApplicationWeaponHudAssets`).
#[derive(Debug, Default)]
pub struct ApplicationWeaponHudAssets {
    pictures: HashMap<String, WeaponHudPicture>,
    logical_aspects: HashMap<String, f64>,
    texture_icons: HashMap<String, HudIcon>,
    loaded: HashSet<String>,
}

impl ApplicationWeaponHudAssets {
    /// Empty asset table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Picture by resource id.
    #[must_use]
    pub fn picture(&self, id: &str) -> Option<&WeaponHudPicture> {
        self.pictures.get(id)
    }

    /// Display aspect for a resource id: logical override, image ratio,
    /// first loaded material stage ratio, else 1 (donor `aspect`).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn aspect(&self, id: Option<&str>) -> f64 {
        let Some(id) = id else {
            return 1.0;
        };
        if let Some(logical) = self.logical_aspects.get(id) {
            return *logical;
        }
        match self.pictures.get(id) {
            Some(WeaponHudPicture::Image { width, height, .. }) => f64::from(*width) / f64::from(*height),
            Some(WeaponHudPicture::Material { stages }) => {
                for stage in stages {
                    if let HudMaterialStage::LoadedImages { width, height } = stage {
                        return f64::from(*width) / f64::from(*height);
                    }
                }
                1.0
            }
            None => 1.0,
        }
    }

    /// Reload every texture-backed icon into a pending commit (donor
    /// `prepareImageRefresh`; Q1/palette pictures are not reloaded).
    pub fn prepare_image_refresh(
        &mut self,
        backend: &mut dyn WeaponHudBackend,
    ) -> Result<WeaponHudImageRefresh, WeaponHudError> {
        let mut commit = WeaponHudImageRefresh::default();
        let icons: Vec<(String, HudIcon)> = self
            .texture_icons
            .iter()
            .map(|(key, icon)| (key.clone(), icon.clone()))
            .collect();
        for (key, icon) in &icons {
            Self::load_icon_into(
                backend,
                &mut self.texture_icons,
                icon,
                key,
                &mut commit.pictures,
                &mut commit.aspects,
            )?;
        }
        Ok(commit)
    }

    /// Load weapon and ammo pictures for a status (donor `prepare`).
    pub fn prepare(
        &mut self,
        backend: &mut dyn WeaponHudBackend,
        resolver: &mut dyn HudIconResolver,
        status: Option<&HudStatusRef>,
    ) -> Result<PreparedHudIcons, WeaponHudError> {
        let Some(status) = status else {
            return Ok(PreparedHudIcons::default());
        };
        let Some(icons) = resolver.resolve(&status.source_provider, &status.source_content, &status.item) else {
            return Ok(PreparedHudIcons::default());
        };
        let weapon = icons
            .selected_weapon
            .as_ref()
            .or(icons.weapon.as_ref())
            .map(|icon| self.load(backend, icon))
            .transpose()?;
        let ammo = icons.ammo.as_ref().map(|icon| self.load(backend, icon)).transpose()?;
        Ok(PreparedHudIcons { weapon, ammo })
    }

    /// Load one icon, memoizing completed keys (donor `load`).
    pub fn load(&mut self, backend: &mut dyn WeaponHudBackend, icon: &HudIcon) -> Result<String, WeaponHudError> {
        let key = icon.key();
        let id = format!("resource:weapon-hud:{key}");
        if self.loaded.contains(&key) {
            return Ok(id);
        }
        Self::load_icon_into(
            backend,
            &mut self.texture_icons,
            icon,
            &key,
            &mut self.pictures,
            &mut self.logical_aspects,
        )?;
        self.loaded.insert(key);
        Ok(id)
    }

    /// Donor `loadIcon`: decode one icon into the given tables.
    fn load_icon_into(
        backend: &mut dyn WeaponHudBackend,
        texture_icons: &mut HashMap<String, HudIcon>,
        icon: &HudIcon,
        key: &str,
        pictures: &mut HashMap<String, WeaponHudPicture>,
        aspects: &mut HashMap<String, f64>,
    ) -> Result<String, WeaponHudError> {
        let id = format!("resource:weapon-hud:{key}");
        match icon {
            HudIcon::Shader { content, name } => {
                pictures.insert(id.clone(), backend.register_shader_picture(content, name)?);
            }
            HudIcon::WadPicture { content, path, .. } | HudIcon::Image { content, path }
                if backend.family(content) == GameFamily::Q1 =>
            {
                let lump = match icon {
                    HudIcon::WadPicture { lump, .. } => Some(lump.as_str()),
                    HudIcon::Image { .. } => None,
                    HudIcon::Shader { .. } => None,
                };
                let asset = backend.open_mount(content, path)?;
                let palette = backend.palette(content);
                let (Some(asset), Some(palette)) = (asset, palette) else {
                    return Err(WeaponHudError::Missing { key: key.to_string() });
                };
                let bytes;
                if let Some(lump) = lump {
                    let archive = decode_wad(&asset.bytes, key).map_err(|error| WeaponHudError::Decode {
                        key: key.to_string(),
                        message: error.to_string(),
                    })?;
                    let entry = archive.lumps.iter().find(|entry| entry.name == lump);
                    let Some(entry) = entry else {
                        return Err(WeaponHudError::WadLumpMissing { key: key.to_string() });
                    };
                    bytes = entry.bytes.clone();
                } else {
                    bytes = asset.bytes.clone();
                }
                let picture = decode_qpic(&bytes, key).map_err(|error| WeaponHudError::Decode {
                    key: key.to_string(),
                    message: error.to_string(),
                })?;
                let image = indexed_render_image(
                    vec![picture.clone()],
                    Palette {
                        colors: palette,
                        source: key.to_string(),
                    },
                    PaletteTransparency::Index(255),
                    None,
                    None,
                )
                .map_err(|error| WeaponHudError::Decode {
                    key: key.to_string(),
                    message: error.to_string(),
                })?;
                backend.register_indexed_image(key, &image, &asset.reference)?;
                pictures.insert(
                    id.clone(),
                    WeaponHudPicture::Image {
                        name: key.to_string(),
                        width: picture.width,
                        height: picture.height,
                    },
                );
            }
            HudIcon::WadPicture { content, path, .. } | HudIcon::Image { content, path } => {
                let texture = backend.load_texture(content, path, HUD_TEXTURE_REQUEST)?;
                let Some(texture) = texture else {
                    return Err(WeaponHudError::ImageMissing { key: key.to_string() });
                };
                pictures.insert(
                    id.clone(),
                    WeaponHudPicture::Image {
                        name: key.to_string(),
                        width: texture.image_width,
                        height: texture.image_height,
                    },
                );
                #[allow(clippy::cast_precision_loss)]
                aspects.insert(id.clone(), f64::from(texture.width) / f64::from(texture.height));
                texture_icons.insert(key.to_string(), icon.clone());
            }
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeBackend {
        families: HashMap<String, GameFamily>,
        palettes: HashMap<String, Vec<u8>>,
        mounts: HashMap<(String, String), WeaponHudMount>,
        textures: HashMap<(String, String), WeaponHudTexture>,
        registered: Vec<(String, u32, u32, String)>,
        texture_requests: Vec<HudTextureRequest>,
        shader_pictures: HashMap<String, WeaponHudPicture>,
    }

    impl FakeBackend {
        fn new() -> Self {
            Self {
                families: HashMap::new(),
                palettes: HashMap::new(),
                mounts: HashMap::new(),
                textures: HashMap::new(),
                registered: Vec::new(),
                texture_requests: Vec::new(),
                shader_pictures: HashMap::new(),
            }
        }
    }

    impl WeaponHudBackend for FakeBackend {
        fn family(&mut self, content: &str) -> GameFamily {
            self.families.get(content).copied().unwrap_or(GameFamily::Q2)
        }

        fn palette(&mut self, content: &str) -> Option<Vec<u8>> {
            self.palettes.get(content).cloned()
        }

        fn open_mount(&mut self, content: &str, path: &str) -> Result<Option<WeaponHudMount>, WeaponHudError> {
            Ok(self.mounts.get(&(content.to_string(), path.to_string())).cloned())
        }

        fn register_shader_picture(&mut self, content: &str, name: &str) -> Result<WeaponHudPicture, WeaponHudError> {
            Ok(self
                .shader_pictures
                .get(&format!("{content}/{name}"))
                .cloned()
                .unwrap_or(WeaponHudPicture::Material {
                    stages: vec![HudMaterialStage::Other],
                }))
        }

        fn load_texture(
            &mut self,
            content: &str,
            path: &str,
            request: HudTextureRequest,
        ) -> Result<Option<WeaponHudTexture>, WeaponHudError> {
            self.texture_requests.push(request);
            Ok(self.textures.get(&(content.to_string(), path.to_string())).cloned())
        }

        fn register_indexed_image(
            &mut self,
            key: &str,
            image: &IndexedRenderImage,
            resource: &str,
        ) -> Result<(), WeaponHudError> {
            assert_eq!(image.transparency, PaletteTransparency::Index(255));
            let level = &image.levels[0];
            self.registered
                .push((key.to_string(), level.width, level.height, resource.to_string()));
            Ok(())
        }
    }

    fn qpic_bytes(width: i32, height: i32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend(std::iter::repeat_n(0u8, (width * height) as usize));
        bytes
    }

    fn wad_bytes(lump_name: &str, lump: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WAD2");
        bytes.extend_from_slice(&1i32.to_le_bytes());
        let directory = 12 + lump.len();
        bytes.extend_from_slice(&(directory as i32).to_le_bytes());
        bytes.extend_from_slice(lump);
        bytes.extend_from_slice(&12i32.to_le_bytes());
        bytes.extend_from_slice(&(lump.len() as i32).to_le_bytes());
        bytes.extend_from_slice(&(lump.len() as i32).to_le_bytes());
        bytes.push(66u8);
        bytes.push(0u8);
        bytes.extend_from_slice(&[0u8; 2]);
        let mut name = [0u8; 16];
        name[..lump_name.len()].copy_from_slice(lump_name.as_bytes());
        bytes.extend_from_slice(&name);
        bytes
    }

    #[test]
    fn keys_match_donor_shapes() {
        assert_eq!(
            HudIcon::Shader {
                content: "c".to_string(),
                name: "s".to_string()
            }
            .key(),
            "c/s"
        );
        assert_eq!(
            HudIcon::Image {
                content: "c".to_string(),
                path: "p.lmp".to_string()
            }
            .key(),
            "c/p.lmp/"
        );
        assert_eq!(
            HudIcon::WadPicture {
                content: "c".to_string(),
                path: "g.wad".to_string(),
                lump: "sb".to_string()
            }
            .key(),
            "c/g.wad/sb"
        );
    }

    #[test]
    fn prepare_without_status_or_icons_returns_nulls() {
        let mut backend = FakeBackend::new();
        let mut assets = ApplicationWeaponHudAssets::new();
        let mut none = |_: &str, _: &str, _: &str| None;
        assert_eq!(
            assets.prepare(&mut backend, &mut none, None).unwrap(),
            PreparedHudIcons::default()
        );
        let status = HudStatusRef {
            source_provider: "q1:official".to_string(),
            source_content: "c".to_string(),
            item: "q1:weapon/shotgun".to_string(),
        };
        assert_eq!(
            assets.prepare(&mut backend, &mut none, Some(&status)).unwrap(),
            PreparedHudIcons::default()
        );
        assert!(backend.registered.is_empty());
    }

    #[test]
    fn prepare_prefers_selected_weapon_and_memoizes() {
        let mut backend = FakeBackend::new();
        backend.families.insert("c".to_string(), GameFamily::Q2);
        backend.textures.insert(
            ("c".to_string(), "w.tga".to_string()),
            WeaponHudTexture {
                image_width: 64,
                image_height: 32,
                width: 128,
                height: 32,
            },
        );
        backend.textures.insert(
            ("c".to_string(), "s.tga".to_string()),
            WeaponHudTexture {
                image_width: 16,
                image_height: 16,
                width: 16,
                height: 16,
            },
        );
        backend.textures.insert(
            ("c".to_string(), "a.tga".to_string()),
            WeaponHudTexture {
                image_width: 8,
                image_height: 8,
                width: 8,
                height: 8,
            },
        );
        let mut assets = ApplicationWeaponHudAssets::new();
        let status = HudStatusRef {
            source_provider: "q2:official".to_string(),
            source_content: "c".to_string(),
            item: "q2:weapon/shotgun".to_string(),
        };
        let mut resolver = |_: &str, _: &str, _: &str| {
            Some(HudIconSet {
                weapon: Some(HudIcon::Image {
                    content: "c".to_string(),
                    path: "w.tga".to_string(),
                }),
                selected_weapon: Some(HudIcon::Image {
                    content: "c".to_string(),
                    path: "s.tga".to_string(),
                }),
                ammo: Some(HudIcon::Image {
                    content: "c".to_string(),
                    path: "a.tga".to_string(),
                }),
            })
        };
        let prepared = assets.prepare(&mut backend, &mut resolver, Some(&status)).unwrap();
        assert_eq!(prepared.weapon.as_deref(), Some("resource:weapon-hud:c/s.tga/"));
        assert_eq!(prepared.ammo.as_deref(), Some("resource:weapon-hud:c/a.tga/"));
        assert_eq!(backend.texture_requests, vec![HUD_TEXTURE_REQUEST; 2]);
        assert_eq!(assets.aspect(prepared.weapon.as_deref()), 1.0);
        assert_eq!(assets.aspect(prepared.ammo.as_deref()), 1.0);
        let calls = backend.texture_requests.len();
        let again = assets.prepare(&mut backend, &mut resolver, Some(&status)).unwrap();
        assert_eq!(again, prepared);
        assert_eq!(backend.texture_requests.len(), calls);
    }

    #[test]
    fn q1_image_decodes_qpic_with_palette() {
        let mut backend = FakeBackend::new();
        backend.families.insert("c".to_string(), GameFamily::Q1);
        backend.palettes.insert("c".to_string(), vec![7u8; 768]);
        backend.mounts.insert(
            ("c".to_string(), "gfx/shotgun.lmp".to_string()),
            WeaponHudMount {
                bytes: qpic_bytes(64, 32),
                reference: "res:shotgun".to_string(),
            },
        );
        let mut assets = ApplicationWeaponHudAssets::new();
        let id = assets
            .load(
                &mut backend,
                &HudIcon::Image {
                    content: "c".to_string(),
                    path: "gfx/shotgun.lmp".to_string(),
                },
            )
            .unwrap();
        assert_eq!(id, "resource:weapon-hud:c/gfx/shotgun.lmp/");
        assert_eq!(
            backend.registered,
            vec![("c/gfx/shotgun.lmp/".to_string(), 64, 32, "res:shotgun".to_string())]
        );
        assert_eq!(assets.aspect(Some(&id)), 2.0);
    }

    #[test]
    fn q1_wad_picture_selects_lump() {
        let mut backend = FakeBackend::new();
        backend.families.insert("c".to_string(), GameFamily::Q1);
        backend.palettes.insert("c".to_string(), vec![0u8; 768]);
        backend.mounts.insert(
            ("c".to_string(), "gfx.wad".to_string()),
            WeaponHudMount {
                bytes: wad_bytes("sb_shells", &qpic_bytes(16, 8)),
                reference: "res:wad".to_string(),
            },
        );
        let mut assets = ApplicationWeaponHudAssets::new();
        let icon = HudIcon::WadPicture {
            content: "c".to_string(),
            path: "gfx.wad".to_string(),
            lump: "sb_shells".to_string(),
        };
        let id = assets.load(&mut backend, &icon).unwrap();
        assert_eq!(assets.aspect(Some(&id)), 2.0);
        let missing = HudIcon::WadPicture {
            content: "c".to_string(),
            path: "gfx.wad".to_string(),
            lump: "nope".to_string(),
        };
        assert_eq!(
            assets.load(&mut backend, &missing).unwrap_err(),
            WeaponHudError::WadLumpMissing {
                key: "c/gfx.wad/nope".to_string()
            }
        );
    }

    #[test]
    fn q1_missing_mount_or_palette_reports_key() {
        let mut backend = FakeBackend::new();
        backend.families.insert("c".to_string(), GameFamily::Q1);
        let mut assets = ApplicationWeaponHudAssets::new();
        let icon = HudIcon::Image {
            content: "c".to_string(),
            path: "gfx/x.lmp".to_string(),
        };
        assert_eq!(
            assets.load(&mut backend, &icon).unwrap_err(),
            WeaponHudError::Missing {
                key: "c/gfx/x.lmp/".to_string()
            }
        );
        backend.mounts.insert(
            ("c".to_string(), "gfx/x.lmp".to_string()),
            WeaponHudMount {
                bytes: qpic_bytes(8, 8),
                reference: "r".to_string(),
            },
        );
        assert_eq!(
            assets.load(&mut backend, &icon).unwrap_err(),
            WeaponHudError::Missing {
                key: "c/gfx/x.lmp/".to_string()
            }
        );
    }

    #[test]
    fn texture_missing_reports_key() {
        let mut backend = FakeBackend::new();
        backend.families.insert("c".to_string(), GameFamily::Q2);
        let mut assets = ApplicationWeaponHudAssets::new();
        let icon = HudIcon::Image {
            content: "c".to_string(),
            path: "pics/w.tga".to_string(),
        };
        assert_eq!(
            assets.load(&mut backend, &icon).unwrap_err(),
            WeaponHudError::ImageMissing {
                key: "c/pics/w.tga/".to_string()
            }
        );
    }

    #[test]
    fn shader_registers_picture() {
        let mut backend = FakeBackend::new();
        backend.shader_pictures.insert(
            "c/iconw_shotgun".to_string(),
            WeaponHudPicture::Material {
                stages: vec![
                    HudMaterialStage::Other,
                    HudMaterialStage::LoadedImages { width: 128, height: 64 },
                ],
            },
        );
        let mut assets = ApplicationWeaponHudAssets::new();
        let id = assets
            .load(
                &mut backend,
                &HudIcon::Shader {
                    content: "c".to_string(),
                    name: "iconw_shotgun".to_string(),
                },
            )
            .unwrap();
        assert_eq!(id, "resource:weapon-hud:c/iconw_shotgun");
        assert_eq!(assets.aspect(Some(&id)), 2.0);
    }

    #[test]
    fn aspect_falls_back_through_picture_kinds() {
        let mut backend = FakeBackend::new();
        backend.families.insert("c".to_string(), GameFamily::Q2);
        backend.textures.insert(
            ("c".to_string(), "w.tga".to_string()),
            WeaponHudTexture {
                image_width: 10,
                image_height: 20,
                width: 30,
                height: 10,
            },
        );
        let mut assets = ApplicationWeaponHudAssets::new();
        assert_eq!(assets.aspect(None), 1.0);
        assert_eq!(assets.aspect(Some("resource:weapon-hud:unknown")), 1.0);
        let id = assets
            .load(
                &mut backend,
                &HudIcon::Image {
                    content: "c".to_string(),
                    path: "w.tga".to_string(),
                },
            )
            .unwrap();
        assert_eq!(assets.aspect(Some(&id)), 3.0);
        backend.shader_pictures.insert(
            "c/plain".to_string(),
            WeaponHudPicture::Material {
                stages: vec![HudMaterialStage::Other],
            },
        );
        let material = assets
            .load(
                &mut backend,
                &HudIcon::Shader {
                    content: "c".to_string(),
                    name: "plain".to_string(),
                },
            )
            .unwrap();
        assert_eq!(assets.aspect(Some(&material)), 1.0);
    }

    #[test]
    fn image_refresh_reloads_textures_and_commits() {
        let mut backend = FakeBackend::new();
        backend.families.insert("c".to_string(), GameFamily::Q2);
        backend.textures.insert(
            ("c".to_string(), "w.tga".to_string()),
            WeaponHudTexture {
                image_width: 8,
                image_height: 8,
                width: 8,
                height: 8,
            },
        );
        let mut assets = ApplicationWeaponHudAssets::new();
        let id = assets
            .load(
                &mut backend,
                &HudIcon::Image {
                    content: "c".to_string(),
                    path: "w.tga".to_string(),
                },
            )
            .unwrap();
        assert_eq!(assets.aspect(Some(&id)), 1.0);
        backend.textures.insert(
            ("c".to_string(), "w.tga".to_string()),
            WeaponHudTexture {
                image_width: 24,
                image_height: 8,
                width: 48,
                height: 8,
            },
        );
        let commit = assets.prepare_image_refresh(&mut backend).unwrap();
        assert_eq!(assets.aspect(Some(&id)), 1.0);
        commit.commit(&mut assets);
        assert_eq!(assets.aspect(Some(&id)), 6.0);
        assert!(matches!(
            assets.picture(&id),
            Some(WeaponHudPicture::Image {
                width: 24,
                height: 8,
                ..
            })
        ));
    }

    #[test]
    fn image_refresh_failure_keeps_old_pictures() {
        let mut backend = FakeBackend::new();
        backend.families.insert("c".to_string(), GameFamily::Q2);
        backend.textures.insert(
            ("c".to_string(), "w.tga".to_string()),
            WeaponHudTexture {
                image_width: 8,
                image_height: 8,
                width: 8,
                height: 8,
            },
        );
        let mut assets = ApplicationWeaponHudAssets::new();
        let id = assets
            .load(
                &mut backend,
                &HudIcon::Image {
                    content: "c".to_string(),
                    path: "w.tga".to_string(),
                },
            )
            .unwrap();
        backend.textures.remove(&("c".to_string(), "w.tga".to_string()));
        assert!(assets.prepare_image_refresh(&mut backend).is_err());
        assert!(matches!(
            assets.picture(&id),
            Some(WeaponHudPicture::Image {
                width: 8,
                height: 8,
                ..
            })
        ));
    }
}
