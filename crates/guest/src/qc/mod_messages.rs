//! QuakeC mod-message routing with signon replay.
//!
//! Ported from `src/compat/qc/mod-messages.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `QcMessageServices` mirrors the engine/client/body surface of
//! `ModHostServices` from `src/world/session/mods.ts`;
//! `QcMessageEntry`/`QcMessagePayload` mirror the routed messages from
//! `src/compat/qc/presentation-host.ts`; `QcLocalMessages` mirrors
//! `QuakeCLocalMessages` from
//! `src/app/bootstrap/simulation/quakec-local-messages.ts`;
//! `QcMessageSink` mirrors the `presentQuakeWorldMessage` /
//! `presentQuakeCLocalMessage` presentation from
//! `src/compat/qc/quakeworld-presentation.ts` and
//! `src/app/bootstrap/simulation/quakec-local-messages.ts`.
//!
//! Adaptation: message bytes are opaque to routing; the host supplies the
//! payload codec and the presentation sink. Client lifecycle arrives through
//! `client_admitted`/`client_retired` instead of a subscription closure.

use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use crate::error::GuestError;

/// Routable message payload.
pub trait QcMessagePayload: Clone {
    /// Whether this payload captures a camera target.
    fn is_set_view(&self) -> bool;
    /// Encode for checkpoints.
    fn encode(&self) -> Vec<u8>;
    /// Decode from a checkpoint.
    fn decode(bytes: &[u8]) -> Result<Self, GuestError>;
}

/// Routed message entry.
#[derive(Debug, Clone, PartialEq)]
pub struct QcMessageEntry<P> {
    /// Addressed actor (broadcast baseline when none).
    pub actor: Option<ActorId>,
    /// Payload.
    pub payload: P,
}

/// Multicast scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MulticastKind {
    /// Potentially visible set.
    Pvs,
    /// Potentially hearable set.
    Phs,
    /// All clients.
    All,
}

/// Multicast destination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MulticastScope {
    /// Origin.
    pub origin: Vec3,
    /// Scope.
    pub scope: MulticastKind,
}

/// Message destination.
#[derive(Debug, Clone, PartialEq)]
pub enum QcDestination {
    /// Signon buffer.
    Signon,
    /// Broadcast.
    Broadcast {
        /// Reliable delivery.
        reliable: bool,
    },
    /// One client.
    Client {
        /// Recipient.
        actor: ActorId,
    },
    /// Multicast scope.
    Multicast {
        /// Scope.
        scope: MulticastScope,
    },
}

/// Message API family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcMessageApi {
    /// QuakeWorld routing with signon replay.
    QuakeWorld,
    /// NetQuake local routing.
    NetQuake,
}

/// Host services for message routing.
pub trait QcMessageServices {
    /// API family.
    fn api(&self) -> QcMessageApi;
    /// Player actors.
    fn players(&self) -> Vec<ActorId>;
    /// Whether an actor is a destination client.
    fn is_client(&self, actor: &ActorId) -> bool;
    /// Whether an actor receives a multicast scope.
    fn receives_multicast(&self, actor: &ActorId, scope: &MulticastScope) -> Result<bool, GuestError>;
    /// Whether the server is loading.
    fn loading(&self) -> bool {
        false
    }
}

/// Presentation sink for routed messages.
pub trait QcMessageSink<P> {
    /// Present one entry to a target.
    fn present(&mut self, entry: &QcMessageEntry<P>, target: Option<&ActorId>, local: &mut QcLocalMessages<P>);
}

/// Local message state: broadcast baseline plus per-client streams.
#[derive(Debug)]
pub struct QcLocalMessages<P> {
    baseline: Vec<P>,
    clients: HashMap<ActorId, Vec<P>>,
    view_baseline: Option<ActorId>,
    view_clients: HashMap<ActorId, ActorId>,
}

impl<P> Default for QcLocalMessages<P> {
    fn default() -> Self {
        Self {
            baseline: Vec::new(),
            clients: HashMap::new(),
            view_baseline: None,
            view_clients: HashMap::new(),
        }
    }
}

