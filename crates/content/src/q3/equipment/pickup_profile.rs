//! Q3 native pickup profile (`pickup-profile.ts`).

use std::rc::Rc;

use qa_guest::qvm::game_data::{AbiProfile, QvmArtifact, QvmRegionEvaluation};
use qa_guest::qvm::game_pickups::{
    QvmGrantEligibility, QvmGrantOperation, QvmGrantWeapon, QvmGrantWeaponStorage, QvmPickupEligibility,
    QvmPickupFields, QvmPickupGate, QvmPickupGrant, QvmPickupProfile, QvmPickupTargets,
};
use qa_guest::qvm::item_catalog::{QvmItemAddress, QvmItemCount, QvmItemFields, QvmItemLayout};

use super::threewave_grapple_profile::THREEWAVE_GRAPPLE_DIGEST;

/// Stock Q3 qagame digest.
pub const STOCK_Q3_PICKUP_DIGEST: &str = "sha256:57c52bf22e4f528c064f8af1553a7103723bab0a02276bb11eed944bf829b219";

/// Non-resource eligibility from Threewave `BG_CanItemBeGrabbed`; the
/// selected owner decides caps and tiers.
fn threewave_dropped_owner(lithium: bool) -> QvmGrantEligibility {
    Rc::new(move |eligibility: QvmPickupEligibility<'_>| {
        let mode = eligibility.call.argument(0)?;
        let alternate = lithium && eligibility.call.argument(3)? != 0;
        Ok((mode != 10 && !alternate)
            || eligibility.item.get_i32(164)? != 2
            || eligibility.item.get_i32(168)? != eligibility.player.get_i32(140)?)
    })
}

fn always_eligible() -> QvmGrantEligibility {
    Rc::new(|_| Ok(true))
}

