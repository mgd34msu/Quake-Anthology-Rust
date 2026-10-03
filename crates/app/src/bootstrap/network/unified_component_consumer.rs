//! Client-side original component replica.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/network/unified-component-consumer.ts`
//! (`UnifiedComponentConsumers`).
//!
//! The replica owns only original client presentation and its exact server
//! activation lease. Donor closures over entries become methods on the
//! collection; asynchronous content loading runs synchronously with order
//! preserved. Content ([`LoadedApplicationContent`](super::super::content::LoadedApplicationContent)),
//! presentation events
//! ([`SimulationPresentationEvent`](super::super::simulation::types::SimulationPresentationEvent)),
//! and user files ([`ModUserFiles`](qa_content::contract::ModUserFiles)) arrive through
//! [`UnifiedComponentHost`], which carries the host-side handles.

use std::collections::HashMap;

use qa_content::contract::{
    borrow_mod_file_mounts, ContentId, ContractError, ModSelection as ContractModSelection, ModUserFiles,
    PresentationOwner,
};
use qa_content::mounts::MountedContent;
use qa_core::identity::{ActorId, ProviderId};
use qa_guest::qvm::client_state::GameStateRecord;
use qa_guest::qvm::mod_presentation::{QvmSceneActor, QvmSceneCommand};
use qa_platform::files::writable::UserFileStore;

use super::unified_components::{
    same_owner, UnifiedComponentAbi, UnifiedComponentFrame, UnifiedComponentFrames, UnifiedComponentRuntime,
    UnifiedComponentSnapshot, UnifiedComponentState, UnifiedComponentUpdate, UnifiedSceneSnapshotData,
};
use crate::persistence::mods::{same_mod_identity, ModIdentity, ModSelection};

/// Component consumer failure.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedComponentConsumerError {
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Consumer failure.
    #[error("{0}")]
    Consumer(String),
}

/// Locally qualified gameplay presentation for one prepared mod.
#[derive(Debug, Clone)]
pub struct PreparedComponentMod {
    /// Mod identity.
    pub identity: ModIdentity,
    /// Declared runtime.
    pub runtime: UnifiedComponentRuntime,
    /// Declared ABI profile.
    pub abi: UnifiedComponentAbi,
    /// Presentation source provider.
    pub source_provider: ProviderId,
    /// Declared HUD mode, when the mod presents one.
    pub hud_mode: Option<String>,
}

/// Content, event, and file surface the consumer needs.
pub trait UnifiedComponentHost {
    /// Fail when the owning presentation moved on.
    fn assert_current(&self) -> Result<(), UnifiedComponentConsumerError>;
    /// Viewing actor, if any.
    fn viewer(&self) -> Option<ActorId>;
    /// Send a component command.
    fn send_command(
        &self,
        owner: &PresentationOwner,
        generation: i64,
        args: &[String],
    ) -> Result<(), UnifiedComponentConsumerError>;
    /// Admit a replicated owner.
    fn admit_replicated_owner(
        &mut self,
        owner: &PresentationOwner,
        content: &str,
    ) -> Result<(), UnifiedComponentConsumerError>;
    /// Retire a replicated owner.
    fn retire_replicated_owner(&mut self, owner: &PresentationOwner) -> Result<(), UnifiedComponentConsumerError>;
    /// Locally prepared gameplay presentations.
    fn prepared_mods(&self) -> Vec<PreparedComponentMod>;
    /// Installed content mounts.
    fn for_content(&self, content: &str) -> Result<MountedContent, UnifiedComponentConsumerError>;
    /// Writable user files.
    fn user_files(&mut self) -> &mut ModUserFiles;
    /// Take extracted user files, when the host owns them.
    fn take_files(&mut self) -> Option<ModUserFiles> {
        None
    }
}

/// Live scene assembled from a frame and retained reliable state.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentScene {
    /// Scene revision.
    pub revision: i64,
    /// Scene snapshot.
    pub snapshot: UnifiedSceneSnapshotData,
    /// Game state.
    pub game_state: GameStateRecord,
    /// Game-state revision.
    pub game_state_revision: i64,
    /// Scene actors.
    pub actors: Vec<QvmSceneActor>,
    /// Retained server commands.
    pub commands: Vec<QvmSceneCommand>,
    /// Baseline scene, once captured.
    pub baseline: Option<Box<UnifiedComponentScene>>,
}

