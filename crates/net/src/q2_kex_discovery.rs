//! KEX LAN discovery: lobby queries and DNS-SD advertisement.
//!
//! Donor provenance: `kexDiscoveryQuery`, `readKexDiscovery`, and `KexMdns`
//! in `src/network/q2/kex/discovery.ts`. Lobby status reuses
//! [`ServerStatus`](crate::services::discovery::ServerStatus); the mDNS
//! packet codec is pure, while the UDP multicast socket stays host-owned:
//! the host feeds inbound datagrams to [`KexMdns::receive`] and provides
//! the send closure (always targeting [`KEX_MDNS_MULTICAST`] port
//! [`KEX_MDNS_PORT`], like the donor).

use std::collections::{BTreeMap, HashMap};

use crate::common::endpoint::{ip_address, ipv4_address, NetworkAddress};
use crate::common::session::WireSelection;
use crate::protocol::ProtocolIdentity;
use crate::q2_kex_packet::{write_kex_packet, KexError, KexPacket, KexReader, KexWriter};
use crate::services::discovery::ServerStatus;

/// Advertised DNS-SD service.
pub const KEX_MDNS_SERVICE: &str = "_game._udp.local";
/// Suffix matching SRV records for the service.
const DOT_MDNS_SERVICE: &str = "._game._udp.local";
/// Advertised DNS-SD instance.
pub const KEX_MDNS_INSTANCE: &str = "Quake II._game._udp.local";
/// mDNS multicast group.
pub const KEX_MDNS_MULTICAST: &str = "224.0.0.251";
/// mDNS UDP port.
pub const KEX_MDNS_PORT: u16 = 5353;
/// Lobby query kind.
const LOBBY_QUERY_KIND: u8 = 129;

/// Build a lobby status query (`kexDiscoveryQuery`).
pub fn kex_discovery_query() -> Result<Vec<u8>, KexError> {
    let mut writer = KexWriter::new();
    writer.string("CRANTIME").string("QuakeII");
    write_kex_packet(&KexPacket {
        flags: 0,
        sequence: 0,
        reliable: 0,
        kind: Some(LOBBY_QUERY_KIND),
        payload: writer.finish(),
    })
}

/// Read a lobby status response (`readKexDiscovery`).
pub fn read_kex_discovery(bytes: &[u8]) -> Result<ServerStatus, KexError> {
    let mut reader = KexReader::new(bytes);
    let name = reader.string()?;
    let players = reader.integer()?;
    let maximum = reader.integer()?;
    if players > 255 || maximum > 255 || players > maximum || name.encode_utf16().count() > 1024 {
        return Err(KexError::Protocol("Invalid KEX lobby status"));
    }
    let mut rules = BTreeMap::new();
    while reader.remaining() != 0 {
        let key = reader.string()?;
        let value = reader.string()?;
        if rules.len() >= 256 {
            return Err(KexError::Protocol("KEX lobby has too many attributes"));
        }
        rules.insert(key, value);
    }
    Ok(ServerStatus {
        name,
        players: players as i64,
        max_players: maximum as i64,
        map: rules.get("map").cloned().unwrap_or_default(),
        rules,
        player_details: Vec::new(),
        wire: WireSelection::Source {
            protocol: ProtocolIdentity::Q2Kex,
        },
    })
}

/// Encode a dotted domain (`domain`).
fn dns_domain(value: &str) -> Result<Vec<u8>, KexError> {
    let mut bytes = Vec::new();
    for label in value.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(KexError::Protocol("Invalid mDNS label"));
        }
        bytes.push(label.len() as u8);
        bytes.extend_from_slice(label.as_bytes());
    }
    bytes.push(0);
    Ok(bytes)
}

/// Encode a record (`record`).
fn dns_record(name: &str, record_type: u16, data: &[u8], ttl: u16) -> Result<Vec<u8>, KexError> {
    let mut bytes = dns_domain(name)?;
    bytes.extend_from_slice(&record_type.to_be_bytes());
    bytes.extend_from_slice(&(if record_type == 12 { 1u16 } else { 0x8001 }).to_be_bytes());
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(&ttl.to_be_bytes());
    bytes.extend_from_slice(&(data.len() as u16).to_be_bytes());
    bytes.extend_from_slice(data);
    Ok(bytes)
}

