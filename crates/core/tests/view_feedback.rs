use qa_core::{
    events::{EventRing, FrameEvent, OutputSubmission, OutputTarget, TextStore},
    primitives::{
        ClientId, ClockObservation, EntityId, HudState, ModuleId, NativePlayerTail,
        PlayerDamageSample, PlayerSignals, PlayerState, ResourceNamespace, RuleSetId, ThinkTime,
        Vec3, ViewFeedback, ViewFeedbackEvent,
    },
    sys_events::EventTime,
};

fn feedback(start: i64) -> ViewFeedback {
    ViewFeedback {
        kick: Vec3([1.0, 0.0, -2.0]),
        screen: [0.25, -0.5],
        value: 7.0,
        clock: ClockObservation {
            native: ThinkTime::Milliseconds(start),
            queued: EventTime(137_000_000),
        },
        deflect_ms: 100.0,
        return_ms: 400.0,
    }
}

#[test]
fn view_kick_deflects_then_returns_without_mutating_player_angles() {
    let player = PlayerState {
        view_angles: Vec3([20.0, -30.0, 40.0]),
        ..Default::default()
    };
    let feedback = feedback(1000);
    for (now, expected) in [
        (1050, Vec3([0.5, 0.0, -1.0])),
        (1100, Vec3([1.0, 0.0, -2.0])),
        (1300, Vec3([0.5, 0.0, -1.0])),
    ] {
        assert_eq!(
            feedback.kick_at(ThinkTime::Milliseconds(now)),
            Some(expected)
        );
    }
    assert_eq!(feedback.kick_at(ThinkTime::Milliseconds(1500)), None);
    assert_eq!(feedback.kick_at(ThinkTime::Milliseconds(1501)), None);
    assert_eq!(player.view_angles, Vec3([20.0, -30.0, 40.0]));
}

#[test]
fn view_kick_retains_native_float_clock_and_double_return_rounding() {
    let kick = feedback(1000)
        .kick_at(ThinkTime::Milliseconds(1499))
        .unwrap();
    assert_eq!(kick.0[0].to_bits(), 0x3b23_d700);
    assert_ne!(kick.0[0].to_bits(), 0.0025f32.to_bits());

    let kick = feedback(16_777_217)
        .kick_at(ThinkTime::Milliseconds(16_777_218))
        .unwrap();
    assert_eq!(kick.0[0].to_bits(), 0.02f32.to_bits());
    // Presentation can still be before its accepted server snapshot. Native
    // deflection does not clamp that signed difference to zero.
    assert_eq!(
        feedback(1000).kick_at(ThinkTime::Milliseconds(950)),
        Some(Vec3([-0.5, -0.0, 1.0]))
    );
    assert_eq!(feedback(1000).kick_at(ThinkTime::Seconds(1.1)), None);
}

#[test]
fn player_and_hud_reset_clear_signals_and_feedback_with_reserved_storage_intact() {
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let mut player = PlayerState::with_capacity(3, 4, 2);
    player.native = Some(NativePlayerTail {
        signals: Some(PlayerSignals {
            sequence: u32::MAX,
            damage: PlayerDamageSample {
                sequence: u32::MAX,
                count: 9,
                ..Default::default()
            },
            ..Default::default()
        }),
        ..NativePlayerTail::new(RuleSetId::Quake3, namespace)
    });
    player.reset();
    assert_eq!(player.native, None);
    assert_eq!(player.inventory.item_counts.len(), 3);
    assert_eq!(player.inventory.powerup_until.len(), 4);
    assert_eq!(player.values.capacity(), 2);

    let mut hud = HudState::with_capacity(3, 4, 2, 5);
    hud.view_feedback = Some(feedback(1000));
    let mut texts = TextStore::load(4, 32).unwrap();
    hud.reset(&mut texts);
    assert_eq!(hud.view_feedback, None);
    assert_eq!(hud.inventory.item_counts.len(), 3);
    assert_eq!(hud.inventory.powerup_until.len(), 4);
    assert_eq!(hud.values.capacity(), 5);
}

#[test]
fn view_feedback_has_one_local_ring_record_and_selected_client_claim() {
    let mut ring = EventRing::load(4, 4, 4, 32).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let selected = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let event = ViewFeedbackEvent {
        client: ClientId(0),
        client_lifetime: EntityId {
            slot: 8,
            generation: 2,
        },
        source: RuleSetId::Quake3,
        namespace: ResourceNamespace::Client(ClientId(0)),
        native_client: 4,
        feedback: feedback(1000),
    };
    let sequence = ring
        .push_to(
            OutputTarget::Local(ClientId(0)),
            FrameEvent::ViewFeedback(event),
        )
        .unwrap();
    for consumer in [other, module] {
        let mut batch = ring.batch(consumer).unwrap();
        assert!(ring.next(&mut batch).is_none());
    }
    for consumer in [presentation, selected] {
        let mut batch = ring.batch(consumer).unwrap();
        let record = ring.next(&mut batch).unwrap();
        let FrameEvent::ViewFeedback(received) = record.event else {
            panic!("view feedback")
        };
        assert_eq!(received, event);
        assert_eq!(record.sequence, sequence);
        assert!(ring.submit(consumer, sequence, OutputSubmission::BestEffort));
    }
    assert_eq!(ring.len(), 0);
}
