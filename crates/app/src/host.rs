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
    pub world_presentation: qa_render::world::WorldPresentationId,
    pub presentation: qa_render::material::world_load::PresentationDefaults,
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
        _huds: &[Option<crate::renderer::hud::HudView<'_>>; SeatId::COUNT],
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
    fn effect_clients(
        &mut self,
        _clients: &[Option<qa_render::effects::EffectClient>; SeatId::COUNT],
        _time: EventTime,
        _gravity: f32,
    ) {
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
    fn effect(&mut self, event: EffectEvent) -> bool {
        self.renderer.effect(event)
    }
    fn effect_clients(
        &mut self,
        clients: &[Option<qa_render::effects::EffectClient>; SeatId::COUNT],
        time: EventTime,
        gravity: f32,
    ) {
        self.renderer.set_effect_clients(*clients, time, gravity);
    }
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
        huds: &[Option<crate::renderer::hud::HudView<'_>>; SeatId::COUNT],
        console: qa_console::presentation::ConsoleView<'_>,
    ) {
        self.renderer.frame(views, scenes, huds, console);
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
    pub local_worlds: [Option<(
        qa_render::world::WorldId,
        qa_render::world::WorldPresentationId,
    )>; SeatId::COUNT],
    pub local_presentations: [qa_render::material::world_load::PresentationDefaults; SeatId::COUNT],
    maxfps: CvarHandle,
    fov: CvarHandle,
    roll_angle: CvarHandle,
    roll_speed: CvarHandle,
    kick_angles: CvarHandle,
    input_handles: qa_console::input_settings::InputHandles,
    pub(crate) movement_handles: qa_console::movement::MovementHandles,
    notify_time: CvarHandle,
    center_time: CvarHandle,
    pub(crate) team_chats_only: CvarHandle,
    volume: CvarHandle,
    particle_gravity: CvarHandle,
    mouse_enabled: CvarHandle,
    previous: Option<EventTime>,
    loop_client_cursor: usize,
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
                .read_to_vec(file, 16 * 1024 * 1024, &mut runtime.script_reader)
                .map_err(|e| format!("localization: {e}"))?;
            runtime.localization = qa_content::localization::Catalog::load(&bytes)
                .map_err(|e| format!("localization: {e:?}"))?;
        }
        console.register("play", crate::audio::play);
        let volume = console.cvars.find("s_volume").ok_or("missing s_volume")?;
        let particle_gravity = console.cvars.find("g_gravity").ok_or("missing g_gravity")?;
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
        let kick_angles = console
            .cvars
            .find("cl_kickangles")
            .ok_or("missing cl_kickangles")?;
        let input_handles = qa_console::input_settings::InputHandles::load(&console.cvars)?;
        let movement_handles = qa_console::movement::MovementHandles::bind(&console.cvars)?;
        let notify_time = console
            .cvars
            .find("con_notifytime")
            .ok_or("missing con_notifytime")?;
        let center_time = console
            .cvars
            .find("cg_centertime")
            .ok_or("missing cg_centertime")?;
        let team_chats_only = console
            .cvars
            .find("cg_teamChatsOnly")
            .ok_or("missing cg_teamChatsOnly")?;
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
            local_presentations: [Default::default(); SeatId::COUNT],
            maxfps,
            fov,
            roll_angle,
            roll_speed,
            kick_angles,
            input_handles,
            movement_handles,
            notify_time,
            center_time,
            team_chats_only,
            volume,
            particle_gravity,
            mouse_enabled,
            previous: None,
            loop_client_cursor: 0,
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
        self.publish_module_client_state();
        result.simulation_ns = simulation.elapsed().as_nanos() as u64;
        self.publish_module_configs();
        self.publish_module_signon();
        for index in 0..self.runtime.snapshots.len() {
            if self.runtime.snapshots[index].is_none() {
                continue;
            }
            let id = ClientId(index as u32);
            let observation = self.snapshot_observation(id, server_time);
            self.runtime
                .send_snapshot(id, server_time, observation, source, |module, ordinal| {
                    self.modules.as_ref()?.bound_sound(module, ordinal)
                });
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
        self.publish_client_feedback();
        self.publish_client_loops();
        result.client_ns = client.elapsed().as_nanos() as u64;
        self.dispatch_output(source, &mut result);
        result.input_ns = source.elapsed().as_nanos() as u64;
        let views = self.client_views();
        for (seat, view) in views.iter().enumerate() {
            if let (Some(view), Some(client)) = (view, self.local_clients[seat])
                && let Some(snapshot) = self.runtime.snapshots[client.0 as usize].as_mut()
            {
                snapshot.submitted_view(qa_core::math::angle_vectors(view.angles).axes());
            }
        }
        source.render(
            &views,
            &self.client_scene_inputs(),
            &self.client_hud_inputs(),
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
            let (client, row, player) = self.local_seat(seat)?;
            let (world, world_presentation) = self.local_worlds[seat]?;
            Some(ClientView {
                world,
                world_presentation,
                presentation: self.local_presentations[seat],
                origin: player.eye(),
                angles: self.client_angles(client, row, player),
                fov_x: self.console.cvars.value(self.fov),
                time_ms: self.time.milliseconds(),
            })
        })
    }

    fn client_angles(
        &self,
        client: ClientId,
        row: &qa_session::clients::Client,
        player: &qa_core::primitives::PlayerState,
    ) -> qa_core::primitives::Vec3 {
        let Some(snapshot) = self.runtime.snapshots[client.0 as usize].as_ref() else {
            return player.view_angles;
        };
        if snapshot.owns_client_view() {
            return player.view_angles;
        }
        let Some(interpolation) = snapshot.interpolation() else {
            return player.view_angles;
        };
        let angles = self
            .runtime
            .network
            .get(client, qa_core::loopback::Endpoint::Client)
            .map_or(player.view_angles, |connection| {
                snapshot.view_angles(
                    &connection.channel,
                    player,
                    snapshot.view_source().is_none_or(|source| {
                        self.console.cvars.integer_in(self.kick_angles, source) != 0
                    }),
                )
            });
        qa_ui::hud::view_angles(&row.hud, angles, player.health, interpolation.time)
    }

    /// Transition due frames from the connection ring without replacing an
    /// active envelope with a snapshot ahead of the presentation clock.
    pub fn publish_client_feedback(&mut self) {
        for snapshot in self.runtime.snapshots.iter_mut().flatten() {
            let Some(_seat) = snapshot.seat() else {
                continue;
            };
            let Some(client) = self
                .runtime
                .server
                .clients
                .get_mut(snapshot.client.0 as usize)
            else {
                continue;
            };
            if client.connection.is_some()
                && let Some(connection) = self
                    .runtime
                    .network
                    .get(snapshot.client, qa_core::loopback::Endpoint::Client)
            {
                snapshot.publish_feedback(
                    client.entity,
                    &connection.channel,
                    self.runtime
                        .client_resources
                        .get(snapshot.client.0 as usize)
                        .and_then(Option::as_ref)
                        .map(|row| &row.state),
                    self.runtime.server.presentation,
                    &mut client.hud,
                    &mut self.runtime.server.events,
                    self.time,
                );
            }
        }
    }

    pub(crate) fn publish_client_values(
        &mut self,
        id: ClientId,
        rules: qa_core::primitives::RuleSetId,
        module: ModuleId,
    ) {
        use qa_core::{
            events::FrameEvent,
            primitives::{NativeDelivery, NativeTarget},
        };
        let bindings = &self.runtime.catalog.hud.values.native(rules).stats;
        let server = &mut self.runtime.server;
        let client = &mut server.clients[id.0 as usize];
        let delivery = NativeDelivery {
            module,
            source: rules,
            target: NativeTarget::Client(id),
            origin: client.player.body.position,
            reliable: true,
            dupe_key: 0,
            component: 0,
            emission: None,
            check_portals: false,
        };
        client.value_publication.publish_changes(
            bindings,
            &client.player.values,
            |value_id, value| {
                server
                    .events
                    .push_delivered(
                        delivery,
                        FrameEvent::PlayerValue {
                            id: value_id,
                            value,
                        },
                    )
                    .is_ok()
            },
        );
    }

    pub(crate) fn alias_roll(&self) -> qa_gameplay::rules::appearance::AliasRoll {
        qa_gameplay::rules::appearance::AliasRoll {
            angle: self
                .console
                .cvars
                .value_in(self.roll_angle, qa_core::primitives::RuleSetId::QuakeWorld),
            speed: self
                .console
                .cvars
                .value_in(self.roll_speed, qa_core::primitives::RuleSetId::QuakeWorld),
        }
    }

    /// Pose readers do not require a loaded renderer or model catalog.
    pub fn client_entities_inputs(
        &self,
    ) -> [Option<crate::client_scene::ClientEntities<'_>>; SeatId::COUNT] {
        let alias_roll = self.alias_roll();
        std::array::from_fn(|seat| {
            let (client, _, player) = self.local_seat(seat)?;
            let snapshot = self.runtime.snapshots.get(client.0 as usize)?.as_ref()?;
            let channel = &self
                .runtime
                .network
                .get(client, qa_core::loopback::Endpoint::Client)?
                .channel;
            Some(crate::client_scene::ClientEntities {
                client,
                player: Some(player),
                current: channel.current_snapshot(),
                previous: channel.previous_snapshot(),
                projection: snapshot.entity_projection(),
                appearance: snapshot.appearance_policy()?,
                alias_roll,
                interpolation: snapshot.interpolation()?,
            })
        })
    }

    pub fn client_scene_inputs(
        &self,
    ) -> [Option<crate::client_scene::ClientSceneInput<'_>>; SeatId::COUNT] {
        let entities = self.client_entities_inputs();
        std::array::from_fn(|seat| {
            let (_, row, player) = self.local_seat(seat)?;
            let entities = entities[seat]?;
            let client = entities.client;
            let snapshot = self.runtime.snapshots.get(client.0 as usize)?.as_ref()?;
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
                appearance: entities.appearance,
                player_projection: snapshot.player_projection(),
                bindings: &resources.state.models,
                static_entities: resources.state.static_entities(),
                models: self.runtime.models.as_deref()?,
                player,
                weapon_view_angles: snapshot.weapon_view_angles(
                    player.view_angles,
                    self.client_angles(client, row, player),
                ),
                alias_roll: entities.alias_roll,
                interpolation: entities.interpolation,
                gun_animation: snapshot.gun_animation(),
                gun_previous: snapshot
                    .gun_animation()
                    .and_then(|animation| animation.previous)
                    .and_then(|sequence| channel.snapshot(sequence)),
                view_entity: snapshot.view_entity(channel.current_snapshot()),
            })
        })
    }

    pub fn client_hud_inputs(&self) -> [Option<crate::renderer::hud::HudView<'_>>; SeatId::COUNT] {
        std::array::from_fn(|seat| {
            let (client, row, _) = self.local_seat(seat)?;
            let resources = self
                .runtime
                .client_resources
                .get(client.0 as usize)?
                .as_ref()?;
            let snapshot = self.runtime.snapshots.get(client.0 as usize)?.as_ref()?;
            Some(crate::renderer::hud::HudView {
                hud: &row.hud,
                texts: &self.runtime.server.events.texts,
                configs: &resources.state.configs,
                layout: resources.state.layout,
                values: &self.runtime.catalog.hud.values,
                rules: row.client_rules,
                time: self.time,
                style_time: if snapshot.protocol
                    == qa_network::commands::packet::Protocol::QuakeWorld28
                {
                    qa_core::primitives::ThinkTime::Seconds(0.0).at(self.time)
                } else {
                    snapshot.interpolation()?.time
                },
                show_text: self.console.editor.seat().is_none(),
            })
        })
    }

    fn advance_client_presentation(&mut self) {
        for snapshot in self.runtime.snapshots.iter_mut().flatten() {
            if let Some(connection) = self
                .runtime
                .network
                .get(snapshot.client, qa_core::loopback::Endpoint::Client)
            {
                snapshot.advance_presentation(&connection.channel, self.time);
            }
        }
    }

    /// Accepted CLIENT rows publish to Presentation once per connection, not
    /// once per seat/view. No native guest or SERVER entity pose is observed.
    pub fn publish_client_loops(&mut self) {
        let alias_roll = self.alias_roll();
        let runtime = &mut self.runtime;
        let server = &mut runtime.server;
        let mut budget = (server.events.capacity() / 4).max(1);
        let count = runtime.snapshots.len();
        for step in 0..count {
            let index = (self.loop_client_cursor + step) % count;
            let Some(snapshot) = runtime.snapshots[index].as_mut() else {
                continue;
            };
            let Some(client) = server.clients.get(snapshot.client.0 as usize) else {
                continue;
            };
            if client.connection.is_none() || server.entities.resolve(client.entity).is_none() {
                continue;
            }
            if snapshot.owns_client_audio() {
                snapshot.maintain_client_audio(
                    client.entity,
                    server.presentation,
                    &mut server.events,
                    &mut budget,
                );
                if budget == 0 {
                    self.loop_client_cursor = (index + 1) % count;
                    break;
                }
                continue;
            }
            let Some(resources) = runtime.client_resources[snapshot.client.0 as usize].as_ref()
            else {
                continue;
            };
            let Some(connection) = runtime
                .network
                .get(snapshot.client, qa_core::loopback::Endpoint::Client)
            else {
                continue;
            };
            snapshot.publish_loops(
                snapshot
                    .seat()
                    .and_then(|seat| runtime.prediction.get(seat.index()))
                    .map(|prediction| &prediction.player),
                client.entity,
                server.presentation,
                &mut server.events,
                &connection.channel,
                &resources.state,
                alias_roll,
                &mut budget,
            );
            if budget == 0 {
                self.loop_client_cursor = (index + 1) % count;
                break;
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
        let effect_clients = std::array::from_fn(|seat| {
            let (client, row, _) = self.local_seat(seat)?;
            Some(qa_render::effects::EffectClient {
                client,
                lifetime: row.entity,
            })
        });
        source.effect_clients(
            &effect_clients,
            self.time,
            self.console
                .cvars
                .value_in(self.particle_gravity, qa_core::primitives::RuleSetId::Quake),
        );
        // Channel replacement/protection observes this frame's listener even
        // before newly received camera feedback is consumed.
        source.listeners(
            &self.client_listeners(),
            self.console.cvars.value(self.volume),
        );
        result.output = crate::output::dispatch(
            &mut self.runtime,
            source,
            &mut self.console.scrollback,
            &self.local_clients,
            self.time,
            f64::from(self.console.cvars.value(self.notify_time)),
            f64::from(self.console.cvars.value(self.center_time)),
        );
        result.output_drains += 1;
        // Feedback has reached the HUD cursor before camera/listener sampling.
        if result.output.view_feedback != 0 {
            source.listeners(
                &self.client_listeners(),
                self.console.cvars.value(self.volume),
            );
        }
        source.mix_audio(&self.client_entities_inputs());
    }

    fn client_listeners(&self) -> [Option<qa_audio::Listener>; SeatId::COUNT] {
        std::array::from_fn(|seat| {
            let (client, row, player) = self.local_seat(seat)?;
            if self.runtime.snapshots[client.0 as usize]
                .as_ref()
                .is_some_and(crate::snapshots::SnapshotBinding::owns_client_audio)
            {
                return None;
            }
            Some(qa_audio::Listener {
                negate_dot: false,
                client,
                entity: Some(
                    self.runtime.snapshots[client.0 as usize]
                        .as_ref()
                        .and_then(|snapshot| {
                            snapshot.view_entity(
                                self.runtime
                                    .network
                                    .get(client, qa_core::loopback::Endpoint::Client)
                                    .and_then(|connection| connection.channel.current_snapshot()),
                            )
                        })
                        .map_or(
                            qa_core::primitives::SoundEmitter::Entity(row.entity),
                            |slot| qa_core::primitives::SoundEmitter::Native {
                                namespace: qa_core::primitives::ResourceNamespace::Client(client),
                                slot: slot as i32,
                            },
                        ),
                ),
                origin: player.eye(),
                right: qa_core::math::angle_vectors(self.client_angles(client, row, player)).right,
            })
        })
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
                    if let Some(binding) = self.runtime.snapshots[id.0 as usize].as_ref()
                        && let Some(seat) = binding.seat()
                        && server.clients[id.0 as usize].connection
                            == Some(qa_session::clients::Connection::Local)
                        && server.clients[id.0 as usize].output == Some(binding.output)
                    {
                        match incoming {
                            Incoming::PlayerValue { index, value } => {
                                let protocol = self.runtime.snapshots[id.0 as usize]
                                    .as_ref()
                                    .map(|binding| binding.protocol);
                                if let Some(protocol) = protocol {
                                    self.runtime.catalog.hud.import_native_stat(
                                        protocol.rule_set(),
                                        id,
                                        index as usize,
                                        value as u32,
                                        &mut self.runtime.prediction[seat.index()].player,
                                        time,
                                    );
                                }
                            }
                            Incoming::SetAngles(angles) => {
                                self.runtime.prediction[seat.index()].player.view_angles = angles;
                                self.runtime.input.set_view_angles(seat, angles);
                            }
                            Incoming::Print(print) => {
                                let _ = self.runtime.server.events.print_level_bytes(
                                    Some(qa_core::events::OutputTarget::Local(id)),
                                    Some(id),
                                    print.kind,
                                    print.level.unwrap_or(print.kind.default_level()),
                                    print.text,
                                );
                            }
                            Incoming::Cue { cue, current } => self.received_cue(id, cue, current),
                            Incoming::Particle(particle) => {
                                if let Some(mut event) = self.runtime.snapshots[id.0 as usize]
                                    .as_ref()
                                    .and_then(|binding| binding.particle_event(particle))
                                {
                                    event.client = Some(id);
                                    event.client_lifetime =
                                        Some(server.clients[id.0 as usize].entity);
                                    event.time = time;
                                    let _ = self.runtime.server.events.push_to(
                                        qa_core::events::OutputTarget::Presentation,
                                        qa_core::events::FrameEvent::Effect(event),
                                    );
                                }
                            }
                            Incoming::StaticEntity(entity) => {
                                if let Some(resources) =
                                    self.runtime.client_resources[id.0 as usize].as_mut()
                                {
                                    resources.state.receive_static_entity(entity);
                                }
                            }
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
                            // Q3 cgame selects retained commands through the
                            // shared CLIENT service after packet intake.
                            Incoming::ReliableCommand { .. } => {}
                            incoming => {
                                if let Some(binding) =
                                    self.runtime.snapshots[id.0 as usize].as_mut()
                                {
                                    binding.apply(
                                        incoming,
                                        &mut self.runtime.prediction[seat.index()].player,
                                        &self.runtime.native_numbers[id.0 as usize],
                                        &self.runtime.catalog.hud,
                                        &self.runtime.catalog.registry,
                                        time,
                                    );
                                }
                            }
                        }
                    }
                }
                qa_core::loopback::Endpoint::Server => match incoming {
                    Incoming::MoveBegin => self.begin_command_batch(id),
                    Incoming::MoveEnd => self.end_command_batch(id),
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
                            && let Err(error) = self.userinfo_module_client(id, text)
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
                            let duplicates = &mut self.runtime.output_duplicates;
                            duplicates.acknowledged(output, receipt);
                            self.runtime.server.events.acknowledge_with(
                                output,
                                receipt,
                                |record| {
                                    if let qa_core::events::FrameEvent::ConfigString {
                                        destination_index,
                                        ..
                                    } = record.event
                                    {
                                        publication.acknowledged(
                                            destination_index as usize,
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
            if self.runtime.server.clients[id.0 as usize]
                .connection
                .is_none()
            {
                disconnected = Some(id);
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
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
                        target.command(seat, event.time, None, text);
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
        let policies = {
            std::array::from_fn(|seat| {
                let mut policy = self.input_handles.policy(&self.console.cvars, rules[seat]);
                if self.local_clients[seat].is_some() {
                    policy.delta_pitch =
                        self.runtime.prediction[seat].player.movement.delta_angles.0[0].degrees();
                }
                policy
            })
        };
        let mut commands = self
            .runtime
            .input
            .build_frame_with_policy(self.time, &policies);
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
                    let result = prediction.advance(commands[seat], &mut trace);
                    prediction.player.movement.tuning = base;
                    if let Err(error) = result {
                        let _ = self.runtime.server.events.console_formatted(format_args!(
                            "Prediction client {}: {error:?}\n",
                            id.0
                        ));
                        continue;
                    }
                    if let Some(seat_id) = SeatId::new(seat as u8)
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
                    .filter(|(_, _, player)| player.prediction.position)
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
    fn command(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<qa_core::sys_events::KeySource>,
        text: &str,
    ) {
        let context = qa_console::views::Context {
            seat,
            event_time: Some(time),
            key_source: source,
            ..self.console.cvars.context()
        };
        if let Err(error) = self.console.append_line(text, context) {
            self.events.reject_publication();
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
