//! Network endpoint identities ported from `src/network/common/endpoint.ts`.
//!
//! IPv4/IPv6 loopback and IPX addresses with the donor's key formatting,
//! record parsing, and `host[:port]` resolution. Name resolution uses the
//! blocking [`std::net::ToSocketAddrs`] instead of the donor's async lookup.

use std::net::{IpAddr, Ipv6Addr, ToSocketAddrs};
use std::str::FromStr;

use thiserror::Error;

/// Error for malformed network addresses.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AddressError {
    /// Port is outside 1..=65535 (or 0..=65535 when zero is allowed).
    #[error("Invalid network port")]
    BadPort,
    /// IPv4 octet is outside 0..=255.
    #[error("Invalid IPv4 octet")]
    BadOctet,
    /// Host is not an IP literal.
    #[error("Expected an IP literal: {0}")]
    NotLiteral(String),
    /// IPv6 literal is malformed.
    #[error("Invalid IPv6 host")]
    BadIpv6,
    /// Bracketed IPv6 literal is missing `]`.
    #[error("Unclosed IPv6 address")]
    UnclosedIpv6,
    /// Port suffix is malformed.
    #[error("Invalid address port")]
    BadPortText,
    /// Resolved family does not match the requested family.
    #[error("Address family does not match selection")]
    FamilyMismatch,
    /// Saved address record is malformed.
    #[error("Invalid network address record")]
    BadRecord,
    /// Saved address record has no usable port.
    #[error("Network address has no port")]
    MissingPort,
    /// Saved address record fields are inconsistent.
    #[error("Invalid network address fields")]
    BadFields,
    /// IPX network/node/port is out of range.
    #[error("Invalid IPX address")]
    BadIpx,
    /// DNS resolution failed.
    #[error("Address resolution failed for {0}")]
    Unresolved(String),
}

/// Network endpoint address (`NetworkAddress`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetworkAddress {
    /// IPv4 host and port.
    Ipv4 { host: [u8; 4], port: u16 },
    /// IPv6 literal (preserving `%scope`) and port.
    Ipv6 { host: String, port: u16 },
    /// Named loopback endpoint.
    Loopback { id: String },
    /// IPX network, node, and port.
    Ipx {
        network: u32,
        node: [u8; 6],
        port: u16,
    },
}

impl NetworkAddress {
    /// Address family name (`ipv4`, `ipv6`, `loopback`, `ipx`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ipv4 { .. } => "ipv4",
            Self::Ipv6 { .. } => "ipv6",
            Self::Loopback { .. } => "loopback",
            Self::Ipx { .. } => "ipx",
        }
    }

    /// True for IPv4 and IPv6 addresses.
    #[must_use]
    pub fn is_ip(&self) -> bool {
        matches!(self, Self::Ipv4 { .. } | Self::Ipv6 { .. })
    }

    /// Port, or `None` for loopback endpoints.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        match self {
            Self::Ipv4 { port, .. }
            | Self::Ipv6 { port, .. }
            | Self::Ipx { port, .. } => Some(*port),
            Self::Loopback { .. } => None,
        }
    }
}

/// Validate a port (`portNumber`).
pub fn port_number(port: u32, allow_zero: bool) -> Result<u16, AddressError> {
    let minimum = u32::from(!allow_zero);
    if port < minimum || port > 65535 {
        return Err(AddressError::BadPort);
    }
    Ok(port as u16)
}

/// Build an IPv4 address (`ipv4Address`).
pub fn ipv4_address(host: [u8; 4], port: u32, allow_zero: bool) -> Result<NetworkAddress, AddressError> {
    Ok(NetworkAddress::Ipv4 {
        host,
        port: port_number(port, allow_zero)?,
    })
}

