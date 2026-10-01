//! Rerelease guest services contract.
//!
//! Port of donor `src/app/bootstrap/simulation/rerelease-guest-services-contract.ts`
//! (`RereleaseGuestServicesOptions`, `RereleaseGuestMapServices`,
//! `RereleaseGuestMessage`, `RereleaseGuestServicesPort`).
//!
//! The donor's `hostOptions` member carries the DLL host's engine/spatial/
//! semantics imports; the Rust [`RereleaseQ2GuestHost`](qa_compat::q2::rerelease::host::RereleaseQ2GuestHost)
//! takes synthetic construction flags instead, so the port trait keeps the
//! live method surface and the source reads the options directly. Likewise
//! `bindMemory` collapses into [`RereleaseGuestServicesPort::bind_host`]:
//! the host owns its memory, and the services implement
//! [`RereleaseCoreServices`](qa_compat::q2::rerelease::imports::RereleaseCoreServices)
//! directly.

use std::collections::HashMap;

use qa_compat::q2::rerelease::debug_shapes::RereleaseDebugShapesEvent;
use qa_compat::q2::rerelease::host::{HostError, RereleaseQ2GuestHost};
use qa_compat::q2::rerelease::navigation::NavigationServices;
use qa_compat::q2::rerelease::world_text::RereleaseWorldTextEvent;
use qa_core::identity::ActorId;
use qa_net::q2_adapters::{Q2RereleaseEntityState, Q2RereleasePlayerState};
use thiserror::Error;

use super::classic_guest_services::{
    ClassicDamageProvenance, ClassicGuestMapServices, ClassicGuestMessage, ClassicGuestServicesOptions, ModelAppearance,
};
use super::types::{GuestLocalize, RereleaseSemanticBindings};

/// Rerelease guest services failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RereleaseGuestServicesError {
    /// Validation failure with a donor-shaped message.
    #[error("rerelease q2: {0}")]
    Invalid(String),
    /// Guest host failure.
    #[error("rerelease q2 host: {0}")]
    Host(String),
}

impl RereleaseGuestServicesError {
    /// Build a validation failure.
    #[must_use]
    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::Invalid(detail.into())
    }
}

impl From<HostError> for RereleaseGuestServicesError {
    fn from(error: HostError) -> Self {
        Self::Host(error.to_string())
    }
}

/// Result for rerelease guest services.
pub type RereleaseResult<T> = Result<T, RereleaseGuestServicesError>;

/// Client clipboard writer.
pub type RereleaseClipboardWrite = Box<dyn FnMut(&str)>;

/// Rerelease clipboard selection.
pub enum RereleaseGuestClipboard {
    /// Headless server clipboard.
    Dedicated,
    /// Client clipboard.
    Client(RereleaseClipboardWrite),
}

/// Debug-shapes sink.
pub type RereleaseDebugShapesFn = Box<dyn FnMut(&RereleaseDebugShapesEvent)>;
/// World-text sink.
pub type RereleaseWorldTextFn = Box<dyn FnMut(&RereleaseWorldTextEvent)>;

/// Rerelease guest services options: the classic options plus the API2023
/// frame cadence, localization, clipboard, debug, navigation, and semantic
/// bindings.
pub struct RereleaseGuestServicesOptions {
    /// Classic base options. The boxed engine already carries `worldActor`.
    pub base: ClassicGuestServicesOptions,
    /// Frame duration in milliseconds.
    pub frame_milliseconds: i32,
    /// String localization.
    pub localize: GuestLocalize,
    /// Clipboard selection.
    pub clipboard: RereleaseGuestClipboard,
    /// Debug-shapes sink.
    pub debug_shapes: RereleaseDebugShapesFn,
    /// World-text sink.
    pub world_text: RereleaseWorldTextFn,
    /// Navigation services.
    pub navigation: Box<dyn NavigationServices>,
    /// Semantic bindings override.
    pub semantic_bindings: Option<RereleaseSemanticBindings>,
    /// Foreign damage provenance.
    pub foreign_damage: Option<ClassicDamageProvenance>,
}

/// World-rebindable subset of the rerelease options.
pub struct RereleaseGuestMapServices {
    /// Classic base binding.
    pub base: ClassicGuestMapServices,
    /// Frame duration in milliseconds.
    pub frame_milliseconds: i32,
    /// String localization.
    pub localize: GuestLocalize,
    /// Debug-shapes sink.
    pub debug_shapes: RereleaseDebugShapesFn,
    /// World-text sink.
    pub world_text: RereleaseWorldTextFn,
    /// Navigation services.
    pub navigation: Box<dyn NavigationServices>,
    /// Foreign damage provenance.
    pub foreign_damage: Option<ClassicDamageProvenance>,
}

