//! Quake III presentation: state.
//!
//! Donor provenance: `src/content/q3/presentation/state.ts`.

use crate::q3anim::PlayerAnimation;
use qa_core::math::{vec3, Axis, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_client::*;
use crate::q3::presentation::retail_snapshot::*;

// ---------------------------------------------------------------------------
// Owned client state (state.ts)
// ---------------------------------------------------------------------------

/// Score row (`ClientScore`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientScore {
    /// Client number.
    pub client: i32,
    /// Score.
    pub score: i32,
    /// Ping.
    pub ping: i32,
    /// Time.
    pub time: i32,
    /// Score flags.
    pub score_flags: i32,
    /// Accuracy.
    pub accuracy: i32,
    /// Impressive count.
    pub impressive_count: i32,
    /// Excellent count.
    pub excellent_count: i32,
    /// Gauntlet count.
    pub guantlet_count: i32,
    /// Defend count.
    pub defend_count: i32,
    /// Assist count.
    pub assist_count: i32,
    /// Perfect.
    pub perfect: i32,
    /// Captures.
    pub captures: i32,
    /// Team tag.
    pub team: i32,
}

/// Reward stack entry (`ClientReward`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientReward {
    /// Sound.
    pub sound: Option<PcmSound>,
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Count.
    pub count: i32,
}

/// Animation lerp frame (`LerpFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct LerpFrame {
    /// Old frame.
    pub old_frame: i32,
    /// Old frame time.
    pub old_frame_time: i32,
    /// Frame.
    pub frame: i32,
    /// Frame time.
    pub frame_time: i32,
    /// Back lerp.
    pub back_lerp: f32,
    /// Animation number.
    pub animation_number: i32,
    /// Current animation.
    pub current_animation: Option<PlayerAnimation>,
    /// Animation time.
    pub animation_time: i32,
}

/// New lerp frame (`createLerpFrame`).
#[must_use]
pub const fn create_lerp_frame() -> LerpFrame {
    LerpFrame {
        old_frame: 0,
        old_frame_time: 0,
        frame: 0,
        frame_time: 0,
        back_lerp: 0.0,
        animation_number: 0,
        current_animation: None,
        animation_time: 0,
    }
}

/// Pose lerp frame (`PoseLerpFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct PoseLerpFrame {
    /// Base frame.
    pub base: LerpFrame,
    /// Yaw angle.
    pub yaw_angle: f32,
    /// Yawing.
    pub yawing: bool,
    /// Pitch angle.
    pub pitch_angle: f32,
    /// Pitching.
    pub pitching: bool,
}

/// New pose lerp frame.
#[must_use]
pub const fn create_pose_lerp_frame() -> PoseLerpFrame {
    PoseLerpFrame {
        base: create_lerp_frame(),
        yaw_angle: 0.0,
        yawing: false,
        pitch_angle: 0.0,
        pitching: false,
    }
}

/// Per-entity player presentation state (`ClientPlayerEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientPlayerEntity {
    /// Legs frame.
    pub legs: PoseLerpFrame,
    /// Torso frame.
    pub torso: PoseLerpFrame,
    /// Pain time.
    pub pain_time: i32,
    /// Pain direction.
    pub pain_direction: bool,
    /// Flag frame.
    pub flag: PoseLerpFrame,
    /// Lightning firing time.
    pub lightning_firing: i32,
    /// Railgun impact point.
    pub railgun_impact: Vec3,
    /// Railgun flash.
    pub railgun_flash: bool,
    /// Barrel angle.
    pub barrel_angle: f32,
    /// Barrel time.
    pub barrel_time: i32,
    /// Barrel spinning.
    pub barrel_spinning: bool,
}

/// New per-entity player state (`createClientPlayerEntity`).
#[must_use]
pub fn create_client_player_entity() -> ClientPlayerEntity {
    ClientPlayerEntity {
        legs: create_pose_lerp_frame(),
        torso: create_pose_lerp_frame(),
        pain_time: 0,
        pain_direction: false,
        flag: create_pose_lerp_frame(),
        lightning_firing: 0,
        railgun_impact: vec3(0.0, 0.0, 0.0),
        railgun_flash: false,
        barrel_angle: 0.0,
        barrel_time: 0,
        barrel_spinning: false,
    }
}

