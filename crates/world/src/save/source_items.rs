//! Source-item ownership ported from `src/persistence/source-items.ts`.
//!
//! Source providers admit items into shared inventories through
//! `world:source-items` records (provider `world:gameplay`, version 1).
//! Each record keeps the hidden primary rows plus per-owner admission
//! groups; restores verify primary/effective coverage exactly. Item icons
//! and held-weapon declarations are small content readers the donor
//! persistence calls into; they are ported here (icon shapes,
//! `normalizeResourcePath`, model-grip rotation checks) so definitions
//! round-trip without the content crates.

use std::collections::{HashMap, HashSet};

use qa_core::identity::SavedActorId;

use super::ownership::ProviderCheckpoint;
use super::records::{read_inventory_entry, read_saved_actor, write_inventory_entry, write_saved_actor};
use super::shared::{read_provider_ref, write_provider_ref, ProviderRef};
use super::value::{
    arr, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str, SaveJson, SaveReader,
};
use crate::inventory::InventoryEntry;
use crate::WorldError;

/// Schema name.
pub const SCHEMA: &str = "world:source-items";
/// Owning provider.
pub const PROVIDER: &str = "world:gameplay";

/// Normalize a relative resource path (donor `normalizeResourcePath`).
pub fn normalize_resource_path(path: &str) -> Result<String, WorldError> {
    let normalized = path.replace('\\', "/");
    let mut bytes = normalized.bytes();
    let drive =
        matches!(bytes.next(), Some(first) if first.is_ascii_alphabetic()) && matches!(bytes.next(), Some(b':'));
    if normalized.is_empty()
        || normalized.contains('\0')
        || drive
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(WorldError::BadSave(format!("Invalid relative resource path: {path}")));
    }
    Ok(normalized)
}

/// Resolved HUD icon for a source item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponHudIcon {
    /// Image resource.
    Image {
        /// Content id.
        content: String,
        /// Resource path.
        path: String,
    },
    /// WAD picture resource.
    WadPicture {
        /// Content id.
        content: String,
        /// Resource path.
        path: String,
        /// Lump name.
        lump: String,
    },
    /// Named shader.
    Shader {
        /// Content id.
        content: String,
        /// Shader name.
        name: String,
    },
}

/// Read a source item icon, requiring the expected content source.
pub fn read_source_item_icon(reader: SaveReader, content: &str) -> Result<WeaponHudIcon, WorldError> {
    let kind = reader.field("kind").choice_str(&["image", "wad-picture", "shader"])?;
    let resource = if kind == "shader" {
        reader.clone()
    } else {
        reader.field("resource")
    };
    if resource.field("content").string()? != content {
        return Err(reader.fail("Item icon belongs to another content source"));
    }
    if kind == "shader" {
        return Ok(WeaponHudIcon::Shader {
            content: content.to_string(),
            name: normalize_resource_path(&reader.field("name").string()?)?,
        });
    }
    let path = normalize_resource_path(&resource.field("path").string()?)?;
    if kind == "image" {
        return Ok(WeaponHudIcon::Image {
            content: content.to_string(),
            path,
        });
    }
    let lump = reader.field("lump").string()?;
    if lump.is_empty() || lump.len() > 16 || lump.contains('\0') {
        return Err(reader.fail("Item icon requires a valid WAD lump name"));
    }
    Ok(WeaponHudIcon::WadPicture {
        content: content.to_string(),
        path,
        lump,
    })
}

/// Write a source item icon.
#[must_use]
pub fn write_source_item_icon(icon: &WeaponHudIcon) -> SaveJson {
    match icon {
        WeaponHudIcon::Image { content, path } => obj(vec![
            ("kind", str("image")),
            ("resource", obj(vec![("content", str(content)), ("path", str(path))])),
        ]),
        WeaponHudIcon::WadPicture { content, path, lump } => obj(vec![
            ("kind", str("wad-picture")),
            ("resource", obj(vec![("content", str(content)), ("path", str(path))])),
            ("lump", str(lump)),
        ]),
        WeaponHudIcon::Shader { content, name } => obj(vec![
            ("kind", str("shader")),
            ("content", str(content)),
            ("name", str(name)),
        ]),
    }
}

/// Model grip transform (origin, rotation axes, invertible scale).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelGrip {
    /// Origin.
    pub origin: (f64, f64, f64),
    /// Rotation axes.
    pub axis: [(f64, f64, f64); 3],
    /// Scale.
    pub scale: (f64, f64, f64),
}

