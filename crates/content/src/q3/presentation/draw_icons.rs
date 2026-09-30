//! Quake III presentation: draw icons.
//!
//! Donor provenance: `src/content/q3/presentation/draw-icons.ts`.

use qa_core::math::{add3, vec3, vec4, Bounds, Vec3};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::items::{find_item_for_powerup, item_list};
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::hud::Shared;
use crate::q3::presentation::ref_entity::RF_NOSHADOW;
use crate::q3::presentation::refdef::RDF_NOWORLDMODEL;
use crate::q3::presentation::retail_snapshot::{
    create_model_entity_with, create_refdef, RefEntity, SceneModel, SceneSkin,
};
use crate::q3::presentation::state::*;
use qa_core::math::angles_to_axis;

/// Draw-icon settings (`ClientDrawIconSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientDrawIconSettings {
    /// Draw icons.
    pub draw_icons: bool,
    /// Draw 3D icons.
    pub draw_3d_icons: bool,
}

/// Icon origin for bounds (`iconOrigin`).
pub(crate) fn icon_origin(bounds: &Bounds, fraction: f32) -> Vec3 {
    let length = fraction * (bounds.max.z - bounds.min.z);
    vec3(
        length / 0.268,
        0.5 * (bounds.min.y + bounds.max.y),
        -0.5 * (bounds.min.z + bounds.max.z),
    )
}

/// 3D icon drawing (`ClientDrawIcons`).
pub struct ClientDrawIcons {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Draw tools.
    pub tools: ClientDrawTools,
    /// Settings reader.
    pub settings: Rc<dyn Fn() -> ClientDrawIconSettings>,
    /// Command queue context (must share the tools queue).
    pub commands: Draw2D,
}

impl ClientDrawIcons {
    /// Assemble draw icons.
    pub fn new(
        state: Shared<ClientGameState>,
        tools: ClientDrawTools,
        settings: Rc<dyn Fn() -> ClientDrawIconSettings>,
        commands: Draw2D,
    ) -> Self {
        if !tools.draw.shares_queue(&commands) {
            panic!("Draw icons and command buffer must share the engine drawing queue");
        }
        if state.borrow().product != tools.media.borrow().product {
            panic!("Draw icons and media products differ");
        }
        Self {
            state,
            tools,
            settings,
            commands,
        }
    }

    /// Draw a 3D model (`draw3DModel`).
    pub fn draw_3d_model(&self, rect: Rect2d, model: &SceneModel, skin: Option<SceneSkin>, origin: Vec3, angles: Vec3) {
        let settings = (self.settings)();
        if !settings.draw_3d_icons || !settings.draw_icons {
            return;
        }
        let viewport = self.tools.adjust_from_640(rect);
        let mut refdef = create_refdef();
        let mut entity = create_model_entity_with(model.clone());
        entity.axis = angles_to_axis(angles);
        entity.origin = vec3(origin.x, origin.y, origin.z);
        entity.custom_skin = skin;
        entity.render_flags = RF_NOSHADOW;
        refdef.render_flags = RDF_NOWORLDMODEL;
        refdef.view_axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
        refdef.fov_x = 30.0;
        refdef.fov_y = 30.0;
        refdef.time = self.state.borrow().time;
        refdef.x = viewport.x.trunc() as i32;
        refdef.y = viewport.y.trunc() as i32;
        refdef.width = viewport.width.trunc() as i32;
        refdef.height = viewport.height.trunc() as i32;
        // ClearScene starts an empty scene without discarding previously queued draw commands.
        let resources = self.tools.media.borrow().resources.clone();
        resources.borrow_mut().clear_scene();
        resources.borrow_mut().add_ref_entity(RefEntity::Model(entity));
        resources.borrow_mut().render_scene(&refdef);
    }

