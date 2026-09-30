//! Quake III presentation: frame.
//!
//! Donor provenance: `src/content/q3/presentation/frame.ts`.

use qa_core::math::{add3, scale3, Axis, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::player_state::MoveFlags;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::server_commands::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Presentation frame loop (frame.ts)
// ---------------------------------------------------------------------------

/// Weapons stat slot for a product.
fn weapons_slot(product: Product) -> usize {
    let slot = match stat_schema(product) {
        StatSchema::Base(layout) => layout.weapons,
        StatSchema::Missionpack(layout) => layout.weapons,
    };
    slot as usize
}

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
            if entity.e_type == EntityType::EtPlayer as i32
                || entity.e_type == EntityType::EtMissile as i32
                || entity.e_type == EntityType::EtGrapple as i32
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
        let owned = snapshot.player_state.stats.get(weapons_slot(state.product));
        for weapon in 1..count {
            if owned & (1 << weapon) != 0 && !required.contains(&weapon) {
                required.push(weapon);
            }
        }
        for entity in &snapshot.entities {
            let current = state.entity_at(entity.number)?.current_state.clone();
            if current.e_type == EntityType::EtPlayer as i32 {
                if !required.contains(&current.weapon) {
                    required.push(current.weapon);
                }
            } else if current.e_type == EntityType::EtMissile as i32 || current.e_type == EntityType::EtGrapple as i32 {
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
        let team = snapshot.player_state.persistant.get(PersistentIndex::PersTeam as usize);
        if team == Team::TeamSpectator as i32 && snapshot.player_state.pm_flags & (MoveFlags::Scoreboard as i32) != 0 {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::q3::base::shared::entity_state::EntityState as CanonicalEntityState;
    use crate::q3::base::shared::player_state::MoveFlags;
    use crate::q3::base::shared::player_state::PlayerState as CanonicalPlayerState;
    use qa_core::math::{vec3, Axis, Vec3};
    use std::collections::HashMap;

    fn test_state() -> ClientGameState {
        ClientGameState::new(Product::Baseq3, 0, 0).unwrap()
    }
    fn test_static() -> ClientGameStaticState {
        ClientGameStaticState::new(Product::Baseq3)
    }

    // ---------- resource doubles ----------
    pub(crate) struct TestFrameHost {
        snap: Option<Snapshot>,
        cvars: HashMap<String, CvarSnapshot>,
        rage_pro: bool,
        weapons: Vec<i32>,
        pre_view_weapon: bool,
        view_weapons: u32,
        test_model: Option<RefEntity>,
        damage_blob: Option<RefEntity>,
        marks: Vec<RefPoly>,
        particles: Vec<RefPoly>,
        scene_entities: Vec<RefEntity>,
        scene_polys: Vec<RefPoly>,
        clears: u32,
        rendered: u32,
        loading_frames: u32,
        draws_2d: u32,
        scoreboards: u32,
        lag: u32,
        prints: Vec<String>,
        listener: Option<(i32, Vec3)>,
        user_command: Option<(i32, f32)>,
        timescale_cvar: Option<f32>,
        timescale: Option<f32>,
        looping: Vec<bool>,
    }

    impl TestFrameHost {
        pub(crate) fn new() -> Self {
            Self {
                snap: None,
                cvars: HashMap::new(),
                rage_pro: false,
                weapons: Vec::new(),
                pre_view_weapon: true,
                view_weapons: 0,
                test_model: None,
                damage_blob: None,
                marks: Vec::new(),
                particles: Vec::new(),
                scene_entities: Vec::new(),
                scene_polys: Vec::new(),
                clears: 0,
                rendered: 0,
                loading_frames: 0,
                draws_2d: 0,
                scoreboards: 0,
                lag: 0,
                prints: Vec::new(),
                listener: None,
                user_command: None,
                timescale_cvar: None,
                timescale: None,
                looping: Vec::new(),
            }
        }

        fn with_cvar(mut self, name: FrameCvar, numeric_value: f32, integer_value: i32) -> Self {
            self.cvars.insert(
                name.as_str().to_string(),
                CvarSnapshot {
                    name: name.as_str().to_string(),
                    value: numeric_value.to_string(),
                    numeric_value,
                    integer_value,
                },
            );
            self
        }
    }

    impl Q3PresentationSceneHost for TestFrameHost {
        fn update_cvars(&mut self) -> PresentResult<()> {
            Ok(())
        }
        fn register_weapon(&mut self, weapon: i32) -> PresentResult<()> {
            self.weapons.push(weapon);
            Ok(())
        }
        fn process_snapshots(
            &mut self,
            state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            state.snap = self.snap.clone();
            Ok(())
        }
        fn packet_options(&self, static_state: &ClientGameStaticState) -> PacketEntityOptions {
            PacketEntityOptions {
                game_type: static_state.game_type,
                smooth_clients: false,
                simple_items: false,
                obelisk_respawn_delay: 0,
            }
        }
        fn add_packet_entities(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &ClientGameStaticState,
            _options: &PacketEntityOptions,
        ) {
        }
        fn poll_impact_marks(&mut self, _state: &ClientGameState) -> Vec<RefPoly> {
            std::mem::take(&mut self.marks)
        }
        fn poll_particles(&mut self, _state: &ClientGameState) -> Vec<RefPoly> {
            std::mem::take(&mut self.particles)
        }
        fn add_local_entities(&mut self, _state: &ClientGameState) {}
        fn play_buffered_sounds(&mut self, _state: &mut ClientGameState) {}
        fn play_buffered_voice_chats(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn add_poly(&mut self, poly: RefPoly) {
            self.scene_polys.push(poly);
        }
        fn add_ref_entity(&mut self, entity: RefEntity) {
            self.scene_entities.push(entity);
        }
        fn clear_scene(&mut self) {
            self.clears += 1;
            self.scene_entities.clear();
            self.scene_polys.clear();
        }
        fn render_scene(&mut self, _state: &ClientGameState) {
            self.rendered += 1;
        }
        fn enter_frame(&mut self, _frame: &Q3PresentationFrame) {}
        fn clear_looping_sounds(&mut self, kill_all: bool) {
            self.looping.push(kill_all);
        }
    }

    impl Q3PresentationFrameHost for TestFrameHost {
        fn hardware_is_rage_pro(&self) -> bool {
            self.rage_pro
        }
        fn predict_player_state(
            &mut self,
            state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            if let Some(snapshot) = &state.snap {
                state.predicted_player_state = snapshot.player_state.clone();
            }
            Ok(())
        }
        fn calculate_view_values(&mut self, _state: &mut ClientGameState) {}
        fn damage_blend_blob(&mut self, _state: &ClientGameState, _rage_pro: bool) -> Option<RefEntity> {
            self.damage_blob.clone()
        }
        fn pre_present_view_weapon(&mut self, _state: &ClientGameState) -> bool {
            self.pre_view_weapon
        }
        fn add_view_weapon(&mut self, _state: &mut ClientGameState) {
            self.view_weapons += 1;
        }
        fn post_present_view_weapon(&mut self, _state: &mut ClientGameState) {}
        fn add_test_model(&mut self, _state: &ClientGameState) -> PresentResult<Option<RefEntity>> {
            Ok(self.test_model.clone())
        }
        fn finish_refdef(&mut self, state: &mut ClientGameState) {
            state.refdef.time = state.time;
        }
        fn powerup_timer_sounds(&mut self, _state: &ClientGameState) {}
        fn set_listener(&mut self, client: i32, origin: Vec3, _axis: Axis) {
            self.listener = Some((client, origin));
        }
        fn add_lagometer_frame_info(&mut self, _state: &ClientGameState) {
            self.lag += 1;
        }
        fn read_frame_cvar(&self, name: FrameCvar) -> CvarSnapshot {
            self.cvars.get(name.as_str()).cloned().unwrap_or(CvarSnapshot {
                name: name.as_str().to_string(),
                value: "0".to_string(),
                numeric_value: 0.0,
                integer_value: 0,
            })
        }
        fn set_timescale_cvar(&mut self, value: f32) {
            self.timescale_cvar = Some(value);
        }
        fn set_timescale(&mut self, value: f32) {
            self.timescale = Some(value);
        }
        fn set_user_command_value(&mut self, weapon: i32, sensitivity: f32) {
            self.user_command = Some((weapon, sensitivity));
        }
        fn loading_frame(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            self.loading_frames += 1;
            Ok(())
        }
        fn draw_tourney_scoreboard(&mut self, _state: &mut ClientGameState, _static_state: &mut ClientGameStaticState) {
            self.scoreboards += 1;
        }
        fn tile_clear(&mut self, _state: &ClientGameState) {}
        fn draw_2d(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            self.draws_2d += 1;
            Ok(())
        }
        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }
    }

    // ---------- session / server doubles ----------
    fn snap_fixture() -> Snapshot {
        let mut ps = CanonicalPlayerState::new(Product::Baseq3, None);
        ps.client_num = 3;
        ps.weapon = 2;
        // Transitional: the base-game weapons slot is 2.
        ps.stats.set(2, (1 << 2) | (1 << 5));
        ps.persistant
            .set(PersistentIndex::PersTeam as usize, Team::TeamFree as i32);
        Snapshot {
            message_number: 1,
            server_time: 1000,
            delta_number: 0,
            flags: 0,
            server_command_number: 0,
            parse_entities_number: 0,
            area_mask: [0; 32],
            player_state: ps,
            entities: vec![CanonicalEntityState {
                number: 5,
                e_type: EntityType::EtPlayer as i32,
                weapon: 3,
                ..CanonicalEntityState::default()
            }],
        }
    }
    fn frame_input() -> Q3PresentationFrame {
        Q3PresentationFrame {
            server_time: 100,
            stereo: FrameStereo::Center,
            demo_playback: false,
            engine_frame_number: 7,
        }
    }
    #[test]
    fn frame_scope_guards_hold() {
        let mut scene = Q3PresentationFrameRuntime::new_scene(TestFrameHost::new());
        let mut state = test_state();
        let mut static_state = test_static();
        assert!(scene
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .is_err());
        let mut primary = Q3PresentationFrameRuntime::new(TestFrameHost::new());
        let camera = SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [1.0; 16],
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 640,
            viewport_height: 480,
        };
        assert!(primary
            .draw_scene_frame(&mut state, &mut static_state, &frame_input(), &camera)
            .is_err());
        primary.close().unwrap();
        assert!(primary
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .is_err());
    }

    #[test]
    fn frame_registers_weapons_and_renders() {
        let mut host = TestFrameHost::new();
        host.snap = Some(snap_fixture());
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        state.weapon_select = 5;
        state.zoom_sensitivity = 1.5;
        state.hyperspace = false;
        state.rendering_third_person = false;
        state.entity_at_mut(5).unwrap().current_state.e_type = EntityType::EtPlayer as i32;
        state.entity_at_mut(5).unwrap().current_state.weapon = 3;
        let before = state.client_frame;
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        assert_eq!(frames.host.weapons, vec![2, 5, 3]);
        assert_eq!(frames.host.rendered, 1);
        assert_eq!(frames.host.draws_2d, 1);
        assert_eq!(frames.host.view_weapons, 1);
        assert_eq!(state.client_frame, before.wrapping_add(1));
        assert_eq!(state.frame_time, 100);
        assert_eq!(state.old_time, 100);
        assert_eq!(frames.host.user_command, Some((5, 1.5)));
        assert_eq!(frames.host.listener.unwrap().0, 3);
        assert_eq!(frames.host.lag, 1);
        assert_eq!(state.refdef.view_origin, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn frame_loading_paths_skip_render() {
        let mut host = TestFrameHost::new();
        host.snap = Some(snap_fixture());
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        state.info_screen_text = "loading".to_string();
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        assert_eq!(frames.host.loading_frames, 1);
        assert_eq!(frames.host.rendered, 0);

        let mut frames = Q3PresentationFrameRuntime::new(TestFrameHost::new());
        let mut state = test_state();
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        assert_eq!(frames.host.loading_frames, 1);
        assert_eq!(frames.host.rendered, 0);
    }

    #[test]
    fn frame_spectator_scoreboard_skips_scene() {
        let mut snapshot = snap_fixture();
        snapshot
            .player_state
            .persistant
            .set(PersistentIndex::PersTeam as usize, Team::TeamSpectator as i32);
        snapshot.player_state.pm_flags = MoveFlags::Scoreboard as i32;
        let mut host = TestFrameHost::new();
        host.snap = Some(snapshot);
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        assert_eq!(frames.host.scoreboards, 1);
        assert_eq!(frames.host.rendered, 0);
        assert_eq!(frames.host.draws_2d, 0);
    }

    #[test]
    fn frame_right_eye_skips_frame_time() {
        let mut host = TestFrameHost::new();
        host.snap = Some(snap_fixture());
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        let input = Q3PresentationFrame {
            stereo: FrameStereo::Right,
            ..frame_input()
        };
        frames.draw_active_frame(&mut state, &mut static_state, &input).unwrap();
        assert_eq!(frames.host.lag, 0);
        assert_eq!(state.frame_time, 0);
        assert_eq!(frames.host.rendered, 1);
    }

    #[test]
    fn frame_timescale_fades_toward_end() {
        let mut host = TestFrameHost::new();
        host.snap = Some(snap_fixture());
        host = host
            .with_cvar(FrameCvar::CgTimescaleFadeEnd, 1.0, 1)
            .with_cvar(FrameCvar::CgTimescaleFadeSpeed, 1.0, 1)
            .with_cvar(FrameCvar::CgTimescale, 0.0, 0);
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        let cvar = frames.host.timescale_cvar.unwrap();
        assert!((cvar - 0.1).abs() < 1e-6, "unexpected timescale {cvar}");
        assert!((frames.host.timescale.unwrap() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn frame_scene_sets_refdef_from_camera() {
        let mut snapshot = snap_fixture();
        snapshot.entities[0].e_type = EntityType::EtMissile as i32;
        snapshot.entities[0].weapon = 99;
        let mut host = TestFrameHost::new();
        host.snap = Some(snapshot);
        let mut frames = Q3PresentationFrameRuntime::new_scene(host);
        let mut state = test_state();
        let mut static_state = test_static();
        let camera = SceneCamera {
            origin: vec3(1.0, 2.0, 3.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [1.0; 16],
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 640,
            viewport_height: 480,
        };
        frames
            .draw_scene_frame(&mut state, &mut static_state, &frame_input(), &camera)
            .unwrap();
        assert_eq!(state.refdef.view_origin, vec3(1.0, 2.0, 3.0));
        assert_eq!(state.refdef.width, 640);
        assert_eq!(state.refdef.height, 480);
        assert!((state.refdef.fov_x - 90.0).abs() < 1e-4);
        assert!((state.refdef.fov_y - 90.0).abs() < 1e-4);
        assert_eq!(state.refdef.time, 100);
        assert_eq!(frames.host.weapons, vec![2, 0]);
    }

    #[test]
    fn frame_scene_rejects_backward_time() {
        let mut frames = Q3PresentationFrameRuntime::new_scene(TestFrameHost::new());
        let mut state = test_state();
        let mut static_state = test_static();
        state.old_time = 200;
        let camera = SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [1.0; 16],
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 640,
            viewport_height: 480,
        };
        assert!(frames
            .draw_scene_frame(&mut state, &mut static_state, &frame_input(), &camera)
            .is_err());
    }

    // ---------- assembly tests ----------
}
