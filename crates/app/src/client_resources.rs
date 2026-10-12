//! Connection-owned configuration and derived numeric presentation bindings.
use crate::{client_scene::ClientModelBindings, model_assets::ModelCatalog};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{ClientId, ModelRef, ModuleId, ResourceNamespace},
};
use qa_network::commands::packet::Protocol;

pub struct ClientResources {
    pub state: ResourceState,
    commands: qa_network::outputs::ConfigCommandReader,
}
pub struct ResourceState {
    pub configs: ConfigStrings,
    pub models: ClientModelBindings,
    pub layout: ConfigLayout,
    pub rejected: u64,
    sounds: Box<[Option<qa_core::primitives::SoundId>]>,
}
impl ClientResources {
    pub fn load(client: ClientId, protocol: Protocol) -> Result<Option<Self>, String> {
        let Some(layout) = ConfigLayout::for_rules(protocol.rule_set()) else {
            return Ok(None);
        };
        Ok(Some(Self {
            commands: qa_network::outputs::ConfigCommandReader::load(),
            state: ResourceState {
                configs: ConfigStrings::load(
                    (0..layout.count).map(|index| layout.capacity(index).unwrap_or(0)),
                )
                .map_err(|error| format!("client configuration: {error:?}"))?,
                models: ClientModelBindings::load(client, layout),
                layout,
                rejected: 0,
                sounds: vec![None; layout.resources[1].count as usize].into_boxed_slice(),
            },
        }))
    }

    pub fn apply(
        &mut self,
        index: usize,
        text: &[u8],
        catalog: Option<&ModelCatalog>,
        bank: Option<&qa_audio::Bank>,
    ) -> bool {
        self.state.apply(index, text, catalog, bank)
    }
    pub fn apply_command(
        &mut self,
        text: &[u8],
        catalog: Option<&ModelCatalog>,
        bank: Option<&qa_audio::Bank>,
    ) -> bool {
        match self.commands.read(text) {
            Ok(Some(config)) => self
                .state
                .apply(config.index as usize, config.text, catalog, bank),
            Ok(None) => true,
            Err(_) => {
                self.state.rejected += 1;
                false
            }
        }
    }
}
impl ResourceState {
    pub fn sound(&self, ordinal: u32) -> Option<qa_core::primitives::SoundId> {
        self.sounds.get(ordinal as usize).copied().flatten()
    }
    fn apply(
        &mut self,
        index: usize,
        text: &[u8],
        catalog: Option<&ModelCatalog>,
        bank: Option<&qa_audio::Bank>,
    ) -> bool {
        match self.configs.set(index, text) {
            Ok(changed) => {
                if changed && let Some(catalog) = catalog {
                    self.rejected +=
                        u64::from(!self.models.config_changed(index, &self.configs, catalog));
                }
                if changed && self.layout.resources[1].contains(index) {
                    let ordinal = index - self.layout.resources[1].first as usize;
                    self.sounds[ordinal] = bank.and_then(|bank| {
                        let path = qa_compat::services::sound_path(text).ok()??;
                        bank.find(std::str::from_utf8(path.as_bytes()).ok()?)
                    });
                }
                true
            }
            Err(_) => {
                self.rejected += 1;
                false
            }
        }
    }
}

/// Publication metadata belongs to the producer. CLIENT reads only received
/// configuration; a stalled channel never makes it observe server storage.
pub struct ConfigPublication {
    module: Option<ModuleId>,
    revision: Option<u64>,
    rows: Box<[u64]>,
    pub unsupported: u64,
    pub pending: Option<(u64, usize)>,
}
impl ConfigPublication {
    pub fn load() -> Self {
        Self {
            module: None,
            revision: None,
            rows: vec![
                0;
                ConfigLayout::for_rules(qa_core::primitives::RuleSetId::Quake2Rerelease)
                    .map_or(0, |layout| layout.count)
            ]
            .into_boxed_slice(),
            unsupported: 0,
            pending: None,
        }
    }
    pub fn reset(&mut self) {
        self.module = None;
        self.revision = None;
        self.rows.fill(0);
        self.pending = None;
    }
    pub fn begin(&mut self, module: ModuleId, revision: u64) -> bool {
        if self.module != Some(module) {
            self.reset();
            self.module = Some(module);
        }
        self.revision != Some(revision)
    }
    pub fn needs(&self, index: usize, revision: u64) -> bool {
        self.rows.get(index).is_some_and(|seen| *seen != revision)
    }
    pub fn published(&mut self, index: usize, revision: u64) {
        if let Some(seen) = self.rows.get_mut(index) {
            *seen = revision;
        }
    }
    pub fn finish(&mut self, revision: u64) {
        self.revision = Some(revision);
    }
}

/// Native resource ordinals are supplied by the owning module/connection,
/// never obtained by narrowing an engine entity or render asset handle.
pub fn model_ordinal(
    reference: ModelRef,
    module: Option<ModuleId>,
    client: ClientId,
    limit: u32,
) -> Option<u32> {
    let namespace_matches = match reference.namespace {
        ResourceNamespace::Module(owner) => Some(owner) == module,
        ResourceNamespace::Client(owner) => owner == client,
    };
    (namespace_matches && reference.ordinal < limit).then_some(reference.ordinal)
}
