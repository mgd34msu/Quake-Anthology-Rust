#[test]
fn native_yaw_turn_preserves_ties_quantization_and_negative_speed() {
    use qa_movement::change_yaw;
    assert_eq!(change_yaw(0.0, 180.0, 90.0), Some(270.0));
    assert_eq!(change_yaw(180.0, 0.0, 90.0), Some(270.0));
    assert_eq!(change_yaw(0.0, 90.0, 30.0), Some(29.998_169));
    assert_eq!(change_yaw(0.0, 90.0, -10.0), Some(350.002_44));
    assert_eq!(change_yaw(-0.0, 0.0, f32::NAN), None);
    assert_eq!(change_yaw(0.0, 90.0, f32::NAN), Some(90.0));
}

#[test]
fn yaw_displacements_match_native_scalar_assignment_bits() {
    // Independent C outputs of Quake's PF_walkmove expression, including
    // signed zero and angles whose trig products narrow differently.
    let rows = [
        (0.0f32, 10.0f32, [0x41200000, 0, 0]),
        (-0.0, 3.0, [0x40400000, 0x80000000, 0]),
        (0.0, -0.0, [0x80000000, 0x80000000, 0]),
        (45.0, 10.0, [0x40e24630, 0x40e24630, 0]),
        (90.0, -7.0, [0x34a44589, 0xc0e00000, 0]),
        (215.0, 17.25, [0xc1621602, 0xc11e4e9d, 0]),
        (359.999, 0.33333334, [0x3eaaaaab, 0xb6c360b3, 0]),
        (1234567.0, 103.25, [0xc278ce78, 0x42a4d244, 0]),
    ];
    for (yaw, distance, bits) in rows {
        assert_eq!(
            qa_movement::yaw_displacement(yaw, distance)
                .0
                .map(f32::to_bits),
            bits
        );
    }
}