/// Entry and caller indices are qualified only for these immutable original
/// executables.
#[must_use]
pub fn q3_native_pickup_profile(artifact: &QvmArtifact) -> Option<QvmPickupProfile> {
    let fields = QvmPickupFields {
        inuse: 520,
        client: 516,
        health: 732,
        item: 804,
        count: 760,
        flags: 536,
    };
    match artifact.module.digest.as_str() {
        digest if digest == STOCK_Q3_PICKUP_DIGEST => Some(QvmPickupProfile {
            module: artifact.module.clone(),
            abi_profile: AbiProfile::Modern,
            entity_stride: 808,
            client_stride: 776,
            fields,
            dropped_flag: 4096,
            items: QvmItemLayout {
                address: QvmItemAddress::Direct(2552),
                count: QvmItemCount::Direct(36),
                live_source: false,
                stride: 52,
                fields: QvmItemFields {
                    class_name: 0,
                    pickup_name: 28,
                    item_type: 36,
                    tag: 40,
                },
                weapon_type: 1,
                ammo_type: 2,
            },
            touch: 103974,
            gate: QvmPickupGate {
                entry: 65953,
                calls: vec![104007],
                item_argument: 1,
                player_argument: 2,
            },
            targets: QvmPickupTargets {
                entry: 129114,
                calls: vec![104340],
            },
            free: 129805,
            objective_types: vec![8],
            grants: vec![
                QvmPickupGrant {
                    item_type: 3,
                    entry: 103594,
                    calls: vec![104107],
                    operation: QvmGrantOperation::Return {
                        accepted_return: 103658,
                    },
                    eligible: always_eligible(),
                },
                QvmPickupGrant {
                    item_type: 2,
                    entry: 103220,
                    calls: vec![104091],
                    operation: QvmGrantOperation::Return {
                        accepted_return: 103265,
                    },
                    eligible: always_eligible(),
                },
            ],
        }),
        digest if digest == THREEWAVE_GRAPPLE_DIGEST => Some(QvmPickupProfile {
            module: artifact.module.clone(),
            abi_profile: AbiProfile::Modern,
            entity_stride: 876,
            client_stride: 944,
            fields,
            dropped_flag: 4096,
            items: QvmItemLayout {
                address: QvmItemAddress::Direct(5304),
                count: QvmItemCount::Direct(50),
                live_source: false,
                stride: 52,
                fields: QvmItemFields {
                    class_name: 0,
                    pickup_name: 28,
                    item_type: 36,
                    tag: 40,
                },
                weapon_type: 1,
                ammo_type: 2,
            },
            touch: 168574,
            gate: QvmPickupGate {
                entry: 20482,
                calls: vec![168697, 168873],
                item_argument: 1,
                player_argument: 2,
            },
            targets: QvmPickupTargets {
                entry: 210519,
                calls: vec![169404],
            },
            free: 211210,
            objective_types: vec![8],
            grants: vec![
                QvmPickupGrant {
                    item_type: 3,
                    entry: 167408,
                    calls: vec![169094],
                    operation: QvmGrantOperation::Return {
                        accepted_return: 167814,
                    },
                    eligible: threewave_dropped_owner(false),
                },
                QvmPickupGrant {
                    item_type: 2,
                    entry: 166848,
                    calls: vec![169078],
                    operation: QvmGrantOperation::Region {
                        entry: 166875,
                        join: 166893,
                        quantity: 20,
                        weapon: None,
                    },
                    eligible: threewave_dropped_owner(true),
                },
                QvmPickupGrant {
                    item_type: 1,
                    entry: 166897,
                    calls: vec![169062],
                    operation: QvmGrantOperation::Region {
                        entry: 167099,
                        join: 167173,
                        quantity: 20,
                        weapon: Some(QvmGrantWeapon {
                            quantity: QvmRegionEvaluation {
                                entry: 166958,
                                join: 167099,
                                inputs: Vec::new(),
                                result: Some(20),
                            },
                            storage: QvmGrantWeaponStorage::Words {
                                bits_offset: 204,
                                ammo_offset: 376,
                            },
                        }),
                    },
                    eligible: threewave_dropped_owner(true),
                },
            ],
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use qa_guest::error::GuestError;
    use qa_guest::qvm::game_data::{QvmFunctionCall, QvmMemoryWindow, QvmRole, QvmSharedMemory};

    use super::*;
    use crate::q3::test_support::fixture_artifact;

    fn eligibility(
        call: &mut QvmFunctionCall,
        item: QvmMemoryWindow,
        player: QvmMemoryWindow,
    ) -> QvmPickupEligibility<'_> {
        QvmPickupEligibility { call, item, player }
    }

    #[test]
    fn profiles_stock_and_threewave() {
        let stock = fixture_artifact(STOCK_Q3_PICKUP_DIGEST, QvmRole::Qagame, 64, &[]);
        let profile = q3_native_pickup_profile(&stock).unwrap();
        assert_eq!((profile.entity_stride, profile.client_stride), (808, 776));
        assert_eq!(profile.grants.len(), 2);
        assert!(matches!(
            profile.grants[0].operation,
            QvmGrantOperation::Return {
                accepted_return: 103658
            }
        ));
        let threewave = fixture_artifact(THREEWAVE_GRAPPLE_DIGEST, QvmRole::Qagame, 64, &[]);
        let profile = q3_native_pickup_profile(&threewave).unwrap();
        assert_eq!((profile.entity_stride, profile.client_stride), (876, 944));
        assert_eq!(profile.grants.len(), 3);
        let QvmGrantOperation::Region { weapon, .. } = &profile.grants[2].operation else {
            panic!("weapon grant must use a region with storage");
        };
        let weapon = weapon.as_ref().unwrap();
        assert!(matches!(
            weapon.storage,
            QvmGrantWeaponStorage::Words {
                bits_offset: 204,
                ammo_offset: 376
            }
        ));
        let other = fixture_artifact("sha256:other", QvmRole::Qagame, 64, &[]);
        assert!(q3_native_pickup_profile(&other).is_none());
    }

    #[test]
    fn threewave_eligibility_follows_dropped_ownership() -> Result<(), GuestError> {
        let memory = QvmSharedMemory::new(1024)?;
        let item = QvmMemoryWindow::new(memory.clone(), 0, 512)?;
        let player = QvmMemoryWindow::new(memory.clone(), 512, 512)?;
        item.set_i32(164, 2)?;
        item.set_i32(168, 7)?;
        player.set_i32(140, 7)?;
        let eligible = threewave_dropped_owner(false);
        let mut foreign_mode = QvmFunctionCall::entered(0, vec![0, 0, 0, 0], memory.clone());
        assert!(eligible(eligibility(&mut foreign_mode, item.clone(), player.clone()))?);
        let mut owned = QvmFunctionCall::entered(0, vec![10, 0, 0, 0], memory.clone());
        assert!(!eligible(eligibility(&mut owned, item.clone(), player.clone()))?);
        player.set_i32(140, 9)?;
        let mut stolen = QvmFunctionCall::entered(0, vec![10, 0, 0, 0], memory.clone());
        assert!(eligible(eligibility(&mut stolen, item.clone(), player.clone()))?);
        player.set_i32(140, 7)?;
        let lithium = threewave_dropped_owner(true);
        let mut lithium_mode = QvmFunctionCall::entered(0, vec![10, 0, 0, 1], memory.clone());
        assert!(!lithium(eligibility(&mut lithium_mode, item, player))?);
        Ok(())
    }
}
