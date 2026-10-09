//! Com_Frame: events/commands, server, events/commands, client.
use crate::Runtime;
use qa_console::commands::Console;
use qa_core::{
    primitives::{ClientId, CvarHandle, EffectEvent, ModuleId, SoundEvent, UserCmd},
    sys_events::{EventKind, EventTime, SeatId, SysEventQueue},
};
use qa_input::Target;
use qa_platform::{EventPump, Stopwatch, Window};
use qa_session::timing::{Tick, TickRate, TickTarget, Timeline};
use std::time::Duration;

/// CLIENT camera values are copied after snapshot application and prediction.
/// This is a fixed per-seat submission, not a second player or command store.
#[derive(Clone, Copy, Debug)]
pub struct ClientView {
    pub world: qa_render::world::WorldId,
    pub origin: qa_core::primitives::Vec3,
    pub angles: qa_core::primitives::Vec3,
    pub fov_x: f32,
    pub time_ms: u64,
}

/// The live adapter delegates every OS operation to platform. The same host
/// function can be checked headlessly with supplied event times.
pub trait FrameSource {
    /// Starts measurement and samples platform time without collecting events.
    fn begin_frame(&mut self) -> EventTime;
    fn poll_events(&mut self, queue: &mut SysEventQueue);
    /// Waits on time only. Physical intake occurs in the two poll_events calls.
    fn wait_time(&mut self, remaining: Duration) -> EventTime;
    fn elapsed(&self) -> Duration;
    fn render(&mut self, _views: &[Option<ClientView>; SeatId::COUNT]) {}
    fn present(&mut self);
    /// False reports an unloaded backend, not successful audio/particle proof.
    fn sound(&mut self, _event: SoundEvent) -> bool {
        false
    }
    fn effect(&mut self, _event: EffectEvent) -> bool {
        false
    }
    /// Native adapters select submission/ACK rules from their protocol tables.
    /// No channel currently loaded means no submission, rather than a fake ACK.
    /// Completes a bounded native reset using already-drained channel state.
    /// This callback must not poll SDL, sockets, stdin or physical time.
    fn resync_output(&mut self, _client: ClientId) -> bool {
        false
    }
    fn output(
        &mut self,
        _client: ClientId,
        _event: qa_core::events::FrameEvent,
        _text: Option<&[u8]>,
    ) -> qa_core::events::OutputSubmission {
        qa_core::events::OutputSubmission::Unsent
    }
}
pub struct LiveFrame<'a> {
    pub pump: &'a mut EventPump,
    pub window: &'a mut Window,
    pub renderer: &'a mut crate::renderer::Renderer,
}
impl FrameSource for LiveFrame<'_> {
    fn begin_frame(&mut self) -> EventTime {
        self.pump.begin_frame()
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.pump.poll_events(self.window, queue);
    }
    fn wait_time(&mut self, remaining: Duration) -> EventTime {
        self.pump.wait_time(remaining)
    }
    fn elapsed(&self) -> Duration {
        self.pump.elapsed()
    }
    fn present(&mut self) {
        self.renderer.present(self.window);
    }
    fn render(&mut self, views: &[Option<ClientView>; SeatId::COUNT]) {
        self.renderer.frame(views);
    }
}

pub struct Provider {
    pub module: ModuleId,
    pub rate: TickRate,
    pub frame: fn(&mut Runtime, Tick),
    pub output: Option<
        fn(&mut Runtime, Tick, qa_core::events::OutputRecord) -> qa_core::events::OutputSubmission,
    >,
}
pub struct FrameHost {
    pub console: Console<Runtime>,
    pub runtime: Runtime,
    pub queue: SysEventQueue,
    pub time: EventTime,
    pub key_downs: u64,
    pub key_repeats: u64,
    pub developer: CvarHandle,
    pub local_clients: [Option<ClientId>; SeatId::COUNT],
    /// A seat's world/presentation choice is independent of its physics rules.
    pub local_worlds: [Option<qa_render::world::WorldId>; SeatId::COUNT],
    maxfps: CvarHandle,
    fov: CvarHandle,
    input_handles: crate::profile::InputHandles,
    notify_time: CvarHandle,
    center_time: CvarHandle,
    previous: Option<EventTime>,
    timeline: Timeline,
    providers: Box<[Provider]>,
    module_outputs: Box<[Option<qa_core::events::OutputConsumerId>]>,
}

