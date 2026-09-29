//! SOCKS5 UDP association ported from `src/network/common/socks.ts`.
//!
//! The donor drives a Node TCP control socket with async reads; this port
//! performs the same greeting, authentication, and `UDP ASSOCIATE` exchange
//! over a blocking [`std::net::TcpStream`] with a five-second deadline. The
//! association still lasts exactly as long as its control socket.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::time::Duration;

use thiserror::Error;

use super::endpoint::{ip_address, ipv4_address, NetworkAddress};

/// SOCKS proxy options (`SocksOptions`).
#[derive(Debug, Clone)]
pub struct SocksOptions {
    /// Proxy hostname or IPv4 literal.
    pub server: String,
    /// Proxy port.
    pub port: u32,
    /// Username (empty for no authentication).
    pub username: String,
    /// Password (empty for no authentication).
    pub password: String,
}

/// Error for SOCKS framing and negotiation failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SocksError {
    /// Credential is empty-invalid, too long, or has a NUL/non-byte char.
    #[error("SOCKS credentials require non-NUL source bytes")]
    BadCredential,
    /// Server or port option is invalid.
    #[error("SOCKS server and port are invalid")]
    BadServer,
    /// Control connection failed or closed.
    #[error("SOCKS control connection failed")]
    ControlFailed,
    /// Negotiation step was rejected.
    #[error("{0}")]
    Rejected(String),
    /// Association was already started or is closed.
    #[error("SOCKS association has already started")]
    AlreadyStarted,
}

fn credential_bytes(text: &str) -> Result<Vec<u8>, SocksError> {
    if text.len() > 255 {
        return Err(SocksError::BadCredential);
    }
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.bytes() {
        if ch == 0 {
            return Err(SocksError::BadCredential);
        }
        bytes.push(ch);
    }
    Ok(bytes)
}

/// Wrap a payload in a SOCKS5 UDP datagram (`socksDatagram`).
#[must_use]
pub fn socks_datagram(to: &[u8; 4], port: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; payload.len() + 10];
    bytes[3] = 1;
    bytes[4..8].copy_from_slice(to);
    bytes[8..10].copy_from_slice(&port.to_be_bytes());
    bytes[10..].copy_from_slice(payload);
    bytes
}

/// Unwrap a SOCKS5 UDP datagram (`readSocksDatagram`).
#[must_use]
pub fn read_socks_datagram(bytes: &[u8]) -> Option<(NetworkAddress, Vec<u8>)> {
    if bytes.len() < 10 || bytes[0] != 0 || bytes[1] != 0 || bytes[2] != 0 || bytes[3] != 1 {
        return None;
    }
    let port = u16::from_be_bytes([bytes[8], bytes[9]]);
    let from = ipv4_address([bytes[4], bytes[5], bytes[6], bytes[7]], u32::from(port), false).ok()?;
    Some((from, bytes[10..].to_vec()))
}

/// Owned SOCKS5 UDP association (`SocksAssociation`).
#[derive(Debug)]
pub struct SocksAssociation {
    stream: Option<TcpStream>,
    relay_address: Option<NetworkAddress>,
    started: bool,
    failed: bool,
}

