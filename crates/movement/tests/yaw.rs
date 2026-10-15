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
