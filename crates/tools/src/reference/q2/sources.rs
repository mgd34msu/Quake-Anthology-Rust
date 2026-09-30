//! Q2 pinned sources (donor `tools/reference/q2/sources.ts`).
//!
//! SHA-256 locks over the classic and rerelease sources backing the Q2
//! oracle, plus verified loading with the donor's double-hash guard: the
//! identity hash must match the lock, and the bytes must still match when
//! read for evaluation.

use std::collections::HashMap;
use std::path::Path;

use crate::error::ToolsError;
use crate::json::Json;
use crate::reference::environment::identify_file;
use crate::verify::hash::hash_bytes;

/// A pinned source file lock.
#[derive(Debug, Clone, Copy)]
pub struct SourceLock {
    /// Stable lock identifier.
    pub id: &'static str,
    /// Project-relative path.
    pub path: &'static str,
    /// SHA-256 hex of the locked bytes.
    pub sha256: &'static str,
}

/// Locked Q2 sources (donor `sourceLocks` order).
pub const SOURCE_LOCKS: [SourceLock; 18] = [
    SourceLock { id: "classicLocal", path: "../qsrc/quake-2/game/g_local.h", sha256: "eb04d914a532f0baa87a12e00bc2e39a3d3a8ee1576bcdf87ea5af419919e095" },
    SourceLock { id: "classicMain", path: "../qsrc/quake-2/game/g_main.c", sha256: "558f05adc5adac93bdc6151f5157b009ac87c255f788240a2a9f41755e3c8fd4" },
    SourceLock { id: "classicPhys", path: "../qsrc/quake-2/game/g_phys.c", sha256: "31e3b813814c249734550d262d2b4766184849563960c0d9715005a6e5743481" },
    SourceLock { id: "classicItems", path: "../qsrc/quake-2/game/g_items.c", sha256: "4d55d1d1c83ff7552326efabc520ca07b1107ef37cd7a877af409eddcd6cdc77" },
    SourceLock { id: "classicCombat", path: "../qsrc/quake-2/game/g_combat.c", sha256: "ee8badcf891215ae3e627a987dadde2e4bf11eb19095e3fca9e1b60c0c658f38" },
    SourceLock { id: "classicMove", path: "../qsrc/quake-2/qcommon/pmove.c", sha256: "8861daaeff7efd7bc6d3c5e65ef99801567f3e06eb14020787206d39dd0038a8" },
    SourceLock { id: "rereleaseReadme", path: "../qsrc/quake2-rerelease-dll/README.md", sha256: "5767fe06b561ee01ee2817e82331577bc6b44e4e7b794c87f275d0c165095c9b" },
    SourceLock { id: "rereleaseLocal", path: "../qsrc/quake2-rerelease-dll/rerelease/g_local.h", sha256: "3257c79f07d9e8ef333342b9a0be0bde7dd514d9de1b9b36173aad157601aecf" },
    SourceLock { id: "rereleaseMain", path: "../qsrc/quake2-rerelease-dll/rerelease/g_main.cpp", sha256: "438e2dcb631b94ff4501e74230237ea739e210da331dfce6e456c6154338d1b5" },
    SourceLock { id: "rereleasePhys", path: "../qsrc/quake2-rerelease-dll/rerelease/g_phys.cpp", sha256: "c09d96106cee08d30bfca282159a487d91a93c852870b3f1ed6b21a4df6b4737" },
    SourceLock { id: "rereleaseTarget", path: "../qsrc/quake2-rerelease-dll/rerelease/g_target.cpp", sha256: "7a620d794956f4dc1bef050cc621984ab51bd7cd4f7a4ce5a0693777e3be4cd9" },
    SourceLock { id: "rereleaseSave", path: "../qsrc/quake2-rerelease-dll/rerelease/g_save.cpp", sha256: "ec2a980c3dd9412a31754e478b33b400b08e30f7cbdfcd964ec516cdda3597c4" },
    SourceLock { id: "rereleaseSpawn", path: "../qsrc/quake2-rerelease-dll/rerelease/g_spawn.cpp", sha256: "9b78e2a1c8e2a739a450add01006e4cf39f0aabc1441b55c30be2f425fcb3742" },
    SourceLock { id: "rereleaseClient", path: "../qsrc/quake2-rerelease-dll/rerelease/p_client.cpp", sha256: "b47a44e43573c1e471ae29b392cd9f603d16310eedecdc84ac09b927aa3cbacf" },
    SourceLock { id: "rereleaseMove", path: "../qsrc/quake2-rerelease-dll/rerelease/p_move.cpp", sha256: "0bed089782386fa1d9a4872a0bcfdae97f5d967a7d8bc3e6dc3fa3dd23dc2cc5" },
    SourceLock { id: "rereleaseGame", path: "../qsrc/quake2-rerelease-dll/rerelease/game.h", sha256: "66defbd069c38c4a1740c5087a3f6f2343d603b956ca0d6e1d89e8ad46d0843f" },
    SourceLock { id: "rereleaseShared", path: "../qsrc/quake2-rerelease-dll/rerelease/bg_local.h", sha256: "0a66fff4539f15603a8563c150b56b4be64bdd36144053730a3eb1301feaad79" },
    SourceLock { id: "rereleaseCgame", path: "../qsrc/quake2-rerelease-dll/rerelease/cg_main.cpp", sha256: "a673f0092f5fffa968fd3affc3d0cb57b02a535ff7c14a04326578bcd8371c90" },
];