impl<P: Clone> QcLocalMessages<P> {
    /// Empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit a client stream.
    pub fn admit(&mut self, actor: &ActorId) {
        self.clients.entry(actor.clone()).or_default();
    }

    /// Retire a client stream.
    pub fn retire(&mut self, actor: &ActorId) {
        self.clients.remove(actor);
        self.view_clients.remove(actor);
    }

    /// Receive messages into the baseline or one client stream.
    pub fn receive(&mut self, messages: &[P], target: Option<&ActorId>, view_target: Option<&ActorId>) {
        match target {
            None => self.baseline.extend(messages.iter().cloned()),
            Some(actor) => self
                .clients
                .entry(actor.clone())
                .or_default()
                .extend(messages.iter().cloned()),
        }
        if let Some(view) = view_target {
            match target {
                None => self.view_baseline = Some(view.clone()),
                Some(actor) => {
                    self.view_clients.insert(actor.clone(), view.clone());
                }
            }
        }
    }

    /// Baseline messages.
    #[must_use]
    pub fn baseline(&self) -> &[P] {
        &self.baseline
    }

    /// Client stream.
    #[must_use]
    pub fn client(&self, actor: &ActorId) -> Option<&[P]> {
        self.clients.get(actor).map(Vec::as_slice)
    }

    /// View target for a stream.
    #[must_use]
    pub fn view_target(&self, actor: Option<&ActorId>) -> Option<&ActorId> {
        match actor {
            None => self.view_baseline.as_ref(),
            Some(actor) => self.view_clients.get(actor),
        }
    }
}

/// Saved local state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcLocalCheckpoint {
    /// Baseline payloads.
    pub baseline: Vec<Vec<u8>>,
    /// Per-client payloads.
    pub clients: Vec<(SavedActorId, Vec<Vec<u8>>)>,
    /// Baseline view target.
    pub view_baseline: Option<SavedActorId>,
    /// Per-client view targets.
    pub view_clients: Vec<(SavedActorId, SavedActorId)>,
}

/// Saved signon entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcSavedEntry {
    /// Addressed actor.
    pub actor: Option<SavedActorId>,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

/// Saved QuakeWorld routing state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcQuakeWorldCheckpoint {
    /// Signon buffer.
    pub signon: Vec<QcSavedEntry>,
    /// Per-client replay cursors.
    pub admitted: Vec<(SavedActorId, usize)>,
}

/// Saved message routing state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcMessageCheckpoint {
    /// Local state.
    pub local: QcLocalCheckpoint,
    /// QuakeWorld state.
    pub quakeworld: Option<QcQuakeWorldCheckpoint>,
}

/// Mod-message router.
pub struct QcModMessages<S, K, P> {
    services: S,
    sink: K,
    local: QcLocalMessages<P>,
    signon: Vec<QcMessageEntry<P>>,
    admitted: HashMap<ActorId, usize>,
}

impl<S: QcMessageServices, K: QcMessageSink<P>, P: QcMessagePayload> QcModMessages<S, K, P> {
    /// Build over services and a presentation sink.
    pub fn new(services: S, sink: K) -> Self {
        Self {
            services,
            sink,
            local: QcLocalMessages::new(),
            signon: Vec::new(),
            admitted: HashMap::new(),
        }
    }

    /// Borrow the services.
    #[must_use]
    pub fn services(&self) -> &S {
        &self.services
    }

    /// Borrow the local state.
    #[must_use]
    pub fn client_state(&self) -> &QcLocalMessages<P> {
        &self.local
    }

    /// Admit a client, replaying buffered signon data.
    pub fn admit(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        self.local.admit(actor);
        let cursor = self.admitted.get(actor).copied().unwrap_or(0);
        for index in cursor..self.signon.len() {
            let entry = self.signon[index].clone();
            let local = &mut self.local;
            self.sink.present(&entry, Some(actor), local);
            self.local
                .receive(std::slice::from_ref(&entry.payload), Some(actor), None);
        }
        self.admitted.insert(actor.clone(), self.signon.len());
        Ok(())
    }

