//! Resource publication uses the same retained output ring as module messages.
use super::{FrameHost, State};
use qa_core::{
    configstrings::ConfigLayout,
    events::OutputTarget,
    primitives::{ClientId, RuleSetId},
};

impl FrameHost {
    /// Module renderer provenance is independent of the SERVER publisher's
    /// configstring profile. Bind it before CLIENT initialization freezes assets.
    pub fn configure_native_scenes(
        &mut self,
        defaults: &[Option<crate::model_assets::ModelProfile>; 5],
        seats: &[[Option<crate::model_assets::ModelProfile>; 5];
             qa_core::sys_events::SeatId::COUNT],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let Some(modules) = self.modules.as_mut() else {
            return Ok(());
        };
        let count = self.local_clients.iter().flatten().count();
        for row in modules
            .rows
            .iter_mut()
            .flatten()
            .filter(|row| row.request.phase == super::Phase::Client)
        {
            let source = row.request.context.native_rules as usize;
            let seat = row.client_connection.and_then(|(client, _)| {
                self.runtime
                    .snapshots
                    .get(client.0 as usize)?
                    .as_ref()?
                    .seat()
            });
            let profile = seat
                .and_then(|seat| seats[seat.index()][source])
                .or(defaults[source])
                .ok_or("missing module renderer profile")?;
            let (viewport, world, presentation) = if let Some(seat) = seat {
                let view_index = self.local_clients[..seat.index()].iter().flatten().count();
                (
                    crate::renderer::viewport(width, height, view_index, count),
                    self.local_worlds[seat.index()],
                    self.local_presentations[seat.index()],
                )
            } else {
                (
                    qa_render::Viewport {
                        x: 0,
                        y: 0,
                        width,
                        height,
                    },
                    None,
                    Default::default(),
                )
            };
            let context = row.request.context.console;
            let rails = ["r_railCoreWidth", "r_railWidth", "r_railSegmentLength"]
                .map(|name| self.console.cvars.bind(name, context));
            let [Some(core), Some(ring), Some(length)] = rails else {
                return Err("missing rail renderer cvars".into());
            };
            let lighting = ["r_ambientScale", "r_directedScale"]
                .map(|name| self.console.cvars.bind(name, context));
            let [Some(ambient), Some(directed)] = lighting else {
                return Err("missing entity lighting cvars".into());
            };
            let software_waterwarp = self
                .console
                .cvars
                .bind("r_waterwarp", context)
                .ok_or("missing software water warp cvar")?;
            row.scene_role.configure(
                profile,
                viewport,
                seat,
                world,
                presentation,
                [core, ring, length],
                [ambient, directed],
                software_waterwarp,
            );
        }
        Ok(())
    }
    /// Native Q3 SERVER storage supplies its own gamestate. Encoding borrows
    /// the source rows; only the existing output ring retains the message.
    pub(crate) fn publish_module_game_states(&mut self) {
        let Some(modules) = &self.modules else { return };
        let runtime = &mut self.runtime;
        for (index, client) in runtime.server.clients.iter().enumerate() {
            let Some(output) = client.output else {
                continue;
            };
            if !client.is_active() || runtime.server.events.needs_resync(output) {
                continue;
            }
            let Some(row) = modules.row(client.module).filter(|row| {
                row.request.phase == super::Phase::Server
                    && row.request.context.native_rules == RuleSetId::Quake3
            }) else {
                continue;
            };
            let id = ClientId(index as u32);
            let Some(connection) = runtime
                .network
                .get_mut(id, qa_core::loopback::Endpoint::Server)
            else {
                continue;
            };
            if connection
                .commands
                .as_ref()
                .map(|commands| commands.protocol)
                != Some(qa_network::commands::packet::Protocol::Quake3_68)
            {
                continue;
            }
            let publication = &mut runtime.config_publications[index];
            publication.connection(client.module, ConfigLayout::QUAKE3);
            publication.require_game_state();
            if row.state != State::Running || publication.game_state_published() {
                continue;
            }
            let Some(native_client) = runtime.snapshots[index]
                .as_ref()
                .map(|binding| binding.native_client())
            else {
                continue;
            };
            let channel = &mut connection.channel;
            let Ok(mut context) = channel.command_context() else {
                continue;
            };
            let mut info = [[0; qa_core::configstrings::MAX_TEXT]; 2];
            // Native CVAR_SERVERINFO / CVAR_SYSTEMINFO use the unified table's
            // source projection. They are engine rows, not game-module text.
            let mut lengths = [0; 2];
            let mut valid = true;
            for (slot, flag) in [4, 8].into_iter().enumerate() {
                match self.console.cvars.info_string(
                    qa_console::views::Context {
                        source: RuleSetId::Quake3,
                        side: qa_console::catalog::Scope::Server,
                        role: qa_console::views::Role::Engine,
                        ..row.request.context.console
                    },
                    flag,
                    &mut info[slot],
                ) {
                    Ok(length) => lengths[slot] = length,
                    Err(_) => valid = false,
                }
            }
            if !valid {
                runtime.server.events.reject_publication();
                continue;
            }
            context.set_system_info(&info[ConfigLayout::Q3_SYSTEM_INFO][..lengths[1]]);
            let command_sequence = channel.command_state().map_or(0, |state| state.queued);
            let configs = (0..row.request.configstrings.max(2)).filter_map(|native_index| {
                if native_index <= ConfigLayout::Q3_SYSTEM_INFO {
                    Some((native_index, &info[native_index][..lengths[native_index]]))
                } else {
                    modules
                        .storage
                        .configstring(client.module, native_index)
                        .ok()
                        .map(|(text, _)| (native_index, text))
                }
            });
            let mut writer = qa_network::message::Writer::new(
                &mut runtime.output_native_bytes,
                qa_network::message::Encoding::Q3,
            );
            if qa_network::channel::commands::write_gamestate(
                &mut writer,
                command_sequence,
                native_client,
                context.checksum_feed,
                configs,
                // These connections currently use zero snapshot baselines.
                // Their first snapshot supplies full zero-base deltas.
                std::iter::empty(),
            )
            .is_err()
            {
                runtime.server.events.reject_publication();
                continue;
            }
            let (length, body_bits) = (writer.size(), writer.bit_position());
            let delivery = qa_core::primitives::NativeDelivery {
                module: client.module,
                source: RuleSetId::Quake3,
                target: qa_core::primitives::NativeTarget::Client(id),
                origin: client.player.body.position,
                reliable: true,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: false,
            };
            let Ok(sequence) = runtime.server.events.game_state(
                delivery,
                command_sequence,
                body_bits,
                &runtime.output_native_bytes[..length],
            ) else {
                continue;
            };
            let _ = channel.set_command_context(context);
            publication.publish_game_state(sequence);
            for native_index in 0..row.request.configstrings {
                if let Ok((text, revision)) =
                    modules.storage.configstring(client.module, native_index)
                {
                    publication.seen(native_index, revision);
                    if ConfigLayout::QUAKE3.resources[1].contains(native_index) {
                        let ordinal =
                            native_index - ConfigLayout::QUAKE3.resources[1].first as usize;
                        let sample = modules
                            .storage
                            .sound_id(
                                client.module,
                                ConfigLayout::QUAKE3.resources[1],
                                ordinal as i32,
                            )
                            .ok()
                            .flatten();
                        publication.primary_sound(ordinal as u32, !text.is_empty(), sample);
                    }
                    publication.published(native_index, Some(sequence));
                }
            }
            for native_index in [ConfigLayout::Q3_SERVER_INFO, ConfigLayout::Q3_SYSTEM_INFO] {
                publication.published(native_index, Some(sequence));
            }
        }
    }
    /// Cold registration only. CLIENT rendering never calls module storage.
    pub fn model_precaches(&self) -> Vec<(RuleSetId, &[u8])> {
        let mut names = Vec::new();
        let Some(modules) = &self.modules else {
            return names;
        };
        for row in modules.rows.iter().flatten() {
            if row.state != State::Running {
                continue;
            }
            let rules = row.request.context.native_rules;
            let Some(models) = modules
                .storage
                .resources(row.request.context.module)
                .map(|resources| resources[0])
            else {
                continue;
            };
            for index in models.range().skip(1) {
                if let Ok((text, _)) = modules
                    .storage
                    .configstring(row.request.context.module, index)
                    && !text.is_empty()
                    && !text.starts_with(b"*")
                {
                    names.push((rules, text));
                }
            }
        }
        names
    }

