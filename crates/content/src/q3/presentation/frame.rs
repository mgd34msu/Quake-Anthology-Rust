//! Quake III presentation: frame.
//!
//! Donor provenance: `src/content/q3/presentation/frame.ts`.

use qa_core::math::{add3, scale3, Axis, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_client::*;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::server_commands::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Presentation frame loop (frame.ts)
// ---------------------------------------------------------------------------

/// Frame stereo eye.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStereo {
    /// Center eye.
    Center,
    /// Left eye.
    Left,
    /// Right eye.
    Right,
}

/// Presentation frame input (`Q3PresentationFrame`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3PresentationFrame {
    /// Server time in milliseconds.
    pub server_time: i32,
    /// Stereo eye.
    pub stereo: FrameStereo,
    /// Demo playback.
    pub demo_playback: bool,
    /// Engine frame number.
    pub engine_frame_number: i32,
}

/// Packet entity options (`PacketEntityOptions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketEntityOptions {
    /// Game type.
    pub game_type: GameType,
    /// Smooth clients.
    pub smooth_clients: bool,
    /// Simple items.
    pub simple_items: bool,
    /// Mission-pack obelisk respawn delay.
    pub obelisk_respawn_delay: i32,
}

/// Supplemental scene camera (`SceneCamera`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneCamera {
    /// Camera origin.
    pub origin: Vec3,
    /// Camera axis.
    pub axis: Axis,
    /// Projection matrix in column-major order.
    pub projection: [f32; 16],
    /// Viewport X.
    pub viewport_x: i32,
    /// Viewport Y.
    pub viewport_y: i32,
    /// Viewport width.
    pub viewport_width: i32,
    /// Viewport height.
    pub viewport_height: i32,
}

/// Frame-loop cvar names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameCvar {
    /// Stereo separation.
    CgStereoSeparation,
    /// Frame statistics print.
    CgStats,
    /// Timescale fade end.
    CgTimescaleFadeEnd,
    /// Timescale fade speed.
    CgTimescaleFadeSpeed,
    /// Timescale.
    CgTimescale,
}

impl FrameCvar {
    /// Source cvar name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CgStereoSeparation => "cg_stereoSeparation",
            Self::CgStats => "cg_stats",
            Self::CgTimescaleFadeEnd => "cg_timescaleFadeEnd",
            Self::CgTimescaleFadeSpeed => "cg_timescaleFadeSpeed",
            Self::CgTimescale => "cg_timescale",
        }
    }
}

