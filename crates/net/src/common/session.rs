//! Session composition and client table ported from
//! `src/network/common/session.ts`.
//!
//! Composition identity canonicalizes the recipe, snapshot schema, and actor
//! configuration exactly like the donor (`canonical`), hashed with the local
//! SHA-256. Identity handles reuse [`qa_core::identity`]; wire selection
//! reuses [`crate::protocol::ProtocolIdentity`]. Simulation events carry an
//! opaque payload because routing only inspects the audience.

use std::collections::{BTreeMap, HashMap};

use qa_core::identity::{ClientId, IdentityOwner, ProviderId, SeatId, SessionId};
use thiserror::Error;

use super::endpoint::{same_address, NetworkAddress};
use super::hash::{hex_lower, Sha256};
use crate::protocol::ProtocolIdentity;

/// Error for composition and session failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    /// Composition identity contains a non-finite number.
    #[error("Composition identity contains a non-finite number")]
    NonFiniteNumber,
    /// Session is closed.
    #[error("Network session is closed")]
    Closed,
    /// Client is stale or belongs to another network session.
    #[error("Client is stale or belongs to another network session")]
    StaleClient,
    /// Client slot is foreign or already connected.
    #[error("Client slot is foreign or already connected")]
    SlotBusy,
    /// Seat belongs to another session.
    #[error("Seat belongs to another session")]
    ForeignSeat,
    /// Seat is already connected.
    #[error("Seat is already connected")]
    SeatBusy,
    /// Invalid remote seat bindings.
    #[error("Invalid remote seat bindings")]
    BadRemoteSeats,
    /// Seat already belongs to a connected client.
    #[error("Seat already belongs to a connected client")]
    SeatTaken,
    /// Client is not awaiting gamestate.
    #[error("Client is not awaiting gamestate")]
    NotAwaiting,
    /// Client has not received gamestate.
    #[error("Client has not received gamestate")]
    NotPrimed,
    /// Port rebinding cannot change base address.
    #[error("Port rebinding cannot change base address")]
    RebindBase,
    /// Composition schema version is not 1.
    #[error("Unsupported session composition version")]
    BadVersion,
    /// Content digest text is invalid.
    #[error("Invalid content digest")]
    BadDigest,
}

/// Canonical JSON value for composition identity.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Number.
    Number(f64),
    /// String.
    String(String),
    /// Array.
    Array(Vec<Json>),
    /// Object.
    Object(BTreeMap<String, Json>),
}

/// Canonical composition encoding (`canonical`).
pub fn canonical(value: &Json) -> Result<String, SessionError> {
    match value {
        Json::Null => Ok("null".to_owned()),
        Json::Bool(value) => Ok(value.to_string()),
        Json::String(value) => Ok(escape_json_string(value)),
        Json::Number(value) => Ok(js_number_string(*value)?),
        Json::Array(values) => {
            let mut text = String::from("[");
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    text.push(',');
                }
                text.push_str(&canonical(value)?);
            }
            text.push(']');
            Ok(text)
        }
        Json::Object(fields) => {
            let mut text = String::from("{");
            for (index, (key, value)) in fields.iter().enumerate() {
                if index > 0 {
                    text.push(',');
                }
                text.push_str(&escape_json_string(key));
                text.push(':');
                text.push_str(&canonical(value)?);
            }
            text.push('}');
            Ok(text)
        }
    }
}

/// JavaScript `String(number)` formatting (ECMA-262 number-to-string).
pub fn js_number_string(value: f64) -> Result<String, SessionError> {
    if !value.is_finite() {
        return Err(SessionError::NonFiniteNumber);
    }
    if value == 0.0 {
        return Ok(if value.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        });
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific.as_str(), "0"));
    let digits: String = mantissa.bytes().filter(|byte| *byte != b'.').map(|byte| byte as char).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let exp: i32 = exponent.parse().unwrap_or(0);
    let n = digits.len() as i32;
    let k = exp + 1;
    if k <= -6 || k > 21 {
        let mut text = format!("{sign}{}", &digits[0..1]);
        if digits.len() > 1 {
            text.push('.');
            text.push_str(&digits[1..]);
        }
        text.push('e');
        let shift = k - 1;
        text.push_str(&format!("{shift:+}"));
        return Ok(text);
    }
    if k <= 0 {
        return Ok(format!("{sign}0.{}{}", "0".repeat((-k) as usize), digits));
    }
    if k as usize >= digits.len() {
        return Ok(format!("{sign}{digits}{}", "0".repeat((k - n) as usize)));
    }
    Ok(format!("{sign}{}.{}", &digits[..k as usize], &digits[k as usize..]))
}

