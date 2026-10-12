//! Connection-owned configuration and derived numeric presentation bindings.
use crate::{client_scene::ClientModelBindings, model_assets::ModelCatalog};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{
        ClientId, EntityRecord, ModelRef, ModuleId, ResourceNamespace, RuleSetId, SpatialPolicy,
    },
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
    sound_policy: SpatialPolicy,
    static_sounds: u32,
    static_entities: Box<[EntityRecord]>,
    static_entity_count: usize,
}
impl ClientResources {
    pub fn load(
        client: ClientId,
        protocol: Protocol,
        sound_rules: RuleSetId,
    ) -> Result<Option<Self>, String> {
        let Some(layout) = protocol.config_layout() else {
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
                sound_policy: qa_gameplay::rules::sound_policy(sound_rules),
                static_sounds: 0,
                static_entities: vec![
                    EntityRecord::default();
                    qa_core::primitives::NATIVE_WIRE_BYTES
                        / qa_network::outputs::STATIC_ENTITY_BYTES
                ]
                .into_boxed_slice(),
                static_entity_count: 0,
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
    /// Native signon copies survive freeing or reusing their source edicts.
    pub fn receive_static_entity(&mut self, mut record: EntityRecord) -> bool {
        let Some(slot) = self.static_entities.get_mut(self.static_entity_count) else {
            self.rejected += 1;
            return false;
        };
        for reference in &mut record.appearance.models {
            *reference = reference.and_then(|model| {
                ModelRef::native(
                    ResourceNamespace::Client(self.models.client()),
                    model.ordinal,
                )
            });
        }
        *slot = record;
        self.static_entity_count += 1;
        true
    }

    pub fn static_entities(&self) -> &[EntityRecord] {
        &self.static_entities[..self.static_entity_count]
    }

    pub fn static_entity_capacity(&self) -> usize {
        self.static_entities.len()
    }

    pub(crate) fn static_emitter(
        &mut self,
        client: ClientId,
        lifetime: qa_core::primitives::EntityId,
    ) -> Option<qa_core::primitives::SoundEmitter> {
        let slot = self.static_sounds;
        self.static_sounds = self.static_sounds.checked_add(1)?;
        Some(qa_core::primitives::SoundEmitter::Static {
            namespace: ResourceNamespace::Client(client),
            lifetime,
            slot,
        })
    }
    pub fn sound_policy(&self) -> SpatialPolicy {
        self.sound_policy
    }
    /// Sound presentation is selected at connection/profile load, independently
    /// of the legacy protocol's resource and message layout.
    pub fn set_sound_policy(&mut self, policy: SpatialPolicy) {
        self.sound_policy = policy;
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
                .bind_name(Some(bank), ordinal as u32, config.text, self.layout.rules)
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
                    self.rejected += u64::from(
                        self.sounds
                            .bind_name(bank, ordinal as u32, text, self.layout.rules)
                            .is_err(),
                    );
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

/// Producer-owned configuration readiness and the destination sound namespace.
/// CLIENT state above contains only received rows; it is never read to project
/// a SERVER sound. All names still belong to source storage or the PCM bank.
pub struct ConfigPublication {
    module: Option<ModuleId>,
    layout: Option<ConfigLayout>,
    revisions: Box<[Option<u64>]>,
    seen: Box<[u64]>,
    rows: Box<[PublishedConfig]>,
    sounds: qa_audio::SoundBindings,
    primary_claims: Box<[bool]>,
    pub unsupported: u64,
    pub pending: Option<(u64, usize)>,
}
#[derive(Clone, Copy, Default)]
struct PublishedConfig {
    sequence: Option<u64>,
    acknowledged: bool,
    unsupported: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceReadiness {
    Pending,
    Ready,
    Unsupported,
}
impl ConfigPublication {
    pub fn load() -> Self {
        let layout = ConfigLayout::for_rules(RuleSetId::Quake2Rerelease);
        let count = layout.count;
        let sounds = layout.resources[1].count as usize;
        Self {
            module: None,
            layout: None,
            revisions: Box::new([]),
            seen: vec![0; count].into_boxed_slice(),
            rows: vec![PublishedConfig::default(); count].into_boxed_slice(),
            sounds: qa_audio::SoundBindings::load(sounds),
            primary_claims: vec![false; sounds].into_boxed_slice(),
            unsupported: 0,
            pending: None,
        }
    }
    /// Source capacities are fixed when the module set is loaded.
    pub fn configure_sources(&mut self, capacity: usize) {
        self.revisions = vec![None; capacity].into_boxed_slice();
        self.reset();
    }
    pub fn bind_sounds(&mut self, bank: &qa_audio::Bank) {
        self.sounds.attach(bank);
        self.revisions.fill(None);
        self.seen.fill(0);
        self.rows.fill(PublishedConfig::default());
        self.primary_claims.fill(false);
    }
    pub fn reset(&mut self) {
        self.module = None;
        self.layout = None;
        self.revisions.fill(None);
        self.seen.fill(0);
        self.rows.fill(PublishedConfig::default());
        self.sounds.clear();
        self.primary_claims.fill(false);
        self.pending = None;
    }
    pub fn connection(&mut self, module: ModuleId, layout: ConfigLayout) {
        if self.module != Some(module) || self.layout.map(|old| old.rules) != Some(layout.rules) {
            self.reset();
            self.module = Some(module);
            self.layout = Some(layout);
        }
    }
    pub fn begin(&self, module: ModuleId, revision: u64) -> bool {
        self.revisions
            .get(module.0 as usize)
            .is_some_and(|seen| *seen != Some(revision))
    }
    pub fn needs(&self, index: usize, revision: u64) -> bool {
        self.seen.get(index).is_some_and(|seen| *seen != revision)
    }
    pub fn seen(&mut self, index: usize, revision: u64) {
        if let Some(seen) = self.seen.get_mut(index) {
            *seen = revision;
        }
    }
    pub fn published(&mut self, destination: usize, sequence: Option<u64>) {
        if let Some(seen) = self.rows.get_mut(destination) {
            *seen = PublishedConfig {
                sequence,
                acknowledged: false,
                unsupported: sequence.is_none(),
            };
        }
    }
    /// The ring supplies the proved ACK and the immutable destination row.
    /// An old row's receipt cannot ready its later rewrite or relocation.
    pub fn unpublished(&self, destination: usize) -> bool {
        self.rows
            .get(destination)
            .is_some_and(|row| row.sequence.is_none() && !row.unsupported)
    }
    pub fn acknowledged(&mut self, destination: usize, sequence: u64) {
        if let Some(row) = self.rows.get_mut(destination)
            && row.sequence == Some(sequence)
        {
            row.acknowledged = true;
        }
    }
    pub fn readiness(&self, destination: usize) -> ResourceReadiness {
        let Some(row) = self.rows.get(destination) else {
            return ResourceReadiness::Unsupported;
        };
        if row.acknowledged {
            ResourceReadiness::Ready
        } else if row.unsupported {
            ResourceReadiness::Unsupported
        } else {
            ResourceReadiness::Pending
        }
    }
    pub fn finish(&mut self, module: ModuleId, revision: u64) {
        if let Some(seen) = self.revisions.get_mut(module.0 as usize) {
            *seen = Some(revision);
        }
    }
    /// Actual nonempty primary registrations own their native ordinals. A new
    /// primary claim may displace a secondary sample; its numeric SoundId can
    /// be registered again in a free slot without aliasing the new primary.
    pub fn primary_sound(
        &mut self,
        ordinal: u32,
        claimed: bool,
        sound: Option<qa_core::primitives::SoundId>,
    ) {
        let Some(layout) = self.layout else {
            return;
        };
        if ordinal >= layout.resources[1].count {
            return;
        }
        let previous = self.primary_claims[ordinal as usize];
        self.primary_claims[ordinal as usize] = claimed;
        if claimed || previous {
            if self.sounds.sound(ordinal) != sound {
                self.rows[layout.resources[1].first as usize + ordinal as usize] =
                    PublishedConfig::default();
            }
            let _ = self.sounds.set(ordinal, sound);
        }
    }
    /// Free duplicate secondary rows after a native primary claim, so growing
    /// primary precaches do not consume two slots for the same loaded sample.
    pub fn release_aliases(&mut self) {
        let Some(layout) = self.layout else {
            return;
        };
        // Lowest actual primary registration wins, independently of a lower
        // secondary slot that happened to be occupied before primary growth.
        for ordinal in (1..layout.resources[1].count).rev() {
            if self.primary_claims[ordinal as usize] {
                self.sounds.prefer(ordinal);
            }
        }
        for ordinal in 1..layout.resources[1].count {
            if !self.primary_claims[ordinal as usize]
                && let Some(sound) = self.sounds.sound(ordinal)
                && self.sounds.ordinal(sound) != Some(ordinal)
            {
                let _ = self.sounds.set(ordinal, None);
                self.rows[layout.resources[1].first as usize + ordinal as usize] =
                    PublishedConfig::default();
            }
        }
    }
    pub fn sound(&self, sound: qa_core::primitives::SoundId) -> Option<(u32, ResourceReadiness)> {
        let layout = self.layout?;
        let ordinal = self
            .sounds
            .ordinal(sound)
            .filter(|&ordinal| ordinal < layout.resources[1].count)?;
        Some((
            ordinal,
            self.readiness(layout.resources[1].first as usize + ordinal as usize),
        ))
    }
    pub fn project_sound(
        &self,
        module: ModuleId,
        native: Option<u32>,
        sound: qa_core::primitives::SoundId,
    ) -> Option<(u32, ResourceReadiness)> {
        if Some(module) == self.module
            && let Some(ordinal) = native
        {
            let layout = self.layout?;
            if ordinal < layout.resources[1].count && self.sounds.sound(ordinal) == Some(sound) {
                return Some((
                    ordinal,
                    self.readiness(layout.resources[1].first as usize + ordinal as usize),
                ));
            }
        }
        self.sound(sound)
    }
    /// Called only while changed precaches are published, never at emission.
    /// Secondary slots grow downwards, leaving low native ordinals available.
    pub fn reserve_sound(&mut self, sound: qa_core::primitives::SoundId) -> Option<u32> {
        let layout = self.layout?;
        if let Some((ordinal, _)) = self.sound(sound) {
            return Some(ordinal);
        }
        let ordinal = (1..layout.resources[1].count).rev().find(|&ordinal| {
            !self.primary_claims[ordinal as usize] && self.sounds.sound(ordinal).is_none()
        })?;
        self.sounds.set(ordinal, Some(sound)).ok()?;
        self.rows[layout.resources[1].first as usize + ordinal as usize] =
            PublishedConfig::default();
        Some(ordinal)
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
