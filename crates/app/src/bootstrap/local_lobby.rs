//! Retained local lobby membership across launched worlds.
//!
//! Donor provenance: `src/app/bootstrap/local-lobby.ts`
//! (`ApplicationLocalLobby`).
//!
//! Sync port over the existing [`qa_net::services::online::LocalLobbyService`]
//! with no behavioral changes to the state machine: membership tracking,
//! generation guards, host/leave/complete transitions, and every error string
//! match the donor. Two structural adaptations apply. First, the donor chains
//! operations through a promise tail for serialization; sync execution is
//! inherently serialized, so the tail is dropped and `close()` leaves
//! directly. Second, the donor shares one service across instances by
//! reference; the sync port owns its service, so cross-instance races (a
//! lobby vanishing under `poll()`, cleanup failures aggregating into
//! `HostCleanup`) are unreachable here — the aggregate branch is still
//! implemented for the owned failure shapes that can reach it.

use qa_net::common::endpoint::NetworkAddress;
use qa_net::common::session::{CompositionIdentity, WireSelection};
use qa_net::services::online::{
    Account, LocalLobbyService, Lobby, LobbyId, LobbyPhase, OnlineError,
};
use thiserror::Error;

/// Failure of a local-lobby operation.
#[derive(Debug, Error)]
pub enum LocalLobbyError {
    /// The session is closed.
    #[error("Local lobby session is closed")]
    Closed,
    /// There is no current membership.
    #[error("No current local lobby membership")]
    NoMembership,
    /// A lobby is already joined.
    #[error("Leave the current lobby before hosting another")]
    AlreadyHosting,
    /// A lobby is already joined.
    #[error("Leave the current lobby before joining another")]
    AlreadyJoined,
    /// Only the host can start.
    #[error("Only the local lobby host can start a match")]
    NotHostStart,
    /// Only the host can complete.
    #[error("Only the local lobby host can complete the match")]
    NotHostComplete,
    /// Publication failed and cleanup also failed (donor `AggregateError`).
    #[error("Lobby publication and host cleanup failed: {0}")]
    HostCleanup(String),
    /// A world transition failed.
    #[error("lobby transition failed: {0}")]
    Transition(String),
    /// The lobby service rejected the operation.
    #[error(transparent)]
    Service(#[from] OnlineError),
}

/// Bound host endpoint and wire (donor `transitions.host` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundLobby {
    /// Bound server endpoint.
    pub endpoint: NetworkAddress,
    /// Bound wire selection.
    pub wire: WireSelection,
}

/// World transitions for lobby lifecycle events (donor `LocalLobbyTransitions`).
///
/// `host` receives a `starting` lobby; `join` receives a `playing` lobby.
pub trait LocalLobbyTransitions {
    /// Bind a host endpoint and wire for a starting lobby.
    fn host(&mut self, lobby: &Lobby) -> Result<BoundLobby, String>;
    /// Join a playing lobby's world.
    fn join(&mut self, lobby: &Lobby) -> Result<(), String>;
    /// Leave a lobby's world.
    fn leave(&mut self, lobby: &Lobby) -> Result<(), String>;
    /// Observe a completed match.
    fn completed(&mut self, lobby: &Lobby) -> Result<(), String>;
}

/// Lobby creation selection (donor `LocalLobbySelection`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalLobbySelection {
    /// Prepared composition.
    pub composition: CompositionIdentity,
}

/// Retained local service membership outliving each launched world.
pub struct ApplicationLocalLobby<T> {
    service: LocalLobbyService,
    /// Local account.
    pub account: Account,
    transitions: T,
    membership: Option<LobbyId>,
    last_lobby: Option<Lobby>,
    launched_generation: u64,
    completed_generation: u64,
    closed: bool,
}

impl<T: LocalLobbyTransitions> ApplicationLocalLobby<T> {
    /// Create a session over a service, account, and transitions.
    pub fn new(service: LocalLobbyService, account: Account, transitions: T) -> Self {
        Self {
            service,
            account,
            transitions,
            membership: None,
            last_lobby: None,
            launched_generation: 0,
            completed_generation: 0,
            closed: false,
        }
    }

