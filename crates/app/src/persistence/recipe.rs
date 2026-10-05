//! Executable recipes ported from `src/persistence/recipe.ts`.
//!
//! Mount/provider/resource identities, execution modules, equipment,
//! weapon behaviors, and the full [`ExecutableRecipe`] envelope. The
//! resource identity hash ([`create_resource_id`]) replicates the donor's
//! `encodeURIComponent` composition exactly; only error reporting is
//! app-local.

use std::collections::HashSet;

use qa_content::contract::ResourceIdentity;
use qa_core::time::ClockProfile;
use qa_guest::checkpoint::{
    read_api, read_module, read_native_abi, write_api, write_module, write_native_call_abi, GameApi, NativeCallAbi,
};
use qa_world::save::shared::{
    identity_parts, read_character, read_clock, read_content_id, read_digest, read_numeric, read_ordering,
    valid_identity_part, validate_content_id, write_character, write_clock, write_numeric, write_ordering,
    CharacterSelection, ProviderRef, SavedNumericProfile,
};
use qa_world::save::value::{arr, int, namespaced, obj, str, SaveJson, SaveReader};
use qa_world::scheduler::FrameOrdering;

use super::mods::{read_gameplay_mod, write_gameplay_mod, ResolvedGameplayMod};
use super::native_weapon::{
    read_saved_native_weapon_declaration, read_weapon_behavior_callback, write_native_weapon_declaration,
    write_weapon_behavior_callback, NativeWeaponBehaviorDeclaration, WeaponBehaviorCallback, WeaponBehaviorDefinition,
};
use super::qvm_grapple::{read_qvm_grapple_definition, write_qvm_grapple_definition, QvmGrappleDefinition};
use super::PersistenceError;

/// Percent-encode per JavaScript `encodeURIComponent`.
#[must_use]
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')')
        {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Validate a relative resource path.
pub fn resource_path(value: &str) -> Result<String, PersistenceError> {
    if value.is_empty()
        || value.contains('\0')
        || value.starts_with('/')
        || value.contains('\\')
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(PersistenceError::BadSave(format!(
            "Expected a relative resource path: {value}"
        )));
    }
    Ok(value.to_string())
}

/// Read a recipe id.
pub fn read_recipe_id(reader: SaveReader) -> Result<String, PersistenceError> {
    let (name, revision) = identity_parts(reader.clone(), "recipe")?;
    Ok(format!("recipe:{name}:{revision}"))
}

/// Read a mount id.
pub fn read_mount_id(reader: SaveReader) -> Result<String, PersistenceError> {
    let (namespace, name) = identity_parts(reader.clone(), "mount")?;
    Ok(format!("mount:{namespace}:{name}"))
}

/// Read a mount-plan id.
pub fn read_mount_plan_id(reader: SaveReader) -> Result<String, PersistenceError> {
    let (name, revision) = identity_parts(reader.clone(), "mount-plan")?;
    Ok(format!("mount-plan:{name}:{revision}"))
}

/// Mount identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountIdentity {
    /// Mount id.
    pub id: String,
    /// Content id.
    pub content: String,
    /// Generation.
    pub generation: u64,
}

fn read_mount_identity(reader: SaveReader) -> Result<MountIdentity, PersistenceError> {
    let generation = reader.field("generation").integer(0)?;
    Ok(MountIdentity {
        id: read_mount_id(reader.field("id"))?,
        content: read_content_id(reader.field("content"))?,
        generation: u64::try_from(generation)
            .map_err(|_| PersistenceError::from(reader.field("generation").fail("expected an integer in range")))?,
    })
}

fn write_mount_identity(identity: &MountIdentity) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    obj(vec![
        ("id", str(&identity.id)),
        ("content", str(&identity.content)),
        ("generation", int(identity.generation as i64)),
    ])
}

/// Content mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentMount {
    /// Archive mount.
    Archive {
        /// Identity.
        identity: MountIdentity,
        /// Format.
        format: String,
        /// Archive path.
        archive_path: String,
        /// Archive digest.
        archive_digest: String,
    },
    /// Loose mount.
    Loose {
        /// Identity.
        identity: MountIdentity,
        /// Root path.
        root_path: String,
    },
}

/// Read a content mount.
pub fn read_mount(reader: SaveReader) -> Result<ContentMount, PersistenceError> {
    let identity = read_mount_identity(reader.field("identity"))?;
    if reader.field("kind").choice_str(&["archive", "loose"])? == "loose" {
        Ok(ContentMount::Loose {
            identity,
            root_path: reader.field("rootPath").string()?,
        })
    } else {
        Ok(ContentMount::Archive {
            identity,
            format: reader.field("format").choice_str(&["pak", "pk3", "kpf", "zip"])?,
            archive_path: reader.field("archivePath").string()?,
            archive_digest: read_digest(reader.field("archiveDigest"))?,
        })
    }
}

/// Write a content mount.
#[must_use]
pub fn write_mount(mount: &ContentMount) -> SaveJson {
    match mount {
        ContentMount::Loose { identity, root_path } => obj(vec![
            ("identity", write_mount_identity(identity)),
            ("kind", str("loose")),
            ("rootPath", str(root_path)),
        ]),
        ContentMount::Archive {
            identity,
            format,
            archive_path,
            archive_digest,
        } => obj(vec![
            ("identity", write_mount_identity(identity)),
            ("kind", str("archive")),
            ("format", str(format)),
            ("archivePath", str(archive_path)),
            ("archiveDigest", str(archive_digest)),
        ]),
    }
}

/// Resource provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceProvenance {
    /// Archive member.
    Archive {
        /// Mount.
        mount: Box<ContentMount>,
        /// Member path.
        member_path: String,
        /// Member index.
        member_index: u64,
    },
    /// Loose member.
    Loose {
        /// Mount.
        mount: Box<ContentMount>,
        /// Member path.
        member_path: String,
    },
}

fn read_provenance(reader: SaveReader) -> Result<ResourceProvenance, PersistenceError> {
    let member_path = reader.field("memberPath").string()?;
    if reader.field("kind").choice_str(&["archive", "loose"])? == "archive" {
        let mount = read_mount(reader.field("mount"))?;
        if !matches!(mount, ContentMount::Archive { .. }) {
            return Err(PersistenceError::from(reader.fail("expected an archive mount")));
        }
        let member_index = reader.field("memberIndex").integer(0)?;
        Ok(ResourceProvenance::Archive {
            mount: Box::new(mount),
            member_path,
            member_index: u64::try_from(member_index).map_err(|_| {
                PersistenceError::from(reader.field("memberIndex").fail("expected an integer in range"))
            })?,
        })
    } else {
        let mount = read_mount(reader.field("mount"))?;
        if !matches!(mount, ContentMount::Loose { .. }) {
            return Err(PersistenceError::from(reader.fail("expected a loose mount")));
        }
        Ok(ResourceProvenance::Loose {
            mount: Box::new(mount),
            member_path,
        })
    }
}

fn write_provenance(provenance: &ResourceProvenance) -> SaveJson {
    match provenance {
        ResourceProvenance::Archive {
            mount,
            member_path,
            member_index,
        } => obj(vec![
            ("kind", str("archive")),
            ("memberPath", str(member_path)),
            ("mount", write_mount(mount)),
            #[allow(clippy::cast_possible_wrap)]
            ("memberIndex", int(*member_index as i64)),
        ]),
        ResourceProvenance::Loose { mount, member_path } => obj(vec![
            ("kind", str("loose")),
            ("memberPath", str(member_path)),
            ("mount", write_mount(mount)),
        ]),
    }
}

/// Resource resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceResolution {
    /// Default order.
    DefaultOrder {
        /// Plan.
        plan: String,
        /// Rank.
        rank: u64,
    },
    /// Prefix order.
    PrefixOrder {
        /// Plan.
        plan: String,
        /// Rank.
        rank: u64,
        /// Prefix.
        prefix: String,
    },
    /// Link.
    Link {
        /// Plan.
        plan: String,
        /// Source prefix.
        source_prefix: String,
        /// Target path.
        target_path: String,
    },
}

