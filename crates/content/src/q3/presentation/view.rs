//! Quake III presentation: view.
//!
//! Donor provenance: `src/content/q3/presentation/view.ts`.

use qa_core::math::{add3, angle_vectors, angles_to_axis, dot3, scale3, sub3, vec3, vec4, Bounds, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_scene::*;
use crate::q3::presentation::ref_entity::*;
use crate::q3::presentation::refdef::*;

// ---------------------------------------------------------------------------
// view.ts
// ---------------------------------------------------------------------------

/// View settings (`ViewSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    /// Video width.
    pub video_width: i32,
    /// Video height.
    pub video_height: i32,
    /// View size.
    pub view_size: i32,
    /// Third person.
    pub third_person: bool,
    /// Third-person range.
    pub third_person_range: f32,
    /// Third-person angle.
    pub third_person_angle: f32,
    /// Camera mode.
    pub camera_mode: bool,
    /// Camera orbit integer.
    pub camera_orbit_integer: i32,
    /// Camera orbit value.
    pub camera_orbit_value: f32,
    /// Camera orbit delay.
    pub camera_orbit_delay: i32,
    /// Error decay.
    pub error_decay: f32,
    /// Run pitch.
    pub run_pitch: f32,
    /// Run roll.
    pub run_roll: f32,
    /// Bob pitch.
    pub bob_pitch: f32,
    /// Bob roll.
    pub bob_roll: f32,
    /// Bob up.
    pub bob_up: f32,
    /// FOV.
    pub fov: f32,
    /// Zoom FOV.
    pub zoom_fov: f32,
    /// DM flags.
    pub dm_flags: i32,
    /// Gun X.
    pub gun_x: f32,
    /// Gun Y.
    pub gun_y: f32,
    /// Gun Z.
    pub gun_z: f32,
}

/// View host (`ViewHost`).
pub trait ViewHost {
    /// Settings.
    fn settings(&self) -> ViewSettings;
    /// Set view size.
    fn set_view_size(&mut self, value: i32);
    /// Set third-person angle value.
    fn set_third_person_angle_value(&mut self, value: f32);
    /// Register a model.
    fn register_model(&mut self, path: &str) -> SceneModel;
    /// Print.
    fn print(&mut self, message: &str);
}

pub(crate) fn view_multiply_add(origin: Vec3, scale: f32, direction: Vec3) -> Vec3 {
    add3(origin, scale3(direction, scale))
}

/// View runtime (`ViewRuntime`).
pub struct ViewRuntime {
    /// Host.
    pub host: Box<dyn ViewHost>,
    model_revision: u64,
}

impl ViewRuntime {
    /// New runtime.
    #[must_use]
    pub fn new(host: Box<dyn ViewHost>) -> Self {
        Self {
            host,
            model_revision: 0,
        }
    }