    /// List all lobbies.
    #[must_use]
    pub fn list(&self) -> Vec<Lobby> {
        self.service.list()
    }

    /// Current membership lobby, if any.
    #[must_use]
    pub fn current(&self) -> Option<Lobby> {
        self.service
            .list()
            .into_iter()
            .find(|lobby| Some(&lobby.id) == self.membership.as_ref())
    }

    fn ensure_open(&self) -> Result<(), LocalLobbyError> {
        if self.closed {
            return Err(LocalLobbyError::Closed);
        }
        Ok(())
    }

    fn require(&self) -> Result<Lobby, LocalLobbyError> {
        let lobby = self.current().ok_or(LocalLobbyError::NoMembership)?;
        if !lobby
            .members
            .iter()
            .any(|member| member.account.id == self.account.id)
        {
            return Err(LocalLobbyError::NoMembership);
        }
        Ok(lobby)
    }

    /// Host a new lobby.
    pub fn host(
        &mut self,
        name: &str,
        capacity: u32,
        selection: &LocalLobbySelection,
        seats: u32,
    ) -> Result<(), LocalLobbyError> {
        self.ensure_open()?;
        if self.membership.is_some() {
            return Err(LocalLobbyError::AlreadyHosting);
        }
        let account = self.account.clone();
        let lobby = self.service.create(
            &account,
            name,
            capacity,
            selection.composition.clone(),
            seats,
        )?;
        self.membership = Some(lobby.id.clone());
        self.last_lobby = Some(lobby);
        self.launched_generation = 0;
        self.completed_generation = 0;
        Ok(())
    }

    /// Join an existing lobby.
    pub fn join(&mut self, id: &str, seats: u32) -> Result<(), LocalLobbyError> {
        self.ensure_open()?;
        if self.membership.is_some() {
            return Err(LocalLobbyError::AlreadyJoined);
        }
        let account = self.account.clone();
        let lobby = self.service.join(id, &account, seats)?;
        self.membership = Some(lobby.id.clone());
        self.last_lobby = Some(lobby);
        self.launched_generation = 0;
        self.completed_generation = 0;
        Ok(())
    }

    /// Change local readiness.
    pub fn ready(&mut self, value: bool) -> Result<(), LocalLobbyError> {
        self.ensure_open()?;
        let lobby = self.require()?;
        let account_id = self.account.id.clone();
        self.service.ready(&lobby.id, &account_id, value)?;
        Ok(())
    }

    /// Start the match, binding and publishing the host world.
    pub fn start(&mut self) -> Result<(), LocalLobbyError> {
        self.ensure_open()?;
        let lobby = self.require()?;
        if lobby.owner != self.account.id {
            return Err(LocalLobbyError::NotHostStart);
        }
        let account_id = self.account.id.clone();
        let next = self.service.start(&lobby.id, &account_id)?;
        let bound = match self
            .transitions
            .host(&next)
            .map_err(LocalLobbyError::Transition)
        {
            Ok(bound) => bound,
            Err(original) => {
                let mut extra = Vec::new();
                if self.current().is_some() {
                    if let Err(cleanup) =
                        self.service.complete(&next.id, &account_id, next.match_generation)
                    {
                        extra.push(cleanup.to_string());
                    }
                }
                return combine_start_failure(original, extra);
            }
        };
        match self.service.publish(
            &next.id,
            &account_id,
            next.match_generation,
            bound.endpoint,
            bound.wire,
        ) {
            Ok(published) => {
                self.last_lobby = Some(published);
                self.launched_generation = next.match_generation;
                Ok(())
            }
            Err(original) => {
                let mut extra = Vec::new();
                if self.current().is_some() {
                    if let Err(cleanup) =
                        self.service.complete(&next.id, &account_id, next.match_generation)
                    {
                        extra.push(cleanup.to_string());
                    }
                }
                if let Err(cleanup) = self.transitions.leave(&next) {
                    extra.push(cleanup);
                }
                combine_start_failure(LocalLobbyError::Service(original), extra)
            }
        }
    }

