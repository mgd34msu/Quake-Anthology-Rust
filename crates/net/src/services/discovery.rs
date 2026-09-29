//! Server browser operations ported from `src/network/services/discovery.ts`.
//!
//! Shared browser operations derived from Quake/Q2 server lists and Q3
//! `cl_main.c`. Codecs retain source text/opcodes; this table never guesses
//! a game's protocol.

use std::collections::{BTreeMap, HashMap};

use thiserror::Error;

use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::numeric::native_atoi;

use super::json::{parse_json, JsonError};
use crate::common::endpoint::{address_key, parse_network_address, AddressRecord, NetworkAddress};
use crate::common::session::{canonical, Json, WireSelection};
use crate::protocol::ProtocolIdentity;
use crate::q3_net::{
    decode_connectionless, encode_connectionless_text, q3_info_value, ConnectionlessReceiver, Q3NetError,
};

/// Error for discovery failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DiscoveryError {
    /// Invalid discovery request timeout.
    #[error("Invalid discovery request timeout")]
    BadTimeout,
    /// Invalid favorites save.
    #[error("Invalid favorites save")]
    BadFavorites,
    /// Discovery wire failure.
    #[error("{0}")]
    Wire(String),
    /// JSON save is malformed.
    #[error("Invalid favorites JSON: {0}")]
    BadJson(#[from] JsonError),
    /// Address record is invalid.
    #[error("Invalid favorite address: {0}")]
    BadAddress(#[from] crate::common::endpoint::AddressError),
}

/// Player detail row.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerDetail {
    /// Name.
    pub name: String,
    /// Score.
    pub score: i64,
    /// Ping in milliseconds.
    pub ping: i64,
}

/// Server status (`ServerStatus`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerStatus {
    /// Server name.
    pub name: String,
    /// Map name.
    pub map: String,
    /// Player count.
    pub players: i64,
    /// Maximum players.
    pub max_players: i64,
    /// Rules.
    pub rules: BTreeMap<String, String>,
    /// Player details.
    pub player_details: Vec<PlayerDetail>,
    /// Wire selection.
    pub wire: WireSelection,
}

/// Discovery source (`DiscoverySource`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiscoverySource {
    /// LAN broadcast.
    Lan,
    /// Master server.
    Master,
    /// Secondary master.
    SecondaryMaster,
    /// Favorite.
    Favorite,
    /// Direct query.
    Direct,
}

/// Browser entry (`BrowserEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserEntry {
    /// Server address.
    pub address: NetworkAddress,
    /// Discovery sources.
    pub sources: Vec<DiscoverySource>,
    /// Last status.
    pub status: Option<ServerStatus>,
    /// Last ping in milliseconds.
    pub ping_milliseconds: Option<f64>,
    /// Last update time.
    pub updated_at: f64,
}

/// Discovery request kind (`DiscoveryRequestKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryRequestKind {
    /// Server info.
    Info,
    /// Full status.
    Status,
}

/// Discovery request handle.
pub type DiscoveryRequestHandle = u64;

/// Per-request timeout policy: default, never, or an explicit deadline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RequestTimeout {
    /// Use the expiry sweep default.
    Default,
    /// Never expire.
    Never,
    /// Expire after these milliseconds.
    After(f64),
}

#[derive(Debug, Clone)]
struct PendingQuery {
    handle: DiscoveryRequestHandle,
    challenge: String,
    address: NetworkAddress,
    request_kind: DiscoveryRequestKind,
    sent_at: f64,
    retain_result: bool,
    timeout: RequestTimeout,
}

#[derive(Debug, Clone)]
struct PendingBroadcast {
    sequence: u64,
    sent_at: f64,
}

/// Discovery request result (`DiscoveryRequestResult`).
#[derive(Debug, Clone, PartialEq)]
pub enum DiscoveryRequestResult {
    /// Request pending.
    Pending {
        /// Address.
        address: NetworkAddress,
        /// Request kind.
        request_kind: DiscoveryRequestKind,
        /// Send time.
        sent_at: f64,
    },
    /// Request completed.
    Completed {
        /// Address.
        address: NetworkAddress,
        /// Request kind.
        request_kind: DiscoveryRequestKind,
        /// Send time.
        sent_at: f64,
        /// Status.
        status: Box<ServerStatus>,
        /// Ping in milliseconds.
        ping_milliseconds: f64,
        /// Completion time.
        completed_at: f64,
    },
    /// Request expired.
    Expired {
        /// Address.
        address: NetworkAddress,
        /// Request kind.
        request_kind: DiscoveryRequestKind,
        /// Send time.
        sent_at: f64,
    },
    /// Request cancelled.
    Cancelled {
        /// Address.
        address: NetworkAddress,
        /// Request kind.
        request_kind: DiscoveryRequestKind,
        /// Send time.
        sent_at: f64,
    },
}

