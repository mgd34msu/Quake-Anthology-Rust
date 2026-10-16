//! Connection-owned configuration and derived numeric presentation bindings.
use crate::{client_scene::ClientModelBindings, model_assets::ModelCatalog};
use qa_compat::services::ClientTermination;
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{
        ClientId, EntityId, EntityRecord, GeometryId, ModelRef, ModelUse, ModuleId, NameId,
        ResourceNamespace, RuleSetId, SpatialPolicy,
    },
};
use qa_network::commands::packet::Protocol;

pub(crate) fn model_world(
    connection: Option<qa_session::clients::Connection>,
    resources: Option<&ClientResources>,
    server: Option<GeometryId>,
) -> Option<GeometryId> {
    match connection? {
        qa_session::clients::Connection::Local => resources?
            .state
            .models
            .collision_world()
            .map(|(geometry, _)| geometry),
        qa_session::clients::Connection::Remote | qa_session::clients::Connection::Bot => server,
    }
}

pub struct ClientResources {
    pub state: ResourceState,
    pub(crate) udp_lease: Option<qa_platform::UdpLease>,
    commands: qa_network::outputs::ConfigCommandReader,
    selected: Box<qa_console::command_text::ParsedCommand>,
    joined: qa_core::text::FixedText<8192>,
    pub unsupported_commands: u64,
    nq_signon: Option<NqSignon>,
    nq_spawn_parameters: qa_core::text::FixedText<{ qa_network::message::LEGACY_STRING_BYTES }>,
}
#[derive(Clone, Copy)]
pub(crate) enum NqReply {
    Prespawn,
    Name,
    Color,
    Spawn,
    Begin,
}
#[derive(Default)]
struct NqSignon {
    stage: u8,
    observed: u8,
    stage2_step: u8,
    resources_admitted: bool,
}
impl NqSignon {
    fn receive(&mut self, stage: u8) -> bool {
        if stage <= self.stage || stage > 4 {
            return false;
        }
        self.stage = stage;
        if stage < 4 {
            self.observed |= 1 << stage;
        }
        true
    }
    fn next(&self) -> Option<NqReply> {
        if self.observed & 2 != 0 {
            Some(NqReply::Prespawn)
        } else if self.observed & 4 != 0 {
            match self.stage2_step {
                0 => Some(NqReply::Name),
                1 => Some(NqReply::Color),
                _ => Some(NqReply::Spawn),
            }
        } else if self.observed & 8 != 0 {
            Some(NqReply::Begin)
        } else {
            None
        }
    }
    fn queued(&mut self, reply: NqReply) {
        match reply {
            NqReply::Prespawn => self.observed &= !2,
            NqReply::Name => self.stage2_step = 1,
            NqReply::Color => self.stage2_step = 2,
            NqReply::Spawn => self.observed &= !4,
            NqReply::Begin => self.observed &= !8,
        }
    }
}

/// Native server header retained independently of camera and physical identity.
#[derive(Clone, Debug)]
pub struct ReceivedServerInfo {
    pub max_clients: u8,
    pub game_type: u8,
    pub level_name: qa_core::text::FixedText<{ qa_network::message::LEGACY_STRING_BYTES }>,
}

#[derive(Clone, Copy)]
enum PresentationReset {
    Complete(Option<u64>),
    Required(EntityId),
    Published {
        lifetime: EntityId,
        owner: qa_core::events::OutputConsumerId,
        sequence: u64,
    },
}

pub struct ResourceState {
    pub configs: ConfigStrings,
    pub models: ClientModelBindings,
    pub layout: ConfigLayout,
    pub rejected: u64,
    server_info: Option<ReceivedServerInfo>,
    presentation_reset: PresentationReset,
    hud_pending: bool,
    client_capacity: u32,
    sounds: qa_audio::SoundBindings,
    sound_dirty: qa_core::bits::BitSet,
    sound_attached: bool,
    player_sounds: [Option<(qa_core::primitives::SoundId, u32)>;
        qa_gameplay::rules::feedback::PLAYER_SOUND_PATHS.len()],
    sound_policy: SpatialPolicy,
    loop_spatial: SpatialPolicy,
    static_sounds: u32,
    static_entities: Box<[crate::client_scene::StaticEntity]>,
    static_entity_count: usize,
    static_dirty: bool,
}
impl ClientResources {
    pub fn load(
        client: ClientId,
        protocol: Protocol,
        sound_rules: RuleSetId,
    ) -> Result<Option<Self>, String> {
        let layout = protocol.resource_layout();
        Ok(Some(Self {
            udp_lease: None,
            commands: qa_network::outputs::ConfigCommandReader::load(),
            selected: Box::default(),
            joined: qa_core::text::FixedText::default(),
            unsupported_commands: 0,
            nq_signon: None,
            nq_spawn_parameters: Default::default(),
            state: ResourceState {
                configs: ConfigStrings::load(
                    (0..layout.count).map(|index| layout.capacity(index).unwrap_or(0)),
                )
                .map_err(|error| format!("client configuration: {error:?}"))?,
                models: ClientModelBindings::load(client, layout),
                layout,
                rejected: 0,
                server_info: None,
                presentation_reset: PresentationReset::Complete(None),
                hud_pending: true,
                client_capacity: qa_network::snapshots::snapshot_limits(protocol).client_capacity,
                sounds: qa_audio::SoundBindings::load(layout.resources[1].count as usize),
                sound_dirty: qa_core::bits::BitSet::empty(layout.resources[1].count as usize),
                sound_attached: false,
                player_sounds: [None; qa_gameplay::rules::feedback::PLAYER_SOUND_PATHS.len()],
                sound_policy: qa_gameplay::rules::sound_policy(sound_rules),
                loop_spatial: qa_gameplay::rules::loop_spatial_policy(sound_rules),
                static_sounds: 0,
                static_entities: vec![
                    crate::client_scene::StaticEntity::default();
                    qa_core::primitives::NATIVE_WIRE_BYTES
                        / qa_network::outputs::STATIC_ENTITY_BYTES
                ]
                .into_boxed_slice(),
                static_entity_count: 0,
                static_dirty: false,
            },
        }))
    }

