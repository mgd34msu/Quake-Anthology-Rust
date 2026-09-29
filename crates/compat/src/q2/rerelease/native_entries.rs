//! Q2 rerelease native entry resolution and combat signatures.
//!
//! Donor: `src/compat/q2/rerelease/native-entries.ts` — bridges
//! artifact-qualified source functions and data into entry addresses.

use qa_guest::core::contracts::{
    GuestAccess, GuestAddress, GuestCallSignature, GuestLayout, GuestStorage, GuestValueLayout, NativeCallAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

use super::layouts::field_offset;

/// Native entry failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NativeEntryError {
    /// Profile belongs to another artifact.
    #[error("Rerelease native entries belong to another artifact")]
    ForeignProfile,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Rerelease ABI for native signatures.
#[must_use]
pub const fn rerelease_abi() -> NativeCallAbi {
    NativeCallAbi::MicrosoftX64
}

/// Three-byte by-value `mod_t` layout.
#[must_use]
pub fn mod_layout() -> GuestLayout {
    use qa_guest::core::contracts::GuestFieldLayout;
    GuestLayout::new(
        "q2-rerelease:mod_t",
        3,
        1,
        8,
        vec![
            GuestFieldLayout {
                name: "id".to_string(),
                byte_offset: 0,
                storage: GuestStorage::Uint8,
                count: 1,
            },
            GuestFieldLayout {
                name: "friendly_fire".to_string(),
                byte_offset: 1,
                storage: GuestStorage::Uint8,
                count: 1,
            },
            GuestFieldLayout {
                name: "no_point_loss".to_string(),
                byte_offset: 2,
                storage: GuestStorage::Uint8,
                count: 1,
            },
        ],
    )
}

fn scalar(storage: GuestStorage) -> GuestValueLayout {
    GuestValueLayout::Scalar(storage)
}

fn signature(parameters: Vec<GuestValueLayout>, result: Option<GuestValueLayout>) -> GuestCallSignature {
    GuestCallSignature {
        abi: rerelease_abi(),
        parameters,
        result,
        variadic: false,
    }
}

/// `G_Spawn` signature: no parameters, pointer result.
#[must_use]
pub fn spawn_signature() -> GuestCallSignature {
    signature(vec![], Some(scalar(GuestStorage::Pointer)))
}

/// `G_FreeEdict` signature: one entity pointer.
#[must_use]
pub fn free_signature() -> GuestCallSignature {
    signature(vec![scalar(GuestStorage::Pointer)], None)
}

/// Stock `T_Damage` semantic fields.
pub const DAMAGE_FIELDS: &[&str] = &[
    "target",
    "inflictor",
    "attacker",
    "direction",
    "point",
    "normal",
    "amount",
    "knockback",
    "flags",
    "cause",
];

/// Stock `P_Damage` power-armor semantic fields.
pub const POWER_ARMOR_FIELDS: &[&str] = &["target", "point", "normal", "amount", "flags"];

fn field_layout(field: &str) -> GuestValueLayout {
    match field {
        "target" | "inflictor" | "attacker" | "direction" | "point" | "normal" => scalar(GuestStorage::Pointer),
        "cause" => GuestValueLayout::Aggregate(mod_layout()),
        _ => scalar(GuestStorage::Int32),
    }
}

/// Stock damage signature over the ten semantic fields.
#[must_use]
pub fn damage_signature() -> GuestCallSignature {
    signature(DAMAGE_FIELDS.iter().map(|field| field_layout(field)).collect(), None)
}

/// Stock power-armor signature with an integer result.
#[must_use]
pub fn power_armor_signature() -> GuestCallSignature {
    signature(
        POWER_ARMOR_FIELDS.iter().map(|field| field_layout(field)).collect(),
        Some(scalar(GuestStorage::Int32)),
    )
}

/// Native entry RVAs from the selected world profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeEntryRvas {
    /// Digest the RVAs were observed against.
    pub digest: &'static str,
    /// `G_Spawn`.
    pub spawn: u64,
    /// `G_FreeEdict`.
    pub free: u64,
    /// `T_Damage`.
    pub damage: u64,
    /// Power-armor stage.
    pub power_armor: u64,
    /// Regular-armor region entry.
    pub regular_armor_entry: u64,
    /// Regular-armor region join.
    pub regular_armor_join: u64,
    /// Armor info table.
    pub armor_table: u64,
    /// Armor stride.
    pub armor_stride: usize,
    /// Inventory count for the table extent.
    pub inventory_count: usize,
    /// Deferred pain processor.
    pub process_pain: u64,
    /// Level time.
    pub time: u64,
}

