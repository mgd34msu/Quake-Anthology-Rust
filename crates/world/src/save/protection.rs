//! Hidden primary-protection checkpoints ported from `src/persistence/protection.ts`.
//!
//! When a copied combat owner hides primary armor behind component
//! protection, the hidden layers are retained in a `world:primary-protection`
//! record (provider `world:gameplay`, version 1) so component restoration can
//! verify coverage.
//!
//! Capture runs over caller-supplied views; the host must be idle like the
//! donor's `combat.assertIdle()`.

use std::collections::HashMap;

use qa_core::identity::SavedActorId;

use super::ownership::ProviderCheckpoint;
use super::records::{read_powered_protection, read_regular_armor, read_saved_actor, write_armor, write_saved_actor};
use super::value::{arr, decode_checkpoint_value, encode_checkpoint_value, namespaced, obj, str, SaveJson, SaveReader};
use crate::combat::{ArmorState, PoweredProtection, RegularArmor};
use crate::WorldError;

/// Schema name.
pub const SCHEMA: &str = "world:primary-protection";
/// Owning provider.
pub const PROVIDER: &str = "world:gameplay";

/// One hidden armor entry.
#[derive(Debug, Clone, PartialEq)]
pub struct HiddenArmorEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Owning provider.
    pub owner: String,
    /// Hidden regular layer, when component-owned.
    pub regular: Option<RegularArmor>,
    /// Hidden powered layer, when component-owned.
    pub powered: Option<PoweredProtection>,
}

/// Capture view for one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectionCapture {
    /// Actor.
    pub actor: SavedActorId,
    /// Owning provider.
    pub owner: String,
    /// Copied primary armor (hosts without hidden primaries pass `None`).
    pub primary: Option<ArmorState>,
    /// Whether component protection owns the regular channel.
    pub regular_owned: bool,
    /// Whether component protection owns the powered channel.
    pub powered_owned: bool,
}

/// Capture hidden primary protection into a provider record.
#[must_use]
pub fn capture_primary_protection(views: &[ProtectionCapture]) -> ProviderCheckpoint {
    let mut entries = Vec::new();
    for view in views {
        let Some(primary) = &view.primary else { continue };
        if !view.regular_owned && !view.powered_owned {
            continue;
        }
        entries.push(HiddenArmorEntry {
            actor: view.actor,
            owner: view.owner.clone(),
            regular: view.regular_owned.then(|| primary.regular.clone()),
            powered: view.powered_owned.then(|| primary.powered.clone()),
        });
    }
    ProviderCheckpoint {
        provider: PROVIDER.to_string(),
        schema: SCHEMA.to_string(),
        version: 1,
        bytes: encode_checkpoint_value(&write_entries(&entries)),
    }
}

fn write_entries(entries: &[HiddenArmorEntry]) -> SaveJson {
    arr(entries
        .iter()
        .map(|entry| {
            let armor = write_armor(&ArmorState {
                regular: entry.regular.clone().unwrap_or(RegularArmor::None),
                powered: entry.powered.clone().unwrap_or(PoweredProtection::None),
            });
            let mut members = vec![("actor", write_saved_actor(entry.actor)), ("owner", str(&entry.owner))];
            if entry.regular.is_some() {
                members.push(("regular", armor.get("regular").expect("written regular").clone()));
            }
            if entry.powered.is_some() {
                members.push(("powered", armor.get("powered").expect("written powered").clone()));
            }
            obj(members)
        })
        .collect())
}

/// Read result: whether a record existed plus entries keyed by actor slot.
#[derive(Debug, Clone, PartialEq)]
pub struct PrimaryProtection {
    /// Whether the save recorded the schema.
    pub recorded: bool,
    /// Entries by `(slot, generation)`.
    pub entries: HashMap<(u32, u32), HiddenArmorEntry>,
}

/// Owner lookup for restore validation.
pub struct ProtectionOwner {
    /// Actor owner (`namespace:name`).
    pub owner: String,
}