fn read_resolution(reader: SaveReader) -> Result<ResourceResolution, PersistenceError> {
    let plan = read_mount_plan_id(reader.field("plan"))?;
    match reader
        .field("kind")
        .choice_str(&["default-order", "prefix-order", "link"])?
        .as_str()
    {
        "default-order" => {
            let rank = reader.field("rank").integer(0)?;
            Ok(ResourceResolution::DefaultOrder {
                plan,
                rank: u64::try_from(rank)
                    .map_err(|_| PersistenceError::from(reader.field("rank").fail("expected an integer in range")))?,
            })
        }
        "prefix-order" => {
            let rank = reader.field("rank").integer(0)?;
            Ok(ResourceResolution::PrefixOrder {
                plan,
                rank: u64::try_from(rank)
                    .map_err(|_| PersistenceError::from(reader.field("rank").fail("expected an integer in range")))?,
                prefix: reader.field("prefix").string()?,
            })
        }
        _ => Ok(ResourceResolution::Link {
            plan,
            source_prefix: reader.field("sourcePrefix").string()?,
            target_path: reader.field("targetPath").string()?,
        }),
    }
}

fn write_resolution(resolution: &ResourceResolution) -> SaveJson {
    match resolution {
        #[allow(clippy::cast_possible_wrap)]
        ResourceResolution::DefaultOrder { plan, rank } => obj(vec![
            ("kind", str("default-order")),
            ("plan", str(plan)),
            ("rank", int(*rank as i64)),
        ]),
        #[allow(clippy::cast_possible_wrap)]
        ResourceResolution::PrefixOrder { plan, rank, prefix } => obj(vec![
            ("kind", str("prefix-order")),
            ("plan", str(plan)),
            ("rank", int(*rank as i64)),
            ("prefix", str(prefix)),
        ]),
        ResourceResolution::Link {
            plan,
            source_prefix,
            target_path,
        } => obj(vec![
            ("kind", str("link")),
            ("plan", str(plan)),
            ("sourcePrefix", str(source_prefix)),
            ("targetPath", str(target_path)),
        ]),
    }
}

/// Resolved resource reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResourceReference {
    /// Resource id.
    pub id: String,
    /// Requested path.
    pub requested_path: String,
    /// Provenance.
    pub provenance: ResourceProvenance,
    /// Canonical value identity (`identity:...`, never a digest).
    pub identity: String,
    /// Byte length.
    pub byte_length: u64,
    /// Resolution.
    pub resolution: ResourceResolution,
}

fn resolution_key(resolution: &ResourceResolution) -> String {
    match resolution {
        ResourceResolution::DefaultOrder { plan, rank } => format!("{plan}:default:{rank}"),
        ResourceResolution::PrefixOrder { plan, rank, prefix } => {
            format!("{plan}:prefix:{}:{rank}", encode_uri_component(prefix))
        }
        ResourceResolution::Link {
            plan,
            source_prefix,
            target_path,
        } => format!(
            "{plan}:link:{}:{}",
            encode_uri_component(source_prefix),
            encode_uri_component(target_path)
        ),
    }
}

/// Compute a resource id from its provenance (donor `createResourceId`).
///
/// `byte_length` is validated (nonnegative) but is not part of the id.
pub fn create_resource_id(
    requested_path: &str,
    provenance: &ResourceProvenance,
    resource_identity: &str,
    _byte_length: u64,
    resolution: &ResourceResolution,
) -> Result<String, PersistenceError> {
    resource_path(requested_path)?;
    let (identity, member_path, source) = match provenance {
        ResourceProvenance::Archive {
            mount,
            member_path,
            member_index,
        } => {
            let ContentMount::Archive {
                identity,
                archive_path,
                archive_digest,
                ..
            } = mount.as_ref()
            else {
                return Err(PersistenceError::BadSave("expected an archive mount".to_string()));
            };
            (
                identity,
                member_path,
                format!("{}:{archive_digest}:{member_index}", encode_uri_component(archive_path)),
            )
        }
        ResourceProvenance::Loose { mount, member_path } => {
            let ContentMount::Loose { identity, root_path } = mount.as_ref() else {
                return Err(PersistenceError::BadSave("expected a loose mount".to_string()));
            };
            (identity, member_path, encode_uri_component(root_path))
        }
    };
    resource_path(member_path)?;
    validate_content_id(&identity.content)
        .map_err(|_| PersistenceError::BadSave("expected a content identity".to_string()))?;
    Ok(format!(
        "resource:{}:{}:{}:{source}:{}:{resource_identity}:{}",
        identity.content,
        identity.id,
        identity.generation,
        encode_uri_component(member_path),
        resolution_key(resolution)
    ))
}

/// Read a resolved resource reference, verifying its identity.
pub fn read_resource(reader: SaveReader) -> Result<ResolvedResourceReference, PersistenceError> {
    let requested_path = reader.field("requestedPath").string()?;
    let provenance = read_provenance(reader.field("provenance"))?;
    let identity_field = reader.field("identity");
    let identity_text = identity_field.string()?;
    let identity = ResourceIdentity::parse(&identity_text)
        .map(|parsed| parsed.canonical())
        .ok_or_else(|| PersistenceError::from(identity_field.fail("expected a resource identity")))?;
    let byte_length = reader.field("byteLength").integer(0)?;
    let resolution = read_resolution(reader.field("resolution"))?;
    #[allow(clippy::cast_sign_loss)]
    let byte_length = byte_length as u64;
    let id = create_resource_id(&requested_path, &provenance, &identity, byte_length, &resolution)?;
    if reader.field("id").string()? != id {
        return Err(PersistenceError::from(
            reader.fail("resource identity differs from its provenance"),
        ));
    }
    Ok(ResolvedResourceReference {
        id,
        requested_path,
        provenance,
        identity,
        byte_length,
        resolution,
    })
}

/// Write a resolved resource reference.
#[must_use]
pub fn write_resource(resource: &ResolvedResourceReference) -> SaveJson {
    #[allow(clippy::cast_possible_wrap)]
    obj(vec![
        ("id", str(&resource.id)),
        ("requestedPath", str(&resource.requested_path)),
        ("provenance", write_provenance(&resource.provenance)),
        ("identity", str(&resource.identity)),
        ("byteLength", int(resource.byte_length as i64)),
        ("resolution", write_resolution(&resource.resolution)),
    ])
}

/// Build a `mount:namespace:name` id (donor `createMountId`).
pub fn create_mount_id(namespace: &str, name: &str) -> Result<String, PersistenceError> {
    if !valid_identity_part(namespace) || !valid_identity_part(name) {
        return Err(PersistenceError::BadSave(format!(
            "expected a mount identity: mount:{namespace}:{name}"
        )));
    }
    Ok(format!("mount:{namespace}:{name}"))
}

/// Build a `mount-plan:namespace:revision` id (donor `createMountPlanId`).
pub fn create_mount_plan_id(namespace: &str, revision: &str) -> Result<String, PersistenceError> {
    if !valid_identity_part(namespace) || !valid_identity_part(revision) {
        return Err(PersistenceError::BadSave(format!(
            "expected a mount-plan identity: mount-plan:{namespace}:{revision}"
        )));
    }
    Ok(format!("mount-plan:{namespace}:{revision}"))
}

/// Borrow a mount identity.
#[must_use]
pub fn mount_identity(mount: &ContentMount) -> &MountIdentity {
    match mount {
        ContentMount::Archive { identity, .. } | ContentMount::Loose { identity, .. } => identity,
    }
}

/// Whether a mount is archive-backed.
#[must_use]
pub fn mount_is_archive(mount: &ContentMount) -> bool {
    matches!(mount, ContentMount::Archive { .. })
}

/// Archive details: format, archive path, archive digest.
#[must_use]
pub fn mount_archive_details(mount: &ContentMount) -> Option<(&str, &str, &str)> {
    match mount {
        ContentMount::Archive {
            format,
            archive_path,
            archive_digest,
            ..
        } => Some((format, archive_path, archive_digest)),
        ContentMount::Loose { .. } => None,
    }
}

/// Clone a mount with a replacement identity and machine-independent path.
///
/// Archive mounts take the path as their archive path, loose mounts as
/// their root path (donor `createUnifiedComposition` mount normalization).
#[must_use]
pub fn mount_relocated(mount: &ContentMount, identity: MountIdentity, path: String) -> ContentMount {
    match mount {
        ContentMount::Archive {
            format, archive_digest, ..
        } => ContentMount::Archive {
            identity,
            format: format.clone(),
            archive_path: path,
            archive_digest: archive_digest.clone(),
        },
        ContentMount::Loose { .. } => ContentMount::Loose {
            identity,
            root_path: path,
        },
    }
}