fn read_triple(reader: SaveReader) -> Result<(f64, f64, f64), WorldError> {
    Ok((
        reader.field("x").finite()?,
        reader.field("y").finite()?,
        reader.field("z").finite()?,
    ))
}

fn write_triple(value: (f64, f64, f64)) -> SaveJson {
    obj(vec![("x", num(value.0)), ("y", num(value.1)), ("z", num(value.2))])
}

fn dot(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}

fn cross(a: (f64, f64, f64), b: (f64, f64, f64)) -> (f64, f64, f64) {
    (a.1 * b.2 - a.2 * b.1, a.2 * b.0 - a.0 * b.2, a.0 * b.1 - a.1 * b.0)
}

/// Read a model grip, validating the rotation axes.
pub fn read_model_grip(reader: SaveReader) -> Result<ModelGrip, WorldError> {
    let axes = reader.field("axis").list(read_triple)?;
    if axes.len() != 3 {
        return Err(reader.fail("Model grip requires three source axes"));
    }
    let (first, second, third) = (axes[0], axes[1], axes[2]);
    if axes.iter().any(|axis| (dot(*axis, *axis) - 1.0).abs() > 0.001)
        || dot(first, second).abs() > 0.001
        || dot(first, third).abs() > 0.001
        || dot(second, third).abs() > 0.001
        || dot(cross(first, second), third) < 0.999
    {
        return Err(reader.fail("Model grip axes must form a rotation"));
    }
    let scale = if reader.field("scale").is_missing() {
        (1.0, 1.0, 1.0)
    } else {
        read_triple(reader.field("scale"))?
    };
    if scale.0 == 0.0 || scale.1 == 0.0 || scale.2 == 0.0 {
        return Err(reader.fail("Model grip scale must be invertible"));
    }
    Ok(ModelGrip {
        origin: read_triple(reader.field("origin"))?,
        axis: [first, second, third],
        scale,
    })
}

/// Write a model grip.
#[must_use]
pub fn write_model_grip(grip: &ModelGrip) -> SaveJson {
    obj(vec![
        ("origin", write_triple(grip.origin)),
        ("axis", arr(grip.axis.iter().map(|axis| write_triple(*axis)).collect())),
        ("scale", write_triple(grip.scale)),
    ])
}

/// Held-weapon view-model declaration.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum HeldWeapon {
    /// No view model.
    None,
    /// Model view.
    Model {
        /// Model path.
        path: String,
        /// Reference frame.
        reference_frame: i64,
        /// Grip transform.
        grip: ModelGrip,
        /// Model digest, when pinned.
        digest: Option<String>,
        /// Fallback model path.
        fallback: Option<String>,
        /// Mesh subset.
        part: Option<HeldWeaponPart>,
    },
}

/// Held-model mesh subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldWeaponPart {
    /// Source digests.
    pub digests: Vec<String>,
    /// Vertices.
    pub vertices: Vec<i64>,
}

/// Read a held-weapon declaration.
pub fn read_held_weapon(reader: SaveReader) -> Result<HeldWeapon, WorldError> {
    if reader.field("kind").choice_str(&["none", "model"])? == "none" {
        return Ok(HeldWeapon::None);
    }
    let model = reader.field("model");
    let part = model.field("part");
    let read_digest = |value: SaveReader| {
        let parsed = value.string()?;
        super::shared::validate_digest(&parsed).map_err(|_| value.fail("held model requires a SHA256 digest"))?;
        Ok(parsed)
    };
    let subset = if part.is_missing() {
        None
    } else {
        let subset = HeldWeaponPart {
            digests: part.field("digests").list(read_digest)?,
            vertices: part.field("vertices").list(|item| item.integer(0))?,
        };
        if subset.digests.is_empty()
            || subset.vertices.is_empty()
            || subset.vertices.iter().collect::<HashSet<_>>().len() != subset.vertices.len()
        {
            return Err(part.fail("held model subset requires source digests and distinct vertices"));
        }
        Some(subset)
    };
    Ok(HeldWeapon::Model {
        path: normalize_resource_path(&model.field("path").string()?)?,
        reference_frame: model.field("referenceFrame").integer(0)?,
        grip: read_model_grip(model.field("grip"))?,
        digest: if model.field("digest").is_missing() {
            None
        } else {
            Some(read_digest(model.field("digest"))?)
        },
        fallback: if model.field("fallback").is_missing() {
            None
        } else {
            Some(normalize_resource_path(&model.field("fallback").string()?)?)
        },
        part: subset,
    })
}

