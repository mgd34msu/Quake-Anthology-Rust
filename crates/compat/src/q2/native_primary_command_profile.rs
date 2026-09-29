//! Port of `src/compat/q2/native-primary-command-profile.ts`.
//! Bridges builtin command profiles: Xatrix classic and retail rerelease tables.

use qa_guest::core::contracts::{GuestRegister, NativeAbi};

use super::native_primary_commands::{
    AmmoGrant, CommandClient, CommandItems, DropCommand, GiveProfile, GrantKind,
    ItemAmmo, NativePrimaryCommandProfile,
};
use super::native_primary_reader::{CLASSIC_DIGEST, RETAIL_DIGEST, NativeRegion};

/// Builtin command profile for a digest, or `None` when unknown.
#[must_use]
pub fn native_primary_command_profile(digest: &str) -> Option<NativePrimaryCommandProfile> {
    if digest == CLASSIC_DIGEST {
        Some(NativePrimaryCommandProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            give: GiveProfile {
                entry: 0x3140,
                weapons: 0x3256,
                ammo: 0x32af,
                unknown: NativeRegion { entry: 0x3455, join: 0x3466 },
                ammo_grants: vec![
                    AmmoGrant {
                        entry: 0x34ce,
                        join: 0x34d5,
                        descriptor: GuestRegister::Rsi,
                        kind: GrantKind::Set,
                    },
                    AmmoGrant {
                        entry: 0x34db,
                        join: 0x34f3,
                        descriptor: GuestRegister::Rsi,
                        kind: GrantKind::Add,
                    },
                ],
                argc: 0x7677c,
                argv: 0x76780,
            },
            drop: DropCommand {
                entry: 0x307c0,
                eligibility: NativeRegion { entry: 0x307e7, join: 0x3083b },
            },
            client: CommandClient {
                pointer: 0x54,
                weapon: 0x704,
                ammo_index: Some(0xdc8),
                inventory: 0x2e4,
            },
            items: CommandItems {
                table: 0x4b828,
                stride: 76,
                count: 48,
                classname: 0,
                flags: 0x38,
                weapon_flag: 1,
                ammunition_flag: 2,
                icon: 0x24,
                ammo: ItemAmmo::Name { offset: 0x34, label: 0x28 },
            },
        })
    } else if digest == RETAIL_DIGEST {
        Some(NativePrimaryCommandProfile {
            digest: RETAIL_DIGEST.to_string(),
            abi: NativeAbi::WindowsX86_64,
            give: GiveProfile {
                entry: 0x56de0,
                weapons: 0x57372,
                ammo: 0x573f7,
                unknown: NativeRegion { entry: 0x57091, join: 0x5781d },
                ammo_grants: vec![
                    AmmoGrant {
                        entry: 0x57127,
                        join: 0x5712e,
                        descriptor: GuestRegister::Rdi,
                        kind: GrantKind::Set,
                    },
                    AmmoGrant {
                        entry: 0x57133,
                        join: 0x5713d,
                        descriptor: GuestRegister::Rdi,
                        kind: GrantKind::Add,
                    },
                ],
                argc: 0x1da780,
                argv: 0x1da788,
            },
            drop: DropCommand {
                entry: 0xd5ec0,
                eligibility: NativeRegion { entry: 0xd5eed, join: 0xd5f30 },
            },
            client: CommandClient {
                pointer: 0x78,
                weapon: 0xbe8,
                ammo_index: None,
                inventory: 0xa80,
            },
            items: CommandItems {
                table: 0x195320,
                stride: 192,
                count: 84,
                classname: 8,
                flags: 0x7c,
                weapon_flag: 1,
                ammunition_flag: 2,
                icon: 0x50,
                ammo: ItemAmmo::Index { offset: 0x74 },
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
    fn resolves_xatrix_profile() {
        let profile = native_primary_command_profile(CLASSIC_DIGEST).expect("xatrix");
        assert_eq!(profile.abi, NativeAbi::WindowsI386);
        assert_eq!(profile.give.ammo_grants.len(), 2);
        assert_eq!(profile.client.ammo_index, Some(0xdc8));
        assert!(matches!(profile.items.ammo, ItemAmmo::Name { .. }));
        assert_eq!(profile.items.count, 48);
    }

    #[test]
    fn resolves_retail_profile() {
        let profile = native_primary_command_profile(RETAIL_DIGEST).expect("retail");
        assert_eq!(profile.abi, NativeAbi::WindowsX86_64);
        assert_eq!(profile.client.ammo_index, None);
        assert!(matches!(profile.items.ammo, ItemAmmo::Index { .. }));
        assert_eq!(profile.items.count, 84);
    }

    #[test]
    fn rejects_unknown_digests() {
        assert!(native_primary_command_profile("sha256:dead").is_none());
    }
}
