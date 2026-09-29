//! Ranking service lifecycle ported from `src/network/services/rankings.ts`.
//!
//! Shared lifecycle for Q3 `sv_rankings.c`. The donor serializes async
//! provider calls through a promise tail; this port runs provider calls
//! synchronously and records the first failure as `unavailable`, preserving
//! the state machine.

use std::collections::HashMap;

use thiserror::Error;

/// Ranking account (`RankingAccount`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankingAccount {
    /// Player id.
    pub player_id: u64,
    /// Rank.
    pub rank: i32,
}

/// Ranking match (`RankingMatch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankingMatch {
    /// Game id.
    pub game_id: u64,
}

/// Account request (`RankingAccountRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingAccountRequest {
    /// Log in.
    Login {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },
    /// Create an account.
    Create {
        /// Username.
        username: String,
        /// Password.
        password: String,
        /// Email.
        email: String,
    },
}

/// Login result (`RankingLoginResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingLoginResult {
    /// Account active.
    Active {
        /// Account.
        account: RankingAccount,
    },
    /// Login denied.
    Denied {
        /// Reason.
        reason: String,
    },
}

/// Service report (`RankingServiceReport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingServiceReport {
    /// Integer report.
    Integer {
        /// Reporting player.
        this: u64,
        /// Other player.
        other: u64,
        /// Key.
        key: i32,
        /// Value.
        value: i32,
        /// Accumulate.
        accumulate: bool,
    },
    /// String report.
    String {
        /// Reporting player.
        this: u64,
        /// Other player.
        other: u64,
        /// Key.
        key: i32,
        /// Value.
        value: String,
    },
}

/// Ranking service provider (`RankingServiceProvider`).
pub trait RankingServiceProvider {
    /// Provider endpoint.
    fn endpoint(&self) -> &str;
    /// Begin a match.
    fn begin(&mut self, game_key: &str) -> Result<RankingMatch, RankingError>;
    /// Log in an account.
    fn login(
        &mut self,
        game: &RankingMatch,
        request: &RankingAccountRequest,
    ) -> Result<RankingLoginResult, RankingError>;
    /// Join an account.
    fn join(&mut self, game: &RankingMatch, account: &RankingAccount) -> Result<(), RankingError>;
    /// Submit a report.
    fn report(&mut self, game: &RankingMatch, report: &RankingServiceReport) -> Result<(), RankingError>;
    /// Service frame.
    fn poll(&mut self) -> Result<(), RankingError>;
    /// Log out an account.
    fn logout(&mut self, game: &RankingMatch, account: &RankingAccount) -> Result<(), RankingError>;
    /// Finish a match.
    fn finish(&mut self, game: &RankingMatch) -> Result<(), RankingError>;
}

/// Error for ranking failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RankingError {
    /// Provider failure.
    #[error("{0}")]
    Provider(String),
    /// Ranking match is already active.
    #[error("Ranking match is already active")]
    AlreadyActive,
    /// Ranking service is not active.
    #[error("Ranking service is not active")]
    NotActive,
    /// Ranking player is already active.
    #[error("Ranking player is already active")]
    PlayerActive,
    /// Ranking account is already joined.
    #[error("Ranking account is already joined")]
    AccountJoined,
    /// Ranking report submission or cleanup failed.
    #[error("Ranking report submission or cleanup failed")]
    CleanupFailed,
}

/// Service state (`RankingServiceState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingServiceState {
    /// Disabled.
    Disabled,
    /// Unavailable.
    Unavailable {
        /// Reason.
        reason: String,
    },
    /// Starting.
    Starting,
    /// Active.
    Active {
        /// Game id.
        game_id: u64,
    },
    /// Ending.
    Ending,
}

/// Player state (`RankingPlayerState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingPlayerState {
    /// New player.
    New,
    /// Spectator.
    Spectator,
    /// Login pending.
    Pending,
    /// Active account.
    Active {
        /// Account.
        account: RankingAccount,
    },
    /// Denied.
    Denied {
        /// Reason.
        reason: String,
    },
}