/// Harvester skull trail (`SkullTrail`).
#[derive(Debug, Clone, PartialEq)]
pub struct SkullTrail {
    /// Positions.
    pub positions: [Vec3; 10],
    /// Position count.
    pub num_positions: i32,
}

pub(crate) fn create_skull_trail() -> SkullTrail {
    SkullTrail {
        positions: [vec3(0.0, 0.0, 0.0); 10],
        num_positions: 0,
    }
}

/// Client entity (`ClientEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientEntity {
    /// Current state.
    pub current_state: EntityState,
    /// Next state.
    pub next_state: EntityState,
    /// Interpolate.
    pub interpolate: bool,
    /// Current valid.
    pub current_valid: bool,
    /// Muzzle flash time.
    pub muzzle_flash_time: i32,
    /// Previous event.
    pub previous_event: i32,
    /// Teleport flag.
    pub teleport_flag: i32,
    /// Trail time.
    pub trail_time: i32,
    /// Dust trail time.
    pub dust_trail_time: i32,
    /// Misc time.
    pub misc_time: i32,
    /// Snapshot time.
    pub snapshot_time: i32,
    /// Player state.
    pub player: ClientPlayerEntity,
    /// Error time.
    pub error_time: i32,
    /// Error origin.
    pub error_origin: Vec3,
    /// Error angles.
    pub error_angles: Vec3,
    /// Extrapolated.
    pub extrapolated: bool,
    /// Raw origin.
    pub raw_origin: Vec3,
    /// Raw angles.
    pub raw_angles: Vec3,
    /// Beam end.
    pub beam_end: Vec3,
    /// Lerped origin.
    pub lerp_origin: Vec3,
    /// Lerped angles.
    pub lerp_angles: Vec3,
}

impl ClientEntity {
    /// Blank client entity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_state: EntityState::default(),
            next_state: EntityState::default(),
            interpolate: false,
            current_valid: false,
            muzzle_flash_time: 0,
            previous_event: 0,
            teleport_flag: 0,
            trail_time: 0,
            dust_trail_time: 0,
            misc_time: 0,
            snapshot_time: 0,
            player: create_client_player_entity(),
            error_time: 0,
            error_origin: vec3(0.0, 0.0, 0.0),
            error_angles: vec3(0.0, 0.0, 0.0),
            extrapolated: false,
            raw_origin: vec3(0.0, 0.0, 0.0),
            raw_angles: vec3(0.0, 0.0, 0.0),
            beam_end: vec3(0.0, 0.0, 0.0),
            lerp_origin: vec3(0.0, 0.0, 0.0),
            lerp_angles: vec3(0.0, 0.0, 0.0),
        }
    }
}

impl Default for ClientEntity {
    fn default() -> Self {
        Self::new()
    }
}

