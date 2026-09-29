//! Q2 rerelease primary pickup profile for the retail artifact.
//!
//! Donor: `src/compat/q2/rerelease/pickup-profile.ts` — bridges the
//! artifact-qualified pickup touch/grant regions into a supply profile.

use qa_guest::core::contracts::{GuestCallSignature, GuestStorage, GuestValueLayout, NativeCallAbi};
use qa_world::WorldError;
use qa_world::inventory::InventoryEntry;
use qa_world::pickups::{PickupGrantPlan, PickupSupplyPreview, preview_pickup_grants};
use thiserror::Error;

use super::layouts::{edict_layout, field_offset, private_edict_prefix_layout};

/// Retail artifact digest admitting the pickup profile.
pub const RETAIL_ARTIFACT_DIGEST: &str =
    "sha256:045d49c53722d9b922caf14f168dd28a97d4c514a6e443a3140560f8668baccd";

/// Pickup profile failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PickupProfileError {
    /// Profile belongs to another artifact.
    #[error("Rerelease pickup profile belongs to another artifact")]
    ForeignArtifact,
    /// Pickup preview failure.
    #[error("Pickup preview rejected: {0}")]
    Preview(String),
}

impl From<WorldError> for PickupProfileError {
    fn from(error: WorldError) -> Self {
        Self::Preview(error.to_string())
    }
}

fn scalar(storage: GuestStorage) -> GuestValueLayout {
    GuestValueLayout::Scalar(storage)
}

fn signature(parameters: Vec<GuestValueLayout>, result: Option<GuestValueLayout>) -> GuestCallSignature {
    GuestCallSignature {
        abi: NativeCallAbi::MicrosoftX64,
        parameters,
        result,
        variadic: false,
    }
}

/// Touch signature: self, other, activator, pickup-kind byte.
#[must_use]
pub fn touch_signature() -> GuestCallSignature {
    signature(
        vec![
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Uint8),
        ],
        None,
    )
}

/// Grant signature: recipient and item, boolean result.
#[must_use]
pub fn grant_signature() -> GuestCallSignature {
    signature(
        vec![
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Pointer),
        ],
        Some(scalar(GuestStorage::Uint8)),
    )
}

/// Ammunition supply check signature.
#[must_use]
pub fn ammo_signature() -> GuestCallSignature {
    signature(
        vec![
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Pointer),
            scalar(GuestStorage::Int32),
        ],
        Some(scalar(GuestStorage::Uint8)),
    )
}

/// Native region: entry and join RVAs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeRegion {
    /// Entry RVA.
    pub entry: u64,
    /// Join RVA.
    pub join: u64,
}

/// Powered consumer check inside a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantConsumer {
    /// Check entry RVA.
    pub entry: u64,
    /// Protection channel: regular or powered.
    pub protection: GrantProtection,
}

/// Grant protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantProtection {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Ammo supply probe inside a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmmoSupply {
    /// Probe entry RVA.
    pub entry: u64,
    /// Amount register lane.
    pub amount_register: &'static str,
}

/// Weapon supply settlement inside a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponSupply {
    /// Ammunition return RVA.
    pub ammo_return: u64,
    /// Settlement RVA.
    pub settle: u64,
    /// Autoswitch region.
    pub autoswitch: NativeRegion,
}

/// Supply probe inside a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantSupply {
    /// Ammunition probe.
    Ammo(AmmoSupply),
    /// Weapon probe.
    Weapon(WeaponSupply),
}

/// One grant site with its recipient region and resource kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupGrant {
    /// Grant entry RVA.
    pub entry: u64,
    /// Recipient region.
    pub recipient: NativeRegion,
    /// Resource kind.
    pub resource: GrantResource,
    /// Consumer checks.
    pub consumers: Vec<GrantConsumer>,
    /// Supply probe, if any.
    pub supply: Option<GrantSupply>,
}

/// Grant resource kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantResource {
    /// Regular armor grant.
    Regular,
    /// Inventory grant.
    Inventory,
}