    /// Connect/map parameters are native CLIENT text, independent of SERVER
    /// QuakeC spawn globals. Reject text the actual wire would truncate.
    pub fn set_nq_spawn_parameters(&mut self, parameters: &[u8]) -> Result<(), &'static str> {
        if self.state.layout.rules != RuleSetId::Quake {
            return Err("spawn parameters require NetQuake");
        }
        let mut command =
            qa_core::text::FixedText::<{ qa_network::message::LEGACY_STRING_BYTES }>::default();
        command
            .set_bytes(b"spawn ")
            .map_err(|_| "spawn command capacity")?;
        command
            .append_bytes(parameters)
            .map_err(|_| "spawn command capacity")?;
        if command
            .as_bytes()
            .iter()
            .any(|&byte| byte == 0 || byte == 255)
            || Protocol::NetQuake15
                .client_string_policy()
                .truncates(command.as_bytes())
        {
            return Err("spawn parameters exceed native string policy");
        }
        self.nq_spawn_parameters
            .set_bytes(parameters)
            .map_err(|_| "spawn parameter capacity")
    }
    pub fn nq_signon_stage(&self) -> Option<u8> {
        self.nq_signon.as_ref().map(|signon| signon.stage)
    }
    pub fn udp_socket(&self) -> Option<u16> {
        self.udp_lease.as_ref().map(qa_platform::UdpLease::socket)
    }
    pub(crate) fn begin_nq_signon(&mut self) {
        self.nq_signon = Some(NqSignon::default());
    }
    pub(crate) fn nq_assignment_max_clients(&self) -> Option<u8> {
        (self.nq_signon.as_ref()?.stage == 0)
            .then_some(self.state.server_info.as_ref()?.max_clients)
    }
    pub(crate) fn awaiting_nq_signon(&self) -> bool {
        self.nq_signon
            .as_ref()
            .is_some_and(|signon| signon.stage < 4 || !signon.resources_admitted)
    }
    pub(crate) fn nq_resources_needed(&self) -> bool {
        self.nq_signon
            .as_ref()
            .is_some_and(|signon| !signon.resources_admitted)
    }
    pub(crate) fn pending_nq_world(&self) -> Option<qa_core::configstrings::ConfigRef<'_>> {
        self.state.server_info.as_ref()?;
        self.state.models.pending_world(&self.state.configs)
    }
    pub(crate) fn nq_resources_admitted(&mut self) {
        if let Some(signon) = &mut self.nq_signon {
            signon.resources_admitted = true;
        }
    }

    pub(crate) fn receive_nq_stage(&mut self, stage: u8) -> bool {
        self.nq_signon
            .get_or_insert_with(NqSignon::default)
            .receive(stage)
    }
    pub(crate) fn receive_nq_entity(&mut self) {
        if let Some(signon) = &mut self.nq_signon
            && signon.stage == 3
        {
            signon.stage = 4;
        }
    }
    pub(crate) fn nq_reply(&self) -> Option<NqReply> {
        self.nq_signon.as_ref()?.next()
    }
    pub(crate) fn nq_spawn_parameters(&self) -> &[u8] {
        self.nq_spawn_parameters.as_bytes()
    }
    pub(crate) fn nq_reply_queued(&mut self, reply: NqReply) {
        if let Some(signon) = &mut self.nq_signon {
            signon.queued(reply);
        }
    }

    pub fn apply(
        &mut self,
        index: usize,
        text: &[u8],
        catalog: Option<&ModelCatalog>,
        bank: Option<&qa_audio::Bank>,
    ) -> bool {
        self.state.apply(index, text, catalog, bank)
    }

    pub(crate) fn apply_gamestate(
        &mut self,
        gamestate: &qa_network::channel::commands::Q3GameState<'_>,
        catalog: Option<&ModelCatalog>,
        bank: Option<&qa_audio::Bank>,
    ) {
        // Both native readers validate the complete replacement before entering
        // this shared CLIENT reset and registration boundary.
        let world = self.state.models.world_binding();
        self.reset_received(world);
        let result = gamestate.for_each_config(|config| self.store_received_config(config));
        self.complete_received(result.is_err(), catalog, bank);
    }

    pub(crate) fn apply_server_info(
        &mut self,
        info: &qa_network::commands::startup::NqServerInfo<'_>,
        world: Option<(
            qa_compat::services::LoadedWorldBinding,
            qa_render::world::WorldPresentationId,
        )>,
        catalog: Option<&ModelCatalog>,
        bank: Option<&qa_audio::Bank>,
    ) {
        self.reset_received(world);
        let mut level_name = qa_core::text::FixedText::default();
        // The checked native string policy admits at most this buffer's size.
        let _ = level_name.set_bytes(info.level_name);
        self.begin_nq_signon();
        self.state.server_info = Some(ReceivedServerInfo {
            max_clients: info.max_clients,
            game_type: info.game_type,
            level_name,
        });
        let result = info.for_each_config(|config| self.store_received_config(config));
        self.complete_received(result.is_err(), catalog, bank);
    }

    fn reset_received(
        &mut self,
        world: Option<(
            qa_compat::services::LoadedWorldBinding,
            qa_render::world::WorldPresentationId,
        )>,
    ) {
        self.state.configs.reset();
        self.commands.reset();
        self.selected.clear();
        self.joined.clear();
        self.state.server_info = None;
        self.state.hud_pending = true;
        self.nq_signon = None;
        self.state.models.reset_received(world);
        self.state.sounds.clear();
        self.state.sound_dirty.clear();
        self.state.static_sounds = 0;
        for entity in &mut self.state.static_entities[..self.state.static_entity_count] {
            *entity = Default::default();
        }
        self.state.static_entity_count = 0;
        self.state.static_dirty = false;
    }

    fn store_received_config(&mut self, config: qa_network::outputs::ConfigString<'_>) {
        self.state.rejected += u64::from(
            self.state
                .configs
                .set(config.index as usize, config.text)
                .is_err(),
        );
    }

    fn complete_received(
        &mut self,
        invalid: bool,
        catalog: Option<&ModelCatalog>,
        bank: Option<&qa_audio::Bank>,
    ) {
        self.state.rejected += u64::from(invalid);
        self.state.models.configs_replaced();
        if let Some(catalog) = catalog {
            self.state.rejected +=
                u64::from(self.state.models.refresh_all(&self.state.configs, catalog));
        }
        for ordinal in 1..self.state.layout.resources[1].count as usize {
            self.state.sound_dirty.insert(ordinal);
        }
        self.state.rejected += u64::from(!self.state.refresh_sounds(bank));
    }
}

