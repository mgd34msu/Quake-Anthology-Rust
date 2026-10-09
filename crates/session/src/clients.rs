use qa_core::primitives::{
    Bounds, ClientId, CollisionShape, CommandIntent, EntityId, GeometryId, HudState, ModuleId,
    NativeEntity, PlayerState, PlayerTail, UserCmd, Vec3,
};
use qa_core::sys_events::EventTime;
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
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
    /// Native spatial insertion order is independent of movement rules.
    pub link_order: LinkOrder,
    pub player: PlayerState,
    pub hud: HudState,
    pub command: UserCmd,
    pub intent: CommandIntent,
    command_pending: bool,
}

pub struct Server {
    pub entities: EntityTable,
    pub area: AreaGrid,
    pub clients: Box<[Client]>,
    pub world_time: EventTime,
    pub world_frame: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServerError {
    ClientCapacity,
    InventoryCapacity,
    EntityCapacity,
}

impl Server {
    /// Arena sizes are slot counts, including any unused zero handle. The app's
    /// gameplay catalogue sizes acquisition and timer rows by common ItemId.
    pub fn load(
        max_clients: usize,
        entity_capacity: usize,
        items: usize,
        powerups: usize,
        weapons: usize,
    ) -> Result<Self, ServerError> {
        if max_clients == 0 || max_clients > u32::MAX as usize {
            return Err(ServerError::ClientCapacity);
        }
        if items == 0 || items > 65536 || powerups > 65536 || weapons > 65536 {
            return Err(ServerError::InventoryCapacity);
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
                link_order: LinkOrder::Tail,
                player: PlayerState::with_capacity(items, powerups),
                command: UserCmd::default(),
                intent: CommandIntent::default(),
                command_pending: false,
                hud: HudState::with_capacity(items, powerups, weapons),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
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
        let client = &mut self.clients[slot];
        client.player.reset();
        client.hud.reset();
        client.player.tail = tail;
        client.command = UserCmd::default();
        client.command_pending = false;
        client.intent = CommandIntent::default();
        client.module = module;
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
        self.area.unlink(client.entity);
        client.connection = None;
        client.player.reset();
        client.hud.reset();
        client.command = UserCmd::default();
        client.command_pending = false;
        client.intent = CommandIntent::default();
        client.module = ModuleId::default();
        client.link_order = LinkOrder::Tail;
        if let Some(entity) = self.entities.reset_client(client.entity) {
            client.entity = entity;
        }
        true
    }
    /// SERVER world ticks build all connected bot commands in client-id order.
    /// Bot modules update the same primitive intent, without local-seat state.
    pub fn build_bot_commands(&mut self, start: EventTime, end: EventTime) {
        let duration = std::time::Duration::from_nanos(end.since(start));
        for client in self.clients.iter_mut() {
            if client.connection == Some(Connection::Bot) {
                let command = qa_input::UserCmdBuilder::build(duration, end, client.intent);
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
        steps
    }
}
