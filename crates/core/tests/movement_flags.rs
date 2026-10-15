use qa_core::primitives::PlayerState;

#[test]
fn player_reset_clears_respawn_gate_without_replacing_inventory_storage() {
    let mut player = PlayerState::with_capacity(2, 3, 4);
    let items = player.inventory.item_counts.as_ptr();
    assert!(!player.movement.respawned);
    player.movement.respawned = true;
    player.reset();
    assert!(!player.movement.respawned);
    assert_eq!(player.inventory.item_counts.as_ptr(), items);
}
