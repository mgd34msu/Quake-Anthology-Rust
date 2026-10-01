//! Quake II deathmatch completion reporting.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q2-match-reports.ts`
//! (`NativeQ2MapTransition`, `classicMatchScore`, `prepareQ2MatchReports`,
//! `rereleaseMatchScore`, `q2MatchCompleted`, `q2MatchReports`). Score math reads the
//! ported protocol stat slots directly; the simulation, native runtime, and recipe
//! (`./simulation/runtime.ts`, out of scope) arrive through the [`Q2MatchSimulation`] seam.
//! Round identities need OS entropy, which the workspace does not provide, so the host
//! mints them (donor `randomUUID`). Events use the ported [`PlayerProgressEvent`].

use qa_core::cvar::{q2_flags, CvarError, CvarRegistry};
use qa_core::identity::{ActorId, SeatId};
use qa_guest::core::contracts::ModuleIdentity;
use thiserror::Error;

use crate::bootstrap::player_progress::{PlayerProgressEvent, ProgressSource};

/// Round-identity cvar (donor module-private `roundIdentity`).
const ROUND_IDENTITY: &str = "qts_matchRoundIdentity";

/// Module transition that ends a native match (donor `NativeQ2MapTransition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeQ2MapTransition {
    /// Transitioning module.
    pub module: ModuleIdentity,
}

/// Failure of a match-report operation, with donor messages.
#[derive(Debug, Error)]
pub enum Q2MatchError {
    /// A stat slot was outside storage (donor `readElement` range error).
    #[error("Q2 field index outside storage: {0}")]
    StatOutOfRange(usize),
    /// Match reporting has no source cvars.
    #[error("Q2 match reporting has no source cvars")]
    NoSourceCvars,
    /// Match reporting was not prepared.
    #[error("Q2 match reporting was not prepared")]
    NotPrepared,
    /// A native transition belongs to another module.
    #[error("Native match transition belongs to another module")]
    ForeignModule,
    /// Registry failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// Read one stat slot (donor `readElement`).
fn read_element(stats: &[i16], index: usize) -> Result<i32, Q2MatchError> {
    stats
        .get(index)
        .copied()
        .map(i32::from)
        .ok_or(Q2MatchError::StatOutOfRange(index))
}

/// Classic end-of-match score (donor `classicMatchScore`).
pub fn classic_match_score(stats: &[i16]) -> Result<Option<i32>, Q2MatchError> {
    if read_element(stats, 17)? != 0 {
        return Ok(None);
    }
    Ok(Some(read_element(stats, 14)?))
}

/// Rerelease end-of-match score (donor `rereleaseMatchScore`).
pub fn rerelease_match_score(stats: &[i16]) -> Result<Option<i32>, Q2MatchError> {
    if read_element(stats, 13)? & 8 == 0 || read_element(stats, 17)? != 0 {
        return Ok(None);
    }
    Ok(Some(read_element(stats, 14)?))
}

/// Source intermission state (donor `source.players.intermission`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q2MatchIntermission {
    /// Match still playing.
    Playing,
    /// Match ended at `started`.
    Ended {
        /// Intermission start timestamp.
        started: f64,
    },
}

/// Native runtime edition (donor `Q2NativeRuntime["edition"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2NativeEdition {
    /// Classic.
    Classic,
    /// Rerelease.
    Rerelease,
}

/// One native player's settled stats (donor `playerState(slot + 1)` view).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2NativePlayerStats {
    /// Client slot.
    pub client_slot: u32,
    /// Player-state stats.
    pub stats: Vec<i16>,
}

/// Native runtime view (donor `Q2NativeRuntime` fields this module reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2MatchNative {
    /// Native module.
    pub module: ModuleIdentity,
    /// Runtime edition.
    pub edition: Q2NativeEdition,
    /// Settled player stats.
    pub players: Vec<Q2NativePlayerStats>,
}

/// One source player (donor `source.players.states` entry).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MatchSourcePlayer {
    /// Whether the player spectates.
    pub spectator: bool,
    /// Match score.
    pub score: f64,
}

/// One local seat (donor `q2MatchReports` locals entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2MatchLocal {
    /// Local actor.
    pub actor: ActorId,
    /// Local seat.
    pub seat: SeatId,
}

/// Simulation, native runtime, and recipe seam (donor `SharedSimulation` subset).
pub trait Q2MatchSimulation {
    /// Whether a source or native runtime exists.
    fn has_q2_runtime(&self) -> bool;
    /// Whether the mode is deathmatch.
    fn deathmatch(&self) -> bool;
    /// The source registry, when present.
    fn server_cvars(&mut self) -> Option<&mut CvarRegistry>;
    /// Mint a fresh round identity (donor `randomUUID`).
    fn new_round_identity(&mut self) -> String;
    /// Source intermission, or [`None`] without a source runtime.
    fn source_intermission(&self) -> Option<Q2MatchIntermission>;
    /// Source player by actor.
    fn source_player(&self, actor: &ActorId) -> Option<Q2MatchSourcePlayer>;
    /// Native runtime view, or [`None`] without one.
    fn native(&self) -> Option<Q2MatchNative>;
    /// Client slot for an actor, or [`None`] when absent.
    fn player_client_slot(&self, actor: &ActorId) -> Option<u32>;
    /// Map geometry path.
    fn map_path(&self) -> String;
}