/// Item table geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemTable {
    /// Table RVA.
    pub table: u64,
    /// Record stride.
    pub stride: usize,
    /// Record count.
    pub count: usize,
    /// Classname field offset.
    pub classname: usize,
    /// Pickup callback offset.
    pub pickup: usize,
}

/// Entity field offsets for pickup bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntityFields {
    /// Item pointer offset.
    pub item: usize,
    /// Count offset.
    pub count: usize,
    /// Spawnflags offset.
    pub spawnflags: usize,
    /// In-use offset.
    pub inuse: usize,
    /// In-use width in bytes.
    pub inuse_bytes: usize,
    /// Generation counter offset.
    pub generation: usize,
}

/// Level time source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSource {
    /// Time address RVA.
    pub address: u64,
    /// Storage: int64 milliseconds.
    pub storage: &'static str,
}

/// Ammunition supply profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmmoProfile {
    /// Check entry RVA.
    pub entry: u64,
    /// Stop RVA.
    pub stop: u64,
    /// Tag offset.
    pub tag: usize,
    /// Per-tag capacity offsets.
    pub capacities: Vec<usize>,
    /// Capacity width in bytes.
    pub capacity_bytes: usize,
}

/// Client supply profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyProfile {
    /// Client pointer offset.
    pub client: usize,
    /// Inventory offset.
    pub inventory: usize,
    /// Flags offset.
    pub flags: usize,
    /// Weapon flag bit.
    pub weapon_flag: u32,
    /// Ammunition profile.
    pub ammo: AmmoProfile,
}

/// Retail pickup profile: touch/grant regions retain the native register
/// saves, dropped checks and respawn calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleasePickupProfile {
    /// Admitted artifact digest.
    pub digest: &'static str,
    /// Touch entry RVA.
    pub touch: u64,
    /// Grant return RVA.
    pub grant_return: u64,
    /// Targets return RVA.
    pub targets_return: u64,
    /// Grant sites.
    pub grants: Vec<PickupGrant>,
    /// Item table.
    pub items: ItemTable,
    /// Entity fields.
    pub entity: PickupEntityFields,
    /// Time source.
    pub time: TimeSource,
    /// Supply profile.
    pub supply: SupplyProfile,
}

/// Build the retail pickup profile for a module digest, if admitted.
#[must_use]
pub fn rerelease_pickup_profile(digest: &str) -> Option<RereleasePickupProfile> {
    if digest != RETAIL_ARTIFACT_DIGEST {
        return None;
    }
    let consumers = vec![GrantConsumer {
        entry: 0x666c0,
        protection: GrantProtection::Powered,
    }];
    let edict = edict_layout();
    let prefix = private_edict_prefix_layout();
    let inuse = field_offset(&edict, "inuse").unwrap_or(1376);
    let generation = field_offset(&prefix, "spawn_count").unwrap_or(1472);
    Some(RereleasePickupProfile {
        digest: RETAIL_ARTIFACT_DIGEST,
        touch: 0x67be0,
        grant_return: 0x67c95,
        targets_return: 0x67f27,
        grants: vec![
            PickupGrant {
                entry: 0x67740,
                recipient: NativeRegion {
                    entry: 0x677d4,
                    join: 0x678e8,
                },
                resource: GrantResource::Regular,
                consumers: Vec::new(),
                supply: None,
            },
            PickupGrant {
                entry: 0x671e0,
                recipient: NativeRegion {
                    entry: 0x67209,
                    join: 0x67357,
                },
                resource: GrantResource::Inventory,
                consumers: consumers.clone(),
                supply: Some(GrantSupply::Ammo(AmmoSupply {
                    entry: 0x67250,
                    amount_register: "rcx",
                })),
            },
            PickupGrant {
                entry: 0xefd80,
                recipient: NativeRegion {
                    entry: 0xefe19,
                    join: 0xefe31,
                },
                resource: GrantResource::Inventory,
                consumers: consumers.clone(),
                supply: Some(GrantSupply::Weapon(WeaponSupply {
                    ammo_return: 0xefeac,
                    settle: 0xefeb3,
                    autoswitch: NativeRegion {
                        entry: 0xeff20,
                        join: 0xeff33,
                    },
                })),
            },
            PickupGrant {
                entry: 0x667b0,
                recipient: NativeRegion {
                    entry: 0x667c4,
                    join: 0x66918,
                },
                resource: GrantResource::Inventory,
                consumers: consumers.clone(),
                supply: None,
            },
            PickupGrant {
                entry: 0x66960,
                recipient: NativeRegion {
                    entry: 0x66974,
                    join: 0x66ce4,
                },
                resource: GrantResource::Inventory,
                consumers,
                supply: None,
            },
        ],
        items: ItemTable {
            table: 0x195320,
            stride: 192,
            count: 84,
            classname: 8,
            pickup: 16,
        },
        entity: PickupEntityFields {
            item: 0x860,
            count: 0x7b8,
            spawnflags: 0x5f0,
            inuse,
            inuse_bytes: 1,
            generation,
        },
        time: TimeSource {
            address: 0x241b28,
            storage: "int64-milliseconds",
        },
        supply: SupplyProfile {
            client: 0x78,
            inventory: 0xa80,
            flags: 0x7c,
            weapon_flag: 1,
            ammo: AmmoProfile {
                entry: 0x670e0,
                stop: 0x6712e,
                tag: 0x90,
                capacities: (0..12).map(|tag| 0xbd0 + tag * 2).collect(),
                capacity_bytes: 2,
            },
        },
    })
}

