use qa_core::primitives::{
    ModuleId, NativePlayerTail, PlayerPresentation, PlayerState, ResourceNamespace, RuleSetId, Vec3,
};

#[test]
fn native_presentation_is_absent_until_import_and_player_reset_preserves_storage() {
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let mut tail = NativePlayerTail::new(RuleSetId::Quake3, namespace);
    assert_eq!(tail.presentation, None);
    tail.presentation = Some(PlayerPresentation {
        bob_cycle: 255,
        legs_timer: -1,
        grapple_point: Vec3([1.0, 2.0, 3.0]),
        jump_pad_entity: 1022,
        ..Default::default()
    });
    let mut player = PlayerState::with_capacity(3, 4, 2);
    player.native = Some(tail);
    player.reset();
    assert_eq!(player.native, None);
    assert_eq!(player.inventory.item_counts.len(), 3);
    assert_eq!(player.inventory.powerup_until.len(), 4);
    assert_eq!(player.values.capacity(), 2);
}