impl SocksAssociation {
    /// Create an unopened association.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stream: None,
            relay_address: None,
            started: false,
            failed: false,
        }
    }

    /// Relay address once negotiation succeeds, else `None`.
    #[must_use]
    pub fn relay(&self) -> Option<&NetworkAddress> {
        if self.failed {
            None
        } else {
            self.relay_address.as_ref()
        }
    }

    /// Negotiate the association for a local UDP port (`open`).
    pub fn open(&mut self, options: &SocksOptions, local_port: u16) -> Result<(), SocksError> {
        if self.started {
            return Err(SocksError::AlreadyStarted);
        }
        self.started = true;
        let result = self.negotiate(options, local_port);
        if result.is_err() {
            self.close();
        }
        result
    }

    fn negotiate(&mut self, options: &SocksOptions, local_port: u16) -> Result<(), SocksError> {
        if options.server.is_empty() || options.port < 1 || options.port > 65535 {
            return Err(SocksError::BadServer);
        }
        let username = credential_bytes(&options.username)?;
        let password = credential_bytes(&options.password)?;
        let authenticated = !username.is_empty() || !password.is_empty();
        let target = format!("{}:{}", options.server, options.port);
        let address: Ipv4Addr = options
            .server
            .parse()
            .map_err(|_| SocksError::ControlFailed)?;
        let deadline = Duration::from_secs(5);
        let stream = TcpStream::connect_timeout(
            &std::net::SocketAddr::new(address.into(), options.port as u16),
            deadline,
        )
        .map_err(|_| SocksError::ControlFailed)?;
        stream.set_read_timeout(Some(deadline)).map_err(|_| SocksError::ControlFailed)?;
        stream.set_write_timeout(Some(deadline)).map_err(|_| SocksError::ControlFailed)?;
        let _ = target;
        self.stream = Some(stream);
        // Repair the source's overwritten buf[2]/uninitialized buf[3] greeting.
        if authenticated {
            self.write(&[5, 2, 0, 2])?;
        } else {
            self.write(&[5, 1, 0])?;
        }
        let method = self.read_exact(2)?;
        if method[0] != 5 || (method[1] != 0 && (method[1] != 2 || !authenticated)) {
            return Err(SocksError::Rejected("SOCKS authentication method rejected".to_owned()));
        }
        if method[1] == 2 {
            let mut request = Vec::with_capacity(3 + username.len() + password.len());
            request.push(1);
            request.push(username.len() as u8);
            request.extend_from_slice(&username);
            request.push(password.len() as u8);
            request.extend_from_slice(&password);
            self.write(&request)?;
            let reply = self.read_exact(2)?;
            if reply[0] != 1 || reply[1] != 0 {
                return Err(SocksError::Rejected("SOCKS authentication failed".to_owned()));
            }
        }
        let request = [
            5,
            3,
            0,
            1,
            0,
            0,
            0,
            0,
            (local_port >> 8) as u8,
            (local_port & 255) as u8,
        ];
        self.write(&request)?;
        let header = self.read_exact(4)?;
        if header[0] != 5 || header[1] != 0 || header[2] != 0 {
            return Err(SocksError::Rejected("SOCKS UDP association rejected".to_owned()));
        }
        if header[3] != 1 {
            return Err(SocksError::Rejected("SOCKS relay address is not IPv4".to_owned()));
        }
        let reply = self.read_exact(6)?;
        let port = u16::from_be_bytes([reply[4], reply[5]]);
        if port == 0 {
            return Err(SocksError::Rejected("SOCKS relay port is zero".to_owned()));
        }
        let host = format!("{}.{}.{}.{}", reply[0], reply[1], reply[2], reply[3]);
        self.relay_address = Some(
            ip_address(&host, u32::from(port), false).map_err(|_| SocksError::ControlFailed)?,
        );
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SocksError> {
        self.stream
            .as_mut()
            .ok_or(SocksError::ControlFailed)?
            .write_all(bytes)
            .map_err(|_| SocksError::ControlFailed)
    }

    fn read_exact(&mut self, length: usize) -> Result<Vec<u8>, SocksError> {
        let mut bytes = vec![0u8; length];
        self.stream
            .as_mut()
            .ok_or(SocksError::ControlFailed)?
            .read_exact(&mut bytes)
            .map_err(|_| SocksError::ControlFailed)?;
        Ok(bytes)
    }

    /// Close the control socket; the relay becomes unavailable.
    pub fn close(&mut self) {
        self.failed = true;
        self.relay_address = None;
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Default for SocksAssociation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::endpoint::address_key;
    use super::*;

    #[test]
    fn datagram_framing_round_trips() {
        let payload = vec![9u8, 8, 7];
        let bytes = socks_datagram(&[10, 0, 0, 1], 27910, &payload);
        assert_eq!(&bytes[..4], &[0, 0, 0, 1]);
        let (from, decoded) = read_socks_datagram(&bytes).unwrap();
        assert_eq!(address_key(&from, true), "10.0.0.1:27910");
        assert_eq!(decoded, payload);
        assert!(read_socks_datagram(&bytes[..9]).is_none());
        assert!(read_socks_datagram(&[1, 0, 0, 1, 0, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn open_rejects_bad_options() {
        let mut association = SocksAssociation::new();
        let options = SocksOptions {
            server: String::new(),
            port: 1080,
            username: String::new(),
            password: String::new(),
        };
        assert_eq!(association.open(&options, 27901), Err(SocksError::BadServer));
    }
}