/// Build an IP address from a literal (`ipAddress`).
pub fn ip_address(host: &str, port: u32, allow_zero: bool) -> Result<NetworkAddress, AddressError> {
    if let Ok(addr) = Ipv6Addr::from_str(host.split('%').next().unwrap_or("")) {
        let scope = host.split('%').nth(1);
        let mut text = addr.to_string();
        if let Some(scope) = scope {
            text.push('%');
            text.push_str(scope);
        }
        return Ok(NetworkAddress::Ipv6 {
            host: text,
            port: port_number(port, allow_zero)?,
        });
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4 {
        let parsed = (
            parts[0].parse::<u16>(),
            parts[1].parse::<u16>(),
            parts[2].parse::<u16>(),
            parts[3].parse::<u16>(),
        );
        if let (Ok(a), Ok(b), Ok(c), Ok(d)) = parsed {
            if a <= 255 && b <= 255 && c <= 255 && d <= 255 {
                return ipv4_address([a as u8, b as u8, c as u8, d as u8], port, allow_zero);
            }
        }
    }
    if host.contains(':') {
        return Err(AddressError::BadIpv6);
    }
    Err(AddressError::NotLiteral(host.to_owned()))
}

/// Build an IPX address (`ipxAddress`).
pub fn ipx_address(
    network: u32,
    node: [u8; 6],
    port: u32,
) -> Result<NetworkAddress, AddressError> {
    Ok(NetworkAddress::Ipx {
        network,
        node,
        port: port_number(port, false)?,
    })
}

/// Printable host part of an IP address (`addressHost`).
#[must_use]
pub fn address_host(address: &NetworkAddress) -> String {
    match address {
        NetworkAddress::Ipv4 { host, .. } => format!("{}.{}.{}.{}", host[0], host[1], host[2], host[3]),
        NetworkAddress::Ipv6 { host, .. } => host.clone(),
        NetworkAddress::Loopback { id } => id.clone(),
        NetworkAddress::Ipx { .. } => String::new(),
    }
}

/// Canonical key for an address (`addressKey`).
#[must_use]
pub fn address_key(address: &NetworkAddress, include_port: bool) -> String {
    match address {
        NetworkAddress::Loopback { id } => format!("loopback:{id}"),
        NetworkAddress::Ipv4 { host, port } => {
            let base = format!("{}.{}.{}.{}", host[0], host[1], host[2], host[3]);
            if include_port {
                format!("{base}:{port}")
            } else {
                base
            }
        }
        NetworkAddress::Ipv6 { host, port } => {
            if include_port {
                format!("[{host}]:{port}")
            } else {
                format!("[{host}]")
            }
        }
        NetworkAddress::Ipx { network, node, port } => {
            let base = format!(
                "ipx:{network:08x}:{}",
                node.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
            );
            if include_port {
                format!("{base}:{port}")
            } else {
                base
            }
        }
    }
}

/// Address equality with optional port comparison (`sameAddress`).
#[must_use]
pub fn same_address(left: &NetworkAddress, right: &NetworkAddress, include_port: bool) -> bool {
    left.kind() == right.kind() && address_key(left, include_port) == address_key(right, include_port)
}

/// Parse a saved address record (`parseNetworkAddress`).
///
/// Records use `kind` plus `port`, an IPv4 `host` octet array, an IPv6 `host`
/// string, a loopback `id`, or an IPX `network`/`node` pair.
pub fn parse_network_address(record: &AddressRecord) -> Result<NetworkAddress, AddressError> {
    match record.kind.as_str() {
        "loopback" => match &record.id {
            Some(id) if !id.is_empty() => Ok(NetworkAddress::Loopback { id: id.clone() }),
            _ => Err(AddressError::BadRecord),
        },
        "ipv6" => {
            let (Some(host), Some(port)) = (&record.host_text, record.port) else {
                return Err(AddressError::MissingPort);
            };
            let address = ip_address(host, port, false)?;
            if !matches!(address, NetworkAddress::Ipv6 { .. }) {
                return Err(AddressError::BadFields);
            }
            Ok(address)
        }
        "ipv4" => {
            let (Some(host), Some(port)) = (&record.host_bytes, record.port) else {
                return Err(AddressError::MissingPort);
            };
            if host.len() != 4 {
                return Err(AddressError::BadFields);
            }
            ipv4_address([host[0], host[1], host[2], host[3]], port, false)
        }
        "ipx" => {
            let (Some(node), Some(network), Some(port)) = (&record.node, record.network, record.port)
            else {
                return Err(AddressError::MissingPort);
            };
            if node.len() != 6 {
                return Err(AddressError::BadFields);
            }
            ipx_address(
                network,
                [node[0], node[1], node[2], node[3], node[4], node[5]],
                port,
            )
        }
        _ => Err(AddressError::BadRecord),
    }
}

/// Saved address record fields for [`parse_network_address`].
#[derive(Debug, Clone, Default)]
pub struct AddressRecord {
    /// Address family name.
    pub kind: String,
    /// IPv4 octets or IPX node bytes.
    pub host_bytes: Option<Vec<u8>>,
    /// IPv6 literal.
    pub host_text: Option<String>,
    /// Loopback id.
    pub id: Option<String>,
    /// IPX network.
    pub network: Option<u32>,
    /// IPX node bytes.
    pub node: Option<Vec<u8>>,
    /// Port.
    pub port: Option<u32>,
}

/// Requested resolution family (`0` = any).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResolveFamily {
    /// Any family.
    #[default]
    Any,
    /// IPv4 only.
    V4,
    /// IPv6 only.
    V6,
}

