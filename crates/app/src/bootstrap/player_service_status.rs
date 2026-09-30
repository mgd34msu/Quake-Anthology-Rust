//! Native service endpoint status for player records.
//!
//! Sync port of donor `src/app/bootstrap/player-service-status.ts`. Local
//! records are never native service submission, so a service without an
//! explicitly configured compatible endpoint reports unconfigured.

use qa_net::common::endpoint::NetworkAddress;

/// Native player service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerService {
    /// Authorization service.
    Authorization,
    /// Rankings service.
    Rankings,
}

impl PlayerService {
    /// Donor wire spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::Rankings => "rankings",
        }
    }
}

/// Configuration status of one player service (`PlayerServiceStatus`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerServiceStatus {
    /// No compatible endpoint is configured.
    Unconfigured {
        /// Service.
        service: PlayerService,
        /// Reason line.
        reason: String,
    },
    /// A compatible endpoint is configured.
    Configured {
        /// Service.
        service: PlayerService,
        /// Configured endpoint.
        endpoint: NetworkAddress,
    },
    /// A configured endpoint is currently unreachable.
    Unavailable {
        /// Service.
        service: PlayerService,
        /// Configured endpoint.
        endpoint: NetworkAddress,
        /// Reason line.
        reason: String,
    },
}

/// Report the status of one service for an optional configured endpoint.
#[must_use]
pub fn player_service_status(
    service: PlayerService,
    endpoint: Option<NetworkAddress>,
) -> PlayerServiceStatus {
    match endpoint {
        None => PlayerServiceStatus::Unconfigured {
            service,
            reason: format!(
                "{} requires an explicitly configured compatible endpoint; local records are not native service submission.",
                service.as_str()
            ),
        },
        Some(endpoint) => PlayerServiceStatus::Configured { service, endpoint },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::common::endpoint::ipv4_address;

    #[test]
    fn unconfigured_names_the_service() {
        for service in [PlayerService::Authorization, PlayerService::Rankings] {
            match player_service_status(service, None) {
                PlayerServiceStatus::Unconfigured {
                    service: actual,
                    reason,
                } => {
                    assert_eq!(actual, service);
                    assert!(reason.starts_with(service.as_str()));
                    assert!(reason.contains("local records are not native service submission"));
                }
                other => panic!("expected unconfigured, got {other:?}"),
            }
        }
    }

    #[test]
    fn configured_keeps_the_endpoint() {
        let endpoint = ipv4_address([127, 0, 0, 1], 27910, false).unwrap();
        match player_service_status(PlayerService::Rankings, Some(endpoint.clone())) {
            PlayerServiceStatus::Configured {
                service,
                endpoint: actual,
            } => {
                assert_eq!(service, PlayerService::Rankings);
                assert_eq!(actual, endpoint);
            }
            other => panic!("expected configured, got {other:?}"),
        }
    }

    #[test]
    fn unavailable_variant_is_representable() {
        let endpoint = ipv4_address([10, 0, 0, 2], 27910, false).unwrap();
        let status = PlayerServiceStatus::Unavailable {
            service: PlayerService::Authorization,
            endpoint,
            reason: "timed out".to_string(),
        };
        assert!(matches!(status, PlayerServiceStatus::Unavailable { .. }));
    }
}