    /// Calculate view values (`calculateViewValues`).
    pub fn calculate_view_values(
        &mut self,
        state: &mut ClientGameState,
        prediction: &dyn PresentPrediction,
    ) -> PresentResult<bool> {
        if state.snap.is_none() {
            return Err(PresentError::state("CG_CalcViewValues requires a current snapshot"));
        }
        let settings = self.host.settings();
        let ps = state.predicted_player_state.clone();
        state.refdef = create_refdef();
        let mut size = if state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.pm_type == MoveType::Intermission)
        {
            100
        } else {
            settings.view_size
        };
        if size < 30 {
            self.host.set_view_size(30);
            size = 30;
        } else if size > 100 {
            self.host.set_view_size(100);
            size = 100;
        }
        state.refdef.width = (settings.video_width.wrapping_mul(size) / 100) & !1;
        state.refdef.height = (settings.video_height.wrapping_mul(size) / 100) & !1;
        state.refdef.x = (settings.video_width - state.refdef.width) / 2;
        state.refdef.y = (settings.video_height - state.refdef.height) / 2;
        if state.refdef.width <= 0 || state.refdef.height <= 0 {
            return Err(PresentError::range("Camera view requires a positive viewport"));
        }
        state.rendering_third_person =
            settings.third_person || state.snap.as_ref().is_some_and(|snap| snap.player_state.health <= 0);
        state.refdef.view_origin = ps.origin;
        state.refdef_view_angles = ps.viewangles;
        if ps.pm_type == MoveType::Intermission {
            state.refdef.view_axis = angles_to_axis(state.refdef_view_angles);
            return self.calculate_fov(state, prediction, &settings);
        }
        state.bob_cycle = (ps.bob_cycle & 128) >> 7;
        state.bob_frac_sin = (((ps.bob_cycle & 127) as f32) / 127.0 * std::f32::consts::PI)
            .sin()
            .abs();
        state.xyspeed = (ps.velocity.x * ps.velocity.x + ps.velocity.y * ps.velocity.y).sqrt();
        let mut third_person_angle = settings.third_person_angle;
        if settings.camera_orbit_integer != 0 && state.time > state.next_orbit_time {
            state.next_orbit_time = state.time.wrapping_add(settings.camera_orbit_delay);
            third_person_angle += settings.camera_orbit_value;
            self.host.set_third_person_angle_value(third_person_angle);
        }
        if settings.error_decay > 0.0 {
            let elapsed = state.time.wrapping_sub(state.predicted_error_time);
            let factor = (settings.error_decay - elapsed as f32) / settings.error_decay;
            if factor > 0.0 && factor < 1.0 {
                state.refdef.view_origin = view_multiply_add(state.refdef.view_origin, factor, state.predicted_error);
            } else {
                state.predicted_error_time = 0;
            }
        }
        if state.rendering_third_person {
            self.offset_third_person(state, prediction, &settings, third_person_angle);
        } else {
            self.offset_first_person(state, &settings)?;
        }
        state.refdef.view_axis = angles_to_axis(state.refdef_view_angles);
        if state.hyperspace {
            state.refdef.render_flags |= RDF_NOWORLDMODEL | RDF_HYPERSPACE;
        }
        self.calculate_fov(state, prediction, &settings)
    }

    /// Finish the refdef (`finishRefdef`).
    pub fn finish_refdef(&mut self, state: &mut ClientGameState) -> PresentResult<Refdef> {
        if state.snap.is_none() {
            return Err(PresentError::state("Finishing the view requires a current snapshot"));
        }
        state.refdef.time = state.time;
        let mut mask = [0u8; 32];
        if let Some(snap) = &state.snap {
            let length = snap.area_mask.len().min(32);
            mask[..length].copy_from_slice(&snap.area_mask[..length]);
        }
        state.refdef.area_mask = mask;
        Ok(copy_refdef(&state.refdef))
    }

    /// Zoom down.
    pub fn zoom_down(&mut self, state: &mut ClientGameState) {
        if state.zoomed {
            return;
        }
        state.zoomed = true;
        state.zoom_time = state.time;
    }

    /// Zoom up.
    pub fn zoom_up(&mut self, state: &mut ClientGameState) {
        if !state.zoomed {
            return;
        }
        state.zoomed = false;
        state.zoom_time = state.time;
    }

    fn offset_third_person(
        &mut self,
        state: &mut ClientGameState,
        prediction: &dyn PresentPrediction,
        settings: &ViewSettings,
        angle: f32,
    ) {
        let ps = state.predicted_player_state.clone();
        state.refdef.view_origin = vec3(
            state.refdef.view_origin.x,
            state.refdef.view_origin.y,
            state.refdef.view_origin.z + ps.viewheight,
        );
        let mut focus_angles = state.refdef_view_angles;
        if ps.health <= 0 {
            let yaw = ps.stats.get(stat_schema(ps.product).dead_yaw) as f32;
            focus_angles = vec3(focus_angles.x, yaw, focus_angles.z);
            state.refdef_view_angles = vec3(state.refdef_view_angles.x, yaw, state.refdef_view_angles.z);
        }
        if focus_angles.x > 45.0 {
            focus_angles = vec3(45.0, focus_angles.y, focus_angles.z);
        }
        let mut focus_point = view_multiply_add(state.refdef.view_origin, 512.0, angle_vectors(focus_angles).forward);
        let mut view = vec3(
            state.refdef.view_origin.x,
            state.refdef.view_origin.y,
            state.refdef.view_origin.z + 8.0,
        );
        state.refdef_view_angles = vec3(
            state.refdef_view_angles.x * 0.5,
            state.refdef_view_angles.y,
            state.refdef_view_angles.z,
        );
        let vectors = angle_vectors(state.refdef_view_angles);
        let radians = angle / 180.0 * std::f32::consts::PI;
        view = view_multiply_add(view, -settings.third_person_range * radians.cos(), vectors.forward);
        view = view_multiply_add(view, -settings.third_person_range * radians.sin(), vectors.right);
        if !settings.camera_mode {
            let bounds = Bounds {
                min: vec3(-4.0, -4.0, -4.0),
                max: vec3(4.0, 4.0, 4.0),
            };
            let trace = prediction.trace_mover(state.refdef.view_origin, view, bounds, ps.client_num, 1);
            if trace.base.fraction != 1.0 {
                view = vec3(
                    trace.base.end.x,
                    trace.base.end.y,
                    trace.base.end.z + (1.0 - trace.base.fraction) * 32.0,
                );
                view = prediction
                    .trace_mover(state.refdef.view_origin, view, bounds, ps.client_num, 1)
                    .base
                    .end;
            }
        }
        state.refdef.view_origin = view;
        focus_point = sub3(focus_point, view);
        let distance = (focus_point.x * focus_point.x + focus_point.y * focus_point.y)
            .sqrt()
            .max(1.0);
        state.refdef_view_angles = vec3(
            -180.0 / std::f32::consts::PI * focus_point.z.atan2(distance),
            state.refdef_view_angles.y - angle,
            state.refdef_view_angles.z,
        );
    }

    fn offset_first_person(&mut self, state: &mut ClientGameState, settings: &ViewSettings) -> PresentResult<()> {
        if state.snap.is_none() {
            return Err(PresentError::state("First-person offset requires a snapshot"));
        }
        if state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.pm_type == MoveType::Intermission)
        {
            return Ok(());
        }
        let ps = state.predicted_player_state.clone();
        if state.snap.as_ref().is_some_and(|snap| snap.player_state.health <= 0) {
            let yaw = state
                .snap
                .as_ref()
                .map(|snap| snap.player_state.stats.get(stat_schema(ps.product).dead_yaw) as f32)
                .unwrap_or(0.0);
            state.refdef_view_angles = vec3(-15.0, yaw, 40.0);
            state.refdef.view_origin = vec3(
                state.refdef.view_origin.x,
                state.refdef.view_origin.y,
                state.refdef.view_origin.z + ps.viewheight,
            );
            return Ok(());
        }
        let mut angles = add3(state.refdef_view_angles, state.kick_angles);
        if state.damage_time != 0.0 {
            let mut ratio = state.time as f32 - state.damage_time;
            if ratio < 100.0 {
                ratio /= 100.0;
            } else {
                ratio = 1.0 - (ratio - 100.0) / 400.0;
            }
            if state.time as f32 - state.damage_time < 100.0 || ratio > 0.0 {
                angles = vec3(
                    angles.x + ratio * state.damage_pitch,
                    angles.y,
                    angles.z + ratio * state.damage_roll,
                );
            }
        }
        angles = vec3(
            angles.x + dot3(ps.velocity, state.refdef.view_axis[0]) * settings.run_pitch,
            angles.y,
            angles.z - dot3(ps.velocity, state.refdef.view_axis[1]) * settings.run_roll,
        );
        let speed = state.xyspeed.max(200.0);
        let mut pitch = state.bob_frac_sin * settings.bob_pitch * speed;
        let mut roll = state.bob_frac_sin * settings.bob_roll * speed;
        if ps.pm_flags & MoveFlags::DUCKED != 0 {
            pitch *= 3.0;
            roll *= 3.0;
        }
        if state.bob_cycle & 1 != 0 {
            roll = -roll;
        }
        state.refdef_view_angles = vec3(angles.x + pitch, angles.y, angles.z + roll);
        let mut height = state.refdef.view_origin.z + ps.viewheight;
        let duck_delta = state.time.wrapping_sub(state.duck_time);
        if duck_delta < 100 {
            height -= state.duck_change * (100 - duck_delta) as f32 / 100.0;
        }
        height += (state.bob_frac_sin * state.xyspeed * settings.bob_up).min(6.0);
        let land_delta = state.time.wrapping_sub(state.land_time) as f32;
        if land_delta < 150.0 {
            height += state.land_change * (land_delta / 150.0);
        } else if land_delta < 450.0 {
            height += state.land_change * (1.0 - (land_delta - 150.0) / 300.0);
        }
        let step_delta = state.time.wrapping_sub(state.step_time);
        if step_delta < 200 {
            height -= state.step_change * (200 - step_delta) as f32 / 200.0;
        }
        state.refdef.view_origin = add3(
            vec3(state.refdef.view_origin.x, state.refdef.view_origin.y, height),
            state.kick_origin,
        );
        Ok(())
    }

    fn calculate_fov(
        &mut self,
        state: &mut ClientGameState,
        prediction: &dyn PresentPrediction,
        settings: &ViewSettings,
    ) -> PresentResult<bool> {
        let mut fov = 90.0f32;
        if state.predicted_player_state.pm_type != MoveType::Intermission {
            fov = if settings.dm_flags & 16 != 0 {
                90.0
            } else {
                settings.fov.clamp(1.0, 160.0)
            };
            let zoom = settings.zoom_fov.clamp(1.0, 160.0);
            let fraction = state.time.wrapping_sub(state.zoom_time) as f32 / 150.0;
            if state.zoomed {
                fov = if fraction > 1.0 {
                    zoom
                } else {
                    fov + fraction * (zoom - fov)
                };
            } else if fraction <= 1.0 {
                fov = zoom + fraction * (fov - zoom);
            }
        }
        let radians = fov / 360.0 * std::f32::consts::PI;
        let tangent = radians.sin() / radians.cos();
        let x = state.refdef.width as f32 / tangent;
        let mut vertical = (state.refdef.height as f32).atan2(x) * 360.0 / std::f32::consts::PI;
        let in_water = prediction.point_contents_pred(state.refdef.view_origin, -1) & (8 | 16 | 32) != 0;
        if in_water {
            let phase = state.time as f32 / 1000.0 * 0.4 * std::f32::consts::PI * 2.0;
            let wave = phase.sin();
            fov += wave;
            vertical -= wave;
        }
        state.refdef.fov_x = fov;
        state.refdef.fov_y = vertical;
        state.zoom_sensitivity = if state.zoomed { vertical / 75.0 } else { 1.0 };
        Ok(in_water)
    }

    /// Damage blend blob (`damageBlendBlob`).
    pub fn damage_blend_blob(
        &mut self,
        state: &ClientGameState,
        shader: Option<SceneShader>,
        rage_pro: bool,
    ) -> Option<RefSpriteEntity> {
        let elapsed = (state.time as f32 - state.damage_time).trunc() as i32;
        if state.damage_value == 0 || rage_pro || elapsed <= 0 || elapsed >= 500 {
            return None;
        }
        let mut entity = create_sprite_entity();
        entity.shading.render_flags = RF_FIRST_PERSON;
        entity.origin = view_multiply_add(state.refdef.view_origin, 8.0, state.refdef.view_axis[0]);
        entity.origin = view_multiply_add(entity.origin, state.damage_x * -8.0, state.refdef.view_axis[1]);
        entity.origin = view_multiply_add(entity.origin, state.damage_y * 8.0, state.refdef.view_axis[2]);
        entity.radius = state.damage_value as f32 * 3.0;
        entity.shading.custom_shader = shader;
        entity.shading.shader_rgba = vec4(
            255.0,
            255.0,
            255.0,
            ((200.0 * (1.0 - elapsed as f32 / 500.0)).trunc() as i32 & 255) as f32,
        );
        Some(entity)
    }

    /// Clear the test model (`clearTestModel`).
    pub fn clear_test_model(&mut self, state: &mut ClientGameState) {
        self.model_revision += 1;
        state.test_model_name = String::new();
        state.test_model_entity = create_model_entity(default_model());
        state.test_gun = false;
    }

    /// Test a model (`testModel`).
    pub fn test_model(&mut self, state: &mut ClientGameState, name: Option<&str>, back_lerp: Option<f32>) {
        self.request_test_model(state, name, back_lerp, false);
    }

    /// Test a gun (`testGun`).
    pub fn test_gun(&mut self, state: &mut ClientGameState, name: Option<&str>, back_lerp: Option<f32>) {
        self.request_test_model(state, name, back_lerp, true);
    }

    fn request_test_model(
        &mut self,
        state: &mut ClientGameState,
        name: Option<&str>,
        back_lerp: Option<f32>,
        gun: bool,
    ) {
        self.model_revision += 1;
        let mut entity = create_model_entity(default_model());
        if let Some(name) = name {
            let nul = name.find('\0');
            let end = nul.map_or(63.min(name.len()), |index| index.min(63));
            state.test_model_name = name[..end].to_string();
            let model = self.host.register_model(&state.test_model_name.clone());
            entity.model = model.clone();
            if let Some(back_lerp) = back_lerp {
                entity.back_lerp = back_lerp;
                entity.frame = 1;
            }
            if model.is_default() {
                self.host.print("Can't register model\n");
            } else {
                entity.origin = view_multiply_add(state.refdef.view_origin, 100.0, state.refdef.view_axis[0]);
                entity.axis = angles_to_axis(vec3(0.0, 180.0 + state.refdef_view_angles.y, 0.0));
                state.test_gun = false;
            }
        }
        state.test_model_entity = entity;
        if gun {
            state.test_gun = true;
            state.test_model_entity.shading.render_flags = RF_MINLIGHT | RF_DEPTHHACK | RF_FIRST_PERSON;
        }
    }

    /// Next model frame.
    pub fn next_model_frame(&mut self, state: &mut ClientGameState) {
        state.test_model_entity.frame = state.test_model_entity.frame.wrapping_add(1);
        let frame = state.test_model_entity.frame;
        self.host.print(&format!("frame {frame}\n"));
    }

    /// Previous model frame.
    pub fn previous_model_frame(&mut self, state: &mut ClientGameState) {
        state.test_model_entity.frame = 0.max(state.test_model_entity.frame.wrapping_sub(1));
        let frame = state.test_model_entity.frame;
        self.host.print(&format!("frame {frame}\n"));
    }

    /// Next model skin.
    pub fn next_model_skin(&mut self, state: &mut ClientGameState) {
        state.test_model_entity.skin_num = state.test_model_entity.skin_num.wrapping_add(1);
        let skin = state.test_model_entity.skin_num;
        self.host.print(&format!("skin {skin}\n"));
    }

    /// Previous model skin.
    pub fn previous_model_skin(&mut self, state: &mut ClientGameState) {
        state.test_model_entity.skin_num = 0.max(state.test_model_entity.skin_num.wrapping_sub(1));
        let skin = state.test_model_entity.skin_num;
        self.host.print(&format!("skin {skin}\n"));
    }

    /// Add the test model (`addTestModel`).
    pub fn add_test_model(&mut self, state: &mut ClientGameState) -> Option<RefEntity> {
        if state.test_model_entity.model.is_default() {
            return None;
        }
        let model = self.host.register_model(&state.test_model_name.clone());
        state.test_model_entity.model = model.clone();
        if model.is_default() {
            self.host.print("Can't register model\n");
            return None;
        }
        if state.test_gun {
            let settings = self.host.settings();
            state.test_model_entity.axis = state.refdef.view_axis;
            state.test_model_entity.origin =
                view_multiply_add(state.refdef.view_origin, settings.gun_x, state.refdef.view_axis[0]);
            state.test_model_entity.origin = view_multiply_add(
                state.test_model_entity.origin,
                settings.gun_y,
                state.refdef.view_axis[1],
            );
            state.test_model_entity.origin = view_multiply_add(
                state.test_model_entity.origin,
                settings.gun_z,
                state.refdef.view_axis[2],
            );
        }
        Some(copy_ref_entity(&RefEntity::Model(state.test_model_entity.clone())))
    }
}