/// Preview pickup grants against admitted entries.
pub fn preview_supply(
    inventory: &[InventoryEntry],
    plan: &PickupGrantPlan,
) -> Result<PickupSupplyPreview, PickupProfileError> {
    Ok(preview_pickup_grants(inventory, plan)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::inventory::{CountArithmetic, CountPolicy};
    use qa_world::pickups::{AmmoAcceptance, AmmoWeapons, PickupAmmoGrant};

    fn entry(item: &str, count: f64, capacity: f64) -> InventoryEntry {
        InventoryEntry {
            item: item.to_string(),
            count,
            capacity,
            count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
        }
    }

    #[test]
    fn retail_profile_is_digest_gated() {
        assert!(rerelease_pickup_profile("sha256:other").is_none());
        let profile = rerelease_pickup_profile(RETAIL_ARTIFACT_DIGEST).expect("profile");
        assert_eq!(profile.touch, 0x67be0);
        assert_eq!(profile.grants.len(), 5);
        assert_eq!(profile.items.count, 84);
        assert_eq!(profile.entity.inuse, 1376);
        assert_eq!(profile.entity.generation, 1472);
        assert_eq!(profile.supply.ammo.capacities.len(), 12);
        assert_eq!(profile.supply.ammo.capacities[0], 0xbd0);
        assert_eq!(touch_signature().parameters.len(), 4);
        assert!(grant_signature().result.is_some());
        assert_eq!(ammo_signature().parameters.len(), 3);
    }

    #[test]
    fn supply_preview_accepts_landing_ammo() {
        let inventory = vec![
            entry("q2:ammo_cells", 10.0, 100.0),
            entry("q2:weapon_hyperblaster", 0.0, 1.0),
        ];
        let plan = PickupGrantPlan::Ammo {
            acceptance: AmmoAcceptance::Positive,
            ammo: vec![PickupAmmoGrant {
                item: "q2:ammo_cells".to_string(),
                amount: 25.0,
            }],
            weapons: AmmoWeapons::SharedAmmo {
                items: vec!["q2:weapon_hyperblaster".to_string()],
            },
        };
        let preview = preview_supply(&inventory, &plan).expect("preview");
        assert!(preview.accepted);
        assert_eq!(preview.ammo.len(), 1);
        assert_eq!(preview.ammo[0].given, 25.0);
    }
}
