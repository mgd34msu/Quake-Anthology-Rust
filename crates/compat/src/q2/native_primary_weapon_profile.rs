//! Port of `src/compat/q2/native-primary-weapon-profile.ts`.
//! Bridges builtin weapon profiles: Xatrix classic and retail rerelease tables.

use qa_guest::core::contracts::NativeAbi;

use super::native_primary_reader::{
    NativeItemField, NativeItemTest, NativeRegion, NativeScalar, RecordKind, TestComparison, CLASSIC_DIGEST,
    RETAIL_DIGEST,
};
use super::native_primary_weapons::{
    AttackAnimation, DamageResult, DecisionField, DelayEvaluate, EquipmentContext, NativePrimaryWeaponProfile,
    ProjectionWrite, SpawnGate, WeaponAnimation, WeaponClient, WeaponDamage, WeaponDecision, WeaponDelay,
    WeaponDispatcher, WeaponEntity, WeaponTime,
};

fn field(record: RecordKind, offset: u32, encoding: NativeScalar) -> NativeItemField {
    NativeItemField {
        record,
        offset,
        encoding,
    }
}

fn scalar_test(record: RecordKind, offset: u32, encoding: NativeScalar, value: f64) -> NativeItemTest {
    NativeItemTest::Scalar {
        field: field(record, offset, encoding),
        mask: None,
        comparison: TestComparison::Equals,
        value,
    }
}

fn decided(entry: u32, join: u32, fields: Vec<NativeItemField>) -> WeaponDecision {
    WeaponDecision {
        entry,
        join,
        fields: fields
            .into_iter()
            .map(|field| DecisionField { field, clear_mask: 1 })
            .collect(),
    }
}

fn xatrix_profile() -> NativePrimaryWeaponProfile {
    let buttons = field(RecordKind::Client, 0xdcc, NativeScalar::Int32);
    let latched = field(RecordKind::Client, 0xdd4, NativeScalar::Int32);
    NativePrimaryWeaponProfile {
        digest: CLASSIC_DIGEST.to_string(),
        abi: NativeAbi::WindowsI386,
        equipment_contexts: vec![EquipmentContext {
            provider: "q2:equipment/hand-grenades".to_string(),
            item: Some("q2:ammo_grenades".to_string()),
        }],
        dispatcher: WeaponDispatcher {
            entry_rva: 0x36710,
            record: RecordKind::Entity,
            argument: 0,
            arguments: 1,
        },
        decisions: vec![
            decided(0x36c96, 0x36ca6, vec![buttons, latched]),
            decided(0x370bd, 0x370ce, vec![buttons, latched]),
            decided(0x3927d, 0x3928e, vec![buttons, latched]),
            decided(0x378f2, 0x378f9, vec![buttons]),
            decided(0x37b28, 0x37b2f, vec![buttons]),
            decided(0x37f6d, 0x37f74, vec![buttons]),
            decided(0x37f97, 0x37f9e, vec![buttons]),
        ],
        committed_input: vec![],
        spawn: SpawnGate {
            entry: 0x319d0,
            accepted: vec![],
        },
        active: vec![
            scalar_test(RecordKind::Image, 0x768c8, NativeScalar::Float32, 0.0),
            scalar_test(RecordKind::Client, 0xd98, NativeScalar::Int32, 0.0),
            scalar_test(RecordKind::Entity, 0x1ec, NativeScalar::Int32, 0.0),
        ],
        continuations: vec![vec![scalar_test(RecordKind::Client, 0xe00, NativeScalar::Int32, 3.0)]],
        time: WeaponTime {
            address: 0x76804,
            encoding: NativeScalar::Float32,
            milliseconds: 1000.0,
        },
        entity: WeaponEntity {
            client: 84,
            water_level: field(RecordKind::Entity, 0x264, NativeScalar::Int32),
            view_height: field(RecordKind::Entity, 0x1fc, NativeScalar::Int32),
            max_health: field(RecordKind::Entity, 0x1e4, NativeScalar::Int32),
        },
        client: WeaponClient {
            byte_length: 3832,
            view_angles: 0xe44,
            buttons,
            latched_buttons: latched,
        },
        attack_animation: AttackAnimation {
            entry: 0x36b60,
            skip: vec![
                NativeRegion {
                    entry: 0x36b6b,
                    join: 0x36be5,
                },
                NativeRegion {
                    entry: 0x36bea,
                    join: 0x36de6,
                },
                NativeRegion {
                    entry: 0x36d31,
                    join: 0x36dcc,
                },
            ],
        },
        animation: WeaponAnimation {
            frame: field(RecordKind::Entity, 56, NativeScalar::Int32),
            end: field(RecordKind::Client, 0xe7c, NativeScalar::Int32),
            priority: field(RecordKind::Client, 0xe80, NativeScalar::Int32),
            duck: field(RecordKind::Client, 0xe84, NativeScalar::Int32),
            run: field(RecordKind::Client, 0xe88, NativeScalar::Int32),
        },
        delay: WeaponDelay {
            flag: field(RecordKind::Image, 0x6b694, NativeScalar::Int32),
            region: NativeRegion {
                entry: 0x3676f,
                join: 0x36793,
            },
            evaluate: DelayEvaluate::SourceFlag {
                factors: vec![1.0, 0.5],
            },
        },
        damage: WeaponDamage::SourceFlag {
            address: 0x6b690,
            encoding: NativeScalar::Int32,
            factors: vec![1.0, 4.0],
            region: NativeRegion {
                entry: 0x36748,
                join: 0x3676f,
            },
        },
    }
}