/// Rerelease guest message: a classic message with a float-multicast dialect
/// tag and a unicast dedupe key.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseGuestMessage {
    /// Classic base message.
    pub base: ClassicGuestMessage,
    /// Unicast dedupe key.
    pub dupe_key: u32,
}

impl RereleaseGuestMessage {
    /// Source multicast dialect tag.
    pub const SOURCE_DIALECT: &'static str = "q2-multicast-float";

    /// Source multicast dialect tag.
    #[must_use]
    pub fn source_dialect(&self) -> &'static str {
        Self::SOURCE_DIALECT
    }
}

/// Entity info snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseEntityInfo {
    /// Bound actor, if any.
    pub actor: Option<ActorId>,
    /// Whether the record is active.
    pub active: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Area numbers.
    pub areas: (i32, i32),
    /// Visibility clusters, if known.
    pub clusters: Option<Vec<i32>>,
    /// First visibility cluster.
    pub first_cluster: i32,
    /// Head node.
    pub headnode: i32,
    /// Owner slot, if any.
    pub owner_slot: Option<u32>,
}

/// Rerelease guest services port.
pub trait RereleaseGuestServicesPort {
    /// Current options.
    fn options(&self) -> &RereleaseGuestServicesOptions;
    /// Bind the services to a guest host.
    fn bind_host(&mut self, host: &mut RereleaseQ2GuestHost) -> RereleaseResult<()>;
    /// Mark spawning complete.
    fn complete_spawn(&mut self);
    /// Validate a map binding.
    fn validate_map(binding: &RereleaseGuestMapServices) -> RereleaseResult<()>;
    /// Publish entity presentation for every owned actor.
    fn publish_entities(&mut self, host: &mut RereleaseQ2GuestHost) -> RereleaseResult<()>;
    /// Read the client ping for a slot.
    fn player_ping(&self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<i32>;
    /// Store the client ping for a slot.
    fn set_player_ping(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32, ping: i32) -> RereleaseResult<()>;
    /// Begin a server frame.
    fn begin_frame(&mut self, frame: u32);
    /// Rebind the services to a new world.
    fn rebind_world(&mut self, binding: RereleaseGuestMapServices) -> RereleaseResult<()>;
    /// Decode the entity state for a slot.
    fn entity_state(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<Q2RereleaseEntityState>;
    /// Decode the player state for a slot.
    fn player_state(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<Q2RereleasePlayerState>;
    /// Resolve the model appearance for a slot.
    fn model_appearance(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<ModelAppearance>;
    /// Read the entity info for a slot.
    fn entity_info(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<RereleaseEntityInfo>;
    /// Drain the queued messages.
    fn drain_messages(&mut self) -> Vec<RereleaseGuestMessage>;
    /// Snapshot the configstrings.
    fn configstrings(&self) -> HashMap<i32, String>;
    /// Publish a configstring.
    fn set_configstring(&mut self, index: i32, value: &str) -> RereleaseResult<()>;
    /// Restore a configstring snapshot.
    fn restore_configstrings(&mut self, values: &HashMap<i32, String>) -> RereleaseResult<()>;
}

#[cfg(test)]
mod tests {
    use qa_compat::q2::classic::records::RawEntityView;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::GuestAddress;

    use super::super::classic_guest_services::{ClassicGuestAudience, MulticastScope};
    use super::*;
    use qa_core::math::Vec3;

    fn audience() -> ClassicGuestAudience {
        ClassicGuestAudience::Multicast {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            scope: MulticastScope::Phs,
        }
    }

    #[test]
    fn message_carries_dialect_and_dupe_key() {
        let message = RereleaseGuestMessage {
            base: ClassicGuestMessage {
                audience: audience(),
                reliable: true,
                bytes: vec![1, 2, 3],
            },
            dupe_key: 42,
        };
        assert_eq!(message.source_dialect(), "q2-multicast-float");
        assert_eq!(RereleaseGuestMessage::SOURCE_DIALECT, "q2-multicast-float");
        assert_eq!(message.dupe_key, 42);
        assert!(message.base.reliable);
    }

    #[test]
    fn entity_info_shape_covers_donor_fields() {
        let info = RereleaseEntityInfo {
            actor: None,
            active: true,
            server_flags: 5,
            areas: (1, 2),
            clusters: Some(vec![7, 9]),
            first_cluster: 7,
            headnode: 11,
            owner_slot: Some(3),
        };
        assert!(info.active);
        assert_eq!(info.areas, (1, 2));
        assert_eq!(info.owner_slot, Some(3));
        let _ = ProviderId::new("q2", "rerelease");
        let _ = RawEntityView {
            slot: 0,
            address: GuestAddress::new(0, 0),
            stride_bytes: 0,
        };
    }

    #[test]
    fn errors_render_donor_messages() {
        let error = RereleaseGuestServicesError::invalid("Invalid API2023 frame");
        assert_eq!(error.to_string(), "rerelease q2: Invalid API2023 frame");
    }
}
