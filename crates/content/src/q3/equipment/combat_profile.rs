//! Q3 native combat profile (`src/content/q3/equipment/combat-profile.ts`).

use qa_guest::error::GuestError;
use qa_guest::qvm::game_combat::{
    QvmArmorTierGuard, QvmArmorTierValue, QvmArmorTiers, QvmCombatCall, QvmCombatCallbacks, QvmCombatMass,
    QvmCombatTeam, QvmDamageFlags, QvmGameArmorDefinition, QvmReactionCall,
};
use qa_guest::qvm::game_combat_binding::{
    QvmPrimaryCombatFields, QvmPrimaryCombatFlags, QvmPrimaryCombatProfile, QvmPrimaryCombatReactions,
    QvmPrimaryCombatState, QvmPrimaryCombatTeam,
};
use qa_guest::qvm::game_data::QvmArtifact;

use super::grapple_profiles::q3_grapple_profile;
use super::lrctf_grapple_profile::LRCTF_GRAPPLE_DIGEST;
use super::threewave_grapple_profile::THREEWAVE_GRAPPLE_DIGEST;

fn armor_call() -> QvmCombatCall {
    QvmCombatCall {
        roles: vec![
            ("target".to_string(), 0),
            ("amount".to_string(), 1),
            ("flags".to_string(), 2),
        ],
        extras: Vec::new(),
    }
}

fn lrctf_armor() -> QvmGameArmorDefinition {
    QvmGameArmorDefinition {
        check_armor: 139839,
        call: armor_call(),
        points_stat: 3,
        protection: 0.66,
        tiers: None,
    }
}

fn threewave_armor() -> QvmGameArmorDefinition {
    QvmGameArmorDefinition {
        check_armor: 161690,
        call: armor_call(),
        points_stat: 6,
        protection: 0.66,
        tiers: Some(QvmArmorTiers {
            stat: 3,
            when_any: vec![
                QvmArmorTierGuard {
                    offset: 107944,
                    equal: true,
                    value: 10,
                },
                QvmArmorTierGuard {
                    offset: 1089728,
                    equal: false,
                    value: 0,
                },
            ],
            values: vec![
                QvmArmorTierValue {
                    tier: 0,
                    protection: 0.3,
                },
                QvmArmorTierValue {
                    tier: 1,
                    protection: 0.6,
                },
                QvmArmorTierValue {
                    tier: 2,
                    protection: 0.8,
                },
            ],
            fallback: 0.3,
        }),
    }
}

/// `G_Damage`'s actual flags, pain and die fields in each admitted qagame.
pub fn q3_native_combat_profile(artifact: &QvmArtifact) -> Result<Option<QvmPrimaryCombatProfile>, GuestError> {
    let Some(source) = q3_grapple_profile(artifact)? else {
        return Ok(None);
    };
    let damage_call = QvmCombatCall {
        roles: vec![
            ("target".to_string(), 0),
            ("inflictor".to_string(), 1),
            ("attacker".to_string(), 2),
            ("direction".to_string(), 3),
            ("point".to_string(), 4),
            ("amount".to_string(), 5),
            ("flags".to_string(), 6),
            ("method".to_string(), 7),
        ],
        extras: Vec::new(),
    };
    let state = QvmPrimaryCombatState {
        health_stat: 0,
        team: QvmPrimaryCombatTeam {
            persistent_stat: 3,
            values: vec![
                QvmCombatTeam {
                    value: 1,
                    team: "q3:1".to_string(),
                },
                QvmCombatTeam {
                    value: 2,
                    team: "q3:2".to_string(),
                },
            ],
        },
        flags: QvmPrimaryCombatFlags {
            notarget: 32,
            invulnerable: 16,
            no_knockback: 2048,
        },
        mass: QvmCombatMass::Constant { value: 200.0 },
    };
    let damage_flags = QvmDamageFlags {
        radius: 1,
        no_armor: 2,
        no_knockback: 4,
        no_protection: 8,
        no_team_protection: 16,
    };
    let pain_call = QvmReactionCall {
        arguments: 3,
        target: 0,
        amount: 2,
    };
    let die_call = QvmReactionCall {
        arguments: 5,
        target: 0,
        amount: 3,
    };
    let (armor, pain, die) = match artifact.module.digest.as_str() {
        digest if digest == LRCTF_GRAPPLE_DIGEST => (lrctf_armor(), 728, 732),
        digest if digest == THREEWAVE_GRAPPLE_DIGEST => (threewave_armor(), 712, 716),
        _ => return Ok(None),
    };
    Ok(Some(QvmPrimaryCombatProfile {
        damage_call,
        module: source.module.clone(),
        abi_profile: source.abi_profile,
        entity_stride: source.entity_stride,
        client_stride: source.client_stride,
        fields: QvmPrimaryCombatFields {
            inuse: source.fields.inuse,
            health: source.fields.health,
            takedamage: source.fields.takedamage,
            parent: source.fields.parent,
            client: source.fields.client,
        },
        callbacks: QvmCombatCallbacks {
            allocate: source.callbacks.allocate,
            free: source.callbacks.free,
            damage: source.callbacks.damage,
        },
        armor,
        reactions: QvmPrimaryCombatReactions {
            flags: 536,
            pain,
            die,
            pain_call,
            die_call,
        },
        grapple_damage_method: source.grapple_damage_method as i32,
        state,
        damage_flags,
    }))
}

