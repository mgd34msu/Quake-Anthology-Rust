use qa_core::{math::clip_component, primitives::Vec3};
use qa_movement::clip_backoff;

#[test]
fn toss_and_bounce_share_native_backoff_before_scalar_response() {
    let velocity = Vec3([6.0, -4.0, -10.0]);
    let normal = Vec3([0.0, 0.0, 1.0]);
    // qsrc SV_Physics_Toss selects 1 for Toss and 1.5 for Bounce; the
    // returned scalar response is shared with the native in-place writer.
    for (overbounce, expected_backoff, expected) in [
        (1.0, -10.0, [6.0_f32, -4.0, 0.0]),
        (1.5, -15.0, [6.0_f32, -4.0, 5.0]),
    ] {
        let backoff = clip_backoff(velocity, normal, overbounce, false);
        assert_eq!(backoff, expected_backoff);
        let actual: [f32; 3] = std::array::from_fn(|axis| {
            clip_component(velocity.0[axis], normal.0[axis], backoff, Some(0.1))
        });
        assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
    }
}

#[test]
fn backoff_retains_native_dot_order_and_q3_outward_division() {
    assert_eq!(
        clip_backoff(Vec3([1.0e20, -1.0e20, 3.0]), Vec3([1.0; 3]), 1.5, false),
        4.5
    );
    let normal = Vec3([1.0, 0.0, 0.0]);
    assert_eq!(
        clip_backoff(Vec3([-10.0, 0.0, 0.0]), normal, 1.001, true).to_bits(),
        0xc120_28f6
    );
    assert_eq!(
        clip_backoff(Vec3([10.0, 0.0, 0.0]), normal, 1.001, true).to_bits(),
        0x411f_d714
    );
    assert_eq!(
        clip_backoff(Vec3([10.0, 0.0, 0.0]), normal, 1.5, false),
        15.0
    );
}
