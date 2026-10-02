//! Client-side native component replica.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-native-consumer.ts`
//! (`UnifiedNativeConsumers`).
//!
//! Native client imports are authored by the server module; clients consume
//! only its public readout. The donor two-phase prepare/commit closures
//! become owned commit values; a commit made stale by an interleaved
//! `retire` fails closed instead of resurrecting the entry. Prepared
//! presentations and product editions arrive through [`UnifiedNativeHost`]
//! since those lanes are unported.

use std::collections::{BTreeMap, HashMap};

use qa_content::contract::{
    mod_instance_provider, ContractError, GameFamily, ModSelection as ContractModSelection, PresentationOwner,
};
use qa_core::identity::{ActorId, ProviderId};

use super::types::NativeCameraEdition;
use super::unified_components::{same_owner, UnifiedComponentFrames, UnifiedComponentUpdate};
use super::unified_native_components::{
    UnifiedNativeFrame, UnifiedNativeHudFrame, UnifiedNativeHudMode, UnifiedNativeHudState, UnifiedNativeProtocol,
    UnifiedNativeState,
};
use super::unified_types::UnifiedNativeCamera;
use crate::persistence::mods::{same_mod_identity, ModIdentity, ModSelection};

/// Native consumer failure.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedNativeConsumerError {
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Consumer failure.
    #[error("{0}")]
    Consumer(String),
}

/// Admitted HUD for one prepared native presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeHudAdmission {
    /// No HUD.
    None,
    /// Status replacement.
    Replace,
    /// Layout overlay.
    Overlay,
}

/// Locally prepared native client presentation.
#[derive(Debug, Clone)]
pub struct PreparedNativeClient {
    /// Admitted HUD.
    pub hud: NativeHudAdmission,
    /// Camera view admitted.
    pub view: bool,
}

/// Locally prepared native mod.
#[derive(Debug, Clone)]
pub struct PreparedNativeMod {
    /// Mod identity.
    pub identity: ModIdentity,
    /// Client presentation, when the mod presents one.
    pub client: Option<PreparedNativeClient>,
}

/// Content and event surface the native consumer needs.
pub trait UnifiedNativeHost {
    /// Fail when the owning presentation moved on.
    fn assert_current(&self) -> Result<(), UnifiedNativeConsumerError>;
    /// Viewing actor, if any.
    fn viewer(&self) -> Option<ActorId>;
    /// Admit a replicated owner.
    fn admit_replicated_owner(
        &mut self,
        owner: &PresentationOwner,
        content: &str,
    ) -> Result<(), UnifiedNativeConsumerError>;
    /// Retire a replicated owner.
    fn retire_replicated_owner(&mut self, owner: &PresentationOwner) -> Result<(), UnifiedNativeConsumerError>;
    /// Locally prepared native presentations.
    fn prepared_mods(&self) -> Vec<PreparedNativeMod>;
    /// Product family and edition for content.
    fn product_edition(&self, content: &str) -> Result<(GameFamily, String), UnifiedNativeConsumerError>;
}

/// Q2 source edition backing one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSourceEdition {
    /// Classic.
    Classic,
    /// Rerelease.
    Rerelease,
}

/// Merged native HUD readout (donor `NativeFrame['hud']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativeClientHud {
    /// HUD mode.
    pub mode: UnifiedNativeHudMode,
    /// Reliable state with committed configstrings.
    pub state: UnifiedNativeHudState,
    /// Volatile frame.
    pub frame: UnifiedNativeHudFrame,
}

/// Native client readout (donor `NativeFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativeClientFrame {
    /// HUD readout.
    pub hud: Option<UnifiedNativeClientHud>,
    /// Camera view.
    pub view: Option<super::types::NativeModCameraView>,
}

/// Admitted native source (donor `ActiveModClientPresentation` handle).
#[derive(Debug, Clone)]
pub struct UnifiedNativeSource {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Component identity.
    pub identity: ModIdentity,
    /// Activation generation.
    pub generation: i64,
}