    /// Draw a head (`drawHead`).
    pub fn draw_head(&self, rect: Rect2d, client_num: i32, head_angles: Vec3) {
        let static_state = self.tools.media.borrow().static_state.clone();
        let client = static_state
            .borrow()
            .client_info
            .get(client_num as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("CG_DrawHead: invalid client number");
            });
        if client_num < 0 {
            panic!("CG_DrawHead: invalid client number");
        }
        let settings = (self.settings)();
        if settings.draw_3d_icons {
            if client.head_model.is_default() {
                return;
            }
            // Canonical client assets resolve to renderer views through the
            // shared resource host's handle table.
            let resources = self.tools.media.borrow().resources.clone();
            let model = client
                .head_model
                .resource_id()
                .map(|id| resources.borrow().model_for_handle(id as i32).unwrap_or_default())
                .unwrap_or_default();
            let bounds = resources.borrow().model_bounds(&model);
            let origin = add3(icon_origin(&bounds, 0.7), client.head_offset);
            let skin = client.head_skin.clone();
            let deferred = client.deferred;
            drop(client);
            self.draw_3d_model(rect, &model, skin, origin, head_angles);
            if deferred {
                let defer = self.tools.media.borrow().graphics.defer_shader.clone();
                self.tools.draw_pic(rect, &defer);
            }
        } else {
            if settings.draw_icons {
                let icon = client.model_icon.clone().and_then(|shader| {
                    self.tools
                        .media
                        .borrow()
                        .resources
                        .borrow()
                        .shader_for_handle(shader.id as i32)
                        .unwrap_or(None)
                });
                drop(client);
                self.tools.draw_pic(rect, &icon);
                let deferred = static_state.borrow().client_info[client_num as usize].deferred;
                if deferred {
                    let defer = self.tools.media.borrow().graphics.defer_shader.clone();
                    self.tools.draw_pic(rect, &defer);
                }
            } else {
                let deferred = client.deferred;
                drop(client);
                if deferred {
                    let defer = self.tools.media.borrow().graphics.defer_shader.clone();
                    self.tools.draw_pic(rect, &defer);
                }
            }
        }
    }

    /// Draw a flag model (`drawFlagModel`).
    pub fn draw_flag_model(&self, rect: Rect2d, team: i32, force_2d: bool) {
        let settings = (self.settings)();
        let media = self.tools.media.borrow();
        if !force_2d && settings.draw_3d_icons {
            let bounds = media.resources.borrow().model_bounds(&media.graphics.red_flag_model);
            let origin = icon_origin(&bounds, 0.5);
            let time = self.state.borrow().time;
            let angles = vec3(0.0, 60.0 * (time as f32 / 2000.0).sin(), 0.0);
            let model = if team == Team::TeamRed as i32 {
                media.graphics.red_flag_model.clone()
            } else if team == Team::TeamBlue as i32 {
                media.graphics.blue_flag_model.clone()
            } else if team == Team::TeamFree as i32 {
                media.graphics.neutral_flag_model.clone()
            } else {
                return;
            };
            drop(media);
            self.draw_3d_model(rect, &model, None, origin, angles);
        } else if settings.draw_icons {
            let powerup = if team == Team::TeamRed as i32 {
                Powerup::PwRedflag
            } else if team == Team::TeamBlue as i32 {
                Powerup::PwBlueflag
            } else if team == Team::TeamFree as i32 {
                Powerup::PwNeutralflag
            } else {
                return;
            };
            let product = media.product;
            let item = find_item_for_powerup(product, powerup);
            if let Some(item) = item {
                let Some(index) = item_list(product)
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate as *const _, item as *const _))
                else {
                    return;
                };
                let visual = media.weapon_registry.borrow().item_visual(index);
                drop(media);
                self.tools.draw_pic(rect, &visual.icon);
            }
        }
    }

    /// Draw a team background (`drawTeamBackground`).
    pub fn draw_team_background(&self, rect: Rect2d, alpha: f32, team: i32) {
        if team != Team::TeamRed as i32 && team != Team::TeamBlue as i32 {
            return;
        }
        self.tools.draw.set_color(Some(vec4(
            if team == Team::TeamRed as i32 { 1.0 } else { 0.0 },
            0.0,
            if team == Team::TeamBlue as i32 { 1.0 } else { 0.0 },
            alpha,
        )));
        let bar = self.tools.media.borrow().graphics.team_status_bar.clone();
        self.tools.draw_pic(
            rect2d(rect.x.trunc(), rect.y.trunc(), rect.width.trunc(), rect.height.trunc()),
            &bar,
        );
        self.tools.draw.set_color(None);
    }
}