fn escape_json_string(value: &str) -> String {
    let mut text = String::with_capacity(value.len() + 2);
    text.push('"');
    for ch in value.chars() {
        match ch {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            ch if (ch as u32) < 0x20 => text.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => text.push(ch),
        }
    }
    text.push('"');
    text
}

/// SHA-256 content digest (`ContentDigest`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentDigest(String);

impl ContentDigest {
    /// Validate a lowercase hex digest.
    pub fn new(text: &str) -> Result<Self, SessionError> {
        if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(SessionError::BadDigest);
        }
        Ok(Self(text.to_ascii_lowercase()))
    }

    /// Digest text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One actor configuration entry in a composition.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorConfigurationEntry {
    /// Actor slot.
    pub slot: u32,
    /// Actor generation.
    pub generation: u32,
    /// Provider configuration.
    pub configuration: Json,
}

/// Session composition (`SessionComposition`).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionComposition {
    /// Mount precedence, executable identities, numeric profiles, timing.
    pub recipe: Json,
    /// Snapshot schema provider.
    pub snapshot_schema: ProviderId,
    /// Actor configurations.
    pub actor_configurations: Vec<ActorConfigurationEntry>,
}

/// Composition with its identity digest (`CompositionIdentity`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompositionIdentity {
    /// Identity digest.
    pub digest: ContentDigest,
    /// Canonical composition.
    pub composition: SessionComposition,
}

fn composition_json(composition: &SessionComposition) -> Json {
    let mut fields = BTreeMap::new();
    fields.insert("schemaVersion".to_owned(), Json::Number(1.0));
    fields.insert("recipe".to_owned(), composition.recipe.clone());
    fields.insert(
        "snapshotSchema".to_owned(),
        Json::String(format!(
            "{}:{}",
            composition.snapshot_schema.namespace, composition.snapshot_schema.name
        )),
    );
    fields.insert(
        "actorConfigurations".to_owned(),
        Json::Array(
            composition
                .actor_configurations
                .iter()
                .map(|entry| {
                    let mut fields = BTreeMap::new();
                    fields.insert("slot".to_owned(), Json::Number(f64::from(entry.slot)));
                    fields.insert(
                        "generation".to_owned(),
                        Json::Number(f64::from(entry.generation)),
                    );
                    fields.insert("configuration".to_owned(), entry.configuration.clone());
                    Json::Object(fields)
                })
                .collect(),
        ),
    );
    Json::Object(fields)
}

/// Compute a composition identity (`compositionIdentity`).
pub fn composition_identity(composition: &SessionComposition) -> Result<CompositionIdentity, SessionError> {
    let text = canonical(&composition_json(composition))?;
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = ContentDigest(hex_lower(&hasher.finish()));
    Ok(CompositionIdentity {
        digest,
        composition: composition.clone(),
    })
}

/// Wire selection (`WireSelection`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WireSelection {
    /// Unified wire with composition binding.
    Unified {
        /// Wire version (1).
        version: u32,
        /// Required composition digest.
        composition: ContentDigest,
        /// Required snapshot schema.
        snapshot_schema: ProviderId,
    },
    /// Source-native protocol wire.
    Source {
        /// Source protocol identity.
        protocol: ProtocolIdentity,
    },
}

/// Wire admission result (`WireAdmission`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireAdmission {
    /// Wire is supported.
    Supported,
    /// Wire is unsupported with reasons.
    Unsupported {
        /// Rejection reasons.
        reasons: Vec<String>,
    },
}