/// Wire packet factory (`DiscoveryWire`).
pub trait DiscoveryWire {
    /// Info/status query with a challenge.
    fn query(&self, kind: DiscoveryRequestKind, challenge: &str) -> Result<Vec<u8>, DiscoveryError>;
    /// Master list query.
    fn master_query(&self) -> Result<Vec<u8>, DiscoveryError>;
    /// Heartbeat.
    fn heartbeat(&self, active: bool) -> Result<Vec<u8>, DiscoveryError>;
}

/// Packet sender (`PacketSender`).
pub trait PacketSender {
    /// Send a packet; `Ok(false)` reports a dropped send.
    fn send(&mut self, to: &NetworkAddress, bytes: &[u8]) -> bool;
}

/// Server browser (`ServerBrowser`).
pub struct ServerBrowser<'a> {
    wire: &'a dyn DiscoveryWire,
    transport: &'a mut dyn PacketSender,
    entries: HashMap<String, BrowserEntry>,
    queries: HashMap<DiscoveryRequestHandle, PendingQuery>,
    results: HashMap<DiscoveryRequestHandle, DiscoveryRequestResult>,
    broadcasts: HashMap<String, PendingBroadcast>,
    query_sequence: u64,
    next_handle: u64,
}

impl<'a> ServerBrowser<'a> {
    /// Create a browser over a wire and transport.
    pub fn new(wire: &'a dyn DiscoveryWire, transport: &'a mut dyn PacketSender) -> Self {
        Self {
            wire,
            transport,
            entries: HashMap::new(),
            queries: HashMap::new(),
            results: HashMap::new(),
            broadcasts: HashMap::new(),
            query_sequence: 0,
            next_handle: 1,
        }
    }

    /// Add or refresh an entry (`add`).
    pub fn add(&mut self, address: NetworkAddress, source: DiscoverySource, now: f64) -> BrowserEntry {
        let key = address_key(&address, true);
        let entry = match self.entries.get(&key) {
            None => BrowserEntry {
                address,
                sources: vec![source],
                status: None,
                ping_milliseconds: None,
                updated_at: now,
            },
            Some(previous) => {
                let mut sources = previous.sources.clone();
                if !sources.contains(&source) {
                    sources.push(source);
                }
                BrowserEntry {
                    sources,
                    ..previous.clone()
                }
            }
        };
        self.entries.insert(key, entry.clone());
        entry
    }

    /// Remove the favorite source (`removeFavorite`).
    pub fn remove_favorite(&mut self, address: &NetworkAddress) {
        self.remove_source(address, DiscoverySource::Favorite);
    }

    /// Remove a source, dropping the entry when sourceless (`removeSource`).
    pub fn remove_source(&mut self, address: &NetworkAddress, source: DiscoverySource) {
        let key = address_key(address, true);
        let Some(entry) = self.entries.get(&key) else {
            return;
        };
        let sources: Vec<DiscoverySource> = entry.sources.iter().copied().filter(|value| *value != source).collect();
        if sources.is_empty() {
            self.entries.remove(&key);
        } else {
            let mut updated = entry.clone();
            updated.sources = sources;
            self.entries.insert(key, updated);
        }
    }

    /// Restore an entry, merging sources (`restoreEntry`).
    pub fn restore_entry(&mut self, entry: BrowserEntry) {
        let key = address_key(&entry.address, true);
        match self.entries.get(&key) {
            None => {
                self.entries.insert(key, entry);
            }
            Some(existing) => {
                let mut sources = existing.sources.clone();
                for source in &entry.sources {
                    if !sources.contains(source) {
                        sources.push(*source);
                    }
                }
                let mut updated = if existing.status.is_none() {
                    entry.clone()
                } else {
                    existing.clone()
                };
                updated.sources = sources;
                self.entries.insert(key, updated);
            }
        }
    }

    /// Look up an entry (`entry`).
    #[must_use]
    pub fn entry(&self, address: &NetworkAddress) -> Option<&BrowserEntry> {
        self.entries.get(&address_key(address, true))
    }

    /// List entries (`list`).
    #[must_use]
    pub fn list(&self) -> Vec<&BrowserEntry> {
        self.entries.values().collect()
    }

    /// Entry count (`size`).
    #[must_use]
    pub fn size(&self) -> usize {
        self.entries.len()
    }

    /// Pending request count (`pendingRequests`).
    #[must_use]
    pub fn pending_requests(&self) -> usize {
        self.queries.len()
    }

    /// Fire-and-forget query, replacing same-kind queries (`query`).
    pub fn query(
        &mut self,
        address: &NetworkAddress,
        now: f64,
        kind: DiscoveryRequestKind,
    ) -> Result<bool, DiscoveryError> {
        let replaced: Vec<DiscoveryRequestHandle> = self
            .queries
            .values()
            .filter(|pending| {
                !pending.retain_result
                    && pending.request_kind == kind
                    && address_key(&pending.address, true) == address_key(address, true)
            })
            .map(|pending| pending.handle)
            .collect();
        for handle in replaced {
            self.release_request(handle);
        }
        Ok(self
            .start_request(address.clone(), now, kind, false, RequestTimeout::Default)?
            .is_some())
    }