/// Write a held-weapon declaration.
#[must_use]
pub fn write_held_weapon(held: &HeldWeapon) -> SaveJson {
    match held {
        HeldWeapon::None => obj(vec![("kind", str("none"))]),
        HeldWeapon::Model {
            path,
            reference_frame,
            grip,
            digest,
            fallback,
            part,
        } => {
            let mut model_members = vec![
                ("path", str(path)),
                ("referenceFrame", int(*reference_frame)),
                ("grip", write_model_grip(grip)),
            ];
            if let Some(digest) = digest {
                model_members.push(("digest", str(digest)));
            }
            if let Some(fallback) = fallback {
                model_members.push(("fallback", str(fallback)));
            }
            if let Some(part) = part {
                model_members.push((
                    "part",
                    obj(vec![
                        ("digests", arr(part.digests.iter().map(|digest| str(digest)).collect())),
                        (
                            "vertices",
                            arr(part.vertices.iter().map(|vertex| int(*vertex)).collect()),
                        ),
                    ]),
                ));
            }
            obj(vec![("kind", str("model")), ("model", obj(model_members))])
        }
    }
}

/// Source item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItemDefinition {
    /// Item id.
    pub item: String,
    /// Display label.
    pub label: String,
    /// Source provider reference.
    pub source: ProviderRef,
    /// HUD icon.
    pub icon: Option<WeaponHudIcon>,
    /// Allowed actions.
    pub actions: Option<Vec<String>>,
    /// Weapon extras (`None` for counters).
    pub weapon: Option<WeaponDefinition>,
}

/// Weapon-only definition extras.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponDefinition {
    /// Held view.
    pub held: Option<HeldWeapon>,
    /// Ammo item (null for none).
    pub ammo: Option<String>,
}

/// Read a source item definition.
pub fn read_definition(reader: SaveReader) -> Result<SourceItemDefinition, WorldError> {
    let source = read_provider_ref(reader.field("source"))?;
    let label = reader.field("label").string()?;
    if label.is_empty() {
        return Err(reader.fail("source item label is empty"));
    }
    let icon = if reader.field("icon").is_missing() {
        None
    } else {
        reader
            .field("icon")
            .nullable(|icon| read_source_item_icon(icon, &source.content))?
    };
    let actions = if reader.field("actions").is_missing() {
        None
    } else {
        let actions = reader.field("actions").list(|item| item.choice_str(&["use", "drop"]))?;
        if actions.is_empty() || actions.iter().collect::<HashSet<_>>().len() != actions.len() {
            return Err(reader.fail("Source item actions are empty or duplicated"));
        }
        Some(actions)
    };
    let kind = reader.field("kind").choice_str(&["counter", "weapon"])?;
    let weapon = if kind == "counter" {
        None
    } else {
        Some(WeaponDefinition {
            held: if reader.field("held").is_missing() {
                None
            } else {
                Some(read_held_weapon(reader.field("held"))?)
            },
            ammo: if reader.field("ammo").value == Some(&SaveJson::Null) {
                None
            } else {
                Some(namespaced(reader.field("ammo"))?)
            },
        })
    };
    Ok(SourceItemDefinition {
        item: namespaced(reader.field("item"))?,
        label,
        source,
        icon,
        actions,
        weapon,
    })
}

/// Write a source item definition.
#[must_use]
pub fn write_definition(definition: &SourceItemDefinition) -> SaveJson {
    let mut members = vec![
        ("item", str(&definition.item)),
        ("label", str(&definition.label)),
        ("source", write_provider_ref(&definition.source)),
    ];
    if let Some(icon) = &definition.icon {
        members.push(("icon", write_source_item_icon(icon)));
    }
    if let Some(actions) = &definition.actions {
        members.push(("actions", arr(actions.iter().map(|action| str(action)).collect())));
    }
    match &definition.weapon {
        None => members.push(("kind", str("counter"))),
        Some(weapon) => {
            members.push(("kind", str("weapon")));
            if let Some(held) = &weapon.held {
                members.push(("held", write_held_weapon(held)));
            }
            members.push(("ammo", weapon.ammo.as_ref().map_or(SaveJson::Null, |ammo| str(ammo))));
        }
    }
    obj(members)
}

/// Admission of one source item into shared inventory.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItemAdmission {
    /// Definition.
    pub definition: SourceItemDefinition,
    /// Admission mode.
    pub admission: SourceAdmission,
}

