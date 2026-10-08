//! Com_Frame: events/commands, server, events/commands, client.
use crate::Runtime;
use qa_console::commands::Console;
use qa_core::{
    primitives::{ClientId, CvarHandle, ModuleId, UserCmd},
    sys_events::{EventKind, EventTime, SeatId, SysEventQueue},
};
use qa_input::{Input, Target};
use qa_platform::{EventPump, Window};
use qa_session::timing::{Tick, TickRate, TickTarget, Timeline};
use std::time::Duration;

/// The live adapter delegates every OS operation to platform. The same host
/// function can be checked headlessly with supplied event times.
pub trait FrameSource {
    fn begin_frame(&mut self, queue: &mut SysEventQueue);
    fn poll_events(&mut self, queue: &mut SysEventQueue);
    fn wait_events(&mut self, queue: &mut SysEventQueue, remaining: Duration);
    fn elapsed(&self) -> Duration;
    fn present(&mut self);
}
pub struct LiveFrame<'a> {
    pub pump: &'a mut EventPump,
    pub window: &'a mut Window,
}
impl FrameSource for LiveFrame<'_> {
    fn begin_frame(&mut self, queue: &mut SysEventQueue) {
        self.pump.begin_frame(self.window, queue);
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.pump.poll_events(self.window, queue);
    }
    fn wait_events(&mut self, queue: &mut SysEventQueue, remaining: Duration) {
        self.pump.wait_events(self.window, queue, remaining);
    }
    fn elapsed(&self) -> Duration {
        self.pump.elapsed()
    }
    fn present(&mut self) {
        self.window.present();
    }
}

pub struct Provider {
    pub module: ModuleId,
    pub rate: TickRate,
    pub frame: fn(&mut Runtime, Tick),
}
pub struct FrameHost {
    pub console: Console<Runtime>,
    pub input: Input,
    pub runtime: Runtime,
    pub queue: SysEventQueue,
    pub time: EventTime,
    pub key_downs: u64,
    pub key_repeats: u64,
    pub developer: CvarHandle,
    pub local_clients: [Option<ClientId>; SeatId::COUNT],
    maxfps: CvarHandle,
    previous: Option<EventTime>,
    timeline: Timeline,
    providers: Box<[Provider]>,
}

#[derive(Default)]
pub struct FrameResult {
    pub events: u64,
    pub drains: u64,
    pub server_ticks: u64,
    pub commands: [UserCmd; SeatId::COUNT],
    pub input_ns: u64,
    pub total_ns: u64,
}

impl FrameHost {
    pub fn load(
        console: Console<Runtime>,
        input: Input,
        runtime: Runtime,
        world_rate: TickRate,
        mut providers: Vec<Provider>,
    ) -> Result<Self, String> {
        let developer = console.cvars.find("developer").ok_or("missing developer")?;
        let maxfps = console
            .cvars
            .find("com_maxfps")
            .ok_or("missing com_maxfps")?;
        let timeline = Timeline::load(world_rate, providers.iter().map(|p| (p.module, p.rate)))
            .map_err(|e| format!("provider clocks: {e:?}"))?;
        providers.sort_unstable_by_key(|p| p.module.0);
        Ok(Self {
            console,
            input,
            runtime,
            queue: SysEventQueue::load(1024, 256 * 1024).map_err(|e| format!("{e:?}"))?,
            time: EventTime::default(),
            key_downs: 0,
            key_repeats: 0,
            developer,
            local_clients: [None; SeatId::COUNT],
            maxfps,
            previous: None,
            timeline,
            providers: providers.into_boxed_slice(),
        })
    }

