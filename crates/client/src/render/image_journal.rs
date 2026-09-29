//! Retained image journal for renderer replacement and resize.
//!
//! Donor provenance: `src/render/image-journal.ts` (`RenderImageJournal`).
//! Successful image mutations are snapshotted (by clone) so a fresh backend
//! can re-upload every resident image; the global texture-mode change replays
//! between the images created before and after it.

use std::collections::BTreeMap;

use super::error::RenderError;
use super::types::{ImageResourceOperation, LevelContent, RenderImage, RendererImage, TextureFilter, TextureSampling};

/// One resident image: its creation plus per-level replacements.
#[derive(Debug, Clone)]
struct ResidentImage {
    before_texture_mode: bool,
    image: RendererImage,
    content: RenderImage,
    sampling: TextureSampling,
    updates: BTreeMap<u32, LevelContent>,
}

/// Resident image summary (`RenderImageJournal.describe` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDescription {
    /// Per-owner image ordinal.
    pub ordinal: u32,
    /// Generated name or requested resource path.
    pub name: String,
    /// Current base-level width (level-0 replacement wins).
    pub width: u32,
    /// Current base-level height.
    pub height: u32,
    /// Creation encoding name.
    pub encoding: &'static str,
    /// Mipmap level count at creation.
    pub mip_levels: usize,
}

fn level_dimensions(content: &LevelContent) -> (u32, u32) {
    match content {
        LevelContent::Rgba(level) => (level.width, level.height),
        LevelContent::Depth(level) => (level.width, level.height),
    }
}

fn upload(record: &ResidentImage, apply: &mut dyn FnMut(&ImageResourceOperation)) {
    apply(&ImageResourceOperation::CreateImage {
        image: record.image.clone(),
        content: record.content.clone(),
        sampling: record.sampling,
    });
    for (level, content) in &record.updates {
        apply(&ImageResourceOperation::UpdateImage {
            image: record.image.clone(),
            level: *level,
            content: content.clone(),
        });
    }
}

fn base_dimensions(content: &RenderImage) -> (u32, u32) {
    match content {
        RenderImage::Indexed8 { levels, .. } | RenderImage::Rgba8 { levels, .. } => {
            levels.first().map_or((0, 0), |level| (level.width, level.height))
        }
        RenderImage::Depth32f { levels } => levels.first().map_or((0, 0), |level| (level.width, level.height)),
    }
}

/// Successful image mutations retained for replay onto a fresh backend.
#[derive(Debug, Clone, Default)]
pub struct RenderImageJournal {
    resident: BTreeMap<u32, ResidentImage>,
    texture_mode: Option<TextureFilter>,
}