/// A locked source identifier (donor `SourceId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceId {
    /// Classic `g_local.h`.
    ClassicLocal,
    /// Classic `g_main.c`.
    ClassicMain,
    /// Classic `g_phys.c`.
    ClassicPhys,
    /// Classic `g_items.c`.
    ClassicItems,
    /// Classic `g_combat.c`.
    ClassicCombat,
    /// Classic `pmove.c`.
    ClassicMove,
    /// Rerelease `README.md`.
    RereleaseReadme,
    /// Rerelease `g_local.h`.
    RereleaseLocal,
    /// Rerelease `g_main.cpp`.
    RereleaseMain,
    /// Rerelease `g_phys.cpp`.
    RereleasePhys,
    /// Rerelease `g_target.cpp`.
    RereleaseTarget,
    /// Rerelease `g_save.cpp`.
    RereleaseSave,
    /// Rerelease `g_spawn.cpp`.
    RereleaseSpawn,
    /// Rerelease `p_client.cpp`.
    RereleaseClient,
    /// Rerelease `p_move.cpp`.
    RereleaseMove,
    /// Rerelease `game.h`.
    RereleaseGame,
    /// Rerelease `bg_local.h`.
    RereleaseShared,
    /// Rerelease `cg_main.cpp`.
    RereleaseCgame,
}

impl SourceId {
    /// Every source identifier in lock order.
    pub const ALL: [Self; 18] = [
        Self::ClassicLocal,
        Self::ClassicMain,
        Self::ClassicPhys,
        Self::ClassicItems,
        Self::ClassicCombat,
        Self::ClassicMove,
        Self::RereleaseReadme,
        Self::RereleaseLocal,
        Self::RereleaseMain,
        Self::RereleasePhys,
        Self::RereleaseTarget,
        Self::RereleaseSave,
        Self::RereleaseSpawn,
        Self::RereleaseClient,
        Self::RereleaseMove,
        Self::RereleaseGame,
        Self::RereleaseShared,
        Self::RereleaseCgame,
    ];

    /// The donor lock key.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClassicLocal => "classicLocal",
            Self::ClassicMain => "classicMain",
            Self::ClassicPhys => "classicPhys",
            Self::ClassicItems => "classicItems",
            Self::ClassicCombat => "classicCombat",
            Self::ClassicMove => "classicMove",
            Self::RereleaseReadme => "rereleaseReadme",
            Self::RereleaseLocal => "rereleaseLocal",
            Self::RereleaseMain => "rereleaseMain",
            Self::RereleasePhys => "rereleasePhys",
            Self::RereleaseTarget => "rereleaseTarget",
            Self::RereleaseSave => "rereleaseSave",
            Self::RereleaseSpawn => "rereleaseSpawn",
            Self::RereleaseClient => "rereleaseClient",
            Self::RereleaseMove => "rereleaseMove",
            Self::RereleaseGame => "rereleaseGame",
            Self::RereleaseShared => "rereleaseShared",
            Self::RereleaseCgame => "rereleaseCgame",
        }
    }
}

