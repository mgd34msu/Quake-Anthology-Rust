//! QuakeWorld application server host types.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/qw-server-types.ts`
//! (`QwApplicationPlayer`, `QwApplicationServerHost`,
//! `QwServerNetworkOptions`). Wire messages reuse
//! [`QuakeWorldMessage`](qa_net::q1_net::QuakeWorldMessage), entities reuse
//! [`Q1WireEntity`](qa_net::q1_net::Q1WireEntity) (the packet-entities
//! payload; donor `QuakeWorldEntity`), input reuses
//! [`QwUserCommand`](qa_world::movement::types::QwUserCommand), and signon
//! reuses [`QuakeWorldSignonHost`](qa_net::q1_net::QuakeWorldSignonHost).
//! The donor's `Exclude<..., { kind: 'packet-entities' | 'invalid-delta' }>`
//! becomes the validated [`QwServerMessage`]. The donor restricts masters
//! and transports to `IpAddress`; the Rust port takes
//! [`NetworkAddress`](qa_net::common::endpoint::NetworkAddress), which has
//! no IP-only newtype yet.

use std::collections::HashMap;

use qa_core::identity::{ActorId, ClientId};
use qa_net::common::endpoint::NetworkAddress;
use qa_net::common::session::WireAdmission;
use qa_net::q1_net::{Q1WireEntity, QuakeWorldConnectRequest, QuakeWorldMessage, QuakeWorldSignonHost};
use qa_net::services::downloads::DownloadSource;
use qa_world::movement::types::QwUserCommand;
use qa_world::session::SimulationOutput;

use super::types::NetworkPresentationEvent;

/// Server message (`QwServerMessage`): any QuakeWorld message except packet
/// entities and invalid deltas (entity payloads travel in the frame).
#[derive(Debug, Clone, PartialEq)]
pub struct QwServerMessage {
    /// Wrapped message.
    pub message: QuakeWorldMessage,
}

impl QwServerMessage {
    /// Whether a message is admissible (neither packet entities nor an
    /// invalid delta).
    #[must_use]
    pub fn is_admissible(message: &QuakeWorldMessage) -> bool {
        !matches!(
            message,
            QuakeWorldMessage::PacketEntities { .. } | QuakeWorldMessage::InvalidDelta { .. }
        )
    }

    /// Wrap a message, rejecting entity payloads.
    #[must_use]
    pub fn new(message: QuakeWorldMessage) -> Option<Self> {
        if Self::is_admissible(&message) {
            Some(Self { message })
        } else {
            None
        }
    }
}

/// QuakeWorld application player (`QwApplicationPlayer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwApplicationPlayer {
    /// Owning client.
    pub client: ClientId,
    /// Bound actor.
    pub actor: ActorId,
    /// Player slot.
    pub slot: u32,
}

/// Admission verdict (donor `admit` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QwApplicationAdmission {
    /// Accepted with a bound player.
    Accepted {
        /// Bound player.
        player: QwApplicationPlayer,
    },
    /// Rejected with a reason.
    Rejected {
        /// Reason text.
        reason: String,
    },
}

/// Administration surface (`QwApplicationServerHost['administration']`).
pub trait QwServerAdministration {
    /// Rcon password.
    fn rcon_password(&self) -> String;
    /// True when the address is blocked.
    fn blocked(&self, from: &NetworkAddress) -> bool;
    /// Status text.
    fn status(&self) -> String;
    /// Log text for a sequence.
    fn log(&self, sequence: i64) -> Option<String>;
    /// Execute an admin command, streaming output.
    fn execute_admin(&mut self, command: &str, write: &mut dyn FnMut(&str));
}

/// Authentication surface (`QwApplicationServerHost['authentication']`).
pub trait QwServerAuthentication {
    /// Client password.
    fn password(&self) -> String;
    /// Spectator password.
    fn spectator_password(&self) -> String;
    /// High characters allowed.
    fn high_characters(&self) -> bool;
}

/// Frame publication (`QwApplicationServerHost['frame']` result).
#[derive(Debug, Clone, PartialEq)]
pub struct QwApplicationFrame {
    /// Visible entities.
    pub entities: Vec<Q1WireEntity>,
    /// Unreliable messages.
    pub messages: Vec<QwServerMessage>,
    /// Reliable messages.
    pub reliable: Vec<QwServerMessage>,
}