/// Presentation context for one viewer (donor `context` output).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentContextView {
    /// Client number.
    pub client_number: i64,
    /// Game state.
    pub game_state: GameStateRecord,
    /// Game-state revision.
    pub game_state_revision: i64,
    /// Viewer snapshot.
    pub snapshot: UnifiedComponentSnapshot,
    /// Weapon-presented flag.
    pub weapon_presented: bool,
    /// Live scene, for scene runtimes.
    pub scene: Option<UnifiedComponentScene>,
}

/// Mounts backing one activation (donor `files` output).
pub struct UnifiedComponentFiles<'a> {
    /// Borrowed file mounts.
    pub mounts: &'a MountedContent,
    /// Writable store.
    pub writable: &'a UserFileStore,
}

/// Client presentation frame (donor `frame` output; view is always null).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedComponentClientFrame {
    /// HUD mode.
    pub hud_mode: String,
}

/// Admitted component source (donor `ActiveModPresentation` handle).
#[derive(Debug, Clone)]
pub struct UnifiedComponentSource {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Component identity.
    pub identity: ModIdentity,
    /// Activation generation.
    pub generation: i64,
    /// ABI profile.
    pub abi: UnifiedComponentAbi,
    /// Runtime.
    pub runtime: UnifiedComponentRuntime,
    /// Qualified presentation.
    pub prepared: PreparedComponentMod,
}

/// Admitted client source (donor `ActiveModClientPresentation` handle).
#[derive(Debug, Clone)]
pub struct UnifiedComponentClientSource {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Component identity.
    pub identity: ModIdentity,
    /// Activation generation.
    pub generation: i64,
}

struct ComponentEntry {
    owner: PresentationOwner,
    identity: ModIdentity,
    prepared: PreparedComponentMod,
    mounts: MountedContent,
    borrowed: bool,
    hud_mode: Option<String>,
    metadata: UnifiedComponentState,
    game_state: GameStateRecord,
    commands: Vec<QvmSceneCommand>,
    command_sequence: i64,
    frame: Option<UnifiedComponentFrame>,
    frame_game_state: GameStateRecord,
    baseline: Option<UnifiedComponentScene>,
    scene: Option<UnifiedComponentScene>,
}

fn contract_selection(selection: &ModSelection) -> ContractModSelection {
    ContractModSelection {
        product: selection.product.clone(),
        id: selection.id.clone(),
    }
}

/// The replica owns only original client presentation and its exact server activation lease.
pub struct UnifiedComponentConsumers {
    host: Box<dyn UnifiedComponentHost>,
    entries: HashMap<ProviderId, ComponentEntry>,
    revision: i64,
    closed: bool,
}

impl UnifiedComponentConsumers {
    /// Empty collection over a host.
    /// Borrow the backing host.
    pub fn host_mut(&mut self) -> &mut dyn UnifiedComponentHost {
        &mut *self.host
    }

    pub fn new(host: Box<dyn UnifiedComponentHost>) -> Self {
        Self {
            host,
            entries: HashMap::new(),
            revision: 0,
            closed: false,
        }
    }

    fn current_check(&self) -> Result<(), UnifiedComponentConsumerError> {
        self.host.assert_current()?;
        if self.closed {
            return Err(UnifiedComponentConsumerError::Consumer(
                "Remote component collection is retired".to_string(),
            ));
        }
        Ok(())
    }

    fn retire(
        host: &mut dyn UnifiedComponentHost,
        entry: &ComponentEntry,
        activation: bool,
    ) -> Result<(), UnifiedComponentConsumerError> {
        if activation {
            host.retire_replicated_owner(&entry.owner)?;
        }
        if entry.borrowed {
            entry.mounts.close();
        }
        Ok(())
    }