/// A line span inside a locked source (donor `SourceLocation`).
#[derive(Debug, Clone, Copy)]
pub struct SourceLocation {
    /// Locked source.
    pub source: SourceId,
    /// First line (1-based, inclusive).
    pub first_line: u32,
    /// Last line (1-based, inclusive).
    pub last_line: u32,
    /// Pinned symbol.
    pub symbol: &'static str,
}

impl SourceLocation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("source".to_owned(), Json::string(self.source.as_str())),
            ("firstLine".to_owned(), Json::uint(u64::from(self.first_line))),
            ("lastLine".to_owned(), Json::uint(u64::from(self.last_line))),
            ("symbol".to_owned(), Json::string(self.symbol)),
        ])
    }
}

/// A verified source identity: lock id plus file identity (donor spread order).
#[derive(Debug, Clone)]
pub struct SourcedIdentity {
    /// Lock identifier.
    pub id: String,
    /// Canonical file path.
    pub path: String,
    /// Byte size.
    pub size: u64,
    /// SHA-256 hex of the bytes.
    pub sha256: String,
}

impl SourcedIdentity {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("path".to_owned(), Json::string(&self.path)),
            ("size".to_owned(), Json::uint(self.size)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
        ])
    }
}

/// Verified source identities plus decoded source text.
#[derive(Debug, Clone)]
pub struct VerifiedSources {
    /// Identities in lock order.
    pub identities: Vec<SourcedIdentity>,
    /// Source text by lock id.
    pub text: HashMap<String, String>,
}

/// Load and verify every locked source under `project_root`.
pub fn load_verified_sources(project_root: &Path) -> Result<VerifiedSources, ToolsError> {
    let mut identities = Vec::with_capacity(SOURCE_LOCKS.len());
    let mut text = HashMap::with_capacity(SOURCE_LOCKS.len());
    for lock in SOURCE_LOCKS {
        let path = project_root.join(lock.path);
        let path_text = path.to_string_lossy().into_owned();
        let identity = identify_file(&path_text)?;
        if identity.sha256 != lock.sha256 {
            return Err(ToolsError::invalid(format!("Q2 source identity changed: {path_text}")));
        }
        let bytes = crate::fsutil::read_bytes(&path)?;
        if hash_bytes(&bytes) != lock.sha256 {
            return Err(ToolsError::invalid(format!("Q2 source changed before evaluation: {path_text}")));
        }
        identities.push(SourcedIdentity {
            id: lock.id.to_owned(),
            path: identity.path,
            size: identity.size,
            sha256: identity.sha256,
        });
        text.insert(
            lock.id.to_owned(),
            String::from_utf8(bytes)
                .map_err(|_| ToolsError::parse(format!("Q2 source is not valid UTF-8: {path_text}")))?,
        );
    }
    Ok(VerifiedSources { identities, text })
}

/// Verified text for one locked source.
pub fn source_text<'a>(sources: &'a HashMap<String, String>, id: &str) -> Result<&'a str, ToolsError> {
    sources
        .get(id)
        .map(String::as_str)
        .ok_or_else(|| ToolsError::invalid(format!("Missing verified Q2 source: {id}")))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn lock_ids_are_unique() {
        let mut ids = BTreeSet::new();
        for lock in SOURCE_LOCKS {
            assert!(ids.insert(lock.id), "duplicate lock {}", lock.id);
            assert_eq!(lock.sha256.len(), 64, "lock {}", lock.id);
        }
    }

    #[test]
    fn source_ids_cover_locks_in_order() {
        assert_eq!(SourceId::ALL.len(), SOURCE_LOCKS.len());
        for (id, lock) in SourceId::ALL.iter().zip(SOURCE_LOCKS.iter()) {
            assert_eq!(id.as_str(), lock.id);
        }
    }

    #[test]
    fn missing_source_reports_id() {
        let sources = HashMap::new();
        let error = source_text(&sources, "classicMain").expect_err("missing");
        assert_eq!(error.to_string(), "Missing verified Q2 source: classicMain");
    }
}
