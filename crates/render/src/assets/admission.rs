use super::{Assets, Sampler};

/// Numeric resource extents admitted by a renderer, independent of filenames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetCounts {
    pub images: usize,
    pub materials: usize,
    pub shader_bindings: usize,
    pub models: usize,
    pub worlds: usize,
    pub presentations: usize,
    pub palettes: usize,
}
impl AssetCounts {
    pub fn contains(self, required: Self) -> bool {
        self.images >= required.images
            && self.materials >= required.materials
            && self.shader_bindings >= required.shader_bindings
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
            shader_bindings: self.shader_bindings.len(),
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
            if let Some(source) = &mut world.material_source {
                let presentations: Vec<_> = world
                    .presentations
                    .iter()
                    .map(|id| &self.presentations[id.0 as usize])
                    .collect();
                source.prune_registration(retained.images, &presentations);
            }
        }
        self.models.truncate(retained.models);
        for index in (retained.shader_bindings..self.shader_bindings.len()).rev() {
            let binding = self.shader_bindings[index];
            self.material_shader_heads[binding.material.0 as usize] = self.shader_next[index];
        }
        self.shader_bindings.truncate(retained.shader_bindings);
        self.shader_next.truncate(retained.shader_bindings);
        self.material_shader_heads.truncate(retained.materials);
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
    model_preparation_revision: u64,
}
impl AssetAdmission {
    pub(crate) fn prepare(
        previous: Option<&Self>,
        assets: &Assets,
    ) -> Result<Option<Self>, &'static str> {
        let counts = assets.counts();
        if assets
            .shader_bindings()
            .iter()
            .any(|&binding| !assets.styled_shader_valid(binding))
        {
            return Err("invalid styled shader binding");
        }
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
            if old == counts
                && previous.model_preparation_revision == assets.model_preparation_revision
            {
                return Ok(None);
            }
        }
        Ok(Some(Self {
            counts,
            images: assets.images().iter().map(ImageVersion::of).collect(),
            model_preparation_revision: assets.model_preparation_revision,
        }))
    }

    pub(crate) fn counts(&self) -> AssetCounts {
        self.counts
    }
}