/// Resolve `host[:port]` text with a default port (`resolveAddress`).
pub fn resolve_address(
    text: &str,
    default_port: u32,
    family: ResolveFamily,
) -> Result<NetworkAddress, AddressError> {
    let mut host = text;
    let mut port = default_port;
    if let Some(rest) = text.strip_prefix('[') {
        let end = rest.find(']').ok_or(AddressError::UnclosedIpv6)?;
        host = &rest[..end];
        let suffix = &rest[end + 1..];
        if !suffix.is_empty() {
            let digits = suffix.strip_prefix(':').ok_or(AddressError::BadPortText)?;
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(AddressError::BadPortText);
            }
            port = digits.parse::<u32>().map_err(|_| AddressError::BadPortText)?;
        }
    } else if text.contains(':') && text.find(':') == text.rfind(':') {
        let separator = text.rfind(':').unwrap_or(0);
        host = &text[..separator];
        let suffix = &text[separator + 1..];
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(AddressError::BadPortText);
        }
        port = suffix.parse::<u32>().map_err(|_| AddressError::BadPortText)?;
    }
    port_number(port, false)?;
    if ip_address(host, port, false).is_ok() {
        let result = ip_address(host, port, false)?;
        let resolved = match &result {
            NetworkAddress::Ipv4 { .. } => ResolveFamily::V4,
            _ => ResolveFamily::V6,
        };
        if family != ResolveFamily::Any && resolved != family {
            return Err(AddressError::FamilyMismatch);
        }
        return Ok(result);
    }
    let candidate = format!("{host}:{port}");
    let mut addrs = candidate
        .to_socket_addrs()
        .map_err(|_| AddressError::Unresolved(host.to_owned()))?;
    for addr in &mut addrs {
        let matches = match (&addr.ip(), family) {
            (IpAddr::V4(_), ResolveFamily::V4 | ResolveFamily::Any)
            | (IpAddr::V6(_), ResolveFamily::V6 | ResolveFamily::Any) => true,
            _ => false,
        };
        if matches {
            return ip_address(&addr.ip().to_string(), u32::from(addr.port()), false);
        }
    }
    Err(AddressError::Unresolved(host.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_keys_match_donor() {
        let v4 = ipv4_address([192, 168, 1, 2], 27910, false).unwrap();
        assert_eq!(address_key(&v4, true), "192.168.1.2:27910");
        assert_eq!(address_key(&v4, false), "192.168.1.2");
        let ipx = ipx_address(1, [0, 0, 0, 0, 0, 1], 0x4002).unwrap();
        assert_eq!(address_key(&ipx, true), "ipx:00000001:000000000001:16386");
        let loopback = NetworkAddress::Loopback { id: "a".to_owned() };
        assert_eq!(address_key(&loopback, true), "loopback:a");
    }

    #[test]
    fn resolve_parses_host_port_forms() {
        let address = resolve_address("127.0.0.1:27910", 1, ResolveFamily::Any).unwrap();
        assert_eq!(address_key(&address, true), "127.0.0.1:27910");
        let address = resolve_address("[::1]:27910", 1, ResolveFamily::Any).unwrap();
        assert!(matches!(address, NetworkAddress::Ipv6 { .. }));
        assert!(resolve_address("[::1", 1, ResolveFamily::Any).is_err());
        assert!(resolve_address("127.0.0.1:27910", 1, ResolveFamily::V6).is_err());
    }
}