/// Admission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAdmission {
    /// Added alongside primary rows.
    Add,
    /// Replaces a primary row.
    ReplacePrimary,
}

/// One owner's admission group.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItemGroup {
    /// Owning provider.
    pub owner: String,
    /// Admissions.
    pub items: Vec<SourceItemAdmission>,
}

/// Hidden source-item record for one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItemsRecord {
    /// Actor.
    pub actor: SavedActorId,
    /// Actor owner.
    pub owner: String,
    /// Hidden primary rows.
    pub primary: Vec<InventoryEntry>,
    /// Admission groups.
    pub groups: Vec<SourceItemGroup>,
}

fn actor_key(actor: SavedActorId) -> (u32, u32) {
    (actor.slot, actor.generation)
}

/// Capture source items into a provider record.
#[must_use]
pub fn capture_source_items(records: &[SourceItemsRecord]) -> ProviderCheckpoint {
    ProviderCheckpoint {
        provider: PROVIDER.to_string(),
        schema: SCHEMA.to_string(),
        version: 1,
        bytes: encode_checkpoint_value(&write_records(records)),
    }
}

fn write_records(records: &[SourceItemsRecord]) -> SaveJson {
    arr(records
        .iter()
        .map(|record| {
            obj(vec![
                ("actor", write_saved_actor(record.actor)),
                ("owner", str(&record.owner)),
                (
                    "primary",
                    arr(record.primary.iter().map(write_inventory_entry).collect()),
                ),
                (
                    "groups",
                    arr(record
                        .groups
                        .iter()
                        .map(|group| {
                            obj(vec![
                                ("owner", str(&group.owner)),
                                (
                                    "items",
                                    arr(group
                                        .items
                                        .iter()
                                        .map(|item| {
                                            obj(vec![
                                                ("definition", write_definition(&item.definition)),
                                                (
                                                    "admission",
                                                    str(match item.admission {
                                                        SourceAdmission::Add => "add",
                                                        SourceAdmission::ReplacePrimary => "replace-primary",
                                                    }),
                                                ),
                                            ])
                                        })
                                        .collect()),
                                ),
                            ])
                        })
                        .collect()),
                ),
            ])
        })
        .collect())
}

/// Effective inventories keyed by actor for cross-checks.
pub type EffectiveInventories = Vec<(SavedActorId, Vec<InventoryEntry>)>;

/// Read source items, verifying primary/effective coverage.
pub fn read_source_items(
    providers: &[ProviderCheckpoint],
    inventories: &EffectiveInventories,
) -> Result<Vec<SourceItemsRecord>, WorldError> {
    let records: Vec<&ProviderCheckpoint> = providers.iter().filter(|record| record.schema == SCHEMA).collect();
    if records.len() > 1 {
        return Err(WorldError::BadSave(format!(
            "{SCHEMA}: unsupported or duplicate source item checkpoint"
        )));
    }
    let Some(record) = records.first() else {
        return Ok(Vec::new());
    };
    if record.provider != PROVIDER || record.version != 1 {
        return Err(WorldError::BadSave(format!(
            "{SCHEMA}: unsupported or duplicate source item checkpoint"
        )));
    }
    let payload = decode_checkpoint_value(&record.bytes)
        .map_err(|_| WorldError::BadSave(format!("{SCHEMA}: unsupported or duplicate source item checkpoint")))?;
    let mut actors = HashSet::new();
    SaveReader::at(&payload, SCHEMA).list(|reader| {
        let actor = read_saved_actor(reader.field("actor"))?;
        let owner = namespaced(reader.field("owner"))?;
        let primary = reader.field("primary").list(read_inventory_entry)?;
        let primary_items: HashSet<&str> = primary.iter().map(|entry| entry.item.as_str()).collect();
        if !actors.insert(actor_key(actor)) || primary_items.len() != primary.len() {
            return Err(reader.fail("duplicate primary inventory or actor"));
        }
        let mut items = HashSet::new();
        let groups = reader.field("groups").list(|group| {
            let provider = namespaced(group.field("owner"))?;
            let admissions = group.field("items").list(|entry| {
                let item = read_definition(entry.field("definition"))?;
                let admission = entry.field("admission").choice_str(&["add", "replace-primary"])?;
                let admission = if admission == "add" {
                    SourceAdmission::Add
                } else {
                    SourceAdmission::ReplacePrimary
                };
                let owned = match admission {
                    SourceAdmission::Add => !primary_items.contains(item.item.as_str()),
                    SourceAdmission::ReplacePrimary => primary_items.contains(item.item.as_str()),
                };
                if item.source.provider != provider || items.contains(item.item.as_str()) || !owned {
                    return Err(entry.fail("source item admission differs from primary ownership"));
                }
                items.insert(item.item.clone());
                Ok(SourceItemAdmission {
                    definition: item,
                    admission,
                })
            })?;
            if admissions.is_empty() {
                return Err(group.fail("empty source item group"));
            }
            Ok(SourceItemGroup {
                owner: provider,
                items: admissions,
            })
        })?;
        let effective: Vec<&Vec<InventoryEntry>> = inventories
            .iter()
            .filter(|(actor_id, _)| actor_key(*actor_id) == actor_key(actor))
            .map(|(_, entries)| entries)
            .collect();
        if groups.is_empty() || effective.len() != 1 {
            return Err(reader.fail("source items have no distinct effective inventory"));
        }
        let rows = effective[0];
        let row_items: HashSet<&str> = rows.iter().map(|entry| entry.item.as_str()).collect();
        let item_refs: HashSet<&str> = items.iter().map(String::as_str).collect();
        let primary_rest: Vec<&InventoryEntry> = primary
            .iter()
            .filter(|entry| !item_refs.contains(entry.item.as_str()))
            .collect();
        let rows_rest: Vec<&InventoryEntry> = rows
            .iter()
            .filter(|entry| !item_refs.contains(entry.item.as_str()))
            .collect();
        if row_items.len() != rows.len()
            || item_refs
                .iter()
                .any(|item| !rows.iter().any(|entry| entry.item == *item))
            || rows_rest != primary_rest
        {
            return Err(reader.fail("source item coverage differs from effective inventory"));
        }
        Ok(SourceItemsRecord {
            actor,
            owner,
            primary,
            groups,
        })
    })
}

