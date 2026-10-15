use qa_core::primitives::PlayerState;

#[test]
fn player_reset_clears_semantic_movement_flags_without_replacing_inventory_storage() {
    let mut player = PlayerState::with_capacity(2, 3, 4);
    let items = player.inventory.item_counts.as_ptr();
    assert!(!player.movement.respawned);
    assert!(!player.movement.ignore_player_collision);
    player.movement.respawned = true;
    player.movement.ignore_player_collision = true;
    player.reset();
    assert!(!player.movement.respawned);
    assert!(!player.movement.ignore_player_collision);
    assert_eq!(player.inventory.item_counts.as_ptr(), items);
}