fn retail_profile() -> NativePrimaryWeaponProfile {
    let buttons = field(RecordKind::Client, 0x1860, NativeScalar::Uint8);
    let latched = field(RecordKind::Client, 0x1862, NativeScalar::Uint8);
    NativePrimaryWeaponProfile {
        digest: RETAIL_DIGEST.to_string(),
        abi: NativeAbi::WindowsX86_64,
        equipment_contexts: vec![EquipmentContext {
            provider: "q2:equipment/hand-grenades".to_string(),
            item: Some("q2:ammo_grenades".to_string()),
        }],
        dispatcher: WeaponDispatcher {
            entry_rva: 0xf05d0,
            record: RecordKind::Entity,
            argument: 0,
            arguments: 1,
        },
        decisions: vec![
            decided(0xf0f3d, 0xf0f4d, vec![buttons, latched]),
            decided(0xf1a4b, 0xf1a5a, vec![buttons, latched]),
            decided(0xf28d1, 0xf28d8, vec![buttons]),
            decided(0xf2967, 0xf296e, vec![buttons]),
            decided(0xf2b7e, 0xf2b85, vec![buttons]),
            decided(0xf3076, 0xf307d, vec![buttons]),
            decided(0xf30be, 0xf30c5, vec![buttons]),
            decided(0x118fe0, 0x118fe7, vec![buttons]),
            decided(0x11913f, 0x119146, vec![buttons]),
            decided(0x119873, 0x11987a, vec![buttons]),
            decided(0x119cd9, 0x119ce0, vec![buttons]),
        ],
        committed_input: vec![vec![scalar_test(RecordKind::Client, 0x1890, NativeScalar::Uint8, 1.0)]],
        spawn: SpawnGate {
            entry: 0xda4b0,
            accepted: vec![scalar_test(RecordKind::Client, 0x1c54, NativeScalar::Uint8, 0.0)],
        },
        active: vec![
            scalar_test(RecordKind::Image, 0x241c30, NativeScalar::Int64, 0.0),
            scalar_test(RecordKind::Client, 0x17d8, NativeScalar::Uint8, 0.0),
            scalar_test(RecordKind::Client, 0x1c54, NativeScalar::Uint8, 0.0),
            scalar_test(RecordKind::Entity, 0x7a4, NativeScalar::Uint8, 0.0),
        ],
        continuations: vec![
            vec![scalar_test(RecordKind::Client, 0x1924, NativeScalar::Int32, 3.0)],
            vec![scalar_test(RecordKind::Client, 0x1890, NativeScalar::Uint8, 1.0)],
        ],
        time: WeaponTime {
            address: 0x241b28,
            encoding: NativeScalar::Int64,
            milliseconds: 1.0,
        },
        entity: WeaponEntity {
            client: 0x78,
            water_level: field(RecordKind::Entity, 0x83c, NativeScalar::Uint8),
            view_height: field(RecordKind::Entity, 0x7a0, NativeScalar::Int32),
            max_health: field(RecordKind::Entity, 0x77c, NativeScalar::Int32),
        },
        client: WeaponClient {
            byte_length: 7344,
            view_angles: 0x1998,
            buttons,
            latched_buttons: latched,
        },
        attack_animation: AttackAnimation {
            entry: 0xf1180,
            skip: vec![NativeRegion {
                entry: 0xf11c3,
                join: 0xf12d8,
            }],
        },
        animation: WeaponAnimation {
            frame: field(RecordKind::Entity, 56, NativeScalar::Int32),
            end: field(RecordKind::Client, 0x19f4, NativeScalar::Int32),
            priority: field(RecordKind::Client, 0x19f8, NativeScalar::Int32),
            duck: field(RecordKind::Client, 0x19fc, NativeScalar::Uint8),
            run: field(RecordKind::Client, 0x19fd, NativeScalar::Uint8),
        },
        delay: WeaponDelay {
            flag: field(RecordKind::Image, 0x1d634d, NativeScalar::Uint8),
            region: NativeRegion {
                entry: 0xeff75,
                join: 0xeff8a,
            },
            evaluate: DelayEvaluate::SourceAnimation {
                entry: 0xf04f0,
                baseline_milliseconds: 100.0,
                projection: vec![
                    ProjectionWrite {
                        field: field(RecordKind::Client, 0x1924, NativeScalar::Int32),
                        value: 3.0,
                    },
                    ProjectionWrite {
                        field: field(RecordKind::Client, 0x78, NativeScalar::Int32),
                        value: 1.0,
                    },
                ],
                writes: vec![field(RecordKind::Client, 0x7c, NativeScalar::Int32)],
            },
        },
        damage: WeaponDamage::SourceResult {
            entry: 0xef5f0,
            result: DamageResult::Uint8,
        },
    }
}