/// Source wire capability (`SourceWireCapability`).
pub trait SourceWireCapability {
    /// Admit a source protocol for a composition.
    fn supports(
        &self,
        composition: &SessionComposition,
        protocol: &ProtocolIdentity,
    ) -> WireAdmission;
}

/// Encode the initial composition offer (`encodeCompositionOffer`).
#[must_use]
pub fn encode_composition_offer(identity: &CompositionIdentity) -> Vec<u8> {
    let payload = canonical(&composition_json(&identity.composition)).unwrap_or_default();
    let payload = payload.as_bytes();
    let mut bytes = vec![0u8; payload.len() + 10];
    bytes[0..4].copy_from_slice(b"QTSU");
    bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
    bytes[6..10].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes[10..].copy_from_slice(payload);
    bytes
}

/// Admit a composition offer against the local identity
/// (`admitCompositionOffer`).
#[must_use]
pub fn admit_composition_offer(local: &CompositionIdentity, bytes: &[u8]) -> WireAdmission {
    if bytes.len() < 10 || &bytes[0..4] != b"QTSU" {
        return WireAdmission::Unsupported {
            reasons: vec!["Not a unified composition offer".to_owned()],
        };
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    let length = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    if version != 1 || length != bytes.len() - 10 {
        return WireAdmission::Unsupported {
            reasons: vec!["Invalid unified composition version or length".to_owned()],
        };
    }
    let mut hasher = Sha256::new();
    hasher.update(&bytes[10..]);
    let digest = ContentDigest(hex_lower(&hasher.finish()));
    if digest == local.digest {
        WireAdmission::Supported
    } else {
        WireAdmission::Unsupported {
            reasons: vec!["Content, executable, numeric, actor or snapshot composition differs".to_owned()],
        }
    }
}

/// Admit a wire selection (`admitWire`).
pub fn admit_wire(
    identity: &CompositionIdentity,
    selection: &WireSelection,
    source: &dyn SourceWireCapability,
) -> WireAdmission {
    match selection {
        WireSelection::Source { protocol } => source.supports(&identity.composition, protocol),
        WireSelection::Unified {
            composition,
            snapshot_schema,
            ..
        } => {
            let mut reasons = Vec::new();
            if composition != &identity.digest {
                reasons.push("Session composition differs".to_owned());
            }
            if snapshot_schema != &identity.composition.snapshot_schema {
                reasons.push("Snapshot schema differs".to_owned());
            }
            if reasons.is_empty() {
                WireAdmission::Supported
            } else {
                WireAdmission::Unsupported { reasons }
            }
        }
    }
}

/// Client attachment (`ClientAttachment`).
#[derive(Debug, Clone, PartialEq)]
pub enum ClientAttachment {
    /// Local seat.
    Local {
        /// Seat handle.
        seat: SeatId,
        /// Endpoint.
        endpoint: NetworkAddress,
    },
    /// Remote endpoint with seat bindings.
    Remote {
        /// Endpoint.
        endpoint: NetworkAddress,
        /// Remote seat bindings.
        remote_seats: Vec<RemoteSeat>,
    },
    /// Headless endpoint.
    Headless {
        /// Endpoint.
        endpoint: NetworkAddress,
    },
}

impl ClientAttachment {
    /// Attachment endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &NetworkAddress {
        match self {
            Self::Local { endpoint, .. }
            | Self::Remote { endpoint, .. }
            | Self::Headless { endpoint } => endpoint,
        }
    }
}

/// Remote seat binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteSeat {
    /// Seat handle.
    pub seat: SeatId,
    /// Remote seat index.
    pub index: i64,
}

/// Connection phase (`ConnectionPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionPhase {
    /// Connected, awaiting gamestate.
    Connected,
    /// Gamestate sent.
    Primed,
    /// Active.
    Active,
}

