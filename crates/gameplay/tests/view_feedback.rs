use qa_core::primitives::{ClockObservation, PlayerDamageSample, RuleSetId, ThinkTime, Vec3};
use qa_core::sys_events::EventTime;
use qa_gameplay::rules::feedback::damage_feedback_rules;

fn clock() -> ClockObservation {
    ClockObservation {
        native: ThinkTime::Milliseconds(5000),
        queued: EventTime(137_000_000),
    }
}

fn axes() -> [Vec3; 3] {
    [
        Vec3([1.0, 0.0, 0.0]),
        Vec3([0.0, 1.0, 0.0]),
        Vec3([0.0, 0.0, 1.0]),
    ]
}

#[test]
fn q3_centered_feedback_uses_health_scaling_and_native_kick_limits() {
    let rules = damage_feedback_rules(RuleSetId::Quake3).unwrap();
    for (health, count, kick) in [(39, 1, 5.0), (40, 7, 7.0), (80, 12, 6.0), (80, 100, 10.0)] {
        let feedback = rules
            .project(
                PlayerDamageSample {
                    yaw: 255,
                    pitch: 255,
                    count,
                    ..Default::default()
                },
                health,
                axes(),
                clock(),
            )
            .unwrap();
        assert_eq!(feedback.kick, Vec3([-kick, 0.0, 0.0]));
        assert_eq!(feedback.screen, [0.0, 0.0]);
        assert_eq!(feedback.value, kick);
        assert_eq!(feedback.clock, clock());
        assert_eq!(feedback.deflect_ms, 100.0);
        assert_eq!(feedback.return_ms, 400.0);
    }
}

#[test]
fn q3_directional_feedback_uses_the_previous_view_axes() {
    let rules = damage_feedback_rules(RuleSetId::Quake3).unwrap();
    let sample = PlayerDamageSample {
        count: 10,
        ..Default::default()
    };
    let forward = rules.project(sample, 40, axes(), clock()).unwrap();
    assert_eq!(forward.kick.0[0], 10.0);
    let turned = rules
        .project(
            sample,
            40,
            [
                Vec3([0.0, -1.0, 0.0]),
                Vec3([-1.0, 0.0, 0.0]),
                Vec3([0.0, 0.0, 1.0]),
            ],
            clock(),
        )
        .unwrap();
    assert_eq!(turned.kick.0[2], 10.0);
    assert_eq!(turned.screen[0], -1.0);
    assert_ne!(turned.kick, forward.kick);
    let at_peak = turned.kick_at(ThinkTime::Milliseconds(5100)).unwrap();
    assert_eq!(at_peak, turned.kick);
}

#[test]
fn feedback_policy_is_selected_from_native_source_and_requires_its_clock() {
    for source in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
    ] {
        assert!(damage_feedback_rules(source).is_none());
    }
    let rules = damage_feedback_rules(RuleSetId::Quake3).unwrap();
    assert!(
        rules
            .project(PlayerDamageSample::default(), 40, axes(), clock())
            .is_none()
    );
    assert!(
        rules
            .project(
                PlayerDamageSample {
                    count: 10,
                    ..Default::default()
                },
                40,
                axes(),
                ClockObservation {
                    native: ThinkTime::Seconds(5.0),
                    ..clock()
                }
            )
            .is_none()
    );
    // These are consumer decisions: projection still retains the event while
    // dead-view and native server-time-zero prevent a later camera kick.
    let feedback = rules
        .project(
            PlayerDamageSample {
                yaw: 255,
                pitch: 255,
                count: 10,
                ..Default::default()
            },
            0,
            axes(),
            ClockObservation {
                native: ThinkTime::Milliseconds(0),
                ..clock()
            },
        )
        .unwrap();
    assert_eq!(feedback.value, 10.0);
}
