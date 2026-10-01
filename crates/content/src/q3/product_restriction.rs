//! Quake III root: product restriction.
//!
//! Donor provenance: `src/content/q3/product-restriction.ts` and
//! `/home/buzzkill/Projects/quake-typescript/src/core/q3-product-policy.ts`.

use qa_core::cvar::{flags as cvar_flags, CvarRegistry};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::records::Q3BaseError;

// ---------------------------------------------------------------------------
// product-restriction.ts (+ core/q3-product-policy.ts restriction layer)
// ---------------------------------------------------------------------------

/// Scrambled product identifier (`FS_SetRestrictions` words).
pub const SCRAMBLED_PRODUCT_ID: [u8; 152] = [
    220, 129, 255, 108, 244, 163, 171, 55, 133, 65, 199, 36, 140, 222, 53, 99, 65, 171, 175, 232, 236, 193, 210, 250,
    169, 104, 231, 231, 21, 201, 170, 208, 135, 175, 130, 136, 85, 215, 71, 23, 96, 32, 96, 83, 44, 240, 219, 138, 184,
    215, 73, 27, 196, 247, 55, 139, 148, 68, 78, 203, 213, 238, 139, 23, 45, 205, 118, 186, 236, 230, 231, 107, 212, 1,
    10, 98, 30, 20, 116, 180, 216, 248, 166, 35, 45, 22, 215, 229, 35, 116, 250, 167, 117, 3, 57, 55, 201, 229, 218,
    222, 128, 12, 141, 149, 32, 110, 168, 215, 184, 53, 31, 147, 62, 12, 138, 67, 132, 54, 125, 6, 221, 148, 140, 4,
    21, 44, 198, 3, 126, 12, 100, 236, 61, 42, 44, 251, 15, 135, 14, 134, 89, 92, 177, 246, 152, 106, 124, 78, 118, 80,
    28, 42,
];

/// Team Arena UI word for the prerelease demo policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamArenaUi {
    /// Retail UI.
    Retail,
    /// Demo UI.
    Demo,
}

/// Q3 product policy (`Q3ProductPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ProductPolicy {
    /// Retail.
    Retail,
    /// Prerelease demo.
    PrereleaseDemo {
        /// Team Arena UI.
        team_arena_ui: TeamArenaUi,
    },
    /// Prerelease Team Arena demo.
    PrereleaseTaDemo,
}

/// Q3 mount restriction (`Q3MountRestriction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3MountRestriction {
    /// No restriction.
    None,
    /// Demo restriction.
    Demo {
        /// Demo directory.
        directory: &'static str,
        /// Demo pak checksum.
        pak_checksum: u32,
    },
}

/// Mount restriction for a policy (`q3MountRestriction`).
#[must_use]
pub fn q3_mount_restriction(policy: Q3ProductPolicy, fs_restrict: bool) -> Q3MountRestriction {
    if fs_restrict || matches!(policy, Q3ProductPolicy::PrereleaseDemo { .. }) {
        Q3MountRestriction::Demo {
            directory: "demota",
            pak_checksum: 437558517,
        }
    } else {
        Q3MountRestriction::None
    }
}

/// Resolve the mount restriction before selecting recipe artifacts; a demo
/// restriction changes the mounted content (`resolveQ3MountRestriction`).
///
/// The donor reads the product identifier asynchronously; this sync port
/// takes the already-read bytes (`None` when the read yields nothing).
pub fn resolve_q3_mount_restriction(
    policy: Q3ProductPolicy,
    fs_restrict: bool,
    product_id: Option<&[u8]>,
) -> Result<Q3MountRestriction, Q3BaseError> {
    let forced = q3_mount_restriction(policy, fs_restrict);
    if matches!(forced, Q3MountRestriction::Demo { .. }) {
        return Ok(forced);
    }
    let Some(product_id) = product_id else {
        return Ok(q3_mount_restriction(policy, true));
    };
    let mut seed: i32 = 5000;
    for (index, scrambled) in SCRAMBLED_PRODUCT_ID.iter().enumerate() {
        if (scrambled ^ ((seed & 255) as u8)) != product_id.get(index).copied().unwrap_or(0) {
            return Err(Q3BaseError::Invalid("Invalid product identification".to_string()));
        }
        seed = seed.wrapping_mul(69069).wrapping_add(1);
    }
    Ok(forced)
}

/// Product policy from the prerelease flags (`q3ProductPolicy`).
#[must_use]
pub fn q3_product_policy(prerelease_demo: bool, prerelease_team_arena_demo: bool) -> Q3ProductPolicy {
    if prerelease_demo {
        Q3ProductPolicy::PrereleaseDemo {
            team_arena_ui: if prerelease_team_arena_demo {
                TeamArenaUi::Demo
            } else {
                TeamArenaUi::Retail
            },
        }
    } else if prerelease_team_arena_demo {
        Q3ProductPolicy::PrereleaseTaDemo
    } else {
        Q3ProductPolicy::Retail
    }
}

/// Register the prerelease cvars and resolve the policy. Call after startup
/// `+set` values have been applied, before mounting content
/// (`registerQ3ProductPolicy`).
pub fn register_q3_product_policy(cvars: &mut CvarRegistry) -> Result<Q3ProductPolicy, Q3BaseError> {
    let demo = cvars
        .register("com_prereleaseDemo", "0", cvar_flags::INIT)
        .map_err(|error| Q3BaseError::Invalid(error.to_string()))?
        .is_some_and(|snapshot| snapshot.integer_value != 0);
    let team_arena = cvars
        .register("com_prereleaseTeamArenaDemo", "0", cvar_flags::INIT)
        .map_err(|error| Q3BaseError::Invalid(error.to_string()))?
        .is_some_and(|snapshot| snapshot.integer_value != 0);
    Ok(q3_product_policy(demo, team_arena))
}

