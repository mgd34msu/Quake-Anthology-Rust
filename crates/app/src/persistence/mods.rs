//! Gameplay mod persistence ported from `src/persistence/mods.ts`.
//!
//! Resolved mod records, mod identities, and mod session checkpoints.
//! Declarations dispatch on `runtime` (`quakec`, `qvm`, `native`) and are
//! retained verbatim; per-runtime callback-shape validation belongs to
//! the content mods readers (donor `src/content/mods/*`), which this
//! crate does not port.

use std::collections::HashSet;

use qa_guest::checkpoint::{read_guest, read_module, write_guest, write_module, GuestCheckpoint, ModuleIdentity};
use qa_world::save::ownership::{read_provider_checkpoint, write_provider_checkpoint, ProviderCheckpoint};
use qa_world::save::shared::{read_digest, read_provider_ref, write_provider_ref, ProviderRef};
use qa_world::save::value::{arr, int, namespaced, obj, str, SaveJson, SaveReader};

use super::PersistenceError;

/// Mod selection: package plus authored component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModSelection {
    /// Package.
    pub product: String,
    /// Component.
    pub id: String,
}

/// Whether a product name is well-formed (shared with startup options).
pub(crate) fn valid_product(product: &str) -> bool {
    let mut chars = product.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '+' | '-'))
}

fn valid_component(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '+' | ':' | '/' | '-'))
}

/// Build a `PRODUCT/COMPONENT` selection key.
pub fn mod_selection_key(selection: &ModSelection) -> Result<String, PersistenceError> {
    if !valid_product(&selection.product) || !valid_component(&selection.id) {
        return Err(PersistenceError::BadSave(
            "Mod selection requires a package and an authored component ID".to_string(),
        ));
    }
    Ok(format!("{}/{}", selection.product, selection.id))
}

/// Parse a `PRODUCT/COMPONENT` selection key.
pub fn read_mod_selection(value: &str) -> Result<ModSelection, PersistenceError> {
    match value.find('/') {
        Some(slash) if slash >= 1 => {
            let selection = ModSelection {
                product: value[..slash].to_string(),
                id: value[slash + 1..].to_string(),
            };
            if mod_selection_key(&selection).as_deref() == Ok(value) {
                Ok(selection)
            } else {
                Err(PersistenceError::BadSave(
                    "Mod selection must be PRODUCT/COMPONENT_ID".to_string(),
                ))
            }
        }
        _ => Err(PersistenceError::BadSave(
            "Mod selection must be PRODUCT/COMPONENT_ID".to_string(),
        )),
    }
}

fn read_selection(reader: SaveReader) -> Result<ModSelection, PersistenceError> {
    let selected = ModSelection {
        product: reader.field("product").string()?,
        id: reader.field("id").string()?,
    };
    mod_selection_key(&selected).map_err(|_| PersistenceError::from(reader.fail("invalid mod selection")))?;
    Ok(selected)
}

fn write_selection(selection: &ModSelection) -> SaveJson {
    obj(vec![("product", str(&selection.product)), ("id", str(&selection.id))])
}

/// Resolved gameplay mod with its retained declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGameplayMod {
    /// Selection.
    pub selection: ModSelection,
    /// Source provider.
    pub source: ProviderRef,
    /// Title.
    pub title: String,
    /// Source title.
    pub source_title: String,
    /// Requirements.
    pub requires: Vec<ModSelection>,
    /// Conflicts.
    pub conflicts: Vec<ModSelection>,
    /// Declaration runtime.
    pub declaration_runtime: String,
    /// Retained declaration record.
    pub declaration: SaveJson,
    /// Declaration digest.
    pub declaration_digest: String,
}

/// Read a resolved gameplay mod.
pub fn read_gameplay_mod(reader: SaveReader) -> Result<ResolvedGameplayMod, PersistenceError> {
    let dependency = |value: SaveReader| -> Result<ModSelection, PersistenceError> {
        read_mod_selection(&format!(
            "{}/{}",
            value.field("product").string()?,
            value.field("id").string()?
        ))
    };
    let declaration = reader.field("declaration");
    Ok(ResolvedGameplayMod {
        selection: read_selection(reader.field("selection"))?,
        source: read_provider_ref(reader.field("source"))?,
        title: reader.field("title").string()?,
        source_title: reader.field("sourceTitle").string()?,
        requires: reader.field("requires").list(dependency)?,
        conflicts: reader.field("conflicts").list(dependency)?,
        declaration_runtime: declaration.field("runtime").choice_str(&["quakec", "qvm", "native"])?,
        declaration: declaration.value.cloned().unwrap_or(SaveJson::Null),
        declaration_digest: read_digest(reader.field("declarationDigest"))?,
    })
}

