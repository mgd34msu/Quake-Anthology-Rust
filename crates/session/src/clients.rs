use qa_core::events::{EventRing, OutputConsumerId, OutputTarget};
use qa_core::primitives::{
    Bounds, ClientId, CollisionOwner, CollisionShape, CommandExecution, CommandIntent, EntityId,
    GeometryId, HudState, ModuleId, NativeEntity, PlayerState, RuleSetId, SnapshotObservation,
    ThinkTime, UserCmd, Vec3,
};
use qa_core::sys_events::EventTime;
use qa_core::text::{FixedText, InfoError, InfoRules, InfoUpdate};
use qa_world::{
    area::{AreaGrid, AttachmentTransport, LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, NativeTraceEntities, TraceError, TraceScratch, WorldTrace,
    },
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

/// Canonical client storage accommodates the largest admitted native userinfo
/// boundary. A module supplies its smaller limit at admission.
pub const USERINFO_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientInfoError {
    Client,
    Info(InfoError),
}

impl From<InfoError> for ClientInfoError {
    fn from(error: InfoError) -> Self {
        Self::Info(error)
    }
}

/// Publication names the admitted SERVER lifetime, even after its entity resets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserinfoStamp {
    pub lifetime: EntityId,
    pub user_id: i32,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct UserinfoRecord<'a> {
    pub stamp: UserinfoStamp,
    pub text: &'a [u8],
    pub queued_revision: Option<u64>,
    pub connected: bool,
}
impl UserinfoRecord<'_> {
    pub fn dirty(self) -> bool {
        self.queued_revision != Some(self.stamp.revision)
    }
}

#[derive(Clone, Copy)]
struct UserinfoPublication {
    stamp: UserinfoStamp,
    queued_revision: Option<u64>,
}

/// Retained until the Presentation consumer has applied the retirement.
/// A canceled ring publication is retried with the same canonical lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioRetirement {
    pub lifetime: EntityId,
    pub publication: Option<(OutputConsumerId, u64)>,
}

/// Q3 SV_CheckTimeouts compares integer millisecond clocks with a strict
/// greater-than against sv_zombietime's native integer view.
pub fn close_timeout_expired(started: EventTime, now: EventTime, seconds: i32) -> bool {
    if now < started {
        return false;
    }
    seconds < 0 || now.milliseconds() - started.milliseconds() > seconds as u64 * 1000
}

/// Closing retains the canonical lifetime and native transport until delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientClosing {
    pub started: EventTime,
    pub final_output: Option<(OutputConsumerId, u64)>,
    pub acknowledged: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClientPhase {
    #[default]
    Active,
    Closing(ClientClosing),
}

pub struct Client {
    pub connection: Option<Connection>,
    pub phase: ClientPhase,
    pub entity: EntityId,
    pub module: ModuleId,
    /// Selected client policy; the built-in gate does not imply a guest load.
    pub client_rules: RuleSetId,
    /// Command construction and its cvar defaults are independent of physics.
    pub input_rules: RuleSetId,
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
    userinfo: FixedText<USERINFO_BYTES>,
    userinfo_publication: Option<UserinfoPublication>,
    pub value_publication: qa_core::primitives::ValueBank,
    pub intent: CommandIntent,
    pub output: Option<OutputConsumerId>,
    pub audio_retirement: Option<AudioRetirement>,
    /// An admitted native Q2 client advances inside ClientThink through Pmove.
    pub module_movement: bool,
    server_authoritative: bool,
    command_pending: bool,
}

impl Client {
    pub fn is_active(&self) -> bool {
        self.connection.is_some() && self.phase == ClientPhase::Active
    }

    pub fn server_authoritative(&self) -> bool {
        self.server_authoritative
    }

    pub fn is_server_active(&self) -> bool {
        self.is_active() && self.server_authoritative
    }

    fn userinfo_changed(&mut self) {
        if let Some(publication) = self.userinfo_publication.as_mut() {
            publication.stamp.revision = publication.stamp.revision.wrapping_add(1);
        }
    }

