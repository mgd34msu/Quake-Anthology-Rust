//! Donor: `src/compat/q2/classic/pickup-profile.ts` — the verified primary
//! pickup profile.
//!
//! Bridges the original touch/grant callsites to the shared pickup supply
//! preview: recipient regions, item-table shape, and ammo capacity fields
//! are pinned per artifact digest.

use qa_guest::core::contracts::{ContentDigest, GuestCallSignature};

use super::combat_profile::XATRIX_DIGEST_VALUE;
use super::layout::{classic_signature, q2_int, q2_pointer};

/// Source region from entry to join address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupRegion {
    /// Region entry offset.
    pub entry: u32,
    /// Region join offset.
    pub join: u32,
}

/// Ammo supply declaration inside a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupAmmoSupply {
    /// Grant entry offset.
    pub entry: u32,
    /// Register carrying the amount (`rbx`).
    pub amount_register: &'static str,
}

/// Weapon supply declaration inside a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupWeaponSupply {
    /// Ammo return offset.
    pub ammo_return: u32,
    /// Settle offset.
    pub settle: u32,
    /// Autoswitch region.
    pub autoswitch: PickupRegion,
}

/// Grant supply declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupGrantSupply {
    /// Ammo grant.
    Ammo(PickupAmmoSupply),
    /// Weapon grant.
    Weapon(PickupWeaponSupply),
}

/// Resource class consumed by a grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupGrantResource {
    /// Regular resource pickup.
    Regular,
    /// Inventory pickup.
    Inventory,
}

/// One pickup grant declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupGrant {
    /// Grant entry offset.
    pub entry: u32,
    /// Recipient region.
    pub recipient: PickupRegion,
    /// Resource class.
    pub resource: PickupGrantResource,
    /// Supply declaration, if any.
    pub supply: Option<PickupGrantSupply>,
}

/// Item table shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupItemTable {
    /// Table base offset.
    pub table: u32,
    /// Record stride.
    pub stride: usize,
    /// Entry count.
    pub count: usize,
    /// Classname field.
    pub classname: usize,
    /// Pickup function field.
    pub pickup: usize,
}

/// Entity fields read around a pickup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntityFields {
    /// Item pointer field.
    pub item: usize,
    /// Count field.
    pub count: usize,
    /// Spawnflags field.
    pub spawnflags: usize,
    /// Inuse field.
    pub inuse: usize,
    /// Inuse width.
    pub inuse_bytes: usize,
}

/// Time source declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupTime {
    /// Time global offset.
    pub address: u32,
    /// Storage encoding.
    pub storage: &'static str,
}

/// Ammo supply interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupAmmoInterface {
    /// Ammo entry offset.
    pub entry: u32,
    /// Ammo call signature.
    pub signature: GuestCallSignature,
    /// Ammo tag field.
    pub tag: usize,
    /// Per-tag capacity fields.
    pub capacities: Vec<usize>,
    /// Capacity width.
    pub capacity_bytes: usize,
}

/// Client supply fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupSupply {
    /// Client pointer field.
    pub client: usize,
    /// Inventory field.
    pub inventory: usize,
    /// Flags field.
    pub flags: usize,
    /// Weapon flag bit.
    pub weapon_flag: u32,
    /// Ammo interface.
    pub ammo: PickupAmmoInterface,
}

/// Verified pickup profile for one original artifact.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicPickupProfile {
    /// Original artifact digest.
    pub digest: ContentDigest,
    /// Touch entry offset.
    pub touch: u32,
    /// Grant return offset.
    pub grant_return: u32,
    /// Targets return offset.
    pub targets_return: u32,
    /// Touch call signature.
    pub touch_signature: GuestCallSignature,
    /// Grant call signature.
    pub grant_signature: GuestCallSignature,
    /// Grant declarations.
    pub grants: Vec<PickupGrant>,
    /// Item table.
    pub items: PickupItemTable,
    /// Entity fields.
    pub entity: PickupEntityFields,
    /// Time source.
    pub time: PickupTime,
    /// Supply interface.
    pub supply: PickupSupply,
}