/// Whether the policy is the prerelease demo (`q3PrereleaseDemo`).
#[must_use]
pub fn q3_prerelease_demo(policy: Q3ProductPolicy) -> bool {
    matches!(policy, Q3ProductPolicy::PrereleaseDemo { .. })
}

/// Whether Team Arena runs as a demo (`q3TeamArenaDemo`).
#[must_use]
pub fn q3_team_arena_demo(policy: Q3ProductPolicy) -> bool {
    matches!(policy, Q3ProductPolicy::PrereleaseTaDemo)
        || matches!(
            policy,
            Q3ProductPolicy::PrereleaseDemo {
                team_arena_ui: TeamArenaUi::Demo
            }
        )
}

/// Map commands available under a policy (`q3ProductMapCommands`).
#[must_use]
pub fn q3_product_map_commands(policy: Q3ProductPolicy) -> Vec<String> {
    if q3_prerelease_demo(policy) {
        vec!["map".to_string()]
    } else {
        ["map", "devmap", "spmap", "spdevmap"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }
}

/// Team Arena catalog file names and extra-content flags
/// (`q3TeamArenaCatalogPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3TeamArenaCatalogPolicy {
    /// Game info file.
    pub game_info: &'static str,
    /// Team info file.
    pub team_info: &'static str,
    /// Additional teams.
    pub additional_teams: bool,
    /// Additional arenas.
    pub additional_arenas: bool,
}

/// Catalog policy for a product policy (`q3TeamArenaCatalogPolicy`).
#[must_use]
pub fn q3_team_arena_catalog_policy(policy: Q3ProductPolicy) -> Q3TeamArenaCatalogPolicy {
    let demo = q3_team_arena_demo(policy);
    Q3TeamArenaCatalogPolicy {
        game_info: if demo { "demogameinfo.txt" } else { "gameinfo.txt" },
        team_info: if demo { "demoteaminfo.txt" } else { "teaminfo.txt" },
        additional_teams: !demo,
        additional_arenas: !demo,
    }
}

/// Application product: policy plus resolved restriction
/// (`Q3ApplicationProduct`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ApplicationProduct {
    /// Product policy.
    pub policy: Q3ProductPolicy,
    /// Mount restriction.
    pub restriction: Q3MountRestriction,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_product_id() -> Vec<u8> {
        let mut seed: i32 = 5000;
        SCRAMBLED_PRODUCT_ID
            .iter()
            .map(|scrambled| {
                let byte = scrambled ^ ((seed & 255) as u8);
                seed = seed.wrapping_mul(69069).wrapping_add(1);
                byte
            })
            .collect()
    }

    #[test]
    fn mount_restriction_matches_fs_setrestrictions() {
        assert_eq!(
            q3_mount_restriction(Q3ProductPolicy::Retail, false),
            Q3MountRestriction::None
        );
        assert_eq!(
            q3_mount_restriction(Q3ProductPolicy::Retail, true),
            Q3MountRestriction::Demo {
                directory: "demota",
                pak_checksum: 437558517
            }
        );
        assert!(matches!(
            q3_mount_restriction(
                Q3ProductPolicy::PrereleaseDemo {
                    team_arena_ui: TeamArenaUi::Retail
                },
                false
            ),
            Q3MountRestriction::Demo { .. }
        ));
        let valid = valid_product_id();
        assert_eq!(
            resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, Some(&valid)).expect("valid"),
            Q3MountRestriction::None
        );
        assert!(matches!(
            resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, None).expect("missing"),
            Q3MountRestriction::Demo { .. }
        ));
        let mut corrupt = valid;
        corrupt[7] ^= 0xff;
        assert!(resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, Some(&corrupt)).is_err());
    }

    #[test]
    fn product_policy_matrix_matches_donor() {
        use qa_core::cmd::Dialect;
        assert_eq!(q3_product_policy(false, false), Q3ProductPolicy::Retail);
        assert_eq!(q3_product_policy(false, true), Q3ProductPolicy::PrereleaseTaDemo);
        let demo_retail = q3_product_policy(true, false);
        assert!(q3_prerelease_demo(demo_retail));
        assert!(!q3_team_arena_demo(demo_retail));
        assert!(q3_team_arena_demo(q3_product_policy(true, true)));
        assert!(q3_team_arena_demo(Q3ProductPolicy::PrereleaseTaDemo));
        assert_eq!(q3_product_map_commands(demo_retail), vec!["map".to_string()]);
        assert_eq!(q3_product_map_commands(Q3ProductPolicy::Retail).len(), 4);
        let catalog = q3_team_arena_catalog_policy(Q3ProductPolicy::Retail);
        assert_eq!(catalog.game_info, "gameinfo.txt");
        assert!(catalog.additional_teams && catalog.additional_arenas);
        let demo_catalog = q3_team_arena_catalog_policy(Q3ProductPolicy::PrereleaseTaDemo);
        assert_eq!(demo_catalog.game_info, "demogameinfo.txt");
        assert!(!demo_catalog.additional_teams && !demo_catalog.additional_arenas);

        let mut cvars = CvarRegistry::new(Dialect::Q3);
        assert_eq!(
            register_q3_product_policy(&mut cvars).expect("policy"),
            Q3ProductPolicy::Retail
        );
        cvars.set("com_prereleaseDemo", "1", true).unwrap();
        // Init cvars latch through registration, so a fresh registry with a
        // preset value resolves the demo policy.
        let mut preset = CvarRegistry::new(Dialect::Q3);
        preset.set("com_prereleaseDemo", "1", true).unwrap();
        assert!(q3_prerelease_demo(
            register_q3_product_policy(&mut preset).expect("demo")
        ));
    }
}