    /// Tracked request with a timeout policy (`request`).
    pub fn request(
        &mut self,
        address: &NetworkAddress,
        now: f64,
        kind: DiscoveryRequestKind,
        timeout: RequestTimeout,
    ) -> Result<Option<DiscoveryRequestHandle>, DiscoveryError> {
        if let RequestTimeout::After(timeout) = timeout {
            if !timeout.is_finite() || timeout < 0.0 {
                return Err(DiscoveryError::BadTimeout);
            }
        }
        self.start_request(address.clone(), now, kind, true, timeout)
    }

    /// Look up a request result (`requestResult`).
    #[must_use]
    pub fn request_result(&self, handle: DiscoveryRequestHandle) -> Option<&DiscoveryRequestResult> {
        self.results.get(&handle)
    }

    /// Cancel a request (`cancelRequest`).
    pub fn cancel_request(&mut self, handle: DiscoveryRequestHandle) -> bool {
        let Some(pending) = self.queries.get(&handle).cloned() else {
            return false;
        };
        self.finish_request(
            &pending,
            DiscoveryRequestResult::Cancelled {
                address: pending.address.clone(),
                request_kind: pending.request_kind,
                sent_at: pending.sent_at,
            },
        );
        true
    }

    /// Release a request and its result (`releaseRequest`).
    pub fn release_request(&mut self, handle: DiscoveryRequestHandle) {
        self.queries.remove(&handle);
        self.results.remove(&handle);
    }

    fn start_request(
        &mut self,
        address: NetworkAddress,
        now: f64,
        kind: DiscoveryRequestKind,
        retain_result: bool,
        timeout: RequestTimeout,
    ) -> Result<Option<DiscoveryRequestHandle>, DiscoveryError> {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.query_sequence += 1;
        let challenge = self.query_sequence.to_string();
        let pending = PendingQuery {
            handle,
            challenge: challenge.clone(),
            address: address.clone(),
            request_kind: kind,
            sent_at: now,
            retain_result,
            timeout,
        };
        self.queries.insert(handle, pending);
        if retain_result {
            self.results.insert(
                handle,
                DiscoveryRequestResult::Pending {
                    address: address.clone(),
                    request_kind: kind,
                    sent_at: now,
                },
            );
        }
        let bytes = self.wire.query(kind, &challenge)?;
        if self.transport.send(&address, &bytes) {
            return Ok(Some(handle));
        }
        self.release_request(handle);
        Ok(None)
    }

    fn finish_request(&mut self, pending: &PendingQuery, result: DiscoveryRequestResult) {
        self.queries.remove(&pending.handle);
        if pending.retain_result {
            self.results.insert(pending.handle, result);
        }
    }

    /// Receive a status reply (`receive`).
    pub fn receive(
        &mut self,
        address: &NetworkAddress,
        status: ServerStatus,
        challenge: Option<&str>,
        now: f64,
        kind: Option<DiscoveryRequestKind>,
    ) -> bool {
        let key = address_key(address, true);
        let matches: Vec<PendingQuery> = self
            .queries
            .values()
            .filter(|query| {
                address_key(&query.address, true) == key
                    && kind.is_none_or(|expected| query.request_kind == expected)
                    && challenge.is_none_or(|expected| query.challenge == expected)
            })
            .cloned()
            .collect();
        // A challenge-less protocol cannot distinguish two requests to the same endpoint.
        if matches.len() > 1 {
            return false;
        }
        let direct = matches.first().cloned();
        let broadcast = if kind == Some(DiscoveryRequestKind::Status) {
            None
        } else if let Some(expected) = challenge {
            self.broadcasts.get(expected).cloned()
        } else {
            self.broadcasts
                .values()
                .max_by_key(|broadcast| broadcast.sequence)
                .cloned()
        };
        let pending = direct
            .clone()
            .map(|query| (query.sent_at, true))
            .or_else(|| broadcast.as_ref().map(|broadcast| (broadcast.sent_at, false)));
        let Some((sent_at, _)) = pending else {
            return false;
        };
        let ping_milliseconds = 0f64.max(now - sent_at);
        let old = if direct.is_none() {
            self.add(address.clone(), DiscoverySource::Lan, now)
        } else if let Some(existing) = self.entries.get(&key) {
            existing.clone()
        } else {
            self.add(address.clone(), DiscoverySource::Direct, now)
        };
        self.entries.insert(
            key,
            BrowserEntry {
                status: Some(status.clone()),
                ping_milliseconds: Some(ping_milliseconds),
                updated_at: now,
                ..old
            },
        );
        if let Some(direct) = direct {
            self.finish_request(
                &direct,
                DiscoveryRequestResult::Completed {
                    address: direct.address.clone(),
                    request_kind: direct.request_kind,
                    sent_at: direct.sent_at,
                    status: Box::new(status),
                    ping_milliseconds,
                    completed_at: now,
                },
            );
        }
        true
    }