/// Static client state surviving frames (`ClientGameStaticState`, `cgs_t`).
#[derive(Debug, Clone)]
pub struct ClientGameStaticState {
    /// Product.
    pub product: Q3Product,
    /// Server command sequence.
    pub server_command_sequence: i32,
    /// Cursor X.
    pub cursor_x: i32,
    /// Cursor Y.
    pub cursor_y: i32,
    /// Event handling.
    pub event_handling: i32,
    /// Active cursor.
    pub active_cursor: Option<SceneShader>,
    /// Team chat messages.
    pub team_chat_msgs: [String; 8],
    /// Team chat message times.
    pub team_chat_msg_times: [i32; 8],
    /// Team chat position.
    pub team_chat_pos: i32,
    /// Last team chat position.
    pub team_last_chat_pos: i32,
    /// Current voice client.
    pub current_voice_client: i32,
    /// Accept order time.
    pub accept_order_time: i32,
    /// Accept task.
    pub accept_task: i32,
    /// Accept leader.
    pub accept_leader: i32,
    /// Accept voice.
    pub accept_voice: String,
    /// Current order.
    pub current_order: i32,
    /// Order pending.
    pub order_pending: bool,
    /// Order time.
    pub order_time: i32,
    /// Client info slots.
    pub client_info: [ClientInfo; MAX_CLIENTS],
    /// Game models.
    pub game_models: [SceneModel; 256],
    /// Game sounds.
    pub game_sounds: [Option<PcmSound>; 256],
    /// Game type.
    pub game_type: GameType,
    /// DM flags.
    pub dm_flags: i32,
    /// Team flags.
    pub team_flags: i32,
    /// Frag limit.
    pub fraglimit: i32,
    /// Capture limit.
    pub capturelimit: i32,
    /// Time limit.
    pub timelimit: i32,
    /// Max clients.
    pub maxclients: i32,
    /// Map name.
    pub mapname: String,
    /// Local server.
    pub local_server: i32,
    /// Red team.
    pub red_team: String,
    /// Blue team.
    pub blue_team: String,
    /// Vote time.
    pub vote_time: i32,
    /// Vote yes.
    pub vote_yes: i32,
    /// Vote no.
    pub vote_no: i32,
    /// Vote modified.
    pub vote_modified: bool,
    /// Vote string.
    pub vote_string: String,
    /// Team vote times.
    pub team_vote_time: [i32; 2],
    /// Team vote yes.
    pub team_vote_yes: [i32; 2],
    /// Team vote no.
    pub team_vote_no: [i32; 2],
    /// Team vote modified.
    pub team_vote_modified: [bool; 2],
    /// Team vote strings.
    pub team_vote_string: [String; 2],
    /// Level start time.
    pub level_start_time: i32,
    /// Scores 1.
    pub scores1: i32,
    /// Scores 2.
    pub scores2: i32,
    /// Red flag state.
    pub redflag: i32,
    /// Blue flag state.
    pub blueflag: i32,
    /// Flag status.
    pub flag_status: i32,
}

impl ClientGameStaticState {
    /// Blank static state.
    #[must_use]
    pub fn new(product: Q3Product) -> Self {
        Self {
            product,
            server_command_sequence: 0,
            cursor_x: 0,
            cursor_y: 0,
            event_handling: 0,
            active_cursor: None,
            team_chat_msgs: std::array::from_fn(|_| String::new()),
            team_chat_msg_times: [0; 8],
            team_chat_pos: 0,
            team_last_chat_pos: 0,
            current_voice_client: 0,
            accept_order_time: 0,
            accept_task: 0,
            accept_leader: 0,
            accept_voice: String::new(),
            current_order: 0,
            order_pending: false,
            order_time: 0,
            client_info: std::array::from_fn(|_| ClientInfo::new()),
            game_models: std::array::from_fn(|_| default_model()),
            game_sounds: std::array::from_fn(|_| None),
            game_type: GameType::Ffa,
            dm_flags: 0,
            team_flags: 0,
            fraglimit: 0,
            capturelimit: 0,
            timelimit: 0,
            maxclients: 0,
            mapname: String::new(),
            local_server: 0,
            red_team: String::new(),
            blue_team: String::new(),
            vote_time: 0,
            vote_yes: 0,
            vote_no: 0,
            vote_modified: false,
            vote_string: String::new(),
            team_vote_time: [0; 2],
            team_vote_yes: [0; 2],
            team_vote_no: [0; 2],
            team_vote_modified: [false; 2],
            team_vote_string: std::array::from_fn(|_| String::new()),
            level_start_time: 0,
            scores1: 0,
            scores2: 0,
            redflag: 0,
            blueflag: 0,
            flag_status: 0,
        }
    }
}