impl RenderImageJournal {
    /// Empty journal.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            resident: BTreeMap::new(),
            texture_mode: None,
        }
    }

    /// Re-upload every resident image through `apply`: images created before
    /// the texture-mode change first, then the mode change, then the rest.
    pub fn replay(&self, apply: &mut dyn FnMut(&ImageResourceOperation)) {
        for record in self.resident.values() {
            if record.before_texture_mode {
                upload(record, apply);
            }
        }
        if let Some(filter) = self.texture_mode {
            apply(&ImageResourceOperation::TextureMode { filter });
        }
        for record in self.resident.values() {
            if !record.before_texture_mode {
                upload(record, apply);
            }
        }
    }

    /// Record a successful image mutation, snapshotting its pixels.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::UnknownImage`] when an update names an ordinal
    /// with no resident image.
    pub fn record(&mut self, operation: &ImageResourceOperation) -> Result<(), RenderError> {
        match operation {
            ImageResourceOperation::CreateImage {
                image,
                content,
                sampling,
            } => {
                self.resident.insert(
                    image.ordinal,
                    ResidentImage {
                        before_texture_mode: false,
                        image: image.clone(),
                        content: content.clone(),
                        sampling: *sampling,
                        updates: BTreeMap::new(),
                    },
                );
            }
            ImageResourceOperation::UpdateImage { image, level, content } => {
                let Some(record) = self.resident.get_mut(&image.ordinal) else {
                    return Err(RenderError::UnknownImage(image.ordinal));
                };
                record.updates.insert(*level, content.clone());
            }
            ImageResourceOperation::ReleaseImage { image } => {
                self.resident.remove(&image.ordinal);
            }
            ImageResourceOperation::TextureMode { filter } => {
                self.texture_mode = Some(*filter);
                for record in self.resident.values_mut() {
                    record.before_texture_mode = true;
                }
            }
        }
        Ok(())
    }

    /// Summarize resident images in ordinal order.
    #[must_use]
    pub fn describe(&self) -> Vec<ImageDescription> {
        self.resident
            .values()
            .map(|record| {
                let (width, height) = record
                    .updates
                    .get(&0)
                    .map_or_else(|| base_dimensions(&record.content), level_dimensions);
                ImageDescription {
                    ordinal: record.image.ordinal,
                    name: record.image.source.display_name().to_string(),
                    width,
                    height,
                    encoding: record.content.encoding(),
                    mip_levels: record.content.mip_levels(),
                }
            })
            .collect()
    }

    /// Forget every resident image and the texture-mode change.
    pub fn clear(&mut self) {
        self.resident.clear();
        self.texture_mode = None;
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::super::types::{ImageLevel, ImageSource, ResourceOwner};
    use super::*;

    fn owner() -> ResourceOwner {
        let authority = IdentityOwner::create("image-journal-test").unwrap();
        ResourceOwner::new(1, authority.session().clone(), 0)
    }

    fn image(owner: &ResourceOwner, ordinal: u32, name: &str) -> RendererImage {
        RendererImage {
            owner: owner.clone(),
            ordinal,
            source: ImageSource::Generated { name: name.to_string() },
            width: 4,
            height: 4,
        }
    }

    fn content(width: u32, height: u32) -> RenderImage {
        RenderImage::Rgba8 {
            levels: vec![ImageLevel {
                width,
                height,
                pixels: vec![1; width as usize * height as usize * 4],
            }],
            border_color: qa_core::math::Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
        }
    }

    fn sampling() -> TextureSampling {
        TextureSampling {
            repeat: false,
            filter: TextureFilter::Linear,
        }
    }

    fn create(owner: &ResourceOwner, ordinal: u32) -> ImageResourceOperation {
        ImageResourceOperation::CreateImage {
            image: image(owner, ordinal, "shot"),
            content: content(4, 4),
            sampling: sampling(),
        }
    }

    #[test]
    fn record_replay_round_trips_creations_and_updates() {
        let owner = owner();
        let mut journal = RenderImageJournal::new();
        journal.record(&create(&owner, 0)).unwrap();
        journal
            .record(&ImageResourceOperation::UpdateImage {
                image: image(&owner, 0, "shot"),
                level: 0,
                content: LevelContent::Rgba(ImageLevel {
                    width: 2,
                    height: 2,
                    pixels: vec![2; 16],
                }),
            })
            .unwrap();
        let mut replayed = Vec::new();
        journal.replay(&mut |operation| replayed.push(operation.clone()));
        assert_eq!(replayed.len(), 2);
        assert!(matches!(
            &replayed[0],
            ImageResourceOperation::CreateImage { image, .. } if image.ordinal == 0
        ));
        assert!(matches!(
            &replayed[1],
            ImageResourceOperation::UpdateImage { level: 0, .. }
        ));
    }

    #[test]
    fn texture_mode_replays_between_old_and_new_images() {
        let owner = owner();
        let mut journal = RenderImageJournal::new();
        journal.record(&create(&owner, 0)).unwrap();
        journal
            .record(&ImageResourceOperation::TextureMode {
                filter: TextureFilter::Nearest,
            })
            .unwrap();
        journal.record(&create(&owner, 1)).unwrap();
        let mut kinds = Vec::new();
        journal.replay(&mut |operation| {
            kinds.push(match operation {
                ImageResourceOperation::CreateImage { image, .. } => format!("create{}", image.ordinal),
                ImageResourceOperation::TextureMode { .. } => "mode".to_string(),
                _ => "other".to_string(),
            })
        });
        assert_eq!(kinds, ["create0", "mode", "create1"]);
    }

    #[test]
    fn release_forgets_and_clear_empties() {
        let owner = owner();
        let mut journal = RenderImageJournal::new();
        journal.record(&create(&owner, 0)).unwrap();
        journal.record(&create(&owner, 1)).unwrap();
        journal
            .record(&ImageResourceOperation::ReleaseImage {
                image: image(&owner, 0, "shot"),
            })
            .unwrap();
        assert_eq!(journal.describe().len(), 1);
        journal.clear();
        assert!(journal.describe().is_empty());
        let mut replayed = Vec::new();
        journal.replay(&mut |operation| replayed.push(operation.clone()));
        assert!(replayed.is_empty());
    }

    #[test]
    fn update_without_resident_image_fails() {
        let owner = owner();
        let mut journal = RenderImageJournal::new();
        let result = journal.record(&ImageResourceOperation::UpdateImage {
            image: image(&owner, 9, "ghost"),
            level: 0,
            content: LevelContent::Rgba(ImageLevel {
                width: 1,
                height: 1,
                pixels: vec![0; 4],
            }),
        });
        assert_eq!(result, Err(RenderError::UnknownImage(9)));
    }

    #[test]
    fn describe_reports_level_zero_dimensions() {
        let owner = owner();
        let mut journal = RenderImageJournal::new();
        journal.record(&create(&owner, 3)).unwrap();
        journal
            .record(&ImageResourceOperation::UpdateImage {
                image: image(&owner, 3, "shot"),
                level: 0,
                content: LevelContent::Rgba(ImageLevel {
                    width: 8,
                    height: 2,
                    pixels: vec![0; 64],
                }),
            })
            .unwrap();
        assert_eq!(
            journal.describe(),
            [ImageDescription {
                ordinal: 3,
                name: "shot".to_string(),
                width: 8,
                height: 2,
                encoding: "rgba8",
                mip_levels: 1,
            }]
        );
    }

    #[test]
    fn snapshots_are_isolated_from_later_mutation() {
        let owner = owner();
        let mut operation = create(&owner, 0);
        let mut journal = RenderImageJournal::new();
        journal.record(&operation).unwrap();
        if let ImageResourceOperation::CreateImage { content, .. } = &mut operation {
            if let RenderImage::Rgba8 { levels, .. } = content {
                levels[0].pixels.fill(255);
            }
        }
        let mut replayed = Vec::new();
        journal.replay(&mut |op| replayed.push(op.clone()));
        let ImageResourceOperation::CreateImage { content, .. } = &replayed[0] else {
            panic!("expected creation");
        };
        let RenderImage::Rgba8 { levels, .. } = content else {
            panic!("expected rgba8");
        };
        assert!(levels[0].pixels.iter().all(|pixel| *pixel == 1));
    }
}
