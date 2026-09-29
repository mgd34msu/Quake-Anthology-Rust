//! Session-owned image registry.
//!
//! Donor provenance: `src/render/scene/resources.ts` (`SceneImageRegistry`,
//! `rgbaImage`). Uploads and releases enter the same queue as their draws.
//! This port keeps the single-scope allocation/commit/require core; the
//! donor's forked child scopes and media-clock animations have no contract
//! in the ordered backend and are not carried over.

use std::collections::HashMap;

use qa_core::math::Vec4;

use crate::render::error::RenderError;
use crate::render::types::{
    ImageLevel, ImageResourceOperation, ImageSource, LevelContent, RenderImage, RendererImage, ResourceOwner,
    TextureFilter, TextureSampling,
};

/// Session-owned images with ordered lifecycle operations.
pub struct SceneImageRegistry {
    owner: ResourceOwner,
    next_ordinal: u32,
    pending: HashMap<u32, RendererImage>,
    resident: HashMap<u32, RendererImage>,
    operations: Vec<ImageResourceOperation>,
    closed: bool,
}

impl SceneImageRegistry {
    /// Create a registry for one renderer lifetime.
    #[must_use]
    pub fn new(owner: ResourceOwner) -> Self {
        Self {
            owner,
            next_ordinal: 0,
            pending: HashMap::new(),
            resident: HashMap::new(),
            operations: Vec::new(),
            closed: false,
        }
    }

    /// Owning renderer lifetime.
    #[must_use]
    pub fn owner(&self) -> &ResourceOwner {
        &self.owner
    }

    /// Take all queued resource operations, leaving the queue empty.
    #[must_use]
    pub fn drain_operations(&mut self) -> Vec<ImageResourceOperation> {
        std::mem::take(&mut self.operations)
    }

    /// Allocate identity before an execution-owned upload; no GPU resource exists yet.
    pub fn allocate(&mut self, width: u32, height: u32, source: ImageSource) -> Result<RendererImage, RenderError> {
        if self.closed {
            return Err(RenderError::OutOfOrder("scene image registry is closed".to_string()));
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal = ordinal
            .checked_add(1)
            .ok_or_else(|| RenderError::OutOfOrder("renderer image ordinal space is exhausted".to_string()))?;
        let image = RendererImage {
            owner: self.owner.clone(),
            ordinal,
            source,
            width,
            height,
        };
        self.pending.insert(ordinal, image.clone());
        Ok(image)
    }

    /// Track ordered resource ownership; execution-owned uploads call after backend success.
    pub fn commit(&mut self, operation: &ImageResourceOperation) -> Result<(), RenderError> {
        match operation {
            ImageResourceOperation::CreateImage { image, .. } => {
                if self.closed {
                    return Err(RenderError::OutOfOrder("scene image registry is closed".to_string()));
                }
                let fresh = self.pending.get(&image.ordinal) == Some(image)
                    && image.owner == self.owner
                    && !self.resident.contains_key(&image.ordinal);
                if !fresh {
                    return Err(RenderError::ImageConflict(image.ordinal));
                }
                self.pending.remove(&image.ordinal);
                self.resident.insert(image.ordinal, image.clone());
                Ok(())
            }
            ImageResourceOperation::UpdateImage { image, .. } => self.require(image),
            ImageResourceOperation::ReleaseImage { image } => {
                self.require(image)?;
                self.resident.remove(&image.ordinal);
                self.pending.remove(&image.ordinal);
                Ok(())
            }
            ImageResourceOperation::TextureMode { .. } => Ok(()),
        }
    }

    /// Allocate, commit, and queue creation of a generated image.
    pub fn register(
        &mut self,
        name: &str,
        content: RenderImage,
        sampling: TextureSampling,
    ) -> Result<RendererImage, RenderError> {
        self.register_with_source(
            name,
            content,
            sampling,
            ImageSource::Generated { name: name.to_string() },
        )
    }

    /// Allocate, commit, and queue creation of an image with an explicit
    /// source (loaded assets keep their content source for journals).
    pub fn register_with_source(
        &mut self,
        _name: &str,
        content: RenderImage,
        sampling: TextureSampling,
        source: ImageSource,
    ) -> Result<RendererImage, RenderError> {
        let (width, height) = base_dimensions(&content)?;
        let image = self.allocate(width, height, source)?;
        let operation = ImageResourceOperation::CreateImage {
            image: image.clone(),
            content,
            sampling,
        };
        self.commit(&operation)?;
        self.operations.push(operation);
        Ok(image)
    }

    /// Queue replacement pixels for one mipmap level.
    pub fn update(&mut self, image: &RendererImage, level: u32, content: LevelContent) -> Result<(), RenderError> {
        if self.closed {
            return Err(RenderError::OutOfOrder("scene image registry is closed".to_string()));
        }
        self.require(image)?;
        self.operations.push(ImageResourceOperation::UpdateImage {
            image: image.clone(),
            level,
            content,
        });
        Ok(())
    }

    /// Drop residency and queue the release.
    pub fn release(&mut self, image: &RendererImage) -> Result<(), RenderError> {
        if self.closed {
            return Err(RenderError::OutOfOrder("scene image registry is closed".to_string()));
        }
        self.require(image)?;
        self.resident.remove(&image.ordinal);
        self.pending.remove(&image.ordinal);
        self.operations
            .push(ImageResourceOperation::ReleaseImage { image: image.clone() });
        Ok(())
    }

    /// Queue a global texture filter mode change.
    pub fn texture_mode(&mut self, filter: TextureFilter) {
        self.operations.push(ImageResourceOperation::TextureMode { filter });
    }

    /// Reject images from another lifetime or already released.
    pub fn require(&self, image: &RendererImage) -> Result<(), RenderError> {
        self.owner.require(&image.owner, "scene image")?;
        if self.resident.get(&image.ordinal) == Some(image) {
            Ok(())
        } else {
            Err(RenderError::UnknownImage(image.ordinal))
        }
    }

    /// Whether the image is resident in this registry.
    #[must_use]
    pub fn is_resident(&self, image: &RendererImage) -> bool {
        image.owner == self.owner && self.resident.get(&image.ordinal) == Some(image)
    }

    /// Look up a resident image by ordinal.
    #[must_use]
    pub fn get(&self, ordinal: u32) -> Option<&RendererImage> {
        self.resident.get(&ordinal)
    }

    /// Queue releases for every resident image and refuse further allocation.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        for image in self.resident.values() {
            self.operations
                .push(ImageResourceOperation::ReleaseImage { image: image.clone() });
        }
        self.resident.clear();
        self.pending.clear();
    }

    /// Whether the registry is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