    /// Query a master server (`queryMaster`).
    pub fn query_master(&mut self, address: &NetworkAddress) -> Result<bool, DiscoveryError> {
        let bytes = self.wire.master_query()?;
        Ok(self.transport.send(address, &bytes))
    }

    /// Record master-listed addresses (`receiveMaster`).
    pub fn receive_master(&mut self, addresses: &[NetworkAddress], now: f64) {
        for address in addresses {
            self.add(address.clone(), DiscoverySource::Master, now);
        }
    }

    /// Broadcast an info query (`broadcast`).
    pub fn broadcast(&mut self, addresses: &[NetworkAddress], now: f64) -> Result<usize, DiscoveryError> {
        let mut sent = 0;
        self.query_sequence += 1;
        let challenge = self.query_sequence.to_string();
        self.broadcasts.insert(
            challenge.clone(),
            PendingBroadcast {
                sequence: self.query_sequence,
                sent_at: now,
            },
        );
        for address in addresses {
            let bytes = self.wire.query(DiscoveryRequestKind::Info, &challenge)?;
            if self.transport.send(address, &bytes) {
                sent += 1;
            }
        }
        if sent == 0 {
            self.broadcasts.remove(&challenge);
        }
        Ok(sent)
    }

    /// Expire broadcasts and timed-out queries (`expireQueries`).
    pub fn expire_queries(&mut self, now: f64, timeout_milliseconds: f64) -> Vec<NetworkAddress> {
        self.broadcasts
            .retain(|_, query| now - query.sent_at < timeout_milliseconds);
        let mut expired = Vec::new();
        let handles: Vec<DiscoveryRequestHandle> = self
            .queries
            .values()
            .filter(|query| match query.timeout {
                RequestTimeout::Never => false,
                RequestTimeout::Default => now - query.sent_at >= timeout_milliseconds,
                RequestTimeout::After(deadline) => now - query.sent_at >= deadline,
            })
            .map(|query| query.handle)
            .collect();
        for handle in handles {
            let Some(query) = self.queries.get(&handle).cloned() else {
                continue;
            };
            if let Some(entry) = self.entries.get(&address_key(&query.address, true)) {
                if !expired
                    .iter()
                    .any(|address: &NetworkAddress| address_key(address, true) == address_key(&entry.address, true))
                {
                    expired.push(entry.address.clone());
                }
            }
            self.finish_request(
                &query,
                DiscoveryRequestResult::Expired {
                    address: query.address.clone(),
                    request_kind: query.request_kind,
                    sent_at: query.sent_at,
                },
            );
        }
        expired
    }

    /// Favorite addresses (`favoriteAddresses`).
    #[must_use]
    pub fn favorite_addresses(&self) -> Vec<NetworkAddress> {
        self.entries
            .values()
            .filter(|entry| entry.sources.contains(&DiscoverySource::Favorite))
            .map(|entry| entry.address.clone())
            .collect()
    }

    /// Serialize favorites (`saveFavorites`).
    #[must_use]
    pub fn save_favorites(&self) -> String {
        let values: Vec<Json> = self.favorite_addresses().iter().map(address_json).collect();
        canonical(&Json::Array(values)).unwrap_or_else(|_| "[]".to_owned())
    }

    /// Restore favorites, replacing the current set (`restoreFavorites`).
    pub fn restore_favorites(&mut self, text: &str) -> Result<(), DiscoveryError> {
        let value = parse_json(text)?;
        let Json::Array(entries) = value else {
            return Err(DiscoveryError::BadFavorites);
        };
        let mut addresses = Vec::new();
        for entry in &entries {
            addresses.push(address_from_json(entry)?);
        }
        for address in self.favorite_addresses() {
            self.remove_favorite(&address);
        }
        for address in addresses {
            self.add(address, DiscoverySource::Favorite, 0.0);
        }
        Ok(())
    }
}

fn address_json(address: &NetworkAddress) -> Json {
    let mut fields = BTreeMap::new();
    match address {
        NetworkAddress::Ipv4 { host, port } => {
            fields.insert("kind".to_owned(), Json::String("ipv4".to_owned()));
            fields.insert(
                "host".to_owned(),
                Json::Array(host.iter().map(|byte| Json::Number(f64::from(*byte))).collect()),
            );
            fields.insert("port".to_owned(), Json::Number(f64::from(*port)));
        }
        NetworkAddress::Ipv6 { host, port } => {
            fields.insert("kind".to_owned(), Json::String("ipv6".to_owned()));
            fields.insert("host".to_owned(), Json::String(host.clone()));
            fields.insert("port".to_owned(), Json::Number(f64::from(*port)));
        }
        NetworkAddress::Loopback { id } => {
            fields.insert("kind".to_owned(), Json::String("loopback".to_owned()));
            fields.insert("id".to_owned(), Json::String(id.clone()));
        }
        NetworkAddress::Ipx { network, node, port } => {
            fields.insert("kind".to_owned(), Json::String("ipx".to_owned()));
            fields.insert("network".to_owned(), Json::Number(f64::from(*network)));
            fields.insert(
                "node".to_owned(),
                Json::Array(node.iter().map(|byte| Json::Number(f64::from(*byte))).collect()),
            );
            fields.insert("port".to_owned(), Json::Number(f64::from(*port)));
        }
    }
    Json::Object(fields)
}