/// The retained channel and current argv storage are borrowed only for one
/// synchronous cgame call or the identical engine configuration consumer.
pub(crate) struct ClientConnectionView<'a> {
    client: ClientId,
    lifetime: EntityId,
    channel: &'a mut qa_network::channel::Channel,
    commands: &'a mut qa_network::commands::connection::Commands,
    resources: &'a mut ClientResources,
    catalog: Option<&'a ModelCatalog>,
    pub(crate) termination: Option<ClientTermination>,
}
pub(crate) fn connection_view<'a>(
    client: ClientId,
    lifetime: EntityId,
    network: &'a mut qa_network::ingress::Connections,
    resources: &'a mut [Option<ClientResources>],
    catalog: Option<&'a ModelCatalog>,
) -> Option<ClientConnectionView<'a>> {
    let connection = network.get_mut(client, qa_core::loopback::Endpoint::Client)?;
    if !connection.phase.is_connected() {
        return None;
    }
    let channel = &mut connection.channel;
    let commands = connection.commands.as_mut()?;
    let resources = resources.get_mut(client.0 as usize)?.as_mut()?;
    Some(ClientConnectionView::new(
        client, lifetime, channel, commands, resources, catalog,
    ))
}
impl<'a> ClientConnectionView<'a> {
    pub(crate) fn new(
        client: ClientId,
        lifetime: EntityId,
        channel: &'a mut qa_network::channel::Channel,
        commands: &'a mut qa_network::commands::connection::Commands,
        resources: &'a mut ClientResources,
        catalog: Option<&'a ModelCatalog>,
    ) -> Self {
        Self {
            client,
            lifetime,
            channel,
            commands,
            resources,
            catalog,
            termination: None,
        }
    }
    fn reject<T>(
        &mut self,
        error: qa_compat::services::CallError,
    ) -> Result<T, qa_compat::services::CallError> {
        self.resources.state.rejected += 1;
        self.termination = Some(ClientTermination::Rejected);
        Err(error)
    }
    fn validate(
        &self,
        server: &qa_session::clients::Server,
    ) -> Result<(), qa_compat::services::CallError> {
        let client = server
            .clients
            .get(self.client.0 as usize)
            .ok_or(qa_compat::services::CallError::Entity)?;
        if client.connection.is_none()
            || client.entity != self.lifetime
            || server.entities.resolve(self.lifetime).is_none()
        {
            return Err(qa_compat::services::CallError::Entity);
        }
        Ok(())
    }
    pub(crate) fn consume_configuration(
        &mut self,
        server: &mut qa_session::clients::Server,
        team_chats_only: bool,
        bank: Option<&qa_audio::Bank>,
    ) -> Result<(), qa_compat::services::CallError> {
        use qa_compat::services::{CallError, ClientConnection};
        let received = self
            .channel
            .command_state()
            .ok_or(CallError::Text)?
            .received;
        let executed = self
            .channel
            .last_executed_server_command()
            .map_err(|_| CallError::Text)?;
        for number in executed + 1..=received {
            if !self.select(number as i32, server, bank)? {
                continue;
            }
            let arguments = self.resources.selected.arguments();
            let kind = match arguments.get_bytes(0) {
                b"cs" | b"map_restart" => continue,
                b"print" => qa_core::primitives::PrintKind::Console,
                b"cp" => qa_core::primitives::PrintKind::Center,
                b"chat" if team_chats_only => continue,
                b"chat" | b"tchat" => qa_core::primitives::PrintKind::Chat,
                _ => {
                    self.resources.unsupported_commands += 1;
                    continue;
                }
            };
            let text = if kind == qa_core::primitives::PrintKind::Chat {
                // Original MAX_SAY_TEXT includes its terminator. Truncate
                // before removing the native chat escape byte.
                let raw = arguments.get_bytes(1);
                self.resources.joined.clear();
                for &byte in &raw[..raw.len().min(149)] {
                    if byte != 0x19 {
                        self.resources
                            .joined
                            .append_bytes(&[byte])
                            .map_err(|_| CallError::Text)?;
                    }
                }
                self.resources
                    .joined
                    .append_bytes(b"\n")
                    .map_err(|_| CallError::Text)?;
                self.resources.joined.as_bytes()
            } else {
                arguments.get_bytes(1)
            };
            // Received CLIENT text cannot become another SERVER publication.
            // EventRing owns admission failures; the native command executes
            // once even when the bounded display consumer cannot accept it.
            let _ = server.events.print_level_bytes(
                Some(qa_core::events::OutputTarget::Local(self.client)),
                Some(self.client),
                kind,
                kind.default_level(),
                text,
            );
        }
        Ok(())
    }
}
impl qa_compat::services::ClientConnection for ClientConnectionView<'_> {
    fn identity(
        &self,
        server: &qa_session::clients::Server,
    ) -> Result<(ClientId, EntityId), qa_compat::services::CallError> {
        self.validate(server)?;
        Ok((self.client, self.lifetime))
    }
    fn command_number(
        &self,
        server: &qa_session::clients::Server,
    ) -> Result<u64, qa_compat::services::CallError> {
        self.validate(server)?;
        self.commands
            .history()
            .map(|history| history.number())
            .ok_or(qa_compat::services::CallError::Unsupported)
    }
    fn usercmd(
        &mut self,
        number: i32,
        server: &qa_session::clients::Server,
    ) -> Result<Option<qa_core::primitives::UserCmd>, qa_compat::services::CallError> {
        use qa_compat::services::CallError;
        self.validate(server)?;
        let history = self.commands.history().ok_or(CallError::Unsupported)?;
        let age = (history.number() as i32).wrapping_sub(number);
        if age < 0 {
            return self.reject(CallError::Text);
        }
        let age = age as usize;
        if age >= history.capacity() {
            return Ok(None);
        }
        // Native startup/map_restart exposes zeroed slots in the retained
        // window, even when no command has been produced for that slot.
        Ok(Some(history.get(age).copied().unwrap_or_default()))
    }
    fn disconnected(&self) -> Option<ClientTermination> {
        self.termination
    }

    fn arguments(&self) -> qa_console::command_text::Arguments<'_> {
        self.resources.selected.arguments()
    }
    fn configstrings(
        &self,
        server: &qa_session::clients::Server,
    ) -> Result<&ConfigStrings, qa_compat::services::CallError> {
        use qa_compat::services::CallError;
        self.validate(server)?;
        if self.resources.state.layout.rules != RuleSetId::Quake3
            || (self.channel.q3_startup().is_none() && self.channel.q3_snapshot_number().is_none())
        {
            return Err(CallError::Unsupported);
        }
        Ok(&self.resources.state.configs)
    }
    fn snapshot_number(
        &self,
        server: &qa_session::clients::Server,
    ) -> Result<(u32, qa_core::primitives::ThinkTime), qa_compat::services::CallError> {
        self.validate(server)?;
        self.channel
            .q3_snapshot_number()
            .ok_or(qa_compat::services::CallError::Unsupported)
    }
    fn snapshot(
        &mut self,
        sequence: i32,
        server: &qa_session::clients::Server,
    ) -> Result<Option<qa_network::snapshots::Q3ClientSnapshot<'_>>, qa_compat::services::CallError>
    {
        self.validate(server)?;
        match self.channel.q3_client_snapshot(sequence) {
            Ok(snapshot) => Ok(snapshot),
            Err(_) => {
                self.resources.state.rejected += 1;
                self.termination = Some(ClientTermination::Rejected);
                Err(qa_compat::services::CallError::Memory)
            }
        }
    }
    fn select(
        &mut self,
        number: i32,
        server: &mut qa_session::clients::Server,
        bank: Option<&qa_audio::Bank>,
    ) -> Result<bool, qa_compat::services::CallError> {
        use qa_compat::services::CallError;
        use qa_network::outputs::ConfigCommandStep;
        self.validate(server)?;
        let Ok(sequence) = u32::try_from(number) else {
            return self.reject(CallError::Text);
        };
        let text = match self.channel.server_command(sequence) {
            Ok(text) => text,
            Err(_) => return self.reject(CallError::Text),
        };
        let parsed = self.resources.selected.parse(text, RuleSetId::Quake3);
        // Native execution precedes preprocessing, including bcs0/bcs1.
        if self.channel.executed_server_command(sequence).is_err() || parsed.is_err() {
            return self.reject(CallError::Text);
        }
        loop {
            let arguments = self.resources.selected.arguments();
            let step = self.resources.commands.read(
                arguments.get_bytes(0),
                arguments.get_bytes(1),
                arguments.get_bytes(2),
            );
            match step {
                Ok(ConfigCommandStep::Pending) => return Ok(false),
                Ok(ConfigCommandStep::Rescan(text)) => {
                    if self
                        .resources
                        .selected
                        .parse(text, RuleSetId::Quake3)
                        .is_err()
                    {
                        return self.reject(CallError::Text);
                    }
                    continue;
                }
                Ok(ConfigCommandStep::Pass) => {}
                Err(_) => return self.reject(CallError::ConfigString),
            }
            let arguments = self.resources.selected.arguments();
            match arguments.get_bytes(0) {
                b"disconnect" => {
                    self.termination = Some(ClientTermination::ServerDisconnect);
                    return Err(CallError::Aborted);
                }
                b"cs" => {
                    let (index, _) = qa_core::text::integer_prefix(arguments.get_bytes(1));
                    let Ok(index) = usize::try_from(index) else {
                        return self.reject(CallError::ConfigString);
                    };
                    if index >= self.resources.state.layout.count
                        || arguments.join_bytes(2, &mut self.resources.joined).is_err()
                    {
                        return self.reject(CallError::ConfigString);
                    }
                    if !self.resources.state.apply(
                        index,
                        self.resources.joined.as_bytes(),
                        self.catalog,
                        bank,
                    ) {
                        self.termination = Some(ClientTermination::Rejected);
                        return Err(CallError::ConfigString);
                    }
                    if index == ConfigLayout::Q3_SYSTEM_INFO {
                        let info = self.resources.joined.as_bytes();
                        let mut context = self
                            .channel
                            .command_context()
                            .map_err(|_| CallError::Text)?;
                        context.set_system_info(info);
                        self.channel
                            .set_command_context(context)
                            .map_err(|_| CallError::Text)?;
                    }
                }
                b"map_restart" => {
                    server.clients[self.client.0 as usize]
                        .hud
                        .clear_notify(&mut server.events.texts);
                    self.commands.reset_history(true);
                }
                b"clientLevelShot" => {
                    // Screenshot generation has no shared CLIENT service yet.
                    self.resources.unsupported_commands += 1;
                    return Ok(false);
                }
                _ => {}
            }
            return Ok(true);
        }
    }
}
impl ResourceState {
    pub(crate) fn hud_pending(&self) -> bool {
        self.hud_pending
    }

