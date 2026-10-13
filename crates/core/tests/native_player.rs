use qa_core::primitives::{
    ClientId, ModuleId, MovementMagnitude, NativePlayerTail, PlayerState, ResourceNamespace,
    RuleSetId,
};

#[test]
fn native_player_metadata_keeps_full_integer_words_and_explicit_zero_ordinals() {
    for namespace in [
        ResourceNamespace::Module(ModuleId(7)),
        ResourceNamespace::Client(ClientId(9)),
    ] {
        let mut tail = NativePlayerTail::new(RuleSetId::Quake3, namespace);
        assert_eq!((tail.client_number, tail.weapon_number), (None, None));
        tail.client_number = Some(0);
        tail.weapon_number = Some(0);
        tail.ground_number = Some(1022);
        tail.flags = 0x8000_3163;
        for native in [i32::MIN, -1, 0, 16_777_217, i32::MAX - 1] {
            tail.gravity = Some(native);
            tail.speed = Some(native);
            let mut player = PlayerState::with_capacity(2, 1, 1);
            player.movement_rules = RuleSetId::QuakeWorld;
            player.trace_rules = RuleSetId::Quake2;
            player.native = Some(tail);
            player.prediction = qa_core::primitives::PredictionPolicy::DISABLED;
            assert_eq!(player.native, Some(tail));
            assert_eq!(player.native.unwrap().gravity, Some(native));
            assert_eq!(player.native.unwrap().speed, Some(native));
            assert_eq!(player.native.unwrap().client_number, Some(0));
            assert_eq!(player.native.unwrap().weapon_number, Some(0));
            assert_eq!(player.native.unwrap().ground_number, Some(1022));
            player.reset();
            assert_eq!(player.native, None);
            assert_eq!(player.prediction, Default::default());
            assert_eq!(player.inventory.item_counts.len(), 2);
        }
    }
}

#[test]
fn native_effective_magnitudes_keep_existing_divide_then_multiply_rounding() {
    for effective in [-137.125f32, 0.0, 250.0, 16_777_217i32 as f32] {
        for multiplier in [-2.75f32, 0.3, 0.5, 1.0, 2.0, 9.125] {
            let base = effective / multiplier;
            let stored = MovementMagnitude::native(effective, multiplier);
            assert_eq!(stored, MovementMagnitude::Base(base));
            assert_eq!(
                stored.effective(multiplier).to_bits(),
                (base * multiplier).to_bits()
            );
        }
    }
}

#[test]
fn zero_multiplier_preserves_native_effective_values_but_authored_bases_still_scale() {
    for multiplier in [0.0f32, -0.0] {
        for effective in [-137.125f32, 0.0, 250.0] {
            let stored = MovementMagnitude::native(effective, multiplier);
            assert_eq!(stored, MovementMagnitude::Effective(effective));
            assert_eq!(stored.effective(multiplier).to_bits(), effective.to_bits());
            // Later capability multiplier changes never reinterpret an already
            // effective native input as a base that needs another scaling.
            assert_eq!(stored.effective(3.0).to_bits(), effective.to_bits());
        }
        assert_eq!(
            MovementMagnitude::Base(800.0)
                .effective(multiplier)
                .to_bits(),
            (800.0 * multiplier).to_bits()
        );
    }
}