/// Sanitize a hostname into an mDNS target (`announce` target mapping).
pub fn mdns_target(hostname: &str) -> String {
    let sanitized: String = hostname
        .chars()
        .map(|cell| {
            if cell.is_ascii_alphanumeric() || cell == '-' {
                cell
            } else {
                '-'
            }
        })
        .take(63)
        .collect();
    format!("{sanitized}.local")
}

/// Parsed record header (`DnsRecord`).
struct DnsRecord {
    name: String,
    record_type: u16,
    start: usize,
    length: usize,
}

/// mDNS packet reader (`DnsReader`).
struct DnsReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> DnsReader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, KexError> {
        if bytes.len() < 12 || bytes.len() > 9000 {
            return Err(KexError::Protocol("Invalid mDNS packet"));
        }
        Ok(Self { bytes, position: 12 })
    }

    fn word(&mut self) -> Result<u16, KexError> {
        let Some(word) = self.bytes.get(self.position..self.position + 2) else {
            return Err(KexError::Protocol("Truncated mDNS record"));
        };
        self.position += 2;
        Ok(u16::from_be_bytes([word[0], word[1]]))
    }

    fn name(&mut self, mut offset: usize, advance: bool) -> Result<String, KexError> {
        let mut labels = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut end = None;
        loop {
            if !seen.insert(offset) || seen.len() > 128 {
                return Err(KexError::Protocol("Invalid mDNS compression"));
            }
            let Some(size) = self.bytes.get(offset).copied() else {
                return Err(KexError::Protocol("Truncated mDNS name"));
            };
            offset += 1;
            if size == 0 {
                if advance {
                    self.position = end.unwrap_or(offset);
                }
                return Ok(labels.join("."));
            }
            if size & 192 == 192 {
                let Some(low) = self.bytes.get(offset).copied() else {
                    return Err(KexError::Protocol("Truncated mDNS pointer"));
                };
                offset += 1;
                if end.is_none() {
                    end = Some(offset);
                }
                offset = (usize::from(size & 63) << 8) | usize::from(low);
                continue;
            }
            if size > 63 || offset + usize::from(size) > self.bytes.len() {
                return Err(KexError::Protocol("Invalid mDNS label"));
            }
            let label = std::str::from_utf8(&self.bytes[offset..offset + usize::from(size)])
                .map_err(|_| KexError::Protocol("Invalid mDNS label"))?;
            labels.push(label.to_owned());
            offset += usize::from(size);
        }
    }

    fn read(&mut self) -> Result<(bool, Vec<DnsRecord>), KexError> {
        let questions = u16::from_be_bytes([self.bytes[4], self.bytes[5]]);
        let count = u32::from(u16::from_be_bytes([self.bytes[6], self.bytes[7]]))
            + u32::from(u16::from_be_bytes([self.bytes[8], self.bytes[9]]))
            + u32::from(u16::from_be_bytes([self.bytes[10], self.bytes[11]]));
        if u32::from(questions) + count > 256 {
            return Err(KexError::Protocol("Too many mDNS records"));
        }
        let mut question = false;
        for _ in 0..questions {
            let name = self.name(self.position, true)?;
            let record_type = self.word()?;
            self.word()?;
            if name.to_lowercase() == KEX_MDNS_SERVICE && (record_type == 12 || record_type == 255) {
                question = true;
            }
        }
        let mut records = Vec::new();
        for _ in 0..count {
            let name = self.name(self.position, true)?;
            let record_type = self.word()?;
            self.word()?;
            self.position += 4;
            let length = self.word()? as usize;
            let start = self.position;
            self.position += length;
            if self.position > self.bytes.len() {
                return Err(KexError::Protocol("Truncated mDNS record"));
            }
            records.push(DnsRecord {
                name,
                record_type,
                start,
                length,
            });
        }
        Ok((question, records))
    }
}

/// DNS-SD advertiser and resolver (`KexMdns`).
///
/// Retail LAN advertises DNS-SD, then queries its own unframed lobby
/// status at the resolved endpoint. The host owns the multicast socket:
/// `send` emits datagrams to the group, `found` receives resolved lobby
/// endpoints, and `failed` receives errors. Malformed or unrelated
/// KEX mDNS send sink.
pub type KexMdnsSend = Box<dyn FnMut(&[u8]) -> Result<(), String> + Send>;

