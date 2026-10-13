use qa_core::{
    effects::NativeParticleBindings,
    events::{
        EventRing, FrameEvent, OutputConsumerId, OutputRecord, OutputSubmission, OutputTarget,
        TextStore, dispatch_consumer,
    },
    primitives::{
        ClientId, EffectEvent, EffectId, EntityId, ModuleId, NativeDelivery, NativeTarget,
        RuleSetId, Vec3,
    },
    sys_events::EventTime,
};

fn binding() -> NativeParticleBindings {
    NativeParticleBindings {
        directional: EffectId(9),
        explosion: EffectId(17),
    }
}

fn consume(ring: &mut EventRing, consumer: OutputConsumerId) -> Vec<EffectEvent> {
    let mut effects = Vec::new();
    dispatch_consumer(
        ring,
        consumer,
        &mut |record: OutputRecord, _: &mut TextStore| {
            let FrameEvent::Effect(effect) = record.event else {
                panic!("unexpected output")
            };
            effects.push(effect);
            OutputSubmission::BestEffort
        },
    );
    effects
}

#[test]
fn native_count_mapping_keeps_color_and_numeric_recipe_identity_separate() {
    let binding = binding();
    for color in 0..=u8::MAX {
        for count in 0..u8::MAX {
            let event = binding.event(Vec3([1.0, -2.0, 3.0]), Vec3([4.0, 5.0, -6.0]), color, count);
            assert_eq!(event.effect, EffectId(9));
            assert_eq!(event.count, u16::from(count));
            assert_eq!(event.color, color);
            assert_eq!(binding.wire_count(event), Some(count));
            assert_eq!(event.time, EventTime::default());
            assert_eq!(event.client, None);
            assert_eq!(event.client_lifetime, None);
        }
        let event = binding.event(Vec3::default(), Vec3::default(), color, u8::MAX);
        assert_eq!(
            (event.effect, event.count, event.color),
            (EffectId(17), 1024, color)
        );
        assert_eq!(binding.wire_count(event), Some(u8::MAX));
    }
    for event in [
        EffectEvent::default(),
        EffectEvent {
            effect: EffectId(9),
            count: 255,
            ..EffectEvent::default()
        },
        EffectEvent {
            effect: EffectId(9),
            count: 256,
            ..EffectEvent::default()
        },
        EffectEvent {
            effect: EffectId(17),
            count: 255,
            ..EffectEvent::default()
        },
    ] {
        assert_eq!(binding.wire_count(event), None);
    }
}

#[test]
fn received_effect_filters_recipient_and_retains_exact_connection_generation() {
    let mut ring = EventRing::load(8, 4, 8, 64).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let selected = ring.bind(OutputTarget::Client(ClientId(3))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(4))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let old = EntityId {
        slot: 2,
        generation: 5,
    };
    let replacement = EntityId {
        slot: old.slot,
        generation: old.generation + 1,
    };
    let event = EffectEvent {
        client: Some(ClientId(3)),
        client_lifetime: Some(old),
        time: EventTime(81_234_567),
        ..binding().event(Vec3([1.0, 2.0, 3.0]), Vec3::default(), 226, 20)
    };
    assert!(event.applies_to_client(ClientId(3)));
    assert!(!event.applies_to_client(ClientId(4)));
    let first = ring
        .push_to(OutputTarget::Local(ClientId(3)), FrameEvent::Effect(event))
        .unwrap();
    let later = EffectEvent {
        client_lifetime: Some(replacement),
        time: EventTime(event.time.0 + 1),
        ..event
    };
    let second = ring
        .push_to(OutputTarget::Presentation, FrameEvent::Effect(later))
        .unwrap();
    assert!(ring.retained_for(presentation, first));
    assert!(ring.retained_for(presentation, second));
    assert!(ring.retained_for(selected, first));
    assert!(!ring.retained_for(selected, second));
    assert!(consume(&mut ring, other).is_empty());
    assert!(consume(&mut ring, module).is_empty());
    assert_eq!(consume(&mut ring, presentation), [event, later]);
    assert!(!ring.retained_for(presentation, second));
    // The earlier selected consumer still owns the head slot, so this range
    // includes the completed later slot until the contiguous head can retire.
    assert_eq!(ring.len(), 2);
    assert_eq!(consume(&mut ring, selected), [event]);
    assert!(ring.is_empty());
}

#[test]
fn client_selected_ordinary_effect_does_not_reach_other_clients() {
    let mut ring = EventRing::load(4, 3, 4, 64).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let selected = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let event = EffectEvent {
        client: Some(ClientId(0)),
        ..binding().event(Vec3::default(), Vec3::default(), 0, 1)
    };
    let sequence = ring.push(FrameEvent::Effect(event)).unwrap();
    assert!(ring.retained_for(presentation, sequence));
    assert!(ring.retained_for(selected, sequence));
    assert!(!ring.retained_for(other, sequence));
    assert!(consume(&mut ring, other).is_empty());
    assert_eq!(consume(&mut ring, presentation), [event]);
    assert_eq!(consume(&mut ring, selected), [event]);
    assert!(ring.is_empty());
}

#[test]
fn server_effect_waits_for_native_delivery_and_has_no_direct_presentation_copy() {
    let mut ring = EventRing::load(4, 4, 4, 64).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let selected = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(2))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(8))).unwrap();
    let event = binding().event(Vec3([8.0, 9.0, 10.0]), Vec3([1.0, 0.0, 0.0]), 0xe0, 6);
    let sequence = ring
        .push_delivered(
            NativeDelivery {
                module: ModuleId(7),
                source: RuleSetId::Quake,
                target: NativeTarget::Client(ClientId(1)),
                origin: event.position,
                reliable: false,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: false,
            },
            FrameEvent::Effect(event),
        )
        .unwrap();
    assert!(!ring.retained_for(presentation, sequence));
    assert!(!ring.retained_for(other, sequence));
    assert!(consume(&mut ring, presentation).is_empty());
    assert!(consume(&mut ring, other).is_empty());
    let mut batch = ring.batch(selected).unwrap();
    let record = ring.next(&mut batch).unwrap();
    assert!(matches!(record.event, FrameEvent::Effect(actual) if actual == event));
    assert!(ring.submit(selected, sequence, OutputSubmission::Unsent));
    assert!(ring.retained_for(selected, sequence));
    assert_eq!(consume(&mut ring, module), [event]);
    assert_eq!(consume(&mut ring, selected), [event]);
    assert!(ring.is_empty());
}
