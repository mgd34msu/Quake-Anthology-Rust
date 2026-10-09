use qa_core::events::{EventRing, OutputConsumerId, OutputTarget};
use qa_core::primitives::{
    Bounds, ClientId, CollisionShape, CommandIntent, EntityId, GeometryId, HudState, ModuleId,
    NativeEntity, PlayerState, PlayerTail, RuleSetId, UserCmd, Vec3,
};
use qa_core::sys_events::EventTime;
use qa_world::{
    area::{AreaGrid, AttachmentTransport, LinkFlags, LinkIntent, LinkOrder},
    collision::{CollisionStore, Contents, TraceScratch, WorldTrace},
    entities::EntityTable,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Local,
    Remote,
    Bot,
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
    pub command: UserCmd,
    pub intent: CommandIntent,
    pub output: Option<OutputConsumerId>,
    command_pending: bool,
}

pub struct Server {
    pub entities: EntityTable,
    pub area: AreaGrid,
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
                intent: CommandIntent::default(),
                command_pending: false,
                output: None,
                hud: HudState::with_capacity(items, powerups, weapons, values),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        // Ring pages plus six independent display slots per client/module.
        let rows = 4096usize
            .checked_add(
                max_clients
                    .checked_add(32)
                    .and_then(|n| n.checked_mul(6))
                    .ok_or(ServerError::OutputCapacity)?,
            )
            .ok_or(ServerError::OutputCapacity)?;
        let mut events = EventRing::load(4096, max_clients + 33, rows, 8192)
            .map_err(|_| ServerError::OutputCapacity)?;
        let presentation = events
            .bind(OutputTarget::Presentation)
            .ok_or(ServerError::OutputCapacity)?;
        Ok(Self {
            events,
            presentation,
            entities,
            area,
            clients,
            world_time: EventTime::default(),
            world_frame: 0,
        })
    }

    pub fn connect(
        &mut self,
        connection: Connection,
        module: ModuleId,
        tail: PlayerTail,
        native: Option<NativeEntity>,
    ) -> Option<ClientId> {
        let slot = self.clients.iter().position(|client| {
            client.connection.is_none() && self.entities.resolve(client.entity).is_some()
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
        client.player.reset();
        client.hud.reset(&mut self.events.texts);
        client.player.tail = tail;
        client.command = UserCmd::default();
        client.command_pending = false;
        client.intent = CommandIntent::default();
        client.module = module;
        client.client_rules = RuleSetId::default();
        client.link_order = LinkOrder::Tail;
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
        client.player.reset();
        client.hud.reset(&mut self.events.texts);
        client.command = UserCmd::default();
        client.command_pending = false;
        client.intent = CommandIntent::default();
        client.module = ModuleId::default();
        client.client_rules = RuleSetId::default();
        client.link_order = LinkOrder::Tail;
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
        let duration = std::time::Duration::from_nanos(end.since(start));
        for client in self.clients.iter_mut() {
            if client.connection == Some(Connection::Bot) {
                let command = qa_input::UserCmdBuilder::build(
                    duration,
                    end,
                    client.intent,
                    policy(client.player.movement_rules),
                );
                client.command =
                    qa_movement::prepare_command(client.player.movement_rules, command);
                client.command_pending = true;
            }
        }
    }

    /// Ingress submits a command once. There is no recorded input history.
    pub fn submit_command(&mut self, id: ClientId, command: UserCmd) {
        let Some(client) = self
            .clients
            .get_mut(id.0 as usize)
            .filter(|client| client.connection.is_some())
        else {
            return;
        };
        client.command = command;
        client.command_pending = true;
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
    ) -> u32 {
        let mut steps = 0;
        for client in self.clients.iter_mut() {
            if client.connection.is_some() && client.command_pending {
                client.command_pending = false;
                let result = {
                    let mut trace = WorldTrace::new(
                        store,
                        geometry,
                        index,
                        &self.entities,
                        &self.area,
                        scratch,
                        Some(client.entity),
                    );
                    qa_movement::pmove(client.command, &mut client.player, &mut trace)
                };
                steps += result.steps;
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
