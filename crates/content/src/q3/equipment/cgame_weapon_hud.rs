//! Cgame weapon HUD profile (`cgame-weapon-hud.ts`).

use qa_guest::qvm::game_data::{QvmArtifact, QvmRole};

/// Ammo branch join inside a status region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmEquipmentAmmoJoin {
    /// Branch entry.
    pub entry: i32,
    /// Branch join.
    pub join: i32,
}

/// One status region entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmEquipmentStatusRegion {
    /// Region entry.
    pub entry: i32,
    /// Visibility decision.
    pub decision: i32,
    /// Whether the branch is taken.
    pub taken: bool,
    /// Ammo branches.
    pub ammo: Vec<QvmEquipmentAmmoJoin>,
}

/// Status-bar source: qualified regions or plain functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmEquipmentStatus {
    /// Qualified regions with ammo branches.
    Regions(Vec<QvmEquipmentStatusRegion>),
    /// Plain function entries.
    Functions(Vec<i32>),
}

/// Low-ammo warning states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmEquipmentWarningStates {
    /// None state.
    pub none: i32,
    /// Low state.
    pub low: i32,
    /// Empty state.
    pub empty: i32,
}

/// Low-ammo warning source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmEquipmentWarning {
    /// Warning entry.
    pub entry: i32,
    /// Warning state address.
    pub state: i32,
    /// Warning states.
    pub states: QvmEquipmentWarningStates,
}

/// Held-weapon source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmEquipmentHeld {
    /// Held entry.
    pub entry: i32,
    /// Gun argument.
    pub gun: i32,
    /// Parent argument word.
    pub parent_argument: i32,
    /// State argument word.
    pub state_argument: i32,
    /// Entity argument word.
    pub entity_argument: i32,
    /// Entity-number offset.
    pub entity_number_offset: i32,
}

/// View-weapon visibility source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmEquipmentView {
    /// View entry.
    pub entry: i32,
    /// Visibility decision.
    pub decision: i32,
    /// Whether the branch is taken.
    pub taken: bool,
}

/// Original HUD entry and final view-weapon visibility decision, qualified
/// per cgame artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmEquipmentPresentationProfile {
    /// HUD entry.
    pub hud: i32,
    /// Warning source.
    pub warning: QvmEquipmentWarning,
    /// Status source.
    pub status: QvmEquipmentStatus,
    /// Held-weapon source.
    pub held: QvmEquipmentHeld,
    /// View-weapon source.
    pub view: QvmEquipmentView,
}

/// Equipment presentation profile for a qualified cgame artifact.
#[must_use]
pub fn q3_equipment_presentation_profile(artifact: &QvmArtifact) -> Option<QvmEquipmentPresentationProfile> {
    if artifact.role != QvmRole::Cgame {
        return None;
    }
    let warning = QvmEquipmentWarningStates {
        none: 0,
        low: 1,
        empty: 2,
    };
    match artifact.module.digest.as_str() {
        "sha256:a4744482c9b93852cc71f4d7ce03b3e4337e5d89844d27d272c2c16d74df07fa" => {
            Some(QvmEquipmentPresentationProfile {
                hud: 108261,
                warning: QvmEquipmentWarning {
                    entry: 22881,
                    state: 1084512,
                    states: warning,
                },
                status: QvmEquipmentStatus::Functions(vec![24534, 24736]),
                held: QvmEquipmentHeld {
                    entry: 106742,
                    gun: 28,
                    parent_argument: 0,
                    state_argument: 1,
                    entity_argument: 2,
                    entity_number_offset: 0,
                },
                view: QvmEquipmentView {
                    entry: 107738,
                    decision: 107828,
                    taken: true,
                },
            })
        }
        "sha256:14858804fb98609ed8b3b3c3b825f0a7cb544063f7e43735c884cd5e4a51157c" => {
            Some(QvmEquipmentPresentationProfile {
                hud: 106815,
                warning: QvmEquipmentWarning {
                    entry: 26123,
                    state: 1012656,
                    states: warning,
                },
                status: QvmEquipmentStatus::Regions(vec![
                    QvmEquipmentStatusRegion {
                        entry: 10083,
                        decision: 10087,
                        taken: true,
                        ammo: vec![
                            QvmEquipmentAmmoJoin {
                                entry: 10141,
                                join: 10228,
                            },
                            QvmEquipmentAmmoJoin {
                                entry: 10402,
                                join: 10525,
                            },
                        ],
                    },
                    QvmEquipmentStatusRegion {
                        entry: 13793,
                        decision: 13797,
                        taken: true,
                        ammo: vec![
                            QvmEquipmentAmmoJoin {
                                entry: 14011,
                                join: 14098,
                            },
                            QvmEquipmentAmmoJoin {
                                entry: 14833,
                                join: 14960,
                            },
                        ],
                    },
                ]),
                held: QvmEquipmentHeld {
                    entry: 103468,
                    gun: 184,
                    parent_argument: 0,
                    state_argument: 1,
                    entity_argument: 2,
                    entity_number_offset: 0,
                },
                view: QvmEquipmentView {
                    entry: 105885,
                    decision: 105975,
                    taken: true,
                },
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::test_support::fixture_artifact;

    #[test]
    fn profiles_both_cgame_artifacts() {
        let first = fixture_artifact(
            "sha256:a4744482c9b93852cc71f4d7ce03b3e4337e5d89844d27d272c2c16d74df07fa",
            QvmRole::Cgame,
            64,
            &[],
        );
        let profile = q3_equipment_presentation_profile(&first).unwrap();
        assert_eq!(profile.hud, 108261);
        assert!(matches!(profile.status, QvmEquipmentStatus::Functions(_)));
        assert_eq!(profile.held.gun, 28);
        let second = fixture_artifact(
            "sha256:14858804fb98609ed8b3b3c3b825f0a7cb544063f7e43735c884cd5e4a51157c",
            QvmRole::Cgame,
            64,
            &[],
        );
        let profile = q3_equipment_presentation_profile(&second).unwrap();
        assert_eq!(profile.hud, 106815);
        let QvmEquipmentStatus::Regions(regions) = &profile.status else {
            panic!("second artifact must use regions");
        };
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].ammo.len(), 2);
    }

    #[test]
    fn rejects_other_roles_and_digests() {
        let qagame = fixture_artifact(
            "sha256:a4744482c9b93852cc71f4d7ce03b3e4337e5d89844d27d272c2c16d74df07fa",
            QvmRole::Qagame,
            64,
            &[],
        );
        assert!(q3_equipment_presentation_profile(&qagame).is_none());
        let other = fixture_artifact("sha256:other", QvmRole::Cgame, 64, &[]);
        assert!(q3_equipment_presentation_profile(&other).is_none());
    }
}