/// Builtin weapon profile for a digest, or `None` when unknown.
#[must_use]
pub fn native_primary_weapon_profile(digest: &str) -> Option<NativePrimaryWeaponProfile> {
    if digest == CLASSIC_DIGEST {
        Some(xatrix_profile())
    } else if digest == RETAIL_DIGEST {
        Some(retail_profile())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_xatrix_profile() {
        let profile = native_primary_weapon_profile(CLASSIC_DIGEST).expect("xatrix");
        assert_eq!(profile.abi, NativeAbi::WindowsI386);
        assert_eq!(profile.dispatcher.entry_rva, 0x36710);
        assert_eq!(profile.decisions.len(), 7);
        assert_eq!(profile.client.byte_length, 3832);
        assert_eq!(profile.entity.client, 84);
        assert!(matches!(profile.delay.evaluate, DelayEvaluate::SourceFlag { .. }));
    }

    #[test]
    fn resolves_retail_profile() {
        let profile = native_primary_weapon_profile(RETAIL_DIGEST).expect("retail");
        assert_eq!(profile.abi, NativeAbi::WindowsX86_64);
        assert_eq!(profile.decisions.len(), 11);
        assert_eq!(profile.client.byte_length, 7344);
        assert!(matches!(
            profile.damage,
            WeaponDamage::SourceResult {
                result: DamageResult::Uint8,
                ..
            }
        ));
        match &profile.delay.evaluate {
            DelayEvaluate::SourceAnimation {
                baseline_milliseconds,
                projection,
                ..
            } => {
                assert_eq!(*baseline_milliseconds, 100.0);
                assert_eq!(projection.len(), 2);
            }
            DelayEvaluate::SourceFlag { .. } => panic!("expected animation delay"),
        }
    }

    #[test]
    fn rejects_unknown_digests() {
        assert!(native_primary_weapon_profile("sha256:dead").is_none());
    }
}
