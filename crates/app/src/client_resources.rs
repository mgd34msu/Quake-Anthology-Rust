//! Connection-owned configuration and derived numeric presentation bindings.
use crate::{client_scene::ClientModelBindings, model_assets::ModelCatalog};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{
        ClientId, EntityId, EntityRecord, ModelRef, ModuleId, ResourceNamespace, RuleSetId,
        SpatialPolicy,
    },
};
use qa_network::commands::packet::Protocol;

pub struct ClientResources {
    pub state: ResourceState,
    commands: qa_network::outputs::ConfigCommandReader,
    selected: Box<qa_console::command_text::ParsedCommand>,
    joined: qa_core::text::FixedText<8192>,
    pub unsupported_commands: u64,
}
pub struct ResourceState {
    pub configs: ConfigStrings,
    pub models: ClientModelBindings,
    pub layout: ConfigLayout,
    pub rejected: u64,
    sounds: qa_audio::SoundBindings,
    player_sounds: [Option<(qa_core::primitives::SoundId, u32)>;
        qa_gameplay::rules::feedback::PLAYER_SOUND_PATHS.len()],
    sound_policy: SpatialPolicy,
    loop_spatial: SpatialPolicy,
    static_sounds: u32,
    static_entities: Box<[EntityRecord]>,
    static_entity_count: usize,
}
impl ClientResources {
    pub fn load(
        client: ClientId,
        protocol: Protocol,
        sound_rules: RuleSetId,
    ) -> Result<Option<Self>, String> {
        let layout = protocol.resource_layout();
        Ok(Some(Self {
            commands: qa_network::outputs::ConfigCommandReader::load(),
            selected: Box::default(),
            joined: qa_core::text::FixedText::default(),
            unsupported_commands: 0,
            state: ResourceState {
                configs: ConfigStrings::load(
                    (0..layout.count).map(|index| layout.capacity(index).unwrap_or(0)),
                )
                .map_err(|error| format!("client configuration: {error:?}"))?,
                models: ClientModelBindings::load(client, layout),
                layout,
                rejected: 0,
                sounds: qa_audio::SoundBindings::load(layout.resources[1].count as usize),
                player_sounds: [None; qa_gameplay::rules::feedback::PLAYER_SOUND_PATHS.len()],
                sound_policy: qa_gameplay::rules::sound_policy(sound_rules),
                loop_spatial: qa_gameplay::rules::loop_spatial_policy(sound_rules),
                static_sounds: 0,
                static_entities: vec![
                    EntityRecord::default();
                    qa_core::primitives::NATIVE_WIRE_BYTES
                        / qa_network::outputs::STATIC_ENTITY_BYTES
                ]
                .into_boxed_slice(),
                static_entity_count: 0,
            },
        }))
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
        // The channel validated every native row, baseline and aggregate byte
        // count before this callback. Replace the sole CLIENT storage; readiness
        // still belongs to the channel's completed outer-message marker.
        self.state.configs.reset();
        self.commands.reset();
        self.selected.clear();
        self.joined.clear();
        let mut rejected = 0;
        let result = gamestate.for_each_config(|config| {
            rejected += u64::from(
                self.state
                    .configs
                    .set(config.index as usize, config.text)
                    .is_err(),
            );
        });
        self.state.rejected += rejected + u64::from(result.is_err());
        if let Some(catalog) = catalog {
            self.state.rejected +=
                u64::from(self.state.models.refresh_all(&self.state.configs, catalog));
        }
        self.state.sounds.clear();
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
    bank: Option<&'a qa_audio::Bank>,
    pub(crate) disconnected: bool,
}
pub(crate) fn connection_view<'a>(
    client: ClientId,
    lifetime: EntityId,
    network: &'a mut qa_network::ingress::Connections,
    resources: &'a mut [Option<ClientResources>],
    catalog: Option<&'a ModelCatalog>,
    bank: Option<&'a qa_audio::Bank>,
) -> Option<ClientConnectionView<'a>> {
    let connection = network.get_mut(client, qa_core::loopback::Endpoint::Client)?;
    let channel = &mut connection.channel;
    let commands = connection.commands.as_mut()?;
    let resources = resources.get_mut(client.0 as usize)?.as_mut()?;
    Some(ClientConnectionView::new(
        client, lifetime, channel, commands, resources, catalog, bank,
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
        bank: Option<&'a qa_audio::Bank>,
    ) -> Self {
        Self {
            client,
            lifetime,
            channel,
            commands,
            resources,
            catalog,
            bank,
            disconnected: false,
        }
    }
    fn reject<T>(
        &mut self,
        error: qa_compat::services::CallError,
    ) -> Result<T, qa_compat::services::CallError> {
        self.resources.state.rejected += 1;
        self.disconnected = true;
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
            if !self.select(number as i32, server)? {
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
    fn disconnected(&self) -> bool {
        self.disconnected
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
                self.disconnected = true;
                Err(qa_compat::services::CallError::Memory)
            }
        }
    }
    fn select(
        &mut self,
        number: i32,
        server: &mut qa_session::clients::Server,
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
                    self.disconnected = true;
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
                        self.bank,
                    ) {
                        self.disconnected = true;
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
    /// Native signon copies survive freeing or reusing their source edicts.
    pub fn receive_static_entity(&mut self, mut record: EntityRecord) -> bool {
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
        *slot = record;
        self.static_entity_count += 1;
        true
    }

    pub fn static_entities(&self) -> &[EntityRecord] {
        &self.static_entities[..self.static_entity_count]
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
        self.sounds.attach(bank);
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

    fn refresh_sounds(&mut self, bank: Option<&qa_audio::Bank>) -> bool {
        let mut complete = true;
        for (ordinal, index) in self.layout.resources[1].range().enumerate() {
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
                if changed && let Some(catalog) = catalog {
                    self.rejected +=
                        u64::from(!self.models.config_changed(index, &self.configs, catalog));
                }
                if changed && self.layout.resources[1].contains(index) {
                    let ordinal = index - self.layout.resources[1].first as usize;
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
    primary_claims: Box<[bool]>,
    pub unsupported: u64,
    pub pending: Option<(u64, usize)>,
    game_state: Option<(u64, bool)>,
    game_state_required: bool,
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
            primary_claims: vec![false; sounds].into_boxed_slice(),
            unsupported: 0,
            pending: None,
            game_state: None,
            game_state_required: false,
        }
    }
    /// Source capacities are fixed when the module set is loaded.
    pub fn configure_sources(&mut self, capacity: usize) {
        self.revisions = vec![None; capacity].into_boxed_slice();
        self.reset();
    }
    pub fn bind_sounds(&mut self, bank: &qa_audio::Bank) {
        self.sounds.attach(bank);
        self.revisions.fill(None);
        self.seen.fill(0);
        self.rows.fill(PublishedConfig::default());
        self.primary_claims.fill(false);
    }
    pub fn reset(&mut self) {
        self.module = None;
        self.layout = None;
        self.revisions.fill(None);
        self.seen.fill(0);
        self.rows.fill(PublishedConfig::default());
        self.sounds.clear();
        self.primary_claims.fill(false);
        self.pending = None;
        self.game_state = None;
        self.game_state_required = false;
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

/// Native resource ordinals are supplied by the owning module/connection,
/// never obtained by narrowing an engine entity or render asset handle.
pub fn model_ordinal(
    reference: ModelRef,
    module: Option<ModuleId>,
    client: ClientId,
    limit: u32,
) -> Option<u32> {
    let namespace_matches = match reference.namespace {
        ResourceNamespace::Module(owner) => Some(owner) == module,
        ResourceNamespace::Client(owner) => owner == client,
    };
    (namespace_matches && reference.ordinal < limit).then_some(reference.ordinal)
}