/// Map saved inventories to their hidden primary rows for provider restore.
#[must_use]
pub fn primary_inventories(records: &[SourceItemsRecord], inventories: &EffectiveInventories) -> EffectiveInventories {
    if records.is_empty() {
        return inventories.clone();
    }
    let by_actor: HashMap<(u32, u32), &SourceItemsRecord> =
        records.iter().map(|record| (actor_key(record.actor), record)).collect();
    inventories
        .iter()
        .map(|(actor, entries)| {
            (
                *actor,
                by_actor
                    .get(&actor_key(*actor))
                    .map_or_else(|| entries.clone(), |record| record.primary.clone()),
            )
        })
        .collect()
}

/// Map restored primary rows back to committed effective rows.
pub fn effective_inventories(
    records: &[SourceItemsRecord],
    saved: &EffectiveInventories,
    restored: &EffectiveInventories,
) -> Result<EffectiveInventories, WorldError> {
    if records.is_empty() {
        return Ok(restored.clone());
    }
    let by_actor: HashSet<(u32, u32)> = records.iter().map(|record| actor_key(record.actor)).collect();
    restored
        .iter()
        .map(|(actor, entries)| {
            if !by_actor.contains(&actor_key(*actor)) {
                return Ok((*actor, entries.clone()));
            }
            let original = saved
                .iter()
                .find(|(saved_actor, _)| actor_key(*saved_actor) == actor_key(*actor));
            match original {
                Some((_, rows)) => Ok((*actor, rows.clone())),
                None => Err(WorldError::BadSave(format!(
                    "{SCHEMA}: missing saved effective inventory"
                ))),
            }
        })
        .collect()
}

/// Restored view for finish validation.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoredSourceView {
    /// Actor.
    pub actor: SavedActorId,
    /// Actor owner.
    pub owner: String,
    /// Restored hidden state (primary rows plus groups).
    pub source: Option<(Vec<InventoryEntry>, Vec<SourceItemGroup>)>,
    /// Committed entries.
    pub entries: Vec<InventoryEntry>,
}