/// Borrow a provenance mount.
#[must_use]
pub fn provenance_mount(provenance: &ResourceProvenance) -> &ContentMount {
    match provenance {
        ResourceProvenance::Archive { mount, .. } | ResourceProvenance::Loose { mount, .. } => mount,
    }
}

/// Borrow a resolution plan.
#[must_use]
pub fn resolution_plan(resolution: &ResourceResolution) -> &str {
    match resolution {
        ResourceResolution::DefaultOrder { plan, .. }
        | ResourceResolution::PrefixOrder { plan, .. }
        | ResourceResolution::Link { plan, .. } => plan,
    }
}

/// Precedence rank (links carry no rank).
#[must_use]
pub fn resolution_rank(resolution: &ResourceResolution) -> u64 {
    match resolution {
        ResourceResolution::DefaultOrder { rank, .. } | ResourceResolution::PrefixOrder { rank, .. } => *rank,
        ResourceResolution::Link { .. } => 0,
    }
}

/// Whether a resolution won through a link.
#[must_use]
pub fn resolution_is_link(resolution: &ResourceResolution) -> bool {
    matches!(resolution, ResourceResolution::Link { .. })
}

/// Rebind a resolved reference onto a replacement mount and plan.
///
/// Mirrors the unified composition's `resourceWithMount`: path shapes are
/// validated, the provenance mount swaps only across matching mount kinds,
/// link paths are validated, the resolution plan swaps, and the resource id
/// is recomputed.
pub fn rebind_resource_reference(
    resource: &ResolvedResourceReference,
    mount: ContentMount,
    plan: String,
) -> Result<ResolvedResourceReference, PersistenceError> {
    resource_path(&resource.requested_path)?;
    let member_path = match &resource.provenance {
        ResourceProvenance::Archive { member_path, .. } | ResourceProvenance::Loose { member_path, .. } => member_path,
    };
    resource_path(member_path)?;
    let kind_matches = matches!(
        (&resource.provenance, &mount),
        (ResourceProvenance::Archive { .. }, ContentMount::Archive { .. })
            | (ResourceProvenance::Loose { .. }, ContentMount::Loose { .. })
    );
    if !kind_matches {
        return Err(PersistenceError::BadSave(
            "Unified resource mount kind differs".to_string(),
        ));
    }
    let provenance = match &resource.provenance {
        ResourceProvenance::Archive {
            member_path,
            member_index,
            ..
        } => ResourceProvenance::Archive {
            mount: Box::new(mount),
            member_path: member_path.clone(),
            member_index: *member_index,
        },
        ResourceProvenance::Loose { member_path, .. } => ResourceProvenance::Loose {
            mount: Box::new(mount),
            member_path: member_path.clone(),
        },
    };
    let resolution = match &resource.resolution {
        ResourceResolution::DefaultOrder { rank, .. } => ResourceResolution::DefaultOrder { plan, rank: *rank },
        ResourceResolution::PrefixOrder { prefix, rank, .. } => ResourceResolution::PrefixOrder {
            plan,
            prefix: prefix.clone(),
            rank: *rank,
        },
        ResourceResolution::Link {
            source_prefix,
            target_path,
            ..
        } => {
            let source = source_prefix.strip_suffix('/').unwrap_or(source_prefix);
            resource_path(source)?;
            resource_path(target_path)?;
            ResourceResolution::Link {
                plan,
                source_prefix: source_prefix.clone(),
                target_path: target_path.clone(),
            }
        }
    };
    let id = create_resource_id(
        &resource.requested_path,
        &provenance,
        &resource.identity,
        resource.byte_length,
        &resolution,
    )?;
    Ok(ResolvedResourceReference {
        id,
        requested_path: resource.requested_path.clone(),
        provenance,
        identity: resource.identity.clone(),
        byte_length: resource.byte_length,
        resolution,
    })
}

/// Prefix mount order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixMountOrder {
    /// Prefix.
    pub prefix: String,
    /// Mounts.
    pub mounts: Vec<String>,
}

/// Resolved mount plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMountPlan {
    /// Id.
    pub id: String,
    /// Mounts.
    pub mounts: Vec<ContentMount>,
    /// Default order.
    pub default_order: Vec<String>,
    /// Prefix orders.
    pub prefix_orders: Vec<PrefixMountOrder>,
}

fn read_mount_plan(reader: SaveReader) -> Result<ResolvedMountPlan, PersistenceError> {
    Ok(ResolvedMountPlan {
        id: read_mount_plan_id(reader.field("id"))?,
        mounts: reader.field("mounts").list(read_mount)?,
        default_order: reader.field("defaultOrder").list(read_mount_id)?,
        prefix_orders: reader
            .field("prefixOrders")
            .list(|order| -> Result<PrefixMountOrder, PersistenceError> {
                Ok(PrefixMountOrder {
                    prefix: order.field("prefix").string()?,
                    mounts: order.field("mounts").list(read_mount_id)?,
                })
            })?,
    })
}

