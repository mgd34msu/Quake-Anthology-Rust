//! Registration provenance and lifetime identities shared by every renderer caller.
use crate::{
    PaletteId,
    material::{
        MaterialFinishPolicy,
        resources::{ImageProfile, ImageSettings},
    },
};
use qa_core::primitives::{ClientId, EntityId, ModelFileId, ModuleId, NameId, RuleSetId};

/// File provenance and presentation are independent of resource names and geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelProfile {
    pub source_rules: RuleSetId,
    pub rules: RuleSetId,
    pub source_palette: Option<PaletteId>,
    pub palette: Option<PaletteId>,
    pub image_settings: ImageSettings,
    pub finish: MaterialFinishPolicy,
}
impl ModelProfile {
    pub fn image_profile(self) -> ImageProfile {
        ImageProfile {
            source_rules: self.source_rules,
            presentation_rules: self.rules,
            source_palette: self.source_palette,
            target_palette: self.palette,
            image_settings: self.image_settings,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderResource {
    World(u32),
    Model(ModelFileId),
    Skin(u32),
    Graphic(NameId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderOwner {
    Client(ClientId, EntityId),
    Private(ModuleId, EntityId),
    Engine,
}
