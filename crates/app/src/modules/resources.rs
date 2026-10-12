//! Resource publication uses the same retained output ring as module messages.
use super::{FrameHost, State};
use qa_core::{
    configstrings::ConfigLayout,
    events::OutputTarget,
    primitives::{ClientId, RuleSetId},
};

impl FrameHost {
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
            let Some(models) = row
                .storage
                .resources(row.request.context.module)
                .map(|resources| resources[0])
            else {
                continue;
            };
            for index in models.range().skip(1) {
                if let Ok((text, _)) = row.storage.configstring(row.request.context.module, index)
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
        profiles: impl IntoIterator<Item = (ClientId, crate::model_assets::ModelProfile)>,
    ) {
        for resources in self.runtime.client_resources.iter_mut().flatten() {
            resources.state.models.reset();
        }
        for (client, profile) in profiles {
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
        let Some(modules) = &self.modules else { return };
        let runtime = &mut self.runtime;
        for (index, client) in runtime.server.clients.iter().enumerate() {
            let Some(output) = client.output else {
                continue;
            };
            if client.connection.is_none() || runtime.server.events.needs_resync(output) {
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
            );
            let changed = modules
                .rows
                .iter()
                .flatten()
                .filter(|row| row.state == State::Running)
                .any(|row| {
                    row.storage
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
                        row.storage.configstring(client.module, native_index)
                    else {
                        continue;
                    };
                    if !publication.needs(native_index, revision) {
                        continue;
                    }
                    let target_index = target.project_index(source, native_index);
                    let is_sound = source.resources[1].contains(native_index);
                    let mut sample = None;
                    if is_sound {
                        let ordinal = native_index - source.resources[1].first as usize;
                        sample = row
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
                    let Some(sounds) = row.storage.resources(module).map(|resources| resources[1])
                    else {
                        continue;
                    };
                    for ordinal in 1..sounds.count {
                        let Some(sound) = row
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
                    if let Ok(revision) = row.storage.config_revision(row.request.context.module) {
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