/// Connected session client (`SessionClient`).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionClient {
    /// Client handle.
    pub id: ClientId,
    /// Attachment.
    pub attachment: ClientAttachment,
    /// Selected wire.
    pub wire: WireSelection,
    /// Connection phase.
    pub phase: ConnectionPhase,
    /// Connect time in milliseconds.
    pub connected_at: f64,
    /// Last receive time in milliseconds.
    pub last_received_at: f64,
}

/// Simulation event audience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventAudience {
    /// Every client.
    World,
    /// One client.
    Client(ClientId),
    /// One seat.
    Seat(SeatId),
}

/// Routable simulation event with an opaque payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulationEvent {
    /// Event audience.
    pub audience: EventAudience,
    /// Opaque payload.
    pub payload: Vec<u8>,
}

struct ClientRecord {
    value: SessionClient,
    events: Vec<SimulationEvent>,
}

/// Session client table (`NetworkSession`).
pub struct NetworkSession<'a> {
    identities: &'a IdentityOwner,
    identity: CompositionIdentity,
    source: Box<dyn SourceWireCapability + 'a>,
    clients: HashMap<u32, ClientRecord>,
    ended: bool,
}

impl<'a> NetworkSession<'a> {
    /// Create a session table over caller-issued identities.
    pub fn new(
        identities: &'a IdentityOwner,
        identity: CompositionIdentity,
        source: Box<dyn SourceWireCapability + 'a>,
    ) -> Self {
        Self {
            identities,
            identity,
            source,
            clients: HashMap::new(),
            ended: false,
        }
    }

    /// Session handle.
    #[must_use]
    pub fn session(&self) -> &SessionId {
        self.identities.session()
    }

    /// Composition identity.
    #[must_use]
    pub fn identity(&self) -> &CompositionIdentity {
        &self.identity
    }

    fn opened(&self) -> Result<(), SessionError> {
        if self.ended {
            return Err(SessionError::Closed);
        }
        Ok(())
    }

    fn record(&mut self, id: &ClientId) -> Result<&mut ClientRecord, SessionError> {
        self.opened()?;
        let owned = self.identities.owns_client(id);
        let record = self.clients.get_mut(&id.slot());
        match (owned, record) {
            (true, Some(record)) if record.value.id == *id => Ok(record),
            _ => Err(SessionError::StaleClient),
        }
    }

    /// Connect a client (`connect`).
    pub fn connect(
        &mut self,
        id: ClientId,
        attachment: ClientAttachment,
        wire: WireSelection,
        now: f64,
    ) -> Result<WireAdmission, SessionError> {
        self.opened()?;
        if !self.identities.owns_client(&id) || self.clients.contains_key(&id.slot()) {
            return Err(SessionError::SlotBusy);
        }
        if let ClientAttachment::Local { seat, .. } = &attachment {
            if !self.identities.owns_seat(seat) {
                return Err(SessionError::ForeignSeat);
            }
            for record in self.clients.values() {
                if matches!(&record.value.attachment, ClientAttachment::Local { seat: other, .. } if other == seat)
                {
                    return Err(SessionError::SeatBusy);
                }
            }
        }
        if let ClientAttachment::Remote { remote_seats, .. } = &attachment {
            let mut indexes = std::collections::HashSet::new();
            let mut seats = std::collections::HashSet::new();
            for seat in remote_seats {
                if seat.index < 0
                    || !self.identities.owns_seat(&seat.seat)
                    || !indexes.insert(seat.index)
                    || !seats.insert(seat.seat.index())
                {
                    return Err(SessionError::BadRemoteSeats);
                }
            }
        }
        let assigned = attachment_seats(&attachment);
        for record in self.clients.values() {
            let existing = attachment_seats(&record.value.attachment);
            if existing.iter().any(|seat| assigned.contains(seat)) {
                return Err(SessionError::SeatTaken);
            }
        }
        let result = admit_wire(&self.identity, &wire, &*self.source);
        if matches!(result, WireAdmission::Unsupported { .. }) {
            return Ok(result);
        }
        self.clients.insert(
            id.slot(),
            ClientRecord {
                value: SessionClient {
                    id,
                    attachment,
                    wire,
                    phase: ConnectionPhase::Connected,
                    connected_at: now,
                    last_received_at: now,
                },
                events: Vec::new(),
            },
        );
        Ok(result)
    }