/// QuakeWorld application server host (`QwApplicationServerHost`).
pub trait QwApplicationServerHost {
    /// Administration surface, when the host exposes one.
    fn administration(&mut self) -> Option<&mut dyn QwServerAdministration> {
        None
    }
    /// Master servers, when the host advertises any.
    fn masters(&self) -> Option<Vec<NetworkAddress>> {
        None
    }
    /// Authentication surface, when the host exposes one.
    fn authentication(&self) -> Option<&dyn QwServerAuthentication> {
        None
    }
    /// Maximum clients.
    fn max_clients(&self) -> u32;
    /// Paused flag.
    fn paused(&self) -> bool;
    /// Native source wire binding.
    fn supports_source_wire(&self) -> WireAdmission;
    /// Admit a connect request.
    fn admit(&mut self, request: &QuakeWorldConnectRequest) -> QwApplicationAdmission;
    /// Resolve a carried client.
    fn carried_player(&self, client: &ClientId) -> QwApplicationPlayer;
    /// Recording player for a client, when the host records one.
    fn recording_player(&self, _client: &ClientId) -> Option<QwApplicationPlayer> {
        None
    }
    /// Recording signon buffers, when the host records them.
    fn recording_signon(&self, _player: &QwApplicationPlayer) -> Option<Vec<Vec<u8>>> {
        None
    }
    /// Client info table.
    fn client_info(&self, player: &QwApplicationPlayer) -> HashMap<String, String>;
    /// Run an action in the command phase, emitting per-recipient messages.
    fn command_phase(
        &mut self,
        player: &QwApplicationPlayer,
        action: &dyn Fn(),
        emit: &mut dyn FnMut(&QwApplicationPlayer, &QwServerMessage),
    );
    /// Disconnect a player.
    fn disconnect(&mut self, player: &QwApplicationPlayer, reason: &str);
    /// Signon host for a player.
    fn signon(&mut self, player: &QwApplicationPlayer) -> Box<dyn QuakeWorldSignonHost>;
    /// Open a download for a player (donor async; resolves inline here).
    fn prepare_download(&mut self, _player: &QwApplicationPlayer, _path: &str) -> Option<Box<dyn DownloadSource>> {
        None
    }
    /// Spawn baselines for a player.
    fn baselines(&self, player: &QwApplicationPlayer) -> Vec<Q1WireEntity>;
    /// Frame publication for a player.
    fn frame(&self, player: &QwApplicationPlayer, output: &SimulationOutput) -> QwApplicationFrame;
    /// Queue one recovered packet group for the application's existing
    /// authoritative step.
    fn command_group(&mut self, player: &QwApplicationPlayer, commands: &[QwUserCommand], sequence: u32);
    /// Run a client command.
    fn command(&mut self, player: &QwApplicationPlayer, name: &str, args: &[String]);
    /// Observe a completed simulation step.
    fn observe(&mut self, output: &SimulationOutput, events: &[NetworkPresentationEvent]);
    /// Print server text.
    fn print(&mut self, text: &str);
}

/// QuakeWorld server network options (`QwServerNetworkOptions`).
pub struct QwServerNetworkOptions<T, H> {
    /// Datagram transport.
    pub transport: T,
    /// Server host.
    pub host: H,
    /// Random source.
    pub random: Box<dyn FnMut() -> f64>,
    /// Idle timeout in milliseconds.
    pub timeout_milliseconds: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::q1_net::QwUnit;

    #[test]
    fn server_messages_exclude_entity_payloads() {
        assert!(QwServerMessage::new(QuakeWorldMessage::Unit(QwUnit::Nop)).is_some());
        assert!(QwServerMessage::new(QuakeWorldMessage::PacketEntities {
            sequence: 1,
            delta_sequence: None,
            entities: Vec::new(),
        })
        .is_none());
        assert!(QwServerMessage::new(QuakeWorldMessage::InvalidDelta {
            sequence: 1,
            requested: 0,
        })
        .is_none());
    }
}
