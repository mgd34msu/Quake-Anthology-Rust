//! Saved direct-server list parsing for the server browser.
//!
//! Sync port of donor `src/app/bootstrap/server-browser-addresses.ts`.
//! Address records reuse `qa_net::common::endpoint`; the JSON-to-record
//! adapter below mirrors the donor `parseNetworkAddress` dispatch exactly,
//! including which malformed shape yields which error.

use thiserror::Error;

use qa_net::common::endpoint::{
    parse_network_address, AddressError, AddressRecord, NetworkAddress,
};

use crate::settings::json::{parse_json, Json};
use crate::settings::SettingsError;

/// Failure to read a direct-server list.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServerBrowserAddressError {
    /// The saved list exceeds 65536 UTF-16 units.
    #[error("Saved direct server list is too large")]
    TooLarge,
    /// The list is not `{ version: 1, servers: [...] }`.
    #[error("Invalid direct server list")]
    InvalidList,
    /// An entry lacks `remote`/`address`.
    #[error("Invalid direct server entry")]
    InvalidEntry,
    /// A `remote` label is empty, too long, or has whitespace/controls.
    #[error("Invalid direct server address")]
    BadText,
    /// A direct server address is not IPv4/IPv6.
    #[error("Direct server requires an IP address")]
    IpRequired,
    /// The address record is malformed (donor: raw `parseNetworkAddress` throw).
    #[error(transparent)]
    Address(#[from] AddressError),
    /// The list is not valid JSON (donor: raw `JSON.parse` throw).
    #[error(transparent)]
    Json(#[from] SettingsError),
}

/// One saved direct server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectServerAddress {
    /// Display label.
    pub remote: String,
    /// IP address.
    pub address: NetworkAddress,
}

/// Maximum saved direct servers (`maximumDirectServers`).
pub const MAXIMUM_DIRECT_SERVERS: usize = 16;

/// JS `string.length` (UTF-16 units).
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// JS `/\s/` (without the unicode flag).
fn is_js_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// Validate a direct-server label.
pub fn direct_server_text(value: &str) -> Result<String, ServerBrowserAddressError> {
    if value.is_empty()
        || utf16_len(value) > 255
        || value
            .chars()
            .any(|c| is_js_whitespace(c) || c.is_control() || c == '\u{7f}')
    {
        return Err(ServerBrowserAddressError::BadText);
    }
    Ok(value.to_string())
}

fn json_port(value: f64) -> Result<u32, AddressError> {
    if value.is_finite() && value.fract() == 0.0 && value >= 0.0 && value <= f64::from(u32::MAX) {
        Ok(value as u32)
    } else {
        Err(AddressError::BadPort)
    }
}

fn json_octet(value: f64) -> Result<u8, AddressError> {
    if value.is_finite() && value.fract() == 0.0 && (0.0..=255.0).contains(&value) {
        Ok(value as u8)
    } else {
        Err(AddressError::BadOctet)
    }
}

fn json_ipx_byte(value: f64) -> Result<u8, AddressError> {
    if value.is_finite() && value.fract() == 0.0 && (0.0..=255.0).contains(&value) {
        Ok(value as u8)
    } else {
        Err(AddressError::BadIpx)
    }
}

fn json_network(value: f64) -> Result<u32, AddressError> {
    if value.is_finite() && value.fract() == 0.0 && value >= 0.0 && value <= f64::from(u32::MAX) {
        Ok(value as u32)
    } else {
        Err(AddressError::BadIpx)
    }
}

