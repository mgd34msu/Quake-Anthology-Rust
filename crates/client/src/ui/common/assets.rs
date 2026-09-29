//! Native menu art loading: manifest validation, renderer upload, skin wiring.
//!
//! Donor provenance: `src/ui/common/assets.ts` (`loadNativeUiArt`). The donor
//! is async; this port is synchronous — the application supplies its
//! installed-asset reader and the renderer owns all uploads.

use std::collections::HashMap;

use crate::error::ClientError;
use crate::render::scene::resources::{rgba_image, SceneImageRegistry};
use crate::render::types::{ImageLevel, RendererImage, TextureFilter, TextureSampling};
use crate::text::draw2d::ImagePicture;
use crate::ui::common::art_manifest::{main_menu_background, menu_background, menu_focus, menu_panel, MenuArtFrame};
use crate::ui::common::skin::{default_ui_skin, UiBorder, UiImageSlice, UiSkin};
use crate::ui::types::ResourceId;

/// Loaded native menu art: skin, white pixel, and per-resource pictures.
pub struct NativeUiArt {
    /// Menu skin wired to the uploaded panel, focus, and background art.
    pub skin: UiSkin,
    /// White pixel picture for fills.
    pub white: ImagePicture,
    pictures: HashMap<ResourceId, ImagePicture>,
    owned: Vec<RendererImage>,
}

impl NativeUiArt {
    /// Look up one uploaded picture by resource.
    pub fn picture(&self, resource: &ResourceId) -> Result<ImagePicture, ClientError> {
        self.pictures.get(resource).copied().ok_or_else(|| {
            ClientError::BadUi(format!("Unregistered native UI image: {resource}"))
        })
    }

    /// Release every owned image back to the registry.
    pub fn release(self, images: &mut SceneImageRegistry) {
        for image in &self.owned {
            let _ = images.release(image);
        }
    }
}

impl std::fmt::Debug for NativeUiArt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeUiArt")
            .field("skin", &self.skin)
            .field("white", &self.white)
            .finish_non_exhaustive()
    }
}

/// Load, validate, and upload every menu image, then wire the native skin.
///
/// `read` decodes one installed asset path. Decoded dimensions must match the
/// manifest; owned images are released before any error returns.
pub fn load_native_ui_art(
    font: &ResourceId,
    images: &mut SceneImageRegistry,
    read: &mut dyn FnMut(&str) -> Result<ImageLevel, ClientError>,
) -> Result<NativeUiArt, ClientError> {
    let resources = [
        ResourceId::new("resource:engine-menu:background")?,
        ResourceId::new("resource:engine-menu:main-background")?,
        ResourceId::new("resource:engine-menu:panel")?,
        ResourceId::new("resource:engine-menu:focus")?,
    ];
    let files = [menu_background, main_menu_background, menu_panel.file(), menu_focus.file()];
    let mut pictures = HashMap::with_capacity(files.len());
    let mut owned: Vec<RendererImage> = Vec::with_capacity(files.len() + 1);
    let sampling = TextureSampling { repeat: false, filter: TextureFilter::Linear };
    for (resource, file) in resources.iter().zip(files.iter()) {
        let level = match read(file.file) {
            Ok(level) => level,
            Err(error) => {
                release_all(images, &owned);
                return Err(error);
            }
        };
        if level.width != file.width || level.height != file.height {
            release_all(images, &owned);
            return Err(ClientError::BadUi(format!(
                "Menu image dimensions differ from the manifest: {}",
                file.file
            )));
        }
        match images.register(file.file, rgba_image(level), sampling) {
            Ok(image) => {
                pictures.insert(
                    resource.clone(),
                    ImagePicture { image: image.ordinal, width: image.width, height: image.height },
                );
                owned.push(image);
            }
            Err(error) => {
                release_all(images, &owned);
                return Err(ClientError::BadUi(format!("Menu image upload failed: {error}")));
            }
        }
    }
    let white_level = ImageLevel { width: 1, height: 1, pixels: vec![255, 255, 255, 255] };
    let white_sampling = TextureSampling { repeat: false, filter: TextureFilter::Nearest };
    let white_image = match images.register("menu-white", rgba_image(white_level), white_sampling) {
        Ok(image) => image,
        Err(error) => {
            release_all(images, &owned);
            return Err(ClientError::BadUi(format!("Menu image upload failed: {error}")));
        }
    };
    let white = ImagePicture { image: white_image.ordinal, width: white_image.width, height: white_image.height };
    owned.push(white_image);
    let mut skin = default_ui_skin(font);
    skin.background = Some(resources[0].clone());
    skin.panel = Some(slice(&resources[2], &menu_panel));
    skin.focus = Some(slice(&resources[3], &menu_focus));
    Ok(NativeUiArt { skin, white, pictures, owned })
}