    pub(crate) fn take_hud_attempt(&mut self) {
        self.hud_pending = false;
    }

    pub(crate) fn request_presentation_reset(&mut self, lifetime: EntityId) {
        self.presentation_reset = PresentationReset::Required(lifetime);
    }

    pub(crate) fn publish_presentation_reset(
        &mut self,
        client: ClientId,
        current_lifetime: EntityId,
        events: &mut qa_core::events::EventRing,
        owner: &mut qa_core::events::OutputConsumerId,
    ) -> bool {
        let lifetime = match self.presentation_reset {
            PresentationReset::Complete(_) => return true,
            PresentationReset::Required(lifetime) => lifetime,
            PresentationReset::Published {
                lifetime,
                owner,
                sequence,
            } => {
                if events.retained_for(owner, sequence) {
                    return true;
                }
                lifetime
            }
        };
        if lifetime != current_lifetime {
            self.presentation_reset = PresentationReset::Complete(None);
            return false;
        }
        self.presentation_reset = PresentationReset::Required(lifetime);
        for _ in 0..2 {
            if !events.resume_if_needed(owner) {
                break;
            }
            let Ok(sequence) = events.push_to(
                qa_core::events::OutputTarget::Presentation,
                qa_core::events::FrameEvent::ResetPresentation { client, lifetime },
            ) else {
                break;
            };
            if events.retained_for(*owner, sequence) {
                self.presentation_reset = PresentationReset::Published {
                    lifetime,
                    owner: *owner,
                    sequence,
                };
                return true;
            }
        }
        false
    }

    pub(crate) fn complete_presentation_reset(
        &mut self,
        lifetime: EntityId,
        owner: qa_core::events::OutputConsumerId,
        sequence: u64,
    ) -> bool {
        if matches!(self.presentation_reset,
            PresentationReset::Published { lifetime: pending, owner: recipient, sequence: published }
                if pending == lifetime && recipient == owner && published == sequence)
        {
            self.presentation_reset = PresentationReset::Complete(Some(sequence));
            true
        } else {
            false
        }
    }

    pub(crate) fn presentation_reset_pending(&self) -> bool {
        !matches!(self.presentation_reset, PresentationReset::Complete(_))
    }

    pub(crate) fn presentation_after_reset(&self, sequence: u64) -> bool {
        match self.presentation_reset {
            PresentationReset::Complete(reset) => reset.is_none_or(|reset| sequence > reset),
            PresentationReset::Required(_) | PresentationReset::Published { .. } => false,
        }
    }

    /// Accepted CLIENT configuration supplies the native player-number range.
    /// Missing, zero or invalid counts never classify ordinary bodies as players.
    pub fn server_info(&self) -> Option<&ReceivedServerInfo> {
        self.server_info.as_ref()
    }