/// Mirror the donor `parseNetworkAddress` dispatch over a JSON value.
fn json_network_address(value: &Json) -> Result<NetworkAddress, AddressError> {
    let Json::Object(_) = value else {
        return Err(AddressError::BadRecord);
    };
    let kind = match value.get("kind") {
        None => return Err(AddressError::BadRecord),
        Some(Json::String(kind)) => Some(kind.as_str()),
        Some(_) => None,
    };
    if kind == Some("loopback") {
        if let Some(Json::String(id)) = value.get("id") {
            if !id.is_empty() {
                return parse_network_address(&AddressRecord {
                    kind: "loopback".to_string(),
                    id: Some(id.clone()),
                    ..AddressRecord::default()
                });
            }
        }
    }
    let port = match value.get("port") {
        Some(Json::Number(port)) => json_port(*port)?,
        _ => return Err(AddressError::MissingPort),
    };
    match kind {
        Some("ipv6") => {
            let Some(Json::String(host)) = value.get("host") else {
                return Err(AddressError::BadFields);
            };
            parse_network_address(&AddressRecord {
                kind: "ipv6".to_string(),
                host_text: Some(host.clone()),
                port: Some(port),
                ..AddressRecord::default()
            })
        }
        Some("ipv4") => {
            let Some(Json::Array(bytes)) = value.get("host") else {
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
                host[index] = json_octet(*octet)?;
            }
            parse_network_address(&AddressRecord {
                kind: "ipv4".to_string(),
                host_bytes: Some(host.to_vec()),
                port: Some(port),
                ..AddressRecord::default()
            })
        }
        Some("ipx") => {
            let (Some(Json::Array(node)), Some(Json::Number(network))) =
                (value.get("node"), value.get("network"))
            else {
                return Err(AddressError::BadFields);
            };
            if node.len() != 6 {
                return Err(AddressError::BadFields);
            }
            let mut raw = [0u8; 6];
            for (index, byte) in node.iter().enumerate() {
                let Json::Number(value) = byte else {
                    return Err(AddressError::BadFields);
                };
                raw[index] = json_ipx_byte(*value)?;
            }
            parse_network_address(&AddressRecord {
                kind: "ipx".to_string(),
                network: Some(json_network(*network)?),
                node: Some(raw.to_vec()),
                port: Some(port),
                ..AddressRecord::default()
            })
        }
        _ => Err(AddressError::BadFields),
    }
}