fn write_mount_plan(plan: &ResolvedMountPlan) -> SaveJson {
    obj(vec![
        ("id", str(&plan.id)),
        ("mounts", arr(plan.mounts.iter().map(write_mount).collect())),
        (
            "defaultOrder",
            arr(plan.default_order.iter().map(|id| str(id)).collect()),
        ),
        (
            "prefixOrders",
            arr(plan
                .prefix_orders
                .iter()
                .map(|order| {
                    obj(vec![
                        ("prefix", str(&order.prefix)),
                        ("mounts", arr(order.mounts.iter().map(|id| str(id)).collect())),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Campaign selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignSelection {
    /// No campaign.
    None,
    /// Campaign.
    Campaign {
        /// Mission.
        mission: ProviderRef,
        /// Gamecode.
        gamecode: ProviderRef,
    },
}

fn read_campaign(reader: SaveReader) -> Result<CampaignSelection, PersistenceError> {
    use qa_world::save::shared::read_provider_ref;
    if reader.field("kind").choice_str(&["none", "campaign"])? == "none" {
        Ok(CampaignSelection::None)
    } else {
        Ok(CampaignSelection::Campaign {
            mission: read_provider_ref(reader.field("mission"))?,
            gamecode: read_provider_ref(reader.field("gamecode"))?,
        })
    }
}

fn write_campaign(selection: &CampaignSelection) -> SaveJson {
    use qa_world::save::shared::write_provider_ref;
    match selection {
        CampaignSelection::None => obj(vec![("kind", str("none"))]),
        CampaignSelection::Campaign { mission, gamecode } => obj(vec![
            ("kind", str("campaign")),
            ("mission", write_provider_ref(mission)),
            ("gamecode", write_provider_ref(gamecode)),
        ]),
    }
}

/// Monster selection target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonsterSelectionTarget {
    /// Map-defined monster.
    MapDefined,
    /// Sourced monster.
    Source {
        /// Source.
        source: ProviderRef,
        /// Classname.
        classname: String,
    },
}

/// Enemy selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnemySelection {
    /// Map-defined enemies.
    MapDefined,
    /// Replaced enemies.
    Replace {
        /// Default.
        default: MonsterSelectionTarget,
        /// Overrides by classname.
        by_classname: Vec<(String, MonsterSelectionTarget)>,
    },
}

fn read_monster_target(reader: SaveReader) -> Result<MonsterSelectionTarget, PersistenceError> {
    use qa_world::save::shared::read_provider_ref;
    if !reader.field("kind").is_missing() {
        reader.field("kind").literal_str("map-defined")?;
        let extra = match &reader.value {
            Some(SaveJson::Object(members)) => members.iter().any(|(key, _)| key != "kind"),
            _ => true,
        };
        if extra {
            return Err(PersistenceError::from(
                reader.fail("expected only a native monster target kind"),
            ));
        }
        return Ok(MonsterSelectionTarget::MapDefined);
    }
    Ok(MonsterSelectionTarget::Source {
        source: read_provider_ref(reader.field("source"))?,
        classname: reader.field("classname").string()?,
    })
}

fn write_monster_target(target: &MonsterSelectionTarget) -> SaveJson {
    use qa_world::save::shared::write_provider_ref;
    match target {
        MonsterSelectionTarget::MapDefined => obj(vec![("kind", str("map-defined"))]),
        MonsterSelectionTarget::Source { source, classname } => obj(vec![
            ("source", write_provider_ref(source)),
            ("classname", str(classname)),
        ]),
    }
}

fn read_enemies(reader: SaveReader) -> Result<EnemySelection, PersistenceError> {
    if reader.field("kind").choice_str(&["map-defined", "replace"])? == "map-defined" {
        return Ok(EnemySelection::MapDefined);
    }
    let overrides = reader.field("byClassname");
    let names = match &overrides.value {
        Some(SaveJson::Object(members)) => members.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>(),
        _ => {
            return Err(PersistenceError::from(
                overrides.fail("expected authored classname replacements"),
            ))
        }
    };
    Ok(EnemySelection::Replace {
        default: read_monster_target(reader.field("default"))?,
        by_classname: names
            .into_iter()
            .map(|name| read_monster_target(overrides.field(&name)).map(|target| (name, target)))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn write_enemies(selection: &EnemySelection) -> SaveJson {
    match selection {
        EnemySelection::MapDefined => obj(vec![("kind", str("map-defined"))]),
        EnemySelection::Replace { default, by_classname } => obj(vec![
            ("kind", str("replace")),
            ("default", write_monster_target(default)),
            (
                "byClassname",
                obj(by_classname
                    .iter()
                    .map(|(name, target)| (name.as_str(), write_monster_target(target)))
                    .collect()),
            ),
        ]),
    }
}

/// Environment selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentSelection {
    /// Audio content default.
    AudioContent,
    /// Disabled.
    Disabled,
    /// Selected resource.
    Selected {
        /// Content.
        content: String,
        /// Path.
        path: String,
    },
}

fn read_environment(reader: SaveReader) -> Result<EnvironmentSelection, PersistenceError> {
    if reader.is_missing() {
        return Ok(EnvironmentSelection::AudioContent);
    }
    let kind = reader
        .field("kind")
        .choice_str(&["audio-content", "disabled", "selected"])?;
    if kind != "selected" {
        return Ok(if kind == "disabled" {
            EnvironmentSelection::Disabled
        } else {
            EnvironmentSelection::AudioContent
        });
    }
    let resource = reader.field("resource");
    Ok(EnvironmentSelection::Selected {
        content: read_content_id(resource.field("content"))?,
        path: resource.field("path").string()?,
    })
}

fn write_environment(selection: &EnvironmentSelection) -> SaveJson {
    match selection {
        EnvironmentSelection::AudioContent => obj(vec![("kind", str("audio-content"))]),
        EnvironmentSelection::Disabled => obj(vec![("kind", str("disabled"))]),
        EnvironmentSelection::Selected { content, path } => obj(vec![
            ("kind", str("selected")),
            ("resource", obj(vec![("content", str(content)), ("path", str(path))])),
        ]),
    }
}

/// Presentation selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationSelection {
    /// Doppler.
    pub doppler: String,
    /// Environment.
    pub environment: EnvironmentSelection,
    /// Assets.
    pub assets: String,
    /// HUD.
    pub hud: ProviderRef,
    /// Effects.
    pub effects: ProviderRef,
    /// Audio.
    pub audio: ProviderRef,
}

fn read_presentation(reader: SaveReader) -> Result<PresentationSelection, PersistenceError> {
    use qa_world::save::shared::read_provider_ref;
    let doppler = reader.field("doppler");
    Ok(PresentationSelection {
        doppler: if doppler.is_missing() {
            "source".to_string()
        } else {
            doppler.field("kind").choice_str(&["source", "disabled"])?
        },
        environment: read_environment(reader.field("environment"))?,
        assets: read_content_id(reader.field("assets"))?,
        hud: read_provider_ref(reader.field("hud"))?,
        effects: read_provider_ref(reader.field("effects"))?,
        audio: read_provider_ref(reader.field("audio"))?,
    })
}

fn write_presentation(selection: &PresentationSelection) -> SaveJson {
    use qa_world::save::shared::write_provider_ref;
    obj(vec![
        ("doppler", obj(vec![("kind", str(&selection.doppler))])),
        ("environment", write_environment(&selection.environment)),
        ("assets", str(&selection.assets)),
        ("hud", write_provider_ref(&selection.hud)),
        ("effects", write_provider_ref(&selection.effects)),
        ("audio", write_provider_ref(&selection.audio)),
    ])
}

/// Resolved execution module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExecutionModule {
    /// Owner.
    pub owner: ProviderRef,
    /// Role.
    pub role: String,
    /// API.
    pub api: GameApi,
    /// Implementation.
    pub implementation: ExecutionImplementation,
}

/// Execution implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionImplementation {
    /// Built-in implementation.
    Builtin {
        /// Implementation.
        implementation: String,
    },
    /// QuakeC artifact.
    Quakec {
        /// Artifact.
        artifact: ResolvedResourceReference,
    },
    /// QVM artifact.
    Qvm {
        /// Artifact.
        artifact: ResolvedResourceReference,
    },
    /// Native artifact.
    Native {
        /// Artifact.
        artifact: ResolvedResourceReference,
        /// ABI profile.
        profile: NativeCallAbi,
    },
}

/// Borrow an execution module artifact, if it has one.
#[must_use]
pub fn module_artifact(module: &ResolvedExecutionModule) -> Option<&ResolvedResourceReference> {
    match &module.implementation {
        ExecutionImplementation::Builtin { .. } => None,
        ExecutionImplementation::Quakec { artifact }
        | ExecutionImplementation::Qvm { artifact }
        | ExecutionImplementation::Native { artifact, .. } => Some(artifact),
    }
}

/// Rebind an execution module's artifact, leaving TypeScript modules unchanged.
///
/// Mirrors the unified composition's recipe rebind over execution modules.
pub fn map_module_artifact<E>(
    module: &ResolvedExecutionModule,
    rebind: &mut impl FnMut(&ResolvedResourceReference) -> Result<ResolvedResourceReference, E>,
) -> Result<ResolvedExecutionModule, E> {
    let mut mapped = module.clone();
    match &mut mapped.implementation {
        ExecutionImplementation::Builtin { .. } => {}
        ExecutionImplementation::Quakec { artifact }
        | ExecutionImplementation::Qvm { artifact }
        | ExecutionImplementation::Native { artifact, .. } => {
            *artifact = rebind(artifact)?;
        }
    }
    Ok(mapped)
}

fn api_kind(api: &GameApi) -> &'static str {
    match api {
        GameApi::Q1Netquake => "q1-netquake",
        GameApi::Q1Quakeworld => "q1-quakeworld",
        GameApi::Q2ClassicGame => "q2-classic-game",
        GameApi::Q2RereleaseGame => "q2-rerelease-game",
        GameApi::Q2RereleaseCgame => "q2-rerelease-cgame",
        GameApi::Q3Qagame { .. } => "q3-qagame",
        GameApi::Q3Cgame { .. } => "q3-cgame",
        GameApi::Q3Ui { .. } => "q3-ui",
    }
}

fn read_execution(reader: SaveReader) -> Result<ResolvedExecutionModule, PersistenceError> {
    use qa_world::save::shared::read_provider_ref;
    let owner = read_provider_ref(reader.field("owner"))?;
    let api = read_api(reader.field("api"))?;
    let role = reader.field("role").choice_str(&["server-game", "client-game", "ui"])?;
    let kind = reader
        .field("kind")
        .choice_str(&["typescript", "quakec", "qvm", "native"])?;
    let fail = |message: &str| PersistenceError::from(reader.fail(message));
    if kind == "typescript" {
        let implementation = namespaced(reader.field("implementation"))?;
        let expected = match api_kind(&api) {
            "q2-rerelease-cgame" | "q3-cgame" => "client-game",
            "q3-ui" => "ui",
            _ => "server-game",
        };
        if role != expected {
            let message = match expected {
                "client-game" => "cgame API requires client-game role",
                "ui" => "UI API requires ui role",
                _ => "game API requires server-game role",
            };
            return Err(fail(message));
        }
        return Ok(ResolvedExecutionModule {
            owner,
            role,
            api,
            implementation: ExecutionImplementation::Builtin { implementation },
        });
    }
    let artifact = read_resource(reader.field("artifact"))?;
    if kind == "quakec" {
        if role != "server-game" || !matches!(api, GameApi::Q1Netquake | GameApi::Q1Quakeworld) {
            return Err(fail("QuakeC requires a Q1 server-game API"));
        }
        return Ok(ResolvedExecutionModule {
            owner,
            role,
            api,
            implementation: ExecutionImplementation::Quakec { artifact },
        });
    }
    if kind == "qvm" {
        let expected = match api_kind(&api) {
            "q3-qagame" => Some("server-game"),
            "q3-cgame" => Some("client-game"),
            "q3-ui" => Some("ui"),
            _ => None,
        };
        match expected {
            Some(expected) if role == expected => {
                return Ok(ResolvedExecutionModule {
                    owner,
                    role,
                    api,
                    implementation: ExecutionImplementation::Qvm { artifact },
                });
            }
            Some("server-game") => return Err(fail("qagame requires server-game role")),
            Some("client-game") => return Err(fail("cgame requires client-game role")),
            Some("ui") => return Err(fail("UI requires ui role")),
            _ => return Err(fail("QVM requires a Q3 API")),
        }
    }
    let profile = read_native_abi(reader.field("profile"))?;
    let expected = match api_kind(&api) {
        "q2-classic-game" | "q2-rerelease-game" | "q3-qagame" => Some("server-game"),
        "q2-rerelease-cgame" | "q3-cgame" => Some("client-game"),
        "q3-ui" => Some("ui"),
        _ => None,
    };
    match expected {
        Some(expected) if role == expected => Ok(ResolvedExecutionModule {
            owner,
            role,
            api,
            implementation: ExecutionImplementation::Native { artifact, profile },
        }),
        Some("server-game") => Err(fail("game API requires server-game role")),
        Some("client-game") => Err(fail("cgame API requires client-game role")),
        Some("ui") => Err(fail("UI API requires ui role")),
        _ => Err(fail("native game module cannot use a QuakeC API")),
    }
}

fn write_execution(module: &ResolvedExecutionModule) -> SaveJson {
    use qa_world::save::shared::write_provider_ref;
    let mut members = vec![
        ("owner", write_provider_ref(&module.owner)),
        ("api", write_api(&module.api)),
        ("role", str(&module.role)),
    ];
    match &module.implementation {
        ExecutionImplementation::Builtin { implementation } => {
            members.push(("kind", str("typescript")));
            members.push(("implementation", str(implementation)));
        }
        ExecutionImplementation::Quakec { artifact } => {
            members.push(("kind", str("quakec")));
            members.push(("artifact", write_resource(artifact)));
        }
        ExecutionImplementation::Qvm { artifact } => {
            members.push(("kind", str("qvm")));
            members.push(("artifact", write_resource(artifact)));
        }
        ExecutionImplementation::Native { artifact, profile } => {
            members.push(("kind", str("native")));
            members.push(("artifact", write_resource(artifact)));
            members.push(("profile", write_native_call_abi(profile)));
        }
    }
    obj(members)
}

/// Grapple selection.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum GrappleSelection {
    /// Disabled.
    Disabled,
    /// Enabled.
    Enabled {
        /// Source.
        source: ProviderRef,
        /// Binding.
        binding: String,
        /// Mechanic.
        mechanic: String,
        /// Edition.
        edition: String,
        /// QVM profile (q3-qvm only).
        profile: Option<QvmGrappleDefinition>,
    },
}

fn read_grapple(reader: SaveReader) -> Result<GrappleSelection, PersistenceError> {
    use qa_world::save::shared::read_provider_ref;
    if reader.field("kind").choice_str(&["disabled", "enabled"])? == "disabled" {
        return Ok(GrappleSelection::Disabled);
    }
    let source = read_provider_ref(reader.field("source"))?;
    let binding = reader.field("binding").choice_str(&["slot", "offhand"])?;
    let mechanic = reader
        .field("mechanic")
        .choice_str(&["q1-threewave", "q2-ctf", "q2-lmctf", "q3-qvm"])?;
    let edition = match mechanic.as_str() {
        "q2-lmctf" | "q3-qvm" => reader.field("edition").literal_str("classic")?,
        _ => reader.field("edition").choice_str(&["classic", "rerelease"])?,
    };
    Ok(GrappleSelection::Enabled {
        source,
        binding,
        mechanic: mechanic.clone(),
        edition,
        profile: if mechanic == "q3-qvm" {
            Some(read_qvm_grapple_definition(reader.field("profile"))?)
        } else {
            None
        },
    })
}

fn write_grapple(selection: &GrappleSelection) -> SaveJson {
    use qa_world::save::shared::write_provider_ref;
    match selection {
        GrappleSelection::Disabled => obj(vec![("kind", str("disabled"))]),
        GrappleSelection::Enabled {
            source,
            binding,
            mechanic,
            edition,
            profile,
        } => {
            let mut members = vec![
                ("kind", str("enabled")),
                ("source", write_provider_ref(source)),
                ("binding", str(binding)),
                ("mechanic", str(mechanic)),
                ("edition", str(edition)),
            ];
            if let Some(profile) = profile {
                members.push(("profile", write_qvm_grapple_definition(profile)));
            }
            obj(members)
        }
    }
}

/// Hand-grenade selection.
#[derive(Debug, Clone, PartialEq)]
pub enum HandGrenadeSelection {
    /// Disabled.
    Disabled,
    /// Enabled.
    Enabled {
        /// Source.
        source: ProviderRef,
        /// Edition.
        edition: String,
        /// Initial ammo.
        initial_ammo: u64,
        /// Capacity.
        capacity: u64,
    },
}

fn read_hand_grenades(reader: SaveReader) -> Result<HandGrenadeSelection, PersistenceError> {
    use qa_world::save::shared::read_provider_ref;
    if reader.field("kind").choice_str(&["disabled", "enabled"])? == "disabled" {
        return Ok(HandGrenadeSelection::Disabled);
    }
    let initial_ammo = reader.field("initialAmmo").integer(0)?;
    let capacity = reader.field("capacity").integer(0)?;
    if initial_ammo > capacity {
        return Err(PersistenceError::from(
            reader.fail("hand grenade allowance exceeds capacity"),
        ));
    }
    reader.field("binding").literal_str("offhand")?;
    Ok(HandGrenadeSelection::Enabled {
        source: read_provider_ref(reader.field("source"))?,
        edition: reader.field("edition").choice_str(&["classic", "rerelease"])?,
        #[allow(clippy::cast_sign_loss)]
        initial_ammo: initial_ammo as u64,
        #[allow(clippy::cast_sign_loss)]
        capacity: capacity as u64,
    })
}

fn write_hand_grenades(selection: &HandGrenadeSelection) -> SaveJson {
    use qa_world::save::shared::write_provider_ref;
    match selection {
        HandGrenadeSelection::Disabled => obj(vec![("kind", str("disabled"))]),
        #[allow(clippy::cast_possible_wrap)]
        HandGrenadeSelection::Enabled {
            source,
            edition,
            initial_ammo,
            capacity,
        } => obj(vec![
            ("kind", str("enabled")),
            ("source", write_provider_ref(source)),
            ("binding", str("offhand")),
            ("edition", str(edition)),
            ("initialAmmo", int(*initial_ammo as i64)),
            ("capacity", int(*capacity as i64)),
        ]),
    }
}

/// Equipment selection.
#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentSelection {
    /// Grapple.
    pub grapple: GrappleSelection,
    /// Hand grenades.
    pub hand_grenades: HandGrenadeSelection,
}

/// Read an equipment selection.
pub fn read_equipment(reader: SaveReader) -> Result<EquipmentSelection, PersistenceError> {
    Ok(EquipmentSelection {
        grapple: read_grapple(reader.field("grapple"))?,
        hand_grenades: read_hand_grenades(reader.field("handGrenades"))?,
    })
}

/// Write an equipment selection.
#[must_use]
pub fn write_equipment(equipment: &EquipmentSelection) -> SaveJson {
    obj(vec![
        ("grapple", write_grapple(&equipment.grapple)),
        ("handGrenades", write_hand_grenades(&equipment.hand_grenades)),
    ])
}

/// QVM weapon behavior component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWeaponComponent {
    /// ABI profile.
    pub abi_profile: String,
    /// Entity stride.
    pub entity_stride: i64,
    /// Level time.
    pub level_time: i64,
    /// Allocate.
    pub allocate: i64,
    /// Free.
    pub free: i64,
    /// Field offsets.
    pub fields: (i64, i64, i64, i64),
}

/// Weapon behavior component.
#[derive(Debug, Clone, PartialEq)]
pub enum WeaponBehaviorComponent {
    /// QVM layout.
    Qvm(QvmWeaponComponent),
    /// Rerelease native declaration.
    RereleaseNative {
        /// Declaration.
        declaration: NativeWeaponBehaviorDeclaration,
    },
}

/// Weapon behavior entry.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorEntry {
    /// Source.
    pub source: ProviderRef,
    /// Artifact.
    pub artifact: ResolvedResourceReference,
    /// Component.
    pub component: Option<WeaponBehaviorComponent>,
    /// Definition.
    pub definition: WeaponBehaviorDefinition,
}

fn read_weapon_behavior(
    reader: SaveReader,
    legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
) -> Result<WeaponBehaviorEntry, PersistenceError> {
    use qa_world::save::shared::read_provider_ref;
    let value = reader.field("definition");
    let module = read_module(value.field("module"))?;
    let source = read_provider_ref(reader.field("source"))?;
    let artifact = read_resource(reader.field("artifact"))?;
    // The artifact carries a value identity while the module carries a
    // content hash: the two are incommensurable by design, so parse only
    // checks selection (provider and path). Byte equality is enforced
    // when the artifact is opened live against its declaration.
    if source.provider != module.id || artifact.requested_path != module.artifact_path {
        return Err(PersistenceError::from(
            reader.fail("weapon behavior source differs from selected artifact"),
        ));
    }
    let fire = read_weapon_behavior_callback(value.field("fire"), &module)?;
    let activate = value
        .field("activate")
        .nullable(|entry| read_weapon_behavior_callback(entry, &module))?;
    let definition = WeaponBehaviorDefinition {
        id: namespaced(value.field("id"))?,
        title: value.field("title").string()?,
        module: module.clone(),
        role: value
            .field("role")
            .choice_str(&["rocket", "grenade", "nail", "bolt", "plasma", "energy", "grapple"])?,
        aspect: value.field("aspect").literal_str("trajectory")?,
        activate: activate.clone(),
        fire: fire.clone(),
    };
    let saved_component = reader.field("component");
    let component_kind = if saved_component.is_missing() {
        None
    } else {
        Some(saved_component.field("kind").choice_str(&["qvm", "rerelease-native"])?)
    };
    let mut component = None;
    if component_kind.as_deref() == Some("rerelease-native")
        || component_kind.is_none() && matches!(fire, WeaponBehaviorCallback::NativeArtifact { .. })
    {
        let target = if component_kind.is_none() {
            saved_component.clone()
        } else {
            saved_component.field("declaration")
        };
        component = Some(WeaponBehaviorComponent::RereleaseNative {
            declaration: read_saved_native_weapon_declaration(target, &definition, legacy_native(&definition))?,
        });
    } else if component_kind.as_deref() == Some("qvm") {
        let layout = saved_component.field("layout");
        let fields = layout.field("fields");
        let entity_stride = layout.field("entityStride").integer(4)?;
        let level_time = layout.field("levelTime").integer(4)?;
        if entity_stride % 4 != 0 || level_time % 4 != 0 {
            return Err(PersistenceError::from(layout.fail("unaligned QVM behavior layout")));
        }
        let mut occupied = HashSet::new();
        let mut offset = |name: &str| {
            let field = fields.field(name);
            let result = field.integer(0)?;
            if result % 4 != 0 || result > entity_stride - 4 || !occupied.insert(result) {
                return Err(PersistenceError::from(
                    field.fail("overlapping or out-of-range QVM behavior field"),
                ));
            }
            Ok(result)
        };
        let inuse = offset("inuse")?;
        let nextthink = offset("nextthink")?;
        let think = offset("think")?;
        let health = offset("health")?;
        layout.field("fireAbi").literal_str("entity-pointer-start-direction")?;
        component = Some(WeaponBehaviorComponent::Qvm(QvmWeaponComponent {
            abi_profile: saved_component
                .field("abiProfile")
                .choice_str(&["q3-modern", "q3-1.16n-base"])?,
            entity_stride,
            level_time,
            allocate: layout.field("allocate").integer(1)?,
            free: layout.field("free").integer(1)?,
            fields: (inuse, nextthink, think, health),
        }));
    }
    let qvm_fire = matches!(fire, WeaponBehaviorCallback::Qvm { .. });
    let native_fire = matches!(fire, WeaponBehaviorCallback::NativeArtifact { .. });
    let quakec_fire = matches!(fire, WeaponBehaviorCallback::Quakec { .. });
    if qvm_fire
        && (!matches!(component, Some(WeaponBehaviorComponent::Qvm(_)))
            || !matches!(activate, None | Some(WeaponBehaviorCallback::Qvm { .. })))
    {
        return Err(PersistenceError::from(
            reader.fail("QVM behavior requires its source layout and QVM callbacks"),
        ));
    }
    if native_fire
        && (!matches!(component, Some(WeaponBehaviorComponent::RereleaseNative { .. }))
            || !matches!(activate, None | Some(WeaponBehaviorCallback::NativeArtifact { .. })))
        || quakec_fire
            && (component.is_some() || !matches!(activate, None | Some(WeaponBehaviorCallback::Quakec { .. })))
    {
        return Err(PersistenceError::from(
            reader.fail("weapon behavior callbacks and component declaration do not match"),
        ));
    }
    Ok(WeaponBehaviorEntry {
        source,
        artifact,
        component,
        definition,
    })
}

fn write_weapon_behavior(entry: &WeaponBehaviorEntry) -> SaveJson {
    use qa_world::save::shared::write_provider_ref;
    let mut members = vec![
        ("source", write_provider_ref(&entry.source)),
        ("artifact", write_resource(&entry.artifact)),
    ];
    if let Some(component) = &entry.component {
        members.push((
            "component",
            match component {
                WeaponBehaviorComponent::Qvm(layout) => obj(vec![
                    ("kind", str("qvm")),
                    ("abiProfile", str(&layout.abi_profile)),
                    (
                        "layout",
                        obj(vec![
                            ("entityStride", int(layout.entity_stride)),
                            ("levelTime", int(layout.level_time)),
                            ("allocate", int(layout.allocate)),
                            ("free", int(layout.free)),
                            (
                                "fields",
                                obj(vec![
                                    ("inuse", int(layout.fields.0)),
                                    ("nextthink", int(layout.fields.1)),
                                    ("think", int(layout.fields.2)),
                                    ("health", int(layout.fields.3)),
                                ]),
                            ),
                            ("fireAbi", str("entity-pointer-start-direction")),
                        ]),
                    ),
                ]),
                WeaponBehaviorComponent::RereleaseNative { declaration } => obj(vec![
                    ("kind", str("rerelease-native")),
                    ("declaration", write_native_weapon_declaration(declaration)),
                ]),
            },
        ));
    }
    members.push((
        "definition",
        obj(vec![
            ("id", str(&entry.definition.id)),
            ("title", str(&entry.definition.title)),
            ("module", write_module(&entry.definition.module)),
            ("role", str(&entry.definition.role)),
            ("aspect", str(&entry.definition.aspect)),
            ("fire", write_weapon_behavior_callback(&entry.definition.fire)),
            (
                "activate",
                entry
                    .definition
                    .activate
                    .as_ref()
                    .map_or(SaveJson::Null, write_weapon_behavior_callback),
            ),
        ]),
    ));
    obj(members)
}

/// Map selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSelection {
    /// Geometry content.
    pub geometry_content: String,
    /// Geometry resource.
    pub geometry: ResolvedResourceReference,
    /// Entities provider.
    pub entities: ProviderRef,
}