    /// Admit a reliable update.
    pub fn update(&mut self, update: &UnifiedComponentUpdate) -> Result<(), UnifiedComponentConsumerError> {
        self.current_check()?;
        if update.revision != self.revision + 1 {
            return Err(UnifiedComponentConsumerError::Consumer(
                "Remote component reliable revision is not consecutive".to_string(),
            ));
        }
        let mut old = std::mem::take(&mut self.entries);
        let mut next: HashMap<ProviderId, ComponentEntry> = HashMap::new();
        let mut created: Vec<ProviderId> = Vec::new();
        let mut replaced: Vec<(ProviderId, ComponentEntry)> = Vec::new();
        let outcome = Self::apply_update(
            &mut *self.host,
            self.closed,
            &mut old,
            &mut next,
            &mut created,
            &mut replaced,
            update,
        );
        if outcome.is_err() {
            for id in &created {
                if let Some(entry) = next.remove(id) {
                    if entry.borrowed {
                        entry.mounts.close();
                    }
                }
            }
            for (id, entry) in next {
                old.insert(id, entry);
            }
            for (id, entry) in replaced {
                old.insert(id, entry);
            }
            self.entries = old;
            return outcome;
        }
        let mut retirees: Vec<(ProviderId, ComponentEntry)> = replaced;
        retirees.extend(old);
        for (id, entry) in retirees {
            let activation =
                next.contains_key(&id) && !next.get(&id).is_some_and(|kept| same_owner(&kept.owner, &entry.owner));
            Self::retire(&mut *self.host, &entry, activation)?;
        }
        self.entries = next;
        self.revision = update.revision;
        for metadata in &update.sources {
            let entry = self
                .entries
                .get_mut(&metadata.identity_block.owner.provider)
                .ok_or_else(|| {
                    UnifiedComponentConsumerError::Consumer("Remote component admission disappeared".to_string())
                })?;
            self.host.admit_replicated_owner(
                &metadata.identity_block.owner,
                &metadata.identity_block.identity.source.content,
            )?;
            let game_state = metadata.game_state.clone().unwrap_or_else(|| entry.game_state.clone());
            entry.metadata = metadata.clone();
            entry.game_state = game_state;
            entry.commands.extend(metadata.commands.iter().cloned());
            if entry.commands.len() > 64 {
                entry.commands.drain(..entry.commands.len() - 64);
            }
            let sequence = i128::from(metadata.command_base) + metadata.commands.len() as i128;
            entry.command_sequence = i64::try_from(sequence).map_err(|_| {
                UnifiedComponentConsumerError::Consumer(
                    "Remote component reliable command history has a gap".to_string(),
                )
            })?;
        }
        Ok(())
    }