#[derive(Default)]
pub struct FrameResult {
    pub events: u64,
    pub drains: u64,
    pub server_ticks: u64,
    pub commands: [UserCmd; SeatId::COUNT],
    pub input_ns: u64,
    pub simulation_ns: u64,
    pub client_ns: u64,
    pub total_ns: u64,
    pub output: crate::output::OutputCounts,
    pub output_drains: u64,
}

impl FrameHost {
    pub fn load(
        console: Console<Runtime>,
        mut runtime: Runtime,
        world_rate: TickRate,
        mut providers: Vec<Provider>,
    ) -> Result<Self, String> {
        let developer = console.cvars.find("developer").ok_or("missing developer")?;
        let maxfps = console
            .cvars
            .find("com_maxfps")
            .ok_or("missing com_maxfps")?;
        let fov = console.cvars.find("cg_fov").ok_or("missing cg_fov")?;
        let input_handles = crate::profile::InputHandles::load(&console.cvars)?;
        let notify_time = console
            .cvars
            .find("con_notifytime")
            .ok_or("missing con_notifytime")?;
        let center_time = console
            .cvars
            .find("cg_centertime")
            .ok_or("missing cg_centertime")?;
        let timeline = Timeline::load(world_rate, providers.iter().map(|p| (p.module, p.rate)))
            .map_err(|e| format!("provider clocks: {e:?}"))?;
        providers.sort_unstable_by_key(|p| p.module.0);
        let module_outputs = providers
            .iter()
            .map(|p| {
                if p.output.is_some() {
                    runtime
                        .server
                        .events
                        .bind(qa_core::events::OutputTarget::Module(p.module))
                        .ok_or("module output capacity")
                        .map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<Box<[_]>, _>>()?;
        Ok(Self {
            console,
            runtime,
            queue: SysEventQueue::load(1024, 256 * 1024).map_err(|e| format!("{e:?}"))?,
            time: EventTime::default(),
            key_downs: 0,
            key_repeats: 0,
            developer,
            local_clients: [None; SeatId::COUNT],
            local_worlds: [None; SeatId::COUNT],
            maxfps,
            fov,
            input_handles,
            notify_time,
            center_time,
            previous: None,
            timeline,
            providers: providers.into_boxed_slice(),
            module_outputs,
        })
    }

    pub fn frame(&mut self, source: &mut impl FrameSource, uncapped: bool) -> FrameResult {
        let mut result = FrameResult::default();
        let mut now = source.begin_frame();
        // Native lastTime starts at zero and is reset if the clock goes back.
        let previous_ms = self
            .previous
            .unwrap_or_default()
            .milliseconds()
            .min(now.milliseconds());
        let fps = self.console.cvars.integer(self.maxfps);
        // Preserve Com_Frame's integer-millisecond cap, including uncapped
        // values <= 0 and its zero period above 1000 fps.
        let period_ms = if uncapped || fps <= 0 {
            0
        } else {
            (1000 / fps) as u64
        };
        let deadline_ns = (previous_ms + period_ms) * 1_000_000;
        while now.0 < deadline_ns && !self.runtime.quit {
            now = source.wait_time(Duration::from_nanos(deadline_ns - now.0));
        }
        // Owner ruling: both intake points physically poll SDL/stdin/UDP, and
        // neither the cap wait nor any other frame phase performs intake.
        source.poll_events(&mut self.queue);
        self.drain(&mut result);
        self.console.execute_frame(&mut self.runtime);
        if self.runtime.quit {
            self.dispatch_output(source, &mut result);
            return result;
        }
        let server_time = self.time;
        self.previous = Some(server_time);
        let simulation = Stopwatch::start();
        let runtime = &mut self.runtime;
        let providers = &self.providers;
        let module_outputs = &mut self.module_outputs;
        let input_handles = &self.input_handles;
        let vars = &self.console.cvars;
        result.server_ticks = self
            .timeline
            .advance(server_time, |tick| match tick.target {
                TickTarget::World => {
                    runtime.server.world_time = tick.end;
                    runtime.server.world_frame = tick.index;
                    runtime
                        .server
                        .build_bot_commands(tick.start, tick.end, |rules| {
                            input_handles.policy(vars, rules)
                        });
                    if let Some(world) = &mut runtime.collision {
                        runtime.server.move_pending_clients(
                            &runtime.geometry,
                            world.geometry,
                            world.index,
                            &mut world.scratch,
                        );
                    }
                }
                TickTarget::Provider(_) => {
                    let slot = tick.source_slot - 1;
                    let provider = &providers[slot];
                    (provider.frame)(runtime, tick);
                    if let (Some(consume), Some(id)) = (provider.output, module_outputs[slot]) {
                        // Local module delivery resumes at its next native tick.
                        let id = if runtime.server.events.needs_resync(id) {
                            let Some(id) = runtime.server.events.resume(id) else {
                                return;
                            };
                            module_outputs[slot] = Some(id);
                            id
                        } else {
                            id
                        };
                        if let Some(mut batch) = runtime.server.events.batch(id) {
                            while let Some(record) = runtime.server.events.next(&mut batch) {
                                let submission = consume(runtime, tick, record);
                                runtime
                                    .server
                                    .events
                                    .submit(id, record.sequence, submission);
                            }
                        }
                    }
                }
            });
        if let Some(world) = &mut runtime.collision {
            runtime.server.move_pending_clients(
                &runtime.geometry,
                world.geometry,
                world.index,
                &mut world.scratch,
            );
        }
        result.simulation_ns = simulation.elapsed().as_nanos() as u64;
        // Immediate server->client packets take this path in the same frame.
        source.poll_events(&mut self.queue);
        self.drain(&mut result);
        self.console.execute_frame(&mut self.runtime);
        if self.runtime.quit {
            self.dispatch_output(source, &mut result);
            return result;
        }
        let client = Stopwatch::start();
        result.commands = self.client_frame();
        result.client_ns = client.elapsed().as_nanos() as u64;
        self.dispatch_output(source, &mut result);
        result.input_ns = source.elapsed().as_nanos() as u64;
        source.render(&self.client_views());
        source.present();
        result.total_ns = source.elapsed().as_nanos() as u64;
        result
    }

    #[expect(
        clippy::question_mark,
        reason = "Preserve the explicit disconnected-client filter before constructing each seat view"
    )]
    pub fn client_views(&self) -> [Option<ClientView>; SeatId::COUNT] {
        std::array::from_fn(|seat| {
            let client = self.local_clients[seat]?;
            let world = self.local_worlds[seat]?;
            if self.runtime.server.clients[client.0 as usize]
                .connection
                .is_none()
            {
                return None;
            }
            let player = &self.runtime.prediction[seat].player;
            Some(ClientView {
                world,
                origin: player.body.position + player.view_offset,
                angles: player.view_angles,
                fov_x: self.console.cvars.value(self.fov),
                time_ms: self.time.milliseconds(),
            })
        })
    }

    pub fn native_input_policy_active(&self) -> bool {
        self.runtime.collision.is_some() && self.local_clients.iter().any(Option::is_some)
    }

    fn dispatch_output(&mut self, source: &mut impl FrameSource, result: &mut FrameResult) {
        result.output = crate::output::dispatch(
            &mut self.runtime,
            source,
            &self.local_clients,
            self.time,
            f64::from(self.console.cvars.value(self.notify_time)),
            f64::from(self.console.cvars.value(self.center_time)),
        );
        result.output_drains += 1;
    }

    pub fn drain(&mut self, result: &mut FrameResult) {
        result.drains += 1;
        loop {
            while let Some(event) = self.queue.pop() {
                self.runtime.input_time = event.time;
                result.events += 1;
                match event.kind {
                    EventKind::Time => {
                        self.time = event.time;
                        self.runtime.input.seed(event.time);
                    }
                    EventKind::Quit => self.runtime.quit = true,
                    EventKind::ConsoleLine(text) => {
                        let context = qa_console::views::Context {
                            event_time: Some(event.time),
                            ..self.console.cvars.context()
                        };
                        let _ = self.console.append_line(text, context);
                    }
                    EventKind::Packet {
                        socket,
                        from,
                        bytes,
                    } => {
                        let server = &mut self.runtime.server;
                        self.runtime.network.receive(
                            socket,
                            from,
                            bytes,
                            event.time,
                            |_, endpoint, incoming| {
                                if endpoint == qa_core::loopback::Endpoint::Server
                                    && let qa_network::ingress::Incoming::Acknowledged {
                                        receipt,
                                        output: Some(output),
                                    } = incoming
                                {
                                    server.events.acknowledge(output, receipt);
                                }
                            },
                        );
                    }
                    _ => self.runtime.input.dispatch(
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
            // Com_EventLoop drains local client packets, then server packets when
            // system events run out. Both use the same packet consumer as UDP.
            if self.runtime.loopback.enqueue(&mut self.queue, self.time) == 0 {
                break;
            }
        }
    }

    fn client_frame(&mut self) -> [UserCmd; SeatId::COUNT] {
        let mut commands = if self.native_input_policy_active() {
            let policies = std::array::from_fn(|seat| {
                let rules = self.local_clients[seat].map_or(
                    qa_core::primitives::RuleSetId::default(),
                    |id| {
                        self.runtime.server.clients[id.0 as usize]
                            .player
                            .movement_rules
                    },
                );
                let mut policy = self.input_handles.policy(&self.console.cvars, rules);
                if let Some(id) = self.local_clients[seat] {
                    policy.delta_pitch = self.runtime.server.clients[id.0 as usize]
                        .player
                        .movement
                        .delta_angles
                        .0[0];
                }
                policy
            });
            self.runtime
                .input
                .build_frame_with_policy(self.time, &policies)
        } else {
            self.runtime
                .input
                .build_frame(self.time, [127; 3], [0.022; 2])
        };
        for (seat, command) in commands.iter_mut().enumerate() {
            let rules =
                self.local_clients[seat].map_or(qa_core::primitives::RuleSetId::default(), |id| {
                    self.runtime.server.clients[id.0 as usize]
                        .player
                        .movement_rules
                });
            *command = qa_movement::prepare_command(rules, *command);
        }
        for (seat, id) in self.local_clients.iter().enumerate() {
            if let Some(id) = id {
                self.runtime.server.submit_command(*id, commands[seat]);
                if let Some(world) = &mut self.runtime.collision {
                    let prediction = &mut self.runtime.prediction[seat];
                    prediction.apply_snapshot(&self.runtime.server.clients[id.0 as usize].player);
                    let client = &self.runtime.server.clients[id.0 as usize];
                    let mut trace = qa_world::collision::WorldTrace::new(
                        &self.runtime.geometry,
                        world.geometry,
                        world.index,
                        &self.runtime.server.entities,
                        &self.runtime.server.area,
                        &mut world.scratch,
                        Some(client.entity),
                    );
                    prediction.advance(commands[seat], &mut trace);
                    if matches!(
                        prediction.player.movement_rules,
                        qa_core::primitives::RuleSetId::Quake
                            | qa_core::primitives::RuleSetId::QuakeWorld
                    ) {
                        let policy = self
                            .input_handles
                            .policy(&self.console.cvars, prediction.player.movement_rules);
                        if let Some(seat_id) = SeatId::new(seat as u8) {
                            prediction.player.view_angles = self.runtime.input.drift_view(
                                seat_id,
                                self.time,
                                policy,
                                &prediction.player,
                                commands[seat].movement[0],
                            );
                        }
                    }
                }
            }
        }
        if self.runtime.collision.is_some() {
            let mut predicted_poses = std::array::from_fn::<_, { SeatId::COUNT }, _>(|seat| {
                let client = self.local_clients[seat]?;
                let client = &self.runtime.server.clients[client.0 as usize];
                client.connection.map(|_| {
                    (
                        client.entity,
                        self.runtime.prediction[seat].player.body.position,
                    )
                })
            });
            self.runtime
                .server
                .area
                .predict_attachments(&self.runtime.server.entities, &mut predicted_poses);
            for (seat, pose) in predicted_poses.iter().enumerate() {
                if let Some((_, position)) = pose {
                    self.runtime.prediction[seat].player.body.position = *position;
                }
            }
        }
        self.runtime.targets.refresh(
            &mut self.runtime.server.entities,
            &self.runtime.catalog.names,
        );
        for client in &mut self.runtime.server.clients {
            if client.connection.is_some() {
                self.runtime
                    .catalog
                    .hud
                    .update(&client.player, &mut client.hud);
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
    fn command(&mut self, seat: SeatId, time: EventTime, text: &str) {
        let context = qa_console::views::Context {
            seat,
            event_time: Some(time),
            ..self.console.cvars.context()
        };
        let _ = self.console.append_line(text, context);
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
