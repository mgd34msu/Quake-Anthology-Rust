//! Q3 primary weapon profile (`src/content/q3/equipment/weapon-profile.ts`).

use qa_guest::qvm::game_data::{AbiProfile, QvmArtifact, QvmRegionEvaluation};
use qa_guest::qvm::game_equipment_movement::{QvmBodyTrace, QvmEquipmentMovementProfile, QvmLocomotion};
use qa_guest::qvm::game_weapons::{
    QvmAvailability, QvmDamageFactor, QvmDelayPlayer, QvmDropAmmo, QvmDropHook, QvmEquipmentContext, QvmGiveHook,
    QvmGiveNamed, QvmPowerupOffsets, QvmPrimaryMatch, QvmPrimaryWeaponProfile, QvmTeleport, QvmTorsoAnimation,
    QvmWaterLevel, QvmWeaponRegion,
};
use qa_guest::qvm::mod_provider::{InputPointerKind, QvmModInputPointer};
use qa_guest::qvm::mod_weapon_stage::{
    DispatcherHead, QvmItemField, QvmItemTest, QvmWeaponActor, QvmWeaponDispatcherDefinition, SelectionValue,
    StagePredicate, StageRequest, StageSelection, TestComparison,
};

use super::threewave_grapple_profile::THREEWAVE_GRAPPLE_DIGEST;
use crate::q3::guest_items::Q3GuestWeapon;

/// Threewave team command (donor `match.teams`). The guest match declaration
/// carries the score word only; these rows reunite with it when the team
/// command consumer is ported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3MatchTeamCommand {
    /// Source team identity, if any.
    pub source: Option<String>,
    /// Shared team identity, if any.
    pub team: Option<String>,
    /// Team command arguments.
    pub arguments: Vec<String>,
}

/// Threewave team commands.
#[must_use]
pub fn threewave_match_teams() -> Vec<Q3MatchTeamCommand> {
    vec![
        Q3MatchTeamCommand {
            source: Some("q3:1".to_string()),
            team: Some("team:red".to_string()),
            arguments: vec!["team".to_string(), "red".to_string()],
        },
        Q3MatchTeamCommand {
            source: Some("q3:2".to_string()),
            team: Some("team:blue".to_string()),
            arguments: vec!["team".to_string(), "blue".to_string()],
        },
        Q3MatchTeamCommand {
            source: None,
            team: None,
            arguments: vec!["team".to_string(), "free".to_string()],
        },
    ]
}

fn client_field(offset: usize) -> QvmItemField {
    QvmItemField {
        record: "client".to_string(),
        offset,
    }
}

