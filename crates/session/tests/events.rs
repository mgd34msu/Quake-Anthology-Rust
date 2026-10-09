use qa_core::{events::*, primitives::*};
use qa_session::events::{EventConsumer, dispatch_consumer};
#[derive(Default)]
struct Consumer {
    events: Vec<u32>,
}
impl EventConsumer for Consumer {
    fn submit(&mut self, record: OutputRecord, texts: &mut TextStore) -> OutputSubmission {
        self.events.push(match record.event {
            FrameEvent::Sound(s) => s.sound.0,
            FrameEvent::Effect(e) => e.effect.0,
            FrameEvent::Print(p) => {
                assert_eq!(texts.get(p.text), Some(b"message".as_slice()));
                2
            }
        });
        OutputSubmission::BestEffort
    }
}
#[test]
fn independent_module_dispatch_routes_all_types_once_and_retires_only_after_both() {
    let mut ring = EventRing::load(8, 2, 8, 32).unwrap();
    let first = ring.bind(OutputTarget::Module(ModuleId(1))).unwrap();
    let second = ring.bind(OutputTarget::Module(ModuleId(2))).unwrap();
    ring.push(FrameEvent::Sound(SoundEvent {
        sound: SoundId(1),
        entity: None,
        channel: 1,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 1.0,
        action: SoundAction::Play,
    }))
    .unwrap();
    ring.print(
        Some(ClientId(0)),
        PrintKind::Center,
        format_args!("message"),
    )
    .unwrap();
    ring.push(FrameEvent::Effect(EffectEvent {
        effect: EffectId(3),
        position: Vec3::default(),
        direction: Vec3::default(),
        count: 20,
    }))
    .unwrap();
    let mut a = Consumer::default();
    let mut b = Consumer::default();
    dispatch_consumer(&mut ring, first, &mut a);
    assert_eq!(a.events, [1, 2, 3]);
    assert_eq!(ring.len(), 3);
    dispatch_consumer(&mut ring, first, &mut a);
    assert_eq!(a.events, [1, 2, 3]);
    dispatch_consumer(&mut ring, second, &mut b);
    assert_eq!(b.events, [1, 2, 3]);
    assert!(ring.is_empty());
}
