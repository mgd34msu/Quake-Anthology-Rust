//! Quake III root: product restriction.
//!
//! Donor provenance: `src/content/q3/product-restriction.ts`.

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