/// Timing entry.
#[derive(Debug, Clone, PartialEq)]
pub struct TimingEntry {
    /// Provider.
    pub provider: String,
    /// Clock.
    pub clock: ClockProfile,
    /// Numeric profile.
    pub numeric: SavedNumericProfile,
}

/// Executable recipe.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutableRecipe {
    /// Mods.
    pub mods: Vec<ResolvedGameplayMod>,
    /// Weapon behaviors.
    pub weapon_behaviors: Vec<WeaponBehaviorEntry>,
    /// Id.
    pub id: String,
    /// Preset.
    pub preset: String,
    /// Map.
    pub map: MapSelection,
    /// Campaign.
    pub campaign: CampaignSelection,
    /// Movement.
    pub movement: ProviderRef,
    /// Character.
    pub character: CharacterSelection,
    /// Weapons.
    pub weapons: Vec<ProviderRef>,
    /// Equipment.
    pub equipment: EquipmentSelection,
    /// Enemies.
    pub enemies: EnemySelection,
    /// Presentation.
    pub presentation: PresentationSelection,
    /// Engine behavior.
    pub engine_behavior: ProviderRef,
    /// Combat.
    pub combat: ProviderRef,
    /// Inventory.
    pub inventory: ProviderRef,
    /// Match.
    pub match_provider: ProviderRef,
    /// Transition.
    pub transition: ProviderRef,
    /// Execution.
    pub execution: Vec<ResolvedExecutionModule>,
    /// Mounts.
    pub mounts: ResolvedMountPlan,
    /// Resources.
    pub resources: Vec<ResolvedResourceReference>,
    /// Timing.
    pub timing: Vec<TimingEntry>,
    /// Ordering.
    pub ordering: FrameOrdering,
}