    pub fn frame(&mut self, source: &mut impl FrameSource, uncapped: bool) -> FrameResult {
        let mut result = FrameResult::default();
        source.begin_frame(&mut self.queue);
        self.drain(&mut result);
        let previous = *self.previous.get_or_insert(self.time);
        let fps = self.console.cvars.integer(self.maxfps);
        // Preserve Com_Frame's integer-millisecond cap, including uncapped
        // values <= 0 and its zero period above 1000 fps.
        let period_ns = if uncapped || fps <= 0 {
            0
        } else {
            (1000 / fps) as u64 * 1_000_000
        };
        while self.time.since(previous) < period_ns && !self.runtime.quit {
            source.wait_events(
                &mut self.queue,
                Duration::from_nanos(period_ns - self.time.since(previous)),
            );
            self.drain(&mut result);
        }
        self.console.execute_frame(&mut self.runtime);
        if self.runtime.quit {
            return result;
        }
        let server_time = self.time;
        self.previous = Some(server_time);
        let runtime = &mut self.runtime;
        let providers = &self.providers;
        result.server_ticks = self
            .timeline
            .advance(server_time, |tick| match tick.target {
                TickTarget::World => {
                    runtime.server.world_time = tick.end;
                    runtime.server.world_frame = tick.index;
                }
                TickTarget::Provider(_) => {
                    (providers[tick.source_slot - 1].frame)(runtime, tick);
                }
            });
        // Immediate server->client packets take this path in the same frame.
        source.poll_events(&mut self.queue);
        self.drain(&mut result);
        self.console.execute_frame(&mut self.runtime);
        if self.runtime.quit {
            return result;
        }
        result.commands = self.client_frame();
        result.input_ns = source.elapsed().as_nanos() as u64;
        source.present();
        result.total_ns = source.elapsed().as_nanos() as u64;
        result
    }

    pub fn drain(&mut self, result: &mut FrameResult) {
        result.drains += 1;
        while let Some(event) = self.queue.pop() {
            result.events += 1;
            match event.kind {
                EventKind::Time => self.time = event.time,
                EventKind::Quit => self.runtime.quit = true,
                EventKind::ConsoleLine(text) => {
                    if self
                        .console
                        .append(text, self.console.cvars.context())
                        .is_ok()
                    {
                        let _ = self.console.append("\n", self.console.cvars.context());
                    }
                }
                EventKind::Packet {
                    socket,
                    from,
                    bytes,
                } => {
                    self.runtime
                        .network
                        .receive(socket, from, bytes, event.time);
                }
                _ => self.input.dispatch(
                    event,
                    &mut ConsoleInput {
                        console: &mut self.console,
                    },
                ),
            }
            if let EventKind::Key {
                down: true, repeat, ..
            } = event.kind
            {
                self.key_downs += 1;
                self.key_repeats += u64::from(repeat);
            }
            if !matches!(event.kind, EventKind::Time | EventKind::Packet { .. }) {
                qa_console::logger::dev_print(
                    &self.console.cvars,
                    self.developer,
                    1,
                    format_args!(
                        "{{\"event\":\"input_diagnostic\",\"time_ns\":{},\"input\":\"{}\"}}",
                        event.time.0,
                        event_name(event.kind)
                    ),
                );
            }
        }
    }

    fn client_frame(&mut self) -> [UserCmd; SeatId::COUNT] {
        // THE-735 supplies cached per-seat movement/mouse policies; these are
        // normalized routing units until movement/prediction and scenes exist.
        let commands =
            self.input
                .build_frame(self.time, [127; 3], [0.022; 2], [None; SeatId::COUNT]);
        for (seat, id) in self.local_clients.iter().enumerate() {
            if let Some(id) = id {
                self.runtime.server.clients[id.0 as usize].command = commands[seat];
            }
        }
        commands
    }
}

struct ConsoleInput<'a> {
    console: &'a mut Console<Runtime>,
}
impl Target for ConsoleInput<'_> {
    fn character(&mut self, _seat: SeatId, _value: char) {}
    fn command(&mut self, _seat: SeatId, text: &str) {
        let context = self.console.cvars.context();
        let _ = self.console.append(text, context);
        let _ = self.console.append("\n", context);
    }
}

fn event_name(kind: EventKind<'_>) -> &'static str {
    match kind {
        EventKind::Time => "time",
        EventKind::Key { .. } => "key",
        EventKind::Char { .. } => "char",
        EventKind::Mouse { .. } => "mouse",
        EventKind::MouseButton { .. } => "mouse_button",
        EventKind::MouseWheel { .. } => "mouse_wheel",
        EventKind::ControllerAxis { .. } => "controller_axis",
        EventKind::ControllerButton { .. } => "controller_button",
        EventKind::DeviceRemoved(_) => "device_removed",
        EventKind::Focus(_) => "focus",
        EventKind::Quit => "quit",
        EventKind::ConsoleLine(_) => "console_line",
        EventKind::Packet { .. } => "packet",
    }
}
