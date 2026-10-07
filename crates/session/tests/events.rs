use qa_core::{
    events::{EventRing, FrameEvent},
    primitives::*,
};
use qa_session::events::{EventConsumer, dispatch_frame};

#[derive(Default)]
struct Consumer {
    events: Vec<u32>,
}
impl EventConsumer for Consumer {
    fn sound(&mut self, event: SoundEvent) {
        self.events.push(event.sound.0);
    }
    fn effect(&mut self, event: EffectEvent) {
        self.events.push(event.effect.0);
    }
    fn print(&mut self, event: PrintEvent) {
        self.events.push(event.text.slot as u32);
    }
}

#[test]
fn one_frame_dispatch_routes_all_three_event_types_and_leaves_empty_ring() {
    let mut ring = EventRing::load(8).unwrap();
    ring.push(FrameEvent::Sound(SoundEvent {
        sound: SoundId(1),
        entity: None,
        channel: 1,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 1.0,
        action: SoundAction::Play,
    }));
    ring.push(FrameEvent::Print(PrintEvent {
        client: Some(ClientId(0)),
        kind: PrintKind::Center,
        text: TextId {
            slot: 2,
            generation: 1,
        },
    }));
    ring.push(FrameEvent::Effect(EffectEvent {
        effect: EffectId(3),
        position: Vec3::default(),
        direction: Vec3::default(),
        count: 20,
    }));
    let mut consumer = Consumer::default();
    dispatch_frame(&mut ring, &mut consumer);
    assert_eq!(consumer.events, [1, 2, 3]);
    assert!(ring.is_empty());
    dispatch_frame(&mut ring, &mut consumer);
    assert_eq!(consumer.events, [1, 2, 3]);
}
