use qa_core::events::{EventRing, OutputConsumerId, OutputTarget};
use qa_core::primitives::{
    Bounds, ClientId, CollisionOwner, CollisionShape, CommandExecution, CommandIntent, EntityId,
    GeometryId, HudState, ModuleId, NativeEntity, PlayerState, RuleSetId, SnapshotObservation,
    UserCmd, Vec3,
};
use qa_core::sys_events::EventTime;
use qa_world::{
    area::{AreaGrid, AttachmentTransport, LinkFlags, LinkIntent, LinkOrder},
    collision::{CollisionStore, Contents, TraceError, TraceScratch, WorldTrace},
    entities::EntityTable,
};

mod hud;
pub use hud::HudPublication;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Local,
    Remote,
    Bot,
}

/// Retained until the Presentation consumer has applied the retirement.
/// A canceled ring publication is retried with the same canonical lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioRetirement {
    pub lifetime: EntityId,
    pub publication: Option<(OutputConsumerId, u64)>,
}

pub struct Client {
    pub connection: Option<Connection>,
    pub entity: EntityId,
    pub module: ModuleId,
    /// Selected client policy; the built-in gate does not imply a guest load.
    pub client_rules: RuleSetId,
    /// Native spatial insertion order is independent of movement rules.
    pub link_order: LinkOrder,
    pub player: PlayerState,
    pub hud: HudState,
    pub hud_publication: HudPublication,
    pub command: UserCmd,
    pub command_execution: CommandExecution,
    pub command_observation: Option<SnapshotObservation>,
    /// Raw native connect parameters, retained on the canonical client slot.
    pub module_spawn_parms: [u32; 16],
    pub value_publication: qa_core::primitives::ValueBank,
    pub intent: CommandIntent,
    pub output: Option<OutputConsumerId>,
    pub audio_retirement: Option<AudioRetirement>,
    /// An admitted native Q2 client advances inside ClientThink through Pmove.
    pub module_movement: bool,
    command_pending: bool,
}

impl Client {
    // audio_retirement is completed by Presentation, never by a state reset.
    fn reset(&mut self, texts: &mut qa_core::events::TextStore, module: ModuleId) {
        self.player.reset();
        self.hud.reset(texts);
        self.hud_publication.reset();
        self.command = UserCmd::default();
        self.command_execution = CommandExecution::default();
        self.command_observation = None;
        self.module_spawn_parms = [0; 16];
        self.value_publication.clear();
        self.command_pending = false;
        self.module_movement = false;
        self.intent = CommandIntent::default();
        self.module = module;
        self.client_rules = RuleSetId::default();
        self.link_order = LinkOrder::Tail;
    }
}