    fn apply_update(
        host: &mut dyn UnifiedComponentHost,
        closed: bool,
        old: &mut HashMap<ProviderId, ComponentEntry>,
        next: &mut HashMap<ProviderId, ComponentEntry>,
        created: &mut Vec<ProviderId>,
        replaced: &mut Vec<(ProviderId, ComponentEntry)>,
        update: &UnifiedComponentUpdate,
    ) -> Result<(), UnifiedComponentConsumerError> {
        for metadata in &update.sources {
            let provider = metadata.identity_block.owner.provider.clone();
            let claimed = old.remove(&provider);
            let same = claimed.as_ref().is_some_and(|entry| {
                same_owner(&entry.owner, &metadata.identity_block.owner)
                    && entry.metadata.identity_block.generation == metadata.identity_block.generation
            });
            let entry = match claimed {
                Some(entry) if same => {
                    if !same_mod_identity(&entry.identity, &metadata.identity_block.identity)
                        || entry.metadata.identity_block.abi != metadata.identity_block.abi
                        || entry.metadata.identity_block.runtime != metadata.identity_block.runtime
                    {
                        return Err(UnifiedComponentConsumerError::Consumer(
                            "Remote component activation changed its admitted identity".to_string(),
                        ));
                    }
                    entry
                }
                previous => {
                    if let Some(previous) = previous {
                        replaced.push((provider.clone(), previous));
                    }
                    let prepared = host
                        .prepared_mods()
                        .into_iter()
                        .find(|candidate| same_mod_identity(&candidate.identity, &metadata.identity_block.identity));
                    let Some(prepared) = prepared else {
                        return Err(UnifiedComponentConsumerError::Consumer(
                            "Remote component differs from its locally qualified presentation".to_string(),
                        ));
                    };
                    if prepared.runtime != metadata.identity_block.runtime
                        || prepared.abi != metadata.identity_block.abi
                        || prepared.source_provider != metadata.identity_block.owner.provider
                    {
                        return Err(UnifiedComponentConsumerError::Consumer(
                            "Remote component differs from its locally qualified presentation".to_string(),
                        ));
                    }
                    let Some(game_state) = metadata.game_state.clone() else {
                        return Err(UnifiedComponentConsumerError::Consumer(
                            "Remote component activation has no original gamestate".to_string(),
                        ));
                    };
                    let installed = host.for_content(&metadata.identity_block.identity.source.content)?;
                    host.assert_current()?;
                    if closed {
                        return Err(UnifiedComponentConsumerError::Consumer(
                            "Remote component collection is retired".to_string(),
                        ));
                    }
                    let selection = contract_selection(&metadata.identity_block.identity.selection);
                    let writable = host.user_files().store(&selection)?;
                    let content = ContentId(metadata.identity_block.identity.source.content.clone());
                    let mounts = borrow_mod_file_mounts(&selection, content, Some(&installed), writable)?;
                    created.push(provider.clone());
                    let hud_mode = prepared.hud_mode.clone();
                    ComponentEntry {
                        owner: metadata.identity_block.owner.clone(),
                        identity: metadata.identity_block.identity.clone(),
                        prepared,
                        mounts,
                        borrowed: true,
                        hud_mode,
                        metadata: metadata.clone(),
                        game_state: game_state.clone(),
                        commands: Vec::new(),
                        command_sequence: metadata.command_base,
                        frame: None,
                        frame_game_state: game_state,
                        baseline: None,
                        scene: None,
                    }
                }
            };
            if metadata.command_base != entry.command_sequence
                || metadata.commands.iter().enumerate().any(|(index, command)| {
                    i128::from(command.sequence) != i128::from(metadata.command_base) + index as i128 + 1
                })
            {
                return Err(UnifiedComponentConsumerError::Consumer(
                    "Remote component reliable command history has a gap".to_string(),
                ));
            }
            if metadata.game_state_revision < entry.metadata.game_state_revision
                || metadata.game_state.is_none() && metadata.game_state_revision != entry.metadata.game_state_revision
            {
                return Err(UnifiedComponentConsumerError::Consumer(
                    "Remote component configstring history has a gap".to_string(),
                ));
            }
            next.insert(provider, entry);
        }
        host.assert_current()?;
        if closed {
            return Err(UnifiedComponentConsumerError::Consumer(
                "Remote component collection is retired".to_string(),
            ));
        }
        Ok(())
    }

