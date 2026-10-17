pub mod audio;
pub mod catalog;
pub mod client_collision;
pub mod client_policy;
pub mod client_resources;
pub mod client_scene;
pub mod host;
pub mod map;
pub mod model_assets;
pub mod modules;
pub mod options;
pub mod output;
pub mod output_namespace;
pub mod profile;
pub mod render_settings;
pub mod renderer;
mod server_downloads;
pub mod snapshots;
pub mod startup;
mod transport;
pub mod window_settings;

use qa_console::commands::{Host, ScriptError};
use qa_content::vfs::Vfs;
use qa_core::loopback::Loopback;
use qa_core::primitives::{
    ClientId, GeometryId, ModuleId, NATIVE_PAYLOAD_BYTES, NATIVE_WIRE_BYTES, PrintKind, Vec3,
};
use qa_network::ingress::Connections;
use qa_session::clients::{Connection, Server};

pub struct Runtime {
    /// One PCM and registration owner retained through play.
    pub sound_assets: Option<Box<audio::SoundAssets>>,
    pub catalog: catalog::GameplayCatalog,
    pub vfs: Vfs,
    pub localization: qa_content::localization::Catalog,
    /// Exact map entity sources in load order, retained for native module import.
    pub entity_sources: Vec<map::NativeEntityText>,
    pub quit: bool,
    pub network: Connections,
    pub server: Server,
    /// One preallocated index over the server's generation-checked entities.
    pub targets: qa_world::targets::TargetIndex,
    /// One owner of admitted geometry for every active world and inline model.
    pub geometry: qa_world::collision::CollisionStore,
    /// Loaded geometry is independent of every player's movement rules.
    pub collision: Option<WorldCollision>,
    pub prediction: [qa_session::prediction::Prediction; qa_core::sys_events::SeatId::COUNT],
    pub snapshots: Box<[Option<snapshots::SnapshotBinding>]>,
    pub client_resources: Box<[Option<client_resources::ClientResources>]>,
    pub(crate) server_downloads: Box<[server_downloads::ServerDownload]>,
    pub client_collision: Box<[Option<client_collision::ClientCollision>]>,
    pub native_numbers: Box<[output_namespace::NativeNumbers]>,
    pub(crate) config_publications: Box<[client_resources::ConfigPublication]>,
    pub output_duplicates: output::duplicates::Coordinator,
    /// One asset and catalog owner retains registration caches across play.
    pub model_assets: Option<model_assets::ModelAssets>,
    pub renderer_configuration:
        Option<std::sync::Arc<qa_core::presentation::RendererConfiguration>>,
    pub scene: Option<renderer::SceneComposer>,
    pub loopback: Loopback,
    pub input: qa_input::Input,
    pub input_time: qa_core::sys_events::EventTime,
    pub(crate) output_bytes: Box<[u8]>,
    pub(crate) output_native_bytes: Box<[u8]>,
    pub(crate) userinfo_cursor: usize,
    pub(crate) userinfo_queued_at: qa_core::sys_events::EventTime,
    script_reader: qa_formats::archive::ArchiveReader,
}

pub struct WorldCollision {
    pub geometry: GeometryId,
    pub index: u32,
    pub scratch: qa_world::collision::TraceScratch,
    pub visibility: Option<std::sync::Arc<qa_world::visibility::VisibilityWorld>>,
    pub leaves: qa_world::leaves::LeafScratch,
    pub visibility_row: Box<[u8]>,
    pub portals: Option<qa_world::portals::AreaPortals>,
}

impl WorldCollision {
    pub fn new(
        store: &qa_world::collision::CollisionStore,
        geometry: GeometryId,
        index: u32,
    ) -> Self {
        let scratch = store.scratch();
        Self {
            geometry,
            index,
            scratch,
            visibility: None,
            leaves: qa_world::leaves::LeafScratch::new(1),
            visibility_row: Box::default(),
            portals: None,
        }
    }
    pub fn with_visibility(
        mut self,
        world: std::sync::Arc<qa_world::visibility::VisibilityWorld>,
    ) -> Self {
        self.leaves = qa_world::leaves::LeafScratch::new(world.node_count() + 1);
        self.visibility_row = vec![0; world.pvs().row_bytes()].into_boxed_slice();
        self.visibility = Some(world);
        self
    }
    pub fn with_portals(mut self, portals: Option<qa_world::portals::AreaPortals>) -> Self {
        self.portals = portals;
        self
    }
}

pub(crate) struct TraceScratchStage<'a> {
    replacements: Vec<(
        &'a mut qa_world::collision::TraceScratch,
        qa_world::collision::TraceScratch,
    )>,
}
impl<'a> TraceScratchStage<'a> {
    pub(crate) fn prepare(
        store: &qa_world::collision::CollisionStore,
        destinations: impl IntoIterator<Item = &'a mut qa_world::collision::TraceScratch>,
    ) -> Self {
        Self {
            replacements: destinations
                .into_iter()
                .map(|destination| (destination, store.scratch()))
                .collect(),
        }
    }

