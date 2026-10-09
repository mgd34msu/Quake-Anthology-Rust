pub mod catalog;
pub mod client_policy;
pub mod host;
pub mod map;
pub mod output;
pub mod profile;
pub mod render_settings;
pub mod renderer;

use qa_console::commands::{Host, ScriptError};
use qa_content::vfs::Vfs;
use qa_core::loopback::Loopback;
use qa_core::primitives::{ClientId, GeometryId, ModuleId, PlayerTail, PrintKind};
use qa_network::ingress::PacketReceiver;
use qa_session::clients::{Connection, Server};

pub struct Runtime {
    pub catalog: catalog::GameplayCatalog,
    pub vfs: Vfs,
    /// Exact map entity sources in load order, retained for native module import.
    pub entity_sources: Vec<map::NativeEntityText>,
    pub quit: bool,
    pub network: PacketReceiver,
    pub server: Server,
    /// One preallocated index over the server's generation-checked entities.
    pub targets: qa_world::targets::TargetIndex,
    /// One owner of admitted geometry for every active world and inline model.
    pub geometry: qa_world::collision::CollisionStore,
    /// Loaded geometry is independent of every player's movement rules.
    pub collision: Option<WorldCollision>,
    pub prediction: [qa_session::prediction::Prediction; qa_core::sys_events::SeatId::COUNT],
    pub loopback: Loopback,
    pub input: qa_input::Input,
    pub input_time: qa_core::sys_events::EventTime,
    script_reader: qa_formats::archive::ArchiveReader,
}

pub struct WorldCollision {
    pub geometry: GeometryId,
    pub index: u32,
    pub scratch: qa_world::collision::TraceScratch,
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
        }
    }
}

impl Runtime {
    pub fn load<'a>(extra_names: impl IntoIterator<Item = &'a [u8]>) -> Result<Self, String> {
        let catalog = catalog::GameplayCatalog::load(extra_names)?;
        let item_slots = catalog.registry.items.len() + 1;
        let weapon_slots = catalog.registry.weapons.len() + 1;
        // Inventory, acquisition times and powerup timers all use common ItemId.
        // Zero is unused; instant and non-powerup items retain zero timer rows.
        // Native powerup ordinals are converted at module/protocol boundaries.
        let server = Server::load(
            64,
            8192,
            item_slots,
            item_slots,
            weapon_slots,
            catalog.hud.values.capacity(),
        )
        .map_err(|e| format!("server: {e:?}"))?;
        let targets = qa_world::targets::TargetIndex::new(&server.entities);
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
        Ok(Self {
            catalog,
            vfs: Vfs::default(),
            entity_sources: Vec::new(),
            quit: false,
            network: PacketReceiver::default(),
            server,
            targets,
            geometry: qa_world::collision::CollisionStore::new(),
            collision: None,
            prediction: std::array::from_fn(|_| Default::default()),
            loopback,
            input: qa_input::Input::load(),
            input_time: qa_core::sys_events::EventTime::default(),
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
    ) -> Result<ClientId, String> {
        let id = self
            .server
            .connect(
                Connection::Local,
                ModuleId::default(),
                PlayerTail::None,
                None,
            )
            .ok_or("no local client slot")?;
        let client = &mut self.server.clients[id.0 as usize];
        client.client_rules = policy.client;
        client.link_order = policy.link_order();
        client.player.movement_rules = policy.movement;
        client.player.trace_rules = policy.trace;
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = spawn.position;
        client.player.view_angles = spawn.angles;
        client.player.health = 100;
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
        self.prediction[seat.index()].apply_snapshot(&client.player);
        self.input.set_view_angles(seat, spawn.angles);
        Ok(id)
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
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError> {
        let file = self.vfs.open(path.as_bytes()).ok_or(ScriptError::Missing)?;
        let length = self.vfs.length(file).map_err(|_| ScriptError::Read)?;
        let length = usize::try_from(length).map_err(|_| ScriptError::TooLong)?;
        if length > destination.len() {
            return Err(ScriptError::TooLong);
        }
        let read = self
            .vfs
            .read_into_reusing(file, &mut destination[..length], &mut self.script_reader)
            .map_err(|_| ScriptError::Read)?;
        if read != length {
            return Err(ScriptError::Read);
        }
        Ok(length)
    }
    fn quit(&mut self) {
        self.quit = true;
    }
}