/// Native entry state carried by commits.
#[derive(Debug, Clone)]
pub struct UnifiedNativeEntry {
    metadata: UnifiedNativeState,
    configs: Option<BTreeMap<i64, String>>,
    frame: Option<NativeFrameValue>,
    edition: NativeSourceEdition,
    view: bool,
}

#[derive(Debug, Clone)]
struct NativeFrameValue {
    viewer: ActorId,
    value: UnifiedNativeClientFrame,
}

/// Pending reliable application: provider, metadata, committed configstrings.
type PendingNativeUpdate = (ProviderId, UnifiedNativeState, Option<BTreeMap<i64, String>>);

/// Prepared reliable commit.
#[derive(Debug, Clone)]
pub struct NativeUpdateCommit {
    revision: i64,
    next: HashMap<ProviderId, UnifiedNativeEntry>,
    pending: Vec<PendingNativeUpdate>,
}

/// Prepared frame commit.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum NativeFrameCommit {
    /// Clear legacy entries.
    LegacyClear,
    /// Install one legacy entry.
    LegacySet {
        /// Entry to install.
        entry: UnifiedNativeEntry,
        /// Viewing actor.
        viewer: ActorId,
        /// Readout value.
        value: UnifiedNativeClientFrame,
    },
    /// Apply volatile frames.
    Frames {
        /// Pending frame applications.
        pending: Vec<(ProviderId, ActorId, UnifiedNativeClientFrame)>,
    },
}

/// Native client imports are authored by the server module; clients consume only its public readout.
pub struct UnifiedNativeConsumers {
    host: Box<dyn UnifiedNativeHost>,
    entries: HashMap<ProviderId, UnifiedNativeEntry>,
    revision: i64,
    legacy: bool,
    closed: bool,
}

fn contract_selection(selection: &ModSelection) -> ContractModSelection {
    ContractModSelection {
        product: selection.product.clone(),
        id: selection.id.clone(),
    }
}

fn admission_mode(admission: &PreparedNativeClient) -> Option<UnifiedNativeHudMode> {
    match admission.hud {
        NativeHudAdmission::None => None,
        NativeHudAdmission::Replace => Some(UnifiedNativeHudMode::ReplaceStatus),
        NativeHudAdmission::Overlay => Some(UnifiedNativeHudMode::LayoutOverlay),
    }
}

impl UnifiedNativeConsumers {
    /// Empty collection over a host.
    pub fn new(host: Box<dyn UnifiedNativeHost>) -> Self {
        Self {
            host,
            entries: HashMap::new(),
            revision: 0,
            legacy: false,
            closed: false,
        }
    }

