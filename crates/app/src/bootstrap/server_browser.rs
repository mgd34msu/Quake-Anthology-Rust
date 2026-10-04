//! Startup server browser.
//!
//! Donor: `src/app/bootstrap/server-browser.ts`
//! (`StartupServerBrowser`). Async operations become sync blocking
//! calls: DNS via `resolve_address`, config via `ConfigStore`, master
//! fetch via an injected supplier, mDNS via an injected
//! `KexMdnsPort`. Entry/query tracking mirrors `ServerBrowser`
//! semantics in the local `ProtocolBrowser`: `qa_net`'s
//! `ServerBrowser<'a>` borrows its wire and transport, so one owner
//! cannot hold four protocol browsers plus a transport; packet codecs
//! and data types are still reused from `qa_net`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use qa_net::common::endpoint::{
    address_host, address_key, ip_address, ipv4_address, ipx_address, resolve_address, AddressError, NetworkAddress,
    ResolveFamily,
};
use qa_net::common::session::WireSelection;
use qa_net::common::transport::{
    monotonic_clock, Clock, DatagramTransport, ReceiveEvent, TransportError, UdpBindOptions, UdpTransport,
};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q1_net::{
    read_net_quake_discovery, read_quake_world_discovery, NetQuakeDiscoveryWire, QuakeWorldDiscoveryWire,
};
use qa_net::q2_kex_discovery::{kex_discovery_query, read_kex_discovery};
use qa_net::q2_net::{read_q2_master_reply, read_q2_out_of_band, read_q2_status, Q2DiscoveryWire, Q2Status};
use qa_net::q3_browser_view::{Q3BrowserCacheList, Q3BrowserCacheRow, Q3BrowserCacheView, Q3BrowserListState};
use qa_net::q3_net::Q3NetError;
use qa_net::services::discovery::{
    decode_q3_master_packet, decode_q3_server_status, BrowserEntry, DiscoveryError, DiscoveryRequestKind,
    DiscoverySource, DiscoveryWire, Q3DiscoveryWire, ServerStatus, Q3_MASTER_PROTOCOL,
};
use thiserror::Error;

use crate::bootstrap::server_browser_addresses::{
    direct_server_text, read_direct_servers, DirectServerAddress, MAXIMUM_DIRECT_SERVERS,
};
use crate::bootstrap::server_browser_cache::{read_q3_browser_cache, write_q3_browser_cache, ServerBrowserCacheError};
use crate::bootstrap::server_master_list::{fetch_server_master_list, MasterHttpResponse, MasterListError};
use crate::bootstrap::team_arena_scores::truncate_utf16;
use crate::settings::config::ConfigStore;
use crate::settings::json::{parse_json, stringify, Json};
use crate::settings::SettingsError;

/// Server browser failure.
#[derive(Debug, Error)]
pub enum ServerBrowserError {
    /// Address is not an IP address.
    #[error("Server requires an IP address")]
    NeedsIp,
    /// Q3 requires an IPv4 endpoint.
    #[error("Q3 browser requires an IPv4 endpoint")]
    NeedsIpv4,
    /// Browser is closed.
    #[error("Server browser is closed")]
    Closed,
    /// Unsupported server protocol.
    #[error("Unsupported server protocol")]
    BadServerProtocol,
    /// Unsupported sort order.
    #[error("Unsupported server sort order")]
    BadSortOrder,
    /// Q3 browser list is full.
    #[error("Q3 browser list is full")]
    ListFull,
    /// Q3 master query send failed.
    #[error("Could not send Q3 master query")]
    MasterSend,
    /// Master query send failed.
    #[error("Could not send master query")]
    Q2MasterSend,
    /// Server query send failed.
    #[error("Could not send server query")]
    QuerySend,
    /// Master address is required.
    #[error("Enter a master address or HTTP list URL")]
    NeedsMaster,
    /// Quake families need an HTTP master list.
    #[error("Use an HTTP master list for Quake or QuakeWorld")]
    NeedsHttpMaster,
    /// Saved server list is too large.
    #[error("Saved server list is too large")]
    SavedTooLarge,
    /// Invalid favorites save.
    #[error("Invalid favorites save")]
    BadFavorites,
    /// Invalid Q3 browser source (type-level in the donor).
    #[error("Invalid Q3 browser source")]
    BadQ3Source,
    /// Sibling address failure (message preserved).
    #[error("{0}")]
    Addresses(String),
    /// Discovery wire failure.
    #[error("{0}")]
    Discovery(#[from] DiscoveryError),
    /// Q3 codec failure.
    #[error("{0}")]
    Q3(#[from] Q3NetError),
    /// KEX codec failure.
    #[error("{0}")]
    Kex(String),
    /// Cache codec failure.
    #[error("{0}")]
    Cache(#[from] ServerBrowserCacheError),
    /// Master list failure.
    #[error("{0}")]
    MasterList(#[from] MasterListError),
    /// Settings failure.
    #[error("{0}")]
    Settings(#[from] SettingsError),
    /// Transport failure.
    #[error("{0}")]
    Transport(#[from] TransportError),
    /// Address failure.
    #[error("{0}")]
    Address(#[from] AddressError),
}

/// Browser protocol (donor `BrowserProtocol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserProtocol {
    /// NetQuake.
    Q1,
    /// QuakeWorld.
    Qw,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

impl BrowserProtocol {
    const ALL: [Self; 4] = [Self::Q1, Self::Qw, Self::Q2, Self::Q3];

    /// Protocol name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Q1 => "q1",
            Self::Qw => "qw",
            Self::Q2 => "q2",
            Self::Q3 => "q3",
        }
    }

    /// Default port.
    #[must_use]
    pub const fn default_port(self) -> u32 {
        match self {
            Self::Q1 => 26000,
            Self::Qw => 27500,
            Self::Q2 => 27910,
            Self::Q3 => 27960,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Q1 => 0,
            Self::Qw => 1,
            Self::Q2 => 2,
            Self::Q3 => 3,
        }
    }
}

/// Connection target (donor `BrowserConnection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserConnection {
    /// Protocol.
    pub protocol: BrowserProtocol,
    /// Remote text.
    pub remote: String,
    /// Q2 KEX marker when the selected entry negotiated KEX.
    pub q2_protocol: Option<ProtocolIdentity>,
}

/// Sort order (donor `BrowserSortOrder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserSortOrder {
    /// Ping: lowest first.
    PingLow,
    /// Ping: highest first.
    PingHigh,
    /// Name: A to Z.
    NameAz,
    /// Name: Z to A.
    NameZa,
    /// Map: A to Z.
    MapAz,
    /// Map: Z to A.
    MapZa,
    /// Players: most first.
    PlayersMost,
    /// Players: fewest first.
    PlayersFewest,
}

impl BrowserSortOrder {
    /// Sort id.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::PingLow => "ping-low",
            Self::PingHigh => "ping-high",
            Self::NameAz => "name-az",
            Self::NameZa => "name-za",
            Self::MapAz => "map-az",
            Self::MapZa => "map-za",
            Self::PlayersMost => "players-most",
            Self::PlayersFewest => "players-fewest",
        }
    }

    /// Sort label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PingLow => "Ping: lowest first",
            Self::PingHigh => "Ping: highest first",
            Self::NameAz => "Name: A to Z",
            Self::NameZa => "Name: Z to A",
            Self::MapAz => "Map: A to Z",
            Self::MapZa => "Map: Z to A",
            Self::PlayersMost => "Players: most first",
            Self::PlayersFewest => "Players: fewest first",
        }
    }
}

/// Sort choice (donor `browserSortOrders` entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserSortChoice {
    /// Id.
    pub id: BrowserSortOrder,
    /// Label.
    pub label: &'static str,
}

/// Available sort orders.
pub const BROWSER_SORT_ORDERS: [BrowserSortChoice; 8] = [
    BrowserSortChoice {
        id: BrowserSortOrder::PingLow,
        label: BrowserSortOrder::PingLow.label(),
    },
    BrowserSortChoice {
        id: BrowserSortOrder::PingHigh,
        label: BrowserSortOrder::PingHigh.label(),
    },
    BrowserSortChoice {
        id: BrowserSortOrder::NameAz,
        label: BrowserSortOrder::NameAz.label(),
    },
    BrowserSortChoice {
        id: BrowserSortOrder::NameZa,
        label: BrowserSortOrder::NameZa.label(),
    },
    BrowserSortChoice {
        id: BrowserSortOrder::MapAz,
        label: BrowserSortOrder::MapAz.label(),
    },
    BrowserSortChoice {
        id: BrowserSortOrder::MapZa,
        label: BrowserSortOrder::MapZa.label(),
    },
    BrowserSortChoice {
        id: BrowserSortOrder::PlayersMost,
        label: BrowserSortOrder::PlayersMost.label(),
    },
    BrowserSortChoice {
        id: BrowserSortOrder::PlayersFewest,
        label: BrowserSortOrder::PlayersFewest.label(),
    },
];