    /// Look up a client (`get`).
    pub fn get(&mut self, id: &ClientId) -> Result<SessionClient, SessionError> {
        Ok(self.record(id)?.value.clone())
    }

    /// List connected clients (`list`).
    pub fn list(&self) -> Result<Vec<SessionClient>, SessionError> {
        self.opened()?;
        Ok(self.clients.values().map(|record| record.value.clone()).collect())
    }

    /// Mark gamestate sent (`prime`).
    pub fn prime(&mut self, id: &ClientId) -> Result<(), SessionError> {
        let record = self.record(id)?;
        if record.value.phase != ConnectionPhase::Connected {
            return Err(SessionError::NotAwaiting);
        }
        record.value.phase = ConnectionPhase::Primed;
        Ok(())
    }

    /// Mark active (`activate`).
    pub fn activate(&mut self, id: &ClientId) -> Result<(), SessionError> {
        let record = self.record(id)?;
        if record.value.phase != ConnectionPhase::Primed {
            return Err(SessionError::NotPrimed);
        }
        record.value.phase = ConnectionPhase::Active;
        Ok(())
    }

    /// Record receipt (`received`).
    pub fn received(&mut self, id: &ClientId, now: f64) -> Result<(), SessionError> {
        let record = self.record(id)?;
        record.value.last_received_at = now;
        Ok(())
    }

    /// Rebind an authenticated endpoint to a new port (`rebind`).
    pub fn rebind(&mut self, id: &ClientId, endpoint: NetworkAddress) -> Result<(), SessionError> {
        let record = self.record(id)?;
        if !same_address(record.value.attachment.endpoint(), &endpoint, false) {
            return Err(SessionError::RebindBase);
        }
        let attachment = match &record.value.attachment {
            ClientAttachment::Local { seat, .. } => ClientAttachment::Local {
                seat: seat.clone(),
                endpoint,
            },
            ClientAttachment::Remote { remote_seats, .. } => ClientAttachment::Remote {
                endpoint,
                remote_seats: remote_seats.clone(),
            },
            ClientAttachment::Headless { .. } => ClientAttachment::Headless { endpoint },
        };
        record.value.attachment = attachment;
        Ok(())
    }

    /// Route an event to its audience (`route`).
    pub fn route(&mut self, event: SimulationEvent) -> Result<(), SessionError> {
        self.opened()?;
        for record in self.clients.values_mut() {
            let value = &record.value;
            let deliver = match &event.audience {
                EventAudience::World => true,
                EventAudience::Client(client) => value.id == *client,
                EventAudience::Seat(seat) => match &value.attachment {
                    ClientAttachment::Local { seat: owned, .. } => owned == seat,
                    ClientAttachment::Remote { remote_seats, .. } => {
                        remote_seats.iter().any(|binding| binding.seat == *seat)
                    }
                    ClientAttachment::Headless { .. } => false,
                },
            };
            if deliver {
                record.events.push(event.clone());
            }
        }
        Ok(())
    }

    /// Drain a client's events (`drain`).
    pub fn drain(&mut self, id: &ClientId) -> Result<Vec<SimulationEvent>, SessionError> {
        Ok(std::mem::take(&mut self.record(id)?.events))
    }

    /// Non-local clients past the timeout (`expired`).
    pub fn expired(&self, now: f64, timeout_milliseconds: f64) -> Result<Vec<ClientId>, SessionError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|client| {
                !matches!(client.attachment, ClientAttachment::Local { .. })
                    && now - client.last_received_at > timeout_milliseconds
            })
            .map(|client| client.id)
            .collect())
    }

    /// Disconnect a client (`disconnect`).
    pub fn disconnect(&mut self, id: &ClientId) -> Result<(), SessionError> {
        self.record(id)?;
        self.clients.remove(&id.slot());
        Ok(())
    }

    /// Close the session (`close`).
    pub fn close(&mut self) {
        if !self.ended {
            self.ended = true;
            self.clients.clear();
        }
    }
}