    pub fn max_clients(&self) -> u32 {
        if let Some(info) = &self.server_info {
            return u32::from(info.max_clients);
        }
        self.layout
            .max_clients()
            .and_then(|index| self.configs.config(index))
            .and_then(|row| u32::try_from(row.integer).ok())
            .filter(|&count| count <= self.client_capacity)
            .unwrap_or(0)
    }

    /// Native signon copies survive freeing or reusing their source edicts.
    pub fn receive_static_entity(
        &mut self,
        mut record: EntityRecord,
        lookup: Option<&dyn qa_compat::services::ModelLookup>,
        world: Option<(
            qa_render::world::WorldId,
            qa_render::world::WorldPresentationId,
        )>,
    ) -> bool {
        let Some(slot) = self.static_entities.get_mut(self.static_entity_count) else {
            self.rejected += 1;
            return false;
        };
        for reference in &mut record.appearance.models {
            *reference = reference.and_then(|model| {
                ModelRef::native(
                    ResourceNamespace::Client(self.models.client()),
                    model.ordinal,
                )
            });
        }
        *slot = self
            .models
            .capture_static(record, &self.configs, lookup, world);
        self.rejected += u64::from(slot.rejected);
        self.static_entity_count += 1;
        self.static_dirty = true;
        true
    }
    pub fn static_entities(&self) -> &[crate::client_scene::StaticEntity] {
        &self.static_entities[..self.static_entity_count]
    }
    pub(crate) fn complete_static_models(&mut self, catalog: &ModelCatalog) {
        for entity in &mut self.static_entities[..self.static_entity_count] {
            self.models.complete_static(entity, catalog);
        }
    }

    pub(crate) fn prepare_models(
        &mut self,
        owner: &mut crate::model_assets::ModelAssets,
        vfs: &qa_content::vfs::Vfs,
        preparer: &mut dyn crate::renderer::AssetPreparer,
    ) -> (u32, bool) {
        let (rejected, published) =
            self.models
                .prepare_pending(&self.configs, owner, vfs, preparer);
        if self.static_dirty
            && let Some((static_rejected, static_published)) = self.models.prepare_static(
                &mut self.static_entities[..self.static_entity_count],
                owner,
                vfs,
                preparer,
            )
        {
            self.static_dirty = false;
            return (rejected + static_rejected, published || static_published);
        }
        (rejected, published)
    }

    pub fn static_entity_capacity(&self) -> usize {
        self.static_entities.len()
    }

    pub(crate) fn static_emitter(
        &mut self,
        client: ClientId,
        lifetime: qa_core::primitives::EntityId,
    ) -> Option<qa_core::primitives::SoundEmitter> {
        let slot = self.static_sounds;
        self.static_sounds = self.static_sounds.checked_add(1)?;
        Some(qa_core::primitives::SoundEmitter::Static {
            namespace: ResourceNamespace::Client(client),
            lifetime,
            slot,
        })
    }
    pub fn sound_policy(&self) -> SpatialPolicy {
        self.sound_policy
    }
    /// Ordinary and entity-loop presentation are selected at connection/profile
    /// load, independently of the legacy protocol's resource and message layout.
    pub fn set_sound_policy(&mut self, policy: SpatialPolicy, entity_loop: SpatialPolicy) {
        self.sound_policy = policy;
        self.loop_spatial = entity_loop;
    }
    pub fn loop_spatial_policy(&self) -> SpatialPolicy {
        self.loop_spatial
    }
    pub fn sound(&self, ordinal: u32) -> Option<qa_core::primitives::SoundId> {
        self.sounds.sound(ordinal)
    }
    /// Fixed media retains the Bank's native engine handle for native channel
    /// suppression; received config ordinals keep their existing wire domain.
    pub(crate) fn player_sound(
        &self,
        binding: qa_gameplay::rules::feedback::PlayerSoundBinding,
    ) -> Option<(qa_core::primitives::SoundId, u32)> {
        use qa_gameplay::rules::feedback::PlayerSoundBinding;
        match binding {
            PlayerSoundBinding::Config(ordinal) => Some((self.sound(ordinal)?, ordinal)),
            PlayerSoundBinding::Fixed(index) => {
                self.player_sounds.get(index as usize).copied().flatten()
            }
        }
    }
    /// Cold bank attachment reserves numeric inverse rows before packet intake.
    pub fn bind_sounds(&mut self, bank: &qa_audio::Bank) -> bool {
        if !self.sound_attached {
            self.sounds.attach(bank);
            self.sound_attached = true;
        }
        for (binding, path) in self
            .player_sounds
            .iter_mut()
            .zip(qa_gameplay::rules::feedback::PLAYER_SOUND_PATHS)
        {
            *binding = bank.registered_handle(path.as_bytes()).and_then(|handle| {
                Some((bank.registered_sound(handle)?, u32::try_from(handle).ok()?))
            });
        }
        self.refresh_sounds(Some(bank))
    }

    /// Accepted config changes are prepared before the packet's following cues.
    /// Failed rows complete once; emission only reads their numeric binding.
    pub(crate) fn prepare_sounds(
        &mut self,
        assets: &mut crate::audio::SoundAssets,
        vfs: &qa_content::vfs::Vfs,
    ) {
        use qa_compat::services::SoundRegistrar;
        if !self.sound_attached {
            self.sounds.attach(assets.bank());
            self.sound_attached = true;
        }
        while let Some(ordinal) = self.sound_dirty.next_set(1) {
            self.sound_dirty.remove(ordinal);
            let index = self.layout.resources[1].first as usize + ordinal;
            let Some(config) = self.configs.config(index) else {
                continue;
            };
            let sound = if config.text.is_empty() {
                Ok(None)
            } else {
                assets.precache(vfs, config.text, self.layout.rules)
            };
            let admitted = sound.as_ref().ok().copied().flatten();
            self.rejected += u64::from(sound.is_err());
            self.rejected += u64::from(self.sounds.set(ordinal as u32, admitted).is_err());
        }
    }