    /// Retire a client.
    pub fn retire_client(&mut self, actor: &ActorId) {
        self.local.retire(actor);
        self.admitted.remove(actor);
    }

    /// Observe a client admission (QuakeWorld only).
    pub fn client_admitted(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        if self.services.api() == QcMessageApi::QuakeWorld {
            self.admit(actor)?;
        }
        Ok(())
    }

    /// Observe a client departure (QuakeWorld only).
    pub fn client_retired(&mut self, actor: &ActorId) {
        if self.services.api() == QcMessageApi::QuakeWorld {
            self.retire_client(actor);
        }
    }

    /// Admit all players (QuakeWorld only).
    pub fn start(&mut self) -> Result<(), GuestError> {
        if self.services.api() != QcMessageApi::QuakeWorld {
            return Ok(());
        }
        let players = self.services.players();
        for actor in &players {
            self.admit(actor)?;
        }
        Ok(())
    }

    /// Route entries to a destination.
    pub fn route(
        &mut self,
        entries: Vec<QcMessageEntry<P>>,
        destination: &QcDestination,
        view_targets: Option<&HashMap<usize, ActorId>>,
    ) -> Result<(), GuestError> {
        match self.services.api() {
            QcMessageApi::QuakeWorld => self.route_quakeworld(entries, destination),
            QcMessageApi::NetQuake => self.route_netquake(entries, destination, view_targets),
        }
    }

    /// QuakeWorld routing.
    fn route_quakeworld(
        &mut self,
        entries: Vec<QcMessageEntry<P>>,
        destination: &QcDestination,
    ) -> Result<(), GuestError> {
        match destination {
            QcDestination::Signon => {
                self.signon.extend(entries);
                self.start()
            }
            QcDestination::Broadcast { .. } => {
                self.start()?;
                for entry in &entries {
                    let local = &mut self.local;
                    self.sink.present(entry, None, local);
                    self.local.receive(std::slice::from_ref(&entry.payload), None, None);
                }
                Ok(())
            }
            QcDestination::Client { actor } => {
                let recipients = vec![actor.clone()];
                self.route_recipients(&entries, &recipients)
            }
            QcDestination::Multicast { scope } => {
                let mut recipients = Vec::new();
                for actor in self.services.players() {
                    if self.services.receives_multicast(&actor, scope)? {
                        recipients.push(actor);
                    }
                }
                self.route_recipients(&entries, &recipients)
            }
        }
    }

    /// Route to explicit recipients, retiring non-clients.
    fn route_recipients(&mut self, entries: &[QcMessageEntry<P>], recipients: &[ActorId]) -> Result<(), GuestError> {
        for actor in recipients {
            if !self.services.is_client(actor) {
                self.retire_client(actor);
                continue;
            }
            self.admit(actor)?;
            for entry in entries {
                let local = &mut self.local;
                self.sink.present(entry, Some(actor), local);
                self.local
                    .receive(std::slice::from_ref(&entry.payload), Some(actor), None);
            }
        }
        Ok(())
    }

    /// NetQuake routing.
    fn route_netquake(
        &mut self,
        entries: Vec<QcMessageEntry<P>>,
        destination: &QcDestination,
        view_targets: Option<&HashMap<usize, ActorId>>,
    ) -> Result<(), GuestError> {
        if matches!(destination, QcDestination::Multicast { .. }) {
            return Err(GuestError::invalid("NetQuake cannot route a multicast message"));
        }
        let target = match destination {
            QcDestination::Client { actor } => Some(actor.clone()),
            _ => None,
        };
        let players = self.services.players();
        for actor in &players {
            self.local.admit(actor);
        }
        for (index, entry) in entries.iter().enumerate() {
            if entry.payload.is_set_view() && view_targets.and_then(|targets| targets.get(&index)).is_none() {
                return Err(GuestError::invalid("QC camera message has no captured source actor"));
            }
            let view = view_targets.and_then(|targets| targets.get(&index));
            self.local
                .receive(std::slice::from_ref(&entry.payload), target.as_ref(), view);
            let local = &mut self.local;
            self.sink.present(entry, target.as_ref(), local);
        }
        Ok(())
    }