/// Ranking lifecycle (`RankingLifecycle`).
pub struct RankingLifecycle<'a> {
    provider: Option<&'a mut dyn RankingServiceProvider>,
    current: RankingServiceState,
    current_match: Option<RankingMatch>,
    players: HashMap<i32, RankingPlayerState>,
    changed: Box<dyn FnMut(i32, &RankingPlayerState) + 'a>,
    service_changed: Box<dyn FnMut(&RankingServiceState) + 'a>,
}

impl<'a> RankingLifecycle<'a> {
    /// Create a lifecycle over an optional provider and change callbacks.
    pub fn new(
        provider: Option<&'a mut dyn RankingServiceProvider>,
        changed: impl FnMut(i32, &RankingPlayerState) + 'a,
        service_changed: impl FnMut(&RankingServiceState) + 'a,
    ) -> Self {
        Self {
            provider,
            current: RankingServiceState::Disabled,
            current_match: None,
            players: HashMap::new(),
            changed: Box::new(changed),
            service_changed: Box::new(service_changed),
        }
    }

    fn set_state(&mut self, state: RankingServiceState) {
        self.current = state;
        (self.service_changed)(&self.current);
    }

    fn set_player(&mut self, slot: i32, state: RankingPlayerState) {
        self.players.insert(slot, state.clone());
        (self.changed)(slot, &state);
    }

    fn fail(&mut self, error: RankingError) -> RankingError {
        self.set_state(RankingServiceState::Unavailable {
            reason: error.to_string(),
        });
        error
    }

    /// Mark the service unavailable (`unavailable`).
    pub fn unavailable(&mut self, reason: &str) {
        self.set_state(RankingServiceState::Unavailable {
            reason: reason.to_owned(),
        });
    }

    /// Current state (`state`).
    #[must_use]
    pub fn state(&self) -> &RankingServiceState {
        &self.current
    }

    /// Player state (`player`).
    #[must_use]
    pub fn player(&self, slot: i32) -> RankingPlayerState {
        self.players.get(&slot).cloned().unwrap_or(RankingPlayerState::New)
    }

