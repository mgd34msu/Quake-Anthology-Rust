//! Windowed model skins: real skin textures for entity batches.
//!
//! Donor provenance: `src/render/scene/models/renderer.ts`
//! (`SceneModelRenderer.load`: Q3 shader names register through the shared
//! shader registry, legacy skins load through the shared texture loader as
//! `skin`/`sprite` usage). The windowed presentation used to resolve every
//! model skin to the shared white handle, so item pickups and weapons drew
//! as flat white boxes; this module resolves them through the world's own
//! texture loader instead, so skin registrations share the world image
//! registry (one ordinal space, drained with the first frame view's uploads)
//! and entity batches bind the decoded mount bytes.

use qa_client::render::scene::image_policy::ImageUsage;
use qa_client::render::scene::models::renderer::{ModelMaterialProvider, RenderFamily};
use qa_client::render::scene::models::types::ModelPalette;
use qa_client::render::scene::textures::{SceneTextureLoadOptions, SceneTextureLoader, TextureFamily};
use qa_client::render::types::{RenderImage, RendererImage, TextureSampling};
use qa_client::render::RenderError;

/// Texture provider resolving model skins through the world's loader.
///
/// Borrowed for the preload pass only ([`SceneModelRenderer::preload_with`](qa_client::render::scene::models::renderer::SceneModelRenderer::preload_with)):
/// Q3 shader names resolve to the representative loader image (the implicit
/// base image, matching the shader compiler's fallback; the windowed run
/// loads no authored `.shader` scripts, exactly like its world), legacy
/// skins load as `skin`/`sprite` usage, and indexed skins register into the
/// shared registry. Absent skins resolve to `None` so the renderer falls
/// back to the missing handle, never failing the run.
pub struct WindowedSkinProvider<'a> {
    family: RenderFamily,
    texture_family: TextureFamily,
    palette: Option<ModelPalette>,
    white: RendererImage,
    missing: RendererImage,
    textures: &'a mut SceneTextureLoader,
}

impl<'a> WindowedSkinProvider<'a> {
    /// Provider over the world's texture loader for one map family.
    pub fn new(textures: &'a mut SceneTextureLoader, family: RenderFamily, palette: Option<ModelPalette>) -> Self {
        let white = textures.white().image.clone();
        let missing = textures.missing().image.clone();
        let texture_family = match family {
            RenderFamily::Q1 => TextureFamily::Q1,
            RenderFamily::Q2 => TextureFamily::Q2,
            RenderFamily::Q3 => TextureFamily::Q3,
        };
        Self {
            family,
            texture_family,
            palette,
            white,
            missing,
            textures,
        }
    }
}

