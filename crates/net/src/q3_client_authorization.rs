//! Quake III client-side master authorization.
//!
//! Donor provenance: `Q3ClientAuthorizationOptions` and
//! `Q3ClientAuthorization` in `src/network/q3/client-authorization.ts`
//! (`CL_RequestAuthorization`). The cvar registry reuses
//! [`CvarRegistry`](qa_core::cvar::CvarRegistry); no q3-cd-key port exists
//! in `qa-core`, so key bytes come from the caller-provided
//! [`Q3CdKeyAuthorization`] reader. Resolution is a caller-provided
//! blocking provider defaulting to
//! [`resolve_q3_authority`](crate::q3_authorization::resolve_q3_authority).

use qa_core::cvar::{flags, CvarRegistry};

use crate::common::endpoint::NetworkAddress;
use crate::q3_authorization::resolve_q3_authority;
use crate::q3_net::{encode_connectionless_text, Q3NetError};

/// CD-key authorization reader (donor `Q3CdKeyState.readAuthorization`).
pub trait Q3CdKeyAuthorization {
    /// Fill 33 bytes with the authorization key.
    fn read_authorization(&mut self, out: &mut [u8; 33]);
}

/// Blocking client authorization bindings.
pub trait Q3ClientAuthorizationBindings {
    /// Whether the demo build restricts keys.
    fn demo_restricted(&mut self) -> bool;
    /// Resolve the authority (blocking); `None` reports failure.
    fn resolve_authority(&mut self) -> Option<NetworkAddress> {
        resolve_q3_authority().ok()
    }
    /// Print.
    fn print(&mut self, text: &str);
}

/// Client authorization (`Q3ClientAuthorization`).
///
/// Retains the client-static authority address; the current connection
/// supplies its socket and guard.
pub struct Q3ClientAuthorization<'a> {
    cvars: &'a mut CvarRegistry,
    keys: &'a mut dyn Q3CdKeyAuthorization,
    bindings: &'a mut dyn Q3ClientAuthorizationBindings,
    address: Option<NetworkAddress>,
}

impl<'a> Q3ClientAuthorization<'a> {
    /// Build authorization over a cvar registry, key reader, and bindings.
    pub fn new(
        cvars: &'a mut CvarRegistry,
        keys: &'a mut dyn Q3CdKeyAuthorization,
        bindings: &'a mut dyn Q3ClientAuthorizationBindings,
    ) -> Self {
        Self {
            cvars,
            keys,
            bindings,
            address: None,
        }
    }

    /// Request authorization (`request`).
    ///
    /// `assert_current` panics when the session is stale, matching the
    /// donor's throwing guard and
    /// [`Q3ClientBindings::assert_current`](crate::q3_net::Q3ClientBindings::assert_current).
    pub fn request(
        &mut self,
        assert_current: &mut dyn FnMut(),
        send: &mut dyn FnMut(&NetworkAddress, &[u8]),
    ) -> Result<(), Q3NetError> {
        assert_current();
        if self.address.is_none() {
            let address = self
                .bindings
                .resolve_authority()
                .filter(|address| matches!(address, NetworkAddress::Ipv4 { .. }));
            assert_current();
            let Some(address) = address else {
                self.bindings.print("Couldn't resolve Q3 authorization server\n");
                return Ok(());
            };
            self.address = Some(address);
        }
        let mut key = "demota".to_owned();
        if !self.bindings.demo_restricted() {
            key.clear();
            let mut bytes = [0u8; 33];
            self.keys.read_authorization(&mut bytes);
            for byte in bytes.iter().take(32) {
                if *byte == 0 {
                    break;
                }
                if byte.is_ascii_alphanumeric() {
                    key.push(*byte as char);
                }
            }
        }
        self.cvars
            .register("cl_anonymous", "0", flags::INIT | flags::SYSTEM_INFO)?;
        let anonymous = self.cvars.get("cl_anonymous").map_or(0, |snapshot| snapshot.integer_value);
        assert_current();
        let Some(address) = self.address.clone() else {
            return Ok(());
        };
        let packet = encode_connectionless_text(&format!("getKeyAuthorize {anonymous} {key}"))?;
        send(&address, &packet);
        assert_current();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::endpoint::ipv4_address;
    use qa_core::cmd::Dialect;

    struct Keys {
        bytes: [u8; 33],
    }

    impl Q3CdKeyAuthorization for Keys {
        fn read_authorization(&mut self, out: &mut [u8; 33]) {
            *out = self.bytes;
        }
    }

    struct Fixture {
        demo_restricted: bool,
        resolve: Option<NetworkAddress>,
        resolves: usize,
        printed: Vec<String>,
    }

    impl Q3ClientAuthorizationBindings for Fixture {
        fn demo_restricted(&mut self) -> bool {
            self.demo_restricted
        }

        fn resolve_authority(&mut self) -> Option<NetworkAddress> {
            self.resolves += 1;
            self.resolve.clone()
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_owned());
        }
    }