fn json_number(value: &Json) -> Option<f64> {
    match value {
        Json::Number(value) => Some(*value),
        _ => None,
    }
}

fn address_from_json(value: &Json) -> Result<NetworkAddress, DiscoveryError> {
    let Json::Object(fields) = value else {
        return Err(DiscoveryError::BadFavorites);
    };
    let kind = match fields.get("kind") {
        Some(Json::String(kind)) => kind.clone(),
        _ => return Err(DiscoveryError::BadFavorites),
    };
    let port = fields.get("port").and_then(json_number).map(|port| port as u32);
    let host_bytes = match fields.get("host") {
        Some(Json::Array(bytes)) => {
            let mut octets = Vec::new();
            for byte in bytes {
                octets.push(json_number(byte).ok_or(DiscoveryError::BadFavorites)? as u8);
            }
            Some(octets)
        }
        _ => None,
    };
    let host_text = match fields.get("host") {
        Some(Json::String(host)) => Some(host.clone()),
        _ => None,
    };
    let id = match fields.get("id") {
        Some(Json::String(id)) => Some(id.clone()),
        _ => None,
    };
    let network = fields
        .get("network")
        .and_then(json_number)
        .map(|network| network as u32);
    let node = match fields.get("node") {
        Some(Json::Array(bytes)) => {
            let mut octets = Vec::new();
            for byte in bytes {
                octets.push(json_number(byte).ok_or(DiscoveryError::BadFavorites)? as u8);
            }
            Some(octets)
        }
        _ => None,
    };
    Ok(parse_network_address(&AddressRecord {
        kind,
        host_bytes,
        host_text,
        id,
        network,
        node,
        port,
    })?)
}

/// Master heartbeat scheduler (`MasterHeartbeat`).
pub struct MasterHeartbeat<'a> {
    wire: &'a dyn DiscoveryWire,
    transport: &'a mut dyn PacketSender,
    interval_milliseconds: f64,
    next_time: f64,
}

impl<'a> MasterHeartbeat<'a> {
    /// Create a heartbeat scheduler.
    pub fn new(wire: &'a dyn DiscoveryWire, transport: &'a mut dyn PacketSender, interval_milliseconds: f64) -> Self {
        Self {
            wire,
            transport,
            interval_milliseconds,
            next_time: f64::NEG_INFINITY,
        }
    }

    /// Send heartbeats when due (`send`).
    pub fn send(
        &mut self,
        masters: &[NetworkAddress],
        now: f64,
        active: bool,
        force: bool,
    ) -> Result<usize, DiscoveryError> {
        if !force && now < self.next_time {
            return Ok(0);
        }
        self.next_time = now + self.interval_milliseconds;
        let mut sent = 0;
        for address in masters {
            let bytes = self.wire.heartbeat(active)?;
            if self.transport.send(address, &bytes) {
                sent += 1;
            }
        }
        Ok(sent)
    }
}

/// Quake III master protocol version.
pub const Q3_MASTER_PROTOCOL: i32 = 68;

/// Quake III discovery wire (`q3DiscoveryWire` in
/// `src/network/q3/discovery.ts`: `CL_GlobalServers_f`,
/// `CL_ServersResponsePacket`, `SVC_Info`/`Status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3DiscoveryWire {
    keywords: Vec<String>,
    protocol: i32,
}

impl Q3DiscoveryWire {
    /// Build a wire for master keywords and protocol.
    pub fn new(keywords: &[String], protocol: i32) -> Result<Self, Q3NetError> {
        if protocol <= 0 {
            return Err(Q3NetError::Range("Invalid Q3 master protocol"));
        }
        for keyword in keywords {
            if keyword.chars().any(|cell| cell.is_whitespace() || cell == '\0') {
                return Err(Q3NetError::Range("Q3 master keywords must be source words"));
            }
        }
        Ok(Self {
            keywords: keywords.to_vec(),
            protocol,
        })
    }
}

impl Default for Q3DiscoveryWire {
    /// Default wire: no keywords, protocol 68.
    fn default() -> Self {
        Self {
            keywords: Vec::new(),
            protocol: Q3_MASTER_PROTOCOL,
        }
    }
}

