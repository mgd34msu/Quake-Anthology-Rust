use qa_core::primitives::SurfaceFlags;

#[test]
fn every_native_surface_bit_round_trips_without_changing_movement_flags() {
    for bit in 0..32 {
        let raw = 1u32 << bit;
        assert_eq!(SurfaceFlags::from_q2(raw).to_q2(), raw);
        assert_eq!(SurfaceFlags::from_q3(raw).to_q3(), raw);
    }
    for raw in [0, 0xffffffff, 0x80000183, 0x55aa00ff] {
        assert_eq!(SurfaceFlags::from_q2(raw).to_q2(), raw);
        assert_eq!(SurfaceFlags::from_q3(raw).to_q3(), raw);
    }
    assert_eq!(SurfaceFlags::from_q2(2), SurfaceFlags::SLICK);
    assert_eq!(SurfaceFlags::from_q3(2 | 8 | 1 | 0x2000 | 0x1000).0, 31);
}

#[test]
fn shared_sky_and_nodraw_semantics_project_to_the_original_protocol_bits() {
    assert_eq!(SurfaceFlags::from_q2(4).to_q3(), 4 | 16);
    assert_eq!(SurfaceFlags::from_q3(4).to_q2(), 4);
    assert_eq!(SurfaceFlags::from_q2(0x82).to_q3(), 0x82);
    assert_eq!(SurfaceFlags::from_q3(0x82).to_q2(), 0x82);
    assert_eq!(SurfaceFlags::from_q3(1 | 8 | 0x1000 | 0x2000).to_q2(), 0);
    assert_eq!(SurfaceFlags::from_q2(1 | 8 | 16 | 32 | 64).to_q3(), 0);
}
