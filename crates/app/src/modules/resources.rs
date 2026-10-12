//! Resource publication uses the same retained output ring as module messages.
use super::{FrameHost, State};
use qa_core::{
    configstrings::ConfigLayout,
    events::OutputTarget,
    primitives::{ClientId, RuleSetId},
};

impl super::Modules {
    pub(crate) fn sound_ordinal(
        &self,
        module: qa_core::primitives::ModuleId,
        sound: qa_core::primitives::SoundId,
    ) -> Option<u32> {
        let row = self.row(module).filter(|row| row.state == State::Running)?;
        row.storage.sound_ordinal(module, sound)
    }
}

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
            let Some(layout) = ConfigLayout::for_rules(rules) else {
                continue;
            };
            for index in layout.resources[0].range().skip(1) {
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
            let Some(row) = modules
                .row(client.module)
                .filter(|row| row.state == State::Running)
            else {
                continue;
            };
            // SERVER selects delivery from its channel, never CLIENT caches.
            // Remote clients do not own an in-process resource projection.
            let Some(target_layout) = runtime
                .network
                .get(
                    qa_core::primitives::ClientId(index as u32),
                    qa_core::loopback::Endpoint::Server,
                )
                .and_then(|connection| connection.commands.as_ref())
                .and_then(|commands| ConfigLayout::for_rules(commands.protocol.rule_set()))
            else {
                continue;
            };
            let source = row.request.context.native_rules;
            let Some(source_layout) = ConfigLayout::for_rules(source) else {
                continue;
            };
            let Ok(revision) = row.storage.config_revision(client.module) else {
                continue;
            };
            let publication = &mut runtime.config_publications[index];
            if !publication.begin(client.module, revision) {
                continue;
            }
            let mut complete = true;
            for native_index in 0..row.request.configstrings.min(source_layout.count) {
                let Ok((text, row_revision)) =
                    row.storage.configstring(client.module, native_index)
                else {
                    continue;
                };
                if !publication.needs(native_index, row_revision) {
                    continue;
                }
                let Some(target_index) = target_layout.project_index(source_layout, native_index)
                else {
                    publication.unsupported += 1;
                    publication.published(native_index, row_revision, None);
                    continue;
                };
                if text.len() > target_layout.capacity(target_index).unwrap_or(0) {
                    publication.unsupported += 1;
                    publication.published(native_index, row_revision, None);
                    continue;
                }
                if runtime.server.events.needs_resync(output) {
                    complete = false;
                    break;
                }
                let Ok(sequence) = runtime.server.events.publish_configstring(
                    Some(OutputTarget::Client(ClientId(index as u32))),
                    client.module,
                    source,
                    native_index as u32,
                    text,
                ) else {
                    complete = false;
                    break;
                };
                publication.published(native_index, row_revision, Some(sequence));
            }
            if complete {
                publication.finish(revision);
            }
        }
    }
}