    /// Provenance comes from each client's publishing module, never its wire
    /// protocol or selected presentation. This is a cold registration query.
    pub fn client_model_sources(&self) -> Vec<(ClientId, RuleSetId)> {
        self.runtime
            .server
            .clients
            .iter()
            .enumerate()
            .filter_map(|(index, client)| {
                client.connection.as_ref()?;
                let row = self
                    .modules
                    .as_ref()?
                    .row(client.module)
                    .filter(|row| row.state == State::Running)?;
                Some((ClientId(index as u32), row.request.context.native_rules))
            })
            .collect()
    }

    pub fn set_model_catalog(
        &mut self,
        models: std::sync::Arc<crate::model_assets::ModelCatalog>,
        profiles: impl IntoIterator<
            Item = (
                ClientId,
                crate::model_assets::ModelProfile,
                Option<(
                    qa_render::world::WorldId,
                    qa_render::world::WorldPresentationId,
                )>,
            ),
        >,
    ) {
        for resources in self.runtime.client_resources.iter_mut().flatten() {
            resources.state.models.reset();
        }
        for (client, profile, world) in profiles {
            let Some(resources) = self
                .runtime
                .client_resources
                .get_mut(client.0 as usize)
                .and_then(Option::as_mut)
            else {
                continue;
            };
            resources.state.rejected += u64::from(resources.state.models.bind_all(
                &resources.state.configs,
                &models,
                profile,
                world,
            ));
        }
        self.runtime.models = Some(models);
    }