    fn current_check(&self) -> Result<(), UnifiedNativeConsumerError> {
        self.host.assert_current()?;
        if self.closed {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native component collection is retired".to_string(),
            ));
        }
        Ok(())
    }

    fn live_entry(&self, provider: &ProviderId) -> Result<&UnifiedNativeEntry, UnifiedNativeConsumerError> {
        self.current_check()?;
        self.entries
            .get(provider)
            .ok_or_else(|| UnifiedNativeConsumerError::Consumer("Remote native component is retired".to_string()))
    }

    fn create(
        &self,
        metadata: &UnifiedNativeState,
        camera_only: bool,
    ) -> Result<UnifiedNativeEntry, UnifiedNativeConsumerError> {
        let prepared = self
            .host
            .prepared_mods()
            .into_iter()
            .find(|candidate| same_mod_identity(&candidate.identity, &metadata.identity));
        let admission = prepared.as_ref().and_then(|prepared| prepared.client.as_ref());
        let Some(admission) = admission else {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native presentation differs from its locally qualified component".to_string(),
            ));
        };
        let provider = mod_instance_provider(&contract_selection(&metadata.identity.selection))?;
        if provider != metadata.owner.provider {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native presentation differs from its locally qualified component".to_string(),
            ));
        }
        let mode = admission_mode(admission);
        if !camera_only && metadata.hud.as_ref().map(|hud| hud.mode) != mode {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native HUD differs from its admitted mode".to_string(),
            ));
        }
        let (family, edition) = self.host.product_edition(&metadata.identity.source.content)?;
        if family != GameFamily::Q2 || (edition != "classic" && edition != "rerelease") {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native presentation has no Q2 source edition".to_string(),
            ));
        }
        Ok(UnifiedNativeEntry {
            metadata: metadata.clone(),
            configs: None,
            frame: None,
            edition: if edition == "classic" {
                NativeSourceEdition::Classic
            } else {
                NativeSourceEdition::Rerelease
            },
            view: admission.view,
        })
    }

    fn read_frame(
        entry: &UnifiedNativeEntry,
        frame: &UnifiedNativeFrame,
        viewer: &ActorId,
    ) -> Result<UnifiedNativeClientFrame, UnifiedNativeConsumerError> {
        if !same_owner(&entry.metadata.owner, &frame.owner)
            || entry.metadata.generation != frame.generation
            || frame.viewer != *viewer
        {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native frame differs from its source or recipient admission".to_string(),
            ));
        }
        let edition_matches = frame.view.as_ref().is_none_or(|view| {
            matches!(
                (&view.native.edition, entry.edition),
                (NativeCameraEdition::Classic, NativeSourceEdition::Classic)
                    | (NativeCameraEdition::Rerelease, NativeSourceEdition::Rerelease)
            )
        });
        if frame.view.is_some() != entry.view || !edition_matches {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native camera differs from its admitted source".to_string(),
            ));
        }
        let hud = entry.metadata.hud.as_ref();
        if hud.is_none() != frame.hud.is_none() {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native frame differs from its admitted HUD".to_string(),
            ));
        }
        let (Some(hud), Some(frame_hud)) = (hud, frame.hud.as_ref()) else {
            return Ok(UnifiedNativeClientFrame {
                hud: None,
                view: frame.view.clone(),
            });
        };
        let expected = if entry.edition == NativeSourceEdition::Classic {
            32
        } else {
            64
        };
        let Some(configs) = entry.configs.as_ref() else {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native HUD differs from its source ABI".to_string(),
            ));
        };
        if frame_hud.stats.len() != expected {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native HUD differs from its source ABI".to_string(),
            ));
        }
        let mut state = hud.frame.clone();
        state.configstrings = Some(configs.clone());
        Ok(UnifiedNativeClientFrame {
            hud: Some(UnifiedNativeClientHud {
                mode: hud.mode,
                state,
                frame: frame_hud.clone(),
            }),
            view: frame.view.clone(),
        })
    }

    /// Validate a reliable update and prepare its commit.
    pub fn prepare_update(
        &mut self,
        update: &UnifiedComponentUpdate,
    ) -> Result<NativeUpdateCommit, UnifiedNativeConsumerError> {
        self.current_check()?;
        if update.revision != self.revision + 1 {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native reliable revision is not consecutive".to_string(),
            ));
        }
        let mut next: HashMap<ProviderId, UnifiedNativeEntry> = HashMap::new();
        let mut pending = Vec::new();
        for metadata in &update.native {
            let previous = self.entries.get(&metadata.owner.provider);
            let same = previous.is_some_and(|previous| {
                same_owner(&previous.metadata.owner, &metadata.owner)
                    && previous.metadata.generation == metadata.generation
            });
            let entry = match previous {
                Some(previous) if same => previous.clone(),
                _ => self.create(metadata, false)?,
            };
            if !same_mod_identity(&entry.metadata.identity, &metadata.identity)
                || entry.metadata.hud.as_ref().map(|hud| hud.mode) != metadata.hud.as_ref().map(|hud| hud.mode)
            {
                return Err(UnifiedNativeConsumerError::Consumer(
                    "Remote native activation changed its admitted identity or HUD".to_string(),
                ));
            }
            let configs = match metadata.hud.as_ref() {
                None => None,
                Some(hud) => hud.frame.configstrings.clone().or_else(|| entry.configs.clone()),
            };
            if let Some(hud) = metadata.hud.as_ref() {
                let protocol_matches = matches!(
                    (&hud.frame.protocol, entry.edition),
                    (UnifiedNativeProtocol::Classic, NativeSourceEdition::Classic)
                        | (UnifiedNativeProtocol::Rerelease, NativeSourceEdition::Rerelease)
                );
                if configs.is_none() || !protocol_matches {
                    return Err(UnifiedNativeConsumerError::Consumer(
                        "Remote native HUD has no source configstrings or differs from its source edition".to_string(),
                    ));
                }
            }
            next.insert(metadata.owner.provider.clone(), entry);
            pending.push((metadata.owner.provider.clone(), metadata.clone(), configs));
        }
        Ok(NativeUpdateCommit {
            revision: update.revision,
            next,
            pending,
        })
    }

    /// Apply a prepared reliable commit.
    pub fn commit_update(&mut self, commit: NativeUpdateCommit) -> Result<(), UnifiedNativeConsumerError> {
        self.current_check()?;
        for (provider, entry) in std::mem::take(&mut self.entries) {
            let kept = commit.next.get(&provider);
            if kept.is_none_or(|kept| !same_owner(&kept.metadata.owner, &entry.metadata.owner)) {
                self.host.retire_replicated_owner(&entry.metadata.owner)?;
            }
        }
        self.entries = commit.next;
        self.revision = commit.revision;
        self.legacy = false;
        for (provider, metadata, configs) in commit.pending {
            let entry = self.entries.get_mut(&provider).ok_or_else(|| {
                UnifiedNativeConsumerError::Consumer("Remote native component is retired".to_string())
            })?;
            entry.metadata = metadata.clone();
            entry.configs = configs;
            self.host
                .admit_replicated_owner(&metadata.owner, &metadata.identity.source.content)?;
        }
        Ok(())
    }

    /// Validate volatile frames and prepare their commit; stale revisions report `None`.
    pub fn prepare(
        &mut self,
        frames: &UnifiedComponentFrames,
        viewer: &ActorId,
        legacy: Option<&UnifiedNativeCamera>,
    ) -> Result<Option<NativeFrameCommit>, UnifiedNativeConsumerError> {
        self.current_check()?;
        if frames.revision < self.revision {
            return Ok(None);
        }
        if frames.revision != self.revision {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native frame lacks reliable admission".to_string(),
            ));
        }
        if let Some(legacy) = legacy {
            if frames.native.is_none() {
                let generation = i64::try_from(legacy.generation).map_err(|_| {
                    UnifiedNativeConsumerError::Consumer("Remote native generation exceeds its range".to_string())
                })?;
                let metadata = UnifiedNativeState {
                    owner: legacy.owner.clone(),
                    identity: legacy.identity.clone(),
                    generation,
                    hud: None,
                };
                let previous = self.entries.get(&metadata.owner.provider);
                let same = previous.is_some_and(|previous| {
                    same_owner(&previous.metadata.owner, &metadata.owner)
                        && previous.metadata.generation == metadata.generation
                });
                let entry = match previous {
                    Some(previous) if same => previous.clone(),
                    _ => self.create(&metadata, true)?,
                };
                if !same_mod_identity(&entry.metadata.identity, &metadata.identity) {
                    return Err(UnifiedNativeConsumerError::Consumer(
                        "Remote native activation changed identity".to_string(),
                    ));
                }
                let frame = UnifiedNativeFrame {
                    owner: legacy.owner.clone(),
                    generation,
                    viewer: viewer.clone(),
                    view: Some(legacy.view.clone()),
                    hud: None,
                };
                let value = Self::read_frame(&entry, &frame, viewer)?;
                return Ok(Some(NativeFrameCommit::LegacySet {
                    entry,
                    viewer: viewer.clone(),
                    value,
                }));
            }
        }
        if self.legacy && frames.native.is_none() {
            return Ok(Some(NativeFrameCommit::LegacyClear));
        }
        let native = frames.native.as_deref().unwrap_or(&[]);
        if native.len() != self.entries.len() {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native frame differs from admitted owners".to_string(),
            ));
        }
        let mut pending = Vec::with_capacity(native.len());
        for frame in native {
            let entry = self.entries.get(&frame.owner.provider).ok_or_else(|| {
                UnifiedNativeConsumerError::Consumer("Remote native frame has an unadmitted owner".to_string())
            })?;
            pending.push((
                frame.owner.provider.clone(),
                frame.viewer.clone(),
                Self::read_frame(entry, frame, viewer)?,
            ));
        }
        if pending.iter().filter(|(_, _, value)| value.view.is_some()).count() > 1
            || pending
                .iter()
                .filter(|(_, _, value)| {
                    value
                        .hud
                        .as_ref()
                        .is_some_and(|hud| hud.mode == UnifiedNativeHudMode::ReplaceStatus)
                })
                .count()
                > 1
        {
            return Err(UnifiedNativeConsumerError::Consumer(
                "Remote native presentation has conflicting exclusive owners".to_string(),
            ));
        }
        Ok(Some(NativeFrameCommit::Frames { pending }))
    }

    /// Apply a prepared frame commit.
    pub fn commit_frames(&mut self, commit: NativeFrameCommit) {
        match commit {
            NativeFrameCommit::LegacyClear => {
                self.entries.clear();
                self.legacy = false;
            }
            NativeFrameCommit::LegacySet {
                mut entry,
                viewer,
                value,
            } => {
                let provider = entry.metadata.owner.provider.clone();
                entry.frame = Some(NativeFrameValue { viewer, value });
                self.entries = HashMap::from([(provider, entry)]);
                self.legacy = true;
            }
            NativeFrameCommit::Frames { pending } => {
                for (provider, viewer, value) in pending {
                    if let Some(entry) = self.entries.get_mut(&provider) {
                        entry.frame = Some(NativeFrameValue { viewer, value });
                    }
                }
            }
        }
    }

    /// Drop one owner without events.
    pub fn retire(&mut self, owner: &PresentationOwner) {
        if self
            .entries
            .get(&owner.provider)
            .is_some_and(|entry| same_owner(&entry.metadata.owner, owner))
        {
            self.entries.remove(&owner.provider);
        }
    }

    /// Admitted sources with a frame.
    pub fn sources(&self) -> Result<Vec<UnifiedNativeSource>, UnifiedNativeConsumerError> {
        self.current_check()?;
        let mut sources: Vec<UnifiedNativeSource> = self
            .entries
            .values()
            .filter(|entry| entry.frame.is_some())
            .map(|entry| UnifiedNativeSource {
                owner: entry.metadata.owner.clone(),
                identity: entry.metadata.identity.clone(),
                generation: entry.metadata.generation,
            })
            .collect();
        sources.sort_by(|left, right| {
            (&left.owner.provider.namespace, &left.owner.provider.name)
                .cmp(&(&right.owner.provider.namespace, &right.owner.provider.name))
        });
        Ok(sources)
    }

    /// Client readout for one actor.
    pub fn frame_for(
        &self,
        provider: &ProviderId,
        actor: &ActorId,
    ) -> Result<Option<UnifiedNativeClientFrame>, UnifiedNativeConsumerError> {
        let entry = self.live_entry(provider)?;
        if self.host.viewer().as_ref() == Some(actor)
            && entry.frame.as_ref().is_some_and(|frame| frame.viewer == *actor)
        {
            Ok(entry.frame.as_ref().map(|frame| frame.value.clone()))
        } else {
            Ok(None)
        }
    }

    /// Close the collection.
    pub fn close(&mut self) {
        self.closed = true;
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Vec3;

    use super::super::types::{NativeCameraBlock, NativeModCameraView, PlayerView};
    use super::super::unified_native_components::{
        UnifiedNativeHudBlock, UnifiedNativeHudFrame, UnifiedNativeHudState,
    };
    use crate::persistence::mods::{ModIdentity, ModSelection};
    use qa_world::save::shared::ProviderRef;

    fn identity() -> ModIdentity {
        ModIdentity {
            selection: ModSelection {
                product: "test".to_string(),
                id: "mod".to_string(),
            },
            source: ProviderRef {
                provider: "test:source".to_string(),
                content: "q2:classic:base:1".to_string(),
            },
            declaration_digest: "sha256:".to_string() + &"ab".repeat(32),
            modules: Vec::new(),
            providers: Vec::new(),
        }
    }

    fn owner() -> PresentationOwner {
        PresentationOwner {
            provider: mod_instance_provider(&contract_selection(&identity().selection)).unwrap(),
            generation: 3,
        }
    }

    fn hud_state() -> UnifiedNativeHudState {
        UnifiedNativeHudState {
            protocol: UnifiedNativeProtocol::Classic,
            configstrings: Some(BTreeMap::from([(0, "cs".to_string())])),
            layout: "layout".to_string(),
            inventory: vec![1, 2],
            player_number: 0,
        }
    }

    fn state() -> UnifiedNativeState {
        UnifiedNativeState {
            owner: owner(),
            identity: identity(),
            generation: 1,
            hud: Some(UnifiedNativeHudBlock {
                mode: UnifiedNativeHudMode::ReplaceStatus,
                frame: hud_state(),
            }),
        }
    }

    fn camera_view() -> NativeModCameraView {
        NativeModCameraView {
            view: PlayerView {
                origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                view_height: 22.0,
                blend: None,
                damage_blend: None,
                kick_angles: None,
                field_of_view: None,
                client_view_offset_delta: None,
                foreign_character_death: None,
                pitch_drift: None,
            },
            native: NativeCameraBlock {
                edition: NativeCameraEdition::Classic,
                movement_origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                render_flags: 0,
                position_prediction: false,
                angular_prediction: false,
                weapon_visible: true,
            },
        }
    }

    fn hud_frame() -> UnifiedNativeHudFrame {
        UnifiedNativeHudFrame {
            stats: vec![0; 32],
            server_frame: 7,
            time_milliseconds: 100.0,
            frame_time_milliseconds: None,
        }
    }

    fn actor() -> ActorId {
        IdentityOwner::create("test").unwrap().actor(1, 0)
    }

    struct FakeHost {
        viewer: Option<ActorId>,
        prepared: Vec<PreparedNativeMod>,
        admitted: Vec<PresentationOwner>,
        retired: Vec<PresentationOwner>,
    }

    impl FakeHost {
        fn new(viewer: Option<ActorId>) -> Self {
            Self {
                viewer,
                prepared: vec![PreparedNativeMod {
                    identity: identity(),
                    client: Some(PreparedNativeClient {
                        hud: NativeHudAdmission::Replace,
                        view: true,
                    }),
                }],
                admitted: Vec::new(),
                retired: Vec::new(),
            }
        }
    }

    impl UnifiedNativeHost for FakeHost {
        fn assert_current(&self) -> Result<(), UnifiedNativeConsumerError> {
            Ok(())
        }

        fn viewer(&self) -> Option<ActorId> {
            self.viewer.clone()
        }

        fn admit_replicated_owner(
            &mut self,
            owner: &PresentationOwner,
            _content: &str,
        ) -> Result<(), UnifiedNativeConsumerError> {
            self.admitted.push(owner.clone());
            Ok(())
        }

        fn retire_replicated_owner(&mut self, owner: &PresentationOwner) -> Result<(), UnifiedNativeConsumerError> {
            self.retired.push(owner.clone());
            Ok(())
        }

        fn prepared_mods(&self) -> Vec<PreparedNativeMod> {
            self.prepared.clone()
        }

        fn product_edition(&self, _content: &str) -> Result<(GameFamily, String), UnifiedNativeConsumerError> {
            Ok((GameFamily::Q2, "classic".to_string()))
        }
    }

    fn commit_update(consumers: &mut UnifiedNativeConsumers, revision: i64, native: Vec<UnifiedNativeState>) {
        let commit = consumers
            .prepare_update(&UnifiedComponentUpdate {
                revision,
                sources: Vec::new(),
                native,
            })
            .unwrap();
        consumers.commit_update(commit).unwrap();
    }

    #[test]
    fn admit_frame_and_read() {
        let viewer = actor();
        let mut consumers = UnifiedNativeConsumers::new(Box::new(FakeHost::new(Some(viewer.clone()))));
        commit_update(&mut consumers, 1, vec![state()]);
        let frame = UnifiedNativeFrame {
            owner: owner(),
            generation: 1,
            viewer: viewer.clone(),
            view: Some(camera_view()),
            hud: Some(hud_frame()),
        };
        let frames = UnifiedComponentFrames {
            revision: 1,
            sources: Vec::new(),
            native: Some(vec![frame]),
        };
        let commit = consumers.prepare(&frames, &viewer, None).unwrap().unwrap();
        consumers.commit_frames(commit);
        assert_eq!(consumers.sources().unwrap().len(), 1);
        let readout = consumers.frame_for(&owner().provider, &viewer).unwrap().unwrap();
        assert!(readout.view.is_some());
        assert_eq!(readout.hud.as_ref().unwrap().mode, UnifiedNativeHudMode::ReplaceStatus);
        assert_eq!(
            readout
                .hud
                .as_ref()
                .unwrap()
                .state
                .configstrings
                .as_ref()
                .unwrap()
                .len(),
            1
        );
        let stale = UnifiedComponentFrames {
            revision: 0,
            sources: Vec::new(),
            native: None,
        };
        assert!(consumers.prepare(&stale, &viewer, None).unwrap().is_none());
    }

    #[test]
    fn rejects_wrong_edition_and_abi() {
        let viewer = actor();
        let mut consumers = UnifiedNativeConsumers::new(Box::new(FakeHost::new(Some(viewer.clone()))));
        let mut bad = state();
        bad.hud.as_mut().unwrap().frame.protocol = UnifiedNativeProtocol::Rerelease;
        assert!(consumers
            .prepare_update(&UnifiedComponentUpdate {
                revision: 1,
                sources: Vec::new(),
                native: vec![bad],
            })
            .is_err());
        commit_update(&mut consumers, 1, vec![state()]);
        let mut frame = UnifiedNativeFrame {
            owner: owner(),
            generation: 1,
            viewer: viewer.clone(),
            view: Some(camera_view()),
            hud: Some(hud_frame()),
        };
        frame.hud.as_mut().unwrap().stats = vec![0; 64];
        assert!(consumers
            .prepare(
                &UnifiedComponentFrames {
                    revision: 1,
                    sources: Vec::new(),
                    native: Some(vec![frame]),
                },
                &viewer,
                None,
            )
            .is_err());
    }

    #[test]
    fn legacy_camera_round_trips() {
        let viewer = actor();
        let mut consumers = UnifiedNativeConsumers::new(Box::new(FakeHost::new(Some(viewer.clone()))));
        let camera = UnifiedNativeCamera {
            owner: owner(),
            identity: identity(),
            generation: 2,
            view: camera_view(),
        };
        let frames = UnifiedComponentFrames {
            revision: 0,
            sources: Vec::new(),
            native: None,
        };
        let commit = consumers.prepare(&frames, &viewer, Some(&camera)).unwrap().unwrap();
        consumers.commit_frames(commit);
        assert_eq!(consumers.sources().unwrap().len(), 1);
        let clearing = UnifiedComponentFrames {
            revision: 0,
            sources: Vec::new(),
            native: None,
        };
        let commit = consumers.prepare(&clearing, &viewer, None).unwrap().unwrap();
        assert!(matches!(commit, NativeFrameCommit::LegacyClear));
        consumers.commit_frames(commit);
        assert!(consumers.sources().unwrap().is_empty());
    }

    #[test]
    fn retire_drops_silently() {
        let viewer = actor();
        let mut consumers = UnifiedNativeConsumers::new(Box::new(FakeHost::new(Some(viewer.clone()))));
        commit_update(&mut consumers, 1, vec![state()]);
        consumers.retire(&owner());
        assert!(consumers.sources().unwrap().is_empty());
        consumers.close();
        assert!(consumers.sources().is_err());
    }
}