    // audio_retirement is completed by Presentation, never by a state reset.
    fn reset(&mut self, texts: &mut qa_core::events::TextStore, module: ModuleId) {
        self.phase = ClientPhase::Active;
        self.player.reset();
        self.hud.reset(texts);
        self.hud_publication.reset();
        self.command = UserCmd::default();
        self.command_execution = CommandExecution::default();
        self.command_observation = None;
        self.module_spawn_parms = [0; 16];
        self.userinfo.clear();
        self.userinfo_publication = None;
        self.value_publication.clear();
        self.command_pending = false;
        self.module_movement = false;
        self.server_authoritative = false;
        self.intent = CommandIntent::default();
        self.module = module;
        self.client_rules = RuleSetId::default();
        self.input_rules = RuleSetId::default();
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
    next_user_id: i32,
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
    /// All nonclient lifetime endings retire the same spatial/observer state.
    pub fn free_entity(&mut self, entity: EntityId, time: ThinkTime) -> bool {
        if !self.entities.release(entity, time) {
            return false;
        }
        self.area.unlink(entity);
        self.navigation.unregister(entity);
        true
    }

    /// A stopped module's never-free rows no longer have a live owner. Reserved
    /// session/client slots remain borrowed; callers retire connections first.
    pub fn retire_module_entities(&mut self, module: ModuleId, time: ThinkTime) -> usize {
        let mut cursor = self.entities.reserved_slots();
        let mut retired = 0;
        while let Some(entity) = self.entities.next_active(cursor) {
            cursor = entity.slot as usize + 1;
            if self.entities.columns.owner[entity.slot as usize] != module {
                continue;
            }
            self.entities.set_never_free(entity, false);
            retired += usize::from(self.free_entity(entity, time));
        }
        retired
    }

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
                phase: ClientPhase::Active,
                entity: EntityId {
                    slot: slot as u32 + 1,
                    generation: 1,
                },
                module: ModuleId::default(),
                client_rules: RuleSetId::default(),
                input_rules: RuleSetId::default(),
                link_order: LinkOrder::Tail,
                player: PlayerState::with_capacity(items, powerups, values),
                command: UserCmd::default(),
                command_execution: CommandExecution::default(),
                command_observation: None,
                module_spawn_parms: [0; 16],
                userinfo: FixedText::default(),
                userinfo_publication: None,
                value_publication: qa_core::primitives::ValueBank::load(values),
                intent: CommandIntent::default(),
                command_pending: false,
                module_movement: false,
                server_authoritative: false,
                output: None,
                audio_retirement: None,
                hud: HudState::with_capacity(items, powerups, weapons, values),
                hud_publication: HudPublication::with_capacity(items, powerups),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        // Ring pages, HUD/module displays and one connection notice per client.
        let rows = 4096usize
            .checked_add(qa_core::events::CONSOLE_DISPLAY_RECORDS)
            .ok_or(ServerError::OutputCapacity)?
            .checked_add(
                max_clients
                    .checked_add(32)
                    .and_then(|n| n.checked_mul(6))
                    .ok_or(ServerError::OutputCapacity)?,
            )
            .and_then(|rows| rows.checked_add(max_clients))
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
            next_user_id: 0,
        })
    }

    pub fn connect(
        &mut self,
        connection: Connection,
        module: ModuleId,
        native: Option<NativeEntity>,
    ) -> Option<ClientId> {
        self.connect_with_authority(connection, module, native, true)
    }

    pub fn connect_client(&mut self) -> Option<ClientId> {
        self.connect_with_authority(Connection::Local, ModuleId::default(), None, false)
    }

    pub fn available_client_slot(&self) -> Option<ClientId> {
        self.clients
            .iter()
            .position(|client| {
                client.connection.is_none()
                    && client.audio_retirement.is_none()
                    && client.userinfo_publication.is_none_or(|publication| {
                        publication.queued_revision == Some(publication.stamp.revision)
                    })
                    && self.entities.resolve(client.entity).is_some()
            })
            .map(|slot| ClientId(slot as u32))
    }

    fn connect_with_authority(
        &mut self,
        connection: Connection,
        module: ModuleId,
        native: Option<NativeEntity>,
        server_authoritative: bool,
    ) -> Option<ClientId> {
        let slot = self.available_client_slot()?.0 as usize;
        let output = if connection == Connection::Bot {
            None
        } else {
            let id = ClientId(slot as u32);
            Some(self.events.bind(if server_authoritative {
                OutputTarget::Client(id)
            } else {
                OutputTarget::Local(id)
            })?)
        };
        let client = &mut self.clients[slot];
        client.output = output;
        client.reset(&mut self.events.texts, module);
        client.connection = Some(connection);
        client.server_authoritative = server_authoritative;
        let entity_slot = client.entity.slot as usize;
        self.entities.columns.appearance[entity_slot].snapshot_excluded = !server_authoritative;
        self.entities.columns.published_fields[entity_slot] = Default::default();
        self.entities.columns.owner[entity_slot] = module;
        // A native module supplies its own raw namespace. Q3's client zero is
        // not the common world's reserved slot zero; an unbound shell has none.
        self.entities.columns.native_entity[entity_slot] = native;
        self.entities.columns.collision_shape[entity_slot] = CollisionShape::Box;
        self.entities.columns.collision_contents[entity_slot] = Contents::BODY.0;
        self.entities.columns.collision_tags[entity_slot] = self
            .entities
            .collision_tags(client.entity, Default::default())?;
        if server_authoritative {
            self.next_user_id = self.next_user_id.wrapping_add(1);
            client.userinfo_publication = Some(UserinfoPublication {
                stamp: UserinfoStamp {
                    lifetime: client.entity,
                    user_id: self.next_user_id,
                    revision: 0,
                },
                queued_revision: None,
            });
            if connection != Connection::Bot {
                self.invalidate_userinfo_publications();
            }
        }
        Some(ClientId(slot as u32))
    }

    pub fn client_for_native(&self, native: NativeEntity) -> Option<ClientId> {
        self.clients
            .iter()
            .position(|client| {
                client.connection.is_some()
                    && client.server_authoritative
                    && self.entities.resolve(client.entity).is_some()
                    && self.entities.columns.native_entity[client.entity.slot as usize]
                        == Some(native)
            })
            .map(|slot| ClientId(slot as u32))
    }

    pub fn userinfo(&self, id: ClientId) -> Option<&[u8]> {
        let client = self.clients.get(id.0 as usize)?;
        (client.connection.is_some() && self.entities.resolve(client.entity).is_some())
            .then(|| client.userinfo.as_bytes())
    }

    /// Borrow the sole raw record, including a disconnected SERVER tombstone.
    pub fn userinfo_record(&self, id: ClientId) -> Option<UserinfoRecord<'_>> {
        let client = self.clients.get(id.0 as usize)?;
        let publication = client.userinfo_publication?;
        Some(UserinfoRecord {
            stamp: publication.stamp,
            text: client.userinfo.as_bytes(),
            queued_revision: publication.queued_revision,
            connected: client.connection.is_some(),
        })
    }

    /// A successful ring insertion owns subsequent reliable delivery and ACKs.
    pub fn mark_userinfo_queued(&mut self, id: ClientId, stamp: UserinfoStamp) -> bool {
        let Some(publication) = self
            .clients
            .get_mut(id.0 as usize)
            .and_then(|client| client.userinfo_publication.as_mut())
            .filter(|publication| publication.stamp == stamp)
        else {
            return false;
        };
        publication.queued_revision = Some(stamp.revision);
        true
    }

    /// A new recipient needs current peers independently of earlier ring ACKs.
    pub fn invalidate_userinfo_publications(&mut self) {
        for client in &mut self.clients {
            if client.is_server_active()
                && let Some(publication) = client.userinfo_publication.as_mut()
            {
                publication.queued_revision = None;
            }
        }
    }

    /// Mutate the existing native record with a boundary-validated update.
    /// The shared info writer retains native removal on capacity failure.
    pub fn update_userinfo(
        &mut self,
        id: ClientId,
        update: InfoUpdate<'_>,
    ) -> Result<(), ClientInfoError> {
        let client = self
            .clients
            .get_mut(id.0 as usize)
            .filter(|client| {
                client.connection.is_some() && self.entities.resolve(client.entity).is_some()
            })
            .ok_or(ClientInfoError::Client)?;
        let previous = client.userinfo.clone();
        let result = client.userinfo.update_info(update);
        if previous.as_bytes() != client.userinfo.as_bytes() {
            client.userinfo_changed();
        }
        result.map_err(Into::into)
    }

    /// Input admission checks the complete record before replacing its raw
    /// bytes. Exact spelling, duplicates and the optional leading slash remain.
    pub fn replace_userinfo(
        &mut self,
        id: ClientId,
        text: &[u8],
        rules: InfoRules,
    ) -> Result<(), ClientInfoError> {
        if rules.bytes == 0 || rules.bytes > USERINFO_BYTES || text.len() >= rules.bytes {
            return Err(InfoError::Capacity.into());
        }
        if text.contains(&0) {
            return Err(InfoError::Invalid.into());
        }
        if !text.is_empty() {
            let text = text.strip_prefix(b"\\").unwrap_or(text);
            let mut fields = text.split(|&byte| byte == b'\\');
            while let Some(key) = fields.next() {
                let value = fields.next().ok_or(InfoError::Invalid)?;
                InfoUpdate::new(key, value, rules)?;
            }
        }
        self.copy_userinfo(id, text)
    }

    /// Q3's native setter copies arbitrary bytes with Q_strncpyz truncation;
    /// it does not apply the input-admission validator.
    pub fn replace_userinfo_raw(
        &mut self,
        id: ClientId,
        text: &[u8],
        bytes: usize,
    ) -> Result<(), ClientInfoError> {
        if bytes == 0 || bytes > USERINFO_BYTES {
            return Err(InfoError::Capacity.into());
        }
        let text = qa_core::text::until_nul(text);
        self.copy_userinfo(id, &text[..text.len().min(bytes - 1)])
    }

    fn copy_userinfo(&mut self, id: ClientId, text: &[u8]) -> Result<(), ClientInfoError> {
        let client = self
            .clients
            .get_mut(id.0 as usize)
            .filter(|client| {
                client.connection.is_some() && self.entities.resolve(client.entity).is_some()
            })
            .ok_or(ClientInfoError::Client)?;
        if client.userinfo.as_bytes() == text {
            return Ok(());
        }
        client
            .userinfo
            .set_bytes(text)
            .map_err(|_| InfoError::Capacity)?;
        client.userinfo_changed();
        Ok(())
    }

    /// Only an admitted client with a live entity in its owning native module
    /// routes movement through ClientThink instead of the engine mover.
    pub fn module_moves(&self, id: ClientId) -> Option<NativeEntity> {
        let client = self.clients.get(id.0 as usize)?;
        if !client.is_server_active() || !client.module_movement {
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
        if !client.server_authoritative
            || client.connection.is_none()
            || self.entities.resolve(client.entity).is_none()
        {
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
        let changed = client.module != native.module
            || self.entities.columns.native_entity[slot] != Some(native);
        if changed {
            self.entities.columns.published_fields[slot] = Default::default();
            client.player.weapon_state = None;
            client.player.native = None;
            client.player.native_clock = None;
            client.player.loop_sound = None;
            client.player.prediction = Default::default();
            client.player.movement.respawned = false;
            client.player.movement.ignore_player_collision = false;
        }
        client.module = native.module;
        self.entities.columns.owner[slot] = native.module;
        self.entities.columns.native_entity[slot] = Some(native);
        if changed {
            self.invalidate_userinfo_publications();
        }
        true
    }

    /// Claim the current admitted lifetime before a synchronous Disconnect
    /// callback. Its public state and output ownership remain available to it.
    pub fn begin_close(&mut self, id: ClientId, lifetime: EntityId, now: EventTime) -> bool {
        let Some(client) = self.clients.get_mut(id.0 as usize) else {
            return false;
        };
        if !client.is_active()
            || client.entity != lifetime
            || self.entities.resolve(lifetime).is_none()
        {
            return false;
        }
        client.phase = ClientPhase::Closing(ClientClosing {
            started: now,
            final_output: None,
            acknowledged: false,
        });
        true
    }

    /// The ring owns the command and text. Retain only its delivery identity.
    pub fn finish_close(&mut self, id: ClientId, lifetime: EntityId, sequence: u64) -> bool {
        let Some(client) = self.clients.get_mut(id.0 as usize) else {
            return false;
        };
        if client.connection.is_none()
            || client.entity != lifetime
            || self.entities.resolve(lifetime).is_none()
        {
            return false;
        }
        let (ClientPhase::Closing(closing), Some(output)) = (&mut client.phase, client.output)
        else {
            return false;
        };
        if closing.final_output.is_some() {
            return false;
        }
        closing.final_output = Some((output, sequence));
        self.area.unlink(lifetime);
        true
    }

    /// Called only for a record retired by an actual native receipt.
    pub fn acknowledge_close(
        &mut self,
        id: ClientId,
        output: OutputConsumerId,
        sequence: u64,
    ) -> bool {
        let Some(client) = self.clients.get_mut(id.0 as usize) else {
            return false;
        };
        let ClientPhase::Closing(closing) = &mut client.phase else {
            return false;
        };
        if client.connection.is_none()
            || client.output != Some(output)
            || closing.final_output != Some((output, sequence))
            || closing.acknowledged
        {
            return false;
        }
        closing.acknowledged = true;
        true
    }

    pub fn disconnect(&mut self, id: ClientId) -> bool {
        let recipients = self.clients.iter().enumerate().any(|(slot, client)| {
            slot != id.0 as usize && client.is_server_active() && client.output.is_some()
        });
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
        if !client.userinfo.as_bytes().is_empty() {
            client.userinfo_changed();
        }
        let publication = client.userinfo_publication.take().map(|mut publication| {
            publication.queued_revision = (!recipients).then_some(publication.stamp.revision);
            publication
        });
        client.reset(&mut self.events.texts, ModuleId::default());
        client.userinfo_publication = publication;
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
            if self.clients[slot].connection != Some(Connection::Bot)
                || !self.clients[slot].is_active()
            {
                continue;
            }
            let module_moves = self.module_moves(ClientId(slot as u32)).is_some();
            let client = &mut self.clients[slot];
            let command = qa_input::UserCmdBuilder::build(
                duration,
                end,
                client.intent,
                policy(client.input_rules),
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

    /// Ingress admits each command once; connection history is retained separately.
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
            .filter(|client| client.is_server_active())
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
        if !client.is_server_active() || !client.command_pending {
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
        patches: qa_core::primitives::PatchClip,
        native: Option<&dyn NativeTraceEntities>,
        foreign: Option<&dyn NativeTraceEntities>,
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
            patches,
            native,
            foreign,
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
        patches: qa_core::primitives::PatchClip,
        native: Option<&dyn NativeTraceEntities>,
        foreign: Option<&dyn NativeTraceEntities>,
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
        )
        .with_patch_clip(patches)
        .with_native_entities(native)
        .with_foreign_entities(foreign);
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
        patches: qa_core::primitives::PatchClip,
        native: Option<&dyn NativeTraceEntities>,
        foreign: Option<&dyn NativeTraceEntities>,
        mut rejected: impl FnMut(ClientId, TraceError),
    ) -> u32 {
        let mut steps = 0;
        for client_index in 0..self.clients.len() {
            let id = ClientId(client_index as u32);
            match self.move_pending_client(
                id, store, geometry, index, scratch, patches, native, foreign,
            ) {
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
        if !self.entities.attachment_ids().is_empty() {
            for client in self.clients.iter_mut() {
                if client.is_active() && self.entities.attachment(client.entity).is_some() {
                    client.player.body.position =
                        self.entities.columns.position[client.entity.slot as usize];
                }
            }
        }
        result
    }
}
