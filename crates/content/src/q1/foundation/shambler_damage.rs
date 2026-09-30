//! Foreign shambler damage adjustment
//! (`src/content/q1/foundation/shambler-damage.ts`).

use super::entity::Q1MonsterSpecies;
use super::gameplay::{AttackCause, DamageDelivery, DamageRequest};
use super::host::DamageAdjust;

/// Q1's rocket impact and radius routines already halve their own
/// shambler damage (`foreignShamblerDamage`).
#[must_use]
pub fn foreign_shambler_damage(species: Option<Q1MonsterSpecies>, request: &DamageRequest) -> Option<DamageAdjust> {
    let cause = &request.attack.cause;
    match cause {
        AttackCause::Q1 { .. } | AttackCause::Environment { .. } => return None,
        _ => {}
    }
    if species != Some(Q1MonsterSpecies::Shambler) {
        return None;
    }
    let rocket = match cause {
        AttackCause::Q2 { means_of_death, .. } => *means_of_death == 8,
        AttackCause::Q3 { means_of_death, .. } => *means_of_death == 6,
        AttackCause::Q1 { .. } | AttackCause::Environment { .. } => false,
    };
    if request.delivery != DamageDelivery::Radius && !rocket {
        return None;
    }
    Some(DamageAdjust {
        amount: f64::from((request.amount * 0.5) as f32),
        knockback: f64::from((request.knockback * 0.5) as f32),
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_core::math::Vec3;
    use qa_core::time::SourceTime;

    use super::super::gameplay::AttackProvenance;
    use super::*;

    fn request(cause: AttackCause, delivery: DamageDelivery) -> DamageRequest {
        let provider = ProviderId::new("q1", "test");
        let owner = qa_core::identity::IdentityOwner::create("test").expect("owner");
        DamageRequest {
            attack: AttackProvenance {
                sequence: 0,
                time: SourceTime::Seconds(0.0),
                attacker: None,
                inflictor: None,
                originating_projectile: None,
                weapon: None,
                weapon_provider: provider.clone(),
                damage_powerup_owner: None,
                combat_provider: provider.clone(),
                inventory_provider: provider.clone(),
                movement_provider: provider,
                cause,
            },
            target: owner.actor(1, 0),
            amount: 100.0,
            knockback: 100.0,
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            point: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            delivery,
        }
    }

    #[test]
    fn halves_foreign_radius_damage_only() {
        let q2_rocket = AttackCause::Q2 {
            means_of_death: 8,
            damage_flags: 0,
            native: None,
        };
        let adjusted = foreign_shambler_damage(
            Some(Q1MonsterSpecies::Shambler),
            &request(q2_rocket, DamageDelivery::Direct),
        )
        .expect("adjusted");
        assert_eq!(adjusted.amount, 50.0);
        let q1 = AttackCause::Q1 {
            death_type: String::new(),
            armor_effect: None,
        };
        assert_eq!(
            foreign_shambler_damage(Some(Q1MonsterSpecies::Shambler), &request(q1, DamageDelivery::Radius)),
            None
        );
        assert_eq!(
            foreign_shambler_damage(
                Some(Q1MonsterSpecies::Army),
                &request(
                    AttackCause::Q2 {
                        means_of_death: 8,
                        damage_flags: 0,
                        native: None
                    },
                    DamageDelivery::Radius
                )
            ),
            None
        );
    }
}