    /// Accept volatile frames; stale revisions report `false`.
    pub fn accept(
        &mut self,
        frames: &UnifiedComponentFrames,
        viewer: &ActorId,
    ) -> Result<bool, UnifiedComponentConsumerError> {
        self.current_check()?;
        if frames.revision < self.revision {
            return Ok(false);
        }
        if frames.revision != self.revision || frames.sources.len() != self.entries.len() {
            return Err(UnifiedComponentConsumerError::Consumer(
                "Remote component frame lacks reliable admission".to_string(),
            ));
        }
        for frame in &frames.sources {
            let entry = self.entries.get(&frame.owner.provider);
            let admitted = entry.is_some_and(|entry| {
                same_owner(&entry.owner, &frame.owner)
                    && entry.metadata.identity_block.generation == frame.generation
                    && entry.metadata.identity_block.abi == frame.abi
                    && entry.metadata.game_state_revision == frame.game_state_revision
                    && frame.viewer == *viewer
                    && frame.bindings.iter().any(|binding| {
                        binding.actor == *viewer
                            && i64::try_from(binding.slot).is_ok_and(|slot| slot == frame.client_number)
                            && !binding.owned
                    })
                    && (entry.metadata.identity_block.runtime == UnifiedComponentRuntime::Scene)
                        == frame.scene.is_some()
            });
            if !admitted {
                return Err(UnifiedComponentConsumerError::Consumer(
                    "Remote component frame differs from its source or recipient admission".to_string(),
                ));
            }
            if let Some(scene) = frame.scene.as_ref() {
                let sequence = entry.map(|entry| entry.command_sequence);
                if sequence.is_none_or(|sequence| scene.snapshot.server_command_sequence != sequence)
                    || scene.snapshot.server_time != frame.snapshot.server_time
                {
                    return Err(UnifiedComponentConsumerError::Consumer(
                        "Remote component snapshot lost its reliable source history".to_string(),
                    ));
                }
            }
        }
        for frame in &frames.sources {
            let entry = self.entries.get_mut(&frame.owner.provider).ok_or_else(|| {
                UnifiedComponentConsumerError::Consumer("Remote component frame owner disappeared".to_string())
            })?;
            entry.frame_game_state = entry.game_state.clone();
            if let Some(scene) = frame.scene.as_ref() {
                let live = UnifiedComponentScene {
                    revision: scene.revision,
                    snapshot: scene.snapshot.clone(),
                    game_state: entry.game_state.clone(),
                    game_state_revision: frame.game_state_revision,
                    actors: frame.bindings.clone(),
                    commands: entry.commands.clone(),
                    baseline: None,
                };
                if entry.baseline.is_none() {
                    entry.baseline = Some(live.clone());
                }
                entry.scene = Some(UnifiedComponentScene {
                    baseline: entry.baseline.clone().map(Box::new),
                    ..live
                });
            }
            entry.frame = Some(frame.clone());
        }
        Ok(true)
    }

    /// Admitted sources with a frame, ordered by provider.
    pub fn sources(&self) -> Result<Vec<UnifiedComponentSource>, UnifiedComponentConsumerError> {
        self.current_check()?;
        let mut sources: Vec<UnifiedComponentSource> = self
            .entries
            .values()
            .filter(|entry| entry.frame.is_some())
            .map(|entry| UnifiedComponentSource {
                owner: entry.owner.clone(),
                identity: entry.identity.clone(),
                generation: entry.metadata.identity_block.generation,
                abi: entry.metadata.identity_block.abi,
                runtime: entry.metadata.identity_block.runtime,
                prepared: entry.prepared.clone(),
            })
            .collect();
        sources.sort_by(|left, right| {
            (&left.owner.provider.namespace, &left.owner.provider.name)
                .cmp(&(&right.owner.provider.namespace, &right.owner.provider.name))
        });
        Ok(sources)
    }

    /// Admitted client sources with a frame and HUD, ordered by provider.
    pub fn client_sources(&self) -> Result<Vec<UnifiedComponentClientSource>, UnifiedComponentConsumerError> {
        self.current_check()?;
        let mut sources: Vec<UnifiedComponentClientSource> = self
            .entries
            .values()
            .filter(|entry| entry.frame.is_some() && entry.hud_mode.is_some())
            .map(|entry| UnifiedComponentClientSource {
                owner: entry.owner.clone(),
                identity: entry.identity.clone(),
                generation: entry.metadata.identity_block.generation,
            })
            .collect();
        sources.sort_by(|left, right| {
            (&left.owner.provider.namespace, &left.owner.provider.name)
                .cmp(&(&right.owner.provider.namespace, &right.owner.provider.name))
        });
        Ok(sources)
    }

    fn live_entry(&self, provider: &ProviderId) -> Result<&ComponentEntry, UnifiedComponentConsumerError> {
        self.current_check()?;
        self.entries.get(provider).ok_or_else(|| {
            UnifiedComponentConsumerError::Consumer("Remote component activation is retired".to_string())
        })
    }

    /// Presentation context for one viewer.
    pub fn context_for(
        &self,
        provider: &ProviderId,
        viewer: &ActorId,
    ) -> Result<Option<UnifiedComponentContextView>, UnifiedComponentConsumerError> {
        let entry = self.live_entry(provider)?;
        let Some(frame) = entry.frame.as_ref() else {
            return Ok(None);
        };
        if self.host.viewer().as_ref() != Some(viewer) || frame.viewer != *viewer {
            return Ok(None);
        }
        Ok(Some(UnifiedComponentContextView {
            client_number: frame.client_number,
            game_state: entry.frame_game_state.clone(),
            game_state_revision: frame.game_state_revision,
            snapshot: frame.snapshot.clone(),
            weapon_presented: frame.weapon_presented,
            scene: entry.scene.clone(),
        }))
    }

