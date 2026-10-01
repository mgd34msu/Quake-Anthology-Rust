//! Client-side automatic download policy.
//!
//! Port of `src/app/bootstrap/network/client-download-policy.ts`. The donor
//! registers Q2/Q3 latched cvars and returns a permission closure; this port
//! keeps that shape over [`CvarRegistry`](qa_core::cvar::CvarRegistry). The
//! local policy never changes the server's download rules.

use qa_core::cvar::{flags, q2_flags, CvarError, CvarRegistry};

/// Download asset category (`ClientDownloadCategory`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDownloadCategory {
    /// File-list metadata.
    Metadata,
    /// Archive package (`.pak`/`.pkz`/`.pk3`).
    Package,
    /// Map, environment, or texture path.
    Map,
    /// Model path.
    Model,
    /// Sound path.
    Sound,
    /// Player skin path.
    Player,
    /// Any other picture path.
    Picture,
}

/// One permission query (`ClientDownloadPermission` request).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientDownloadRequest {
    /// Transport family.
    pub transport: ClientDownloadTransport,
    /// Asset category.
    pub category: ClientDownloadCategory,
}

/// Download transport (`'http' | 'native'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDownloadTransport {
    /// HTTP download.
    Http,
    /// Native in-protocol download.
    Native,
}

/// Game family selecting the cvar set (`'q2' | 'q3'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDownloadFamily {
    /// Quake II policy cvars.
    Q2,
    /// Quake III policy cvar.
    Q3,
}

/// Classify a download path (`clientDownloadCategory`).
#[must_use]
pub fn client_download_category(path: &str) -> ClientDownloadCategory {
    let name = path.to_lowercase();
    if name.ends_with(".pak") || name.ends_with(".pkz") || name.ends_with(".pk3") {
        return ClientDownloadCategory::Package;
    }
    if name.starts_with("maps/") || name.starts_with("env/") || name.starts_with("textures/") {
        return ClientDownloadCategory::Map;
    }
    if name.starts_with("players/") {
        return ClientDownloadCategory::Player;
    }
    if name.starts_with("models/") {
        return ClientDownloadCategory::Model;
    }
    if name.starts_with("sound/") {
        return ClientDownloadCategory::Sound;
    }
    ClientDownloadCategory::Picture
}

/// Permission checker returned by [`create_client_download_permission`].
///
/// The donor returns a closure over its registry; Rust cannot lend the
/// registry to a closure and also mutate it, so the family tag travels in
/// this struct and each [`check`](Self::check) call reads the live registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientDownloadPermission {
    /// Game family selecting the cvar set.
    pub family: ClientDownloadFamily,
}

impl ClientDownloadPermission {
    /// Answer one permission query against the live registry.
    #[must_use]
    pub fn check(&self, cvars: &CvarRegistry, request: &ClientDownloadRequest) -> bool {
        if self.family == ClientDownloadFamily::Q3 {
            return (cvars
                .get("cl_allowDownload")
                .map_or(0, |snapshot| snapshot.integer_value))
                != 0;
        }
        if cvars.variable_value("allow_download") <= 0.0 {
            return false;
        }
        if request.transport == ClientDownloadTransport::Http && cvars.variable_value("cl_http_downloads") == 0.0 {
            return false;
        }
        match request.category {
            ClientDownloadCategory::Map => cvars.variable_value("allow_download_maps") != 0.0,
            ClientDownloadCategory::Model => cvars.variable_value("allow_download_models") != 0.0,
            ClientDownloadCategory::Sound => cvars.variable_value("allow_download_sounds") != 0.0,
            ClientDownloadCategory::Player => cvars.variable_value("allow_download_players") != 0.0,
            // Archives can contain every category; never use them to bypass
            // an asset restriction.
            ClientDownloadCategory::Package => ["maps", "models", "sounds", "players"]
                .iter()
                .all(|name| cvars.variable_value(&format!("allow_download_{name}")) != 0.0),
            ClientDownloadCategory::Metadata | ClientDownloadCategory::Picture => true,
        }
    }
}

/// Register the family policy cvars and return the permission checker
/// (`createClientDownloadPermission`).
///
/// Nonpositive `allow_download` disables Q2 HTTP and native automatic
/// transfers (the donor corrects its own `-1` inconsistency this way).
pub fn create_client_download_permission(
    cvars: &mut CvarRegistry,
    family: ClientDownloadFamily,
) -> Result<ClientDownloadPermission, CvarError> {
    if family == ClientDownloadFamily::Q3 {
        cvars.register("cl_allowDownload", "0", flags::ARCHIVE)?;
        return Ok(ClientDownloadPermission { family });
    }
    cvars.register("allow_download", "1", q2_flags::ARCHIVE)?;
    cvars.register("cl_http_downloads", "1", q2_flags::ARCHIVE)?;
    for category in ["maps", "models", "sounds", "players"] {
        cvars.register(&format!("allow_download_{category}"), "1", q2_flags::ARCHIVE)?;
    }
    Ok(ClientDownloadPermission { family })
}

