use qa_core::{primitives::*, sys_events::EventTime};

#[test]
fn damage_view_keeps_native_zero_dead_view_and_yaw_rules() {
    let mut hud = HudState::default();
    let angles = Vec3([10.0, -0.0, 20.0]);
    let feedback = qa_gameplay::rules::feedback::damage_feedback_rules(RuleSetId::Quake3)
        .unwrap()
        .project(
            PlayerDamageSample {
                sequence: 1,
                yaw: 255,
                pitch: 255,
                count: 20,
            },
            90,
            [Vec3::default(); 3],
            ClockObservation {
                native: ThinkTime::Milliseconds(1000),
                queued: EventTime::from_milliseconds(100),
            },
        )
        .unwrap();
    hud.view_feedback = Some(feedback);
    let now = ThinkTime::Milliseconds(1050);
    let kicked = qa_ui::hud::view_angles(&hud, angles, 90, now);
    assert_ne!(kicked.0[0].to_bits(), angles.0[0].to_bits());
    assert_eq!(kicked.0[1].to_bits(), angles.0[1].to_bits());
    assert_eq!(kicked.0[2].to_bits(), angles.0[2].to_bits());
    assert_eq!(qa_ui::hud::view_angles(&hud, angles, 0, now), angles);
    assert_eq!(qa_ui::hud::view_angles(&hud, angles, -1, now), angles);
    assert_eq!(
        qa_ui::hud::view_angles(&hud, angles, 90, ThinkTime::Milliseconds(1500)),
        angles
    );
    hud.view_feedback.as_mut().unwrap().clock.native = ThinkTime::Milliseconds(0);
    assert_eq!(
        qa_ui::hud::view_angles(&hud, angles, 90, ThinkTime::Milliseconds(50)),
        angles
    );
    hud.reset(&mut qa_core::events::TextStore::load(1, 1).unwrap());
    assert!(hud.view_feedback.is_none());
}
