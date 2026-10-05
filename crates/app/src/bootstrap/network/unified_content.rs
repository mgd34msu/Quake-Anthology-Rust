//! Unified composition identity and content resolution.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/network/unified-content.ts`
//! (`unifiedResourceId`, `createUnifiedComposition`, `readUnifiedComposition`,
//! `encodeUnifiedComposition`, `decodeUnifiedComposition`,
//! `resolveUnifiedComposition`, `loadUnifiedContent`, `resolveUnifiedResource`,
//! `buildUnifiedComposition`).
//!
//! Machine paths, mount generations, and resource ids never carry peer
//! authority: the composition normalizes every mount onto `unified/<index>`
//! paths and `unified:<n>` plan ids, then digests the canonical recipe plus
//! map sidecars. Recipes round-trip through the app
//! [`read_recipe`](crate::persistence::recipe) codec; catalog and mount-plan
//! IO reuse [`qa_content`]; canonical JSON and SHA-256 reuse
//! [`qa_net::common::session::canonical`] and [`qa_content::hash`].
//! [`qa_net::common::session::composition_identity`] is not reused because it
//! digests a fixed four-field composition while the donor digest also covers
//! sidecars. Async donor flow runs synchronously with order preserved.
//! Loaded-application handles
//! ([`LoadedApplicationContent`](super::super::content::LoadedApplicationContent))
//! are injected through [`UnifiedLoadedContent`] and [`UnifiedContentLoader`],
//! which carry the host-side handles; every check the
//! donor performs (precedence, byte identity, digest equality,
//! close-on-mismatch) runs here.

use std::collections::{BTreeMap, HashMap, HashSet};

use qa_content::catalog::{CatalogError, InstalledCatalog};
use qa_content::contract::{
    create_content_digest, ArchiveFormat, ContentDigest, ContentId, ContentMount as ContractMount, ContractError,
    MountPlanId, PrefixMountOrder as ContractPrefixOrder, ResolvedMountPlan as ContractMountPlan, ResourceId,
    ResourceIdentity, MAX_SAFE_INTEGER,
};
use qa_content::hash::sha256_hex;
use qa_content::mounts::{archive_digest_or_compute, open_mount_plan, MountError, OpenMountOptions, ResourceRef};
use qa_content::paths::normalize_resource_path;
use qa_net::common::session::{canonical, Json, SessionError};
use qa_world::save::shared::{read_content_id, read_digest};
use qa_world::save::value::{
    arr, decode_checkpoint_value, encode_checkpoint_value, int, obj, str as json_str, SaveJson, SaveReader,
};
use qa_world::WorldError;

use super::unified_frame_codec::UnifiedResourceKey;
use crate::persistence::recipe::{
    create_mount_id, create_mount_plan_id, map_module_artifact, module_artifact, mount_archive_details, mount_identity,
    mount_relocated, provenance_mount, read_mount, read_recipe, rebind_resource_reference, resolution_is_link,
    resolution_plan, resolution_rank, write_recipe, ContentMount, ExecutableRecipe, MountIdentity, PrefixMountOrder,
    ResolvedMountPlan, ResolvedResourceReference,
};
use crate::persistence::PersistenceError;

/// Unified composition failure.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedContentError {
    /// Checkpoint value failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Persistence failure.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Session failure.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// Composition failure.
    #[error("{0}")]
    Composition(String),
}

/// Unified map sidecar (donor `UnifiedMapSidecar`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedMapSidecar {
    /// Content identity.
    pub content: ContentId,
    /// Resource path.
    pub path: String,
    /// Resource key, when the sidecar resolved.
    pub resource: Option<UnifiedResourceKey>,
}

/// Unified composition identity (donor `UnifiedCompositionIdentity`).
///
/// The composition always carries schema version 1, the
/// `qts:snapshot-v10` snapshot schema, and an empty actor-configuration
/// list; only the normalized recipe and sidecars vary.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedCompositionIdentity {
    /// Identity digest.
    pub digest: ContentDigest,
    /// Normalized recipe.
    pub recipe: ExecutableRecipe,
    /// Map sidecars.
    pub sidecars: Vec<UnifiedMapSidecar>,
}

/// Snapshot schema carried by every unified composition.
pub const UNIFIED_SNAPSHOT_SCHEMA: &str = "qts:snapshot-v10";

fn unified_path(value: &str) -> Result<String, UnifiedContentError> {
    match normalize_resource_path(value) {
        Ok(normalized) if normalized == value => Ok(value.to_string()),
        _ => Err(UnifiedContentError::Composition(
            "Unified paths must use forward slashes".to_string(),
        )),
    }
}