    /// A row is recorded as published only after its text lease reaches the
    /// ring. Rejected rows are retried; successful rows are not duplicated.
    pub(crate) fn configure_output_resources(&mut self) {
        let capacity = self
            .modules
            .as_ref()
            .map_or(0, |modules| modules.rows.len());
        for publication in &mut self.runtime.config_publications {
            publication.configure_sources(capacity);
        }
    }
    pub(crate) fn bind_output_sounds(&mut self, bank: &qa_audio::Bank) {
        for publication in &mut self.runtime.config_publications {
            publication.bind_sounds(bank);
        }
    }
    pub(crate) fn publish_module_configs(&mut self) {
        let modules = self.modules.as_ref();
        let runtime = &mut self.runtime;
        // The window shell and isolated channel consumers have no loaded
        // SERVER. Native modules or registered collision geometry supply it.
        let publish_limits = modules.is_some() || runtime.geometry.geometries().next().is_some();
        for (index, client) in runtime.server.clients.iter().enumerate() {
            let Some(output) = client.output else {
                continue;
            };
            if !client.is_active() || runtime.server.events.needs_resync(output) {
                continue;
            }
            let Some(protocol) = runtime
                .network
                .get(ClientId(index as u32), qa_core::loopback::Endpoint::Server)
                .and_then(|connection| connection.commands.as_ref())
                .map(|commands| commands.protocol)
            else {
                continue;
            };
            let Some(target) = protocol.config_layout() else {
                continue;
            };
            let publication = &mut runtime.config_publications[index];
            publication.connection(client.module, target);
            runtime.native_numbers[index].refresh(
                &runtime.server.entities,
                runtime.server.entities.columns.native_entity[client.entity.slot as usize]
                    .map(|native| native.module),
                protocol,
                runtime
                    .server
                    .clients
                    .iter()
                    .filter(|client| client.is_active())
                    .map(|client| client.entity),
            );
            if publish_limits && let Some(destination) = target.max_clients() {
                let count = runtime.native_numbers[index].player_count();
                if publication.needs_client_limit(count) {
                    let mut text = qa_core::text::FixedText::<16>::default();
                    use std::fmt::Write;
                    if write!(&mut text, "{count}").is_err() {
                        continue;
                    }
                    let Ok(sequence) = runtime.server.events.publish_configstring(
                        Some(OutputTarget::Client(ClientId(index as u32))),
                        client.module,
                        target.rules,
                        destination as u32,
                        destination as u32,
                        text.as_bytes(),
                    ) else {
                        continue;
                    };
                    publication.published_client_limit(destination, count, sequence);
                }
            }
            let Some(modules) = modules else { continue };
            let changed = modules
                .rows
                .iter()
                .flatten()
                .filter(|row| row.state == State::Running)
                .any(|row| {
                    modules
                        .storage
                        .config_revision(row.request.context.module)
                        .is_ok_and(|revision| {
                            publication.begin(row.request.context.module, revision)
                        })
                });
            if !changed {
                continue;
            }
            let mut complete = true;
            // General config rows and native sound claims belong to the primary
            // namespace. Secondary models/HUD/general config mapping is separate.
            if let Some(row) = modules
                .row(client.module)
                .filter(|row| row.state == State::Running)
            {
                let source = ConfigLayout::for_rules(row.request.context.native_rules);
                for native_index in 0..row.request.configstrings.min(source.count) {
                    let Ok((text, revision)) =
                        modules.storage.configstring(client.module, native_index)
                    else {
                        continue;
                    };
                    if !publication.needs(native_index, revision) {
                        continue;
                    }
                    if source.rules == RuleSetId::Quake3
                        && target.rules == RuleSetId::Quake3
                        && publication.game_state_published()
                        && native_index <= ConfigLayout::Q3_SYSTEM_INFO
                    {
                        // SERVER info is projected from engine cvars, not a
                        // module's empty or private copy of those two rows.
                        publication.seen(native_index, revision);
                        continue;
                    }
                    let target_index = target.project_index(source, native_index);
                    let is_sound = source.resources[1].contains(native_index);
                    let mut sample = None;
                    if is_sound {
                        let ordinal = native_index - source.resources[1].first as usize;
                        sample = modules
                            .storage
                            .sound_id(client.module, source.resources[1], ordinal as i32)
                            .ok()
                            .flatten();
                        publication.primary_sound(ordinal as u32, !text.is_empty(), sample);
                    }
                    let Some(destination) = target_index else {
                        publication.unsupported += 1;
                        publication.seen(native_index, revision);
                        continue;
                    };
                    if target.max_clients() == Some(destination) {
                        // Player ordinals belong to this connection's native
                        // namespace, independently of the publishing module.
                        publication.seen(native_index, revision);
                        continue;
                    }
                    // Preserve exact text only when both native filename
                    // codecs agree, including Quake's literal prefix bytes.
                    let converted;
                    let text =
                        if is_sound
                            && !text.is_empty()
                            && !qa_audio::same_sound_codec(source.rules, target.rules)
                        {
                            let Some(name) = runtime.sound_bank.as_deref().zip(sample).and_then(
                                |(bank, sample)| {
                                    qa_audio::native_sound_name(target.rules, bank, sample)
                                },
                            ) else {
                                publication.unsupported += 1;
                                publication.published(destination, None);
                                publication.seen(native_index, revision);
                                continue;
                            };
                            converted = name;
                            converted.as_bytes()
                        } else {
                            text
                        };
                    if protocol
                        .config_capacity(destination)
                        .is_none_or(|limit| text.len() > limit)
                    {
                        publication.unsupported += 1;
                        publication.published(destination, None);
                        publication.seen(native_index, revision);
                        continue;
                    }
                    if runtime.server.events.needs_resync(output) {
                        complete = false;
                        break;
                    }
                    match runtime.server.events.publish_configstring(
                        Some(OutputTarget::Client(ClientId(index as u32))),
                        client.module,
                        source.rules,
                        native_index as u32,
                        destination as u32,
                        text,
                    ) {
                        Ok(sequence) => {
                            publication.published(destination, Some(sequence));
                            publication.seen(native_index, revision);
                        }
                        Err(_) => {
                            complete = false;
                            break;
                        }
                    }
                }
            }
            publication.release_aliases();
            if let Some(bank) = runtime.sound_bank.as_deref() {
                // Running secondary module resources get first claim on free
                // slots; resolving a PCM handle never rebuilds or hashes a path.
                for row in modules.rows.iter().flatten().filter(|row| {
                    row.state == State::Running && row.request.context.module != client.module
                }) {
                    let module = row.request.context.module;
                    let Some(sounds) = modules
                        .storage
                        .resources(module)
                        .map(|resources| resources[1])
                    else {
                        continue;
                    };
                    for ordinal in 1..sounds.count {
                        let Some(sound) = modules
                            .storage
                            .sound_id(module, sounds, ordinal as i32)
                            .ok()
                            .flatten()
                        else {
                            continue;
                        };
                        if !publish_sound(
                            publication,
                            &mut runtime.server.events,
                            output,
                            ClientId(index as u32),
                            module,
                            row.request.context.native_rules,
                            sounds.first + ordinal,
                            target,
                            protocol,
                            bank,
                            sound,
                        ) {
                            complete = false;
                            break;
                        }
                    }
                }
                // Registered PCM names are immutable. Preserve old numeric
                // events after a primary row rewrite by relocating their sample,
                // including when its module no longer declares the old ordinal.
                for sound in 1..bank.len() {
                    if !publish_sound(
                        publication,
                        &mut runtime.server.events,
                        output,
                        ClientId(index as u32),
                        client.module,
                        target.rules,
                        target.resources[1].first,
                        target,
                        protocol,
                        bank,
                        qa_core::primitives::SoundId(sound as u32),
                    ) {
                        complete = false;
                        break;
                    }
                }
            }
            if complete {
                for row in modules
                    .rows
                    .iter()
                    .flatten()
                    .filter(|row| row.state == State::Running)
                {
                    if let Ok(revision) =
                        modules.storage.config_revision(row.request.context.module)
                    {
                        publication.finish(row.request.context.module, revision);
                    }
                }
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Source provenance and destination namespace stay independent at publication"
)]
fn publish_sound(
    publication: &mut crate::client_resources::ConfigPublication,
    events: &mut qa_core::events::EventRing,
    output: qa_core::events::OutputConsumerId,
    client: ClientId,
    module: qa_core::primitives::ModuleId,
    source: RuleSetId,
    source_index: u32,
    target: ConfigLayout,
    protocol: qa_network::commands::packet::Protocol,
    bank: &qa_audio::Bank,
    sound: qa_core::primitives::SoundId,
) -> bool {
    let Some(ordinal) = publication.reserve_sound(sound) else {
        return true;
    };
    let index = target.resources[1].first as usize + ordinal as usize;
    // Ready and already-staged rows need no second config record. Pending with
    // no sequence is a new reservation or a row whose publication was rejected.
    if !publication.unpublished(index) {
        return true;
    }
    let Some(name) = qa_audio::native_sound_name(target.rules, bank, sound) else {
        publication.unsupported += 1;
        publication.published(index, None);
        return true;
    };
    if protocol
        .config_capacity(index)
        .is_none_or(|limit| name.as_bytes().len() > limit)
    {
        publication.unsupported += 1;
        publication.published(index, None);
        return true;
    }
    if events.needs_resync(output) {
        return false;
    }
    match events.publish_configstring(
        Some(OutputTarget::Client(client)),
        module,
        source,
        source_index,
        index as u32,
        name.as_bytes(),
    ) {
        Ok(sequence) => {
            publication.published(index, Some(sequence));
            true
        }
        Err(_) => false,
    }
}