    fn authority() -> NetworkAddress {
        ipv4_address([203, 0, 113, 7], 27952, false).unwrap()
    }

    #[test]
    fn request_filters_key_bytes() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        let mut bytes = [0u8; 33];
        bytes[..9].copy_from_slice(b"AB-12 cd!");
        let mut keys = Keys { bytes };
        let mut fixture = Fixture {
            demo_restricted: false,
            resolve: Some(authority()),
            resolves: 0,
            printed: Vec::new(),
        };
        let mut auth = Q3ClientAuthorization::new(&mut cvars, &mut keys, &mut fixture);
        let mut sent = Vec::new();
        auth.request(&mut || {}, &mut |to, packet| sent.push((to.clone(), packet.to_vec())))
            .unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, authority());
        assert_eq!(
            sent[0].1,
            encode_connectionless_text("getKeyAuthorize 0 AB12cd").unwrap()
        );
        assert_eq!(auth.address, Some(authority()));
    }

    #[test]
    fn demo_builds_use_demota() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        let mut keys = Keys { bytes: [b'Z'; 33] };
        let mut fixture = Fixture {
            demo_restricted: true,
            resolve: Some(authority()),
            resolves: 0,
            printed: Vec::new(),
        };
        let mut auth = Q3ClientAuthorization::new(&mut cvars, &mut keys, &mut fixture);
        let mut sent = Vec::new();
        auth.request(&mut || {}, &mut |to, packet| sent.push((to.clone(), packet.to_vec())))
            .unwrap();
        assert_eq!(
            sent[0].1,
            encode_connectionless_text("getKeyAuthorize 0 demota").unwrap()
        );
    }

    #[test]
    fn failed_resolve_prints_and_caches_nothing() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        let mut keys = Keys { bytes: [0; 33] };
        let mut fixture = Fixture {
            demo_restricted: false,
            resolve: None,
            resolves: 0,
            printed: Vec::new(),
        };
        let mut sent = Vec::new();
        {
            let mut auth = Q3ClientAuthorization::new(&mut cvars, &mut keys, &mut fixture);
            auth.request(&mut || {}, &mut |to, packet| sent.push((to.clone(), packet.to_vec())))
                .unwrap();
        }
        assert!(sent.is_empty());
        assert_eq!(fixture.printed, vec!["Couldn't resolve Q3 authorization server\n"]);
        // Failures are not cached: the next request resolves again.
        fixture.resolve = Some(authority());
        {
            let mut auth = Q3ClientAuthorization::new(&mut cvars, &mut keys, &mut fixture);
            auth.request(&mut || {}, &mut |to, packet| sent.push((to.clone(), packet.to_vec())))
                .unwrap();
        }
        assert_eq!(sent.len(), 1);
        assert_eq!(fixture.resolves, 2);
    }

    #[test]
    fn anonymous_cvar_flows_into_packet() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars.register("cl_anonymous", "1", flags::INIT | flags::SYSTEM_INFO).unwrap();
        let mut keys = Keys { bytes: [0; 33] };
        let mut fixture = Fixture {
            demo_restricted: true,
            resolve: Some(authority()),
            resolves: 0,
            printed: Vec::new(),
        };
        let mut auth = Q3ClientAuthorization::new(&mut cvars, &mut keys, &mut fixture);
        let mut sent = Vec::new();
        auth.request(&mut || {}, &mut |to, packet| sent.push((to.clone(), packet.to_vec())))
            .unwrap();
        assert_eq!(
            sent[0].1,
            encode_connectionless_text("getKeyAuthorize 1 demota").unwrap()
        );
    }
}
