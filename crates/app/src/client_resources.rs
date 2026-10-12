//! Connection-owned configuration and derived numeric presentation bindings.
use crate::{client_scene::ClientModelBindings, model_assets::ModelCatalog};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{ClientId, ModelRef, ModuleId, ResourceNamespace, RuleSetId},
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
    sounds: qa_audio::SoundBindings,
    sound_rules: RuleSetId,
}
impl ClientResources {
    pub fn load(
        client: ClientId,
        protocol: Protocol,
        sound_rules: RuleSetId,
    ) -> Result<Option<Self>, String> {
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
                sounds: qa_audio::SoundBindings::load(layout.resources[1].count as usize),
                sound_rules,
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
    pub fn sound_rules(&self) -> RuleSetId {
        self.sound_rules
    }
    /// Sound presentation is selected at connection/profile load, independently
    /// of the legacy protocol's resource and message layout.
    pub fn set_sound_rules(&mut self, rules: RuleSetId) {
        self.sound_rules = rules;
    }
    pub fn sound(&self, ordinal: u32) -> Option<qa_core::primitives::SoundId> {
        self.sounds.sound(ordinal)
    }
    /// Cold bank attachment reserves numeric inverse rows before packet intake.
    pub fn bind_sounds(&mut self, bank: &qa_audio::Bank) -> bool {
        self.sounds.attach(bank);
        for (ordinal, index) in self.layout.resources[1].range().enumerate() {
            let Some(config) = self.configs.config(index) else {
                return false;
            };
            if self
                .sounds
                .bind_name(Some(bank), ordinal as u32, config.text)
                .is_err()
            {
                return false;
            }
        }
        true
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
                    self.rejected +=
                        u64::from(self.sounds.bind_name(bank, ordinal as u32, text).is_err());
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
    rows: Box<[PublishedConfig]>,
    pub unsupported: u64,
    pub pending: Option<(u64, usize)>,
}
#[derive(Clone, Copy, Default)]
struct PublishedConfig {
    revision: u64,
    sequence: Option<u64>,
    acknowledged: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceReadiness {
    Pending,
    Ready,
    Unsupported,
}
impl ConfigPublication {
    pub fn load() -> Self {
        Self {
            module: None,
            revision: None,
            rows: vec![
                PublishedConfig::default();
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
        self.rows.fill(PublishedConfig::default());
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
        self.rows
            .get(index)
            .is_some_and(|seen| seen.revision != revision)
    }
    pub fn published(&mut self, index: usize, revision: u64, sequence: Option<u64>) {
        if let Some(seen) = self.rows.get_mut(index) {
            *seen = PublishedConfig {
                revision,
                sequence,
                acknowledged: false,
            };
        }
    }
    pub fn acknowledged(&mut self, module: ModuleId, index: usize, sequence: u64) {
        if self.module == Some(module)
            && let Some(row) = self.rows.get_mut(index)
            && row.sequence == Some(sequence)
        {
            row.acknowledged = true;
        }
    }
    pub fn readiness(&self, module: ModuleId, index: usize) -> ResourceReadiness {
        let Some(row) = self.rows.get(index).filter(|_| self.module == Some(module)) else {
            return ResourceReadiness::Unsupported;
        };
        if row.acknowledged {
            ResourceReadiness::Ready
        } else if row.sequence.is_some() || row.revision == 0 {
            ResourceReadiness::Pending
        } else {
            ResourceReadiness::Unsupported
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
