use super::{Assets, Sampler};

/// Numeric resource extents admitted by a renderer, independent of filenames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetCounts {
    pub images: usize,
    pub materials: usize,
    pub models: usize,
    pub worlds: usize,
    pub presentations: usize,
    pub palettes: usize,
}
impl AssetCounts {
    pub fn contains(self, required: Self) -> bool {
        self.images >= required.images
            && self.materials >= required.materials
            && self.models >= required.models
            && self.worlds >= required.worlds
            && self.presentations >= required.presentations
            && self.palettes >= required.palettes
    }
}

impl Assets {
    pub fn counts(&self) -> AssetCounts {
        AssetCounts {
            images: self.images.len(),
            materials: self.materials.len(),
            models: self.models.len(),
            worlds: self.worlds.len(),
            presentations: self.presentations.len(),
            palettes: self.palettes.len(),
        }
    }

    /// Discard resources from a rejected cold registration. Interned names
    /// retain their identities; every admitted numeric resource prefix stays.
    pub fn discard_registration_suffix(
        &mut self,
        retained: AssetCounts,
    ) -> Result<(), &'static str> {
        if !self.counts().contains(retained) {
            return Err("invalid registration resource prefix");
        }
        self.worlds.truncate(retained.worlds);
        self.presentations.truncate(retained.presentations);
        for world in &mut self.worlds {
            world
                .presentations
                .retain(|id| (id.0 as usize) < retained.presentations);
        }
        self.models.truncate(retained.models);
        self.materials.truncate(retained.materials);
        self.images.truncate(retained.images);
        self.palettes.truncate(retained.palettes);
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ImageVersion {
    preparation: u64,
    sampler: Option<Sampler>,
}
impl ImageVersion {
    fn of(image: &super::Image) -> Self {
        Self {
            preparation: image.preparation_revision,
            sampler: image.native_sampler,
        }
    }
}

/// A prepared resource prefix. Candidate ownership moves into the backend only
/// after its fallible CPU preparation or GL upload has succeeded.
pub(crate) struct AssetAdmission {
    counts: AssetCounts,
    images: Box<[ImageVersion]>,
}
impl AssetAdmission {
    pub(crate) fn prepare(
        previous: Option<&Self>,
        assets: &Assets,
    ) -> Result<Option<Self>, &'static str> {
        let counts = assets.counts();
        if let Some(previous) = previous {
            let old = previous.counts;
            if !counts.contains(old) {
                return Err("renderer asset prefix changed incompatibly");
            }
            if previous
                .images
                .iter()
                .zip(assets.images())
                .any(|(old, image)| *old != ImageVersion::of(image))
            {
                return Err("renderer image preparation changed");
            }
            if old == counts {
                return Ok(None);
            }
        }
        Ok(Some(Self {
            counts,
            images: assets.images().iter().map(ImageVersion::of).collect(),
        }))
    }

    pub(crate) fn counts(&self) -> AssetCounts {
        self.counts
    }
}
