//! Quake III foundation: weapon behavior.
//!
//! Donor provenance: `src/content/q3/foundation/weapon-behavior.ts`.

use crate::contract::{ItemId, ProjectileRole};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::arsenal_mirror::*;
use crate::q3::foundation::mirrors::*;

// ---------------------------------------------------------------------------
// weapon-behavior.ts: projectile roles.
// ---------------------------------------------------------------------------

/// Projectile behavior (`q3ProjectileBehavior` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ProjectileBehavior {
    /// Weapon item.
    pub weapon: ItemId,
    /// Projectile role.
    pub role: ProjectileRole,
}

/// Projectile behavior for a weapon (`q3ProjectileBehavior`).
pub fn q3_projectile_behavior(weapon: i32) -> Result<Q3ProjectileBehavior, Q3FoundationError> {
    let item = q3_weapon_item(weapon).ok_or_else(|| failed(format!("Unknown Q3 projectile weapon {weapon}")))?;
    let role = if weapon == Q3Weapon::GRENADE_LAUNCHER || weapon == Q3Weapon::PROX_LAUNCHER {
        ProjectileRole::Grenade
    } else if weapon == Q3Weapon::ROCKET_LAUNCHER {
        ProjectileRole::Rocket
    } else if weapon == Q3Weapon::PLASMAGUN {
        ProjectileRole::Plasma
    } else if weapon == Q3Weapon::BFG {
        ProjectileRole::Energy
    } else if weapon == Q3Weapon::GRAPPLING_HOOK {
        ProjectileRole::Grapple
    } else if weapon == Q3Weapon::NAILGUN {
        ProjectileRole::Nail
    } else {
        return Err(failed(format!("Q3 weapon {weapon} does not launch a projectile")));
    };
    Ok(Q3ProjectileBehavior {
        weapon: item.item.to_string(),
        role,
    })
}
