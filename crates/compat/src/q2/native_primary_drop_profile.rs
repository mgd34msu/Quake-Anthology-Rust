//! Port of `src/compat/q2/native-primary-drop-profile.ts`.
//! Bridges builtin drop profiles: classic and retail rerelease tables.

use qa_guest::core::contracts::NativeAbi;

use super::native_primary_drop::{
    DropClient, InventoryDrop, NativePrimaryDropProfile,
};
use super::native_primary_reader::{CLASSIC_DIGEST, RETAIL_DIGEST, NativeRegion};

/// Builtin drop profile for a digest, or `None` when unknown.
#[must_use]
pub fn native_primary_drop_profile(digest: &str) -> Option<NativePrimaryDropProfile> {
    if digest == CLASSIC_DIGEST {
        Some(NativePrimaryDropProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            client: DropClient {
                pointer: 0x54,
                inventory: 0x2e4,
                cursor: 0x2e0,
                weapon: 0x704,
                pending: 0xddc,
            },
            named: 0x3860,
            inventory: InventoryDrop { entry: 0x3c80, admitted: 0x3c8b },
            find: 0x9590,
            lookup_return: 0x3872,
            allocate: 0xad00,
            free: 0x19140,
            consumer: None,
            callbacks: vec![
                NativeRegion { entry: 0x39dc, join: 0x39df },
                NativeRegion { entry: 0x3cdc, join: 0x3cde },
            ],
            debits: vec![
                NativeRegion { entry: 0x36b57, join: 0x36b59 },
                NativeRegion { entry: 0xa5ab, join: 0xa5ae },
            ],
        })
    } else if digest == RETAIL_DIGEST {
        Some(NativePrimaryDropProfile {
            digest: RETAIL_DIGEST.to_string(),
            abi: NativeAbi::WindowsX86_64,
            client: DropClient {
                pointer: 0x78,
                inventory: 0xa80,
                cursor: 0xa70,
                weapon: 0xbe8,
                pending: 0x1898,
            },
            named: 0x58410,
            inventory: InventoryDrop { entry: 0x58970, admitted: 0x58998 },
            find: 0x660a0,
            lookup_return: 0x58541,
            allocate: 0x680c0,
            free: 0x96600,
            consumer: Some(NativeRegion { entry: 0x674a6, join: 0x674ab }),
            callbacks: vec![
                NativeRegion { entry: 0x58601, join: 0x58607 },
                NativeRegion { entry: 0x58a22, join: 0x58a28 },
            ],
            debits: vec![
                NativeRegion { entry: 0xf0964, join: 0xf096b },
                NativeRegion { entry: 0x6749f, join: 0x674a3 },
            ],
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
        let profile = native_primary_drop_profile(CLASSIC_DIGEST).expect("classic");
        assert_eq!(profile.abi, NativeAbi::WindowsI386);
        assert_eq!(profile.consumer, None);
        assert_eq!(profile.callbacks.len(), 2);
        assert_eq!(profile.debits.len(), 2);
        assert_eq!(profile.client.cursor, 0x2e0);
    }

    #[test]
    fn resolves_retail_profile() {
        let profile = native_primary_drop_profile(RETAIL_DIGEST).expect("retail");
        assert_eq!(profile.abi, NativeAbi::WindowsX86_64);
        assert!(profile.consumer.is_some());
        assert_eq!(profile.client.pending, 0x1898);
    }

    #[test]
    fn rejects_unknown_digests() {
        assert!(native_primary_drop_profile("sha256:dead").is_none());
    }
}