/// `host:port` text for an IP address.
pub fn browser_address(address: &NetworkAddress) -> Result<String, ServerBrowserError> {
    match address {
        NetworkAddress::Ipv4 { port, .. } => Ok(format!("{}:{port}", address_host(address))),
        NetworkAddress::Ipv6 { port, .. } => Ok(format!("[{}]:{port}", address_host(address))),
        _ => Err(ServerBrowserError::NeedsIp),
    }
}

/// Browser transport event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserEvent {
    /// Inbound datagram.
    Packet {
        /// Sender.
        from: NetworkAddress,
        /// Payload.
        payload: Vec<u8>,
    },
    /// Transport-level failure.
    Error {
        /// Message.
        message: String,
    },
}

/// Send/poll surface the browser needs (implemented by
/// `UdpTransport`; faked in tests).
pub trait BrowserTransport {
    /// Send a datagram; `false` reports a dropped send.
    fn send(&self, to: &NetworkAddress, bytes: &[u8]) -> bool;
    /// Pop the oldest event.
    fn poll(&self) -> Option<BrowserEvent>;
    /// Close the transport.
    fn close(&self);
}

impl BrowserTransport for UdpTransport {
    fn send(&self, to: &NetworkAddress, bytes: &[u8]) -> bool {
        if !to.is_ip() {
            return false;
        }
        DatagramTransport::send(self, to, bytes).unwrap_or(false)
    }

    fn poll(&self) -> Option<BrowserEvent> {
        loop {
            match DatagramTransport::poll(self) {
                Ok(Some(ReceiveEvent::Packet { from, payload, .. })) => {
                    return Some(BrowserEvent::Packet { from, payload });
                }
                Ok(Some(ReceiveEvent::Error { error })) => {
                    return Some(BrowserEvent::Error { message: error });
                }
                Ok(Some(ReceiveEvent::Dropped { .. })) => continue,
                Ok(None) => return None,
                Err(error) => {
                    return Some(BrowserEvent::Error {
                        message: error.to_string(),
                    })
                }
            }
        }
    }

    fn close(&self) {
        DatagramTransport::close(self);
    }
}

/// Minimal KEX mDNS port. The donor owns an async multicast socket;
/// sync callers inject an implementation (or none) via
/// `set_kex_discovery`.
pub trait KexMdnsPort {
    /// Broadcast a service query.
    fn query(&mut self);
    /// Drain resolved lobby endpoints.
    fn take_found(&mut self) -> Vec<NetworkAddress>;
    /// Drain failure messages.
    fn take_failed(&mut self) -> Vec<String>;
    /// Close the listener.
    fn close(&mut self);
}

fn q3_source(source: u8) -> Result<DiscoverySource, ServerBrowserError> {
    match source {
        0 => Ok(DiscoverySource::Lan),
        1 => Ok(DiscoverySource::SecondaryMaster),
        2 => Ok(DiscoverySource::Master),
        3 => Ok(DiscoverySource::Favorite),
        _ => Err(ServerBrowserError::BadQ3Source),
    }
}

fn checked_port(value: Option<&Json>) -> Result<u32, AddressError> {
    let Some(Json::Number(port)) = value else {
        return Err(AddressError::MissingPort);
    };
    if port.fract() != 0.0 || *port < 1.0 || *port > 65535.0 {
        return Err(AddressError::BadPort);
    }
    Ok(*port as u32)
}

fn member<'a>(fields: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    fields.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

/// Donor `parseNetworkAddress` over a JSON value.
fn parse_address_json(value: &Json) -> Result<NetworkAddress, AddressError> {
    let Json::Object(fields) = value else {
        return Err(AddressError::BadRecord);
    };
    let kind_value = member(fields, "kind");
    if kind_value.is_none() {
        return Err(AddressError::BadRecord);
    }
    let kind = match kind_value {
        Some(Json::String(kind)) => kind.as_str(),
        _ => "\0",
    };
    if kind == "loopback" {
        if let Some(Json::String(id)) = member(fields, "id") {
            if !id.is_empty() {
                return Ok(NetworkAddress::Loopback { id: id.clone() });
            }
        }
    }
    let port = checked_port(member(fields, "port"))?;
    match kind {
        "ipv6" => {
            let Some(Json::String(host)) = member(fields, "host") else {
                return Err(AddressError::BadFields);
            };
            let address = ip_address(host, port, false)?;
            if !matches!(address, NetworkAddress::Ipv6 { .. }) {
                return Err(AddressError::BadFields);
            }
            Ok(address)
        }
        "ipv4" => {
            let Some(Json::Array(bytes)) = member(fields, "host") else {
                return Err(AddressError::BadFields);
            };
            if bytes.len() != 4 {
                return Err(AddressError::BadFields);
            }
            let mut host = [0u8; 4];
            for (index, byte) in bytes.iter().enumerate() {
                let Json::Number(octet) = byte else {
                    return Err(AddressError::BadFields);
                };
                if octet.fract() != 0.0 || *octet < 0.0 || *octet > 255.0 {
                    return Err(AddressError::BadOctet);
                }
                host[index] = *octet as u8;
            }
            ipv4_address(host, port, false)
        }
        "ipx" => {
            let (Some(Json::Array(node)), Some(Json::Number(network))) =
                (member(fields, "node"), member(fields, "network"))
            else {
                return Err(AddressError::BadFields);
            };
            if node.len() != 6 {
                return Err(AddressError::BadFields);
            }
            let mut bytes = [0u8; 6];
            for (index, byte) in node.iter().enumerate() {
                let Json::Number(octet) = byte else {
                    return Err(AddressError::BadFields);
                };
                if octet.fract() != 0.0 || *octet < 0.0 || *octet > 255.0 {
                    return Err(AddressError::BadIpx);
                }
                bytes[index] = *octet as u8;
            }
            if network.fract() != 0.0 || *network < 0.0 || *network > 4_294_967_295.0 {
                return Err(AddressError::BadIpx);
            }
            ipx_address(*network as u32, bytes, port)
        }
        _ => Err(AddressError::BadFields),
    }
}

fn address_record_json(address: &NetworkAddress) -> Json {
    match address {
        NetworkAddress::Ipv4 { host, port } => Json::Object(vec![
            ("kind".to_owned(), Json::String("ipv4".to_owned())),
            (
                "host".to_owned(),
                Json::Array(host.iter().map(|byte| Json::Number(f64::from(*byte))).collect()),
            ),
            ("port".to_owned(), Json::Number(f64::from(*port))),
        ]),
        NetworkAddress::Ipv6 { host, port } => Json::Object(vec![
            ("kind".to_owned(), Json::String("ipv6".to_owned())),
            ("host".to_owned(), Json::String(host.clone())),
            ("port".to_owned(), Json::Number(f64::from(*port))),
        ]),
        NetworkAddress::Loopback { id } => Json::Object(vec![
            ("kind".to_owned(), Json::String("loopback".to_owned())),
            ("id".to_owned(), Json::String(id.clone())),
        ]),
        NetworkAddress::Ipx { network, node, port } => Json::Object(vec![
            ("kind".to_owned(), Json::String("ipx".to_owned())),
            ("network".to_owned(), Json::Number(f64::from(*network))),
            (
                "node".to_owned(),
                Json::Array(node.iter().map(|byte| Json::Number(f64::from(*byte))).collect()),
            ),
            ("port".to_owned(), Json::Number(f64::from(*port))),
        ]),
    }
}

fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if (c as u32) < 0x20 || (c as u32) == 0x7f {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// Q2 wire that always sends `status` queries (donor override).
struct Q2StatusWire(Q2DiscoveryWire);

impl DiscoveryWire for Q2StatusWire {
    fn query(&self, _kind: DiscoveryRequestKind, challenge: &str) -> Result<Vec<u8>, DiscoveryError> {
        self.0.query(DiscoveryRequestKind::Status, challenge)
    }

    fn master_query(&self) -> Result<Vec<u8>, DiscoveryError> {
        self.0.master_query()
    }

    fn heartbeat(&self, active: bool) -> Result<Vec<u8>, DiscoveryError> {
        self.0.heartbeat(active)
    }
}

#[derive(Debug, Clone)]
struct PendingQuery {
    handle: u64,
    challenge: String,
    address: NetworkAddress,
    request_kind: DiscoveryRequestKind,
    sent_at: f64,
}

#[derive(Debug, Clone)]
struct PendingBroadcast {
    sequence: u64,
    sent_at: f64,
}

/// One protocol's entry/query table with `ServerBrowser` semantics.
#[derive(Clone)]
struct ProtocolBrowser {
    wire: Rc<dyn DiscoveryWire>,
    transport: Rc<dyn BrowserTransport>,
    entries: Vec<BrowserEntry>,
    queries: HashMap<u64, PendingQuery>,
    broadcasts: HashMap<String, PendingBroadcast>,
    query_sequence: u64,
    next_handle: u64,
}

impl ProtocolBrowser {
    fn new(wire: Rc<dyn DiscoveryWire>, transport: Rc<dyn BrowserTransport>) -> Self {
        Self {
            wire,
            transport,
            entries: Vec::new(),
            queries: HashMap::new(),
            broadcasts: HashMap::new(),
            query_sequence: 0,
            next_handle: 0,
        }
    }

    fn position(&self, address: &NetworkAddress) -> Option<usize> {
        let key = address_key(address, true);
        self.entries
            .iter()
            .position(|entry| address_key(&entry.address, true) == key)
    }

    fn add(&mut self, address: NetworkAddress, source: DiscoverySource, now: f64) -> BrowserEntry {
        if let Some(index) = self.position(&address) {
            if !self.entries[index].sources.contains(&source) {
                self.entries[index].sources.push(source);
            }
            return self.entries[index].clone();
        }
        let entry = BrowserEntry {
            address,
            sources: vec![source],
            status: None,
            ping_milliseconds: None,
            updated_at: now,
        };
        self.entries.push(entry.clone());
        entry
    }

    fn remove_source(&mut self, address: &NetworkAddress, source: DiscoverySource) {
        let Some(index) = self.position(address) else { return };
        self.entries[index].sources.retain(|candidate| *candidate != source);
        if self.entries[index].sources.is_empty() {
            self.entries.remove(index);
        }
    }

    fn remove_favorite(&mut self, address: &NetworkAddress) {
        self.remove_source(address, DiscoverySource::Favorite);
    }

    fn restore_entry(&mut self, entry: BrowserEntry) {
        let key = address_key(&entry.address, true);
        let Some(index) = self
            .entries
            .iter()
            .position(|candidate| address_key(&candidate.address, true) == key)
        else {
            self.entries.push(entry);
            return;
        };
        let mut sources = self.entries[index].sources.clone();
        for source in &entry.sources {
            if !sources.contains(source) {
                sources.push(*source);
            }
        }
        let mut updated = if self.entries[index].status.is_none() {
            entry
        } else {
            self.entries[index].clone()
        };
        updated.sources = sources;
        self.entries[index] = updated;
    }

    fn entry(&self, address: &NetworkAddress) -> Option<&BrowserEntry> {
        self.position(address).map(|index| &self.entries[index])
    }

    fn list(&self) -> Vec<&BrowserEntry> {
        self.entries.iter().collect()
    }

    fn list_owned(&self) -> Vec<BrowserEntry> {
        self.entries.clone()
    }

    fn size(&self) -> usize {
        self.entries.len()
    }

    fn pending_requests(&self) -> usize {
        self.queries.len()
    }

    fn query(
        &mut self,
        address: &NetworkAddress,
        now: f64,
        kind: DiscoveryRequestKind,
    ) -> Result<bool, ServerBrowserError> {
        let key = address_key(address, true);
        let replaced: Vec<u64> = self
            .queries
            .values()
            .filter(|pending| pending.request_kind == kind && address_key(&pending.address, true) == key)
            .map(|pending| pending.handle)
            .collect();
        for handle in replaced {
            self.queries.remove(&handle);
        }
        self.next_handle += 1;
        self.query_sequence += 1;
        let handle = self.next_handle;
        let challenge = self.query_sequence.to_string();
        self.queries.insert(
            handle,
            PendingQuery {
                handle,
                challenge: challenge.clone(),
                address: address.clone(),
                request_kind: kind,
                sent_at: now,
            },
        );
        let bytes = match self.wire.query(kind, &challenge) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.queries.remove(&handle);
                return Err(error.into());
            }
        };
        if self.transport.send(address, &bytes) {
            Ok(true)
        } else {
            self.queries.remove(&handle);
            Ok(false)
        }
    }

    fn query_master(&mut self, address: &NetworkAddress) -> Result<bool, ServerBrowserError> {
        Ok(self.transport.send(address, &self.wire.master_query()?))
    }

    fn receive(
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
            .as_ref()
            .map(|query| query.sent_at)
            .or_else(|| broadcast.as_ref().map(|broadcast| broadcast.sent_at));
        let Some(sent_at) = pending else { return false };
        let ping_milliseconds = 0f64.max(now - sent_at);
        if direct.is_none() {
            self.add(address.clone(), DiscoverySource::Lan, now);
        } else if self.entry(address).is_none() {
            self.add(address.clone(), DiscoverySource::Direct, now);
        }
        if let Some(index) = self.position(address) {
            self.entries[index].status = Some(status);
            self.entries[index].ping_milliseconds = Some(ping_milliseconds);
            self.entries[index].updated_at = now;
        }
        if let Some(direct) = direct {
            self.queries.remove(&direct.handle);
        }
        true
    }

    fn broadcast(&mut self, addresses: &[NetworkAddress], now: f64) -> Result<usize, ServerBrowserError> {
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

    fn expire_queries(&mut self, now: f64, timeout_milliseconds: f64) -> Vec<NetworkAddress> {
        self.broadcasts
            .retain(|_, query| now - query.sent_at < timeout_milliseconds);
        let handles: Vec<u64> = self
            .queries
            .values()
            .filter(|query| now - query.sent_at >= timeout_milliseconds)
            .map(|query| query.handle)
            .collect();
        let mut expired = Vec::new();
        for handle in handles {
            let Some(query) = self.queries.get(&handle).cloned() else {
                continue;
            };
            if let Some(entry) = self.entry(&query.address) {
                if !expired
                    .iter()
                    .any(|address: &NetworkAddress| address_key(address, true) == address_key(&entry.address, true))
                {
                    expired.push(entry.address.clone());
                }
            }
            self.queries.remove(&handle);
        }
        expired
    }

    fn favorite_addresses(&self) -> Vec<NetworkAddress> {
        self.entries
            .iter()
            .filter(|entry| entry.sources.contains(&DiscoverySource::Favorite))
            .map(|entry| entry.address.clone())
            .collect()
    }

    fn save_favorites(&self) -> String {
        stringify(&Json::Array(
            self.favorite_addresses().iter().map(address_record_json).collect(),
        ))
    }

    fn restore_favorites(&mut self, text: &str) -> Result<(), ServerBrowserError> {
        let value = parse_json(text)?;
        let Json::Array(entries) = value else {
            return Err(ServerBrowserError::BadFavorites);
        };
        let mut addresses = Vec::with_capacity(entries.len());
        for entry in &entries {
            addresses.push(parse_address_json(entry)?);
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

#[derive(Debug, Clone)]
struct Q3MasterQuery {
    source: u8,
    address: Option<NetworkAddress>,
    started_at: f64,
    received: bool,
}

#[derive(Debug, Clone)]
struct Q2MasterQuery {
    address: NetworkAddress,
    started_at: f64,
}

#[derive(Debug, Clone)]
struct PendingStatusQuery {
    protocol: BrowserProtocol,
    address: NetworkAddress,
}

#[derive(Debug, Clone)]
struct ListSnapshot {
    generation: i32,
    addresses: Vec<NetworkAddress>,
}

/// Master-list HTTP supplier.
pub type MasterFetch = Box<dyn FnMut(&str) -> Result<MasterHttpResponse, MasterListError>>;

/// Startup server browser (donor `StartupServerBrowser`).
pub struct StartupServerBrowser {
    transport: Rc<dyn BrowserTransport>,
    config: ConfigStore,
    now: Clock,
    cores: [ProtocolBrowser; 4],
    direct_servers: HashMap<BrowserProtocol, Vec<DirectServerAddress>>,
    addresses: HashMap<BrowserProtocol, String>,
    master_addresses: HashMap<BrowserProtocol, String>,
    closing: bool,
    closed: bool,
    list_generations: HashMap<u8, i32>,
    list_snapshots: HashMap<u8, ListSnapshot>,
    list_resets: HashMap<u8, i32>,
    master: Option<Q3MasterQuery>,
    cached_view: Option<Q3BrowserCacheView>,
    q2_master: Option<Q2MasterQuery>,
    kex_discovery: Option<Box<dyn KexMdnsPort>>,
    kex_queries: HashMap<String, f64>,
    pending_status: BTreeMap<String, PendingStatusQuery>,
    master_fetch: MasterFetch,
    /// Active protocol.
    pub protocol: BrowserProtocol,
    /// Filter text.
    pub filter: String,
    /// Favorites only.
    pub favorites_only: bool,
    /// Sort order.
    pub sort_order: BrowserSortOrder,
    /// Hide empty servers.
    pub hide_empty: bool,
    /// Hide full servers.
    pub hide_full: bool,
    /// Selected address key.
    pub selected: Option<String>,
    /// Status line.
    pub status: String,
}

impl StartupServerBrowser {
    fn cores_for(transport: &Rc<dyn BrowserTransport>) -> [ProtocolBrowser; 4] {
        [
            ProtocolBrowser::new(Rc::new(NetQuakeDiscoveryWire), transport.clone()),
            ProtocolBrowser::new(Rc::new(QuakeWorldDiscoveryWire), transport.clone()),
            ProtocolBrowser::new(
                Rc::new(Q2StatusWire(Q2DiscoveryWire::new(ProtocolIdentity::Q2Classic, || {
                    Q2Status {
                        server_info: String::new(),
                        players: Vec::new(),
                    }
                }))),
                transport.clone(),
            ),
            ProtocolBrowser::new(Rc::new(Q3DiscoveryWire::default()), transport.clone()),
        ]
    }

    fn with_transport(config: ConfigStore, transport: Rc<dyn BrowserTransport>, now: Clock) -> Self {
        let cores = Self::cores_for(&transport);
        Self {
            transport,
            config,
            now,
            cores,
            direct_servers: HashMap::new(),
            addresses: HashMap::new(),
            master_addresses: HashMap::new(),
            closing: false,
            closed: false,
            list_generations: HashMap::new(),
            list_snapshots: HashMap::new(),
            list_resets: HashMap::new(),
            master: None,
            cached_view: None,
            q2_master: None,
            kex_discovery: None,
            kex_queries: HashMap::new(),
            pending_status: BTreeMap::new(),
            master_fetch: Box::new(|_| {
                Err(MasterListError::Fetch(
                    "master list HTTP fetch is not configured".to_owned(),
                ))
            }),
            protocol: BrowserProtocol::Q1,
            filter: String::new(),
            favorites_only: false,
            sort_order: BrowserSortOrder::PingLow,
            hide_empty: false,
            hide_full: false,
            selected: None,
            status: String::new(),
        }
    }

    /// Open a browser, binding a UDP transport and loading saved state.
    pub fn open(config: ConfigStore) -> Result<Self, ServerBrowserError> {
        let mut options = UdpBindOptions::new("0.0.0.0", 0);
        options.broadcast = true;
        let transport: Rc<dyn BrowserTransport> = Rc::new(UdpTransport::bind(&options)?);
        Self::open_with_transport(config, transport, monotonic_clock())
    }

    /// Open with an injected transport and clock (tests).
    pub fn open_with_transport(
        config: ConfigStore,
        transport: Rc<dyn BrowserTransport>,
        now: Clock,
    ) -> Result<Self, ServerBrowserError> {
        let mut browser = Self::with_transport(config, transport, now);
        match browser.load_saved_state() {
            Ok(()) => Ok(browser),
            Err(error) => {
                browser.transport.close();
                Err(error)
            }
        }
    }

    /// Inject the master-list HTTP supplier.
    pub fn set_master_fetch(&mut self, fetch: MasterFetch) {
        self.master_fetch = fetch;
    }

    /// Inject the KEX mDNS listener.
    pub fn set_kex_discovery(&mut self, kex: Box<dyn KexMdnsPort>) {
        self.kex_discovery = Some(kex);
    }

    fn load_saved_state(&mut self) -> Result<(), ServerBrowserError> {
        for protocol in BrowserProtocol::ALL {
            let name = format!("servers-{}", protocol.as_str());
            if let Some(saved) = self.config.load_text(&name)? {
                if saved.len() > 65536 {
                    return Err(ServerBrowserError::SavedTooLarge);
                }
                let core = &mut self.cores[protocol.index()];
                core.restore_favorites(&saved)?;
                for address in core.favorite_addresses() {
                    browser_address(&address)?;
                }
            }
        }
        for protocol in BrowserProtocol::ALL {
            let name = format!("servers-master-{}", protocol.as_str());
            if let Some(master) = self.config.load_text(&name)? {
                let trimmed = master.trim().to_owned();
                if trimmed.encode_utf16().count() <= 2048 {
                    self.master_addresses.insert(protocol, trimmed);
                }
            }
            let name = format!("servers-direct-{}", protocol.as_str());
            let Some(saved) = self.config.load_text(&name)? else {
                continue;
            };
            let servers =
                read_direct_servers(&saved).map_err(|error| ServerBrowserError::Addresses(error.to_string()))?;
            for server in &servers {
                self.cores[protocol.index()].add(server.address.clone(), DiscoverySource::Direct, 0.0);
            }
            if let Some(last) = servers.first() {
                self.addresses.insert(protocol, last.remote.clone());
            }
            self.direct_servers.insert(protocol, servers);
        }
        self.load_q3_cache()?;
        Ok(())
    }

    /// Master address for the active protocol.
    #[must_use]
    pub fn master_address(&self) -> String {
        self.master_addresses.get(&self.protocol).cloned().unwrap_or_default()
    }

    /// Set the master address for the active protocol.
    pub fn set_master_address(&mut self, value: String) {
        self.master_addresses.insert(self.protocol, value);
    }

    /// Connect address for the active protocol.
    #[must_use]
    pub fn address(&self) -> String {
        self.addresses
            .get(&self.protocol)
            .cloned()
            .unwrap_or_else(|| format!("localhost:{}", self.protocol.default_port()))
    }

    /// Set the connect address for the active protocol.
    pub fn set_address(&mut self, value: String) {
        self.addresses.insert(self.protocol, value);
    }

    /// Fail when closed.
    pub fn assert_open(&self) -> Result<(), ServerBrowserError> {
        if self.closing || self.closed {
            return Err(ServerBrowserError::Closed);
        }
        Ok(())
    }

    /// Q3 list state for a source.
    pub fn q3_list(&mut self, source: u8) -> Result<Q3BrowserListState, ServerBrowserError> {
        self.assert_open()?;
        let membership = q3_source(source)?;
        let generation = self.list_generations.get(&source).copied().unwrap_or(0);
        let stale = self
            .list_snapshots
            .get(&source)
            .is_none_or(|snapshot| snapshot.generation != generation);
        if stale {
            let addresses = self.cores[BrowserProtocol::Q3.index()]
                .list()
                .iter()
                .filter(|entry| entry.sources.contains(&membership))
                .map(|entry| entry.address.clone())
                .collect();
            self.list_snapshots
                .insert(source, ListSnapshot { generation, addresses });
        }
        let addresses = self
            .list_snapshots
            .get(&source)
            .map(|snapshot| snapshot.addresses.clone())
            .unwrap_or_default();
        Ok(Q3BrowserListState {
            addresses,
            pending: self
                .master
                .as_ref()
                .is_some_and(|master| master.source == source && !master.received),
            generation,
            reset_generation: self.list_resets.get(&source).copied().unwrap_or(0),
        })
    }

    fn clear_q3(&mut self, source: u8) {
        let Ok(membership) = q3_source(source) else { return };
        let core = &mut self.cores[BrowserProtocol::Q3.index()];
        for entry in core.list_owned() {
            core.remove_source(&entry.address, membership);
        }
        *self.list_generations.entry(source).or_insert(0) = self
            .list_generations
            .get(&source)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        *self.list_resets.entry(source).or_insert(0) =
            self.list_resets.get(&source).copied().unwrap_or(0).saturating_add(1);
    }

    fn bump_q3_generation(&mut self, source: u8) {
        let next = self
            .list_generations
            .get(&source)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        self.list_generations.insert(source, next);
    }

    /// Resolve Q3 server text, or `None` when unusable.
    pub fn resolve_q3(&mut self, text: &str) -> Result<Option<NetworkAddress>, ServerBrowserError> {
        self.assert_open()?;
        let cleaned = match direct_server_text(text) {
            Ok(cleaned) => cleaned,
            Err(_) => return Ok(None),
        };
        let address = match resolve_address(&cleaned, 27960, ResolveFamily::V4) {
            Ok(address) => address,
            Err(_) => return Ok(None),
        };
        match &address {
            NetworkAddress::Ipv4 { host, port } if *port != 0 && *host != [255, 255, 255, 255] => Ok(Some(address)),
            _ => Ok(None),
        }
    }

    /// Add an address to a Q3 list.
    pub fn add_q3(&mut self, source: u8, address: &NetworkAddress) -> Result<(), ServerBrowserError> {
        self.assert_open()?;
        if !matches!(address, NetworkAddress::Ipv4 { port, .. } if *port != 0) {
            return Err(ServerBrowserError::NeedsIpv4);
        }
        let membership = q3_source(source)?;
        let key = address_key(address, true);
        let core = &mut self.cores[BrowserProtocol::Q3.index()];
        let listed: Vec<BrowserEntry> = core
            .list()
            .iter()
            .filter(|entry| entry.sources.contains(&membership))
            .map(|entry| (*entry).clone())
            .collect();
        if listed.iter().any(|entry| address_key(&entry.address, true) == key) {
            return Ok(());
        }
        if listed.len() >= if source == 2 { 8192 } else { 128 } {
            return Err(ServerBrowserError::ListFull);
        }
        core.add(address.clone(), membership, 0.0);
        self.bump_q3_generation(source);
        Ok(())
    }

    /// Remove an address from a Q3 list.
    pub fn remove_q3(&mut self, source: u8, address: &NetworkAddress) -> Result<(), ServerBrowserError> {
        self.assert_open()?;
        let membership = q3_source(source)?;
        self.cores[BrowserProtocol::Q3.index()].remove_source(address, membership);
        self.bump_q3_generation(source);
        Ok(())
    }

    /// Broadcast a Q3 LAN scan.
    pub fn scan_q3(&mut self) -> Result<(), ServerBrowserError> {
        self.assert_open()?;
        self.clear_q3(0);
        let now = (self.now)();
        let targets = [
            ipv4_address([255, 255, 255, 255], 27960, false)?,
            ipv4_address([255, 255, 255, 255], 27961, false)?,
            ipv4_address([255, 255, 255, 255], 27962, false)?,
            ipv4_address([255, 255, 255, 255], 27963, false)?,
        ];
        self.cores[BrowserProtocol::Q3.index()].broadcast(&targets, now)?;
        Ok(())
    }

    /// Query a Q3 master server.
    pub fn request_q3_master(
        &mut self,
        source: u8,
        remote: &str,
        protocol: i32,
        keywords: &[String],
    ) -> Result<(), ServerBrowserError> {
        self.assert_open()?;
        if source != 1 && source != 2 {
            return Err(ServerBrowserError::BadQ3Source);
        }
        let bytes = Q3DiscoveryWire::new(keywords, protocol)?.master_query()?;
        self.clear_q3(source);
        let now = (self.now)();
        self.master = Some(Q3MasterQuery {
            source,
            address: None,
            started_at: now,
            received: false,
        });
        let cleaned = direct_server_text(remote).map_err(|error| ServerBrowserError::Addresses(error.to_string()))?;
        let address = match resolve_address(&cleaned, 27950, ResolveFamily::V4) {
            Ok(address) => address,
            Err(error) => {
                self.master = None;
                return Err(error.into());
            }
        };
        let now = (self.now)();
        self.master = Some(Q3MasterQuery {
            source,
            address: Some(address.clone()),
            started_at: now,
            received: false,
        });
        if !self.transport.send(&address, &bytes) {
            self.master = None;
            return Err(ServerBrowserError::MasterSend);
        }
        Ok(())
    }

    /// Load the Q3 cache, preserving live statuses.
    pub fn load_q3_cache(&mut self) -> Result<Option<Q3BrowserCacheView>, ServerBrowserError> {
        let Some(saved) = self.config.load_text("servers-cache-q3")? else {
            return Ok(None);
        };
        let cache = read_q3_browser_cache(&saved)?;
        let live: HashMap<String, BrowserEntry> = self.cores[BrowserProtocol::Q3.index()]
            .list()
            .iter()
            .filter(|entry| entry.status.is_some())
            .map(|entry| (address_key(&entry.address, true), (*entry).clone()))
            .collect();
        for source in [1u8, 2, 3] {
            self.clear_q3(source);
        }
        self.master = None;
        for entry in &cache.entries {
            let key = address_key(&entry.address, true);
            let merged = match live.get(&key) {
                None => entry.clone(),
                Some(current) => BrowserEntry {
                    status: current.status.clone(),
                    ping_milliseconds: current.ping_milliseconds,
                    updated_at: current.updated_at,
                    ..entry.clone()
                },
            };
            self.cores[BrowserProtocol::Q3.index()].restore_entry(merged);
        }
        self.cached_view = cache.view.clone();
        Ok(cache.view)
    }

    /// Save the Q3 cache.
    pub fn save_q3_cache(&mut self, view: &Q3BrowserCacheView) -> Result<(), ServerBrowserError> {
        let serialized = write_q3_browser_cache(&self.cores[BrowserProtocol::Q3.index()].list_owned(), Some(view))?;
        self.config.dump("servers-cache-q3", &serialized)?;
        self.cached_view = Some(view.clone());
        Ok(())
    }

    fn current_cache_view(&self) -> Option<Q3BrowserCacheView> {
        let cached = self.cached_view.as_ref()?;
        let core = &self.cores[BrowserProtocol::Q3.index()];
        let mut lists = Vec::with_capacity(cached.lists.len());
        for list in &cached.lists {
            let Ok(membership) = q3_source(list.source) else {
                continue;
            };
            let entries: Vec<&BrowserEntry> = core
                .list()
                .into_iter()
                .filter(|entry| entry.sources.contains(&membership))
                .collect();
            let keys: HashSet<String> = entries.iter().map(|entry| address_key(&entry.address, true)).collect();
            let mut rows: Vec<Q3BrowserCacheRow> = list
                .rows
                .iter()
                .filter(|row| keys.contains(&address_key(&row.address, true)))
                .cloned()
                .collect();
            let mut retained: HashSet<String> = rows.iter().map(|row| address_key(&row.address, true)).collect();
            let capacity = if list.source == 2 { 4096 } else { 128 };
            for entry in entries {
                if rows.len() >= capacity {
                    break;
                }
                if retained.insert(address_key(&entry.address, true)) {
                    rows.push(Q3BrowserCacheRow {
                        address: entry.address.clone(),
                        name: truncate_utf16(entry.status.as_ref().map_or("", |status| status.name.as_str()), 31)
                            .to_owned(),
                        visible: 1,
                        ping: -1,
                    });
                }
            }
            lists.push(Q3BrowserCacheList {
                source: list.source,
                rows,
            });
        }
        Some(Q3BrowserCacheView { lists })
    }

    /// Choose the active protocol.
    pub fn choose(&mut self, protocol: &str) -> Result<(), ServerBrowserError> {
        self.protocol = match protocol {
            "q1" => BrowserProtocol::Q1,
            "qw" => BrowserProtocol::Qw,
            "q2" => BrowserProtocol::Q2,
            "q3" => BrowserProtocol::Q3,
            _ => return Err(ServerBrowserError::BadServerProtocol),
        };
        self.selected = None;
        Ok(())
    }

    /// Choose the sort order.
    pub fn choose_sort(&mut self, order: &str) -> Result<(), ServerBrowserError> {
        let Some(choice) = BROWSER_SORT_ORDERS.iter().find(|choice| choice.id.id() == order) else {
            return Err(ServerBrowserError::BadSortOrder);
        };
        self.sort_order = choice.id;
        Ok(())
    }

    /// Selected entry, if any.
    #[must_use]
    pub fn selected_entry(&self) -> Option<BrowserEntry> {
        let selected = self.selected.as_ref()?;
        self.cores[self.protocol.index()]
            .list()
            .iter()
            .find(|entry| address_key(&entry.address, true) == *selected)
            .map(|entry| (*entry).clone())
    }

    /// Detail lines for the selected entry.
    pub fn details(&self) -> Result<Vec<String>, ServerBrowserError> {
        let Some(entry) = self.selected_entry() else {
            return Ok(vec!["Select a server first.".to_owned()]);
        };
        let Some(status) = &entry.status else {
            return Ok(vec![
                browser_address(&entry.address)?,
                "No status response yet.".to_owned(),
            ]);
        };
        let mut lines = vec![
            clean_text(&status.name),
            browser_address(&entry.address)?,
            format!(
                "Map: {}  Players: {}/{}",
                clean_text(&status.map),
                status.players,
                status.max_players
            ),
        ];
        for player in &status.player_details {
            lines.push(format!(
                "{}  score {}  ping {}",
                clean_text(&player.name),
                player.score,
                player.ping
            ));
        }
        let mut rules: Vec<(&String, &String)> = status.rules.iter().collect();
        rules.sort_by_key(|rule| rule.0);
        for (key, value) in rules {
            lines.push(format!("{}: {}", clean_text(key), clean_text(value)));
        }
        Ok(lines)
    }

    fn queue_status(&mut self, protocol: BrowserProtocol, address: &NetworkAddress) {
        let key = format!("{}:{}", protocol.as_str(), address_key(address, true));
        self.pending_status.insert(
            key,
            PendingStatusQuery {
                protocol,
                address: address.clone(),
            },
        );
    }

    /// Discover servers via the master address or HTTP list URL.
    pub fn discover(&mut self) -> Result<(), ServerBrowserError> {
        self.assert_open()?;
        let protocol = self.protocol;
        let remote = self.master_address().trim().to_owned();
        if remote.is_empty() {
            return Err(ServerBrowserError::NeedsMaster);
        }
        let lower = remote.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            let names = fetch_server_master_list(&remote, &mut self.master_fetch)?;
            let mut addresses = Vec::with_capacity(names.len());
            for name in &names {
                addresses.push(resolve_address(name, protocol.default_port(), ResolveFamily::V4)?);
            }
            for address in &addresses {
                let known = self.cores[protocol.index()].entry(address).is_some();
                if self.cores[protocol.index()].size() >= 4096 && !known {
                    break;
                }
                if protocol == BrowserProtocol::Q3 {
                    self.add_q3(2, address)?;
                } else {
                    self.cores[protocol.index()].add(address.clone(), DiscoverySource::Master, 0.0);
                }
                self.queue_status(protocol, address);
            }
            self.status = format!("Found {} servers; querying status", addresses.len());
        } else if protocol == BrowserProtocol::Q2 {
            let stripped = remote.strip_prefix("udp://").unwrap_or(&remote);
            let cleaned =
                direct_server_text(stripped).map_err(|error| ServerBrowserError::Addresses(error.to_string()))?;
            let address = resolve_address(&cleaned, 27900, ResolveFamily::V4)?;
            let now = (self.now)();
            self.q2_master = Some(Q2MasterQuery {
                address: address.clone(),
                started_at: now,
            });
            if !self.cores[protocol.index()].query_master(&address)? {
                return Err(ServerBrowserError::Q2MasterSend);
            }
            self.status = "Querying master...".to_owned();
        } else if protocol == BrowserProtocol::Q3 {
            let stripped = remote.strip_prefix("udp://").unwrap_or(&remote).to_owned();
            self.request_q3_master(
                2,
                &stripped,
                Q3_MASTER_PROTOCOL,
                &["empty".to_owned(), "full".to_owned()],
            )?;
            self.status = "Querying master...".to_owned();
        } else {
            return Err(ServerBrowserError::NeedsHttpMaster);
        }
        self.config
            .dump(&format!("servers-master-{}", protocol.as_str()), &remote)?;
        Ok(())
    }

    /// Empty-list message.
    #[must_use]
    pub fn empty_message(&self) -> String {
        if self.cores[self.protocol.index()].list().is_empty() {
            "No servers yet. Enter an address or find LAN.".to_owned()
        } else {
            "No servers match these filters.".to_owned()
        }
    }

    /// Filtered and sorted rows.
    pub fn rows(&self) -> Result<Vec<BrowserEntry>, ServerBrowserError> {
        let search = self.filter.trim().to_lowercase();
        let mut rows: Vec<(BrowserEntry, String)> = Vec::new();
        for entry in self.cores[self.protocol.index()].list_owned() {
            if self.favorites_only && !entry.sources.contains(&DiscoverySource::Favorite) {
                continue;
            }
            if let Some(status) = &entry.status {
                if status.max_players > 0 {
                    if self.hide_empty && status.players <= 0 {
                        continue;
                    }
                    if self.hide_full && status.players >= status.max_players {
                        continue;
                    }
                }
            }
            let remote = browser_address(&entry.address)?;
            let haystack = format!(
                "{} {} {} {} {remote}",
                entry.status.as_ref().map_or("", |status| status.name.as_str()),
                entry.status.as_ref().map_or("", |status| status.map.as_str()),
                entry.status.as_ref().map_or("", |status| {
                    status
                        .rules
                        .get("gamedir")
                        .or_else(|| status.rules.get("game"))
                        .map_or("", String::as_str)
                }),
                entry.status.as_ref().map_or(String::new(), |status| {
                    status
                        .player_details
                        .iter()
                        .map(|player| {
                            let mut name = player.name.clone();
                            while let Some(caret) = name.find('^') {
                                let strip = if name.as_bytes().get(caret + 1).is_some_and(u8::is_ascii_digit) {
                                    2
                                } else {
                                    1
                                };
                                name.replace_range(caret..caret + strip, "");
                            }
                            name
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
            );
            if !haystack.to_lowercase().contains(&search) {
                continue;
            }
            rows.push((entry, remote));
        }
        let sort = self.sort_order;
        rows.sort_by(|(a, a_remote), (b, b_remote)| {
            match (a.status.is_none(), b.status.is_none()) {
                (true, false) => return std::cmp::Ordering::Greater,
                (false, true) => return std::cmp::Ordering::Less,
                _ => {}
            }
            let compared = match sort {
                BrowserSortOrder::PingLow | BrowserSortOrder::PingHigh => {
                    match (a.ping_milliseconds.is_none(), b.ping_milliseconds.is_none()) {
                        (true, false) => return std::cmp::Ordering::Greater,
                        (false, true) => return std::cmp::Ordering::Less,
                        _ => {}
                    }
                    let ordering = (a.ping_milliseconds.unwrap_or(0.0))
                        .partial_cmp(&b.ping_milliseconds.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal);
                    if sort == BrowserSortOrder::PingLow {
                        ordering
                    } else {
                        ordering.reverse()
                    }
                }
                BrowserSortOrder::NameAz | BrowserSortOrder::NameZa => {
                    let ordering = a
                        .status
                        .as_ref()
                        .map_or("", |status| status.name.as_str())
                        .to_lowercase()
                        .cmp(
                            &b.status
                                .as_ref()
                                .map_or("", |status| status.name.as_str())
                                .to_lowercase(),
                        );
                    if sort == BrowserSortOrder::NameAz {
                        ordering
                    } else {
                        ordering.reverse()
                    }
                }
                BrowserSortOrder::MapAz | BrowserSortOrder::MapZa => {
                    let ordering = a
                        .status
                        .as_ref()
                        .map_or("", |status| status.map.as_str())
                        .to_lowercase()
                        .cmp(
                            &b.status
                                .as_ref()
                                .map_or("", |status| status.map.as_str())
                                .to_lowercase(),
                        );
                    if sort == BrowserSortOrder::MapAz {
                        ordering
                    } else {
                        ordering.reverse()
                    }
                }
                BrowserSortOrder::PlayersMost | BrowserSortOrder::PlayersFewest => {
                    let ordering = a
                        .status
                        .as_ref()
                        .map_or(0, |status| status.players)
                        .cmp(&b.status.as_ref().map_or(0, |status| status.players));
                    if sort == BrowserSortOrder::PlayersFewest {
                        ordering
                    } else {
                        ordering.reverse()
                    }
                }
            };
            compared.then_with(|| a_remote.cmp(b_remote))
        });
        Ok(rows.into_iter().map(|(entry, _)| entry).collect())
    }

    /// Select a row by address key.
    pub fn select(&mut self, key: &str) -> Result<(), ServerBrowserError> {
        let Some(entry) = self
            .rows()?
            .into_iter()
            .find(|entry| address_key(&entry.address, true) == key)
        else {
            return Ok(());
        };
        let direct = if entry.sources.contains(&DiscoverySource::Direct) {
            self.direct_servers
                .get(&self.protocol)
                .and_then(|servers| servers.iter().find(|server| address_key(&server.address, true) == key))
        } else {
            None
        };
        let remote = direct.map_or_else(|| browser_address(&entry.address), |server| Ok(server.remote.clone()))?;
        self.selected = Some(key.to_owned());
        self.addresses.insert(self.protocol, remote);
        Ok(())
    }

    /// Query the connect address.
    pub fn query(&mut self) -> Result<(), ServerBrowserError> {
        let protocol = self.protocol;
        let remote = direct_server_text(self.address().trim())
            .map_err(|error| ServerBrowserError::Addresses(error.to_string()))?;
        let address = resolve_address(&remote, protocol.default_port(), ResolveFamily::V4)?;
        self.remember_direct(protocol, remote, address.clone())?;
        let now = (self.now)();
        if !self.cores[protocol.index()].query(&address, now, DiscoveryRequestKind::Status)? {
            return Err(ServerBrowserError::QuerySend);
        }
        self.status = "Query sent".to_owned();
        Ok(())
    }

    /// Scan the local network.
    pub fn scan(&mut self) -> Result<(), ServerBrowserError> {
        self.assert_open()?;
        if self.protocol == BrowserProtocol::Q3 {
            self.scan_q3()?;
            self.status = "Searching local network...".to_owned();
            return Ok(());
        }
        if self.protocol == BrowserProtocol::Q2 {
            if let Some(kex) = self.kex_discovery.as_mut() {
                kex.query();
            }
        }
        let now = (self.now)();
        let target = ipv4_address([255, 255, 255, 255], self.protocol.default_port(), false)?;
        self.cores[self.protocol.index()].broadcast(std::slice::from_ref(&target), now)?;
        self.status = "Searching local network...".to_owned();
        Ok(())
    }

    /// Toggle a favorite for the connect address.
    pub fn favorite(&mut self) -> Result<(), ServerBrowserError> {
        let protocol = self.protocol;
        let remote = direct_server_text(self.address().trim())
            .map_err(|error| ServerBrowserError::Addresses(error.to_string()))?;
        let address = resolve_address(&remote, protocol.default_port(), ResolveFamily::V4)?;
        let core = &mut self.cores[protocol.index()];
        let before = core.save_favorites();
        let removing = core
            .entry(&address)
            .is_some_and(|entry| entry.sources.contains(&DiscoverySource::Favorite));
        if removing {
            core.remove_favorite(&address);
        } else {
            core.add(address.clone(), DiscoverySource::Favorite, 0.0);
        }
        if protocol == BrowserProtocol::Q3 {
            self.bump_q3_generation(3);
        }
        let view = if protocol == BrowserProtocol::Q3 {
            self.current_cache_view()
        } else {
            None
        };
        let dumped = if protocol == BrowserProtocol::Q3 {
            let serialized = write_q3_browser_cache(&self.cores[protocol.index()].list_owned(), view.as_ref())?;
            self.config.dump("servers-cache-q3", &serialized)
        } else {
            let saved = self.cores[protocol.index()].save_favorites();
            self.config.dump(&format!("servers-{}", protocol.as_str()), &saved)
        };
        if let Err(error) = dumped {
            self.cores[protocol.index()].restore_favorites(&before)?;
            if protocol == BrowserProtocol::Q3 {
                self.bump_q3_generation(3);
            }
            return Err(error.into());
        }
        if protocol == BrowserProtocol::Q3 {
            self.cached_view = view;
        }
        self.status = if removing {
            "Favorite removed".to_owned()
        } else {
            "Favorite added".to_owned()
        };
        Ok(())
    }

    /// Build a connection for the connect address.
    pub fn connection(&mut self) -> Result<BrowserConnection, ServerBrowserError> {
        let protocol = self.protocol;
        let remote = direct_server_text(self.address().trim())
            .map_err(|error| ServerBrowserError::Addresses(error.to_string()))?;
        let address = resolve_address(&remote, protocol.default_port(), ResolveFamily::V4)?;
        self.remember_direct(protocol, remote.clone(), address)?;
        let mut q2_protocol = None;
        for entry in self.cores[protocol.index()].list_owned() {
            if browser_address(&entry.address)? == remote {
                if protocol == BrowserProtocol::Q2
                    && matches!(
                        entry.status.as_ref().map(|status| &status.wire),
                        Some(WireSelection::Source {
                            protocol: ProtocolIdentity::Q2Kex
                        })
                    )
                {
                    q2_protocol = Some(ProtocolIdentity::Q2Kex);
                }
                break;
            }
        }
        Ok(BrowserConnection {
            protocol,
            remote,
            q2_protocol,
        })
    }

    fn remember_direct(
        &mut self,
        protocol: BrowserProtocol,
        remote: String,
        address: NetworkAddress,
    ) -> Result<(), ServerBrowserError> {
        let mut servers = vec![DirectServerAddress {
            remote,
            address: address.clone(),
        }];
        if let Some(existing) = self.direct_servers.get(&protocol) {
            servers.extend(existing.iter().cloned());
        }
        let mut seen = HashSet::new();
        servers.retain(|server| seen.insert(address_key(&server.address, true)));
        servers.truncate(MAXIMUM_DIRECT_SERVERS);
        let serialized = stringify(&Json::Object(vec![
            ("version".to_owned(), Json::Number(1.0)),
            (
                "servers".to_owned(),
                Json::Array(
                    servers
                        .iter()
                        .map(|server| {
                            Json::Object(vec![
                                ("remote".to_owned(), Json::String(server.remote.clone())),
                                ("address".to_owned(), address_record_json(&server.address)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]));
        self.config.dump(
            &format!("servers-direct-{}", protocol.as_str()),
            &format!("{serialized}\n"),
        )?;
        self.direct_servers.insert(protocol, servers);
        self.cores[protocol.index()].add(address, DiscoverySource::Direct, 0.0);
        Ok(())
    }

    /// Pump network events and query queues.
    pub fn poll(&mut self) -> Result<(), ServerBrowserError> {
        self.assert_open()?;
        if let Some(kex) = self.kex_discovery.as_mut() {
            for address in kex.take_found() {
                let key = address_key(&address, true);
                let now = (self.now)();
                if now - self.kex_queries.get(&key).copied().unwrap_or(f64::NEG_INFINITY) < 1000.0
                    || self.kex_queries.len() >= 256
                {
                    continue;
                }
                self.kex_queries.insert(key, now);
                let query = kex_discovery_query().map_err(|error| ServerBrowserError::Kex(error.to_string()))?;
                self.transport.send(&address, &query);
            }
            for message in kex.take_failed() {
                self.status = message;
            }
        }
        while let Some(event) = self.transport.poll() {
            match event {
                BrowserEvent::Error { message } => {
                    self.status = message;
                }
                BrowserEvent::Packet { from, payload } => self.handle_packet(&from, &payload),
            }
        }
        let now = (self.now)();
        for protocol in BrowserProtocol::ALL {
            if !self.cores[protocol.index()].expire_queries(now, 3000.0).is_empty() && protocol == self.protocol {
                self.status = "No response from server".to_owned();
            }
        }
        let mut sent = 0;
        for key in self.pending_status.keys().cloned().collect::<Vec<_>>() {
            if sent == 4 {
                break;
            }
            let Some(query) = self.pending_status.get(&key).cloned() else {
                continue;
            };
            if self.cores[query.protocol.index()].pending_requests() >= 16 {
                continue;
            }
            self.pending_status.remove(&key);
            let now = (self.now)();
            self.cores[query.protocol.index()].query(&query.address, now, DiscoveryRequestKind::Status)?;
            sent += 1;
        }
        let now = (self.now)();
        if self
            .q2_master
            .as_ref()
            .is_some_and(|master| now - master.started_at >= 3000.0)
        {
            self.q2_master = None;
        }
        if self
            .master
            .as_ref()
            .is_some_and(|master| now - master.started_at >= 3000.0)
        {
            self.master = None;
        }
        Ok(())
    }

    fn handle_packet(&mut self, from: &NetworkAddress, payload: &[u8]) {
        if let Some(master) = &self.q2_master {
            if address_key(&master.address, true) == address_key(from, true) {
                match read_q2_master_reply(payload) {
                    Ok(Some(addresses)) => {
                        for address in &addresses {
                            let core = &mut self.cores[BrowserProtocol::Q2.index()];
                            if core.size() >= 4096 && core.entry(address).is_none() {
                                break;
                            }
                            core.add(address.clone(), DiscoverySource::Master, 0.0);
                            self.queue_status(BrowserProtocol::Q2, address);
                        }
                        self.status = format!("Found {} servers; querying status", addresses.len());
                        return;
                    }
                    Ok(None) => {}
                    Err(_) => {
                        self.status = "Malformed master reply".to_owned();
                        return;
                    }
                }
            }
        }
        if let Some(master) = self.master.clone() {
            if let Some(address) = &master.address {
                if address_key(address, true) == address_key(from, true) {
                    if let Ok(packet) = decode_q3_master_packet(payload) {
                        for address in &packet.addresses {
                            let listed = self.cores[BrowserProtocol::Q3.index()]
                                .list()
                                .iter()
                                .filter(|entry| {
                                    entry
                                        .sources
                                        .contains(&q3_source(master.source).unwrap_or(DiscoverySource::Master))
                                })
                                .count();
                            if listed >= if master.source == 2 { 8192 } else { 128 } {
                                break;
                            }
                            if self.add_q3(master.source, address).is_err() {
                                break;
                            }
                            self.queue_status(BrowserProtocol::Q3, address);
                        }
                        self.master = if packet.complete {
                            None
                        } else {
                            Some(Q3MasterQuery {
                                received: true,
                                ..master
                            })
                        };
                        return;
                    }
                }
            }
        }
        if let Some(sent_at) = self.kex_queries.get(&address_key(from, true)).copied() {
            if let Ok(status) = read_kex_discovery(payload) {
                let now = (self.now)();
                let core = &mut self.cores[BrowserProtocol::Q2.index()];
                let entry = core.add(from.clone(), DiscoverySource::Lan, now);
                core.restore_entry(BrowserEntry {
                    status: Some(status),
                    ping_milliseconds: Some(0f64.max(now - sent_at)),
                    updated_at: now,
                    ..entry
                });
                self.kex_queries.remove(&address_key(from, true));
                self.status = "Server updated".to_owned();
                return;
            }
        }
        for protocol in BrowserProtocol::ALL {
            let decoded = match protocol {
                BrowserProtocol::Q1 => read_net_quake_discovery(payload).ok().map(|status| (status, None)),
                BrowserProtocol::Qw => read_quake_world_discovery(payload).ok().map(|status| (status, None)),
                BrowserProtocol::Q2 => read_q2_out_of_band(payload, false)
                    .and_then(|message| read_q2_status(&message, ProtocolIdentity::Q2Classic))
                    .map(|status| (status, None)),
                BrowserProtocol::Q3 => decode_q3_server_status(payload)
                    .ok()
                    .map(|reply| (reply.status, Some(reply.challenge))),
            };
            let Some((status, challenge)) = decoded else { continue };
            if status.players < 0
                || status.max_players < 0
                || status.players > 1024
                || status.max_players > 1024
                || status.name.encode_utf16().count() > 1024
                || status.map.encode_utf16().count() > 1024
            {
                continue;
            }
            let status = ServerStatus {
                name: clean_text(&status.name),
                map: clean_text(&status.map),
                ..status
            };
            let core = &mut self.cores[protocol.index()];
            let was_lan = core
                .entry(from)
                .is_some_and(|entry| entry.sources.contains(&DiscoverySource::Lan));
            let now = (self.now)();
            if core.receive(from, status, challenge.as_deref(), now, None) {
                self.status = "Server updated".to_owned();
                if protocol == BrowserProtocol::Q3
                    && !was_lan
                    && self.cores[protocol.index()]
                        .entry(from)
                        .is_some_and(|entry| entry.sources.contains(&DiscoverySource::Lan))
                {
                    self.bump_q3_generation(0);
                }
            }
        }
    }

    /// Close the browser.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closing = true;
        if let Some(kex) = self.kex_discovery.as_mut() {
            kex.close();
        }
        self.kex_discovery = None;
        self.kex_queries.clear();
        self.q2_master = None;
        self.pending_status.clear();
        self.master = None;
        self.closed = true;
        self.transport.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct FakeTransport {
        sent: RefCell<Vec<(NetworkAddress, Vec<u8>)>>,
        inbox: RefCell<VecDeque<BrowserEvent>>,
    }

    impl FakeTransport {
        fn shared() -> Rc<Self> {
            Rc::new(Self {
                sent: RefCell::new(Vec::new()),
                inbox: RefCell::new(VecDeque::new()),
            })
        }
    }

    impl BrowserTransport for FakeTransport {
        fn send(&self, to: &NetworkAddress, bytes: &[u8]) -> bool {
            if !to.is_ip() {
                return false;
            }
            self.sent.borrow_mut().push((to.clone(), bytes.to_vec()));
            true
        }

        fn poll(&self) -> Option<BrowserEvent> {
            self.inbox.borrow_mut().pop_front()
        }

        fn close(&self) {}
    }

    struct FakeClock {
        now: std::sync::Arc<std::sync::Mutex<f64>>,
    }

    impl FakeClock {
        fn new(start: f64) -> (Self, Clock) {
            let now = std::sync::Arc::new(std::sync::Mutex::new(start));
            let clock_now = now.clone();
            (Self { now }, std::sync::Arc::new(move || *clock_now.lock().unwrap()))
        }

        fn set(&self, value: f64) {
            *self.now.lock().unwrap() = value;
        }
    }

    fn test_config() -> ConfigStore {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("qa-server-browser-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        ConfigStore::new(root)
    }

    fn open_fake() -> (StartupServerBrowser, Rc<FakeTransport>, FakeClock) {
        let transport = FakeTransport::shared();
        let (clock, now) = FakeClock::new(1000.0);
        let browser = StartupServerBrowser::open_with_transport(test_config(), transport.clone(), now).unwrap();
        (browser, transport, clock)
    }

    #[test]
    fn shared_truncate_keeps_browser_budgets() {
        assert_eq!(truncate_utf16("Quake III Arena server", 31), "Quake III Arena server");
        assert_eq!(
            truncate_utf16("abcdefghijklmnopqrstuvwxyz0123456789", 31),
            "abcdefghijklmnopqrstuvwxyz01234"
        );
        assert_eq!(truncate_utf16("a🙂bc", 2), "a");
        assert_eq!(truncate_utf16("a🙂bc", 3), "a🙂");
    }

    #[test]
    fn choose_validates() {
        let (mut browser, _, _) = open_fake();
        assert!(browser.choose("q3").is_ok());
        assert_eq!(browser.protocol, BrowserProtocol::Q3);
        assert!(browser.choose("q9").is_err());
        assert!(browser.choose_sort("name-az").is_ok());
        assert!(browser.choose_sort("bogus").is_err());
        assert_eq!(browser.address(), "localhost:27960");
        assert_eq!(browser.empty_message(), "No servers yet. Enter an address or find LAN.");
        assert_eq!(browser.details().unwrap(), vec!["Select a server first.".to_owned()]);
    }

    #[test]
    fn q3_lists_enforce_caps_and_generations() {
        let (mut browser, _, _) = open_fake();
        assert!(browser
            .add_q3(9, &ipv4_address([1, 2, 3, 4], 27960, false).unwrap())
            .is_err());
        let first = browser.q3_list(2).unwrap();
        assert!(!first.pending && first.addresses.is_empty());
        for octet in 0..128u32 {
            let address = ipv4_address([10, 0, 0, (octet % 250) as u8], 27960 + (octet / 250), false).unwrap();
            browser.add_q3(1, &address).unwrap();
        }
        // Duplicate add is a no-op.
        let dup = ipv4_address([10, 0, 0, 0], 27960, false).unwrap();
        browser.add_q3(1, &dup).unwrap();
        assert_eq!(browser.q3_list(1).unwrap().addresses.len(), 128);
        let full = ipv4_address([10, 9, 9, 9], 27960, false).unwrap();
        assert!(matches!(browser.add_q3(1, &full), Err(ServerBrowserError::ListFull)));
        browser.remove_q3(1, &dup).unwrap();
        assert_eq!(browser.q3_list(1).unwrap().addresses.len(), 127);
        assert!(browser.q3_list(1).unwrap().generation > first.generation);
    }

    #[test]
    fn query_and_expire_flow() {
        let (mut browser, transport, clock) = open_fake();
        browser.choose("q2").unwrap();
        browser.set_address("127.0.0.1:27910".to_owned());
        browser.query().unwrap();
        assert_eq!(browser.status, "Query sent");
        assert_eq!(transport.sent.borrow().len(), 1);
        clock.set(6000.0);
        browser.poll().unwrap();
        assert_eq!(browser.status, "No response from server");
    }

    #[test]
    fn favorite_toggles_and_persists() {
        let (mut browser, _, _) = open_fake();
        browser.set_address("127.0.0.1:26000".to_owned());
        browser.favorite().unwrap();
        assert_eq!(browser.status, "Favorite added");
        browser.favorite().unwrap();
        assert_eq!(browser.status, "Favorite removed");
    }

    #[test]
    fn discover_http_lists_servers() {
        let (mut browser, _, _) = open_fake();
        browser.choose("q3").unwrap();
        browser.set_master_address("https://master.example/list".to_owned());
        browser.set_master_fetch(Box::new(|_| {
            Ok(MasterHttpResponse {
                status: 200,
                body: b"# c\n127.0.0.1:27960\n127.0.0.1:27960\n".to_vec(),
            })
        }));
        browser.discover().unwrap();
        assert_eq!(browser.status, "Found 1 servers; querying status");
        assert_eq!(browser.q3_list(2).unwrap().addresses.len(), 1);
    }

    #[test]
    fn resolve_q3_filters_unusable() {
        let (mut browser, _, _) = open_fake();
        let address = browser.resolve_q3("127.0.0.1:27960").unwrap().unwrap();
        assert_eq!(address_key(&address, true), "127.0.0.1:27960");
        assert!(browser.resolve_q3("has space").unwrap().is_none());
        assert!(browser.resolve_q3("255.255.255.255:27960").unwrap().is_none());
    }

    #[test]
    fn cache_save_load_roundtrip() {
        let (mut browser, _, _) = open_fake();
        let address = ipv4_address([127, 0, 0, 1], 27960, false).unwrap();
        browser.add_q3(2, &address).unwrap();
        let view = Q3BrowserCacheView {
            lists: vec![
                Q3BrowserCacheList {
                    source: 1,
                    rows: vec![],
                },
                Q3BrowserCacheList {
                    source: 2,
                    rows: vec![Q3BrowserCacheRow {
                        address: address.clone(),
                        name: String::new(),
                        visible: 1,
                        ping: -1,
                    }],
                },
                Q3BrowserCacheList {
                    source: 3,
                    rows: vec![],
                },
            ],
        };
        browser.save_q3_cache(&view).unwrap();
        let loaded = browser.load_q3_cache().unwrap().unwrap();
        assert_eq!(loaded.lists.len(), 3);
        assert_eq!(browser.q3_list(2).unwrap().addresses, vec![address]);
    }

    #[test]
    fn close_retires_browser() {
        let (mut browser, _, _) = open_fake();
        browser.close();
        browser.close();
        assert!(browser.poll().is_err());
        assert!(browser.scan().is_err());
    }
}
