//! Quake III application host types.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q3-types.ts` (`Q3ApplicationPlayer`,
//! `Q3SourceRoundBinding`, `Q3NetworkRoundRestart`, `Q3ApplicationServerHost`,
//! `q3GameCallback`). The donor's synchronous-or-`Promise` host calls become
//! synchronous. Admission, downloads, rates, snapshots, and wire commands
//! reuse `qa-net` (`Q3ServerAdmissionBindings`, `Q3AcceptedConnect`,
//! `Q3DownloadReadFile`, `Q3ServerRate`, `Gamestate`, `WireUserCommand`,
//! `Q3PureServer`); product identity reuses
//! [`Q3Product`](qa_net::q3_net::Q3Product) (donor
//! `/home/buzzkill/Projects/quake-typescript/src/network/q3/state/product.ts`); entity/player snapshots reuse
//! [`Q3EntityState`](qa_net::q3_net::Q3EntityState) and
//! [`Q3PlayerState`](qa_net::q3_net::Q3PlayerState).

use qa_core::identity::{ActorId, ClientId};
use qa_net::common::commands::ActorCommand;
use qa_net::common::session::WireAdmission;
use qa_net::q3::WireUserCommand;
use qa_net::q3_net::{
    Gamestate, Q3AcceptedConnect, Q3DownloadReadFile, Q3EntityState, Q3PlayerState, Q3Product, Q3PureServer,
    Q3ServerRate,
};
use qa_net::services::admin::ServerAdministration;
use thiserror::Error;

use super::types::NetworkPresentationEvent;

/// Quake III application player (`Q3ApplicationPlayer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ApplicationPlayer {
    /// Owning client.
    pub client: ClientId,
    /// Bound actor.
    pub actor: ActorId,
    /// Source entity number.
    pub source_entity: u32,
}

/// Admission verdict (`Q3ApplicationAdmission`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3ApplicationAdmission {
    /// Accepted with a bound player.
    Accepted {
        /// Bound player.
        player: Q3ApplicationPlayer,
    },
    /// Rejected with a reason.
    Rejected {
        /// Reason text.
        reason: String,
    },
}

/// Source round-trip binding (`Q3SourceRoundBinding`).
pub trait Q3SourceRoundBinding {
    /// Preflight the round.
    fn preflight(&mut self);
    /// Rebind the round.
    fn rebind(&mut self);
    /// Reconnect a client mid-round.
    fn reconnect(
        &mut self,
        client: &ClientId,
        userinfo: &str,
        last_command: &WireUserCommand,
    ) -> Q3ApplicationAdmission;
}

/// Snapshot server bit (donor `0 | 4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SnapshotServerBit {
    /// Bit 0.
    Zero,
    /// Bit 4.
    Four,
}

impl Q3SnapshotServerBit {
    /// Wire value.
    #[must_use]
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::Four => 4,
        }
    }
}

/// Network round restart (`Q3NetworkRoundRestart`).
pub trait Q3NetworkRoundRestart {
    /// Restarted clients.
    fn clients(&self) -> Vec<ClientId>;
    /// Snapshot server bit.
    fn snapshot_server_bit(&self) -> Q3SnapshotServerBit;
    /// Bind the source (donor async; resolves inline here).
    fn bind_source(&mut self);
    /// Receive presentation events (donor async; resolves inline here).
    fn receive_events(&mut self, events: &[NetworkPresentationEvent]);
    /// Reconnect one client (donor async; resolves inline here).
    fn reconnect_client(&mut self, client: &ClientId) -> bool;
}

/// Admission surface (`Q3ApplicationServerHost['admission']`).
pub trait Q3ApplicationAdmissionSurface {
    /// Private client slots.
    fn private_clients(&self) -> i32;
    /// Private password.
    fn private_password(&self) -> String;
    /// Reconnect limit in seconds.
    fn reconnect_limit_seconds(&self) -> i32;
    /// Minimum ping.
    fn minimum_ping(&self) -> f32;
    /// Maximum ping.
    fn maximum_ping(&self) -> f32;
    /// Demo restriction flag.
    fn demo_restricted(&self) -> bool;
    /// Whether the server runs.
    fn enabled(&self) -> bool;
    /// Game directory.
    fn game_directory(&self) -> String;
    /// Strict authentication mode.
    fn strict_auth(&self) -> String;
    /// Flood protection flag.
    fn flood_protect(&self) -> bool;
}

/// Configstring sink for [`Q3ApplicationServerHost::prepare`].
pub type Q3ConfigstringFn<'a> = &'a mut dyn FnMut(u32, &str);

/// Snapshot publication (`Q3ApplicationServerHost['snapshot']` result).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ApplicationSnapshot {
    /// Player state.
    pub player: Q3PlayerState,
    /// Area mask.
    pub area_mask: Vec<u8>,
    /// Visible entities.
    pub entities: Vec<Q3EntityState>,
}

