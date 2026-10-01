//! Mount-aware image reader with original-source fallback.
//!
//! Donor provenance: `src/app/bootstrap/image-reader.ts`
//! (`mountedImageReader`). Direct port: the reader remembers which mount won
//! each path and resolves originals beneath user replacements.

use qa_client::render::scene::textures::{SceneAsset, SceneAssetReader};
use qa_client::render::types::ImageSource;
use qa_client::render::RenderError;
use qa_content::catalog::InstalledCatalog;
use qa_content::contract::{ContentMount, ResourceProvenance};
use qa_content::mounts::{MountedContent, OpenedResource};
use std::cell::RefCell;
use std::collections::HashMap;

fn backend(error: impl std::fmt::Display) -> RenderError {
    RenderError::Backend(error.to_string())
}

fn scene_asset(asset: OpenedResource) -> SceneAsset {
    SceneAsset {
        bytes: asset.bytes,
        source: ImageSource::Resource {
            requested_path: asset.reference.requested_path,
        },
    }
}

fn winning_mount(asset: &OpenedResource) -> ContentMount {
    match asset.reference.provenance.clone() {
        ResourceProvenance::Archive { mount, .. } => ContentMount::Archive(mount),
        ResourceProvenance::Loose { mount, .. } => ContentMount::Loose(mount),
    }
}

/// Mount-aware scene asset reader.
pub struct MountedImageReader<'a> {
    catalog: &'a InstalledCatalog,
    mounts: &'a MountedContent,
    winners: RefCell<HashMap<String, Option<ContentMount>>>,
}

/// Build a reader over `catalog` winners and `mounts` bytes.
///
/// Original dimensions belong to the winning product, including installed mods.
#[must_use]
pub fn mounted_image_reader<'a>(catalog: &'a InstalledCatalog, mounts: &'a MountedContent) -> MountedImageReader<'a> {
    MountedImageReader {
        catalog,
        mounts,
        winners: RefCell::new(HashMap::new()),
    }
}

impl MountedImageReader<'_> {
    fn user_mount(&self, mount: &ContentMount) -> Result<bool, RenderError> {
        let product = self
            .catalog
            .product(mount.identity().content.as_str())
            .map_err(backend)?;
        let Some(user) = product.user_content.as_ref() else {
            return Ok(false);
        };
        match mount {
            ContentMount::Loose(loose) => Ok(loose.root_path == user.root),
            ContentMount::Archive(archive) => Ok(user.archives.iter().any(|entry| entry.path == archive.archive_path)),
        }
    }
}

impl SceneAssetReader for MountedImageReader<'_> {
    fn read(&self, path: &str) -> Result<Option<SceneAsset>, RenderError> {
        let asset = self.mounts.open(path, |_| true).map_err(backend)?;
        self.winners
            .borrow_mut()
            .insert(path.to_owned(), asset.as_ref().map(winning_mount));
        Ok(asset.map(scene_asset))
    }

    fn read_original(&self, path: &str) -> Result<Option<SceneAsset>, RenderError> {
        self.mounts.assert_open().map_err(backend)?;
        let winner = match self.winners.borrow().get(path) {
            Some(cached) => cached.clone(),
            None => {
                let winner = self
                    .mounts
                    .open(path, |_| true)
                    .map_err(backend)?
                    .as_ref()
                    .map(winning_mount);
                self.winners.borrow_mut().insert(path.to_owned(), winner.clone());
                winner
            }
        };
        let Some(winner) = winner else {
            return Ok(None);
        };
        if !self.user_mount(&winner)? {
            return Ok(None);
        }
        let content = winner.identity().content.clone();
        let asset = self
            .mounts
            .open(path, |mount| {
                mount.identity().content == content && !self.user_mount(mount).unwrap_or(true)
            })
            .map_err(backend)?;
        Ok(asset.map(scene_asset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation, UserContent};
    use qa_content::contract::{ContentId, LooseMount, MountId, MountIdentity, MountPlanId, ResolvedMountPlan};
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-image-reader-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn loose_mount(name: &str, content: &str, root: &std::path::Path) -> ContentMount {
        ContentMount::Loose(LooseMount {
            identity: MountIdentity {
                id: MountId(format!("mount:test:{name}")),
                content: ContentId(content.to_owned()),
                generation: 1,
            },
            root_path: root.to_string_lossy().into_owned(),
        })
    }

    fn catalog(content: &str, user_root: &std::path::Path) -> InstalledCatalog {
        InstalledCatalog::new(
            "/corpus".to_owned(),
            vec![CatalogProduct {
                id: ContentId(content.to_owned()),
                expectation: ProductExpectation {
                    id: format!("{content}-expectation"),
                    family: qa_content::contract::GameFamily::Q1,
                    edition: "classic".to_owned(),
                    campaign: "id1".to_owned(),
                    title: "Quake".to_owned(),
                    content_directory: "id1".to_owned(),
                    base_product: None,
                    required_content_archives: Vec::new(),
                    required_programs: Vec::new(),
                    map_witness: None,
                    unresolved_reason: None,
                },
                availability: ProductAvailability::Installed,
                archives: Vec::new(),
                loose_root: None,
                user_content: Some(UserContent {
                    root: user_root.to_string_lossy().into_owned(),
                    archives: Vec::new(),
                }),
                maps: Vec::new(),
                diagnostics: Vec::new(),
            }],
            Vec::new(),
            1,
            None,
        )
        .expect("catalog")
    }

    #[test]
    fn read_tracks_winners_and_originals_skip_user_mounts() {
        let user_root = scratch_dir("user");
        let corpus_root = scratch_dir("corpus");
        std::fs::write(user_root.join("pic.lmp"), b"user-bytes").expect("write");
        std::fs::write(corpus_root.join("pic.lmp"), b"corpus-bytes").expect("write");
        std::fs::write(corpus_root.join("stock.lmp"), b"stock-bytes").expect("write");
        let user = loose_mount("user", "q1:test:base:1", &user_root);
        let corpus = loose_mount("corpus", "q1:test:base:1", &corpus_root);
        let plan = ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:1".to_owned()),
            mounts: vec![user.clone(), corpus.clone()],
            default_order: vec![user.identity().id.clone(), corpus.identity().id.clone()],
            prefix_orders: Vec::new(),
        };
        let mounts = open_mount_plan(
            &plan,
            OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .expect("open");
        let catalog = catalog("q1:test:base:1", &user_root);
        let reader = mounted_image_reader(&catalog, &mounts);
        let asset = reader.read("pic.lmp").expect("read").expect("asset");
        assert_eq!(asset.bytes, b"user-bytes");
        let original = reader
            .read_original("pic.lmp")
            .expect("original")
            .expect("original asset");
        assert_eq!(original.bytes, b"corpus-bytes");
        let stock = reader.read("stock.lmp").expect("read").expect("asset");
        assert_eq!(stock.bytes, b"stock-bytes");
        assert!(reader.read_original("stock.lmp").expect("original").is_none());
        assert!(reader.read("missing.lmp").expect("read").is_none());
        assert!(reader.read_original("missing.lmp").expect("original").is_none());
        std::fs::remove_dir_all(&user_root).expect("cleanup");
        std::fs::remove_dir_all(&corpus_root).expect("cleanup");
    }
}