/// Build one skin slice from a manifest frame.
fn slice(resource: &ResourceId, frame: &MenuArtFrame) -> UiImageSlice {
    UiImageSlice {
        resource: resource.clone(),
        width: frame.region.width as f32,
        height: frame.region.height as f32,
        uv: frame.region.uv,
        border: UiBorder {
            l: frame.border.l as f32,
            t: frame.border.t as f32,
            r: frame.border.r as f32,
            b: frame.border.b as f32,
        },
        border_scale: Some(frame.border_scale),
    }
}

/// Release partially owned images on the error path.
fn release_all(images: &mut SceneImageRegistry, owned: &[RendererImage]) {
    for image in owned {
        let _ = images.release(image);
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::*;
    use crate::render::types::ResourceOwner;

    fn registry() -> SceneImageRegistry {
        let authority = IdentityOwner::create("native-ui-art-test").unwrap();
        SceneImageRegistry::new(ResourceOwner::new(1, authority.session().clone(), 0))
    }

    fn solid(width: u32, height: u32) -> ImageLevel {
        ImageLevel { width, height, pixels: vec![9; (width * height * 4) as usize] }
    }

    fn reader() -> impl FnMut(&str) -> Result<ImageLevel, ClientError> {
        |path| match path {
            "assets/ui/menu-background.png" => Ok(solid(1536, 1024)),
            "assets/ui/main-menu-background.png" => Ok(solid(1672, 941)),
            "assets/ui/menu-panel.png" => Ok(solid(1254, 1254)),
            "assets/ui/menu-focus.png" => Ok(solid(2172, 724)),
            other => Err(ClientError::BadUi(format!("missing asset: {other}"))),
        }
    }

    #[test]
    fn loads_skin_and_pictures() {
        let mut images = registry();
        let font = ResourceId::new("resource:test:font").unwrap();
        let art = load_native_ui_art(&font, &mut images, &mut reader()).unwrap();
        assert_eq!(art.white.width, 1);
        assert_eq!(art.skin.font, font);
        assert_eq!(
            art.skin.background.as_ref().unwrap().as_str(),
            "resource:engine-menu:background"
        );
        let panel = art.skin.panel.as_ref().unwrap();
        assert_eq!(panel.width, 1254.0);
        assert_eq!(panel.border_scale, Some(0.2));
        assert_eq!(panel.resource.as_str(), "resource:engine-menu:panel");
        let focus = art.skin.focus.as_ref().unwrap();
        assert_eq!(focus.resource.as_str(), "resource:engine-menu:focus");
        assert_eq!(focus.border_scale, Some(0.0625));
        let background = ResourceId::new("resource:engine-menu:background").unwrap();
        let picture = art.picture(&background).unwrap();
        assert_eq!((picture.width, picture.height), (1536, 1024));
        assert!(art.picture(&ResourceId::new("resource:test:missing").unwrap()).is_err());
        art.release(&mut images);
        assert_eq!(images.drain_operations().len(), 10);
    }

    #[test]
    fn dimension_mismatch_releases_owned() {
        let mut images = registry();
        let font = ResourceId::new("resource:test:font").unwrap();
        let mut bad = |path: &str| match path {
            "assets/ui/menu-background.png" => Ok(solid(1536, 1024)),
            "assets/ui/main-menu-background.png" => Ok(solid(8, 8)),
            _ => Err(ClientError::BadUi("unexpected read".to_string())),
        };
        let error = load_native_ui_art(&font, &mut images, &mut bad).unwrap_err();
        assert!(error.to_string().contains("dimensions differ"));
        let operations = images.drain_operations();
        assert_eq!(operations.len(), 2);
        assert!(operations.iter().any(|operation| matches!(
            operation,
            crate::render::types::ImageResourceOperation::ReleaseImage { .. }
        )));
    }

    #[test]
    fn reader_failure_releases_owned() {
        let mut images = registry();
        let font = ResourceId::new("resource:test:font").unwrap();
        let mut failing = |path: &str| {
            if path == "assets/ui/menu-background.png" {
                Ok(solid(1536, 1024))
            } else {
                Err(ClientError::BadUi("disk gone".to_string()))
            }
        };
        assert!(load_native_ui_art(&font, &mut images, &mut failing).is_err());
        assert_eq!(images.drain_operations().len(), 2);
    }
}
