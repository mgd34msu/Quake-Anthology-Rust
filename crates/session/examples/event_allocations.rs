#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;
use qa_core::events::{EventConsumer, dispatch_consumer};
use qa_core::{
    events::{EventRing, FrameEvent, OutputRecord, OutputSubmission, OutputTarget, TextStore},
    primitives::*,
};

struct Consumer {
    counts: [u64; 3],
    stale: u64,
}
impl EventConsumer for Consumer {
    fn submit(&mut self, record: OutputRecord, texts: &mut TextStore) -> OutputSubmission {
        match record.event {
            FrameEvent::Music(control) => {
                std::hint::black_box(control);
            }
            FrameEvent::StaticEntity(event) => {
                std::hint::black_box(event);
            }
            FrameEvent::ConfigString { text, .. } => {
                std::hint::black_box(texts.get(text));
            }
            FrameEvent::Native(event) | FrameEvent::GameState { payload: event, .. } => {
                std::hint::black_box(event);
            }
            FrameEvent::Hud(event) => {
                std::hint::black_box(event);
            }
            FrameEvent::ViewFeedback(event) => {
                std::hint::black_box(event);
            }
            FrameEvent::PlayerValue { id, value } => {
                std::hint::black_box((id, value));
            }
            FrameEvent::SetAngles(angles) => {
                std::hint::black_box(angles);
            }
            FrameEvent::Sound(event) => {
                std::hint::black_box(event);
                self.counts[0] += 1;
            }
            FrameEvent::Effect(event) => {
                std::hint::black_box(event);
                self.counts[1] += 1;
            }
            FrameEvent::Print(event) => {
                if texts.get(event.text) != Some(b"message\n".as_slice()) {
                    self.stale += 1;
                }
                self.counts[2] += 1;
            }
        }
        OutputSubmission::BestEffort
    }
}

fn main() -> Result<(), &'static str> {
    let mut events = EventRing::load(1024, 1, 512, 1024).map_err(|_| "ring capacity")?;
    let id = events
        .bind(OutputTarget::Client(ClientId(0)))
        .ok_or("consumer")?;
    let mut counts = [0; 3];
    let mut stale = 0;
    allocation_counter::start();
    for _ in 0..10_000 {
        for index in 0..256 {
            events
                .push(FrameEvent::Sound(SoundEvent {
                    sound: SoundId(index),
                    entity: None,
                    channel: 1,
                    position: Vec3::default(),
                    volume: 1.0,
                    attenuation: 1.0,
                    action: SoundAction::Play,
                    ..qa_core::primitives::SoundEvent::default()
                }))
                .map_err(|_| "publish")?;
            events
                .push(FrameEvent::Effect(EffectEvent {
                    effect: EffectId(index),
                    position: Vec3::default(),
                    direction: Vec3::default(),
                    count: 10,
                    ..EffectEvent::default()
                }))
                .map_err(|_| "publish")?;
            events
                .print(
                    Some(ClientId(0)),
                    PrintKind::Notify,
                    format_args!("message\n"),
                )
                .map_err(|_| "text capacity")?;
        }
        let mut consumer = Consumer { counts, stale };
        dispatch_consumer(&mut events, id, &mut consumer);
        counts = consumer.counts;
        stale = consumer.stale;
        if !events.is_empty() {
            return Err("events retained after frame");
        }
    }
    let allocations = allocation_counter::stop();
    println!(
        "{{\"scope\":\"headless event dispatch, not gameplay or sound playback\",\"frames\":10000,\"sounds\":{},\"effects\":{},\"prints\":{},\"stale_texts\":{stale},\"remaining_events\":{},\"consumer_overflow\":{},\"allocations_after_load\":{allocations}}}",
        counts[0],
        counts[1],
        counts[2],
        events.len(),
        events.counters(id).ok_or("consumer")?.overflow
    );
    if counts == [2_560_000; 3] && stale == 0 && allocations == 0 {
        Ok(())
    } else {
        Err("dispatch or allocation mismatch")
    }
}