/// Resource key for a resolved reference (donor `resourceKey`).
#[must_use]
pub fn resource_key(value: &ResolvedResourceReference) -> UnifiedResourceKey {
    UnifiedResourceKey {
        content: ContentId(mount_identity(provenance_mount(&value.provenance)).content.clone()),
        path: value.requested_path.clone(),
        identity: value.identity.clone(),
        byte_length: value.byte_length,
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for char in value.chars() {
        match char {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            other if (other as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", other as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Negotiated resource id for a unified key (donor `unifiedResourceId`).
pub fn unified_resource_id(key: &UnifiedResourceKey) -> Result<ResourceId, UnifiedContentError> {
    unified_path(&key.path)?;
    if key.byte_length > MAX_SAFE_INTEGER {
        return Err(UnifiedContentError::Composition(
            "Invalid unified resource length".to_string(),
        ));
    }
    let payload = format!(
        "[{},{},{},{}]",
        json_escape(key.content.as_str()),
        json_escape(&key.path),
        json_escape(&key.identity),
        key.byte_length
    );
    Ok(ResourceId(format!(
        "resource:unified:{}",
        sha256_hex(payload.as_bytes())
    )))
}

/// Negotiated resource id for a resolved reference.
pub fn unified_resource_id_for_reference(value: &ResolvedResourceReference) -> Result<ResourceId, UnifiedContentError> {
    unified_resource_id(&resource_key(value))
}

fn read_key(reader: SaveReader) -> Result<UnifiedResourceKey, UnifiedContentError> {
    let identity_field = reader.field("identity");
    let identity = ResourceIdentity::parse(&identity_field.string().map_err(UnifiedContentError::from)?)
        .map(|parsed| parsed.canonical())
        .ok_or_else(|| UnifiedContentError::Composition("expected a resource identity".to_string()))?;
    Ok(UnifiedResourceKey {
        content: ContentId(read_content_id(reader.field("content"))?),
        path: unified_path(&reader.field("path").string()?)?,
        identity,
        byte_length: reader
            .field("byteLength")
            .integer(0)
            .map_err(UnifiedContentError::from)
            .and_then(|length| {
                u64::try_from(length)
                    .map_err(|_| UnifiedContentError::Composition("Invalid unified resource length".to_string()))
            })?,
    })
}

#[allow(clippy::cast_possible_wrap)]
fn write_key(key: &UnifiedResourceKey) -> SaveJson {
    obj(vec![
        ("content", json_str(key.content.as_str())),
        ("path", json_str(&key.path)),
        ("identity", json_str(&key.identity)),
        ("byteLength", int(key.byte_length as i64)),
    ])
}

fn read_sidecars(reader: SaveReader) -> Result<Vec<UnifiedMapSidecar>, UnifiedContentError> {
    reader.list(|entry| {
        Ok::<_, UnifiedContentError>(UnifiedMapSidecar {
            content: ContentId(read_content_id(entry.field("content"))?),
            path: unified_path(&entry.field("path").string()?)?,
            resource: entry.field("resource").nullable(read_key)?,
        })
    })
}

fn write_sidecars(sidecars: &[UnifiedMapSidecar]) -> SaveJson {
    arr(sidecars
        .iter()
        .map(|entry| {
            obj(vec![
                ("content", json_str(entry.content.as_str())),
                ("path", json_str(&entry.path)),
                ("resource", entry.resource.as_ref().map_or(SaveJson::Null, write_key)),
            ])
        })
        .collect())
}

fn save_to_json(value: &SaveJson) -> Result<Json, UnifiedContentError> {
    match value {
        SaveJson::Null => Ok(Json::Null),
        SaveJson::Bool(value) => Ok(Json::Bool(*value)),
        SaveJson::Number(value) => Ok(Json::Number(*value)),
        #[allow(clippy::cast_precision_loss)]
        SaveJson::BigInt(value) => Ok(Json::Number(*value as f64)),
        SaveJson::Bytes(_) => Err(UnifiedContentError::Composition(
            "Composition identity contains a non-serializable value".to_string(),
        )),
        SaveJson::String(value) => Ok(Json::String(value.clone())),
        SaveJson::Array(values) => Ok(Json::Array(values.iter().map(save_to_json).collect::<Result<_, _>>()?)),
        SaveJson::Object(members) => Ok(Json::Object(
            members
                .iter()
                .map(|(key, member)| Ok((key.clone(), save_to_json(member)?)))
                .collect::<Result<_, UnifiedContentError>>()?,
        )),
    }
}

fn composition_digest(
    recipe: &ExecutableRecipe,
    sidecars: &[UnifiedMapSidecar],
) -> Result<ContentDigest, UnifiedContentError> {
    let mut fields = BTreeMap::new();
    fields.insert("schemaVersion".to_string(), Json::Number(1.0));
    fields.insert("recipe".to_string(), save_to_json(&write_recipe(recipe))?);
    fields.insert(
        "snapshotSchema".to_string(),
        Json::String(UNIFIED_SNAPSHOT_SCHEMA.to_string()),
    );
    fields.insert("actorConfigurations".to_string(), Json::Array(Vec::new()));
    fields.insert(
        "sidecars".to_string(),
        save_to_json(&write_sidecars(sidecars)).and_then(|value| match value {
            Json::Array(values) => Ok(Json::Array(values)),
            _ => Err(UnifiedContentError::Composition(
                "unified sidecars must encode as an array".to_string(),
            )),
        })?,
    );
    Ok(create_content_digest(&sha256_hex(
        canonical(&Json::Object(fields))?.as_bytes(),
    ))?)
}

/// Normalize a recipe onto unified mounts and digest it (donor `createUnifiedComposition`).
pub fn create_unified_composition(
    recipe: &ExecutableRecipe,
    sidecars: &[UnifiedMapSidecar],
) -> Result<UnifiedCompositionIdentity, UnifiedContentError> {
    let mut plans: HashMap<String, String> = HashMap::new();
    let mut plan_for = |id: &str| -> Result<String, UnifiedContentError> {
        if let Some(existing) = plans.get(id) {
            return Ok(existing.clone());
        }
        let logical = create_mount_plan_id("unified", &plans.len().to_string())?;
        plans.insert(id.to_string(), logical.clone());
        Ok(logical)
    };
    let plan_id = plan_for(&recipe.mounts.id)?;
    let mut mounts: HashMap<String, ContentMount> = HashMap::new();
    for (index, mount) in recipe.mounts.mounts.iter().enumerate() {
        if mounts.contains_key(&mount_identity(mount).id) {
            return Err(UnifiedContentError::Composition(
                "Repeated unified mount identity".to_string(),
            ));
        }
        let identity = MountIdentity {
            id: create_mount_id("unified", &index.to_string())?,
            content: mount_identity(mount).content.clone(),
            generation: 0,
        };
        let path = match mount_archive_details(mount) {
            Some((format, _, _)) => format!("unified/{index}.{format}"),
            None => format!("unified/{index}"),
        };
        let normalized = mount_relocated(mount, identity, path);
        mounts.insert(mount_identity(mount).id.clone(), normalized);
    }
    let mount_for = |id: &str| -> Result<ContentMount, UnifiedContentError> {
        mounts
            .get(id)
            .cloned()
            .ok_or_else(|| UnifiedContentError::Composition("Unified reference uses an undeclared mount".to_string()))
    };
    let order = |values: &[String]| -> Result<Vec<String>, UnifiedContentError> {
        let distinct: HashSet<&String> = values.iter().collect();
        if distinct.len() != values.len() || values.len() != mounts.len() {
            return Err(UnifiedContentError::Composition(
                "Unified mount order must include every mount once".to_string(),
            ));
        }
        values
            .iter()
            .map(|id| Ok(mount_identity(&mount_for(id)?).id.clone()))
            .collect()
    };
    let mut prefix_orders = Vec::with_capacity(recipe.mounts.prefix_orders.len());
    for value in &recipe.mounts.prefix_orders {
        let trimmed = value.prefix.strip_suffix('/').unwrap_or(&value.prefix);
        unified_path(trimmed)?;
        prefix_orders.push(PrefixMountOrder {
            prefix: value.prefix.clone(),
            mounts: order(&value.mounts)?,
        });
    }
    let ordered: Vec<ContentMount> = recipe
        .mounts
        .mounts
        .iter()
        .map(|mount| mount_for(&mount_identity(mount).id))
        .collect::<Result<_, _>>()?;
    let normalized_plan = ResolvedMountPlan {
        id: plan_id,
        mounts: ordered,
        default_order: order(&recipe.mounts.default_order)?,
        prefix_orders,
    };
    let mut rebind = |resource: &ResolvedResourceReference| -> Result<ResolvedResourceReference, UnifiedContentError> {
        let mount = mount_for(&mount_identity(provenance_mount(&resource.provenance)).id)?;
        let plan = plan_for(resolution_plan(&resource.resolution))?;
        Ok(rebind_resource_reference(resource, mount, plan)?)
    };
    let mut normalized = recipe.clone();
    normalized.mounts = normalized_plan;
    normalized.map.geometry = rebind(&recipe.map.geometry)?;
    normalized.resources = recipe.resources.iter().map(&mut rebind).collect::<Result<_, _>>()?;
    normalized.execution = recipe
        .execution
        .iter()
        .map(|module| map_module_artifact(module, &mut rebind))
        .collect::<Result<_, _>>()?;
    let checked = read_sidecars(SaveReader::new(&write_sidecars(sidecars)))?;
    let mut keys = HashSet::new();
    for entry in &checked {
        let key = format!("{}/{}", entry.content.as_str(), entry.path);
        if !keys.insert(key) {
            return Err(UnifiedContentError::Composition(
                "Repeated unified map sidecar".to_string(),
            ));
        }
    }
    let digest = composition_digest(&normalized, &checked)?;
    Ok(UnifiedCompositionIdentity {
        digest,
        recipe: normalized,
        sidecars: checked,
    })
}

/// Validate a composition value (donor `readUnifiedComposition`).
pub fn read_unified_composition(reader: SaveReader) -> Result<UnifiedCompositionIdentity, UnifiedContentError> {
    let composition = reader.field("composition");
    composition.field("schemaVersion").literal_i64(1)?;
    composition
        .field("snapshotSchema")
        .literal_str(UNIFIED_SNAPSHOT_SCHEMA)?;
    if !composition
        .field("actorConfigurations")
        .list(Ok::<_, UnifiedContentError>)?
        .is_empty()
    {
        return Err(UnifiedContentError::Composition(
            "actor configurations are negotiated by frame".to_string(),
        ));
    }
    let recipe = read_recipe(composition.field("recipe"), &|_| None)?;
    let sidecars = read_sidecars(composition.field("sidecars"))?;
    let identity = create_unified_composition(&recipe, &sidecars)?;
    if composition_digest(&recipe, &sidecars)? != identity.digest {
        return Err(UnifiedContentError::Composition(
            "noncanonical or mismatched unified composition".to_string(),
        ));
    }
    let digest = read_digest(reader.field("digest"))?;
    if identity.digest.as_str() != digest {
        return Err(UnifiedContentError::Composition(
            "noncanonical or mismatched unified composition".to_string(),
        ));
    }
    Ok(identity)
}

/// Write a composition identity value.
#[must_use]
pub fn write_unified_composition(identity: &UnifiedCompositionIdentity) -> SaveJson {
    obj(vec![
        ("digest", json_str(identity.digest.as_str())),
        (
            "composition",
            obj(vec![
                ("schemaVersion", int(1)),
                ("recipe", write_recipe(&identity.recipe)),
                ("snapshotSchema", json_str(UNIFIED_SNAPSHOT_SCHEMA)),
                ("actorConfigurations", arr(Vec::new())),
                ("sidecars", write_sidecars(&identity.sidecars)),
            ]),
        ),
    ])
}

/// Encode a composition identity (donor `encodeUnifiedComposition`).
#[must_use]
pub fn encode_unified_composition(identity: &UnifiedCompositionIdentity) -> Vec<u8> {
    encode_checkpoint_value(&write_unified_composition(identity))
}

/// Decode a composition identity (donor `decodeUnifiedComposition`).
pub fn decode_unified_composition(bytes: &[u8]) -> Result<UnifiedCompositionIdentity, UnifiedContentError> {
    let value = decode_checkpoint_value(bytes)?;
    read_unified_composition(SaveReader::new(&value))
}

fn contract_archive_format_name(format: &ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Pak => "pak",
        ArchiveFormat::Pk3 => "pk3",
        ArchiveFormat::Kpf => "kpf",
        ArchiveFormat::Zip => "zip",
    }
}

/// Whether a catalog mount can serve an offered mount.
fn candidate_matches(offered: &ContentMount, candidate: &ContractMount) -> Result<bool, UnifiedContentError> {
    if mount_identity(offered).content != candidate.identity().content.as_str() {
        return Ok(false);
    }
    match (mount_archive_details(offered), candidate) {
        (Some((format, _, digest)), ContractMount::Archive(found)) => Ok(contract_archive_format_name(&found.format)
            == format
            && archive_digest_or_compute(found)?.as_str() == digest),
        (None, ContractMount::Loose(_)) => Ok(true),
        _ => Ok(false),
    }
}

/// Local mount in both recipe and catalog form.
#[derive(Debug, Clone)]
struct LocalMount {
    /// Recipe-side mount for rebinding.
    app: ContentMount,
    /// Catalog-side mount for plan IO.
    contract: ContractMount,
}

#[allow(clippy::cast_possible_wrap)]
fn contract_mount_to_app(mount: &ContractMount) -> Result<ContentMount, UnifiedContentError> {
    let identity = mount.identity();
    let mut members = vec![(
        "identity",
        obj(vec![
            ("id", json_str(identity.id.as_str())),
            ("content", json_str(identity.content.as_str())),
            ("generation", int(identity.generation as i64)),
        ]),
    )];
    match mount {
        ContractMount::Archive(found) => {
            members.push(("kind", json_str("archive")));
            members.push(("format", json_str(contract_archive_format_name(&found.format))));
            members.push(("archivePath", json_str(&found.archive_path)));
            members.push(("archiveDigest", json_str(archive_digest_or_compute(found)?.as_str())));
        }
        ContractMount::Loose(found) => {
            members.push(("kind", json_str("loose")));
            members.push(("rootPath", json_str(&found.root_path)));
        }
    }
    let json = obj(members);
    Ok(read_mount(SaveReader::at(&json, "unified.mount"))?)
}

/// Rebuild a local recipe from installed catalog mounts (donor `resolveUnifiedComposition`).
pub fn resolve_unified_composition(
    offered: &UnifiedCompositionIdentity,
    catalog: &InstalledCatalog,
) -> Result<ExecutableRecipe, UnifiedContentError> {
    let recomputed = create_unified_composition(&offered.recipe, &offered.sidecars)?;
    if recomputed.digest != offered.digest || composition_digest(&offered.recipe, &offered.sidecars)? != offered.digest
    {
        return Err(UnifiedContentError::Composition(
            "noncanonical or mismatched unified composition".to_string(),
        ));
    }
    let recipe = &offered.recipe;
    let mut available: HashMap<String, Vec<ContractMount>> = HashMap::new();
    for mount in &recipe.mounts.mounts {
        let content = mount_identity(mount).content.clone();
        if !available.contains_key(&content) {
            available.insert(content.clone(), catalog.mounts_for(&content)?);
        }
    }
    let mut mounts: HashMap<String, LocalMount> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for mount in &recipe.mounts.mounts {
        let identity = mount_identity(mount);
        let empty = Vec::new();
        let candidates = available.get(&identity.content).unwrap_or(&empty);
        let mut local = None;
        for candidate in candidates {
            if used.contains(candidate.identity().id.as_str()) {
                continue;
            }
            if candidate_matches(mount, candidate)? {
                local = Some(candidate);
                break;
            }
        }
        let Some(local) = local else {
            return Err(UnifiedContentError::Composition(format!(
                "Installed content lacks unified mount {}/{}",
                identity.content, identity.id
            )));
        };
        used.insert(local.identity().id.as_str().to_string());
        mounts.insert(
            identity.id.clone(),
            LocalMount {
                app: contract_mount_to_app(local)?,
                contract: local.clone(),
            },
        );
    }
    let mount_for = |id: &str| -> Result<LocalMount, UnifiedContentError> {
        mounts
            .get(id)
            .cloned()
            .ok_or_else(|| UnifiedContentError::Composition("Unknown unified mount".to_string()))
    };
    let ordered_app: Vec<ContentMount> = recipe
        .mounts
        .mounts
        .iter()
        .map(|mount| Ok(mount_for(&mount_identity(mount).id)?.app))
        .collect::<Result<_, UnifiedContentError>>()?;
    let ordered_contract: Vec<ContractMount> = recipe
        .mounts
        .mounts
        .iter()
        .map(|mount| Ok(mount_for(&mount_identity(mount).id)?.contract))
        .collect::<Result<_, UnifiedContentError>>()?;
    let map_order = |values: &[String]| -> Result<Vec<String>, UnifiedContentError> {
        values
            .iter()
            .map(|id| Ok(mount_identity(&mount_for(id)?.app).id.clone()))
            .collect()
    };
    let plan = ResolvedMountPlan {
        id: recipe.mounts.id.clone(),
        mounts: ordered_app,
        default_order: map_order(&recipe.mounts.default_order)?,
        prefix_orders: recipe
            .mounts
            .prefix_orders
            .iter()
            .map(|value| {
                Ok(PrefixMountOrder {
                    prefix: value.prefix.clone(),
                    mounts: map_order(&value.mounts)?,
                })
            })
            .collect::<Result<_, UnifiedContentError>>()?,
    };
    let map_contract_order = |values: &[String]| -> Result<Vec<qa_content::contract::MountId>, UnifiedContentError> {
        values
            .iter()
            .map(|id| Ok(mount_for(id)?.contract.identity().id.clone()))
            .collect()
    };
    let contract_plan = ContractMountPlan {
        id: MountPlanId(plan.id.clone()),
        mounts: ordered_contract,
        default_order: map_contract_order(&recipe.mounts.default_order)?,
        prefix_orders: recipe
            .mounts
            .prefix_orders
            .iter()
            .map(|value| {
                Ok(ContractPrefixOrder {
                    prefix: value.prefix.clone(),
                    mounts: map_contract_order(&value.mounts)?,
                })
            })
            .collect::<Result<_, UnifiedContentError>>()?,
    };
    let mut rebind = |resource: &ResolvedResourceReference| -> Result<ResolvedResourceReference, UnifiedContentError> {
        let mount = mount_for(&mount_identity(provenance_mount(&resource.provenance)).id)?.app;
        Ok(rebind_resource_reference(
            resource,
            mount,
            resolution_plan(&resource.resolution).to_string(),
        )?)
    };
    let mut local = recipe.clone();
    local.mounts = plan.clone();
    local.map.geometry = rebind(&recipe.map.geometry)?;
    local.resources = recipe.resources.iter().map(&mut rebind).collect::<Result<_, _>>()?;
    local.execution = recipe
        .execution
        .iter()
        .map(|module| map_module_artifact(module, &mut rebind))
        .collect::<Result<_, _>>()?;
    let opened = open_mount_plan(&contract_plan, OpenMountOptions::default())?;
    let mut references = vec![&local.map.geometry];
    references.extend(local.resources.iter());
    for module in &local.execution {
        if let Some(artifact) = module_artifact(module) {
            references.push(artifact);
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut unique: Vec<&ResolvedResourceReference> = Vec::new();
    for reference in references {
        if seen.insert(reference.id.as_str()) {
            unique.push(reference);
        }
    }
    for resource in unique {
        opened.read(ResourceRef::Path(&resource.requested_path))?;
        if resolution_plan(&resource.resolution) == plan.id.as_str() {
            let resolved = opened.resolve(&resource.requested_path)?;
            if resolved.as_ref().map(|value| value.id.as_str()) != Some(resource.id.as_str()) {
                return Err(UnifiedContentError::Composition(format!(
                    "Unified resource precedence differs: {}",
                    resource.requested_path
                )));
            }
        } else if !resolution_is_link(&resource.resolution)
            && resolution_rank(&resource.resolution) >= plan.mounts.len() as u64
        {
            return Err(UnifiedContentError::Composition(
                "Unified resource precedence is outside its mount plan".to_string(),
            ));
        }
    }
    if create_unified_composition(&local, &offered.sidecars)?.digest != offered.digest {
        return Err(UnifiedContentError::Composition(
            "Local unified recipe differs from admitted composition".to_string(),
        ));
    }
    Ok(local)
}

/// Loaded-application surface the composition loader needs.
///
/// Mirrors the donor `LoadedApplicationContent` calls used by
/// `buildUnifiedComposition`, `loadUnifiedContent`, and
/// `resolveUnifiedResource`.
pub trait UnifiedLoadedContent {
    /// Recipe the content was loaded from.
    fn recipe(&self) -> &ExecutableRecipe;
    /// Observed map sidecars.
    fn map_sidecars(&self) -> Vec<UnifiedLoadedSidecar>;
    /// Open a resource by content and path.
    fn open_resource(
        &self,
        content: &ContentId,
        path: &str,
    ) -> Result<Option<ResolvedResourceReference>, UnifiedContentError>;
    /// Close the loaded content.
    fn close(self)
    where
        Self: Sized;
}

/// Observed map sidecar.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedLoadedSidecar {
    /// Content identity.
    pub content: ContentId,
    /// Resource path.
    pub path: String,
    /// Resolved reference, when the sidecar resolved.
    pub resource: Option<ResolvedResourceReference>,
}

/// Build a composition from loaded content (donor `buildUnifiedComposition`).
pub fn build_unified_composition(
    content: &dyn UnifiedLoadedContent,
) -> Result<UnifiedCompositionIdentity, UnifiedContentError> {
    let sidecars = content
        .map_sidecars()
        .into_iter()
        .map(|value| UnifiedMapSidecar {
            content: value.content,
            path: value.path,
            resource: value.resource.as_ref().map(resource_key),
        })
        .collect::<Vec<_>>();
    create_unified_composition(content.recipe(), &sidecars)
}

/// Application loader for unified content (donor `loadApplicationContent` surface).
pub trait UnifiedContentLoader {
    /// Loaded content handle.
    type Content: UnifiedLoadedContent;
    /// Product family and edition for map entities content.
    fn product_family(&self, content: &ContentId) -> Result<(String, String), UnifiedContentError>;
    /// Load content for a recipe and family.
    fn load(&self, recipe: &ExecutableRecipe, family: &str) -> Result<Self::Content, UnifiedContentError>;
}

fn unified_family<'a>(family: &'a str, edition: &str) -> &'a str {
    if family == "q1" && edition == "quakeworld" {
        "qw"
    } else {
        family
    }
}

/// Load and verify unified content (donor `loadUnifiedContent`).
pub fn load_unified_content<Loader: UnifiedContentLoader>(
    loader: &Loader,
    identity: &UnifiedCompositionIdentity,
    catalog: &InstalledCatalog,
) -> Result<Loader::Content, UnifiedContentError> {
    let recipe = resolve_unified_composition(identity, catalog)?;
    let entities = recipe.map.entities.content.clone();
    let (family, edition) = loader.product_family(&ContentId(entities))?;
    let family = unified_family(&family, &edition);
    let loaded = loader.load(&recipe, family)?;
    if build_unified_composition(&loaded)?.digest != identity.digest {
        loaded.close();
        return Err(UnifiedContentError::Composition(
            "Unified map sidecars differ from the authoritative world".to_string(),
        ));
    }
    Ok(loaded)
}

/// Resolve and verify one unified resource (donor `resolveUnifiedResource`).
pub fn resolve_unified_resource(
    content: &dyn UnifiedLoadedContent,
    key: &UnifiedResourceKey,
) -> Result<ResolvedResourceReference, UnifiedContentError> {
    unified_path(&key.path)?;
    let opened = content.open_resource(&key.content, &key.path)?;
    match opened {
        Some(reference)
            if mount_identity(provenance_mount(&reference.provenance)).content == key.content.as_str()
                && reference.identity == key.identity
                && reference.byte_length == key.byte_length =>
        {
            Ok(reference)
        }
        _ => Err(UnifiedContentError::Composition(format!(
            "Unified resource is absent or differs: {}/{}",
            key.content.as_str(),
            key.path
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::recipe::fixture_recipe;
    use qa_content::contract::{
        ArchiveMount as ContractArchiveMount, LooseMount as ContractLooseMount, MountId,
        MountIdentity as ContractMountIdentity,
    };

    fn empty_catalog() -> InstalledCatalog {
        InstalledCatalog::new("corpus".to_string(), Vec::new(), Vec::new(), 0, None).unwrap()
    }

    #[test]
    fn resource_id_rejects_relative_escape() {
        let key = UnifiedResourceKey {
            content: ContentId("q1:classic:base:1".to_string()),
            path: "../escape.bsp".to_string(),
            identity: "identity:0:0:8:0".to_string(),
            byte_length: 8,
        };
        assert!(unified_resource_id(&key).is_err());
    }

    #[test]
    fn resource_id_is_stable() {
        let key = UnifiedResourceKey {
            content: ContentId("q1:classic:base:1".to_string()),
            path: "maps/e1m1.bsp".to_string(),
            identity: "identity:0:0:8:0".to_string(),
            byte_length: 8,
        };
        let first = unified_resource_id(&key).unwrap();
        let second = unified_resource_id(&key).unwrap();
        assert_eq!(first, second);
        assert!(first.as_str().starts_with("resource:unified:"));
        assert_eq!(first.as_str().len(), "resource:unified:".len() + 64);
    }

    #[test]
    fn composition_round_trips_through_identity() {
        let recipe = fixture_recipe();
        let identity = create_unified_composition(&recipe, &[]).unwrap();
        assert_eq!(identity.recipe.mounts.id, "mount-plan:unified:0");
        assert_eq!(mount_identity(&identity.recipe.mounts.mounts[0]).id, "mount:unified:0");
        let encoded = encode_unified_composition(&identity);
        let decoded = decode_unified_composition(&encoded).unwrap();
        assert_eq!(decoded.digest, identity.digest);
        assert_eq!(decoded.recipe, identity.recipe);
    }

    fn field_mut<'a>(value: &'a mut SaveJson, name: &str) -> &'a mut SaveJson {
        let SaveJson::Object(members) = value else {
            panic!("expected an object");
        };
        members
            .iter_mut()
            .find(|(key, _)| key == name)
            .map(|(_, member)| member)
            .expect("expected a field")
    }

    #[test]
    fn read_rejects_tampered_digest() {
        let identity = create_unified_composition(&fixture_recipe(), &[]).unwrap();
        let mut value = write_unified_composition(&identity);
        let digest = field_mut(&mut value, "digest");
        let SaveJson::String(text) = digest else {
            panic!("expected a digest string");
        };
        text.push('0');
        assert!(read_unified_composition(SaveReader::new(&value)).is_err());
    }

    #[test]
    fn read_rejects_machine_paths() {
        let identity = create_unified_composition(&fixture_recipe(), &[]).unwrap();
        let mut value = write_unified_composition(&identity);
        let composition = field_mut(&mut value, "composition");
        let recipe = field_mut(composition, "recipe");
        let mounts = field_mut(recipe, "mounts");
        let list = field_mut(mounts, "mounts");
        let SaveJson::Array(entries) = list else {
            panic!("expected a mount list");
        };
        let root = field_mut(&mut entries[0], "rootPath");
        *root = json_str("/machine/absolute");
        assert!(read_unified_composition(SaveReader::new(&value)).is_err());
    }

    #[test]
    fn resolve_fails_without_installed_mounts() {
        let identity = create_unified_composition(&fixture_recipe(), &[]).unwrap();
        assert!(resolve_unified_composition(&identity, &empty_catalog()).is_err());
    }

    #[derive(Debug, Clone)]
    struct FakeContent {
        recipe: ExecutableRecipe,
        sidecars: Vec<UnifiedLoadedSidecar>,
        resource: Option<ResolvedResourceReference>,
    }

    impl UnifiedLoadedContent for FakeContent {
        fn recipe(&self) -> &ExecutableRecipe {
            &self.recipe
        }

        fn map_sidecars(&self) -> Vec<UnifiedLoadedSidecar> {
            self.sidecars.clone()
        }

        fn open_resource(
            &self,
            _content: &ContentId,
            _path: &str,
        ) -> Result<Option<ResolvedResourceReference>, UnifiedContentError> {
            Ok(self.resource.clone())
        }

        fn close(self) {}
    }

    #[test]
    fn build_matches_create_for_observed_sidecars() {
        let recipe = fixture_recipe();
        let expected = create_unified_composition(&recipe, &[]).unwrap();
        let content = FakeContent {
            recipe,
            sidecars: Vec::new(),
            resource: None,
        };
        assert_eq!(build_unified_composition(&content).unwrap().digest, expected.digest);
    }

    #[test]
    fn resolve_resource_checks_byte_identity() {
        let recipe = fixture_recipe();
        let reference = recipe.map.geometry.clone();
        let expected = reference.id.clone();
        let key = resource_key(&reference);
        let content = FakeContent {
            recipe,
            sidecars: Vec::new(),
            resource: Some(reference),
        };
        assert_eq!(resolve_unified_resource(&content, &key).unwrap().id, expected);
        let absent = FakeContent {
            recipe: fixture_recipe(),
            sidecars: Vec::new(),
            resource: None,
        };
        assert!(resolve_unified_resource(&absent, &key).is_err());
        let mut tampered = key.clone();
        tampered.byte_length += 1;
        assert!(resolve_unified_resource(&content, &tampered).is_err());
    }

    struct FakeLoader;

    impl UnifiedContentLoader for FakeLoader {
        type Content = FakeContent;

        fn product_family(&self, _content: &ContentId) -> Result<(String, String), UnifiedContentError> {
            Ok(("q1".to_string(), "quakeworld".to_string()))
        }

        fn load(&self, recipe: &ExecutableRecipe, _family: &str) -> Result<FakeContent, UnifiedContentError> {
            Ok(FakeContent {
                recipe: recipe.clone(),
                sidecars: Vec::new(),
                resource: None,
            })
        }
    }

    #[test]
    fn load_propagates_resolve_failure() {
        let identity = create_unified_composition(&fixture_recipe(), &[]).unwrap();
        assert!(load_unified_content(&FakeLoader, &identity, &empty_catalog()).is_err());
    }

    #[test]
    fn family_mapping_prefers_qw() {
        assert_eq!(unified_family("q1", "quakeworld"), "qw");
        assert_eq!(unified_family("q1", "classic"), "q1");
        assert_eq!(unified_family("q3", "classic"), "q3");
    }

    fn contract_loose(id: &str, content: &str, root: &str) -> ContractMount {
        ContractMount::Loose(ContractLooseMount {
            identity: ContractMountIdentity {
                id: MountId(id.to_string()),
                content: ContentId(content.to_string()),
                generation: 0,
            },
            root_path: root.to_string(),
        })
    }

    #[test]
    fn candidate_matching_compares_kind_and_bytes() {
        let recipe = fixture_recipe();
        let offered = &recipe.mounts.mounts[0];
        let identity = mount_identity(offered);
        let mirror = contract_loose("mount:local:0", &identity.content, "/local/base");
        assert!(candidate_matches(offered, &mirror).unwrap());
        assert_eq!(
            mount_identity(&contract_mount_to_app(&mirror).unwrap()).content,
            identity.content
        );
        let foreign = contract_loose("mount:local:0", "q2:classic:base:1", "/local/base");
        assert!(!candidate_matches(offered, &foreign).unwrap());
        let archive = ContractMount::Archive(ContractArchiveMount {
            identity: ContractMountIdentity {
                id: MountId("mount:local:0".to_string()),
                content: ContentId(identity.content.clone()),
                generation: 0,
            },
            format: ArchiveFormat::Pak,
            archive_path: "baseq3/pak0.pak".to_string(),
            archive_digest: qa_content::contract::LazyArchiveDigest::computed(ContentDigest(format!(
                "sha256:{}",
                "ab".repeat(32)
            ))),
        });
        assert_eq!(
            mount_archive_details(&contract_mount_to_app(&archive).unwrap())
                .unwrap()
                .0,
            "pak"
        );
        assert!(!candidate_matches(offered, &archive).unwrap());
    }
}