/// Quake III application server host (`Q3ApplicationServerHost`).
pub trait Q3ApplicationServerHost {
    /// Admission surface, when the host exposes one.
    fn admission(&self) -> Option<&dyn Q3ApplicationAdmissionSurface> {
        None
    }
    /// Administration surface, when the host exposes one.
    fn administration(&mut self) -> Option<&mut dyn ServerAdministration> {
        None
    }
    /// Source round binding, when the host exposes one.
    fn source_round(&mut self) -> Option<&mut dyn Q3SourceRoundBinding> {
        None
    }
    /// Product identity.
    fn product(&self) -> Q3Product;
    /// Maximum clients.
    fn max_clients(&self) -> u32;
    /// Prepare a server id (donor async; resolves inline here).
    fn prepare(&mut self, checksum_feed: i32, server_id: i32, configstring: Option<Q3ConfigstringFn<'_>>);
    /// Pure server for a server id.
    fn pure(&self, server_id: i32, checksum_feed_server_id: Option<i32>) -> Q3PureServer;
    /// Whether downloads are enabled.
    fn downloads_enabled(&self) -> bool;
    /// Open a download by name.
    fn open_download(&mut self, name: &str) -> Option<Box<dyn Q3DownloadReadFile>>;
    /// Rate settings for a player.
    fn rate(&self, player: &Q3ApplicationPlayer) -> Q3ServerRate;
    /// Native source wire binding.
    fn supports_source_wire(&self) -> WireAdmission;
    /// Server time in milliseconds.
    fn time(&self) -> i32;
    /// Occupied client slots.
    fn occupied_slots(&self) -> Vec<i32>;
    /// Admit a connection (donor sync-or-async; resolves inline here).
    fn admit(&mut self, request: &Q3AcceptedConnect) -> Q3ApplicationAdmission;
    /// Resolve a carried client.
    fn carried_player(&self, client: &ClientId) -> Q3ApplicationPlayer;
    /// Disconnect a player (donor sync-or-async; resolves inline here).
    fn disconnect(&mut self, player: &Q3ApplicationPlayer, reason: &str);
    /// Game state for a player.
    fn game_state(&self, player: &Q3ApplicationPlayer, server_id: i32) -> Gamestate;
    /// Snapshot for a player.
    fn snapshot(&self, player: &Q3ApplicationPlayer) -> Q3ApplicationSnapshot;
    /// Begin a player session for the first user command. Returns whether the
    /// network should also run the command through [`input`](Self::input):
    /// the donor calls `input` when `begin` is undefined, so the default
    /// reports unhandled while session owners start the session and return
    /// `false`.
    fn begin(&mut self, _player: &Q3ApplicationPlayer, _command: &WireUserCommand) -> bool {
        true
    }
    /// Convert a client command into an actor command (donor
    /// sync-or-async; resolves inline here).
    fn input(&mut self, player: &Q3ApplicationPlayer, command: &WireUserCommand, sequence: u32)
        -> Option<ActorCommand>;
    /// Run a client command (donor sync-or-async; resolves inline here).
    fn command(&mut self, player: &Q3ApplicationPlayer, name: &str, args: &[String]);
    /// Update a player userinfo string (donor sync-or-async; inline here).
    fn userinfo(&mut self, player: &Q3ApplicationPlayer, value: &str);
    /// Status reply for a challenge, or `None` to stay silent.
    fn status(&self, challenge: &str, detailed: bool) -> Option<String>;
    /// Print server text.
    fn print(&mut self, text: &str);
}

/// Game callback failure (`Q3GameCallbackError`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3GameCallbackError {
    /// The callback panicked; Rust callbacks cannot throw, so a panic is
    /// the only failure this wrapper can observe.
    #[error("Q3 game callback failed: {0}")]
    Failed(String),
}

/// Run a game callback, wrapping a panic as [`Q3GameCallbackError`]
/// (`q3GameCallback`).
pub fn q3_game_callback<T>(callback: impl FnOnce() -> T) -> Result<T, Q3GameCallbackError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
        Ok(value) => Ok(value),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(ToString::to_string))
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(Q3GameCallbackError::Failed(message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_bit_spells_wire_values() {
        assert_eq!(Q3SnapshotServerBit::Zero.as_u8(), 0);
        assert_eq!(Q3SnapshotServerBit::Four.as_u8(), 4);
    }

    #[test]
    fn q3_game_callback_passes_values_and_wraps_panics() {
        assert_eq!(q3_game_callback(|| 7).expect("value"), 7);
        let error = q3_game_callback(|| -> i32 { panic!("boom") }).expect_err("panic");
        assert_eq!(error, Q3GameCallbackError::Failed("boom".to_string()));
    }
}