/// Validate restored source-item ownership against the saved records.
pub fn check_source_item_restore(
    records: &[SourceItemsRecord],
    saved: &EffectiveInventories,
    restored: &[RestoredSourceView],
) -> Result<(), WorldError> {
    let mut done = HashSet::new();
    for view in restored {
        let expected = records
            .iter()
            .find(|record| actor_key(record.actor) == actor_key(view.actor));
        match (&view.source, expected) {
            (None, None) => {}
            (Some((primary, groups)), Some(record))
                if record.owner == view.owner && &record.primary == primary && &record.groups == groups => {}
            _ => {
                return Err(WorldError::BadSave(format!(
                    "{SCHEMA}: restored source item ownership or hidden inventory differs"
                )));
            }
        }
        if let Some(record) = expected {
            let effective = saved
                .iter()
                .find(|(actor, _)| actor_key(*actor) == actor_key(record.actor));
            match effective {
                Some((_, rows)) if rows == &view.entries => {}
                _ => {
                    return Err(WorldError::BadSave(format!(
                        "{SCHEMA}: restored source inventory differs from its committed state"
                    )));
                }
            }
            done.insert(actor_key(record.actor));
        }
    }
    if done.len() != records.len() {
        return Err(WorldError::BadSave(format!(
            "{SCHEMA}: saved source item actor was not restored"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::ItemId;

    fn entry(item: &str, count: f64) -> InventoryEntry {
        InventoryEntry {
            item: ItemId::from(item),
            count,
            capacity: 100.0,
            count_policy: None,
        }
    }

    fn sample_record() -> SourceItemsRecord {
        SourceItemsRecord {
            actor: SavedActorId { slot: 1, generation: 0 },
            owner: "q2:game".to_string(),
            primary: vec![entry("q2:shells", 10.0)],
            groups: vec![SourceItemGroup {
                owner: "q2:rogue".to_string(),
                items: vec![SourceItemAdmission {
                    definition: SourceItemDefinition {
                        item: "q2:tracker".to_string(),
                        label: "Tracker".to_string(),
                        source: ProviderRef {
                            provider: "q2:rogue".to_string(),
                            content: "q2:classic:base:1".to_string(),
                        },
                        icon: None,
                        actions: Some(vec!["use".to_string()]),
                        weapon: Some(WeaponDefinition {
                            held: None,
                            ammo: Some("q2:cells".to_string()),
                        }),
                    },
                    admission: SourceAdmission::Add,
                }],
            }],
        }
    }

    #[test]
    fn records_round_trip_with_coverage_checks() {
        let record = sample_record();
        let checkpoint = capture_source_items(std::slice::from_ref(&record));
        let saved = vec![(record.actor, vec![entry("q2:shells", 10.0), entry("q2:tracker", 1.0)])];
        let read = read_source_items(std::slice::from_ref(&checkpoint), &saved).unwrap();
        assert_eq!(read, vec![record.clone()]);
        let primary = primary_inventories(&read, &saved);
        assert_eq!(primary[0].1, vec![entry("q2:shells", 10.0)]);
        let restored = vec![RestoredSourceView {
            actor: record.actor,
            owner: "q2:game".to_string(),
            source: Some((record.primary.clone(), record.groups.clone())),
            entries: vec![entry("q2:shells", 10.0), entry("q2:tracker", 1.0)],
        }];
        assert!(check_source_item_restore(&read, &saved, &restored).is_ok());
        let effective = effective_inventories(&read, &saved, &primary).unwrap();
        assert_eq!(effective, saved);
    }

    #[test]
    fn admission_mismatches_fail() {
        let mut record = sample_record();
        record.groups[0].items[0].admission = SourceAdmission::ReplacePrimary;
        let checkpoint = capture_source_items(std::slice::from_ref(&record));
        let saved = vec![(record.actor, vec![entry("q2:shells", 10.0), entry("q2:tracker", 1.0)])];
        assert!(read_source_items(std::slice::from_ref(&checkpoint), &saved).is_err());
    }

    #[test]
    fn paths_grips_icons_validate() {
        assert_eq!(normalize_resource_path("a/b").unwrap(), "a/b");
        assert_eq!(normalize_resource_path("a\\b").unwrap(), "a/b");
        assert!(normalize_resource_path("../a").is_err());
        assert!(normalize_resource_path("C:/a").is_err());
        assert!(normalize_resource_path("").is_err());
        let grip = ModelGrip {
            origin: (0.0, 0.0, 0.0),
            axis: [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)],
            scale: (1.0, 1.0, 1.0),
        };
        let json = write_model_grip(&grip);
        assert_eq!(read_model_grip(SaveReader::new(&json)).unwrap(), grip);
        let icon = WeaponHudIcon::WadPicture {
            content: "q1:classic:id1:1".to_string(),
            path: "gfx/icons".to_string(),
            lump: "icon".to_string(),
        };
        let json = write_source_item_icon(&icon);
        assert_eq!(
            read_source_item_icon(SaveReader::new(&json), "q1:classic:id1:1").unwrap(),
            icon
        );
        assert!(read_source_item_icon(SaveReader::new(&json), "q2:classic:base:1").is_err());
    }
}
