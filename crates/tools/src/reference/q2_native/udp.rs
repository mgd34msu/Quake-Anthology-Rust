//! Connectionless UDP probes (donor `tools/reference/q2-native/udp.ts`).
//!
//! Finds an unused loopback port and sends `0xFFFFFFFF`-prefixed requests,
//! collecting reply datagrams for up to two seconds. Reply text decodes as
//! Latin-1 exactly like the donor's `latin1` conversion.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use crate::error::ToolsError;
use crate::json::Json;
use crate::sha256::to_hex;

/// Bind an ephemeral loopback port, then release it and return its number.
pub fn unused_port() -> Result<u16, ToolsError> {
    let socket = UdpSocket::bind("127.0.0.1:0").map_err(|error| ToolsError::io("binding a UDP probe socket", error))?;
    socket
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| ToolsError::io("reading a UDP probe port", error))
}

/// One reply datagram.
#[derive(Debug, Clone)]
pub struct UdpPacket {
    /// Raw bytes as lowercase hex.
    pub hex: String,
    /// Raw bytes decoded as Latin-1.
    pub text: String,
    /// Sender address.
    pub from: String,
    /// Sender port.
    pub port: u16,
    /// Milliseconds from request to receipt.
    pub elapsed_ms: f64,
}

impl UdpPacket {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("hex".to_owned(), Json::string(&self.hex)),
            ("text".to_owned(), Json::string(&self.text)),
            ("from".to_owned(), Json::string(&self.from)),
            ("port".to_owned(), Json::uint(u64::from(self.port))),
            ("elapsedMs".to_owned(), Json::float(self.elapsed_ms)),
        ])
    }
}

/// A connectionless query and its replies.
#[derive(Debug, Clone)]
pub struct UdpQuery {
    /// Queried destination.
    pub destination: String,
    /// Request text.
    pub request: String,
    /// Sent bytes as lowercase hex.
    pub sent_hex: String,
    /// Reply datagrams.
    pub response: Vec<UdpPacket>,
}

impl UdpQuery {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("destination".to_owned(), Json::string(&self.destination)),
            ("request".to_owned(), Json::string(&self.request)),
            ("sentHex".to_owned(), Json::string(&self.sent_hex)),
            (
                "response".to_owned(),
                Json::array(self.response.iter().map(UdpPacket::to_json).collect()),
            ),
        ])
    }
}

/// Decode bytes as Latin-1 (donor `Buffer.toString("latin1")`).
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

/// Send one connectionless request and collect replies for up to two seconds.
pub fn query(port: u16, request: &str) -> Result<UdpQuery, ToolsError> {
    let start = Instant::now();
    let socket = UdpSocket::bind("127.0.0.1:0").map_err(|error| ToolsError::io("binding a UDP probe socket", error))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(10)))
        .map_err(|error| ToolsError::io("configuring a UDP probe socket", error))?;
    let mut bytes = vec![255, 255, 255, 255];
    bytes.extend_from_slice(request.as_bytes());
    bytes.push(b'\n');
    let destination = format!("127.0.0.1:{port}");
    socket
        .send_to(&bytes, &destination)
        .map_err(|error| ToolsError::io(format!("sending a UDP probe to {destination}"), error))?;
    let mut response = Vec::new();
    let mut chunk = [0u8; 65536];
    while response.is_empty() && start.elapsed() < Duration::from_millis(2000) {
        match socket.recv_from(&mut chunk) {
            Ok((count, peer)) => response.push(UdpPacket {
                hex: to_hex(&chunk[..count]),
                text: latin1(&chunk[..count]),
                from: peer.ip().to_string(),
                port: peer.port(),
                elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            }),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(ToolsError::io("receiving a UDP probe reply", error)),
        }
    }
    Ok(UdpQuery {
        destination,
        request: request.to_owned(),
        sent_hex: to_hex(&bytes),
        response,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_an_unused_port() {
        let port = unused_port().expect("port");
        assert!(port > 0);
        assert!(UdpSocket::bind(format!("127.0.0.1:{port}")).is_ok());
    }

    #[test]
    fn queries_a_loopback_responder() {
        let server = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let port = server.local_addr().expect("addr").port();
        let responder = std::thread::spawn(move || {
            let mut chunk = [0u8; 65536];
            let (count, peer) = server.recv_from(&mut chunk).expect("recv");
            assert_eq!(&chunk[..4], &[255, 255, 255, 255]);
            assert!(chunk[4..count].starts_with(b"status\n"));
            server.send_to(&chunk[..count], peer).expect("send");
        });
        let result = query(port, "status").expect("query");
        assert_eq!(result.destination, format!("127.0.0.1:{port}"));
        assert_eq!(result.request, "status");
        assert!(result.sent_hex.starts_with("ffffffff"), "{}", result.sent_hex);
        assert_eq!(result.response.len(), 1);
        let packet = &result.response[0];
        assert_eq!(packet.from, "127.0.0.1");
        assert_eq!(packet.port, port);
        assert_eq!(
            packet.text,
            latin1(&[255, 255, 255, 255, b's', b't', b'a', b't', b'u', b's', b'\n'])
        );
        responder.join().expect("responder");
    }

    #[test]
    fn decodes_latin1_bytes() {
        assert_eq!(latin1(&[0x41, 0xFF, 0x80]), "Aÿ\u{80}");
    }
}