/// inbound traffic is ignored.
pub struct KexMdns {
    advertised_port: Option<u16>,
    closed: bool,
    endpoints: HashMap<String, (String, u16)>,
    addresses: HashMap<String, NetworkAddress>,
    hostname: String,
    ipv4_hosts: Vec<[u8; 4]>,
    send: KexMdnsSend,
    found: Box<dyn FnMut(NetworkAddress) + Send>,
    failed: Box<dyn FnMut(String) + Send>,
}

impl KexMdns {
    /// Open an advertiser, announcing immediately when a port is
    /// advertised (donor `KexMdns.open`).
    pub fn new(
        advertised_port: Option<u16>,
        hostname: &str,
        ipv4_hosts: Vec<[u8; 4]>,
        send: impl FnMut(&[u8]) -> Result<(), String> + Send + 'static,
        found: impl FnMut(NetworkAddress) + Send + 'static,
        failed: impl FnMut(String) + Send + 'static,
    ) -> Self {
        let mut owner = Self {
            advertised_port,
            closed: false,
            endpoints: HashMap::new(),
            addresses: HashMap::new(),
            hostname: hostname.to_owned(),
            ipv4_hosts,
            send: Box::new(send),
            found: Box::new(found),
            failed: Box::new(failed),
        };
        if advertised_port.is_some() {
            owner.announce(120);
        }
        owner
    }

    /// Emit a datagram, routing send failures to `failed`.
    fn emit(&mut self, bytes: &[u8]) {
        if let Err(error) = (self.send)(bytes) {
            (self.failed)(error);
        }
    }

