//! Com_Frame: events/commands, server, events/commands, client.
use crate::Runtime;
use qa_console::commands::Console;
use qa_core::{
    primitives::{
        ClientId, CollisionOwner, CvarHandle, EffectEvent, ModuleId, SoundEvent, UserCmd,
    },
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
    fn begin_frame(&mut self) -> Duration;
    fn poll_events(&mut self, queue: &mut SysEventQueue);
    /// Selection only; SDL applies it during an existing physical intake.
    fn request_mouse_capture(&mut self, _requested: bool) {}
    /// Waits on time only. Physical intake occurs in the two poll_events calls.
    fn wait_time(&mut self, remaining: Duration) -> Duration;
    fn elapsed(&self) -> Duration;
    fn render(
        &mut self,
        _views: &[Option<ClientView>; SeatId::COUNT],
        _scenes: &[Option<crate::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        _console: qa_console::presentation::ConsoleView<'_>,
    ) {
    }
    fn present(&mut self);
    /// False reports an unloaded backend, not successful audio/particle proof.
    fn sound(&mut self, _event: SoundEvent) -> bool {
        false
    }
    fn listeners(
        &mut self,
        _listeners: &[Option<qa_audio::Listener>; SeatId::COUNT],
        _volume: f32,
    ) {
    }
    fn mix_audio(
        &mut self,
        _entities: &[Option<crate::client_scene::ClientEntities<'_>>; SeatId::COUNT],
    ) {
    }
    fn effect(&mut self, _event: EffectEvent) -> bool {
        false
    }
    /// Native encoding, sequencing and ACKs belong to the shared Channel.
    /// This primitive transport reports only datagram admission, never an ACK.
    fn send_packet(&mut self, _socket: u16, _to: std::net::SocketAddr, _bytes: &[u8]) -> bool {
        false
    }
}
pub struct LiveFrame<'a> {
    pub pump: &'a mut EventPump,
    pub window: &'a mut Window,
    pub renderer: &'a mut crate::renderer::Renderer,
    pub audio: &'a mut crate::audio::Output,
}
impl FrameSource for LiveFrame<'_> {
    fn sound(&mut self, event: SoundEvent) -> bool {
        self.audio.mixer.sound(event)
    }
    fn listeners(&mut self, listeners: &[Option<qa_audio::Listener>; SeatId::COUNT], volume: f32) {
        self.audio.mixer.listen(listeners, volume);
    }
    fn mix_audio(
        &mut self,
        entities: &[Option<crate::client_scene::ClientEntities<'_>>; SeatId::COUNT],
    ) {
        self.audio.submit(entities);
    }
    fn send_packet(&mut self, socket: u16, to: std::net::SocketAddr, bytes: &[u8]) -> bool {
        self.pump.send_udp(socket, to, bytes).is_ok()
    }
    fn begin_frame(&mut self) -> Duration {
        self.pump.begin_frame()
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.pump.poll_events(self.window, queue);
    }
    fn request_mouse_capture(&mut self, requested: bool) {
        self.window.request_mouse_capture(requested);
    }
    fn wait_time(&mut self, remaining: Duration) -> Duration {
        self.pump.wait_time(remaining)
    }
    fn elapsed(&self) -> Duration {
        self.pump.elapsed()
    }
    fn present(&mut self) {
        self.renderer.present(self.window);
    }
    fn render(
        &mut self,
        views: &[Option<ClientView>; SeatId::COUNT],
        scenes: &[Option<crate::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        console: qa_console::presentation::ConsoleView<'_>,
    ) {
        self.renderer.frame(views, scenes, console);
    }
}

#[derive(Clone, Copy)]
pub struct Provider {
    pub module: ModuleId,
    pub rate: TickRate,
    pub frame: fn(&mut FrameHost, Tick),
    pub output: Option<
        fn(
            &mut [qa_session::clients::Client],
            Tick,
            qa_core::events::OutputRecord,
            &mut qa_core::events::TextStore,
        ) -> qa_core::events::OutputSubmission,
    >,
}
pub struct FrameHost {
    pub(crate) modules: Option<crate::modules::Modules>,
    pub(crate) muzzle_sounds: crate::audio::MuzzleSounds,
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
    roll_angle: CvarHandle,
    roll_speed: CvarHandle,
    input_handles: crate::profile::InputHandles,
    pub(crate) movement_handles: qa_console::movement::MovementHandles,
    notify_time: CvarHandle,
    center_time: CvarHandle,
    volume: CvarHandle,
    mouse_enabled: CvarHandle,
    previous: Option<EventTime>,
    timeline: Timeline,
    providers: Box<[Provider]>,
    module_outputs: Box<[Option<qa_core::events::OutputConsumerId>]>,
    pub(crate) command_tokens: qa_console::command_text::Tokens,
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
        mut console: Console<Runtime>,
        mut runtime: Runtime,
        world_rate: TickRate,
        mut providers: Vec<Provider>,
    ) -> Result<Self, String> {
        let localization = console
            .cvars
            .bind("loc_file", console.cvars.context())
            .ok_or("missing loc_file")?;
        let path = console
            .cvars
            .read(localization)
            .map_err(|_| "invalid loc_file")?;
        let path = if path.as_str().is_empty() {
            "localization/loc_english.txt"
        } else {
            path.as_str()
        };
        if let Some(file) = runtime.vfs.open(path.as_bytes()) {
            let bytes = runtime
                .vfs
                .read_to_vec(
                    file,
                    16 * 1024 * 1024,
                    &mut qa_formats::archive::ArchiveReader::default(),
                )
                .map_err(|e| format!("localization: {e}"))?;
            runtime.localization = qa_content::localization::Catalog::load(&bytes)
                .map_err(|e| format!("localization: {e:?}"))?;
        }
        console.register("play", crate::audio::play);
        let volume = console.cvars.find("s_volume").ok_or("missing s_volume")?;
        let mouse_enabled = console.cvars.find("in_mouse").ok_or("missing in_mouse")?;
        let developer = console.cvars.find("developer").ok_or("missing developer")?;
        let maxfps = console
            .cvars
            .find("com_maxfps")
            .ok_or("missing com_maxfps")?;
        let fov = console.cvars.find("cg_fov").ok_or("missing cg_fov")?;
        let roll_angle = console
            .cvars
            .find("cl_rollangle")
            .ok_or("missing cl_rollangle")?;
        let roll_speed = console
            .cvars
            .find("cl_rollspeed")
            .ok_or("missing cl_rollspeed")?;
        let input_handles = crate::profile::InputHandles::load(&console.cvars)?;
        let movement_handles = qa_console::movement::MovementHandles::bind(&console.cvars)?;
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
            modules: None,
            muzzle_sounds: crate::audio::MuzzleSounds::default(),
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
            roll_angle,
            roll_speed,
            input_handles,
            movement_handles,
            notify_time,
            center_time,
            volume,
            mouse_enabled,
            previous: None,
            command_tokens: Default::default(),
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
            .min(now.as_millis().min(u128::from(u64::MAX)) as u64);
        let fps = self.console.cvars.integer(self.maxfps);
        // Preserve Com_Frame's integer-millisecond cap, including uncapped
        // values <= 0 and its zero period above 1000 fps.
        let period_ms = if uncapped || fps <= 0 {
            0
        } else {
            (1000 / fps) as u64
        };
        let deadline_ns = EventTime::from_milliseconds(previous_ms + period_ms).0;
        let deadline = Duration::from_nanos(deadline_ns);
        while now < deadline && !self.runtime.quit {
            now = source.wait_time(deadline - now);
        }
        // Owner ruling: both intake points physically poll SDL/stdin/UDP, and
        // neither the cap wait nor any other frame phase performs intake.
        source.request_mouse_capture(self.mouse_capture_requested());
        source.poll_events(&mut self.queue);
        self.drain(&mut result);
        self.console.execute_frame(&mut self.runtime);
        if self.runtime.quit {
            source.request_mouse_capture(false);
            self.shutdown_modules();
            self.dispatch_output(source, &mut result);
            return result;
        }
        self.initialize_modules(crate::modules::Phase::Server);
        let server_time = self.time;
        let frame_span = EventTime(server_time.since(self.previous.unwrap_or_default()));
        let simulation = Stopwatch::start();
        while let Some(tick) = self.timeline.next_tick(server_time) {
            result.server_ticks += 1;
            match tick.target {
                TickTarget::World => {
                    self.runtime.server.world_time = tick.end;
                    self.runtime.server.world_frame = tick.index;
                    self.runtime
                        .server
                        .build_bot_commands(tick.start, tick.end, |rules| {
                            self.input_handles.policy(&self.console.cvars, rules)
                        });
                    self.run_pending_commands(frame_span);
                }
                TickTarget::Provider(_) => {
                    let slot = tick.source_slot - 1;
                    let provider = self.providers[slot];
                    (provider.frame)(self, tick);
                    if let (Some(consume), Some(id)) =
                        (provider.output, self.module_outputs[slot].as_mut())
                    {
                        let server = &mut self.runtime.server;
                        if !server.events.resume_if_needed(id) {
                            continue;
                        }
                        let clients = &mut server.clients;
                        qa_core::events::dispatch_consumer(
                            &mut server.events,
                            *id,
                            &mut |record, texts: &mut qa_core::events::TextStore| {
                                consume(clients, tick, record, texts)
                            },
                        );
                    }
                }
            }
        }
        self.run_pending_commands(frame_span);
        result.simulation_ns = simulation.elapsed().as_nanos() as u64;
        self.publish_module_configs();
        for id in self.local_clients.into_iter().flatten() {
            let observation = self.snapshot_observation(id, server_time);
            self.runtime
                .send_local_snapshot(id, server_time, observation);
        }
        // Immediate server->client packets take this path in the same frame.
        source.request_mouse_capture(self.mouse_capture_requested());
        source.poll_events(&mut self.queue);
        self.drain(&mut result);
        self.previous = Some(server_time);
        self.console.execute_frame(&mut self.runtime);
        if self.runtime.quit {
            source.request_mouse_capture(false);
            self.shutdown_modules();
            self.dispatch_output(source, &mut result);
            return result;
        }
        let client = Stopwatch::start();
        self.advance_client_presentation();
        result.commands = self.client_frame();
        self.client_modules();
        result.client_ns = client.elapsed().as_nanos() as u64;
        self.dispatch_output(source, &mut result);
        result.input_ns = source.elapsed().as_nanos() as u64;
        source.render(
            &self.client_views(),
            &self.client_scene_inputs(),
            qa_console::presentation::ConsoleView {
                editor: &self.console.editor,
                scrollback: &self.console.scrollback,
                texts: &self.runtime.server.events.texts,
                rules: self.console.cvars.context().source,
                time: self.time,
            },
        );
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
            let (_, _, player) = self.local_seat(seat)?;
            let world = self.local_worlds[seat]?;
            Some(ClientView {
                world,
                origin: player.eye(),
                angles: player.view_angles,
                fov_x: self.console.cvars.value(self.fov),
                time_ms: self.time.milliseconds(),
            })
        })
    }

    /// Pose readers do not require a loaded renderer or model catalog.
    pub fn client_entities_inputs(
        &self,
    ) -> [Option<crate::client_scene::ClientEntities<'_>>; SeatId::COUNT] {
        std::array::from_fn(|seat| {
            let (client, _, _) = self.local_seat(seat)?;
            let snapshot = self
                .runtime
                .local_snapshots
                .get(client.0 as usize)?
                .as_ref()?;
            let channel = &self
                .runtime
                .network
                .get(client, qa_core::loopback::Endpoint::Client)?
                .channel;
            Some(crate::client_scene::ClientEntities {
                client,
                current: channel.current_snapshot(),
                previous: channel.previous_snapshot(),
                projection: snapshot.entity_projection(),
                interpolation: snapshot.interpolation(),
            })
        })
    }

    pub fn client_scene_inputs(
        &self,
    ) -> [Option<crate::client_scene::ClientSceneInput<'_>>; SeatId::COUNT] {
        let alias_roll = qa_gameplay::rules::appearance::AliasRoll {
            angle: self
                .console
                .cvars
                .value_in(self.roll_angle, qa_core::primitives::RuleSetId::QuakeWorld),
            speed: self
                .console
                .cvars
                .value_in(self.roll_speed, qa_core::primitives::RuleSetId::QuakeWorld),
        };
        let entities = self.client_entities_inputs();
        std::array::from_fn(|seat| {
            let (_, _, player) = self.local_seat(seat)?;
            let entities = entities[seat]?;
            let client = entities.client;
            let snapshot = self
                .runtime
                .local_snapshots
                .get(client.0 as usize)?
                .as_ref()?;
            let resources = self
                .runtime
                .client_resources
                .get(client.0 as usize)?
                .as_ref()?;
            let channel = &self
                .runtime
                .network
                .get(client, qa_core::loopback::Endpoint::Client)?
                .channel;
            Some(crate::client_scene::ClientSceneInput {
                client,
                current: entities.current,
                previous: entities.previous,
                projection: entities.projection,
                player_projection: snapshot.player_projection(),
                bindings: &resources.state.models,
                models: self.runtime.models.as_deref()?,
                player,
                alias_roll,
                interpolation: entities.interpolation,
                gun_animation: snapshot.gun_animation(),
                gun_previous: snapshot
                    .gun_animation()
                    .and_then(|animation| animation.previous)
                    .and_then(|sequence| channel.snapshot(sequence)),
                view_entity: snapshot.view_entity(),
            })
        })
    }

    fn advance_client_presentation(&mut self) {
        for snapshot in self.runtime.local_snapshots.iter_mut().flatten() {
            if let Some(connection) = self
                .runtime
                .network
                .get(snapshot.client, qa_core::loopback::Endpoint::Client)
            {
                snapshot.advance_presentation(&connection.channel, self.time);
            }
        }
    }

    fn local_seat(
        &self,
        seat: usize,
    ) -> Option<(
        qa_core::primitives::ClientId,
        &qa_session::clients::Client,
        &qa_core::primitives::PlayerState,
    )> {
        let client = self.local_clients.get(seat).copied().flatten()?;
        let row = self.runtime.server.clients.get(client.0 as usize)?;
        row.connection?;
        Some((client, row, &self.runtime.prediction.get(seat)?.player))
    }

    pub fn native_input_policy_active(&self) -> bool {
        self.runtime.collision.is_some() && self.local_clients.iter().any(Option::is_some)
    }
    fn mouse_capture_requested(&self) -> bool {
        !self.runtime.quit
            && self.console.editor.seat().is_none()
            && self.console.cvars.integer(self.mouse_enabled) != 0
            && self
                .local_worlds
                .iter()
                .zip(self.local_clients)
                .any(|(world, client)| {
                    world.is_some()
                        && client
                            .and_then(|id| self.runtime.server.clients.get(id.0 as usize))
                            .is_some_and(|row| {
                                row.connection == Some(qa_session::clients::Connection::Local)
                            })
                })
    }

    fn dispatch_output(&mut self, source: &mut impl FrameSource, result: &mut FrameResult) {
        let listeners = std::array::from_fn(|seat| {
            let (client, row, player) = self.local_seat(seat)?;
            Some(qa_audio::Listener {
                client,
                entity: Some(
                    self.runtime.local_snapshots[client.0 as usize]
                        .as_ref()
                        .and_then(|snapshot| snapshot.view_entity())
                        .map_or(
                            qa_core::primitives::SoundEmitter::Entity(row.entity),
                            |slot| qa_core::primitives::SoundEmitter::Native {
                                namespace: qa_core::primitives::ResourceNamespace::Client(client),
                                slot: slot as i32,
                            },
                        ),
                ),
                origin: player.eye(),
                right: qa_core::math::angle_vectors(player.view_angles).right,
            })
        });
        source.listeners(&listeners, self.console.cvars.value(self.volume));
        let modules = &self.modules;
        result.output = crate::output::dispatch(
            &mut self.runtime,
            &mut |module, sound| modules.as_ref()?.sound_ordinal(module, sound),
            source,
            &mut self.console.scrollback,
            &self.local_clients,
            self.time,
            f64::from(self.console.cvars.value(self.notify_time)),
            f64::from(self.console.cvars.value(self.center_time)),
        );
        result.output_drains += 1;
        source.mix_audio(&self.client_entities_inputs());
    }

    fn receive_packet(
        &mut self,
        socket: u16,
        from: qa_core::sys_events::Peer,
        bytes: &[u8],
        time: EventTime,
    ) {
        // Ownership moves temporarily; the callback retains neither the packet
        // nor an extra usercmd queue, and the same native channels are restored.
        let mut network = std::mem::replace(
            &mut self.runtime.network,
            qa_network::ingress::Connections::load(0),
        );
        let mut disconnected = None;
        network.receive(socket, from, bytes, time, |id, endpoint, incoming| {
            use qa_network::ingress::Incoming;
            match endpoint {
                qa_core::loopback::Endpoint::Client => {
                    let server = &self.runtime.server;
                    if let Some(binding) = self.runtime.local_snapshots[id.0 as usize].as_ref()
                        && server.clients[id.0 as usize].connection
                            == Some(qa_session::clients::Connection::Local)
                        && server.clients[id.0 as usize].output == Some(binding.output)
                    {
                        let seat = binding.seat;
                        match incoming {
                            Incoming::Print(print) => {
                                let _ = self.runtime.server.events.print_level_bytes(
                                    None,
                                    Some(id),
                                    print.kind,
                                    print.level.unwrap_or(print.kind.default_level()),
                                    print.text,
                                );
                            }
                            Incoming::Cue { cue, current } => self.received_cue(id, cue, current),
                            Incoming::ConfigString(config) => {
                                if let Some(resources) =
                                    self.runtime.client_resources[id.0 as usize].as_mut()
                                {
                                    resources.apply(
                                        config.index as usize,
                                        config.text,
                                        self.runtime.models.as_deref(),
                                        self.runtime.sound_bank.as_deref(),
                                    );
                                }
                            }
                            Incoming::ReliableCommand { text, .. } => {
                                if let Some(resources) =
                                    self.runtime.client_resources[id.0 as usize].as_mut()
                                {
                                    resources.apply_command(
                                        text,
                                        self.runtime.models.as_deref(),
                                        self.runtime.sound_bank.as_deref(),
                                    );
                                }
                            }
                            incoming => {
                                if let Some(binding) =
                                    self.runtime.local_snapshots[id.0 as usize].as_mut()
                                {
                                    binding.apply(
                                        incoming,
                                        &mut self.runtime.prediction[seat.index()].player,
                                    );
                                }
                            }
                        }
                    }
                }
                qa_core::loopback::Endpoint::Server => match incoming {
                    Incoming::CommandString {
                        protocol,
                        text,
                        output,
                        ..
                    } => {
                        if output.is_some()
                            && self.runtime.server.clients[id.0 as usize].output == output
                        {
                            match self.client_string_command(id, protocol.rule_set(), text) {
                                Ok(std::ops::ControlFlow::Break(())) => {
                                    disconnected = Some(id);
                                    return std::ops::ControlFlow::Break(());
                                }
                                Ok(std::ops::ControlFlow::Continue(())) => {}
                                Err(error) => {
                                    let _ = self.runtime.server.events.console_formatted(
                                        format_args!("ClientCommand: {error}\n"),
                                    );
                                }
                            }
                        }
                    }
                    Incoming::Userinfo { text, output, .. } => {
                        if output.is_some()
                            && self.runtime.server.clients[id.0 as usize].output == output
                            && self.runtime.server.module_moves(id).is_some()
                            && let Err(error) = self.userinfo_q2_client(id, text)
                        {
                            let _ = self.runtime.server.events.console_formatted(format_args!(
                                "ClientUserinfoChanged: {error}\n"
                            ));
                        }
                    }
                    Incoming::Disconnect { output } => {
                        if output.is_some()
                            && self.runtime.server.clients[id.0 as usize].output == output
                        {
                            self.disconnect_client(id);
                            disconnected = Some(id);
                            return std::ops::ControlFlow::Break(());
                        }
                    }
                    Incoming::Command {
                        mut command,
                        native_weapon,
                        observation,
                        execution,
                        output,
                    } => {
                        if output.is_some()
                            && self.runtime.server.clients[id.0 as usize].output == output
                        {
                            if let Some(number) = native_weapon {
                                command.weapon = self.runtime.catalog.registry.native_weapon(
                                    qa_core::primitives::RuleSetId::Quake3,
                                    u16::from(number),
                                );
                            }
                            self.received_command(id, command, execution, observation);
                        }
                    }
                    Incoming::Acknowledged {
                        receipt,
                        output: Some(output),
                    } => {
                        if self.runtime.server.clients[id.0 as usize].output == Some(output) {
                            let publication = &mut self.runtime.config_publications[id.0 as usize];
                            self.runtime.server.events.acknowledge_with(
                                output,
                                receipt,
                                |record| {
                                    if let qa_core::events::FrameEvent::ConfigString {
                                        module,
                                        index,
                                        ..
                                    } = record.event
                                    {
                                        publication.acknowledged(
                                            module,
                                            index as usize,
                                            record.sequence,
                                        );
                                    }
                                },
                            );
                        }
                    }
                    _ => {}
                },
            }
            std::ops::ControlFlow::Continue(())
        });
        if let Some(id) = disconnected {
            network.unbind(id);
        }
        self.runtime.network = network;
    }

    pub fn drain(&mut self, result: &mut FrameResult) {
        result.drains += 1;
        // Move the sole queue while callbacks borrow the composed host. Its
        // closed placeholder owns no payload storage and admits no intake.
        let mut queue = std::mem::take(&mut self.queue);
        loop {
            while let Some(event) = queue.pop() {
                self.runtime.input_time = event.time;
                result.events += 1;
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
                match event.kind {
                    EventKind::Time => {
                        self.time = event.time;
                        self.runtime.input.seed(event.time);
                    }
                    EventKind::Quit => self.runtime.quit = true,
                    EventKind::ConsoleLine(text) => {
                        let seat = self.console.cvars.context().seat;
                        let mut target = ConsoleInput {
                            console: &mut self.console,
                            events: &mut self.runtime.server.events,
                            time: event.time,
                        };
                        target.command(seat, event.time, text);
                    }
                    EventKind::Packet {
                        socket,
                        from,
                        bytes,
                    } => {
                        self.receive_packet(socket, from, bytes, event.time);
                    }
                    _ => self.runtime.input.dispatch(
                        event,
                        &mut ConsoleInput {
                            console: &mut self.console,
                            events: &mut self.runtime.server.events,
                            time: event.time,
                        },
                    ),
                }
                self.console
                    .sync_editor_input(&mut self.runtime.input, event.time);
            }
            // Com_EventLoop drains local client packets, then server packets when
            // system events run out. Both use the same packet consumer as UDP.
            if self.runtime.loopback.enqueue(&mut queue, self.time) == 0 {
                break;
            }
        }
        self.queue = queue;
    }

    fn client_frame(&mut self) -> [UserCmd; SeatId::COUNT] {
        let rules: [_; SeatId::COUNT] = std::array::from_fn(|seat| {
            self.local_clients[seat].map_or(qa_core::primitives::RuleSetId::default(), |_| {
                self.runtime.prediction[seat].player.movement_rules
            })
        });
        let policies = self.native_input_policy_active().then(|| {
            std::array::from_fn(|seat| {
                let mut policy = self.input_handles.policy(&self.console.cvars, rules[seat]);
                if self.local_clients[seat].is_some() {
                    policy.delta_pitch =
                        self.runtime.prediction[seat].player.movement.delta_angles.0[0];
                }
                policy
            })
        });
        let mut commands = if let Some(policies) = &policies {
            self.runtime
                .input
                .build_frame_with_policy(self.time, policies)
        } else {
            self.runtime
                .input
                .build_frame(self.time, [127; 3], [0.022; 2])
        };
        for (seat, command) in commands.iter_mut().enumerate() {
            *command = qa_movement::prepare_command(rules[seat], *command);
        }
        for (seat, id) in self.local_clients.iter().enumerate() {
            if let Some(id) = id {
                self.runtime
                    .send_local_command(*id, &commands[seat], self.time);
                if let Some(world) = &mut self.runtime.collision {
                    let prediction = &mut self.runtime.prediction[seat];
                    let client = &self.runtime.server.clients[id.0 as usize];
                    let mut trace = qa_world::collision::WorldTrace::new(
                        &self.runtime.geometry,
                        world.geometry,
                        world.index,
                        &self.runtime.server.entities,
                        &self.runtime.server.area,
                        &mut world.scratch,
                        CollisionOwner::Lifetime(client.entity),
                    );
                    let base = prediction.player.movement.tuning;
                    prediction.player.movement.tuning = self.movement_handles.resolve(
                        &self.console.cvars,
                        prediction.player.movement_rules,
                        base,
                    );
                    prediction.advance(commands[seat], &mut trace);
                    prediction.player.movement.tuning = base;
                    if let Some(policies) = &policies
                        && let Some(seat_id) = SeatId::new(seat as u8)
                        && let Some(angles) = self.runtime.input.drift_view(
                            seat_id,
                            self.time,
                            policies[seat],
                            &prediction.player,
                            commands[seat].movement[0],
                        )
                    {
                        prediction.player.view_angles = angles;
                    }
                }
            }
        }
        if self.runtime.collision.is_some() {
            let mut predicted_poses = std::array::from_fn::<_, { SeatId::COUNT }, _>(|seat| {
                self.local_seat(seat)
                    .map(|(_, client, player)| (client.entity, player.body.position))
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
        for (seat, &id) in self.local_clients.iter().enumerate() {
            let Some(id) = id else {
                continue;
            };
            if self.local_clients[..seat].contains(&Some(id)) {
                continue;
            }
            let Some(client) = self.runtime.server.clients.get_mut(id.0 as usize) else {
                continue;
            };
            if client.connection != Some(qa_session::clients::Connection::Local) {
                continue;
            }
            let player = &self.runtime.prediction[seat].player;
            self.runtime
                .catalog
                .hud
                .project_stats(player, &mut client.hud);
            client
                .hud_publication
                .publish_changes(player, &mut self.runtime.server.events, id);
        }
        commands
    }
}

struct ConsoleInput<'a> {
    console: &'a mut Console<Runtime>,
    events: &'a mut qa_core::events::EventRing,
    time: EventTime,
}
impl Target for ConsoleInput<'_> {
    fn key(&mut self, seat: SeatId, control: u16, down: bool, repeat: bool) -> bool {
        match self
            .console
            .edit_key(seat, control, down, repeat, self.time, |text| {
                let _ = self.events.console_formatted(text);
            }) {
            Ok(caught) => caught,
            Err(error) => {
                let _ = self
                    .events
                    .console_formatted(format_args!("Console input: {error:?}\n"));
                true
            }
        }
    }
    fn character(&mut self, seat: SeatId, value: char) {
        if self.console.editor.character(seat, value).is_err() {
            let _ = self.events.console_bytes(b"Console input line is full\n");
        }
    }
    fn command(&mut self, seat: SeatId, time: EventTime, text: &str) {
        let context = qa_console::views::Context {
            seat,
            event_time: Some(time),
            ..self.console.cvars.context()
        };
        if let Err(error) = self.console.append_line(text, context) {
            let _ = self
                .events
                .console_formatted(format_args!("Input command: {error:?}\n"));
        }
    }
}

fn event_name(kind: EventKind<'_>) -> &'static str {
    match kind {
        EventKind::Time => "time",
        EventKind::Key { .. } => "key",
        EventKind::Text { .. } => "text",
        EventKind::Mouse { .. } => "mouse",
        EventKind::MouseButton { .. } => "mouse_button",
        EventKind::MouseWheel { .. } => "mouse_wheel",
        EventKind::ControllerAxis { .. } => "controller_axis",
        EventKind::ControllerButton { .. } => "controller_button",
        EventKind::DeviceRemoved(_) => "device_removed",
        EventKind::Focus(_) => "focus",
        EventKind::MouseCapture(_) => "mouse_capture",
        EventKind::Quit => "quit",
        EventKind::ConsoleLine(_) => "console_line",
        EventKind::Packet { .. } => "packet",
    }
}