/// Read an executable recipe.
pub fn read_recipe(
    reader: SaveReader,
    legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
) -> Result<ExecutableRecipe, PersistenceError> {
    use qa_world::save::shared::read_provider_ref;
    let map = reader.field("map");
    Ok(ExecutableRecipe {
        mods: if reader.field("mods").is_missing() {
            Vec::new()
        } else {
            reader.field("mods").list(read_gameplay_mod)?
        },
        weapon_behaviors: if reader.field("weaponBehaviors").is_missing() {
            Vec::new()
        } else {
            reader
                .field("weaponBehaviors")
                .list(|value| read_weapon_behavior(value, legacy_native))?
        },
        id: {
            reader.field("schemaVersion").literal_i64(3)?;
            read_recipe_id(reader.field("id"))?
        },
        preset: read_recipe_id(reader.field("preset"))?,
        map: MapSelection {
            geometry_content: read_content_id(map.field("geometryContent"))?,
            geometry: read_resource(map.field("geometry"))?,
            entities: read_provider_ref(map.field("entities"))?,
        },
        campaign: read_campaign(reader.field("campaign"))?,
        movement: read_provider_ref(reader.field("movement"))?,
        character: read_character(reader.field("character"))?,
        weapons: reader
            .field("weapons")
            .list(|value| read_provider_ref(value).map_err(PersistenceError::from))?,
        equipment: read_equipment(reader.field("equipment"))?,
        enemies: read_enemies(reader.field("enemies"))?,
        presentation: read_presentation(reader.field("presentation"))?,
        engine_behavior: read_provider_ref(reader.field("engineBehavior"))?,
        combat: read_provider_ref(reader.field("combat"))?,
        inventory: read_provider_ref(reader.field("inventory"))?,
        match_provider: read_provider_ref(reader.field("match"))?,
        transition: read_provider_ref(reader.field("transition"))?,
        execution: reader.field("execution").list(read_execution)?,
        mounts: read_mount_plan(reader.field("mounts"))?,
        resources: reader.field("resources").list(read_resource)?,
        timing: reader
            .field("timing")
            .list(|timing| -> Result<TimingEntry, PersistenceError> {
                Ok(TimingEntry {
                    provider: namespaced(timing.field("provider"))?,
                    clock: read_clock(timing.field("clock"))?,
                    numeric: read_numeric(timing.field("numeric"))?,
                })
            })?,
        ordering: read_ordering(reader.field("ordering"))?,
    })
}

