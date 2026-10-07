use qa_core::{
    events::{EventRing, FrameEvent, TextStore},
    primitives::*,
};

fn sound(id: u32) -> FrameEvent {
    FrameEvent::Sound(SoundEvent {
        sound: SoundId(id),
        entity: None,
        channel: 1,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 1.0,
        action: SoundAction::Play,
    })
}

#[test]
fn ring_overwrites_oldest_in_order_and_drains_without_retaining_events() {
    assert!(EventRing::load(3).is_err());
    let mut ring = EventRing::load(4).unwrap();
    for id in 0..10 {
        ring.push(sound(id));
    }
    assert_eq!((ring.len(), ring.overwritten()), (4, 6));
    let ids: Vec<_> = ring
        .drain()
        .map(|event| match event {
            FrameEvent::Sound(s) => s.sound.0,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(ids, [6, 7, 8, 9]);
    assert!(ring.is_empty());
    for id in 10..13 {
        ring.push(sound(id));
    }
    assert_eq!(ring.drain().count(), 3);
    assert!(ring.pop().is_none());
    assert_eq!(ring.overwritten(), 6);
}

#[test]
fn text_handles_do_not_alias_recycled_messages_and_keep_source_newlines() {
    let mut text = TextStore::load(2, 8).unwrap();
    let first = text.insert(b"first\n").unwrap();
    let second = text.insert(b"second").unwrap();
    assert_eq!(text.get(first), Some(b"first\n".as_slice()));
    let third = text.insert(b"third is long").unwrap();
    assert!(text.get(first).is_none());
    assert_eq!(text.get(second), Some(b"second".as_slice()));
    assert_eq!(text.get(third), Some(b"third is".as_slice()));
    assert_eq!((text.overwritten(), text.truncated()), (1, 1));
}
