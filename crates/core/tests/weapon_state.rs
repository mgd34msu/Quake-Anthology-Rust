use qa_core::primitives::{
    ClientId, ModuleId, PlayerState, ResourceNamespace, RuleSetId, WeaponReadiness, WeaponState,
};

#[test]
fn weapon_readiness_keeps_native_width_units_and_resource_owner() {
    for bits in [0, 0x8000_0000, 0x3f80_0001, 0xffc0_1234] {
        let state = WeaponState {
            source: RuleSetId::QuakeWorld,
            namespace: ResourceNamespace::Module(ModuleId(17)),
            readiness: WeaponReadiness::DeadlineSeconds(f32::from_bits(bits)),
        };
        let mut player = PlayerState::default();
        player.movement_rules = RuleSetId::Quake3;
        player.trace_rules = RuleSetId::Quake2;
        player.weapon_state = Some(state);
        let stored = player.weapon_state.unwrap();
        assert_eq!(stored.source, RuleSetId::QuakeWorld);
        assert_eq!(stored.namespace, ResourceNamespace::Module(ModuleId(17)));
        let WeaponReadiness::DeadlineSeconds(deadline) = stored.readiness else {
            panic!("native QC seconds");
        };
        assert_eq!(deadline.to_bits(), bits);
    }
    for time in [i32::MIN, -197_000_071, -1, 0, i32::MAX] {
        let state = WeaponState {
            source: RuleSetId::Quake3,
            namespace: ResourceNamespace::Client(ClientId(5)),
            readiness: WeaponReadiness::RemainingMilliseconds(time),
        };
        assert_eq!(
            state.readiness,
            WeaponReadiness::RemainingMilliseconds(time)
        );
    }
}

#[test]
fn player_reset_clears_readiness_without_inventing_a_native_default() {
    let mut player = PlayerState::with_capacity(3, 4, 2);
    assert_eq!(player.weapon_state, None);
    player.weapon_state = Some(WeaponState {
        source: RuleSetId::Quake,
        namespace: ResourceNamespace::Module(ModuleId(2)),
        readiness: WeaponReadiness::DeadlineSeconds(8.25),
    });
    player.reset();
    assert_eq!(player.weapon_state, None);
    assert_eq!(player.inventory.item_counts.len(), 3);
}