/// Threewave `PM_Weapon`'s attack decision runs after its original cooldown
/// and change traversal.
#[must_use]
pub fn q3_primary_weapon_profile(artifact: &QvmArtifact, weapons: &[Q3GuestWeapon]) -> Option<QvmPrimaryWeaponProfile> {
    if artifact.module.digest != THREEWAVE_GRAPPLE_DIGEST {
        return None;
    }
    Some(QvmPrimaryWeaponProfile {
        module: artifact.module.clone(),
        match_declaration: Some(QvmPrimaryMatch { score: 248 }),
        abi_profile: AbiProfile::Modern,
        equipment_movement: QvmEquipmentMovementProfile {
            move_entry: 35535,
            slice: 34707,
            duck: 32561,
            movement_global: 1091860,
            locomotion: QvmLocomotion {
                entry: 35397,
                join: 35503,
            },
            mins: 180,
            maxs: 192,
            body_trace: Some(QvmBodyTrace {
                callback: 224,
                mask: 28,
            }),
        },
        entity_stride: 876,
        client_stride: 944,
        client_pointer: 516,
        stage: QvmWeaponDispatcherDefinition {
            dispatcher: DispatcherHead {
                entry: 33648,
                actor: QvmWeaponActor {
                    record: "client".to_string(),
                    pointer: QvmModInputPointer {
                        kind: InputPointerKind::Global { address: 1091860 },
                        indirections: vec![0],
                        offset: 0,
                    },
                },
            },
            predicates: vec![StagePredicate {
                instruction: 34044,
                unselected: false,
            }],
            settled: vec![
                QvmItemTest {
                    field: client_field(44),
                    mask: None,
                    comparison: TestComparison::AtMost,
                    value: 0,
                },
                QvmItemTest {
                    field: client_field(148),
                    mask: None,
                    comparison: TestComparison::Equals,
                    value: 0,
                },
            ],
            selection: StageSelection {
                field: client_field(144),
                values: weapons
                    .iter()
                    .map(|weapon| SelectionValue {
                        value: weapon.weapon,
                        item: weapon.item.clone(),
                    })
                    .collect(),
            },
            request: StageRequest {
                entry: 33226,
                argument: 0,
                accepted: vec![QvmItemTest {
                    field: client_field(148),
                    mask: None,
                    comparison: TestComparison::Equals,
                    value: 2,
                }],
            },
        },
        damage_factor: QvmDamageFactor {
            entry: 217003,
            result: 1616724,
            stop: QvmWeaponRegion {
                entry: 217081,
                join: 217113,
            },
        },
        equipment_contexts: vec![QvmEquipmentContext {
            provider: "q2:equipment/hand-grenades".to_string(),
            item: None,
        }],
        delay: QvmRegionEvaluation {
            entry: 34318,
            join: 34350,
            inputs: vec![12],
            result: Some(12),
        },
        delay_player: QvmDelayPlayer {
            movement_global: 1091860,
            player_offset: 0,
        },
        teleport: QvmTeleport {
            entry: 118339,
            region: QvmRegionEvaluation {
                entry: 118552,
                join: 118726,
                inputs: Vec::new(),
                result: None,
            },
            objectives: QvmRegionEvaluation {
                entry: 118552,
                join: 118701,
                inputs: Vec::new(),
                result: None,
            },
            spawn: 127849,
            view: 128463,
        },
        max_health: 220,
        persistent_max_health: 548,
        availability: QvmAvailability {
            movement_type: 4,
            excluded: vec![1, 2, 4, 7, 8],
            health: 184,
            team: 260,
            spectator_team: 3,
            flags: 12,
            respawn_flag: 512,
        },
        powerups: QvmPowerupOffsets {
            quad: 312,
            haste: 320,
            flight: 332,
        },
        torso_animation: QvmTorsoAnimation {
            entry: 27646,
            attack: 7,
            melee: 8,
        },
        water_level: QvmWaterLevel {
            entity_offset: 788,
            movement_offset: 208,
        },
        drop: QvmDropHook {
            entry: 158347,
            argument: 0,
            weapon: 192,
            ammo: QvmDropAmmo::Offset(376),
            region: QvmWeaponRegion {
                entry: 158571,
                join: 158674,
            },
        },
        give: QvmGiveHook {
            entry: 139391,
            argument: 0,
            weapons: 139524,
            ammo: 139574,
            named: QvmGiveNamed {
                entry: 139936,
                join: 139947,
                name: 24,
                item: 36,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use qa_guest::qvm::game_data::QvmRole;

    use super::*;
    use crate::q3::guest_items::BASE_Q3_GUEST_WEAPONS;
    use crate::q3::test_support::fixture_artifact;

    #[test]
    fn profiles_threewave_with_guest_selection() {
        let artifact = fixture_artifact(THREEWAVE_GRAPPLE_DIGEST, QvmRole::Qagame, 64, &[]);
        let profile = q3_primary_weapon_profile(&artifact, &BASE_Q3_GUEST_WEAPONS).unwrap();
        assert_eq!(
            (profile.entity_stride, profile.client_stride, profile.client_pointer),
            (876, 944, 516)
        );
        assert_eq!(profile.match_declaration.unwrap().score, 248);
        assert_eq!(profile.stage.selection.values.len(), 10);
        assert_eq!(profile.stage.selection.values[0].value, 1);
        assert_eq!(profile.equipment_movement.move_entry, 35535);
        assert_eq!(profile.availability.excluded, vec![1, 2, 4, 7, 8]);
        assert!(matches!(profile.drop.ammo, QvmDropAmmo::Offset(376)));
        assert_eq!(threewave_match_teams().len(), 3);
    }

    #[test]
    fn rejects_other_digests() {
        let other = fixture_artifact("sha256:other", QvmRole::Qagame, 64, &[]);
        assert!(q3_primary_weapon_profile(&other, &[]).is_none());
    }
}