    /// Checkpoint routing state.
    pub fn capture(&self) -> QcMessageCheckpoint {
        let mut clients: Vec<(SavedActorId, Vec<Vec<u8>>)> = self
            .local
            .clients
            .iter()
            .map(|(actor, messages)| {
                (
                    SavedActorId::from(actor),
                    messages.iter().map(QcMessagePayload::encode).collect(),
                )
            })
            .collect();
        clients.sort_by_key(|entry| (entry.0.slot, entry.0.generation));
        let mut view_clients: Vec<(SavedActorId, SavedActorId)> = self
            .local
            .view_clients
            .iter()
            .map(|(actor, target)| (SavedActorId::from(actor), SavedActorId::from(target)))
            .collect();
        view_clients.sort_by(|left, right| {
            (left.0.slot, left.0.generation, left.1.slot, left.1.generation).cmp(&(
                right.0.slot,
                right.0.generation,
                right.1.slot,
                right.1.generation,
            ))
        });
        let local = QcLocalCheckpoint {
            baseline: self.local.baseline.iter().map(QcMessagePayload::encode).collect(),
            clients,
            view_baseline: self.local.view_baseline.as_ref().map(SavedActorId::from),
            view_clients,
        };
        let quakeworld = (self.services.api() == QcMessageApi::QuakeWorld).then(|| {
            let mut admitted: Vec<(SavedActorId, usize)> = self
                .admitted
                .iter()
                .map(|(actor, cursor)| (SavedActorId::from(actor), *cursor))
                .collect();
            admitted.sort_by_key(|entry| (entry.0.slot, entry.0.generation));
            QcQuakeWorldCheckpoint {
                signon: self
                    .signon
                    .iter()
                    .map(|entry| QcSavedEntry {
                        actor: entry.actor.as_ref().map(SavedActorId::from),
                        payload: entry.payload.encode(),
                    })
                    .collect(),
                admitted,
            }
        });
        QcMessageCheckpoint { local, quakeworld }
    }