    /// Broadcast a service query (`query`).
    pub fn query(&mut self) {
        if self.closed {
            return;
        }
        let mut bytes = vec![0u8, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        match dns_domain(KEX_MDNS_SERVICE) {
            Ok(domain) => bytes.extend_from_slice(&domain),
            Err(error) => {
                (self.failed)(error.to_string());
                return;
            }
        }
        bytes.extend_from_slice(&[0, 12, 0, 1]);
        self.emit(&bytes);
    }

    /// Announce records (`announce`).
    fn announce(&mut self, ttl: u16) {
        let Some(port) = self.advertised_port else {
            return;
        };
        if self.closed {
            return;
        }
        let target = mdns_target(&self.hostname);
        match self.announce_records(&target, port, ttl) {
            Ok(bytes) => self.emit(&bytes),
            Err(error) => (self.failed)(error.to_string()),
        }
    }

    /// Build an announcement response.
    fn announce_records(&self, target: &str, port: u16, ttl: u16) -> Result<Vec<u8>, KexError> {
        let mut records = Vec::new();
        records.push(dns_record(KEX_MDNS_SERVICE, 12, &dns_domain(KEX_MDNS_INSTANCE)?, ttl)?);
        let mut service = vec![0, 0, 0, 0];
        service.extend_from_slice(&port.to_be_bytes());
        service.extend_from_slice(&dns_domain(target)?);
        records.push(dns_record(KEX_MDNS_INSTANCE, 33, &service, ttl)?);
        records.push(dns_record(KEX_MDNS_INSTANCE, 16, &[0], ttl)?);
        for host in &self.ipv4_hosts {
            records.push(dns_record(target, 1, host, ttl)?);
        }
        let mut bytes = vec![0, 0, 0x84, 0, 0, 0];
        bytes.extend_from_slice(&(records.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        for record in &records {
            bytes.extend_from_slice(record);
        }
        Ok(bytes)
    }

    /// Send a goodbye and close (`close`).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        if self.advertised_port.is_some() {
            self.announce(0);
        }
        self.closed = true;
    }

    /// Handle an inbound multicast datagram (`receive`).
    pub fn receive(&mut self, bytes: &[u8]) {
        let mut reader = match DnsReader::new(bytes) {
            Ok(reader) => reader,
            Err(_) => return,
        };
        let (question, records) = match reader.read() {
            Ok(parsed) => parsed,
            Err(_) => return,
        };
        if question {
            self.announce(120);
        }
        for entry in &records {
            if entry.record_type == 33 && entry.name.to_lowercase().ends_with(DOT_MDNS_SERVICE) && entry.length >= 7 {
                let Some(port) = bytes
                    .get(entry.start + 4..entry.start + 6)
                    .map(|word| u16::from_be_bytes([word[0], word[1]]))
                else {
                    return;
                };
                let target = match reader.name(entry.start + 6, false) {
                    Ok(target) => target.to_lowercase(),
                    Err(_) => return,
                };
                if port != 0 && self.endpoints.len() < 256 {
                    self.endpoints.insert(entry.name.clone(), (target, port));
                }
            } else if entry.record_type == 1 && entry.length == 4 && self.addresses.len() < 256 {
                // KEX_LAN_PORT: resolved lobbies are re-ported per endpoint below.
                let Some(octets) = bytes.get(entry.start..entry.start + 4) else {
                    return;
                };
                if let Ok(address) = ipv4_address([octets[0], octets[1], octets[2], octets[3]], 5069, false) {
                    self.addresses.insert(entry.name.to_lowercase(), address);
                }
            } else if entry.record_type == 28 && entry.length == 16 && self.addresses.len() < 256 {
                let mut parts = Vec::with_capacity(8);
                for index in 0..8 {
                    let Some(word) = bytes.get(entry.start + index * 2..entry.start + index * 2 + 2) else {
                        return;
                    };
                    parts.push(format!("{:x}", u16::from_be_bytes([word[0], word[1]])));
                }
                let Ok(address) = ip_address(&parts.join(":"), 5069, false) else {
                    return;
                };
                self.addresses.insert(entry.name.to_lowercase(), address);
            }
        }
        for (target, port) in self.endpoints.values() {
            if let Some(address) = self.addresses.get(target) {
                let mut resolved = address.clone();
                match &mut resolved {
                    NetworkAddress::Ipv4 { port: endpoint, .. }
                    | NetworkAddress::Ipv6 { port: endpoint, .. }
                    | NetworkAddress::Ipx { port: endpoint, .. } => *endpoint = *port,
                    NetworkAddress::Loopback { .. } => {}
                }
                (self.found)(resolved);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type MdnsOwner = (
        KexMdns,
        Arc<Mutex<Vec<Vec<u8>>>>,
        Arc<Mutex<Vec<NetworkAddress>>>,
        Arc<Mutex<Vec<String>>>,
    );

    fn owner(advertised_port: Option<u16>) -> MdnsOwner {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let found = Arc::new(Mutex::new(Vec::new()));
        let failed = Arc::new(Mutex::new(Vec::new()));
        let send_sink = sent.clone();
        let found_sink = found.clone();
        let failed_sink = failed.clone();
        let mdns = KexMdns::new(
            advertised_port,
            "test-host",
            vec![[192, 168, 1, 2]],
            move |bytes| {
                send_sink.lock().unwrap().push(bytes.to_vec());
                Ok(())
            },
            move |address| found_sink.lock().unwrap().push(address),
            move |error| failed_sink.lock().unwrap().push(error),
        );
        (mdns, sent, found, failed)
    }

    #[test]
    fn discovery_query_is_byte_exact() {
        let bytes = kex_discovery_query().unwrap();
        let mut payload = vec![8];
        payload.extend_from_slice(b"CRANTIME");
        payload.push(7);
        payload.extend_from_slice(b"QuakeII");
        let mut expected = vec![1, 64];
        expected.push(129);
        expected.extend_from_slice(&payload);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn lobby_status_round_trip() {
        let mut writer = KexWriter::new();
        writer
            .string("Frag House")
            .integer(3)
            .integer(8)
            .string("map")
            .string("q2dm1")
            .string("gamedir")
            .string("baseq2");
        let status = read_kex_discovery(&writer.finish()).unwrap();
        assert_eq!(status.name, "Frag House");
        assert_eq!(status.players, 3);
        assert_eq!(status.max_players, 8);
        assert_eq!(status.map, "q2dm1");
        assert_eq!(status.rules.get("gamedir").map(String::as_str), Some("baseq2"));
        assert!(status.player_details.is_empty());
        assert!(matches!(
            status.wire,
            WireSelection::Source {
                protocol: ProtocolIdentity::Q2Kex
            }
        ));
    }

    #[test]
    fn lobby_status_validates_counts() {
        let mut writer = KexWriter::new();
        writer.string("x").integer(9).integer(8);
        assert_eq!(
            read_kex_discovery(&writer.finish()),
            Err(KexError::Protocol("Invalid KEX lobby status"))
        );
        let mut writer = KexWriter::new();
        writer.string("x").integer(256).integer(256);
        assert_eq!(
            read_kex_discovery(&writer.finish()),
            Err(KexError::Protocol("Invalid KEX lobby status"))
        );
        let mut writer = KexWriter::new();
        writer.string(&"y".repeat(1025)).integer(0).integer(1);
        assert_eq!(
            read_kex_discovery(&writer.finish()),
            Err(KexError::Protocol("Invalid KEX lobby status"))
        );
    }

    #[test]
    fn lobby_attributes_are_capped() {
        let mut writer = KexWriter::new();
        writer.string("x").integer(0).integer(1);
        for index in 0..257 {
            writer.string(&format!("k{index}")).string("v");
        }
        assert_eq!(
            read_kex_discovery(&writer.finish()),
            Err(KexError::Protocol("KEX lobby has too many attributes"))
        );
    }

    #[test]
    fn target_sanitizes_hostnames() {
        assert_eq!(mdns_target("My Host!"), "My-Host-.local");
        assert_eq!(mdns_target(&"a".repeat(100)).len(), 63 + ".local".len());
    }

    #[test]
    fn query_packet_is_byte_exact() {
        let (mut mdns, sent, _, _) = owner(None);
        mdns.query();
        let datagrams = sent.lock().unwrap().clone();
        assert_eq!(datagrams.len(), 1);
        let mut expected = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        expected.extend_from_slice(&dns_domain(KEX_MDNS_SERVICE).unwrap());
        expected.extend_from_slice(&[0, 12, 0, 1]);
        assert_eq!(datagrams[0], expected);
    }

    #[test]
    fn open_announces_and_close_says_goodbye() {
        let (mut mdns, sent, _, failed) = owner(Some(5069));
        let announced = sent.lock().unwrap().clone();
        assert_eq!(announced.len(), 1);
        assert_eq!(&announced[0][..8], &[0, 0, 0x84, 0, 0, 0, 0, 4]);
        // SRV port and A record travel in the announcement.
        assert!(announced[0].windows(2).any(|window| window == [0x13, 0xCD]));
        assert!(announced[0].windows(4).any(|window| window == [192, 168, 1, 2]));
        mdns.close();
        mdns.query();
        let datagrams = sent.lock().unwrap().clone();
        assert_eq!(datagrams.len(), 2);
        assert_ne!(datagrams[0], datagrams[1]);
        assert!(failed.lock().unwrap().is_empty());
    }

    fn response_packet() -> Vec<u8> {
        let mut service = vec![0, 0, 0, 0];
        service.extend_from_slice(&5071u16.to_be_bytes());
        service.extend_from_slice(&dns_domain("peer.local").unwrap());
        let srv = dns_record("Quake II._game._udp.local", 33, &service, 120).unwrap();
        let a = dns_record("peer.local", 1, &[10, 0, 0, 5], 120).unwrap();
        let mut bytes = vec![0, 0, 0x84, 0, 0, 0, 0, 2, 0, 0, 0, 0];
        bytes.extend_from_slice(&srv);
        bytes.extend_from_slice(&a);
        bytes
    }

    #[test]
    fn response_resolves_lobby_endpoint() {
        let (mut mdns, _, found, _) = owner(None);
        mdns.receive(&response_packet());
        let found = found.lock().unwrap().clone();
        assert_eq!(found.len(), 1);
        assert!(matches!(
            found[0],
            NetworkAddress::Ipv4 {
                host: [10, 0, 0, 5],
                port: 5071
            }
        ));
    }

    #[test]
    fn question_triggers_announce() {
        let (mut mdns, sent, _, _) = owner(Some(5069));
        sent.lock().unwrap().clear();
        let mut query = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        query.extend_from_slice(&dns_domain(KEX_MDNS_SERVICE).unwrap());
        query.extend_from_slice(&[0, 12, 0, 1]);
        mdns.receive(&query);
        assert_eq!(sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn malformed_traffic_is_ignored() {
        let (mut mdns, sent, found, _) = owner(None);
        mdns.receive(&[0, 1, 2]);
        mdns.receive(&[0; 12]);
        // Compression pointer loop.
        mdns.receive(&[0, 0, 0x84, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xC0, 0x0C]);
        assert!(sent.lock().unwrap().is_empty());
        assert!(found.lock().unwrap().is_empty());
    }

    #[test]
    fn send_failure_routes_to_failed() {
        let failed = Arc::new(Mutex::new(Vec::new()));
        let sink = failed.clone();
        let mut mdns = KexMdns::new(
            None,
            "test-host",
            Vec::new(),
            |_| Err("socket down".to_owned()),
            |_| {},
            move |error| sink.lock().unwrap().push(error),
        );
        mdns.query();
        assert_eq!(failed.lock().unwrap().as_slice(), ["socket down"]);
    }
}