/// Owned per-frame client state (`ClientGameState`, `cg_t`).
#[derive(Debug, Clone)]
pub struct ClientGameState {
    /// Product.
    pub product: Q3Product,
    /// Client number.
    pub client_num: i32,
    /// Processed snapshot number.
    pub processed_snapshot_num: i32,
    entities: Vec<ClientEntity>,
    /// Predicted player entity.
    pub predicted_player_entity: ClientEntity,
    /// Skull trails.
    pub skull_trails: [SkullTrail; MAX_CLIENTS],
    /// Solid entity numbers.
    pub solid_entities: Vec<usize>,
    /// Trigger entity numbers.
    pub trigger_entities: Vec<usize>,
    /// Score count.
    pub num_scores: i32,
    /// Selected score.
    pub selected_score: i32,
    /// Scores request time.
    pub scores_request_time: i32,
    /// Show scores.
    pub show_scores: bool,
    /// Score fade time.
    pub score_fade_time: i32,
    /// Scoreboard showing.
    pub score_board_showing: bool,
    /// Deferred player loading.
    pub deferred_player_loading: i32,
    /// Center print text.
    pub center_print: String,
    /// Center print time.
    pub center_print_time: i32,
    /// Center print char width.
    pub center_print_char_width: i32,
    /// Center print Y.
    pub center_print_y: i32,
    /// Center print lines.
    pub center_print_lines: i32,
    /// Head start yaw.
    pub head_start_yaw: f32,
    /// Head end yaw.
    pub head_end_yaw: f32,
    /// Head start pitch.
    pub head_start_pitch: f32,
    /// Head end pitch.
    pub head_end_pitch: f32,
    /// Head start time.
    pub head_start_time: i32,
    /// Head end time.
    pub head_end_time: i32,
    /// Voice time.
    pub voice_time: i32,
    /// Crosshair client number.
    pub crosshair_client_num: i32,
    /// Crosshair client time.
    pub crosshair_client_time: i32,
    /// Scores.
    pub scores: [ClientScore; MAX_CLIENTS],
    /// Team scores.
    pub team_scores: [i32; 2],
    /// Sorted team player count.
    pub num_sorted_team_players: i32,
    /// Sorted team players.
    pub sorted_team_players: [i32; 8],
    /// Warmup count.
    pub warmup_count: i32,
    /// Level shot.
    pub level_shot: bool,
    /// Info screen text.
    pub info_screen_text: String,
    /// Voice chat time.
    pub voice_chat_time: i32,
    /// Voice chat buffer in.
    pub voice_chat_buffer_in: usize,
    /// Voice chat buffer out.
    pub voice_chat_buffer_out: usize,
    /// Sound buffer in.
    pub sound_buffer_in: i32,
    /// Sound buffer out.
    pub sound_buffer_out: i32,
    /// Sound time.
    pub sound_time: i32,
    /// Sound buffer.
    pub sound_buffer: [Option<PcmSound>; 20],
    /// Spectator list.
    pub spectator_list: String,
    /// Spectator length.
    pub spectator_len: usize,
    /// Spectator width.
    pub spectator_width: i32,
    /// Spectator time.
    pub spectator_time: i32,
    /// Spectator offset.
    pub spectator_offset: i32,
    /// Spectator paint X.
    pub spectator_paint_x: i32,
    /// Spectator paint X2.
    pub spectator_paint_x2: i32,
    /// Spectator paint length.
    pub spectator_paint_len: i32,
    /// Latest snapshot number.
    pub latest_snapshot_num: i32,
    /// Latest snapshot time.
    pub latest_snapshot_time: i32,
    /// Current snapshot.
    pub snap: Option<RetailSnapshot>,
    /// Next snapshot.
    pub next_snap: Option<RetailSnapshot>,
    /// Time.
    pub time: i32,
    /// Old time.
    pub old_time: i32,
    /// Frame time.
    pub frame_time: i32,
    /// Physics time.
    pub physics_time: i32,
    /// Frame interpolation.
    pub frame_interpolation: f32,
    /// This-frame teleport.
    pub this_frame_teleport: bool,
    /// Next-frame teleport.
    pub next_frame_teleport: bool,
    /// Client frame.
    pub client_frame: i32,
    /// Auto angles.
    pub auto_angles: Vec3,
    /// Fast auto angles.
    pub auto_angles_fast: Vec3,
    /// Auto axis.
    pub auto_axis: Axis,
    /// Fast auto axis.
    pub auto_axis_fast: Axis,
    /// Map restart.
    pub map_restart: bool,
    /// Hyperspace.
    pub hyperspace: bool,
    /// Predicted player state.
    pub predicted_player_state: PlayerState,
    /// Valid predicted player state.
    pub valid_pps: bool,
    /// Predicted error time.
    pub predicted_error_time: i32,
    /// Predicted error.
    pub predicted_error: Vec3,
    /// Event sequence.
    pub event_sequence: i32,
    /// Killer name.
    pub killer_name: String,
    /// Item pickup.
    pub item_pickup: i32,
    /// Item pickup time.
    pub item_pickup_time: i32,
    /// Item pickup blend time.
    pub item_pickup_blend_time: i32,
    /// Weapon select.
    pub weapon_select: i32,
    /// Weapon select time.
    pub weapon_select_time: i32,
    /// Land change.
    pub land_change: f32,
    /// Land time.
    pub land_time: i32,
    /// Step change.
    pub step_change: f32,
    /// Step time.
    pub step_time: i32,
    /// Powerup active.
    pub powerup_active: i32,
    /// Powerup time.
    pub powerup_time: i32,
    /// View definition.
    pub refdef: Refdef,
    /// View angles.
    pub refdef_view_angles: Vec3,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// XY speed.
    pub xyspeed: f32,
    /// Bob fraction sine.
    pub bob_frac_sin: f32,
    /// Rendering third person.
    pub rendering_third_person: bool,
    /// Test gun.
    pub test_gun: bool,
    /// Kick angles.
    pub kick_angles: Vec3,
    /// Kick origin.
    pub kick_origin: Vec3,
    /// Damage time.
    pub damage_time: f32,
    /// Damage kick end time.
    pub damage_kick_end_time: f32,
    /// Attacker time.
    pub attacker_time: i32,
    /// Low ammo warning.
    pub low_ammo_warning: i32,
    /// Reward stack.
    pub reward_stack: i32,
    /// Reward time.
    pub reward_time: i32,
    /// Rewards.
    pub rewards: [ClientReward; 10],
    /// Intermission started.
    pub intermission_started: bool,
    /// Warmup.
    pub warmup: i32,
    /// Time-limit warnings.
    pub timelimit_warnings: i32,
    /// Frag-limit warnings.
    pub fraglimit_warnings: i32,
    /// Damage pitch.
    pub damage_pitch: f32,
    /// Damage roll.
    pub damage_roll: f32,
    /// Damage X.
    pub damage_x: f32,
    /// Damage Y.
    pub damage_y: f32,
    /// Damage value.
    pub damage_value: f32,
    /// Duck change.
    pub duck_change: f32,
    /// Duck time.
    pub duck_time: i32,
    /// Zoomed.
    pub zoomed: bool,
    /// Zoom time.
    pub zoom_time: i32,
    /// Zoom sensitivity.
    pub zoom_sensitivity: f32,
    /// Next orbit time.
    pub next_orbit_time: i32,
    /// Test model name.
    pub test_model_name: String,
    /// Test model entity.
    pub test_model_entity: RefModelEntity,
    /// Predictable events ring (16).
    pub predictable_events: PlayerStateSlots,
}

