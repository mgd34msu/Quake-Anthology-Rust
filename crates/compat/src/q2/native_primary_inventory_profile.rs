//! Port of `src/compat/q2/native-primary-inventory-profile.ts`.
//! Bridges builtin inventory profiles: classic and retail rerelease tables.

use qa_guest::core::contracts::NativeAbi;

use super::native_primary_inventory::{
    InventoryPrototypes, NamedUseProfile, NativePrimaryInventoryProfile, NextProfile,
    PreviousProfile, SelectionWrite, UseProfile, ValidateProfile,
};
use super::native_primary_reader::{CLASSIC_DIGEST, RETAIL_DIGEST, NativeRegion};

fn prototypes() -> InventoryPrototypes {
    InventoryPrototypes {
        weapon: "q2:weapon_blaster".to_string(),
        ammunition: "q2:ammo_shells".to_string(),
        usable: "q2:item_quad".to_string(),
        passive: "q2:key_data_cd".to_string(),
        droppable: "q2:item_quad".to_string(),
        undroppable: "q2:weapon_blaster".to_string(),
    }
}

/// Builtin inventory profile for a digest, or `None` when unknown.
#[must_use]
pub fn native_primary_inventory_profile(digest: &str) -> Option<NativePrimaryInventoryProfile> {
    if digest == CLASSIC_DIGEST {
        Some(NativePrimaryInventoryProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            client: 0x54,
            inventory: 0x2e4,
            count: 256,
            cursor: 0x2e0,
            empty: -1,
            prototypes: prototypes(),
            selection_writes: vec![SelectionWrite { offset: 0x2e0, bytes: 4 }],
            next: NextProfile {
                entry: 0x2fe0,
                scan: 0x3003,
                join: 0x306a,
                menu_argument: false,
            },
            previous: PreviousProfile {
                entry: 0x3070,
                scan: 0x3093,
                join: 0x30ff,
            },
            validate: ValidateProfile { entry: 0x3110, scan: None },
            use_profile: UseProfile {
                entry: 0x3a60,
                call: 0x3abc,
                join: 0x3abe,
            },
            named_use: NamedUseProfile {
                entry: 0x36d0,
                lookup_call: 0x36dd,
                lookup_return: 0x36e2,
                call: 0x384c,
                join: 0x384f,
            },
        })
    } else if digest == RETAIL_DIGEST {
        Some(NativePrimaryInventoryProfile {
            digest: RETAIL_DIGEST.to_string(),
            abi: NativeAbi::WindowsX86_64,
            client: 0x78,
            inventory: 0xa80,
            count: 84,
            cursor: 0xa70,
            empty: 0,
            prototypes: prototypes(),
            selection_writes: vec![
                SelectionWrite { offset: 0xa70, bytes: 4 },
                SelectionWrite { offset: 0xa78, bytes: 8 },
                SelectionWrite { offset: 0x10c, bytes: 2 },
            ],
            next: NextProfile {
                entry: 0x56a40,
                scan: 0x56a76,
                join: 0x56b1e,
                menu_argument: true,
            },
            previous: PreviousProfile {
                entry: 0x56b30,
                scan: 0x56bde,
                join: 0x56c84,
            },
            validate: ValidateProfile {
                entry: 0x56c90,
                scan: Some(NativeRegion { entry: 0x56ca6, join: 0x56d14 }),
            },
            use_profile: UseProfile {
                entry: 0x58670,
                call: 0x58775,
                join: 0x5877b,
            },
            named_use: NamedUseProfile {
                entry: 0x581f0,
                lookup_call: 0x582cf,
                lookup_return: 0x582d4,
                call: 0x583bc,
                join: 0x583c2,
            },
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_classic_profile() {
        let profile = native_primary_inventory_profile(CLASSIC_DIGEST).expect("classic");
        assert_eq!(profile.abi, NativeAbi::WindowsI386);
        assert_eq!(profile.count, 256);
        assert_eq!(profile.empty, -1);
        assert!(!profile.next.menu_argument);
        assert_eq!(profile.validate.scan, None);
    }

    #[test]
    fn resolves_retail_profile() {
        let profile = native_primary_inventory_profile(RETAIL_DIGEST).expect("retail");
        assert_eq!(profile.abi, NativeAbi::WindowsX86_64);
        assert_eq!(profile.count, 84);
        assert_eq!(profile.empty, 0);
        assert!(profile.next.menu_argument);
        assert!(profile.validate.scan.is_some());
        assert_eq!(profile.selection_writes.len(), 3);
    }

    #[test]
    fn rejects_unknown_digests() {
        assert!(native_primary_inventory_profile("sha256:dead").is_none());
    }
}
