//! Quake III server-side master authorization.
//!
//! Donor provenance: `Q3ServerAuthorizationOptions` and
//! `Q3ServerAuthorization` in `src/network/q3/authorization.ts`
//! (`SV_GetChallenge`). The async resolve becomes a caller-provided
//! blocking provider following the [`services::online`](crate::services::online)
//! pattern; challenges reuse [`Q3Challenge`](crate::q3_net::Q3Challenge)
//! and packets reuse
//! [`encode_connectionless_text`](crate::q3_net::encode_connectionless_text).

use crate::common::endpoint::{resolve_address, AddressError, NetworkAddress, ResolveFamily};
use crate::q3_net::{encode_connectionless_text, Q3Challenge, Q3NetError};

/// Native authorization host.
pub const Q3_AUTHORITY_HOST: &str = "authorize.quake3arena.com";
/// Native authorization port.
pub const Q3_AUTHORITY_PORT: u32 = 27952;

/// Blocking authorization bindings (donor `Q3ServerAuthorizationOptions`).
pub trait Q3ServerAuthorizationBindings {
    /// Whether authorization is enabled.
    fn enabled(&mut self) -> bool;
    /// Game directory.
    fn game_directory(&mut self) -> String;
    /// Strict-auth setting.
    fn strict_auth(&mut self) -> String;
    /// Resolve the authority (blocking). The default performs real DNS
    /// for the native host.
    fn resolve_authority(&mut self) -> Result<NetworkAddress, String> {
        resolve_q3_authority().map_err(|error| error.to_string())
    }
    /// Send a packet.
    fn send(&mut self, to: &NetworkAddress, packet: &[u8]);
    /// Print.
    fn print(&mut self, text: &str);
}

/// Resolve the native authority over blocking DNS (`resolveAddress` with
/// family 4, which enforces the donor's IPv4 requirement).
pub fn resolve_q3_authority() -> Result<NetworkAddress, AddressError> {
    resolve_address(Q3_AUTHORITY_HOST, Q3_AUTHORITY_PORT, ResolveFamily::V4)
}

/// Server authorization (`Q3ServerAuthorization`).
///
/// Uses the existing server socket and caches the native authorization
/// endpoint, including failed lookups like the donor's cached promise.
pub struct Q3ServerAuthorization<'a> {
    bindings: &'a mut dyn Q3ServerAuthorizationBindings,
    attempted: bool,
    resolved: Option<NetworkAddress>,
}

impl<'a> Q3ServerAuthorization<'a> {
    /// Build authorization over bindings.
    pub fn new(bindings: &'a mut dyn Q3ServerAuthorizationBindings) -> Self {
        Self {
            bindings,
            attempted: false,
            resolved: None,
        }
    }

    /// Cached authority address.
    pub fn address(&self) -> Option<&NetworkAddress> {
        self.resolved.as_ref()
    }

    /// Resolve and cache the authority (`resolve`).
    fn resolve(&mut self) -> Option<NetworkAddress> {
        match self.bindings.resolve_authority() {
            Ok(address) => {
                if !matches!(address, NetworkAddress::Ipv4 { .. }) {
                    self.bindings.print(
                        "Couldn't resolve Q3 authorization server: Q3 authorization requires IPv4\n",
                    );
                    return None;
                }
                self.resolved = Some(address.clone());
                Some(address)
            }
            Err(error) => {
                self.bindings
                    .print(&format!("Couldn't resolve Q3 authorization server: {error}\n"));
                None
            }
        }
    }