    /// Restore routing state.
    pub fn restore(
        &mut self,
        saved: &QcMessageCheckpoint,
        resolve: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<(), GuestError> {
        let mut baseline = Vec::with_capacity(saved.local.baseline.len());
        for bytes in &saved.local.baseline {
            baseline.push(P::decode(bytes)?);
        }
        let mut clients = HashMap::new();
        for (actor, messages) in &saved.local.clients {
            let live = resolve(actor)
                .ok_or_else(|| GuestError::BadSave("Saved message client has no live target".to_string()))?;
            let mut decoded = Vec::with_capacity(messages.len());
            for bytes in messages {
                decoded.push(P::decode(bytes)?);
            }
            clients.insert(live, decoded);
        }
        let mut view_baseline = None;
        if let Some(target) = saved.local.view_baseline.as_ref() {
            view_baseline = Some(
                resolve(target)
                    .ok_or_else(|| GuestError::BadSave("Saved message view has no live target".to_string()))?,
            );
        }
        let mut view_clients = HashMap::new();
        for (actor, target) in &saved.local.view_clients {
            let live = resolve(actor)
                .ok_or_else(|| GuestError::BadSave("Saved message view has no live target".to_string()))?;
            let live_target = resolve(target)
                .ok_or_else(|| GuestError::BadSave("Saved message view has no live target".to_string()))?;
            view_clients.insert(live, live_target);
        }
        self.local = QcLocalMessages {
            baseline,
            clients,
            view_baseline,
            view_clients,
        };
        match (&saved.quakeworld, self.services.api() == QcMessageApi::QuakeWorld) {
            (Some(state), true) => {
                let mut signon = Vec::with_capacity(state.signon.len());
                for entry in &state.signon {
                    let actor =
                        match entry.actor.as_ref() {
                            Some(saved) => Some(resolve(saved).ok_or_else(|| {
                                GuestError::BadSave("Saved signon entry has no live target".to_string())
                            })?),
                            None => None,
                        };
                    signon.push(QcMessageEntry {
                        actor,
                        payload: P::decode(&entry.payload)?,
                    });
                }
                let mut admitted = HashMap::new();
                let mut seen = HashSet::new();
                for (actor, cursor) in &state.admitted {
                    let live = resolve(actor)
                        .ok_or_else(|| GuestError::BadSave("Saved signon cursor has no live target".to_string()))?;
                    if *cursor > signon.len() || !seen.insert(live.clone()) {
                        return Err(GuestError::BadSave("Invalid component signon cursor".to_string()));
                    }
                    admitted.insert(live, *cursor);
                }
                self.signon = signon;
                self.admitted = admitted;
                Ok(())
            }
            (None, false) => {
                self.signon.clear();
                self.admitted.clear();
                Ok(())
            }
            _ => Err(GuestError::BadSave("Saved mod message services differ".to_string())),
        }
    }

    /// Close routing.
    pub fn close(&mut self) {
        self.signon.clear();
        self.admitted.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct FakePayload {
        set_view: bool,
        text: String,
    }

    impl QcMessagePayload for FakePayload {
        fn is_set_view(&self) -> bool {
            self.set_view
        }

        fn encode(&self) -> Vec<u8> {
            let mut bytes = vec![u8::from(self.set_view)];
            bytes.extend_from_slice(self.text.as_bytes());
            bytes
        }

        fn decode(bytes: &[u8]) -> Result<Self, GuestError> {
            let (flag, text) = bytes
                .split_first()
                .ok_or_else(|| GuestError::BadSave("Empty message payload".to_string()))?;
            Ok(Self {
                set_view: *flag != 0,
                text: String::from_utf8_lossy(text).into_owned(),
            })
        }
    }

    struct FakeServices {
        api: QcMessageApi,
        players: Vec<ActorId>,
        clients: HashSet<ActorId>,
        multicast: HashSet<ActorId>,
    }

    impl QcMessageServices for FakeServices {
        fn api(&self) -> QcMessageApi {
            self.api
        }

        fn players(&self) -> Vec<ActorId> {
            self.players.clone()
        }

        fn is_client(&self, actor: &ActorId) -> bool {
            self.clients.contains(actor)
        }

        fn receives_multicast(&self, actor: &ActorId, _scope: &MulticastScope) -> Result<bool, GuestError> {
            Ok(self.multicast.contains(actor))
        }
    }

    #[derive(Default)]
    struct FakeSink {
        presented: Vec<(Option<ActorId>, String)>,
    }

    impl QcMessageSink<FakePayload> for FakeSink {
        fn present(
            &mut self,
            entry: &QcMessageEntry<FakePayload>,
            target: Option<&ActorId>,
            _local: &mut QcLocalMessages<FakePayload>,
        ) {
            self.presented.push((target.cloned(), entry.payload.text.clone()));
        }
    }

    fn entry(text: &str) -> QcMessageEntry<FakePayload> {
        QcMessageEntry {
            actor: None,
            payload: FakePayload {
                set_view: false,
                text: text.to_string(),
            },
        }
    }

    #[test]
    fn quakeworld_signon_replays_to_late_joiners() {
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("messages").unwrap();
        let first = owner.actor(1, 1);
        let late = owner.actor(2, 1);
        let services = FakeServices {
            api: QcMessageApi::QuakeWorld,
            players: vec![first.clone()],
            clients: [first.clone(), late.clone()].into_iter().collect(),
            multicast: HashSet::new(),
        };
        let mut router = QcModMessages::new(services, FakeSink::default());
        router
            .route(vec![entry("hello")], &QcDestination::Signon, None)
            .unwrap();
        assert_eq!(router.sink.presented, vec![(Some(first.clone()), "hello".to_string())]);
        router.services.players.push(late.clone());
        router.client_admitted(&late).unwrap();
        assert!(router
            .sink
            .presented
            .contains(&(Some(late.clone()), "hello".to_string())));
        assert_eq!(router.client_state().client(&late).unwrap().len(), 1);
    }

    #[test]
    fn quakeworld_multicast_filters_and_retires() {
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("messages-mc").unwrap();
        let inside = owner.actor(1, 1);
        let outside = owner.actor(2, 1);
        let ghost = owner.actor(3, 1);
        let services = FakeServices {
            api: QcMessageApi::QuakeWorld,
            players: vec![inside.clone(), outside.clone(), ghost.clone()],
            clients: [inside.clone()].into_iter().collect(),
            multicast: [inside.clone(), ghost.clone()].into_iter().collect(),
        };
        let mut router = QcModMessages::new(services, FakeSink::default());
        let scope = MulticastScope {
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            scope: MulticastKind::Pvs,
        };
        router
            .route(vec![entry("bang")], &QcDestination::Multicast { scope }, None)
            .unwrap();
        assert!(router
            .sink
            .presented
            .contains(&(Some(inside.clone()), "bang".to_string())));
        assert!(!router
            .sink
            .presented
            .iter()
            .any(|(target, _)| target.as_ref() == Some(&outside)));
        assert!(!router
            .sink
            .presented
            .iter()
            .any(|(target, _)| target.as_ref() == Some(&ghost)));
        assert!(router.client_state().client(&ghost).is_none());
    }

    #[test]
    fn netquake_routes_and_requires_view_targets() {
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("messages-nq").unwrap();
        let player = owner.actor(1, 1);
        let services = FakeServices {
            api: QcMessageApi::NetQuake,
            players: vec![player.clone()],
            clients: [player.clone()].into_iter().collect(),
            multicast: HashSet::new(),
        };
        let mut router = QcModMessages::new(services, FakeSink::default());
        router
            .route(vec![entry("hi")], &QcDestination::Broadcast { reliable: false }, None)
            .unwrap();
        assert_eq!(router.client_state().baseline().len(), 1);
        let scope = MulticastScope {
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            scope: MulticastKind::All,
        };
        assert!(router
            .route(vec![entry("x")], &QcDestination::Multicast { scope }, None)
            .is_err());
        let camera = QcMessageEntry {
            actor: None,
            payload: FakePayload {
                set_view: true,
                text: "cam".to_string(),
            },
        };
        assert!(router
            .route(vec![camera.clone()], &QcDestination::Broadcast { reliable: true }, None)
            .is_err());
        let views = [(0, player.clone())].into_iter().collect();
        router
            .route(vec![camera], &QcDestination::Broadcast { reliable: true }, Some(&views))
            .unwrap();
        assert_eq!(router.client_state().view_target(None), Some(&player));
    }

    #[test]
    fn checkpoint_restore_round_trip() {
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("messages-save").unwrap();
        let player = owner.actor(1, 1);
        let services = FakeServices {
            api: QcMessageApi::QuakeWorld,
            players: vec![player.clone()],
            clients: [player.clone()].into_iter().collect(),
            multicast: HashSet::new(),
        };
        let mut router = QcModMessages::new(services, FakeSink::default());
        router
            .route(vec![entry("saved")], &QcDestination::Signon, None)
            .unwrap();
        let saved = router.capture();
        assert!(saved.quakeworld.is_some());
        let mut revived = QcModMessages::new(
            FakeServices {
                api: QcMessageApi::QuakeWorld,
                players: vec![player.clone()],
                clients: [player.clone()].into_iter().collect(),
                multicast: HashSet::new(),
            },
            FakeSink::default(),
        );
        revived
            .restore(&saved, &|saved| (saved.slot == 1).then(|| player.clone()))
            .unwrap();
        let again = revived.capture();
        assert_eq!(again, saved);
        let mut netquake = QcModMessages::new(
            FakeServices {
                api: QcMessageApi::NetQuake,
                players: Vec::new(),
                clients: HashSet::new(),
                multicast: HashSet::new(),
            },
            FakeSink::default(),
        );
        assert!(netquake.restore(&saved, &|_| None).is_err());
        revived.close();
        assert!(revived.capture().quakeworld.unwrap().signon.is_empty());
    }
}