/// One in-flight download (`ClientDownloadProgress`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientDownloadProgress {
    /// Requested path.
    pub path: String,
    /// Transport family.
    pub transport: ClientDownloadTransport,
    /// Bytes received.
    pub received: u64,
    /// Total bytes, when advertised.
    pub total: Option<u64>,
    /// Completion percent, when computable.
    pub percent: Option<f64>,
    /// Transfer phase.
    pub phase: ClientDownloadPhase,
}

/// Transfer phase (`'pending' | 'running' | 'done'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDownloadPhase {
    /// Queued.
    Pending,
    /// In flight.
    Running,
    /// Finished.
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    fn request(transport: ClientDownloadTransport, category: ClientDownloadCategory) -> ClientDownloadRequest {
        ClientDownloadRequest { transport, category }
    }

    #[test]
    fn classifies_paths() {
        assert_eq!(client_download_category("maps/q2dm1.bsp"), ClientDownloadCategory::Map);
        assert_eq!(client_download_category("ENV/sky.tga"), ClientDownloadCategory::Map);
        assert_eq!(
            client_download_category("textures/wall.wal"),
            ClientDownloadCategory::Map
        );
        assert_eq!(
            client_download_category("players/male/tris.md2"),
            ClientDownloadCategory::Player
        );
        assert_eq!(
            client_download_category("models/weapons/v_shot.md2"),
            ClientDownloadCategory::Model
        );
        assert_eq!(
            client_download_category("sound/world/wind.wav"),
            ClientDownloadCategory::Sound
        );
        assert_eq!(
            client_download_category("baseq2/maps.pkz"),
            ClientDownloadCategory::Package
        );
        assert_eq!(
            client_download_category("PICS/health.PCX"),
            ClientDownloadCategory::Picture
        );
    }

    #[test]
    fn q2_defaults_allow_everything() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        let permission =
            create_client_download_permission(&mut cvars, ClientDownloadFamily::Q2).expect("register policy");
        for category in [
            ClientDownloadCategory::Metadata,
            ClientDownloadCategory::Package,
            ClientDownloadCategory::Map,
            ClientDownloadCategory::Model,
            ClientDownloadCategory::Sound,
            ClientDownloadCategory::Player,
            ClientDownloadCategory::Picture,
        ] {
            assert!(permission.check(&cvars, &request(ClientDownloadTransport::Native, category)));
            assert!(permission.check(&cvars, &request(ClientDownloadTransport::Http, category)));
        }
    }

    #[test]
    fn q2_gates_http_and_categories() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        let permission =
            create_client_download_permission(&mut cvars, ClientDownloadFamily::Q2).expect("register policy");
        cvars.set("cl_http_downloads", "0", true).expect("set");
        assert!(!permission.check(
            &cvars,
            &request(ClientDownloadTransport::Http, ClientDownloadCategory::Map)
        ));
        assert!(permission.check(
            &cvars,
            &request(ClientDownloadTransport::Native, ClientDownloadCategory::Map)
        ));
        cvars.set("allow_download_maps", "0", true).expect("set");
        assert!(!permission.check(
            &cvars,
            &request(ClientDownloadTransport::Native, ClientDownloadCategory::Map)
        ));
        assert!(!permission.check(
            &cvars,
            &request(ClientDownloadTransport::Native, ClientDownloadCategory::Package)
        ));
        assert!(permission.check(
            &cvars,
            &request(ClientDownloadTransport::Native, ClientDownloadCategory::Model)
        ));
    }

    #[test]
    fn q2_nonpositive_master_disables_all() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        let permission =
            create_client_download_permission(&mut cvars, ClientDownloadFamily::Q2).expect("register policy");
        cvars.set("allow_download", "-1", true).expect("set");
        assert!(!permission.check(
            &cvars,
            &request(ClientDownloadTransport::Native, ClientDownloadCategory::Metadata)
        ));
    }

    #[test]
    fn q3_uses_single_toggle() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        let permission =
            create_client_download_permission(&mut cvars, ClientDownloadFamily::Q3).expect("register policy");
        assert!(!permission.check(
            &cvars,
            &request(ClientDownloadTransport::Native, ClientDownloadCategory::Map)
        ));
        cvars.set("cl_allowDownload", "1", true).expect("set");
        assert!(permission.check(
            &cvars,
            &request(ClientDownloadTransport::Http, ClientDownloadCategory::Package)
        ));
    }
}
