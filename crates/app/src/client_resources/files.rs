//! Native package admission is a cold connection transition in the shared VFS.
use super::*;

impl ClientResources {
    /// The channel owns reliable bytes and retries; startup stays Loading while
    /// that same window is full, without retaining a second pending command.
    pub(crate) fn queue_q3_references(
        &self,
        vfs: &qa_content::vfs::Vfs,
        channel: &mut qa_network::channel::Channel,
    ) -> Result<bool, String> {
        let context = channel
            .command_context()
            .map_err(|error| format!("Q3 package report context: {error:?}"))?;
        let mut command = qa_core::text::FixedText::<
            { qa_network::message::COMMAND_STRING_BYTES - 2 },
        >::default();
        self.state
            .file_search()
            .q3_reference_command(vfs, context.server_id, self.state.q3_pure(), &mut command)
            .map_err(|error| format!("Q3 package report: {error}"))?;
        Self::queue_q3_control(channel, command.as_bytes())
    }

    fn queue_q3_control(
        channel: &mut qa_network::channel::Channel,
        command: &[u8],
    ) -> Result<bool, String> {
        match channel.queue_client_command(Protocol::Quake3_68, command) {
            Ok(_) => Ok(true),
            Err(qa_network::channel::TransmitError::Full) => Ok(false),
            Err(error) => Err(format!("Q3 control submission: {error:?}")),
        }
    }

    pub(crate) fn begin_q3_renderer_reset(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
    ) -> Result<(), String> {
        if self.startup.q3_renderer_reset() {
            self.state
                .file_search()
                .clear_q3_renderer_references(vfs)
                .map_err(|error| format!("Q3 renderer package references: {error}"))?;
        }
        Ok(())
    }

    pub(crate) fn queue_q3_renderer_reset(
        &mut self,
        channel: &mut qa_network::channel::Channel,
    ) -> Result<bool, String> {
        if Self::queue_q3_control(channel, b"vdr")? {
            self.startup.q3_renderer_reset_queued();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Derive every replacement from the requested namespace, never from the
    /// previous server's restricted view. Downloads finish before this freezes
    /// mount membership and starts a fresh native package-reference account.
    pub(crate) fn admit_q3_files(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        checksum_feed: u32,
        random: qa_core::random::Random,
    ) -> Result<(), String> {
        if self.startup.q3() != Some(startup::Q3Phase::Downloading) {
            return Err("Q3 files admitted outside package startup".into());
        }
        let info = self
            .state
            .configs
            .config(ConfigLayout::Q3_SYSTEM_INFO)
            .map_or(&b""[..], |row| row.text);
        let list = qa_core::text::info_value(info, b"sv_paks", false)
            .map_or(&b""[..], |range| &info[range]);
        let mut tokens = qa_formats::text::Tokens::words(list);
        let mut checksums = Vec::new();
        // FS_PureServerSetLoadedPaks retains at most MAX_SEARCH_PATHS words.
        while checksums.len() < qa_network::downloads::Q3_PACKAGE_PATHS {
            let Some(word) = tokens
                .next()
                .map_err(|error| format!("Q3 package list: {error:?}"))?
            else {
                break;
            };
            checksums.push(qa_core::text::atoi_prefix(word) as u32);
        }
        let policy = vfs
            .q3_search_view(Some(self.requested_files.view()), &checksums, false)
            .map_err(|error| format!("Q3 package admission: {error}"))?;
        let tracked = vfs
            .track_q3_references(Some(&policy), checksum_feed, random)
            .map_err(|error| format!("Q3 package references: {error}"))?;
        let files = vfs
            .bind_search(Some(&tracked))
            .map_err(|error| format!("Q3 client files: {error}"))?;
        self.state.files = files;
        self.state.sound_source = None;
        self.state.cues_prepared = false;
        self.state.player_sounds.fill(None);
        self.state.muzzle_sounds.clear();
        self.startup.q3_packages_ready();
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/client_files.rs"]
mod tests;
