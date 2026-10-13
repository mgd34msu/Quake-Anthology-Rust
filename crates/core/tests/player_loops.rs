use qa_core::primitives::{ClientId, ModuleId, PlayerState, ResourceNamespace, SoundRef};

#[test]
fn player_loop_keeps_native_resource_width_and_namespace() {
    for namespace in [
        ResourceNamespace::Module(ModuleId(7)),
        ResourceNamespace::Client(ClientId(3)),
    ] {
        let mut player = PlayerState::default();
        assert_eq!(player.loop_sound, None);
        for ordinal in [1, 65_535, 65_536, i32::MAX as u32, u32::MAX] {
            player.loop_sound = SoundRef::native(namespace, ordinal);
            assert_eq!(player.loop_sound.unwrap().namespace, namespace);
            assert_eq!(player.loop_sound.unwrap().ordinal, ordinal);
        }
        player.loop_sound = SoundRef::native(namespace, 0);
        assert_eq!(player.loop_sound, None);
    }
}

#[test]
fn player_reset_clears_loop_while_retaining_load_sized_inventory_and_values() {
    let mut player = PlayerState::with_capacity(3, 4, 2);
    player.loop_sound = SoundRef::native(ResourceNamespace::Module(ModuleId(7)), 9);
    player.reset();
    assert_eq!(player.loop_sound, None);
    assert_eq!(player.inventory.item_counts.len(), 3);
    assert_eq!(player.inventory.powerup_until.len(), 4);
    assert_eq!(player.values.capacity(), 2);
}