    pub(crate) fn commit(self) {
        for (destination, scratch) in self.replacements {
            *destination = scratch;
        }
    }
}

pub(crate) fn runtime_trace_scratches<'a>(
    collision: &'a mut Option<WorldCollision>,
    clients: &'a mut [Option<client_collision::ClientCollision>],
) -> impl Iterator<Item = &'a mut qa_world::collision::TraceScratch> {
    collision.iter_mut().map(|world| &mut world.scratch).chain(
        clients
            .iter_mut()
            .flatten()
            .map(client_collision::ClientCollision::trace_scratch),
    )
}

impl Runtime {
    pub fn sound_bank(&self) -> Option<&qa_audio::Bank> {
        self.sound_assets.as_deref().map(audio::SoundAssets::bank)
    }
    pub(crate) fn prepare_traces(
        &mut self,
        module_rules: impl Iterator<Item = qa_core::primitives::RuleSetId>,
    ) -> Result<(), String> {
        if let Some(rules) = self
            .server
            .clients
            .iter()
            .filter(|client| client.connection.is_some())
            .map(|client| client.player.trace_rules)
            .chain(module_rules)
            .map(|rules| qa_world::collision::trace_policy(rules).0)
            .find(|rules| rules.hull_shape == qa_world::collision::HullShape::ExactBox)
        {
            self.geometry
                .prepare_traces(rules, Default::default())
                .map_err(|error| format!("caller hull capability: {error:?}"))?;
        }
        TraceScratchStage::prepare(
            &self.geometry,
            runtime_trace_scratches(&mut self.collision, &mut self.client_collision),
        )
        .commit();
        Ok(())
    }

