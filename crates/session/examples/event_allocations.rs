#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;
use qa_core::{
    events::{EventRing, FrameEvent, TextStore},
    primitives::*,
};
use qa_session::events::{EventConsumer, dispatch_frame};

struct Consumer<'a> {
    texts: &'a TextStore,
    counts: [u64; 3],
    stale: u64,
}
impl EventConsumer for Consumer<'_> {
    fn sound(&mut self, event: SoundEvent) {
        std::hint::black_box(event);
        self.counts[0] += 1;
    }
    fn effect(&mut self, event: EffectEvent) {
        std::hint::black_box(event);
        self.counts[1] += 1;
    }
    fn print(&mut self, event: PrintEvent) {
        if self.texts.get(event.text) != Some(b"message\n".as_slice()) {
            self.stale += 1;
        }
        self.counts[2] += 1;
    }
}

fn main() -> Result<(), &'static str> {
    let mut events = EventRing::load(1024).map_err(|_| "ring capacity")?;
    let mut texts = TextStore::load(512, 1024).map_err(|_| "text capacity")?;
    let mut counts = [0; 3];
    let mut stale = 0;
    allocation_counter::start();
    for _ in 0..10_000 {
        for index in 0..256 {
            events.push(FrameEvent::Sound(SoundEvent {
                sound: SoundId(index),
                entity: None,
                channel: 1,
                position: Vec3::default(),
                volume: 1.0,
                attenuation: 1.0,
                action: SoundAction::Play,
            }));
            events.push(FrameEvent::Effect(EffectEvent {
                effect: EffectId(index),
                position: Vec3::default(),
                direction: Vec3::default(),
                count: 10,
            }));
            events.push(FrameEvent::Print(PrintEvent {
                client: Some(ClientId(0)),
                kind: PrintKind::Notify,
                text: texts
                    .insert(b"message\n")
                    .ok_or("text generations exhausted")?,
            }));
        }
        let mut consumer = Consumer {
            texts: &texts,
            counts,
            stale,
        };
        dispatch_frame(&mut events, &mut consumer);
        counts = consumer.counts;
        stale = consumer.stale;
        if !events.is_empty() {
            return Err("events retained after frame");
        }
    }
    let allocations = allocation_counter::stop();
    println!(
        "{{\"scope\":\"headless event dispatch, not gameplay or sound playback\",\"frames\":10000,\"sounds\":{},\"effects\":{},\"prints\":{},\"stale_texts\":{stale},\"remaining_events\":{},\"overwritten_events\":{},\"allocations_after_load\":{allocations}}}",
        counts[0],
        counts[1],
        counts[2],
        events.len(),
        events.overwritten()
    );
    if counts == [2_560_000; 3] && stale == 0 && allocations == 0 {
        Ok(())
    } else {
        Err("dispatch or allocation mismatch")
    }
}
