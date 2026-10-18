//! Engine-owned Q3 config rows, prepared once per module and shared by clients.
use qa_console::{cvars::Cvars, views::Context};
use qa_content::vfs::{MountId, SearchView, Vfs};
use qa_core::{
    configstrings::{ConfigLayout, MAX_TEXT},
    primitives::{FileSearchId, RuleSetId},
    text::FixedText,
};

type Generation = (u64, u64, u64, Option<FileSearchId>);

pub(super) fn load_directory_default(
    cvars: &mut Cvars,
    vfs: &Vfs,
    requests: &[super::ModuleRequest],
) -> Result<(), String> {
    let base = qa_content::products::native_base(RuleSetId::Quake3).ok_or("Q3 base product")?;
    let mut selected: Option<FixedText<{ qa_core::names::MAX_PATH_BYTES }>> = None;
    for request in requests.iter().filter(|request| {
        request.phase == super::Phase::Server && request.context.native_rules == RuleSetId::Quake3
    }) {
        let Some(source) = request.source.as_ref() else {
            continue;
        };
        let Some(directory) = vfs
            .source_game_directory(source, base)
            .map_err(|error| format!("SERVER program origin: {error}"))?
        else {
            continue;
        };
        if selected
            .as_ref()
            .is_some_and(|selected| selected.as_bytes() != directory.as_bytes())
        {
            // One host has one canonical filesystem setting. A mixed provider
            // set must not inherit whichever program happened to load last.
            return Ok(());
        }
        selected = Some(directory);
    }
    if let Some(directory) = selected {
        let view = cvars
            .bind(
                "fs_game",
                Context {
                    source: RuleSetId::Quake3,
                    side: qa_console::catalog::Scope::Server,
                    role: qa_console::views::Role::Engine,
                    ..Context::default()
                },
            )
            .ok_or("missing fs_game")?;
        cvars
            .load_default(view, directory.as_str())
            .map_err(|error| format!("SERVER game directory default: {error:?}"))?;
    }
    Ok(())
}

#[derive(Default)]
struct Row {
    text: FixedText<MAX_TEXT>,
    revision: u64,
    prepared: Option<Generation>,
}
pub(super) struct ServerInfo {
    context: Context,
    map: Option<qa_console::cvars::View>,
    map_seen: Option<(Option<FileSearchId>, u64)>,
    rows: [Row; 2],
    mounts: Vec<MountId>,
    pub pure: bool,
    pub ready: bool,
}
impl ServerInfo {
    pub fn load(cvars: &Cvars, context: Context, mounts: usize) -> Self {
        let context = Context {
            source: RuleSetId::Quake3,
            side: qa_console::catalog::Scope::Server,
            role: qa_console::views::Role::Engine,
            ..context
        };
        Self {
            map: cvars.bind("mapname", context),
            context,
            map_seen: None,
            rows: Default::default(),
            mounts: Vec::with_capacity(mounts.max(qa_network::downloads::Q3_PACKAGE_PATHS)),
            pure: false,
            ready: false,
        }
    }
    pub fn text(&self, index: usize) -> &[u8] {
        self.rows[index].text.as_bytes()
    }
    pub fn revision(&self, index: usize) -> u64 {
        self.rows[index].revision
    }
    pub fn publish(
        &self,
        publication: &mut crate::client_resources::ConfigPublication,
        events: &mut qa_core::events::EventRing,
        client: qa_core::primitives::ClientId,
        module: qa_core::primitives::ModuleId,
    ) -> bool {
        if !self.ready {
            return false;
        }
        let mut system_published = false;
        for index in [ConfigLayout::Q3_SERVER_INFO, ConfigLayout::Q3_SYSTEM_INFO] {
            let revision = self.revision(index);
            if publication.server_info[index] == Some(revision) {
                continue;
            }
            let Ok(sequence) = events.publish_configstring(
                Some(qa_core::events::OutputTarget::Client(client)),
                module,
                RuleSetId::Quake3,
                index as u32,
                index as u32,
                self.text(index),
            ) else {
                continue;
            };
            publication.published(index, Some(sequence));
            publication.server_info[index] = Some(revision);
            system_published |= index == ConfigLayout::Q3_SYSTEM_INFO;
        }
        system_published
    }
    pub fn prepare(
        &mut self,
        cvars: &Cvars,
        vfs: &Vfs,
        source: Option<&SearchView>,
        source_id: Option<FileSearchId>,
    ) -> bool {
        self.ready = false;
        if source.is_some_and(|source| !vfs.read_view(None).accepts_search(source)) {
            return false;
        }
        let view = vfs.read_view(source);
        if let Some(map) = self.map {
            let key = (source_id, cvars.view_generation(map));
            if self.map_seen != Some(key) {
                // The map was parsed before the program's reference account
                // existed. Add its selected source without rereading the BSP.
                if let Some(path) = cvars
                    .read(map)
                    .ok()
                    .filter(|name| !name.as_str().is_empty())
                    .and_then(|name| crate::map::virtual_path(name.as_str()).ok())
                {
                    let _ = view.open(path.as_bytes());
                }
                self.map_seen = Some(key);
            }
        }
        let Ok(references) = view.q3_reference_revision() else {
            return false;
        };
        let native = qa_compat::services::userinfo_rules(RuleSetId::Quake3);
        for (index, flag) in [4, 8].into_iter().enumerate() {
            let key = (
                cvars.info_generation(RuleSetId::Quake3, flag),
                if index == 1 { vfs.revision() } else { 0 },
                if index == 1 { references } else { 0 },
                source_id,
            );
            if self.rows[index].prepared == Some(key) {
                continue;
            }
            let rules = if index == ConfigLayout::Q3_SYSTEM_INFO {
                qa_core::text::InfoRules {
                    bytes: MAX_TEXT,
                    key_bytes: MAX_TEXT,
                    value_bytes: MAX_TEXT,
                    ..native
                }
            } else {
                native
            };
            let mut bytes = [0; MAX_TEXT];
            let Ok(mut length) = cvars.info_string(self.context, flag, rules, &mut bytes) else {
                return false;
            };
            if index == ConfigLayout::Q3_SYSTEM_INFO {
                let pure = qa_network::pure::enabled(&bytes[..length]);
                if view
                    .q3_package_info(pure, &mut bytes, &mut length, rules, &mut self.mounts)
                    .is_err()
                {
                    return false;
                }
                self.pure = pure;
            }
            let row = &mut self.rows[index];
            if row.text.as_bytes() != &bytes[..length] {
                if row.text.set_bytes(&bytes[..length]).is_err() {
                    return false;
                }
                row.revision = row.revision.wrapping_add(1);
            }
            row.prepared = Some(key);
        }
        self.ready = true;
        true
    }
}

#[cfg(test)]
#[path = "../../tests/unit/server_info.rs"]
mod tests;
