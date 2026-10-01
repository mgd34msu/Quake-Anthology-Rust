//! Quake application host types.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q1-types.ts` (`Q1ApplicationPlayer`,
//! `Q1ApplicationGameState`, `Q1ApplicationServerHost`,
//! `Q1ServerNetworkOptions`). Wire messages reuse
//! [`NetQuakeMessage`](qa_net::q1_net::NetQuakeMessage), entities reuse
//! [`Q1WireEntity`](qa_net::q1_net::Q1WireEntity) (donor
//! `Q1ExtendedEntityState`), input reuses
//! [`Q1UserCommand`](qa_world::movement::types::Q1UserCommand), and protocol
//! identity reuses [`ProtocolIdentity`](qa_net::protocol::ProtocolIdentity).
//! The donor's `Exclude<..., { kind: 'entity' }>` becomes the validated
//! [`Q1ApplicationMessage`].

use std::collections::HashMap;

use qa_core::identity::{ActorId, ClientId};
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::NetworkAddress;
use qa_net::common::session::WireAdmission;
use qa_net::protocol::ProtocolIdentity;
use qa_net::q1_net::{NetQuakeMessage, Q1WireEntity};
use qa_world::movement::types::Q1UserCommand;
use qa_world::session::SimulationOutput;

use super::types::NetworkPresentationEvent;

/// Quake application player (`Q1ApplicationPlayer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ApplicationPlayer {
    /// Owning client.
    pub client: ClientId,
    /// Bound actor.
    pub actor: ActorId,
    /// Source entity number.
    pub source_entity: u32,
}

/// Application message (`Q1ApplicationMessage`): any NetQuake message
/// except an entity update (entities travel in the frame payload).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ApplicationMessage {
    /// Wrapped message.
    pub message: NetQuakeMessage,
}

impl Q1ApplicationMessage {
    /// Whether a message is admissible (not an entity update).
    #[must_use]
    pub fn is_admissible(message: &NetQuakeMessage) -> bool {
        !matches!(message, NetQuakeMessage::Entity { .. })
    }

    /// Wrap a message, rejecting entity updates.
    #[must_use]
    pub fn new(message: NetQuakeMessage) -> Option<Self> {
        if Self::is_admissible(&message) {
            Some(Self { message })
        } else {
            None
        }
    }
}

/// Server-info message (`Extract<NetQuakeMessage, { kind: 'server-info' }>`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ServerInfo {
    /// Wrapped message.
    pub message: NetQuakeMessage,
}

impl Q1ServerInfo {
    /// Wrap a message, accepting only server-info.
    #[must_use]
    pub fn new(message: NetQuakeMessage) -> Option<Self> {
        if matches!(message, NetQuakeMessage::ServerInfo { .. }) {
            Some(Self { message })
        } else {
            None
        }
    }
}

/// Decoded Quake game state (`Q1ApplicationGameState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ApplicationGameState {
    /// Server info.
    pub info: Q1ServerInfo,
    /// Spawn baselines by entity number.
    pub baselines: HashMap<u32, Q1WireEntity>,
    /// Signon messages.
    pub signon: Vec<Q1ApplicationMessage>,
}

/// Admission verdict (donor `admit` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1ApplicationAdmission {
    /// Accepted with a bound player.
    Accepted {
        /// Bound player.
        player: Q1ApplicationPlayer,
    },
    /// Rejected with a reason.
    Rejected {
        /// Reason text.
        reason: String,
    },
}

/// Frame publication (`Q1ApplicationServerHost['frame']` result).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ApplicationFrame {
    /// Frame time in seconds.
    pub seconds: f64,
    /// Unreliable messages.
    pub messages: Vec<Q1ApplicationMessage>,
    /// Reliable messages.
    pub reliable: Vec<Q1ApplicationMessage>,
    /// Datagram messages.
    pub datagram: Vec<Q1ApplicationMessage>,
    /// Visible entities.
    pub entities: Vec<Q1WireEntity>,
}

/// Quake application server host (`Q1ApplicationServerHost`).
pub trait Q1ApplicationServerHost {
    /// Whether an address is rejected before admission.
    fn rejects(&self, _address: &NetworkAddress) -> bool {
        false
    }
    /// Wire protocol.
    fn protocol(&self) -> ProtocolIdentity;
    /// Maximum clients.
    fn max_clients(&self) -> u32;
    /// Map name.
    fn map_name(&self) -> String;
    /// Native source wire binding.
    fn supports_source_wire(&self) -> WireAdmission;
    /// Admit an address.
    fn admit(&mut self, from: &NetworkAddress) -> Q1ApplicationAdmission;
    /// Resolve a carried client.
    fn carried_player(&self, client: &ClientId) -> Q1ApplicationPlayer;
    /// Disconnect a player.
    fn disconnect(&mut self, player: &Q1ApplicationPlayer, reason: &str);
    /// Game state for a player.
    fn game_state(&self, player: &Q1ApplicationPlayer) -> Q1ApplicationGameState;
    /// Spawn messages for a player.
    fn spawn(&mut self, player: &Q1ApplicationPlayer) -> Vec<Q1ApplicationMessage>;
    /// Frame publication for a player.
    fn frame(&self, player: &Q1ApplicationPlayer, output: &SimulationOutput) -> Q1ApplicationFrame;
    /// Convert a client command into an actor command.
    fn input(&mut self, player: &Q1ApplicationPlayer, command: &Q1UserCommand, sequence: u32) -> ActorCommand;
    /// Run a client command.
    fn command(&mut self, player: &Q1ApplicationPlayer, name: &str, args: &[String]);
    /// Observe a completed simulation step.
    fn observe(&mut self, output: &SimulationOutput, events: &[NetworkPresentationEvent]);
    /// Print server text.
    fn print(&mut self, text: &str);
}

/// Quake server network options (`Q1ServerNetworkOptions`).
pub struct Q1ServerNetworkOptions<T, H> {
    /// Datagram transport.
    pub transport: T,
    /// Server host.
    pub host: H,
    /// Idle timeout in milliseconds.
    pub timeout_milliseconds: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::q1_net::NqUnit;
    use qa_net::q1_wide::NqProfile;

    fn unit_message() -> NetQuakeMessage {
        NetQuakeMessage::Unit(NqUnit::Nop)
    }

    fn server_info_message() -> NetQuakeMessage {
        NetQuakeMessage::ServerInfo {
            protocol: NqProfile::Netquake,
            max_clients: 8,
            game_type: 0,
            level: "start".to_string(),
            models: Vec::new(),
            sounds: Vec::new(),
        }
    }

    #[test]
    fn application_messages_exclude_entities() {
        assert!(Q1ApplicationMessage::new(unit_message()).is_some());
        assert!(Q1ApplicationMessage::new(NetQuakeMessage::Entity {
            state: Q1WireEntity::default(),
        })
        .is_none());
    }

    #[test]
    fn server_info_accepts_only_server_info() {
        assert!(Q1ServerInfo::new(unit_message()).is_none());
        assert!(Q1ServerInfo::new(server_info_message()).is_some());
        assert!(Q1ApplicationMessage::new(server_info_message()).is_some());
    }
}
