pub mod audio;
pub mod catalog;
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
pub mod snapshots;
pub mod startup;
mod transport;

use qa_console::commands::{Host, ScriptError};
use qa_content::vfs::Vfs;
use qa_core::loopback::Loopback;
use qa_core::primitives::{
    ClientId, GeometryId, ModuleId, NATIVE_PAYLOAD_BYTES, NATIVE_WIRE_BYTES, PlayerTail, PrintKind,
};
use qa_network::ingress::Connections;
use qa_session::clients::{Connection, Server};

pub struct Runtime {
    pub sound_bank: Option<std::sync::Arc<qa_audio::Bank>>,
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
    pub native_numbers: Box<[output_namespace::NativeNumbers]>,
    pub(crate) config_publications: Box<[client_resources::ConfigPublication]>,
    pub output_duplicates: output::duplicates::Coordinator,
    pub models: Option<std::sync::Arc<model_assets::ModelCatalog>>,
    /// Cold owned assets and metadata become the same frozen catalog after precache.
    pub model_loader: Option<model_assets::ModelCatalogBuilder>,
    pub loopback: Loopback,
    pub input: qa_input::Input,
    pub input_time: qa_core::sys_events::EventTime,
    pub(crate) output_bytes: Box<[u8]>,
    pub(crate) output_native_bytes: Box<[u8]>,
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

impl Runtime {
    pub fn engine_services<'a>(
        &'a mut self,
        console: &'a mut qa_console::commands::Console<Self>,
        storage: &'a mut qa_compat::services::ServiceStorage,
        scratch: &'a mut qa_world::collision::TraceScratch,
    ) -> qa_compat::services::EngineServices<'a> {
        let (cvars, commands) = console.module_parts();
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
        let model_lookup = if self.model_loader.is_none() {
            self.models
                .as_deref()
                .map(|models| models as &dyn qa_compat::services::ModelLookup)
        } else {
            None
        };
        qa_compat::services::EngineServices {
            server: &mut self.server,
            input: &mut self.input,
            cvars,
            commands,
            vfs: &self.vfs,
            localization: &self.localization,
            registry: &self.catalog.registry,
            names: &mut self.catalog.names,
            model_loader: self
                .model_loader
                .as_mut()
                .map(|loader| loader as &mut dyn qa_compat::services::ModelLoader),
            model_lookup,
            storage,
            geometry: &self.geometry,
            world: identity,
            visibility,
            portals,
            scratch,
        }
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
            sound_bank: None,
            models: None,
            model_loader: None,
            client_resources: std::iter::repeat_with(|| None).take(max_clients).collect(),
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
            prediction: std::array::from_fn(|_| qa_session::prediction::Prediction {
                player: qa_core::primitives::PlayerState::with_capacity(
                    item_slots,
                    item_slots,
                    value_slots,
                ),
            }),
            snapshots: std::iter::repeat_with(|| None).take(max_clients).collect(),
            loopback,
            input: qa_input::Input::load(),
            input_time: qa_core::sys_events::EventTime::default(),
            output_bytes: vec![0; NATIVE_PAYLOAD_BYTES].into_boxed_slice(),
            output_native_bytes: vec![0; NATIVE_WIRE_BYTES].into_boxed_slice(),
            script_reader: qa_formats::archive::ArchiveReader::default(),
        })
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
        use qa_network::commands::packet::Protocol;
        let tail = match protocol {
            Protocol::NetQuake15 | Protocol::QuakeWorld28 => PlayerTail::Q1 {
                attack_finished: 0.0,
            },
            Protocol::Quake2_34 | Protocol::Quake2Repro1038 => PlayerTail::Q2,
            Protocol::Quake3_68 => PlayerTail::Q3 { weapon_time: 0 },
        };
        let id = self
            .server
            .connect(Connection::Local, ModuleId::default(), tail, None)
            .ok_or("no local client slot")?;
        let result = (|| {
            self.output_duplicates.reset_client(id);
            self.network.unbind(id);
            self.input.reset_usercmd_value(seat);
            self.loopback.clear_client(id);
            let client = &mut self.server.clients[id.0 as usize];
            client.client_rules = policy.client;
            client.link_order = qa_gameplay::rules::link_order(policy.client);
            for player in [
                &mut client.player,
                &mut self.prediction[seat.index()].player,
            ] {
                player.reset();
                player.tail = tail;
                player.movement_rules = policy.movement;
                player.trace_rules = policy.trace;
                qa_movement::set_bounds(player);
                player.body.position = spawn.position;
                player.view_angles = spawn.angles;
                player.health = 100;
            }
            self.server
                .entities
                .columns
                .set_body(client.entity.slot as usize, client.player.body);
            self.server.entities.columns.angles[client.entity.slot as usize] = spawn.angles;
            self.server.area.link(
                &self.server.entities,
                client.entity,
                qa_world::area::LinkFlags::SOLID,
                client.link_order,
                qa_world::area::LinkIntent::Explicit,
            );
            for endpoint in [
                qa_core::loopback::Endpoint::Client,
                qa_core::loopback::Endpoint::Server,
            ] {
                let connection = qa_network::ingress::Connection {
                    route: qa_network::ingress::Route {
                        socket: endpoint.socket(),
                        peer: qa_core::sys_events::Peer::Loopback(id),
                    },
                    channel: qa_network::channel::Channel::load(
                        protocol.channel(),
                        endpoint,
                        8192,
                        16,
                    )
                    .map_err(|e| e.to_string())?,
                    output: None,
                    pending_output: None,
                    commands: Some(qa_network::commands::connection::Commands::load(protocol)),
                };
                if endpoint == qa_core::loopback::Endpoint::Server {
                    self.bind_server_connection(id, connection, native_client, Some(seat))?;
                } else {
                    self.network
                        .bind(id, endpoint, connection)
                        .map_err(|e| format!("local channel {e:?}"))?;
                }
            }
            self.input.set_view_angles(seat, spawn.angles);
            Ok(id)
        })();
        if result.is_err() {
            self.disconnect_client(id);
        }
        result
    }

    /// Tear down one connection; audio retains only its canonical CLIENT lifetime.
    pub fn disconnect_client(&mut self, id: ClientId) -> bool {
        let slot = id.0 as usize;
        let Some(client) = self.server.clients.get_mut(slot) else {
            return false;
        };
        if client.connection.is_none() {
            return false;
        }
        if self.client_resources[slot].is_some() {
            debug_assert!(client.audio_retirement.is_none());
            client
                .audio_retirement
                .get_or_insert(qa_session::clients::AudioRetirement {
                    lifetime: client.entity,
                    publication: None,
                });
        }
        self.output_duplicates.reset_client(id);
        self.server.disconnect(id);
        self.network.unbind(id);
        self.loopback.clear_client(id);
        self.snapshots[slot] = None;
        self.client_resources[slot] = None;
        self.config_publications[slot].reset();
        self.native_numbers[slot].reset();
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

    /// A connected native channel supplies its client number explicitly.
    /// Local and remote SERVERs share the same serializer and number owner;
    /// only a local connection creates CLIENT presentation resources.
    pub fn bind_server_connection(
        &mut self,
        id: ClientId,
        mut connection: qa_network::ingress::Connection,
        native_client: u32,
        seat: Option<qa_core::sys_events::SeatId>,
    ) -> Result<(), String> {
        let slot = id.0 as usize;
        if self.client_resources.get(slot).is_some_and(Option::is_some) {
            self.disconnect_client(id);
            return Err(
                "CLIENT resource replacement requires reconnect after audio retirement".into(),
            );
        }
        let client = self.server.clients.get(slot).ok_or("client slot")?;
        if !matches!(
            (client.connection, seat),
            (Some(Connection::Local), Some(_)) | (Some(Connection::Remote), None)
        ) {
            return Err("CLIENT seat must belong to a local connection".into());
        }
        let protocol = connection
            .commands
            .as_ref()
            .ok_or("connection needs native command policy")?
            .protocol;
        let output = client.output.ok_or("client output consumer")?;
        let mut binding = snapshots::SnapshotBinding::load(
            id,
            seat,
            output,
            protocol,
            native_client,
            &self.catalog.hud.values,
        );
        if let Some(visibility) = self
            .collision
            .as_ref()
            .and_then(|world| world.visibility.as_deref())
        {
            binding.load_visibility(visibility);
        }
        let resources = if seat.is_some() {
            client_resources::ClientResources::load(id, protocol, client.client_rules)?
        } else {
            None
        };
        let self_entity = client.entity;
        connection.output = Some(output);
        self.network
            .bind(id, qa_core::loopback::Endpoint::Server, connection)
            .map_err(|e| format!("server channel {e:?}"))?;
        self.snapshots[slot] = Some(binding);
        self.client_resources[slot] = resources;
        self.config_publications[slot].reset();
        self.native_numbers[slot].configure(protocol);
        self.native_numbers[slot].reset();
        self.native_numbers[slot].set_native_clients(self.server.clients.len() as u32);
        self.native_numbers[slot].set_self(self_entity, native_client, protocol);
        Ok(())
    }

    /// Commands are submitted only after native framing and queued ingress.
    pub fn send_local_command(
        &mut self,
        id: ClientId,
        command: &qa_core::primitives::UserCmd,
        time: qa_core::sys_events::EventTime,
    ) -> bool {
        let Some(connection) = self
            .network
            .get_mut(id, qa_core::loopback::Endpoint::Client)
        else {
            return false;
        };
        let Some(commands) = &connection.commands else {
            return false;
        };
        let mut bytes = [0; 1400];
        let native_weapon = self.catalog.registry.command_weapon_number(command.weapon);
        let Ok(length) = commands.encode(command, native_weapon, &connection.channel, &mut bytes)
        else {
            return false;
        };
        let snapshot_request = commands.snapshot_request(&connection.channel);
        transport::send(
            &mut connection.channel,
            time,
            &mut (),
            |channel, ()| {
                channel
                    .prepare_move(&bytes[..length], time, snapshot_request)
                    .is_ok_and(|packet| packet.is_some())
            },
            |bytes| {
                self.loopback
                    .send(qa_core::loopback::Endpoint::Client, id, bytes)
                    .is_ok()
            },
            |_, (), disposition| disposition != qa_network::channel::Unreliable::Deferred,
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
        let model_limit = binding.protocol.resource_layout().resources[0].count;
        let mut resolve_model = |reference| {
            client_resources::model_ordinal(reference, Some(client.module), id, model_limit)
        };
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
                            entities: &self.server.entities,
                            players: &self.server.clients,
                            native_module,
                            self_entity: client.entity,
                            visibility,
                            portals,
                            resolve_model: &mut resolve_model,
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
            |bytes| transport::admit_output(&mut self.loopback, source, route, bytes),
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
    fn input(&mut self) -> &mut qa_input::Input {
        &mut self.input
    }
    fn input_time(&self) -> qa_core::sys_events::EventTime {
        self.input_time
    }
    fn print(&mut self, text: std::fmt::Arguments<'_>) {
        self.print_event(None, PrintKind::Console, text);
    }
    fn reject_input(&mut self, count: u64) {
        for _ in 0..count {
            self.server.events.reject_publication();
        }
    }
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError> {
        let file = self.vfs.open(path.as_bytes()).ok_or(ScriptError::Missing)?;
        self.vfs
            .read_whole_into(file, destination, &mut self.script_reader)
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
