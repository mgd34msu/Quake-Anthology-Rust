//! Q3 server browser cache codec.
//!
//! Donor: `src/app/bootstrap/server-browser-cache.ts`
//! (`readQ3BrowserCache`, `writeQ3BrowserCache`). JSON goes through
//! `crate::settings::json`; addresses reuse `qa_net` endpoint
//! validators; entries/view reuse `qa_net` discovery types.

use std::collections::{BTreeMap, HashMap, HashSet};

use qa_net::common::endpoint::{
    address_key, ip_address, ipv4_address, ipx_address, AddressError, NetworkAddress,
};
use qa_net::common::session::WireSelection;
use qa_net::protocol::ProtocolIdentity;
use qa_net::q3_browser_view::{Q3BrowserCacheList, Q3BrowserCacheRow, Q3BrowserCacheView};
use qa_net::services::discovery::{BrowserEntry, DiscoverySource, PlayerDetail, ServerStatus};
use thiserror::Error;

use crate::settings::json::{parse_json, stringify, Json};
use crate::settings::SettingsError;

/// Browser cache failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServerBrowserCacheError {
    /// Cache exceeds 8 MiB.
    #[error("Browser cache is too large")]
    TooLarge,
    /// Expected a JSON object.
    #[error("Invalid browser cache object")]
    BadObject,
    /// Expected a JSON array.
    #[error("Invalid browser cache array")]
    BadArray,
    /// Expected an integer in range.
    #[error("Invalid browser cache integer")]
    BadInteger,
    /// Expected bounded text without NUL.
    #[error("Invalid browser cache text")]
    BadText,
    /// Cache addresses must be IPv4 with a port.
    #[error("Browser cache requires an IPv4 endpoint")]
    NeedsIpv4,
    /// Expected a finite nonnegative time.
    #[error("Invalid browser cache time")]
    BadTime,
    /// Cache status requires Q3 protocol 68.
    #[error("Browser cache status requires Q3 protocol 68")]
    BadProtocol,
    /// Too many status rules.
    #[error("Too many browser cache rules")]
    TooManyRules,
    /// Malformed status rule pair.
    #[error("Invalid browser cache rule")]
    BadRule,
    /// Too many players.
    #[error("Too many browser cache players")]
    TooManyPlayers,
    /// Unsupported version or protocol.
    #[error("Unsupported browser cache version or protocol")]
    BadVersion,
    /// Too many entries.
    #[error("Too many browser cache entries")]
    TooManyEntries,
    /// Duplicate endpoint.
    #[error("Duplicate browser cache endpoint")]
    DuplicateEndpoint,
    /// Invalid cached source.
    #[error("Invalid cached browser source")]
    BadSource,
    /// Empty or repeated memberships.
    #[error("Invalid cached browser memberships")]
    BadMemberships,
    /// Too many master entries.
    #[error("Too many cached master entries")]
    TooManyMaster,
    /// View requires three UI lists.
    #[error("Browser cache requires three UI lists")]
    BadListCount,
    /// Invalid list source.
    #[error("Invalid browser cache list source")]
    BadListSource,
    /// Too many UI rows.
    #[error("Too many browser cache UI rows")]
    TooManyRows,
    /// UI name must be source bytes.
    #[error("Browser cache UI name must contain source bytes")]
    BadRowName,
    /// Duplicate or mismatched UI row.
    #[error("Browser cache UI row has duplicate or mismatched membership")]
    BadRowMembership,
    /// JSON failure.
    #[error("{0}")]
    Json(#[from] SettingsError),
    /// Address record failure.
    #[error("{0}")]
    Address(#[from] AddressError),
}

/// Parsed cache.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3BrowserCache {
    /// Entries.
    pub entries: Vec<BrowserEntry>,
    /// UI view.
    pub view: Option<Q3BrowserCacheView>,
}

const MAXIMUM_BYTES: usize = 8 << 20;

fn record(value: Option<&Json>) -> Result<&Vec<(String, Json)>, ServerBrowserCacheError> {
    match value {
        Some(Json::Object(fields)) => Ok(fields),
        _ => Err(ServerBrowserCacheError::BadObject),
    }
}

fn member<'a>(fields: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    fields.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

