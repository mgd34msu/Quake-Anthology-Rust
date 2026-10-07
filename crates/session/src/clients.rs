use qa_core::primitives::{ClientId, EntityId, ModuleId, PlayerState, PlayerTail, UserCmd};
use qa_world::entities::EntityTable;

pub const MAX_CLIENTS: usize = 64;
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
    pub player: PlayerState,
    pub command: UserCmd,
}

pub struct Server {
    pub entities: EntityTable,
    pub clients: [Client; MAX_CLIENTS],
    limit: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServerError {
    ClientCapacity,
    InventoryCapacity,
    EntityCapacity,
}

impl Server {
    pub fn load(
        max_clients: usize,
        entity_capacity: usize,
        items: usize,
        powerups: usize,
    ) -> Result<Self, ServerError> {
        if max_clients == 0 || max_clients > MAX_CLIENTS {
            return Err(ServerError::ClientCapacity);
        }
        if items == 0 || items > 65536 || powerups > 65536 {
            return Err(ServerError::InventoryCapacity);
        }
        let entities = EntityTable::new(entity_capacity, max_clients + 1)
            .map_err(|_| ServerError::EntityCapacity)?;
        let clients = std::array::from_fn(|slot| Client {
            connection: None,
            entity: EntityId {
                slot: slot as u32 + 1,
                generation: 1,
            },
            module: ModuleId::default(),
            player: PlayerState::with_capacity(
                if slot < max_clients { items } else { 0 },
                if slot < max_clients { powerups } else { 0 },
            ),
            command: UserCmd::default(),
        });
        Ok(Self {
            entities,
            clients,
            limit: max_clients,
        })
    }

    pub fn connect(
        &mut self,
        connection: Connection,
        module: ModuleId,
        tail: PlayerTail,
    ) -> Option<ClientId> {
        let slot = self.clients[..self.limit].iter().position(|client| {
            client.connection.is_none() && self.entities.resolve(client.entity).is_some()
        })?;
        let client = &mut self.clients[slot];
        client.player.reset();
        client.player.tail = tail;
        client.command = UserCmd::default();
        client.module = module;
        client.connection = Some(connection);
        self.entities.columns.owner[client.entity.slot as usize] = module;
        Some(ClientId(slot as u8))
    }

    pub fn disconnect(&mut self, id: ClientId) -> bool {
        let Some(client) = self
            .clients
            .get_mut(id.0 as usize)
            .filter(|client| client.connection.is_some())
        else {
            return false;
        };
        client.connection = None;
        client.player.reset();
        client.command = UserCmd::default();
        client.module = ModuleId::default();
        if let Some(entity) = self.entities.reset_client(client.entity) {
            client.entity = entity;
        }
        true
    }
}