    fn refresh_sounds(&mut self, bank: Option<&qa_audio::Bank>) -> bool {
        let mut complete = true;
        for (ordinal, index) in self.layout.resources[1].range().enumerate() {
            if !self.sound_dirty.contains(ordinal) {
                // Completed registrar decisions include source-rejected media.
                // A canonical sample usable by another source cannot revive it.
                continue;
            }
            let Some(config) = self.configs.config(index) else {
                complete = false;
                continue;
            };
            if self
                .sounds
                .bind_name(bank, ordinal as u32, config.text, self.layout.rules)
                .is_err()
            {
                complete = false;
            }
        }
        complete
    }
    fn apply(
        &mut self,
        index: usize,
        text: &[u8],
        catalog: Option<&ModelCatalog>,
        bank: Option<&qa_audio::Bank>,
    ) -> bool {
        if self.layout.rules == RuleSetId::Quake3
            && self
                .configs
                .config(index)
                .is_some_and(|row| row.text != text)
            && qa_compat::configstrings::q3_game_state_data_count(
                &self.configs,
                Some((index, text)),
            )
            .is_err()
        {
            self.rejected += 1;
            return false;
        }
        match self.configs.set(index, text) {
            Ok(changed) => {
                if changed {
                    self.models.config_received(index);
                }
                if changed && let Some(catalog) = catalog {
                    self.rejected +=
                        u64::from(!self.models.config_changed(index, &self.configs, catalog));
                }
                if changed && self.layout.resources[1].contains(index) {
                    let ordinal = index - self.layout.resources[1].first as usize;
                    if ordinal != 0 {
                        self.sound_dirty.insert(ordinal);
                    }
                    self.rejected += u64::from(
                        self.sounds
                            .bind_name(bank, ordinal as u32, text, self.layout.rules)
                            .is_err(),
                    );
                }
                true
            }
            Err(_) => {
                self.rejected += 1;
                false
            }
        }
    }
}