    pub(crate) fn synchronize_client_collision(
        &mut self,
        id: ClientId,
        frame: qa_network::snapshots::ReceivedFrame<'_>,
    ) {
        let slot = id.0 as usize;
        if let (Some(view), Some(binding), Some(resources)) = (
            self.client_collision.get_mut(slot).and_then(Option::as_mut),
            self.snapshots.get(slot).and_then(Option::as_ref),
            self.client_resources.get(slot).and_then(Option::as_ref),
        ) {
            let _ = view.synchronize(
                frame,
                binding.entity_projection(),
                &resources.state.models,
                &self.geometry,
                binding.player_entity(Some(frame)),
                resources.state.max_clients(),
            );
        }
    }
    pub fn engine_services<'a>(
        &'a mut self,
        console: &'a mut qa_console::commands::Console<Self>,
        storage: &'a mut qa_compat::services::ServiceStorage,
        scratch: &'a mut qa_world::collision::TraceScratch,
    ) -> qa_compat::services::EngineServices<'a> {
        self.engine_service_parts(console, storage, scratch, None).0
    }
    pub(crate) fn client_connection_parts(
        &mut self,
        client: ClientId,
        lifetime: qa_core::primitives::EntityId,
    ) -> Option<(
        client_resources::ClientConnectionView<'_>,
        &mut Server,
        Option<&qa_audio::Bank>,
    )> {
        let view = client_resources::connection_view(
            client,
            lifetime,
            &mut self.network,
            &mut self.client_resources,
            self.model_assets
                .as_ref()
                .filter(|owner| !owner.initial_registration())
                .map(model_assets::ModelAssets::catalog),
        )?;
        Some((
            view,
            &mut self.server,
            self.sound_assets.as_deref().map(audio::SoundAssets::bank),
        ))
    }
    pub(crate) fn engine_service_parts<'a>(
        &'a mut self,
        console: &'a mut qa_console::commands::Console<Self>,
        storage: &'a mut qa_compat::services::ServiceStorage,
        scratch: &'a mut qa_world::collision::TraceScratch,
        client: Option<(ClientId, qa_core::primitives::EntityId)>,
    ) -> (
        qa_compat::services::EngineServices<'a>,
        Option<client_resources::ClientConnectionView<'a>>,
        Option<&'a mut renderer::SceneComposer>,
        Option<model_assets::ModelView<'a>>,
    ) {
        let (model_loader, model_view) = match self.model_assets.as_mut() {
            Some(owner) if owner.initial_registration() => (
                Some(owner as &mut dyn qa_compat::services::ModelLoader),
                None,
            ),
            Some(owner) => (None, Some(owner.view())),
            None => (None, None),
        };
        let client_connection = client.and_then(|(client, lifetime)| {
            client_resources::connection_view(
                client,
                lifetime,
                &mut self.network,
                &mut self.client_resources,
                model_view.map(|view| view.catalog),
            )
        });
        let client_audio = client.and_then(|(client, _)| {
            self.snapshots
                .get_mut(client.0 as usize)?
                .as_mut()
                .map(|binding| binding as &mut dyn qa_compat::services::ClientAudio)
        });
        let (cvars, commands, editor) = console.module_parts();
        let world = self.collision.as_mut();
        let (identity, visibility, portals) = match world {
            Some(world) => (
                Some((world.geometry, world.index)),
                world
                    .visibility
                    .as_deref()
                    .map(|visibility| (visibility, &mut world.leaves, &mut *world.visibility_row)),
                world.portals.as_mut(),
            ),
            None => (None, None, None),
        };
        let model_lookup =
            model_view.map(|view| view.catalog as &dyn qa_compat::services::ModelLookup);
        let services = qa_compat::services::EngineServices {
            server: &mut self.server,
            callbacks: None,
            client_disconnect: None,
            input: &mut self.input,
            cvars,
            commands,
            editor: Some(editor),
            client_connection: None,
            client_collision: None,
            renderer: self.renderer_configuration.as_deref(),
            scene: None,
            client_audio,
            vfs: &self.vfs,
            localization: &self.localization,
            registry: &self.catalog.registry,
            names: &mut self.catalog.names,
            model_loader,
            model_lookup,
            sound_loader: self
                .sound_assets
                .as_deref_mut()
                .map(|loader| loader as &mut dyn qa_compat::services::SoundRegistrar),
            sound_bank: None,
            storage,
            geometry: &self.geometry,
            world: identity,
            native_geometry: identity.map(|(geometry, _)| geometry),
            foreign_trace: None,
            visibility,
            portals,
            scratch,
        };
        (services, client_connection, self.scene.as_mut(), model_view)
    }

    /// Common client capacity is chosen at load; native limits belong to each connection.
    pub fn load<'a>(
        max_clients: usize,
        extra_names: impl IntoIterator<Item = &'a [u8]>,
    ) -> Result<Self, String> {
        let catalog = catalog::GameplayCatalog::load(extra_names)?;
        let item_slots = catalog.registry.items.len() + 1;
        let weapon_slots = catalog.registry.weapons.len() + 1;
        // Inventory, acquisition times and powerup timers all use common ItemId.
        // Zero is unused; instant and non-powerup items retain zero timer rows.
        // Native powerup ordinals are converted at module/protocol boundaries.
        let server = Server::load(
            max_clients,
            8192,
            item_slots,
            item_slots,
            weapon_slots,
            catalog.hud.values.capacity(),
        )
        .map_err(|e| format!("server: {e:?}"))?;
        let targets = qa_world::targets::TargetIndex::new(&server.entities);
        let output_duplicates =
            output::duplicates::Coordinator::load(server.events.capacity(), max_clients);
        // Boot capacity covers the largest planned native local message.
        // Native connection negotiation supplies narrower limits later.
        let loopback = Loopback::load(std::iter::repeat_n(
            qa_core::loopback::LoopbackLimits {
                maximum_message: 64_000,
                payload_bytes: 128_000,
                messages: 16,
            },
            server.clients.len(),
        ))
        .map_err(|e| format!("loopback: {e:?}"))?;
        let value_slots = catalog.hud.values.capacity();
        Ok(Self {
            sound_assets: None,
            model_assets: None,
            renderer_configuration: None,
            scene: None,
            client_resources: std::iter::repeat_with(|| None).take(max_clients).collect(),
            server_downloads: std::iter::repeat_with(server_downloads::ServerDownload::default)
                .take(max_clients)
                .collect(),
            client_collision: std::iter::repeat_with(|| None).take(max_clients).collect(),
            native_numbers: std::iter::repeat_with(|| {
                output_namespace::NativeNumbers::load(
                    qa_network::commands::packet::Protocol::Quake2Repro1038,
                )
            })
            .take(max_clients)
            .collect(),
            config_publications: std::iter::repeat_with(client_resources::ConfigPublication::load)
                .take(max_clients)
                .collect(),
            output_duplicates,
            catalog,
            vfs: Vfs::default(),
            localization: qa_content::localization::Catalog::default(),
            entity_sources: Vec::new(),
            quit: false,
            network: Connections::load(max_clients),
            server,
            targets,
            geometry: qa_world::collision::CollisionStore::new(),
            collision: None,
            prediction: std::array::from_fn(|_| {
                qa_session::prediction::Prediction::new(
                    qa_core::primitives::PlayerState::with_capacity(
                        item_slots,
                        item_slots,
                        value_slots,
                    ),
                )
            }),
            snapshots: std::iter::repeat_with(|| None).take(max_clients).collect(),
            loopback,
            input: qa_input::Input::load(),
            input_time: qa_core::sys_events::EventTime::default(),
            userinfo_cursor: 0,
            userinfo_queued_at: Default::default(),
            output_bytes: vec![0; NATIVE_PAYLOAD_BYTES].into_boxed_slice(),
            output_native_bytes: vec![0; NATIVE_WIRE_BYTES].into_boxed_slice(),
            script_reader: qa_formats::archive::ArchiveReader::default(),
        })
    }

    fn reserve_client_seat(
        &mut self,
        seat: qa_core::sys_events::SeatId,
        policy: client_policy::ClientPolicy,
        authority: qa_core::loopback::Endpoint,
    ) -> Result<ClientId, String> {
        if self.snapshots.iter().flatten().any(|binding| {
            binding.seat() == Some(seat)
                && self
                    .server
                    .clients
                    .get(binding.client.0 as usize)
                    .is_some_and(|client| {
                        client.connection.is_some()
                            && client.output == Some(binding.output)
                            && self.server.entities.resolve(client.entity).is_some()
                    })
        }) {
            return Err("local seat already connected".into());
        }
        let id = match authority {
            qa_core::loopback::Endpoint::Server => {
                self.server
                    .connect(Connection::Local, ModuleId::default(), None)
            }
            qa_core::loopback::Endpoint::Client => self.server.connect_client(),
        }
        .ok_or("no local client slot")?;
        self.output_duplicates.reset_client(id);
        self.network.unbind(id);
        self.input.reset_usercmd_value(seat);
        self.prediction[seat.index()].clear_history();
        self.loopback.clear_client(id);
        let client = &mut self.server.clients[id.0 as usize];
        client.client_rules = policy.client;
        client.input_rules = policy.input;
        client.link_order = qa_gameplay::rules::link_order(policy.client);
        for player in [
            &mut client.player,
            &mut self.prediction[seat.index()].player,
        ] {
            player.reset();
            player.movement_rules = policy.movement;
            player.trace_rules = policy.trace;
            qa_movement::set_bounds(player);
            player.health = 100;
        }
        self.input.set_view_angles(seat, Vec3::default());
        Ok(id)
    }

    /// A local connection uses the existing SERVER command consumer and CLIENT
    /// prediction state. Map anchors never choose the player's movement rules.
    pub fn connect_local(
        &mut self,
        seat: qa_core::sys_events::SeatId,
        spawn: map::SpawnAnchor,
        policy: client_policy::ClientPolicy,
        protocol: qa_network::commands::packet::Protocol,
        native_client: u32,
    ) -> Result<ClientId, String> {
        let id = self.reserve_client_seat(seat, policy, qa_core::loopback::Endpoint::Server)?;
        let result = (|| {
            let client = &mut self.server.clients[id.0 as usize];
            for player in [
                &mut client.player,
                &mut self.prediction[seat.index()].player,
            ] {
                player.body.position = spawn.position;
                player.view_angles = spawn.angles;
            }
            self.server
                .entities
                .columns
                .set_body(client.entity.slot as usize, client.player.body);
            self.server
                .entities
                .columns
                .set_angles(client.entity.slot as usize, spawn.angles);
            self.server.area.link(
                &self.server.entities,
                client.entity,
                qa_world::area::LinkFlags::SOLID,
                client.link_order,
                qa_world::area::LinkIntent::Explicit,
            );
            let maximum_message = if protocol == qa_network::commands::packet::Protocol::Quake3_68 {
                qa_network::commands::packet::Q3_MESSAGE_BYTES
            } else {
                8192
            };
            for endpoint in [
                qa_core::loopback::Endpoint::Client,
                qa_core::loopback::Endpoint::Server,
            ] {
                let connection = qa_network::ingress::Connection {
                    phase: qa_network::ingress::ConnectionPhase::Connected,
                    route: qa_network::ingress::Route {
                        socket: endpoint.socket(),
                        peer: qa_core::sys_events::Peer::Loopback(id),
                    },
                    channel: qa_network::channel::Channel::load(
                        protocol.channel(),
                        endpoint,
                        maximum_message,
                        16,
                    )
                    .map_err(|e| e.to_string())?,
                    output: None,
                    pending_output: None,
                    commands: Some(qa_network::commands::connection::Commands::load(
                        protocol,
                        maximum_message,
                    )),
                };
                self.bind_connection(
                    id,
                    connection,
                    snapshots::NativePlayerBinding::Supplied(native_client),
                    Some(seat),
                )?;
            }
            self.input.set_view_angles(seat, spawn.angles);
            Ok(id)
        })();
        if result.is_err() {
            self.disconnect_client(id);
        }
        result
    }

    /// Reserve one native socket CLIENT through negotiation and signon. The
    /// supplied phase owns transport progress; no SERVER actor is created.
    pub fn connect_client(
        &mut self,
        seat: qa_core::sys_events::SeatId,
        policy: client_policy::ClientPolicy,
        native_player: snapshots::NativePlayerBinding,
        connection: qa_network::ingress::Connection,
        lease: Option<qa_platform::UdpLease>,
    ) -> Result<ClientId, String> {
        if connection.channel.endpoint() != qa_core::loopback::Endpoint::Client
            || !matches!(connection.route.peer, qa_core::sys_events::Peer::Socket(_))
        {
            return Err("CLIENT requires a socket channel".into());
        }
        if lease
            .as_ref()
            .is_some_and(|lease| lease.socket() != connection.route.socket)
        {
            return Err("CLIENT socket lease does not match its route".into());
        }
        let id = self.reserve_client_seat(seat, policy, qa_core::loopback::Endpoint::Client)?;
        if let Err(error) = self.bind_connection(id, connection, native_player, Some(seat)) {
            self.disconnect_client(id);
            return Err(error);
        }
        let Some(resources) = self.client_resources[id.0 as usize].as_mut() else {
            self.disconnect_client(id);
            return Err("CLIENT requires native resources".into());
        };
        resources.udp_lease = lease;
        Ok(id)
    }

    pub(crate) fn ensure_client_presentation_reset(&mut self, id: ClientId) -> bool {
        let slot = id.0 as usize;
        let Some(client) = self.server.clients.get(slot).filter(|client| {
            client.connection == Some(qa_session::clients::Connection::Local)
                && self.server.entities.resolve(client.entity).is_some()
        }) else {
            return false;
        };
        let Some(resources) = self.client_resources.get_mut(slot).and_then(Option::as_mut) else {
            return false;
        };
        resources.state.publish_presentation_reset(
            id,
            client.entity,
            &mut self.server.events,
            &mut self.server.presentation,
        )
    }

    pub(crate) fn publish_presentation_resets(&mut self) {
        for slot in 0..self.client_resources.len() {
            let _ = self.ensure_client_presentation_reset(ClientId(slot as u32));
        }
    }

    fn retire_client_audio(&mut self, id: ClientId) {
        let slot = id.0 as usize;
        if self.client_resources.get(slot).is_some_and(Option::is_some)
            && let Some(client) = self.server.clients.get_mut(slot)
        {
            client
                .audio_retirement
                .get_or_insert(qa_session::clients::AudioRetirement {
                    lifetime: client.entity,
                    publication: None,
                });
        }
    }

    /// Stop CLIENT presentation while its channel sends the native final ACK.
    pub(crate) fn close_client_presentation(&mut self, id: ClientId) -> bool {
        let slot = id.0 as usize;
        if self
            .server
            .clients
            .get(slot)
            .is_none_or(|client| client.connection.is_none())
            || self
                .snapshots
                .get(slot)
                .and_then(Option::as_ref)
                .is_none_or(|binding| binding.presentation_closed())
        {
            return false;
        }
        self.retire_client_audio(id);
        if let Some(binding) = self.snapshots[slot].as_mut() {
            binding.close_presentation();
        }
        if let Some(collision) = self.client_collision[slot].as_mut() {
            collision.invalidate();
        }
        if let Some(connection) = self
            .network
            .get_mut(id, qa_core::loopback::Endpoint::Client)
            && let Some(commands) = connection.commands.as_mut()
        {
            commands.reset_history(true);
        }
        true
    }

    /// Tear down one connection; audio retains only its canonical CLIENT lifetime.
    pub fn disconnect_client(&mut self, id: ClientId) -> bool {
        let slot = id.0 as usize;
        if self
            .server
            .clients
            .get(slot)
            .is_none_or(|client| client.connection.is_none())
        {
            return false;
        }
        if self
            .snapshots
            .get(slot)
            .and_then(Option::as_ref)
            .is_none_or(|binding| !binding.presentation_closed())
        {
            self.retire_client_audio(id);
        }
        self.retain_peer_numbers(self.server.clients[slot].entity);
        self.output_duplicates.reset_client(id);
        self.server_downloads[slot].reset();
        self.server.disconnect(id);
        self.network.unbind(id);
        self.loopback.clear_client(id);
        self.snapshots[slot] = None;
        self.client_resources[slot] = None;
        self.client_collision[slot] = None;
        self.config_publications[slot].reset();
        self.native_numbers[slot].reset();
        self.publish_userinfos();
        true
    }

    /// Retry canceled Presentation publications without copying sound events.
    /// Completion is recorded only by the Presentation consumer after delivery.
    pub fn publish_audio_retirements(&mut self) {
        let server = &mut self.server;
        if !server.events.resume_if_needed(&mut server.presentation) {
            return;
        }
        let retained = server
            .clients
            .iter()
            .filter(|client| {
                client.audio_retirement.is_some_and(|retirement| {
                    retirement.publication.is_some_and(|(owner, sequence)| {
                        server.events.retained_for(owner, sequence)
                    })
                })
            })
            .count();
        // Do not overflow Presentation with its own queued retirement controls.
        // Ordinary stalled owners are retired by EventRing's existing policy.
        let mut available = server.events.capacity().saturating_sub(retained);
        for (slot, client) in server.clients.iter_mut().enumerate() {
            let Some(retirement) = &mut client.audio_retirement else {
                continue;
            };
            if retirement
                .publication
                .is_some_and(|(owner, sequence)| server.events.retained_for(owner, sequence))
            {
                continue;
            }
            retirement.publication = None;
            if available == 0 {
                break;
            }
            let event = qa_core::primitives::SoundEvent::retire_client(
                ClientId(slot as u32),
                retirement.lifetime,
            );
            // A full ring can cancel Presentation itself. Resume that epoch and
            // retry the same retained lifetime once, rather than mistaking a
            // canceled admission for delivery or waiting forever on an ACK.
            for _ in 0..2 {
                if !server.events.resume_if_needed(&mut server.presentation) {
                    break;
                }
                let consumer = server.presentation;
                let Ok(sequence) = server.events.push_to(
                    qa_core::events::OutputTarget::Presentation,
                    qa_core::events::FrameEvent::Sound(event),
                ) else {
                    break;
                };
                if server.events.retained_for(consumer, sequence) {
                    retirement.publication = Some((consumer, sequence));
                    available -= 1;
                    break;
                }
            }
        }
    }

    fn prepare_connection_binding(
        &self,
        id: ClientId,
        protocol: qa_network::commands::packet::Protocol,
        native_player: snapshots::NativePlayerBinding,
        seat: Option<qa_core::sys_events::SeatId>,
    ) -> Result<
        (
            snapshots::SnapshotBinding,
            Option<client_resources::ClientResources>,
            Option<client_collision::ClientCollision>,
        ),
        String,
    > {
        let client = self
            .server
            .clients
            .get(id.0 as usize)
            .ok_or("client slot")?;
        let mut binding = snapshots::SnapshotBinding::load(
            id,
            seat,
            client.output.ok_or("client output consumer")?,
            protocol,
            client.client_rules,
            native_player,
            &self.catalog.hud.values,
        );
        binding.bind_particles(&self.catalog.registry.effects);
        if let Some(visibility) = self
            .collision
            .as_ref()
            .and_then(|world| world.visibility.as_deref())
        {
            binding.load_visibility(visibility);
        }
        let mut resources = if seat.is_some() {
            client_resources::ClientResources::load(id, protocol, client.client_rules)?
        } else {
            None
        };
        if let (Some(resources), Some(assets)) = (resources.as_mut(), self.sound_assets.as_deref())
        {
            resources.state.bind_sounds(assets.bank());
        }
        let collision = seat
            .map(|_| client_collision::ClientCollision::load(id, protocol, &self.geometry))
            .transpose()
            .map_err(|error| format!("CLIENT collision: {error:?}"))?;
        Ok((binding, resources, collision))
    }

    /// Both local endpoints share one connection binding. An accepted socket
    /// CLIENT owns a seat; a remote client of this SERVER owns no local seat.
    pub fn bind_connection(
        &mut self,
        id: ClientId,
        mut connection: qa_network::ingress::Connection,
        native_player: snapshots::NativePlayerBinding,
        seat: Option<qa_core::sys_events::SeatId>,
    ) -> Result<(), String> {
        use qa_core::loopback::Endpoint;
        let slot = id.0 as usize;
        let endpoint = connection.channel.endpoint();
        let connecting = matches!(
            connection.phase,
            qa_network::ingress::ConnectionPhase::Connecting(_)
        );
        if self.network.get(id, endpoint).is_some() {
            if self.client_resources.get(slot).is_some_and(Option::is_some) {
                self.disconnect_client(id);
                return Err(
                    "CLIENT resource replacement requires reconnect after audio retirement".into(),
                );
            }
            return Err("connection endpoint already bound".into());
        }
        let client = self.server.clients.get(slot).ok_or("client slot")?;
        if endpoint == Endpoint::Server && !client.server_authoritative() {
            return Err("CLIENT-only reservation cannot bind a SERVER endpoint".into());
        }
        if !matches!(
            (endpoint, client.connection, seat),
            (Endpoint::Server, Some(Connection::Local), Some(_))
                | (Endpoint::Server, Some(Connection::Remote), None)
                | (Endpoint::Client, Some(Connection::Local), Some(_))
        ) {
            return Err("CLIENT seat must belong to a local connection".into());
        }
        let protocol = connection
            .commands
            .as_ref()
            .ok_or("connection needs native command policy")?
            .protocol;
        let limits = qa_network::snapshots::snapshot_limits(protocol);
        match native_player.number() {
            Some(number) if number < limits.client_capacity => {}
            None if endpoint == Endpoint::Client
                && matches!(
                    protocol,
                    qa_network::commands::packet::Protocol::NetQuake15
                        | qa_network::commands::packet::Protocol::QuakeWorld28
                )
                && matches!(connection.route.peer, qa_core::sys_events::Peer::Socket(_)) => {}
            _ => {
                return Err(
                    "native client outside protocol capacity or awaiting SERVER identity".into(),
                );
            }
        }
        let output = client.output.ok_or("client output consumer")?;
        let self_entity = client.entity;
        let prepared = if let Some(binding) = self.snapshots[slot].as_ref() {
            let paired = self
                .network
                .get(id, Endpoint::from_index(endpoint.index() ^ 1));
            if binding.protocol != protocol
                || binding.native_client() != native_player.number()
                || binding.seat() != seat
                || binding.output != output
                || !matches!(connection.route.peer, qa_core::sys_events::Peer::Loopback(peer) if peer == id)
                || !paired.is_some_and(|paired| matches!(paired.route.peer, qa_core::sys_events::Peer::Loopback(peer) if peer == id))
            {
                return Err("connection binding does not match its paired endpoint".into());
            }
            None
        } else {
            Some(self.prepare_connection_binding(id, protocol, native_player, seat)?)
        };
        connection.output = Some(output);
        self.network
            .bind(id, endpoint, connection)
            .map_err(|error| format!("native channel {error:?}"))?;
        if let Some((binding, mut resources, collision)) = prepared {
            if connecting && let Some(resources) = resources.as_mut() {
                resources.begin_startup(protocol);
            }
            self.snapshots[slot] = Some(binding);
            self.client_resources[slot] = resources;
            self.client_collision[slot] = collision;
            self.config_publications[slot].reset();
        }
        if let (Endpoint::Server, Some(native_client)) = (endpoint, native_player.number()) {
            self.native_numbers[slot].configure(protocol);
            self.native_numbers[slot].reset();
            self.native_numbers[slot].set_native_clients(
                (self.server.clients.len() as u32)
                    .max(native_client + 1)
                    .min(limits.client_capacity),
                limits.first_entity,
            );
            self.native_numbers[slot].set_self(self_entity, native_client, protocol);
            self.server.invalidate_userinfo_publications();
        }
        Ok(())
    }

    /// Commands are submitted only after native framing and queued ingress.
    pub fn send_client_command(
        &mut self,
        id: ClientId,
        command: &qa_core::primitives::UserCmd,
        time: qa_core::sys_events::EventTime,
        source: &mut impl host::FrameSource,
    ) -> bool {
        let signing_on = self
            .client_resources
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .is_some_and(|resources| resources.awaiting_startup())
            || self
                .snapshots
                .get(id.0 as usize)
                .and_then(Option::as_ref)
                .is_some_and(|binding| binding.native_client().is_none());
        let Some(connection) = self
            .network
            .get_mut(id, qa_core::loopback::Endpoint::Client)
        else {
            return false;
        };
        if !connection.phase.is_connected() {
            return false;
        }
        let Some(commands) = &mut connection.commands else {
            return false;
        };
        let route = connection.route;
        let recorded = if signing_on {
            None
        } else {
            let Ok(command) = qa_network::commands::canonical_command(commands.protocol, *command)
            else {
                return false;
            };
            // Active CLIENT numbers advance even while a preceding flight waits.
            let Ok(number) = commands.record(command) else {
                return false;
            };
            Some((command, number))
        };
        // Startup has no move. Pending fragments retain their original move
        // watermark. Both use the same control-only submission path.
        if recorded.is_none()
            || connection.channel.pending_packet().is_some()
            || connection.channel.pending_fragments()
        {
            transport::send(
                &mut connection.channel,
                time,
                &mut (),
                |channel, ()| {
                    channel
                        .prepare(None, time)
                        .is_ok_and(|packet| packet.is_some())
                },
                |bytes| {
                    transport::admit_output(
                        &mut self.loopback,
                        &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                        qa_core::loopback::Endpoint::Client,
                        route,
                        bytes,
                    )
                },
                |channel, (), _| !channel.pending_fragments(),
            );
            return false;
        }
        let Some((command, number)) = recorded else {
            return false;
        };
        let Ok(maximum) = connection.channel.native_payload_capacity() else {
            return false;
        };
        let Some(bytes) = self.output_native_bytes.get_mut(..maximum) else {
            return false;
        };
        let registry = &self.catalog.registry;
        let Ok(length) = commands.encode(
            &command,
            |weapon| registry.command_weapon_number(weapon),
            &connection.channel,
            bytes,
        ) else {
            return false;
        };
        let snapshot_request = commands.snapshot_request(&connection.channel);
        transport::send(
            &mut connection.channel,
            time,
            &mut false,
            |channel, _| {
                channel
                    .prepare_move(
                        &bytes[..length],
                        time,
                        snapshot_request,
                        Some(qa_network::channel::MoveCommand {
                            number,
                            server_time_ms: command.server_time_ms,
                        }),
                    )
                    .is_ok_and(|packet| packet.is_some())
            },
            |bytes| {
                transport::admit_output(
                    &mut self.loopback,
                    &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                    qa_core::loopback::Endpoint::Client,
                    route,
                    bytes,
                )
            },
            |channel, included, disposition| {
                *included |= disposition == qa_network::channel::Unreliable::Included;
                *included && !channel.pending_fragments()
            },
        )
        .complete
    }

    /// SERVER snapshots use the same native framing and queued Packet path as
    /// commands. A rejected transport retains its exact prepared bytes.
    pub fn send_snapshot(
        &mut self,
        id: ClientId,
        time: qa_core::sys_events::EventTime,
        observation: Option<qa_core::primitives::SnapshotObservation>,
        source: &mut impl host::FrameSource,
        mut bound_sound: impl FnMut(ModuleId, u32) -> Option<qa_core::primitives::SoundId>,
        mut bound_model: impl FnMut(
            ModuleId,
            u32,
            Option<GeometryId>,
            qa_core::primitives::ModelUse,
        ) -> Option<crate::client_resources::ModelName>,
    ) -> bool {
        use qa_core::loopback::Endpoint;
        use qa_network::channel::Unreliable;
        let client = &self.server.clients[id.0 as usize];
        let Some(binding) = self.snapshots[id.0 as usize].as_mut() else {
            return false;
        };
        if client.connection.is_none() || client.output != Some(binding.output) {
            return false;
        }
        let Some(connection) = self.network.get_mut(id, Endpoint::Server) else {
            return false;
        };
        if connection.pending_output.is_some() {
            return false;
        }
        let mut bytes = &mut *self.output_bytes;
        let native_module = self.server.entities.columns.native_entity[client.entity.slot as usize]
            .map(|native| native.module);
        let publication = &self.config_publications[id.0 as usize];
        let model_world = crate::client_resources::model_world(
            client.connection,
            self.client_resources[id.0 as usize].as_ref(),
            self.collision.as_ref().map(|world| world.geometry),
        );
        let mut resolve_model = |reference, usage| match publication.project_model_ref(
            reference,
            usage,
            id,
            |module, ordinal, usage| bound_model(module, ordinal, model_world, usage),
        ) {
            client_resources::ModelProjection::Ready(ordinal) => Some(ordinal),
            client_resources::ModelProjection::Pending
            | client_resources::ModelProjection::Unsupported => None,
        };
        let mut resolve_sound =
            |reference| publication.project_sound_ref(reference, id, &mut bound_sound);
        let collision = &mut self.collision;
        let route = connection.route;
        let mut dropped = false;
        let mut snapshot_completed = false;
        let control_only = std::cell::Cell::new(false);
        let submitted = transport::send(
            &mut connection.channel,
            time,
            binding,
            |channel, binding| {
                if channel.pending_controls() != 0 || channel.pending_fragments() {
                    channel
                        .prepare_output(time)
                        .is_ok_and(|packet| packet.is_some())
                } else {
                    let sequence = binding.sequence(channel, time);
                    if !binding.needs_send(channel, sequence) {
                        control_only.set(true);
                        return channel.has_output()
                            && channel
                                .prepare_output(time)
                                .is_ok_and(|packet| packet.is_some());
                    }
                    let delta = connection
                        .commands
                        .as_ref()
                        .and_then(|commands| commands.delta_request());
                    let (visibility, portals) = match collision.as_mut() {
                        Some(world) => (
                            world.visibility.as_deref().map(|visibility| {
                                (visibility, &mut world.leaves, &mut *world.visibility_row)
                            }),
                            world.portals.as_ref(),
                        ),
                        None => (None, None),
                    };
                    let Ok(length) = binding.encode(
                        channel,
                        &client.player,
                        snapshots::EntitySource {
                            receiver: id,
                            entities: &self.server.entities,
                            players: &self.server.clients,
                            registry: &self.catalog.registry,
                            native_module,
                            self_entity: client.entity,
                            visibility,
                            portals,
                            resolve_model: &mut resolve_model,
                            resolve_sound: &mut resolve_sound,
                        },
                        &mut self.native_numbers[id.0 as usize],
                        sequence,
                        time,
                        delta,
                        &mut bytes,
                    ) else {
                        return false;
                    };
                    let Ok(Some(packet)) =
                        channel.prepare_snapshot(sequence, observation, &bytes[..length], time)
                    else {
                        return false;
                    };
                    if packet.unreliable == Unreliable::Included {
                        binding.pending = Some(sequence);
                    }
                    true
                }
            },
            |bytes| {
                transport::admit_output(
                    &mut self.loopback,
                    &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                    qa_core::loopback::Endpoint::Server,
                    route,
                    bytes,
                )
            },
            |channel, binding, disposition| {
                if disposition == Unreliable::Dropped {
                    binding.pending = None;
                    dropped = true;
                    true
                } else if !channel.pending_fragments()
                    && let Some(sequence) = binding.pending.take()
                {
                    binding.submitted(sequence);
                    snapshot_completed = true;
                    true
                } else {
                    control_only.get() && !channel.pending_fragments()
                }
            },
        );
        submitted.complete && snapshot_completed && !dropped
    }

    /// Shared print service for console and gameplay/module callers.
    pub fn print_event(
        &mut self,
        client: Option<ClientId>,
        kind: PrintKind,
        text: std::fmt::Arguments<'_>,
    ) {
        let _ = self.server.events.print(client, kind, text);
    }
}