pub struct Server {
    pub entities: EntityTable,
    pub area: AreaGrid,
    pub navigation: qa_navigation::Observers,
    pub clients: Box<[Client]>,
    pub events: EventRing,
    pub presentation: OutputConsumerId,
    pub world_time: EventTime,
    pub world_frame: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServerError {
    ClientCapacity,
    InventoryCapacity,
    ValueCapacity,
    EntityCapacity,
    OutputCapacity,
}

impl Server {
    /// Sizes are load-time slot counts. Item zero is unused; numeric value zero
    /// is valid. The app's catalog supplies the common numeric schema capacity.
    pub fn load(
        max_clients: usize,
        entity_capacity: usize,
        items: usize,
        powerups: usize,
        weapons: usize,
        values: usize,
    ) -> Result<Self, ServerError> {
        if max_clients == 0 || max_clients > u32::MAX as usize {
            return Err(ServerError::ClientCapacity);
        }
        if items == 0 || items > 65536 || powerups > 65536 || weapons > 65536 {
            return Err(ServerError::InventoryCapacity);
        }
        if u32::try_from(values.saturating_sub(1)).is_err() {
            return Err(ServerError::ValueCapacity);
        }
        let reserved = max_clients
            .checked_add(1)
            .ok_or(ServerError::ClientCapacity)?;
        let entities =
            EntityTable::new(entity_capacity, reserved).map_err(|_| ServerError::EntityCapacity)?;
        // Map load replaces these bounds before linking its entities.
        let area = AreaGrid::load(
            entity_capacity,
            Bounds {
                mins: Vec3([-131072.0; 3]),
                maxs: Vec3([131072.0; 3]),
            },
        )
        .map_err(|_| ServerError::EntityCapacity)?;
        let clients = (0..max_clients)
            .map(|slot| Client {
                connection: None,
                entity: EntityId {
                    slot: slot as u32 + 1,
                    generation: 1,
                },
                module: ModuleId::default(),
                client_rules: RuleSetId::default(),
                link_order: LinkOrder::Tail,
                player: PlayerState::with_capacity(items, powerups, values),
                command: UserCmd::default(),
                command_execution: CommandExecution::default(),
                command_observation: None,
                module_spawn_parms: [0; 16],
                value_publication: qa_core::primitives::ValueBank::load(values),
                intent: CommandIntent::default(),
                command_pending: false,
                module_movement: false,
                output: None,
                audio_retirement: None,
                hud: HudState::with_capacity(items, powerups, weapons, values),
                hud_publication: HudPublication::with_capacity(items, powerups),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        // Ring pages plus six independent display slots per client/module.
        let rows = 4096usize
            .checked_add(qa_core::events::CONSOLE_DISPLAY_RECORDS)
            .ok_or(ServerError::OutputCapacity)?
            .checked_add(
                max_clients
                    .checked_add(32)
                    .and_then(|n| n.checked_mul(6))
                    .ok_or(ServerError::OutputCapacity)?,
            )
            .ok_or(ServerError::OutputCapacity)?;
        let mut events = EventRing::load(
            4096,
            max_clients + 33,
            rows,
            qa_core::primitives::OUTPUT_PAGE_BYTES,
        )
        .map_err(|_| ServerError::OutputCapacity)?;
        let presentation = events
            .bind(OutputTarget::Presentation)
            .ok_or(ServerError::OutputCapacity)?;
        Ok(Self {
            events,
            presentation,
            entities,
            area,
            navigation: qa_navigation::Observers::load(entity_capacity),
            clients,
            world_time: EventTime::default(),
            world_frame: 0,
        })
    }

    pub fn connect(
        &mut self,
        connection: Connection,
        module: ModuleId,
        native: Option<NativeEntity>,
    ) -> Option<ClientId> {
        let slot = self.clients.iter().position(|client| {
            client.connection.is_none()
                && client.audio_retirement.is_none()
                && self.entities.resolve(client.entity).is_some()
        })?;
        let output = if connection == Connection::Bot {
            None
        } else {
            Some(
                self.events
                    .bind(OutputTarget::Client(ClientId(slot as u32)))?,
            )
        };
        let client = &mut self.clients[slot];
        client.output = output;
        client.reset(&mut self.events.texts, module);
        client.connection = Some(connection);
        let entity_slot = client.entity.slot as usize;
        self.entities.columns.owner[entity_slot] = module;
        // A native module supplies its own raw namespace. Q3's client zero is
        // not the common world's reserved slot zero; an unbound shell has none.
        self.entities.columns.native_entity[entity_slot] = native;
        self.entities.columns.collision_shape[entity_slot] = CollisionShape::Box;
        self.entities.columns.collision_contents[entity_slot] = Contents::BODY.0;
        Some(ClientId(slot as u32))
    }

    pub fn client_for_native(&self, native: NativeEntity) -> Option<ClientId> {
        self.clients
            .iter()
            .position(|client| {
                client.connection.is_some()
                    && self.entities.resolve(client.entity).is_some()
                    && self.entities.columns.native_entity[client.entity.slot as usize]
                        == Some(native)
            })
            .map(|slot| ClientId(slot as u32))
    }

    /// Only an admitted client with a live entity in its owning native module
    /// routes movement through ClientThink instead of the engine mover.
    pub fn module_moves(&self, id: ClientId) -> Option<NativeEntity> {
        let client = self.clients.get(id.0 as usize)?;
        if client.connection.is_none() || !client.module_movement {
            return None;
        }
        let slot = self.entities.resolve(client.entity)?;
        self.entities.columns.native_entity[slot].filter(|native| native.module == client.module)
    }

    /// A module supplies its namespace after its API and entity table bind.
    pub fn bind_client_native(&mut self, id: ClientId, native: NativeEntity) -> bool {
        let Some(client) = self.clients.get(id.0 as usize) else {
            return false;
        };
        if client.connection.is_none() || self.entities.resolve(client.entity).is_none() {
            return false;
        }
        if self
            .entities
            .find_native(native, Some(client.entity))
            .is_some()
        {
            return false;
        }
        let client = &mut self.clients[id.0 as usize];
        let slot = client.entity.slot as usize;
        if client.module != native.module
            || self.entities.columns.native_entity[slot] != Some(native)
        {
            client.player.weapon_state = None;
        }
        client.module = native.module;
        self.entities.columns.owner[slot] = native.module;
        self.entities.columns.native_entity[slot] = Some(native);
        true
    }

    pub fn disconnect(&mut self, id: ClientId) -> bool {
        let Some(client) = self
            .clients
            .get_mut(id.0 as usize)
            .filter(|client| client.connection.is_some())
        else {
            return false;
        };
        if let Some(output) = client.output.take() {
            self.events.unbind(output);
        }
        self.area.unlink(client.entity);
        client.connection = None;
        client.reset(&mut self.events.texts, ModuleId::default());
        if let Some(entity) = self.entities.reset_client(client.entity) {
            client.entity = entity;
        }
        true
    }
    /// SERVER world ticks build all connected bot commands in client-id order.
    /// Bot modules update the same primitive intent, without local-seat state.
    pub fn build_bot_commands(
        &mut self,
        start: EventTime,
        end: EventTime,
        mut policy: impl FnMut(RuleSetId) -> qa_input::InputPolicy,
    ) {
        let duration = EventTime(end.since(start));
        for slot in 0..self.clients.len() {
            if self.clients[slot].connection != Some(Connection::Bot) {
                continue;
            }
            let module_moves = self.module_moves(ClientId(slot as u32)).is_some();
            let client = &mut self.clients[slot];
            let command = qa_input::UserCmdBuilder::build(
                duration,
                end,
                client.intent,
                policy(client.player.movement_rules),
            );
            client.command = if module_moves {
                command
            } else {
                qa_movement::prepare_command(client.player.movement_rules, command)
            };
            client.command_execution = CommandExecution::PerCommand;
            client.command_observation = None;
            client.command_pending = true;
        }
    }

    /// Ingress submits a command once. There is no recorded input history.
    pub fn submit_command(
        &mut self,
        id: ClientId,
        command: UserCmd,
        execution: CommandExecution,
        observation: Option<SnapshotObservation>,
    ) {
        let Some(client) = self
            .clients
            .get_mut(id.0 as usize)
            .filter(|client| client.connection.is_some())
        else {
            return;
        };
        client.command = command;
        client.command_execution = execution;
        client.command_observation = observation;
        client.command_pending = true;
    }

    /// Frame-latest wire commands have no msec field. Resolve them against the
    /// queued SERVER frame which consumes them, never a packet-arrival span.
    pub fn resolve_frame_command(&mut self, id: ClientId, span: EventTime, end: EventTime) {
        let module_moves = self.module_moves(id).is_some();
        let Some(client) = self.clients.get_mut(id.0 as usize) else {
            return;
        };
        if client.command_pending && client.command_execution == CommandExecution::FrameLatest {
            let command = UserCmd {
                duration_ms: span.milliseconds().min(u64::from(u16::MAX)) as u16,
                duration_ns: span.0,
                server_time_ms: end.milliseconds() as i32,
                ..client.command
            };
            client.command = if module_moves {
                command
            } else {
                qa_movement::prepare_command(client.player.movement_rules, command)
            };
        }
    }

    /// Consume the one admitted command. Native module hosts use this same
    /// pending bit before ClientThink; taking it never runs engine movement.
    pub fn take_pending_command(
        &mut self,
        id: ClientId,
    ) -> Option<(UserCmd, Option<SnapshotObservation>)> {
        let client = self.clients.get_mut(id.0 as usize)?;
        if client.connection.is_none() || !client.command_pending {
            return None;
        }
        client.command_pending = false;
        Some((client.command, client.command_observation.take()))
    }

    /// The single authoritative engine movement/body/link operation. Packet
    /// protocols consume here before admitting their next command; frame-latest
    /// protocols and bots use the batch below. The caller transports attachments
    /// after its ordered body commits through commit_attachments.
    pub fn move_pending_client(
        &mut self,
        id: ClientId,
        store: &CollisionStore,
        geometry: GeometryId,
        index: u32,
        scratch: &mut TraceScratch,
    ) -> Result<u32, TraceError> {
        let module_moves = self.module_moves(id).is_some();
        let Some((command, _)) = self.take_pending_command(id) else {
            return Ok(0);
        };
        if module_moves {
            return Ok(0);
        }
        let result = self.move_client(
            id,
            command,
            store,
            geometry,
            index,
            scratch,
            Default::default(),
            crate::physics::no_body_boundary,
        )?;
        let client = &mut self.clients[id.0 as usize];
        self.entities
            .columns
            .set_body(client.entity.slot as usize, client.player.body);
        self.area.link(
            &self.entities,
            client.entity,
            LinkFlags::SOLID,
            client.link_order,
            LinkIntent::Commit,
        );
        Ok(result.steps)
    }

    /// The same authoritative movement entry also serves a module lifecycle's
    /// native phase. Its ABI adapter commits and relinks the returned pose.
    pub fn move_client<E, B>(
        &mut self,
        id: ClientId,
        command: UserCmd,
        store: &CollisionStore,
        geometry: GeometryId,
        index: u32,
        scratch: &mut TraceScratch,
        options: qa_movement::MoveOptions,
        mut boundary: B,
    ) -> Result<qa_movement::MovementResult, E>
    where
        E: From<TraceError>,
        B: FnMut(&EntityTable, &mut PlayerState, qa_core::primitives::MotionWrite) -> Result<(), E>,
    {
        let client = &mut self.clients[id.0 as usize];
        let mut trace = WorldTrace::new(
            store,
            geometry,
            index,
            &self.entities,
            &self.area,
            scratch,
            CollisionOwner::Lifetime(client.entity),
        );
        qa_movement::pmove(
            command,
            &mut client.player,
            &mut trace,
            options,
            |player, field| boundary(&self.entities, player, field),
        )
    }

    /// Local, network and bot clients share the same authoritative entry.
    /// Caller drains ingress before this SERVER phase; commands are consumed
    /// once even when several world/provider ticks occur in a host frame.
    pub fn move_pending_clients(
        &mut self,
        store: &CollisionStore,
        geometry: GeometryId,
        index: u32,
        scratch: &mut TraceScratch,
        mut rejected: impl FnMut(ClientId, TraceError),
    ) -> u32 {
        let mut steps = 0;
        for client_index in 0..self.clients.len() {
            let id = ClientId(client_index as u32);
            match self.move_pending_client(id, store, geometry, index, scratch) {
                Ok(count) => steps += count,
                Err(error) => rejected(id, error),
            }
        }
        self.commit_attachments();
        steps
    }

    /// Authoritative body commits transport the shared graph once, then update
    /// attached clients' hot state before snapshots and prediction are copied.
    pub fn commit_attachments(&mut self) -> AttachmentTransport {
        let result = self.area.transport_attachments(&mut self.entities);
        if result.moved != 0 {
            for client in self.clients.iter_mut() {
                if client.connection.is_some() && self.entities.attachment(client.entity).is_some()
                {
                    client.player.body = self.entities.columns.body(client.entity.slot as usize);
                }
            }
        }
        result
    }
}
