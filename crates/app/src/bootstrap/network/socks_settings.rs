//! Client SOCKS proxy settings.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/socks-settings.ts`
//! (`ClientSocksSettings`). Imported Q3 network settings retain Q3
//! archive/latch semantics for every remote family. The donor's registry
//! constructor takes a command context and print sink; the Rust registry
//! collects notifications instead, so the constructor only takes the
//! registry dialect inputs it needs. The donor's asynchronous `connect`
//! becomes synchronous over [`UdpTransport::connect_socks`].

use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::CommandContext;
use qa_core::cvar::{flags, CvarError, CvarRegistry};
use qa_net::common::socks::{SocksError, SocksOptions};
use qa_net::common::transport::UdpTransport;
use thiserror::Error;

/// SOCKS dial failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SocksSettingsError {
    /// Latched cvar application failed.
    #[error("{0}")]
    Cvar(#[from] CvarError),
    /// SOCKS negotiation failed.
    #[error("{0}")]
    Socks(#[from] SocksError),
}

/// Minimal mirror of the donor `CommandCvarRouting` interface, which has no
/// Rust home yet: registries answer ownership and visibility per source.
pub trait CvarRouting {
    /// Registry owning a name for a source.
    fn owner<'a>(&'a self, name: &str, source: &CommandContext) -> &'a CvarRegistry;
    /// Registries visible from a source.
    fn visible<'a>(&'a self, source: &CommandContext) -> Vec<&'a CvarRegistry>;
}

/// Routing that prefers the SOCKS registry (`route` result).
pub struct SocksCvarRouting<'a> {
    /// Fallback routing.
    pub base: &'a dyn CvarRouting,
    /// SOCKS registry shadowing the base.
    pub socks: &'a CvarRegistry,
}

impl CvarRouting for SocksCvarRouting<'_> {
    fn owner<'a>(&'a self, name: &str, source: &CommandContext) -> &'a CvarRegistry {
        if self.socks.get(name).is_none() {
            return self.base.owner(name, source);
        }
        self.socks
    }

    fn visible<'a>(&'a self, source: &CommandContext) -> Vec<&'a CvarRegistry> {
        let mut registries = self.base.visible(source);
        registries.push(self.socks);
        registries
    }
}

/// Q3 SOCKS settings registry (`ClientSocksSettings`).
pub struct ClientSocksSettings {
    /// Owned settings registry.
    pub cvars: CvarRegistry,
}

impl ClientSocksSettings {
    /// Register the `net_socks*` cvars with archive/latch semantics.
    pub fn new() -> Result<Self, CvarError> {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        let latched = flags::ARCHIVE | flags::LATCH;
        cvars.register("net_socksEnabled", "0", latched)?;
        cvars.register("net_socksServer", "", latched)?;
        cvars.register("net_socksPort", "1080", latched)?;
        cvars.register("net_socksUsername", "", latched)?;
        cvars.register("net_socksPassword", "", latched)?;
        Ok(Self { cvars })
    }

    /// Shadow a base routing with this registry (`route`).
    #[must_use]
    pub fn route<'a>(&'a self, base: &'a dyn CvarRouting) -> SocksCvarRouting<'a> {
        SocksCvarRouting {
            base,
            socks: &self.cvars,
        }
    }

    /// Apply latched values and dial the proxy when enabled (`connect`).
    ///
    /// Reads `other` when supplied (so callers can stage values in a
    /// scratch registry first), else the owned registry.
    pub fn connect(
        &mut self,
        transport: &UdpTransport,
        other: Option<&mut CvarRegistry>,
    ) -> Result<(), SocksSettingsError> {
        let registry = other.unwrap_or(&mut self.cvars);
        Self::connect_registry(registry, transport)
    }

    /// Apply latched values on `registry` and dial when enabled.
    fn connect_registry(registry: &mut CvarRegistry, transport: &UdpTransport) -> Result<(), SocksSettingsError> {
        registry.apply_latched(None)?;
        if registry
            .get("net_socksEnabled")
            .map_or(0, |snapshot| snapshot.integer_value)
            == 0
        {
            return Ok(());
        }
        let port = registry
            .get("net_socksPort")
            .map_or(1080, |snapshot| snapshot.integer_value)
            & 65535;
        transport.connect_socks(&SocksOptions {
            server: registry.variable_string("net_socksServer"),
            port: u32::try_from(port).unwrap_or(0),
            username: registry.variable_string("net_socksUsername"),
            password: registry.variable_string("net_socksPassword"),
        })?;
        Ok(())
    }
}

impl Default for ClientSocksSettings {
    fn default() -> Self {
        Self::new().expect("SOCKS cvars register")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd_buffer::CommandOrigin;
    use qa_core::identity::IdentityOwner;

    struct BaseRouting {
        registry: CvarRegistry,
    }

    impl CvarRouting for BaseRouting {
        fn owner<'a>(&'a self, _name: &str, _source: &CommandContext) -> &'a CvarRegistry {
            &self.registry
        }

        fn visible<'a>(&'a self, _source: &CommandContext) -> Vec<&'a CvarRegistry> {
            vec![&self.registry]
        }
    }

    fn context() -> CommandContext {
        let owner = IdentityOwner::create("socks-test").expect("owner");
        CommandContext::new(owner.session().clone(), CommandOrigin::LocalConsole)
    }

    #[test]
    fn registers_socks_cvars() {
        let settings = ClientSocksSettings::default();
        assert_eq!(settings.cvars.variable_string("net_socksPort"), "1080");
        assert_eq!(settings.cvars.dialect(), Dialect::Q3);
    }

    #[test]
    fn routes_socks_names_to_socks_registry() {
        let settings = ClientSocksSettings::default();
        let base = BaseRouting {
            registry: CvarRegistry::new(Dialect::Q3),
        };
        let routed = settings.route(&base);
        let source = context();
        assert!(std::ptr::eq(routed.owner("net_socksPort", &source), &settings.cvars));
        assert!(std::ptr::eq(routed.owner("sensitivity", &source), &base.registry));
        assert_eq!(routed.visible(&source).len(), 2);
    }

    #[test]
    fn disabled_proxy_skips_dial() {
        let mut settings = ClientSocksSettings::default();
        let transport =
            UdpTransport::bind(&qa_net::common::transport::UdpBindOptions::new("127.0.0.1", 0)).expect("bind");
        settings.connect(&transport, None).expect("no-op");
    }

    #[test]
    fn applies_latched_before_connect() {
        let mut settings = ClientSocksSettings::default();
        settings.cvars.stage("net_socksPort", "9050").expect("stage port");
        let transport =
            UdpTransport::bind(&qa_net::common::transport::UdpBindOptions::new("127.0.0.1", 0)).expect("bind");
        settings.connect(&transport, None).expect("connect");
        assert_eq!(settings.cvars.variable_string("net_socksPort"), "9050");
    }
}
