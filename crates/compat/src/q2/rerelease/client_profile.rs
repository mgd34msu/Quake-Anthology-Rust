//! Q2 rerelease client record profiles (source vs observed retail).
//!
//! Donor: `src/compat/q2/rerelease/client-profile.ts` — bridges the source
//! `gclient_t` prefix and the artifact-observed retail client fields.

use qa_guest::core::contracts::{GuestFieldLayout, GuestLayout, GuestStorage};

use super::layouts::{client_layout, private_client_prefix_layout};

/// Profile authority: source header digest or observed artifact digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientAuthority {
    /// Source header provenance.
    Source {
        /// Header digest.
        header_digest: String,
    },
    /// Observed retail artifact provenance.
    Artifact {
        /// Artifact digest.
        digest: String,
    },
}

/// Client record profile: layout plus inventory/ammunition counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseClientProfile {
    /// Profile authority.
    pub authority: ClientAuthority,
    /// Client record layout.
    pub layout: GuestLayout,
    /// `IT_TOTAL` inventory slots.
    pub inventory_count: usize,
    /// `AMMO_MAX` capacity slots.
    pub ammo_count: usize,
}

/// Source header digest from the donor.
pub const SOURCE_HEADER_DIGEST: &str =
    "sha256:3257c79f07d9e8ef333342b9a0be0bde7dd514d9de1b9b36173aad157601aecf";

/// Retail artifact digest from the donor.
pub const RETAIL_ARTIFACT_DIGEST: &str =
    "sha256:045d49c53722d9b922caf14f168dd28a97d4c514a6e443a3140560f8668baccd";

/// Source client profile: 82 inventory slots, 12 ammo capacities.
#[must_use]
pub fn source_client_profile() -> RereleaseClientProfile {
    RereleaseClientProfile {
        authority: ClientAuthority::Source {
            header_digest: SOURCE_HEADER_DIGEST.to_string(),
        },
        layout: private_client_prefix_layout(),
        inventory_count: 82,
        ammo_count: 12,
    }
}

fn sparse_field(
    name: &str,
    byte_offset: usize,
    storage: GuestStorage,
    count: usize,
) -> GuestFieldLayout {
    GuestFieldLayout {
        name: name.to_string(),
        byte_offset,
        storage,
        count,
    }
}

/// Retail observed profile: sparse artifact fields over the shared prefix.
///
/// Observed notes from the donor: Bot_SetWeapon sees inventory at 0xa80,
/// weapon at 0xbe8, newweapon at 0x1898, no_weapon_chains at 0x1b68 with an
/// 84-item bound; Bot_UseItem sees selected_item at 0xa70; Init allocates
/// 7344 bytes per client; ClientConnect initializes twelve int16 ammo
/// capacities at 3024; Invulnerability writes int64 milliseconds at 6672;
/// ClientThink copies pm viewangles to v_angle at 0x1998.
#[must_use]
pub fn retail_client_profile() -> RereleaseClientProfile {
    let shared = client_layout();
    let mut fields: Vec<GuestFieldLayout> = shared
        .fields
        .iter()
        .map(|field| GuestFieldLayout {
            name: format!("shared.{}", field.name),
            byte_offset: field.byte_offset,
            storage: field.storage,
            count: field.count,
        })
        .collect();
    fields.push(sparse_field(
        "resp.score",
        0x17c8,
        GuestStorage::Int32,
        1,
    ));
    fields.push(sparse_field(
        "resp.ctf_team",
        0x17dc,
        GuestStorage::Int32,
        1,
    ));
    fields.push(sparse_field(
        "pers.selected_item",
        2672,
        GuestStorage::Int32,
        1,
    ));
    fields.push(sparse_field(
        "pers.inventory",
        2688,
        GuestStorage::Int32,
        84,
    ));
    fields.push(sparse_field(
        "pers.max_ammo",
        3024,
        GuestStorage::Int16,
        12,
    ));
    fields.push(sparse_field(
        "pers.weapon",
        3048,
        GuestStorage::Pointer,
        1,
    ));
    fields.push(sparse_field(
        "newweapon",
        6296,
        GuestStorage::Pointer,
        1,
    ));
    fields.push(sparse_field("v_angle", 6552, GuestStorage::Float32, 3));
    fields.push(sparse_field(
        "invincible_time",
        6672,
        GuestStorage::Int64,
        1,
    ));
    fields.push(sparse_field(
        "no_weapon_chains",
        7016,
        GuestStorage::Uint8,
        1,
    ));
    RereleaseClientProfile {
        authority: ClientAuthority::Artifact {
            digest: RETAIL_ARTIFACT_DIGEST.to_string(),
        },
        layout: GuestLayout::new(
            "q2-rerelease-retail:observed-client-fields",
            7344,
            8,
            8,
            fields,
        ),
        inventory_count: 84,
        ammo_count: 12,
    }
}

/// Select the profile matching a module digest, if any.
#[must_use]
pub fn client_profile_for_digest(digest: &str) -> Option<RereleaseClientProfile> {
    if digest == RETAIL_ARTIFACT_DIGEST {
        Some(retail_client_profile())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::layouts::field_offset;
    use super::*;

    #[test]
    fn source_profile_uses_private_prefix() {
        let profile = source_client_profile();
        assert_eq!(profile.inventory_count, 82);
        assert_eq!(profile.ammo_count, 12);
        assert!(matches!(
            profile.authority,
            ClientAuthority::Source { .. }
        ));
        assert!(field_offset(&profile.layout, "shared.ps.ping").is_err());
        assert!(field_offset(&profile.layout, "shared.ping").is_ok());
        assert!(field_offset(&profile.layout, "pers.inventory").is_ok());
    }

    #[test]
    fn retail_profile_has_observed_sparse_fields() {
        let profile = retail_client_profile();
        assert_eq!(profile.inventory_count, 84);
        assert_eq!(profile.layout.byte_length, 7344);
        assert_eq!(
            field_offset(&profile.layout, "pers.inventory").unwrap(),
            2688
        );
        assert_eq!(
            field_offset(&profile.layout, "pers.weapon").unwrap(),
            3048
        );
        assert_eq!(field_offset(&profile.layout, "v_angle").unwrap(), 6552);
        assert_eq!(
            field_offset(&profile.layout, "invincible_time").unwrap(),
            6672
        );
        assert_eq!(
            field_offset(&profile.layout, "no_weapon_chains").unwrap(),
            7016
        );
        assert!(field_offset(&profile.layout, "shared.ping").is_ok());
        assert!(client_profile_for_digest(RETAIL_ARTIFACT_DIGEST).is_some());
        assert!(client_profile_for_digest(SOURCE_HEADER_DIGEST).is_none());
    }
}