impl DiscoveryWire for Q3DiscoveryWire {
    fn query(&self, kind: DiscoveryRequestKind, challenge: &str) -> Result<Vec<u8>, DiscoveryError> {
        let command = if kind == DiscoveryRequestKind::Info {
            "getinfo"
        } else {
            "getstatus"
        };
        encode_connectionless_text(&format!("{command} {challenge}"))
            .map_err(|error| DiscoveryError::Wire(error.to_string()))
    }

    fn master_query(&self) -> Result<Vec<u8>, DiscoveryError> {
        let keywords = if self.keywords.is_empty() {
            String::new()
        } else {
            format!(" {}", self.keywords.join(" "))
        };
        encode_connectionless_text(&format!("getservers {}{keywords}", self.protocol))
            .map_err(|error| DiscoveryError::Wire(error.to_string()))
    }

    fn heartbeat(&self, _active: bool) -> Result<Vec<u8>, DiscoveryError> {
        // The donor sends the same heartbeat regardless of server state.
        encode_connectionless_text("heartbeat QuakeArena-1\n")
            .map_err(|error| DiscoveryError::Wire(error.to_string()))
    }
}

/// Decoded master response (`decodeQ3MasterPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3MasterPacket {
    /// Server addresses.
    pub addresses: Vec<NetworkAddress>,
    /// Whether the EOT marker closed the list.
    pub complete: bool,
}

/// Decode a master response (`decodeQ3MasterPacket`).
pub fn decode_q3_master_packet(bytes: &[u8]) -> Result<Q3MasterPacket, Q3NetError> {
    let prefix = b"getserversResponse";
    if bytes.len() < prefix.len() + 4 || bytes[0..4] != [255, 255, 255, 255] || bytes[4..4 + prefix.len()] != *prefix
    {
        return Err(Q3NetError::Range("Not a Q3 master response"));
    }
    let mut addresses = Vec::new();
    let mut cursor = 4 + prefix.len();
    while cursor < bytes.len() && bytes[cursor] != b'\\' {
        cursor += 1;
    }
    while cursor < bytes.len() && bytes[cursor] == b'\\' {
        cursor += 1;
        if bytes.get(cursor..cursor + 3) == Some(b"EOT".as_slice()) {
            return Ok(Q3MasterPacket {
                addresses,
                complete: true,
            });
        }
        if addresses.len() >= 256 {
            break;
        }
        let Some(entry) = bytes.get(cursor..cursor + 6) else {
            break;
        };
        cursor += 6;
        if bytes.get(cursor) != Some(&b'\\') {
            break;
        }
        addresses.push(NetworkAddress::Ipv4 {
            host: [entry[0], entry[1], entry[2], entry[3]],
            port: (u16::from(entry[4]) << 8) | u16::from(entry[5]),
        });
    }
    Ok(Q3MasterPacket {
        addresses,
        complete: false,
    })
}

/// Decode master addresses only (`decodeQ3MasterResponse`).
pub fn decode_q3_master_response(bytes: &[u8]) -> Result<Vec<NetworkAddress>, Q3NetError> {
    decode_q3_master_packet(bytes).map(|packet| packet.addresses)
}

/// Sanitize a response payload (`payloadText`).
fn q3_payload_text(bytes: &[u8]) -> String {
    let mut text = String::new();
    for byte in bytes {
        if *byte == 0 {
            break;
        }
        text.push(char::from(if *byte == 37 || *byte > 127 { 46 } else { *byte }));
    }
    text
}

/// Decoded server browser response (`decodeQ3ServerStatus`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ServerStatus {
    /// Echoed challenge.
    pub challenge: String,
    /// Server status.
    pub status: ServerStatus,
}

/// Decode a server browser response (`decodeQ3ServerStatus`).
pub fn decode_q3_server_status(bytes: &[u8]) -> Result<Q3ServerStatus, Q3NetError> {
    let packet = decode_connectionless(bytes, ConnectionlessReceiver::Client)?;
    let command = packet.command.to_lowercase();
    if command != "inforesponse" && command != "statusresponse" {
        return Err(Q3NetError::Range("Not a Q3 server browser response"));
    }
    let text = q3_payload_text(&packet.payload);
    let lines: Vec<&str> = text.split('\n').collect();
    let info = lines.first().copied().unwrap_or("");
    let mut rules = BTreeMap::new();
    let fields: Vec<&str> = info.split('\\').collect();
    let mut index = usize::from(info.starts_with('\\'));
    while index + 1 < fields.len() {
        rules.insert(fields[index].to_owned(), fields[index + 1].to_owned());
        index += 2;
    }
    if command == "inforesponse" && native_atoi(&q3_info_value(info, "protocol")?)? != Q3_MASTER_PROTOCOL {
        return Err(Q3NetError::Range("Server uses another Q3 wire version"));
    }
    let mut player_details = Vec::new();
    if command == "statusresponse" {
        for line in lines.iter().skip(1) {
            if line.is_empty() {
                continue;
            }
            let argv = tokenize_command(line, Dialect::Q3, TextMode::Source)?.argv;
            if argv.len() >= 3 {
                player_details.push(PlayerDetail {
                    name: argv[2].clone(),
                    score: i64::from(native_atoi(&argv[0])?),
                    ping: i64::from(native_atoi(&argv[1])?),
                });
            }
        }
    }
    let players = if command == "statusresponse" {
        player_details.len() as i64
    } else {
        i64::from(native_atoi(&q3_info_value(info, "clients")?)?)
    };
    let mut name = q3_info_value(info, "hostname")?;
    if name.is_empty() {
        name = q3_info_value(info, "sv_hostname")?;
    }
    Ok(Q3ServerStatus {
        challenge: q3_info_value(info, "challenge")?,
        status: ServerStatus {
            name,
            map: q3_info_value(info, "mapname")?,
            players,
            max_players: i64::from(native_atoi(&q3_info_value(info, "sv_maxclients")?)?),
            rules,
            player_details,
            wire: WireSelection::Source {
                protocol: ProtocolIdentity::Q3,
            },
        },
    })
}

