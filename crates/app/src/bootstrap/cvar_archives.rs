//! Archived cvar persistence.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/cvar-archives.ts`
//! (`loadCvarArchive`, `saveCvarArchive`).
//! Async store access becomes sync [`ConfigStore`] reads and writes.

use std::collections::HashSet;

use qa_content::contract::encode_uri_component;
use qa_core::cmd::Dialect;
use qa_core::cvar::{CvarArchiveEntry, CvarRegistry};
use thiserror::Error;

use crate::console::llm_batch::dialect_name;
use crate::settings::config::ConfigStore;
use crate::settings::json::{parse_json, stringify, Json};
use crate::settings::SettingsError;

/// Cvar archive failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CvarArchiveError {
    /// Archive text is not a cvar archive document.
    #[error("Invalid cvar archive: {0}")]
    Invalid(String),
    /// Archive holds two entries with one name.
    #[error("duplicate archived cvar")]
    Duplicate,
    /// Store or JSON failure.
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

/// Archive owner head (`CvarArchiveOwner[0]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CvarArchiveHead {
    /// Source game profile.
    Source,
    /// Client profile.
    Client,
    /// Input profile.
    Input,
    /// Movement profile.
    Movement,
    /// Fallback profile.
    Fallback,
}

impl CvarArchiveHead {
    /// Owner head text.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Client => "client",
            Self::Input => "input",
            Self::Movement => "movement",
            Self::Fallback => "fallback",
        }
    }
}

/// Archive owner path (`CvarArchiveOwner`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarArchiveOwner {
    /// Owner head.
    pub head: CvarArchiveHead,
    /// Remaining owner path parts.
    pub rest: Vec<String>,
}

impl CvarArchiveOwner {
    /// Build an owner path.
    #[must_use]
    pub fn new(head: CvarArchiveHead, rest: Vec<String>) -> Self {
        Self { head, rest }
    }
}

/// Archive path (`cvars/<owner...>.json`, parts URI-encoded).
#[must_use]
pub fn cvar_archive_path(owner: &CvarArchiveOwner) -> String {
    let mut parts = vec![owner.head.name().to_string()];
    parts.extend(owner.rest.iter().cloned());
    format!(
        "cvars/{}.json",
        parts
            .iter()
            .map(|part| encode_uri_component(part))
            .collect::<Vec<_>>()
            .join("/")
    )
}

fn invalid(message: impl Into<String>) -> CvarArchiveError {
    CvarArchiveError::Invalid(message.into())
}

/// Load archived entries, or an empty list when no archive exists
/// (`loadCvarArchive`).
pub fn load_cvar_archive(
    store: &ConfigStore,
    owner: &CvarArchiveOwner,
    dialect: Dialect,
) -> Result<Vec<CvarArchiveEntry>, CvarArchiveError> {
    let Some(text) = store.load_text(&cvar_archive_path(owner))? else {
        return Ok(Vec::new());
    };
    let value = parse_json(&text).map_err(|_| invalid("cvar archive is not JSON"))?;
    let version = value.get("version");
    if !matches!(version, Some(Json::Number(version)) if *version == 1.0) {
        return Err(invalid("cvar archive version must be 1"));
    }
    if value.get("dialect") != Some(&Json::String(dialect_name(dialect).to_string())) {
        return Err(invalid("cvar archive dialect does not match"));
    }
    let Some(Json::Array(items)) = value.get("entries") else {
        return Err(invalid("cvar archive entries must be a list"));
    };
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        match (item.get("name"), item.get("value")) {
            (Some(Json::String(name)), Some(Json::String(value))) => entries.push(CvarArchiveEntry {
                name: name.clone(),
                value: value.clone(),
            }),
            _ => return Err(invalid("cvar archive entry needs a name and value")),
        }
    }
    let names: HashSet<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    if names.len() != entries.len() {
        return Err(CvarArchiveError::Duplicate);
    }
    Ok(entries)
}