impl ClientGameState {
    /// New client state.
    pub fn new(product: Q3Product, client_num: i32, processed_snapshot_num: i32) -> PresentResult<Self> {
        if client_num < 0 || client_num >= MAX_CLIENTS as i32 {
            return Err(range_msg("Invalid cgame client number"));
        }
        Ok(Self {
            product,
            client_num,
            processed_snapshot_num,
            entities: vec![ClientEntity::new(); MAX_ENTITIES],
            predicted_player_entity: ClientEntity::new(),
            skull_trails: std::array::from_fn(|_| create_skull_trail()),
            solid_entities: Vec::new(),
            trigger_entities: Vec::new(),
            num_scores: 0,
            selected_score: 0,
            scores_request_time: 0,
            show_scores: false,
            score_fade_time: 0,
            score_board_showing: false,
            deferred_player_loading: 0,
            center_print: String::new(),
            center_print_time: 0,
            center_print_char_width: 0,
            center_print_y: 0,
            center_print_lines: 0,
            head_start_yaw: 0.0,
            head_end_yaw: 0.0,
            head_start_pitch: 0.0,
            head_end_pitch: 0.0,
            head_start_time: 0,
            head_end_time: 0,
            voice_time: 0,
            crosshair_client_num: 0,
            crosshair_client_time: 0,
            scores: [ClientScore::default(); MAX_CLIENTS],
            team_scores: [0; 2],
            num_sorted_team_players: 0,
            sorted_team_players: [0; 8],
            warmup_count: 0,
            level_shot: false,
            info_screen_text: String::new(),
            voice_chat_time: 0,
            voice_chat_buffer_in: 0,
            voice_chat_buffer_out: 0,
            sound_buffer_in: 0,
            sound_buffer_out: 0,
            sound_time: 0,
            sound_buffer: std::array::from_fn(|_| None),
            spectator_list: String::new(),
            spectator_len: 0,
            spectator_width: 0,
            spectator_time: 0,
            spectator_offset: 0,
            spectator_paint_x: 0,
            spectator_paint_x2: 0,
            spectator_paint_len: 0,
            latest_snapshot_num: 0,
            latest_snapshot_time: 0,
            snap: None,
            next_snap: None,
            time: 0,
            old_time: 0,
            frame_time: 0,
            physics_time: 0,
            frame_interpolation: 0.0,
            this_frame_teleport: false,
            next_frame_teleport: false,
            client_frame: 0,
            auto_angles: vec3(0.0, 0.0, 0.0),
            auto_angles_fast: vec3(0.0, 0.0, 0.0),
            auto_axis: [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)],
            auto_axis_fast: [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)],
            map_restart: false,
            hyperspace: false,
            predicted_player_state: PlayerState::new(product),
            valid_pps: false,
            predicted_error_time: 0,
            predicted_error: vec3(0.0, 0.0, 0.0),
            event_sequence: 0,
            killer_name: String::new(),
            item_pickup: 0,
            item_pickup_time: 0,
            item_pickup_blend_time: 0,
            weapon_select: 0,
            weapon_select_time: 0,
            land_change: 0.0,
            land_time: 0,
            step_change: 0.0,
            step_time: 0,
            powerup_active: 0,
            powerup_time: 0,
            refdef: create_refdef(),
            refdef_view_angles: vec3(0.0, 0.0, 0.0),
            bob_cycle: 0,
            xyspeed: 0.0,
            bob_frac_sin: 0.0,
            rendering_third_person: false,
            test_gun: false,
            kick_angles: vec3(0.0, 0.0, 0.0),
            kick_origin: vec3(0.0, 0.0, 0.0),
            damage_time: 0.0,
            damage_kick_end_time: 0.0,
            attacker_time: 0,
            low_ammo_warning: 0,
            reward_stack: 0,
            reward_time: 0,
            rewards: std::array::from_fn(|_| ClientReward {
                sound: None,
                shader: None,
                count: 0,
            }),
            intermission_started: false,
            warmup: 0,
            timelimit_warnings: 0,
            fraglimit_warnings: 0,
            damage_pitch: 0.0,
            damage_roll: 0.0,
            damage_x: 0.0,
            damage_y: 0.0,
            damage_value: 0.0,
            duck_change: 0.0,
            duck_time: 0,
            zoomed: false,
            zoom_time: 0,
            zoom_sensitivity: 0.0,
            next_orbit_time: 0,
            test_model_name: String::new(),
            test_model_entity: create_model_entity(),
            predictable_events: PlayerStateSlots::new(16),
        })
    }

    /// Entity by number (`entityAt`).
    pub fn entity_at(&self, number: i32) -> PresentResult<&ClientEntity> {
        if number < 0 {
            return Err(range_msg(format!("Invalid cgame entity number {number}")));
        }
        at(&self.entities, number as usize, "Invalid cgame entity number")
    }

    /// Mutable entity by number.
    pub fn entity_at_mut(&mut self, number: i32) -> PresentResult<&mut ClientEntity> {
        if number < 0 {
            return Err(range_msg(format!("Invalid cgame entity number {number}")));
        }
        let len = self.entities.len();
        self.entities
            .get_mut(number as usize)
            .ok_or_else(|| range_msg(format!("Invalid cgame entity number {number} outside {len}")))
    }

    /// Take the predicted player state, leaving a fresh record.
    pub fn take_predicted_player_state(&mut self) -> PlayerState {
        std::mem::replace(&mut self.predicted_player_state, PlayerState::new(self.product))
    }
}