impl ModelMaterialProvider for WindowedSkinProvider<'_> {
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
        name: &str,
        image: RenderImage,
        sampling: TextureSampling,
    ) -> Result<RendererImage, RenderError> {
        Ok(self.textures.register(name, image, sampling, None, None)?.image)
    }

    fn load_external(&mut self, path: &str, sprite: bool) -> Result<Option<RendererImage>, RenderError> {
        let texture = self.textures.load(
            path,
            &SceneTextureLoadOptions {
                mipmap: !sprite,
                repeat: true,
                family: self.texture_family,
                usage: Some(if sprite { ImageUsage::Sprite } else { ImageUsage::Skin }),
            },
        )?;
        Ok(texture.map(|texture| texture.image))
    }

    fn shader_image(&mut self, name: &str) -> Result<Option<RendererImage>, RenderError> {
        let texture = self.textures.load(
            name,
            &SceneTextureLoadOptions {
                mipmap: true,
                repeat: true,
                family: TextureFamily::Q3,
                usage: Some(ImageUsage::Skin),
            },
        )?;
        Ok(texture.map(|texture| texture.image))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use qa_client::render::scene::resources::SceneImageRegistry;
    use qa_client::render::scene::textures::{SceneAsset, SceneAssetReader};
    use qa_client::render::types::{ImageLevel, ImageResourceOperation, ImageSource, ResourceOwner, TextureFilter};

    use super::*;

    /// Stub reader folding paths to lowercase, like product mounts.
    struct StubReader {
        files: HashMap<String, Vec<u8>>,
    }

    impl SceneAssetReader for StubReader {
        fn read(&self, path: &str) -> Result<Option<SceneAsset>, RenderError> {
            Ok(self.files.get(&path.to_lowercase()).map(|bytes| SceneAsset {
                bytes: bytes.clone(),
                source: ImageSource::Resource {
                    requested_path: path.to_string(),
                },
            }))
        }
    }

    /// Uncompressed 32-bit TGA with the given red pixels.
    fn tga(width: u32, height: u32, red: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&(width as u16).to_le_bytes());
        bytes[14..16].copy_from_slice(&(height as u16).to_le_bytes());
        bytes[16] = 32;
        for _ in 0..width * height {
            bytes.extend_from_slice(&[10, 20, red, 255]);
        }
        bytes
    }

    fn test_owner() -> ResourceOwner {
        let session = qa_core::identity::IdentityOwner::create("windowed-skins-test")
            .unwrap()
            .session()
            .clone();
        ResourceOwner::new(11, session, 0)
    }

    fn loader(files: &[(&str, Vec<u8>)]) -> SceneTextureLoader {
        SceneTextureLoader::new(
            SceneImageRegistry::new(test_owner()),
            Box::new(StubReader {
                files: files
                    .iter()
                    .map(|(path, bytes)| (path.to_lowercase(), bytes.clone()))
                    .collect(),
            }),
            None,
            None,
            224,
        )
        .unwrap()
    }

    fn sampling() -> TextureSampling {
        TextureSampling {
            repeat: true,
            filter: TextureFilter::Linear,
        }
    }

    #[test]
    fn q3_shader_names_resolve_through_extension_candidates() {
        let mut textures = loader(&[("models/ammo/rockammo.tga", tga(2, 2, 200))]);
        let white = textures.white().image.clone();
        let mut provider = WindowedSkinProvider::new(&mut textures, RenderFamily::Q3, None);
        // MD3 shader names carry an explicit (often uppercase) extension;
        // the loader probes it first, then the family order.
        let image = provider
            .shader_image("models/ammo/rockammo.TGA")
            .unwrap()
            .expect("skin resolves");
        assert_ne!(image, white);
        assert_ne!(image, provider.missing_image());
        assert!(matches!(image.source, ImageSource::Resource { .. }));
        let bare = provider
            .shader_image("models/ammo/rockammo")
            .unwrap()
            .expect("bare shader name resolves");
        assert_ne!(bare, white);
        assert!(matches!(bare.source, ImageSource::Resource { .. }));
    }

    #[test]
    fn absent_skins_fall_back_to_missing_without_failing() {
        let mut textures = loader(&[]);
        let mut provider = WindowedSkinProvider::new(&mut textures, RenderFamily::Q3, None);
        assert_eq!(provider.shader_image("models/ammo/none.TGA").unwrap(), None);
        assert_eq!(provider.load_external("models/ammo/none.tga", false).unwrap(), None);
        assert_eq!(provider.load_external("sprites/none.spr", true).unwrap(), None);
        // The "*default" selection resolves to the missing handle itself.
        let default = provider.shader_image("*default").unwrap().expect("default");
        assert_eq!(default, provider.missing_image());
    }

    #[test]
    fn indexed_skins_register_into_the_shared_registry() {
        let mut textures = loader(&[]);
        let mut provider = WindowedSkinProvider::new(
            &mut textures,
            RenderFamily::Q1,
            Some(ModelPalette {
                colors: vec![0; 768],
                source: "gfx/palette.lmp".to_string(),
            }),
        );
        let handle = provider
            .register_indexed(
                "skin:0:0",
                RenderImage::Rgba8 {
                    levels: vec![ImageLevel {
                        width: 2,
                        height: 2,
                        pixels: vec![9; 2 * 2 * 4],
                    }],
                    border_color: qa_core::math::vec4(0.0, 0.0, 0.0, 1.0),
                },
                sampling(),
            )
            .unwrap();
        assert_ne!(handle, provider.white_image());
        assert!(matches!(handle.source, ImageSource::Generated { .. }));
        let operations = provider.textures.images_mut().drain_operations();
        assert!(
            operations.iter().any(|operation| matches!(
                operation,
                ImageResourceOperation::CreateImage { image, .. } if image == &handle
            )),
            "indexed skin queues its upload with the world images"
        );
    }
}