/// Original Xatrix PE profile: recipient regions contain no map effects and
/// joins retain respawn tails.
#[must_use]
pub fn xatrix_pickup_profile() -> ClassicPickupProfile {
    ClassicPickupProfile {
        digest: ContentDigest::new("sha256", XATRIX_DIGEST_VALUE),
        touch: 0xab00,
        grant_return: 0xab38,
        targets_return: 0xac90,
        touch_signature: classic_signature(
            vec![q2_pointer(), q2_pointer(), q2_pointer(), q2_pointer()],
            None,
            false,
        ),
        grant_signature: classic_signature(vec![q2_pointer(), q2_pointer()], Some(q2_int()), false),
        grants: vec![
            PickupGrant {
                entry: 0xa780,
                recipient: PickupRegion { entry: 0xa795, join: 0xa8cb },
                resource: PickupGrantResource::Regular,
                supply: None,
            },
            PickupGrant {
                entry: 0xa3e0,
                recipient: PickupRegion { entry: 0xa3e4, join: 0xa4b8 },
                resource: PickupGrantResource::Inventory,
                supply: Some(PickupGrantSupply::Ammo(PickupAmmoSupply {
                    entry: 0xa41c,
                    amount_register: "rbx",
                })),
            },
            PickupGrant {
                entry: 0x35ff0,
                recipient: PickupRegion { entry: 0x36064, join: 0x36077 },
                resource: PickupGrantResource::Inventory,
                supply: Some(PickupGrantSupply::Weapon(PickupWeaponSupply {
                    ammo_return: 0x360dc,
                    settle: 0x360e5,
                    autoswitch: PickupRegion { entry: 0x3614a, join: 0x3619b },
                })),
            },
            PickupGrant {
                entry: 0x9960,
                recipient: PickupRegion { entry: 0x996b, join: 0x9a72 },
                resource: PickupGrantResource::Inventory,
                supply: None,
            },
            PickupGrant {
                entry: 0x9ac0,
                recipient: PickupRegion { entry: 0x9acb, join: 0x9d98 },
                resource: PickupGrantResource::Inventory,
                supply: None,
            },
        ],
        items: PickupItemTable { table: 0x4b828, stride: 76, count: 48, classname: 0, pickup: 4 },
        entity: PickupEntityFields {
            item: 0x288,
            count: 0x214,
            spawnflags: 0x11c,
            inuse: 88,
            inuse_bytes: 4,
        },
        time: PickupTime { address: 0x76804, storage: "float32-seconds" },
        supply: PickupSupply {
            client: 0x54,
            inventory: 0x2e4,
            flags: 0x38,
            weapon_flag: 1,
            ammo: PickupAmmoInterface {
                entry: 0xa310,
                signature: classic_signature(
                    vec![q2_pointer(), q2_pointer(), q2_int()],
                    Some(q2_int()),
                    false,
                ),
                tag: 0x44,
                capacities: vec![0x6e4, 0x6e8, 0x6ec, 0x6f0, 0x6f4, 0x6f8, 0x6fc, 0x700],
                capacity_bytes: 4,
            },
        },
    }
}

/// Select the pickup profile for an artifact digest, if admitted.
#[must_use]
pub fn classic_pickup_profile(digest: &ContentDigest) -> Option<ClassicPickupProfile> {
    if digest.algorithm == "sha256" && digest.value == XATRIX_DIGEST_VALUE {
        Some(xatrix_pickup_profile())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_guest::core::contracts::GuestValueLayout;

    #[test]
    fn xatrix_profile_matches_its_digest() {
        let digest = ContentDigest::new("sha256", XATRIX_DIGEST_VALUE);
        let profile = classic_pickup_profile(&digest).unwrap();
        assert_eq!(profile.touch, 0xab00);
        assert_eq!(profile.grants.len(), 5);
        assert_eq!(profile.items.count, 48);
        assert_eq!(profile.supply.ammo.capacities.len(), 8);
        assert_eq!(profile.touch_signature.parameters.len(), 4);
        assert_eq!(profile.grant_signature.result, Some(q2_int()));
        assert!(classic_pickup_profile(&ContentDigest::new("sha256", "00")).is_none());
    }

    #[test]
    fn grant_regions_and_signatures_match_donor() {
        let profile = xatrix_pickup_profile();
        assert!(profile.grants.iter().all(|grant| grant.recipient.entry < grant.recipient.join));
        let ammo = profile.grants[1].supply.unwrap();
        assert!(matches!(ammo, PickupGrantSupply::Ammo(_)));
        let weapon = profile.grants[2].supply.unwrap();
        assert!(matches!(weapon, PickupGrantSupply::Weapon(_)));
        assert_eq!(
            profile.touch_signature.parameters,
            vec![q2_pointer(), q2_pointer(), q2_pointer(), q2_pointer()]
        );
        assert!(matches!(
            profile.supply.ammo.signature.result,
            Some(GuestValueLayout::Scalar(_))
        ));
        assert_eq!(profile.entity.inuse, 88);
        assert_eq!(profile.time.storage, "float32-seconds");
    }
}