    /// Request authorization for a challenge (`request`).
    ///
    /// The donor re-checks challenge identity after its await; with no
    /// await in sync code that check is vacuous and omitted.
    pub fn request(&mut self, challenge: &Q3Challenge) -> Result<(), Q3NetError> {
        let Some(client) = &challenge.address else {
            return Ok(());
        };
        let NetworkAddress::Ipv4 { host, .. } = client else {
            return Ok(());
        };
        if !self.bindings.enabled() {
            return Ok(());
        }
        if !self.attempted {
            self.attempted = true;
            self.resolve();
        }
        let Some(authority) = self.resolved.clone() else {
            return Ok(());
        };
        if !self.bindings.enabled() {
            return Ok(());
        }
        let game = self.bindings.game_directory();
        let game = if game.is_empty() { "baseq3".to_owned() } else { game };
        if game.encode_utf16().count() >= 1024 {
            return Err(Q3NetError::Range(
                "Q3 authorization game directory exceeds source buffer",
            ));
        }
        let strict = self.bindings.strict_auth();
        let packet = encode_connectionless_text(&format!(
            "getIpAuthorize {} {}.{}.{}.{} {game} 0 {strict}",
            challenge.challenge, host[0], host[1], host[2], host[3]
        ))?;
        self.bindings.send(&authority, &packet);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::endpoint::ipv4_address;

    struct Fixture {
        enabled: bool,
        game: String,
        strict: String,
        resolve: Result<NetworkAddress, String>,
        resolves: usize,
        sent: Vec<(NetworkAddress, Vec<u8>)>,
        printed: Vec<String>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                enabled: true,
                game: String::new(),
                strict: "1".to_owned(),
                resolve: Ok(ipv4_address([203, 0, 113, 7], 27952, false).unwrap()),
                resolves: 0,
                sent: Vec::new(),
                printed: Vec::new(),
            }
        }
    }

    impl Q3ServerAuthorizationBindings for Fixture {
        fn enabled(&mut self) -> bool {
            self.enabled
        }

        fn game_directory(&mut self) -> String {
            self.game.clone()
        }

        fn strict_auth(&mut self) -> String {
            self.strict.clone()
        }

        fn resolve_authority(&mut self) -> Result<NetworkAddress, String> {
            self.resolves += 1;
            self.resolve.clone()
        }

        fn send(&mut self, to: &NetworkAddress, packet: &[u8]) {
            self.sent.push((to.clone(), packet.to_vec()));
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_owned());
        }
    }

    fn challenge() -> Q3Challenge {
        Q3Challenge {
            address: Some(ipv4_address([1, 2, 3, 4], 27960, false).unwrap()),
            challenge: 123,
            time: 0,
            first_time: 0,
            ping_time: 0,
            connected: false,
        }
    }

    #[test]
    fn request_sends_authorize_packet() {
        let mut fixture = Fixture::new();
        fixture.game = "missionpack".to_owned();
        let mut auth = Q3ServerAuthorization::new(&mut fixture);
        auth.request(&challenge()).unwrap();
        assert!(auth.address().is_some());
        let (to, packet) = &fixture.sent[0];
        assert_eq!(*to, fixture.resolve.clone().unwrap());
        assert_eq!(
            packet,
            &encode_connectionless_text("getIpAuthorize 123 1.2.3.4 missionpack 0 1").unwrap()
        );
    }

    #[test]
    fn empty_game_defaults_to_baseq3() {
        let mut fixture = Fixture::new();
        let mut auth = Q3ServerAuthorization::new(&mut fixture);
        auth.request(&challenge()).unwrap();
        assert_eq!(
            fixture.sent[0].1,
            encode_connectionless_text("getIpAuthorize 123 1.2.3.4 baseq3 0 1").unwrap()
        );
    }

    #[test]
    fn disabled_or_remote_skips_resolve() {
        let mut fixture = Fixture::new();
        fixture.enabled = false;
        let remote = Q3Challenge {
            address: Some(NetworkAddress::Ipv6 {
                host: "::1".to_owned(),
                port: 27960,
            }),
            challenge: 123,
            time: 0,
            first_time: 0,
            ping_time: 0,
            connected: false,
        };
        {
            let mut auth = Q3ServerAuthorization::new(&mut fixture);
            auth.request(&challenge()).unwrap();
            auth.request(&remote).unwrap();
        }
        assert!(fixture.sent.is_empty());
        assert_eq!(fixture.resolves, 0);
    }

    #[test]
    fn failed_lookup_prints_once() {
        let mut fixture = Fixture::new();
        fixture.resolve = Err("no route".to_owned());
        let mut auth = Q3ServerAuthorization::new(&mut fixture);
        let request = challenge();
        auth.request(&request).unwrap();
        auth.request(&request).unwrap();
        assert!(fixture.sent.is_empty());
        assert_eq!(fixture.resolves, 1);
        assert_eq!(fixture.printed, vec!["Couldn't resolve Q3 authorization server: no route\n"]);
    }

    #[test]
    fn oversized_game_directory_fails() {
        let mut fixture = Fixture::new();
        fixture.game = "g".repeat(1024);
        let mut auth = Q3ServerAuthorization::new(&mut fixture);
        assert_eq!(
            auth.request(&challenge()),
            Err(Q3NetError::Range(
                "Q3 authorization game directory exceeds source buffer"
            ))
        );
    }
}