    /// Mounts backing one activation.
    pub fn files_for(
        &mut self,
        provider: &ProviderId,
    ) -> Result<Option<UnifiedComponentFiles<'_>>, UnifiedComponentConsumerError> {
        self.current_check()?;
        let selection = match self.entries.get(provider) {
            Some(entry) => contract_selection(&entry.identity.selection),
            None => return Ok(None),
        };
        let writable = self.host.user_files().store(&selection)?;
        match self.entries.get(provider) {
            Some(entry) => Ok(Some(UnifiedComponentFiles {
                mounts: &entry.mounts,
                writable,
            })),
            None => Ok(None),
        }
    }

    /// Scene bindings for one activation.
    pub fn bindings_for(&self, provider: &ProviderId) -> Result<Vec<QvmSceneActor>, UnifiedComponentConsumerError> {
        let entry = self.live_entry(provider)?;
        Ok(entry
            .frame
            .as_ref()
            .map(|frame| frame.bindings.clone())
            .unwrap_or_default())
    }

    /// Resolve a bound actor by slot.
    pub fn actor_for(
        &self,
        provider: &ProviderId,
        slot: usize,
    ) -> Result<Option<ActorId>, UnifiedComponentConsumerError> {
        let entry = self.live_entry(provider)?;
        Ok(entry.frame.as_ref().and_then(|frame| {
            frame
                .bindings
                .iter()
                .find(|binding| binding.slot == slot)
                .map(|binding| binding.actor.clone())
        }))
    }

    /// Whether an actor binding is live for the current viewer.
    pub fn live_for(&self, provider: &ProviderId, actor: &ActorId) -> bool {
        if self.closed {
            return false;
        }
        let Some(entry) = self.entries.get(provider) else {
            return false;
        };
        self.host.viewer().as_ref() == entry.frame.as_ref().map(|frame| &frame.viewer).or(Some(actor))
            && entry
                .frame
                .as_ref()
                .is_some_and(|frame| frame.bindings.iter().any(|binding| binding.actor == *actor))
    }

    /// Client presentation frame for one actor.
    pub fn client_frame_for(
        &self,
        provider: &ProviderId,
        actor: &ActorId,
    ) -> Result<Option<UnifiedComponentClientFrame>, UnifiedComponentConsumerError> {
        let entry = self.live_entry(provider)?;
        if self.context_for(provider, actor)?.is_none() {
            return Ok(None);
        }
        Ok(entry.hud_mode.as_ref().map(|hud_mode| UnifiedComponentClientFrame {
            hud_mode: hud_mode.clone(),
        }))
    }

    /// Send a component command after checking the viewer lease.
    pub fn client_command(
        &self,
        provider: &ProviderId,
        viewer: &ActorId,
        args: &[String],
    ) -> Result<(), UnifiedComponentConsumerError> {
        let entry = self.live_entry(provider)?;
        if self.context_for(provider, viewer)?.is_none() {
            return Err(UnifiedComponentConsumerError::Consumer(
                "Remote component command viewer is retired".to_string(),
            ));
        }
        self.host
            .send_command(&entry.owner, entry.metadata.identity_block.generation, args)
    }

    /// Retire every entry, aggregating failures.
    pub fn close(&mut self) -> Result<(), UnifiedComponentConsumerError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let entries = std::mem::take(&mut self.entries);
        let mut failures = Vec::new();
        for entry in entries.values() {
            if let Err(error) = Self::retire(&mut *self.host, entry, false) {
                failures.push(error.to_string());
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(UnifiedComponentConsumerError::Consumer(format!(
                "Remote component retirement failed: {}",
                failures.join("; ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::MountPlanId;
    use qa_content::contract::ResolvedMountPlan as ContractMountPlan;
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_core::identity::IdentityOwner;
    use qa_guest::qvm::client_state::GameStateRecord;
    use qa_guest::qvm::game_data::AbiProfile;
    use qa_guest::qvm::mod_presentation::QvmSceneActor;
    use qa_guest::qvm::player_record::{qvm_player_state_bytes, read_source_qvm_player_state};

    use super::super::unified_components::{
        UnifiedComponentAbi as Abi, UnifiedComponentIdentity, UnifiedComponentRuntime,
    };
    use crate::persistence::mods::{ModIdentity, ModSelection};
    use qa_world::save::shared::ProviderRef;

    fn owner() -> PresentationOwner {
        PresentationOwner {
            provider: ProviderId::new("test", "mod"),
            generation: 3,
        }
    }

    fn identity() -> ModIdentity {
        ModIdentity {
            selection: ModSelection {
                product: "test".to_string(),
                id: "mod".to_string(),
            },
            source: ProviderRef {
                provider: "test:source".to_string(),
                content: "q3:classic:base:1".to_string(),
            },
            declaration_digest: "sha256:".to_string() + &"ab".repeat(32),
            modules: Vec::new(),
            providers: Vec::new(),
        }
    }

    fn game_state() -> GameStateRecord {
        GameStateRecord {
            string_offsets: Vec::new(),
            string_data: Vec::new(),
            data_count: 0,
        }
    }

    fn snapshot() -> UnifiedComponentSnapshot {
        let bytes = vec![0u8; qvm_player_state_bytes(AbiProfile::Modern)];
        UnifiedComponentSnapshot {
            server_time: 100,
            player_state: read_source_qvm_player_state(&bytes, AbiProfile::Modern).unwrap(),
        }
    }

    fn state() -> UnifiedComponentState {
        UnifiedComponentState {
            identity_block: UnifiedComponentIdentity {
                owner: owner(),
                identity: identity(),
                generation: 1,
                abi: Abi::Modern,
                runtime: UnifiedComponentRuntime::PlayerEvents,
            },
            game_state_revision: 4,
            game_state: Some(game_state()),
            command_base: 0,
            commands: Vec::new(),
        }
    }

    fn empty_mounts() -> MountedContent {
        let plan = ContractMountPlan {
            id: MountPlanId("mount-plan:test:1".to_string()),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        open_mount_plan(&plan, OpenMountOptions::default()).unwrap()
    }

    struct FakeHost {
        viewer: Option<ActorId>,
        prepared: Vec<PreparedComponentMod>,
        files: ModUserFiles,
        admitted: Vec<PresentationOwner>,
        retired: Vec<PresentationOwner>,
    }

    impl FakeHost {
        fn new(viewer: Option<ActorId>) -> Self {
            Self {
                viewer,
                prepared: vec![PreparedComponentMod {
                    identity: identity(),
                    runtime: UnifiedComponentRuntime::PlayerEvents,
                    abi: Abi::Modern,
                    source_provider: ProviderId::new("test", "mod"),
                    hud_mode: Some("status".to_string()),
                }],
                files: ModUserFiles::new("user"),
                admitted: Vec::new(),
                retired: Vec::new(),
            }
        }
    }

    impl UnifiedComponentHost for FakeHost {
        fn assert_current(&self) -> Result<(), UnifiedComponentConsumerError> {
            Ok(())
        }

        fn viewer(&self) -> Option<ActorId> {
            self.viewer.clone()
        }

        fn send_command(
            &self,
            _owner: &PresentationOwner,
            _generation: i64,
            _args: &[String],
        ) -> Result<(), UnifiedComponentConsumerError> {
            Ok(())
        }

        fn admit_replicated_owner(
            &mut self,
            owner: &PresentationOwner,
            _content: &str,
        ) -> Result<(), UnifiedComponentConsumerError> {
            self.admitted.push(owner.clone());
            Ok(())
        }

        fn retire_replicated_owner(&mut self, owner: &PresentationOwner) -> Result<(), UnifiedComponentConsumerError> {
            self.retired.push(owner.clone());
            Ok(())
        }

        fn prepared_mods(&self) -> Vec<PreparedComponentMod> {
            self.prepared.clone()
        }

        fn for_content(&self, _content: &str) -> Result<MountedContent, UnifiedComponentConsumerError> {
            Ok(empty_mounts())
        }

        fn user_files(&mut self) -> &mut ModUserFiles {
            &mut self.files
        }
    }

    fn actor() -> ActorId {
        IdentityOwner::create("test").unwrap().actor(1, 0)
    }

    fn frame(viewer: &ActorId) -> UnifiedComponentFrame {
        UnifiedComponentFrame {
            owner: owner(),
            generation: 1,
            abi: Abi::Modern,
            viewer: viewer.clone(),
            client_number: 0,
            game_state_revision: 4,
            snapshot: snapshot(),
            weapon_presented: false,
            bindings: vec![QvmSceneActor {
                actor: viewer.clone(),
                slot: 0,
                owned: false,
            }],
            scene: None,
        }
    }

    #[test]
    fn admit_accept_and_read() {
        let viewer = actor();
        let host = FakeHost::new(Some(viewer.clone()));
        let mut consumers = UnifiedComponentConsumers::new(Box::new(host));
        consumers
            .update(&UnifiedComponentUpdate {
                revision: 1,
                sources: vec![state()],
                native: Vec::new(),
            })
            .unwrap();
        assert!(consumers.sources().unwrap().is_empty());
        let frames = UnifiedComponentFrames {
            revision: 1,
            sources: vec![frame(&viewer)],
            native: None,
        };
        assert!(consumers.accept(&frames, &viewer).unwrap());
        assert_eq!(consumers.sources().unwrap().len(), 1);
        assert_eq!(consumers.client_sources().unwrap().len(), 1);
        let context = consumers
            .context_for(&ProviderId::new("test", "mod"), &viewer)
            .unwrap()
            .unwrap();
        assert_eq!(context.client_number, 0);
        assert_eq!(
            consumers.actor_for(&ProviderId::new("test", "mod"), 0).unwrap(),
            Some(viewer.clone())
        );
        assert!(consumers.live_for(&ProviderId::new("test", "mod"), &viewer));
        assert!(consumers
            .client_frame_for(&ProviderId::new("test", "mod"), &viewer)
            .unwrap()
            .is_some());
        consumers
            .client_command(&ProviderId::new("test", "mod"), &viewer, &["go".to_string()])
            .unwrap();
        let stale = UnifiedComponentFrames {
            revision: 0,
            sources: Vec::new(),
            native: None,
        };
        assert!(!consumers.accept(&stale, &viewer).unwrap());
        consumers.close().unwrap();
        assert!(consumers.sources().is_err());
    }

    #[test]
    fn rejects_gaps_and_mismatches() {
        let viewer = actor();
        let host = FakeHost::new(Some(viewer.clone()));
        let mut consumers = UnifiedComponentConsumers::new(Box::new(host));
        assert!(consumers
            .update(&UnifiedComponentUpdate {
                revision: 2,
                sources: Vec::new(),
                native: Vec::new(),
            })
            .is_err());
        let mut bad = state();
        bad.game_state = None;
        assert!(consumers
            .update(&UnifiedComponentUpdate {
                revision: 1,
                sources: vec![bad],
                native: Vec::new(),
            })
            .is_err());
        consumers
            .update(&UnifiedComponentUpdate {
                revision: 1,
                sources: vec![state()],
                native: Vec::new(),
            })
            .unwrap();
        let mut frame = frame(&viewer);
        frame.generation = 2;
        assert!(consumers
            .accept(
                &UnifiedComponentFrames {
                    revision: 1,
                    sources: vec![frame],
                    native: None,
                },
                &viewer
            )
            .is_err());
    }

    #[test]
    fn removal_closes_without_activation_event() {
        let viewer = actor();
        let host = FakeHost::new(Some(viewer));
        let mut consumers = UnifiedComponentConsumers::new(Box::new(host));
        consumers
            .update(&UnifiedComponentUpdate {
                revision: 1,
                sources: vec![state()],
                native: Vec::new(),
            })
            .unwrap();
        consumers
            .update(&UnifiedComponentUpdate {
                revision: 2,
                sources: Vec::new(),
                native: Vec::new(),
            })
            .unwrap();
        assert!(consumers.sources().unwrap().is_empty());
    }
}
