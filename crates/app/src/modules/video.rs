use super::Phase;
use crate::{
    host::FrameHost,
    model_assets::{ImageRefresh, ModelProfile},
    renderer::native::SceneVideoSelection,
};
use qa_core::primitives::{EntityId, ModuleId};
use qa_render::world::{WorldId, WorldPresentationId};

pub(crate) struct VideoImageRoles {
    rows: Vec<(ModuleId, EntityId, SceneVideoSelection)>,
}

impl FrameHost {
    /// Capture selected resources before old Shutdown clears per-instance worlds.
    pub(crate) fn video_image_selections(
        &self,
        profiles: &mut Vec<ModelProfile>,
        presentations: &mut Vec<(WorldId, WorldPresentationId)>,
    ) {
        let Some(modules) = &self.modules else { return };
        let resources = self.runtime.model_assets.as_ref().map(|owner| owner.view());
        for row in modules.rows.iter().flatten() {
            if row.request.phase != Phase::Client {
                continue;
            }
            if let Some(profile) = row.scene_role.profile()
                && !profiles.contains(&profile)
            {
                profiles.push(profile);
            }
            if let Some(world) = resources.and_then(|view| row.scene_role.world_selection(view))
                && !presentations.contains(&world)
            {
                presentations.push(world);
            }
        }
    }

    /// The cold transaction has no guest callbacks between staging and commit.
    pub(crate) fn stage_video_image_roles(
        &self,
        refresh: &ImageRefresh,
        width: u32,
        height: u32,
    ) -> Result<VideoImageRoles, String> {
        let seats = std::array::from_fn(|seat| self.local_seat(seat).map(|(client, _, _)| client));
        let mut rows = Vec::new();
        if let Some(modules) = &self.modules {
            for row in modules.rows.iter().flatten() {
                if row.request.phase != Phase::Client {
                    continue;
                }
                if row.request.program.is_active() {
                    return Err("video image selection has an active CLIENT module".to_owned());
                }
                rows.push((
                    row.request.context.module,
                    row.request.anchor,
                    row.scene_role
                        .stage_video_selection(refresh, width, height, &seats),
                ));
            }
        }
        Ok(VideoImageRoles { rows })
    }

    pub(crate) fn commit_video_image_roles(&mut self, staged: VideoImageRoles) {
        for (module, anchor, selection) in staged.rows {
            if let Some(row) = self
                .module_row_mut(module)
                .filter(|row| row.request.anchor == anchor)
            {
                row.scene_role.publish_video_selection(selection);
            }
        }
    }
}