    /// Begin a match (`begin`).
    pub fn begin(&mut self, enabled: bool, single_player: bool, game_key: &str) -> Result<(), RankingError> {
        if self.current_match.is_some() {
            return Err(RankingError::AlreadyActive);
        }
        if !enabled || single_player {
            self.set_state(RankingServiceState::Disabled);
            return Ok(());
        }
        if self.provider.is_none() {
            self.set_state(RankingServiceState::Unavailable {
                reason: "No ranking service is configured. Local progress and match records remain available."
                    .to_owned(),
            });
            return Ok(());
        }
        self.set_state(RankingServiceState::Starting);
        let started = self.provider.as_deref_mut().map(|provider| provider.begin(game_key));
        match started.unwrap_or_else(|| unreachable!("provider presence was checked")) {
            Ok(game) => {
                self.current_match = Some(game);
                self.set_state(RankingServiceState::Active {
                    game_id: game.game_id,
                });
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    /// Log in or create an account (`account`).
    pub fn account(&mut self, slot: i32, request: RankingAccountRequest) -> Result<(), RankingError> {
        if self.provider.is_none() || self.current_match.is_none() || !matches!(self.current, RankingServiceState::Active { .. }) {
            return Err(RankingError::NotActive);
        }
        if matches!(self.player(slot), RankingPlayerState::Active { .. }) {
            return Err(RankingError::PlayerActive);
        }
        self.set_player(slot, RankingPlayerState::Pending);
        let game = self.current_match.unwrap_or(RankingMatch { game_id: 0 });
        let result = self.provider.as_deref_mut().unwrap_or_else(|| unreachable!("provider presence was checked")).login(&game, &request);
        match result {
            Ok(RankingLoginResult::Denied { reason }) => {
                self.set_player(slot, RankingPlayerState::Denied { reason });
                Ok(())
            }
            Ok(RankingLoginResult::Active { account }) => {
                if self.players.values().any(|state| matches!(state, RankingPlayerState::Active { account: other } if other.player_id == account.player_id)) {
                    let error = RankingError::AccountJoined;
                    self.set_player(slot, RankingPlayerState::Denied {
                        reason: error.to_string(),
                    });
                    return Err(error);
                }
                let join = self.provider.as_deref_mut().unwrap_or_else(|| unreachable!("provider presence was checked")).join(&game, &account);
                match join {
                    Ok(()) => {
                        self.set_player(slot, RankingPlayerState::Active { account });
                        Ok(())
                    }
                    Err(error) => {
                        let reason = error.to_string();
                        self.set_player(slot, RankingPlayerState::Denied { reason });
                        Err(self.fail(error))
                    }
                }
            }
            Err(error) => {
                let reason = error.to_string();
                self.set_player(slot, RankingPlayerState::Denied { reason });
                Err(self.fail(error))
            }
        }
    }

    fn id(&self, slot: i32) -> Option<u64> {
        if slot == -1 {
            return Some(0);
        }
        match self.player(slot) {
            RankingPlayerState::Active { account } => Some(account.player_id),
            _ => None,
        }
    }

    /// Submit an integer report (`reportInt`).
    pub fn report_int(
        &mut self,
        this: i32,
        other: i32,
        key: i32,
        value: i32,
        accumulate: bool,
    ) -> Result<(), RankingError> {
        self.report(this, other, |first, second| RankingServiceReport::Integer {
            this: first,
            other: second,
            key,
            value,
            accumulate,
        })
    }

    /// Submit a string report (`reportString`).
    pub fn report_string(&mut self, this: i32, other: i32, key: i32, value: &str) -> Result<(), RankingError> {
        self.report(this, other, |first, second| RankingServiceReport::String {
            this: first,
            other: second,
            key,
            value: value.to_owned(),
        })
    }

    fn report(
        &mut self,
        this: i32,
        other: i32,
        make: impl FnOnce(u64, u64) -> RankingServiceReport,
    ) -> Result<(), RankingError> {
        if !matches!(self.current, RankingServiceState::Active { .. })
            || self.current_match.is_none()
            || self.provider.is_none()
        {
            return Ok(());
        }
        let (Some(first), Some(second)) = (self.id(this), self.id(other)) else {
            return Ok(());
        };
        let game = self.current_match.unwrap_or(RankingMatch { game_id: 0 });
        let report = make(first, second);
        if let Err(error) = self.provider.as_deref_mut().unwrap_or_else(|| unreachable!("provider presence was checked")).report(&game, &report) {
            return Err(self.fail(error));
        }
        Ok(())
    }

    /// Service frame (`frame`).
    pub fn frame(&mut self) -> Result<(), RankingError> {
        if matches!(self.current, RankingServiceState::Active { .. }) {
            if let Some(provider) = self.provider.as_deref_mut() {
                if let Err(error) = provider.poll() {
                    return Err(self.fail(error));
                }
            }
        }
        Ok(())
    }

    /// Reset a denied or spectator slot (`reset`).
    pub fn reset(&mut self, slot: i32) -> Result<(), RankingError> {
        let state = self.player(slot);
        if matches!(self.current, RankingServiceState::Active { .. })
            && matches!(state, RankingPlayerState::Denied { .. } | RankingPlayerState::Spectator)
        {
            self.set_player(slot, RankingPlayerState::New);
        }
        Ok(())
    }

    /// Move a slot to spectator (`spectate`).
    pub fn spectate(&mut self, slot: i32) -> Result<(), RankingError> {
        if !matches!(self.current, RankingServiceState::Active { .. }) {
            return Ok(());
        }
        let state = self.player(slot);
        if let RankingPlayerState::Active { account } = state {
            if let Some(game) = self.current_match {
                if let Some(provider) = self.provider.as_deref_mut() {
                    if let Err(error) = provider.logout(&game, &account) {
                        return Err(self.fail(error));
                    }
                }
            }
        }
        self.set_player(slot, RankingPlayerState::Spectator);
        Ok(())
    }

    /// Disconnect a slot (`disconnect`).
    pub fn disconnect(&mut self, slot: i32) -> Result<(), RankingError> {
        let player = self.player(slot);
        if let RankingPlayerState::Active { account } = player {
            if let Some(game) = self.current_match {
                if let Some(provider) = self.provider.as_deref_mut() {
                    let _ = provider.logout(&game, &account);
                }
            }
        }
        self.players.remove(&slot);
        (self.changed)(slot, &RankingPlayerState::New);
        Ok(())
    }

    /// End the match (`end`).
    pub fn end(&mut self) -> Result<(), RankingError> {
        let Some(game) = self.current_match else {
            return Ok(());
        };
        self.set_state(RankingServiceState::Ending);
        let mut failures = 0;
        let slots: Vec<i32> = self.players.keys().copied().collect();
        for slot in slots {
            let player = self.player(slot);
            if let RankingPlayerState::Active { account } = player {
                if let Some(provider) = self.provider.as_deref_mut() {
                    if provider.logout(&game, &account).is_err() {
                        failures += 1;
                    }
                }
            }
            (self.changed)(slot, &RankingPlayerState::New);
        }
        if let Some(provider) = self.provider.as_deref_mut() {
            if provider.finish(&game).is_err() {
                failures += 1;
            }
        }
        self.current_match = None;
        self.players.clear();
        self.set_state(RankingServiceState::Disabled);
        if failures > 0 {
            return Err(RankingError::CleanupFailed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub {
        game: u64,
    }

    impl RankingServiceProvider for Stub {
        fn endpoint(&self) -> &str {
            "stub://rankings"
        }

        fn begin(&mut self, _game_key: &str) -> Result<RankingMatch, RankingError> {
            Ok(RankingMatch { game_id: self.game })
        }

        fn login(
            &mut self,
            _game: &RankingMatch,
            _request: &RankingAccountRequest,
        ) -> Result<RankingLoginResult, RankingError> {
            Ok(RankingLoginResult::Active {
                account: RankingAccount { player_id: 7, rank: 1 },
            })
        }

        fn join(&mut self, _game: &RankingMatch, _account: &RankingAccount) -> Result<(), RankingError> {
            Ok(())
        }

        fn report(&mut self, _game: &RankingMatch, _report: &RankingServiceReport) -> Result<(), RankingError> {
            Ok(())
        }

        fn poll(&mut self) -> Result<(), RankingError> {
            Ok(())
        }

        fn logout(&mut self, _game: &RankingMatch, _account: &RankingAccount) -> Result<(), RankingError> {
            Ok(())
        }

        fn finish(&mut self, _game: &RankingMatch) -> Result<(), RankingError> {
            Ok(())
        }
    }

    #[test]
    fn lifecycle_runs_a_match() {
        let mut stub = Stub { game: 42 };
        let mut lifecycle = RankingLifecycle::new(Some(&mut stub), |_, _| {}, |_| {});
        lifecycle.begin(true, false, "q3").unwrap();
        assert!(matches!(lifecycle.state(), RankingServiceState::Active { .. }));
        lifecycle
            .account(0, RankingAccountRequest::Login {
                username: "a".to_owned(),
                password: "b".to_owned(),
            })
            .unwrap();
        assert!(matches!(lifecycle.player(0), RankingPlayerState::Active { .. }));
        lifecycle.report_int(0, -1, 1, 2, true).unwrap();
        lifecycle.spectate(0).unwrap();
        lifecycle.end().unwrap();
        assert!(matches!(lifecycle.state(), RankingServiceState::Disabled));
    }
}