/// Format a timestamp the way JavaScript template interpolation does.
fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value.trunc() as i64)
    } else {
        format!("{value}")
    }
}

/// Publish and seed the round identity for deathmatch (donor `prepareQ2MatchReports`).
pub fn prepare_q2_match_reports(simulation: &mut impl Q2MatchSimulation, restoring: bool) -> Result<(), Q2MatchError> {
    if !simulation.has_q2_runtime() || !simulation.deathmatch() {
        return Ok(());
    }
    let needs_identity = {
        let cvars = simulation.server_cvars().ok_or(Q2MatchError::NoSourceCvars)?;
        cvars.register(ROUND_IDENTITY, "", q2_flags::READ_ONLY)?;
        !restoring || cvars.variable_string(ROUND_IDENTITY).is_empty()
    };
    if needs_identity {
        let identity = simulation.new_round_identity();
        let cvars = simulation.server_cvars().ok_or(Q2MatchError::NoSourceCvars)?;
        cvars.set(ROUND_IDENTITY, &identity, true)?;
    }
    Ok(())
}

/// Whether the deathmatch has completed (donor `q2MatchCompleted`).
pub fn q2_match_completed(
    simulation: &impl Q2MatchSimulation,
    transition: Option<&NativeQ2MapTransition>,
) -> Result<bool, Q2MatchError> {
    if !simulation.deathmatch() {
        return Ok(false);
    }
    if let Some(intermission) = simulation.source_intermission() {
        return Ok(!matches!(intermission, Q2MatchIntermission::Playing));
    }
    let Some(native) = simulation.native() else {
        return Ok(false);
    };
    if let Some(transition) = transition {
        if native.module != transition.module {
            return Err(Q2MatchError::ForeignModule);
        }
        return Ok(true);
    }
    if native.edition != Q2NativeEdition::Rerelease {
        return Ok(false);
    }
    for player in &native.players {
        if read_element(&player.stats, 13)? & 8 != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Build completion events for local seats (donor `q2MatchReports`).
pub fn q2_match_reports(
    simulation: &mut impl Q2MatchSimulation,
    locals: &[Q2MatchLocal],
    transition: Option<&NativeQ2MapTransition>,
) -> Result<Vec<PlayerProgressEvent>, Q2MatchError> {
    if !q2_match_completed(simulation, transition)? {
        return Ok(Vec::new());
    }
    let identity = simulation
        .server_cvars()
        .map(|cvars| cvars.variable_string(ROUND_IDENTITY))
        .unwrap_or_default();
    if identity.is_empty() {
        return Err(Q2MatchError::NotPrepared);
    }
    let map = simulation.map_path();
    if let Some(Q2MatchIntermission::Ended { started }) = simulation.source_intermission() {
        let mut reports = Vec::new();
        for local in locals {
            let Some(player) = simulation.source_player(&local.actor) else {
                continue;
            };
            if player.spectator {
                continue;
            }
            reports.push(PlayerProgressEvent::MatchCompleted {
                source: ProgressSource::Q2,
                participant: format!("local-seat:{}", local.seat.index()),
                event: format!("match:{identity}:{}", js_number(started)),
                map: map.clone(),
                score: player.score,
            });
        }
        return Ok(reports);
    }
    let Some(native) = simulation.native() else {
        return Ok(Vec::new());
    };
    let mut reports = Vec::new();
    for local in locals {
        let Some(slot) = simulation.player_client_slot(&local.actor) else {
            continue;
        };
        let Some(player) = native.players.iter().find(|player| player.client_slot == slot) else {
            continue;
        };
        let score = if native.edition == Q2NativeEdition::Classic || transition.is_some() {
            classic_match_score(&player.stats)?
        } else {
            rerelease_match_score(&player.stats)?
        };
        let Some(score) = score else {
            continue;
        };
        reports.push(PlayerProgressEvent::MatchCompleted {
            source: ProgressSource::Q2,
            participant: format!("local-seat:{}", local.seat.index()),
            event: format!("match:{identity}"),
            map: map.clone(),
            score: f64::from(score),
        });
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    struct Stub {
        cvars: CvarRegistry,
        intermission: Option<Q2MatchIntermission>,
        native: Option<Q2MatchNative>,
        source_players: Vec<(ActorId, Q2MatchSourcePlayer)>,
        slots: Vec<(ActorId, u32)>,
        identities: u32,
    }

    impl Q2MatchSimulation for Stub {
        fn has_q2_runtime(&self) -> bool {
            self.intermission.is_some() || self.native.is_some()
        }
        fn deathmatch(&self) -> bool {
            true
        }
        fn server_cvars(&mut self) -> Option<&mut CvarRegistry> {
            Some(&mut self.cvars)
        }
        fn new_round_identity(&mut self) -> String {
            self.identities += 1;
            format!("round-{}", self.identities)
        }
        fn source_intermission(&self) -> Option<Q2MatchIntermission> {
            self.intermission
        }
        fn source_player(&self, actor: &ActorId) -> Option<Q2MatchSourcePlayer> {
            self.source_players
                .iter()
                .find(|(id, _)| id == actor)
                .map(|(_, player)| *player)
        }
        fn native(&self) -> Option<Q2MatchNative> {
            self.native.clone()
        }
        fn player_client_slot(&self, actor: &ActorId) -> Option<u32> {
            self.slots.iter().find(|(id, _)| id == actor).map(|(_, slot)| *slot)
        }
        fn map_path(&self) -> String {
            "maps/q2dm1.bsp".to_string()
        }
    }

    fn stub() -> (Stub, IdentityOwner) {
        let owner = IdentityOwner::create("q2-match").unwrap();
        let stub = Stub {
            cvars: CvarRegistry::new(Dialect::Q2Rerelease),
            intermission: None,
            native: None,
            source_players: Vec::new(),
            slots: Vec::new(),
            identities: 0,
        };
        (stub, owner)
    }

    fn stats(pairs: &[(usize, i16)]) -> Vec<i16> {
        let mut stats = vec![0; 32];
        for (index, value) in pairs {
            stats[*index] = *value;
        }
        stats
    }

    #[test]
    fn scores_follow_stat_slots() {
        assert_eq!(classic_match_score(&stats(&[(14, 9)])).unwrap(), Some(9));
        assert_eq!(classic_match_score(&stats(&[(14, 9), (17, 1)])).unwrap(), None);
        assert_eq!(rerelease_match_score(&stats(&[(13, 8), (14, 4)])).unwrap(), Some(4));
        assert_eq!(rerelease_match_score(&stats(&[(14, 4)])).unwrap(), None);
        assert!(classic_match_score(&[]).is_err());
    }

    #[test]
    fn prepare_seeds_identity_once() {
        let (mut stub, _) = stub();
        stub.intermission = Some(Q2MatchIntermission::Playing);
        prepare_q2_match_reports(&mut stub, false).unwrap();
        assert_eq!(stub.cvars.variable_string(ROUND_IDENTITY), "round-1");
        prepare_q2_match_reports(&mut stub, true).unwrap();
        assert_eq!(stub.cvars.variable_string(ROUND_IDENTITY), "round-1");
        assert_eq!(stub.identities, 1);
    }

    #[test]
    fn completed_prefers_source_intermission() {
        let (mut stub, _) = stub();
        stub.intermission = Some(Q2MatchIntermission::Ended { started: 12.0 });
        assert!(q2_match_completed(&stub, None).unwrap());
        stub.intermission = Some(Q2MatchIntermission::Playing);
        assert!(!q2_match_completed(&stub, None).unwrap());
    }

    #[test]
    fn reports_cover_source_players() {
        let (mut stub, owner) = stub();
        let actor = owner.actor(0, 1);
        stub.intermission = Some(Q2MatchIntermission::Ended { started: 7.0 });
        stub.source_players.push((
            actor.clone(),
            Q2MatchSourcePlayer {
                spectator: false,
                score: 11.0,
            },
        ));
        prepare_q2_match_reports(&mut stub, false).unwrap();
        let locals = vec![Q2MatchLocal {
            actor,
            seat: owner.seat(0),
        }];
        let reports = q2_match_reports(&mut stub, &locals, None).unwrap();
        assert_eq!(reports.len(), 1);
        let PlayerProgressEvent::MatchCompleted { event, score, .. } = &reports[0] else {
            panic!("expected match completion");
        };
        assert_eq!(event, "match:round-1:7");
        assert_eq!(*score, 11.0);
    }

    #[test]
    fn native_transition_completes_and_reports() {
        let (mut stub, owner) = stub();
        let actor = owner.actor(0, 1);
        let module = ModuleIdentity::new(
            qa_core::identity::ProviderId::new("test", "native"),
            "q2-native",
            qa_guest::core::contracts::ContentDigest::new("sha256", "abc"),
            "1",
        );
        stub.native = Some(Q2MatchNative {
            module: module.clone(),
            edition: Q2NativeEdition::Rerelease,
            players: vec![Q2NativePlayerStats {
                client_slot: 0,
                stats: stats(&[(14, 5)]),
            }],
        });
        stub.slots.push((actor.clone(), 0));
        let transition = NativeQ2MapTransition { module };
        assert!(q2_match_completed(&stub, Some(&transition)).unwrap());
        prepare_q2_match_reports(&mut stub, false).unwrap();
        let locals = vec![Q2MatchLocal {
            actor,
            seat: owner.seat(0),
        }];
        let reports = q2_match_reports(&mut stub, &locals, Some(&transition)).unwrap();
        assert_eq!(reports.len(), 1);
    }
}
