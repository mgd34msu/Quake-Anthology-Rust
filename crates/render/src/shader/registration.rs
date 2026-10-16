//! Registration provenance and lifetime identities shared by every renderer caller.
use crate::{PaletteId, material::resources::ImageSettings};
use qa_core::primitives::{ClientId, EntityId, ModuleId, NameId, RuleSetId};

/// File provenance and presentation are independent of resource names and geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelProfile {
    pub source_rules: RuleSetId,
    pub rules: RuleSetId,
    pub source_palette: Option<PaletteId>,
    pub palette: Option<PaletteId>,
    pub image_settings: ImageSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderResource {
    World(u32),
    Model(u32),
    Skin(u32),
    Graphic(NameId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderOwner {
    Client(ClientId, EntityId),
    Private(ModuleId, EntityId),
    Engine,
}