impl Host for Runtime {
    fn command_producer_live(&self, producer: qa_console::views::CommandProducer) -> bool {
        self.server
            .entities
            .resolve(producer.anchor)
            .is_some_and(|slot| self.server.entities.columns.owner[slot] == producer.module)
    }
    fn input(&mut self) -> &mut qa_input::Input {
        &mut self.input
    }
    fn input_time(&self) -> qa_core::sys_events::EventTime {
        self.input_time
    }
    fn print(&mut self, text: std::fmt::Arguments<'_>) {
        self.print_event(None, PrintKind::Console, text);
    }
    fn print_bytes(&mut self, text: &[u8]) {
        let _ = self.server.events.console_bytes(text);
    }
    fn reject_input(&mut self, count: u64) {
        for _ in 0..count {
            self.server.events.reject_publication();
        }
    }
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError> {
        let file = self.vfs.open(path.as_bytes()).ok_or(ScriptError::Missing)?;
        self.vfs
            .read_whole_into(&file, destination, &mut self.script_reader)
            .map_err(|error| match error {
                qa_content::vfs::VfsError::Missing => ScriptError::Missing,
                qa_content::vfs::VfsError::Size | qa_content::vfs::VfsError::Limit => {
                    ScriptError::TooLong
                }
                _ => ScriptError::Read,
            })
    }
    fn quit(&mut self) {
        self.quit = true;
    }
}