/// Scene-subset frame host (`Q3PresentationSceneHost`).
///
/// Snapshots, packets, marks, particles, local entities, frame audio, voice
/// chats, and scene submission arrive through these methods; donor `async`
/// boundaries are synchronous here.
pub trait Q3PresentationSceneHost {
    /// Refresh configuration cvars.
    fn update_cvars(&mut self) -> PresentResult<()>;
    /// Register a required weapon.
    fn register_weapon(&mut self, weapon: i32) -> PresentResult<()>;
    /// Process snapshots.
    fn process_snapshots(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()>;
    /// Current packet presentation settings.
    fn packet_options(&self, static_state: &ClientGameStaticState) -> PacketEntityOptions;
    /// Add packet entities.
    fn add_packet_entities(
        &mut self,
        state: &mut ClientGameState,
        static_state: &ClientGameStaticState,
        options: &PacketEntityOptions,
    );
    /// Drain impact-mark polygons.
    fn poll_impact_marks(&mut self, state: &ClientGameState) -> Vec<RefPoly>;
    /// Drain particle polygons.
    fn poll_particles(&mut self, state: &ClientGameState) -> Vec<RefPoly>;
    /// Add local entities.
    fn add_local_entities(&mut self, state: &ClientGameState);
    /// Play buffered sounds.
    fn play_buffered_sounds(&mut self, state: &mut ClientGameState);
    /// Play buffered voice chats.
    fn play_buffered_voice_chats(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()>;
    /// Submit a scene polygon.
    fn add_poly(&mut self, poly: RefPoly);
    /// Submit a scene entity.
    fn add_ref_entity(&mut self, entity: RefEntity);
    /// Clear the scene.
    fn clear_scene(&mut self);
    /// Render the state's view definition.
    fn render_scene(&mut self, state: &ClientGameState);
    /// Enter a frame.
    fn enter_frame(&mut self, frame: &Q3PresentationFrame);
    /// Clear looping sounds.
    fn clear_looping_sounds(&mut self, kill_all: bool);
}

/// Full frame host (`Q3PresentationFrameHost`).
pub trait Q3PresentationFrameHost: Q3PresentationSceneHost {
    /// Whether the hardware is a Rage Pro.
    fn hardware_is_rage_pro(&self) -> bool;
    /// Predict the player state.
    fn predict_player_state(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()>;
    /// Calculate view values.
    fn calculate_view_values(&mut self, state: &mut ClientGameState);
    /// Damage blend blob entity, if any.
    fn damage_blend_blob(&mut self, state: &ClientGameState, rage_pro: bool) -> Option<RefEntity>;
    /// Recipe pre-hook for the view weapon; `false` skips it.
    fn pre_present_view_weapon(&mut self, state: &ClientGameState) -> bool;
    /// Add the view weapon.
    fn add_view_weapon(&mut self, state: &mut ClientGameState);
    /// Recipe post-hook for the view weapon.
    fn post_present_view_weapon(&mut self, state: &mut ClientGameState);
    /// Test model entity, if any.
    fn add_test_model(&mut self, state: &ClientGameState) -> PresentResult<Option<RefEntity>>;
    /// Finish the state's view definition.
    fn finish_refdef(&mut self, state: &mut ClientGameState);
    /// Powerup timer sounds.
    fn powerup_timer_sounds(&mut self, state: &ClientGameState);
    /// Set the audio listener.
    fn set_listener(&mut self, client: i32, origin: Vec3, axis: Axis);
    /// Record lagometer frame info.
    fn add_lagometer_frame_info(&mut self, state: &ClientGameState);
    /// Read a frame-loop cvar.
    fn read_frame_cvar(&self, name: FrameCvar) -> CvarSnapshot;
    /// Write the timescale cvar.
    fn set_timescale_cvar(&mut self, value: f32);
    /// Apply the engine timescale.
    fn set_timescale(&mut self, value: f32);
    /// Set the user command value.
    fn set_user_command_value(&mut self, weapon: i32, sensitivity: f32);
    /// Draw a loading frame.
    fn loading_frame(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()>;
    /// Draw the tournament scoreboard.
    fn draw_tourney_scoreboard(&mut self, state: &mut ClientGameState, static_state: &mut ClientGameStaticState);
    /// Clear the view border.
    fn tile_clear(&mut self, state: &ClientGameState);
    /// Draw 2D elements.
    fn draw_2d(&mut self, state: &mut ClientGameState, static_state: &mut ClientGameStaticState) -> PresentResult<()>;
    /// Diagnostic print.
    fn print(&mut self, text: &str);
}

/// Frame-loop scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameScope {
    Primary,
    Scene,
}

/// Presentation frame runtime (`Q3PresentationFrameRuntime`).
///
/// Frame presentation never advances simulation or reads another seat's input
/// queue. Synchronous completion order replaces the donor's reentrancy guards
/// against interleaved async work.
pub struct Q3PresentationFrameRuntime<H> {
    /// Host services.
    pub host: H,
    scope: FrameScope,
    drawing: bool,
    scene_drawing: bool,
    closed: bool,
}

impl<H: Q3PresentationSceneHost> Q3PresentationFrameRuntime<H> {
    /// New supplemental scene runtime.
    #[must_use]
    pub fn new_scene(host: H) -> Self {
        Self {
            host,
            scope: FrameScope::Scene,
            drawing: false,
            scene_drawing: false,
            closed: false,
        }
    }

    /// Retire the presentation.
    pub fn close(&mut self) -> PresentResult<()> {
        if self.drawing && !self.scene_drawing {
            return Err(state_msg("Cannot retire cgame during a frame"));
        }
        self.closed = true;
        self.host.clear_scene();
        Ok(())
    }

    /// Draw a supplemental scene frame.
    pub fn draw_scene_frame(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        input: &Q3PresentationFrame,
        camera: &SceneCamera,
    ) -> PresentResult<()> {
        if self.scope != FrameScope::Scene {
            return Err(state_msg("Primary cgame requires its full frame"));
        }
        if self.closed || self.drawing {
            return Err(state_msg("Cgame frame is closed or already drawing"));
        }
        self.drawing = true;
        self.scene_drawing = true;
        let result = self.scene_frame(state, static_state, input, camera);
        self.drawing = false;
        self.scene_drawing = false;
        result
    }

    fn scene_frame(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        input: &Q3PresentationFrame,
        camera: &SceneCamera,
    ) -> PresentResult<()> {
        if input.server_time < state.old_time {
            return Err(state_msg("Supplemental cgame source time moved backward"));
        }
        state.time = input.server_time;
        self.host.enter_frame(input);
        self.host.update_cvars()?;
        self.host.clear_looping_sounds(false);
        self.host.clear_scene();
        self.host.process_snapshots(state, static_state)?;
        let snapshot = match state.snap.clone() {
            None => return Ok(()),
            Some(snapshot) if snapshot.flags & 2 != 0 => return Ok(()),
            Some(snapshot) => snapshot,
        };
        state.predicted_player_state = snapshot.player_state.clone();
        state.physics_time = snapshot.server_time;
        state.frame_time = state.time.wrapping_sub(state.old_time).max(0);
        state.old_time = state.time;
        state.client_frame = state.client_frame.wrapping_add(1);
        state.refdef.view_origin = camera.origin;
        state.refdef.view_axis = camera.axis;
        state.refdef.x = camera.viewport_x;
        state.refdef.y = camera.viewport_y;
        state.refdef.width = camera.viewport_width;
        state.refdef.height = camera.viewport_height;
        state.refdef.fov_x = ((1.0f64 / f64::from(camera.projection[0])).atan() * 360.0 / std::f64::consts::PI) as f32;
        state.refdef.fov_y = ((1.0f64 / f64::from(camera.projection[5])).atan() * 360.0 / std::f64::consts::PI) as f32;
        state.refdef.time = state.time;
        let count = weapon_count(state.product);
        let mut required = vec![state.predicted_player_state.weapon];
        for entity in &snapshot.entities {
            if entity.e_type == EntityType::Player as i32
                || entity.e_type == EntityType::Missile as i32
                || entity.e_type == EntityType::Grapple as i32
            {
                let weapon = if entity.weapon > count { 0 } else { entity.weapon };
                if !required.contains(&weapon) {
                    required.push(weapon);
                }
            }
        }
        for weapon in required {
            self.host.register_weapon(weapon)?;
        }
        let options = self.host.packet_options(static_state);
        self.host.add_packet_entities(state, static_state, &options);
        for poly in self.host.poll_impact_marks(state) {
            self.host.add_poly(poly);
        }
        for poly in self.host.poll_particles(state) {
            self.host.add_poly(poly);
        }
        self.host.add_local_entities(state);
        self.host.play_buffered_sounds(state);
        self.host.play_buffered_voice_chats(state, static_state)?;
        Ok(())
    }
}

impl<H: Q3PresentationFrameHost> Q3PresentationFrameRuntime<H> {
    /// New primary frame runtime.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self {
            host,
            scope: FrameScope::Primary,
            drawing: false,
            scene_drawing: false,
            closed: false,
        }
    }

    /// Draw the active frame.
    pub fn draw_active_frame(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        input: &Q3PresentationFrame,
    ) -> PresentResult<()> {
        if self.scope != FrameScope::Primary {
            return Err(state_msg("Supplemental cgame cannot draw the primary frame"));
        }
        if self.closed || self.drawing {
            return Err(state_msg("Cgame frame is closed or already drawing"));
        }
        self.drawing = true;
        let result = self.frame(state, static_state, input);
        self.drawing = false;
        result
    }

    fn frame(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        frame: &Q3PresentationFrame,
    ) -> PresentResult<()> {
        state.time = frame.server_time;
        self.host.enter_frame(frame);
        self.host.update_cvars()?;
        if !state.info_screen_text.is_empty() {
            return self.host.loading_frame(state, static_state);
        }
        self.host.clear_looping_sounds(false);
        self.host.clear_scene();
        self.host.process_snapshots(state, static_state)?;
        let snapshot = match state.snap.clone() {
            None => return self.host.loading_frame(state, static_state),
            Some(snapshot) if snapshot.flags & 2 != 0 => {
                return self.host.loading_frame(state, static_state);
            }
            Some(snapshot) => snapshot,
        };
        self.host
            .set_user_command_value(state.weapon_select, state.zoom_sensitivity);
        state.client_frame = state.client_frame.wrapping_add(1);
        self.host.predict_player_state(state, static_state)?;
        self.host.calculate_view_values(state);
        let count = weapon_count(state.product);
        let mut required = vec![state.predicted_player_state.weapon];
        let owned = snapshot.player_state.stats.get(stat_schema(state.product).weapons)?;
        for weapon in 1..count {
            if owned & (1 << weapon) != 0 && !required.contains(&weapon) {
                required.push(weapon);
            }
        }
        for entity in &snapshot.entities {
            let current = state.entity_at(entity.number)?.current_state.clone();
            if current.e_type == EntityType::Player as i32 {
                if !required.contains(&current.weapon) {
                    required.push(current.weapon);
                }
            } else if current.e_type == EntityType::Missile as i32 || current.e_type == EntityType::Grapple as i32 {
                let weapon = if current.weapon > count { 0 } else { current.weapon };
                if !required.contains(&weapon) {
                    required.push(weapon);
                }
            }
        }
        for weapon in required {
            self.host.register_weapon(weapon)?;
        }
        if !state.rendering_third_person {
            let rage_pro = self.host.hardware_is_rage_pro();
            if let Some(damage) = self.host.damage_blend_blob(state, rage_pro) {
                self.host.add_ref_entity(damage);
            }
        }
        if !state.hyperspace {
            let options = self.host.packet_options(static_state);
            self.host.add_packet_entities(state, static_state, &options);
            for poly in self.host.poll_impact_marks(state) {
                self.host.add_poly(poly);
            }
            for poly in self.host.poll_particles(state) {
                self.host.add_poly(poly);
            }
            self.host.add_local_entities(state);
        }
        if self.host.pre_present_view_weapon(state) {
            self.host.add_view_weapon(state);
            self.host.post_present_view_weapon(state);
        }
        self.host.play_buffered_sounds(state);
        self.host.play_buffered_voice_chats(state, static_state)?;
        if !state.test_model_entity.model.is_default() {
            if let Some(model) = self.host.add_test_model(state)? {
                self.host.add_ref_entity(model);
            }
        }
        self.host.finish_refdef(state);
        self.host.powerup_timer_sounds(state);
        let (view_origin, view_axis) = (state.refdef.view_origin, state.refdef.view_axis);
        self.host
            .set_listener(snapshot.player_state.client_num, view_origin, view_axis);
        if frame.stereo != FrameStereo::Right {
            state.frame_time = state.time.wrapping_sub(state.old_time).max(0);
            state.old_time = state.time;
            self.host.add_lagometer_frame_info(state);
        }
        self.fade_timescale(state);
        let team = snapshot.player_state.persistant.get(PersistentIndex::Team as i32)?;
        if team == Team::Spectator as i32 && snapshot.player_state.pm_flags & MoveFlags::SCOREBOARD != 0 {
            self.host.draw_tourney_scoreboard(state, static_state);
            return Ok(());
        }
        let stereo = self.host.read_frame_cvar(FrameCvar::CgStereoSeparation).numeric_value;
        let separation = match frame.stereo {
            FrameStereo::Center => 0.0,
            FrameStereo::Left => stereo * -0.5,
            FrameStereo::Right => stereo * 0.5,
        };
        self.host.tile_clear(state);
        let base = state.refdef.view_origin;
        if separation != 0.0 {
            state.refdef.view_origin = add3(base, scale3(state.refdef.view_axis[1], -separation));
        }
        self.host.render_scene(state);
        state.refdef.view_origin = base;
        self.host.draw_2d(state, static_state)?;
        if self.host.read_frame_cvar(FrameCvar::CgStats).integer_value != 0 {
            self.host.print(&format!("cg.clientFrame:{}\n", state.client_frame));
        }
        Ok(())
    }

    fn fade_timescale(&mut self, state: &mut ClientGameState) {
        let end = self.host.read_frame_cvar(FrameCvar::CgTimescaleFadeEnd).numeric_value;
        let speed = self.host.read_frame_cvar(FrameCvar::CgTimescaleFadeSpeed).numeric_value;
        let current = self.host.read_frame_cvar(FrameCvar::CgTimescale).numeric_value;
        if current == end {
            return;
        }
        let delta = speed * state.frame_time as f32 / 1000.0;
        let value = if current < end {
            end.min(current + delta)
        } else {
            end.max(current - delta)
        };
        self.host.set_timescale_cvar(value);
        if speed != 0.0 {
            self.host.set_timescale(value);
        }
    }
}
