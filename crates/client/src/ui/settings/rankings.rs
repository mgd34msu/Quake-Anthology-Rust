//! Ranking account view state for the settings menu.
//!
//! Donor provenance: `src/ui/settings/rankings.ts` in full. The service and
//! player snapshots below are UI-local mirrors of the shared lifecycle in
//! `src/network/services/rankings.ts` (`RankingServiceState` and
//! `RankingPlayerState`); the network side keeps the authoritative lifecycle
//! and the menu only reads these snapshots.

/// UI-local mirror of the donor `RankingAccount`: one signed-in identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RankingAccount {
    /// Provider-issued player id (donor `playerId`).
    pub player_id: u64,
    /// Provider-issued rank shown in the status line.
    pub rank: i32,
}

/// UI-local mirror of the donor `RankingServiceState`.
///
/// The menu never shows the donor `gameId`, and the transient `starting` and
/// `ending` states collapse into [`RankingServiceView::Busy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingServiceView {
    /// Rankings are disabled for this match.
    Disabled,
    /// The provider failed or is not configured; carries the donor `reason`.
    Unavailable {
        /// Human-readable reason.
        message: String,
    },
    /// The provider is starting or ending (donor `starting` / `ending`).
    Busy,
    /// The provider is running (donor `active`).
    Active,
}

/// UI-local mirror of the donor `RankingPlayerState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingPlayerView {
    /// Fresh slot that has not signed in yet (donor `new`).
    Idle,
    /// Signed out; may sign in to play ranked (donor `spectator`).
    Spectator,
    /// A sign-in round trip is in flight (donor `pending`).
    Pending,
    /// Signed in (donor `active`).
    Active {
        /// Signed-in account.
        account: RankingAccount,
    },
    /// The provider refused the last sign-in (donor `denied`).
    Denied {
        /// Human-readable reason.
        reason: String,
    },
}

/// UI-local mirror of the donor `RankingAccountRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingAccountRequest {
    /// Sign in to an existing account.
    Login {
        /// Account name.
        username: String,
        /// Account password.
        password: String,
    },
    /// Create a new account.
    Create {
        /// Account name.
        username: String,
        /// Account password.
        password: String,
        /// Contact address for the new account.
        email: String,
    },
}

/// Account UI actions for one admitted source slot, never a platform identity
/// inferred from a player name.
///
/// The donor methods are async (`Promise<void>`) and reject with `Error`; this
/// port is synchronous and reports the rejection message as `Err(String)`.
/// Callers must run slow providers off the draw path so the menu factory never
/// blocks, and surface the returned message in the menu status line.
pub trait RankingAccountActions {
    /// Read the current service snapshot.
    fn service(&self) -> RankingServiceView;
    /// Read the current player snapshot.
    fn player(&self) -> RankingPlayerView;
    /// Sign in or create an account.
    fn submit(&mut self, request: RankingAccountRequest) -> Result<(), String>;
    /// Return a denied or spectating slot to the fresh state.
    fn reset(&mut self) -> Result<(), String>;
    /// Sign out into the spectating state.
    fn spectate(&mut self) -> Result<(), String>;
}

/// Menu-facing view over the service and player snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingAccountView {
    /// Rankings are disabled for this match.
    Disabled,
    /// The provider failed or is not configured.
    Unavailable {
        /// Human-readable reason.
        message: String,
    },
    /// The provider is starting or ending.
    Busy,
    /// The provider is running; the player snapshot selects the menu rows.
    Account {
        /// Current player snapshot.
        player: RankingPlayerView,
    },
}

/// Map service and player snapshots to the menu-facing view.
///
/// Mirrors the donor `rankingAccountView`: disabled and unavailable short
/// circuit before the player snapshot is read, the transient service states
/// collapse to busy, and only an active service exposes the player.
#[must_use]
pub fn ranking_account_view(actions: &dyn RankingAccountActions) -> RankingAccountView {
    match actions.service() {
        RankingServiceView::Disabled => RankingAccountView::Disabled,
        RankingServiceView::Unavailable { message } => RankingAccountView::Unavailable { message },
        RankingServiceView::Busy => RankingAccountView::Busy,
        RankingServiceView::Active => RankingAccountView::Account {
            player: actions.player(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeActions {
        service: RankingServiceView,
        player: RankingPlayerView,
    }

    impl RankingAccountActions for FakeActions {
        fn service(&self) -> RankingServiceView {
            self.service.clone()
        }

        fn player(&self) -> RankingPlayerView {
            self.player.clone()
        }

        fn submit(&mut self, _request: RankingAccountRequest) -> Result<(), String> {
            Ok(())
        }

        fn reset(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn spectate(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    fn actions(service: RankingServiceView, player: RankingPlayerView) -> FakeActions {
        FakeActions { service, player }
    }

    #[test]
    fn disabled_service_maps_without_reading_player() {
        let denied = RankingPlayerView::Denied {
            reason: "unread".to_string(),
        };
        let view = ranking_account_view(&actions(RankingServiceView::Disabled, denied));
        assert_eq!(view, RankingAccountView::Disabled);
    }

    #[test]
    fn unavailable_service_carries_reason_without_reading_player() {
        let view = ranking_account_view(&actions(
            RankingServiceView::Unavailable {
                message: "No provider.".to_string(),
            },
            RankingPlayerView::Active {
                account: RankingAccount { player_id: 7, rank: 3 },
            },
        ));
        assert_eq!(
            view,
            RankingAccountView::Unavailable {
                message: "No provider.".to_string(),
            }
        );
    }

    #[test]
    fn busy_service_collapses_transient_states() {
        let view = ranking_account_view(&actions(RankingServiceView::Busy, RankingPlayerView::Pending));
        assert_eq!(view, RankingAccountView::Busy);
    }

    #[test]
    fn active_service_exposes_every_player_state() {
        let players = vec![
            RankingPlayerView::Idle,
            RankingPlayerView::Spectator,
            RankingPlayerView::Pending,
            RankingPlayerView::Active {
                account: RankingAccount {
                    player_id: 11,
                    rank: 42,
                },
            },
            RankingPlayerView::Denied {
                reason: "Bad password.".to_string(),
            },
        ];
        for player in players {
            let view = ranking_account_view(&actions(RankingServiceView::Active, player.clone()));
            assert_eq!(view, RankingAccountView::Account { player });
        }
    }
}