/// Read hidden primary protection, validating owners and coverage.
pub fn read_primary_protection(
    providers: &[ProviderCheckpoint],
    resolve: &dyn Fn(SavedActorId) -> Option<ProtectionOwner>,
) -> Result<PrimaryProtection, WorldError> {
    let records: Vec<&ProviderCheckpoint> = providers.iter().filter(|record| record.schema == SCHEMA).collect();
    if records.len() > 1 {
        return Err(WorldError::BadSave(format!(
            "{SCHEMA}: duplicate primary protection checkpoint"
        )));
    }
    let mut entries = HashMap::new();
    let Some(record) = records.first() else {
        return Ok(PrimaryProtection {
            recorded: false,
            entries,
        });
    };
    if record.provider != PROVIDER || record.version != 1 {
        return Err(WorldError::BadSave(format!(
            "{SCHEMA}: unsupported primary protection checkpoint"
        )));
    }
    let payload = decode_checkpoint_value(&record.bytes)
        .map_err(|_| WorldError::BadSave(format!("{SCHEMA}: unsupported primary protection checkpoint")))?;
    SaveReader::at(&payload, SCHEMA).list(|value| {
        let saved = read_saved_actor(value.field("actor"))?;
        let owner = namespaced(value.field("owner"))?;
        let resolved = resolve(saved);
        match resolved {
            Some(resolved) if resolved.owner == owner && !entries.contains_key(&(saved.slot, saved.generation)) => {}
            _ => return Err(value.fail("missing or duplicate primary protection owner")),
        }
        let regular = value.field("regular");
        let powered = value.field("powered");
        if regular.is_missing() && powered.is_missing() {
            return Err(value.fail("empty hidden primary armor"));
        }
        let armor = ArmorState {
            regular: if regular.is_missing() {
                RegularArmor::None
            } else {
                read_regular_armor(regular.clone())?
            },
            powered: if powered.is_missing() {
                PoweredProtection::None
            } else {
                read_powered_protection(powered.clone())?
            },
        };
        if matches!(armor.regular, RegularArmor::Source { .. }) {
            return Err(regular.fail("copied primary armor cannot own a source formula"));
        }
        entries.insert(
            (saved.slot, saved.generation),
            HiddenArmorEntry {
                actor: saved,
                owner,
                regular: if regular.is_missing() {
                    None
                } else {
                    Some(armor.regular)
                },
                powered: if powered.is_missing() {
                    None
                } else {
                    Some(armor.powered)
                },
            },
        );
        Ok(())
    })?;
    Ok(PrimaryProtection {
        recorded: true,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::ArmorState;

    fn armor() -> ArmorState {
        ArmorState {
            regular: RegularArmor::Q3 {
                points: 25.0,
                protection: 0.5,
            },
            powered: PoweredProtection::Shield { cells: 4 },
        }
    }

    #[test]
    fn capture_and_read_round_trip() {
        let actor = SavedActorId { slot: 1, generation: 0 };
        let record = capture_primary_protection(&[
            ProtectionCapture {
                actor,
                owner: "q3:game".to_string(),
                primary: Some(armor()),
                regular_owned: true,
                powered_owned: true,
            },
            ProtectionCapture {
                actor: SavedActorId { slot: 2, generation: 0 },
                owner: "q3:game".to_string(),
                primary: None,
                regular_owned: false,
                powered_owned: false,
            },
        ]);
        assert_eq!(record.provider, PROVIDER);
        let read = read_primary_protection(&[record], &|saved| {
            (saved == actor).then(|| ProtectionOwner {
                owner: "q3:game".to_string(),
            })
        })
        .unwrap();
        assert!(read.recorded);
        assert_eq!(read.entries.len(), 1);
        assert_eq!(read.entries[&(1, 0)].regular, Some(armor().regular));
        let empty = read_primary_protection(&[], &|_| None).unwrap();
        assert!(!empty.recorded);
    }

    #[test]
    fn validation_rejects_bad_records() {
        let actor = SavedActorId { slot: 1, generation: 0 };
        let record = capture_primary_protection(&[ProtectionCapture {
            actor,
            owner: "q3:game".to_string(),
            primary: Some(armor()),
            regular_owned: true,
            powered_owned: false,
        }]);
        // Duplicate schema records.
        assert!(read_primary_protection(&[record.clone(), record.clone()], &|_| None).is_err());
        // Owner mismatch.
        assert!(
            read_primary_protection(std::slice::from_ref(&record), &|_| Some(ProtectionOwner {
                owner: "q2:game".to_string()
            }))
            .is_err()
        );
        // Source-formula regular layer.
        let source = ProtectionCapture {
            actor,
            owner: "q3:game".to_string(),
            primary: Some(ArmorState {
                regular: RegularArmor::Source {
                    points: 1.0,
                    item: None,
                },
                powered: PoweredProtection::None,
            }),
            regular_owned: true,
            powered_owned: false,
        };
        let record = capture_primary_protection(&[source]);
        assert!(read_primary_protection(&[record], &|_| Some(ProtectionOwner {
            owner: "q3:game".to_string()
        }))
        .is_err());
    }
}