fn array(value: Option<&Json>) -> Result<&Vec<Json>, ServerBrowserCacheError> {
    match value {
        Some(Json::Array(items)) => Ok(items),
        _ => Err(ServerBrowserCacheError::BadArray),
    }
}

fn integer(value: Option<&Json>, minimum: f64, maximum: f64) -> Result<i64, ServerBrowserCacheError> {
    match value {
        Some(Json::Number(number))
            if number.is_finite() && number.fract() == 0.0 && *number >= minimum && *number <= maximum =>
        {
            Ok(*number as i64)
        }
        _ => Err(ServerBrowserCacheError::BadInteger),
    }
}

/// Donor `text`: UTF-16 length budget, no NUL.
fn text(value: Option<&Json>, maximum: usize) -> Result<String, ServerBrowserCacheError> {
    match value {
        Some(Json::String(text)) if text.encode_utf16().count() <= maximum && !text.contains('\0') => {
            Ok(text.clone())
        }
        _ => Err(ServerBrowserCacheError::BadText),
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

/// Donor `parseNetworkAddress` over a JSON value, preserving its
/// branch order and messages.
fn parse_address_json(value: Option<&Json>) -> Result<NetworkAddress, AddressError> {
    let Some(Json::Object(_)) = value else {
        return Err(AddressError::BadRecord);
    };
    let fields = match value {
        Some(Json::Object(fields)) => fields,
        _ => return Err(AddressError::BadRecord),
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

fn address(value: Option<&Json>) -> Result<NetworkAddress, ServerBrowserCacheError> {
    let endpoint = parse_address_json(value)?;
    match &endpoint {
        NetworkAddress::Ipv4 { port, .. } if *port != 0 => Ok(endpoint),
        _ => Err(ServerBrowserCacheError::NeedsIpv4),
    }
}

fn finite(value: Option<&Json>) -> Result<f64, ServerBrowserCacheError> {
    match value {
        Some(Json::Number(number)) if number.is_finite() && *number >= 0.0 => Ok(*number),
        _ => Err(ServerBrowserCacheError::BadTime),
    }
}

fn status(value: Option<&Json>) -> Result<Option<ServerStatus>, ServerBrowserCacheError> {
    let Some(value) = value else {
        return Err(ServerBrowserCacheError::BadObject);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let source = record(Some(value))?;
    if !matches!(member(source, "protocol"), Some(Json::Number(protocol)) if *protocol == 68.0) {
        return Err(ServerBrowserCacheError::BadProtocol);
    }
    let pairs = array(member(source, "rules"))?;
    if pairs.len() > 1024 {
        return Err(ServerBrowserCacheError::TooManyRules);
    }
    let mut rules = BTreeMap::new();
    for pair in pairs {
        let fields = array(Some(pair))?;
        if fields.len() != 2 {
            return Err(ServerBrowserCacheError::BadRule);
        }
        rules.insert(text(Some(&fields[0]), 1024)?, text(Some(&fields[1]), 8192)?);
    }
    let players = array(member(source, "playerDetails"))?;
    if players.len() > 1024 {
        return Err(ServerBrowserCacheError::TooManyPlayers);
    }
    let mut player_details = Vec::with_capacity(players.len());
    for player in players {
        let fields = record(Some(player))?;
        player_details.push(PlayerDetail {
            name: text(member(fields, "name"), 1024)?,
            score: integer(member(fields, "score"), -2_147_483_648.0, 2_147_483_647.0)?,
            ping: integer(member(fields, "ping"), -2_147_483_648.0, 2_147_483_647.0)?,
        });
    }
    Ok(Some(ServerStatus {
        name: text(member(source, "name"), 1024)?,
        map: text(member(source, "map"), 1024)?,
        players: integer(member(source, "players"), 0.0, 1024.0)?,
        max_players: integer(member(source, "maxPlayers"), 0.0, 1024.0)?,
        rules,
        player_details,
        wire: WireSelection::Source { protocol: ProtocolIdentity::Q3 },
    }))
}

fn address_json(address: &NetworkAddress) -> Json {
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

fn source_name(source: DiscoverySource) -> Option<&'static str> {
    match source {
        DiscoverySource::Master => Some("master"),
        DiscoverySource::SecondaryMaster => Some("secondary-master"),
        DiscoverySource::Favorite => Some("favorite"),
        DiscoverySource::Lan | DiscoverySource::Direct => None,
    }
}

/// Read a serialized Q3 browser cache.
pub fn read_q3_browser_cache(serialized: &str) -> Result<Q3BrowserCache, ServerBrowserCacheError> {
    if serialized.len() > MAXIMUM_BYTES {
        return Err(ServerBrowserCacheError::TooLarge);
    }
    let parsed = parse_json(serialized)?;
    let root = record(Some(&parsed))?;
    if !matches!(member(root, "version"), Some(Json::Number(version)) if *version == 1.0)
        || !matches!(member(root, "protocol"), Some(Json::String(protocol)) if protocol == "q3")
    {
        return Err(ServerBrowserCacheError::BadVersion);
    }
    let values = array(member(root, "entries"))?;
    if values.len() > 16384 {
        return Err(ServerBrowserCacheError::TooManyEntries);
    }
    let mut by_address = HashMap::new();
    let mut entries = Vec::with_capacity(values.len());
    for value in values {
        let entry = record(Some(value))?;
        let endpoint = address(member(entry, "address"))?;
        let key = address_key(&endpoint, true);
        if by_address.contains_key(&key) {
            return Err(ServerBrowserCacheError::DuplicateEndpoint);
        }
        let raw_sources = array(member(entry, "sources"))?;
        let mut sources = Vec::with_capacity(raw_sources.len());
        for source in raw_sources {
            match source {
                Json::String(name) if name == "master" => sources.push(DiscoverySource::Master),
                Json::String(name) if name == "secondary-master" => {
                    sources.push(DiscoverySource::SecondaryMaster);
                }
                Json::String(name) if name == "favorite" => sources.push(DiscoverySource::Favorite),
                _ => return Err(ServerBrowserCacheError::BadSource),
            }
        }
        if sources.is_empty() || HashSet::<DiscoverySource>::from_iter(sources.iter().copied()).len() != sources.len()
        {
            return Err(ServerBrowserCacheError::BadMemberships);
        }
        let ping_milliseconds = match member(entry, "pingMilliseconds") {
            None => return Err(ServerBrowserCacheError::BadTime),
            Some(Json::Null) => None,
            value => Some(finite(value)?),
        };
        let parsed = BrowserEntry {
            address: endpoint,
            sources,
            status: status(member(entry, "status"))?,
            ping_milliseconds,
            updated_at: finite(member(entry, "updatedAt"))?,
        };
        by_address.insert(key, parsed.clone());
        entries.push(parsed);
    }
    if entries.iter().filter(|entry| entry.sources.contains(&DiscoverySource::Master)).count() > 8192
        || entries
            .iter()
            .filter(|entry| entry.sources.contains(&DiscoverySource::SecondaryMaster))
            .count()
            > 128
    {
        return Err(ServerBrowserCacheError::TooManyMaster);
    }
    let view_value = member(root, "view");
    if view_value.is_none() {
        return Err(ServerBrowserCacheError::BadObject);
    }
    if matches!(view_value, Some(Json::Null)) {
        return Ok(Q3BrowserCache { entries, view: None });
    }
    let lists = array(member(record(view_value)?, "lists"))?;
    if lists.len() != 3 {
        return Err(ServerBrowserCacheError::BadListCount);
    }
    let mut seen = HashSet::new();
    let mut view_lists = Vec::with_capacity(3);
    for value in lists {
        let list = record(Some(value))?;
        let source = match member(list, "source") {
            Some(Json::Number(source)) if *source == 1.0 || *source == 2.0 || *source == 3.0 => {
                *source as u8
            }
            _ => return Err(ServerBrowserCacheError::BadListSource),
        };
        if !seen.insert(source) {
            return Err(ServerBrowserCacheError::BadListSource);
        }
        let rows = array(member(list, "rows"))?;
        if rows.len() > if source == 2 { 4096 } else { 128 } {
            return Err(ServerBrowserCacheError::TooManyRows);
        }
        let membership = if source == 1 {
            DiscoverySource::SecondaryMaster
        } else if source == 2 {
            DiscoverySource::Master
        } else {
            DiscoverySource::Favorite
        };
        let mut row_keys = HashSet::new();
        let mut view_rows = Vec::with_capacity(rows.len());
        for value in rows {
            let row = record(Some(value))?;
            let name = text(member(row, "name"), 31)?;
            if name.chars().any(|c| c as u32 > 255) {
                return Err(ServerBrowserCacheError::BadRowName);
            }
            let endpoint = address(member(row, "address"))?;
            let key = address_key(&endpoint, true);
            let listed = by_address.get(&key).is_some_and(|entry: &BrowserEntry| entry.sources.contains(&membership));
            if !row_keys.insert(key) || !listed {
                return Err(ServerBrowserCacheError::BadRowMembership);
            }
            view_rows.push(Q3BrowserCacheRow {
                address: endpoint,
                name,
                visible: integer(member(row, "visible"), -2_147_483_648.0, 2_147_483_647.0)? as i32,
                ping: integer(member(row, "ping"), -2_147_483_648.0, 2_147_483_647.0)? as i32,
            });
        }
        view_lists.push(Q3BrowserCacheList { source, rows: view_rows });
    }
    Ok(Q3BrowserCache { entries, view: Some(Q3BrowserCacheView { lists: view_lists }) })
}

/// Write a Q3 browser cache, validating the output by re-reading it.
pub fn write_q3_browser_cache(
    entries: &[BrowserEntry],
    view: Option<&Q3BrowserCacheView>,
) -> Result<String, ServerBrowserCacheError> {
    let mut encoded = Vec::new();
    for entry in entries {
        let sources: Vec<Json> = entry
            .sources
            .iter()
            .filter_map(|source| source_name(*source).map(|name| Json::String(name.to_owned())))
            .collect();
        if sources.is_empty() {
            continue;
        }
        let status = match &entry.status {
            None => Json::Null,
            Some(status) => Json::Object(vec![
                ("name".to_owned(), Json::String(status.name.clone())),
                ("map".to_owned(), Json::String(status.map.clone())),
                ("players".to_owned(), Json::Number(status.players as f64)),
                ("maxPlayers".to_owned(), Json::Number(status.max_players as f64)),
                (
                    "rules".to_owned(),
                    Json::Array(
                        status
                            .rules
                            .iter()
                            .map(|(key, value)| {
                                Json::Array(vec![Json::String(key.clone()), Json::String(value.clone())])
                            })
                            .collect(),
                    ),
                ),
                (
                    "playerDetails".to_owned(),
                    Json::Array(
                        status
                            .player_details
                            .iter()
                            .map(|player| {
                                Json::Object(vec![
                                    ("name".to_owned(), Json::String(player.name.clone())),
                                    ("score".to_owned(), Json::Number(player.score as f64)),
                                    ("ping".to_owned(), Json::Number(player.ping as f64)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("protocol".to_owned(), Json::Number(68.0)),
            ]),
        };
        encoded.push(Json::Object(vec![
            ("address".to_owned(), address_json(&entry.address)),
            ("sources".to_owned(), Json::Array(sources)),
            ("status".to_owned(), status),
            (
                "pingMilliseconds".to_owned(),
                entry.ping_milliseconds.map_or(Json::Null, Json::Number),
            ),
            ("updatedAt".to_owned(), Json::Number(entry.updated_at)),
        ]));
    }
    let view_json = match view {
        None => Json::Null,
        Some(view) => Json::Object(vec![(
            "lists".to_owned(),
            Json::Array(
                view.lists
                    .iter()
                    .map(|list| {
                        Json::Object(vec![
                            ("source".to_owned(), Json::Number(f64::from(list.source))),
                            (
                                "rows".to_owned(),
                                Json::Array(
                                    list.rows
                                        .iter()
                                        .map(|row| {
                                            Json::Object(vec![
                                                ("address".to_owned(), address_json(&row.address)),
                                                ("name".to_owned(), Json::String(row.name.clone())),
                                                ("visible".to_owned(), Json::Number(f64::from(row.visible))),
                                                ("ping".to_owned(), Json::Number(f64::from(row.ping))),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        )]),
    };
    let serialized = stringify(&Json::Object(vec![
        ("version".to_owned(), Json::Number(1.0)),
        ("protocol".to_owned(), Json::String("q3".to_owned())),
        ("entries".to_owned(), Json::Array(encoded)),
        ("view".to_owned(), view_json),
    ]));
    read_q3_browser_cache(&serialized)?;
    Ok(serialized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> NetworkAddress {
        ipv4_address([192, 168, 1, 10], 27960, false).unwrap()
    }

    fn entry() -> BrowserEntry {
        BrowserEntry {
            address: endpoint(),
            sources: vec![DiscoverySource::Master, DiscoverySource::Favorite],
            status: Some(ServerStatus {
                name: "server".to_owned(),
                map: "q3dm1".to_owned(),
                players: 2,
                max_players: 8,
                rules: BTreeMap::from([("gamedir".to_owned(), "baseq3".to_owned())]),
                player_details: vec![PlayerDetail { name: "p".to_owned(), score: 3, ping: 20 }],
                wire: WireSelection::Source { protocol: ProtocolIdentity::Q3 },
            }),
            ping_milliseconds: Some(20.0),
            updated_at: 1000.0,
        }
    }

    fn view() -> Q3BrowserCacheView {
        Q3BrowserCacheView {
            lists: vec![
                Q3BrowserCacheList { source: 1, rows: vec![] },
                Q3BrowserCacheList {
                    source: 2,
                    rows: vec![Q3BrowserCacheRow { address: endpoint(), name: "server".to_owned(), visible: 1, ping: 20 }],
                },
                Q3BrowserCacheList {
                    source: 3,
                    rows: vec![Q3BrowserCacheRow { address: endpoint(), name: "server".to_owned(), visible: 1, ping: 20 }],
                },
            ],
        }
    }

    #[test]
    fn roundtrip_preserves_entries_and_view() {
        let serialized = write_q3_browser_cache(&[entry()], Some(&view())).unwrap();
        let cache = read_q3_browser_cache(&serialized).unwrap();
        assert_eq!(cache.entries, vec![entry()]);
        assert_eq!(cache.view, Some(view()));
    }

    #[test]
    fn roundtrip_drops_lan_only_entries() {
        let mut lan = entry();
        lan.sources = vec![DiscoverySource::Lan];
        let serialized = write_q3_browser_cache(&[lan], None).unwrap();
        let cache = read_q3_browser_cache(&serialized).unwrap();
        assert!(cache.entries.is_empty());
        assert_eq!(cache.view, None);
    }

    #[test]
    fn rejects_bad_version_duplicates_and_memberships() {
        assert_eq!(
            read_q3_browser_cache(r#"{"version":2,"protocol":"q3","entries":[],"view":null}"#),
            Err(ServerBrowserCacheError::BadVersion)
        );
        let both = write_q3_browser_cache(&[entry(), entry()], None);
        assert!(both.is_err());
        let mut bad = entry();
        bad.sources = vec![DiscoverySource::Master, DiscoverySource::Master];
        // Bypasses the writer (which filters but keeps order); read path rejects.
        let serialized = format!(
            r#"{{"version":1,"protocol":"q3","entries":[{{"address":{{"kind":"ipv4","host":[192,168,1,10],"port":27960}},"sources":["master","master"],"status":null,"pingMilliseconds":null,"updatedAt":1}}],"view":null}}"#
        );
        let _ = bad;
        assert_eq!(
            read_q3_browser_cache(&serialized),
            Err(ServerBrowserCacheError::BadMemberships)
        );
    }

    #[test]
    fn rejects_view_row_mismatch() {
        let mut orphan = view();
        orphan.lists[1].rows[0].address = ipv4_address([10, 0, 0, 1], 27960, false).unwrap();
        assert!(write_q3_browser_cache(&[entry()], Some(&orphan)).is_err());
    }

    #[test]
    fn rejects_non_ipv4_and_bad_status() {
        let serialized = r#"{"version":1,"protocol":"q3","entries":[{"address":{"kind":"ipv6","host":"::1","port":27960},"sources":["master"],"status":null,"pingMilliseconds":null,"updatedAt":1}],"view":null}"#;
        assert_eq!(read_q3_browser_cache(serialized), Err(ServerBrowserCacheError::NeedsIpv4));
        let serialized = r#"{"version":1,"protocol":"q3","entries":[{"address":{"kind":"ipv4","host":[1,2,3,4],"port":27960},"sources":["master"],"status":{"name":"n","map":"m","players":0,"maxPlayers":0,"rules":[],"playerDetails":[],"protocol":66},"pingMilliseconds":null,"updatedAt":1}],"view":null}"#;
        assert_eq!(read_q3_browser_cache(serialized), Err(ServerBrowserCacheError::BadProtocol));
    }
}