/// Build true-color content from one mipmap level.
#[must_use]
pub fn rgba_image(level: ImageLevel) -> RenderImage {
    RenderImage::Rgba8 {
        levels: vec![level],
        border_color: Vec4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        },
    }
}

/// Base-level dimensions, validating pixel coverage.
fn base_dimensions(content: &RenderImage) -> Result<(u32, u32), RenderError> {
    match content {
        RenderImage::Indexed8 { levels, .. } => {
            let level = levels.first().ok_or_else(|| RenderError::BadDimensions {
                width: 0,
                height: 0,
                detail: "image has no mipmap levels".to_string(),
            })?;
            validate_level(level.width, level.height, level.pixels.len(), 1)?;
            Ok((level.width, level.height))
        }
        RenderImage::Rgba8 { levels, .. } => {
            let level = levels.first().ok_or_else(|| RenderError::BadDimensions {
                width: 0,
                height: 0,
                detail: "image has no mipmap levels".to_string(),
            })?;
            validate_level(level.width, level.height, level.pixels.len(), 4)?;
            Ok((level.width, level.height))
        }
        RenderImage::Depth32f { levels } => {
            let level = levels.first().ok_or_else(|| RenderError::BadDimensions {
                width: 0,
                height: 0,
                detail: "image has no mipmap levels".to_string(),
            })?;
            validate_level(level.width, level.height, level.pixels.len(), 1)?;
            Ok((level.width, level.height))
        }
    }
}

