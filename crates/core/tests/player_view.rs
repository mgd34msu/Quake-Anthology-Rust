use qa_core::primitives::{
    NativeViewFlags, PlayerState, PlayerView, PlayerViewFlags, Vec3, ViewFlagLayout,
};

#[test]
fn authored_camera_and_physical_height_have_independent_ownership() {
    let mut player = PlayerState::default();
    player.body.position = Vec3([10.0, 20.0, 30.0]);
    player.movement.view_height = 26.0;
    assert_eq!(player.eye(), Vec3([10.0, 20.0, 56.0]));
    let raw = Vec3([1.25, -2.5, 24.125]);
    player.view.accept_offset(raw, None);
    player.movement.view_height = 12.0;
    assert_eq!(player.view.raw_offset(), Some((raw, None)));
    assert_eq!(player.eye(), Vec3([11.25, 17.5, 54.125]));
    player
        .view
        .accept_offset(Vec3([1.25, -2.5, 2.125]), Some(22.0));
    assert_eq!(player.eye(), Vec3([11.25, 17.5, 54.125]));
    player.movement.view_height = -2.0;
    assert_eq!(player.eye(), Vec3([11.25, 17.5, 54.125]));
    player.view.clear_offset();
    assert_eq!(player.eye(), Vec3([10.0, 20.0, 28.0]));
}

#[test]
fn split_eye_keeps_native_origin_then_offset_then_height_order() {
    let mut view = PlayerView::default();
    let origin = Vec3([0.0, 0.0, 16_777_216.0]);
    view.accept_offset(Vec3([0.0, 0.0, 1.0]), Some(22.0));
    assert_eq!(
        view.eye(origin, 0.0).0[2].to_bits(),
        16_777_238.0f32.to_bits()
    );
    assert_eq!(
        (origin + view.effective_offset(0.0)).0[2].to_bits(),
        16_777_240.0f32.to_bits()
    );
    assert_eq!(view.raw_offset(), Some((Vec3([0.0, 0.0, 1.0]), Some(22.0))));
}

#[test]
fn absent_height_preserves_negative_zero_but_explicit_zero_adds() {
    let mut view = PlayerView::default();
    let raw = Vec3([-0.0; 3]);
    view.accept_offset(raw, None);
    assert_eq!(
        view.effective_offset(0.0).0.map(f32::to_bits),
        [(-0.0f32).to_bits(); 3]
    );
    assert_eq!(view.eye(raw, 0.0).0[2].to_bits(), (-0.0f32).to_bits());
    view.accept_offset(raw, Some(0.0));
    assert_eq!(view.effective_offset(0.0).0[2].to_bits(), 0.0f32.to_bits());
    assert_eq!(view.eye(raw, 0.0).0[2].to_bits(), 0.0f32.to_bits());
    assert_eq!(
        view.raw_offset().unwrap().0.0[2].to_bits(),
        (-0.0f32).to_bits()
    );
}

#[test]
fn loaded_flag_layout_distinguishes_kex_and_repro_bit_four() {
    let raw = 0x8000_0015;
    let kex = ViewFlagLayout::KEX.decode(raw);
    assert!(kex.contains(PlayerViewFlags::UNDERWATER));
    assert!(kex.contains(PlayerViewFlags::INFRARED));
    assert!(kex.contains(PlayerViewFlags::NO_WEAPON_LERP));
    assert!(!kex.contains(PlayerViewFlags::TELEPORT));
    let residual = Some(NativeViewFlags {
        layout: ViewFlagLayout::KEX,
        raw,
    });
    assert_eq!(ViewFlagLayout::KEX.encode(kex, residual), raw);
    assert_eq!(ViewFlagLayout::REPRO.encode(kex, residual), 5);
    let repro = ViewFlagLayout::REPRO.decode(16);
    assert!(repro.contains(PlayerViewFlags::TELEPORT));
    assert!(!repro.contains(PlayerViewFlags::NO_WEAPON_LERP));
    assert_eq!(ViewFlagLayout::KEX.encode(repro, None), 0);
}

#[test]
fn reset_clears_camera_authority_without_replacing_reserved_banks() {
    let mut player = PlayerState::with_capacity(2, 3, 4);
    player.view.accept_offset(Vec3([1.0, 2.0, 3.0]), Some(-2.0));
    player.view.fov_x = Some(65.0);
    player.view.screen_blend = [0.25; 4];
    player.view.damage_blend = [0.5; 4];
    player.view.flags = PlayerViewFlags::INFRARED;
    player.reset();
    assert_eq!(player.view, PlayerView::default());
    assert_eq!(player.movement.view_height, 0.0);
    assert_eq!(player.values.capacity(), 4);
    assert_eq!(player.inventory.item_counts.len(), 2);
    assert_eq!(player.inventory.powerup_until.len(), 3);
}