/// Resolved native entries within the loaded module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereleaseNativeEntries {
    /// `G_Spawn`.
    pub spawn: GuestAddress,
    /// `G_FreeEdict`.
    pub free: GuestAddress,
    /// `T_Damage`.
    pub damage: GuestAddress,
    /// Power-armor stage.
    pub power_armor: GuestAddress,
    /// Regular-armor region entry.
    pub regular_armor_entry: GuestAddress,
    /// Regular-armor region join.
    pub regular_armor_join: GuestAddress,
    /// Armor info table.
    pub armor_info_table: GuestAddress,
    /// Deferred pain processor.
    pub process_pain: GuestAddress,
    /// Level time.
    pub time: GuestAddress,
}

/// Resolve artifact-qualified source functions and data.
pub fn rerelease_entries(
    memory: &mut SparseGuestMemory,
    module_digest: &str,
    image_base: GuestAddress,
    profile: &NativeEntryRvas,
) -> Result<RereleaseNativeEntries, NativeEntryError> {
    if profile.digest != module_digest {
        return Err(NativeEntryError::ForeignProfile);
    }
    let entry = |memory: &mut SparseGuestMemory, rva: u64| {
        memory.offset(image_base, rva as i64).map_err(NativeEntryError::from)
    };
    let data = |memory: &mut SparseGuestMemory, rva: u64, bytes: usize| {
        let address = memory.offset(image_base, rva as i64)?;
        memory.check(address, bytes, GuestAccess::Read)?;
        Ok(address)
    };
    Ok(RereleaseNativeEntries {
        spawn: entry(memory, profile.spawn)?,
        free: entry(memory, profile.free)?,
        damage: entry(memory, profile.damage)?,
        power_armor: entry(memory, profile.power_armor)?,
        regular_armor_entry: entry(memory, profile.regular_armor_entry)?,
        regular_armor_join: entry(memory, profile.regular_armor_join)?,
        armor_info_table: data(
            memory,
            profile.armor_table,
            (profile.inventory_count - 1) * profile.armor_stride + 8,
        )?,
        process_pain: entry(memory, profile.process_pain)?,
        time: data(memory, profile.time, 8)?,
    })
}

/// Verify a table layout exposes every expected entry name.
pub fn verify_table_names(layout: &GuestLayout, names: &[&str]) -> Result<(), NativeEntryError> {
    for name in names {
        field_offset(layout, name).map_err(|_| NativeEntryError::ForeignProfile)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    const DIGEST: &str = "sha256:045d49c53722d9b922caf14f168dd28a97d4c514a6e443a3140560f8668baccd";

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "native-entries-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn profile() -> NativeEntryRvas {
        NativeEntryRvas {
            digest: DIGEST,
            spawn: 0x100,
            free: 0x200,
            damage: 0x300,
            power_armor: 0x400,
            regular_armor_entry: 0x500,
            regular_armor_join: 0x600,
            armor_table: 0x1000,
            armor_stride: 192,
            inventory_count: 4,
            process_pain: 0x700,
            time: 0x2000,
        }
    }

    #[test]
    fn signatures_cover_stock_fields() {
        assert_eq!(spawn_signature().parameters.len(), 0);
        assert!(spawn_signature().result.is_some());
        assert_eq!(free_signature().parameters.len(), 1);
        assert!(free_signature().result.is_none());
        let damage = damage_signature();
        assert_eq!(damage.parameters.len(), 10);
        assert!(matches!(damage.parameters[9], GuestValueLayout::Aggregate(_)));
        let power = power_armor_signature();
        assert_eq!(power.parameters.len(), 5);
        assert_eq!(power.result, Some(GuestValueLayout::Scalar(GuestStorage::Int32)));
        assert_eq!(mod_layout().byte_length, 3);
    }

    #[test]
    fn entries_resolve_within_the_image() {
        let mut memory = test_memory();
        let image_base = memory.allocate(&GuestAllocationOptions::bytes(0x3000)).expect("alloc");
        let entries = rerelease_entries(&mut memory, DIGEST, image_base, &profile()).expect("entries");
        assert_eq!(entries.spawn.offset, image_base.offset + 0x100);
        assert_eq!(entries.regular_armor_join.offset, image_base.offset + 0x600);
        assert_eq!(entries.time.offset, image_base.offset + 0x2000);
        let foreign = rerelease_entries(&mut memory, "sha256:other", image_base, &profile());
        assert_eq!(foreign.unwrap_err(), NativeEntryError::ForeignProfile);
        let layout = super::super::layouts::trace_layout();
        verify_table_names(&layout, &["fraction", "ent"]).expect("names");
        assert!(verify_table_names(&layout, &["nope"]).is_err());
    }
}