/// Check positive dimensions and exact pixel coverage.
fn validate_level(width: u32, height: u32, samples: usize, channels: usize) -> Result<(), RenderError> {
    if width == 0 || height == 0 {
        return Err(RenderError::BadDimensions {
            width,
            height,
            detail: "image dimensions must be positive".to_string(),
        });
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(channels));
    if expected != Some(samples) {
        return Err(RenderError::BadDimensions {
            width,
            height,
            detail: format!("pixel length {samples} does not cover {width}x{height}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::*;

    fn test_owner(identity: u64) -> ResourceOwner {
        let authority = IdentityOwner::create("scene-resources-test").unwrap();
        ResourceOwner::new(identity, authority.session().clone(), 0)
    }

    fn sampling() -> TextureSampling {
        TextureSampling {
            repeat: false,
            filter: TextureFilter::Linear,
        }
    }

    fn level(width: u32, height: u32) -> ImageLevel {
        ImageLevel {
            width,
            height,
            pixels: vec![7; width as usize * height as usize * 4],
        }
    }

    #[test]
    fn register_allocates_monotonic_ordinals_and_queues_create() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        let first = registry.register("a", rgba_image(level(2, 2)), sampling()).unwrap();
        let second = registry.register("b", rgba_image(level(4, 1)), sampling()).unwrap();
        assert_eq!((first.ordinal, second.ordinal), (0, 1));
        assert_eq!(first.width, 2);
        assert_eq!(first.height, 2);
        assert!(registry.require(&first).is_ok());
        assert!(registry.is_resident(&second));
        assert_eq!(registry.get(0), Some(&first));
        assert_eq!(registry.get(9), None);
        let operations = registry.drain_operations();
        assert_eq!(operations.len(), 2);
        assert!(matches!(
            &operations[0],
            ImageResourceOperation::CreateImage { image, .. } if image == &first
        ));
        assert!(registry.drain_operations().is_empty());
    }

    #[test]
    fn allocate_without_commit_is_not_resident() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        let image = registry
            .allocate(
                2,
                2,
                ImageSource::Generated {
                    name: "scratch".to_string(),
                },
            )
            .unwrap();
        assert!(!registry.is_resident(&image));
        assert_eq!(registry.require(&image), Err(RenderError::UnknownImage(image.ordinal)));
        let operation = ImageResourceOperation::CreateImage {
            image: image.clone(),
            content: rgba_image(level(2, 2)),
            sampling: sampling(),
        };
        registry.commit(&operation).unwrap();
        assert!(registry.require(&image).is_ok());
    }

    #[test]
    fn foreign_owner_images_are_rejected() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        let image = registry.register("a", rgba_image(level(1, 1)), sampling()).unwrap();
        let foreign = SceneImageRegistry::new(test_owner(2));
        assert!(matches!(foreign.require(&image), Err(RenderError::ForeignOwner(_))));
        assert!(!foreign.is_resident(&image));
    }

    #[test]
    fn commit_rejects_double_create_and_unknown_images() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        let image = registry.register("a", rgba_image(level(1, 1)), sampling()).unwrap();
        let duplicate = ImageResourceOperation::CreateImage {
            image: image.clone(),
            content: rgba_image(level(1, 1)),
            sampling: sampling(),
        };
        assert_eq!(
            registry.commit(&duplicate),
            Err(RenderError::ImageConflict(image.ordinal))
        );
        let mut forged = image.clone();
        forged.ordinal = 42;
        let forged_create = ImageResourceOperation::CreateImage {
            image: forged,
            content: rgba_image(level(1, 1)),
            sampling: sampling(),
        };
        assert_eq!(registry.commit(&forged_create), Err(RenderError::ImageConflict(42)));
    }

    #[test]
    fn release_then_require_fails_and_queues_release() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        let image = registry.register("a", rgba_image(level(1, 1)), sampling()).unwrap();
        registry.release(&image).unwrap();
        assert!(!registry.is_resident(&image));
        assert_eq!(registry.require(&image), Err(RenderError::UnknownImage(image.ordinal)));
        assert_eq!(registry.release(&image), Err(RenderError::UnknownImage(image.ordinal)));
        let operations = registry.drain_operations();
        assert_eq!(operations.len(), 2);
        assert!(matches!(
            &operations[1],
            ImageResourceOperation::ReleaseImage { image: released } if released == &image
        ));
    }

    #[test]
    fn update_queues_level_uploads_for_residents() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        let image = registry.register("a", rgba_image(level(2, 1)), sampling()).unwrap();
        registry.update(&image, 0, LevelContent::Rgba(level(2, 1))).unwrap();
        let operations = registry.drain_operations();
        assert_eq!(operations.len(), 2);
        assert!(matches!(
            &operations[1],
            ImageResourceOperation::UpdateImage { image: updated, level: 0, .. }
            if updated == &image
        ));
        let mut forged = image.clone();
        forged.ordinal = 7;
        assert_eq!(
            registry.update(&forged, 0, LevelContent::Rgba(level(2, 1))),
            Err(RenderError::UnknownImage(7))
        );
    }

    #[test]
    fn texture_mode_drains() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        registry.texture_mode(TextureFilter::Nearest);
        assert_eq!(
            registry.drain_operations(),
            vec![ImageResourceOperation::TextureMode {
                filter: TextureFilter::Nearest,
            }]
        );
    }

    #[test]
    fn register_validates_dimensions_and_pixels() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        let empty_levels = RenderImage::Rgba8 {
            levels: vec![],
            border_color: Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
        };
        assert!(matches!(
            registry.register("bad", empty_levels, sampling()),
            Err(RenderError::BadDimensions { .. })
        ));
        let zero = ImageLevel {
            width: 0,
            height: 4,
            pixels: vec![],
        };
        assert!(matches!(
            registry.register("bad", rgba_image(zero), sampling()),
            Err(RenderError::BadDimensions { .. })
        ));
        let short = ImageLevel {
            width: 2,
            height: 2,
            pixels: vec![0; 3],
        };
        assert!(matches!(
            registry.register("bad", rgba_image(short), sampling()),
            Err(RenderError::BadDimensions { .. })
        ));
    }

    #[test]
    fn closed_registry_refuses_lifecycle_calls() {
        let mut registry = SceneImageRegistry::new(test_owner(1));
        let image = registry.register("a", rgba_image(level(1, 1)), sampling()).unwrap();
        let _ = registry.drain_operations();
        registry.close();
        assert!(registry.is_closed());
        assert!(matches!(
            registry.allocate(1, 1, image.source.clone()),
            Err(RenderError::OutOfOrder(_))
        ));
        assert!(matches!(
            registry.register("b", rgba_image(level(1, 1)), sampling()),
            Err(RenderError::OutOfOrder(_))
        ));
        assert!(matches!(
            registry.update(&image, 0, LevelContent::Rgba(level(1, 1))),
            Err(RenderError::OutOfOrder(_))
        ));
        let operations = registry.drain_operations();
        assert_eq!(operations.len(), 1);
        assert!(matches!(
            &operations[0],
            ImageResourceOperation::ReleaseImage { image: released } if released == &image
        ));
        registry.close();
        assert!(registry.drain_operations().is_empty());
    }
}