/// Test fixture recipe shared with network composition tests.
#[cfg(test)]
pub(crate) fn fixture_recipe() -> ExecutableRecipe {
    tests::fixture_recipe()
}

/// Write an executable recipe.
#[must_use]
pub fn write_recipe(recipe: &ExecutableRecipe) -> SaveJson {
    use qa_world::save::shared::write_provider_ref;
    let mut members = Vec::new();
    if !recipe.mods.is_empty() {
        members.push(("mods", arr(recipe.mods.iter().map(write_gameplay_mod).collect())));
    }
    if !recipe.weapon_behaviors.is_empty() {
        members.push((
            "weaponBehaviors",
            arr(recipe.weapon_behaviors.iter().map(write_weapon_behavior).collect()),
        ));
    }
    members.extend(vec![
        ("schemaVersion", int(3)),
        ("id", str(&recipe.id)),
        ("preset", str(&recipe.preset)),
        (
            "map",
            obj(vec![
                ("geometryContent", str(&recipe.map.geometry_content)),
                ("geometry", write_resource(&recipe.map.geometry)),
                ("entities", write_provider_ref(&recipe.map.entities)),
            ]),
        ),
        ("campaign", write_campaign(&recipe.campaign)),
        ("movement", write_provider_ref(&recipe.movement)),
        ("character", write_character(&recipe.character)),
        ("weapons", arr(recipe.weapons.iter().map(write_provider_ref).collect())),
        ("equipment", write_equipment(&recipe.equipment)),
        ("enemies", write_enemies(&recipe.enemies)),
        ("presentation", write_presentation(&recipe.presentation)),
        ("engineBehavior", write_provider_ref(&recipe.engine_behavior)),
        ("combat", write_provider_ref(&recipe.combat)),
        ("inventory", write_provider_ref(&recipe.inventory)),
        ("match", write_provider_ref(&recipe.match_provider)),
        ("transition", write_provider_ref(&recipe.transition)),
        ("execution", arr(recipe.execution.iter().map(write_execution).collect())),
        ("mounts", write_mount_plan(&recipe.mounts)),
        ("resources", arr(recipe.resources.iter().map(write_resource).collect())),
        (
            "timing",
            arr(recipe
                .timing
                .iter()
                .map(|entry| {
                    obj(vec![
                        ("provider", str(&entry.provider)),
                        ("clock", write_clock(entry.clock)),
                        ("numeric", write_numeric(&entry.numeric)),
                    ])
                })
                .collect()),
        ),
        ("ordering", write_ordering(&recipe.ordering)),
    ]);
    obj(members)
}

