use qa_core::{
    events::{EventRing, FrameEvent},
    primitives::{EffectEvent, PrintEvent, SoundEvent},
};

pub trait EventConsumer {
    fn sound(&mut self, event: SoundEvent);
    fn effect(&mut self, event: EffectEvent);
    fn print(&mut self, event: PrintEvent);
}

/// Called once after simulation, before audio, particles and seat HUDs update.
pub fn dispatch_frame(events: &mut EventRing, consumer: &mut impl EventConsumer) {
    for event in events.drain() {
        match event {
            FrameEvent::Sound(sound) => consumer.sound(sound),
            FrameEvent::Effect(effect) => consumer.effect(effect),
            FrameEvent::Print(print) => consumer.print(print),
        }
    }
}