fn attachment_seats(attachment: &ClientAttachment) -> Vec<SeatId> {
    match attachment {
        ClientAttachment::Local { seat, .. } => vec![seat.clone()],
        ClientAttachment::Remote { remote_seats, .. } => {
            remote_seats.iter().map(|binding| binding.seat.clone()).collect()
        }
        ClientAttachment::Headless { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::q1;

    struct AcceptAll;

    impl SourceWireCapability for AcceptAll {
        fn supports(
            &self,
            _composition: &SessionComposition,
            _protocol: &ProtocolIdentity,
        ) -> WireAdmission {
            WireAdmission::Supported
        }
    }

    fn composition() -> SessionComposition {
        SessionComposition {
            recipe: Json::Object(BTreeMap::new()),
            snapshot_schema: ProviderId::new("test", "snap"),
            actor_configurations: Vec::new(),
        }
    }

    #[test]
    fn js_numbers_match_donor_format() {
        assert_eq!(js_number_string(0.0).unwrap(), "0");
        assert_eq!(js_number_string(-0.0).unwrap(), "-0");
        assert_eq!(js_number_string(42.0).unwrap(), "42");
        assert_eq!(js_number_string(0.1 + 0.2).unwrap(), "0.30000000000000004");
        assert_eq!(js_number_string(1e21).unwrap(), "1e+21");
        assert_eq!(js_number_string(1e20).unwrap(), "100000000000000000000");
        assert_eq!(js_number_string(1e-6).unwrap(), "0.000001");
        assert_eq!(js_number_string(1e-7).unwrap(), "1e-7");
        assert_eq!(js_number_string(-123.456).unwrap(), "-123.456");
    }

    #[test]
    fn composition_offer_round_trips() {
        let identity = composition_identity(&composition()).unwrap();
        let offer = encode_composition_offer(&identity);
        assert_eq!(admit_composition_offer(&identity, &offer), WireAdmission::Supported);
        assert!(matches!(
            admit_composition_offer(&identity, &[0, 1, 2]),
            WireAdmission::Unsupported { .. }
        ));
        let wire = WireSelection::Source {
            protocol: ProtocolIdentity::Q1Netquake,
        };
        assert_eq!(admit_wire(&identity, &wire, &AcceptAll), WireAdmission::Supported);
        let _ = q1::PROTOCOL_VERSION;
    }

    #[test]
    fn session_connects_primes_and_expires() {
        use crate::common::endpoint::ipv4_address;
        let owner = IdentityOwner::create("test").unwrap();
        let identity = composition_identity(&composition()).unwrap();
        let mut session = NetworkSession::new(&owner, identity, Box::new(AcceptAll));
        let id = owner.client(1, 1);
        let endpoint = ipv4_address([127, 0, 0, 1], 27910, false).unwrap();
        let wire = WireSelection::Source {
            protocol: ProtocolIdentity::Q1Netquake,
        };
        let admission = session
            .connect(
                id.clone(),
                ClientAttachment::Headless {
                    endpoint: endpoint.clone(),
                },
                wire,
                1000.0,
            )
            .unwrap();
        assert_eq!(admission, WireAdmission::Supported);
        session.prime(&id).unwrap();
        session.activate(&id).unwrap();
        assert_eq!(session.get(&id).unwrap().phase, ConnectionPhase::Active);
        session
            .route(SimulationEvent {
                audience: EventAudience::World,
                payload: vec![1],
            })
            .unwrap();
        assert_eq!(session.drain(&id).unwrap().len(), 1);
        assert_eq!(session.expired(10000.0, 1000.0).unwrap(), vec![id.clone()]);
        let rebound = ipv4_address([127, 0, 0, 1], 27911, false).unwrap();
        session.rebind(&id, rebound).unwrap();
        session.disconnect(&id).unwrap();
        assert!(session.list().unwrap().is_empty());
    }
}
