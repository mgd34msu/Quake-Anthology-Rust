//! Client-frame routing of the one output ring, before presentation.
use crate::{Runtime, host::FrameSource};
use qa_core::{
    events::TextStore,
    primitives::{ClientId, EffectEvent, PrintEvent, PrintKind, SoundEvent},
    sys_events::{EventTime, SeatId},
};
use qa_session::{
    clients::Server,
    events::{EventConsumer, dispatch_frame},
};

#[derive(Default)]
pub struct OutputCounts {
    pub sounds: u64,
    pub effects: u64,
    pub prints: u64,
    pub unhandled_sounds: u64,
    pub unhandled_effects: u64,
    pub stale_texts: u64,
}

pub fn dispatch(
    runtime: &mut Runtime,
    source: &mut impl FrameSource,
    local: &[Option<ClientId>; SeatId::COUNT],
    time: EventTime,
    notify_time: f64,
    center_time: f64,
) -> OutputCounts {
    let mut consumer = Consumer {
        source,
        server: &mut runtime.server,
        texts: &runtime.texts,
        local,
        now: time.0 as f64 * 1e-9,
        notify_time,
        center_time,
        counts: OutputCounts::default(),
    };
    dispatch_frame(&mut runtime.events, &mut consumer);
    for id in local.iter().flatten() {
        qa_ui::hud::expire_messages(
            &mut consumer.server.clients[id.0 as usize].hud,
            consumer.now,
        );
    }
    consumer.counts
}

struct Consumer<'a, S> {
    source: &'a mut S,
    server: &'a mut Server,
    texts: &'a TextStore,
    local: &'a [Option<ClientId>; SeatId::COUNT],
    now: f64,
    notify_time: f64,
    center_time: f64,
    counts: OutputCounts,
}
impl<S: FrameSource> EventConsumer for Consumer<'_, S> {
    fn sound(&mut self, event: SoundEvent) {
        self.counts.sounds += 1;
        self.counts.unhandled_sounds += u64::from(!self.source.sound(event));
    }
    fn effect(&mut self, event: EffectEvent) {
        self.counts.effects += 1;
        self.counts.unhandled_effects += u64::from(!self.source.effect(event));
    }
    fn print(&mut self, event: PrintEvent) {
        self.counts.prints += 1;
        let Some(text) = self.texts.get(event.text) else {
            self.counts.stale_texts += 1;
            return;
        };
        // Remote recipients go through the shared wire path when R11 lands.
        // Console broadcasts remain visible in dedicated/headless sessions.
        if event
            .client
            .is_some_and(|id| !self.local.contains(&Some(id)))
        {
            return;
        }
        if matches!(
            event.kind,
            PrintKind::Console | PrintKind::Notify | PrintKind::Chat
        ) {
            qa_console::logger::console_bytes(text);
        }
        if event.kind == PrintKind::Console {
            return;
        }
        let mut visited = 0u64;
        for &id in self.local.iter().flatten() {
            let bit = 1u64 << id.0;
            if visited & bit == 0 && event.client.is_none_or(|to| to == id) {
                visited |= bit;
                qa_ui::hud::print(
                    &mut self.server.clients[id.0 as usize].hud,
                    event,
                    self.now,
                    self.notify_time,
                    self.center_time,
                );
            }
        }
    }
}