    fn launch(&mut self, lobby: &Lobby) -> Result<(), LocalLobbyError> {
        if lobby.match_generation <= self.launched_generation {
            return Ok(());
        }
        self.transitions
            .join(lobby)
            .map_err(LocalLobbyError::Transition)?;
        self.launched_generation = lobby.match_generation;
        Ok(())
    }

    /// Poll membership: launch new generations, observe completions.
    pub fn poll(&mut self) -> Result<(), LocalLobbyError> {
        self.ensure_open()?;
        if self.membership.is_none() {
            return Ok(());
        }
        let Some(lobby) = self.current() else {
            let previous = self.last_lobby.take();
            self.membership = None;
            if let Some(previous) = previous {
                self.transitions
                    .leave(&previous)
                    .map_err(LocalLobbyError::Transition)?;
            }
            return Ok(());
        };
        self.last_lobby = Some(lobby.clone());
        match &lobby.phase {
            LobbyPhase::Playing { .. } => self.launch(&lobby),
            LobbyPhase::Open
                if self.launched_generation > self.completed_generation =>
            {
                self.transitions
                    .completed(&lobby)
                    .map_err(LocalLobbyError::Transition)?;
                self.completed_generation = self.launched_generation;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Complete the launched match generation.
    pub fn complete(&mut self) -> Result<(), LocalLobbyError> {
        self.ensure_open()?;
        let lobby = self.require()?;
        if lobby.owner != self.account.id {
            return Err(LocalLobbyError::NotHostComplete);
        }
        let account_id = self.account.id.clone();
        let completed = self
            .service
            .complete(&lobby.id, &account_id, self.launched_generation)?;
        if self.launched_generation > self.completed_generation {
            self.transitions
                .completed(&completed)
                .map_err(LocalLobbyError::Transition)?;
            self.completed_generation = self.launched_generation;
        }
        Ok(())
    }

    fn leave_current(&mut self) -> Result<(), LocalLobbyError> {
        let lobby = self.current().or_else(|| self.last_lobby.clone());
        self.membership = None;
        self.last_lobby = None;
        let Some(lobby) = lobby else {
            return Ok(());
        };
        if self
            .service
            .list()
            .iter()
            .any(|current| current.id == lobby.id)
        {
            let account_id = self.account.id.clone();
            self.service.leave(&lobby.id, &account_id)?;
        }
        self.transitions
            .leave(&lobby)
            .map_err(LocalLobbyError::Transition)?;
        Ok(())
    }

    /// Leave the current lobby.
    pub fn leave(&mut self) -> Result<(), LocalLobbyError> {
        self.ensure_open()?;
        self.leave_current()
    }

    /// Close the session, leaving any current lobby.
    pub fn close(&mut self) -> Result<(), LocalLobbyError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.leave_current()
    }
}

fn combine_start_failure(
    original: LocalLobbyError,
    extra: Vec<String>,
) -> Result<(), LocalLobbyError> {
    if extra.is_empty() {
        return Err(original);
    }
    let mut failures = vec![original.to_string()];
    failures.extend(extra);
    Err(LocalLobbyError::HostCleanup(failures.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_net::common::session::{
        CompositionIdentity, ContentDigest, Json, SessionComposition, WireSelection,
    };

    #[derive(Default)]
    struct FakeTransitions {
        calls: Vec<String>,
        bound: Option<BoundLobby>,
        fail: Option<String>,
    }

    impl FakeTransitions {
        fn fail_on(&mut self, call: &str, message: &str) {
            self.fail = Some(format!("{call}:{message}"));
        }

        fn check(&mut self, call: &str) -> Result<(), String> {
            self.calls.push(call.to_string());
            match &self.fail {
                Some(fail) if fail.starts_with(call) => {
                    Err(fail.split_once(':').map(|(_, reason)| reason).unwrap_or("fail").to_string())
                }
                _ => Ok(()),
            }
        }
    }

    impl LocalLobbyTransitions for FakeTransitions {
        fn host(&mut self, _lobby: &Lobby) -> Result<BoundLobby, String> {
            self.check("host")?;
            self.bound.clone().ok_or_else(|| "no binding".to_string())
        }
        fn join(&mut self, _lobby: &Lobby) -> Result<(), String> {
            self.check("join")
        }
        fn leave(&mut self, _lobby: &Lobby) -> Result<(), String> {
            self.check("leave")
        }
        fn completed(&mut self, _lobby: &Lobby) -> Result<(), String> {
            self.check("completed")
        }
    }

    fn account() -> Account {
        Account {
            id: "account:aaa".to_string(),
            name: "host".to_string(),
        }
    }

    fn composition() -> CompositionIdentity {
        let digest = ContentDigest::new(&"ab".repeat(32)).unwrap();
        let schema = ProviderId::new("session", "snapshot");
        CompositionIdentity {
            digest: digest.clone(),
            composition: SessionComposition {
                recipe: Json::Null,
                snapshot_schema: schema,
                actor_configurations: Vec::new(),
            },
        }
    }

    fn bound(composition: &CompositionIdentity) -> BoundLobby {
        BoundLobby {
            endpoint: NetworkAddress::Loopback {
                id: "test".to_string(),
            },
            wire: WireSelection::Unified {
                version: 1,
                composition: composition.digest.clone(),
                snapshot_schema: composition.composition.snapshot_schema.clone(),
            },
        }
    }

    fn session() -> (ApplicationLocalLobby<FakeTransitions>, LocalLobbySelection) {
        let composition = composition();
        let mut transitions = FakeTransitions::default();
        transitions.bound = Some(bound(&composition));
        (
            ApplicationLocalLobby::new(LocalLobbyService::new(), account(), transitions),
            LocalLobbySelection { composition },
        )
    }

    #[test]
    fn host_ready_start_poll_complete_leave() {
        let (mut lobby, selection) = session();
        lobby.host("game", 4, &selection, 1).unwrap();
        assert!(lobby.current().is_some());
        assert!(matches!(
            lobby.host("other", 4, &selection, 1).unwrap_err(),
            LocalLobbyError::AlreadyHosting
        ));
        lobby.ready(true).unwrap();
        lobby.start().unwrap();
        assert!(matches!(
            lobby.current().unwrap().phase,
            LobbyPhase::Playing { .. }
        ));
        lobby.poll().unwrap();
        lobby.poll().unwrap();
        lobby.complete().unwrap();
        lobby.leave().unwrap();
        assert!(lobby.current().is_none());
    }

    #[test]
    fn join_guards_and_missing_membership() {
        let (mut lobby, selection) = session();
        assert!(matches!(
            lobby.ready(true).unwrap_err(),
            LocalLobbyError::NoMembership
        ));
        assert!(matches!(
            lobby.join("lobby:absent", 1).unwrap_err(),
            LocalLobbyError::Service(_)
        ));
        lobby.host("game", 4, &selection, 1).unwrap();
        assert!(matches!(
            lobby.join("lobby:other", 1).unwrap_err(),
            LocalLobbyError::AlreadyJoined
        ));
        assert!(matches!(
            lobby.start().unwrap_err(),
            LocalLobbyError::Service(_)
        ));
    }

    #[test]
    fn failed_host_completes_back_to_open() {
        let (mut lobby, selection) = session();
        lobby.host("game", 4, &selection, 1).unwrap();
        lobby.ready(true).unwrap();
        lobby.transitions.fail_on("host", "bind failed");
        let error = lobby.start().unwrap_err();
        assert!(matches!(error, LocalLobbyError::Transition(_)));
        assert!(matches!(
            lobby.current().unwrap().phase,
            LobbyPhase::Open
        ));
    }

    #[test]
    fn transition_failures_map_and_close_locks() {
        let (mut lobby, selection) = session();
        lobby.host("game", 4, &selection, 1).unwrap();
        lobby.ready(true).unwrap();
        lobby.start().unwrap();
        lobby.transitions.fail_on("join", "join failed");
        assert!(matches!(
            lobby.poll().unwrap_err(),
            LocalLobbyError::Transition(_)
        ));
        lobby.transitions.fail = None;
        lobby.close().unwrap();
        lobby.close().unwrap();
        assert!(matches!(
            lobby.poll().unwrap_err(),
            LocalLobbyError::Closed
        ));
        assert!(matches!(
            lobby.leave().unwrap_err(),
            LocalLobbyError::Closed
        ));
    }
}
