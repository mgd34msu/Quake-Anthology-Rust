//! Quake III foundation: weapon behavior.
//!
//! Donor provenance: `src/content/q3/foundation/weapon-behavior.ts`.

use crate::contract::{ItemId, ProjectileRole};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::arsenal::*;
use qa_world::movement::q3::constants::weapon;
use thiserror::Error;

// ---------------------------------------------------------------------------
// weapon-behavior.ts: projectile roles.
// ---------------------------------------------------------------------------

/// Weapon behavior failure (donor `Error` throws).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WeaponBehaviorError {
    /// Operation failure (donor `Error`).
    #[error("{0}")]
    Failed(String),
}

fn failed(message: impl Into<String>) -> WeaponBehaviorError {
    WeaponBehaviorError::Failed(message.into())
}

/// Projectile behavior (`q3ProjectileBehavior` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ProjectileBehavior {
    /// Weapon item.
    pub weapon: ItemId,
    /// Projectile role.
    pub role: ProjectileRole,
}

/// Projectile behavior for a weapon (`q3ProjectileBehavior`).
pub fn q3_projectile_behavior(weapon: i32) -> Result<Q3ProjectileBehavior, WeaponBehaviorError> {
    let item = q3_weapon_item(weapon).ok_or_else(|| failed(format!("Unknown Q3 projectile weapon {weapon}")))?;
    let role = if weapon == weapon::GRENADE_LAUNCHER || weapon == weapon::PROX_LAUNCHER {
        ProjectileRole::Grenade
    } else if weapon == weapon::ROCKET_LAUNCHER {
        ProjectileRole::Rocket
    } else if weapon == weapon::PLASMAGUN {
        ProjectileRole::Plasma
    } else if weapon == weapon::BFG {
        ProjectileRole::Energy
    } else if weapon == weapon::GRAPPLING_HOOK {
        ProjectileRole::Grapple
    } else if weapon == weapon::NAILGUN {
        ProjectileRole::Nail
    } else {
        return Err(failed(format!("Q3 weapon {weapon} does not launch a projectile")));
    };
    Ok(Q3ProjectileBehavior {
        weapon: item.item.to_string(),
        role,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projectile_behavior_maps_roles() {
        assert_eq!(
            q3_projectile_behavior(weapon::ROCKET_LAUNCHER).unwrap(),
            Q3ProjectileBehavior {
                weapon: "q3:weapon/rocketlauncher".to_string(),
                role: ProjectileRole::Rocket,
            }
        );
        assert_eq!(
            q3_projectile_behavior(weapon::PROX_LAUNCHER).unwrap().role,
            ProjectileRole::Grenade
        );
        assert_eq!(
            q3_projectile_behavior(weapon::NAILGUN).unwrap().role,
            ProjectileRole::Nail
        );
        assert!(q3_projectile_behavior(99).is_err());
        assert!(q3_projectile_behavior(weapon::MACHINEGUN).is_err());
    }
}