/// Write a resolved gameplay mod.
#[must_use]
pub fn write_gameplay_mod(value: &ResolvedGameplayMod) -> SaveJson {
    let dependency = |selection: &ModSelection| write_selection(selection);
    obj(vec![
        ("selection", write_selection(&value.selection)),
        ("source", write_provider_ref(&value.source)),
        ("title", str(&value.title)),
        ("sourceTitle", str(&value.source_title)),
        ("requires", arr(value.requires.iter().map(dependency).collect())),
        ("conflicts", arr(value.conflicts.iter().map(dependency).collect())),
        ("declaration", value.declaration.clone()),
        ("declarationDigest", str(&value.declaration_digest)),
    ])
}

/// Mod identity for session checkpoints.
#[derive(Debug, Clone, PartialEq)]
pub struct ModIdentity {
    /// Selection.
    pub selection: ModSelection,
    /// Source provider.
    pub source: ProviderRef,
    /// Declaration digest.
    pub declaration_digest: String,
    /// Modules.
    pub modules: Vec<ModuleIdentity>,
    /// Provider contracts (without payloads).
    pub providers: Vec<(String, String, i64)>,
}

/// Read a mod identity.
pub fn read_mod_identity(reader: SaveReader) -> Result<ModIdentity, PersistenceError> {
    Ok(ModIdentity {
        selection: read_selection(reader.field("selection"))?,
        source: read_provider_ref(reader.field("source"))?,
        declaration_digest: read_digest(reader.field("declarationDigest"))?,
        modules: reader
            .field("modules")
            .list(|value| read_module(value).map_err(PersistenceError::from))?,
        providers: reader
            .field("providers")
            .list(|value| -> Result<(String, String, i64), PersistenceError> {
                Ok((
                    namespaced(value.field("provider"))?,
                    namespaced(value.field("schema"))?,
                    value.field("version").integer(0)?,
                ))
            })?,
    })
}