/// Read a saved `{ version: 1, servers: [...] }` direct-server list.
pub fn read_direct_servers(text: &str) -> Result<Vec<DirectServerAddress>, ServerBrowserAddressError> {
    if utf16_len(text) > 65536 {
        return Err(ServerBrowserAddressError::TooLarge);
    }
    let value = parse_json(text)?;
    let Json::Object(_) = value else {
        return Err(ServerBrowserAddressError::InvalidList);
    };
    let version_ok = matches!(value.get("version"), Some(Json::Number(version)) if *version == 1.0);
    let servers = match value.get("servers") {
        Some(Json::Array(servers)) => servers,
        _ => return Err(ServerBrowserAddressError::InvalidList),
    };
    if !version_ok || servers.len() > MAXIMUM_DIRECT_SERVERS {
        return Err(ServerBrowserAddressError::InvalidList);
    }
    servers
        .iter()
        .map(|item| {
            let Json::Object(_) = item else {
                return Err(ServerBrowserAddressError::InvalidEntry);
            };
            let (Some(remote_value), Some(address_value)) =
                (item.get("remote"), item.get("address"))
            else {
                return Err(ServerBrowserAddressError::InvalidEntry);
            };
            let remote = match remote_value {
                Json::String(remote) => direct_server_text(remote)?,
                _ => return Err(ServerBrowserAddressError::BadText),
            };
            let address = json_network_address(address_value)?;
            if !address.is_ip() {
                return Err(ServerBrowserAddressError::IpRequired);
            }
            Ok(DirectServerAddress { remote, address })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::json::stringify;

    fn list_json(entries: &[(&str, Json)]) -> String {
        let servers = entries
            .iter()
            .map(|(remote, address)| {
                Json::Object(vec![
                    ("remote".to_string(), Json::String(remote.to_string())),
                    ("address".to_string(), address.clone()),
                ])
            })
            .collect();
        stringify(&Json::Object(vec![
            ("version".to_string(), Json::Number(1.0)),
            ("servers".to_string(), Json::Array(servers)),
        ]))
    }

    fn ipv4_record() -> Json {
        Json::Object(vec![
            ("kind".to_string(), Json::String("ipv4".to_string())),
            (
                "host".to_string(),
                Json::Array(vec![
                    Json::Number(192.0),
                    Json::Number(168.0),
                    Json::Number(0.0),
                    Json::Number(2.0),
                ]),
            ),
            ("port".to_string(), Json::Number(27910.0)),
        ])
    }

    fn ipv6_record() -> Json {
        Json::Object(vec![
            ("kind".to_string(), Json::String("ipv6".to_string())),
            ("host".to_string(), Json::String("::1".to_string())),
            ("port".to_string(), Json::Number(27910.0)),
        ])
    }

    #[test]
    fn reads_ipv4_and_ipv6_servers() {
        let text = list_json(&[("home", ipv4_record()), ("v6", ipv6_record())]);
        let servers = read_direct_servers(&text).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].remote, "home");
        assert_eq!(servers[0].address.kind(), "ipv4");
        assert_eq!(servers[1].address.kind(), "ipv6");
        assert!(servers.iter().all(|server| server.address.is_ip()));
    }

    #[test]
    fn rejects_bad_lists_entries_and_labels() {
        assert_eq!(
            read_direct_servers("[]").unwrap_err(),
            ServerBrowserAddressError::InvalidList
        );
        assert_eq!(
            read_direct_servers(r#"{"version":2,"servers":[]}"#).unwrap_err(),
            ServerBrowserAddressError::InvalidList
        );
        assert_eq!(
            read_direct_servers(r#"{"version":1,"servers":{}}"#).unwrap_err(),
            ServerBrowserAddressError::InvalidList
        );
        let oversized = "x".repeat(65537);
        assert_eq!(
            read_direct_servers(&oversized).unwrap_err(),
            ServerBrowserAddressError::TooLarge
        );
        let many: Vec<(String, Json)> = (0..17)
            .map(|index| (format!("s{index}"), ipv4_record()))
            .collect();
        let many_ref: Vec<(&str, Json)> =
            many.iter().map(|(name, record)| (name.as_str(), record.clone())).collect();
        assert_eq!(
            read_direct_servers(&list_json(&many_ref)).unwrap_err(),
            ServerBrowserAddressError::InvalidList
        );
        assert_eq!(
            read_direct_servers(
                &stringify(&Json::Object(vec![
                    ("version".to_string(), Json::Number(1.0)),
                    (
                        "servers".to_string(),
                        Json::Array(vec![Json::Object(vec![(
                            "remote".to_string(),
                            Json::String("x".to_string())
                        )])])
                    )
                ]))
            )
            .unwrap_err(),
            ServerBrowserAddressError::InvalidEntry
        );
        assert_eq!(
            direct_server_text("has space").unwrap_err(),
            ServerBrowserAddressError::BadText
        );
        assert_eq!(
            direct_server_text("").unwrap_err(),
            ServerBrowserAddressError::BadText
        );
        assert!(direct_server_text("arena:27910").is_ok());
    }

    #[test]
    fn requires_ip_addresses_and_valid_records() {
        let loopback = Json::Object(vec![
            ("kind".to_string(), Json::String("loopback".to_string())),
            ("id".to_string(), Json::String("local".to_string())),
        ]);
        assert_eq!(
            read_direct_servers(&list_json(&[("local", loopback)])).unwrap_err(),
            ServerBrowserAddressError::IpRequired
        );
        let no_port = Json::Object(vec![
            ("kind".to_string(), Json::String("ipv4".to_string())),
            (
                "host".to_string(),
                Json::Array(vec![
                    Json::Number(1.0),
                    Json::Number(2.0),
                    Json::Number(3.0),
                    Json::Number(4.0)
                ])
            ),
        ]);
        assert!(matches!(
            read_direct_servers(&list_json(&[("x", no_port)])).unwrap_err(),
            ServerBrowserAddressError::Address(AddressError::MissingPort)
        ));
        let bad_octet = Json::Object(vec![
            ("kind".to_string(), Json::String("ipv4".to_string())),
            (
                "host".to_string(),
                Json::Array(vec![
                    Json::Number(300.0),
                    Json::Number(0.0),
                    Json::Number(0.0),
                    Json::Number(1.0)
                ])
            ),
            ("port".to_string(), Json::Number(27910.0)),
        ]);
        assert!(matches!(
            read_direct_servers(&list_json(&[("x", bad_octet)])).unwrap_err(),
            ServerBrowserAddressError::Address(AddressError::BadOctet)
        ));
        assert!(matches!(
            read_direct_servers("{oops").unwrap_err(),
            ServerBrowserAddressError::Json(_)
        ));
    }
}