/// Save archived entries, defaulting to the registry's archive entries
/// (`saveCvarArchive`).
pub fn save_cvar_archive(
    store: &ConfigStore,
    owner: &CvarArchiveOwner,
    registry: &CvarRegistry,
    entries: Option<&[CvarArchiveEntry]>,
) -> Result<(), CvarArchiveError> {
    let fallback;
    let entries = match entries {
        Some(entries) => entries,
        None => {
            fallback = registry.archive_entries(&|_| true);
            &fallback
        }
    };
    let document = Json::Object(vec![
        ("version".to_string(), Json::Number(1.0)),
        (
            "dialect".to_string(),
            Json::String(dialect_name(registry.dialect()).to_string()),
        ),
        (
            "entries".to_string(),
            Json::Array(
                entries
                    .iter()
                    .map(|entry| {
                        Json::Object(vec![
                            ("name".to_string(), Json::String(entry.name.clone())),
                            ("value".to_string(), Json::String(entry.value.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    store.dump(&cvar_archive_path(owner), &format!("{}\n", stringify(&document)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cvar::flags;

    fn store() -> ConfigStore {
        let root = std::env::temp_dir().join(format!(
            "qa-cvar-archive-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        ConfigStore { root }
    }

    fn owner() -> CvarArchiveOwner {
        CvarArchiveOwner::new(
            CvarArchiveHead::Source,
            vec!["q2-classic baseq2".to_string(), "provider".to_string()],
        )
    }

    fn registry() -> CvarRegistry {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars.register("sensitivity", "3", flags::ARCHIVE).unwrap();
        cvars.set("sensitivity", "5", true).unwrap();
        cvars.register("name", "player", 0).unwrap();
        cvars
    }

    #[test]
    fn archive_path_encodes_owner_parts() {
        assert_eq!(
            cvar_archive_path(&owner()),
            "cvars/source/q2-classic%20baseq2/provider.json"
        );
    }

    #[test]
    fn missing_archive_loads_empty() {
        assert_eq!(load_cvar_archive(&store(), &owner(), Dialect::Q3).unwrap(), Vec::new());
    }

    #[test]
    fn round_trip_preserves_archive_entries() {
        let store = store();
        let cvars = registry();
        save_cvar_archive(&store, &owner(), &cvars, None).unwrap();
        let entries = load_cvar_archive(&store, &owner(), Dialect::Q3).unwrap();
        assert_eq!(
            entries,
            vec![CvarArchiveEntry {
                name: "sensitivity".to_string(),
                value: "5".to_string(),
            }]
        );
        let _ = std::fs::remove_dir_all(&store.root);
    }

    #[test]
    fn explicit_entries_override_registry() {
        let store = store();
        let cvars = registry();
        let entries = vec![CvarArchiveEntry {
            name: "custom".to_string(),
            value: "7".to_string(),
        }];
        save_cvar_archive(&store, &owner(), &cvars, Some(&entries)).unwrap();
        assert_eq!(load_cvar_archive(&store, &owner(), Dialect::Q3).unwrap(), entries);
        let _ = std::fs::remove_dir_all(&store.root);
    }

    #[test]
    fn rejects_version_dialect_and_duplicate_mismatches() {
        let store = store();
        store
            .dump(
                &cvar_archive_path(&owner()),
                "{\"version\":2,\"dialect\":\"q3\",\"entries\":[]}\n",
            )
            .unwrap();
        assert!(load_cvar_archive(&store, &owner(), Dialect::Q3).is_err());
        store
            .dump(
                &cvar_archive_path(&owner()),
                "{\"version\":1,\"dialect\":\"q2-classic\",\"entries\":[]}\n",
            )
            .unwrap();
        assert!(load_cvar_archive(&store, &owner(), Dialect::Q3).is_err());
        store
            .dump(
                &cvar_archive_path(&owner()),
                "{\"version\":1,\"dialect\":\"q3\",\"entries\":[{\"name\":\"a\",\"value\":\"1\"},{\"name\":\"a\",\"value\":\"2\"}]}\n",
            )
            .unwrap();
        assert_eq!(
            load_cvar_archive(&store, &owner(), Dialect::Q3).unwrap_err(),
            CvarArchiveError::Duplicate
        );
        let _ = std::fs::remove_dir_all(&store.root);
    }
}