/// Encode a status response (`encodeQ3Status`).
pub fn encode_q3_status(info: &str, players: &[PlayerDetail]) -> Result<Vec<u8>, Q3NetError> {
    let mut rows = String::new();
    for player in players {
        let row = format!("{} {} \"{}\"\n", player.score, player.ping, player.name);
        let row = String::from_utf16_lossy(&row.encode_utf16().take(1023).collect::<Vec<_>>());
        if rows.encode_utf16().count() + row.encode_utf16().count() >= 16384 {
            break;
        }
        rows.push_str(&row);
    }
    encode_connectionless_text(&format!("statusResponse\n{info}\n{rows}"))
}

/// Encode an info response (`encodeQ3Info`).
pub fn encode_q3_info(info: &str) -> Result<Vec<u8>, Q3NetError> {
    encode_connectionless_text(&format!("infoResponse\n{info}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::endpoint::ipv4_address;

    struct FakeWire;
    struct FakeTransport {
        sent: Vec<(NetworkAddress, Vec<u8>)>,
    }

    impl DiscoveryWire for FakeWire {
        fn query(&self, kind: DiscoveryRequestKind, challenge: &str) -> Result<Vec<u8>, DiscoveryError> {
            Ok(format!(
                "{}:{challenge}",
                if kind == DiscoveryRequestKind::Info {
                    "info"
                } else {
                    "status"
                }
            )
            .into_bytes())
        }

        fn master_query(&self) -> Result<Vec<u8>, DiscoveryError> {
            Ok(b"master".to_vec())
        }

        fn heartbeat(&self, active: bool) -> Result<Vec<u8>, DiscoveryError> {
            Ok(vec![u8::from(active)])
        }
    }

    impl PacketSender for FakeTransport {
        fn send(&mut self, to: &NetworkAddress, bytes: &[u8]) -> bool {
            self.sent.push((to.clone(), bytes.to_vec()));
            true
        }
    }

    fn status() -> ServerStatus {
        ServerStatus {
            name: "server".to_owned(),
            map: "dm1".to_owned(),
            players: 1,
            max_players: 8,
            rules: BTreeMap::new(),
            player_details: Vec::new(),
            wire: WireSelection::Source {
                protocol: ProtocolIdentity::Q1Netquake,
            },
        }
    }

    #[test]
    fn browser_tracks_requests() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut browser = ServerBrowser::new(&wire, &mut transport);
        let address = ipv4_address([127, 0, 0, 1], 26000, false).unwrap();
        let handle = browser
            .request(&address, 0.0, DiscoveryRequestKind::Info, RequestTimeout::Default)
            .unwrap()
            .unwrap();
        assert_eq!(browser.pending_requests(), 1);
        assert!(browser.receive(&address, status(), None, 50.0, None));
        assert!(matches!(
            browser.request_result(handle),
            Some(DiscoveryRequestResult::Completed { .. })
        ));
        assert_eq!(browser.entry(&address).unwrap().ping_milliseconds, Some(50.0));
    }

    #[test]
    fn favorites_save_and_restore() {
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut browser = ServerBrowser::new(&wire, &mut transport);
        let address = ipv4_address([10, 0, 0, 5], 27910, false).unwrap();
        browser.add(address, DiscoverySource::Favorite, 0.0);
        let saved = browser.save_favorites();
        let wire = FakeWire;
        let mut transport = FakeTransport { sent: Vec::new() };
        let mut restored = ServerBrowser::new(&wire, &mut transport);
        restored.restore_favorites(&saved).unwrap();
        assert_eq!(restored.favorite_addresses().len(), 1);
    }

    #[test]
    fn q3_wire_packets_match_donor() {
        let wire = Q3DiscoveryWire::new(&["ctf".to_owned(), "tour".to_owned()], 68).unwrap();
        assert_eq!(
            wire.query(DiscoveryRequestKind::Info, "xyz").unwrap(),
            encode_connectionless_text("getinfo xyz").unwrap()
        );
        assert_eq!(
            wire.query(DiscoveryRequestKind::Status, "xyz").unwrap(),
            encode_connectionless_text("getstatus xyz").unwrap()
        );
        assert_eq!(
            wire.master_query().unwrap(),
            encode_connectionless_text("getservers 68 ctf tour").unwrap()
        );
        assert_eq!(
            wire.heartbeat(true).unwrap(),
            encode_connectionless_text("heartbeat QuakeArena-1\n").unwrap()
        );
        assert_eq!(
            Q3DiscoveryWire::new(&[], 0),
            Err(Q3NetError::Range("Invalid Q3 master protocol"))
        );
        assert_eq!(
            Q3DiscoveryWire::new(&["two words".to_owned()], 68),
            Err(Q3NetError::Range("Q3 master keywords must be source words"))
        );
    }

    fn master_bytes(entries: &[[u8; 6]], complete: bool) -> Vec<u8> {
        let mut bytes = vec![255, 255, 255, 255];
        bytes.extend_from_slice(b"getserversResponse");
        for entry in entries {
            bytes.push(b'\\');
            bytes.extend_from_slice(entry);
        }
        bytes.push(b'\\');
        if complete {
            bytes.extend_from_slice(b"EOT");
        }
        bytes
    }

    #[test]
    fn q3_master_packet_round_trip() {
        let bytes = master_bytes(&[[10, 0, 0, 1, 0x13, 0xCD], [10, 0, 0, 2, 0x13, 0xCE]], true);
        let packet = decode_q3_master_packet(&bytes).unwrap();
        assert!(packet.complete);
        assert_eq!(
            packet.addresses,
            vec![
                ipv4_address([10, 0, 0, 1], 5069, false).unwrap(),
                ipv4_address([10, 0, 0, 2], 5070, false).unwrap(),
            ]
        );
        assert_eq!(decode_q3_master_response(&bytes).unwrap(), packet.addresses);
        let partial = decode_q3_master_packet(&master_bytes(&[[10, 0, 0, 1, 0, 80]], false)).unwrap();
        assert!(!partial.complete);
        assert_eq!(partial.addresses.len(), 1);
    }

    #[test]
    fn q3_master_packet_rejects_garbage() {
        assert_eq!(
            decode_q3_master_packet(b"nope"),
            Err(Q3NetError::Range("Not a Q3 master response"))
        );
        // Entry without a closing backslash is dropped.
        let mut bytes = vec![255, 255, 255, 255];
        bytes.extend_from_slice(b"getserversResponse\\");
        bytes.extend_from_slice(&[1, 2, 3, 4, 0, 80]);
        assert!(decode_q3_master_packet(&bytes).unwrap().addresses.is_empty());
    }

    #[test]
    fn q3_info_response_round_trip() {
        let info = "\\hostname\\Frag House\\mapname\\q3dm1\\clients\\3\\sv_maxclients\\8\\protocol\\68\\challenge\\abc";
        let decoded = decode_q3_server_status(&encode_q3_info(info).unwrap()).unwrap();
        assert_eq!(decoded.challenge, "abc");
        assert_eq!(decoded.status.name, "Frag House");
        assert_eq!(decoded.status.map, "q3dm1");
        assert_eq!(decoded.status.players, 3);
        assert_eq!(decoded.status.max_players, 8);
        assert!(matches!(
            decoded.status.wire,
            WireSelection::Source {
                protocol: ProtocolIdentity::Q3
            }
        ));
    }

    #[test]
    fn q3_status_response_counts_players() {
        let players = vec![
            PlayerDetail {
                name: "alice".to_owned(),
                score: 10,
                ping: 20,
            },
            PlayerDetail {
                name: "bob".to_owned(),
                score: -5,
                ping: 30,
            },
        ];
        let info = "\\hostname\\Duel\\mapname\\q3tourney1\\sv_maxclients\\2\\challenge\\z9";
        let decoded = decode_q3_server_status(&encode_q3_status(info, &players).unwrap()).unwrap();
        assert_eq!(decoded.challenge, "z9");
        assert_eq!(decoded.status.players, 2);
        assert_eq!(decoded.status.player_details, players);
    }

    #[test]
    fn q3_responses_validate_wire() {
        let wrong = encode_q3_info("\\hostname\\x\\protocol\\67").unwrap();
        assert_eq!(
            decode_q3_server_status(&wrong),
            Err(Q3NetError::Range("Server uses another Q3 wire version"))
        );
        let other = encode_connectionless_text("print\nhello").unwrap();
        assert_eq!(
            decode_q3_server_status(&other),
            Err(Q3NetError::Range("Not a Q3 server browser response"))
        );
    }
}