#[cfg(test)]
mod tests {
    use qa_guest::qvm::game_data::QvmRole;

    use super::*;
    use crate::q3::test_support::fixture_artifact;

    fn lrctf_artifact() -> QvmArtifact {
        fixture_artifact(
            LRCTF_GRAPPLE_DIGEST,
            QvmRole::Qagame,
            1_008_984,
            &[
                178797, 179014, 183076, 183171, 151164, 183577, 5362, 15874, 184569, 140358, 169306, 22369,
            ],
        )
    }

    fn threewave_artifact() -> QvmArtifact {
        fixture_artifact(
            THREEWAVE_GRAPPLE_DIGEST,
            QvmRole::Qagame,
            1_091_864,
            &[
                210993, 211210, 217563, 215035, 215169, 177663, 16897, 29990, 162405, 197341, 35535,
            ],
        )
    }

    #[test]
    fn lrctf_combat_has_flat_armor() {
        let profile = q3_native_combat_profile(&lrctf_artifact()).unwrap().unwrap();
        assert_eq!((profile.entity_stride, profile.client_stride), (856, 872));
        assert_eq!(profile.fields.client, 516);
        assert_eq!(profile.callbacks.damage, 140358);
        assert_eq!(profile.armor.check_armor, 139839);
        assert_eq!(profile.armor.tiers, None);
        assert_eq!(
            (profile.reactions.flags, profile.reactions.pain, profile.reactions.die),
            (536, 728, 732)
        );
        assert_eq!(profile.grapple_damage_method, 23);
        assert_eq!(profile.state.health_stat, 0);
        assert_eq!(profile.damage_flags.no_team_protection, 16);
    }

    #[test]
    fn threewave_combat_has_tiered_armor() {
        let profile = q3_native_combat_profile(&threewave_artifact()).unwrap().unwrap();
        assert_eq!(profile.armor.check_armor, 161690);
        let tiers = profile.armor.tiers.unwrap();
        assert_eq!(tiers.stat, 3);
        assert_eq!(tiers.values.len(), 3);
        assert_eq!(tiers.fallback, 0.3);
        assert_eq!((profile.reactions.pain, profile.reactions.die), (712, 716));
        assert_eq!(profile.grapple_damage_method, 29);
    }

    #[test]
    fn rejects_unqualified_artifacts() {
        let other = fixture_artifact("sha256:other", QvmRole::Qagame, 64, &[]);
        assert!(q3_native_combat_profile(&other).unwrap().is_none());
    }
}