/// Producer-owned configuration readiness and the destination sound namespace.
/// CLIENT state above contains only received rows; it is never read to project
/// a SERVER sound. All names still belong to source storage or the PCM bank.
pub struct ConfigPublication {
    module: Option<ModuleId>,
    layout: Option<ConfigLayout>,
    revisions: Box<[Option<u64>]>,
    seen: Box<[u64]>,
    rows: Box<[PublishedConfig]>,
    sounds: qa_audio::SoundBindings,
    models: qa_core::bindings::OrdinalBindings<NameId>,
    model_claims: Box<[bool]>,
    model_world: Option<GeometryId>,
    primary_claims: Box<[bool]>,
    pub unsupported: u64,
    pub pending: Option<(u64, usize)>,
    game_state: Option<(u64, bool)>,
    game_state_required: bool,
    last_client_limit: Option<u32>,
}
#[derive(Clone, Copy, Default)]
struct PublishedConfig {
    sequence: Option<u64>,
    acknowledged: bool,
    unsupported: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceReadiness {
    Pending,
    Ready,
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelName {
    pub name: Option<NameId>,
    pub revision: u64,
    pub source: RuleSetId,
}
use qa_gameplay::rules::appearance::{ModelInput, model_input};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelProjection {
    Pending,
    Ready(u32),
    Unsupported,
}
impl ConfigPublication {
    pub fn load() -> Self {
        let layout = ConfigLayout::for_rules(RuleSetId::Quake2Rerelease);
        let count = layout.count;
        let sounds = layout.resources[1].count as usize;
        Self {
            module: None,
            layout: None,
            revisions: Box::new([]),
            seen: vec![0; count].into_boxed_slice(),
            rows: vec![PublishedConfig::default(); count].into_boxed_slice(),
            sounds: qa_audio::SoundBindings::load(sounds),
            models: qa_core::bindings::OrdinalBindings::load(
                layout.resources[0].count as usize,
                0,
                0,
                false,
            ),
            model_claims: vec![false; layout.resources[0].count as usize].into_boxed_slice(),
            model_world: None,
            primary_claims: vec![false; sounds].into_boxed_slice(),
            unsupported: 0,
            pending: None,
            game_state: None,
            game_state_required: false,
            last_client_limit: None,
        }
    }
    /// Source capacities are fixed when the module set is loaded.
    pub fn configure_sources(&mut self, capacity: usize, model_names: usize) {
        self.revisions = vec![None; capacity].into_boxed_slice();
        self.models.configure(model_names, 0);
        self.reset();
    }
    pub fn bind_sounds(&mut self, bank: &qa_audio::Bank) {
        self.sounds.attach(bank);
        self.revisions.fill(None);
        self.seen.fill(0);
        self.rows.fill(PublishedConfig::default());
        self.primary_claims.fill(false);
        self.last_client_limit = None;
    }
    pub fn reset(&mut self) {
        self.module = None;
        self.layout = None;
        self.revisions.fill(None);
        self.seen.fill(0);
        self.rows.fill(PublishedConfig::default());
        self.sounds.clear();
        self.models.clear();
        self.model_claims.fill(false);
        self.model_world = None;
        self.primary_claims.fill(false);
        self.pending = None;
        self.game_state = None;
        self.game_state_required = false;
        self.last_client_limit = None;
    }
    pub fn connection(&mut self, module: ModuleId, layout: ConfigLayout) {
        if self.module != Some(module) || self.layout.map(|old| old.rules) != Some(layout.rules) {
            self.reset();
            self.module = Some(module);
            self.layout = Some(layout);
        }
    }
    pub fn begin(&self, module: ModuleId, revision: u64) -> bool {
        self.revisions
            .get(module.0 as usize)
            .is_some_and(|seen| *seen != Some(revision))
    }
    pub fn needs(&self, index: usize, revision: u64) -> bool {
        self.seen.get(index).is_some_and(|seen| *seen != revision)
    }
    pub fn seen(&mut self, index: usize, revision: u64) {
        if let Some(seen) = self.seen.get_mut(index) {
            *seen = revision;
        }
    }
    pub fn published(&mut self, destination: usize, sequence: Option<u64>) {
        if let Some(seen) = self.rows.get_mut(destination) {
            *seen = PublishedConfig {
                sequence,
                acknowledged: false,
                unsupported: sequence.is_none(),
            };
        }
    }
    pub(crate) fn needs_client_limit(&self, count: u32) -> bool {
        self.last_client_limit != Some(count)
    }
    pub(crate) fn published_client_limit(&mut self, index: usize, count: u32, sequence: u64) {
        self.published(index, Some(sequence));
        self.last_client_limit = Some(count);
    }
    /// The ring supplies the proved ACK and the immutable destination row.
    /// An old row's receipt cannot ready its later rewrite or relocation.
    pub fn unpublished(&self, destination: usize) -> bool {
        self.rows
            .get(destination)
            .is_some_and(|row| row.sequence.is_none() && !row.unsupported)
    }
    pub fn acknowledged(&mut self, destination: usize, sequence: u64) {
        if let Some(row) = self.rows.get_mut(destination)
            && row.sequence == Some(sequence)
        {
            row.acknowledged = true;
        }
    }
    pub(crate) fn game_state_published(&self) -> bool {
        self.game_state.is_some()
    }
    pub(crate) fn require_game_state(&mut self) {
        self.game_state_required = true;
    }
    pub(crate) fn publish_game_state(&mut self, sequence: u64) {
        self.game_state = Some((sequence, false));
    }
    pub(crate) fn game_state_waiting(&self) -> bool {
        self.game_state_required
            && !self
                .game_state
                .is_some_and(|(_, acknowledged)| acknowledged)
    }
    pub(crate) fn acknowledge_game_state(&mut self, sequence: u64) {
        if let Some((published, acknowledged)) = &mut self.game_state
            && *published == sequence
        {
            *acknowledged = true;
            for row in &mut self.rows {
                if row.sequence == Some(sequence) {
                    row.acknowledged = true;
                }
            }
        }
    }
    pub fn readiness(&self, destination: usize) -> ResourceReadiness {
        let Some(row) = self.rows.get(destination) else {
            return ResourceReadiness::Unsupported;
        };
        if row.acknowledged {
            ResourceReadiness::Ready
        } else if row.unsupported {
            ResourceReadiness::Unsupported
        } else {
            ResourceReadiness::Pending
        }
    }
    pub fn finish(&mut self, module: ModuleId, revision: u64) {
        if let Some(seen) = self.revisions.get_mut(module.0 as usize) {
            *seen = Some(revision);
        }
    }
    /// Actual nonempty primary registrations own their native ordinals. A new
    /// primary claim may displace a secondary sample; its numeric SoundId can
    /// be registered again in a free slot without aliasing the new primary.
    pub fn primary_sound(
        &mut self,
        ordinal: u32,
        claimed: bool,
        sound: Option<qa_core::primitives::SoundId>,
    ) {
        let Some(layout) = self.layout else {
            return;
        };
        if ordinal >= layout.resources[1].count {
            return;
        }
        let previous = self.primary_claims[ordinal as usize];
        self.primary_claims[ordinal as usize] = claimed;
        if claimed || previous {
            if self.sounds.sound(ordinal) != sound {
                self.rows[layout.resources[1].first as usize + ordinal as usize] =
                    PublishedConfig::default();
            }
            let _ = self.sounds.set(ordinal, sound);
        }
    }
    /// Free duplicate secondary rows after a native primary claim, so growing
    /// primary precaches do not consume two slots for the same loaded sample.
    pub fn release_aliases(&mut self) {
        let Some(layout) = self.layout else {
            return;
        };
        // Lowest actual primary registration wins, independently of a lower
        // secondary slot that happened to be occupied before primary growth.
        for ordinal in (1..layout.resources[1].count).rev() {
            if self.primary_claims[ordinal as usize] {
                self.sounds.prefer(ordinal);
            }
        }
        for ordinal in 1..layout.resources[1].count {
            if !self.primary_claims[ordinal as usize]
                && let Some(sound) = self.sounds.sound(ordinal)
                && self.sounds.ordinal(sound) != Some(ordinal)
            {
                let _ = self.sounds.set(ordinal, None);
                self.rows[layout.resources[1].first as usize + ordinal as usize] =
                    PublishedConfig::default();
            }
        }
    }
    pub fn sound(&self, sound: qa_core::primitives::SoundId) -> Option<(u32, ResourceReadiness)> {
        let layout = self.layout?;
        let ordinal = self
            .sounds
            .ordinal(sound)
            .filter(|&ordinal| ordinal < layout.resources[1].count)?;
        Some((
            ordinal,
            self.readiness(layout.resources[1].first as usize + ordinal as usize),
        ))
    }
    pub fn project_sound(
        &self,
        module: ModuleId,
        native: Option<u32>,
        sound: qa_core::primitives::SoundId,
    ) -> Option<(u32, ResourceReadiness)> {
        if Some(module) == self.module
            && let Some(ordinal) = native
        {
            let layout = self.layout?;
            if ordinal < layout.resources[1].count && self.sounds.sound(ordinal) == Some(sound) {
                return Some((
                    ordinal,
                    self.readiness(layout.resources[1].first as usize + ordinal as usize),
                ));
            }
        }
        self.sound(sound)
    }
    /// Snapshot references use their declared resource owner, independently of
    /// the entity namespace and this connection's gameplay module.
    pub fn project_sound_ref(
        &self,
        reference: qa_core::primitives::SoundRef,
        client: ClientId,
        mut bound_sound: impl FnMut(ModuleId, u32) -> Option<qa_core::primitives::SoundId>,
    ) -> Option<u32> {
        let (ordinal, readiness) = match reference.namespace {
            ResourceNamespace::Module(module) => {
                if let Some(sound) = bound_sound(module, reference.ordinal) {
                    self.project_sound(module, Some(reference.ordinal), sound)?
                } else {
                    // A dedicated host need not decode PCM. Native primary
                    // rows retain ordinals only after compatible text is ACKed.
                    let layout = self.layout?;
                    if Some(module) != self.module
                        || reference.ordinal >= layout.resources[1].count
                        || !self.primary_claims[reference.ordinal as usize]
                    {
                        return None;
                    }
                    (
                        reference.ordinal,
                        self.readiness(
                            layout.resources[1].first as usize + reference.ordinal as usize,
                        ),
                    )
                }
            }
            ResourceNamespace::Client(owner) if owner == client => {
                let layout = self.layout?;
                if reference.ordinal >= layout.resources[1].count {
                    return None;
                }
                self.sounds.sound(reference.ordinal)?;
                (
                    reference.ordinal,
                    self.readiness(layout.resources[1].first as usize + reference.ordinal as usize),
                )
            }
            ResourceNamespace::Client(_) => return None,
        };
        (readiness == ResourceReadiness::Ready).then_some(ordinal)
    }
    /// Rebinding the CLIENT's loaded world changes the interpretation of inline
    /// filenames. Republish model rows through their existing receipt lifecycle.
    pub(crate) fn model_world(&mut self, world: Option<GeometryId>) {
        if self.model_world == world {
            return;
        }
        self.model_world = world;
        self.models.clear();
        self.model_claims.fill(false);
        self.revisions.fill(None);
        self.seen.fill(0);
        if let Some(layout) = self.layout {
            self.rows[layout.resources[0].range()].fill(PublishedConfig::default());
        }
    }
    pub(crate) fn primary_model_fits(&self, ordinal: u32) -> bool {
        self.layout.is_some_and(|layout| {
            ordinal < layout.resources[0].count && layout.resource_policies[0].permits(ordinal)
        })
    }
    pub(crate) fn model_alias_fits(&self, ordinal: u32) -> bool {
        self.primary_model_fits(ordinal)
            && self
                .layout
                .is_some_and(|layout| layout.rules != RuleSetId::Quake2 || ordinal < 127)
    }
    pub fn primary_model(&mut self, ordinal: u32, claimed: bool, name: Option<NameId>) {
        let Some(layout) = self.layout else { return };
        if !self.primary_model_fits(ordinal) {
            return;
        }
        let previous = self.model_claims[ordinal as usize];
        self.model_claims[ordinal as usize] = claimed;
        if claimed || previous {
            if self.models.get(ordinal) != name {
                self.rows[layout.resources[0].first as usize + ordinal as usize] =
                    PublishedConfig::default();
            }
            let _ = self.models.set(ordinal, name);
        }
    }
    pub fn release_model_aliases(&mut self) {
        let Some(layout) = self.layout else { return };
        for ordinal in (1..layout.resources[0].count).rev() {
            if self.model_claims[ordinal as usize] && self.model_alias_fits(ordinal) {
                self.models.prefer(ordinal);
            }
        }
        for ordinal in 1..layout.resources[0].count {
            if !self.model_claims[ordinal as usize]
                && let Some(name) = self.models.get(ordinal)
                && self.models.ordinal(name) != Some(ordinal)
            {
                let _ = self.models.set(ordinal, None);
                self.rows[layout.resources[0].first as usize + ordinal as usize] =
                    PublishedConfig::default();
            }
        }
    }
    /// Only changed configuration publication reserves new native slots.
    pub fn reserve_model(&mut self, name: NameId) -> Option<u32> {
        let layout = self.layout?;
        if let Some(ordinal) = self
            .models
            .ordinal(name)
            .filter(|&ordinal| self.model_alias_fits(ordinal))
        {
            return Some(ordinal);
        }
        let ordinal = (1..layout.resources[0].count).rev().find(|&ordinal| {
            self.model_alias_fits(ordinal)
                && !self.model_claims[ordinal as usize]
                && self.models.get(ordinal).is_none()
        })?;
        self.models.set(ordinal, Some(name)).ok()?;
        self.rows[layout.resources[0].first as usize + ordinal as usize] =
            PublishedConfig::default();
        Some(ordinal)
    }
    /// One producer-owned model projection serves entity, player, view weapon
    /// and static records. CLIENT tables never ready a SERVER reference.
    pub fn project_model_ref(
        &self,
        reference: ModelRef,
        usage: ModelUse,
        client: ClientId,
        mut bound_name: impl FnMut(ModuleId, u32, ModelUse) -> Option<ModelName>,
    ) -> ModelProjection {
        use ModelProjection::{Pending, Ready, Unsupported};
        let Some(layout) = self.layout else {
            return Unsupported;
        };
        let primary = match reference.namespace {
            ResourceNamespace::Module(module) => Some(module) == self.module,
            ResourceNamespace::Client(owner) => owner == client,
        };
        if primary && reference.ordinal == 0 {
            return Ready(0);
        }
        let cell_fits = |ordinal, translucent| {
            ordinal < layout.resources[0].count
                && !(ordinal == 255
                    && matches!(layout.rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
                    && matches!(usage, ModelUse::Entity(0 | 1)))
                && !(layout.rules == RuleSetId::Quake2
                    && usage == ModelUse::Entity(1)
                    && ordinal >= if translucent { 127 } else { 128 })
        };
        let translucent;
        let ordinal = match reference.namespace {
            ResourceNamespace::Module(module) => {
                let Some(source) = bound_name(module, reference.ordinal, usage) else {
                    return Unsupported;
                };
                if reference.ordinal >= ConfigLayout::for_rules(source.source).resources[0].count {
                    return Unsupported;
                }
                let input = match model_input(source.source, reference.ordinal, Some(usage)) {
                    ModelInput::Player => {
                        return if primary
                            && matches!(
                                layout.rules,
                                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
                            ) {
                            Ready(255)
                        } else {
                            Unsupported
                        };
                    }
                    ModelInput::Resource(ordinal) => {
                        translucent = false;
                        ordinal
                    }
                    ModelInput::LinkedTranslucent(ordinal) => {
                        translucent = true;
                        ordinal
                    }
                };
                let Some(name) = source.name else {
                    return Unsupported;
                };
                if primary
                    && self.primary_model_fits(input)
                    && cell_fits(input, translucent)
                    && self.models.get(input) == Some(name)
                {
                    input
                } else if let Some(ordinal) = self
                    .models
                    .ordinal(name)
                    .filter(|&ordinal| cell_fits(ordinal, translucent))
                {
                    ordinal
                } else {
                    return if self.begin(module, source.revision) {
                        Pending
                    } else {
                        Unsupported
                    };
                }
            }
            ResourceNamespace::Client(owner) if owner == client => {
                let input = match model_input(layout.rules, reference.ordinal, Some(usage)) {
                    ModelInput::Player => return Ready(255),
                    ModelInput::Resource(ordinal) => {
                        translucent = false;
                        ordinal
                    }
                    ModelInput::LinkedTranslucent(ordinal) => {
                        translucent = true;
                        ordinal
                    }
                };
                if self.models.get(input).is_none() {
                    return Unsupported;
                }
                input
            }
            ResourceNamespace::Client(_) => return Unsupported,
        };
        if !cell_fits(ordinal, translucent) {
            return Unsupported;
        }
        match self.readiness(layout.resources[0].first as usize + ordinal as usize) {
            ResourceReadiness::Ready => {
                if translucent && layout.rules == RuleSetId::Quake2 {
                    Ready(ordinal | 0x80)
                } else {
                    Ready(ordinal)
                }
            }
            ResourceReadiness::Pending => Pending,
            ResourceReadiness::Unsupported => Unsupported,
        }
    }

    /// Called only while changed precaches are published, never at emission.
    /// Secondary slots grow downwards, leaving low native ordinals available.
    pub fn reserve_sound(&mut self, sound: qa_core::primitives::SoundId) -> Option<u32> {
        let layout = self.layout?;
        if let Some((ordinal, _)) = self.sound(sound) {
            return Some(ordinal);
        }
        let ordinal = (1..layout.resources[1].count).rev().find(|&ordinal| {
            !self.primary_claims[ordinal as usize] && self.sounds.sound(ordinal).is_none()
        })?;
        self.sounds.set(ordinal, Some(sound)).ok()?;
        self.rows[layout.resources[1].first as usize + ordinal as usize] =
            PublishedConfig::default();
        Some(ordinal)
    }
}