/// Test fixture recipe payload for the unified-save tests.
#[cfg(test)]
pub fn tests_fixture_recipe_json() -> SaveJson {
    write_recipe(&tests::fixture_recipe())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::numeric::{Arithmetic, FloatToInt};
    use qa_guest::checkpoint::ModuleIdentity;
    use qa_world::save::value::{decode_checkpoint_value, encode_checkpoint_value};

    fn provider_ref(provider: &str) -> ProviderRef {
        ProviderRef {
            provider: provider.to_string(),
            content: "q3:classic:base:1".to_string(),
        }
    }

    fn resource(requested_path: &str) -> ResolvedResourceReference {
        let provenance = ResourceProvenance::Loose {
            mount: Box::new(ContentMount::Loose {
                identity: MountIdentity {
                    id: "mount:q3:base".to_string(),
                    content: "q3:classic:base:1".to_string(),
                    generation: 0,
                },
                root_path: "baseq3".to_string(),
            }),
            member_path: requested_path.to_string(),
        };
        let identity = "identity:0:0:128:0".to_string();
        let resolution = ResourceResolution::DefaultOrder {
            plan: "mount-plan:q3:1".to_string(),
            rank: 0,
        };
        let id = create_resource_id(requested_path, &provenance, &identity, 128, &resolution).unwrap();
        ResolvedResourceReference {
            id,
            requested_path: requested_path.to_string(),
            provenance,
            identity,
            byte_length: 128,
            resolution,
        }
    }

    pub fn fixture_recipe() -> ExecutableRecipe {
        recipe()
    }

    fn recipe() -> ExecutableRecipe {
        ExecutableRecipe {
            mods: Vec::new(),
            weapon_behaviors: Vec::new(),
            id: "recipe:q3:1".to_string(),
            preset: "recipe:q3:base".to_string(),
            map: MapSelection {
                geometry_content: "q3:classic:base:1".to_string(),
                geometry: resource("maps/q3dm1.bsp"),
                entities: provider_ref("q3:game"),
            },
            campaign: CampaignSelection::None,
            movement: provider_ref("q3:movement"),
            character: CharacterSelection {
                definition: provider_ref("q3:character"),
                appearance: provider_ref("q3:appearance"),
            },
            weapons: vec![provider_ref("q3:weapons")],
            equipment: EquipmentSelection {
                grapple: GrappleSelection::Disabled,
                hand_grenades: HandGrenadeSelection::Disabled,
            },
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: "source".to_string(),
                environment: EnvironmentSelection::AudioContent,
                assets: "q3:classic:base:1".to_string(),
                hud: provider_ref("q3:hud"),
                effects: provider_ref("q3:effects"),
                audio: provider_ref("q3:audio"),
            },
            engine_behavior: provider_ref("q3:engine"),
            combat: provider_ref("q3:combat"),
            inventory: provider_ref("q3:inventory"),
            match_provider: provider_ref("q3:match"),
            transition: provider_ref("q3:transition"),
            execution: vec![ResolvedExecutionModule {
                owner: provider_ref("q3:game"),
                role: "server-game".to_string(),
                api: GameApi::Q3Qagame { version: 8 },
                implementation: ExecutionImplementation::Builtin {
                    implementation: "q3:game-impl".to_string(),
                },
            }],
            mounts: ResolvedMountPlan {
                id: "mount-plan:q3:1".to_string(),
                mounts: vec![ContentMount::Loose {
                    identity: MountIdentity {
                        id: "mount:q3:base".to_string(),
                        content: "q3:classic:base:1".to_string(),
                        generation: 0,
                    },
                    root_path: "baseq3".to_string(),
                }],
                default_order: vec!["mount:q3:base".to_string()],
                prefix_orders: Vec::new(),
            },
            resources: vec![resource("maps/q3dm1.bsp")],
            timing: vec![TimingEntry {
                provider: "q3:game".to_string(),
                clock: ClockProfile::Q3 {
                    server_frame_milliseconds: 8.0,
                    fixed_movement_milliseconds: None,
                },
                numeric: SavedNumericProfile {
                    id: "q3:binary32".to_string(),
                    arithmetic: Arithmetic::Binary32EachOp,
                    float_to_int: FloatToInt::CheckedTruncation,
                },
            }],
            ordering: FrameOrdering::Native {
                clock: ClockProfile::Q3 {
                    server_frame_milliseconds: 8.0,
                    fixed_movement_milliseconds: None,
                },
            },
        }
    }

    fn round_trip(recipe: &ExecutableRecipe) -> ExecutableRecipe {
        let json = write_recipe(recipe);
        let back = decode_checkpoint_value(&encode_checkpoint_value(&json)).unwrap();
        read_recipe(SaveReader::at(&back, "recipe"), &|_| None).unwrap()
    }

    #[test]
    fn recipes_round_trip() {
        assert_eq!(round_trip(&recipe()), recipe());
        assert_eq!(encode_uri_component("a b/c+d"), "a%20b%2Fc%2Bd");
        assert_eq!(encode_uri_component("AZaz09-_.!~*'()"), "AZaz09-_.!~*'()");
        assert!(resource_path("maps/q3dm1.bsp").is_ok());
        assert!(resource_path("../escape").is_err());
    }

    #[test]
    fn recipe_rules_hold() {
        // Resource identity must match provenance.
        let mut bad = recipe();
        bad.map.geometry.id = "resource:tampered".to_string();
        assert!(read_recipe(SaveReader::new(&write_recipe(&bad)), &|_| None).is_err());
        // Role/API matrix.
        let mut bad = recipe();
        bad.execution[0].role = "ui".to_string();
        assert!(read_recipe(SaveReader::new(&write_recipe(&bad)), &|_| None).is_err());
        // Grenade allowance.
        let mut bad = recipe();
        bad.equipment.hand_grenades = HandGrenadeSelection::Enabled {
            source: provider_ref("q2:game"),
            edition: "classic".to_string(),
            initial_ammo: 9,
            capacity: 5,
        };
        assert!(read_recipe(SaveReader::new(&write_recipe(&bad)), &|_| None).is_err());
        // QVM layout overlap.
        let mut bad = recipe();
        bad.weapon_behaviors = vec![WeaponBehaviorEntry {
            source: ProviderRef {
                provider: "q3:mod".to_string(),
                content: "q3:classic:base:1".to_string(),
            },
            artifact: resource("vm/mod.qvm"),
            component: Some(WeaponBehaviorComponent::Qvm(QvmWeaponComponent {
                abi_profile: "q3-modern".to_string(),
                entity_stride: 64,
                level_time: 64,
                allocate: 1,
                free: 2,
                fields: (0, 0, 8, 12),
            })),
            definition: WeaponBehaviorDefinition {
                id: "q3:plasma".to_string(),
                title: "Plasma".to_string(),
                module: ModuleIdentity {
                    id: "q3:mod".to_string(),
                    artifact_path: "vm/mod.qvm".to_string(),
                    digest: format!("sha256:{}", "4".repeat(64)),
                    revision: "1".to_string(),
                },
                role: "plasma".to_string(),
                aspect: "trajectory".to_string(),
                activate: None,
                fire: WeaponBehaviorCallback::Qvm {
                    module: ModuleIdentity {
                        id: "q3:mod".to_string(),
                        artifact_path: "vm/mod.qvm".to_string(),
                        digest: format!("sha256:{}", "4".repeat(64)),
                        revision: "1".to_string(),
                    },
                    instruction_index: 9,
                },
            },
        }];
        // Overlapping QVM layout fields fail.
        assert!(read_recipe(SaveReader::new(&write_recipe(&bad)), &|_| None).is_err());
    }

    #[test]
    fn unified_ids_validate_parts() {
        assert_eq!(create_mount_id("unified", "0").unwrap(), "mount:unified:0");
        assert_eq!(create_mount_plan_id("unified", "0").unwrap(), "mount-plan:unified:0");
        assert!(create_mount_id("", "0").is_err());
        assert!(create_mount_id("unified", "a/b").is_err());
        assert!(create_mount_plan_id("unified", "..").is_err());
    }

    #[test]
    fn rebind_swaps_mount_and_plan() {
        let original = resource("maps/q3dm1.bsp");
        let fresh = ContentMount::Loose {
            identity: MountIdentity {
                id: "mount:unified:0".to_string(),
                content: "q3:classic:base:1".to_string(),
                generation: 0,
            },
            root_path: "unified/0".to_string(),
        };
        let rebound = rebind_resource_reference(&original, fresh, "mount-plan:unified:0".to_string()).unwrap();
        assert_eq!(resolution_plan(&rebound.resolution), "mount-plan:unified:0");
        assert_eq!(resolution_rank(&rebound.resolution), 0);
        assert!(!resolution_is_link(&rebound.resolution));
        assert_ne!(rebound.id, original.id);
        assert!(rebound.id.starts_with("resource:q3:classic:base:1:mount:unified:0:0:"));
        assert_eq!(
            mount_identity(provenance_mount(&rebound.provenance)).id,
            "mount:unified:0"
        );
    }

    #[test]
    fn rebind_rejects_kind_mismatch() {
        let original = resource("maps/q3dm1.bsp");
        let archive = ContentMount::Archive {
            identity: MountIdentity {
                id: "mount:unified:0".to_string(),
                content: "q3:classic:base:1".to_string(),
                generation: 0,
            },
            format: "pak".to_string(),
            archive_path: "unified/0.pak".to_string(),
            archive_digest: format!("sha256:{}", "4".repeat(64)),
        };
        assert!(rebind_resource_reference(&original, archive, "mount-plan:unified:0".to_string()).is_err());
    }

    #[test]
    fn module_artifact_mapping_skips_typescript() {
        let typescript = ResolvedExecutionModule {
            owner: provider_ref("q3:mod"),
            role: "game".to_string(),
            api: GameApi::Q1Netquake,
            implementation: ExecutionImplementation::Builtin {
                implementation: "mod.js".to_string(),
            },
        };
        assert!(module_artifact(&typescript).is_none());
        let mut calls = 0;
        let mapped = map_module_artifact(&typescript, &mut |_| -> Result<_, PersistenceError> {
            calls += 1;
            unreachable!();
        })
        .unwrap();
        assert_eq!(calls, 0);
        assert_eq!(mapped, typescript);
        let quakec = ResolvedExecutionModule {
            owner: provider_ref("q3:mod"),
            role: "game".to_string(),
            api: GameApi::Q1Netquake,
            implementation: ExecutionImplementation::Quakec {
                artifact: resource("progs.dat"),
            },
        };
        let mapped = map_module_artifact(&quakec, &mut |artifact| {
            rebind_resource_reference(
                artifact,
                ContentMount::Loose {
                    identity: MountIdentity {
                        id: "mount:unified:0".to_string(),
                        content: "q3:classic:base:1".to_string(),
                        generation: 0,
                    },
                    root_path: "unified/0".to_string(),
                },
                "mount-plan:unified:0".to_string(),
            )
        })
        .unwrap();
        let artifact = module_artifact(&mapped).unwrap();
        assert_eq!(resolution_plan(&artifact.resolution), "mount-plan:unified:0");
    }
}