/// Write a mod identity.
#[must_use]
pub fn write_mod_identity(identity: &ModIdentity) -> SaveJson {
    obj(vec![
        ("selection", write_selection(&identity.selection)),
        ("source", write_provider_ref(&identity.source)),
        ("declarationDigest", str(&identity.declaration_digest)),
        ("modules", arr(identity.modules.iter().map(write_module).collect())),
        (
            "providers",
            arr(identity
                .providers
                .iter()
                .map(|(provider, schema, version)| {
                    obj(vec![
                        ("provider", str(provider)),
                        ("schema", str(schema)),
                        ("version", int(*version)),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Mod private checkpoint: guests plus provider records.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCheckpoint {
    /// Identity.
    pub identity: ModIdentity,
    /// Guests.
    pub guests: Vec<GuestCheckpoint>,
    /// Providers.
    pub providers: Vec<ProviderCheckpoint>,
}

/// Mod session checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct ModSessionCheckpoint {
    /// Mods.
    pub mods: Vec<ModCheckpoint>,
}

/// Read a mod session checkpoint.
pub fn read_mod_session(reader: SaveReader) -> Result<ModSessionCheckpoint, PersistenceError> {
    reader.field("version").literal_i64(1)?;
    let mut identities = HashSet::new();
    let mods = reader
        .field("mods")
        .list(|entry| -> Result<ModCheckpoint, PersistenceError> {
            let identity = entry.field("identity");
            let state = entry.field("state");
            let key = mod_selection_key(&read_selection(identity.field("selection"))?)
                .map_err(|_| PersistenceError::from(identity.field("selection").fail("invalid mod selection")))?;
            if !identities.insert(key) {
                return Err(PersistenceError::from(
                    identity.field("selection").fail("duplicate saved mod selection"),
                ));
            }
            Ok(ModCheckpoint {
                identity: read_mod_identity(identity)?,
                guests: state
                    .field("guests")
                    .list(|value| read_guest(value).map_err(PersistenceError::from))?,
                providers: state
                    .field("providers")
                    .list(|value| read_provider_checkpoint(value).map_err(PersistenceError::from))?,
            })
        })?;
    Ok(ModSessionCheckpoint { mods })
}

/// Write a mod session checkpoint.
#[must_use]
pub fn write_mod_session(session: &ModSessionCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(1)),
        (
            "mods",
            arr(session
                .mods
                .iter()
                .map(|entry| {
                    obj(vec![
                        ("identity", write_mod_identity(&entry.identity)),
                        (
                            "state",
                            obj(vec![
                                ("guests", arr(entry.guests.iter().map(write_guest).collect())),
                                (
                                    "providers",
                                    arr(entry.providers.iter().map(write_provider_checkpoint).collect()),
                                ),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Compare mod identities by value.
#[must_use]
pub fn same_mod_identity(left: &ModIdentity, right: &ModIdentity) -> bool {
    mod_selection_key(&left.selection).ok() == mod_selection_key(&right.selection).ok()
        && left.source == right.source
        && left.declaration_digest == right.declaration_digest
        && left.modules == right.modules
        && left.providers == right.providers
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_guest::checkpoint::GameApi;
    use qa_guest::checkpoint::{GuestCheckpoint, GuestPrivateState};

    fn provider_ref() -> ProviderRef {
        ProviderRef {
            provider: "q2:game".to_string(),
            content: "q2:classic:base:1".to_string(),
        }
    }

    fn module() -> ModuleIdentity {
        ModuleIdentity {
            id: "q2:mod".to_string(),
            artifact_path: "mod.dll".to_string(),
            digest: format!("sha256:{}", "2".repeat(64)),
            revision: "1".to_string(),
        }
    }

    #[test]
    fn mods_round_trip() {
        let gameplay = ResolvedGameplayMod {
            selection: ModSelection {
                product: "rogue".to_string(),
                id: "tracker".to_string(),
            },
            source: provider_ref(),
            title: "Tracker".to_string(),
            source_title: "Rogue".to_string(),
            requires: Vec::new(),
            conflicts: Vec::new(),
            declaration_runtime: "qvm".to_string(),
            declaration: obj(vec![("runtime", str("qvm")), ("entry", int(1))]),
            declaration_digest: format!("sha256:{}", "3".repeat(64)),
        };
        let json = write_gameplay_mod(&gameplay);
        assert_eq!(read_gameplay_mod(SaveReader::new(&json)).unwrap(), gameplay);
        assert_eq!(mod_selection_key(&gameplay.selection).unwrap(), "rogue/tracker");
        assert!(read_mod_selection("rogue/tracker").is_ok());
        assert!(read_mod_selection("rogue").is_err());
        let session = ModSessionCheckpoint {
            mods: vec![ModCheckpoint {
                identity: ModIdentity {
                    selection: gameplay.selection.clone(),
                    source: provider_ref(),
                    declaration_digest: gameplay.declaration_digest.clone(),
                    modules: vec![module()],
                    providers: vec![("q2:mod".to_string(), "q2:mod-state".to_string(), 1)],
                },
                guests: vec![GuestCheckpoint::Typescript {
                    module: module(),
                    random: Vec::new(),
                    callbacks: Vec::new(),
                    api: GameApi::Q2RereleaseGame,
                    state: GuestPrivateState {
                        module: module(),
                        format: "q2:mod-state".to_string(),
                        bytes: vec![1],
                    },
                }],
                providers: vec![ProviderCheckpoint {
                    provider: "q2:mod".to_string(),
                    schema: "q2:mod-state".to_string(),
                    version: 1,
                    bytes: vec![2, 3],
                }],
            }],
        };
        let json = write_mod_session(&session);
        assert_eq!(read_mod_session(SaveReader::new(&json)).unwrap(), session);
    }

    #[test]
    fn duplicate_mod_selections_fail() {
        let identity = ModIdentity {
            selection: ModSelection {
                product: "rogue".to_string(),
                id: "tracker".to_string(),
            },
            source: provider_ref(),
            declaration_digest: format!("sha256:{}", "3".repeat(64)),
            modules: Vec::new(),
            providers: Vec::new(),
        };
        let entry = obj(vec![
            ("identity", write_mod_identity(&identity)),
            (
                "state",
                obj(vec![("guests", arr(Vec::new())), ("providers", arr(Vec::new()))]),
            ),
        ]);
        let session = obj(vec![("version", int(1)), ("mods", arr(vec![entry.clone(), entry]))]);
        assert!(read_mod_session(SaveReader::new(&session)).is_err());
    }
}
